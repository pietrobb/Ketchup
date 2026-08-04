use crate::document::{DocumentId, Snapshot};
use crate::graph::{DerivedIdentity, sha256_hex};
use crate::prismatic::{
    Aabb, CanonicalJoint, JointId, JointValidationOutcome, TolerancePolicy, validate_joint_overlap,
};

pub const VALIDATOR_PROTOCOL_V1: &str = "ketchup.validator-protocol.v1";
pub const DIAGNOSTIC_SCHEMA_V1: &str = "ketchup.validation-diagnostic.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ValidationClass {
    CanonicalInvariant,
    Collision,
    DeclaredJoint,
    StructuralBestEffort,
    Manufacturability,
    Advisory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ReadScope {
    CanonicalGraph,
    DerivedGeometry,
    DeclaredJoints,
    Materials,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceLimits {
    pub maximum_input_bytes: u64,
    pub maximum_work_units: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorDescriptor {
    pub contract_id: String,
    pub contract_version: u32,
    pub implementation_id: String,
    pub implementation_version: String,
    pub input_schema: String,
    pub validation_class: ValidationClass,
    pub read_scopes: Vec<ReadScope>,
    pub deterministic: bool,
    pub limits: ResourceLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyRequirement {
    Required,
    Optional,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicySeverity {
    Advisory,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoverningStandardRef {
    pub standard: String,
    pub jurisdiction: String,
    pub edition: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationPolicyRef {
    pub policy_id: String,
    pub policy_version: u32,
    pub contract_id: String,
    pub contract_version: u32,
    pub requirement: PolicyRequirement,
    pub severity: PolicySeverity,
    pub blocks_release: bool,
    pub governing_standard: Option<GoverningStandardRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationInvocation {
    pub protocol: &'static str,
    pub document_id: DocumentId,
    pub revision_id: u64,
    pub canonical_digest: String,
    pub accepted_derived_results: Vec<DerivedIdentity>,
    pub contract_id: String,
    pub contract_version: u32,
    pub implementation_id: String,
    pub implementation_version: String,
    pub validation_class: ValidationClass,
    pub read_scopes: Vec<ReadScope>,
    pub deterministic: bool,
    pub resource_limits: ResourceLimits,
    pub policy_id: String,
    pub policy_version: u32,
    pub policy_severity: PolicySeverity,
    pub governing_standard: Option<GoverningStandardRef>,
    pub input_schema: String,
    pub diagnostic_schema: &'static str,
    pub input_digest: String,
}

impl ValidationInvocation {
    #[must_use]
    pub fn bind(
        snapshot: &Snapshot,
        descriptor: &ValidatorDescriptor,
        policy: &ValidationPolicyRef,
        mut accepted_derived_results: Vec<DerivedIdentity>,
        input: &[u8],
    ) -> Self {
        accepted_derived_results.sort();
        accepted_derived_results.dedup();
        Self {
            protocol: VALIDATOR_PROTOCOL_V1,
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            accepted_derived_results,
            contract_id: descriptor.contract_id.clone(),
            contract_version: descriptor.contract_version,
            implementation_id: descriptor.implementation_id.clone(),
            implementation_version: descriptor.implementation_version.clone(),
            validation_class: descriptor.validation_class,
            read_scopes: descriptor.read_scopes.clone(),
            deterministic: descriptor.deterministic,
            resource_limits: descriptor.limits,
            policy_id: policy.policy_id.clone(),
            policy_version: policy.policy_version,
            policy_severity: policy.severity,
            governing_standard: policy.governing_standard.clone(),
            input_schema: descriptor.input_schema.clone(),
            diagnostic_schema: DIAGNOSTIC_SCHEMA_V1,
            input_digest: sha256_hex(input),
        }
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.revision_id == snapshot.revision_id()
            && self.canonical_digest == snapshot.canonical_digest()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationState {
    Passed,
    Failed,
    NotEvaluated,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Information,
    Advisory,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticLocation {
    pub entity: Option<DerivedIdentity>,
    pub joint: Option<JointId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationDiagnostic {
    pub schema: &'static str,
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub location: DiagnosticLocation,
    pub policy_id: String,
    pub policy_version: u32,
    pub evidence: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationReport {
    pub invocation: ValidationInvocation,
    pub state: ValidationState,
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub assumptions: Vec<String>,
    pub unresolved_conditions: Vec<String>,
}

impl ValidationReport {
    #[must_use]
    pub fn unavailable(invocation: ValidationInvocation, evidence: impl Into<String>) -> Self {
        let diagnostic = ValidationDiagnostic {
            schema: DIAGNOSTIC_SCHEMA_V1,
            code: "validator.unavailable".to_owned(),
            severity: DiagnosticSeverity::Error,
            location: DiagnosticLocation {
                entity: None,
                joint: None,
            },
            policy_id: invocation.policy_id.clone(),
            policy_version: invocation.policy_version,
            evidence: evidence.into(),
        };
        Self {
            invocation,
            state: ValidationState::Unavailable,
            diagnostics: vec![diagnostic],
            assumptions: vec![],
            unresolved_conditions: vec![],
        }
    }

    #[must_use]
    pub fn not_evaluated(invocation: ValidationInvocation, evidence: impl Into<String>) -> Self {
        let evidence = evidence.into();
        let policy_id = invocation.policy_id.clone();
        let policy_version = invocation.policy_version;
        Self {
            invocation,
            state: ValidationState::NotEvaluated,
            diagnostics: vec![ValidationDiagnostic {
                schema: DIAGNOSTIC_SCHEMA_V1,
                code: "validator.not-evaluated".to_owned(),
                severity: DiagnosticSeverity::Warning,
                location: DiagnosticLocation {
                    entity: None,
                    joint: None,
                },
                policy_id,
                policy_version,
                evidence: evidence.clone(),
            }],
            assumptions: vec![],
            unresolved_conditions: vec![evidence],
        }
    }
}

pub struct ValidationExecution<'a, Input: ?Sized> {
    pub snapshot: &'a Snapshot,
    pub invocation: ValidationInvocation,
    pub policy: &'a ValidationPolicyRef,
    pub input: &'a Input,
}

pub trait HostNeutralValidator<Input: ?Sized> {
    fn descriptor(&self) -> &ValidatorDescriptor;

    fn invoke(&self, execution: ValidationExecution<'_, Input>) -> ValidationReport;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyDecision {
    pub state: ValidationState,
    pub blocks_release: bool,
}

#[must_use]
pub fn decide(policy: &ValidationPolicyRef, report: &ValidationReport) -> PolicyDecision {
    let contract_matches = report.invocation.contract_id == policy.contract_id
        && report.invocation.contract_version == policy.contract_version
        && report.invocation.policy_id == policy.policy_id
        && report.invocation.policy_version == policy.policy_version;
    let state = if contract_matches {
        report.state
    } else {
        ValidationState::NotEvaluated
    };
    let required_not_passed = policy.requirement == PolicyRequirement::Required
        && matches!(
            state,
            ValidationState::Failed | ValidationState::NotEvaluated | ValidationState::Unavailable
        );
    PolicyDecision {
        state,
        blocks_release: policy.blocks_release && required_not_passed,
    }
}

pub const PRISMATIC_VALIDATOR_CONTRACT_V1: &str = "ketchup.validator.prismatic-joints.v1";
pub const PRISMATIC_VALIDATOR_IMPLEMENTATION_V1: &str =
    "ketchup.builtin.prismatic-joints.cpu-f64.v1";
pub const PRISMATIC_VALIDATOR_INPUT_V1: &str = "ketchup.prismatic-joint-input.v1";
pub const BEAM_VALIDATION_POLICY_V1: &str = "ketchup.policy.beam-fabrication.v1";

#[must_use]
pub fn prismatic_validator_descriptor() -> ValidatorDescriptor {
    ValidatorDescriptor {
        contract_id: PRISMATIC_VALIDATOR_CONTRACT_V1.to_owned(),
        contract_version: 1,
        implementation_id: PRISMATIC_VALIDATOR_IMPLEMENTATION_V1.to_owned(),
        implementation_version: "1.0.0".to_owned(),
        input_schema: PRISMATIC_VALIDATOR_INPUT_V1.to_owned(),
        validation_class: ValidationClass::DeclaredJoint,
        read_scopes: vec![ReadScope::DerivedGeometry, ReadScope::DeclaredJoints],
        deterministic: true,
        limits: ResourceLimits {
            maximum_input_bytes: 16 * 1024 * 1024,
            maximum_work_units: 1_000_000,
        },
    }
}

#[must_use]
pub fn beam_validation_policy() -> ValidationPolicyRef {
    ValidationPolicyRef {
        policy_id: BEAM_VALIDATION_POLICY_V1.to_owned(),
        policy_version: 1,
        contract_id: PRISMATIC_VALIDATOR_CONTRACT_V1.to_owned(),
        contract_version: 1,
        requirement: PolicyRequirement::Required,
        severity: PolicySeverity::Error,
        blocks_release: true,
        governing_standard: None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrismaticJointCase {
    pub left_identity: DerivedIdentity,
    pub right_identity: DerivedIdentity,
    pub left_bounds: Aabb,
    pub right_bounds: Aabb,
    pub declared_joint: Option<CanonicalJoint>,
}

#[must_use]
pub fn prismatic_input_bytes(cases: &[PrismaticJointCase], tolerance: TolerancePolicy) -> Vec<u8> {
    let mut input = Vec::new();
    push_bytes(&mut input, PRISMATIC_VALIDATOR_INPUT_V1.as_bytes());
    push_bytes(&mut input, tolerance.id().as_bytes());
    input.extend_from_slice(&tolerance.epsilon_mm().to_bits().to_le_bytes());
    input.extend_from_slice(&(cases.len() as u64).to_le_bytes());
    for case in cases {
        push_identity(&mut input, &case.left_identity);
        push_identity(&mut input, &case.right_identity);
        push_aabb(&mut input, case.left_bounds);
        push_aabb(&mut input, case.right_bounds);
        if let Some(joint) = &case.declared_joint {
            input.push(1);
            input.extend_from_slice(&joint.id().0.to_le_bytes());
            push_identity(&mut input, joint.participant_a());
            push_identity(&mut input, joint.participant_b());
            push_aabb(&mut input, joint.volume());
        } else {
            input.push(0);
        }
    }
    input
}

fn prismatic_input_len(cases: &[PrismaticJointCase], tolerance: TolerancePolicy) -> Option<u64> {
    let mut length = 0;
    checked_add_bytes(&mut length, PRISMATIC_VALIDATOR_INPUT_V1.as_bytes())?;
    checked_add_bytes(&mut length, tolerance.id().as_bytes())?;
    checked_add(&mut length, 16)?;
    for case in cases {
        checked_add_identity(&mut length, &case.left_identity)?;
        checked_add_identity(&mut length, &case.right_identity)?;
        checked_add(&mut length, 97)?;
        if let Some(joint) = &case.declared_joint {
            checked_add(&mut length, 56)?;
            checked_add_identity(&mut length, joint.participant_a())?;
            checked_add_identity(&mut length, joint.participant_b())?;
        }
    }
    Some(length)
}

fn checked_add(length: &mut u64, amount: u64) -> Option<()> {
    *length = length.checked_add(amount)?;
    Some(())
}

fn checked_add_bytes(length: &mut u64, value: &[u8]) -> Option<()> {
    checked_add(length, 8)?;
    checked_add(length, u64::try_from(value.len()).ok()?)
}

fn checked_add_identity(length: &mut u64, identity: &DerivedIdentity) -> Option<()> {
    checked_add(length, 16)?;
    for segment in identity.slot_path.segments() {
        checked_add(length, 8)?;
        checked_add_bytes(length, segment.output_port.as_bytes())?;
        checked_add_bytes(length, segment.semantic_key.as_bytes())?;
    }
    Some(())
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value);
}

fn push_identity(output: &mut Vec<u8>, identity: &DerivedIdentity) {
    output.extend_from_slice(&identity.root_rule_node_id.0.to_le_bytes());
    output.extend_from_slice(&(identity.slot_path.segments().len() as u64).to_le_bytes());
    for segment in identity.slot_path.segments() {
        output.extend_from_slice(&segment.producer_rule_id.0.to_le_bytes());
        push_bytes(output, segment.output_port.as_bytes());
        push_bytes(output, segment.semantic_key.as_bytes());
    }
}

fn push_aabb(output: &mut Vec<u8>, bounds: Aabb) {
    for value in bounds.min().into_iter().chain(bounds.max()) {
        output.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}

pub struct BuiltinPrismaticValidator {
    descriptor: ValidatorDescriptor,
    tolerance: TolerancePolicy,
}

impl BuiltinPrismaticValidator {
    #[must_use]
    pub fn new(tolerance: TolerancePolicy) -> Self {
        Self {
            descriptor: prismatic_validator_descriptor(),
            tolerance,
        }
    }

    #[must_use]
    pub fn with_limits(tolerance: TolerancePolicy, limits: ResourceLimits) -> Self {
        let mut descriptor = prismatic_validator_descriptor();
        descriptor.limits = limits;
        Self {
            descriptor,
            tolerance,
        }
    }
}

impl Default for BuiltinPrismaticValidator {
    fn default() -> Self {
        Self::new(TolerancePolicy::default())
    }
}

impl HostNeutralValidator<[PrismaticJointCase]> for BuiltinPrismaticValidator {
    fn descriptor(&self) -> &ValidatorDescriptor {
        &self.descriptor
    }

    fn invoke(&self, execution: ValidationExecution<'_, [PrismaticJointCase]>) -> ValidationReport {
        let work_units = u64::try_from(execution.input.len()).unwrap_or(u64::MAX);
        if work_units > self.descriptor.limits.maximum_work_units {
            return ValidationReport::not_evaluated(
                execution.invocation,
                "validator input exceeds its declared work envelope",
            );
        }
        let Some(input_len) = prismatic_input_len(execution.input, self.tolerance) else {
            return ValidationReport::not_evaluated(
                execution.invocation,
                "validator input byte length overflows its declared envelope",
            );
        };
        if input_len > self.descriptor.limits.maximum_input_bytes {
            return ValidationReport::not_evaluated(
                execution.invocation,
                "validator input exceeds its declared byte envelope",
            );
        }

        let input_bytes = prismatic_input_bytes(execution.input, self.tolerance);
        debug_assert_eq!(input_bytes.len() as u64, input_len);
        let mut accepted = execution
            .input
            .iter()
            .flat_map(|case| [case.left_identity.clone(), case.right_identity.clone()])
            .collect::<Vec<_>>();
        accepted.sort();
        accepted.dedup();
        let descriptor_matches = execution.invocation.protocol == VALIDATOR_PROTOCOL_V1
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
        } else if execution.invocation.accepted_derived_results != accepted {
            Some("accepted derived-result identity set does not match validator input")
        } else if execution.invocation.input_digest != sha256_hex(&input_bytes) {
            Some("validator input digest does not match the supplied input")
        } else {
            None
        };
        if let Some(reason) = reason {
            return ValidationReport::not_evaluated(execution.invocation, reason);
        }
        evaluate_prismatic_joints(execution.invocation, execution.input, self.tolerance)
    }
}

fn evaluate_prismatic_joints(
    invocation: ValidationInvocation,
    cases: &[PrismaticJointCase],
    tolerance: TolerancePolicy,
) -> ValidationReport {
    let mut diagnostics = Vec::new();
    for case in cases {
        if let Some(joint) = &case.declared_joint
            && !joint.connects(&case.left_identity, &case.right_identity)
        {
            diagnostics.push(ValidationDiagnostic {
                schema: DIAGNOSTIC_SCHEMA_V1,
                code: "joint.participant-mismatch".to_owned(),
                severity: DiagnosticSeverity::Error,
                location: DiagnosticLocation {
                    entity: Some(case.right_identity.clone()),
                    joint: Some(joint.id()),
                },
                policy_id: invocation.policy_id.clone(),
                policy_version: invocation.policy_version,
                evidence: "declared joint does not connect the evaluated participant identities"
                    .to_owned(),
            });
            continue;
        }
        let outcome = match validate_joint_overlap(
            case.left_bounds,
            case.right_bounds,
            case.declared_joint.as_ref(),
            tolerance,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                return ValidationReport::not_evaluated(
                    invocation,
                    format!("deterministic prismatic evaluation failed: {error}"),
                );
            }
        };
        let Some(outcome) = outcome else {
            continue;
        };
        let (code, severity, evidence) = match outcome {
            JointValidationOutcome::OverlapInsideDeclaredJointOk => continue,
            JointValidationOutcome::OverlapOutsideDeclaredJointError => (
                "joint.overlap-outside-declared-volume",
                DiagnosticSeverity::Error,
                "penetration vertices exceed the Euclidean tolerance region of the declared joint",
            ),
            JointValidationOutcome::OverlapWithoutJointError => (
                "collision.undeclared-penetration",
                DiagnosticSeverity::Error,
                "penetration exists without a declared joint",
            ),
            JointValidationOutcome::DeclaredJointWithEmptyIntersectionError => (
                "joint.declared-without-intersection",
                DiagnosticSeverity::Error,
                "declared joint has no required penetrating intersection",
            ),
        };
        diagnostics.push(ValidationDiagnostic {
            schema: DIAGNOSTIC_SCHEMA_V1,
            code: code.to_owned(),
            severity,
            location: DiagnosticLocation {
                entity: Some(case.right_identity.clone()),
                joint: case.declared_joint.as_ref().map(CanonicalJoint::id),
            },
            policy_id: invocation.policy_id.clone(),
            policy_version: invocation.policy_version,
            evidence: evidence.to_owned(),
        });
    }
    ValidationReport {
        invocation,
        state: if diagnostics.is_empty() {
            ValidationState::Passed
        } else {
            ValidationState::Failed
        },
        diagnostics,
        assumptions: vec![],
        unresolved_conditions: vec![],
    }
}
