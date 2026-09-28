//! Generic checks over an evaluated model. They report issues; they never
//! reject the model. Production export decides whether issues block.

use crate::eval::{TOLERANCE_MM, contact};
use crate::exact::ExactShapes;
use crate::frame::{self, Obb};
use crate::model::{Part, ProgramBooleanKind, ProgramModel, ProgramPartBody};
use serde::Serialize;

/// Issue kind for a box overlap that only the exact solids can decide.
pub const COLLISION_UNVERIFIED: &str = "collision_unverified";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub kind: &'static str,
    pub parts: Vec<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub where_mm: Option<([f64; 3], [f64; 3])>,
    pub hint: String,
}

/// Bounds of `b` in the frame of `a` when both share one orientation.
fn in_frame_of(a: &Part, b: &Part) -> Option<([f64; 3], [f64; 3])> {
    if !frame::same_orientation(&a.rotation, &b.rotation) {
        return None;
    }
    let origin = a.to_local(b.at_mm);
    let (min, max) = b.local_bounds();
    Some((
        std::array::from_fn(|axis| origin[axis] + min[axis]),
        std::array::from_fn(|axis| origin[axis] + max[axis]),
    ))
}

/// World bounds of the region where the two parts' boxes overlap.
pub(crate) fn overlap(a: &Part, b: &Part) -> Option<([f64; 3], [f64; 3])> {
    let intersect = |(a_min, a_max): ([f64; 3], [f64; 3]), (b_min, b_max): ([f64; 3], [f64; 3])| {
        let min: [f64; 3] = std::array::from_fn(|axis| f64::max(a_min[axis], b_min[axis]));
        let max: [f64; 3] = std::array::from_fn(|axis| f64::min(a_max[axis], b_max[axis]));
        (0..3)
            .all(|axis| max[axis] - min[axis] > TOLERANCE_MM)
            .then_some((min, max))
    };
    if let Some(b_local) = in_frame_of(a, b) {
        let (min, max) = intersect(a.local_bounds(), b_local)?;
        return Some(Obb::new(a.at_mm, &a.rotation, min, max).world_bounds());
    }
    if a.obb().separation(&b.obb()) >= -TOLERANCE_MM {
        return None;
    }
    frame::common_region(&[a.obb(), b.obb()], TOLERANCE_MM)
}

/// Smallest distance between two boxes (0 when they touch or overlap).
fn gap(a: &Part, b: &Part) -> f64 {
    let Some((b_min, b_max)) = in_frame_of(a, b) else {
        return a.obb().separation(&b.obb()).max(0.0);
    };
    let (a_min, a_max) = a.local_bounds();
    (0..3)
        .map(|axis| {
            (b_min[axis] - a_max[axis])
                .max(a_min[axis] - b_max[axis])
                .max(0.0)
        })
        .map(|distance| distance * distance)
        .sum::<f64>()
        .sqrt()
}

pub(crate) fn contains(outer: ([f64; 3], [f64; 3]), inner: ([f64; 3], [f64; 3])) -> bool {
    (0..3).all(|axis| {
        inner.0[axis] >= outer.0[axis] - TOLERANCE_MM
            && inner.1[axis] <= outer.1[axis] + TOLERANCE_MM
    })
}

pub(crate) fn world_pockets(part: &Part) -> impl Iterator<Item = ([f64; 3], [f64; 3])> + '_ {
    part.pockets.iter().map(|pocket| {
        let (min, max) = pocket.local_box(part.size_mm);
        Obb::new(part.at_mm, &part.rotation, min, max).world_bounds()
    })
}

/// False when `a`'s booleans certainly removed the volume it shares with `b`:
/// `a` had `b` itself subtracted, or one subtracted plain box tool covers the
/// whole shared volume, or an intersect tool leaves none of it.
pub(crate) fn booleans_leave_overlap(a: &Part, b: &Part) -> bool {
    if a.subtracts(b) {
        return false;
    }
    let shared = [a.obb(), b.obb()];
    let corners = frame::intersection_vertices(&shared, TOLERANCE_MM);
    a.booleans.iter().all(|boolean| {
        let tool = &boolean.tool;
        match boolean.kind {
            // Only a plain box is exactly its bounding box; anything else
            // removes less, so it cannot prove the overlap gone.
            ProgramBooleanKind::Subtract => {
                !(matches!(tool.body, ProgramPartBody::Panel)
                    && tool.booleans.is_empty()
                    && !corners.is_empty()
                    && corners
                        .iter()
                        .all(|corner| tool.obb().contains(*corner, TOLERANCE_MM)))
            }
            // The tool's box holds the whole tool, so no common volume with
            // the box means none with the tool.
            ProgramBooleanKind::Intersect => {
                frame::common_region(&[shared[0], shared[1], tool.obb()], TOLERANCE_MM).is_some()
            }
        }
    })
}

/// Whether the solid of `part`, before booleans, is exactly its box: a panel,
/// or an extrusion of a rectangle, with no cut, finish or moved face.
pub(crate) fn is_box(part: &Part) -> bool {
    let plain = part.cuts.is_empty() && part.fillets.is_empty() && part.face_offsets.is_empty();
    plain
        && match &part.body {
            ProgramPartBody::Panel => true,
            ProgramPartBody::Extrusion { segments, .. }
                if segments.iter().all(|segment| segment.arc.is_none()) =>
            {
                let (min, max) = part.local_bounds();
                let (width, depth) = (max[0] - min[0], max[1] - min[1]);
                let doubled_area: f64 = segments
                    .iter()
                    .map(|s| s.start_mm[0] * s.end_mm[1] - s.end_mm[0] * s.start_mm[1])
                    .sum();
                (doubled_area.abs() / 2.0 - width * depth).abs() <= TOLERANCE_MM * (width + depth)
            }
            ProgramPartBody::Extrusion { .. }
            | ProgramPartBody::Revolve { .. }
            | ProgramPartBody::Sweep { .. }
            | ProgramPartBody::Loft { .. } => false,
        }
}

/// Whether boxes alone cannot decide if the solids of `a` and `b` overlap.
/// Two boxes overlap by a convex volume; one box tool subtracted from it or
/// intersected with it leaves a decidable convex question, which
/// `booleans_leave_overlap` already answered. Profile bodies, and two or more
/// tools reaching into the shared volume, need the exact solids.
fn needs_exact_shapes(a: &Part, b: &Part) -> bool {
    if !is_box(a) || !is_box(b) {
        return true;
    }
    let shared = [a.obb(), b.obb()];
    let mut reaching = a.booleans.iter().chain(&b.booleans).filter(|boolean| {
        frame::common_region(&[shared[0], shared[1], boolean.tool.obb()], TOLERANCE_MM).is_some()
    });
    match (reaching.next(), reaching.next()) {
        (None, _) => false,
        (Some(boolean), None) => !is_box(&boolean.tool) || !boolean.tool.booleans.is_empty(),
        (Some(_), Some(_)) => true,
    }
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn collisions(model: &ProgramModel, exact: &ExactShapes, issues: &mut Vec<Issue>) {
    let bounds: Vec<_> = model.parts.iter().map(Part::world_bounds).collect();
    let mut order: Vec<usize> = (0..model.parts.len()).collect();
    order.sort_by(|left, right| bounds[*left].0[0].total_cmp(&bounds[*right].0[0]));
    for (position, &left) in order.iter().enumerate() {
        let a = &model.parts[left];
        for &right in &order[position + 1..] {
            let b = &model.parts[right];
            if bounds[right].0[0] >= bounds[left].1[0] - TOLERANCE_MM {
                break;
            }
            let Some(region) = overlap(a, b) else {
                continue;
            };
            if !booleans_leave_overlap(a, b) || !booleans_leave_overlap(b, a) {
                continue;
            }
            let seated = world_pockets(a)
                .chain(world_pockets(b))
                .any(|pocket| contains(pocket, region));
            let declared = model.joints.iter().any(|joint| {
                joint.parts.contains(&a.name)
                    && joint.parts.contains(&b.name)
                    && joint
                        .volume_mm
                        .is_some_and(|volume| contains(volume, region))
            });
            if seated || declared {
                continue;
            }
            let (min, max) = region;
            if needs_exact_shapes(a, b) {
                match exact.pair(&a.name, &b.name) {
                    Some(pair) if pair.penetrating() => {
                        issues.push(Issue {
                            severity: Severity::Error,
                            kind: "collision",
                            parts: vec![a.name.clone(), b.name.clone()],
                            message: format!(
                                "the solids of {} and {} overlap ({} mm³)",
                                a.name,
                                b.name,
                                (pair.common_volume_mm3 * 10.0).round() / 10.0
                            ),
                            where_mm: Some((min.map(round), max.map(round))),
                            hint: "Move or resize one part so they only touch, or subtract one \
                                   from the other, or declare a joint with a volume."
                                .to_owned(),
                        });
                        continue;
                    }
                    Some(_) => continue,
                    None => {}
                }
                issues.push(Issue {
                    severity: Severity::Warning,
                    kind: COLLISION_UNVERIFIED,
                    parts: vec![a.name.clone(), b.name.clone()],
                    message: format!(
                        "the bounding boxes of {} and {} overlap by {} x {} x {} mm; \
                         whether the solids themselves overlap needs the exact shapes",
                        a.name,
                        b.name,
                        round(max[0] - min[0]),
                        round(max[1] - min[1]),
                        round(max[2] - min[2]),
                    ),
                    where_mm: Some((min.map(round), max.map(round))),
                    hint: "KetchupProgram check/build and the Kečup window settle this on the \
                           exact solids; a box-only check cannot."
                        .to_owned(),
                });
                continue;
            }
            issues.push(Issue {
                severity: Severity::Error,
                kind: "collision",
                parts: vec![a.name.clone(), b.name.clone()],
                message: format!(
                    "{} and {} overlap by {} x {} x {} mm",
                    a.name,
                    b.name,
                    round(max[0] - min[0]),
                    round(max[1] - min[1]),
                    round(max[2] - min[2]),
                ),
                where_mm: Some((min.map(round), max.map(round))),
                hint: "Move or resize one part so they only touch, or cut a pocket/groove \
                       for the other part, or declare a joint with a volume."
                    .to_owned(),
            });
        }
    }
}

fn holes(model: &ProgramModel, issues: &mut Vec<Issue>) {
    for part in &model.parts {
        for hole in &part.holes {
            let (u_axis, v_axis) = hole.face.uv_axes();
            let radius = hole.diameter_mm / 2.0;
            let thickness = part.size_mm[hole.face.axis()];
            let inside = hole.u_mm - radius >= -TOLERANCE_MM
                && hole.u_mm + radius <= part.size_mm[u_axis] + TOLERANCE_MM
                && hole.v_mm - radius >= -TOLERANCE_MM
                && hole.v_mm + radius <= part.size_mm[v_axis] + TOLERANCE_MM;
            let entry = part.to_world(hole.face.local_point(part.size_mm, hole.u_mm, hole.v_mm));
            if !inside {
                issues.push(Issue {
                    severity: Severity::Error,
                    kind: "hole_outside_face",
                    parts: vec![part.name.clone()],
                    message: format!(
                        "hole {} (d{}) at ({}, {}) on face {} of {} does not fit on the {} x {} mm face",
                        hole.id,
                        hole.diameter_mm,
                        round(hole.u_mm),
                        round(hole.v_mm),
                        hole.face.name(),
                        part.name,
                        round(part.size_mm[u_axis]),
                        round(part.size_mm[v_axis]),
                    ),
                    where_mm: Some((entry.map(round), entry.map(round))),
                    hint: "Move the hole inside the face or enlarge the part.".to_owned(),
                });
            }
            if hole.depth_mm >= thickness - TOLERANCE_MM {
                issues.push(Issue {
                    severity: Severity::Error,
                    kind: "hole_breaks_through",
                    parts: vec![part.name.clone()],
                    message: format!(
                        "hole {} is {} mm deep but {} is only {} mm thick there",
                        hole.id,
                        round(hole.depth_mm),
                        part.name,
                        round(thickness),
                    ),
                    where_mm: Some((entry.map(round), entry.map(round))),
                    hint:
                        "Use a shorter dowel/screw or a thicker part, or drill from the other side."
                            .to_owned(),
                });
            }
        }
    }
}

fn joints(model: &ProgramModel, exact: &ExactShapes, issues: &mut Vec<Issue>) {
    for joint in &model.joints {
        let (Some(a), Some(b)) = (model.part(&joint.parts[0]), model.part(&joint.parts[1])) else {
            continue;
        };
        // `None`: apart by an unknown clearance above the contact tolerance.
        let apart = match exact.decides(a, b) {
            Some(pair) => pair.gap_mm().map_or(Some(None), |gap| {
                (gap > joint.max_gap_mm + TOLERANCE_MM).then_some(Some(gap))
            }),
            None => (contact(a, b).is_none()
                && overlap(a, b).is_none()
                && gap(a, b) > joint.max_gap_mm + TOLERANCE_MM)
                .then(|| Some(gap(a, b))),
        };
        if let Some(apart) = apart {
            if apart.is_none() && joint.max_gap_mm > TOLERANCE_MM {
                continue;
            }
            issues.push(Issue {
                severity: Severity::Error,
                kind: "joint_without_contact",
                parts: joint.parts.to_vec(),
                message: format!(
                    "joint {} connects {} and {}, but {} (allowed gap {} mm)",
                    joint.name,
                    a.name,
                    b.name,
                    apart.map_or("their solids do not touch".to_owned(), |gap| format!(
                        "they are {} mm apart",
                        round(gap)
                    )),
                    joint.max_gap_mm
                ),
                where_mm: None,
                hint: "Move the parts face to face or remove the joint.".to_owned(),
            });
        }
    }
}

fn params(model: &ProgramModel, issues: &mut Vec<Issue>) {
    for param in &model.params {
        let low = param.min.is_some_and(|min| param.value < min);
        let high = param.max.is_some_and(|max| param.value > max);
        if low || high {
            issues.push(Issue {
                severity: Severity::Error,
                kind: "param_out_of_range",
                parts: Vec::new(),
                message: format!(
                    "param {} = {} is outside [{}, {}]",
                    param.name,
                    param.value,
                    param
                        .min
                        .map_or("-inf".to_owned(), |value| value.to_string()),
                    param
                        .max
                        .map_or("inf".to_owned(), |value| value.to_string()),
                ),
                where_mm: None,
                hint: "Choose a value inside the declared range.".to_owned(),
            });
        }
    }
}

/// Parts that neither rest on z = 0 nor touch, through other parts, something
/// that does. Contact is face contact or a declared joint.
fn support(model: &ProgramModel, exact: &ExactShapes, issues: &mut Vec<Issue>) {
    let count = model.parts.len();
    let mut supported: Vec<bool> = model
        .parts
        .iter()
        .map(|part| {
            // Booleans (a trim at the floor) can end a part exactly at z = 0
            // although its uncut box reaches below.
            let (min, max) = part.world_bounds();
            if part.booleans.is_empty() {
                min[2].abs() <= TOLERANCE_MM
            } else {
                min[2] <= TOLERANCE_MM && max[2] > TOLERANCE_MM
            }
        })
        .collect();
    if !supported.iter().any(|value| *value) {
        return;
    }
    let mut changed = true;
    while changed {
        changed = false;
        for left in 0..count {
            if supported[left] {
                continue;
            }
            let part = &model.parts[left];
            let joined = |other: &Part| {
                let touching = match exact.decides(part, other) {
                    Some(pair) => pair.penetrating() || pair.touching(),
                    None => {
                        contact(part, other).is_some()
                            || overlap(part, other).is_some()
                            || ((part.is_rotated() || other.is_rotated())
                                && gap(part, other) <= TOLERANCE_MM)
                    }
                };
                touching
                    || model.joints.iter().any(|joint| {
                        joint.parts.contains(&part.name) && joint.parts.contains(&other.name)
                    })
            };
            if (0..count).any(|right| supported[right] && joined(&model.parts[right])) {
                supported[left] = true;
                changed = true;
            }
        }
    }
    for (index, part) in model.parts.iter().enumerate() {
        if !supported[index] {
            issues.push(Issue {
                severity: Severity::Warning,
                kind: "floating_part",
                parts: vec![part.name.clone()],
                message: format!(
                    "{} touches nothing that rests on the floor (z = 0)",
                    part.name
                ),
                where_mm: {
                    let (min, max) = part.world_bounds();
                    Some((min.map(round), max.map(round)))
                },
                hint: "Move it onto a supporting part or connect it with a joint.".to_owned(),
            });
        }
    }
}

/// All generic checks on boxes. Errors first, then warnings; stable order.
#[must_use]
pub fn validate(model: &ProgramModel) -> Vec<Issue> {
    validate_with(model, &ExactShapes::default())
}

/// All generic checks, taking the exact answer wherever the boxes of a pair
/// misstate its solids.
#[must_use]
pub fn validate_with(model: &ProgramModel, exact: &ExactShapes) -> Vec<Issue> {
    let mut issues = Vec::new();
    params(model, &mut issues);
    collisions(model, exact, &mut issues);
    holes(model, &mut issues);
    joints(model, exact, &mut issues);
    support(model, exact, &mut issues);
    crate::expect::check(model, exact, &mut issues);
    issues.sort_by_key(|issue| issue.severity);
    issues
}
