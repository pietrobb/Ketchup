//! Single integration test binary for this crate: every `tests/*.rs` file is a
//! module here, so the crate links one test executable instead of one per file.

mod components;
mod connectivity;
mod contact_review;
mod document;
mod exact_shapes;
mod example_programs;
mod expect;
mod face_at;
mod faces;
mod motion;
mod profile_arcs;
mod program;
mod relations;
mod report_material;
mod revolve_bounds;
mod support;
mod sweep_loft;

#[path = "../../ketchup-model/tests/support/integration_support.rs"]
mod integration_support;

#[test]
fn every_test_file_is_registered() {
    integration_support::assert_every_test_file_is_registered(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")),
        include_str!("integration.rs"),
    );
}
