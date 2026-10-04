//! Applying saved-view commands to the product model.

use super::*;

/// Stores or removes a saved view; `command` must be a saved-view command.
pub(super) fn apply_saved_view_command(
    product: &mut ProductModel,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::UpsertSavedView(view) => {
            ensure_product_id(view.id.0)?;
            ensure_name(&view.name)?;
            let name_taken = product
                .saved_views
                .values()
                .any(|other| other.id != view.id && other.name == view.name);
            if name_taken
                || !view.is_well_formed()
                || !view
                    .hidden_tags
                    .iter()
                    .all(|id| product.tags.contains_key(id))
            {
                return Err(CanonicalError::InvalidSavedView(view.id));
            }
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

/// A deleted tag no longer exists, so no saved view can keep hiding it.
pub(super) fn forget_tag_in_saved_views(product: &mut ProductModel, id: TagId) {
    for view in product.saved_views.values_mut() {
        if view.hidden_tags.contains(&id) {
            Arc::make_mut(view).hidden_tags.remove(&id);
        }
    }
}
