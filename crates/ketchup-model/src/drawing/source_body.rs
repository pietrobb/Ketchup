//! The one current body a drawing draws for a definition: with modelled
//! threads the body with them cosmetic, which drawings show by ISO 6410.
use super::DrawingError;
use crate::document::{DefinitionId, Snapshot};
use crate::exact_product::{ExactBodyPackage, ExactResultRegistry};
use std::sync::Arc;

pub(super) fn unique_current_package(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    definition_id: DefinitionId,
) -> Result<Arc<ExactBodyPackage>, DrawingError> {
    let mut current = results
        .render_values(snapshot)
        .filter(|package| package.definition_id() == definition_id);
    let package = current.next();
    if current.next().is_some() {
        return Err(DrawingError::SourceAmbiguous);
    }
    if let Some(package) = package {
        return results
            .drawing_body(package)
            .ok_or(DrawingError::CosmeticThreadsPending);
    }
    if results
        .values()
        .any(|package| package.definition_id() == definition_id)
    {
        Err(DrawingError::SourceStale)
    } else {
        Err(DrawingError::SourceFailed)
    }
}
