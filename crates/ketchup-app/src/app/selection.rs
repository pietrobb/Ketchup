//! Selecting, deselecting and inverting, selection windows, topological selection and deleting the selection.

use crate::*;

impl KetchupApp {
    #[doc(hidden)]
    pub fn headless_select_solid_tool_operand(
        &mut self,
        occurrence_id: OccurrenceId,
        keep_tool: bool,
    ) -> bool {
        if self.active_solid_tool_operation().is_none() {
            return false;
        }
        let Some(definition_id) = self
            .document
            .current()
            .occurrence(occurrence_id)
            .map(|occurrence| occurrence.definition_id())
        else {
            return false;
        };
        let selecting_tool = self.solid_tools.target.is_some();
        self.select_solid_tool_occurrence(
            Some(SelectionId {
                definition_id,
                instance_path: InstancePath::root(occurrence_id),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            }),
            keep_tool,
        );
        if selecting_tool {
            self.has_occurrence_operation_preview()
        } else {
            self.solid_tools
                .target
                .as_ref()
                .is_some_and(|selection| selection.instance_path.root_occurrence() == occurrence_id)
        }
    }

    pub(crate) fn selected_instance_paths(&self) -> BTreeSet<InstancePath> {
        let mut paths = self.selection.occurrences.clone();
        if let Some(primary) = &self.selection.primary {
            paths.insert(primary.instance_path.clone());
        }
        paths
    }

    pub(crate) fn select_all_instances_source_plan(&self) -> Option<SelectAllInstancesSourcePlan> {
        if self.selection.selected_group.is_some() {
            return None;
        }
        let source_instance_paths = self.selected_instance_paths();
        let source_count = source_instance_paths.len();
        let source_instance_path = (source_count == 1).then(|| {
            source_instance_paths
                .first()
                .expect("one selected instance exists")
                .clone()
        })?;
        let scene = self.active_scene_query();
        let source = scene
            .iter()
            .find(|item| item.instance_path == source_instance_path)?;
        let definition_id = source.definition_id;
        let definition_name = source.definition_name.clone();
        let target_instance_paths = scene
            .into_iter()
            .filter(|item| item.definition_id == definition_id)
            .map(|item| item.instance_path)
            .collect::<BTreeSet<_>>();
        let target_count = target_instance_paths.len();
        (target_count > 1).then_some(SelectAllInstancesSourcePlan {
            source_revision: self.document_revision(),
            source_instance_paths,
            source_count,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            source_instance_path,
            definition_id,
            definition_name,
            target_instance_paths,
            target_count,
        })
    }

    pub(crate) fn selection_count(&self) -> usize {
        self.selected_instance_paths().len()
    }

    pub(crate) fn correction_selection_is_current(&self, selection: &CorrectionSelection) -> bool {
        match selection {
            CorrectionSelection::Group(group_id) => {
                self.selection.selected_group == Some(*group_id)
            }
            CorrectionSelection::Occurrences {
                occurrence_ids,
                primary_occurrence_id,
            } => {
                self.selection.selected_group.is_none()
                    && self.selected_occurrence_ids().eq(occurrence_ids)
                    && primary_occurrence_id.is_none_or(|expected| {
                        self.selected_move_reference().is_some_and(|selection| {
                            selection.instance_path.is_root()
                                && selection.instance_path.root_occurrence() == expected
                        })
                    })
            }
        }
    }

    pub(crate) fn deselect_source_plan(&self) -> Option<DeselectSourcePlan> {
        (!self.selection.occurrences.is_empty()
            || self.selection.primary.is_some()
            || self.selection.selected_group.is_some())
        .then(|| DeselectSourcePlan {
            source_revision: self.document_revision(),
            occurrence_paths: self.selection.occurrences.clone(),
            occurrence_count: self.selection.occurrences.len(),
            primary: self.selection.primary.clone(),
            selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
        })
    }

    pub(crate) fn apply_deselect_source_plan(&mut self, plan: DeselectSourcePlan) -> bool {
        if plan.occurrence_count != plan.occurrence_paths.len()
            || self.deselect_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        self.end_transform_correction();
        self.selection.clear();
        self.digest = self.catalog.text("digest-selection-cleared");
        true
    }

    pub(crate) fn clear_selection(&mut self) -> bool {
        let Some(plan) = self.deselect_source_plan() else {
            return false;
        };
        self.apply_deselect_source_plan(plan)
    }

    pub(crate) fn bind_topological_selection(
        &self,
        locator: &TopologicalPickLocator,
    ) -> Option<SnapshotBoundTopologicalSelection> {
        let snapshot = self.document.current();
        let topology_results = self.topology_results_for_snapshot(&snapshot)?;
        self.topology_projection(&snapshot)
            .topological_pick_current(&snapshot, topology_results, locator)
            .ok()
    }

    #[doc(hidden)]
    pub fn select_topological_locator(&mut self, locator: TopologicalPickLocator) -> bool {
        self.select_topological_locator_additive(locator, false)
    }

    #[doc(hidden)]
    pub fn select_topological_locator_additive(
        &mut self,
        locator: TopologicalPickLocator,
        additive: bool,
    ) -> bool {
        let Some(topological) = self.bind_topological_selection(&locator) else {
            return false;
        };
        let target = topological.target();
        let element = match locator.kind {
            TopologicalElementKind::Face => self
                .topological_push_pull_face_element(&self.document.current(), &target.reference)
                .unwrap_or(ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                }),
            TopologicalElementKind::Edge => {
                let Ok(ordinal) = u8::try_from(locator.ordinal) else {
                    return false;
                };
                ElementId::Edge(ordinal)
            }
            TopologicalElementKind::Vertex => {
                let Ok(ordinal) = u8::try_from(locator.ordinal) else {
                    return false;
                };
                ElementId::Endpoint(ordinal)
            }
        };
        self.end_transform_correction();
        self.selection.select_topological(
            SelectionId {
                definition_id: target.reference.definition_id,
                instance_path: target.instance_path.clone(),
                element,
            },
            topological,
            additive,
        )
    }

    pub(crate) fn selection_window_projected_mesh(
        &self,
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        transform: Transform,
        local_box: Option<ProjectedBox>,
        viewport: Rect,
    ) -> Option<(Vec<Pos2>, Vec<[usize; 3]>)> {
        let exact = self.interaction_exact_registry(snapshot);
        let mut exact_vertices = Vec::new();
        let mut exact_triangles = Vec::new();
        for package in exact
            .render_values(snapshot)
            .filter(|package| package.definition_id() == definition_id)
        {
            let offset = exact_vertices.len();
            exact_vertices.extend(package.vertices().iter().map(|vertex| {
                transform_model_point(
                    transform,
                    Vec3::new(
                        vertex.position_mm[0],
                        vertex.position_mm[1],
                        vertex.position_mm[2],
                    ),
                )
            }));
            exact_triangles.extend(
                package
                    .triangles()
                    .iter()
                    .map(|triangle| triangle.vertex_indices.map(|index| offset + index as usize)),
            );
        }
        if !exact_vertices.is_empty() && !exact_triangles.is_empty() {
            return Some((
                exact_vertices
                    .into_iter()
                    .map(|point| self.project(point, viewport))
                    .collect(),
                exact_triangles,
            ));
        }

        if let Some(definition) = snapshot.definition(definition_id)
            && let Some(mesh) =
                definition.feature_ids().iter().find_map(|feature_id| {
                    match snapshot.feature(*feature_id)?.kind() {
                        FeatureKind::MeshBody(mesh) => Some(mesh),
                        _ => None,
                    }
                })
        {
            return Some((
                mesh.vertices_mm
                    .iter()
                    .map(|vertex| {
                        self.project(
                            transform_model_point(
                                transform,
                                Vec3::new(vertex[0], vertex[1], vertex[2]),
                            ),
                            viewport,
                        )
                    })
                    .collect(),
                mesh.triangles
                    .iter()
                    .map(|triangle| triangle.map(|index| index as usize))
                    .collect(),
            ));
        }

        if let Some((vertices, triangles)) =
            renderer::canonical_definition_fallback_mesh(snapshot, definition_id)
        {
            return Some((
                vertices
                    .into_iter()
                    .map(|vertex| {
                        self.project(
                            transform_model_point(
                                transform,
                                Vec3::new(vertex[0], vertex[1], vertex[2]),
                            ),
                            viewport,
                        )
                    })
                    .collect(),
                triangles
                    .into_iter()
                    .map(|triangle| triangle.map(|index| index as usize))
                    .collect(),
            ));
        }

        let [minimum, maximum] =
            self.definition_local_bounds(snapshot, definition_id, local_box, true)?;
        let size = maximum - minimum;
        let vertices = box_corners(size.x, size.y, size.z)
            .into_iter()
            .map(|corner| {
                self.project(transform_model_point(transform, corner + minimum), viewport)
            })
            .collect();
        let triangles = box_faces()
            .into_iter()
            .flat_map(|face| {
                [
                    [face.corners[0], face.corners[1], face.corners[2]],
                    [face.corners[0], face.corners[2], face.corners[3]],
                ]
            })
            .collect();
        Some((vertices, triangles))
    }

    pub(crate) fn selection_window_paths(
        &self,
        start: Pos2,
        cursor: Pos2,
        viewport: Rect,
    ) -> BTreeSet<InstancePath> {
        let window = Rect::from_two_pos(start, cursor).intersect(viewport);
        let crossing = cursor.x < start.x;
        let snapshot = self.document.current();
        self.active_scene_query_for_snapshot(&snapshot)
            .into_iter()
            .filter_map(|occurrence| {
                let (projected, triangles) = self.selection_window_projected_mesh(
                    &snapshot,
                    occurrence.definition_id,
                    occurrence.transform,
                    None,
                    viewport,
                )?;
                if projected.is_empty()
                    || !projected
                        .iter()
                        .all(|point| point.x.is_finite() && point.y.is_finite())
                {
                    return None;
                }
                let selected = if crossing {
                    triangles.iter().any(|triangle| {
                        let [first, second, third] = triangle.map(|index| projected[index]);
                        triangle_intersects_rect([first, second, third], window)
                    })
                } else {
                    projected.iter().all(|point| window.contains(*point))
                };
                selected.then_some(occurrence.instance_path)
            })
            .collect()
    }

    pub(crate) fn complete_selection_window(&mut self, drag: SelectionWindowDrag, viewport: Rect) {
        if drag.start.distance(drag.cursor) < 4.0 {
            if !drag.additive {
                self.clear_selection();
            }
            return;
        }
        let paths = self.selection_window_paths(drag.start, drag.cursor, viewport);
        self.end_transform_correction();
        if !drag.additive {
            self.selection.clear();
        }
        self.selection.occurrences.extend(paths);
        self.digest = self.catalog.format(
            "digest-selected-all",
            &BTreeMap::from([("count", self.selection_count().to_string())]),
        );
    }

    pub(crate) fn apply_select_all_instances_source_plan(
        &mut self,
        plan: SelectAllInstancesSourcePlan,
    ) -> bool {
        if plan.source_revision != self.document_revision()
            || plan.source_count != plan.source_instance_paths.len()
            || plan.target_count != plan.target_instance_paths.len()
            || !plan
                .target_instance_paths
                .contains(&plan.source_instance_path)
            || self.select_all_instances_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        self.end_transform_correction();
        self.selection.clear();
        self.selection.occurrences = plan.target_instance_paths;
        self.digest = self.catalog.format(
            "digest-selected-definition",
            &BTreeMap::from([
                ("name", plan.definition_name),
                ("count", plan.target_count.to_string()),
            ]),
        );
        true
    }

    pub(crate) fn select_all_instances(&mut self) -> bool {
        let Some(plan) = self.select_all_instances_source_plan() else {
            return false;
        };
        self.apply_select_all_instances_source_plan(plan)
    }

    pub(crate) fn select_all_source_plan(&self) -> Option<SelectAllSourcePlan> {
        let target_instance_paths = self
            .active_scene_query()
            .into_iter()
            .map(|item| item.instance_path)
            .collect::<BTreeSet<_>>();
        let target_count = target_instance_paths.len();
        (target_count > 0
            && (self.selection.primary.is_some()
                || self.selection.selected_group.is_some()
                || self.selection.occurrences != target_instance_paths))
            .then_some(SelectAllSourcePlan {
                source_revision: self.document_revision(),
                source_occurrence_paths: self.selection.occurrences.clone(),
                source_primary: self.selection.primary.clone(),
                source_selected_group: self.selection.selected_group,
                edit_context: self.selection.edit_context.clone(),
                target_instance_paths,
                target_count,
            })
    }

    pub(crate) fn apply_select_all_source_plan(&mut self, plan: SelectAllSourcePlan) -> bool {
        if plan.target_count != plan.target_instance_paths.len()
            || self.select_all_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        self.end_transform_correction();
        self.selection.clear();
        self.selection.occurrences = plan.target_instance_paths;
        self.digest = self.catalog.format(
            "digest-selected-all",
            &BTreeMap::from([("count", plan.target_count.to_string())]),
        );
        true
    }

    pub(crate) fn select_all(&mut self) -> bool {
        let Some(plan) = self.select_all_source_plan() else {
            return false;
        };
        self.apply_select_all_source_plan(plan)
    }

    pub(crate) fn invert_selection_source_plan(&self) -> Option<InvertSelectionSourcePlan> {
        let active_instance_paths = self
            .active_scene_query()
            .into_iter()
            .filter(|item| item.visible)
            .map(|item| item.instance_path)
            .collect::<BTreeSet<_>>();
        if active_instance_paths.is_empty() {
            return None;
        }
        let selected_instance_paths = self.selected_instance_paths();
        let target_instance_paths = active_instance_paths
            .difference(&selected_instance_paths)
            .cloned()
            .collect::<BTreeSet<_>>();
        let target_count = target_instance_paths.len();
        Some(InvertSelectionSourcePlan {
            source_revision: self.document_revision(),
            source_occurrence_paths: self.selection.occurrences.clone(),
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            active_instance_paths,
            target_instance_paths,
            target_count,
        })
    }

    pub(crate) fn apply_invert_selection_source_plan(
        &mut self,
        plan: InvertSelectionSourcePlan,
    ) -> bool {
        if plan.target_count != plan.target_instance_paths.len()
            || self.invert_selection_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        self.end_transform_correction();
        self.selection.clear();
        self.selection.occurrences = plan.target_instance_paths;
        self.digest = self.catalog.format(
            "digest-inverted-selection",
            &BTreeMap::from([("count", plan.target_count.to_string())]),
        );
        true
    }

    pub(crate) fn invert_selection(&mut self) -> bool {
        let Some(plan) = self.invert_selection_source_plan() else {
            return false;
        };
        self.apply_invert_selection_source_plan(plan)
    }

    pub(crate) fn selected_revolve_profile(&self) -> Option<RevolveToolState> {
        let selection = self.selection.primary.as_ref()?;
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        if item.extrusion_feature_id.is_some() || !item.instance_path.is_root() {
            return None;
        }
        let snapshot = self.document.current();
        let profile = snapshot.feature(item.profile_feature_id)?;
        let is_closed = match profile.kind() {
            FeatureKind::Profile { closed, .. } => *closed,
            _ => false,
        };
        if !is_closed || profile.definition_id() != selection.definition_id {
            return None;
        }
        let resolved = snapshot.resolve_instance_path(&item.instance_path).ok()?;
        let world_transform = resolved.world_transform;
        let matrix = world_transform.matrix();
        if matrix[0] != 1.0
            || matrix[1] != 0.0
            || matrix[2] != 0.0
            || matrix[4] != 0.0
            || matrix[5] != 1.0
            || matrix[6] != 0.0
            || matrix[8] != 0.0
            || matrix[9] != 0.0
            || matrix[10] != 1.0
        {
            return None;
        }
        let translation_mm = Vec3::new(matrix[3], matrix[7], matrix[11]);
        Some(RevolveToolState {
            source: RevolveSourcePlan {
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
                translation_mm,
                plane_z: translation_mm.z,
            },
            axis_start_mm: None,
            axis_end_mm: None,
        })
    }

    pub(crate) fn selected_planar_offset_profile(&self) -> Option<(DefinitionId, FeatureId)> {
        self.planar_offset_source_plan()
            .map(|source| (source.definition_id, source.profile_feature_id))
    }

    pub(crate) fn selected_general_finish_target(
        &self,
        kind: TopologicalElementKind,
    ) -> Option<(
        DefinitionId,
        FeatureId,
        Vec<SnapshotBoundTopologicalSelection>,
        Vec<TopologicalElementRef>,
    )> {
        let selection = self.selection.primary.as_ref()?;
        if !(1..=ketchup_model::tolerance::limits::FEATURE_REFERENCES)
            .contains(&self.selection.topological.len())
        {
            return None;
        }
        let snapshot = self.document.current();
        let mut references = Vec::with_capacity(self.selection.topological.len());
        for (_, topological) in &self.selection.topological {
            let resolved = topological
                .resolve_current(&snapshot, &self.exact.topology_results)
                .ok()?;
            if resolved.instance_path != selection.instance_path
                || resolved.reference.definition_id != selection.definition_id
                || resolved.reference.kind != kind
            {
                return None;
            }
            references.push(resolved.reference);
        }
        let target_feature_id = references.first()?.producer_feature_id;
        let target = snapshot.feature(target_feature_id)?;
        if target.definition_id() != selection.definition_id || !target.kind().produces_body() {
            return None;
        }
        let package = self
            .exact
            .results
            .get_render(&snapshot, selection.definition_id)?;
        if package.producer_feature_id() != target_feature_id {
            return None;
        }
        ExactBRepGraph::from_snapshot(&snapshot, selection.definition_id, target_feature_id)
            .ok()?;
        references = match kind {
            TopologicalElementKind::Face => plan_topology_finish_kind(
                GeneralFinishKind::Shell,
                target_feature_id,
                references,
                Dimension::new("1".to_owned(), 1.0).ok()?,
            ),
            TopologicalElementKind::Edge => plan_topology_finish_kind(
                GeneralFinishKind::Fillet,
                target_feature_id,
                references,
                Dimension::new("1".to_owned(), 1.0).ok()?,
            ),
            TopologicalElementKind::Vertex => None,
        }
        .and_then(|feature| {
            feature
                .topological_picks()
                .map(|(_, picks)| picks.into_iter().cloned().collect())
        })?;
        Some((
            selection.definition_id,
            target_feature_id,
            self.selection
                .topological
                .iter()
                .map(|(_, topological)| topological.clone())
                .collect(),
            references,
        ))
    }

    #[must_use]
    pub fn general_finish_preview_selection_parameters(
        &self,
    ) -> Option<(
        FeatureId,
        Vec<TopologicalElementRef>,
        GeneralFinishKind,
        f64,
    )> {
        let preview = self.tool_preview.get::<GeneralFinishPreview>()?;
        self.general_finish_preview_is_current().then(|| {
            (
                preview.plan.source.target_feature_id,
                preview
                    .plan
                    .source
                    .topological_selections
                    .iter()
                    .map(|selection| selection.target().reference.clone())
                    .collect(),
                preview.plan.source.kind,
                f64::from_bits(preview.plan.amount_mm_bits),
            )
        })
    }

    pub(crate) fn selection_visibility_source_plan(
        &self,
        target_visible: bool,
    ) -> Option<SelectionVisibilitySourcePlan> {
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_count = occurrence_paths.len();
        let occurrence_ids = self.selected_occurrence_ids();
        if occurrence_count == 0 || occurrence_ids.len() != occurrence_count {
            return None;
        }
        let snapshot = self.document.current();
        let source_visibility = occurrence_ids
            .iter()
            .map(|id| Some((*id, snapshot.occurrence(*id)?.visible())))
            .collect::<Option<BTreeMap<_, _>>>()?;
        let changed_occurrence_ids = source_visibility
            .iter()
            .filter_map(|(id, visible)| (*visible != target_visible).then_some(*id))
            .collect::<BTreeSet<_>>();
        let commands = changed_occurrence_ids
            .iter()
            .map(|id| CanonicalCommand::SetOccurrenceVisibility {
                id: *id,
                visible: target_visible,
            })
            .collect::<Vec<_>>();
        (!changed_occurrence_ids.is_empty()).then_some(SelectionVisibilitySourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            occurrence_paths,
            occurrence_count,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            source_visibility,
            changed_occurrence_ids,
            target_visible,
            commands,
        })
    }

    pub(crate) fn apply_selection_visibility_source_plan(
        &mut self,
        plan: SelectionVisibilitySourcePlan,
    ) -> bool {
        if plan.source_revision != self.document.current().revision_id()
            || plan.source_digest != self.document.current().canonical_digest()
            || plan.occurrence_count != plan.occurrence_paths.len()
            || self
                .selection_visibility_source_plan(plan.target_visible)
                .as_ref()
                != Some(&plan)
        {
            return false;
        }
        let count = plan.changed_occurrence_ids.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(plan.commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            if plan.target_visible {
                "digest-unhidden"
            } else {
                "digest-hidden"
            },
            &BTreeMap::from([("count", count.to_string())]),
        );
        true
    }

    /// Commit the visibility of every selected occurrence as one undo step.
    pub fn set_selection_visibility(&mut self, visible: bool) -> bool {
        let Some(plan) = self.selection_visibility_source_plan(visible) else {
            return false;
        };
        self.apply_selection_visibility_source_plan(plan)
    }

    pub(crate) fn selected_active_boxes(&self) -> Vec<RenderBox> {
        let selected = self.selected_instance_paths();
        self.active_boxes()
            .into_iter()
            .filter(|item| selected.contains(&item.instance_path))
            .collect()
    }

    /// Center visible selected occurrences without changing magnification or view direction.
    pub fn center_selection(&mut self) {
        let boxes = self.selected_active_boxes();
        let count = boxes.len();
        let Some(rect) = self.camera.viewport_rect else {
            return;
        };
        let corners = boxes
            .iter()
            .flat_map(|item| {
                box_corners(item.size_mm.x, item.size_mm.y, item.size_mm.z)
                    .map(|point| point + item.origin_mm)
            })
            .collect::<Vec<_>>();
        let Some([minimum, maximum]) = bounds_of(corners.iter().copied()) else {
            return;
        };
        let centre = (minimum + maximum) * 0.5;
        self.camera.target_z = centre.z;
        self.camera.pan = Vec2::ZERO;
        self.camera.pan = rect.center() - self.project(centre, rect);
        self.digest = self.catalog.format(
            "digest-center-selection",
            &BTreeMap::from([("count", count.to_string())]),
        );
    }

    pub(crate) fn selected_box(&self) -> Option<RenderBox> {
        let instance_path = self.selected_instance_paths().into_iter().next()?;
        self.active_boxes()
            .into_iter()
            .find(|item| item.instance_path == instance_path)
    }

    pub(crate) fn select_drawn_profile(
        &mut self,
        definition_id: DefinitionId,
        occurrence_id: OccurrenceId,
    ) {
        let selection = SelectionId {
            definition_id,
            instance_path: InstancePath::root(occurrence_id),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        };
        self.selection.select_exact(selection.clone(), false);
        self.hover.drawn_profile = Some((self.document.current().revision_id(), selection));
    }

    pub(crate) fn cut_selection_to_clipboard(&mut self) -> bool {
        let Some(plan) = self.cut_source_plan() else {
            return false;
        };
        self.apply_cut_source_plan(plan)
    }

    pub(crate) fn delete_selection_source_plan(&self) -> Option<DeleteSelectionSourcePlan> {
        if !self.selection.edit_context.is_empty() {
            return None;
        }
        let snapshot = self.document.current();
        let (occurrence_ids, group_ids) = if let Some(group_id) = self.selection.selected_group {
            if self.selection.primary.is_some() || snapshot.group(group_id).is_none() {
                return None;
            }
            let expected_selection = self
                .active_scene_query()
                .into_iter()
                .filter(|occurrence| {
                    occurrence.instance_path.is_root() && occurrence.parent == Some(group_id)
                })
                .map(|occurrence| occurrence.instance_path)
                .collect::<BTreeSet<_>>();
            if expected_selection.is_empty() || self.selection.occurrences != expected_selection {
                return None;
            }

            let mut group_ids = BTreeSet::from([group_id]);
            loop {
                let children = snapshot
                    .groups()
                    .filter(|group| {
                        group
                            .parent()
                            .is_some_and(|parent| group_ids.contains(&parent))
                            && !group_ids.contains(&group.id())
                    })
                    .map(|group| group.id())
                    .collect::<BTreeSet<_>>();
                if children.is_empty() {
                    break;
                }
                group_ids.extend(children);
            }
            let occurrence_ids = snapshot
                .occurrences()
                .filter(|occurrence| {
                    occurrence
                        .parent()
                        .is_some_and(|id| group_ids.contains(&id))
                })
                .map(|occurrence| occurrence.id())
                .collect::<BTreeSet<_>>();
            (occurrence_ids, group_ids)
        } else {
            let paths = self.selected_instance_paths();
            if paths.is_empty() || paths.iter().any(|path| !path.is_root()) {
                return None;
            }
            let occurrence_ids = paths
                .into_iter()
                .map(|path| path.root_occurrence())
                .collect::<BTreeSet<_>>();
            (occurrence_ids, BTreeSet::new())
        };

        if occurrence_ids.is_empty()
            || occurrence_ids.iter().any(|id| {
                snapshot.occurrence(*id).is_none()
                    || snapshot
                        .collections()
                        .any(|collection| collection.occurrence_ids().any(|member| member == *id))
                    || snapshot.assembly_mates().any(|mate| {
                        mate.endpoint_a().occurrence_id() == *id
                            || mate.endpoint_b().occurrence_id() == *id
                    })
            })
        {
            return None;
        }

        let occurrence_count = occurrence_ids.len();
        let group_count = group_ids.len();
        let mut ordered_group_ids = group_ids.iter().copied().collect::<Vec<_>>();
        ordered_group_ids.sort_by_key(|id| {
            let mut depth = 0_usize;
            let mut parent = snapshot.group(*id).and_then(|group| group.parent());
            while let Some(parent_id) = parent {
                if !group_ids.contains(&parent_id) {
                    break;
                }
                depth += 1;
                parent = snapshot.group(parent_id).and_then(|group| group.parent());
            }
            (std::cmp::Reverse(depth), *id)
        });
        let mut commands = occurrence_ids
            .iter()
            .copied()
            .map(|id| CanonicalCommand::DeleteOccurrence { id })
            .collect::<Vec<_>>();
        commands.extend(
            ordered_group_ids
                .into_iter()
                .map(|id| CanonicalCommand::DeleteGroup { id }),
        );
        Some(DeleteSelectionSourcePlan {
            source_revision: snapshot.revision_id(),
            occurrence_ids,
            occurrence_count,
            group_ids,
            group_count,
            commands,
        })
    }

    pub(crate) fn apply_delete_selection_source_plan(
        &mut self,
        plan: DeleteSelectionSourcePlan,
    ) -> bool {
        if plan.source_revision != self.document.current().revision_id()
            || plan.occurrence_count != plan.occurrence_ids.len()
            || plan.group_count != plan.group_ids.len()
            || plan.commands.len() != plan.occurrence_count + plan.group_count
            || self.delete_selection_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        let DeleteSelectionSourcePlan {
            occurrence_count,
            commands,
            ..
        } = plan;
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.selection.clear();
        self.tool_preview.close::<EphemeralBoxPreview>();
        self.status_key = "status-object-deleted";
        self.digest = self.catalog.format(
            "digest-deleted",
            &BTreeMap::from([("count", occurrence_count.to_string())]),
        );
        true
    }

    pub fn delete_selected(&mut self) -> bool {
        let Some(plan) = self.delete_selection_source_plan() else {
            return false;
        };
        self.apply_delete_selection_source_plan(plan)
    }

    #[must_use]
    pub fn selected_reference(&self) -> Option<SelectionId> {
        self.selection.primary.clone()
    }

    pub(crate) fn reconcile_selection(&mut self) {
        let snapshot = self.document.current();
        self.selection
            .occurrences
            .retain(|path| snapshot.resolve_instance_path(path).is_ok());
        if self.selection.primary.as_ref().is_some_and(|selection| {
            snapshot
                .resolve_instance_path(&selection.instance_path)
                .is_err()
        }) {
            self.selection.primary = None;
        }
        if self
            .selection
            .selected_group
            .is_some_and(|group_id| snapshot.group(group_id).is_none())
        {
            self.selection.selected_group = None;
        }
        while self
            .selection
            .edit_context
            .last()
            .is_some_and(|context| match context {
                EditContext::Group(group_id) => snapshot.group(*group_id).is_none(),
                EditContext::Definition {
                    definition_id,
                    instance_path,
                } => snapshot
                    .bind_scene_query(SceneQueryContext::Definition {
                        definition_id: *definition_id,
                        instance_path: instance_path.clone(),
                    })
                    .is_err(),
            })
        {
            self.selection.edit_context.pop();
        }
        if !self.selection.edit_context.is_empty() {
            let allowed_paths = self
                .active_scene_query()
                .into_iter()
                .map(|occurrence| occurrence.instance_path)
                .collect::<BTreeSet<_>>();
            self.selection
                .occurrences
                .retain(|path| allowed_paths.contains(path));
            if self
                .selection
                .primary
                .as_ref()
                .is_some_and(|selection| !allowed_paths.contains(&selection.instance_path))
            {
                self.selection.primary = None;
            }
        }
        if self.selection.primary.is_none()
            || self
                .selection
                .topological
                .iter()
                .any(|(_, selection)| !selection.is_current(&snapshot))
        {
            self.selection.topological.clear();
        }
        let _ = self.current_transform_correction();
        self.push_pull.distance_input.clear();
    }

    pub(crate) fn selected_face_extent_mm(&self) -> Option<f64> {
        let selection = self.selection.primary.as_ref()?;
        if matches!(selection.element, ElementId::TopologicalFace(_)) {
            return Some(0.0);
        }
        self.selected_box()
            .and_then(|item| face_extent(&item, Some(&selection.element)))
    }

    pub(crate) fn selected_pocket(&self) -> Option<(FeatureId, Dimension)> {
        let definition_id = self.selection.primary.as_ref()?.definition_id;
        let snapshot = self.document.current();
        snapshot
            .definition(definition_id)?
            .feature_ids()
            .iter()
            .rev()
            .find_map(|feature_id| {
                let feature = snapshot.feature(*feature_id)?;
                let FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(_),
                    extent: FeatureExtent::Blind(depth),
                    operation: PadOperation::Cut { .. },
                    ..
                }) = feature.kind()
                else {
                    return None;
                };
                Some((*feature_id, depth.clone()))
            })
    }

    pub(crate) fn set_selected_pocket_depth(&mut self, depth_mm: f64) -> bool {
        let Some((feature_id, _)) = self.selected_pocket() else {
            return false;
        };
        let Ok(dimension) = Dimension::new(format_height(depth_mm), depth_mm) else {
            return false;
        };
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: feature_id,
                    dimension,
                },
            ]))
            .is_err()
        {
            self.digest = self.catalog.text("digest-pocket-invalid-depth");
            return false;
        }
        self.value_box.input = format_height(depth_mm);
        self.status_key = "status-pocket-created";
        self.digest = self.catalog.format(
            "digest-pocket-depth-edited",
            &BTreeMap::from([("depth", format_height(depth_mm))]),
        );
        true
    }

    pub(crate) fn cancel_or_deselect(&mut self) {
        // Escape closes a spline that has enough points instead of dropping them.
        if self.active_tool == ActiveTool::Spline && self.complete_spline() {
            return;
        }
        if matches!(
            self.active_tool,
            ActiveTool::Move | ActiveTool::Rotate | ActiveTool::Scale
        ) {
            self.end_transform_correction();
        }
        self.face_workflow.set_xray_preview(false);
        if self.feature_history_preview_pending() {
            self.cancel_feature_history_preview();
        } else if self.active_tool == ActiveTool::Helix {
            self.clear_ephemeral_edit_state();
            self.active_tool = ActiveTool::Select;
            self.status_key = "status-ready";
            self.digest = self.catalog.text("digest-cancelled");
        } else if self.active_tool == ActiveTool::Mirror {
            self.gesture.mirror = None;
            self.active_tool = ActiveTool::Select;
            self.status_key = "status-ready";
            self.digest = self.catalog.text("digest-cancelled");
        } else if self.active_tool == ActiveTool::ZoomWindow {
            self.gesture.drag.close::<ZoomWindowDrag>();
            self.active_tool = ActiveTool::Select;
            self.status_key = "status-ready";
            self.digest = self.catalog.text("digest-cancelled");
        } else if self.gesture.measure.start.is_some() {
            self.clear_measurement();
            self.digest = self.catalog.text("digest-measure-cleared");
            self.status_key = "status-measure-first-point";
        } else if self.has_preview()
            || self.has_occurrence_operation_preview()
            || self.solid_tools.revolve.is_some()
            || self.tool_preview.get::<RevolvePreview>().is_some()
            || self.tool_preview.get::<PlanarOffsetPreview>().is_some()
            || self.tool_preview.get::<SweepPreview>().is_some()
            || self.tool_preview.get::<LoftPreview>().is_some()
            || self.tool_preview.get::<GeneralFinishPreview>().is_some()
            || self.solid_tools.target.is_some()
            || self.gesture.drag.get::<PushPullAnchor>().is_some()
            || self.transform_tool.session.is_some()
            || self.gesture.sketch.armed
        {
            self.clear_ephemeral_edit_state();
            self.cancel_rectangle_sketch();
            self.digest = self.catalog.text("digest-cancelled");
        } else if self.selection_count() > 0 {
            self.dispatch_command(AppCommand::Deselect);
        } else {
            self.exit_edit_context();
        }
    }

    /// Which validators the panel will run on the next manual request.
    #[must_use]
    pub fn validator_panel_selection(&self) -> Vec<&'static str> {
        ASSISTANT_VALIDATOR_IDS
            .into_iter()
            .filter(|validator| self.validator_panel.selection.contains(validator))
            .collect()
    }
}

fn triangle_intersects_rect(triangle: [Pos2; 3], rect: Rect) -> bool {
    if triangle.iter().any(|point| rect.contains(*point)) {
        return true;
    }
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    if corners
        .iter()
        .any(|point| screen_triangle_contains_point(triangle, *point))
    {
        return true;
    }
    let triangle_edges = [
        [triangle[0], triangle[1]],
        [triangle[1], triangle[2]],
        [triangle[2], triangle[0]],
    ];
    let rectangle_edges = [
        [corners[0], corners[1]],
        [corners[1], corners[2]],
        [corners[2], corners[3]],
        [corners[3], corners[0]],
    ];
    triangle_edges.into_iter().any(|triangle_edge| {
        rectangle_edges
            .into_iter()
            .any(|rectangle_edge| screen_segments_intersect(triangle_edge, rectangle_edge))
    })
}
