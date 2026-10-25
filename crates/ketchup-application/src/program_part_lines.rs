//! A program the editor rewrote that then fails is refused at the line of the
//! user's program, not at the line of the rewritten text the user never saw
//! (review 2026-10-19 WB-5, review 2026-10-24 §3).
use crate::RuleProgramApplyError;
use ketchup_model::document::RuleProgramSource;
use ketchup_program::ErrorLocation;
use std::collections::HashMap;

/// Where a line is in the user's program: `Ok` for a line the edit left as
/// it was, `Err` for one the edit wrote, with the user's line it replaces
/// when there is one.
type UsersLine = Result<usize, Option<usize>>;

/// Where each line of a program the changes of a batch wrote is in the
/// user's, followed change by change: compared as a whole, a deleted last
/// line and a line a later change appends read like the user's line
/// rewritten in place (capability finding run 321).
pub(crate) struct UsersLines(Vec<UsersLine>);

impl UsersLines {
    /// The lines of the user's program `users`, each its own.
    pub(crate) fn of(users: &str) -> Self {
        Self((1..=users.split('\n').count()).map(Ok).collect())
    }

    /// The lines of `written`, one edit of `users`.
    pub(crate) fn of_edit(users: &str, written: &str) -> Self {
        Self(step_lines(written, users))
    }

    /// The lines of `written`, which one more change wrote from `before`,
    /// the program these lines are of.
    pub(crate) fn after(&self, before: &str, written: &str) -> Self {
        Self(
            (step_lines(written, before).into_iter())
                .map(|line| match line {
                    Ok(at) => self.line(at),
                    Err(Some(at)) => Err(self.line(at).map_or_else(|o| o, Some)),
                    Err(None) => Err(None),
                })
                .collect(),
        )
    }

    fn line(&self, line: usize) -> UsersLine {
        (line.checked_sub(1).and_then(|at| self.0.get(at)))
            .copied()
            .unwrap_or(Err(None))
    }
}

/// Where each line of `written`, which one change wrote from `before`, is in
/// `before`.
fn step_lines(written: &str, before: &str) -> Vec<UsersLine> {
    let written: Vec<_> = written.split('\n').collect();
    let users: Vec<_> = before.split('\n').collect();
    let shorter = written.len().min(users.len());
    let prefix = (written.iter().zip(&users))
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = (written.iter().rev().zip(users.iter().rev()))
        .take(shorter - prefix)
        .take_while(|(a, b)| a == b)
        .count();
    let (written_block, users_block) = (
        &written[prefix..written.len() - suffix],
        &users[prefix..users.len() - suffix],
    );
    // A line found once on each side is the user's, however many lines the
    // edit added or removed around it.
    let mut counts = HashMap::<&str, (usize, usize, usize)>::new();
    for text in written_block {
        counts.entry(text).or_default().0 += 1;
    }
    for (at, text) in users_block.iter().enumerate() {
        let count = counts.entry(text).or_default();
        count.1 += 1;
        count.2 = at;
    }
    let anchor = |at: usize| match counts[written_block[at]] {
        (1, 1, users_at) => Some(users_at),
        _ => None,
    };
    // The nearest lines found once on each side, before and after each line.
    let mut before = Vec::with_capacity(written_block.len());
    let mut last = (0, 0);
    for at in 0..written_block.len() {
        before.push(last);
        if let Some(users_at) = anchor(at) {
            last = (at + 1, users_at + 1);
        }
    }
    let mut after = vec![(0, 0); written_block.len()];
    let mut next = (written_block.len(), users_block.len());
    for at in (0..written_block.len()).rev() {
        after[at] = next;
        if let Some(users_at) = anchor(at) {
            next = (at, users_at);
        }
    }
    let block = (0..written_block.len()).map(|offset| {
        if let Some(at) = anchor(offset) {
            return Ok(prefix + at + 1);
        }
        // Between them the edit rewrote the lines in place only when it wrote
        // as many as it replaced; otherwise the line is one it added
        // (capability finding run 319).
        let (before, after) = (before[offset], after[offset]);
        if after.1.checked_sub(before.1) != Some(after.0 - before.0) {
            return Err(None);
        }
        let users_at = before.1 + offset - before.0;
        let line = prefix + users_at + 1;
        if written_block[offset] == users_block[users_at] {
            Ok(line)
        } else {
            Err(Some(line))
        }
    });
    (1..=prefix)
        .map(Ok)
        .chain(block)
        .chain(
            (written.len() - suffix + 1..=written.len())
                .map(|line| Ok(line + users.len() - written.len())),
        )
        .collect()
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
/// the lines of `users`; `lines` are where the lines of `written` are there.
pub(crate) fn on_users_lines(
    error: RuleProgramApplyError,
    written: &RuleProgramSource,
    users: &RuleProgramSource,
    lines: &UsersLines,
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
    let place = |line: usize| moved(&location, line, written, users, lines);
    let (message, location) = match lines.line(failing) {
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
    lines: &UsersLines,
) -> Box<ErrorLocation> {
    let mut moved = ErrorLocation::of_line(&users.file_name, &users.source, line);
    moved.call_stack = (location.call_stack.iter())
        .filter_map(|frame| {
            let mut frame = frame.clone();
            if frame.at.file == written.file_name {
                frame.at.line = lines.line(frame.at.line).ok()?;
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
/// refusal has no location. `lines` are where the lines of `written` are in
/// `users`.
pub(crate) fn refusal_on_users_lines(
    error: RuleProgramApplyError,
    written: &RuleProgramSource,
    users: &RuleProgramSource,
    lines: &UsersLines,
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
    let line = |line| lines.line(line).map_or_else(|o| o, Some);
    let location = location.and_then(|location| {
        if location.at.file != written.file_name {
            return Some(location);
        }
        Some(moved(
            &location,
            line(location.at.line)?,
            written,
            users,
            lines,
        ))
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
    use super::{UsersLine, UsersLines, users_words};

    fn users_line(written: &str, users: &str, line: usize) -> UsersLine {
        UsersLines::of_edit(users, written).line(line)
    }

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
        assert_eq!(users_line(added, users, 3), Err(None));
        assert_eq!(users_line(added, users, 6), Err(None));
        // An added line beside one an earlier change removed is still added.
        assert_eq!(users_line("a\nb\nM\n", "a\nc\nb\n", 3), Err(None));
        // A line written in place, and the unchanged one between two edits.
        let renamed = "A\nb\nC\nd\n";
        assert_eq!(users_line(renamed, users, 3), Err(Some(3)));
        assert_eq!(users_line(renamed, users, 2), Ok(2));
    }

    #[test]
    fn a_line_appended_where_an_earlier_change_deleted_the_last_is_added() {
        // As one edit, the text reads like line 3 rewritten in place.
        assert_eq!(users_line("a\nb\nM\n", "a\nb\nc\n", 3), Err(Some(3)));
        // Change by change, c is deleted and M appended (capability finding
        // run 321).
        let users = "a\nb\nc\n";
        let deleted = UsersLines::of(users).after(users, "a\nb\n");
        let appended = deleted.after("a\nb\n", "a\nb\nM\n");
        assert_eq!(appended.line(3), Err(None));
        assert_eq!(appended.line(2), Ok(2));
        // A line one change rewrites in place stays the user's.
        let renamed = appended.after("a\nb\nM\n", "a\nB\nM\n");
        assert_eq!(renamed.line(2), Err(Some(2)));
        assert_eq!(renamed.line(3), Err(None));
        assert_eq!(renamed.line(1), Ok(1));
    }
}
