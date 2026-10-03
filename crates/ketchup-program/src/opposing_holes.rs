//! Clearance between parallel cylindrical bores entering from opposite sides.
use crate::eval::TOLERANCE_MM;
use crate::model::Part;
use crate::validate::{Issue, Severity, THIN_WALL_MM};
use ketchup_geometry::linalg::{dot, length};
use ketchup_model::tolerance::ROUNDING;

pub(crate) fn issues(part: &Part, issues: &mut Vec<Issue>) {
    let holes: Vec<_> = part.finished_holes().collect();
    for (index, (a, (a_entry, axis))) in holes.iter().enumerate() {
        for (b, (b_entry, b_axis)) in &holes[index + 1..] {
            if (0..3).any(|i| (axis[i] + b_axis[i]).abs() > ROUNDING) {
                continue;
            }
            let delta = std::array::from_fn(|i| b_entry[i] - a_entry[i]);
            let span = dot(delta, *axis);
            if span <= TOLERANCE_MM {
                continue;
            }
            let radial: [f64; 3] = std::array::from_fn(|i| delta[i] - span * axis[i]);
            let offset = length(radial);
            let a_radius = a.diameter_mm / 2.;
            let b_radius = b.diameter_mm / 2.;
            let radial_gap = (offset - a_radius - b_radius).max(0.);
            let axial_gap = (span - a.depth_mm - b.depth_mm).max(0.);
            let clearance = radial_gap.hypot(axial_gap);
            if clearance >= THIN_WALL_MM - TOLERANCE_MM {
                continue;
            }
            // Closest points on two cylinders = closest points on their axial
            // intervals and on their circular cross sections independently.
            let (a_z, b_z) = if axial_gap > 0. {
                (a.depth_mm, span - b.depth_mm)
            } else {
                let middle = ((span - b.depth_mm).max(0.) + a.depth_mm.min(span)) / 2.;
                (middle, middle)
            };
            let (a_r, b_r) = if radial_gap > 0. {
                (a_radius, offset - b_radius)
            } else {
                let middle =
                    ((offset - b_radius).max(-a_radius) + a_radius.min(offset + b_radius)) / 2.;
                (middle, middle)
            };
            let point = |z: f64, r: f64| {
                part.to_world(std::array::from_fn(|i| {
                    a_entry[i]
                        + axis[i] * z
                        + if offset > ROUNDING {
                            radial[i] * r / offset
                        } else {
                            0.
                        }
                }))
            };
            let first = point(a_z, a_r);
            let second = point(b_z, b_r);
            let round = |x: f64| (x * 1000.).round() / 1000.;
            let meeting = clearance <= TOLERANCE_MM;
            issues.push(Issue {
                severity: if meeting { Severity::Error } else { Severity::Warning },
                kind: if meeting { "opposing_holes_intersect" } else { "opposing_holes_wall_too_thin" },
                parts: vec![part.name.clone()],
                message: format!(
                    "opposing holes {} and {} in {} leave {} mm between their cylindrical bores{}; keep at least {} mm",
                    a.id, b.id, part.name, round(clearance),
                    if meeting { " (they meet or intersect)" } else { "" }, THIN_WALL_MM,
                ),
                where_mm: Some((
                    std::array::from_fn(|i| round(first[i].min(second[i]))),
                    std::array::from_fn(|i| round(first[i].max(second[i]))),
                )),
                hint: "Reduce the drilling depths, stagger the holes farther apart, or increase the part thickness.".to_owned(),
            });
        }
    }
}
