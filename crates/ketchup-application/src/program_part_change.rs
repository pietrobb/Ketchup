//! Deletes, recolours, renames or mirrors program parts, as the editor asks,
//! by editing the program that makes them: the call that creates a part is
//! removed or changed in place, a mirrored copy is a `mirrored()` line. A
//! part made in a loop or by the program's own function is refused with the
//! line to change instead, as is a part another line still uses.
use crate::RuleProgramApplyError;
use crate::program_part_edit::{quote, refused, written_point};
use crate::rule_program::evaluate;
use ketchup_model::document::RuleProgramSource;
use ketchup_model::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
use ketchup_program::source_edit::{self, CreatingCall, point_literal};
use ketchup_program::{Evaluated, ProgramModel};
use std::ops::Range;

/// One change of a program part the editor asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum PartChange {
    Delete,
    Color(Option<[u8; 3]>),
    Rename(String),
    /// A new part `copy`: the part mirrored across the world plane through
    /// `point` with direction `normal`.
    Mirror {
        copy: String,
        point: [f64; 3],
        normal: [f64; 3],
    },
}

/// Rewrites `source` so that it makes each part changed as asked, in order,
/// and nothing else changes. `None` when a change names something that is
/// not a part the program makes (a component or a copy of one).
///
/// # Errors
/// A part the program cannot change in place (made in a loop or by a
/// function of the program, or used by another line), or a program that no
/// longer evaluates or does not make what the change asked for.
pub fn rewrite_rule_program_parts(
    source: &RuleProgramSource,
    changes: &[(String, PartChange)],
) -> Result<Option<RuleProgramSource>, RuleProgramApplyError> {
    let mut source = source.clone();
    let mut evaluated = evaluate(&source)?;
    if changes.is_empty()
        || changes
            .iter()
            .any(|(name, _)| evaluated.model.part(name).is_none())
    {
        return Ok(None);
    }
    for (name, change) in changes {
        let edit = Edit {
            source: &source,
            evaluated: &evaluated,
            name,
        };
        let (rewritten, expected) = match change {
            PartChange::Delete => edit.delete()?,
            PartChange::Color(color) => edit.color(*color)?,
            PartChange::Rename(new) => edit.rename(new)?,
            PartChange::Mirror {
                copy,
                point,
                normal,
            } => edit.mirror(copy, *point, *normal)?,
        };
        let after = evaluate(&rewritten)?;
        if !expected.holds(&evaluated.model, &after.model) {
            return Err(refused(
                "program_part_change_mismatch",
                format!(
                    "the program did not make part {} as the edit asked; change it in the program",
                    quote(name)
                ),
            ));
        }
        source = rewritten;
        evaluated = after;
    }
    Ok(Some(source))
}

/// What the program must make after a change, from what it made before.
enum Expected {
    Same(Box<ProgramModel>),
    /// `before` with every name `from` read as `to`, and `to` continuing `from`.
    Renamed {
        from: String,
        to: String,
    },
    /// `before` and a new part `copy` with these world bounds.
    Mirrored {
        copy: String,
        bounds: ([f64; 3], [f64; 3]),
    },
}

impl Expected {
    fn holds(&self, before: &ProgramModel, after: &ProgramModel) -> bool {
        match self {
            Self::Same(expected) => **expected == *after,
            Self::Renamed { from, to } => {
                let mut expected = serde_json::to_value(before).expect("models serialize");
                renamed(&mut expected, from, to);
                expected["continuations"][to.as_str()] = from.as_str().into();
                serde_json::to_value(after).is_ok_and(|after| after == expected)
            }
            Self::Mirrored { copy, bounds } => {
                let Some(made) = after.part(copy) else {
                    return false;
                };
                let (min, max) = made.world_bounds();
                let near = |a: [f64; 3], b: [f64; 3]| {
                    (0..3).all(|i| (a[i] - b[i]).abs() <= DEFAULT_LINEAR_TOLERANCE_MM)
                };
                let mut rest = after.clone();
                rest.parts.retain(|part| &part.name != copy);
                near(min, bounds.0) && near(max, bounds.1) && rest == *before
            }
        }
    }
}

/// `value` with the name `from` read as `to`, also where it starts a name the
/// program derives from it ("a transform", "a/notch").
fn renamed(value: &mut serde_json::Value, from: &str, to: &str) {
    match value {
        serde_json::Value::String(text)
            if text
                .strip_prefix(from)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '/'])) =>
        {
            text.replace_range(..from.len(), to);
        }
        serde_json::Value::Array(items) => {
            items.iter_mut().for_each(|item| renamed(item, from, to))
        }
        serde_json::Value::Object(fields) => {
            let keys = fields
                .keys()
                .filter(|key| *key == from)
                .cloned()
                .collect::<Vec<_>>();
            for key in keys {
                if let Some(field) = fields.remove(&key) {
                    fields.insert(to.to_owned(), field);
                }
            }
            fields
                .values_mut()
                .for_each(|field| renamed(field, from, to));
        }
        _ => {}
    }
}

struct Edit<'a> {
    source: &'a RuleProgramSource,
    evaluated: &'a Evaluated,
    name: &'a str,
}

impl Edit<'_> {
    fn text(&self) -> &str {
        &self.source.source
    }

    fn with(&self, mut edits: Vec<(Range<usize>, String)>) -> RuleProgramSource {
        edits.sort_by_key(|(bytes, _)| std::cmp::Reverse(bytes.start));
        let mut rewritten = self.source.clone();
        for (bytes, text) in edits {
            rewritten.source.replace_range(bytes, &text);
        }
        rewritten
    }

    fn line_of(&self, byte: usize) -> usize {
        self.text()[..byte].matches('\n').count() + 1
    }

    /// The call that makes the part on its own, outside loops and the
    /// program's functions, where `doing` can change it for this part alone.
    fn creating_call(&self, doing: &str) -> Result<CreatingCall, RuleProgramApplyError> {
        let name = quote(self.name);
        let call = self
            .evaluated
            .part_sources
            .get(self.name)
            .and_then(|recorded| {
                source_edit::creating_call(&self.source.file_name, self.text(), recorded)
            })
            .ok_or_else(|| {
                refused(
                    "program_part_call_not_found",
                    format!(
                        "the line that makes part {name} was not found; {doing} in the program"
                    ),
                )
            })?;
        if let Some(line) = call.loop_line {
            return Err(refused(
                "program_part_in_loop",
                format!(
                    "part {name} is made in a loop on line {line}, with the other parts of the loop; {doing} in the program"
                ),
            ));
        }
        if let Some(function) = &call.function {
            return Err(refused(
                "program_part_in_function",
                format!(
                    "part {name} is made by the program's function {function}() called on line {}; {doing} in the program",
                    call.lines.first
                ),
            ));
        }
        Ok(call)
    }

    fn delete(&self) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let doing = "delete it";
        let name = quote(self.name);
        let call = self.creating_call(doing)?;
        let Some(statement) = call.statement.clone() else {
            return Err(refused(
                "program_part_shared_line",
                format!(
                    "part {name} is made on line {} together with other values; {doing} in the program",
                    call.lines.first
                ),
            ));
        };
        let mut removed = vec![statement];
        for lines in &self.evaluated.part_sources[self.name] {
            if *lines == call.lines {
                continue;
            }
            if let Some((other, _)) = self
                .evaluated
                .part_sources
                .iter()
                .find(|(other, recorded)| other.as_str() != self.name && recorded.contains(lines))
            {
                return Err(refused(
                    "program_part_referenced",
                    format!(
                        "part {name} is used on line {} together with part {}; delete that line first",
                        lines.first,
                        quote(other)
                    ),
                ));
            }
            let Some(changing) =
                source_edit::top_level_call_statement(&self.source.file_name, self.text(), *lines)
            else {
                return Err(refused(
                    "program_part_referenced",
                    format!(
                        "part {name} is changed on line {}; {doing} in the program",
                        lines.first
                    ),
                ));
            };
            removed.push(changing);
        }
        removed.sort_by_key(|bytes| bytes.start);
        removed.dedup();
        if let Some((_, line)) =
            source_edit::string_literals(&self.source.file_name, self.text(), self.name)
                .into_iter()
                .find(|(bytes, _)| {
                    !removed
                        .iter()
                        .any(|gone| gone.start <= bytes.start && bytes.end <= gone.end)
                })
        {
            return Err(refused(
                "program_part_referenced",
                format!("part {name} is used on line {line}; delete that use first"),
            ));
        }
        let rewritten = self.with(
            removed
                .iter()
                .map(|bytes| (bytes.clone(), String::new()))
                .collect(),
        );
        if let Err(RuleProgramApplyError::Program {
            message, location, ..
        }) = evaluate(&rewritten)
        {
            let at = location.map_or_else(String::new, |location| {
                format!(
                    " on line {}",
                    self.original_line(&removed, location.at.line)
                )
            });
            return Err(refused(
                "program_part_referenced",
                format!("part {name} is still used{at}: {message}"),
            ));
        }
        let mut expected = self.evaluated.model.clone();
        expected.parts.retain(|part| part.name != self.name);
        expected.continuations.remove(self.name);
        Ok((rewritten, Expected::Same(Box::new(expected))))
    }

    /// The line of the source before `removed` was cut out.
    fn original_line(&self, removed: &[Range<usize>], line: usize) -> usize {
        let mut original = line;
        for bytes in removed {
            if self.line_of(bytes.start) <= original {
                original += self.text()[bytes.clone()].matches('\n').count();
            }
        }
        original
    }

    fn color(
        &self,
        color: Option<[u8; 3]>,
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let call = self.creating_call("change its colour")?;
        let written = color.map(|[r, g, b]| format!("color=({r}, {g}, {b})"));
        let edit = match (call.color, written) {
            (Some((argument, _)), Some(written)) => (argument, written),
            (Some((argument, _)), None) => {
                let before = self.text()[..argument.start].trim_end();
                let start = before.strip_suffix(',').map_or(argument.start, str::len);
                (start..argument.end, String::new())
            }
            (None, Some(written)) => {
                let inside = self.text()[..call.bytes.end - 1].trim_end();
                let separator = if inside.ends_with([',', '(']) {
                    " "
                } else {
                    ", "
                };
                (inside.len()..inside.len(), format!("{separator}{written}"))
            }
            (None, None) => (0..0, String::new()),
        };
        let rewritten = self.with(vec![edit]);
        let mut expected = self.evaluated.model.clone();
        expected
            .parts
            .iter_mut()
            .filter(|part| part.name == self.name)
            .for_each(|part| part.color = color);
        Ok((rewritten, Expected::Same(Box::new(expected))))
    }

    fn rename(&self, new: &str) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let doing = "rename it";
        let name = quote(self.name);
        if new.trim().is_empty() || self.evaluated.model.part(new).is_some() {
            return Err(refused(
                "program_part_name_taken",
                format!(
                    "the program already makes a part named {}; choose another name",
                    quote(new)
                ),
            ));
        }
        let call = self.creating_call(doing)?;
        if call.name.is_none() {
            return Err(refused(
                "program_part_computed_name",
                format!(
                    "part {name} gets its name from an expression on line {}; {doing} in the program",
                    call.lines.first
                ),
            ));
        }
        let renamed = self.with(
            source_edit::string_literals(&self.source.file_name, self.text(), self.name)
                .into_iter()
                .map(|(bytes, _)| (bytes, quote(new)))
                .collect(),
        );
        // The renamed part keeps the document part, and its ID, of the old name.
        let mut rewritten = renamed.clone();
        if let Some(was) = source_edit::continuation_of(&renamed.file_name, &renamed.source, new) {
            rewritten.source.replace_range(was, &name);
        } else {
            if !rewritten.source.ends_with('\n') {
                rewritten.source.push('\n');
            }
            rewritten
                .source
                .push_str(&format!("continue_part({}, was={name})\n", quote(new)));
        }
        Ok((
            rewritten,
            Expected::Renamed {
                from: self.name.to_owned(),
                to: new.to_owned(),
            },
        ))
    }

    fn mirror(
        &self,
        copy: &str,
        point: [f64; 3],
        normal: [f64; 3],
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let length = normal.iter().map(|value| value * value).sum::<f64>().sqrt();
        if self.evaluated.model.part(copy).is_some() || length <= 0.0 || !length.is_finite() {
            return Err(refused(
                "program_part_mirror",
                format!(
                    "the mirrored copy of part {} needs a new name and a plane with a direction",
                    quote(self.name)
                ),
            ));
        }
        let normal = normal.map(|value| (value / length * 1.0e9).round() / 1.0e9);
        let point = written_point(point);
        let part = self
            .evaluated
            .model
            .part(self.name)
            .expect("checked by the caller");
        let (low, high) = part.local_bounds();
        let n = normal;
        let mut bounds = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for corner in 0..8 {
            let local = std::array::from_fn(|i| {
                if corner >> i & 1 == 0 {
                    low[i]
                } else {
                    high[i]
                }
            });
            let world = part.to_world(local);
            let across = (0..3).map(|i| (world[i] - point[i]) * n[i]).sum::<f64>();
            for i in 0..3 {
                let reflected = world[i] - 2.0 * across * n[i];
                bounds.0[i] = bounds.0[i].min(reflected);
                bounds.1[i] = bounds.1[i].max(reflected);
            }
        }
        let mut rewritten = self.source.clone();
        if !rewritten.source.ends_with('\n') {
            rewritten.source.push('\n');
        }
        rewritten.source.push_str(&format!(
            "mirrored({}, {}, point={}, normal={})\n",
            quote(self.name),
            quote(copy),
            point_literal(point),
            point_literal(normal)
        ));
        Ok((
            rewritten,
            Expected::Mirrored {
                copy: copy.to_owned(),
                bounds,
            },
        ))
    }
}

#[cfg(test)]
#[path = "program_part_change_tests.rs"]
mod tests;
