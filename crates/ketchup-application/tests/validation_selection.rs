use std::collections::{BTreeMap, BTreeSet};

use ketchup_application::{
    AssistantValidationSelection, DocumentSession, StructuralValidationScope,
    plan_assistant_cad_edit_program, scoped_static_load_report,
    validation::{
        assistant_assembly_constraints_report, assistant_assembly_retention_report,
        assistant_static_load_report, assistant_validation_context,
    },
};
use ketchup_core::{
    assembly_joint::{
        AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind, AssemblyJointLimits,
    },
    assembly_recipe::{
        AssemblyRecipe, RecipeEditScope, RecipeFaceRef, RecipeKey, RecipePartAdoption,
        RecipePartMobility, RecipeRelation, RecipeRelationKind, RecognizedRecipeFeatureKind,
    },
    assistant_sidecar::{
        AssistantCadEditOperation, AssistantCadEditProgram, AssistantDowelJointFace,
        AssistantInstancePath, AssistantStandardDowel,
    },
    document::*,
    exact_product::ExactResultRegistry,
    joinery::DowelJointId,
    validation::ValidatorRoleIndex,
};

fn structural_document(occurrence_count: u64) -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Structural participant".into(),
        },
        CanonicalCommand::UpsertClassificationDimension {
            id: ClassificationDimensionId(1),
            name: "ketchup.validator-role.v1".into(),
            categories: vec![
                (
                    ClassificationCategoryId(1),
                    "physics.static.load:case-a".into(),
                ),
                (
                    ClassificationCategoryId(2),
                    "physics.static.support:case-a".into(),
                ),
            ],
        },
    ];
    commands.extend(
        (1..=occurrence_count).map(|id| CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(1),
            name: format!("Structural occurrence {id}"),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        }),
    );
    commands.extend([
        CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(1),
            dimension_id: ClassificationDimensionId(1),
            category_id: Some(ClassificationCategoryId(1)),
        },
        CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(2),
            dimension_id: ClassificationDimensionId(1),
            category_id: Some(ClassificationCategoryId(2)),
        },
    ]);
    for (id, name, value) in [
        (1, "physics.gravity_x_m_s2", 0.0),
        (2, "physics.gravity_y_m_s2", 0.0),
        (3, "physics.gravity_z_m_s2", -9.81),
        (4, "physics.mass_kg.occurrence.1", 100.0),
        (5, "physics.applied_load_n.occurrence.1", 200.0),
        (6, "physics.support_capacity_n.occurrence.2", 2_000.0),
    ] {
        commands.push(CanonicalCommand::CreateEvaluatorNode {
            id: NodeId(id),
            name: name.into(),
            dimension: Dimension::new(value.to_string(), value).unwrap(),
            dependencies: Vec::new(),
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

#[test]
fn unchecked_public_selection_cannot_report_success_without_a_known_validator() {
    let session = DocumentSession::default();
    for requested in [
        BTreeSet::from(["bogus"]),
        BTreeSet::from(["bogus", "collision"]),
    ] {
        let selection = AssistantValidationSelection {
            mode: "only",
            requested,
            unknown: Vec::new(),
        };
        assert!(!selection.is_valid());
        let report = session.validators(&selection);
        assert_eq!(report["state"], "not_evaluated");
        assert_eq!(report["complete"], false);
        assert_eq!(report["executed"], serde_json::json!([]));
        assert_eq!(
            report["selection_error"],
            "unknown_or_empty_validator_selection"
        );
    }
    assert_eq!(session.visible_undo_steps(), 0);
}

fn recipe_key(value: &str) -> RecipeKey {
    RecipeKey::new(value).unwrap()
}

fn contact_recipe_document(signed_gap_mm: f64) -> DocumentStore {
    let mut document = DocumentStore::new();
    let profile = |id, definition_id| CanonicalCommand::CreateFeature {
        id: FeatureId(id),
        definition_id: DefinitionId(definition_id),
        name: format!("Panel {definition_id} profile"),
        kind: FeatureKind::Profile {
            points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        },
    };
    let extrusion = |id, definition_id, profile_id| CanonicalCommand::CreateFeature {
        id: FeatureId(id),
        definition_id: DefinitionId(definition_id),
        name: format!("Panel {definition_id} extrusion"),
        kind: FeatureKind::Extrusion {
            profile: FeatureId(profile_id),
            height: Dimension::new("10", 10.0).unwrap(),
        },
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Rear panel".into(),
            },
            profile(1, 1),
            extrusion(2, 1, 1),
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Rear panel".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Top panel".into(),
            },
            profile(3, 2),
            extrusion(4, 2, 3),
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(2),
                name: "Top panel".into(),
                transform: Transform::from_translation(0.0, 0.0, 10.0 + signed_gap_mm).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let part = |name: &str, occurrence_id, profile_id, extrusion_id| RecipePartAdoption {
        key: recipe_key(name),
        instance_path: InstancePath::root(OccurrenceId(occurrence_id)),
        mobility: RecipePartMobility::Fixed,
        edit_scope: RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(occurrence_id))),
        parameters: BTreeMap::new(),
        features: vec![
            (
                recipe_key(&format!("{name}-profile")),
                FeatureId(profile_id),
                RecognizedRecipeFeatureKind::Profile,
            ),
            (
                recipe_key(&format!("{name}-extrusion")),
                FeatureId(extrusion_id),
                RecognizedRecipeFeatureKind::Extrusion,
            ),
        ],
    };
    let recipe = AssemblyRecipe::adopt(
        &snapshot,
        recipe_key("contact-fixture"),
        vec![part("rear", 1, 1, 2), part("top", 2, 3, 4)],
        vec![RecipeRelation {
            key: recipe_key("rear-top-contact"),
            kind: RecipeRelationKind::Contact,
            first: RecipeFaceRef {
                part: recipe_key("rear"),
                role: "extrusion.top".into(),
            },
            second: RecipeFaceRef {
                part: recipe_key("top"),
                role: "extrusion.bottom".into(),
            },
        }],
        Vec::new(),
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    document
}

#[test]
fn declared_recipe_contact_reports_area_normals_gap_and_penetration() {
    let touching = contact_recipe_document(0.0);
    let passed = assistant_assembly_constraints_report(&touching.current(), true, true);
    assert_eq!(passed["state"], "passed", "{passed:#}");
    assert_eq!(passed["complete"], true, "{passed:#}");
    assert_eq!(passed["contacts"][0]["normal_dot"], -1.0);
    assert_eq!(passed["contacts"][0]["signed_gap_mm"], 0.0);
    assert_eq!(passed["contacts"][0]["overlap_area_mm2"], 100.0);
    assert_eq!(passed["contacts"][0]["normals_opposed"], true);

    let separated = contact_recipe_document(2.0);
    let gap = assistant_assembly_constraints_report(&separated.current(), true, true);
    assert_eq!(gap["state"], "failed", "{gap:#}");
    assert_eq!(gap["complete"], true, "{gap:#}");
    assert_eq!(gap["contacts"][0]["gap_mm"], 2.0);
    assert_eq!(gap["contacts"][0]["penetration_mm"], 0.0);
    assert_eq!(gap["issues"][0]["code"], "assembly.contact_gap");
    let integrated = assistant_validation_context(
        &separated.current(),
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::only(&["assembly_retention"]),
    );
    assert_eq!(integrated["state"], "failed", "{integrated:#}");
    assert_eq!(
        integrated["assembly_constraints"]["issues"][0]["code"],
        "assembly.contact_gap"
    );
    assert_eq!(integrated["issue_count"], 1);

    let intersecting = contact_recipe_document(-2.0);
    let penetration = assistant_assembly_constraints_report(&intersecting.current(), true, true);
    assert_eq!(penetration["state"], "failed", "{penetration:#}");
    assert_eq!(penetration["contacts"][0]["gap_mm"], 0.0);
    assert_eq!(penetration["contacts"][0]["penetration_mm"], 2.0);
    assert_eq!(
        penetration["issues"][0]["code"],
        "assembly.contact_penetration"
    );
}

#[test]
fn contact_tolerance_is_checked_on_both_sides_without_hiding_incomplete_coverage() {
    // Pin the public validator's current tolerance, independently of its implementation.
    // The reference fixture permits 1e-6 mm; the validator is stricter at 1e-7 mm.
    for signed_gap in [
        -2.0, -1.0e-6, -1.01e-7, -0.99e-7, 0.0, 0.99e-7, 1.01e-7, 1.0e-6, 2.0,
    ] {
        let document = contact_recipe_document(signed_gap);
        let snapshot = document.current();
        let expected_pass = signed_gap.abs() < 1.0e-7;
        let report = assistant_assembly_constraints_report(&snapshot, true, true);
        assert_eq!(
            report["state"],
            if expected_pass { "passed" } else { "failed" },
            "gap={signed_gap}: {report:#}"
        );
        assert_eq!(report["complete"], true);
        assert_eq!(report["applicable_count"], 1);
        assert_eq!(report["unknown_count"], 0);
        assert_eq!(report["contacts"].as_array().unwrap().len(), 1);
        let contact = &report["contacts"][0];
        assert_eq!(contact["tolerance_mm"], 1.0e-7);
        assert!((contact["signed_gap_mm"].as_f64().unwrap() - signed_gap).abs() < 1.0e-12);
        assert!((contact["gap_mm"].as_f64().unwrap() - signed_gap.max(0.0)).abs() < 1.0e-12);
        assert!(
            (contact["penetration_mm"].as_f64().unwrap() - (-signed_gap).max(0.0)).abs() < 1.0e-12
        );
        assert_eq!(contact["normal_dot"], -1.0);
        assert_eq!(contact["overlap_area_mm2"], 100.0);
        assert_eq!(report["issue_count"], usize::from(!expected_pass));
        if !expected_pass {
            assert_eq!(
                report["issues"][0]["code"],
                if signed_gap > 0.0 {
                    "assembly.contact_gap"
                } else {
                    "assembly.contact_penetration"
                }
            );
        }
        let incomplete = assistant_assembly_constraints_report(&snapshot, true, false);
        assert_eq!(incomplete["state"], "not_evaluated", "{incomplete:#}");
        assert_eq!(incomplete["complete"], false);
        assert_eq!(incomplete["contacts"], report["contacts"]);
        assert_eq!(incomplete["issues"], report["issues"]);
        assert_eq!(incomplete["canonical_digest"], snapshot.canonical_digest());
    }
}

#[test]
fn unsupported_recipe_face_is_not_reported_as_a_passed_contact() {
    let mut document = contact_recipe_document(0.0);
    let snapshot = document.current();
    let source = snapshot.assembly_recipe().unwrap();
    let mut relation = source.relations().next().unwrap().clone();
    relation.first.role = "unrecognized.curved-face".into();
    let recipe = AssemblyRecipe::new(
        source.schema(),
        source.key().clone(),
        source
            .parts()
            .map(|part| (part.key.clone(), part.clone()))
            .collect(),
        [(relation.key.clone(), relation)].into(),
        source
            .joinery()
            .map(|joint| (joint.key.clone(), joint.clone()))
            .collect(),
        source
            .owned_features()
            .map(|feature| (feature.key.clone(), feature.clone()))
            .collect(),
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    let report = assistant_assembly_constraints_report(&document.current(), true, true);
    assert_eq!(report["state"], "not_evaluated", "{report:#}");
    assert_eq!(report["complete"], false);
    assert_eq!(report["unknown_count"], 1);
    assert_eq!(report["applicable_count"], 1);
    assert!(report["contacts"].as_array().unwrap().is_empty());
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "contact_face_unsupported_or_unavailable"
    );
    assert_eq!(report["not_evaluated"][0]["relation"], "rear-top-contact");
    assert_eq!(report["revision"], document.current().revision_id());
    assert_eq!(
        report["canonical_digest"],
        document.current().canonical_digest()
    );
}

#[test]
fn assembly_retention_finds_an_unjoined_back_panel_and_passes_after_a_fixed_connection() {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Cabinet panel".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(1),
            definition_id: DefinitionId(1),
            name: "Panel profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(2),
            definition_id: DefinitionId(1),
            name: "Panel solid".into(),
            kind: FeatureKind::Extrusion {
                profile: FeatureId(1),
                height: Dimension::new("10", 10.0).unwrap(),
            },
        },
        CanonicalCommand::UpsertClassificationDimension {
            id: ClassificationDimensionId(7),
            name: "ketchup.assembly-retention-role.v1".into(),
            categories: vec![(
                ClassificationCategoryId(9),
                "part:nightstand-carcass".into(),
            )],
        },
    ];
    for (id, name, x) in [
        (1, "Left side", 0.0),
        (2, "Top", 20.0),
        (3, "Back panel", 40.0),
    ] {
        commands.extend([
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(1),
                name: name.into(),
                transform: Transform::from_translation(x, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(id),
                dimension_id: ClassificationDimensionId(7),
                category_id: Some(ClassificationCategoryId(9)),
            },
        ]);
    }
    commands.extend([
        CanonicalCommand::SetOccurrenceGrounded {
            id: OccurrenceId(1),
            grounded: true,
        },
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(1),
            OccurrenceId(1),
            OccurrenceId(2),
            AssemblyJointKind::Fixed,
        )),
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let failed = assistant_assembly_retention_report(&document.current(), true, true);
    assert_eq!(failed["state"], "failed", "{failed:#}");
    assert_eq!(failed["complete"], true, "{failed:#}");
    assert_eq!(failed["issue_count"], 1, "{failed:#}");
    assert_eq!(failed["issues"][0]["occurrence_id"], 3, "{failed:#}");
    assert_eq!(failed["issues"][0]["name"], "Back panel", "{failed:#}");
    assert_eq!(
        failed["issues"][0]["free_translation_directions"],
        serde_json::json!(["+X", "-X", "+Y", "-Y", "+Z", "-Z"])
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                AssemblyJointId(2),
                OccurrenceId(2),
                OccurrenceId(3),
                AssemblyJointKind::Fixed,
            )),
        ]))
        .unwrap();
    let passed = assistant_validation_context(
        &document.current(),
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::only(&["assembly_retention"]),
    );
    assert_eq!(passed["state"], "passed", "{passed:#}");
    assert_eq!(passed["complete"], true, "{passed:#}");
    assert_eq!(
        passed["assembly_retention"]["schema"],
        "ketchup.assembly-retention.v2"
    );
    assert_eq!(
        passed["assembly_retention"]["revision"],
        document.current().revision_id()
    );
    assert_eq!(
        passed["assembly_retention"]["canonical_digest"],
        document.current().canonical_digest()
    );
    assert_eq!(
        passed["assembly_retention"]["scope"]["retention_base_occurrence_ids"],
        serde_json::json!([1])
    );
    assert_eq!(passed["assembly_retention"]["issue_count"], 0);
    assert_eq!(
        passed["assembly_retention"]["evaluations"][2]["translation_probe"]["+Y"],
        "retained"
    );
    assert_eq!(
        passed["assembly_retention"]["evaluations"][2]["rotation_probe"]["-Z"],
        "retained"
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: false,
            },
        ]))
        .unwrap();
    let missing_base = assistant_assembly_retention_report(&document.current(), true, true);
    assert_eq!(missing_base["state"], "not_evaluated", "{missing_base:#}");
    assert_eq!(missing_base["complete"], false, "{missing_base:#}");
    assert_eq!(
        missing_base["not_evaluated"][0]["reason"],
        "retention_base_not_declared"
    );
}

#[test]
fn entire_connected_subassembly_still_fails_when_detached_from_the_base() {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Retention participant".into(),
        },
        CanonicalCommand::UpsertClassificationDimension {
            id: ClassificationDimensionId(10),
            name: "ketchup.assembly-retention-role.v1".into(),
            categories: vec![(ClassificationCategoryId(12), "part:detached-unit".into())],
        },
    ];
    for id in 1..=4 {
        commands.extend([
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(1),
                name: format!("Part {id}"),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(id),
                dimension_id: ClassificationDimensionId(10),
                category_id: Some(ClassificationCategoryId(12)),
            },
        ]);
    }
    commands.extend([
        CanonicalCommand::SetOccurrenceGrounded {
            id: OccurrenceId(1),
            grounded: true,
        },
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(4),
            OccurrenceId(1),
            OccurrenceId(2),
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(5),
            OccurrenceId(3),
            OccurrenceId(4),
            AssemblyJointKind::Fixed,
        )),
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let report = assistant_assembly_retention_report(&document.current(), true, true);
    assert_eq!(report["state"], "failed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["issue_count"], 2, "{report:#}");
    assert_eq!(report["issues"][0]["occurrence_id"], 3);
    assert_eq!(report["issues"][1]["occurrence_id"], 4);
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .all(|issue| issue["code"] == "assembly.part_disconnected")
    );
}

#[test]
fn all_ungrounded_parts_apart_from_the_base_are_reported_individually() {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Separated panel".into(),
        },
        CanonicalCommand::UpsertClassificationDimension {
            id: ClassificationDimensionId(11),
            name: "ketchup.assembly-retention-role.v1".into(),
            categories: vec![(ClassificationCategoryId(13), "part:separated-unit".into())],
        },
    ];
    for (id, x) in [(1, 0.0), (2, 1_000.0), (3, 2_000.0)] {
        commands.extend([
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(1),
                name: format!("Separated part {id}"),
                transform: Transform::from_translation(x, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(id),
                dimension_id: ClassificationDimensionId(11),
                category_id: Some(ClassificationCategoryId(13)),
            },
        ]);
    }
    commands.push(CanonicalCommand::SetOccurrenceGrounded {
        id: OccurrenceId(1),
        grounded: true,
    });
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let report = assistant_assembly_retention_report(&document.current(), true, true);
    assert_eq!(report["state"], "failed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["issue_count"], 2, "{report:#}");
    assert_eq!(report["unknown_count"], 0, "{report:#}");
    assert_eq!(report["issues"][0]["occurrence_id"], 2);
    assert_eq!(report["issues"][1]["occurrence_id"], 3);
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .all(|issue| issue["code"] == "assembly.part_disconnected")
    );
}

fn physical_dowel_document(count: u32, duplicate_joint: bool) -> DocumentStore {
    let mut document = DocumentStore::new();
    let panel = |name: &str, translation_mm| AssistantCadEditOperation::CreatePanel {
        name: name.into(),
        dimensions_mm: [100.0, 50.0, 18.0],
        holes: Vec::new(),
        pockets: Vec::new(),
        translation_mm,
        rotation: None,
    };
    let panels = plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &AssistantCadEditProgram {
            operations: vec![
                panel("Retention base", [0.0, 0.0, 0.0]),
                panel("Dowel-connected panel", [0.0, 0.0, 18.0]),
            ],
        },
    )
    .unwrap();
    document.apply_batch(&panels).unwrap();
    let face = |occurrence_id, face_z, inward_unit_local| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        },
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let joint = |name: &str| AssistantCadEditOperation::CreatePhysicalDowelJoint {
        joint_id: None,
        name: name.into(),
        first: face(1, 18.0, [0.0, 0.0, -1.0]),
        second: face(2, 0.0, [0.0, 0.0, 1.0]),
        first_center_local_mm: [20.0, 20.0, 18.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count,
        spacing_mm: if count > 1 { 40.0 } else { 0.0 },
        dowel: AssistantStandardDowel::D8x30,
        first_insertion_mm: None,
    };
    let dowels = plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &AssistantCadEditProgram {
            operations: vec![joint("Smooth dowel row")],
        },
    )
    .unwrap();
    document.apply_batch(&dowels).unwrap();
    if duplicate_joint {
        let mut duplicate = document
            .current()
            .dowel_joint(DowelJointId(1))
            .unwrap()
            .clone();
        duplicate.id = DowelJointId(2);
        duplicate.name = "Duplicate smooth dowel row".into();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::UpsertDowelJoint(duplicate),
            ]))
            .unwrap();
    }
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(8),
                name: "ketchup.assembly-retention-role.v1".into(),
                categories: vec![(ClassificationCategoryId(10), "part:smooth-dowels".into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(8),
                category_id: Some(ClassificationCategoryId(10)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(2),
                dimension_id: ClassificationDimensionId(8),
                category_id: Some(ClassificationCategoryId(10)),
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    document
}

#[test]
fn verified_smooth_dowels_are_connected_but_do_not_claim_axial_retention() {
    let document = physical_dowel_document(2, false);

    let report = assistant_assembly_retention_report(&document.current(), true, true);
    assert_eq!(report["state"], "not_evaluated", "{report:#}");
    assert_eq!(report["complete"], false, "{report:#}");
    assert_eq!(report["issue_count"], 0, "{report:#}");
    assert_eq!(report["unknown_count"], 1, "{report:#}");
    assert_eq!(report["accepted_connections"][0]["connected"], true);
    assert_eq!(report["accepted_connections"][0]["retained"], false);
    assert_eq!(
        report["accepted_connections"][0]["unproven_degrees_of_freedom"],
        serde_json::json!(["translation_along_dowel_axis"])
    );
    assert_eq!(report["evaluations"][1]["result"], "unknown");
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "connected_but_retention_model_incomplete"
    );

    let constraints = assistant_assembly_constraints_report(&document.current(), true, true);
    assert_eq!(constraints["state"], "passed", "{constraints:#}");
    assert_eq!(constraints["complete"], true, "{constraints:#}");
    assert_eq!(constraints["schema"], "ketchup.assembly-constraints.v1");
    assert_eq!(
        constraints["document_id"],
        document.current().document_id().0
    );
    assert_eq!(constraints["revision"], document.current().revision_id());
    assert_eq!(
        constraints["canonical_digest"],
        document.current().canonical_digest()
    );
    assert_eq!(constraints["assumptions"].as_array().unwrap().len(), 4);
    assert_eq!(constraints["scope"]["physical_dowel_pair_count"], 2);
    assert_eq!(constraints["full_probe_overlaps"]["expected_count"], 2);
    assert_eq!(
        constraints["full_probe_overlaps"]["verified_expected_count"],
        2
    );
    assert_eq!(constraints["full_probe_overlaps"]["unexpected_count"], 0);
    let first_pair = &constraints["physical_dowel_joints"][0]["pairs"][0];
    assert_eq!(first_pair["axis_dot"], -1.0);
    assert_eq!(first_pair["first"]["diameter_mm"], 8.0);
    assert_eq!(first_pair["first"]["depth_mm"], 16.0);
    assert_eq!(first_pair["second"]["depth_mm"], 16.0);
    assert_eq!(first_pair["first"]["minimum_edge_distance_mm"], 16.0);
    assert_eq!(
        first_pair["physical_probe_coincidence"]["full_length_coincident"],
        true
    );
    assert_eq!(
        first_pair["physical_probe_coincidence"]["first_probe_endpoints_world_mm"],
        first_pair["physical_probe_coincidence"]["second_probe_endpoints_world_mm"]
    );
}

#[test]
fn physical_dowel_validation_is_invariant_under_global_rigid_motion() {
    let baseline = physical_dowel_document(2, false);
    let baseline_report = assistant_assembly_constraints_report(&baseline.current(), true, true);
    assert_eq!(baseline_report["state"], "passed");
    let angle = 37.0_f64.to_radians();
    let (sin, cos) = angle.sin_cos();
    for (rotation, translation) in [
        (
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [1200.0, -340.0, 57.0],
        ),
        (
            [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            [-71.0, 400.0, 29.0],
        ),
        (
            [[1.0, 0.0, 0.0], [0.0, cos, -sin], [0.0, sin, cos]],
            [133.0, -85.0, 212.0],
        ),
    ] {
        let mut document = physical_dowel_document(2, false);
        let global = Transform::from_matrix([
            rotation[0][0],
            rotation[0][1],
            rotation[0][2],
            translation[0],
            rotation[1][0],
            rotation[1][1],
            rotation[1][2],
            translation[1],
            rotation[2][0],
            rotation[2][1],
            rotation[2][2],
            translation[2],
            0.0,
            0.0,
            0.0,
            1.0,
        ])
        .unwrap();
        let snapshot = document.current();
        let commands = snapshot
            .occurrences()
            .map(|occurrence| {
                assert!(occurrence.parent().is_none());
                CanonicalCommand::SetOccurrenceTransform {
                    id: occurrence.id(),
                    transform: global.compose(occurrence.transform()),
                }
            })
            .collect();
        document.apply_batch(&CommandBatch::new(commands)).unwrap();
        let moved = document.current();
        let report = assistant_assembly_constraints_report(&moved, true, true);
        assert_eq!(report["state"], "passed", "{report:#}");
        assert_eq!(report["complete"], true);
        assert_eq!(report["canonical_digest"], moved.canonical_digest());
        assert_eq!(report["scope"]["physical_dowel_pair_count"], 2);
        assert_eq!(report["full_probe_overlaps"]["verified_expected_count"], 2);
        assert_eq!(report["full_probe_overlaps"]["unexpected_count"], 0);
        for index in 0..2 {
            let pair = &report["physical_dowel_joints"][0]["pairs"][index];
            assert!((pair["axis_dot"].as_f64().unwrap() + 1.0).abs() < 1e-9);
            assert_eq!(pair["first"]["diameter_mm"], 8.0);
            assert_eq!(pair["first"]["depth_mm"], 16.0);
            assert_eq!(pair["second"]["depth_mm"], 16.0);
            assert_eq!(
                pair["physical_probe_coincidence"]["full_length_coincident"],
                true
            );
            let original = &baseline_report["physical_dowel_joints"][0]["pairs"][index]["physical_probe_coincidence"]
                ["first_probe_endpoints_world_mm"];
            for endpoint in 0..2 {
                for axis in 0..3 {
                    // Independent matrix arithmetic, not recipe/compiler contact calculations.
                    let expected = translation[axis]
                        + (0..3)
                            .map(|column| {
                                rotation[axis][column]
                                    * original[endpoint][column].as_f64().unwrap()
                            })
                            .sum::<f64>();
                    for side in [
                        "first_probe_endpoints_world_mm",
                        "second_probe_endpoints_world_mm",
                    ] {
                        let actual = pair["physical_probe_coincidence"][side][endpoint][axis]
                            .as_f64()
                            .unwrap();
                        assert!(
                            (actual - expected).abs() < 1e-7,
                            "{side}: {actual} != {expected}"
                        );
                    }
                }
            }
        }
        let retention = assistant_assembly_retention_report(&moved, true, true);
        assert_eq!(retention["state"], "not_evaluated");
        assert_eq!(retention["complete"], false);
        assert_eq!(retention["accepted_connections"][0]["connected"], true);
        assert_eq!(retention["accepted_connections"][0]["retained"], false);
        assert_eq!(
            retention["accepted_connections"][0]["unproven_degrees_of_freedom"],
            serde_json::json!(["translation_along_dowel_axis"])
        );
    }
}

#[test]
fn one_smooth_dowel_is_connected_but_leaves_axial_translation_and_rotation_unproven() {
    let document = physical_dowel_document(1, false);
    let retention = assistant_assembly_retention_report(&document.current(), true, true);
    assert_eq!(retention["state"], "not_evaluated", "{retention:#}");
    assert_eq!(retention["complete"], false, "{retention:#}");
    assert_eq!(retention["issue_count"], 0, "{retention:#}");
    assert_eq!(retention["unknown_count"], 1, "{retention:#}");
    assert_eq!(retention["accepted_connections"][0]["connected"], true);
    assert_eq!(retention["accepted_connections"][0]["retained"], false);
    assert_eq!(
        retention["accepted_connections"][0]["unproven_degrees_of_freedom"],
        serde_json::json!(["translation_along_dowel_axis", "rotation_about_dowel_axis"])
    );
    assert_eq!(
        retention["accepted_connections"][0]["reason"],
        "one_smooth_dowel_does_not_prevent_axial_pullout_or_rotation_about_its_axis"
    );
    assert_eq!(retention["evaluations"][1]["connected_to_base"], true);
    assert_eq!(retention["evaluations"][1]["result"], "unknown");

    let constraints = assistant_assembly_constraints_report(&document.current(), true, true);
    assert_eq!(constraints["state"], "passed", "{constraints:#}");
    assert_eq!(constraints["complete"], true, "{constraints:#}");
    assert_eq!(constraints["full_probe_overlaps"]["expected_count"], 1);
    assert_eq!(
        constraints["full_probe_overlaps"]["verified_expected_count"],
        1
    );
    assert_eq!(constraints["full_probe_overlaps"]["unexpected_count"], 0);
}

#[test]
fn duplicate_cross_joint_probe_overlap_is_not_treated_as_a_dowel_exception() {
    let document = physical_dowel_document(1, true);
    let report = assistant_assembly_constraints_report(&document.current(), true, true);
    assert_eq!(report["state"], "failed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["issue_count"], 1, "{report:#}");
    assert_eq!(report["full_probe_overlaps"]["expected_count"], 2);
    assert_eq!(report["full_probe_overlaps"]["verified_expected_count"], 2);
    assert_eq!(report["full_probe_overlaps"]["unexpected_count"], 2);
    assert_eq!(
        report["issues"][0]["code"],
        "assembly.unexpected_physical_probe_overlap"
    );
    assert_eq!(report["issues"][0]["unexpected_complete_overlap_count"], 2);
    assert_ne!(
        report["issues"][0]["overlaps"][0]["first_joint_id"],
        report["issues"][0]["overlaps"][0]["second_joint_id"]
    );
}

#[test]
fn declared_prismatic_drawer_is_connected_and_allowed_to_move() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Drawer assembly".into(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Cabinet base".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(1),
                name: "Movable drawer".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(9),
                name: "ketchup.assembly-retention-role.v1".into(),
                categories: vec![(ClassificationCategoryId(11), "part:drawer-unit".into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(9),
                category_id: Some(ClassificationCategoryId(11)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(2),
                dimension_id: ClassificationDimensionId(9),
                category_id: Some(ClassificationCategoryId(11)),
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                AssemblyJointId(3),
                OccurrenceId(1),
                OccurrenceId(2),
                AssemblyJointKind::Prismatic {
                    axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                    limits: Some(AssemblyJointLimits::new(0.0, 400.0)),
                    position_mm: 0.0,
                },
            )),
        ]))
        .unwrap();

    let report = assistant_assembly_retention_report(&document.current(), true, true);
    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["issue_count"], 0, "{report:#}");
    assert_eq!(report["unknown_count"], 0, "{report:#}");
    assert_eq!(report["evaluations"][1]["declared_mobility"], "movable");
    assert_eq!(report["evaluations"][1]["result"], "movable_as_declared");
    assert_eq!(
        report["accepted_connections"][0]["allowed_degrees_of_freedom"],
        serde_json::json!(["translation_along_declared_axis"])
    );
}

#[test]
fn structural_scope_is_deterministic_and_stale_after_mutation() {
    let mut document = DocumentStore::new();
    let scope = StructuralValidationScope::bind(
        &document.current(),
        [OccurrenceId(2), OccurrenceId(1), OccurrenceId(2)],
    );
    assert!(scope.is_current(&document.current()));
    assert_eq!(
        scope.occurrence_ids(),
        &BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );

    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: TagId(1),
            name: "revision change".into(),
            visible: true,
        }]))
        .unwrap();
    assert!(!scope.is_current(&document.current()));
    let report = scoped_static_load_report(&document.current(), &scope, || false);
    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "stale_structural_scope"
    );
}

#[test]
fn structural_scope_binding_stops_at_resource_limit() {
    let document = DocumentStore::new();
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, (1..=10_001).map(OccurrenceId));

    assert_eq!(scope.occurrence_ids().len(), 10_000);
    let report = scoped_static_load_report(&snapshot, &scope, || false);
    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "structural_scope_resource_limit"
    );
}

#[test]
fn scoped_static_load_includes_boundary_support_without_exact_geometry() {
    let document = structural_document(2);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["applicable_count"], 1);
    assert_eq!(report["evaluations"][0]["occurrence_id"], 1);
    assert_eq!(report["evaluations"][0]["supports"][0]["occurrence_id"], 2);
    assert_eq!(
        report["coverage"]["boundary_support_occurrence_ids"],
        serde_json::json!([2])
    );
    assert_eq!(report["coverage"]["exact_geometry_loaded_count"], 0);
}

#[test]
fn scoped_static_load_cancellation_is_incomplete() {
    let document = structural_document(2);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let mut polls = 0;
    let report = scoped_static_load_report(&snapshot, &scope, || {
        polls += 1;
        polls > 11
    });

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(report["not_evaluated"][0]["reason"], "validation_cancelled");
    assert_eq!(report["coverage"]["checked_load_occurrence_count"], 0);
}

#[test]
fn scoped_static_load_missing_role_is_incomplete() {
    let document = structural_document(3);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(3)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "missing_or_invalid_static_load_role"
    );
}

#[test]
fn scoped_static_load_missing_input_is_incomplete() {
    let mut document = structural_document(2);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameEvaluatorNode {
                id: NodeId(4),
                name: "unrelated parameter".into(),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated", "{report:#}");
    assert_eq!(report["complete"], false, "{report:#}");
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "missing_or_ambiguous_mass"
    );
}

#[test]
fn global_static_load_reports_every_role_and_load_declaration_as_incomplete() {
    let mut document = structural_document(6);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(3),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(4),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(7),
                name: "physics.applied_load_n.occurrence.3".into(),
                dimension: Dimension::new("200", 200.0).unwrap(),
                dependencies: Vec::new(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(8),
                name: "physics.applied_load_n.occurrence.5".into(),
                dimension: Dimension::new("200", 200.0).unwrap(),
                dependencies: Vec::new(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(9),
                name: "physics.mass_kg.occurrence.6".into(),
                dimension: Dimension::new("100", 100.0).unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let names = snapshot
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let roles = ValidatorRoleIndex::from_snapshot(&snapshot);

    let report = assistant_static_load_report(&snapshot, &names, &roles, true, true);

    assert_eq!(report["state"], "not_evaluated", "{report:#}");
    assert_eq!(report["complete"], false, "{report:#}");
    assert_eq!(report["applicable_count"], 0, "{report:#}");
    assert_eq!(report["not_evaluated"].as_array().unwrap().len(), 5);
    for (index, occurrence_id, reason) in [
        (0, 3, "missing_or_ambiguous_mass"),
        (1, 4, "missing_or_ambiguous_mass"),
        (2, 5, "missing_or_invalid_static_load_role"),
        (3, 6, "missing_or_invalid_static_load_role"),
        (4, 1, "incomplete_static_load_case"),
    ] {
        assert_eq!(
            report["not_evaluated"][index]["occurrence_id"], occurrence_id,
            "{report:#}"
        );
        assert_eq!(
            report["not_evaluated"][index]["reason"], reason,
            "{report:#}"
        );
    }
}

#[test]
fn shared_support_capacity_aggregates_by_load_case_with_scoped_global_parity() {
    let mut document = structural_document(3);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(2),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(3),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(2)),
            },
            CanonicalCommand::RenameEvaluatorNode {
                id: NodeId(6),
                name: "physics.support_capacity_n.occurrence.3".into(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(7),
                name: "physics.mass_kg.occurrence.2".into(),
                dimension: Dimension::new("100", 100.0).unwrap(),
                dependencies: Vec::new(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(8),
                name: "physics.applied_load_n.occurrence.2".into(),
                dimension: Dimension::new("200", 200.0).unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1), OccurrenceId(2)]);
    let report = scoped_static_load_report(&snapshot, &scope, || false);
    let names = snapshot
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let roles = ValidatorRoleIndex::from_snapshot(&snapshot);
    let global_report = assistant_static_load_report(&snapshot, &names, &roles, true, true);

    for aggregate in [&report, &global_report] {
        assert_eq!(aggregate["state"], "failed", "{aggregate:#}");
        assert_eq!(aggregate["complete"], true, "{aggregate:#}");
        assert_eq!(aggregate["applicable_count"], 2);
        assert_eq!(aggregate["issue_count"], 1);
        assert_eq!(
            aggregate["evaluations"][0]["case_load_occurrence_ids"],
            serde_json::json!([1, 2])
        );
        assert_eq!(aggregate["evaluations"][0]["resultant_force_n"], 1_181.0);
        assert_eq!(
            aggregate["evaluations"][0]["case_resultant_force_n"],
            2_362.0
        );
        assert_eq!(
            aggregate["evaluations"][0]["total_support_capacity_n"],
            2_000.0
        );
        assert_eq!(aggregate["evaluations"][0]["capacity_margin_n"], -362.0);
        assert_eq!(
            aggregate["issues"][0]["load_occurrence_ids"],
            serde_json::json!([1, 2])
        );
        assert_eq!(aggregate["issues"][0]["capacity_shortfall_n"], 362.0);
    }
    assert_eq!(report["evaluations"], global_report["evaluations"]);
    assert_eq!(report["issues"], global_report["issues"]);

    let isolated_scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);
    let isolated_report = scoped_static_load_report(&snapshot, &isolated_scope, || false);
    assert_eq!(isolated_report["state"], "not_evaluated");
    assert_eq!(isolated_report["complete"], false);
    assert_eq!(
        isolated_report["not_evaluated"][0]["reason"],
        "shared_static_load_case_outside_scope"
    );
}

#[test]
fn furniture_validators_use_nonuniform_occurrence_scale_and_reject_shear() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Shelf".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Shelf profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [500.0, 0.0], [500.0, 300.0], [0.0, 300.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Shelf solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("20", 20.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Scaled shelf".into(),
                transform: Transform::from_matrix([
                    2.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ])
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Case".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(2),
                name: "Case profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [400.0, 0.0], [400.0, 400.0], [0.0, 400.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(2),
                name: "Case solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(3),
                    height: Dimension::new("800", 800.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(2),
                name: "Scaled case".into(),
                transform: Transform::from_matrix([
                    0.5, 0.0, 0.0, 2_000.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0,
                    1.0,
                ])
                .unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(1),
                name: "ketchup.validator-role.v1".into(),
                categories: vec![
                    (ClassificationCategoryId(1), "furniture.shelf.xy".into()),
                    (ClassificationCategoryId(2), "furniture.case.z".into()),
                ],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(2),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(2)),
            },
        ]))
        .unwrap();
    let selection =
        AssistantValidationSelection::only(&["shelf_deflection", "tipping", "anchoring"]);
    let exact_results = ExactResultRegistry::default();
    let snapshot = document.current();
    let report = assistant_validation_context(&snapshot, &exact_results, &selection);

    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["state"], "failed", "{report:#}");
    let shelf = &report["shelf_deflection"];
    assert_eq!(shelf["state"], "failed", "{shelf:#}");
    assert_eq!(shelf["evaluations"][0]["span_mm"], 1_000.0);
    assert_eq!(shelf["evaluations"][0]["depth_mm"], 300.0);
    assert_eq!(shelf["evaluations"][0]["thickness_mm"], 20.0);
    assert_eq!(shelf["evaluations"][0]["result"], "failed");
    let tipping = &report["tipping"];
    assert_eq!(tipping["state"], "failed", "{tipping:#}");
    assert_eq!(tipping["evaluations"][0]["base_depth_mm"], 200.0);
    assert_eq!(tipping["evaluations"][0]["height_mm"], 1_600.0);
    assert_eq!(tipping["evaluations"][0]["result"], "failed");
    let anchoring = &report["anchoring"];
    assert_eq!(anchoring["state"], "failed", "{anchoring:#}");
    assert_eq!(anchoring["evaluations"][0]["height_depth_ratio"], 8.0);
    assert_eq!(anchoring["evaluations"][0]["anchoring_required"], true);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_matrix([
                    1.0, 0.25, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ])
                .unwrap(),
            },
        ]))
        .unwrap();
    let sheared = assistant_validation_context(&document.current(), &exact_results, &selection);
    assert_eq!(sheared["shelf_deflection"]["state"], "not_evaluated");
    assert_eq!(sheared["shelf_deflection"]["complete"], false);
    assert_eq!(
        sheared["shelf_deflection"]["not_evaluated"][0]["reason"],
        "occurrence transform shears the declared source frame"
    );
}

#[test]
fn gravity_support_does_not_propagate_through_an_unproven_envelope_contact() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Boolean support".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Outer profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Outer solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "Opening profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[20.0, 20.0], [80.0, 20.0], [80.0, 80.0], [20.0, 80.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Opening solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(3),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DefinitionId(1),
                name: "Cut support".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    target: FeatureId(2),
                    tool: FeatureId(4),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Ground support".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: "Load".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(6),
                definition_id: DefinitionId(2),
                name: "Load profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(7),
                definition_id: DefinitionId(2),
                name: "Load solid".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(6),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(2),
                definition_id: DefinitionId(2),
                name: "Load over opening".into(),
                transform: Transform::from_translation(40.0, 40.0, 10.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(3),
                definition_id: DefinitionId(2),
                name: "Transitive load".into(),
                transform: Transform::from_translation(40.0, 40.0, 20.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let report = assistant_validation_context(
        &snapshot,
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::only(&["gravity_support"]),
    );

    assert_eq!(report["state"], "not_evaluated", "{report:#}");
    assert_eq!(report["complete"], false, "{report:#}");
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "exact_gravity_contact_unavailable"
    );
    assert_eq!(report["gravity_support"]["state"], "not_evaluated");
    assert_eq!(report["gravity_support"]["complete"], false);
    assert_eq!(report["gravity_support"]["unsupported_count"], 0);
    assert_eq!(report["gravity_support"]["unproven_contact_count"], 2);
    assert_eq!(
        report["gravity_support"]["issues"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        report["gravity_support"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .all(|issue| issue["code"] == "gravity.contact-unproven")
    );
}

#[test]
fn scoped_static_load_rejects_more_loads_than_can_be_reported() {
    let document = structural_document(101);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, (1..=101).map(OccurrenceId));

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "structural_load_resource_limit"
    );
}

#[test]
fn scoped_static_load_rejects_over_budget_output_text() {
    let mut document = structural_document(2);
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::RenameEntity {
            id: OccurrenceId(1),
            name: "x".repeat(4 * 1024 * 1024 + 1),
        }]))
        .unwrap();
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "not_evaluated");
    assert_eq!(report["complete"], false);
    assert_eq!(
        report["not_evaluated"][0]["reason"],
        "structural_text_resource_limit"
    );
}

#[test]
fn scoped_static_load_handles_ten_thousand_sparse_occurrences() {
    let document = structural_document(10_000);
    let snapshot = document.current();
    let scope = StructuralValidationScope::bind(&snapshot, [OccurrenceId(1)]);

    let report = scoped_static_load_report(&snapshot, &scope, || false);

    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["complete"], true, "{report:#}");
    assert_eq!(report["coverage"]["requested_occurrence_count"], 1);
    assert_eq!(report["coverage"]["checked_load_occurrence_count"], 1);
    assert_eq!(report["coverage"]["exact_geometry_loaded_count"], 0);
}

#[test]
fn canonical_catalog_selections_remain_valid() {
    assert!(AssistantValidationSelection::all("all").is_valid());
    assert!(AssistantValidationSelection::only(&["gravity_support"]).is_valid());
    assert!(!AssistantValidationSelection::only(&[]).is_valid());
    assert!(!AssistantValidationSelection::only(&["bogus"]).is_valid());
}
