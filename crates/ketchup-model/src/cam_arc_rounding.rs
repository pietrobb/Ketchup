//! R1120ac-CAM-900: an arc is written on the grid of its dialect, and a controller runs the
//! written words, not the planned arc. When rounding makes the ends of a short arc meet, or puts
//! the end behind the start, the controller turns I/J into a (nearly) full circle through the part.
//! An arc whose written form turns differently from the planned one is written as a line when the
//! line stays within half a step of the planned arc, and refused otherwise.

use crate::cam::CamPostprocessorError;
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
    /// Whether the written arc runs as the planned one on a controller, or must be a line.
    pub(crate) fn as_planned(
        &self,
        start_mm: &[f64; 3],
        end_mm: &[f64; 3],
        center_mm: &[f64; 3],
        scale: f64,
    ) -> Result<Written, CamPostprocessorError> {
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
            return Ok(Written::Arc);
        }
        // The planned arc strays from its chord by its sagitta; a line is as good as the program
        // only when that is under half a step.
        let sagitta = planned_radius * (1.0 - (planned.min(std::f64::consts::PI) / 2.0).cos());
        if planned < std::f64::consts::PI && sagitta <= step / 2.0 {
            Ok(Written::Line)
        } else {
            Err(CamPostprocessorError::UnwritableArc)
        }
    }
}
