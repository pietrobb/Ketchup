//! Rejections recorded for the client's next error response, each returning its code.

use super::*;

/// Records that `code` failed because of `cause` (kept as the rejection's
/// cause chain under the catalog reason) and returns the code.
pub(super) fn failed_because(
    code: &'static str,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> &'static str {
    record_rejection(&rejected(code).caused_by(cause), json!({}));
    code
}

/// Records why `code` failed for the client and returns the code.
pub(super) fn failure(
    code: &'static str,
    reason: impl Into<String>,
    details: Value,
) -> &'static str {
    record_rejection(&rejected(code).reason(reason), details);
    code
}

/// Records a planner diagnostic as the rejection of `code`: its target and
/// repair hint replace the generic ones of the code.
pub(super) fn planning_failure(
    code: &'static str,
    diagnostic: &AssistantRejectionDiagnostic,
) -> &'static str {
    record_rejection(
        &rejected(code)
            .target(diagnostic.target.clone())
            .reason(diagnostic.failed_invariant.clone())
            .fix_hint(diagnostic.repair_hint.clone()),
        json!({
            "diagnostic_code": diagnostic.code,
            "operation": diagnostic.operation,
            "retryable": diagnostic.retryable,
            "published": false,
        }),
    );
    code
}
