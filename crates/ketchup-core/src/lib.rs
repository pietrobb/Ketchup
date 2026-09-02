#![forbid(unsafe_code)]

pub mod adapters;
pub mod assembly;
pub mod assembly_joint;
pub mod assistant_sidecar;
#[cfg(feature = "named-product-fixtures")]
pub mod beam_m4ae;
#[cfg(feature = "named-product-fixtures")]
pub mod beam_m5;
pub mod exact_revolve;

pub mod bottle_m6 {
    pub use crate::exact_revolve::*;
}
pub mod document;
pub mod drawing;
pub mod exact_brep_graph;
pub mod exact_product;
pub mod exact_validation;
pub mod extension;
pub mod fabrication;
pub mod feature_history;
pub mod graph;
pub mod import;
pub mod intent;
pub mod linear_hardware;
pub mod mechanical_contract;
pub mod mechanical_coupling;
pub mod persistence;
pub mod prismatic;
pub mod reference_examples;
#[cfg(feature = "named-product-fixtures")]
pub mod release_capstone;
pub mod shared_change;
pub mod sketch;
pub mod space;
pub mod state_view;
pub mod topology;
pub mod validation;
pub mod validator_hosting;

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
}
