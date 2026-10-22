//! Snapping to and measuring on the lines of an underlay: the endpoint, intersection,
//! midpoint and nearest point of its lines and arcs near a point, and the length of one.
//!
//! The lines keep no record of the drawing's entities: an arc is a run of chords
//! ([`super::UnderlayLines`]), and loose lines and arcs that meet end to end are one
//! polyline. A vertex is therefore taken as an end of a line or arc (a corner) unless the
//! chords on both sides of it are the equal chords of one arc, as the import cuts them.
//! An arc's midpoint and nearest point lie on its chords, at most
//! [`crate::tolerance::UNDERLAY_ARC_CHORD_MM`] from the arc itself.

use super::{UnderlayLines, arc_chord_step};
use crate::tolerance::{APPROXIMATION, ROUNDING};
use ketchup_geometry::linalg::{cross2, dot2, nearest_point_on_segment2};

/// What a snap found on an underlay line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnderlaySnapKind {
    /// An end of a line or an arc.
    Endpoint,
    /// Where two lines or arcs cross, or one ends on another.
    Intersection,
    /// The middle of a line or an arc, halfway along it.
    Midpoint,
    /// The point of a line or arc nearest the point asked about.
    Nearest,
}

/// A point of an underlay line near the point asked about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnderlaySnap {
    pub kind: UnderlaySnapKind,
    pub point_mm: [f64; 2],
    /// From the point asked about.
    pub distance_mm: f64,
    /// The index of its layer in the underlay's layers.
    pub layer: usize,
    /// The straight piece it lies on, numbered across all layers in their order.
    pub segment: usize,
}

/// The best snap of each kind within the distance asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnderlaySnaps {
    pub endpoint: Option<UnderlaySnap>,
    pub intersection: Option<UnderlaySnap>,
    pub midpoint: Option<UnderlaySnap>,
    pub nearest: Option<UnderlaySnap>,
}

impl UnderlaySnaps {
    /// Endpoint, intersection, midpoint, nearest: the order a snap prefers them in.
    pub fn iter(&self) -> impl Iterator<Item = &UnderlaySnap> {
        [
            &self.endpoint,
            &self.intersection,
            &self.midpoint,
            &self.nearest,
        ]
        .into_iter()
        .flatten()
    }
}

/// One line or arc of an underlay, from corner to corner: what a measurement of its
/// length reads.
#[derive(Clone, Debug, PartialEq)]
pub struct UnderlayPiece {
    pub layer: usize,
    pub points_mm: Vec<[f64; 2]>,
    pub length_mm: f64,
}

/// A run of straight pieces of one polyline between two corners. The pieces of a closed
/// polyline may wrap around its end.
#[derive(Clone, Copy, Debug)]
struct Piece {
    /// The first segment of its polyline and how many the polyline has.
    polyline_first: usize,
    polyline_len: usize,
    /// Its first segment, counted from `polyline_first`, and how many it has.
    first: usize,
    count: usize,
    /// The segment its midpoint lies on, and the midpoint; none for a closed ring
    /// without corners, a circle, whose halfway point depends only on where it starts.
    midpoint: Option<(usize, [f64; 2])>,
}

impl Piece {
    fn segments(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.count).map(|step| self.polyline_first + (self.first + step) % self.polyline_len)
    }
}

/// The segments of an underlay in a uniform grid, to find those near a point without
/// looking at every one: the Levoča drawing has half a million.
#[derive(Debug)]
pub struct UnderlaySnapIndex {
    segments: Vec<[[f64; 2]; 2]>,
    /// One past the last segment of each layer.
    layer_ends: Vec<usize>,
    /// Per segment: bit 0 its start is a corner, bit 1 its end is.
    corners: Vec<u8>,
    piece_of: Vec<usize>,
    pieces: Vec<Piece>,
    grids: Grids,
}

const START_CORNER: u8 = 1;
const END_CORNER: u8 = 2;

/// The most segments near a point whose crossings are looked for, the nearest ones: a
/// dense hatch under a wide snap radius would otherwise cost the square of thousands.
const MAX_CROSSING_SEGMENTS: usize = 256;

impl UnderlaySnapIndex {
    #[must_use]
    pub fn new(lines: &UnderlayLines) -> Self {
        let capacity = lines.segment_count();
        let mut segments = Vec::with_capacity(capacity);
        let mut corners = Vec::with_capacity(capacity);
        let mut piece_of = Vec::with_capacity(capacity);
        let mut pieces = Vec::new();
        let mut layer_ends = Vec::with_capacity(lines.layers().len());
        for layer in lines.layers() {
            for points in layer.polylines() {
                add_polyline(
                    points,
                    &mut segments,
                    &mut corners,
                    &mut piece_of,
                    &mut pieces,
                );
            }
            layer_ends.push(segments.len());
        }
        let grids = Grids::new(&segments);
        Self {
            segments,
            layer_ends,
            corners,
            piece_of,
            pieces,
            grids,
        }
    }

    /// How many times the index lists a segment in all: a few per segment, however long
    /// the segments are.
    #[must_use]
    pub fn listed(&self) -> usize {
        self.grids.0.iter().map(|grid| grid.entries.len()).sum()
    }

    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    fn layer_of(&self, segment: usize) -> usize {
        self.layer_ends.partition_point(|end| *end <= segment)
    }

    /// The nearest endpoint, midpoint and point on a line within `radius_mm` of
    /// `point_mm`, on the layers `shown` keeps.
    pub fn snaps(
        &self,
        point_mm: [f64; 2],
        radius_mm: f64,
        shown: impl Fn(usize) -> bool,
    ) -> UnderlaySnaps {
        let mut snaps = UnderlaySnaps::default();
        if !(radius_mm >= 0.0 && point_mm.iter().all(|value| value.is_finite())) {
            return snaps;
        }
        let offer = |slot: &mut Option<UnderlaySnap>, snap: UnderlaySnap| {
            if snap.distance_mm <= radius_mm
                && slot.is_none_or(|best| {
                    (snap.distance_mm, snap.segment) < (best.distance_mm, best.segment)
                })
            {
                *slot = Some(snap);
            }
        };
        let mut near = Vec::new();
        self.grids.near(point_mm, radius_mm, |segment| {
            let layer = self.layer_of(segment);
            if !shown(layer) {
                return;
            }
            let [start, end] = self.segments[segment];
            let snap = |kind, at: [f64; 2]| UnderlaySnap {
                kind,
                point_mm: at,
                distance_mm: distance(at, point_mm),
                layer,
                segment,
            };
            for (bit, at) in [(START_CORNER, start), (END_CORNER, end)] {
                if self.corners[segment] & bit != 0 {
                    offer(&mut snaps.endpoint, snap(UnderlaySnapKind::Endpoint, at));
                }
            }
            if let Some((on, midpoint)) = self.pieces[self.piece_of[segment]].midpoint
                && on == segment
            {
                offer(
                    &mut snaps.midpoint,
                    snap(UnderlaySnapKind::Midpoint, midpoint),
                );
            }
            let nearest = snap(
                UnderlaySnapKind::Nearest,
                nearest_point_on_segment2(point_mm, start, end),
            );
            if nearest.distance_mm <= radius_mm {
                near.push((nearest.distance_mm, segment));
            }
            offer(&mut snaps.nearest, nearest);
        });
        // A crossing within the radius lies on two segments within it.
        near.sort_unstable_by_key(|(_, segment)| *segment);
        near.dedup_by_key(|(_, segment)| *segment);
        if near.len() > MAX_CROSSING_SEGMENTS {
            near.select_nth_unstable_by(MAX_CROSSING_SEGMENTS, |a, b| {
                a.0.total_cmp(&b.0).then(a.1.cmp(&b.1))
            });
            near.truncate(MAX_CROSSING_SEGMENTS);
        }
        for (position, (_, first)) in near.iter().enumerate() {
            for (_, second) in &near[position + 1..] {
                let (first, second) = ((*first).min(*second), (*first).max(*second));
                let Some(at) = crossing(self.segments[first], self.segments[second]) else {
                    continue;
                };
                offer(
                    &mut snaps.intersection,
                    UnderlaySnap {
                        kind: UnderlaySnapKind::Intersection,
                        point_mm: at,
                        distance_mm: distance(at, point_mm),
                        layer: self.layer_of(first),
                        segment: first,
                    },
                );
            }
        }
        snaps
    }

    /// The line or arc `segment` is a piece of, from corner to corner.
    #[must_use]
    pub fn piece(&self, segment: usize) -> Option<UnderlayPiece> {
        let piece = self.pieces[*self.piece_of.get(segment)?];
        let mut points_mm = Vec::with_capacity(piece.count + 1);
        let mut length_mm = 0.0;
        for index in piece.segments() {
            let [start, end] = self.segments[index];
            if points_mm.is_empty() {
                points_mm.push(start);
            }
            points_mm.push(end);
            length_mm += distance(start, end);
        }
        Some(UnderlayPiece {
            layer: self.layer_of(segment),
            points_mm,
            length_mm,
        })
    }
}

/// Adds the segments of one polyline, their corners and the pieces between them.
fn add_polyline(
    points: &[[f64; 2]],
    segments: &mut Vec<[[f64; 2]; 2]>,
    corners: &mut Vec<u8>,
    piece_of: &mut Vec<usize>,
    pieces: &mut Vec<Piece>,
) {
    let first = segments.len();
    let count = points.len() - 1;
    let closed = count >= 3 && points[0] == points[count];
    // Whether vertex `index` (0..=count) ends a line or arc.
    let corner = |index: usize| {
        if index == 0 || index == count {
            !closed || !is_arc_vertex(points[count - 1], points[0], points[1])
        } else {
            !is_arc_vertex(points[index - 1], points[index], points[index + 1])
        }
    };
    let is_corner = (0..=count).map(corner).collect::<Vec<_>>();
    for index in 0..count {
        segments.push([points[index], points[index + 1]]);
        corners.push(
            if is_corner[index] { START_CORNER } else { 0 }
                | if is_corner[index + 1] { END_CORNER } else { 0 },
        );
    }
    // Pieces start at corners; a closed polyline starts at its first corner, so that its
    // last piece may wrap around its end.
    let start = (0..count).find(|index| is_corner[*index]);
    let ring = start.is_none();
    let start = start.unwrap_or(0);
    let mut step = 0;
    while step < count {
        let piece_first = (start + step) % count;
        let mut length = 1;
        while step + length < count && !is_corner[(piece_first + length) % count] {
            length += 1;
        }
        let index = pieces.len();
        let mut piece = Piece {
            polyline_first: first,
            polyline_len: count,
            first: piece_first,
            count: length,
            midpoint: None,
        };
        piece_of.extend(std::iter::repeat_n(index, length));
        if !ring {
            piece.midpoint = midpoint(&piece, segments);
        }
        pieces.push(piece);
        step += length;
    }
    // `piece_of` was filled in piece order; a wrapped closed polyline starts at `start`,
    // so put each segment's piece in its own place.
    if start != 0 {
        piece_of[first..].rotate_right(start);
    }
}

/// The point halfway along `piece`, and the segment it lies on.
fn midpoint(piece: &Piece, segments: &[[[f64; 2]; 2]]) -> Option<(usize, [f64; 2])> {
    let length = |index: usize| distance(segments[index][0], segments[index][1]);
    let total = piece.segments().map(length).sum::<f64>();
    let mut remaining = total * 0.5;
    for index in piece.segments() {
        let here = length(index);
        if remaining <= here && here > ROUNDING {
            let [start, end] = segments[index];
            let t = remaining / here;
            return Some((
                index,
                [
                    start[0] + (end[0] - start[0]) * t,
                    start[1] + (end[1] - start[1]) * t,
                ],
            ));
        }
        remaining -= here;
    }
    None
}

/// Whether `vertex` lies inside one arc: the chords on both sides of it are as long as
/// each other and turn by no more than the import's chord step for their radius
/// ([`arc_chord_step`]). A straight joint is a corner, as is a sharper one.
fn is_arc_vertex(before: [f64; 2], vertex: [f64; 2], after: [f64; 2]) -> bool {
    let incoming = difference(vertex, before);
    let outgoing = difference(after, vertex);
    let (first, second) = (distance(vertex, before), distance(after, vertex));
    if first <= ROUNDING || second <= ROUNDING {
        return false;
    }
    if (first - second).abs() > APPROXIMATION * first.max(second) + ROUNDING {
        return false;
    }
    let cross = cross2(incoming, outgoing).abs();
    if cross <= APPROXIMATION * first * second {
        return false;
    }
    let turn = cross.atan2(dot2(incoming, outgoing));
    let radius = first / (2.0 * (turn * 0.5).sin());
    turn <= arc_chord_step(radius) * (1.0 + APPROXIMATION)
}

fn difference(to: [f64; 2], from: [f64; 2]) -> [f64; 2] {
    [to[0] - from[0], to[1] - from[1]]
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Where segments `a` and `b` cross, or one ends on the other; not where both merely end,
/// as the chords of one polyline do at their shared corner, and not along parallel ones.
fn crossing(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> Option<[f64; 2]> {
    let (along_a, along_b) = (difference(a[1], a[0]), difference(b[1], b[0]));
    let (length_a, length_b) = (distance(a[0], a[1]), distance(b[0], b[1]));
    let denominator = cross2(along_a, along_b);
    if length_a <= ROUNDING
        || length_b <= ROUNDING
        || denominator.abs() <= APPROXIMATION * length_a * length_b
    {
        return None;
    }
    let between = difference(b[0], a[0]);
    let t = cross2(between, along_b) / denominator;
    let u = cross2(between, along_a) / denominator;
    let inside = |value: f64, length: f64| {
        let slack = APPROXIMATION / length;
        (-slack..=1.0 + slack).contains(&value)
    };
    if !inside(t, length_a) || !inside(u, length_b) {
        return None;
    }
    let at = [a[0][0] + along_a[0] * t, a[0][1] + along_a[1] * t];
    let ends_here =
        |[start, end]: [[f64; 2]; 2]| distance(at, start).min(distance(at, end)) <= APPROXIMATION;
    (!(ends_here(a) && ends_here(b))).then_some(at)
}

/// A count as a coordinate factor; grid counts stay far below `u32::MAX`.
fn factor(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// Square cells over the segments' bounds, each listing the segments that cross it.
#[derive(Debug)]
struct Grid {
    /// The low edge of each column and of each row.
    columns: Vec<f64>,
    rows: Vec<f64>,
    cell: f64,
    /// Where each cell's segments start in `entries`; one more than the cells.
    starts: Vec<usize>,
    entries: Vec<usize>,
}

/// The low edges of the cells from `low` on, the last one at `high` or below it.
fn edges(low: f64, high: f64, cell: f64) -> Vec<f64> {
    let mut edges = vec![low];
    loop {
        let next = low + factor(edges.len()) * cell;
        if next > high {
            return edges;
        }
        edges.push(next);
    }
}

/// The cell of `edges` that holds `value`; a value outside them is in the cell at that end.
fn cell_of(edges: &[f64], value: f64) -> usize {
    edges
        .partition_point(|edge| *edge <= value)
        .saturating_sub(1)
}

/// The most cells of a grid one segment is listed in. A longer segment, a line of a
/// hatch or a survey axis across the drawing, is listed in a grid of coarser cells, so
/// that the index grows with the segments, not with their length.
const MAX_CELLS_PER_SEGMENT: f64 = 16.0;

/// Grids over the same bounds, each of cells twice as large as the one before; a segment
/// is in the finest one it crosses at most [`MAX_CELLS_PER_SEGMENT`] cells of.
#[derive(Debug, Default)]
struct Grids(Vec<Grid>);

impl Grids {
    fn new(segments: &[[[f64; 2]; 2]]) -> Self {
        let Some(first) = segments.first() else {
            return Self::default();
        };
        let (mut low, mut high) = (first[0], first[0]);
        for point in segments.iter().flatten() {
            for axis in 0..2 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
        let size = difference(high, low);
        let count = factor(segments.len());
        // About one cell per segment, at most four.
        let mut cell = (size[0].max(1.0) * size[1].max(1.0) / count).sqrt();
        let cells = |cell: f64| (size[0] / cell + 1.0) * (size[1] / cell + 1.0);
        while cells(cell) > 4.0 * count + 16.0 {
            cell *= 2.0;
        }
        let mut levels: Vec<Vec<usize>> = Vec::new();
        for (index, [start, end]) in segments.iter().enumerate() {
            let along = difference(*end, *start).map(f64::abs);
            let (mut level, mut size) = (0, cell);
            // A segment crosses at most this many cells of `size`.
            while along[0] / size + along[1] / size + 3.0 > MAX_CELLS_PER_SEGMENT {
                level += 1;
                size *= 2.0;
            }
            if levels.len() <= level {
                levels.resize_with(level + 1, Vec::new);
            }
            levels[level].push(index);
        }
        let mut size = cell;
        let mut grids = Vec::new();
        for members in levels {
            if !members.is_empty() {
                grids.push(Grid::new(segments, &members, [low, high], size));
            }
            size *= 2.0;
        }
        Self(grids)
    }

    /// Calls `found` with every segment in a cell within `radius` of `point`; a segment
    /// may come more than once.
    fn near(&self, point: [f64; 2], radius: f64, mut found: impl FnMut(usize)) {
        for grid in &self.0 {
            grid.near(point, radius, &mut found);
        }
    }
}

impl Grid {
    /// The grid of cells `cell` wide over `bounds` listing the `members` of `segments`.
    fn new(
        segments: &[[[f64; 2]; 2]],
        members: &[usize],
        [low, high]: [[f64; 2]; 2],
        cell: f64,
    ) -> Self {
        let mut grid = Self {
            columns: edges(low[0], high[0], cell),
            rows: edges(low[1], high[1], cell),
            cell,
            starts: Vec::new(),
            entries: Vec::new(),
        };
        let mut counts = vec![0_usize; grid.columns.len() * grid.rows.len() + 1];
        for member in members {
            grid.cells_of(segments[*member], |cell| counts[cell + 1] += 1);
        }
        for index in 1..counts.len() {
            counts[index] += counts[index - 1];
        }
        let mut fill = counts.clone();
        let mut entries = vec![0; counts[counts.len() - 1]];
        for member in members {
            grid.cells_of(segments[*member], |cell| {
                entries[fill[cell]] = *member;
                fill[cell] += 1;
            });
        }
        grid.entries = entries;
        grid.starts = counts;
        grid
    }

    /// Calls `found` with every cell `segment` crosses, row by row.
    fn cells_of(&self, [start, end]: [[f64; 2]; 2], mut found: impl FnMut(usize)) {
        let (bottom, top) = if start[1] <= end[1] {
            (start, end)
        } else {
            (end, start)
        };
        let rise = top[1] - bottom[1];
        // The x of the segment at `y`; a level segment spans its whole width in its row.
        let at = |y: f64, level: f64| {
            if rise <= 0.0 {
                level
            } else {
                bottom[0] + (top[0] - bottom[0]) * ((y - bottom[1]) / rise).clamp(0.0, 1.0)
            }
        };
        for row in cell_of(&self.rows, bottom[1])..=cell_of(&self.rows, top[1]) {
            // The part of the segment inside this row's band.
            let low = self.rows[row];
            let from = at(low.max(bottom[1]), bottom[0]);
            let to = at((low + self.cell).min(top[1]), top[0]);
            let columns =
                cell_of(&self.columns, from.min(to))..=cell_of(&self.columns, from.max(to));
            for column in columns {
                found(row * self.columns.len() + column);
            }
        }
    }

    /// Calls `found` with every segment in a cell within `radius` of `point`; a segment
    /// may come more than once.
    fn near(&self, point: [f64; 2], radius: f64, mut found: impl FnMut(usize)) {
        let outside = |edges: &[f64], value: f64| {
            edges.first().is_none_or(|low| value + radius < *low)
                || edges
                    .last()
                    .is_none_or(|last| value - radius > *last + self.cell)
        };
        if outside(&self.columns, point[0]) || outside(&self.rows, point[1]) {
            return;
        }
        let columns =
            cell_of(&self.columns, point[0] - radius)..=cell_of(&self.columns, point[0] + radius);
        for row in cell_of(&self.rows, point[1] - radius)..=cell_of(&self.rows, point[1] + radius) {
            for column in columns.clone() {
                let cell = row * self.columns.len() + column;
                for entry in &self.entries[self.starts[cell]..self.starts[cell + 1]] {
                    found(*entry);
                }
            }
        }
    }
}
