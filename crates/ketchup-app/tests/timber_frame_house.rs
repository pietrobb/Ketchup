//! End-to-end AI-native acceptance proof: a timber-frame house built from an
//! empty document through the generic Assistant CAD edit program only.
//!
//! No named-shape intent, no fixture, no mesh shortcut. Every step is a natural
//! language request that the host compiles into canonical commands, reviews as
//! a proposal, and commits as exactly one undo step.
//!
//! The second test pins the two generality limits this scenario hit, so that a
//! later change which lifts them fails loudly instead of silently.
mod harness;

use eframe::egui;
use harness::{ScriptedAssistantTransport, Shell};
use ketchup_core::assistant_sidecar::{
    AssistantCadBodyFeature, AssistantCadBooleanOperation, AssistantCadDeletePolicy,
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCadPartFeature, AssistantCadRotation, AssistantChatResult, AssistantPrincipalPlane,
    AssistantSketchConstraint, AssistantSketchEntity, AssistantWorkplaneSpec,
};
use ketchup_core::document::{DefinitionId, FeatureId, FeatureKind, InstancePath, Snapshot};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFeatureChainRequest, ExactResultRegistry,
};
use ketchup_core::exact_validation::{
    BuiltinGeneralBodyValidator, GeneralBodyParticipant, GeneralClearanceCase,
    general_body_input_bytes, general_body_validation_policy,
};
use ketchup_core::fabrication::{GeneralManufacturingKind, ProjectionStatus};
use ketchup_core::persistence;
use ketchup_core::prismatic::TolerancePolicy;
use ketchup_core::validation::{
    HostNeutralValidator, ValidationExecution, ValidationInvocation, ValidationReport,
    ValidationState,
};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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

fn wait_for_assistant_proposal(shell: &mut Shell) {
    let confirm = shell.catalog().text("assistant-confirm");
    for _ in 0..200 {
        shell.step();
        if shell.app().assistant_proposal().is_some() && shell.has_visible_label(&confirm) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "scripted assistant response did not reach accessible proposal review: {:?}",
        shell.app().assistant_messages()
    );
}

/// Send one natural-language request, prove preview mutates nothing, confirm it,
/// and prove the commit is exactly one revision and one undo step.
fn build_step(
    shell: &mut Shell,
    transport: &Arc<ScriptedAssistantTransport>,
    request: &str,
    program: AssistantCadEditProgram,
) {
    transport.queue_cad_edit_program(request, program);
    let revision_before = shell.app().document_revision();
    let digest_before = shell.app().canonical_digest();
    let undo_before = shell.app().undo_step_count();

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_assistant_proposal(shell);

    assert_eq!(
        shell.app().document_revision(),
        revision_before,
        "{request}: preview must not change the revision"
    );
    assert_eq!(
        shell.app().canonical_digest(),
        digest_before,
        "{request}: preview must not change the canonical digest"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        undo_before,
        "{request}: preview must not add an undo step"
    );

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(
        shell.app().document_revision(),
        revision_before + 1,
        "{request}: confirmation must add exactly one revision"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        undo_before + 1,
        "{request}: confirmation must add exactly one undo step"
    );
    assert_ne!(
        shell.app().canonical_digest(),
        digest_before,
        "{request}: confirmation must change the canonical digest"
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

/// Drive the whole frame from an empty document through the Assistant, and
/// return the shell plus the baseline the build started from.
fn build_timber_frame_house() -> (Shell, Arc<ScriptedAssistantTransport>, u64, String) {
    let requests = [
        "Clear the document so the site is empty",
        "Lay the sill plate ring for a 6000 by 4000 mm timber frame house",
        "Raise the first stud on the front wall",
        "Repeat that stud every 625 mm and cap the wall with a top plate",
        "Sheath the front wall with an 18 mm panel",
        "Lay the ridge beam and add a rafter sloping up to it",
    ];
    let transport = Arc::new(ScriptedAssistantTransport::new(requests.map(|request| {
        (
            request.to_owned(),
            AssistantChatResult {
                message: "Review the timber frame step.".to_owned(),
                model_intent: None,
            },
        )
    })));
    let mut shell = Shell::with_assistant_transport(transport.clone());
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
        &transport,
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
        &transport,
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
        &transport,
        requests[2],
        AssistantCadEditProgram {
            operations: vec![timber(
                "Front stud",
                TIMBER_WIDTH_MM,
                TIMBER_DEPTH_MM,
                WALL_HEIGHT_MM,
                [0.0, 0.0, PLATE_THICKNESS_MM],
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
        &transport,
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

    // 4. Front sheathing standing on the XZ plane.
    let (sheathing_entities, sheathing_constraints) = rectangle(HOUSE_LENGTH_MM, WALL_HEIGHT_MM);
    build_step(
        &mut shell,
        &transport,
        requests[4],
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                name: "Front sheathing".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities: sheathing_entities,
                constraints: sheathing_constraints,
                feature: AssistantCadPartFeature::Extrusion {
                    distance_mm: SHEATHING_THICKNESS_MM,
                },
                translation_mm: [0.0, -SHEATHING_THICKNESS_MM, PLATE_THICKNESS_MM],
                rotation: None,
            }],
        },
    );

    // 5. Ridge beam and one rafter placed by an arbitrary finite rotation.
    let eaves_height = PLATE_THICKNESS_MM + WALL_HEIGHT_MM + PLATE_THICKNESS_MM;
    let rafter_run = HOUSE_WIDTH_MM / 2.0;
    let rafter_length = rafter_run.hypot(RIDGE_RISE_MM);
    let rafter_pitch_degrees = RIDGE_RISE_MM.atan2(rafter_run).to_degrees();
    build_step(
        &mut shell,
        &transport,
        requests[5],
        AssistantCadEditProgram {
            operations: vec![
                timber(
                    "Ridge beam",
                    HOUSE_LENGTH_MM,
                    TIMBER_WIDTH_MM,
                    2.0 * TIMBER_WIDTH_MM,
                    [
                        0.0,
                        HOUSE_WIDTH_MM / 2.0 - TIMBER_WIDTH_MM / 2.0,
                        eaves_height + RIDGE_RISE_MM,
                    ],
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
                        angle_degrees: -rafter_pitch_degrees,
                    }),
                ),
            ],
        },
    );

    assert_eq!(transport.remaining_responses(), 0);
    (shell, transport, baseline_revision, baseline_digest)
}

/// Every structural member the Assistant authored, by name.
const HOUSE_MEMBERS: [&str; 9] = [
    "Sill plate front",
    "Sill plate back",
    "Sill plate left",
    "Sill plate right",
    "Front stud",
    "Top plate front",
    "Front sheathing",
    "Ridge beam",
    "Rafter",
];

/// Prove the built frame is exact, editable, savable and undoable.
#[test]
fn assistant_builds_a_timber_frame_house_from_an_empty_document() {
    let (mut shell, _transport, baseline_revision, baseline_digest) = build_timber_frame_house();

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

    // The whole build must unwind step by step back to the empty document.
    while shell.app().document_revision() > baseline_revision {
        assert!(shell.app_mut().undo());
    }
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
}

fn exact_worker_path() -> PathBuf {
    let name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let colocated = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(name);
    if colocated.is_file() {
        colocated
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug")
            .join(name)
    }
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
    let (shell, _transport, _revision, _digest) = build_timber_frame_house();
    let snapshot = shell.app().document_snapshot();
    let tolerance = TolerancePolicy::default();

    // Solve every member exactly, through the same worker the app uses.
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let packages = HOUSE_MEMBERS
        .iter()
        .map(|name| {
            let definition_id = definition_id_of(&shell, name);
            let request = ExactFeatureChainRequest::from_snapshot(&snapshot, definition_id)
                .unwrap_or_else(|error| panic!("{name} must yield an exact request: {error}"));
            let package = worker
                .evaluate_rectangle(&request)
                .unwrap_or_else(|error| panic!("{name} must solve exactly: {error}"));
            Arc::new(ExactBodyPackage::from(package))
        })
        .collect::<Vec<_>>();
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

    let projection = ketchup_core::fabrication::project_general_fabrication(
        &snapshot, &registry, &cases, &report, tolerance,
    )
    .unwrap();
    assert_eq!(
        projection,
        ketchup_core::fabrication::project_general_fabrication(
            &snapshot, &registry, &cases, &report, tolerance,
        )
        .unwrap(),
        "the manufacturable handoff must regenerate deterministically"
    );

    // One bill-of-materials row per distinct member, with the studs pooled.
    assert_eq!(projection.bom.envelope.status, ProjectionStatus::Complete);
    assert_eq!(projection.bom.rows.len(), visible_definitions.len());
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
    assert!(drawings.contains("ketchup.general-drawing-svg.v1"));
    let manufacturing =
        String::from_utf8(projection.manufacturing_export(&snapshot).unwrap()).unwrap();
    assert!(manufacturing.contains("kind=stock"));
}

/// The window opening this scenario needs cannot be expressed today. Both
/// blocking limits fail closed with a specific machine code and no mutation.
#[test]
fn assistant_cannot_yet_cut_an_opening_into_a_part_it_created() {
    let request = "Sheath the front wall";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the sheathing.".to_owned(),
            model_intent: None,
        },
    )]));
    let (entities, constraints) = rectangle(HOUSE_LENGTH_MM, WALL_HEIGHT_MM);
    let mut shell = Shell::with_assistant_transport(transport.clone());
    build_step(
        &mut shell,
        &transport,
        request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::CreatePart {
                name: "Front sheathing".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities,
                constraints,
                feature: AssistantCadPartFeature::Extrusion {
                    distance_mm: SHEATHING_THICKNESS_MM,
                },
                translation_mm: [0.0, 0.0, 0.0],
                rotation: None,
            }],
        },
    );

    let sheathing = definition_id_of(&shell, "Front sheathing");
    let sheathing_pad = body_feature_id_of(&shell, "Front sheathing");
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    // Limit 1: a Pocket profile must be a legacy Profile feature, but the only
    // profile the generic program can author is a Sketch, so an opening cut into
    // an Assistant-created part is unreachable.
    let (window_entities, window_constraints) = rectangle(1_200.0, 1_400.0);
    let sketch_then_pocket = AssistantCadEditProgram {
        operations: vec![
            AssistantCadEditOperation::CreateSketch {
                definition_id: sheathing.0,
                name: "Window opening".to_owned(),
                workplane: AssistantWorkplaneSpec::Principal {
                    plane: AssistantPrincipalPlane::Xz,
                },
                entities: window_entities,
                constraints: window_constraints,
            },
            AssistantCadEditOperation::AppendFeature {
                definition_id: sheathing.0,
                name: "Window pocket".to_owned(),
                feature: AssistantCadBodyFeature::Pocket {
                    target_feature_id: sheathing_pad.0,
                    // The sketch this same program just authored.
                    profile_feature_id: sheathing_pad.0 + 2,
                    depth_mm: SHEATHING_THICKNESS_MM,
                },
            },
        ],
    };
    let rejection = shell
        .app()
        .plan_assistant_cad_edit_program(&sketch_then_pocket)
        .expect_err("a sketch-profiled pocket must fail closed");
    assert_eq!(rejection.code, "canonical.invalid_feature_ownership");
    assert_eq!(rejection.operation, "append_feature");
    assert!(rejection.retryable);

    // Limit 2: CreatePart always opens a new definition, no operation appends a
    // second solid to an existing one, and a body created earlier in the same
    // program is not yet visible to a later operation. A Boolean between two
    // Assistant-built bodies is therefore unreachable as well.
    let second_part = AssistantCadEditProgram {
        operations: vec![
            timber("Window block", 1_200.0, 1_400.0, 100.0, [0.0; 3], None),
            AssistantCadEditOperation::AppendFeature {
                definition_id: sheathing.0,
                name: "Window cut".to_owned(),
                feature: AssistantCadBodyFeature::Boolean {
                    operation: AssistantCadBooleanOperation::Cut,
                    target_feature_id: sheathing_pad.0,
                    tool_feature_id: sheathing_pad.0 + 3,
                },
            },
        ],
    };
    let rejection = shell
        .app()
        .plan_assistant_cad_edit_program(&second_part)
        .expect_err("a Boolean against a body from the same program must fail closed");
    assert_eq!(rejection.code, "canonical.feature_not_found");
    assert_eq!(rejection.operation, "append_feature");

    // Neither rejection may touch the document.
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert!(shell.app().assistant_proposal().is_none());
}
