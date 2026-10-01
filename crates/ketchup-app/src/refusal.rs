//! A GUI refusal is the one typed rejection (AGENTS.md §2). Its code is the
//! message-catalog key, so the GUI and its tests tell refusals apart by kind;
//! its reason is that message in the user's language; a failure from a lower
//! layer stays attached as the cause.

use ketchup_interaction::LocaleCatalog;
use ketchup_rejection::{Rejection, RejectionPhase};
use std::collections::BTreeMap;
use std::error::Error;

pub(crate) trait Refuse {
    /// The request breaks the rule the catalog message `key` states.
    fn refusal(&self, key: &'static str) -> Rejection;

    /// As [`Refuse::refusal`] for a message with arguments.
    fn refusal_with(&self, key: &'static str, arguments: &BTreeMap<&str, String>) -> Rejection;

    /// The message `key` followed by the cause, which stays attached.
    fn refusal_because(
        &self,
        key: &'static str,
        cause: impl Error + Send + Sync + 'static,
    ) -> Rejection;
}

impl Refuse for LocaleCatalog {
    fn refusal(&self, key: &'static str) -> Rejection {
        Rejection::new(key, RejectionPhase::Validation).reason(self.text(key))
    }

    fn refusal_with(&self, key: &'static str, arguments: &BTreeMap<&str, String>) -> Rejection {
        Rejection::new(key, RejectionPhase::Validation).reason(self.format(key, arguments))
    }

    fn refusal_because(
        &self,
        key: &'static str,
        cause: impl Error + Send + Sync + 'static,
    ) -> Rejection {
        Rejection::new(key, RejectionPhase::Planning)
            .reason(self.text_because(key, &cause))
            .caused_by(cause)
    }
}

/// A lower-layer failure shown as it is; `code` names the step that failed.
pub(crate) fn failed(code: &'static str, cause: impl Error + Send + Sync + 'static) -> Rejection {
    Rejection::new(code, RejectionPhase::Planning)
        .reason(cause.to_string())
        .caused_by(cause)
}

/// A typed-in field that does not parse as what it must be; the parse error stays
/// attached as the cause.
pub(crate) fn invalid_field(
    field: &str,
    requirement: &str,
    cause: impl Error + Send + Sync + 'static,
) -> Rejection {
    Rejection::new("field.invalid", RejectionPhase::Validation)
        .target(field.to_owned())
        .reason(format!("{field} must be {requirement}: {cause}"))
        .caused_by(cause)
}
