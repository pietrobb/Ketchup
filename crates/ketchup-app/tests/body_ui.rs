//! Program 4 multi-body authoring replayed offscreen through AccessKit.

mod harness;

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{Key, accesskit::Role};
use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::document::{BodyId, BooleanOperation, DefinitionId, FeatureId, FeatureKind};
use ketchup_core::exact_product::ExactFeatureChainRequest;
use ketchup_core::intent::WorkflowIntent;

fn open_body_editor(shell: &mut Shell) {
    let title = shell.catalog().text("body-title");
    shell.click_role_and_label(Role::Button, &title);
}

fn confirm_preview(shell: &mut Shell) {
    assert!(
        shell.app().body_preview_pending(),
        "{}",
        shell.app().action_digest()
    );
    let confirm = shell.catalog().text("body-confirm-preview");
    shell.click_button_label(&confirm);
    assert!(!shell.app().body_preview_pending());
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

fn wait_for_exact_bodies(shell: &mut Shell, expected: usize) {
    for _ in 0..500 {
        shell.step();
        shell.settle();
        if shell.app().exact_render_body_count() == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        shell.app().exact_render_body_count(),
        expected,
        "producers={:?}, digest={}",
        shell.app().exact_current_producer_ids(),
        shell.app().action_digest()
    );
}

fn sorted_exact_bounds(shell: &Shell) -> Vec<[[f64; 3]; 2]> {
    let mut bounds = shell.app().exact_render_bounds();
    bounds.sort_by(|left, right| {
        left[1][2]
            .total_cmp(&right[1][2])
            .then(left[1][0].total_cmp(&right[1][0]))
            .then(left[1][1].total_cmp(&right[1][1]))
    });
    bounds
}

#[test]
fn body_section_previews_selects_activates_shows_hides_creates_and_combines_atomically() {
    const DEFINITION: DefinitionId = DefinitionId(1);
    const BASE_BODY: BodyId = BodyId(1);
    const TOOL_BODY: BodyId = BodyId(2);

    let mut shell = Shell::new();
    open_body_editor(&mut shell);
    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    let initial_undo = shell.app().undo_step_count();

    let preview_create = shell.catalog().text("body-preview-create");
    shell.click_button_label(&preview_create);
    assert!(shell.app().body_preview_pending());
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert_eq!(shell.app().undo_step_count(), initial_undo);
    let cancel = shell.catalog().text("body-cancel-preview");
    shell.click_button_label(&cancel);
    assert!(!shell.app().body_preview_pending());
    assert_eq!(shell.app().canonical_digest(), initial_digest);

    shell.click_button_label(&preview_create);
    confirm_preview(&mut shell);
    let created = shell.app().document_snapshot();
    let definition = created.definition(DEFINITION).unwrap();
    assert_eq!(definition.bodies().count(), 2);
    assert_eq!(definition.active_body_id(), TOOL_BODY);
    assert!(matches!(
        created
            .feature(*definition.feature_ids().last().unwrap())
            .unwrap()
            .kind(),
        FeatureKind::Extrusion { .. }
    ));
    assert_eq!(shell.app().undo_step_count(), initial_undo + 1);

    let tool_name = shell.catalog().format(
        "body-default-name",
        &std::collections::BTreeMap::from([("number", "2".to_owned())]),
    );
    let hide = shell.catalog().format(
        "body-preview-hide",
        &std::collections::BTreeMap::from([("name", tool_name.clone())]),
    );
    let before_hide = shell.app().canonical_digest();
    shell.click_button_label(&hide);
    assert_eq!(shell.app().canonical_digest(), before_hide);
    shell.click_button_label(&cancel);
    assert!(
        shell
            .app()
            .document_snapshot()
            .definition(DEFINITION)
            .unwrap()
            .body(TOOL_BODY)
            .unwrap()
            .visible()
    );
    shell.click_button_label(&hide);
    confirm_preview(&mut shell);
    assert!(
        !shell
            .app()
            .document_snapshot()
            .definition(DEFINITION)
            .unwrap()
            .body(TOOL_BODY)
            .unwrap()
            .visible()
    );

    let show = shell.catalog().format(
        "body-preview-show",
        &std::collections::BTreeMap::from([("name", tool_name)]),
    );
    shell.click_button_label(&show);
    confirm_preview(&mut shell);
    assert!(
        shell
            .app()
            .document_snapshot()
            .definition(DEFINITION)
            .unwrap()
            .body(TOOL_BODY)
            .unwrap()
            .visible()
    );

    let base_name = "Body".to_owned();
    let select = shell.catalog().format(
        "body-select",
        &std::collections::BTreeMap::from([("name", base_name.clone())]),
    );
    let before_select = shell.app().canonical_digest();
    let undo_before_select = shell.app().undo_step_count();
    shell.click_button_label(&select);
    assert_eq!(shell.app().selected_body_id(), Some(BASE_BODY));
    assert_eq!(shell.app().canonical_digest(), before_select);
    assert_eq!(shell.app().undo_step_count(), undo_before_select);

    let activate = shell.catalog().format(
        "body-preview-activate",
        &std::collections::BTreeMap::from([("name", base_name)]),
    );
    shell.click_button_label(&activate);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .definition(DEFINITION)
            .unwrap()
            .active_body_id(),
        TOOL_BODY
    );
    confirm_preview(&mut shell);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .definition(DEFINITION)
            .unwrap()
            .active_body_id(),
        BASE_BODY
    );

    let combine = shell.catalog().text("body-preview-combine");
    let before_combine = shell.app().canonical_digest();
    let undo_before_combine = shell.app().undo_step_count();
    shell.click_button_label(&combine);
    assert_eq!(shell.app().canonical_digest(), before_combine);
    assert_eq!(shell.app().undo_step_count(), undo_before_combine);
    shell.click_button_label(&cancel);
    assert_eq!(shell.app().canonical_digest(), before_combine);
    shell.click_button_label(&combine);
    confirm_preview(&mut shell);

    let combined = shell.app().document_snapshot();
    let definition = combined.definition(DEFINITION).unwrap();
    let result = combined
        .feature(*definition.feature_ids().last().unwrap())
        .unwrap();
    assert!(matches!(
        result.kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Union,
            ..
        }
    ));
    assert_eq!(
        definition
            .feature_body_ownership(result.id())
            .unwrap()
            .output_body_id(),
        Some(BASE_BODY)
    );
    assert_eq!(definition.active_body_id(), BASE_BODY);
    assert_eq!(definition.body(TOOL_BODY).unwrap().consumed_by(), None);
    assert_eq!(shell.app().undo_step_count(), undo_before_combine + 1);
}

#[test]
fn serial_multibody_accesskit_workflow_recomputes_undoes_and_round_trips() {
    const DEFINITION: DefinitionId = DefinitionId(1);
    const BASE_BODY: BodyId = BodyId(1);
    const TOOL_BODY: BodyId = BodyId(2);
    const BASE_EXTRUSION: FeatureId = FeatureId(2);
    const TOOL_EXTRUSION: FeatureId = FeatureId(3);

    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("body-ui.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 1);
    open_body_editor(&mut shell);

    let create = shell.catalog().text("body-preview-create");
    shell.click_button_label(&create);
    confirm_preview(&mut shell);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(
        shell.app().exact_current_producer_ids(),
        vec![BASE_EXTRUSION, TOOL_EXTRUSION]
    );
    assert_eq!(
        sorted_exact_bounds(&shell),
        vec![
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
        ]
    );
    assert_eq!(shell.app().instanced_scene_triangle_count(), 24);

    let tool_name = shell.catalog().format(
        "body-default-name",
        &std::collections::BTreeMap::from([("number", "2".to_owned())]),
    );
    let hide = shell.catalog().format(
        "body-preview-hide",
        &std::collections::BTreeMap::from([("name", tool_name.clone())]),
    );
    shell.click_button_label(&hide);
    confirm_preview(&mut shell);
    wait_for_exact_bodies(&mut shell, 1);
    let show = shell.catalog().format(
        "body-preview-show",
        &std::collections::BTreeMap::from([("name", tool_name)]),
    );
    shell.click_button_label(&show);
    confirm_preview(&mut shell);
    wait_for_exact_bodies(&mut shell, 2);

    let select_base = shell.catalog().format(
        "body-select",
        &std::collections::BTreeMap::from([("name", "Body".to_owned())]),
    );
    shell.click_button_label(&select_base);
    let activate_base = shell.catalog().format(
        "body-preview-activate",
        &std::collections::BTreeMap::from([("name", "Body".to_owned())]),
    );
    shell.click_button_label(&activate_base);
    confirm_preview(&mut shell);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .definition(DEFINITION)
            .unwrap()
            .active_body_id(),
        BASE_BODY
    );

    open_body_editor(&mut shell);
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: TOOL_EXTRUSION,
                value_text: "120".to_owned(),
            })
    );
    shell.settle();
    let assistant_confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&assistant_confirm);
    assert!(
        shell.app().assistant_proposal().is_none(),
        "{}",
        shell.app().action_digest()
    );
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(TOOL_EXTRUSION)
            .unwrap()
            .kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 120.0
    ));
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(
        sorted_exact_bounds(&shell),
        vec![
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
            [[0.0, 0.0, 0.0], [100.0, 60.0, 120.0]],
        ]
    );
    let edited_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(
        sorted_exact_bounds(&shell),
        vec![
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
        ]
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().canonical_digest(), edited_digest);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    wait_for_exact_bodies(&mut shell, 2);

    open_body_editor(&mut shell);
    let operation = shell.catalog().text("body-operation");
    shell.click_role_and_label(Role::ComboBox, &operation);
    let intersect = shell.catalog().text("body-boolean-intersect");
    shell.click_button_label(&intersect);
    let combine = shell.catalog().text("body-preview-combine");
    shell.click_button_label(&combine);
    confirm_preview(&mut shell);
    wait_for_exact_bodies(&mut shell, 2);
    let combined_digest = shell.app().canonical_digest();
    let combined = shell.app().document_snapshot();
    let definition = combined.definition(DEFINITION).unwrap();
    let result = combined
        .feature(*definition.feature_ids().last().unwrap())
        .unwrap();
    assert!(matches!(
        result.kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Intersect,
            ..
        }
    ));
    assert_eq!(definition.body(TOOL_BODY).unwrap().consumed_by(), None);
    assert_eq!(shell.app().instanced_scene_triangle_count(), 24);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    wait_for_exact_bodies(&mut shell, 2);
    assert!(!matches!(
        shell
            .app()
            .document_snapshot()
            .feature(result.id())
            .map(|feature| feature.kind()),
        Some(FeatureKind::Boolean { .. })
    ));
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().canonical_digest(), combined_digest);

    let persisted_revision = shell.app().document_revision();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file(), "{}", shell.app().action_digest());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), combined_digest);
    assert_eq!(shell.app().document_revision(), persisted_revision);
    assert!(!shell.app().can_undo());
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().instanced_scene_triangle_count(), 24);

    let failing_worker = directory.path().join(if cfg!(windows) {
        "body-worker.exe"
    } else {
        "body-worker"
    });
    std::fs::copy(exact_worker_path(), &failing_worker).unwrap();
    shell
        .app_mut()
        .connect_exact_worker(&failing_worker)
        .unwrap();
    wait_for_exact_bodies(&mut shell, 2);
    let create = shell.catalog().text("body-preview-create");
    if !shell.has_role_and_label(Role::Button, &create) {
        open_body_editor(&mut shell);
    }
    shell.click_button_label(&create);
    confirm_preview(&mut shell);
    wait_for_exact_bodies(&mut shell, 3);
    let failed_body_feature = FeatureId(5);
    let before_request = ExactFeatureChainRequest::from_snapshot_for_producer(
        &shell.app().document_snapshot(),
        DEFINITION,
        failed_body_feature,
    )
    .unwrap();

    std::fs::write(&failing_worker, b"not an executable").unwrap();
    open_body_editor(&mut shell);
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: failed_body_feature,
                value_text: "120".to_owned(),
            })
    );
    shell.settle();
    shell.click_row(&assistant_confirm);
    let after_failed_edit = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );
    let after_request = ExactFeatureChainRequest::from_snapshot_for_producer(
        &shell.app().document_snapshot(),
        DEFINITION,
        failed_body_feature,
    )
    .unwrap();
    assert_eq!(f64::from_bits(after_request.height_bits), 120.0);
    assert_ne!(
        after_request.canonical_input_digest,
        before_request.canonical_input_digest
    );
    for _ in 0..20 {
        shell.step();
        shell.settle();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        after_failed_edit
    );
    assert_eq!(
        shell.app().exact_current_producer_ids(),
        vec![FeatureId(4), TOOL_EXTRUSION]
    );
    assert_eq!(
        sorted_exact_bounds(&shell),
        vec![
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
            [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
        ]
    );
}

#[test]
fn invalid_selection_and_stale_confirmation_are_fail_closed() {
    let mut shell = Shell::new();
    open_body_editor(&mut shell);
    let initial = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );

    let combine = shell.catalog().text("body-preview-combine");
    shell.click_button_label(&combine);
    assert!(!shell.app().body_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        initial
    );

    let create = shell.catalog().text("body-preview-create");
    shell.click_button_label(&create);
    assert!(shell.app().body_preview_pending());
    open_body_editor(&mut shell);
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: DefinitionId(1),
                name: "Intervening body edit".to_owned(),
            })
    );
    shell.settle();
    let assistant_confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&assistant_confirm);
    let after_intervening_edit = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );

    open_body_editor(&mut shell);
    let confirm = shell.catalog().text("body-confirm-preview");
    shell.click_button_label(&confirm);
    assert!(!shell.app().body_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        after_intervening_edit
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .definition(DefinitionId(1))
            .unwrap()
            .bodies()
            .count(),
        1
    );
}

#[test]
fn body_controls_are_complete_localized_accesskit_nodes() {
    for catalog in [
        ketchup_interaction::LocaleCatalog::english(),
        ketchup_interaction::LocaleCatalog::slovak(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        open_body_editor(&mut shell);
        for key in [
            "body-definition",
            "body-name",
            "body-feature-name",
            "body-source-feature",
            "body-target",
            "body-tool",
            "body-operation",
        ] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_visible_label(&label)
                    || shell.has_role_and_label(Role::ComboBox, &label)
                    || shell.has_role_and_label(Role::TextInput, &label),
                "missing localized AccessKit control {key}: {label}"
            );
        }
        for key in ["body-preview-create", "body-preview-combine"] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(Role::Button, &label),
                "missing localized AccessKit button {key}: {label}"
            );
        }
        let operation = shell.catalog().text("body-operation");
        shell.click_role_and_label(Role::ComboBox, &operation);
        for key in [
            "body-boolean-cut",
            "body-boolean-union",
            "body-boolean-intersect",
        ] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(Role::Button, &label),
                "missing localized Boolean option {key}: {label}"
            );
        }
        shell.press_key(Key::Escape);
    }
}
