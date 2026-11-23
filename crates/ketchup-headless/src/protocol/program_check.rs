//! The exact half of `program_check`: the program built in a scratch document.
use super::*;

impl Server {
    /// Builds the evaluated program in a scratch document, leaving the
    /// session's document untouched, and settles on its exact solids what the
    /// boxes cannot, exactly as `program_apply` and the window do, by the
    /// deadline of the whole check.
    pub(super) fn exact_program_check(
        &self,
        source: &ketchup_model::document::RuleProgramSource,
        evaluated: &ketchup_program::Evaluated,
        report: &mut ketchup_program::Report,
        deadline: CheckDeadline,
    ) -> Option<Value> {
        if ketchup_program::exact_candidates(&evaluated.model).is_empty() {
            return None;
        }
        // The scratch document of a large model takes seconds to build, so it
        // is built once and the build stops between parts at the deadline.
        let past_deadline = || deadline.remaining().is_zero();
        let mut scratch = DocumentSession::new(self.settings.clone());
        let snapshot = match scratch.build_rule_model_scratch(
            evaluated.reference_model(),
            source.clone(),
            &past_deadline,
        ) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                return Some(exact_check_past_deadline(
                    report,
                    &evaluated.model,
                    deadline,
                ));
            }
            Err(error) => {
                let reason = Error::from(error).code;
                return Some(json!({"state": "incomplete",
                "not_evaluated": [{"reason": "program_build_failed", "detail": reason}]}));
            }
        };
        verify_rule_program_exact(
            &snapshot,
            &evaluated.model,
            report,
            scratch.container_data(),
            self.settings.exact_worker_path.clone(),
            deadline,
            Arc::new(AtomicBool::new(false)),
        )
    }
}
