#![forbid(unsafe_code)]

pub mod adapters;
pub mod beam_m4ae;
pub mod beam_m5;
pub mod document;
pub mod exact_product;
pub mod exact_validation;
pub mod fabrication;
pub mod graph;
pub mod persistence;
pub mod prismatic;
pub mod state_view;
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
}
