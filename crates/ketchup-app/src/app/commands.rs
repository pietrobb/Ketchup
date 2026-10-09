//! Command dispatch, command search, value input, parameter editor and validator panel.

use crate::*;

impl KetchupApp {
    #[must_use]
    pub const fn parameter_last_recomputed_nodes(&self) -> &BTreeSet<NodeId> {
        &self.parameter.last_recomputed_nodes
    }

    /// The visible label of `command` in the active locale.
    #[must_use]
    pub fn command_label(&self, command: AppCommand) -> String {
        self.catalog.text(CommandRegistry::spec(command).label_key)
    }

    /// Whether the command can mutate the current selection/state.
    #[must_use]
    pub fn command_is_enabled(&self, command: AppCommand) -> bool {
        self.command_enabled(command)
    }

    pub(crate) fn command_enabled(&self, id: AppCommand) -> bool {
        let spec = CommandRegistry::spec(id);
        let snapshot = self.document.current();
        spec.implemented
            && match id {
                AppCommand::Undo => self.can_undo() || self.ephemeral_edit_active(),
                AppCommand::Redo => self.can_redo() || self.ephemeral_edit_active(),
                AppCommand::Copy => self.copy_source_plan().is_some(),
                AppCommand::Cut => self.cut_source_plan().is_some(),
                AppCommand::Paste => self.paste_source_plan().is_some(),
                AppCommand::Duplicate => self.duplicate_source_plan().is_some(),
                AppCommand::Delete => self.delete_selection_source_plan().is_some(),
                AppCommand::Deselect => self.deselect_source_plan().is_some(),
                AppCommand::SelectAll => self.select_all_source_plan().is_some(),
                AppCommand::InvertSelection => self.invert_selection_source_plan().is_some(),
                AppCommand::PlanarOffset => self.selected_planar_offset_profile().is_some(),
                AppCommand::Sweep => self.sweep_preview_candidate().is_some(),
                AppCommand::Loft => self.loft_preview_candidate().is_some(),
                AppCommand::Revolve => self.selected_revolve_profile().is_some(),
                AppCommand::Shell => self
                    .selected_general_finish_target(TopologicalElementKind::Face)
                    .is_some(),
                AppCommand::Fillet | AppCommand::Chamfer => self
                    .selected_general_finish_target(TopologicalElementKind::Edge)
                    .is_some(),
                AppCommand::SolidSubtract
                | AppCommand::SolidTrim
                | AppCommand::SolidUnion
                | AppCommand::SolidIntersect
                | AppCommand::SolidSplit => {
                    let candidates = self
                        .active_boxes()
                        .into_iter()
                        .filter(|item| item.instance_path.is_root())
                        .collect::<Vec<_>>();
                    let mut exact_features = BTreeMap::new();
                    candidates
                        .iter()
                        .filter(|item| {
                            let Some(feature_id) =
                                *exact_features.entry(item.definition_id).or_insert_with(|| {
                                    exact_solid_tool_feature_id(&snapshot, item.definition_id)
                                })
                            else {
                                return false;
                            };
                            let Some(feature) = snapshot.feature(feature_id) else {
                                return false;
                            };
                            let has_current_geometry = matches!(
                                feature.kind(),
                                FeatureKind::Pad(PadSpec {
                                    profile: PadProfile::Feature(_),
                                    extent: FeatureExtent::Blind(_),
                                    operation: PadOperation::NewBody,
                                    ..
                                }) | FeatureKind::ImportedExactBody(_)
                            ) || self
                                .exact
                                .results
                                .get_render(&snapshot, item.definition_id)
                                .is_some();
                            has_current_geometry
                                && snapshot
                                    .world_transform_for_occurrence(
                                        item.instance_path.root_occurrence(),
                                    )
                                    .and_then(Transform::rigid_inverse)
                                    .is_some()
                        })
                        .count()
                        >= 2
                }
                AppCommand::Mirror => self.mirror_sources().is_some(),
                AppCommand::Group => self.group_selection_source_plan().is_some(),
                AppCommand::Ungroup => self.ungroup_selection_source_plan().is_some(),
                AppCommand::MakeComponent => self.make_component_source_plan().is_some(),
                AppCommand::MakeUnique => self.make_unique_source_plan().is_some(),
                AppCommand::ConvertSelectedMeshToExact => {
                    self.selected_mesh_feature_id().is_some() && !self.mesh_conversion_active()
                }
                AppCommand::ReplaceComponent => self.component_replacement_source_plan().is_some(),
                AppCommand::SelectAllInstances => self.select_all_instances_source_plan().is_some(),
                AppCommand::AssignTag => self.tag_assignment_source_plan().is_some(),
                AppCommand::AlignOccurrences => {
                    self.occurrence_alignment_source_plan().is_some()
                        || (self.selection.selected_group.is_none()
                            && self.selected_instance_paths().len() == 2
                            && self.selected_root_occurrence_ids().is_err())
                }
                AppCommand::DistributeOccurrences => {
                    self.occurrence_distribution_source_plan().is_some()
                }
                AppCommand::LinearPattern => self.linear_pattern_source_plan().is_some(),
                AppCommand::RectangularPattern => self.rectangular_pattern_source_plan().is_some(),
                AppCommand::CircularPattern => self.circular_pattern_source_plan().is_some(),
                AppCommand::GroundOccurrence => {
                    self.grounded_occurrence_source_plan(true).is_some()
                }
                AppCommand::UngroundOccurrence => {
                    self.grounded_occurrence_source_plan(false).is_some()
                }
                AppCommand::RenameOccurrence => self.occurrence_rename_source_plan().is_some(),
                AppCommand::RenameDefinition => self.definition_rename_source_plan().is_some(),
                AppCommand::PurgeUnused => self.purge_unused_source_plan().is_some(),
                AppCommand::Hide => self.selection_visibility_source_plan(false).is_some(),
                AppCommand::HideOthers => self.hide_others_source_plan().is_some(),
                AppCommand::Unhide => self.selection_visibility_source_plan(true).is_some(),
                AppCommand::UnhideAll => self.unhide_all_source_plan().is_some(),
                AppCommand::View(ViewFlag::HiddenObjects) => {
                    self.view.contains(ViewFlag::HiddenObjects)
                        || self.hidden_occurrence_count() > 0
                }
                AppCommand::ViewShaded => {
                    self.view.contains(ViewFlag::Wireframe)
                        || self.view.contains(ViewFlag::Monochrome)
                        || self.view.contains(ViewFlag::HiddenLine)
                }
                AppCommand::PreviousView => self.camera.previous_view.is_some(),
                AppCommand::ZoomSelection | AppCommand::CenterSelection => {
                    !self.selected_active_boxes().is_empty()
                }
                AppCommand::ZoomIn => self.camera.zoom < MAX_CAMERA_ZOOM,
                AppCommand::ZoomOut => self.camera.zoom > MIN_CAMERA_ZOOM,
                _ => true,
            }
    }

    pub(crate) fn dispatch_command(&mut self, id: AppCommand) {
        if !self.command_enabled(id) {
            return;
        }
        let spec = CommandRegistry::spec(id);
        if let Some(tool) = spec.tool {
            let tool_changed = self.active_tool != tool;
            let retained_move_axis = (tool == ActiveTool::Move && self.active_tool == tool)
                .then_some(self.gesture.transform.move_axis_lock);
            let retained_rotate_axis = (tool == ActiveTool::Rotate && self.active_tool == tool)
                .then_some(self.gesture.transform.rotate_axis_lock);
            let retained_scale_axis = (tool == ActiveTool::Scale && self.active_tool == tool)
                .then_some(self.gesture.transform.scale_axis_lock);
            self.clear_ephemeral_edit_state();
            if tool_changed {
                self.end_transform_correction();
            }
            if let Some(axis) = retained_move_axis {
                self.gesture.transform.move_axis_lock = axis;
            }
            if let Some(axis) = retained_rotate_axis {
                self.gesture.transform.rotate_axis_lock = axis;
            }
            if let Some(axis) = retained_scale_axis {
                self.gesture.transform.scale_axis_lock = axis;
            }
            self.cancel_rectangle_sketch();
            self.active_tool = tool;
            if tool == ActiveTool::PushPull {
                // A profile just drawn is pushed next, even when the pointer still
                // rests on the face it was drawn on.
                let revision = self.document.current().revision_id();
                let drawn = self
                    .hover
                    .drawn_profile
                    .clone()
                    .filter(|(drawn_revision, selection)| {
                        *drawn_revision == revision
                            && self.selection.primary.as_ref() == Some(selection)
                    })
                    .map(|(_, selection)| selection);
                let target = drawn
                    .or_else(|| self.hover.target.clone())
                    .filter(|selection| {
                        matches!(
                            selection.element,
                            ElementId::Face { .. } | ElementId::TopologicalFace(_)
                        )
                    })
                    .or_else(|| {
                        self.selection.primary.clone().filter(|selection| {
                            matches!(
                                selection.element,
                                ElementId::Face { .. } | ElementId::TopologicalFace(_)
                            )
                        })
                    });
                if let Some(target) = target {
                    self.select_push_pull_reference(target);
                }
            }
            self.value_box.input.clear();
            if tool == ActiveTool::PlanarOffset {
                self.refresh_planar_offset_preview();
            } else if tool == ActiveTool::Helix {
                self.begin_helix_tool();
            } else if tool == ActiveTool::Sweep {
                self.refresh_sweep_preview();
            } else if tool == ActiveTool::Loft {
                self.refresh_loft_preview();
            } else if tool == ActiveTool::Revolve {
                self.begin_revolve_tool();
            } else if matches!(
                tool,
                ActiveTool::Shell | ActiveTool::Fillet | ActiveTool::Chamfer
            ) {
                self.value_box.input = if tool == ActiveTool::Shell {
                    "2".to_owned()
                } else {
                    "1".to_owned()
                };
                self.refresh_general_finish_preview();
            } else if matches!(
                tool,
                ActiveTool::Line
                    | ActiveTool::Rectangle
                    | ActiveTool::Circle
                    | ActiveTool::Arc
                    | ActiveTool::Polygon
                    | ActiveTool::Ellipse
                    | ActiveTool::Spline
            ) {
                self.gesture.sketch.armed = true;
                self.status_key = match tool {
                    ActiveTool::Line => "status-line-start",
                    ActiveTool::Circle => "status-circle-center",
                    ActiveTool::Arc => "status-arc-start",
                    ActiveTool::Polygon => "status-polygon-center",
                    ActiveTool::Ellipse => "status-ellipse-center",
                    ActiveTool::Spline => "status-spline-start",
                    _ => "status-sketch-first-point",
                };
                if tool == ActiveTool::Polygon {
                    self.value_box.input = self.gesture.sketch.polygon_sides().to_string();
                }
            } else if tool == ActiveTool::Mirror {
                self.status_key = "status-mirror-face";
            } else if tool == ActiveTool::Measure {
                self.status_key = "status-measure-first-point";
            } else if matches!(
                tool,
                ActiveTool::SolidSubtract
                    | ActiveTool::SolidTrim
                    | ActiveTool::SolidUnion
                    | ActiveTool::SolidIntersect
                    | ActiveTool::SolidSplit
            ) {
                self.selection.clear();
                self.status_key = "status-solid-tool-target";
            }
            self.digest = self.catalog.format(
                "digest-tool-active",
                &BTreeMap::from([("tool", self.catalog.text(tool.label_key()))]),
            );
            return;
        }
        let camera_before = matches!(
            id,
            AppCommand::HomeView
                | AppCommand::ViewIso
                | AppCommand::ViewTop
                | AppCommand::ViewBottom
                | AppCommand::ViewFront
                | AppCommand::ViewBack
                | AppCommand::ViewRight
                | AppCommand::ViewLeft
                | AppCommand::View(_)
                | AppCommand::ViewShaded
                | AppCommand::ViewProjection
                | AppCommand::ZoomFit
                | AppCommand::ZoomSelection
                | AppCommand::CenterSelection
                | AppCommand::ZoomIn
                | AppCommand::ZoomOut
        )
        .then(|| self.camera_view_state());
        match id {
            AppCommand::New
            | AppCommand::Open
            | AppCommand::Save
            | AppCommand::SaveAs
            | AppCommand::ImportMeshStl
            | AppCommand::ImportDrawingDxf
            | AppCommand::ImportExactStep
            | AppCommand::ImportExactIges
            | AppCommand::ImportSketchupScene
            | AppCommand::ImportBlenderGlb
            | AppCommand::ExportDrawingDxf
            | AppCommand::ExportExactStep
            | AppCommand::ExportExactIges
            | AppCommand::ExportMeshStl
            | AppCommand::ExportPrintThreeMf
            | AppCommand::ExportBlenderGlb
            | AppCommand::ExportGeneralFabrication
            | AppCommand::ExportWeldmentCutList
            | AppCommand::ExportProjectDrawings
            | AppCommand::ExportSheetMetalManufacturing
            | AppCommand::ExportHomagMpr
            | AppCommand::ReviewCamExport
            | AppCommand::ReviewStaticFea
            | AppCommand::ReviewLocalPdm
            | AppCommand::ExportHundeggerBtlx => {
                self.dispatch_file_command(id);
            }
            AppCommand::Undo => {
                if self.undo() {
                    self.digest = self.catalog.text("digest-undo");
                }
            }
            AppCommand::Redo => {
                if self.redo() {
                    self.digest = self.catalog.text("digest-redo");
                }
            }
            AppCommand::Copy => {
                self.clipboard.announce |= self.copy_selection_to_clipboard();
            }
            AppCommand::Cut => {
                self.clipboard.announce |= self.cut_selection_to_clipboard();
            }
            AppCommand::Paste => {
                self.paste_clipboard();
            }
            AppCommand::Duplicate => {
                self.duplicate_selection();
            }
            AppCommand::Delete => {
                self.delete_selected();
            }
            AppCommand::Deselect => {
                self.clear_selection();
            }
            AppCommand::SelectAll => {
                self.select_all();
            }
            AppCommand::InvertSelection => {
                self.invert_selection();
            }
            AppCommand::Group => {
                self.group_selected();
            }
            AppCommand::Ungroup => {
                self.ungroup_selected();
            }
            AppCommand::MakeComponent => {
                self.make_component();
            }
            AppCommand::MakeUnique => {
                self.make_unique();
            }
            AppCommand::ConvertSelectedMeshToExact => {
                self.begin_mesh_conversion_review();
            }
            AppCommand::ReplaceComponent => {
                self.begin_component_replacement();
            }
            AppCommand::SelectAllInstances => {
                self.select_all_instances();
            }
            AppCommand::AssignTag => {
                self.begin_tag_assignment();
            }
            AppCommand::AlignOccurrences => {
                self.begin_occurrence_align();
            }
            AppCommand::DistributeOccurrences => {
                self.begin_occurrence_distribution();
            }
            AppCommand::LinearPattern => {
                self.begin_linear_pattern();
            }
            AppCommand::RectangularPattern => {
                self.begin_rectangular_pattern();
            }
            AppCommand::CircularPattern => {
                self.begin_circular_pattern();
            }
            AppCommand::GroundOccurrence => {
                self.set_selected_occurrence_grounded(true);
            }
            AppCommand::UngroundOccurrence => {
                self.set_selected_occurrence_grounded(false);
            }
            AppCommand::RenameOccurrence => {
                self.begin_occurrence_rename();
            }
            AppCommand::RenameDefinition => {
                self.begin_definition_rename();
            }
            AppCommand::PurgeUnused => {
                self.purge_unused_definitions();
            }
            AppCommand::Hide => {
                self.set_selection_visibility(false);
            }
            AppCommand::HideOthers => {
                self.hide_others();
            }
            AppCommand::Unhide => {
                self.set_selection_visibility(true);
            }
            AppCommand::UnhideAll => {
                self.unhide_all();
            }
            AppCommand::PreviousView => self.previous_view(),
            AppCommand::HomeView => self.home_view(),
            AppCommand::ViewIso => self.look_isometric(),
            AppCommand::ViewTop => self.look_from(0.0, 0.0, "view-top"),
            AppCommand::ViewBottom => self.look_from(0.0, std::f32::consts::PI, "view-bottom"),
            AppCommand::ViewFront => {
                self.look_from(0.0, -std::f32::consts::FRAC_PI_2, "view-front");
            }
            AppCommand::ViewBack => self.look_from(
                std::f32::consts::PI,
                -std::f32::consts::FRAC_PI_2,
                "view-back",
            ),
            AppCommand::ViewRight => self.look_from(
                -std::f32::consts::FRAC_PI_2,
                -std::f32::consts::FRAC_PI_2,
                "view-right",
            ),
            AppCommand::ViewLeft => self.look_from(
                std::f32::consts::FRAC_PI_2,
                -std::f32::consts::FRAC_PI_2,
                "view-left",
            ),
            AppCommand::View(flag) => self.toggle_view(flag),
            AppCommand::ViewShaded => self.restore_shaded(),
            AppCommand::ViewProjection => self.toggle_projection_mode(),
            AppCommand::ZoomFit => self.zoom_fit(),
            AppCommand::ZoomSelection => self.zoom_selection(),
            AppCommand::CenterSelection => self.center_selection(),
            AppCommand::ZoomIn => self.zoom_by(CAMERA_ZOOM_STEP, "digest-zoom-in"),
            AppCommand::ZoomOut => self.zoom_by(CAMERA_ZOOM_STEP.recip(), "digest-zoom-out"),
            AppCommand::Shortcuts => self.panels.shortcuts_open = true,
            AppCommand::CommandSearch => {
                self.command_search.focus = true;
                self.command_search.highlight = 0;
            }
            AppCommand::About => self.panels.about_open = true,
            AppCommand::MaterialTakeoff => self.takeoff.open = true,
            AppCommand::Select
            | AppCommand::Line
            | AppCommand::Rectangle
            | AppCommand::Circle
            | AppCommand::Arc
            | AppCommand::Polygon
            | AppCommand::Ellipse
            | AppCommand::Spline
            | AppCommand::Mirror
            | AppCommand::SolidSubtract
            | AppCommand::SolidTrim
            | AppCommand::SolidUnion
            | AppCommand::SolidIntersect
            | AppCommand::SolidSplit
            | AppCommand::PlanarOffset
            | AppCommand::Helix
            | AppCommand::Sweep
            | AppCommand::Loft
            | AppCommand::Revolve
            | AppCommand::Shell
            | AppCommand::Fillet
            | AppCommand::Chamfer
            | AppCommand::PushPull
            | AppCommand::Move
            | AppCommand::Rotate
            | AppCommand::Scale
            | AppCommand::Measure
            | AppCommand::Orbit
            | AppCommand::Pan
            | AppCommand::ZoomWindow => {}
        }
        if let Some(before) = camera_before {
            self.remember_camera_change(before);
        }
    }

    /// Contents of the value box, where exact input is typed.
    #[must_use]
    pub fn value_input(&self) -> &str {
        &self.value_box.input
    }

    pub(crate) fn cancel_ephemeral_edit_for_history(&mut self) -> bool {
        if !self.ephemeral_edit_active() {
            return false;
        }
        let helix = self.active_tool == ActiveTool::Helix;
        self.clear_ephemeral_edit_state();
        self.cancel_rectangle_sketch();
        if helix {
            self.active_tool = ActiveTool::Select;
        }
        true
    }

    pub(crate) fn complete_mutation_with_publication<T, P, E>(
        &mut self,
        mutate: impl FnOnce(&mut DocumentStore) -> Result<(T, P), E>,
        publish: impl FnOnce(&mut Self, P),
    ) -> Result<T, WorkRecoveryMutationError<E>> {
        self.exact.mutation_readiness = MutationReadiness::Pending;
        let owned_by_program = self.document.current_rule_program().is_some();
        let result = self.mutate_document_with_work_recovery(mutate);
        match result {
            Ok(((value, publication), publication_error)) => {
                if owned_by_program
                    && !self.program_detach_warned
                    && let Some(program) = self.document.detached_rule_program()
                {
                    self.program_detach_notice = Some(program.file_name.clone());
                    self.program_detach_warned = true;
                }
                publish(self, publication);
                self.forget_scene_no_longer_shown();
                let snapshot = self.document.current();
                self.rebind_exact_results(&snapshot);
                self.exact.mutation_readiness = MutationReadiness::Ready;
                match publication_error {
                    Some(error) => Err(WorkRecoveryMutationError::Recovery(error)),
                    None => Ok(value),
                }
            }
            Err(error) => {
                self.exact.mutation_readiness = MutationReadiness::Ready;
                Err(error)
            }
        }
    }

    /// A typed value refused because the program is still being evaluated is
    /// applied by the frame that finds the evaluation ready.
    fn await_program_if_planning(&mut self) {
        if self.status_key == "status-program-planning" {
            self.value_input_awaits_program = Some(self.active_tool);
        }
    }

    pub(crate) fn apply_value_input(&mut self) -> bool {
        if self.gesture.sketch.armed && self.gesture.sketch.start.is_some() {
            return match self.active_tool {
                ActiveTool::Line => self.complete_exact_line(),
                ActiveTool::Circle => self.complete_exact_circle(),
                ActiveTool::Arc => self.complete_exact_arc(),
                ActiveTool::Polygon => self.complete_exact_polygon(),
                ActiveTool::Ellipse => self.complete_exact_ellipse(),
                ActiveTool::Spline => self.complete_exact_spline(),
                _ => self.complete_exact_rectangle(),
            };
        }
        if self.active_tool == ActiveTool::Polygon && self.gesture.sketch.armed {
            return self.set_polygon_sides_from_value_box();
        }
        if self.active_tool == ActiveTool::Scale {
            let Some(factor) = self
                .value_box
                .input
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|factor| factor.is_finite() && *factor > 0.0 && *factor <= 1_000.0)
            else {
                self.digest = self.catalog.text("digest-scale-invalid-factor");
                return false;
            };
            if let Some(mut drag) = self.take_scale_session() {
                drag.factor = factor;
                return self.commit_scale_drag(&drag);
            }
            if self.current_scale_correction().is_some() {
                if self.correct_last_scale(factor) {
                    return true;
                }
                self.end_transform_correction();
                return false;
            }
            if !scale_is_meaningful(factor) {
                self.digest = self.catalog.text("digest-scale-invalid-factor");
                return false;
            }
            return self.scale_selected(factor);
        }
        if self.active_tool == ActiveTool::Rotate {
            let Some(angle_degrees) =
                parse_angle_degrees(&self.value_box.input).filter(|angle| angle.abs() <= 360.0)
            else {
                self.digest = self.catalog.text("digest-rotate-invalid-angle");
                return false;
            };
            // With no gesture in flight the value box edits the turn that just
            // happened rather than adding a second one.
            if self.rotate_session().is_none() && self.correct_last_rotation(angle_degrees) {
                return true;
            }
            if !rotation_is_meaningful(angle_degrees) {
                self.digest = self.catalog.text("digest-rotate-invalid-angle");
                return false;
            }
            // A gesture in flight already knows its centre and axis; typing an
            // angle replaces the one read from the pointer.
            if let Some(mut drag) = self.take_rotate_session(None) {
                drag.angle_degrees = angle_degrees;
                return self.commit_rotate_drag(&drag);
            }
            return self.rotate_selected(angle_degrees);
        }
        if self.active_tool == ActiveTool::Mirror {
            return self.commit_mirror();
        }
        if self.active_tool == ActiveTool::PlanarOffset {
            return self.refresh_planar_offset_preview() && self.confirm_planar_offset_preview();
        }
        if self.active_tool == ActiveTool::Revolve {
            return self.refresh_revolve_preview() && self.confirm_revolve_preview();
        }
        if matches!(
            self.active_tool,
            ActiveTool::Shell | ActiveTool::Fillet | ActiveTool::Chamfer
        ) {
            let applied =
                self.refresh_general_finish_preview() && self.confirm_general_finish_preview();
            self.await_program_if_planning();
            return applied;
        }
        if self.active_tool == ActiveTool::PushPull {
            let selection = self.selection.primary.clone();
            let Some(selection) = selection else {
                self.status_key = "error-push-pull-selection-required";
                self.digest = self.catalog.text("error-push-pull-selection-required");
                return false;
            };
            if parse_distance_mm(&self.value_box.input).is_none() {
                self.digest = self.catalog.text("digest-nothing-to-apply");
                return false;
            }
            // A value typed straight after a Push/Pull is planned as an absolute
            // replacement against its guarded parent. The committed tip remains
            // untouched throughout chooser and preview interaction.
            let current = self.document.current();
            let correction = self.push_pull.last.as_ref().filter(|operation| {
                operation.selection == selection
                    && operation.revision == current.revision_id()
                    && operation.canonical_digest == current.canonical_digest()
            });
            let planning = correction
                .and_then(|_| self.document.tip_replacement_parent().ok())
                .map_or(SmartPushPullPlanning::Append, |parent| {
                    SmartPushPullPlanning::TipReplacement(parent)
                });
            self.gesture.drag.close::<PushPullDrag>();
            self.gesture.drag.close::<PushPullAnchor>();
            self.push_pull.distance_input = self.value_box.input.clone();
            if self.start_preview_for(planning) && self.confirm_push_pull_preview() {
                self.digest = self.catalog.format(
                    "digest-exact-value-applied",
                    &BTreeMap::from([(
                        "value",
                        parse_distance_mm(&self.value_box.input)
                            .map_or_else(String::new, format_signed_mm),
                    )]),
                );
                return true;
            }
            self.await_program_if_planning();
        }
        if self.active_tool == ActiveTool::Move {
            let value = self.value_box.input.trim();
            if value.starts_with(['x', 'X', '*', '×', '/']) {
                let Some((mode, count)) = parse_move_copy_array(value) else {
                    self.digest = self.catalog.text("digest-copy-array-invalid");
                    return false;
                };
                if self.apply_current_move_copy_array(mode, count) {
                    return true;
                }
                self.digest = self.catalog.text("digest-copy-array-unavailable");
                return false;
            }
            // A pinned axis turns a plain number into travel along that axis,
            // which is how a part gets set down exactly 25 mm higher. There a
            // single comma is a decimal comma ("12,5"), not a two-value vector.
            let axis_travel = self.gesture.transform.move_axis_lock.and_then(|axis| {
                let distance = parse_distance_mm(&self.value_box.input)?;
                (distance.abs() >= 0.01).then(|| axis_direction(axis) * distance)
            });
            let exact_vector = axis_travel
                .is_none()
                .then(|| parse_move_vector(&self.value_box.input))
                .flatten();
            let typed = axis_travel.or(exact_vector);
            // A gesture in flight already knows its target and its copy mode;
            // typing a value replaces the one the pointer is showing.
            if let Some(delta_mm) = typed.filter(|delta_mm| length(*delta_mm) > 0.0)
                && let Some(mut drag) = self.take_move_session(None)
            {
                drag.delta_mm = delta_mm;
                if self.commit_move_drag(&drag) {
                    self.set_move_vector_correction_enabled(exact_vector.is_some());
                    self.digest = self.catalog.format(
                        "digest-exact-move-applied",
                        &BTreeMap::from([("value", format_vector_mm(delta_mm))]),
                    );
                    return true;
                }
                return false;
            }
            if let Some(delta_mm) = typed {
                if self.current_move_copy_correction().is_some() {
                    if self.correct_move_copy_delta(delta_mm) {
                        self.digest = self.catalog.format(
                            "digest-exact-move-applied",
                            &BTreeMap::from([("value", format_vector_mm(delta_mm))]),
                        );
                        return true;
                    }
                    self.end_transform_correction();
                }
                let previous = self
                    .current_move_correction()
                    .filter(|(_, operation)| operation.accepts_vector_correction);
                if self.transform_tool.correction.is_some() && previous.is_none() {
                    self.end_transform_correction();
                }
                if let Some((selection, previous)) = previous {
                    if self.correct_move_delta(selection, previous, delta_mm, true) {
                        self.digest = self.catalog.format(
                            "digest-exact-move-applied",
                            &BTreeMap::from([("value", format_vector_mm(delta_mm))]),
                        );
                        return true;
                    }
                    self.end_transform_correction();
                    return false;
                }
                if self.move_selected(delta_mm) {
                    self.set_move_vector_correction_enabled(exact_vector.is_some());
                    self.digest = self.catalog.format(
                        "digest-exact-move-applied",
                        &BTreeMap::from([("value", format_vector_mm(delta_mm))]),
                    );
                    return true;
                }
            } else if let Some(distance_mm) = parse_distance_mm(&self.value_box.input) {
                if let Some((_, previous)) = self.current_move_copy_correction() {
                    let previous_distance_mm = length(previous.delta_mm);
                    if previous_distance_mm > 0.0
                        && self.correct_move_copy_delta(
                            previous.delta_mm * (distance_mm / previous_distance_mm),
                        )
                    {
                        self.digest = self.catalog.format(
                            "digest-exact-move-applied",
                            &BTreeMap::from([("value", format_signed_mm(distance_mm))]),
                        );
                        return true;
                    }
                    self.end_transform_correction();
                }
                let previous = self.current_move_correction();
                if self.transform_tool.correction.is_some() && previous.is_none() {
                    self.end_transform_correction();
                }
                if let Some((selection, previous)) = previous {
                    let correction_mm = distance_mm - previous.applied_distance_mm;
                    if correction_mm.abs() < 0.01 {
                        self.digest = self.catalog.format(
                            "digest-exact-move-applied",
                            &BTreeMap::from([("value", format_signed_mm(distance_mm))]),
                        );
                        return true;
                    }
                    if self.correct_move_delta(
                        selection,
                        previous.clone(),
                        previous.direction * distance_mm,
                        false,
                    ) {
                        self.digest = self.catalog.format(
                            "digest-exact-move-applied",
                            &BTreeMap::from([("value", format_signed_mm(distance_mm))]),
                        );
                        return true;
                    }
                    self.end_transform_correction();
                    return false;
                } else if self.move_selected(Vec3::new(distance_mm, 0.0, 0.0)) {
                    self.digest = self.catalog.format(
                        "digest-exact-move-applied",
                        &BTreeMap::from([("value", format_signed_mm(distance_mm))]),
                    );
                    return true;
                }
            }
        }
        if self.face_offset_confirmation_pending() {
            return true;
        }
        self.digest = self.catalog.text("digest-nothing-to-apply");
        false
    }

    pub(crate) fn command_button(&mut self, ui: &mut egui::Ui, id: AppCommand) {
        let spec = CommandRegistry::spec(id);
        let label = self.catalog.text(spec.label_key);
        let shortcut = keymap::shortcut_text(&self.catalog, id);
        let enabled = self.command_enabled(id);
        if ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_hover_text(shortcut)
            .clicked()
        {
            self.dispatch_command(id);
        }
    }

    pub(crate) fn menu_command(&mut self, ui: &mut egui::Ui, id: AppCommand) {
        let spec = CommandRegistry::spec(id);
        let label = self.catalog.format(
            "menu-command",
            &BTreeMap::from([
                ("label", self.catalog.text(spec.label_key)),
                ("shortcut", keymap::shortcut_text(&self.catalog, id)),
            ]),
        );
        let enabled = self.command_enabled(id);
        let response = ui.add_enabled(enabled, egui::Button::new(label));
        name_widget(&response, enabled, &self.catalog.text(spec.label_key));
        if response.clicked() {
            self.dispatch_command(id);
            ui.close();
        }
    }

    /// Why a disabled command cannot run now, in the user's words.
    pub(crate) fn command_unavailable_reason(&self, id: AppCommand) -> String {
        let key = match id {
            AppCommand::Undo => "command-unavailable-nothing-to-undo",
            AppCommand::Redo => "command-unavailable-nothing-to-redo",
            AppCommand::Paste => "command-unavailable-nothing-to-paste",
            AppCommand::PreviousView => "command-unavailable-no-previous-view",
            AppCommand::ZoomIn | AppCommand::ZoomOut => "command-unavailable-zoom-limit",
            AppCommand::UnhideAll | AppCommand::View(ViewFlag::HiddenObjects) => {
                "command-unavailable-nothing-hidden"
            }
            AppCommand::ViewShaded => "command-unavailable-already-shaded",
            AppCommand::PurgeUnused => "command-unavailable-nothing-unused",
            AppCommand::SolidSubtract
            | AppCommand::SolidTrim
            | AppCommand::SolidUnion
            | AppCommand::SolidIntersect
            | AppCommand::SolidSplit => "command-unavailable-two-solids",
            _ => "command-unavailable-selection",
        };
        self.catalog.text(key)
    }

    /// Commands whose name in the shown language or in English matches
    /// `query`, ignoring case and diacritics; names that start with it first.
    pub fn command_search_matches(&self, query: &str) -> Vec<(AppCommand, String, bool)> {
        static ENGLISH: std::sync::LazyLock<LocaleCatalog> =
            std::sync::LazyLock::new(LocaleCatalog::english);
        let query = fold_for_search(query.trim());
        if query.is_empty() {
            return Vec::new();
        }
        let mut ranked = CommandRegistry::COMMANDS
            .iter()
            .filter(|spec| spec.implemented && spec.id != AppCommand::CommandSearch)
            .filter_map(|spec| {
                let label = self.catalog.text(spec.label_key);
                let rank = [
                    fold_for_search(&label),
                    fold_for_search(&ENGLISH.text(spec.label_key)),
                ]
                .iter()
                .filter_map(|name| {
                    if name.starts_with(&query) {
                        Some(0)
                    } else if name
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|word| word.starts_with(&query))
                    {
                        Some(1)
                    } else {
                        name.contains(&query).then_some(2)
                    }
                })
                .min()?;
                Some((rank, spec.id, label))
            })
            .collect::<Vec<_>>();
        ranked.sort_by_key(|(rank, ..)| *rank);
        ranked
            .into_iter()
            .take(MAX_COMMAND_SEARCH_RESULTS)
            .map(|(_, id, label)| (id, label, self.command_enabled(id)))
            .collect()
    }

    /// The command search: Ctrl+K focuses it, arrows move the highlight,
    /// Enter runs it, Escape clears. Each row shows its shortcut, and a
    /// command that cannot run now says why.
    pub(crate) fn show_command_search(&mut self, ui: &mut egui::Ui) {
        let label = self.catalog.text("command-search");
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.command_search.query)
                .hint_text(self.catalog.text("command-search-placeholder"))
                .desired_width(170.0),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label.clone())
        });
        if std::mem::take(&mut self.command_search.focus) {
            response.request_focus();
        }
        if response.changed() {
            self.command_search.highlight = 0;
        }
        let query = self.command_search.query.clone();
        if query.trim().is_empty() {
            return;
        }
        let matches = self.command_search_matches(&query);
        let typing = response.has_focus() || response.lost_focus();
        let (down, up, enter, escape) = if typing {
            ui.input_mut(|input| {
                (
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    input.key_pressed(egui::Key::Enter),
                    input.key_pressed(egui::Key::Escape),
                )
            })
        } else {
            (false, false, false, false)
        };
        let count = matches.len().max(1);
        let mut highlight = self.command_search.highlight.min(count - 1);
        if down {
            highlight = (highlight + 1) % count;
        }
        if up {
            highlight = (highlight + count - 1) % count;
        }
        self.command_search.highlight = highlight;
        if escape {
            self.command_search.query.clear();
            response.surrender_focus();
            return;
        }
        let rows = matches
            .iter()
            .map(|(command, label, enabled)| {
                (
                    *command,
                    label.clone(),
                    *enabled,
                    keymap::shortcut_text(&self.catalog, *command),
                    (!enabled).then(|| self.command_unavailable_reason(*command)),
                )
            })
            .collect::<Vec<_>>();
        let mut chosen = (enter && !rows.is_empty()).then_some(highlight);
        egui::Area::new(egui::Id::new("command-search-results"))
            .order(egui::Order::Foreground)
            .fixed_pos(response.rect.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(response.rect.width().max(260.0));
                    if rows.is_empty() {
                        ui.label(self.catalog.text("command-search-empty"));
                    }
                    for (index, (_, label, enabled, shortcut, reason)) in rows.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let button =
                                egui::Button::new(label.as_str()).selected(index == highlight);
                            let row = ui.add_enabled(*enabled, button);
                            // Screen readers hear which row Enter will run.
                            row.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Button,
                                    *enabled,
                                    index == highlight,
                                    label.as_str(),
                                )
                            });
                            if row.clicked() {
                                chosen = Some(index);
                            }
                            if !shortcut.is_empty() {
                                ui.weak(shortcut.as_str());
                            }
                        });
                        if let Some(reason) = reason {
                            ui.weak(reason.as_str());
                        }
                    }
                });
            });
        let Some((command, _, enabled, _, reason)) = chosen.and_then(|index| rows.get(index))
        else {
            return;
        };
        if !enabled {
            self.digest = reason.clone().unwrap_or_default();
            return;
        }
        ui.memory_mut(|memory| memory.surrender_focus(response.id));
        self.command_search.query.clear();
        self.command_search.highlight = 0;
        self.dispatch_command(*command);
    }

    pub(crate) fn parameter_expression_nodes(&self) -> Vec<(NodeId, String, String)> {
        self.document
            .current()
            .evaluator_nodes()
            .filter_map(|node| match node.kind() {
                EvaluatorNodeKind::Expression { source, .. }
                | EvaluatorNodeKind::Rule { source, .. } => {
                    Some((node.id(), node.name().to_owned(), source.clone()))
                }
                EvaluatorNodeKind::Parameter { .. } => None,
            })
            .collect()
    }

    pub(crate) fn apply_parameter_expression(&mut self) -> bool {
        let Some(node_id) = self.parameter.editor_node else {
            return false;
        };
        let snapshot = self.document.current();
        let current_provenance = (
            snapshot.document_id(),
            snapshot.revision_id(),
            snapshot.canonical_digest(),
        );
        if self.parameter.provenance.as_ref() != Some(&current_provenance) {
            self.parameter.provenance = Some(current_provenance);
            self.digest = self.catalog.text("error-parameter-stale");
            return false;
        }
        let batch = CommandBatch::edit_evaluator_and_recompute_affected(
            EvaluatorParameterEdit::SetExpression {
                id: node_id,
                expression: self.parameter.expression_input.clone(),
            },
            EvaluationIdentity::default(),
        );
        let proposal = match self.document.prepare_proposal(batch) {
            Ok(proposal) => proposal,
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-parameter-expression",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                return false;
            }
        };
        if proposal.document_id() != snapshot.document_id()
            || proposal.provenance_revision() != snapshot.revision_id()
            || proposal.provenance_digest() != snapshot.canonical_digest()
        {
            self.digest = self.catalog.text("error-parameter-stale");
            return false;
        }
        match self.commit_verified_proposal_with_work_recovery(&proposal) {
            Ok(committed) => {
                self.parameter.last_recomputed_nodes =
                    committed.revision().recomputed_nodes().clone();
                self.parameter.canonical_source = self.parameter.expression_input.clone();
                let committed_snapshot = committed.revision().snapshot();
                self.parameter.provenance = Some((
                    committed_snapshot.document_id(),
                    committed_snapshot.revision_id(),
                    committed_snapshot.canonical_digest(),
                ));
                let value = committed
                    .revision()
                    .evaluation()
                    .and_then(|report| report.node(node_id))
                    .and_then(|node| match node.status {
                        EvaluationStatus::Evaluated(value) => Some(format_height(value)),
                        EvaluationStatus::Error(_) => None,
                    })
                    .unwrap_or_default();
                self.digest = self.catalog.format(
                    "digest-parameter-applied",
                    &BTreeMap::from([("node", node_id.0.to_string()), ("value", value)]),
                );
                self.status_key = "status-ready";
                true
            }
            Err(WorkRecoveryMutationError::Mutation(ProposalCommitError::Stale(_))) => {
                self.digest = self.catalog.text("error-parameter-stale");
                false
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-parameter-expression",
                    &BTreeMap::from([("reason", error.to_string())]),
                );
                false
            }
        }
    }

    /// Every validator the operator can run by hand, in canonical order.
    #[must_use]
    pub const fn validator_ids() -> [&'static str; ASSISTANT_VALIDATOR_IDS.len()] {
        ASSISTANT_VALIDATOR_IDS
    }

    /// The findings of the last manual validator run, if one has been made.
    #[must_use]
    pub fn validator_panel_report(&self) -> Option<&ValidatorPanelReport> {
        self.validator_panel
            .state
            .report_source
            .as_ref()
            .filter(|source| source.matches(&self.document.current()))
            .and(self.validator_panel.report.as_ref())
    }

    /// Runs the selected validators on the current document without mutating it.
    ///
    /// This is the same evidence the Assistant reads, so the operator can see
    /// exactly what a validator says without asking the Assistant first.
    pub fn run_validator_panel(&mut self) {
        if self.validator_panel_pending() {
            return;
        }
        let snapshot = self.document.current();
        self.rebind_exact_results(&snapshot);
        let selection = AssistantValidationSelection {
            mode: "only",
            requested: self.validator_panel.selection.clone(),
            unknown: Vec::new(),
        };
        let validation =
            self.assistant_validation_context(&snapshot, &self.exact.results, &selection);
        self.validator_panel.state.report_source =
            Some(validator_ui::ValidatorSnapshot::new(&snapshot));
        self.validator_panel.state.notice = None;
        self.validator_panel.report = Some(validator_panel_report(&validation));
    }

    pub(crate) fn show_validator_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(self.catalog.text("validators-title"))
            .id_salt("validator-panel")
            .default_open(false)
            .show(ui, |ui| self.show_validator_panel_content(ui));
        ui.separator();
    }

    pub(crate) fn show_validator_panel_content(&mut self, ui: &mut egui::Ui) {
        for validator in ASSISTANT_VALIDATOR_IDS {
            let mut enabled = self.validator_panel.selection.contains(validator);
            let label = self.catalog.text(&format!("validator-{validator}-name"));
            if ui.checkbox(&mut enabled, &label).changed() {
                if enabled {
                    self.validator_panel.selection.insert(validator);
                } else {
                    self.validator_panel.selection.remove(validator);
                }
            }
            ui.label(
                egui::RichText::new(self.catalog.text(&format!("validator-{validator}-what")))
                    .small()
                    .color(self.palette().dim),
            );
        }
        let run = self.catalog.text("validators-run");
        if ui
            .add_enabled(
                !self.validator_panel_pending() && !self.validator_panel.selection.is_empty(),
                egui::Button::new(&run),
            )
            .clicked()
        {
            self.start_validator_panel(ui.ctx());
        }
        if self.validator_panel_pending() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.catalog.text("validators-pending"));
            });
        }
        if let Some(notice) = self.validator_panel.state.notice {
            ui.label(self.catalog.text(notice));
        }
        let Some(report) = self.validator_panel_report() else {
            if !self.validator_panel_pending() && self.validator_panel.state.notice.is_none() {
                ui.label(self.catalog.text("validators-not-run"));
            }
            return;
        };
        ui.label(
            self.catalog.format(
                "validators-summary",
                &BTreeMap::from([
                    (
                        "state",
                        self.catalog
                            .text(&format!("validators-state-{}", report.state)),
                    ),
                    ("count", report.issue_count.to_string()),
                    ("revision", report.revision.to_string()),
                ]),
            ),
        );
        if !report.complete {
            ui.label(self.catalog.text("validators-incomplete"));
        }
        for (validator, reason) in &report.not_evaluated {
            ui.label(self.catalog.format(
                "validators-not-evaluated",
                &BTreeMap::from([
                    (
                        "validator",
                        self.catalog.text(&format!("validator-{validator}-name")),
                    ),
                    ("reason", reason.clone()),
                ]),
            ));
        }
        if report.findings.is_empty() {
            ui.label(self.catalog.text(if report.complete {
                "validators-no-findings"
            } else {
                "validators-no-confirmed-findings"
            }));
            return;
        }
        for finding in &report.findings {
            ui.label(
                self.catalog.format(
                    "validators-finding",
                    &BTreeMap::from([
                        (
                            "validator",
                            self.catalog
                                .text(&format!("validator-{}-name", finding.validator)),
                        ),
                        ("severity", finding.severity.clone()),
                        ("code", finding.code.clone()),
                        ("parts", finding.parts.join(", ")),
                    ]),
                ),
            );
            if !finding.detail.is_empty() {
                ui.label(
                    egui::RichText::new(&finding.detail)
                        .small()
                        .color(self.palette().dim),
                );
            }
        }
    }

    pub(crate) fn show_parameter_editor(&mut self, ui: &mut egui::Ui) {
        let nodes = self.parameter_expression_nodes();
        if nodes.is_empty() {
            self.parameter.editor_node = None;
            self.parameter.expression_input.clear();
            self.parameter.canonical_source.clear();
            self.parameter.provenance = None;
            return;
        }
        let selected_is_current = self
            .parameter
            .editor_node
            .is_some_and(|selected| nodes.iter().any(|(id, _, _)| *id == selected));
        if !selected_is_current {
            self.parameter.editor_node = Some(nodes[0].0);
        }
        let mut selected = self
            .parameter
            .editor_node
            .expect("an editable evaluator node was selected");
        let previous_selected = selected;
        let selected_name = nodes
            .iter()
            .find(|(id, _, _)| *id == selected)
            .map(|(_, name, _)| name.clone())
            .expect("the selected evaluator node is present");

        section_header(ui, self.palette(), &self.catalog.text("parameters-title"));
        let selector_label = self.catalog.text("parameters-node");
        ui.label(&selector_label);
        let selector = egui::ComboBox::from_id_salt("parameter-expression-node")
            .width(ui.available_width())
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                for (id, name, _) in &nodes {
                    ui.selectable_value(&mut selected, *id, name);
                }
            });
        selector.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &selector_label)
        });
        self.parameter.editor_node = Some(selected);
        let canonical_source = nodes
            .iter()
            .find(|(id, _, _)| *id == selected)
            .map(|(_, _, source)| source.clone())
            .expect("the selected evaluator node is present");
        let snapshot = self.document.current();
        let current_provenance = (
            snapshot.document_id(),
            snapshot.revision_id(),
            snapshot.canonical_digest(),
        );
        // An untouched input follows the document, e.g. after an AI edit.
        let untouched = self.parameter.expression_input == self.parameter.canonical_source;
        if selected != previous_selected
            || canonical_source != self.parameter.canonical_source
            || (untouched && self.parameter.provenance.as_ref() != Some(&current_provenance))
        {
            self.parameter.expression_input = canonical_source.clone();
            self.parameter.canonical_source = canonical_source;
            self.parameter.provenance = Some(current_provenance);
        }

        let input_label = self.catalog.text("parameters-expression");
        ui.label(&input_label);
        let input = ui.add(
            egui::TextEdit::singleline(&mut self.parameter.expression_input)
                .hint_text(self.catalog.text("parameters-expression-hint")),
        );
        input.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &input_label)
        });
        ui.small(self.catalog.text("parameters-expression-help"));

        let snapshot = self.document.current();
        if let Ok(report) = snapshot.evaluate(&EvaluationIdentity::default())
            && let Some(node) = report.node(selected)
            && let EvaluationStatus::Evaluated(value) = node.status
        {
            ui.label(self.catalog.format(
                "parameters-result",
                &BTreeMap::from([("value", format_height(value))]),
            ));
        }
        if ui.button(self.catalog.text("parameters-apply")).clicked() {
            self.apply_parameter_expression();
        }
        ui.separator();
    }
}

const MAX_COMMAND_SEARCH_RESULTS: usize = 8;

/// Lower case without diacritics, so "posun" finds "Posunúť".
fn fold_for_search(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'ä' => 'a',
            'č' => 'c',
            'ď' => 'd',
            'é' | 'ě' => 'e',
            'í' => 'i',
            'ĺ' | 'ľ' => 'l',
            'ň' => 'n',
            'ó' | 'ô' | 'ö' => 'o',
            'ŕ' | 'ř' => 'r',
            'š' => 's',
            'ť' => 't',
            'ú' | 'ů' | 'ü' => 'u',
            'ý' => 'y',
            'ž' => 'z',
            other => other,
        })
        .collect()
}

fn parse_move_copy_array(input: &str) -> Option<(MoveCopyArrayMode, usize)> {
    let trimmed = input.trim();
    let (mode, count) = if let Some(count) = trimmed
        .strip_prefix('x')
        .or_else(|| trimmed.strip_prefix('X'))
        .or_else(|| trimmed.strip_prefix('*'))
        .or_else(|| trimmed.strip_prefix('×'))
    {
        (MoveCopyArrayMode::Multiply, count)
    } else {
        (MoveCopyArrayMode::Divide, trimmed.strip_prefix('/')?)
    };
    let count = count.trim().parse::<usize>().ok()?;
    (1..=MAX_PATTERN_COUNT)
        .contains(&count)
        .then_some((mode, count))
}

fn parse_move_vector(input: &str) -> Option<Vec3> {
    let trimmed = input.trim();
    let numeric = trimmed
        .strip_suffix("mm")
        .or_else(|| trimmed.strip_suffix("MM"))
        .unwrap_or(trimmed);
    let values = numeric
        .split([',', ';', 'x', 'X', '*'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let vector = match values.as_slice() {
        [x, y] => Vec3::new(*x, *y, 0.0),
        [x, y, z] => Vec3::new(*x, *y, *z),
        _ => return None,
    };
    (vector.x.is_finite() && vector.y.is_finite() && vector.z.is_finite()).then_some(vector)
}

pub(crate) fn format_vector_mm(vector: Vec3) -> String {
    format!(
        "{},{},{} mm",
        format_height(vector.x),
        format_height(vector.y),
        format_height(vector.z)
    )
}

fn parse_angle_degrees(input: &str) -> Option<f64> {
    let trimmed = input.trim();
    let numeric = trimmed
        .strip_suffix('\u{b0}')
        .or_else(|| trimmed.strip_suffix("deg"))
        .or_else(|| trimmed.strip_suffix("DEG"))
        .unwrap_or(trimmed)
        .trim();
    let degrees = numeric.parse::<f64>().ok()?;
    degrees.is_finite().then_some(degrees)
}

pub(crate) fn format_signed_mm(distance: f64) -> String {
    if distance > 0.0 {
        format!("+{} mm", format_height(distance))
    } else {
        format!("{} mm", format_height(distance))
    }
}
