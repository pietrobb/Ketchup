use ketchup_application::evaluation::{
    ExactEvaluationSelection, exact_source, plan_incremental_exact_evaluation,
};
use ketchup_application::workflow_trace::{
    AssemblyWorkflowTraceRecorder, WorkflowEvidenceKind, WorkflowPhase, WorkflowRunProfile,
};
use ketchup_core::assembly_recipe::{
    AssemblyRecipe, RecipeEditScope, RecipeKey, RecipePartAdoption, RecipePartMobility,
    RecognizedRecipeFeatureKind,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    InstancePath, OccurrenceId, Transform,
};
use ketchup_core::{graph::sha256_hex, persistence};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    schema: String,
    request: ReferenceRequest,
    fixtures: Vec<FixtureEntry>,
    large_fixture: LargeFixture,
    historical_attempt: HistoricalAttempt,
}

#[derive(Debug, Deserialize)]
struct ReferenceRequest {
    target_part: TargetPart,
    dowel: DowelSpec,
    rows: Vec<RowSpec>,
    expected: ExpectedResult,
}

#[derive(Debug, Deserialize)]
struct TargetPart {
    source_dimensions_mm: [f64; 3],
    target_dimensions_mm: [f64; 3],
    required_gap_mm: f64,
    contact_tolerance_mm: f64,
}

#[derive(Debug, Deserialize)]
struct DowelSpec {
    diameter_mm: f64,
    length_mm: f64,
    hole_depth_each_part_mm: f64,
    probe_endpoint_tolerance_mm: f64,
}

#[derive(Debug, Deserialize)]
struct RowSpec {
    key: String,
    action: String,
    centers_rear_local_mm: Vec<[f64; 3]>,
}

#[derive(Debug, Deserialize)]
struct ExpectedResult {
    rear_joint_rows: usize,
    rear_dowels: usize,
    new_rear_dowels: usize,
    total_document_dowel_joints: usize,
    total_document_dowels: usize,
    replacement_part_allowed: bool,
    hidden_superseded_part_allowed: bool,
    undo_steps_added: usize,
    illegal_collisions: usize,
    maximum_sequential_tool_round_trips: usize,
    planning_budget_ms: u64,
    host_job_budget_ms: u64,
    end_to_end_budget_ms: u64,
}

#[derive(Debug, Deserialize)]
struct FixtureEntry {
    path: String,
    role: String,
    bytes: u64,
    sha256: String,
    revision: u64,
    canonical_digest: String,
    definitions: usize,
    features: usize,
    instances: usize,
    dowel_joints: usize,
    #[serde(default)]
    known_defects: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LargeFixture {
    cabinet_count: usize,
    panel_count: usize,
    generation: String,
}

#[derive(Debug, Deserialize)]
struct HistoricalAttempt {
    reported_total_minutes: u64,
    phase_trace_available: bool,
    policy: String,
}

#[derive(Serialize)]
struct FixtureBaseline {
    path: String,
    role: String,
    sha256: String,
    canonical_digest: String,
    cold_open_ms: f64,
    definitions: usize,
    features: usize,
    occurrences: usize,
    dowel_joints: usize,
}

#[derive(Serialize)]
struct BaselineReport {
    schema: &'static str,
    evidence_kind: &'static str,
    fixtures: Vec<FixtureBaseline>,
    large_fixture_cabinets: usize,
    large_fixture_panels: usize,
    historical_total_minutes_reported_by_user: u64,
    historical_phase_trace_available: bool,
    limitations: [&'static str; 3],
}

fn fixture_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fast_assembly")
}

fn manifest() -> FixtureManifest {
    serde_json::from_slice(
        &std::fs::read(fixture_directory().join("manifest.json")).expect("fixture manifest"),
    )
    .expect("valid fixture manifest")
}

#[test]
fn nightstand_inputs_are_immutable_and_the_interrupted_result_is_not_an_oracle() {
    let manifest = manifest();
    assert_eq!(manifest.schema, "ketchup.fast-assembly-fixtures.v1");
    assert_eq!(manifest.fixtures.len(), 3);

    for expected in &manifest.fixtures {
        let path = fixture_directory().join(&expected.path);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len() as u64, expected.bytes, "{}", expected.path);
        assert_eq!(sha256_hex(&bytes), expected.sha256, "{}", expected.path);

        let loaded = persistence::load_file(&path).unwrap();
        assert!(loaded.is_editable());
        let snapshot = loaded.snapshot();
        assert_eq!(
            snapshot.revision_id(),
            expected.revision,
            "{}",
            expected.path
        );
        assert_eq!(
            snapshot.canonical_digest(),
            expected.canonical_digest,
            "{}",
            expected.path
        );
        assert_eq!(snapshot.definitions().count(), expected.definitions);
        assert_eq!(snapshot.features().count(), expected.features);
        assert_eq!(snapshot.occurrences().count(), expected.instances);
        assert_eq!(snapshot.dowel_joints().count(), expected.dowel_joints);
    }

    let interrupted = manifest
        .fixtures
        .iter()
        .find(|fixture| fixture.role == "diagnostic_non_oracle")
        .unwrap();
    assert!(
        interrupted
            .known_defects
            .contains(&"replaced_the_rear_part_instead_of_editing_it".to_owned())
    );
    assert!(
        interrupted
            .known_defects
            .contains(&"missing_left_side_dowel_row".to_owned())
    );
    assert!(
        interrupted
            .known_defects
            .contains(&"missing_right_side_dowel_row".to_owned())
    );
}

#[test]
fn v9_rear_panel_adoption_preserves_the_existing_model_and_round_trips() {
    let path = fixture_directory().join("nightstand_v9_retention.ketchup");
    let mut document = persistence::load_file(path)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let before = document.current();
    let before_digest = before.canonical_digest();
    let before_undo_steps = document.visible_undo_steps();
    let feature_specs = [
        (
            "rear/body/workplane",
            94,
            RecognizedRecipeFeatureKind::Workplane,
        ),
        ("rear/body/sketch", 95, RecognizedRecipeFeatureKind::Sketch),
        ("rear/body/pad", 96, RecognizedRecipeFeatureKind::Pad),
        (
            "rear/lower-dowel-1/workplane",
            127,
            RecognizedRecipeFeatureKind::Workplane,
        ),
        (
            "rear/lower-dowel-1/sketch",
            128,
            RecognizedRecipeFeatureKind::Sketch,
        ),
        (
            "rear/lower-dowel-1/pocket",
            129,
            RecognizedRecipeFeatureKind::Pocket,
        ),
        (
            "rear/lower-dowel-2/workplane",
            130,
            RecognizedRecipeFeatureKind::Workplane,
        ),
        (
            "rear/lower-dowel-2/sketch",
            131,
            RecognizedRecipeFeatureKind::Sketch,
        ),
        (
            "rear/lower-dowel-2/pocket",
            132,
            RecognizedRecipeFeatureKind::Pocket,
        ),
        (
            "rear/lower-dowel-3/workplane",
            133,
            RecognizedRecipeFeatureKind::Workplane,
        ),
        (
            "rear/lower-dowel-3/sketch",
            134,
            RecognizedRecipeFeatureKind::Sketch,
        ),
        (
            "rear/lower-dowel-3/pocket",
            135,
            RecognizedRecipeFeatureKind::Pocket,
        ),
    ];
    let before_fingerprints = feature_specs
        .iter()
        .map(|(_, id, _)| {
            before
                .feature_canonical_fingerprint(FeatureId(*id))
                .unwrap()
        })
        .collect::<Vec<_>>();
    let recipe = AssemblyRecipe::adopt(
        &before,
        RecipeKey::new("nightstand").unwrap(),
        vec![RecipePartAdoption {
            key: RecipeKey::new("rear").unwrap(),
            instance_path: InstancePath::root(OccurrenceId(6)),
            mobility: RecipePartMobility::Fixed,
            edit_scope: RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(6))),
            parameters: BTreeMap::new(),
            features: feature_specs
                .iter()
                .map(|(key, id, kind)| (RecipeKey::new(*key).unwrap(), FeatureId(*id), *kind))
                .collect(),
        }],
        vec![],
        vec![],
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe.clone()),
        ]))
        .unwrap();

    let adopted = document.current();
    assert_ne!(adopted.canonical_digest(), before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo_steps + 1);
    assert_eq!(adopted.definitions().count(), 15);
    assert_eq!(adopted.features().count(), 144);
    assert_eq!(adopted.occurrences().count(), 53);
    assert_eq!(adopted.dowel_joints().count(), 7);
    assert_eq!(
        adopted.occurrence(OccurrenceId(6)).unwrap().definition_id(),
        DefinitionId(5)
    );
    for ((_, id, _), fingerprint) in feature_specs.iter().zip(&before_fingerprints) {
        assert_eq!(
            adopted.feature_canonical_fingerprint(FeatureId(*id)),
            Some(fingerprint.clone())
        );
    }

    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let reopened = persistence::load(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let round_tripped = reopened.current();
    assert_eq!(round_tripped.assembly_recipe(), Some(&recipe));
    assert_eq!(round_tripped.canonical_digest(), adopted.canonical_digest());
    assert_eq!(round_tripped.definitions().count(), 15);
    assert_eq!(round_tripped.features().count(), 144);
    assert_eq!(round_tripped.occurrences().count(), 53);
    assert_eq!(round_tripped.dowel_joints().count(), 7);
}

#[test]
fn original_v9_rear_sketch_resizes_without_replacing_the_panel_or_lower_holes() {
    assert_original_rear_sketch_resizes("nightstand_v9_retention.ketchup");
}

#[test]
fn original_v8_rear_sketch_resizes_without_replacing_the_panel() {
    assert_original_rear_sketch_resizes("nightstand_v8_full_probe.ketchup");
}

fn assert_original_rear_sketch_resizes(fixture: &str) {
    use ketchup_core::assembly_recipe::{
        RecipeDimensionAnchor, RecipeParameter, RecipeParameterUnit, RecipePatchNode,
        RecipeSemanticChange, RecipeSemanticPatch, compile_assembly_recipe_patch,
    };
    use ketchup_core::document::{FeatureParameterTarget, ParameterValueType};

    let mut document = persistence::load_file(fixture_directory().join(fixture))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let before = document.current();
    let key = |name: &str| RecipeKey::new(name).unwrap();
    let target =
        FeatureParameterTarget::new(FeatureId(95), "bounds.height", ParameterValueType::Length)
            .unwrap();
    assert_eq!(before.feature_parameter_value(&target), Some(216.0));
    let context = ketchup_application::model_query::ModelQuery::default()
        .edit_context(
            &before,
            &ketchup_core::exact_product::ExactResultRegistry::default(),
            0,
            &ketchup_application::model_query::EditContextRequest {
                targets: vec![ketchup_core::assistant_sidecar::AssistantInstancePath {
                    root_occurrence_id: 6,
                    steps: vec![],
                }],
            },
        )
        .unwrap();
    let parameters = context["targets"][0]["features"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|feature| feature["parameters"].as_array().unwrap())
        .collect::<Vec<_>>();
    for (path, value) in [
        ("bounds.width", 464.0),
        ("bounds.height", 216.0),
        ("extent.distance", 8.0),
    ] {
        assert!(
            parameters.iter().any(|parameter| parameter["path"] == path
                && parameter["value"] == value
                && parameter["unit"] == "mm"
                && parameter["editable"] == true),
            "missing AI edit context parameter {path}"
        );
    }
    let features = |definition_id: u64, name: &str| {
        before
            .features()
            .filter(|feature| feature.definition_id() == DefinitionId(definition_id))
            .map(|feature| {
                let kind = match feature.kind() {
                    FeatureKind::Workplane(_) => RecognizedRecipeFeatureKind::Workplane,
                    FeatureKind::Sketch(_) => RecognizedRecipeFeatureKind::Sketch,
                    FeatureKind::Pad(_) => RecognizedRecipeFeatureKind::Pad,
                    FeatureKind::Pocket { .. } => RecognizedRecipeFeatureKind::Pocket,
                    other => panic!("unexpected rear feature {other:?}"),
                };
                (
                    key(&format!("{name}/feature-{}", feature.id().0)),
                    feature.id(),
                    kind,
                )
            })
            .collect()
    };
    let recipe = AssemblyRecipe::adopt(
        &before,
        key("nightstand"),
        vec![
            RecipePartAdoption {
                key: key("rear"),
                instance_path: InstancePath::root(OccurrenceId(6)),
                mobility: RecipePartMobility::Fixed,
                edit_scope: RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(6))),
                parameters: BTreeMap::from([(
                    key("height"),
                    RecipeParameter {
                        value: 216.0,
                        unit: RecipeParameterUnit::Millimetres,
                        target: Some(target.clone()),
                    },
                )]),
                features: features(5, "rear"),
            },
            RecipePartAdoption {
                key: key("top"),
                instance_path: InstancePath::root(OccurrenceId(1)),
                mobility: RecipePartMobility::Fixed,
                edit_scope: RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
                parameters: BTreeMap::new(),
                features: features(1, "top"),
            },
        ],
        vec![ketchup_core::assembly_recipe::RecipeRelation {
            key: key("rear-top"),
            kind: ketchup_core::assembly_recipe::RecipeRelationKind::Contact,
            first: ketchup_core::assembly_recipe::RecipeFaceRef {
                part: key("rear"),
                role: "bounds.y.maximum".into(),
            },
            second: ketchup_core::assembly_recipe::RecipeFaceRef {
                part: key("top"),
                role: "bounds.z.minimum".into(),
            },
        }],
        vec![],
    )
    .unwrap();
    let adoption = CanonicalCommand::SetAssemblyRecipe(recipe);
    let adopted = before
        .preview_batch(&CommandBatch::new(vec![adoption.clone()]))
        .unwrap();
    let patch = RecipeSemanticPatch {
        nodes: vec![RecipePatchNode {
            key: key("extend-rear"),
            dependencies: vec![],
            change: RecipeSemanticChange::ExtendUntilContact {
                part: key("rear"),
                parameter: key("height"),
                relation: key("rear-top"),
                anchor: RecipeDimensionAnchor::Minimum,
            },
        }],
    };
    let compiled = compile_assembly_recipe_patch(&adopted, &patch).unwrap();
    let mut commands = vec![adoption];
    commands.extend(compiled.batch.unwrap().commands().iter().cloned());
    let undo = document.visible_undo_steps();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let after = document.current();
    assert!((after.feature_parameter_value(&target).unwrap() - 218.0).abs() < 1.0e-6);
    let FeatureKind::Sketch(sketch) = after.feature(FeatureId(95)).unwrap().kind() else {
        panic!("rear sketch replaced");
    };
    let corners = [[0.0, 0.0], [464.0, 0.0], [464.0, 218.0], [0.0, 218.0]];
    for (index, entity) in sketch.entities.iter().enumerate() {
        let ketchup_core::sketch::SketchEntity::Line {
            id,
            start_mm,
            end_mm,
        } = entity
        else {
            panic!("rear edge replaced");
        };
        assert_eq!(id.0, index as u64 + 1);
        for axis in 0..2 {
            assert!((start_mm[axis] - corners[index][axis]).abs() < 1.0e-6);
            assert!((end_mm[axis] - corners[(index + 1) % 4][axis]).abs() < 1.0e-6);
        }
    }
    let rear = after.occurrence(OccurrenceId(6)).unwrap().transform();
    let top = after.occurrence(OccurrenceId(1)).unwrap().transform();
    for x in [0.0, 464.0] {
        for z in [0.0, 8.0] {
            let height = after.feature_parameter_value(&target).unwrap();
            let world_z = rear.matrix()[8] * x
                + rear.matrix()[9] * height
                + rear.matrix()[10] * z
                + rear.matrix()[11];
            assert!((world_z - top.matrix()[11]).abs() < 1.0e-6);
        }
    }
    assert_eq!(after.features().count(), before.features().count());
    assert_eq!(after.definitions().count(), before.definitions().count());
    assert_eq!(
        after.occurrences().collect::<Vec<_>>(),
        before.occurrences().collect::<Vec<_>>()
    );
    assert_eq!(
        after.dowel_joints().collect::<Vec<_>>(),
        before.dowel_joints().collect::<Vec<_>>()
    );
    for feature in before
        .features()
        .filter(|feature| feature.id() != FeatureId(95))
    {
        assert_eq!(after.feature(feature.id()), Some(feature));
    }
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("resized-original-v9.ketchup");
    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    std::fs::write(&saved, bytes).unwrap();
    let mut cold = ketchup_application::DocumentSession::open(
        &saved,
        ketchup_application::SessionSettings::default(),
    )
    .unwrap();
    assert!(cold.exact_results().is_empty());
    assert!(
        cold.evaluate_with_timeout(Duration::from_secs(60))
            .unwrap()
            .establishes_full_baseline()
    );
    assert_eq!(cold.snapshot().canonical_digest(), after.canonical_digest());
    let snapshot = cold.snapshot();
    let bodies = cold.exact_results().body_values(&snapshot).unwrap();
    let rear_bodies = bodies
        .iter()
        .filter(|(body, _)| body.definition_id == DefinitionId(5))
        .collect::<Vec<_>>();
    assert_eq!(rear_bodies.len(), 1);
    for (actual, expected) in rear_bodies[0]
        .1
        .bounds_mm()
        .iter()
        .flatten()
        .zip([0.0, 0.0, 0.0, 464.0, 218.0, 8.0])
    {
        assert!(
            (actual - expected).abs() < 1.0e-6,
            "exact rear bound {actual} != {expected}"
        );
    }
    for joint in snapshot.dowel_joints() {
        let projection =
            ketchup_core::joinery::project_dowel_joint_contract(&snapshot, joint).unwrap();
        assert_eq!(projection.pairs.len(), 3);
        for pair in projection.pairs {
            assert!(
                pair.physical_probe_coincidence
                    .unwrap()
                    .maximum_endpoint_error_mm
                    < 1.0e-6
            );
            assert_eq!(pair.first.depth_mm, 16.0);
            assert_eq!(pair.second.depth_mm, 16.0);
        }
    }
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        after.canonical_digest()
    );
}

#[test]
fn original_v9_rear_physical_joinery_after_unique_sides() {
    use ketchup_application::plan_assistant_cad_edit_program;
    use ketchup_core::assistant_sidecar::AssistantCadParameterValueType;
    use ketchup_core::assistant_sidecar::{
        AssistantCadEditOperation, AssistantCadEditProgram, AssistantDowelJointFace,
        AssistantInstancePath, AssistantStandardDowel,
    };
    use ketchup_core::exact_product::ExactResultRegistry;
    use ketchup_core::joinery::project_dowel_joint_contract;
    use std::collections::BTreeSet;

    let path = fixture_directory().join("nightstand_v9_retention.ketchup");
    let mut document = persistence::load_file(&path)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let before = document.current();
    let undo = document.visible_undo_steps();
    let invalid = plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::MakeOccurrenceUnique {
                occurrence_id: 999_999,
            }],
        },
    );
    assert!(invalid.is_err());
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    let face = |id, origin, inward, maximum| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: id,
            steps: vec![],
        },
        face_origin_local_mm: origin,
        inward_unit_local: inward,
        bounds_min_local_mm: [0.0; 3],
        bounds_max_local_mm: maximum,
    };
    let rear = |origin, inward| face(6, origin, inward, [464.0, 218.0, 8.0]);
    let side = |id| face(id, [0.0; 3], [0.0, 0.0, 1.0], [350.0, 432.0, 18.0]);
    let row = |name: &str, first, second, center, direction, spacing| {
        AssistantCadEditOperation::CreatePhysicalDowelJoint {
            joint_id: None,
            name: name.into(),
            first,
            second,
            first_center_local_mm: center,
            row_unit_first_local: direction,
            count: 3,
            spacing_mm: spacing,
            dowel: AssistantStandardDowel::D8x30,
            first_insertion_mm: None,
        }
    };
    let program = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::SetFeatureParameter {
                feature_id: 95,
                parameter_path: "bounds.height".into(),
                value_type: AssistantCadParameterValueType::Length,
                value: 218.0,
            },
            AssistantCadEditOperation::MakeOccurrenceUnique { occurrence_id: 2 },
            AssistantCadEditOperation::MakeOccurrenceUnique { occurrence_id: 3 },
            row(
                "Rear to top",
                rear([0.0, 218.0, 0.0], [0.0, -1.0, 0.0]),
                face(1, [0.0; 3], [0.0, 0.0, 1.0], [500.0, 350.0, 18.0]),
                [80.0, 218.0, 4.0],
                [1.0, 0.0, 0.0],
                152.0,
            ),
            row(
                "Rear to left",
                rear([0.0; 3], [1.0, 0.0, 0.0]),
                side(2),
                [0.0, 54.5, 4.0],
                [0.0, 1.0, 0.0],
                54.5,
            ),
            row(
                "Rear to right",
                rear([464.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
                side(3),
                [464.0, 54.5, 4.0],
                [0.0, 1.0, 0.0],
                54.5,
            ),
        ],
    };
    let planned = plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program,
    )
    .unwrap();
    document.apply_batch(&planned).unwrap();
    let after = document.current();
    assert_eq!(document.visible_undo_steps(), undo + 1);
    assert_eq!(after.occurrences().count(), before.occurrences().count());
    assert_eq!(
        after.occurrence(OccurrenceId(6)),
        before.occurrence(OccurrenceId(6))
    );
    assert_eq!(after.dowel_joints().count(), 10);
    let repeated = plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::MakeOccurrenceUnique { occurrence_id: 2 },
                AssistantCadEditOperation::MakeOccurrenceUnique { occurrence_id: 3 },
            ],
        },
    )
    .unwrap();
    assert!(repeated.commands().is_empty());
    assert_eq!(
        after
            .dowel_joints()
            .map(|joint| joint.count as usize)
            .sum::<usize>(),
        30
    );
    assert_eq!(
        after.definition(DefinitionId(2)),
        before.definition(DefinitionId(2))
    );
    for feature in before
        .features()
        .filter(|feature| feature.id() != FeatureId(95))
    {
        assert_eq!(after.feature(feature.id()), Some(feature));
    }
    for occurrence in before
        .occurrences()
        .filter(|occurrence| ![OccurrenceId(2), OccurrenceId(3)].contains(&occurrence.id()))
    {
        assert_eq!(after.occurrence(occurrence.id()), Some(occurrence));
    }
    for (id, centers) in [
        (
            8,
            [[80.0, 218.0, 4.0], [232.0, 218.0, 4.0], [384.0, 218.0, 4.0]],
        ),
        (9, [[0.0, 54.5, 4.0], [0.0, 109.0, 4.0], [0.0, 163.5, 4.0]]),
        (
            10,
            [[464.0, 54.5, 4.0], [464.0, 109.0, 4.0], [464.0, 163.5, 4.0]],
        ),
    ] {
        let joint = after
            .dowel_joint(ketchup_core::joinery::DowelJointId(id))
            .unwrap();
        let projected = project_dowel_joint_contract(&after, joint).unwrap();
        assert_eq!(projected.pairs.len(), 3);
        for (pair, center) in projected.pairs.iter().zip(centers) {
            for axis in 0..3 {
                let transform = before.occurrence(OccurrenceId(6)).unwrap().transform();
                let m = transform.matrix();
                let world = m[axis * 4] * center[0]
                    + m[axis * 4 + 1] * center[1]
                    + m[axis * 4 + 2] * center[2]
                    + m[axis * 4 + 3];
                assert!((pair.first.shared_center_world_mm[axis] - world).abs() < 1.0e-6);
            }
            assert_eq!(pair.first.diameter_mm, 8.0);
            assert_eq!(pair.first.depth_mm, 16.0);
            assert_eq!(pair.second.depth_mm, 16.0);
            assert!(
                pair.physical_probe_coincidence
                    .unwrap()
                    .maximum_endpoint_error_mm
                    < 1.0e-6
            );
        }
    }
    for joint in before.dowel_joints() {
        let old = project_dowel_joint_contract(&before, joint).unwrap();
        let new =
            project_dowel_joint_contract(&after, after.dowel_joint(joint.id).unwrap()).unwrap();
        assert_eq!(old, new);
    }
    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("v9-repaired.ketchup");
    std::fs::write(&saved, &bytes).unwrap();
    let mut cold = ketchup_application::DocumentSession::open(
        &saved,
        ketchup_application::SessionSettings::default(),
    )
    .unwrap();
    assert!(cold.exact_results().is_empty());
    assert!(
        cold.evaluate_with_timeout(Duration::from_secs(60))
            .unwrap()
            .establishes_full_baseline()
    );
    let reopened = cold.snapshot();
    assert_eq!(reopened.canonical_digest(), after.canonical_digest());
    let rear_definition = after.occurrence(OccurrenceId(6)).unwrap().definition_id();
    let bodies = cold.exact_results().body_values(&reopened).unwrap();
    let rear_bodies = bodies
        .iter()
        .filter(|(body, _)| body.definition_id == rear_definition)
        .collect::<Vec<_>>();
    assert_eq!(rear_bodies.len(), 1);
    for (actual, expected) in rear_bodies[0]
        .1
        .bounds_mm()
        .iter()
        .flatten()
        .zip([0.0, 0.0, 0.0, 464.0, 218.0, 8.0])
    {
        assert!(
            (actual - expected).abs() < 1.0e-6,
            "exact rear bound {actual} != {expected}"
        );
    }
    for occurrence_id in [OccurrenceId(2), OccurrenceId(3)] {
        assert_eq!(
            bodies
                .iter()
                .filter(|(body, _)| body.definition_id
                    == after.occurrence(occurrence_id).unwrap().definition_id())
                .count(),
            1
        );
    }
    for joint in reopened.dowel_joints() {
        let projection = project_dowel_joint_contract(&reopened, joint).unwrap();
        assert_eq!(projection.pairs.len(), 3);
        for pair in projection.pairs {
            assert!(
                pair.physical_probe_coincidence
                    .unwrap()
                    .maximum_endpoint_error_mm
                    < 1.0e-6
            );
        }
    }
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        after.canonical_digest()
    );
}

#[test]
fn original_shared_side_make_unique_preserves_physical_holes_and_recipe() {
    use ketchup_core::assembly_recipe::{RecipeParameter, RecipeParameterUnit};
    use ketchup_core::document::{FeatureParameterTarget, ParameterValueType};
    use ketchup_core::joinery::project_dowel_joint_contract;

    for fixture in [
        "nightstand_v8_full_probe.ketchup",
        "nightstand_v9_retention.ketchup",
    ] {
        for occurrence_id in [OccurrenceId(2), OccurrenceId(3)] {
            let mut document = persistence::load_file(fixture_directory().join(fixture))
                .unwrap()
                .into_editable()
                .ok()
                .unwrap();
            let original = document.current();
            let source_id = original.occurrence(occurrence_id).unwrap().definition_id();
            let source = original.definition(source_id).unwrap();
            let sketch_id = source
                .feature_ids()
                .iter()
                .copied()
                .find(|id| {
                    matches!(
                        original.feature(*id).unwrap().kind(),
                        FeatureKind::Sketch(_)
                    )
                })
                .unwrap();
            let target =
                FeatureParameterTarget::new(sketch_id, "bounds.width", ParameterValueType::Length)
                    .unwrap();
            let key = |name: &str| RecipeKey::new(name).unwrap();
            let recipe = AssemblyRecipe::adopt(
                &original,
                key("nightstand"),
                vec![RecipePartAdoption {
                    key: key("side"),
                    instance_path: InstancePath::root(occurrence_id),
                    mobility: RecipePartMobility::Fixed,
                    edit_scope: RecipeEditScope::SharedDefinition(source_id),
                    parameters: BTreeMap::from([(
                        key("width"),
                        RecipeParameter {
                            value: original.feature_parameter_value(&target).unwrap(),
                            unit: RecipeParameterUnit::Millimetres,
                            target: Some(target),
                        },
                    )]),
                    features: source
                        .feature_ids()
                        .iter()
                        .map(|id| {
                            let kind = match original.feature(*id).unwrap().kind() {
                                FeatureKind::Workplane(_) => RecognizedRecipeFeatureKind::Workplane,
                                FeatureKind::Sketch(_) => RecognizedRecipeFeatureKind::Sketch,
                                FeatureKind::Pad(_) => RecognizedRecipeFeatureKind::Pad,
                                FeatureKind::Pocket { .. } => RecognizedRecipeFeatureKind::Pocket,
                                other => panic!("unexpected source feature {other:?}"),
                            };
                            (key(&format!("side/feature-{}", id.0)), *id, kind)
                        })
                        .collect(),
                }],
                vec![],
                vec![],
            )
            .unwrap();
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetAssemblyRecipe(recipe),
                ]))
                .unwrap();
            let before = document.current();
            let undo_steps = document.visible_undo_steps();
            let next_definition = before
                .definitions()
                .map(|definition| definition.id().0)
                .max()
                .unwrap()
                + 1;
            let next_feature = before
                .features()
                .map(|feature| feature.id().0)
                .max()
                .unwrap()
                + 1;
            let proposal = document
                .prepare_proposal(CommandBatch::new(vec![
                    CanonicalCommand::CloneDefinitionAndRepoint(
                        ketchup_core::document::CloneDefinitionPlan::new(
                            occurrence_id,
                            source_id,
                            DefinitionId(next_definition),
                            "Independent side".into(),
                            source
                                .feature_ids()
                                .iter()
                                .enumerate()
                                .map(|(index, id)| (*id, FeatureId(next_feature + index as u64)))
                                .collect(),
                        ),
                    ),
                ]))
                .unwrap();
            use ketchup_core::document::AuthoritativeDependency;
            assert!(
                proposal
                    .authoritative_dependencies()
                    .contains(&AuthoritativeDependency::AssemblyRecipe)
            );
            assert!(
                proposal
                    .authoritative_writes()
                    .contains(&AuthoritativeDependency::AssemblyRecipe)
            );
            for joint in before.dowel_joints().filter(|joint| {
                joint.first.instance_path == InstancePath::root(occurrence_id)
                    || joint.second.instance_path == InstancePath::root(occurrence_id)
            }) {
                assert!(
                    proposal
                        .authoritative_dependencies()
                        .contains(&AuthoritativeDependency::DowelJoint(joint.id))
                );
                assert!(
                    proposal
                        .authoritative_writes()
                        .contains(&AuthoritativeDependency::DowelJoint(joint.id))
                );
            }
            assert_eq!(
                document.current().canonical_digest(),
                before.canonical_digest()
            );
            let mut added_joint = before
                .dowel_joints()
                .find(|joint| {
                    joint.first.instance_path == InstancePath::root(occurrence_id)
                        || joint.second.instance_path == InstancePath::root(occurrence_id)
                })
                .unwrap()
                .clone();
            added_joint.id = ketchup_core::joinery::DowelJointId(
                before.dowel_joints().map(|joint| joint.id.0).max().unwrap() + 1,
            );
            added_joint.name = "Concurrent joint".into();
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::UpsertDowelJoint(added_joint),
                ]))
                .unwrap();
            let concurrent = document.current();
            assert!(matches!(
                document.commit_proposal(&proposal),
                Err(ketchup_core::document::ProposalCommitError::Stale(_))
            ));
            assert_eq!(
                document.current().canonical_digest(),
                concurrent.canonical_digest()
            );
            assert_eq!(document.visible_undo_steps(), undo_steps + 1);
            document.undo().unwrap();
            assert_eq!(
                document.current().canonical_digest(),
                before.canonical_digest()
            );
            document
                .make_unique(occurrence_id, "Independent side")
                .unwrap();
            let after = document.current();
            let new_id = after.occurrence(occurrence_id).unwrap().definition_id();
            assert_ne!(new_id, source_id);
            assert_eq!(after.occurrences().count(), before.occurrences().count());
            assert_eq!(
                after.definitions().count(),
                before.definitions().count() + 1
            );
            assert_eq!(
                after.features().count(),
                before.features().count() + source.feature_ids().len()
            );
            for feature in before.features() {
                assert_eq!(after.feature(feature.id()), Some(feature));
            }
            for definition in before.definitions() {
                assert_eq!(after.definition(definition.id()), Some(definition));
            }
            for occurrence in before.occurrences() {
                if occurrence.id() != occurrence_id {
                    assert_eq!(after.occurrence(occurrence.id()), Some(occurrence));
                } else {
                    let changed = after.occurrence(occurrence_id).unwrap();
                    assert_eq!(changed.transform(), occurrence.transform());
                    assert_eq!(changed.visible(), occurrence.visible());
                    assert_eq!(changed.name(), occurrence.name());
                }
            }
            let mapping = source
                .feature_ids()
                .iter()
                .copied()
                .zip(
                    after
                        .definition(new_id)
                        .unwrap()
                        .feature_ids()
                        .iter()
                        .copied(),
                )
                .collect::<BTreeMap<_, _>>();
            let recipe = after.assembly_recipe().unwrap();
            recipe.audit(&after).unwrap();
            let part = recipe.parts().next().unwrap();
            assert_eq!(part.definition_id, new_id);
            assert_eq!(
                part.edit_scope,
                RecipeEditScope::Occurrence(InstancePath::root(occurrence_id))
            );
            assert_eq!(
                part.parameters[&key("width")]
                    .target
                    .as_ref()
                    .unwrap()
                    .feature_id,
                mapping[&sketch_id]
            );
            for owned in recipe.owned_features() {
                let previous = before
                    .assembly_recipe()
                    .unwrap()
                    .owned_features()
                    .find(|old| old.key == owned.key)
                    .unwrap();
                assert_eq!(owned.feature_id, mapping[&previous.feature_id]);
            }
            assert_eq!(after.dowel_joints().count(), before.dowel_joints().count());
            let mut remapped_sides = 0;
            for joint in before.dowel_joints() {
                let mut expected = joint.clone();
                for pair in expected.physical_hole_pairs.as_mut().unwrap() {
                    if joint.first.instance_path == InstancePath::root(occurrence_id) {
                        pair.first_pocket_feature_id = mapping[&pair.first_pocket_feature_id];
                        remapped_sides += 1;
                    }
                    if joint.second.instance_path == InstancePath::root(occurrence_id) {
                        pair.second_pocket_feature_id = mapping[&pair.second_pocket_feature_id];
                        remapped_sides += 1;
                    }
                }
                assert_eq!(after.dowel_joint(joint.id), Some(&expected));
                assert_eq!(
                    project_dowel_joint_contract(&after, &expected).unwrap(),
                    project_dowel_joint_contract(&before, joint).unwrap()
                );
            }
            assert!(remapped_sides > 0);
            let bytes =
                persistence::save_document_store(&document, &persistence::ContainerData::default())
                    .unwrap();
            let directory = tempfile::tempdir().unwrap();
            let saved = directory.path().join("independent-side.ketchup");
            std::fs::write(&saved, &bytes).unwrap();
            let mut cold = ketchup_application::DocumentSession::open(
                &saved,
                ketchup_application::SessionSettings::default(),
            )
            .unwrap();
            assert!(cold.exact_results().is_empty());
            assert!(
                cold.evaluate_with_timeout(Duration::from_secs(60))
                    .unwrap()
                    .establishes_full_baseline()
            );
            let snapshot = cold.snapshot();
            let bodies = cold.exact_results().body_values(&snapshot).unwrap();
            let source_bodies = bodies
                .iter()
                .filter(|(body, _)| body.definition_id == source_id)
                .collect::<Vec<_>>();
            let cloned_bodies = bodies
                .iter()
                .filter(|(body, _)| body.definition_id == new_id)
                .collect::<Vec<_>>();
            assert_eq!(source_bodies.len(), 1);
            assert_eq!(cloned_bodies.len(), 1);
            assert_eq!(
                source_bodies[0].1.bounds_mm(),
                cloned_bodies[0].1.bounds_mm()
            );
            assert_eq!(source_bodies[0].1.vertices(), cloned_bodies[0].1.vertices());
            assert_eq!(
                source_bodies[0].1.triangles(),
                cloned_bodies[0].1.triangles()
            );
            for joint in snapshot.dowel_joints() {
                let projection = project_dowel_joint_contract(&snapshot, joint).unwrap();
                for pair in projection.pairs {
                    assert_eq!(pair.first.diameter_mm, 8.0);
                    assert_eq!(pair.second.diameter_mm, 8.0);
                    assert_eq!(pair.first.depth_mm, 16.0);
                    assert_eq!(pair.second.depth_mm, 16.0);
                    assert!(
                        pair.physical_probe_coincidence
                            .unwrap()
                            .maximum_endpoint_error_mm
                            < 1.0e-6
                    );
                }
            }
            let reopened = persistence::load(&bytes)
                .unwrap()
                .into_editable()
                .ok()
                .unwrap();
            assert_eq!(
                reopened.current().canonical_digest(),
                after.canonical_digest()
            );
            assert_eq!(document.visible_undo_steps(), undo_steps + 1);
            document.undo().unwrap();
            assert_eq!(
                document.current().canonical_digest(),
                before.canonical_digest()
            );
            document.redo().unwrap();
            assert_eq!(
                document.current().canonical_digest(),
                after.canonical_digest()
            );
        }
    }
}

#[test]
fn reference_request_freezes_geometry_joinery_tolerances_and_sla_before_implementation() {
    let manifest = manifest();
    let request = manifest.request;
    assert_eq!(
        request.target_part.source_dimensions_mm,
        [464.0, 216.0, 8.0]
    );
    assert_eq!(
        request.target_part.target_dimensions_mm,
        [464.0, 218.0, 8.0]
    );
    assert_eq!(request.target_part.required_gap_mm, 0.0);
    assert_eq!(request.target_part.contact_tolerance_mm, 1.0e-6);
    assert_eq!(request.dowel.diameter_mm, 8.0);
    assert_eq!(request.dowel.length_mm, 30.0);
    assert_eq!(request.dowel.hole_depth_each_part_mm, 16.0);
    assert_eq!(request.dowel.probe_endpoint_tolerance_mm, 1.0e-6);
    assert_eq!(request.rows.len(), 4);
    assert!(
        request
            .rows
            .iter()
            .all(|row| row.centers_rear_local_mm.len() == 3)
    );
    assert_eq!(
        request
            .rows
            .iter()
            .filter(|row| row.action == "preserve")
            .count(),
        1
    );
    assert_eq!(
        request
            .rows
            .iter()
            .filter(|row| row.action == "add")
            .count(),
        3
    );
    assert_eq!(
        request
            .rows
            .iter()
            .map(|row| row.key.as_str())
            .collect::<Vec<_>>(),
        [
            "rear_to_middle_shelf",
            "rear_to_top_board",
            "rear_to_left_side",
            "rear_to_right_side",
        ]
    );
    assert_eq!(request.expected.rear_joint_rows, 4);
    assert_eq!(request.expected.rear_dowels, 12);
    assert_eq!(request.expected.new_rear_dowels, 9);
    assert_eq!(request.expected.total_document_dowel_joints, 10);
    assert_eq!(request.expected.total_document_dowels, 30);
    assert!(!request.expected.replacement_part_allowed);
    assert!(!request.expected.hidden_superseded_part_allowed);
    assert_eq!(request.expected.undo_steps_added, 1);
    assert_eq!(request.expected.illegal_collisions, 0);
    assert_eq!(request.expected.maximum_sequential_tool_round_trips, 3);
    assert_eq!(request.expected.planning_budget_ms, 500);
    assert_eq!(request.expected.host_job_budget_ms, 10_000);
    assert_eq!(request.expected.end_to_end_budget_ms, 60_000);
    assert_eq!(manifest.large_fixture.cabinet_count, 17);
    assert_eq!(manifest.large_fixture.panel_count, 278);
    assert_eq!(manifest.large_fixture.generation, "deterministic_in_test");
    assert_eq!(manifest.historical_attempt.reported_total_minutes, 21);
    assert!(!manifest.historical_attempt.phase_trace_available);
    assert!(
        manifest
            .historical_attempt
            .policy
            .starts_with("Do not reconstruct")
    );
}

#[test]
fn trace_distinguishes_partial_host_and_real_llm_evidence() {
    let hash = "a".repeat(64);
    let mut host = AssemblyWorkflowTraceRecorder::new(
        "synthetic-host-1",
        WorkflowEvidenceKind::SyntheticHost,
        hash.clone(),
        WorkflowRunProfile::current(None, None),
    );
    host.record_phase(WorkflowPhase::ModelContext, Duration::from_millis(1));
    host.record_phase(WorkflowPhase::Plan, Duration::from_millis(1));
    let incomplete = host.finish();
    assert!(!incomplete.complete);
    assert!(incomplete.missing_phases.contains(&WorkflowPhase::Commit));
    incomplete.validate().unwrap();

    let mut end_to_end = AssemblyWorkflowTraceRecorder::new(
        "llm-e2e-1",
        WorkflowEvidenceKind::LlmEndToEnd,
        hash,
        WorkflowRunProfile::current(Some("test-provider".into()), Some("test-model".into())),
    );
    for phase in [
        WorkflowPhase::ModelContext,
        WorkflowPhase::LlmWait,
        WorkflowPhase::ToolWait,
        WorkflowPhase::Plan,
        WorkflowPhase::Commit,
        WorkflowPhase::Exact,
        WorkflowPhase::Validators,
        WorkflowPhase::Save,
        WorkflowPhase::Viewport,
    ] {
        end_to_end.record_phase(phase, Duration::from_micros(1));
    }
    end_to_end.record_tool_round_trip();
    let complete = end_to_end.finish();
    assert!(complete.complete);
    assert_eq!(complete.sequential_tool_round_trips, 1);
    complete.validate().unwrap();
    let mut round_trip: ketchup_application::workflow_trace::AssemblyWorkflowTrace =
        serde_json::from_slice(&serde_json::to_vec(&complete).unwrap()).unwrap();
    // JSON parsing can round the measured f64 by one ULP; all non-time fields stay exact.
    assert!(
        (round_trip.total_elapsed_ms - complete.total_elapsed_ms).abs()
            <= f64::EPSILON * complete.total_elapsed_ms.abs()
    );
    round_trip.total_elapsed_ms = complete.total_elapsed_ms;
    assert_eq!(round_trip, complete);
}

fn deterministic_large_panel_fixture() -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = Vec::new();
    let mut panel_index = 0_u64;
    for cabinet in 0..17_u64 {
        let panel_count = if cabinet < 6 { 17 } else { 16 };
        for local_panel in 0..panel_count {
            panel_index += 1;
            let definition_id = DefinitionId(panel_index);
            let profile_id = FeatureId(panel_index * 2 - 1);
            let body_id = FeatureId(panel_index * 2);
            let occurrence_id = OccurrenceId(panel_index);
            let width = 400.0 + (local_panel % 5) as f64 * 25.0;
            let height = 300.0 + (local_panel % 7) as f64 * 20.0;
            commands.extend([
                CanonicalCommand::CreateDefinition {
                    id: definition_id,
                    name: format!("cabinet-{cabinet:02}-panel-{local_panel:02}"),
                },
                CanonicalCommand::CreateFeature {
                    id: profile_id,
                    definition_id,
                    name: "panel-profile".into(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: body_id,
                    definition_id,
                    name: "panel-body".into(),
                    kind: FeatureKind::Extrusion {
                        profile: profile_id,
                        height: Dimension::from_decimal("18").unwrap(),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: occurrence_id,
                    definition_id,
                    name: format!("cabinet-{cabinet:02}-panel-{local_panel:02}"),
                    transform: Transform::from_translation(
                        cabinet as f64 * 1_000.0,
                        local_panel as f64 * 25.0,
                        0.0,
                    )
                    .unwrap(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]);
        }
    }
    assert_eq!(panel_index, 278);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

#[test]
fn physical_joinery_in_278_panel_fixture_drills_both_parts_in_every_cabinet() {
    use ketchup_application::plan_assistant_cad_edit_program;
    use ketchup_core::assistant_sidecar::{
        AssistantCadEditOperation, AssistantCadEditProgram, AssistantDowelJointFace,
        AssistantInstancePath, AssistantStandardDowel,
    };
    use ketchup_core::exact_product::ExactResultRegistry;
    use ketchup_core::joinery::project_dowel_joint_contract;
    use std::collections::BTreeSet;

    let mut document = deterministic_large_panel_fixture();
    let before = document.current();
    assert_eq!(before.occurrences().count(), 278);
    let mut first_ids = Vec::new();
    let mut transforms = Vec::new();
    for cabinet in 0..17_u64 {
        let first = if cabinet < 6 {
            cabinet * 17 + 1
        } else {
            6 * 17 + (cabinet - 6) * 16 + 1
        };
        first_ids.push(first);
        transforms.push(CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(first + 1),
            transform: Transform::from_translation(cabinet as f64 * 1_000.0, 0.0, 18.0).unwrap(),
        });
        for local_panel in 2..if cabinet < 6 { 17 } else { 16 } {
            transforms.push(CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(first + local_panel),
                transform: Transform::from_translation(
                    cabinet as f64 * 1_000.0,
                    local_panel as f64 * 500.0,
                    0.0,
                )
                .unwrap(),
            });
        }
    }
    document
        .apply_batch(&CommandBatch::new(transforms))
        .unwrap();
    let positioned = document.current();
    let history = document.visible_undo_steps();
    let face = |id, z, inward, width, height| AssistantDowelJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: id,
            steps: vec![],
        },
        face_origin_local_mm: [0.0, 0.0, z],
        inward_unit_local: inward,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [width, height, 18.0],
    };
    let program = AssistantCadEditProgram {
        operations: first_ids
            .iter()
            .map(
                |&first| AssistantCadEditOperation::CreatePhysicalDowelJoint {
                    joint_id: None,
                    name: format!("Cabinet {first} physical row"),
                    first: face(first, 18.0, [0.0, 0.0, -1.0], 400.0, 300.0),
                    second: face(first + 1, 0.0, [0.0, 0.0, 1.0], 425.0, 320.0),
                    first_center_local_mm: [60.0, 60.0, 18.0],
                    row_unit_first_local: [1.0, 0.0, 0.0],
                    count: 3,
                    spacing_mm: 100.0,
                    dowel: AssistantStandardDowel::D8x30,
                    first_insertion_mm: None,
                },
            )
            .collect(),
    };
    let planned = plan_assistant_cad_edit_program(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program,
    )
    .unwrap();
    document.apply_batch(&planned).unwrap();
    let after = document.current();
    assert_eq!(after.occurrences().count(), 278);
    assert_eq!(after.dowel_joints().count(), 17);
    assert_eq!(
        after.dowel_joints().map(|joint| joint.count).sum::<u32>(),
        51
    );
    assert_eq!(document.visible_undo_steps(), history + 1);
    for (index, joint) in after.dowel_joints().enumerate() {
        assert_eq!(
            joint.first.instance_path.root_occurrence().0,
            first_ids[index]
        );
        assert_eq!(
            joint.second.instance_path.root_occurrence().0,
            first_ids[index] + 1
        );
        let bindings = joint.physical_hole_pairs.as_ref().unwrap();
        assert_eq!(bindings.len(), 3);
        for binding in bindings {
            for feature_id in [
                binding.first_pocket_feature_id,
                binding.second_pocket_feature_id,
            ] {
                assert!(matches!(
                    after.feature(feature_id).unwrap().kind(),
                    FeatureKind::Pocket { .. }
                ));
                assert!(before.feature(feature_id).is_none());
            }
        }
        let projection = project_dowel_joint_contract(&after, joint).unwrap();
        assert_eq!(projection.pairs.len(), 3);
        for (point, pair) in projection.pairs.iter().enumerate() {
            let expected_x = index as f64 * 1_000.0 + 60.0 + point as f64 * 100.0;
            assert_eq!(pair.first.shared_center_world_mm[1], 60.0);
            assert!((pair.first.shared_center_world_mm[0] - expected_x).abs() < 1.0e-6);
            assert_eq!(pair.first.diameter_mm, 8.0);
            assert_eq!(pair.first.depth_mm, 16.0);
            assert_eq!(pair.second.depth_mm, 16.0);
            assert!(
                pair.physical_probe_coincidence
                    .unwrap()
                    .maximum_endpoint_error_mm
                    < 1.0e-6
            );
        }
    }
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        positioned.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        after.canonical_digest()
    );
}

#[test]
fn local_edit_in_278_panel_fixture_scopes_one_producer_and_one_occurrence() {
    let mut document = deterministic_large_panel_fixture();
    let before = document.current();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: FeatureId(1),
                points_mm: vec![[0.0, 0.0], [425.0, 0.0], [425.0, 300.0], [0.0, 300.0]],
            },
        ]))
        .unwrap();

    let plan = plan_incremental_exact_evaluation(
        &before,
        &document.current(),
        Some(&exact_source(&before)),
    )
    .unwrap();
    let ExactEvaluationSelection::Scoped(producers) = &plan.selection else {
        panic!("valid full baseline must permit a scoped plan: {plan:?}");
    };
    assert_eq!(producers.len(), 1, "{plan:?}");
    assert_eq!(plan.collision_occurrences.len(), 1, "{plan:?}");
    assert_eq!(plan.changed_feature_count, 1, "{plan:?}");
    assert_eq!(plan.changed_scene_occurrence_count, 0, "{plan:?}");
}

#[test]
fn baseline_report_measures_preserved_inputs_and_builds_the_17_cabinet_278_panel_fixture() {
    let manifest = manifest();
    let mut fixtures = Vec::new();
    for expected in &manifest.fixtures {
        let path = fixture_directory().join(&expected.path);
        let started = Instant::now();
        let loaded = persistence::load_file(&path).unwrap();
        let cold_open = started.elapsed();
        let snapshot = loaded.snapshot();
        fixtures.push(FixtureBaseline {
            path: expected.path.clone(),
            role: expected.role.clone(),
            sha256: expected.sha256.clone(),
            canonical_digest: snapshot.canonical_digest().to_owned(),
            cold_open_ms: cold_open.as_secs_f64() * 1_000.0,
            definitions: snapshot.definitions().count(),
            features: snapshot.features().count(),
            occurrences: snapshot.occurrences().count(),
            dowel_joints: snapshot.dowel_joints().count(),
        });
    }

    let large = deterministic_large_panel_fixture();
    assert_eq!(large.current().definitions().count(), 278);
    assert_eq!(large.current().features().count(), 556);
    assert_eq!(large.current().occurrences().count(), 278);

    let report = BaselineReport {
        schema: "ketchup.fast-assembly-baseline.v1",
        evidence_kind: "diagnostic_introspection_and_synthetic_fixture",
        fixtures,
        large_fixture_cabinets: manifest.large_fixture.cabinet_count,
        large_fixture_panels: manifest.large_fixture.panel_count,
        historical_total_minutes_reported_by_user: manifest
            .historical_attempt
            .reported_total_minutes,
        historical_phase_trace_available: manifest.historical_attempt.phase_trace_available,
        limitations: [
            "The historical 21-minute attempt has no phase trace and is not reconstructed.",
            "Fixture open timings are diagnostic introspection, not a host edit benchmark.",
            "The 278-panel fixture is synthetic and is not an LLM end-to-end result.",
        ],
    };
    let json = serde_json::to_string_pretty(&report).unwrap();
    eprintln!("KETCHUP_FAST_ASSEMBLY_BASELINE={json}");
    if let Some(path) = std::env::var_os("KETCHUP_FAST_ASSEMBLY_BASELINE_PATH") {
        std::fs::write(path, format!("{json}\n")).unwrap();
    }
}
