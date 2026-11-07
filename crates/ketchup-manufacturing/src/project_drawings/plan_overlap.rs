//! How much of one level lies over another in plan, their triangles found
//! through a grid of the boxes around them, not each against each (review
//! 2026-11-07 P3: three levels of 20 000 triangles took 3 s, and nothing
//! stopped it).
use super::polygon_area;
use ketchup_geometry::linalg::cross2;
use ketchup_model::tolerance::ROUNDING;
use std::sync::atomic::{AtomicBool, Ordering};

type Triangle = [[f64; 2]; 3];

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

/// The area of `mine` over `theirs`, both counter-clockwise; each of mine
/// counts at most its own area, however many of theirs lie on one another
/// (review 2026-11-07 P3: a floor laid twice, two layers of one top, counted
/// twice). None once `stop` is set.
pub(super) fn area_over(mine: &[Triangle], theirs: &[Triangle], stop: &AtomicBool) -> Option<f64> {
    overlap(mine, theirs, stop).map(|(area, _)| area)
}

/// The area of `mine` over `theirs`, and how many times one of theirs was
/// looked at, in the grid and against one of mine: never more than one of
/// theirs against each of mine and the grid, as `theirs` are passed in order
/// where the cells near one of mine hold more than half as many (review
/// 2026-11-07 r26 P3: a fan took 231 s, 16.5 s each against each).
fn overlap(mine: &[Triangle], theirs: &[Triangle], stop: &AtomicBool) -> Option<(f64, usize)> {
    let grid = Grid::of(theirs);
    // The triangle of mine each of theirs was last overlapped with, so one
    // in several cells is clipped once.
    let mut seen = vec![usize::MAX; theirs.len()];
    let mut looked = grid.entries;
    let mut next_stop = 0;
    let mut total = 0.0;
    for (index, triangle) in mine.iter().enumerate() {
        if looked >= next_stop {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            next_stop = looked + LOOKS_BETWEEN_STOPS;
        }
        let [min, max] = boxed(triangle);
        let clip = |candidate: usize| {
            let [other_min, other_max] = grid.boxes[candidate];
            if (0..2).all(|axis| {
                max[axis].min(other_max[axis]) - min[axis].max(other_min[axis]) > ROUNDING
            }) {
                polygon_area(&clipped(triangle, &theirs[candidate]))
            } else {
                0.0
            }
        };
        let span = grid.span(min, max);
        // Through the cells one of theirs costs more than passed in order.
        let over: f64 = if grid.entries_in(span).saturating_mul(2) > theirs.len() {
            looked += theirs.len();
            (0..theirs.len()).map(clip).sum()
        } else {
            let mut over = 0.0;
            for candidate in grid.near(span) {
                looked += 1;
                if std::mem::replace(&mut seen[candidate], index) != index {
                    over += clip(candidate);
                }
            }
            over
        };
        total += over.min(polygon_area(triangle));
    }
    Some((total, looked))
}

/// The first and last column, and row, of cells.
type Span = ([usize; 2], [usize; 2]);

/// The triangles of a level by the cells of a uniform grid their boxes touch.
struct Grid {
    /// Where each column, and each row, begins.
    starts: [Vec<f64>; 2],
    cells: Vec<Vec<usize>>,
    /// How many triangles the cells hold before each, a column and a row
    /// more than there are cells, so the cells near one count at once.
    before: Vec<usize>,
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
                before: Vec::new(),
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
        grid.before = vec![0; (columns + 1) * (rows + 1)];
        for row in 0..rows {
            for column in 0..columns {
                grid.before[(row + 1) * (columns + 1) + column + 1] =
                    grid.cells[row * columns + column].len()
                        + grid.before[row * (columns + 1) + column + 1]
                        + grid.before[(row + 1) * (columns + 1) + column]
                        - grid.before[row * (columns + 1) + column];
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

    /// How many triangles the cells of `span` hold together.
    fn entries_in(&self, ([first_column, last_column], [first_row, last_row]): Span) -> usize {
        let stride = self.starts[0].len() + 1;
        let at = |row: usize, column: usize| self.before[row * stride + column];
        at(last_row + 1, last_column + 1) + at(first_row, first_column)
            - at(first_row, last_column + 1)
            - at(last_row + 1, first_column)
    }

    /// The triangles in the cells of `span`, one in several cells as often
    /// as it is in them.
    fn near(&self, span: Span) -> impl Iterator<Item = usize> + '_ {
        let ([first_column, last_column], [first_row, last_row]) = span;
        (first_row..=last_row).flat_map(move |row| {
            (first_column..=last_column).flat_map(move |column| {
                self.cells[row * self.starts[0].len() + column]
                    .iter()
                    .copied()
            })
        })
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
    triangle.iter().fold(
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
fn clipped(triangle: &Triangle, within: &Triangle) -> Vec<[f64; 2]> {
    let mut points = triangle.to_vec();
    for index in 0..3 {
        let (from, to) = (within[index], within[(index + 1) % 3]);
        let edge = [to[0] - from[0], to[1] - from[1]];
        let side = |point: [f64; 2]| cross2(edge, [point[0] - from[0], point[1] - from[1]]);
        let mut kept = Vec::with_capacity(points.len() + 1);
        for (position, point) in points.iter().enumerate() {
            let next = points[(position + 1) % points.len()];
            let (here, there) = (side(*point), side(next));
            if here >= 0.0 {
                kept.push(*point);
            }
            if (here >= 0.0) != (there >= 0.0) {
                let t = here / (here - there);
                kept.push([
                    point[0] + t * (next[0] - point[0]),
                    point[1] + t * (next[1] - point[1]),
                ]);
            }
        }
        points = kept;
        if points.is_empty() {
            break;
        }
    }
    points
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
        assert!(overlap(&slab, &slab, &AtomicBool::new(false)).is_some());
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

        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                stop.store(true, Ordering::Relaxed);
            });
            let started = std::time::Instant::now();
            assert!(overlap(&slab, &slab, &stop).is_none());
            let took = started.elapsed();
            assert!(took < std::time::Duration::from_millis(300), "{took:?}");
        });
    }
}
