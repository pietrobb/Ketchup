//! The visible lines of a view, for a drawing file that cannot paint faces
//! over lines (DXF, DWG). The PDF paints the view far to near: faces hide the
//! edges drawn before them and the cut fills hide everything below the cut.
//! Here each edge keeps exactly the pieces no later face or cut fill covers,
//! so the DXF shows what the PDF shows. An edge on the boundary of a nearer
//! face stays visible, as its stroke does on paper; collinear pieces are
//! joined into one line.

use super::{DrawnView, EDGE_STROKE_MM, Item};
use ketchup_geometry::linalg::{cross2, dot2};
use ketchup_model::tolerance::HIDDEN_LINE_TOUCH_MM;
use std::collections::BTreeMap;

/// One visible line of the view in view millimetres, with its pen width on paper.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct VisibleLine {
    pub(super) points: Vec<[f64; 2]>,
    pub(super) width_mm: f64,
}

/// Something painted over what was drawn before it: a face or a cut fill.
struct Occluder {
    order: usize,
    loops: Vec<Vec<[f64; 2]>>,
    min: [f64; 2],
    max: [f64; 2],
}

impl Occluder {
    fn new(order: usize, loops: Vec<Vec<[f64; 2]>>) -> Self {
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        for point in loops.iter().flatten() {
            for axis in 0..2 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        Self {
            order,
            loops,
            min,
            max,
        }
    }

    fn sides(&self) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
        self.loops
            .iter()
            .filter(|points| points.len() >= 2)
            .flat_map(|points| {
                (0..points.len())
                    .map(move |index| (points[index], points[(index + 1) % points.len()]))
            })
    }

    /// Whether `point` lies inside (even-odd) and clear of every side.
    fn covers(&self, point: [f64; 2]) -> bool {
        let mut inside = false;
        for (a, b) in self.sides() {
            if distance_to_side(point, a, b) <= HIDDEN_LINE_TOUCH_MM {
                return false;
            }
            if (a[1] > point[1]) != (b[1] > point[1]) {
                let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if point[0] < x {
                    inside = !inside;
                }
            }
        }
        inside
    }

    /// The parts of `a`–`b`, as parameter intervals, this covers.
    fn covered(&self, a: [f64; 2], b: [f64; 2]) -> Vec<[f64; 2]> {
        let mut cuts = vec![0.0, 1.0];
        for (c, d) in self.sides() {
            if let Some(t) = crossing(a, b, c, d) {
                cuts.push(t);
            }
        }
        cuts.sort_by(f64::total_cmp);
        let length = distance(a, b);
        let mut covered: Vec<[f64; 2]> = Vec::new();
        for pair in cuts.windows(2) {
            if (pair[1] - pair[0]) * length <= HIDDEN_LINE_TOUCH_MM {
                continue;
            }
            let t = f64::midpoint(pair[0], pair[1]);
            if self.covers(lerp(a, b, t)) {
                match covered.last_mut() {
                    Some(last) if (pair[0] - last[1]) * length <= HIDDEN_LINE_TOUCH_MM => {
                        last[1] = pair[1];
                    }
                    _ => covered.push([pair[0], pair[1]]),
                }
            }
        }
        covered
    }
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn distance_to_side(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let side = [b[0] - a[0], b[1] - a[1]];
    let squared = dot2(side, side);
    let t = if squared > 0.0 {
        (dot2([point[0] - a[0], point[1] - a[1]], side) / squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(point, lerp(a, b, t))
}

/// Where along `a`–`b` (0..1) it crosses the side `c`–`d`, if it does.
fn crossing(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<f64> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let denominator = cross2(r, s);
    if denominator == 0.0 {
        return None;
    }
    let q = [c[0] - a[0], c[1] - a[1]];
    let t = cross2(q, s) / denominator;
    let u = cross2(q, r) / denominator;
    ((0.0..=1.0).contains(&u) && t > 0.0 && t < 1.0).then_some(t)
}

/// A count of cells as a coordinate factor; a grid has at most 512 cells a side.
fn factor(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// `value` rounded to a whole number, as a key that sorts as the number does: equal
/// numbers, minus zero among them, give one key.
fn whole_key(value: f64) -> u64 {
    let bits = (value.round() + 0.0).to_bits();
    if bits >> 63 == 1 {
        !bits
    } else {
        bits | 1 << 63
    }
}

/// The occluders by the cells of a regular grid their bounds touch.
struct Grid {
    min: [f64; 2],
    cell: [f64; 2],
    size: usize,
    cells: Vec<Vec<usize>>,
}

impl Grid {
    fn new(min: [f64; 2], max: [f64; 2], occluders: &[Occluder]) -> Self {
        // not a tolerance: about four occluders a cell, at most 512 cells a side.
        let mut size = 1;
        while size < 512 && size * size * 4 < occluders.len() {
            size += 1;
        }
        let cell = std::array::from_fn(|axis| ((max[axis] - min[axis]) / factor(size)).max(1.0));
        let mut grid = Self {
            min,
            cell,
            size,
            cells: vec![Vec::new(); size * size],
        };
        for (index, occluder) in occluders.iter().enumerate() {
            let [low_x, low_y] = grid.cell_of(occluder.min);
            let [high_x, high_y] = grid.cell_of(occluder.max);
            for y in low_y..=high_y {
                for x in low_x..=high_x {
                    grid.cells[y * size + x].push(index);
                }
            }
        }
        grid
    }

    /// The column and row of `point`, clamped to the grid: the last whose low edge
    /// is not beyond it, found by halving.
    fn cell_of(&self, point: [f64; 2]) -> [usize; 2] {
        std::array::from_fn(|axis| {
            let (mut low, mut high) = (0, self.size - 1);
            while low < high {
                let middle = (low + high).div_ceil(2);
                if self.min[axis] + factor(middle) * self.cell[axis] <= point[axis] {
                    low = middle;
                } else {
                    high = middle - 1;
                }
            }
            low
        })
    }

    /// The cells `a`–`b` passes through, walked cell by cell.
    fn cells_along(&self, a: [f64; 2], b: [f64; 2]) -> Vec<usize> {
        let [mut x, mut y] = self.cell_of(a);
        let end = self.cell_of(b);
        let mut cells = vec![y * self.size + x];
        let direction = [b[0] - a[0], b[1] - a[1]];
        let step = direction.map(|value| if value > 0.0 { 1_i64 } else { -1 });
        let boundary = |axis: usize, index: usize| {
            let index = factor(index) + if direction[axis] > 0.0 { 1.0 } else { 0.0 };
            self.min[axis] + index * self.cell[axis]
        };
        let next_t = |axis: usize, index: usize| {
            if direction[axis] == 0.0 {
                f64::INFINITY
            } else {
                (boundary(axis, index) - a[axis]) / direction[axis]
            }
        };
        let (mut t_x, mut t_y) = (next_t(0, x), next_t(1, y));
        while [x, y] != end && cells.len() <= 2 * self.size + 2 {
            let moved = if t_x < t_y {
                x = x.saturating_add_signed(isize::try_from(step[0]).unwrap_or(0));
                t_x = next_t(0, x);
                0
            } else {
                y = y.saturating_add_signed(isize::try_from(step[1]).unwrap_or(0));
                t_y = next_t(1, y);
                1
            };
            if moved == 0 && x >= self.size || moved == 1 && y >= self.size {
                break;
            }
            cells.push(y * self.size + x);
        }
        cells
    }
}

/// One piece of a line to keep: its ends and pen width.
struct Piece {
    a: [f64; 2],
    b: [f64; 2],
    width_mm: f64,
}

/// The visible lines of `view`.
pub(super) fn visible_lines(view: &DrawnView) -> Vec<VisibleLine> {
    let mut occluders = Vec::new();
    let mut segments = Vec::new();
    for (order, item) in view.items.iter().enumerate() {
        match item {
            Item::Face { points, .. } => occluders.push(Occluder::new(order, vec![points.clone()])),
            Item::Edge([a, b]) => segments.push((order, *a, *b, EDGE_STROKE_MM)),
            Item::Thread { points, width_mm } => segments.extend(
                points
                    .windows(2)
                    .map(|pair| (order, pair[0], pair[1], *width_mm)),
            ),
        }
    }
    let cut_order = view.items.len();
    for (loops, _) in &view.cuts {
        occluders.push(Occluder::new(cut_order, loops.clone()));
    }
    for item in &view.on_cut {
        if let Item::Thread { points, width_mm } = item {
            segments.extend(
                points
                    .windows(2)
                    .map(|pair| (cut_order + 1, pair[0], pair[1], *width_mm)),
            );
        }
    }
    let grid = Grid::new(view.min, view.max, &occluders);
    let mut seen = vec![usize::MAX; occluders.len()];
    let mut pieces = Vec::new();
    for (index, (order, a, b, width_mm)) in segments.into_iter().enumerate() {
        let (low, high) = (
            [a[0].min(b[0]), a[1].min(b[1])],
            [a[0].max(b[0]), a[1].max(b[1])],
        );
        let mut hidden: Vec<[f64; 2]> = Vec::new();
        for cell in grid.cells_along(a, b) {
            for &candidate in &grid.cells[cell] {
                if seen[candidate] == index {
                    continue;
                }
                seen[candidate] = index;
                let occluder = &occluders[candidate];
                if occluder.order <= order
                    || occluder.max[0] < low[0]
                    || occluder.max[1] < low[1]
                    || occluder.min[0] > high[0]
                    || occluder.min[1] > high[1]
                {
                    continue;
                }
                hidden.extend(occluder.covered(a, b));
            }
        }
        hidden.sort_by(|left, right| left[0].total_cmp(&right[0]));
        let mut start = 0.0;
        for [from, to] in hidden.into_iter().chain([[1.0, 1.0]]) {
            if from > start {
                pieces.push(Piece {
                    a: lerp(a, b, start),
                    b: lerp(a, b, from),
                    width_mm,
                });
            }
            start = f64::max(start, to);
        }
    }
    joined(pieces)
}

/// The pieces, collinear ones that touch or overlap joined into one line, and
/// pieces too short to see on any sheet left out.
fn joined(pieces: Vec<Piece>) -> Vec<VisibleLine> {
    // Pieces on one line: its direction (made unique), its distance from the
    // origin and the pen; along it, where each piece starts and ends.
    let mut lines: BTreeMap<(u64, u64, u64), Vec<(f64, f64)>> = BTreeMap::new();
    let mut directions = BTreeMap::new();
    for Piece { a, b, width_mm } in pieces {
        let length = distance(a, b);
        // not a tolerance: a line shorter than half a model millimetre draws nothing visible.
        if length < 0.5 {
            continue;
        }
        let mut direction = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
        if direction[0] < 0.0 || (direction[0] == 0.0 && direction[1] < 0.0) {
            direction = direction.map(|value| -value);
        }
        let offset = direction[0] * a[1] - direction[1] * a[0];
        // not a tolerance: directions are told apart to a millionth of a radian.
        let key = (
            whole_key(direction[1].atan2(direction[0]) * 1_000_000.0),
            whole_key(offset / HIDDEN_LINE_TOUCH_MM),
            whole_key(width_mm / HIDDEN_LINE_TOUCH_MM),
        );
        let along = |point: [f64; 2]| dot2(direction, point);
        let (start, end) = (along(a).min(along(b)), along(a).max(along(b)));
        lines.entry(key).or_default().push((start, end));
        directions
            .entry(key)
            .or_insert((direction, offset, width_mm));
    }
    let mut out = Vec::new();
    for (key, mut spans) in lines {
        let (direction, offset, width_mm) = directions[&key];
        let point = |along: f64| {
            [
                direction[0] * along - direction[1] * offset,
                direction[1] * along + direction[0] * offset,
            ]
        };
        spans.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut current = spans[0];
        for span in spans.into_iter().skip(1) {
            if span.0 <= current.1 + HIDDEN_LINE_TOUCH_MM {
                current.1 = current.1.max(span.1);
            } else {
                out.push(VisibleLine {
                    points: vec![point(current.0), point(current.1)],
                    width_mm,
                });
                current = span;
            }
        }
        out.push(VisibleLine {
            points: vec![point(current.0), point(current.1)],
            width_mm,
        });
    }
    out
}
