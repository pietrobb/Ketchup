//! Program 6 body-aware feature history replayed offscreen through AccessKit.

mod harness;

use eframe::egui::{Key, accesskit::Role};
use harness::{Shell, ctrl};
use ketchup_app::AppCommand;
use ketchup_core::document::{BodyId, DefinitionId, FeatureId, FeatureKind};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFINITION: DefinitionId = DefinitionId(1);
const BODY: BodyId = BodyId(1);
const EXTRUSION: FeatureId = FeatureId(2);

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

fn wait_for_exact_body(shell: &mut Shell) {
    for _ in 0..500 {
        shell.step();
        shell.settle();
        if shell.app().exact_render_body_count() == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "current exact body unavailable: {}",
        shell.app().action_digest()
    );
}

fn open_history(shell: &mut Shell) {
    let title = shell.catalog().text("feature-history-title");
    shell.click_role_and_label(Role::Button, &title);
}

fn stamp(shell: &Shell) -> (u64, String, usize, usize) {
    (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    )
}

fn replace_exact_value(shell: &mut Shell, value: &str) {
    let label = shell.catalog().text("feature-history-exact-value");
    shell.focus_text_input(&label);
    shell.key(Key::A, ctrl());
    shell.type_text(value);
}

fn confirm(shell: &mut Shell) {
    assert!(shell.app().feature_history_preview_pending());
    let label = shell.catalog().text("feature-history-confirm");
    shell.click_button_label(&label);
    assert!(
        !shell.app().feature_history_preview_pending(),
        "{}",
        shell.app().action_digest()
    );
}

#[test]
fn serial_history_panel_edits_cancels_suppresses_resumes_and_undoes_atomically() {
    let mut shell = Shell::new();
    open_history(&mut shell);
    assert_eq!(shell.app().feature_history_selected_body_id(), Some(BODY));
    assert_eq!(
        shell.app().feature_history_selected_feature_id(),
        Some(EXTRUSION)
    );
    let initial = stamp(&shell);

    replace_exact_value(&mut shell, "35");
    let preview_edit = shell.catalog().text("feature-history-preview-edit");
    shell.click_button_label(&preview_edit);
    assert!(shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), initial);
    shell.press_key(Key::Escape);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), initial);

    replace_exact_value(&mut shell, "35");
    shell.click_button_label(&preview_edit);
    assert_eq!(stamp(&shell), initial);
    confirm(&mut shell);
    assert_eq!(shell.app().document_revision(), initial.0 + 1);
    assert_eq!(shell.app().undo_step_count(), initial.2 + 1);
    let snapshot = shell.app().document_snapshot();
    let FeatureKind::Extrusion { height, .. } = snapshot.feature(EXTRUSION).unwrap().kind() else {
        panic!("expected editable Extrusion")
    };
    assert_eq!(
        height.millimetres(),
        35.0,
        "{}",
        shell.app().action_digest()
    );
    let edited_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(EXTRUSION)
            .unwrap()
            .kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 20.0
    ));
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), edited_digest);

    let before_suppress = stamp(&shell);
    let preview_suppress = shell.catalog().text("feature-history-preview-suppress");
    shell.click_button_label(&preview_suppress);
    assert!(shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before_suppress);
    confirm(&mut shell);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, BODY),
        Some(&BTreeSet::from([EXTRUSION]))
    );
    assert_eq!(shell.app().undo_step_count(), before_suppress.2 + 1);
    let suppressed_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, BODY),
        None
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), suppressed_digest);

    let before_resume = stamp(&shell);
    let preview_resume = shell.catalog().text("feature-history-preview-resume");
    shell.click_button_label(&preview_resume);
    assert!(shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before_resume);
    confirm(&mut shell);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, BODY),
        None
    );
    assert_eq!(shell.app().undo_step_count(), before_resume.2 + 1);

    let before_invalid = stamp(&shell);
    replace_exact_value(&mut shell, "not-a-number");
    shell.click_button_label(&preview_edit);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before_invalid);
}

#[test]
fn shared_change_impact_panel_is_accessible_observational_and_commits_atomically() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_body(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-edit", AppCommand::Copy);
    shell.click_menu_command("menu-edit", AppCommand::Paste);
    assert_eq!(shell.app().active_box_count(), 2);
    wait_for_exact_body(&mut shell);
    open_history(&mut shell);

    let before = stamp(&shell);
    replace_exact_value(&mut shell, "35");
    let preview_edit = shell.catalog().text("feature-history-preview-edit");
    shell.click_button_label(&preview_edit);
    assert_eq!(
        shell.app().feature_history_shared_impact_counts(),
        Some([2, 1, 0, 0, 1, 2])
    );
    assert_eq!(stamp(&shell), before);

    let visible = shell
        .catalog()
        .text("feature-history-shared-impact-visible");
    let occurrence = |id: u64| {
        shell.catalog().format(
            "feature-history-shared-impact-occurrence",
            &BTreeMap::from([("id", id.to_string()), ("visibility", visible.clone())]),
        )
    };
    let pending = shell
        .catalog()
        .text("feature-history-shared-impact-export-pending");
    let step = shell
        .catalog()
        .text("feature-history-shared-impact-export-step");
    let stl = shell
        .catalog()
        .text("feature-history-shared-impact-export-stl");
    let expected_labels = [
        shell.catalog().text("feature-history-shared-impact-title"),
        shell.catalog().format(
            "feature-history-shared-impact-occurrences",
            &BTreeMap::from([("items", format!("{}, {}", occurrence(1), occurrence(2)))]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-bodies",
            &BTreeMap::from([("items", "1".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-mates",
            &BTreeMap::from([("items", "—".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-views",
            &BTreeMap::from([("items", "—".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-jobs",
            &BTreeMap::from([("items", "1:2".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-exports",
            &BTreeMap::from([("items", format!("{step}: {pending}, {stl}: {pending}"))]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-diagnostics",
            &BTreeMap::from([
                ("revision", before.0.to_string()),
                ("definition", DEFINITION.0.to_string()),
            ]),
        ),
    ];
    for label in expected_labels {
        assert!(
            shell.has_visible_label(&label),
            "missing shared-impact AccessKit label: {label}"
        );
    }

    shell.press_key(Key::Escape);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before);

    replace_exact_value(&mut shell, "35");
    shell.click_button_label(&preview_edit);
    confirm(&mut shell);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.2 + 1);
    assert_eq!(shell.app().exact_render_bounds()[0][1][2], 35.0);
}

#[test]
fn feature_history_controls_are_complete_localized_accesskit_nodes() {
    for catalog in [
        ketchup_interaction::LocaleCatalog::english(),
        ketchup_interaction::LocaleCatalog::slovak(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        open_history(&mut shell);
        for key in ["feature-history-definition", "feature-history-parameter"] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(Role::ComboBox, &label),
                "missing localized AccessKit ComboBox {key}: {label}"
            );
        }
        let value = shell.catalog().text("feature-history-exact-value");
        assert!(
            shell.has_role_and_label(Role::TextInput, &value),
            "missing localized AccessKit input: {value}"
        );
        for key in [
            "feature-history-preview-edit",
            "feature-history-preview-suppress",
            "feature-history-preview-resume",
        ] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(Role::Button, &label),
                "missing localized AccessKit button {key}: {label}"
            );
        }
    }
}
