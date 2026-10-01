//! Single integration test binary for this crate: every `tests/*.rs` file is a
//! module here, so the crate links one test executable instead of one per file.

mod assistant_process;
mod exact_brep_graph;
mod exact_evidence_transport;
mod generic_sketch_pocket;
mod job_lifecycle;
mod plugin_sandbox;
mod scheduler_identity;
mod scheduler_reliability;
mod sheet_metal;
mod validator_sandbox;
mod weldment_joint;
mod weldment_member;

#[path = "../../ketchup-model/tests/support/integration_support.rs"]
mod integration_support;

#[test]
fn every_test_file_is_registered() {
    integration_support::assert_every_test_file_is_registered(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")),
        include_str!("integration.rs"),
    );
}
