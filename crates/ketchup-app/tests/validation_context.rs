//! Read-only validation evidence (collision, support, physics, hardware and
//! spatial roles) as MCP `validate` and the validator panel read it.

use crate::harness::{
    BoxPart, Shell, replace_scene_with_boxes, rotate_occurrence, translate_occurrence,
};
use eframe::egui;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_assistant::sidecar::{AssistantCadEditOperation, AssistantCadEditProgram};
use ketchup_model::document::{ClassificationCategoryId, ClassificationDimensionId};
use ketchup_model::validation::VALIDATOR_ROLE_DIMENSION_V1;
use std::collections::{BTreeMap, BTreeSet};

fn apply_evaluator_inputs(shell: &mut Shell, inputs: &[(&str, f64)]) {
    for (index, (name, value)) in inputs.iter().enumerate() {
        shell
            .app_mut()
            .headless_apply_cad_program(&AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::CreateEvaluatorInput {
                    node_id: 900 + index as u64,
                    name: (*name).to_owned(),
                    value: *value,
                }],
            })
            .unwrap();
    }
}

fn select_occurrence_named(shell: &mut Shell, name: &str) {
    let occurrence_id = shell
        .app()
        .document_snapshot()
        .occurrences()
        .find(|occurrence| occurrence.name() == name)
        .unwrap_or_else(|| panic!("no occurrence named {name}"))
        .id();
    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
}

fn assign_validator_roles(shell: &mut Shell, assignments: &[(&str, &str, &str)]) {
    let first_role = assignments.first().unwrap().2;
    assert!(
        shell
            .app_mut()
            .create_classification_dimension(VALIDATOR_ROLE_DIMENSION_V1, first_role)
    );
    let dimension_id = shell
        .app()
        .document_snapshot()
        .classification_dimensions()
        .find(|dimension| dimension.name() == VALIDATOR_ROLE_DIMENSION_V1)
        .unwrap()
        .id();
    for role in assignments
        .iter()
        .map(|(_, _, role)| *role)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|role| *role != first_role)
    {
        assert!(
            shell
                .app_mut()
                .add_classification_category(dimension_id, role)
        );
    }
    let role_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .classification_dimension(dimension_id)
            .unwrap()
            .categories()
            .map(|category| (category.name().to_owned(), category.id()))
            .collect::<BTreeMap<_, _>>()
    };
    for (name, _dimensions, role) in assignments {
        select_occurrence_named(shell, name);
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(dimension_id, Some(role_ids[*role])),
            "failed to assign validator role {role} to {name}"
        );
    }
}

#[test]
fn validation_context_runs_current_collision_validation_without_mutating_or_adding_undo() {
    let mut shell = Shell::new();
    replace_scene_with_boxes(
        shell.app_mut(),
        &[
            BoxPart {
                name: "Left cabinet",
                size_mm: [100.0, 100.0, 100.0],
                origin_mm: [0.0, 0.0, 0.0],
            },
            BoxPart {
                name: "Right cabinet",
                size_mm: [100.0, 100.0, 100.0],
                origin_mm: [50.0, 0.0, 0.0],
            },
        ],
    );
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    let context = shell.app_mut().headless_validation_context(&[]);
    let validation = &context;
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], false);
    assert_eq!(validation["collision"]["state"], "failed");
    assert_eq!(validation["collision"]["complete"], true);
    assert_eq!(validation["checked_occurrence_count"], 2);
    assert_eq!(validation["checked_pair_count"], 1);
    assert_eq!(validation["issue_count"], 1);
    assert_eq!(validation["issues"][0]["code"], "collision.detected");
    assert_eq!(validation["issues"][0]["left_name"], "Left cabinet");
    assert_eq!(validation["issues"][0]["right_name"], "Right cabinet");
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    let right_id = validation["issues"][0]["right_occurrence_id"]
        .as_u64()
        .unwrap();
    translate_occurrence(shell.app_mut(), right_id, [50.0, 0.0, 0.0]);
    let clean_revision = shell.app().document_revision();
    let clean_undo_steps = shell.app().undo_step_count();
    let clean = shell.app_mut().headless_validation_context(&[]);
    assert_eq!(clean["state"], "not_evaluated");
    assert_eq!(clean["complete"], false);
    assert_eq!(clean["collision"]["state"], "passed");
    assert_eq!(clean["collision"]["complete"], true);
    assert_eq!(clean["issue_count"], 0);
    assert_eq!(shell.app().document_revision(), clean_revision);
    assert_eq!(shell.app().undo_step_count(), clean_undo_steps);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(
        shell.app_mut().headless_validation_context(&[])["state"],
        "failed"
    );
}
#[test]
fn validation_context_finds_transitively_supported_and_floating_parts_without_mutation() {
    let mut shell = Shell::new();
    replace_scene_with_boxes(
        shell.app_mut(),
        &[
            BoxPart {
                name: "Opaque alpha",
                size_mm: [100.0, 100.0, 10.0],
                origin_mm: [0.0, 0.0, 0.0],
            },
            BoxPart {
                name: "Floating decoy beta",
                size_mm: [80.0, 80.0, 10.0],
                origin_mm: [10.0, 10.0, 10.0],
            },
            BoxPart {
                name: "Opaque gamma",
                size_mm: [60.0, 60.0, 10.0],
                origin_mm: [20.0, 20.0, 20.0],
            },
            BoxPart {
                name: "Supported decoy delta",
                size_mm: [20.0, 20.0, 20.0],
                origin_mm: [200.0, 0.0, 50.0],
            },
        ],
    );
    assign_validator_roles(
        &mut shell,
        &[
            (
                "Opaque alpha",
                "100 × 100 × 10",
                "physics.gravity.ground:stack",
            ),
            (
                "Floating decoy beta",
                "80 × 80 × 10",
                "physics.gravity.body:stack",
            ),
            ("Opaque gamma", "60 × 60 × 10", "physics.gravity.body:stack"),
            (
                "Supported decoy delta",
                "20 × 20 × 20",
                "physics.gravity.body:isolated",
            ),
        ],
    );
    apply_evaluator_inputs(
        &mut shell,
        &[
            ("physics.gravity_x_m_s2", 0.0),
            ("physics.gravity_y_m_s2", 0.0),
            ("physics.gravity_z_m_s2", -9.81),
        ],
    );
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let context = shell.app_mut().headless_validation_context(&[]);
    let gravity = &context["gravity_support"];
    assert_eq!(gravity["state"], "failed", "{context:#}");
    assert_eq!(gravity["complete"], true);
    assert_eq!(gravity["checked_occurrence_count"], 4);
    assert_eq!(gravity["unsupported_count"], 1);
    assert_eq!(gravity["issues"][0]["code"], "gravity.unsupported");
    assert_eq!(gravity["issues"][0]["name"], "Supported decoy delta");
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    let floating_id = gravity["issues"][0]["occurrence_id"].as_u64().unwrap();
    translate_occurrence(shell.app_mut(), floating_id, [0.0, 0.0, -50.0]);
    shell.settle();
    let still_unseeded = shell.app_mut().headless_validation_context(&[]);
    assert_eq!(still_unseeded["gravity_support"]["state"], "failed");
    assert_eq!(still_unseeded["gravity_support"]["unsupported_count"], 1);
}
#[test]
fn validation_context_reports_beam_deflection_tipping_and_anchoring_with_explicit_limits() {
    let mut shell = Shell::new();
    replace_scene_with_boxes(
        shell.app_mut(),
        &[
            BoxPart {
                name: "Opaque panel A",
                size_mm: [1_000.0, 300.0, 12.0],
                origin_mm: [0.0, 0.0, 0.0],
            },
            BoxPart {
                name: "Shelf decoy B",
                size_mm: [300.0, 400.0, 1_800.0],
                origin_mm: [1_500.0, 0.0, 0.0],
            },
            BoxPart {
                name: "Opaque case C",
                size_mm: [800.0, 500.0, 600.0],
                origin_mm: [2_500.0, 0.0, 0.0],
            },
            BoxPart {
                name: "Tall cabinet unassigned decoy",
                size_mm: [100.0, 100.0, 2_000.0],
                origin_mm: [4_000.0, 0.0, 0.0],
            },
        ],
    );
    shell.settle();
    let (shelf_occurrence_id, tall_case_occurrence_id, low_case_occurrence_id) = {
        let snapshot = shell.app().document_snapshot();
        let occurrence_id = |name: &str| {
            snapshot
                .occurrences()
                .find(|occurrence| occurrence.name() == name)
                .unwrap()
                .id()
        };
        (
            occurrence_id("Opaque panel A"),
            occurrence_id("Shelf decoy B"),
            occurrence_id("Opaque case C"),
        )
    };

    assert!(
        shell
            .app_mut()
            .create_classification_dimension(VALIDATOR_ROLE_DIMENSION_V1, "physics.beam.xy",)
    );
    let role_dimension_id: ClassificationDimensionId = shell
        .app()
        .document_snapshot()
        .classification_dimensions()
        .find(|dimension| dimension.name() == VALIDATOR_ROLE_DIMENSION_V1)
        .unwrap()
        .id();
    assert!(
        shell
            .app_mut()
            .add_classification_category(role_dimension_id, "physics.freestanding.z")
    );
    let (shelf_role_id, case_role_id): (ClassificationCategoryId, ClassificationCategoryId) = {
        let snapshot = shell.app().document_snapshot();
        let dimension = snapshot
            .classification_dimension(role_dimension_id)
            .unwrap();
        (
            dimension
                .categories()
                .find(|category| category.name() == "physics.beam.xy")
                .unwrap()
                .id(),
            dimension
                .categories()
                .find(|category| category.name() == "physics.freestanding.z")
                .unwrap()
                .id(),
        )
    };
    for (occurrence_id, role_id, name, _dimensions) in [
        (
            shelf_occurrence_id,
            shelf_role_id,
            "Opaque panel A",
            "1000 × 300 × 12",
        ),
        (
            tall_case_occurrence_id,
            case_role_id,
            "Shelf decoy B",
            "300 × 400 × 1800",
        ),
        (
            low_case_occurrence_id,
            case_role_id,
            "Opaque case C",
            "800 × 500 × 600",
        ),
    ] {
        select_occurrence_named(&mut shell, name);
        assert!(shell.app().occurrence_is_selected(occurrence_id));
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(role_dimension_id, Some(role_id))
        );
    }

    rotate_occurrence(
        shell.app_mut(),
        shelf_occurrence_id.0,
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        45.0,
    );
    rotate_occurrence(
        shell.app_mut(),
        tall_case_occurrence_id.0,
        [1_500.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        45.0,
    );
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let validation_context =
        shell
            .app_mut()
            .headless_validation_context(&["beam_deflection", "tipping", "anchoring"]);
    let validation = &validation_context;
    assert_eq!(
        validation["requested"],
        serde_json::json!(["beam_deflection", "tipping", "anchoring"])
    );
    assert_eq!(validation["executed"], validation["requested"]);
    assert_eq!(
        validation["skipped"],
        serde_json::json!([
            "group_connectivity",
            "collision",
            "assembly_retention",
            "gravity_support",
            "hardware_manufacturing",
            "room_placement",
            "passage_clearance",
            "clearance_occupancy",
            "static_load"
        ])
    );
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], true);
    assert_eq!(validation["issue_count"], 3);

    let shelf = &validation["beam_deflection"];
    assert_eq!(shelf["state"], "failed");
    assert_eq!(shelf["complete"], true);
    assert_eq!(shelf["applicable_count"], 1);
    assert_eq!(shelf["issue_count"], 1);
    assert_eq!(
        shelf["issues"][0]["code"],
        "physics.beam_deflection_exceeded"
    );
    assert_eq!(shelf["issues"][0]["name"], "Opaque panel A");
    assert_eq!(shelf["issues"][0]["role"], "physics.beam.xy");
    assert_eq!(
        shelf["issues"][0]["role_source"],
        "canonical_classification"
    );
    assert_eq!(shelf["issues"][0]["span_mm"], 1_000.0);
    assert_eq!(shelf["issues"][0]["depth_mm"], 300.0);
    assert_eq!(shelf["issues"][0]["thickness_mm"], 12.0);
    assert_eq!(shelf["inputs"]["design_load_n"], 500.0);
    assert_eq!(shelf["inputs"]["default_material"], "engineered_wood");
    assert_eq!(shelf["issues"][0]["material"], "engineered_wood");
    assert_eq!(shelf["issues"][0]["elastic_modulus_n_mm2"], 2_500.0);
    assert!(
        shelf["issues"][0]["predicted_deflection_mm"]
            .as_f64()
            .unwrap()
            > shelf["issues"][0]["allowable_deflection_mm"]
                .as_f64()
                .unwrap()
    );

    let tipping = &validation["tipping"];
    assert_eq!(tipping["state"], "failed");
    assert_eq!(tipping["applicable_count"], 2);
    assert_eq!(tipping["issue_count"], 1);
    assert_eq!(
        tipping["issues"][0]["code"],
        "stability.tip_angle_below_limit"
    );
    assert_eq!(tipping["issues"][0]["name"], "Shelf decoy B");
    assert_eq!(tipping["issues"][0]["role"], "physics.freestanding.z");
    assert_eq!(
        tipping["issues"][0]["role_source"],
        "canonical_classification"
    );
    assert_eq!(tipping["issues"][0]["base_depth_mm"], 300.0);
    assert_eq!(tipping["issues"][0]["height_mm"], 1_800.0);
    assert_eq!(tipping["limit"]["minimum_tip_angle_degrees"], 15.0);

    let anchoring = &validation["anchoring"];
    assert_eq!(anchoring["state"], "failed");
    assert_eq!(anchoring["applicable_count"], 2);
    assert_eq!(anchoring["required_count"], 1);
    assert_eq!(anchoring["issues"][0]["code"], "stability.anchor_required");
    assert_eq!(anchoring["issues"][0]["name"], "Shelf decoy B");
    assert_eq!(anchoring["issues"][0]["role"], "physics.freestanding.z");
    assert_eq!(anchoring["issues"][0]["base_depth_mm"], 300.0);
    assert_eq!(anchoring["issues"][0]["height_mm"], 1_800.0);
    assert_eq!(
        anchoring["issues"][0]["anchor_declaration"],
        "not_available_in_current_document_schema"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}
#[test]
fn validation_context_reports_hardware_and_manufacturing_from_roles_and_source_geometry() {
    let mut shell = Shell::with_dialogs_at_size(
        ScriptedFileDialogs::new(),
        egui::Vec2::new(1_280.0, 1_600.0),
    );
    replace_scene_with_boxes(
        shell.app_mut(),
        &[
            BoxPart {
                name: "Opaque manufacturing A",
                size_mm: [600.0, 500.0, 18.0],
                origin_mm: [0.0, 0.0, 0.0],
            },
            BoxPart {
                name: "Opaque manufacturing B",
                size_mm: [30.0, 30.0, 10.0],
                origin_mm: [1.0, 50.0, 4.0],
            },
            BoxPart {
                name: "Opaque manufacturing C",
                size_mm: [10.0, 10.0, 18.0],
                origin_mm: [28.0, 50.0, 0.0],
            },
            BoxPart {
                name: "Opaque manufacturing D",
                size_mm: [500.0, 12.0, 45.0],
                origin_mm: [0.0, 600.0, 100.0],
            },
            BoxPart {
                name: "Opaque manufacturing E",
                size_mm: [480.0, 12.0, 45.0],
                origin_mm: [0.0, 700.0, 105.0],
            },
            BoxPart {
                name: "Opaque manufacturing F",
                size_mm: [600.0, 500.0, 4.0],
                origin_mm: [1_000.0, 0.0, 0.0],
            },
        ],
    );
    shell.settle();
    let occurrence_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .occurrences()
            .map(|occurrence| (occurrence.name().to_owned(), occurrence.id()))
            .collect::<BTreeMap<_, _>>()
    };
    let role_names = [
        "manufacturing.panel.xy:door-a",
        "manufacturing.cup-bore.z:door-a",
        "manufacturing.hole.z:door-a",
        "hardware.linear-pair.x:drawer-a",
        "manufacturing.panel.xy:back-a",
    ];
    assert!(
        shell
            .app_mut()
            .create_classification_dimension(VALIDATOR_ROLE_DIMENSION_V1, role_names[0],)
    );
    let role_dimension_id = shell
        .app()
        .document_snapshot()
        .classification_dimensions()
        .find(|dimension| dimension.name() == VALIDATOR_ROLE_DIMENSION_V1)
        .unwrap()
        .id();
    for role in &role_names[1..] {
        assert!(
            shell
                .app_mut()
                .add_classification_category(role_dimension_id, role)
        );
    }
    let role_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .classification_dimension(role_dimension_id)
            .unwrap()
            .categories()
            .map(|category| (category.name().to_owned(), category.id()))
            .collect::<BTreeMap<_, _>>()
    };
    for (name, _dimensions, role) in [
        ("Opaque manufacturing A", "600 × 500 × 18", role_names[0]),
        ("Opaque manufacturing B", "30 × 30 × 10", role_names[1]),
        ("Opaque manufacturing C", "10 × 10 × 18", role_names[2]),
        ("Opaque manufacturing D", "500 × 12 × 45", role_names[3]),
        ("Opaque manufacturing E", "480 × 12 × 45", role_names[3]),
        ("Opaque manufacturing F", "600 × 500 × 4", role_names[4]),
    ] {
        select_occurrence_named(&mut shell, name);
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(role_dimension_id, Some(role_ids[role]),)
        );
    }
    for occurrence_id in occurrence_ids.values() {
        rotate_occurrence(
            shell.app_mut(),
            occurrence_id.0,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            45.0,
        );
    }
    // Which topology the validator reads depends on whether the exact worker has
    // answered: wait for all six solids so the run does not race it.
    let exact = shell.wait_until(|shell| {
        shell.settle();
        shell.app().exact_render_body_count() == occurrence_ids.len()
    });
    assert!(
        exact,
        "expected {} exact bodies, found {}",
        occurrence_ids.len(),
        shell.app().exact_render_body_count()
    );
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let validation_context = shell
        .app_mut()
        .headless_validation_context(&["hardware_manufacturing"]);
    let validation = &validation_context;
    assert_eq!(
        validation["requested"],
        serde_json::json!(["hardware_manufacturing"])
    );
    assert_eq!(validation["executed"], validation["requested"]);
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], true);
    let manufacturing = &validation["hardware_manufacturing"];
    assert_eq!(manufacturing["state"], "failed");
    assert_eq!(manufacturing["complete"], true);
    assert_eq!(manufacturing["issue_count"], 5);
    assert_eq!(manufacturing["not_evaluated"], serde_json::json!([]));
    assert_eq!(
        manufacturing["limits"]["minimum_hole_edge_material_mm"],
        5.0
    );
    assert_eq!(
        manufacturing["limits"]["minimum_hole_spacing_material_mm"],
        3.0
    );
    assert_eq!(manufacturing["limits"]["minimum_cup_diameter_mm"], 35.0);
    assert_eq!(manufacturing["limits"]["minimum_cup_depth_mm"], 12.0);
    assert_eq!(manufacturing["limits"]["minimum_panel_thickness_mm"], 6.0);
    assert_eq!(
        manufacturing["assumptions"],
        serde_json::json!([
            "canonical validator roles declare panels, holes, cup bores, linear-hardware pairs, source axes, and association groups",
            "a hole or cup-bore association group must resolve to exactly one explicitly declared host panel",
            "panel thickness and hole depth use declared source-frame axes bound to accepted topology",
            "hole radial envelopes are conservatively treated as circular using their largest source-frame radial extent",
            "each linear-hardware association group must contain exactly two members",
        ])
    );
    let issues = manufacturing["issues"].as_array().unwrap();
    let issue = |code: &str| {
        issues
            .iter()
            .find(|issue| issue["code"] == code)
            .unwrap_or_else(|| panic!("missing {code} in {issues:#?}"))
    };
    assert_eq!(
        issue("manufacturing.panel_below_minimum_thickness")["name"],
        "Opaque manufacturing F"
    );
    assert_eq!(
        issue("manufacturing.panel_below_minimum_thickness")["role"],
        "manufacturing.panel.xy:back-a"
    );
    assert_eq!(
        issue("manufacturing.panel_below_minimum_thickness")["thickness_mm"],
        4.0
    );
    assert_eq!(
        issue("manufacturing.hole_too_close_to_edge")["name"],
        "Opaque manufacturing B"
    );
    assert_eq!(
        issue("manufacturing.hole_too_close_to_edge")["host_name"],
        "Opaque manufacturing A"
    );
    assert_eq!(
        issue("manufacturing.cup_bore_envelope_below_minimum")["name"],
        "Opaque manufacturing B"
    );
    assert_eq!(
        issue("manufacturing.hole_spacing_below_minimum")["left_name"],
        "Opaque manufacturing B"
    );
    assert_eq!(
        issue("hardware.linear_pair_misaligned")["left_name"],
        "Opaque manufacturing D"
    );
    assert_eq!(
        issue("hardware.linear_pair_misaligned")["right_name"],
        "Opaque manufacturing E"
    );
    let evaluations = manufacturing["evaluations"].as_array().unwrap();
    let hole = evaluations
        .iter()
        .find(|evaluation| evaluation["rule"] == "hole_edge_distance")
        .unwrap();
    assert_eq!(hole["role"], "manufacturing.cup-bore.z:door-a");
    assert_eq!(hole["host_role"], "manufacturing.panel.xy:door-a");
    assert_eq!(hole["topology_source"], "accepted_exact_brep");
    let pair = evaluations
        .iter()
        .find(|evaluation| evaluation["rule"] == "linear_hardware_pair_alignment")
        .unwrap();
    assert_eq!(pair["length_axis"], 0);
    assert_eq!(pair["length_mismatch_mm"], 20.0);
    assert_eq!(
        pair["topology_sources"],
        serde_json::json!(["accepted_exact_brep", "accepted_exact_brep"])
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}
#[test]
fn validation_context_reports_spatial_roles_and_oriented_narrow_phase() {
    let mut shell = Shell::with_dialogs_at_size(
        ScriptedFileDialogs::new(),
        egui::Vec2::new(1_280.0, 1_600.0),
    );
    replace_scene_with_boxes(
        shell.app_mut(),
        &[
            BoxPart {
                name: "Passage decoy A",
                size_mm: [4_000.0, 3_000.0, 2_500.0],
                origin_mm: [0.0, 0.0, 0.0],
            },
            BoxPart {
                name: "Room decoy B",
                size_mm: [600.0, 400.0, 1_800.0],
                origin_mm: [200.0, 200.0, 0.0],
            },
            BoxPart {
                name: "Opaque spatial C",
                size_mm: [300.0, 600.0, 800.0],
                origin_mm: [3_900.0, 100.0, 0.0],
            },
            BoxPart {
                name: "Furniture decoy D",
                size_mm: [800.0, 2_500.0, 1_900.0],
                origin_mm: [1_000.0, 250.0, 0.0],
            },
            BoxPart {
                name: "Room envelope decoy E",
                size_mm: [500.0, 800.0, 900.0],
                origin_mm: [1_200.0, 1_000.0, 0.0],
            },
            BoxPart {
                name: "Passage decoy F",
                size_mm: [300.0, 500.0, 900.0],
                origin_mm: [1_900.0, 1_000.0, 0.0],
            },
        ],
    );
    shell.settle();
    let occurrence_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .occurrences()
            .map(|occurrence| (occurrence.name().to_owned(), occurrence.id()))
            .collect::<BTreeMap<_, _>>()
    };
    let role_names = [
        "spatial.room:living",
        "spatial.occupant:living",
        "spatial.passage.xy:living",
        "spatial.obstacle:living",
    ];
    assert!(
        shell
            .app_mut()
            .create_classification_dimension(VALIDATOR_ROLE_DIMENSION_V1, role_names[0],)
    );
    let role_dimension_id = shell
        .app()
        .document_snapshot()
        .classification_dimensions()
        .find(|dimension| dimension.name() == VALIDATOR_ROLE_DIMENSION_V1)
        .unwrap()
        .id();
    for role in &role_names[1..] {
        assert!(
            shell
                .app_mut()
                .add_classification_category(role_dimension_id, role)
        );
    }
    let role_ids = {
        let snapshot = shell.app().document_snapshot();
        snapshot
            .classification_dimension(role_dimension_id)
            .unwrap()
            .categories()
            .map(|category| (category.name().to_owned(), category.id()))
            .collect::<BTreeMap<_, _>>()
    };
    for (name, _dimensions, role) in [
        ("Passage decoy A", "4000 × 3000 × 2500", role_names[0]),
        ("Room decoy B", "600 × 400 × 1800", role_names[1]),
        ("Opaque spatial C", "300 × 600 × 800", role_names[1]),
        ("Furniture decoy D", "800 × 2500 × 1900", role_names[2]),
        ("Room envelope decoy E", "500 × 800 × 900", role_names[1]),
        ("Passage decoy F", "300 × 500 × 900", role_names[3]),
    ] {
        select_occurrence_named(&mut shell, name);
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(role_dimension_id, Some(role_ids[role]),)
        );
    }
    for occurrence_id in occurrence_ids.values() {
        rotate_occurrence(
            shell.app_mut(),
            occurrence_id.0,
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            45.0,
        );
    }
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let validation_context = shell
        .app_mut()
        .headless_validation_context(&["room_placement", "passage_clearance"]);
    let validation = &validation_context;
    assert_eq!(
        validation["requested"],
        serde_json::json!(["room_placement", "passage_clearance"])
    );
    assert_eq!(validation["executed"], validation["requested"]);
    assert_eq!(validation["state"], "failed");
    assert_eq!(validation["complete"], true);
    assert_eq!(validation["issue_count"], 3);

    let placement = &validation["room_placement"];
    assert_eq!(placement["state"], "failed");
    assert_eq!(placement["complete"], true);
    assert_eq!(placement["applicable_count"], 3);
    assert_eq!(placement["issue_count"], 1);
    assert_eq!(
        placement["issues"][0]["code"],
        "room.occupant_outside_boundary"
    );
    assert_eq!(placement["issues"][0]["name"], "Opaque spatial C");
    assert_eq!(placement["issues"][0]["room_name"], "Passage decoy A");
    assert!(
        (placement["issues"][0]["outside_by_mm"]["right"]
            .as_f64()
            .unwrap()
            - 200.0)
            .abs()
            < 1.0e-9
    );
    assert_eq!(placement["issues"][0]["role"], "spatial.occupant:living");
    assert_eq!(placement["issues"][0]["room_role"], "spatial.room:living");
    assert_eq!(placement["issues"][0]["evidence_class"], "tolerant");
    assert_eq!(
        placement["issues"][0]["narrow_phase_method"],
        "ketchup.method.general-body-obb-sat.cpu-f64.v1"
    );

    let passage = &validation["passage_clearance"];
    assert_eq!(passage["state"], "failed");
    assert_eq!(passage["complete"], true);
    assert_eq!(passage["applicable_count"], 1);
    assert_eq!(passage["issue_count"], 2);
    assert_eq!(passage["limits"]["minimum_width_mm"], 900.0);
    assert_eq!(passage["limits"]["minimum_headroom_mm"], 2_000.0);
    assert_eq!(
        passage["evaluations"][0]["role"],
        "spatial.passage.xy:living"
    );
    assert_eq!(passage["evaluations"][0]["width_mm"], 800.0);
    assert_eq!(passage["evaluations"][0]["headroom_mm"], 1_900.0);
    let issues = passage["issues"].as_array().unwrap();
    assert!(issues.iter().any(|issue| {
        issue["code"] == "room.passage_envelope_below_minimum"
            && issue["name"] == "Furniture decoy D"
    }));
    let blocked = issues
        .iter()
        .find(|issue| issue["code"] == "room.passage_blocked")
        .unwrap();
    assert_eq!(blocked["obstacle_name"], "Room envelope decoy E");
    assert_eq!(blocked["obstacle_role"], "spatial.occupant:living");
    assert_eq!(blocked["narrow_phase_relation"], "intersecting");
    assert_eq!(
        blocked["narrow_phase_method"],
        "ketchup.method.general-body-obb-sat.cpu-f64.v1"
    );
    assert!(
        issues
            .iter()
            .all(|issue| issue["obstacle_name"] != "Passage decoy F"),
        "world-AABB-only overlap must not create a passage blocker: {issues:#?}"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}
#[test]
fn validation_context_does_not_claim_spatial_validation_without_canonical_roles() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let validation_context = shell
        .app_mut()
        .headless_validation_context(&["room_placement", "passage_clearance"]);
    let validation = &validation_context;
    assert_eq!(validation["state"], "not_evaluated");
    assert_eq!(validation["complete"], false);
    assert_eq!(
        validation["room_placement"]["role_error"],
        "validator roles cannot be read: classification dimension \"ketchup.validator-role.v1\" is missing"
    );
    assert_eq!(
        validation["passage_clearance"]["role_error"],
        "validator roles cannot be read: classification dimension \"ketchup.validator-role.v1\" is missing"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}
#[test]
fn validation_context_calculates_static_load_from_explicit_canonical_physics_inputs() {
    let mut shell = Shell::new();
    replace_scene_with_boxes(
        shell.app_mut(),
        &[
            BoxPart {
                name: "Support decoy epsilon",
                size_mm: [500.0, 500.0, 500.0],
                origin_mm: [0.0, 0.0, 500.0],
            },
            BoxPart {
                name: "Load decoy zeta",
                size_mm: [500.0, 500.0, 500.0],
                origin_mm: [0.0, 0.0, 0.0],
            },
        ],
    );
    assign_validator_roles(
        &mut shell,
        &[
            (
                "Support decoy epsilon",
                "500 × 500 × 500",
                "physics.static.load:case-a",
            ),
            (
                "Load decoy zeta",
                "500 × 500 × 500",
                "physics.static.support:case-a",
            ),
        ],
    );
    let scene = shell.app().document_snapshot().scene_query();
    let loaded_id = scene
        .iter()
        .find(|occurrence| occurrence.occurrence_name == "Support decoy epsilon")
        .unwrap()
        .occurrence_id
        .0;
    let support_id = scene
        .iter()
        .find(|occurrence| occurrence.occurrence_name == "Load decoy zeta")
        .unwrap()
        .occurrence_id
        .0;
    let mass_name = format!("physics.mass_kg.occurrence.{loaded_id}");
    let load_name = format!("physics.applied_load_n.occurrence.{loaded_id}");
    let capacity_name = format!("physics.support_capacity_n.occurrence.{support_id}");
    let qualification = serde_json::json!({
        "source": "synthetic test capacity, not design data", "units": "N",
        "mode": "compression", "direction_world": [0, 0, -1],
        "assumptions": "single support under axial compression", "additive": false
    })
    .to_string();
    for (name, dimension, value) in [
        (
            "Support decoy epsilon",
            "ketchup.static-load-mode.v1",
            "compression",
        ),
        (
            "Load decoy zeta",
            "ketchup.support-capacity.v1",
            qualification.as_str(),
        ),
    ] {
        assert!(
            shell
                .app_mut()
                .create_classification_dimension(dimension, value)
        );
        let snapshot = shell.app().document_snapshot();
        let dimension = snapshot
            .classification_dimension_named(dimension)
            .unwrap()
            .unwrap();
        let category = dimension.categories().next().unwrap().id();
        let occurrence = snapshot
            .occurrences()
            .find(|part| part.name() == name)
            .unwrap()
            .id();
        assert!(shell.app_mut().headless_select_occurrence(occurrence));
        assert!(
            shell
                .app_mut()
                .assign_selection_to_classification(dimension.id(), Some(category))
        );
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .occurrence_classification(occurrence, dimension.id()),
            Some(category),
            "{name}"
        );
    }
    apply_evaluator_inputs(
        &mut shell,
        &[
            ("physics.gravity_x_m_s2", 0.0),
            ("physics.gravity_y_m_s2", 0.0),
            ("physics.gravity_z_m_s2", -9.81),
            (&mass_name, 100.0),
            (&load_name, 200.0),
            (&capacity_name, 1_000.0),
        ],
    );
    shell.settle();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let validation_context = shell
        .app_mut()
        .headless_validation_context(&["static_load"]);
    let validation = &validation_context;
    assert_eq!(validation["requested"], serde_json::json!(["static_load"]));
    assert_eq!(validation["executed"], validation["requested"]);
    assert_eq!(validation["state"], "failed", "{validation:#}");
    assert_eq!(validation["complete"], true);
    assert_eq!(validation["issue_count"], 1);
    let report = &validation["static_load"];
    assert_eq!(report["state"], "failed");
    assert_eq!(report["complete"], true);
    assert_eq!(report["applicable_count"], 1);
    assert_eq!(report["issue_count"], 1);
    let evaluation = &report["evaluations"][0];
    assert_eq!(evaluation["occurrence_id"], loaded_id);
    assert_eq!(evaluation["name"], "Support decoy epsilon");
    assert_eq!(evaluation["mass"]["value_kg"], 100.0);
    assert_eq!(evaluation["applied_load"]["value_n"], 200.0);
    assert_eq!(
        evaluation["gravity"]["vector_m_s2"],
        serde_json::json!([0.0, 0.0, -9.81])
    );
    assert_eq!(
        evaluation["gravity"]["direction"],
        serde_json::json!([0.0, 0.0, -1.0])
    );
    assert_eq!(evaluation["weight_force_n"], 981.0);
    assert_eq!(evaluation["resultant_force_n"], 1_181.0);
    assert_eq!(evaluation["total_support_capacity_n"], 1_000.0);
    assert_eq!(evaluation["capacity_margin_n"], -181.0);
    assert_eq!(evaluation["supports"][0]["occurrence_id"], support_id);
    assert_eq!(evaluation["supports"][0]["name"], "Load decoy zeta");
    assert_eq!(
        report["issues"][0]["code"],
        "physics.support_capacity_exceeded"
    );
    assert_eq!(report["issues"][0]["capacity_shortfall_n"], 181.0);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}
#[test]
fn validation_context_does_not_estimate_static_load_without_explicit_physics_inputs() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let validation_context = shell
        .app_mut()
        .headless_validation_context(&["static_load"]);
    let validation = &validation_context;
    assert_eq!(validation["requested"], serde_json::json!(["static_load"]));
    assert_eq!(validation["state"], "not_evaluated");
    assert_eq!(validation["complete"], false);
    assert_eq!(validation["issue_count"], 0);
    assert_eq!(validation["static_load"]["state"], "not_evaluated");
    assert_eq!(
        validation["static_load"]["not_evaluated"][0]["reason"],
        "missing_or_ambiguous_canonical_roles"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}
