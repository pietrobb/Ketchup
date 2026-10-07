//! Load path (`load_path()`): the members a program declares load-bearing
//! must each pass their weight down. A part is carried when it rests on the
//! floor or is anchored, or when its centre of mass lies, in plan, over the
//! places where it rests from above on carried parts or hangs on a carried
//! part through a joint declared `bearing=True`. Touching a part from the side
//! carries nothing, and resting on one end only does not hold a beam.

use crate::contact::ContactFaces;
use crate::eval::TOLERANCE_MM;
use crate::model::{Part, ProgramModel, ProgramPartBody};
use crate::validate::{Issue, Severity};
use ketchup_geometry::linalg::cross2;
use ketchup_tolerance::ROUNDING;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LoadPath {
    pub name: String,
    /// Only parts carrying one of these tags are members; empty: every part.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub only_tags: BTreeSet<String>,
    /// Tags of other parts that may carry members (a floor deck); besides
    /// them only members and parts on the floor or anchored carry anything.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub carrier_tags: BTreeSet<String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub ignore: BTreeSet<String>,
    pub hint: String,
}

impl LoadPath {
    pub(crate) fn member(&self, part: &Part) -> bool {
        !self.ignore.contains(&part.name)
            && (self.only_tags.is_empty() || !part.tags.is_disjoint(&self.only_tags))
    }
}

/// A contact face whose outward normal points at least this much downward
/// (60° from vertical) lets the part rest on the other part.
const RESTS_ON: f64 = -0.5;

/// Where a part bears on one supporter, as world points.
struct Bearing {
    supporter: usize,
    points: Vec<[f64; 3]>,
}

/// Centre of the part's volume: the profile centroid of a straight-sided
/// extrusion, otherwise the centre of its box. Cuts are not deducted.
pub(crate) fn centre_of_mass(part: &Part) -> [f64; 3] {
    if let ProgramPartBody::Extrusion {
        segments,
        distance_mm,
        ..
    } = &part.body
        && segments.iter().all(|segment| segment.is_line())
    {
        let (mut area, mut cx, mut cy) = (0.0, 0.0, 0.0);
        for segment in segments {
            let ([x0, y0], [x1, y1]) = (segment.start_mm, segment.end_mm);
            let cross = x0 * y1 - x1 * y0;
            area += cross;
            cx += (x0 + x1) * cross;
            cy += (y0 + y1) * cross;
        }
        if area.abs() > f64::EPSILON {
            return part.to_world([cx / (3.0 * area), cy / (3.0 * area), distance_mm / 2.0]);
        }
    }
    let (min, max) = part.local_bounds();
    part.to_world(std::array::from_fn(|axis| (min[axis] + max[axis]) / 2.0))
}

/// Positive when o → a → b turns counter-clockwise.
fn turn(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    cross2([a[0] - o[0], a[1] - o[1]], [b[0] - o[0], b[1] - o[1]])
}

/// Convex hull, counter-clockwise, of points in plan.
fn hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup_by(|a, b| (a[0] - b[0]).abs() <= ROUNDING && (a[1] - b[1]).abs() <= ROUNDING);
    if points.len() < 3 {
        return points;
    }
    let mut lower: Vec<[f64; 2]> = Vec::new();
    for &point in &points {
        while lower.len() >= 2 && turn(lower[lower.len() - 2], lower[lower.len() - 1], point) <= 0.0
        {
            lower.pop();
        }
        lower.push(point);
    }
    let mut upper: Vec<[f64; 2]> = Vec::new();
    for &point in points.iter().rev() {
        while upper.len() >= 2 && turn(upper[upper.len() - 2], upper[upper.len() - 1], point) <= 0.0
        {
            upper.pop();
        }
        upper.push(point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

/// Whether `p` lies inside the hull of `points` or within the tolerance of it.
fn over(p: [f64; 2], points: &[[f64; 3]]) -> bool {
    let hull = hull(points.iter().map(|point| [point[0], point[1]]).collect());
    match hull.len() {
        0 => false,
        1 => segment_distance(p, hull[0], hull[0]) <= TOLERANCE_MM,
        2 => segment_distance(p, hull[0], hull[1]) <= TOLERANCE_MM,
        count => {
            (0..count).all(|i| turn(hull[i], hull[(i + 1) % count], p) >= 0.0)
                || (0..count)
                    .any(|i| segment_distance(p, hull[i], hull[(i + 1) % count]) <= TOLERANCE_MM)
        }
    }
}

fn corners((min, max): ([f64; 3], [f64; 3])) -> Vec<[f64; 3]> {
    (0..4)
        .map(|corner: usize| {
            [
                if corner & 1 == 0 { min[0] } else { max[0] },
                if corner & 2 == 0 { min[1] } else { max[1] },
                min[2],
            ]
        })
        .collect()
}

pub(crate) fn on_floor(part: &Part, floor: f64) -> bool {
    let (min, max) = part.world_bounds();
    if part.booleans().next().is_none() {
        (min[2] - floor).abs() <= TOLERANCE_MM
    } else {
        min[2] <= floor + TOLERANCE_MM && max[2] > floor + TOLERANCE_MM
    }
}

fn round(value: f64) -> f64 {
    (value * 10.0).round() / 10.0 + 0.0
}

pub(crate) fn issues<'a>(
    model: &'a ProgramModel,
    issues: &mut Vec<Issue>,
    faces: &mut ContactFaces<'a>,
) {
    if model.load_paths.is_empty() {
        return;
    }
    let parts = &model.parts;
    let floor = model.floor_z_mm.unwrap_or(0.0);
    let grounded = model.grounded_parts();
    let index = |name: &str| parts.iter().position(|part| part.name == name);
    let mut bearings: Vec<Vec<Bearing>> = parts.iter().map(|_| Vec::new()).collect();
    // Side contacts, kept only to name them when a member is not carried.
    let mut beside: Vec<BTreeSet<usize>> = parts.iter().map(|_| BTreeSet::new()).collect();

    let bounds: Vec<_> = parts.iter().map(Part::world_bounds).collect();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|left, right| bounds[*left].0[0].total_cmp(&bounds[*right].0[0]));
    for (position, &left) in order.iter().enumerate() {
        for &right in &order[position + 1..] {
            if bounds[right].0[0] > bounds[left].1[0] + TOLERANCE_MM {
                break;
            }
            let (a, b) = (&parts[left], &parts[right]);
            if model.are_alternatives(a, b)
                || (0..3).any(|axis| {
                    bounds[left].0[axis] > bounds[right].1[axis] + TOLERANCE_MM
                        || bounds[right].0[axis] > bounds[left].1[axis] + TOLERANCE_MM
                })
            {
                continue;
            }
            for contact in faces.contacts(a, b) {
                if contact.normal[2] <= RESTS_ON {
                    bearings[left].push(Bearing {
                        supporter: right,
                        points: contact.points_mm,
                    });
                } else if contact.normal[2] >= -RESTS_ON {
                    bearings[right].push(Bearing {
                        supporter: left,
                        points: contact.points_mm,
                    });
                } else {
                    beside[left].insert(right);
                    beside[right].insert(left);
                }
            }
        }
    }
    for joint in model.joints.iter().filter(|joint| joint.bearing) {
        let (Some(left), Some(right)) = (index(&joint.parts[0]), index(&joint.parts[1])) else {
            continue;
        };
        let points = if joint.fasteners_mm.is_empty() {
            match faces.contact(&parts[left], &parts[right]) {
                Some(contact) => contact.points_mm,
                None => {
                    let (a, b) = (bounds[left], bounds[right]);
                    corners((
                        std::array::from_fn(|axis| a.0[axis].max(b.0[axis])),
                        std::array::from_fn(|axis| a.1[axis].min(b.1[axis])),
                    ))
                }
            }
        } else {
            joint.fasteners_mm.clone()
        };
        bearings[left].push(Bearing {
            supporter: right,
            points: points.clone(),
        });
        bearings[right].push(Bearing {
            supporter: left,
            points,
        });
    }

    let centres: Vec<_> = parts.iter().map(centre_of_mass).collect();
    let mut carried: Vec<bool> = parts
        .iter()
        .map(|part| grounded.contains(&part.name) || on_floor(part, floor))
        .collect();
    // Windows, doors and finishes touching a member do not hold it up.
    let may_carry: Vec<bool> = parts
        .iter()
        .zip(&carried)
        .map(|(part, carried)| {
            *carried
                || model
                    .load_paths
                    .iter()
                    .any(|path| path.member(part) || !part.tags.is_disjoint(&path.carrier_tags))
        })
        .collect();
    for list in &mut bearings {
        list.retain(|bearing| may_carry[bearing.supporter]);
    }
    for (item, set) in beside.iter_mut().enumerate() {
        if !may_carry[item] {
            set.clear();
        }
        // A part it also rests on is not only beside it.
        set.retain(|other| {
            may_carry[*other]
                && !bearings[item]
                    .iter()
                    .any(|bearing| bearing.supporter == *other)
        });
    }
    let mut changed = true;
    while changed {
        changed = false;
        for item in 0..parts.len() {
            if carried[item] || !may_carry[item] {
                continue;
            }
            let points: Vec<[f64; 3]> = bearings[item]
                .iter()
                .filter(|bearing| carried[bearing.supporter])
                .flat_map(|bearing| bearing.points.iter().copied())
                .collect();
            if over([centres[item][0], centres[item][1]], &points) {
                carried[item] = true;
                changed = true;
            }
        }
    }

    let mut reported = BTreeSet::new();
    for path in &model.load_paths {
        for (item, part) in parts.iter().enumerate() {
            if carried[item] || !path.member(part) || !reported.insert(item) {
                continue;
            }
            let names = |set: &mut dyn Iterator<Item = usize>| {
                let mut names: Vec<_> = set.map(|other| parts[other].name.as_str()).collect();
                names.sort_unstable();
                names.dedup();
                names.join(", ")
            };
            let carrying = names(
                &mut bearings[item]
                    .iter()
                    .filter(|bearing| carried[bearing.supporter])
                    .map(|bearing| bearing.supporter),
            );
            let uncarried = names(
                &mut bearings[item]
                    .iter()
                    .filter(|bearing| !carried[bearing.supporter])
                    .map(|bearing| bearing.supporter),
            );
            let sideways = names(&mut beside[item].iter().copied());
            let centre = centres[item];
            let mut message = format!(
                "{} ({}) is not carried: its centre of mass at ({}, {}) mm",
                part.name,
                path.name,
                round(centre[0]),
                round(centre[1])
            );
            message.push_str(&if carrying.is_empty() {
                " rests on nothing that carries it".to_owned()
            } else {
                format!(" is not over its supports on {carrying}")
            });
            if !uncarried.is_empty() {
                message.push_str(&format!(
                    "; what it rests on or hangs from ({uncarried}) is not carried itself"
                ));
            }
            if !sideways.is_empty() {
                message.push_str(&format!(
                    "; it only touches {sideways} from the side, which carries nothing without a bearing joint"
                ));
            }
            let (min, max) = bounds[item];
            issues.push(Issue {
                severity: Severity::Error,
                kind: "member_not_carried",
                parts: vec![part.name.clone()],
                message,
                where_mm: Some((min.map(round), max.map(round))),
                hint: path.hint.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_over_a_hull_or_its_edge_is_over_it() {
        let square = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
        ];
        assert!(over([5.0, 5.0], &square));
        assert!(over([10.0, 5.0], &square));
        assert!(!over([10.5, 5.0], &square));
        let line = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]];
        assert!(over([5.0, 0.0], &line));
        assert!(!over([5.0, 1.0], &line));
        assert!(!over([0.0, 0.0], &[]));
    }
}
