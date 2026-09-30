//! Single integration test binary for this crate: every `tests/*.rs` file is a
//! module here, so the crate links one test executable instead of one per file.

mod batch_task;
mod cad_program;
mod collision_brep;
mod document_session;
mod evaluation_deadline;
mod mesh_conversion;
mod model_query;
mod model_query_catalogs;
mod operations_support;
mod rule_exact_collisions;
mod rule_program_extras;
mod rule_program_face_shapes;
mod rule_program_named_topology;
mod rule_program_operations;
mod rule_program_split_union;
mod validation_selection;
mod workflow_trace;

#[path = "../../ketchup-model/tests/support/integration_support.rs"]
mod integration_support;

#[test]
fn every_test_file_is_registered() {
    integration_support::assert_every_test_file_is_registered(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")),
        include_str!("integration.rs"),
    );
}
