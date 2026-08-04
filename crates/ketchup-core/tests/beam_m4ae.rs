use ketchup_core::beam_m4ae::*;
use ketchup_core::prismatic::*;

#[test]
fn tolerance_aabb_obb_sat_and_touching_are_conservative() {
    assert_eq!(
        TolerancePolicy::new(0.0),
        Err(PrismaticError::InvalidTolerance)
    );
    let t = TolerancePolicy::default();
    assert_eq!(t.id(), "ketchup.prismatic-tolerance.v1");
    let a = Aabb::bounded_volume([0.0, 0.0, 0.0], [2.0, 2.0, 2.0]).unwrap();
    let touching = Aabb::bounded_volume([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]).unwrap();
    assert_eq!(
        collide_axis_aligned_prisms(a, touching, t)
            .unwrap()
            .relation,
        CollisionRelation::Intersecting
    );
    let separated = Aabb::bounded_volume([3.0, 0.0, 0.0], [4.0, 1.0, 1.0]).unwrap();
    assert_eq!(
        collide_axis_aligned_prisms(a, separated, t)
            .unwrap()
            .relation,
        CollisionRelation::Separated
    );
    let c = std::f64::consts::FRAC_1_SQRT_2;
    let rotated = Obb::new(
        [1.0, 1.0, 0.0],
        [[c, c, 0.0], [-c, c, 0.0], [0.0, 0.0, 1.0]],
        [1.0, 0.5, 0.5],
    )
    .unwrap();
    assert_eq!(
        obb_sat(&Obb::from_aabb(a).unwrap(), &rotated, t).unwrap(),
        CollisionRelation::Intersecting
    );
    let far = Obb::new([10.0, 0.0, 0.0], rotated.axes(), rotated.half_extents()).unwrap();
    assert_eq!(
        obb_sat(&Obb::from_aabb(a).unwrap(), &far, t).unwrap(),
        CollisionRelation::Separated
    );
}

#[test]
fn validator_emits_all_four_joint_outcomes() {
    let w = BeamWorkspace::load().unwrap();
    let s = w.snapshot();
    let pieces = w.slice().pieces.clone();
    let body = &pieces[0];
    let proxy = &pieces[1];
    let joint = s.joint(JointId(1)).unwrap();
    let t = TolerancePolicy::default();
    assert_eq!(
        validate_joint_overlap(body.bounds, proxy.bounds, Some(joint), t).unwrap(),
        Some(JointValidationOutcome::OverlapInsideDeclaredJointOk)
    );
    let tiny = CanonicalJoint::new(
        JointId(99),
        joint.participant_a().clone(),
        joint.participant_b().clone(),
        Aabb::bounded_volume([210.0, 0.0, 400.0], [220.0, 200.0, 420.0]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        validate_joint_overlap(body.bounds, proxy.bounds, Some(&tiny), t).unwrap(),
        Some(JointValidationOutcome::OverlapOutsideDeclaredJointError)
    );
    assert_eq!(
        validate_joint_overlap(body.bounds, proxy.bounds, None, t).unwrap(),
        Some(JointValidationOutcome::OverlapWithoutJointError)
    );
    let far = Aabb::bounded_volume([9000.0, 0.0, 400.0], [9160.0, 200.0, 440.0]).unwrap();
    assert_eq!(
        validate_joint_overlap(body.bounds, far, Some(joint), t).unwrap(),
        Some(JointValidationOutcome::DeclaredJointWithEmptyIntersectionError)
    );
}

#[test]
fn exact_fixture_change_bom_containment_and_slot_health() {
    let mut w = BeamWorkspace::load().unwrap();
    let expected = [
        (210., 370., 290.),
        (785., 945., 865.),
        (1360., 1520., 1440.),
        (1935., 2095., 2015.),
        (2510., 2670., 2590.),
        (3085., 3245., 3165.),
        (3660., 3820., 3740.),
        (4228., 4388., 4308.),
        (4796., 4956., 4876.),
        (5364., 5524., 5444.),
        (5932., 6092., 6012.),
        (6500., 6660., 6580.),
    ];
    assert_eq!(
        w.slice()
            .positions
            .iter()
            .map(|p| (p.start_mm, p.end_mm, p.centre_mm))
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(6660.0 + TERMINAL_GAP_MM + END_RESIDUAL_MM, BEAM_TOTAL_MM);
    assert_eq!(
        w.slice()
            .bom
            .rows
            .iter()
            .map(|r| (r.quantity, r.length_mm))
            .collect::<Vec<_>>(),
        vec![(1, 7260.0), (12, 160.0)]
    );
    assert_eq!(w.slice().validation, BeamValidationVerdict::Green);
    for (piece, outcome) in w
        .slice()
        .pieces
        .iter()
        .skip(1)
        .zip(&w.slice().joint_outcomes)
    {
        assert!(piece.identity.slot_path.segments().len() == 2);
        assert!(outcome.is_ok());
    }
    let ids = w
        .slice()
        .pieces
        .iter()
        .map(|p| p.identity.clone())
        .collect::<Vec<_>>();
    let bom_rev = w.slice().bom.generated_for_revision;
    w.set_zone1_gap_mm(420.0).unwrap();
    let change = w.last_change().unwrap();
    assert_eq!(
        change.recomputed_nodes,
        BTreeSet::from([ZONE1_GAP_NODE, BEAM_RULE_NODE])
    );
    assert!(!change.recomputed_nodes.contains(&UNRELATED_NODE));
    assert!(change.bom_regenerated);
    assert_ne!(w.slice().bom.generated_for_revision, bom_rev);
    assert_eq!(
        w.slice()
            .pieces
            .iter()
            .map(|p| p.identity.clone())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(w.slice().validation, BeamValidationVerdict::Green);
    w.set_zone1_count(3).unwrap();
    assert_eq!(
        w.snapshot()
            .override_by_id(GROOVE_OVERRIDE_ID)
            .unwrap()
            .health,
        ketchup_core::graph::SlotResolution::Lost { segment_index: 1 }
    );
    let mut w = BeamWorkspace::load().unwrap();
    w.duplicate_override_key().unwrap();
    assert_eq!(
        w.snapshot()
            .override_by_id(GROOVE_OVERRIDE_ID)
            .unwrap()
            .health,
        ketchup_core::graph::SlotResolution::Ambiguous { segment_index: 1 }
    );
}

use std::collections::BTreeSet;
