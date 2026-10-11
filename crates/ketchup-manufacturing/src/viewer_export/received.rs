//! Comments a customer returned on an exported package, matched to the
//! author's model by its document identity and full occurrence paths only,
//! never by names or similar geometry. Comments are never CAD commands.

use super::view;
use ketchup_model::document::{
    InstancePath, InstancePathStep, LocalGroupId, LocalGroupKey, LocalOccurrenceId,
    LocalOccurrenceKey, OccurrenceId, Snapshot,
};
use ketchup_rejection::{Rejection, RejectionPhase};

/// How the author's model relates to the export the comments were written on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportState {
    /// The model is unchanged since the export.
    Current,
    /// The model was edited since; components may have moved or gone.
    Changed,
}

/// Whether `comments` belong to `snapshot`'s project, and whether the model
/// changed since their export. Comments on another project are refused.
pub fn export_state(
    snapshot: &Snapshot,
    comments: &view::Comments,
) -> Result<ExportState, Rejection> {
    let source = &comments.export.source;
    if source.document_id != snapshot.document_id().0 {
        return Err(
            Rejection::new("viewer.comments.project", RejectionPhase::Request)
                .reason("These comments were written on another project.")
                .fix_hint("Open the project the package was exported from, then its comments."),
        );
    }
    Ok(if source.canonical_digest == snapshot.canonical_digest() {
        ExportState::Current
    } else {
        ExportState::Changed
    })
}

/// The model path of an exported component, or `None` when any step is gone
/// or now belongs to another definition.
pub fn model_path(snapshot: &Snapshot, path: &view::InstancePath) -> Option<InstancePath> {
    let root = snapshot.occurrence(OccurrenceId(path.root_occurrence_id))?;
    let mut definition_id = root.definition_id();
    let mut model = InstancePath::root(root.id());
    for step in &path.steps {
        match *step {
            view::PathStep::Group {
                owner_definition_id,
                local_id,
            } => {
                let local_id = LocalGroupId(local_id);
                (owner_definition_id == definition_id.0).then_some(())?;
                snapshot.local_group(LocalGroupKey {
                    definition_id,
                    local_id,
                })?;
                model = model.with_step(InstancePathStep::Group(local_id));
            }
            view::PathStep::Occurrence {
                owner_definition_id,
                local_id,
            } => {
                let local_id = LocalOccurrenceId(local_id);
                (owner_definition_id == definition_id.0).then_some(())?;
                definition_id = snapshot
                    .local_occurrence(LocalOccurrenceKey {
                        definition_id,
                        local_id,
                    })?
                    .definition_id();
                model = model.with_step(InstancePathStep::Occurrence(local_id));
            }
        }
    }
    snapshot.resolve_instance_path(&model).ok()?;
    Some(model)
}

/// An arrow end in the author's world, following its component where it is
/// now; `None` when the component is gone.
pub fn anchor_point(snapshot: &Snapshot, anchor: &view::Anchor) -> Option<[f64; 3]> {
    let Some(path) = &anchor.occurrence else {
        return Some(anchor.point_mm);
    };
    let resolved = snapshot
        .resolve_instance_path(&model_path(snapshot, path)?)
        .ok()?;
    Some(resolved.world_transform.transform_point(anchor.point_mm))
}

/// Components a comment refers to that are no longer in the model.
pub fn missing_parts<'a>(
    snapshot: &Snapshot,
    comment: &'a view::Comment,
) -> Vec<&'a view::InstancePath> {
    let mut missing: Vec<_> = comment
        .selected
        .iter()
        .chain(
            comment
                .arrows
                .iter()
                .flat_map(|arrow| [&arrow.tip, &arrow.tail])
                .filter_map(|anchor| anchor.occurrence.as_ref()),
        )
        .filter(|path| model_path(snapshot, path).is_none())
        .collect();
    missing.sort();
    missing.dedup();
    missing
}
