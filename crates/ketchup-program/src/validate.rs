//! Generic checks over an evaluated model. They report issues; they never
//! reject the model. Production export decides whether issues block.

use crate::eval::{TOLERANCE_MM, contact};
use crate::model::{Part, ProgramModel};
use serde::Serialize;

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

fn overlap(a: &Part, b: &Part) -> Option<([f64; 3], [f64; 3])> {
    let (a_min, a_max, b_min, b_max) = (a.at_mm, a.max_mm(), b.at_mm, b.max_mm());
    let min: [f64; 3] = std::array::from_fn(|axis| a_min[axis].max(b_min[axis]));
    let max: [f64; 3] = std::array::from_fn(|axis| a_max[axis].min(b_max[axis]));
    (0..3)
        .all(|axis| max[axis] - min[axis] > TOLERANCE_MM)
        .then_some((min, max))
}

/// Smallest distance between two boxes (0 when they touch or overlap).
fn gap(a: &Part, b: &Part) -> f64 {
    let (a_min, a_max, b_min, b_max) = (a.at_mm, a.max_mm(), b.at_mm, b.max_mm());
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

fn contains(outer: ([f64; 3], [f64; 3]), inner: ([f64; 3], [f64; 3])) -> bool {
    (0..3).all(|axis| {
        inner.0[axis] >= outer.0[axis] - TOLERANCE_MM
            && inner.1[axis] <= outer.1[axis] + TOLERANCE_MM
    })
}

fn world_pockets(part: &Part) -> impl Iterator<Item = ([f64; 3], [f64; 3])> + '_ {
    part.pockets.iter().map(|pocket| {
        let (min, max) = pocket.local_box(part.size_mm);
        (part.to_world(min), part.to_world(max))
    })
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn collisions(model: &ProgramModel, issues: &mut Vec<Issue>) {
    let mut order: Vec<usize> = (0..model.parts.len()).collect();
    order.sort_by(|left, right| {
        model.parts[*left].at_mm[0].total_cmp(&model.parts[*right].at_mm[0])
    });
    for (position, &left) in order.iter().enumerate() {
        let a = &model.parts[left];
        for &right in &order[position + 1..] {
            let b = &model.parts[right];
            if b.at_mm[0] >= a.max_mm()[0] - TOLERANCE_MM {
                break;
            }
            let Some(region) = overlap(a, b) else {
                continue;
            };
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

fn joints(model: &ProgramModel, issues: &mut Vec<Issue>) {
    for joint in &model.joints {
        let (Some(a), Some(b)) = (model.part(&joint.parts[0]), model.part(&joint.parts[1])) else {
            continue;
        };
        if contact(a, b).is_none()
            && overlap(a, b).is_none()
            && gap(a, b) > joint.max_gap_mm + TOLERANCE_MM
        {
            issues.push(Issue {
                severity: Severity::Error,
                kind: "joint_without_contact",
                parts: joint.parts.to_vec(),
                message: format!(
                    "joint {} connects {} and {}, but they are {} mm apart (allowed {} mm)",
                    joint.name,
                    a.name,
                    b.name,
                    round(gap(a, b)),
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
fn support(model: &ProgramModel, issues: &mut Vec<Issue>) {
    let count = model.parts.len();
    let mut supported: Vec<bool> = model
        .parts
        .iter()
        .map(|part| part.at_mm[2].abs() <= TOLERANCE_MM)
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
                contact(part, other).is_some()
                    || overlap(part, other).is_some()
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
                where_mm: Some((part.at_mm.map(round), part.max_mm().map(round))),
                hint: "Move it onto a supporting part or connect it with a joint.".to_owned(),
            });
        }
    }
}

/// All generic checks. Errors first, then warnings; stable order.
#[must_use]
pub fn validate(model: &ProgramModel) -> Vec<Issue> {
    let mut issues = Vec::new();
    params(model, &mut issues);
    collisions(model, &mut issues);
    holes(model, &mut issues);
    joints(model, &mut issues);
    support(model, &mut issues);
    issues.sort_by_key(|issue| issue.severity);
    issues
}
