//! Single integration test binary for this crate: every `tests/*.rs` file is a
//! module here, so the crate links one test executable instead of one per file.

mod application_planner_parity;
mod assembly_ui;
mod body_ui;
mod cad_programs;
mod capstone;
mod capstone_chain;
mod drawing_tools_ui;
mod exact_house_xray;
mod face_workflow_ui;
mod feature_history_ui;
mod feature_history_ui_verifier;
mod file_workflow;
mod garden_studio_source_parity;
mod gravity_support_panel;
mod harness;
mod headless_shell;
mod helix_thread_tools;
mod instanced_rendering;
mod line_spatial_ui;
mod live_bridge_bootstrap;
mod live_bridge_image;
mod live_bridge_transactions;
mod manual_bottle_documentation;
mod nested_transform_ui;
mod occurrence_color_ui;
mod performance_ai_house;
mod production_performance;
mod rectangle_context_ui;
mod rectangle_spatial_ui;
mod scene_snapping_ui;
mod timber_frame_house;
mod validation_context;
mod validator_panel_ui;
mod viewer_export_house;
mod viewer_release_smoke;
mod viewport_shell;

#[path = "../../ketchup-model/tests/support/integration_support.rs"]
mod integration_support;

#[test]
fn every_test_file_is_registered() {
    integration_support::assert_every_test_file_is_registered(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")),
        include_str!("integration.rs"),
    );
}
