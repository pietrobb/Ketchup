use ketchup_core::assistant_sidecar::{AssistantRejectionDiagnostic, AssistantRejectionPhase};
use ketchup_core::document::CanonicalError;

pub type AssistantRejection = Box<AssistantRejectionDiagnostic>;
pub type AssistantPlanningResult<T> = Result<T, AssistantRejection>;

pub fn assistant_rejection(
    phase: AssistantRejectionPhase,
    code: impl Into<String>,
    operation: impl Into<String>,
    target: impl Into<String>,
    failed_invariant: impl Into<String>,
    repair_hint: impl Into<String>,
    retryable: bool,
) -> AssistantRejection {
    let diagnostic = AssistantRejectionDiagnostic {
        phase,
        code: code.into(),
        operation: operation.into(),
        target: target.into(),
        failed_invariant: failed_invariant.into(),
        repair_hint: repair_hint.into(),
        retryable,
    };
    debug_assert_eq!(diagnostic.validate(), Ok(()));
    Box::new(diagnostic)
}

pub fn assistant_planning_rejection(
    code: &'static str,
    operation: &str,
    target: &str,
    failed_invariant: impl Into<String>,
    repair_hint: &'static str,
) -> AssistantRejection {
    assistant_rejection(
        AssistantRejectionPhase::ProposalPlanning,
        code,
        operation,
        target,
        failed_invariant,
        repair_hint,
        true,
    )
}

pub fn assistant_canonical_rejection(
    error: CanonicalError,
    operation: &str,
    target: &str,
) -> AssistantRejection {
    let retryable = !matches!(
        error,
        CanonicalError::IdExhausted | CanonicalError::RevisionExhausted
    );
    assistant_rejection(
        AssistantRejectionPhase::CanonicalValidation,
        error.code(),
        operation,
        target,
        error.to_string(),
        "Revise the target or operation so the reported canonical invariant remains valid.",
        retryable,
    )
}
