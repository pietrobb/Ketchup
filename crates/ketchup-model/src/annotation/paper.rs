//! How a dimension and a leader are drawn on paper, the same in every drawing
//! Kečup writes (the plan DXF, the sheets of a drawing set, a drawing sheet):
//! the dimension line with an oblique tick at each end, the extension lines
//! from the measured points, where the value sits, and a leader's filled
//! arrowhead. Points lie on a page that runs y up; `paper` is how many of its
//! units one millimetre on paper takes (the scale's N for a plan DXF in model
//! millimetres at 1:N, 1 on a sheet).

use crate::tolerance::ROUNDING;

/// On paper: the gap between a measured point and its extension line, how far
/// the extension line runs past the dimension line, the length of an oblique
/// tick, the gap between a dimension line and its value, and the length and
/// width of a leader's arrowhead.
pub const EXTENSION_GAP_MM: f64 = 1.0;
pub const EXTENSION_BEYOND_MM: f64 = 2.0;
pub const TICK_MM: f64 = 3.0;
pub const VALUE_GAP_MM: f64 = 1.0;
pub const ARROW_LENGTH_MM: f64 = 3.0;
pub const ARROW_WIDTH_MM: f64 = 1.0;

/// The marks of an aligned dimension.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionMarks {
    /// The dimension line, a tick at each of its ends, and the extension line
    /// of each measured point not lying on it.
    pub lines: Vec<[[f64; 2]; 2]>,
    /// The middle of the value's baseline, [`VALUE_GAP_MM`] above the line.
    pub value_at: [f64; 2],
    /// The value's counter-clockwise turn: it reads from left to right, or
    /// from bottom to top.
    pub value_angle_degrees: f64,
}

/// The marks of the dimension of `measured` whose line is `line`; `None` for
/// a line of no length.
#[must_use]
pub fn dimension_marks(
    measured: [[f64; 2]; 2],
    line: [[f64; 2]; 2],
    paper: f64,
) -> Option<DimensionMarks> {
    let along = direction(line[0], line[1])?;
    let mut lines = vec![line];
    let tick = TICK_MM * paper / 2.0 / std::f64::consts::SQRT_2;
    let slant = [(along[0] - along[1]) * tick, (along[0] + along[1]) * tick];
    for end in line {
        lines.push([offset(end, slant, -1.0), offset(end, slant, 1.0)]);
    }
    let (gap, beyond) = (EXTENSION_GAP_MM * paper, EXTENSION_BEYOND_MM * paper);
    for (point, end) in measured.into_iter().zip(line) {
        if distance(point, end) <= gap {
            continue;
        }
        let Some(toward) = direction(point, end) else {
            continue;
        };
        lines.push([offset(point, toward, gap), offset(end, toward, beyond)]);
    }
    let reading = if along[0] < -ROUNDING || (along[0].abs() <= ROUNDING && along[1] < 0.0) {
        [-along[0], -along[1]]
    } else {
        along
    };
    let middle = [
        f64::midpoint(line[0][0], line[1][0]),
        f64::midpoint(line[0][1], line[1][1]),
    ];
    Some(DimensionMarks {
        lines,
        value_at: offset(middle, [-reading[1], reading[0]], VALUE_GAP_MM * paper),
        value_angle_degrees: reading[1].atan2(reading[0]).to_degrees(),
    })
}

/// The corners of a leader's filled arrowhead, its tip at `point` and its base
/// toward the text at `at`, never longer than the leader; `None` when the two
/// coincide.
#[must_use]
pub fn leader_arrowhead(point: [f64; 2], at: [f64; 2], paper: f64) -> Option<[[f64; 2]; 3]> {
    let toward = direction(point, at)?;
    let base = offset(
        point,
        toward,
        (ARROW_LENGTH_MM * paper).min(distance(point, at)),
    );
    let side = [-toward[1], toward[0]];
    let half = ARROW_WIDTH_MM * paper / 2.0;
    Some([point, offset(base, side, half), offset(base, side, -half)])
}

fn direction(from: [f64; 2], to: [f64; 2]) -> Option<[f64; 2]> {
    let length = distance(from, to);
    (length > 0.0).then(|| [(to[0] - from[0]) / length, (to[1] - from[1]) / length])
}

fn offset(point: [f64; 2], by: [f64; 2], times: f64) -> [f64; 2] {
    [point[0] + by[0] * times, point[1] + by[1] * times]
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dimension_pointing_left_reads_from_left_to_right_above_its_line() {
        let marks = dimension_marks(
            [[100.0, 0.0], [0.0, 0.0]],
            [[100.0, 10.0], [0.0, 10.0]],
            2.0,
        )
        .unwrap();
        assert_eq!(marks.value_at, [50.0, 12.0]);
        assert_eq!(marks.value_angle_degrees, 0.0);
        // The line, two ticks, and the extension lines from 2 above each
        // point to 4 past the line.
        assert_eq!(marks.lines.len(), 5);
        assert_eq!(marks.lines[3], [[100.0, 2.0], [100.0, 14.0]]);
        assert_eq!(dimension_marks([[0.0; 2]; 2], [[1.0, 1.0]; 2], 1.0), None);
    }

    #[test]
    fn a_leaders_arrowhead_is_no_longer_than_the_leader() {
        assert_eq!(
            leader_arrowhead([0.0, 0.0], [10.0, 0.0], 1.0),
            Some([[0.0, 0.0], [3.0, 0.5], [3.0, -0.5]])
        );
        assert_eq!(
            leader_arrowhead([0.0, 0.0], [2.0, 0.0], 1.0).unwrap()[1],
            [2.0, 0.5]
        );
        assert_eq!(leader_arrowhead([1.0, 1.0], [1.0, 1.0], 1.0), None);
    }
}
