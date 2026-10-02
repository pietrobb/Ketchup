//! Boundary sampling and plane sections of generic extrusions.
use super::*;
use crate::contact::polygon::{self, Rings};
use ketchup_model::tolerance::{
    BOUNDARY_CHORD_MM, BOUNDARY_MIN_ANGLE_RAD, DEFAULT_LINEAR_TOLERANCE_MM,
};

fn distance2(a: [f64; 2], b: [f64; 2]) -> f64 {
    ketchup_geometry::linalg::distance(flat(a), flat(b))
}

/// Curved edges are sampled with a BOUNDARY_CHORD_MM mm chord tolerance. Straight edges
/// retain their exact vertices (including concave ones).
pub(super) fn profile_outline(segments: &[ProgramProfileSegment]) -> Vec<[f64; 2]> {
    let mut points = Vec::new();
    for segment in segments {
        points.push(segment.start_mm);
        if let Some(arc) = segment.arc {
            let radius = distance2(segment.start_mm, arc.center_mm);
            let start = (segment.start_mm[1] - arc.center_mm[1])
                .atan2(segment.start_mm[0] - arc.center_mm[0]);
            let end =
                (segment.end_mm[1] - arc.center_mm[1]).atan2(segment.end_mm[0] - arc.center_mm[0]);
            let sign = if arc.clockwise { -1.0 } else { 1.0 };
            let span = ((end - start) * sign).rem_euclid(TAU);
            let span = if span < ketchup_model::tolerance::ROUNDING {
                TAU
            } else {
                span
            };
            let step = (2.0 * (1.0 - BOUNDARY_CHORD_MM / radius.max(BOUNDARY_CHORD_MM)).acos())
                .max(BOUNDARY_MIN_ANGLE_RAD);
            let count = (span / step).ceil() as usize;
            for i in 1..count {
                let angle = start + sign * span * i as f64 / count as f64;
                points.push([
                    arc.center_mm[0] + radius * angle.cos(),
                    arc.center_mm[1] + radius * angle.sin(),
                ]);
            }
        } else if segment.bezier.is_some() {
            let mut stack = vec![(0.0, 1.0, 0_u32)];
            while let Some((a, b, depth)) = stack.pop() {
                let p = segment.bezier_point(a);
                let q = segment.bezier_point(b);
                let deviation = [0.25, 0.5, 0.75]
                    .into_iter()
                    .map(|t| {
                        let actual = segment.bezier_point(a + (b - a) * t);
                        distance2(actual, [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t])
                    })
                    .fold(0.0, f64::max);
                if deviation > BOUNDARY_CHORD_MM && depth < 20 {
                    stack.push(((a + b) * 0.5, b, depth + 1));
                    stack.push((a, (a + b) * 0.5, depth + 1));
                } else if b < 1.0 {
                    points.push(q);
                }
            }
        }
    }
    points
}

/// Section of an extrusion in world coordinates projected onto `face`.
/// Handles arbitrary relative rotation, concavity, and parallel side cuts.
pub(super) fn extrusion_section(tool: &Part, face: &FaceFrame) -> Rings {
    let ProgramPartBody::Extrusion {
        segments,
        distance_mm,
        ..
    } = &tool.body
    else {
        return Vec::new();
    };
    let outline = profile_outline(segments);
    let normal = frame::apply(&frame::transposed(&tool.rotation), face.normal);
    let origin = tool.to_local(face.origin_mm);
    let offset = dot(normal, origin);
    let project = |p| face.coordinates(tool.to_world(p));
    if normal[2].abs() > ketchup_model::tolerance::ROUNDING {
        let height = |p: [f64; 2]| (offset - normal[0] * p[0] - normal[1] * p[1]) / normal[2];
        let projected: Rings = vec![
            outline
                .iter()
                .map(|&p| project([p[0], p[1], height(p)]))
                .collect(),
        ];
        let heights: Vec<_> = outline.iter().map(|&point| height(point)).collect();
        if heights.iter().all(|&z| z < -DEFAULT_LINEAR_TOLERANCE_MM)
            || heights
                .iter()
                .all(|&z| z > distance_mm + DEFAULT_LINEAR_TOLERANCE_MM)
        {
            return Vec::new();
        }
        if heights.iter().all(|&z| {
            z >= -DEFAULT_LINEAR_TOLERANCE_MM && z <= distance_mm + DEFAULT_LINEAR_TOLERANCE_MM
        }) {
            return projected;
        }
        // Clip a convex enclosing rectangle by the two cap half-planes, then
        // intersect it with the actual outline, not a convex replacement.
        let (lo, hi) = projected[0].iter().fold(
            ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
            |(lo, hi), p| {
                (
                    std::array::from_fn(|i| lo[i].min(p[i] - 1.0)),
                    std::array::from_fn(|i| hi[i].max(p[i] + 1.0)),
                )
            },
        );
        let mut clip = vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]];
        for upper in [false, true] {
            let value = |p| {
                let z = tool.to_local(face.point(p))[2];
                if upper { z - distance_mm } else { -z }
            };
            let mut next = Vec::new();
            for (p, q) in clip
                .iter()
                .copied()
                .zip(clip.iter().copied().cycle().skip(1))
                .take(clip.len())
            {
                let (a, b) = (value(p), value(q));
                if a <= DEFAULT_LINEAR_TOLERANCE_MM {
                    next.push(p);
                }
                if (a <= DEFAULT_LINEAR_TOLERANCE_MM) != (b <= DEFAULT_LINEAR_TOLERANCE_MM) {
                    let t = a / (a - b);
                    next.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
                }
            }
            clip = next;
        }
        if clip.len() < 3 {
            return Vec::new();
        }
        polygon::boolean(&projected, &vec![clip], false)
    } else {
        let direction = [-normal[1], normal[0]];
        let value =
            |p: [f64; 2]| ketchup_geometry::linalg::dot2([normal[0], normal[1]], p) - offset;
        let mut hits = Vec::new();
        for (&p, &q) in outline
            .iter()
            .zip(outline.iter().cycle().skip(1))
            .take(outline.len())
        {
            let (a, b) = (value(p), value(q));
            if a.abs() <= DEFAULT_LINEAR_TOLERANCE_MM {
                hits.push(p);
            }
            if a * b < 0.0 {
                let t = a / (a - b);
                hits.push([p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]);
            }
        }
        hits.sort_by(|p, q| {
            ketchup_geometry::linalg::dot2(*p, direction)
                .total_cmp(&ketchup_geometry::linalg::dot2(*q, direction))
        });
        hits.dedup_by(|p, q| distance2(*p, *q) < DEFAULT_LINEAR_TOLERANCE_MM);
        let outline = vec![outline];
        hits.windows(2)
            .filter_map(|pair| {
                let (p, q) = (pair[0], pair[1]);
                let mid = [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5];
                (polygon::contains(&outline, mid)
                    || polygon::on_boundary(&outline, mid, DEFAULT_LINEAR_TOLERANCE_MM))
                .then(|| {
                    vec![
                        project([p[0], p[1], 0.0]),
                        project([q[0], q[1], 0.0]),
                        project([q[0], q[1], *distance_mm]),
                        project([p[0], p[1], *distance_mm]),
                    ]
                })
            })
            .collect()
    }
}
