//! Single integration test binary for this crate: every `tests/*.rs` file is a
//! module here, so the crate links one test executable instead of one per file.

mod batch_task;
mod cad_program;
mod collision_brep;
mod contact_joints;
mod document_session;
mod evaluation_deadline;
mod group_connectivity;
mod mesh_conversion;
mod model_query;
mod model_query_catalogs;
mod operations_support;
mod rule_exact_assemblies;
mod rule_exact_collisions;
mod rule_program_colors;
mod rule_program_components;
mod rule_program_continuity;
mod rule_program_extras;
mod rule_program_face_shapes;
mod rule_program_groups;
mod rule_program_instances;
mod rule_program_local_groups;
mod rule_program_members;
mod rule_program_motion;
mod rule_program_named_topology;
mod rule_program_nested;
mod rule_program_operations;
mod rule_program_review;
mod rule_program_split_union;
mod rule_program_support;
mod rule_program_threads;
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
