//! Tags and classification: creating, renaming, deleting, assigning, selecting by and showing or hiding tags.

use crate::*;

/// Commands that add (`member`) or remove `tag` on each occurrence whose membership
/// differs, keeping its other tags.
fn tag_membership_commands(
    snapshot: &Snapshot,
    occurrence_ids: impl IntoIterator<Item = OccurrenceId>,
    tag: TagId,
    member: bool,
) -> Vec<CanonicalCommand> {
    occurrence_ids
        .into_iter()
        .filter_map(|id| {
            let mut tags = snapshot.occurrence(id)?.tags().clone();
            let changed = if member {
                tags.insert(tag)
            } else {
                tags.remove(&tag)
            };
            changed.then_some(CanonicalCommand::SetOccurrenceTags { id, tags })
        })
        .collect()
}

impl KetchupApp {
    pub(crate) fn next_tag_id(&self) -> Option<TagId> {
        self.document
            .current()
            .tags()
            .map(|tag| tag.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(TagId)
    }

    pub(crate) fn tag_creation_source_plan(
        &self,
        occurrence_ids: Option<&BTreeSet<OccurrenceId>>,
    ) -> Option<TagCreationSourcePlan> {
        let snapshot = self.document.current();
        let id = self.next_tag_id()?;
        if let Some(ids) = occurrence_ids
            && (ids.is_empty()
                || self.selected_occurrence_ids() != *ids
                || !ids.iter().all(|id| snapshot.occurrence(*id).is_some()))
        {
            return None;
        }
        Some(TagCreationSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            tags: snapshot
                .tags()
                .map(|tag| (tag.id(), (tag.name().to_owned(), tag.visible())))
                .collect(),
            id,
            occurrence_ids: occurrence_ids.cloned(),
        })
    }

    pub(crate) fn can_begin_tag_creation(
        &self,
        occurrence_ids: Option<&BTreeSet<OccurrenceId>>,
    ) -> bool {
        self.tag_creation_source_plan(occurrence_ids).is_some()
    }

    pub(crate) fn tag_creation_plan(
        &self,
        source: &TagCreationSourcePlan,
        candidate: &str,
    ) -> Option<TagCreationPlan> {
        (self
            .tag_creation_source_plan(source.occurrence_ids.as_ref())
            .as_ref()
            == Some(source))
        .then_some(())?;
        let target_name = candidate.trim();
        if target_name.is_empty() || source.tags.values().any(|(name, _)| name == target_name) {
            return None;
        }
        let mut commands = vec![CanonicalCommand::CreateTag {
            id: source.id,
            name: target_name.to_owned(),
            visible: true,
        }];
        if let Some(occurrence_ids) = &source.occurrence_ids {
            commands.extend(tag_membership_commands(
                &self.document.current(),
                occurrence_ids.iter().copied(),
                source.id,
                true,
            ));
        }
        Some(TagCreationPlan {
            source: source.clone(),
            target_name: target_name.to_owned(),
            commands,
        })
    }

    pub(crate) fn begin_tag_creation(&mut self, occurrence_ids: Option<BTreeSet<OccurrenceId>>) {
        let Some(source) = self.tag_creation_source_plan(occurrence_ids.as_ref()) else {
            return;
        };
        self.modal.open(PendingTagCreation {
            source,
            name: String::new(),
        });
    }

    #[must_use]
    pub fn tag_creation_visible(&self) -> bool {
        self.modal.get::<PendingTagCreation>().is_some()
    }

    #[must_use]
    pub fn tag_creation_input(&self) -> Option<&str> {
        self.modal
            .get::<PendingTagCreation>()
            .map(|pending| pending.name.as_str())
    }

    pub(crate) fn apply_tag_creation_plan(&mut self, plan: TagCreationPlan) -> bool {
        if self
            .tag_creation_plan(&plan.source, &plan.target_name)
            .as_ref()
            != Some(&plan)
        {
            return false;
        }
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(plan.commands.clone()))
            .is_err()
        {
            return false;
        }
        self.modal.close::<PendingTagCreation>();
        self.digest = if let Some(occurrence_ids) = plan.source.occurrence_ids {
            self.catalog.format(
                "digest-created-tag-from-selection",
                &BTreeMap::from([
                    ("name", plan.target_name),
                    ("count", occurrence_ids.len().to_string()),
                ]),
            )
        } else {
            self.catalog.format(
                "digest-created-tag",
                &BTreeMap::from([("name", plan.target_name)]),
            )
        };
        true
    }

    pub fn confirm_tag_creation(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingTagCreation>().cloned() else {
            return false;
        };
        let Some(plan) = self.tag_creation_plan(&pending.source, &pending.name) else {
            return false;
        };
        self.apply_tag_creation_plan(plan)
    }

    pub(crate) fn tag_deletion_source_plan(&self, id: TagId) -> Option<TagDeletionSourcePlan> {
        let snapshot = self.document.current();
        let tag = snapshot.tag(id)?;
        if snapshot
            .local_occurrences()
            .any(|occurrence| occurrence.tags().contains(&id))
        {
            return None;
        }
        Some(TagDeletionSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            tags: snapshot
                .tags()
                .map(|tag| (tag.id(), (tag.name().to_owned(), tag.visible())))
                .collect(),
            id,
            original_name: tag.name().to_owned(),
            original_visible: tag.visible(),
            occurrence_ids: snapshot
                .occurrences_with_tag(id)
                .map(|occurrence| occurrence.id())
                .collect(),
        })
    }

    pub(crate) fn tag_deletion_plan(
        &self,
        source: &TagDeletionSourcePlan,
    ) -> Option<TagDeletionPlan> {
        (self.tag_deletion_source_plan(source.id).as_ref() == Some(source)).then_some(())?;
        let mut commands = tag_membership_commands(
            &self.document.current(),
            source.occurrence_ids.iter().copied(),
            source.id,
            false,
        );
        commands.push(CanonicalCommand::DeleteTag { id: source.id });
        Some(TagDeletionPlan {
            source: source.clone(),
            commands,
        })
    }

    pub(crate) fn can_delete_tag(&self, id: TagId) -> bool {
        self.tag_deletion_source_plan(id).is_some()
    }

    pub(crate) fn begin_tag_deletion(&mut self, id: TagId) {
        let Some(source) = self.tag_deletion_source_plan(id) else {
            return;
        };
        self.modal.open(PendingTagDeletion { source });
    }

    #[must_use]
    pub fn tag_deletion_visible(&self) -> bool {
        self.modal.get::<PendingTagDeletion>().is_some()
    }

    pub(crate) fn apply_tag_deletion_plan(&mut self, plan: TagDeletionPlan) -> bool {
        if self.tag_deletion_plan(&plan.source).as_ref() != Some(&plan) {
            return false;
        }
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(plan.commands.clone()))
            .is_err()
        {
            return false;
        }
        let count = plan.source.occurrence_ids.len();
        self.modal.close::<PendingTagDeletion>();
        self.digest = self.catalog.format(
            if count == 0 {
                "digest-deleted-tag"
            } else {
                "digest-deleted-used-tag"
            },
            &BTreeMap::from([
                ("name", plan.source.original_name),
                ("count", count.to_string()),
            ]),
        );
        true
    }

    pub fn confirm_tag_deletion(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingTagDeletion>().cloned() else {
            return false;
        };
        let Some(plan) = self.tag_deletion_plan(&pending.source) else {
            return false;
        };
        self.apply_tag_deletion_plan(plan)
    }

    pub(crate) fn tag_clear_source_plan(&self, id: TagId) -> Option<TagClearSourcePlan> {
        let snapshot = self.document.current();
        let tag = snapshot.tag(id)?;
        let occurrence_ids = snapshot
            .occurrences_with_tag(id)
            .map(|occurrence| occurrence.id())
            .collect::<BTreeSet<_>>();
        (!occurrence_ids.is_empty()).then(|| TagClearSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            tags: snapshot
                .tags()
                .map(|tag| (tag.id(), (tag.name().to_owned(), tag.visible())))
                .collect(),
            id,
            original_name: tag.name().to_owned(),
            original_visible: tag.visible(),
            occurrence_ids,
        })
    }

    pub(crate) fn tag_clear_plan(&self, source: &TagClearSourcePlan) -> Option<TagClearPlan> {
        (self.tag_clear_source_plan(source.id).as_ref() == Some(source)).then(|| TagClearPlan {
            source: source.clone(),
            commands: tag_membership_commands(
                &self.document.current(),
                source.occurrence_ids.iter().copied(),
                source.id,
                false,
            ),
        })
    }

    pub(crate) fn can_clear_tag(&self, id: TagId) -> bool {
        self.tag_clear_source_plan(id).is_some()
    }

    pub(crate) fn begin_tag_clear(&mut self, id: TagId) {
        let Some(source) = self.tag_clear_source_plan(id) else {
            return;
        };
        self.modal.open(PendingTagClear { source });
    }

    #[must_use]
    pub fn tag_clear_visible(&self) -> bool {
        self.modal.get::<PendingTagClear>().is_some()
    }

    pub(crate) fn apply_tag_clear_plan(&mut self, plan: TagClearPlan) -> bool {
        if self.tag_clear_plan(&plan.source).as_ref() != Some(&plan) {
            return false;
        }
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(plan.commands.clone()))
            .is_err()
        {
            return false;
        }
        let count = plan.source.occurrence_ids.len();
        self.modal.close::<PendingTagClear>();
        self.digest = self.catalog.format(
            "digest-cleared-tag",
            &BTreeMap::from([
                ("name", plan.source.original_name),
                ("count", count.to_string()),
            ]),
        );
        true
    }

    pub fn confirm_tag_clear(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingTagClear>().cloned() else {
            return false;
        };
        let Some(plan) = self.tag_clear_plan(&pending.source) else {
            return false;
        };
        self.apply_tag_clear_plan(plan)
    }

    pub(crate) fn tag_rename_source_plan(&self, id: TagId) -> Option<TagRenameSourcePlan> {
        let snapshot = self.document.current();
        let tag = snapshot.tag(id)?;
        Some(TagRenameSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            tags: snapshot
                .tags()
                .map(|tag| (tag.id(), (tag.name().to_owned(), tag.visible())))
                .collect(),
            id,
            original_name: tag.name().to_owned(),
            original_visible: tag.visible(),
            occurrence_ids: snapshot
                .occurrences_with_tag(id)
                .map(|occurrence| occurrence.id())
                .collect(),
        })
    }

    pub(crate) fn tag_rename_plan(
        &self,
        source: &TagRenameSourcePlan,
        candidate: &str,
    ) -> Option<TagRenamePlan> {
        (self.tag_rename_source_plan(source.id).as_ref() == Some(source)).then_some(())?;
        let target_name = candidate.trim();
        (!target_name.is_empty()
            && target_name != source.original_name
            && !source
                .tags
                .iter()
                .any(|(id, (name, _))| *id != source.id && name == target_name))
        .then(|| TagRenamePlan {
            source: source.clone(),
            target_name: target_name.to_owned(),
            command: CanonicalCommand::SetTagName {
                id: source.id,
                name: target_name.to_owned(),
            },
        })
    }

    pub(crate) fn begin_tag_rename(&mut self, id: TagId) {
        let Some(source) = self.tag_rename_source_plan(id) else {
            return;
        };
        let name = source.original_name.clone();
        self.modal.open(PendingTagRename { source, name });
    }

    #[must_use]
    pub fn tag_rename_visible(&self) -> bool {
        self.modal.get::<PendingTagRename>().is_some()
    }

    #[must_use]
    pub fn tag_rename_input(&self) -> Option<&str> {
        self.modal
            .get::<PendingTagRename>()
            .map(|pending| pending.name.as_str())
    }

    pub(crate) fn apply_tag_rename_plan(&mut self, plan: TagRenamePlan) -> bool {
        if self
            .tag_rename_plan(&plan.source, &plan.target_name)
            .as_ref()
            != Some(&plan)
        {
            return false;
        }
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![plan.command.clone()]))
            .is_err()
        {
            return false;
        }
        self.modal.close::<PendingTagRename>();
        self.digest = self.catalog.format(
            "digest-renamed-tag",
            &BTreeMap::from([
                ("old_name", plan.source.original_name),
                ("name", plan.target_name),
            ]),
        );
        true
    }

    pub fn confirm_tag_rename(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingTagRename>().cloned() else {
            return false;
        };
        let Some(plan) = self.tag_rename_plan(&pending.source, &pending.name) else {
            return false;
        };
        self.apply_tag_rename_plan(plan)
    }

    pub(crate) fn tag_options(&self) -> Vec<(TagId, String)> {
        self.document
            .current()
            .tags()
            .map(|tag| (tag.id(), tag.name().to_owned()))
            .collect()
    }

    pub(crate) fn tag_rows(&self) -> Vec<(TagId, String, bool, usize)> {
        let snapshot = self.document.current();
        snapshot
            .tags()
            .map(|tag| {
                (
                    tag.id(),
                    tag.name().to_owned(),
                    tag.visible(),
                    snapshot.occurrences_with_tag(tag.id()).count(),
                )
            })
            .collect()
    }

    pub(crate) fn tag_occurrence_paths(
        &self,
        id: TagId,
    ) -> Option<(String, BTreeSet<InstancePath>)> {
        let snapshot = self.document.current();
        let name = snapshot.tag(id)?.name().to_owned();
        let paths = self
            .active_scene_query()
            .into_iter()
            .filter(|item| {
                item.instance_path.is_root()
                    && snapshot
                        .occurrence(item.instance_path.root_occurrence())
                        .is_some_and(|occurrence| occurrence.tags().contains(&id))
            })
            .map(|item| item.instance_path)
            .collect();
        Some((name, paths))
    }

    pub(crate) fn can_select_tag_occurrences(&self, id: TagId) -> bool {
        let Some((_, paths)) = self.tag_occurrence_paths(id) else {
            return false;
        };
        !paths.is_empty()
            && (self.selection.primary.is_some()
                || self.selection.selected_group.is_some()
                || self.selection.occurrences != paths)
    }

    pub fn select_tag_occurrences(&mut self, id: TagId) -> bool {
        let Some((name, paths)) = self.tag_occurrence_paths(id) else {
            return false;
        };
        if paths.is_empty()
            || (self.selection.primary.is_none()
                && self.selection.selected_group.is_none()
                && self.selection.occurrences == paths)
        {
            return false;
        }
        let count = paths.len();
        self.selection.clear();
        self.selection.occurrences = paths;
        self.digest = self.catalog.format(
            "digest-selected-tag",
            &BTreeMap::from([("name", name), ("count", count.to_string())]),
        );
        true
    }

    pub(crate) fn all_tagged_selection_plan(&self) -> Option<BTreeSet<InstancePath>> {
        let snapshot = self.document.current();
        let paths = self
            .active_scene_query()
            .into_iter()
            .filter(|item| {
                item.instance_path.is_root()
                    && snapshot
                        .occurrence(item.instance_path.root_occurrence())
                        .is_some_and(|occurrence| !occurrence.tags().is_empty())
            })
            .map(|item| item.instance_path)
            .collect::<BTreeSet<_>>();
        (!paths.is_empty()
            && (self.selection.primary.is_some()
                || self.selection.selected_group.is_some()
                || self.selection.occurrences != paths))
            .then_some(paths)
    }

    pub(crate) fn can_select_all_tagged_occurrences(&self) -> bool {
        self.all_tagged_selection_plan().is_some()
    }

    pub fn select_all_tagged_occurrences(&mut self) -> bool {
        let Some(paths) = self.all_tagged_selection_plan() else {
            return false;
        };
        let count = paths.len();
        self.selection.clear();
        self.selection.occurrences = paths;
        self.digest = self.catalog.format(
            "digest-selected-all-tagged",
            &BTreeMap::from([("count", count.to_string())]),
        );
        true
    }

    pub(crate) fn all_untagged_selection_plan(&self) -> Option<BTreeSet<InstancePath>> {
        let snapshot = self.document.current();
        let paths = self
            .active_scene_query()
            .into_iter()
            .filter(|item| {
                item.instance_path.is_root()
                    && snapshot
                        .occurrence(item.instance_path.root_occurrence())
                        .is_some_and(|occurrence| occurrence.tags().is_empty())
            })
            .map(|item| item.instance_path)
            .collect::<BTreeSet<_>>();
        (!paths.is_empty()
            && (self.selection.primary.is_some()
                || self.selection.selected_group.is_some()
                || self.selection.occurrences != paths))
            .then_some(paths)
    }

    pub(crate) fn can_select_untagged_occurrences(&self) -> bool {
        self.all_untagged_selection_plan().is_some()
    }

    pub fn select_untagged_occurrences(&mut self) -> bool {
        let Some(paths) = self.all_untagged_selection_plan() else {
            return false;
        };
        let count = paths.len();
        self.selection.clear();
        self.selection.occurrences = paths;
        self.digest = self.catalog.format(
            "digest-selected-untagged",
            &BTreeMap::from([("count", count.to_string())]),
        );
        true
    }

    pub(crate) fn tag_assignment_plan(&self, id: TagId) -> Option<(String, Vec<CanonicalCommand>)> {
        let snapshot = self.document.current();
        let name = snapshot.tag(id)?.name().to_owned();
        let commands = tag_membership_commands(&snapshot, self.selected_occurrence_ids(), id, true);
        (!commands.is_empty()).then_some((name, commands))
    }

    pub(crate) fn can_assign_selection_to_tag(&self, id: TagId) -> bool {
        self.tag_assignment_plan(id).is_some()
    }

    pub(crate) fn tag_removal_plan(&self, id: TagId) -> Option<(String, Vec<CanonicalCommand>)> {
        let snapshot = self.document.current();
        let name = snapshot.tag(id)?.name().to_owned();
        let commands =
            tag_membership_commands(&snapshot, self.selected_occurrence_ids(), id, false);
        (!commands.is_empty()).then_some((name, commands))
    }

    pub(crate) fn can_remove_selection_from_tag(&self, id: TagId) -> bool {
        self.tag_removal_plan(id).is_some()
    }

    pub fn assign_selection_to_tag(&mut self, id: TagId) -> bool {
        let Some((name, commands)) = self.tag_assignment_plan(id) else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-assigned-tag",
            &BTreeMap::from([("count", count.to_string()), ("tag", name)]),
        );
        true
    }

    pub fn remove_selection_from_tag(&mut self, id: TagId) -> bool {
        let Some((name, commands)) = self.tag_removal_plan(id) else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-removed-selected-tag",
            &BTreeMap::from([("count", count.to_string()), ("tag", name)]),
        );
        true
    }

    pub(crate) fn classification_rows(&self) -> Vec<ClassificationDimensionRow> {
        let snapshot = self.document.current();
        snapshot
            .classification_dimensions()
            .map(|dimension| {
                let categories = dimension
                    .categories()
                    .map(|category| {
                        let count = snapshot
                            .occurrences()
                            .filter(|occurrence| {
                                snapshot.occurrence_classification(occurrence.id(), dimension.id())
                                    == Some(category.id())
                            })
                            .count();
                        ClassificationCategoryRow {
                            id: category.id(),
                            name: category.name().to_owned(),
                            occurrence_count: count,
                        }
                    })
                    .collect();
                ClassificationDimensionRow {
                    id: dimension.id(),
                    name: dimension.name().to_owned(),
                    categories,
                }
            })
            .collect()
    }

    pub fn create_classification_dimension(
        &mut self,
        name: &str,
        first_category_name: &str,
    ) -> bool {
        let name = name.trim();
        let first_category_name = first_category_name.trim();
        let snapshot = self.document.current();
        if name.is_empty()
            || first_category_name.is_empty()
            || snapshot
                .classification_dimensions()
                .any(|dimension| dimension.name() == name)
        {
            return false;
        }
        let Some(dimension_id) = snapshot
            .classification_dimensions()
            .map(|dimension| dimension.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(ClassificationDimensionId)
        else {
            return false;
        };
        let Some(category_id) = snapshot
            .classification_dimensions()
            .flat_map(|dimension| dimension.categories())
            .map(|category| category.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(ClassificationCategoryId)
        else {
            return false;
        };
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![
                CanonicalCommand::UpsertClassificationDimension {
                    id: dimension_id,
                    name: name.to_owned(),
                    categories: vec![(category_id, first_category_name.to_owned())],
                },
            ]))
            .is_err()
        {
            return false;
        }
        self.classification.selected_dimension = Some(dimension_id);
        self.classification.dimension_name_input.clear();
        self.classification.category_name_input.clear();
        self.digest = self.catalog.format(
            "digest-created-dimension",
            &BTreeMap::from([
                ("dimension", name.to_owned()),
                ("category", first_category_name.to_owned()),
            ]),
        );
        true
    }

    pub fn add_classification_category(
        &mut self,
        dimension_id: ClassificationDimensionId,
        name: &str,
    ) -> bool {
        let name = name.trim();
        let snapshot = self.document.current();
        let Some(dimension) = snapshot.classification_dimension(dimension_id) else {
            return false;
        };
        if name.is_empty()
            || dimension
                .categories()
                .any(|category| category.name() == name)
        {
            return false;
        }
        let Some(category_id) = snapshot
            .classification_dimensions()
            .flat_map(|dimension| dimension.categories())
            .map(|category| category.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(ClassificationCategoryId)
        else {
            return false;
        };
        let dimension_name = dimension.name().to_owned();
        let mut categories = dimension
            .categories()
            .map(|category| (category.id(), category.name().to_owned()))
            .collect::<Vec<_>>();
        categories.push((category_id, name.to_owned()));
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![
                CanonicalCommand::UpsertClassificationDimension {
                    id: dimension_id,
                    name: dimension_name.clone(),
                    categories,
                },
            ]))
            .is_err()
        {
            return false;
        }
        self.classification.category_name_input.clear();
        self.digest = self.catalog.format(
            "digest-added-dimension-category",
            &BTreeMap::from([("dimension", dimension_name), ("category", name.to_owned())]),
        );
        true
    }

    pub fn assign_selection_to_classification(
        &mut self,
        dimension_id: ClassificationDimensionId,
        category_id: Option<ClassificationCategoryId>,
    ) -> bool {
        let snapshot = self.document.current();
        let Some(dimension) = snapshot.classification_dimension(dimension_id) else {
            return false;
        };
        let category_name = match category_id {
            Some(category_id) => {
                let Some(category) = dimension.category(category_id) else {
                    return false;
                };
                category.name().to_owned()
            }
            None => self.catalog.text("dimensions-unassigned"),
        };
        let dimension_name = dimension.name().to_owned();
        let commands = self
            .selected_occurrence_ids()
            .into_iter()
            .filter(|occurrence_id| {
                snapshot.occurrence_classification(*occurrence_id, dimension_id) != category_id
            })
            .map(
                |occurrence_id| CanonicalCommand::SetOccurrenceClassification {
                    occurrence_id,
                    dimension_id,
                    category_id,
                },
            )
            .collect::<Vec<_>>();
        if commands.is_empty() {
            return false;
        }
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-assigned-dimension-category",
            &BTreeMap::from([
                ("dimension", dimension_name),
                ("category", category_name),
                ("count", count.to_string()),
            ]),
        );
        true
    }

    #[must_use]
    pub fn tag_visibility(&self, id: TagId) -> Option<bool> {
        self.document.current().tag(id).map(|tag| tag.visible())
    }

    pub fn set_tag_visibility(&mut self, id: TagId, visible: bool) -> bool {
        let snapshot = self.document.current();
        let Some(tag) = snapshot.tag(id) else {
            return false;
        };
        if tag.visible() == visible {
            return false;
        }
        let name = tag.name().to_owned();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(vec![
                CanonicalCommand::SetTagVisibility { id, visible },
            ]))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-tag-visibility",
            &BTreeMap::from([
                ("name", name),
                (
                    "visibility",
                    self.catalog.text(if visible {
                        "visibility-shown"
                    } else {
                        "visibility-hidden"
                    }),
                ),
            ]),
        );
        true
    }

    pub(crate) fn all_tag_visibility_plan(&self, visible: bool) -> Option<Vec<CanonicalCommand>> {
        let commands = self
            .document
            .current()
            .tags()
            .filter(|tag| tag.visible() != visible)
            .map(|tag| CanonicalCommand::SetTagVisibility {
                id: tag.id(),
                visible,
            })
            .collect::<Vec<_>>();
        (!commands.is_empty()).then_some(commands)
    }

    pub(crate) fn can_set_all_tag_visibility(&self, visible: bool) -> bool {
        self.all_tag_visibility_plan(visible).is_some()
    }

    pub fn set_all_tag_visibility(&mut self, visible: bool) -> bool {
        let Some(commands) = self.all_tag_visibility_plan(visible) else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-all-tags-visibility",
            &BTreeMap::from([
                ("count", count.to_string()),
                (
                    "visibility",
                    self.catalog.text(if visible {
                        "visibility-shown"
                    } else {
                        "visibility-hidden"
                    }),
                ),
            ]),
        );
        true
    }

    pub(crate) fn invert_tag_visibility_plan(&self) -> Option<Vec<CanonicalCommand>> {
        let commands = self
            .document
            .current()
            .tags()
            .map(|tag| CanonicalCommand::SetTagVisibility {
                id: tag.id(),
                visible: !tag.visible(),
            })
            .collect::<Vec<_>>();
        (!commands.is_empty()).then_some(commands)
    }

    pub(crate) fn can_invert_tag_visibility(&self) -> bool {
        self.invert_tag_visibility_plan().is_some()
    }

    pub fn invert_tag_visibility(&mut self) -> bool {
        let Some(commands) = self.invert_tag_visibility_plan() else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-inverted-tags-visibility",
            &BTreeMap::from([("count", count.to_string())]),
        );
        true
    }

    pub(crate) fn tag_isolation_plan(&self, id: TagId) -> Option<(String, Vec<CanonicalCommand>)> {
        let snapshot = self.document.current();
        let name = snapshot.tag(id)?.name().to_owned();
        let commands = snapshot
            .tags()
            .filter_map(|tag| {
                let visible = tag.id() == id;
                (tag.visible() != visible).then_some(CanonicalCommand::SetTagVisibility {
                    id: tag.id(),
                    visible,
                })
            })
            .collect::<Vec<_>>();
        (!commands.is_empty()).then_some((name, commands))
    }

    pub(crate) fn can_isolate_tag(&self, id: TagId) -> bool {
        self.tag_isolation_plan(id).is_some()
    }

    pub fn isolate_tag(&mut self, id: TagId) -> bool {
        let Some((name, commands)) = self.tag_isolation_plan(id) else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-isolated-tag",
            &BTreeMap::from([("count", count.to_string()), ("name", name)]),
        );
        true
    }

    pub(crate) fn selected_tag_ids(&self) -> BTreeSet<TagId> {
        let snapshot = self.document.current();
        self.selected_occurrence_ids()
            .into_iter()
            .filter_map(|id| snapshot.occurrence(id))
            .flat_map(|occurrence| occurrence.tags().iter().copied())
            .collect()
    }

    pub(crate) fn selected_tag_visibility_plan(
        &self,
        visible: bool,
    ) -> Option<(usize, Vec<CanonicalCommand>)> {
        let selected_tags = self.selected_tag_ids();
        if selected_tags.is_empty() {
            return None;
        }
        let commands = self
            .document
            .current()
            .tags()
            .filter(|tag| selected_tags.contains(&tag.id()) && tag.visible() != visible)
            .map(|tag| CanonicalCommand::SetTagVisibility {
                id: tag.id(),
                visible,
            })
            .collect::<Vec<_>>();
        (!commands.is_empty()).then_some((selected_tags.len(), commands))
    }

    pub(crate) fn can_hide_selected_tags(&self) -> bool {
        self.selected_tag_visibility_plan(false).is_some()
    }

    pub fn hide_selected_tags(&mut self) -> bool {
        let Some((tag_count, commands)) = self.selected_tag_visibility_plan(false) else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-hidden-selected-tags",
            &BTreeMap::from([
                ("count", count.to_string()),
                ("tags", tag_count.to_string()),
            ]),
        );
        true
    }

    pub(crate) fn can_show_selected_tags(&self) -> bool {
        self.selected_tag_visibility_plan(true).is_some()
    }

    pub fn show_selected_tags(&mut self) -> bool {
        let Some((tag_count, commands)) = self.selected_tag_visibility_plan(true) else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-shown-selected-tags",
            &BTreeMap::from([
                ("count", count.to_string()),
                ("tags", tag_count.to_string()),
            ]),
        );
        true
    }

    pub(crate) fn selected_tag_inversion_plan(&self) -> Option<Vec<CanonicalCommand>> {
        let selected_tags = self.selected_tag_ids();
        let commands = self
            .document
            .current()
            .tags()
            .filter(|tag| selected_tags.contains(&tag.id()))
            .map(|tag| CanonicalCommand::SetTagVisibility {
                id: tag.id(),
                visible: !tag.visible(),
            })
            .collect::<Vec<_>>();
        (!commands.is_empty()).then_some(commands)
    }

    pub(crate) fn can_invert_selected_tags(&self) -> bool {
        self.selected_tag_inversion_plan().is_some()
    }

    pub fn invert_selected_tags(&mut self) -> bool {
        let Some(commands) = self.selected_tag_inversion_plan() else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-inverted-selected-tags",
            &BTreeMap::from([("count", count.to_string())]),
        );
        true
    }

    pub(crate) fn matching_tag_selection_plan(&self) -> Option<(usize, BTreeSet<InstancePath>)> {
        let selected_tags = self.selected_tag_ids();
        if selected_tags.is_empty() {
            return None;
        }
        let snapshot = self.document.current();
        let paths: BTreeSet<InstancePath> = self
            .active_scene_query()
            .into_iter()
            .filter(|item| {
                item.instance_path.is_root()
                    && snapshot
                        .occurrence(item.instance_path.root_occurrence())
                        .is_some_and(|occurrence| !occurrence.tags().is_disjoint(&selected_tags))
            })
            .map(|item| item.instance_path)
            .collect();
        (!paths.is_empty()
            && (self.selection.primary.is_some()
                || self.selection.selected_group.is_some()
                || self.selection.occurrences != paths))
            .then_some((selected_tags.len(), paths))
    }

    pub(crate) fn can_select_matching_tags(&self) -> bool {
        self.matching_tag_selection_plan().is_some()
    }

    pub fn select_matching_tags(&mut self) -> bool {
        let Some((tag_count, paths)) = self.matching_tag_selection_plan() else {
            return false;
        };
        let count = paths.len();
        self.selection.clear();
        self.selection.occurrences = paths;
        self.digest = self.catalog.format(
            "digest-selected-matching-tags",
            &BTreeMap::from([
                ("count", count.to_string()),
                ("tags", tag_count.to_string()),
            ]),
        );
        true
    }

    pub(crate) fn selected_tag_isolation_plan(&self) -> Option<(usize, Vec<CanonicalCommand>)> {
        let selected_tags = self.selected_tag_ids();
        if selected_tags.is_empty() {
            return None;
        }
        let commands = self
            .document
            .current()
            .tags()
            .filter_map(|tag| {
                let visible = selected_tags.contains(&tag.id());
                (tag.visible() != visible).then_some(CanonicalCommand::SetTagVisibility {
                    id: tag.id(),
                    visible,
                })
            })
            .collect::<Vec<_>>();
        (!commands.is_empty()).then_some((selected_tags.len(), commands))
    }

    pub(crate) fn can_isolate_selected_tags(&self) -> bool {
        self.selected_tag_isolation_plan().is_some()
    }

    pub fn isolate_selected_tags(&mut self) -> bool {
        let Some((tag_count, commands)) = self.selected_tag_isolation_plan() else {
            return false;
        };
        let count = commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.digest = self.catalog.format(
            "digest-isolated-selected-tags",
            &BTreeMap::from([
                ("count", count.to_string()),
                ("tags", tag_count.to_string()),
            ]),
        );
        true
    }

    pub(crate) fn tag_assignment_source_plan(&self) -> Option<TagAssignmentSourcePlan> {
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_count = occurrence_paths.len();
        let occurrence_ids = self.selected_occurrence_ids();
        (occurrence_count > 0 && occurrence_count == occurrence_ids.len()).then_some(())?;
        let snapshot = self.document.current();
        let source_tags = occurrence_ids
            .iter()
            .copied()
            .map(|id| Some((id, snapshot.occurrence(id)?.tags().clone())))
            .collect::<Option<BTreeMap<_, _>>>()?;
        let available_tags = self.tag_options().into_iter().collect::<BTreeMap<_, _>>();
        (!available_tags.is_empty()).then_some(())?;
        // The dialog starts from the tags every selected part already has.
        let initial_tags = source_tags.values().skip(1).fold(
            source_tags.values().next().cloned().unwrap_or_default(),
            |common, tags| common.intersection(tags).copied().collect(),
        );
        Some(TagAssignmentSourcePlan {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            occurrence_paths,
            occurrence_count,
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            source_tags,
            available_tags,
            initial_tags,
        })
    }

    pub(crate) fn dialog_tag_assignment_plan(
        &self,
        pending: &PendingTagAssignment,
    ) -> Option<TagAssignmentPlan> {
        let source = self.tag_assignment_source_plan()?;
        (source == pending.source && source.occurrence_count == source.occurrence_paths.len())
            .then_some(())?;
        let target_tag_name = if pending.target_tags.is_empty() {
            self.catalog.text("dialog-assign-tag-untagged")
        } else {
            pending
                .target_tags
                .iter()
                .map(|id| source.available_tags.get(id).cloned())
                .collect::<Option<Vec<_>>>()?
                .join(", ")
        };
        let changed_occurrence_ids = source
            .source_tags
            .iter()
            .filter_map(|(id, tags)| (*tags != pending.target_tags).then_some(*id))
            .collect::<BTreeSet<_>>();
        let commands = changed_occurrence_ids
            .iter()
            .copied()
            .map(|id| CanonicalCommand::SetOccurrenceTags {
                id,
                tags: pending.target_tags.clone(),
            })
            .collect::<Vec<_>>();
        (!commands.is_empty()).then_some(TagAssignmentPlan {
            source,
            target_tags: pending.target_tags.clone(),
            target_tag_name,
            changed_occurrence_ids,
            commands,
        })
    }

    pub(crate) fn begin_tag_assignment(&mut self) {
        let Some(source) = self.tag_assignment_source_plan() else {
            return;
        };
        let target_tags = source.initial_tags.clone();
        self.modal.open(PendingTagAssignment {
            source,
            target_tags,
        });
    }

    #[must_use]
    pub fn tag_assignment_visible(&self) -> bool {
        self.modal.get::<PendingTagAssignment>().is_some()
    }

    #[must_use]
    pub fn tag_assignment_input(&self) -> Option<BTreeSet<TagId>> {
        self.modal
            .get::<PendingTagAssignment>()
            .map(|pending| pending.target_tags.clone())
    }

    #[must_use]
    pub fn occurrence_tags(&self, occurrence_id: OccurrenceId) -> BTreeSet<TagId> {
        self.document
            .current()
            .occurrence(occurrence_id)
            .map(|occurrence| occurrence.tags().clone())
            .unwrap_or_default()
    }

    pub(crate) fn apply_tag_assignment_plan(&mut self, plan: TagAssignmentPlan) -> bool {
        let pending = PendingTagAssignment {
            source: plan.source.clone(),
            target_tags: plan.target_tags.clone(),
        };
        if plan.changed_occurrence_ids.len() != plan.commands.len()
            || self.dialog_tag_assignment_plan(&pending).as_ref() != Some(&plan)
        {
            return false;
        }
        let count = plan.commands.len();
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(plan.commands))
            .is_err()
        {
            return false;
        }
        self.modal.close::<PendingTagAssignment>();
        self.digest = self.catalog.format(
            "digest-assigned-tag",
            &BTreeMap::from([("count", count.to_string()), ("tag", plan.target_tag_name)]),
        );
        true
    }

    pub fn confirm_tag_assignment(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingTagAssignment>().cloned() else {
            return false;
        };
        let Some(plan) = self.dialog_tag_assignment_plan(&pending) else {
            return false;
        };
        self.apply_tag_assignment_plan(plan)
    }

    #[must_use]
    pub const fn tags_visible(&self) -> bool {
        self.panels.tags_visible
    }

    pub(crate) fn show_classification_dimensions(&mut self, ui: &mut egui::Ui) {
        if !self.panels.dimensions_visible {
            return;
        }
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("dock-dimensions"));
        ui.small(self.catalog.text("dimensions-help"));

        let dimension_label = self.catalog.text("dimensions-name");
        ui.label(&dimension_label);
        ui.add(
            egui::TextEdit::singleline(&mut self.classification.dimension_name_input)
                .hint_text(self.catalog.text("dimensions-name-hint")),
        )
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &dimension_label)
        });
        let first_category_label = self.catalog.text("dimensions-first-category");
        ui.label(&first_category_label);
        ui.add(
            egui::TextEdit::singleline(&mut self.classification.category_name_input)
                .hint_text(self.catalog.text("dimensions-category-hint")),
        )
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &first_category_label)
        });
        let create_clicked = ui
            .add_enabled(
                !self.classification.dimension_name_input.trim().is_empty()
                    && !self.classification.category_name_input.trim().is_empty(),
                egui::Button::new(self.catalog.text("dimensions-create")),
            )
            .clicked();
        if create_clicked {
            let name = self.classification.dimension_name_input.clone();
            let category = self.classification.category_name_input.clone();
            self.create_classification_dimension(&name, &category);
        }

        let rows = self.classification_rows();
        if rows.is_empty() {
            ui.weak(self.catalog.text("dimensions-empty"));
            return;
        }
        if !rows
            .iter()
            .any(|row| Some(row.id) == self.classification.selected_dimension)
        {
            self.classification.selected_dimension = Some(rows[0].id);
        }
        let mut selected = self
            .classification
            .selected_dimension
            .expect("a classification dimension is available");
        let selected_name = rows
            .iter()
            .find(|row| row.id == selected)
            .map(|row| row.name.as_str())
            .expect("the selected classification dimension is available");
        let selector_label = self.catalog.text("dimensions-active");
        ui.label(&selector_label);
        egui::ComboBox::from_id_salt("classification-dimension")
            .width(ui.available_width())
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                for row in &rows {
                    ui.selectable_value(&mut selected, row.id, &row.name);
                }
            })
            .response
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &selector_label)
            });
        self.classification.selected_dimension = Some(selected);

        let category_label = self.catalog.text("dimensions-new-category");
        ui.label(&category_label);
        let mut add_category_clicked = false;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.classification.category_name_input)
                    .hint_text(self.catalog.text("dimensions-category-hint")),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &category_label)
            });
            add_category_clicked = ui
                .add_enabled(
                    !self.classification.category_name_input.trim().is_empty(),
                    egui::Button::new(self.catalog.text("dimensions-add-category")),
                )
                .clicked();
        });
        if add_category_clicked {
            let category = self.classification.category_name_input.clone();
            self.add_classification_category(selected, &category);
        }

        let selected_occurrences = self.selected_occurrence_ids();
        let snapshot = self.document.current();
        let clear_enabled = selected_occurrences.iter().any(|occurrence_id| {
            snapshot
                .occurrence_classification(*occurrence_id, selected)
                .is_some()
        });
        let mut assignment = None;
        if let Some(row) = rows.iter().find(|row| row.id == selected) {
            for category in &row.categories {
                ui.horizontal_wrapped(|ui| {
                    ui.label(self.catalog.format(
                        "dimensions-category-row",
                        &BTreeMap::from([
                            ("category", category.name.clone()),
                            ("count", category.occurrence_count.to_string()),
                        ]),
                    ));
                    if ui
                        .add_enabled(
                            !selected_occurrences.is_empty(),
                            egui::Button::new(self.catalog.text("dimensions-assign-selection")),
                        )
                        .clicked()
                    {
                        assignment = Some(Some(category.id));
                    }
                });
            }
        }
        if ui
            .add_enabled(
                clear_enabled,
                egui::Button::new(self.catalog.text("dimensions-clear-selection")),
            )
            .clicked()
        {
            assignment = Some(None);
        }
        if let Some(category_id) = assignment {
            self.assign_selection_to_classification(selected, category_id);
        }
    }

    pub(crate) fn show_tag_creation_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingTagCreation>() else {
            return;
        };
        if self
            .tag_creation_source_plan(pending.source.occurrence_ids.as_ref())
            .as_ref()
            != Some(&pending.source)
        {
            self.modal.close::<PendingTagCreation>();
            return;
        }
        let mut name = pending.name.clone();
        let mut open = true;
        let mut create = false;
        let mut cancel = false;
        let title_key = if pending.source.occurrence_ids.is_some() {
            "dialog-create-tag-from-selection-title"
        } else {
            "dialog-create-tag-title"
        };
        egui::Window::new(self.catalog.text(title_key))
            .id(egui::Id::new("create-tag"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                let input_label = self.catalog.text("dialog-create-tag-name");
                ui.label(&input_label);
                let input = ui.add(egui::TextEdit::singleline(&mut name));
                input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &input_label)
                });
                let can_create = self.tag_creation_plan(&pending.source, &name).is_some();
                ui.separator();
                ui.horizontal(|ui| {
                    let confirm_key = if pending.source.occurrence_ids.is_some() {
                        "dialog-create-tag-from-selection-confirm"
                    } else {
                        "dialog-create-tag-confirm"
                    };
                    create = ui
                        .add_enabled(
                            can_create,
                            egui::Button::new(self.catalog.text(confirm_key)),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-create-tag-cancel"))
                        .clicked();
                });
            });
        if let Some(pending) = self.modal.get_mut::<PendingTagCreation>() {
            pending.name = name;
        }
        if cancel || !open {
            self.modal.close::<PendingTagCreation>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if create {
            self.confirm_tag_creation();
        }
    }

    pub(crate) fn show_tag_deletion_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingTagDeletion>() else {
            return;
        };
        if self.tag_deletion_source_plan(pending.source.id).as_ref() != Some(&pending.source) {
            self.modal.close::<PendingTagDeletion>();
            return;
        }
        let mut open = true;
        let mut delete = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-delete-tag-title"))
            .id(egui::Id::new("delete-tag"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    if pending.source.occurrence_ids.is_empty() {
                        "dialog-delete-tag-message"
                    } else {
                        "dialog-delete-used-tag-message"
                    },
                    &BTreeMap::from([
                        ("name", pending.source.original_name.clone()),
                        ("count", pending.source.occurrence_ids.len().to_string()),
                    ]),
                ));
                ui.separator();
                ui.horizontal(|ui| {
                    delete = ui
                        .button(self.catalog.text("dialog-delete-tag-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-delete-tag-cancel"))
                        .clicked();
                });
            });
        if cancel || !open {
            self.modal.close::<PendingTagDeletion>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if delete {
            self.confirm_tag_deletion();
        }
    }

    pub(crate) fn show_tag_clear_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingTagClear>() else {
            return;
        };
        if self.tag_clear_source_plan(pending.source.id).as_ref() != Some(&pending.source) {
            self.modal.close::<PendingTagClear>();
            return;
        }
        let count = pending.source.occurrence_ids.len();
        let mut open = true;
        let mut clear = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-clear-tag-title"))
            .id(egui::Id::new("clear-tag"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-clear-tag-message",
                    &BTreeMap::from([
                        ("name", pending.source.original_name.clone()),
                        ("count", count.to_string()),
                    ]),
                ));
                ui.separator();
                ui.horizontal(|ui| {
                    clear = ui
                        .button(self.catalog.text("dialog-clear-tag-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-clear-tag-cancel"))
                        .clicked();
                });
            });
        if cancel || !open {
            self.modal.close::<PendingTagClear>();
        } else if clear {
            self.confirm_tag_clear();
        }
    }

    pub(crate) fn show_tag_rename_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingTagRename>() else {
            return;
        };
        if self.tag_rename_source_plan(pending.source.id).as_ref() != Some(&pending.source) {
            self.modal.close::<PendingTagRename>();
            return;
        }
        let mut name = pending.name.clone();
        let mut open = true;
        let mut rename = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-rename-tag-title"))
            .id(egui::Id::new("rename-tag"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                let input_label = self.catalog.text("dialog-rename-tag-name");
                ui.label(&input_label);
                let input = ui.add(egui::TextEdit::singleline(&mut name));
                input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &input_label)
                });
                let can_rename = self.tag_rename_plan(&pending.source, &name).is_some();
                ui.separator();
                ui.horizontal(|ui| {
                    rename = ui
                        .add_enabled(
                            can_rename,
                            egui::Button::new(self.catalog.text("dialog-rename-tag-confirm")),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-rename-tag-cancel"))
                        .clicked();
                });
            });
        if let Some(pending) = self.modal.get_mut::<PendingTagRename>() {
            pending.name = name;
        }
        if cancel || !open {
            self.modal.close::<PendingTagRename>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if rename {
            self.confirm_tag_rename();
        }
    }

    pub(crate) fn show_tag_assignment_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingTagAssignment>() else {
            return;
        };
        if self.tag_assignment_source_plan().as_ref() != Some(&pending.source) {
            self.modal.close::<PendingTagAssignment>();
            return;
        }
        let options = pending
            .source
            .available_tags
            .iter()
            .map(|(id, name)| (*id, name.clone()))
            .collect::<Vec<_>>();
        let mut tags = pending.target_tags.clone();
        let mut open = true;
        let mut assign = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-assign-tag-title"))
            .id(egui::Id::new("assign-tag"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.text("dialog-assign-tag-tag"));
                if ui
                    .selectable_label(
                        tags.is_empty(),
                        self.catalog.text("dialog-assign-tag-untagged"),
                    )
                    .clicked()
                {
                    tags.clear();
                }
                for (candidate, name) in &options {
                    let mut member = tags.contains(candidate);
                    if ui.checkbox(&mut member, name).changed() {
                        if member {
                            tags.insert(*candidate);
                        } else {
                            tags.remove(candidate);
                        }
                    }
                }
                let has_changes = self
                    .dialog_tag_assignment_plan(&PendingTagAssignment {
                        source: pending.source.clone(),
                        target_tags: tags.clone(),
                    })
                    .is_some();
                ui.separator();
                ui.horizontal(|ui| {
                    assign = ui
                        .add_enabled(
                            has_changes,
                            egui::Button::new(self.catalog.text("dialog-assign-tag-confirm")),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-assign-tag-cancel"))
                        .clicked();
                });
            });
        if let Some(pending) = self.modal.get_mut::<PendingTagAssignment>() {
            pending.target_tags = tags;
        }
        if cancel || !open {
            self.modal.close::<PendingTagAssignment>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if assign {
            self.confirm_tag_assignment();
        }
    }
}
