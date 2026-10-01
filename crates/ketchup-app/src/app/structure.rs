//! Model structure: occurrences, definitions, groups, components, renaming, replacing and visibility.

use crate::*;

impl KetchupApp {
    #[doc(hidden)]
    pub fn headless_select_occurrence(&mut self, occurrence_id: OccurrenceId) -> bool {
        if self.document.current().occurrence(occurrence_id).is_none() {
            return false;
        }
        self.selection.select_occurrence(occurrence_id, false);
        true
    }

    pub(crate) fn definition_is_imported_exact_body(
        snapshot: &Snapshot,
        definition_id: DefinitionId,
    ) -> bool {
        snapshot
            .definition(definition_id)
            .is_some_and(|definition| {
                matches!(
                    definition.feature_ids(),
                    [feature_id]
                        if snapshot.feature(*feature_id).is_some_and(|feature| {
                            matches!(feature.kind(), FeatureKind::ImportedExactBody(_))
                        })
                )
            })
    }

    /// Canonical definition referenced by an occurrence.
    #[must_use]
    pub fn occurrence_definition_id(&self, occurrence_id: OccurrenceId) -> Option<DefinitionId> {
        self.document
            .current()
            .occurrence(occurrence_id)
            .map(|occurrence| occurrence.definition_id())
    }

    /// Derived box origin and size for an occurrence in model millimetres.
    #[must_use]
    pub fn occurrence_box_geometry(&self, occurrence_id: u64) -> Option<(Vec3, Vec3)> {
        self.active_boxes()
            .into_iter()
            .find(|item| item.instance_path == InstancePath::root(OccurrenceId(occurrence_id)))
            .map(|item| (item.origin_mm, item.size_mm))
    }

    /// How many occurrences are currently selected.
    #[must_use]
    pub fn selected_occurrence_count(&self) -> usize {
        self.selection.occurrences.len()
    }

    #[must_use]
    pub fn occurrence_is_selected(&self, id: OccurrenceId) -> bool {
        self.selected_instance_paths()
            .contains(&InstancePath::root(id))
    }

    /// How many definitions the active document holds.
    #[must_use]
    pub fn definition_count(&self) -> usize {
        self.document.current().definitions().count()
    }

    #[must_use]
    pub fn occurrence_count(&self) -> usize {
        self.document.current().occurrences().count()
    }

    /// How many groups the active document holds.
    #[must_use]
    pub fn group_count(&self) -> usize {
        self.document.current().groups().count()
    }

    pub(crate) fn selected_root_occurrence_ids(
        &self,
    ) -> Result<BTreeSet<OccurrenceId>, RootOccurrenceSelectionError> {
        let paths = self.selected_instance_paths();
        let root_count = paths.iter().filter(|path| path.is_root()).count();
        if root_count == paths.len() {
            return Ok(paths
                .into_iter()
                .map(|path| path.root_occurrence())
                .collect());
        }
        Err(if root_count == 0 {
            RootOccurrenceSelectionError::Nested { paths }
        } else {
            RootOccurrenceSelectionError::Mixed { paths }
        })
    }

    pub(crate) fn root_occurrence_selection_error(
        &self,
        error: &RootOccurrenceSelectionError,
    ) -> String {
        let (key, paths) = match error {
            RootOccurrenceSelectionError::Nested { paths } => {
                ("selection-error-nested-instance-paths", paths)
            }
            RootOccurrenceSelectionError::Mixed { paths } => {
                ("selection-error-mixed-instance-paths", paths)
            }
        };
        self.catalog.format(
            key,
            &BTreeMap::from([(
                "paths",
                paths
                    .iter()
                    .map(Self::assistant_instance_path_label)
                    .collect::<Vec<_>>()
                    .join(", "),
            )]),
        )
    }

    pub(crate) fn selected_occurrence_ids(&self) -> BTreeSet<OccurrenceId> {
        if matches!(
            self.selection.edit_context.last(),
            Some(EditContext::Definition { .. })
        ) {
            return BTreeSet::new();
        }
        self.selected_root_occurrence_ids().unwrap_or_default()
    }

    pub(crate) fn occurrence_alignment_source_plan(&self) -> Option<OccurrenceAlignmentSourcePlan> {
        let (moving_id, reference_id) = self.selected_alignment_pair()?;
        self.occurrence_alignment_source_plan_for_pair(moving_id, reference_id)
    }

    pub(crate) fn occurrence_alignment_source_plan_for_pair(
        &self,
        moving_id: OccurrenceId,
        reference_id: OccurrenceId,
    ) -> Option<OccurrenceAlignmentSourcePlan> {
        if self.selection.selected_group.is_some()
            || moving_id == reference_id
            || matches!(
                self.selection.edit_context.last(),
                Some(EditContext::Definition { .. })
            )
        {
            return None;
        }
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_count = occurrence_paths.len();
        let snapshot = self.document.current();
        let moving = snapshot.occurrence(moving_id)?;
        let reference = snapshot.occurrence(reference_id)?;
        let boxes = self.active_boxes();
        let moving_box = boxes
            .iter()
            .find(|item| item.instance_path == InstancePath::root(moving_id))?
            .clone();
        let reference_box = boxes
            .iter()
            .find(|item| item.instance_path == InstancePath::root(reference_id))?
            .clone();
        (self.occurrence_in_active_context(&InstancePath::root(moving_id))
            && self.occurrence_in_active_context(&InstancePath::root(reference_id)))
        .then_some(OccurrenceAlignmentSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            occurrence_paths,
            occurrence_count,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            moving_id,
            moving_definition_id: moving.definition_id(),
            moving_name: moving.name().to_owned(),
            moving_transform: moving.transform(),
            moving_parent: moving.parent(),
            moving_tag: moving.tag(),
            moving_visible: moving.visible(),
            moving_box,
            reference_id,
            reference_definition_id: reference.definition_id(),
            reference_name: reference.name().to_owned(),
            reference_transform: reference.transform(),
            reference_parent: reference.parent(),
            reference_tag: reference.tag(),
            reference_visible: reference.visible(),
            reference_box,
        })
    }

    pub(crate) fn occurrence_distribution_source_plan(
        &self,
    ) -> Option<OccurrenceDistributionSourcePlan> {
        let occurrence_ids = self.selected_occurrence_ids();
        self.occurrence_distribution_source_plan_for_ids(&occurrence_ids)
    }

    pub(crate) fn occurrence_distribution_source_plan_for_ids(
        &self,
        occurrence_ids: &BTreeSet<OccurrenceId>,
    ) -> Option<OccurrenceDistributionSourcePlan> {
        if occurrence_ids.len() < 3
            || self.selection.selected_group.is_some()
            || matches!(
                self.selection.edit_context.last(),
                Some(EditContext::Definition { .. })
            )
        {
            return None;
        }
        let occurrence_paths = self.selected_instance_paths();
        if occurrence_paths
            != occurrence_ids
                .iter()
                .copied()
                .map(InstancePath::root)
                .collect()
        {
            return None;
        }
        let snapshot = self.document.current();
        let active_boxes = self.active_boxes();
        let mut occurrences = BTreeMap::new();
        for id in occurrence_ids {
            let occurrence = snapshot.occurrence(*id)?;
            let render_box = active_boxes
                .iter()
                .find(|item| item.instance_path == InstancePath::root(*id))?
                .clone();
            if !self.occurrence_in_active_context(&InstancePath::root(*id)) {
                return None;
            }
            occurrences.insert(
                *id,
                OccurrenceDistributionSourceItem {
                    definition_id: occurrence.definition_id(),
                    name: occurrence.name().to_owned(),
                    transform: occurrence.transform(),
                    parent: occurrence.parent(),
                    tag: occurrence.tag(),
                    visible: occurrence.visible(),
                    render_box,
                },
            );
        }
        Some(OccurrenceDistributionSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            occurrence_count: occurrence_paths.len(),
            occurrence_paths,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            occurrences,
        })
    }

    pub(crate) fn linear_pattern_source_plan_for_occurrence(
        &self,
        occurrence_id: OccurrenceId,
    ) -> Option<LinearPatternSourcePlan> {
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_count = occurrence_paths.len();
        let snapshot = self.document.current();
        let source = snapshot.occurrence(occurrence_id)?;
        let definition_id = source.definition_id();
        let definition = snapshot.definition(definition_id)?;
        let next_occurrence_id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let existing_definition_occurrence_count = snapshot
            .scene_query()
            .into_iter()
            .filter(|item| item.definition_id == definition_id)
            .count();
        self.occurrence_in_active_context(&InstancePath::root(occurrence_id))
            .then_some(LinearPatternSourcePlan {
                source_revision: snapshot.revision_id(),
                source_digest: snapshot.canonical_digest(),
                occurrence_paths,
                occurrence_count,
                source_primary: self.selection.primary.clone(),
                source_selected_group: self.selection.selected_group,
                edit_context: self.selection.edit_context.clone(),
                occurrence_id,
                definition_id,
                definition_name: definition.name().to_owned(),
                source_transform: source.transform(),
                source_parent: source.parent(),
                source_tag: source.tag(),
                source_visible: source.visible(),
                source_color: source.color(),
                next_occurrence_id,
                existing_definition_occurrence_count,
            })
    }

    pub(crate) fn rectangular_pattern_source_plan_for_occurrence(
        &self,
        occurrence_id: OccurrenceId,
    ) -> Option<RectangularPatternSourcePlan> {
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_count = occurrence_paths.len();
        let snapshot = self.document.current();
        let source = snapshot.occurrence(occurrence_id)?;
        let definition_id = source.definition_id();
        let definition = snapshot.definition(definition_id)?;
        let next_occurrence_id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let existing_definition_occurrence_count = snapshot
            .scene_query()
            .into_iter()
            .filter(|item| item.definition_id == definition_id)
            .count();
        self.occurrence_in_active_context(&InstancePath::root(occurrence_id))
            .then_some(RectangularPatternSourcePlan {
                source_revision: snapshot.revision_id(),
                source_digest: snapshot.canonical_digest(),
                occurrence_paths,
                occurrence_count,
                source_primary: self.selection.primary.clone(),
                source_selected_group: self.selection.selected_group,
                edit_context: self.selection.edit_context.clone(),
                occurrence_id,
                definition_id,
                definition_name: definition.name().to_owned(),
                source_transform: source.transform(),
                source_parent: source.parent(),
                source_tag: source.tag(),
                source_visible: source.visible(),
                source_color: source.color(),
                next_occurrence_id,
                existing_definition_occurrence_count,
            })
    }

    pub(crate) fn circular_pattern_source_plan_for_occurrence(
        &self,
        occurrence_id: OccurrenceId,
    ) -> Option<CircularPatternSourcePlan> {
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_count = occurrence_paths.len();
        let snapshot = self.document.current();
        let source = snapshot.occurrence(occurrence_id)?;
        let definition_id = source.definition_id();
        let definition = snapshot.definition(definition_id)?;
        let next_occurrence_id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let existing_definition_occurrence_count = snapshot
            .scene_query()
            .into_iter()
            .filter(|item| item.definition_id == definition_id)
            .count();
        self.occurrence_in_active_context(&InstancePath::root(occurrence_id))
            .then_some(CircularPatternSourcePlan {
                source_revision: snapshot.revision_id(),
                source_digest: snapshot.canonical_digest(),
                occurrence_paths,
                occurrence_count,
                source_primary: self.selection.primary.clone(),
                source_selected_group: self.selection.selected_group,
                edit_context: self.selection.edit_context.clone(),
                occurrence_id,
                definition_id,
                definition_name: definition.name().to_owned(),
                source_transform: source.transform(),
                source_parent: source.parent(),
                source_tag: source.tag(),
                source_visible: source.visible(),
                source_color: source.color(),
                next_occurrence_id,
                existing_definition_occurrence_count,
            })
    }

    pub(crate) fn selected_definition_id(&self) -> Option<DefinitionId> {
        self.selected_definition_id_for_snapshot(&self.document.current())
    }

    pub(crate) fn selected_definition_id_for_snapshot(
        &self,
        snapshot: &Snapshot,
    ) -> Option<DefinitionId> {
        let selected = self.selected_instance_paths();
        let definitions = self
            .active_scene_query_for_snapshot(snapshot)
            .into_iter()
            .filter(|item| selected.contains(&item.instance_path))
            .map(|item| item.definition_id)
            .collect::<BTreeSet<_>>();
        (definitions.len() == 1).then(|| *definitions.first().expect("one definition exists"))
    }

    pub(crate) fn occurrence_in_active_context(&self, instance_path: &InstancePath) -> bool {
        self.active_scene_query()
            .into_iter()
            .any(|occurrence| occurrence.instance_path == *instance_path)
    }

    pub(crate) fn select_group(&mut self, group_id: GroupId) -> bool {
        let snapshot = self.document.current();
        let Some(group) = snapshot.group(group_id) else {
            return false;
        };
        let ids = self
            .active_scene_query()
            .into_iter()
            .filter(|occurrence| {
                occurrence.instance_path.is_root() && occurrence.parent == Some(group_id)
            })
            .map(|occurrence| occurrence.instance_path)
            .collect::<BTreeSet<_>>();
        if ids.is_empty() {
            return false;
        }
        let name = group.name().to_owned();
        self.end_transform_correction();
        self.selection.clear();
        self.selection.occurrences = ids;
        self.selection.selected_group = Some(group_id);
        self.digest = self.catalog.format(
            "digest-selected-group",
            &BTreeMap::from([
                ("name", name),
                ("count", self.selection_count().to_string()),
            ]),
        );
        true
    }

    pub(crate) fn enter_group_context(&mut self, group_id: GroupId) -> bool {
        if self.document.current().group(group_id).is_none() {
            return false;
        }
        self.cancel_transform_session();
        self.end_transform_correction();
        self.selection
            .edit_context
            .push(EditContext::Group(group_id));
        self.invalidate_pending_import_reviews();
        self.selection.clear();
        self.digest = self.catalog.text("digest-entered-group-context");
        true
    }

    pub(crate) fn enter_occurrence_context(&mut self, instance_path: InstancePath) -> bool {
        let snapshot = self.document.current();
        let Ok(resolved) = snapshot.resolve_instance_path(&instance_path) else {
            return false;
        };
        if !self.occurrence_in_active_context(&instance_path) {
            return false;
        }
        let root = snapshot.occurrence(instance_path.root_occurrence());
        let context = match (self.selection.edit_context.last(), root) {
            (None, Some(occurrence))
                if instance_path.is_root() && occurrence.parent().is_some() =>
            {
                EditContext::Group(occurrence.parent().unwrap())
            }
            _ => EditContext::Definition {
                definition_id: resolved.definition_id,
                instance_path,
            },
        };
        self.cancel_transform_session();
        self.end_transform_correction();
        self.selection.edit_context.push(context.clone());
        self.invalidate_pending_import_reviews();
        self.selection.clear();
        self.digest = self.catalog.text(match context {
            EditContext::Group(_) => "digest-entered-group-context",
            EditContext::Definition { .. } => "digest-entered-component-context",
        });
        true
    }

    pub(crate) fn select_from_outliner(&mut self, instance_path: InstancePath, additive: bool) {
        if !self.occurrence_in_active_context(&instance_path) {
            return;
        }
        self.end_transform_correction();
        let snapshot = self.document.current();
        let root_id = instance_path.root_occurrence();
        if self.selection.edit_context.is_empty()
            && instance_path.is_root()
            && let Some(group_id) = snapshot
                .occurrence(root_id)
                .and_then(|occurrence| occurrence.parent())
        {
            self.select_group(group_id);
            return;
        }
        self.selection.select_path(instance_path.clone(), additive);
        if let Some(item) = snapshot
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == instance_path)
        {
            self.digest = self.catalog.format(
                "digest-selected-outliner",
                &BTreeMap::from([("name", item.occurrence_name)]),
            );
        }
    }

    pub(crate) fn select_definition(&mut self, definition_id: DefinitionId, additive: bool) {
        let snapshot = self.document.current();
        let Some(definition) = snapshot.definition(definition_id) else {
            return;
        };
        self.end_transform_correction();
        let ids = self
            .active_scene_query()
            .into_iter()
            .filter(|item| item.definition_id == definition_id)
            .map(|item| item.instance_path)
            .collect::<Vec<_>>();
        let name = definition.name().to_owned();
        if !additive {
            self.selection.clear();
        }
        self.selection.occurrences.extend(ids.iter().cloned());
        self.digest = self.catalog.format(
            "digest-selected-definition",
            &BTreeMap::from([("name", name), ("count", ids.len().to_string())]),
        );
    }

    pub(crate) fn outliner_query(&self) -> Vec<OutlinerDefinition> {
        let snapshot = self.document.current();
        let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
        let scoped = !self.selection.edit_context.is_empty();
        let scene = if scoped {
            self.active_scene_query()
        } else {
            snapshot.scene_query()
        };
        snapshot
            .definitions()
            .filter(|definition| {
                !scoped
                    || scene
                        .iter()
                        .any(|occurrence| occurrence.definition_id == definition.id())
            })
            .map(|definition| {
                let size = projection
                    .occurrences()
                    .iter()
                    .find(|occurrence| occurrence.body.definition_id == definition.id())
                    .and_then(|occurrence| occurrence.local_box.map(|local_box| local_box.size_mm))
                    .unwrap_or(Vec3::ZERO);
                let occurrences = scene
                    .iter()
                    .filter(|item| item.definition_id == definition.id())
                    .map(|item| {
                        #[cfg(test)]
                        let matrix = item.transform.matrix();
                        OutlinerOccurrence {
                            instance_path: item.instance_path.clone(),
                            name: item.occurrence_name.clone(),
                            #[cfg(test)]
                            position: format!(
                                "{},{}",
                                format_height(matrix[3]),
                                format_height(matrix[7])
                            ),
                            visible: item.visible,
                            parent: item.parent,
                        }
                    })
                    .collect();
                OutlinerDefinition {
                    id: definition.id(),
                    name: definition.name().to_owned(),
                    specification: format!(
                        "{} × {} × {}",
                        format_height(size.x),
                        format_height(size.y),
                        format_height(size.z)
                    ),
                    occurrences,
                }
            })
            .collect()
    }

    pub(crate) fn outliner_groups(&self) -> Vec<OutlinerGroup> {
        let snapshot = self.document.current();
        snapshot
            .groups()
            .map(|group| OutlinerGroup {
                id: group.id(),
                name: group.name().to_owned(),
                member_count: snapshot
                    .occurrences()
                    .filter(|occurrence| occurrence.parent() == Some(group.id()))
                    .count(),
            })
            .collect()
    }

    pub(crate) fn selected_group_id(&self) -> Option<GroupId> {
        if let Some(group_id) = self.selection.selected_group {
            return Some(group_id);
        }
        let snapshot = self.document.current();
        let mut ids = self.selected_occurrence_ids().into_iter();
        let first = snapshot.occurrence(ids.next()?)?.parent()?;
        ids.all(|id| {
            snapshot
                .occurrence(id)
                .is_some_and(|occurrence| occurrence.parent() == Some(first))
        })
        .then_some(first)
    }

    pub(crate) fn make_unique_source_plan(&self) -> Option<MakeUniqueSourcePlan> {
        let occurrence_paths = self.selected_instance_paths();
        let source_path = (occurrence_paths.len() == 1)
            .then(|| occurrence_paths.first().expect("one occurrence exists"))?;
        source_path.is_root().then_some(())?;
        let occurrence_id = source_path.root_occurrence();
        let snapshot = self.document.current();
        let source_definition_id = snapshot.occurrence(occurrence_id)?.definition_id();
        let source = snapshot.definition(source_definition_id)?;
        let source_definition_name = source.name().to_owned();
        let visible_peer_ids = self
            .active_scene_query()
            .into_iter()
            .filter(|item| item.definition_id == source_definition_id)
            .filter_map(|item| {
                item.instance_path
                    .is_root()
                    .then(|| item.instance_path.root_occurrence())
            })
            .collect::<BTreeSet<_>>();
        let visible_peer_count = visible_peer_ids.len();
        (visible_peer_count > 1 && visible_peer_ids.contains(&occurrence_id)).then_some(())?;
        let new_definition_id = DefinitionId(
            snapshot
                .definitions()
                .map(|definition| definition.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let first_feature_id = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)?;
        let feature_id_map = source
            .feature_ids()
            .iter()
            .enumerate()
            .map(|(offset, source_id)| {
                first_feature_id
                    .checked_add(offset as u64)
                    .map(|new_id| (*source_id, FeatureId(new_id)))
            })
            .collect::<Option<Vec<_>>>()?;
        let new_definition_name = self.catalog.format(
            "model-unique-name",
            &BTreeMap::from([("name", source_definition_name.clone())]),
        );
        Some(MakeUniqueSourcePlan {
            source_revision: snapshot.revision_id(),
            occurrence_paths,
            occurrence_id,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            source_definition_id,
            source_definition_name,
            visible_peer_ids,
            visible_peer_count,
            command: CloneDefinitionPlan::new(
                occurrence_id,
                source_definition_id,
                new_definition_id,
                new_definition_name,
                feature_id_map,
            ),
        })
    }

    pub(crate) fn occurrence_rename_source_plan(&self) -> Option<OccurrenceRenameSourcePlan> {
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_count = occurrence_paths.len();
        let occurrence_path = (occurrence_count == 1)
            .then(|| occurrence_paths.first().expect("one occurrence exists"))?;
        occurrence_path.is_root().then_some(())?;
        let occurrence_id = occurrence_path.root_occurrence();
        let snapshot = self.document.current();
        let original_name = snapshot.occurrence(occurrence_id)?.name().to_owned();
        self.occurrence_in_active_context(occurrence_path)
            .then_some(OccurrenceRenameSourcePlan {
                source_revision: snapshot.revision_id(),
                source_digest: snapshot.canonical_digest(),
                occurrence_paths,
                occurrence_count,
                source_primary: self.selection.primary.clone(),
                source_selected_group: self.selection.selected_group,
                edit_context: self.selection.edit_context.clone(),
                occurrence_id,
                original_name,
            })
    }

    pub(crate) fn occurrence_rename_plan(
        &self,
        pending: &PendingOccurrenceRename,
    ) -> Option<OccurrenceRenamePlan> {
        let source = self.occurrence_rename_source_plan()?;
        (source == pending.source && source.occurrence_count == source.occurrence_paths.len())
            .then_some(())?;
        let target_name = pending.name.trim();
        (!target_name.is_empty() && target_name != source.original_name).then_some(())?;
        Some(OccurrenceRenamePlan {
            command: CanonicalCommand::RenameEntity {
                id: source.occurrence_id,
                name: target_name.to_owned(),
            },
            source,
            target_name: target_name.to_owned(),
        })
    }

    pub(crate) fn begin_occurrence_rename(&mut self) {
        let Some(source) = self.occurrence_rename_source_plan() else {
            return;
        };
        self.modal.open(PendingOccurrenceRename {
            name: source.original_name.clone(),
            source,
        });
    }

    pub(crate) fn apply_occurrence_rename_plan(&mut self, plan: OccurrenceRenamePlan) -> bool {
        let pending = PendingOccurrenceRename {
            source: plan.source.clone(),
            name: plan.target_name.clone(),
        };
        if self.occurrence_rename_plan(&pending).as_ref() != Some(&plan)
            || self
                .apply_batch_with_work_recovery(&CommandBatch::new(vec![plan.command]))
                .is_err()
        {
            return false;
        }
        self.modal.close::<PendingOccurrenceRename>();
        self.digest = self.catalog.format(
            "digest-renamed-occurrence",
            &BTreeMap::from([("name", plan.target_name)]),
        );
        true
    }

    pub fn confirm_occurrence_rename(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingOccurrenceRename>().cloned() else {
            return false;
        };
        let Some(plan) = self.occurrence_rename_plan(&pending) else {
            return false;
        };
        self.apply_occurrence_rename_plan(plan)
    }

    #[must_use]
    pub fn rename_occurrence_visible(&self) -> bool {
        self.modal.get::<PendingOccurrenceRename>().is_some()
    }

    #[must_use]
    pub fn rename_occurrence_input(&self) -> Option<&str> {
        self.modal
            .get::<PendingOccurrenceRename>()
            .map(|pending| pending.name.as_str())
    }

    #[must_use]
    pub fn occurrence_name(&self, occurrence_id: OccurrenceId) -> Option<String> {
        self.document
            .current()
            .occurrence(occurrence_id)
            .map(|occurrence| occurrence.name().to_owned())
    }

    pub(crate) fn replacement_definition_options(
        &self,
        occurrence_id: OccurrenceId,
    ) -> Vec<(DefinitionId, String)> {
        let snapshot = self.document.current();
        let Some(occurrence) = snapshot.occurrence(occurrence_id) else {
            return Vec::new();
        };
        snapshot
            .definitions()
            .filter(|definition| definition.id() != occurrence.definition_id())
            .map(|definition| (definition.id(), definition.name().to_owned()))
            .collect()
    }

    pub(crate) fn component_replacement_source_plan(
        &self,
    ) -> Option<ComponentReplacementSourcePlan> {
        let occurrence_paths = self.selected_instance_paths();
        let source_path = (occurrence_paths.len() == 1)
            .then(|| occurrence_paths.first().expect("one occurrence exists"))?;
        source_path.is_root().then_some(())?;
        let occurrence_id = source_path.root_occurrence();
        let snapshot = self.document.current();
        let occurrence = snapshot.occurrence(occurrence_id)?;
        let source_definition_id = occurrence.definition_id();
        let source_definition_name = snapshot.definition(source_definition_id)?.name().to_owned();
        let candidate_definitions = self
            .replacement_definition_options(occurrence_id)
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let candidate_count = candidate_definitions.len();
        let initial_target_definition_id = *candidate_definitions.first_key_value()?.0;
        Some(ComponentReplacementSourcePlan {
            source_revision: snapshot.revision_id(),
            occurrence_paths,
            occurrence_count: 1,
            occurrence_id,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            source_definition_id,
            source_definition_name,
            candidate_definitions,
            candidate_count,
            initial_target_definition_id,
        })
    }

    pub(crate) fn component_replacement_plan(
        &self,
        pending: &PendingComponentReplacement,
    ) -> Option<ComponentReplacementPlan> {
        let source = self.component_replacement_source_plan()?;
        (source == pending.source
            && source.occurrence_count == source.occurrence_paths.len()
            && source.candidate_count == source.candidate_definitions.len())
        .then_some(())?;
        let target_definition_name = source
            .candidate_definitions
            .get(&pending.target_definition_id)?
            .clone();
        Some(ComponentReplacementPlan {
            command: CanonicalCommand::RepointOccurrence {
                id: source.occurrence_id,
                definition_id: pending.target_definition_id,
            },
            source,
            target_definition_id: pending.target_definition_id,
            target_definition_name,
        })
    }

    pub(crate) fn begin_component_replacement(&mut self) {
        let Some(source) = self.component_replacement_source_plan() else {
            return;
        };
        let target_definition_id = source.initial_target_definition_id;
        self.modal.open(PendingComponentReplacement {
            source,
            target_definition_id,
        });
    }

    #[must_use]
    pub fn component_replacement_visible(&self) -> bool {
        self.modal.get::<PendingComponentReplacement>().is_some()
    }

    #[must_use]
    pub fn component_replacement_input(&self) -> Option<(OccurrenceId, DefinitionId)> {
        self.modal
            .get::<PendingComponentReplacement>()
            .map(|pending| (pending.source.occurrence_id, pending.target_definition_id))
    }

    pub(crate) fn apply_component_replacement_plan(
        &mut self,
        plan: ComponentReplacementPlan,
    ) -> bool {
        let pending = PendingComponentReplacement {
            source: plan.source.clone(),
            target_definition_id: plan.target_definition_id,
        };
        if self.component_replacement_plan(&pending).as_ref() != Some(&plan) {
            return false;
        }
        let ComponentReplacementPlan {
            source,
            target_definition_id,
            target_definition_name,
            command,
        } = plan;
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![command]))
            .is_err()
        {
            return false;
        }
        if let Some(primary) = self.selection.primary.as_mut()
            && primary.instance_path == InstancePath::root(source.occurrence_id)
        {
            primary.definition_id = target_definition_id;
        }
        self.modal.close::<PendingComponentReplacement>();
        self.digest = self.catalog.format(
            "digest-replaced-component",
            &BTreeMap::from([("name", target_definition_name)]),
        );
        true
    }

    pub fn confirm_component_replacement(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingComponentReplacement>().cloned() else {
            return false;
        };
        let Some(plan) = self.component_replacement_plan(&pending) else {
            return false;
        };
        self.apply_component_replacement_plan(plan)
    }

    pub(crate) fn definition_rename_source_plan(&self) -> Option<DefinitionRenameSourcePlan> {
        let instance_paths = self.selected_instance_paths();
        let instance_count = instance_paths.len();
        (instance_count > 0).then_some(())?;
        let definition_id = self.selected_definition_id()?;
        let snapshot = self.document.current();
        let original_name = snapshot.definition(definition_id)?.name().to_owned();
        Some(DefinitionRenameSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            instance_paths,
            instance_count,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            definition_id,
            original_name,
        })
    }

    pub(crate) fn definition_rename_plan(
        &self,
        pending: &PendingDefinitionRename,
    ) -> Option<DefinitionRenamePlan> {
        let source = self.definition_rename_source_plan()?;
        (source == pending.source && source.instance_count == source.instance_paths.len())
            .then_some(())?;
        let target_name = pending.name.trim();
        (!target_name.is_empty() && target_name != source.original_name).then_some(())?;
        Some(DefinitionRenamePlan {
            command: CanonicalCommand::RenameDefinition {
                id: source.definition_id,
                name: target_name.to_owned(),
            },
            source,
            target_name: target_name.to_owned(),
        })
    }

    pub(crate) fn begin_definition_rename(&mut self) {
        let Some(source) = self.definition_rename_source_plan() else {
            return;
        };
        self.modal.open(PendingDefinitionRename {
            name: source.original_name.clone(),
            source,
        });
    }

    pub(crate) fn apply_definition_rename_plan(&mut self, plan: DefinitionRenamePlan) -> bool {
        let pending = PendingDefinitionRename {
            source: plan.source.clone(),
            name: plan.target_name.clone(),
        };
        if self.definition_rename_plan(&pending).as_ref() != Some(&plan)
            || self
                .apply_batch_with_work_recovery(&CommandBatch::new(vec![plan.command]))
                .is_err()
        {
            return false;
        }
        self.modal.close::<PendingDefinitionRename>();
        self.digest = self.catalog.format(
            "digest-renamed-definition",
            &BTreeMap::from([("name", plan.target_name)]),
        );
        true
    }

    pub fn confirm_definition_rename(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingDefinitionRename>().cloned() else {
            return false;
        };
        let Some(plan) = self.definition_rename_plan(&pending) else {
            return false;
        };
        self.apply_definition_rename_plan(plan)
    }

    pub(crate) fn purgeable_definition_ids(&self) -> BTreeSet<DefinitionId> {
        let snapshot = self.document.current();
        let referenced = snapshot
            .occurrences()
            .map(|occurrence| occurrence.definition_id())
            .chain(
                snapshot
                    .local_occurrences()
                    .map(|occurrence| occurrence.definition_id()),
            )
            .collect::<BTreeSet<_>>();
        snapshot
            .definitions()
            .filter(|definition| {
                !referenced.contains(&definition.id())
                    && definition.local_occurrence_ids().is_empty()
                    && definition.local_group_ids().is_empty()
            })
            .map(|definition| definition.id())
            .collect()
    }

    pub(crate) fn purge_unused_source_plan(&self) -> Option<PurgeUnusedSourcePlan> {
        let definition_ids = self.purgeable_definition_ids();
        let definition_count = definition_ids.len();
        (definition_count > 0).then_some(PurgeUnusedSourcePlan {
            definition_ids,
            definition_count,
        })
    }

    #[must_use]
    pub fn rename_definition_visible(&self) -> bool {
        self.modal.get::<PendingDefinitionRename>().is_some()
    }

    #[must_use]
    pub fn rename_definition_input(&self) -> Option<&str> {
        self.modal
            .get::<PendingDefinitionRename>()
            .map(|pending| pending.name.as_str())
    }

    #[must_use]
    pub fn definition_name(&self, definition_id: DefinitionId) -> Option<String> {
        self.document
            .current()
            .definition(definition_id)
            .map(|definition| definition.name().to_owned())
    }

    /// Number of definitions the Model menu can safely purge right now.
    #[must_use]
    pub fn purgeable_definition_count(&self) -> usize {
        self.purge_unused_source_plan()
            .map_or(0, |plan| plan.definition_count)
    }

    pub(crate) fn apply_purge_unused_source_plan(&mut self, plan: PurgeUnusedSourcePlan) -> bool {
        if plan.definition_count != plan.definition_ids.len()
            || self.purge_unused_source_plan().as_ref() != Some(&plan)
            || self
                .apply_batch_with_work_recovery(&CommandBatch::new(
                    plan.definition_ids
                        .into_iter()
                        .map(|id| CanonicalCommand::DeleteDefinition { id })
                        .collect(),
                ))
                .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-purged-unused",
            &BTreeMap::from([("count", plan.definition_count.to_string())]),
        );
        true
    }

    /// Remove every unreferenced leaf definition as one canonical undo step.
    pub fn purge_unused_definitions(&mut self) -> bool {
        let Some(plan) = self.purge_unused_source_plan() else {
            return false;
        };
        self.apply_purge_unused_source_plan(plan)
    }

    pub(crate) fn grounded_occurrence_source_plan(
        &self,
        target_grounded: bool,
    ) -> Option<GroundedOccurrenceSourcePlan> {
        if self.selection.selected_group.is_some() || !self.selection.edit_context.is_empty() {
            return None;
        }
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_count = occurrence_paths.len();
        let occurrence_path = (occurrence_count == 1)
            .then(|| occurrence_paths.first().expect("one occurrence exists"))?;
        if !occurrence_path.is_root() {
            return None;
        }
        let occurrence_id = occurrence_path.root_occurrence();
        let snapshot = self.document.current();
        snapshot.occurrence(occurrence_id)?;
        let source_grounded = snapshot.occurrence_is_grounded(occurrence_id);
        (source_grounded != target_grounded).then_some(GroundedOccurrenceSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            occurrence_paths,
            occurrence_count,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            occurrence_id,
            source_grounded,
            target_grounded,
            command: CanonicalCommand::SetOccurrenceGrounded {
                id: occurrence_id,
                grounded: target_grounded,
            },
        })
    }

    pub(crate) fn apply_grounded_occurrence_source_plan(
        &mut self,
        plan: GroundedOccurrenceSourcePlan,
    ) -> bool {
        if plan.source_revision != self.document.current().revision_id()
            || plan.source_digest != self.document.current().canonical_digest()
            || plan.occurrence_count != plan.occurrence_paths.len()
            || self
                .grounded_occurrence_source_plan(plan.target_grounded)
                .as_ref()
                != Some(&plan)
            || self
                .apply_batch_with_work_recovery(&CommandBatch::new(vec![plan.command]))
                .is_err()
        {
            return false;
        }
        self.digest = self.catalog.text(if plan.target_grounded {
            "digest-grounded-occurrence"
        } else {
            "digest-ungrounded-occurrence"
        });
        true
    }

    /// Ground or unground the one selected root occurrence as one undo step.
    pub fn set_selected_occurrence_grounded(&mut self, grounded: bool) -> bool {
        let Some(plan) = self.grounded_occurrence_source_plan(grounded) else {
            return false;
        };
        self.apply_grounded_occurrence_source_plan(plan)
    }

    pub(crate) fn hide_others_source_plan(&self) -> Option<HideOthersSourcePlan> {
        self.selection.edit_context.is_empty().then_some(())?;
        let selected_occurrence_ids = self.selected_occurrence_ids();
        let selected_paths = self.selected_instance_paths();
        if selected_occurrence_ids.is_empty()
            || selected_occurrence_ids.len() != selected_paths.len()
            || selected_paths.iter().any(|path| !path.is_root())
        {
            return None;
        }
        let snapshot = self.document.current();
        let hidden_occurrence_ids = snapshot
            .occurrences()
            .filter(|occurrence| {
                occurrence.visible() && !selected_occurrence_ids.contains(&occurrence.id())
            })
            .map(|occurrence| occurrence.id())
            .collect::<BTreeSet<_>>();
        let occurrence_count = hidden_occurrence_ids.len();
        let commands = hidden_occurrence_ids
            .iter()
            .copied()
            .map(|id| CanonicalCommand::SetOccurrenceVisibility { id, visible: false })
            .collect::<Vec<_>>();
        (occurrence_count > 0).then_some(HideOthersSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            selected_occurrence_ids,
            edit_context: self.selection.edit_context.clone(),
            hidden_occurrence_ids,
            occurrence_count,
            commands,
        })
    }

    pub(crate) fn apply_hide_others_source_plan(&mut self, plan: HideOthersSourcePlan) -> bool {
        if plan.occurrence_count != plan.hidden_occurrence_ids.len()
            || plan.occurrence_count != plan.commands.len()
            || self.hide_others_source_plan().as_ref() != Some(&plan)
            || self
                .apply_batch_with_work_recovery(&CommandBatch::new(plan.commands))
                .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-hidden-others",
            &BTreeMap::from([("count", plan.occurrence_count.to_string())]),
        );
        true
    }

    /// Hide every visible unselected root occurrence as one canonical undo step.
    pub fn hide_others(&mut self) -> bool {
        let Some(plan) = self.hide_others_source_plan() else {
            return false;
        };
        self.apply_hide_others_source_plan(plan)
    }

    pub(crate) fn unhide_all_source_plan(&self) -> Option<UnhideAllSourcePlan> {
        self.selection.edit_context.is_empty().then_some(())?;
        let snapshot = self.document.current();
        let hidden_occurrence_ids = snapshot
            .occurrences()
            .filter(|occurrence| !occurrence.visible())
            .map(|occurrence| occurrence.id())
            .collect::<BTreeSet<_>>();
        let occurrence_count = hidden_occurrence_ids.len();
        let commands = hidden_occurrence_ids
            .iter()
            .copied()
            .map(|id| CanonicalCommand::SetOccurrenceVisibility { id, visible: true })
            .collect::<Vec<_>>();
        (occurrence_count > 0).then_some(UnhideAllSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            edit_context: self.selection.edit_context.clone(),
            hidden_occurrence_ids,
            occurrence_count,
            commands,
        })
    }

    pub(crate) fn apply_unhide_all_source_plan(&mut self, plan: UnhideAllSourcePlan) -> bool {
        if plan.occurrence_count != plan.hidden_occurrence_ids.len()
            || plan.occurrence_count != plan.commands.len()
            || self.unhide_all_source_plan().as_ref() != Some(&plan)
            || self
                .apply_batch_with_work_recovery(&CommandBatch::new(plan.commands))
                .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-unhidden-all",
            &BTreeMap::from([("count", plan.occurrence_count.to_string())]),
        );
        true
    }

    /// Restore every hidden root occurrence as one canonical undo step.
    pub fn unhide_all(&mut self) -> bool {
        let Some(plan) = self.unhide_all_source_plan() else {
            return false;
        };
        self.apply_unhide_all_source_plan(plan)
    }

    #[must_use]
    pub const fn outliner_visible(&self) -> bool {
        self.panels.outliner_visible
    }

    /// How many occurrences of the active document are hidden.
    #[must_use]
    pub fn hidden_occurrence_count(&self) -> usize {
        self.document
            .current()
            .scene_query()
            .iter()
            .filter(|item| !item.visible)
            .count()
    }

    pub(crate) fn group_selection_source_plan(&self) -> Option<GroupSelectionSourcePlan> {
        if self.selection.selected_group.is_some() {
            return None;
        }
        let paths = self.selected_instance_paths();
        if paths.len() < 2 || paths.iter().any(|path| !path.is_root()) {
            return None;
        }
        let occurrence_ids = paths
            .iter()
            .map(InstancePath::root_occurrence)
            .collect::<BTreeSet<_>>();
        let snapshot = self.document.current();
        let parents = occurrence_ids
            .iter()
            .map(|id| {
                let occurrence = snapshot.occurrence(*id)?;
                self.occurrence_in_active_context(&InstancePath::root(*id))
                    .then_some(occurrence.parent())
            })
            .collect::<Option<BTreeSet<_>>>()?;
        if parents.len() != 1 {
            return None;
        }
        let parent = *parents.first()?;
        let group_id = GroupId(
            snapshot
                .groups()
                .map(|group| group.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let mut commands = vec![CanonicalCommand::CreateGroup {
            id: group_id,
            name: self.catalog.format(
                "model-default-group",
                &BTreeMap::from([("number", group_id.0.to_string())]),
            ),
            transform: Transform::identity(),
            parent,
        }];
        commands.extend(occurrence_ids.iter().copied().map(|id| {
            CanonicalCommand::SetOccurrenceParent {
                id,
                parent: Some(group_id),
            }
        }));
        Some(GroupSelectionSourcePlan {
            source_revision: snapshot.revision_id(),
            group_id,
            occurrence_count: occurrence_ids.len(),
            occurrence_ids,
            edit_context: self.selection.edit_context.clone(),
            commands,
        })
    }

    pub(crate) fn ungroup_selection_source_plan(&self) -> Option<UngroupSelectionSourcePlan> {
        if !self.selection.edit_context.is_empty() || self.selection.primary.is_some() {
            return None;
        }
        let group_id = self.selection.selected_group?;
        let snapshot = self.document.current();
        let group = snapshot.group(group_id)?;
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
        let occurrence_ids = expected_selection
            .iter()
            .map(InstancePath::root_occurrence)
            .collect::<BTreeSet<_>>();
        let occurrences = occurrence_ids
            .iter()
            .map(|id| Some((*id, snapshot.occurrence(*id)?.transform())))
            .collect::<Option<Vec<_>>>()?;
        let child_groups = snapshot
            .groups()
            .filter(|child| child.parent() == Some(group_id))
            .map(|child| (child.id(), child.transform()))
            .collect::<Vec<_>>();
        let group_transform = group.transform();
        let parent = group.parent();
        let mut commands = Vec::new();
        for (id, transform) in &occurrences {
            commands.push(CanonicalCommand::SetOccurrenceTransform {
                id: *id,
                transform: group_transform.compose(*transform),
            });
            commands.push(CanonicalCommand::SetOccurrenceParent { id: *id, parent });
        }
        for (id, transform) in &child_groups {
            commands.push(CanonicalCommand::SetGroupTransform {
                id: *id,
                transform: group_transform.compose(*transform),
            });
            commands.push(CanonicalCommand::SetGroupParent { id: *id, parent });
        }
        commands.push(CanonicalCommand::DeleteGroup { id: group_id });
        Some(UngroupSelectionSourcePlan {
            source_revision: snapshot.revision_id(),
            group_id,
            occurrence_count: occurrence_ids.len(),
            occurrence_ids,
            item_count: occurrences.len() + child_groups.len(),
            edit_context: self.selection.edit_context.clone(),
            commands,
        })
    }

    pub(crate) fn apply_group_selection_source_plan(
        &mut self,
        plan: GroupSelectionSourcePlan,
    ) -> bool {
        if plan.source_revision != self.document_revision()
            || plan.occurrence_count != plan.occurrence_ids.len()
            || self.group_selection_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        let GroupSelectionSourcePlan {
            group_id,
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
        self.select_group(group_id);
        self.digest = self.catalog.format(
            "digest-grouped",
            &BTreeMap::from([("count", occurrence_count.to_string())]),
        );
        true
    }

    pub fn group_selected(&mut self) -> bool {
        let Some(plan) = self.group_selection_source_plan() else {
            return false;
        };
        self.apply_group_selection_source_plan(plan)
    }

    pub(crate) fn apply_ungroup_selection_source_plan(
        &mut self,
        plan: UngroupSelectionSourcePlan,
    ) -> bool {
        if plan.source_revision != self.document_revision()
            || plan.occurrence_count != plan.occurrence_ids.len()
            || self.ungroup_selection_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        let UngroupSelectionSourcePlan {
            occurrence_ids,
            item_count,
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
        self.selection
            .occurrences
            .extend(occurrence_ids.into_iter().map(InstancePath::root));
        self.digest = self.catalog.format(
            "digest-ungrouped",
            &BTreeMap::from([("count", item_count.to_string())]),
        );
        true
    }

    pub fn ungroup_selected(&mut self) -> bool {
        let Some(plan) = self.ungroup_selection_source_plan() else {
            return false;
        };
        self.apply_ungroup_selection_source_plan(plan)
    }

    pub(crate) fn make_component_source_plan(&self) -> Option<MakeComponentSourcePlan> {
        if !self.selection.edit_context.is_empty()
            || (self.selection.selected_group.is_some() && self.selection.primary.is_some())
        {
            return None;
        }
        let snapshot = self.document.current();
        let (
            group_id,
            group_name,
            occurrence_paths,
            occurrence_count,
            subtree_occurrence_count,
            mut commands,
        ) = if let Some(group_id) = self.selection.selected_group {
            let group = snapshot.group(group_id)?;
            if group.parent().is_some() {
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
            let mut local_ids = snapshot
                .groups()
                .filter(|candidate| candidate.id() != group_id)
                .filter_map(|candidate| {
                    let mut parent = candidate.parent();
                    while let Some(id) = parent {
                        if id == group_id {
                            return Some(candidate.id().0);
                        }
                        parent = snapshot.group(id).and_then(|group| group.parent());
                    }
                    None
                })
                .collect::<BTreeSet<_>>();
            let mut subtree_occurrence_count = 0;
            for occurrence in snapshot.occurrences().filter(|occurrence| {
                Self::group_contains_occurrence(&snapshot, group_id, occurrence.id())
            }) {
                if !local_ids.insert(occurrence.id().0) {
                    return None;
                }
                subtree_occurrence_count += 1;
            }
            (
                group_id,
                group.name().to_owned(),
                expected_selection.clone(),
                expected_selection.len(),
                subtree_occurrence_count,
                Vec::new(),
            )
        } else {
            let group_plan = self.group_selection_source_plan()?;
            if group_plan.occurrence_ids.iter().any(|id| {
                snapshot
                    .occurrence(*id)
                    .is_none_or(|occurrence| occurrence.parent().is_some())
            }) {
                return None;
            }
            let occurrence_paths = group_plan
                .occurrence_ids
                .iter()
                .copied()
                .map(InstancePath::root)
                .collect::<BTreeSet<_>>();
            let occurrence_count = occurrence_paths.len();
            (
                group_plan.group_id,
                self.catalog.format(
                    "model-default-group",
                    &BTreeMap::from([("number", group_plan.group_id.0.to_string())]),
                ),
                occurrence_paths,
                occurrence_count,
                occurrence_count,
                group_plan.commands,
            )
        };
        if subtree_occurrence_count == 0 {
            return None;
        }
        let new_definition_id = DefinitionId(
            snapshot
                .definitions()
                .map(|definition| definition.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let new_occurrence_id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let component_name = self.catalog.format(
            "model-component-name",
            &BTreeMap::from([("number", group_id.0.to_string())]),
        );
        commands.push(CanonicalCommand::ConvertGroupToComponent(
            ConvertGroupPlan::new(
                group_id,
                new_definition_id,
                new_occurrence_id,
                component_name.clone(),
            ),
        ));
        Some(MakeComponentSourcePlan {
            source_revision: snapshot.revision_id(),
            group_id,
            group_name,
            occurrence_count,
            occurrence_paths,
            primary: self.selection.primary.clone(),
            selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            component_name,
            subtree_occurrence_count,
            new_definition_id,
            new_occurrence_id,
            commands,
        })
    }

    pub(crate) fn apply_make_component_source_plan(
        &mut self,
        plan: MakeComponentSourcePlan,
    ) -> bool {
        if plan.source_revision != self.document_revision()
            || plan.occurrence_count != plan.occurrence_paths.len()
            || self.make_component_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        let MakeComponentSourcePlan {
            group_id,
            component_name,
            subtree_occurrence_count,
            new_definition_id,
            new_occurrence_id,
            commands,
            ..
        } = plan;
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        debug_assert!(
            self.document
                .current()
                .definition(new_definition_id)
                .is_some()
        );
        debug_assert!(
            self.document
                .current()
                .occurrence(new_occurrence_id)
                .is_some()
        );
        debug_assert!(self.document.current().group(group_id).is_none());
        self.selection.clear();
        self.selection.select_occurrence(new_occurrence_id, false);
        self.digest = self.catalog.format(
            "digest-made-component",
            &BTreeMap::from([
                ("name", component_name),
                ("count", subtree_occurrence_count.to_string()),
            ]),
        );
        true
    }

    pub fn make_component(&mut self) -> bool {
        let Some(plan) = self.make_component_source_plan() else {
            return false;
        };
        self.apply_make_component_source_plan(plan)
    }

    pub(crate) fn apply_make_unique_source_plan(&mut self, plan: MakeUniqueSourcePlan) -> bool {
        if plan.source_revision != self.document_revision()
            || plan.visible_peer_count != plan.visible_peer_ids.len()
            || self.make_unique_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        let MakeUniqueSourcePlan {
            occurrence_id,
            command,
            ..
        } = plan;
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![
                CanonicalCommand::CloneDefinitionAndRepoint(command),
            ]))
            .is_err()
        {
            return false;
        }
        self.selection.select_occurrence(occurrence_id, false);
        self.digest = self.catalog.text("digest-made-unique");
        true
    }

    pub fn make_unique(&mut self) -> bool {
        let Some(plan) = self.make_unique_source_plan() else {
            return false;
        };
        self.apply_make_unique_source_plan(plan)
    }

    pub fn exact_reference_for_occurrence(
        &self,
        instance_path: &InstancePath,
        role: ExactFaceRole,
    ) -> Option<AssemblySelectionTarget> {
        let snapshot = self.document.current();
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| &occurrence.instance_path == instance_path)?;
        let package = self
            .exact
            .results
            .get_render(&snapshot, occurrence.definition_id)?;
        Some(AssemblySelectionTarget {
            instance_path: instance_path.clone(),
            body: package.reference(role)?.clone(),
        })
    }

    pub(crate) fn rotate_copy_occurrences(
        &mut self,
        selection: &SelectionId,
        occurrence_paths: &BTreeSet<InstancePath>,
        centre_mm: Vec3,
        axis: Axis,
        angle_degrees: f64,
    ) -> bool {
        if !rotation_is_meaningful(angle_degrees)
            || occurrence_paths.is_empty()
            || !occurrence_paths.contains(&selection.instance_path)
            || occurrence_paths
                .iter()
                .any(|path| !path.is_root() || !self.occurrence_in_active_context(path))
            || matches!(
                self.selection.edit_context.last(),
                Some(EditContext::Definition { .. })
            )
        {
            return false;
        }
        let snapshot = self.document.current();
        let source_ids = occurrence_paths
            .iter()
            .map(InstancePath::root_occurrence)
            .collect::<Vec<_>>();
        let primary_source_id = selection.instance_path.root_occurrence();
        let Some(primary_source_index) = source_ids
            .iter()
            .position(|source_id| *source_id == primary_source_id)
        else {
            return false;
        };
        if snapshot
            .occurrence(primary_source_id)
            .is_none_or(|source| source.definition_id() != selection.definition_id)
        {
            return false;
        }
        let Ok(rotation) = world_rotation_transform(centre_mm, axis, angle_degrees) else {
            return false;
        };
        let Some(first_new_id) = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return false;
        };
        let mut commands = Vec::new();
        let mut targets = Vec::with_capacity(source_ids.len());
        let mut created_per_definition = BTreeMap::<DefinitionId, usize>::new();
        for (index, source_id) in source_ids.iter().copied().enumerate() {
            let Some(source) = snapshot.occurrence(source_id) else {
                return false;
            };
            let definition_id = source.definition_id();
            let Some(transform) = world_edit_in_parent_space(
                &snapshot,
                source.parent(),
                source.transform(),
                rotation,
            ) else {
                return false;
            };
            let Some(target_id) = first_new_id.checked_add(index as u64).map(OccurrenceId) else {
                return false;
            };
            let Some(definition) = snapshot.definition(definition_id) else {
                return false;
            };
            let existing = snapshot
                .scene_query()
                .into_iter()
                .filter(|item| item.definition_id == definition_id)
                .count();
            let created = created_per_definition.entry(definition_id).or_default();
            *created += 1;
            commands.push(CanonicalCommand::CreateOccurrence {
                id: target_id,
                definition_id,
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", definition.name().to_owned()),
                        ("number", (existing + *created).to_string()),
                    ]),
                ),
                transform,
                parent: source.parent(),
                tag: source.tag(),
                visible: source.visible(),
            });
            if let Some(color) = source.color() {
                commands.push(CanonicalCommand::SetOccurrenceColor {
                    id: target_id,
                    color: Some(color),
                });
            }
            targets.push((target_id, definition_id));
        }
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        let target_paths = targets
            .iter()
            .map(|(target_id, _)| InstancePath::root(*target_id))
            .collect::<BTreeSet<_>>();
        let (primary_target_id, primary_definition_id) = targets[primary_source_index];
        self.selection.select_exact(
            SelectionId {
                definition_id: primary_definition_id,
                instance_path: InstancePath::root(primary_target_id),
                element: selection.element.clone(),
            },
            false,
        );
        self.selection.occurrences.extend(target_paths);
        let target_ids = targets
            .iter()
            .map(|(target_id, _)| *target_id)
            .collect::<BTreeSet<_>>();
        self.record_transform_correction(
            CorrectionSelection::Occurrences {
                occurrence_ids: target_ids,
                primary_occurrence_id: None,
            },
            CorrectionOperation::Rotate(RotateCorrection {
                copy_source_occurrence_ids: Some(source_ids),
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

    pub(crate) fn copy_occurrences(
        &mut self,
        selection: &SelectionId,
        occurrence_paths: &BTreeSet<InstancePath>,
        delta_mm: Vec3,
    ) -> bool {
        let distance_mm = vector_length(delta_mm);
        if !delta_mm.x.is_finite()
            || !delta_mm.y.is_finite()
            || !delta_mm.z.is_finite()
            || distance_mm <= 0.0
            || occurrence_paths.is_empty()
            || !occurrence_paths.contains(&selection.instance_path)
            || occurrence_paths
                .iter()
                .any(|path| !path.is_root() || !self.occurrence_in_active_context(path))
            || matches!(
                self.selection.edit_context.last(),
                Some(EditContext::Definition { .. })
            )
        {
            return false;
        }
        let snapshot = self.document.current();
        let source_ids = occurrence_paths
            .iter()
            .map(InstancePath::root_occurrence)
            .collect::<Vec<_>>();
        let primary_source_id = selection.instance_path.root_occurrence();
        let Some(primary_source_index) = source_ids
            .iter()
            .position(|source_id| *source_id == primary_source_id)
        else {
            return false;
        };
        if snapshot
            .occurrence(primary_source_id)
            .is_none_or(|source| source.definition_id() != selection.definition_id)
        {
            return false;
        }
        let Some(first_new_id) = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            return false;
        };
        let mut commands = Vec::new();
        let mut targets = Vec::with_capacity(source_ids.len());
        let mut created_per_definition = BTreeMap::<DefinitionId, usize>::new();
        for (index, source_id) in source_ids.iter().copied().enumerate() {
            let Some(source) = snapshot.occurrence(source_id) else {
                return false;
            };
            let definition_id = source.definition_id();
            let Some(transform) = translated_in_parent_space(
                &snapshot,
                source.parent(),
                source.transform(),
                delta_mm,
            ) else {
                return false;
            };
            let Some(target_id) = first_new_id.checked_add(index as u64).map(OccurrenceId) else {
                return false;
            };
            let Some(definition) = snapshot.definition(definition_id) else {
                return false;
            };
            let existing = snapshot
                .scene_query()
                .into_iter()
                .filter(|item| item.definition_id == definition_id)
                .count();
            let created = created_per_definition.entry(definition_id).or_default();
            *created += 1;
            commands.push(CanonicalCommand::CreateOccurrence {
                id: target_id,
                definition_id,
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", definition.name().to_owned()),
                        ("number", (existing + *created).to_string()),
                    ]),
                ),
                transform,
                parent: source.parent(),
                tag: source.tag(),
                visible: source.visible(),
            });
            if let Some(color) = source.color() {
                commands.push(CanonicalCommand::SetOccurrenceColor {
                    id: target_id,
                    color: Some(color),
                });
            }
            targets.push((target_id, definition_id));
        }
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        let target_paths = targets
            .iter()
            .map(|(target_id, _)| InstancePath::root(*target_id))
            .collect::<BTreeSet<_>>();
        let (primary_target_id, primary_definition_id) = targets[primary_source_index];
        self.selection.select_exact(
            SelectionId {
                definition_id: primary_definition_id,
                instance_path: InstancePath::root(primary_target_id),
                element: selection.element.clone(),
            },
            false,
        );
        self.selection.occurrences.extend(target_paths);
        let target_ids = targets
            .iter()
            .map(|(target_id, _)| *target_id)
            .collect::<BTreeSet<_>>();
        let correction_selection = CorrectionSelection::Occurrences {
            occurrence_ids: target_ids,
            primary_occurrence_id: Some(primary_target_id),
        };
        let correction = CorrectionOperation::MoveCopy(MoveCopyCorrection {
            source_occurrence_ids: source_ids,
            first_copy_occurrence_id: targets[0].0,
            primary_source_index,
            element: selection.element.clone(),
            delta_mm,
            array_mode: MoveCopyArrayMode::Multiply,
            array_count: 1,
        });
        self.record_transform_correction(correction_selection, correction);
        self.status_key = "status-object-copied";
        self.digest = self.catalog.format(
            "digest-copy-committed",
            &BTreeMap::from([("distance", format_height(distance_mm))]),
        );
        true
    }

    pub(crate) fn occurrence_alignment_plan(
        &self,
        source: &OccurrenceAlignmentSourcePlan,
        axis: Axis,
        mode: AlignMode,
    ) -> Option<OccurrenceAlignmentPlan> {
        if self
            .occurrence_alignment_source_plan_for_pair(source.moving_id, source.reference_id)
            .as_ref()
            != Some(source)
        {
            return None;
        }
        let moving_coordinate_mm = alignment_coordinate(&source.moving_box, axis, mode);
        let reference_coordinate_mm = alignment_coordinate(&source.reference_box, axis, mode);
        let offset_mm = reference_coordinate_mm - moving_coordinate_mm;
        if !offset_mm.is_finite() || offset_mm.abs() <= f64::EPSILON {
            return None;
        }
        let delta_mm = axis_vector(axis, offset_mm);
        let transform = translated_in_parent_space(
            &self.document.current(),
            source.moving_parent,
            source.moving_transform,
            delta_mm,
        )?;
        let command = CanonicalCommand::SetOccurrenceTransform {
            id: source.moving_id,
            transform,
        };
        let mut preview_box = source.moving_box.clone();
        preview_box.origin_mm = preview_box.origin_mm + delta_mm;
        Some(OccurrenceAlignmentPlan {
            source: source.clone(),
            axis,
            mode,
            moving_coordinate_mm,
            reference_coordinate_mm,
            offset_mm,
            command,
            preview_box,
        })
    }

    pub(crate) fn begin_occurrence_align(&mut self) {
        if let Err(error) = self.selected_root_occurrence_ids() {
            self.digest = self.root_occurrence_selection_error(&error);
            return;
        }
        let Some(source) = self.occurrence_alignment_source_plan() else {
            return;
        };
        self.tool_preview.close::<OccurrenceOperationPreview>();
        self.modal.open(PendingOccurrenceAlign {
            source,
            axis: Axis::X,
            mode: AlignMode::Center,
            preview_plan: None,
        });
    }

    #[must_use]
    pub fn occurrence_align_visible(&self) -> bool {
        self.modal.get::<PendingOccurrenceAlign>().is_some()
    }

    #[must_use]
    pub fn occurrence_align_inputs(&self) -> Option<(OccurrenceId, OccurrenceId, Axis, AlignMode)> {
        self.modal.get::<PendingOccurrenceAlign>().map(|pending| {
            (
                pending.source.moving_id,
                pending.source.reference_id,
                pending.axis,
                pending.mode,
            )
        })
    }

    pub(crate) fn occurrence_alignment_binding_is_current(
        &self,
        pending: &PendingOccurrenceAlign,
    ) -> bool {
        self.occurrence_alignment_source_plan().as_ref() == Some(&pending.source)
    }

    #[must_use]
    pub fn occurrence_align_preview_is_current(&self) -> bool {
        self.modal
            .get::<PendingOccurrenceAlign>()
            .is_some_and(|pending| {
                let Some(plan) =
                    self.occurrence_alignment_plan(&pending.source, pending.axis, pending.mode)
                else {
                    return false;
                };
                self.occurrence_alignment_binding_is_current(pending)
                    && pending.preview_plan.as_ref() == Some(&plan)
                    && self
                        .tool_preview
                        .get::<OccurrenceOperationPreview>()
                        .is_some_and(|preview| {
                            self.has_occurrence_operation_preview()
                                && preview.source_revision == plan.source.source_revision
                                && preview.batch.commands() == std::slice::from_ref(&plan.command)
                        })
            })
    }

    pub fn preview_pending_occurrence_align(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingOccurrenceAlign>().cloned() else {
            return false;
        };
        let Some(plan) =
            self.occurrence_alignment_plan(&pending.source, pending.axis, pending.mode)
        else {
            return false;
        };
        if let Some(current) = self.modal.get_mut::<PendingOccurrenceAlign>() {
            current.preview_plan = None;
        }
        if !self.preview_occurrence_alignment_plan(plan.clone()) {
            return false;
        }
        if let Some(current) = self.modal.get_mut::<PendingOccurrenceAlign>() {
            current.preview_plan = Some(plan);
        }
        true
    }

    pub fn confirm_occurrence_align(&mut self) -> bool {
        let Some(plan) = self
            .modal
            .get::<PendingOccurrenceAlign>()
            .and_then(|pending| pending.preview_plan.clone())
        else {
            return false;
        };
        if !self.occurrence_align_preview_is_current()
            || self
                .occurrence_alignment_plan(&plan.source, plan.axis, plan.mode)
                .as_ref()
                != Some(&plan)
            || !self.confirm_occurrence_operation_preview()
        {
            return false;
        }
        self.modal.close::<PendingOccurrenceAlign>();
        true
    }

    pub fn preview_align_occurrences(
        &mut self,
        moving_id: OccurrenceId,
        reference_id: OccurrenceId,
        axis: Axis,
        mode: AlignMode,
    ) -> bool {
        self.tool_preview.close::<OccurrenceOperationPreview>();
        let Some(source) = self.occurrence_alignment_source_plan_for_pair(moving_id, reference_id)
        else {
            return false;
        };
        let Some(plan) = self.occurrence_alignment_plan(&source, axis, mode) else {
            return false;
        };
        self.preview_occurrence_alignment_plan(plan)
    }

    pub(crate) fn preview_occurrence_alignment_plan(
        &mut self,
        plan: OccurrenceAlignmentPlan,
    ) -> bool {
        self.tool_preview.close::<OccurrenceOperationPreview>();
        if self
            .occurrence_alignment_plan(&plan.source, plan.axis, plan.mode)
            .as_ref()
            != Some(&plan)
        {
            return false;
        }
        let batch = CommandBatch::new(vec![plan.command.clone()]);
        self.tool_preview.open(OccurrenceOperationPreview {
            source_revision: plan.source.source_revision,
            command_digest: batch.digest(),
            batch,
            boxes: BTreeMap::from([(plan.source.moving_id, plan.preview_box.clone())]),
            hidden_occurrences: BTreeSet::new(),
            selection_after: None,
            committed_digest_key: "digest-align-committed",
            canonical_plan: Some(OccurrenceCanonicalPreviewPlan::Alignment(Box::new(
                plan.clone(),
            ))),
            solid_tool_plan: None,
        });
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            "digest-align-live",
            &BTreeMap::from([
                ("axis", alignment_axis_label(plan.axis).to_owned()),
                ("mode", alignment_mode_label(plan.mode).to_owned()),
            ]),
        );
        true
    }

    pub(crate) fn occurrence_distribution_plan(
        &self,
        source: &OccurrenceDistributionSourcePlan,
        axis: Axis,
        mode: DistributionMode,
    ) -> Option<OccurrenceDistributionPlan> {
        let occurrence_ids = source.occurrences.keys().copied().collect();
        if self
            .occurrence_distribution_source_plan_for_ids(&occurrence_ids)
            .as_ref()
            != Some(source)
        {
            return None;
        }
        let align_mode = match mode {
            DistributionMode::Centers => AlignMode::Center,
            DistributionMode::EqualGaps => AlignMode::Minimum,
        };
        let mut items = source
            .occurrences
            .iter()
            .map(|(id, item)| {
                let coordinate = alignment_coordinate(&item.render_box, axis, align_mode);
                coordinate
                    .is_finite()
                    .then_some((*id, item.clone(), coordinate))
            })
            .collect::<Option<Vec<_>>>()?;
        items.sort_by(|left, right| {
            left.2
                .total_cmp(&right.2)
                .then_with(|| left.0.cmp(&right.0))
        });
        let first = items.first()?.2;
        let (spacing_mm, target_coordinates_mm) = match mode {
            DistributionMode::Centers => {
                let span = items.last()?.2 - first;
                if !span.is_finite() || span.abs() <= f64::EPSILON {
                    return None;
                }
                let spacing = span / (items.len() - 1) as f64;
                (
                    spacing,
                    (0..items.len())
                        .map(|index| first + spacing * index as f64)
                        .collect::<Vec<_>>(),
                )
            }
            DistributionMode::EqualGaps => {
                let last = &items.last()?.1.render_box;
                let span = alignment_coordinate(last, axis, AlignMode::Maximum) - first;
                let occupied = items
                    .iter()
                    .map(|(_, item, _)| {
                        alignment_coordinate(&item.render_box, axis, AlignMode::Maximum)
                            - alignment_coordinate(&item.render_box, axis, AlignMode::Minimum)
                    })
                    .sum::<f64>();
                let free = span - occupied;
                if !free.is_finite() || free < -f64::EPSILON {
                    return None;
                }
                let spacing = free.max(0.0) / (items.len() - 1) as f64;
                let mut target = first;
                let targets = items
                    .iter()
                    .map(|(_, item, _)| {
                        let current = target;
                        target += alignment_coordinate(&item.render_box, axis, AlignMode::Maximum)
                            - alignment_coordinate(&item.render_box, axis, AlignMode::Minimum)
                            + spacing;
                        current
                    })
                    .collect();
                (spacing, targets)
            }
        };
        if !spacing_mm.is_finite() {
            return None;
        }
        let ordered_occurrence_ids = items.iter().map(|(id, _, _)| *id).collect();
        let source_coordinates_mm = items.iter().map(|(_, _, coordinate)| *coordinate).collect();
        let mut commands = Vec::with_capacity(items.len() - 2);
        let mut preview_boxes = BTreeMap::new();
        let item_count = items.len();
        for (index, ((id, item, coordinate), target)) in items
            .into_iter()
            .zip(target_coordinates_mm.iter().copied())
            .enumerate()
        {
            if index == 0 || index + 1 == item_count {
                continue;
            }
            let offset_mm = target - coordinate;
            if offset_mm.abs() <= f64::EPSILON {
                continue;
            }
            let delta_mm = axis_vector(axis, offset_mm);
            let transform = translated_transform(item.transform, delta_mm).ok()?;
            commands.push(CanonicalCommand::SetOccurrenceTransform { id, transform });
            let mut preview_box = item.render_box;
            preview_box.origin_mm = preview_box.origin_mm + delta_mm;
            preview_boxes.insert(id, preview_box);
        }
        (!commands.is_empty()).then_some(OccurrenceDistributionPlan {
            source: source.clone(),
            axis,
            mode,
            ordered_occurrence_ids,
            source_coordinates_mm,
            target_coordinates_mm,
            spacing_mm,
            commands,
            preview_boxes,
        })
    }

    pub(crate) fn begin_occurrence_distribution(&mut self) {
        let Some(source) = self.occurrence_distribution_source_plan() else {
            return;
        };
        self.tool_preview.close::<OccurrenceOperationPreview>();
        self.modal.open(PendingOccurrenceDistribution {
            source,
            axis: Axis::X,
            mode: DistributionMode::Centers,
            preview_plan: None,
        });
    }

    #[must_use]
    pub fn occurrence_distribution_visible(&self) -> bool {
        self.modal.get::<PendingOccurrenceDistribution>().is_some()
    }

    #[must_use]
    pub fn occurrence_distribution_axis(&self) -> Option<Axis> {
        self.modal
            .get::<PendingOccurrenceDistribution>()
            .map(|pending| pending.axis)
    }

    #[must_use]
    pub fn occurrence_distribution_mode(&self) -> Option<DistributionMode> {
        self.modal
            .get::<PendingOccurrenceDistribution>()
            .map(|pending| pending.mode)
    }

    pub(crate) fn occurrence_distribution_binding_is_current(
        &self,
        pending: &PendingOccurrenceDistribution,
    ) -> bool {
        self.occurrence_distribution_source_plan().as_ref() == Some(&pending.source)
    }

    #[must_use]
    pub fn occurrence_distribution_preview_is_current(&self) -> bool {
        self.modal
            .get::<PendingOccurrenceDistribution>()
            .is_some_and(|pending| {
                let Some(plan) =
                    self.occurrence_distribution_plan(&pending.source, pending.axis, pending.mode)
                else {
                    return false;
                };
                self.occurrence_distribution_binding_is_current(pending)
                    && pending.preview_plan.as_ref() == Some(&plan)
                    && self
                        .tool_preview
                        .get::<OccurrenceOperationPreview>()
                        .is_some_and(|preview| {
                            self.has_occurrence_operation_preview()
                                && preview.source_revision == plan.source.source_revision
                                && preview.batch.commands() == plan.commands
                                && preview.boxes == plan.preview_boxes
                        })
            })
    }

    pub fn preview_pending_occurrence_distribution(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingOccurrenceDistribution>().cloned() else {
            return false;
        };
        let Some(plan) =
            self.occurrence_distribution_plan(&pending.source, pending.axis, pending.mode)
        else {
            return false;
        };
        if let Some(current) = self.modal.get_mut::<PendingOccurrenceDistribution>() {
            current.preview_plan = None;
        }
        if !self.preview_occurrence_distribution_plan(plan.clone()) {
            return false;
        }
        if let Some(current) = self.modal.get_mut::<PendingOccurrenceDistribution>() {
            current.preview_plan = Some(plan);
        }
        true
    }

    pub fn confirm_occurrence_distribution(&mut self) -> bool {
        let Some(plan) = self
            .modal
            .get::<PendingOccurrenceDistribution>()
            .and_then(|pending| pending.preview_plan.clone())
        else {
            return false;
        };
        if !self.occurrence_distribution_preview_is_current()
            || self
                .occurrence_distribution_plan(&plan.source, plan.axis, plan.mode)
                .as_ref()
                != Some(&plan)
            || !self.confirm_occurrence_operation_preview()
        {
            return false;
        }
        self.modal.close::<PendingOccurrenceDistribution>();
        true
    }

    pub fn preview_distribute_occurrences(
        &mut self,
        occurrence_ids: &BTreeSet<OccurrenceId>,
        axis: Axis,
        mode: DistributionMode,
    ) -> bool {
        self.tool_preview.close::<OccurrenceOperationPreview>();
        let Some(source) = self.occurrence_distribution_source_plan_for_ids(occurrence_ids) else {
            return false;
        };
        let Some(plan) = self.occurrence_distribution_plan(&source, axis, mode) else {
            return false;
        };
        self.preview_occurrence_distribution_plan(plan)
    }

    pub(crate) fn preview_occurrence_distribution_plan(
        &mut self,
        plan: OccurrenceDistributionPlan,
    ) -> bool {
        self.tool_preview.close::<OccurrenceOperationPreview>();
        if self
            .occurrence_distribution_plan(&plan.source, plan.axis, plan.mode)
            .as_ref()
            != Some(&plan)
        {
            return false;
        }
        let batch = CommandBatch::new(plan.commands.clone());
        self.tool_preview.open(OccurrenceOperationPreview {
            source_revision: plan.source.source_revision,
            command_digest: batch.digest(),
            batch,
            boxes: plan.preview_boxes.clone(),
            hidden_occurrences: BTreeSet::new(),
            selection_after: None,
            committed_digest_key: "digest-distribute-committed",
            canonical_plan: Some(OccurrenceCanonicalPreviewPlan::Distribution(plan.clone())),
            solid_tool_plan: None,
        });
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            match plan.mode {
                DistributionMode::Centers => "digest-distribute-live",
                DistributionMode::EqualGaps => "digest-distribute-gaps-live",
            },
            &BTreeMap::from([
                ("axis", alignment_axis_label(plan.axis).to_owned()),
                ("count", plan.source.occurrences.len().to_string()),
                ("spacing", format_height(plan.spacing_mm)),
            ]),
        );
        true
    }

    pub(crate) fn select_solid_tool_occurrence(
        &mut self,
        selection: Option<SelectionId>,
        keep_tool: bool,
    ) {
        let Some(selection) = selection else {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return;
        };
        if self.solid_tool_candidate(&selection).is_none() {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return;
        }
        if self.solid_tools.target.is_none() {
            self.solid_tools.target = Some(selection.clone());
            self.selection.select_exact(selection, false);
            self.status_key = if self.active_solid_tool_operation() == Some(BooleanOperation::Split)
            {
                "status-solid-split-tool"
            } else {
                "status-solid-tool-tool"
            };
            self.digest = self.catalog.text(
                if self.active_solid_tool_operation() == Some(BooleanOperation::Split) {
                    "digest-solid-split-target-selected"
                } else {
                    "digest-solid-tool-target-selected"
                },
            );
            return;
        }
        self.prepare_solid_tool_preview(selection, keep_tool);
    }

    pub(crate) fn canonical_occurrence_preview_is_current(
        &self,
        preview: &OccurrenceOperationPreview,
    ) -> bool {
        let Some(plan) = preview.canonical_plan.as_ref() else {
            return true;
        };
        let (source_revision, batch, boxes, committed_digest_key) = match plan {
            OccurrenceCanonicalPreviewPlan::Alignment(plan) => {
                if self
                    .occurrence_alignment_plan(&plan.source, plan.axis, plan.mode)
                    .as_ref()
                    != Some(plan.as_ref())
                {
                    return false;
                }
                (
                    plan.source.source_revision,
                    CommandBatch::new(vec![plan.command.clone()]),
                    BTreeMap::from([(plan.source.moving_id, plan.preview_box.clone())]),
                    "digest-align-committed",
                )
            }
            OccurrenceCanonicalPreviewPlan::Distribution(plan) => {
                if self
                    .occurrence_distribution_plan(&plan.source, plan.axis, plan.mode)
                    .as_ref()
                    != Some(plan)
                {
                    return false;
                }
                (
                    plan.source.source_revision,
                    CommandBatch::new(plan.commands.clone()),
                    plan.preview_boxes.clone(),
                    "digest-distribute-committed",
                )
            }
            OccurrenceCanonicalPreviewPlan::LinearPattern(plan) => {
                if self
                    .linear_pattern_plan(&plan.source, plan.axis, plan.spacing_mm, plan.count)
                    .as_ref()
                    != Some(plan)
                {
                    return false;
                }
                let Some(boxes) = self.linear_pattern_preview_boxes(plan) else {
                    return false;
                };
                (
                    plan.source.source_revision,
                    CommandBatch::new(plan.commands.clone()),
                    boxes,
                    "digest-linear-pattern-committed",
                )
            }
            OccurrenceCanonicalPreviewPlan::RectangularPattern(plan) => {
                if self
                    .rectangular_pattern_plan(
                        &plan.source,
                        RectangularPatternSpec {
                            primary_axis: plan.primary_axis,
                            primary_spacing_mm: plan.primary_spacing_mm,
                            primary_count: plan.primary_count,
                            secondary_axis: plan.secondary_axis,
                            secondary_spacing_mm: plan.secondary_spacing_mm,
                            secondary_count: plan.secondary_count,
                        },
                    )
                    .as_ref()
                    != Some(plan)
                {
                    return false;
                }
                let Some(boxes) = self.rectangular_pattern_preview_boxes(plan) else {
                    return false;
                };
                (
                    plan.source.source_revision,
                    CommandBatch::new(plan.commands.clone()),
                    boxes,
                    "digest-rectangular-pattern-committed",
                )
            }
            OccurrenceCanonicalPreviewPlan::CircularPattern(plan) => {
                if self
                    .circular_pattern_plan(
                        &plan.source,
                        plan.axis,
                        plan.centre_mm,
                        plan.angle_step_degrees,
                        plan.count,
                    )
                    .as_ref()
                    != Some(plan)
                {
                    return false;
                }
                let Some(boxes) = self.circular_pattern_preview_boxes(plan) else {
                    return false;
                };
                (
                    plan.source.source_revision,
                    CommandBatch::new(plan.commands.clone()),
                    boxes,
                    "digest-circular-pattern-committed",
                )
            }
        };
        preview.source_revision == source_revision
            && preview.batch == batch
            && preview.boxes == boxes
            && preview.hidden_occurrences.is_empty()
            && preview.selection_after.is_none()
            && preview.committed_digest_key == committed_digest_key
    }

    #[must_use]
    pub fn has_occurrence_operation_preview(&self) -> bool {
        self.tool_preview
            .get::<OccurrenceOperationPreview>()
            .is_some_and(|preview| {
                let authority_count = [
                    preview.canonical_plan.is_some(),
                    preview.solid_tool_plan.is_some(),
                ]
                .into_iter()
                .filter(|present| *present)
                .count();
                authority_count == 1
                    && self.canonical_occurrence_preview_is_current(preview)
                    && preview.source_revision == self.document.current().revision_id()
                    && preview.command_digest == preview.batch.digest()
                    && preview.solid_tool_plan.as_ref().is_none_or(|plan| {
                        self.solid_tool_preview_plan(&plan.source).as_ref() == Some(plan)
                            && preview.batch == CommandBatch::new(vec![plan.command.clone()])
                            && preview.boxes
                                == BTreeMap::from([(
                                    plan.source.target_selection.instance_path.root_occurrence(),
                                    plan.preview_box.clone(),
                                )])
                            && preview.hidden_occurrences == plan.hidden_occurrences
                            && preview.selection_after.as_ref() == Some(&plan.selection_after)
                            && preview.committed_digest_key == plan.committed_digest_key
                    })
            })
    }

    #[must_use]
    pub fn occurrence_operation_preview_geometry(
        &self,
        occurrence_id: OccurrenceId,
    ) -> Option<(Vec3, Vec3)> {
        self.tool_preview
            .get::<OccurrenceOperationPreview>()?
            .boxes
            .get(&occurrence_id)
            .map(|item| (item.origin_mm, item.size_mm))
    }

    pub fn confirm_occurrence_operation_preview(&mut self) -> bool {
        if !self.has_occurrence_operation_preview() {
            self.tool_preview.close::<OccurrenceOperationPreview>();
            self.status_key = "error-preview-stale";
            return false;
        }
        let Some(preview) = self.tool_preview.remove::<OccurrenceOperationPreview>() else {
            return false;
        };
        if self.apply_batch_with_work_recovery(&preview.batch).is_err() {
            self.status_key = "error-preview-stale";
            return false;
        }
        if let Some(selection) = preview.selection_after {
            self.selection.select_exact(selection, false);
        }
        self.solid_tools.target = None;
        self.status_key = "status-ready";
        self.digest = self.catalog.text(preview.committed_digest_key);
        true
    }

    pub(crate) fn duplicate_source_plan(&self) -> Option<DuplicateSourcePlan> {
        if !self.selection.edit_context.is_empty() {
            return None;
        }
        let CopySourcePlan {
            occurrence_ids: source_occurrence_ids,
            occurrence_count: source_occurrence_count,
        } = self.copy_source_plan()?;
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
        let mut duplicated = Vec::new();

        for source_id in source_occurrence_ids.iter().copied() {
            let source = snapshot.occurrence(source_id)?;
            let definition = snapshot.definition(source.definition_id())?;
            let transform =
                translated_transform(source.transform(), Vec3::new(100.0, 100.0, 0.0)).ok()?;
            let existing = snapshot
                .scene_query()
                .into_iter()
                .filter(|item| item.definition_id == definition.id())
                .count();
            let added = additions_by_definition.entry(definition.id()).or_default();
            *added += 1;
            let target_id = OccurrenceId(next_id);
            next_id = next_id.checked_add(1)?;
            commands.push(CanonicalCommand::CreateOccurrence {
                id: target_id,
                definition_id: definition.id(),
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", definition.name().to_owned()),
                        ("number", (existing + *added).to_string()),
                    ]),
                ),
                transform,
                parent: source.parent(),
                tag: source.tag(),
                visible: source.visible(),
            });
            if let Some(color) = source.color() {
                commands.push(CanonicalCommand::SetOccurrenceColor {
                    id: target_id,
                    color: Some(color),
                });
            }
            duplicated.push((target_id, definition.id()));
        }
        Some(DuplicateSourcePlan {
            source_revision,
            source_occurrence_ids,
            source_occurrence_count,
            commands,
            duplicated,
        })
    }

    pub(crate) fn apply_duplicate_source_plan(&mut self, plan: DuplicateSourcePlan) -> bool {
        if plan.source_revision != self.document.current().revision_id()
            || plan.source_occurrence_count != plan.source_occurrence_ids.len()
            || plan
                .commands
                .iter()
                .filter(|command| matches!(command, CanonicalCommand::CreateOccurrence { .. }))
                .count()
                != plan.source_occurrence_count
            || plan.duplicated.len() != plan.source_occurrence_count
            || self.duplicate_source_plan().as_ref() != Some(&plan)
        {
            return false;
        }
        let DuplicateSourcePlan {
            commands,
            duplicated,
            ..
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
            .extend(duplicated.iter().map(|(id, _)| InstancePath::root(*id)));
        if let Some((occurrence_id, definition_id)) = duplicated.first().copied() {
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
            "digest-duplicated-selection",
            &BTreeMap::from([("count", duplicated.len().to_string())]),
        );
        true
    }

    pub(crate) fn duplicate_selection(&mut self) -> bool {
        let Some(plan) = self.duplicate_source_plan() else {
            return false;
        };
        self.apply_duplicate_source_plan(plan)
    }

    /// The definition a push/pull preview is reshaping.
    pub(crate) fn push_pull_preview_definition(&self) -> Option<DefinitionId> {
        self.tool_preview
            .get::<EphemeralBoxPreview>()
            .map(|preview| preview.plan.source.target.definition_id)
    }

    pub(crate) fn group_contains_occurrence(
        snapshot: &Snapshot,
        group_id: GroupId,
        occurrence_id: OccurrenceId,
    ) -> bool {
        let mut parent = snapshot
            .occurrence(occurrence_id)
            .and_then(|occurrence| occurrence.parent());
        while let Some(candidate) = parent {
            if candidate == group_id {
                return true;
            }
            parent = snapshot.group(candidate).and_then(|group| group.parent());
        }
        false
    }

    /// The bounds of `definition_id` in its own space, taken from whatever
    /// geometry the viewport actually paints for it.
    ///
    /// An imported exact body carries no canonical box, so a Rotate pivot read
    /// from the box proxies alone would silently miss every STEP part.
    pub(crate) fn definition_local_bounds(
        &self,
        snapshot: &Snapshot,
        definition_id: DefinitionId,
        local_box: Option<ProjectedBox>,
        use_exact_bounds: bool,
    ) -> Option<[Vec3; 2]> {
        if let Some(package) = use_exact_bounds
            .then(|| self.exact_results_for_snapshot(snapshot))
            .flatten()
            .and_then(|results| results.get_render(snapshot, definition_id))
        {
            let [minimum, maximum] = package.bounds_mm();
            return Some([
                Vec3::new(minimum[0], minimum[1], minimum[2]),
                Vec3::new(maximum[0], maximum[1], maximum[2]),
            ]);
        }
        if let Some(definition) = snapshot.definition(definition_id) {
            if let [feature_id] = definition.feature_ids()
                && let Some(feature) = snapshot.feature(*feature_id)
                && let FeatureKind::ImportedExactBody(spec) = feature.kind()
            {
                return Some([
                    Vec3::new(
                        spec.bounds_mm[0][0],
                        spec.bounds_mm[0][1],
                        spec.bounds_mm[0][2],
                    ),
                    Vec3::new(
                        spec.bounds_mm[1][0],
                        spec.bounds_mm[1][1],
                        spec.bounds_mm[1][2],
                    ),
                ]);
            }
            if let Some([minimum, maximum]) =
                definition
                    .feature_ids()
                    .iter()
                    .rev()
                    .find_map(|feature_id| {
                        ExactBRepGraph::from_snapshot(snapshot, definition_id, *feature_id)
                            .ok()?
                            .producer_bounds_mm()
                            .ok()?
                    })
            {
                return Some([
                    Vec3::new(minimum[0], minimum[1], minimum[2]),
                    Vec3::new(maximum[0], maximum[1], maximum[2]),
                ]);
            }
            for feature_id in definition.feature_ids() {
                if let Some(feature) = snapshot.feature(*feature_id)
                    && let FeatureKind::MeshBody(mesh) = feature.kind()
                {
                    return bounds_of(
                        mesh.vertices_mm
                            .iter()
                            .map(|vertex| Vec3::new(vertex[0], vertex[1], vertex[2])),
                    );
                }
            }
            if let Some((_, vertices, _)) = canonical_sketch_profile_mesh(snapshot, definition_id) {
                return bounds_of(
                    vertices
                        .into_iter()
                        .map(|vertex| Vec3::new(vertex[0], vertex[1], vertex[2])),
                );
            }
        }
        local_box.map(|item| [item.origin_mm, item.origin_mm + item.size_mm])
    }

    pub(crate) fn show_occurrence_rename_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingOccurrenceRename>().cloned() else {
            return;
        };
        let mut name = pending.name.clone();
        let mut open = true;
        let mut rename = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-rename-occurrence-title"))
            .id(egui::Id::new("rename-occurrence"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                let input_label = self.catalog.text("dialog-rename-occurrence-name");
                ui.label(&input_label);
                let input = ui.add(egui::TextEdit::singleline(&mut name));
                input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &input_label)
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let mut candidate = pending.clone();
                    candidate.name = name.clone();
                    rename = ui
                        .add_enabled(
                            self.occurrence_rename_plan(&candidate).is_some(),
                            egui::Button::new(
                                self.catalog.text("dialog-rename-occurrence-confirm"),
                            ),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-rename-occurrence-cancel"))
                        .clicked();
                });
            });
        if let Some(pending) = self.modal.get_mut::<PendingOccurrenceRename>() {
            pending.name = name;
        }
        if cancel || !open {
            self.modal.close::<PendingOccurrenceRename>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if rename {
            self.confirm_occurrence_rename();
        }
    }

    pub(crate) fn show_definition_rename_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingDefinitionRename>().cloned() else {
            return;
        };
        let mut name = pending.name.clone();
        let mut open = true;
        let mut rename = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-rename-definition-title"))
            .id(egui::Id::new("rename-definition"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                let input_label = self.catalog.text("dialog-rename-definition-name");
                ui.label(&input_label);
                let input = ui.add(egui::TextEdit::singleline(&mut name));
                input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &input_label)
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let mut candidate = pending.clone();
                    candidate.name = name.clone();
                    rename = ui
                        .add_enabled(
                            self.definition_rename_plan(&candidate).is_some(),
                            egui::Button::new(
                                self.catalog.text("dialog-rename-definition-confirm"),
                            ),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-rename-definition-cancel"))
                        .clicked();
                });
            });
        if let Some(pending) = self.modal.get_mut::<PendingDefinitionRename>() {
            pending.name = name;
        }
        if cancel || !open {
            self.modal.close::<PendingDefinitionRename>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if rename {
            self.confirm_definition_rename();
        }
    }

    pub(crate) fn show_component_replacement_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingComponentReplacement>().cloned() else {
            return;
        };
        let mut definition_id = pending.target_definition_id;
        let options = pending
            .source
            .candidate_definitions
            .iter()
            .map(|(id, name)| (*id, name.clone()))
            .collect::<Vec<_>>();
        let mut open = true;
        let mut replace = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-replace-component-title"))
            .id(egui::Id::new("replace-component"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.text("dialog-replace-component-definition"));
                for (candidate, name) in &options {
                    ui.selectable_value(&mut definition_id, *candidate, name);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    let mut candidate = pending.clone();
                    candidate.target_definition_id = definition_id;
                    replace = ui
                        .add_enabled(
                            self.component_replacement_plan(&candidate).is_some(),
                            egui::Button::new(
                                self.catalog.text("dialog-replace-component-confirm"),
                            ),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-replace-component-cancel"))
                        .clicked();
                });
            });
        if let Some(pending) = self.modal.get_mut::<PendingComponentReplacement>() {
            pending.target_definition_id = definition_id;
        }
        if cancel || !open {
            self.modal.close::<PendingComponentReplacement>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if replace {
            self.confirm_component_replacement();
        }
    }

    pub(crate) fn show_occurrence_align_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingOccurrenceAlign>() else {
            return;
        };
        let previous_axis = pending.axis;
        let previous_mode = pending.mode;
        let mut axis = previous_axis;
        let mut mode = previous_mode;
        let binding_is_current = self.occurrence_alignment_binding_is_current(pending);
        let preview_is_current = self.occurrence_align_preview_is_current();
        let mut open = true;
        let mut preview = false;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-align-occurrences-title"))
            .id(egui::Id::new("align-occurrences"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.text("dialog-align-occurrences-semantics"));
                ui.label(self.catalog.text("dialog-align-occurrences-axis"));
                ui.horizontal(|ui| {
                    for (candidate, key) in [
                        (Axis::X, "dialog-align-occurrences-axis-x"),
                        (Axis::Y, "dialog-align-occurrences-axis-y"),
                        (Axis::Z, "dialog-align-occurrences-axis-z"),
                    ] {
                        ui.selectable_value(&mut axis, candidate, self.catalog.text(key));
                    }
                });
                ui.label(self.catalog.text("dialog-align-occurrences-mode"));
                ui.horizontal(|ui| {
                    for (candidate, key) in [
                        (AlignMode::Minimum, "dialog-align-occurrences-minimum"),
                        (AlignMode::Center, "dialog-align-occurrences-center"),
                        (AlignMode::Maximum, "dialog-align-occurrences-maximum"),
                    ] {
                        ui.selectable_value(&mut mode, candidate, self.catalog.text(key));
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    preview = ui
                        .add_enabled(
                            binding_is_current,
                            egui::Button::new(
                                self.catalog.text("dialog-align-occurrences-preview"),
                            ),
                        )
                        .clicked();
                    confirm = ui
                        .add_enabled(
                            preview_is_current,
                            egui::Button::new(
                                self.catalog.text("dialog-align-occurrences-confirm"),
                            ),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-align-occurrences-cancel"))
                        .clicked();
                });
            });
        let changed = axis != previous_axis || mode != previous_mode;
        if let Some(pending) = self.modal.get_mut::<PendingOccurrenceAlign>() {
            pending.axis = axis;
            pending.mode = mode;
            if changed {
                pending.preview_plan = None;
            }
        }
        if changed {
            self.tool_preview.close::<OccurrenceOperationPreview>();
        }
        if cancel || !open {
            self.modal.close::<PendingOccurrenceAlign>();
            self.tool_preview.close::<OccurrenceOperationPreview>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if preview {
            self.preview_pending_occurrence_align();
        } else if confirm {
            self.confirm_occurrence_align();
        }
    }

    pub(crate) fn show_occurrence_distribution_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingOccurrenceDistribution>() else {
            return;
        };
        let previous_axis = pending.axis;
        let previous_mode = pending.mode;
        let mut axis = previous_axis;
        let mut mode = previous_mode;
        let binding_is_current = self.occurrence_distribution_binding_is_current(pending);
        let preview_is_current = self.occurrence_distribution_preview_is_current();
        let mut open = true;
        let mut preview = false;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-distribute-occurrences-title"))
            .id(egui::Id::new("distribute-occurrences"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.text(match mode {
                    DistributionMode::Centers => "dialog-distribute-occurrences-centers-semantics",
                    DistributionMode::EqualGaps => "dialog-distribute-occurrences-gaps-semantics",
                }));
                ui.label(self.catalog.text("dialog-distribute-occurrences-mode"));
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut mode,
                        DistributionMode::Centers,
                        self.catalog.text("dialog-distribute-occurrences-centers"),
                    );
                    ui.selectable_value(
                        &mut mode,
                        DistributionMode::EqualGaps,
                        self.catalog
                            .text("dialog-distribute-occurrences-equal-gaps"),
                    );
                });
                ui.label(self.catalog.text("dialog-distribute-occurrences-axis"));
                ui.horizontal(|ui| {
                    for (candidate, key) in [
                        (Axis::X, "dialog-distribute-occurrences-axis-x"),
                        (Axis::Y, "dialog-distribute-occurrences-axis-y"),
                        (Axis::Z, "dialog-distribute-occurrences-axis-z"),
                    ] {
                        ui.selectable_value(&mut axis, candidate, self.catalog.text(key));
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    preview = ui
                        .add_enabled(
                            binding_is_current,
                            egui::Button::new(
                                self.catalog.text("dialog-distribute-occurrences-preview"),
                            ),
                        )
                        .clicked();
                    confirm = ui
                        .add_enabled(
                            preview_is_current,
                            egui::Button::new(
                                self.catalog.text("dialog-distribute-occurrences-confirm"),
                            ),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-distribute-occurrences-cancel"))
                        .clicked();
                });
            });
        let changed = axis != previous_axis || mode != previous_mode;
        if let Some(pending) = self.modal.get_mut::<PendingOccurrenceDistribution>() {
            pending.axis = axis;
            pending.mode = mode;
            if changed {
                pending.preview_plan = None;
            }
        }
        if changed {
            self.tool_preview.close::<OccurrenceOperationPreview>();
        }
        if cancel || !open {
            self.modal.close::<PendingOccurrenceDistribution>();
            self.tool_preview.close::<OccurrenceOperationPreview>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if preview {
            self.preview_pending_occurrence_distribution();
        } else if confirm {
            self.confirm_occurrence_distribution();
        }
    }
}

fn alignment_coordinate(item: &RenderBox, axis: Axis, mode: AlignMode) -> f64 {
    let (origin, size) = match axis {
        Axis::X => (item.origin_mm.x, item.size_mm.x),
        Axis::Y => (item.origin_mm.y, item.size_mm.y),
        Axis::Z => (item.origin_mm.z, item.size_mm.z),
    };
    match mode {
        AlignMode::Minimum => origin,
        AlignMode::Center => origin + size * 0.5,
        AlignMode::Maximum => origin + size,
    }
}

const fn alignment_mode_label(mode: AlignMode) -> &'static str {
    match mode {
        AlignMode::Minimum => "minimum",
        AlignMode::Center => "center",
        AlignMode::Maximum => "maximum",
    }
}

pub(crate) fn translated_in_parent_space(
    snapshot: &Snapshot,
    parent: Option<GroupId>,
    local: Transform,
    delta: Vec3,
) -> Option<Transform> {
    world_edit_in_parent_space(
        snapshot,
        parent,
        local,
        Transform::from_translation(delta.x, delta.y, delta.z).ok()?,
    )
}
