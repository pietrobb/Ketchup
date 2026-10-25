//! A program the editor rewrote that then fails is refused at the line of the
//! user's program, not at the line of the rewritten text the user never saw
//! (review 2026-10-19 WB-5, review 2026-10-24 §3).
use crate::RuleProgramApplyError;
use ketchup_model::document::RuleProgramSource;
use ketchup_program::ErrorLocation;

/// Where line `line` of `written` is in `users`: `Ok` for a line the edit
/// left as it was, `Err` for one the edit wrote, with the user's line it
/// replaces when there is one.
fn users_line(written: &str, users: &str, line: usize) -> Result<usize, Option<usize>> {
    let written: Vec<_> = written.split('\n').collect();
    let users: Vec<_> = users.split('\n').collect();
    let shorter = written.len().min(users.len());
    let prefix = (written.iter().zip(&users))
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = (written.iter().rev().zip(users.iter().rev()))
        .take(shorter - prefix)
        .take_while(|(a, b)| a == b)
        .count();
    if line <= prefix {
        return Ok(line);
    }
    if line > written.len() - suffix {
        return Ok(line + users.len() - written.len());
    }
    let (written_block, users_block) = (
        &written[prefix..written.len() - suffix],
        &users[prefix..users.len() - suffix],
    );
    let offset = line - prefix - 1;
    // A line found once on each side is the user's, however many lines the
    // edit added or removed around it.
    let text = written_block[offset];
    let once = |block: &[&str]| {
        let mut found = block
            .iter()
            .enumerate()
            .filter(|(_, other)| **other == text);
        match (found.next(), found.next()) {
            (Some((at, _)), None) => Some(at),
            _ => None,
        }
    };
    if once(written_block).is_some()
        && let Some(at) = once(users_block)
    {
        return Ok(prefix + at + 1);
    }
    // The edit changed lines in place: the line is where it was.
    if written_block.len() == users_block.len() && text == users_block[offset] {
        return Ok(line);
    }
    Err((offset < users_block.len()).then_some(line))
}

/// The interpreter's own words in `message`, without its traceback and
/// excerpt, which quote the rewritten text.
fn interpreter_words(message: &str) -> &str {
    message
        .lines()
        .find_map(|line| line.strip_prefix("error: "))
        .unwrap_or(message)
}

/// `error` of evaluating `written`, the editor's rewrite of `users`, told on
/// the lines of `users`.
pub(crate) fn on_users_lines(
    error: RuleProgramApplyError,
    written: &RuleProgramSource,
    users: &RuleProgramSource,
) -> RuleProgramApplyError {
    let RuleProgramApplyError::Program {
        code,
        message,
        location: Some(location),
    } = error
    else {
        return error;
    };
    if written.source == users.source || location.at.file != written.file_name {
        return RuleProgramApplyError::Program {
            code,
            message,
            location: Some(location),
        };
    }
    let words = interpreter_words(&message);
    let failing = location.at.line;
    let failing_text =
        (written.source.split('\n').nth(failing.saturating_sub(1))).unwrap_or_default();
    let place = |line: usize| moved(&location, line, written, users);
    let (message, location) = match users_line(&written.source, &users.source, failing) {
        Ok(line) => {
            let mut moved = place(line);
            moved.at.column = location.at.column;
            (
                format!("after the edit the program fails on line {line}: {words}"),
                Some(moved),
            )
        }
        Err(Some(line)) => (
            format!(
                "the edit writes line {line} as `{}`, and the program then fails there: {words}",
                failing_text.trim()
            ),
            Some(place(line)),
        ),
        Err(None) => (
            format!(
                "the line the edit adds, `{}`, fails: {words}",
                failing_text.trim()
            ),
            None,
        ),
    };
    RuleProgramApplyError::Program {
        code,
        message,
        location,
    }
}

/// `location`, on lines of `written`, at line `line` of `users`.
fn moved(
    location: &ErrorLocation,
    line: usize,
    written: &RuleProgramSource,
    users: &RuleProgramSource,
) -> Box<ErrorLocation> {
    let mut moved = ErrorLocation::of_line(&users.file_name, &users.source, line);
    moved.call_stack = (location.call_stack.iter())
        .filter_map(|frame| {
            let mut frame = frame.clone();
            if frame.at.file == written.file_name {
                frame.at.line = users_line(&written.source, &users.source, frame.at.line).ok()?;
            }
            Some(frame)
        })
        .collect();
    moved
}

/// `error`, a refusal on the lines of `written`, the program as the earlier
/// changes of a batch left it, told on the lines of `users`: the lines its
/// words name and its location (review 2026-10-24 WB-5). A line an earlier
/// change added is no line of the user's, so it is named as such and the
/// refusal has no location.
pub(crate) fn refusal_on_users_lines(
    error: RuleProgramApplyError,
    written: &RuleProgramSource,
    users: &RuleProgramSource,
) -> RuleProgramApplyError {
    let RuleProgramApplyError::Program {
        code,
        message,
        location,
    } = error
    else {
        return error;
    };
    if written.source == users.source {
        return RuleProgramApplyError::Program {
            code,
            message,
            location,
        };
    }
    let line = |line| users_line(&written.source, &users.source, line).map_or_else(|o| o, Some);
    let location = location.and_then(|location| {
        if location.at.file != written.file_name {
            return Some(location);
        }
        Some(moved(&location, line(location.at.line)?, written, users))
    });
    RuleProgramApplyError::Program {
        code,
        message: users_words(&message, line),
        location,
    }
}

const ADDED_LINE: &str = "a line an earlier change of the edit adds";

/// `message` with each "line N" and "lines N, M" outside its quoted names
/// on the user's line `line(N)`.
fn users_words(message: &str, line: impl Fn(usize) -> Option<usize>) -> String {
    let mut words = String::with_capacity(message.len());
    let mut at = 0;
    let mut quoted = false;
    while let Some(next) = message[at..].chars().next() {
        let rest = &message[at..];
        if quoted && next == '\\' {
            let escape: usize = rest.chars().take(2).map(char::len_utf8).sum();
            words.push_str(&rest[..escape]);
            at += escape;
            continue;
        }
        if next == '"' {
            quoted = !quoted;
        } else if !quoted
            && words
                .chars()
                .last()
                .is_none_or(|last| !last.is_alphanumeric())
            && let Some(numbers) = rest
                .strip_prefix("lines ")
                .or_else(|| rest.strip_prefix("line "))
            && let (found, read) = line_numbers(numbers)
            && !found.is_empty()
        {
            let head = &rest[..rest.len() - numbers.len()];
            if let [only] = found[..] {
                match line(only) {
                    Some(users) => words.push_str(&format!("{head}{users}")),
                    None => words.push_str(ADDED_LINE),
                }
            } else {
                let numbers: Vec<_> = (found.iter())
                    .map(|&number| {
                        line(number)
                            .map_or_else(|| ADDED_LINE.to_owned(), |users| users.to_string())
                    })
                    .collect();
                words.push_str(head);
                words.push_str(&numbers.join(", "));
            }
            at += head.len() + read;
            continue;
        }
        words.push(next);
        at += next.len_utf8();
    }
    words
}

/// The numbers "N" or "N, M, …" that `text` starts with, and the bytes they take.
fn line_numbers(text: &str) -> (Vec<usize>, usize) {
    let mut numbers = Vec::new();
    let mut read = 0;
    loop {
        let separator = if numbers.is_empty() { "" } else { ", " };
        let Some(rest) = text[read..].strip_prefix(separator) else {
            break;
        };
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let Ok(number) = rest[..digits].parse() else {
            break;
        };
        numbers.push(number);
        read += separator.len() + digits;
    }
    (numbers, read)
}

#[cfg(test)]
mod tests {
    use super::{users_line, users_words};

    #[test]
    fn the_lines_a_refusal_names_are_moved_but_not_those_in_a_quoted_name() {
        let line = |line| (line != 9).then_some(line + 1);
        assert_eq!(
            users_words(
                "part \"line 2\" is used on lines 3, 9; change line 4, then outline 5",
                line
            ),
            "part \"line 2\" is used on lines 4, a line an earlier change of the edit adds; \
             change line 5, then outline 5"
        );
        assert_eq!(
            users_words("named \"m\\\" line 1\" on line 9;", line),
            "named \"m\\\" line 1\" on a line an earlier change of the edit adds;"
        );
    }

    #[test]
    fn a_line_is_found_in_the_users_program_past_lines_the_edit_added_or_removed() {
        let users = "a\nb\nc\nd\n";
        // Unchanged before and after a line the edit removed.
        let removed = "a\nc\nd\n";
        assert_eq!(users_line(removed, users, 1), Ok(1));
        assert_eq!(users_line(removed, users, 3), Ok(4));
        // A line added in the middle and one at the end.
        let added = "a\nb\nmove\nc\nd\npush\n";
        assert_eq!(users_line(added, users, 4), Ok(3));
        assert_eq!(users_line(added, users, 3), Err(Some(3)));
        assert_eq!(users_line(added, users, 6), Err(None));
        // A line written in place, and the unchanged one between two edits.
        let renamed = "A\nb\nC\nd\n";
        assert_eq!(users_line(renamed, users, 3), Err(Some(3)));
        assert_eq!(users_line(renamed, users, 2), Ok(2));
    }
}
