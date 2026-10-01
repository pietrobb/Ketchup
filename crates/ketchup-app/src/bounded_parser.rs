//! Import parsers run behind `catch_unwind`, so a parser bug stops one import instead of the window.

use std::any::Any;
use std::panic::{UnwindSafe, catch_unwind};

/// Runs one parser step; a panic becomes a rejection naming the parser and the panic message.
pub(crate) fn run_bounded_parser<T>(
    parser: impl std::fmt::Display,
    step: impl FnOnce() -> T + UnwindSafe,
) -> Result<T, String> {
    catch_unwind(step).map_err(|payload| {
        format!(
            "bounded {parser} parser stopped without publishing geometry: {}",
            panic_message(payload.as_ref())
        )
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
        assert_eq!(
            literal,
            Err(
                "bounded STL parser stopped without publishing geometry: facet count overflow"
                    .to_owned()
            )
        );
        let formatted = run_bounded_parser("GLB", || -> u8 { panic!("chunk {} is short", 2) });
        assert!(formatted.unwrap_err().ends_with(": chunk 2 is short"));
        assert_eq!(run_bounded_parser("DXF", || 7), Ok(7));
    }
}
