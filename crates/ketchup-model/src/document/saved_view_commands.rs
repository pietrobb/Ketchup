//! Applying saved-view commands to the product model.

use super::*;

/// Stores or removes a saved view; `command` must be a saved-view command.
pub(super) fn apply_saved_view_command(
    product: &mut ProductModel,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::UpsertSavedView(view) => {
            validate_saved_view(product, view)?;
            product.saved_views.insert(view.id, Arc::new(view.clone()));
        }
        CanonicalCommand::DeleteSavedView { id } => {
            product
                .saved_views
                .remove(id)
                .ok_or(CanonicalError::SavedViewNotFound(*id))?;
        }
        _ => {}
    }
    Ok(())
}

/// Checks one saved view against the product it is (or will be) stored in.
pub(super) fn validate_saved_view(
    product: &ProductModel,
    view: &SavedView,
) -> Result<(), CanonicalError> {
    ensure_product_id(view.id.0)?;
    ensure_name(&view.name)?;
    let name_holder = product
        .saved_views
        .values()
        .find(|other| other.id != view.id && other.name == view.name);
    let problem = name_holder
        .map(|other| SavedViewProblem::NameTaken(other.id))
        .or_else(|| view.shape_problem())
        .or_else(|| {
            view.hidden_tags
                .iter()
                .find(|tag| !product.tags.contains_key(tag))
                .map(|tag| SavedViewProblem::MissingTag(*tag))
        });
    match problem {
        Some(problem) => Err(CanonicalError::InvalidSavedView {
            id: view.id,
            problem,
        }),
        None => Ok(()),
    }
}

/// A tag may be deleted only when no part carries it; the error names who does.
pub(super) fn ensure_tag_unassigned(
    product: &ProductModel,
    tag: TagId,
) -> Result<(), CanonicalError> {
    let holders = product
        .occurrences
        .iter()
        .filter(|(_, occurrence)| occurrence.tags.contains(&tag))
        .map(|(id, _)| TagHolder::Occurrence(*id))
        .chain(
            product
                .local_occurrences
                .iter()
                .filter(|(_, occurrence)| occurrence.tags.contains(&tag))
                .map(|(key, _)| TagHolder::LocalOccurrence(*key)),
        )
        .collect::<Vec<_>>();
    match holders.first() {
        Some(holder) => Err(CanonicalError::TagInUse {
            tag,
            holder: *holder,
            holders: holders.len(),
        }),
        None => Ok(()),
    }
}

fn ensure_tags_exist(product: &ProductModel, tags: &BTreeSet<TagId>) -> Result<(), CanonicalError> {
    match tags.iter().find(|tag| !product.tags.contains_key(tag)) {
        Some(missing) => Err(CanonicalError::TagNotFound(*missing)),
        None => Ok(()),
    }
}

pub(super) fn set_occurrence_tags(
    product: &mut ProductModel,
    id: OccurrenceId,
    tags: &BTreeSet<TagId>,
) -> Result<(), CanonicalError> {
    ensure_tags_exist(product, tags)?;
    let existing = product
        .occurrences
        .get_mut(&id)
        .ok_or(CanonicalError::OccurrenceNotFound(id))?;
    Arc::make_mut(existing).tags = tags.clone();
    Ok(())
}

/// The layers of a part inside a definition; without this a layer a program
/// gave a nested part could never be taken off it, nor deleted.
pub(super) fn set_local_occurrence_tags(
    product: &mut ProductModel,
    key: LocalOccurrenceKey,
    tags: &BTreeSet<TagId>,
) -> Result<(), CanonicalError> {
    ensure_tags_exist(product, tags)?;
    group_conversion::local_occurrence_mut(product, key)?.tags = tags.clone();
    Ok(())
}

/// A deleted tag no longer exists, so no saved view can keep hiding it.
pub(super) fn forget_tag_in_saved_views(product: &mut ProductModel, id: TagId) {
    for view in product.saved_views.values_mut() {
        if view.hidden_tags.contains(&id) {
            Arc::make_mut(view).hidden_tags.remove(&id);
        }
    }
}
