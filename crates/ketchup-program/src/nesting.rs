//! How deeply a program nests: brackets, operators and blocks inside each
//! other. The interpreter parses, compiles and evaluates a program by
//! recursion, a stack frame or more per level, so a program a few kilobytes
//! long nesting thousands deep overflowed the stack and killed the process
//! with no answer (review 2026-11-22 R1122ac-PG-1103: a list nested 3 000
//! deep, or a sum of 5 000 terms). The nesting is read from the program's
//! tokens, before anything recurses on it, and refused with its line.

use crate::eval::{ErrorLocation, ProgramError, evaluation_error};
use starlark::codemap::CodeMap;
use starlark_syntax::lexer::{Lexer, Token};

/// How deeply a program may nest, counted as [`too_deep`] does. Hand-written
/// and generated programs stay far below it; the evaluation's own stack
/// holds it even in a debug build.
pub(crate) const MAX_NESTING: usize = 200;

/// The stack an evaluation runs on. The interpreter recurses per level a
/// program nests, tens of kilobytes a level in a debug build: on a stack of
/// its own, not the caller's (1 MB on a Windows main thread), the levels
/// [`too_deep`] lets through all fit. Reserved, committed only as used.
const EVALUATION_STACK_BYTES: usize = 64 << 20;

/// `evaluate` run on a thread with [`EVALUATION_STACK_BYTES`] of stack.
pub(crate) fn on_own_stack<T: Send>(
    evaluate: impl FnOnce() -> Result<T, ProgramError> + Send,
) -> Result<T, ProgramError> {
    std::thread::scope(|scope| {
        let evaluation = std::thread::Builder::new()
            .name("program evaluation".into())
            .stack_size(EVALUATION_STACK_BYTES)
            .spawn_scoped(scope, evaluate)
            .map_err(|error| evaluation_error("internal_error", error))?;
        evaluation
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// A `syntax_error` at the line where `source` nests too deep, if it does.
pub(crate) fn refuse_too_deep(file_name: &str, source: &str) -> Result<(), ProgramError> {
    match too_deep(file_name, source) {
        None => Ok(()),
        Some(line) => Err(ProgramError {
            code: "syntax_error",
            message: format!(
                "{file_name}:{line}: the program nests brackets, operators or blocks more than \
                 {MAX_NESTING} deep; give the inner parts names and build the value in steps"
            ),
            location: Some(ErrorLocation::of_line(file_name, source, line)),
        }),
    }
}

/// The line (from 1) where `source` first nests deeper than [`MAX_NESTING`],
/// if it does. What tokens can nest is counted at most: each operator,
/// keyword, `.`, `:` or closed bracket adds a level to the element it is in
/// until the next comma or statement, each open bracket an element of its
/// own, an indented block two levels and each `elif` of a chain one. Source the lexer refuses
/// is left to the parser to report.
pub(crate) fn too_deep(file_name: &str, source: &str) -> Option<usize> {
    let codemap = CodeMap::new(file_name.to_owned(), source.to_owned());
    let dialect = crate::eval::dialect();
    // Per open bracket (the statement first): the levels its current element
    // nests. Per block (the file first): the `elif`s chained in it.
    let mut elements = vec![0_usize];
    let mut blocks = vec![0_usize];
    let mut line_start = true;
    for (begin, token, _) in Lexer::new(source, &dialect, codemap).map_while(Result::ok) {
        let current = elements.len() - 1;
        let starts_line = std::mem::replace(&mut line_start, false);
        match token {
            Token::Newline | Token::Semicolon => {
                elements = vec![0];
                line_start = true;
            }
            Token::Indent => {
                blocks.push(0);
                line_start = true;
            }
            Token::Dedent => {
                if blocks.len() > 1 {
                    blocks.pop();
                }
                line_start = true;
            }
            Token::Comma => elements[current] = 0,
            Token::OpeningSquare | Token::OpeningCurly | Token::OpeningRound => elements.push(0),
            Token::ClosingSquare | Token::ClosingCurly | Token::ClosingRound => {
                if elements.len() > 1 {
                    elements.pop();
                }
                if let Some(levels) = elements.last_mut() {
                    *levels += 1;
                }
            }
            Token::Elif if starts_line => {
                if let Some(chain) = blocks.last_mut() {
                    *chain += 1;
                }
                elements[current] += 1;
            }
            Token::Else if starts_line => {}
            Token::Identifier(_)
            | Token::Int(_)
            | Token::Float(_)
            | Token::String(_)
            | Token::FString(_)
            | Token::Pass
            | Token::Break
            | Token::Continue => {
                if starts_line && let Some(chain) = blocks.last_mut() {
                    *chain = 0;
                }
            }
            _ => {
                if starts_line && let Some(chain) = blocks.last_mut() {
                    *chain = 0;
                }
                elements[current] += 1;
            }
        }
        let depth = blocks.iter().map(|chain| 2 + chain).sum::<usize>()
            + elements.iter().map(|levels| 1 + levels).sum::<usize>();
        if depth > MAX_NESTING {
            return Some(source[..begin].matches('\n').count() + 1);
        }
    }
    None
}
