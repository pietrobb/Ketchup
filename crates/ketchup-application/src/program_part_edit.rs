//! Writes a rigid move or turn of a program part, made in the editor, into the
//! program that owns it, so the program keeps owning the document. A part
//! whose creating call has a literal `at=` is moved there when the program
//! then makes what a `move()` line makes, tools included; otherwise the
//! program gets a `move()` / `rotate()` line for the part, as Push/Pull gets
//! `push_pull()`.
use crate::RuleProgramApplyError;
use crate::program_part_change::{Edit, Expected};
use crate::rule_program::evaluate;
use ketchup_geometry::linalg::{cross, dot, length};
use ketchup_model::document::{RuleProgramSource, Transform};
use ketchup_program::frame::{self, Mat3};
use ketchup_program::source_edit::{number_literal, point_literal, quote, written, written_point};

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
    /// written pivot sits at the part, not at the axis' foot at the origin.
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

/// As [`refused`], with line `line` of `source` as where the program must change.
pub(crate) fn refused_at(
    source: &RuleProgramSource,
    line: usize,
    code: &str,
    message: impl Into<String>,
) -> RuleProgramApplyError {
    RuleProgramApplyError::Program {
        code: code.to_owned(),
        message: message.into(),
        location: Some(ketchup_program::ErrorLocation::of_line(
            &source.file_name,
            &source.source,
            line,
        )),
    }
}

impl Edit<'_> {
    /// The part's literal `at=` shifted, when its creating call has one
    /// outside any loop and the program then makes what the `move()` line
    /// makes; otherwise a `rotate()` / `move()` line for the part.
    pub(crate) fn transform(
        &self,
        world_edit: Transform,
    ) -> Result<(RuleProgramSource, Expected), RuleProgramApplyError> {
        let model = &self.evaluated.model;
        let part = model.part(self.name).expect("checked by the caller");
        let motion = Motion::of(world_edit, part.at_mm)?;
        let (at, rotation) = motion.frame_after(part.at_mm, &part.rotation);
        let expected = Expected::Placed {
            part: self.name.to_owned(),
            at,
            rotation,
        };
        let lines = self.motion_lines(&motion);
        let appended = self.appended(&lines);
        // The literal is written only when the program then makes exactly
        // what the move() line makes: a move() carries the tools of the
        // part's booleans along, a literal `at=` does not, so a hole would
        // stay behind in the world while the part moves.
        if motion.turn.is_none()
            && let Some(call) = self.found_call().filter(|call| call.loop_line.is_none())
            && let Some(literal) = call.at
            && let Ok(moved_by_line) = evaluate(&appended)
        {
            let moved = written_point(std::array::from_fn(|i| literal.value[i] + motion.shift[i]));
            let rewritten = self.with(vec![(literal.bytes, point_literal(moved))]);
            if evaluate(&rewritten).is_ok_and(|after| {
                expected.holds(model, &after.model) && after.model == moved_by_line.model
            }) {
                return Ok((rewritten, expected));
            }
        }
        Ok((appended, expected))
    }

    /// The `rotate()` / `move()` lines that make `motion` of the part.
    fn motion_lines(&self, motion: &Motion) -> String {
        let mut lines = String::new();
        if let Some(turn) = motion.turn {
            lines.push_str(&format!(
                "rotate({}, axis={}, angle={}, pivot={})\n",
                quote(self.name),
                point_literal(turn.axis),
                number_literal(turn.angle_degrees),
                point_literal(turn.pivot)
            ));
        }
        if motion.shift != [0.0; 3] {
            let by = point_literal(motion.shift);
            lines.push_str(&format!("move({}, by={by})\n", quote(self.name)));
        }
        lines
    }
}
