//! Reading a Starlark program for display: which spans of a line are
//! keywords, strings, numbers or comments, and where each top-level function
//! starts and ends, so the program view can colour the code and jump to it.

use std::ops::Range;

/// What a span of a source line is, for colouring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Token {
    Plain,
    Keyword,
    /// `True`, `False`, `None`.
    Constant,
    String,
    Number,
    Comment,
    /// A name followed by `(`.
    Call,
    /// The name after `def`.
    Definition,
}

const KEYWORDS: &[&str] = &[
    "and", "break", "continue", "def", "elif", "else", "for", "if", "in", "lambda", "load", "not",
    "or", "pass", "return",
];
const CONSTANTS: &[&str] = &["True", "False", "None"];

fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Splits one line into coloured spans covering the whole line. `open`
/// carries a triple-quoted string left open by the previous line and is
/// updated for the next one.
pub(crate) fn highlight_line(
    line: &str,
    open: &mut Option<&'static str>,
) -> Vec<(Range<usize>, Token)> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    if let Some(quote) = *open {
        let Some(end) = line.find(quote) else {
            spans.push((0..line.len(), Token::String));
            return spans;
        };
        *open = None;
        index = end + quote.len();
        spans.push((0..index, Token::String));
    }
    let mut plain_from = index;
    let mut after_def = false;
    let push = |spans: &mut Vec<_>, plain_from: &mut usize, range: Range<usize>, token| {
        if *plain_from < range.start {
            spans.push((*plain_from..range.start, Token::Plain));
        }
        *plain_from = range.end;
        spans.push((range, token));
    };
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'#' {
            push(
                &mut spans,
                &mut plain_from,
                index..bytes.len(),
                Token::Comment,
            );
            index = bytes.len();
        } else if byte == b'"' || byte == b'\'' {
            let triple = if byte == b'"' { "\"\"\"" } else { "'''" };
            let end = if line[index..].starts_with(triple) {
                match line[index + 3..].find(triple) {
                    Some(offset) => index + 3 + offset + 3,
                    None => {
                        *open = Some(triple);
                        bytes.len()
                    }
                }
            } else {
                let mut end = index + 1;
                while end < bytes.len() {
                    match bytes[end] {
                        b'\\' => end += 2,
                        quote if quote == byte => {
                            end += 1;
                            break;
                        }
                        _ => end += 1,
                    }
                }
                end.min(bytes.len())
            };
            push(&mut spans, &mut plain_from, index..end, Token::String);
            index = end;
        } else if byte.is_ascii_digit() && (index == 0 || !identifier_byte(bytes[index - 1])) {
            let mut end = index;
            while end < bytes.len() && (identifier_byte(bytes[end]) || bytes[end] == b'.') {
                end += 1;
            }
            push(&mut spans, &mut plain_from, index..end, Token::Number);
            index = end;
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let mut end = index;
            while end < bytes.len() && identifier_byte(bytes[end]) {
                end += 1;
            }
            let word = &line[index..end];
            let token = if after_def {
                Token::Definition
            } else if KEYWORDS.contains(&word) {
                Token::Keyword
            } else if CONSTANTS.contains(&word) {
                Token::Constant
            } else if line[end..].trim_start().starts_with('(') {
                Token::Call
            } else {
                Token::Plain
            };
            if token != Token::Plain {
                push(&mut spans, &mut plain_from, index..end, token);
            }
            after_def = word == "def";
            index = end;
        } else {
            index += 1;
        }
    }
    if plain_from < bytes.len() {
        spans.push((plain_from..bytes.len(), Token::Plain));
    }
    spans
}

/// One top-level `def` and the 1-based lines its body spans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Function {
    pub name: String,
    pub first: usize,
    pub last: usize,
}

/// The program's top-level functions in source order. A function ends at
/// its last indented line before the next top-level statement.
pub(crate) fn functions(source: &str) -> Vec<Function> {
    let mut functions = Vec::new();
    let mut open: Option<Function> = None;
    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        let top_level = !line.starts_with([' ', '\t']) && !line.trim().is_empty();
        if top_level && !line.starts_with('#') {
            functions.extend(open.take());
            if let Some(rest) = line.strip_prefix("def ") {
                let name = rest.split('(').next().unwrap_or(rest).trim().to_owned();
                open = Some(Function {
                    name,
                    first: number,
                    last: number,
                });
            }
        } else if let Some(function) = &mut open
            && !line.trim().is_empty()
            && !top_level
        {
            function.last = number;
        }
    }
    functions.extend(open);
    functions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens<'a>(line: &'a str, open: &mut Option<&'static str>) -> Vec<(&'a str, Token)> {
        let spans = highlight_line(line, open);
        let covered: String = spans
            .iter()
            .map(|(range, _)| &line[range.clone()])
            .collect();
        assert_eq!(covered, line, "the spans cover the line exactly once");
        spans
            .into_iter()
            .map(|(range, token)| (&line[range], token))
            .filter(|(_, token)| *token != Token::Plain)
            .collect()
    }

    #[test]
    fn a_line_is_split_into_keywords_strings_numbers_calls_and_comments() {
        let mut open = None;
        assert_eq!(
            tokens(
                "def box(): return extrude(\"Box-1\", d = 40.5, ok = True) # pozdĺž",
                &mut open
            ),
            vec![
                ("def", Token::Keyword),
                ("box", Token::Definition),
                ("return", Token::Keyword),
                ("extrude", Token::Call),
                ("\"Box-1\"", Token::String),
                ("40.5", Token::Number),
                ("True", Token::Constant),
                ("# pozdĺž", Token::Comment),
            ]
        );
        // A quote inside a string and a name with digits stay what they are.
        assert_eq!(
            tokens(r#"x2 = 'it\'s' + "dvere Ø2400""#, &mut open),
            vec![
                (r"'it\'s'", Token::String),
                ("\"dvere Ø2400\"", Token::String)
            ]
        );
        assert_eq!(open, None);
    }

    #[test]
    fn a_triple_quoted_string_carries_over_lines() {
        let mut open = None;
        assert_eq!(
            tokens("doc = \"\"\"first", &mut open),
            vec![("\"\"\"first", Token::String)]
        );
        assert_eq!(
            tokens("# not a comment", &mut open),
            vec![("# not a comment", Token::String)]
        );
        assert_eq!(
            tokens("end\"\"\" + 1", &mut open),
            vec![("end\"\"\"", Token::String), ("1", Token::Number)]
        );
        assert_eq!(open, None);
    }

    #[test]
    fn functions_span_their_indented_body() {
        let source = "# header\nload(\"x\", \"y\")\n\ndef box():\n    return 1\n\n# between\ndef door(a,\n         b):\n    x = a\n\n    return x\n\nmain()\n";
        assert_eq!(
            functions(source),
            vec![
                Function {
                    name: "box".into(),
                    first: 4,
                    last: 5
                },
                Function {
                    name: "door".into(),
                    first: 8,
                    last: 12
                },
            ]
        );
    }
}
