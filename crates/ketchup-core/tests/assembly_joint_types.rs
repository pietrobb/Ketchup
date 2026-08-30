use ketchup_core::assembly_joint::{
    ASSEMBLY_JOINT_SCHEMA_V1, ASSEMBLY_MOTION_STUDY_SCHEMA_V1, AssemblyJoint, AssemblyJointAxis,
    AssemblyJointId, AssemblyJointKind, AssemblyJointLimits, AssemblyMotionDriver,
    AssemblyMotionStudy, AssemblyMotionStudyId,
};
use ketchup_core::document::OccurrenceId;

fn axis_z() -> AssemblyJointAxis {
    AssemblyJointAxis::new([0.0, 0.0, 1.0], [0.0, 0.0, 0.0])
}

fn revolute(limits: Option<AssemblyJointLimits>, position_degrees: f64) -> AssemblyJointKind {
    AssemblyJointKind::Revolute {
        axis: axis_z(),
        limits,
        position_degrees,
    }
}

fn prismatic(limits: Option<AssemblyJointLimits>, position_mm: f64) -> AssemblyJointKind {
    AssemblyJointKind::Prismatic {
        axis: axis_z(),
        limits,
        position_mm,
    }
}

fn driver(joint: u64, position: f64) -> AssemblyMotionDriver {
    AssemblyMotionDriver::new(AssemblyJointId(joint), position)
}

#[test]
fn fixed_joint_is_valid_and_has_no_axis_limits_or_position() {
    let kind = AssemblyJointKind::Fixed;
    assert!(kind.is_valid());
    assert_eq!(kind.axis(), None);
    assert_eq!(kind.limits(), None);
    assert_eq!(kind.position(), None);
    assert_eq!(kind.with_position(1.0), None);
    assert_eq!(
        kind.with_limits(Some(AssemblyJointLimits::new(0.0, 1.0))),
        None
    );
}

#[test]
fn joint_wrapper_exposes_schema_ids_and_kind() {
    let kind = revolute(None, 15.0);
    let joint = AssemblyJoint::new(AssemblyJointId(7), OccurrenceId(1), OccurrenceId(2), kind);
    assert_eq!(joint.schema(), ASSEMBLY_JOINT_SCHEMA_V1);
    assert_eq!(joint.id(), AssemblyJointId(7));
    assert_eq!(joint.parent_occurrence_id(), OccurrenceId(1));
    assert_eq!(joint.child_occurrence_id(), OccurrenceId(2));
    assert_eq!(joint.kind(), kind);
    assert!(joint.has_valid_shape());
}

#[test]
fn joint_shape_rejects_reserved_id_self_joint_and_invalid_kind() {
    assert!(
        !AssemblyJoint::new(
            AssemblyJointId(0),
            OccurrenceId(1),
            OccurrenceId(2),
            AssemblyJointKind::Fixed,
        )
        .has_valid_shape()
    );
    for (parent, child) in [
        (OccurrenceId(0), OccurrenceId(2)),
        (OccurrenceId(2), OccurrenceId(0)),
    ] {
        assert!(
            !AssemblyJoint::new(AssemblyJointId(1), parent, child, AssemblyJointKind::Fixed,)
                .has_valid_shape()
        );
    }
    assert!(
        !AssemblyJoint::new(
            AssemblyJointId(1),
            OccurrenceId(2),
            OccurrenceId(2),
            AssemblyJointKind::Fixed,
        )
        .has_valid_shape()
    );
    assert!(
        !AssemblyJoint::new(
            AssemblyJointId(1),
            OccurrenceId(1),
            OccurrenceId(2),
            revolute(None, f64::NAN),
        )
        .has_valid_shape()
    );
}

#[test]
fn arbitrary_axes_and_pivots_are_valid() {
    let cases = [
        AssemblyJointAxis::new([1.0, 2.0, -3.0], [10.5, -20.25, 300.0]),
        AssemblyJointAxis::new([0.0, 1e-7, 0.0], [0.0, 0.0, 0.0]),
        AssemblyJointAxis::new([-1.0, -1.0, -1.0], [1_000_000.0, -1_000_000.0, 0.0]),
    ];
    for axis in cases {
        assert!(axis.is_valid(), "{axis:?}");
    }
    let axis = cases[0];
    let expected_length = 14.0_f64.sqrt();
    assert_eq!(
        axis.direction_in_parent(),
        [
            1.0 / expected_length,
            2.0 / expected_length,
            -3.0 / expected_length
        ]
    );
    assert_eq!(axis.pivot_in_parent_mm(), [10.5, -20.25, 300.0]);
}

#[test]
fn zero_nan_and_oversized_axes_are_invalid() {
    let bad = [
        AssemblyJointAxis::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
        AssemblyJointAxis::new([f64::NAN, 0.0, 1.0], [0.0, 0.0, 0.0]),
        AssemblyJointAxis::new([0.0, 0.0, 1.0], [f64::NAN, 0.0, 0.0]),
        AssemblyJointAxis::new([f64::INFINITY, 0.0, 0.0], [0.0, 0.0, 0.0]),
        AssemblyJointAxis::new([1.0e-13, 0.0, 0.0], [0.0, 0.0, 0.0]),
        AssemblyJointAxis::new([0.0, 0.0, 1.0], [0.0, 1_000_000.1, 0.0]),
    ];
    for axis in bad {
        assert!(!axis.is_valid(), "{axis:?}");
    }
}

#[test]
fn invalid_axis_invalidates_revolute_and_prismatic() {
    let zero_axis = AssemblyJointAxis::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
    assert!(
        !AssemblyJointKind::Revolute {
            axis: zero_axis,
            limits: None,
            position_degrees: 0.0,
        }
        .is_valid()
    );
    assert!(
        !AssemblyJointKind::Prismatic {
            axis: zero_axis,
            limits: None,
            position_mm: 0.0,
        }
        .is_valid()
    );
}

#[test]
fn valid_limits_accept_in_range_positions() {
    let limits = AssemblyJointLimits::new(-90.0, 90.0);
    assert_eq!(limits.min(), -90.0);
    assert_eq!(limits.max(), 90.0);
    assert!(limits.contains(-90.0));
    assert!(limits.contains(0.0));
    assert!(limits.contains(90.0));
    assert!(!limits.contains(90.000001));
    assert!(!limits.contains(f64::NAN));

    assert!(revolute(Some(limits), 45.0).is_valid());
    assert!(prismatic(Some(AssemblyJointLimits::new(0.0, 100.0)), 100.0).is_valid());
}

#[test]
fn invalid_limits_reject_joints() {
    // min > max
    assert!(!revolute(Some(AssemblyJointLimits::new(10.0, -10.0)), 0.0).is_valid());
    // non-finite bounds
    assert!(!revolute(Some(AssemblyJointLimits::new(f64::NAN, 1.0)), 0.5).is_valid());
    assert!(!prismatic(Some(AssemblyJointLimits::new(0.0, f64::INFINITY)), 1.0).is_valid());
    // bounds outside domain caps
    assert!(!revolute(Some(AssemblyJointLimits::new(-360_000.1, 0.0)), 0.0).is_valid());
    assert!(!prismatic(Some(AssemblyJointLimits::new(0.0, 1_000_000.1)), 1.0).is_valid());
    // position outside limits
    assert!(!revolute(Some(AssemblyJointLimits::new(-10.0, 10.0)), 10.5).is_valid());
    assert!(!prismatic(Some(AssemblyJointLimits::new(0.0, 5.0)), -0.1).is_valid());
}

#[test]
fn positions_must_be_finite_and_within_domain_caps() {
    assert!(revolute(None, 360_000.0).is_valid());
    assert!(!revolute(None, 360_000.1).is_valid());
    assert!(!revolute(None, f64::NAN).is_valid());
    assert!(prismatic(None, -1_000_000.0).is_valid());
    assert!(!prismatic(None, 1_000_000.1).is_valid());
    assert!(!prismatic(None, f64::INFINITY).is_valid());
}

#[test]
fn with_position_updates_only_position() {
    let limits = Some(AssemblyJointLimits::new(-180.0, 180.0));
    let updated = revolute(limits, 10.0)
        .with_position(-45.0)
        .expect("revolute");
    assert_eq!(updated.position(), Some(-45.0));
    assert_eq!(updated.axis(), Some(axis_z()));
    assert_eq!(updated.limits(), limits);
    assert!(updated.is_valid());

    let updated = prismatic(None, 1.0)
        .with_position(250.0)
        .expect("prismatic");
    assert_eq!(updated.position(), Some(250.0));
    // Out-of-limit updates are representable but invalid.
    let out = revolute(limits, 0.0)
        .with_position(181.0)
        .expect("revolute");
    assert!(!out.is_valid());
}

#[test]
fn with_limits_replaces_or_clears_limits() {
    let replacement = Some(AssemblyJointLimits::new(0.0, 30.0));
    let updated = revolute(None, 15.0)
        .with_limits(replacement)
        .expect("revolute");
    assert_eq!(updated.limits(), replacement);
    assert_eq!(updated.position(), Some(15.0));
    assert!(updated.is_valid());

    let cleared = prismatic(replacement, 15.0)
        .with_limits(None)
        .expect("prismatic");
    assert_eq!(cleared.limits(), None);
    assert!(cleared.is_valid());
}

#[test]
fn motion_study_sorts_drivers_deterministically() {
    let study = AssemblyMotionStudy::new(
        AssemblyMotionStudyId(1),
        "sweep",
        vec![driver(3, 30.0), driver(1, 10.0), driver(2, 20.0)],
    );
    assert_eq!(study.schema(), ASSEMBLY_MOTION_STUDY_SCHEMA_V1);
    assert_eq!(study.id(), AssemblyMotionStudyId(1));
    assert_eq!(study.name(), "sweep");
    assert_eq!(
        study.drivers(),
        &[driver(1, 10.0), driver(2, 20.0), driver(3, 30.0)]
    );
    assert!(study.has_valid_shape());

    // Same drivers in any input order produce an identical study.
    let reordered = AssemblyMotionStudy::new(
        AssemblyMotionStudyId(1),
        "sweep",
        vec![driver(2, 20.0), driver(3, 30.0), driver(1, 10.0)],
    );
    assert_eq!(study, reordered);
}

#[test]
fn duplicate_driver_joint_ids_invalidate_study() {
    let study = AssemblyMotionStudy::new(
        AssemblyMotionStudyId(1),
        "dupes",
        vec![driver(2, 1.0), driver(2, 2.0), driver(1, 0.0)],
    );
    assert!(!study.has_valid_shape());
}

#[test]
fn invalid_studies_are_rejected() {
    // Zero study id.
    assert!(
        !AssemblyMotionStudy::new(AssemblyMotionStudyId(0), "s", vec![driver(1, 0.0)])
            .has_valid_shape()
    );
    // Blank name.
    assert!(
        !AssemblyMotionStudy::new(AssemblyMotionStudyId(1), "  ", vec![driver(1, 0.0)])
            .has_valid_shape()
    );
    // No drivers.
    assert!(!AssemblyMotionStudy::new(AssemblyMotionStudyId(1), "s", vec![]).has_valid_shape());
    // Zero joint id in driver.
    assert!(
        !AssemblyMotionStudy::new(AssemblyMotionStudyId(1), "s", vec![driver(0, 0.0)])
            .has_valid_shape()
    );
    // Non-finite driver position.
    assert!(
        !AssemblyMotionStudy::new(AssemblyMotionStudyId(1), "s", vec![driver(1, f64::NAN)])
            .has_valid_shape()
    );
}

#[test]
fn motion_driver_accessors_round_trip() {
    let d = driver(9, -12.5);
    assert_eq!(d.joint_id(), AssemblyJointId(9));
    assert_eq!(d.position(), -12.5);
}
