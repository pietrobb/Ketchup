//! Independent Program 7 shared-change UI verification through AccessKit.

mod harness;

use eframe::egui::{Key, accesskit::Role};
use harness::{Shell, ctrl};
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
};
use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CommandBatch, DefinitionId, Dimension,
    DocumentStore, FeatureId, FeatureKind, OccurrenceId, Transform,
};
use ketchup_core::drawing::{DrawingSheet, DrawingSheetId, DrawingSource};
use ketchup_core::exact_product::{ExactFaceRole, ExactFeatureChainRequest};
use ketchup_core::intent::WorkflowIntent;
use ketchup_core::persistence;
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFINITION: DefinitionId = DefinitionId(1);
const BODY: BodyId = BodyId(1);
const PROFILE: FeatureId = FeatureId(1);
const EXTRUSION: FeatureId = FeatureId(2);
const CUT_PROFILE: FeatureId = FeatureId(3);
const POCKET: FeatureId = FeatureId(4);
const TOOL_BODY: BodyId = BodyId(2);
const TOOL_PROFILE: FeatureId = FeatureId(20);
const TOOL_EXTRUSION: FeatureId = FeatureId(21);
const UNION: FeatureId = FeatureId(30);
const FIRST: OccurrenceId = OccurrenceId(1);
const SECOND: OccurrenceId = OccurrenceId(2);
const PLANAR_MATE: AssemblyMateId = AssemblyMateId(40);
const AXIAL_MATE: AssemblyMateId = AssemblyMateId(41);
const SHEET: DrawingSheetId = DrawingSheetId(50);

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

fn click_preview(shell: &mut Shell, key: &str) {
    let label = shell.catalog().text(key);
    shell.click_button_label(&label);
}

fn confirm(shell: &mut Shell) {
    assert!(shell.app().feature_history_preview_pending());
    let label = shell.catalog().text("feature-history-confirm");
    shell.click_button_label(&label);
    assert!(!shell.app().feature_history_preview_pending());
}

fn body_label(shell: &Shell, body_id: BodyId, status_key: &str) -> String {
    let snapshot = shell.app().document_snapshot();
    let body = snapshot
        .definition(DEFINITION)
        .unwrap()
        .body(body_id)
        .unwrap();
    shell.catalog().format(
        "feature-history-select-body",
        &BTreeMap::from([
            ("name", body.name().to_owned()),
            ("id", body_id.0.to_string()),
            ("status", shell.catalog().text(status_key)),
        ]),
    )
}

fn feature_label(shell: &Shell, feature_id: FeatureId) -> String {
    let snapshot = shell.app().document_snapshot();
    let feature = snapshot.feature(feature_id).unwrap();
    shell.catalog().format(
        "feature-history-select-feature",
        &BTreeMap::from([
            ("name", feature.name().to_owned()),
            ("id", feature_id.0.to_string()),
            (
                "status",
                shell.catalog().text("feature-history-state-active"),
            ),
        ]),
    )
}

fn profile(size: f64) -> FeatureKind {
    FeatureKind::Profile {
        points_mm: vec![[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]],
    }
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

fn write_shared_dependency_fixture(path: &Path) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Shared verifier part".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Shared profile".to_owned(),
                kind: profile(20.0),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Shared extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Cut profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[2.0, 2.0], [8.0, 2.0], [8.0, 8.0], [2.0, 8.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Pocket".to_owned(),
                kind: FeatureKind::Pocket {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                    depth: Dimension::from_decimal("5").unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "First reuse".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Second reuse".to_owned(),
                transform: Transform::from_translation(30.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: SECOND,
                grounded: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let request =
        ExactFeatureChainRequest::from_snapshot_for_body(&snapshot, DEFINITION, BODY).unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker.evaluate_rectangle(&request).unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    let bottom = package.reference(ExactFaceRole::Bottom).unwrap().clone();
    let east = package.reference(ExactFaceRole::East).unwrap().clone();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                PLANAR_MATE,
                AssemblyMateEndpoint::resolved(FIRST, top),
                AssemblyMateEndpoint::resolved(SECOND, bottom),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: false,
                },
            )),
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AXIAL_MATE,
                AssemblyMateEndpoint::resolved(FIRST, east.clone()),
                AssemblyMateEndpoint::resolved(SECOND, east),
                AssemblyMateKind::ConcentricAxial { reversed: false },
            )),
            CanonicalCommand::CreateDrawingSheet(
                DrawingSheet::new(
                    SHEET,
                    "Shared assembly",
                    DrawingSource::RigidAssembly {
                        occurrence_ids: vec![FIRST, SECOND],
                    },
                )
                .unwrap(),
            ),
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn mate_fingerprints(shell: &Shell) -> Vec<String> {
    shell
        .app()
        .document_snapshot()
        .assembly_mates()
        .flat_map(|mate| [mate.endpoint_a(), mate.endpoint_b()])
        .map(|endpoint| endpoint.reference().result_fingerprint.clone())
        .collect()
}

fn write_multibody_fixture(path: &Path, cross_body_union: bool) {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Verifier part".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: PROFILE,
            definition_id: DEFINITION,
            name: "Base profile".to_owned(),
            kind: profile(20.0),
        },
        CanonicalCommand::CreateFeature {
            id: EXTRUSION,
            definition_id: DEFINITION,
            name: "Base extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: PROFILE,
                height: Dimension::from_decimal("8").unwrap(),
            },
        },
        CanonicalCommand::CreateBody {
            definition_id: DEFINITION,
            id: TOOL_BODY,
            name: "Tool body".to_owned(),
            visible: cross_body_union,
        },
        CanonicalCommand::SetActiveBody {
            definition_id: DEFINITION,
            id: TOOL_BODY,
        },
        CanonicalCommand::CreateFeature {
            id: TOOL_PROFILE,
            definition_id: DEFINITION,
            name: "Tool profile".to_owned(),
            kind: profile(4.0),
        },
        CanonicalCommand::CreateFeature {
            id: TOOL_EXTRUSION,
            definition_id: DEFINITION,
            name: "Tool extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: TOOL_PROFILE,
                height: Dimension::from_decimal("3").unwrap(),
            },
        },
        CanonicalCommand::SetActiveBody {
            definition_id: DEFINITION,
            id: BODY,
        },
    ];
    if cross_body_union {
        commands.push(CanonicalCommand::CreateFeature {
            id: UNION,
            definition_id: DEFINITION,
            name: "Union".to_owned(),
            kind: FeatureKind::Boolean {
                operation: BooleanOperation::Union,
                target: EXTRUSION,
                tool: TOOL_EXTRUSION,
            },
        });
    }
    commands.push(CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(1),
        definition_id: DEFINITION,
        name: "Verifier occurrence".to_owned(),
        transform: Transform::identity(),
        parent: None,
        tag: None,
        visible: true,
    });
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

#[test]
fn shared_change_serial_accesskit_replay_rebinds_dependencies_exports_and_persists() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("shared-dependency-source.ketchup");
    let saved = directory.path().join("shared-dependency-saved.ketchup");
    let stl = directory.path().join("shared-dependency.stl");
    let step = directory.path().join("shared-dependency.step");
    write_shared_dependency_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .queue_export(&stl)
        .queue_export(&step)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_confirm_high_risk_as(450)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_body(&mut shell);

    let second = shell.top_face_centre(SECOND.0);
    shell.click_at(second);
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    open_history(&mut shell);
    let body = body_label(&shell, BODY, "feature-history-body-active");
    shell.click_role_and_label(Role::Button, &body);
    let profile = feature_label(&shell, PROFILE);
    shell.click_role_and_label(Role::Button, &profile);
    assert_eq!(
        shell.app().feature_history_selected_feature_id(),
        Some(PROFILE)
    );
    let extrusion = feature_label(&shell, EXTRUSION);
    shell.click_role_and_label(Role::Button, &extrusion);
    assert_eq!(
        shell.app().feature_history_selected_feature_id(),
        Some(EXTRUSION)
    );

    let source = stamp(&shell);
    let source_transforms = [FIRST, SECOND].map(|id| {
        shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()
    });
    let source_fingerprints = mate_fingerprints(&shell);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );

    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    assert_eq!(
        shell.app().feature_history_shared_impact_counts(),
        Some([2, 1, 4, 3, 1, 2])
    );
    assert_eq!(stamp(&shell), source);
    let cancel = shell.catalog().text("feature-history-cancel");
    shell.click_button_label(&cancel);
    assert_eq!(stamp(&shell), source);
    assert_eq!(mate_fingerprints(&shell), source_fingerprints);

    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    shell.press_key(Key::Escape);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), source);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );

    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    confirm(&mut shell);
    assert_eq!(shell.app().document_revision(), source.0 + 1);
    assert_eq!(shell.app().undo_step_count(), source.2 + 1);
    assert_eq!(shell.app().exact_render_bounds()[0][1][2], 35.0);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );
    let edited_fingerprints = mate_fingerprints(&shell);
    assert_ne!(edited_fingerprints, source_fingerprints);
    assert_eq!(
        [FIRST, SECOND].map(|id| shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()),
        source_transforms
    );
    let edited_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    wait_for_exact_body(&mut shell);
    assert_eq!(shell.app().canonical_digest(), source.1);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    wait_for_exact_body(&mut shell);
    assert_eq!(shell.app().canonical_digest(), edited_digest);
    assert_eq!(mate_fingerprints(&shell), edited_fingerprints);

    let cut_profile = feature_label(&shell, CUT_PROFILE);
    shell.click_role_and_label(Role::Button, &cut_profile);
    click_preview(&mut shell, "feature-history-preview-suppress");
    assert_eq!(
        shell.app().feature_history_shared_impact_counts(),
        Some([2, 1, 4, 3, 1, 2])
    );
    let before_suppress = stamp(&shell);
    confirm(&mut shell);
    assert_eq!(
        shell.app().undo_step_count(),
        before_suppress.2 + 1,
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, BODY),
        Some(&BTreeSet::from([CUT_PROFILE, POCKET]))
    );
    assert_eq!(shell.app().exact_current_producer_ids(), vec![EXTRUSION]);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );
    let suppressed_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    wait_for_exact_body(&mut shell);
    assert_eq!(shell.app().canonical_digest(), edited_digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    wait_for_exact_body(&mut shell);
    assert_eq!(shell.app().canonical_digest(), suppressed_digest);

    click_preview(&mut shell, "feature-history-preview-resume");
    assert_eq!(
        shell.app().feature_history_shared_impact_counts(),
        Some([2, 1, 4, 3, 1, 2])
    );
    let before_resume = stamp(&shell);
    confirm(&mut shell);
    assert_eq!(shell.app().undo_step_count(), before_resume.2 + 1);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, BODY),
        None
    );
    assert_eq!(shell.app().exact_current_producer_ids(), vec![POCKET]);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );

    let before_exports = stamp(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(stamp(&shell), before_exports);
    assert!(stl.is_file(), "{}", shell.app().action_digest());
    assert!(step.is_file(), "{}", shell.app().action_digest());

    let persisted_digest = shell.app().canonical_digest();
    let persisted_revision = shell.app().document_revision();
    let persisted_fingerprints = mate_fingerprints(&shell);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_body(&mut shell);
    open_history(&mut shell);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().document_revision(), persisted_revision);
    assert!(!shell.app().can_undo());
    assert_eq!(mate_fingerprints(&shell), persisted_fingerprints);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );
}

#[test]
fn shared_change_worker_failure_preserves_history_dependencies_and_last_valid_exports() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("shared-worker-failure.ketchup");
    let stl = directory.path().join("last-valid.stl");
    let worker = directory.path().join(if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    });
    let parked_worker = worker.with_extension("parked");
    write_shared_dependency_fixture(&fixture);
    std::fs::copy(exact_worker_path(), &worker).unwrap();
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .queue_export(&stl)
        .always_confirm_high_risk_as(451)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell.app_mut().connect_exact_worker(&worker).unwrap();
    wait_for_exact_body(&mut shell);
    open_history(&mut shell);
    let extrusion = feature_label(&shell, EXTRUSION);
    shell.click_role_and_label(Role::Button, &extrusion);

    let before = stamp(&shell);
    let bounds_before = shell.app().exact_render_bounds();
    let mates_before = mate_fingerprints(&shell);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );
    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    assert_eq!(
        shell.app().feature_history_shared_impact_counts(),
        Some([2, 1, 4, 3, 1, 2])
    );
    std::fs::rename(&worker, &parked_worker).unwrap();
    confirm(&mut shell);
    assert_eq!(stamp(&shell), before);
    assert_eq!(shell.app().exact_render_bounds(), bounds_before);
    assert_eq!(mate_fingerprints(&shell), mates_before);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );

    std::fs::rename(&parked_worker, &worker).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert!(stl.is_file(), "{}", shell.app().action_digest());
    assert_eq!(stamp(&shell), before);
}

#[test]
fn complete_serial_accesskit_history_replay_is_atomic_stale_safe_and_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("feature-history-ui.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    open_history(&mut shell);

    let initial = stamp(&shell);
    let body = body_label(&shell, BODY, "feature-history-body-active");
    shell.click_role_and_label(Role::Button, &body);
    assert_eq!(shell.app().feature_history_selected_body_id(), Some(BODY));
    assert_eq!(stamp(&shell), initial);

    let profile = feature_label(&shell, PROFILE);
    shell.click_role_and_label(Role::Button, &profile);
    assert_eq!(
        shell.app().feature_history_selected_feature_id(),
        Some(PROFILE)
    );
    assert_eq!(stamp(&shell), initial);
    let extrusion = feature_label(&shell, EXTRUSION);
    shell.click_role_and_label(Role::Button, &extrusion);
    assert_eq!(
        shell.app().feature_history_selected_feature_id(),
        Some(EXTRUSION)
    );
    assert_eq!(stamp(&shell), initial);

    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    assert!(shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), initial);
    let cancel = shell.catalog().text("feature-history-cancel");
    shell.click_button_label(&cancel);
    assert_eq!(stamp(&shell), initial);

    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    confirm(&mut shell);
    assert_eq!(shell.app().document_revision(), initial.0 + 1);
    assert_eq!(shell.app().undo_step_count(), initial.2 + 1);
    assert!(matches!(
        shell.app().document_snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 35.0
    ));
    let edited_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(matches!(
        shell.app().document_snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 20.0
    ));
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), edited_digest);

    replace_exact_value(&mut shell, "40");
    click_preview(&mut shell, "feature-history-preview-edit");
    assert!(shell.app().feature_history_preview_pending());
    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: DEFINITION,
                name: "Intervening edit".to_owned(),
            },)
    );
    shell.settle();
    let assistant_confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&assistant_confirm);
    let after_intervening_edit = stamp(&shell);
    confirm(&mut shell);
    assert_eq!(stamp(&shell), after_intervening_edit);
    assert!(matches!(
        shell.app().document_snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 35.0
    ));

    click_preview(&mut shell, "feature-history-preview-suppress");
    let before_escape = stamp(&shell);
    assert!(shell.app().feature_history_preview_pending());
    shell.press_key(Key::Escape);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before_escape);

    click_preview(&mut shell, "feature-history-preview-suppress");
    let before_suppress = stamp(&shell);
    confirm(&mut shell);
    assert_eq!(shell.app().undo_step_count(), before_suppress.2 + 1);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, BODY),
        Some(&BTreeSet::from([EXTRUSION]))
    );
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

    click_preview(&mut shell, "feature-history-preview-resume");
    let before_resume = stamp(&shell);
    confirm(&mut shell);
    assert_eq!(shell.app().undo_step_count(), before_resume.2 + 1);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, BODY),
        None
    );

    let persisted_digest = shell.app().canonical_digest();
    let persisted_revision = shell.app().document_revision();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().document_revision(), persisted_revision);
    assert!(!shell.app().can_undo());
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, BODY),
        None
    );
    assert!(matches!(
        shell.app().document_snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 35.0
    ));
}

#[test]
fn hidden_body_selection_and_cancel_are_observational_through_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("hidden-body.ketchup");
    write_multibody_fixture(&fixture, false);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    open_history(&mut shell);
    let before = stamp(&shell);

    let hidden = body_label(&shell, TOOL_BODY, "feature-history-body-hidden");
    shell.click_role_and_label(Role::Button, &hidden);
    assert_eq!(
        shell.app().feature_history_selected_body_id(),
        Some(TOOL_BODY)
    );
    let tool = feature_label(&shell, TOOL_EXTRUSION);
    shell.click_role_and_label(Role::Button, &tool);
    assert_eq!(
        shell.app().feature_history_selected_feature_id(),
        Some(TOOL_EXTRUSION)
    );
    assert_eq!(stamp(&shell), before);

    click_preview(&mut shell, "feature-history-preview-suppress");
    assert!(shell.app().feature_history_preview_pending());
    let cancel = shell.catalog().text("feature-history-cancel");
    shell.click_button_label(&cancel);
    assert_eq!(stamp(&shell), before);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .suppressed_feature_ids(DEFINITION, TOOL_BODY),
        None
    );
}

#[test]
fn cross_body_invalid_boundary_and_invalid_value_preserve_history_and_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("cross-body.ketchup");
    write_multibody_fixture(&fixture, true);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    open_history(&mut shell);

    let tool_body = body_label(&shell, TOOL_BODY, "feature-history-body-visible");
    shell.click_role_and_label(Role::Button, &tool_body);
    let tool = feature_label(&shell, TOOL_EXTRUSION);
    shell.click_role_and_label(Role::Button, &tool);
    let before_boundary = stamp(&shell);
    let features_before = [EXTRUSION, TOOL_EXTRUSION, UNION]
        .map(|id| shell.app().document_snapshot().feature(id).unwrap().clone());
    click_preview(&mut shell, "feature-history-preview-suppress");
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before_boundary);
    for (id, feature) in [EXTRUSION, TOOL_EXTRUSION, UNION]
        .into_iter()
        .zip(features_before.iter())
    {
        assert_eq!(shell.app().document_snapshot().feature(id), Some(feature));
    }

    let active = body_label(&shell, BODY, "feature-history-body-active");
    shell.click_role_and_label(Role::Button, &active);
    let base = feature_label(&shell, EXTRUSION);
    shell.click_role_and_label(Role::Button, &base);
    let before_invalid = stamp(&shell);
    replace_exact_value(&mut shell, "not-a-dimension");
    click_preview(&mut shell, "feature-history-preview-edit");
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before_invalid);
    assert!(matches!(
        shell.app().document_snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 8.0
    ));
}
