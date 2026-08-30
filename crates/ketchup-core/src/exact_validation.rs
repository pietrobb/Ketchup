#![forbid(unsafe_code)]

use crate::document::{
    DefinitionId, FeatureId, FeatureKind, InstancePath, InstancePathStep, Snapshot, Transform,
};
use crate::exact_brep_graph::ExactBRepGraph;
use crate::exact_product::{
    BodyResultIdentity, BodySubshapeRef, EXACT_RECTANGLE_EVALUATOR_V1, ExactBodyPackage,
    ExactFaceRole, ExactRenderPackage, ExactResultKey, ExactResultRegistry,
};
use crate::graph::{DerivedIdentity, SlotResolution, sha256_hex};
use crate::prismatic::{
    Aabb, CanonicalJoint, JointValidationOutcome, TolerancePolicy, validate_joint_overlap,
};
use crate::validation::{
    DIAGNOSTIC_SCHEMA_V1, DiagnosticLocation, DiagnosticSeverity, EvidenceClass, EvidenceCounts,
    HostNeutralValidator, PermittedErrorDirection, PolicyRequirement, PolicySeverity, ReadScope,
    ResourceLimits, TolerantEvidence, ValidationClass, ValidationDiagnostic, ValidationExecution,
    ValidationInvocation, ValidationPolicyRef, ValidationReport, ValidationState,
    ValidatorDescriptor,
};
use std::collections::BTreeMap;

pub const EXACT_VALIDATOR_CONTRACT_V1: &str = "ketchup.validator.exact-bodies.v1";
pub const EXACT_VALIDATOR_IMPLEMENTATION_V1: &str = "ketchup.builtin.exact-bodies.aabb-cpu-f64.v1";
pub const EXACT_VALIDATOR_INPUT_V1: &str = "ketchup.exact-body-validation-input.v1";
pub const EXACT_VALIDATION_POLICY_V1: &str = "ketchup.policy.exact-body-validation.v1";
pub const EXACT_AABB_ENVELOPE_METHOD_V1: &str =
    "ketchup.method.exact-body-aabb-envelope.cpu-f64.v1";
pub const GENERAL_BODY_VALIDATOR_CONTRACT_V1: &str = "ketchup.validator.general-bodies.v1";
pub const GENERAL_BODY_VALIDATOR_IMPLEMENTATION_V1: &str =
    "ketchup.builtin.general-bodies.obb-sat-cpu-f64.v1";
pub const GENERAL_BODY_VALIDATOR_INPUT_V1: &str = "ketchup.general-body-validation-input.v1";
pub const GENERAL_BODY_VALIDATION_POLICY_V1: &str = "ketchup.policy.general-body-validation.v1";
pub const GENERAL_BODY_AABB_METHOD_V1: &str =
    "ketchup.method.general-body-aabb-envelope.cpu-f64.v1";
pub const GENERAL_BODY_SOURCE_FRAME_METHOD_V1: &str =
    "ketchup.method.general-body-source-frame-extents.cpu-f64.v1";
pub const GENERAL_BODY_OBB_NARROW_PHASE_METHOD_V1: &str =
    "ketchup.method.general-body-obb-sat.cpu-f64.v1";
pub const GRAVITY_SUPPORT_VALIDATOR_CONTRACT_V1: &str = "ketchup.validator.gravity-support.v1";
pub const GRAVITY_SUPPORT_VALIDATOR_IMPLEMENTATION_V1: &str =
    "ketchup.builtin.gravity-support.obb-sat-cpu-f64.v1";
pub const GRAVITY_SUPPORT_VALIDATOR_INPUT_V1: &str = "ketchup.gravity-support-input.v1";
pub const GRAVITY_SUPPORT_VALIDATION_POLICY_V1: &str = "ketchup.policy.gravity-support.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactValidationError {
    StaleOrInvalidPackage,
    UnresolvedDerivedIdentity,
    InvalidInstancePath,
    UnsupportedTransform,
    InvalidBounds,
    InvalidClearance,
    InvalidFaceReference,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExactFacePlane {
    pub axis: usize,
    pub coordinate_mm: f64,
    pub outward_sign: i8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactBodyParticipant {
    pub derived_identity: DerivedIdentity,
    pub instance_path: InstancePath,
    pub result_identity: BodyResultIdentity,
    pub bounds: Aabb,
    pub evidence_class: EvidenceClass,
    references: Vec<BodySubshapeRef>,
    face_planes: BTreeMap<ExactFaceRole, ExactFacePlane>,
}

impl ExactBodyParticipant {
    pub fn accept(
        snapshot: &Snapshot,
        derived_identity: DerivedIdentity,
        instance_path: InstancePath,
        package: &ExactRenderPackage,
        tolerance: TolerancePolicy,
    ) -> Result<Self, ExactValidationError> {
        if snapshot.resolve_slot(&derived_identity) != SlotResolution::Resolved {
            return Err(ExactValidationError::UnresolvedDerivedIdentity);
        }
        if !package.is_current(snapshot) {
            return Err(ExactValidationError::StaleOrInvalidPackage);
        }
        let resolved = snapshot
            .resolve_instance_path(&instance_path)
            .map_err(|_| ExactValidationError::InvalidInstancePath)?;
        if resolved.definition_id != package.identity.definition_id {
            return Err(ExactValidationError::InvalidInstancePath);
        }
        let matrix = resolved.world_transform.matrix();
        if matrix[0] != 1.0
            || matrix[5] != 1.0
            || matrix[10] != 1.0
            || matrix[15] != 1.0
            || [1, 2, 4, 6, 8, 9, 12, 13, 14]
                .into_iter()
                .any(|index| matrix[index] != 0.0)
        {
            return Err(ExactValidationError::UnsupportedTransform);
        }
        let translation = [matrix[3], matrix[7], matrix[11]];
        let min = std::array::from_fn(|axis| package.bounds_mm[0][axis] + translation[axis]);
        let max = std::array::from_fn(|axis| package.bounds_mm[1][axis] + translation[axis]);
        let bounds =
            Aabb::bounded_volume(min, max).map_err(|_| ExactValidationError::InvalidBounds)?;
        let mut face_planes = BTreeMap::new();
        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
            ExactFaceRole::West,
            ExactFaceRole::CutWest,
            ExactFaceRole::CutEast,
            ExactFaceRole::CutSouth,
            ExactFaceRole::CutNorth,
            ExactFaceRole::PocketFloor,
            ExactFaceRole::PocketWest,
            ExactFaceRole::PocketEast,
            ExactFaceRole::PocketSouth,
            ExactFaceRole::PocketNorth,
        ] {
            if package.reference(role).is_none() {
                continue;
            }
            let (axis, outward_sign) = match role {
                ExactFaceRole::Top => (2, 1),
                ExactFaceRole::Bottom => (2, -1),
                ExactFaceRole::East => (0, 1),
                ExactFaceRole::West => (0, -1),
                ExactFaceRole::CutWest => (0, -1),
                ExactFaceRole::CutEast => (0, 1),
                ExactFaceRole::CutSouth => (1, -1),
                ExactFaceRole::CutNorth => (1, 1),
                ExactFaceRole::PocketFloor => (2, 1),
                ExactFaceRole::PocketWest => (0, -1),
                ExactFaceRole::PocketEast => (0, 1),
                ExactFaceRole::PocketSouth => (1, -1),
                ExactFaceRole::PocketNorth => (1, 1),
                ExactFaceRole::CircleSide
                | ExactFaceRole::ArcSide
                | ExactFaceRole::LinearSide
                | ExactFaceRole::CutCircle
                | ExactFaceRole::CutLinear
                | ExactFaceRole::CutArc
                | ExactFaceRole::RevolveBottom
                | ExactFaceRole::RevolveBody
                | ExactFaceRole::RevolveShoulder
                | ExactFaceRole::RevolveNeck
                | ExactFaceRole::RevolveMouth
                | ExactFaceRole::RevolveSide0
                | ExactFaceRole::RevolveSide1
                | ExactFaceRole::RevolveStart
                | ExactFaceRole::RevolveEnd
                | ExactFaceRole::ShellOuterBottom
                | ExactFaceRole::ShellOuterBody
                | ExactFaceRole::ShellOuterShoulder
                | ExactFaceRole::ShellOuterNeck
                | ExactFaceRole::ShellRim
                | ExactFaceRole::ShellInnerBottom
                | ExactFaceRole::ShellInnerBody
                | ExactFaceRole::ShellInnerShoulder
                | ExactFaceRole::ShellInnerNeck
                | ExactFaceRole::BoxShellOuterBottom
                | ExactFaceRole::BoxShellOuterEast
                | ExactFaceRole::BoxShellRim
                | ExactFaceRole::PlanarOffsetFace
                | ExactFaceRole::SweepStart
                | ExactFaceRole::SweepEnd
                | ExactFaceRole::SweepSide0
                | ExactFaceRole::SweepSide1
                | ExactFaceRole::SweepSide2
                | ExactFaceRole::SweepSide3
                | ExactFaceRole::LoftStart
                | ExactFaceRole::LoftEnd
                | ExactFaceRole::LoftSide => {
                    return Err(ExactValidationError::InvalidFaceReference);
                }
            };
            let mut coordinates = package
                .triangles
                .iter()
                .filter(|triangle| triangle.face_role == Some(role))
                .flat_map(|triangle| triangle.vertex_indices)
                .map(|index| package.vertices[index as usize].position_mm[axis]);
            let Some(first) = coordinates.next() else {
                return Err(ExactValidationError::InvalidFaceReference);
            };
            if !first.is_finite() || coordinates.any(|value| value.to_bits() != first.to_bits()) {
                return Err(ExactValidationError::InvalidFaceReference);
            }
            face_planes.insert(
                role,
                ExactFacePlane {
                    axis,
                    coordinate_mm: first + translation[axis],
                    outward_sign,
                },
            );
        }
        let evidence_class = if package.identity.evaluator == EXACT_RECTANGLE_EVALUATOR_V1
            && package.vertices.len() == 8
            && package.triangles.len() == 12
        {
            EvidenceClass::Exact
        } else {
            EvidenceClass::Tolerant(
                TolerantEvidence::new(
                    tolerance.epsilon_mm(),
                    EXACT_AABB_ENVELOPE_METHOD_V1,
                    PermittedErrorDirection::FalsePositiveOnly,
                )
                .expect("the exact-body tolerance and method identity are valid"),
            )
        };
        Ok(Self {
            derived_identity,
            instance_path,
            result_identity: package.identity.clone(),
            bounds,
            evidence_class,
            references: package.references.clone(),
            face_planes,
        })
    }

    #[must_use]
    pub fn reference(&self, role: ExactFaceRole) -> Option<&BodySubshapeRef> {
        self.references
            .iter()
            .find(|reference| reference.role() == Some(role))
    }

    pub fn face_plane(
        &self,
        reference: &BodySubshapeRef,
    ) -> Result<ExactFacePlane, ExactValidationError> {
        if !self
            .references
            .iter()
            .any(|candidate| candidate == reference)
        {
            return Err(ExactValidationError::InvalidFaceReference);
        }
        self.face_planes
            .get(
                &reference
                    .role()
                    .ok_or(ExactValidationError::InvalidFaceReference)?,
            )
            .copied()
            .ok_or(ExactValidationError::InvalidFaceReference)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExactClearanceRelation {
    Separated,
    Touching,
    Intersecting,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactClearanceCase {
    pub left: ExactBodyParticipant,
    pub right: ExactBodyParticipant,
    required_minimum_mm_bits: u64,
}

impl ExactClearanceCase {
    pub fn new(
        left: ExactBodyParticipant,
        right: ExactBodyParticipant,
        required_minimum_mm: f64,
    ) -> Result<Self, ExactValidationError> {
        if !required_minimum_mm.is_finite() || required_minimum_mm < 0.0 {
            return Err(ExactValidationError::InvalidClearance);
        }
        Ok(Self {
            left,
            right,
            required_minimum_mm_bits: required_minimum_mm.to_bits(),
        })
    }

    #[must_use]
    pub fn required_minimum_mm(&self) -> f64 {
        f64::from_bits(self.required_minimum_mm_bits)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExactJointCase {
    pub left: ExactBodyParticipant,
    pub right: ExactBodyParticipant,
    pub left_contact: BodySubshapeRef,
    pub right_contact: BodySubshapeRef,
    pub declared_joint: CanonicalJoint,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExactValidationCase {
    Clearance(Box<ExactClearanceCase>),
    Joint(Box<ExactJointCase>),
}

impl ExactValidationCase {
    fn participants(&self) -> [&ExactBodyParticipant; 2] {
        match self {
            Self::Clearance(case) => [&case.left, &case.right],
            Self::Joint(case) => [&case.left, &case.right],
        }
    }
}

#[must_use]
pub fn exact_validator_descriptor() -> ValidatorDescriptor {
    ValidatorDescriptor {
        contract_id: EXACT_VALIDATOR_CONTRACT_V1.to_owned(),
        contract_version: 1,
        implementation_id: EXACT_VALIDATOR_IMPLEMENTATION_V1.to_owned(),
        implementation_version: "1.0.0".to_owned(),
        input_schema: EXACT_VALIDATOR_INPUT_V1.to_owned(),
        validation_class: ValidationClass::Collision,
        read_scopes: vec![ReadScope::DerivedGeometry, ReadScope::DeclaredJoints],
        deterministic: true,
        limits: ResourceLimits {
            maximum_input_bytes: 16 * 1024 * 1024,
            maximum_work_units: 1_000_000,
        },
    }
}

#[must_use]
pub fn exact_validation_policy() -> ValidationPolicyRef {
    ValidationPolicyRef {
        policy_id: EXACT_VALIDATION_POLICY_V1.to_owned(),
        policy_version: 1,
        contract_id: EXACT_VALIDATOR_CONTRACT_V1.to_owned(),
        contract_version: 1,
        requirement: PolicyRequirement::Required,
        severity: PolicySeverity::Error,
        blocks_release: true,
        governing_standard: None,
    }
}

pub struct BuiltinExactValidator {
    descriptor: ValidatorDescriptor,
    tolerance: TolerancePolicy,
}

impl BuiltinExactValidator {
    #[must_use]
    pub fn new(tolerance: TolerancePolicy) -> Self {
        Self {
            descriptor: exact_validator_descriptor(),
            tolerance,
        }
    }

    #[must_use]
    pub fn with_limits(tolerance: TolerancePolicy, limits: ResourceLimits) -> Self {
        let mut descriptor = exact_validator_descriptor();
        descriptor.limits = limits;
        Self {
            descriptor,
            tolerance,
        }
    }
}

impl Default for BuiltinExactValidator {
    fn default() -> Self {
        Self::new(TolerancePolicy::default())
    }
}

impl HostNeutralValidator<[ExactValidationCase]> for BuiltinExactValidator {
    fn descriptor(&self) -> &ValidatorDescriptor {
        &self.descriptor
    }

    fn invoke(
        &self,
        execution: ValidationExecution<'_, [ExactValidationCase]>,
    ) -> ValidationReport {
        let evidence_class = input_evidence(execution.input, self.tolerance);
        let work_units = u64::try_from(execution.input.len()).unwrap_or(u64::MAX);
        if work_units > self.descriptor.limits.maximum_work_units {
            return ValidationReport::not_evaluated(
                execution.invocation,
                evidence_class,
                "validator input exceeds its declared work envelope",
            );
        }
        let Some(input_len) = exact_input_len(execution.input) else {
            return ValidationReport::not_evaluated(
                execution.invocation,
                evidence_class,
                "validator input byte length overflows its declared envelope",
            );
        };
        if input_len > self.descriptor.limits.maximum_input_bytes {
            return ValidationReport::not_evaluated(
                execution.invocation,
                evidence_class,
                "validator input exceeds its declared byte envelope",
            );
        }
        let input_bytes = exact_input_bytes(execution.input);
        debug_assert_eq!(input_bytes.len() as u64, input_len);
        let mut accepted_derived_results = execution
            .input
            .iter()
            .flat_map(|case| {
                case.participants()
                    .map(|body| body.derived_identity.clone())
            })
            .collect::<Vec<_>>();
        accepted_derived_results.sort();
        accepted_derived_results.dedup();
        let mut accepted_exact_results = execution
            .input
            .iter()
            .flat_map(|case| case.participants().map(|body| body.result_identity.clone()))
            .collect::<Vec<_>>();
        accepted_exact_results.sort();
        accepted_exact_results.dedup();
        let descriptor_matches = execution.invocation.contract_id == self.descriptor.contract_id
            && execution.invocation.contract_version == self.descriptor.contract_version
            && execution.invocation.implementation_id == self.descriptor.implementation_id
            && execution.invocation.implementation_version
                == self.descriptor.implementation_version
            && execution.invocation.input_schema == self.descriptor.input_schema
            && execution.invocation.validation_class == self.descriptor.validation_class
            && execution.invocation.read_scopes == self.descriptor.read_scopes
            && execution.invocation.deterministic == self.descriptor.deterministic
            && execution.invocation.resource_limits == self.descriptor.limits
            && execution.invocation.diagnostic_schema == DIAGNOSTIC_SCHEMA_V1;
        let policy_matches = execution.invocation.policy_id == execution.policy.policy_id
            && execution.invocation.policy_version == execution.policy.policy_version
            && execution.invocation.policy_severity == execution.policy.severity
            && execution.invocation.governing_standard == execution.policy.governing_standard
            && execution.invocation.contract_id == execution.policy.contract_id
            && execution.invocation.contract_version == execution.policy.contract_version;
        let reason = if !execution.invocation.is_current(execution.snapshot) {
            Some("snapshot binding is stale or mismatched")
        } else if !descriptor_matches {
            Some("validator descriptor envelope is incompatible")
        } else if !policy_matches {
            Some("validation policy envelope is incompatible")
        } else if execution.invocation.accepted_derived_results != accepted_derived_results {
            Some("accepted derived-result identity set does not match validator input")
        } else if execution.invocation.accepted_exact_results != accepted_exact_results {
            Some("accepted exact-result identity set does not match validator input")
        } else if execution.invocation.input_digest != sha256_hex(&input_bytes) {
            Some("validator input digest does not match the supplied input")
        } else {
            None
        };
        if let Some(reason) = reason {
            return ValidationReport::not_evaluated(execution.invocation, evidence_class, reason);
        }
        evaluate_cases(
            execution.snapshot,
            execution.invocation,
            execution.input,
            self.tolerance,
        )
    }
}

fn input_evidence(cases: &[ExactValidationCase], tolerance: TolerancePolicy) -> EvidenceClass {
    EvidenceClass::weakest(
        cases
            .iter()
            .flat_map(|case| case.participants().map(|body| &body.evidence_class)),
        tolerant_evidence(tolerance),
    )
}

fn case_evidence(case: &ExactValidationCase, tolerance: TolerancePolicy) -> EvidenceClass {
    EvidenceClass::weakest(
        case.participants().map(|body| &body.evidence_class),
        tolerant_evidence(tolerance),
    )
}

fn tolerant_evidence(tolerance: TolerancePolicy) -> TolerantEvidence {
    TolerantEvidence::new(
        tolerance.epsilon_mm(),
        EXACT_AABB_ENVELOPE_METHOD_V1,
        PermittedErrorDirection::FalsePositiveOnly,
    )
    .expect("the exact-body tolerance and method identity are valid")
}

fn evaluate_cases(
    snapshot: &Snapshot,
    invocation: ValidationInvocation,
    cases: &[ExactValidationCase],
    tolerance: TolerancePolicy,
) -> ValidationReport {
    let mut diagnostics = Vec::new();
    let mut evidence_counts = EvidenceCounts::default();
    let mut failed = false;
    for case in cases {
        let evidence_class = case_evidence(case, tolerance);
        evidence_counts.record(&evidence_class);
        match case {
            ExactValidationCase::Clearance(case) => {
                let (relation, gap_mm) = clearance(case.left.bounds, case.right.bounds);
                let passes = relation == ExactClearanceRelation::Separated
                    && gap_mm >= case.required_minimum_mm();
                failed |= !passes;
                diagnostics.push(ValidationDiagnostic {
                    schema: DIAGNOSTIC_SCHEMA_V1,
                    code: if passes {
                        "clearance.minimum-satisfied"
                    } else {
                        "clearance.minimum-not-met"
                    }
                    .to_owned(),
                    severity: if passes {
                        DiagnosticSeverity::Information
                    } else {
                        DiagnosticSeverity::Error
                    },
                    evidence_class,
                    location: DiagnosticLocation {
                        entity: Some(case.right.derived_identity.clone()),
                        exact_body: Some(case.right.result_identity.clone()),
                        joint: None,
                    },
                    policy_id: invocation.policy_id.clone(),
                    policy_version: invocation.policy_version,
                    evidence: format!(
                        "relation={relation:?}; minimum_gap_mm={gap_mm:.9}; required_mm={:.9}",
                        case.required_minimum_mm()
                    ),
                });
            }
            ExactValidationCase::Joint(case) => {
                let left_plane = case.left.face_plane(&case.left_contact);
                let right_plane = case.right.face_plane(&case.right_contact);
                let canonical_matches = snapshot.joint(case.declared_joint.id())
                    == Some(&case.declared_joint)
                    && case
                        .declared_joint
                        .connects(&case.left.derived_identity, &case.right.derived_identity);
                let contact_matches = match (left_plane, right_plane) {
                    (Ok(left), Ok(right)) => {
                        let joint_min = case.declared_joint.volume().min();
                        let joint_max = case.declared_joint.volume().max();
                        left.axis == right.axis
                            && left.outward_sign == -right.outward_sign
                            && left.coordinate_mm >= joint_min[left.axis]
                            && left.coordinate_mm <= joint_max[left.axis]
                            && right.coordinate_mm >= joint_min[right.axis]
                            && right.coordinate_mm <= joint_max[right.axis]
                    }
                    (Err(_), _) | (_, Err(_)) => false,
                };
                let outcome = canonical_matches
                    .then(|| {
                        validate_joint_overlap(
                            case.left.bounds,
                            case.right.bounds,
                            Some(&case.declared_joint),
                            tolerance,
                        )
                        .ok()
                        .flatten()
                    })
                    .flatten();
                let passes = contact_matches
                    && outcome == Some(JointValidationOutcome::OverlapInsideDeclaredJointOk);
                failed |= !passes;
                diagnostics.push(ValidationDiagnostic {
                    schema: DIAGNOSTIC_SCHEMA_V1,
                    code: if passes {
                        "joint.exact-contact-validated"
                    } else {
                        "joint.exact-contact-mismatch"
                    }
                    .to_owned(),
                    severity: if passes {
                        DiagnosticSeverity::Information
                    } else {
                        DiagnosticSeverity::Error
                    },
                    evidence_class,
                    location: DiagnosticLocation {
                        entity: Some(case.right.derived_identity.clone()),
                        exact_body: Some(case.right.result_identity.clone()),
                        joint: Some(case.declared_joint.id()),
                    },
                    policy_id: invocation.policy_id.clone(),
                    policy_version: invocation.policy_version,
                    evidence: format!(
                        "canonical_joint={canonical_matches}; exact_contacts={contact_matches}; overlap={outcome:?}"
                    ),
                });
            }
        }
    }
    ValidationReport {
        invocation,
        state: if failed {
            ValidationState::Failed
        } else {
            ValidationState::Passed
        },
        evidence_counts,
        diagnostics,
        assumptions: vec![],
        unresolved_conditions: vec![],
    }
}

#[must_use]
pub fn exact_input_bytes(cases: &[ExactValidationCase]) -> Vec<u8> {
    let mut output = Vec::new();
    push_bytes(&mut output, EXACT_VALIDATOR_INPUT_V1.as_bytes());
    output.extend_from_slice(&(cases.len() as u64).to_le_bytes());
    for case in cases {
        match case {
            ExactValidationCase::Clearance(case) => {
                output.push(0);
                push_participant(&mut output, &case.left);
                push_participant(&mut output, &case.right);
                output.extend_from_slice(&case.required_minimum_mm_bits.to_le_bytes());
            }
            ExactValidationCase::Joint(case) => {
                output.push(1);
                push_participant(&mut output, &case.left);
                push_participant(&mut output, &case.right);
                push_bytes(&mut output, case.left_contact.lineage_digest.as_bytes());
                push_bytes(&mut output, case.right_contact.lineage_digest.as_bytes());
                output.extend_from_slice(&case.declared_joint.id().0.to_le_bytes());
                push_aabb(&mut output, case.declared_joint.volume());
            }
        }
    }
    output
}

fn exact_input_len(cases: &[ExactValidationCase]) -> Option<u64> {
    let mut length = 8_u64;
    add_bytes(&mut length, EXACT_VALIDATOR_INPUT_V1.as_bytes())?;
    for case in cases {
        add(&mut length, 1)?;
        for participant in case.participants() {
            add_participant(&mut length, participant)?;
        }
        match case {
            ExactValidationCase::Clearance(_) => add(&mut length, 8)?,
            ExactValidationCase::Joint(case) => {
                add_bytes(&mut length, case.left_contact.lineage_digest.as_bytes())?;
                add_bytes(&mut length, case.right_contact.lineage_digest.as_bytes())?;
                add(&mut length, 56)?;
            }
        }
    }
    Some(length)
}

fn push_participant(output: &mut Vec<u8>, participant: &ExactBodyParticipant) {
    output.extend_from_slice(
        &participant
            .derived_identity
            .root_rule_node_id
            .0
            .to_le_bytes(),
    );
    output.extend_from_slice(
        &(participant.derived_identity.slot_path.segments().len() as u64).to_le_bytes(),
    );
    for segment in participant.derived_identity.slot_path.segments() {
        output.extend_from_slice(&segment.producer_rule_id.0.to_le_bytes());
        push_bytes(output, segment.output_port.as_bytes());
        push_bytes(output, segment.semantic_key.as_bytes());
    }
    output.extend_from_slice(&participant.result_identity.source_revision.to_le_bytes());
    push_bytes(output, participant.result_identity.source_digest.as_bytes());
    push_bytes(
        output,
        participant
            .result_identity
            .canonical_input_digest
            .as_bytes(),
    );
    push_bytes(
        output,
        participant.result_identity.exact_input_digest.as_bytes(),
    );
    push_bytes(
        output,
        participant.result_identity.result_fingerprint.as_bytes(),
    );
    push_bytes(output, participant.result_identity.evaluator.as_bytes());
    push_bytes(output, participant.result_identity.backend.as_bytes());
    push_bytes(output, participant.result_identity.tolerance.as_bytes());
    push_instance_path(output, &participant.instance_path);
    push_aabb(output, participant.bounds);
}

fn add_participant(length: &mut u64, participant: &ExactBodyParticipant) -> Option<()> {
    add(length, 40)?;
    for segment in participant.derived_identity.slot_path.segments() {
        add(length, 8)?;
        add_bytes(length, segment.output_port.as_bytes())?;
        add_bytes(length, segment.semantic_key.as_bytes())?;
    }
    for value in [
        participant.result_identity.source_digest.as_bytes(),
        participant
            .result_identity
            .canonical_input_digest
            .as_bytes(),
        participant.result_identity.exact_input_digest.as_bytes(),
        participant.result_identity.result_fingerprint.as_bytes(),
        participant.result_identity.evaluator.as_bytes(),
        participant.result_identity.backend.as_bytes(),
        participant.result_identity.tolerance.as_bytes(),
    ] {
        add_bytes(length, value)?;
    }
    for _ in participant.instance_path.steps() {
        add(length, 9)?;
    }
    add(length, 48)
}

fn push_instance_path(output: &mut Vec<u8>, path: &InstancePath) {
    output.extend_from_slice(&path.root_occurrence().0.to_le_bytes());
    output.extend_from_slice(&(path.steps().len() as u64).to_le_bytes());
    for step in path.steps() {
        let (tag, id) = match step {
            InstancePathStep::Group(id) => (0, id.0),
            InstancePathStep::Occurrence(id) => (1, id.0),
        };
        output.push(tag);
        output.extend_from_slice(&id.to_le_bytes());
    }
}

fn push_aabb(output: &mut Vec<u8>, bounds: Aabb) {
    for value in bounds.min().into_iter().chain(bounds.max()) {
        output.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value);
}

fn add(length: &mut u64, amount: u64) -> Option<()> {
    *length = length.checked_add(amount)?;
    Some(())
}

fn add_bytes(length: &mut u64, value: &[u8]) -> Option<()> {
    add(length, 8)?;
    add(length, u64::try_from(value.len()).ok()?)
}

fn clearance(left: Aabb, right: Aabb) -> (ExactClearanceRelation, f64) {
    let gaps = std::array::from_fn::<_, 3, _>(|axis| {
        (right.min()[axis] - left.max()[axis])
            .max(left.min()[axis] - right.max()[axis])
            .max(0.0)
    });
    let gap = gaps
        .into_iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if gap > 0.0 {
        return (ExactClearanceRelation::Separated, gap);
    }
    let positive_intersection = (0..3).all(|axis| {
        left.max()[axis].min(right.max()[axis]) > left.min()[axis].max(right.min()[axis])
    });
    if positive_intersection {
        (ExactClearanceRelation::Intersecting, 0.0)
    } else {
        (ExactClearanceRelation::Touching, 0.0)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GeneralBodySource {
    Exact(ExactResultKey),
    CanonicalMesh {
        definition_id: DefinitionId,
        feature_id: FeatureId,
        geometry_digest: String,
    },
    CanonicalExtrusion {
        definition_id: DefinitionId,
        profile_id: FeatureId,
        extrusion_id: FeatureId,
        geometry_digest: String,
    },
    CanonicalExactGraph {
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
        graph_digest: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneralBodyGeometryEvidence {
    source_frame_extents_mm: [f64; 3],
    source_frame_center_world_mm: [f64; 3],
    source_axis_world_direction: [[f64; 3]; 3],
    source_axis_world_scale: [f64; 3],
    source_axis_world_z_alignment: [f64; 3],
}

impl GeneralBodyGeometryEvidence {
    #[must_use]
    pub const fn source_frame_extents_mm(&self) -> [f64; 3] {
        self.source_frame_extents_mm
    }

    #[must_use]
    pub const fn source_frame_center_world_mm(&self) -> [f64; 3] {
        self.source_frame_center_world_mm
    }

    #[must_use]
    pub fn source_axis_world_direction(&self, axis: usize) -> Option<[f64; 3]> {
        self.source_axis_world_direction.get(axis).copied()
    }

    #[must_use]
    pub fn source_axis_world_scale(&self, axis: usize) -> Option<f64> {
        self.source_axis_world_scale.get(axis).copied()
    }

    #[must_use]
    pub fn source_axis_world_z_alignment(&self, axis: usize) -> Option<f64> {
        self.source_axis_world_z_alignment.get(axis).copied()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralBodyParticipant {
    instance_path: InstancePath,
    source: GeneralBodySource,
    bounds: Aabb,
    geometry_evidence: GeneralBodyGeometryEvidence,
    evidence_class: EvidenceClass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralBodyValidationError {
    InvalidOrHiddenInstance,
    UnavailableOrAmbiguousGeometry,
    StaleExactResult,
    InvalidGeometry,
    InvalidGravityVector,
    InvalidClearance,
}

impl GeneralBodyParticipant {
    pub fn accept(
        snapshot: &Snapshot,
        registry: &ExactResultRegistry,
        instance_path: InstancePath,
        tolerance: TolerancePolicy,
    ) -> Result<Self, GeneralBodyValidationError> {
        let occurrence = snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| occurrence.instance_path == instance_path && occurrence.visible)
            .ok_or(GeneralBodyValidationError::InvalidOrHiddenInstance)?;
        let transform = occurrence.transform;
        let (source, vertices, exact_box) =
            if let Some(package) = registry.get(&occurrence.definition_id) {
                if !package.is_current(snapshot) {
                    return Err(GeneralBodyValidationError::StaleExactResult);
                }
                let exact_box = matches!(
                    package.as_ref(),
                    ExactBodyPackage::Rectangle(render)
                        if render.identity.evaluator == EXACT_RECTANGLE_EVALUATOR_V1
                            && render.vertices.len() == 8
                            && render.triangles.len() == 12
                            && is_translation_only(transform)
                );
                (
                    GeneralBodySource::Exact(package.result_key()),
                    package
                        .vertices()
                        .iter()
                        .map(|vertex| vertex.position_mm)
                        .collect::<Vec<_>>(),
                    exact_box,
                )
            } else {
                let definition = snapshot
                    .definition(occurrence.definition_id)
                    .ok_or(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry)?;
                match definition.feature_ids() {
                    [feature_id] => {
                        let FeatureKind::MeshBody(spec) = snapshot
                            .feature(*feature_id)
                            .ok_or(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry)?
                            .kind()
                        else {
                            return Err(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry);
                        };
                        (
                            GeneralBodySource::CanonicalMesh {
                                definition_id: occurrence.definition_id,
                                feature_id: *feature_id,
                                geometry_digest: mesh_geometry_digest(spec),
                            },
                            spec.vertices_mm.clone(),
                            false,
                        )
                    }
                    [profile_id, extrusion_id] => {
                        let FeatureKind::Profile { points_mm } = snapshot
                            .feature(*profile_id)
                            .ok_or(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry)?
                            .kind()
                        else {
                            return Err(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry);
                        };
                        let FeatureKind::Extrusion { profile, height } = snapshot
                            .feature(*extrusion_id)
                            .ok_or(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry)?
                            .kind()
                        else {
                            return Err(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry);
                        };
                        if profile != profile_id || height.millimetres() <= 0.0 {
                            return Err(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry);
                        }
                        let vertices = points_mm
                            .iter()
                            .flat_map(|point| {
                                [
                                    [point[0], point[1], 0.0],
                                    [point[0], point[1], height.millimetres()],
                                ]
                            })
                            .collect::<Vec<_>>();
                        let exact_box = is_axis_aligned_rectangle_profile(points_mm)
                            && is_translation_only(transform);
                        (
                            GeneralBodySource::CanonicalExtrusion {
                                definition_id: occurrence.definition_id,
                                profile_id: *profile_id,
                                extrusion_id: *extrusion_id,
                                geometry_digest: canonical_extrusion_geometry_digest(
                                    points_mm,
                                    height.millimetres(),
                                ),
                            },
                            vertices,
                            exact_box,
                        )
                    }
                    feature_ids => {
                        let producer_feature_id = *feature_ids
                            .last()
                            .ok_or(GeneralBodyValidationError::UnavailableOrAmbiguousGeometry)?;
                        let graph = ExactBRepGraph::from_snapshot(
                            snapshot,
                            occurrence.definition_id,
                            producer_feature_id,
                        )
                        .map_err(|_| GeneralBodyValidationError::UnavailableOrAmbiguousGeometry)?;
                        let [minimum, maximum] = graph
                            .producer_bounds_mm()
                            .map_err(|_| GeneralBodyValidationError::InvalidGeometry)?
                            .ok_or(GeneralBodyValidationError::InvalidGeometry)?;
                        let vertices = [minimum[0], maximum[0]]
                            .into_iter()
                            .flat_map(|x| {
                                [minimum[1], maximum[1]].into_iter().flat_map(move |y| {
                                    [minimum[2], maximum[2]].into_iter().map(move |z| [x, y, z])
                                })
                            })
                            .collect();
                        (
                            GeneralBodySource::CanonicalExactGraph {
                                definition_id: occurrence.definition_id,
                                producer_feature_id,
                                graph_digest: graph.graph_digest,
                            },
                            vertices,
                            false,
                        )
                    }
                }
            };
        let geometry_evidence = general_body_geometry_evidence(transform, &vertices)?;
        let bounds = transformed_body_bounds(transform, &vertices)?;
        let evidence_class = if exact_box {
            EvidenceClass::Exact
        } else {
            EvidenceClass::Tolerant(general_tolerant_evidence(tolerance))
        };
        Ok(Self {
            instance_path,
            source,
            bounds,
            geometry_evidence,
            evidence_class,
        })
    }

    #[must_use]
    pub fn instance_path(&self) -> &InstancePath {
        &self.instance_path
    }

    #[must_use]
    pub fn source(&self) -> &GeneralBodySource {
        &self.source
    }

    #[must_use]
    pub const fn bounds(&self) -> Aabb {
        self.bounds
    }

    #[must_use]
    pub const fn geometry_evidence(&self) -> &GeneralBodyGeometryEvidence {
        &self.geometry_evidence
    }

    #[must_use]
    pub const fn evidence_class(&self) -> &EvidenceClass {
        &self.evidence_class
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneralBodyNarrowPhaseRelation {
    Separated,
    Touching,
    Intersecting,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralBodyNarrowPhase {
    pub relation: GeneralBodyNarrowPhaseRelation,
    pub signed_separation_mm: f64,
    pub separation_axis_world: [f64; 3],
    pub evidence_class: EvidenceClass,
    pub method: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralBodyContainment {
    pub clearances_mm: [f64; 6],
    pub evidence_class: EvidenceClass,
    pub method: &'static str,
}

#[derive(Clone, Copy)]
struct GeneralBodyObb {
    center: [f64; 3],
    axes: [[f64; 3]; 3],
    half_extents: [f64; 3],
}

pub fn general_body_narrow_phase(
    left: &GeneralBodyParticipant,
    right: &GeneralBodyParticipant,
    tolerance: TolerancePolicy,
) -> Result<GeneralBodyNarrowPhase, GeneralBodyValidationError> {
    let left_obb = general_body_obb(left)?;
    let right_obb = general_body_obb(right)?;
    let delta = vector_subtract(right_obb.center, left_obb.center);
    let mut axes = Vec::with_capacity(15);
    axes.extend(left_obb.axes);
    axes.extend(right_obb.axes);
    for left_axis in left_obb.axes {
        for right_axis in right_obb.axes {
            let cross = vector_cross(left_axis, right_axis);
            let length = vector_length(cross);
            if length > 1.0e-12 {
                axes.push(cross.map(|value| value / length));
            }
        }
    }
    let (signed_separation_mm, mut separation_axis_world) = axes
        .into_iter()
        .map(|axis| {
            (
                vector_dot(delta, axis).abs()
                    - obb_projection_radius(left_obb, axis)
                    - obb_projection_radius(right_obb, axis),
                axis,
            )
        })
        .max_by(|left, right| f64::total_cmp(&left.0, &right.0))
        .ok_or(GeneralBodyValidationError::InvalidGeometry)?;
    if vector_dot(delta, separation_axis_world) < 0.0 {
        separation_axis_world = separation_axis_world.map(|component| -component);
    }
    let epsilon_mm = tolerance.epsilon_mm();
    let relation = if signed_separation_mm > epsilon_mm {
        GeneralBodyNarrowPhaseRelation::Separated
    } else if signed_separation_mm >= -epsilon_mm {
        GeneralBodyNarrowPhaseRelation::Touching
    } else {
        GeneralBodyNarrowPhaseRelation::Intersecting
    };
    Ok(GeneralBodyNarrowPhase {
        relation,
        signed_separation_mm,
        separation_axis_world,
        evidence_class: general_obb_evidence(left, right, tolerance),
        method: GENERAL_BODY_OBB_NARROW_PHASE_METHOD_V1,
    })
}

pub fn general_body_containment(
    container: &GeneralBodyParticipant,
    body: &GeneralBodyParticipant,
    tolerance: TolerancePolicy,
) -> Result<GeneralBodyContainment, GeneralBodyValidationError> {
    let container_obb = general_body_obb(container)?;
    let body_obb = general_body_obb(body)?;
    let center_delta = vector_subtract(body_obb.center, container_obb.center);
    let mut clearances_mm = [0.0; 6];
    for axis in 0..3 {
        let center = vector_dot(center_delta, container_obb.axes[axis]);
        let radius = obb_projection_radius(body_obb, container_obb.axes[axis]);
        clearances_mm[axis * 2] = container_obb.half_extents[axis] + center - radius;
        clearances_mm[axis * 2 + 1] = container_obb.half_extents[axis] - center - radius;
    }
    Ok(GeneralBodyContainment {
        clearances_mm,
        evidence_class: general_obb_evidence(container, body, tolerance),
        method: GENERAL_BODY_OBB_NARROW_PHASE_METHOD_V1,
    })
}

fn general_body_obb(
    participant: &GeneralBodyParticipant,
) -> Result<GeneralBodyObb, GeneralBodyValidationError> {
    let geometry = participant.geometry_evidence();
    let axes = std::array::from_fn(|axis| {
        geometry
            .source_axis_world_direction(axis)
            .expect("three source axes are always present")
    });
    for left in 0..3 {
        for right in left + 1..3 {
            if vector_dot(axes[left], axes[right]).abs() > 1.0e-9 {
                return Err(GeneralBodyValidationError::InvalidGeometry);
            }
        }
    }
    let extents = geometry.source_frame_extents_mm();
    let half_extents = std::array::from_fn(|axis| {
        extents[axis]
            * geometry
                .source_axis_world_scale(axis)
                .expect("three source-axis scales are always present")
            * 0.5
    });
    Ok(GeneralBodyObb {
        center: geometry.source_frame_center_world_mm(),
        axes,
        half_extents,
    })
}

fn general_obb_evidence(
    left: &GeneralBodyParticipant,
    right: &GeneralBodyParticipant,
    tolerance: TolerancePolicy,
) -> EvidenceClass {
    if matches!(left.evidence_class(), EvidenceClass::Exact)
        && matches!(right.evidence_class(), EvidenceClass::Exact)
    {
        EvidenceClass::Exact
    } else {
        EvidenceClass::Tolerant(
            TolerantEvidence::new(
                tolerance.epsilon_mm(),
                GENERAL_BODY_OBB_NARROW_PHASE_METHOD_V1,
                PermittedErrorDirection::FalsePositiveOnly,
            )
            .expect("the general-body tolerance and OBB method identity are valid"),
        )
    }
}

fn obb_projection_radius(obb: GeneralBodyObb, axis: [f64; 3]) -> f64 {
    (0..3)
        .map(|index| obb.half_extents[index] * vector_dot(obb.axes[index], axis).abs())
        .sum()
}

fn vector_dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    (0..3).map(|axis| left[axis] * right[axis]).sum()
}

fn vector_subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| left[axis] - right[axis])
}

fn vector_cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn vector_length(vector: [f64; 3]) -> f64 {
    vector_dot(vector, vector).sqrt()
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralClearanceCase {
    pub left: GeneralBodyParticipant,
    pub right: GeneralBodyParticipant,
    required_minimum_mm_bits: u64,
}

impl GeneralClearanceCase {
    pub fn new(
        left: GeneralBodyParticipant,
        right: GeneralBodyParticipant,
        required_minimum_mm: f64,
    ) -> Result<Self, GeneralBodyValidationError> {
        if !required_minimum_mm.is_finite() || required_minimum_mm < 0.0 {
            return Err(GeneralBodyValidationError::InvalidClearance);
        }
        Ok(Self {
            left,
            right,
            required_minimum_mm_bits: required_minimum_mm.to_bits(),
        })
    }

    #[must_use]
    pub fn required_minimum_mm(&self) -> f64 {
        f64::from_bits(self.required_minimum_mm_bits)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GravitySupportParticipant {
    pub body: GeneralBodyParticipant,
    pub support_group: String,
    pub explicitly_grounded: bool,
}

impl GravitySupportParticipant {
    #[must_use]
    pub fn new(
        body: GeneralBodyParticipant,
        support_group: impl Into<String>,
        explicitly_grounded: bool,
    ) -> Self {
        Self {
            body,
            support_group: support_group.into(),
            explicitly_grounded,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GravitySupportInput {
    participants: Vec<GravitySupportParticipant>,
    gravity_vector_m_s2: [f64; 3],
    gravity_direction: [f64; 3],
    gravity_magnitude_m_s2: f64,
}

impl GravitySupportInput {
    pub fn new(
        participants: Vec<GravitySupportParticipant>,
        gravity_vector_m_s2: [f64; 3],
    ) -> Result<Self, GeneralBodyValidationError> {
        if gravity_vector_m_s2
            .into_iter()
            .any(|component| !component.is_finite())
        {
            return Err(GeneralBodyValidationError::InvalidGravityVector);
        }
        let gravity_magnitude_m_s2 = vector_length(gravity_vector_m_s2);
        if gravity_magnitude_m_s2 <= f64::EPSILON {
            return Err(GeneralBodyValidationError::InvalidGravityVector);
        }
        Ok(Self {
            participants,
            gravity_vector_m_s2,
            gravity_direction: gravity_vector_m_s2
                .map(|component| component / gravity_magnitude_m_s2),
            gravity_magnitude_m_s2,
        })
    }

    #[must_use]
    pub fn participants(&self) -> &[GravitySupportParticipant] {
        &self.participants
    }

    #[must_use]
    pub const fn gravity_vector_m_s2(&self) -> [f64; 3] {
        self.gravity_vector_m_s2
    }

    #[must_use]
    pub const fn gravity_direction(&self) -> [f64; 3] {
        self.gravity_direction
    }

    #[must_use]
    pub const fn gravity_magnitude_m_s2(&self) -> f64 {
        self.gravity_magnitude_m_s2
    }
}

#[must_use]
pub fn gravity_support_validator_descriptor() -> ValidatorDescriptor {
    ValidatorDescriptor {
        contract_id: GRAVITY_SUPPORT_VALIDATOR_CONTRACT_V1.to_owned(),
        contract_version: 1,
        implementation_id: GRAVITY_SUPPORT_VALIDATOR_IMPLEMENTATION_V1.to_owned(),
        implementation_version: "1.0.0".to_owned(),
        input_schema: GRAVITY_SUPPORT_VALIDATOR_INPUT_V1.to_owned(),
        validation_class: ValidationClass::StructuralBestEffort,
        read_scopes: vec![ReadScope::CanonicalGraph, ReadScope::DerivedGeometry],
        deterministic: true,
        limits: ResourceLimits {
            maximum_input_bytes: 16 * 1024 * 1024,
            maximum_work_units: 1_000_000,
        },
    }
}

#[must_use]
pub fn gravity_support_validation_policy() -> ValidationPolicyRef {
    ValidationPolicyRef {
        policy_id: GRAVITY_SUPPORT_VALIDATION_POLICY_V1.to_owned(),
        policy_version: 1,
        contract_id: GRAVITY_SUPPORT_VALIDATOR_CONTRACT_V1.to_owned(),
        contract_version: 1,
        requirement: PolicyRequirement::Optional,
        severity: PolicySeverity::Warning,
        blocks_release: false,
        governing_standard: None,
    }
}

pub struct BuiltinGravitySupportValidator {
    descriptor: ValidatorDescriptor,
    tolerance: TolerancePolicy,
}

impl BuiltinGravitySupportValidator {
    #[must_use]
    pub fn new(tolerance: TolerancePolicy) -> Self {
        Self {
            descriptor: gravity_support_validator_descriptor(),
            tolerance,
        }
    }
}

impl Default for BuiltinGravitySupportValidator {
    fn default() -> Self {
        Self::new(TolerancePolicy::default())
    }
}

#[must_use]
pub fn general_body_validator_descriptor() -> ValidatorDescriptor {
    ValidatorDescriptor {
        contract_id: GENERAL_BODY_VALIDATOR_CONTRACT_V1.to_owned(),
        contract_version: 1,
        implementation_id: GENERAL_BODY_VALIDATOR_IMPLEMENTATION_V1.to_owned(),
        implementation_version: "1.0.0".to_owned(),
        input_schema: GENERAL_BODY_VALIDATOR_INPUT_V1.to_owned(),
        validation_class: ValidationClass::Collision,
        read_scopes: vec![ReadScope::CanonicalGraph, ReadScope::DerivedGeometry],
        deterministic: true,
        limits: ResourceLimits {
            maximum_input_bytes: 16 * 1024 * 1024,
            maximum_work_units: 1_000_000,
        },
    }
}

#[must_use]
pub fn general_body_validation_policy() -> ValidationPolicyRef {
    ValidationPolicyRef {
        policy_id: GENERAL_BODY_VALIDATION_POLICY_V1.to_owned(),
        policy_version: 1,
        contract_id: GENERAL_BODY_VALIDATOR_CONTRACT_V1.to_owned(),
        contract_version: 1,
        requirement: PolicyRequirement::Required,
        severity: PolicySeverity::Error,
        blocks_release: true,
        governing_standard: None,
    }
}

pub struct BuiltinGeneralBodyValidator {
    descriptor: ValidatorDescriptor,
    tolerance: TolerancePolicy,
}

impl BuiltinGeneralBodyValidator {
    #[must_use]
    pub fn new(tolerance: TolerancePolicy) -> Self {
        Self {
            descriptor: general_body_validator_descriptor(),
            tolerance,
        }
    }
}

impl Default for BuiltinGeneralBodyValidator {
    fn default() -> Self {
        Self::new(TolerancePolicy::default())
    }
}

impl HostNeutralValidator<GravitySupportInput> for BuiltinGravitySupportValidator {
    fn descriptor(&self) -> &ValidatorDescriptor {
        &self.descriptor
    }

    fn invoke(&self, execution: ValidationExecution<'_, GravitySupportInput>) -> ValidationReport {
        let evidence_class = gravity_input_evidence(execution.input, self.tolerance);
        let participant_count =
            u64::try_from(execution.input.participants().len()).unwrap_or(u64::MAX);
        let work_units = participant_count.saturating_mul(participant_count);
        if work_units > self.descriptor.limits.maximum_work_units {
            return ValidationReport::not_evaluated(
                execution.invocation,
                evidence_class,
                "gravity-support input exceeds its declared work envelope",
            );
        }
        let input_bytes = gravity_support_input_bytes(execution.input);
        if u64::try_from(input_bytes.len()).unwrap_or(u64::MAX)
            > self.descriptor.limits.maximum_input_bytes
        {
            return ValidationReport::not_evaluated(
                execution.invocation,
                evidence_class,
                "gravity-support input exceeds its declared byte envelope",
            );
        }
        let descriptor_matches = execution.invocation.protocol
            == crate::validation::VALIDATOR_PROTOCOL_V1
            && execution.invocation.contract_id == self.descriptor.contract_id
            && execution.invocation.contract_version == self.descriptor.contract_version
            && execution.invocation.implementation_id == self.descriptor.implementation_id
            && execution.invocation.implementation_version
                == self.descriptor.implementation_version
            && execution.invocation.input_schema == self.descriptor.input_schema
            && execution.invocation.validation_class == self.descriptor.validation_class
            && execution.invocation.read_scopes == self.descriptor.read_scopes
            && execution.invocation.deterministic == self.descriptor.deterministic
            && execution.invocation.resource_limits == self.descriptor.limits
            && execution.invocation.diagnostic_schema == DIAGNOSTIC_SCHEMA_V1;
        let policy_matches = execution.invocation.policy_id == execution.policy.policy_id
            && execution.invocation.policy_version == execution.policy.policy_version
            && execution.invocation.policy_severity == execution.policy.severity
            && execution.invocation.governing_standard == execution.policy.governing_standard
            && execution.invocation.contract_id == execution.policy.contract_id
            && execution.invocation.contract_version == execution.policy.contract_version;
        let reason = if !execution.invocation.is_current(execution.snapshot) {
            Some("snapshot binding is stale or mismatched")
        } else if !descriptor_matches {
            Some("validator descriptor envelope is incompatible")
        } else if !policy_matches {
            Some("validation policy envelope is incompatible")
        } else if !execution.invocation.accepted_derived_results.is_empty()
            || !execution.invocation.accepted_exact_results.is_empty()
        {
            Some("legacy narrow result identities are incompatible with gravity-support input")
        } else if execution.invocation.input_digest != sha256_hex(&input_bytes) {
            Some("validator input digest does not match the supplied input")
        } else {
            None
        };
        if let Some(reason) = reason {
            return ValidationReport::not_evaluated(execution.invocation, evidence_class, reason);
        }
        evaluate_gravity_support(execution.invocation, execution.input, self.tolerance)
    }
}

impl HostNeutralValidator<[GeneralClearanceCase]> for BuiltinGeneralBodyValidator {
    fn descriptor(&self) -> &ValidatorDescriptor {
        &self.descriptor
    }

    fn invoke(
        &self,
        execution: ValidationExecution<'_, [GeneralClearanceCase]>,
    ) -> ValidationReport {
        let evidence_class = general_input_evidence(execution.input, self.tolerance);
        let work_units = u64::try_from(execution.input.len()).unwrap_or(u64::MAX);
        if work_units > self.descriptor.limits.maximum_work_units {
            return ValidationReport::not_evaluated(
                execution.invocation,
                evidence_class,
                "validator input exceeds its declared work envelope",
            );
        }
        let input_bytes = general_body_input_bytes(execution.input);
        if u64::try_from(input_bytes.len()).unwrap_or(u64::MAX)
            > self.descriptor.limits.maximum_input_bytes
        {
            return ValidationReport::not_evaluated(
                execution.invocation,
                evidence_class,
                "validator input exceeds its declared byte envelope",
            );
        }
        let descriptor_matches = execution.invocation.protocol
            == crate::validation::VALIDATOR_PROTOCOL_V1
            && execution.invocation.contract_id == self.descriptor.contract_id
            && execution.invocation.contract_version == self.descriptor.contract_version
            && execution.invocation.implementation_id == self.descriptor.implementation_id
            && execution.invocation.implementation_version
                == self.descriptor.implementation_version
            && execution.invocation.input_schema == self.descriptor.input_schema
            && execution.invocation.validation_class == self.descriptor.validation_class
            && execution.invocation.read_scopes == self.descriptor.read_scopes
            && execution.invocation.deterministic == self.descriptor.deterministic
            && execution.invocation.resource_limits == self.descriptor.limits
            && execution.invocation.diagnostic_schema == DIAGNOSTIC_SCHEMA_V1;
        let policy_matches = execution.invocation.policy_id == execution.policy.policy_id
            && execution.invocation.policy_version == execution.policy.policy_version
            && execution.invocation.policy_severity == execution.policy.severity
            && execution.invocation.governing_standard == execution.policy.governing_standard
            && execution.invocation.contract_id == execution.policy.contract_id
            && execution.invocation.contract_version == execution.policy.contract_version;
        let reason = if !execution.invocation.is_current(execution.snapshot) {
            Some("snapshot binding is stale or mismatched")
        } else if !descriptor_matches {
            Some("validator descriptor envelope is incompatible")
        } else if !policy_matches {
            Some("validation policy envelope is incompatible")
        } else if !execution.invocation.accepted_derived_results.is_empty()
            || !execution.invocation.accepted_exact_results.is_empty()
        {
            Some("legacy narrow result identities are incompatible with general registry-key input")
        } else if execution
            .input
            .iter()
            .any(|case| general_body_narrow_phase(&case.left, &case.right, self.tolerance).is_err())
        {
            Some("oriented narrow-phase geometry is unavailable or non-orthogonal")
        } else if execution.invocation.input_digest != sha256_hex(&input_bytes) {
            Some("validator input digest does not match the supplied input")
        } else {
            None
        };
        if let Some(reason) = reason {
            return ValidationReport::not_evaluated(execution.invocation, evidence_class, reason);
        }
        evaluate_general_clearance(execution.invocation, execution.input, self.tolerance)
    }
}

#[must_use]
pub fn gravity_support_input_bytes(input: &GravitySupportInput) -> Vec<u8> {
    let mut output = Vec::new();
    push_bytes(&mut output, GRAVITY_SUPPORT_VALIDATOR_INPUT_V1.as_bytes());
    for component in input.gravity_vector_m_s2() {
        output.extend_from_slice(&component.to_bits().to_le_bytes());
    }
    output.extend_from_slice(&(input.participants().len() as u64).to_le_bytes());
    for participant in input.participants() {
        push_general_participant(&mut output, &participant.body);
        push_bytes(&mut output, participant.support_group.as_bytes());
        output.push(u8::from(participant.explicitly_grounded));
    }
    output
}

#[derive(Clone, Debug)]
enum GravitySupportSource {
    ExplicitGrounding,
    Contact(usize),
}

fn evaluate_gravity_support(
    invocation: ValidationInvocation,
    input: &GravitySupportInput,
    tolerance: TolerancePolicy,
) -> ValidationReport {
    let participants = input.participants();
    let support_direction = input.gravity_direction().map(|component| -component);
    let mut support = participants
        .iter()
        .map(|participant| {
            participant
                .explicitly_grounded
                .then_some(GravitySupportSource::ExplicitGrounding)
        })
        .collect::<Vec<_>>();
    loop {
        let mut changed = false;
        for candidate_index in 0..participants.len() {
            if support[candidate_index].is_some() {
                continue;
            }
            let candidate = &participants[candidate_index].body;
            if let Some(supporter_index) = (0..participants.len()).find(|&supporter_index| {
                supporter_index != candidate_index
                    && support[supporter_index].is_some()
                    && participants[candidate_index].support_group
                        == participants[supporter_index].support_group
                    && body_rests_on(
                        candidate,
                        &participants[supporter_index].body,
                        support_direction,
                        tolerance,
                    )
            }) {
                support[candidate_index] = Some(GravitySupportSource::Contact(supporter_index));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut evidence_counts = EvidenceCounts::default();
    let mut diagnostics = Vec::with_capacity(participants.len());
    let mut failed = false;
    for (participant, source) in participants.iter().zip(support) {
        let evidence_class = match &source {
            Some(GravitySupportSource::Contact(supporter_index)) => EvidenceClass::weakest(
                [
                    &participant.body.evidence_class,
                    &participants[*supporter_index].body.evidence_class,
                ],
                general_tolerant_evidence(tolerance),
            ),
            _ => participant.body.evidence_class.clone(),
        };
        evidence_counts.record(&evidence_class);
        let (code, severity, evidence) = match source {
            Some(GravitySupportSource::ExplicitGrounding) => (
                "gravity.supported-explicit",
                DiagnosticSeverity::Information,
                format!(
                    "body={}; gravity_direction={:?}; support_group={}; canonical_grounding=true",
                    instance_path_label(&participant.body.instance_path),
                    input.gravity_direction(),
                    participant.support_group
                ),
            ),
            Some(GravitySupportSource::Contact(supporter_index)) => (
                "gravity.supported-contact",
                DiagnosticSeverity::Information,
                format!(
                    "body={}; gravity_direction={:?}; support_group={}; supported_by={}; contact_method={}",
                    instance_path_label(&participant.body.instance_path),
                    input.gravity_direction(),
                    participant.support_group,
                    instance_path_label(&participants[supporter_index].body.instance_path),
                    GENERAL_BODY_OBB_NARROW_PHASE_METHOD_V1
                ),
            ),
            None => {
                failed = true;
                (
                    "gravity.unsupported",
                    DiagnosticSeverity::Warning,
                    format!(
                        "body={}; gravity_direction={:?}; support_group={}; no_explicit_ground_or_supported_contact=true",
                        instance_path_label(&participant.body.instance_path),
                        input.gravity_direction(),
                        participant.support_group
                    ),
                )
            }
        };
        diagnostics.push(ValidationDiagnostic {
            schema: DIAGNOSTIC_SCHEMA_V1,
            code: code.to_owned(),
            severity,
            evidence_class,
            location: DiagnosticLocation {
                entity: None,
                exact_body: None,
                joint: None,
            },
            policy_id: invocation.policy_id.clone(),
            policy_version: invocation.policy_version,
            evidence,
        });
    }
    ValidationReport {
        invocation,
        state: if failed {
            ValidationState::Failed
        } else {
            ValidationState::Passed
        },
        evidence_counts,
        diagnostics,
        assumptions: vec![
            "gravity uses the explicit typed non-zero vector supplied in the validator input"
                .to_owned(),
            "only explicitly grounded participants seed support propagation".to_owned(),
            "load-bearing contact requires same-group oriented OBB-SAT touching on the support plane"
                .to_owned(),
        ],
        unresolved_conditions: vec![],
    }
}

fn body_rests_on(
    candidate: &GeneralBodyParticipant,
    supporter: &GeneralBodyParticipant,
    support_direction: [f64; 3],
    tolerance: TolerancePolicy,
) -> bool {
    let Ok(candidate_obb) = general_body_obb(candidate) else {
        return false;
    };
    let Ok(supporter_obb) = general_body_obb(supporter) else {
        return false;
    };
    let candidate_lower_mm = vector_dot(candidate_obb.center, support_direction)
        - obb_projection_radius(candidate_obb, support_direction);
    let supporter_upper_mm = vector_dot(supporter_obb.center, support_direction)
        + obb_projection_radius(supporter_obb, support_direction);
    (candidate_lower_mm - supporter_upper_mm).abs() <= tolerance.epsilon_mm()
        && general_body_narrow_phase(candidate, supporter, tolerance)
            .is_ok_and(|evidence| evidence.relation == GeneralBodyNarrowPhaseRelation::Touching)
}

fn gravity_input_evidence(
    input: &GravitySupportInput,
    tolerance: TolerancePolicy,
) -> EvidenceClass {
    EvidenceClass::weakest(
        input
            .participants()
            .iter()
            .map(|participant| &participant.body.evidence_class),
        general_tolerant_evidence(tolerance),
    )
}

#[must_use]
pub fn general_body_input_bytes(cases: &[GeneralClearanceCase]) -> Vec<u8> {
    let mut output = Vec::new();
    push_bytes(&mut output, GENERAL_BODY_VALIDATOR_INPUT_V1.as_bytes());
    output.extend_from_slice(&(cases.len() as u64).to_le_bytes());
    for case in cases {
        push_general_participant(&mut output, &case.left);
        push_general_participant(&mut output, &case.right);
        output.extend_from_slice(&case.required_minimum_mm_bits.to_le_bytes());
    }
    output
}

fn evaluate_general_clearance(
    invocation: ValidationInvocation,
    cases: &[GeneralClearanceCase],
    tolerance: TolerancePolicy,
) -> ValidationReport {
    let mut diagnostics = Vec::new();
    let mut evidence_counts = EvidenceCounts::default();
    let mut failed = false;
    for case in cases {
        let narrow_phase = general_body_narrow_phase(&case.left, &case.right, tolerance)
            .expect("accepted general-body participants have valid OBB evidence");
        let evidence_class = narrow_phase.evidence_class.clone();
        evidence_counts.record(&evidence_class);
        let relation = narrow_phase.relation;
        let gap_mm = narrow_phase.signed_separation_mm.max(0.0);
        let collision_only = case.required_minimum_mm() == 0.0;
        let passes = if collision_only {
            relation != GeneralBodyNarrowPhaseRelation::Intersecting
        } else {
            relation == GeneralBodyNarrowPhaseRelation::Separated
                && gap_mm >= case.required_minimum_mm()
        };
        failed |= !passes;
        diagnostics.push(ValidationDiagnostic {
            schema: DIAGNOSTIC_SCHEMA_V1,
            code: match (collision_only, passes) {
                (true, true) => "collision.none",
                (true, false) => "collision.detected",
                (false, true) => "clearance.minimum-satisfied",
                (false, false) => "clearance.minimum-not-met",
            }
            .to_owned(),
            severity: if passes {
                DiagnosticSeverity::Information
            } else {
                DiagnosticSeverity::Error
            },
            evidence_class,
            location: DiagnosticLocation {
                entity: None,
                exact_body: None,
                joint: None,
            },
            policy_id: invocation.policy_id.clone(),
            policy_version: invocation.policy_version,
            evidence: format!(
                "left={}; right={}; relation={relation:?}; minimum_gap_mm={gap_mm:.9}; required_mm={:.9}",
                instance_path_label(&case.left.instance_path),
                instance_path_label(&case.right.instance_path),
                case.required_minimum_mm()
            ),
        });
    }
    ValidationReport {
        invocation,
        state: if failed {
            ValidationState::Failed
        } else {
            ValidationState::Passed
        },
        evidence_counts,
        diagnostics,
        assumptions: vec![],
        unresolved_conditions: vec![],
    }
}

fn general_input_evidence(
    cases: &[GeneralClearanceCase],
    tolerance: TolerancePolicy,
) -> EvidenceClass {
    EvidenceClass::weakest(
        cases
            .iter()
            .flat_map(|case| [&case.left.evidence_class, &case.right.evidence_class]),
        general_tolerant_evidence(tolerance),
    )
}

fn general_tolerant_evidence(tolerance: TolerancePolicy) -> TolerantEvidence {
    TolerantEvidence::new(
        tolerance.epsilon_mm(),
        GENERAL_BODY_OBB_NARROW_PHASE_METHOD_V1,
        PermittedErrorDirection::FalsePositiveOnly,
    )
    .expect("the general-body tolerance and method identity are valid")
}

fn general_body_geometry_evidence(
    transform: Transform,
    vertices: &[[f64; 3]],
) -> Result<GeneralBodyGeometryEvidence, GeneralBodyValidationError> {
    let first = vertices
        .first()
        .copied()
        .ok_or(GeneralBodyValidationError::InvalidGeometry)?;
    let mut minimum = first;
    let mut maximum = first;
    for vertex in &vertices[1..] {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(vertex[axis]);
            maximum[axis] = maximum[axis].max(vertex[axis]);
        }
    }
    let source_frame_extents_mm = std::array::from_fn(|axis| maximum[axis] - minimum[axis]);
    if source_frame_extents_mm
        .iter()
        .any(|extent| !extent.is_finite() || *extent <= 0.0)
    {
        return Err(GeneralBodyValidationError::InvalidGeometry);
    }
    let matrix = transform.matrix();
    let source_frame_center: [f64; 3] =
        std::array::from_fn(|axis| f64::midpoint(minimum[axis], maximum[axis]));
    let source_frame_center_world_mm = [
        matrix[0] * source_frame_center[0]
            + matrix[1] * source_frame_center[1]
            + matrix[2] * source_frame_center[2]
            + matrix[3],
        matrix[4] * source_frame_center[0]
            + matrix[5] * source_frame_center[1]
            + matrix[6] * source_frame_center[2]
            + matrix[7],
        matrix[8] * source_frame_center[0]
            + matrix[9] * source_frame_center[1]
            + matrix[10] * source_frame_center[2]
            + matrix[11],
    ];
    let mut source_axis_world_direction = [[0.0; 3]; 3];
    let mut source_axis_world_scale = [0.0; 3];
    let mut source_axis_world_z_alignment = [0.0; 3];
    for axis in 0..3 {
        let direction = [matrix[axis], matrix[4 + axis], matrix[8 + axis]];
        let length = direction
            .into_iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        if !length.is_finite() || length <= 0.0 {
            return Err(GeneralBodyValidationError::InvalidGeometry);
        }
        source_axis_world_direction[axis] = direction.map(|value| value / length);
        source_axis_world_scale[axis] = length;
        source_axis_world_z_alignment[axis] = source_axis_world_direction[axis][2].abs();
    }
    Ok(GeneralBodyGeometryEvidence {
        source_frame_extents_mm,
        source_frame_center_world_mm,
        source_axis_world_direction,
        source_axis_world_scale,
        source_axis_world_z_alignment,
    })
}

fn transformed_body_bounds(
    transform: Transform,
    vertices: &[[f64; 3]],
) -> Result<Aabb, GeneralBodyValidationError> {
    let first = vertices
        .first()
        .copied()
        .ok_or(GeneralBodyValidationError::InvalidGeometry)?;
    let mut minimum = transform_body_point(transform, first);
    let mut maximum = minimum;
    for vertex in &vertices[1..] {
        let point = transform_body_point(transform, *vertex);
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    Aabb::bounded_volume(minimum, maximum).map_err(|_| GeneralBodyValidationError::InvalidGeometry)
}

fn transform_body_point(transform: Transform, point: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
        matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
    ]
}

fn is_translation_only(transform: Transform) -> bool {
    let matrix = transform.matrix();
    matrix[0] == 1.0
        && matrix[5] == 1.0
        && matrix[10] == 1.0
        && matrix[15] == 1.0
        && [1, 2, 4, 6, 8, 9, 12, 13, 14]
            .into_iter()
            .all(|index| matrix[index] == 0.0)
}

fn is_axis_aligned_rectangle_profile(points_mm: &[[f64; 2]]) -> bool {
    points_mm.len() == 4
        && points_mm[0][1] == points_mm[1][1]
        && points_mm[1][0] == points_mm[2][0]
        && points_mm[2][1] == points_mm[3][1]
        && points_mm[3][0] == points_mm[0][0]
        && points_mm[1][0] > points_mm[0][0]
        && points_mm[3][1] > points_mm[0][1]
}

fn canonical_extrusion_geometry_digest(points_mm: &[[f64; 2]], height_mm: f64) -> String {
    let mut bytes = Vec::new();
    push_bytes(&mut bytes, b"ketchup.canonical-profile-extrusion.v1");
    bytes.extend_from_slice(&(points_mm.len() as u64).to_le_bytes());
    for point in points_mm {
        for coordinate in point {
            bytes.extend_from_slice(&coordinate.to_bits().to_le_bytes());
        }
    }
    bytes.extend_from_slice(&height_mm.to_bits().to_le_bytes());
    sha256_hex(&bytes)
}

fn mesh_geometry_digest(spec: &crate::document::MeshBodySpec) -> String {
    let mut bytes = Vec::new();
    push_bytes(&mut bytes, spec.schema.as_bytes());
    bytes.extend_from_slice(&(spec.vertices_mm.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(spec.triangles.len() as u64).to_le_bytes());
    for vertex in &spec.vertices_mm {
        for coordinate in vertex {
            bytes.extend_from_slice(&coordinate.to_bits().to_le_bytes());
        }
    }
    for triangle in &spec.triangles {
        for index in triangle {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
    }
    sha256_hex(&bytes)
}

fn push_general_participant(output: &mut Vec<u8>, participant: &GeneralBodyParticipant) {
    push_instance_path(output, &participant.instance_path);
    match &participant.source {
        GeneralBodySource::Exact(key) => {
            output.push(0);
            output.extend_from_slice(&key.document_id.0.to_le_bytes());
            output.extend_from_slice(&key.source_revision.to_le_bytes());
            push_bytes(output, key.source_digest.as_bytes());
            output.extend_from_slice(&key.definition_id.0.to_le_bytes());
            output.extend_from_slice(&key.producer_feature_id.0.to_le_bytes());
            push_bytes(output, key.canonical_input_digest.as_bytes());
            push_bytes(output, key.exact_input_digest.as_bytes());
            push_bytes(output, key.evaluator.as_bytes());
            push_bytes(output, key.backend.as_bytes());
            push_bytes(output, key.tolerance.as_bytes());
            push_bytes(output, key.schema.as_bytes());
            push_bytes(output, key.result_fingerprint.as_bytes());
        }
        GeneralBodySource::CanonicalMesh {
            definition_id,
            feature_id,
            geometry_digest,
        } => {
            output.push(1);
            output.extend_from_slice(&definition_id.0.to_le_bytes());
            output.extend_from_slice(&feature_id.0.to_le_bytes());
            push_bytes(output, geometry_digest.as_bytes());
        }
        GeneralBodySource::CanonicalExtrusion {
            definition_id,
            profile_id,
            extrusion_id,
            geometry_digest,
        } => {
            output.push(2);
            output.extend_from_slice(&definition_id.0.to_le_bytes());
            output.extend_from_slice(&profile_id.0.to_le_bytes());
            output.extend_from_slice(&extrusion_id.0.to_le_bytes());
            push_bytes(output, geometry_digest.as_bytes());
        }
        GeneralBodySource::CanonicalExactGraph {
            definition_id,
            producer_feature_id,
            graph_digest,
        } => {
            output.push(3);
            output.extend_from_slice(&definition_id.0.to_le_bytes());
            output.extend_from_slice(&producer_feature_id.0.to_le_bytes());
            push_bytes(output, graph_digest.as_bytes());
        }
    }
    push_aabb(output, participant.bounds);
    for extent in participant.geometry_evidence.source_frame_extents_mm {
        output.extend_from_slice(&extent.to_bits().to_le_bytes());
    }
    for coordinate in participant.geometry_evidence.source_frame_center_world_mm {
        output.extend_from_slice(&coordinate.to_bits().to_le_bytes());
    }
    for direction in participant.geometry_evidence.source_axis_world_direction {
        for coordinate in direction {
            output.extend_from_slice(&coordinate.to_bits().to_le_bytes());
        }
    }
    for alignment in participant.geometry_evidence.source_axis_world_z_alignment {
        output.extend_from_slice(&alignment.to_bits().to_le_bytes());
    }
    match &participant.evidence_class {
        EvidenceClass::Exact => output.push(0),
        EvidenceClass::Tolerant(evidence) => {
            output.push(1);
            output.extend_from_slice(&evidence.applied_threshold_mm().to_bits().to_le_bytes());
            push_bytes(output, evidence.method_identity.as_bytes());
            output.push(match evidence.permitted_error_direction {
                PermittedErrorDirection::FalsePositiveOnly => 0,
                PermittedErrorDirection::FalseNegativeOnly => 1,
                PermittedErrorDirection::BidirectionalBounded => 2,
            });
        }
    }
}

fn instance_path_label(path: &InstancePath) -> String {
    let mut label = format!("occurrence:{}", path.root_occurrence().0);
    for step in path.steps() {
        match step {
            InstancePathStep::Group(id) => label.push_str(&format!("/group:{}", id.0)),
            InstancePathStep::Occurrence(id) => {
                label.push_str(&format!("/occurrence:{}", id.0));
            }
        }
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gravity_participant(
        occurrence_id: u64,
        minimum: [f64; 3],
        maximum: [f64; 3],
        explicitly_grounded: bool,
    ) -> GravitySupportParticipant {
        GravitySupportParticipant::new(
            GeneralBodyParticipant {
                instance_path: InstancePath::root(crate::document::OccurrenceId(occurrence_id)),
                source: GeneralBodySource::CanonicalMesh {
                    definition_id: DefinitionId(occurrence_id),
                    feature_id: FeatureId(occurrence_id),
                    geometry_digest: format!("geometry-{occurrence_id}"),
                },
                bounds: Aabb::bounded_volume(minimum, maximum).unwrap(),
                geometry_evidence: GeneralBodyGeometryEvidence {
                    source_frame_extents_mm: std::array::from_fn(|axis| {
                        maximum[axis] - minimum[axis]
                    }),
                    source_frame_center_world_mm: std::array::from_fn(|axis| {
                        f64::midpoint(minimum[axis], maximum[axis])
                    }),
                    source_axis_world_direction: [
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [0.0, 0.0, 1.0],
                    ],
                    source_axis_world_scale: [1.0; 3],
                    source_axis_world_z_alignment: [0.0, 0.0, 1.0],
                },
                evidence_class: EvidenceClass::Exact,
            },
            "main",
            explicitly_grounded,
        )
    }

    #[test]
    fn gravity_support_propagates_from_explicit_grounding() {
        let snapshot = crate::document::DocumentStore::new().current();
        let participants = vec![
            gravity_participant(1, [0.0, 0.0, 0.0], [100.0, 100.0, 10.0], true),
            gravity_participant(2, [10.0, 10.0, 10.0], [90.0, 90.0, 20.0], false),
            gravity_participant(3, [20.0, 20.0, 20.0], [80.0, 80.0, 30.0], false),
            gravity_participant(4, [200.0, 0.0, 50.0], [220.0, 20.0, 70.0], false),
            gravity_participant(5, [300.0, 0.0, 100.0], [320.0, 20.0, 120.0], true),
        ];
        let validator_input = GravitySupportInput::new(participants, [0.0, 0.0, -9.81]).unwrap();
        let validator = BuiltinGravitySupportValidator::default();
        assert_eq!(
            validator.descriptor().implementation_id,
            "ketchup.builtin.gravity-support.obb-sat-cpu-f64.v1"
        );
        let policy = gravity_support_validation_policy();
        let input = gravity_support_input_bytes(&validator_input);
        let invocation = ValidationInvocation::bind(
            &snapshot,
            validator.descriptor(),
            &policy,
            Vec::new(),
            &input,
        );

        let report = validator.invoke(ValidationExecution {
            snapshot: &snapshot,
            invocation,
            policy: &policy,
            input: &validator_input,
        });

        assert_eq!(report.state, ValidationState::Failed);
        assert_eq!(report.diagnostics[0].code, "gravity.supported-explicit");
        assert_eq!(report.diagnostics[1].code, "gravity.supported-contact");
        assert_eq!(report.diagnostics[2].code, "gravity.supported-contact");
        assert_eq!(report.diagnostics[3].code, "gravity.unsupported");
        assert_eq!(report.diagnostics[4].code, "gravity.supported-explicit");
    }

    #[test]
    fn clearance_classifies_gap_touch_and_intersection() {
        let body = Aabb::bounded_volume([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]).unwrap();
        let separated = Aabb::bounded_volume([13.0, 14.0, 0.0], [20.0, 20.0, 10.0]).unwrap();
        let touching = Aabb::bounded_volume([10.0, 2.0, 2.0], [20.0, 8.0, 8.0]).unwrap();
        let intersecting = Aabb::bounded_volume([9.0, 2.0, 2.0], [20.0, 8.0, 8.0]).unwrap();

        assert_eq!(
            clearance(body, separated),
            (ExactClearanceRelation::Separated, 5.0)
        );
        assert_eq!(
            clearance(body, touching),
            (ExactClearanceRelation::Touching, 0.0)
        );
        assert_eq!(
            clearance(body, intersecting),
            (ExactClearanceRelation::Intersecting, 0.0)
        );
    }
}
