#![forbid(unsafe_code)]

use crate::protocol::{
    ExportReceipt, FailureDetail, Frame, GraphInput, MAX_REPLY_FRAME_BYTES,
    MAX_REQUEST_FRAME_BYTES, MeshReceipt, SourceFile, VolumeMeshReceipt, WorkerFailure,
    WorkerImportPart, WorkerReply, WorkerRequest, protocol_identity, read_frame, write_frame,
};
use crate::{
    CamSimulationWireCollision, CamSimulationWireEvidence, CamSimulationWireRequest,
    EXACT_VOLUME_MESH_WIRE_SCHEMA_V1, ExactVolumeMeshWireOptions,
    MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES, StepAssemblyManifest, StepXdeWorkerEvidence,
    StepXdeWorkerNode, StepXdeWorkerPart, WorkerExactBRepGraphEdgeEvidence,
    WorkerExactBRepGraphFaceEvidence, WorkerExactBRepGraphResult,
    WorkerExactVolumeBoundaryTriangle, WorkerExactVolumeMesh,
};
use ketchup_exact::naming::{NamedBoolean, NamedSegment, valid_name};
use ketchup_exact::{
    AdvancedChamferMode, AxialToolMotion, AxialToolSweepSpec, BoxSpec, CircleExtrudeSpec,
    EdgeFinish, ExactBackend, ExactBodyBooleanOperation, ExactKernel, ExactOpOutput,
    ExactPairRelation, ExactVolumeMeshOptions, FramedLoftProfile, FramedLoftSection,
    FramedLoftSpec, LoftSurfaceContinuity, PlanarProfileLoop, PlanarProfileSegment, Point3,
    RectangleOffsetSpec, ShellDirection as NativeShellDirection, Size3, SpatialProfileSegment,
    StepXdeExportNode, StepXdeExportPart, ThreadGrooveSpec,
};
use ketchup_geometry::linalg::{self, Affine3, Vec3};
use ketchup_model::cam::CAM_SIMULATION_SCHEMA_V1;
use ketchup_model::document::Transform;
use ketchup_model::exact_brep_graph::{
    ExactBRepBooleanOperation, ExactBRepChamferMode, ExactBRepDraft, ExactBRepEdgeFinishKind,
    ExactBRepGraph, ExactBRepImportedFormat, ExactBRepLinearInterval, ExactBRepLoftContinuity,
    ExactBRepLoftSection, ExactBRepOperation, ExactBRepPlanarGeometry, ExactBRepPlanarLoop,
    ExactBRepPlanarSegment, ExactBRepProfile, ExactBRepProfileFaceReference,
    ExactBRepShellDirection, ExactBRepSpatialPath, ExactBRepSpatialPathSegment,
    ExactBRepThreadGroove, ExactBRepTopologyKind, ExactBRepTopologySelector,
    ExactBRepWeldmentJointPolicy, ExactBRepWeldmentJointPrimary,
    exact_brep_planar_rectangle_bounds, exact_sheet_metal_shape,
};
use ketchup_model::exact_product::{
    EXACT_BREP_GRAPH_EVALUATOR_V1, ExactConeFaceEvidence, ExactFaceRole,
};
use ketchup_model::graph::{is_sha256_hex, sha256_hex};
use ketchup_model::import::{MAX_STEP_MESH_TRIANGLES, StepImportMesh, StepMeshTriangle};
use ketchup_model::sheet_metal::{SheetMetalError, SheetMetalShape};
use ketchup_model::tolerance::limits;
use ketchup_model::tolerance::{APPROXIMATION, DEFAULT_LINEAR_TOLERANCE_MM, MAX_COORDINATE_MM};
use ketchup_model::topology::{
    TopologicalElementRef, TopologicalReferenceStability, topological_edge_provenance_tokens,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

type WorkerResult = Result<WorkerReply, WorkerFailure>;

mod output_cache;
#[path = "bin/pair_query/mod.rs"]
mod pair_query;
mod source_document;

use source_document::{SourceDocument, XdeFormat};

pub fn run_stdio() {
    crate::child_process::exit_with_parent();
    if !ExactBackend::route_kernel_messages_to_stderr() {
        // A kernel message on stdout would corrupt the reply frames.
        eprintln!("exact worker: kernel messages could not be moved off stdout");
        std::process::exit(2);
    }
    let backend = ExactBackend::new();
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut pairs = pair_query::PairQuerySession::default();
    let mut source = SourceDocument::default();
    loop {
        let reply = match read_frame::<WorkerRequest>(&mut stdin, MAX_REQUEST_FRAME_BYTES) {
            Frame::Message(request) => handle_request(&backend, &mut pairs, &mut source, request),
            // A complete frame that does not decode leaves the stream in sync.
            Frame::Malformed(_) => Err(WorkerFailure::invalid_request()),
            Frame::Closed | Frame::TooLarge | Frame::Transport(_) => break,
        };
        let reply = reply.unwrap_or_else(WorkerReply::Failure);
        let written = match write_frame(&mut stdout, &reply, MAX_REPLY_FRAME_BYTES) {
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                write_frame(&mut stdout, &reply.too_large(), MAX_REPLY_FRAME_BYTES)
            }
            written => written,
        };
        if written.is_err() {
            break;
        }
    }
}

fn handle_request(
    backend: &ExactBackend,
    pairs: &mut pair_query::PairQuerySession,
    read: &mut SourceDocument,
    request: WorkerRequest,
) -> WorkerResult {
    match request {
        WorkerRequest::Hello => Ok(WorkerReply::Hello {
            protocol: protocol_identity().to_owned(),
        }),
        WorkerRequest::EvaluateGraph(input) => exact_brep_graph_response(backend, &input),
        WorkerRequest::TessellateGraph {
            input,
            result_fingerprint,
            output,
        } => exact_brep_graph_mesh_response(backend, &input, &result_fingerprint, &output),
        WorkerRequest::VolumeMeshGraph {
            input,
            result_fingerprint,
            options,
            output,
        } => exact_brep_graph_volume_mesh_response(
            backend,
            &input,
            &result_fingerprint,
            options,
            &output,
        ),
        WorkerRequest::ExportGraphStep {
            input,
            result_fingerprint,
            output,
        } => exact_brep_graph_step_response(backend, &input, &result_fingerprint, &output),
        WorkerRequest::SimulateCam { graph, request } => {
            cam_simulation_response(backend, &graph, &request)
        }
        WorkerRequest::InspectStepXde(source) => {
            m21_step_xde_inspection_response(backend, read, &source)
        }
        WorkerRequest::InspectStepPart { source, part_index } => {
            m21_step_part_inspection_response(backend, read, &source, part_index)
        }
        WorkerRequest::TessellateStepPart {
            source,
            part_index,
            output,
        } => m21_step_part_mesh_response(backend, read, &source, part_index, &output),
        WorkerRequest::ExportStepXdePart {
            source,
            part_index,
            result_fingerprint,
            output,
        } => m21_step_xde_part_export_response(
            backend,
            read,
            &source,
            part_index,
            &result_fingerprint,
            &output,
        ),
        WorkerRequest::ConvertStepXdeToIges { source, output, .. } => {
            m21_step_xde_to_iges_response(backend, read, &source, &output)
        }
        WorkerRequest::InspectIgesXde(source) => {
            m21_iges_xde_inspection_response(backend, read, &source)
        }
        WorkerRequest::InspectIgesPart { source, part_index } => {
            m21_iges_part_inspection_response(backend, read, &source, part_index)
        }
        WorkerRequest::TessellateIgesPart {
            source,
            part_index,
            output,
        } => m21_iges_part_mesh_response(backend, read, &source, part_index, &output),
        WorkerRequest::AssembleStep {
            manifest,
            sources,
            output,
        } => m21_step_assembly_response(backend, &manifest, &sources, &output),
        WorkerRequest::PairBegin => pairs.begin(),
        WorkerRequest::PairLoad(input) => pairs.load(backend, &input),
        WorkerRequest::PairQuery(query) => pairs.query(backend, &query),
        WorkerRequest::PairEnd => pairs.end(),
        // Programs and DWG files go to ketchup-program-worker.
        WorkerRequest::EvaluateProgram(_)
        | WorkerRequest::ReportProgram(_)
        | WorkerRequest::ReportBeforeExact(_)
        | WorkerRequest::RefineReport(_)
        | WorkerRequest::ConvertDwg(_)
        | WorkerRequest::ReadUnderlay(_)
        | WorkerRequest::WriteUnderlay(_) => Err(WorkerFailure::invalid_request()),
        WorkerRequest::Exception => match backend.exception_probe() {
            Ok(_) => Err(WorkerFailure::code("unexpected_success")),
            Err(error) => Err(WorkerFailure::code(error.code.as_str())),
        },
        WorkerRequest::Sleep { milliseconds } => {
            std::thread::sleep(Duration::from_millis(milliseconds));
            Ok(WorkerReply::Done)
        }
        // Ends without a reply like a crash, but without abort(): on Windows every abort
        // files a Windows Error Reporting report and holds the process ~1.5 s.
        WorkerRequest::Crash => std::process::exit(1),
        WorkerRequest::Allocate { bytes } => {
            let block = vec![0_u8; usize::try_from(bytes).unwrap_or(usize::MAX)];
            std::hint::black_box(&block);
            Ok(WorkerReply::Done)
        }
    }
}

/// The typed graph arrives decoded, so its structure and digest are re-proven here.
fn validated_graph(graph: &ExactBRepGraph) -> Result<(), WorkerFailure> {
    graph.validate().map_err(|error| WorkerFailure {
        detail: Some(FailureDetail {
            diagnostic: error.to_string(),
            operation: "validate_exact_brep_graph".to_owned(),
            input_digest: graph.graph_digest.clone(),
            backend: ketchup_exact::backend_fingerprint().to_owned(),
        }),
        ..WorkerFailure::invalid_request()
    })
}

fn utf8_path(path: &Path) -> Result<&str, WorkerFailure> {
    path.to_str()
        .filter(|path| !path.is_empty())
        .ok_or_else(WorkerFailure::invalid_request)
}

fn verified_exact_brep_graph_sources(
    graph: &ExactBRepGraph,
    supplied: &[SourceFile],
) -> Result<Vec<(String, tempfile::NamedTempFile)>, WorkerFailure> {
    let mut expected = BTreeMap::<String, (u64, ExactBRepImportedFormat)>::new();
    let mut source_order = Vec::new();
    for node in &graph.nodes {
        let ExactBRepOperation::ImportedExact {
            source_sha256,
            source_byte_len,
            source_format,
            ..
        } = &node.operation
        else {
            continue;
        };
        let source_sha256 = source_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if let Some(previous) = expected.get(&source_sha256) {
            if *previous != (*source_byte_len, *source_format) {
                return Err(WorkerFailure::invalid_request());
            }
        } else {
            expected.insert(source_sha256.clone(), (*source_byte_len, *source_format));
            source_order.push(source_sha256);
        }
    }
    if source_order.len() > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES
        || supplied.len() != source_order.len()
    {
        return Err(WorkerFailure::invalid_request());
    }
    let mut sources = Vec::with_capacity(source_order.len());
    for (expected_sha256, source) in source_order.iter().zip(supplied) {
        if source.sha256 != *expected_sha256 {
            return Err(WorkerFailure::invalid_request());
        }
        let (expected_byte_len, format) = expected[expected_sha256];
        let label = match format {
            ExactBRepImportedFormat::Step => "STEP",
            ExactBRepImportedFormat::Iges => "IGES",
        };
        let copy = verified_copy(source, label, "evaluate_exact_brep_graph")?;
        if copy
            .as_file()
            .metadata()
            .map_err(|error| backend_failure("evaluate_exact_brep_graph", &error.to_string()))?
            .len()
            != expected_byte_len
        {
            return Err(backend_failure(
                "evaluate_exact_brep_graph",
                &format!("{label} source byte length does not match the exact graph identity"),
            ));
        }
        sources.push((expected_sha256.clone(), copy));
    }
    Ok(sources)
}

/// Validates the graph, stages its sources and evaluates it through the output cache.
fn evaluated_graph_input(
    backend: &ExactBackend,
    input: &GraphInput,
) -> Result<std::rc::Rc<ExactOpOutput>, WorkerFailure> {
    validated_graph(&input.graph)?;
    let sources = verified_exact_brep_graph_sources(&input.graph, &input.sources)?;
    evaluate_exact_brep_graph_cached(backend, &input.graph, &sources)
        .map_err(|error| geometry_failure(&error))
}

fn exact_brep_graph_mesh_response(
    backend: &ExactBackend,
    input: &GraphInput,
    result_fingerprint: &str,
    output_path: &Path,
) -> WorkerResult {
    // The client asks for topology evidence first; reuse that evaluation
    // instead of rebuilding the whole feature chain a second time.
    let output = evaluated_graph_input(backend, input)?;
    if output.body.result_fingerprint != result_fingerprint {
        return Err(WorkerFailure::code("invalid_result"));
    }
    let bounds = output.body.topology.bounds_mm;
    let diagonal = ((bounds.max.x - bounds.min.x).powi(2)
        + (bounds.max.y - bounds.min.y).powi(2)
        + (bounds.max.z - bounds.min.z).powi(2))
    .sqrt();
    if !diagonal.is_finite() || diagonal <= 0.0 {
        return Err(WorkerFailure::code("invalid_shape"));
    }
    let deflection =
        (diagonal * DISPLAY_MESH_RELATIVE_DEFLECTION).max(DISPLAY_MESH_MIN_DEFLECTION_MM);
    let mesh = backend
        .tessellate_body(
            &output.body,
            deflection,
            STEP_MESH_ANGULAR_DEFLECTION,
            u32::try_from(limits::MESH_TRIANGLES).unwrap_or(u32::MAX),
        )
        .map_err(|error| geometry_failure(&error))?;
    write_display_mesh(
        mesh,
        output_path,
        &output.body.result_fingerprint,
        "tessellate_exact_brep_graph",
    )
}

/// Writes a display mesh to the caller's file and describes it by count and digest.
fn write_display_mesh(
    mesh: ketchup_exact::ExactTessellation,
    output_path: &Path,
    result_fingerprint: &str,
    operation: &str,
) -> WorkerResult {
    let mesh = StepImportMesh {
        vertices_mm: mesh.vertices_mm,
        triangles: mesh
            .triangles
            .into_iter()
            .map(|triangle| StepMeshTriangle {
                vertex_indices: triangle.vertex_indices,
                face_ordinal: triangle.face_ordinal,
            })
            .collect(),
    };
    let encoded = mesh.encode();
    std::fs::write(output_path, &encoded)
        .map_err(|error| backend_failure(operation, &error.to_string()))?;
    Ok(WorkerReply::Mesh(MeshReceipt {
        result_fingerprint: result_fingerprint.to_owned(),
        vertex_count: mesh.vertices_mm.len() as u32,
        triangle_count: mesh.triangles.len() as u32,
        sha256: sha256_hex(&encoded),
    }))
}

fn exact_brep_graph_volume_mesh_response(
    backend: &ExactBackend,
    input: &GraphInput,
    result_fingerprint: &str,
    options: ExactVolumeMeshWireOptions,
    output_path: &Path,
) -> WorkerResult {
    validated_graph(&input.graph)?;
    let graph = &input.graph;
    let sources = verified_exact_brep_graph_sources(graph, &input.sources)?;
    let output =
        evaluate_exact_brep_graph(backend, graph, &sources).map_err(|e| geometry_failure(&e))?;
    if output.body.result_fingerprint != result_fingerprint {
        return Err(WorkerFailure::code("invalid_result"));
    }
    let mesh = backend
        .volume_mesh_body(
            &output.body,
            ExactVolumeMeshOptions {
                surface_deflection_mm: options.surface_deflection_mm,
                angular_deflection_rad: options.angular_deflection_rad,
                max_tetrahedra: options.max_tetrahedra,
                max_relative_volume_error: options.max_relative_volume_error,
                min_tetrahedron_quality: options.min_tetrahedron_quality,
            },
        )
        .map_err(|error| geometry_failure(&error))?;
    let mesh = WorkerExactVolumeMesh {
        schema: EXACT_VOLUME_MESH_WIRE_SCHEMA_V1.to_owned(),
        graph_digest: graph.graph_digest.clone(),
        source_result_fingerprint: mesh.source_result_fingerprint,
        request_digest: mesh.request_digest,
        mesh_fingerprint: mesh.mesh_fingerprint,
        vertices_mm: mesh.vertices_mm,
        tetrahedra: mesh
            .tetrahedra
            .into_iter()
            .map(|tetrahedron| tetrahedron.vertex_indices)
            .collect(),
        boundary_triangles: mesh
            .boundary_triangles
            .into_iter()
            .map(|triangle| WorkerExactVolumeBoundaryTriangle {
                vertex_indices: triangle.vertex_indices,
                face_ordinal: triangle.face_ordinal,
            })
            .collect(),
        exact_volume_mm3: mesh.exact_volume_mm3,
        tetrahedral_volume_mm3: mesh.tetrahedral_volume_mm3,
        relative_volume_error: mesh.relative_volume_error,
        minimum_signed_volume_mm3: mesh.minimum_signed_volume_mm3,
        minimum_quality: mesh.minimum_quality,
        maximum_edge_ratio: mesh.maximum_edge_ratio,
    };
    let encoded = serde_json::to_vec(&mesh)
        .map_err(|error| backend_failure("volume_mesh_exact_brep_graph", &error.to_string()))?;
    std::fs::write(output_path, &encoded)
        .map_err(|error| backend_failure("volume_mesh_exact_brep_graph", &error.to_string()))?;
    Ok(WorkerReply::VolumeMesh(VolumeMeshReceipt {
        result_fingerprint: output.body.result_fingerprint.clone(),
        vertex_count: mesh.vertices_mm.len() as u32,
        tetrahedron_count: mesh.tetrahedra.len() as u32,
        boundary_triangle_count: mesh.boundary_triangles.len() as u32,
        sha256: sha256_hex(&encoded),
        mesh_fingerprint: mesh.mesh_fingerprint,
    }))
}

fn exact_brep_graph_step_response(
    backend: &ExactBackend,
    input: &GraphInput,
    result_fingerprint: &str,
    output_path: &Path,
) -> WorkerResult {
    validated_graph(&input.graph)?;
    let sources = verified_exact_brep_graph_sources(&input.graph, &input.sources)?;
    let output = evaluate_exact_brep_graph(backend, &input.graph, &sources)
        .map_err(|error| geometry_failure(&error))?;
    if output.body.result_fingerprint != result_fingerprint {
        return Err(WorkerFailure::code("invalid_result"));
    }
    backend
        .export_step(&output.body, utf8_path(output_path)?)
        .map_err(|error| geometry_failure(&error))?;
    Ok(WorkerReply::Exported(ExportReceipt {
        result_fingerprint: output.body.result_fingerprint.clone(),
        sha256: None,
    }))
}

fn exact_brep_graph_face_evidence(output: &ExactOpOutput) -> Vec<WorkerExactBRepGraphFaceEvidence> {
    let mut mappings = BTreeMap::<(String, String), BTreeSet<u32>>::new();
    for history in &output.topology_history {
        let (Some(semantic_role), Some(face_ordinal)) =
            (history.semantic_role.as_ref(), history.output_face_ordinal)
        else {
            continue;
        };
        mappings
            .entry((semantic_role.clone(), history.source_element_id.clone()))
            .or_default()
            .insert(face_ordinal);
    }
    let mut named = mappings
        .into_iter()
        .filter_map(|((role, source), ordinals)| {
            (ordinals.len() == 1).then(|| (role, source, *ordinals.first().expect("one face")))
        })
        .collect::<Vec<_>>();
    for face in &output.body.topology.faces {
        if !named.iter().any(|(_, _, ordinal)| *ordinal == face.ordinal) {
            named.push((String::new(), String::new(), face.ordinal));
        }
    }
    named
        .into_iter()
        .filter_map(|(semantic_role, source_element_id, face_ordinal)| {
            let face = output
                .body
                .topology
                .faces
                .iter()
                .find(|face| face.ordinal == face_ordinal)?;
            Some(WorkerExactBRepGraphFaceEvidence {
                semantic_role,
                source_element_id,
                face_ordinal,
                surface_kind: face.surface_kind.clone(),
                geometric_fingerprint: face.geometric_fingerprint.clone(),
                centroid_mm: [face.centroid_mm.x, face.centroid_mm.y, face.centroid_mm.z],
                unit_normal: [face.normal.x, face.normal.y, face.normal.z],
                axis_origin_mm: face
                    .axis_origin_mm
                    .map(|origin| [origin.x, origin.y, origin.z]),
                unit_axis_direction: face
                    .axis_direction
                    .map(|direction| [direction.x, direction.y, direction.z]),
                cone: face.cone.map(|cone| ExactConeFaceEvidence {
                    unit_axis_direction: [
                        cone.axis_direction.x,
                        cone.axis_direction.y,
                        cone.axis_direction.z,
                    ],
                    unit_normal: [cone.normal.x, cone.normal.y, cone.normal.z],
                }),
            })
        })
        .collect()
}

fn exact_brep_graph_edge_evidence(output: &ExactOpOutput) -> Vec<WorkerExactBRepGraphEdgeEvidence> {
    output
        .body
        .topology
        .edges
        .iter()
        .map(|edge| WorkerExactBRepGraphEdgeEvidence {
            edge_ordinal: edge.ordinal,
            curve_kind: edge.curve_kind.clone(),
            length_mm: edge.length_mm,
            centroid_mm: [edge.centroid_mm.x, edge.centroid_mm.y, edge.centroid_mm.z],
            bounds_mm: [
                [
                    edge.bounds_mm.min.x,
                    edge.bounds_mm.min.y,
                    edge.bounds_mm.min.z,
                ],
                [
                    edge.bounds_mm.max.x,
                    edge.bounds_mm.max.y,
                    edge.bounds_mm.max.z,
                ],
            ],
            closed: edge.closed,
            circle_radius_mm: edge.circle_radius_mm,
            axis_origin_mm: edge
                .axis_origin_mm
                .map(|origin| [origin.x, origin.y, origin.z]),
            unit_axis_direction: edge
                .axis_direction
                .map(|direction| [direction.x, direction.y, direction.z]),
            adjacent_face_ordinals: edge.adjacent_face_ordinals.clone(),
        })
        .collect()
}

fn exact_brep_graph_response(backend: &ExactBackend, input: &GraphInput) -> WorkerResult {
    let output = evaluated_graph_input(backend, input)?;
    let graph = &input.graph;
    let topology = &output.body.topology;
    let area_mm2 = if graph.terminal_is_surface() {
        let area = topology.faces.iter().map(|face| face.area_mm2).sum::<f64>();
        if topology.face_count == 0
            || topology.faces.len() != topology.face_count as usize
            || !area.is_finite()
            || area <= 0.0
        {
            return Err(WorkerFailure::code("invalid_shape"));
        }
        area
    } else {
        0.0
    };
    Ok(WorkerReply::Graph(WorkerExactBRepGraphResult {
        canonical_input_digest: graph.canonical_input_digest.clone(),
        graph_digest: graph.graph_digest.clone(),
        producer_feature_id: graph.producer_feature_id,
        result_fingerprint: output.body.result_fingerprint.clone(),
        exact_input_digest: output.input_digest.clone(),
        volume_mm3: topology.volume_mm3,
        mass: topology.mass.filter(|_| !graph.terminal_is_surface()),
        area_mm2,
        bounds_mm: [
            topology.bounds_mm.min.x,
            topology.bounds_mm.min.y,
            topology.bounds_mm.min.z,
            topology.bounds_mm.max.x,
            topology.bounds_mm.max.y,
            topology.bounds_mm.max.z,
        ],
        topology_counts: [
            topology.vertex_count,
            topology.edge_count,
            topology.face_count,
            topology.shell_count,
            topology.solid_count,
        ],
        wire_count: Some(topology.wire_count),
        backend: output.backend_fingerprint.to_owned(),
        tolerance: output.tolerance_report.profile.to_owned(),
        faces: exact_brep_graph_face_evidence(&output),
        edges: exact_brep_graph_edge_evidence(&output),
    }))
}

fn exact_brep_sheet_metal(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    sheet: &SheetMetalShape,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let invalid = |error: SheetMetalError| exact_brep_graph_error(graph, &error.to_string());
    let folded = sheet.fold().map_err(invalid)?;
    let polygon = |corners: Vec<[f64; 2]>| {
        corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .map(|(start, end)| PlanarProfileSegment::Line {
                start_mm: *start,
                end_mm: *end,
            })
            .collect::<Vec<_>>()
    };
    let placed = |profile: &[PlanarProfileSegment], depth: f64, frame: linalg::Frame| {
        let local = backend.extrude_mixed_profile(profile, depth)?;
        backend.transform_body(&local.body, &frame.to_affine().to_row_major())
    };
    let base = sheet.face_corners(None).map_err(invalid)?;
    let mut output = backend.extrude_mixed_profile(&polygon(base), sheet.thickness_mm)?;
    for (index, bend) in folded.iter().enumerate() {
        let arc = bend.arc;
        let section = vec![
            PlanarProfileSegment::CircularArc {
                start_mm: arc.outer_start_mm,
                end_mm: arc.outer_end_mm,
                center_mm: arc.center_mm,
                clockwise: arc.clockwise,
            },
            PlanarProfileSegment::Line {
                start_mm: arc.outer_end_mm,
                end_mm: arc.inner_end_mm,
            },
            PlanarProfileSegment::CircularArc {
                start_mm: arc.inner_end_mm,
                end_mm: arc.inner_start_mm,
                center_mm: arc.center_mm,
                clockwise: !arc.clockwise,
            },
            PlanarProfileSegment::Line {
                start_mm: arc.inner_start_mm,
                end_mm: arc.outer_start_mm,
            },
        ];
        let flange = sheet.face_corners(Some(index)).map_err(invalid)?;
        for (profile, depth, frame) in [
            (section, bend.span_mm, bend.frame),
            (polygon(flange), sheet.thickness_mm, bend.flange),
        ] {
            let part = placed(&profile, depth, frame)?;
            output = backend.boolean_bodies(
                &output.body,
                &part.body,
                ExactBodyBooleanOperation::Union,
            )?;
        }
    }
    Ok(output)
}

fn cam_simulation_response(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    request: &CamSimulationWireRequest,
) -> WorkerResult {
    validated_graph(graph)?;
    let bounded_values = request
        .stock_bounds_mm
        .iter()
        .flatten()
        .chain(request.setup_to_world.iter())
        .copied()
        .chain([
            request.cutter_radius_mm,
            request.cutter_length_mm,
            request.tool_length_mm,
            request.holder_radius_mm,
            request.holder_offset_mm,
            request.holder_length_mm,
        ])
        .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE_MM);
    let bounded_pairs = request
        .motions
        .len()
        .checked_mul(request.fixtures.len().saturating_mul(2).saturating_add(2))
        .is_some_and(|checks| checks <= crate::MAX_CAM_SIMULATION_PAIR_CHECKS);
    if request.schema != CAM_SIMULATION_SCHEMA_V1
        || request.target_exact_graph_digest != graph.graph_digest
        || !is_sha256_hex(&request.plan_digest)
        || !is_sha256_hex(&request.toolpath_digest)
        || request.motions.is_empty()
        || request.motions.len() > crate::MAX_CAM_SIMULATION_MOTIONS
        || request.fixtures.len() > crate::MAX_CAM_SIMULATION_FIXTURES
        || !bounded_pairs
        || !bounded_values
        || request.cutter_radius_mm <= 0.0
        || request.cutter_length_mm <= 0.0
        || request.tool_length_mm < request.cutter_length_mm
        || request.holder_radius_mm <= 0.0
        || request.holder_offset_mm < request.tool_length_mm
        || request.holder_length_mm <= 0.0
        || (0..3).any(|axis| request.stock_bounds_mm[0][axis] >= request.stock_bounds_mm[1][axis])
        || request.motions.iter().any(|motion| {
            motion.kind > 3
                || motion.path_kind > 1
                || motion
                    .start_mm
                    .into_iter()
                    .chain(motion.end_mm)
                    .chain(motion.center_mm)
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        })
        || request.fixtures.iter().any(|fixture| {
            fixture.id == 0
                || (0..3).any(|axis| fixture.bounds_mm[0][axis] >= fixture.bounds_mm[1][axis])
                || fixture
                    .bounds_mm
                    .iter()
                    .flatten()
                    .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        })
        || request
            .fixtures
            .iter()
            .map(|fixture| fixture.id)
            .collect::<BTreeSet<_>>()
            .len()
            != request.fixtures.len()
    {
        return Err(WorkerFailure::invalid_request());
    }
    simulate_cam_geometry(backend, graph, request)
        .map(WorkerReply::Cam)
        .map_err(|error| geometry_failure(&error))
}

fn cam_axial_sweep(
    backend: &ExactBackend,
    motion: &crate::CamSimulationWireMotion,
    radius_mm: f64,
    axial_length_mm: f64,
    axial_offset_mm: f64,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let offset = |mut point: [f64; 3]| {
        point[2] += axial_offset_mm;
        point
    };
    backend.sweep_axial_tool(AxialToolSweepSpec {
        motion: if motion.path_kind == 0 {
            AxialToolMotion::Line {
                start_mm: offset(motion.start_mm),
                end_mm: offset(motion.end_mm),
            }
        } else {
            AxialToolMotion::Arc {
                start_mm: offset(motion.start_mm),
                end_mm: offset(motion.end_mm),
                center_mm: offset(motion.center_mm),
                clockwise: motion.clockwise,
            }
        },
        radius_mm,
        axial_length_mm,
    })
}

fn cam_world_sweep(
    backend: &ExactBackend,
    request: &CamSimulationWireRequest,
    motion: &crate::CamSimulationWireMotion,
    radius_mm: f64,
    axial_length_mm: f64,
    axial_offset_mm: f64,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let local = cam_axial_sweep(backend, motion, radius_mm, axial_length_mm, axial_offset_mm)?;
    backend.transform_body(&local.body, &request.setup_to_world)
}

#[allow(clippy::too_many_arguments)]
fn append_cam_collision(
    collisions: &mut Vec<CamSimulationWireCollision>,
    backend: &ExactBackend,
    motion_index: usize,
    motion_kind: u8,
    participant: u8,
    target: u8,
    fixture_id: Option<u64>,
    moving: &ketchup_exact::ExactBody,
    obstacle: &ketchup_exact::ExactBody,
    tolerance_mm: f64,
) -> Result<(), ketchup_exact::GeometryError> {
    let relation = backend.query_body_pair(moving, obstacle, tolerance_mm)?;
    if relation.relation != ExactPairRelation::Separated {
        collisions.push(CamSimulationWireCollision {
            motion_index,
            motion_kind,
            participant,
            target,
            fixture_id,
            common_volume_mm3: relation.common_volume_mm3,
            contact_area_mm2: relation.common_contact_area_mm2,
            distance_mm: relation.distance_mm,
        });
    }
    Ok(())
}

fn simulate_cam_geometry(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    request: &CamSimulationWireRequest,
) -> Result<CamSimulationWireEvidence, ketchup_exact::GeometryError> {
    let target = evaluate_exact_brep_graph(backend, graph, &[])?;
    let tolerance_mm = graph.tolerance.linear_mm();
    let box_from_bounds = |bounds: [[f64; 3]; 2]| BoxSpec {
        origin_mm: Point3 {
            x: bounds[0][0],
            y: bounds[0][1],
            z: bounds[0][2],
        },
        size_mm: Size3 {
            x: bounds[1][0] - bounds[0][0],
            y: bounds[1][1] - bounds[0][1],
            z: bounds[1][2] - bounds[0][2],
        },
    };
    let mut stock = backend.make_box(box_from_bounds(request.stock_bounds_mm))?;
    let stock_before_mm3 = stock.body.topology.volume_mm3;
    let fixture_bodies = request
        .fixtures
        .iter()
        .map(|fixture| {
            backend
                .make_box(box_from_bounds(fixture.bounds_mm))
                .map(|body| (fixture.id, body))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut collisions = Vec::new();
    let mut cutting_motion_count = 0;
    for (motion_index, motion) in request.motions.iter().enumerate() {
        let tool = cam_world_sweep(
            backend,
            request,
            motion,
            request.cutter_radius_mm,
            request.tool_length_mm,
            0.0,
        )?;
        let holder = cam_world_sweep(
            backend,
            request,
            motion,
            request.holder_radius_mm,
            request.holder_length_mm,
            request.holder_offset_mm,
        )?;
        if motion.kind == 0 {
            append_cam_collision(
                &mut collisions,
                backend,
                motion_index,
                motion.kind,
                0,
                0,
                None,
                &tool.body,
                &stock.body,
                tolerance_mm,
            )?;
        }
        append_cam_collision(
            &mut collisions,
            backend,
            motion_index,
            motion.kind,
            1,
            0,
            None,
            &holder.body,
            &stock.body,
            tolerance_mm,
        )?;
        for (fixture_id, fixture) in &fixture_bodies {
            append_cam_collision(
                &mut collisions,
                backend,
                motion_index,
                motion.kind,
                0,
                1,
                Some(*fixture_id),
                &tool.body,
                &fixture.body,
                tolerance_mm,
            )?;
            append_cam_collision(
                &mut collisions,
                backend,
                motion_index,
                motion.kind,
                1,
                1,
                Some(*fixture_id),
                &holder.body,
                &fixture.body,
                tolerance_mm,
            )?;
        }
        if matches!(motion.kind, 1 | 2) {
            cutting_motion_count += 1;
            let cutter = cam_world_sweep(
                backend,
                request,
                motion,
                request.cutter_radius_mm,
                request.cutter_length_mm,
                0.0,
            )?;
            stock = backend.boolean_bodies(
                &stock.body,
                &cutter.body,
                ExactBodyBooleanOperation::Cut,
            )?;
        }
    }
    let stock_after_mm3 = stock.body.topology.volume_mm3;
    let common = backend.query_body_pair(&stock.body, &target.body, tolerance_mm)?;
    let residual_stock_mm3 = (stock_after_mm3 - common.common_volume_mm3).max(0.0);
    let gouge_mm3 = (target.body.topology.volume_mm3 - common.common_volume_mm3).max(0.0);
    let evidence = CamSimulationWireEvidence {
        schema: CAM_SIMULATION_SCHEMA_V1.to_owned(),
        plan_digest: request.plan_digest.clone(),
        toolpath_digest: request.toolpath_digest.clone(),
        target_exact_graph_digest: request.target_exact_graph_digest.clone(),
        motion_count: request.motions.len(),
        cutting_motion_count,
        fixture_count: request.fixtures.len(),
        stock_before_mm3,
        stock_after_mm3,
        removed_stock_mm3: (stock_before_mm3 - stock_after_mm3).max(0.0),
        residual_stock_mm3,
        gouge_mm3,
        collisions,
        backend: ketchup_exact::backend_fingerprint().to_owned(),
        tolerance: ketchup_exact::tolerance_profile().to_owned(),
        result_fingerprint: String::new(),
    };
    Ok(evidence)
}

// Terminal outputs of validated graphs, keyed by the recomputed graph digest
// (which covers imported source hashes).
thread_local! {
    static GRAPH_OUTPUT_CACHE: std::cell::RefCell<output_cache::OutputCache<ExactOpOutput>> =
        std::cell::RefCell::default();
}

fn evaluate_exact_brep_graph_cached(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    imported_sources: &[(String, tempfile::NamedTempFile)],
) -> Result<std::rc::Rc<ExactOpOutput>, ketchup_exact::GeometryError> {
    if let Some(output) = GRAPH_OUTPUT_CACHE.with(|cache| cache.borrow().get(&graph.graph_digest)) {
        return Ok(output);
    }
    let output = std::rc::Rc::new(evaluate_exact_brep_graph(backend, graph, imported_sources)?);
    let bytes = output.retained_bytes();
    GRAPH_OUTPUT_CACHE.with(|cache| {
        cache.borrow_mut().insert(
            graph.graph_digest.clone(),
            std::rc::Rc::clone(&output),
            bytes,
        );
    });
    Ok(output)
}

fn evaluate_exact_brep_graph(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    imported_sources: &[(String, tempfile::NamedTempFile)],
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    graph
        .validate()
        .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
    let needs_names = exact_brep_nodes_needing_names(graph);
    let mut outputs = Vec::<ExactOpOutput>::with_capacity(graph.nodes.len());
    let mut face_names = Vec::<Option<Vec<String>>>::with_capacity(graph.nodes.len());
    let mut extrusion_lineage = Vec::<bool>::with_capacity(graph.nodes.len());
    let mut xde_documents = Vec::<(String, ketchup_exact::StepXdeDocument)>::new();
    for (node_index, node) in graph.nodes.iter().enumerate() {
        let mut produced_face_names = None;
        let output = match &node.operation {
            ExactBRepOperation::Extrude {
                profile, interval, ..
            } => {
                let profile = &graph.profiles[profile.0 as usize];
                if !needs_names[node_index] || !exact_brep_profile_is_nameable(profile, true) {
                    exact_brep_profile_body(backend, profile, *interval)?
                } else {
                    let (output, names) =
                        exact_brep_named_profile_body(backend, profile, *interval)?;
                    produced_face_names = Some(names);
                    output
                }
            }
            ExactBRepOperation::ProfileCut {
                target,
                profile,
                depth_bits,
                interval,
                tool_name,
                ..
            } => {
                let target_output = &outputs[target.0 as usize];
                let profile = &graph.profiles[profile.0 as usize];
                let interval = if depth_bits.is_none() {
                    exact_brep_through_all_interval(
                        graph,
                        &target_output.body,
                        profile.frame().origin.to_array(),
                        *interval,
                    )?
                } else {
                    *interval
                };
                if let (Some(tool_name), Some(target_names)) =
                    (tool_name, face_names[target.0 as usize].as_ref())
                    && needs_names[node_index]
                    && exact_brep_profile_is_nameable(profile, true)
                {
                    let (tool, tool_names) =
                        exact_brep_named_profile_body(backend, profile, interval)?;
                    // Generated cuts (e.g. pin holes) carry display names no
                    // program can reference; a node-derived token keeps the
                    // target's other face names alive instead of failing.
                    let tool_name = if valid_name(tool_name) {
                        tool_name.clone()
                    } else {
                        format!("cut_{node_index}")
                    };
                    let named = backend
                        .named_boolean_output(
                            target_output,
                            target_names,
                            &tool,
                            &tool_names,
                            &tool_name,
                            NamedBoolean::Cut,
                        )
                        .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
                    let (output, names) = named.into_output_and_names();
                    produced_face_names = Some(names);
                    output
                } else {
                    let tool = exact_brep_profile_body(backend, profile, interval)?;
                    backend.boolean_bodies(
                        &target_output.body,
                        &tool.body,
                        ExactBodyBooleanOperation::Cut,
                    )?
                }
            }
            ExactBRepOperation::RigidTransform {
                target,
                matrix_bits,
            }
            | ExactBRepOperation::Scale {
                target,
                matrix_bits,
            } => {
                // A rigid motion or an axis stretch keeps every face and its name.
                let input = &outputs[target.0 as usize];
                let matrix = matrix_bits.map(f64::from_bits);
                match face_names[target.0 as usize].as_ref() {
                    Some(names) if needs_names[node_index] => {
                        produced_face_names = Some(names.clone());
                        transform_named_output(backend, input, &matrix)?
                    }
                    _ => backend.transform_body(&input.body, &matrix)?,
                }
            }
            ExactBRepOperation::Boolean {
                operation,
                target,
                tool,
                tool_name,
            } => {
                let named = match operation {
                    ExactBRepBooleanOperation::Cut => Some(NamedBoolean::Cut),
                    ExactBRepBooleanOperation::Union => Some(NamedBoolean::Union),
                    ExactBRepBooleanOperation::Intersect => Some(NamedBoolean::Intersect),
                    ExactBRepBooleanOperation::Split => None,
                };
                match (named, face_names[target.0 as usize].as_ref()) {
                    (Some(named), Some(target_names)) if needs_names[node_index] => {
                        let tool_output = &outputs[tool.0 as usize];
                        // A tool without program names still gets one name per
                        // face, split into `face#1`, `face#2`, ... by position.
                        let tool_names = face_names[tool.0 as usize].clone().unwrap_or_else(|| {
                            vec!["face".to_owned(); tool_output.body.topology.faces.len()]
                        });
                        let tool_name = tool_name
                            .as_ref()
                            .filter(|name| valid_name(name))
                            .cloned()
                            .unwrap_or_else(|| format!("boolean_{node_index}"));
                        let (output, names) = backend
                            .named_boolean_output(
                                &outputs[target.0 as usize],
                                target_names,
                                tool_output,
                                &tool_names,
                                &tool_name,
                                named,
                            )
                            .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?
                            .into_output_and_names();
                        produced_face_names = Some(names);
                        output
                    }
                    _ => backend.boolean_bodies(
                        &outputs[target.0 as usize].body,
                        &outputs[tool.0 as usize].body,
                        match operation {
                            ExactBRepBooleanOperation::Cut => ExactBodyBooleanOperation::Cut,
                            ExactBRepBooleanOperation::Union => ExactBodyBooleanOperation::Union,
                            ExactBRepBooleanOperation::Intersect => {
                                ExactBodyBooleanOperation::Intersect
                            }
                            ExactBRepBooleanOperation::Split => ExactBodyBooleanOperation::Split,
                        },
                    )?,
                }
            }
            ExactBRepOperation::Revolve {
                profile,
                axis_start_bits,
                axis_end_bits,
                angle_degrees_bits,
            } => {
                let profile = &graph.profiles[profile.0 as usize];
                let axis_start_mm = axis_start_bits.map(f64::from_bits);
                let axis_end_mm = axis_end_bits.map(f64::from_bits);
                let angle_degrees = f64::from_bits(*angle_degrees_bits);
                if !needs_names[node_index] || !exact_brep_profile_is_nameable(profile, false) {
                    exact_brep_revolve(backend, profile, axis_start_mm, axis_end_mm, angle_degrees)?
                } else {
                    let (output, names) = exact_brep_named_revolve(
                        backend,
                        profile,
                        axis_start_mm,
                        axis_end_mm,
                        angle_degrees,
                    )?;
                    produced_face_names = Some(names);
                    output
                }
            }
            ExactBRepOperation::PlanarOffset {
                profile,
                distance_bits,
            } => exact_brep_planar_offset(
                backend,
                &graph.profiles[profile.0 as usize],
                f64::from_bits(*distance_bits),
            )?,
            ExactBRepOperation::PlanarSurface { profile } => {
                exact_brep_planar_surface(backend, &graph.profiles[profile.0 as usize])?
            }
            ExactBRepOperation::Sweep { profile, path } => exact_brep_sweep(
                backend,
                &graph.profiles[profile.0 as usize],
                &graph.profiles[path.0 as usize],
            )?,
            ExactBRepOperation::SpatialSweep {
                profile,
                path,
                up_bits: up,
            } => exact_brep_spatial_sweep(backend, &graph.profiles[profile.0 as usize], path, *up)?,
            ExactBRepOperation::WeldmentJoint {
                first,
                second,
                joint_point_bits,
                first_direction_bits,
                second_direction_bits,
                policy,
                primary,
            } => exact_brep_weldment_joint(
                backend,
                graph,
                &outputs[first.0 as usize],
                &outputs[second.0 as usize],
                joint_point_bits.map(f64::from_bits),
                first_direction_bits.map(f64::from_bits),
                second_direction_bits.map(f64::from_bits),
                *policy,
                *primary,
            )?,
            ExactBRepOperation::Loft {
                sections,
                guide,
                continuity,
            } => exact_brep_loft(backend, graph, sections, guide.as_ref(), *continuity, true)?,
            ExactBRepOperation::LoftSurface {
                sections,
                guide,
                continuity,
            } => exact_brep_loft(backend, graph, sections, guide.as_ref(), *continuity, false)?,
            ExactBRepOperation::SurfaceTrim { target, cutter } => backend.trim_surface(
                &outputs[target.0 as usize].body,
                &outputs[cutter.0 as usize].body,
            )?,
            ExactBRepOperation::SurfaceExtend {
                target,
                distance_bits,
            } => backend.extend_planar_surface(
                &outputs[target.0 as usize].body,
                f64::from_bits(*distance_bits),
            )?,
            ExactBRepOperation::SurfaceKnit {
                surfaces,
                tolerance_bits,
                make_solid,
            } => {
                let surfaces = surfaces
                    .iter()
                    .map(|surface| &outputs[surface.0 as usize].body)
                    .collect::<Vec<_>>();
                backend.knit_surfaces(&surfaces, f64::from_bits(*tolerance_bits), *make_solid)?
            }
            ExactBRepOperation::SurfaceThicken {
                target,
                thickness_bits,
                direction,
            } => backend.thicken_surface(
                &outputs[target.0 as usize].body,
                f64::from_bits(*thickness_bits),
                match direction {
                    ExactBRepShellDirection::Inward => NativeShellDirection::Inward,
                    ExactBRepShellDirection::Outward => NativeShellDirection::Outward,
                    ExactBRepShellDirection::Symmetric => NativeShellDirection::Symmetric,
                },
            )?,
            ExactBRepOperation::Shell {
                target,
                removed_faces,
                thickness_bits,
                direction,
                profile_faces,
                name,
            } => {
                let target_output = &outputs[target.0 as usize];
                if let (false, Some(prefix)) = (profile_faces.is_empty(), name) {
                    let names = face_names[target.0 as usize].as_ref().ok_or_else(|| {
                        exact_brep_graph_error(
                            graph,
                            "named shell target has no program face names",
                        )
                    })?;
                    let open = profile_faces
                        .iter()
                        .map(exact_brep_profile_face_name)
                        .collect::<Vec<_>>();
                    let open = open.iter().map(String::as_str).collect::<Vec<_>>();
                    let named = backend
                        .named_shell_output(
                            target_output,
                            names,
                            &open,
                            f64::from_bits(*thickness_bits),
                            prefix,
                        )
                        .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
                    let (output, names) = named.into_output_and_names();
                    produced_face_names = Some(names);
                    output
                } else {
                    let ordinals = exact_brep_topology_ordinals(
                        graph,
                        *target,
                        target_output,
                        removed_faces,
                        ExactBRepTopologyKind::Face,
                    )?;
                    backend.shell_body_with_direction(
                        &target_output.body,
                        &ordinals,
                        f64::from_bits(*thickness_bits),
                        match direction {
                            ExactBRepShellDirection::Inward => NativeShellDirection::Inward,
                            ExactBRepShellDirection::Outward => NativeShellDirection::Outward,
                            ExactBRepShellDirection::Symmetric => NativeShellDirection::Symmetric,
                        },
                    )?
                }
            }
            ExactBRepOperation::FaceOffset {
                target,
                face,
                profile_face,
                distance_bits,
            } => {
                let target_output = &outputs[target.index()];
                let distance = f64::from_bits(*distance_bits);
                if let Some(profile_face) = profile_face {
                    let names = face_names[target.index()].as_ref().ok_or_else(|| {
                        exact_brep_graph_error(
                            graph,
                            "named face offset target has no program face names",
                        )
                    })?;
                    let face_name = exact_brep_profile_face_name(profile_face);
                    let named = backend
                        .named_offset_face_output(target_output, names, &face_name, distance)
                        .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
                    let (output, names) = named.into_output_and_names();
                    produced_face_names = Some(names);
                    output
                } else {
                    let face = face.as_ref().ok_or_else(|| {
                        exact_brep_graph_error(graph, "face offset selector is missing")
                    })?;
                    let ordinal = exact_brep_face_ordinal(graph, *target, target_output, face)?;
                    backend.offset_body_face(&target_output.body, ordinal, distance)?
                }
            }
            ExactBRepOperation::Draft(draft) => evaluate_draft(backend, graph, &outputs, draft)?,
            ExactBRepOperation::ThreadGroove(thread) => {
                evaluate_thread_groove(backend, &outputs, thread)?
            }
            ExactBRepOperation::EdgeFinish {
                target,
                edges,
                profile_edges,
                kind,
                amount_bits,
                fillet_radius_stations,
                chamfer_mode,
                chamfer_edge_sides,
            } => {
                let target_output = &outputs[target.0 as usize];
                if !profile_edges.is_empty() {
                    let names = face_names[target.0 as usize].as_ref().ok_or_else(|| {
                        exact_brep_graph_error(
                            graph,
                            "named edge finish target has no profile face names",
                        )
                    })?;
                    let edge_names = profile_edges
                        .iter()
                        .map(|edge| {
                            (
                                exact_brep_profile_face_name(&edge.first),
                                exact_brep_profile_face_name(&edge.second),
                            )
                        })
                        .collect::<Vec<_>>();
                    let edge_name_refs = edge_names
                        .iter()
                        .map(|(first, second)| (first.as_str(), second.as_str()))
                        .collect::<Vec<_>>();
                    let named = backend
                        .named_finish_output(
                            target_output,
                            names,
                            &edge_name_refs,
                            match kind {
                                ExactBRepEdgeFinishKind::Fillet => EdgeFinish::Fillet,
                                ExactBRepEdgeFinishKind::Chamfer => EdgeFinish::Chamfer,
                            },
                            f64::from_bits(*amount_bits),
                        )
                        .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
                    let (output, names) = named.into_output_and_names();
                    produced_face_names = Some(names);
                    output
                } else {
                    let ordinals = exact_brep_topology_ordinals(
                        graph,
                        *target,
                        target_output,
                        edges,
                        ExactBRepTopologyKind::Edge,
                    )?;
                    let radius_stations = fillet_radius_stations
                        .iter()
                        .map(|station| {
                            [
                                f64::from_bits(station.position_bits),
                                f64::from_bits(station.radius_bits),
                            ]
                        })
                        .collect::<Vec<_>>();
                    match chamfer_mode {
                        ExactBRepChamferMode::Symmetric => backend
                            .finish_body_with_radius_stations(
                                &target_output.body,
                                &ordinals,
                                match kind {
                                    ExactBRepEdgeFinishKind::Fillet => EdgeFinish::Fillet,
                                    ExactBRepEdgeFinishKind::Chamfer => EdgeFinish::Chamfer,
                                },
                                f64::from_bits(*amount_bits),
                                &radius_stations,
                            )?,
                        ExactBRepChamferMode::TwoDistance {
                            second_distance_bits,
                        } => {
                            let face_selectors = chamfer_edge_sides
                                .iter()
                                .map(|selection| selection.side_face.clone())
                                .collect::<Vec<_>>();
                            let (edge_ordinals, face_ordinals) = exact_brep_chamfer_ordinal_pairs(
                                graph,
                                *target,
                                target_output,
                                edges,
                                &face_selectors,
                            )?;
                            backend.finish_body_advanced_chamfer(
                                &target_output.body,
                                &edge_ordinals,
                                &face_ordinals,
                                f64::from_bits(*amount_bits),
                                AdvancedChamferMode::TwoDistance {
                                    second_distance_mm: f64::from_bits(*second_distance_bits),
                                },
                            )?
                        }
                        ExactBRepChamferMode::DistanceAngle { angle_degrees_bits } => {
                            let face_selectors = chamfer_edge_sides
                                .iter()
                                .map(|selection| selection.side_face.clone())
                                .collect::<Vec<_>>();
                            let (edge_ordinals, face_ordinals) = exact_brep_chamfer_ordinal_pairs(
                                graph,
                                *target,
                                target_output,
                                edges,
                                &face_selectors,
                            )?;
                            backend.finish_body_advanced_chamfer(
                                &target_output.body,
                                &edge_ordinals,
                                &face_ordinals,
                                f64::from_bits(*amount_bits),
                                AdvancedChamferMode::DistanceAngle {
                                    angle_degrees: f64::from_bits(*angle_degrees_bits),
                                },
                            )?
                        }
                    }
                }
            }
            ExactBRepOperation::SheetMetal {
                base_mm_bits,
                thickness_bits,
                k_factor_bits,
                bends,
            } => exact_brep_sheet_metal(
                backend,
                graph,
                &exact_sheet_metal_shape(base_mm_bits, *thickness_bits, *k_factor_bits, bends),
            )?,
            ExactBRepOperation::ImportedExact { .. } => exact_brep_imported_body(
                backend,
                graph,
                &node.operation,
                imported_sources,
                &mut xde_documents,
            )?,
        };
        let extrusion = extrusion_lineage_of(&node.operation, &extrusion_lineage);
        let mut output = output;
        if let Some(names) = &produced_face_names {
            exact_brep_expose_face_names(&mut output, names, extrusion);
        }
        outputs.push(output);
        face_names.push(produced_face_names);
        extrusion_lineage.push(extrusion);
    }
    outputs
        .pop()
        .ok_or_else(|| exact_brep_graph_error(graph, "graph produced no exact body"))
}

/// The body an `ImportedExact` node names: its whole source, or one XDE part
/// of it read as STEP or IGES. Parts of one source read it once per graph,
/// into `xde_documents`.
fn exact_brep_imported_body(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    operation: &ExactBRepOperation,
    imported_sources: &[(String, tempfile::NamedTempFile)],
    xde_documents: &mut Vec<(String, ketchup_exact::StepXdeDocument)>,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let ExactBRepOperation::ImportedExact {
        source_sha256,
        result_fingerprint,
        source_format,
        source_part_index,
        ..
    } = operation
    else {
        return Err(exact_brep_graph_error(
            graph,
            "node is not an imported body",
        ));
    };
    let source_sha256 = source_sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let source = imported_sources
        .iter()
        .find_map(|(digest, source)| (digest == &source_sha256).then_some(source.path()))
        .ok_or_else(|| exact_brep_graph_error(graph, "imported exact graph source is unavailable"))?
        .to_string_lossy();
    let mut output = match (source_format, source_part_index) {
        (ExactBRepImportedFormat::Step, None) => backend.import_step(&source)?,
        (ExactBRepImportedFormat::Iges, None) => backend.import_iges(&source)?,
        (format, Some(part_index)) => {
            let read = match xde_documents
                .iter()
                .position(|(digest, _)| *digest == source_sha256)
            {
                Some(position) => position,
                None => {
                    let document = match format {
                        ExactBRepImportedFormat::Step => backend.open_step_xde(&source),
                        ExactBRepImportedFormat::Iges => backend.open_iges_xde(&source),
                    }
                    .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
                    xde_documents.push((source_sha256.clone(), document));
                    xde_documents.len() - 1
                }
            };
            xde_documents[read].1.part(*part_index)?
        }
    };
    if !imported_result_matches(&source_sha256, &output, result_fingerprint) {
        return Err(exact_brep_graph_error(
            graph,
            "imported exact graph result fingerprint does not match its source",
        ));
    }
    output.body.result_fingerprint = result_fingerprint.clone();
    output.input_digest = source_sha256;
    Ok(output)
}

/// Whether a node's body still descends from an extrusion: an extrusion
/// itself, or an operation that reshapes the body of one.
fn extrusion_lineage_of(operation: &ExactBRepOperation, lineage: &[bool]) -> bool {
    match operation {
        ExactBRepOperation::Extrude { .. } => true,
        ExactBRepOperation::ProfileCut { target, .. }
        | ExactBRepOperation::FaceOffset { target, .. }
        | ExactBRepOperation::Draft(ExactBRepDraft { target, .. })
        | ExactBRepOperation::ThreadGroove(ExactBRepThreadGroove { target, .. })
        | ExactBRepOperation::EdgeFinish { target, .. }
        | ExactBRepOperation::Boolean { target, .. }
        | ExactBRepOperation::RigidTransform { target, .. }
        | ExactBRepOperation::Scale { target, .. } => lineage[target.index()],
        _ => false,
    }
}

/// A modelled thread cut into its target's body.
fn evaluate_thread_groove(
    backend: &ExactBackend,
    outputs: &[ExactOpOutput],
    thread: &ExactBRepThreadGroove,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    backend.cut_thread(
        &outputs[thread.target.index()].body,
        &ThreadGrooveSpec {
            origin_mm: thread.origin_bits.map(f64::from_bits),
            axis: thread.axis_bits.map(f64::from_bits),
            reference: thread.reference_bits.map(f64::from_bits),
            pitch_mm: f64::from_bits(thread.pitch_bits),
            start_mm: f64::from_bits(thread.start_bits),
            end_mm: f64::from_bits(thread.end_bits),
            left_handed: thread.left_handed,
            profile_mm: thread
                .profile_bits
                .iter()
                .map(|point| point.map(f64::from_bits))
                .collect(),
            trim_mm: thread.trim_bits.map(|trim| trim.map(f64::from_bits)),
            crest_mm: thread.crest_bits.map(|crest| crest.map(f64::from_bits)),
        },
    )
}

fn exact_brep_through_all_interval(
    graph: &ExactBRepGraph,
    target: &ketchup_exact::ExactBody,
    origin_mm: [f64; 3],
    template: ExactBRepLinearInterval,
) -> Result<ExactBRepLinearInterval, ketchup_exact::GeometryError> {
    let direction = template.direction();
    let bounds = target.topology.bounds_mm;
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for x in [bounds.min.x, bounds.max.x] {
        for y in [bounds.min.y, bounds.max.y] {
            for z in [bounds.min.z, bounds.max.z] {
                let projection = (x - origin_mm[0]) * direction[0]
                    + (y - origin_mm[1]) * direction[1]
                    + (z - origin_mm[2]) * direction[2];
                minimum = minimum.min(projection);
                maximum = maximum.max(projection);
            }
        }
    }
    let start = minimum - 1.0;
    let end = maximum + 1.0;
    if !start.is_finite()
        || !end.is_finite()
        || start.abs() > MAX_COORDINATE_MM
        || end.abs() > MAX_COORDINATE_MM
        || end <= start
    {
        return Err(exact_brep_graph_error(
            graph,
            "through-all target bounds are invalid or outside the bounded envelope",
        ));
    }
    Ok(ExactBRepLinearInterval {
        direction_bits: template.direction_bits,
        start_bits: start.to_bits(),
        end_bits: end.to_bits(),
    })
}

fn durable_face_ordinal(output: &ExactOpOutput, reference: &TopologicalElementRef) -> Option<u32> {
    let matches = exact_brep_graph_face_evidence(output)
        .into_iter()
        .filter(|face| {
            face.semantic_role == reference.producer_element_id
                && face.source_element_id == reference.source_element_id
        })
        .map(|face| face.face_ordinal)
        .collect::<BTreeSet<_>>();
    if matches.len() != 1 {
        return None;
    }
    matches.into_iter().next()
}

fn durable_edge_ordinal(output: &ExactOpOutput, reference: &TopologicalElementRef) -> Option<u32> {
    let face_evidence = exact_brep_graph_face_evidence(output);
    let mut face_provenance = BTreeMap::<u32, Vec<(&str, &str)>>::new();
    for face in &face_evidence {
        face_provenance
            .entry(face.face_ordinal)
            .or_default()
            .push((&face.semantic_role, &face.source_element_id));
    }
    let matches = output
        .body
        .topology
        .edges
        .iter()
        .filter_map(|edge| {
            let adjacent_faces = edge
                .adjacent_face_ordinals
                .iter()
                .map(|face_ordinal| {
                    let [face] = face_provenance.get(face_ordinal)?.as_slice() else {
                        return None;
                    };
                    Some(*face)
                })
                .collect::<Option<Vec<_>>>()?;
            let (source_element_id, producer_element_id) =
                topological_edge_provenance_tokens(&adjacent_faces)?;
            (source_element_id == reference.source_element_id
                && producer_element_id == reference.producer_element_id)
                .then_some(edge.ordinal)
        })
        .collect::<BTreeSet<_>>();
    if matches.len() != 1 {
        return None;
    }
    matches.into_iter().next()
}

fn exact_brep_topology_ordinals_preserving_order(
    graph: &ExactBRepGraph,
    target: ketchup_model::exact_brep_graph::ExactBRepNodeId,
    target_output: &ExactOpOutput,
    selectors: &[ExactBRepTopologySelector],
    expected_kind: ExactBRepTopologyKind,
) -> Result<Vec<u32>, ketchup_exact::GeometryError> {
    let target_node = &graph.nodes[target.0 as usize];
    let kind_token = match expected_kind {
        ExactBRepTopologyKind::Face => "face",
        ExactBRepTopologyKind::Edge => "edge",
    };
    let (
        producer_prefix,
        source_prefix,
        expected_evaluator,
        expected_result_fingerprint,
        imported_target,
    ) = match &target_node.operation {
        ExactBRepOperation::ImportedExact {
            source_sha256,
            result_fingerprint,
            ..
        } => {
            let source_sha256 = source_sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            (
                format!("imported-result/{kind_token}/"),
                format!("imported-source/{source_sha256}/{kind_token}/"),
                "ketchup.imported-step-evaluator.v1",
                result_fingerprint.as_str(),
                true,
            )
        }
        _ => (
            format!("generated-result/{kind_token}/"),
            format!("generated-source/{kind_token}/"),
            EXACT_BREP_GRAPH_EVALUATOR_V1,
            target_output.body.result_fingerprint.as_str(),
            false,
        ),
    };
    let ordinals = selectors
        .iter()
        .map(|selector| {
            if selector.kind != expected_kind {
                return Err(exact_brep_graph_error(
                    graph,
                    "topology selector has the wrong element kind",
                ));
            }
            let reference = selector
                .reference()
                .map_err(|error| exact_brep_graph_error(graph, &error.to_string()))?;
            if reference.producer_feature_id.0 != target_node.source_feature_id
                || reference.source_feature_id.0 != target_node.source_feature_id
                || reference.evaluator != expected_evaluator
                || reference.backend != target_output.backend_fingerprint
                || reference.tolerance != target_output.tolerance_report.profile
            {
                return Err(exact_brep_graph_error(
                    graph,
                    "topology selector is stale or belongs to a different exact target",
                ));
            }
            if reference.result_fingerprint == expected_result_fingerprint
                && let Some(ordinal_token) =
                    reference.producer_element_id.strip_prefix(&producer_prefix)
                && let Ok(ordinal) = ordinal_token.parse::<u32>()
                && ordinal_token == ordinal.to_string()
                && reference.source_element_id == format!("{source_prefix}{ordinal}")
            {
                return Ok(ordinal);
            }
            if imported_target || reference.stability != TopologicalReferenceStability::Guaranteed {
                return Err(exact_brep_graph_error(
                    graph,
                    "topology selector has no durable unambiguous provenance",
                ));
            }
            let ordinal = match expected_kind {
                ExactBRepTopologyKind::Face => durable_face_ordinal(target_output, &reference),
                ExactBRepTopologyKind::Edge => durable_edge_ordinal(target_output, &reference),
            }
            .ok_or_else(|| {
                exact_brep_graph_error(
                    graph,
                    "topology selector provenance is lost or ambiguous after recompute",
                )
            })?;
            Ok(ordinal)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ordinals)
}

fn exact_brep_topology_ordinals(
    graph: &ExactBRepGraph,
    target: ketchup_model::exact_brep_graph::ExactBRepNodeId,
    target_output: &ExactOpOutput,
    selectors: &[ExactBRepTopologySelector],
    expected_kind: ExactBRepTopologyKind,
) -> Result<Vec<u32>, ketchup_exact::GeometryError> {
    let mut ordinals = exact_brep_topology_ordinals_preserving_order(
        graph,
        target,
        target_output,
        selectors,
        expected_kind,
    )?;
    ordinals.sort_unstable();
    if ordinals.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(exact_brep_graph_error(
            graph,
            "topology selector ordinals are duplicated",
        ));
    }
    Ok(ordinals)
}

/// The ordinal of the one face `selector` names on `target_output`.
fn exact_brep_face_ordinal(
    graph: &ExactBRepGraph,
    target: ketchup_model::exact_brep_graph::ExactBRepNodeId,
    target_output: &ExactOpOutput,
    selector: &ExactBRepTopologySelector,
) -> Result<u32, ketchup_exact::GeometryError> {
    let ordinals = exact_brep_topology_ordinals(
        graph,
        target,
        target_output,
        std::slice::from_ref(selector),
        ExactBRepTopologyKind::Face,
    )?;
    let [ordinal] = ordinals[..] else {
        return Err(exact_brep_graph_error(
            graph,
            &format!(
                "face selector resolved to {} faces instead of one",
                ordinals.len()
            ),
        ));
    };
    Ok(ordinal)
}

fn evaluate_draft(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    outputs: &[ExactOpOutput],
    draft: &ExactBRepDraft,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let target_output = &outputs[draft.target.index()];
    let faces = exact_brep_topology_ordinals(
        graph,
        draft.target,
        target_output,
        &draft.faces,
        ExactBRepTopologyKind::Face,
    )?;
    let neutral = exact_brep_face_ordinal(graph, draft.target, target_output, &draft.neutral)?;
    backend.draft_body(
        &target_output.body,
        &faces,
        neutral,
        f64::from_bits(draft.angle_degrees_bits),
    )
}

fn exact_brep_chamfer_ordinal_pairs(
    graph: &ExactBRepGraph,
    target: ketchup_model::exact_brep_graph::ExactBRepNodeId,
    target_output: &ExactOpOutput,
    edge_selectors: &[ExactBRepTopologySelector],
    face_selectors: &[ExactBRepTopologySelector],
) -> Result<(Vec<u32>, Vec<u32>), ketchup_exact::GeometryError> {
    let edge_ordinals = exact_brep_topology_ordinals_preserving_order(
        graph,
        target,
        target_output,
        edge_selectors,
        ExactBRepTopologyKind::Edge,
    )?;
    let face_ordinals = exact_brep_topology_ordinals_preserving_order(
        graph,
        target,
        target_output,
        face_selectors,
        ExactBRepTopologyKind::Face,
    )?;
    let mut pairs = edge_ordinals
        .into_iter()
        .zip(face_ordinals)
        .collect::<Vec<_>>();
    pairs.sort_unstable_by_key(|pair| pair.0);
    if pairs.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(exact_brep_graph_error(
            graph,
            "advanced chamfer edge ordinals are duplicated",
        ));
    }
    Ok(pairs.into_iter().unzip())
}

fn exact_brep_loft(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    sections: &[ExactBRepLoftSection],
    guide: Option<&ExactBRepSpatialPath>,
    continuity: ExactBRepLoftContinuity,
    make_solid: bool,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let guide_segments = guide.map(|guide| {
        guide
            .segments
            .iter()
            .map(exact_brep_spatial_path_segment)
            .collect::<Vec<_>>()
    });
    let continuity = match continuity {
        ExactBRepLoftContinuity::Position => LoftSurfaceContinuity::Position,
        ExactBRepLoftContinuity::Tangent => LoftSurfaceContinuity::Tangent,
        ExactBRepLoftContinuity::Curvature => LoftSurfaceContinuity::Curvature,
    };
    let mut outer_sections = Vec::with_capacity(sections.len());
    let mut hole_sections: Option<Vec<Vec<FramedLoftSection>>> = None;
    for section in sections {
        let profile = &graph.profiles[section.profile.0 as usize];
        let (outer, holes) = match &profile.geometry {
            ExactBRepPlanarGeometry::Spline { control_point_bits } => (
                FramedLoftProfile::Spline {
                    control_points_mm: control_point_bits
                        .iter()
                        .map(|point| point.map(f64::from_bits))
                        .collect(),
                },
                Vec::new(),
            ),
            ExactBRepPlanarGeometry::Circle { .. }
            | ExactBRepPlanarGeometry::Boundary { closed: true, .. } => (
                FramedLoftProfile::Planar(exact_brep_planar_offset_loop(profile)?),
                Vec::new(),
            ),
            ExactBRepPlanarGeometry::Region { outer, holes } => (
                FramedLoftProfile::Planar(exact_brep_planar_loop(outer)),
                holes
                    .iter()
                    .map(|hole| FramedLoftProfile::Planar(exact_brep_planar_loop(hole)))
                    .collect(),
            ),
            _ => {
                return Err(exact_brep_profile_error(
                    profile,
                    "exact Loft requires closed line/arc/circle/cubic or spline sections",
                ));
            }
        };
        let elevation_mm = f64::from_bits(section.elevation_bits);
        let frame = profile.frame();
        let hole_groups = hole_sections.get_or_insert_with(|| {
            (0..holes.len())
                .map(|_| Vec::with_capacity(sections.len()))
                .collect()
        });
        if hole_groups.len() != holes.len() {
            return Err(exact_brep_profile_error(
                profile,
                "exact Loft requires the same hole count in every section",
            ));
        }
        outer_sections.push(FramedLoftSection {
            elevation_mm,
            frame,
            profile: outer,
        });
        for (group, hole) in hole_groups.iter_mut().zip(holes) {
            group.push(FramedLoftSection {
                elevation_mm,
                frame,
                profile: hole,
            });
        }
    }
    let mut output = if make_solid {
        backend.loft_framed_profiles_with_controls(
            &FramedLoftSpec {
                sections: outer_sections,
            },
            guide_segments.as_deref(),
            continuity,
        )?
    } else {
        backend.loft_framed_surface(
            &FramedLoftSpec {
                sections: outer_sections,
            },
            guide_segments.as_deref(),
            continuity,
        )?
    };
    let hole_sections = hole_sections.unwrap_or_default();
    if !make_solid && !hole_sections.is_empty() {
        return Err(ketchup_exact::GeometryError {
            code: ketchup_exact::GeometryErrorCode::InvalidProfile,
            diagnostic: "Loft surface holes are not supported".to_owned(),
            operation: "loft_framed_surface",
            input_digest: graph.canonical_input_digest.clone(),
            backend_fingerprint: ketchup_exact::backend_fingerprint(),
        });
    }
    for sections in hole_sections {
        let tool = backend.loft_framed_profiles_with_controls(
            &FramedLoftSpec { sections },
            guide_segments.as_deref(),
            continuity,
        )?;
        output =
            backend.boolean_bodies(&output.body, &tool.body, ExactBodyBooleanOperation::Cut)?;
    }
    Ok(output)
}

fn exact_brep_revolve(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let segments = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary { .. } => exact_brep_boundary_segments(profile, true)?,
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let center = center_bits.map(f64::from_bits);
            let radius = f64::from_bits(*radius_bits);
            let positive = [center[0] + radius, center[1]];
            let negative = [center[0] - radius, center[1]];
            vec![
                PlanarProfileSegment::CircularArc {
                    start_mm: positive,
                    end_mm: negative,
                    center_mm: center,
                    clockwise: false,
                },
                PlanarProfileSegment::CircularArc {
                    start_mm: negative,
                    end_mm: positive,
                    center_mm: center,
                    clockwise: false,
                },
            ]
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            let outer = exact_brep_planar_loop(outer);
            let holes = holes.iter().map(exact_brep_planar_loop).collect::<Vec<_>>();
            let local = backend.revolve_planar_region(
                &outer,
                &holes,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            )?;
            return transform_local_to_profile_frame(backend, profile, &local);
        }
        ExactBRepPlanarGeometry::Spline { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact revolve requires a closed line/arc/circle/cubic profile",
            ));
        }
    };
    let local =
        backend.revolve_general_profile(&segments, axis_start_mm, axis_end_mm, angle_degrees)?;
    transform_local_to_profile_frame(backend, profile, &local)
}

fn transform_local_to_profile_frame(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    local: &ExactOpOutput,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    backend.transform_body(&local.body, &profile.frame().to_affine().to_row_major())
}

/// The local-to-world matrix of `profile`'s plane with z along the
/// interval direction, starting where the interval starts.
fn extrusion_matrix(profile: &ExactBRepProfile, interval: ExactBRepLinearInterval) -> [f64; 16] {
    let frame = profile.frame();
    let direction = Vec3::from(interval.direction());
    linalg::Frame {
        origin: frame.origin + direction * interval.start_mm(),
        z: direction,
        ..frame
    }
    .to_affine()
    .to_row_major()
}

fn transform_named_output(
    backend: &ExactBackend,
    local: &ExactOpOutput,
    matrix: &[f64; 16],
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let mut output = backend.transform_body(&local.body, matrix)?;
    let affine = Affine3::from_row_major(*matrix);
    let transform_point = |point: Point3| {
        let point = affine.transform_point(Vec3::new(point.x, point.y, point.z));
        Point3 {
            x: point.x,
            y: point.y,
            z: point.z,
        }
    };
    let close = |left: f64, right: f64| {
        (left - right).abs() <= DEFAULT_LINEAR_TOLERANCE_MM * left.abs().max(right.abs()).max(1.0)
    };
    let same_point = |left: Point3, right: Point3| {
        close(left.x, right.x) && close(left.y, right.y) && close(left.z, right.z)
    };
    // A motion or an even scale by `s` keeps every surface kind and scales
    // lengths by `s`; an uneven stretch keeps only the topology.
    let column = |index: usize| Vec3::new(matrix[index], matrix[4 + index], matrix[8 + index]);
    let s = column(0).length();
    let similarity = (close(column(1).length(), s)
        && close(column(2).length(), s)
        && [(0, 1), (0, 2), (1, 2)]
            .into_iter()
            .all(|(a, b)| close(column(a).dot(column(b)) / (s * s), 0.0)))
    .then_some(s);
    let faces_match = local.body.topology.faces.len() == output.body.topology.faces.len()
        && local
            .body
            .topology
            .faces
            .iter()
            .zip(&output.body.topology.faces)
            .all(|(before, after)| {
                before.ordinal == after.ordinal
                    && before.edge_count == after.edge_count
                    && similarity.is_none_or(|s| {
                        before.surface_kind == after.surface_kind
                            && close(before.area_mm2 * s * s, after.area_mm2)
                            && same_point(transform_point(before.centroid_mm), after.centroid_mm)
                    })
            });
    let edges_match = local.body.topology.edges.len() == output.body.topology.edges.len()
        && local
            .body
            .topology
            .edges
            .iter()
            .zip(&output.body.topology.edges)
            .all(|(before, after)| {
                before.ordinal == after.ordinal
                    && similarity.is_none_or(|s| {
                        before.curve_kind == after.curve_kind
                            && close(before.length_mm * s, after.length_mm)
                            && same_point(transform_point(before.centroid_mm), after.centroid_mm)
                    })
            });
    if faces_match && edges_match {
        output.topology_history.clone_from(&local.topology_history);
    }
    Ok(output)
}

fn exact_brep_sweep(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    path: &ExactBRepProfile,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let ExactBRepPlanarGeometry::Boundary {
        closed: false,
        segments,
    } = &path.geometry
    else {
        return Err(exact_brep_profile_error(
            path,
            "exact sweep requires an open planar path",
        ));
    };
    if segments.len() > 1 {
        let profile_segments = exact_brep_boundary_segments(profile, true)?;
        let path_segments = exact_brep_boundary_segments(path, false)?;
        return backend.sweep_planar_profile(&profile_segments, &path_segments);
    }
    let [
        ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        },
    ] = segments.as_slice()
    else {
        return Err(exact_brep_profile_error(
            path,
            "exact sweep currently requires one straight path segment",
        ));
    };
    let start = start_bits.map(f64::from_bits);
    let end = end_bits.map(f64::from_bits);
    let direction = [end[0] - start[0], end[1] - start[1]];
    let length = direction[0].hypot(direction[1]);
    let tangent = [direction[0] / length, direction[1] / length];
    let section = [tangent[1], -tangent[0]];
    let local = exact_brep_profile_body(
        backend,
        profile,
        ExactBRepLinearInterval {
            direction_bits: profile.frame().z.to_array().map(f64::to_bits),
            start_bits: 0.0_f64.to_bits(),
            end_bits: length.to_bits(),
        },
    )?;
    backend.transform_body(
        &local.body,
        &[
            section[0], 0.0, tangent[0], start[0], section[1], 0.0, tangent[1], start[1], 0.0, 1.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    )
}

#[allow(clippy::too_many_arguments)]
fn exact_brep_weldment_joint(
    backend: &ExactKernel,
    graph: &ExactBRepGraph,
    first: &ExactOpOutput,
    second: &ExactOpOutput,
    joint: [f64; 3],
    first_direction: [f64; 3],
    second_direction: [f64; 3],
    policy: ExactBRepWeldmentJointPolicy,
    primary: ExactBRepWeldmentJointPrimary,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let input_relation =
        backend.query_body_pair(&first.body, &second.body, graph.tolerance.linear_mm())?;
    if input_relation.relation != ExactPairRelation::Penetrating
        || input_relation.common_volume_mm3 <= APPROXIMATION
    {
        return Err(exact_brep_graph_error(
            graph,
            "weldment joint members must have a bounded positive intersection",
        ));
    }
    match policy {
        ExactBRepWeldmentJointPolicy::Butt => match primary {
            ExactBRepWeldmentJointPrimary::First => {
                let second_trimmed = backend.boolean_bodies(
                    &second.body,
                    &first.body,
                    ExactBodyBooleanOperation::Cut,
                )?;
                finish_exact_brep_weldment_joint(graph, backend, &first.body, &second_trimmed.body)
            }
            ExactBRepWeldmentJointPrimary::Second => {
                let first_trimmed = backend.boolean_bodies(
                    &first.body,
                    &second.body,
                    ExactBodyBooleanOperation::Cut,
                )?;
                finish_exact_brep_weldment_joint(graph, backend, &first_trimmed.body, &second.body)
            }
        },
        ExactBRepWeldmentJointPolicy::Miter => {
            let normal = [
                first_direction[0] - second_direction[0],
                first_direction[1] - second_direction[1],
                first_direction[2] - second_direction[2],
            ];
            let first_keep = [
                joint[0] + first_direction[0],
                joint[1] + first_direction[1],
                joint[2] + first_direction[2],
            ];
            let second_keep = [
                joint[0] + second_direction[0],
                joint[1] + second_direction[1],
                joint[2] + second_direction[2],
            ];
            let first_trimmed =
                backend.trim_body_by_plane(&first.body, joint, normal, first_keep)?;
            let second_trimmed =
                backend.trim_body_by_plane(&second.body, joint, normal, second_keep)?;
            finish_exact_brep_weldment_joint(
                graph,
                backend,
                &first_trimmed.body,
                &second_trimmed.body,
            )
        }
    }
}

fn finish_exact_brep_weldment_joint(
    graph: &ExactBRepGraph,
    backend: &ExactKernel,
    first: &ketchup_exact::ExactBody,
    second: &ketchup_exact::ExactBody,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let relation = backend.query_body_pair(first, second, APPROXIMATION)?;
    if relation.relation == ExactPairRelation::Penetrating {
        return Err(exact_brep_graph_error(
            graph,
            "weldment joint output members still penetrate",
        ));
    }
    let combined = backend.combine_bodies(first, second)?;
    if combined.body.topology.solid_count != 2 {
        return Err(exact_brep_graph_error(
            graph,
            "weldment joint must preserve exactly two participant solids",
        ));
    }
    Ok(combined)
}

fn exact_brep_spatial_sweep(
    backend: &ExactKernel,
    profile: &ExactBRepProfile,
    path: &ExactBRepSpatialPath,
    up_bits: Option<[u64; 3]>,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let up = up_bits.map(|up| up.map(f64::from_bits));
    let (outer, holes) = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary { closed: true, .. }
        | ExactBRepPlanarGeometry::Circle { .. } => {
            (exact_brep_planar_offset_loop(profile)?, Vec::new())
        }
        ExactBRepPlanarGeometry::Region { outer, holes } => (
            exact_brep_planar_loop(outer),
            holes.iter().map(exact_brep_planar_loop).collect(),
        ),
        ExactBRepPlanarGeometry::Boundary { closed: false, .. }
        | ExactBRepPlanarGeometry::Spline { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact spatial sweep requires a closed line/arc/circle/cubic profile or region",
            ));
        }
    };
    let path = path
        .segments
        .iter()
        .map(exact_brep_spatial_path_segment)
        .collect::<Vec<_>>();
    let mut output = exact_brep_spatial_sweep_loop(backend, outer, &path, up)?;
    for hole in holes {
        let tool = exact_brep_spatial_sweep_loop(backend, hole, &path, up)?;
        output =
            backend.boolean_bodies(&output.body, &tool.body, ExactBodyBooleanOperation::Cut)?;
    }
    Ok(output)
}

fn exact_brep_spatial_sweep_loop(
    backend: &ExactKernel,
    profile: PlanarProfileLoop,
    path: &[SpatialProfileSegment],
    up: Option<[f64; 3]>,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let segments = match profile {
        PlanarProfileLoop::Segments(segments) => segments,
        PlanarProfileLoop::Circle {
            center_mm,
            radius_mm,
        } => {
            let positive = [center_mm[0] + radius_mm, center_mm[1]];
            let negative = [center_mm[0] - radius_mm, center_mm[1]];
            vec![
                PlanarProfileSegment::CircularArc {
                    start_mm: positive,
                    end_mm: negative,
                    center_mm,
                    clockwise: false,
                },
                PlanarProfileSegment::CircularArc {
                    start_mm: negative,
                    end_mm: positive,
                    center_mm,
                    clockwise: false,
                },
            ]
        }
    };
    backend.sweep_spatial_profile(&segments, path, up)
}

fn exact_brep_spatial_path_segment(segment: &ExactBRepSpatialPathSegment) -> SpatialProfileSegment {
    match segment {
        ExactBRepSpatialPathSegment::Line {
            start_bits,
            end_bits,
        } => SpatialProfileSegment::Line {
            start_mm: start_bits.map(f64::from_bits),
            end_mm: end_bits.map(f64::from_bits),
        },
        ExactBRepSpatialPathSegment::CircularArc {
            start_bits,
            end_bits,
            center_bits,
            normal_bits,
            clockwise,
        } => SpatialProfileSegment::CircularArc {
            start_mm: start_bits.map(f64::from_bits),
            end_mm: end_bits.map(f64::from_bits),
            center_mm: center_bits.map(f64::from_bits),
            normal: normal_bits.map(f64::from_bits),
            clockwise: *clockwise,
        },
        ExactBRepSpatialPathSegment::CubicBezier {
            start_bits,
            control_1_bits,
            control_2_bits,
            end_bits,
        } => SpatialProfileSegment::CubicBezier {
            start_mm: start_bits.map(f64::from_bits),
            control_1_mm: control_1_bits.map(f64::from_bits),
            control_2_mm: control_2_bits.map(f64::from_bits),
            end_mm: end_bits.map(f64::from_bits),
        },
    }
}

fn exact_brep_planar_surface(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let planar_loop = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary { .. } => {
            PlanarProfileLoop::Segments(exact_brep_boundary_segments(profile, true)?)
        }
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let center_mm = center_bits.map(f64::from_bits);
            let radius_mm = f64::from_bits(*radius_bits);
            let positive = [center_mm[0] + radius_mm, center_mm[1]];
            let negative = [center_mm[0] - radius_mm, center_mm[1]];
            PlanarProfileLoop::Segments(vec![
                PlanarProfileSegment::CircularArc {
                    start_mm: positive,
                    end_mm: negative,
                    center_mm,
                    clockwise: false,
                },
                PlanarProfileSegment::CircularArc {
                    start_mm: negative,
                    end_mm: positive,
                    center_mm,
                    clockwise: false,
                },
            ])
        }
        ExactBRepPlanarGeometry::Region { outer, holes } if holes.is_empty() => {
            exact_brep_planar_loop(outer)
        }
        ExactBRepPlanarGeometry::Region { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact planar surface does not yet support profile holes",
            ));
        }
        ExactBRepPlanarGeometry::Spline { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact planar surface requires a closed line/arc/circle/cubic profile",
            ));
        }
    };
    let local = backend.planar_surface_profile(&planar_loop)?;
    transform_local_to_profile_frame(backend, profile, &local)
}

fn exact_brep_planar_offset(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    distance_mm: f64,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let local = if let ExactBRepPlanarGeometry::Region { outer, holes } = &profile.geometry {
        backend.offset_planar_region(
            &exact_brep_planar_loop(outer),
            &holes.iter().map(exact_brep_planar_loop).collect::<Vec<_>>(),
            distance_mm,
        )?
    } else if let Some([min_x, min_y, max_x, max_y]) = exact_brep_planar_rectangle_bounds(profile) {
        backend.offset_rectangle(RectangleOffsetSpec {
            min_mm: [min_x, min_y],
            max_mm: [max_x, max_y],
            distance_mm,
        })?
    } else {
        backend.offset_planar_profile(&exact_brep_planar_offset_loop(profile)?, distance_mm)?
    };
    if profile.frame() == linalg::Frame::WORLD {
        Ok(local)
    } else {
        transform_local_to_profile_frame(backend, profile, &local)
    }
}

fn exact_brep_planar_offset_loop(
    profile: &ExactBRepProfile,
) -> Result<PlanarProfileLoop, ketchup_exact::GeometryError> {
    match &profile.geometry {
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => Ok(PlanarProfileLoop::Circle {
            center_mm: center_bits.map(f64::from_bits),
            radius_mm: f64::from_bits(*radius_bits),
        }),
        ExactBRepPlanarGeometry::Boundary { .. } => Ok(PlanarProfileLoop::Segments(
            exact_brep_boundary_segments(profile, true)?,
        )),
        _ => Err(exact_brep_profile_error(
            profile,
            "exact planar offset requires a line/arc boundary or circle",
        )),
    }
}

fn exact_brep_boundary_segments(
    profile: &ExactBRepProfile,
    require_closed: bool,
) -> Result<Vec<PlanarProfileSegment>, ketchup_exact::GeometryError> {
    let ExactBRepPlanarGeometry::Boundary { closed, segments } = &profile.geometry else {
        return Err(exact_brep_profile_error(
            profile,
            "exact operation requires a line/arc boundary",
        ));
    };
    if *closed != require_closed {
        return Err(exact_brep_profile_error(
            profile,
            "exact boundary closure does not match the operation",
        ));
    }
    Ok(segments
        .iter()
        .map(|segment| match segment {
            ExactBRepPlanarSegment::Line {
                start_bits,
                end_bits,
            } => PlanarProfileSegment::Line {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            },
            ExactBRepPlanarSegment::CircularArc {
                start_bits,
                end_bits,
                center_bits,
                clockwise,
            } => PlanarProfileSegment::CircularArc {
                start_mm: start_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
                center_mm: center_bits.map(f64::from_bits),
                clockwise: *clockwise,
            },
            ExactBRepPlanarSegment::CubicBezier {
                start_bits,
                control_1_bits,
                control_2_bits,
                end_bits,
            } => PlanarProfileSegment::CubicBezier {
                start_mm: start_bits.map(f64::from_bits),
                control_1_mm: control_1_bits.map(f64::from_bits),
                control_2_mm: control_2_bits.map(f64::from_bits),
                end_mm: end_bits.map(f64::from_bits),
            },
        })
        .collect())
}

fn exact_brep_named_segments(
    profile: &ExactBRepProfile,
) -> Result<Vec<NamedSegment>, ketchup_exact::GeometryError> {
    let segments = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary { .. } => exact_brep_boundary_segments(profile, true)?,
        ExactBRepPlanarGeometry::Region { outer, holes } if holes.is_empty() => {
            match exact_brep_planar_loop(outer) {
                PlanarProfileLoop::Segments(segments) => segments,
                PlanarProfileLoop::Circle { .. } => {
                    return Err(exact_brep_profile_error(
                        profile,
                        "named profile segments require a line/arc boundary",
                    ));
                }
            }
        }
        ExactBRepPlanarGeometry::Region { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "named profile segments do not yet support holes",
            ));
        }
        _ => {
            return Err(exact_brep_profile_error(
                profile,
                "named profile segments require a line/arc boundary",
            ));
        }
    };
    if segments.len() != profile.segment_entity_ids.len() {
        return Err(exact_brep_profile_error(
            profile,
            "named profile segment identities do not match its geometry",
        ));
    }
    Ok(profile
        .segment_entity_ids
        .iter()
        .zip(segments)
        .map(|(entity_id, segment)| NamedSegment::new(format!("segment_{entity_id}"), segment))
        .collect())
}

fn exact_brep_profile_face_name(reference: &ExactBRepProfileFaceReference) -> String {
    match reference {
        ExactBRepProfileFaceReference::Start => "start".to_owned(),
        ExactBRepProfileFaceReference::End => "end".to_owned(),
        ExactBRepProfileFaceReference::Segment { entity_id, .. } => {
            format!("segment_{entity_id}")
        }
        ExactBRepProfileFaceReference::NamedResult { name } => name.clone(),
    }
}

fn exact_brep_named_profile_body(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Result<(ExactOpOutput, Vec<String>), ketchup_exact::GeometryError> {
    if exact_brep_profile_is_circle(profile) {
        return exact_brep_named_circle_body(backend, profile, interval);
    }
    let named = backend
        .named_extrude(
            &exact_brep_named_segments(profile)?,
            0.0,
            interval.length_mm(),
        )
        .map_err(|error| exact_brep_profile_error(profile, &error.to_string()))?;
    let (local, names) = named.into_output_and_names();
    let matrix = extrusion_matrix(profile, interval);
    let output = transform_named_output(backend, &local, &matrix)?;
    Ok((output, names))
}

/// Publishes program face names as the body's face history, one record per
/// face. Extrusion caps keep the standard `extrusion.top`/`extrusion.bottom`
/// roles the interactive tools resolve against (`end`/`start` in a program);
/// every other face is exposed under its program name.
fn exact_brep_expose_face_names(output: &mut ExactOpOutput, names: &[String], extrusion: bool) {
    let mut exposed = BTreeSet::new();
    output.topology_history.retain_mut(|record| {
        let Some(face) = record.output_face_ordinal else {
            return true;
        };
        let Some(name) = names.get(face as usize) else {
            return false;
        };
        if !exposed.insert(face) {
            return false;
        }
        let has_standard_role = [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::LinearSide,
            ExactFaceRole::ArcSide,
            ExactFaceRole::CircleSide,
        ]
        .into_iter()
        .any(|role| {
            record.semantic_role.as_deref() == Some(role.semantic_role())
                && record.source_element_id == role.source_element_id()
        });
        if has_standard_role {
            return true;
        }
        let (role, source) = match name.as_str() {
            "end" if extrusion => (
                ExactFaceRole::Top.semantic_role(),
                ExactFaceRole::Top.source_element_id(),
            ),
            "start" if extrusion => (
                ExactFaceRole::Bottom.semantic_role(),
                ExactFaceRole::Bottom.source_element_id(),
            ),
            name => (name, name),
        };
        record.semantic_role = Some(role.to_owned());
        record.source_element_id = source.to_owned();
        true
    });
}

/// Nodes whose faces a later node references by program name. Only these are
/// evaluated through the naming path; every other body keeps the standard
/// evaluation and its host-issued topology references.
fn exact_brep_nodes_needing_names(graph: &ExactBRepGraph) -> Vec<bool> {
    let mut needs_names = vec![false; graph.nodes.len()];
    for (index, node) in graph.nodes.iter().enumerate().rev() {
        let (target, consumes_names) = match &node.operation {
            ExactBRepOperation::FaceOffset {
                target,
                profile_face,
                ..
            } => (target, profile_face.is_some()),
            ExactBRepOperation::EdgeFinish {
                target,
                profile_edges,
                ..
            } => (target, !profile_edges.is_empty()),
            ExactBRepOperation::Shell {
                target,
                profile_faces,
                ..
            } => (target, !profile_faces.is_empty()),
            ExactBRepOperation::ProfileCut { target, .. }
            | ExactBRepOperation::RigidTransform { target, .. }
            | ExactBRepOperation::Scale { target, .. } => (target, false),
            ExactBRepOperation::Boolean { target, tool, .. } => {
                // The tool's own names become `tool_name.face` in the result.
                if needs_names[index]
                    && let Some(slot) = needs_names.get_mut(tool.0 as usize)
                {
                    *slot = true;
                }
                (target, false)
            }
            _ => continue,
        };
        if (consumes_names || needs_names[index])
            && let Some(slot) = needs_names.get_mut(target.0 as usize)
        {
            *slot = true;
        }
    }
    needs_names
}

/// Whether every face this profile sweeps can carry a program name: exactly
/// one entity identity per swept boundary segment. Profiles outside this set
/// (holes, open or spline geometry, a revolved circle, a circle assembled from
/// two arcs) keep the unnamed evaluation; a later named reference to them
/// fails explicitly.
fn exact_brep_profile_is_nameable(profile: &ExactBRepProfile, circle_allowed: bool) -> bool {
    let ids = profile.segment_entity_ids.len();
    match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary { closed, segments } => {
            *closed && ids > 0 && ids == segments.len()
        }
        ExactBRepPlanarGeometry::Circle { .. } => circle_allowed && ids == 1,
        ExactBRepPlanarGeometry::Region { outer, holes } if holes.is_empty() => match outer {
            ExactBRepPlanarLoop::Circle { .. } => circle_allowed && ids == 1,
            ExactBRepPlanarLoop::Boundary { segments } => ids > 0 && ids == segments.len(),
        },
        _ => false,
    }
}

fn exact_brep_profile_is_circle(profile: &ExactBRepProfile) -> bool {
    match &profile.geometry {
        ExactBRepPlanarGeometry::Circle { .. } => true,
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            holes.is_empty() && matches!(outer, ExactBRepPlanarLoop::Circle { .. })
        }
        _ => false,
    }
}

/// A full circle is one profile entity and sweeps one cylindrical wall, so the
/// wall carries that entity's name and the caps are `start` and `end` by their
/// position along the extrusion. Any other topology is rejected, never guessed.
fn exact_brep_named_circle_body(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Result<(ExactOpOutput, Vec<String>), ketchup_exact::GeometryError> {
    let [entity_id] = profile.segment_entity_ids.as_slice() else {
        return Err(exact_brep_profile_error(
            profile,
            "named circle profile must have exactly one segment identity",
        ));
    };
    let output = exact_brep_profile_body(backend, profile, interval)?;
    let origin = profile.frame().origin;
    let direction = Vec3::from(interval.direction());
    let along = |point: Point3| (Vec3::new(point.x, point.y, point.z) - origin).dot(direction);
    let tolerance = APPROXIMATION * interval.length_mm().max(1.0);
    let faces = &output.body.topology.faces;
    let mut names = vec![String::new(); faces.len()];
    let mut counts = [0_usize; 3];
    for face in faces {
        let slot = names.get_mut(face.ordinal as usize).ok_or_else(|| {
            exact_brep_profile_error(profile, "circle extrusion face ordinal is out of range")
        })?;
        let position = along(face.centroid_mm);
        let (name, index) = match face.surface_kind.as_str() {
            "cylinder" => (format!("segment_{entity_id}"), 0),
            "plane" if (position - interval.start_mm()).abs() <= tolerance => {
                ("start".to_owned(), 1)
            }
            "plane" if (position - interval.end_mm()).abs() <= tolerance => ("end".to_owned(), 2),
            _ => {
                return Err(exact_brep_profile_error(
                    profile,
                    "circle extrusion produced a face that is neither its wall nor a cap",
                ));
            }
        };
        *slot = name;
        counts[index] += 1;
    }
    if faces.len() != 3 || counts != [1, 1, 1] {
        return Err(exact_brep_profile_error(
            profile,
            "circle extrusion must have exactly one wall and two caps",
        ));
    }
    Ok((output, names))
}

fn exact_brep_named_revolve(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
) -> Result<(ExactOpOutput, Vec<String>), ketchup_exact::GeometryError> {
    let named = backend
        .named_revolve(
            &exact_brep_named_segments(profile)?,
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
        )
        .map_err(|error| exact_brep_profile_error(profile, &error.to_string()))?;
    let (local, names) = named.into_output_and_names();
    let matrix = profile.frame().to_affine().to_row_major();
    let output = transform_named_output(backend, &local, &matrix)?;
    Ok((output, names))
}

fn exact_brep_planar_loop(planar_loop: &ExactBRepPlanarLoop) -> PlanarProfileLoop {
    match planar_loop {
        ExactBRepPlanarLoop::Boundary { segments } => PlanarProfileLoop::Segments(
            segments
                .iter()
                .map(|segment| match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } => PlanarProfileSegment::Line {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => PlanarProfileSegment::CircularArc {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                        center_mm: center_bits.map(f64::from_bits),
                        clockwise: *clockwise,
                    },
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } => PlanarProfileSegment::CubicBezier {
                        start_mm: start_bits.map(f64::from_bits),
                        control_1_mm: control_1_bits.map(f64::from_bits),
                        control_2_mm: control_2_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                })
                .collect(),
        ),
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => PlanarProfileLoop::Circle {
            center_mm: center_bits.map(f64::from_bits),
            radius_mm: f64::from_bits(*radius_bits),
        },
    }
}

fn exact_brep_profile_body(
    backend: &ExactBackend,
    profile: &ExactBRepProfile,
    interval: ExactBRepLinearInterval,
) -> Result<ExactOpOutput, ketchup_exact::GeometryError> {
    let distance_mm = interval.length_mm();
    let local = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments,
        } => {
            let segments = segments
                .iter()
                .map(|segment| match segment {
                    ExactBRepPlanarSegment::Line {
                        start_bits,
                        end_bits,
                    } => PlanarProfileSegment::Line {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                    ExactBRepPlanarSegment::CircularArc {
                        start_bits,
                        end_bits,
                        center_bits,
                        clockwise,
                    } => PlanarProfileSegment::CircularArc {
                        start_mm: start_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                        center_mm: center_bits.map(f64::from_bits),
                        clockwise: *clockwise,
                    },
                    ExactBRepPlanarSegment::CubicBezier {
                        start_bits,
                        control_1_bits,
                        control_2_bits,
                        end_bits,
                    } => PlanarProfileSegment::CubicBezier {
                        start_mm: start_bits.map(f64::from_bits),
                        control_1_mm: control_1_bits.map(f64::from_bits),
                        control_2_mm: control_2_bits.map(f64::from_bits),
                        end_mm: end_bits.map(f64::from_bits),
                    },
                })
                .collect::<Vec<_>>();
            backend.extrude_mixed_profile(&segments, distance_mm)?
        }
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => backend.extrude_circle(CircleExtrudeSpec {
            center_mm: center_bits.map(f64::from_bits),
            radius_mm: f64::from_bits(*radius_bits),
            height_mm: distance_mm,
        })?,
        ExactBRepPlanarGeometry::Region { outer, holes } => backend.extrude_planar_region(
            &exact_brep_planar_loop(outer),
            &holes.iter().map(exact_brep_planar_loop).collect::<Vec<_>>(),
            distance_mm,
        )?,
        ExactBRepPlanarGeometry::Boundary { closed: false, .. }
        | ExactBRepPlanarGeometry::Spline { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact extrusion requires a closed line/arc/circle profile",
            ));
        }
    };
    let matrix = extrusion_matrix(profile, interval);
    if matrix
        == [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    {
        Ok(local)
    } else {
        backend.transform_body(&local.body, &matrix)
    }
}

fn exact_brep_graph_error(
    graph: &ExactBRepGraph,
    diagnostic: &str,
) -> ketchup_exact::GeometryError {
    ketchup_exact::GeometryError {
        code: ketchup_exact::GeometryErrorCode::InvalidParameter,
        diagnostic: diagnostic.to_owned(),
        operation: "evaluate_exact_brep_graph",
        input_digest: graph.graph_digest.clone(),
        backend_fingerprint: ketchup_exact::backend_fingerprint(),
    }
}

fn exact_brep_profile_error(
    profile: &ExactBRepProfile,
    diagnostic: &str,
) -> ketchup_exact::GeometryError {
    ketchup_exact::GeometryError {
        code: ketchup_exact::GeometryErrorCode::InvalidProfile,
        diagnostic: diagnostic.to_owned(),
        operation: "evaluate_exact_brep_profile",
        input_digest: sha256_hex(format!("profile:{}", profile.id.0).as_bytes()),
        backend_fingerprint: ketchup_exact::backend_fingerprint(),
    }
}

/// Fixed angular deflection so the same part always meshes identically, in
/// display meshes too: no chord of a curved face turns through more.
pub const STEP_MESH_ANGULAR_DEFLECTION: f64 = 0.35;
// not a tolerance: display mesh chord height relative to the part diagonal.
const DISPLAY_MESH_RELATIVE_DEFLECTION: f64 = 1.0e-3;
// not a tolerance: the finest display mesh chord height.
const DISPLAY_MESH_MIN_DEFLECTION_MM: f64 = 1.0e-3;

fn step_import_result_fingerprint(
    source_sha256: &str,
    output: &ketchup_exact::ExactOpOutput,
) -> String {
    let [min, max] = bounds_mm(output);
    let measures = [
        output.body.topology.volume_mm3,
        min[0],
        min[1],
        min[2],
        max[0],
        max[1],
        max[2],
    ];
    imported_fingerprint(
        &imported_fingerprint_topology(source_sha256, output),
        &measures,
    )
}

/// What an imported result fingerprint hashes ahead of the volume and bounds.
fn imported_fingerprint_topology(source_sha256: &str, output: &ExactOpOutput) -> String {
    let topology = &output.body.topology;
    format!(
        "{source_sha256}:{}:{}:{}:{}:{}:{}:{}",
        output.backend_fingerprint,
        output.tolerance_report.profile,
        topology.vertex_count,
        topology.edge_count,
        topology.face_count,
        topology.shell_count,
        topology.solid_count,
    )
}

/// `measures` are the volume, then the bounds' min and max corners.
fn imported_fingerprint(topology: &str, measures: &[f64; 7]) -> String {
    let mut signature = topology.to_owned();
    for measure in measures {
        signature.push_str(&format!(":{}", measure.to_bits()));
    }
    ketchup_geometry::reference::fnv1a64_fingerprint(&signature)
}

/// Whether `expected` names this imported body: its own fingerprint, or the
/// one of the same topology whose volume and bounds lie a few last bits away,
/// as a document saved where libm rounded differently recorded it (ADR 0010).
/// The fingerprint hashes those bits, so the neighbours are searched.
fn imported_result_matches(source_sha256: &str, output: &ExactOpOutput, expected: &str) -> bool {
    let [min, max] = bounds_mm(output);
    let measures = [
        output.body.topology.volume_mm3,
        min[0],
        min[1],
        min[2],
        max[0],
        max[1],
        max[2],
    ];
    imported_fingerprint_near(
        &imported_fingerprint_topology(source_sha256, output),
        &measures,
        expected,
    )
}

fn imported_fingerprint_near(topology: &str, measures: &[f64; 7], expected: &str) -> bool {
    use ketchup_model::import::{IMPORTED_BOUNDS_ULPS, IMPORTED_MEASURE_ULPS, ulps_from};
    if imported_fingerprint(topology, measures) == expected {
        return true;
    }
    let reach = |slot: usize| {
        (if slot == 0 {
            IMPORTED_MEASURE_ULPS
        } else {
            IMPORTED_BOUNDS_ULPS
        })
        .cast_signed()
    };
    let mut steps = [0_i64; 7];
    for (slot, step) in steps.iter_mut().enumerate() {
        *step = -reach(slot);
    }
    loop {
        let mut candidate = *measures;
        let reachable = candidate.iter_mut().zip(steps).all(|(measure, step)| {
            ulps_from(*measure, step)
                .map(|near| *measure = near)
                .is_some()
        });
        if reachable && imported_fingerprint(topology, &candidate) == expected {
            return true;
        }
        let mut slot = 0;
        loop {
            if slot == steps.len() {
                return false;
            }
            if steps[slot] < reach(slot) {
                steps[slot] += 1;
                break;
            }
            steps[slot] = -reach(slot);
            slot += 1;
        }
    }
}

/// Copies a declared source into a private file after proving its SHA-256.
fn verified_copy(
    source: &SourceFile,
    label: &str,
    operation: &'static str,
) -> Result<tempfile::NamedTempFile, WorkerFailure> {
    let failure = |diagnostic: &str| backend_failure(operation, diagnostic);
    let extension = label.to_ascii_lowercase();
    let mut copy = tempfile::Builder::new()
        .prefix(&format!("ketchup-verified-{extension}-"))
        .suffix(&format!(".{extension}"))
        .tempfile()
        .map_err(|error| failure(&error.to_string()))?;
    // The copy is hashed as it is written, so the bytes proven are the bytes
    // OCCT reads, and a source of any size is never held whole.
    let copied =
        crate::copy_regular_file_hashed(&source.path, &format!("{label} source"), &mut copy)
            .map_err(|error| failure(&error.to_string()))?;
    if !is_sha256_hex(&source.sha256) || copied != source.sha256 {
        return Err(failure(&format!(
            "{label} source bytes do not match the declared SHA-256"
        )));
    }
    copy.flush().map_err(|error| failure(&error.to_string()))?;
    Ok(copy)
}

fn verified_step_copy(
    source: &SourceFile,
    operation: &'static str,
) -> Result<tempfile::NamedTempFile, WorkerFailure> {
    verified_copy(source, "STEP", operation)
}

fn verified_iges_copy(
    source: &SourceFile,
    operation: &'static str,
) -> Result<tempfile::NamedTempFile, WorkerFailure> {
    verified_copy(source, "IGES", operation)
}

fn temp_path(file: &tempfile::NamedTempFile) -> String {
    file.path().to_string_lossy().into_owned()
}

fn body_kind(output: &ExactOpOutput) -> &'static str {
    if output.body.topology.solid_count == 0 {
        "surface"
    } else {
        "solid"
    }
}

fn face_area_mm2(output: &ExactOpOutput) -> f64 {
    output
        .body
        .topology
        .faces
        .iter()
        .map(|face| face.area_mm2)
        .sum()
}

fn topology_counts(output: &ExactOpOutput) -> [u32; 5] {
    let topology = &output.body.topology;
    [
        topology.vertex_count,
        topology.edge_count,
        topology.face_count,
        topology.shell_count,
        topology.solid_count,
    ]
}

fn bounds_mm(output: &ExactOpOutput) -> [[f64; 3]; 2] {
    let bounds = output.body.topology.bounds_mm;
    [
        [bounds.min.x, bounds.min.y, bounds.min.z],
        [bounds.max.x, bounds.max.y, bounds.max.z],
    ]
}

fn import_part_reply(
    source_sha256: &str,
    output: &ExactOpOutput,
    source_unit: &str,
) -> WorkerReply {
    WorkerReply::ImportPart(WorkerImportPart {
        result_fingerprint: step_import_result_fingerprint(source_sha256, output),
        body_kind: body_kind(output).to_owned(),
        topology_counts: topology_counts(output),
        area_mm2: face_area_mm2(output),
        volume_mm3: output.body.topology.volume_mm3,
        bounds_mm: bounds_mm(output),
        source_unit: source_unit.to_owned(),
        backend: output.backend_fingerprint.to_owned(),
        tolerance: output.tolerance_report.profile.to_owned(),
    })
}

/// Inspects every XDE part of a verified source and returns the whole manifest.
fn xde_evidence_reply(
    source: &SourceFile,
    source_byte_len: u64,
    source_unit: &str,
    manifest: ketchup_exact::StepXdeManifest,
    import_part: impl Fn(u32) -> Result<ExactOpOutput, ketchup_exact::GeometryError>,
) -> WorkerResult {
    let mut parts = Vec::with_capacity(manifest.parts.len());
    for part in manifest.parts {
        let output = import_part(part.index).map_err(|error| geometry_failure(&error))?;
        parts.push(StepXdeWorkerPart {
            index: part.index,
            name: part.name,
            name_from_source: part.name_from_source,
            color: part.color,
            result_fingerprint: step_import_result_fingerprint(&source.sha256, &output),
            body_kind: body_kind(&output).to_owned(),
            solid_count: output.body.topology.solid_count,
            topology_counts: topology_counts(&output),
            area_mm2: face_area_mm2(&output),
            volume_mm3: output.body.topology.volume_mm3,
            bounds_mm: bounds_mm(&output),
            backend: output.backend_fingerprint.to_owned(),
            tolerance: output.tolerance_report.profile.to_owned(),
        });
    }
    Ok(WorkerReply::Xde(StepXdeWorkerEvidence {
        source_sha256: source.sha256.clone(),
        source_byte_len,
        source_unit: source_unit.to_owned(),
        parts,
        nodes: manifest
            .nodes
            .into_iter()
            .map(|node| StepXdeWorkerNode {
                id: node.id,
                parent_id: node.parent_id,
                part_index: node.part_index,
                name: node.name,
                name_from_source: node.name_from_source,
                color: node.color,
                transform: node.transform,
            })
            .collect(),
    }))
}

fn iges_source_unit(
    raw_unit: Option<String>,
    operation: &'static str,
) -> Result<&'static str, WorkerFailure> {
    let Some(raw_unit) = raw_unit else {
        return Err(backend_failure(
            operation,
            "IGES source has missing or ambiguous representation length units",
        ));
    };
    match raw_unit.as_str() {
        "mm" | "millimetre" | "millimeter" => Ok("millimetre"),
        "cm" | "centimetre" | "centimeter" => Ok("centimetre"),
        "m" | "metre" | "meter" => Ok("metre"),
        "in" | "inch" => Ok("inch"),
        "ft" | "foot" => Ok("foot"),
        _ => Err(backend_failure(
            operation,
            "IGES source declares an unsupported length unit",
        )),
    }
}

fn step_source_unit(
    backend: &ExactBackend,
    source_path: &str,
    operation: &'static str,
) -> Result<String, WorkerFailure> {
    backend.step_length_unit_name(source_path).ok_or_else(|| {
        backend_failure(
            operation,
            "STEP source has missing or ambiguous representation length units",
        )
    })
}

fn m21_iges_xde_inspection_response(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
) -> WorkerResult {
    let (document, source_byte_len) =
        read.get(backend, source, XdeFormat::Iges, "inspect_iges_xde")?;
    let source_unit = iges_source_unit(document.length_unit_name(), "inspect_iges_xde")?;
    let manifest = document
        .manifest()
        .map_err(|error| backend_failure("inspect_iges_xde", &error.to_string()))?;
    xde_evidence_reply(source, source_byte_len, source_unit, manifest, |index| {
        document.part(index)
    })
}

fn m21_iges_part_inspection_response(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
    part_index: Option<u32>,
) -> WorkerResult {
    const OPERATION: &str = "inspect_iges_part";
    let (raw_unit, output) = match part_index {
        Some(index) => {
            let (document, _) = read.get(backend, source, XdeFormat::Iges, OPERATION)?;
            (document.length_unit_name(), document.part(index))
        }
        None => {
            let copy = verified_iges_copy(source, OPERATION)?;
            let source_path = temp_path(&copy);
            (
                backend.iges_length_unit_name(&source_path),
                backend.import_iges(&source_path),
            )
        }
    };
    let source_unit = iges_source_unit(raw_unit, OPERATION)?;
    let output = output.map_err(|error| geometry_failure(&error))?;
    Ok(import_part_reply(&source.sha256, &output, source_unit))
}

/// Tessellates an imported body; the receipt repeats its result fingerprint so the
/// caller can only bind a mesh to the exact body it already committed to.
fn imported_mesh_response(
    backend: &ExactBackend,
    source: &SourceFile,
    output: ExactOpOutput,
    output_path: &Path,
    label: &str,
    operation: &'static str,
) -> WorkerResult {
    let bounds = output.body.topology.bounds_mm;
    let diagonal = ((bounds.max.x - bounds.min.x).powi(2)
        + (bounds.max.y - bounds.min.y).powi(2)
        + (bounds.max.z - bounds.min.z).powi(2))
    .sqrt();
    if !diagonal.is_finite() || diagonal <= 0.0 {
        return Err(backend_failure(
            operation,
            &format!("{label} part has no measurable extent to tessellate"),
        ));
    }
    let deflection =
        (diagonal * DISPLAY_MESH_RELATIVE_DEFLECTION).max(DISPLAY_MESH_MIN_DEFLECTION_MM);
    let mesh = backend
        .tessellate_body(
            &output.body,
            deflection,
            STEP_MESH_ANGULAR_DEFLECTION,
            MAX_STEP_MESH_TRIANGLES,
        )
        .map_err(|error| geometry_failure(&error))?;
    write_display_mesh(
        mesh,
        output_path,
        &step_import_result_fingerprint(&source.sha256, &output),
        operation,
    )
}

/// The body an import request names: XDE part `part_index` of the source read
/// once for the worker, or without an index the whole file read on its own.
fn imported_body(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
    format: XdeFormat,
    part_index: Option<u32>,
    operation: &'static str,
) -> Result<ExactOpOutput, WorkerFailure> {
    match part_index {
        Some(index) => read.get(backend, source, format, operation)?.0.part(index),
        None => {
            let copy = verified_copy(source, format.label(), operation)?;
            match format {
                XdeFormat::Step => backend.import_step(&temp_path(&copy)),
                XdeFormat::Iges => backend.import_iges(&temp_path(&copy)),
            }
        }
    }
    .map_err(|error| geometry_failure(&error))
}

fn m21_iges_part_mesh_response(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
    part_index: Option<u32>,
    output_path: &Path,
) -> WorkerResult {
    let output = imported_body(
        backend,
        read,
        source,
        XdeFormat::Iges,
        part_index,
        "tessellate_iges_part",
    )?;
    imported_mesh_response(
        backend,
        source,
        output,
        output_path,
        "IGES",
        "tessellate_iges_part",
    )
}

/// A part of the source to write: its index in the manifest and its name.
type SourcePart = (u32, String);

/// The flat IGES roots of a STEP XDE `manifest`: one per occurrence of a part,
/// placed in the world, and the source parts (index and name) they place in
/// the order their files are written. Each part is written once, however many occurrences
/// place it: an assembly of 750 copies of a few parts wrote 750 STEP files
/// (review 2026-10-18 EX-9).
fn flat_iges_roots(
    manifest: &ketchup_exact::StepXdeManifest,
    operation: &'static str,
) -> Result<(Vec<SourcePart>, Vec<StepXdeExportNode>), WorkerFailure> {
    let mut world_transforms: Vec<Transform> = Vec::with_capacity(manifest.nodes.len());
    let mut inherited_colors: Vec<Option<[u8; 3]>> = Vec::with_capacity(manifest.nodes.len());
    let mut source_parts = Vec::new();
    let mut written = BTreeMap::<u32, u32>::new();
    let mut next_part = 0_u32;
    let mut nodes = Vec::new();
    for (index, node) in manifest.nodes.iter().enumerate() {
        if node.id as usize != index
            || node
                .parent_id
                .is_some_and(|parent| parent as usize >= index)
        {
            return Err(backend_failure(
                operation,
                "STEP XDE hierarchy is not canonical parent-before order",
            ));
        }
        let Some(local) = Transform::from_matrix(node.transform)
            .ok()
            .filter(|transform| transform.rigid_inverse().is_some())
        else {
            return Err(backend_failure(
                operation,
                "STEP XDE hierarchy contains a non-rigid transform",
            ));
        };
        let world = node.parent_id.map_or(local, |parent| {
            world_transforms[parent as usize].compose(local)
        });
        let inherited = node.color.or_else(|| {
            node.parent_id
                .and_then(|parent| inherited_colors[parent as usize])
        });
        world_transforms.push(world);
        inherited_colors.push(inherited);
        let Some(source_part_index) = node.part_index else {
            continue;
        };
        let Some(source_part) = manifest.parts.get(source_part_index as usize) else {
            return Err(backend_failure(
                operation,
                "STEP XDE occurrence references a missing part",
            ));
        };
        let part_index = *written.entry(source_part_index).or_insert_with(|| {
            source_parts.push((source_part_index, source_part.name.clone()));
            next_part += 1;
            next_part - 1
        });
        nodes.push(StepXdeExportNode {
            parent_id: None,
            part_index: Some(part_index),
            name: node.name.clone(),
            color: inherited.or(source_part.color),
            transform: *world.matrix(),
        });
    }
    if nodes.is_empty() || nodes.len() > crate::MAX_STEP_ASSEMBLY_NODES {
        return Err(backend_failure(
            operation,
            "STEP XDE source has no bounded leaf occurrences",
        ));
    }
    Ok((source_parts, nodes))
}

fn m21_step_xde_to_iges_response(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
    output_path: &Path,
) -> WorkerResult {
    const OPERATION: &str = "convert_step_xde_to_iges";
    let output_path = utf8_path(output_path)?;
    let (document, _) = read.get(backend, source, XdeFormat::Step, OPERATION)?;
    let manifest = document
        .manifest()
        .map_err(|error| backend_failure(OPERATION, &error.to_string()))?;
    let (source_parts, nodes) = flat_iges_roots(&manifest, OPERATION)?;
    let directory = tempfile::Builder::new()
        .prefix("ketchup-iges-flat-roots-")
        .tempdir()
        .map_err(|error| backend_failure(OPERATION, &error.to_string()))?;
    let mut parts = Vec::with_capacity(source_parts.len());
    for (source_part_index, name) in source_parts {
        let imported = document
            .part(source_part_index)
            .map_err(|error| geometry_failure(&error))?;
        let part_path = directory.path().join(format!("part-{}.step", parts.len()));
        backend
            .export_step(&imported.body, &part_path.to_string_lossy())
            .map_err(|error| geometry_failure(&error))?;
        parts.push(StepXdeExportPart {
            path: part_path.to_string_lossy().into_owned(),
            name,
        });
    }
    backend
        .export_iges_xde_assembly(&parts, &nodes, output_path)
        .map_err(|error| geometry_failure(&error))?;
    let written = backend
        .open_iges_xde(output_path)
        .map_err(|error| backend_failure("reread_iges_xde_output", &error.to_string()))?;
    let reread = written
        .manifest()
        .map_err(|error| backend_failure("reread_iges_xde_output", &error.to_string()))?;
    // IGES has no shared definitions: each occurrence is a root of its own.
    if reread.parts.len() != nodes.len()
        || reread.nodes.len() != nodes.len()
        || reread.nodes.iter().any(|node| node.parent_id.is_some())
        || reread.nodes.iter().zip(&nodes).any(|(actual, expected)| {
            actual.name != expected.name || actual.color != expected.color
        })
    {
        return Err(backend_failure(
            "reread_iges_xde_output",
            "IGES output lost a flat root, name, or color",
        ));
    }
    Ok(WorkerReply::Exported(ExportReceipt {
        result_fingerprint: iges_output_fingerprint(&written, &reread, &source.sha256)?,
        sha256: None,
    }))
}

/// The fingerprint of a written IGES file: of every root's body as it reads back, in
/// order, so a root other than the first that changes changes it too.
fn iges_output_fingerprint(
    written: &ketchup_exact::StepXdeDocument,
    manifest: &ketchup_exact::StepXdeManifest,
    source_sha256: &str,
) -> Result<String, WorkerFailure> {
    let signature = manifest
        .parts
        .iter()
        .map(|part| {
            let body = written
                .part(part.index)
                .map_err(|error| geometry_failure(&error))?;
            Ok(step_import_result_fingerprint(source_sha256, &body))
        })
        .collect::<Result<Vec<_>, WorkerFailure>>()?;
    Ok(ketchup_geometry::reference::fnv1a64_fingerprint(
        &signature.join(":"),
    ))
}

fn m21_step_xde_inspection_response(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
) -> WorkerResult {
    let (document, source_byte_len) =
        read.get(backend, source, XdeFormat::Step, "inspect_step_xde")?;
    let source_unit = step_document_unit(document, "inspect_step_xde")?;
    let manifest = document
        .manifest()
        .map_err(|error| backend_failure("inspect_step_xde", &error.to_string()))?;
    xde_evidence_reply(source, source_byte_len, &source_unit, manifest, |index| {
        document.part(index)
    })
}

fn step_document_unit(
    document: &ketchup_exact::StepXdeDocument,
    operation: &'static str,
) -> Result<String, WorkerFailure> {
    document.length_unit_name().ok_or_else(|| {
        backend_failure(
            operation,
            "STEP source has missing or ambiguous representation length units",
        )
    })
}

fn m21_step_part_inspection_response(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
    part_index: Option<u32>,
) -> WorkerResult {
    const OPERATION: &str = "inspect_step_part";
    let (source_unit, output) = match part_index {
        Some(index) => {
            let (document, _) = read.get(backend, source, XdeFormat::Step, OPERATION)?;
            (
                step_document_unit(document, OPERATION)?,
                document.part(index),
            )
        }
        None => {
            let copy = verified_step_copy(source, OPERATION)?;
            let source_path = temp_path(&copy);
            (
                step_source_unit(backend, &source_path, OPERATION)?,
                backend.import_step(&source_path),
            )
        }
    };
    let output = output.map_err(|error| geometry_failure(&error))?;
    Ok(import_part_reply(&source.sha256, &output, &source_unit))
}

fn read_step_output(path: &str, operation: &str) -> Result<Vec<u8>, WorkerFailure> {
    let file = std::fs::File::open(path)
        .map_err(|error| backend_failure(operation, &error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| backend_failure(operation, &error.to_string()))?;
    if !metadata.is_file() {
        return Err(backend_failure(
            operation,
            "STEP output is not a regular file",
        ));
    }
    let mut bytes = crate::reserve_whole(metadata.len(), "STEP output")
        .map_err(|error| backend_failure(operation, &error.to_string()))?;
    (&file)
        .read_to_end(&mut bytes)
        .map_err(|error| backend_failure(operation, &error.to_string()))?;
    Ok(bytes)
}

fn m21_step_xde_part_export_response(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
    part_index: u32,
    result_fingerprint: &str,
    output_path: &Path,
) -> WorkerResult {
    let output_path = utf8_path(output_path)?;
    let output = imported_body(
        backend,
        read,
        source,
        XdeFormat::Step,
        Some(part_index),
        "export_step_xde_part",
    )?;
    if !imported_result_matches(&source.sha256, &output, result_fingerprint) {
        return Err(WorkerFailure::code("invalid_shape"));
    }
    backend
        .export_step(&output.body, output_path)
        .map_err(|error| geometry_failure(&error))?;
    let exported = read_step_output(output_path, "export_step_xde_part")?;
    Ok(WorkerReply::Exported(ExportReceipt {
        result_fingerprint: result_fingerprint.to_owned(),
        sha256: Some(sha256_hex(&exported)),
    }))
}

fn m21_step_part_mesh_response(
    backend: &ExactBackend,
    read: &mut SourceDocument,
    source: &SourceFile,
    part_index: Option<u32>,
    output_path: &Path,
) -> WorkerResult {
    let output = imported_body(
        backend,
        read,
        source,
        XdeFormat::Step,
        part_index,
        "tessellate_step_part",
    )?;
    imported_mesh_response(
        backend,
        source,
        output,
        output_path,
        "STEP",
        "tessellate_step_part",
    )
}

fn m21_step_assembly_response(
    backend: &ExactBackend,
    manifest: &StepAssemblyManifest,
    sources: &[PathBuf],
    output_path: &Path,
) -> WorkerResult {
    let output_path = utf8_path(output_path)?;
    if manifest.schema != "ketchup.step-xde-assembly.v2"
        || !ketchup_model::document::Snapshot::is_canonical_digest(&manifest.source_digest)
        || manifest.parts.is_empty()
        || manifest.parts.len() > crate::MAX_STEP_ASSEMBLY_NODES
        || manifest.nodes.is_empty()
        || manifest.nodes.len() > crate::MAX_STEP_ASSEMBLY_NODES
    {
        return Err(backend_failure(
            "verify_step_manifest",
            "STEP XDE assembly manifest schema, digest, parts, or nodes are invalid",
        ));
    }
    if sources.len() != manifest.parts.len() {
        return Err(backend_failure(
            "verify_step_manifest",
            "STEP XDE assembly source count differs from the manifest",
        ));
    }

    let mut verified_sources = Vec::with_capacity(sources.len());
    let mut export_parts = Vec::with_capacity(sources.len());
    for (manifest_part, source_path) in manifest.parts.iter().zip(sources) {
        if manifest_part.document_id != manifest.document_id
            || manifest_part.source_revision != manifest.source_revision
            || manifest_part.source_digest != manifest.source_digest
            || manifest_part.definition_id == 0
            || manifest_part.producer_feature_id == 0
            || manifest_part.name.is_empty()
            || manifest_part.name.len() > crate::MAX_STEP_ASSEMBLY_NAME_BYTES
            || manifest_part.name.chars().any(char::is_control)
            || !ketchup_geometry::reference::is_fnv1a64_fingerprint(
                &manifest_part.expected_result_fingerprint,
            )
            || !ketchup_geometry::reference::is_fnv1a64_fingerprint(
                &manifest_part.imported_result_fingerprint,
            )
            || !is_sha256_hex(&manifest_part.source_sha256)
        {
            return Err(backend_failure(
                "verify_step_part",
                "STEP part manifest identity, name, fingerprint, or SHA-256 is malformed",
            ));
        }
        let copy = verified_step_copy(
            &SourceFile {
                sha256: manifest_part.source_sha256.clone(),
                path: source_path.clone(),
            },
            "verify_step_part",
        )?;
        let imported = backend
            .import_step(&temp_path(&copy))
            .map_err(|error| geometry_failure(&error))?;
        if step_import_result_fingerprint(&manifest_part.source_sha256, &imported)
            != manifest_part.imported_result_fingerprint
        {
            return Err(backend_failure(
                "verify_step_part",
                "STEP part reimport identity differs from the inspected manifest identity",
            ));
        }
        export_parts.push(StepXdeExportPart {
            path: temp_path(&copy),
            name: manifest_part.name.clone(),
        });
        verified_sources.push(copy);
    }

    let mut export_nodes = Vec::with_capacity(manifest.nodes.len());
    for (index, node) in manifest.nodes.iter().enumerate() {
        if node.id as usize != index
            || node.name.is_empty()
            || node.name.len() > crate::MAX_STEP_ASSEMBLY_NAME_BYTES
            || node.name.chars().any(char::is_control)
            || node
                .parent_id
                .is_some_and(|parent| parent as usize >= index)
            || node
                .part_index
                .is_some_and(|part| part as usize >= manifest.parts.len())
            || node
                .parent_id
                .is_some_and(|parent| manifest.nodes[parent as usize].part_index.is_some())
        {
            return Err(backend_failure(
                "verify_step_hierarchy",
                "STEP XDE assembly hierarchy is malformed",
            ));
        }
        export_nodes.push(StepXdeExportNode {
            parent_id: node.parent_id,
            part_index: node.part_index,
            name: node.name.clone(),
            color: node.color,
            transform: node.transform_bits.map(f64::from_bits),
        });
    }
    backend
        .export_step_xde_assembly(&export_parts, &export_nodes, output_path)
        .map_err(|error| geometry_failure(&error))?;
    let reread_xde = backend
        .open_step_xde(output_path)
        .and_then(|document| document.manifest())
        .map_err(|error| backend_failure("reread_step_xde_output", &error.to_string()))?;
    if reread_xde.parts.len() != manifest.parts.len()
        || reread_xde
            .nodes
            .iter()
            .filter(|node| node.part_index.is_some())
            .count()
            != manifest
                .nodes
                .iter()
                .filter(|node| node.part_index.is_some())
                .count()
    {
        return Err(backend_failure(
            "reread_step_xde_output",
            "STEP XDE output lost a part definition or occurrence",
        ));
    }
    let step_bytes = read_step_output(output_path, "read_step_output")?;
    let reread = backend
        .import_step(output_path)
        .map_err(|error| geometry_failure(&error))?;
    Ok(WorkerReply::Exported(ExportReceipt {
        result_fingerprint: reread.body.result_fingerprint.clone(),
        sha256: Some(sha256_hex(&step_bytes)),
    }))
}

fn geometry_failure(error: &ketchup_exact::GeometryError) -> WorkerFailure {
    WorkerFailure {
        code: error.code.as_str().to_owned(),
        detail: Some(FailureDetail {
            diagnostic: error.diagnostic.clone(),
            operation: error.operation.to_owned(),
            input_digest: error.input_digest.clone(),
            backend: error.backend_fingerprint.to_owned(),
        }),
    }
}

fn backend_failure(operation: &str, diagnostic: &str) -> WorkerFailure {
    WorkerFailure {
        code: ketchup_exact::GeometryErrorCode::BackendException
            .as_str()
            .to_owned(),
        detail: Some(FailureDetail {
            diagnostic: diagnostic.to_owned(),
            operation: operation.to_owned(),
            input_digest: sha256_hex(diagnostic.as_bytes()),
            backend: ketchup_exact::backend_fingerprint().to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0010: the drawer example's rail 9117663_3, saved on Windows with
    /// MSVC's libm, evaluates one bit away in volume under CORE-MATH. The
    /// fingerprint the document recorded still names it; one further off not.
    #[test]
    fn an_imported_fingerprint_a_few_last_bits_away_still_names_the_body() {
        let topology = "d18b54c6b8b5bcee522ab01167d04ee82dc04d01ae01ae1e8265779ced1038be:\
             occt-8.0.1:b8f597c677811d1f9f4d8a97f5ae2825c0353a42:r0-v1:\
             r0-v1:bbox=1e-6mm:volume_abs=1e-6mm3:volume_rel=1e-10:140:212:74:1:1";
        let bounds = [
            518.25,
            -31.0,
            -40.75,
            562.000_000_1,
            387.6,
            9.050_000_000_000_004,
        ];
        let measures = |volume: f64| {
            let mut measures = [volume; 7];
            measures[1..].copy_from_slice(&bounds);
            measures
        };
        let recorded = measures(74_890.358_324_392_06);
        let evaluated = measures(74_890.358_324_392_07);
        let recorded_fingerprint = "fnv1a64:6b9e8a10cebef52a";
        assert_eq!(
            imported_fingerprint(topology, &recorded),
            recorded_fingerprint
        );
        assert_eq!(
            imported_fingerprint(topology, &evaluated),
            "fnv1a64:7bb317a07566e095"
        );
        assert!(imported_fingerprint_near(
            topology,
            &evaluated,
            recorded_fingerprint
        ));

        let mut both = evaluated;
        both[3] = ketchup_model::import::ulps_from(both[3], -1).unwrap();
        assert!(imported_fingerprint_near(
            topology,
            &both,
            recorded_fingerprint
        ));
        let mut far = recorded;
        far[0] = ketchup_model::import::ulps_from(far[0], 9).unwrap();
        assert!(!imported_fingerprint_near(
            topology,
            &far,
            recorded_fingerprint
        ));
        let mut far = recorded;
        far[6] = ketchup_model::import::ulps_from(far[6], 2).unwrap();
        assert!(!imported_fingerprint_near(
            topology,
            &far,
            recorded_fingerprint
        ));
        assert!(!imported_fingerprint_near(
            &topology.replace(":74:", ":75:"),
            &evaluated,
            recorded_fingerprint
        ));

        // EX-60: a body bounded at -0.0, recorded a few bits off in volume, or
        // at +0.0, is the recorded one as the window takes it.
        let mut zero = recorded;
        zero[1] = -0.0;
        let zero_fingerprint = imported_fingerprint(topology, &zero);
        for (slot, value) in [(0, evaluated[0]), (1, 0.0)] {
            let mut near = zero;
            near[slot] = value;
            assert!(imported_fingerprint_near(
                topology,
                &near,
                &zero_fingerprint
            ));
        }
    }

    #[test]
    fn step_output_reader_reads_a_file_past_32_mib_whole() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.step");
        let file = std::fs::File::create(&path).unwrap();
        let length = 32 * 1024 * 1024 + 1;
        file.set_len(length).unwrap();

        let bytes = read_step_output(path.to_str().unwrap(), "test_step_output").unwrap();
        assert_eq!(bytes.len() as u64, length);
    }

    #[test]
    fn verified_step_and_iges_staging_never_writes_beside_the_source() {
        let workspace = tempfile::tempdir().unwrap();
        let source_directory = workspace.path().join("read-only-media");
        std::fs::create_dir(&source_directory).unwrap();

        for (name, payload, label) in [
            ("part.step", b"STEP source".as_slice(), "STEP"),
            ("part.iges", b"IGES source".as_slice(), "IGES"),
        ] {
            let path = source_directory.join(name);
            std::fs::write(&path, payload).unwrap();
            let source = SourceFile {
                sha256: sha256_hex(payload),
                path: path.clone(),
            };
            let staged = verified_copy(&source, label, "test_import").unwrap();
            assert_ne!(staged.path().parent(), path.parent());
            assert_eq!(std::fs::read(staged.path()).unwrap(), payload);
            let forged = SourceFile {
                sha256: "0".repeat(64),
                path,
            };
            assert!(verified_copy(&forged, label, "test_import").is_err());
        }
    }

    // Review 2026-10-17 EX-2: inspection and conversion each parsed a two-part
    // source 4 times (unit, manifest, one per part).
    #[test]
    fn step_xde_inspection_and_iges_conversion_parse_the_source_once() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step");
        let source = SourceFile {
            sha256: sha256_hex(&std::fs::read(&path).unwrap()),
            path,
        };
        let backend = ExactBackend::new();
        let parses_during = |request: &dyn Fn() -> WorkerResult| {
            let before = ExactBackend::step_source_parses_on_this_thread();
            request().unwrap();
            ExactBackend::step_source_parses_on_this_thread() - before
        };

        let inspection = parses_during(&|| {
            let reply = m21_step_xde_inspection_response(
                &backend,
                &mut SourceDocument::default(),
                &source,
            )?;
            let WorkerReply::Xde(evidence) = &reply else {
                panic!("inspection answered {reply:?}");
            };
            assert_eq!(evidence.parts.len(), 2);
            Ok(reply)
        });

        let output = tempfile::tempdir().unwrap();
        let conversion = parses_during(&|| {
            m21_step_xde_to_iges_response(
                &backend,
                &mut SourceDocument::default(),
                &source,
                &output.path().join("flat.igs"),
            )
        });
        assert_eq!((inspection, conversion), (1, 1));
    }

    // Review 2026-10-18 EX-9: converting to IGES wrote a STEP file of a part
    // for every occurrence that places it. The corpus places one of its two
    // parts twice: two part files are written for three IGES roots.
    #[test]
    fn iges_conversion_writes_each_part_once_for_all_its_occurrences() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step");
        let manifest = ExactBackend::new()
            .open_step_xde(path.to_str().unwrap())
            .unwrap()
            .manifest()
            .unwrap();
        let (source_parts, nodes) = flat_iges_roots(&manifest, "test").unwrap();
        assert_eq!(source_parts.len(), 2);
        assert_eq!(nodes.len(), 3);
        assert!(
            nodes
                .iter()
                .all(|node| node.part_index.is_some_and(|index| index < 2))
        );

        let output = tempfile::tempdir().unwrap();
        let iges = output.path().join("flat.igs");
        m21_step_xde_to_iges_response(
            &ExactBackend::new(),
            &mut SourceDocument::default(),
            &SourceFile {
                sha256: sha256_hex(&std::fs::read(&path).unwrap()),
                path,
            },
            &iges,
        )
        .unwrap();
        let written = ExactBackend::new()
            .open_iges_xde(iges.to_str().unwrap())
            .unwrap()
            .manifest()
            .unwrap();
        assert_eq!((written.parts.len(), written.nodes.len()), (3, 3));
    }

    // Review 2026-10-21 P3: the receipt of an IGES conversion was the fingerprint of the
    // first root alone, so it said nothing of the others. It now covers every root.
    #[test]
    fn an_iges_conversion_receipt_covers_every_root_it_wrote() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step");
        let source = SourceFile {
            sha256: sha256_hex(&std::fs::read(&path).unwrap()),
            path,
        };
        let output = tempfile::tempdir().unwrap();
        let iges = output.path().join("flat.igs");
        let backend = ExactBackend::new();
        let reply =
            m21_step_xde_to_iges_response(&backend, &mut SourceDocument::default(), &source, &iges)
                .unwrap();
        let WorkerReply::Exported(receipt) = reply else {
            panic!("the conversion answered {reply:?}");
        };
        let written = backend.open_iges_xde(iges.to_str().unwrap()).unwrap();
        let roots = (0..3)
            .map(|index| {
                step_import_result_fingerprint(&source.sha256, &written.part(index).unwrap())
            })
            .collect::<Vec<_>>();
        assert_ne!(receipt.result_fingerprint, roots[0]);
        assert_eq!(
            receipt.result_fingerprint,
            ketchup_geometry::reference::fnv1a64_fingerprint(&roots.join(":"))
        );
    }

    // Review 2026-10-19 P3: the document a worker keeps was named after the
    // temporary copy it read, deleted right after, so the evidence of every part
    // and failure answered from it changed with each read of the same source.
    #[test]
    fn a_kept_source_document_is_named_by_its_source_not_its_deleted_copy() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step");
        let step = SourceFile {
            sha256: sha256_hex(&std::fs::read(&corpus).unwrap()),
            path: corpus,
        };
        let backend = ExactBackend::new();
        let digests = || {
            let mut read = SourceDocument::default();
            let (document, _) = read
                .get(&backend, &step, XdeFormat::Step, "inspect_step_part")
                .unwrap();
            let part = document.part(0).unwrap().input_digest;
            (part, document.part(u32::MAX).unwrap_err().input_digest)
        };
        assert_eq!(digests(), digests());
    }

    // Review 2026-10-18 EX-8: evaluating an imported part asked the worker to
    // inspect it, then to tessellate it, and each request parsed the whole
    // source again (inspection twice: unit, then part). A 100-part STEP took 300
    // full parses on every reopen. One worker now reads a source once for every
    // request about it, and a different source replaces it.
    #[test]
    fn a_worker_parses_an_imported_source_once_for_every_request_about_its_parts() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step");
        let step = SourceFile {
            sha256: sha256_hex(&std::fs::read(&corpus).unwrap()),
            path: corpus,
        };
        let output = tempfile::tempdir().unwrap();
        let iges_path = output.path().join("flat.igs");
        m21_step_xde_to_iges_response(
            &ExactBackend::new(),
            &mut SourceDocument::default(),
            &step,
            &iges_path,
        )
        .unwrap();
        let iges = SourceFile {
            sha256: sha256_hex(&std::fs::read(&iges_path).unwrap()),
            path: iges_path,
        };
        let backend = ExactBackend::new();
        let mut pairs = pair_query::PairQuerySession::default();
        let mut read = SourceDocument::default();
        let mut send = |request| handle_request(&backend, &mut pairs, &mut read, request).unwrap();
        let parses = || {
            ExactBackend::step_source_parses_on_this_thread()
                + ExactBackend::iges_source_parses_on_this_thread()
        };
        let mesh = |index: u32| output.path().join(format!("part-{index}.bin"));

        let before = parses();
        let WorkerReply::Xde(evidence) = send(WorkerRequest::InspectStepXde(step.clone())) else {
            panic!("the STEP source inspects as XDE");
        };
        for part in &evidence.parts {
            let index = Some(part.index);
            let source = step.clone();
            let WorkerReply::ImportPart(inspected) = send(WorkerRequest::InspectStepPart {
                source,
                part_index: index,
            }) else {
                panic!("part {} inspects", part.index);
            };
            assert_eq!(inspected.result_fingerprint, part.result_fingerprint);
            send(WorkerRequest::TessellateStepPart {
                source: step.clone(),
                part_index: index,
                output: mesh(part.index),
            });
            send(WorkerRequest::ExportStepXdePart {
                source: step.clone(),
                part_index: part.index,
                result_fingerprint: part.result_fingerprint.clone(),
                output: output.path().join(format!("part-{}.step", part.index)),
            });
        }
        let step_parses = parses() - before;

        let before = parses();
        for index in 0..3 {
            let (source, part_index) = (iges.clone(), Some(index));
            send(WorkerRequest::InspectIgesPart { source, part_index });
            let (source, mesh) = (iges.clone(), mesh(index));
            send(WorkerRequest::TessellateIgesPart {
                source,
                part_index,
                output: mesh,
            });
        }
        let iges_parses = parses() - before;

        let before = parses();
        let (source, part_index) = (step.clone(), Some(0));
        send(WorkerRequest::InspectStepPart { source, part_index });
        assert_eq!(
            (
                evidence.parts.len(),
                step_parses,
                iges_parses,
                parses() - before
            ),
            (2, 1, 1, 1),
            "parts, then parses of the STEP, of the IGES and of the STEP after it"
        );
    }

    // Review 2026-10-17 EX-2 (IGES half): inspecting an N-part IGES parsed it
    // N + 2 times (unit, manifest, one per part); one part took two parses.
    #[test]
    fn iges_xde_inspection_parses_the_source_once() {
        let step = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step");
        let backend = ExactBackend::new();
        let output = tempfile::tempdir().unwrap();
        let path = output.path().join("flat.igs");
        m21_step_xde_to_iges_response(
            &backend,
            &mut SourceDocument::default(),
            &SourceFile {
                sha256: sha256_hex(&std::fs::read(&step).unwrap()),
                path: step,
            },
            &path,
        )
        .unwrap();
        let source = SourceFile {
            sha256: sha256_hex(&std::fs::read(&path).unwrap()),
            path,
        };
        let parses_during = |request: &dyn Fn() -> WorkerResult| {
            let before = ExactBackend::iges_source_parses_on_this_thread();
            request().unwrap();
            ExactBackend::iges_source_parses_on_this_thread() - before
        };

        let inspection = parses_during(&|| {
            let reply = m21_iges_xde_inspection_response(
                &backend,
                &mut SourceDocument::default(),
                &source,
            )?;
            let WorkerReply::Xde(evidence) = &reply else {
                panic!("inspection answered {reply:?}");
            };
            assert_eq!(evidence.parts.len(), 3);
            Ok(reply)
        });
        let one_part = parses_during(&|| {
            m21_iges_part_inspection_response(
                &backend,
                &mut SourceDocument::default(),
                &source,
                Some(1),
            )
        });
        assert_eq!((inspection, one_part), (1, 1));
    }

    // OCCT's IGES translators share process-wide state: an IGES read beside a
    // writer switching its BRep mode ended the test process with an access
    // violation. Every IGES read and write now holds one lock.
    #[test]
    fn iges_reads_and_writes_on_two_threads_do_not_race() {
        let step = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step");
        let step = SourceFile {
            sha256: sha256_hex(&std::fs::read(&step).unwrap()),
            path: step,
        };
        let output = tempfile::tempdir().unwrap();
        let iges = output.path().join("flat.igs");
        m21_step_xde_to_iges_response(
            &ExactBackend::new(),
            &mut SourceDocument::default(),
            &step,
            &iges,
        )
        .unwrap();
        let iges = SourceFile {
            sha256: sha256_hex(&std::fs::read(&iges).unwrap()),
            path: iges,
        };
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let backend = ExactBackend::new();
                for round in 0..8 {
                    let path = output.path().join(format!("written-{round}.igs"));
                    m21_step_xde_to_iges_response(
                        &backend,
                        &mut SourceDocument::default(),
                        &step,
                        &path,
                    )
                    .unwrap();
                }
            });
            scope.spawn(|| {
                let backend = ExactBackend::new();
                for _ in 0..8 {
                    m21_iges_xde_inspection_response(
                        &backend,
                        &mut SourceDocument::default(),
                        &iges,
                    )
                    .unwrap();
                }
            });
        });
    }

    #[test]
    fn cam_collision_uses_the_document_tolerance_not_the_default() {
        let backend = ExactBackend::new();
        let unit_box = |x: f64| {
            backend
                .make_box(BoxSpec {
                    origin_mm: Point3 { x, y: 0.0, z: 0.0 },
                    size_mm: Size3 {
                        x: 1.0,
                        y: 1.0,
                        z: 1.0,
                    },
                })
                .unwrap()
        };
        let holder = unit_box(0.0);
        // A 1 µm gap: apart under the default tolerance, touching under a 10 µm one.
        let fixture = unit_box(1.001);
        let mut graph: ExactBRepGraph = serde_json::from_slice(include_bytes!(
            "../tests/fixtures/v9_top_side_contact/left.graph"
        ))
        .unwrap();
        let collisions_at = |tolerance_mm: f64| {
            let mut collisions = Vec::new();
            append_cam_collision(
                &mut collisions,
                &backend,
                0,
                0,
                1,
                1,
                Some(7),
                &holder.body,
                &fixture.body,
                tolerance_mm,
            )
            .unwrap();
            collisions
        };
        assert!(collisions_at(graph.tolerance.linear_mm()).is_empty());
        graph.tolerance = ketchup_model::tolerance::TolerancePolicy::new(0.01).unwrap();
        let touching = collisions_at(graph.tolerance.linear_mm());
        assert_eq!(touching.len(), 1);
        assert!((touching[0].distance_mm - 0.001).abs() < 1.0e-9);
    }

    #[test]
    fn typed_requests_handshake_and_refuse_forged_graph_digests() {
        let backend = ExactBackend::new();
        let mut pairs = pair_query::PairQuerySession::default();
        assert_eq!(
            handle_request(
                &backend,
                &mut pairs,
                &mut SourceDocument::default(),
                WorkerRequest::Hello,
            ),
            Ok(WorkerReply::Hello {
                protocol: protocol_identity().to_owned()
            })
        );
        // A graph whose digest no longer matches its content is refused before evaluation.
        let mut graph: ExactBRepGraph = serde_json::from_slice(include_bytes!(
            "../tests/fixtures/v9_top_side_contact/left.graph"
        ))
        .unwrap();
        graph.validate().unwrap();
        let schema = graph.schema.clone();
        graph.graph_digest = "0".repeat(64);
        assert_eq!(
            handle_request(
                &backend,
                &mut pairs,
                &mut SourceDocument::default(),
                WorkerRequest::EvaluateGraph(GraphInput {
                    graph,
                    sources: Vec::new(),
                }),
            ),
            Err(WorkerFailure {
                detail: Some(FailureDetail {
                    diagnostic:
                        ketchup_model::exact_brep_graph::ExactBRepGraphError::DigestMismatch
                            .to_string(),
                    operation: "validate_exact_brep_graph".to_owned(),
                    input_digest: "0".repeat(64),
                    backend: ketchup_exact::backend_fingerprint().to_owned(),
                }),
                ..WorkerFailure::invalid_request()
            }),
            "{schema}"
        );
    }

    #[test]
    fn spatial_path_conversion_preserves_all_coordinate_bits_handles_and_orientation() {
        let bits = |values: [f64; 3]| values.map(f64::to_bits);

        let line = ExactBRepSpatialPathSegment::Line {
            start_bits: bits([-0.0, 2.5, -7.25]),
            end_bits: bits([11.0, -13.5, 17.75]),
        };
        let SpatialProfileSegment::Line { start_mm, end_mm } =
            exact_brep_spatial_path_segment(&line)
        else {
            panic!("line conversion changed the segment kind");
        };
        assert_eq!(start_mm.map(f64::to_bits), bits([-0.0, 2.5, -7.25]));
        assert_eq!(end_mm.map(f64::to_bits), bits([11.0, -13.5, 17.75]));

        let arc = ExactBRepSpatialPathSegment::CircularArc {
            start_bits: bits([-19.25, 23.5, -29.75]),
            end_bits: bits([31.0, -37.5, 41.75]),
            center_bits: bits([43.25, -47.5, 53.75]),
            normal_bits: bits([-0.0, -1.0, 0.0]),
            clockwise: true,
        };
        let SpatialProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            normal,
            clockwise,
        } = exact_brep_spatial_path_segment(&arc)
        else {
            panic!("circular arc conversion changed the segment kind");
        };
        assert_eq!(start_mm.map(f64::to_bits), bits([-19.25, 23.5, -29.75]));
        assert_eq!(end_mm.map(f64::to_bits), bits([31.0, -37.5, 41.75]));
        assert_eq!(center_mm.map(f64::to_bits), bits([43.25, -47.5, 53.75]));
        assert_eq!(normal.map(f64::to_bits), bits([-0.0, -1.0, 0.0]));
        assert!(clockwise);

        let cubic = ExactBRepSpatialPathSegment::CubicBezier {
            start_bits: bits([-59.25, 61.5, -67.75]),
            control_1_bits: bits([71.25, -73.5, 79.75]),
            control_2_bits: bits([-83.25, 89.5, -97.75]),
            end_bits: bits([101.25, -103.5, 107.75]),
        };
        let SpatialProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } = exact_brep_spatial_path_segment(&cubic)
        else {
            panic!("cubic Bezier conversion changed the segment kind");
        };
        assert_eq!(start_mm.map(f64::to_bits), bits([-59.25, 61.5, -67.75]));
        assert_eq!(control_1_mm.map(f64::to_bits), bits([71.25, -73.5, 79.75]));
        assert_eq!(control_2_mm.map(f64::to_bits), bits([-83.25, 89.5, -97.75]));
        assert_eq!(end_mm.map(f64::to_bits), bits([101.25, -103.5, 107.75]));
    }
}
