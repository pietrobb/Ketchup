//! The one typed rejection of Ketchup (AGENTS.md §2): every refused request says
//! what was refused (`code`), when (`phase`), which field, part or program line
//! (`target`), why (`reason`), what to change (`fix_hint`), and keeps the error
//! that caused it (`source`).

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// The stage of work that refused the request.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionPhase {
    /// The request or file could not be read as its format.
    Request,
    /// The input breaks a rule of its contract.
    Validation,
    /// No edit could be planned for the input.
    Planning,
    /// The geometry could not be evaluated.
    Evaluation,
    /// The evaluated result failed a check.
    Verification,
    /// The result could not be published or saved.
    Commit,
    /// A file, process or connection failed.
    Io,
}

/// Why a request was refused and what to do about it.
#[derive(Clone)]
pub struct Rejection {
    code: Cow<'static, str>,
    phase: RejectionPhase,
    target: String,
    reason: String,
    fix_hint: String,
    source: Option<Arc<dyn Error + Send + Sync>>,
}

impl Rejection {
    /// A rejection with a machine-readable `code` such as `document.parameter.not_found`.
    /// Its reason defaults to the code until [`Rejection::reason`] sets one.
    #[must_use]
    pub fn new(code: impl Into<Cow<'static, str>>, phase: RejectionPhase) -> Self {
        let code = code.into();
        Self {
            reason: code.to_string(),
            code,
            phase,
            target: String::new(),
            fix_hint: String::new(),
            source: None,
        }
    }

    /// The field, part, file or program line the rejection is about.
    #[must_use]
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    /// Why the request was refused, in plain language.
    #[must_use]
    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = reason.into();
        self
    }

    /// A concrete change that makes the request succeed.
    #[must_use]
    pub fn fix_hint(mut self, fix_hint: impl Into<String>) -> Self {
        self.fix_hint = fix_hint.into();
        self
    }

    /// Keeps the error that caused the rejection.
    #[must_use]
    pub fn caused_by(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Arc::new(source));
        self
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub const fn phase(&self) -> RejectionPhase {
        self.phase
    }

    #[must_use]
    pub fn target_name(&self) -> &str {
        &self.target
    }

    #[must_use]
    pub fn reason_text(&self) -> &str {
        &self.reason
    }

    #[must_use]
    pub fn fix_hint_text(&self) -> &str {
        &self.fix_hint
    }

    /// The messages of the causing errors, outermost first.
    #[must_use]
    pub fn causes(&self) -> Vec<String> {
        let mut causes = Vec::new();
        let mut next = self
            .source
            .as_deref()
            .map(|error| error as &(dyn Error + 'static));
        while let Some(error) = next {
            causes.push(error.to_string());
            next = error.source();
        }
        causes
    }
}

impl fmt::Debug for Rejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Rejection")
            .field("code", &self.code)
            .field("phase", &self.phase)
            .field("target", &self.target)
            .field("reason", &self.reason)
            .field("fix_hint", &self.fix_hint)
            .field("causes", &self.causes())
            .finish()
    }
}

impl PartialEq for Rejection {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
            && self.phase == other.phase
            && self.target == other.target
            && self.reason == other.reason
            && self.fix_hint == other.fix_hint
            && self.causes() == other.causes()
    }
}

impl fmt::Display for Rejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code)?;
        if !self.target.is_empty() {
            write!(formatter, " at {}", self.target)?;
        }
        write!(formatter, ": {}", self.reason)?;
        if !self.fix_hint.is_empty() {
            write!(formatter, " Fix: {}", self.fix_hint)?;
        }
        Ok(())
    }
}

impl Error for Rejection {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|error| error as &(dyn Error + 'static))
    }
}

/// The serialized form: the cause chain travels as its messages.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RejectionRecord {
    code: String,
    phase: RejectionPhase,
    target: String,
    reason: String,
    fix_hint: String,
    causes: Vec<String>,
}

/// A cause received as text from another process.
#[derive(Debug)]
struct ReportedCause {
    message: String,
    source: Option<Box<ReportedCause>>,
}

impl fmt::Display for ReportedCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ReportedCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|error| error as &(dyn Error + 'static))
    }
}

impl Serialize for Rejection {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RejectionRecord {
            code: self.code.to_string(),
            phase: self.phase,
            target: self.target.clone(),
            reason: self.reason.clone(),
            fix_hint: self.fix_hint.clone(),
            causes: self.causes(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Rejection {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let record = RejectionRecord::deserialize(deserializer)?;
        let source = record
            .causes
            .into_iter()
            .rev()
            .fold(None, |source, message| {
                Some(Box::new(ReportedCause { message, source }))
            });
        Ok(Self {
            code: Cow::Owned(record.code),
            phase: record.phase,
            target: record.target,
            reason: record.reason,
            fix_hint: record.fix_hint,
            source: source.map(|cause| Arc::new(*cause) as Arc<dyn Error + Send + Sync>),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Outer(std::num::ParseIntError);

    impl fmt::Display for Outer {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("layer thickness is not a number")
        }
    }

    impl Error for Outer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    fn parse_failure() -> Rejection {
        let cause = "12x".parse::<u32>().unwrap_err();
        Rejection::new("document.parameter.not_a_number", RejectionPhase::Request)
            .target("parameters.thickness")
            .reason("The thickness must be a whole number of millimetres.")
            .fix_hint("Write the thickness as digits, for example 18.")
            .caused_by(Outer(cause))
    }

    #[test]
    fn a_rejection_keeps_its_whole_cause_chain() {
        let rejection = parse_failure();
        assert_eq!(
            rejection.causes(),
            [
                "layer thickness is not a number",
                "invalid digit found in string"
            ]
        );
        let source = rejection.source().expect("the cause is kept");
        assert_eq!(source.to_string(), "layer thickness is not a number");
        assert!(source.source().is_some());
    }

    #[test]
    fn display_names_code_target_reason_and_fix() {
        assert_eq!(
            parse_failure().to_string(),
            "document.parameter.not_a_number at parameters.thickness: The thickness must be a \
             whole number of millimetres. Fix: Write the thickness as digits, for example 18."
        );
        assert_eq!(
            Rejection::new("bridge.busy", RejectionPhase::Io).to_string(),
            "bridge.busy: bridge.busy"
        );
    }

    #[test]
    fn json_round_trip_keeps_every_field_and_the_cause_messages() {
        let rejection = parse_failure();
        let json = serde_json::to_value(&rejection).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "code": "document.parameter.not_a_number",
                "phase": "request",
                "target": "parameters.thickness",
                "reason": "The thickness must be a whole number of millimetres.",
                "fix_hint": "Write the thickness as digits, for example 18.",
                "causes": ["layer thickness is not a number", "invalid digit found in string"],
            })
        );
        let received: Rejection = serde_json::from_value(json).unwrap();
        assert_eq!(received, rejection);
        assert_eq!(
            received
                .source()
                .and_then(Error::source)
                .map(ToString::to_string),
            Some("invalid digit found in string".to_owned())
        );
    }

    #[test]
    fn unknown_fields_are_refused() {
        let error = serde_json::from_value::<Rejection>(serde_json::json!({
            "code": "a", "phase": "io", "target": "", "reason": "", "fix_hint": "",
            "causes": [], "hint": "misspelled"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("hint"));
    }
}
