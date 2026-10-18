//! A GUI refusal is the one typed rejection (AGENTS.md §2). Its code is the
//! message-catalog key, so the GUI and its tests tell refusals apart by kind;
//! its reason is that message in the user's language; a failure from a lower
//! layer stays attached as the cause.

use crate::length_input::{DecimalSeparator, Refusal, read_values};
use eframe::egui;
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

    /// The field whose rule the message `key` states refuses its text for
    /// the reason `refusal` gives, both in the user's language.
    fn field_refusal(&self, key: &'static str, refusal: Refusal) -> Rejection;
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

    fn field_refusal(&self, key: &'static str, refusal: Refusal) -> Rejection {
        Rejection::new(key, RejectionPhase::Validation)
            .reason(self.text_because(key, self.text(refusal.message_key())))
            .caused_by(refusal)
    }
}

/// Under a dialog's value field, why its text is refused when that is not
/// plain to see: a comma where the decimal separator is a point. True when
/// it showed one.
pub(crate) fn show_value_refusal(
    ui: &mut egui::Ui,
    catalog: &LocaleCatalog,
    text: &str,
    decimal: DecimalSeparator,
) -> bool {
    let refused = read_values(text, decimal) == Err(Refusal::Comma);
    if refused {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            catalog.text(Refusal::Comma.message_key()),
        );
    }
    refused
}

/// A lower-layer failure shown as it is; `code` names the step that failed.
pub(crate) fn failed(code: &'static str, cause: impl Error + Send + Sync + 'static) -> Rejection {
    Rejection::new(code, RejectionPhase::Planning)
        .reason(cause.to_string())
        .caused_by(cause)
}
