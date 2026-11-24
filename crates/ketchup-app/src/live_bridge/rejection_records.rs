//! Rejections recorded for the client's next error response, each returning its code.

use super::*;

/// Records `rejection` with the request-specific `details` as the result of
/// the next error response. A file another program changed, which is why it was
/// refused, is named as `changed_by_another`: the Connect tab lists no write of
/// the client's there (R1122ac, review §4 P3).
pub(super) fn record_rejection(rejection: &Rejection, mut details: Value) {
    if let (Some(path), Some(object)) = (
        crate::export_bundle::changed_by_another(rejection),
        details.as_object_mut(),
    ) {
        object.insert("changed_by_another".to_owned(), json!(path));
    }
    let mut value = serde_json::to_value(rejection).unwrap_or(Value::Null);
    if let (Some(object), Some(details)) = (value.as_object_mut(), details.as_object())
        && !details.is_empty()
    {
        object.insert("details".to_owned(), Value::Object(details.clone()));
    }
    ERROR_DETAILS.with(|slot| *slot.borrow_mut() = Some(value));
}

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
