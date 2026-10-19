//! Writes a rigid move or turn of program parts, made in the editor, into the
//! program that owns them, so the program keeps owning the document. A part
//! whose creating call has a literal `at=` is moved there; otherwise the
//! program gets a `move()` / `rotate()` line for the part, as Push/Pull gets
//! `push_pull()`.
use crate::RuleProgramApplyError;
use crate::rule_program::evaluate;
use ketchup_geometry::linalg::{cross, dot, length};
use ketchup_model::document::{RuleProgramSource, Transform};
use ketchup_model::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
use ketchup_program::ProgramModel;
use ketchup_program::frame::{self, Mat3};
use ketchup_program::source_edit::{creating_call, number_literal, point_literal};
use std::collections::BTreeMap;

/// A written length or angle keeps six decimals (a nanometre, a micro-degree):
/// what the editor shows, without the noise of the matrix it came from.
pub(crate) fn written(value: f64) -> f64 {
    let value = (value * 1.0e6).round() / 1.0e6;
    if value == 0.0 { 0.0 } else { value }
}

pub(crate) fn written_point(point: [f64; 3]) -> [f64; 3] {
    point.map(written)
}

/// A world rigid motion as the program writes it: a turn about a pivot, then a shift.
#[derive(Clone, Copy, Debug)]
struct Motion {
    turn: Option<Turn>,
    shift: [f64; 3],
}

#[derive(Clone, Copy, Debug)]
struct Turn {
    axis: [f64; 3],
    angle_degrees: f64,
    pivot: [f64; 3],
}

impl Motion {
    /// `near` picks the pivot on the turn's axis line closest to it, so the
    /// written pivot sits at the parts, not at the axis' foot at the origin.
    fn of(world_edit: Transform, near: [f64; 3]) -> Result<Self, RuleProgramApplyError> {
        if !world_edit.is_rigid() {
            return Err(refused(
                "program_transform_not_rigid",
                "only a move or a turn of program parts can be written into the program; \
                 change the part's size in the program",
            ));
        }
        let m = world_edit.matrix();
        let rotation: Mat3 = [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]];
        let shift = [m[3], m[7], m[11]];
        let Some((axis, angle)) = axis_angle_of(&rotation) else {
            return Ok(Self {
                turn: None,
                shift: written_point(shift),
            });
        };
        // The pivot c on the axis solves (I - R)c = shift across the axis:
        // c = ((1 - cos)·s + sin·(a × s)) / (2(1 - cos)).
        let along = dot(shift, axis);
        let across: [f64; 3] = std::array::from_fn(|i| shift[i] - along * axis[i]);
        let (sin, cos) = angle.to_radians().sin_cos();
        let turned = cross(axis, across);
        let foot: [f64; 3] = std::array::from_fn(|i| {
            ((1.0 - cos) * across[i] + sin * turned[i]) / (2.0 * (1.0 - cos))
        });
        let toward = dot(std::array::from_fn(|i| near[i] - foot[i]), axis);
        Ok(Self {
            turn: Some(Turn {
                axis: axis.map(|value| (value * 1.0e9).round() / 1.0e9),
                angle_degrees: written(angle),
                pivot: written_point(std::array::from_fn(|i| foot[i] + toward * axis[i])),
            }),
            shift: written_point(axis.map(|value| value * along)),
        })
    }

    /// Where the motion takes a part frame (`at`, rotation), computed as the
    /// program's `rotate()` and `move()` do.
    fn frame_after(&self, at: [f64; 3], rotation: &Mat3) -> ([f64; 3], Mat3) {
        let (mut at, mut rotation) = (at, *rotation);
        if let Some(turn) = self.turn
            && let Some(turning) = frame::axis_angle(turn.axis, turn.angle_degrees)
        {
            let offset = frame::apply(&turning, std::array::from_fn(|i| at[i] - turn.pivot[i]));
            at = std::array::from_fn(|i| turn.pivot[i] + offset[i]);
            rotation = frame::multiply(&turning, &rotation);
        }
        (std::array::from_fn(|i| at[i] + self.shift[i]), rotation)
    }
}

/// Unit axis and right-handed angle in degrees (0, 180] of a rotation, or
/// `None` for no turn.
fn axis_angle_of(rotation: &Mat3) -> Option<([f64; 3], f64)> {
    if frame::is_identity(rotation) {
        return None;
    }
    let r = rotation;
    let cos = ((r[0][0] + r[1][1] + r[2][2] - 1.0) / 2.0).clamp(-1.0, 1.0);
    let twice_sin_axis = [r[2][1] - r[1][2], r[0][2] - r[2][0], r[1][0] - r[0][1]];
    let twice_sin = length(twice_sin_axis);
    if let Some(axis) = frame::normalized(twice_sin_axis) {
        return Some((axis, (twice_sin / 2.0).atan2(cos).to_degrees()));
    }
    // A half turn: R = 2aaᵀ - I, so the largest diagonal names a safe column.
    let k = (0..3)
        .max_by(|&i, &j| r[i][i].total_cmp(&r[j][j]))
        .unwrap_or(0);
    let a_k = f64::midpoint(r[k][k], 1.0).sqrt();
    let axis = std::array::from_fn(|j| if j == k { a_k } else { r[k][j] / (2.0 * a_k) });
    Some((frame::normalized(axis)?, 180.0))
}

pub(crate) fn refused(code: &str, message: impl Into<String>) -> RuleProgramApplyError {
    RuleProgramApplyError::Program {
        code: code.to_owned(),
        message: message.into(),
        location: None,
    }
}

pub(crate) fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("strings serialize")
}

/// Rewrites `source` so that each of `parts` ends where the world rigid
/// motion `world_edit` takes it, and nothing else changes. `None` when the
/// edit moves something that is not a part the program makes (a component
/// or a copy of one): this function has nothing to write for it.
///
/// # Errors
/// A motion that is not rigid, or a program that no longer evaluates or
/// does not place the parts where the edit put them.
pub fn rewrite_rule_program_transform(
    source: &RuleProgramSource,
    parts: &[String],
    world_edit: Transform,
) -> Result<Option<RuleProgramSource>, RuleProgramApplyError> {
    let evaluated = evaluate(source)?;
    let model = &evaluated.model;
    let Some(made) = parts
        .iter()
        .map(|name| model.part(name).map(|part| (name.as_str(), part)))
        .collect::<Option<Vec<_>>>()
        .filter(|made| !made.is_empty())
    else {
        return Ok(None);
    };
    let motion = Motion::of(world_edit, made[0].1.at_mm)?;
    let expected = made
        .into_iter()
        .map(|(name, part)| (name, motion.frame_after(part.at_mm, &part.rotation)))
        .collect::<BTreeMap<_, _>>();
    if motion.turn.is_none()
        && let Some(rewritten) = moved_at_literals(source, &evaluated, &expected, motion.shift)
        && evaluate(&rewritten).is_ok_and(|after| matches(model, &after.model, &expected))
    {
        return Ok(Some(rewritten));
    }
    let mut rewritten = source.clone();
    if !rewritten.source.ends_with('\n') {
        rewritten.source.push('\n');
    }
    for name in parts {
        if let Some(turn) = motion.turn {
            rewritten.source.push_str(&format!(
                "rotate({}, axis={}, angle={}, pivot={})\n",
                quote(name),
                point_literal(turn.axis),
                number_literal(turn.angle_degrees),
                point_literal(turn.pivot)
            ));
        }
        if motion.shift != [0.0; 3] {
            rewritten.source.push_str(&format!(
                "move({}, by={})\n",
                quote(name),
                point_literal(motion.shift)
            ));
        }
    }
    let after = evaluate(&rewritten)?;
    if !matches(model, &after.model, &expected) {
        return Err(refused(
            "program_transform_mismatch",
            format!(
                "the program did not place {} where the edit put it; move the parts in the program",
                parts.join(", ")
            ),
        ));
    }
    Ok(Some(rewritten))
}

/// `source` with each part's literal `at=` shifted, when every part has its
/// own creating call with one, outside any loop.
fn moved_at_literals(
    source: &RuleProgramSource,
    evaluated: &ketchup_program::Evaluated,
    expected: &BTreeMap<&str, ([f64; 3], Mat3)>,
    shift: [f64; 3],
) -> Option<RuleProgramSource> {
    let mut edits = Vec::new();
    for name in expected.keys() {
        let recorded = evaluated.part_sources.get(*name)?;
        let call = creating_call(&source.file_name, &source.source, recorded)?;
        let at = call.at.filter(|_| call.loop_line.is_none())?;
        let moved = written_point(std::array::from_fn(|i| at.value[i] + shift[i]));
        edits.push((at.bytes, point_literal(moved)));
    }
    edits.sort_by_key(|(bytes, _)| std::cmp::Reverse(bytes.start));
    // Two parts of one call (a function of the program) cannot move apart there.
    if edits.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    let mut rewritten = source.clone();
    for (bytes, text) in edits {
        rewritten.source.replace_range(bytes, &text);
    }
    Some(rewritten)
}

/// The moved parts sit where expected; every other part is as it was.
fn matches(
    before: &ProgramModel,
    after: &ProgramModel,
    expected: &BTreeMap<&str, ([f64; 3], Mat3)>,
) -> bool {
    before.parts.len() == after.parts.len()
        && before.parts.iter().zip(&after.parts).all(|(old, new)| {
            match expected.get(old.name.as_str()) {
                None => old == new,
                Some((at, rotation)) => {
                    new.name == old.name
                        && new.size_mm == old.size_mm
                        && (0..3)
                            .all(|i| (new.at_mm[i] - at[i]).abs() <= DEFAULT_LINEAR_TOLERANCE_MM)
                        && frame::same_orientation(&new.rotation, rotation)
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(source: &str) -> RuleProgramSource {
        RuleProgramSource {
            file_name: "edit.star".to_owned(),
            source: source.to_owned(),
            overrides: BTreeMap::new(),
            variants: BTreeMap::default(),
        }
    }

    fn part_at(source: &RuleProgramSource, name: &str) -> ([f64; 3], Mat3) {
        let evaluated = evaluate(source).expect("evaluates");
        let part = evaluated.model.part(name).expect("part");
        (part.at_mm, part.rotation)
    }

    #[test]
    fn move_of_a_literal_position_edits_at_in_place() {
        let source = program(
            "box(\"a\", (100, 20, 30), at=(10, 0, 5))\nbox(\"b\", (10, 10, 10), at=(0, 50, 0))\n",
        );
        let by = Transform::from_translation(25.5, -10.0, 0.0).unwrap();
        let rewritten = rewrite_rule_program_transform(&source, &["a".to_owned()], by)
            .unwrap()
            .unwrap();
        assert_eq!(
            rewritten.source,
            "box(\"a\", (100, 20, 30), at=(35.5, -10, 5))\nbox(\"b\", (10, 10, 10), at=(0, 50, 0))\n"
        );
        // 10 + 25.5 = 35.5, 0 - 10 = -10, z stays 5; "b" is untouched.
        assert_eq!(part_at(&rewritten, "a").0, [35.5, -10.0, 5.0]);
        assert_eq!(part_at(&rewritten, "b").0, [0.0, 50.0, 0.0]);
    }

    #[test]
    fn move_of_a_computed_position_appends_a_move() {
        let source = program("x = 10\nbox(\"a\", (100, 20, 30), at=(x, 0, 0))\n");
        let by = Transform::from_translation(0.0, 0.0, 40.0).unwrap();
        let rewritten = rewrite_rule_program_transform(&source, &["a".to_owned()], by)
            .unwrap()
            .unwrap();
        assert!(
            rewritten.source.ends_with("move(\"a\", by=(0, 0, 40))\n"),
            "{}",
            rewritten.source
        );
        assert_eq!(part_at(&rewritten, "a").0, [10.0, 0.0, 40.0]);
    }

    #[test]
    fn parts_of_a_loop_are_moved_after_it() {
        let source =
            program("for i in range(3):\n    box(\"s%d\" % i, (10, 10, 10), at=(0, 0, 20 * i))\n");
        let by = Transform::from_translation(5.0, 0.0, 0.0).unwrap();
        let rewritten = rewrite_rule_program_transform(&source, &["s1".to_owned()], by)
            .unwrap()
            .unwrap();
        assert!(rewritten.source.ends_with("move(\"s1\", by=(5, 0, 0))\n"));
        assert_eq!(part_at(&rewritten, "s1").0, [5.0, 0.0, 20.0]);
        assert_eq!(part_at(&rewritten, "s2").0, [0.0, 0.0, 40.0]);
    }

    #[test]
    fn quarter_turn_about_a_point_is_written_as_rotate() {
        let source = program("box(\"a\", (100, 20, 30), at=(10, 0, 0))\n");
        // A quarter turn about the vertical line through (60, 10): x' = 60 - (y - 10), y' = 10 + (x - 60).
        let turn = Transform::from_matrix([
            0.0, -1.0, 0.0, 70.0, 1.0, 0.0, 0.0, -50.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
        .unwrap();
        let rewritten = rewrite_rule_program_transform(&source, &["a".to_owned()], turn)
            .unwrap()
            .unwrap();
        assert!(
            rewritten
                .source
                .ends_with("rotate(\"a\", axis=(0, 0, 1), angle=90, pivot=(60, 10, 0))\n"),
            "{}",
            rewritten.source
        );
        // The origin (10, 0, 0) goes to (60 - (0 - 10), 10 + (10 - 60), 0) = (70, -40, 0);
        // the part's local x now points along world y.
        let (at, rotation) = part_at(&rewritten, "a");
        assert!(
            (0..3).all(|i| (at[i] - [70.0, -40.0, 0.0][i]).abs() < 1.0e-9),
            "{at:?}"
        );
        assert!(frame::same_orientation(
            &rotation,
            &[[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
        ));
    }

    #[test]
    fn nothing_is_written_for_what_the_program_does_not_make_as_a_part() {
        let source = program("box(\"a\", (10, 10, 10))\n");
        let by = Transform::from_translation(1.0, 0.0, 0.0).unwrap();
        for parts in [
            vec!["copy".to_owned()],
            vec!["a".to_owned(), "copy".to_owned()],
        ] {
            assert!(
                rewrite_rule_program_transform(&source, &parts, by)
                    .unwrap()
                    .is_none()
            );
        }
    }
}
