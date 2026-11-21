//! Refusals of a CAD edit program that name the operation and the field they
//! are about (R1121ac P3 of EXA-901).

use super::*;
use crate::diagnostics::assistant_feature_input_rejection;
use ketchup_assistant::request_invalid::{AssistantRequestInvalid, AssistantRequestProblem};

/// A refused CAD edit program: an error inside one operation names the
/// operation and the field, and a broken rule is its own repair (R1121ac P3).
pub(super) fn program_invalid_rejection(
    error: &AssistantRequestInvalid,
    document_target: &str,
) -> AssistantRejection {
    let target = match (error.operation(), error.item_name()) {
        (Some(index), Some(item)) => format!("operation:{index}.{item}"),
        (Some(index), None) => format!("operation:{index}"),
        (None, _) => document_target.to_owned(),
    };
    let repair_hint = match (error.problem(), error.item_name()) {
        (AssistantRequestProblem::Violates(rule), Some(item)) => format!("Change {item}: {rule}."),
        _ => "Return a bounded CAD edit program that satisfies the Assistant schema invariants."
            .to_owned(),
    };
    assistant_rejection(
        AssistantRejectionPhase::IntentValidation,
        "intent.cad_edit_program_invalid",
        "cad_edit_program",
        target,
        error.to_string(),
        repair_hint,
        true,
    )
}

/// `error` of the batch so far: a refused input of a feature an operation of the
/// program made names that operation, which the program knows, and its field.
pub(super) fn batch_rejection(
    feature_operations: &BTreeMap<FeatureId, usize>,
    error: CanonicalError,
    operation: &str,
    target: &str,
) -> AssistantRejection {
    if let CanonicalError::InvalidFeatureInput(input) = error
        && let Some(index) = input
            .feature
            .and_then(|feature| feature_operations.get(&feature))
    {
        return assistant_feature_input_rejection(input, operation, &format!("operation:{index}"));
    }
    assistant_canonical_rejection(error, operation, target)
}

impl StagedPlanningContext {
    /// `error` of the batch staged so far, as [`batch_rejection`] names it.
    pub(super) fn refusal(
        &self,
        error: CanonicalError,
        operation: &str,
        target: &str,
    ) -> AssistantRejection {
        batch_rejection(&self.feature_operations(), error, operation, target)
    }

    /// The operation of the program that made each feature so far.
    pub(super) fn feature_operations(&self) -> BTreeMap<FeatureId, usize> {
        self.operation_outputs
            .iter()
            .flat_map(|(index, outputs)| {
                [
                    outputs.sketch_feature,
                    outputs.body_feature,
                    outputs.construction_feature,
                ]
                .into_iter()
                .flatten()
                .map(move |feature| (feature, *index))
            })
            .collect()
    }
}
