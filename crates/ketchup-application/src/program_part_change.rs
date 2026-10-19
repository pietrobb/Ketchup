//! Writes what the editor does to program parts into the program that makes
//! them: the call that creates a part is removed or changed in place, a value
//! one `param()` drives is overridden, and a mirrored copy, move, turn,
//! Push/Pull, fillet or chamfer is a line of its own. A part made in a loop or by the program's
//! own function is refused with the line to change instead, as is a part
//! another line still uses.
use crate::RuleProgramApplyError;
use crate::program_part_edit::refused;
use crate::rule_program::evaluate;
use ketchup_model::document::{RuleProgramSource, Transform};
use ketchup_model::tolerance::{ACCUMULATED_ROUNDING, DEFAULT_LINEAR_TOLERANCE_MM};
use ketchup_program::frame::{self, Mat3};
use ketchup_program::model::{ProgramEdgeFinishKind, ProgramPartBody};
use ketchup_program::source_edit::{
    self, CreatingCall, number_literal, point_literal, quote, written_point,
};
use ketchup_program::{Evaluated, ProgramFeatureKind, ProgramModel};
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
    /// The named face pushed out (or pulled in) by `distance` millimetres.
    PushPull {
        face: String,
        distance: f64,
    },
    /// Edges, each named by its two faces, rounded or bevelled by `amount`.
    Finish {
        kind: ProgramEdgeFinishKind,
        edges: Vec<[String; 2]>,
        amount: f64,
    },
    /// A value of one of the part's features, as the parameter editor sets it.
    Parameter {
        feature: String,
        path: String,
        value: f64,
    },
    /// The part moved or turned by this world rigid motion.
    Transform(Transform),
}

/// `source` with `edits` (bytes and their new text) made.
fn with_edits(
    source: &RuleProgramSource,
    mut edits: Vec<(Range<usize>, String)>,
) -> RuleProgramSource {
    edits.sort_by_key(|(bytes, _)| std::cmp::Reverse(bytes.start));
    let mut rewritten = source.clone();
    for (bytes, text) in edits {
        rewritten.source.replace_range(bytes, &text);
    }
    rewritten
}

/// `source` with `lines` added at its end.
fn appended(source: &RuleProgramSource, lines: &str) -> RuleProgramSource {
    let mut rewritten = source.clone();
    source_edit::append_lines(&mut rewritten.source, lines);
    rewritten
}

/// Rewrites `source` so that it makes each part changed as asked, in order,
/// and nothing else changes. `None` when a change names something that is
/// not a part the program makes (a component or a copy of one), or the
/// changes delete every line of the program.
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
            PartChange::PushPull { face, distance } => edit.push_pull(face, *distance)?,
            PartChange::Finish {
                kind,
                edges,
                amount,
            } => edit.finish(*kind, edges, *amount)?,
            PartChange::Parameter {
                feature,
                path,
                value,
            } => edit.parameter(feature, path, *value)?,
            PartChange::Transform(world_edit) => edit.transform(*world_edit)?,
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
    // Every line deleted: a program cannot be empty, and nothing of it is left
    // to keep, so the edit is committed as it is.
    Ok((!source.source.trim().is_empty()).then_some(source))
}

/// What the program must make after a change, from what it made before.
pub(crate) enum Expected {
    Same(Box<ProgramModel>),
    /// `before` with `part`'s frame at `at` turned to `rotation`.
    Placed {
        part: String,
        at: [f64; 3],
        rotation: Mat3,
    },
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
    /// `part` reads `value` where `reading` looks.
    Reads {
        part: String,
        reading: Reading,
        value: f64,
    },
}

/// A value of a program part that an edit sets.
pub(crate) enum Reading {
    /// The last Push/Pull of the face, else the extrusion's length at its end cap.
    Face(String),
    Feature {
        feature: String,
        path: String,
    },
    /// How many operations the part has.
    Operations,
}

impl Reading {
    fn of(&self, model: &ProgramModel, part: &str) -> Option<f64> {
        let part = model.part(part)?;
        match self {
            Self::Face(face) => part
                .face_offsets()
                .filter(|offset| &offset.face == face)
                .last()
                .map(|offset| offset.distance_mm)
                .or(match &part.body {
                    ProgramPartBody::Extrusion {
                        distance_mm, caps, ..
                    } if face == "end" || *face == caps[1] => Some(*distance_mm),
                    _ => None,
                }),
            Self::Feature { feature, path } => part
                .features
                .iter()
                .find(|made| &made.name == feature)?
                .parameters
                .iter()
                .find(|parameter| &parameter.path == path)
                .map(|parameter| parameter.value),
            Self::Operations => u32::try_from(part.operations.len()).ok().map(f64::from),
        }
    }
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= DEFAULT_LINEAR_TOLERANCE_MM)
}

impl Expected {
    pub(crate) fn holds(&self, before: &ProgramModel, after: &ProgramModel) -> bool {
        match self {
            Self::Placed { part, at, rotation } => {
                before.parts.len() == after.parts.len()
                    && before.parts.iter().zip(&after.parts).all(|(old, new)| {
                        if old.name != *part {
                            return old == new;
                        }
                        new.name == old.name
                            && new.size_mm == old.size_mm
                            && near(new.at_mm, *at)
                            && frame::same_orientation(&new.rotation, rotation)
                    })
            }
            Self::Reads {
                part,
                reading,
                value,
            } => reading
                .of(after, part)
                .is_some_and(|read| (read - value).abs() <= ACCUMULATED_ROUNDING),
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

pub(crate) struct Edit<'a> {
    source: &'a RuleProgramSource,
    pub(crate) evaluated: &'a Evaluated,
    pub(crate) name: &'a str,
}

impl Edit<'_> {
    fn text(&self) -> &str {
        &self.source.source
    }

    pub(crate) fn with(&self, edits: Vec<(Range<usize>, String)>) -> RuleProgramSource {
        with_edits(self.source, edits)
    }

    fn line_of(&self, byte: usize) -> usize {
        self.text()[..byte].matches('\n').count() + 1
    }

    pub(crate) fn found_call(&self) -> Option<CreatingCall> {
        let recorded = self.evaluated.part_sources.get(self.name)?;
        source_edit::creating_call(&self.source.file_name, self.text(), recorded)
    }

    /// The call that makes the part on its own, outside loops and the
    /// program's functions, where `doing` can change it for this part alone.
    fn creating_call(&self, doing: &str) -> Result<CreatingCall, RuleProgramApplyError> {
        let name = quote(self.name);
        let call = self.found_call().ok_or_else(|| {
            refused(
                "program_part_call_not_found",
                format!("the line that makes part {name} was not found; {doing} in the program"),
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
        let rewritten = match source_edit::continuation_of(&renamed.file_name, &renamed.source, new)
        {
            Some(was) => with_edits(&renamed, vec![(was, name)]),
            None => appended(
                &renamed,
                &format!("continue_part({}, was={name})\n", quote(new)),
            ),
        };
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
        let line = format!(
            "mirrored({}, {}, point={}, normal={})\n",
            quote(self.name),
            quote(copy),
            point_literal(point),
            point_literal(normal)
        );
        let copy = copy.to_owned();
        Ok((self.appended(&line), Expected::Mirrored { copy, bounds }))
    }

    pub(crate) fn appended(&self, lines: &str) -> RuleProgramSource {
        appended(self.source, lines)
    }

    fn reads(&self, reading: Reading, value: f64) -> Expected {
        Expected::Reads {
            part: self.name.to_owned(),
            reading,
            value,
        }
    }

    /// The source with the one `param()` that drives what `reading` looks
    /// at overridden so the part reads `value`. `None` when not exactly one
    /// parameter drives it, or the override misses: a probe proposes, it
    /// does not promise the program is linear.
    fn driven(&self, reading: &Reading, value: f64) -> Option<RuleProgramSource> {
        let baseline = reading.of(&self.evaluated.model, self.name)?;
        let mut drivers = Vec::new();
        for parameter in &self.evaluated.model.params {
            let within = |value: f64| {
                parameter.min.is_none_or(|minimum| value >= minimum)
                    && parameter.max.is_none_or(|maximum| value <= maximum)
            };
            let current =
                (self.source.overrides.get(&parameter.name).copied()).unwrap_or(parameter.value);
            // not a tolerance: a probe step large enough to change the evaluated model.
            let step = current.abs().mul_add(1.0e-4, 1.0e-3);
            let Some(probed) = [current + step, current - step]
                .into_iter()
                .find(|v| within(*v))
            else {
                continue;
            };
            let mut probe = self.source.clone();
            probe.overrides.insert(parameter.name.clone(), probed);
            // A speculative step may cross into another valid branch of the program.
            let Some(probed_value) = evaluate(&probe)
                .ok()
                .and_then(|probe| reading.of(&probe.model, self.name))
            else {
                continue;
            };
            let slope = (probed_value - baseline) / (probed - current);
            let replacement = current + (value - baseline) / slope;
            if slope.abs() > ACCUMULATED_ROUNDING && within(replacement) {
                drivers.push((parameter.name.clone(), replacement));
            }
        }
        let [(parameter, replacement)] = drivers.as_slice() else {
            return None;
        };
        let mut rewritten = self.source.clone();
        rewritten.overrides.insert(parameter.clone(), *replacement);
        evaluate(&rewritten)
            .ok()
            .and_then(|after| reading.of(&after.model, self.name))
            .is_some_and(|after| (after - value).abs() <= ACCUMULATED_ROUNDING)
            .then_some(rewritten)
    }

    /// The one `param()` that drives the face is overridden; otherwise the
    /// program gets a `push_pull()` line.
    fn push_pull(
        &self,
        face: &str,
        distance: f64,
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        if !distance.is_finite() || distance.abs() <= f64::EPSILON {
            return Err(refused(
                "invalid_push_pull",
                "Push/Pull distance must be finite and non-zero",
            ));
        }
        let reading = Reading::Face(face.to_owned());
        if let Some(before) = reading.of(&self.evaluated.model, self.name)
            && let Some(rewritten) = self.driven(&reading, before + distance)
        {
            return Ok((rewritten, self.reads(reading, before + distance)));
        }
        let part = self.evaluated.model.part(self.name);
        let count = part.map_or(0, |part| part.face_offsets().count());
        let line = format!(
            "push_pull({}, face={}, distance={}, name={})\n",
            quote(self.name),
            quote(face),
            number_literal(distance),
            quote(&format!("GUI Push/Pull {}", count + 1))
        );
        Ok((self.appended(&line), self.reads(reading, distance)))
    }

    fn finish(
        &self,
        kind: ProgramEdgeFinishKind,
        edges: &[[String; 2]],
        amount: f64,
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let (call, amount_name) = match kind {
            ProgramEdgeFinishKind::Fillet => ("fillet", "radius"),
            ProgramEdgeFinishKind::Chamfer => ("chamfer", "distance"),
        };
        let edges = edges
            .iter()
            .map(|[first, second]| format!("[{}, {}]", quote(first), quote(second)))
            .collect::<Vec<_>>();
        let line = format!(
            "{call}({}, edges=[{}], {amount_name}={})\n",
            quote(self.name),
            edges.join(", "),
            number_literal(amount)
        );
        let operations = Reading::Operations.of(&self.evaluated.model, self.name);
        let expected = self.reads(Reading::Operations, operations.unwrap_or_default() + 1.0);
        Ok((self.appended(&line), expected))
    }

    /// The one `param()` that drives the value is overridden; an
    /// extrusion's length no parameter drives is a Push/Pull of its end.
    /// Any other value is refused with the line that writes it.
    fn parameter(
        &self,
        feature: &str,
        path: &str,
        value: f64,
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let name = quote(self.name);
        let reading = Reading::Feature {
            feature: feature.to_owned(),
            path: path.to_owned(),
        };
        let Some(current) = reading.of(&self.evaluated.model, self.name) else {
            return Err(refused(
                "program_parameter_unknown",
                format!("part {name} has no value {feature} · {path} in its program"),
            ));
        };
        if let Some(rewritten) = self.driven(&reading, value) {
            return Ok((rewritten, self.reads(reading, value)));
        }
        let end = self.evaluated.model.part(self.name).and_then(|part| {
            let pad = (part.features.iter())
                .any(|made| made.name == feature && made.kind == ProgramFeatureKind::Pad);
            match &part.body {
                ProgramPartBody::Extrusion { caps, .. } if pad => Some(caps[1].clone()),
                _ => None,
            }
        });
        if let Some(end) = end.filter(|_| path == "extent.distance") {
            return self.push_pull(&end, value - current);
        }
        let line = self
            .found_call()
            .map_or_else(String::new, |call| format!(" on line {}", call.lines.first));
        Err(refused(
            "program_parameter_not_driven",
            format!(
                "{feature} · {path} of part {name} is not set by one param(); change it{line} in the program"
            ),
        ))
    }
}

#[cfg(test)]
#[path = "program_part_change_tests.rs"]
mod tests;
