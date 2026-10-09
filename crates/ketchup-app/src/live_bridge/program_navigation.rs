//! Reading a large program in pieces (`program read` with `lines`, `search`,
//! `part` or `mode=outline`) and changing its parameters without sending the
//! source (`program action=set_params`).
#[cfg(test)]
#[path = "program_navigation_tests.rs"]
mod tests;

use super::program_access::invalid;
use super::*;
use ketchup_program::SourceLines;

/// Most lines one answer returns; the agent reads the rest by range.
const MAX_LINES: usize = 400;
/// Most text one answer returns, well inside one response frame.
const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Longest outline entry text.
const MAX_OUTLINE_CHARS: usize = 120;

/// Which piece of the program to return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Piece {
    /// An inclusive, 1-based line range.
    Lines(usize, usize),
    /// The lines containing a text, ignoring case.
    Search(String),
    /// The lines that define a part, with two neighbours.
    Part(String),
    /// Top-level definitions, parameters and comment sections.
    Outline,
}

impl Piece {
    pub(super) fn from_request(
        lines: Option<[usize; 2]>,
        search: Option<String>,
        part: Option<String>,
        outline: bool,
    ) -> Result<Self, &'static str> {
        let piece = match (lines, search, part, outline) {
            (Some([first, last]), None, None, false) => {
                if first == 0 || last < first {
                    return Err(invalid(
                        "lines",
                        format!("lines [{first}, {last}] is not a 1-based range"),
                        "Give lines=[first, last] with 1 <= first <= last.",
                    ));
                }
                Self::Lines(first, last)
            }
            (None, Some(text), None, false) if !text.is_empty() => Self::Search(text),
            (None, None, Some(name), false) => Self::Part(name),
            (None, None, None, true) => Self::Outline,
            _ => {
                return Err(invalid(
                    "piece",
                    "Give exactly one of lines, search (nonempty), part or mode=outline.",
                    "Read mode=outline first, then lines=[first, last] of the section you need.",
                ));
            }
        };
        Ok(piece)
    }

    fn needs_part_sources(&self) -> bool {
        matches!(self, Self::Part(_))
    }
}

/// The numbered lines `indices` (0-based) of `lines`, bounded; `true` when cut.
fn numbered(lines: &[&str], indices: impl IntoIterator<Item = usize>) -> (Vec<Value>, bool) {
    let mut rows = Vec::new();
    let mut bytes = 0;
    for index in indices {
        let Some(text) = lines.get(index) else {
            break;
        };
        if rows.len() == MAX_LINES || bytes + text.len() > MAX_TEXT_BYTES {
            return (rows, true);
        }
        bytes += text.len();
        rows.push(json!({"line": index + 1, "text": text}));
    }
    (rows, false)
}

/// A comment at column 0 that starts a block: the first comment line after a
/// blank line or code.
fn starts_section(lines: &[&str], index: usize) -> bool {
    lines[index].starts_with('#') && (index == 0 || !lines[index - 1].trim_start().starts_with('#'))
}

fn outline_kind(lines: &[&str], index: usize) -> Option<&'static str> {
    let line = lines[index];
    if line.starts_with("def ") {
        Some("def")
    } else if starts_section(lines, index) {
        Some("section")
    } else if !line.starts_with([' ', '\t', '#']) && line.contains("param(") {
        Some("param")
    } else {
        None
    }
}

fn outline(lines: &[&str]) -> (Vec<Value>, bool) {
    let entries = (0..lines.len())
        .filter_map(|index| outline_kind(lines, index).map(|kind| (index, kind)))
        .collect::<Vec<_>>();
    let sections = entries
        .iter()
        .filter(|(_, kind)| *kind == "section")
        .map(|(index, _)| *index)
        .collect::<Vec<_>>();
    let rows = entries
        .iter()
        .take(MAX_LINES)
        .map(|&(index, kind)| {
            let text = lines[index].trim_end();
            let mut row = json!({"line": index + 1, "kind": kind,
                "text": text.chars().take(MAX_OUTLINE_CHARS).collect::<String>()});
            if kind == "section" {
                // A section reaches to the line before the next one.
                let next = sections.iter().find(|&&start| start > index);
                row["last"] = json!(next.map_or(lines.len(), |&start| start));
            }
            row
        })
        .collect();
    (rows, entries.len() > MAX_LINES)
}

/// Answers `piece` of `source`; `part_sources` are the defining lines of
/// each part (needed only for [`Piece::Part`]).
pub(super) fn read_piece(
    file_name: &str,
    source: &str,
    part_sources: Option<&BTreeMap<String, Vec<SourceLines>>>,
    piece: &Piece,
) -> Result<Value, &'static str> {
    let lines = source.split_inclusive('\n').collect::<Vec<_>>();
    let mut answer = json!({"file_name": file_name, "line_count": lines.len()});
    let (rows, truncated) = match piece {
        Piece::Lines(first, last) => numbered(&lines, *first - 1..*last),
        Piece::Search(text) => {
            let needle = text.to_lowercase();
            let hits = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.to_lowercase().contains(&needle))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            answer["matches"] = json!(hits.len());
            numbered(&lines, hits)
        }
        Piece::Part(name) => {
            let ranges = part_sources.and_then(|sources| sources.get(name)).ok_or_else(|| {
                invalid(
                    "part",
                    format!("program part {name:?} does not exist"),
                    "Use search=<text> or mode=outline to find the part, or mode=selection for the selected parts.",
                )
            })?;
            answer["lines_of_part"] = json!(
                ranges
                    .iter()
                    .map(|range| [range.first, range.last])
                    .collect::<Vec<_>>()
            );
            let included = ranges
                .iter()
                .flat_map(|range| range.first.saturating_sub(3)..range.last.saturating_add(2))
                .collect::<BTreeSet<_>>();
            numbered(&lines, included)
        }
        Piece::Outline => {
            let (rows, truncated) = outline(&lines);
            answer["outline"] = json!(rows);
            answer["truncated"] = json!(truncated);
            answer["hint"] = json!("Read a section with lines=[line, last].");
            return Ok(answer);
        }
    };
    answer["lines"] = json!(rows);
    answer["truncated"] = json!(truncated);
    answer["hint"] = json!(
        "Line text retains line endings: concatenate directly for patch old text. When truncated, read on with lines=[next, last]."
    );
    Ok(answer)
}

impl LiveBridge {
    pub(super) fn program_piece(
        app: &KetchupApp,
        expected: &Option<Stamp>,
        piece: &Piece,
    ) -> Result<Value, &'static str> {
        Self::guard(app, expected)?;
        let Some(program) = app.document.current_rule_program() else {
            return Ok(
                json!({"source": null, "hint": "No program owns this document; undo a detaching edit or apply a complete program."}),
            );
        };
        if !piece.needs_part_sources() {
            return read_piece(&program.file_name, &program.source, None, piece);
        }
        // A bridge request answers in the same call.
        let evaluated = app
            .program_evaluations
            .get_blocking(program)
            .map_err(|error| failed_because("program_rejected", error))?;
        read_piece(
            &program.file_name,
            &program.source,
            Some(&evaluated.part_sources),
            piece,
        )
    }

    /// The program apply of `set_params`: the current source with `params`
    /// replacing those values and keeping every other stored value.
    pub(super) fn expand_set_params(
        app: &KetchupApp,
        expected: Stamp,
        params: BTreeMap<String, f64>,
    ) -> Result<Request, &'static str> {
        Self::guard(app, &Some(expected.clone()))?;
        let program = app.document.current_rule_program().ok_or_else(|| {
            invalid(
                "params",
                "No program owns this document.",
                "Undo a detaching edit or apply a complete program first.",
            )
        })?;
        if params.is_empty() {
            return Err(invalid(
                "params",
                "params is empty.",
                "Give the parameter values to change, e.g. {\"width\": 900}.",
            ));
        }
        let evaluated = app
            .program_evaluations
            .get_blocking(program)
            .map_err(|error| failed_because("program_rejected", error))?;
        let known = &evaluated.model.params;
        if let Some(unknown) = params
            .keys()
            .find(|name| !known.iter().any(|param| &param.name == *name))
        {
            let names = known
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(invalid(
                "params",
                format!("the program declares no parameter {unknown:?}"),
                &format!("Declared parameters: {names}."),
            ));
        }
        let mut overrides = program.overrides.clone();
        overrides.extend(params);
        Ok(Request::ApplyProgram {
            expected: Some(expected),
            source: program.source.clone(),
            overrides: Some(overrides),
            file_name: Some(program.file_name.clone()),
            replace_document: false,
        })
    }
}
