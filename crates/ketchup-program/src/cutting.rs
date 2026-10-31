//! Cutting plans: the made parts of a material laid out on the stock the
//! program declares for it (`stock()`), lengths cut from bars of a few stock
//! lengths (1D) or rectangles cut from sheets by guillotine cuts (2D), with
//! the saw kerf between pieces and a trimmed edge. A heuristic (best fit,
//! largest pieces first), not a proven optimum.

use crate::bom::key;
use crate::model::{Part, ProgramModel};
use crate::table::{Cell, Table};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Pieces of one stock group beyond this are listed as not planned.
pub const MAX_PIECES: usize = 5000;

/// A piece fits a space it fills exactly.
const EPS: f64 = ketchup_tolerance::APPROXIMATION;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StockKind {
    /// Bars of a cross-section, each piece cut to its length.
    Lengths,
    /// Sheets of a thickness, each piece a rectangle.
    Sheets,
}

/// What a material is cut from.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Stock {
    pub kind: StockKind,
    /// Lengths: one entry per stock length. Sheets: [length, width], length
    /// the longer side.
    pub sizes_mm: Vec<Vec<f64>>,
    /// Width of one saw cut.
    pub kerf_mm: f64,
    /// Cut off every edge (each end of a bar) before the pieces.
    pub trim_mm: f64,
    /// Sheets: the part attribute naming the local axis ("x", "y", "z")
    /// that must run along the sheet's length; a part without it may turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub along: Option<String>,
}

/// How much of one stock size a plan uses.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct StockUse {
    pub size_mm: Vec<f64>,
    pub count: usize,
}

/// A piece laid on its bar or sheet.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Placed {
    pub part: String,
    /// From the stock's corner (its start for a bar), trim included.
    pub at_mm: Vec<f64>,
    /// Along the stock's length, then across it (sheets).
    pub size_mm: Vec<f64>,
    /// The piece's length runs across the sheet's length.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub turned: bool,
}

/// One bar or sheet and what is cut from it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Layout {
    pub stock_mm: Vec<f64>,
    pub pieces: Vec<Placed>,
    /// Pieces' length or area over the stock's.
    pub used_percent: f64,
    /// Bars: the rest after the last cut, less the end's trim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offcut_mm: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Unplaced {
    pub part: String,
    pub reason: String,
}

/// The parts of one material and cross-section (lengths) or thickness
/// (sheets) on their stock.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CuttingPlan {
    pub material: String,
    pub kind: StockKind,
    /// Lengths: [width, thickness] of the bars; sheets: [thickness].
    pub section_mm: Vec<f64>,
    pub kerf_mm: f64,
    pub trim_mm: f64,
    pub stock: Vec<StockUse>,
    pub layouts: Vec<Layout>,
    /// Pieces' length or area over all the stock's.
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unplaced: Vec<Unplaced>,
}

/// The cutting plans as a table for the saw: one row per piece, in cut order
/// on its bar or sheet, then one row per piece that fits no stock.
#[must_use]
pub fn cutting_table(plans: &[CuttingPlan]) -> Table {
    let header = [
        "material",
        "kind",
        "section_mm",
        "stock_no",
        "stock_mm",
        "part",
        "at_mm",
        "size_mm",
        "turned",
        "used_percent",
        "offcut_mm",
        "note",
    ]
    .map(str::to_owned)
    .into();
    let joined = |values: &[f64]| {
        values
            .iter()
            .map(|value| format!("{}", (value * 10.0).round() / 10.0))
            .collect::<Vec<_>>()
            .join(" x ")
    };
    let mut rows = Vec::new();
    for plan in plans {
        let kind = match plan.kind {
            StockKind::Lengths => "lengths",
            StockKind::Sheets => "sheets",
        };
        let first = |row: Vec<Cell>| {
            let mut cells = vec![
                Cell::Text(plan.material.clone()),
                Cell::Text(kind.to_owned()),
                Cell::Text(joined(&plan.section_mm)),
            ];
            cells.extend(row);
            cells
        };
        for (number, layout) in plan.layouts.iter().enumerate() {
            for piece in &layout.pieces {
                rows.push(first(vec![
                    Cell::Count(number + 1),
                    Cell::Text(joined(&layout.stock_mm)),
                    Cell::Text(piece.part.clone()),
                    Cell::Text(joined(&piece.at_mm)),
                    Cell::Text(joined(&piece.size_mm)),
                    Cell::Text(if piece.turned { "yes" } else { "" }.to_owned()),
                    Cell::Number(layout.used_percent),
                    layout
                        .offcut_mm
                        .map_or(Cell::Text(String::new()), Cell::Number),
                    Cell::Text(String::new()),
                ]));
            }
        }
        for piece in &plan.unplaced {
            rows.push(first(vec![
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(piece.part.clone()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(["not planned: ", piece.reason.as_str()].concat()),
            ]));
        }
    }
    Table { header, rows }
}

fn percent(part: f64, whole: f64) -> f64 {
    if whole <= 0.0 {
        return 0.0;
    }
    (part / whole * 1000.0).round() / 10.0
}

/// The part attribute naming the local axis ("x", "y" or "z") a bar's length
/// runs along (library `member()`).
pub const LENGTH_AXIS: &str = "length_axis";
/// The part attribute naming the local axis of a sheet's thickness, or of a
/// bar's section side that is a layer's thickness (library `buildup()`).
pub const THICKNESS_AXIS: &str = "thickness_axis";

/// The blank of a part: its length (longest first), section or size and
/// thickness, and which local axis each became. The axes a part names by
/// [`LENGTH_AXIS`] or [`THICKNESS_AXIS`] come where they belong, the others
/// longest first (review 2026-10-29 PG-61: a piece narrower than its sheet
/// was thick, or a bar shorter than its section was wide, was grouped by the
/// wrong size).
fn blank(part: &Part, kind: StockKind) -> ([f64; 3], [usize; 3]) {
    let (min, max) = part.local_bounds();
    let extent: [f64; 3] = std::array::from_fn(|axis| max[axis] - min[axis]);
    let longest_first = |axes: &mut [usize]| {
        axes.sort_by(|left, right| extent[*right].total_cmp(&extent[*left]));
    };
    let named = |attribute: &str| match part.attributes.get(attribute).map(String::as_str) {
        Some("x" | "X") => Some(0),
        Some("y" | "Y") => Some(1),
        Some("z" | "Z") => Some(2),
        _ => None,
    };
    let others = |axis: usize| {
        let mut rest = [(axis + 1) % 3, (axis + 2) % 3];
        longest_first(&mut rest);
        rest
    };
    let mut order = [0, 1, 2];
    longest_first(&mut order);
    match (kind, named(LENGTH_AXIS), named(THICKNESS_AXIS)) {
        (StockKind::Lengths, Some(axis), _) => {
            let [a, b] = others(axis);
            order = [axis, a, b];
        }
        (StockKind::Lengths, None, Some(axis)) => {
            let [a, b] = others(axis);
            order = [a, axis, b];
            longest_first(&mut order[1..]);
        }
        (StockKind::Sheets, _, Some(axis)) => {
            let [a, b] = others(axis);
            order = [a, b, axis];
        }
        _ => {}
    }
    (order.map(|axis| extent[axis]), order)
}

/// The parts of one material and section or thickness.
struct Group {
    section: Vec<f64>,
    pieces: Vec<Piece>,
    unplaced: Vec<Unplaced>,
}

struct Piece {
    part: String,
    /// Lengths: [length]; sheets: [length, width].
    size: [f64; 2],
    /// Sheets: whether it may lie turned, or must (Some(true)) or must not.
    turned: Option<bool>,
}

/// Every cutting plan of the made parts of `parts` (names) whose material
/// has stock.
#[must_use]
pub fn plans<'a>(
    model: &ProgramModel,
    parts: impl IntoIterator<Item = &'a str>,
) -> Vec<CuttingPlan> {
    let by_name = model.parts_by_name();
    // (material, section keys) -> its parts
    let mut groups: BTreeMap<(String, Vec<i64>), Group> = BTreeMap::new();
    for name in parts {
        let Some(part) = by_name.get(name) else {
            continue;
        };
        let Some(material) = &part.material else {
            continue;
        };
        let Some(stock) = model.stock.get(material) else {
            continue;
        };
        let (dimensions, order) = blank(part, stock.kind);
        let section = match stock.kind {
            StockKind::Lengths => vec![dimensions[1], dimensions[2]],
            StockKind::Sheets => vec![dimensions[2]],
        };
        let group = groups
            .entry((material.clone(), section.iter().map(|v| key(*v)).collect()))
            .or_insert_with(|| Group {
                section,
                pieces: Vec::new(),
                unplaced: Vec::new(),
            });
        let mut piece = Piece {
            part: part.name.clone(),
            size: [dimensions[0], dimensions[1]],
            turned: None,
        };
        if stock.kind == StockKind::Sheets
            && let Some(attribute) = &stock.along
        {
            let axis = match part.attributes.get(attribute).map(String::as_str) {
                Some("x" | "X") => Some(0),
                Some("y" | "Y") => Some(1),
                Some("z" | "Z") => Some(2),
                _ => None,
            };
            match axis.map(|axis| order.iter().position(|o| *o == axis)) {
                Some(Some(0)) => piece.turned = Some(false),
                Some(Some(1)) => piece.turned = Some(true),
                Some(_) => {
                    group.unplaced.push(Unplaced {
                        part: piece.part,
                        reason: format!(
                            "its {attribute} runs through its thickness, not along the sheet"
                        ),
                    });
                    continue;
                }
                None => {}
            }
        }
        group.pieces.push(piece);
    }
    groups
        .into_iter()
        .map(|((material, _), group)| {
            let Group {
                section,
                mut pieces,
                mut unplaced,
            } = group;
            let stock = &model.stock[&material];
            pieces.sort_by(|a, b| {
                (b.size[0] * b.size[1].max(1.0))
                    .total_cmp(&(a.size[0] * a.size[1].max(1.0)))
                    .then(b.size[0].total_cmp(&a.size[0]))
                    .then(a.part.cmp(&b.part))
            });
            for extra in pieces.drain(pieces.len().min(MAX_PIECES)..) {
                unplaced.push(Unplaced {
                    part: extra.part,
                    reason: format!("more than {MAX_PIECES} pieces of one stock are not planned"),
                });
            }
            let layouts = match stock.kind {
                StockKind::Lengths => cut_lengths(stock, pieces, &mut unplaced),
                StockKind::Sheets => cut_sheets(stock, pieces, &mut unplaced),
            };
            let measure = |sizes: &[f64]| sizes.iter().product::<f64>();
            let used: f64 = layouts
                .iter()
                .flat_map(|layout| &layout.pieces)
                .map(|piece| measure(&piece.size_mm))
                .sum();
            let whole: f64 = layouts.iter().map(|layout| measure(&layout.stock_mm)).sum();
            let mut counts: BTreeMap<Vec<i64>, StockUse> = BTreeMap::new();
            for layout in &layouts {
                counts
                    .entry(layout.stock_mm.iter().map(|v| key(*v)).collect())
                    .or_insert_with(|| StockUse {
                        size_mm: layout.stock_mm.clone(),
                        count: 0,
                    })
                    .count += 1;
            }
            CuttingPlan {
                material,
                kind: stock.kind,
                section_mm: section,
                kerf_mm: stock.kerf_mm,
                trim_mm: stock.trim_mm,
                stock: counts.into_values().collect(),
                layouts,
                used_percent: percent(used, whole),
                unplaced,
            }
        })
        .collect()
}

/// Best fit, longest first, on bars of the longest stock length; each bar
/// then shrinks to the shortest stock length its pieces fit.
fn cut_lengths(stock: &Stock, pieces: Vec<Piece>, unplaced: &mut Vec<Unplaced>) -> Vec<Layout> {
    let mut lengths: Vec<f64> = stock.sizes_mm.iter().map(|size| size[0]).collect();
    lengths.sort_by(f64::total_cmp);
    let usable = |length: f64| length - 2.0 * stock.trim_mm;
    let longest = lengths.last().copied().unwrap_or(0.0);
    // Pieces and the length they take, kerfs between them.
    let mut bars: Vec<(Vec<Piece>, f64)> = Vec::new();
    for piece in pieces {
        let length = piece.size[0];
        if length > usable(longest) + EPS {
            unplaced.push(Unplaced {
                part: piece.part,
                reason: format!(
                    "{length} mm is longer than the longest stock length {longest} mm less its trim"
                ),
            });
            continue;
        }
        let needs = |taken: f64, count: usize| {
            taken + length + if count == 0 { 0.0 } else { stock.kerf_mm }
        };
        let best = bars
            .iter()
            .enumerate()
            .filter(|(_, (cut, taken))| needs(*taken, cut.len()) <= usable(longest) + EPS)
            .min_by(|(_, (_, a)), (_, (_, b))| b.total_cmp(a))
            .map(|(index, _)| index);
        match best {
            Some(index) => {
                let (cut, taken) = &mut bars[index];
                *taken = needs(*taken, cut.len());
                cut.push(piece);
            }
            None => bars.push((vec![piece], length)),
        }
    }
    bars.into_iter()
        .map(|(cut, taken)| {
            let length = lengths
                .iter()
                .copied()
                .find(|length| taken <= usable(*length) + EPS)
                .unwrap_or(longest);
            let mut at = stock.trim_mm;
            let placed: Vec<Placed> = cut
                .into_iter()
                .map(|piece| {
                    let placed = Placed {
                        part: piece.part,
                        at_mm: vec![at],
                        size_mm: vec![piece.size[0]],
                        turned: false,
                    };
                    at += piece.size[0] + stock.kerf_mm;
                    placed
                })
                .collect();
            let pieces: f64 = placed.iter().map(|piece| piece.size_mm[0]).sum();
            Layout {
                stock_mm: vec![length],
                used_percent: percent(pieces, length),
                offcut_mm: Some(((length - stock.trim_mm - at).max(0.0) * 10.0).round() / 10.0),
                pieces: placed,
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Free {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

struct Sheet {
    size: [f64; 2],
    /// Per sheet axis: its factory edges stay, no trim comes off there.
    edges: [bool; 2],
    free: Vec<Free>,
    placed: Vec<Placed>,
}

/// Guillotine packing (best short side fit, the shorter leftover split off)
/// on sheets of the largest stock size; each sheet then shrinks to the
/// smallest stock size its pieces fit. Each piece takes its size plus one
/// kerf, the usable sheet one kerf more than its trimmed size. A piece as
/// long or as wide as a stock sheet (the whole sheet, or one cut only across)
/// keeps that sheet's factory edges along that side: such sheets are not
/// trimmed there and take only pieces spanning them the same way.
fn cut_sheets(stock: &Stock, pieces: Vec<Piece>, unplaced: &mut Vec<Unplaced>) -> Vec<Layout> {
    let mut sizes: Vec<[f64; 2]> = stock.sizes_mm.iter().map(|s| [s[0], s[1]]).collect();
    sizes.sort_by(|a, b| (a[0] * a[1]).total_cmp(&(b[0] * b[1])));
    let Some(&largest) = sizes.last() else {
        return Vec::new();
    };
    let kerf = stock.kerf_mm;
    let trim = stock.trim_mm;
    let usable = |size: [f64; 2]| [size[0] - 2.0 * trim + kerf, size[1] - 2.0 * trim + kerf];
    let fresh = |size: [f64; 2], edges: [bool; 2]| {
        let [w, h] = usable(size);
        let (x, w) = if edges[0] {
            (0.0, size[0] + kerf)
        } else {
            (trim, w)
        };
        let (y, h) = if edges[1] {
            (0.0, size[1] + kerf)
        } else {
            (trim, h)
        };
        Sheet {
            size,
            edges,
            free: vec![Free { x, y, w, h }],
            placed: Vec::new(),
        }
    };
    let mut sheets: Vec<Sheet> = Vec::new();
    for piece in pieces {
        let orientations: Vec<bool> = match piece.turned {
            Some(turned) => vec![turned],
            None => vec![false, true],
        };
        let footprint = |turned: bool| {
            let [l, w] = piece.size;
            if turned {
                [w + kerf, l + kerf]
            } else {
                [l + kerf, w + kerf]
            }
        };
        // The sheet axes the piece spans whole; without trim nothing differs.
        let spans = |size: [f64; 2], turned: bool| {
            let [w, h] = footprint(turned);
            [
                trim > EPS && (w - kerf - size[0]).abs() < EPS,
                trim > EPS && (h - kerf - size[1]).abs() < EPS,
            ]
        };
        // (short side leftover, long side leftover, sheet, free rect, turned)
        let consider = |sheets: &[Sheet]| {
            let mut best: Option<(f64, f64, usize, usize, bool)> = None;
            for (s, sheet) in sheets.iter().enumerate() {
                for (f, free) in sheet.free.iter().enumerate() {
                    for &turned in &orientations {
                        if spans(sheet.size, turned) != sheet.edges {
                            continue;
                        }
                        let [w, h] = footprint(turned);
                        if w <= free.w + EPS && h <= free.h + EPS {
                            let (dw, dh) = (free.w - w, free.h - h);
                            let fit = (dw.min(dh), dw.max(dh), s, f, turned);
                            if best.is_none_or(|b| (fit.0, fit.1) < (b.0, b.1)) {
                                best = Some(fit);
                            }
                        }
                    }
                }
            }
            best
        };
        let mut best = consider(&sheets);
        if best.is_none() {
            // The smallest stock sheet the piece spans and fits, else the
            // largest, trimmed all round.
            let edged = sizes.iter().find_map(|&size| {
                orientations.iter().find_map(|&turned| {
                    let edges = spans(size, turned);
                    let free = fresh(size, edges).free[0];
                    let [w, h] = footprint(turned);
                    (edges.contains(&true) && w <= free.w + EPS && h <= free.h + EPS)
                        .then_some((size, edges))
                })
            });
            let (size, edges) = edged.unwrap_or((largest, [false, false]));
            sheets.push(fresh(size, edges));
            best = consider(&sheets);
        }
        let Some((_, _, s, f, turned)) = best else {
            sheets.pop();
            let [l, w] = piece.size;
            unplaced.push(Unplaced {
                part: piece.part,
                reason: format!(
                    "{l} x {w} mm does not fit the largest stock sheet {} x {} mm less its trim{}",
                    largest[0],
                    largest[1],
                    if piece.turned.is_some() {
                        format!(
                            ", laid with its {} along the sheet",
                            stock.along.as_deref().unwrap_or("axis")
                        )
                    } else {
                        String::new()
                    }
                ),
            });
            continue;
        };
        let sheet = &mut sheets[s];
        let free = sheet.free.swap_remove(f);
        let [w, h] = footprint(turned);
        let [l, wd] = piece.size;
        sheet.placed.push(Placed {
            part: piece.part,
            at_mm: vec![free.x, free.y],
            size_mm: if turned { vec![wd, l] } else { vec![l, wd] },
            turned,
        });
        // Split along the shorter leftover: the larger remaining rectangle
        // stays whole.
        let (right_w, top_h) = (free.w - w, free.h - h);
        let (right, top) = if right_w < top_h {
            (
                Free {
                    x: free.x + w,
                    y: free.y,
                    w: right_w,
                    h,
                },
                Free {
                    x: free.x,
                    y: free.y + h,
                    w: free.w,
                    h: top_h,
                },
            )
        } else {
            (
                Free {
                    x: free.x + w,
                    y: free.y,
                    w: right_w,
                    h: free.h,
                },
                Free {
                    x: free.x,
                    y: free.y + h,
                    w,
                    h: top_h,
                },
            )
        };
        for rect in [right, top] {
            if rect.w > EPS && rect.h > EPS {
                sheet.free.push(rect);
            }
        }
    }
    sheets
        .into_iter()
        .map(|sheet| {
            let reach = sheet.placed.iter().fold([0.0f64; 2], |reach, piece| {
                [
                    reach[0].max(piece.at_mm[0] + piece.size_mm[0] + kerf),
                    reach[1].max(piece.at_mm[1] + piece.size_mm[1] + kerf),
                ]
            });
            let size = if sheet.edges.contains(&true) {
                sheet.size
            } else {
                sizes
                    .iter()
                    .copied()
                    .find(|size| {
                        let [w, h] = usable(*size);
                        reach[0] - trim <= w + EPS && reach[1] - trim <= h + EPS
                    })
                    .unwrap_or(sheet.size)
            };
            let area: f64 = sheet
                .placed
                .iter()
                .map(|p| p.size_mm[0] * p.size_mm[1])
                .sum();
            Layout {
                stock_mm: size.to_vec(),
                used_percent: percent(area, size[0] * size[1]),
                offcut_mm: None,
                pieces: sheet.placed,
            }
        })
        .collect()
}
