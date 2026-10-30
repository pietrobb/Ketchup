//! Trim, extend and offset of a straight line of an underlay's drawing. A trimmed or
//! extended line keeps its id and gets new ends, a second piece of a trimmed line and an
//! offset are copies of it: a line of the drawing keeps all it has but its ends (review
//! 2026-10-29 CD-61). Each is one batch, so one Undo step.

use super::drawn::{AddedObject, DrawnShape, stretch, then};
use super::edits::{UnderlayEdits, UnderlayObjectError, current_read, top_level, with_edits};
use super::object_query::ObjectIndex;
use super::objects::ObjectDetail;
use super::{Underlay, UnderlayId, UnderlayLines, UnderlayPlanError};
use crate::document::{CommandBatch, Snapshot};
use crate::tolerance::ROUNDING;
use ketchup_geometry::linalg::{cross2 as cross, dot2, sub2 as sub};
use std::ops::Range;

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

/// The kinds of the drawing's straight lines whose ends alone are written anew.
const STRETCHED_KINDS: [&str; 3] = ["LINE", "LWPOLYLINE", "POLYLINE"];

/// The kinds of object whose lines a trim or an extension does not stop at: hatches and
/// fills, and dimensions (review 2026-10-29 CD-65).
const NOT_CUTTING_KINDS: [&str; 5] = ["HATCH", "MPOLYGON", "SOLID", "TRACE", "DIMENSION"];

/// Crossings this close to an end of the line, as a share of its length, are at the end.
const END: f64 = ROUNDING;

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
    let mut edits = underlay.edits.clone();
    // What the line is now: drawn, or a copy of a line of the drawing as read.
    let current = match object.added_by {
        Some(added_by) => edits
            .added
            .get(super::index(added_by))
            .cloned()
            .ok_or(UnderlayObjectError::NotFound(object_id))?,
        None => current_read(&edits, object_id),
    };
    if current.copy_of().is_some() && !STRETCHED_KINDS.contains(&object.kind.name()) {
        return Err(UnderlayObjectError::NotALine(object_id));
    }
    // The line with the ends `ends`: a read one keeps all it has but its ends.
    let line = |ends: [[f64; 2]; 2]| match &current {
        AddedObject::Drawn { layer, .. } => Ok(AddedObject::Drawn {
            layer: layer.clone(),
            shape: DrawnShape::Polyline {
                points_mm: ends.to_vec(),
                closed: false,
            },
        }),
        AddedObject::Copy { of, map } | AddedObject::Stretched { of, map } => {
            let ends = stretch([a, b], ends).ok_or(UnderlayObjectError::NotALine(object_id))?;
            Ok(AddedObject::Stretched {
                of: *of,
                map: then(map, &ends),
            })
        }
    };
    // The line itself made `now`: it keeps its id.
    let reshape = |edits: &mut UnderlayEdits, now: AddedObject| match object.added_by {
        Some(added_by) => edits.added[super::index(added_by)] = now,
        None => edits.place_read(object_id, &now),
    };
    let at = |t: f64| [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
    match edit {
        LineEdit::Trim { pick_mm } => {
            let picked = along(a, b, pick_mm);
            let crossings = crossings(lines, &cutters(snapshot, underlay, lines, object_id), a, b);
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
            let mut pieces = before
                .map(|before| [a, at(before)])
                .into_iter()
                .chain(after.map(|after| [at(after), b]));
            let first = pieces
                .next()
                .ok_or(UnderlayObjectError::NoCrossing(object_id))?;
            let second = pieces.next().map(line).transpose()?;
            reshape(&mut edits, line(first)?);
            edits.added.extend(second);
        }
        LineEdit::Extend { pick_mm } => {
            let crossings = crossings(lines, &cutters(snapshot, underlay, lines, object_id), a, b);
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
            reshape(&mut edits, line(reached)?);
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
            edits.added.push(line([moved(a), moved(b)])?);
        }
    }
    Ok(with_edits(underlay, edits))
}

/// Where `point` lies along a → b: 0 at a, 1 at b.
fn along(a: [f64; 2], b: [f64; 2], point: [f64; 2]) -> f64 {
    let direction = sub(b, a);
    dot2(sub(point, a), direction) / dot2(direction, direction)
}

/// For each layer of `lines`, `None` when the user hides it, else the runs of its points
/// that cut nothing: those of `own`, and those of hatches, fills and dimensions and of what
/// they place (review 2026-10-29 CD-65).
fn cutters(
    snapshot: &Snapshot,
    underlay: &Underlay,
    lines: &UnderlayLines,
    own: u32,
) -> Vec<Option<Vec<Range<usize>>>> {
    let objects = &lines.objects().objects;
    let mut uncutting = vec![false; objects.len()];
    for (index, object) in objects.iter().enumerate() {
        let annotation = matches!(
            object.detail,
            ObjectDetail::Hatch { .. } | ObjectDetail::Dimension { .. }
        ) || NOT_CUTTING_KINDS.contains(&object.kind.name());
        let placed_by_one = object
            .parent
            .is_some_and(|parent| uncutting[super::index(parent)]);
        uncutting[index] = annotation || placed_by_one;
    }
    let mut cutters = (0..lines.layers().len())
        .map(|layer| {
            let shown = underlay
                .layers
                .get(layer)
                .is_none_or(|layer| snapshot.tag(layer.tag).is_none_or(|tag| tag.visible()));
            shown.then(Vec::new)
        })
        .collect::<Vec<_>>();
    let own = super::index(own);
    for (index, object) in objects.iter().enumerate() {
        if !uncutting[index] && index != own {
            continue;
        }
        for part in &object.parts {
            if let Some(Some(skipped)) = cutters.get_mut(super::index(part.layer)) {
                skipped.push(super::index(part.points.start)..super::index(part.points.end));
            }
        }
    }
    cutters
}

/// Where along the endless line through a and b (0 at a, 1 at b) a straight piece of the
/// drawing's lines crosses or touches it, of the pieces `cutters` lets cut.
fn crossings(
    lines: &UnderlayLines,
    cutters: &[Option<Vec<Range<usize>>>],
    a: [f64; 2],
    b: [f64; 2],
) -> Vec<f64> {
    let direction = sub(b, a);
    let mut found = Vec::new();
    for (layer, skipped) in lines.layers().iter().zip(cutters) {
        let Some(skipped) = skipped else {
            continue;
        };
        let points = layer.points_mm();
        let mut start = 0;
        for &end in layer.polyline_ends() {
            for piece in start..end.saturating_sub(1) {
                if skipped
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
