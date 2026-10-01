//! Import parsers run behind `catch_unwind`, so a parser bug stops one import instead of the window.

use std::any::Any;
use std::fmt;
use std::panic::{UnwindSafe, catch_unwind};

/// A parser that panicked: which parser, and the panic message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParserPanic {
    pub(crate) parser: String,
    pub(crate) message: String,
}

impl fmt::Display for ParserPanic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "bounded {} parser stopped without publishing geometry: {}",
            self.parser, self.message
        )
    }
}

impl std::error::Error for ParserPanic {}

/// Runs one parser step; a panic becomes a rejection naming the parser and the panic message.
pub(crate) fn run_bounded_parser<T>(
    parser: impl fmt::Display,
    step: impl FnOnce() -> T + UnwindSafe,
) -> Result<T, ParserPanic> {
    catch_unwind(step).map_err(|payload| ParserPanic {
        parser: parser.to_string(),
        message: panic_message(payload.as_ref()).to_owned(),
    })
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("the panic carried no message")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parser_panic_names_the_parser_and_its_message() {
        let literal = run_bounded_parser("STL", || -> u8 { panic!("facet count overflow") });
        let panic = literal.unwrap_err();
        assert_eq!(
            panic,
            ParserPanic {
                parser: "STL".to_owned(),
                message: "facet count overflow".to_owned(),
            }
        );
        assert_eq!(
            panic.to_string(),
            "bounded STL parser stopped without publishing geometry: facet count overflow"
        );
        let formatted = run_bounded_parser("GLB", || -> u8 { panic!("chunk {} is short", 2) });
        assert_eq!(formatted.unwrap_err().message, "chunk 2 is short");
        assert_eq!(run_bounded_parser("DXF", || 7), Ok(7));
    }
}
