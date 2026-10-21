//! The letters of an underlay's texts for the GPU: the outline of each glyph used once, and
//! every letter as an instance of it, kept in cells of nearby letters. A frame draws a cell
//! whole, as one stroke along the baseline per letter while its letters are too small on
//! screen to read, or not at all while it is out of view: a drawing full of texts costs a
//! frame about what the letters it shows readably cost.

use super::{single_precision, text, vertex_index};
use ketchup_model::underlay::UnderlayLines;
use std::collections::BTreeMap;
use std::ops::Range;

/// About how many letters a cell holds: few enough that a view leaves most cells out,
/// enough that a whole drawing is few draws.
const LETTERS_PER_CELL: usize = 4096;
/// A letter whose capitals are lower than this on screen, in pixels, is drawn as a stroke.
const READABLE_PX: f32 = 3.0;

/// Bytes of one [`LetterInstance`] on the GPU.
pub(super) const INSTANCE_STRIDE: u64 = 36;

#[derive(Debug, Default, PartialEq)]
pub(super) struct Letters {
    /// The outline of every glyph used, in font units, two points per piece.
    pub outlines: Vec<[f32; 2]>,
    /// One glyph's letters of one cell.
    pub runs: Vec<LetterRun>,
    /// Grouped by cell, then by glyph.
    pub instances: Vec<LetterInstance>,
    pub cells: Vec<LetterCell>,
}

/// The letters of one glyph in a cell: its outline's vertices and their instances.
#[derive(Debug, PartialEq)]
pub(super) struct LetterRun {
    pub vertices: Range<u32>,
    pub instances: Range<u32>,
}

/// A letter: its glyph's outline drawn from `origin` along `x_axis` and `y_axis`
/// (millimetres per font unit), on a layer, in the color of a style; `advance` font units
/// long as a stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LetterInstance {
    pub origin: [f32; 2],
    pub x_axis: [f32; 2],
    pub y_axis: [f32; 2],
    pub layer: u32,
    pub advance: f32,
    pub style: u32,
}

impl LetterInstance {
    pub(super) fn write(&self, bytes: &mut Vec<u8>) {
        for value in [self.origin, self.x_axis, self.y_axis].iter().flatten() {
            bytes.extend(value.to_ne_bytes());
        }
        bytes.extend(self.layer.to_ne_bytes());
        bytes.extend(self.advance.to_ne_bytes());
        bytes.extend(self.style.to_ne_bytes());
    }
}

/// Letters near each other: their extent, the capital height of the tallest, in
/// millimetres, and their instances and runs.
#[derive(Debug, PartialEq)]
pub(super) struct LetterCell {
    low: [f32; 2],
    high: [f32; 2],
    letter_mm: f32,
    pub instances: Range<u32>,
    pub runs: Range<usize>,
}

/// How a frame draws a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CellView {
    Hidden,
    Strokes,
    Whole,
}

/// A letter while the cells are made.
struct Placed {
    cell: usize,
    glyph: u16,
    instance: LetterInstance,
    bounds: [[f64; 2]; 2],
    letter_mm: f64,
}

impl Letters {
    /// The letters of every layer's texts. The outline vertices each layer's letters draw
    /// are added to its count in `layer_vertices`.
    pub(super) fn new(lines: &UnderlayLines, layer_vertices: &mut [u64]) -> Self {
        let cap_units = text::cap_units();
        let mut glyphs = BTreeMap::<u16, (Vec<[f32; 2]>, [[f64; 2]; 2])>::new();
        let mut placed = Vec::new();
        for (index, layer) in lines.layers().iter().enumerate() {
            for text in layer.texts() {
                text::lay_out(text, &mut |letter| {
                    let (outline, glyph_bounds) = glyphs.entry(letter.glyph).or_insert_with(|| {
                        let outline = text::glyph_outline(letter.glyph);
                        let bounds = bounds_of(outline.iter().map(|point| point.map(f64::from)));
                        (outline, bounds)
                    });
                    if outline.is_empty() {
                        return;
                    }
                    layer_vertices[index] += u64::from(vertex_index(outline.len()));
                    let [low, high] = *glyph_bounds;
                    let corners =
                        [low, [high[0], low[1]], high, [low[0], high[1]]].map(|[x, y]| {
                            [0, 1].map(|axis| {
                                letter.origin_mm[axis]
                                    + letter.x_axis[axis] * x
                                    + letter.y_axis[axis] * y
                            })
                        });
                    placed.push(Placed {
                        cell: 0,
                        glyph: letter.glyph,
                        instance: LetterInstance {
                            origin: letter.origin_mm.map(gpu_float),
                            x_axis: letter.x_axis.map(gpu_float),
                            y_axis: letter.y_axis.map(gpu_float),
                            layer: vertex_index(index),
                            advance: gpu_float(letter.advance),
                            style: text.style,
                        },
                        bounds: bounds_of(corners.into_iter()),
                        letter_mm: letter.y_axis[0].hypot(letter.y_axis[1]) * cap_units,
                    });
                });
            }
        }
        if placed.is_empty() {
            return Self::default();
        }
        // A square grid over where the letters stand, about LETTERS_PER_CELL to a cell.
        let cells = placed.len().div_ceil(LETTERS_PER_CELL);
        let side = cells.isqrt() + usize::from(cells.isqrt().pow(2) < cells);
        let origin = |letter: &Placed| letter.instance.origin.map(f64::from);
        let [low, high] = bounds_of(placed.iter().map(origin));
        // The inner edges between the columns and between the rows.
        let edges = [0, 1].map(|axis| {
            (1..side)
                .map(|edge| low[axis] + (high[axis] - low[axis]) * count(edge) / count(side))
                .collect::<Vec<_>>()
        });
        for letter in &mut placed {
            let at = origin(letter);
            let [column, row] =
                [0, 1].map(|axis| edges[axis].partition_point(|edge| *edge <= at[axis]));
            letter.cell = row * side + column;
        }
        placed.sort_unstable_by_key(|letter| (letter.cell, letter.glyph));
        let mut letters = Self::default();
        let mut outline_vertices = BTreeMap::<u16, Range<u32>>::new();
        for cell in placed.chunk_by(|left, right| left.cell == right.cell) {
            let first_instance = vertex_index(letters.instances.len());
            let first_run = letters.runs.len();
            for run in cell.chunk_by(|left, right| left.glyph == right.glyph) {
                let glyph = run[0].glyph;
                let vertices = if let Some(vertices) = outline_vertices.get(&glyph) {
                    vertices.clone()
                } else {
                    let start = vertex_index(letters.outlines.len());
                    letters.outlines.extend_from_slice(&glyphs[&glyph].0);
                    let vertices = start..vertex_index(letters.outlines.len());
                    outline_vertices.insert(glyph, vertices.clone());
                    vertices
                };
                let start = vertex_index(letters.instances.len());
                letters
                    .instances
                    .extend(run.iter().map(|letter| letter.instance));
                letters.runs.push(LetterRun {
                    vertices,
                    instances: start..vertex_index(letters.instances.len()),
                });
            }
            let [low, high] = bounds_of(cell.iter().flat_map(|letter| letter.bounds));
            letters.cells.push(LetterCell {
                low: low.map(gpu_float),
                high: high.map(gpu_float),
                letter_mm: gpu_float(
                    cell.iter()
                        .map(|letter| letter.letter_mm)
                        .fold(0.0, f64::max),
                ),
                instances: first_instance..vertex_index(letters.instances.len()),
                runs: first_run..letters.runs.len(),
            });
        }
        letters
    }
}

impl LetterCell {
    /// How a frame through `world_to_clip` (column-major) onto a viewport this many pixels
    /// wide and high draws the cell. Seen from behind the camera's plane, a cell is drawn
    /// whole: no size on screen is known for it.
    pub(super) fn view(&self, world_to_clip: &[f32; 16], viewport_px: [f32; 2]) -> CellView {
        let clip = |[x, y]: [f32; 2]| {
            [0, 1, 2, 3].map(|row| {
                world_to_clip[row] * x + world_to_clip[4 + row] * y + world_to_clip[12 + row]
            })
        };
        let corners = [
            self.low,
            [self.high[0], self.low[1]],
            self.high,
            [self.low[0], self.high[1]],
        ];
        let projected = corners.map(clip);
        if projected.iter().any(|point| point[3] <= f32::EPSILON) {
            return CellView::Whole;
        }
        let beyond =
            |axis: usize, side: f32| projected.iter().all(|point| side * point[axis] > point[3]);
        if beyond(0, 1.0) || beyond(0, -1.0) || beyond(1, 1.0) || beyond(1, -1.0) {
            return CellView::Hidden;
        }
        let pixels = |from: [f32; 4], to: [f32; 4]| {
            let [dx, dy] = [0, 1]
                .map(|axis| (to[axis] / to[3] - from[axis] / from[3]) * viewport_px[axis] / 2.0);
            dx.hypot(dy)
        };
        let tallest = corners
            .iter()
            .zip(projected)
            .flat_map(|(corner, at)| {
                [[self.letter_mm, 0.0], [0.0, self.letter_mm]]
                    .map(|[dx, dy]| pixels(at, clip([corner[0] + dx, corner[1] + dy])))
            })
            .fold(0.0, f32::max);
        if tallest < READABLE_PX {
            CellView::Strokes
        } else {
            CellView::Whole
        }
    }
}

/// A millimetre or font-unit value in the GPU's single precision.
fn gpu_float(value: f64) -> f32 {
    single_precision([value, 0.0, 0.0])[0]
}

/// A count of cells as a coordinate factor; a grid's side stays far below `u32::MAX`.
fn count(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

fn bounds_of(points: impl Iterator<Item = [f64; 2]>) -> [[f64; 2]; 2] {
    points.fold(
        [[f64::MAX, f64::MAX], [f64::MIN, f64::MIN]],
        |[low, high], point| {
            [
                [low[0].min(point[0]), low[1].min(point[1])],
                [high[0].max(point[0]), high[1].max(point[1])],
            ]
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::underlay::UnderlayLinesBuilder;
    use ketchup_model::underlay::text::UnderlayText;

    /// A top view of the square `centre ± half` millimetres on a 1000 x 1000 pixel viewport.
    fn top_view(centre: [f32; 2], half: f32) -> [f32; 16] {
        let scale = 1.0 / half;
        [
            scale,
            0.0,
            0.0,
            0.0,
            0.0,
            scale,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            -centre[0] * scale,
            -centre[1] * scale,
            0.5,
            1.0,
        ]
    }

    #[test]
    fn a_cell_is_drawn_whole_as_strokes_or_not_by_its_letters_size_on_screen_and_place() {
        let mut builder = UnderlayLinesBuilder::default();
        builder.add_text("labels", UnderlayText::new("Hall", [0.0, 0.0], 100.0));
        let (_, lines) = builder.finish();
        let mut vertices = [0];
        let letters = Letters::new(&lines, &mut vertices);
        let [cell] = &letters.cells[..] else {
            panic!("one cell")
        };
        assert!((cell.letter_mm - 100.0).abs() < 0.01);
        let viewport = [1000.0, 1000.0];
        // 100 mm capitals: 50 px on a 2 m wide view, 0.5 px on a 200 m one.
        assert_eq!(
            cell.view(&top_view([100.0, 50.0], 1000.0), viewport),
            CellView::Whole
        );
        assert_eq!(
            cell.view(&top_view([100.0, 50.0], 100_000.0), viewport),
            CellView::Strokes
        );
        assert_eq!(
            cell.view(&top_view([5000.0, 0.0], 1000.0), viewport),
            CellView::Hidden
        );
    }

    #[test]
    fn many_letters_fall_into_cells_of_nearby_letters_grouped_by_glyph() {
        let mut builder = UnderlayLinesBuilder::default();
        for row in 0..100 {
            for column in 0..100 {
                let at = [f64::from(column) * 1000.0, f64::from(row) * 500.0];
                builder.add_text("labels", UnderlayText::new("Hall", at, 100.0));
            }
        }
        let (_, lines) = builder.finish();
        let mut vertices = [0];
        let letters = Letters::new(&lines, &mut vertices);
        assert_eq!(letters.instances.len(), 40_000);
        // 40,000 letters: a 4 x 4 grid, each cell 2,500 letters of three glyphs.
        assert_eq!(letters.cells.len(), 16);
        for cell in &letters.cells {
            assert_eq!(cell.instances.len(), 2_500);
            assert_eq!(cell.runs.len(), 3);
            let runs = &letters.runs[cell.runs.clone()];
            assert_eq!(runs[0].instances.start, cell.instances.start);
            assert_eq!(runs[2].instances.end, cell.instances.end);
        }
        let outline = |glyph: usize| letters.runs[glyph].vertices.clone();
        assert!((0..3).all(|glyph| letters.runs[glyph + 3].vertices == outline(glyph)));
        let drawn = letters
            .runs
            .iter()
            .map(|run| (run.vertices.len() * run.instances.len()) as u64)
            .sum::<u64>();
        assert_eq!(vertices, [drawn]);
    }
}
