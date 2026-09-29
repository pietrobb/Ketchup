//! Scheduler side of the typed exact-worker protocol: one request, one reply,
//! each checked against the request that produced it.
use super::*;
use crate::protocol::{
    ExportReceipt, Frame, GraphInput, MAX_REPLY_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, MeshReceipt,
    SourceFile, WorkerFailure, WorkerImportPart, WorkerReply, WorkerRequest, encode_frame,
    protocol_identity, read_frame,
};

pub(crate) struct WorkerWriteRequest {
    pub(crate) frame: Vec<u8>,
    pub(crate) acknowledgment: Sender<Result<(), String>>,
}

pub(crate) enum WorkerResponse {
    Reply(Box<WorkerReply>),
    Exited,
    TooLarge,
    Malformed(String),
    Transport(String),
}

impl ExactWorkerClient {
    pub(crate) fn read_bounded_output(
        &mut self,
        path: &Path,
        maximum_bytes: u64,
        limit_message: &str,
    ) -> Result<Vec<u8>, WorkerError> {
        match read_bounded_regular_file(path, maximum_bytes, limit_message) {
            Ok(bytes) => Ok(bytes),
            Err(error) => self.fail(error),
        }
    }

    pub fn spawn(executable: impl AsRef<Path>) -> Result<Self, WorkerError> {
        let executable = guarded_exact_worker_identity(executable.as_ref())?;
        Self::spawn_guarded(executable)
    }

    pub(crate) fn spawn_guarded(executable: GuardedExactWorker) -> Result<Self, WorkerError> {
        let working_directory = executable
            .executable
            .parent()
            .expect("canonical exact worker executable has a parent");
        let temp_directory =
            tempfile::tempdir().map_err(|error| WorkerError::Spawn(error.to_string()))?;
        let mut command = Command::new(&executable.executable);
        command
            .current_dir(working_directory)
            .env_clear()
            .env("TEMP", temp_directory.path())
            .env("TMP", temp_directory.path())
            .env("TMPDIR", temp_directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = spawn_exact_worker_command(command)
            .map_err(|error| WorkerError::Spawn(error.to_string()))?;
        let stdin = child
            .stdin()
            .take()
            .ok_or_else(|| WorkerError::Spawn("worker stdin was not piped".to_owned()))?;
        let stdout = child
            .stdout()
            .take()
            .ok_or_else(|| WorkerError::Spawn("worker stdout was not piped".to_owned()))?;
        let (write_sender, write_receiver) = mpsc::channel();
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        spawn_worker_writer(stdin, write_receiver);
        spawn_worker_reader(stdout, response_sender);
        Ok(Self {
            child,
            terminated: false,
            write_sender,
            response_receiver,
            _temp_directory: temp_directory,
        })
    }

    /// Handshake: the worker must speak the protocol of this exact build.
    pub fn ping(&mut self) -> Result<(), WorkerError> {
        self.ping_with_cancellation(&NEVER_CANCELLED)
    }

    pub(crate) fn ping_with_cancellation(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        match self.call(
            &WorkerRequest::Hello,
            cancelled,
            DEFAULT_WORKER_REQUEST_TIMEOUT,
        )? {
            WorkerReply::Hello { protocol } if protocol == protocol_identity() => Ok(()),
            WorkerReply::Hello { protocol } => self.fail_protocol(format!(
                "worker protocol {protocol} differs from scheduler protocol {}",
                protocol_identity()
            )),
            reply => self.unexpected(reply),
        }
    }

    pub(crate) fn simulate_cam_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        request: &CamSimulationWireRequest,
        cancelled: &AtomicBool,
    ) -> Result<CamSimulationWireEvidence, WorkerError> {
        validated_graph(graph)?;
        let request = WorkerRequest::SimulateCam {
            graph: graph.clone(),
            request: Box::new(request.clone()),
        };
        match self.call(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)? {
            WorkerReply::Cam(evidence) => Ok(evidence),
            reply => self.unexpected(reply),
        }
    }

    pub(crate) fn evaluate_exact_brep_graph_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<WorkerExactBRepGraphResult, WorkerError> {
        let request = WorkerRequest::EvaluateGraph(graph_input(graph, imported_sources)?);
        match self.call(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)? {
            WorkerReply::Graph(result) => Ok(result),
            reply => self.unexpected(reply),
        }
    }

    pub(crate) fn tessellate_exact_brep_graph_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        output_path: &Path,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        let request = WorkerRequest::TessellateGraph {
            input: graph_input(graph, imported_sources)?,
            result_fingerprint: result_fingerprint.to_owned(),
            output: output_path.to_owned(),
        };
        match self.call(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)? {
            WorkerReply::Mesh(receipt) => self.verified_mesh(
                receipt,
                result_fingerprint,
                output_path,
                "exact B-Rep graph mesh",
            ),
            reply => self.unexpected(reply),
        }
    }

    pub(crate) fn volume_mesh_exact_brep_graph_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        options: ExactVolumeMeshWireOptions,
        output_path: &Path,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<WorkerExactVolumeMesh, WorkerError> {
        let request = WorkerRequest::VolumeMeshGraph {
            input: graph_input(graph, imported_sources)?,
            result_fingerprint: result_fingerprint.to_owned(),
            options,
            output: output_path.to_owned(),
        };
        let receipt = match self.call(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)? {
            WorkerReply::VolumeMesh(receipt) => receipt,
            reply => return self.unexpected(reply),
        };
        let max_tetrahedra = options.max_tetrahedra;
        let maximum_vertices = max_tetrahedra.saturating_mul(4);
        let maximum_boundary_triangles = max_tetrahedra.saturating_mul(4);
        if receipt.result_fingerprint != result_fingerprint
            || !is_sha256_digest(&receipt.sha256)
            || receipt.vertex_count < 4
            || receipt.tetrahedron_count == 0
            || receipt.tetrahedron_count > max_tetrahedra
            || receipt.boundary_triangle_count == 0
            || receipt.vertex_count > maximum_vertices
            || receipt.boundary_triangle_count > maximum_boundary_triangles
        {
            return self.fail_protocol(format!("invalid volume-mesh receipt {receipt:?}"));
        }
        let encoded = self.read_bounded_output(
            output_path,
            MAX_EXACT_VOLUME_MESH_OUTPUT_BYTES,
            "exact worker volume-mesh output exceeds the bounded 64 MiB envelope",
        )?;
        if sha256_hex(&encoded) != receipt.sha256 {
            return Err(WorkerError::Transport(
                "exact volume-mesh digest does not match the worker receipt".to_owned(),
            ));
        }
        let mesh = serde_json::from_slice::<WorkerExactVolumeMesh>(&encoded)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if mesh.schema != EXACT_VOLUME_MESH_WIRE_SCHEMA_V1
            || mesh.graph_digest != graph.graph_digest
            || mesh.source_result_fingerprint != result_fingerprint
            || mesh.mesh_fingerprint != receipt.mesh_fingerprint
            || mesh.vertices_mm.len() as u32 != receipt.vertex_count
            || mesh.tetrahedra.len() as u32 != receipt.tetrahedron_count
            || mesh.boundary_triangles.len() as u32 != receipt.boundary_triangle_count
        {
            return Err(WorkerError::Transport(
                "exact volume mesh does not match the worker receipt or request identity"
                    .to_owned(),
            ));
        }
        Ok(mesh)
    }

    pub(crate) fn export_exact_brep_graph_step_with_cancellation(
        &mut self,
        graph: &ExactBRepGraph,
        result_fingerprint: &str,
        output_path: &Path,
        imported_sources: &[(&str, &Path)],
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let request = WorkerRequest::ExportGraphStep {
            input: graph_input(graph, imported_sources)?,
            result_fingerprint: result_fingerprint.to_owned(),
            output: output_path.to_owned(),
        };
        match self.call(&request, cancelled, EXACT_BREP_GRAPH_REQUEST_TIMEOUT)? {
            WorkerReply::Exported(ExportReceipt {
                result_fingerprint: exported,
                sha256: None,
            }) if exported == result_fingerprint => Ok(()),
            reply => self.unexpected(reply),
        }
    }

    pub(crate) fn inspect_step_xde_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<StepXdeImportEvidence, WorkerError> {
        let request = WorkerRequest::InspectStepXde(source_file(source_sha256, path));
        let evidence = self.inspect_xde(&request, source_sha256, cancelled)?;
        let (source_sha256, source_unit) = self.xde_identity(&evidence, source_sha256)?;
        let source_byte_len = evidence.source_byte_len;
        let parts = evidence
            .parts
            .into_iter()
            .map(|part| StepXdePartEvidence {
                index: part.index,
                name: part.name.clone(),
                name_from_source: part.name_from_source,
                color: part.color,
                exact: xde_part_evidence(part, source_sha256, source_byte_len, source_unit),
            })
            .collect();
        let nodes = evidence
            .nodes
            .into_iter()
            .map(|node| {
                let transform = Transform::from_matrix(node.transform)
                    .ok()
                    .filter(|transform| transform.rigid_inverse().is_some())?;
                Some(StepXdeNodeEvidence {
                    id: node.id,
                    parent_id: node.parent_id,
                    part_index: node.part_index,
                    name: node.name,
                    name_from_source: node.name_from_source,
                    color: node.color,
                    transform,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| WorkerError::Protocol("STEP XDE transform is not rigid".to_owned()))?;
        Ok(StepXdeImportEvidence {
            source_sha256,
            source_byte_len,
            parts,
            nodes,
        })
    }

    pub(crate) fn inspect_iges_xde_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<IgesXdeImportEvidence, WorkerError> {
        let request = WorkerRequest::InspectIgesXde(source_file(source_sha256, path));
        let evidence = self.inspect_xde(&request, source_sha256, cancelled)?;
        let (source_sha256, source_unit) = self.xde_identity(&evidence, source_sha256)?;
        let source_byte_len = evidence.source_byte_len;
        let parts = evidence
            .parts
            .into_iter()
            .map(|part| IgesXdePartEvidence {
                index: part.index,
                name: part.name.clone(),
                name_from_source: part.name_from_source,
                color: part.color,
                exact: xde_part_evidence(part, source_sha256, source_byte_len, source_unit),
            })
            .collect();
        let nodes = evidence
            .nodes
            .into_iter()
            .map(|node| {
                if node.parent_id.is_some() {
                    return None;
                }
                let part_index = node.part_index?;
                let transform = Transform::from_matrix(node.transform)
                    .ok()
                    .filter(|transform| *transform == Transform::identity())?;
                Some(IgesXdeNodeEvidence {
                    id: node.id,
                    part_index,
                    name: node.name,
                    name_from_source: node.name_from_source,
                    color: node.color,
                    transform,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                WorkerError::Protocol(
                    "IGES XDE evidence is not a flat identity-root model".to_owned(),
                )
            })?;
        Ok(IgesXdeImportEvidence {
            source_sha256,
            source_byte_len,
            parts,
            nodes,
        })
    }

    fn inspect_xde(
        &mut self,
        request: &WorkerRequest,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<StepXdeWorkerEvidence, WorkerError> {
        match self.call(request, cancelled, DEFAULT_WORKER_REQUEST_TIMEOUT)? {
            WorkerReply::Xde(evidence)
                if evidence.source_sha256 == source_sha256
                    && evidence
                        .parts
                        .iter()
                        .all(|part| parse_import_body_kind(&part.body_kind).is_some()) =>
            {
                Ok(evidence)
            }
            reply => self.unexpected(reply),
        }
    }

    fn xde_identity(
        &mut self,
        evidence: &StepXdeWorkerEvidence,
        source_sha256: &str,
    ) -> Result<([u8; 32], ImportLengthUnit), WorkerError> {
        let digest = decode_sha256(source_sha256)
            .ok_or_else(|| WorkerError::Protocol("invalid import source SHA-256".to_owned()))?;
        match parse_source_unit(&evidence.source_unit) {
            Some(unit) => Ok((digest, unit)),
            None => self.fail_protocol(format!(
                "unsupported import source unit {}",
                evidence.source_unit
            )),
        }
    }

    pub(crate) fn inspect_step_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        part_index: Option<u32>,
        cancelled: &AtomicBool,
    ) -> Result<StepImportEvidence, WorkerError> {
        let request = WorkerRequest::InspectStepPart {
            source: source_file(source_sha256, path),
            part_index,
        };
        self.inspect_part(&request, path, source_sha256, cancelled)
    }

    pub(crate) fn inspect_iges_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        part_index: Option<u32>,
        cancelled: &AtomicBool,
    ) -> Result<StepImportEvidence, WorkerError> {
        let request = WorkerRequest::InspectIgesPart {
            source: source_file(source_sha256, path),
            part_index,
        };
        self.inspect_part(&request, path, source_sha256, cancelled)
    }

    fn inspect_part(
        &mut self,
        request: &WorkerRequest,
        path: &Path,
        source_sha256: &str,
        cancelled: &AtomicBool,
    ) -> Result<StepImportEvidence, WorkerError> {
        let part = match self.call(request, cancelled, DEFAULT_WORKER_REQUEST_TIMEOUT)? {
            WorkerReply::ImportPart(part) => part,
            reply => return self.unexpected(reply),
        };
        let source_sha256_bytes = decode_sha256(source_sha256)
            .ok_or_else(|| WorkerError::Protocol("invalid import source SHA-256".to_owned()))?;
        let source_byte_len = std::fs::metadata(path)
            .map_err(|error| WorkerError::Transport(error.to_string()))?
            .len();
        let WorkerImportPart {
            result_fingerprint,
            body_kind,
            topology_counts,
            area_mm2,
            volume_mm3,
            bounds_mm,
            source_unit,
            backend,
            tolerance,
        } = part;
        let (Some(body_kind), Some(source_unit)) = (
            parse_import_body_kind(&body_kind),
            parse_source_unit(&source_unit),
        ) else {
            return self.fail_protocol(format!(
                "invalid imported body kind {body_kind} or unit {source_unit}"
            ));
        };
        if !is_fnv1a64_digest(&result_fingerprint) {
            return self.fail_protocol(format!(
                "invalid imported result fingerprint {result_fingerprint}"
            ));
        }
        Ok(StepImportEvidence {
            source_sha256: source_sha256_bytes,
            source_byte_len,
            source_unit,
            result_fingerprint,
            body_kind,
            solid_count: topology_counts[4],
            topology_counts,
            area_mm2,
            volume_mm3,
            bounds_mm,
            backend,
            tolerance,
        })
    }

    pub(crate) fn tessellate_iges_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        result_fingerprint: &str,
        output_path: &Path,
        part_index: Option<u32>,
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        let request = WorkerRequest::TessellateIgesPart {
            source: source_file(source_sha256, path),
            part_index,
            output: output_path.to_owned(),
        };
        match self.call(&request, cancelled, DEFAULT_WORKER_REQUEST_TIMEOUT)? {
            WorkerReply::Mesh(receipt) => self.verified_mesh(
                receipt,
                result_fingerprint,
                output_path,
                "imported IGES display mesh",
            ),
            reply => self.unexpected(reply),
        }
    }

    pub(crate) fn tessellate_step_part_request_with_cancellation(
        &mut self,
        path: &Path,
        source_sha256: &str,
        result_fingerprint: &str,
        output_path: &Path,
        part_index: Option<u32>,
        cancelled: &AtomicBool,
    ) -> Result<StepImportMesh, WorkerError> {
        let request = WorkerRequest::TessellateStepPart {
            source: source_file(source_sha256, path),
            part_index,
            output: output_path.to_owned(),
        };
        match self.call(&request, cancelled, DEFAULT_WORKER_REQUEST_TIMEOUT)? {
            WorkerReply::Mesh(receipt) => self.verified_mesh(
                receipt,
                result_fingerprint,
                output_path,
                "imported STEP display mesh",
            ),
            reply => self.unexpected(reply),
        }
    }

    /// Binds a display mesh file to its receipt and the committed result fingerprint.
    fn verified_mesh(
        &mut self,
        receipt: MeshReceipt,
        result_fingerprint: &str,
        output_path: &Path,
        label: &str,
    ) -> Result<StepImportMesh, WorkerError> {
        if receipt.result_fingerprint != result_fingerprint || !is_sha256_digest(&receipt.sha256) {
            return self.fail_protocol(format!("{label} receipt {receipt:?} does not match"));
        }
        let encoded = read_step_import_mesh_output(
            output_path,
            receipt.vertex_count,
            receipt.triangle_count,
        )?;
        if sha256_hex(&encoded) != receipt.sha256 {
            return Err(WorkerError::Transport(format!(
                "{label} digest does not match the worker receipt"
            )));
        }
        let mesh = StepImportMesh::decode(&encoded)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        if mesh.vertices_mm.len() as u32 != receipt.vertex_count
            || mesh.triangles.len() as u32 != receipt.triangle_count
        {
            return Err(WorkerError::Transport(format!(
                "{label} size does not match the worker receipt"
            )));
        }
        Ok(mesh)
    }

    pub(crate) fn convert_step_to_iges_request_with_cancellation(
        &mut self,
        source_path: &Path,
        source_sha256: &str,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<Vec<u8>, WorkerError> {
        let request = WorkerRequest::ConvertStepXdeToIges {
            source: source_file(source_sha256, source_path),
            output: output_path.to_owned(),
        };
        match self.call(&request, cancelled, DEFAULT_WORKER_REQUEST_TIMEOUT)? {
            WorkerReply::Exported(ExportReceipt {
                result_fingerprint,
                sha256: None,
            }) if is_fnv1a64_digest(&result_fingerprint) => {}
            reply => return self.unexpected(reply),
        }
        self.read_bounded_output(
            output_path,
            MAX_STEP_SOURCE_BYTES,
            "exact worker IGES output exceeds the bounded 32 MiB envelope",
        )
    }

    pub(crate) fn export_step_xde_part_request_with_cancellation(
        &mut self,
        source_path: &Path,
        source_sha256: &str,
        part_index: u32,
        result_fingerprint: &str,
        output_path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        let request = WorkerRequest::ExportStepXdePart {
            source: source_file(source_sha256, source_path),
            part_index,
            result_fingerprint: result_fingerprint.to_owned(),
            output: output_path.to_owned(),
        };
        let sha256 = match self.call(&request, cancelled, DEFAULT_WORKER_REQUEST_TIMEOUT)? {
            WorkerReply::Exported(ExportReceipt {
                result_fingerprint: exported,
                sha256: Some(sha256),
            }) if exported == result_fingerprint && is_sha256_digest(&sha256) => sha256,
            reply => return self.unexpected(reply),
        };
        let bytes = self.read_bounded_output(
            output_path,
            MAX_STEP_SOURCE_BYTES,
            "exact worker STEP output exceeds the bounded 32 MiB envelope",
        )?;
        if sha256_hex(&bytes) != sha256 {
            return self.fail_protocol("worker STEP XDE part output hash mismatch".to_owned());
        }
        Ok(())
    }

    pub(crate) fn assemble_step_model_request_with_cancellation(
        &mut self,
        manifest: &StepAssemblyManifest,
        sources: &[PathBuf],
        path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<Vec<u8>, WorkerError> {
        if manifest.parts.len() != sources.len() {
            return Err(WorkerError::Protocol(
                "STEP assembly manifest/source count mismatch".to_owned(),
            ));
        }
        let request = WorkerRequest::AssembleStep {
            manifest: manifest.clone(),
            sources: sources.to_vec(),
            output: path.to_owned(),
        };
        let sha256 = match self.call(&request, cancelled, DEFAULT_WORKER_REQUEST_TIMEOUT)? {
            WorkerReply::Exported(ExportReceipt {
                result_fingerprint,
                sha256: Some(sha256),
            }) if is_fnv1a64_digest(&result_fingerprint) && is_sha256_digest(&sha256) => sha256,
            reply => return self.unexpected(reply),
        };
        let bytes = self.read_bounded_output(
            path,
            MAX_STEP_SOURCE_BYTES,
            "exact worker STEP output exceeds the bounded 32 MiB envelope",
        )?;
        if sha256_hex(&bytes) != sha256 {
            return self.fail_protocol("worker STEP output hash mismatch".to_owned());
        }
        Ok(bytes)
    }

    pub fn exception_probe(&mut self) -> Result<String, WorkerError> {
        match self.request_with_timeout(
            &WorkerRequest::Exception,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT,
        )? {
            WorkerReply::Failure(WorkerFailure { code, detail: None })
                if is_geometry_error_code(&code) =>
            {
                Ok(code)
            }
            reply => self.unexpected(reply),
        }
    }

    pub fn begin_killable_job(&mut self, duration: Duration) -> Result<(), WorkerError> {
        let deadline = Instant::now() + DEFAULT_WORKER_REQUEST_TIMEOUT;
        let milliseconds = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.write_request_until(&WorkerRequest::Sleep { milliseconds }, deadline)
    }

    pub fn crash(&mut self) -> Result<(), WorkerError> {
        let deadline = Instant::now() + DEFAULT_WORKER_REQUEST_TIMEOUT;
        self.write_request_until(&WorkerRequest::Crash, deadline)?;
        match self.next_response_until(deadline)? {
            WorkerResponse::Exited => self
                .child
                .wait()
                .map(|_| ())
                .map_err(|error| WorkerError::Transport(error.to_string())),
            response => self.fail_response(response),
        }
    }

    pub fn cancel(mut self) -> Result<Duration, WorkerError> {
        let started = Instant::now();
        // Terminate the job without waiting for asynchronous Windows teardown.

        child_process::terminate(&mut *self.child)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;

        Ok(started.elapsed())
    }

    /// Sends one request; a failure reply becomes an error and, unless it is a
    /// geometry refusal, retires the worker.
    pub(crate) fn call(
        &mut self,
        request: &WorkerRequest,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<WorkerReply, WorkerError> {
        match self.request_with_timeout(request, cancelled, timeout)? {
            WorkerReply::Failure(failure) => {
                let error = failure_error(failure);
                if matches!(error, WorkerError::Geometry(_)) {
                    Err(error)
                } else {
                    self.fail(error)
                }
            }
            reply => Ok(reply),
        }
    }

    /// Like [`Self::call`] for requests whose only success is `Done`.
    pub(crate) fn call_done(
        &mut self,
        request: &WorkerRequest,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<(), WorkerError> {
        match self.call(request, cancelled, timeout)? {
            WorkerReply::Done => Ok(()),
            reply => self.unexpected(reply),
        }
    }

    pub(crate) fn request_with_timeout(
        &mut self,
        request: &WorkerRequest,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<WorkerReply, WorkerError> {
        // Writing and receiving share one budget; cancellation still polls at 10 ms.
        let deadline = Instant::now() + timeout;
        self.write_request_until_with_cancellation(request, deadline, cancelled, timeout)?;
        match self.next_response_until_with_cancellation(deadline, cancelled, timeout)? {
            WorkerResponse::Reply(reply) => Ok(*reply),
            WorkerResponse::Exited => self.fail(WorkerError::WorkerExited),
            response => self.fail_response(response),
        }
    }

    fn fail_response<T>(&mut self, response: WorkerResponse) -> Result<T, WorkerError> {
        match response {
            WorkerResponse::Reply(reply) => self.unexpected(*reply),
            WorkerResponse::Exited => self.fail(WorkerError::WorkerExited),
            WorkerResponse::TooLarge => self.fail(WorkerError::ResponseTooLarge {
                max_bytes: MAX_REPLY_FRAME_BYTES,
            }),
            WorkerResponse::Malformed(message) => {
                self.fail(WorkerError::MalformedTransport(message))
            }
            WorkerResponse::Transport(message) => self.fail(WorkerError::Transport(message)),
        }
    }

    fn write_request_until(
        &mut self,
        request: &WorkerRequest,
        deadline: Instant,
    ) -> Result<(), WorkerError> {
        self.write_request_until_with_cancellation(
            request,
            deadline,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT,
        )
    }

    fn write_request_until_with_cancellation(
        &mut self,
        request: &WorkerRequest,
        deadline: Instant,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<(), WorkerError> {
        self.ensure_not_cancelled(cancelled)?;
        let frame = encode_frame(request, MAX_REQUEST_FRAME_BYTES)
            .map_err(|error| WorkerError::Protocol(error.to_string()))?;
        let (acknowledgment, receiver) = mpsc::channel();
        if self
            .write_sender
            .send(WorkerWriteRequest {
                frame,
                acknowledgment,
            })
            .is_err()
        {
            self.ensure_not_cancelled(cancelled)?;
            return self.fail(WorkerError::MalformedTransport(
                "worker request writer disconnected".to_owned(),
            ));
        }
        loop {
            self.ensure_not_cancelled(cancelled)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return self.fail(WorkerError::RequestTimedOut(timeout));
            }
            match receiver.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
                Ok(result) => {
                    self.ensure_not_cancelled(cancelled)?;
                    return match result {
                        Ok(()) => Ok(()),
                        Err(message) => self.fail(WorkerError::Transport(message)),
                    };
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.ensure_not_cancelled(cancelled)?;
                    return self.fail(WorkerError::MalformedTransport(
                        "worker request writer disconnected before acknowledging the write"
                            .to_owned(),
                    ));
                }
            }
        }
    }

    fn next_response_until(&mut self, deadline: Instant) -> Result<WorkerResponse, WorkerError> {
        self.next_response_until_with_cancellation(
            deadline,
            &NEVER_CANCELLED,
            DEFAULT_WORKER_REQUEST_TIMEOUT,
        )
    }

    fn next_response_until_with_cancellation(
        &mut self,
        deadline: Instant,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<WorkerResponse, WorkerError> {
        loop {
            self.ensure_not_cancelled(cancelled)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return self.fail(WorkerError::RequestTimedOut(timeout));
            }
            match self
                .response_receiver
                .recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL))
            {
                Ok(response) => {
                    self.ensure_not_cancelled(cancelled)?;
                    return Ok(response);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.ensure_not_cancelled(cancelled)?;
                    return self.fail(WorkerError::MalformedTransport(
                        "worker response reader disconnected without a terminal event".to_owned(),
                    ));
                }
            }
        }
    }

    pub(crate) fn ensure_not_cancelled(
        &mut self,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        if cancelled.load(Ordering::Acquire) {
            self.fail(WorkerError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn unexpected<T>(&mut self, reply: WorkerReply) -> Result<T, WorkerError> {
        self.fail(unexpected_reply(&reply))
    }

    fn fail_protocol<T>(&mut self, response: String) -> Result<T, WorkerError> {
        self.fail(WorkerError::Protocol(response))
    }

    fn fail<T>(&mut self, error: WorkerError) -> Result<T, WorkerError> {
        self.terminate_worker();
        Err(error)
    }

    pub(crate) fn terminate_worker(&mut self) {
        self.terminated = true; // Never reuse a job whose termination has started.
        let _ = child_process::terminate(&mut *self.child);
    }
}

pub(crate) fn unexpected_reply(reply: &WorkerReply) -> WorkerError {
    let summary = format!("{reply:?}").chars().take(240).collect::<String>();
    WorkerError::Protocol(format!("unexpected worker reply {summary}"))
}

fn failure_error(failure: WorkerFailure) -> WorkerError {
    if !is_geometry_error_code(&failure.code) {
        return WorkerError::Protocol(format!("worker refused the request: {}", failure.code));
    }
    match failure.detail {
        None => WorkerError::Geometry(failure.code),
        Some(detail) => WorkerError::Geometry(format!(
            "{}; operation={}; diagnostic={}; input_digest={}; backend={}",
            failure.code, detail.operation, detail.diagnostic, detail.input_digest, detail.backend
        )),
    }
}

fn validated_graph(graph: &ExactBRepGraph) -> Result<(), WorkerError> {
    graph
        .validate()
        .map_err(|error| WorkerError::Protocol(error.to_string()))
}

fn graph_input(
    graph: &ExactBRepGraph,
    imported_sources: &[(&str, &Path)],
) -> Result<GraphInput, WorkerError> {
    validated_graph(graph)?;
    Ok(GraphInput {
        graph: graph.clone(),
        sources: imported_sources
            .iter()
            .map(|(sha256, path)| source_file(sha256, path))
            .collect(),
    })
}

pub(crate) fn source_file(sha256: &str, path: &Path) -> SourceFile {
    SourceFile {
        sha256: sha256.to_owned(),
        path: path.to_owned(),
    }
}

fn parse_source_unit(unit: &str) -> Option<ImportLengthUnit> {
    match unit {
        "millimetre" => Some(ImportLengthUnit::Millimetre),
        "centimetre" => Some(ImportLengthUnit::Centimetre),
        "metre" => Some(ImportLengthUnit::Metre),
        "inch" => Some(ImportLengthUnit::Inch),
        "foot" => Some(ImportLengthUnit::Foot),
        _ => None,
    }
}

fn xde_part_evidence(
    part: StepXdeWorkerPart,
    source_sha256: [u8; 32],
    source_byte_len: u64,
    source_unit: ImportLengthUnit,
) -> StepImportEvidence {
    StepImportEvidence {
        source_sha256,
        source_byte_len,
        source_unit,
        result_fingerprint: part.result_fingerprint,
        body_kind: parse_import_body_kind(&part.body_kind).expect("worker body kind was validated"),
        solid_count: part.solid_count,
        topology_counts: part.topology_counts,
        area_mm2: part.area_mm2,
        volume_mm3: part.volume_mm3,
        bounds_mm: part.bounds_mm,
        backend: part.backend,
        tolerance: part.tolerance,
    }
}

fn spawn_worker_writer(mut stdin: ChildStdin, receiver: Receiver<WorkerWriteRequest>) {
    let _ = std::thread::spawn(move || {
        while let Ok(request) = receiver.recv() {
            let result = stdin
                .write_all(&request.frame)
                .and_then(|()| stdin.flush())
                .map_err(|error| error.to_string());
            let failed = result.is_err();
            let _ = request.acknowledgment.send(result);
            if failed {
                break;
            }
        }
    });
}

fn spawn_worker_reader(stdout: ChildStdout, sender: mpsc::SyncSender<WorkerResponse>) {
    let _ = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let response = match read_frame::<WorkerReply>(&mut reader, MAX_REPLY_FRAME_BYTES) {
                Frame::Message(reply) => WorkerResponse::Reply(Box::new(reply)),
                Frame::Closed => WorkerResponse::Exited,
                Frame::TooLarge => WorkerResponse::TooLarge,
                Frame::Malformed(message) => WorkerResponse::Malformed(message),
                Frame::Transport(message) => WorkerResponse::Transport(message),
            };
            let terminal = !matches!(response, WorkerResponse::Reply(_));
            if sender.send(response).is_err() || terminal {
                break;
            }
        }
    });
}

impl Drop for ExactWorkerClient {
    fn drop(&mut self) {
        // Do not block cancellation on Windows kernel teardown.
        let _ = child_process::terminate(&mut *self.child);
    }
}
