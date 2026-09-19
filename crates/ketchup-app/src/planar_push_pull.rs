use super::*;
use ketchup_application::evaluation::{
    ProducerKey, publish_exact_products, start_exact_evaluation_scoped,
};
use ketchup_interaction::exact_projection::ExactSurfaceHit;
#[cfg(test)]
#[path = "planar_push_pull_tests.rs"]
pub(crate) mod tests;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlanarFace {
    pub origin: Vec3,
    pub normal: Vec3,
    pub local_to_world_scale: f64,
}

pub(super) struct FaceOffsetDragMesh {
    definition_id: DefinitionId,
    caps: Vec<[Vec3; 3]>,
    walls: Vec<[Vec3; 4]>,
    distance_mm: f64,
}

pub(super) struct FaceOffsetEvaluation {
    source: ExactSource,
    document: DocumentStore,
    task: Option<ExactEvaluationTask>,
    render: ExactResultRegistry,
    topology: ExactResultRegistry,
    ready: bool,
    failed: bool,
    confirm_requested: bool,
}

pub(super) fn face_ordinal(
    package: &ExactBodyPackage,
    reference: &TopologicalElementRef,
) -> Option<u32> {
    package
        .topological_references()
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .position(|candidate| candidate == reference)
        .and_then(|ordinal| u32::try_from(ordinal).ok())
}

pub(super) fn triangle_element(
    package: &ExactBodyPackage,
    index: usize,
    normal: Vec3,
) -> ElementId {
    package
        .topological_reference_for_triangle(index)
        .and_then(|reference| face_ordinal(package, reference))
        .map(ElementId::TopologicalFace)
        .unwrap_or_else(|| {
            package.triangles()[index]
                .face_role
                .and_then(exact_face_element)
                .unwrap_or_else(|| face_element_from_normal(normal))
        })
}

fn planar_face(
    package: &ExactBodyPackage,
    reference: &TopologicalElementRef,
    transform: Transform,
) -> Option<PlanarFace> {
    let triangles = package
        .triangles()
        .iter()
        .enumerate()
        .filter(|(index, _)| package.topological_reference_for_triangle(*index) == Some(reference))
        .map(|(_, triangle)| {
            triangle.vertex_indices.map(|index| {
                let p = package.vertices()[index as usize].position_mm;
                Vec3::new(p[0], p[1], p[2])
            })
        })
        .collect::<Vec<_>>();
    let first = *triangles.first()?;
    let local_normal = triangle_normal(first);
    let length = vector_length(local_normal);
    if length <= 1.0e-12 {
        return None;
    }
    let local_normal = local_normal * (1.0 / length);
    if vector_length(local_normal) < 0.99 {
        return None;
    }
    let tolerance = package
        .bounds_mm()
        .into_iter()
        .flatten()
        .map(f64::abs)
        .fold(1.0, f64::max)
        * 1.0e-8;
    if triangles
        .iter()
        .flatten()
        .any(|p| dot(*p - first[0], local_normal).abs() > tolerance)
    {
        return None;
    }
    let world = first.map(|p| transform_model_point(transform, p));
    let normal = triangle_normal(world);
    let length = vector_length(normal);
    if length <= 1.0e-12 {
        return None;
    }
    let normal = normal * (1.0 / length);
    let transformed_normal = transform_model_point(transform, first[0] + local_normal) - world[0];
    let normal = if dot(transformed_normal, normal) < 0.0 {
        normal * -1.0
    } else {
        normal
    };
    let scale = dot(transformed_normal, normal);
    // Only similarities preserve a perpendicular offset as a perpendicular offset.
    if scale <= 1.0e-9 || vector_length(transformed_normal - normal * scale) > scale * 1.0e-8 {
        return None;
    }
    Some(PlanarFace {
        origin: world[0],
        normal,
        local_to_world_scale: scale,
    })
}

impl KetchupApp {
    pub(super) fn planar_source_box(&self, target: &SelectionId) -> Option<RenderBox> {
        self.selected_planar_face(target)?;
        let snapshot = self.document.current();
        let package = self
            .topology_results
            .get_render(&snapshot, target.definition_id)?;
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == target.instance_path)?;
        let mut minimum = [f64::INFINITY; 3];
        let mut maximum = [f64::NEG_INFINITY; 3];
        for vertex in package.vertices() {
            let p = vertex.position_mm;
            let p = transform_model_point(occurrence.transform, Vec3::new(p[0], p[1], p[2]));
            for (i, value) in [p.x, p.y, p.z].into_iter().enumerate() {
                minimum[i] = minimum[i].min(value);
                maximum[i] = maximum[i].max(value);
            }
        }
        Some(RenderBox {
            definition_id: target.definition_id,
            profile_feature_id: package.producer_feature_id(),
            extrusion_feature_id: None,
            instance_path: target.instance_path.clone(),
            origin_mm: Vec3::new(minimum[0], minimum[1], minimum[2]),
            size_mm: Vec3::new(
                maximum[0] - minimum[0],
                maximum[1] - minimum[1],
                maximum[2] - minimum[2],
            ),
        })
    }

    pub(super) fn derive_planar_preview(
        &self,
        source: &PushPullSourcePlan,
        principal: ProposalPrincipal,
        expression: &str,
        distance: f64,
    ) -> Option<(PushPullPreviewPlan, CommandBatch, SmartPushPullProposal)> {
        if distance.abs() < 0.01 {
            return None;
        }
        let face = self.selected_planar_face(&source.target)?;
        let snapshot = self.push_pull_planning_snapshot();
        let batch = push_pull_batch(
            &snapshot,
            &source.target,
            &source.target_box,
            source.topological_reference.as_ref(),
            distance / face.local_to_world_scale,
            0.0,
            expression.to_owned(),
        )?;
        let proposal = if principal == ProposalPrincipal::ManualClient {
            self.prepare_manual_push_pull_proposal(batch.clone())
        } else {
            self.prepare_smart_push_pull_proposal(batch.clone(), principal)
                .map(SmartPushPullProposal::Append)
        }?;
        let shared_count = snapshot
            .scene_query()
            .into_iter()
            .filter(|item| item.definition_id == source.target.definition_id)
            .count();
        Some((
            PushPullPreviewPlan {
                source: source.clone(),
                principal,
                distance_expression: expression.to_owned(),
                distance_mm_bits: distance.to_bits(),
                current_extent_mm_bits: 0.0_f64.to_bits(),
                new_extent_mm_bits: distance.to_bits(),
                commands: batch.commands().to_vec(),
                exact_request: None,
                preview_box: source.target_box.clone(),
                shared_count,
            },
            batch,
            proposal,
        ))
    }

    pub(super) fn planar_screen_projection(
        &self,
        target: &SelectionId,
        pointer: Pos2,
        rect: Rect,
    ) -> Option<(Vec2, f32)> {
        let ray = self.view_ray(pointer, rect)?;
        let snapshot = self.document.current();
        let topology = self.topology_results_for_snapshot(&snapshot)?;
        let projection = ExactInteractionProjection::from_snapshot(&snapshot, topology);
        let hit = projection
            .exact_surface_picks(ray)
            .into_iter()
            .find(|hit| {
                hit.definition_id == target.definition_id
                    && hit.instance_path == target.instance_path
                    && self.exact_hit_element(hit).as_ref() == Some(&target.element)
            })?;
        let projected = self.project(hit.position_mm + hit.outward_normal, rect)
            - self.project(hit.position_mm, rect);
        let scale = projected.length();
        if scale > 1.0e-4 {
            Some((projected / scale, scale))
        } else {
            Some((
                Vec2::new(0.0, -1.0),
                (self.zoom * rect.width().min(rect.height()) / 420.0).max(1.0e-4),
            ))
        }
    }

    pub(super) fn interaction_exact_registry(&self, snapshot: &Snapshot) -> ExactResultRegistry {
        let topology = self.topology_results_for_snapshot(snapshot);
        let mut registry = ExactResultRegistry::default();
        if let Some(render) = self.exact_results_for_snapshot(snapshot) {
            for package in render.render_values(snapshot) {
                if topology
                    .and_then(|registry| registry.get_render(snapshot, package.definition_id()))
                    .is_none()
                {
                    let _ = registry.insert_current(snapshot, Arc::clone(package));
                }
            }
        }
        if let Some(topology) = topology {
            for package in topology.render_values(snapshot) {
                let _ = registry.insert_current(snapshot, Arc::clone(package));
            }
        }
        registry
    }

    pub(super) fn exact_hit_element(&self, hit: &ExactSurfaceHit) -> Option<ElementId> {
        if let Some(target) = hit.topological_target.as_ref() {
            let snapshot = self.document.current();
            let package = self
                .topology_results_for_snapshot(&snapshot)?
                .get_render(&snapshot, hit.definition_id)?;
            return face_ordinal(&package, &target.target().reference)
                .map(ElementId::TopologicalFace);
        }
        hit.durable_target
            .as_ref()
            .and_then(|target| target.body.role())
            .and_then(exact_face_element)
            .or_else(|| exact_surface_element(hit.outward_normal))
    }

    pub(super) fn selected_planar_face(&self, target: &SelectionId) -> Option<PlanarFace> {
        let snapshot = self.document.current();
        let (_, bound) = self
            .selection
            .topological
            .iter()
            .find(|(selection, _)| selection == target)?;
        let resolved = bound
            .resolve_current(&snapshot, &self.topology_results)
            .ok()?;
        let package = self
            .topology_results
            .get_render(&snapshot, target.definition_id)?;
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == target.instance_path)?;
        planar_face(&package, &resolved.reference, occurrence.transform)
    }

    pub(super) fn select_push_pull_target(
        &mut self,
        target: SelectionId,
        pointer: Pos2,
        rect: Rect,
    ) {
        if let Some(topological) = self.topological_selection_at_screen(pointer, rect, &target) {
            self.selection
                .select_topological(target, topological, false);
        } else {
            self.select_from_viewport(Some(target), false);
        }
    }

    pub(super) fn select_push_pull_reference(&mut self, target: SelectionId) {
        if let ElementId::TopologicalFace(ordinal) = target.element {
            let snapshot = self.document.current();
            if let Some(package) = self
                .topology_results
                .get_render(&snapshot, target.definition_id)
            {
                self.select_topological_locator(TopologicalPickLocator {
                    instance_path: target.instance_path,
                    producer_feature_id: package.producer_feature_id(),
                    kind: TopologicalElementKind::Face,
                    ordinal,
                });
            }
        } else {
            self.selection.select_exact(target, false);
        }
    }

    pub(super) fn preview_requires_face_offset_evaluation(&self) -> bool {
        self.preview_box.as_ref().is_some_and(|preview| {
            preview.plan.source.topological_reference.is_some()
                || (preview
                    .plan
                    .source
                    .target_box
                    .extrusion_feature_id
                    .is_none()
                    && self
                        .push_pull_planning_snapshot()
                        .feature(preview.plan.source.target_box.profile_feature_id)
                        .is_some_and(|feature| {
                            matches!(
                                feature.kind(),
                                FeatureKind::SegmentProfile {
                                    closed: true,
                                    segments,
                                } if segments.iter().all(|segment| {
                                    matches!(segment, ProfileSegment::Line { .. })
                                })
                            )
                        }))
        })
    }

    pub(super) fn begin_face_offset_evaluation(&mut self) {
        if !self.preview_requires_face_offset_evaluation() {
            self.face_offset_evaluation = None;
            return;
        }
        let Some(preview) = self.preview_box.as_ref() else {
            self.face_offset_evaluation = None;
            return;
        };
        let Some(proposal) = self.smart_push_pull_proposal.as_ref() else {
            return;
        };
        let Some(snapshot) = proposal.preview(&self.document) else {
            return;
        };
        let source = ketchup_application::evaluation::exact_source(&snapshot);
        if self
            .face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| evaluation.source == source || evaluation.task.is_some())
        {
            return;
        }
        let Some(document) =
            ketchup_core::persistence::save_container(&snapshot, &self.container_data)
                .ok()
                .and_then(|bytes| ketchup_core::persistence::load(&bytes).ok())
                .and_then(|loaded| loaded.into_editable().ok())
        else {
            return;
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
        let worker = self.exact_worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        });
        let task = start_exact_evaluation_scoped(
            snapshot,
            &self.container_data,
            &ExactResultRegistry::default(),
            &ExactResultRegistry::default(),
            worker,
            Some(&scope),
            || {},
        );
        self.face_offset_evaluation = Some(FaceOffsetEvaluation {
            source,
            document,
            task: Some(task),
            render: ExactResultRegistry::default(),
            topology: ExactResultRegistry::default(),
            ready: false,
            failed: false,
            confirm_requested: false,
        });
    }

    pub(super) fn request_face_offset_confirmation(&mut self) -> bool {
        self.begin_face_offset_evaluation();
        let Some(evaluation) = self.face_offset_evaluation.as_mut() else {
            return false;
        };
        evaluation.confirm_requested = true;
        true
    }

    pub(super) fn poll_face_offset_evaluation(&mut self, context: &egui::Context) {
        if let Some(due) = self.face_offset_preview_due {
            if !self.has_preview() {
                self.face_offset_preview_due = None;
            } else if Instant::now() >= due
                && self
                    .face_offset_evaluation
                    .as_ref()
                    .is_none_or(|evaluation| evaluation.task.is_none())
            {
                self.face_offset_preview_due = None;
                self.begin_face_offset_evaluation();
            } else {
                context.request_repaint_after(Duration::from_millis(16));
            }
        }
        let Some(evaluation) = self.face_offset_evaluation.as_mut() else {
            return;
        };
        let Some(task) = evaluation.task.as_ref() else {
            return;
        };
        match task.poll() {
            Err(TryRecvError::Empty) => {
                context.request_repaint_after(Duration::from_millis(16));
                return;
            }
            result => {
                let report = result.ok().and_then(Result::ok).and_then(|products| {
                    publish_exact_products(
                        &mut evaluation.document,
                        &mut evaluation.render,
                        &mut evaluation.topology,
                        task,
                        products,
                    )
                    .ok()
                });
                evaluation.ready =
                    report.is_some_and(|report| report.complete && report.topology_complete);
                evaluation.failed = !evaluation.ready;
                evaluation.task = None;
            }
        }
        let confirm_requested = evaluation.confirm_requested;
        let confirm = confirm_requested && evaluation.ready;
        let failed = evaluation.failed;
        if !self.face_offset_evaluation_is_current() {
            if confirm_requested {
                self.face_offset_evaluation = None;
                self.request_face_offset_confirmation();
            }
            context.request_repaint();
            return;
        }
        if failed {
            self.digest = "Push/Pull: exact evaluation failed; document unchanged".to_owned();
        }
        if confirm {
            self.confirm_preview();
        }
    }

    fn face_offset_evaluation_is_current(&self) -> bool {
        self.face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| {
                self.smart_push_pull_proposal
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
            .face_offset_evaluation
            .as_ref()
            .filter(|evaluation| evaluation.ready)?;
        let preview = self.preview_box.as_ref()?;
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

    pub(super) fn face_offset_confirmation_pending(&self) -> bool {
        self.face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| evaluation.confirm_requested && !evaluation.failed)
    }

    pub(super) fn confirm_face_offset_preview(&mut self) -> bool {
        if !self.has_preview() {
            return false;
        }
        if self
            .face_offset_evaluation
            .as_ref()
            .is_some_and(|evaluation| evaluation.task.is_none())
            && !self.face_offset_evaluation_is_current()
        {
            self.face_offset_evaluation = None;
            self.request_face_offset_confirmation();
            return false;
        }
        let Some(evaluation) = self.face_offset_evaluation.as_mut() else {
            return false;
        };
        if !evaluation.ready {
            evaluation.confirm_requested = !evaluation.failed;
            return false;
        }
        let Some(proposal) = self.smart_push_pull_proposal.as_ref() else {
            return false;
        };
        let Some(snapshot) = proposal.preview(&self.document) else {
            return false;
        };
        if ketchup_application::evaluation::exact_source(&snapshot) != evaluation.source {
            self.face_offset_evaluation = None;
            self.request_face_offset_confirmation();
            return false;
        }
        let Some(proposal) = self.smart_push_pull_proposal.take() else {
            return false;
        };
        let evaluation = self.face_offset_evaluation.as_ref().unwrap();
        let mut render = ExactResultRegistry::carried_forward(&snapshot, &self.exact_results);
        let mut topology = ExactResultRegistry::carried_forward(&snapshot, &self.topology_results);
        for package in evaluation.render.values() {
            if render
                .insert_current(&snapshot, Arc::clone(package))
                .is_err()
            {
                return false;
            }
        }
        for package in evaluation.topology.values() {
            if topology
                .insert_current(&snapshot, Arc::clone(package))
                .is_err()
            {
                return false;
            }
        }
        if self
            .mutate_document_with_work_recovery(|document| {
                proposal
                    .commit(document)
                    .map_err(|error| error.to_string())?;
                document
                    .register_exact_reference_evidence(&render)
                    .map_err(|error| error.to_string())?;
                document
                    .register_exact_reference_evidence(&topology)
                    .map_err(|error| error.to_string())
            })
            .is_err()
        {
            return false;
        }
        let snapshot = self.document.current();
        self.rebind_exact_results(&snapshot);
        self.exact_results = render;
        self.topology_results = topology;
        self.clear_ephemeral_edit_state();
        self.selection.clear();
        self.interaction_projection_cache.get_mut().take();
        self.render_plan = None;
        self.status_key = "status-ready";
        true
    }

    pub(super) fn face_offset_drag_mesh(&self) -> Option<FaceOffsetDragMesh> {
        let preview = self.preview_box.as_ref()?;
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
            f64::from(self.yaw.sin() * self.pitch.sin()),
            f64::from(self.yaw.cos() * self.pitch.sin()),
            -f64::from(self.pitch.cos()),
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
        for (points, alpha) in surfaces {
            let projected = points
                .iter()
                .map(|p| self.project(*p, rect))
                .collect::<Vec<_>>();
            if projected_polygon_has_area(&projected) {
                painter.add(egui::Shape::convex_polygon(
                    projected,
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
                    Stroke::NONE,
                ));
            }
        }
        for edge in outlines {
            painter.line_segment(
                edge.map(|p| self.project(p, rect)),
                Stroke::new(2.0_f32, color),
            );
        }
    }
}
