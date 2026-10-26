//! Fills of an underlay: the areas a drawing paints solid, drawn under its lines in the
//! color of their style. A fill is the part of the plane inside an odd number of its
//! rings, so an island in an area is a hole and an area in that island is painted again.
//! It is never snapped to.

use super::{Reader, UnderlayLinesError, f64_at, push_count};

/// One painted area: closed rings, the last point of each joined back to its first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnderlayFill {
    /// The index of its style ([`super::UnderlayLines::style`]); only its color is drawn.
    pub(super) style: u32,
    /// The index in `points_mm` one past the last point of each ring.
    pub(super) ends: Vec<usize>,
    pub(super) points_mm: Vec<[f64; 2]>,
}

/// The fewest points a ring encloses an area with.
const RING_POINTS: usize = 3;

impl UnderlayFill {
    /// A fill of `rings` (in millimetres, each at least three points) in the style
    /// `style`; rings of fewer points are left out.
    pub(super) fn new(style: u32, rings: &[Vec<[f64; 2]>]) -> Self {
        let mut fill = Self {
            style,
            ..Self::default()
        };
        for ring in rings.iter().filter(|ring| ring.len() >= RING_POINTS) {
            fill.points_mm.extend_from_slice(ring);
            fill.ends.push(fill.points_mm.len());
        }
        fill
    }

    #[must_use]
    pub fn style(&self) -> u32 {
        self.style
    }

    /// Every point of every ring.
    #[must_use]
    pub fn points_mm(&self) -> &[[f64; 2]] {
        &self.points_mm
    }

    pub fn rings(&self) -> impl Iterator<Item = &[[f64; 2]]> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts
            .zip(self.ends.iter().copied())
            .map(|(start, end)| &self.points_mm[start..end])
    }

    /// Whether it paints `point_mm`: inside an odd number of its rings, as
    /// [`Self::triangles`] paints them.
    #[must_use]
    pub fn contains(&self, point_mm: [f64; 2]) -> bool {
        let [x, y] = point_mm;
        let mut inside = false;
        for ring in self.rings() {
            for (index, a) in ring.iter().enumerate() {
                let b = ring[(index + 1) % ring.len()];
                if (a[1] > y) != (b[1] > y) && x < a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
                {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// How many edge crossings [`Self::triangles`] visits: its work.
    #[must_use]
    pub fn sweep_crossings(&self) -> usize {
        sweep_crossings(self.rings())
    }

    /// Triangles that cover exactly its area: between each two heights at which a ring
    /// has a point, the edges crossing that band are paired left to right (even-odd). A
    /// pair of the same two edges in the next band carries its trapezoid on, so each one
    /// ends only where a point changes its pair: a few per point, two triangles each.
    /// The work grows with the heights times the edges ([`sweep_crossings`] counts it).
    #[must_use]
    pub fn triangles(&self) -> Vec<[[f64; 2]; 3]> {
        // Each edge from its lower end to its upper one; a level edge bounds no band.
        let mut edges = self
            .rings()
            .flat_map(|ring| {
                ring.iter()
                    .zip(ring.iter().cycle().skip(1))
                    .map(|(start, end)| (*start, *end))
            })
            .filter(|(start, end)| start[1] != end[1])
            .map(|(start, end)| {
                if start[1] < end[1] {
                    (start, end)
                } else {
                    (end, start)
                }
            })
            .collect::<Vec<_>>();
        edges.sort_by(|left, right| left.0[1].total_cmp(&right.0[1]));
        let mut heights = self
            .points_mm
            .iter()
            .map(|point| point[1])
            .collect::<Vec<_>>();
        heights.sort_by(f64::total_cmp);
        heights.dedup();
        let x_at = |(low, high): ([f64; 2], [f64; 2]), y: f64| {
            low[0] + (high[0] - low[0]) * (y - low[1]) / (high[1] - low[1])
        };
        let mut triangles = Vec::new();
        let mut trapezoid = |left: usize, right: usize, bottom: f64, top: f64| {
            let corners = [
                [x_at(edges[left], bottom), bottom],
                [x_at(edges[right], bottom), bottom],
                [x_at(edges[right], top), top],
                [x_at(edges[left], top), top],
            ];
            triangles.push([corners[0], corners[1], corners[2]]);
            triangles.push([corners[0], corners[2], corners[3]]);
        };
        // The edges crossing the band, kept in their order of the band before, so that
        // sorting them again finds them almost sorted.
        let mut active = Vec::<usize>::new();
        let mut next = 0;
        // The trapezoids still open: left edge, right edge, the height they start at;
        // and by left edge, the right edge and start of the one it bounds.
        let mut open = Vec::<(usize, usize, f64)>::new();
        let mut started = vec![None::<(usize, f64)>; edges.len()];
        let mut pairs = Vec::new();
        for band in heights.windows(2) {
            let [bottom, top] = [band[0], band[1]];
            while next < edges.len() && edges[next].0[1] <= bottom {
                active.push(next);
                next += 1;
            }
            // An edge ending at or below the band's bottom no longer crosses it; every
            // other one runs through the whole band, which no point lies inside.
            active.retain(|edge| edges[*edge].1[1] > bottom);
            let middle = (bottom + top) / 2.0;
            active.sort_by(|left, right| {
                x_at(edges[*left], middle).total_cmp(&x_at(edges[*right], middle))
            });
            // A pair the band before had already goes on from where it started.
            pairs.clear();
            for pair in active.chunks_exact(2) {
                let [left, right] = [pair[0], pair[1]];
                let start = match started[left] {
                    Some((same, start)) if same == right => {
                        started[left] = None;
                        start
                    }
                    _ => bottom,
                };
                pairs.push((left, right, start));
            }
            for (left, right, start) in open.drain(..) {
                if started[left].is_some_and(|(same, _)| same == right) {
                    started[left] = None;
                    trapezoid(left, right, start, bottom);
                }
            }
            for (left, right, start) in &pairs {
                started[*left] = Some((*right, *start));
            }
            open.extend_from_slice(&pairs);
        }
        if let Some(top) = heights.last() {
            for (left, right, start) in open {
                trapezoid(left, right, start, *top);
            }
        }
        triangles
    }

    /// Writes its style, its rings' ends and its points.
    pub(super) fn encode(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.style.to_le_bytes());
        push_count(bytes, self.ends.len());
        push_count(bytes, self.points_mm.len());
        for end in &self.ends {
            push_count(bytes, *end);
        }
        for point in &self.points_mm {
            bytes.extend_from_slice(&point[0].to_le_bytes());
            bytes.extend_from_slice(&point[1].to_le_bytes());
        }
    }

    /// Reads what [`Self::encode`] wrote: rings of at least three finite points.
    pub(super) fn decode(reader: &mut Reader<'_>) -> Result<Self, UnderlayLinesError> {
        let style = reader.take(1, 4)?;
        let style = u32::from_le_bytes([style[0], style[1], style[2], style[3]]);
        let ring_count = reader.count()?;
        let point_count = reader.count()?;
        let ends = (0..ring_count)
            .map(|_| reader.count())
            .collect::<Result<Vec<_>, _>>()?;
        let points_mm = reader
            .take(point_count, 16)?
            .chunks_exact(16)
            .map(|point| [f64_at(&point[..8]), f64_at(&point[8..])])
            .collect::<Vec<_>>();
        let mut start = 0;
        for end in &ends {
            if *end < start + RING_POINTS {
                return Err(UnderlayLinesError::InvalidPolyline);
            }
            start = *end;
        }
        if start != points_mm.len() || ends.is_empty() {
            return Err(UnderlayLinesError::InvalidPolyline);
        }
        if points_mm.iter().flatten().any(|value| !value.is_finite()) {
            return Err(UnderlayLinesError::NotFinite);
        }
        Ok(Self {
            style,
            ends,
            points_mm,
        })
    }
}

/// How many edge crossings [`UnderlayFill::triangles`] visits for a fill of `rings`,
/// counted from the heights of their points and edges alone: each band, the edges that
/// cross it. Rings of fewer than three points are left out, as [`UnderlayFill::new`] does.
pub(crate) fn sweep_crossings<'a>(rings: impl IntoIterator<Item = &'a [[f64; 2]]>) -> usize {
    let (mut heights, mut lows, mut highs) = (Vec::new(), Vec::new(), Vec::new());
    for ring in rings.into_iter().filter(|ring| ring.len() >= RING_POINTS) {
        for (start, end) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            heights.push(start[1]);
            if start[1] != end[1] {
                lows.push(start[1].min(end[1]));
                highs.push(start[1].max(end[1]));
            }
        }
    }
    for values in [&mut heights, &mut lows, &mut highs] {
        values.sort_by(f64::total_cmp);
    }
    heights.dedup();
    let (mut started, mut ended, mut count) = (0, 0, 0_usize);
    for band in heights.windows(2) {
        // An edge that has ended by the band's bottom started below it too.
        started += lows[started..].partition_point(|low| *low <= band[0]);
        ended += highs[ended..].partition_point(|high| *high <= band[0]);
        count = count.saturating_add(started - ended);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(triangles: &[[[f64; 2]; 3]]) -> f64 {
        triangles
            .iter()
            .map(|[a, b, c]| {
                ketchup_geometry::linalg::cross2(
                    [b[0] - a[0], b[1] - a[1]],
                    [c[0] - a[0], c[1] - a[1]],
                )
                .abs()
                    / 2.0
            })
            .sum()
    }

    fn square(low: f64, high: f64) -> Vec<[f64; 2]> {
        vec![[low, low], [high, low], [high, high], [low, high]]
    }

    #[test]
    fn an_island_is_a_hole_and_an_area_in_it_is_painted_again() {
        let outer = UnderlayFill::new(0, &[square(0.0, 100.0)]);
        assert!((area(&outer.triangles()) - 10_000.0).abs() < 1e-6);
        let island = UnderlayFill::new(0, &[square(0.0, 100.0), square(30.0, 70.0)]);
        assert!((area(&island.triangles()) - 8_400.0).abs() < 1e-6);
        let nested = UnderlayFill::new(
            0,
            &[square(0.0, 100.0), square(30.0, 70.0), square(40.0, 60.0)],
        );
        assert!((area(&nested.triangles()) - 8_800.0).abs() < 1e-6);
    }

    /// An outer square with `islands` small squares side by side, each a little higher
    /// than the one before, so that every point has a height of its own.
    fn staggered(islands: u32) -> Vec<Vec<[f64; 2]>> {
        let mut rings = vec![square(0.0, 40_000.0)];
        for island in 0..islands {
            let [x, y] = [
                20.0 + 30.0 * f64::from(island),
                100.0 + 0.001 * f64::from(island),
            ];
            rings.push(vec![
                [x, y],
                [x + 10.0, y],
                [x + 10.0, y + 10.0],
                [x, y + 10.0],
            ]);
        }
        rings
    }

    #[test]
    fn islands_at_heights_of_their_own_take_a_few_triangles_per_point() {
        // Each band crosses every island, so the sweep's work grows with the square of
        // the points, and an import counts it first; its triangles grow with the points.
        for islands in [500, 1000] {
            let fill = UnderlayFill::new(0, &staggered(islands));
            let triangles = fill.triangles();
            let points = fill.points_mm().len();
            assert!(
                triangles.len() <= 4 * points,
                "{} > 4 × {points}",
                triangles.len()
            );
            let painted = 40_000.0 * 40_000.0 - 100.0 * f64::from(islands);
            assert!((area(&triangles) - painted).abs() < 1e-3 * painted.sqrt());
        }
        let fill = UnderlayFill::new(0, &staggered(500));
        assert_eq!(fill.points_mm().len(), 2_004);
        assert_eq!(fill.sweep_crossings(), 502_002);
        let flat = UnderlayFill::new(0, &[vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]]]);
        assert_eq!(flat.sweep_crossings(), 0);
    }

    #[test]
    fn a_ring_turning_either_way_or_of_one_line_paints_alike() {
        let mut clockwise = square(0.0, 10.0);
        clockwise.reverse();
        let fill = UnderlayFill::new(0, &[clockwise]);
        assert!((area(&fill.triangles()) - 100.0).abs() < 1e-9);
        let flat = UnderlayFill::new(0, &[vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]]]);
        assert!(flat.triangles().is_empty());
    }
}
