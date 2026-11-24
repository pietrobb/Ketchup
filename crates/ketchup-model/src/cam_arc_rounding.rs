//! R1120ac-CAM-900: an arc is written on the grid of its dialect, and a controller runs the
//! written words, not the planned arc. When rounding makes the ends of a short arc meet, or puts
//! the end behind the start, the controller turns I/J into a (nearly) full circle through the part.
//! An arc whose line stays within half a step of it is written as that line; one whose written
//! form turns differently from the planned one is refused.

use ketchup_tolerance::CAM_ARC_GRID_STEPS;
use std::f64::consts::TAU;

pub(crate) struct WrittenArc {
    pub(crate) start_mm: [f64; 3],
    pub(crate) end_mm: [f64; 3],
    pub(crate) center_offset_mm: [f64; 2],
    pub(crate) clockwise: bool,
}

pub(crate) enum Written {
    Arc,
    Line,
}

/// Angle turned from `start` to `end` about `center` in the given direction, in `[0, TAU)`.
fn turn(start: [f64; 2], end: [f64; 2], center: [f64; 2], clockwise: bool) -> f64 {
    let angle = |point: [f64; 2]| (point[1] - center[1]).atan2(point[0] - center[0]);
    let counterclockwise = (angle(end) - angle(start)).rem_euclid(TAU);
    if clockwise {
        (TAU - counterclockwise).rem_euclid(TAU)
    } else {
        counterclockwise
    }
}

fn length(vector: [f64; 2]) -> f64 {
    vector[0].hypot(vector[1])
}

impl WrittenArc {
    /// Whether the written arc runs as the planned one on a controller, or must be a line; None
    /// where it can be neither.
    pub(crate) fn as_planned(
        &self,
        start_mm: &[f64; 3],
        end_mm: &[f64; 3],
        center_mm: &[f64; 3],
        scale: f64,
    ) -> Option<Written> {
        let step = 1.0 / scale;
        let xy = |point: &[f64; 3]| [point[0], point[1]];
        let planned_radius = length([start_mm[0] - center_mm[0], start_mm[1] - center_mm[1]]);
        // A planned full circle closes on itself.
        let planned = if xy(start_mm) == xy(end_mm) {
            TAU
        } else {
            turn(xy(start_mm), xy(end_mm), xy(center_mm), self.clockwise)
        };
        let start = xy(&self.start_mm);
        let end = xy(&self.end_mm);
        let center = [
            start[0] + self.center_offset_mm[0],
            start[1] + self.center_offset_mm[1],
        ];
        let start_radius = length(self.center_offset_mm);
        let end_radius = length([end[0] - center[0], end[1] - center[1]]);
        // The planned arc strays from its chord by its sagitta; a line is as good as the program
        // when that is at most half a step, and is written then: an arc that flat turns so little
        // that a controller may run its I/J as a full circle (review 2026-11-21 ac P3).
        let sagitta = planned_radius * (1.0 - (planned.min(std::f64::consts::PI) / 2.0).cos());
        if planned < std::f64::consts::PI && sagitta <= step / 2.0 {
            return Some(Written::Line);
        }
        // A controller runs I/J between equal ends as a full circle.
        let written = if start == end {
            TAU
        } else {
            turn(start, end, center, self.clockwise)
        };
        let tolerance = CAM_ARC_GRID_STEPS * step;
        if start_radius > tolerance
            && (start_radius - end_radius).abs() <= tolerance
            && (written - planned).abs() * planned_radius <= tolerance
        {
            Some(Written::Arc)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review 2026-11-21 ac P3: an arc of radius 2 500 mm over a chord of 0.0011 mm turns about
    /// 4.4e-7 rad, under the angle below which a controller may run I/J as a full circle; its
    /// rounded ends stay apart, and it was written as an arc. An arc within half a step of its
    /// chord is a line.
    #[test]
    fn an_arc_within_half_a_step_of_its_chord_is_a_line() {
        let scale = 1000.0;
        let round = |value: f64| (value * scale).round() / scale;
        let radius = 2500.0;
        let center = [0.0, radius, 0.0];
        let half = 0.0011 / radius / 2.0;
        let at = |angle: f64| {
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
                0.0,
            ]
        };
        let below = -std::f64::consts::FRAC_PI_2;
        let (start, end) = (at(below - half), at(below + half));
        let written = WrittenArc {
            start_mm: start.map(round),
            end_mm: end.map(round),
            center_offset_mm: [round(center[0] - start[0]), round(center[1] - start[1])],
            clockwise: false,
        };
        assert_ne!(written.start_mm, written.end_mm);
        assert!(matches!(
            written.as_planned(&start, &end, &center, scale),
            Some(Written::Line)
        ));
    }
}
