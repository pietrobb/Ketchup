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
use ketchup_program::model::{Part, ProgramEdgeFinishKind, ProgramPartBody};
use ketchup_program::source_edit::{
    self, CreatingCall, number_literal, point_literal, quote, written, written_point,
};
use ketchup_program::{Evaluated, ProgramFeatureKind, ProgramModel};
use std::collections::{BTreeMap, BTreeSet};
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
        let after = evaluate(&rewritten).map_err(|error| expected.kept_lines().unwrap_or(error))?;
        if !expected.holds(&evaluated.model, &after.model) {
            return Err(expected.kept_lines().unwrap_or_else(|| {
                refused(
                    "program_part_change_mismatch",
                    format!(
                        "the program did not make part {} as the edit asked; change it in the program",
                        quote(name)
                    ),
                )
            }));
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
    /// `before` with every part name `from` read as `to`, and `to` continuing
    /// `from`; `kept` are the lines whose literal `from` is not the part's.
    Renamed {
        from: String,
        to: String,
        kept: Vec<usize>,
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
    /// The extrusion's length with every Push/Pull of its caps.
    Length,
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
            Self::Length => match &part.body {
                ProgramPartBody::Extrusion {
                    distance_mm, caps, ..
                } => Some(
                    distance_mm
                        + part
                            .face_offsets()
                            .filter(|offset| {
                                caps.iter()
                                    .any(|cap| offset.face.split('#').next() == Some(cap))
                            })
                            .map(|offset| offset.distance_mm)
                            .sum::<f64>(),
                ),
                _ => None,
            },
        }
    }
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= DEFAULT_LINEAR_TOLERANCE_MM)
}

impl Expected {
    /// A rename that left a literal of the old name the program still needs:
    /// refused with the lines to change by hand.
    fn kept_lines(&self) -> Option<RuleProgramApplyError> {
        let Self::Renamed { from, to, kept } = self else {
            return None;
        };
        let lines = kept
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let word = if kept.len() == 1 { "line" } else { "lines" };
        (!kept.is_empty()).then(|| {
            refused(
                "program_part_name_elsewhere",
                format!(
                    "{word} {lines} of the program read {} not as the part's name, and renaming the part to {} alone changes what the program makes; change {word} {lines} in the program",
                    quote(from),
                    quote(to)
                ),
            )
        })
    }

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
            Self::Renamed { from, to, .. } => {
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

/// Fields of a program model that hold names of something else than parts:
/// parameters, materials, attributes and tags keep their names in a rename.
const NOT_PART_NAMES: [&str; 9] = [
    "params",
    "material",
    "attributes",
    "tags",
    "material_weights",
    "weight_scope",
    "production_scope",
    "strength_classes",
    "alternative_tags",
];

/// `value` with the part name `from` read as `to`, also where it starts a
/// name the program derives from it ("a transform", "a/notch").
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
                .iter_mut()
                .filter(|(key, _)| !NOT_PART_NAMES.contains(&key.as_str()))
                .for_each(|(_, field)| renamed(field, from, to));
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
        let Some(made) = call.name else {
            return Err(refused(
                "program_part_computed_name",
                format!(
                    "part {name} gets its name from an expression on line {}; {doing} in the program",
                    call.lines.first
                ),
            ));
        };
        // Only the literals that name the part change: a material, a
        // parameter or a text that reads the same stays as it is.
        let mut edits = vec![(made.clone(), quote(new))];
        let mut kept = Vec::new();
        for (bytes, line) in
            source_edit::string_literals(&self.source.file_name, self.text(), self.name)
        {
            if bytes == made {
                continue;
            }
            if self.names_the_part(&bytes, new) {
                edits.push((bytes, quote(new)));
            } else {
                kept.push(line);
            }
        }
        let renamed = self.with(edits);
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
                kept,
            },
        ))
    }

    /// Whether the string literal at `bytes` names this part: read as
    /// `other`, a name no part has, the program stops at that name.
    fn names_the_part(&self, bytes: &Range<usize>, other: &str) -> bool {
        matches!(
            evaluate(&self.with(vec![(bytes.clone(), quote(other))])),
            Err(RuleProgramApplyError::Program { message, .. })
                if message.contains(&format!("{other:?}"))
        )
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
    /// at overridden so the part reads `value`, or the other parts that
    /// override changes too. `None` when not exactly one parameter drives
    /// it, or the override misses: a probe proposes, it does not promise the
    /// program is linear.
    fn driven(&self, reading: &Reading, value: f64) -> Option<Driven> {
        let baseline = reading.of(&self.evaluated.model, self.name)?;
        let mut drivers = Vec::new();
        let mut outside = Vec::new();
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
            if slope.abs() <= ACCUMULATED_ROUNDING {
                continue;
            }
            if within(replacement) {
                drivers.push((parameter.name.clone(), replacement));
            } else {
                outside.push((parameter, replacement));
            }
        }
        let [(parameter, replacement)] = drivers.as_slice() else {
            // The one parameter that drives the value, past the range the
            // program declares for it.
            return match outside.as_slice() {
                [(parameter, replacement)] if drivers.is_empty() => Some(Driven::OutOfRange {
                    parameter: parameter.name.clone(),
                    min: parameter.min,
                    max: parameter.max,
                    value: *replacement,
                }),
                _ => None,
            };
        };
        // Without the noise of the probe: a whole default stays whole, else
        // nine decimals, as long as the part still reads `value`.
        let whole = (self.evaluated.model.params.iter())
            .any(|made| &made.name == parameter && made.default.fract() == 0.0)
            && (replacement - replacement.round()).abs() <= DEFAULT_LINEAR_TOLERANCE_MM;
        let candidates = [
            whole.then(|| replacement.round()),
            Some((replacement * 1.0e9).round() / 1.0e9),
            Some(*replacement),
        ];
        let (rewritten, after) = candidates.into_iter().flatten().find_map(|replacement| {
            let mut rewritten = self.source.clone();
            rewritten.overrides.insert(parameter.clone(), replacement);
            let after = evaluate(&rewritten).ok()?;
            reading
                .of(&after.model, self.name)
                .is_some_and(|after| (after - value).abs() <= ACCUMULATED_ROUNDING)
                .then_some((rewritten, after))
        })?;
        let parts = self.other_parts_changed(&after.model);
        Some(if parts.is_empty() {
            Driven::Alone(rewritten)
        } else {
            Driven::Shared {
                parameter: parameter.clone(),
                parts,
            }
        })
    }

    /// The parts and tools other than this one that `after` adds, removes
    /// or makes differently.
    fn other_parts_changed(&self, after: &ProgramModel) -> Vec<String> {
        fn bodies(model: &ProgramModel) -> BTreeMap<&str, &Part> {
            (model.parts.iter().chain(&model.tools))
                .map(|part| (part.name.as_str(), part))
                .collect()
        }
        let (before, after) = (bodies(&self.evaluated.model), bodies(after));
        let names = before.keys().chain(after.keys()).collect::<BTreeSet<_>>();
        names
            .into_iter()
            .filter(|name| **name != self.name && before.get(*name) != after.get(*name))
            .map(|name| (*name).to_owned())
            .collect()
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
        // A param() that changes other parts too is left alone: the line
        // below pushes this part's face only.
        if let Some(before) = reading.of(&self.evaluated.model, self.name)
            && let Some(Driven::Alone(rewritten)) = self.driven(&reading, before + distance)
        {
            return Ok((rewritten, self.reads(reading, before + distance)));
        }
        let part = self.evaluated.model.part(self.name);
        let count = part.map_or(0, |part| part.face_offsets().count());
        // The editor names an extrusion's caps "start" and "end"; the
        // program names them as the extrusion does.
        let written_face = match (part.map(|part| &part.body), face) {
            (Some(ProgramPartBody::Extrusion { caps, .. }), "start") => caps[0].as_str(),
            (Some(ProgramPartBody::Extrusion { caps, .. }), "end") => caps[1].as_str(),
            _ => face,
        };
        let line = format!(
            "push_pull({}, face={}, distance={}, name={})\n",
            quote(self.name),
            quote(written_face),
            number_literal(distance),
            quote(&format!("GUI Push/Pull {}", count + 1))
        );
        let reading = Reading::Face(written_face.to_owned());
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
    /// extrusion's length is set as [`Self::length`] does. Any other value is
    /// refused with the line that writes it.
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
        let end = self.evaluated.model.part(self.name).and_then(|part| {
            let pad = (part.features.iter())
                .any(|made| made.name == feature && made.kind == ProgramFeatureKind::Pad);
            match &part.body {
                ProgramPartBody::Extrusion { caps, .. } if pad && path == "extent.distance" => {
                    Some(caps[1].clone())
                }
                _ => None,
            }
        });
        if let Some(end) = end {
            return self.length(feature, &end, current, value);
        }
        let driven = self.driven(&reading, value);
        if let Some(Driven::Alone(rewritten)) = driven {
            return Ok((rewritten, self.reads(reading, value)));
        }
        let what = format!("{feature} · {path}");
        if let Some(refusal) =
            (driven.as_ref()).and_then(|driven| driven.out_of_range(self.name, &what))
        {
            return Err(refusal);
        }
        let line = self
            .found_call()
            .map_or_else(String::new, |call| format!(" on line {}", call.lines.first));
        if let Some(Driven::Shared { parameter, parts }) = driven {
            let more = parts.len().saturating_sub(5);
            let mut named = (parts.iter().take(5))
                .map(|part| quote(part))
                .collect::<Vec<_>>();
            if more > 0 {
                named.push(format!("{more} more"));
            }
            return Err(refused(
                "program_parameter_shared",
                format!(
                    "param({}) sets {feature} · {path} of part {name} and also changes {}; set the parameter {} to change them all, or change part {name}{line} in the program",
                    quote(&parameter),
                    named.join(", "),
                    quote(&parameter)
                ),
            ));
        }
        Err(refused(
            "program_parameter_not_driven",
            format!(
                "{feature} · {path} of part {name} is not set by one param(); change it{line} in the program"
            ),
        ))
    }

    /// The extrusion `feature`, `pad` long as the program writes it, made
    /// `value` long: by the one `param()` that alone makes its length, else
    /// by one `push_pull()` of its `end` named [`LENGTH`]. A value typed
    /// again writes that line again, so it replaces the first value instead
    /// of adding to it; the Push/Pulls of other lines stay, and the whole
    /// length with them must stay above 0.
    fn length(
        &self,
        feature: &str,
        end: &str,
        pad: f64,
        value: f64,
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let name = quote(self.name);
        let part = self
            .evaluated
            .model
            .part(self.name)
            .expect("checked by the caller");
        let written_before = part
            .face_offsets()
            .find(|offset| offset.name == LENGTH)
            .map(|offset| offset.distance_mm);
        let length = Reading::Length.of(&self.evaluated.model, self.name);
        let others = length.map_or(0.0, |length| {
            length - pad - written_before.unwrap_or_default()
        });
        let total = value + others;
        if !total.is_finite() || total <= DEFAULT_LINEAR_TOLERANCE_MM {
            return Err(self.too_short(feature, total));
        }
        // With no line of its own yet, the one param() that makes the whole
        // length alone is overridden; once the line is there, it is written
        // again, so a parameter never adds to it.
        match self.driven(&Reading::Length, total) {
            Some(Driven::Alone(rewritten)) if written_before.is_none() => {
                return Ok((rewritten, self.reads(Reading::Length, total)));
            }
            Some(driven) => {
                // The length line would pass by the range the program
                // declares for the parameter that makes the length.
                if let Some(refusal) = driven.out_of_range(self.name, feature) {
                    return Err(refusal);
                }
            }
            None => {}
        }
        let distance = written(value - pad);
        let line = (distance.abs() > DEFAULT_LINEAR_TOLERANCE_MM).then(|| {
            format!(
                "push_pull({name}, face={}, distance={}, name={})\n",
                quote(end),
                number_literal(distance),
                quote(LENGTH)
            )
        });
        let earlier = source_edit::named_call_statement(
            &self.source.file_name,
            self.text(),
            "push_pull",
            self.name,
            &[("face", end), ("name", LENGTH)],
        );
        let rewritten = match (earlier, line) {
            (Some(bytes), line) => self.with(vec![(bytes, line.unwrap_or_default())]),
            (None, _) if written_before.is_some() => {
                return Err(refused(
                    "program_length_not_written",
                    format!(
                        "the push_pull() named {} of part {name} is not a line of its own on face {}; change it in the program",
                        quote(LENGTH),
                        quote(end)
                    ),
                ));
            }
            (None, Some(line)) => self.appended(&line),
            (None, None) => self.source.clone(),
        };
        Ok((rewritten, self.reads(Reading::Length, total)))
    }

    fn too_short(&self, feature: &str, length: f64) -> RuleProgramApplyError {
        let length = if length.is_finite() {
            number_literal(written(length))
        } else {
            length.to_string()
        };
        refused(
            "program_length_not_positive",
            format!(
                "{feature} of part {} would be {length} mm long; type a length above 0",
                quote(self.name)
            ),
        )
    }
}

/// Name of the `push_pull()` that sets an extrusion's length typed for it.
const LENGTH: &str = "Length";

/// What overriding the one `param()` that drives a part's value does.
enum Driven {
    /// The overridden source, which changes this part alone.
    Alone(RuleProgramSource),
    /// The `param()` changes these other parts too, or adds or removes them.
    Shared {
        parameter: String,
        parts: Vec<String>,
    },
    /// The one `param()` that drives the value would have to be `value`,
    /// outside the range the program declares for it.
    OutOfRange {
        parameter: String,
        min: Option<f64>,
        max: Option<f64>,
        value: f64,
    },
}

impl Driven {
    /// The refusal of a value its one parameter cannot take, if it is one.
    fn out_of_range(&self, part: &str, what: &str) -> Option<RuleProgramApplyError> {
        let Self::OutOfRange {
            parameter,
            min,
            max,
            value,
        } = self
        else {
            return None;
        };
        let number = |value: f64| number_literal(written(value));
        let range = match (min, max) {
            (Some(min), Some(max)) => format!("from {} to {}", number(*min), number(*max)),
            (Some(min), None) => format!("at least {}", number(*min)),
            (None, Some(max)) => format!("at most {}", number(*max)),
            (None, None) => return None,
        };
        Some(refused(
            "program_parameter_out_of_range",
            format!(
                "{what} of part {} is set by param({}), which the program allows {range}, and it would be {}; type a value within that range, or change the range in the program",
                quote(part),
                quote(parameter),
                number(*value)
            ),
        ))
    }
}

#[cfg(test)]
#[path = "program_part_change_tests.rs"]
mod tests;
