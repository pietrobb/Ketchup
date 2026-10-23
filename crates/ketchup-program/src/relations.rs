//! A compact map of how parts sit against each other: which faces touch and
//! over what area, how deep parts reach into each other and why that is
//! allowed, and the clearance between nearby parts. It lets a caller check a
//! model against its intent without a picture.

use crate::contact::{ContactFaces, contact};
use crate::exact::{ExactPair, ExactShapes};
use crate::frame::{self, Obb};
use crate::model::{Part, ProgramBooleanKind, ProgramModel};
use crate::validate::{self, COLLISION_UNVERIFIED, Issue};
use ketchup_geometry::linalg::dot;
use ketchup_model::tolerance::CONTACT_FIT_MM;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Parts closer than this are listed with their clearance.
pub const GAP_LIMIT_MM: f64 = 20.0;

/// How far beyond [`GAP_LIMIT_MM`] the world bounds of two parts must lie
/// apart before their boxes go unmeasured; every pair nearer than that is
/// measured on its boxes, so rounding never drops a pair at the limit.
const BOUNDS_MARGIN_MM: f64 = 1.0;

type Bounds = ([f64; 3], [f64; 3]);

/// Whether the world bounds of two parts lie further apart than the gap
/// limit and its margin. Each box lies inside its world bounds, so the boxes
/// then lie further apart than the gap limit too.
fn bounds_beyond_gap_limit(a: &Bounds, b: &Bounds) -> bool {
    let distance = (0..3)
        .map(|i| (a.0[i] - b.1[i]).max(b.0[i] - a.1[i]).max(0.0).powi(2))
        .sum::<f64>()
        .sqrt();
    distance > GAP_LIMIT_MM + BOUNDS_MARGIN_MM
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// Faces touch over a patch of positive area.
    Contact,
    /// The parts touch only along an edge or at a corner.
    Touch,
    /// The parts reach into each other.
    Overlap,
    /// A clearance of at most [`GAP_LIMIT_MM`], or any clearance of a joint.
    Gap,
}

/// Why an overlap exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapStatus {
    /// One part had the other subtracted: a socket, mortise or through hole.
    Subtracted,
    /// A subtract/intersect tool removed the shared volume.
    Removed,
    /// The shared volume lies in a pocket or groove.
    Pocket,
    /// The shared volume lies inside a declared joint volume.
    Joint,
    /// A threaded fastener engages a hole tapped with its thread.
    Thread,
    /// The solids overlap: a collision error.
    Collision,
    /// Only the exact solids can decide.
    Unverified,
    /// The boxes overlap but the exact solids do not.
    BoxesOnly,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Relation {
    pub parts: [String; 2],
    pub kind: RelationKind,
    /// Contact: the touching face of each part, in that part's own frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub faces: Option<[String; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_mm2: Option<f64>,
    /// Overlap: the shortest move along `direction` that separates them; for
    /// a socket (`cut_in`), how far the other part reaches into it along its
    /// own axis, which `direction` then gives.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<OverlapStatus>,
    /// Subtracted: the part that holds the socket.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cut_in: Option<String>,
    /// Gap: the clearance; absent when the exact solids are apart by an
    /// unmeasured amount although their boxes touch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_mm: Option<f64>,
    /// World unit vector from the first part towards the second.
    pub direction: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joint: Option<String>,
    /// Measured on boxes although a part is not exactly its box (profile
    /// body, trim or intersect); the exact solid may differ.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub approx: bool,
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0 + 0.0
}

fn round_direction(direction: [f64; 3]) -> [f64; 3] {
    direction.map(|value| (value * 1000.0).round() / 1000.0 + 0.0)
}

pub(crate) fn polygon_area(points: &[[f64; 3]]) -> f64 {
    let Some(first) = points.first() else {
        return 0.0;
    };
    let mut sum = [0.0; 3];
    for pair in points[1..].windows(2) {
        let a: [f64; 3] = std::array::from_fn(|i| pair[0][i] - first[i]);
        let b: [f64; 3] = std::array::from_fn(|i| pair[1][i] - first[i]);
        sum = ketchup_geometry::linalg::add(sum, ketchup_geometry::linalg::cross(a, b));
    }
    (sum.iter().map(|value| value * value).sum::<f64>()).sqrt() / 2.0
}

fn approximate(part: &Part) -> bool {
    !validate::is_box(part)
        || part
            .booleans()
            .any(|boolean| boolean.kind == ProgramBooleanKind::Intersect)
}

/// Corners of the volume `a` and `b` share, measured on their boxes, cut
/// down by the intersect tools of either part and by each subtract tool
/// that acts on that volume like a half-space (a trim): it covers the volume
/// except beyond one of its faces. Other subtract tools are ignored.
fn shared_volume(a: &Part, b: &Part) -> Vec<[f64; 3]> {
    let booleans = || a.finished_booleans().chain(b.finished_booleans());
    let mut planes: Vec<([f64; 3], f64)> = [a.obb(), b.obb()]
        .iter()
        .flat_map(Obb::planes)
        .chain(
            booleans()
                .filter(|(boolean, _)| boolean.kind == ProgramBooleanKind::Intersect)
                .flat_map(|(_, finished)| finished.planes()),
        )
        .collect();
    let mut vertices = frame::polytope_vertices(&planes, CONTACT_FIT_MM);
    for (boolean, finished) in booleans() {
        let tool = &boolean.tool;
        if boolean.kind != ProgramBooleanKind::Subtract
            || tool.name == a.name
            || tool.name == b.name
            || !validate::is_box(tool)
            || tool.booleans().next().is_some()
            || vertices.is_empty()
        {
            continue;
        }
        let tool_planes = finished.planes();
        let outside = |plane: &([f64; 3], f64)| {
            vertices
                .iter()
                .any(|point| dot(plane.0, *point) > plane.1 + CONTACT_FIT_MM)
        };
        let mut sticking_out = tool_planes.iter().filter(|plane| outside(plane));
        if let (Some(&(normal, offset)), None) = (sticking_out.next(), sticking_out.next()) {
            planes.push((normal.map(|value| -value), -offset));
            vertices = frame::polytope_vertices(&planes, CONTACT_FIT_MM);
        }
    }
    vertices
}

/// How far `inserted` reaches into the shared volume: the extent of that
/// volume along the local axis of `inserted` it covers the smallest share
/// of, and that axis in world.
fn insertion(inserted: &Part, vertices: &[[f64; 3]]) -> (f64, [f64; 3]) {
    let obb = inserted.obb();
    (0..3)
        .map(|axis| {
            let direction = obb.axes[axis];
            let (low, high) = vertices
                .iter()
                .map(|point| dot(*point, direction))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), along| {
                    (low.min(along), high.max(along))
                });
            let extent = high - low;
            (
                extent / (2.0 * obb.half[axis]).max(CONTACT_FIT_MM),
                extent,
                direction,
            )
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map_or((0.0, [0.0, 0.0, 1.0]), |(_, extent, direction)| {
            (extent, direction)
        })
}

fn overlap_status(model: &ProgramModel, a: &Part, b: &Part) -> (OverlapStatus, Option<String>) {
    if a.subtracts(b) {
        return (OverlapStatus::Subtracted, Some(a.name.clone()));
    }
    if b.subtracts(a) {
        return (OverlapStatus::Subtracted, Some(b.name.clone()));
    }
    if !validate::booleans_leave_overlap(a, b) || !validate::booleans_leave_overlap(b, a) {
        return (OverlapStatus::Removed, None);
    }
    let Some(region) = validate::overlap(a, b) else {
        return (OverlapStatus::BoxesOnly, None);
    };
    if validate::seated_in_pocket(a, b) {
        return (OverlapStatus::Pocket, None);
    }
    let declared = model.joints.iter().any(|joint| {
        joint.parts.contains(&a.name)
            && joint.parts.contains(&b.name)
            && joint
                .volume_mm
                .is_some_and(|volume| validate::contains(volume, region))
    });
    if declared {
        return (OverlapStatus::Joint, None);
    }
    if validate::threaded_into(a, b, region) {
        return (OverlapStatus::Thread, None);
    }
    // Collision or unverified; `sync_with_issues` settles which.
    (OverlapStatus::Collision, None)
}

fn relate<'a>(
    model: &ProgramModel,
    exact: &ExactShapes,
    a: &'a Part,
    b: &'a Part,
    joint: Option<&str>,
    bounds: Option<[&Bounds; 2]>,
    faces: &mut ContactFaces<'a>,
) -> Option<Relation> {
    let (oa, ob) = (a.obb(), b.obb());
    let (direction, separation) = oa.separating_axis(&ob);
    // Apart boxes without a joint further than the gap limit are dropped
    // below before anything reads the pair's exact answer; bounds that far
    // apart settle that without measuring the boxes.
    if separation > CONTACT_FIT_MM
        && joint.is_none()
        && bounds.is_some_and(|[ba, bb]| bounds_beyond_gap_limit(ba, bb))
    {
        return None;
    }
    let mut relation = Relation {
        parts: [a.name.clone(), b.name.clone()],
        kind: RelationKind::Gap,
        faces: None,
        area_mm2: None,
        depth_mm: None,
        status: None,
        cut_in: None,
        gap_mm: None,
        direction: round_direction(direction),
        joint: joint.map(str::to_owned),
        approx: approximate(a) || approximate(b),
    };
    if separation > CONTACT_FIT_MM {
        let gap = oa.distance(&ob);
        if gap > GAP_LIMIT_MM && joint.is_none() {
            return None;
        }
        relation.gap_mm = Some(round1(gap));
    } else if separation >= -CONTACT_FIT_MM {
        match faces.contact(a, b) {
            Some(patch) => {
                relation.kind = RelationKind::Contact;
                relation.faces = Some([patch.face_a.clone(), patch.face_b.clone()]);
                relation.area_mm2 = Some(polygon_area(&patch.points_mm).round());
                relation.direction = round_direction(patch.normal);
            }
            None => relation.kind = RelationKind::Touch,
        }
    } else {
        let (status, cut_in) = overlap_status(model, a, b);
        let vertices = shared_volume(a, b);
        relation.kind = RelationKind::Overlap;
        relation.status = Some(status);
        if vertices.is_empty() {
            relation.status = Some(OverlapStatus::Removed);
        } else if let Some(cut_in) = &cut_in {
            // A socket: report how far the other part reaches into it.
            let inserted = if *cut_in == a.name { b } else { a };
            let (depth, axis) = insertion(inserted, &vertices);
            let towards: [f64; 3] = std::array::from_fn(|i| ob.centre[i] - oa.centre[i]);
            let sign = if dot(axis, towards) < 0.0 { -1.0 } else { 1.0 };
            relation.depth_mm = Some(round1(depth));
            relation.direction = round_direction(axis.map(|value| value * sign));
        } else {
            relation.depth_mm = Some(round1(-separation));
        }
        relation.cut_in = cut_in;
    }
    match exact.decides(a, b) {
        Some(pair)
            if !pair.penetrating()
                && (!intended_overlap(&relation)
                    || pair.gap_mm().is_some_and(|gap| gap > CONTACT_FIT_MM)) =>
        {
            Some(exact_relation(a, b, relation, pair))
        }
        _ => Some(relation),
    }
}

/// An overlap the program meant: a socket, a removed volume, a pocket or a
/// joint volume.
fn intended_overlap(relation: &Relation) -> bool {
    matches!(
        relation.status,
        Some(
            OverlapStatus::Subtracted
                | OverlapStatus::Removed
                | OverlapStatus::Pocket
                | OverlapStatus::Joint
        )
    )
}

/// The relation of two solids that do not overlap, from the exact answer.
fn exact_relation(a: &Part, b: &Part, mut relation: Relation, pair: ExactPair) -> Relation {
    relation.approx = false;
    relation.faces = None;
    relation.area_mm2 = None;
    relation.depth_mm = None;
    relation.status = None;
    relation.cut_in = None;
    relation.gap_mm = None;
    if pair.contact_area_mm2 > CONTACT_FIT_MM * CONTACT_FIT_MM {
        relation.kind = RelationKind::Contact;
        relation.area_mm2 = Some(pair.contact_area_mm2.round());
        if let Some(patch) = contact(a, b) {
            relation.faces = Some([patch.face_a.clone(), patch.face_b.clone()]);
            relation.direction = round_direction(patch.normal);
        }
    } else if pair.touching() {
        relation.kind = RelationKind::Touch;
    } else {
        relation.kind = RelationKind::Gap;
        relation.gap_mm = pair.distance_mm.map(round1);
    }
    relation
}

/// Every pair of parts that touch, overlap or lie within [`GAP_LIMIT_MM`],
/// plus every jointed pair; in model order, the first part first.
#[must_use]
pub fn relations(model: &ProgramModel, issues: &[Issue]) -> Vec<Relation> {
    relations_with(model, issues, &ExactShapes::default())
}

/// [`relations`], taking the exact answer wherever the boxes of a pair
/// misstate its solids.
#[must_use]
pub fn relations_with(
    model: &ProgramModel,
    issues: &[Issue],
    exact: &ExactShapes,
) -> Vec<Relation> {
    relations_by_bounds(model, issues, exact, true)
}

/// [`relations_with`]; without `skip_by_bounds` every candidate pair apart
/// is measured on its boxes, as a reference for tests.
fn relations_by_bounds(
    model: &ProgramModel,
    issues: &[Issue],
    exact: &ExactShapes,
    skip_by_bounds: bool,
) -> Vec<Relation> {
    let parts = &model.parts;
    let bounds: Vec<_> = parts.iter().map(Part::world_bounds).collect();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|left, right| bounds[*left].0[0].total_cmp(&bounds[*right].0[0]));
    let joint_of = |a: &Part, b: &Part| {
        model
            .joints
            .iter()
            .find(|joint| joint.parts.contains(&a.name) && joint.parts.contains(&b.name))
            .map(|joint| joint.name.as_str())
    };
    let mut pairs = Vec::new();
    for (position, &left) in order.iter().enumerate() {
        for &right in &order[position + 1..] {
            if bounds[right].0[0] > bounds[left].1[0] + GAP_LIMIT_MM {
                break;
            }
            pairs.push((left.min(right), left.max(right)));
        }
    }
    for joint in &model.joints {
        let index = |name: &str| parts.iter().position(|part| part.name == name);
        if let (Some(a), Some(b)) = (index(&joint.parts[0]), index(&joint.parts[1])) {
            pairs.push((a.min(b), a.max(b)));
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    let mut faces = ContactFaces::default();
    let mut relations: Vec<Relation> = pairs
        .into_iter()
        .filter(|(a, b)| a != b)
        .filter_map(|(a, b)| {
            let pair_bounds = skip_by_bounds.then(|| [&bounds[a], &bounds[b]]);
            let (a, b) = (&parts[a], &parts[b]);
            relate(model, exact, a, b, joint_of(a, b), pair_bounds, &mut faces)
        })
        .collect();
    sync_with_issues(&mut relations, issues);
    relations
}

/// Settles collision/unverified/boxes-only overlaps from the current issues,
/// e.g. after the exact solids decided an unverified pair.
pub fn sync_with_issues(relations: &mut [Relation], issues: &[Issue]) {
    fn pair<'a>(kind: &'a str, a: &'a str, b: &'a str) -> (&'a str, &'a str, &'a str) {
        (kind, a.min(b), a.max(b))
    }
    let flagged: HashSet<(&str, &str, &str)> = issues
        .iter()
        .filter(|issue| issue.parts.len() == 2)
        .map(|issue| pair(issue.kind, &issue.parts[0], &issue.parts[1]))
        .collect();
    let issue_for = |parts: &[String; 2], kind| flagged.contains(&pair(kind, &parts[0], &parts[1]));
    for relation in relations {
        if !matches!(
            relation.status,
            Some(OverlapStatus::Collision | OverlapStatus::Unverified | OverlapStatus::BoxesOnly)
        ) {
            continue;
        }
        relation.status = Some(if issue_for(&relation.parts, "collision") {
            OverlapStatus::Collision
        } else if issue_for(&relation.parts, COLLISION_UNVERIFIED) {
            OverlapStatus::Unverified
        } else {
            OverlapStatus::BoxesOnly
        });
    }
}

#[cfg(test)]
#[path = "relations_tests.rs"]
mod tests;
