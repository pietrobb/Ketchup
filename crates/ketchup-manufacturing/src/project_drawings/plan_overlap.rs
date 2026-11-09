//! How much of one level lies over another in plan, their triangles found
//! through a grid of the boxes around them, not each against each (review
//! 2026-11-07 P3: three levels of 20 000 triangles took 3 s, and nothing
//! stopped it).
use super::polygon_area;
use ketchup_geometry::linalg::cross2;
use ketchup_model::tolerance::ROUNDING;
use std::sync::atomic::{AtomicBool, Ordering};

type Triangle = [[f64; 2]; 3];

/// A convex piece and its box: its least and its greatest corner.
type Piece = (Vec<[f64; 2]>, [[f64; 2]; 2]);

/// The most cells along one side of the grid.
const MOST_CELLS_ACROSS: u32 = 1024;

/// The most cells the boxes of a level's triangles are in, on average, before
/// the grid is made coarser: a fan of triangles, as of a round slab, each
/// from its middle to its edge, is in a great many (review 2026-11-07 r26 P3).
const MOST_CELLS_PER_TRIANGLE: usize = 16;

/// How many of theirs are looked at between two looks at `stop`.
const LOOKS_BETWEEN_STOPS: usize = 1 << 16;

/// The share of a level's triangles, at each side, left out of where its
/// grid is laid: one far away, as a survey point a kilometre off, made a cell
/// of the whole level (review 2026-11-07 r26 P3). They are in the outer cells.
const TRIANGLES_OUTSIDE_PER_SIDE: usize = 100;

/// The area of `mine` over `theirs`, both counter-clockwise; what of each of
/// mine lies under several of theirs counts once, however many lie on one
/// another there, in whole or in part (review 2026-11-07 P3: a floor laid
/// twice, two layers of one top, counted twice; review 2026-11-08: so did
/// two over part of one). None once `stop` is set.
pub(super) fn area_over(mine: &[Triangle], theirs: &[Triangle], stop: &AtomicBool) -> Option<f64> {
    overlap(mine, theirs, stop).map(|(area, _)| area)
}

/// The area of `mine` over `theirs`, and how many times one of theirs, or a
/// piece of mine against one of theirs, was looked at. One of mine is cut
/// along the cells of the grid, each part against the triangles of its cell
/// alone, so a slab of two triangles over a mesh of thousands is cut where
/// each lies, not as a whole (review 2026-11-09 MF-303: 3 200 took 454 s),
/// and only in the cells it reaches (review 2026-11-07 r26 P3: a fan took
/// 231 s, 16.5 s each against each).
fn overlap(mine: &[Triangle], theirs: &[Triangle], stop: &AtomicBool) -> Option<(f64, usize)> {
    let grid = Grid::of(theirs);
    let mut looks = Looks {
        looked: grid.entries,
        next_stop: 0,
        stop,
    };
    let mut total = 0.0;
    for triangle in mine {
        looks.count(0)?;
        let [min, max] = boxed(triangle);
        let ([first_column, last_column], [first_row, last_row]) = grid.span(min, max);
        for row in first_row..=last_row {
            // The cells of the row the triangle reaches, not all its box spans:
            // a thin one of a fan crosses few of them.
            let [[_, bottom], [_, top]] = grid.cell_box(first_column, row);
            let band = clipped_to(
                triangle,
                [[f64::NEG_INFINITY, bottom], [f64::INFINITY, top]],
            );
            if band.is_empty() {
                continue;
            }
            let [low, high] = boxed_points(&band);
            let ([from, to], _) = grid.span(low, high);
            for column in from.max(first_column)..=to.min(last_column) {
                let part = clipped_to(&band, grid.cell_box(column, row));
                let cell = grid.cell(column, row);
                total += covered(&part, cell, theirs, &grid.boxes, &mut looks)?;
            }
        }
    }
    Some((total, looks.looked))
}

/// How many looks were made, and when `stop` is looked at next.
struct Looks<'a> {
    looked: usize,
    next_stop: usize,
    stop: &'a AtomicBool,
}

impl Looks<'_> {
    /// Counts `count` more looks; None once `stop` is set, which is looked at
    /// every [`LOOKS_BETWEEN_STOPS`].
    fn count(&mut self, count: usize) -> Option<()> {
        self.looked += count;
        if self.looked >= self.next_stop {
            if self.stop.load(Ordering::Relaxed) {
                return None;
            }
            self.next_stop = self.looked + LOOKS_BETWEEN_STOPS;
        }
        Some(())
    }
}

/// The area of the convex `part` over the `candidates` of `theirs`, whose boxes
/// are `boxes`: what lies under several counts once (review 2026-11-08: two of
/// theirs on one another over part of mine counted twice). None once `stop` is
/// set.
fn covered(
    part: &[[f64; 2]],
    candidates: &[usize],
    theirs: &[Triangle],
    boxes: &[[[f64; 2]; 2]],
    looks: &mut Looks<'_>,
) -> Option<f64> {
    let own = polygon_area(part);
    if own <= ROUNDING * perimeter(part) {
        return Some(0.0);
    }
    let part_box = boxed_points(part);
    // What of `part` none of theirs looked at so far lies over, in pieces that
    // do not overlap, each with its box.
    let mut uncovered = vec![(part.to_vec(), part_box)];
    for &candidate in candidates {
        if uncovered.is_empty() {
            break;
        }
        looks.count(1)?;
        let within = boxes[candidate];
        if overlapping(part_box, within) {
            // Each piece looked at counts, so a cancel is seen while a part
            // is cut into many (review 2026-11-09 MF-303).
            looks.count(uncovered.len())?;
            subtract(&mut uncovered, &theirs[candidate], within);
        }
    }
    let left: f64 = uncovered.iter().map(|(piece, _)| polygon_area(piece)).sum();
    Some((own - left).max(0.0))
}

/// The part of the convex `polygon` within the box from `min` to `max`, whose
/// sides may be infinite.
fn clipped_to(polygon: &[[f64; 2]], [min, max]: [[f64; 2]; 2]) -> Vec<[f64; 2]> {
    // Each side as a line with the box on its left.
    let sides = [
        (min[0], [min[0], 0.0], [min[0], -1.0]),
        (max[0], [max[0], 0.0], [max[0], 1.0]),
        (min[1], [0.0, min[1]], [1.0, min[1]]),
        (max[1], [0.0, max[1]], [-1.0, max[1]]),
    ];
    let mut points = polygon.to_vec();
    for (at, from, to) in sides {
        if at.is_finite() {
            points = split(&points, from, to).0;
            if points.is_empty() {
                break;
            }
        }
    }
    points
}

/// Whether the boxes `a` and `b` overlap by more than [`ROUNDING`] along both axes.
fn overlapping([min, max]: [[f64; 2]; 2], [other_min, other_max]: [[f64; 2]; 2]) -> bool {
    (0..2).all(|axis| max[axis].min(other_max[axis]) - min[axis].max(other_min[axis]) > ROUNDING)
}

/// Whether the convex `piece` and `within`, both counter-clockwise, share no
/// area: a side of one has all of the other on its outside or on it.
fn apart(piece: &[[f64; 2]], within: &Triangle) -> bool {
    let beyond = |from: [f64; 2], to: [f64; 2], points: &[[f64; 2]]| {
        let edge = [to[0] - from[0], to[1] - from[1]];
        // A side shorter than ROUNDING, where a cut ran through or by a corner,
        // points nowhere it can be trusted to.
        edge[0].hypot(edge[1]) > ROUNDING
            && points
                .iter()
                .all(|point| cross2(edge, [point[0] - from[0], point[1] - from[1]]) <= 0.0)
    };
    (0..3).any(|index| beyond(within[index], within[(index + 1) % 3], piece))
        || (0..piece.len())
            .any(|index| beyond(piece[index], piece[(index + 1) % piece.len()], within))
}

/// `uncovered` less `within`, whose box is `within_box`: only a piece it
/// overlaps is cut (review 2026-11-09 MF-303).
fn subtract(uncovered: &mut Vec<Piece>, within: &Triangle, within_box: [[f64; 2]; 2]) {
    let mut left = Vec::with_capacity(uncovered.len() + 3);
    for (piece, piece_box) in uncovered.drain(..) {
        if overlapping(piece_box, within_box) && !apart(&piece, within) {
            left.extend(outside(&piece, within).into_iter().map(|part| {
                let part_box = boxed_points(&part);
                (part, part_box)
            }));
        } else {
            left.push((piece, piece_box));
        }
    }
    *uncovered = left;
}

/// The first and last column, and row, of cells.
type Span = ([usize; 2], [usize; 2]);

/// The triangles of a level by the cells of a uniform grid their boxes touch.
struct Grid {
    /// Where each column, and each row, begins.
    starts: [Vec<f64>; 2],
    cells: Vec<Vec<usize>>,
    /// How many triangles all cells hold.
    entries: usize,
    boxes: Vec<[[f64; 2]; 2]>,
}

impl Grid {
    /// About one cell per triangle, at most [`MOST_CELLS_ACROSS`] a side,
    /// laid where all but the farthest triangles are, and coarser while their
    /// boxes are in more than [`MOST_CELLS_PER_TRIANGLE`] cells each.
    fn of(triangles: &[Triangle]) -> Self {
        let boxes: Vec<[[f64; 2]; 2]> = triangles.iter().map(boxed).collect();
        let [min, max] = laid(&boxes);
        let size = [(max[0] - min[0]).max(0.0), (max[1] - min[1]).max(0.0)];
        let count = f64::from(u32::try_from(triangles.len()).unwrap_or(u32::MAX).max(1));
        let side = (size[0] * size[1] / count).sqrt();
        let mut across = [0, 1].map(|axis| {
            if side > ROUNDING && size[axis].is_finite() {
                (1..MOST_CELLS_ACROSS)
                    .find(|cells| f64::from(*cells) * side >= size[axis])
                    .unwrap_or(MOST_CELLS_ACROSS)
            } else {
                1
            }
        });
        let most = MOST_CELLS_PER_TRIANGLE.saturating_mul(boxes.len());
        let mut grid = loop {
            let starts = [0, 1].map(|axis| {
                let origin = if min[axis].is_finite() {
                    min[axis]
                } else {
                    0.0
                };
                let cell = size[axis] / f64::from(across[axis]);
                (0..across[axis])
                    .map(|column| origin + f64::from(column) * cell)
                    .collect::<Vec<f64>>()
            });
            let grid = Self {
                cells: Vec::new(),
                entries: 0,
                starts,
                boxes: Vec::new(),
            };
            let entries = boxes
                .iter()
                .map(|[low, high]| cells_in(grid.span(*low, *high)))
                .fold(0, usize::saturating_add);
            if entries <= most || across == [1, 1] {
                break Self { entries, ..grid };
            }
            across = across.map(|cells| (cells / 2).max(1));
        };
        let columns = grid.starts[0].len();
        let rows = grid.starts[1].len();
        grid.cells = vec![Vec::new(); columns * rows];
        for (index, [low, high]) in boxes.iter().enumerate() {
            let ([first_column, last_column], [first_row, last_row]) = grid.span(*low, *high);
            for row in first_row..=last_row {
                for column in first_column..=last_column {
                    grid.cells[row * columns + column].push(index);
                }
            }
        }
        grid.boxes = boxes;
        grid
    }

    /// The first and last column, and row, of the cells from `min` to `max`;
    /// what is outside the grid is in its outer cells.
    fn span(&self, min: [f64; 2], max: [f64; 2]) -> Span {
        let at = |axis: usize, value: f64| {
            self.starts[axis]
                .partition_point(|start| *start <= value)
                .saturating_sub(1)
        };
        (
            [at(0, min[0]), at(0, max[0])],
            [at(1, min[1]), at(1, max[1])],
        )
    }

    /// The box of the cell at `column` and `row`; the outer cells reach on
    /// without end, as [`Grid::span`] puts what is outside the grid in them.
    fn cell_box(&self, column: usize, row: usize) -> [[f64; 2]; 2] {
        let bounds = |axis: usize, index: usize| {
            let starts = &self.starts[axis];
            let low = if index == 0 {
                f64::NEG_INFINITY
            } else {
                starts[index]
            };
            [low, starts.get(index + 1).copied().unwrap_or(f64::INFINITY)]
        };
        let ([left, right], [bottom, top]) = (bounds(0, column), bounds(1, row));
        [[left, bottom], [right, top]]
    }

    /// The triangles whose boxes touch the cell at `column` and `row`.
    fn cell(&self, column: usize, row: usize) -> &[usize] {
        &self.cells[row * self.starts[0].len() + column]
    }
}

/// How many cells `span` is.
fn cells_in(([first_column, last_column], [first_row, last_row]): Span) -> usize {
    (last_column - first_column + 1).saturating_mul(last_row - first_row + 1)
}

/// The least and greatest corner of where the grid of `boxes` is laid: around
/// them all, but for the [`TRIANGLES_OUTSIDE_PER_SIDE`]th part farthest at
/// each side. None of them, none of it.
fn laid(boxes: &[[[f64; 2]; 2]]) -> [[f64; 2]; 2] {
    let mut corners = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    if boxes.is_empty() {
        return corners;
    }
    let outside = boxes.len() / TRIANGLES_OUTSIDE_PER_SIDE;
    for axis in 0..2 {
        let mut lows: Vec<f64> = boxes.iter().map(|[low, _]| low[axis]).collect();
        let mut highs: Vec<f64> = boxes.iter().map(|[_, high]| high[axis]).collect();
        corners[0][axis] = *lows.select_nth_unstable_by(outside, f64::total_cmp).1;
        corners[1][axis] = *highs
            .select_nth_unstable_by(boxes.len() - 1 - outside, f64::total_cmp)
            .1;
    }
    corners
}

/// The box around `triangle`: its least and its greatest corner.
fn boxed(triangle: &Triangle) -> [[f64; 2]; 2] {
    boxed_points(triangle)
}

/// The box around `points`: their least and their greatest corner.
fn boxed_points(points: &[[f64; 2]]) -> [[f64; 2]; 2] {
    points.iter().fold(
        [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
        |[min, max], corner| {
            [
                [min[0].min(corner[0]), min[1].min(corner[1])],
                [max[0].max(corner[0]), max[1].max(corner[1])],
            ]
        },
    )
}

/// The part of `triangle` inside `within`, both counter-clockwise.
#[cfg(test)]
fn clipped(triangle: &Triangle, within: &Triangle) -> Vec<[f64; 2]> {
    let mut points = triangle.to_vec();
    for index in 0..3 {
        points = split(&points, within[index], within[(index + 1) % 3]).0;
        if points.is_empty() {
            break;
        }
    }
    points
}

/// The parts of the convex `piece` outside `within`, both counter-clockwise,
/// none on another; slivers thinner than [`ROUNDING`] are left out. A piece
/// `within` does not overlap is left whole, not cut along its edges carried
/// on (review 2026-11-09 MF-303).
fn outside(piece: &[[f64; 2]], within: &Triangle) -> Vec<Vec<[f64; 2]>> {
    let mut parts = Vec::new();
    let mut inside = piece.to_vec();
    for index in 0..3 {
        let (kept, cut) = split(&inside, within[index], within[(index + 1) % 3]);
        if polygon_area(&cut) > ROUNDING * perimeter(&cut) {
            parts.push(cut);
        }
        inside = kept;
        if inside.is_empty() {
            break;
        }
    }
    if polygon_area(&inside) <= ROUNDING * perimeter(&inside) {
        return vec![piece.to_vec()];
    }
    parts
}

/// The convex `points` on the left of the line `from` `to`, and on its right.
fn split(points: &[[f64; 2]], from: [f64; 2], to: [f64; 2]) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    let edge = [to[0] - from[0], to[1] - from[1]];
    let side = |point: [f64; 2]| cross2(edge, [point[0] - from[0], point[1] - from[1]]);
    let mut left = Vec::with_capacity(points.len() + 1);
    let mut right = Vec::with_capacity(points.len() + 1);
    for (position, point) in points.iter().enumerate() {
        let next = points[(position + 1) % points.len()];
        let (here, there) = (side(*point), side(next));
        if here >= 0.0 {
            left.push(*point);
        } else {
            right.push(*point);
        }
        if (here >= 0.0) != (there >= 0.0) {
            let t = here / (here - there);
            let crossing = [
                point[0] + t * (next[0] - point[0]),
                point[1] + t * (next[1] - point[1]),
            ];
            left.push(crossing);
            right.push(crossing);
        }
    }
    (left, right)
}

/// How far around `points`.
fn perimeter(points: &[[f64; 2]]) -> f64 {
    (0..points.len())
        .map(|index| {
            let [from, to] = [points[index], points[(index + 1) % points.len()]];
            (to[0] - from[0]).hypot(to[1] - from[1])
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `across` × `across` square tiles 50 mm a side, two triangles each.
    fn tiles(across: u32) -> Vec<Triangle> {
        let mut triangles = Vec::new();
        for index in 0..across * across {
            let [x, y] = [
                f64::from(index % across) * 50.0,
                f64::from(index / across) * 50.0,
            ];
            triangles.push([[x, y], [x + 50.0, y], [x + 50.0, y + 50.0]]);
            triangles.push([[x, y], [x + 50.0, y + 50.0], [x, y + 50.0]]);
        }
        triangles
    }

    /// A round slab of `count` triangles from its middle, 1 m across.
    fn fan(count: u32) -> Vec<Triangle> {
        let corner = |index: u32| {
            let angle = std::f64::consts::TAU * f64::from(index) / f64::from(count);
            [500.0 * angle.cos(), 500.0 * angle.sin()]
        };
        (0..count)
            .map(|index| [[0.0, 0.0], corner(index), corner(index + 1)])
            .collect()
    }

    /// A mesh of `2 × across²` triangles over the square 0 to 2 000 mm, its
    /// inner corners moved by up to a third of a cell, as a slab meshed
    /// unevenly is.
    fn uneven(across: u32) -> Vec<Triangle> {
        let cell = 2000.0 / f64::from(across);
        let corner = |column: u32, row: u32| {
            let mut point = [f64::from(column) * cell, f64::from(row) * cell];
            if (1..across).contains(&column) && (1..across).contains(&row) {
                // A fixed scatter, the same on every run.
                let seed = f64::from(column * 7919 + row * 104_729);
                point[0] += cell / 3.0 * (seed * 0.618_033_988_7).fract().mul_add(2.0, -1.0);
                point[1] += cell / 3.0 * (seed * 0.414_213_562_3).fract().mul_add(2.0, -1.0);
            }
            point
        };
        let mut triangles = Vec::new();
        for row in 0..across {
            for column in 0..across {
                let [a, b, c, d] = [
                    corner(column, row),
                    corner(column + 1, row),
                    corner(column + 1, row + 1),
                    corner(column, row + 1),
                ];
                triangles.push([a, b, c]);
                triangles.push([a, c, d]);
            }
        }
        triangles
    }

    /// Review 2026-11-09 MF-303: each of theirs cut every piece of mine still
    /// uncovered along its edges carried on, pieces it does not touch too, so a
    /// slab of two triangles over 3 200 took 454 s, and a cancel was not seen.
    #[test]
    fn a_slab_over_an_uneven_mesh_is_cut_only_where_they_overlap() {
        let slab = rectangle([0.0, 0.0], [2000.0, 2000.0]);
        let mesh = uneven(40);
        assert_eq!(mesh.len(), 3200);
        let (area, looked) = overlap(&slab, &mesh, &AtomicBool::new(false)).unwrap();
        assert!((area - 2000.0 * 2000.0).abs() < 1.0, "{area}");
        assert!(looked <= 64 * mesh.len(), "{looked}");
        // Half the slab over the mesh, half beside it.
        let beside = rectangle([1000.0, 0.0], [3000.0, 2000.0]);
        let (area, looked) = overlap(&beside, &mesh, &AtomicBool::new(false)).unwrap();
        assert!((area - 1000.0 * 2000.0).abs() < 1.0, "{area}");
        assert!(looked <= 64 * mesh.len(), "{looked}");
    }

    /// Review 2026-11-09 MF-303: 3 200 triangles under a slab of two took
    /// 454 s; now well under a second, and a cancel is seen within 100 ms.
    #[test]
    #[ignore = "perf: release timing budget, runs nightly"]
    fn perf_a_slab_over_an_uneven_mesh_of_3200_takes_under_a_second() {
        let slab = rectangle([0.0, 0.0], [2000.0, 2000.0]);
        let mesh = uneven(40);
        let started = std::time::Instant::now();
        let (area, looked) = overlap(&slab, &mesh, &AtomicBool::new(false)).unwrap();
        let took = started.elapsed();
        println!("{} triangles: {took:?}, {looked} looks", mesh.len());
        assert!((area - 2000.0 * 2000.0).abs() < 1.0, "{area}");
        assert!(took < std::time::Duration::from_secs(1), "{took:?}");
        let longest = took.mul_f64((LOOKS_BETWEEN_STOPS as f64 / looked as f64).min(1.0));
        assert!(
            longest < std::time::Duration::from_millis(100),
            "{longest:?}"
        );
    }

    /// A rectangle `from` to `to`, counter-clockwise, as two triangles.
    fn rectangle(from: [f64; 2], to: [f64; 2]) -> [Triangle; 2] {
        [[from, [to[0], from[1]], to], [from, to, [from[0], to[1]]]]
    }

    /// Review 2026-11-08: each of mine counted at most its own area, but two
    /// of theirs over part of it, one on the other, counted twice: the left
    /// 60 mm and the left 50 mm of a 100 mm square came to 8 050 mm², not
    /// 6 000 mm².
    #[test]
    fn two_of_theirs_on_one_another_over_part_of_mine_count_once() {
        let mine = rectangle([0.0, 0.0], [100.0, 100.0]);
        let theirs = [
            rectangle([0.0, 0.0], [60.0, 100.0]),
            rectangle([0.0, 0.0], [50.0, 100.0]),
            rectangle([-10.0, 20.0], [55.0, 30.0]),
        ]
        .concat();
        let area = area_over(&mine, &theirs, &AtomicBool::new(false)).unwrap();
        assert!((area - 6000.0).abs() < 1e-6, "{area}");
        // Each of theirs over one of mine in part, side by side, still counts.
        let halves = [
            rectangle([0.0, 0.0], [50.0, 100.0]),
            rectangle([50.0, 0.0], [100.0, 100.0]),
        ]
        .concat();
        let area = area_over(&mine, &halves, &AtomicBool::new(false)).unwrap();
        assert!((area - 10_000.0).abs() < 1e-6, "{area}");
    }

    /// Review 2026-11-07 r26 P3: one triangle far away, as a survey point,
    /// made the grid one cell, so each was looked at against each.
    #[test]
    fn a_far_triangle_leaves_the_grid_where_the_level_is() {
        let mine = tiles(40);
        for far in [1.0e6, 5.0e9] {
            let mut theirs = mine.clone();
            theirs.push([[far, far], [far + 50.0, far], [far, far + 50.0]]);
            let (area, looked) = overlap(&mine, &theirs, &AtomicBool::new(false)).unwrap();
            assert!((area - 2000.0 * 2000.0).abs() < 1e-3, "{area}");
            assert!(
                looked <= 64 * (mine.len() + theirs.len()),
                "{far}: {looked}"
            );
        }
    }

    /// Review 2026-11-07 r26 P3: a fan of triangles was in so many cells
    /// that the grid looked at them more often than each against each.
    #[test]
    fn a_fan_is_looked_at_no_more_than_each_against_each() {
        let slab = fan(720);
        let (area, looked) = overlap(&slab, &slab, &AtomicBool::new(false)).unwrap();
        let own: f64 = slab.iter().map(|triangle| polygon_area(triangle)).sum();
        assert!((area - own).abs() < 1e-6 * own, "{area} {own}");
        let grid = MOST_CELLS_PER_TRIANGLE * slab.len();
        assert!(looked <= slab.len() * slab.len() + grid, "{looked}");
    }

    /// Review 2026-11-07 r26 P3: 20 000 triangles of a fan took 231 s, each
    /// against each 16.5 s, and a cancel waited up to 3 s.
    #[test]
    #[ignore = "perf: release timing budget, runs nightly"]
    fn perf_a_fan_is_overlapped_as_fast_as_each_against_each_and_stops_at_once() {
        let slab = fan(20_000);
        let started = std::time::Instant::now();
        let (_, looked) = overlap(&slab, &slab, &AtomicBool::new(false)).unwrap();
        let through_grid = started.elapsed();
        // As before the grid: each against each whose box it overlaps.
        let started = std::time::Instant::now();
        let boxes: Vec<_> = slab.iter().map(boxed).collect();
        let mut each = 0.0;
        for (mine, [min, max]) in slab.iter().zip(&boxes) {
            for (theirs, [other_min, other_max]) in slab.iter().zip(&boxes) {
                if (0..2).all(|axis| {
                    max[axis].min(other_max[axis]) - min[axis].max(other_min[axis]) > ROUNDING
                }) {
                    each += polygon_area(&clipped(mine, theirs));
                }
            }
        }
        let each_against_each = started.elapsed();
        assert!(each > 0.0);
        println!("through the grid {through_grid:?}, each against each {each_against_each:?}");
        assert!(
            through_grid <= each_against_each.mul_f64(1.25),
            "{through_grid:?} against {each_against_each:?}"
        );

        // `stop` is looked at again at most these looks later, those between
        // two looks and the most one of mine makes past them.
        let between = LOOKS_BETWEEN_STOPS + slab.len();
        let longest = through_grid.mul_f64(between as f64 / looked as f64);
        assert!(
            longest < std::time::Duration::from_millis(100),
            "{longest:?}"
        );
    }
}
