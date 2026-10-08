//! Moving, rotating, scaling, copying, pasting, Push/Pull, patterns and alignment of the selection.

use crate::*;
use ketchup_geometry::linalg::CubicBezier;

impl KetchupApp {
    pub(crate) fn selected_alignment_pair(&self) -> Option<(OccurrenceId, OccurrenceId)> {
        let selected = self.selected_occurrence_ids();
        if selected.len() != 2 {
            return None;
        }
        let primary = self.selection.primary.as_ref()?;
        if !primary.instance_path.is_root() {
            return None;
        }
        let moving_id = primary.instance_path.root_occurrence();
        selected.contains(&moving_id).then_some((
            moving_id,
            *selected
                .iter()
                .find(|candidate| **candidate != moving_id)
                .expect("two selected occurrences include one reference"),
        ))
    }

    pub(crate) fn end_transform_correction(&mut self) {
        self.transform_tool.correction = None;
    }

    pub(crate) fn record_transform_correction(
        &mut self,
        selection: CorrectionSelection,
        operation: CorrectionOperation,
    ) {
        self.transform_tool.correction = Some(CorrectionSession {
            revision: self.document_revision(),
            canonical_digest: self.canonical_digest(),
            selection,
            operation,
        });
    }

    pub(crate) fn current_transform_correction(&mut self) -> Option<&CorrectionSession> {
        let stale = self
            .transform_tool
            .correction
            .as_ref()
            .is_some_and(|session| !self.correction_session_is_current(session));
        if stale {
            self.end_transform_correction();
        }
        self.transform_tool.correction.as_ref()
    }

    pub(crate) fn current_move_correction(
        &mut self,
    ) -> Option<(CorrectionSelection, MoveCorrection)> {
        let session = self.current_transform_correction()?;
        let CorrectionOperation::Move(operation) = &session.operation else {
            return None;
        };
        Some((session.selection.clone(), operation.clone()))
    }

    pub(crate) fn current_move_copy_correction(
        &mut self,
    ) -> Option<(CorrectionSelection, MoveCopyCorrection)> {
        let session = self.current_transform_correction()?;
        let CorrectionOperation::MoveCopy(operation) = &session.operation else {
            return None;
        };
        Some((session.selection.clone(), operation.clone()))
    }

    pub(crate) fn current_rotate_correction(
        &mut self,
    ) -> Option<(CorrectionSelection, RotateCorrection)> {
        let session = self.current_transform_correction()?;
        let CorrectionOperation::Rotate(operation) = &session.operation else {
            return None;
        };
        Some((session.selection.clone(), operation.clone()))
    }

    pub(crate) fn current_scale_correction(
        &mut self,
    ) -> Option<(CorrectionSelection, ScaleCorrection)> {
        let session = self.current_transform_correction()?;
        let CorrectionOperation::Scale(operation) = &session.operation else {
            return None;
        };
        Some((session.selection.clone(), operation.clone()))
    }

    pub(crate) fn correction_transform_plan(
        &self,
        parent: &TipReplacementParent,
        selection: &CorrectionSelection,
        world_edit: Transform,
    ) -> Option<TransformPlan> {
        let base = parent.snapshot();
        let (target, primary_occurrence_id) = match selection {
            CorrectionSelection::Occurrences {
                occurrence_ids,
                primary_occurrence_id,
            } => (
                TransformTarget::Occurrences(
                    occurrence_ids
                        .iter()
                        .copied()
                        .map(InstancePath::root)
                        .collect(),
                ),
                (*primary_occurrence_id).or_else(|| occurrence_ids.iter().next().copied())?,
            ),
            CorrectionSelection::Group(group_id) => (
                TransformTarget::Group(*group_id),
                self.selection.occurrences.iter().next()?.root_occurrence(),
            ),
        };
        let primary = base.occurrence(primary_occurrence_id)?;
        TransformPlan::prepare(
            base,
            TransformRequest {
                source_document_id: base.document_id(),
                source_revision: base.revision_id(),
                primary_occurrence: primary_occurrence_id,
                primary_definition: primary.definition_id(),
                target,
                world_edit,
            },
        )
    }

    pub(crate) fn set_move_vector_correction_enabled(&mut self, enabled: bool) {
        if let Some(CorrectionSession {
            operation: CorrectionOperation::Move(operation),
            ..
        }) = self.transform_tool.correction.as_mut()
        {
            operation.accepts_vector_correction = enabled;
        }
    }

    pub(crate) fn move_session(&self) -> Option<(&MoveDrag, ToolSessionPhase)> {
        match self.transform_tool.session.as_ref()? {
            ToolSession::Move { phase, drag } => Some((drag, *phase)),
            ToolSession::Rotate { .. } | ToolSession::Scale(_) => None,
        }
    }

    pub(crate) fn move_session_mut(&mut self) -> Option<&mut MoveDrag> {
        match self.transform_tool.session.as_mut()? {
            ToolSession::Move { drag, .. } => Some(drag),
            ToolSession::Rotate { .. } | ToolSession::Scale(_) => None,
        }
    }

    pub(crate) fn set_move_session(&mut self, phase: ToolSessionPhase, drag: MoveDrag) {
        self.transform_tool.session = Some(ToolSession::Move { phase, drag });
    }

    pub(crate) fn take_move_session(
        &mut self,
        phase: Option<ToolSessionPhase>,
    ) -> Option<MoveDrag> {
        match self.transform_tool.session.take() {
            Some(ToolSession::Move {
                phase: actual,
                drag,
            }) if phase.is_none_or(|expected| expected == actual) => Some(drag),
            session => {
                self.transform_tool.session = session;
                None
            }
        }
    }

    pub(crate) fn rotate_session(&self) -> Option<(&RotateDrag, ToolSessionPhase)> {
        match self.transform_tool.session.as_ref()? {
            ToolSession::Rotate { phase, drag } => Some((drag, *phase)),
            ToolSession::Move { .. } | ToolSession::Scale(_) => None,
        }
    }

    pub(crate) fn rotate_session_mut(&mut self) -> Option<&mut RotateDrag> {
        match self.transform_tool.session.as_mut()? {
            ToolSession::Rotate { drag, .. } => Some(drag),
            ToolSession::Move { .. } | ToolSession::Scale(_) => None,
        }
    }

    pub(crate) fn set_rotate_session(&mut self, phase: ToolSessionPhase, drag: RotateDrag) {
        self.transform_tool.session = Some(ToolSession::Rotate { phase, drag });
    }

    pub(crate) fn take_rotate_session(
        &mut self,
        phase: Option<ToolSessionPhase>,
    ) -> Option<RotateDrag> {
        match self.transform_tool.session.take() {
            Some(ToolSession::Rotate {
                phase: actual,
                drag,
            }) if phase.is_none_or(|expected| expected == actual) => Some(drag),
            session => {
                self.transform_tool.session = session;
                None
            }
        }
    }

    pub(crate) fn scale_session(&self) -> Option<&ScaleDrag> {
        match self.transform_tool.session.as_ref()? {
            ToolSession::Scale(drag) => Some(drag),
            ToolSession::Move { .. } | ToolSession::Rotate { .. } => None,
        }
    }

    pub(crate) fn scale_session_mut(&mut self) -> Option<&mut ScaleDrag> {
        match self.transform_tool.session.as_mut()? {
            ToolSession::Scale(drag) => Some(drag),
            ToolSession::Move { .. } | ToolSession::Rotate { .. } => None,
        }
    }

    pub(crate) fn set_scale_session(&mut self, drag: ScaleDrag) {
        self.transform_tool.session = Some(ToolSession::Scale(drag));
    }

    pub(crate) fn take_scale_session(&mut self) -> Option<ScaleDrag> {
        match self.transform_tool.session.take() {
            Some(ToolSession::Scale(drag)) => Some(drag),
            session => {
                self.transform_tool.session = session;
                None
            }
        }
    }

    pub(crate) fn cancel_transform_session(&mut self) {
        self.transform_tool.session = None;
        self.transform_tool.input.reset();
        self.gesture.transform.move_copy = false;
        self.gesture.transform.rotate_copy = false;
    }

    pub(crate) fn reset_transform_interaction(&mut self) {
        self.cancel_transform_session();
        self.gesture.transform.move_axis_lock = None;
        self.gesture.transform.rotate_axis_lock = None;
        self.gesture.transform.scale_axis_lock = None;
    }

    pub(crate) fn topological_push_pull_box(
        &self,
        snapshot: &Snapshot,
        target: &SelectionId,
        reference: &TopologicalElementRef,
    ) -> Option<RenderBox> {
        let package = self
            .exact
            .topology_results
            .get_render(snapshot, reference.definition_id)?;
        if reference.kind != TopologicalElementKind::Face
            || reference.definition_id != target.definition_id
            || reference.producer_feature_id != package.producer_feature_id()
        {
            return None;
        }
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| occurrence.instance_path == target.instance_path)?;
        let matrix = occurrence.transform.matrix();
        if matrix[0] != 1.0
            || matrix[1] != 0.0
            || matrix[2] != 0.0
            || matrix[4] != 0.0
            || matrix[5] != 1.0
            || matrix[6] != 0.0
            || matrix[8] != 0.0
            || matrix[9] != 0.0
            || matrix[10] != 1.0
            || matrix[12] != 0.0
            || matrix[13] != 0.0
            || matrix[14] != 0.0
            || matrix[15] != 1.0
        {
            return None;
        }
        let [minimum, maximum] = package.bounds_mm();
        Some(RenderBox {
            definition_id: target.definition_id,
            profile_feature_id: reference.producer_feature_id,
            extrusion_feature_id: None,
            instance_path: target.instance_path.clone(),
            origin_mm: Vec3::new(
                minimum[0] + matrix[3],
                minimum[1] + matrix[7],
                minimum[2] + matrix[11],
            ),
            size_mm: Vec3::new(
                maximum[0] - minimum[0],
                maximum[1] - minimum[1],
                maximum[2] - minimum[2],
            ),
        })
    }

    pub(crate) fn topological_push_pull_face_element(
        &self,
        snapshot: &Snapshot,
        reference: &TopologicalElementRef,
    ) -> Option<ElementId> {
        if reference.kind != TopologicalElementKind::Face {
            return None;
        }
        let package = self
            .exact
            .topology_results
            .get_render(snapshot, reference.definition_id)?;
        if package.producer_feature_id() != reference.producer_feature_id {
            return None;
        }
        planar_push_pull::face_ordinal(package.as_ref(), reference).map(ElementId::TopologicalFace)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn move_copy_mode_active(&self) -> bool {
        self.gesture.transform.move_copy
    }

    #[doc(hidden)]
    #[must_use]
    pub fn rotate_copy_mode_active(&self) -> bool {
        self.gesture.transform.rotate_copy
    }

    #[doc(hidden)]
    #[must_use]
    pub fn transform_gesture_active(&self) -> bool {
        self.transform_tool.session.is_some()
    }

    pub(crate) fn selected_move_reference(&self) -> Option<SelectionId> {
        if let Some(primary) = &self.selection.primary {
            return Some(primary.clone());
        }
        let instance_path = self.selection.occurrences.iter().next()?.clone();
        let snapshot = self.document.current();
        let occurrence = snapshot.occurrence(instance_path.root_occurrence())?;
        Some(SelectionId {
            definition_id: occurrence.definition_id(),
            instance_path,
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        })
    }

    pub(crate) fn commit_move_drag(&mut self, drag: &MoveDrag) -> bool {
        self.gesture.transform.move_copy = false;
        self.transform_tool.input.reset();
        if !self.move_preview_is_current(drag) {
            self.digest = self.catalog.text("error-preview-stale");
            return false;
        }
        if let Some(target) = drag.profile_target.as_ref().filter(|_| !drag.copy) {
            let delta_mm = [
                dot(drag.delta_mm, target.world_x_axis),
                dot(drag.delta_mm, target.world_y_axis),
            ];
            let distance_mm = delta_mm[0].hypot(delta_mm[1]);
            if distance_mm < 0.01 {
                return false;
            }
            let preview = match ketchup_model::feature_history::prepare_body_profile_translation(
                &self.document,
                ketchup_model::feature_history::BodyProfileTranslationRequest {
                    definition_id: target.definition_id,
                    body_id: target.body_id,
                    profile_id: target.profile_id,
                    delta_mm,
                },
                ProposalPrincipal::ManualClient,
            ) {
                Ok(preview) => preview,
                Err(error) => {
                    self.digest = error.to_string();
                    return false;
                }
            };
            if self
                .commit_proposal_with_work_recovery(&preview.proposal)
                .is_err()
            {
                return false;
            }
            self.selection.select_exact(drag.selection.clone(), false);
            self.end_transform_correction();
            self.status_key = "status-object-moved";
            self.digest = self.catalog.format(
                "digest-move-committed",
                &BTreeMap::from([("distance", format_height(distance_mm))]),
            );
            true
        } else if drag.copy {
            self.copy_occurrences(&drag.selection, &drag.occurrence_paths, drag.delta_mm)
        } else {
            let Some(plan) = self.move_transform_plan(drag) else {
                return false;
            };
            self.commit_move_transform(plan, &drag.selection, drag.delta_mm)
        }
    }

    pub(crate) fn commit_move_transform(
        &mut self,
        plan: TransformPlan,
        primary_selection: &SelectionId,
        delta_mm: Vec3,
    ) -> bool {
        let distance_mm = length(delta_mm);
        if !delta_mm.x.is_finite()
            || !delta_mm.y.is_finite()
            || !delta_mm.z.is_finite()
            || distance_mm <= 0.0
        {
            return false;
        }
        let Ok(target) = self.commit_transform_plan(plan) else {
            return false;
        };
        let selection = match target {
            TransformTarget::Occurrences(paths) => {
                self.selection
                    .select_exact(primary_selection.clone(), false);
                self.selection.occurrences.extend(paths.iter().cloned());
                CorrectionSelection::Occurrences {
                    occurrence_ids: paths.iter().map(InstancePath::root_occurrence).collect(),
                    primary_occurrence_id: None,
                }
            }
            TransformTarget::Group(group_id) => {
                self.select_group(group_id);
                CorrectionSelection::Group(group_id)
            }
        };
        self.record_transform_correction(
            selection,
            CorrectionOperation::Move(MoveCorrection {
                direction: delta_mm * (1.0 / distance_mm),
                applied_distance_mm: distance_mm,
                accepts_vector_correction: false,
            }),
        );
        self.status_key = "status-object-moved";
        self.digest = self.catalog.format(
            "digest-move-committed",
            &BTreeMap::from([("distance", format_height(distance_mm))]),
        );
        true
    }

    pub(crate) fn commit_rotate_drag(&mut self, drag: &RotateDrag) -> bool {
        self.gesture.transform.rotate_copy = false;
        self.transform_tool.input.reset();
        if !self.rotate_preview_is_current(drag) {
            self.digest = self.catalog.text("error-preview-stale");
            return false;
        }
        if drag.copy {
            return self.rotate_copy_occurrences(
                &drag.selection,
                &drag.occurrence_paths,
                drag.centre_mm,
                drag.axis,
                drag.angle_degrees,
            );
        }
        if !rotation_is_meaningful(drag.angle_degrees) {
            return false;
        }
        let Some(plan) = self.rotate_transform_plan(drag) else {
            return false;
        };
        self.commit_rotate_transform(
            plan,
            &drag.selection,
            drag.centre_mm,
            drag.axis,
            drag.angle_degrees,
        )
    }

    pub(crate) fn commit_rotate_transform(
        &mut self,
        plan: TransformPlan,
        primary_selection: &SelectionId,
        centre_mm: Vec3,
        axis: Axis,
        angle_degrees: f64,
    ) -> bool {
        let Ok(target) = self.commit_transform_plan(plan) else {
            return false;
        };
        let selection = match target {
            TransformTarget::Occurrences(paths) => {
                self.selection
                    .select_exact(primary_selection.clone(), false);
                self.selection.occurrences = paths.clone();
                CorrectionSelection::Occurrences {
                    occurrence_ids: paths.iter().map(InstancePath::root_occurrence).collect(),
                    primary_occurrence_id: None,
                }
            }
            TransformTarget::Group(group_id) => {
                self.select_group(group_id);
                CorrectionSelection::Group(group_id)
            }
        };
        self.record_transform_correction(
            selection,
            CorrectionOperation::Rotate(RotateCorrection {
                copy_source_occurrence_ids: None,
                centre_mm,
                axis,
            }),
        );
        self.status_key = "status-object-rotated";
        self.digest = self.catalog.format(
            "digest-rotate-committed",
            &BTreeMap::from([
                ("angle", format_angle(angle_degrees)),
                ("axis", self.catalog.text(axis_name_key(axis))),
            ]),
        );
        true
    }

    /// Turn the current selection by `angle_degrees` about the locked axis.
    ///
    /// This is the same commit the viewport gesture performs, exposed so a
    /// typed angle and the headless shell reach it without synthesising a drag.
    pub fn rotate_selected(&mut self, angle_degrees: f64) -> bool {
        let snapshot = self.document.current();
        let group_id = self.selection.selected_group;
        let occurrence_paths = self.selected_instance_paths();
        let applies: Box<dyn Fn(&InstancePath) -> bool> = match group_id {
            Some(group_id) => Box::new(move |path: &InstancePath| {
                Self::group_contains_occurrence(&snapshot, group_id, path.root_occurrence())
            }),
            None => {
                let occurrence_paths = occurrence_paths.clone();
                Box::new(move |path: &InstancePath| occurrence_paths.contains(path))
            }
        };
        let Some(centre_mm) = self.rotation_centre_for(&applies) else {
            return false;
        };
        self.rotate_selected_around(
            centre_mm,
            self.gesture.transform.rotate_axis_lock.unwrap_or(Axis::Z),
            angle_degrees,
        )
    }

    pub(crate) fn rotate_selected_around(
        &mut self,
        centre_mm: Vec3,
        axis: Axis,
        angle_degrees: f64,
    ) -> bool {
        if !rotation_is_meaningful(angle_degrees) {
            return false;
        }
        let snapshot = self.document.current();
        let Some(selection) = self.selected_move_reference() else {
            return false;
        };
        let occurrence_paths = self.selected_instance_paths();
        let Ok(world_edit) = world_rotation_transform(centre_mm, axis, angle_degrees) else {
            return false;
        };
        let Some(request) = self.transform_request(
            snapshot.document_id(),
            snapshot.revision_id(),
            &selection,
            &occurrence_paths,
            self.selection.selected_group,
            world_edit,
        ) else {
            return false;
        };
        let Some(plan) = TransformPlan::prepare(&snapshot, request) else {
            return false;
        };
        self.commit_rotate_transform(plan, &selection, centre_mm, axis, angle_degrees)
    }

    /// Re-turn the last rotation to `angle_degrees` instead of adding to it.
    ///
    /// The correction is planned against the guarded parent of the tip, so the
    /// body lands on exactly the angle typed — 45 followed by 40 ends at 40,
    /// not 85 — and the whole thing stays a single undo step. Zero is a
    /// legitimate correction back to where the body started.
    pub(crate) fn correct_last_rotation(&mut self, angle_degrees: f64) -> bool {
        let Some((selection, previous)) = self.current_rotate_correction() else {
            return false;
        };
        // Picking a different axis means the user wants another turn, not a
        // different value for the one already made.
        if previous.axis != self.gesture.transform.rotate_axis_lock.unwrap_or(Axis::Z) {
            return false;
        }
        let Ok(parent) = self.document.tip_replacement_parent() else {
            return false;
        };
        let Ok(rotation) =
            world_rotation_transform(previous.centre_mm, previous.axis, angle_degrees)
        else {
            return false;
        };
        let base = parent.snapshot();
        let batch = if let Some(source_ids) = &previous.copy_source_occurrence_ids {
            let CorrectionSelection::Occurrences {
                occurrence_ids: target_ids,
                ..
            } = &selection
            else {
                return false;
            };
            if source_ids.is_empty() || source_ids.len() != target_ids.len() {
                return false;
            }
            let Some(expected_first_id) = base
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .map(OccurrenceId)
            else {
                return false;
            };
            let mut commands = Vec::new();
            let mut created_per_definition = BTreeMap::<DefinitionId, usize>::new();
            for (index, (source_id, target_id)) in source_ids
                .iter()
                .copied()
                .zip(target_ids.iter().copied())
                .enumerate()
            {
                if expected_first_id
                    .0
                    .checked_add(index as u64)
                    .map(OccurrenceId)
                    != Some(target_id)
                {
                    return false;
                }
                let Some(source) = base.occurrence(source_id) else {
                    return false;
                };
                let Some(definition) = base.definition(source.definition_id()) else {
                    return false;
                };
                let Some(transform) =
                    world_edit_in_parent_space(base, source.parent(), source.transform(), rotation)
                else {
                    return false;
                };
                let existing = base
                    .scene_query()
                    .into_iter()
                    .filter(|item| item.definition_id == source.definition_id())
                    .count();
                let created = created_per_definition
                    .entry(source.definition_id())
                    .or_default();
                *created += 1;
                commands.push(CanonicalCommand::CreateOccurrence {
                    id: target_id,
                    definition_id: source.definition_id(),
                    name: self.catalog.format(
                        "model-copy-occurrence",
                        &BTreeMap::from([
                            ("name", definition.name().to_owned()),
                            ("number", (existing + *created).to_string()),
                        ]),
                    ),
                    transform,
                    parent: source.parent(),
                    tags: source.tags().clone(),
                    visible: source.visible(),
                });
                if let Some(color) = source.color() {
                    commands.push(CanonicalCommand::SetOccurrenceColor {
                        id: target_id,
                        color: Some(color),
                    });
                }
            }
            CommandBatch::new(commands)
        } else {
            let Some(plan) = self.correction_transform_plan(&parent, &selection, rotation) else {
                return false;
            };
            let (_, batch) = plan.into_commit();
            batch
        };
        let Ok(proposal) = self.document.prepare_tip_replacement_proposal(
            &parent,
            batch,
            ProposalContext::canonical_preview(),
        ) else {
            return false;
        };
        if self
            .complete_mutation_with_work_recovery(|document| {
                document.commit_tip_replacement_proposal(&proposal)
            })
            .is_err()
        {
            return false;
        }
        self.record_transform_correction(selection, CorrectionOperation::Rotate(previous.clone()));
        self.status_key = "status-object-rotated";
        self.digest = self.catalog.format(
            "digest-rotate-committed",
            &BTreeMap::from([
                ("angle", format_angle(angle_degrees)),
                ("axis", self.catalog.text(axis_name_key(previous.axis))),
            ]),
        );
        true
    }

    pub(crate) fn correct_last_scale(&mut self, factor: f64) -> bool {
        let Some((selection, previous)) = self.current_scale_correction() else {
            return false;
        };
        if previous.axis != self.gesture.transform.scale_axis_lock
            || !factor.is_finite()
            || factor <= 0.0
            || factor > 1_000.0
        {
            return false;
        }
        let Ok(parent) = self.document.tip_replacement_parent() else {
            return false;
        };
        let Ok(scale) = world_scale_transform(previous.centre_mm, factor, previous.axis) else {
            return false;
        };
        let Some(plan) = self.correction_transform_plan(&parent, &selection, scale) else {
            return false;
        };
        if !self.commit_transform_correction(&parent, plan) {
            return false;
        }
        self.record_transform_correction(selection, CorrectionOperation::Scale(previous.clone()));
        self.status_key = "status-object-scaled";
        self.digest = self.catalog.format(
            "digest-scale-committed",
            &BTreeMap::from([
                ("factor", format_scale_factor(factor)),
                (
                    "axis",
                    self.catalog
                        .text(previous.axis.map_or("axis-name-uniform", axis_name_key)),
                ),
            ]),
        );
        true
    }

    pub(crate) fn correct_move_delta(
        &mut self,
        selection: CorrectionSelection,
        previous: MoveCorrection,
        delta_mm: Vec3,
        accepts_vector_correction: bool,
    ) -> bool {
        let distance_mm = length(delta_mm);
        if !delta_mm.x.is_finite() || !delta_mm.y.is_finite() || !delta_mm.z.is_finite() {
            return false;
        }
        let Ok(parent) = self.document.tip_replacement_parent() else {
            return false;
        };
        let Ok(world_edit) = Transform::from_translation(delta_mm.x, delta_mm.y, delta_mm.z) else {
            return false;
        };
        let Some(plan) = self.correction_transform_plan(&parent, &selection, world_edit) else {
            return false;
        };
        if !self.commit_transform_correction(&parent, plan) {
            return false;
        }
        self.record_transform_correction(
            selection,
            CorrectionOperation::Move(MoveCorrection {
                direction: if distance_mm > 0.0 {
                    delta_mm * (1.0 / distance_mm)
                } else {
                    previous.direction
                },
                applied_distance_mm: distance_mm,
                accepts_vector_correction,
            }),
        );
        self.status_key = "status-object-moved";
        true
    }

    pub fn move_selected(&mut self, delta_mm: Vec3) -> bool {
        let snapshot = self.document.current();
        let Some(selection) = self.selected_move_reference() else {
            return false;
        };
        let occurrence_paths = self.selected_instance_paths();
        let Ok(world_edit) = Transform::from_translation(delta_mm.x, delta_mm.y, delta_mm.z) else {
            return false;
        };
        let Some(request) = self.transform_request(
            snapshot.document_id(),
            snapshot.revision_id(),
            &selection,
            &occurrence_paths,
            self.selection.selected_group,
            world_edit,
        ) else {
            return false;
        };
        let Some(plan) = TransformPlan::prepare(&snapshot, request) else {
            return false;
        };
        self.commit_move_transform(plan, &selection, delta_mm)
    }

    pub fn copy_selected(&mut self, delta_mm: Vec3) -> bool {
        let Some(selection) = self.selected_move_reference() else {
            return false;
        };
        let occurrence_paths = self.selected_instance_paths();
        self.copy_occurrences(&selection, &occurrence_paths, delta_mm)
    }

    pub(crate) fn correct_move_copy_delta(&mut self, delta_mm: Vec3) -> bool {
        if !delta_mm.x.is_finite() || !delta_mm.y.is_finite() || !delta_mm.z.is_finite() {
            return false;
        }
        let Some((_, mut previous)) = self.current_move_copy_correction() else {
            return false;
        };
        previous.delta_mm = delta_mm;
        self.apply_move_copy_array(previous.clone(), previous.array_mode, previous.array_count)
    }

    pub(crate) fn copy_source_plan(&self) -> Option<CopySourcePlan> {
        if self.selection.selected_group.is_some() {
            return None;
        }
        let occurrence_ids = self.selected_occurrence_ids();
        let occurrence_count = occurrence_ids.len();
        let snapshot = self.document.current();
        (occurrence_count > 0
            && occurrence_ids.iter().all(|id| {
                snapshot.occurrence(*id).is_some_and(|occurrence| {
                    snapshot.definition(occurrence.definition_id()).is_some()
                        && self.occurrence_in_active_context(&InstancePath::root(*id))
                })
            }))
        .then_some(CopySourcePlan {
            occurrence_ids,
            occurrence_count,
        })
    }

    pub(crate) fn apply_copy_source_plan(&mut self, plan: CopySourcePlan) -> bool {
        if plan.occurrence_count != plan.occurrence_ids.len()
            || self.copy_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        self.clipboard.occurrences = plan.occurrence_ids.into_iter().collect();
        self.clipboard.cut_occurrences.clear();
        self.digest = self.catalog.format(
            "digest-copied-to-clipboard",
            &BTreeMap::from([("count", plan.occurrence_count.to_string())]),
        );
        true
    }

    pub(crate) fn copy_selection_to_clipboard(&mut self) -> bool {
        let Some(plan) = self.copy_source_plan() else {
            return false;
        };
        self.apply_copy_source_plan(plan)
    }

    pub(crate) fn paste_source_plan(&self) -> Option<PasteSourcePlan> {
        if self.clipboard.occurrences.is_empty() || !self.selection.edit_context.is_empty() {
            return None;
        }
        let source_occurrence_ids = self
            .clipboard
            .occurrences
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let source_occurrence_count = self.clipboard.occurrences.len();
        if source_occurrence_count != source_occurrence_ids.len()
            || !self
                .clipboard
                .occurrences
                .iter()
                .copied()
                .eq(source_occurrence_ids.iter().copied())
            || (!self.clipboard.cut_occurrences.is_empty()
                && (self.clipboard.cut_occurrences.len() != source_occurrence_count
                    || !self
                        .clipboard
                        .cut_occurrences
                        .iter()
                        .map(|item| item.source_occurrence_id)
                        .eq(source_occurrence_ids.iter().copied())))
        {
            return None;
        }
        let snapshot = self.document.current();
        let source_revision = snapshot.revision_id();
        let mut next_id = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)?;
        let mut additions_by_definition = BTreeMap::<DefinitionId, usize>::new();
        let mut commands = Vec::new();
        let mut pasted = Vec::new();

        for source_id in source_occurrence_ids.iter().copied() {
            let cut_source = self
                .clipboard
                .cut_occurrences
                .iter()
                .find(|item| item.source_occurrence_id == source_id);
            let live_source = if cut_source.is_none() {
                Some(snapshot.occurrence(source_id)?)
            } else {
                None
            };
            let (definition_id, source_transform, parent, tags, visible, color) =
                if let Some(source) = cut_source {
                    (
                        source.definition_id,
                        source.transform,
                        source.parent,
                        source.tags.clone(),
                        source.visible,
                        source.color,
                    )
                } else {
                    let source = live_source?;
                    (
                        source.definition_id(),
                        source.transform(),
                        source.parent(),
                        source.tags().clone(),
                        source.visible(),
                        source.color(),
                    )
                };
            let definition = snapshot.definition(definition_id)?;
            let transform =
                translated_transform(source_transform, Vec3::new(100.0, 100.0, 0.0)).ok()?;
            let existing = snapshot
                .scene_query()
                .into_iter()
                .filter(|item| item.definition_id == definition_id)
                .count();
            let added = additions_by_definition.entry(definition_id).or_default();
            *added += 1;
            let target_id = OccurrenceId(next_id);
            next_id = next_id.checked_add(1)?;
            commands.push(CanonicalCommand::CreateOccurrence {
                id: target_id,
                definition_id,
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", definition.name().to_owned()),
                        ("number", (existing + *added).to_string()),
                    ]),
                ),
                transform,
                parent,
                tags,
                visible,
            });
            if let Some(color) = color {
                commands.push(CanonicalCommand::SetOccurrenceColor {
                    id: target_id,
                    color: Some(color),
                });
            }
            pasted.push((target_id, definition_id));
        }
        Some(PasteSourcePlan {
            source_revision,
            source_occurrence_ids,
            source_occurrence_count,
            commands,
            pasted,
        })
    }

    pub(crate) fn apply_paste_source_plan(&mut self, plan: PasteSourcePlan) -> bool {
        if plan.source_revision != self.document.current().revision_id()
            || plan.source_occurrence_count != plan.source_occurrence_ids.len()
            || self.paste_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        let PasteSourcePlan {
            commands, pasted, ..
        } = plan;
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }

        self.selection.clear();
        self.selection
            .occurrences
            .extend(pasted.iter().map(|(id, _)| InstancePath::root(*id)));
        if let Some((occurrence_id, definition_id)) = pasted.first().copied() {
            self.selection.primary = Some(SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            });
        }
        self.status_key = "status-object-copied";
        self.digest = self.catalog.format(
            "digest-pasted-from-clipboard",
            &BTreeMap::from([("count", pasted.len().to_string())]),
        );
        true
    }

    pub(crate) fn paste_clipboard(&mut self) -> bool {
        let Some(plan) = self.paste_source_plan() else {
            return false;
        };
        self.apply_paste_source_plan(plan)
    }

    pub fn rotate_selected_90(&mut self) -> bool {
        let Some(selection) = &self.selection.primary else {
            return false;
        };
        let snapshot = self.document.current();
        if !selection.instance_path.is_root() {
            return false;
        }
        let occurrence_id = selection.instance_path.root_occurrence();
        let Some(occurrence) = snapshot.occurrence(occurrence_id) else {
            return false;
        };
        let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
        let Some(local_box) = projection
            .occurrences()
            .iter()
            .find(|projected| projected.occurrence_id == occurrence_id)
            .and_then(|projected| projected.local_box)
        else {
            return false;
        };
        let Ok(transform) = rotate_transform_90(occurrence.transform(), local_box) else {
            return false;
        };
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: occurrence_id,
                    transform,
                },
            ]))
            .is_err()
        {
            return false;
        }
        self.status_key = "status-object-rotated";
        true
    }

    pub(crate) fn push_pull_face_selected(&self) -> bool {
        matches!(
            self.selection
                .primary
                .as_ref()
                .map(|selection| selection.element.clone()),
            Some(ElementId::Face { .. } | ElementId::TopologicalFace(_))
        )
    }

    pub fn set_push_pull_distance_input(&mut self, value: impl Into<String>) {
        self.push_pull.distance_input = value.into();
    }

    pub(crate) fn prepare_smart_push_pull_proposal(
        &self,
        batch: CommandBatch,
        principal: ProposalPrincipal,
    ) -> Option<Proposal> {
        let context = match principal {
            ProposalPrincipal::ManualClient => ProposalContext::canonical_preview(),
            ProposalPrincipal::LocalAssistant => ProposalContext::local_assistant_model(),
            _ => return None,
        };
        self.prepare_smart_push_pull_proposal_with_context(batch, context)
            .ok()
    }

    pub(crate) fn prepare_smart_push_pull_proposal_with_context(
        &self,
        batch: CommandBatch,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        self.document.prepare_proposal_with_context(batch, context)
    }

    pub(crate) fn push_pull_planning_snapshot(&self) -> Snapshot {
        match self.push_pull.smart_planning.as_ref() {
            Some(SmartPushPullPlanning::TipReplacement(parent)) => parent.snapshot().clone(),
            _ => self.document.current(),
        }
    }

    pub(crate) fn prepare_manual_push_pull_proposal(
        &self,
        batch: CommandBatch,
    ) -> Option<SmartPushPullProposal> {
        match self.push_pull.smart_planning.as_ref() {
            Some(SmartPushPullPlanning::TipReplacement(parent)) => self
                .document
                .prepare_tip_replacement_proposal(
                    parent,
                    batch,
                    ProposalContext::canonical_preview(),
                )
                .ok()
                .map(SmartPushPullProposal::TipReplacement),
            _ => self
                .prepare_smart_push_pull_proposal(batch, ProposalPrincipal::ManualClient)
                .map(SmartPushPullProposal::Append),
        }
    }

    pub(crate) fn push_pull_planning_plan(&self) -> Option<PushPullPlanningPlan> {
        match self.push_pull.smart_planning.as_ref()? {
            SmartPushPullPlanning::Append => Some(PushPullPlanningPlan::Append),
            SmartPushPullPlanning::TipReplacement(parent) => {
                Some(PushPullPlanningPlan::TipReplacement {
                    document_id: parent.document_id(),
                    parent_revision: parent.parent_revision(),
                    parent_digest: parent.parent_digest().to_owned(),
                    superseded_revision: parent.superseded_revision(),
                    superseded_digest: parent.superseded_digest().to_owned(),
                })
            }
        }
    }

    pub(crate) fn push_pull_source_plan(&self, target: &SelectionId) -> Option<PushPullSourcePlan> {
        let snapshot = self.document.current();
        let planning_snapshot = self.push_pull_planning_snapshot();
        let (topological_selection, topological_reference) =
            match self.selection.topological.as_slice() {
                [] => (None, None),
                [(_, topological)] => {
                    let resolved = topological
                        .resolve_current(&snapshot, &self.exact.topology_results)
                        .ok()?;
                    if resolved.instance_path != target.instance_path
                        || resolved.reference.definition_id != target.definition_id
                        || resolved.reference.kind != TopologicalElementKind::Face
                        || self
                            .topological_push_pull_face_element(&snapshot, &resolved.reference)?
                            != target.element
                    {
                        return None;
                    }
                    (Some(topological.clone()), Some(resolved.reference))
                }
                _ => return None,
            };
        let target_box = self.planar_source_box(target).or_else(|| {
            self.active_boxes_for_snapshot(&planning_snapshot)
                .into_iter()
                .find(|item| item.instance_path == target.instance_path)
                .or_else(|| {
                    self.topological_push_pull_box(
                        &snapshot,
                        target,
                        topological_reference.as_ref()?,
                    )
                })
        })?;
        if target_box.definition_id != target.definition_id {
            return None;
        }
        Some(PushPullSourcePlan {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            planning: self.push_pull_planning_plan()?,
            target: target.clone(),
            topological_selection,
            topological_reference,
            target_box,
        })
    }

    pub(crate) fn derive_push_pull_preview_plan(
        &self,
        source: &PushPullSourcePlan,
        principal: ProposalPrincipal,
        distance_expression: &str,
        distance_mm: f64,
    ) -> Option<(PushPullPreviewPlan, CommandBatch, SmartPushPullProposal)> {
        if self.push_pull_source_plan(&source.target).as_ref() != Some(source)
            || !distance_mm.is_finite()
        {
            return None;
        }
        if source.topological_reference.is_some() {
            return self.derive_planar_preview(source, principal, distance_expression, distance_mm);
        }
        let current_extent_mm = face_extent(&source.target_box, Some(&source.target.element))?;
        let new_extent = computed_length(current_extent_mm + distance_mm)?;
        let new_extent_mm = new_extent.millimetres();
        let preview_box =
            resize_box_from_face(&source.target_box, &source.target.element, new_extent_mm)
                .or_else(|| {
                    (source.target_box.extrusion_feature_id.is_none()
                        && new_extent_mm < 0.0
                        && source.target.element
                            == (ElementId::Face {
                                axis: Axis::Z,
                                side: Side::Maximum,
                            }))
                    .then(|| {
                        let mut preview_box = source.target_box.clone();
                        preview_box.origin_mm.z += new_extent_mm;
                        preview_box.size_mm.z = -new_extent_mm;
                        preview_box
                    })
                })?;
        let planning_snapshot = self.push_pull_planning_snapshot();
        let batch = push_pull_batch(
            &planning_snapshot,
            &source.target,
            &source.target_box,
            source.topological_reference.as_ref(),
            distance_mm,
            new_extent_mm,
            new_extent.source_token().to_owned(),
        )?;
        let proposal = if principal == ProposalPrincipal::ManualClient {
            self.prepare_manual_push_pull_proposal(batch.clone())
        } else {
            self.prepare_smart_push_pull_proposal(batch.clone(), principal)
                .map(SmartPushPullProposal::Append)
        }?;
        if proposal.batch() != &batch {
            return None;
        }
        let shared_count = planning_snapshot
            .scene_query()
            .into_iter()
            .find(|candidate| candidate.instance_path == source.target.instance_path)
            .map_or(1, |candidate| candidate.shared_occurrence_count);
        Some((
            PushPullPreviewPlan {
                source: source.clone(),
                principal,
                distance_expression: distance_expression.to_owned(),
                distance_mm_bits: distance_mm.to_bits(),
                current_extent_mm_bits: current_extent_mm.to_bits(),
                new_extent_mm_bits: new_extent_mm.to_bits(),
                commands: batch.commands().to_vec(),
                preview_box,
                shared_count,
            },
            batch,
            proposal,
        ))
    }

    pub(crate) fn prepare_box_push_pull_preview(
        &mut self,
        selection: SelectionId,
        item: RenderBox,
        distance_mm: f64,
        principal: ProposalPrincipal,
    ) -> bool {
        let distance_expression = self.push_pull.distance_input.clone();
        if parse_distance_mm(&distance_expression).map(f64::to_bits) != Some(distance_mm.to_bits())
        {
            return false;
        }
        let Some(source) = self.push_pull_source_plan(&selection) else {
            return false;
        };
        if source.target_box != item {
            return false;
        }
        let Some((plan, batch, proposal)) = self.derive_push_pull_preview_plan(
            &source,
            principal,
            &distance_expression,
            distance_mm,
        ) else {
            return false;
        };
        let new_extent_mm = f64::from_bits(plan.new_extent_mm_bits);
        let shared_count = plan.shared_count;
        let digest = proposal.command_digest().to_owned();
        self.push_pull.smart_proposal = Some(proposal);
        self.tool_preview.close::<OccurrenceOperationPreview>();
        if plan.source.topological_reference.is_some()
            && self
                .tool_preview
                .get::<EphemeralBoxPreview>()
                .is_none_or(|preview| preview.plan != plan)
        {
            self.push_pull.face_offset_preview_due =
                Some(Instant::now() + Duration::from_millis(150));
        }
        let preview = EphemeralBoxPreview { plan, batch };
        // Derived just now from this state, so its check need not derive it again.
        let key = self.preview_check_key(&self.document.current(), &preview);
        *self.push_pull.preview_check.borrow_mut() = Some((key, Some(digest)));
        self.tool_preview.open(preview);
        self.status_key = "status-preview";
        self.digest = match &selection.element {
            ElementId::Face { axis: Axis::Z, .. } => self.catalog.format(
                "digest-push-pull-live",
                &BTreeMap::from([
                    ("distance", format_signed_mm(distance_mm)),
                    ("height", format_height(new_extent_mm)),
                    ("count", shared_count.to_string()),
                ]),
            ),
            ElementId::Face { .. } | ElementId::TopologicalFace(_) => self.catalog.format(
                "digest-push-pull-profile-live",
                &BTreeMap::from([
                    ("distance", format_signed_mm(distance_mm)),
                    ("extent", format_height(new_extent_mm)),
                ]),
            ),
            _ => self.catalog.text("digest-nothing-to-apply"),
        };
        true
    }

    #[must_use]
    pub fn push_pull_preview_exact_evaluator(&self) -> Option<&'static str> {
        let (batch, definition_id) = if self.has_preview() {
            let preview = self.tool_preview.get::<EphemeralBoxPreview>()?;
            (&preview.batch, preview.plan.source.target.definition_id)
        } else if self.has_occurrence_operation_preview() {
            let preview = self.tool_preview.get::<OccurrenceOperationPreview>()?;
            (
                &preview.batch,
                preview.selection_after.as_ref()?.definition_id,
            )
        } else {
            return None;
        };
        let snapshot = if let Some(proposal) = self.push_pull.smart_proposal.as_ref() {
            if proposal.batch() != batch {
                return None;
            }
            proposal.preview(&self.document)?
        } else {
            self.document.preview_batch(batch).ok()?
        };
        ExactBRepGraph::from_snapshot(
            &snapshot,
            definition_id,
            exact_solid_tool_feature_id(&snapshot, definition_id)?,
        )
        .ok()
        .map(|_| ketchup_model::exact_product::EXACT_BREP_GRAPH_EVALUATOR_V1)
    }

    #[must_use]
    pub fn push_pull_preview_render_depth_mm(&self) -> Option<f64> {
        let preview = self.tool_preview.get::<EphemeralBoxPreview>()?;
        let mesh = self
            .canonical_profile_viewport_mesh(&self.document.current(), &preview.plan.preview_box)?;
        let (minimum, maximum) = mesh.0.iter().map(|position| position[2]).fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(minimum, maximum), z| (minimum.min(z), maximum.max(z)),
        );
        (minimum.is_finite() && maximum.is_finite()).then_some(maximum - minimum)
    }

    pub(crate) fn confirm_push_pull_preview(&mut self) -> bool {
        if self.has_occurrence_operation_preview() {
            self.confirm_occurrence_operation_preview()
        } else {
            self.confirm_preview()
        }
    }

    pub(crate) fn clear_push_pull_preview(&mut self) {
        self.push_pull.face_offset_evaluation = None;
        self.push_pull.face_offset_preview_due = None;
        self.tool_preview.close::<EphemeralBoxPreview>();
        self.push_pull.smart_proposal = None;
        self.push_pull.smart_planning = None;
        self.tool_preview.close::<OccurrenceOperationPreview>();
        self.tool_preview.close::<drawn_shape::DrawnShapePreview>();
    }

    pub(crate) fn commit_transform_plan(
        &mut self,
        plan: TransformPlan,
    ) -> Result<TransformTarget, WorkRecoveryMutationError<CanonicalError>> {
        let (target, batch) = plan.into_commit();
        self.apply_batch_with_work_recovery(&batch)?;
        Ok(target)
    }

    pub(crate) fn commit_transform_correction(
        &mut self,
        parent: &TipReplacementParent,
        plan: TransformPlan,
    ) -> bool {
        let (_, batch) = plan.into_commit();
        let Ok(proposal) = self.document.prepare_tip_replacement_proposal(
            parent,
            batch,
            ProposalContext::canonical_preview(),
        ) else {
            return false;
        };
        self.complete_mutation_with_work_recovery(|document| {
            document.commit_tip_replacement_proposal(&proposal)
        })
        .is_ok()
    }

    pub(crate) fn push_pull_gesture_is_current(&self, drag: &PushPullDrag) -> bool {
        let snapshot = self.document.current();
        drag.source_document_id == snapshot.document_id()
            && drag.source_revision == snapshot.revision_id()
            && drag.source_digest == snapshot.canonical_digest()
            && self.selection.primary.as_ref() == Some(&drag.selection)
    }

    pub(crate) fn push_pull_target_snap(&self) -> Option<SnapResult> {
        if !self.face_workflow.snaps_enabled() {
            return None;
        }
        if self.hover.overlap_index == 0
            && let Some(snap) = self
                .hover
                .snap
                .as_ref()
                .filter(|snap| snap.kind != SnapKind::Face)
        {
            return Some(snap.clone());
        }
        let hit = self
            .hover
            .pick
            .as_ref()?
            .overlap_choice(self.hover.overlap_index)?;
        Some(SnapResult {
            kind: SnapKind::Face,
            reference: hit.reference.clone(),
            position_mm: hit.position_mm,
            distance_mm: 0.0,
        })
    }

    pub(crate) fn push_pull_snap_distance(&self, drag: &PushPullDrag) -> Option<f64> {
        let target = self.push_pull_target_snap()?;
        if target.kind == SnapKind::Face && target.reference == drag.selection {
            return None;
        }
        if let Some(face) = self.selected_planar_face(&drag.selection) {
            let distance = dot(target.position_mm - face.origin, face.normal);
            // A face in the start plane (the one a profile was drawn on, say)
            // would push by nothing.
            return (distance.is_finite() && distance.abs() >= 0.01).then_some(distance);
        }
        let ElementId::Face {
            axis: source_axis,
            side: source_side,
        } = drag.selection.element
        else {
            return None;
        };
        let source = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == drag.selection.instance_path)?;
        let (source_coordinate, target_coordinate) = match source_axis {
            Axis::X => (
                source.origin_mm.x
                    + if source_side == Side::Maximum {
                        source.size_mm.x
                    } else {
                        0.0
                    },
                target.position_mm.x,
            ),
            Axis::Y => (
                source.origin_mm.y
                    + if source_side == Side::Maximum {
                        source.size_mm.y
                    } else {
                        0.0
                    },
                target.position_mm.y,
            ),
            Axis::Z => (
                source.origin_mm.z
                    + if source_side == Side::Maximum {
                        source.size_mm.z
                    } else {
                        0.0
                    },
                target.position_mm.z,
            ),
        };
        let outward_sign = if source_side == Side::Maximum {
            1.0
        } else {
            -1.0
        };
        let distance = (target_coordinate - source_coordinate) * outward_sign;
        (distance.is_finite()
            && distance.abs() >= 0.01
            && (drag.extent_start_mm <= 0.01 || distance > -drag.extent_start_mm + 0.01))
            .then_some(distance)
    }

    pub(crate) fn update_push_pull_gesture(&mut self, drag: &PushPullDrag, pointer: Pos2) -> bool {
        if !self.push_pull_gesture_is_current(drag) {
            self.clear_ephemeral_edit_state();
            self.status_key = "error-preview-stale";
            self.digest = self.catalog.text("error-preview-stale");
            return false;
        }
        let snapped = self.push_pull_snap_distance(drag);
        let distance = snapped.unwrap_or_else(|| {
            push_pull_distance_from_pointer(drag, pointer, self.face_workflow.snaps_enabled())
        });
        let input = snapped.map_or_else(|| format_height(distance), |value| value.to_string());
        // A held button repaints every frame; re-planning an unchanged
        // distance would cost a whole-document proposal per frame.
        if input == self.push_pull.distance_input
            && (self.has_preview() || self.has_drawn_shape_preview())
        {
            return true;
        }
        self.push_pull.distance_input = input;
        self.value_box.input = self.push_pull.distance_input.clone();
        if distance.abs() >= 0.01 {
            self.start_preview()
        } else {
            self.clear_push_pull_preview();
            self.status_key = "status-ready";
            self.digest = self.catalog.text("digest-nothing-to-apply");
            false
        }
    }

    #[must_use]
    pub fn push_pull_click_anchor_active(&self) -> bool {
        self.gesture.drag.get::<PushPullAnchor>().is_some()
    }

    pub(crate) fn push_pull_pointer_target(&self) -> Option<SelectionId> {
        self.hover
            .target
            .clone()
            .filter(|selection| {
                matches!(
                    selection.element,
                    ElementId::Face { .. } | ElementId::TopologicalFace(_)
                )
            })
            .or_else(|| {
                self.selection
                    .primary
                    .as_ref()
                    .filter(|selection| {
                        matches!(
                            selection.element,
                            ElementId::Face { .. } | ElementId::TopologicalFace(_)
                        ) && self.hover.pick.as_ref().is_some_and(|pick| {
                            pick.overlapping
                                .iter()
                                .any(|hit| hit.reference == **selection)
                        })
                    })
                    .cloned()
            })
    }

    /// Whether the live UI selection still belongs to this transform session.
    /// Document freshness and target existence are owned by `TransformRequest`.
    pub(crate) fn transform_selection_is_current(
        &self,
        occurrence_paths: &BTreeSet<InstancePath>,
        group_id: Option<GroupId>,
    ) -> bool {
        if let Some(group_id) = group_id {
            self.selection.selected_group == Some(group_id)
        } else {
            self.selection.selected_group.is_none()
                && self.selected_instance_paths() == *occurrence_paths
        }
    }

    pub(crate) fn move_preview_is_current(&self, drag: &MoveDrag) -> bool {
        if !drag.copy && drag.profile_target.is_none() {
            return self.move_transform_plan(drag).is_some();
        }
        self.transform_source_is_current(
            drag.source_document_id,
            drag.source_revision,
            &drag.selection,
            &drag.occurrence_paths,
            drag.group_id,
        )
    }

    pub(crate) fn transform_request(
        &self,
        source_document_id: DocumentId,
        source_revision: u64,
        selection: &SelectionId,
        occurrence_paths: &BTreeSet<InstancePath>,
        group_id: Option<GroupId>,
        world_edit: Transform,
    ) -> Option<TransformRequest> {
        self.transform_selection_is_current(occurrence_paths, group_id)
            .then(|| TransformRequest {
                source_document_id,
                source_revision,
                primary_occurrence: selection.instance_path.root_occurrence(),
                primary_definition: selection.definition_id,
                target: group_id.map_or_else(
                    || TransformTarget::Occurrences(occurrence_paths.clone()),
                    TransformTarget::Group,
                ),
                world_edit,
            })
    }

    pub(crate) fn transform_source_is_current(
        &self,
        source_document_id: DocumentId,
        source_revision: u64,
        selection: &SelectionId,
        occurrence_paths: &BTreeSet<InstancePath>,
        group_id: Option<GroupId>,
    ) -> bool {
        self.transform_request(
            source_document_id,
            source_revision,
            selection,
            occurrence_paths,
            group_id,
            Transform::identity(),
        )
        .is_some_and(|request| request.matches_source(&self.document.current()))
    }

    pub(crate) fn move_transform_plan(&self, drag: &MoveDrag) -> Option<TransformPlan> {
        if drag.copy
            || drag.profile_target.is_some()
            || (drag.group_id.is_none()
                && matches!(
                    self.selection.edit_context.last(),
                    Some(EditContext::Definition { .. })
                ))
        {
            return None;
        }
        let request = self.transform_request(
            drag.source_document_id,
            drag.source_revision,
            &drag.selection,
            &drag.occurrence_paths,
            drag.group_id,
            Transform::from_translation(drag.delta_mm.x, drag.delta_mm.y, drag.delta_mm.z).ok()?,
        )?;
        TransformPlan::prepare(&self.document.current(), request)
    }

    pub(crate) fn rotate_transform_plan(&self, drag: &RotateDrag) -> Option<TransformPlan> {
        if drag.copy
            || (drag.group_id.is_none()
                && matches!(
                    self.selection.edit_context.last(),
                    Some(EditContext::Definition { .. })
                ))
        {
            return None;
        }
        let request = self.transform_request(
            drag.source_document_id,
            drag.source_revision,
            &drag.selection,
            &drag.occurrence_paths,
            drag.group_id,
            world_rotation_transform(drag.centre_mm, drag.axis, drag.angle_degrees).ok()?,
        )?;
        TransformPlan::prepare(&self.document.current(), request)
    }

    pub(crate) fn scale_transform_plan(&self, drag: &ScaleDrag) -> Option<TransformPlan> {
        if drag.group_id.is_none()
            && (matches!(
                self.selection.edit_context.last(),
                Some(EditContext::Definition { .. })
            ) || drag
                .occurrence_paths
                .iter()
                .any(|path| !self.occurrence_in_active_context(path)))
        {
            return None;
        }
        let request = self.transform_request(
            drag.source_document_id,
            drag.source_revision,
            &drag.selection,
            &drag.occurrence_paths,
            drag.group_id,
            world_scale_transform(drag.centre_mm, drag.factor, drag.axis).ok()?,
        )?;
        TransformPlan::prepare(&self.document.current(), request)
    }

    pub(crate) fn move_profile_target_at(
        &self,
        snapshot: &Snapshot,
        selection: &SelectionId,
        pointer: Pos2,
        rect: Rect,
        copy: bool,
        group_id: Option<GroupId>,
    ) -> Option<MoveProfileTarget> {
        if copy || group_id.is_some() {
            return None;
        }
        let ray = self.view_ray(pointer, rect)?;
        let hit = self.exact_projection(snapshot).exact_surface_pick(ray)?;
        let hit_position = hit.position_mm;
        if hit.instance_path != selection.instance_path
            || hit.definition_id != selection.definition_id
        {
            return None;
        }
        let ExactBodyPackage::Graph(package) = self
            .exact
            .results
            .get_render(snapshot, selection.definition_id)?
            .as_ref()
        else {
            return None;
        };
        let hit_occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| occurrence.instance_path == selection.instance_path)?;
        let local_hit = inverse_transform_point(hit_occurrence.transform, hit_position)?;
        // Tessellated walls deviate from the exact surface by the mesh deflection.
        let profile_id = FeatureId(
            package
                .graph
                .profile_cut_at([local_hit.x, local_hit.y, local_hit.z], 0.1)?,
        );
        let definition = snapshot.definition(selection.definition_id)?;
        // A pocket pushed from a drawn shape keeps its profile in the drawing's
        // plane, in the tool's own body, placed on the part by a rigid transform.
        let placement = definition.feature_ids().iter().find_map(|id| {
            let FeatureKind::RigidTransform { target, transform } = snapshot.feature(*id)?.kind()
            else {
                return None;
            };
            matches!(
                snapshot.feature(*target)?.kind(),
                FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(profile), extent: FeatureExtent::Blind(_), operation: PadOperation::NewBody, .. }) if *profile == profile_id
            )
            .then_some((*target, *transform))
        });
        let body_producer = placement.map_or(
            FeatureId(package.graph.producer_feature_id),
            |(prism, _)| prism,
        );
        let body_id = definition
            .feature_body_ownership(body_producer)?
            .output_body_id()?;
        let profile = snapshot.feature(profile_id)?;
        let (local_origin, local_x_axis, local_y_axis) = match profile.kind() {
            FeatureKind::Sketch(spec) => {
                let FeatureKind::Workplane(workplane) = snapshot.feature(spec.workplane)?.kind()
                else {
                    return None;
                };
                (
                    Vec3::new(
                        workplane.frame.origin_mm[0],
                        workplane.frame.origin_mm[1],
                        workplane.frame.origin_mm[2],
                    ),
                    Vec3::new(
                        workplane.frame.x_axis[0],
                        workplane.frame.x_axis[1],
                        workplane.frame.x_axis[2],
                    ),
                    Vec3::new(
                        workplane.frame.y_axis[0],
                        workplane.frame.y_axis[1],
                        workplane.frame.y_axis[2],
                    ),
                )
            }
            FeatureKind::Profile { .. } => (
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ),
            _ => return None,
        };
        let (local_origin, local_x_axis, local_y_axis) = match placement {
            Some((_, transform)) => {
                let m = transform.matrix();
                let axis = |v: Vec3| {
                    Vec3::new(
                        m[0] * v.x + m[1] * v.y + m[2] * v.z,
                        m[4] * v.x + m[5] * v.y + m[6] * v.z,
                        m[8] * v.x + m[9] * v.y + m[10] * v.z,
                    )
                };
                (
                    transform_model_point(transform, local_origin),
                    axis(local_x_axis),
                    axis(local_y_axis),
                )
            }
            None => (local_origin, local_x_axis, local_y_axis),
        };
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| occurrence.instance_path == selection.instance_path)?;
        let matrix = occurrence.transform.matrix();
        let to_world = |axis: Vec3| {
            Vec3::new(
                matrix[0] * axis.x + matrix[1] * axis.y + matrix[2] * axis.z,
                matrix[4] * axis.x + matrix[5] * axis.y + matrix[6] * axis.z,
                matrix[8] * axis.x + matrix[9] * axis.y + matrix[10] * axis.z,
            )
        };
        let world_x_axis = to_world(local_x_axis);
        let world_y_axis = to_world(local_y_axis);
        let mut world_origin = transform_model_point(occurrence.transform, local_origin);
        let normal = cross(world_x_axis, world_y_axis);
        let normal_length = length(normal);
        if normal_length > ROUNDING {
            let unit_normal = normal * (1.0 / normal_length);
            world_origin =
                world_origin + unit_normal * dot(hit_position - world_origin, unit_normal);
        }
        Some(MoveProfileTarget {
            definition_id: selection.definition_id,
            body_id,
            profile_id,
            world_origin,
            world_x_axis,
            world_y_axis,
        })
    }

    pub(crate) fn rotate_preview_is_current(&self, drag: &RotateDrag) -> bool {
        if !drag.copy {
            return self.rotate_transform_plan(drag).is_some();
        }
        self.transform_source_is_current(
            drag.source_document_id,
            drag.source_revision,
            &drag.selection,
            &drag.occurrence_paths,
            drag.group_id,
        )
    }

    pub(crate) fn move_drag_applies_to_path(
        &self,
        drag: &MoveDrag,
        instance_path: &InstancePath,
    ) -> bool {
        if let Some(group_id) = drag.group_id {
            Self::group_contains_occurrence(
                &self.document.current(),
                group_id,
                instance_path.root_occurrence(),
            )
        } else {
            drag.occurrence_paths.contains(instance_path)
        }
    }

    pub(crate) fn move_preview_transform_overrides(&self) -> BTreeMap<InstancePath, Transform> {
        self.move_session()
            .and_then(|(drag, _)| self.move_transform_plan(drag))
            .map(|plan| plan.preview_overrides().clone())
            .unwrap_or_default()
    }

    /// World-space paths painted as the non-authoritative cut-profile Move preview.
    #[must_use]
    pub fn move_profile_preview_paths(&self) -> Vec<Vec<Vec3>> {
        let Some((drag, _)) = self.move_session() else {
            return Vec::new();
        };
        if !self.move_preview_is_current(drag) || drag.copy {
            return Vec::new();
        }
        let Some(target) = drag.profile_target.as_ref() else {
            return Vec::new();
        };
        let snapshot = self.document.current();
        let Some(profile) = snapshot.feature(target.profile_id) else {
            return Vec::new();
        };
        let delta_x = dot(drag.delta_mm, target.world_x_axis);
        let delta_y = dot(drag.delta_mm, target.world_y_axis);
        let preview_offset = target.world_x_axis * delta_x + target.world_y_axis * delta_y;
        let world = |point: [f64; 2]| {
            target.world_origin
                + target.world_x_axis * point[0]
                + target.world_y_axis * point[1]
                + preview_offset
        };
        let arc = |start_mm, end_mm, center_mm, clockwise| {
            profile_arc_polyline(start_mm, end_mm, center_mm, clockwise, 32)
                .into_iter()
                .map(world)
                .collect::<Vec<_>>()
        };

        match profile.kind() {
            FeatureKind::Sketch(spec) => spec
                .entities
                .iter()
                .map(|entity| match entity {
                    SketchEntity::Line {
                        start_mm, end_mm, ..
                    } => vec![world(*start_mm), world(*end_mm)],
                    SketchEntity::Arc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                        ..
                    } => arc(*start_mm, *end_mm, *center_mm, *clockwise),
                    SketchEntity::CubicBezier {
                        start_mm,
                        control_1_mm,
                        control_2_mm,
                        end_mm,
                        ..
                    } => (0..=32)
                        .map(|step| {
                            let curve = CubicBezier::new([
                                *start_mm,
                                *control_1_mm,
                                *control_2_mm,
                                *end_mm,
                            ]);
                            world(curve.eval(f64::from(step) / 32.0))
                        })
                        .collect(),
                    SketchEntity::Circle {
                        center_mm,
                        radius_mm,
                        ..
                    } => (0..=64)
                        .map(|step| {
                            let angle = std::f64::consts::TAU * f64::from(step) / 64.0;
                            world([
                                center_mm[0] + radius_mm * angle.cos(),
                                center_mm[1] + radius_mm * angle.sin(),
                            ])
                        })
                        .collect(),
                })
                .collect(),
            FeatureKind::Profile { segments, .. } => segments
                .iter()
                .map(|segment| match segment {
                    ProfileSegment::Line { start_mm, end_mm } => {
                        vec![world(*start_mm), world(*end_mm)]
                    }
                    ProfileSegment::CircularArc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    } => arc(*start_mm, *end_mm, *center_mm, *clockwise),
                    ProfileSegment::CubicBezier {
                        start_mm,
                        control_1_mm,
                        control_2_mm,
                        end_mm,
                    } => (0..=32)
                        .map(|step| {
                            let curve = CubicBezier::new([
                                *start_mm,
                                *control_1_mm,
                                *control_2_mm,
                                *end_mm,
                            ]);
                            world(curve.eval(f64::from(step) / 32.0))
                        })
                        .collect(),
                    // The guide runs through the points the spline passes through.
                    ProfileSegment::Spline { points_mm } => {
                        points_mm.iter().copied().map(world).collect()
                    }
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn begin_move_drag_at(&mut self, pointer: Pos2, rect: Rect, copy: bool) -> bool {
        let snapshot = self.document.current();
        let circular_profile = (!copy)
            .then(|| self.circular_cut_profile_target_at_screen(&snapshot, pointer, rect))
            .flatten();
        let Some(selection) = circular_profile
            .as_ref()
            .map(|(selection, _, _)| selection.clone())
            .or_else(|| {
                self.hover
                    .snap
                    .as_ref()
                    .filter(|snap| snap.kind != SnapKind::Face)
                    .map(|snap| snap.reference.clone())
                    .or_else(|| self.hover.target.clone())
                    .filter(|selection| self.occurrence_in_active_context(&selection.instance_path))
            })
        else {
            self.digest = self.catalog.text("digest-move-start-missed");
            return false;
        };
        if circular_profile.is_some() || !self.selection.contains(&selection.instance_path) {
            self.select_from_viewport(Some(selection.clone()), false);
        }
        let plane_z = circular_profile
            .as_ref()
            .map(|(_, _, position)| position.z)
            .or_else(|| {
                self.hover
                    .pick
                    .as_ref()
                    .filter(|pick| pick.primary.reference.instance_path == selection.instance_path)
                    .map(|pick| pick.primary.position_mm.z)
                    .or_else(|| {
                        self.active_boxes()
                            .into_iter()
                            .find(|item| item.instance_path == selection.instance_path)
                            .map(|item| item.origin_mm.z)
                    })
            });
        let Some(plane_z) = plane_z else {
            return false;
        };
        let pointer_start_world = circular_profile
            .as_ref()
            .map(|(_, _, position)| *position)
            .or_else(|| {
                self.hover
                    .snap
                    .as_ref()
                    .filter(|snap| {
                        snap.kind != SnapKind::Face
                            && snap.reference.instance_path == selection.instance_path
                    })
                    .map(|snap| snap.position_mm)
                    .or_else(|| self.screen_to_plane(pointer, rect, plane_z))
            });
        let Some(pointer_start_world) = pointer_start_world else {
            return false;
        };
        self.value_box.input = "0".to_owned();
        let group_id = self.selection.selected_group;
        let profile_target = circular_profile
            .as_ref()
            .map(|(_, target, _)| target.clone())
            .or_else(|| {
                self.move_profile_target_at(&snapshot, &selection, pointer, rect, copy, group_id)
            });
        // The press itself is where the travel starts. Waiting for the first
        // pointer sample instead would silently drop the opening slice of the
        // gesture, and the body would land short of where it was dragged.
        let axis_reference = self.gesture.transform.move_axis_lock.and_then(|axis| {
            let ray = self.view_ray(pointer, rect)?;
            axis_travel_along(&ray, pointer_start_world, axis)
        });
        let occurrence_paths = if profile_target.is_some() {
            BTreeSet::from([selection.instance_path.clone()])
        } else {
            self.selected_instance_paths()
        };
        self.set_move_session(
            ToolSessionPhase::Gesture,
            MoveDrag {
                source_document_id: snapshot.document_id(),
                source_revision: snapshot.revision_id(),
                selection,
                occurrence_paths,
                group_id,
                profile_target,
                pointer_start_world,
                plane_z: pointer_start_world.z,
                axis: self.gesture.transform.move_axis_lock,
                axis_reference,
                delta_mm: Vec3::ZERO,
                copy: group_id.is_none() && copy,
            },
        );
        true
    }

    /// Re-read the live translation from the pointer, either along the pinned
    /// axis or across the horizontal plane the gesture started on.
    ///
    /// The first sample after a pin only establishes where the gesture sits on
    /// that axis, so pinning mid-drag never makes the body jump.
    pub(crate) fn advance_move(&self, drag: &mut MoveDrag, pointer: Pos2, rect: Rect, free: bool) {
        let Some(axis) = drag.axis else {
            if let Some(snap) = self.hover.snap.as_ref().filter(|snap| {
                snap.kind != SnapKind::Face
                    && snap.reference.instance_path != drag.selection.instance_path
            }) {
                drag.delta_mm = snap.position_mm - drag.pointer_start_world;
            } else if let Some(world) = self.screen_to_plane(pointer, rect, drag.plane_z) {
                let delta = continuous_move_delta(drag.pointer_start_world, world, free);
                let destination = drag.pointer_start_world + delta;
                drag.delta_mm = if free {
                    delta
                } else {
                    self.move_inference_target_at_screen(pointer, rect, destination)
                        .map_or(delta, |target| target - drag.pointer_start_world)
                };
            }
            return;
        };
        let Some(ray) = self.view_ray(pointer, rect) else {
            return;
        };
        let Some(travel) = axis_travel_along(&ray, drag.pointer_start_world, axis) else {
            return;
        };
        let Some(reference) = drag.axis_reference else {
            drag.axis_reference = Some(travel);
            drag.delta_mm = Vec3::ZERO;
            return;
        };
        let distance = travel - reference;
        drag.delta_mm = axis_direction(axis) * distance;
    }

    pub(crate) fn apply_transform_input_event(
        &mut self,
        tool: ActiveTool,
        event: TransformInputEvent,
    ) {
        match (tool, event) {
            (ActiveTool::Move, TransformInputEvent::ToggleCopy) => {
                let copy_allowed = self.selection.selected_group.is_none();
                self.gesture.transform.move_copy =
                    copy_allowed && !self.gesture.transform.move_copy;
                let copy_mode = self.gesture.transform.move_copy;
                if let Some(drag) = self.move_session_mut() {
                    drag.copy = drag.group_id.is_none() && copy_mode;
                }
                let delta_mm = self
                    .move_session()
                    .map_or(Vec3::ZERO, |(drag, _)| drag.delta_mm);
                self.digest = self.catalog.format(
                    if self.gesture.transform.move_copy {
                        "digest-copy-live"
                    } else {
                        "digest-move-live"
                    },
                    &BTreeMap::from([
                        ("distance", format_height(length(delta_mm))),
                        ("vector", format_vector_mm(delta_mm)),
                    ]),
                );
            }
            (ActiveTool::Rotate, TransformInputEvent::ToggleCopy) => {
                let copy_allowed = self.selection.selected_group.is_none();
                self.gesture.transform.rotate_copy =
                    copy_allowed && !self.gesture.transform.rotate_copy;
                let copy_mode = self.gesture.transform.rotate_copy;
                if let Some(drag) = self.rotate_session_mut() {
                    drag.copy = drag.group_id.is_none() && copy_mode;
                }
                let (angle, axis) = self.rotate_session().map_or(
                    (
                        0.0,
                        self.gesture.transform.rotate_axis_lock.unwrap_or(Axis::Z),
                    ),
                    |(drag, _)| (drag.angle_degrees, drag.axis),
                );
                self.digest = self.catalog.format(
                    if self.gesture.transform.rotate_copy {
                        "digest-rotate-copy-live"
                    } else {
                        "digest-rotate-live"
                    },
                    &BTreeMap::from([
                        ("angle", format_angle(angle)),
                        ("axis", self.catalog.text(axis_name_key(axis))),
                    ]),
                );
            }
            (_, TransformInputEvent::CopyRequested) | (_, TransformInputEvent::ToggleCopy) => {}
        }
    }

    pub(crate) fn interpret_transform_input_for(
        &mut self,
        tool: ActiveTool,
        enabled: bool,
        command_down: bool,
        command_chord: bool,
    ) {
        if let Some(event) =
            self.transform_tool
                .input
                .interpret_command(enabled, command_down, command_chord)
        {
            self.apply_transform_input_event(tool, event);
        }
    }

    #[cfg(test)]
    pub(crate) fn update_move_copy_modifier(&mut self, modifier_down: bool) {
        self.interpret_transform_input_for(ActiveTool::Move, true, modifier_down, false);
    }

    #[cfg(test)]
    pub(crate) fn update_rotate_copy_modifier(&mut self, modifier_down: bool) {
        self.interpret_transform_input_for(ActiveTool::Rotate, true, modifier_down, false);
    }

    /// Pin the Move tool to `axis`, or release the pin when `axis` is `None`.
    pub(crate) fn set_move_axis_lock(&mut self, axis: Option<Axis>) {
        self.end_transform_correction();
        self.gesture.transform.move_axis_lock = axis;
        if let Some(drag) = self.move_session_mut() {
            drag.axis = axis;
            drag.axis_reference = None;
            drag.delta_mm = Vec3::ZERO;
        }
        self.value_box.input = "0".to_owned();
        self.digest = self.catalog.format(
            "digest-move-axis-locked",
            &BTreeMap::from([(
                "axis",
                self.catalog
                    .text(axis.map_or("axis-name-plane", axis_name_key)),
            )]),
        );
    }

    pub(crate) fn set_scale_axis_lock(&mut self, axis: Option<Axis>) {
        if matches!(
            self.transform_tool
                .correction
                .as_ref()
                .map(|session| &session.operation),
            Some(CorrectionOperation::Scale(_))
        ) {
            self.end_transform_correction();
        }
        self.gesture.transform.scale_axis_lock = axis;
        if let Some(drag) = self.scale_session_mut() {
            drag.axis = axis;
        }
        self.digest = self.catalog.format(
            "digest-scale-axis-locked",
            &BTreeMap::from([(
                "axis",
                self.catalog
                    .text(axis.map_or("axis-name-uniform", axis_name_key)),
            )]),
        );
    }

    pub(crate) fn scale_preview_is_current(&self, drag: &ScaleDrag) -> bool {
        self.scale_transform_plan(drag).is_some()
    }

    pub(crate) fn scale_preview_transform_overrides(&self) -> BTreeMap<InstancePath, Transform> {
        self.scale_session()
            .and_then(|drag| self.scale_transform_plan(drag))
            .map(|plan| plan.preview_overrides().clone())
            .unwrap_or_default()
    }

    pub(crate) fn scale_selected(&mut self, factor: f64) -> bool {
        let Some(selection) = self.selected_move_reference() else {
            self.digest = self.catalog.text("digest-scale-start-missed");
            return false;
        };
        let snapshot = self.document.current();
        let group_id = self.selection.selected_group;
        let occurrence_paths = self.selected_instance_paths();
        let applies: Box<dyn Fn(&InstancePath) -> bool> = match group_id {
            Some(group_id) => {
                let snapshot = snapshot.clone();
                Box::new(move |path: &InstancePath| {
                    Self::group_contains_occurrence(&snapshot, group_id, path.root_occurrence())
                })
            }
            None => {
                let occurrence_paths = occurrence_paths.clone();
                Box::new(move |path: &InstancePath| occurrence_paths.contains(path))
            }
        };
        let Some(centre_mm) = self.rotation_centre_for(&applies) else {
            return false;
        };
        let drag = ScaleDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            selection,
            occurrence_paths,
            group_id,
            centre_mm,
            centre_screen: Pos2::ZERO,
            reference_radius_points: 1.0,
            axis: self.gesture.transform.scale_axis_lock,
            factor,
        };
        self.commit_scale_drag(&drag)
    }

    pub(crate) fn begin_scale_drag_at(&mut self, pointer: Pos2, rect: Rect) -> bool {
        let selected = self.selected_move_reference();
        let Some(selection) = selected.clone().or_else(|| {
            self.hover
                .target
                .clone()
                .filter(|selection| self.occurrence_in_active_context(&selection.instance_path))
        }) else {
            self.digest = self.catalog.text("digest-scale-start-missed");
            return false;
        };
        if selected.is_none() {
            self.select_from_viewport(Some(selection.clone()), false);
        }
        let snapshot = self.document.current();
        let group_id = self.selection.selected_group;
        let occurrence_paths = self.selected_instance_paths();
        let applies: Box<dyn Fn(&InstancePath) -> bool> = match group_id {
            Some(group_id) => {
                let snapshot = snapshot.clone();
                Box::new(move |path: &InstancePath| {
                    Self::group_contains_occurrence(&snapshot, group_id, path.root_occurrence())
                })
            }
            None => {
                let occurrence_paths = occurrence_paths.clone();
                Box::new(move |path: &InstancePath| occurrence_paths.contains(path))
            }
        };
        let Some(centre_mm) = self.rotation_centre_for(&applies) else {
            return false;
        };
        let centre_screen = self.project(centre_mm, rect);
        let reference_radius_points = pointer.distance(centre_screen);
        if reference_radius_points < 2.0 {
            self.digest = self.catalog.text("digest-scale-start-too-close");
            return false;
        }
        self.value_box.input = "1".to_owned();
        self.set_scale_session(ScaleDrag {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            selection,
            occurrence_paths,
            group_id,
            centre_mm,
            centre_screen,
            reference_radius_points,
            axis: self.gesture.transform.scale_axis_lock,
            factor: 1.0,
        });
        true
    }

    pub(crate) fn advance_scale(drag: &mut ScaleDrag, pointer: Pos2) {
        let factor = f64::from(pointer.distance(drag.centre_screen) / drag.reference_radius_points);
        if factor.is_finite() {
            drag.factor = factor.clamp(0.01, 1_000.0);
        }
    }

    pub(crate) fn commit_scale_drag(&mut self, drag: &ScaleDrag) -> bool {
        if !self.scale_preview_is_current(drag) {
            self.digest = self.catalog.text("error-preview-stale");
            return false;
        }
        if !scale_is_meaningful(drag.factor) {
            return false;
        }
        let Some(plan) = self.scale_transform_plan(drag) else {
            return false;
        };
        let Ok(target) = self.commit_transform_plan(plan) else {
            return false;
        };
        let selection = match target {
            TransformTarget::Occurrences(paths) => {
                self.selection.select_exact(drag.selection.clone(), false);
                self.selection.occurrences = paths.clone();
                CorrectionSelection::Occurrences {
                    occurrence_ids: paths.iter().map(InstancePath::root_occurrence).collect(),
                    primary_occurrence_id: None,
                }
            }
            TransformTarget::Group(group_id) => {
                self.select_group(group_id);
                CorrectionSelection::Group(group_id)
            }
        };
        self.record_transform_correction(
            selection,
            CorrectionOperation::Scale(ScaleCorrection {
                centre_mm: drag.centre_mm,
                axis: drag.axis,
            }),
        );
        self.status_key = "status-object-scaled";
        self.digest = self.catalog.format(
            "digest-scale-committed",
            &BTreeMap::from([
                ("factor", format_scale_factor(drag.factor)),
                (
                    "axis",
                    self.catalog
                        .text(drag.axis.map_or("axis-name-uniform", axis_name_key)),
                ),
            ]),
        );
        true
    }

    #[doc(hidden)]
    #[must_use]
    pub fn scale_preview_factor(&self) -> Option<f64> {
        self.scale_session()
            .filter(|drag| self.scale_preview_is_current(drag))
            .map(|drag| drag.factor)
    }

    /// The Rotate gesture currently driving a preview, if any.
    pub(crate) fn active_rotate_gesture(&self) -> Option<&RotateDrag> {
        self.rotate_session()
            .map(|(drag, _)| drag)
            .filter(|drag| self.rotate_preview_is_current(drag))
    }

    pub(crate) fn rotate_drag_applies_to_path(
        &self,
        drag: &RotateDrag,
        instance_path: &InstancePath,
    ) -> bool {
        if let Some(group_id) = drag.group_id {
            Self::group_contains_occurrence(
                &self.document.current(),
                group_id,
                instance_path.root_occurrence(),
            )
        } else {
            drag.occurrence_paths.contains(instance_path)
        }
    }

    pub(crate) fn rotate_preview_transform_overrides(&self) -> BTreeMap<InstancePath, Transform> {
        self.active_rotate_gesture()
            .and_then(|drag| self.rotate_transform_plan(drag))
            .map(|plan| plan.preview_overrides().clone())
            .unwrap_or_default()
    }

    pub(crate) fn rotation_preview_transforms(
        &self,
        copy: bool,
    ) -> BTreeMap<InstancePath, Transform> {
        if !copy {
            return self.rotate_preview_transform_overrides();
        }
        let Some(drag) = self.active_rotate_gesture().filter(|drag| drag.copy) else {
            return BTreeMap::new();
        };
        let Ok(rotation) = world_rotation_transform(drag.centre_mm, drag.axis, drag.angle_degrees)
        else {
            return BTreeMap::new();
        };
        self.document
            .current()
            .scene_query()
            .into_iter()
            .filter(|occurrence| self.rotate_drag_applies_to_path(drag, &occurrence.instance_path))
            .map(|occurrence| {
                (
                    occurrence.instance_path,
                    rotation.compose(occurrence.transform),
                )
            })
            .collect()
    }

    /// Every occurrence transform the live Move and Rotate previews replace.
    ///
    /// Only one of the two gestures can be running at a time, so this is a
    /// plain union rather than a composition.
    pub(crate) fn preview_transform_overrides(&self) -> BTreeMap<InstancePath, Transform> {
        let mut overrides = self.assembly_preview_transform_overrides();
        overrides.extend(self.move_preview_transform_overrides());
        overrides.extend(self.rotate_preview_transform_overrides());
        overrides.extend(self.scale_preview_transform_overrides());
        overrides
    }

    /// The world point a Rotate gesture turns about: the centre of the painted
    /// bounds of everything the gesture applies to.
    pub(crate) fn rotation_centre_for(
        &self,
        applies: &dyn Fn(&InstancePath) -> bool,
    ) -> Option<Vec3> {
        let [minimum, maximum] = self.rotation_bounds_for(applies)?;
        Some((minimum + maximum) * 0.5)
    }

    /// The painted world bounds of everything a Rotate gesture applies to.
    ///
    /// The guide needs the extent as well as the centre, so the protractor is
    /// drawn around the body instead of at an arbitrary fixed radius.
    pub(crate) fn rotation_bounds_for(
        &self,
        applies: &dyn Fn(&InstancePath) -> bool,
    ) -> Option<[Vec3; 2]> {
        let snapshot = self.document.current();
        self.refresh_interaction_projection_cache(&snapshot);
        let cache = self.hover.projection_cache.borrow();
        let projection = &cache
            .as_ref()
            .expect("interaction cache was built")
            .canonical;
        let corners = projection
            .occurrences()
            .iter()
            .filter(|occurrence| occurrence.visible && applies(&occurrence.instance_path))
            .filter_map(|occurrence| {
                let [minimum, maximum] = self.definition_local_bounds(
                    &snapshot,
                    occurrence.body.definition_id,
                    occurrence.local_box,
                    true,
                )?;
                let size = maximum - minimum;
                Some(box_corners(size.x, size.y, size.z).map(|corner| {
                    transform_model_point(occurrence.canonical_world_transform, corner + minimum)
                }))
            })
            .flatten()
            .collect::<Vec<_>>();
        bounds_of(corners.into_iter())
    }

    /// Re-read the live angle in the chosen plane. Only a click establishes
    /// the starting arm; hovering after choosing a pivot never starts a turn.
    pub(crate) fn advance_rotation(
        &self,
        drag: &mut RotateDrag,
        pointer: Pos2,
        rect: Rect,
        free: bool,
    ) {
        let Some(world) = self.screen_to_rotation_plane(pointer, rect, drag.centre_mm, drag.axis)
        else {
            return;
        };
        let arm = world - drag.centre_mm;
        let Some(reference_mm) = drag.reference_mm else {
            return;
        };
        if let Some(angle) = rotation_angle_degrees(reference_mm, arm, drag.axis) {
            drag.angle_degrees = snapped_rotation_degrees(angle, free);
        }
    }

    pub(crate) fn begin_rotate_drag_at(&mut self, pointer: Pos2, rect: Rect, copy: bool) -> bool {
        let selected = self.selected_move_reference();
        let Some(selection) = selected.clone().or_else(|| {
            self.hover
                .snap
                .as_ref()
                .map(|snap| snap.reference.clone())
                .or_else(|| self.hover.target.clone())
                .filter(|selection| self.occurrence_in_active_context(&selection.instance_path))
        }) else {
            self.digest = self.catalog.text("digest-rotate-start-missed");
            return false;
        };
        if selected.is_none() {
            self.select_from_viewport(Some(selection.clone()), false);
        }
        let snapshot = self.document.current();
        let group_id = self.selection.selected_group;
        let occurrence_paths = self.selected_instance_paths();
        let axis = self.gesture.transform.rotate_axis_lock.unwrap_or(Axis::Z);
        let applies: Box<dyn Fn(&InstancePath) -> bool> = match group_id {
            Some(group_id) => {
                let snapshot = snapshot.clone();
                Box::new(move |path: &InstancePath| {
                    Self::group_contains_occurrence(&snapshot, group_id, path.root_occurrence())
                })
            }
            None => {
                let occurrence_paths = occurrence_paths.clone();
                Box::new(move |path: &InstancePath| occurrence_paths.contains(path))
            }
        };
        let centre_mm = self
            .scene_snap_at_screen(pointer, rect, 8.0, None)
            .map(|snap| snap.position_mm)
            .or_else(|| {
                self.datum_snap_at_screen(pointer, rect, None)
                    .map(|(point, _)| point)
            })
            .or_else(|| self.surface_point_at_screen(pointer, rect))
            .or_else(|| {
                self.screen_to_rotation_plane(
                    pointer,
                    rect,
                    self.rotation_centre_for(&applies)?,
                    axis,
                )
            });
        let Some(centre_mm) = centre_mm else {
            return false;
        };
        let reference_mm = None;
        self.value_box.input = "0".to_owned();
        self.set_rotate_session(
            ToolSessionPhase::Gesture,
            RotateDrag {
                source_document_id: snapshot.document_id(),
                source_revision: snapshot.revision_id(),
                selection,
                occurrence_paths,
                group_id,
                centre_mm,
                axis,
                reference_mm,
                angle_degrees: 0.0,
                copy: group_id.is_none() && (copy || self.gesture.transform.rotate_copy),
            },
        );
        true
    }

    /// Pin the Rotate tool to `axis`, or release the pin when `axis` is `None`.
    ///
    /// A live gesture keeps its centre but drops its starting arm, so the next
    /// click establishes one in the new plane without a hover-induced turn.
    pub(crate) fn set_rotate_axis_lock(&mut self, axis: Option<Axis>) {
        if matches!(
            self.transform_tool
                .correction
                .as_ref()
                .map(|session| &session.operation),
            Some(CorrectionOperation::Rotate(_))
        ) {
            self.end_transform_correction();
        }
        self.gesture.transform.rotate_axis_lock = axis;
        let resolved = axis.unwrap_or(Axis::Z);
        if let Some(drag) = self.rotate_session_mut() {
            drag.axis = resolved;
            drag.reference_mm = None;
            drag.angle_degrees = 0.0;
        }
        self.value_box.input = "0".to_owned();
        self.digest = self.catalog.format(
            "digest-rotate-axis-locked",
            &BTreeMap::from([(
                "axis",
                self.catalog.text(match axis {
                    Some(Axis::X) => "axis-name-x",
                    Some(Axis::Y) => "axis-name-y",
                    Some(Axis::Z) => "axis-name-z",
                    None => "axis-name-free",
                }),
            )]),
        );
    }

    /// What the Rotate guide should draw this frame, if anything.
    ///
    /// The guide appears as soon as the tool is armed on a selection, not only
    /// once a gesture is in flight: the whole point is to show which body will
    /// turn and about which coloured axis before anything moves.
    pub(crate) fn rotation_guide(&self) -> Option<RotationGuide> {
        if self.active_tool != ActiveTool::Rotate {
            return None;
        }
        if let Some(drag) = self.active_rotate_gesture() {
            let bounds = self.rotation_bounds_for(&|path: &InstancePath| {
                self.rotate_drag_applies_to_path(drag, path)
            })?;
            return Some(RotationGuide {
                centre_mm: drag.centre_mm,
                axis: drag.axis,
                radius_mm: rotation_guide_radius(bounds, drag.axis),
                start_degrees: drag
                    .reference_mm
                    .and_then(|arm| plane_angle_degrees(arm, drag.axis)),
                angle_degrees: drag.angle_degrees,
            });
        }
        let axis = self.gesture.transform.rotate_axis_lock.unwrap_or(Axis::Z);
        let bounds = if let Some(group_id) = self.selection.selected_group {
            let snapshot = self.document.current();
            self.rotation_bounds_for(&|path: &InstancePath| {
                Self::group_contains_occurrence(&snapshot, group_id, path.root_occurrence())
            })?
        } else {
            let target = self.selected_move_reference()?.instance_path;
            self.rotation_bounds_for(&|path: &InstancePath| *path == target)?
        };
        Some(RotationGuide {
            centre_mm: (bounds[0] + bounds[1]) * 0.5,
            axis,
            radius_mm: rotation_guide_radius(bounds, axis),
            start_degrees: None,
            angle_degrees: 0.0,
        })
    }

    pub(crate) fn prioritize_push_pull_profile_pick(&self, pick: &mut PickResult) {
        if self.active_tool != ActiveTool::PushPull {
            return;
        }
        let snapshot = self.document.current();
        let profiles = self
            .active_boxes()
            .into_iter()
            .filter(|item| {
                item.extrusion_feature_id.is_none()
                    && snapshot.feature(item.profile_feature_id).is_some_and(
                        |feature| match feature.kind() {
                            FeatureKind::Profile { closed, .. } => *closed,
                            FeatureKind::Sketch(_) => true,
                            _ => false,
                        },
                    )
            })
            .map(|item| item.instance_path)
            .collect::<BTreeSet<_>>();
        let Some(index) = pick.overlapping.iter().position(|hit| {
            matches!(hit.reference.element, ElementId::Face { .. })
                && profiles.contains(&hit.reference.instance_path)
        }) else {
            return;
        };
        pick.overlapping.rotate_left(index);
        pick.primary = pick.overlapping[0].clone();
        if pick.snap.kind == SnapKind::Face {
            pick.snap.reference = pick.primary.reference.clone();
            pick.snap.position_mm = pick.primary.position_mm;
        }
    }
}

pub(crate) fn axis_vector(axis: Axis, value: f64) -> Vec3 {
    match axis {
        Axis::X => Vec3::new(value, 0.0, 0.0),
        Axis::Y => Vec3::new(0.0, value, 0.0),
        Axis::Z => Vec3::new(0.0, 0.0, value),
    }
}

pub(crate) const fn alignment_axis_label(axis: Axis) -> &'static str {
    match axis {
        Axis::X => "X",
        Axis::Y => "Y",
        Axis::Z => "Z",
    }
}

/// How far along the line through `anchor` in direction `axis` the pointer has
/// reached, measured at the point where that line passes closest to the view
/// ray.
///
/// Intersecting a plane is not enough here: the blue axis is vertical, and the
/// vertical plane that would carry it is not defined by the axis alone. Taking
/// the closest approach instead works for all three axes and only fails when the
/// axis points almost straight at the eye, where the reading would be noise
/// anyway.
pub(crate) fn axis_travel_along(ray: &Ray, anchor: Vec3, axis: Axis) -> Option<f64> {
    let unit = axis_direction(axis);
    let separation = anchor - ray.origin;
    let ray_length_squared = dot(ray.direction, ray.direction);
    let alignment = dot(unit, ray.direction);
    // `unit` is a unit vector, so its own square length is exactly one.
    let denominator = ray_length_squared - alignment * alignment;
    if denominator.abs() <= ROUNDING {
        return None;
    }
    let travel = alignment.mul_add(
        dot(ray.direction, separation),
        -ray_length_squared * dot(unit, separation),
    ) / denominator;
    travel.is_finite().then_some(travel)
}

pub(crate) fn world_rotation_transform(
    centre_mm: Vec3,
    axis: Axis,
    angle_degrees: f64,
) -> Result<Transform, CanonicalError> {
    world_axis_rotation_transform(centre_mm, axis_direction(axis), angle_degrees)
}

fn world_scale_transform(
    centre_mm: Vec3,
    factor: f64,
    axis: Option<Axis>,
) -> Result<Transform, CanonicalError> {
    if !factor.is_finite() || factor <= 0.0 || factor > 1_000.0 {
        return Err(CanonicalError::InvalidTransform);
    }
    let [scale_x, scale_y, scale_z] = match axis {
        Some(Axis::X) => [factor, 1.0, 1.0],
        Some(Axis::Y) => [1.0, factor, 1.0],
        Some(Axis::Z) => [1.0, 1.0, factor],
        None => [factor; 3],
    };
    Transform::from_matrix([
        scale_x,
        0.0,
        0.0,
        centre_mm.x * (1.0 - scale_x),
        0.0,
        scale_y,
        0.0,
        centre_mm.y * (1.0 - scale_y),
        0.0,
        0.0,
        scale_z,
        centre_mm.z * (1.0 - scale_z),
        0.0,
        0.0,
        0.0,
        1.0,
    ])
}

pub(crate) fn scale_is_meaningful(factor: f64) -> bool {
    // not a tolerance: a smaller change is below the input resolution.
    factor.is_finite() && factor > 0.0 && factor <= 1_000.0 && (factor - 1.0).abs() >= 1.0e-4
}

pub(crate) fn format_scale_factor(factor: f64) -> String {
    format!("{factor:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

/// The turn from `reference` to `current` about `axis`, both measured from the
/// rotation centre, in degrees within (-180, 180].
fn rotation_angle_degrees(reference: Vec3, current: Vec3, axis: Axis) -> Option<f64> {
    let unit = axis_direction(axis);
    let flatten = |arm: Vec3| arm - unit * dot(arm, unit);
    let (from, to) = (flatten(reference), flatten(current));
    if length(from) < ROTATION_MIN_ARM_MM || length(to) < ROTATION_MIN_ARM_MM {
        return None;
    }
    let angle = dot(cross(from, to), unit).atan2(dot(from, to)).to_degrees();
    angle.is_finite().then_some(angle)
}

/// Whether an angle turns the body far enough to be worth a canonical revision.
pub(crate) fn rotation_is_meaningful(angle_degrees: f64) -> bool {
    angle_degrees.is_finite() && angle_degrees.abs() >= 0.01 && angle_degrees.abs() <= 360.0
}

/// Where an arm points inside the rotation plane, in the same degrees the
/// gesture reports.
fn plane_angle_degrees(arm_mm: Vec3, axis: Axis) -> Option<f64> {
    let (first, second) = axis_plane_frame(axis);
    let (along, across) = (dot(arm_mm, first), dot(arm_mm, second));
    (along.hypot(across) >= ROTATION_MIN_ARM_MM).then(|| across.atan2(along).to_degrees())
}

/// A protractor radius that clears the body it turns, taken from the extent in
/// the rotation plane rather than from a fixed screen size.
fn rotation_guide_radius(bounds: [Vec3; 2], axis: Axis) -> f64 {
    let size = bounds[1] - bounds[0];
    let (first, second) = match axis {
        Axis::X => (size.y, size.z),
        Axis::Y => (size.z, size.x),
        Axis::Z => (size.x, size.y),
    };
    (first.max(second) * 0.62).max(1.0)
}

pub(crate) const fn axis_name_key(axis: Axis) -> &'static str {
    match axis {
        Axis::X => "axis-name-x",
        Axis::Y => "axis-name-y",
        Axis::Z => "axis-name-z",
    }
}

pub(crate) fn format_angle(angle_degrees: f64) -> String {
    let rounded = (angle_degrees * 10.0).round() / 10.0;
    if (rounded - rounded.round()).abs() < f64::EPSILON {
        format!("{}", rounded.round() as i64)
    } else {
        format!("{rounded:.1}")
    }
}

fn snapped_rotation_degrees(angle_degrees: f64, free: bool) -> f64 {
    if free {
        (angle_degrees * 10.0).round() / 10.0
    } else {
        (angle_degrees / ROTATION_SNAP_DEGREES).round() * ROTATION_SNAP_DEGREES
    }
}

fn rotate_transform_90(
    transform: Transform,
    local_box: ProjectedBox,
) -> Result<Transform, CanonicalError> {
    let center_x = local_box.origin_mm.x + local_box.size_mm.x * 0.5;
    let center_y = local_box.origin_mm.y + local_box.size_mm.y * 0.5;
    let local_rotation = Transform::from_matrix([
        0.0,
        -1.0,
        0.0,
        center_x + center_y,
        1.0,
        0.0,
        0.0,
        center_y - center_x,
        0.0,
        0.0,
        1.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ])?;
    Ok(transform.compose(local_rotation))
}

/// A length the tool computed (`120.1 + 13.2`) as the decimal a person would
/// write (`133.3`): the shortest token of at most nine decimals that differs
/// from it only by float noise (relative `NEGLIGIBLE`), with that value, so the
/// noise never reaches the history or the document while a snapped
/// `26.876543211` stays exact.
pub(crate) fn computed_length(mm: f64) -> Option<Dimension> {
    let noise = ketchup_model::tolerance::NEGLIGIBLE * mm.abs().max(1.0);
    (0..=9)
        .find_map(|places| {
            let token = format!("{mm:.places$}");
            let value = token.parse::<f64>().ok()?;
            ((value - mm).abs() <= noise).then_some((token, value))
        })
        .and_then(|(token, value)| Dimension::new(token, value).ok())
        .or_else(|| Dimension::new(mm.to_string(), mm).ok())
}

pub(crate) fn push_pull_batch(
    snapshot: &Snapshot,
    selection: &SelectionId,
    item: &RenderBox,
    topological_reference: Option<&TopologicalElementRef>,
    distance_mm: f64,
    new_extent_mm: f64,
    source_token: String,
) -> Option<CommandBatch> {
    let resolved = snapshot
        .resolve_instance_path(&selection.instance_path)
        .ok()?;
    if resolved.definition_id != selection.definition_id
        || item.definition_id != selection.definition_id
        || item.instance_path != selection.instance_path
    {
        return None;
    }
    if let Some(reference) = topological_reference {
        let producer = snapshot.feature(reference.producer_feature_id)?;
        if producer.definition_id() == selection.definition_id
            && producer.kind().produces_body()
            && snapshot
                .definition(selection.definition_id)?
                .feature_ids()
                .last()
                == Some(&reference.producer_feature_id)
            && reference.definition_id == selection.definition_id
            && reference.kind == TopologicalElementKind::Face
        {
            // Pulling the end cap of an extrusion lengthens the extrusion: one
            // smooth body instead of a second prism fused on with a seam.
            if let FeatureKind::Pad(PadSpec {
                direction: FeatureDirection::AlongNormal | FeatureDirection::OppositeNormal,
                extent: FeatureExtent::Blind(extent),
                operation: PadOperation::NewBody,
                ..
            }) = producer.kind()
                && reference.producer_element_id == ExactFaceRole::Top.semantic_role()
            {
                let length = computed_length(extent.millimetres() + distance_mm)?;
                if length.millimetres() <= 0.01 {
                    return None;
                }
                return Some(CommandBatch::new(vec![
                    CanonicalCommand::SetFeatureDimension {
                        id: reference.producer_feature_id,
                        dimension: length,
                    },
                ]));
            }
            let id = FeatureId(
                snapshot
                    .features()
                    .map(|feature| feature.id().0)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)?,
            );
            return Some(CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id,
                definition_id: selection.definition_id,
                name: "Face Offset".to_owned(),
                kind: FeatureKind::FaceOffset {
                    target: reference.producer_feature_id,
                    face: FaceRef::from(reference.clone()),
                    distance: Dimension::new(distance_mm.to_string(), distance_mm).ok()?,
                },
            }]));
        }
    }
    let profile = snapshot.feature(item.profile_feature_id)?;
    if profile.definition_id() != selection.definition_id {
        return None;
    }
    if let Some(extrusion_id) = item.extrusion_feature_id {
        let extrusion = snapshot.feature(extrusion_id)?;
        if extrusion.definition_id() != selection.definition_id
            || !matches!(
                extrusion.kind(),
                FeatureKind::Pad(PadSpec { profile: PadProfile::Feature(profile), extent: FeatureExtent::Blind(_), operation: PadOperation::NewBody, .. }) if *profile == item.profile_feature_id
            )
        {
            return None;
        }
    }
    let ElementId::Face { axis, side } = selection.element else {
        return None;
    };
    if let FeatureKind::Sketch(sketch) = profile.kind() {
        if item.extrusion_feature_id.is_some() || new_extent_mm <= 0.01 {
            return None;
        }
        let workplane = snapshot.feature(sketch.workplane)?;
        if workplane.definition_id() != selection.definition_id {
            return None;
        }
        let FeatureKind::Workplane(workplane) = workplane.kind() else {
            return None;
        };
        let expected_face = face_element_from_normal(Vec3::new(
            workplane.frame.normal[0],
            workplane.frame.normal[1],
            workplane.frame.normal[2],
        ));
        if selection.element != expected_face {
            return None;
        }
        let [region] = sketch.solved_regions().ok()?.try_into().ok()?;
        if !region.holes.is_empty() {
            return None;
        }
        let id = FeatureId(
            snapshot
                .features()
                .map(|feature| feature.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        return Some(CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id,
            definition_id: selection.definition_id,
            name: "Pad".to_owned(),
            kind: FeatureKind::Pad(PadSpec {
                profile: PadProfile::SketchRegion {
                    sketch: item.profile_feature_id,
                    region: region.id,
                },
                direction: FeatureDirection::AlongNormal,
                extent: FeatureExtent::Blind(Dimension::new(source_token, new_extent_mm).ok()?),
                operation: PadOperation::NewBody,
            }),
        }]));
    }
    let mut commands = Vec::new();
    match axis {
        Axis::Z => {
            let dimension = Dimension::new(source_token, new_extent_mm).ok()?;
            if let Some(extrusion_id) = item.extrusion_feature_id {
                commands.push(CanonicalCommand::SetFeatureDimension {
                    id: extrusion_id,
                    dimension,
                });
            } else {
                let extrusion_id = FeatureId(
                    snapshot
                        .features()
                        .map(|feature| feature.id().0)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)?,
                );
                commands.push(CanonicalCommand::CreateFeature {
                    id: extrusion_id,
                    definition_id: selection.definition_id,
                    name: "Extrusion".to_owned(),
                    kind: FeatureKind::extrusion(item.profile_feature_id, dimension),
                });
            }
        }
        Axis::X | Axis::Y => {
            let points_mm = profile.kind().polygon_points()?;
            let index = usize::from(matches!(axis, Axis::Y));
            let coordinate = |point: &[f64; 2]| point[index];
            let minimum = points_mm.iter().map(coordinate).min_by(f64::total_cmp)?;
            let maximum = points_mm.iter().map(coordinate).max_by(f64::total_cmp)?;
            let old_extent = maximum - minimum;
            let mut resized = points_mm.clone();
            for point in &mut resized {
                let normalized = (coordinate(point) - minimum) / old_extent;
                let value = minimum + normalized * new_extent_mm;
                point[index] = value;
            }
            commands.push(CanonicalCommand::SetProfilePoints {
                id: item.profile_feature_id,
                points_mm: resized,
            });
        }
    }
    if side == Side::Minimum {
        if !selection.instance_path.is_root() {
            return None;
        }
        let occurrence_id = selection.instance_path.root_occurrence();
        let occurrence = snapshot.occurrence(occurrence_id)?;
        let delta = match axis {
            Axis::X => Vec3::new(item.size_mm.x - new_extent_mm, 0.0, 0.0),
            Axis::Y => Vec3::new(0.0, item.size_mm.y - new_extent_mm, 0.0),
            Axis::Z => Vec3::new(0.0, 0.0, item.size_mm.z - new_extent_mm),
        };
        commands.push(CanonicalCommand::SetOccurrenceTransform {
            id: occurrence_id,
            transform: translated_transform(occurrence.transform(), delta).ok()?,
        });
    }
    Some(CommandBatch::new(commands))
}

fn inverse_transform_point(transform: Transform, point: Vec3) -> Option<Vec3> {
    let inverse = transform.affine().invert().ok()?;
    Some(inverse.transform_point(point))
}

pub(crate) fn continuous_move_delta(start: Vec3, end: Vec3, constrain_axis: bool) -> Vec3 {
    let mut delta = Vec3::new(end.x - start.x, end.y - start.y, 0.0);
    if constrain_axis {
        if delta.x.abs() >= delta.y.abs() {
            delta.y = 0.0;
        } else {
            delta.x = 0.0;
        }
    }
    delta
}

pub(crate) fn push_pull_distance_from_pointer(
    drag: &PushPullDrag,
    pointer: Pos2,
    snaps_enabled: bool,
) -> f64 {
    let pointer_delta = pointer - drag.pointer_start;
    let raw_distance =
        f64::from(pointer_delta.dot(drag.screen_normal)) / f64::from(drag.pixels_per_mm);
    let distance = if snaps_enabled {
        let grid_distance = (raw_distance / GRID_STEP_MM).round() * GRID_STEP_MM;
        let tolerance_mm = (PRECISE_SNAP_SCREEN_TOLERANCE_PX / f64::from(drag.pixels_per_mm))
            .min(PRECISE_SNAP_WORLD_TOLERANCE_MM);
        if (grid_distance - raw_distance).abs() <= tolerance_mm {
            grid_distance
        } else {
            raw_distance
        }
    } else {
        raw_distance
    };
    if !matches!(drag.selection.element, ElementId::TopologicalFace(_))
        && drag.extent_start_mm > 0.01
    {
        distance.max(-drag.extent_start_mm + 0.01)
    } else {
        distance
    }
}

/// How many straight pieces stand in for one curved profile segment where the
/// window needs its outline as points: snapping to it and measuring from it.
pub(crate) const PROFILE_CURVE_STEPS: usize = 64;

/// The points one profile segment runs through, `steps` pieces per curve; `None` for a
/// spline, whose course between its points only the exact kernel knows.
pub(crate) fn profile_segment_polyline(
    segment: &ProfileSegment,
    steps: usize,
) -> Option<Vec<[f64; 2]>> {
    Some(match segment {
        ProfileSegment::Line { start_mm, end_mm } => vec![*start_mm, *end_mm],
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => profile_arc_polyline(*start_mm, *end_mm, *center_mm, *clockwise, steps),
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => {
            let curve = CubicBezier::new([*start_mm, *control_1_mm, *control_2_mm, *end_mm]);
            (0..=steps)
                .map(|step| curve.eval(step as f64 / steps as f64))
                .collect()
        }
        ProfileSegment::Spline { .. } => return None,
    })
}

pub(crate) fn profile_arc_polyline(
    start_mm: [f64; 2],
    end_mm: [f64; 2],
    center_mm: [f64; 2],
    clockwise: bool,
    segments: usize,
) -> Vec<[f64; 2]> {
    let start_angle = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
    let end_angle = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
    let mut sweep = end_angle - start_angle;
    if clockwise {
        while sweep >= 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else {
        while sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
    }
    let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
    (0..=segments)
        .map(|step| {
            let angle = start_angle + sweep * step as f64 / segments as f64;
            [
                center_mm[0] + radius * angle.cos(),
                center_mm[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

pub(crate) fn face_extent(item: &RenderBox, element: Option<&ElementId>) -> Option<f64> {
    match element? {
        ElementId::Face { axis: Axis::X, .. } => Some(item.size_mm.x),
        ElementId::Face { axis: Axis::Y, .. } => Some(item.size_mm.y),
        ElementId::Face { axis: Axis::Z, .. } => Some(item.size_mm.z),
        _ => None,
    }
}

pub(crate) fn resize_box_from_face(
    item: &RenderBox,
    element: &ElementId,
    new_extent_mm: f64,
) -> Option<RenderBox> {
    let mut item = item.clone();
    if !new_extent_mm.is_finite() || new_extent_mm <= 0.01 {
        return None;
    }
    let ElementId::Face { axis, side } = element else {
        return None;
    };
    let (origin, extent) = match axis {
        Axis::X => (&mut item.origin_mm.x, &mut item.size_mm.x),
        Axis::Y => (&mut item.origin_mm.y, &mut item.size_mm.y),
        Axis::Z => (&mut item.origin_mm.z, &mut item.size_mm.z),
    };
    if *side == Side::Minimum {
        *origin += *extent - new_extent_mm;
    }
    *extent = new_extent_mm;
    Some(item)
}
