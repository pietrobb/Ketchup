//! The title block in the lower right corner of a drawing sheet and the ISO A
//! sheet formats with their frame.

use crate::sheet_pdf::{Anchor, Mark, Page, SheetFont, Stroke};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A field of the title block. `Scale`, `Format` and `Sheet` come from the sheet itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleField {
    /// Who made the drawings: office or company.
    Office,
    /// The responsible (authorised) designer.
    Designer,
    /// Who drew it.
    Author,
    CheckedBy,
    Client,
    Project,
    Location,
    /// What the sheet shows.
    Drawing,
    /// Project stage, e.g. a building permit design.
    Stage,
    Date,
    DrawingNumber,
    JobNumber,
    Scale,
    Format,
    /// The sheet's number in its set, "n / N".
    Sheet,
}

impl TitleField {
    /// The fields a user fills in, in the order a form lists them.
    pub const EDITABLE: [Self; 12] = [
        Self::Project,
        Self::Location,
        Self::Client,
        Self::Drawing,
        Self::DrawingNumber,
        Self::Stage,
        Self::Date,
        Self::JobNumber,
        Self::Office,
        Self::Designer,
        Self::Author,
        Self::CheckedBy,
    ];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Office => "office",
            Self::Designer => "designer",
            Self::Author => "author",
            Self::CheckedBy => "checked_by",
            Self::Client => "client",
            Self::Project => "project",
            Self::Location => "location",
            Self::Drawing => "drawing",
            Self::Stage => "stage",
            Self::Date => "date",
            Self::DrawingNumber => "drawing_number",
            Self::JobNumber => "job_number",
            Self::Scale => "scale",
            Self::Format => "format",
            Self::Sheet => "sheet",
        }
    }
}

/// Field values by field; a missing field leaves its cell empty.
pub type TitleBlock = BTreeMap<TitleField, String>;

/// What a document keeps for its drawing sheets: the format (automatic when
/// absent), the title block, the height of the plan cut and the scale the
/// automatic format is chosen for.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SheetSettings {
    #[serde(default)]
    pub format: Option<SheetFormat>,
    #[serde(default)]
    pub title_block: TitleBlock,
    /// Height of the plan cut above the floor, whole millimetres;
    /// [`DEFAULT_PLAN_CUT_ABOVE_FLOOR_MM`] when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_cut_above_floor_mm: Option<u32>,
    /// The coarsest scale 1:N the automatic format may use;
    /// [`DEFAULT_COARSEST_SCALE`] when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coarsest_scale: Option<u32>,
}

/// The usual plan cut: at window-sill height, 1.2 m above the floor.
pub const DEFAULT_PLAN_CUT_ABOVE_FLOOR_MM: u32 = 1200;
/// Building drawings are read at 1:50.
pub const DEFAULT_COARSEST_SCALE: u32 = 50;

impl SheetSettings {
    #[must_use]
    pub fn plan_cut_above_floor_mm(&self) -> f64 {
        f64::from(
            self.plan_cut_above_floor_mm
                .unwrap_or(DEFAULT_PLAN_CUT_ABOVE_FLOOR_MM),
        )
    }

    #[must_use]
    pub fn coarsest_scale(&self) -> u32 {
        self.coarsest_scale.unwrap_or(DEFAULT_COARSEST_SCALE)
    }
}

/// ISO 216 A sheets, used in landscape.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum SheetFormat {
    A3,
    A2,
    A1,
    A0,
}

impl SheetFormat {
    pub const ALL: [Self; 4] = [Self::A3, Self::A2, Self::A1, Self::A0];

    /// Width and height in landscape.
    #[must_use]
    pub const fn size_mm(self) -> [f64; 2] {
        match self {
            Self::A3 => [420.0, 297.0],
            Self::A2 => [594.0, 420.0],
            Self::A1 => [841.0, 594.0],
            Self::A0 => [1189.0, 841.0],
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::A3 => "A3",
            Self::A2 => "A2",
            Self::A1 => "A1",
            Self::A0 => "A0",
        }
    }
}

/// The frame margins (ISO 5457): a 20 mm filing margin on the left, 10 mm elsewhere.
pub const FRAME_LEFT_MM: f64 = 20.0;
pub const FRAME_MM: f64 = 10.0;
pub const TITLE_BLOCK_MM: [f64; 2] = [180.0, 49.0];

/// The drawing area inside the frame: min and max corner.
#[must_use]
pub fn drawing_area(format: SheetFormat) -> [[f64; 2]; 2] {
    let [width, height] = format.size_mm();
    [
        [FRAME_LEFT_MM, FRAME_MM],
        [width - FRAME_MM, height - FRAME_MM],
    ]
}

/// One cell: field, width in mm and whether its value is the prominent one.
type Cell = (TitleField, f64, bool);

/// Rows from the top: height and cells left to right, 180 mm each.
const ROWS: [(f64, &[Cell]); 5] = [
    (
        9.0,
        &[
            (TitleField::Office, 95.0, false),
            (TitleField::Designer, 85.0, false),
        ],
    ),
    (
        9.0,
        &[
            (TitleField::Author, 55.0, false),
            (TitleField::CheckedBy, 40.0, false),
            (TitleField::Client, 85.0, false),
        ],
    ),
    (
        9.0,
        &[
            (TitleField::Project, 95.0, true),
            (TitleField::Location, 85.0, false),
        ],
    ),
    (
        13.0,
        &[
            (TitleField::Drawing, 135.0, true),
            (TitleField::DrawingNumber, 45.0, true),
        ],
    ),
    (
        9.0,
        &[
            (TitleField::Stage, 30.0, false),
            (TitleField::Date, 35.0, false),
            (TitleField::Scale, 30.0, true),
            (TitleField::Format, 20.0, false),
            (TitleField::Sheet, 20.0, false),
            (TitleField::JobNumber, 45.0, false),
        ],
    ),
];

const LABEL_MM: f64 = 1.6;
/// The smallest value height, and the step it shrinks by, in tenths of a millimetre.
const SMALLEST_VALUE_TENTHS: u32 = 16;
/// Baseline to baseline of a wrapped value, per millimetre of its height.
const LINE_PITCH: f64 = 1.5;
/// What a cell keeps free of value lines: the label above them and a margin below.
const VALUE_MARGINS_MM: f64 = 3.8;

/// A title block value that does not fit its cell even in lines of the
/// smallest height: the export would lose part of it, so it is refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TitleValueDoesNotFit {
    pub field: TitleField,
    /// The lines of the smallest height the cell holds.
    pub lines: usize,
    /// The characters of the value those lines hold.
    pub characters: usize,
    /// The characters of the whole value.
    pub length: usize,
}

impl std::fmt::Display for TitleValueDoesNotFit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the title block value {} does not fit its cell: {} lines of {LABEL_MM} mm hold {} of its {} characters",
            self.field.key(),
            self.lines,
            self.characters,
            self.length
        )
    }
}

impl std::error::Error for TitleValueDoesNotFit {}

/// How many lines of `height` a cell `cell_height` high holds below its label.
fn lines_in_cell(cell_height: f64, height: f64) -> usize {
    let room = cell_height - VALUE_MARGINS_MM;
    let mut lines = 0;
    let mut block = height;
    while block <= room {
        lines += 1;
        block += LINE_PITCH * height;
    }
    lines
}

/// `text` broken at spaces into lines no wider than `room`; a word wider than
/// `room` on its own is broken between its characters.
fn wrap(font: &SheetFont, text: &str, height: f64, room: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let joined = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if font.width_mm(&joined, height) <= room {
            line = joined;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        for character in word.chars() {
            line.push(character);
            if line.chars().count() > 1 && font.width_mm(&line, height) > room {
                line.pop();
                lines.push(std::mem::replace(&mut line, character.to_string()));
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The lines of `value` and their height in a cell `width` × `cell_height`:
/// the largest height from `wanted_tenths` down at which the whole value fits,
/// on one line when it can.
fn fit_value(
    font: &SheetFont,
    field: TitleField,
    value: &str,
    wanted_tenths: u32,
    [width, cell_height]: [f64; 2],
) -> Result<(Vec<String>, f64), TitleValueDoesNotFit> {
    let room = width - 3.0;
    for tenths in (SMALLEST_VALUE_TENTHS..=wanted_tenths).rev() {
        let height = f64::from(tenths) / 10.0;
        let capacity = lines_in_cell(cell_height, height);
        if capacity == 0 {
            continue;
        }
        if font.width_mm(value, height) <= room {
            return Ok((vec![value.to_owned()], height));
        }
        let lines = wrap(font, value, height, room);
        if lines.len() <= capacity {
            return Ok((lines, height));
        }
    }
    let smallest = f64::from(SMALLEST_VALUE_TENTHS) / 10.0;
    let lines = lines_in_cell(cell_height, smallest);
    Err(TitleValueDoesNotFit {
        field,
        lines,
        characters: wrap(font, value, smallest, room)
            .iter()
            .take(lines)
            .map(|line| line.chars().count())
            .sum(),
        length: value.chars().count(),
    })
}

/// Draws the sheet frame and the title block with `labels` and `values`.
///
/// # Errors
/// [`TitleValueDoesNotFit`] when a value does not fit its cell even in lines
/// of the smallest height; it is never cut short.
pub fn draw_frame_and_title_block(
    page: &mut Page,
    format: SheetFormat,
    labels: &BTreeMap<TitleField, String>,
    values: &TitleBlock,
) -> Result<(), TitleValueDoesNotFit> {
    let font = SheetFont::standard();
    let [[left, top], [right, bottom]] = drawing_area(format);
    page.rect([left, top], [right, bottom], Stroke::solid(0.7));
    let origin = [right - TITLE_BLOCK_MM[0], bottom - TITLE_BLOCK_MM[1]];
    page.rect(origin, [right, bottom], Stroke::solid(0.5));
    let mut y = origin[1];
    for (height, cells) in ROWS {
        let mut x = origin[0];
        for (field, width, prominent) in cells {
            page.rect([x, y], [x + width, y + height], Stroke::solid(0.25));
            if let Some(label) = labels.get(field) {
                page.text([x + 1.2, y + 2.4], label, LABEL_MM, Anchor::Start);
            }
            if let Some(value) = values
                .get(field)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                let wanted_tenths = match (prominent, height > 10.0) {
                    (true, true) => 45,
                    (true, false) => 30,
                    (false, _) => 25,
                };
                let (lines, size) =
                    fit_value(font, *field, value, wanted_tenths, [*width, height])?;
                // The block of lines sits in the middle of the cell below its label.
                let pitch = LINE_PITCH * size;
                let block = size + lines.iter().skip(1).map(|_| pitch).sum::<f64>();
                let last = y + height - (height - 2.4 - block) / 2.0 - 0.6;
                let mut baseline = last - (block - size);
                for line in lines {
                    page.marks.push(Mark::Text {
                        at: [x + 1.5, baseline],
                        text: line,
                        height_mm: size,
                        anchor: Anchor::Start,
                        angle_deg: 0.0,
                        bold: *prominent,
                    });
                    baseline += pitch;
                }
            }
            x += width;
        }
        y += height;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_title_block_row_spans_the_block() {
        for (_, cells) in ROWS {
            let width: f64 = cells.iter().map(|(_, width, _)| width).sum();
            assert!((width - TITLE_BLOCK_MM[0]).abs() < 1e-9);
        }
        let height: f64 = ROWS.iter().map(|(height, _)| height).sum();
        assert!((height - TITLE_BLOCK_MM[1]).abs() < 1e-9);
    }

    /// The value lines written into the title block: text, baseline and height.
    fn value_lines(page: &Page) -> Vec<(String, [f64; 2], f64)> {
        page.marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Text {
                    text,
                    at,
                    height_mm,
                    ..
                } => Some((text.clone(), *at, *height_mm)),
                _ => None,
            })
            .collect()
    }

    /// The Project cell of an A3 sheet: its left, top, right and bottom edge.
    fn project_cell() -> [f64; 4] {
        let [_, [right, bottom]] = drawing_area(SheetFormat::A3);
        let left = right - TITLE_BLOCK_MM[0];
        let top = bottom - TITLE_BLOCK_MM[1] + 18.0;
        [left, top, left + 95.0, top + 9.0]
    }

    #[test]
    fn a_long_value_shrinks_to_fit_its_cell() {
        let mut page = Page {
            size_mm: SheetFormat::A3.size_mm(),
            marks: Vec::new(),
        };
        // Two lines of 1.6 mm hold it; one line of 1.6 mm does not.
        let long = "Rodinný dom s podkrovím a garážou, prístavba a nadstavba ".repeat(2);
        let values = TitleBlock::from([(TitleField::Project, long.clone())]);
        draw_frame_and_title_block(&mut page, SheetFormat::A3, &BTreeMap::new(), &values)
            .expect("the project fits in lines");
        let font = SheetFont::standard();
        let [left, top, right, bottom] = project_cell();
        assert!(
            font.width_mm(long.trim(), LABEL_MM) > right - left,
            "on one line even the smallest height overflows the cell"
        );
        let lines = value_lines(&page);
        assert!(lines.len() > 1, "the project is wrapped: {lines:?}");
        for (text, [x, baseline], height) in &lines {
            assert!(*height >= LABEL_MM, "{text:?} is {height} mm high");
            assert!(
                *x >= left && x + font.width_mm(text, *height) <= right,
                "{text:?}"
            );
            assert!(
                baseline - height >= top + 2.4 && *baseline <= bottom,
                "{text:?}"
            );
        }
        let written: Vec<&str> = lines
            .iter()
            .flat_map(|(text, _, _)| text.split_whitespace())
            .collect();
        assert_eq!(written, long.split_whitespace().collect::<Vec<_>>());
    }

    #[test]
    fn a_value_too_long_even_for_the_smallest_lines_is_refused_not_cut_short() {
        let mut page = Page {
            size_mm: SheetFormat::A3.size_mm(),
            marks: Vec::new(),
        };
        let long = "Rodinný dom s podkrovím a garážou, prístavba a nadstavba ".repeat(8);
        let values = TitleBlock::from([(TitleField::Project, long.clone())]);
        let refused =
            draw_frame_and_title_block(&mut page, SheetFormat::A3, &BTreeMap::new(), &values)
                .expect_err("the project cannot be written whole");
        assert_eq!(refused.field, TitleField::Project);
        assert_eq!(refused.lines, 2, "a 9 mm row holds two lines of 1.6 mm");
        assert_eq!(refused.length, long.trim().chars().count());
        assert!(refused.characters > 0 && refused.characters < refused.length);
        assert!(refused.to_string().contains("project"), "{refused}");
    }
}
