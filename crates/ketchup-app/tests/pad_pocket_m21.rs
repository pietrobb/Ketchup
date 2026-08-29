//! Program 2 Pad/Pocket workflows replayed offscreen through AccessKit.

mod harness;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{Key, accesskit::Role};
use harness::{Shell, ctrl};
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::assembly::AssemblySolveStatus;
use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind, InstancePath, OccurrenceId, Transform,
};
use ketchup_core::exact_product::{ExactFaceRole, ExactFeatureChainRequest};
use ketchup_core::intent::WorkflowIntent;
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, PocketSpec, PrincipalPlane, SketchConstraint,
    SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId, SketchPointKind,
    SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use ketchup_interaction::{Ray, Vec3};
use ketchup_scheduler::ExactWorkerSupervisor;

const OFFSET_DEFINITION: DefinitionId = DefinitionId(100);
const OFFSET_BASE_PLANE: FeatureId = FeatureId(101);
const OFFSET_PLANE: FeatureId = FeatureId(102);
const OFFSET_SKETCH: FeatureId = FeatureId(103);
const OFFSET_PAD: FeatureId = FeatureId(104);
const OFFSET_OCCURRENCE: OccurrenceId = OccurrenceId(105);

const POCKET_DEFINITION: DefinitionId = DefinitionId(200);
const POCKET_BASE_PLANE: FeatureId = FeatureId(201);
const POCKET_BASE_SKETCH: FeatureId = FeatureId(202);
const POCKET_BASE_PAD: FeatureId = FeatureId(203);
const POCKET_FACE_PLANE: FeatureId = FeatureId(204);
const POCKET_SKETCH: FeatureId = FeatureId(205);
const POCKET: FeatureId = FeatureId(206);
const POCKET_OCCURRENCE: OccurrenceId = OccurrenceId(207);

fn point(entity: u64, point: SketchPointKind) -> SketchPointRef {
    SketchPointRef {
        entity: SketchEntityId(entity),
        point,
    }
}

fn rectangle_sketch(workplane: FeatureId, min_mm: [f64; 2], max_mm: [f64; 2]) -> SketchSpec {
    let corners = [
        min_mm,
        [max_mm[0], min_mm[1]],
        max_mm,
        [min_mm[0], max_mm[1]],
    ];
    let entities = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: corners[index],
            end_mm: corners[(index + 1) % corners.len()],
        })
        .collect::<Vec<_>>();
    let mut constraints = Vec::new();
    for index in 0..4 {
        let entity = index as u64 + 1;
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 1),
            kind: SketchConstraintKind::FixedPoint {
                point: point(entity, SketchPointKind::Start),
                position_mm: corners[index],
            },
        });
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 2),
            kind: SketchConstraintKind::FixedPoint {
                point: point(entity, SketchPointKind::End),
                position_mm: corners[(index + 1) % corners.len()],
            },
        });
    }
    SketchSpec {
        workplane,
        entities,
        constraints,
    }
}

fn write_fixture(path: &Path) {
    let offset_sketch = rectangle_sketch(OFFSET_PLANE, [0.0, 0.0], [20.0, 10.0]);
    let offset_region = offset_sketch.solved_regions().unwrap()[0].id;
    let base_sketch = rectangle_sketch(POCKET_BASE_PLANE, [10.0, 20.0], [110.0, 80.0]);
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: OFFSET_DEFINITION,
                name: "Offset Pad".into(),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET_BASE_PLANE,
                definition_id: OFFSET_DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET_PLANE,
                definition_id: OFFSET_DEFINITION,
                name: "Offset XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Offset {
                        base: OFFSET_BASE_PLANE,
                        distance: Dimension::from_decimal("5").unwrap(),
                    },
                    frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(5.0),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET_SKETCH,
                definition_id: OFFSET_DEFINITION,
                name: "Fixed rectangle".into(),
                kind: FeatureKind::Sketch(offset_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET_PAD,
                definition_id: OFFSET_DEFINITION,
                name: "Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: OFFSET_SKETCH,
                    region: offset_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("10").unwrap()),
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OFFSET_OCCURRENCE,
                definition_id: OFFSET_DEFINITION,
                name: "Offset Pad #1".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateDefinition {
                id: POCKET_DEFINITION,
                name: "Face Pocket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET_BASE_PLANE,
                definition_id: POCKET_DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET_BASE_SKETCH,
                definition_id: POCKET_DEFINITION,
                name: "Base rectangle".into(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET_BASE_PAD,
                definition_id: POCKET_DEFINITION,
                name: "Base Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: POCKET_BASE_SKETCH,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("18").unwrap()),
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: POCKET_OCCURRENCE,
                definition_id: POCKET_DEFINITION,
                name: "Face Pocket #1".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();

    let request =
        ExactFeatureChainRequest::from_snapshot(&document.current(), POCKET_DEFINITION).unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let package = worker.evaluate_rectangle(&request).unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }

    let pocket_sketch = rectangle_sketch(POCKET_FACE_PLANE, [30.0, 20.0], [50.0, 35.0]);
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: POCKET_FACE_PLANE,
                definition_id: POCKET_DEFINITION,
                name: "Pad top face".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame {
                        origin_mm: [10.0, 20.0, 18.0],
                        x_axis: [1.0, 0.0, 0.0],
                        y_axis: [0.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                }),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET_SKETCH,
                definition_id: POCKET_DEFINITION,
                name: "Pocket rectangle".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: POCKET_DEFINITION,
                name: "Pocket".into(),
                kind: FeatureKind::SketchPocket(PocketSpec {
                    target: POCKET_BASE_PAD,
                    sketch: POCKET_SKETCH,
                    region: pocket_region,
                    support: Box::new(top),
                    direction: FeatureDirection::OppositeNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("6").unwrap()),
                }),
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
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

fn wait_for_exact_bodies(shell: &mut Shell) {
    for _ in 0..150 {
        shell.settle();
        if shell.app().exact_render_body_count() == 2 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        shell.app().exact_render_body_count(),
        2,
        "visible={:?}, face_plane={:?}",
        shell.app().exact_current_producer_ids(),
        shell.app().document_snapshot().feature(POCKET_FACE_PLANE)
    );
}

fn sorted_bounds(shell: &Shell) -> Vec<[[f64; 3]; 2]> {
    let mut bounds = shell.app().exact_render_bounds();
    bounds.sort_by(|left, right| {
        left[0][0]
            .total_cmp(&right[0][0])
            .then(left[0][1].total_cmp(&right[0][1]))
    });
    bounds
}

fn assert_exact_scene(shell: &Shell, offset_z: f64, pocket_height: f64) {
    assert_eq!(
        sorted_bounds(shell),
        vec![
            [[0.0, 0.0, offset_z], [20.0, 10.0, offset_z + 10.0]],
            [[10.0, 20.0, 0.0], [110.0, 80.0, pocket_height]],
        ]
    );
    assert_eq!(
        shell.app().exact_render_triangle_count(),
        40,
        "current producers: {:?}",
        shell.app().exact_current_producer_ids()
    );
    assert_eq!(shell.app().exact_stable_reference_count(), 11);
}

fn prepare_dimension(shell: &mut Shell, target: FeatureId, value: &str) -> bool {
    let prepared = shell
        .app_mut()
        .prepare_assistant_intent(WorkflowIntent::SetFeatureDimension {
            target,
            value_text: value.to_owned(),
        });
    shell.settle();
    prepared
}

#[test]
fn offset_pad_and_face_pocket_recompute_safely_through_headless_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("pad-pocket-fixture.ketchup");
    let saved = directory.path().join("pad-pocket-saved.ketchup");
    write_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 5.0, 18.0);
    assert!(!shell.app().can_undo());

    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    assert!(prepare_dimension(&mut shell, OFFSET_PLANE, "8"));
    let cancel = shell.catalog().text("assistant-cancel");
    shell.click_row(&cancel);
    assert_eq!(shell.app().document_revision(), initial_revision);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert!(!shell.app().can_undo());

    assert!(prepare_dimension(&mut shell, OFFSET_PLANE, "8"));
    let confirm = shell.catalog().text("assistant-confirm");
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), initial_revision + 1);
    assert_eq!(shell.app().undo_step_count(), 1);
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 8.0, 18.0);
    let offset_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 5.0, 18.0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), offset_digest);
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 8.0, 18.0);

    assert!(prepare_dimension(&mut shell, POCKET, "7"));
    shell.click_row(&confirm);
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 8.0, 18.0);
    let pocket_digest = shell.app().canonical_digest();
    let pocket_revision = shell.app().document_revision();
    let pocket_undo = shell.app().undo_step_count();
    let pocket_snapshot = shell.app().document_snapshot();
    let FeatureKind::SketchPocket(spec) = pocket_snapshot.feature(POCKET).unwrap().kind() else {
        panic!("expected sketch Pocket");
    };
    assert_eq!(spec.extent.blind_distance().unwrap().millimetres(), 7.0);

    assert!(prepare_dimension(&mut shell, POCKET_BASE_PAD, "20"));
    shell.click_row(&confirm);
    assert_eq!(shell.app().document_revision(), pocket_revision + 1);
    assert_eq!(shell.app().undo_step_count(), pocket_undo + 1);
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 8.0, 20.0);
    let recomputed = shell.app().document_snapshot();
    let FeatureKind::Workplane(face_plane) = recomputed.feature(POCKET_FACE_PLANE).unwrap().kind()
    else {
        panic!("expected generated-face workplane");
    };
    assert_eq!(face_plane.frame.origin_mm, [10.0, 20.0, 20.0]);
    let recomputed_digest = shell.app().canonical_digest();
    let recomputed_revision = shell.app().document_revision();
    let recomputed_undo = shell.app().undo_step_count();
    let recomputed_redo = shell.app().redo_step_count();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), pocket_digest);
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 8.0, 18.0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), recomputed_digest);
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 8.0, 20.0);

    assert!(!prepare_dimension(&mut shell, POCKET_BASE_PAD, "0"));
    assert_eq!(shell.app().document_revision(), recomputed_revision);
    assert_eq!(shell.app().canonical_digest(), recomputed_digest);
    assert_eq!(shell.app().undo_step_count(), recomputed_undo);
    assert_eq!(shell.app().redo_step_count(), recomputed_redo);
    assert_exact_scene(&shell, 8.0, 20.0);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file(), "{}", shell.app().action_digest());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), recomputed_digest);
    assert_eq!(shell.app().document_revision(), recomputed_revision);
    assert!(!shell.app().can_undo());
    wait_for_exact_bodies(&mut shell);
    assert_exact_scene(&shell, 8.0, 20.0);
}

fn capstone_canonical_identity(shell: &Shell) -> Vec<String> {
    let contract =
        ketchup_core::release_capstone::ReleaseCapstoneContract::mechanical_plate_fixture();
    let snapshot = shell.app().document_snapshot();
    let definitions = [
        (contract.plate_definition_id, contract.plate_body_id),
        (contract.shared_definition_id, contract.shared_body_id),
        (
            contract.replacement_definition_id,
            contract.replacement_body_id,
        ),
    ];
    let mut identity = definitions
        .into_iter()
        .map(|(definition_id, body_id)| {
            let definition = snapshot.definition(definition_id).unwrap();
            let body = definition.body(body_id).unwrap();
            format!(
                "definition:{}:body:{}:{body:?}",
                definition.id().0,
                body_id.0
            )
        })
        .collect::<Vec<_>>();
    for feature_id in contract
        .plate_feature_ids
        .all()
        .into_iter()
        .chain(contract.shared_feature_ids.all())
        .chain(contract.replacement_feature_ids.all())
    {
        let feature = snapshot.feature(feature_id).unwrap();
        identity.push(format!(
            "feature:{}:definition:{}:{:?}",
            feature_id.0,
            feature.definition_id().0,
            feature.kind()
        ));
    }
    for occurrence_id in [
        contract.plate_occurrence_id,
        contract.first_shared_occurrence_id,
        contract.second_shared_occurrence_id,
    ] {
        let occurrence = snapshot.occurrence(occurrence_id).unwrap();
        identity.push(format!(
            "occurrence:{}:definition:{}",
            occurrence_id.0,
            occurrence.definition_id().0
        ));
    }
    identity.sort();
    identity
}

fn open_part_authoring(shell: &mut Shell) {
    let title = shell.catalog().text("part-authoring-title");
    shell.click_role_and_label(Role::Button, &title);
}

fn click_part_authoring(shell: &mut Shell, key: &str) {
    let label = shell.catalog().text(key);
    shell.click_role_and_label(Role::Button, &label);
}

fn open_feature_history(shell: &mut Shell) {
    let definition = shell.catalog().text("feature-history-definition");
    if !shell.has_role_and_label(Role::ComboBox, &definition) {
        let title = shell.catalog().text("feature-history-title");
        shell.click_role_and_label(Role::Button, &title);
    }
}

fn select_feature_history_definition(shell: &mut Shell, name: &str) {
    let label = shell.catalog().text("feature-history-definition");
    shell.click_role_and_label(Role::ComboBox, &label);
    shell.click_button_label(name);
}

fn feature_history_body_label(
    shell: &Shell,
    definition_id: DefinitionId,
    body_id: BodyId,
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
            (
                "status",
                shell.catalog().text("feature-history-body-active"),
            ),
        ]),
    )
}

fn feature_history_feature_label(shell: &Shell, feature_id: FeatureId) -> String {
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

fn replace_feature_history_value(shell: &mut Shell, value: &str) {
    let label = shell.catalog().text("feature-history-exact-value");
    shell.focus_text_input(&label);
    shell.key(Key::A, ctrl());
    shell.type_text(value);
}

fn click_exact_occurrence_top(shell: &mut Shell, occurrence_id: OccurrenceId, height_mm: f64) {
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    let snapshot = shell.app().document_snapshot();
    let transform = snapshot.occurrence(occurrence_id).unwrap().transform();
    let matrix = transform.matrix();
    let point = Vec3::new(
        matrix[2].mul_add(height_mm, matrix[3]),
        matrix[6].mul_add(height_mm, matrix[7]),
        matrix[10].mul_add(height_mm, matrix[11]),
    );
    let screen = shell.app().project_to_screen(point, shell.viewport_rect());
    assert!(shell.viewport_rect().contains(screen));
    shell.click_at(screen);
}

fn preview_feature_history(shell: &mut Shell, key: &str) {
    let label = shell.catalog().text(key);
    shell.click_role_and_label(Role::Button, &label);
    assert!(
        shell.app().feature_history_preview_pending(),
        "{}",
        shell.app().action_digest()
    );
}

fn confirm_feature_history(shell: &mut Shell) {
    let label = shell.catalog().text("feature-history-confirm");
    shell.click_role_and_label(Role::Button, &label);
    assert!(!shell.app().feature_history_preview_pending());
}

fn wait_for_capstone_producers(shell: &mut Shell, expected: &[FeatureId]) {
    for _ in 0..200 {
        shell.settle();
        let mut actual = shell.app().exact_current_producer_ids();
        actual.sort();
        let mut expected = expected.to_vec();
        expected.sort();
        if actual == expected && shell.app().headless_part_authoring_step_ready() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "exact producers did not converge: {:?}; {}",
        shell.app().exact_current_producer_ids(),
        shell.app().action_digest()
    );
}

#[test]
fn capstone_parts_are_authored_from_new_through_serial_accesskit() {
    let contract =
        ketchup_core::release_capstone::ReleaseCapstoneContract::mechanical_plate_fixture();
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("capstone-parts.ketchup");
    let step = directory.path().join("capstone-assembly.step");
    let stl = directory.path().join("capstone-assembly.stl");
    let failed_step = directory.path().join("failed-worker.step");
    let reopened_step = directory.path().join("reopened-capstone.step");
    let reopened_stl = directory.path().join("reopened-capstone.stl");
    let shared_step = directory.path().join("shared-edit.step");
    let shared_stl = directory.path().join("shared-edit.stl");
    let unique_step = directory.path().join("unique-edit.step");
    let unique_stl = directory.path().join("unique-edit.stl");
    let replaced_step = directory.path().join("component-replaced.step");
    let replaced_stl = directory.path().join("component-replaced.stl");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .queue_export(&step)
        .queue_export(&stl)
        .queue_export(&failed_step)
        .queue_export(&reopened_step)
        .queue_export(&reopened_stl)
        .queue_export(&shared_step)
        .queue_export(&shared_stl)
        .queue_export(&unique_step)
        .queue_export(&unique_stl)
        .queue_export(&replaced_step)
        .queue_export(&replaced_stl)
        .queue_open(&saved)
        .always_confirm_high_risk_as(10_480)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    open_part_authoring(&mut shell);

    let blank = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );
    shell
        .app_mut()
        .set_headless_part_authoring_dimensions("120,0,8");
    click_part_authoring(&mut shell, "part-authoring-preview");
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        blank
    );

    shell
        .app_mut()
        .set_headless_part_authoring_dimensions("120,80,8");
    assert!(shell.app().headless_part_authoring_proposal_parity());
    click_part_authoring(&mut shell, "part-authoring-preview");
    assert_eq!(shell.app().canonical_digest(), blank.1);
    click_part_authoring(&mut shell, "part-authoring-cancel");
    assert_eq!(shell.app().canonical_digest(), blank.1);
    assert_eq!(shell.app().undo_step_count(), blank.2);

    click_part_authoring(&mut shell, "part-authoring-preview");
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), blank.1);
    assert_eq!(shell.app().undo_step_count(), blank.2);

    click_part_authoring(&mut shell, "part-authoring-preview");
    assert!(shell.app_mut().make_headless_part_authoring_preview_stale());
    shell.settle();
    let intervening = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    click_part_authoring(&mut shell, "part-authoring-confirm");
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        intervening
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), blank.1);

    click_part_authoring(&mut shell, "part-authoring-preview");
    click_part_authoring(&mut shell, "part-authoring-confirm");
    assert_eq!(shell.app().undo_step_count(), blank.2 + 1);
    let plate_digest = shell.app().canonical_digest();
    let plate_revision = shell.app().document_revision();
    let plate = shell.app().document_snapshot();
    assert!(matches!(
        plate
            .feature(contract.plate_feature_ids.principal_workplane)
            .unwrap()
            .kind(),
        FeatureKind::Workplane(_)
    ));
    assert!(matches!(
        plate
            .feature(contract.plate_feature_ids.base_sketch)
            .unwrap()
            .kind(),
        FeatureKind::Sketch(_)
    ));
    assert!(matches!(
        plate
            .feature(contract.plate_feature_ids.pad)
            .unwrap()
            .kind(),
        FeatureKind::Pad(_)
    ));
    wait_for_capstone_producers(&mut shell, &[contract.plate_feature_ids.pad]);
    let plate_output = shell
        .app()
        .headless_face_workflow_exact_output_fingerprints();
    assert_eq!(shell.app().exact_render_body_count(), 1);

    assert!(
        shell
            .app_mut()
            .apply_assistant_intent(WorkflowIntent::SetOccurrenceVisibility {
                target: contract.plate_occurrence_id,
                visible: false,
            },)
    );
    let hidden = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
    );
    assert!(!shell.app().headless_part_authoring_step_ready());
    click_part_authoring(&mut shell, "part-authoring-preview");
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
            shell
                .app()
                .headless_face_workflow_exact_output_fingerprints(),
        ),
        hidden
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    wait_for_capstone_producers(&mut shell, &[contract.plate_feature_ids.pad]);
    assert_eq!(shell.app().canonical_digest(), plate_digest);
    assert_eq!(
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        plate_output
    );

    shell
        .app_mut()
        .set_headless_part_authoring_dimensions("20,9");
    click_part_authoring(&mut shell, "part-authoring-preview");
    assert_eq!(shell.app().document_revision(), plate_revision);
    assert_eq!(shell.app().canonical_digest(), plate_digest);
    assert_eq!(
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        plate_output
    );
    assert!(
        shell
            .app_mut()
            .connect_exact_worker(directory.path().join("missing-worker.exe"))
            .is_err()
    );
    assert_eq!(
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        plate_output
    );

    shell
        .app_mut()
        .set_headless_part_authoring_dimensions("20,4");
    assert!(shell.app().headless_part_authoring_proposal_parity());
    click_part_authoring(&mut shell, "part-authoring-preview");
    assert_eq!(shell.app().canonical_digest(), plate_digest);
    click_part_authoring(&mut shell, "part-authoring-confirm");
    assert_eq!(shell.app().undo_step_count(), blank.2 + 2);
    wait_for_capstone_producers(&mut shell, &[contract.plate_feature_ids.pad]);
    let support = shell.app().document_snapshot();
    assert!(matches!(
        support
            .feature(contract.plate_feature_ids.face_workplane)
            .unwrap()
            .kind(),
        FeatureKind::Workplane(_)
    ));
    assert!(matches!(
        support
            .feature(contract.plate_feature_ids.pocket_sketch)
            .unwrap()
            .kind(),
        FeatureKind::Sketch(_)
    ));

    let support_stamp = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
    );
    for health in [
        WorkplaneSupportHealth::Ambiguous,
        WorkplaneSupportHealth::Lost,
    ] {
        assert!(
            shell
                .app_mut()
                .set_headless_part_authoring_reference_health(health)
        );
        let unresolved = shell.app().document_snapshot();
        let FeatureKind::Workplane(workplane) = unresolved
            .feature(contract.plate_feature_ids.face_workplane)
            .unwrap()
            .kind()
        else {
            panic!("expected face workplane")
        };
        assert!(matches!(
            workplane.support,
            WorkplaneSupport::PlanarFace { health: actual, .. } if actual == health
        ));
        assert!(!shell.app().headless_part_authoring_step_ready());
        click_part_authoring(&mut shell, "part-authoring-preview");
        assert_eq!(
            (
                shell.app().document_revision(),
                shell.app().canonical_digest(),
                shell.app().undo_step_count(),
                shell.app().redo_step_count(),
                shell
                    .app()
                    .headless_face_workflow_exact_output_fingerprints(),
            ),
            support_stamp
        );
        assert!(
            shell
                .app_mut()
                .set_headless_part_authoring_reference_health(WorkplaneSupportHealth::Resolved)
        );
    }
    assert!(
        shell
            .app()
            .headless_part_authoring_unsupported_profile_refused()
    );
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
            shell
                .app()
                .headless_face_workflow_exact_output_fingerprints(),
        ),
        support_stamp
    );
    assert!(shell.app().headless_part_authoring_proposal_parity());

    click_part_authoring(&mut shell, "part-authoring-preview");
    click_part_authoring(&mut shell, "part-authoring-confirm");
    assert_eq!(shell.app().undo_step_count(), blank.2 + 3);
    wait_for_capstone_producers(&mut shell, &[contract.plate_feature_ids.pocket]);
    let pocket = shell.app().document_snapshot();
    assert!(matches!(
        pocket
            .feature(contract.plate_feature_ids.pocket)
            .unwrap()
            .kind(),
        FeatureKind::SketchPocket(_)
    ));

    assert!(shell.app().headless_part_authoring_proposal_parity());
    click_part_authoring(&mut shell, "part-authoring-preview");
    let pocket_digest = shell.app().canonical_digest();
    click_part_authoring(&mut shell, "part-authoring-cancel");
    assert_eq!(shell.app().canonical_digest(), pocket_digest);
    click_part_authoring(&mut shell, "part-authoring-preview");
    click_part_authoring(&mut shell, "part-authoring-confirm");
    assert_eq!(shell.app().undo_step_count(), blank.2 + 4);
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
        ],
    );
    assert_eq!(shell.app().exact_render_body_count(), 2);
    assert!(
        shell.app().instanced_scene_triangle_count() > shell.app().exact_render_triangle_count()
    );
    let fastener_pick = shell
        .app()
        .exact_pick_durable(
            Ray::new(Vec3::new(-30.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
        )
        .expect("reused Fastener A is exact-pickable");
    assert_eq!(
        fastener_pick.instance_path,
        InstancePath::root(contract.first_shared_occurrence_id)
    );
    let authored = shell.app().document_snapshot();
    assert_eq!(
        authored
            .occurrence(contract.first_shared_occurrence_id)
            .unwrap()
            .definition_id(),
        contract.shared_definition_id
    );
    assert_eq!(
        authored
            .occurrence(contract.second_shared_occurrence_id)
            .unwrap()
            .definition_id(),
        contract.shared_definition_id
    );
    assert!(matches!(
        authored
            .feature(contract.replacement_feature_ids.pad)
            .unwrap()
            .kind(),
        FeatureKind::Pad(_)
    ));

    let authored_digest = shell.app().canonical_digest();
    let authored_identity = capstone_canonical_identity(&shell);
    let authored_subshapes = shell.app().headless_part_authoring_subshape_lineage();
    let authored_outputs = shell
        .app()
        .headless_face_workflow_exact_output_fingerprints();
    assert!(!authored_subshapes.is_empty());
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(
        shell
            .app()
            .document_snapshot()
            .definition(contract.shared_definition_id)
            .is_none()
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), authored_digest);
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
        ],
    );
    assert_eq!(capstone_canonical_identity(&shell), authored_identity);
    assert_eq!(
        shell.app().headless_part_authoring_subshape_lineage(),
        authored_subshapes
    );
    assert_eq!(
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        authored_outputs
    );

    let persisted_digest = shell.app().canonical_digest();
    let persisted_revision = shell.app().document_revision();
    let persisted_bounds = sorted_bounds(&shell);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().document_revision(), persisted_revision);
    assert!(!shell.app().can_undo());
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
        ],
    );
    assert_eq!(sorted_bounds(&shell), persisted_bounds);
    assert_eq!(capstone_canonical_identity(&shell), authored_identity);
    assert_eq!(
        shell.app().headless_part_authoring_subshape_lineage(),
        authored_subshapes
    );
    assert_eq!(
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        authored_outputs
    );

    let parts_digest = shell.app().canonical_digest();
    let assembly_title = shell.catalog().text("assembly-title");
    shell.click_role_and_label(Role::Button, &assembly_title);
    let compose = shell.catalog().text("assembly-preview-capstone");
    shell.click_button_label(&compose);
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(shell.app().canonical_digest(), parts_digest);
    let confirm = shell.catalog().text("assembly-confirm-preview");
    shell.click_button_label(&confirm);
    assert_eq!(shell.app().assembly_mate_count(), 2);
    assert_eq!(shell.app().grounded_occurrence_count(), 2);
    assert_eq!(
        shell.app().headless_capstone_assembly_summary(),
        Some((AssemblySolveStatus::FullyConstrained, 0, 0, 0))
    );
    let composed_digest = shell.app().canonical_digest();
    let composed_transforms = [
        contract.plate_occurrence_id,
        contract.first_shared_occurrence_id,
        contract.second_shared_occurrence_id,
    ]
    .map(|id| {
        shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()
    });

    shell.settle();
    let solve = shell.catalog().text("assembly-preview-solve");
    shell.click_button_label(&solve);
    assert_eq!(
        shell.app().assembly_solve_status(),
        Some(AssemblySolveStatus::FullyConstrained)
    );
    assert!(shell.app().assembly_preview_pending());
    shell.click_button_label(&confirm);
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
        ],
    );
    assert_eq!(
        shell.app().headless_capstone_assembly_summary(),
        Some((AssemblySolveStatus::FullyConstrained, 0, 0, 0))
    );
    let solved_digest = shell.app().canonical_digest();
    let solved_transforms = [
        contract.plate_occurrence_id,
        contract.first_shared_occurrence_id,
        contract.second_shared_occurrence_id,
    ]
    .map(|id| {
        shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()
    });
    assert_ne!(solved_transforms, composed_transforms);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), composed_digest);
    assert_eq!(
        [
            contract.plate_occurrence_id,
            contract.first_shared_occurrence_id,
            contract.second_shared_occurrence_id,
        ]
        .map(|id| shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()),
        composed_transforms
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), solved_digest);
    assert_eq!(
        [
            contract.plate_occurrence_id,
            contract.first_shared_occurrence_id,
            contract.second_shared_occurrence_id,
        ]
        .map(|id| shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()),
        solved_transforms
    );
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
        ],
    );
    assert_eq!(shell.app().exact_render_body_count(), 2);
    assert!(
        shell.app().instanced_scene_triangle_count() > shell.app().exact_render_triangle_count()
    );
    let solved_pick = shell
        .app()
        .exact_pick_durable(
            Ray::new(Vec3::new(0.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
        )
        .expect("the solved fastener is exact-pickable");
    assert_eq!(
        solved_pick.instance_path,
        InstancePath::root(contract.first_shared_occurrence_id)
    );

    let drawing = shell.catalog().text("assembly-preview-capstone-drawing");
    shell.click_button_label(&drawing);
    assert!(shell.app().assembly_preview_pending());
    shell.click_button_label(&confirm);
    shell.settle();
    let drawing_fingerprint = shell
        .app()
        .headless_capstone_drawing_fingerprint()
        .expect("the capstone drawing is current");
    assert_eq!(drawing_fingerprint.1, vec!["front", "top", "right"]);
    let drawing_digest = shell.app().canonical_digest();
    let drawing_exact_outputs = shell
        .app()
        .headless_face_workflow_exact_output_fingerprints();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(
        shell
            .app()
            .headless_capstone_drawing_fingerprint()
            .is_none()
    );
    assert_eq!(
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        drawing_exact_outputs
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), drawing_digest);
    assert_eq!(
        shell.app().headless_capstone_drawing_fingerprint(),
        Some(drawing_fingerprint.clone())
    );

    let output_state = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    );
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        output_state
    );
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
        ),
        output_state
    );
    assert!(
        std::fs::read_to_string(&step)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    assert!(std::fs::read_to_string(&stl).unwrap().starts_with("solid "));
    let step_bytes = std::fs::read(&step).unwrap();
    let stl_bytes = std::fs::read(&stl).unwrap();
    let last_valid_stamp = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
        [
            contract.plate_occurrence_id,
            contract.first_shared_occurrence_id,
            contract.second_shared_occurrence_id,
        ]
        .map(|id| {
            shell
                .app()
                .document_snapshot()
                .occurrence(id)
                .unwrap()
                .transform()
        }),
        shell.app().headless_capstone_drawing_fingerprint(),
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        shell.app().exact_render_body_count(),
        shell.app().instanced_scene_triangle_count(),
        format!(
            "{:?}",
            shell.app().exact_pick_durable(
                Ray::new(Vec3::new(0.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
            )
        ),
    );
    assert_eq!(
        shell.app().headless_capstone_assembly_refusal_paths(),
        vec![
            "under-constrained",
            "redundant",
            "conflicting-over-constrained",
            "stale-confirmation",
            "ambiguous",
            "lost",
            "unsupported",
        ]
    );
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
            [
                contract.plate_occurrence_id,
                contract.first_shared_occurrence_id,
                contract.second_shared_occurrence_id,
            ]
            .map(|id| shell
                .app()
                .document_snapshot()
                .occurrence(id)
                .unwrap()
                .transform()),
            shell.app().headless_capstone_drawing_fingerprint(),
            shell
                .app()
                .headless_face_workflow_exact_output_fingerprints(),
            shell.app().exact_render_body_count(),
            shell.app().instanced_scene_triangle_count(),
            format!(
                "{:?}",
                shell.app().exact_pick_durable(
                    Ray::new(Vec3::new(0.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
                )
            ),
        ),
        last_valid_stamp
    );
    shell
        .app_mut()
        .headless_force_exact_worker_path(directory.path().join("missing-worker.exe"));
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert!(!failed_step.exists());
    assert_eq!(std::fs::read(&step).unwrap(), step_bytes);
    assert_eq!(std::fs::read(&stl).unwrap(), stl_bytes);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
            shell.app().redo_step_count(),
            [
                contract.plate_occurrence_id,
                contract.first_shared_occurrence_id,
                contract.second_shared_occurrence_id,
            ]
            .map(|id| shell
                .app()
                .document_snapshot()
                .occurrence(id)
                .unwrap()
                .transform()),
            shell.app().headless_capstone_drawing_fingerprint(),
            shell
                .app()
                .headless_face_workflow_exact_output_fingerprints(),
            shell.app().exact_render_body_count(),
            shell.app().instanced_scene_triangle_count(),
            format!(
                "{:?}",
                shell.app().exact_pick_durable(
                    Ray::new(Vec3::new(0.0, 0.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).unwrap(),
                )
            ),
        ),
        last_valid_stamp
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
        ],
    );

    shell.click_menu_command("menu-file", AppCommand::Save);
    let model_bytes = std::fs::read(&saved).unwrap();
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), output_state.1);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
        ],
    );
    assert_eq!(
        shell.app().headless_capstone_assembly_summary(),
        Some((AssemblySolveStatus::FullyConstrained, 0, 0, 0))
    );
    assert_eq!(
        shell.app().headless_capstone_drawing_fingerprint(),
        Some(drawing_fingerprint)
    );
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert_eq!(std::fs::read(&saved).unwrap(), model_bytes);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(std::fs::read(&reopened_step).unwrap(), step_bytes);
    assert_eq!(std::fs::read(&reopened_stl).unwrap(), stl_bytes);
    assert_eq!(std::fs::read(&step).unwrap(), step_bytes);
    assert_eq!(std::fs::read(&stl).unwrap(), stl_bytes);

    let original_transforms = [
        contract.plate_occurrence_id,
        contract.first_shared_occurrence_id,
        contract.second_shared_occurrence_id,
    ]
    .map(|id| {
        shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()
    });
    let target_definition = shell
        .app()
        .document_snapshot()
        .definition(contract.replacement_definition_id)
        .unwrap()
        .clone();

    click_exact_occurrence_top(&mut shell, contract.first_shared_occurrence_id, 30.0);
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    open_feature_history(&mut shell);
    let shared_name = shell.catalog().text("part-authoring-shared-fastener");
    select_feature_history_definition(&mut shell, &shared_name);
    let shared_body = feature_history_body_label(
        &shell,
        contract.shared_definition_id,
        contract.shared_body_id,
    );
    shell.click_role_and_label(Role::Button, &shared_body);
    let shared_pad = feature_history_feature_label(&shell, contract.shared_feature_ids.pad);
    shell.click_role_and_label(Role::Button, &shared_pad);
    let shared_scope = shell.catalog().text("feature-history-change-scope-shared");
    shell.click_role_and_label(Role::RadioButton, &shared_scope);
    replace_feature_history_value(&mut shell, "32");
    let before_shared = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().headless_capstone_drawing_fingerprint(),
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
    );
    preview_feature_history(&mut shell, "feature-history-preview-edit");
    assert!(shell.app().feature_history_shared_impact_counts().is_some());
    assert_eq!(shell.app().canonical_digest(), before_shared.1);
    confirm_feature_history(&mut shell);
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
        ],
    );
    assert_eq!(shell.app().document_revision(), before_shared.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before_shared.2 + 1);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .definition(contract.replacement_definition_id),
        Some(&target_definition)
    );
    assert_eq!(
        [
            contract.plate_occurrence_id,
            contract.first_shared_occurrence_id,
            contract.second_shared_occurrence_id,
        ]
        .map(|id| shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()),
        original_transforms
    );
    assert_ne!(
        shell.app().headless_capstone_drawing_fingerprint(),
        before_shared.3
    );
    assert_ne!(
        shell
            .app()
            .headless_face_workflow_exact_output_fingerprints(),
        before_shared.4
    );
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    let shared_step_bytes = std::fs::read(&shared_step).unwrap();
    let shared_stl_bytes = std::fs::read(&shared_stl).unwrap();
    assert_ne!(shared_step_bytes, step_bytes);
    assert_ne!(shared_stl_bytes, stl_bytes);

    click_exact_occurrence_top(&mut shell, contract.second_shared_occurrence_id, 32.0);
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    open_feature_history(&mut shell);
    let shared_name = shell.catalog().text("part-authoring-shared-fastener");
    select_feature_history_definition(&mut shell, &shared_name);
    let shared_body = feature_history_body_label(
        &shell,
        contract.shared_definition_id,
        contract.shared_body_id,
    );
    shell.click_role_and_label(Role::Button, &shared_body);
    let shared_pad = feature_history_feature_label(&shell, contract.shared_feature_ids.pad);
    shell.click_role_and_label(Role::Button, &shared_pad);
    let make_unique = shell
        .catalog()
        .text("feature-history-change-scope-make-unique");
    shell.click_role_and_label(Role::RadioButton, &make_unique);
    replace_feature_history_value(&mut shell, "34");
    let before_unique = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().headless_capstone_drawing_fingerprint(),
    );
    preview_feature_history(&mut shell, "feature-history-preview-edit");
    assert_eq!(
        shell.app().feature_history_fork_identity(),
        Some([1002, 200, 301])
    );
    assert!(shell.app().feature_history_fork_impact_counts().is_some());
    assert_eq!(shell.app().canonical_digest(), before_unique.1);
    confirm_feature_history(&mut shell);
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.shared_feature_ids.pad,
            FeatureId(315),
        ],
    );
    assert_eq!(shell.app().document_revision(), before_unique.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before_unique.2 + 1);
    assert_eq!(
        shell
            .app()
            .occurrence_definition_id(contract.first_shared_occurrence_id),
        Some(contract.shared_definition_id)
    );
    assert_eq!(
        shell
            .app()
            .occurrence_definition_id(contract.second_shared_occurrence_id),
        Some(DefinitionId(301))
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .definition(contract.replacement_definition_id),
        Some(&target_definition)
    );
    assert_eq!(
        [
            contract.plate_occurrence_id,
            contract.first_shared_occurrence_id,
            contract.second_shared_occurrence_id,
        ]
        .map(|id| shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()),
        original_transforms
    );
    assert_ne!(
        shell.app().headless_capstone_drawing_fingerprint(),
        before_unique.3
    );
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    let unique_step_bytes = std::fs::read(&unique_step).unwrap();
    let unique_stl_bytes = std::fs::read(&unique_stl).unwrap();
    assert_ne!(unique_step_bytes, shared_step_bytes);
    assert_ne!(unique_stl_bytes, shared_stl_bytes);
    assert_eq!(std::fs::read(&shared_step).unwrap(), shared_step_bytes);
    assert_eq!(std::fs::read(&shared_stl).unwrap(), shared_stl_bytes);

    click_exact_occurrence_top(&mut shell, contract.first_shared_occurrence_id, 32.0);
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    open_feature_history(&mut shell);
    let shared_name = shell.catalog().text("part-authoring-shared-fastener");
    select_feature_history_definition(&mut shell, &shared_name);
    let shared_body = feature_history_body_label(
        &shell,
        contract.shared_definition_id,
        contract.shared_body_id,
    );
    shell.click_role_and_label(Role::Button, &shared_body);
    assert_eq!(
        shell.app().feature_history_selected_body_id(),
        Some(contract.shared_body_id)
    );
    let replace = shell
        .catalog()
        .text("feature-history-change-scope-replace-component");
    shell.click_role_and_label(Role::RadioButton, &replace);
    let target = shell.catalog().text("feature-history-replacement-target");
    shell.click_role_and_label(Role::ComboBox, &target);
    let target_name = shell.catalog().text("part-authoring-target-fastener");
    shell.click_button_label(&target_name);
    assert_eq!(
        shell.app().feature_history_selection_ids(),
        (
            Some(contract.shared_definition_id),
            Some(contract.shared_body_id),
            Some(contract.replacement_definition_id),
        )
    );
    let before_replacement = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().headless_capstone_drawing_fingerprint(),
    );
    preview_feature_history(&mut shell, "feature-history-preview-replace-component");
    assert_eq!(
        shell.app().feature_history_replacement_identity(),
        Some([1001, 200, 300])
    );
    assert!(
        shell
            .app()
            .feature_history_replacement_impact_counts()
            .is_some()
    );
    assert_eq!(shell.app().canonical_digest(), before_replacement.1);
    let unique_definition = shell
        .app()
        .document_snapshot()
        .definition(DefinitionId(301))
        .unwrap()
        .clone();
    let unique_occurrence = shell
        .app()
        .document_snapshot()
        .occurrence(contract.second_shared_occurrence_id)
        .unwrap()
        .clone();
    confirm_feature_history(&mut shell);
    wait_for_capstone_producers(
        &mut shell,
        &[
            contract.plate_feature_ids.pocket,
            contract.replacement_feature_ids.pad,
            FeatureId(315),
        ],
    );
    assert_eq!(shell.app().document_revision(), before_replacement.0 + 1);
    assert_eq!(shell.app().undo_step_count(), before_replacement.2 + 1);
    assert_eq!(
        shell
            .app()
            .occurrence_definition_id(contract.first_shared_occurrence_id),
        Some(contract.replacement_definition_id)
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .definition(DefinitionId(301)),
        Some(&unique_definition)
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(contract.second_shared_occurrence_id),
        Some(&unique_occurrence)
    );
    assert_eq!(
        [
            contract.plate_occurrence_id,
            contract.first_shared_occurrence_id,
            contract.second_shared_occurrence_id,
        ]
        .map(|id| shell
            .app()
            .document_snapshot()
            .occurrence(id)
            .unwrap()
            .transform()),
        original_transforms
    );
    assert_ne!(
        shell.app().headless_capstone_drawing_fingerprint(),
        before_replacement.3
    );
    for mate in shell.app().document_snapshot().assembly_mates() {
        for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
            if endpoint.occurrence_id() == contract.first_shared_occurrence_id {
                assert_eq!(
                    endpoint.reference().definition_id,
                    contract.replacement_definition_id
                );
            }
        }
    }
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    let replaced_step_bytes = std::fs::read(&replaced_step).unwrap();
    let replaced_stl_bytes = std::fs::read(&replaced_stl).unwrap();
    assert_ne!(replaced_step_bytes, unique_step_bytes);
    assert_ne!(replaced_stl_bytes, unique_stl_bytes);
    assert_eq!(std::fs::read(&step).unwrap(), step_bytes);
    assert_eq!(std::fs::read(&stl).unwrap(), stl_bytes);
    assert_eq!(std::fs::read(&unique_step).unwrap(), unique_step_bytes);
    assert_eq!(std::fs::read(&unique_stl).unwrap(), unique_stl_bytes);
}
