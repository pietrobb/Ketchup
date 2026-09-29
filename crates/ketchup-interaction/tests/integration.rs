//! Single integration test binary for this crate: every `tests/*.rs` file is a
//! module here, so the crate links one test executable instead of one per file.

mod face_intent;
mod gate_c1a_projection_authority;
mod gate_c_interaction;
mod push_pull_gesture;
mod rectangle_face_authoring;
mod spatial_m16;

#[path = "../../ketchup-core/tests/support/integration_support.rs"]
mod integration_support;

#[test]
fn every_test_file_is_registered() {
    integration_support::assert_every_test_file_is_registered(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")),
        include_str!("integration.rs"),
    );
}
