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

/// How many triangles are overlapped between two looks at `stop`.
const TRIANGLES_BETWEEN_STOPS: usize = 256;

/// The area of `mine` over `theirs`, both counter-clockwise; each of mine
/// counts at most its own area, however many of theirs lie on one another
/// (review 2026-11-07 P3: a floor laid twice, two layers of one top, counted
/// twice). None once `stop` is set.
pub(super) fn area_over(mine: &[Triangle], theirs: &[Triangle], stop: &AtomicBool) -> Option<f64> {
    let grid = Grid::of(theirs);
    // The triangle of mine each of theirs was last overlapped with, so one
    // in several cells is clipped once.
    let mut seen = vec![usize::MAX; theirs.len()];
    let mut total = 0.0;
    for (index, triangle) in mine.iter().enumerate() {
        if index % TRIANGLES_BETWEEN_STOPS == 0 && stop.load(Ordering::Relaxed) {
            return None;
        }
        let [min, max] = boxed(triangle);
        let mut over = 0.0;
        for candidate in grid.near(min, max) {
            if std::mem::replace(&mut seen[candidate], index) == index {
                continue;
            }
            let [other_min, other_max] = grid.boxes[candidate];
            if (0..2).all(|axis| {
                max[axis].min(other_max[axis]) - min[axis].max(other_min[axis]) > ROUNDING
            }) {
                over += polygon_area(&clipped(triangle, &theirs[candidate]));
            }
        }
        total += over.min(polygon_area(triangle));
    }
    Some(total)
}

/// The triangles of a level by the cells of a uniform grid their boxes touch.
struct Grid {
    /// Where each column, and each row, begins.
    starts: [Vec<f64>; 2],
    cells: Vec<Vec<usize>>,
    boxes: Vec<[[f64; 2]; 2]>,
}

impl Grid {
    /// About one cell per triangle, at most [`MOST_CELLS_ACROSS`] a side.
    fn of(triangles: &[Triangle]) -> Self {
        let boxes: Vec<[[f64; 2]; 2]> = triangles.iter().map(boxed).collect();
        let [min, max] = boxes.iter().fold(
            [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
            |[min, max], [low, high]| {
                [
                    [min[0].min(low[0]), min[1].min(low[1])],
                    [max[0].max(high[0]), max[1].max(high[1])],
                ]
            },
        );
        let size = [(max[0] - min[0]).max(0.0), (max[1] - min[1]).max(0.0)];
        let count = f64::from(u32::try_from(triangles.len()).unwrap_or(u32::MAX).max(1));
        let side = (size[0] * size[1] / count).sqrt();
        let starts = [0, 1].map(|axis| {
            let across = if side > ROUNDING && size[axis].is_finite() {
                (1..MOST_CELLS_ACROSS)
                    .find(|cells| f64::from(*cells) * side >= size[axis])
                    .unwrap_or(MOST_CELLS_ACROSS)
            } else {
                1
            };
            let origin = if min[axis].is_finite() {
                min[axis]
            } else {
                0.0
            };
            let cell = size[axis] / f64::from(across);
            (0..across)
                .map(|column| origin + f64::from(column) * cell)
                .collect::<Vec<f64>>()
        });
        let columns = starts[0].len();
        let mut grid = Self {
            cells: vec![Vec::new(); columns * starts[1].len()],
            starts,
            boxes: Vec::new(),
        };
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

    /// The first and last column, and row, of the cells from `min` to `max`.
    fn span(&self, min: [f64; 2], max: [f64; 2]) -> ([usize; 2], [usize; 2]) {
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

    /// The triangles in the cells from `min` to `max`, one in several cells
    /// as often as it is in them.
    fn near(&self, min: [f64; 2], max: [f64; 2]) -> impl Iterator<Item = usize> + '_ {
        let ([first_column, last_column], [first_row, last_row]) = self.span(min, max);
        (first_row..=last_row).flat_map(move |row| {
            (first_column..=last_column).flat_map(move |column| {
                self.cells[row * self.starts[0].len() + column]
                    .iter()
                    .copied()
            })
        })
    }
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
