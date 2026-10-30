//! Writes what the editor does to program parts into the program that makes
//! them: the call that creates a part is removed or changed in place, a value
//! one `param()` drives is overridden, and a mirrored copy, move, turn,
//! Push/Pull, fillet or chamfer is a line of its own. A part made in a loop or by the program's
//! own function is refused with the line to change instead, as is a part
//! another line still uses.
use crate::RuleProgramApplyError;
use crate::program_part_edit::{refused, refused_at};
use crate::program_part_lines::{UsersLines, on_users_lines, refusal_on_users_lines};
use crate::rule_program::evaluate;
use ketchup_model::document::{RuleProgramSource, Transform};
use ketchup_model::tolerance::{ACCUMULATED_ROUNDING, DEFAULT_LINEAR_TOLERANCE_MM};
use ketchup_program::frame::{self, Mat3};
use ketchup_program::model::{
    Part, ProgramEdgeFinishKind, ProgramFeature, ProgramOperation, ProgramPartBody,
};
use ketchup_program::source_edit::{
    self, CreatingCall, number_literal, point_literal, quote, written, written_point,
};
use ketchup_program::{Evaluated, ProgramFeatureKind, ProgramModel, SourceLines};
use std::borrow::Cow;
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
    Ok(rewrite_rule_program_parts_from(source, None, changes)?.map(|rewritten| rewritten.source))
}

/// A program rewritten by [`rewrite_rule_program_parts_from`], with the
/// evaluation that checked it.
pub struct RewrittenProgram {
    pub source: RuleProgramSource,
    pub evaluated: Evaluated,
}

/// [`rewrite_rule_program_parts`] that starts from `current`, the evaluation
/// of `source` the caller holds (evaluated here when `None`), and keeps the
/// evaluation of the rewritten program: on a large program each evaluation costs as
/// much as the rest of the edit.
///
/// # Errors
/// As [`rewrite_rule_program_parts`].
pub fn rewrite_rule_program_parts_from(
    source: &RuleProgramSource,
    current: Option<&Evaluated>,
    changes: &[(String, PartChange)],
) -> Result<Option<RewrittenProgram>, RuleProgramApplyError> {
    let users = source;
    let mut source = users.clone();
    let mut evaluated = match current {
        Some(current) => Cow::Borrowed(current),
        None => Cow::Owned(evaluate(&source)?),
    };
    // A name an earlier change of the batch gives (a rename, a mirrored
    // copy) is a part of the program by the time its change comes.
    let mut parts = (evaluated.model.parts.iter())
        .map(|part| part.name.clone())
        .collect::<BTreeSet<_>>();
    for (name, change) in changes {
        if !parts.contains(name) {
            return Ok(None);
        }
        match change {
            PartChange::Rename(new) => {
                parts.remove(name);
                parts.insert(new.clone());
            }
            PartChange::Mirror { copy, .. } => {
                parts.insert(copy.clone());
            }
            PartChange::Delete => {
                parts.remove(name);
            }
            _ => {}
        }
    }
    if changes.is_empty() {
        return Ok(None);
    }
    // The document name of each part renamed earlier in the batch: the
    // document still has the part under that name.
    let mut document_names = BTreeMap::<String, String>::new();
    // Many parts moved alike are written and checked in one evaluation, not
    // two or three for each part (review 2026-10-19 WB-6).
    if let Some((source, evaluated)) =
        crate::program_part_edit::transformed_at_once(&source, &evaluated, changes)?
    {
        return Ok(Some(RewrittenProgram { source, evaluated }));
    }
    if let Some(coloured) = coloured_at_once(&source, &evaluated, changes)? {
        return Ok(Some(coloured));
    }
    // Where each line of `source` is in the user's program.
    let mut lines = UsersLines::of(&users.source);
    for (name, change) in changes {
        let edit = Edit {
            source: &source,
            evaluated: &evaluated,
            name,
        };
        // Refused on the lines of the program the earlier changes left, told
        // on the user's.
        let on_users =
            |error: RuleProgramApplyError| refusal_on_users_lines(error, &source, users, &lines);
        let mut checked = None;
        let (rewritten, expected) = match change {
            PartChange::Delete => edit.delete().map_err(on_users)?,
            PartChange::Color(color) => edit.color(*color).map_err(on_users)?,
            PartChange::Rename(new) => {
                let document_name = document_names.remove(name).unwrap_or_else(|| name.clone());
                let renamed = edit.rename(new, &document_name).map_err(on_users)?;
                document_names.insert(new.clone(), document_name);
                renamed
            }
            PartChange::Mirror {
                copy,
                point,
                normal,
            } => edit.mirror(copy, *point, *normal).map_err(on_users)?,
            PartChange::PushPull { face, distance } => {
                edit.push_pull(face, *distance).map_err(on_users)?
            }
            PartChange::Finish {
                kind,
                edges,
                amount,
            } => edit.finish(*kind, edges, *amount).map_err(on_users)?,
            PartChange::Parameter {
                feature,
                path,
                value,
            } => edit.parameter(feature, path, *value).map_err(on_users)?,
            PartChange::Transform(world_edit) => {
                let (rewritten, expected, after) = edit.transform(*world_edit).map_err(on_users)?;
                checked = after;
                (rewritten, expected)
            }
        };
        let after = match checked {
            Some(after) => after,
            None => evaluate(&rewritten).map_err(|error| {
                (expected.kept_lines(&source).map(on_users)).unwrap_or_else(|| {
                    let written = lines.after(&source.source, &rewritten.source);
                    on_users_lines(error, &rewritten, users, &written)
                })
            })?,
        };
        if !expected.holds(&evaluated.model, &after.model) {
            return Err(on_users(expected.kept_lines(&source).unwrap_or_else(|| {
                let message = format!(
                    "the program did not make part {} as the edit asked: {}; change it in the program",
                    quote(name),
                    expected.difference(&evaluated.model, &after.model)
                );
                match edit.found_call() {
                    Some(call) => {
                        edit.refused_on(call.lines.first, "program_part_change_mismatch", message)
                    }
                    None => refused("program_part_change_mismatch", message),
                }
            })));
        }
        lines = lines.after(&source.source, &rewritten.source);
        source = rewritten;
        evaluated = Cow::Owned(after);
    }
    // Every part deleted: a program cannot be empty, and what is left of it (a
    // comment, a param()) makes nothing, so the edit is committed as it is and
    // the window says no part is left.
    let makes_nothing = evaluated.model.parts.is_empty() && evaluated.model.instances.is_empty();
    Ok((!makes_nothing).then(|| RewrittenProgram {
        source,
        evaluated: evaluated.into_owned(),
    }))
}

/// A batch that only colours parts, each once, written in one go and checked
/// as a whole: colouring a part and its mirrored copy alike is no "copy
/// changed too" (review 2026-10-24 WB-23). `None` for any other batch.
fn coloured_at_once(
    source: &RuleProgramSource,
    evaluated: &Evaluated,
    changes: &[(String, PartChange)],
) -> Result<Option<RewrittenProgram>, RuleProgramApplyError> {
    let names: BTreeSet<_> = changes.iter().map(|(name, _)| name).collect();
    if changes.len() < 2 || names.len() < changes.len() {
        return Ok(None);
    }
    let mut colours = BTreeMap::new();
    for (name, change) in changes {
        let PartChange::Color(color) = change else {
            return Ok(None);
        };
        colours.insert(name.as_str(), *color);
    }
    let mut edits = Vec::new();
    for name in colours.keys() {
        let edit = Edit {
            source,
            evaluated,
            name,
        };
        edits.push(edit.colour_edit(colours[name])?);
    }
    // Parts one call makes are coloured one after another.
    edits.sort_by_key(|(bytes, _)| bytes.start);
    if edits
        .windows(2)
        .any(|pair| pair[1].0.start < pair[0].0.end || pair[1].0 == pair[0].0)
    {
        return Ok(None);
    }
    let rewritten = with_edits(source, edits);
    let after = evaluate(&rewritten).map_err(|error| {
        let lines = UsersLines::of_edit(&source.source, &rewritten.source);
        on_users_lines(error, &rewritten, source, &lines)
    })?;
    let mut expected = evaluated.model.clone();
    for part in &mut expected.parts {
        if let Some(color) = colours.get(part.name.as_str()) {
            part.color = *color;
        }
    }
    let expected = Expected::Same(Box::new(expected));
    if !expected.holds(&evaluated.model, &after.model) {
        return Err(refused(
            "program_part_change_mismatch",
            format!(
                "the program did not colour the parts as the edit asked: {}; change it in the program",
                expected.difference(&evaluated.model, &after.model)
            ),
        ));
    }
    Ok(Some(RewrittenProgram {
        source: rewritten,
        evaluated: after,
    }))
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
    /// the document's part `was`; `kept` are the lines whose literal `from`
    /// is not the part's.
    Renamed {
        from: String,
        to: String,
        was: String,
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
    fn kept_lines(&self, source: &RuleProgramSource) -> Option<RuleProgramApplyError> {
        let Self::Renamed { from, to, kept, .. } = self else {
            return None;
        };
        let lines = kept
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let word = if kept.len() == 1 { "line" } else { "lines" };
        kept.first().map(|first| {
            refused_at(
                source,
                *first,
                "program_part_name_elsewhere",
                format!(
                    "{word} {lines} of the program read {} not as the part's name, and renaming the part to {} alone changes what the program makes; change {word} {lines} in the program",
                    quote(from),
                    quote(to)
                ),
            )
        })
    }

    /// What `after` makes otherwise than the change asked, for a refusal.
    fn difference(&self, before: &ProgramModel, after: &ProgramModel) -> String {
        match self {
            Self::Placed { part, at, .. } => match after.part(part) {
                None => format!("it no longer makes part {}", quote(part)),
                Some(made) if !near(made.at_mm, *at) => format!(
                    "part {} stands at {} instead of {}",
                    quote(part),
                    point_literal(written_point(made.at_mm)),
                    point_literal(written_point(*at))
                ),
                Some(_) => changed_parts(before, after, Some(part))
                    .unwrap_or_else(|| format!("part {} is turned otherwise", quote(part))),
            },
            Self::Reads {
                part,
                reading,
                value,
            } => match reading.of(after, part) {
                Some(read) => format!(
                    "part {} reads {} instead of {}",
                    quote(part),
                    number_literal(written(read)),
                    number_literal(written(*value))
                ),
                None => format!("part {} no longer has that value", quote(part)),
            },
            Self::Same(expected) => changed_parts(expected, after, None)
                .unwrap_or_else(|| "what it declares besides parts changed".to_owned()),
            Self::Renamed { from, to, was, .. } => {
                if after.part(to).is_none() {
                    return format!("it makes no part {}", quote(to));
                }
                let (old, new) = (names(before), names(after));
                let gone = old
                    .difference(&new)
                    .filter(|name| *name != from)
                    .cloned()
                    .collect::<Vec<_>>();
                let added = new
                    .difference(&old)
                    .filter(|name| *name != to)
                    .cloned()
                    .collect::<Vec<_>>();
                if !gone.is_empty() && !added.is_empty() {
                    return format!("it also renames {} to {}", listed(&gone), listed(&added));
                }
                // Review 2026-10-26 WB-41: the first value it makes otherwise.
                let (expected, kept) = renamed_expected(before, after, from, to, was);
                serde_json::to_value(after)
                    .ok()
                    .and_then(|after| rename_mismatch(&expected, &after, from, to, &kept, ""))
                    .map_or_else(
                        || "its parts changed besides the name".to_owned(),
                        |mismatch| format!("{mismatch} besides the name"),
                    )
            }
            Self::Mirrored { copy, bounds } => match after.part(copy) {
                None => format!("it makes no part {}", quote(copy)),
                Some(made)
                    if !(near(made.world_bounds().0, bounds.0)
                        && near(made.world_bounds().1, bounds.1)) =>
                {
                    format!(
                        "part {} lies from {} to {} instead of {} to {}",
                        quote(copy),
                        point_literal(written_point(made.world_bounds().0)),
                        point_literal(written_point(made.world_bounds().1)),
                        point_literal(written_point(bounds.0)),
                        point_literal(written_point(bounds.1))
                    )
                }
                Some(_) => changed_parts(before, after, Some(copy))
                    .unwrap_or_else(|| "what it declares besides parts changed".to_owned()),
            },
        }
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
            Self::Renamed { from, to, was, .. } => {
                let (expected, kept) = renamed_expected(before, after, from, to, was);
                serde_json::to_value(after).is_ok_and(|after| {
                    rename_mismatch(&expected, &after, from, to, &kept, "").is_none()
                })
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

/// "parts a, b and 3 more changed" for the parts and tools `after` makes
/// otherwise than `expected`, `except` aside; `None` when none differs.
fn changed_parts(
    expected: &ProgramModel,
    after: &ProgramModel,
    except: Option<&str>,
) -> Option<String> {
    fn bodies(model: &ProgramModel) -> BTreeMap<&str, &Part> {
        (model.parts.iter().chain(&model.tools))
            .map(|part| (part.name.as_str(), part))
            .collect()
    }
    let (expected, after) = (bodies(expected), bodies(after));
    let changed = (expected.keys().chain(after.keys()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|name| Some(**name) != except && expected.get(*name) != after.get(*name))
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    (!changed.is_empty()).then(|| format!("{} changed too", listed(&changed)))
}

/// Up to five quoted names, then how many more.
fn listed(names: &[String]) -> String {
    if names.is_empty() {
        return "nothing".to_owned();
    }
    let mut shown = names
        .iter()
        .take(5)
        .map(|name| quote(name))
        .collect::<Vec<_>>();
    if names.len() > 5 {
        shown.push(format!("{} more", names.len() - 5));
    }
    shown.join(", ")
}

/// Fields of a program model that hold names of something else than parts:
/// parameters, materials, attributes, tags and annotations keep their names
/// and texts in a rename.
const NOT_PART_NAMES: [&str; 13] = [
    "annotations",
    "params",
    "material",
    "attributes",
    "tags",
    "material_weights",
    "weight_scope",
    "production_scope",
    "strength_classes",
    "stock",
    "prices",
    "currency",
    "alternative_tags",
];

/// What follows `from` in `name` when `name` is one derived from it
/// ("a transform", "a/notch"): `Some("")` for `from` itself.
fn derived_from<'n>(name: &'n str, from: &str) -> Option<&'n str> {
    name.strip_prefix(from)
        .filter(|rest| rest.is_empty() || rest.starts_with([' ', '/']))
}

/// The names of the parts, tools, groups, components and instances a
/// program makes.
fn names(model: &ProgramModel) -> BTreeSet<String> {
    (model.parts.iter().chain(&model.tools))
        .map(|part| &part.name)
        .chain(model.groups.iter().map(|group| &group.name))
        .chain(model.components.iter().map(|component| &component.name))
        .chain(model.instances.iter().map(|instance| &instance.name))
        .cloned()
        .collect()
}

/// What else than a part of the program `name` already names: a component,
/// a group, an instance, a tool or the earlier name another part than
/// `part` continues.
fn name_use(model: &ProgramModel, part: &str, name: &str) -> Option<String> {
    let named = |found: bool, what: &str| found.then(|| what.to_owned());
    named(
        model.components.iter().any(|item| item.name == name),
        "a component",
    )
    .or_else(|| named(model.groups.iter().any(|item| item.name == name), "a group"))
    .or_else(|| {
        named(
            model.instances.iter().any(|item| item.name == name),
            "an instance",
        )
    })
    .or_else(|| named(model.tools.iter().any(|item| item.name == name), "a tool"))
    .or_else(|| {
        model
            .continuations
            .iter()
            .find(|(other, was)| *was == name && *other != part)
            .map(|(other, _)| format!("the earlier name of part {}", quote(other)))
    })
}

/// `before` as JSON with the continuation of the rename of `from` to `to`, and
/// the names `kept`: a part, tool or group whose name starts with the old one
/// ("p 2", "a mirrored") keeps its name, unless the program makes it from the
/// part's name and now names it after the new one.
fn renamed_expected(
    before: &ProgramModel,
    after: &ProgramModel,
    from: &str,
    to: &str,
    was: &str,
) -> (serde_json::Value, Vec<String>) {
    let now = names(after);
    let kept = names(before)
        .into_iter()
        .filter(|other| {
            derived_from(other, from)
                .is_some_and(|rest| !now.contains(&format!("{to}{rest}")) || now.contains(other))
        })
        .collect::<Vec<_>>();
    let mut expected = before.clone();
    expected.continuations.remove(from);
    expected.continuations.insert(to.to_owned(), was.to_owned());
    let expected = serde_json::to_value(expected).expect("models serialize");
    (expected, kept)
}

/// `text` with every whole `from` in it read as `to`: at its start ("a/notch")
/// or after a `:` or `+` of an id derived from it ("dowel:a:1", "dowel:b+a",
/// review 2026-10-26 WB-41), followed by its end, a space, `/`, `:` or `+`;
/// but not where one of the names `kept` stands. `None` when it holds none.
fn renamed_text(text: &str, from: &str, to: &str, kept: &[String]) -> Option<String> {
    let mut renamed = String::new();
    let mut rest = 0;
    for (at, _) in text.match_indices(from) {
        let after = &text[at + from.len()..];
        let whole = (at == 0 || text[..at].ends_with([':', '+']))
            && (after.is_empty() || after.starts_with([' ', '/', ':', '+']))
            && !kept
                .iter()
                .any(|kept| derived_from(&text[at..], kept).is_some());
        if whole && at >= rest {
            renamed.push_str(&text[rest..at]);
            renamed.push_str(to);
            rest = at + from.len();
        }
    }
    (rest > 0).then(|| renamed + &text[rest..])
}

/// Where `after` is not `before` with the part name `from` read as `to` in
/// some of the places that hold it, also within a name or id the program
/// derives from it ([`renamed_text`]), but not in the names `kept` or the names
/// derived from them: the first value it makes otherwise, at `path`. A string
/// that only reads the same, a text, a feature kind or a face, stays as it was
/// (review 2026-10-24 WB-24).
fn rename_mismatch(
    before: &serde_json::Value,
    after: &serde_json::Value,
    from: &str,
    to: &str,
    kept: &[String],
    path: &str,
) -> Option<String> {
    use serde_json::Value;
    let renamed = |text: &str| renamed_text(text, from, to, kept);
    let differs = || {
        let at = if path.is_empty() { "the model" } else { path };
        Some(format!("{at} is {after} instead of {before}"))
    };
    match (before, after) {
        (Value::String(old), Value::String(new)) => {
            if old == new || renamed(old).is_some_and(|renamed| renamed == *new) {
                None
            } else {
                differs()
            }
        }
        (Value::Array(old), Value::Array(new)) if old.len() == new.len() => old
            .iter()
            .zip(new)
            .enumerate()
            .find_map(|(index, (old, new))| {
                let item = match old.get("name").and_then(Value::as_str) {
                    Some(name) => format!("{path}[{}]", quote(name)),
                    None => format!("{path}[{index}]"),
                };
                rename_mismatch(old, new, from, to, kept, &item)
            }),
        (Value::Object(old), Value::Object(new)) if old.len() == new.len() => {
            old.iter().find_map(|(key, old)| {
                let field = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                let Some(new) = new
                    .get(key)
                    .or_else(|| renamed(key).and_then(|renamed| new.get(&renamed)))
                else {
                    return Some(format!("{field} is gone"));
                };
                if NOT_PART_NAMES.contains(&key.as_str()) {
                    (old != new).then(|| format!("{field} is {new} instead of {old}"))
                } else {
                    rename_mismatch(old, new, from, to, kept, &field)
                }
            })
        }
        _ if before == after => None,
        _ => differs(),
    }
}

pub(crate) struct Edit<'a> {
    pub(crate) source: &'a RuleProgramSource,
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

    /// " on line N", N the line that makes part `other`, for a refusal of a
    /// name the program already gives it; the name typed is what changes,
    /// so the refusal stays on it.
    fn made_on(&self, other: &str) -> String {
        let other = Edit {
            name: other,
            ..*self
        };
        (other.found_call())
            .map_or_else(String::new, |call| format!(" on line {}", call.lines.first))
    }

    /// As [`refused`], naming `line` of the program as the one to change.
    fn refused_on(
        &self,
        line: usize,
        code: &str,
        message: impl Into<String>,
    ) -> RuleProgramApplyError {
        refused_at(self.source, line, code, message)
    }

    pub(crate) fn found_call(&self) -> Option<CreatingCall> {
        let recorded = self.evaluated.part_sources.get(self.name)?;
        // A copy is made by its own line, not by the one that made its source
        // (review 2026-10-24 WB-23).
        let inherited = self.inherited();
        let own: Vec<_> = (recorded.iter())
            .filter(|lines| !inherited.contains(lines))
            .copied()
            .collect();
        source_edit::creating_call(&self.source.file_name, self.text(), &own)
    }

    fn inherited(&self) -> &[SourceLines] {
        (self.evaluated.inherited_sources.get(self.name)).map_or(&[], Vec::as_slice)
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
            return Err(self.refused_on(
                line,
                "program_part_in_loop",
                format!(
                    "part {name} is made in a loop on line {line}, with the other parts of the loop; {doing} in the program"
                ),
            ));
        }
        if let Some(function) = &call.function {
            return Err(self.refused_on(
                call.lines.first,
                "program_part_in_function",
                format!(
                    "part {name} is made by the program's function {function}() called on line {}; {doing} in the program",
                    call.lines.first
                ),
            ));
        }
        let (made, changed) = self.made_along(&call);
        if !made.is_empty() || !changed.is_empty() {
            let mut also = Vec::new();
            if !made.is_empty() {
                also.push(format!("makes {} too", listed(&made)));
            }
            if !changed.is_empty() {
                also.push(format!("changes {}", listed(&changed)));
            }
            return Err(self.refused_on(
                call.lines.first,
                "program_part_made_with_others",
                format!(
                    "part {name} is one of what {}() on line {} makes, which also {}; {doing} in the program",
                    self.callee(&call),
                    call.lines.first,
                    also.join(" and ")
                ),
            ));
        }
        Ok(call)
    }

    /// The other parts the creating `call` makes, and the parts another call
    /// makes that it changes: `window()` makes a frame and a pane and cuts
    /// its wall, so one of them is not changed alone (review 2026-10-24 WB-25).
    pub(crate) fn made_along(&self, call: &CreatingCall) -> (Vec<String>, Vec<String>) {
        let (mut made, mut changed) = (Vec::new(), Vec::new());
        for (other, recorded) in &self.evaluated.part_sources {
            let inherited = self.evaluated.inherited_sources.get(other);
            if other == self.name
                || self.evaluated.model.part(other).is_none()
                || !recorded.contains(&call.lines)
                || inherited.is_some_and(|lines| lines.contains(&call.lines))
            {
                continue;
            }
            let theirs = Edit {
                source: self.source,
                evaluated: self.evaluated,
                name: other,
            }
            .found_call();
            if theirs.is_some_and(|theirs| theirs.lines == call.lines) {
                made.push(other.clone());
            } else {
                changed.push(other.clone());
            }
        }
        (made, changed)
    }

    /// The name of the function `call` calls.
    pub(crate) fn callee<'s>(&'s self, call: &CreatingCall) -> &'s str {
        let text = &self.text()[call.bytes.clone()];
        text.split('(').next().unwrap_or(text).trim()
    }

    fn delete(&self) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let doing = "delete it";
        let name = quote(self.name);
        let call = self.creating_call(doing)?;
        let Some(statement) = call.statement.clone() else {
            if let Some(line) = call.condition_line {
                return Err(self.refused_on(
                    line,
                    "program_part_in_condition",
                    format!(
                        "part {name} is made under the if on line {line}; {doing} in the program"
                    ),
                ));
            }
            return Err(self.refused_on(
                call.lines.first,
                "program_part_shared_line",
                format!(
                    "part {name} is made on line {} together with other values; {doing} in the program",
                    call.lines.first
                ),
            ));
        };
        let mut removed = vec![statement];
        for lines in &self.evaluated.part_sources[self.name] {
            // The lines that made a copy's source stay with the source
            // (review 2026-10-24 WB-23).
            if *lines == call.lines || self.inherited().contains(lines) {
                continue;
            }
            if let Some((other, _)) = self
                .evaluated
                .part_sources
                .iter()
                .find(|(other, recorded)| other.as_str() != self.name && recorded.contains(lines))
            {
                return Err(self.refused_on(
                    lines.first,
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
                return Err(self.refused_on(
                    lines.first,
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
        // A text, a material or a parameter that reads the same is no use of
        // the part (review 2026-10-24 WB-24).
        let unused = self.unused_name();
        let made = self.naming_literal(&call);
        if let Some((_, line)) =
            source_edit::string_literals(&self.source.file_name, self.text(), self.name)
                .into_iter()
                .find(|(bytes, _)| {
                    !removed
                        .iter()
                        .any(|gone| gone.start <= bytes.start && bytes.end <= gone.end)
                        && self.names_the_part(made.as_ref(), bytes, &unused)
                })
        {
            return Err(self.refused_on(
                line,
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
            let line = location.map(|location| self.original_line(&removed, location.at.line));
            let at = line.map_or_else(String::new, |line| format!(" on line {line}"));
            let message = format!("part {name} is still used{at}: {message}");
            return Err(match line {
                Some(line) => self.refused_on(line, "program_part_referenced", message),
                None => refused("program_part_referenced", message),
            });
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
        let rewritten = self.with(vec![self.colour_edit(color)?]);
        let mut expected = self.evaluated.model.clone();
        expected
            .parts
            .iter_mut()
            .filter(|part| part.name == self.name)
            .for_each(|part| part.color = color);
        Ok((rewritten, Expected::Same(Box::new(expected))))
    }

    /// The bytes of the creating call that give the part `color`, and their text.
    fn colour_edit(
        &self,
        color: Option<[u8; 3]>,
    ) -> Result<(Range<usize>, String), RuleProgramApplyError> {
        let call = self.creating_call("change its colour")?;
        let written = color.map(|[r, g, b]| format!("color=({r}, {g}, {b})"));
        Ok(match (call.color, written) {
            (Some((argument, _)), Some(written)) => (argument, written),
            (Some((argument, _)), None) => {
                let before = self.text()[..argument.start].trim_end();
                let start = before.strip_suffix(',').map_or(argument.start, str::len);
                (start..argument.end, String::new())
            }
            (None, Some(written)) => match call.arguments_end {
                Some(end) => (end..end, format!(", {written}")),
                None => {
                    let inside = self.text()[..call.bytes.end - 1].trim_end().len();
                    (inside..inside, written)
                }
            },
            (None, None) => (0..0, String::new()),
        })
    }

    /// `document_name` is the part's name in the document, which its new
    /// name continues.
    fn rename(
        &self,
        new: &str,
        document_name: &str,
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let doing = "rename it";
        let name = quote(self.name);
        if new.trim().is_empty() {
            return Err(refused(
                "program_part_name_taken",
                "a part needs a name; type one no part of the program has",
            ));
        }
        if self.evaluated.model.part(new).is_some() {
            return Err(refused(
                "program_part_name_taken",
                format!(
                    "the program already makes a part named {}{}; choose another name",
                    quote(new),
                    self.made_on(new)
                ),
            ));
        }
        if let Some(used) = name_use(&self.evaluated.model, self.name, new) {
            return Err(refused(
                "program_part_name_taken",
                format!(
                    "the program already uses {} as {used}; choose another name",
                    quote(new)
                ),
            ));
        }
        let call = self.creating_call(doing)?;
        let literals = source_edit::string_literals(&self.source.file_name, self.text(), self.name);
        let Some(made) = self.naming_literal(&call) else {
            return Err(self.refused_on(
                call.lines.first,
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
        let unused = self.unused_name();
        for (bytes, line) in literals {
            if bytes == made {
                continue;
            }
            if self.names_the_part(Some(&made), &bytes, &unused) {
                edits.push((bytes, quote(new)));
            } else {
                kept.push(line);
            }
        }
        let renamed = self.with(edits);
        // The renamed part keeps the document part, and its ID, of the old name.
        let was = quote(document_name);
        let rewritten = match source_edit::continuation_of(&renamed.file_name, &renamed.source, new)
        {
            Some(bytes) => with_edits(&renamed, vec![(bytes, was)]),
            None => appended(
                &renamed,
                &format!("continue_part({}, was={was})\n", quote(new)),
            ),
        };
        Ok((
            rewritten,
            Expected::Renamed {
                from: self.name.to_owned(),
                to: new.to_owned(),
                was: document_name.to_owned(),
                kept,
            },
        ))
    }

    /// Whether the string literal at `bytes` names this part: read as
    /// `other`, a name no part has, the program stops at that name, and no
    /// longer once the literal `made` that names the part is `other` too. A
    /// face or a kind that reads the same fails either way (review 2026-10-24
    /// WB-24).
    fn names_the_part(
        &self,
        made: Option<&Range<usize>>,
        bytes: &Range<usize>,
        other: &str,
    ) -> bool {
        let stops_at_other = |edits: Vec<(Range<usize>, String)>| {
            matches!(
                evaluate(&self.with(edits)),
                Err(RuleProgramApplyError::Program { message, .. })
                    if message.contains(&format!("{other:?}"))
            )
        };
        stops_at_other(vec![(bytes.clone(), quote(other))])
            && made.is_none_or(|made| {
                !stops_at_other(vec![
                    (made.clone(), quote(other)),
                    (bytes.clone(), quote(other)),
                ])
            })
    }

    /// The string literal of the creating call that is the part's name: the
    /// second one of `mirrored("a", "a mirrored", ...)` (review 2026-10-24 WB-23).
    fn naming_literal(&self, call: &CreatingCall) -> Option<Range<usize>> {
        source_edit::string_literals(&self.source.file_name, self.text(), self.name)
            .into_iter()
            .map(|(bytes, _)| bytes)
            .find(|bytes| call.bytes.start <= bytes.start && bytes.end <= call.bytes.end)
    }

    /// A name nothing of the program has, nor its source mentions.
    fn unused_name(&self) -> String {
        let taken = names(&self.evaluated.model);
        (1_usize..)
            .map(|n| format!("{} gone {n}", self.name))
            .find(|name| !taken.contains(name) && !self.text().contains(name.as_str()))
            .expect("some name is unused")
    }

    fn mirror(
        &self,
        copy: &str,
        point: [f64; 3],
        normal: [f64; 3],
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let length = normal.iter().map(|value| value * value).sum::<f64>().sqrt();
        if self.evaluated.model.part(copy).is_some() {
            return Err(refused(
                "program_part_mirror",
                format!(
                    "the program already makes a part named {}{}; choose another name for the mirrored copy",
                    quote(copy),
                    self.made_on(copy)
                ),
            ));
        }
        if length <= 0.0 || !length.is_finite() {
            return Err(refused(
                "program_part_mirror",
                format!(
                    "the mirrored copy of part {} needs a plane with a direction",
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
        let changes = self.changed_besides(reading, &after.model);
        Some(if changes.is_empty() {
            Driven::Alone(rewritten)
        } else {
            Driven::Shared {
                parameter: parameter.clone(),
                changes,
            }
        })
    }

    /// What `after` makes differently besides what `reading` reads of this
    /// part: other parts and tools, other values of this part, and what the
    /// program declares besides parts, each named for a message.
    fn changed_besides(&self, reading: &Reading, after: &ProgramModel) -> Vec<String> {
        let before = &self.evaluated.model;
        let mut changes = (self.other_parts_changed(after).iter())
            .map(|name| quote(name))
            .collect::<Vec<_>>();
        if let (Some(old), Some(new)) = (before.part(self.name), after.part(self.name)) {
            changes.extend(own_values_changed(old, new, reading));
        }
        changes.extend(declarations_changed(before, after));
        changes
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
            number_literal(written(distance)),
            quote(&self.push_pull_name())
        );
        let reading = Reading::Face(written_face.to_owned());
        Ok((self.appended(&line), self.reads(reading, written(distance))))
    }

    /// "GUI Push/Pull N" for the part's next Push/Pull: N one past its
    /// Push/Pulls, or the next number no feature of the part is named with,
    /// as a deleted line can leave that name taken (review 2026-10-24 WB-22).
    fn push_pull_name(&self) -> String {
        let part = self.evaluated.model.part(self.name);
        let taken = part.map_or_else(BTreeSet::new, |part| {
            (part.features.iter().map(|made| made.name.as_str()))
                .chain(part.face_offsets().map(|offset| offset.name.as_str()))
                .collect()
        });
        let count = part.map_or(0, |part| part.face_offsets().count());
        (count + 1..)
            .map(|n| format!("GUI Push/Pull {n}"))
            .find(|name| !taken.contains(name.as_str()))
            .expect("some number is free")
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
            number_literal(written(amount))
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
        if driven.is_none()
            && let Some(rewritten) = self.literal_driven(&reading, value)
        {
            return Ok((rewritten, self.reads(reading, value)));
        }
        let what = format!("{feature} · {path}");
        if let Some(refusal) =
            (driven.as_ref()).and_then(|driven| driven.out_of_range(self.name, &what))
        {
            return Err(refusal);
        }
        // The line that writes the value: the operation that makes the
        // feature (a push_pull(), a fillet()), else the creating call.
        let written_on = self
            .line_making(&reading)
            .or_else(|| self.found_call().map(|call| call.lines.first));
        let line = written_on.map_or_else(String::new, |line| format!(" on line {line}"));
        let refusal = |code: &str, message: String| match written_on {
            Some(line) => self.refused_on(line, code, message),
            None => refused(code, message),
        };
        if let Some(Driven::Shared { parameter, changes }) = driven {
            let more = changes.len().saturating_sub(5);
            let mut named = changes.into_iter().take(5).collect::<Vec<_>>();
            if more > 0 {
                named.push(format!("{more} more"));
            }
            return Err(refusal(
                "program_parameter_shared",
                format!(
                    "param({}) sets {feature} · {path} of part {name} and also changes {}; set the parameter {} to change them all, or change part {name}{line} in the program",
                    quote(&parameter),
                    named.join(", "),
                    quote(&parameter)
                ),
            ));
        }
        Err(refusal(
            "program_parameter_not_driven",
            format!(
                "{feature} · {path} of part {name} is not set by one param(); change it{line} in the program"
            ),
        ))
    }

    /// The program with the one number literal of the part's creating call,
    /// outside loops and the program's functions, that makes the part read
    /// `value` while nothing else the program makes changes, written in
    /// place as Move writes `at=` (review 2026-10-24 WB-26). `None` when no
    /// literal does.
    fn literal_driven(&self, reading: &Reading, value: f64) -> Option<RuleProgramSource> {
        /// Probes past these many literals of one call are not worth their evaluations.
        const MOST_LITERALS: usize = 32;
        let call = (self.found_call())
            .filter(|call| call.loop_line.is_none() && call.function.is_none())?;
        let baseline = reading.of(&self.evaluated.model, self.name)?;
        let literals =
            source_edit::number_literals(&self.source.file_name, self.text(), call.bytes);
        literals
            .into_iter()
            .take(MOST_LITERALS)
            .find_map(|(bytes, current)| {
                // not a tolerance: a probe step large enough to change the evaluated model.
                let step = current.abs().mul_add(1.0e-4, 1.0e-3);
                let probe = self.with(vec![(bytes.clone(), number_literal(current + step))]);
                let probed = reading.of(&evaluate(&probe).ok()?.model, self.name)?;
                let slope = (probed - baseline) / step;
                if slope.abs() <= ACCUMULATED_ROUNDING {
                    return None;
                }
                let replacement = written(current + (value - baseline) / slope);
                let rewritten = self.with(vec![(bytes, number_literal(replacement))]);
                let after = evaluate(&rewritten).ok()?;
                let reads = reading
                    .of(&after.model, self.name)
                    .is_some_and(|read| (read - value).abs() <= ACCUMULATED_ROUNDING);
                (reads && self.changed_besides(reading, &after.model).is_empty())
                    .then_some(rewritten)
            })
    }

    /// The first line besides the part's creating call without which the
    /// part no longer reads what `reading` reads: the operation that makes it.
    fn line_making(&self, reading: &Reading) -> Option<usize> {
        let before = reading.of(&self.evaluated.model, self.name)?;
        let creating = self.found_call().map(|call| call.lines);
        let mut lines = self.evaluated.part_sources.get(self.name)?.clone();
        lines.sort_by_key(|lines| lines.first);
        lines.into_iter().find_map(|lines| {
            if Some(lines) == creating {
                return None;
            }
            let statement =
                source_edit::top_level_call_statement(&self.source.file_name, self.text(), lines)?;
            let without = self.with(vec![(statement, String::new())]);
            let read = evaluate(&without)
                .ok()
                .and_then(|after| reading.of(&after.model, self.name));
            read.is_none_or(|read| (read - before).abs() > ACCUMULATED_ROUNDING)
                .then_some(lines.first)
        })
    }

    /// The part of the extrusion `feature`, `pad` long as the program writes
    /// it, made `value` long in all: by the one `param()` that alone makes
    /// its length, else by one `push_pull()` of its `end` named [`LENGTH`].
    /// A value typed again writes that line again, so it replaces the first
    /// value instead of adding to it, unless a later line reads the part and
    /// another part would change too: then the change is added at the end. The Push/Pulls of other lines stay, so
    /// the extrusion is made `value` less what they add, and a value they
    /// leave no extrusion above 0 for is refused.
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
        if !value.is_finite() || value <= DEFAULT_LINEAR_TOLERANCE_MM {
            return Err(self.too_short(feature, value));
        }
        let extrusion = value - others;
        if extrusion <= DEFAULT_LINEAR_TOLERANCE_MM {
            let lines = (part.face_offsets())
                .filter(|offset| offset.name != LENGTH)
                .filter_map(|offset| {
                    source_edit::named_call_statement(
                        &self.source.file_name,
                        self.text(),
                        "push_pull",
                        self.name,
                        &[("name", &offset.name)],
                    )
                })
                .map(|bytes| self.line_of(bytes.start).to_string())
                .collect::<Vec<_>>();
            let pulls = match lines.as_slice() {
                [] => "the Push/Pulls of other lines".to_owned(),
                [line] => format!("the Push/Pull on line {line}"),
                lines => format!("the Push/Pulls on lines {}", lines.join(", ")),
            };
            return Err(refused(
                "program_length_not_positive",
                format!(
                    "{feature} of part {name} would be {} mm long for the part to be {} mm long with {pulls}; type a length above {}",
                    number_literal(written(extrusion)),
                    number_literal(written(value)),
                    number_literal(written(others))
                ),
            ));
        }
        // With no line of its own yet, the one param() that makes the whole
        // length alone is overridden; once the line is there, it is written
        // again, so a parameter never adds to it.
        let shared = match self.driven(&Reading::Length, value) {
            Some(Driven::Alone(rewritten)) if written_before.is_none() => {
                return Ok((rewritten, self.reads(Reading::Length, value)));
            }
            Some(driven) => {
                // The length line would pass by the range the program
                // declares for the parameter that makes the length.
                if let Some(refusal) = driven.out_of_range(self.name, feature) {
                    return Err(refusal);
                }
                match driven {
                    Driven::Shared { parameter, .. } => Some(parameter),
                    _ => None,
                }
            }
            None => None,
        };
        let distance = written(extrusion - pad);
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
        if earlier.is_none() && written_before.is_some() {
            return Err(refused(
                "program_length_not_written",
                format!(
                    "the push_pull() named {} of part {name} is not a line of its own on face {}; change it in the program",
                    quote(LENGTH),
                    quote(end)
                ),
            ));
        }
        let build = |line: Option<String>| match &earlier {
            Some(bytes) => {
                let in_place = self.with(vec![(bytes.clone(), line.unwrap_or_default())]);
                // A line after the length line that reads the part (a
                // mirrored copy, a part placed at its end) would change with
                // it (review 2026-10-24 WB-20): the change is then added at
                // the end, where nothing reads it.
                let alone = evaluate(&in_place).is_ok_and(|after| {
                    self.other_parts_changed(&after.model).is_empty()
                        && declarations_changed(&self.evaluated.model, &after.model).is_empty()
                });
                let now = length.expect("an extrusion has a length");
                if alone {
                    in_place
                } else if (value - now).abs() <= DEFAULT_LINEAR_TOLERANCE_MM {
                    self.source.clone()
                } else {
                    self.appended(&format!(
                        "push_pull({name}, face={}, distance={}, name={})\n",
                        quote(end),
                        number_literal(written(value - now)),
                        quote(&self.push_pull_name())
                    ))
                }
            }
            None => line.map_or_else(|| self.source.clone(), |line| self.appended(&line)),
        };
        // A length a shared param() makes is written against that parameter
        // (`distance=10 - H`), so setting the parameter later leaves the typed
        // length (review 2026-10-24 WB-21). Only where the program names the
        // parameter at its top level and the line keeps the length when the
        // parameter changes; else the number.
        // The line written so before no longer changes with the parameter,
        // so the parameter is the one it names.
        let shared = shared.or_else(|| {
            let statement = &self.text()[earlier.clone()?];
            let (_, rest) = statement.split_once("distance=")?;
            let (_, variable) = rest.split_once(',')?.0.split_once(" - ")?;
            (self.evaluated.model.params.iter())
                .find(|made| {
                    parameter_variable(self.text(), &made.name).as_deref() == Some(variable.trim())
                })
                .map(|made| made.name.clone())
        });
        let against = line.as_ref().and(shared.as_ref()).and_then(|parameter| {
            let variable = parameter_variable(self.text(), parameter)?;
            let candidate = build(Some(format!(
                "push_pull({name}, face={}, distance={} - {variable}, name={})\n",
                quote(end),
                number_literal(written(extrusion)),
                quote(LENGTH)
            )));
            self.keeps_length(&candidate, parameter, value)
                .then_some(candidate)
        });
        // Review 2026-10-26 WB-21: a length the parameter makes through any other
        // expression (`H - 2 * T + 16`), or written so before, is written as the
        // length itself (`to=`), so it stays whatever makes the part so far.
        let written_to = earlier
            .as_ref()
            .is_some_and(|bytes| self.text()[bytes.clone()].contains(" to="));
        let absolute = || {
            let write = |to: f64| {
                build(Some(format!(
                    "push_pull({name}, face={}, to={}, name={})\n",
                    quote(end),
                    number_literal(written(to)),
                    quote(LENGTH)
                )))
            };
            // What Push/Pulls after the line add is taken off the length it writes.
            let first = write(value);
            let made = evaluate(&first)
                .ok()
                .and_then(|after| Reading::Length.of(&after.model, self.name))?;
            let candidate = if (made - value).abs() <= ACCUMULATED_ROUNDING {
                first
            } else {
                write(2.0 * value - made)
            };
            let holds = match &shared {
                Some(parameter) => self.keeps_length(&candidate, parameter, value),
                None => evaluate(&candidate)
                    .ok()
                    .and_then(|after| Reading::Length.of(&after.model, self.name))
                    .is_some_and(|length| (length - value).abs() <= ACCUMULATED_ROUNDING),
            };
            holds.then_some(candidate)
        };
        let absolute = (against.is_none() && line.is_some() && (shared.is_some() || written_to))
            .then(absolute)
            .flatten();
        let rewritten = against.or(absolute).unwrap_or_else(|| build(line));
        Ok((rewritten, self.reads(Reading::Length, value)))
    }

    /// `source` makes this part `value` long, and still does with
    /// `parameter` set to another value within its range.
    fn keeps_length(&self, source: &RuleProgramSource, parameter: &str, value: f64) -> bool {
        let reads = |source: &RuleProgramSource| {
            evaluate(source)
                .ok()
                .and_then(|after| Reading::Length.of(&after.model, self.name))
                .is_some_and(|length| (length - value).abs() <= ACCUMULATED_ROUNDING)
        };
        let Some(made) = (self.evaluated.model.params.iter()).find(|made| made.name == parameter)
        else {
            return false;
        };
        let current = source
            .overrides
            .get(parameter)
            .copied()
            .unwrap_or(made.value);
        let within = |value: f64| {
            made.min.is_none_or(|minimum| value >= minimum)
                && made.max.is_none_or(|maximum| value <= maximum)
        };
        // not a tolerance: a probe step large enough to change the evaluated model.
        let step = current.abs().mul_add(0.1, 1.0);
        let Some(probed) = [current + step, current - step]
            .into_iter()
            .find(|v| within(*v))
        else {
            return false;
        };
        let mut probe = source.clone();
        probe.overrides.insert(parameter.to_owned(), probed);
        reads(source) && reads(&probe)
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

/// The variable a top-level line `NAME = param("parameter", ...)` of `text`
/// gives the parameter's value.
fn parameter_variable(text: &str, parameter: &str) -> Option<String> {
    let literals = [format!("\"{parameter}\""), format!("'{parameter}'")];
    text.lines().find_map(|line| {
        let (variable, value) = line.split_once('=')?;
        let variable = variable.trim_end();
        let call = value.trim_start().strip_prefix("param(")?.trim_start();
        let identifier = !variable.is_empty()
            && !variable.starts_with(|c: char| c.is_ascii_digit())
            && variable
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        (identifier
            && literals
                .iter()
                .any(|literal| call.starts_with(literal.as_str())))
        .then(|| variable.to_owned())
    })
}

/// The values of part `old` besides what `reading` reads that `new` makes
/// differently, named for a message: what the part is and how it is turned,
/// the numbers of its feature tree and where its holes are on their faces.
/// The points of a sketch the edit sets move together, so they are the one
/// value, but a box's width and depth are two; an extrusion's length is its
/// Pad with the Push/Pulls of the faces read.
fn own_values_changed(old: &Part, new: &Part, reading: &Reading) -> Vec<String> {
    let name = quote(&old.name);
    let mut changes = Vec::new();
    if let (serde_json::Value::Object(before), serde_json::Value::Object(after)) =
        (skeleton(old), skeleton(new))
    {
        let keys = before.keys().chain(after.keys()).collect::<BTreeSet<_>>();
        changes.extend(
            (keys.into_iter())
                .filter(|key| before.get(*key) != after.get(*key))
                .map(|key| format!("{} of part {name}", key.replace('_', " "))),
        );
    }
    let offset_face = |feature: &str| {
        old.operations.iter().find_map(|operation| match operation {
            ProgramOperation::FaceOffset(offset) if offset.name == feature => {
                Some(offset.face.as_str())
            }
            _ => None,
        })
    };
    let caps = match &old.body {
        ProgramPartBody::Extrusion { caps, .. } => Some(caps),
        _ => None,
    };
    let read = |feature: &ProgramFeature, path: &str| match reading {
        Reading::Feature {
            feature: read,
            path: at,
        } => {
            feature.name == *read
                && (path == at
                    || (feature.kind == ProgramFeatureKind::Sketch
                        && !(feature.parameters.iter())
                            .any(|made| made.path.starts_with("bounds."))))
        }
        Reading::Length => {
            feature.kind == ProgramFeatureKind::Pad
                || offset_face(&feature.name).is_some_and(|face| {
                    caps.is_some_and(|caps| {
                        caps.iter().any(|cap| face.split('#').next() == Some(cap))
                    })
                })
        }
        Reading::Face(face) => {
            (feature.kind == ProgramFeatureKind::Pad
                && caps.is_some_and(|caps| face == "end" || *face == caps[1]))
                || offset_face(&feature.name) == Some(face.as_str())
        }
        Reading::Operations => false,
    };
    let values = |part: &Part| {
        let mut values = BTreeMap::new();
        for feature in &part.features {
            if feature.kind != ProgramFeatureKind::Workplane {
                for made in &feature.parameters {
                    values.insert((feature.name.clone(), made.path.clone()), made.value);
                }
            }
        }
        values
    };
    let now = values(new);
    for feature in old.features.iter() {
        for made in &feature.parameters {
            let key = (feature.name.clone(), made.path.clone());
            if feature.kind != ProgramFeatureKind::Workplane
                && now.get(&key) != Some(&made.value)
                && !read(feature, &made.path)
            {
                changes.push(format!("{} · {} of part {name}", key.0, key.1));
            }
        }
    }
    for (was, hole) in old.holes().zip(new.holes()) {
        if was.at_mm != hole.at_mm {
            changes.push(format!("where hole {} is on part {name}", quote(&was.id)));
        }
    }
    changes
}

/// A part without the numbers its feature tree lists or that follow from
/// them: its size, its place, the points and lengths of its body and of
/// its operations. A shell's thickness and a swept or lofted body, which
/// the tree does not list, stay.
fn skeleton(part: &Part) -> serde_json::Value {
    fn zeroed(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Number(_) => *value = 0.into(),
            serde_json::Value::Array(items) => items.iter_mut().for_each(zeroed),
            serde_json::Value::Object(fields) => fields.values_mut().for_each(zeroed),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(part).expect("parts serialize");
    if let Some(fields) = value.as_object_mut() {
        for derived in ["size_mm", "at_mm", "features"] {
            fields.remove(derived);
        }
        if matches!(
            part.body,
            ProgramPartBody::Extrusion { .. } | ProgramPartBody::Revolve { .. }
        ) && let Some(body) = fields.get_mut("body")
        {
            zeroed(body);
        }
        if let Some(serde_json::Value::Array(operations)) = fields.get_mut("operations") {
            for (operation, made) in operations.iter_mut().zip(&part.operations) {
                if !matches!(made, ProgramOperation::Shell(_)) {
                    zeroed(operation);
                }
            }
        }
    }
    value
}

/// What the program declares besides its parameters, parts and tools
/// (joints, loads, expectations, ...) that `after` declares differently,
/// named for a message.
fn declarations_changed(before: &ProgramModel, after: &ProgramModel) -> Vec<String> {
    let fields = |model: &ProgramModel| {
        let mut value = serde_json::to_value(model).expect("models serialize");
        let mut fields = value
            .as_object_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        for made in ["params", "parts", "tools"] {
            fields.remove(made);
        }
        fields
    };
    let (before, after) = (fields(before), fields(after));
    let keys = before.keys().chain(after.keys()).collect::<BTreeSet<_>>();
    keys.into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .map(|key| format!("the {} of the program", key.replace('_', " ")))
        .collect()
}

/// What overriding the one `param()` that drives a part's value does.
enum Driven {
    /// The overridden source, which changes this part alone.
    Alone(RuleProgramSource),
    /// The `param()` changes these too: other parts it changes, adds or
    /// removes, other values of this part, what the program declares.
    Shared {
        parameter: String,
        changes: Vec<String>,
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
