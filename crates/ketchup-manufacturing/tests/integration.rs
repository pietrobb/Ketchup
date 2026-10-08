//! Single integration test binary for this crate: every `tests/*.rs` file is a
//! module here, so the crate links one test executable instead of one per file.

mod blender_glb_export;
mod dxf_export;
mod fabrication_validation;
mod three_mf_export;

#[path = "../../ketchup-model/tests/support/integration_support.rs"]
mod integration_support;

#[path = "support/btlx_blank.rs"]
mod btlx_blank;

#[test]
fn every_test_file_is_registered() {
    integration_support::assert_every_test_file_is_registered(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")),
        include_str!("integration.rs"),
    );
}
