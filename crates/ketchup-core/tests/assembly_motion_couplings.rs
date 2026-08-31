use ketchup_core::assembly_joint::{
    AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind, AssemblyJointLimits,
    AssemblyKinematicSolveError, AssemblyKinematicSolveStatus, AssemblyMotionDriver,
    AssemblyMotionStudy, AssemblyMotionStudyId, solve_assembly_joint_kinematics_with_drivers,
    solve_assembly_motion_study,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore, OccurrenceId,
    Transform,
};
use ketchup_core::mechanical_coupling::{
    AssemblyMotionCoupling, AssemblyMotionCouplingId, AssemblyMotionDirection,
    AssemblyTransmissionKind, GearMeshKind, ScrewHandedness,
};
use ketchup_core::{persistence, state_view::encode_semantic_state};

const DEFINITION: DefinitionId = DefinitionId(1);
const ROOT: OccurrenceId = OccurrenceId(10);
const GEAR_INPUT: AssemblyJointId = AssemblyJointId(101);
const GEAR_OUTPUT: AssemblyJointId = AssemblyJointId(102);
const BELT_OUTPUT: AssemblyJointId = AssemblyJointId(103);
const CHAIN_OUTPUT: AssemblyJointId = AssemblyJointId(104);
const RACK_OUTPUT: AssemblyJointId = AssemblyJointId(105);
const SCREW_OUTPUT: AssemblyJointId = AssemblyJointId(106);
const STUDY: AssemblyMotionStudyId = AssemblyMotionStudyId(300);

fn revolute() -> AssemblyJointKind {
    AssemblyJointKind::Revolute {
        axis: AssemblyJointAxis::new([0.0, 0.0, 1.0], [0.0, 0.0, 0.0]),
        limits: Some(AssemblyJointLimits::new(-360.0, 360.0)),
        position_degrees: 0.0,
    }
}

fn prismatic(limits: AssemblyJointLimits) -> AssemblyJointKind {
    AssemblyJointKind::Prismatic {
        axis: AssemblyJointAxis::new([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        limits: Some(limits),
        position_mm: 0.0,
    }
}

fn occurrence(id: u64) -> CanonicalCommand {
    CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(id),
        definition_id: DEFINITION,
        name: format!("part-{id}"),
        transform: Transform::from_translation(id as f64 * 10.0, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    }
}

fn coupling(
    id: u64,
    input: AssemblyJointId,
    output: AssemblyJointId,
    transmission: AssemblyTransmissionKind,
) -> CanonicalCommand {
    CanonicalCommand::CreateAssemblyMotionCoupling(AssemblyMotionCoupling::new(
        AssemblyMotionCouplingId(id),
        input,
        output,
        0.0,
        0.0,
        transmission,
    ))
}

fn transmission_document(rack_limits: AssemblyJointLimits) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Transmission chain".into(),
            },
            occurrence(ROOT.0),
            occurrence(11),
            occurrence(12),
            occurrence(13),
            occurrence(14),
            occurrence(15),
            occurrence(16),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                GEAR_INPUT,
                ROOT,
                OccurrenceId(11),
                revolute(),
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                GEAR_OUTPUT,
                ROOT,
                OccurrenceId(12),
                revolute(),
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                BELT_OUTPUT,
                ROOT,
                OccurrenceId(13),
                revolute(),
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                CHAIN_OUTPUT,
                ROOT,
                OccurrenceId(14),
                revolute(),
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                RACK_OUTPUT,
                ROOT,
                OccurrenceId(15),
                prismatic(rack_limits),
            )),
            CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
                SCREW_OUTPUT,
                ROOT,
                OccurrenceId(16),
                prismatic(AssemblyJointLimits::new(-100.0, 100.0)),
            )),
            coupling(
                201,
                GEAR_INPUT,
                GEAR_OUTPUT,
                AssemblyTransmissionKind::GearPair {
                    input_teeth: 20,
                    output_teeth: 40,
                    mesh: GearMeshKind::External,
                },
            ),
            coupling(
                202,
                GEAR_OUTPUT,
                BELT_OUTPUT,
                AssemblyTransmissionKind::Belt {
                    input_pitch_diameter_mm: 40.0,
                    output_pitch_diameter_mm: 20.0,
                    crossed: false,
                },
            ),
            coupling(
                203,
                BELT_OUTPUT,
                CHAIN_OUTPUT,
                AssemblyTransmissionKind::Chain {
                    input_sprocket_teeth: 15,
                    output_sprocket_teeth: 30,
                },
            ),
            coupling(
                204,
                CHAIN_OUTPUT,
                RACK_OUTPUT,
                AssemblyTransmissionKind::RackAndPinion {
                    pinion_pitch_diameter_mm: 20.0,
                    direction: AssemblyMotionDirection::Same,
                },
            ),
            coupling(
                205,
                CHAIN_OUTPUT,
                SCREW_OUTPUT,
                AssemblyTransmissionKind::LeadScrew {
                    lead_mm_per_revolution: 8.0,
                    handedness: ScrewHandedness::Right,
                },
            ),
        ]))
        .unwrap();
    document
}

fn position(positions: &[(AssemblyJointId, f64)], joint_id: AssemblyJointId) -> f64 {
    positions
        .iter()
        .find_map(|(id, value)| (*id == joint_id).then_some(*value))
        .unwrap()
}

#[test]
fn one_driver_propagates_through_gears_belt_chain_rack_and_screw_and_round_trips() {
    let mut document = transmission_document(AssemblyJointLimits::new(-100.0, 100.0));
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionStudy(AssemblyMotionStudy::new(
                STUDY,
                "motor to linear outputs",
                vec![AssemblyMotionDriver::new(GEAR_INPUT, 180.0)],
            )),
        ]))
        .unwrap();

    let solution = solve_assembly_motion_study(&document.current(), STUDY).unwrap();
    assert_eq!(
        solution.status(),
        AssemblyKinematicSolveStatus::FullyConstrained
    );
    assert_eq!(solution.remaining_dof(), 0);
    let positions = solution.driven_joint_positions();
    assert_eq!(position(positions, GEAR_INPUT), 180.0);
    assert_eq!(position(positions, GEAR_OUTPUT), -90.0);
    assert_eq!(position(positions, BELT_OUTPUT), -180.0);
    assert_eq!(position(positions, CHAIN_OUTPUT), -90.0);
    assert!((position(positions, RACK_OUTPUT) + 5.0 * std::f64::consts::PI).abs() <= 1.0e-10);
    assert!((position(positions, SCREW_OUTPUT) + 2.0).abs() <= 1.0e-10);

    let publication = solution.publication_batch(&document.current()).unwrap();
    document.apply_batch(&publication).unwrap();
    let committed = document.current();
    let state = encode_semantic_state(&committed);
    for kind in [
        "gear_pair",
        "belt",
        "chain",
        "rack_and_pinion",
        "lead_screw",
    ] {
        assert!(state.complete_v1().contains(&format!("kind:{kind}")));
    }

    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        committed.canonical_digest()
    );
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        state.complete_v1()
    );
    assert_eq!(reopened.snapshot().assembly_motion_couplings().count(), 5);
}

#[test]
fn invalid_joint_types_and_inconsistent_cycles_fail_atomically() {
    let mut document = transmission_document(AssemblyJointLimits::new(-100.0, 100.0));
    let before = document.current().canonical_digest();
    let invalid_type = AssemblyMotionCoupling::new(
        AssemblyMotionCouplingId(250),
        GEAR_INPUT,
        RACK_OUTPUT,
        0.0,
        0.0,
        AssemblyTransmissionKind::GearPair {
            input_teeth: 10,
            output_teeth: 10,
            mesh: GearMeshKind::External,
        },
    );
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionCoupling(invalid_type)
        ])),
        Err(CanonicalError::InvalidAssemblyMotionCoupling(
            AssemblyMotionCouplingId(250)
        ))
    ));
    assert_eq!(document.current().canonical_digest(), before);

    let inconsistent_cycle = AssemblyMotionCoupling::new(
        AssemblyMotionCouplingId(251),
        CHAIN_OUTPUT,
        GEAR_INPUT,
        0.0,
        0.0,
        AssemblyTransmissionKind::GearPair {
            input_teeth: 10,
            output_teeth: 10,
            mesh: GearMeshKind::External,
        },
    );
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMotionCoupling(inconsistent_cycle)
        ])),
        Err(CanonicalError::InvalidAssemblyMotionCoupling(_))
    ));
    assert_eq!(document.current().canonical_digest(), before);
}

#[test]
fn conflicting_drivers_and_derived_limit_violations_are_rejected() {
    let document = transmission_document(AssemblyJointLimits::new(-100.0, 100.0));
    assert_eq!(
        solve_assembly_joint_kinematics_with_drivers(
            &document.current(),
            &[
                AssemblyMotionDriver::new(GEAR_INPUT, 180.0),
                AssemblyMotionDriver::new(GEAR_OUTPUT, 0.0),
            ],
        ),
        Err(AssemblyKinematicSolveError::CouplingConflict(
            AssemblyMotionCouplingId(201)
        ))
    );

    let limited = transmission_document(AssemblyJointLimits::new(-5.0, 5.0));
    assert_eq!(
        solve_assembly_joint_kinematics_with_drivers(
            &limited.current(),
            &[AssemblyMotionDriver::new(GEAR_INPUT, 180.0)],
        ),
        Err(AssemblyKinematicSolveError::CoupledPositionOutsideLimits(
            RACK_OUTPUT
        ))
    );
}
