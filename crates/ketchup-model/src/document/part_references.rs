//! Register of the product collections a batch prunes when a part they name goes
//! away (MD-2).
//!
//! Deleting an occurrence, repointing it or reshaping the local structure a nested
//! instance path goes through makes some entries stop resolving. Collections that
//! refuse such a batch (collections, mates, assembly and pin joints) fail loudly;
//! the ones listed here drop those entries silently. A reviewed proposal therefore
//! depends on every prunable entry of the parts it touches, so an entry the user
//! adds after the review makes the proposal stale instead of being dropped.
use super::*;
use serde::Serialize;

/// Every field of [`ProductModel`] (and its flattened support declarations) that a
/// batch prunes when a part it names stops resolving. The test below fails when a
/// product field is added without being classified here or in [`KEPT`].
#[cfg(test)]
const PRUNED: &[&str] = &[
    "grounded_occurrences",
    "classification_assignments",
    "instance_transform_overrides",
    "grounded_instances",
    "contact_joints",
];

/// Product fields a batch never prunes for a vanished part: they either hold no
/// part reference, are the parts themselves, or refuse or fail validation.
#[cfg(test)]
const KEPT: &[&str] = &[
    "document_id",
    "units",
    "tolerance",
    "support",
    "floor_z_mm",
    "evaluator_nodes",
    "overrides",
    "feature_parameter_bindings",
    "feature_parameter_provenance",
    "joints",
    "spaces",
    "clearance_volumes",
    "cam_plans",
    "pin_joints",
    "assembly_recipe",
    "exact_reference_evidence",
    "persistent_dimensions",
    "tags",
    "saved_views",
    "classification_dimensions",
    "collections",
    "import_receipts",
    "definitions",
    "features",
    "body_feature_suppression",
    "occurrences",
    "assembly_mates",
    "assembly_joints",
    "assembly_motion_couplings",
    "assembly_motion_studies",
    "mechanical_interfaces",
    "mechanical_conditions",
    "drawing_sheets",
    "groups",
    "local_occurrences",
    "local_groups",
    "production_codes",
    "canonical_digest",
    "exact_graphs",
];

/// The prunable entries that name `id` or a part below it.
pub(super) fn prunable_references_to(
    product: &ProductModel,
    id: OccurrenceId,
) -> impl Serialize + '_ {
    prunable_references(product, move |path| path.root_occurrence() == id, Some(id))
}

/// The prunable entries whose nested path goes through the local structure of
/// `definition_id`.
pub(super) fn prunable_references_through(
    product: &ProductModel,
    definition_id: DefinitionId,
) -> impl Serialize + '_ {
    prunable_references(
        product,
        move |path| path_goes_through(product, path, definition_id),
        None,
    )
}

fn prunable_references<'a>(
    product: &'a ProductModel,
    names: impl Fn(&InstancePath) -> bool,
    root: Option<OccurrenceId>,
) -> impl Serialize + 'a {
    (
        root.map(|id| product.grounded_occurrences.contains(&id)),
        root.map(|id| {
            product
                .classification_assignments
                .iter()
                .filter(|((occurrence_id, _), _)| *occurrence_id == id)
                .collect::<Vec<_>>()
        }),
        product
            .instance_transform_overrides
            .iter()
            .filter(|(path, _)| names(path))
            .collect::<Vec<_>>(),
        product
            .support
            .grounded_instances
            .iter()
            .filter(|path| names(path))
            .collect::<Vec<_>>(),
        product
            .support
            .contact_joints
            .iter()
            .filter(|joint| joint.parts.iter().any(&names))
            .collect::<Vec<_>>(),
    )
}

/// Whether a step of `path` is looked up among the local groups or occurrences of
/// `definition_id`.
fn path_goes_through(
    product: &ProductModel,
    path: &InstancePath,
    definition_id: DefinitionId,
) -> bool {
    let Some(root) = product.occurrences.get(&path.root_occurrence()) else {
        return false;
    };
    let mut owner = root.definition_id;
    for step in path.steps() {
        if owner == definition_id {
            return true;
        }
        if let InstancePathStep::Occurrence(local_id) = *step {
            let Some(occurrence) = product.local_occurrences.get(&LocalOccurrenceKey {
                definition_id: owner,
                local_id,
            }) else {
                return false;
            };
            owner = occurrence.definition_id;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn struct_fields(source: &str, header: &str) -> Vec<String> {
        let body = source
            .split_once(header)
            .and_then(|(_, rest)| rest.split_once("\n}"))
            .map(|(body, _)| body)
            .unwrap_or_else(|| panic!("{header} not found"));
        body.lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("pub(crate) ")
                    .or(line.trim().strip_prefix("pub "))
            })
            .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim().to_owned()))
            .collect()
    }

    #[test]
    fn every_product_field_is_classified_as_pruned_or_kept() {
        let mut fields = struct_fields(
            include_str!("../document.rs"),
            "pub(crate) struct ProductModel {",
        );
        fields.extend(struct_fields(
            include_str!("support.rs"),
            "pub(crate) struct SupportDeclarations {",
        ));
        assert!(fields.len() > 40, "{fields:?}");
        let unclassified = fields
            .iter()
            .filter(|field| !PRUNED.contains(&field.as_str()) && !KEPT.contains(&field.as_str()))
            .collect::<Vec<_>>();
        assert!(
            unclassified.is_empty(),
            "classify in part_references.rs whether a batch prunes these when a part goes: {unclassified:?}"
        );
        let stale = PRUNED
            .iter()
            .chain(KEPT)
            .filter(|name| !fields.iter().any(|field| field == *name))
            .collect::<Vec<_>>();
        assert!(stale.is_empty(), "no longer product fields: {stale:?}");
    }
}
