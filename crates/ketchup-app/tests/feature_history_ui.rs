//! Program 6 body-aware feature history replayed offscreen through AccessKit.

mod harness;

use eframe::egui::{Key, accesskit::Role};
use harness::{Shell, ctrl};
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
};
use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind, OccurrenceId, Transform,
};
use ketchup_core::drawing::{DrawingSheet, DrawingSheetId, DrawingSource};
use ketchup_core::exact_product::{ExactFaceRole, ExactFeatureChainRequest};
use ketchup_core::persistence;
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFINITION: DefinitionId = DefinitionId(1);
const BODY: BodyId = BodyId(1);
const EXTRUSION: FeatureId = FeatureId(2);
const REPLACEMENT_SOURCE: DefinitionId = DefinitionId(1);
const REPLACEMENT_TARGET: DefinitionId = DefinitionId(2);
const REPLACEMENT_SOURCE_PROFILE: FeatureId = FeatureId(10);
const REPLACEMENT_SOURCE_EXTRUSION: FeatureId = FeatureId(11);
const REPLACEMENT_TARGET_PROFILE: FeatureId = FeatureId(20);
const REPLACEMENT_TARGET_EXTRUSION: FeatureId = FeatureId(21);
const REPLACEMENT_SELECTED: OccurrenceId = OccurrenceId(100);
const REPLACEMENT_SIBLING: OccurrenceId = OccurrenceId(101);
const REPLACEMENT_TARGET_OCCURRENCE: OccurrenceId = OccurrenceId(200);
const REPLACEMENT_PLANAR_MATE: AssemblyMateId = AssemblyMateId(300);
const REPLACEMENT_AXIAL_MATE: AssemblyMateId = AssemblyMateId(301);
const REPLACEMENT_SHEET: DrawingSheetId = DrawingSheetId(400);

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

fn wait_for_exact_bodies(shell: &mut Shell, expected: usize) {
    for _ in 0..500 {
        shell.step();
        shell.settle();
        if shell.app().exact_render_body_count() == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "expected {expected} current exact bodies, found {}: {}",
        shell.app().exact_render_body_count(),
        shell.app().action_digest()
    );
}

fn write_component_replacement_fixture(path: &Path) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: REPLACEMENT_SOURCE,
                name: "Source component".to_owned(),
            },
            CanonicalCommand::CreateDefinition {
                id: REPLACEMENT_TARGET,
                name: "Compatible target".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: REPLACEMENT_SOURCE_PROFILE,
                definition_id: REPLACEMENT_SOURCE,
                name: "Source profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: REPLACEMENT_SOURCE_EXTRUSION,
                definition_id: REPLACEMENT_SOURCE,
                name: "Source extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: REPLACEMENT_SOURCE_PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: REPLACEMENT_TARGET_PROFILE,
                definition_id: REPLACEMENT_TARGET,
                name: "Target profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [14.0, 0.0], [14.0, 8.0], [0.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: REPLACEMENT_TARGET_EXTRUSION,
                definition_id: REPLACEMENT_TARGET,
                name: "Target extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: REPLACEMENT_TARGET_PROFILE,
                    height: Dimension::from_decimal("18").unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: REPLACEMENT_SELECTED,
                definition_id: REPLACEMENT_SOURCE,
                name: "Selected source".to_owned(),
                transform: Transform::from_translation(5.0, 6.0, 7.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: REPLACEMENT_SIBLING,
                definition_id: REPLACEMENT_SOURCE,
                name: "Source sibling".to_owned(),
                transform: Transform::from_translation(30.0, 6.0, 7.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: REPLACEMENT_TARGET_OCCURRENCE,
                definition_id: REPLACEMENT_TARGET,
                name: "Existing target".to_owned(),
                transform: Transform::from_translation(55.0, 6.0, 7.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let source_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&snapshot, REPLACEMENT_SOURCE, BodyId(1))
            .unwrap();
    let target_request =
        ExactFeatureChainRequest::from_snapshot_for_body(&snapshot, REPLACEMENT_TARGET, BodyId(1))
            .unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let source = worker.evaluate_rectangle(&source_request).unwrap();
    let target = worker.evaluate_rectangle(&target_request).unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                REPLACEMENT_PLANAR_MATE,
                AssemblyMateEndpoint::resolved(
                    REPLACEMENT_SELECTED,
                    source.reference(ExactFaceRole::Top).unwrap().clone(),
                ),
                AssemblyMateEndpoint::resolved(
                    REPLACEMENT_TARGET_OCCURRENCE,
                    target.reference(ExactFaceRole::Bottom).unwrap().clone(),
                ),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                REPLACEMENT_AXIAL_MATE,
                AssemblyMateEndpoint::resolved(
                    REPLACEMENT_SELECTED,
                    source.reference(ExactFaceRole::East).unwrap().clone(),
                ),
                AssemblyMateEndpoint::resolved(
                    REPLACEMENT_TARGET_OCCURRENCE,
                    target.reference(ExactFaceRole::East).unwrap().clone(),
                ),
                AssemblyMateKind::ConcentricAxial { reversed: false },
            )),
            CanonicalCommand::SetOccurrenceGrounded {
                id: REPLACEMENT_SELECTED,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: REPLACEMENT_SIBLING,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: REPLACEMENT_TARGET_OCCURRENCE,
                grounded: true,
            },
            CanonicalCommand::CreateDrawingSheet(
                DrawingSheet::new(
                    REPLACEMENT_SHEET,
                    "Replacement assembly",
                    DrawingSource::RigidAssembly {
                        occurrence_ids: vec![
                            REPLACEMENT_SELECTED,
                            REPLACEMENT_SIBLING,
                            REPLACEMENT_TARGET_OCCURRENCE,
                        ],
                    },
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
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
fn make_unique_choice_previews_selected_fork_and_commits_one_undo_step() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_body(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-edit", AppCommand::Copy);
    shell.click_menu_command("menu-edit", AppCommand::Paste);
    wait_for_exact_body(&mut shell);
    let second = shell.top_face_centre(2);
    shell.click_at(second);
    open_history(&mut shell);

    let make_unique = shell
        .catalog()
        .text("feature-history-change-scope-make-unique");
    assert!(shell.has_role_and_label(Role::RadioButton, &make_unique));
    shell.click_role_and_label(Role::RadioButton, &make_unique);
    let before = stamp(&shell);
    replace_exact_value(&mut shell, "35");
    let preview_edit = shell.catalog().text("feature-history-preview-edit");
    shell.click_button_label(&preview_edit);

    assert_eq!(shell.app().feature_history_fork_identity(), Some([2, 1, 2]));
    assert_eq!(
        shell.app().feature_history_fork_impact_counts(),
        Some([1, 1, 0, 0, 1, 2])
    );
    assert_eq!(stamp(&shell), before);
    let visible = shell
        .catalog()
        .text("feature-history-shared-impact-visible");
    let sibling = shell.catalog().format(
        "feature-history-shared-impact-occurrence",
        &BTreeMap::from([("id", "1".to_owned()), ("visibility", visible)]),
    );
    let pending = shell
        .catalog()
        .text("feature-history-shared-impact-export-pending");
    let step = shell
        .catalog()
        .text("feature-history-shared-impact-export-step");
    let stl = shell
        .catalog()
        .text("feature-history-shared-impact-export-stl");
    for label in [
        shell.catalog().text("feature-history-fork-impact-title"),
        shell.catalog().format(
            "feature-history-fork-impact-identity",
            &BTreeMap::from([
                ("occurrence", "2".to_owned()),
                ("source", "1".to_owned()),
                ("fork", "2".to_owned()),
            ]),
        ),
        shell.catalog().format(
            "feature-history-fork-impact-siblings",
            &BTreeMap::from([("items", sibling)]),
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
            &BTreeMap::from([("items", "1:4".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-exports",
            &BTreeMap::from([("items", format!("{step}: {pending}, {stl}: {pending}"))]),
        ),
        shell.catalog().format(
            "feature-history-fork-impact-diagnostics",
            &BTreeMap::from([
                ("revision", before.0.to_string()),
                ("source", "1".to_owned()),
                ("fork", "2".to_owned()),
            ]),
        ),
    ] {
        assert!(
            shell.has_visible_label(&label),
            "missing Make Unique AccessKit label: {label}"
        );
    }

    shell.press_key(Key::Escape);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before);
    assert_eq!(shell.app().definition_count(), 1);

    replace_exact_value(&mut shell, "35");
    shell.click_button_label(&preview_edit);
    confirm(&mut shell);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.2 + 1);
    assert_eq!(shell.app().definition_count(), 2);
    assert_eq!(
        shell
            .app()
            .occurrence_definition_id(ketchup_core::document::OccurrenceId(1)),
        Some(DEFINITION)
    );
    assert_eq!(
        shell
            .app()
            .occurrence_definition_id(ketchup_core::document::OccurrenceId(2)),
        Some(DefinitionId(2))
    );
    assert!(matches!(
        shell.app().document_snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 20.0
    ));
    assert!(matches!(
        shell.app().document_snapshot().feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 35.0
    ));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().definition_count(), 1);
    assert_eq!(
        shell
            .app()
            .occurrence_definition_id(ketchup_core::document::OccurrenceId(2)),
        Some(DEFINITION)
    );
}

#[test]
fn replace_component_choice_previews_complete_impact_and_commits_one_undo_step() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("component-replacement.ketchup");
    write_component_replacement_fixture(&fixture);
    let mut shell = Shell::with_dialogs(
        ScriptedFileDialogs::new()
            .queue_open(&fixture)
            .always_discard(),
    );
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 2);
    shell.click_at(shell.top_face_centre(REPLACEMENT_SELECTED.0));
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    open_history(&mut shell);

    let replace = shell
        .catalog()
        .text("feature-history-change-scope-replace-component");
    assert!(shell.has_role_and_label(Role::RadioButton, &replace));
    shell.click_role_and_label(Role::RadioButton, &replace);
    let target = shell.catalog().text("feature-history-replacement-target");
    assert!(shell.has_role_and_label(Role::ComboBox, &target));
    let preview = shell
        .catalog()
        .text("feature-history-preview-replace-component");
    assert!(shell.has_role_and_label(Role::Button, &preview));

    let before = stamp(&shell);
    let source_snapshot = shell.app().document_snapshot();
    let source_definition = source_snapshot
        .definition(REPLACEMENT_SOURCE)
        .unwrap()
        .clone();
    let target_definition = source_snapshot
        .definition(REPLACEMENT_TARGET)
        .unwrap()
        .clone();
    let source_sibling = source_snapshot
        .occurrence(REPLACEMENT_SIBLING)
        .unwrap()
        .clone();
    let target_occurrence = source_snapshot
        .occurrence(REPLACEMENT_TARGET_OCCURRENCE)
        .unwrap()
        .clone();
    shell.click_button_label(&preview);
    assert!(shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before);
    assert_eq!(
        shell.app().feature_history_replacement_identity(),
        Some([100, 1, 2])
    );
    assert_eq!(
        shell.app().feature_history_replacement_impact_counts(),
        Some([1, 2, 3, 1, 1, 2, 3, 1, 2])
    );

    let front = shell
        .catalog()
        .text("feature-history-shared-impact-view-front");
    let top = shell
        .catalog()
        .text("feature-history-shared-impact-view-top");
    let right = shell
        .catalog()
        .text("feature-history-shared-impact-view-right");
    let current = shell
        .catalog()
        .text("feature-history-shared-impact-export-current");
    let step = shell
        .catalog()
        .text("feature-history-shared-impact-export-step");
    let stl = shell
        .catalog()
        .text("feature-history-shared-impact-export-stl");
    for label in [
        shell
            .catalog()
            .text("feature-history-replacement-impact-title"),
        shell.catalog().format(
            "feature-history-replacement-impact-identity",
            &BTreeMap::from([
                ("occurrence", "100".to_owned()),
                ("source", "1".to_owned()),
                ("target", "2".to_owned()),
            ]),
        ),
        shell.catalog().format(
            "feature-history-replacement-impact-bodies",
            &BTreeMap::from([("items", "1→1".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-replacement-impact-features",
            &BTreeMap::from([("items", "10→20, 11→21".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-replacement-impact-siblings",
            &BTreeMap::from([("items", "S101, T200".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-mates",
            &BTreeMap::from([("items", "300@100, 301@100".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-views",
            &BTreeMap::from([("items", format!("400:{front}, 400:{top}, 400:{right}"))]),
        ),
        shell.catalog().format(
            "feature-history-replacement-impact-jobs",
            &BTreeMap::from([("items", "2:1:21".to_owned())]),
        ),
        shell.catalog().format(
            "feature-history-shared-impact-exports",
            &BTreeMap::from([("items", format!("{step}: {current}, {stl}: {current}"))]),
        ),
    ] {
        assert!(
            shell.has_visible_label(&label),
            "missing replacement-impact AccessKit label: {label}"
        );
    }

    shell.press_key(Key::Escape);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before);
    shell.click_button_label(&preview);
    confirm(&mut shell);
    assert_eq!(shell.app().document_revision(), before.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before.2 + 1);
    assert_eq!(
        shell.app().occurrence_definition_id(REPLACEMENT_SELECTED),
        Some(REPLACEMENT_TARGET)
    );
    let replaced = shell.app().document_snapshot();
    assert_eq!(
        replaced.definition(REPLACEMENT_SOURCE),
        Some(&source_definition)
    );
    assert_eq!(
        replaced.definition(REPLACEMENT_TARGET),
        Some(&target_definition)
    );
    assert_eq!(
        replaced.occurrence(REPLACEMENT_SIBLING),
        Some(&source_sibling)
    );
    assert_eq!(
        replaced.occurrence(REPLACEMENT_TARGET_OCCURRENCE),
        Some(&target_occurrence)
    );
    let replaced_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.1);
    assert_eq!(
        shell.app().occurrence_definition_id(REPLACEMENT_SELECTED),
        Some(REPLACEMENT_SOURCE)
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), replaced_digest);
}

#[test]
fn feature_history_controls_are_complete_localized_accesskit_nodes() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory
        .path()
        .join("localized-component-replacement.ketchup");
    write_component_replacement_fixture(&fixture);
    for catalog in [
        ketchup_interaction::LocaleCatalog::english(),
        ketchup_interaction::LocaleCatalog::slovak(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        open_history(&mut shell);
        for key in ["feature-history-definition", "feature-history-parameter"] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(Role::ComboBox, &label),
                "missing localized AccessKit ComboBox {key}: {label}"
            );
        }
        for key in [
            "feature-history-change-scope-shared",
            "feature-history-change-scope-make-unique",
        ] {
            let label = shell.catalog().text(key);
            assert!(
                shell.has_role_and_label(Role::RadioButton, &label),
                "missing localized AccessKit radio {key}: {label}"
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

    for catalog in [
        ketchup_interaction::LocaleCatalog::english(),
        ketchup_interaction::LocaleCatalog::slovak(),
    ] {
        let dialogs = ScriptedFileDialogs::new()
            .queue_open(&fixture)
            .always_discard();
        let mut shell = Shell::with_catalog_and_dialogs(catalog, dialogs);
        shell.click_menu_command("menu-file", AppCommand::Open);
        open_history(&mut shell);
        let replace = shell
            .catalog()
            .text("feature-history-change-scope-replace-component");
        assert!(shell.has_role_and_label(Role::RadioButton, &replace));
        shell.click_role_and_label(Role::RadioButton, &replace);
        let target_key = "feature-history-replacement-target";
        let target_label = shell.catalog().text(target_key);
        assert!(
            shell.has_role_and_label(Role::ComboBox, &target_label),
            "missing localized AccessKit ComboBox {target_key}: {target_label}"
        );
        let preview = shell
            .catalog()
            .text("feature-history-preview-replace-component");
        assert!(
            shell.has_role_and_label(Role::Button, &preview),
            "missing localized replacement preview button: {preview}"
        );
    }
}
