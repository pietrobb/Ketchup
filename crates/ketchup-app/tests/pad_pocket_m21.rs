//! Program 2 Pad/Pocket workflows replayed offscreen through AccessKit.

mod harness;

use std::path::{Path, PathBuf};
use std::time::Duration;

use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    OccurrenceId, Transform,
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
    assert_eq!(spec.extent.distance().millimetres(), 7.0);

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
