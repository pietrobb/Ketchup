//! Why a bounded Assistant request is refused before it reaches planning.
//!
//! Every schema check of the sidecar answers with this one type: the refused
//! part of the request, an optional named item inside it, what is wrong with
//! it and the refusal underneath, so callers keep the reason typed instead of
//! re-parsing text.

use ketchup_geometry::sketch::SketchError;
use std::fmt;

/// One refused part of an Assistant request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantRequestInvalid {
    subject: &'static str,
    item: Option<String>,
    problem: AssistantRequestProblem,
    cause: Option<Box<AssistantRequestCause>>,
}

/// What is wrong with the refused part.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssistantRequestProblem {
    /// It breaks a schema rule of its kind.
    Invalid,
    /// It breaks the named rule.
    Violates(&'static str),
    /// Its identifier is used twice.
    Duplicate,
    /// It holds nothing to do.
    Empty,
    /// It holds more than the limit allows.
    ExceedsLimit(usize),
    /// It cannot appear together with the named part.
    ConflictsWith(&'static str),
    /// It reaches outside the named bounds.
    Outside(&'static str),
    /// It names document content that only planning can resolve.
    NeedsDocumentResolution,
}

/// The refusal underneath a refused part.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssistantRequestCause {
    Sketch(SketchError),
    Request(AssistantRequestInvalid),
}

impl AssistantRequestInvalid {
    pub const fn new(subject: &'static str, problem: AssistantRequestProblem) -> Self {
        Self {
            subject,
            item: None,
            problem,
            cause: None,
        }
    }

    /// The part breaks a schema rule of its kind.
    pub const fn invalid(subject: &'static str) -> Self {
        Self::new(subject, AssistantRequestProblem::Invalid)
    }

    /// Names the refused item inside the part, such as a pocket id.
    #[must_use]
    pub fn item(mut self, item: impl ToString) -> Self {
        self.item = Some(item.to_string());
        self
    }

    #[must_use]
    pub fn caused_by(mut self, cause: impl Into<AssistantRequestCause>) -> Self {
        self.cause = Some(Box::new(cause.into()));
        self
    }

    pub const fn subject(&self) -> &'static str {
        self.subject
    }

    pub const fn problem(&self) -> AssistantRequestProblem {
        self.problem
    }
}

impl From<SketchError> for AssistantRequestCause {
    fn from(error: SketchError) -> Self {
        Self::Sketch(error)
    }
}

impl From<AssistantRequestInvalid> for AssistantRequestCause {
    fn from(error: AssistantRequestInvalid) -> Self {
        Self::Request(error)
    }
}

impl fmt::Display for AssistantRequestInvalid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "assistant {}", self.subject)?;
        if let Some(item) = &self.item {
            write!(formatter, " {item:?}")?;
        }
        match self.problem {
            AssistantRequestProblem::Invalid => formatter.write_str(" is invalid")?,
            AssistantRequestProblem::Violates(rule) => write!(formatter, " is invalid: {rule}")?,
            AssistantRequestProblem::Duplicate => formatter.write_str(" is used twice")?,
            AssistantRequestProblem::Empty => formatter.write_str(" is empty")?,
            AssistantRequestProblem::ExceedsLimit(limit) => {
                write!(formatter, " exceeds its limit of {limit}")?;
            }
            AssistantRequestProblem::ConflictsWith(other) => {
                write!(formatter, " cannot be combined with {other}")?;
            }
            AssistantRequestProblem::Outside(bounds) => write!(formatter, " is outside {bounds}")?,
            AssistantRequestProblem::NeedsDocumentResolution => {
                formatter.write_str(" requires document resolution")?;
            }
        }
        if let Some(cause) = &self.cause {
            write!(formatter, ": {cause}")?;
        }
        Ok(())
    }
}

impl fmt::Display for AssistantRequestCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sketch(error) => error.fmt(formatter),
            Self::Request(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AssistantRequestInvalid {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_deref().map(|cause| match cause {
            AssistantRequestCause::Sketch(error) => error as &(dyn std::error::Error + 'static),
            AssistantRequestCause::Request(error) => error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_problem_names_its_subject_item_and_cause() {
        use AssistantRequestProblem::*;
        let plane = AssistantRequestInvalid::invalid("construction plane creation");
        let cases = [
            (
                AssistantRequestInvalid::invalid("CAD body feature"),
                "assistant CAD body feature is invalid",
            ),
            (
                AssistantRequestInvalid::new("panel pocket", Violates("it must stay in the panel"))
                    .item("p1"),
                "assistant panel pocket \"p1\" is invalid: it must stay in the panel",
            ),
            (
                AssistantRequestInvalid::new("sketch entity ID", Duplicate).item(7),
                "assistant sketch entity ID \"7\" is used twice",
            ),
            (
                AssistantRequestInvalid::new("proposal", Empty),
                "assistant proposal is empty",
            ),
            (
                AssistantRequestInvalid::new("proposal box count", ExceedsLimit(64)),
                "assistant proposal box count exceeds its limit of 64",
            ),
            (
                AssistantRequestInvalid::new("parameter edit", ConflictsWith("geometry mutations")),
                "assistant parameter edit cannot be combined with geometry mutations",
            ),
            (
                AssistantRequestInvalid::new("subtraction", Outside("its body")),
                "assistant subtraction is outside its body",
            ),
            (
                AssistantRequestInvalid::new("referenced axis", NeedsDocumentResolution),
                "assistant referenced axis requires document resolution",
            ),
            (
                AssistantRequestInvalid::invalid("construction-plane workplane")
                    .caused_by(plane.clone()),
                "assistant construction-plane workplane is invalid: assistant construction plane creation is invalid",
            ),
            (
                AssistantRequestInvalid::invalid("workplane frame")
                    .caused_by(SketchError::InvalidWorkplaneFrame),
                "assistant workplane frame is invalid: workplane frame is not finite, orthonormal, and right-handed",
            ),
        ];
        for (error, message) in cases {
            assert_eq!(error.to_string(), message);
        }
        let nested =
            AssistantRequestInvalid::invalid("construction-plane workplane").caused_by(plane);
        let source = std::error::Error::source(&nested).expect("the nested refusal is the source");
        assert_eq!(
            source.to_string(),
            "assistant construction plane creation is invalid"
        );
        assert!(std::error::Error::source(&AssistantRequestInvalid::invalid("axis")).is_none());
    }
}
