use ketchup_core::beam_m4ae::BeamWorkspace;
use ketchup_core::prismatic::{
    Aabb, CanonicalJoint, JointId, JointValidationOutcome, TolerancePolicy, validate_joint_overlap,
};
use ketchup_core::validation::*;

#[test]
fn host_neutral_contract_binds_inputs_and_required_failures_fail_closed() {
    let workspace = BeamWorkspace::load().unwrap();
    let snapshot = workspace.snapshot();
    let descriptor = prismatic_validator_descriptor();
    let policy = beam_validation_policy();
    let accepted = workspace
        .slice()
        .pieces
        .iter()
        .map(|piece| piece.identity.clone())
        .collect::<Vec<_>>();
    let invocation = ValidationInvocation::bind(
        &snapshot,
        &descriptor,
        &policy,
        accepted,
        b"host-neutral deterministic payload",
    );
    assert_eq!(invocation.protocol, VALIDATOR_PROTOCOL_V1);
    assert!(invocation.is_current(&snapshot));
    assert_eq!(invocation.input_digest.len(), 64);
    assert_eq!(descriptor.validation_class, ValidationClass::DeclaredJoint);
    assert!(descriptor.deterministic);
    assert_eq!(
        descriptor.read_scopes,
        [ReadScope::DerivedGeometry, ReadScope::DeclaredJoints]
    );

    let unavailable = ValidationReport::unavailable(
        invocation.clone(),
        EvidenceClass::Exact,
        "implementation missing",
    );
    assert_eq!(
        decide(&policy, &unavailable).state,
        ValidationState::Unavailable
    );
    assert!(decide(&policy, &unavailable).blocks_release);
    assert_ne!(unavailable.state, ValidationState::Passed);

    let not_evaluated =
        ValidationReport::not_evaluated(invocation, EvidenceClass::Exact, "trusted input absent");
    assert_eq!(
        decide(&policy, &not_evaluated).state,
        ValidationState::NotEvaluated
    );
    assert!(decide(&policy, &not_evaluated).blocks_release);
    assert_eq!(not_evaluated.diagnostics[0].policy_id, policy.policy_id);

    let mut optional = policy.clone();
    optional.requirement = PolicyRequirement::Optional;
    assert!(!decide(&optional, &unavailable).blocks_release);
}

#[test]
fn built_in_joint_validator_returns_stable_structured_diagnostics() {
    let workspace = BeamWorkspace::load().unwrap();
    let snapshot = workspace.snapshot();
    let body = &workspace.slice().pieces[0];
    let proxy = &workspace.slice().pieces[1];
    let cases = vec![PrismaticJointCase {
        left_identity: body.identity.clone(),
        right_identity: proxy.identity.clone(),
        left_evidence_class: EvidenceClass::Exact,
        right_evidence_class: EvidenceClass::Exact,
        left_bounds: body.bounds,
        right_bounds: proxy.bounds,
        declared_joint: None,
    }];
    let validator = BuiltinPrismaticValidator::default();
    let policy = beam_validation_policy();
    let tolerance = TolerancePolicy::default();
    let input = prismatic_input_bytes(&cases, tolerance);
    let invocation = ValidationInvocation::bind(
        &snapshot,
        validator.descriptor(),
        &policy,
        vec![body.identity.clone(), proxy.identity.clone()],
        &input,
    );
    let report = validator.invoke(ValidationExecution {
        snapshot: &snapshot,
        invocation,
        policy: &policy,
        input: &cases,
    });
    assert_eq!(report.state, ValidationState::Failed);
    assert_eq!(
        report.evidence_counts,
        EvidenceCounts {
            exact: 1,
            tolerant: 0
        }
    );
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].evidence_class, EvidenceClass::Exact);
    assert_eq!(
        report.diagnostics[0].code,
        "collision.undeclared-penetration"
    );
    assert_eq!(
        report.diagnostics[0].location.entity.as_ref(),
        Some(&proxy.identity)
    );
    assert_eq!(report.diagnostics[0].schema, DIAGNOSTIC_SCHEMA_V1);
    assert!(decide(&policy, &report).blocks_release);
}

#[test]
fn mixed_participants_take_the_weakest_evidence_class_with_required_metadata() {
    let workspace = BeamWorkspace::load().unwrap();
    let snapshot = workspace.snapshot();
    let body = &workspace.slice().pieces[0];
    let proxy = &workspace.slice().pieces[1];
    let tolerance = TolerancePolicy::default();
    let tolerant_participant = TolerantEvidence::new(
        tolerance.epsilon_mm(),
        "fixture.curved-envelope.v1",
        PermittedErrorDirection::FalsePositiveOnly,
    )
    .unwrap();
    let cases = vec![PrismaticJointCase {
        left_identity: body.identity.clone(),
        right_identity: proxy.identity.clone(),
        left_evidence_class: EvidenceClass::Exact,
        right_evidence_class: EvidenceClass::Tolerant(tolerant_participant),
        left_bounds: body.bounds,
        right_bounds: proxy.bounds,
        declared_joint: None,
    }];
    let validator = BuiltinPrismaticValidator::new(tolerance);
    let policy = beam_validation_policy();
    let input = prismatic_input_bytes(&cases, tolerance);
    let invocation = ValidationInvocation::bind(
        &snapshot,
        validator.descriptor(),
        &policy,
        vec![body.identity.clone(), proxy.identity.clone()],
        &input,
    );
    let report = validator.invoke(ValidationExecution {
        snapshot: &snapshot,
        invocation,
        policy: &policy,
        input: &cases,
    });

    assert_eq!(
        report.evidence_counts,
        EvidenceCounts {
            exact: 0,
            tolerant: 1
        }
    );
    let EvidenceClass::Tolerant(evidence) = &report.diagnostics[0].evidence_class else {
        panic!("a mixed pair must not be promoted to Exact");
    };
    assert_eq!(evidence.applied_threshold_mm(), tolerance.epsilon_mm());
    assert_eq!(
        evidence.method_identity,
        PRISMATIC_VALIDATOR_IMPLEMENTATION_V1
    );
    assert_eq!(
        evidence.permitted_error_direction,
        PermittedErrorDirection::FalsePositiveOnly
    );
}

#[test]
fn execution_rejects_tampering_stale_snapshots_limits_and_wrong_joint_participants() {
    let mut workspace = BeamWorkspace::load().unwrap();
    let old_snapshot = workspace.snapshot();
    let body = workspace.slice().pieces[0].clone();
    let proxy = workspace.slice().pieces[1].clone();
    let valid_joint = old_snapshot.joint(JointId(1)).unwrap().clone();
    let valid_cases = vec![PrismaticJointCase {
        left_identity: body.identity.clone(),
        right_identity: proxy.identity.clone(),
        left_evidence_class: EvidenceClass::Exact,
        right_evidence_class: EvidenceClass::Exact,
        left_bounds: body.bounds,
        right_bounds: proxy.bounds,
        declared_joint: Some(valid_joint.clone()),
    }];
    let policy = beam_validation_policy();
    let validator = BuiltinPrismaticValidator::default();
    let input = prismatic_input_bytes(&valid_cases, TolerancePolicy::default());
    let mut tampered = ValidationInvocation::bind(
        &old_snapshot,
        validator.descriptor(),
        &policy,
        vec![body.identity.clone(), proxy.identity.clone()],
        &input,
    );
    tampered.input_digest = "00".repeat(32);
    assert_eq!(
        validator
            .invoke(ValidationExecution {
                snapshot: &old_snapshot,
                invocation: tampered,
                policy: &policy,
                input: &valid_cases,
            })
            .state,
        ValidationState::NotEvaluated
    );

    let limited = BuiltinPrismaticValidator::with_limits(
        TolerancePolicy::default(),
        ResourceLimits {
            maximum_input_bytes: 16 * 1024 * 1024,
            maximum_work_units: 0,
        },
    );
    let limited_invocation = ValidationInvocation::bind(
        &old_snapshot,
        limited.descriptor(),
        &policy,
        vec![body.identity.clone(), proxy.identity.clone()],
        &input,
    );
    assert_eq!(
        limited
            .invoke(ValidationExecution {
                snapshot: &old_snapshot,
                invocation: limited_invocation,
                policy: &policy,
                input: &valid_cases,
            })
            .state,
        ValidationState::NotEvaluated
    );

    let byte_limited = BuiltinPrismaticValidator::with_limits(
        TolerancePolicy::default(),
        ResourceLimits {
            maximum_input_bytes: 0,
            maximum_work_units: 1,
        },
    );
    let byte_limited_invocation = ValidationInvocation::bind(
        &old_snapshot,
        byte_limited.descriptor(),
        &policy,
        vec![body.identity.clone(), proxy.identity.clone()],
        &input,
    );
    let byte_limited_report = byte_limited.invoke(ValidationExecution {
        snapshot: &old_snapshot,
        invocation: byte_limited_invocation,
        policy: &policy,
        input: &valid_cases,
    });
    assert_eq!(byte_limited_report.state, ValidationState::NotEvaluated);
    assert_eq!(
        byte_limited_report.diagnostics[0].evidence,
        "validator input exceeds its declared byte envelope"
    );

    let stale_invocation = ValidationInvocation::bind(
        &old_snapshot,
        validator.descriptor(),
        &policy,
        vec![body.identity.clone(), proxy.identity.clone()],
        &input,
    );
    workspace.set_zone1_gap_mm(420.0).unwrap();
    assert_eq!(
        validator
            .invoke(ValidationExecution {
                snapshot: &workspace.snapshot(),
                invocation: stale_invocation,
                policy: &policy,
                input: &valid_cases,
            })
            .state,
        ValidationState::NotEvaluated
    );

    let wrong_joint = CanonicalJoint::new(
        JointId(900),
        body.identity.clone(),
        workspace.slice().pieces[2].identity.clone(),
        valid_joint.volume(),
    )
    .unwrap();
    let wrong_cases = vec![PrismaticJointCase {
        left_identity: body.identity.clone(),
        right_identity: proxy.identity.clone(),
        left_evidence_class: EvidenceClass::Exact,
        right_evidence_class: EvidenceClass::Exact,
        left_bounds: body.bounds,
        right_bounds: proxy.bounds,
        declared_joint: Some(wrong_joint),
    }];
    let wrong_input = prismatic_input_bytes(&wrong_cases, TolerancePolicy::default());
    let wrong_invocation = ValidationInvocation::bind(
        &old_snapshot,
        validator.descriptor(),
        &policy,
        vec![body.identity, proxy.identity],
        &wrong_input,
    );
    let report = validator.invoke(ValidationExecution {
        snapshot: &old_snapshot,
        invocation: wrong_invocation,
        policy: &policy,
        input: &wrong_cases,
    });
    assert_eq!(report.state, ValidationState::Failed);
    assert_eq!(report.diagnostics[0].code, "joint.participant-mismatch");
}

#[test]
fn furnigen_prismatic_v1_preserves_hard_failures_without_false_negatives() {
    let fixture = include_str!("fixtures/furnigen/prismatic-v1.tsv");
    assert!(fixture.contains("source_commit=45dca2fbee382e5fa40cecd176d2361e6c53fa7f"));
    let mut hard_failures = 0;
    let mut detected_hard_failures = 0;
    let mut observed = Vec::new();

    for line in fixture
        .lines()
        .filter(|line| !line.starts_with('#'))
        .skip(1)
    {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 10, "invalid corpus row: {line}");
        let id = fields[0];
        let kind = fields[1];
        let left_min = parse_xyz(fields[2]);
        let left_max = parse_xyz(fields[3]);
        let right_min = parse_xyz(fields[4]);
        let right_max = parse_xyz(fields[5]);
        let tolerance = TolerancePolicy::new(fields[6].parse::<f64>().unwrap()).unwrap();
        let expected = fields[7];
        let actual = match kind {
            "collision" => {
                let left = Aabb::bounded_volume(left_min, left_max).unwrap();
                let right = Aabb::bounded_volume(right_min, right_max).unwrap();
                match validate_joint_overlap(left, right, None, tolerance).unwrap() {
                    Some(JointValidationOutcome::OverlapWithoutJointError) => "failed",
                    None => "passed",
                    other => panic!("unexpected collision result for {id}: {other:?}"),
                }
            }
            "envelope" => {
                let subject = Aabb::bounded_volume(left_min, left_max).unwrap();
                let envelope = Aabb::bounded_volume(right_min, right_max).unwrap();
                if subject
                    .extents()
                    .into_iter()
                    .zip(envelope.extents())
                    .all(|(actual, limit)| actual <= limit + tolerance.epsilon_mm())
                {
                    "passed"
                } else {
                    "failed"
                }
            }
            "advisory" => "not-evaluated",
            other => panic!("unknown corpus kind {other}"),
        };
        if expected == "failed" {
            hard_failures += 1;
            if actual == "failed" {
                detected_hard_failures += 1;
            }
        }
        observed.push((id, actual));
        assert_eq!(actual, expected, "corpus classification mismatch for {id}");
    }

    assert_eq!(hard_failures, 3);
    assert_eq!(detected_hard_failures, hard_failures);
    assert_eq!(observed.len(), 8);
}

fn parse_xyz(value: &str) -> [f64; 3] {
    let values = value
        .split(',')
        .map(|part| part.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    values
        .try_into()
        .expect("fixture XYZ must contain three values")
}
