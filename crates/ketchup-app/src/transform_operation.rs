use std::collections::{BTreeMap, BTreeSet};

use ketchup_application::transforms::world_edit_in_parent_space;
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentId, GroupId, InstancePath, OccurrenceId,
    Snapshot, Transform,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TransformTarget {
    Occurrences(BTreeSet<InstancePath>),
    Group(GroupId),
}

#[derive(Clone)]
pub(crate) struct TransformRequest {
    pub source_document_id: DocumentId,
    pub source_revision: u64,
    pub primary_occurrence: OccurrenceId,
    pub primary_definition: DefinitionId,
    pub target: TransformTarget,
    pub world_edit: Transform,
}

pub(crate) struct TransformPlan {
    target: TransformTarget,
    preview_overrides: BTreeMap<InstancePath, Transform>,
    batch: CommandBatch,
}

impl TransformRequest {
    pub(crate) fn matches_source(&self, snapshot: &Snapshot) -> bool {
        self.source_document_id == snapshot.document_id()
            && self.source_revision == snapshot.revision_id()
            && snapshot
                .occurrence(self.primary_occurrence)
                .is_some_and(|occurrence| occurrence.definition_id() == self.primary_definition)
            && match &self.target {
                TransformTarget::Occurrences(paths) => {
                    let primary_path = InstancePath::root(self.primary_occurrence);
                    !paths.is_empty()
                        && paths.contains(&primary_path)
                        && paths.iter().all(|path| {
                            path.is_root() && snapshot.occurrence(path.root_occurrence()).is_some()
                        })
                }
                TransformTarget::Group(group_id) => snapshot.group(*group_id).is_some(),
            }
    }
}

impl TransformPlan {
    pub(crate) fn prepare(snapshot: &Snapshot, request: TransformRequest) -> Option<Self> {
        if !request.matches_source(snapshot) {
            return None;
        }

        let (commands, preview_overrides) = match &request.target {
            TransformTarget::Occurrences(paths) => {
                let mut commands = Vec::with_capacity(paths.len());
                let mut preview_overrides = BTreeMap::new();
                for path in paths {
                    let occurrence = snapshot.occurrence(path.root_occurrence())?;
                    let transform = world_edit_in_parent_space(
                        snapshot,
                        occurrence.parent(),
                        occurrence.transform(),
                        request.world_edit,
                    )?;
                    commands.push(CanonicalCommand::SetOccurrenceTransform {
                        id: occurrence.id(),
                        transform,
                    });
                    let scene_occurrence = snapshot
                        .scene_query()
                        .into_iter()
                        .find(|candidate| candidate.instance_path == *path)?;
                    preview_overrides.insert(
                        path.clone(),
                        request.world_edit.compose(scene_occurrence.transform),
                    );
                }
                (commands, preview_overrides)
            }
            TransformTarget::Group(group_id) => {
                let group = snapshot.group(*group_id)?;
                let transform = world_edit_in_parent_space(
                    snapshot,
                    group.parent(),
                    group.transform(),
                    request.world_edit,
                )?;
                let preview_overrides = snapshot
                    .scene_query()
                    .into_iter()
                    .filter(|occurrence| {
                        group_contains_occurrence(
                            snapshot,
                            *group_id,
                            occurrence.instance_path.root_occurrence(),
                        )
                    })
                    .map(|occurrence| {
                        (
                            occurrence.instance_path,
                            request.world_edit.compose(occurrence.transform),
                        )
                    })
                    .collect();
                (
                    vec![CanonicalCommand::SetGroupTransform {
                        id: *group_id,
                        transform,
                    }],
                    preview_overrides,
                )
            }
        };

        (!commands.is_empty()).then(|| Self {
            target: request.target,
            preview_overrides,
            batch: CommandBatch::new(commands),
        })
    }

    pub(crate) fn preview_overrides(&self) -> &BTreeMap<InstancePath, Transform> {
        &self.preview_overrides
    }

    pub(crate) fn into_commit(self) -> (TransformTarget, CommandBatch) {
        (self.target, self.batch)
    }
}

fn group_contains_occurrence(
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
