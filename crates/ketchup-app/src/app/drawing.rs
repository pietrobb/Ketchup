//! Drawing tools and profiles, previews of feature tools (revolve, sweep, loft, offset, finish, cut).

use crate::*;

impl KetchupApp {
    #[must_use]
    pub const fn line_axis_lock(&self) -> Option<Axis> {
        self.gesture.sketch.axis_lock
    }

    pub(crate) fn box_height_mm(&self, definition_id: DefinitionId) -> Option<f64> {
        self.box_height_mm_for_snapshot(&self.document.current(), definition_id)
    }

    pub(crate) fn box_height_mm_for_snapshot(
        &self,
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> Option<f64> {
        self.active_boxes_for_snapshot(snapshot)
            .into_iter()
            .find(|item| item.definition_id == definition_id)
            .map(|item| item.size_mm.z)
    }

    pub(crate) fn active_boxes(&self) -> Vec<RenderBox> {
        let snapshot = self.document.current();
        self.refresh_interaction_projection_cache(&snapshot);
        let cache = self.hover.projection_cache.borrow();
        cache
            .as_ref()
            .expect("interaction cache was built")
            .render_boxes
            .get_or_init(|| {
                self.render_boxes_from_projection(
                    &snapshot,
                    &cache
                        .as_ref()
                        .expect("interaction cache was built")
                        .canonical,
                    true,
                )
            })
            .clone()
    }

    pub(crate) fn active_boxes_for_snapshot(&self, snapshot: &Snapshot) -> Vec<RenderBox> {
        let current = self.document.current();
        if snapshot.document_id() == current.document_id()
            && snapshot.revision_id() == current.revision_id()
            && snapshot.canonical_digest() == current.canonical_digest()
        {
            return self.active_boxes();
        }
        self.render_boxes_from_projection(
            snapshot,
            &CanonicalInteractionProjection::from_snapshot(snapshot),
            true,
        )
    }

    #[must_use]
    pub fn active_box_count(&self) -> usize {
        self.document.current().scene_query().len()
    }

    pub(crate) fn begin_revolve_tool(&mut self) -> bool {
        let Some(tool) = self.selected_revolve_profile() else {
            return false;
        };
        self.solid_tools.revolve = Some(tool);
        self.tool_preview.close::<RevolvePreview>();
        self.value_box.input = "360".to_owned();
        self.status_key = "status-revolve-axis-start";
        true
    }

    pub(crate) fn add_revolve_axis_point(&mut self, point_mm: Vec3) -> bool {
        let Some(mut tool) = self.solid_tools.revolve.clone() else {
            return false;
        };
        let snapshot = self.document.current();
        if snapshot.document_id() != tool.source.source_document_id
            || snapshot.revision_id() != tool.source.source_revision
            || snapshot.canonical_digest() != tool.source.source_digest
        {
            self.solid_tools.revolve = None;
            self.tool_preview.close::<RevolvePreview>();
            self.status_key = "error-preview-stale";
            return false;
        }
        let local = [
            point_mm.x - tool.source.translation_mm.x,
            point_mm.y - tool.source.translation_mm.y,
        ];
        if let Some(start) = tool.axis_start_mm {
            if (local[0] - start[0]).hypot(local[1] - start[1]) <= 0.01 {
                return false;
            }
            tool.axis_end_mm = Some(local);
            self.status_key = "status-revolve-angle";
        } else {
            tool.axis_start_mm = Some(local);
            self.status_key = "status-revolve-axis-end";
        }
        self.solid_tools.revolve = Some(tool);
        self.refresh_revolve_preview()
    }

    pub(crate) fn derive_revolve_preview_plan(
        &self,
        source: &RevolveSourcePlan,
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    ) -> Option<(RevolvePreviewPlan, CommandBatch)> {
        if angle_degrees <= 0.0 || angle_degrees > 360.0 {
            return None;
        }
        let current = self.selected_revolve_profile()?;
        if &current.source != source {
            return None;
        }
        let snapshot = self.document.current();
        let generated_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)?;
        let command = CanonicalCommand::CreateFeature {
            id: generated_feature_id,
            definition_id: source.definition_id,
            name: self.catalog.text("model-revolve-feature"),
            kind: FeatureKind::Revolve {
                profile: source.profile_feature_id,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            },
        };
        let batch = CommandBatch::new(vec![command.clone()]);
        let preview_snapshot = self.document.preview_batch(&batch).ok()?;
        let exact_request = ExactBRepGraph::from_snapshot(
            &preview_snapshot,
            source.definition_id,
            generated_feature_id,
        )
        .ok()?;
        Some((
            RevolvePreviewPlan {
                source: source.clone(),
                generated_feature_id,
                axis_start_mm,
                axis_end_mm,
                angle_degrees_bits: angle_degrees.to_bits(),
                command,
                exact_request,
            },
            batch,
        ))
    }

    pub(crate) fn refresh_revolve_preview(&mut self) -> bool {
        self.tool_preview.close::<RevolvePreview>();
        let Some(tool) = self.solid_tools.revolve.as_ref() else {
            return false;
        };
        let Some(axis_start_mm) = tool.axis_start_mm else {
            return false;
        };
        let Some(axis_end_mm) = tool.axis_end_mm else {
            return false;
        };
        let Some(angle_degrees) = parse_distance_mm(&self.value_box.input)
            .filter(|angle| *angle > 0.0 && *angle <= 360.0)
        else {
            self.digest = self.catalog.text("digest-revolve-invalid-angle");
            return false;
        };
        let Some((plan, batch)) = self.derive_revolve_preview_plan(
            &tool.source,
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
        ) else {
            self.status_key = "error-preview-stale";
            return false;
        };
        self.tool_preview.open(RevolvePreview { plan, batch });
        self.status_key = "status-revolve-preview";
        self.digest = self.catalog.format(
            "digest-revolve-live",
            &BTreeMap::from([("angle", format_height(angle_degrees))]),
        );
        true
    }

    #[must_use]
    pub fn has_revolve_preview(&self) -> bool {
        let Some(preview) = self.tool_preview.get::<RevolvePreview>() else {
            return false;
        };
        let Some(tool) = self.solid_tools.revolve.as_ref() else {
            return false;
        };
        let angle_degrees = f64::from_bits(preview.plan.angle_degrees_bits);
        tool.source == preview.plan.source
            && tool.axis_start_mm == Some(preview.plan.axis_start_mm)
            && tool.axis_end_mm == Some(preview.plan.axis_end_mm)
            && parse_distance_mm(&self.value_box.input).map(f64::to_bits)
                == Some(preview.plan.angle_degrees_bits)
            && self
                .derive_revolve_preview_plan(
                    &preview.plan.source,
                    preview.plan.axis_start_mm,
                    preview.plan.axis_end_mm,
                    angle_degrees,
                )
                .is_some_and(|(plan, batch)| plan == preview.plan && batch == preview.batch)
    }

    #[must_use]
    pub fn revolve_preview_parameters(&self) -> Option<([f64; 2], [f64; 2], f64)> {
        self.has_revolve_preview().then(|| {
            let preview = self
                .tool_preview
                .get::<RevolvePreview>()
                .expect("a current Revolve preview exists");
            (
                preview.plan.axis_start_mm,
                preview.plan.axis_end_mm,
                f64::from_bits(preview.plan.angle_degrees_bits),
            )
        })
    }

    #[must_use]
    pub fn latest_revolve_parameters(&self) -> Option<(FeatureId, [f64; 2], [f64; 2], f64)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::Revolve {
                    axis_start_mm,
                    axis_end_mm,
                    angle_degrees,
                    ..
                } = feature.kind()
                else {
                    return None;
                };
                Some((feature.id(), *axis_start_mm, *axis_end_mm, *angle_degrees))
            })
            .last()
    }

    pub(crate) fn confirm_revolve_preview(&mut self) -> bool {
        if !self.has_revolve_preview() {
            self.tool_preview.close::<RevolvePreview>();
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.tool_preview.remove::<RevolvePreview>() else {
            return false;
        };
        if self.apply_batch_with_work_recovery(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.value_box.input.clear();
        self.status_key = "status-ready";
        self.digest = self.catalog.format(
            "digest-revolve-committed",
            &BTreeMap::from([(
                "angle",
                format_height(f64::from_bits(preview.plan.angle_degrees_bits)),
            )]),
        );
        true
    }

    pub(crate) fn planar_offset_source_plan(&self) -> Option<PlanarOffsetSourcePlan> {
        let selection = self.selection.primary.as_ref()?;
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
            || !selection.instance_path.is_root()
        {
            return None;
        }
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        if item.extrusion_feature_id.is_some() {
            return None;
        }
        let snapshot = self.document.current();
        let definition = snapshot.definition(selection.definition_id)?;
        if definition.feature_ids() != [item.profile_feature_id] {
            return None;
        }
        let profile = snapshot.feature(item.profile_feature_id)?;
        let FeatureKind::Profile { segments, closed } = profile.kind() else {
            return None;
        };
        ketchup_model::exact_product::exact_planar_offset_profile(
            segments,
            *closed,
            snapshot.tolerance(),
        )?;
        let world_transform = snapshot
            .resolve_instance_path(&selection.instance_path)
            .ok()?
            .world_transform;
        Some(PlanarOffsetSourcePlan {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            definition_id: selection.definition_id,
            profile_feature_id: item.profile_feature_id,
            profile_kind: profile.kind().clone(),
            world_transform,
        })
    }

    /// How far the pointer is from the selected profile's outline in the
    /// profile's plane: negative inside, positive outside.
    pub(crate) fn planar_offset_distance_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
    ) -> Option<f64> {
        let source = self.planar_offset_source_plan()?;
        let FeatureKind::Profile { segments, .. } = &source.profile_kind else {
            return None;
        };
        let outline = segments
            .iter()
            .map(|segment| profile_segment_polyline(segment, PROFILE_CURVE_STEPS))
            .collect::<Option<Vec<_>>>()?
            .concat();
        let place = |point: Vec3| transform_model_point(source.world_transform, point);
        let origin = place(Vec3::ZERO);
        let x_axis = (place(Vec3::new(1.0, 0.0, 0.0)) - origin).normalized()?;
        let y_axis = (place(Vec3::new(0.0, 1.0, 0.0)) - origin).normalized()?;
        let frame = WorkplaneFrame {
            origin_mm: origin.to_array(),
            x_axis: x_axis.to_array(),
            y_axis: y_axis.to_array(),
            normal: x_axis.cross(y_axis).normalized()?.to_array(),
        };
        let hit = crate::drawing_plane::local_point(
            frame,
            self.screen_to_workplane(pointer, rect, frame)?,
        );
        signed_outline_distance(&outline, [hit.x, hit.y])
    }

    /// The offset the Offset tool applies: the typed distance, else the one the
    /// pointer asks for, shown as it is written into the document.
    fn planar_offset_request(&self) -> Option<(String, f64)> {
        let expression = if self.value_box.input.trim().is_empty() {
            format_height(self.gesture.planar_offset_mm?)
        } else {
            self.value_box.input.clone()
        };
        let distance_mm = parse_distance_mm(&expression)?;
        Some((expression, distance_mm))
    }

    pub(crate) fn derive_planar_offset_preview_plan(
        &self,
        source: &PlanarOffsetSourcePlan,
        distance_expression: &str,
        distance_mm: f64,
    ) -> Option<(PlanarOffsetPreviewPlan, CommandBatch)> {
        if distance_mm.abs() <= APPROXIMATION
            || self.planar_offset_source_plan().as_ref() != Some(source)
        {
            return None;
        }
        let distance = Dimension::new(distance_expression.to_owned(), distance_mm).ok()?;
        let snapshot = self.document.current();
        let generated_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)?;
        let command = CanonicalCommand::CreateFeature {
            id: generated_feature_id,
            definition_id: source.definition_id,
            name: self.catalog.text("model-planar-offset-feature"),
            kind: FeatureKind::PlanarOffset {
                profile: source.profile_feature_id,
                distance,
            },
        };
        let batch = CommandBatch::new(vec![command.clone()]);
        let preview_snapshot = self.document.preview_batch(&batch).ok()?;
        let exact_graph = ExactBRepGraph::from_snapshot(
            &preview_snapshot,
            source.definition_id,
            generated_feature_id,
        )
        .ok()?;
        Some((
            PlanarOffsetPreviewPlan {
                source: source.clone(),
                generated_feature_id,
                distance_expression: distance_expression.to_owned(),
                distance_mm_bits: distance_mm.to_bits(),
                command,
                exact_graph,
            },
            batch,
        ))
    }

    pub(crate) fn refresh_planar_offset_preview(&mut self) -> bool {
        self.tool_preview.close::<PlanarOffsetPreview>();
        let Some(source) = self.planar_offset_source_plan() else {
            return false;
        };
        let Some((expression, distance_mm)) = self
            .planar_offset_request()
            .filter(|(_, distance)| distance.abs() > APPROXIMATION)
        else {
            self.digest = self.catalog.text("digest-planar-offset-invalid-distance");
            return false;
        };
        let Some((plan, batch)) =
            self.derive_planar_offset_preview_plan(&source, &expression, distance_mm)
        else {
            self.digest = self.catalog.text("digest-planar-offset-invalid-distance");
            return false;
        };
        self.tool_preview.open(PlanarOffsetPreview { plan, batch });
        self.status_key = "status-planar-offset-preview";
        self.digest = self.catalog.format(
            "digest-planar-offset-live",
            &BTreeMap::from([("distance", format_height(distance_mm))]),
        );
        true
    }

    #[must_use]
    pub fn planar_offset_preview_parameters(&self) -> Option<(FeatureId, f64, [[f64; 3]; 2])> {
        let preview = self.tool_preview.get::<PlanarOffsetPreview>()?;
        if !self.planar_offset_preview_is_current() {
            return None;
        }
        Some((
            preview.plan.source.profile_feature_id,
            f64::from_bits(preview.plan.distance_mm_bits),
            preview.plan.exact_graph.producer_bounds_mm().ok()??,
        ))
    }

    #[must_use]
    pub fn planar_offset_preview_is_current(&self) -> bool {
        let Some(preview) = self.tool_preview.get::<PlanarOffsetPreview>() else {
            return false;
        };
        self.planar_offset_request()
            .is_some_and(|(expression, distance_mm)| {
                expression == preview.plan.distance_expression
                    && distance_mm.to_bits() == preview.plan.distance_mm_bits
            })
            && self
                .derive_planar_offset_preview_plan(
                    &preview.plan.source,
                    &preview.plan.distance_expression,
                    f64::from_bits(preview.plan.distance_mm_bits),
                )
                .is_some_and(|(plan, batch)| plan == preview.plan && batch == preview.batch)
    }

    #[must_use]
    pub fn latest_planar_offset_parameters(&self) -> Option<(FeatureId, FeatureId, f64)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::PlanarOffset { profile, distance } = feature.kind() else {
                    return None;
                };
                Some((feature.id(), *profile, distance.millimetres()))
            })
            .last()
    }

    pub(crate) fn confirm_planar_offset_preview(&mut self) -> bool {
        if !self.planar_offset_preview_is_current() {
            self.tool_preview.close::<PlanarOffsetPreview>();
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.tool_preview.remove::<PlanarOffsetPreview>() else {
            return false;
        };
        if self.apply_batch_with_work_recovery(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.value_box.input.clear();
        self.status_key = "status-ready";
        self.digest = self.catalog.format(
            "digest-planar-offset-committed",
            &BTreeMap::from([(
                "distance",
                format_height(f64::from_bits(preview.plan.distance_mm_bits)),
            )]),
        );
        true
    }

    pub(crate) fn sweep_source_plan(&self) -> Option<SweepSourcePlan> {
        let selection = self.selection.primary.as_ref()?;
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
            || !selection.instance_path.is_root()
        {
            return None;
        }
        let snapshot = self.document.current();
        let definition = snapshot.definition(selection.definition_id)?;
        let [profile_feature_id, path_feature_id] = definition.feature_ids() else {
            return None;
        };
        let profile = snapshot.feature(*profile_feature_id)?;
        let path = snapshot.feature(*path_feature_id)?;
        let world_transform = snapshot
            .resolve_instance_path(&selection.instance_path)
            .ok()?
            .world_transform;
        Some(SweepSourcePlan {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            definition_id: selection.definition_id,
            profile_feature_id: *profile_feature_id,
            profile_kind: profile.kind().clone(),
            path_feature_id: *path_feature_id,
            path_kind: path.kind().clone(),
            world_transform,
        })
    }

    pub(crate) fn derive_sweep_preview_plan(
        &self,
        source: &SweepSourcePlan,
    ) -> Option<(SweepPreviewPlan, CommandBatch)> {
        if self.sweep_source_plan().as_ref() != Some(source) {
            return None;
        }
        let snapshot = self.document.current();
        let generated_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)?;
        let command = CanonicalCommand::CreateFeature {
            id: generated_feature_id,
            definition_id: source.definition_id,
            name: self.catalog.text("model-sweep-feature"),
            kind: FeatureKind::sweep(source.profile_feature_id, source.path_feature_id),
        };
        let batch = CommandBatch::new(vec![command.clone()]);
        let preview_snapshot = self.document.preview_batch(&batch).ok()?;
        let exact_graph = ExactBRepGraph::from_snapshot(
            &preview_snapshot,
            source.definition_id,
            generated_feature_id,
        )
        .ok()?;
        Some((
            SweepPreviewPlan {
                source: source.clone(),
                generated_feature_id,
                command,
                exact_graph,
            },
            batch,
        ))
    }

    pub(crate) fn sweep_preview_candidate(&self) -> Option<(SweepPreviewPlan, CommandBatch)> {
        let source = self.sweep_source_plan()?;
        self.derive_sweep_preview_plan(&source)
    }

    pub(crate) fn refresh_sweep_preview(&mut self) -> bool {
        self.tool_preview.close::<SweepPreview>();
        let Some((plan, batch)) = self.sweep_preview_candidate() else {
            self.digest = self.catalog.text("digest-sweep-invalid-inputs");
            return false;
        };
        self.tool_preview.open(SweepPreview { plan, batch });
        self.status_key = "status-sweep-preview";
        self.digest = self.catalog.text("digest-sweep-live");
        true
    }

    #[must_use]
    pub fn sweep_preview_parameters(&self) -> Option<(FeatureId, FeatureId, [[f64; 3]; 2])> {
        let preview = self.tool_preview.get::<SweepPreview>()?;
        if !self.sweep_preview_is_current() {
            return None;
        }
        Some((
            preview.plan.source.profile_feature_id,
            preview.plan.source.path_feature_id,
            preview.plan.exact_graph.producer_bounds_mm().ok()??,
        ))
    }

    #[must_use]
    pub fn sweep_preview_is_current(&self) -> bool {
        let Some(preview) = self.tool_preview.get::<SweepPreview>() else {
            return false;
        };
        self.derive_sweep_preview_plan(&preview.plan.source)
            .is_some_and(|(plan, batch)| plan == preview.plan && batch == preview.batch)
    }

    #[must_use]
    pub fn latest_sweep_parameters(&self) -> Option<(FeatureId, FeatureId, FeatureId)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::Sweep { profile, path, .. } = feature.kind() else {
                    return None;
                };
                Some((feature.id(), *profile, *path))
            })
            .last()
    }

    pub(crate) fn confirm_sweep_preview(&mut self) -> bool {
        if !self.sweep_preview_is_current() {
            self.tool_preview.close::<SweepPreview>();
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.tool_preview.remove::<SweepPreview>() else {
            return false;
        };
        if self.apply_batch_with_work_recovery(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.status_key = "status-ready";
        self.digest = self.catalog.text("digest-sweep-committed");
        true
    }

    pub(crate) fn loft_source_plan(&self) -> Option<LoftSourcePlan> {
        let selection = self.selection.primary.as_ref()?;
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
            || !selection.instance_path.is_root()
        {
            return None;
        }
        let (definition_id, sections) = self.solid_tools.loft_input_sections.as_ref()?;
        if selection.definition_id != *definition_id {
            return None;
        }
        let snapshot = self.document.current();
        let definition = snapshot.definition(*definition_id)?;
        if !definition
            .feature_ids()
            .iter()
            .copied()
            .eq(sections.iter().map(|section| section.profile))
        {
            return None;
        }
        let profile_kinds = sections
            .iter()
            .map(|section| {
                snapshot
                    .feature(section.profile)
                    .map(|feature| (section.profile, feature.kind().clone()))
            })
            .collect::<Option<Vec<_>>>()?;
        let world_transform = snapshot
            .resolve_instance_path(&selection.instance_path)
            .ok()?
            .world_transform;
        Some(LoftSourcePlan {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            definition_id: *definition_id,
            sections: sections.clone(),
            profile_kinds,
            world_transform,
        })
    }

    pub(crate) fn derive_loft_preview_plan(
        &self,
        source: &LoftSourcePlan,
    ) -> Option<(LoftPreviewPlan, CommandBatch)> {
        if self.loft_source_plan().as_ref() != Some(source) {
            return None;
        }
        let snapshot = self.document.current();
        let generated_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)?;
        let command = CanonicalCommand::CreateFeature {
            id: generated_feature_id,
            definition_id: source.definition_id,
            name: self.catalog.text("model-loft-feature"),
            kind: FeatureKind::Loft {
                sections: source.sections.clone(),
                guide: None,
                continuity: LoftContinuity::Position,
            },
        };
        let batch = CommandBatch::new(vec![command.clone()]);
        let preview_snapshot = self.document.preview_batch(&batch).ok()?;
        let exact_graph = ExactBRepGraph::from_snapshot(
            &preview_snapshot,
            source.definition_id,
            generated_feature_id,
        )
        .ok()?;
        Some((
            LoftPreviewPlan {
                source: source.clone(),
                generated_feature_id,
                command,
                exact_graph,
            },
            batch,
        ))
    }

    pub(crate) fn loft_preview_candidate(&self) -> Option<(LoftPreviewPlan, CommandBatch)> {
        let source = self.loft_source_plan()?;
        self.derive_loft_preview_plan(&source)
    }

    pub(crate) fn refresh_loft_preview(&mut self) -> bool {
        self.tool_preview.close::<LoftPreview>();
        let Some((plan, batch)) = self.loft_preview_candidate() else {
            self.digest = self.catalog.text("digest-loft-invalid-inputs");
            return false;
        };
        self.tool_preview.open(LoftPreview { plan, batch });
        self.status_key = "status-loft-preview";
        self.digest = self.catalog.text("digest-loft-live");
        true
    }

    #[must_use]
    pub fn loft_preview_parameters(&self) -> Option<LoftPreviewParameters> {
        let preview = self.tool_preview.get::<LoftPreview>()?;
        if !self.loft_preview_is_current() {
            return None;
        }
        Some((
            preview
                .plan
                .source
                .sections
                .iter()
                .map(|section| (section.profile, section.elevation_mm))
                .collect(),
            preview.plan.exact_graph.producer_bounds_mm().ok()??,
        ))
    }

    #[must_use]
    pub fn loft_preview_is_current(&self) -> bool {
        let Some(preview) = self.tool_preview.get::<LoftPreview>() else {
            return false;
        };
        self.derive_loft_preview_plan(&preview.plan.source)
            .is_some_and(|(plan, batch)| plan == preview.plan && batch == preview.batch)
    }

    #[must_use]
    pub fn latest_loft_parameters(&self) -> Option<(FeatureId, Vec<(FeatureId, f64)>)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::Loft { sections, .. } = feature.kind() else {
                    return None;
                };
                Some((
                    feature.id(),
                    sections
                        .iter()
                        .map(|section| (section.profile, section.elevation_mm))
                        .collect(),
                ))
            })
            .last()
    }

    pub(crate) fn confirm_loft_preview(&mut self) -> bool {
        if !self.loft_preview_is_current() {
            self.tool_preview.close::<LoftPreview>();
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.tool_preview.remove::<LoftPreview>() else {
            return false;
        };
        if self.apply_batch_with_work_recovery(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.solid_tools.loft_input_sections = None;
        self.active_tool = ActiveTool::Select;
        self.status_key = "status-ready";
        self.digest = self.catalog.text("digest-loft-committed");
        true
    }

    pub(crate) fn general_finish_source_plan(
        &self,
        kind: GeneralFinishKind,
    ) -> Option<GeneralFinishSourcePlan> {
        let selection = self.selection.primary.as_ref()?;
        let expected_kind = if kind == GeneralFinishKind::Shell {
            TopologicalElementKind::Face
        } else {
            TopologicalElementKind::Edge
        };
        let (definition_id, target_feature_id, topological_selections, references) =
            self.selected_general_finish_target(expected_kind)?;
        let snapshot = self.document.current();
        let target_feature_kind = snapshot.feature(target_feature_id)?.kind().clone();
        let world_transform = snapshot
            .resolve_instance_path(&selection.instance_path)
            .ok()?
            .world_transform;
        let exact_graph =
            ExactBRepGraph::from_snapshot(&snapshot, definition_id, target_feature_id).ok()?;
        debug_assert!(
            references
                .iter()
                .all(|reference| reference.producer_feature_id == target_feature_id)
        );
        Some(GeneralFinishSourcePlan {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            definition_id,
            target_feature_id,
            target_feature_kind,
            topological_selections,
            kind,
            world_transform,
            exact_graph,
        })
    }

    pub(crate) fn derive_general_finish_preview_plan(
        &self,
        source: &GeneralFinishSourcePlan,
        amount_input: &str,
    ) -> Option<(GeneralFinishPreviewPlan, CommandBatch)> {
        let active_kind = match self.active_tool {
            ActiveTool::Shell => GeneralFinishKind::Shell,
            ActiveTool::Fillet => GeneralFinishKind::Fillet,
            ActiveTool::Chamfer => GeneralFinishKind::Chamfer,
            _ => return None,
        };
        if active_kind != source.kind
            || self.general_finish_source_plan(source.kind).as_ref() != Some(source)
        {
            return None;
        }
        let amount_mm = parse_distance_mm(amount_input).filter(|amount| *amount > 0.0)?;
        let dimension = Dimension::new(amount_input.to_owned(), amount_mm).ok()?;
        let snapshot = self.document.current();
        let generated_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)?;
        let references = source
            .topological_selections
            .iter()
            .map(|selection| {
                selection
                    .resolve_current(&snapshot, &self.exact.topology_results)
                    .ok()
                    .map(|resolved| resolved.reference)
            })
            .collect::<Option<Vec<_>>>()?;
        let feature_kind = plan_topology_finish_kind(
            source.kind,
            source.target_feature_id,
            references,
            dimension,
        )?;
        let name_key = match source.kind {
            GeneralFinishKind::Shell => "model-shell-feature",
            GeneralFinishKind::Fillet => "model-fillet-feature",
            GeneralFinishKind::Chamfer => "model-chamfer-feature",
        };
        let command = CanonicalCommand::CreateFeature {
            id: generated_feature_id,
            definition_id: source.definition_id,
            name: self.catalog.text(name_key),
            kind: feature_kind,
        };
        let batch = CommandBatch::new(vec![command.clone()]);
        let preview_snapshot = self.document.preview_batch(&batch).ok()?;
        let exact_graph = ExactBRepGraph::from_snapshot(
            &preview_snapshot,
            source.definition_id,
            generated_feature_id,
        )
        .ok()?;
        Some((
            GeneralFinishPreviewPlan {
                source: source.clone(),
                generated_feature_id,
                amount_mm_bits: amount_mm.to_bits(),
                command,
                exact_graph,
            },
            batch,
        ))
    }

    pub(crate) fn refresh_general_finish_preview(&mut self) -> bool {
        self.tool_preview.close::<GeneralFinishPreview>();
        let kind = match self.active_tool {
            ActiveTool::Shell => GeneralFinishKind::Shell,
            ActiveTool::Fillet => GeneralFinishKind::Fillet,
            ActiveTool::Chamfer => GeneralFinishKind::Chamfer,
            _ => return false,
        };
        let Some(source) = self.general_finish_source_plan(kind) else {
            return false;
        };
        let Some((plan, batch)) =
            self.derive_general_finish_preview_plan(&source, &self.value_box.input)
        else {
            self.digest = self.catalog.text("digest-general-finish-invalid-amount");
            return false;
        };
        let amount_mm = f64::from_bits(plan.amount_mm_bits);
        self.tool_preview.open(GeneralFinishPreview { plan, batch });
        self.status_key = "status-general-finish-preview";
        self.digest = self.catalog.format(
            "digest-general-finish-live",
            &BTreeMap::from([("amount", format_height(amount_mm))]),
        );
        true
    }

    #[must_use]
    pub fn general_finish_preview_parameters(
        &self,
    ) -> Option<(FeatureId, TopologicalElementRef, GeneralFinishKind, f64)> {
        let (target, references, kind, amount) =
            self.general_finish_preview_selection_parameters()?;
        Some((target, references.into_iter().next()?, kind, amount))
    }

    #[must_use]
    pub fn general_finish_preview_is_current(&self) -> bool {
        let Some(preview) = self.tool_preview.get::<GeneralFinishPreview>() else {
            return false;
        };
        self.derive_general_finish_preview_plan(&preview.plan.source, &self.value_box.input)
            .is_some_and(|(plan, batch)| plan == preview.plan && batch == preview.batch)
    }

    pub(crate) fn confirm_general_finish_preview(&mut self) -> bool {
        if !self.general_finish_preview_is_current() {
            self.tool_preview.close::<GeneralFinishPreview>();
            self.status_key = "error-preview-stale";
            return false;
        }
        // Naming the picked edges needs the program evaluated; the preview
        // stays and the commit runs again once it is ready.
        if self.program_is_planning() {
            self.status_key = "status-program-planning";
            self.digest = self.catalog.text("status-program-planning");
            return false;
        }
        let Some(preview) = self.tool_preview.remove::<GeneralFinishPreview>() else {
            return false;
        };
        let amount_mm = f64::from_bits(preview.plan.amount_mm_bits);
        match self.program_general_finish(&preview.plan.source, amount_mm) {
            Some(Err(error)) => {
                self.digest = error.reason_text().to_owned();
                return false;
            }
            Some(Ok(())) => {}
            None => {
                if self.apply_batch_with_work_recovery(&preview.batch).is_err() {
                    self.status_key = "error-preview-stale";
                    return false;
                }
            }
        }
        self.clear_ephemeral_edit_state();
        self.active_tool = ActiveTool::Select;
        self.value_box.input.clear();
        self.status_key = "status-ready";
        self.digest = self.catalog.format(
            "digest-general-finish-committed",
            &BTreeMap::from([(
                "amount",
                format_height(f64::from_bits(preview.plan.amount_mm_bits)),
            )]),
        );
        true
    }

    /// Current non-authoritative Circle preview as centre and radius.
    #[must_use]
    pub fn circle_preview_geometry(&self) -> Option<(Vec3, f64)> {
        (self.active_tool == ActiveTool::Circle)
            .then_some((self.gesture.sketch.start?, self.gesture.sketch.cursor?))
            .map(|(center, cursor)| {
                (
                    center,
                    length(Vec3::new(cursor.x - center.x, cursor.y - center.y, 0.0)),
                )
            })
    }

    /// Outline of the closed shape being drawn, in world coordinates, while its
    /// next point follows the pointer; `None` when no such shape is in progress.
    #[must_use]
    pub fn closed_shape_preview_outline(&self) -> Option<Vec<Vec3>> {
        let center = self.gesture.sketch.start?;
        let cursor = self.gesture.sketch.cursor?;
        let ellipse = |radius_x: f64, radius_y: f64, turn: f64| {
            let (sin, cos) = turn.sin_cos();
            (0..PREVIEW_CURVE_SEGMENTS)
                .map(|step| {
                    let angle = std::f64::consts::TAU * step as f64 / PREVIEW_CURVE_SEGMENTS as f64;
                    let (x, y) = (radius_x * angle.cos(), radius_y * angle.sin());
                    [cos * x - sin * y, sin * x + cos * y]
                })
                .collect::<Vec<_>>()
        };
        let corners = match (self.active_tool, self.gesture.sketch.end) {
            (ActiveTool::Polygon, _) => {
                let direction = cursor - center;
                self.polygon_corners(center, length(direction), direction)
            }
            (ActiveTool::Circle, _) | (ActiveTool::Ellipse, None) => {
                let radius = length(self.drawing_local_delta(center, cursor));
                ellipse(radius, radius, 0.0)
            }
            (ActiveTool::Ellipse, Some(major_end)) => {
                let (radius_x, radius_y, turn) = self.ellipse_axes(center, major_end, cursor)?;
                ellipse(radius_x, radius_y, turn)
            }
            (ActiveTool::Spline, _) => {
                let mut points = self.spline_local_points();
                let next = self.drawing_local_delta(center, cursor);
                if points.last().is_some_and(|last| {
                    (next.x - last[0]).hypot(next.y - last[1]) > limits::MIN_LENGTH_MM
                }) {
                    points.push([next.x, next.y]);
                }
                closed_spline_preview(&points)
            }
            _ => return None,
        };
        Some(
            corners
                .into_iter()
                .map(|[x, y]| self.drawing_world_delta(center, Vec3::new(x, y, 0.0)))
                .collect(),
        )
    }

    /// The newest drawn profile: its segments in the profile's own plane and the
    /// placement of that plane in the world.
    #[must_use]
    pub fn latest_profile(&self) -> Option<(Transform, Vec<ProfileSegment>)> {
        let snapshot = self.document.current();
        snapshot
            .occurrences()
            .filter_map(|occurrence| {
                let definition = snapshot.definition(occurrence.definition_id())?;
                definition.feature_ids().iter().find_map(|feature_id| {
                    let FeatureKind::Profile { segments, .. } =
                        snapshot.feature(*feature_id)?.kind()
                    else {
                        return None;
                    };
                    Some((occurrence.id(), occurrence.transform(), segments.clone()))
                })
            })
            .max_by_key(|(id, _, _)| *id)
            .map(|(_, transform, segments)| (transform, segments))
    }

    /// Number of canonical closed two-arc Circle profiles in the document.
    #[must_use]
    pub fn circle_profile_count(&self) -> usize {
        self.document
            .current()
            .features()
            .filter(|feature| {
                let FeatureKind::Profile { segments, closed } = feature.kind() else {
                    return false;
                };
                exact_circle_geometry(segments, *closed).is_some()
            })
            .count()
    }

    /// Centre and radius of the newest canonical Circle occurrence.
    #[must_use]
    pub fn latest_circle_geometry(&self) -> Option<(Vec3, f64)> {
        let snapshot = self.document.current();
        snapshot
            .occurrences()
            .filter_map(|occurrence| {
                let definition = snapshot.definition(occurrence.definition_id())?;
                let (center, radius) = definition.feature_ids().iter().find_map(|feature_id| {
                    let FeatureKind::Profile { segments, closed } =
                        snapshot.feature(*feature_id)?.kind()
                    else {
                        return None;
                    };
                    exact_circle_geometry(segments, *closed)
                })?;
                let transform = occurrence.transform();
                let [x, y, z] = transform.transform_point([center[0], center[1], 0.0]);
                Some((occurrence.id(), Vec3::new(x, y, z), radius))
            })
            .max_by_key(|(id, _, _)| *id)
            .map(|(_, center, radius)| (center, radius))
    }

    /// Current non-authoritative endpoint-bulge Arc preview.
    #[must_use]
    pub fn arc_preview_geometry(&self) -> Option<(Vec3, Vec3, Vec3, bool)> {
        let start = self.gesture.sketch.start?;
        (self.active_tool == ActiveTool::Arc)
            .then_some(self.drawing_arc(
                start,
                self.gesture.sketch.end?,
                self.gesture.sketch.cursor?,
            )?)
            .map(|arc| {
                (
                    start,
                    self.drawing_world_delta(start, arc.end),
                    self.drawing_world_delta(start, arc.center),
                    arc.clockwise,
                )
            })
    }

    /// Number of canonical closed Arc-plus-chord profiles in the document.
    #[must_use]
    pub fn arc_profile_count(&self) -> usize {
        self.document
            .current()
            .features()
            .filter(|feature| {
                let FeatureKind::Profile { segments, closed } = feature.kind() else {
                    return false;
                };
                exact_arc_profile_geometry(segments, *closed).is_some()
            })
            .count()
    }

    /// World-space geometry of the newest canonical Arc-plus-chord profile.
    #[must_use]
    pub fn latest_arc_geometry(&self) -> Option<(Vec3, Vec3, Vec3, bool)> {
        let snapshot = self.document.current();
        snapshot
            .occurrences()
            .filter_map(|occurrence| {
                let definition = snapshot.definition(occurrence.definition_id())?;
                let (start, end, center, clockwise) =
                    definition.feature_ids().iter().find_map(|feature_id| {
                        let FeatureKind::Profile { segments, closed } =
                            snapshot.feature(*feature_id)?.kind()
                        else {
                            return None;
                        };
                        exact_arc_profile_geometry(segments, *closed)
                    })?;
                let transform = occurrence.transform();
                let world = |point: [f64; 2]| {
                    let [x, y, z] = transform.transform_point([point[0], point[1], 0.0]);
                    Vec3::new(x, y, z)
                };
                Some((
                    occurrence.id(),
                    world(start),
                    world(end),
                    world(center),
                    clockwise,
                ))
            })
            .max_by_key(|(id, _, _, _, _)| *id)
            .map(|(_, start, end, center, clockwise)| (start, end, center, clockwise))
    }

    /// Whether the keyboard shortcut reference is on screen.
    #[must_use]
    pub const fn shortcuts_visible(&self) -> bool {
        self.panels.shortcuts_open
    }

    pub fn create_closed_polyline(&mut self, points_mm: Vec<[f64; 2]>) -> bool {
        self.create_profile_at(Vec3::ZERO, points_mm)
    }

    pub fn create_closed_segment_profile(
        &mut self,
        origin_mm: Vec3,
        segments: Vec<ProfileSegment>,
    ) -> bool {
        if !is_closed_profile(&segments, true) {
            return false;
        }
        self.create_segment_profile_at(
            Transform::from_translation(origin_mm.x, origin_mm.y, origin_mm.z)
                .expect("validated profile origin is canonical"),
            segments,
            true,
            "model-default-arc",
            "model-arc-profile",
        )
    }

    pub fn create_sweep_inputs(
        &mut self,
        profile_points_mm: Vec<[f64; 2]>,
        path_start_mm: [f64; 2],
        path_end_mm: [f64; 2],
    ) -> bool {
        let snapshot = self.document.current();
        let next_definition = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let next_feature = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let next_occurrence = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let (Some(definition), Some(profile), Some(occurrence)) =
            (next_definition, next_feature, next_occurrence)
        else {
            return false;
        };
        let Some(path) = profile.checked_add(1) else {
            return false;
        };
        let definition_id = DefinitionId(definition);
        let profile_feature_id = FeatureId(profile);
        let path_feature_id = FeatureId(path);
        let occurrence_id = OccurrenceId(occurrence);
        let name = self.catalog.format(
            "model-default-box",
            &BTreeMap::from([("number", definition.to_string())]),
        );
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: name.clone(),
            },
            CanonicalCommand::CreateFeature {
                id: profile_feature_id,
                definition_id,
                name: self.catalog.text("model-default-profile"),
                kind: FeatureKind::polygon(&profile_points_mm),
            },
            CanonicalCommand::CreateFeature {
                id: path_feature_id,
                definition_id,
                name: self.catalog.text("model-sweep-path"),
                kind: FeatureKind::Profile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: path_start_mm,
                        end_mm: path_end_mm,
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: self.catalog.format(
                    "model-default-occurrence",
                    &BTreeMap::from([("name", name)]),
                ),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]);
        if self.apply_batch_with_work_recovery(&batch).is_err() {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        self.status_key = "status-sweep-inputs-selected";
        true
    }

    pub fn create_spatial_sweep(
        &mut self,
        profile_segments: Vec<ProfileSegment>,
        path_segments: Vec<SpatialPathSegment>,
    ) -> bool {
        let snapshot = self.document.current();
        let next_definition = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let next_feature = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let next_occurrence = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let (Some(definition), Some(profile), Some(occurrence)) =
            (next_definition, next_feature, next_occurrence)
        else {
            return false;
        };
        let Some(path) = profile.checked_add(1) else {
            return false;
        };
        let Some(sweep) = path.checked_add(1) else {
            return false;
        };
        let definition_id = DefinitionId(definition);
        let profile_feature_id = FeatureId(profile);
        let path_feature_id = FeatureId(path);
        let sweep_feature_id = FeatureId(sweep);
        let occurrence_id = OccurrenceId(occurrence);
        let name = self.catalog.format(
            "model-default-box",
            &BTreeMap::from([("number", definition.to_string())]),
        );
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name: name.clone(),
            },
            CanonicalCommand::CreateFeature {
                id: profile_feature_id,
                definition_id,
                name: self.catalog.text("model-default-profile"),
                kind: FeatureKind::Profile {
                    segments: profile_segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: path_feature_id,
                definition_id,
                name: self.catalog.text("model-sweep-path"),
                kind: FeatureKind::SpatialPath {
                    segments: path_segments,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep_feature_id,
                definition_id,
                name: self.catalog.text("model-sweep-feature"),
                kind: FeatureKind::sweep(profile_feature_id, path_feature_id),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: self.catalog.format(
                    "model-default-occurrence",
                    &BTreeMap::from([("name", name)]),
                ),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]);
        if self.apply_batch_with_work_recovery(&batch).is_err() {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        self.status_key = "status-sweep-inputs-selected";
        true
    }

    pub fn create_loft_inputs(&mut self, sections: Vec<(Vec<[f64; 2]>, f64)>) -> bool {
        if !(2..=16).contains(&sections.len()) {
            return false;
        }
        let snapshot = self.document.current();
        let Some(definition_id) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(DefinitionId)
        else {
            return false;
        };
        let first_feature = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let next_occurrence = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1);
        let (Some(first_feature), Some(occurrence)) = (first_feature, next_occurrence) else {
            return false;
        };
        let name = self.catalog.format(
            "model-loft-definition",
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let mut commands = vec![CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: name.clone(),
        }];
        let mut loft_sections = Vec::with_capacity(sections.len());
        for (index, (control_points_mm, elevation_mm)) in sections.into_iter().enumerate() {
            let Some(feature_id) = first_feature.checked_add(index as u64).map(FeatureId) else {
                return false;
            };
            commands.push(CanonicalCommand::CreateFeature {
                id: feature_id,
                definition_id,
                name: self.catalog.format(
                    "model-spline-profile",
                    &BTreeMap::from([("number", (index + 1).to_string())]),
                ),
                kind: FeatureKind::closed_spline(&control_points_mm),
            });
            loft_sections.push(LoftSection {
                profile: feature_id,
                elevation_mm,
            });
        }
        let occurrence_id = OccurrenceId(occurrence);
        commands.push(CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: self.catalog.format(
                "model-default-occurrence",
                &BTreeMap::from([("name", name)]),
            ),
            transform: Transform::identity(),
            parent: None,
            tags: Default::default(),
            visible: true,
        });
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.solid_tools.loft_input_sections = Some((definition_id, loft_sections));
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        self.status_key = "status-loft-inputs-selected";
        true
    }

    pub(crate) fn create_segment_profile_at(
        &mut self,
        transform: Transform,
        segments: Vec<ProfileSegment>,
        closed: bool,
        default_name_key: &str,
        profile_name_key: &str,
    ) -> bool {
        let snapshot = self.document.current();
        let Some(definition_id) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(DefinitionId)
        else {
            return false;
        };
        let Some(profile_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let Some(occurrence_id) = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(OccurrenceId)
        else {
            return false;
        };
        let name = self.catalog.format(
            default_name_key,
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let occurrence_name = self.catalog.format(
            "model-default-occurrence",
            &BTreeMap::from([("name", name.clone())]),
        );
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name,
            },
            CanonicalCommand::CreateFeature {
                id: profile_id,
                definition_id,
                name: self.catalog.text(profile_name_key),
                kind: FeatureKind::Profile { segments, closed },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: occurrence_name,
                transform,

                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]);
        if !self.apply_drawing_batch(&batch) {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.push_pull.distance_input.clear();
        self.select_drawn_profile(definition_id, occurrence_id);
        true
    }

    pub(crate) fn create_profile_at(&mut self, origin_mm: Vec3, points_mm: Vec<[f64; 2]>) -> bool {
        let snapshot = self.document.current();
        let Some(definition_id) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(DefinitionId)
        else {
            return false;
        };
        let Some(profile_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let Some(occurrence_id) = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(OccurrenceId)
        else {
            return false;
        };
        let name = self.catalog.format(
            "model-default-box",
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let occurrence_name = self.catalog.format(
            "model-default-occurrence",
            &BTreeMap::from([("name", name.clone())]),
        );
        let batch = CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name,
            },
            CanonicalCommand::CreateFeature {
                id: profile_id,
                definition_id,
                name: self.catalog.text("model-default-profile"),
                kind: FeatureKind::polygon(&points_mm),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: occurrence_name,
                transform: Transform::from_translation(origin_mm.x, origin_mm.y, origin_mm.z)
                    .expect("validated profile origin is canonical"),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]);
        if !self.apply_drawing_batch(&batch) {
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.push_pull.distance_input.clear();
        self.select_drawn_profile(definition_id, occurrence_id);
        self.status_key = "status-sketch-created";
        true
    }

    pub(crate) fn create_box_at(&mut self, origin_mm: Vec3, size_mm: Vec3) -> bool {
        if !size_mm.x.is_finite()
            || !size_mm.y.is_finite()
            || !size_mm.z.is_finite()
            || size_mm.x <= 0.01
            || size_mm.y <= 0.01
            || size_mm.z <= 0.01
        {
            return false;
        }
        let snapshot = self.document.current();
        let definition_id = DefinitionId(
            snapshot
                .definitions()
                .map(|definition| definition.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let first_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            + 1;
        let profile_id = FeatureId(first_feature_id);
        let extrusion_id = FeatureId(first_feature_id + 1);
        let occurrence_id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let name = self.catalog.format(
            "model-default-box",
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let occurrence_name = self.catalog.format(
            "model-default-occurrence",
            &BTreeMap::from([("name", name.clone())]),
        );
        if self
            .apply_batch_with_work_recovery(&create_box_batch(
                definition_id,
                [profile_id, extrusion_id],
                occurrence_id,
                [
                    &name,
                    &self.catalog.text("model-default-profile"),
                    &self.catalog.text("model-default-extrusion"),
                    &occurrence_name,
                ],
                origin_mm,
                size_mm,
            ))
            .is_err()
        {
            return false;
        }
        self.tool_preview.close::<EphemeralBoxPreview>();
        self.push_pull.distance_input.clear();
        self.selection.select_exact(
            SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            false,
        );
        self.status_key = "status-box-created";
        true
    }

    pub fn create_box(&mut self) -> bool {
        let offset = self.active_box_count() as f64 * 35.0;
        self.create_box_at(
            Vec3::new(offset, offset, 0.0),
            Vec3::new(BOX_WIDTH_MM, BOX_DEPTH_MM, 20.0),
        )
    }

    pub(crate) fn cut_source_plan(&self) -> Option<CutSourcePlan> {
        if !self.selection.edit_context.is_empty() {
            return None;
        }
        let CopySourcePlan {
            occurrence_ids,
            occurrence_count,
        } = self.copy_source_plan()?;
        let DeleteSelectionSourcePlan { commands, .. } = self.delete_selection_source_plan()?;
        let deleted_ids = commands
            .iter()
            .map(|command| match command {
                CanonicalCommand::DeleteOccurrence { id } => Some(*id),
                _ => None,
            })
            .collect::<Option<BTreeSet<_>>>()?;
        if occurrence_count != commands.len() || deleted_ids != occurrence_ids {
            return None;
        }
        let snapshot = self.document.current();
        let clipboard = occurrence_ids
            .iter()
            .copied()
            .map(|source_occurrence_id| {
                let occurrence = snapshot.occurrence(source_occurrence_id)?;
                Some(CutClipboardOccurrence {
                    color: occurrence.color(),
                    source_occurrence_id,
                    definition_id: occurrence.definition_id(),
                    transform: occurrence.transform(),
                    parent: occurrence.parent(),
                    tags: occurrence.tags().clone(),
                    visible: occurrence.visible(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(CutSourcePlan {
            source_revision: snapshot.revision_id(),
            occurrence_ids,
            occurrence_count,
            clipboard,
            commands,
        })
    }

    pub(crate) fn apply_cut_source_plan(&mut self, plan: CutSourcePlan) -> bool {
        if plan.source_revision != self.document.current().revision_id()
            || plan.occurrence_count != plan.occurrence_ids.len()
            || plan.clipboard.len() != plan.occurrence_count
            || self.cut_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        let CutSourcePlan {
            occurrence_ids,
            occurrence_count,
            clipboard,
            commands,
            ..
        } = plan;
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.clipboard.occurrences = occurrence_ids.into_iter().collect();
        self.clipboard.cut_occurrences = clipboard;
        self.selection.clear();
        self.tool_preview.close::<EphemeralBoxPreview>();
        self.status_key = "status-object-deleted";
        self.digest = self.catalog.format(
            "digest-cut-to-clipboard",
            &BTreeMap::from([("count", occurrence_count.to_string())]),
        );
        true
    }

    pub fn start_preview(&mut self) -> bool {
        self.start_preview_for(SmartPushPullPlanning::Append)
    }

    pub(crate) fn start_preview_for(&mut self, planning: SmartPushPullPlanning) -> bool {
        self.status_key = "error-preview-stale";
        self.digest = self.catalog.text("error-preview-stale");
        if self.exact.mutation_readiness != MutationReadiness::Ready {
            return false;
        }
        let prepared = self.try_start_preview_for(planning);
        if !prepared {
            self.clear_push_pull_preview();
        }
        prepared
    }

    pub(crate) fn try_start_preview_for(&mut self, planning: SmartPushPullPlanning) -> bool {
        self.push_pull.smart_planning = Some(planning.clone());
        let Some(selection) = self.selection.primary.clone() else {
            self.clear_ephemeral_edit_state();
            self.status_key = "error-push-pull-selection-required";
            self.digest = self.catalog.text("error-push-pull-selection-required");
            return false;
        };
        if !self.occurrence_in_active_context(&selection.instance_path) {
            return false;
        }
        let Some(distance_mm) = parse_distance_mm(&self.push_pull.distance_input) else {
            return false;
        };
        if let Some(failure) = self.face_workflow.take_headless_failure() {
            self.clear_ephemeral_edit_state();
            self.status_key = "error-preview-stale";
            self.digest = match failure {
                HeadlessFaceWorkflowFailure::FailedEvaluation => {
                    "Headless exact evaluation failed; last-valid output preserved".to_owned()
                }
                HeadlessFaceWorkflowFailure::Ambiguous => {
                    "Headless face target is Ambiguous; mutation refused".to_owned()
                }
                HeadlessFaceWorkflowFailure::Lost => {
                    "Headless face target is Lost; mutation refused".to_owned()
                }
            };
            return false;
        }
        if let Some(prepared) = self.prepare_drawn_shape_preview(&selection, distance_mm) {
            return prepared;
        }
        let planning_snapshot = self.push_pull_planning_snapshot();
        let Some(source) = self.push_pull_source_plan(&selection) else {
            return false;
        };
        let item = source.target_box;
        if distance_mm < -0.01
            && item.extrusion_feature_id.is_none()
            && planning_snapshot
                .feature(item.profile_feature_id)
                .is_some_and(|feature| {
                    // Pulling below a profile is planned from its polygon.
                    matches!(feature.kind(), FeatureKind::Profile { .. })
                        && feature.kind().polygon_points().is_none()
                })
        {
            return false;
        }
        self.prepare_box_push_pull_preview(
            selection,
            item,
            distance_mm,
            ProposalPrincipal::ManualClient,
        )
    }

    pub(crate) fn has_preview(&self) -> bool {
        let Some(preview) = self.tool_preview.get::<EphemeralBoxPreview>() else {
            return false;
        };
        if self.push_pull.distance_input != preview.plan.distance_expression
            || parse_distance_mm(&self.push_pull.distance_input).map(f64::to_bits)
                != Some(preview.plan.distance_mm_bits)
            || self.selection.primary.as_ref() != Some(&preview.plan.source.target)
        {
            return false;
        }
        let snapshot = self.document.current();
        // Called several times a frame: compare in place, build a key only on a miss.
        let cached = self
            .push_pull
            .preview_check
            .borrow()
            .as_ref()
            .and_then(|(cached, digest)| {
                self.preview_check_is(cached, &snapshot, preview)
                    .then(|| digest.clone())
            });
        let digest = match cached {
            Some(digest) => digest,
            None => {
                let key = self.preview_check_key(&snapshot, preview);
                let digest = self
                    .derive_push_pull_preview_plan(
                        &preview.plan.source,
                        preview.plan.principal,
                        &preview.plan.distance_expression,
                        f64::from_bits(preview.plan.distance_mm_bits),
                    )
                    .filter(|(plan, batch, candidate)| {
                        *plan == preview.plan
                            && *batch == preview.batch
                            && candidate.is_current(&snapshot)
                    })
                    .map(|(_, _, candidate)| candidate.command_digest().to_owned());
                *self.push_pull.preview_check.borrow_mut() = Some((key, digest.clone()));
                digest
            }
        };
        digest.is_some_and(|digest| {
            self.push_pull
                .smart_proposal
                .as_ref()
                .is_some_and(|proposal| {
                    proposal.is_current(&snapshot)
                        && proposal.batch() == &preview.batch
                        && proposal.command_digest() == digest
                })
        })
    }

    /// `key == self.preview_check_key(snapshot, preview)` without cloning.
    fn preview_check_is(
        &self,
        key: &PreviewCheckKey,
        snapshot: &Snapshot,
        preview: &EphemeralBoxPreview,
    ) -> bool {
        // Exhaustive, so a field added to the key cannot be left out here.
        let PreviewCheckKey {
            document_id,
            revision_id,
            canonical_digest,
            planning,
            exact_results_stamps,
            primary,
            selected_group,
            edit_context,
            topological,
            preview: key_preview,
        } = key;
        *document_id == snapshot.document_id()
            && *revision_id == snapshot.revision_id()
            && *exact_results_stamps
                == (
                    self.exact.results.contents_stamp(),
                    self.exact.topology_results.contents_stamp(),
                )
            && *primary == self.selection.primary
            && *selected_group == self.selection.selected_group
            && *edit_context == self.selection.edit_context
            && *topological == self.selection.topological
            && key_preview == preview
            && *planning == self.push_pull_planning_plan()
            && *canonical_digest == snapshot.canonical_digest()
    }

    pub(crate) fn preview_check_key(
        &self,
        snapshot: &Snapshot,
        preview: &EphemeralBoxPreview,
    ) -> PreviewCheckKey {
        PreviewCheckKey {
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            planning: self.push_pull_planning_plan(),
            exact_results_stamps: (
                self.exact.results.contents_stamp(),
                self.exact.topology_results.contents_stamp(),
            ),
            primary: self.selection.primary.clone(),
            selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            topological: self.selection.topological.clone(),
            preview: preview.clone(),
        }
    }

    pub fn cancel_preview(&mut self) {
        self.clear_ephemeral_edit_state();
        self.status_key = "status-ready";
    }

    pub fn confirm_preview(&mut self) -> bool {
        if let Some(confirmed) = self.confirm_drawn_shape_preview() {
            return confirmed;
        }
        if self.push_pull.face_offset_evaluation.is_some() {
            return self.confirm_face_offset_preview();
        }
        if self.has_preview() && self.preview_requires_face_offset_evaluation() {
            if !self.request_face_offset_confirmation() {
                self.status_key = "error-preview-stale";
            }
            return false;
        }
        if !self.has_preview() {
            self.tool_preview.close::<EphemeralBoxPreview>();
            self.push_pull.smart_proposal = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.tool_preview.get::<EphemeralBoxPreview>().cloned() else {
            return false;
        };
        let Some((plan, batch, proposal)) = self.derive_push_pull_preview_plan(
            &preview.plan.source,
            preview.plan.principal,
            &preview.plan.distance_expression,
            f64::from_bits(preview.plan.distance_mm_bits),
        ) else {
            return false;
        };
        let rule_program = match self.rewrite_program_push_pull(
            &preview.plan.source,
            f64::from_bits(preview.plan.distance_mm_bits),
        ) {
            Ok(source) => source,
            Err(error) => {
                self.digest = error.reason_text().to_owned();
                return false;
            }
        };
        if plan != preview.plan
            || batch != preview.batch
            || self
                .complete_mutation_with_work_recovery(move |document| {
                    proposal
                        .commit(document)
                        .map_err(|error| failed("push_pull.commit", error))?;
                    rule_program
                        .map_or(Ok(()), |source| document.bind_rule_program(source))
                        .map_err(|error| failed("push_pull.bind_program", error))
                })
                .is_err()
        {
            self.tool_preview.close::<EphemeralBoxPreview>();
            self.push_pull.smart_proposal = None;
            self.status_key = "error-preview-stale";
            return false;
        }
        let committed_extent = face_extent(
            &preview.plan.preview_box,
            Some(&preview.plan.source.target.element),
        );
        self.tool_preview.close::<EphemeralBoxPreview>();
        self.push_pull.smart_proposal = None;
        self.status_key = "status-ready";
        if let Some(selection) = self.selection.primary.clone() {
            self.push_pull.last = Some(LastPushPull {
                selection: selection.clone(),
                revision: self.document_revision(),
                canonical_digest: self.canonical_digest(),
            });
            self.digest = self.catalog.format(
                match selection.element {
                    ElementId::Face { axis: Axis::Z, .. } => "digest-push-pull-committed-height",
                    _ => "digest-push-pull-committed-profile",
                },
                &BTreeMap::from([
                    (
                        "distance",
                        parse_distance_mm(&self.push_pull.distance_input)
                            .map_or_else(String::new, format_signed_mm),
                    ),
                    (
                        "height",
                        committed_extent.map_or_else(String::new, format_height),
                    ),
                ]),
            );
        }
        true
    }

    #[must_use]
    pub fn preview_action_digest(&self) -> Option<String> {
        if !self.has_preview() {
            return None;
        }
        let selection = self.selection.primary.clone()?;
        let snapshot = self.document.current();
        let resolved = snapshot
            .resolve_instance_path(&selection.instance_path)
            .ok()?;
        if resolved.definition_id != selection.definition_id {
            return None;
        }
        let definition = snapshot.definition(resolved.definition_id)?;
        if matches!(selection.element, ElementId::TopologicalFace(_)) {
            return Some(format!(
                "{}: {}",
                definition.name(),
                format_signed_mm(parse_distance_mm(&self.push_pull.distance_input)?)
            ));
        }
        let from = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)
            .and_then(|item| face_extent(&item, Some(&selection.element)))?;
        let to = self
            .tool_preview
            .get::<EphemeralBoxPreview>()
            .and_then(|item| face_extent(&item.plan.preview_box, Some(&selection.element)))?;
        Some(self.catalog.format(
            "action-smart-push-pull-height",
            &BTreeMap::from([
                ("feature", definition.name().to_owned()),
                ("from", format_height(from)),
                ("to", format_height(to)),
            ]),
        ))
    }

    /// Visible open line-profile segments in world space.
    #[must_use]
    pub fn open_profile_line_segments(&self) -> Vec<(SelectionId, [Vec3; 2])> {
        let snapshot = self.document.current();
        snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .flat_map(|occurrence| {
                let Some(definition) = snapshot.definition(occurrence.definition_id) else {
                    return Vec::new();
                };
                definition
                    .feature_ids()
                    .iter()
                    .filter_map(|feature_id| snapshot.feature(*feature_id))
                    .filter_map(|feature| match feature.kind() {
                        FeatureKind::Profile {
                            segments,
                            closed: false,
                        } => Some(segments),
                        _ => None,
                    })
                    .flat_map(|segments| {
                        segments.iter().enumerate().filter_map(|(index, segment)| {
                            let ProfileSegment::Line { start_mm, end_mm } = segment else {
                                return None;
                            };
                            Some((
                                SelectionId {
                                    definition_id: occurrence.definition_id,
                                    instance_path: occurrence.instance_path.clone(),
                                    element: ElementId::Edge(index.try_into().ok()?),
                                },
                                [
                                    transform_model_point(
                                        occurrence.transform,
                                        Vec3::new(start_mm[0], start_mm[1], 0.0),
                                    ),
                                    transform_model_point(
                                        occurrence.transform,
                                        Vec3::new(end_mm[0], end_mm[1], 0.0),
                                    ),
                                ],
                            ))
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Whether the value box currently owns the keyboard, either because it
    /// already has focus or because a keystroke this frame asked for it.
    pub(crate) fn value_box_is_being_typed_into(&self, context: &egui::Context) -> bool {
        self.value_box.focus || context.wants_keyboard_input()
    }

    pub(crate) fn proxy_preview_is_active(&self, item: &RenderBox) -> bool {
        let push_pull_preview = self.has_preview()
            && self.push_pull.face_offset_evaluation.is_none()
            && self
                .tool_preview
                .get::<EphemeralBoxPreview>()
                .is_some_and(|preview| preview.plan.source.topological_reference.is_none())
            && self.push_pull_preview_definition() == Some(item.definition_id);
        // Asked for every box of the scene: the cheap path test first, so the
        // transform plan is only prepared for the boxes being moved.
        let move_preview = self.move_session().is_some_and(|(drag, _)| {
            (drag.copy || drag.profile_target.is_none())
                && self.move_drag_applies_to_path(drag, &item.instance_path)
                && self.move_preview_is_current(drag)
        });

        let occurrence_preview = self.has_occurrence_operation_preview()
            && item.instance_path.is_root()
            && self
                .tool_preview
                .get::<OccurrenceOperationPreview>()
                .is_some_and(|preview| {
                    preview
                        .boxes
                        .contains_key(&item.instance_path.root_occurrence())
                });
        push_pull_preview || move_preview || occurrence_preview
    }

    pub(crate) fn cancel_rectangle_sketch(&mut self) {
        self.gesture.sketch.armed = false;
        self.gesture.sketch.start = None;
        self.gesture.sketch.end = None;
        self.gesture.sketch.cursor = None;
        self.gesture.sketch.chain_origin = None;
        self.gesture.sketch.chain_points.clear();
        self.gesture.sketch.chain_items.clear();
        self.gesture.sketch.axis_lock = None;
        self.status_key = "status-ready";
    }

    pub(crate) fn complete_line_sketch(&mut self, start: Vec3, end: Vec3) -> bool {
        if let Some(origin) = self.gesture.sketch.chain_origin {
            let close_distance = length(Vec3::new(
                end.x - origin.x,
                end.y - origin.y,
                end.z - origin.z,
            ));
            if close_distance.is_finite() && close_distance <= 0.1 {
                return self.gesture.sketch.chain_points.len() >= 3 && self.close_line_chain();
            }
        }

        let length_mm = length(end - start);
        if !length_mm.is_finite() || length_mm <= 0.01 {
            return false;
        }
        let Some((transform, points)) = line_geometry::planar_points(&[start, end]) else {
            return false;
        };
        let created = self.create_segment_profile_at(
            transform,
            vec![ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [points[1].x, points[1].y],
            }],
            false,
            "model-default-line",
            "model-line-profile",
        );
        if created {
            if let Some(selection) = self.selection.primary.as_ref() {
                self.gesture.sketch.chain_items.push((
                    selection.definition_id,
                    selection.instance_path.root_occurrence(),
                ));
            }
            self.gesture.sketch.chain_points.push(end);
            self.gesture.sketch.armed = true;
            self.gesture.sketch.start = Some(end);
            self.gesture.sketch.cursor = Some(end);
            self.value_box.input.clear();
            self.status_key = "status-line-end";
            self.digest = self.catalog.format(
                "digest-exact-line",
                &BTreeMap::from([("length", format_height(length_mm))]),
            );
        }
        created
    }

    pub(crate) fn close_line_chain(&mut self) -> bool {
        let Some((transform, points)) =
            line_geometry::planar_points(&self.gesture.sketch.chain_points)
        else {
            return false;
        };
        let mut twice_area = 0.0;
        for edge in points.windows(2) {
            twice_area += edge[0].x * edge[1].y - edge[1].x * edge[0].y;
        }
        if let Some(last) = points.last() {
            twice_area += last.x * points[0].y - points[0].x * last.y;
        }
        if !twice_area.is_finite() || twice_area.abs() <= APPROXIMATION {
            return false;
        }

        let mut segments = points
            .as_slice()
            .windows(2)
            .map(|edge| ProfileSegment::Line {
                start_mm: [edge[0].x, edge[0].y],
                end_mm: [edge[1].x, edge[1].y],
            })
            .collect::<Vec<_>>();
        let last = *points.last().expect("validated line chain");
        segments.push(ProfileSegment::Line {
            start_mm: [last.x, last.y],
            end_mm: [0.0, 0.0],
        });

        let snapshot = self.document.current();
        if self
            .gesture
            .sketch
            .chain_items
            .iter()
            .any(|(definition_id, occurrence_id)| {
                snapshot
                    .occurrence(*occurrence_id)
                    .is_none_or(|occurrence| occurrence.definition_id() != *definition_id)
            })
        {
            return false;
        }
        let Some(definition_id) = snapshot
            .definitions()
            .map(|definition| definition.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(DefinitionId)
        else {
            return false;
        };
        let Some(profile_id) = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(FeatureId)
        else {
            return false;
        };
        let Some(occurrence_id) = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(OccurrenceId)
        else {
            return false;
        };
        let name = self.catalog.format(
            "model-default-line",
            &BTreeMap::from([("number", definition_id.0.to_string())]),
        );
        let occurrence_name = self.catalog.format(
            "model-default-occurrence",
            &BTreeMap::from([("name", name.clone())]),
        );
        let mut commands = self
            .gesture
            .sketch
            .chain_items
            .iter()
            .map(|(_, occurrence_id)| CanonicalCommand::DeleteOccurrence { id: *occurrence_id })
            .collect::<Vec<_>>();
        commands.extend(
            self.gesture
                .sketch
                .chain_items
                .iter()
                .map(|(definition_id, _)| CanonicalCommand::DeleteDefinition {
                    id: *definition_id,
                }),
        );
        commands.extend([
            CanonicalCommand::CreateDefinition {
                id: definition_id,
                name,
            },
            CanonicalCommand::CreateFeature {
                id: profile_id,
                definition_id,
                name: self.catalog.text("model-line-profile"),
                kind: FeatureKind::Profile {
                    segments,
                    closed: true,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id,
                name: occurrence_name,
                transform,

                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]);
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        let segment_count = self.gesture.sketch.chain_points.len();
        self.clear_ephemeral_edit_state();
        self.gesture.sketch.armed = false;
        self.gesture.sketch.start = None;
        self.gesture.sketch.cursor = None;
        self.gesture.sketch.chain_origin = None;
        self.gesture.sketch.chain_points.clear();
        self.gesture.sketch.chain_items.clear();
        self.value_box.input.clear();
        self.select_drawn_profile(definition_id, occurrence_id);
        self.status_key = "status-line-closed";
        self.digest = self.catalog.format(
            "digest-line-closed",
            &BTreeMap::from([("count", segment_count.to_string())]),
        );
        true
    }

    pub(crate) fn complete_circle_sketch(&mut self, center: Vec3, radial_point: Vec3) -> bool {
        let direction = radial_point - center;
        self.complete_circle(center, length(direction), direction)
    }

    pub(crate) fn complete_circle(
        &mut self,
        center: Vec3,
        radius_mm: f64,
        direction: Vec3,
    ) -> bool {
        if !radius_mm.is_finite() || radius_mm <= 0.01 {
            return false;
        }
        let direction = self.drawing_local_delta(center, center + direction);
        let direction_length = length(direction);
        let unit = if direction_length > 0.01 {
            Vec3::new(
                direction.x / direction_length,
                direction.y / direction_length,
                0.0,
            )
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let radial = [unit.x * radius_mm, unit.y * radius_mm];
        let opposite = [-radial[0], -radial[1]];
        let created = self.create_segment_profile_at(
            self.drawing_transform(center),
            vec![
                ProfileSegment::CircularArc {
                    start_mm: radial,
                    end_mm: opposite,
                    center_mm: [0.0, 0.0],
                    clockwise: false,
                },
                ProfileSegment::CircularArc {
                    start_mm: opposite,
                    end_mm: radial,
                    center_mm: [0.0, 0.0],
                    clockwise: false,
                },
            ],
            true,
            "model-default-circle",
            "model-circle-profile",
        );
        if created {
            self.gesture.sketch.armed = self.uses_drawing_plane();
            self.gesture.sketch.start = None;
            self.gesture.sketch.cursor = None;
            self.value_box.input = format_height(radius_mm);
            self.status_key = "status-circle-created";
            self.digest = self.catalog.format(
                "digest-exact-circle",
                &BTreeMap::from([("radius", format_height(radius_mm))]),
            );
        }
        created
    }

    /// Places the next point of the shape being drawn: the first point starts
    /// it, later points finish it or fix one more of its dimensions.
    pub(crate) fn place_sketch_point(&mut self, point: Vec3) {
        let Some(start) = self.gesture.sketch.start else {
            self.gesture.sketch.start = Some(point);
            self.gesture.sketch.cursor = Some(point);
            if self.active_tool == ActiveTool::Line {
                self.gesture.sketch.chain_origin = Some(point);
            }
            if matches!(self.active_tool, ActiveTool::Line | ActiveTool::Spline) {
                self.gesture.sketch.chain_points.clear();
                self.gesture.sketch.chain_points.push(point);
                self.gesture.sketch.chain_items.clear();
            }
            self.value_box.input.clear();
            self.status_key = match self.active_tool {
                ActiveTool::Line => "status-line-end",
                ActiveTool::Spline => "status-spline-next",
                ActiveTool::Circle => "status-circle-radius",
                ActiveTool::Polygon => "status-polygon-corner",
                ActiveTool::Ellipse => "status-ellipse-major",
                ActiveTool::Arc => "status-arc-end",
                _ => "status-sketch-second-point",
            };
            return;
        };
        match (self.active_tool, self.gesture.sketch.end) {
            (ActiveTool::Line, _) => {
                self.complete_line_sketch(start, point);
            }
            (ActiveTool::Circle, _) => {
                self.complete_circle_sketch(start, point);
            }
            (ActiveTool::Polygon, _) => {
                self.complete_polygon_sketch(start, point);
            }
            (ActiveTool::Spline, _) => {
                self.add_spline_point(point);
            }
            (ActiveTool::Arc, Some(end)) => {
                self.complete_arc_sketch(start, end, point);
            }
            (ActiveTool::Ellipse, Some(end)) => {
                self.complete_ellipse_sketch(start, end, point);
            }
            (ActiveTool::Arc | ActiveTool::Ellipse, None) => {
                if length(point - start) > limits::MIN_LENGTH_MM {
                    self.gesture.sketch.end = Some(point);
                    self.gesture.sketch.cursor = Some(point);
                    self.value_box.input.clear();
                    self.status_key = if self.active_tool == ActiveTool::Arc {
                        "status-arc-bulge"
                    } else {
                        "status-ellipse-minor"
                    };
                }
            }
            _ => {
                self.complete_rectangle_sketch(start, point);
            }
        }
    }

    /// Adds the next point the spline being drawn passes through; its first
    /// point again closes the spline.
    pub(crate) fn add_spline_point(&mut self, point: Vec3) -> bool {
        let points = &self.gesture.sketch.chain_points;
        if points
            .first()
            .is_some_and(|first| length(point - *first) <= limits::MIN_LENGTH_MM)
        {
            return self.complete_spline();
        }
        if points
            .last()
            .is_none_or(|last| length(point - *last) <= limits::MIN_LENGTH_MM)
        {
            return false;
        }
        self.gesture.sketch.chain_points.push(point);
        self.gesture.sketch.cursor = Some(point);
        self.value_box.input.clear();
        self.status_key = if self.gesture.sketch.chain_points.len() >= SPLINE_MIN_POINTS {
            "status-spline-close"
        } else {
            "status-spline-next"
        };
        true
    }

    /// Placed spline points in the drawing plane of the first one, relative to it.
    fn spline_local_points(&self) -> Vec<[f64; 2]> {
        let Some(&origin) = self.gesture.sketch.chain_points.first() else {
            return Vec::new();
        };
        self.gesture
            .sketch
            .chain_points
            .iter()
            .map(|point| {
                let local = self.drawing_local_delta(origin, *point);
                [local.x, local.y]
            })
            .collect()
    }

    /// Closes the spline being drawn through its placed points as one exact
    /// profile; `false` while it has too few points.
    pub(crate) fn complete_spline(&mut self) -> bool {
        let points = self.spline_local_points();
        let Some(&origin) = self.gesture.sketch.chain_points.first() else {
            return false;
        };
        if points.len() < SPLINE_MIN_POINTS {
            return false;
        }
        let created = self.create_segment_profile_at(
            self.drawing_transform(origin),
            vec![ProfileSegment::Spline {
                points_mm: points.iter().chain(points.first()).copied().collect(),
            }],
            true,
            "model-default-spline",
            "model-spline-curve",
        );
        if created {
            self.gesture.sketch.armed = self.uses_drawing_plane();
            self.gesture.sketch.start = None;
            self.gesture.sketch.cursor = None;
            self.gesture.sketch.chain_points.clear();
            self.value_box.input.clear();
            self.status_key = "status-spline-created";
            self.digest = self.catalog.format(
                "digest-exact-spline",
                &BTreeMap::from([("count", points.len().to_string())]),
            );
        }
        created
    }

    /// Half-axes and turn of the ellipse centred on `center` whose first
    /// half-axis ends at `major_end` and whose second one reaches `minor_point`'s
    /// distance from the first, all in the drawing plane.
    pub(crate) fn ellipse_axes(
        &self,
        center: Vec3,
        major_end: Vec3,
        minor_point: Vec3,
    ) -> Option<(f64, f64, f64)> {
        let major = self.drawing_local_delta(center, major_end);
        let radius_x = length(major);
        if radius_x <= limits::MIN_LENGTH_MM {
            return None;
        }
        let minor = self.drawing_local_delta(center, minor_point);
        let radius_y = (major.x * minor.y - major.y * minor.x).abs() / radius_x;
        (radius_y > limits::MIN_LENGTH_MM).then(|| (radius_x, radius_y, major.y.atan2(major.x)))
    }

    pub(crate) fn complete_ellipse_sketch(
        &mut self,
        center: Vec3,
        major_end: Vec3,
        minor_point: Vec3,
    ) -> bool {
        self.ellipse_axes(center, major_end, minor_point)
            .is_some_and(|(radius_x, radius_y, turn)| {
                self.complete_ellipse(center, radius_x, radius_y, turn)
            })
    }

    pub(crate) fn complete_ellipse(
        &mut self,
        center: Vec3,
        radius_x_mm: f64,
        radius_y_mm: f64,
        turn_radians: f64,
    ) -> bool {
        if !(radius_x_mm.is_finite() && radius_y_mm.is_finite())
            || radius_x_mm.min(radius_y_mm) <= limits::MIN_LENGTH_MM
        {
            return false;
        }
        let created = self.create_segment_profile_at(
            self.drawing_transform(center),
            ellipse_segments([0.0, 0.0], radius_x_mm, radius_y_mm, turn_radians),
            true,
            "model-default-ellipse",
            "model-ellipse-profile",
        );
        if created {
            self.gesture.sketch.armed = self.uses_drawing_plane();
            self.gesture.sketch.start = None;
            self.gesture.sketch.end = None;
            self.gesture.sketch.cursor = None;
            self.value_box.input = format_height(radius_y_mm);
            self.status_key = "status-ellipse-created";
            self.digest = self.catalog.format(
                "digest-exact-ellipse",
                &BTreeMap::from([
                    ("radius_x", format_height(radius_x_mm)),
                    ("radius_y", format_height(radius_y_mm)),
                ]),
            );
        }
        created
    }

    pub(crate) fn complete_polygon_sketch(&mut self, center: Vec3, corner: Vec3) -> bool {
        let direction = corner - center;
        self.complete_polygon(center, length(direction), direction)
    }

    /// Corners of the polygon about `center` in the drawing plane, the first one
    /// along `direction`.
    pub(crate) fn polygon_corners(
        &self,
        center: Vec3,
        radius_mm: f64,
        direction: Vec3,
    ) -> Vec<[f64; 2]> {
        let local = self.drawing_local_delta(center, center + direction);
        let angle = if length(local) > limits::MIN_LENGTH_MM {
            local.y.atan2(local.x)
        } else {
            0.0
        };
        regular_polygon_points(self.gesture.sketch.polygon_sides(), radius_mm, angle)
    }

    pub(crate) fn complete_polygon(
        &mut self,
        center: Vec3,
        radius_mm: f64,
        direction: Vec3,
    ) -> bool {
        if !radius_mm.is_finite() || radius_mm <= limits::MIN_LENGTH_MM {
            return false;
        }
        let sides = self.gesture.sketch.polygon_sides();
        let corners = self.polygon_corners(center, radius_mm, direction);
        let created = self.create_segment_profile_at(
            self.drawing_transform(center),
            polygon_segments(&corners),
            true,
            "model-default-polygon",
            "model-polygon-profile",
        );
        if created {
            self.gesture.sketch.armed = self.uses_drawing_plane();
            self.gesture.sketch.start = None;
            self.gesture.sketch.cursor = None;
            self.value_box.input = sides.to_string();
            self.status_key = "status-polygon-created";
            self.digest = self.catalog.format(
                "digest-exact-polygon",
                &BTreeMap::from([
                    ("sides", sides.to_string()),
                    ("radius", format_height(radius_mm)),
                ]),
            );
        }
        created
    }

    /// Reads the corner count typed before the polygon centre is placed.
    pub(crate) fn set_polygon_sides_from_value_box(&mut self) -> bool {
        let Some(sides) = self
            .value_box
            .input
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|sides| (3..=limits::PATH_SEGMENTS).contains(sides))
        else {
            self.digest = self.catalog.format(
                "digest-polygon-invalid-sides",
                &BTreeMap::from([("max", limits::PATH_SEGMENTS.to_string())]),
            );
            return false;
        };
        self.gesture.sketch.polygon_sides = Some(sides);
        self.digest = self.catalog.format(
            "digest-polygon-sides",
            &BTreeMap::from([("sides", sides.to_string())]),
        );
        true
    }

    pub(crate) fn complete_arc_sketch(
        &mut self,
        start: Vec3,
        end: Vec3,
        bulge_point: Vec3,
    ) -> bool {
        let Some(arc) = self.drawing_arc(start, end, bulge_point) else {
            return false;
        };
        let local_end = [arc.end.x, arc.end.y];
        let local_center = [arc.center.x, arc.center.y];
        let created = self.create_segment_profile_at(
            self.drawing_transform(start),
            vec![
                ProfileSegment::CircularArc {
                    start_mm: [0.0, 0.0],
                    end_mm: local_end,
                    center_mm: local_center,
                    clockwise: arc.clockwise,
                },
                ProfileSegment::Line {
                    start_mm: local_end,
                    end_mm: [0.0, 0.0],
                },
            ],
            true,
            "model-default-arc",
            "model-arc-profile",
        );
        if created {
            let bulge_mm = self.drawing_bulge(start, end, bulge_point).abs();
            self.gesture.sketch.armed = self.uses_drawing_plane();
            self.gesture.sketch.start = None;
            self.gesture.sketch.end = None;
            self.gesture.sketch.cursor = None;
            self.value_box.input = format_height(bulge_mm);
            self.status_key = "status-arc-created";
            self.digest = self.catalog.format(
                "digest-exact-arc",
                &BTreeMap::from([("bulge", format_height(bulge_mm))]),
            );
        }
        created
    }

    pub(crate) fn complete_datum_rectangle(&mut self, start: Vec3, end: Vec3) -> bool {
        let Some(batch) = self.datum_rectangle_batch(start, end) else {
            return false;
        };
        let frame = self.drawing_frame(Some(start));
        let frame_origin = Vec3::new(frame.origin_mm[0], frame.origin_mm[1], frame.origin_mm[2]);
        let frame_x = Vec3::new(frame.x_axis[0], frame.x_axis[1], frame.x_axis[2]);
        let frame_y = Vec3::new(frame.y_axis[0], frame.y_axis[1], frame.y_axis[2]);
        let start_uv = [
            dot(start - frame_origin, frame_x),
            dot(start - frame_origin, frame_y),
        ];
        let end_uv = [
            dot(end - frame_origin, frame_x),
            dot(end - frame_origin, frame_y),
        ];
        let width_mm = (end_uv[0] - start_uv[0]).abs();
        let depth_mm = (end_uv[1] - start_uv[1]).abs();
        if self.apply_batch_with_work_recovery(&batch).is_err() {
            self.digest = self.catalog.text("error-preview-stale");
            return false;
        }
        self.clear_ephemeral_edit_state();
        self.gesture.sketch.armed = self.uses_drawing_plane();
        self.gesture.sketch.start = None;
        self.gesture.sketch.cursor = None;
        self.value_box.input.clear();
        self.status_key = "status-sketch-first-point";
        self.digest = self.catalog.format(
            "digest-exact-rectangle",
            &BTreeMap::from([
                ("width", format_height(width_mm)),
                ("depth", format_height(depth_mm)),
            ]),
        );
        true
    }

    pub(crate) fn complete_rectangle_sketch(&mut self, start: Vec3, end: Vec3) -> bool {
        if self.face_workflow_datum() != PrincipalPlane::Xy {
            return self.complete_datum_rectangle(start, end);
        }
        let frame = self.drawing_frame(Some(start));
        let frame_origin = Vec3::new(frame.origin_mm[0], frame.origin_mm[1], frame.origin_mm[2]);
        let frame_x = Vec3::new(frame.x_axis[0], frame.x_axis[1], frame.x_axis[2]);
        let frame_y = Vec3::new(frame.y_axis[0], frame.y_axis[1], frame.y_axis[2]);
        let start_uv = [
            dot(start - frame_origin, frame_x),
            dot(start - frame_origin, frame_y),
        ];
        let end_uv = [
            dot(end - frame_origin, frame_x),
            dot(end - frame_origin, frame_y),
        ];
        let origin_uv = [start_uv[0].min(end_uv[0]), start_uv[1].min(end_uv[1])];
        let origin = frame_origin + frame_x * origin_uv[0] + frame_y * origin_uv[1];
        let size = Vec3::new(
            (end_uv[0] - start_uv[0]).abs(),
            (end_uv[1] - start_uv[1]).abs(),
            0.0,
        );
        let created = self.create_profile_at(
            origin,
            vec![[0.0, 0.0], [size.x, 0.0], [size.x, size.y], [0.0, size.y]],
        );
        if created {
            self.gesture.sketch.armed = self.uses_drawing_plane();
            self.gesture.sketch.start = None;
            self.gesture.sketch.cursor = None;
            self.value_box.input.clear();
            self.status_key = "status-sketch-first-point";
            self.digest = self.catalog.format(
                "digest-exact-rectangle",
                &BTreeMap::from([
                    ("width", format_height(size.x)),
                    ("depth", format_height(size.y)),
                ]),
            );
        }
        created
    }

    pub(crate) fn drawing_frame(&self, point: Option<Vec3>) -> WorkplaneFrame {
        let plane = self.face_workflow_datum();
        let mut frame = WorkplaneFrame::principal(plane);
        if let Some(point) = point {
            let n = frame.normal;
            let offset = dot(point, Vec3::new(n[0], n[1], n[2]));
            frame.origin_mm = n.map(|v| v * offset);
        }
        frame
    }

    pub(crate) fn rectangle_plane_z(&self, pointer: Pos2, rect: Rect) -> f64 {
        let Some(selection) = self.exact_pick_at_screen(pointer, rect) else {
            return 0.0;
        };
        if selection.element
            != (ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            })
        {
            return 0.0;
        }
        self.active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)
            .map_or(0.0, |item| item.origin_mm.z + item.size_mm.z)
    }

    pub(crate) fn handle_shortcuts(&mut self, context: &egui::Context) {
        // Escape in a scene name field keeps the old name; the field, drawn
        // later in this frame, would otherwise save what was typed.
        if self.saved_views_ui.renaming.is_some()
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.saved_views_ui.renaming = None;
        }
        let command_modifier = context.input(|input| input.modifiers.command);
        let command_chord = context.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command
                ) || matches!(
                    event,
                    egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)
                )
            })
        });
        let transform_input_enabled = !context.wants_keyboard_input()
            && match self.active_tool {
                ActiveTool::Move => self.move_session().is_some(),
                ActiveTool::Rotate => self.rotate_session().is_some(),
                _ => false,
            };
        self.interpret_transform_input_for(
            self.active_tool,
            transform_input_enabled,
            command_modifier,
            command_chord,
        );
        let alt_pick_through = !context.wants_keyboard_input()
            && context.input(|input| input.modifiers.alt)
            && self
                .hover
                .pick
                .as_ref()
                .is_some_and(|pick| pick.overlapping.len() > 1);
        let cycle_with_alt = self.face_workflow.update_alt_pick_through(alt_pick_through);
        self.face_workflow.set_xray_preview(alt_pick_through);
        let typing = context.wants_keyboard_input();
        let command = context.input_mut(|input| keymap::pressed(input, typing));
        let cycle_overlap = cycle_with_alt
            || (!typing
                && context.input_mut(|input| input.consume_shortcut(&keymap::CYCLE_OVERLAP)));
        let confirm = !typing && context.input(|input| input.key_pressed(keymap::CONFIRM));
        if !context.wants_keyboard_input() {
            let typed = context.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::Text(text)
                            if text
                                .chars()
                                .all(|character| "0123456789.,-;xX*×/ ".contains(character)) =>
                        {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<String>()
            });
            // Space alone is the Select shortcut; it must not open the value
            // field, which would then hold the keyboard against every shortcut.
            if !typed.trim().is_empty() {
                self.value_box.input.clear();
                self.value_box.input.push_str(&typed);
                self.value_box.focus = true;
                if self.active_tool == ActiveTool::PlanarOffset {
                    self.refresh_planar_offset_preview();
                } else if self.active_tool == ActiveTool::Revolve {
                    self.refresh_revolve_preview();
                } else if matches!(
                    self.active_tool,
                    ActiveTool::Shell | ActiveTool::Fillet | ActiveTool::Chamfer
                ) {
                    self.refresh_general_finish_preview();
                }
            }
        }

        // Drawing and transform tools pin their direction to a coloured axis
        // with the arrow keys. Read the lock before the command chain so an
        // arrow never falls through to a tool shortcut.
        if matches!(
            self.active_tool,
            ActiveTool::Rotate
                | ActiveTool::Move
                | ActiveTool::Scale
                | ActiveTool::Line
                | ActiveTool::Rectangle
                | ActiveTool::Circle
                | ActiveTool::Arc
                | ActiveTool::Polygon
                | ActiveTool::Ellipse
        ) && (!context.wants_keyboard_input()
            || context.memory(|memory| memory.has_focus(egui::Id::new("value-box-input"))))
        {
            let requested = keymap::AXIS_LOCKS.into_iter().find(|(key, _)| {
                context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, *key))
            });
            if let Some((_, axis)) = requested {
                let typed_value = context
                    .memory(|memory| memory.has_focus(egui::Id::new("value-box-input")))
                    .then(|| self.value_box.input.clone());
                let held = match self.active_tool {
                    ActiveTool::Move => self.gesture.transform.move_axis_lock,
                    ActiveTool::Scale => self.gesture.transform.scale_axis_lock,
                    ActiveTool::Line => self.gesture.sketch.axis_lock,
                    _ if self.uses_drawing_plane() => Some(match self.face_workflow_datum() {
                        PrincipalPlane::Xy => Axis::Z,
                        PrincipalPlane::Xz => Axis::Y,
                        PrincipalPlane::Yz => Axis::X,
                    }),
                    _ => self.gesture.transform.rotate_axis_lock,
                };
                // Pressing the axis already held releases it, so one key both
                // locks and unlocks.
                let axis = if axis.is_some() && axis == held {
                    None
                } else {
                    axis
                };
                match self.active_tool {
                    ActiveTool::Move => self.set_move_axis_lock(axis),
                    ActiveTool::Scale => self.set_scale_axis_lock(axis),
                    ActiveTool::Line => {
                        self.gesture.sketch.axis_lock = axis;
                        self.digest = self.catalog.format(
                            "digest-line-axis-locked",
                            &BTreeMap::from([(
                                "axis",
                                self.catalog
                                    .text(axis.map_or("axis-name-plane", axis_name_key)),
                            )]),
                        );
                    }
                    _ if self.uses_drawing_plane() => self.set_drawing_plane(match axis {
                        Some(Axis::X) => PrincipalPlane::Yz,
                        Some(Axis::Y) => PrincipalPlane::Xz,
                        _ => PrincipalPlane::Xy,
                    }),
                    _ => self.set_rotate_axis_lock(axis),
                }
                if let Some(value) = typed_value {
                    self.value_box.input = value;
                }
            }
        }

        if let Some(command) = command {
            match command {
                // Esc first cancels whatever is in progress; only with nothing
                // left to cancel does it deselect.
                AppCommand::Deselect => self.cancel_or_deselect(),
                AppCommand::Copy => {
                    if self.command_enabled(AppCommand::Copy) {
                        self.dispatch_command(AppCommand::Copy);
                        context.copy_text("Ketchup object selection".to_owned());
                    }
                }
                command => self.dispatch_command(command),
            }
        } else if cycle_overlap {
            self.cycle_hover_overlap();
            self.hover.alt_choice = cycle_with_alt;
        } else if confirm && self.active_tool == ActiveTool::Spline {
            self.complete_spline();
        } else if confirm && (self.has_preview() || self.has_drawn_shape_preview()) {
            self.confirm_preview();
        } else if confirm && self.has_occurrence_operation_preview() {
            self.confirm_push_pull_preview();
        } else if confirm && self.tool_preview.get::<SweepPreview>().is_some() {
            self.confirm_sweep_preview();
        } else if confirm && self.tool_preview.get::<LoftPreview>().is_some() {
            self.confirm_loft_preview();
        }
    }

    pub(crate) fn show_tool_rail(&mut self, ui: &mut egui::Ui) {
        // Grouped the way the design groups them: pick, draw, modify, measure,
        // navigate. A group boundary draws a hairline.
        const TOOLS: [(AppCommand, u8); 16] = [
            (AppCommand::Select, 0),
            (AppCommand::Line, 1),
            (AppCommand::Rectangle, 1),
            (AppCommand::Circle, 1),
            (AppCommand::Arc, 1),
            (AppCommand::Polygon, 1),
            (AppCommand::Ellipse, 1),
            (AppCommand::Spline, 1),
            (AppCommand::PlanarOffset, 1),
            (AppCommand::PushPull, 2),
            (AppCommand::Move, 2),
            (AppCommand::Rotate, 2),
            (AppCommand::Mirror, 2),
            (AppCommand::Measure, 3),
            (AppCommand::Orbit, 4),
            (AppCommand::Pan, 4),
        ];
        let palette = self.palette();
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut group = TOOLS[0].1;
            for (id, tool_group) in TOOLS {
                if tool_group != group {
                    group = tool_group;
                    ui.add_space(5.0);
                    let (rule, _) = ui.allocate_exact_size(Vec2::new(20.0, 1.0), Sense::hover());
                    ui.painter().rect_filled(rule, 0.0, palette.line);
                    ui.add_space(5.0);
                }
                let spec = CommandRegistry::spec(id);
                let active = spec.tool == Some(self.active_tool);
                let enabled = self.command_enabled(id);
                let label = self.catalog.text(spec.label_key);
                let response = ui.add_enabled(
                    enabled,
                    egui::Button::new("")
                        .frame(false)
                        .min_size(Vec2::new(TOOL_BUTTON_SIZE, TOOL_BUTTON_SIZE)),
                );
                self.paint_rail_button(ui, &response, id, enabled, active);
                name_widget(&response, enabled, &label);
                if response
                    .on_hover_text(self.catalog.format(
                        "tool-tooltip",
                        &BTreeMap::from([
                            ("tool", label.clone()),
                            ("shortcut", keymap::shortcut_text(&self.catalog, spec.id)),
                        ]),
                    ))
                    .clicked()
                {
                    self.dispatch_command(id);
                }
            }
            ui.add_space((ui.available_height() - TOOL_BUTTON_SIZE - 12.0).max(0.0));
            let enabled = self.command_enabled(AppCommand::Delete);
            let response = ui.add_enabled(
                enabled,
                egui::Button::new("")
                    .frame(false)
                    .min_size(Vec2::new(TOOL_BUTTON_SIZE, TOOL_BUTTON_SIZE)),
            );
            self.paint_rail_button(ui, &response, AppCommand::Delete, enabled, false);
            name_widget(&response, enabled, &self.command_label(AppCommand::Delete));
            if response
                .on_hover_text(self.catalog.text("tooltip-delete"))
                .clicked()
            {
                self.dispatch_command(AppCommand::Delete);
            }
        });
    }

    pub(crate) fn show_pocket_properties(&mut self, ui: &mut egui::Ui) {
        let Some((feature_id, depth)) = self.selected_pocket() else {
            self.solid_tools.pocket_editor_feature = None;
            self.solid_tools.pocket_depth_input.clear();
            self.solid_tools.pocket_depth_source.clear();
            return;
        };
        // An untouched input follows the document, e.g. after an AI edit.
        let source = depth.source_token();
        let tools = &mut self.solid_tools;
        let untouched = tools.pocket_depth_input == tools.pocket_depth_source;
        if tools.pocket_editor_feature != Some(feature_id)
            || (untouched && tools.pocket_depth_source != source)
        {
            tools.pocket_editor_feature = Some(feature_id);
            tools.pocket_depth_input = source.to_owned();
            tools.pocket_depth_source = source.to_owned();
        }
        section_header(
            ui,
            self.palette(),
            &self.catalog.text("pocket-properties-title"),
        );
        ui.horizontal(|ui| {
            ui.label(self.catalog.text("pocket-properties-depth"));
            ui.text_edit_singleline(&mut self.solid_tools.pocket_depth_input);
            ui.label(self.catalog.text("unit-mm"));
        });
        let apply = ui
            .button(self.catalog.text("pocket-properties-apply"))
            .clicked();
        if apply {
            if let Some(depth_mm) = parse_distance_mm(&self.solid_tools.pocket_depth_input) {
                if self.set_selected_pocket_depth(depth_mm) {
                    self.solid_tools.pocket_depth_input = format_height(depth_mm);
                    self.solid_tools.pocket_depth_source = format_height(depth_mm);
                }
            } else {
                self.digest = self.catalog.text("digest-pocket-invalid-depth");
            }
        }
        ui.separator();
    }

    pub(crate) fn show_shortcuts_window(&mut self, context: &egui::Context) {
        if !self.panels.shortcuts_open {
            return;
        }
        let mut open = true;
        egui::Window::new(self.catalog.text("shortcuts-title"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                for spec in CommandRegistry::COMMANDS
                    .iter()
                    .filter(|spec| keymap::binding(spec.id).is_some())
                {
                    ui.label(self.catalog.format(
                        "shortcuts-row",
                        &BTreeMap::from([
                            ("command", self.catalog.text(spec.label_key)),
                            ("shortcut", keymap::shortcut_text(&self.catalog, spec.id)),
                        ]),
                    ));
                }
                ui.separator();
                if ui.button(self.catalog.text("shortcuts-close")).clicked() {
                    self.panels.shortcuts_open = false;
                }
            });
        if !open {
            self.panels.shortcuts_open = false;
        }
    }
}

pub(crate) fn create_box_batch(
    definition_id: DefinitionId,
    feature_ids: [FeatureId; 2],
    occurrence_id: OccurrenceId,
    names: [&str; 4],
    origin_mm: Vec3,
    size_mm: Vec3,
) -> CommandBatch {
    let [profile_id, extrusion_id] = feature_ids;
    let [name, profile_name, extrusion_name, occurrence_name] = names;
    CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: name.to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: profile_id,
            definition_id,
            name: profile_name.to_owned(),
            kind: FeatureKind::polygon(&[
                [0.0, 0.0],
                [size_mm.x, 0.0],
                [size_mm.x, size_mm.y],
                [0.0, size_mm.y],
            ]),
        },
        CanonicalCommand::CreateFeature {
            id: extrusion_id,
            definition_id,
            name: extrusion_name.to_owned(),
            kind: FeatureKind::extrusion(
                profile_id,
                Dimension::new(format_height(size_mm.z), size_mm.z)
                    .expect("validated box height is canonical"),
            ),
        },
        CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: occurrence_name.to_owned(),
            transform: Transform::from_translation(origin_mm.x, origin_mm.y, origin_mm.z)
                .expect("validated box origin is canonical"),
            parent: None,
            tags: Default::default(),
            visible: true,
        },
    ])
}

pub(crate) fn parse_rectangle_dimensions(input: &str) -> Option<[f64; 2]> {
    let values = input
        .split([',', ';', 'x', 'X', '*'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    match values.as_slice() {
        [width, depth]
            if width.is_finite() && depth.is_finite() && *width > 0.01 && *depth > 0.01 =>
        {
            Some([*width, *depth])
        }
        _ => None,
    }
}

fn exact_arc_profile_geometry(
    segments: &[ProfileSegment],
    closed: bool,
) -> Option<ExactArcProfileGeometry> {
    if !closed {
        return None;
    }
    match segments {
        [
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
            ProfileSegment::Line {
                start_mm: line_start,
                end_mm: line_end,
            },
        ] if end_mm == line_start && start_mm == line_end => {
            Some((*start_mm, *end_mm, *center_mm, *clockwise))
        }
        [
            ProfileSegment::Line {
                start_mm: line_start,
                end_mm: line_end,
            },
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            },
        ] if line_end == start_mm && line_start == end_mm => {
            Some((*start_mm, *end_mm, *center_mm, *clockwise))
        }
        _ => None,
    }
}

pub(crate) fn exact_circle_geometry(
    segments: &[ProfileSegment],
    closed: bool,
) -> Option<([f64; 2], f64)> {
    let [
        ProfileSegment::CircularArc {
            start_mm: first_start,
            end_mm: first_end,
            center_mm: first_center,
            clockwise: first_clockwise,
        },
        ProfileSegment::CircularArc {
            start_mm: second_start,
            end_mm: second_end,
            center_mm: second_center,
            clockwise: second_clockwise,
        },
    ] = segments
    else {
        return None;
    };
    if !closed
        || first_start != second_end
        || first_end != second_start
        || first_center != second_center
        || first_clockwise != second_clockwise
    {
        return None;
    }
    let first_vector = [
        first_start[0] - first_center[0],
        first_start[1] - first_center[1],
    ];
    let end_vector = [
        first_end[0] - first_center[0],
        first_end[1] - first_center[1],
    ];
    if first_vector[0] != -end_vector[0] || first_vector[1] != -end_vector[1] {
        return None;
    }
    let radius = first_vector[0].hypot(first_vector[1]);
    (radius.is_finite() && radius > 0.0).then_some((*first_center, radius))
}

/// A closed curve through `points` that previews the spline being drawn: a
/// uniform Catmull-Rom loop, close to the periodic spline the exact kernel
/// builds through the same points.
fn closed_spline_preview(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let count = points.len();
    if count < 2 {
        return points.to_vec();
    }
    let steps = (PREVIEW_CURVE_SEGMENTS / count).max(4);
    (0..count)
        .flat_map(|span| {
            let [p0, p1, p2, p3] =
                [count - 1, 0, 1, 2].map(|offset| points[(span + offset) % count]);
            (0..steps).map(move |step| {
                let t = step as f64 / steps as f64;
                let (t2, t3) = (t * t, t * t * t);
                std::array::from_fn(|axis| {
                    0.5 * (2.0 * p1[axis]
                        + (p2[axis] - p0[axis]) * t
                        + (2.0 * p0[axis] - 5.0 * p1[axis] + 4.0 * p2[axis] - p3[axis]) * t2
                        + (3.0 * p1[axis] - p0[axis] - 3.0 * p2[axis] + p3[axis]) * t3)
                })
            })
        })
        .collect()
}
