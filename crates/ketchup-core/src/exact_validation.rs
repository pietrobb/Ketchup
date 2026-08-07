#![forbid(unsafe_code)]

use crate::document::{InstancePath, InstancePathStep, Snapshot};
use crate::exact_product::{
    BodyResultIdentity, BodySubshapeRef, EXACT_RECTANGLE_EVALUATOR_V1, ExactFaceRole,
    ExactRenderPackage,
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
            ExactFaceRole::CutWest,
            ExactFaceRole::CutEast,
            ExactFaceRole::CutSouth,
            ExactFaceRole::CutNorth,
        ] {
            if package.reference(role).is_none() {
                continue;
            }
            let (axis, outward_sign) = match role {
                ExactFaceRole::Top => (2, 1),
                ExactFaceRole::Bottom => (2, -1),
                ExactFaceRole::East => (0, 1),
                ExactFaceRole::CutWest => (0, -1),
                ExactFaceRole::CutEast => (0, 1),
                ExactFaceRole::CutSouth => (1, -1),
                ExactFaceRole::CutNorth => (1, 1),
                ExactFaceRole::RevolveBottom
                | ExactFaceRole::RevolveBody
                | ExactFaceRole::RevolveShoulder
                | ExactFaceRole::RevolveNeck
                | ExactFaceRole::RevolveMouth
                | ExactFaceRole::ShellOuterBottom
                | ExactFaceRole::ShellOuterBody
                | ExactFaceRole::ShellOuterShoulder
                | ExactFaceRole::ShellOuterNeck
                | ExactFaceRole::ShellRim
                | ExactFaceRole::ShellInnerBottom
                | ExactFaceRole::ShellInnerBody
                | ExactFaceRole::ShellInnerShoulder
                | ExactFaceRole::ShellInnerNeck => {
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

#[cfg(test)]
mod tests {
    use super::*;

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
