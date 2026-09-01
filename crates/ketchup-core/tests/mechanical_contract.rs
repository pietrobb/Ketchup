use ketchup_core::assembly_joint::{
    AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind, AssemblyJointLimits,
    AssemblyMotionDriver, AssemblyMotionStudy, AssemblyMotionStudyId, sample_assembly_motion_study,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, OccurrenceId, Snapshot, Transform,
};
use ketchup_core::mechanical_contract::{
    MechanicalAxisAlignment, MechanicalCondition, MechanicalConditionId, MechanicalConditionKind,
    MechanicalContractError, MechanicalInterface, MechanicalInterfaceId, MechanicalPlanarFrame,
    MechanicalRole, MechanicalViolationKind, capture_authored_face_frame,
    preview_mechanical_contract,
};
use ketchup_core::mechanical_coupling::{
    AssemblyMotionCoupling, AssemblyMotionCouplingId, AssemblyMotionDirection,
    AssemblyTransmissionKind, GearMeshKind,
};
use ketchup_core::{persistence, state_view::encode_semantic_state};

const WALL: OccurrenceId = OccurrenceId(1);
const RAIL: OccurrenceId = OccurrenceId(2);
const SLIDER: OccurrenceId = OccurrenceId(3);
const MOUNT_JOINT: AssemblyJointId = AssemblyJointId(101);
const SLIDE_JOINT: AssemblyJointId = AssemblyJointId(102);
const STUDY: AssemblyMotionStudyId = AssemblyMotionStudyId(300);

const WALL_FACE: MechanicalInterfaceId = MechanicalInterfaceId(11);
const RAIL_BACK_FACE: MechanicalInterfaceId = MechanicalInterfaceId(12);
const RAIL_TOP_FACE: MechanicalInterfaceId = MechanicalInterfaceId(13);
const SLIDER_BOTTOM_FACE: MechanicalInterfaceId = MechanicalInterfaceId(14);
const RAIL_END_FACE: MechanicalInterfaceId = MechanicalInterfaceId(15);

const CONTACT: MechanicalConditionId = MechanicalConditionId(21);
const SUPPORT: MechanicalConditionId = MechanicalConditionId(22);
const AXIS: MechanicalConditionId = MechanicalConditionId(23);
const TRAVEL: MechanicalConditionId = MechanicalConditionId(24);

const REQUIRED_TRAVEL_MM: f64 = 280.0;

fn append_box(
    commands: &mut Vec<CanonicalCommand>,
    definition_id: u64,
    occurrence_id: OccurrenceId,
    name: &str,
    size_mm: [f64; 3],
    position_mm: [f64; 3],
) {
    let definition_id = DefinitionId(definition_id);
    let profile = FeatureId(definition_id.0 * 10);
    let extrusion = FeatureId(definition_id.0 * 10 + 1);
    commands.extend([
        CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: name.to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: profile,
            definition_id,
            name: format!("{name} profile"),
            kind: FeatureKind::Profile {
                points_mm: vec![
                    [0.0, 0.0],
                    [size_mm[0], 0.0],
                    [size_mm[0], size_mm[1]],
                    [0.0, size_mm[1]],
                ],
            },
        },
        CanonicalCommand::CreateFeature {
            id: extrusion,
            definition_id,
            name: format!("{name} body"),
            kind: FeatureKind::Extrusion {
                profile,
                height: Dimension::from_decimal(size_mm[2].to_string()).unwrap(),
            },
        },
        CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name: name.to_owned(),
            transform: Transform::from_translation(position_mm[0], position_mm[1], position_mm[2])
                .unwrap(),
            parent: None,
            tag: None,
            visible: true,
        },
    ]);
}

fn slide_joint(axis: [f64; 3], limits: AssemblyJointLimits) -> AssemblyJointKind {
    AssemblyJointKind::Prismatic {
        axis: AssemblyJointAxis::new(axis, [0.0, 0.0, 0.0]),
        limits: Some(limits),
        position_mm: 0.0,
    }
}

fn geometry_document_with_slide_axis(slide_axis: [f64; 3]) -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = Vec::new();
    append_box(
        &mut commands,
        1,
        WALL,
        "Wall panel",
        [20.0, 400.0, 300.0],
        [0.0, 0.0, 0.0],
    );
    append_box(
        &mut commands,
        2,
        RAIL,
        "Rail",
        [30.0, 380.0, 40.0],
        [20.0, 10.0, 100.0],
    );
    append_box(
        &mut commands,
        3,
        SLIDER,
        "Slider",
        [25.0, 100.0, 20.0],
        [20.0, 10.0, 140.0],
    );
    commands.extend([
        CanonicalCommand::SetOccurrenceGrounded {
            id: WALL,
            grounded: true,
        },
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            MOUNT_JOINT,
            WALL,
            RAIL,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            SLIDE_JOINT,
            RAIL,
            SLIDER,
            slide_joint(
                slide_axis,
                AssemblyJointLimits::new(0.0, REQUIRED_TRAVEL_MM),
            ),
        )),
        CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
            STUDY,
            "Open the slider",
            vec![AssemblyMotionDriver::new(SLIDE_JOINT, REQUIRED_TRAVEL_MM)],
        )),
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

fn interface(
    snapshot: &Snapshot,
    id: MechanicalInterfaceId,
    occurrence_id: OccurrenceId,
    role: MechanicalRole,
    face_ordinal: u32,
) -> MechanicalInterface {
    let frame = capture_authored_face_frame(snapshot, occurrence_id, face_ordinal)
        .expect("the face must be captured from the authored body itself");
    MechanicalInterface::new(id, occurrence_id, role, face_ordinal, "", frame)
}

fn contract_commands(snapshot: &Snapshot) -> Vec<CanonicalCommand> {
    vec![
        CanonicalCommand::CreateMechanicalInterface(interface(
            snapshot,
            WALL_FACE,
            WALL,
            MechanicalRole::Mounting,
            1,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            snapshot,
            RAIL_BACK_FACE,
            RAIL,
            MechanicalRole::Mounting,
            0,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            snapshot,
            RAIL_TOP_FACE,
            RAIL,
            MechanicalRole::Support,
            5,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            snapshot,
            SLIDER_BOTTOM_FACE,
            SLIDER,
            MechanicalRole::Support,
            4,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            snapshot,
            RAIL_END_FACE,
            RAIL,
            MechanicalRole::Guide,
            3,
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            CONTACT,
            MechanicalConditionKind::PlanarContact {
                first: WALL_FACE,
                second: RAIL_BACK_FACE,
                offset_mm: 0.0,
                tolerance_mm: 1.0e-6,
            },
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            SUPPORT,
            MechanicalConditionKind::Support {
                supported: SLIDER_BOTTOM_FACE,
                supporting: RAIL_TOP_FACE,
                tolerance_mm: 1.0e-6,
            },
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            AXIS,
            MechanicalConditionKind::JointAxisAlignment {
                joint_id: SLIDE_JOINT,
                interface: RAIL_END_FACE,
                alignment: MechanicalAxisAlignment::Parallel,
                tolerance_degrees: 1.0,
            },
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            TRAVEL,
            MechanicalConditionKind::JointTravel {
                joint_id: SLIDE_JOINT,
                minimum: 0.0,
                maximum: REQUIRED_TRAVEL_MM,
            },
        )),
    ]
}

fn geometry_document() -> DocumentStore {
    geometry_document_with_slide_axis([0.0, 1.0, 0.0])
}

fn contract_document_with_slide_axis(slide_axis: [f64; 3]) -> DocumentStore {
    let mut document = geometry_document_with_slide_axis(slide_axis);
    let commands = contract_commands(&document.current());
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

fn contract_document() -> DocumentStore {
    contract_document_with_slide_axis([0.0, 1.0, 0.0])
}

fn violations_of(
    document: &DocumentStore,
    study: AssemblyMotionStudyId,
) -> Vec<MechanicalViolationKind> {
    preview_mechanical_contract(&document.current(), study, 8)
        .unwrap()
        .violations()
        .iter()
        .map(|violation| violation.kind())
        .collect()
}

fn violations(document: &DocumentStore) -> Vec<MechanicalViolationKind> {
    preview_mechanical_contract(&document.current(), STUDY, 8)
        .unwrap()
        .violations()
        .iter()
        .map(|violation| violation.kind())
        .collect()
}

#[test]
fn captured_frames_come_from_the_authored_geometry_not_from_typed_numbers() {
    let document = geometry_document();
    let snapshot = document.current();

    let wall_face = capture_authored_face_frame(&snapshot, WALL, 1).unwrap();
    assert_eq!(wall_face.origin_mm(), [20.0, 200.0, 150.0]);
    assert_eq!(wall_face.normal(), [1.0, 0.0, 0.0]);
    assert_eq!(wall_face.area_mm2(), 120_000.0);
    assert_eq!(
        wall_face.bounds_mm(),
        [[20.0, 0.0, 0.0], [20.0, 400.0, 300.0]]
    );

    let rail_back = capture_authored_face_frame(&snapshot, RAIL, 0).unwrap();
    assert_eq!(rail_back.origin_mm(), [0.0, 190.0, 20.0]);
    assert_eq!(rail_back.normal(), [-1.0, 0.0, 0.0]);
    assert_eq!(rail_back.area_mm2(), 15_200.0);

    let rail_top = capture_authored_face_frame(&snapshot, RAIL, 5).unwrap();
    assert_eq!(rail_top.origin_mm(), [15.0, 190.0, 40.0]);
    assert_eq!(rail_top.normal(), [0.0, 0.0, 1.0]);
    assert_eq!(rail_top.area_mm2(), 11_400.0);

    assert_eq!(capture_authored_face_frame(&snapshot, RAIL, 6), None);
}

#[test]
fn a_physically_correct_assembly_satisfies_the_contract_along_the_whole_path() {
    let document = contract_document();
    let report = preview_mechanical_contract(&document.current(), STUDY, 16).unwrap();

    assert!(
        report.is_satisfied(),
        "unexpected violations: {:?}",
        report.violations()
    );
    assert_eq!(report.evaluated_conditions(), 4);
    assert_eq!(report.evaluated_samples(), 17);
    assert_eq!(report.study_id(), STUDY);
    assert_eq!(report.source_revision(), document.current().revision_id());
}

#[test]
fn a_part_that_floats_away_from_its_mounting_wall_is_rejected() {
    let mut document = contract_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: RAIL,
                transform: Transform::from_translation(21.0, 10.0, 100.0).unwrap(),
            },
        ]))
        .unwrap();

    let report = preview_mechanical_contract(&document.current(), STUDY, 4).unwrap();
    assert!(!report.is_satisfied());
    let contact = report
        .violations()
        .iter()
        .find(|violation| violation.condition_id() == Some(CONTACT))
        .expect("the mounting contact must fail");
    let MechanicalViolationKind::ContactGap {
        measured_mm,
        allowed_mm,
    } = contact.kind()
    else {
        panic!("expected a contact gap, got {:?}", contact.kind());
    };
    assert!((measured_mm - 1.0).abs() <= 1.0e-9);
    assert_eq!(allowed_mm, 0.0);
    assert_eq!(contact.interface_id(), Some(RAIL_BACK_FACE));
}

#[test]
fn mounting_with_the_wrong_face_of_the_same_part_is_rejected() {
    let mut document = contract_document();
    let snapshot = document.current();
    // The rail's outward +X face is a real face of the same body, but it faces
    // away from the wall: the contract must not accept it as a mounting face.
    let flipped = interface(&snapshot, RAIL_BACK_FACE, RAIL, MechanicalRole::Mounting, 1);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateMechanicalInterface(flipped),
        ]))
        .unwrap();

    let report = preview_mechanical_contract(&document.current(), STUDY, 4).unwrap();
    let contact = report
        .violations()
        .iter()
        .find(|violation| violation.condition_id() == Some(CONTACT))
        .expect("the reversed mounting face must fail");
    let MechanicalViolationKind::ContactOrientation { measured_cosine } = contact.kind() else {
        panic!("expected an orientation failure, got {:?}", contact.kind());
    };
    assert!((measured_cosine - 1.0).abs() <= 1.0e-9);
}

#[test]
fn travelling_past_the_end_of_the_supporting_face_loses_support() {
    let mut document = contract_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointLimits {
                id: SLIDE_JOINT,
                limits: Some(AssemblyJointLimits::new(0.0, 400.0)),
            },
            CanonicalCommand::UpdateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "Open the slider",
                vec![AssemblyMotionDriver::new(SLIDE_JOINT, 400.0)],
            )),
        ]))
        .unwrap();

    let report = preview_mechanical_contract(&document.current(), STUDY, 16).unwrap();
    let lost = report
        .violations()
        .iter()
        .find(|violation| {
            matches!(
                violation.kind(),
                MechanicalViolationKind::SupportLost { .. }
            )
        })
        .expect("the slider must lose its support before the end of travel");
    assert_eq!(lost.condition_id(), Some(SUPPORT));
    assert!(lost.progress() > 0.9);
    // The same document is still fully supported over the originally proven range.
    assert!(
        report
            .violations()
            .iter()
            .all(|violation| violation.progress() > 0.5)
    );
}

#[test]
fn a_guide_axis_turned_the_wrong_way_is_rejected() {
    let document = contract_document_with_slide_axis([1.0, 0.0, 0.0]);

    let report = preview_mechanical_contract(&document.current(), STUDY, 2).unwrap();
    let misaligned = report
        .violations()
        .iter()
        .find(|violation| violation.condition_id() == Some(AXIS))
        .expect("a guide axis perpendicular to the declared one must fail");
    let MechanicalViolationKind::AxisMisaligned {
        measured_degrees,
        allowed_degrees,
    } = misaligned.kind()
    else {
        panic!("expected an axis failure, got {:?}", misaligned.kind());
    };
    assert!((measured_degrees - 90.0).abs() <= 1.0e-9);
    assert_eq!(allowed_degrees, 1.0);
}

#[test]
fn a_joint_that_cannot_reach_the_required_travel_is_rejected() {
    let mut document = contract_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyJointLimits {
                id: SLIDE_JOINT,
                limits: Some(AssemblyJointLimits::new(0.0, 200.0)),
            },
            CanonicalCommand::UpdateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "Open the slider",
                vec![AssemblyMotionDriver::new(SLIDE_JOINT, 200.0)],
            )),
        ]))
        .unwrap();

    let report = preview_mechanical_contract(&document.current(), STUDY, 4).unwrap();
    assert_eq!(
        report
            .violations()
            .iter()
            .map(|violation| (violation.condition_id(), violation.kind()))
            .collect::<Vec<_>>(),
        vec![(
            Some(TRAVEL),
            MechanicalViolationKind::TravelNotCovered {
                required_minimum: 0.0,
                required_maximum: REQUIRED_TRAVEL_MM,
            }
        )]
    );
}

#[test]
fn an_invented_frame_that_no_face_produces_is_rejected() {
    let mut document = contract_document();
    let snapshot = document.current();
    let real = capture_authored_face_frame(&snapshot, WALL, 1).unwrap();
    let invented = MechanicalPlanarFrame::new(
        [
            real.origin_mm()[0] + 5.0,
            real.origin_mm()[1],
            real.origin_mm()[2],
        ],
        real.normal(),
        real.area_mm2(),
        [
            [
                real.bounds_mm()[0][0] + 5.0,
                real.bounds_mm()[0][1],
                real.bounds_mm()[0][2],
            ],
            [
                real.bounds_mm()[1][0] + 5.0,
                real.bounds_mm()[1][1],
                real.bounds_mm()[1][2],
            ],
        ],
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateMechanicalInterface(MechanicalInterface::new(
                WALL_FACE,
                WALL,
                MechanicalRole::Mounting,
                1,
                "",
                invented,
            )),
        ]))
        .unwrap();

    let report = preview_mechanical_contract(&document.current(), STUDY, 2).unwrap();
    assert!(
        report.violations().iter().any(|violation| {
            violation.interface_id() == Some(WALL_FACE)
                && violation.kind() == MechanicalViolationKind::UnverifiableFrame
        }),
        "a frame five millimetres off the real face must not be provable: {:?}",
        report.violations()
    );
}

#[test]
fn geometry_evidence_that_does_not_belong_to_the_body_is_rejected() {
    let mut document = contract_document();
    let snapshot = document.current();
    let frame = capture_authored_face_frame(&snapshot, WALL, 1).unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateMechanicalInterface(MechanicalInterface::new(
                WALL_FACE,
                WALL,
                MechanicalRole::Mounting,
                1,
                "fnv1a64:0000000000000000",
                frame,
            )),
        ]))
        .unwrap();

    let report = preview_mechanical_contract(&document.current(), STUDY, 2).unwrap();
    assert!(report.violations().iter().any(|violation| {
        violation.interface_id() == Some(WALL_FACE)
            && violation.kind() == MechanicalViolationKind::StaleGeometryEvidence
    }));
}

#[test]
fn a_declared_role_without_a_proving_condition_is_reported() {
    let mut document = contract_document();
    let snapshot = document.current();
    let orphan = MechanicalInterfaceId(16);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateMechanicalInterface(interface(
                &snapshot,
                orphan,
                SLIDER,
                MechanicalRole::Mounting,
                5,
            )),
        ]))
        .unwrap();

    assert!(
        violations(&document).contains(&MechanicalViolationKind::RoleWithoutCondition {
            role: MechanicalRole::Mounting,
        })
    );
}

#[test]
fn the_contract_survives_save_and_open_and_is_visible_to_the_agent() {
    let document = contract_document();
    let snapshot = document.current();
    let bytes = persistence::save(&snapshot);
    let reopened = persistence::load(&bytes).unwrap();
    let reopened_snapshot = reopened.snapshot();

    assert_eq!(
        reopened_snapshot.mechanical_interfaces().count(),
        snapshot.mechanical_interfaces().count()
    );
    assert_eq!(
        reopened_snapshot.mechanical_conditions().count(),
        snapshot.mechanical_conditions().count()
    );
    assert_eq!(
        reopened_snapshot.mechanical_interface(RAIL_TOP_FACE),
        snapshot.mechanical_interface(RAIL_TOP_FACE)
    );
    assert_eq!(
        reopened_snapshot.mechanical_condition(SUPPORT),
        snapshot.mechanical_condition(SUPPORT)
    );
    assert_eq!(
        reopened_snapshot.canonical_digest(),
        snapshot.canonical_digest()
    );
    assert!(
        preview_mechanical_contract(&reopened_snapshot, STUDY, 8)
            .unwrap()
            .is_satisfied()
    );

    let state = encode_semantic_state(&reopened_snapshot);
    assert!(
        state
            .complete_v1()
            .contains("mechanical_interface.13=schema:")
    );
    assert!(state.agent_v1().contains("mechanical_condition.22="));
    assert!(state.agent_v1().contains("role:support"));
    assert!(state.agent_v1().contains("kind:joint_travel"));
}

#[test]
fn conditions_cannot_reference_missing_interfaces_joints_or_fixed_axes() {
    let mut document = contract_document();
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
                MechanicalConditionId(31),
                MechanicalConditionKind::PlanarContact {
                    first: WALL_FACE,
                    second: MechanicalInterfaceId(999),
                    offset_mm: 0.0,
                    tolerance_mm: 0.0,
                },
            )),
        ])),
        Err(CanonicalError::MechanicalInterfaceNotFound(
            MechanicalInterfaceId(999)
        ))
    ));
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
                MechanicalConditionId(32),
                MechanicalConditionKind::JointAxisAlignment {
                    joint_id: MOUNT_JOINT,
                    interface: RAIL_END_FACE,
                    alignment: MechanicalAxisAlignment::Parallel,
                    tolerance_degrees: 1.0,
                },
            )),
        ])),
        Err(CanonicalError::InvalidMechanicalCondition(
            MechanicalConditionId(32)
        ))
    ));
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteMechanicalInterface { id: WALL_FACE },
        ])),
        Err(CanonicalError::MechanicalInterfaceInCondition(WALL_FACE))
    ));
}

/// A swinging mount: the rail is hinged on the wall and driven through a full
/// turn. Both end poses put the rail flat against the wall, so any validator
/// that only inspected the start and the end pose would call this assembly
/// correct. It is not: in between, the rail leaves the wall entirely.
fn swinging_mount_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = Vec::new();
    append_box(
        &mut commands,
        1,
        WALL,
        "Wall panel",
        [20.0, 400.0, 300.0],
        [0.0, 0.0, 0.0],
    );
    append_box(
        &mut commands,
        2,
        RAIL,
        "Rail",
        [30.0, 380.0, 40.0],
        [20.0, 10.0, 100.0],
    );
    commands.extend([
        CanonicalCommand::SetOccurrenceGrounded {
            id: WALL,
            grounded: true,
        },
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            MOUNT_JOINT,
            WALL,
            RAIL,
            AssemblyJointKind::Revolute {
                axis: AssemblyJointAxis::new([0.0, 0.0, 1.0], [20.0, 0.0, 0.0]),
                limits: Some(AssemblyJointLimits::new(0.0, 360.0)),
                position_degrees: 0.0,
            },
        )),
        CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
            STUDY,
            "Swing the rail",
            vec![AssemblyMotionDriver::new(MOUNT_JOINT, 360.0)],
        )),
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let snapshot = document.current();
    let commands = vec![
        CanonicalCommand::CreateMechanicalInterface(interface(
            &snapshot,
            WALL_FACE,
            WALL,
            MechanicalRole::Mounting,
            1,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            &snapshot,
            RAIL_BACK_FACE,
            RAIL,
            MechanicalRole::Mounting,
            0,
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            CONTACT,
            MechanicalConditionKind::PlanarContact {
                first: WALL_FACE,
                second: RAIL_BACK_FACE,
                offset_mm: 0.0,
                tolerance_mm: 1.0e-6,
            },
        )),
    ];
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

#[test]
fn a_violation_that_exists_only_between_the_end_poses_is_still_rejected() {
    let document = swinging_mount_document();
    let report = preview_mechanical_contract(&document.current(), STUDY, 4).unwrap();

    assert_eq!(report.evaluated_samples(), 5);
    assert!(
        !report.is_satisfied(),
        "the mid-path swing away from the wall must be reported"
    );

    // Both static end poses are genuinely mounted, so a start/end-only check passes.
    for progress in [0.0, 1.0] {
        assert!(
            report
                .violations()
                .iter()
                .all(|violation| violation.progress() != progress),
            "pose at progress {progress} is physically correct and must not be blamed"
        );
    }

    // Every reported failure is an interior sample of the same path.
    assert!(
        report
            .violations()
            .iter()
            .all(|violation| violation.progress() > 0.0 && violation.progress() < 1.0)
    );
    let mid = report
        .violations()
        .iter()
        .find(|violation| violation.progress() == 0.5)
        .expect("the half-turn pose must fail");
    assert_eq!(mid.condition_id(), Some(CONTACT));
    let MechanicalViolationKind::ContactOrientation { measured_cosine } = mid.kind() else {
        panic!("expected an orientation failure, got {:?}", mid.kind());
    };
    assert!((measured_cosine - 1.0).abs() <= 1.0e-9);
}

// ---------------------------------------------------------------------------
// The same contract on a generic transmission chain: nothing below is specific
// to any product. A gear pair drives a rack, and the mechanical contract is
// evaluated against the motion the transmission derives — not against a pose
// somebody typed in.
// ---------------------------------------------------------------------------

const BASE: OccurrenceId = OccurrenceId(41);
const CARRIAGE: OccurrenceId = OccurrenceId(42);
const BRACKET: OccurrenceId = OccurrenceId(43);
const PINION: OccurrenceId = OccurrenceId(44);
const WHEEL: OccurrenceId = OccurrenceId(45);

const DRIVE_JOINT: AssemblyJointId = AssemblyJointId(301);
const WHEEL_JOINT: AssemblyJointId = AssemblyJointId(302);
const RACK_JOINT: AssemblyJointId = AssemblyJointId(303);
const BRACKET_JOINT: AssemblyJointId = AssemblyJointId(304);
const CHAIN_STUDY: AssemblyMotionStudyId = AssemblyMotionStudyId(400);

const BASE_TOP: MechanicalInterfaceId = MechanicalInterfaceId(51);
const CARRIAGE_FOOT: MechanicalInterfaceId = MechanicalInterfaceId(52);
const BASE_END: MechanicalInterfaceId = MechanicalInterfaceId(53);
const BRACKET_FACE: MechanicalInterfaceId = MechanicalInterfaceId(54);
const CARRIAGE_FLANK: MechanicalInterfaceId = MechanicalInterfaceId(55);

const CARRIAGE_SUPPORT: MechanicalConditionId = MechanicalConditionId(61);
const BRACKET_CONTACT: MechanicalConditionId = MechanicalConditionId(62);
const RACK_GUIDE: MechanicalConditionId = MechanicalConditionId(63);
const RACK_TRAVEL: MechanicalConditionId = MechanicalConditionId(64);

/// Four input turns backwards drive the 20/40 gear pair, and the pinion converts
/// that into 251.327… mm of rack travel.
const DRIVE_DEGREES: f64 = -1440.0;
const PINION_PITCH_DIAMETER_MM: f64 = 40.0;

fn spin(limits: AssemblyJointLimits) -> AssemblyJointKind {
    AssemblyJointKind::Revolute {
        axis: AssemblyJointAxis::new([0.0, 0.0, 1.0], [0.0, 0.0, 0.0]),
        limits: Some(limits),
        position_degrees: 0.0,
    }
}

fn transmission_document(rack_limits: AssemblyJointLimits, drive: f64) -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = Vec::new();
    append_box(
        &mut commands,
        21,
        BASE,
        "Bed",
        [400.0, 200.0, 20.0],
        [0.0, 0.0, 0.0],
    );
    append_box(
        &mut commands,
        22,
        CARRIAGE,
        "Carriage",
        [60.0, 60.0, 30.0],
        [20.0, 70.0, 20.0],
    );
    append_box(
        &mut commands,
        23,
        BRACKET,
        "End bracket",
        [20.0, 60.0, 60.0],
        [-20.0, 70.0, 20.0],
    );
    append_box(
        &mut commands,
        24,
        PINION,
        "Drive gear",
        [40.0, 40.0, 40.0],
        [100.0, 70.0, 20.0],
    );
    append_box(
        &mut commands,
        25,
        WHEEL,
        "Driven gear",
        [40.0, 40.0, 40.0],
        [200.0, 70.0, 20.0],
    );
    let spin_limits = AssemblyJointLimits::new(-360_000.0, 360_000.0);
    commands.extend([
        CanonicalCommand::SetOccurrenceGrounded {
            id: BASE,
            grounded: true,
        },
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            DRIVE_JOINT,
            BASE,
            PINION,
            spin(spin_limits),
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            WHEEL_JOINT,
            BASE,
            WHEEL,
            spin(spin_limits),
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            RACK_JOINT,
            BASE,
            CARRIAGE,
            AssemblyJointKind::Prismatic {
                axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                limits: Some(rack_limits),
                position_mm: 0.0,
            },
        )),
        CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            BRACKET_JOINT,
            BASE,
            BRACKET,
            AssemblyJointKind::Fixed,
        )),
        CanonicalCommand::CreateAssemblyMotionCoupling(AssemblyMotionCoupling::new(
            AssemblyMotionCouplingId(401),
            DRIVE_JOINT,
            WHEEL_JOINT,
            0.0,
            0.0,
            AssemblyTransmissionKind::GearPair {
                input_teeth: 20,
                output_teeth: 40,
                mesh: GearMeshKind::External,
            },
        )),
        CanonicalCommand::CreateAssemblyMotionCoupling(AssemblyMotionCoupling::new(
            AssemblyMotionCouplingId(402),
            WHEEL_JOINT,
            RACK_JOINT,
            0.0,
            0.0,
            AssemblyTransmissionKind::RackAndPinion {
                pinion_pitch_diameter_mm: PINION_PITCH_DIAMETER_MM,
                direction: AssemblyMotionDirection::Same,
            },
        )),
        CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
            CHAIN_STUDY,
            "Run the transmission",
            vec![AssemblyMotionDriver::new(DRIVE_JOINT, drive)],
        )),
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let snapshot = document.current();
    let contract = vec![
        CanonicalCommand::CreateMechanicalInterface(interface(
            &snapshot,
            BASE_TOP,
            BASE,
            MechanicalRole::Support,
            5,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            &snapshot,
            CARRIAGE_FOOT,
            CARRIAGE,
            MechanicalRole::Support,
            4,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            &snapshot,
            BASE_END,
            BASE,
            MechanicalRole::Mounting,
            0,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            &snapshot,
            BRACKET_FACE,
            BRACKET,
            MechanicalRole::Mounting,
            1,
        )),
        CanonicalCommand::CreateMechanicalInterface(interface(
            &snapshot,
            CARRIAGE_FLANK,
            CARRIAGE,
            MechanicalRole::Guide,
            3,
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            CARRIAGE_SUPPORT,
            MechanicalConditionKind::Support {
                supported: CARRIAGE_FOOT,
                supporting: BASE_TOP,
                tolerance_mm: 1.0e-6,
            },
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            BRACKET_CONTACT,
            MechanicalConditionKind::PlanarContact {
                first: BASE_END,
                second: BRACKET_FACE,
                offset_mm: 0.0,
                tolerance_mm: 1.0e-6,
            },
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            RACK_GUIDE,
            MechanicalConditionKind::JointAxisAlignment {
                joint_id: RACK_JOINT,
                interface: CARRIAGE_FLANK,
                alignment: MechanicalAxisAlignment::Perpendicular,
                tolerance_degrees: 1.0e-6,
            },
        )),
        CanonicalCommand::CreateMechanicalCondition(MechanicalCondition::new(
            RACK_TRAVEL,
            MechanicalConditionKind::JointTravel {
                joint_id: RACK_JOINT,
                minimum: 0.0,
                maximum: rack_limits.max(),
            },
        )),
    ];
    document.apply_batch(&CommandBatch::new(contract)).unwrap();
    document
}

fn derived_rack_travel_mm(drive_degrees: f64) -> f64 {
    let wheel_degrees = -0.5 * drive_degrees;
    std::f64::consts::PI * PINION_PITCH_DIAMETER_MM / 360.0 * wheel_degrees
}

#[test]
fn a_generic_transmission_chain_satisfies_the_same_contract_over_its_derived_motion() {
    let travel_mm = derived_rack_travel_mm(DRIVE_DEGREES);
    assert!((travel_mm - 251.327_412_287_183_45).abs() <= 1.0e-9);

    let document = transmission_document(AssemblyJointLimits::new(0.0, 260.0), DRIVE_DEGREES);
    let report = preview_mechanical_contract(&document.current(), CHAIN_STUDY, 12).unwrap();
    assert!(
        report.is_satisfied(),
        "the correct chain must satisfy its contract: {:?}",
        report.violations()
    );
    assert_eq!(report.evaluated_samples(), 13);
    assert_eq!(report.evaluated_conditions(), 4);

    // The rack really is driven through the couplings, not by a typed-in pose.
    let path = sample_assembly_motion_study(&document.current(), CHAIN_STUDY, 1).unwrap();
    let end = path.samples().last().unwrap();
    let moved_mm = end
        .solution()
        .pose(CARRIAGE)
        .unwrap()
        .world_transform()
        .matrix()[3]
        - document
            .current()
            .occurrence(CARRIAGE)
            .unwrap()
            .transform()
            .matrix()[3];
    assert!((moved_mm - travel_mm).abs() <= 1.0e-9);
}

#[test]
fn a_transmission_ratio_that_overruns_the_supporting_bed_is_rejected() {
    // Nine input turns instead of four: the same, unchanged contract now sees the
    // carriage driven straight off the end of the bed that carries it.
    let overrun = -3240.0;
    assert!(derived_rack_travel_mm(overrun) > 400.0);
    let document = transmission_document(AssemblyJointLimits::new(0.0, 600.0), overrun);
    let report = preview_mechanical_contract(&document.current(), CHAIN_STUDY, 12).unwrap();

    let lost = report
        .violations()
        .iter()
        .find(|violation| {
            matches!(
                violation.kind(),
                MechanicalViolationKind::SupportLost { .. }
            )
        })
        .expect("a carriage driven past the bed must lose its support");
    assert_eq!(lost.condition_id(), Some(CARRIAGE_SUPPORT));
    assert!(lost.progress() > 0.5 && lost.progress() < 1.0 + f64::EPSILON);
    // The first half of the same run is still fully supported.
    assert!(
        report
            .violations()
            .iter()
            .all(|violation| violation.progress() > 0.5)
    );
}

#[test]
fn the_generic_chain_rejects_an_offset_mount_a_bad_frame_and_a_wrong_guide_axis() {
    let limits = AssemblyJointLimits::new(0.0, 260.0);

    // An end bracket pushed 1 mm off the bed it bolts to.
    let mut document = transmission_document(limits, DRIVE_DEGREES);
    let mut matrix = *document
        .current()
        .occurrence(BRACKET)
        .unwrap()
        .transform()
        .matrix();
    matrix[3] += 1.0;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: BRACKET,
                transform: Transform::from_matrix(matrix).unwrap(),
            },
        ]))
        .unwrap();
    let gap = preview_mechanical_contract(&document.current(), CHAIN_STUDY, 4)
        .unwrap()
        .violations()
        .iter()
        .find(|violation| violation.condition_id() == Some(BRACKET_CONTACT))
        .copied()
        .expect("a bracket floating off its mount must be rejected");
    let MechanicalViolationKind::ContactGap { measured_mm, .. } = gap.kind() else {
        panic!("expected a mounting gap, got {:?}", gap.kind());
    };
    assert!((measured_mm + 1.0).abs() <= 1.0e-9);

    // A mounting frame whose normal was turned around no longer matches the body.
    let mut document = transmission_document(limits, DRIVE_DEGREES);
    let original = document
        .current()
        .mechanical_interface(BRACKET_FACE)
        .unwrap()
        .clone();
    let frame = original.frame();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateMechanicalInterface(MechanicalInterface::new(
                original.id(),
                original.occurrence_id(),
                original.role(),
                original.face_ordinal(),
                original.geometry_fingerprint(),
                MechanicalPlanarFrame::new(
                    frame.origin_mm(),
                    frame.normal().map(|value| -value),
                    frame.area_mm2(),
                    frame.bounds_mm(),
                ),
            )),
        ]))
        .unwrap();
    assert!(
        violations_of(&document, CHAIN_STUDY).contains(&MechanicalViolationKind::UnverifiableFrame)
    );

    // A guide plane declared to face along the travel axis instead of across it.
    let mut document = transmission_document(limits, DRIVE_DEGREES);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateMechanicalCondition(MechanicalCondition::new(
                RACK_GUIDE,
                MechanicalConditionKind::JointAxisAlignment {
                    joint_id: RACK_JOINT,
                    interface: CARRIAGE_FLANK,
                    alignment: MechanicalAxisAlignment::Parallel,
                    tolerance_degrees: 1.0e-6,
                },
            )),
        ]))
        .unwrap();
    assert!(violations_of(&document, CHAIN_STUDY).iter().any(|kind| {
        matches!(
            kind,
            MechanicalViolationKind::AxisMisaligned {
                measured_degrees, ..
            } if (measured_degrees - 90.0).abs() <= 1.0e-9
        )
    }));
}

#[test]
fn a_document_without_conditions_does_not_pretend_to_be_validated() {
    let document = geometry_document();
    assert_eq!(
        preview_mechanical_contract(&document.current(), STUDY, 4),
        Err(MechanicalContractError::NoConditions)
    );
}
