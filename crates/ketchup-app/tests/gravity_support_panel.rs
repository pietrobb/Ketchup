//! Statics has to catch real nonsense, not excuse itself.
//!
//! Before this, `gravity_support` gave up on any ordinary document because it
//! demanded classification roles and a typed gravity vector that nothing in a
//! freshly built model ever declares — so a frame with a beam hanging in
//! mid-air was reported as "not evaluated" rather than rejected. The roles are
//! now read from what the document already states: every visible solid is a
//! body, and the occurrences the document grounds are the only support seeds.
//! No floor is invented — without a grounded occurrence there is still no seed
//! and the validator stays honestly unevaluated.
//!
//! The same structure is driven through the operator's validator panel twice:
//! rejected by name while the ridge beam floats, accepted once it is lowered
//! onto the posts that carry it.

mod harness;

use eframe::egui::accesskit::Role;
use harness::Shell;
use ketchup_app::ValidatorPanelReport;
use ketchup_core::assistant_sidecar::{
    AssistantBoxIntent, AssistantModelIntent, AssistantTranslationIntent,
};
use ketchup_core::document::OccurrenceId;
use std::collections::BTreeMap;

const FOUNDATION_TOP_MM: f64 = 300.0;
const POST_TOP_MM: f64 = 2_800.0;
/// How far above the posts the ridge beam is left hanging.
const FLOATING_GAP_MM: f64 = 700.0;

fn model_intent(
    replace_scene: bool,
    boxes: Vec<AssistantBoxIntent>,
    translations: Vec<AssistantTranslationIntent>,
) -> AssistantModelIntent {
    AssistantModelIntent {
        replace_scene,
        boxes,
        translations,
        rotations: Vec::new(),
        profile_translations: Vec::new(),
        parameter_edits: Vec::new(),
        linear_arrays: Vec::new(),
        bottles: Vec::new(),
        gable_roofs: Vec::new(),
        staircases: Vec::new(),
        oriented_beams: Vec::new(),
        balloon_texts: Vec::new(),
    }
}

fn timber(name: &str, size_mm: [f64; 3], origin_mm: [f64; 3]) -> AssistantBoxIntent {
    AssistantBoxIntent {
        name: name.to_owned(),
        size_mm,
        origin_mm,
        subtract_boxes: Vec::new(),
    }
}

/// A minimal frame: a foundation on the ground, two posts standing on it, and
/// a ridge beam left hanging above the posts instead of resting on them.
fn build_frame_with_a_floating_ridge(shell: &mut Shell) {
    let post_height_mm = POST_TOP_MM - FOUNDATION_TOP_MM;
    assert!(shell.app_mut().prepare_assistant_model_intent(model_intent(
        true,
        vec![
            timber(
                "Foundation",
                [4_000.0, 400.0, FOUNDATION_TOP_MM],
                [0.0, 0.0, 0.0],
            ),
            timber(
                "Post left",
                [200.0, 200.0, post_height_mm],
                [0.0, 0.0, FOUNDATION_TOP_MM],
            ),
            timber(
                "Post right",
                [200.0, 200.0, post_height_mm],
                [3_800.0, 0.0, FOUNDATION_TOP_MM],
            ),
            timber(
                "Ridge beam",
                [4_000.0, 200.0, 200.0],
                [0.0, 0.0, POST_TOP_MM + FLOATING_GAP_MM],
            ),
        ],
        Vec::new(),
    )));
    assert!(shell.app_mut().confirm_assistant_proposal());
}

fn occurrence_id_of(shell: &Shell, name: &str) -> OccurrenceId {
    shell
        .app()
        .document_snapshot()
        .scene_query()
        .into_iter()
        .find(|occurrence| occurrence.occurrence_name == name)
        .unwrap_or_else(|| panic!("{name} must be in the scene"))
        .occurrence_id
}

/// Ground one occurrence through the document's own explicit grounding fact.
fn ground(shell: &mut Shell, name: &str) {
    let id = occurrence_id_of(shell, name);
    shell.click_role_and_label(Role::Button, &shell.catalog().text("assembly-title"));
    let preview = shell.catalog().format(
        "assembly-preview-ground",
        &BTreeMap::from([("name", name.to_owned())]),
    );
    if shell.has_visible_label(&preview) {
        shell.click_button_label(&preview);
        shell.click_button_label(&shell.catalog().text("assembly-confirm-preview"));
    }
    shell.click_role_and_label(Role::Button, &shell.catalog().text("assembly-title"));
    shell.settle();
    assert!(
        shell.app().document_snapshot().occurrence_is_grounded(id),
        "{name} must be grounded afterwards"
    );
}

/// Run the validator panel the way the operator does, from the dock.
fn run_validators(shell: &mut Shell) -> ValidatorPanelReport {
    let run = shell.catalog().text("validators-run");
    if !shell.has_role_and_label(Role::Button, &run) {
        shell.click_role_and_label(Role::Button, &shell.catalog().text("validators-title"));
        shell.settle();
    }
    shell.click_button_label(&run);
    // The panel validates on a background worker, so the operator watches a
    // spinner until the report lands; the test has to wait for the same thing.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while shell.app().validator_panel_pending() {
        assert!(
            std::time::Instant::now() < deadline,
            "the validator panel never finished"
        );
        shell.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    shell
        .app()
        .validator_panel_report()
        .expect("running the panel must produce a report")
        .clone()
}

/// Why `gravity_support` declined to judge, if it declined at all.
fn gravity_not_evaluated_reason(report: &ValidatorPanelReport) -> Option<String> {
    report
        .not_evaluated
        .iter()
        .find(|(validator, _)| validator == "gravity_support")
        .map(|(_, reason)| reason.clone())
}

fn unsupported_parts(report: &ValidatorPanelReport) -> Vec<String> {
    report
        .findings
        .iter()
        .filter(|finding| finding.validator == "gravity_support")
        .flat_map(|finding| finding.parts.clone())
        .collect()
}

#[test]
fn a_structure_with_nothing_grounded_is_not_silently_approved() {
    let mut shell = Shell::new();
    build_frame_with_a_floating_ridge(&mut shell);
    shell.settle();

    let report = run_validators(&mut shell);
    assert!(
        unsupported_parts(&report).is_empty(),
        "an unevaluated validator must not invent findings"
    );
    let reason = gravity_not_evaluated_reason(&report).expect(
        "without a grounded occurrence there is no support seed, so gravity support must decline",
    );
    assert!(
        !reason.is_empty(),
        "the operator must be told what is missing, got {reason:?}"
    );
}

#[test]
fn the_panel_rejects_a_floating_member_by_name_and_accepts_it_once_it_is_carried() {
    let mut shell = Shell::new();
    build_frame_with_a_floating_ridge(&mut shell);
    shell.settle();
    ground(&mut shell, "Foundation");

    let revision_before = shell.app().document_revision();
    let digest_before = shell.app().canonical_digest();
    let undo_before = shell.app().undo_step_count();

    let rejected = run_validators(&mut shell);
    assert_eq!(
        rejected.state, "failed",
        "a beam hanging {FLOATING_GAP_MM} mm above the posts must be rejected: {rejected:#?}"
    );
    assert_eq!(
        gravity_not_evaluated_reason(&rejected),
        None,
        "gravity support must actually judge a grounded structure, not decline"
    );
    let floating = unsupported_parts(&rejected);
    assert!(
        floating.iter().any(|part| part.starts_with("Ridge beam")),
        "the finding must name the member that is hanging in mid-air, got {floating:#?}"
    );
    assert!(
        !floating.iter().any(|part| part.starts_with("Post")),
        "the posts stand on the grounded foundation and must not be reported, got {floating:#?}"
    );

    // Reading statics is observation, not a change.
    assert_eq!(shell.app().document_revision(), revision_before);
    assert_eq!(shell.app().canonical_digest(), digest_before);
    assert_eq!(shell.app().undo_step_count(), undo_before);

    // Repair: lower the ridge beam onto the posts that are meant to carry it.
    let ridge = occurrence_id_of(&shell, "Ridge beam");
    assert!(shell.app_mut().prepare_assistant_model_intent(model_intent(
        false,
        Vec::new(),
        vec![AssistantTranslationIntent {
            occurrence_id: ridge.0,
            delta_mm: [0.0, 0.0, -FLOATING_GAP_MM],
        }],
    )));
    assert!(shell.app_mut().confirm_assistant_proposal());
    shell.settle();

    let accepted = run_validators(&mut shell);
    assert_eq!(
        gravity_not_evaluated_reason(&accepted),
        None,
        "gravity support must still judge the repaired frame, not decline"
    );
    assert!(
        unsupported_parts(&accepted).is_empty(),
        "once the ridge beam rests on the posts nothing may be unsupported: {accepted:#?}"
    );
}
