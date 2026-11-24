//! The revisions of a drawing set (review 2026-11-08 §5.3): each sheet lists
//! them in a table beside its title block, index, date and what changed, the
//! headings at the bottom and each revision above the one before it, as
//! ISO 7200 lays out a revision table.

use crate::title_block::{SheetFormat, TITLE_BLOCK_MM, drawing_area, wrap};
use ketchup_model::sheet_pdf::{Anchor, Page, SheetFont, Stroke};
use serde::{Deserialize, Serialize};

/// One revision of the set, as the designer writes it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SheetRevision {
    /// Its index, such as "A" or "01".
    pub index: String,
    pub date: String,
    /// What changed.
    pub change: String,
}

impl SheetRevision {
    fn is_blank(&self) -> bool {
        [&self.index, &self.date, &self.change]
            .iter()
            .all(|text| text.trim().is_empty())
    }
}

/// The headings of the table's columns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RevisionCaptions {
    pub index: String,
    pub date: String,
    pub change: String,
}

/// The table's width and its columns', left to right: index, date, change.
pub const REVISION_TABLE_MM: f64 = 150.0;
const COLUMNS_MM: [f64; 3] = [15.0, 25.0, 110.0];
const HEADING_ROW_MM: f64 = 5.0;
const CAPTION_MM: f64 = 1.6;
const TEXT_MM: f64 = 2.0;
/// Baseline to baseline of a wrapped change.
const PITCH_MM: f64 = 3.0;
/// What a row keeps free above and below its lines.
const ROW_MARGINS_MM: f64 = 3.0;

/// More revisions, or longer ones, than the table beside the title block
/// holds: the export would lose some, so it is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionsDoNotFit {
    pub revisions: usize,
    /// The newest revisions the table holds.
    pub fit: usize,
}

impl std::fmt::Display for RevisionsDoNotFit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the revision table beside the title block holds the newest {} of the {} revisions; shorten what they say changed or leave out the oldest",
            self.fit, self.revisions
        )
    }
}

impl std::error::Error for RevisionsDoNotFit {}

/// The lines of each cell of `revision` and the height of its row.
fn row(font: &SheetFont, revision: &SheetRevision) -> ([Vec<String>; 3], f64) {
    let cells = [&revision.index, &revision.date, &revision.change];
    let lines: [Vec<String>; 3] =
        std::array::from_fn(|column| wrap(font, cells[column], TEXT_MM, COLUMNS_MM[column] - 3.0));
    let count = lines.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let height = TEXT_MM + (1..count).map(|_| PITCH_MM).sum::<f64>() + ROW_MARGINS_MM;
    (lines, height)
}

/// Draws the table of `revisions` (oldest first; blank ones left out) to the
/// left of the title block of a `format` sheet, its bottom on the frame; none
/// without a revision.
///
/// # Errors
/// [`RevisionsDoNotFit`] when the rows are higher than the title block.
pub fn draw_revisions(
    page: &mut Page,
    format: SheetFormat,
    captions: &RevisionCaptions,
    revisions: &[SheetRevision],
) -> Result<(), RevisionsDoNotFit> {
    let revisions = revisions
        .iter()
        .filter(|revision| !revision.is_blank())
        .collect::<Vec<_>>();
    if revisions.is_empty() {
        return Ok(());
    }
    let font = SheetFont::standard();
    let rows = revisions
        .iter()
        .map(|revision| row(font, revision))
        .collect::<Vec<_>>();
    let room = TITLE_BLOCK_MM[1] - HEADING_ROW_MM;
    let mut used = 0.0;
    let fit = rows
        .iter()
        .rev()
        .take_while(|(_, height)| {
            used += height;
            used <= room
        })
        .count();
    if fit < rows.len() {
        return Err(RevisionsDoNotFit {
            revisions: rows.len(),
            fit,
        });
    }
    let [_, [right, bottom]] = drawing_area(format);
    let left = right - TITLE_BLOCK_MM[0] - REVISION_TABLE_MM;
    let headings = [&captions.index, &captions.date, &captions.change];
    let mut top = bottom - HEADING_ROW_MM;
    let mut x = left;
    for (heading, width) in headings.into_iter().zip(COLUMNS_MM) {
        page.rect([x, top], [x + width, bottom], Stroke::solid(0.25));
        page.text(
            [x + 1.2, top + HEADING_ROW_MM / 2.0 + CAPTION_MM / 2.0],
            heading,
            CAPTION_MM,
            Anchor::Start,
        );
        x += width;
    }
    for (lines, height) in rows {
        let row_bottom = top;
        top -= height;
        let mut x = left;
        for (cell, width) in lines.iter().zip(COLUMNS_MM) {
            page.rect([x, top], [x + width, row_bottom], Stroke::solid(0.25));
            let mut baseline = top + ROW_MARGINS_MM / 2.0 + TEXT_MM;
            for line in cell {
                page.text([x + 1.5, baseline], line, TEXT_MM, Anchor::Start);
                baseline += PITCH_MM;
            }
            x += width;
        }
    }
    page.rect(
        [left, top],
        [right - TITLE_BLOCK_MM[0], bottom],
        Stroke::solid(0.5),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::sheet_pdf::Mark;

    fn texts(page: &Page) -> Vec<(String, [f64; 2])> {
        page.marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Text { text, at, .. } => Some((text.clone(), *at)),
                _ => None,
            })
            .collect()
    }

    fn revision(index: &str, change: &str) -> SheetRevision {
        SheetRevision {
            index: index.to_owned(),
            date: "8. 11. 2026".to_owned(),
            change: change.to_owned(),
        }
    }

    fn captions() -> RevisionCaptions {
        RevisionCaptions {
            index: "Index".to_owned(),
            date: "Dátum".to_owned(),
            change: "Popis zmeny".to_owned(),
        }
    }

    #[test]
    fn the_revisions_stand_beside_the_title_block_newest_on_top() {
        let mut page = Page {
            size_mm: SheetFormat::A3.size_mm(),
            marks: Vec::new(),
        };
        let revisions = [
            revision("A", "Prvé vydanie"),
            revision("B", "Posunuté okno v kuchyni"),
        ];
        draw_revisions(&mut page, SheetFormat::A3, &captions(), &revisions).unwrap();
        let written = texts(&page);
        let at = |text: &str| {
            written
                .iter()
                .find(|(written, _)| written == text)
                .unwrap_or_else(|| panic!("{text} is not written: {written:?}"))
                .1
        };
        let [_, [right, bottom]] = drawing_area(SheetFormat::A3);
        let title_left = right - TITLE_BLOCK_MM[0];
        for (text, [x, y]) in &written {
            assert!(
                *x >= title_left - REVISION_TABLE_MM && *x < title_left,
                "{text} at {x} is not left of the title block"
            );
            assert!(
                *y <= bottom && *y >= bottom - TITLE_BLOCK_MM[1],
                "{text} at {y}"
            );
        }
        // Headings at the bottom, A above them and B, the newer, above A.
        assert!(at("Popis zmeny")[1] > at("Prvé vydanie")[1]);
        assert!(at("Prvé vydanie")[1] > at("Posunuté okno v kuchyni")[1]);
        assert!(at("A")[0] < at("Prvé vydanie")[0]);
    }

    #[test]
    fn no_revision_draws_no_table() {
        let mut page = Page {
            size_mm: SheetFormat::A3.size_mm(),
            marks: Vec::new(),
        };
        draw_revisions(
            &mut page,
            SheetFormat::A3,
            &captions(),
            &[SheetRevision::default()],
        )
        .unwrap();
        assert!(page.marks.is_empty());
    }

    #[test]
    fn a_long_change_wraps_and_too_many_revisions_are_refused_not_dropped() {
        let mut page = Page {
            size_mm: SheetFormat::A3.size_mm(),
            marks: Vec::new(),
        };
        let long = "Zmenená skladba strechy, doplnené prestupy a posunuté okná ".repeat(3);
        draw_revisions(
            &mut page,
            SheetFormat::A3,
            &captions(),
            &[revision("A", &long)],
        )
        .unwrap();
        let font = SheetFont::standard();
        let change_lines = texts(&page)
            .into_iter()
            .filter(|(text, _)| long.contains(text.as_str()) && text.len() > 3)
            .collect::<Vec<_>>();
        assert!(change_lines.len() > 1, "{change_lines:?}");
        for (text, _) in &change_lines {
            assert!(
                font.width_mm(text, TEXT_MM) <= COLUMNS_MM[2] - 3.0,
                "{text}"
            );
        }
        let many = (0..12)
            .map(|index| revision(&index.to_string(), "Zmena"))
            .collect::<Vec<_>>();
        let refused = draw_revisions(&mut page, SheetFormat::A3, &captions(), &many)
            .expect_err("twelve rows are higher than the title block");
        assert_eq!(refused.revisions, 12);
        assert_eq!(refused.fit, 8, "eight rows of 5 mm fit 44 mm");
    }
}
