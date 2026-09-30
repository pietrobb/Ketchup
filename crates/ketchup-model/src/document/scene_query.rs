use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneQueryContext {
    Group(GroupId),
    Definition {
        definition_id: DefinitionId,
        instance_path: InstancePath,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundSceneQuery {
    pub(super) document_id: DocumentId,
    pub(super) source_revision: u64,
    pub(super) source_digest: String,
    pub(super) context: SceneQueryContext,
}

impl BoundSceneQuery {
    #[must_use]
    pub const fn document_id(&self) -> DocumentId {
        self.document_id
    }

    #[must_use]
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    #[must_use]
    pub fn source_digest(&self) -> &str {
        &self.source_digest
    }

    #[must_use]
    pub const fn context(&self) -> &SceneQueryContext {
        &self.context
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneQueryError {
    InvalidContext,
    SnapshotMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneQueryBudgetKind {
    Occurrences,
    PathSteps,
    TextBytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneQueryBudgetExceeded {
    pub kind: SceneQueryBudgetKind,
    pub limit: usize,
    pub observed_at_least: usize,
}

impl fmt::Display for SceneQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidContext => "scene query context is invalid or hidden",
            Self::SnapshotMismatch => "scene query is bound to a different snapshot",
        })
    }
}

impl std::error::Error for SceneQueryError {}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneOccurrence {
    pub occurrence_id: OccurrenceId,
    pub instance_path: InstancePath,
    pub definition_id: DefinitionId,
    pub occurrence_name: String,
    pub definition_name: String,
    pub transform: Transform,
    pub parent: Option<GroupId>,
    pub local_parent: Option<LocalGroupId>,
    pub visible: bool,
    pub shared_occurrence_count: usize,
    pub color: Option<[u8; 3]>,
}

impl SceneOccurrence {
    /// Whether this projection still describes `snapshot`.
    #[must_use]
    pub fn matches_snapshot(&self, snapshot: &Snapshot) -> bool {
        let Ok(resolved) = snapshot.resolve_instance_path(&self.instance_path) else {
            return false;
        };
        let root_id = self.instance_path.root_occurrence();
        let Some(root) = snapshot.occurrence(root_id) else {
            return false;
        };
        let mut definition_id = root.definition_id;
        let mut name = root.name.as_str();
        let mut parent = root.parent;
        let mut local_parent = None;
        let mut visible = snapshot.occurrence_effectively_visible(root_id) == Some(true);
        let mut color = root.color;
        for step in self.instance_path.steps() {
            if let InstancePathStep::Occurrence(local_id) = step {
                let Some(local) = snapshot.local_occurrence(LocalOccurrenceKey {
                    definition_id,
                    local_id: *local_id,
                }) else {
                    return false;
                };
                visible &= local.visible
                    && local
                        .tag
                        .and_then(|id| snapshot.tag(id))
                        .is_none_or(|tag| tag.visible);
                color = color.or(local.color);
                definition_id = local.definition_id;
                name = &local.name;
                parent = None;
                local_parent = local.parent;
            }
        }
        !matches!(
            self.instance_path.steps().last(),
            Some(InstancePathStep::Group(_))
        ) && self.occurrence_id == root_id
            && self.definition_id == resolved.definition_id
            && self.transform == resolved.world_transform
            && self.occurrence_name == name
            && snapshot
                .definition(definition_id)
                .is_some_and(|definition| self.definition_name == definition.name)
            && self.parent == parent
            && self.local_parent == local_parent
            && self.visible == visible
            && self.color == color
    }

    /// Resolved inherited sRGB color for this projected occurrence.
    #[must_use]
    pub const fn color(&self) -> Option<[u8; 3]> {
        self.color
    }
}
