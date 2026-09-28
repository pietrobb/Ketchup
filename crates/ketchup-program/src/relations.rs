//! A compact map of how parts sit against each other: which faces touch and
//! over what area, how deep parts reach into each other and why that is
//! allowed, and the clearance between nearby parts. It lets a caller check a
//! model against its intent without a picture.

use crate::eval::{TOLERANCE_MM, contact};
use crate::frame::{self, Obb};
use crate::model::{Face, Part, ProgramBooleanKind, ProgramModel};
use crate::validate::{self, COLLISION_UNVERIFIED, Issue};
use serde::Serialize;

/// Parts closer than this are listed with their clearance.
pub const GAP_LIMIT_MM: f64 = 20.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
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
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
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
    /// The solids overlap: a collision error.
    Collision,
    /// Only the exact solids can decide.
    Unverified,
    /// The boxes overlap but the exact solids do not.
    BoxesOnly,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Relation {
    pub parts: [String; 2],
    pub kind: RelationKind,
    /// Contact: the touching face of each part, in that part's own frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub faces: Option<[Face; 2]>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_mm: Option<f64>,
    /// World unit vector from the first part towards the second.
    pub direction: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joint: Option<String>,
    /// Measured on boxes although a part is not exactly its box (profile
    /// body, trim or intersect); the exact solid may differ.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
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
        sum[0] += a[1] * b[2] - a[2] * b[1];
        sum[1] += a[2] * b[0] - a[0] * b[2];
        sum[2] += a[0] * b[1] - a[1] * b[0];
    }
    (sum.iter().map(|value| value * value).sum::<f64>()).sqrt() / 2.0
}

fn approximate(part: &Part) -> bool {
    !validate::is_box(part)
        || part
            .booleans
            .iter()
            .any(|boolean| boolean.kind == ProgramBooleanKind::Intersect)
}

/// Corners of the volume `a` and `b` share, measured on their boxes, cut
/// down by the intersect tools of either part and by each subtract tool
/// that acts on that volume like a half-space (a trim): it covers the volume
/// except beyond one of its faces. Other subtract tools are ignored.
fn shared_volume(a: &Part, b: &Part) -> Vec<[f64; 3]> {
    let booleans = || a.booleans.iter().chain(&b.booleans);
    let mut planes: Vec<([f64; 3], f64)> = [a.obb(), b.obb()]
        .iter()
        .flat_map(Obb::planes)
        .chain(
            booleans()
                .filter(|boolean| boolean.kind == ProgramBooleanKind::Intersect)
                .flat_map(|boolean| boolean.tool.obb().planes()),
        )
        .collect();
    let mut vertices = frame::polytope_vertices(&planes, TOLERANCE_MM);
    for boolean in booleans() {
        let tool = &boolean.tool;
        if boolean.kind != ProgramBooleanKind::Subtract
            || tool.name == a.name
            || tool.name == b.name
            || !validate::is_box(tool)
            || !tool.booleans.is_empty()
            || vertices.is_empty()
        {
            continue;
        }
        let tool_planes = tool.obb().planes();
        let outside = |plane: &([f64; 3], f64)| {
            vertices
                .iter()
                .any(|point| frame::dot(plane.0, *point) > plane.1 + TOLERANCE_MM)
        };
        let mut sticking_out = tool_planes.iter().filter(|plane| outside(plane));
        if let (Some(&(normal, offset)), None) = (sticking_out.next(), sticking_out.next()) {
            planes.push((normal.map(|value| -value), -offset));
            vertices = frame::polytope_vertices(&planes, TOLERANCE_MM);
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
                .map(|point| frame::dot(*point, direction))
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), along| {
                    (low.min(along), high.max(along))
                });
            let extent = high - low;
            (
                extent / (2.0 * obb.half[axis]).max(TOLERANCE_MM),
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
    if validate::world_pockets(a)
        .chain(validate::world_pockets(b))
        .any(|pocket| validate::contains(pocket, region))
    {
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
    // Collision or unverified; `sync_with_issues` settles which.
    (OverlapStatus::Collision, None)
}

fn relate(model: &ProgramModel, a: &Part, b: &Part, joint: Option<&str>) -> Option<Relation> {
    let (oa, ob) = (a.obb(), b.obb());
    let (direction, separation) = oa.separating_axis(&ob);
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
    if separation > TOLERANCE_MM {
        let gap = oa.distance(&ob);
        if gap > GAP_LIMIT_MM && joint.is_none() {
            return None;
        }
        relation.gap_mm = Some(round1(gap));
    } else if separation >= -TOLERANCE_MM {
        match contact(a, b) {
            Some(patch) => {
                relation.kind = RelationKind::Contact;
                relation.faces = Some([patch.face_a, patch.face_b]);
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
            let sign = if frame::dot(axis, towards) < 0.0 {
                -1.0
            } else {
                1.0
            };
            relation.depth_mm = Some(round1(depth));
            relation.direction = round_direction(axis.map(|value| value * sign));
        } else {
            relation.depth_mm = Some(round1(-separation));
        }
        relation.cut_in = cut_in;
    }
    Some(relation)
}

/// Every pair of parts that touch, overlap or lie within [`GAP_LIMIT_MM`],
/// plus every jointed pair; in model order, the first part first.
#[must_use]
pub fn relations(model: &ProgramModel, issues: &[Issue]) -> Vec<Relation> {
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
    let mut relations: Vec<Relation> = pairs
        .into_iter()
        .filter(|(a, b)| a != b)
        .filter_map(|(a, b)| {
            let (a, b) = (&parts[a], &parts[b]);
            relate(model, a, b, joint_of(a, b))
        })
        .collect();
    sync_with_issues(&mut relations, issues);
    relations
}

/// Settles collision/unverified/boxes-only overlaps from the current issues,
/// e.g. after the exact solids decided an unverified pair.
pub fn sync_with_issues(relations: &mut [Relation], issues: &[Issue]) {
    let issue_for = |parts: &[String; 2], kind: &str| {
        issues.iter().any(|issue| {
            issue.kind == kind
                && issue.parts.len() == 2
                && issue.parts.contains(&parts[0])
                && issue.parts.contains(&parts[1])
        })
    };
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
