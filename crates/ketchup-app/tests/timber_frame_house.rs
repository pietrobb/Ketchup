//! End-to-end acceptance proof: a timber-frame house built from an empty
//! document through the generic typed CAD edit program only (the program an
//! MCP client proposes and commits).
//!
//! No named-shape intent, no fixture, no mesh shortcut. Every step is one typed
//! program that the host compiles into canonical commands, applies, verifies,
//! and commits as exactly one undo step.
//!
//! The second test pins the two generality limits this scenario hit, so that a
//! later change which lifts them fails loudly instead of silently.
use crate::harness;

use eframe::egui::accesskit::Role;
use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_application::validation::ASSISTANT_VALIDATOR_IDS;
use ketchup_assistant::sidecar::{
    AssistantCadBodyFeature, AssistantCadClassificationCategory, AssistantCadDeletePolicy,
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCadFeatureReference, AssistantCadPartFeature, AssistantCadProgramFeatureOutput,
    AssistantCadProgramFeatureReference, AssistantCadRotation, AssistantPrincipalPlane,
    AssistantSketchConstraint, AssistantSketchEntity, AssistantWorkplaneSpec,
};
use ketchup_geometry::sketch::{FeatureExtent, PadOperation, PadProfile, PadSpec};
use ketchup_manufacturing::fabrication::{
    FABRICATION_ROLE_DIMENSION_V1, GeneralManufacturingKind, ProjectionStatus, TIMBER_MATERIAL_V1,
    TIMBER_MEMBER_ROLE_V1,
};
use ketchup_model::document::{
    DefinitionId, EdgeFinishKind, FeatureId, FeatureKind, InstancePath, Snapshot,
};
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_model::exact_product::{
    ExactBodyPackage, ExactResultRegistry, terminal_body_exact_graphs,
};
use ketchup_model::exact_validation::{
    BuiltinGeneralBodyValidator, BuiltinGravitySupportValidator, GeneralBodyParticipant,
    GeneralClearanceCase, GravitySupportInput, GravitySupportParticipant, general_body_input_bytes,
    general_body_validation_policy, gravity_support_input_bytes, gravity_support_validation_policy,
};
use ketchup_model::persistence;
use ketchup_model::tolerance::TolerancePolicy;
use ketchup_model::validation::{
    HostNeutralValidator, ValidationExecution, ValidationInvocation, ValidationReport,
    ValidationState,
};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Structural dimensions of the proof house, in millimetres.
const HOUSE_LENGTH_MM: f64 = 6_000.0;
const HOUSE_WIDTH_MM: f64 = 4_000.0;
const WALL_HEIGHT_MM: f64 = 2_600.0;
const TIMBER_WIDTH_MM: f64 = 100.0;
const TIMBER_DEPTH_MM: f64 = 60.0;
const PLATE_THICKNESS_MM: f64 = 60.0;
const STUD_SPACING_MM: f64 = 625.0;
const STUD_INSTANCES: u32 = 9;
const SHEATHING_THICKNESS_MM: f64 = 18.0;
const RIDGE_RISE_MM: f64 = 1_200.0;

/// A closed axis-aligned rectangle as four constrained sketch lines.
fn rectangle(
    width_mm: f64,
    height_mm: f64,
) -> (Vec<AssistantSketchEntity>, Vec<AssistantSketchConstraint>) {
    let corners = [
        [0.0, 0.0],
        [width_mm, 0.0],
        [width_mm, height_mm],
        [0.0, height_mm],
    ];
    let entities = (0..4)
        .map(|index| AssistantSketchEntity::Line {
            id: index as u64 + 1,
            start_mm: corners[index],
            end_mm: corners[(index + 1) % 4],
        })
        .collect::<Vec<_>>();
    let constraints = (0..4)
        .map(|index| {
            let id = index as u64 + 1;
            if index % 2 == 0 {
                AssistantSketchConstraint::Horizontal { id, entity_id: id }
            } else {
                AssistantSketchConstraint::Vertical { id, entity_id: id }
            }
        })
        .collect::<Vec<_>>();
    (entities, constraints)
}

/// One extruded timber member placed by translation and optional rotation.
fn timber(
    name: &str,
    plan_width_mm: f64,
    plan_height_mm: f64,
    extrusion_mm: f64,
    translation_mm: [f64; 3],
    rotation: Option<AssistantCadRotation>,
) -> AssistantCadEditOperation {
    let (entities, constraints) = rectangle(plan_width_mm, plan_height_mm);
    AssistantCadEditOperation::CreatePart {
        holes: Vec::new(),
        pockets: Vec::new(),
        name: name.to_owned(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities,
        constraints,
        feature: AssistantCadPartFeature::Extrusion {
            distance_mm: extrusion_mm,
        },
        translation_mm,
        rotation,
    }
}

/// Apply one typed CAD program the way an MCP client's propose + commit does,
/// and prove it landed as exactly one revision and one undo step.
fn build_step(shell: &mut Shell, request: &str, program: AssistantCadEditProgram) {
    let revision_before = shell.app().document_revision();
    let digest_before = shell.app().canonical_digest();
    let undo_before = shell.app().undo_step_count();
    if let Err(error) = shell.app_mut().headless_apply_cad_program(&program) {
        panic!("{request}: the program must apply: {error}");
    }
    shell.step();
    assert_eq!(
        shell.app().document_revision(),
        revision_before + 1,
        "{request}: the edit must add exactly one revision"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        undo_before + 1,
        "{request}: the edit must add exactly one undo step"
    );
    assert_ne!(
        shell.app().canonical_digest(),
        digest_before,
        "{request}: the edit must change the canonical digest"
    );
}

fn definition_id_of(shell: &Shell, name: &str) -> DefinitionId {
    shell
        .app()
        .document_snapshot()
        .definitions()
        .find(|definition| definition.name() == name)
        .unwrap_or_else(|| panic!("definition {name} must exist"))
        .id()
}

fn body_feature_id_of(shell: &Shell, name: &str) -> FeatureId {
    let snapshot = shell.app().document_snapshot();
    let definition = snapshot
        .definitions()
        .find(|definition| definition.name() == name)
        .unwrap_or_else(|| panic!("definition {name} must exist"));
    definition
        .feature_ids()
        .iter()
        .copied()
        .find(|id| matches!(snapshot.feature(*id).unwrap().kind(), FeatureKind::Pad(_)))
        .unwrap_or_else(|| panic!("definition {name} must own a pad"))
}

/// Drive the whole frame from an empty document through typed CAD programs, and
/// return the shell plus the baseline the build started from.
fn build_timber_frame_house() -> (Shell, u64, String) {
    let requests = [
        "Clear the document so the site is empty",
        "Lay the sill plate ring for a 6000 by 4000 mm timber frame house",
        "Raise the first stud on the front wall",
        "Repeat that stud every 625 mm and cap the wall with a top plate",
        "Sheath the front wall with an 18 mm panel",
        "Stand the gable posts, carry the ridge beam on them and add a rafter sloping up to it",
        "Mark every completed house occurrence as a timber member for fabrication",
    ];
    let mut shell = Shell::new();
    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();

    // 0. Clear whatever the new document opened with, so the site is empty.
    let existing = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .collect::<Vec<_>>();
    assert!(!existing.is_empty(), "a new document must start occupied");
    build_step(
        &mut shell,
        requests[0],
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Delete {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: existing,
                },
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            }],
        },
    );
    assert_eq!(shell.app().document_snapshot().occurrences().count(), 0);

    // 1. Sill plate ring closing the whole footprint.
    let inner_width = HOUSE_WIDTH_MM - 2.0 * TIMBER_WIDTH_MM;
    build_step(
        &mut shell,
        requests[1],
        AssistantCadEditProgram {
            operations: vec![
                timber(
                    "Sill plate front",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    PLATE_THICKNESS_MM,
                    [0.0, 0.0, 0.0],
                    None,
                ),
                timber(
                    "Sill plate back",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    PLATE_THICKNESS_MM,
                    [0.0, HOUSE_WIDTH_MM - TIMBER_WIDTH_MM, 0.0],
                    None,
                ),
                timber(
                    "Sill plate left",
                    TIMBER_WIDTH_MM,
                    inner_width,
                    PLATE_THICKNESS_MM,
                    [0.0, TIMBER_WIDTH_MM, 0.0],
                    None,
                ),
                timber(
                    "Sill plate right",
                    TIMBER_WIDTH_MM,
                    inner_width,
                    PLATE_THICKNESS_MM,
                    [HOUSE_LENGTH_MM - TIMBER_WIDTH_MM, TIMBER_WIDTH_MM, 0.0],
                    None,
                ),
            ],
        },
    );
    assert_eq!(shell.app().document_snapshot().occurrences().count(), 4);

    // 2. The first stud, standing on the sill.
    build_step(
        &mut shell,
        requests[2],
        AssistantCadEditProgram {
            operations: vec![timber(
                "Front stud",
                TIMBER_WIDTH_MM,
                TIMBER_DEPTH_MM,
                WALL_HEIGHT_MM,
                [0.0, SHEATHING_THICKNESS_MM, PLATE_THICKNESS_MM],
                None,
            )],
        },
    );
    let stud_occurrence = shell
        .app()
        .document_snapshot()
        .occurrences()
        .find(|occurrence| occurrence.name() == "Front stud")
        .expect("stud occurrence must exist")
        .id();

    // 3. A real linear pattern of studs, plus the top plate, as one step.
    let before_pattern = shell.app().document_snapshot().occurrences().count();
    build_step(
        &mut shell,
        requests[3],
        AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::LinearPattern {
                    selector: AssistantCadEntitySelector::Occurrences {
                        occurrence_ids: vec![stud_occurrence.0],
                    },
                    instances: STUD_INSTANCES,
                    step_mm: [STUD_SPACING_MM, 0.0, 0.0],
                },
                timber(
                    "Top plate front",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    PLATE_THICKNESS_MM,
                    [0.0, 0.0, PLATE_THICKNESS_MM + WALL_HEIGHT_MM],
                    None,
                ),
            ],
        },
    );
    let after_pattern = shell.app().document_snapshot().occurrences().count();
    assert!(
        after_pattern > before_pattern + 1,
        "the linear pattern must add real stud instances"
    );
    let stud_definition = definition_id_of(&shell, "Front stud");
    let patterned_studs = shell
        .app()
        .document_snapshot()
        .occurrences()
        .filter(|occurrence| occurrence.definition_id() == stud_definition)
        .count();
    assert_eq!(
        patterned_studs, STUD_INSTANCES as usize,
        "every stud instance must reuse the same parametric definition"
    );

    // 4. Front sheathing standing on the XZ plane. The plane's normal is -y,
    //    so the sketch is placed one thickness in and extrudes back onto the
    //    front sill plate.
    let (sheathing_entities, sheathing_constraints) = rectangle(HOUSE_LENGTH_MM, WALL_HEIGHT_MM);
    build_step(
        &mut shell,
        requests[4],
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                holes: Vec::new(),
                pockets: Vec::new(),
                name: "Front sheathing".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities: sheathing_entities,
                constraints: sheathing_constraints,
                feature: AssistantCadPartFeature::Extrusion {
                    distance_mm: SHEATHING_THICKNESS_MM,
                },
                translation_mm: [0.0, SHEATHING_THICKNESS_MM, PLATE_THICKNESS_MM],
                rotation: None,
            }],
        },
    );

    // 5. Gable posts, the ridge beam they carry, and one rafter placed by an
    //    arbitrary finite rotation. The rafter runs from the outer top arris of
    //    the plate up to the near face of the ridge, so every roof member is
    //    carried by something that reaches the ground.
    let eaves_height = PLATE_THICKNESS_MM + WALL_HEIGHT_MM + PLATE_THICKNESS_MM;
    let ridge_near_face_mm = HOUSE_WIDTH_MM / 2.0 - TIMBER_WIDTH_MM / 2.0;
    let ridge_soffit_mm = eaves_height + RIDGE_RISE_MM;
    let rafter_run = ridge_near_face_mm;
    let rafter_length = rafter_run.hypot(RIDGE_RISE_MM);
    let rafter_pitch_degrees = RIDGE_RISE_MM.atan2(rafter_run).to_degrees();
    build_step(
        &mut shell,
        requests[5],
        AssistantCadEditProgram {
            operations: vec![
                timber(
                    "Gable post left",
                    TIMBER_WIDTH_MM,
                    TIMBER_WIDTH_MM,
                    ridge_soffit_mm - PLATE_THICKNESS_MM,
                    [0.0, ridge_near_face_mm, PLATE_THICKNESS_MM],
                    None,
                ),
                timber(
                    "Gable post right",
                    TIMBER_WIDTH_MM,
                    TIMBER_WIDTH_MM,
                    ridge_soffit_mm - PLATE_THICKNESS_MM,
                    [
                        HOUSE_LENGTH_MM - TIMBER_WIDTH_MM,
                        ridge_near_face_mm,
                        PLATE_THICKNESS_MM,
                    ],
                    None,
                ),
                timber(
                    "Ridge beam",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    2.0 * TIMBER_WIDTH_MM,
                    [0.0, ridge_near_face_mm, ridge_soffit_mm],
                    None,
                ),
                timber(
                    "Rafter",
                    TIMBER_WIDTH_MM,
                    rafter_length,
                    TIMBER_DEPTH_MM,
                    [0.0, 0.0, eaves_height],
                    Some(AssistantCadRotation {
                        pivot_mm: [0.0, 0.0, eaves_height],
                        axis: [1.0, 0.0, 0.0],
                        angle_degrees: rafter_pitch_degrees,
                    }),
                ),
            ],
        },
    );

    let snapshot = shell.app().document_snapshot();
    let dimension_id = snapshot
        .classification_dimensions()
        .map(|dimension| dimension.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let category_id = snapshot
        .classification_dimensions()
        .flat_map(|dimension| dimension.categories())
        .map(|category| category.id().0)
        .max()
        .unwrap_or(0)
        + 1;
    let occurrence_ids = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| occurrence.occurrence_id.0)
        .collect::<Vec<_>>();
    build_step(
        &mut shell,
        requests[6],
        AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::UpsertClassificationDimension {
                    dimension_id,
                    name: FABRICATION_ROLE_DIMENSION_V1.to_owned(),
                    categories: vec![AssistantCadClassificationCategory {
                        id: category_id,
                        name: TIMBER_MEMBER_ROLE_V1.to_owned(),
                    }],
                },
                AssistantCadEditOperation::SetOccurrenceClassification {
                    selector: AssistantCadEntitySelector::Occurrences { occurrence_ids },
                    dimension_id,
                    category_id: Some(category_id),
                },
            ],
        },
    );

    (shell, baseline_revision, baseline_digest)
}

/// Every structural member the Assistant authored, by name.
const HOUSE_MEMBERS: [&str; 11] = [
    "Sill plate front",
    "Sill plate back",
    "Sill plate left",
    "Sill plate right",
    "Front stud",
    "Top plate front",
    "Front sheathing",
    "Gable post left",
    "Gable post right",
    "Ridge beam",
    "Rafter",
];

/// Prove the built frame is exact, editable, savable and undoable.
#[test]
fn assistant_builds_a_timber_frame_house_from_an_empty_document() {
    let (mut shell, baseline_revision, baseline_digest) = build_timber_frame_house();

    // The rafter must really be tilted, not axis-aligned like every other member.
    let snapshot = shell.app().document_snapshot();
    let rafter_transform = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Rafter")
        .expect("rafter occurrence must exist")
        .transform();
    let rafter = rafter_transform.matrix();
    assert!(
        rafter[6].abs() > 0.1 && rafter[10].abs() > 0.1,
        "the rafter must carry a real out-of-plane rotation, got {rafter:?}"
    );

    // Every structural member must stay an editable exact chain, never a mesh.
    let committed = shell.app().document_snapshot();
    for definition in committed.definitions() {
        for feature_id in definition.feature_ids() {
            assert!(
                !matches!(
                    committed.feature(*feature_id).unwrap().kind(),
                    FeatureKind::MeshBody(_)
                ),
                "{} must stay an editable feature chain",
                definition.name()
            );
        }
    }

    // Every generated member must compile into the general exact BRep graph.
    for name in HOUSE_MEMBERS {
        let definition_id = definition_id_of(&shell, name);
        let body = body_feature_id_of(&shell, name);
        ExactBRepGraph::from_snapshot(&committed, definition_id, body)
            .unwrap_or_else(|error| panic!("{name} must compile into an exact graph: {error}"));
    }

    // Save and reopen: the whole house must round-trip losslessly and stay editable.
    let committed_digest = committed.canonical_digest();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("timber-frame-house.ketchup");
    persistence::save_atomic(&path, &committed).unwrap();
    let outcome = persistence::load_file(&path).unwrap();
    assert!(outcome.is_editable());
    assert_eq!(outcome.snapshot().canonical_digest(), committed_digest);
    publish_artifact("timber-frame-house.ketchup", &std::fs::read(&path).unwrap());

    // The whole build must unwind step by step back to the empty document.
    while shell.app().document_revision() > baseline_revision {
        assert!(shell.app_mut().undo());
    }
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
}

/// Copy a produced artifact into the test report directory when one is set,
/// so the proof house can be opened and inspected instead of only asserted on.
fn publish_artifact(name: &str, bytes: &[u8]) {
    if let Some(path) = ketchup_test_env::report_path(name) {
        std::fs::write(path, bytes).unwrap();
    }
}

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

/// Solves every house member exactly, through the same worker the app uses.
fn exact_member_packages(shell: &Shell, snapshot: &Snapshot) -> Vec<Arc<ExactBodyPackage>> {
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    HOUSE_MEMBERS
        .iter()
        .map(|name| {
            let definition_id = definition_id_of(shell, name);
            let graphs = terminal_body_exact_graphs(snapshot, definition_id)
                .unwrap_or_else(|error| panic!("{name} must compile to an exact graph: {error}"));
            let [graph] = graphs.values().collect::<Vec<_>>()[..] else {
                panic!("{name} must have one body");
            };
            let package = worker
                .evaluate_exact_brep_graph(graph)
                .unwrap_or_else(|error| panic!("{name} must solve exactly: {error}"));
            Arc::new(ExactBodyPackage::Graph(package))
        })
        .collect()
}

fn general_report(
    snapshot: &Snapshot,
    cases: &[GeneralClearanceCase],
    tolerance: TolerancePolicy,
) -> ValidationReport {
    let validator = BuiltinGeneralBodyValidator::new(tolerance);
    let policy = general_body_validation_policy();
    let input = general_body_input_bytes(cases);
    let invocation =
        ValidationInvocation::bind(snapshot, validator.descriptor(), &policy, vec![], &input);
    validator.invoke(ValidationExecution {
        snapshot,
        invocation,
        policy: &policy,
        input: cases,
    })
}

/// The Assistant-built house must carry all the way to a manufacturable
/// handoff: a bill of materials, per-piece drawings and machining operations,
/// each exportable only while it still matches the document it came from.
#[test]
fn the_timber_frame_house_projects_a_manufacturable_handoff() {
    let (shell, _revision, _digest) = build_timber_frame_house();
    let snapshot = shell.app().document_snapshot();
    let tolerance = TolerancePolicy::default();

    let packages = exact_member_packages(&shell, &snapshot);
    let registry = ExactResultRegistry::accept(&snapshot, packages).unwrap();

    // Cover every visible member with a clearance case; members touch, so the
    // required minimum is zero and interference would still be caught.
    let participants = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| {
            GeneralBodyParticipant::accept(
                &snapshot,
                &registry,
                InstancePath::root(occurrence.occurrence_id),
                tolerance,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{} must be an accepted general body: {error:?}",
                    occurrence.occurrence_name
                )
            })
        })
        .collect::<Vec<_>>();
    let visible_definitions = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| occurrence.definition_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        participants.len() >= 8 + STUD_INSTANCES as usize,
        "every house member must be an accepted general body"
    );
    let cases = (0..participants.len())
        .map(|index| {
            GeneralClearanceCase::new(
                participants[index].clone(),
                participants[(index + 1) % participants.len()].clone(),
                0.0,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let report = general_report(&snapshot, &cases, tolerance);
    assert_eq!(
        report.state,
        ValidationState::Passed,
        "{:#?}",
        report.diagnostics
    );

    let projection = ketchup_manufacturing::fabrication::project_general_fabrication(
        &snapshot, &registry, &cases, &report, tolerance,
    )
    .unwrap();
    assert_eq!(
        projection,
        ketchup_manufacturing::fabrication::project_general_fabrication(
            &snapshot, &registry, &cases, &report, tolerance,
        )
        .unwrap(),
        "the manufacturable handoff must regenerate deterministically"
    );

    // One bill-of-materials row per distinct member, with the studs pooled.
    assert_eq!(projection.bom.envelope.status, ProjectionStatus::Complete);
    assert_eq!(projection.bom.rows.len(), visible_definitions.len());
    assert!(
        projection
            .bom
            .rows
            .iter()
            .all(|row| row.material_key == TIMBER_MATERIAL_V1)
    );
    for name in HOUSE_MEMBERS {
        let definition_id = definition_id_of(&shell, name);
        assert!(
            projection
                .bom
                .rows
                .iter()
                .any(|row| row.definition_id == definition_id),
            "{name} must appear in the bill of materials"
        );
    }
    let stud_definition = definition_id_of(&shell, "Front stud");
    let stud_row = projection
        .bom
        .rows
        .iter()
        .find(|row| row.definition_id == stud_definition)
        .expect("the studs must appear as one pooled row");
    assert_eq!(stud_row.quantity, STUD_INSTANCES as usize);
    assert_eq!(stud_row.dimensions.height_mm, WALL_HEIGHT_MM);
    for row in &projection.bom.rows {
        assert_eq!(row.validation_state, ValidationState::Passed);
    }

    // Every member must get a drawing, and every drawing three views.
    assert_eq!(
        projection.drawings.drawings.len(),
        visible_definitions.len()
    );
    for drawing in &projection.drawings.drawings {
        assert_eq!(drawing.views.len(), 3);
        assert_eq!(drawing.dimensions.len(), 3);
    }

    // Every member must resolve to a machining operation, none left unresolved.
    assert_eq!(
        projection.manufacturing.operations.len(),
        visible_definitions.len()
    );
    assert!(projection.manufacturing.unresolved_sources.is_empty());
    for operation in &projection.manufacturing.operations {
        assert_eq!(operation.kind, GeneralManufacturingKind::Stock);
    }

    // All three exports must succeed against the document they describe.
    let bom = String::from_utf8(projection.bom_export(&snapshot).unwrap()).unwrap();
    assert!(bom.contains(&format!("quantity={}", STUD_INSTANCES)));
    let drawings = String::from_utf8(projection.drawing_svg(&snapshot).unwrap()).unwrap();
    assert!(drawings.contains("ketchup.general-drawing-svg.v3"));
    let manufacturing =
        String::from_utf8(projection.manufacturing_export(&snapshot).unwrap()).unwrap();
    assert!(manufacturing.contains("kind=stock"));

    publish_artifact("timber-frame-house-bom.txt", bom.as_bytes());
    publish_artifact("timber-frame-house-drawings.svg", drawings.as_bytes());
    publish_artifact(
        "timber-frame-house-manufacturing.txt",
        manufacturing.as_bytes(),
    );
}

#[test]
fn measured_house_change_assembly_fabrication_step_and_reopen_workflow() {
    let total_started = Instant::now();
    let build_started = Instant::now();
    let (mut shell, _baseline_revision, _baseline_digest) = build_timber_frame_house();
    let model_build = build_started.elapsed();

    let stud_definition = definition_id_of(&shell, "Front stud");
    let changed_definition = definition_id_of(&shell, "Front sheathing");
    let changed_body = body_feature_id_of(&shell, "Front sheathing");
    let change_started = Instant::now();
    let change_request = "Reduce the front sheathing thickness to 16 mm";
    build_step(
        &mut shell,
        change_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::SetDimension {
                feature_id: changed_body.0,
                constraint_id: None,
                value_mm: 16.0,
            }],
        },
    );
    let parametric_change = change_started.elapsed();
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrences()
            .filter(|occurrence| occurrence.definition_id() == stud_definition)
            .count(),
        STUD_INSTANCES as usize
    );

    let assembly_started = Instant::now();
    let final_snapshot = shell.app().document_snapshot();
    let assembly_scene = final_snapshot.scene_query();
    let assembly = assembly_started.elapsed();
    assert_eq!(assembly_scene.len(), 19);
    assert_eq!(
        final_snapshot.definitions().count(),
        HOUSE_MEMBERS.len() + 1
    );
    assert_eq!(
        assembly_scene
            .iter()
            .filter(|occurrence| occurrence.definition_id == stud_definition)
            .count(),
        STUD_INSTANCES as usize
    );
    let final_revision = final_snapshot.revision_id();
    let final_digest = final_snapshot.canonical_digest();
    let final_undo_steps = shell.app().undo_step_count();

    let exact_started = Instant::now();
    let packages = exact_member_packages(&shell, &final_snapshot);
    let registry = ExactResultRegistry::accept(&final_snapshot, packages).unwrap();
    let exact_rebuild = exact_started.elapsed();
    let changed_bounds = registry
        .get_render(&final_snapshot, changed_definition)
        .unwrap()
        .bounds_mm();
    let changed_extents =
        std::array::from_fn::<_, 3, _>(|axis| changed_bounds[1][axis] - changed_bounds[0][axis]);
    assert!(
        changed_extents
            .iter()
            .any(|extent| (*extent - 16.0).abs() < 1.0e-9),
        "changed exact sheathing extents were {changed_extents:?}"
    );

    let tolerance = TolerancePolicy::default();
    let participants = final_snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| {
            GeneralBodyParticipant::accept(
                &final_snapshot,
                &registry,
                InstancePath::root(occurrence.occurrence_id),
                tolerance,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let cases = (0..participants.len())
        .map(|index| {
            GeneralClearanceCase::new(
                participants[index].clone(),
                participants[(index + 1) % participants.len()].clone(),
                0.0,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let fabrication_started = Instant::now();
    let validation = general_report(&final_snapshot, &cases, tolerance);
    assert_eq!(validation.state, ValidationState::Passed);
    let projection = ketchup_manufacturing::fabrication::project_general_fabrication(
        &final_snapshot,
        &registry,
        &cases,
        &validation,
        tolerance,
    )
    .unwrap();
    let bom = projection.bom_export(&final_snapshot).unwrap();
    let drawing = projection.drawing_svg(&final_snapshot).unwrap();
    let manufacturing = projection.manufacturing_export(&final_snapshot).unwrap();
    let fabrication = fabrication_started.elapsed();
    assert_eq!(projection.bom.envelope.status, ProjectionStatus::Complete);
    assert_eq!(
        projection.drawings.envelope.status,
        ProjectionStatus::Complete
    );
    assert_eq!(
        projection.manufacturing.envelope.status,
        ProjectionStatus::Complete
    );

    let exact_by_definition = registry.render_by_definition(&final_snapshot);
    let model = final_snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| {
            (
                exact_by_definition[&occurrence.definition_id]
                    .as_ref()
                    .clone(),
                occurrence.transform,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(model.len(), participants.len());
    let directory = tempfile::tempdir().unwrap();
    let step_path = directory.path().join("changed-house.step");
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let step_export_started = Instant::now();
    worker
        .export_current_model_step(&final_snapshot, &model, &step_path)
        .unwrap();
    let step_export = step_export_started.elapsed();
    let step_bytes = std::fs::read(&step_path).unwrap();
    assert!(step_bytes.windows(9).any(|window| window == b"ISO-10303"));
    let step_import_started = Instant::now();
    let imported = worker
        .inspect_step_import_with_cancellation(
            &step_path,
            &ketchup_model::graph::sha256_hex(&step_bytes),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
    let step_import = step_import_started.elapsed();
    assert!(imported.solid_count >= model.len() as u32);
    assert!(imported.volume_mm3 > 0.0);

    assert!(shell.app_mut().undo());
    let stale_snapshot = shell.app().document_snapshot();
    assert_ne!(stale_snapshot.canonical_digest(), final_digest);
    let stale_path = directory.path().join("stale.step");
    std::fs::write(&stale_path, b"preserved destination").unwrap();
    assert!(
        worker
            .export_current_model_step(&stale_snapshot, &model, &stale_path)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&stale_path).unwrap(),
        b"preserved destination"
    );
    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().document_revision(), final_revision);
    assert_eq!(shell.app().canonical_digest(), final_digest);
    assert_eq!(shell.app().undo_step_count(), final_undo_steps);

    let native_path = directory.path().join("changed-house.ketchup");
    let native_save_started = Instant::now();
    persistence::save_atomic(&native_path, &final_snapshot).unwrap();
    let native_save = native_save_started.elapsed();
    let native_reopen_started = Instant::now();
    let reopened = persistence::load_file(&native_path).unwrap();
    let native_reopen = native_reopen_started.elapsed();
    assert!(reopened.is_editable());
    assert_eq!(reopened.snapshot().revision_id(), final_revision);
    assert_eq!(reopened.snapshot().canonical_digest(), final_digest);
    assert_eq!(reopened.snapshot().occurrences().count(), model.len());

    let metrics = serde_json::json!({
        "schema": "ketchup.production-acceptance-workflow.v1",
        "claim_scope": "local Ketchup end-to-end evidence; no SolidWorks comparison measured",
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "occurrences": model.len(),
        "definitions": final_snapshot.definitions().count(),
        "exact_packages": registry.render_values(&final_snapshot).count(),
        "model_build_ms": model_build.as_secs_f64() * 1000.0,
        "parametric_change_ms": parametric_change.as_secs_f64() * 1000.0,
        "assembly_scene_projection_ms": assembly.as_secs_f64() * 1000.0,
        "exact_rebuild_ms": exact_rebuild.as_secs_f64() * 1000.0,
        "fabrication_ms": fabrication.as_secs_f64() * 1000.0,
        "step_export_ms": step_export.as_secs_f64() * 1000.0,
        "step_import_ms": step_import.as_secs_f64() * 1000.0,
        "native_save_ms": native_save.as_secs_f64() * 1000.0,
        "native_reopen_ms": native_reopen.as_secs_f64() * 1000.0,
        "total_ms": total_started.elapsed().as_secs_f64() * 1000.0,
        "bom_bytes": bom.len(),
        "drawing_bytes": drawing.len(),
        "manufacturing_bytes": manufacturing.len(),
        "step_bytes": step_bytes.len(),
        "native_bytes": std::fs::metadata(&native_path).unwrap().len(),
        "stale_export_preserved_destination": true,
        "undo_redo_restored_identity": true
    });
    let json = serde_json::to_string_pretty(&metrics).unwrap();
    eprintln!("KETCHUP_PRODUCTION_ACCEPTANCE={json}");
    if let Some(path) = ketchup_test_env::report_path("production_acceptance.json") {
        std::fs::write(path, format!("{json}\n")).unwrap();
    }
}

/// Manufacturability is not structural sanity. The same house must also be
/// held up by gravity: every member either sits on the ground or rests on
/// something that does. This is the check the acceptance proof was missing —
/// it is what catches a ridge beam hanging in mid-air.
#[test]
fn the_timber_frame_house_must_stand_up_under_gravity() {
    let (shell, _revision, _digest) = build_timber_frame_house();
    let snapshot = shell.app().document_snapshot();
    let tolerance = TolerancePolicy::default();

    let packages = exact_member_packages(&shell, &snapshot);
    let registry = ExactResultRegistry::accept(&snapshot, packages).unwrap();

    // Only the sill plates are founded on the ground. Everything else has to
    // earn its support through real contact with something already supported.
    let sill_definitions = [
        "Sill plate front",
        "Sill plate back",
        "Sill plate left",
        "Sill plate right",
    ]
    .map(|name| definition_id_of(&shell, name));
    let participants = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| {
            let body = GeneralBodyParticipant::accept(
                &snapshot,
                &registry,
                InstancePath::root(occurrence.occurrence_id),
                tolerance,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{} must be an accepted general body: {error:?}",
                    occurrence.occurrence_name
                )
            });
            let grounded = occurrence.occurrence_name.starts_with("Sill plate")
                || sill_definitions.contains(&occurrence.definition_id);
            GravitySupportParticipant::new(body, "house", grounded)
        })
        .collect::<Vec<_>>();

    let input = GravitySupportInput::new(participants, [0.0, 0.0, -9.81]).unwrap();
    let validator = BuiltinGravitySupportValidator::new(tolerance);
    let policy = gravity_support_validation_policy();
    let bytes = gravity_support_input_bytes(&input);
    let invocation = ValidationInvocation::bind(
        &snapshot,
        validator.descriptor(),
        &policy,
        Vec::new(),
        &bytes,
    );
    let report = validator.invoke(ValidationExecution {
        snapshot: &snapshot,
        invocation,
        policy: &policy,
        input: &input,
    });

    let unsupported = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "gravity.unsupported")
        .map(|diagnostic| {
            let evidence = diagnostic.evidence.clone();
            match snapshot.scene_query().into_iter().find(|occurrence| {
                evidence.starts_with(&format!("body=occurrence:{};", occurrence.occurrence_id.0))
            }) {
                Some(occurrence) => occurrence.occurrence_name,
                None => evidence,
            }
        })
        .collect::<Vec<_>>();
    assert!(
        unsupported.is_empty(),
        "the Assistant-built house has {} member(s) floating in mid-air: {:#?}",
        unsupported.len(),
        unsupported
    );
    // The rafter is pitched: it bears on the plate arris and the ridge face
    // along lines, so no positive bearing area can be proven for it. It is
    // reported as unproven rather than supported; every other member is proven.
    let unproven = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "gravity.contact-unproven")
        .filter_map(|diagnostic| {
            snapshot.scene_query().into_iter().find(|occurrence| {
                diagnostic
                    .evidence
                    .starts_with(&format!("body=occurrence:{};", occurrence.occurrence_id.0))
            })
        })
        .map(|occurrence| occurrence.occurrence_name)
        .collect::<Vec<_>>();
    assert_eq!(unproven, ["Rafter"], "{:#?}", report.diagnostics);
    assert_eq!(
        report.state,
        ValidationState::NotEvaluated,
        "{:#?}",
        report.diagnostics
    );
}

/// A wall and its rectangular opening are authored as one reviewed Assistant
/// program. Every later operation addresses an earlier result by typed local
/// identity; no guessed host ID or named-shape branch is involved.
#[test]
fn assistant_opening_survives_accesskit_undo_redo_validation_and_reopen() {
    let request = "Create a wall with a rectangular window opening";
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("assistant-wall-opening.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    let (wall_entities, wall_constraints) = rectangle(HOUSE_LENGTH_MM, WALL_HEIGHT_MM);
    let corners = [
        [2_400.0, 600.0],
        [3_600.0, 600.0],
        [3_600.0, 2_000.0],
        [2_400.0, 2_000.0],
    ];
    let window_entities = (0..4)
        .map(|index| AssistantSketchEntity::Line {
            id: index as u64 + 1,
            start_mm: corners[index],
            end_mm: corners[(index + 1) % corners.len()],
        })
        .collect::<Vec<_>>();
    let window_constraints = (0..4)
        .map(|index| {
            let id = index as u64 + 1;
            if index % 2 == 0 {
                AssistantSketchConstraint::Horizontal { id, entity_id: id }
            } else {
                AssistantSketchConstraint::Vertical { id, entity_id: id }
            }
        })
        .collect::<Vec<_>>();
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let program = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::CreatePart {
                holes: Vec::new(),
                pockets: Vec::new(),
                name: "Front sheathing".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities: wall_entities,
                constraints: wall_constraints,
                feature: AssistantCadPartFeature::Extrusion {
                    distance_mm: SHEATHING_THICKNESS_MM,
                },
                translation_mm: [1_000.0, 1_000.0, 1_000.0],
                rotation: None,
            },
            AssistantCadEditOperation::CreateSketch {
                definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                    0,
                    AssistantCadProgramFeatureOutput::Definition,
                )),
                name: "Window opening".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities: window_entities,
                constraints: window_constraints,
            },
            AssistantCadEditOperation::AppendFeature {
                definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                    0,
                    AssistantCadProgramFeatureOutput::Definition,
                )),
                name: "Window pocket".to_owned(),
                feature: AssistantCadBodyFeature::Pocket {
                    target_feature_id: AssistantCadFeatureReference::ProgramOutput(output(
                        0,
                        AssistantCadProgramFeatureOutput::BodyFeature,
                    )),
                    profile_feature_id: AssistantCadFeatureReference::ProgramOutput(output(
                        1,
                        AssistantCadProgramFeatureOutput::SketchFeature,
                    )),
                    depth_mm: SHEATHING_THICKNESS_MM,
                },
            },
        ],
    };

    let baseline_digest = shell.app().canonical_digest();
    build_step(&mut shell, request, program);
    let committed_digest = shell.app().canonical_digest();
    let committed_revision = shell.app().document_revision();
    let sheathing = definition_id_of(&shell, "Front sheathing");
    let committed = shell.app().document_snapshot();
    let pocket_id = committed
        .definitions()
        .find(|definition| definition.id() == sheathing)
        .unwrap()
        .feature_ids()
        .iter()
        .copied()
        .find(|id| {
            matches!(
                committed.feature(*id).unwrap().kind(),
                FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(_),
                    extent: FeatureExtent::Blind(_),
                    operation: PadOperation::Cut { .. },
                    ..
                })
            )
        })
        .expect("the wall must retain its generic Sketch-driven pocket");
    let FeatureKind::Pad(PadSpec {
        profile: PadProfile::Feature(profile),
        extent: FeatureExtent::Blind(_),
        operation: PadOperation::Cut { target, .. },
        ..
    }) = committed.feature(pocket_id).unwrap().kind()
    else {
        unreachable!("the pocket kind was just matched")
    };
    assert!(matches!(
        committed.feature(*target).unwrap().kind(),
        FeatureKind::Pad(_)
    ));
    assert!(matches!(
        committed.feature(*profile).unwrap().kind(),
        FeatureKind::Sketch(_)
    ));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().document_revision(), committed_revision);

    let reopened = shell.app().document_snapshot();
    let reopened_pocket = reopened
        .feature(pocket_id)
        .expect("the pocket identity must survive Save/Open");
    assert!(matches!(
        reopened_pocket.kind(),
        FeatureKind::Pad(PadSpec {
            profile: PadProfile::Feature(_),
            extent: FeatureExtent::Blind(_),
            operation: PadOperation::Cut { .. },
            ..
        })
    ));
    let graph = ExactBRepGraph::from_snapshot(&reopened, sheathing, pocket_id)
        .expect("the reopened wall opening must remain an exact editable chain");
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    let expected_volume =
        (HOUSE_LENGTH_MM * WALL_HEIGHT_MM - 1_200.0 * 1_400.0) * SHEATHING_THICKNESS_MM;
    assert!((package.volume_mm3 - expected_volume).abs() <= 1.0e-6);

    let run = shell.catalog().text("validators-run");
    shell.click_role_and_label(Role::Button, &shell.catalog().text("validators-title"));
    for validator in ASSISTANT_VALIDATOR_IDS {
        if validator != "collision" {
            let label = shell.catalog().text(&format!("validator-{validator}-name"));
            shell.click_role_and_label(Role::CheckBox, &label);
        }
    }
    shell.click_button_label(&run);
    let deadline = Instant::now() + Duration::from_secs(60);
    while shell.app().validator_panel_pending() {
        assert!(
            Instant::now() < deadline,
            "the validator panel never finished"
        );
        shell.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = shell
        .app()
        .validator_panel_report()
        .expect("the reopened exact model must produce a validator report");
    assert_eq!(report.revision, committed_revision);
    assert_eq!(report.canonical_digest, committed_digest);
    assert_eq!(report.executed, vec!["collision"]);
    assert_eq!(report.state, "passed", "{report:#?}");
    assert!(report.complete, "{report:#?}");
    assert_eq!(report.issue_count, 0, "{report:#?}");
}

/// The generic Assistant can feed its own closed-profile and open-path Sketches
/// into the same reviewed exact Sweep path used by canonical fixtures.
#[test]
fn assistant_authored_sketches_feed_a_reviewed_exact_sweep() {
    let clear_request = "Clear the document for a sweep test";
    let profile_request = "Create a square sweep profile part";
    let path_request = "Create an open straight sweep path in that part";
    let sweep_request = "Sweep the authored square profile along the authored path";
    let mut shell = Shell::new();

    let existing = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .collect::<Vec<_>>();
    build_step(
        &mut shell,
        clear_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Delete {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: existing,
                },
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            }],
        },
    );
    assert_eq!(shell.app().document_snapshot().occurrences().count(), 0);

    let (entities, constraints) = rectangle(20.0, 20.0);
    build_step(
        &mut shell,
        profile_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                holes: Vec::new(),
                pockets: Vec::new(),
                name: "Sweep profile".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Yz,
                },
                entities,
                constraints,
                feature: AssistantCadPartFeature::Extrusion { distance_mm: 5.0 },
                translation_mm: [0.0; 3],
                rotation: None,
            }],
        },
    );
    let definition_id = definition_id_of(&shell, "Sweep profile");

    build_step(
        &mut shell,
        path_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreateSketch {
                definition_id: definition_id.0.into(),
                name: "Sweep path".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xy,
                },
                entities: vec![AssistantSketchEntity::Line {
                    id: 1,
                    start_mm: [0.0, 0.0],
                    end_mm: [200.0, 0.0],
                }],
                constraints: vec![AssistantSketchConstraint::Horizontal {
                    id: 1,
                    entity_id: 1,
                }],
            }],
        },
    );

    let authored = shell.app().document_snapshot();
    let profile_id = authored
        .features()
        .find(|feature| feature.name() == "Sweep profile sketch")
        .expect("the generic part must own its Assistant-authored profile sketch")
        .id();
    let path_id = authored
        .features()
        .find(|feature| feature.name() == "Sweep path")
        .expect("the generic CreateSketch operation must author the path")
        .id();
    assert!(matches!(
        authored.feature(profile_id).unwrap().kind(),
        FeatureKind::Sketch(_)
    ));
    assert!(matches!(
        authored.feature(path_id).unwrap().kind(),
        FeatureKind::Sketch(_)
    ));

    let sweep_program = AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::AppendFeature {
            definition_id: definition_id.0.into(),
            name: "Assistant sketch sweep".to_owned(),
            feature: AssistantCadBodyFeature::Sweep {
                profile_feature_id: profile_id.0,
                path_feature_id: path_id.0,
            },
        }],
    };
    build_step(&mut shell, sweep_request, sweep_program);

    let committed = shell.app().document_snapshot();
    let sweep_id = committed
        .features()
        .find(|feature| feature.name() == "Assistant sketch sweep")
        .expect("the reviewed Assistant step must create the Sweep")
        .id();
    assert!(matches!(
        committed.feature(sweep_id).unwrap().kind(),
        FeatureKind::Sweep { profile, path, .. }
            if *profile == profile_id && *path == path_id
    ));
    let graph = ExactBRepGraph::from_snapshot(&committed, definition_id, sweep_id).unwrap();
    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened.snapshot(), definition_id, sweep_id).unwrap(),
        graph
    );
    // The sketched square is swept from where it was drawn, along the
    // authored path: exactly a 200 mm bar over the square.
    assert!(
        graph
            .profiles
            .iter()
            .all(|profile| profile.source_feature_id == profile_id.0)
    );
    let expected_bounds = [[0.0, 0.0, 0.0], [200.0, 20.0, 20.0]];
    let [envelope_min, envelope_max] = graph.producer_bounds_mm().unwrap().unwrap();
    assert!(
        (0..3).all(|axis| envelope_min[axis] <= expected_bounds[0][axis]
            && envelope_max[axis] >= expected_bounds[1][axis])
    );
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert!((package.volume_mm3 - 80_000.0).abs() <= 1.0e-6);
    assert_eq!(package.bounds_mm, expected_bounds);
}

/// Unlike fixture-backed geometry, this starts from an Assistant-authored body.
/// The headless hook publishes the worker result deterministically; this test
/// isolates the generic Assistant context/planner/commit path for a real fillet.
#[test]
fn assistant_authored_part_accepts_a_host_issued_topology_fillet() {
    let clear_request = "Clear the document for a fillet test";
    let part_request = "Create a small rectangular part";
    let fillet_request = "Fillet one current edge of that part by 2 mm";
    let mut shell = Shell::new();

    let existing = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .collect::<Vec<_>>();
    build_step(
        &mut shell,
        clear_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::Delete {
                selector: AssistantCadEntitySelector::Occurrences {
                    occurrence_ids: existing,
                },
                dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
            }],
        },
    );

    let (entities, constraints) = rectangle(40.0, 30.0);
    build_step(
        &mut shell,
        part_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                holes: Vec::new(),
                pockets: Vec::new(),
                name: "Fillet block".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xy,
                },
                entities,
                constraints,
                feature: AssistantCadPartFeature::Extrusion { distance_mm: 20.0 },
                translation_mm: [0.0; 3],
                rotation: None,
            }],
        },
    );

    let definition_id = definition_id_of(&shell, "Fillet block");
    let body_id = body_feature_id_of(&shell, "Fillet block");
    let base_graph =
        ExactBRepGraph::from_snapshot(&shell.app().document_snapshot(), definition_id, body_id)
            .expect("the Assistant-authored block must compile into an exact graph");
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let base_package = worker.evaluate_exact_brep_graph(&base_graph).unwrap();
    let base_volume_mm3 = base_package.volume_mm3;
    let edge_reference = base_package
        .topological_references
        .iter()
        .find(|reference| {
            reference
                .producer_element_id
                .starts_with("generated-result/edge/")
        })
        .expect("the exact block must publish host-issued edge references")
        .lineage_digest
        .clone();
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(base_package))
    );
    build_step(
        &mut shell,
        fillet_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::AppendFeature {
                definition_id: definition_id.0.into(),
                name: "Rounded edge".to_owned(),
                feature: AssistantCadBodyFeature::TopologyFillet {
                    target_feature_id: body_id.0,
                    edge_reference_ids: vec![edge_reference.clone()],
                    radius_mm: 2.0,
                    radius_stations: Vec::new(),
                },
            }],
        },
    );

    let committed = shell.app().document_snapshot();
    let fillet_id = committed
        .features()
        .find(|feature| feature.name() == "Rounded edge")
        .expect("the reviewed fillet must be canonical")
        .id();
    assert!(matches!(
        committed.feature(fillet_id).unwrap().kind(),
        FeatureKind::EdgeFinish {
            target,
            edges,
            kind: EdgeFinishKind::Fillet,
            amount,
            ..
        } if *target == body_id
            && edges.len() == 1
            && edges[0].topological().is_some_and(|edge| edge.lineage_digest == edge_reference)
            && amount.millimetres() == 2.0
    ));
    let fillet_graph = ExactBRepGraph::from_snapshot(&committed, definition_id, fillet_id)
        .expect("the Assistant fillet must remain an exact graph");
    let fillet_package = worker.evaluate_exact_brep_graph(&fillet_graph).unwrap();
    assert!(fillet_package.volume_mm3 > 0.0);
    assert!(
        fillet_package.volume_mm3 < base_volume_mm3,
        "a convex edge fillet must remove exact material from the block"
    );
    assert_eq!(fillet_package.topology_counts[4], 1);
}
