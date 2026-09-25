//! Program 3 rigid-assembly authoring replayed offscreen through AccessKit.

mod harness;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{Key, Modifiers, accesskit::Role};
use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::assembly::{AssemblyMateKind, AssemblySolveStatus};
use ketchup_core::assembly_joint::{
    AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind, AssemblyJointLimits,
    AssemblyMotionDriver, AssemblyMotionStudy, AssemblyMotionStudyId,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    OccurrenceId, TagId, Transform,
};
use ketchup_core::drawing::{DrawingSheetId, DrawingSource};
use ketchup_core::intent::WorkflowIntent;
use ketchup_core::mechanical_coupling::{
    AssemblyMotionCoupling, AssemblyMotionCouplingId, AssemblyMotionDirection,
    AssemblyTransmissionKind, GearMeshKind, ScrewHandedness,
};
use ketchup_core::persistence;
use ketchup_interaction::LocaleCatalog;

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
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

#[derive(Clone, Copy)]
enum KinematicFailureFixture {
    CouplingConflict,
    CoupledLimit,
}

fn write_kinematic_failure_fixture(path: &Path, fixture: KinematicFailureFixture) {
    let definition = DefinitionId(1);
    let selected_tag = TagId(50);
    let root = OccurrenceId(10);
    let input_child = OccurrenceId(11);
    let output_child = OccurrenceId(12);
    let input_joint = AssemblyJointId(101);
    let output_joint = AssemblyJointId(102);
    let output_limits = match fixture {
        KinematicFailureFixture::CouplingConflict => AssemblyJointLimits::new(-100.0, 100.0),
        KinematicFailureFixture::CoupledLimit => AssemblyJointLimits::new(-5.0, 5.0),
    };
    let drivers = match fixture {
        KinematicFailureFixture::CouplingConflict => vec![
            AssemblyMotionDriver::new(input_joint, 180.0),
            AssemblyMotionDriver::new(output_joint, 0.0),
        ],
        KinematicFailureFixture::CoupledLimit => {
            vec![AssemblyMotionDriver::new(input_joint, 180.0)]
        }
    };
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Kinematic fixture".into(),
            },
            CanonicalCommand::CreateTag {
                id: selected_tag,
                name: "Driven pair".into(),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: root,
                definition_id: definition,
                name: "Root".into(),
                transform: Transform::from_translation(0.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: Some(selected_tag),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: input_child,
                definition_id: definition,
                name: "Input".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: Some(selected_tag),
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: output_child,
                definition_id: definition,
                name: "Output".into(),
                transform: Transform::from_translation(40.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                input_joint,
                root,
                input_child,
                AssemblyJointKind::Revolute {
                    axis: AssemblyJointAxis::new([0.0, 0.0, 1.0], [0.0, 0.0, 0.0]),
                    limits: Some(AssemblyJointLimits::new(-360.0, 360.0)),
                    position_degrees: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                output_joint,
                root,
                output_child,
                AssemblyJointKind::Prismatic {
                    axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                    limits: Some(output_limits),
                    position_mm: 0.0,
                },
            )),
            CanonicalCommand::CreateAssemblyMotionCoupling(AssemblyMotionCoupling::new(
                AssemblyMotionCouplingId(201),
                input_joint,
                output_joint,
                0.0,
                0.0,
                AssemblyTransmissionKind::RackAndPinion {
                    pinion_pitch_diameter_mm: 20.0,
                    direction: AssemblyMotionDirection::Same,
                },
            )),
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                AssemblyMotionStudyId(300),
                "Failure preview",
                drivers,
            )),
        ]))
        .unwrap();
    std::fs::write(path, persistence::save(&document.current())).unwrap();
}

fn write_coupling_authoring_fixture(path: &Path) {
    let definition = DefinitionId(1);
    let root = OccurrenceId(10);
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: definition,
            name: "Coupling authoring fixture".into(),
        },
        CanonicalCommand::CreateOccurrence {
            id: root,
            definition_id: definition,
            name: "Root".into(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        },
    ];
    for offset in 1..=10_u64 {
        let occurrence_id = OccurrenceId(10 + offset);
        let joint_id = AssemblyJointId(100 + offset);
        commands.push(CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id: definition,
            name: format!("Driven {offset}"),
            transform: Transform::from_translation(offset as f64 * 20.0, 0.0, 0.0).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        });
        let kind = if matches!(joint_id.0, 108 | 110) {
            AssemblyJointKind::Prismatic {
                axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                limits: Some(AssemblyJointLimits::new(-100.0, 100.0)),
                position_mm: 0.0,
            }
        } else if joint_id.0 == 109 {
            AssemblyJointKind::Helical {
                axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                limits: Some(AssemblyJointLimits::new(-360.0, 360.0)),
                lead_mm_per_revolution: 8.0,
                position_degrees: 0.0,
            }
        } else {
            AssemblyJointKind::Revolute {
                axis: AssemblyJointAxis::new([0.0, 0.0, 1.0], [0.0, 0.0, 0.0]),
                limits: Some(AssemblyJointLimits::new(-360.0, 360.0)),
                position_degrees: 0.0,
            }
        };
        commands.push(CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            joint_id,
            root,
            occurrence_id,
            kind,
        )));
    }
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    std::fs::write(path, persistence::save(&document.current())).unwrap();
}

fn write_drag_fixture(path: &Path, mover_transform: Transform, joint_kind: AssemblyJointKind) {
    let definition = DefinitionId(1);
    let profile = FeatureId(1);
    let extrusion = FeatureId(2);
    let obstacle = OccurrenceId(10);
    let mover = OccurrenceId(11);
    let joint = AssemblyJointId(101);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Collision drag fixture".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Unit profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: extrusion,
                definition_id: definition,
                name: "Unit body".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::from_decimal("1").unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: obstacle,
                definition_id: definition,
                name: "Obstacle".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: mover,
                definition_id: definition,
                name: "Mover".into(),
                transform: mover_transform,
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                joint, obstacle, mover, joint_kind,
            )),
        ]))
        .unwrap();
    std::fs::write(path, persistence::save(&document.current())).unwrap();
}

fn write_collision_drag_fixture(path: &Path) {
    write_drag_fixture(
        path,
        Transform::from_translation(-10.0, 0.0, 0.0).unwrap(),
        AssemblyJointKind::Prismatic {
            axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
            limits: Some(AssemblyJointLimits::new(0.0, 20.0)),
            position_mm: 0.0,
        },
    );
}

fn write_rotational_drag_fixture(path: &Path) {
    write_drag_fixture(
        path,
        Transform::from_translation(-10.0, 0.0, 0.0).unwrap(),
        AssemblyJointKind::Revolute {
            axis: AssemblyJointAxis::new([0.0, 0.0, 1.0], [0.0, 0.0, 0.0]),
            limits: Some(AssemblyJointLimits::new(0.0, 360.0)),
            position_degrees: 0.0,
        },
    );
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
    assert!(shell.app().can_undo());
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
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-kinematic-summary",
        &BTreeMap::from([
            ("status", shell.catalog().text("assembly-kinematic-under"),),
            ("dof", "1".to_owned()),
        ]),
    )));
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-kinematic-joint-diagnostic",
        &BTreeMap::from([
            ("id", "1".to_owned()),
            ("dof", "1".to_owned()),
            ("drivers", "0".to_owned()),
        ]),
    )));
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
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-kinematic-summary",
        &BTreeMap::from([
            ("status", shell.catalog().text("assembly-kinematic-fully"),),
            ("dof", "0".to_owned()),
        ]),
    )));
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-kinematic-joint-diagnostic",
        &BTreeMap::from([
            ("id", "1".to_owned()),
            ("dof", "0".to_owned()),
            ("drivers", "1".to_owned()),
        ]),
    )));
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
    assert!(shell.app().can_undo());
}

#[test]
fn helical_joint_is_created_edited_driven_and_reopened_through_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("helical-assembly.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    open_assembly_editor(&mut shell);
    insert_occurrence(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    assert_eq!(shell.app().selected_occurrence_count(), 2);

    let kind_label = shell.catalog().text("assembly-joint-kind");
    shell.click_role_and_label(Role::ComboBox, &kind_label);
    shell.click_button_label(&shell.catalog().text("assembly-joint-kind-helical"));
    let lead_label = shell.catalog().text("assembly-joint-lead");
    assert!(shell.has_role_and_label(Role::TextInput, &lead_label));
    shell
        .app_mut()
        .headless_set_assembly_joint_parameters(0.0, 0.0);
    shell.settle();

    let preview = shell.catalog().text("assembly-preview-joint");
    let before_invalid = (
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
        before_invalid
    );
    assert!(
        shell
            .app()
            .action_digest()
            .contains(&shell.catalog().text("assembly-error-joint-lead"))
    );

    shell
        .app_mut()
        .headless_set_assembly_joint_parameters(0.0, 8.0);
    shell.settle();
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(shell.app().assembly_joint_count(), 0);
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-kinematic-summary",
        &BTreeMap::from([
            ("status", shell.catalog().text("assembly-kinematic-under")),
            ("dof", "1".to_owned()),
        ]),
    )));
    confirm_preview(&mut shell);
    let joint_id = shell
        .app()
        .document_snapshot()
        .assembly_joints()
        .next()
        .unwrap()
        .id();
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .assembly_joint(joint_id)
            .unwrap()
            .kind(),
        AssemblyJointKind::Helical {
            lead_mm_per_revolution: 8.0,
            position_degrees: 0.0,
            ..
        }
    ));

    assert!(shell.has_role_and_label(
        Role::TextInput,
        &shell.catalog().text("assembly-joint-position")
    ));
    shell
        .app_mut()
        .headless_set_assembly_joint_parameters(180.0, 12.0);
    shell.settle();
    let before_edit = (
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(shell.app().canonical_digest(), before_edit.0);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().undo_step_count(), before_edit.1 + 1);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .assembly_joint(joint_id)
            .unwrap()
            .kind(),
        AssemblyJointKind::Helical {
            lead_mm_per_revolution: 12.0,
            position_degrees: 180.0,
            ..
        }
    ));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .assembly_joint(joint_id)
            .unwrap()
            .kind(),
        AssemblyJointKind::Helical {
            lead_mm_per_revolution: 8.0,
            position_degrees: 0.0,
            ..
        }
    ));
    shell.click_menu_command("menu-edit", AppCommand::Redo);

    shell.app_mut().headless_set_assembly_motion_position(90.0);
    let motion_preview = shell.catalog().text("assembly-preview-motion-study");
    shell.click_button_label(&motion_preview);
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-kinematic-summary",
        &BTreeMap::from([
            ("status", shell.catalog().text("assembly-kinematic-fully")),
            ("dof", "0".to_owned()),
        ]),
    )));
    confirm_preview(&mut shell);
    assert_eq!(shell.app().assembly_motion_study_count(), 1);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    let persisted = persistence::load_file(&saved).unwrap();
    assert_eq!(persisted.snapshot().canonical_digest(), persisted_digest);
    let core_bytes = persistence::save(&persisted.snapshot());
    assert_eq!(
        persistence::save(&persistence::load(&core_bytes).unwrap().snapshot()),
        core_bytes
    );
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().assembly_joint_count(), 1);
    assert_eq!(shell.app().assembly_motion_study_count(), 1);
    assert!(shell.app().can_undo());
}

#[test]
fn all_motion_couplings_are_authored_edited_and_reopened_through_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("coupling-source.ketchup");
    let saved = directory.path().join("coupling-saved.ketchup");
    write_coupling_authoring_fixture(&source);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    open_assembly_editor(&mut shell);

    let preview = shell.catalog().text("assembly-preview-coupling");
    let cancel = shell.catalog().text("assembly-cancel-preview");
    shell.app_mut().headless_set_assembly_coupling_parameters(
        [101, 102],
        [0.0, 0.0],
        ["0", "40"],
        false,
    );
    shell.settle();
    let before_invalid = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&preview);
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(shell.app().assembly_motion_coupling_count(), 0);
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before_invalid
    );
    assert!(
        shell
            .app()
            .action_digest()
            .contains(&shell.catalog().text("assembly-error-coupling-parameter"))
    );

    for (index, kind_key, input, output, first, second, option) in [
        (
            0,
            "assembly-coupling-kind-gear",
            101,
            102,
            "20",
            "40",
            false,
        ),
        (1, "assembly-coupling-kind-belt", 103, 104, "40", "20", true),
        (
            2,
            "assembly-coupling-kind-chain",
            105,
            106,
            "15",
            "30",
            false,
        ),
        (3, "assembly-coupling-kind-rack", 107, 108, "20", "", true),
        (4, "assembly-coupling-kind-screw", 109, 110, "8", "", false),
    ] {
        if index != 0 {
            shell.click_role_and_label(
                Role::ComboBox,
                &shell.catalog().text("assembly-coupling-kind"),
            );
            shell.click_button_label(&shell.catalog().text(kind_key));
        }
        shell.app_mut().headless_set_assembly_coupling_parameters(
            [input, output],
            [0.0, 0.0],
            [first, second],
            option,
        );
        shell.settle();
        let before = (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        );
        shell.click_button_label(&preview);
        assert!(shell.app().assembly_preview_pending());
        assert_eq!(shell.app().assembly_motion_coupling_count(), index);
        assert_eq!(shell.app().canonical_digest(), before.1);
        assert!(shell.has_visible_label(&shell.catalog().format(
            "assembly-kinematic-summary",
            &BTreeMap::from([
                ("status", shell.catalog().text("assembly-kinematic-under")),
                ("dof", (10 - index - 1).to_string()),
            ]),
        )));
        if index == 0 {
            shell.click_button_label(&cancel);
            assert_eq!(shell.app().assembly_motion_coupling_count(), 0);
            assert_eq!(shell.app().canonical_digest(), before.1);
            shell.click_button_label(&preview);
        }
        confirm_preview(&mut shell);
        assert_eq!(shell.app().assembly_motion_coupling_count(), index + 1);
        assert_eq!(shell.app().document_revision(), before.0 + 1);
        assert_eq!(shell.app().undo_step_count(), before.2 + 1);
    }

    let snapshot = shell.app().document_snapshot();
    assert!(matches!(
        snapshot
            .assembly_motion_coupling(AssemblyMotionCouplingId(1))
            .unwrap()
            .transmission(),
        AssemblyTransmissionKind::GearPair {
            input_teeth: 20,
            output_teeth: 40,
            mesh: GearMeshKind::External,
        }
    ));
    assert!(matches!(
        snapshot
            .assembly_motion_coupling(AssemblyMotionCouplingId(2))
            .unwrap()
            .transmission(),
        AssemblyTransmissionKind::Belt {
            input_pitch_diameter_mm: 40.0,
            output_pitch_diameter_mm: 20.0,
            crossed: true,
        }
    ));
    assert!(matches!(
        snapshot
            .assembly_motion_coupling(AssemblyMotionCouplingId(3))
            .unwrap()
            .transmission(),
        AssemblyTransmissionKind::Chain {
            input_sprocket_teeth: 15,
            output_sprocket_teeth: 30,
        }
    ));
    assert!(matches!(
        snapshot
            .assembly_motion_coupling(AssemblyMotionCouplingId(4))
            .unwrap()
            .transmission(),
        AssemblyTransmissionKind::RackAndPinion {
            pinion_pitch_diameter_mm: 20.0,
            direction: AssemblyMotionDirection::Opposite,
        }
    ));
    assert!(matches!(
        snapshot
            .assembly_motion_coupling(AssemblyMotionCouplingId(5))
            .unwrap()
            .transmission(),
        AssemblyTransmissionKind::LeadScrew {
            lead_mm_per_revolution: 8.0,
            handedness: ScrewHandedness::Right,
        }
    ));

    let edit = shell.catalog().format(
        "assembly-edit-coupling",
        &BTreeMap::from([("id", "5".to_owned())]),
    );
    shell.click_button_label(&edit);
    shell.app_mut().headless_set_assembly_coupling_parameters(
        [109, 110],
        [0.0, 0.0],
        ["16", ""],
        true,
    );
    shell.settle();
    let before_edit = (
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&preview);
    assert_eq!(shell.app().canonical_digest(), before_edit.0);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().undo_step_count(), before_edit.1 + 1);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .assembly_motion_coupling(AssemblyMotionCouplingId(5))
            .unwrap()
            .transmission(),
        AssemblyTransmissionKind::LeadScrew {
            lead_mm_per_revolution: 16.0,
            handedness: ScrewHandedness::Left,
        }
    ));
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .assembly_motion_coupling(AssemblyMotionCouplingId(5))
            .unwrap()
            .transmission(),
        AssemblyTransmissionKind::LeadScrew {
            lead_mm_per_revolution: 8.0,
            handedness: ScrewHandedness::Right,
        }
    ));
    shell.click_menu_command("menu-edit", AppCommand::Redo);

    shell.app_mut().headless_set_assembly_coupling_parameters(
        [109, 110],
        [0.0, 0.0],
        ["24", ""],
        true,
    );
    shell.settle();
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_intervening_undo = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&shell.catalog().text("assembly-confirm-preview"));
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        after_intervening_undo
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    let bytes = std::fs::read(&saved).unwrap();
    let persisted = persistence::load(&bytes).unwrap();
    assert_eq!(persisted.snapshot().canonical_digest(), persisted_digest);
    assert_eq!(persisted.snapshot().assembly_motion_couplings().count(), 5);
    let core_bytes = persistence::save(&persisted.snapshot());
    assert_eq!(
        persistence::save(&persistence::load(&core_bytes).unwrap().snapshot()),
        core_bytes
    );
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().assembly_motion_coupling_count(), 5);
    assert!(shell.app().can_undo());
}

#[test]
fn mechanism_drag_collision_preview_blocks_or_allows_contact_through_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("collision-drag-source.ketchup");
    let saved = directory.path().join("collision-drag-saved.ketchup");
    write_collision_drag_fixture(&source);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    open_assembly_editor(&mut shell);

    assert!(shell.has_role_and_label(
        Role::CheckBox,
        &shell.catalog().text("assembly-drag-check-collisions")
    ));
    assert!(shell.has_role_and_label(
        Role::CheckBox,
        &shell.catalog().text("assembly-drag-allow-contact")
    ));
    let preview = shell.catalog().text("assembly-preview-drag");
    let confirm = shell.catalog().text("assembly-confirm-preview");
    let cancel = shell.catalog().text("assembly-cancel-preview");
    let baseline = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );

    shell.app_mut().headless_set_assembly_drag(101, 4.0, false);
    shell.settle();
    shell.click_button_label(&preview);
    assert!(shell.has_visible_label(&shell.catalog().text("assembly-drag-clearance-safe")));
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-drag-clearance-minimum",
        &BTreeMap::from([
            ("clearance", "5".to_owned()),
            ("first", "10".to_owned()),
            ("second", "11".to_owned()),
            ("progress", "1".to_owned()),
        ]),
    )));
    confirm_preview(&mut shell);
    assert_eq!(shell.app().undo_step_count(), baseline.2 + 1);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline.1);

    shell.app_mut().headless_set_assembly_drag(101, 20.0, false);
    shell.settle();
    shell.click_button_label(&preview);
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-drag-clearance-contact",
        &BTreeMap::from([
            ("first", "10".to_owned()),
            ("second", "11".to_owned()),
            ("progress", "0.45".to_owned()),
            (
                "policy",
                shell.catalog().text("assembly-drag-contact-blocked"),
            ),
        ]),
    )));
    shell.click_button_label(&confirm);
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(shell.app().canonical_digest(), baseline.1);
    assert_eq!(shell.app().undo_step_count(), baseline.2);
    shell.click_button_label(&cancel);

    shell
        .app_mut()
        .headless_set_assembly_drag_collision_policy(true, true);
    shell.click_button_label(&preview);
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-drag-clearance-contact",
        &BTreeMap::from([
            ("first", "10".to_owned()),
            ("second", "11".to_owned()),
            ("progress", "0.45".to_owned()),
            (
                "policy",
                shell.catalog().text("assembly-drag-contact-allowed"),
            ),
        ]),
    )));
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let stale_document = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&confirm);
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        stale_document
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.app_mut().headless_set_assembly_drag(101, 20.0, false);
    shell.settle();
    shell.click_button_label(&preview);
    confirm_preview(&mut shell);
    assert_eq!(shell.app().undo_step_count(), baseline.2 + 1);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_joint(AssemblyJointId(101))
            .unwrap()
            .kind()
            .position(),
        Some(20.0)
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Redo);

    shell.app_mut().headless_set_assembly_drag(101, 20.0, false);
    shell.settle();
    shell.click_button_label(&preview);
    assert!(!shell.app().assembly_preview_pending());
    assert!(
        shell
            .app()
            .action_digest()
            .contains(&shell.catalog().text("assembly-error-drag-no-change"))
    );

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_joint(AssemblyJointId(101))
            .unwrap()
            .kind()
            .position(),
        Some(20.0)
    );
    assert!(shell.app().can_undo());
}

#[test]
fn rotational_drag_uncertainty_is_visible_and_cannot_be_overridden_as_known_contact() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("rotational-drag-source.ketchup");
    write_rotational_drag_fixture(&source);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    open_assembly_editor(&mut shell);
    shell
        .app_mut()
        .headless_set_assembly_drag_collision_policy(true, true);
    shell
        .app_mut()
        .headless_set_assembly_drag(101, 180.0, false);
    shell.settle();
    let baseline = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );

    shell.click_button_label(&shell.catalog().text("assembly-preview-drag"));
    assert!(shell.app().assembly_preview_pending());
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-drag-clearance-unresolved",
        &BTreeMap::from([
            ("first", "10".to_owned()),
            ("second", "11".to_owned()),
            ("start", "0".to_owned()),
            ("end", "0.03125".to_owned()),
        ]),
    )));
    assert!(!shell.has_visible_label(&shell.catalog().text("assembly-drag-clearance-safe")));

    shell.click_button_label(&shell.catalog().text("assembly-confirm-preview"));
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        baseline
    );
    shell.click_button_label(&shell.catalog().text("assembly-cancel-preview"));
    assert!(!shell.app().assembly_preview_pending());
}

#[test]
fn mechanism_drag_is_previewed_clamped_coupled_and_reopened_through_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("drag-source.ketchup");
    let saved = directory.path().join("drag-saved.ketchup");
    write_coupling_authoring_fixture(&source);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    open_assembly_editor(&mut shell);
    shell
        .app_mut()
        .headless_set_assembly_drag_collision_policy(false, false);

    assert!(shell.has_role_and_label(Role::ComboBox, &shell.catalog().text("assembly-drag-joint")));
    assert!(shell.has_role_and_label(
        Role::TextInput,
        &shell.catalog().text("assembly-drag-position")
    ));
    let preview = shell.catalog().text("assembly-preview-drag");
    let before_invalid = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell
        .app_mut()
        .headless_set_assembly_drag(101, 500.0, false);
    shell.settle();
    shell.click_button_label(&preview);
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        before_invalid
    );
    assert!(
        shell
            .app()
            .action_digest()
            .contains(&shell.catalog().format(
                "assembly-error-drag-unreachable",
                &BTreeMap::from([("id", "101".to_owned())]),
            ))
    );

    shell.app_mut().headless_set_assembly_drag(101, 500.0, true);
    shell.settle();
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    assert_eq!(shell.app().canonical_digest(), before_invalid.1);
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-drag-result",
        &BTreeMap::from([
            ("id", "101".to_owned()),
            ("requested", "500".to_owned()),
            ("applied", "360".to_owned()),
            ("limited", shell.catalog().text("assembly-drag-limited")),
        ]),
    )));
    confirm_preview(&mut shell);
    assert_eq!(shell.app().undo_step_count(), before_invalid.2 + 1);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_joint(AssemblyJointId(101))
            .unwrap()
            .kind()
            .position(),
        Some(360.0)
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_joint(AssemblyJointId(101))
            .unwrap()
            .kind()
            .position(),
        Some(0.0)
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);

    for (joint_id, position) in [(108, 25.0), (109, 90.0)] {
        shell
            .app_mut()
            .headless_set_assembly_drag(joint_id, position, false);
        shell.settle();
        shell.click_button_label(&preview);
        assert!(shell.app().assembly_preview_pending());
        confirm_preview(&mut shell);
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .assembly_joint(AssemblyJointId(joint_id))
                .unwrap()
                .kind()
                .position(),
            Some(position)
        );
    }

    shell.click_role_and_label(
        Role::ComboBox,
        &shell.catalog().text("assembly-coupling-kind"),
    );
    shell.click_button_label(&shell.catalog().text("assembly-coupling-kind-screw"));
    shell.app_mut().headless_set_assembly_coupling_parameters(
        [109, 110],
        [90.0, 0.0],
        ["8", ""],
        false,
    );
    shell.settle();
    shell.click_button_label(&shell.catalog().text("assembly-preview-coupling"));
    confirm_preview(&mut shell);

    shell
        .app_mut()
        .headless_set_assembly_drag(109, 180.0, false);
    shell.settle();
    let before_coupled_drag = shell.app().undo_step_count();
    shell.click_button_label(&preview);
    assert!(shell.has_visible_label(&shell.catalog().format(
        "assembly-kinematic-summary",
        &BTreeMap::from([
            ("status", shell.catalog().text("assembly-kinematic-under")),
            ("dof", "8".to_owned()),
        ]),
    )));
    confirm_preview(&mut shell);
    assert_eq!(shell.app().undo_step_count(), before_coupled_drag + 1);
    let snapshot = shell.app().document_snapshot();
    assert_eq!(
        snapshot
            .assembly_joint(AssemblyJointId(109))
            .unwrap()
            .kind()
            .position(),
        Some(180.0)
    );
    assert_eq!(
        snapshot
            .assembly_joint(AssemblyJointId(110))
            .unwrap()
            .kind()
            .position(),
        Some(2.0)
    );

    shell.app_mut().headless_set_assembly_drag(109, 90.0, false);
    shell.settle();
    shell.click_button_label(&preview);
    assert!(shell.app().assembly_preview_pending());
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_intervening_undo = (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
    );
    shell.click_button_label(&shell.catalog().text("assembly-confirm-preview"));
    assert!(!shell.app().assembly_preview_pending());
    assert_eq!(
        (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        ),
        after_intervening_undo
    );
    shell.click_menu_command("menu-edit", AppCommand::Redo);

    shell
        .app_mut()
        .headless_set_assembly_drag(109, 180.0, false);
    shell.settle();
    shell.click_button_label(&preview);
    assert!(!shell.app().assembly_preview_pending());
    assert!(
        shell
            .app()
            .action_digest()
            .contains(&shell.catalog().text("assembly-error-drag-no-change"))
    );

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    let bytes = std::fs::read(&saved).unwrap();
    let persisted = persistence::load(&bytes).unwrap();
    assert_eq!(persisted.snapshot().canonical_digest(), persisted_digest);
    let core_bytes = persistence::save(&persisted.snapshot());
    assert_eq!(
        persistence::save(&persistence::load(&core_bytes).unwrap().snapshot()),
        core_bytes
    );
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .assembly_joint(AssemblyJointId(110))
            .unwrap()
            .kind()
            .position(),
        Some(2.0)
    );
    assert!(shell.app().can_undo());
}

#[test]
fn kinematic_conflict_and_coupled_limit_are_localized_and_fail_closed_in_preview() {
    let directory = tempfile::tempdir().unwrap();
    for (index, fixture, error_key, id) in [
        (
            0,
            KinematicFailureFixture::CouplingConflict,
            "assembly-error-kinematic-conflict-coupling",
            "201",
        ),
        (
            1,
            KinematicFailureFixture::CoupledLimit,
            "assembly-error-kinematic-limit-joint",
            "102",
        ),
    ] {
        let source = directory
            .path()
            .join(format!("kinematic-failure-{index}.ketchup"));
        write_kinematic_failure_fixture(&source, fixture);
        let mut shell = Shell::with_dialogs(
            ScriptedFileDialogs::new()
                .queue_open(&source)
                .always_discard(),
        );
        shell.click_menu_command("menu-file", AppCommand::Open);
        assert!(shell.app_mut().select_tag_occurrences(TagId(50)));
        shell.settle();
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell.app_mut().headless_set_assembly_motion_position(180.0);
        open_assembly_editor(&mut shell);

        let before = (
            shell.app().document_revision(),
            shell.app().canonical_digest(),
            shell.app().undo_step_count(),
        );
        let preview = shell.catalog().text("assembly-preview-motion-study");
        shell.click_button_label(&preview);
        assert!(!shell.app().assembly_preview_pending());
        assert_eq!(
            (
                shell.app().document_revision(),
                shell.app().canonical_digest(),
                shell.app().undo_step_count(),
            ),
            before
        );
        let reason = shell
            .catalog()
            .format(error_key, &BTreeMap::from([("id", id.to_owned())]));
        let error = shell
            .catalog()
            .format("assembly-error", &BTreeMap::from([("reason", reason)]));
        assert_eq!(shell.app().action_digest(), error);
        let accessible_status = format!(
            "{}  ·  {error}",
            shell.catalog().format(
                "status-selected",
                &BTreeMap::from([("count", "2".to_owned())]),
            )
        );
        assert!(shell.has_visible_label(&accessible_status));
    }
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
    let instance_paths = occurrences
        .iter()
        .map(|(id, _)| ketchup_core::document::InstancePath::root(*id))
        .collect::<Vec<_>>();
    let snapshot = shell.app().document_snapshot();
    let sheet = snapshot.drawing_sheet(sheet_id).unwrap();
    assert_eq!(
        sheet.source(),
        &DrawingSource::RigidAssemblyInstances {
            instance_paths: instance_paths.clone(),
        }
    );
    assert_eq!(sheet.bom_balloons().len(), 3);
    assert!(
        sheet
            .bom_balloons()
            .iter()
            .all(|balloon| balloon.position() == 1)
    );
    assert_eq!(
        sheet
            .bom_balloons()
            .iter()
            .map(|balloon| balloon.instance_path().clone())
            .collect::<Vec<_>>(),
        instance_paths
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
        &DrawingSource::RigidAssemblyInstances { instance_paths }
    );
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    wait_for_stable_references(&mut shell);
    assert_eq!(
        shell.app().headless_drawing_fingerprint(sheet_id),
        Some(exact_fingerprint)
    );
    assert!(shell.app().can_undo());
}

#[test]
fn production_surface_keeps_general_workflows() {
    for catalog in [LocaleCatalog::english(), LocaleCatalog::slovak()] {
        let mut shell = Shell::with_catalog(catalog);

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
