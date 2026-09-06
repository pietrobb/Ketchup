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
use ketchup_app::AssistantMessageRole;
use ketchup_core::assistant_sidecar::{
    AssistantCadBodyFeature, AssistantCadBooleanOperation, AssistantCadDeletePolicy,
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantCadPartFeature, AssistantCadRotation, AssistantChatResult, AssistantDistribution,
    AssistantPrincipalPlane, AssistantSketchConstraint, AssistantSketchEntity,
    AssistantWorkplaneSpec,
};
use ketchup_core::document::{DefinitionId, FeatureId, FeatureKind, InstancePath, Snapshot};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFeatureChainRequest, ExactResultRegistry,
};
use ketchup_core::exact_validation::{
    BuiltinGeneralBodyValidator, BuiltinGravitySupportValidator, GeneralBodyParticipant,
    GeneralClearanceCase, GravitySupportInput, GravitySupportParticipant, general_body_input_bytes,
    general_body_validation_policy, gravity_support_input_bytes, gravity_support_validation_policy,
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

fn json_string_ending_with<'a>(value: &'a serde_json::Value, suffix: &str) -> Option<&'a str> {
    match value {
        serde_json::Value::String(text) => text.ends_with(suffix).then_some(text),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| json_string_ending_with(value, suffix)),
        serde_json::Value::Object(values) => values
            .values()
            .find_map(|value| json_string_ending_with(value, suffix)),
        _ => None,
    }
}

fn wait_for_live_assistant_proposal(shell: &mut Shell) {
    let confirm = shell.catalog().text("assistant-confirm");
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        shell.step();
        if shell.app().assistant_proposal().is_some() && shell.has_visible_label(&confirm) {
            return;
        }
        assert!(
            !shell
                .app()
                .assistant_messages()
                .iter()
                .any(|message| message.role == AssistantMessageRole::Error),
            "live assistant returned an error: {:?}",
            shell.app().assistant_messages()
        );
        assert!(
            Instant::now() < deadline,
            "live assistant did not reach proposal review: {:?}",
            shell.app().assistant_messages()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
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
        "Stand the gable posts, carry the ridge beam on them and add a rafter sloping up to it",
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
                translation_mm: [0.0, 0.0, PLATE_THICKNESS_MM],
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
        &transport,
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

    assert_eq!(transport.remaining_responses(), 0);
    (shell, transport, baseline_revision, baseline_digest)
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

/// Prove the production OAuth Assistant can author and extend real exact
/// structure across revision-bound turns instead of replaying a Rust-authored
/// program. The complete house remains a larger opt-in acceptance proof.
#[test]
#[ignore = "requires the production OAuth binary, login and live GPT-5.6 requests"]
fn live_oauth_assistant_builds_a_supported_house_frame_across_turns() {
    let mut shell = Shell::new();
    shell.app_mut().set_assistant_diagnostics_enabled(true);
    let handshake = shell.app().assistant_handshake();
    assert_eq!(handshake.distribution, AssistantDistribution::PrivateOauth);
    assert_eq!(handshake.provider, "codex-oauth");
    assert_eq!(handshake.model, "gpt-5.6-sol");

    let baseline_revision = shell.app().document_revision();
    let baseline_digest = shell.app().canonical_digest();
    assert!(shell.app().document_snapshot().occurrences().count() > 0);

    let request = "Use one typed cad_edit_program, not model_intent and not prose alone. First delete every existing occurrence listed in the current document context with remove_references so the site is empty. Then create exactly four separate rectangular extruded parts on the XY plane, all dimensions in millimetres: 'Foundation beam' is 4000 by 400 and extruded 300 at translation [0,0,0]; 'Left post' is 200 by 200 and extruded 2500 at [0,0,300]; 'Right post' is 200 by 200 and extruded 2500 at [3800,0,300]; 'Header beam' is 4000 by 200 and extruded 200 at [0,0,2800]. Omit the optional rotation field entirely for every part. Draw the Foundation beam rectangle with corners [0,0], [4000,0], [4000,400], [0,400]; both post rectangles with corners [0,0], [200,0], [200,200], [0,200]; and the Header beam rectangle with corners [0,0], [4000,0], [4000,200], [0,200]. Close each rectangle with four line entities and set constraints to an empty array for every part. Do not emit any constraint object. Do not add any other part and do not approximate with mesh geometry.";
    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(egui::Key::Enter);
    wait_for_live_assistant_proposal(&mut shell);

    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
    assert!(shell.app().assistant_messages().iter().any(|message| {
        message.role == AssistantMessageRole::Assistant
            && message.source.contains("gpt-5.6-sol")
            && !message.text.trim().is_empty()
    }));
    let diagnostics = shell
        .app()
        .last_assistant_api_diagnostics()
        .expect("the live provider response must retain bounded diagnostics");
    assert_eq!(diagnostics.provider, "codex-oauth");
    assert_eq!(diagnostics.model, "gpt-5.6-sol");
    assert!(diagnostics.input_tokens > 0 && diagnostics.output_tokens > 0);
    let provider_response: serde_json::Value = serde_json::from_str(&diagnostics.response_text)
        .expect("the captured provider response must be JSON");
    assert_eq!(
        provider_response["cad_edit_program"]["operations"]
            .as_array()
            .expect("GPT-5.6 must return a typed CAD program")
            .len(),
        5
    );

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), baseline_revision + 1);
    let committed = shell.app().document_snapshot();
    assert_eq!(committed.occurrences().count(), 4);
    let expected = [
        ("Foundation beam", [4_000.0, 400.0, 300.0], [0.0, 0.0, 0.0]),
        ("Left post", [200.0, 200.0, 2_500.0], [0.0, 0.0, 300.0]),
        ("Right post", [200.0, 200.0, 2_500.0], [3_800.0, 0.0, 300.0]),
        ("Header beam", [4_000.0, 200.0, 200.0], [0.0, 0.0, 2_800.0]),
    ];
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    for &(name, size, translation) in &expected {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("live-authored {name} must exist"));
        let transform = occurrence.transform();
        let matrix = transform.matrix();
        assert_eq!([matrix[3], matrix[7], matrix[11]], translation);
        assert_eq!(
            [
                [matrix[0], matrix[1], matrix[2]],
                [matrix[4], matrix[5], matrix[6]],
                [matrix[8], matrix[9], matrix[10]],
            ],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );

        let definition_id = definition_id_of(&shell, name);
        let body = body_feature_id_of(&shell, name);
        let graph = ExactBRepGraph::from_snapshot(&committed, definition_id, body)
            .unwrap_or_else(|error| panic!("live-authored {name} must compile exactly: {error}"));
        let package = worker
            .evaluate_exact_brep_graph(&graph)
            .unwrap_or_else(|error| panic!("live-authored {name} must evaluate in OCCT: {error}"));
        let expected_volume = size.iter().product::<f64>();
        assert!((package.volume_mm3 - expected_volume).abs() <= expected_volume * 1.0e-9);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], size]);
        assert_eq!(package.topology_counts, [8, 12, 6, 1, 1]);
    }

    let first_identities = expected.map(|(name, _, _)| {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("live-authored {name} must exist before turn two"));
        (
            name,
            occurrence.id(),
            occurrence.definition_id(),
            occurrence.transform(),
        )
    });
    let first_revision = shell.app().document_revision();
    let first_digest = shell.app().canonical_digest();
    let second_request = "Use one typed cad_edit_program, not model_intent and not prose alone. Extend the existing front portal into a supported 4000 by 4000 mm house frame by creating exactly six additional separate rectangular extruded parts on the XY plane. Keep every existing occurrence and do not modify or delete it. 'Rear foundation beam' is 4000 by 400 and extruded 300 at translation [0,3600,0]. 'Rear left post' and 'Rear right post' are each 200 by 200 and extruded 2500 at [0,3800,300] and [3800,3800,300]. 'Rear header beam' is 4000 by 200 and extruded 200 at [0,3800,2800]. 'Left top tie' and 'Right top tie' are each 200 by 3600 and extruded 200 at [0,200,2800] and [3800,200,2800]. Draw each 4000 by 400 rectangle with corners [0,0], [4000,0], [4000,400], [0,400]; each 200 by 200 rectangle with corners [0,0], [200,0], [200,200], [0,200]; the 4000 by 200 rectangle with corners [0,0], [4000,0], [4000,200], [0,200]; and each 200 by 3600 rectangle with corners [0,0], [200,0], [200,3600], [0,3600]. Close every rectangle with four line entities whose objects use exactly the keys type, id, start_mm and end_mm; never use start or end. Set constraints to an empty array and omit the optional rotation field entirely for every part. Do not emit any constraint object, add any other part, or approximate with mesh geometry.";
    shell.focus_text_input(&input);
    shell.type_text(second_request);
    shell.press_key(egui::Key::Enter);
    wait_for_live_assistant_proposal(&mut shell);

    assert_eq!(shell.app().document_revision(), first_revision);
    assert_eq!(shell.app().canonical_digest(), first_digest);
    let diagnostics = shell
        .app()
        .last_assistant_api_diagnostics()
        .expect("the second live provider response must retain bounded diagnostics");
    assert_eq!(diagnostics.provider, "codex-oauth");
    assert_eq!(diagnostics.model, "gpt-5.6-sol");
    assert!(diagnostics.input_tokens > 0 && diagnostics.output_tokens > 0);
    let second_provider_message =
        json_string_ending_with(&diagnostics.request_payload, second_request)
            .expect("the provider payload must contain the second request");
    let (document_context, provider_prompt) = second_provider_message
        .strip_prefix("<document-context>")
        .and_then(|message| message.split_once("</document-context>\n\n"))
        .expect("the second request must carry a serialized document context");
    assert_eq!(provider_prompt, second_request);
    let document_context: serde_json::Value = serde_json::from_str(document_context)
        .expect("the second provider context must remain valid JSON");
    assert_eq!(document_context["revision"], first_revision);
    assert_eq!(document_context["canonical_digest"], first_digest);
    assert_eq!(document_context["occurrence_count"], 4);
    let conversation = document_context["conversation"]
        .as_array()
        .expect("the second context must retain the first app turn");
    assert!(
        conversation.iter().any(|message| {
            message["role"] == "user" && message["text"].as_str() == Some(request)
        })
    );
    assert!(conversation.iter().any(|message| {
        message["role"] == "assistant"
            && message["text"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty())
    }));
    for (name, _, _) in expected {
        assert!(
            document_context["occurrences"]
                .as_array()
                .expect("the context must list the first-turn occurrences")
                .iter()
                .any(|occurrence| occurrence["name"] == name),
            "the second provider context must expose {name}"
        );
    }

    let provider_response: serde_json::Value = serde_json::from_str(&diagnostics.response_text)
        .expect("the second captured provider response must be JSON");
    let operations = provider_response["cad_edit_program"]["operations"]
        .as_array()
        .expect("GPT-5.6 must return a second typed CAD program");
    assert_eq!(operations.len(), 6);
    assert!(
        operations
            .iter()
            .all(|operation| operation["operation"] == "create_part"),
        "turn two may only append the six requested parts"
    );

    shell.click_row(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().document_revision(), first_revision + 1);
    assert_ne!(shell.app().canonical_digest(), first_digest);
    let committed = shell.app().document_snapshot();
    assert_eq!(committed.occurrences().count(), 10);
    for (name, occurrence_id, definition_id, transform) in first_identities {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("turn two must preserve {name}"));
        assert_eq!(occurrence.id(), occurrence_id);
        assert_eq!(occurrence.definition_id(), definition_id);
        assert_eq!(occurrence.transform(), transform);
    }
    let extension = [
        (
            "Rear foundation beam",
            [4_000.0, 400.0, 300.0],
            [0.0, 3_600.0, 0.0],
        ),
        (
            "Rear left post",
            [200.0, 200.0, 2_500.0],
            [0.0, 3_800.0, 300.0],
        ),
        (
            "Rear right post",
            [200.0, 200.0, 2_500.0],
            [3_800.0, 3_800.0, 300.0],
        ),
        (
            "Rear header beam",
            [4_000.0, 200.0, 200.0],
            [0.0, 3_800.0, 2_800.0],
        ),
        (
            "Left top tie",
            [200.0, 3_600.0, 200.0],
            [0.0, 200.0, 2_800.0],
        ),
        (
            "Right top tie",
            [200.0, 3_600.0, 200.0],
            [3_800.0, 200.0, 2_800.0],
        ),
    ];
    for (name, size, translation) in extension {
        let occurrence = committed
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap_or_else(|| panic!("live-authored {name} must exist"));
        let transform = occurrence.transform();
        let matrix = transform.matrix();
        assert_eq!([matrix[3], matrix[7], matrix[11]], translation);
        assert_eq!(
            [
                [matrix[0], matrix[1], matrix[2]],
                [matrix[4], matrix[5], matrix[6]],
                [matrix[8], matrix[9], matrix[10]],
            ],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        let definition_id = definition_id_of(&shell, name);
        let body = body_feature_id_of(&shell, name);
        let graph = ExactBRepGraph::from_snapshot(&committed, definition_id, body)
            .unwrap_or_else(|error| panic!("live-authored {name} must compile exactly: {error}"));
        let package = worker
            .evaluate_exact_brep_graph(&graph)
            .unwrap_or_else(|error| panic!("live-authored {name} must evaluate in OCCT: {error}"));
        let expected_volume = size.iter().product::<f64>();
        assert!((package.volume_mm3 - expected_volume).abs() <= expected_volume * 1.0e-9);
        assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], size]);
        assert_eq!(package.topology_counts, [8, 12, 6, 1, 1]);
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("live-oauth-house-frame.ketchup");
    persistence::save_atomic(&path, &committed).unwrap();
    let outcome = persistence::load_file(&path).unwrap();
    assert!(outcome.is_editable());
    assert_eq!(
        outcome.snapshot().canonical_digest(),
        committed.canonical_digest()
    );
    publish_artifact(
        "live-oauth-house-frame.ketchup",
        &std::fs::read(path).unwrap(),
    );

    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), first_revision);
    assert_eq!(shell.app().canonical_digest(), first_digest);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().document_revision(), baseline_revision);
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
}

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
    publish_artifact("timber-frame-house.ketchup", &std::fs::read(&path).unwrap());

    // The whole build must unwind step by step back to the empty document.
    while shell.app().document_revision() > baseline_revision {
        assert!(shell.app_mut().undo());
    }
    assert_eq!(shell.app().canonical_digest(), baseline_digest);
}

/// Copy a produced artifact next to the operator when `KETCHUP_HOUSE_OUT` is set,
/// so the proof house can be opened and inspected instead of only asserted on.
fn publish_artifact(name: &str, bytes: &[u8]) {
    let Some(directory) = std::env::var_os("KETCHUP_HOUSE_OUT") else {
        return;
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(name), bytes).unwrap();
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

    publish_artifact("timber-frame-house-bom.txt", bom.as_bytes());
    publish_artifact("timber-frame-house-drawings.svg", drawings.as_bytes());
    publish_artifact(
        "timber-frame-house-manufacturing.txt",
        manufacturing.as_bytes(),
    );
}

/// Manufacturability is not structural sanity. The same house must also be
/// held up by gravity: every member either sits on the ground or rests on
/// something that does. This is the check the acceptance proof was missing —
/// it is what catches a ridge beam hanging in mid-air.
#[test]
fn the_timber_frame_house_must_stand_up_under_gravity() {
    let (shell, _transport, _revision, _digest) = build_timber_frame_house();
    let snapshot = shell.app().document_snapshot();
    let tolerance = TolerancePolicy::default();

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
    assert_eq!(
        report.state,
        ValidationState::Passed,
        "{:#?}",
        report.diagnostics
    );
}

/// The window opening this scenario needs was blocked by two generality limits.
/// The first one is gone: a Pocket now consumes a Sketch profile authored by the
/// same generic program, so the Assistant can cut an opening into a part it just
/// created. The second still fails closed with a specific machine code and no
/// mutation, and this test pins that split.
#[test]
fn assistant_cuts_an_opening_but_cannot_yet_boolean_two_parts_it_created() {
    let requests = [
        "Sheath the front wall",
        "Cut a window opening into that sheathing",
    ];
    let transport = Arc::new(ScriptedAssistantTransport::new(requests.map(|request| {
        (
            request.to_owned(),
            AssistantChatResult {
                message: "Review the sheathing.".to_owned(),
                model_intent: None,
            },
        )
    })));
    let (entities, constraints) = rectangle(HOUSE_LENGTH_MM, WALL_HEIGHT_MM);
    let mut shell = Shell::with_assistant_transport(transport.clone());
    build_step(
        &mut shell,
        &transport,
        requests[0],
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

    // Lifted limit: a Pocket now accepts the Sketch profile the same program
    // authored, so the opening goes all the way through review to commit.
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
    build_step(&mut shell, &transport, requests[1], sketch_then_pocket);

    // The committed definition really owns the pocket, driven by that sketch and
    // targeting the pad, and the whole part stays an editable exact chain.
    let committed = shell.app().document_snapshot();
    let pocket_id = committed
        .definitions()
        .find(|definition| definition.id() == sheathing)
        .expect("the sheathing definition must survive the cut")
        .feature_ids()
        .iter()
        .copied()
        .find(|id| {
            matches!(
                committed.feature(*id).unwrap().kind(),
                FeatureKind::Pocket { .. }
            )
        })
        .expect("the sheathing must own the window pocket");
    let FeatureKind::Pocket {
        target, profile, ..
    } = committed.feature(pocket_id).unwrap().kind()
    else {
        unreachable!("the pocket kind was just matched")
    };
    assert_eq!(*target, sheathing_pad);
    assert_eq!(profile.0, sheathing_pad.0 + 2);
    ExactBRepGraph::from_snapshot(&committed, sheathing, pocket_id)
        .expect("the pocketed sheathing must compile into an exact graph");

    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    // Limit 2: CreatePart always opens a new definition, no operation appends a
    // second solid to an existing one, and a body created earlier in the same
    // program is not yet visible to a later operation. A Boolean between two
    // Assistant-built bodies is therefore unreachable as well.
    // The pad the CreatePart below would author: workplane, sketch, then pad.
    let next_part_pad = committed
        .features()
        .map(|feature| feature.id().0)
        .max()
        .expect("the document owns features")
        + 3;
    let second_part = AssistantCadEditProgram {
        operations: vec![
            timber("Window block", 1_200.0, 1_400.0, 100.0, [0.0; 3], None),
            AssistantCadEditOperation::AppendFeature {
                definition_id: sheathing.0,
                name: "Window cut".to_owned(),
                feature: AssistantCadBodyFeature::Boolean {
                    operation: AssistantCadBooleanOperation::Cut,
                    target_feature_id: sheathing_pad.0,
                    tool_feature_id: next_part_pad,
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
