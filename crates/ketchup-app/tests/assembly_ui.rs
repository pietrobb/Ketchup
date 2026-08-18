//! Program 3 rigid-assembly authoring replayed offscreen through AccessKit.

mod harness;

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{Key, Modifiers, accesskit::Role};
use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::assembly::{AssemblyMateKind, AssemblySolveStatus};
use ketchup_core::document::{DefinitionId, FeatureId, FeatureKind};
use ketchup_core::intent::WorkflowIntent;
use ketchup_core::persistence;
use ketchup_interaction::LocaleCatalog;

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

fn wait_for_stable_references(shell: &mut Shell) {
    for _ in 0..150 {
        shell.settle();
        if shell.app().exact_stable_reference_count() >= 2 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        shell.app().exact_stable_reference_count() >= 2,
        "the exact worker must publish current selectable assembly references"
    );
}

fn open_assembly_editor(shell: &mut Shell) {
    let title = shell.catalog().text("assembly-title");
    shell.click_role_and_label(Role::Button, &title);
}

fn confirm_preview(shell: &mut Shell) {
    assert!(
        shell.app().assembly_preview_pending(),
        "{}",
        shell.app().action_digest()
    );
    let confirm = shell.catalog().text("assembly-confirm-preview");
    shell.click_button_label(&confirm);
    assert!(!shell.app().assembly_preview_pending());
}

fn insert_occurrence(shell: &mut Shell) {
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo = shell.app().undo_step_count();
    let preview = shell.catalog().text("assembly-preview-insert");
    shell.click_button_label(&preview);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo);
    confirm_preview(shell);
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().undo_step_count(), undo + 1);
}

#[test]
fn rigid_assembly_authoring_is_previewed_atomic_undoable_and_losslessly_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("assembly-ui.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_stable_references(&mut shell);
    open_assembly_editor(&mut shell);

    insert_occurrence(&mut shell);
    insert_occurrence(&mut shell);
    assert_eq!(shell.app().occurrence_count(), 3);

    let snapshot = shell.app().document_snapshot();
    let occurrences = snapshot
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect::<Vec<_>>();
    let first_name = &occurrences[0].1;
    let second_name = &occurrences[1].1;

    let ground = shell.catalog().format(
        "assembly-preview-ground",
        &std::collections::BTreeMap::from([("name", first_name.clone())]),
    );
    let before_ground = shell.app().canonical_digest();
    shell.click_button_label(&ground);
    assert_eq!(shell.app().canonical_digest(), before_ground);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().grounded_occurrence_count(), 1);

    let endpoint_a = shell.catalog().format(
        "assembly-use-endpoint-a",
        &std::collections::BTreeMap::from([("name", first_name.clone())]),
    );
    shell.click_button_label(&endpoint_a);
    let endpoint_b = shell.catalog().format(
        "assembly-use-endpoint-b",
        &std::collections::BTreeMap::from([("name", second_name.clone())]),
    );
    shell.click_button_label(&endpoint_b);
    assert_eq!(
        shell.app().assembly_endpoint_references_ready(),
        (true, true),
        "references={}, digest={}",
        shell.app().exact_stable_reference_count(),
        shell.app().action_digest()
    );
    let reversed = shell.catalog().text("assembly-reversed");
    shell.click_role_and_label(Role::CheckBox, &reversed);
    let create_mate = shell.catalog().text("assembly-preview-create-mate");
    let before_mate = shell.app().canonical_digest();
    shell.click_button_label(&create_mate);
    assert_eq!(shell.app().canonical_digest(), before_mate);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().assembly_mate_count(), 1);

    let mate_id = shell
        .app()
        .document_snapshot()
        .assembly_mates()
        .next()
        .unwrap()
        .id();
    let edit = shell.catalog().format(
        "assembly-edit-mate",
        &std::collections::BTreeMap::from([("id", mate_id.0.to_string())]),
    );
    shell.click_button_label(&edit);
    let value_label = shell.catalog().text("assembly-distance-value");
    shell.focus_text_input(&value_label);
    shell.key(Key::A, Modifiers::CTRL);
    shell.type_text("5");
    let update = shell.catalog().text("assembly-preview-update-mate");
    shell.click_button_label(&update);
    confirm_preview(&mut shell);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .assembly_mate(mate_id)
            .unwrap()
            .kind(),
        AssemblyMateKind::CoincidentPlanar { offset_mm: 5.0, .. }
    ));

    let solve = shell.catalog().text("assembly-preview-solve");
    shell.click_button_label(&solve);
    assert!(matches!(
        shell.app().assembly_solve_status(),
        Some(AssemblySolveStatus::UnderConstrained | AssemblySolveStatus::FullyConstrained)
    ));
    if shell.app().assembly_preview_pending() {
        confirm_preview(&mut shell);
    }

    let remove = shell.catalog().format(
        "assembly-remove-mate",
        &std::collections::BTreeMap::from([("id", mate_id.0.to_string())]),
    );
    let before_cancel = shell.app().canonical_digest();
    shell.click_button_label(&remove);
    let cancel = shell.catalog().text("assembly-cancel-preview");
    shell.click_button_label(&cancel);
    assert_eq!(shell.app().canonical_digest(), before_cancel);
    assert_eq!(shell.app().assembly_mate_count(), 1);

    shell.click_button_label(&remove);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().assembly_mate_count(), 0);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().assembly_mate_count(), 1);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().assembly_mate_count(), 0);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().assembly_mate_count(), 1);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    let persisted = persistence::load_file(&saved).unwrap();
    assert_eq!(persisted.snapshot().assembly_mates().count(), 1);
    assert_eq!(persisted.snapshot().grounded_occurrences().count(), 1);
    assert_eq!(persisted.snapshot().occurrences().count(), 3);

    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().assembly_mate_count(), 1);
    assert_eq!(shell.app().grounded_occurrence_count(), 1);
    assert_eq!(shell.app().occurrence_count(), 3);
    assert!(!shell.app().can_undo());
}

#[test]
fn definition_edit_requires_reviewed_assembly_recompute_before_solve() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_stable_references(&mut shell);
    open_assembly_editor(&mut shell);

    insert_occurrence(&mut shell);
    insert_occurrence(&mut shell);
    let occurrences = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(occurrences.len(), 3);

    let ground = shell.catalog().format(
        "assembly-preview-ground",
        &std::collections::BTreeMap::from([("name", occurrences[0].1.clone())]),
    );
    shell.click_button_label(&ground);
    confirm_preview(&mut shell);
    let endpoint_a = shell.catalog().format(
        "assembly-use-endpoint-a",
        &std::collections::BTreeMap::from([("name", occurrences[0].1.clone())]),
    );
    shell.click_button_label(&endpoint_a);
    let endpoint_b = shell.catalog().format(
        "assembly-use-endpoint-b",
        &std::collections::BTreeMap::from([("name", occurrences[1].1.clone())]),
    );
    shell.click_button_label(&endpoint_b);
    let reversed = shell.catalog().text("assembly-reversed");
    shell.click_role_and_label(Role::CheckBox, &reversed);
    let create_mate = shell.catalog().text("assembly-preview-create-mate");
    shell.click_button_label(&create_mate);
    confirm_preview(&mut shell);

    let before_edit = shell
        .app()
        .document_snapshot()
        .assembly_mates()
        .next()
        .unwrap()
        .endpoint_a()
        .reference()
        .canonical_input_digest
        .clone();
    open_assembly_editor(&mut shell);
    let definition_revision = shell.app().document_revision();
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: FeatureId(2),
                value_text: "120".to_owned(),
            },)
    );
    shell.settle();
    let confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), definition_revision + 1);
    assert!(matches!(
        shell.app().document_snapshot().feature(FeatureId(2)).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 120.0
    ));
    wait_for_stable_references(&mut shell);
    open_assembly_editor(&mut shell);

    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo = shell.app().undo_step_count();
    let solve = shell.catalog().text("assembly-preview-solve");
    shell.click_button_label(&solve);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo);
    assert!(
        shell.app().assembly_preview_pending(),
        "definition recompute must produce one reviewed rebind/solve proposal: {}",
        shell.app().action_digest()
    );
    confirm_preview(&mut shell);

    let rebound = shell
        .app()
        .document_snapshot()
        .assembly_mates()
        .next()
        .unwrap()
        .endpoint_a()
        .reference()
        .canonical_input_digest
        .clone();
    assert_ne!(rebound, before_edit);
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().undo_step_count(), undo + 1);
}

#[test]
fn invalid_selection_stale_confirmation_and_conflict_are_fail_closed() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_stable_references(&mut shell);
    open_assembly_editor(&mut shell);

    insert_occurrence(&mut shell);
    insert_occurrence(&mut shell);
    let occurrences = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(occurrences.len(), 3);
    let ground = shell.catalog().format(
        "assembly-preview-ground",
        &std::collections::BTreeMap::from([("name", occurrences[0].1.clone())]),
    );
    shell.click_button_label(&ground);
    confirm_preview(&mut shell);

    let endpoint_a = shell.catalog().format(
        "assembly-use-endpoint-a",
        &std::collections::BTreeMap::from([("name", occurrences[0].1.clone())]),
    );
    let invalid_endpoint_b = shell.catalog().format(
        "assembly-use-endpoint-b",
        &std::collections::BTreeMap::from([("name", occurrences[0].1.clone())]),
    );
    shell.click_button_label(&endpoint_a);
    shell.click_button_label(&invalid_endpoint_b);
    let before_invalid = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    let create_mate = shell.catalog().text("assembly-preview-create-mate");
    shell.click_button_label(&create_mate);
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before_invalid
    );

    let endpoint_b = shell.catalog().format(
        "assembly-use-endpoint-b",
        &std::collections::BTreeMap::from([("name", occurrences[1].1.clone())]),
    );
    shell.click_button_label(&endpoint_b);
    let reversed = shell.catalog().text("assembly-reversed");
    shell.click_role_and_label(Role::CheckBox, &reversed);
    shell.click_button_label(&create_mate);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().assembly_mate_count(), 1);

    let value_label = shell.catalog().text("assembly-distance-value");
    shell.focus_text_input(&value_label);
    shell.key(Key::A, Modifiers::CTRL);
    shell.type_text("5");
    let before_conflict = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&create_mate);
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(
        shell.app().assembly_solve_status(),
        Some(AssemblySolveStatus::OverConstrained)
    );
    assert_eq!(shell.app().assembly_mate_count(), 1);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before_conflict
    );

    let preview_insert = shell.catalog().text("assembly-preview-insert");
    shell.click_button_label(&preview_insert);
    assert!(shell.app().assembly_preview_pending());
    open_assembly_editor(&mut shell);
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: DefinitionId(1),
                name: "Stale assembly definition".to_owned(),
            })
    );
    shell.settle();
    let assistant_confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&assistant_confirm);
    let after_intervening_edit = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    open_assembly_editor(&mut shell);
    let confirm = shell.catalog().text("assembly-confirm-preview");
    shell.click_button_label(&confirm);
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(shell.app().occurrence_count(), 3);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        after_intervening_edit
    );
}

#[test]
fn lost_reference_solve_after_open_is_fail_closed_without_exact_results() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("assembly-ui-lost.ketchup");
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&saved));
    author
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_stable_references(&mut author);
    open_assembly_editor(&mut author);
    insert_occurrence(&mut author);
    insert_occurrence(&mut author);
    let occurrences = author
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.name().to_owned())
        .collect::<Vec<_>>();
    let ground = author.catalog().format(
        "assembly-preview-ground",
        &std::collections::BTreeMap::from([("name", occurrences[0].clone())]),
    );
    author.click_button_label(&ground);
    confirm_preview(&mut author);
    let endpoint_a = author.catalog().format(
        "assembly-use-endpoint-a",
        &std::collections::BTreeMap::from([("name", occurrences[0].clone())]),
    );
    let endpoint_b = author.catalog().format(
        "assembly-use-endpoint-b",
        &std::collections::BTreeMap::from([("name", occurrences[1].clone())]),
    );
    author.click_button_label(&endpoint_a);
    author.click_button_label(&endpoint_b);
    let reversed = author.catalog().text("assembly-reversed");
    author.click_role_and_label(Role::CheckBox, &reversed);
    let create_mate = author.catalog().text("assembly-preview-create-mate");
    author.click_button_label(&create_mate);
    confirm_preview(&mut author);
    author.click_menu_command("menu-file", AppCommand::SaveAs);
    let saved_digest = author.app().canonical_digest();
    assert!(saved.is_file());
    drop(author);

    let mut reopened = Shell::with_dialogs(
        ScriptedFileDialogs::new()
            .queue_open(&saved)
            .always_discard(),
    );
    reopened.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(reopened.app().canonical_digest(), saved_digest);
    assert_eq!(reopened.app().occurrence_count(), 3);
    assert_eq!(reopened.app().assembly_mate_count(), 1);
    let unavailable_worker = directory.path().join("unavailable-exact-worker");
    std::fs::write(&unavailable_worker, b"not an executable").unwrap();
    reopened
        .app_mut()
        .connect_exact_worker(&unavailable_worker)
        .unwrap();
    assert!(
        reopened
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
                target: FeatureId(2),
                value_text: "120".to_owned(),
            })
    );
    reopened.settle();
    let assistant_confirm = reopened.catalog().text("assistant-confirm");
    reopened.click_row(&assistant_confirm);
    assert!(matches!(
        reopened
            .app()
            .document_snapshot()
            .feature(FeatureId(2))
            .unwrap()
            .kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 120.0
    ));
    open_assembly_editor(&mut reopened);
    let before_lost = (
        reopened.app().document_revision(),
        reopened.app().canonical_digest(),
        reopened.app().undo_step_count(),
    );
    let solve = reopened.catalog().text("assembly-preview-solve");
    reopened.click_button_label(&solve);
    assert!(!reopened.app().assembly_preview_pending());
    assert_eq!(reopened.app().assembly_mate_count(), 1);
    assert_eq!(
        (
            reopened.app().document_revision(),
            reopened.app().canonical_digest(),
            reopened.app().undo_step_count(),
        ),
        before_lost
    );
    assert_eq!(
        reopened.app().action_digest(),
        reopened.catalog().format(
            "assembly-error",
            &std::collections::BTreeMap::from([(
                "reason",
                reopened.catalog().text("assembly-error-solve-refused"),
            )]),
        )
    );
}

#[test]
fn assembly_editor_controls_are_localized_and_accessible() {
    for catalog in [LocaleCatalog::english(), LocaleCatalog::slovak()] {
        let mut shell = Shell::with_catalog(catalog);
        open_assembly_editor(&mut shell);
        for key in [
            "assembly-preview-insert",
            "assembly-mate-kind",
            "assembly-reference-a",
            "assembly-reference-b",
        ] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(
                    if key == "assembly-preview-insert" {
                        Role::Button
                    } else {
                        Role::ComboBox
                    },
                    &label,
                ),
                "missing localized AccessKit control {key}"
            );
        }
    }
}
