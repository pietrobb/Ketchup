#![forbid(unsafe_code)]

pub mod adapters;
pub mod assembly;
pub mod assistant_sidecar;
pub mod beam_m4ae;
pub mod beam_m5;
pub mod bottle_m6;
pub mod document;
pub mod drawing;
pub mod exact_product;
pub mod exact_validation;
pub mod extension;
pub mod fabrication;
pub mod feature_history;
pub mod graph;
pub mod import;
pub mod intent;
pub mod persistence;
pub mod prismatic;
pub mod shared_change;
pub mod sketch;
pub mod space;
pub mod state_view;
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
