//! Program 3 rigid-assembly authoring replayed offscreen through AccessKit.

mod harness;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{Key, Modifiers, accesskit::Role};
use harness::Shell;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{AppCommand, AssistantWorkspaceMode};
use ketchup_core::assembly::{AssemblyMateKind, AssemblySolveStatus};
use ketchup_core::assembly_joint::{
    AssemblyJointId, AssemblyJointKind, AssemblyMotionDriver,
    solve_assembly_joint_kinematics_with_drivers,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureId, FeatureKind, Transform,
};
use ketchup_core::drawing::{DrawingSheetId, DrawingSource};
use ketchup_core::import::ImportFormat;
use ketchup_core::intent::WorkflowIntent;
use ketchup_core::mechanical_contract::{
    MechanicalAxisAlignment, MechanicalCondition, MechanicalConditionKind, MechanicalInterface,
    MechanicalPlanarFrame, MechanicalRole, MechanicalViolationKind, preview_mechanical_contract,
};
use ketchup_core::persistence;
use ketchup_core::reference_examples::{
    HETTICH_EXAMPLE_BOTTOM, HETTICH_EXAMPLE_DRAWER, HETTICH_EXAMPLE_DRAWER_BACK,
    HETTICH_EXAMPLE_DRAWER_JOINT, HETTICH_EXAMPLE_DRAWER_LEFT_SIDE,
    HETTICH_EXAMPLE_DRAWER_RIGHT_SIDE, HETTICH_EXAMPLE_DRAWER_SUPPORT,
    HETTICH_EXAMPLE_GUIDE_ALIGNMENT, HETTICH_EXAMPLE_LEFT_CABINET_RAIL,
    HETTICH_EXAMPLE_LEFT_MOUNTING_CONTACT, HETTICH_EXAMPLE_LEFT_SIDE, HETTICH_EXAMPLE_MOTION_STUDY,
    HETTICH_EXAMPLE_RIGHT_MOUNTING_CONTACT, HETTICH_EXAMPLE_RIGHT_RAIL_MOUNTING,
    HETTICH_EXAMPLE_RIGHT_SIDE,
};
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
fn joints_and_motion_studies_share_ui_and_assistant_atomic_preview_contract() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("assembly-kinematics-ui-ai.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    open_assembly_editor(&mut shell);
    insert_occurrence(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    assert_eq!(shell.app().selected_occurrence_count(), 2);

    let joint_preview = shell.catalog().text("assembly-preview-joint");
    let assembly_cancel = shell.catalog().text("assembly-cancel-preview");
    let baseline = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&joint_preview);
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(shell.app().assembly_joint_count(), 0);
    assert_eq!(shell.app().canonical_digest(), baseline.1);
    shell.click_button_label(&assembly_cancel);
    assert_eq!(shell.app().assembly_joint_count(), 0);
    assert_eq!(shell.app().canonical_digest(), baseline.1);

    shell.click_button_label(&joint_preview);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().assembly_joint_count(), 1);
    assert_eq!(shell.app().document_revision(), baseline.0 + 1);
    assert_eq!(shell.app().undo_step_count(), baseline.2 + 1);

    let joint_position = shell.catalog().text("assembly-joint-position");
    shell.focus_text_input(&joint_position);
    shell.key(Key::A, Modifiers::CTRL);
    shell.type_text("10");
    let assistant_joint = shell.catalog().text("assistant-preview-assembly-joint");
    shell.click_button_label(&assistant_joint);
    assert!(
        shell.app().assistant_proposal().is_some(),
        "selected={}, digest={}",
        shell.app().selected_occurrence_count(),
        shell.app().action_digest()
    );
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .assembly_joints()
            .next()
            .unwrap()
            .kind(),
        AssemblyJointKind::Prismatic {
            position_mm: 0.0,
            ..
        }
    ));
    open_assembly_editor(&mut shell);
    shell.settle();
    let assistant_confirm = shell.catalog().text("assistant-confirm");
    assert!(
        shell.has_visible_label(&assistant_confirm),
        "assistant review button missing: {}",
        shell.app().action_digest()
    );
    shell.click_button_label(&assistant_confirm);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_joints()
            .next()
            .unwrap()
            .kind()
            .position(),
        Some(10.0),
        "{}",
        shell.app().action_digest()
    );
    open_assembly_editor(&mut shell);

    let assistant_motion = shell.catalog().text("assistant-preview-motion-study");
    shell.click_button_label(&assistant_motion);
    assert!(shell.app().assistant_proposal().is_some());
    assert_eq!(shell.app().assembly_motion_study_count(), 0);
    open_assembly_editor(&mut shell);
    shell.settle();
    shell.click_button_label(&shell.catalog().text("assistant-cancel"));
    assert_eq!(shell.app().assembly_motion_study_count(), 0);

    shell.click_button_label(&assistant_motion);
    shell.settle();
    shell.click_button_label(&shell.catalog().text("assistant-confirm"));
    assert_eq!(shell.app().assembly_motion_study_count(), 1);
    open_assembly_editor(&mut shell);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_motion_studies()
            .next()
            .unwrap()
            .drivers()[0]
            .position(),
        25.0
    );

    shell.app_mut().headless_set_assembly_motion_position(35.0);
    let motion_preview = shell.catalog().text("assembly-preview-motion-study");
    shell.click_button_label(&motion_preview);
    confirm_preview(&mut shell);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_motion_studies()
            .next()
            .unwrap()
            .drivers()[0]
            .position(),
        35.0
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_motion_studies()
            .next()
            .unwrap()
            .drivers()[0]
            .position(),
        25.0
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_motion_studies()
            .next()
            .unwrap()
            .drivers()[0]
            .position(),
        35.0
    );

    shell.click_button_label(&assistant_motion);
    open_assembly_editor(&mut shell);
    let before_stale = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_menu_command("menu-edit", AppCommand::Deselect);
    shell.settle();
    shell.click_button_label(&shell.catalog().text("assistant-confirm"));
    assert!(shell.app().assistant_proposal().is_none());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before_stale
    );

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().assembly_joint_count(), 1);
    assert_eq!(shell.app().assembly_motion_study_count(), 1);
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
fn drawing_from_selection_is_previewed_cancelable_and_one_step_undoable() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_stable_references(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    open_assembly_editor(&mut shell);

    let preview = shell.catalog().text("assembly-preview-selection-drawing");
    let before = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before
    );
    let cancel = shell.catalog().text("assembly-cancel-preview");
    shell.click_button_label(&cancel);
    assert_eq!(shell.app().document_snapshot().drawing_sheets().count(), 0);
    assert_eq!(shell.app().canonical_digest(), before.1);

    shell.click_button_label(&preview);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().document_snapshot().drawing_sheets().count(), 1);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.2 + 1);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().document_snapshot().drawing_sheets().count(), 0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().document_snapshot().drawing_sheets().count(), 1);
}

#[test]
fn selection_drawing_rejects_drift_and_non_rigid_sources_then_round_trips_exactly() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("selection-drawing.ketchup");
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
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    open_assembly_editor(&mut shell);

    let preview = shell.catalog().text("assembly-preview-selection-drawing");
    let confirm = shell.catalog().text("assembly-confirm-preview");
    let cancel = shell.catalog().text("assembly-cancel-preview");
    let initial = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    shell.click_menu_command("menu-edit", AppCommand::Deselect);
    assert_eq!(shell.app().selected_occurrence_count(), 0);
    shell.click_button_label(&confirm);
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        initial,
        "selection drift must invalidate confirmation without mutation"
    );

    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    shell.click_button_label(&cancel);
    assert_eq!(shell.app().document_snapshot().drawing_sheets().count(), 0);
    assert_eq!(shell.app().canonical_digest(), initial.1);

    insert_occurrence(&mut shell);
    insert_occurrence(&mut shell);
    let occurrences = shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| (occurrence.id(), occurrence.name().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(occurrences.len(), 3);
    shell.click_menu_command("menu-edit", AppCommand::Deselect);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    assert_eq!(shell.app().selected_occurrence_count(), 3);

    let before_non_rigid = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&preview);
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before_non_rigid,
        "an unconstrained multi-selection must be refused fail-closed"
    );

    for (_, name) in &occurrences {
        let ground = shell.catalog().format(
            "assembly-preview-ground",
            &std::collections::BTreeMap::from([("name", name.clone())]),
        );
        shell.click_button_label(&ground);
        confirm_preview(&mut shell);
    }
    assert_eq!(shell.app().grounded_occurrence_count(), 3);
    shell.click_menu_command("menu-edit", AppCommand::Deselect);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);

    let before_commit = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(shell.app().canonical_digest(), before_commit.1);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().document_revision(), before_commit.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before_commit.2 + 1);

    let sheet_id = DrawingSheetId(1);
    let occurrence_ids = occurrences.iter().map(|(id, _)| *id).collect::<Vec<_>>();
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .drawing_sheet(sheet_id)
            .unwrap()
            .source(),
        &DrawingSource::RigidAssembly {
            occurrence_ids: occurrence_ids.clone(),
        }
    );
    let exact_fingerprint = shell
        .app()
        .headless_drawing_fingerprint(sheet_id)
        .expect("the selected rigid assembly must have a current exact drawing");
    assert_eq!(exact_fingerprint.1, vec!["front", "top", "right"]);
    let committed_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(
        shell
            .app()
            .document_snapshot()
            .drawing_sheet(sheet_id)
            .is_none()
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(
        shell.app().headless_drawing_fingerprint(sheet_id),
        Some(exact_fingerprint.clone())
    );

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    let persisted = persistence::load_file(&saved).unwrap();
    assert_eq!(
        persisted
            .snapshot()
            .drawing_sheet(sheet_id)
            .unwrap()
            .source(),
        &DrawingSource::RigidAssembly { occurrence_ids }
    );
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    wait_for_stable_references(&mut shell);
    assert_eq!(
        shell.app().headless_drawing_fingerprint(sheet_id),
        Some(exact_fingerprint)
    );
    assert!(!shell.app().can_undo());
}

#[test]
fn production_surface_excludes_capstone_actions_and_keeps_general_workflows() {
    for catalog in [LocaleCatalog::english(), LocaleCatalog::slovak()] {
        let mut shell = Shell::with_catalog(catalog);

        let part_authoring = shell.catalog().text("part-authoring-title");
        assert!(!shell.has_role_and_label(Role::Button, &part_authoring));

        for key in ["feature-history-title", "body-title", "assembly-title"] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(Role::Button, &label),
                "missing general production workflow {key}: {label}"
            );
        }

        open_assembly_editor(&mut shell);
        for key in [
            "assembly-preview-insert",
            "assembly-preview-selection-drawing",
        ] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(Role::Button, &label),
                "missing general Assembly/Drawing action {key}: {label}"
            );
        }
        for key in [
            "assembly-preview-capstone",
            "assembly-preview-capstone-drawing",
        ] {
            let label = shell.catalog().text(key);
            assert!(
                !shell.has_role_and_label(Role::Button, &label),
                "product-specific action remains on the production surface: {key}: {label}"
            );
        }

        shell.click_command(AppCommand::Rectangle);
        let face_workflow = shell.catalog().text("face-workflow-title");
        assert!(
            shell.has_role_and_label(Role::Button, &face_workflow),
            "missing general selection-driven Part workflow: {face_workflow}"
        );
        assert!(shell.offers(AppCommand::PushPull));
    }
}

#[test]
fn assembly_editor_controls_are_localized_and_accessible() {
    for catalog in [LocaleCatalog::english(), LocaleCatalog::slovak()] {
        let mut shell = Shell::with_catalog(catalog);
        open_assembly_editor(&mut shell);
        for key in [
            "assembly-preview-insert",
            "assembly-preview-selection-drawing",
            "assembly-mate-kind",
            "assembly-reference-a",
            "assembly-reference-b",
        ] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(
                    if matches!(
                        key,
                        "assembly-preview-insert" | "assembly-preview-selection-drawing"
                    ) {
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

fn hettich_document() -> DocumentStore {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/hettich-quadro-v6-drawer.ketchup");
    match persistence::load(&std::fs::read(&source).unwrap()).unwrap() {
        persistence::LoadOutcome::Editable { document, .. } => document,
        persistence::LoadOutcome::ReviewOnly(_) => {
            panic!("the shipped example must load as an editable document")
        }
    }
}

/// The shipped Hettich assembly is proven by the persisted mechanical contract that
/// travels inside the document — not by anything this test knows about part names.
/// Each mutation below is a real design error an author could make, and each one
/// must be caught by the same production validator that accepts the correct file.
#[test]
fn the_hettich_assembly_is_proven_by_its_persisted_contract_and_rejects_real_mutations() {
    let document = hettich_document();
    let snapshot = document.current();

    // 1. Roles and mounting faces come from the document, not from names.
    let roles = snapshot
        .mechanical_interfaces()
        .map(|interface| (interface.id(), interface.occurrence_id(), interface.role()))
        .collect::<Vec<_>>();
    assert_eq!(roles.len(), 7);
    assert_eq!(
        roles
            .iter()
            .filter(|(_, _, role)| *role == MechanicalRole::Mounting)
            .count(),
        4
    );
    assert_eq!(
        roles
            .iter()
            .filter(|(_, _, role)| *role == MechanicalRole::Support)
            .count(),
        2
    );
    assert_eq!(
        roles
            .iter()
            .filter(|(_, _, role)| *role == MechanicalRole::Guide)
            .count(),
        1
    );
    assert_eq!(snapshot.mechanical_conditions().count(), 5);

    let report = preview_mechanical_contract(&snapshot, HETTICH_EXAMPLE_MOTION_STUDY, 16).unwrap();
    assert!(
        report.is_satisfied(),
        "the shipped assembly must satisfy its own contract: {:?}",
        report.violations()
    );
    assert_eq!(report.evaluated_samples(), 17);
    assert_eq!(report.evaluated_conditions(), 5);

    // 2. A mounting face pushed 1 mm off the cabinet wall.
    let mut mutated = hettich_document();
    let rail = mutated
        .current()
        .occurrence(HETTICH_EXAMPLE_LEFT_CABINET_RAIL)
        .unwrap()
        .transform();
    let mut matrix = *rail.matrix();
    matrix[7] += 1.0;
    mutated
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: HETTICH_EXAMPLE_LEFT_CABINET_RAIL,
                transform: Transform::from_matrix(matrix).unwrap(),
            },
        ]))
        .unwrap();
    let gap = preview_mechanical_contract(&mutated.current(), HETTICH_EXAMPLE_MOTION_STUDY, 4)
        .unwrap()
        .violations()
        .iter()
        .find(|violation| violation.condition_id() == Some(HETTICH_EXAMPLE_LEFT_MOUNTING_CONTACT))
        .copied()
        .expect("a rail floating 1 mm off its wall must be rejected");
    let MechanicalViolationKind::ContactGap { measured_mm, .. } = gap.kind() else {
        panic!("expected a mounting gap, got {:?}", gap.kind());
    };
    assert!((measured_mm.abs() - 1.0).abs() <= 1.0e-9);

    // 3. A mounting interface whose declared normal points away from the wall.
    let mut mutated = hettich_document();
    let original = mutated
        .current()
        .mechanical_interface(HETTICH_EXAMPLE_RIGHT_RAIL_MOUNTING)
        .unwrap()
        .clone();
    let frame = original.frame();
    let reversed = MechanicalPlanarFrame::new(
        frame.origin_mm(),
        frame.normal().map(|value| -value),
        frame.area_mm2(),
        frame.bounds_mm(),
    );
    mutated
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateMechanicalInterface(MechanicalInterface::new(
                original.id(),
                original.occurrence_id(),
                original.role(),
                original.face_ordinal(),
                original.geometry_fingerprint(),
                reversed,
            )),
        ]))
        .unwrap();
    let flipped = preview_mechanical_contract(&mutated.current(), HETTICH_EXAMPLE_MOTION_STUDY, 4)
        .unwrap()
        .violations()
        .iter()
        .find(|violation| violation.condition_id() == Some(HETTICH_EXAMPLE_RIGHT_MOUNTING_CONTACT))
        .copied()
        .expect("a mounting face turned away from the wall must be rejected");
    assert!(matches!(
        flipped.kind(),
        MechanicalViolationKind::ContactOrientation { .. }
    ));

    // 4. A supported part slid off the panel that carries it.
    let mut mutated = hettich_document();
    let back = mutated
        .current()
        .occurrence(HETTICH_EXAMPLE_DRAWER_BACK)
        .unwrap()
        .transform();
    let mut matrix = *back.matrix();
    matrix[3] += 40.0;
    mutated
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: HETTICH_EXAMPLE_DRAWER_BACK,
                transform: Transform::from_matrix(matrix).unwrap(),
            },
        ]))
        .unwrap();
    let unsupported =
        preview_mechanical_contract(&mutated.current(), HETTICH_EXAMPLE_MOTION_STUDY, 4)
            .unwrap()
            .violations()
            .iter()
            .find(|violation| violation.condition_id() == Some(HETTICH_EXAMPLE_DRAWER_SUPPORT))
            .copied()
            .expect("a part that no longer overlaps its support must be rejected");
    assert!(matches!(
        unsupported.kind(),
        MechanicalViolationKind::SupportLost { .. }
    ));

    // 5. A guide plane declared to run along the travel axis instead of across it.
    let mut mutated = hettich_document();
    let guide = mutated
        .current()
        .mechanical_condition(HETTICH_EXAMPLE_GUIDE_ALIGNMENT)
        .unwrap()
        .clone();
    let MechanicalConditionKind::JointAxisAlignment {
        joint_id,
        interface,
        tolerance_degrees,
        ..
    } = guide.kind()
    else {
        panic!("the guide condition must constrain a joint axis");
    };
    mutated
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateMechanicalCondition(MechanicalCondition::new(
                guide.id(),
                MechanicalConditionKind::JointAxisAlignment {
                    joint_id,
                    interface,
                    alignment: MechanicalAxisAlignment::Parallel,
                    tolerance_degrees,
                },
            )),
        ]))
        .unwrap();
    let misaligned =
        preview_mechanical_contract(&mutated.current(), HETTICH_EXAMPLE_MOTION_STUDY, 4)
            .unwrap()
            .violations()
            .iter()
            .find(|violation| violation.condition_id() == Some(HETTICH_EXAMPLE_GUIDE_ALIGNMENT))
            .copied()
            .expect("a guide plane that does not face the declared axis must be rejected");
    let MechanicalViolationKind::AxisMisaligned {
        measured_degrees, ..
    } = misaligned.kind()
    else {
        panic!("expected an axis failure, got {:?}", misaligned.kind());
    };
    assert!((measured_degrees - 90.0).abs() <= 1.0e-9);
}

#[test]
fn hettich_drawer_example_opens_and_edits_through_the_general_headless_assembly_ui() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/hettich-quadro-v6-drawer.ketchup");
    assert!(source.is_file());
    let loaded = persistence::load(&std::fs::read(&source).unwrap()).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let imported = loaded_snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some((feature.id(), spec)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(imported.len(), 6);
    assert!(imported.iter().all(|(_, spec)| spec.solid_count == 1));
    let topology_totals = imported.iter().fold([0_u32; 5], |mut totals, (_, spec)| {
        for (total, count) in totals.iter_mut().zip(spec.topology_counts.unwrap()) {
            *total += count;
        }
        totals
    });
    assert_eq!(topology_totals, [624, 928, 318, 6, 6]);
    let imported_feature_ids = imported
        .iter()
        .map(|(feature_id, _)| *feature_id)
        .collect::<Vec<_>>();
    assert_eq!(loaded.container_data().blobs().len(), 6);
    assert_eq!(
        loaded_snapshot
            .import_receipts()
            .filter(|receipt| {
                receipt.format() == ImportFormat::Step
                    && receipt.source_name().starts_with("Hettich ")
            })
            .count(),
        6
    );
    // The runner members are selected by what they are — bodies imported from the
    // STEP file — never by how they happen to be named.
    let imported_definitions = imported
        .iter()
        .map(|(feature_id, _)| {
            loaded_snapshot
                .feature(*feature_id)
                .unwrap()
                .definition_id()
        })
        .collect::<BTreeSet<_>>();
    let mounted_parts = loaded_snapshot
        .occurrences()
        .filter(|occurrence| imported_definitions.contains(&occurrence.definition_id()))
        .collect::<Vec<_>>();
    assert_eq!(mounted_parts.len(), 6);
    let left_wall_inner_y = loaded_snapshot
        .occurrence(HETTICH_EXAMPLE_LEFT_SIDE)
        .unwrap()
        .transform()
        .matrix()[7]
        + 18.0;
    let right_wall_inner_y = loaded_snapshot
        .occurrence(HETTICH_EXAMPLE_RIGHT_SIDE)
        .unwrap()
        .transform()
        .matrix()[7];
    let drawer_left_outer_y = loaded_snapshot
        .occurrence(HETTICH_EXAMPLE_DRAWER_LEFT_SIDE)
        .unwrap()
        .transform()
        .matrix()[7];
    let drawer_right_outer_y = loaded_snapshot
        .occurrence(HETTICH_EXAMPLE_DRAWER_RIGHT_SIDE)
        .unwrap()
        .transform()
        .matrix()[7]
        + 16.0;
    assert_eq!(right_wall_inner_y - left_wall_inner_y, 562.0);
    assert_eq!(drawer_right_outer_y - drawer_left_outer_y, 522.0);
    assert_eq!(drawer_left_outer_y - left_wall_inner_y, 20.0);
    assert_eq!(right_wall_inner_y - drawer_right_outer_y, 20.0);
    // Each member is measured, then the whole measured set is compared as a set.
    // No expectation is keyed to a part name.
    let mut measured = Vec::new();
    let drawer_side_bottom_z = loaded_snapshot
        .occurrence(HETTICH_EXAMPLE_DRAWER_LEFT_SIDE)
        .unwrap()
        .transform()
        .matrix()[11];
    let drawer_bottom_underside_z = loaded_snapshot
        .occurrence(HETTICH_EXAMPLE_DRAWER)
        .unwrap()
        .transform()
        .matrix()[11];
    assert!((drawer_bottom_underside_z - drawer_side_bottom_z - 12.0).abs() <= 1.0e-6);
    for occurrence in &mounted_parts {
        let transform = occurrence.transform();
        let matrix = transform.matrix();
        assert_eq!(&matrix[0..3], &[0.0, 1.0, 0.0]);
        assert_eq!(&matrix[4..7], &[-1.0, 0.0, 0.0]);
        assert_eq!(&matrix[8..11], &[0.0, 0.0, 1.0]);
        assert_eq!(matrix[7], 600.0);
        assert_eq!(matrix[11], 140.75);
        let spec = imported
            .iter()
            .find(|(feature_id, _)| {
                loaded_snapshot
                    .feature(*feature_id)
                    .unwrap()
                    .definition_id()
                    == occurrence.definition_id()
            })
            .unwrap()
            .1;
        let mounted_depth = [
            matrix[1] * spec.bounds_mm[0][1] + matrix[3],
            matrix[1] * spec.bounds_mm[1][1] + matrix[3],
        ];
        let mounted_height = [
            spec.bounds_mm[0][2] + matrix[11],
            spec.bounds_mm[1][2] + matrix[11],
        ];
        assert!(mounted_height[0] >= 0.0 && mounted_height[1] <= 350.0);
        assert!(mounted_depth[1] <= 600.0);
        measured.push((
            spec.topology_counts.unwrap()[2],
            format!("{:.4}", spec.bounds_mm[1][1] - spec.bounds_mm[0][1]),
            format!("{:.4}", matrix[3]),
            (mounted_height[1] - drawer_side_bottom_z).abs() <= 1.0e-6,
        ));
    }
    measured.sort();
    assert_eq!(
        measured,
        vec![
            (35, "482.5000".to_owned(), "-419.0000".to_owned(), true),
            (35, "482.5000".to_owned(), "-419.0000".to_owned(), true),
            (50, "387.5000".to_owned(), "-194.0000".to_owned(), false),
            (50, "387.5000".to_owned(), "-194.0000".to_owned(), false),
            (74, "418.6000".to_owned(), "31.0000".to_owned(), false),
            (74, "418.6000".to_owned(), "31.0000".to_owned(), false),
        ]
    );
    let closed = solve_assembly_joint_kinematics_with_drivers(
        &loaded_snapshot,
        &[
            AssemblyMotionDriver::new(HETTICH_EXAMPLE_DRAWER_JOINT, 0.0),
            AssemblyMotionDriver::new(AssemblyJointId(113), 0.0),
            AssemblyMotionDriver::new(AssemblyJointId(115), 0.0),
        ],
    )
    .unwrap();
    for occurrence in &mounted_parts {
        let closed_transform = closed.pose(occurrence.id()).unwrap().world_transform();
        let closed_matrix = closed_transform.matrix();
        assert_eq!(
            closed_matrix[3],
            31.0,
            "{} must return to the unmodified closed STEP assembly",
            occurrence.name()
        );
    }
    let moved = solve_assembly_joint_kinematics_with_drivers(
        &loaded_snapshot,
        &[
            AssemblyMotionDriver::new(HETTICH_EXAMPLE_DRAWER_JOINT, 300.0),
            AssemblyMotionDriver::new(AssemblyJointId(113), 150.0),
            AssemblyMotionDriver::new(AssemblyJointId(115), 150.0),
        ],
    )
    .unwrap();
    let drawer_travel_mm = moved
        .pose(HETTICH_EXAMPLE_DRAWER)
        .unwrap()
        .world_transform()
        .matrix()[3]
        - loaded_snapshot
            .occurrence(HETTICH_EXAMPLE_DRAWER)
            .unwrap()
            .transform()
            .matrix()[3];
    assert_eq!(drawer_travel_mm, 150.0);
    let mut articulation = Vec::new();
    for occurrence in &mounted_parts {
        let moved_transform = moved
            .pose(occurrence.id())
            .expect("every exact runner part must have a solved kinematic pose")
            .world_transform();
        let moved_matrix = moved_transform.matrix();
        assert_eq!(moved_matrix[7], 600.0);
        assert_eq!(moved_matrix[11], 140.75);
        articulation.push((
            format!(
                "{:.4}",
                moved_matrix[3] - occurrence.transform().matrix()[3]
            ),
            format!("{:.4}", moved_matrix[3]),
        ));
    }
    articulation.sort();
    assert_eq!(
        articulation,
        vec![
            ("0.0000".to_owned(), "31.0000".to_owned()),
            ("0.0000".to_owned(), "31.0000".to_owned()),
            (format!("{drawer_travel_mm:.4}"), "-269.0000".to_owned()),
            (format!("{drawer_travel_mm:.4}"), "-269.0000".to_owned()),
            (
                format!("{:.4}", drawer_travel_mm / 2.0),
                "-119.0000".to_owned()
            ),
            (
                format!("{:.4}", drawer_travel_mm / 2.0),
                "-119.0000".to_owned()
            ),
        ]
    );
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("hettich-drawer-edited.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().occurrence_count(), 17);
    assert_eq!(shell.app().assembly_joint_count(), 16);
    assert_eq!(shell.app().assembly_motion_study_count(), 1);
    assert!(!shell.app().can_undo());
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    for _ in 0..1_500 {
        shell.settle();
        if imported_feature_ids.iter().all(|feature_id| {
            shell
                .app()
                .exact_current_producer_ids()
                .contains(feature_id)
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        imported_feature_ids.iter().all(|feature_id| {
            shell
                .app()
                .exact_current_producer_ids()
                .contains(feature_id)
        }),
        "all six official STEP solids must reach the exact viewport products: expected={imported_feature_ids:?}, actual={:?}",
        shell.app().exact_current_producer_ids()
    );
    assert!(
        shell.app().exact_render_triangle_count() >= 11_516,
        "the official STEP did not contribute its 11,516 display triangles"
    );
    shell.app_mut().enable_headless_instanced_scene();
    shell.settle();
    assert!(
        shell.app().instanced_scene_triangle_count() >= 11_516,
        "the official STEP never reached the painted scene"
    );
    let joint = shell
        .app()
        .document_snapshot()
        .assembly_joint(HETTICH_EXAMPLE_DRAWER_JOINT)
        .unwrap()
        .clone();
    assert!(matches!(
        joint.kind(),
        AssemblyJointKind::Prismatic {
            limits: Some(limits),
            position_mm: 450.0,
            ..
        } if limits.min() == 0.0 && limits.max() == 450.0
    ));
    for joint_id in [AssemblyJointId(113), AssemblyJointId(115)] {
        assert!(matches!(
            shell
                .app()
                .document_snapshot()
                .assembly_joint(joint_id)
                .unwrap()
                .kind(),
            AssemblyJointKind::Prismatic {
                axis,
                limits: Some(limits),
                position_mm: 225.0,
            } if axis.direction_in_parent() == [0.0, -1.0, 0.0]
                && limits.min() == 0.0
                && limits.max() == 225.0
        ));
    }

    shell
        .app_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
    shell.settle();
    let bottom_row = shell.catalog().format(
        "outliner-object",
        &std::collections::BTreeMap::from([
            ("name", "Cabinet bottom".to_owned()),
            ("dimensions", "600 × 562 × 18".to_owned()),
            ("visibility", "◉".to_owned()),
        ]),
    );
    let drawer_row = shell.catalog().format(
        "outliner-object",
        &std::collections::BTreeMap::from([
            ("name", "Drawer bottom 490 mm".to_owned()),
            ("dimensions", "450 × 490 × 13".to_owned()),
            ("visibility", "◉".to_owned()),
        ]),
    );
    shell.click_row(&bottom_row);
    shell.click_row_with(&drawer_row, Modifiers::SHIFT);
    assert_eq!(shell.app().selected_occurrence_count(), 2);
    assert!(shell.app().occurrence_is_selected(HETTICH_EXAMPLE_BOTTOM));
    assert!(shell.app().occurrence_is_selected(HETTICH_EXAMPLE_DRAWER));

    open_assembly_editor(&mut shell);
    shell.app_mut().headless_set_assembly_motion_position(300.0);
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let preview = shell.catalog().text("assembly-preview-motion-study");
    shell.click_button_label(&preview);
    assert!(
        shell.app().assembly_preview_pending(),
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    confirm_preview(&mut shell);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_motion_study(HETTICH_EXAMPLE_MOTION_STUDY)
            .unwrap()
            .drivers()
            .iter()
            .map(|driver| driver.position())
            .collect::<Vec<_>>(),
        vec![300.0, 150.0, 150.0]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_motion_study(HETTICH_EXAMPLE_MOTION_STUDY)
            .unwrap()
            .drivers()
            .iter()
            .map(|driver| driver.position())
            .collect::<Vec<_>>(),
        vec![0.0, 0.0, 0.0]
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let edited_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), edited_digest);
    assert_eq!(shell.app().occurrence_count(), 17);
    assert_eq!(shell.app().assembly_joint_count(), 16);
    assert_eq!(shell.app().assembly_motion_study_count(), 1);
    assert!(!shell.app().can_undo());
}
