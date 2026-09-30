#![forbid(unsafe_code)]

pub mod adapters;
pub mod assembly;
pub mod assembly_joint;
pub mod assembly_recipe;
pub mod cam;
pub mod document;
pub mod drawing;
pub mod drawing_export;
pub mod exact_brep_graph;
pub mod exact_product;
pub mod exact_validation;
pub mod feature_history;
pub mod graph;
pub mod import;
pub mod mechanical_contract;
pub mod mechanical_coupling;
pub mod mesh_recognition;
pub mod persistence;
pub mod pin_joint;
pub mod shared_change;
pub mod sheet_metal;
pub mod space;
pub mod state_view;
#[cfg(feature = "testing")]
pub mod testing;
pub use ketchup_tolerance as tolerance;
pub mod topology;
pub mod validation;

/// Returns the canonical application name for toolchain smoke tests.
#[must_use]
pub const fn application_name() -> &'static str {
    "Ketchup"
}

#[cfg(test)]
mod tests {
    use super::application_name;

    #[test]
    fn application_name_is_stable() {
        assert_eq!(application_name(), "Ketchup");
    }

    #[test]
    fn geometry_errors_keep_their_canonical_codes() {
        use crate::document::CanonicalError;
        use crate::graph::GraphError;
        use ketchup_geometry::dimension::DimensionError;
        use ketchup_geometry::slot::SlotError;

        let codes = [
            DimensionError::EmptySourceToken,
            DimensionError::InvalidDecimalToken,
            DimensionError::OutsideEnvelope,
        ]
        .map(|error| CanonicalError::from(error).code());
        assert_eq!(
            codes,
            [
                "canonical.empty_source_token",
                "canonical.invalid_decimal_token",
                "canonical.dimension_outside_envelope",
            ]
        );
        assert_eq!(
            CanonicalError::from(DimensionError::OutsideEnvelope).to_string(),
            DimensionError::OutsideEnvelope.to_string()
        );
        for (slot, graph) in [
            (SlotError::ReservedNodeId, GraphError::ReservedNodeId),
            (
                SlotError::InvalidSemanticKey,
                GraphError::InvalidSemanticKey,
            ),
            (SlotError::EmptySlotPath, GraphError::EmptySlotPath),
            (SlotError::SlotPathLimit, GraphError::SlotPathLimit),
        ] {
            assert_eq!(GraphError::from(slot), graph);
            assert_eq!(graph.to_string(), slot.to_string());
        }
    }
}
