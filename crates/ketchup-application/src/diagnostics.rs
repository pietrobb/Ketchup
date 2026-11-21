use ketchup_assistant::sidecar::{AssistantRejectionDiagnostic, AssistantRejectionPhase};
use ketchup_model::document::{CanonicalError, FeatureInputError};

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
        causes: Vec::new(),
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
    if let CanonicalError::InvalidFeatureInput(input) = error {
        let entity = input
            .feature
            .map_or_else(|| target.to_owned(), |id| format!("feature:{}", id.0));
        return assistant_feature_input_rejection(input, operation, &entity);
    }
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

/// A refused feature input: the target is the field of `entity` (the feature,
/// or the operation of the batch that made it) and the hint the field's own
/// repair, not a general one (R1121ac P3 of EXA-901).
pub fn assistant_feature_input_rejection(
    input: FeatureInputError,
    operation: &str,
    entity: &str,
) -> AssistantRejection {
    assistant_rejection(
        AssistantRejectionPhase::CanonicalValidation,
        CanonicalError::InvalidFeatureInput(input).code(),
        operation,
        format!("{entity}.{}", input.field),
        CanonicalError::InvalidFeatureInput(input).to_string(),
        format!("Change {}: {}.", input.field, input.rule),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::document::{FeatureId, FeatureInputField};

    #[test]
    fn a_refused_feature_input_names_its_feature_field_and_repair() {
        let input = FeatureInputError {
            feature: Some(FeatureId(620)),
            field: FeatureInputField::Weight,
            rule: "a weight must lie between 0.000001 and 1000000",
        };
        let rejection = assistant_canonical_rejection(
            CanonicalError::InvalidFeatureInput(input),
            "edit",
            "document:1",
        );
        assert_eq!(rejection.target, "feature:620.weight");
        assert_eq!(
            rejection.repair_hint,
            "Change weight: a weight must lie between 0.000001 and 1000000."
        );
        assert!(
            rejection
                .failed_invariant
                .starts_with("feature 620 input weight")
        );
    }
}
