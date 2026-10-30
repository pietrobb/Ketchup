//! Closed regions of an underlay's lines: the areas that straight pieces enclose, found
//! the way a person finds them on paper. Pieces are split where they cross or where one
//! ends on another (within a tolerance), pieces with a loose end are left out, and each
//! face of what remains is a region with its outline, the holes other closed figures cut
//! in it, and its area. Nothing is assumed of what the lines draw: a room, a parcel and a
//! cut-out are found alike.

use std::collections::HashMap;

use ketchup_geometry::linalg::{cross2, dot2};
use ketchup_tolerance::ROUNDING;

/// The most pieces one search takes: past it the caller narrows the layers or the box.
pub const MAX_REGION_SEGMENTS: usize = 200_000;
/// How near line ends join unless a search names its own distance, in document mm.
pub const DEFAULT_JOIN_MM: f64 = ketchup_tolerance::UNDERLAY_JOIN_MM;
/// The farthest join distance a search takes: past it, distinct lines merge.
pub const MAX_JOIN_MM: f64 = 1_000.0;
/// The most crossings one search splits pieces at.
pub const MAX_REGION_CROSSINGS: usize = 2_000_000;

/// One closed region: an outline counterclockwise and holes clockwise, in the
/// millimetres of the pieces it was found in.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub outline: Vec<[f64; 2]>,
    pub holes: Vec<Vec<[f64; 2]>>,
    /// Inside the outline and outside its holes.
    pub area_mm2: f64,
    pub bounds: [[f64; 2]; 2],
}

impl Region {
    /// Whether `point` lies inside it: inside its outline and in none of its holes.
    #[must_use]
    pub fn contains(&self, point: [f64; 2]) -> bool {
        in_box(self.bounds, point)
            && super::objects::ring_contains(&self.outline, point)
            && !self
                .holes
                .iter()
                .any(|hole| super::objects::ring_contains(hole, point))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionError {
    TooManySegments { count: usize, limit: usize },
    TooManyCrossings { limit: usize },
}

impl std::fmt::Display for RegionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManySegments { count, limit } => write!(
                formatter,
                "{count} straight pieces are more than the {limit} one search takes"
            ),
            Self::TooManyCrossings { limit } => {
                write!(formatter, "the pieces cross more than {limit} times")
            }
        }
    }
}

impl std::error::Error for RegionError {}

/// The regions the straight pieces `segments` enclose, ends and crossings closer than
/// `tolerance_mm` taken as one point, largest first.
///
/// # Errors
/// More pieces than [`MAX_REGION_SEGMENTS`] or crossings than [`MAX_REGION_CROSSINGS`].
pub fn find_regions(
    segments: &[[[f64; 2]; 2]],
    tolerance_mm: f64,
) -> Result<Vec<Region>, RegionError> {
    let (points, edges) = plane_graph(segments, tolerance_mm)?;
    Ok(faces(&points, &edges))
}

/// The closed figures the straight pieces `segments` draw, each one region however many
/// faces the lines inside it split it into: its outline the figure's outer edge, its holes
/// the outer edges of the figures directly inside it, largest first. A view of a part,
/// its silhouette, with the edges seen within it ignored.
///
/// # Errors
/// As [`find_regions`].
pub fn find_figures(
    segments: &[[[f64; 2]; 2]],
    tolerance_mm: f64,
) -> Result<Vec<Region>, RegionError> {
    let (points, edges) = plane_graph(segments, tolerance_mm)?;
    let (_, outer) = walks(&points, &edges);
    let mut figures = outer
        .iter()
        .map(|(ring, area, _)| {
            let mut outline = ring.clone();
            outline.reverse();
            Region {
                bounds: bounds_of(ring),
                outline,
                holes: Vec::new(),
                area_mm2: *area,
            }
        })
        .collect::<Vec<_>>();
    let bounds = figures
        .iter()
        .map(|figure| figure.bounds)
        .collect::<Vec<_>>();
    let index = BoxIndex::new(&bounds);
    for (ring, area, owner) in &outer {
        let around = smallest_around(&outer, &bounds, &index, ring[0], *owner);
        if let Some(index) = around {
            // Turned as `faces` turns a hole.
            let mut hole = ring.clone();
            hole.reverse();
            figures[index].holes.push(hole);
            figures[index].area_mm2 -= area;
        }
    }
    figures.sort_by(|a, b| b.area_mm2.total_cmp(&a.area_mm2));
    Ok(figures)
}

/// The vertices and edges `segments` make once split where they meet, ends within the
/// tolerance one vertex, and every edge with a loose end taken away.
type PlaneGraph = (Vec<[f64; 2]>, Vec<[usize; 2]>);

fn plane_graph(segments: &[[[f64; 2]; 2]], tolerance_mm: f64) -> Result<PlaneGraph, RegionError> {
    if segments.len() > MAX_REGION_SEGMENTS {
        return Err(RegionError::TooManySegments {
            count: segments.len(),
            limit: MAX_REGION_SEGMENTS,
        });
    }
    let tolerance = tolerance_mm.max(ketchup_tolerance::APPROXIMATION);
    let segments = segments
        .iter()
        .copied()
        .filter(|[a, b]| (b[0] - a[0]).hypot(b[1] - a[1]) > tolerance)
        .collect::<Vec<_>>();
    let cuts = cuts(&segments, tolerance)?;
    let mut vertices = Vertices::new(tolerance);
    let mut edges = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (segment, mut at) in segments.iter().zip(cuts) {
        at.push(0.0);
        at.push(1.0);
        at.sort_by(f64::total_cmp);
        let ids = at
            .iter()
            .map(|t| vertices.id(lerp(segment[0], segment[1], *t)))
            .collect::<Vec<_>>();
        for pair in ids.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a != b && seen.insert((a.min(b), a.max(b))) {
                edges.push([a, b]);
            }
        }
    }
    let points = vertices.points;
    let edges = without_loose_ends(points.len(), edges);
    Ok((points, edges))
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn in_box(bounds: [[f64; 2]; 2], point: [f64; 2]) -> bool {
    point[0] >= bounds[0][0]
        && point[0] <= bounds[1][0]
        && point[1] >= bounds[0][1]
        && point[1] <= bounds[1][1]
}

/// Points closer than the tolerance merged into one vertex.
struct Vertices {
    cell: f64,
    points: Vec<[f64; 2]>,
    grid: HashMap<Cell, Vec<usize>>,
}

impl Vertices {
    fn new(tolerance: f64) -> Self {
        Self {
            cell: tolerance,
            points: Vec::new(),
            grid: HashMap::new(),
        }
    }

    fn id(&mut self, point: [f64; 2]) -> usize {
        let [x, y] = point.map(|value| cell_of(value, self.cell));
        for dx in [-1.0, 0.0, 1.0] {
            for dy in [-1.0, 0.0, 1.0] {
                for id in self.grid.get(&cell(x + dx, y + dy)).into_iter().flatten() {
                    let other = self.points[*id];
                    if (other[0] - point[0]).hypot(other[1] - point[1]) <= self.cell {
                        return *id;
                    }
                }
            }
        }
        self.points.push(point);
        let id = self.points.len() - 1;
        self.grid.entry(cell(x, y)).or_default().push(id);
        id
    }
}

/// A grid cell by the bits of its whole-number column and row.
type Cell = [u64; 2];

/// The whole number of cells of size `cell` below `value`, as a float.
fn cell_of(value: f64, cell: f64) -> f64 {
    // `+ 0.0` makes a -0.0 the 0.0 cell.
    (value / cell).floor() + 0.0
}

fn cell(column: f64, row: f64) -> Cell {
    [(column + 0.0).to_bits(), (row + 0.0).to_bits()]
}

/// Every cell from `low` to `high`, columns and rows whole numbers.
fn cells(low: [f64; 2], high: [f64; 2], mut found: impl FnMut(Cell)) {
    let mut column = low[0];
    while column <= high[0] {
        let mut row = low[1];
        while row <= high[1] {
            found(cell(column, row));
            row += 1.0;
        }
        column += 1.0;
    }
}

/// How many cells [`cells`] gives from `low` to `high`, saturating.
fn box_cells(low: [f64; 2], high: [f64; 2]) -> usize {
    let side = |axis: usize| {
        let mut count = 0usize;
        let mut at = low[axis];
        while at <= high[axis] && count <= MAX_REGION_CROSSINGS {
            count += 1;
            at += 1.0;
        }
        count
    };
    side(0).saturating_mul(side(1))
}

fn count_f64(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// How many cells a piece's box may cover before it is looked for along its way instead.
const LONG_PIECE_CELLS: usize = 64;

const TOO_MANY_CROSSINGS: RegionError = RegionError::TooManyCrossings {
    limit: MAX_REGION_CROSSINGS,
};

/// The side of the square holding as many of `centers` as it may hold of them, the
/// farthest fiftieth on each side left out: a stray line kilometres from the plan does
/// not make its cells so large that the whole plan falls in one (review 2026-10-26 UO-40).
fn robust_span(mut centers: [Vec<f64>; 2]) -> f64 {
    let mut span = 0.0_f64;
    for values in &mut centers {
        let last = values.len() - 1;
        let (low, high) = (last / 50, last - last / 50);
        let high_value = *values.select_nth_unstable_by(high, f64::total_cmp).1;
        let low_value = *values.select_nth_unstable_by(low, f64::total_cmp).1;
        span = span.max(high_value - low_value);
    }
    span
}

fn overlap(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> bool {
    a[0][0].max(b[0][0]) <= a[1][0].min(b[1][0]) && a[0][1].max(b[0][1]) <= a[1][1].min(b[1][1])
}

/// The cells of points along `segment` half a cell apart, each once in a row.
fn path_cells(segment: [[f64; 2]; 2], size: f64, steps: usize) -> Vec<[f64; 2]> {
    let mut path: Vec<[f64; 2]> = Vec::with_capacity(steps + 1);
    for step in 0..=steps {
        let at = lerp(segment[0], segment[1], count_f64(step) / count_f64(steps));
        let here = at.map(|value| cell_of(value, size));
        if path.last() != Some(&here) {
            path.push(here);
        }
    }
    path
}

/// For each segment, the parameters along it where another crosses or touches it.
fn cuts(segments: &[[[f64; 2]; 2]], tolerance: f64) -> Result<Vec<Vec<f64>>, RegionError> {
    let mut cuts = vec![Vec::new(); segments.len()];
    if segments.len() < 2 {
        return Ok(cuts);
    }
    let boxes = segments
        .iter()
        .map(|[a, b]| {
            [
                [a[0].min(b[0]) - tolerance, a[1].min(b[1]) - tolerance],
                [a[0].max(b[0]) + tolerance, a[1].max(b[1]) + tolerance],
            ]
        })
        .collect::<Vec<_>>();
    let centers = [0, 1].map(|axis| {
        (boxes.iter())
            .map(|[low, high]| f64::midpoint(low[axis], high[axis]))
            .collect()
    });
    let per_side = count_f64(segments.len()).sqrt().max(1.0);
    let size = (robust_span(centers) / per_side).max(tolerance * 4.0);
    let mut grid: HashMap<Cell, Vec<usize>> = HashMap::new();
    let mut long = Vec::new();
    let mut registered = 0usize;
    for (index, [low, high]) in boxes.iter().enumerate() {
        let (from, to) = (
            low.map(|v| cell_of(v, size)),
            high.map(|v| cell_of(v, size)),
        );
        let covered = box_cells(from, to);
        if covered > LONG_PIECE_CELLS {
            long.push(index);
            continue;
        }
        registered += covered;
        if registered > MAX_REGION_CROSSINGS {
            return Err(TOO_MANY_CROSSINGS);
        }
        cells(from, to, |key| grid.entry(key).or_default().push(index));
    }
    let mut crossings = 0;
    let mut cut = |i: usize, j: usize, cuts: &mut [Vec<f64>]| {
        for (t, u) in meet(segments[i], segments[j], tolerance) {
            cuts[i].push(t);
            cuts[j].push(u);
            crossings += 1;
        }
        if crossings > MAX_REGION_CROSSINGS {
            return Err(TOO_MANY_CROSSINGS);
        }
        Ok(())
    };
    let mut keys = grid.keys().copied().collect::<Vec<_>>();
    keys.sort_unstable();
    for key in keys {
        let members = &grid[&key];
        for (position, &i) in members.iter().enumerate() {
            for &j in &members[position + 1..] {
                let (a, b) = (boxes[i], boxes[j]);
                if !overlap(a, b) {
                    continue;
                }
                // Each pair once: in the cell holding the low corner of their overlap.
                let overlap_low = [a[0][0].max(b[0][0]), a[0][1].max(b[0][1])];
                if cell(cell_of(overlap_low[0], size), cell_of(overlap_low[1], size)) == key {
                    cut(i, j, &mut cuts)?;
                }
            }
        }
    }
    // A long piece is looked for along its way, in the cells of points half a cell apart
    // on it and their neighbours: where it meets another piece, within the tolerance of a
    // quarter cell, a point of it lies in a cell next to one the other is in.
    let mut paths = Vec::with_capacity(long.len());
    let mut long_grid: HashMap<Cell, Vec<usize>> = HashMap::new();
    for &index in &long {
        let [a, b] = segments[index];
        let steps = ((a[0] - b[0]).hypot(a[1] - b[1]) / (size / 2.0)).ceil();
        let steps = if steps < count_f64(MAX_REGION_CROSSINGS) {
            // A whole number below the limit.
            steps as usize
        } else {
            MAX_REGION_CROSSINGS
        };
        registered += steps;
        if registered > MAX_REGION_CROSSINGS {
            return Err(TOO_MANY_CROSSINGS);
        }
        let path = path_cells(segments[index], size, steps.max(1));
        for [column, row] in &path {
            long_grid
                .entry(cell(*column, *row))
                .or_default()
                .push(index);
        }
        paths.push(path);
    }
    for (&i, path) in long.iter().zip(&paths) {
        let mut near = Vec::new();
        for [column, row] in path {
            for dx in [-1.0, 0.0, 1.0] {
                for dy in [-1.0, 0.0, 1.0] {
                    let key = cell(column + dx, row + dy);
                    near.extend(grid.get(&key).into_iter().flatten());
                    near.extend((long_grid.get(&key).into_iter().flatten()).filter(|j| **j > i));
                }
            }
        }
        near.sort_unstable();
        near.dedup();
        for j in near {
            if overlap(boxes[i], boxes[j]) {
                cut(i, j, &mut cuts)?;
            }
        }
    }
    Ok(cuts)
}

/// Boxes found by a point inside them: each in the cells it covers, or, covering more than
/// [`LONG_PIECE_CELLS`] cells as large as most boxes, in a list looked through for every
/// point.
struct BoxIndex {
    size: f64,
    grid: HashMap<Cell, Vec<usize>>,
    large: Vec<usize>,
}

impl BoxIndex {
    fn new(boxes: &[[[f64; 2]; 2]]) -> Self {
        let mut sides = (boxes.iter())
            .map(|[low, high]| (high[0] - low[0]).max(high[1] - low[1]))
            .collect::<Vec<_>>();
        let size = if sides.is_empty() {
            1.0
        } else {
            let middle = sides.len() / 2;
            (*sides.select_nth_unstable_by(middle, f64::total_cmp).1).max(ROUNDING)
        };
        let mut index = Self {
            size,
            grid: HashMap::new(),
            large: Vec::new(),
        };
        for (at, [low, high]) in boxes.iter().enumerate() {
            let (from, to) = (
                low.map(|v| cell_of(v, size)),
                high.map(|v| cell_of(v, size)),
            );
            if box_cells(from, to) > LONG_PIECE_CELLS {
                index.large.push(at);
            } else {
                cells(from, to, |key| index.grid.entry(key).or_default().push(at));
            }
        }
        index
    }

    /// The boxes that may hold `point`, in no order.
    fn around(&self, point: [f64; 2]) -> impl Iterator<Item = usize> + '_ {
        let key = cell(cell_of(point[0], self.size), cell_of(point[1], self.size));
        (self.grid.get(&key).into_iter().flatten())
            .chain(&self.large)
            .copied()
    }
}

/// The smallest of `rings` around `probe` of another figure than `owner`: of those of
/// equal area, the first.
fn smallest_around(
    rings: &[Walk],
    bounds: &[[[f64; 2]; 2]],
    index: &BoxIndex,
    probe: [f64; 2],
    owner: usize,
) -> Option<usize> {
    index
        .around(probe)
        .filter(|at| {
            let (outline, _, figure) = &rings[*at];
            *figure != owner
                && in_box(bounds[*at], probe)
                && super::objects::ring_contains(outline, probe)
        })
        .min_by(|a, b| rings[*a].1.total_cmp(&rings[*b].1).then(a.cmp(b)))
}

/// Where two segments meet, as parameters along each: a crossing, an end on the other,
/// or the ends of their overlap when they run along each other.
fn meet(p: [[f64; 2]; 2], q: [[f64; 2]; 2], tolerance: f64) -> Vec<(f64, f64)> {
    let r = [p[1][0] - p[0][0], p[1][1] - p[0][1]];
    let s = [q[1][0] - q[0][0], q[1][1] - q[0][1]];
    let (r_len, s_len) = (r[0].hypot(r[1]), s[0].hypot(s[1]));
    let along = |point: [f64; 2], start: [f64; 2], dir: [f64; 2], len: f64| {
        let t = dot2([point[0] - start[0], point[1] - start[1]], dir) / (len * len);
        let foot = [start[0] + dir[0] * t, start[1] + dir[1] * t];
        (t, (point[0] - foot[0]).hypot(point[1] - foot[1]))
    };
    let within = |t: f64, len: f64| t >= -tolerance / len && t <= 1.0 + tolerance / len;
    let cross = cross2(r, s);
    let mut found = Vec::new();
    if cross.abs() > ketchup_tolerance::NEGLIGIBLE * r_len * s_len {
        let d = [q[0][0] - p[0][0], q[0][1] - p[0][1]];
        let t = cross2(d, s) / cross;
        let u = cross2(d, r) / cross;
        if within(t, r_len) && within(u, s_len) {
            found.push((t.clamp(0.0, 1.0), u.clamp(0.0, 1.0)));
        }
        return found;
    }
    // Parallel: each end lying on the other splits it there.
    for (end, u) in [(q[0], 0.0), (q[1], 1.0)] {
        let (t, off) = along(end, p[0], r, r_len);
        if off <= tolerance && within(t, r_len) {
            found.push((t.clamp(0.0, 1.0), u));
        }
    }
    for (end, t) in [(p[0], 0.0), (p[1], 1.0)] {
        let (u, off) = along(end, q[0], s, s_len);
        if off <= tolerance && within(u, s_len) {
            found.push((t, u.clamp(0.0, 1.0)));
        }
    }
    found
}

/// The edges left once every edge with an end no other edge meets is taken away, again
/// until none is left.
fn without_loose_ends(vertex_count: usize, edges: Vec<[usize; 2]>) -> Vec<[usize; 2]> {
    let mut degree = vec![0usize; vertex_count];
    for [a, b] in &edges {
        degree[*a] += 1;
        degree[*b] += 1;
    }
    let mut touching = vec![Vec::new(); vertex_count];
    for (index, [a, b]) in edges.iter().enumerate() {
        touching[*a].push(index);
        touching[*b].push(index);
    }
    let mut alive = vec![true; edges.len()];
    let mut loose = (0..vertex_count)
        .filter(|vertex| degree[*vertex] == 1)
        .collect::<Vec<_>>();
    while let Some(vertex) = loose.pop() {
        for &edge in &touching[vertex] {
            if !alive[edge] {
                continue;
            }
            alive[edge] = false;
            for end in edges[edge] {
                degree[end] -= 1;
                if degree[end] == 1 {
                    loose.push(end);
                }
            }
        }
    }
    edges
        .into_iter()
        .zip(alive)
        .filter_map(|(edge, alive)| alive.then_some(edge))
        .collect()
}

fn signed_area(ring: &[[f64; 2]]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.0
}

fn bounds_of(ring: &[[f64; 2]]) -> [[f64; 2]; 2] {
    let mut bounds = [ring[0], ring[0]];
    for point in ring {
        bounds[0] = [bounds[0][0].min(point[0]), bounds[0][1].min(point[1])];
        bounds[1] = [bounds[1][0].max(point[0]), bounds[1][1].max(point[1])];
    }
    bounds
}

/// The bounded faces of the plane graph of `edges`, each with the outer boundaries of the
/// other closed figures inside it as holes.
fn faces(points: &[[f64; 2]], edges: &[[usize; 2]]) -> Vec<Region> {
    let (bounded, outer) = walks(points, edges);
    let mut regions = bounded
        .iter()
        .map(|(ring, area, _)| Region {
            bounds: bounds_of(ring),
            outline: ring.clone(),
            holes: Vec::new(),
            area_mm2: *area,
        })
        .collect::<Vec<_>>();
    // Each figure's hole found among the faces around it, not among all of them: a plan of
    // thousands of rooms and pillars took seconds (review 2026-10-26 UO-40).
    let bounds = regions
        .iter()
        .map(|region| region.bounds)
        .collect::<Vec<_>>();
    let index = BoxIndex::new(&bounds);
    for (ring, area, owner) in outer {
        let around = smallest_around(&bounded, &bounds, &index, ring[0], owner);
        if let Some(index) = around {
            let mut hole = ring;
            hole.reverse();
            regions[index].holes.push(hole);
            regions[index].area_mm2 -= area;
        }
    }
    regions.sort_by(|a, b| b.area_mm2.total_cmp(&a.area_mm2));
    regions
}

/// A ring walked around a face, its area and the connected figure it belongs to.
type Walk = (Vec<[f64; 2]>, f64, usize);

/// Every face walk of the plane graph: the bounded faces counterclockwise, and each
/// figure's outer edge, clockwise, with the area it encloses.
fn walks(points: &[[f64; 2]], edges: &[[usize; 2]]) -> (Vec<Walk>, Vec<Walk>) {
    // Half-edges 2k (a→b) and 2k+1 (b→a), each vertex's outgoing ones counterclockwise.
    let mut outgoing = vec![Vec::new(); points.len()];
    for (index, [a, b]) in edges.iter().enumerate() {
        outgoing[*a].push(2 * index);
        outgoing[*b].push(2 * index + 1);
    }
    let ends = |half: usize| {
        let [a, b] = edges[half / 2];
        if half.is_multiple_of(2) {
            (a, b)
        } else {
            (b, a)
        }
    };
    let angle = |half: usize| {
        let (from, to) = ends(half);
        (points[to][1] - points[from][1]).atan2(points[to][0] - points[from][0])
    };
    let mut slot = vec![0; edges.len() * 2];
    for list in &mut outgoing {
        list.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
        for (position, half) in list.iter().enumerate() {
            slot[*half] = position;
        }
    }
    // The connected figure of each vertex, so a figure is never its own hole.
    let mut figure = (0..points.len()).collect::<Vec<_>>();
    fn root(figure: &mut [usize], mut vertex: usize) -> usize {
        while figure[vertex] != vertex {
            figure[vertex] = figure[figure[vertex]];
            vertex = figure[vertex];
        }
        vertex
    }
    for [a, b] in edges {
        let (ra, rb) = (root(&mut figure, *a), root(&mut figure, *b));
        figure[ra] = rb;
    }
    // The walk each half-edge is on; `usize::MAX` for one not walked yet.
    let mut walk_of = vec![usize::MAX; edges.len() * 2];
    let mut rings = Vec::new();
    for start in 0..edges.len() * 2 {
        if walk_of[start] != usize::MAX {
            continue;
        }
        let mut ring = Vec::new();
        let mut half = start;
        while walk_of[half] == usize::MAX {
            walk_of[half] = rings.len();
            let (from, to) = ends(half);
            ring.push(points[from]);
            // Arrived at `to`: the next edge clockwise from the way back keeps the face
            // on the left.
            let back = half ^ 1;
            let list = &outgoing[to];
            half = list[(slot[back] + list.len() - 1) % list.len()];
        }
        rings.push((start, ring));
    }
    // Review 2026-10-26 UO-47: an edge with the same face on both sides, such as a line
    // from a wall to a free-standing pillar, bounds nothing; walked, it would make the
    // room one outline along it twice. Without it the pillar is a figure of its own.
    if (0..edges.len()).any(|edge| walk_of[2 * edge] == walk_of[2 * edge + 1]) {
        let kept = (0..edges.len())
            .filter(|edge| walk_of[2 * edge] != walk_of[2 * edge + 1])
            .map(|edge| edges[edge])
            .collect::<Vec<_>>();
        return walks(points, &kept);
    }
    let mut bounded = Vec::new();
    let mut outer = Vec::new();
    for (start, ring) in rings {
        let area = signed_area(&ring);
        let owner = root(&mut figure, ends(start).0);
        if area > 0.0 {
            bounded.push((ring, area, owner));
        } else if area < 0.0 {
            outer.push((ring, -area, owner));
        }
    }
    (bounded, outer)
}

/// Pieces that run side by side: `a` and `b` index the segments, `gap_mm` is how far
/// apart they lie and `overlap_mm` how long they run alongside each other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParallelPair {
    pub a: usize,
    pub b: usize,
    pub gap_mm: f64,
    pub overlap_mm: f64,
}

/// The pairs of `segments` within `angle_deg` of parallel, `min_gap_mm` to `max_gap_mm`
/// apart and alongside each other for at least `min_overlap_mm`, at most `limit` of them
/// in segment order.
#[must_use]
pub fn parallel_pairs(
    segments: &[[[f64; 2]; 2]],
    min_gap_mm: f64,
    max_gap_mm: f64,
    angle_deg: f64,
    min_overlap_mm: f64,
    limit: usize,
) -> Vec<ParallelPair> {
    let cos_limit = angle_deg.to_radians().cos();
    let reach = max_gap_mm.max(0.0);
    let boxes = segments
        .iter()
        .map(|[a, b]| {
            [
                [a[0].min(b[0]) - reach, a[1].min(b[1]) - reach],
                [a[0].max(b[0]) + reach, a[1].max(b[1]) + reach],
            ]
        })
        .collect::<Vec<_>>();
    let size = (reach * 4.0).max(100.0);
    let mut grid: HashMap<Cell, Vec<usize>> = HashMap::new();
    for (index, [a, b]) in segments.iter().enumerate() {
        let low = [a[0].min(b[0]), a[1].min(b[1])].map(|v| cell_of(v, size));
        let high = [a[0].max(b[0]), a[1].max(b[1])].map(|v| cell_of(v, size));
        cells(low, high, |key| grid.entry(key).or_default().push(index));
    }
    let mut pairs = Vec::new();
    for (i, [a0, a1]) in segments.iter().enumerate() {
        let d = [a1[0] - a0[0], a1[1] - a0[1]];
        let len = d[0].hypot(d[1]);
        if len < min_overlap_mm.max(ROUNDING) {
            continue;
        }
        let unit = [d[0] / len, d[1] / len];
        let [low, high] = boxes[i];
        let (from, to) = (
            low.map(|v| cell_of(v, size)),
            high.map(|v| cell_of(v, size)),
        );
        let mut near = Vec::new();
        cells(from, to, |key| {
            near.extend(
                grid.get(&key)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|j| *j > i),
            );
        });
        near.sort_unstable();
        near.dedup();
        for j in near {
            let [b0, b1] = segments[j];
            let e = [b1[0] - b0[0], b1[1] - b0[1]];
            let e_len = e[0].hypot(e[1]);
            if e_len < ROUNDING || dot2(unit, e).abs() / e_len < cos_limit {
                continue;
            }
            let project = |point: [f64; 2]| {
                let v = [point[0] - a0[0], point[1] - a0[1]];
                (dot2(v, unit), cross2(unit, v))
            };
            let ((s0, n0), (s1, n1)) = (project(b0), project(b1));
            let gap = f64::midpoint(n0, n1).abs();
            if n0.signum() != n1.signum() && n0.abs() > ROUNDING && n1.abs() > ROUNDING {
                continue;
            }
            let overlap = s0.max(s1).min(len) - s0.min(s1).max(0.0);
            if gap >= min_gap_mm && gap <= max_gap_mm && overlap >= min_overlap_mm {
                pairs.push(ParallelPair {
                    a: i,
                    b: j,
                    gap_mm: gap,
                    overlap_mm: overlap,
                });
                if pairs.len() >= limit {
                    return pairs;
                }
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(low: f64, high: f64) -> Vec<[[f64; 2]; 2]> {
        let c = [[low, low], [high, low], [high, high], [low, high]];
        (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect()
    }

    #[test]
    fn two_rooms_split_by_a_wall_ending_on_the_outline_and_an_island() {
        let mut lines = square(0.0, 100.0);
        // A wall from the bottom to the top at x = 40, its ends on the outline.
        lines.push([[40.0, 0.0], [40.0, 100.0]]);
        // A pillar in the right room, and a loose line that encloses nothing.
        lines.extend(square(60.0, 70.0));
        lines.push([[10.0, 10.0], [20.0, 20.0]]);
        let regions = find_regions(&lines, 0.01).unwrap();
        let areas = regions
            .iter()
            .map(|r| r.area_mm2.round())
            .collect::<Vec<_>>();
        assert_eq!(areas, [5_900.0, 4_000.0, 100.0]);
        assert_eq!(regions[0].holes.len(), 1);
        assert!(regions[0].contains([80.0, 50.0]));
        assert!(!regions[0].contains([65.0, 65.0]), "in the pillar");
        assert!(regions[1].contains([15.0, 50.0]));
    }

    /// Review 2026-10-26 UO-47: a line from a wall to a free-standing pillar made the room
    /// one outline walked along that line twice, a slit OCCT builds no solid from. The line
    /// bounds nothing: the room has the pillar as a hole, and every corner once.
    #[test]
    fn a_line_from_a_wall_to_a_pillar_leaves_the_pillar_a_hole_not_a_slit() {
        let mut lines = square(0.0, 100.0);
        lines.extend(square(60.0, 70.0));
        lines.push([[70.0, 65.0], [100.0, 65.0]]);
        for found in [
            find_regions(&lines, 0.01).unwrap(),
            find_figures(&lines, 0.01).unwrap(),
        ] {
            let areas = found.iter().map(|r| r.area_mm2.round()).collect::<Vec<_>>();
            assert_eq!(areas, [9_900.0, 100.0]);
            assert_eq!(found[0].holes.len(), 1, "{:?}", found[0]);
            for ring in std::iter::once(&found[0].outline).chain(&found[0].holes) {
                let mut corners = ring.iter().map(|p| p.map(f64::to_bits)).collect::<Vec<_>>();
                corners.sort_unstable();
                corners.dedup();
                assert_eq!(corners.len(), ring.len(), "a corner walked twice: {ring:?}");
            }
            assert!(found[0].contains([80.0, 50.0]) && !found[0].contains([65.0, 62.0]));
        }
    }

    #[test]
    fn a_figure_is_its_outer_edge_whatever_lines_split_it_inside() {
        let mut lines = square(0.0, 100.0);
        lines.push([[40.0, 0.0], [40.0, 100.0]]);
        lines.extend(square(60.0, 70.0));
        // An island in the pillar: a figure of the pillar's, not of the outline's.
        lines.extend(square(64.0, 66.0));
        lines.push([[10.0, 10.0], [20.0, 20.0]]);
        let figures = find_figures(&lines, 0.01).unwrap();
        let areas = figures
            .iter()
            .map(|r| r.area_mm2.round())
            .collect::<Vec<_>>();
        assert_eq!(areas, [9_900.0, 96.0, 4.0]);
        assert_eq!(figures[0].holes.len(), 1);
        assert!(figures[0].contains([15.0, 50.0]) && figures[0].contains([80.0, 50.0]));
        assert!(!figures[0].contains([62.0, 62.0]), "in the pillar");
        assert!(signed_area(&figures[0].outline) > 0.0);
        assert!(figures[1].contains([62.0, 62.0]) && !figures[1].contains([65.0, 65.0]));
    }

    #[test]
    fn crossing_lines_close_a_region_and_near_misses_join_within_the_tolerance() {
        // A # pattern: the middle square closes where the lines cross.
        let lines = [
            [[0.0, 10.0], [30.0, 10.0]],
            [[0.0, 20.0], [30.0, 20.0]],
            [[10.0, 0.0], [10.0, 30.0]],
            [[20.0, 0.0], [20.0, 30.0]],
        ];
        let regions = find_regions(&lines, 0.01).unwrap();
        assert_eq!(regions.len(), 1);
        assert!((regions[0].area_mm2 - 100.0).abs() < 1e-9);
        // A square whose corner misses by 0.5 mm closes with a 1 mm tolerance only.
        let mut open = square(0.0, 10.0);
        open[3] = [[0.0, 10.0], [0.0, 0.5]];
        assert!(find_regions(&open, 0.1).unwrap().is_empty());
        assert_eq!(find_regions(&open, 1.0).unwrap().len(), 1);
    }

    /// A plan of `columns` × `rows` rooms of 200 × 120 mm, a pillar in each, its walls
    /// drawn room by room, and a line 9 km away, as a DWG often holds.
    fn plan(columns: u32, rows: u32) -> Vec<[[f64; 2]; 2]> {
        let mut lines = vec![[[9.0e6, 9.0e6], [9.0e6 + 1_000.0, 9.0e6]]];
        for column in 0..columns {
            for row in 0..rows {
                let [x, y] = [f64::from(column) * 200.0, f64::from(row) * 120.0];
                lines.push([[x, y], [x + 200.0, y]]);
                lines.push([[x, y], [x, y + 120.0]]);
                if column + 1 == columns {
                    lines.push([[x + 200.0, y], [x + 200.0, y + 120.0]]);
                }
                if row + 1 == rows {
                    lines.push([[x, y + 120.0], [x + 200.0, y + 120.0]]);
                }
                let pillar = [[x + 90.0, y + 50.0], [x + 110.0, y + 70.0]];
                let c = [
                    pillar[0],
                    [pillar[1][0], pillar[0][1]],
                    pillar[1],
                    [pillar[0][0], pillar[1][1]],
                ];
                lines.extend((0..4).map(|i| [c[i], c[(i + 1) % 4]]));
            }
        }
        lines
    }

    #[test]
    fn a_line_far_from_the_plan_leaves_its_rooms_as_they_were() {
        let lines = plan(12, 9);
        let regions = find_regions(&lines, 0.01).unwrap();
        assert_eq!(regions, find_regions(&lines[1..], 0.01).unwrap());
        // Each room with its pillar as a hole, and each pillar.
        assert_eq!(regions.len(), 2 * 12 * 9);
        assert!((regions[0].area_mm2 - (200.0 * 120.0 - 400.0)).abs() < 1e-6);
        assert!(regions[..12 * 9].iter().all(|room| room.holes.len() == 1));
        let figures = find_figures(&lines, 0.01).unwrap();
        assert_eq!(figures.len(), 1 + 12 * 9);
        assert_eq!(figures[0].holes.len(), 12 * 9);
    }

    /// Review 2026-10-26 UO-40: 200 000 pieces took 9.6 s, and a stray line 9 km away
    /// made one cell of the whole plan: 29 s. Each search, regions or figures, takes about
    /// half a second in release here; the budget is a second each.
    #[test]
    #[ignore = "perf: release timing budget, runs nightly"]
    fn perf_a_plan_of_two_hundred_thousand_pieces_and_a_stray_line_is_searched_in_a_second() {
        let lines = plan(200, 166);
        assert!((199_000..=MAX_REGION_SEGMENTS).contains(&lines.len()));
        let started = std::time::Instant::now();
        let regions = find_regions(&lines, 0.01).unwrap();
        let regions_took = started.elapsed();
        let started = std::time::Instant::now();
        let figures = find_figures(&lines, 0.01).unwrap();
        let figures_took = started.elapsed();
        assert!(regions.len() > 60_000 && figures.len() > 30_000);
        assert!(regions_took.as_secs_f64() < 1.0, "regions {regions_took:?}");
        assert!(figures_took.as_secs_f64() < 1.0, "figures {figures_took:?}");
    }

    #[test]
    fn walls_drawn_as_two_lines_pair_at_their_thickness() {
        let lines = [
            [[0.0, 0.0], [5_000.0, 0.0]],
            [[0.0, 300.0], [5_000.0, 300.0]],
            // A shorter one 150 apart alongside the first for 1 m.
            [[4_000.0, -150.0], [6_000.0, -150.0]],
            // Across: never a pair.
            [[100.0, -500.0], [100.0, 500.0]],
        ];
        let pairs = parallel_pairs(&lines, 50.0, 400.0, 2.0, 500.0, 100);
        assert_eq!(pairs.len(), 2);
        assert_eq!((pairs[0].a, pairs[0].b), (0, 1));
        assert!((pairs[0].gap_mm - 300.0).abs() < 1e-9);
        assert!((pairs[0].overlap_mm - 5_000.0).abs() < 1e-9);
        assert_eq!((pairs[1].a, pairs[1].b), (0, 2));
        assert!((pairs[1].overlap_mm - 1_000.0).abs() < 1e-9);
    }
}
