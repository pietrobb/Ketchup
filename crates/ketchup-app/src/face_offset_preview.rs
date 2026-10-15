//! The exact preview of a Push/Pull on an exact face: the offset body is
//! evaluated in the background while the user drags or types, drawn as a
//! swept guide until it arrives, and committed with its exact result.
use super::*;
use crate::planar_push_pull::planar_face;
use ketchup_application::evaluation::{
    ProducerKey, publish_exact_products, start_exact_evaluation_scoped,
};

pub(super) struct FaceOffsetDragMesh {
    pub(super) definition_id: DefinitionId,
    pub(super) caps: Vec<[Vec3; 3]>,
    pub(super) walls: Vec<[Vec3; 4]>,
    pub(super) distance_mm: f64,
}

pub(super) struct FaceOffsetEvaluation {
    pub(super) source: ExactSource,
    pub(super) document: DocumentStore,
    pub(super) task: Option<ExactEvaluationTask>,
    pub(super) render: ExactResultRegistry,
    pub(super) topology: ExactResultRegistry,
    pub(super) ready: bool,
    /// Why the exact evaluation of the preview did not finish.
    pub(super) failure: Option<Rejection>,
}

/// Longest a confirmation waits for the exact offset of its preview: a
/// backstop only, the worker ends each request well before it.
const FACE_OFFSET_CONFIRMATION_WAIT: Duration = Duration::from_secs(60);

fn face_offset_rejection(reason: &str) -> Rejection {
    Rejection::new("push_pull.exact_evaluation", RejectionPhase::Evaluation).reason(reason)
}

/// The reason of a failed preview evaluation followed by its causes.
pub(super) fn face_offset_failure_text(failure: &Rejection) -> String {
    std::iter::once(failure.reason_text().to_owned())
        .chain(failure.causes())
        .collect::<Vec<_>>()
        .join(": ")
}

impl KetchupApp {
    pub(super) fn preview_requires_face_offset_evaluation(&self) -> bool {
        // A bare polygon profile previews as its own prism, so only a picked face
        // of an evaluated body waits for the exact offset.
        self.tool_preview
            .get::<EphemeralBoxPreview>()
            .is_some_and(|preview| preview.plan.source.topological_reference.is_some())
    }

    pub(super) fn begin_face_offset_evaluation(&mut self) {
        let Some(preview) = self
            .tool_preview
            .get::<EphemeralBoxPreview>()
            .filter(|preview| preview.plan.source.topological_reference.is_some())
        else {
            self.push_pull.face_offset_evaluation = None;
            return;
        };
        let Some(proposal) = self.push_pull.smart_proposal.as_ref() else {
            return;
        };
        let Some(snapshot) = proposal.preview(&self.document) else {
            return;
        };
        let source = ketchup_application::evaluation::exact_source(&snapshot);
        if self
            .push_pull
            .face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| evaluation.source == source || evaluation.task.is_some())
        {
            return;
        }
        // The products publish into a store over the preview itself; the
        // document is neither copied nor written and read back per distance.
        let document = match DocumentStore::from_snapshot(&snapshot) {
            Ok(document) => document,
            Err(error) => {
                self.report_face_offset_failure(
                    &face_offset_rejection("the preview cannot be evaluated").caused_by(error),
                );
                return;
            }
        };
        let Some(feature_id) = snapshot
            .definition(preview.plan.source.target.definition_id)
            .and_then(|definition| definition.feature_ids().last())
            .copied()
        else {
            return;
        };
        let scope = BTreeSet::from([ProducerKey {
            definition_id: preview.plan.source.target.definition_id,
            feature_id,
        }]);
        let worker = self.exact.worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        let task = start_exact_evaluation_scoped(
            snapshot,
            &self.file.container_data,
            &ExactResultRegistry::default(),
            &ExactResultRegistry::default(),
            worker,
            Some(&scope),
            || {},
        );
        self.push_pull.face_offset_evaluation = Some(FaceOffsetEvaluation {
            source,
            document,
            task: Some(task),
            render: ExactResultRegistry::default(),
            topology: ExactResultRegistry::default(),
            ready: false,
            failure: None,
        });
    }

    /// Publishes the finished evaluation of the preview into its own store,
    /// or records why it has no complete exact result. `wait` blocks for it
    /// (the worker bounds each request, so it ends with the products or the
    /// reason they did not come); otherwise an evaluation still running is
    /// left alone and this answers false.
    fn finish_face_offset_evaluation(&mut self, wait: bool) -> bool {
        let Some(evaluation) = self.push_pull.face_offset_evaluation.as_mut() else {
            return false;
        };
        let Some(task) = evaluation.task.take() else {
            return false;
        };
        let result = if wait {
            Ok(task.wait(FACE_OFFSET_CONFIRMATION_WAIT))
        } else {
            match task.poll() {
                Err(TryRecvError::Empty) => {
                    evaluation.task = Some(task);
                    return false;
                }
                result => result.map_err(|error| {
                    face_offset_rejection("the exact evaluation stopped without an answer")
                        .caused_by(error)
                }),
            }
        };
        let report = result
            .and_then(|products| {
                products.map_err(|error| {
                    face_offset_rejection("the exact evaluation failed").caused_by(error)
                })
            })
            .and_then(|products| {
                publish_exact_products(
                    &mut evaluation.document,
                    &mut evaluation.render,
                    &mut evaluation.topology,
                    &task,
                    products,
                )
                .map_err(|error| {
                    face_offset_rejection("the exact result could not be published")
                        .caused_by(error)
                })
            })
            .and_then(|report| {
                if report.complete && report.topology_complete {
                    Ok(())
                } else {
                    Err(face_offset_rejection(
                        report
                            .not_evaluated
                            .as_deref()
                            .unwrap_or("the exact evaluation is incomplete"),
                    ))
                }
            });
        evaluation.ready = report.is_ok();
        evaluation.failure = report.err();
        true
    }

    /// Whether the exact offset of the preview could not be evaluated; its
    /// reason is on the status line.
    pub(super) fn face_offset_failed(&self) -> bool {
        self.push_pull
            .face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| evaluation.failure.is_some())
    }

    pub(super) fn poll_face_offset_evaluation(&mut self, context: &egui::Context) {
        if let Some(due) = self.push_pull.face_offset_preview_due {
            if !self.has_preview() {
                self.push_pull.face_offset_preview_due = None;
            } else if Instant::now() >= due
                && self
                    .push_pull
                    .face_offset_evaluation
                    .as_ref()
                    .is_none_or(|evaluation| evaluation.task.is_none())
            {
                self.push_pull.face_offset_preview_due = None;
                self.begin_face_offset_evaluation();
            } else {
                context.request_repaint_after(Duration::from_millis(16));
            }
        }
        let running = self
            .push_pull
            .face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| evaluation.task.is_some());
        if !running {
            return;
        }
        if !self.finish_face_offset_evaluation(false) {
            context.request_repaint_after(Duration::from_millis(16));
        } else if !self.face_offset_evaluation_is_current() {
            context.request_repaint();
        } else if let Some(failure) = self
            .push_pull
            .face_offset_evaluation
            .as_ref()
            .and_then(|evaluation| evaluation.failure.clone())
        {
            self.report_face_offset_failure(&failure);
        }
    }

    fn report_face_offset_failure(&mut self, failure: &Rejection) {
        self.digest = self.catalog.format(
            "digest-push-pull-exact-failed",
            &BTreeMap::from([("reason", face_offset_failure_text(failure))]),
        );
    }

    fn face_offset_evaluation_is_current(&self) -> bool {
        self.push_pull
            .face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| {
                self.push_pull
                    .smart_proposal
                    .as_ref()
                    .and_then(|proposal| proposal.preview(&self.document))
                    .is_some_and(|snapshot| {
                        ketchup_application::evaluation::exact_source(&snapshot)
                            == evaluation.source
                    })
            })
    }

    pub(super) fn face_offset_preview_package(
        &self,
        definition_id: DefinitionId,
    ) -> Option<Arc<ExactBodyPackage>> {
        let evaluation = self
            .push_pull
            .face_offset_evaluation
            .as_ref()
            .filter(|evaluation| evaluation.ready)?;
        let preview = self.tool_preview.get::<EphemeralBoxPreview>()?;
        if preview.plan.source.target.definition_id != definition_id
            || !self.has_preview()
            || !self.face_offset_evaluation_is_current()
        {
            return None;
        }
        evaluation
            .render
            .get_render(&evaluation.document.current(), definition_id)
            .cloned()
    }

    /// Confirms an exact-face Push/Pull now. An offset still being evaluated
    /// is waited for here, so Enter either commits or puts its reason on the
    /// status line in this frame; a confirmation left to land later was lost
    /// when the user moved on first. A preview whose evaluation failed (a
    /// worker that timed out on a loaded machine) or went stale is evaluated
    /// again once.
    pub(super) fn confirm_face_offset_preview(&mut self) -> bool {
        if !self.has_preview() {
            return false;
        }
        for _ in 0..2 {
            // A stale finished evaluation is replaced by `begin`; a failed one
            // of the same preview has to be dropped first.
            if self.face_offset_failed() {
                self.push_pull.face_offset_evaluation = None;
            }
            self.begin_face_offset_evaluation();
            self.finish_face_offset_evaluation(true);
            if self.face_offset_evaluation_is_current() {
                break;
            }
        }
        let Some(evaluation) = self.push_pull.face_offset_evaluation.as_ref() else {
            self.status_key = "error-preview-stale";
            return false;
        };
        if !evaluation.ready || !self.face_offset_evaluation_is_current() {
            let failure = evaluation.failure.clone().unwrap_or_else(|| {
                face_offset_rejection("the preview changed while it was evaluated")
            });
            self.report_face_offset_failure(&failure);
            return false;
        }
        let Some(proposal) = self.push_pull.smart_proposal.as_ref() else {
            return false;
        };
        let Some(snapshot) = proposal.preview(&self.document) else {
            return false;
        };
        let Some(preview) = self.tool_preview.get::<EphemeralBoxPreview>() else {
            return false;
        };
        let target = preview.plan.source.target.clone();
        let distance_mm = f64::from_bits(preview.plan.distance_mm_bits);
        let face = self
            .selected_planar_face(&target)
            .map(|face| (preview.plan.source.clone(), face));
        let rule_program = match self.rewrite_program_push_pull(&preview.plan.source, distance_mm) {
            Ok(source) => source,
            Err(error) => {
                self.digest = error.reason_text().to_owned();
                return false;
            }
        };
        let Some(evaluation) = self.push_pull.face_offset_evaluation.as_ref() else {
            return false;
        };
        let Some(proposal) = self.push_pull.smart_proposal.take() else {
            return false;
        };
        let mut render = ExactResultRegistry::carried_forward(&snapshot, &self.exact.results);
        let mut topology =
            ExactResultRegistry::carried_forward(&snapshot, &self.exact.topology_results);
        for (registry, packages) in [
            (&mut render, &evaluation.render),
            (&mut topology, &evaluation.topology),
        ] {
            for package in packages.values() {
                if registry
                    .insert_current(&snapshot, Arc::clone(package))
                    .is_err()
                {
                    return false;
                }
            }
        }
        if self
            .complete_mutation_and_exact_results_with_work_recovery(
                move |document, exact_results, topology_results| {
                    proposal
                        .commit(document)
                        .map_err(|error| failed("face_offset.commit", error))?;
                    if let Some(source) = rule_program {
                        document
                            .bind_rule_program(source)
                            .map_err(|error| failed("face_offset.bind_program", error))?;
                    }
                    document
                        .register_exact_reference_evidence(&render)
                        .map_err(|error| failed("face_offset.render_evidence", error))?;
                    document
                        .register_exact_reference_evidence(&topology)
                        .map_err(|error| failed("face_offset.topology_evidence", error))?;
                    *exact_results = render;
                    *topology_results = topology;
                    Ok::<(), Rejection>(())
                },
            )
            .is_err()
        {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.hover.projection_cache.get_mut().take();
        // The pushed face stays selected, as a box face does: on the exact
        // result it is the face moved by the distance along its normal.
        let moved = face.as_ref().and_then(|(_, face)| {
            self.exact_face_on_plane(
                &target,
                face.normal,
                dot(face.origin, face.normal) + distance_mm,
            )
        });
        match moved {
            Some((selection, topological)) => {
                self.selection.clear();
                self.selection
                    .select_topological(selection.clone(), topological, false);
                self.remember_push_pull(selection, face);
            }
            None => {
                self.remember_push_pull(target, face);
                self.selection.clear();
            }
        }
        self.render.plan = None;
        self.status_key = "status-ready";
        true
    }

    pub(super) fn face_offset_drag_mesh(&self) -> Option<FaceOffsetDragMesh> {
        let preview = self.tool_preview.get::<EphemeralBoxPreview>()?;
        let source = &preview.plan.source;
        let reference = source.topological_reference.as_ref()?;
        if !self.has_preview()
            || self
                .face_offset_preview_package(source.target.definition_id)
                .is_some()
        {
            return None;
        }
        let world_face = self.selected_planar_face(&source.target)?;
        let snapshot = self.document.current();
        let package = self
            .exact
            .topology_results
            .get_render(&snapshot, source.target.definition_id)?;
        let local_face = planar_face(package, reference, Transform::identity())?;
        let distance_mm = f64::from_bits(preview.plan.distance_mm_bits);
        let offset = local_face.normal * (distance_mm / world_face.local_to_world_scale);
        let point = |index: u32| {
            let [x, y, z] = package.vertices()[index as usize].position_mm;
            Vec3::new(x, y, z)
        };
        let mut caps = Vec::new();
        let mut edges = BTreeMap::<[u32; 2], ([u32; 2], usize)>::new();
        for (index, triangle) in package.triangles().iter().enumerate() {
            if package.topological_reference_for_triangle(index) != Some(reference) {
                continue;
            }
            caps.push(triangle.vertex_indices.map(|index| point(index) + offset));
            let [a, b, c] = triangle.vertex_indices;
            for edge in [[a, b], [b, c], [c, a]] {
                let mut key = edge;
                key.sort_unstable();
                edges.entry(key).or_insert((edge, 0)).1 += 1;
            }
        }
        // Sweep the actual tessellated face, including inner wires. Interior
        // triangulation edges are not walls. This is the same prism operand as
        // OCCT Fuse/Cut, not a claim that the Boolean result is already evaluated.
        let walls = edges
            .into_values()
            .filter(|(_, uses)| *uses == 1)
            .map(|(edge, _)| {
                let [a, b] = edge.map(point);
                [a, b, b + offset, a + offset]
            })
            .collect();
        Some(FaceOffsetDragMesh {
            definition_id: source.target.definition_id,
            caps,
            walls,
            distance_mm,
        })
    }

    pub(super) fn paint_face_offset_guide(&self, painter: &egui::Painter, rect: Rect) {
        let Some(mesh) = self.face_offset_drag_mesh() else {
            return;
        };
        let color = if mesh.distance_mm > 0.0 {
            Color32::from_rgb(80, 206, 190)
        } else {
            Color32::from_rgb(240, 130, 90)
        };
        let forward = Vec3::new(
            f64::from(self.camera.yaw.sin() * self.camera.pitch.sin()),
            f64::from(self.camera.yaw.cos() * self.camera.pitch.sin()),
            -f64::from(self.camera.pitch.cos()),
        );
        let mut surfaces = Vec::new();
        let mut outlines = Vec::new();
        for occurrence in self
            .document
            .current()
            .scene_query()
            .into_iter()
            .filter(|item| item.visible && item.definition_id == mesh.definition_id)
        {
            let world = |p| transform_model_point(occurrence.transform, p);
            for cap in &mesh.caps {
                surfaces.push((cap.map(world).to_vec(), 110));
            }
            for wall in &mesh.walls {
                let points = wall.map(world);
                surfaces.push((points.to_vec(), 45));
                outlines.extend([[points[2], points[3]], [points[0], points[3]]]);
            }
        }
        let depth = |points: &[Vec3]| {
            points.iter().map(|p| point_depth(*p, forward)).sum::<f64>() / points.len() as f64
        };
        surfaces.sort_by(|(a, _), (b, _)| depth(b).total_cmp(&depth(a)));
        let mut fill = egui::Mesh::default();
        for (points, alpha) in surfaces {
            let projected = points
                .iter()
                .map(|p| self.project(*p, rect))
                .collect::<Vec<_>>();
            if projected_polygon_has_area(&projected) {
                super::viewport_feedback::add_filled_polygon(
                    &mut fill,
                    &projected,
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
                );
            }
        }
        if !fill.indices.is_empty() {
            painter.add(egui::Shape::mesh(fill));
        }
        for edge in outlines {
            painter.line_segment(
                edge.map(|p| self.project(p, rect)),
                Stroke::new(2.0_f32, color),
            );
        }
    }
}
