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
const FORK_EXTRUSION: FeatureId = FeatureId(6);
const FORK_CUT_PROFILE: FeatureId = FeatureId(7);
const FORK_POCKET: FeatureId = FeatureId(8);
const REPLACEMENT_SOURCE: DefinitionId = DefinitionId(101);
const REPLACEMENT_TARGET: DefinitionId = DefinitionId(102);
const REPLACEMENT_SOURCE_PROFILE: FeatureId = FeatureId(110);
const REPLACEMENT_SOURCE_EXTRUSION: FeatureId = FeatureId(111);
const REPLACEMENT_TARGET_PROFILE: FeatureId = FeatureId(120);
const REPLACEMENT_TARGET_EXTRUSION: FeatureId = FeatureId(121);
const REPLACEMENT_SELECTED: OccurrenceId = OccurrenceId(1100);
const REPLACEMENT_SIBLING: OccurrenceId = OccurrenceId(1101);
const REPLACEMENT_TARGET_OCCURRENCE: OccurrenceId = OccurrenceId(1200);
const REPLACEMENT_PLANAR_MATE: AssemblyMateId = AssemblyMateId(1300);
const REPLACEMENT_AXIAL_MATE: AssemblyMateId = AssemblyMateId(1301);
const REPLACEMENT_SHEET: DrawingSheetId = DrawingSheetId(1400);

#[derive(Clone, Copy)]
enum ReplacementFixture {
    Complete,
    Failed,
    Hidden,
    Incompatible,
    UnderConstrained,
    OverConstrained,
    Lost,
    UnsupportedMate,
}

fn open_history(shell: &mut Shell) {
    let definition = shell.catalog().text("feature-history-definition");
    if !shell.has_role_and_label(Role::ComboBox, &definition) {
        let title = shell.catalog().text("feature-history-title");
        shell.click_role_and_label(Role::Button, &title);
    }
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
    body_label_for(shell, DEFINITION, body_id, status_key)
}

fn body_label_for(
    shell: &Shell,
    definition_id: DefinitionId,
    body_id: BodyId,
    status_key: &str,
) -> String {
    let snapshot = shell.app().document_snapshot();
    let body = snapshot
        .definition(definition_id)
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
    wait_for_exact_bodies(shell, 1);
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

fn write_component_replacement_fixture(path: &Path, variant: ReplacementFixture) {
    let mut document = DocumentStore::new();
    let target_transform = if matches!(variant, ReplacementFixture::OverConstrained) {
        Transform::from_translation(55.0, 6.0, 0.0).unwrap()
    } else {
        Transform::from_translation(55.0, 6.0, 7.0).unwrap()
    };
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: REPLACEMENT_SOURCE,
            name: "Verifier replacement source".to_owned(),
        },
        CanonicalCommand::CreateDefinition {
            id: REPLACEMENT_TARGET,
            name: "Verifier compatible target".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: REPLACEMENT_SOURCE_PROFILE,
            definition_id: REPLACEMENT_SOURCE,
            name: "Source profile".to_owned(),
            kind: profile(10.0),
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
            name: "Selected replacement source".to_owned(),
            transform: Transform::from_translation(5.0, 6.0, 7.0).unwrap(),
            parent: None,
            tag: None,
            visible: !matches!(variant, ReplacementFixture::Hidden),
        },
        CanonicalCommand::CreateOccurrence {
            id: REPLACEMENT_SIBLING,
            definition_id: REPLACEMENT_SOURCE,
            name: "Unchanged source sibling".to_owned(),
            transform: Transform::from_translation(30.0, 6.0, 7.0).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::CreateOccurrence {
            id: REPLACEMENT_TARGET_OCCURRENCE,
            definition_id: REPLACEMENT_TARGET,
            name: "Unchanged target occurrence".to_owned(),
            transform: target_transform,
            parent: None,
            tag: None,
            visible: true,
        },
    ];
    if matches!(variant, ReplacementFixture::Incompatible) {
        commands.push(CanonicalCommand::CreateFeature {
            id: FeatureId(122),
            definition_id: REPLACEMENT_TARGET,
            name: "Unmatched target feature".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[1.0, 1.0], [2.0, 1.0], [1.0, 2.0]],
            },
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

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
    if matches!(variant, ReplacementFixture::Failed) {
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::DeleteOccurrence {
                    id: REPLACEMENT_TARGET_OCCURRENCE,
                },
                CanonicalCommand::SetOccurrenceGrounded {
                    id: REPLACEMENT_SELECTED,
                    grounded: true,
                },
                CanonicalCommand::SetOccurrenceGrounded {
                    id: REPLACEMENT_SIBLING,
                    grounded: true,
                },
            ]))
            .unwrap();
        document.discard_history_before_current();
        persistence::save_atomic(path, &document.current()).unwrap();
        return;
    }
    let selected_endpoint = AssemblyMateEndpoint::resolved(
        REPLACEMENT_SELECTED,
        source.reference(ExactFaceRole::Top).unwrap().clone(),
    );
    let planar_kind = if matches!(variant, ReplacementFixture::UnsupportedMate) {
        AssemblyMateKind::Distance { distance_mm: 4.0 }
    } else {
        AssemblyMateKind::CoincidentPlanar {
            offset_mm: 0.0,
            reversed: false,
        }
    };
    let mut dependency_commands = vec![CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
        REPLACEMENT_PLANAR_MATE,
        selected_endpoint,
        AssemblyMateEndpoint::resolved(
            REPLACEMENT_TARGET_OCCURRENCE,
            target.reference(ExactFaceRole::Bottom).unwrap().clone(),
        ),
        planar_kind,
    ))];
    if !matches!(variant, ReplacementFixture::UnderConstrained) {
        dependency_commands.push(CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
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
        )));
    }
    if !matches!(variant, ReplacementFixture::UnderConstrained) {
        dependency_commands.push(CanonicalCommand::SetOccurrenceGrounded {
            id: REPLACEMENT_SELECTED,
            grounded: true,
        });
    }
    dependency_commands.extend([
        CanonicalCommand::SetOccurrenceGrounded {
            id: REPLACEMENT_SIBLING,
            grounded: true,
        },
        CanonicalCommand::SetOccurrenceGrounded {
            id: REPLACEMENT_TARGET_OCCURRENCE,
            grounded: true,
        },
    ]);
    if !matches!(
        variant,
        ReplacementFixture::UnderConstrained | ReplacementFixture::Lost
    ) {
        dependency_commands.push(CanonicalCommand::CreateDrawingSheet(
            DrawingSheet::new(
                REPLACEMENT_SHEET,
                "Verifier replacement assembly",
                DrawingSource::RigidAssembly {
                    occurrence_ids: vec![
                        REPLACEMENT_SELECTED,
                        REPLACEMENT_SIBLING,
                        REPLACEMENT_TARGET_OCCURRENCE,
                    ],
                },
            )
            .unwrap(),
        ));
    }
    document
        .apply_batch(&CommandBatch::new(dependency_commands))
        .unwrap();
    if matches!(variant, ReplacementFixture::Lost) {
        let mate = document
            .current()
            .assembly_mate(REPLACEMENT_PLANAR_MATE)
            .unwrap()
            .clone();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::RebindAssemblyMate(AssemblyMate::new(
                    REPLACEMENT_PLANAR_MATE,
                    AssemblyMateEndpoint::lost(
                        REPLACEMENT_SELECTED,
                        source.reference(ExactFaceRole::Top).unwrap().clone(),
                    ),
                    mate.endpoint_b().clone(),
                    mate.kind(),
                )),
            ]))
            .unwrap();
    }
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn write_shared_dependency_fixture(path: &Path) {
    write_shared_fixture(path, false, true);
}

fn write_shared_history_fixture(path: &Path, suppressed: bool) {
    write_shared_fixture(path, suppressed, false);
}

fn write_shared_fixture(path: &Path, suppressed: bool, with_dependencies: bool) {
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
    if suppressed {
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetBodyFeatureSuppression {
                    definition_id: DEFINITION,
                    body_id: BODY,
                    suppressed_feature_ids: vec![CUT_PROFILE, POCKET],
                },
            ]))
            .unwrap();
    }
    if with_dependencies {
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
    }
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

fn mate_fingerprints_from(mates: &[AssemblyMate]) -> Vec<String> {
    mates
        .iter()
        .flat_map(|mate| [mate.endpoint_a(), mate.endpoint_b()])
        .map(|endpoint| endpoint.reference().result_fingerprint.clone())
        .collect()
}

fn occurrence_mate_fingerprints(shell: &Shell, occurrence_id: OccurrenceId) -> Vec<String> {
    shell
        .app()
        .document_snapshot()
        .assembly_mates()
        .flat_map(|mate| [mate.endpoint_a(), mate.endpoint_b()])
        .filter(|endpoint| endpoint.occurrence_id() == occurrence_id)
        .map(|endpoint| endpoint.reference().result_fingerprint.clone())
        .collect()
}

fn select_make_unique(shell: &mut Shell) {
    let label = shell
        .catalog()
        .text("feature-history-change-scope-make-unique");
    shell.click_role_and_label(Role::RadioButton, &label);
}

fn select_component_replacement(shell: &mut Shell) {
    let label = shell
        .catalog()
        .text("feature-history-change-scope-replace-component");
    shell.click_role_and_label(Role::RadioButton, &label);
    let target = shell.catalog().text("feature-history-replacement-target");
    shell.click_role_and_label(Role::ComboBox, &target);
    shell.click_button_label("Verifier compatible target");
}

fn preview_component_replacement(shell: &mut Shell) {
    click_preview(shell, "feature-history-preview-replace-component");
}

fn select_definition(shell: &mut Shell, name: &str) {
    let label = shell.catalog().text("feature-history-definition");
    shell.click_role_and_label(Role::ComboBox, &label);
    shell.click_button_label(name);
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
fn component_replacement_serial_accesskit_replay_is_atomic_local_exportable_and_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("replacement-verifier-source.ketchup");
    let saved = directory.path().join("replacement-verifier-saved.ketchup");
    let stl = directory.path().join("replacement-verifier.stl");
    let step = directory.path().join("replacement-verifier.step");
    write_component_replacement_fixture(&fixture, ReplacementFixture::Complete);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .queue_export(&stl)
        .queue_export(&step)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_confirm_high_risk_as(455)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 2);

    shell.click_at(shell.top_face_centre(REPLACEMENT_SELECTED.0));
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    open_history(&mut shell);
    select_component_replacement(&mut shell);

    let source = stamp(&shell);
    let source_snapshot = shell.app().document_snapshot();
    let source_definition = source_snapshot
        .definition(REPLACEMENT_SOURCE)
        .unwrap()
        .clone();
    let target_definition = source_snapshot
        .definition(REPLACEMENT_TARGET)
        .unwrap()
        .clone();
    let selected_before = source_snapshot
        .occurrence(REPLACEMENT_SELECTED)
        .unwrap()
        .clone();
    let sibling_before = source_snapshot
        .occurrence(REPLACEMENT_SIBLING)
        .unwrap()
        .clone();
    let target_occurrence_before = source_snapshot
        .occurrence(REPLACEMENT_TARGET_OCCURRENCE)
        .unwrap()
        .clone();
    let mates_before = [REPLACEMENT_PLANAR_MATE, REPLACEMENT_AXIAL_MATE]
        .map(|id| source_snapshot.assembly_mate(id).unwrap().clone());
    let render_before = shell.app().exact_render_bounds();

    preview_component_replacement(&mut shell);
    assert!(shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), source);
    assert_eq!(
        shell.app().feature_history_replacement_identity(),
        Some([1100, 101, 102])
    );
    assert_eq!(
        shell.app().feature_history_replacement_impact_counts(),
        Some([1, 2, 3, 1, 1, 2, 3, 1, 2])
    );
    let cancel = shell.catalog().text("feature-history-cancel");
    shell.click_button_label(&cancel);
    assert_eq!(stamp(&shell), source);
    assert_eq!(
        mate_fingerprints(&shell),
        mate_fingerprints_from(&mates_before)
    );
    assert_eq!(shell.app().exact_render_bounds(), render_before);

    preview_component_replacement(&mut shell);
    shell.press_key(Key::Escape);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), source);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );

    preview_component_replacement(&mut shell);
    confirm(&mut shell);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().document_revision(), source.0 + 1);
    assert_eq!(shell.app().undo_step_count(), source.2 + 1);
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
        Some(&sibling_before)
    );
    assert_eq!(
        replaced.occurrence(REPLACEMENT_TARGET_OCCURRENCE),
        Some(&target_occurrence_before)
    );
    let selected_after = replaced.occurrence(REPLACEMENT_SELECTED).unwrap();
    assert_eq!(selected_after.id(), selected_before.id());
    assert_eq!(selected_after.transform(), selected_before.transform());
    assert_eq!(selected_after.parent(), selected_before.parent());
    assert_eq!(selected_after.tag(), selected_before.tag());
    assert_eq!(selected_after.visible(), selected_before.visible());
    let mates_after = [REPLACEMENT_PLANAR_MATE, REPLACEMENT_AXIAL_MATE]
        .map(|id| replaced.assembly_mate(id).unwrap().clone());
    for (before, after) in mates_before.iter().zip(&mates_after) {
        assert_eq!(after.endpoint_b(), before.endpoint_b());
        assert_eq!(after.endpoint_a().occurrence_id(), REPLACEMENT_SELECTED);
        assert_eq!(
            after.endpoint_a().reference().definition_id,
            REPLACEMENT_TARGET
        );
    }
    assert_ne!(
        mate_fingerprints_from(&mates_after),
        mate_fingerprints_from(&mates_before)
    );
    select_definition(&mut shell, "Verifier compatible target");
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );
    assert_eq!(shell.app().exact_render_bounds(), render_before);
    let replaced_digest = shell.app().canonical_digest();

    let before_exports = stamp(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(stamp(&shell), before_exports);
    assert!(stl.is_file(), "{}", shell.app().action_digest());
    assert!(step.is_file(), "{}", shell.app().action_digest());

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().canonical_digest(), source.1);
    assert_eq!(
        shell.app().occurrence_definition_id(REPLACEMENT_SELECTED),
        Some(REPLACEMENT_SOURCE)
    );
    assert_eq!(
        mate_fingerprints(&shell),
        mate_fingerprints_from(&mates_before)
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().canonical_digest(), replaced_digest);
    assert_eq!(
        mate_fingerprints(&shell),
        mate_fingerprints_from(&mates_after)
    );

    let persisted_revision = shell.app().document_revision();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().canonical_digest(), replaced_digest);
    assert_eq!(shell.app().document_revision(), persisted_revision);
    assert!(!shell.app().can_undo());
    assert_eq!(
        shell.app().occurrence_definition_id(REPLACEMENT_SELECTED),
        Some(REPLACEMENT_TARGET)
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(REPLACEMENT_SIBLING),
        Some(&sibling_before)
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(REPLACEMENT_TARGET_OCCURRENCE),
        Some(&target_occurrence_before)
    );
    assert_eq!(
        mate_fingerprints(&shell),
        mate_fingerprints_from(&mates_after)
    );
    open_history(&mut shell);
    select_definition(&mut shell, "Verifier compatible target");
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );
}

#[test]
fn component_replacement_stale_confirm_preserves_intervening_state_and_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("replacement-stale.ketchup");
    write_component_replacement_fixture(&fixture, ReplacementFixture::Complete);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 2);
    shell.click_at(shell.top_face_centre(REPLACEMENT_SELECTED.0));
    open_history(&mut shell);
    select_component_replacement(&mut shell);
    preview_component_replacement(&mut shell);
    assert!(shell.app().feature_history_preview_pending());

    assert!(
        shell
            .app_mut()
            .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                target: REPLACEMENT_SOURCE,
                name: "Intervening source name".to_owned(),
            },)
    );
    assert!(shell.app_mut().confirm_assistant_proposal());
    shell.settle();
    let intervening = stamp(&shell);
    let bounds = shell.app().exact_render_bounds();
    let mates = mate_fingerprints(&shell);
    confirm(&mut shell);

    assert_eq!(stamp(&shell), intervening);
    assert_eq!(
        shell.app().occurrence_definition_id(REPLACEMENT_SELECTED),
        Some(REPLACEMENT_SOURCE)
    );
    assert_eq!(shell.app().exact_render_bounds(), bounds);
    assert_eq!(mate_fingerprints(&shell), mates);
    assert!(shell.app().action_digest().contains("stale"));
}

#[test]
fn component_replacement_invalid_dependency_paths_fail_closed_through_accesskit() {
    for (variant, name, diagnostic) in [
        (
            ReplacementFixture::Incompatible,
            "incompatible",
            "different feature counts",
        ),
        (
            ReplacementFixture::UnderConstrained,
            "under-constrained",
            "UnderConstrained",
        ),
        (
            ReplacementFixture::OverConstrained,
            "over-constrained",
            "OverConstrained",
        ),
        (ReplacementFixture::Lost, "lost", "lost exact reference"),
        (
            ReplacementFixture::UnsupportedMate,
            "unsupported-mate",
            "not planar or axial",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join(format!("replacement-{name}.ketchup"));
        write_component_replacement_fixture(&fixture, variant);
        let dialogs = ScriptedFileDialogs::new()
            .queue_open(&fixture)
            .always_discard();
        let mut shell = Shell::with_dialogs(dialogs);
        shell.click_menu_command("menu-file", AppCommand::Open);
        shell
            .app_mut()
            .connect_exact_worker(exact_worker_path())
            .unwrap();
        wait_for_exact_bodies(&mut shell, 2);
        shell.click_at(shell.top_face_centre(REPLACEMENT_SELECTED.0));
        assert_eq!(shell.app().selected_occurrence_count(), 1, "{name}");
        open_history(&mut shell);
        select_component_replacement(&mut shell);
        let before = stamp(&shell);
        let bounds = shell.app().exact_render_bounds();
        let mates = mate_fingerprints(&shell);
        preview_component_replacement(&mut shell);

        assert!(
            !shell.app().feature_history_preview_pending(),
            "{name}: {}",
            shell.app().action_digest()
        );
        assert_eq!(stamp(&shell), before, "{name}");
        assert_eq!(shell.app().exact_render_bounds(), bounds, "{name}");
        if matches!(variant, ReplacementFixture::Lost) {
            let snapshot = shell.app().document_snapshot();
            let mate = snapshot.assembly_mate(REPLACEMENT_PLANAR_MATE).unwrap();
            assert_eq!(
                mate.endpoint_a().health(),
                ketchup_core::assembly::AssemblyReferenceHealth::Lost
            );
        } else {
            assert_eq!(mate_fingerprints(&shell), mates, "{name}");
        }
        assert!(
            shell.app().action_digest().contains(diagnostic),
            "{name}: {}",
            shell.app().action_digest()
        );
    }
}

#[test]
fn component_replacement_hidden_self_cross_document_and_failed_paths_are_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let hidden_fixture = directory.path().join("replacement-hidden.ketchup");
    write_component_replacement_fixture(&hidden_fixture, ReplacementFixture::Hidden);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&hidden_fixture)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 2);
    open_history(&mut shell);
    let replace = shell
        .catalog()
        .text("feature-history-change-scope-replace-component");
    shell.click_role_and_label(Role::RadioButton, &replace);
    let target = shell.catalog().text("feature-history-replacement-target");
    shell.click_role_and_label(Role::ComboBox, &target);
    assert!(shell.has_role_and_label(Role::Button, "Verifier compatible target"));
    assert!(!shell.has_role_and_label(Role::Button, "Verifier replacement source"));
    assert!(!shell.has_role_and_label(Role::Button, "External document target"));
    shell.click_button_label("Verifier compatible target");
    let before = stamp(&shell);
    let preview = shell
        .catalog()
        .text("feature-history-preview-replace-component");
    assert!(shell.has_role_and_label(Role::Button, &preview));
    shell.click_role_and_label(Role::Button, &preview);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), before);
    assert_eq!(
        shell.app().occurrence_definition_id(REPLACEMENT_SELECTED),
        Some(REPLACEMENT_SOURCE)
    );

    let failed_fixture = directory.path().join("replacement-worker-failed.ketchup");
    write_component_replacement_fixture(&failed_fixture, ReplacementFixture::Failed);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&failed_fixture)
        .always_discard();
    let mut failed = Shell::with_dialogs(dialogs);
    failed.click_menu_command("menu-file", AppCommand::Open);
    failed
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_body(&mut failed);
    failed.click_at(failed.top_face_centre(REPLACEMENT_SELECTED.0));
    open_history(&mut failed);
    select_component_replacement(&mut failed);
    failed
        .app_mut()
        .headless_force_exact_worker_path(directory.path().join("missing-worker.exe"));
    let before = stamp(&failed);
    preview_component_replacement(&mut failed);
    assert!(!failed.app().feature_history_preview_pending());
    assert_eq!(stamp(&failed), before);
    assert_eq!(
        failed.app().occurrence_definition_id(REPLACEMENT_SELECTED),
        Some(REPLACEMENT_SOURCE)
    );
    assert!(
        failed.app().action_digest().contains("worker spawn failed"),
        "{}",
        failed.app().action_digest()
    );
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
fn make_unique_serial_accesskit_replay_rebinds_locally_exports_and_persists() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("make-unique-source.ketchup");
    let saved = directory.path().join("make-unique-saved.ketchup");
    let stl = directory.path().join("make-unique.stl");
    let step = directory.path().join("make-unique.step");
    write_shared_dependency_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .queue_export(&stl)
        .queue_export(&step)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_confirm_high_risk_as(452)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_body(&mut shell);

    shell.click_at(shell.top_face_centre(SECOND.0));
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
    select_make_unique(&mut shell);

    let source = stamp(&shell);
    let source_transforms = [FIRST, SECOND].map(|id| {
        shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()
    });
    let source_first_mates = occurrence_mate_fingerprints(&shell, FIRST);
    let source_second_mates = occurrence_mate_fingerprints(&shell, SECOND);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );

    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    assert_eq!(shell.app().feature_history_fork_identity(), Some([2, 1, 2]));
    assert_eq!(
        shell.app().feature_history_fork_impact_counts(),
        Some([1, 1, 2, 3, 1, 2])
    );
    assert_eq!(stamp(&shell), source);
    assert!(shell.has_visible_label(&shell.catalog().text("feature-history-fork-impact-title")));
    let cancel = shell.catalog().text("feature-history-cancel");
    shell.click_button_label(&cancel);
    assert_eq!(stamp(&shell), source);
    assert_eq!(
        occurrence_mate_fingerprints(&shell, SECOND),
        source_second_mates
    );

    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    shell.press_key(Key::Escape);
    assert!(!shell.app().feature_history_preview_pending());
    assert_eq!(stamp(&shell), source);

    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    confirm(&mut shell);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().document_revision(), source.0 + 1);
    assert_eq!(shell.app().undo_step_count(), source.2 + 1);
    assert_eq!(shell.app().definition_count(), 2);
    assert_eq!(
        shell.app().occurrence_definition_id(FIRST),
        Some(DEFINITION)
    );
    assert_eq!(
        shell.app().occurrence_definition_id(SECOND),
        Some(DefinitionId(2))
    );
    assert!(matches!(
        shell.app().document_snapshot().feature(EXTRUSION).unwrap().kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 20.0
    ));
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(FORK_EXTRUSION)
            .unwrap()
            .kind(),
        FeatureKind::Extrusion { height, .. } if height.millimetres() == 35.0
    ));
    assert_eq!(
        [FIRST, SECOND].map(|id| shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()),
        source_transforms
    );
    assert_eq!(
        occurrence_mate_fingerprints(&shell, FIRST),
        source_first_mates
    );
    let fork_second_mates = occurrence_mate_fingerprints(&shell, SECOND);
    assert_ne!(fork_second_mates, source_second_mates);
    let mut heights = shell
        .app()
        .exact_render_bounds()
        .into_iter()
        .map(|bounds| bounds[1][2])
        .collect::<Vec<_>>();
    heights.sort_by(f64::total_cmp);
    assert_eq!(heights, vec![20.0, 35.0]);
    let fork_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    wait_for_exact_body(&mut shell);
    assert_eq!(shell.app().canonical_digest(), source.1);
    assert_eq!(shell.app().definition_count(), 1);
    assert_eq!(
        shell.app().occurrence_definition_id(SECOND),
        Some(DEFINITION)
    );
    assert_eq!(
        occurrence_mate_fingerprints(&shell, SECOND),
        source_second_mates
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().canonical_digest(), fork_digest);
    assert_eq!(
        occurrence_mate_fingerprints(&shell, SECOND),
        fork_second_mates
    );

    let before_exports = stamp(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(stamp(&shell), before_exports);
    assert!(stl.is_file(), "{}", shell.app().action_digest());
    assert!(step.is_file(), "{}", shell.app().action_digest());

    let persisted_digest = shell.app().canonical_digest();
    let persisted_revision = shell.app().document_revision();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().document_revision(), persisted_revision);
    assert!(!shell.app().can_undo());
    assert_eq!(
        shell.app().occurrence_definition_id(FIRST),
        Some(DEFINITION)
    );
    assert_eq!(
        shell.app().occurrence_definition_id(SECOND),
        Some(DefinitionId(2))
    );
    assert_eq!(
        occurrence_mate_fingerprints(&shell, FIRST),
        source_first_mates
    );
    assert_eq!(
        occurrence_mate_fingerprints(&shell, SECOND),
        fork_second_mates
    );
    open_history(&mut shell);
    let fork_name = shell.catalog().format(
        "model-unique-name",
        &BTreeMap::from([("name", "Shared verifier part".to_owned())]),
    );
    select_definition(&mut shell, &fork_name);
    assert_eq!(
        shell.app().feature_history_current_dependency_counts(),
        Some([2, 3])
    );
}

#[test]
fn make_unique_suffix_suppress_and_resume_are_selected_occurrence_only() {
    for (source_suppressed, preview_key) in [
        (false, "feature-history-preview-suppress"),
        (true, "feature-history-preview-resume"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join(if source_suppressed {
            "make-unique-resume.ketchup"
        } else {
            "make-unique-suppress.ketchup"
        });
        write_shared_history_fixture(&fixture, source_suppressed);
        let dialogs = ScriptedFileDialogs::new()
            .queue_open(&fixture)
            .always_discard();
        let mut shell = Shell::with_dialogs(dialogs);
        shell.click_menu_command("menu-file", AppCommand::Open);
        shell
            .app_mut()
            .connect_exact_worker(exact_worker_path())
            .unwrap();
        wait_for_exact_body(&mut shell);
        shell.click_at(shell.top_face_centre(SECOND.0));
        open_history(&mut shell);
        let selected = feature_label(
            &shell,
            if source_suppressed {
                EXTRUSION
            } else {
                CUT_PROFILE
            },
        );
        shell.click_role_and_label(Role::Button, &selected);
        select_make_unique(&mut shell);

        let source = stamp(&shell);
        let source_transforms = [FIRST, SECOND].map(|id| {
            shell
                .app()
                .document_snapshot()
                .occurrence(id)
                .unwrap()
                .transform()
        });
        click_preview(&mut shell, preview_key);
        assert_eq!(shell.app().feature_history_fork_identity(), Some([2, 1, 2]));
        assert_eq!(
            shell.app().feature_history_fork_impact_counts(),
            Some([1, 1, 0, 0, 1, 2])
        );
        assert_eq!(stamp(&shell), source);
        confirm(&mut shell);
        wait_for_exact_bodies(&mut shell, 2);

        assert_eq!(shell.app().document_revision(), source.0 + 1);
        assert_eq!(shell.app().undo_step_count(), source.2 + 1);
        assert_eq!(
            shell.app().occurrence_definition_id(FIRST),
            Some(DEFINITION)
        );
        assert_eq!(
            shell.app().occurrence_definition_id(SECOND),
            Some(DefinitionId(2))
        );
        let expected_source = source_suppressed.then(|| BTreeSet::from([CUT_PROFILE, POCKET]));
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .suppressed_feature_ids(DEFINITION, BODY),
            expected_source.as_ref()
        );
        let expected_fork =
            (!source_suppressed).then(|| BTreeSet::from([FORK_CUT_PROFILE, FORK_POCKET]));
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .suppressed_feature_ids(DefinitionId(2), BODY),
            expected_fork.as_ref()
        );
        assert_eq!(
            [FIRST, SECOND].map(|id| shell
                .app()
                .document_snapshot()
                .occurrence(id)
                .unwrap()
                .transform()),
            source_transforms
        );
        let expected_producers = if source_suppressed {
            vec![EXTRUSION, FORK_POCKET]
        } else {
            vec![POCKET, FORK_EXTRUSION]
        };
        assert_eq!(shell.app().exact_current_producer_ids(), expected_producers);
        let fork_digest = shell.app().canonical_digest();

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        wait_for_exact_body(&mut shell);
        assert_eq!(shell.app().canonical_digest(), source.1);
        assert_eq!(
            shell.app().occurrence_definition_id(SECOND),
            Some(DEFINITION)
        );
        assert_eq!(
            [FIRST, SECOND].map(|id| shell
                .app()
                .document_snapshot()
                .occurrence(id)
                .unwrap()
                .transform()),
            source_transforms
        );
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        wait_for_exact_bodies(&mut shell, 2);
        assert_eq!(shell.app().canonical_digest(), fork_digest);
        assert_eq!(
            [FIRST, SECOND].map(|id| shell
                .app()
                .document_snapshot()
                .occurrence(id)
                .unwrap()
                .transform()),
            source_transforms
        );
    }
}

#[test]
fn make_unique_lost_dependency_refuses_atomically_and_keeps_last_valid_export() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("make-unique-lost.ketchup");
    let stl = directory.path().join("make-unique-lost-last-valid.stl");
    write_shared_dependency_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .queue_export(&stl)
        .always_confirm_high_risk_as(453)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_body(&mut shell);
    shell.click_at(shell.top_face_centre(SECOND.0));
    open_history(&mut shell);
    let cut_profile = feature_label(&shell, CUT_PROFILE);
    shell.click_role_and_label(Role::Button, &cut_profile);
    select_make_unique(&mut shell);

    let before = stamp(&shell);
    let bounds_before = shell.app().exact_render_bounds();
    let mates_before = mate_fingerprints(&shell);
    let transforms_before = [FIRST, SECOND].map(|id| {
        shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()
    });
    click_preview(&mut shell, "feature-history-preview-suppress");
    assert_eq!(
        shell.app().feature_history_fork_impact_counts(),
        Some([1, 1, 2, 3, 1, 2])
    );
    confirm(&mut shell);
    assert_eq!(stamp(&shell), before);
    assert_eq!(shell.app().definition_count(), 1);
    assert_eq!(
        shell.app().occurrence_definition_id(SECOND),
        Some(DEFINITION)
    );
    assert_eq!(shell.app().exact_render_bounds(), bounds_before);
    assert_eq!(mate_fingerprints(&shell), mates_before);
    assert_eq!(
        [FIRST, SECOND].map(|id| shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()),
        transforms_before
    );
    assert!(shell.app().action_digest().contains("Lost"));

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert!(stl.is_file(), "{}", shell.app().action_digest());
    assert_eq!(stamp(&shell), before);
}

#[test]
fn make_unique_worker_failure_preserves_history_dependencies_and_last_valid_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("make-unique-worker-failure.ketchup");
    let stl = directory.path().join("make-unique-worker-last-valid.stl");
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
        .always_confirm_high_risk_as(454)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell.app_mut().connect_exact_worker(&worker).unwrap();
    wait_for_exact_body(&mut shell);
    shell.click_at(shell.top_face_centre(SECOND.0));
    open_history(&mut shell);
    let extrusion = feature_label(&shell, EXTRUSION);
    shell.click_role_and_label(Role::Button, &extrusion);
    select_make_unique(&mut shell);

    let before = stamp(&shell);
    let bounds_before = shell.app().exact_render_bounds();
    let mates_before = mate_fingerprints(&shell);
    replace_exact_value(&mut shell, "35");
    click_preview(&mut shell, "feature-history-preview-edit");
    assert_eq!(
        shell.app().feature_history_fork_impact_counts(),
        Some([1, 1, 2, 3, 1, 2])
    );
    std::fs::rename(&worker, &parked_worker).unwrap();
    confirm(&mut shell);
    assert_eq!(stamp(&shell), before);
    assert_eq!(shell.app().definition_count(), 1);
    assert_eq!(
        shell.app().occurrence_definition_id(SECOND),
        Some(DEFINITION)
    );
    assert_eq!(shell.app().exact_render_bounds(), bounds_before);
    assert_eq!(mate_fingerprints(&shell), mates_before);

    std::fs::rename(&parked_worker, &worker).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert!(stl.is_file(), "{}", shell.app().action_digest());
    assert_eq!(stamp(&shell), before);
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
    let before_intervening_edit = stamp(&shell);
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
    assert_eq!(after_intervening_edit.0, before_intervening_edit.0 + 1);
    assert_eq!(after_intervening_edit.2, before_intervening_edit.2 + 1);
    assert!(shell.app().assistant_proposal().is_none());
    assert!(shell.app().feature_history_preview_pending());
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .definition(DEFINITION)
            .unwrap()
            .name(),
        "Intervening edit"
    );
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
fn single_use_definition_never_offers_make_unique_or_mutates_history() {
    let mut shell = Shell::new();
    open_history(&mut shell);
    let before = stamp(&shell);
    let make_unique = shell
        .catalog()
        .text("feature-history-change-scope-make-unique");
    assert!(!shell.has_role_and_label(Role::RadioButton, &make_unique));
    let extrusion = feature_label(&shell, EXTRUSION);
    shell.click_role_and_label(Role::Button, &extrusion);
    assert_eq!(stamp(&shell), before);
    assert_eq!(shell.app().definition_count(), 1);
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
