use std::collections::{BTreeMap, BTreeSet};

use ketchup_application::transforms::world_edit_in_parent_space;
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentId, GroupId, InstancePath, OccurrenceId,
    Snapshot, Transform,
};
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::placement::plan_underlay_edit;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TransformTarget {
    Occurrences(BTreeSet<InstancePath>),
    Group(GroupId),
    /// The underlay's placement takes the edit, as a fit does ([`plan_underlay_edit`]).
    Underlay(UnderlayId),
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
    world_edit: Transform,
    preview_overrides: BTreeMap<InstancePath, Transform>,
    batch: CommandBatch,
}

impl TransformRequest {
    pub(crate) fn matches_source(&self, snapshot: &Snapshot) -> bool {
        if let TransformTarget::Underlay(id) = self.target {
            return self.source_document_id == snapshot.document_id()
                && self.source_revision == snapshot.revision_id()
                && snapshot.underlay(id).is_some();
        }
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
                TransformTarget::Underlay(_) => false,
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
                let scene = snapshot.scene_query();
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
                    let scene_occurrence = scene
                        .iter()
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
            TransformTarget::Underlay(id) => {
                // A refused edit has no plan and no preview; the commit names why
                // (`KetchupApp::underlay_edit_refused`).
                let batch = plan_underlay_edit(snapshot, *id, request.world_edit).ok()?;
                (batch.commands().to_vec(), BTreeMap::new())
            }
        };

        (!commands.is_empty()).then(|| Self {
            target: request.target,
            world_edit: request.world_edit,
            preview_overrides,
            batch: CommandBatch::new(commands),
        })
    }

    /// The plan of the move or turn `world_edit` of the underlay `id` as `snapshot` has it.
    pub(crate) fn underlay(
        snapshot: &Snapshot,
        id: UnderlayId,
        world_edit: Transform,
    ) -> Option<Self> {
        let batch = plan_underlay_edit(snapshot, id, world_edit).ok()?;
        Some(Self {
            target: TransformTarget::Underlay(id),
            world_edit,
            preview_overrides: BTreeMap::new(),
            batch,
        })
    }

    pub(crate) const fn batch(&self) -> &CommandBatch {
        &self.batch
    }

    pub(crate) fn preview_overrides(&self) -> &BTreeMap<InstancePath, Transform> {
        &self.preview_overrides
    }

    pub(crate) const fn target(&self) -> &TransformTarget {
        &self.target
    }

    pub(crate) fn into_commit(self) -> (TransformTarget, CommandBatch) {
        (self.target, self.batch)
    }

    /// The rigid world motion the plan applies to its target.
    pub(crate) const fn world_edit(&self) -> Transform {
        self.world_edit
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
