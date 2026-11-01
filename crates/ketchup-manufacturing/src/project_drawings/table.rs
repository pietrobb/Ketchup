//! A ruled table on a sheet: a bold caption row, then a row per entry, the
//! last column aligned right.

use ketchup_model::sheet_pdf::{Anchor, Mark, Page, SheetFont, Stroke};

// not a tolerance: the pen width of the table rules and of a mark's box.
pub(super) const RULE_MM: f64 = 0.25;
const TEXT_MM: f64 = 3.5;
const ROW_MM: f64 = 7.0;
const PAD_MM: f64 = 2.0;

/// The width of each column: its widest cell, caption included.
fn column_widths(columns: &[String], rows: &[Vec<String>]) -> Vec<f64> {
    let font = SheetFont::standard();
    let width = |text: &str| font.width_mm(text, TEXT_MM) + 2.0 * PAD_MM;
    (0..columns.len())
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| width(cell))
                .fold(width(&columns[column]), f64::max)
        })
        .collect()
}

/// The width and height of the table on the page.
pub(super) fn table_size(columns: &[String], rows: &[Vec<String>]) -> [f64; 2] {
    [
        column_widths(columns, rows).iter().sum(),
        rows.iter().fold(ROW_MM, |height, _| height + ROW_MM),
    ]
}

/// Draws the table with its top left corner at `[left, top]`.
pub(super) fn draw_table(
    page: &mut Page,
    [left, top]: [f64; 2],
    columns: &[String],
    rows: &[Vec<String>],
) {
    let widths = column_widths(columns, rows);
    let [width, height] = table_size(columns, rows);
    let (right, bottom) = (left + width, top + height);
    let rule = Stroke::solid(RULE_MM);
    page.rect([left, top], [right, bottom], rule.clone());
    let mut x = left;
    for width in &widths[..widths.len().saturating_sub(1)] {
        x += width;
        page.line(vec![[x, top], [x, bottom]], rule.clone());
    }
    let baseline = (ROW_MM + TEXT_MM) / 2.0;
    let last = columns.len().saturating_sub(1);
    let mut y = top;
    for (index, cells) in std::iter::once(columns)
        .chain(rows.iter().map(Vec::as_slice))
        .enumerate()
    {
        if index > 0 {
            page.line(vec![[left, y], [right, y]], rule.clone());
        }
        let mut x = left;
        for (column, text) in cells.iter().enumerate() {
            let (at, anchor) = if column == last && index > 0 {
                (x + widths[column] - PAD_MM, Anchor::End)
            } else {
                (x + PAD_MM, Anchor::Start)
            };
            page.marks.push(Mark::Text {
                at: [at, y + baseline],
                text: text.clone(),
                height_mm: TEXT_MM,
                anchor,
                angle_deg: 0.0,
                bold: index == 0,
            });
            x += widths[column];
        }
        y += ROW_MM;
    }
}
