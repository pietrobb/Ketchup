use super::*;
use ketchup_geometry::linalg::normalize_within;
use ketchup_interaction::exact_projection::ExactSurfaceHit;
use ketchup_model::tolerance::{ACCUMULATED_ROUNDING, ROUNDING, SCREEN_ROUNDING_PX};
#[cfg(test)]
#[path = "planar_push_pull_tests.rs"]
pub(crate) mod tests;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PlanarFace {
    pub origin: Vec3,
    pub normal: Vec3,
    pub local_to_world_scale: f64,
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

/// `face_ordinal` of every face reference, by its position among the faces:
/// computed once per body so classifying each triangle stays O(1).
pub(super) fn canonical_face_ordinals(package: &ExactBodyPackage) -> Vec<Option<u32>> {
    let mut first = BTreeMap::new();
    package
        .topological_references()
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .enumerate()
        .map(|(ordinal, reference)| {
            *first
                .entry(reference)
                .or_insert_with(|| u32::try_from(ordinal).ok())
        })
        .collect()
}

pub(super) fn triangle_element(
    package: &ExactBodyPackage,
    face_ordinals: &[Option<u32>],
    index: usize,
    normal: Vec3,
) -> ElementId {
    package
        .triangle_face_ordinal(index)
        .and_then(|ordinal| face_ordinals.get(ordinal as usize).copied().flatten())
        .map(ElementId::TopologicalFace)
        .unwrap_or_else(|| {
            package.triangles()[index]
                .face_role
                .and_then(exact_face_element)
                .unwrap_or_else(|| face_element_from_normal(normal))
        })
}

pub(super) fn planar_face(
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
    let local_normal = normalize_within(triangle_normal(first), ROUNDING)?;
    let tolerance = package
        .bounds_mm()
        .into_iter()
        .flatten()
        .map(f64::abs)
        .fold(1.0, f64::max)
        * ACCUMULATED_ROUNDING;
    if triangles
        .iter()
        .flatten()
        .any(|p| dot(*p - first[0], local_normal).abs() > tolerance)
    {
        return None;
    }
    let world = first.map(|p| transform_model_point(transform, p));
    let normal = normalize_within(triangle_normal(world), ROUNDING)?;
    let transformed_normal = transform_model_point(transform, first[0] + local_normal) - world[0];
    let normal = if dot(transformed_normal, normal) < 0.0 {
        normal * -1.0
    } else {
        normal
    };
    let scale = dot(transformed_normal, normal);
    // Only similarities preserve a perpendicular offset as a perpendicular offset.
    if scale <= ROUNDING
        || length(transformed_normal - normal * scale) > scale * ACCUMULATED_ROUNDING
    {
        return None;
    }
    Some(PlanarFace {
        origin: world[0],
        normal,
        local_to_world_scale: scale,
    })
}

/// The outward unit normal of one side of an axis-aligned box and the
/// position of that side's plane along it.
fn box_side_plane(item: &RenderBox, axis: Axis, side: Side) -> (Vec3, f64) {
    let unit = axis_vector(axis, 1.0);
    let low = dot(item.origin_mm, unit);
    match side {
        Side::Maximum => (unit, low + dot(item.size_mm, unit)),
        Side::Minimum => (unit * -1.0, -low),
    }
}

impl KetchupApp {
    /// How far `target`'s exact body reaches behind `face` against its normal;
    /// `None` without that body. Pushing the face that far or further in
    /// leaves no body behind it.
    pub(super) fn exact_depth_behind(
        &self,
        target: &SelectionId,
        face: &PlanarFace,
    ) -> Option<f64> {
        let snapshot = self.document.current();
        let package = self
            .exact
            .topology_results
            .get_render(&snapshot, target.definition_id)?;
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == target.instance_path)?;
        package
            .vertices()
            .iter()
            .map(|vertex| {
                let p = vertex.position_mm;
                let p = transform_model_point(occurrence.transform, Vec3::new(p[0], p[1], p[2]));
                -dot(p - face.origin, face.normal)
            })
            .reduce(f64::max)
    }

    pub(super) fn planar_source_box(&self, target: &SelectionId) -> Option<RenderBox> {
        self.selected_planar_face(target)?;
        let snapshot = self.document.current();
        let package = self
            .exact
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

    pub(super) fn rewrite_program_push_pull(
        &self,
        source: &PushPullSourcePlan,
        distance_mm: f64,
    ) -> Result<Option<ketchup_model::document::RuleProgramSource>, Rejection> {
        let Some(program) = self.document.current_rule_program() else {
            return Ok(None);
        };
        let refusal =
            |code, reason| Rejection::new(code, RejectionPhase::Validation).reason(reason);
        let reference = source.topological_reference.as_ref().ok_or_else(|| {
            refusal(
                "push_pull.program_face_unnamed",
                "Push/Pull on a program-owned part requires a named face",
            )
        })?;
        let snapshot = self.document.current();
        let occurrence = snapshot
            .occurrence(source.target.instance_path.root_occurrence())
            .ok_or_else(|| {
                refusal(
                    "push_pull.program_part_missing",
                    "Push/Pull program part is missing",
                )
            })?;
        let face = self.selected_planar_face(&source.target).ok_or_else(|| {
            refusal(
                "push_pull.program_face_not_planar",
                "Push/Pull program face is no longer planar",
            )
        })?;
        // The evaluator exposes a program extrusion's caps under the standard
        // extrusion roles; in the program they are `end` and `start`.
        let face_name = match reference.producer_element_id.as_str() {
            role if role == ExactFaceRole::Top.semantic_role() => "end",
            role if role == ExactFaceRole::Bottom.semantic_role() => "start",
            name => name,
        };
        let change = ketchup_application::PartChange::PushPull {
            face: face_name.to_owned(),
            distance: distance_mm / face.local_to_world_scale,
        };
        ketchup_application::rewrite_rule_program_parts(
            program,
            &[(occurrence.name().to_owned(), change)],
        )
        .map_err(|error| failed("push_pull.program_rewrite", error))
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
        // A click-anchored push/pull may start with the pointer off the selected
        // face; the face's own plane then gives the drag direction.
        let (position, normal) = projection
            .exact_surface_picks(ray)
            .into_iter()
            .find(|hit| {
                hit.definition_id == target.definition_id
                    && hit.instance_path == target.instance_path
                    && self.exact_hit_element(hit).as_ref() == Some(&target.element)
            })
            .map(|hit| (hit.position_mm, hit.outward_normal))
            .or_else(|| {
                self.selected_planar_face(target)
                    .map(|face| (face.origin, face.normal))
            })?;
        let projected = self.project(position + normal, rect) - self.project(position, rect);
        let scale = projected.length();
        if scale > SCREEN_ROUNDING_PX {
            Some((projected / scale, scale))
        } else {
            Some((
                Vec2::new(0.0, -1.0),
                (self.camera.zoom * rect.width().min(rect.height()) / 420.0)
                    .max(SCREEN_ROUNDING_PX),
            ))
        }
    }

    pub(super) fn interaction_exact_registry(&self, snapshot: &Snapshot) -> ExactResultRegistry {
        let topology = self.topology_results_for_snapshot(snapshot);
        // One pass over the topology results instead of one per render package.
        let mut topology_counts = BTreeMap::<DefinitionId, usize>::new();
        for package in topology
            .into_iter()
            .flat_map(|registry| registry.render_values(snapshot))
        {
            *topology_counts.entry(package.definition_id()).or_default() += 1;
        }
        let mut registry = ExactResultRegistry::default();
        if let Some(render) = self.exact_results_for_snapshot(snapshot) {
            for package in render.render_values(snapshot) {
                // `get_render` is unique-or-nothing: render results stand in
                // unless topology has exactly one package for this definition.
                if topology_counts.get(&package.definition_id()) != Some(&1) {
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

    /// `interaction_exact_registry(snapshot).get_render(definition_id)`
    /// without building the registry: called per hovered or selected body
    /// every frame, where rebuilding it checked every body's freshness.
    pub(super) fn interaction_render_package(
        &self,
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Option<Arc<ExactBodyPackage>> {
        if let Some(topology) = self.topology_results_for_snapshot(snapshot) {
            let mut matches = topology
                .render_values(snapshot)
                .filter(|package| package.definition_id() == definition_id);
            if let Some(package) = matches.next() {
                return matches.next().is_none().then(|| Arc::clone(package));
            }
        }
        self.exact_results_for_snapshot(snapshot)?
            .get_render(snapshot, definition_id)
            .cloned()
    }

    pub(super) fn exact_hit_element(&self, hit: &ExactSurfaceHit) -> Option<ElementId> {
        if let Some(target) = hit.topological_target.as_ref() {
            let snapshot = self.document.current();
            let topology = self.topology_results_for_snapshot(&snapshot)?;
            let key = (
                snapshot.document_id(),
                snapshot.revision_id(),
                topology.contents_stamp(),
            );
            let mut memo = self.exact.topology_by_definition.borrow_mut();
            if memo.as_ref().is_none_or(|memo| memo.key != key) {
                *memo = Some(app_state::TopologyByDefinition {
                    key,
                    packages: topology
                        .render_by_definition(&snapshot)
                        .into_iter()
                        .map(|(definition_id, package)| (definition_id, Arc::clone(package)))
                        .collect(),
                });
            }
            let package = memo.as_ref()?.packages.get(&hit.definition_id)?;
            return face_ordinal(package, &target.target().reference)
                .map(ElementId::TopologicalFace);
        }
        hit.durable_target
            .as_ref()
            .and_then(|target| target.body.role())
            .and_then(exact_face_element)
            .or_else(|| exact_surface_element(hit.outward_normal))
    }

    /// While a typed value corrects the last exact-face Push/Pull, that
    /// Push/Pull's plan and face. The face is bound to its parent revision,
    /// which the current exact results no longer describe.
    pub(super) fn corrected_push_pull_face(
        &self,
        target: &SelectionId,
    ) -> Option<&(PushPullSourcePlan, PlanarFace)> {
        if !matches!(
            self.push_pull.smart_planning,
            Some(SmartPushPullPlanning::TipReplacement(_))
        ) {
            return None;
        }
        let last = self.current_last_push_pull()?;
        if last.selection != *target || last.topological != self.selection.topological {
            return None;
        }
        last.face.as_ref()
    }

    pub(super) fn selected_planar_face(&self, target: &SelectionId) -> Option<PlanarFace> {
        if let Some((_, face)) = self.corrected_push_pull_face(target) {
            return Some(face.clone());
        }
        let snapshot = self.document.current();
        let (_, bound) = self
            .selection
            .topological
            .iter()
            .find(|(selection, _)| selection == target)?;
        let resolved = bound
            .resolve_current(&snapshot, &self.exact.topology_results)
            .ok()?;
        let package = self
            .exact
            .topology_results
            .get_render(&snapshot, target.definition_id)?;
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == target.instance_path)?;
        planar_face(package, &resolved.reference, occurrence.transform)
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
                .exact
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

    /// A box face clicked in the viewport before the body's exact result moves,
    /// once that result is published, to the one exact face on the same side's
    /// plane, so it keeps its fill and Push/Pull offsets the exact face; an
    /// ambiguous side stays as it was. It waits while a dialog, preview, drag,
    /// typed correction or other tool depends on the selection as it is.
    /// Called every frame: a lookup until the body is there, then one search.
    pub(crate) fn carry_box_face_selection_to_exact(&mut self) {
        let Some(primary) = self.exact.clicked_box_face.clone() else {
            return;
        };
        if self.selection.primary.as_ref() != Some(&primary) {
            self.exact.clicked_box_face = None;
            return;
        }
        let ElementId::Face { axis, side } = primary.element else {
            return;
        };
        if !self.selection.topological.is_empty()
            || self.modal.is_some()
            || self.tool_preview.is_some()
            || self.gesture.drag.is_some()
            || !matches!(self.active_tool, ActiveTool::Select | ActiveTool::PushPull)
            || self
                .current_last_push_pull()
                .is_some_and(|last| last.selection == primary)
        {
            return;
        }
        let snapshot = self.document.current();
        if self
            .exact
            .topology_results
            .get_render(&snapshot, primary.definition_id)
            .is_none()
        {
            return;
        }
        // The exact body is there: the side is looked up once, found or not.
        self.exact.clicked_box_face = None;
        let Some(item) = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == primary.instance_path)
        else {
            return;
        };
        // A drawn profile without an extrusion is pushed as that profile; its
        // flat exact face is not the target.
        if item.extrusion_feature_id.is_none() {
            return;
        }
        let (outward, plane) = box_side_plane(&item, axis, side);
        let Some((selection, topological)) = self.exact_face_on_plane(&primary, outward, plane)
        else {
            return;
        };
        if self.selection.exact_references.get(&primary.instance_path) == Some(&primary) {
            self.selection
                .exact_references
                .insert(primary.instance_path, selection.clone());
        }
        self.selection
            .topological
            .push((selection.clone(), topological));
        self.selection.primary = Some(selection);
    }

    /// The one planar face of `target`'s exact body whose outward normal is
    /// `outward` and whose plane lies at `plane` along it, bound for selection;
    /// `None` when no face or several faces lie there.
    pub(super) fn exact_face_on_plane(
        &self,
        target: &SelectionId,
        outward: Vec3,
        plane: f64,
    ) -> Option<(SelectionId, SnapshotBoundTopologicalSelection)> {
        let snapshot = self.document.current();
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| occurrence.instance_path == target.instance_path)?;
        let package = self
            .exact
            .topology_results
            .get_render(&snapshot, target.definition_id)?;
        let tolerance = package
            .bounds_mm()
            .into_iter()
            .flatten()
            .map(f64::abs)
            .fold(plane.abs().max(1.0), f64::max)
            * ACCUMULATED_ROUNDING;
        let ordinals = package
            .topological_references()
            .iter()
            .filter(|reference| reference.kind == TopologicalElementKind::Face)
            .filter(|reference| {
                planar_face(package, reference, occurrence.transform).is_some_and(|face| {
                    dot(face.normal, outward) >= 1.0 - ROUNDING
                        && (dot(face.origin, outward) - plane).abs() <= tolerance
                })
            })
            .filter_map(|reference| face_ordinal(package, reference))
            .collect::<BTreeSet<_>>();
        let [ordinal] = ordinals.into_iter().collect::<Vec<_>>()[..] else {
            return None;
        };
        let topological = self.bind_topological_selection(&TopologicalPickLocator {
            instance_path: target.instance_path.clone(),
            producer_feature_id: package.producer_feature_id(),
            kind: TopologicalElementKind::Face,
            ordinal,
        })?;
        let selection = SelectionId {
            element: ElementId::TopologicalFace(ordinal),
            ..target.clone()
        };
        Some((selection, topological))
    }
}
