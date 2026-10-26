//! Trim, extend and offset of a straight line of an underlay's drawing. A trimmed or
//! extended line is taken out and the lines it becomes are drawn on its layer; an offset
//! draws a parallel line beside it. Each is one batch, so one Undo step.

use super::drawn::{AddedObject, DrawnShape};
use super::edits::{UnderlayObjectError, top_level, with_edits};
use super::object_query::ObjectIndex;
use super::{UnderlayId, UnderlayLines, UnderlayPlanError};
use crate::document::{CommandBatch, Snapshot};

/// What becomes of a straight line of a drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineEdit {
    /// Cuts away its piece around `pick_mm`, from the nearest line of the drawing that
    /// crosses it on one side of that point to the nearest on the other, or to its end.
    Trim { pick_mm: [f64; 2] },
    /// Its end nearer `pick_mm` reaches on to the first line of the drawing it meets.
    Extend { pick_mm: [f64; 2] },
    /// A parallel line on the side of `side_mm`, `distance_mm` away, or through `side_mm`
    /// when no distance is given.
    Offset {
        distance_mm: Option<f64>,
        side_mm: [f64; 2],
    },
}

/// Crossings this close to an end of the line, as a share of its length, are at the end.
const END: f64 = 1e-9;

/// One batch, so one Undo step, that trims, extends or offsets the straight line
/// `object_id` of the drawing of underlay `id`, whose lines as drawn now are `lines`.
pub fn plan_underlay_line_edit(
    snapshot: &Snapshot,
    id: UnderlayId,
    lines: &UnderlayLines,
    object_id: u32,
    edit: LineEdit,
) -> Result<CommandBatch, UnderlayObjectError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayObjectError::Plan(UnderlayPlanError::NotFound(id)))?;
    let objects = &lines.objects().objects;
    if objects.is_empty() {
        return Err(UnderlayObjectError::NoObjects);
    }
    let object = top_level(objects, object_id)?;
    let index = ObjectIndex::new(lines);
    let [a, b] = match index.own_polylines(object_id).as_slice() {
        [[a, b]] => [*a, *b],
        _ => return Err(UnderlayObjectError::NotALine(object_id)),
    };
    let layer = lines.objects().layer_name(object).to_owned();
    let line = |points: [[f64; 2]; 2]| AddedObject::Drawn {
        layer: layer.clone(),
        shape: DrawnShape::Polyline {
            points_mm: points.to_vec(),
            closed: false,
        },
    };
    let mut edits = underlay.edits.clone();
    let at = |t: f64| [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
    match edit {
        LineEdit::Trim { pick_mm } => {
            let picked = along(a, b, pick_mm);
            let crossings = crossings(lines, object, a, b);
            let before = crossings
                .iter()
                .copied()
                .filter(|t| *t > END && *t < picked)
                .fold(None, |most: Option<f64>, t| {
                    Some(most.map_or(t, |m| m.max(t)))
                });
            let after = crossings
                .iter()
                .copied()
                .filter(|t| *t < 1.0 - END && *t > picked)
                .fold(None, |least: Option<f64>, t| {
                    Some(least.map_or(t, |l| l.min(t)))
                });
            if before.is_none() && after.is_none() {
                return Err(UnderlayObjectError::NoCrossing(object_id));
            }
            edits.removed_objects.insert(object_id);
            if let Some(before) = before {
                edits.added.push(line([a, at(before)]));
            }
            if let Some(after) = after {
                edits.added.push(line([at(after), b]));
            }
        }
        LineEdit::Extend { pick_mm } => {
            let crossings = crossings(lines, object, a, b);
            let from_end = along(a, b, pick_mm) >= 0.5;
            let reached = if from_end {
                crossings
                    .into_iter()
                    .filter(|t| *t > 1.0 + END)
                    .min_by(f64::total_cmp)
                    .map(|t| [a, at(t)])
            } else {
                crossings
                    .into_iter()
                    .filter(|t| *t < -END)
                    .max_by(f64::total_cmp)
                    .map(|t| [at(t), b])
            };
            let reached = reached.ok_or(UnderlayObjectError::NoCrossing(object_id))?;
            edits.removed_objects.insert(object_id);
            edits.added.push(line(reached));
        }
        LineEdit::Offset {
            distance_mm,
            side_mm,
        } => {
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            let side = cross(sub(b, a), sub(side_mm, a)) / length;
            let distance = distance_mm.unwrap_or(side.abs());
            if !distance.is_finite() || distance <= 0.0 || !side.is_finite() || side == 0.0 {
                return Err(UnderlayObjectError::InvalidDistance);
            }
            // The unit normal to the left of a → b, turned to the side asked for.
            let sign = side.signum() * distance / length;
            let shift = [-(b[1] - a[1]) * sign, (b[0] - a[0]) * sign];
            let moved = |p: [f64; 2]| [p[0] + shift[0], p[1] + shift[1]];
            edits.added.push(line([moved(a), moved(b)]));
        }
    }
    Ok(with_edits(underlay, edits))
}

/// Where `point` lies along a → b: 0 at a, 1 at b.
fn along(a: [f64; 2], b: [f64; 2], point: [f64; 2]) -> f64 {
    let direction = sub(b, a);
    let squared = direction[0] * direction[0] + direction[1] * direction[1];
    (sub(point, a)[0] * direction[0] + sub(point, a)[1] * direction[1]) / squared
}

/// Where along the endless line through a and b (0 at a, 1 at b) a straight piece of the
/// drawing's lines crosses or touches it, but for the pieces of `own`.
fn crossings(
    lines: &UnderlayLines,
    own: &super::objects::UnderlayObject,
    a: [f64; 2],
    b: [f64; 2],
) -> Vec<f64> {
    let direction = sub(b, a);
    let mut found = Vec::new();
    for (layer_index, layer) in lines.layers().iter().enumerate() {
        let own_points = own
            .parts
            .iter()
            .filter(|part| super::index(part.layer) == layer_index)
            .map(|part| super::index(part.points.start)..super::index(part.points.end))
            .collect::<Vec<_>>();
        let points = layer.points_mm();
        let mut start = 0;
        for &end in layer.polyline_ends() {
            for piece in start..end.saturating_sub(1) {
                if own_points
                    .iter()
                    .any(|range| range.start <= piece && piece + 1 < range.end)
                {
                    continue;
                }
                let [c, d] = [points[piece], points[piece + 1]];
                let span = sub(d, c);
                let denominator = cross(direction, span);
                if denominator.abs() <= f64::EPSILON * (direction[0].abs() + direction[1].abs()) {
                    continue;
                }
                let to_c = sub(c, a);
                let on_piece = cross(to_c, direction) / denominator;
                if (-END..=1.0 + END).contains(&on_piece) {
                    found.push(cross(to_c, span) / denominator);
                }
            }
            start = end;
        }
    }
    found
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
