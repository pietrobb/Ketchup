//! Where in the user's program an evaluation failed: file, line and column,
//! the lines around it and the calls that led there.
use serde::Serialize;
use starlark::codemap::FileSpan;

/// Lines shown on each side of the failing line.
const CONTEXT_LINES: usize = 2;
/// Longest excerpt line kept; a minified program must not flood the answer.
const MAX_LINE_CHARS: usize = 200;
/// Innermost calls kept from a deep recursion.
const MAX_FRAMES: usize = 12;

/// One numbered line of the evaluated source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NumberedLine {
    pub line: usize,
    pub text: String,
}

/// A 1-based position in a program file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Position {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

/// One call on the way to the failure, outermost first: the function called
/// and where it was called from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CallFrame {
    pub function: String,
    #[serde(flatten)]
    pub at: Position,
}

/// The innermost place in the user's own program (not the library) where the
/// failure happened, the source around it and the call stack.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ErrorLocation {
    #[serde(flatten)]
    pub at: Position,
    /// The failing line and up to two lines on each side, from the evaluated
    /// text: after a patch these are the patched lines the caller never saw.
    pub excerpt: Vec<NumberedLine>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub call_stack: Vec<CallFrame>,
}

fn position(span: &FileSpan) -> Position {
    let begin = span.resolve_span().begin;
    Position {
        file: span.filename().to_owned(),
        line: begin.line + 1,
        column: begin.column + 1,
    }
}

/// Locates `error` in `source` (the program named `file_name`), or `None`
/// when no position of the failure lies in the user's program.
pub(super) fn locate(
    file_name: &str,
    source: &str,
    error: &starlark::Error,
) -> Option<Box<ErrorLocation>> {
    let frames = &error.call_stack().frames;
    let mut call_stack = frames
        .iter()
        .filter_map(|frame| {
            Some(CallFrame {
                function: frame.name.clone(),
                at: position(frame.location.as_ref()?),
            })
        })
        .collect::<Vec<_>>();
    let at = frames
        .iter()
        .filter_map(|frame| frame.location.as_ref())
        .chain(error.span())
        .rfind(|span| span.filename() == file_name)
        .map(position)?;
    call_stack.drain(..call_stack.len().saturating_sub(MAX_FRAMES));
    let first = at.line.saturating_sub(CONTEXT_LINES).max(1);
    let excerpt = source
        .lines()
        .enumerate()
        .skip(first - 1)
        .take(at.line + CONTEXT_LINES + 1 - first)
        .map(|(index, text)| NumberedLine {
            line: index + 1,
            text: text.chars().take(MAX_LINE_CHARS).collect(),
        })
        .collect();
    Some(Box::new(ErrorLocation {
        at,
        excerpt,
        call_stack,
    }))
}
