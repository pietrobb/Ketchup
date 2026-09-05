//! The operator must be able to see what the validators say without asking the
//! Assistant first: which validators exist, what each one checks, a button that
//! runs them on the current document, and findings that name concrete parts.

mod harness;

use eframe::egui::accesskit::Role;
use harness::Shell;
use ketchup_app::KetchupApp;
use ketchup_core::assistant_sidecar::{AssistantBoxIntent, AssistantModelIntent};

fn empty_intent(boxes: Vec<AssistantBoxIntent>) -> AssistantModelIntent {
    AssistantModelIntent {
        replace_scene: true,
        boxes,
        translations: Vec::new(),
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

fn build_two_colliding_columns(shell: &mut Shell) {
    assert!(
        shell
            .app_mut()
            .prepare_assistant_model_intent(empty_intent(vec![
                AssistantBoxIntent {
                    name: "Column A".to_owned(),
                    size_mm: [400.0, 400.0, 2_500.0],
                    origin_mm: [0.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
                AssistantBoxIntent {
                    name: "Column B".to_owned(),
                    size_mm: [400.0, 400.0, 2_500.0],
                    origin_mm: [200.0, 0.0, 0.0],
                    subtract_boxes: Vec::new(),
                },
            ]))
    );
    assert!(shell.app_mut().confirm_assistant_proposal());
}

#[test]
fn the_validator_panel_names_every_validator_and_says_what_it_checks() {
    for catalog in [
        ketchup_interaction::LocaleCatalog::english(),
        ketchup_interaction::LocaleCatalog::slovak(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.settle();
        assert_eq!(KetchupApp::validator_ids().len(), 9);
        for validator in KetchupApp::validator_ids() {
            let name = shell.catalog().text(&format!("validator-{validator}-name"));
            let what = shell.catalog().text(&format!("validator-{validator}-what"));
            assert!(
                !name.starts_with("validator-") && !what.starts_with("validator-"),
                "{validator} must be localized, got {name} / {what}"
            );
            assert!(
                shell.has_role_and_label(Role::CheckBox, &name),
                "the panel must offer {validator} as a runnable checkbox: {name}"
            );
            assert!(
                shell.has_visible_label(&what),
                "the panel must explain what {validator} checks: {what}"
            );
        }
        let run = shell.catalog().text("validators-run");
        assert!(
            shell.has_role_and_label(Role::Button, &run),
            "the panel must offer a run button: {run}"
        );
        assert!(shell.has_visible_label(&shell.catalog().text("validators-not-run")));
    }
}

#[test]
fn running_the_panel_reports_findings_that_name_the_offending_parts() {
    let mut shell = Shell::new();
    build_two_colliding_columns(&mut shell);
    shell.settle();
    assert!(shell.app().validator_panel_report().is_none());

    let revision_before = shell.app().document_revision();
    let digest_before = shell.app().canonical_digest();
    let undo_before = shell.app().undo_step_count();
    shell.click_button_label(&shell.catalog().text("validators-run"));
    shell.settle();

    let report = shell
        .app()
        .validator_panel_report()
        .expect("clicking run must produce a report")
        .clone();
    assert_eq!(report.revision, revision_before);
    assert_eq!(report.canonical_digest, digest_before);
    assert_eq!(report.executed, KetchupApp::validator_ids().to_vec());
    assert_eq!(report.state, "failed");
    assert!(report.issue_count >= 1);

    let collision = report
        .findings
        .iter()
        .find(|finding| finding.validator == "collision")
        .expect("two overlapping columns must be reported as a collision");
    assert_eq!(collision.code, "collision.detected");
    assert_eq!(
        collision.parts,
        vec!["Column A (#2)".to_owned(), "Column B (#3)".to_owned()],
        "a finding must name the concrete parts it refers to"
    );

    // Running validators is observation, not a change.
    assert_eq!(shell.app().document_revision(), revision_before);
    assert_eq!(shell.app().canonical_digest(), digest_before);
    assert_eq!(shell.app().undo_step_count(), undo_before);
}

#[test]
fn the_panel_runs_only_the_validators_the_operator_selected() {
    let mut shell = Shell::new();
    build_two_colliding_columns(&mut shell);
    shell.settle();

    for validator in KetchupApp::validator_ids() {
        if validator != "collision" {
            let name = shell.catalog().text(&format!("validator-{validator}-name"));
            shell.click_role_and_label(Role::CheckBox, &name);
            shell.settle();
        }
    }
    assert_eq!(shell.app().validator_panel_selection(), vec!["collision"]);

    shell.click_button_label(&shell.catalog().text("validators-run"));
    shell.settle();

    let report = shell.app().validator_panel_report().unwrap();
    assert_eq!(report.executed, vec!["collision"]);
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.validator == "collision"),
        "a deselected validator must not report findings"
    );
}
