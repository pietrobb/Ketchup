//! Planar boundary operations. Rings use even/odd filling, independent of winding.
//! Boolean boundaries are split at all crossings, classified on both sides, then
//! stitched. No convex-hull or bounding-box replacement of missing material.
use ketchup_geometry::linalg::{cross2 as cross, dot2 as dot, signed_outline_distance};

pub type Point = [f64; 2];
pub type Rings = Vec<Vec<Point>>;
const EPS: f64 = ketchup_model::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn near(a: Point, b: Point) -> bool {
    dot(sub(a, b), sub(a, b)) <= EPS * EPS
}
fn edges(rings: &Rings) -> impl Iterator<Item = (Point, Point)> + '_ {
    rings.iter().flat_map(|ring| {
        ring.iter()
            .copied()
            .zip(ring.iter().copied().cycle().skip(1))
            .take(ring.len())
    })
}

pub fn contains(rings: &Rings, p: Point) -> bool {
    contains_rings(rings.iter(), p)
}

fn contains_rings<'a>(rings: impl Iterator<Item = &'a Vec<Point>>, p: Point) -> bool {
    rings
        .filter(|ring| {
            let mut inside = false;
            for (a, b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                if (a[1] > p[1]) != (b[1] > p[1])
                    && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                {
                    inside = !inside;
                }
            }
            inside
        })
        .count()
        % 2
        == 1
}

pub fn on_boundary(rings: &Rings, p: Point, tolerance: f64) -> bool {
    rings
        .iter()
        .any(|ring| signed_outline_distance(ring, p).is_some_and(|d| d.abs() <= tolerance))
}

fn edge_candidates(source: &[(Point, Point)]) -> Vec<Vec<usize>> {
    let bounds: Vec<_> = source
        .iter()
        .map(|(a, b)| {
            (
                std::array::from_fn::<_, 2, _>(|i| a[i].min(b[i])),
                std::array::from_fn::<_, 2, _>(|i| a[i].max(b[i])),
            )
        })
        .collect();
    let mut order: Vec<_> = (0..source.len()).collect();
    order.sort_by(|&i, &j| bounds[i].0[0].total_cmp(&bounds[j].0[0]));
    let mut candidates = vec![Vec::new(); source.len()];
    for (position, &i) in order.iter().enumerate() {
        for &j in &order[position..] {
            if bounds[j].0[0] > bounds[i].1[0] + EPS {
                break;
            }
            if bounds[i].0[1] > bounds[j].1[1] + EPS || bounds[j].0[1] > bounds[i].1[1] + EPS {
                continue;
            }
            candidates[i].push(j);
            if i != j {
                candidates[j].push(i);
            }
        }
    }
    candidates
} // Split only segment pairs whose bounding intervals overlap.
pub fn boolean(a: &Rings, b: &Rings, subtract: bool) -> Rings {
    let source: Vec<_> = edges(a)
        .chain(edges(b))
        .filter(|(p, q)| !near(*p, *q))
        .collect();
    let bounds = |rings: &Rings| {
        rings
            .iter()
            .map(|ring| {
                ring.iter().fold(
                    ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                    |(lo, hi), p| {
                        (
                            std::array::from_fn(|i| lo[i].min(p[i])),
                            std::array::from_fn(|i| hi[i].max(p[i])),
                        )
                    },
                )
            })
            .collect::<Vec<_>>()
    };
    let bounds_a = bounds(a);
    let bounds_b = bounds(b);
    let filled = |p: Point| {
        let inside = |(lo, hi): &(Point, Point)| (0..2).all(|i| p[i] >= lo[i] && p[i] <= hi[i]);
        contains_rings(
            a.iter()
                .zip(&bounds_a)
                .filter(|(_, bounds)| inside(bounds))
                .map(|(ring, _)| ring),
            p,
        ) && (contains_rings(
            b.iter()
                .zip(&bounds_b)
                .filter(|(_, bounds)| inside(bounds))
                .map(|(ring, _)| ring),
            p,
        ) != subtract)
    };
    let mut boundary: Vec<(Point, Point)> = Vec::new();
    let candidates = edge_candidates(&source);
    for (index, &(p, q)) in source.iter().enumerate() {
        let d = sub(q, p);
        let length = dot(d, d).sqrt();
        let mut cuts = vec![0.0, 1.0];
        for &other in &candidates[index] {
            let (r, s) = source[other];
            // The sweep rejects only disjoint bounds.
            // All intersecting or tolerance-adjacent pairs still use
            // the same exact segment formulas and sorted cut parameters.
            // This also retains collinear overlaps.

            let e = sub(s, r);
            let determinant = cross(d, e);
            if determinant.abs() > EPS * length * dot(e, e).sqrt() {
                let t = cross(sub(r, p), e) / determinant;
                let u = cross(sub(r, p), d) / determinant;
                if (-EPS..=1.0 + EPS).contains(&t) && (-EPS..=1.0 + EPS).contains(&u) {
                    cuts.push(t.clamp(0.0, 1.0));
                }
            } else if cross(sub(r, p), d).abs() <= EPS * length {
                for end in [r, s] {
                    let t = dot(sub(end, p), d) / dot(d, d);
                    if (0.0..=1.0).contains(&t) {
                        cuts.push(t);
                    }
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|x, y| (*x - *y).abs() * length <= EPS);
        let at = |t: f64| [p[0] + t * d[0], p[1] + t * d[1]];
        for pair in cuts.windows(2) {
            let (start, end) = (at(pair[0]), at(pair[1]));
            let middle = at((pair[0] + pair[1]) * 0.5);
            let delta =
                (length * (pair[1] - pair[0]) * ketchup_model::tolerance::BOUNDARY_PROBE_FRACTION)
                    .min(EPS * 8.0);
            let normal = [-d[1] / length * delta, d[0] / length * delta];
            let left = filled([middle[0] + normal[0], middle[1] + normal[1]]);
            let right = filled([middle[0] - normal[0], middle[1] - normal[1]]);
            if left == right {
                continue;
            }
            let edge = if left { (start, end) } else { (end, start) };
            if !boundary
                .iter()
                .any(|&(x, y)| near(x, edge.0) && near(y, edge.1))
            {
                boundary.push(edge);
            }
        }
    }
    let mut rings = Vec::new();
    while let Some((start, mut end)) = boundary.pop() {
        let mut ring = vec![start];
        while !near(end, start) {
            ring.push(end);
            let Some(index) = boundary.iter().position(|(p, _)| near(*p, end)) else {
                break;
            };
            end = boundary.remove(index).1;
        }
        if near(end, start) && ring.len() >= 3 {
            // Remove collinear split points, retaining the actual concave vertices.
            let original = ring.clone();
            ring = (0..original.len())
                .filter_map(|i| {
                    let p = original[(i + original.len() - 1) % original.len()];
                    let q = original[i];
                    let r = original[(i + 1) % original.len()];
                    (cross(sub(q, p), sub(r, q)).abs() > EPS * (dot(sub(r, p), sub(r, p)).sqrt()))
                        .then_some(q)
                })
                .collect();
            if ring.len() >= 3 {
                rings.push(ring);
            }
        }
    }
    rings
}

pub fn area(rings: &Rings) -> f64 {
    edges(rings).map(|(a, b)| cross(a, b)).sum::<f64>().abs() * 0.5
}

/// Encode multiple rings in the existing single-walk API. Every synthetic
/// connector is traversed in both directions, so area and even/odd filling
/// are unchanged. Consumers measuring edge clearance must cancel these pairs.
pub fn walk(rings: &Rings) -> Vec<Point> {
    let Some(first) = rings.first() else {
        return Vec::new();
    };
    let mut points = first.clone();
    for ring in rings.iter().skip(1).filter(|r| !r.is_empty()) {
        points.push(first[0]);
        points.extend(ring.iter().copied());
        points.push(ring[0]);
    }
    points
}
