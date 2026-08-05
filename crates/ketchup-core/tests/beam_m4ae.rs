use ketchup_core::beam_m4ae::*;
use ketchup_core::fabrication::PieceDimensions;
use ketchup_core::prismatic::*;
use ketchup_core::validation::ValidationState;

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

    let euclidean = TolerancePolicy::new(1.0).unwrap();
    let participant_a = joint.participant_a().clone();
    let participant_b = joint.participant_b().clone();
    let bounded = CanonicalJoint::new(
        JointId(100),
        participant_a,
        participant_b,
        Aabb::bounded_volume([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]).unwrap(),
    )
    .unwrap();
    let solid = Aabb::bounded_volume([0.0, 0.0, 0.0], [2.0, 2.0, 1.0]).unwrap();
    let one_axis = Aabb::bounded_volume([0.0, 0.0, 0.0], [1.9, 1.0, 1.0]).unwrap();
    assert_eq!(
        validate_joint_overlap(solid, one_axis, Some(&bounded), euclidean).unwrap(),
        Some(JointValidationOutcome::OverlapInsideDeclaredJointOk)
    );
    let corner = Aabb::bounded_volume([0.0, 0.0, 0.0], [1.8, 1.8, 1.0]).unwrap();
    assert_eq!(
        validate_joint_overlap(solid, corner, Some(&bounded), euclidean).unwrap(),
        Some(JointValidationOutcome::OverlapOutsideDeclaredJointError)
    );
}

#[test]
fn joint_driven_half_lap_keeps_one_authority_and_all_four_geometry_verdicts() {
    let workspace = BeamWorkspace::load().unwrap();
    let snapshot = workspace.snapshot();
    let beam = &workspace.slice().exact_pieces[0];
    let crossing = &workspace.slice().exact_pieces[1];
    let half_lap = &workspace.slice().half_laps[0];
    let joint = snapshot.joint(half_lap.joint_id).unwrap();
    let tolerance = TolerancePolicy::default();

    assert_eq!(half_lap.participant_a, beam.identity);
    assert_eq!(half_lap.participant_b, crossing.identity);
    assert_eq!(beam.source_joints[0], half_lap.joint_id);
    assert_eq!(crossing.source_joints, [half_lap.joint_id]);
    assert_eq!(
        half_lap.participant_a_notch.volume() + half_lap.participant_b_notch.volume(),
        joint.volume().volume()
    );
    assert_eq!(
        half_lap.contact.extents(),
        [GROOVE_WIDTH_MM, BEAM_WIDTH_MM, 0.0]
    );
    assert_eq!(beam.geometry.components().len(), 14);
    assert_eq!(crossing.geometry.components().len(), 1);
    assert_eq!(
        validate_joint_geometry(&beam.geometry, &crossing.geometry, Some(joint), tolerance)
            .unwrap(),
        Some(JointValidationOutcome::OverlapInsideDeclaredJointOk)
    );

    let tiny = CanonicalJoint::new(
        JointId(99),
        beam.identity.clone(),
        crossing.identity.clone(),
        Aabb::bounded_volume(
            half_lap.contact.min(),
            [
                half_lap.contact.min()[0] + 10.0,
                half_lap.contact.max()[1],
                half_lap.contact.max()[2] + 1.0,
            ],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        validate_joint_geometry(&beam.geometry, &crossing.geometry, Some(&tiny), tolerance)
            .unwrap(),
        Some(JointValidationOutcome::OverlapOutsideDeclaredJointError)
    );

    let penetrating_crossing =
        ExactPrismaticBody::solid(workspace.slice().pieces[1].bounds).unwrap();
    assert_eq!(
        validate_joint_geometry(&beam.geometry, &penetrating_crossing, None, tolerance).unwrap(),
        Some(JointValidationOutcome::OverlapWithoutJointError)
    );
    let far = ExactPrismaticBody::solid(
        Aabb::bounded_volume([9000.0, 0.0, 380.0], [9160.0, 200.0, 420.0]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        validate_joint_geometry(&beam.geometry, &far, Some(joint), tolerance).unwrap(),
        Some(JointValidationOutcome::DeclaredJointWithEmptyIntersectionError)
    );

    let mut reversed_components = beam.geometry.components().to_vec();
    reversed_components.reverse();
    assert_eq!(
        ExactPrismaticBody::from_components(beam.geometry.stock(), reversed_components).unwrap(),
        beam.geometry
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
    assert_eq!(w.slice().validation_report.state, ValidationState::Passed);
    assert_eq!(w.slice().validation_report.evidence_counts.exact, 12);
    assert_eq!(w.slice().validation_report.evidence_counts.tolerant, 0);
    assert_eq!(w.slice().half_laps.len(), 12);
    assert_eq!(w.slice().exact_pieces.len(), 13);
    assert_eq!(w.slice().exact_pieces[0].geometry.components().len(), 14);
    assert!(
        w.slice()
            .exact_pieces
            .iter()
            .skip(1)
            .all(|piece| piece.geometry.components().len() == 1)
    );
    assert_eq!(
        w.slice().full_bom.evidence_counts,
        w.slice().validation_report.evidence_counts
    );
    assert!(w.slice().full_bom.envelope.is_current(&w.snapshot()));
    assert_eq!(w.slice().full_bom.rows.len(), 2);
    assert_eq!(
        w.slice()
            .full_bom
            .rows
            .iter()
            .map(|row| (
                row.stable_row_id.as_str(),
                row.material_key.as_str(),
                row.quantity,
                row.dimensions,
                row.validation_state,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "beam-a/body",
                "ketchup.material.timber.unspecified.v1",
                1,
                PieceDimensions {
                    length_mm: 7260.0,
                    width_mm: 200.0,
                    height_mm: 420.0,
                },
                ValidationState::Passed,
            ),
            (
                "beam-a/crossing-members",
                "ketchup.material.fixture-proxy.v1",
                12,
                PieceDimensions {
                    length_mm: 160.0,
                    width_mm: 200.0,
                    height_mm: 40.0,
                },
                ValidationState::Passed,
            ),
        ]
    );
    assert!(w.slice().dimension_sheet.envelope.is_current(&w.snapshot()));
    assert_eq!(
        w.slice().dimension_sheet.chains[0].grouped_labels,
        ["415 × 6", "408 × 5", "400"]
    );
    assert!(
        w.slice().dimension_sheet.chains[0]
            .segments
            .iter()
            .all(
                |segment| !segment.from.piece.slot_path.segments().is_empty()
                    && !segment.to.piece.slot_path.segments().is_empty()
            )
    );
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
    let full_bom_digest = w.slice().full_bom.envelope.result_digest.clone();
    let dimension_digest = w.slice().dimension_sheet.envelope.result_digest.clone();
    let bom_row_ids = w
        .slice()
        .full_bom
        .rows
        .iter()
        .map(|row| row.stable_row_id.clone())
        .collect::<Vec<_>>();
    let dimension_segment_ids = w.slice().dimension_sheet.chains[0]
        .segments
        .iter()
        .map(|segment| segment.stable_segment_id.clone())
        .collect::<Vec<_>>();
    w.set_zone1_gap_mm(420.0).unwrap();
    let change = w.last_change().unwrap();
    assert_eq!(
        change.recomputed_nodes,
        BTreeSet::from([ZONE1_GAP_NODE, BEAM_RULE_NODE])
    );
    assert!(!change.recomputed_nodes.contains(&UNRELATED_NODE));
    assert!(change.bom_regenerated);
    assert!(change.dimensions_regenerated);
    assert!(change.validator_ran);
    assert_ne!(w.slice().bom.generated_for_revision, bom_rev);
    assert_eq!(w.slice().full_bom.envelope.result_digest, full_bom_digest);
    assert_ne!(
        w.slice().dimension_sheet.envelope.result_digest,
        dimension_digest
    );
    assert_eq!(
        w.slice()
            .full_bom
            .rows
            .iter()
            .map(|row| row.stable_row_id.clone())
            .collect::<Vec<_>>(),
        bom_row_ids
    );
    assert_eq!(
        w.slice().dimension_sheet.chains[0]
            .segments
            .iter()
            .map(|segment| segment.stable_segment_id.clone())
            .collect::<Vec<_>>(),
        dimension_segment_ids
    );
    assert_eq!(
        w.slice().dimension_sheet.chains[0].grouped_labels,
        ["420 × 6", "408 × 5", "400"]
    );
    assert_eq!(
        w.slice()
            .pieces
            .iter()
            .map(|p| p.identity.clone())
            .collect::<Vec<_>>(),
        ids
    );
    assert_eq!(w.slice().validation, BeamValidationVerdict::Green);
    assert_eq!(w.slice().validation_report.evidence_counts.exact, 12);
    assert_eq!(w.slice().validation_report.evidence_counts.tolerant, 0);
    assert_eq!(w.slice().exact_pieces[0].geometry.components().len(), 14);
    w.set_zone1_count(3).unwrap();
    assert_eq!(
        w.snapshot()
            .override_by_id(GROOVE_OVERRIDE_ID)
            .unwrap()
            .health,
        ketchup_core::graph::SlotResolution::Lost { segment_index: 1 }
    );
    assert_eq!(
        w.slice().dimension_sheet.envelope.status,
        ketchup_core::fabrication::ProjectionStatus::Incomplete
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
