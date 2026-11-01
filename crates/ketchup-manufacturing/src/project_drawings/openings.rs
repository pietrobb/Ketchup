//! Windows and doors on the drawing set: each plan marks the openings of its
//! storey where they stand, and a schedule sheet lists every mark with the
//! designation it is bought by and how many openings carry it, as the
//! hardware list counts them.

use super::table::{RULE_MM, draw_table, table_size};
use super::{DrawingSolid, Point};
use crate::title_block::{SheetFormat, TITLE_BLOCK_MM, drawing_area};
use ketchup_model::sheet_pdf::{Anchor, Mark, Page, SheetFont, Stroke};
use serde::Serialize;

const MARK_TEXT_MM: f64 = 3.5;
const MARK_PAD_MM: f64 = 1.0;

/// One window or door: the mark the plans and the schedule show it by, the
/// designation it is bought by and the names of the solids it is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingOpening {
    pub mark: String,
    pub designation: String,
    pub parts: Vec<String>,
}

/// The openings and how their schedule sheet is titled.
#[derive(Clone, Debug, Default)]
pub struct OpeningSchedule {
    pub openings: Vec<DrawingOpening>,
    /// What the sheet shows, in its title block.
    pub title: String,
    /// Captions of the mark, designation and count columns.
    pub columns: [String; 3],
}

/// One row of the schedule: the openings of one mark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScheduleRow {
    pub mark: String,
    pub designation: String,
    pub count: usize,
}

impl ScheduleRow {
    fn cells(&self) -> [String; 3] {
        [
            self.mark.clone(),
            self.designation.clone(),
            self.count.to_string(),
        ]
    }
}

impl OpeningSchedule {
    /// One row per mark and designation, in the order they first come.
    #[must_use]
    pub fn rows(&self) -> Vec<ScheduleRow> {
        let mut rows: Vec<ScheduleRow> = Vec::new();
        for opening in &self.openings {
            match rows
                .iter_mut()
                .find(|row| row.mark == opening.mark && row.designation == opening.designation)
            {
                Some(row) => row.count += 1,
                None => rows.push(ScheduleRow {
                    mark: opening.mark.clone(),
                    designation: opening.designation.clone(),
                    count: 1,
                }),
            }
        }
        rows
    }
}

/// Where the mark of each opening stands: the middle of its solids in plan,
/// at their lowest point. An opening none of whose solids is drawn has none.
pub(super) fn opening_points(
    openings: &[DrawingOpening],
    solids: &[DrawingSolid],
) -> Vec<(String, Point)> {
    openings
        .iter()
        .filter_map(|opening| {
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            for point in solids
                .iter()
                .filter(|solid| opening.parts.contains(&solid.name))
                .flat_map(|solid| solid.triangles.iter().flatten())
            {
                for axis in 0..3 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
            min[0].is_finite().then(|| {
                (
                    opening.mark.clone(),
                    [
                        f64::midpoint(min[0], max[0]),
                        f64::midpoint(min[1], max[1]),
                        min[2],
                    ],
                )
            })
        })
        .collect()
}

/// The mark `text` boxed and centred at `at` (page mm); `painted` lays a white
/// ground under it, so it reads over a cut wall.
pub(super) fn draw_mark(page: &mut Page, at: [f64; 2], text: &str, painted: bool) {
    let half = [
        SheetFont::standard().width_mm(text, MARK_TEXT_MM) / 2.0 + MARK_PAD_MM,
        MARK_TEXT_MM / 2.0 + MARK_PAD_MM,
    ];
    let min = [at[0] - half[0], at[1] - half[1]];
    let max = [at[0] + half[0], at[1] + half[1]];
    if painted {
        page.marks.push(Mark::Fill {
            loops: vec![vec![min, [max[0], min[1]], max, [min[0], max[1]]]],
            color: [255, 255, 255],
            outline: None,
        });
    }
    page.rect(min, max, Stroke::solid(RULE_MM));
    page.text(
        [at[0], at[1] + MARK_TEXT_MM / 2.0],
        text,
        MARK_TEXT_MM,
        Anchor::Middle,
    );
}

fn cells(rows: &[ScheduleRow]) -> Vec<Vec<String>> {
    rows.iter().map(|row| row.cells().to_vec()).collect()
}

/// The requested format, or the smallest whose drawing area above the title
/// block holds the table; `None` when it does not hold it.
pub(super) fn schedule_format(
    columns: &[String; 3],
    rows: &[ScheduleRow],
    requested: Option<SheetFormat>,
) -> Option<SheetFormat> {
    let [width, height] = table_size(columns, &cells(rows));
    requested
        .map_or_else(|| SheetFormat::ALL.to_vec(), |format| vec![format])
        .into_iter()
        .find(|format| {
            let [[left, top], [right, bottom]] = drawing_area(*format);
            width <= right - left && height <= bottom - top - TITLE_BLOCK_MM[1]
        })
}

/// Draws the table at the top left of the drawing area: the captions, then a
/// row per mark, the count aligned right.
pub(super) fn draw_schedule(
    page: &mut Page,
    format: SheetFormat,
    columns: &[String; 3],
    rows: &[ScheduleRow],
) {
    draw_table(page, drawing_area(format)[0], columns, &cells(rows));
}
