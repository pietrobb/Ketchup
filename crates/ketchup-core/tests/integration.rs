//! Single integration test binary for this crate: every `tests/*.rs` file is a
//! module here, so the crate links one test executable instead of one per file.

mod assembly_contract;
mod assembly_joint_types;
mod assembly_kinematics_contract;
mod assembly_motion_couplings;
mod assembly_recipe;
mod blender_glb_export;
mod body_contract;
mod cam_plan;
mod component_replacement_impact;
mod component_replacement_impact_verifier;
mod dxf_export;
mod dxf_import;
mod exact_brep_graph;
mod exact_brep_graph_resources;
mod exact_parameter_editing;
mod exact_parameter_editing_verifier;
mod face_ref_migration;
mod fea;
mod feature_history;
mod feature_history_verifier;
mod free_workplane;
mod gate_a1;
mod generic_sketch_pocket;
mod graph_m2;
mod import_contract;
mod legacy_migration;
mod local_pdm;
mod mechanical_contract;
mod mesh_recognition;
mod occurrence_fork_impact;
mod orthographic_drawing;
mod pad_migration;
mod pad_pocket;
mod persistence_m2;
mod product_document;
mod production_codes;
mod profile_migration;
mod shared_change_impact;
mod sketch_dof;
mod sketchup_scene_import;
mod state_view;
mod stl_import;
mod suffix_suppress_resume;
mod suffix_suppress_resume_verifier;
mod three_mf_export;
mod topology_identity;
mod validation_m17;
mod validator_hosting_m7c;
mod workplane_sketch;

#[path = "support/integration_support.rs"]
mod integration_support;

#[test]
fn every_test_file_is_registered() {
    integration_support::assert_every_test_file_is_registered(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests")),
        include_str!("integration.rs"),
    );
}
