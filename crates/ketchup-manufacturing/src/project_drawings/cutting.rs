//! Cutting plans on the drawing set: for each plan the caller gives, every
//! bar or sheet of stock with the pieces cut from it, laid out as the plan
//! places them, on as many sheets as they need. Sheets of stock are drawn to
//! scale; bars are drawn to scale along their length as strips of one height,
//! since their cross-section says nothing on a cutting plan. Each piece is
//! labelled inside its outline when the label fits there, turned along a
//! narrow piece when it fits so; the rest are listed in the summary (the
//! cutting list has them all).

use super::{ProjectDrawingError, SCALES};
use crate::title_block::{SheetFormat, TITLE_BLOCK_MM, drawing_area};
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, SheetFont, Stroke};
use serde::Serialize;

// not a tolerance: the pen of a bar's or sheet's outline.
const STOCK_STROKE_MM: f64 = 0.35;
// not a tolerance: the pen of a piece's outline.
const PIECE_STROKE_MM: f64 = 0.18;
const LABEL_MM: f64 = 2.5;
const CAPTION_MM: f64 = 3.5;
/// Above each bar or sheet for its caption.
const CAPTION_ROOM_MM: f64 = 7.0;
/// Between bars or sheets, and kept clear of the frame and title block.
const GAP_MM: f64 = 8.0;
/// The drawn height of a bar.
const BAR_MM: f64 = 8.0;
// not a tolerance: room around a label inside its piece.
const LABEL_PAD_MM: f64 = 0.8;
/// The finest scale gives way to a coarser one until a sheet holds this many
/// bars or sheets of stock (or all of them).
const PER_SHEET: usize = 4;

/// A piece on its bar or sheet: from the stock's corner, along its length
/// then across it.
#[derive(Clone, Debug, PartialEq)]
pub struct CuttingPiece {
    pub label: String,
    pub at_mm: [f64; 2],
    pub size_mm: [f64; 2],
}

/// One bar (`size_mm[1]` its width, not drawn to scale) or sheet of stock.
#[derive(Clone, Debug, PartialEq)]
pub struct CuttingStock {
    pub caption: String,
    pub size_mm: [f64; 2],
    pub pieces: Vec<CuttingPiece>,
}

/// One cutting plan: its title and its bars or sheets in cutting order.
#[derive(Clone, Debug, PartialEq)]
pub struct CuttingSheet {
    pub title: String,
    pub bars: bool,
    pub stock: Vec<CuttingStock>,
}

/// What a sheet of a cutting plan shows: the captions of its bars or sheets,
/// how many piece labels it draws and the pieces too small for theirs.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CuttingSummary {
    pub stock: Vec<String>,
    pub labels: usize,
    pub unlabelled: Vec<String>,
}

/// One sheet of a cutting plan laid out.
pub(super) struct PlannedCutting {
    pub(super) plan: usize,
    pub(super) title: String,
    pub(super) format: SheetFormat,
    pub(super) scale: u32,
    /// The bars or sheets and the top left corner of each on the page.
    placed: Vec<(CuttingStock, [f64; 2])>,
    bars: bool,
}

/// Width and height of one bar or sheet on the page, caption included.
fn cell(stock: &CuttingStock, bars: bool, scale: f64) -> [f64; 2] {
    let height = if bars {
        BAR_MM
    } else {
        stock.size_mm[1] / scale
    };
    [stock.size_mm[0] / scale, CAPTION_ROOM_MM + height]
}

/// The room for the bars or sheets: the drawing area less a gap, above the
/// title block.
fn room(format: SheetFormat) -> ([f64; 2], [f64; 2]) {
    let [[left, top], [right, bottom]] = drawing_area(format);
    (
        [left + GAP_MM, top + GAP_MM],
        [
            right - left - 2.0 * GAP_MM,
            bottom - top - TITLE_BLOCK_MM[1] - 2.0 * GAP_MM,
        ],
    )
}

/// Rows of bars or sheets, left to right, each row as tall as its tallest:
/// the corner of each, relative to the room, until the room is full.
fn arrange(stock: &[CuttingStock], bars: bool, scale: f64, room: [f64; 2]) -> Vec<[f64; 2]> {
    let mut corners = Vec::new();
    let (mut x, mut y, mut row) = (0.0, 0.0, 0.0f64);
    for piece in stock {
        let [width, height] = cell(piece, bars, scale);
        if x > 0.0 && x + width > room[0] {
            (x, y, row) = (0.0, y + row + GAP_MM, 0.0);
        }
        if width > room[0] || y + height > room[1] {
            break;
        }
        corners.push([x, y]);
        x += width + GAP_MM;
        row = row.max(height);
    }
    corners
}

/// Lays `sheet` out on the format the caller fixes, else the smallest that
/// holds its largest bar or sheet at a scale of the set's list: the finest
/// scale at which a sheet holds [`PER_SHEET`] of them, else the coarsest that
/// holds one. None for a plan without stock.
pub(super) fn plan_cutting(
    plan: usize,
    sheet: &CuttingSheet,
    requested: Option<SheetFormat>,
) -> Result<Vec<PlannedCutting>, ProjectDrawingError> {
    if sheet.stock.is_empty() {
        return Ok(Vec::new());
    }
    let formats = requested.map_or(SheetFormat::ALL.to_vec(), |format| vec![format]);
    let wanted = sheet.stock.len().min(PER_SHEET);
    let mut chosen = None;
    'formats: for format in &formats {
        let (_, room) = room(*format);
        let mut holding_one = None;
        for scale in SCALES {
            let fits = |stock: &CuttingStock| {
                let [width, height] = cell(stock, sheet.bars, f64::from(scale));
                width <= room[0] && height <= room[1]
            };
            if !sheet.stock.iter().all(fits) {
                continue;
            }
            holding_one.get_or_insert(scale);
            if arrange(&sheet.stock, sheet.bars, f64::from(scale), room).len() >= wanted {
                chosen = Some((*format, scale));
                break 'formats;
            }
        }
        if let Some(scale) = holding_one {
            chosen = Some((*format, scale));
            break;
        }
    }
    let Some((format, scale)) = chosen else {
        return Err(ProjectDrawingError::DoesNotFit {
            scale: SCALES[SCALES.len() - 1],
            format: requested,
        });
    };
    let (corner, room) = room(format);
    let mut rest = sheet.stock.as_slice();
    let mut planned = Vec::new();
    while !rest.is_empty() {
        let corners = arrange(rest, sheet.bars, f64::from(scale), room);
        let (here, later) = rest.split_at(corners.len().max(1));
        planned.push(PlannedCutting {
            plan,
            title: sheet.title.clone(),
            format,
            scale,
            placed: here
                .iter()
                .cloned()
                .zip(
                    corners
                        .into_iter()
                        .map(|[x, y]| [corner[0] + x, corner[1] + y]),
                )
                .collect(),
            bars: sheet.bars,
        });
        rest = later;
    }
    Ok(planned)
}

impl PlannedCutting {
    /// Draws the bars or sheets with their pieces and captions.
    pub(super) fn draw(&self, page: &mut Page) -> CuttingSummary {
        let font = SheetFont::standard();
        let scale = f64::from(self.scale);
        let mut summary = CuttingSummary::default();
        for (stock, [left, top]) in &self.placed {
            page.marks.push(Mark::Text {
                at: [*left, top + CAPTION_MM + 1.0],
                text: stock.caption.clone(),
                height_mm: CAPTION_MM,
                anchor: Anchor::Start,
                angle_deg: 0.0,
                bold: true,
            });
            summary.stock.push(stock.caption.clone());
            let top = top + CAPTION_ROOM_MM;
            // Across a bar: the strip's height over the bar's width.
            let across = if self.bars {
                BAR_MM / stock.size_mm[1].max(f64::MIN_POSITIVE)
            } else {
                1.0 / scale
            };
            let at = |[x, y]: [f64; 2]| [left + x / scale, top + y * across];
            page.rect(
                at([0.0, 0.0]),
                at(stock.size_mm),
                Stroke::solid(STOCK_STROKE_MM),
            );
            for piece in &stock.pieces {
                let min = at(piece.at_mm);
                let max = at([
                    piece.at_mm[0] + piece.size_mm[0],
                    piece.at_mm[1] + piece.size_mm[1],
                ]);
                page.rect(min, max, Stroke::solid(PIECE_STROKE_MM));
                let [width, height] = [max[0] - min[0], max[1] - min[1]];
                let long = font.width_mm(&piece.label, LABEL_MM) + 2.0 * LABEL_PAD_MM;
                let short = LABEL_MM + 2.0 * LABEL_PAD_MM;
                let middle = [f64::midpoint(min[0], max[0]), f64::midpoint(min[1], max[1])];
                let angle = if long <= width && short <= height {
                    Some(0.0)
                } else if long <= height && short <= width {
                    Some(90.0)
                } else {
                    None
                };
                match angle {
                    Some(angle_deg) => {
                        // The baseline half a letter below (or beside) the middle.
                        let at = if angle_deg == 0.0 {
                            [middle[0], middle[1] + LABEL_MM / 2.0]
                        } else {
                            [middle[0] + LABEL_MM / 2.0, middle[1]]
                        };
                        page.marks.push(Mark::Text {
                            at,
                            text: piece.label.clone(),
                            height_mm: LABEL_MM,
                            anchor: Anchor::Middle,
                            angle_deg,
                            bold: false,
                        });
                        summary.labels += 1;
                    }
                    None => summary.unlabelled.push(piece.label.clone()),
                }
            }
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::tolerance::ROUNDING;

    fn sheet_of(count: usize) -> CuttingSheet {
        CuttingSheet {
            title: "OSB 18".to_owned(),
            bars: false,
            stock: (0..count)
                .map(|index| CuttingStock {
                    caption: format!("{} – 2500 x 1250", index + 1),
                    size_mm: [2500.0, 1250.0],
                    pieces: vec![
                        CuttingPiece {
                            label: "wall A 1200 x 600".to_owned(),
                            at_mm: [10.0, 10.0],
                            size_mm: [1200.0, 600.0],
                        },
                        CuttingPiece {
                            label: "strip 2400 x 60".to_owned(),
                            at_mm: [10.0, 614.0],
                            size_mm: [2400.0, 60.0],
                        },
                        CuttingPiece {
                            label: "post 300 x 1200".to_owned(),
                            at_mm: [1214.0, 10.0],
                            size_mm: [60.0, 1200.0],
                        },
                    ],
                })
                .collect(),
        }
    }

    #[test]
    fn sheets_of_stock_go_on_as_many_drawing_sheets_as_they_need_at_one_scale() {
        let planned = plan_cutting(0, &sheet_of(9), None).unwrap();
        // 1:20 on A3: 125 x 62.5 mm each, two to a row and two rows.
        assert_eq!(
            planned
                .iter()
                .map(|sheet| (sheet.format, sheet.scale, sheet.placed.len()))
                .collect::<Vec<_>>(),
            [
                (SheetFormat::A3, 20, 4),
                (SheetFormat::A3, 20, 4),
                (SheetFormat::A3, 20, 1)
            ]
        );
        let (corner, room) = room(SheetFormat::A3);
        for sheet in &planned {
            for (stock, [x, y]) in &sheet.placed {
                let [width, height] = cell(stock, false, 20.0);
                assert!(*x >= corner[0] && x + width <= corner[0] + room[0] + ROUNDING);
                assert!(*y >= corner[1] && y + height <= corner[1] + room[1] + ROUNDING);
            }
        }
    }

    #[test]
    fn a_label_goes_inside_its_piece_along_it_or_turned_or_is_listed() {
        let planned = plan_cutting(0, &sheet_of(1), None).unwrap();
        let mut page = Page {
            size_mm: planned[0].format.size_mm(),
            marks: Vec::new(),
        };
        let summary = planned[0].draw(&mut page);
        assert_eq!(summary.stock, ["1 – 2500 x 1250"]);
        // At 1:10 the post is 6 x 120 mm, its label turned; the strip 6 mm high.
        assert_eq!(planned[0].scale, 10);
        let angles: Vec<(&str, f64)> = page
            .marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Text {
                    text, angle_deg, ..
                } => Some((text.as_str(), *angle_deg)),
                _ => None,
            })
            .collect();
        assert_eq!(
            angles,
            [
                ("1 – 2500 x 1250", 0.0),
                ("wall A 1200 x 600", 0.0),
                ("strip 2400 x 60", 0.0),
                ("post 300 x 1200", 90.0)
            ]
        );
        assert_eq!(summary.labels, 3);
        assert!(summary.unlabelled.is_empty());
    }

    #[test]
    fn bars_are_strips_to_scale_along_their_length() {
        let sheet = CuttingSheet {
            title: "KVH 120 x 60".to_owned(),
            bars: true,
            stock: vec![CuttingStock {
                caption: "1 – 13000".to_owned(),
                size_mm: [13000.0, 120.0],
                pieces: vec![
                    CuttingPiece {
                        label: "s1 3200".to_owned(),
                        at_mm: [0.0, 0.0],
                        size_mm: [3200.0, 120.0],
                    },
                    CuttingPiece {
                        label: "block s2 90".to_owned(),
                        at_mm: [3204.0, 0.0],
                        size_mm: [90.0, 120.0],
                    },
                ],
            }],
        };
        let planned = plan_cutting(0, &sheet, None).unwrap();
        // 13 m is 260 mm at 1:50, the first scale that holds it on A3.
        assert_eq!((planned[0].format, planned[0].scale), (SheetFormat::A3, 50));
        let mut page = Page::default();
        let summary = planned[0].draw(&mut page);
        assert_eq!(summary.labels, 1);
        assert_eq!(summary.unlabelled, ["block s2 90"]);
        let outline = page
            .marks
            .iter()
            .find_map(|mark| match mark {
                Mark::Line { points, .. } => Some(points.clone()),
                _ => None,
            })
            .unwrap();
        assert!((outline[2][0] - outline[0][0] - 260.0).abs() < ROUNDING);
        assert!((outline[2][1] - outline[0][1] - BAR_MM).abs() < ROUNDING);
    }

    #[test]
    fn stock_too_large_for_the_fixed_format_is_refused() {
        let mut sheet = sheet_of(1);
        sheet.stock[0].size_mm = [500_000.0, 1250.0];
        assert_eq!(
            plan_cutting(0, &sheet, Some(SheetFormat::A3)).err(),
            Some(ProjectDrawingError::DoesNotFit {
                scale: 1000,
                format: Some(SheetFormat::A3)
            })
        );
    }
}
