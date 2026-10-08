//! The title block in the lower right corner of a drawing sheet and the ISO A
//! sheet formats with their frame.

use crate::sheet_pdf::{Anchor, Mark, Page, SheetFont, Stroke};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A field of the title block. `Scale` and `Format` come from the sheet itself.
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
            (TitleField::Format, 25.0, false),
            (TitleField::JobNumber, 60.0, false),
        ],
    ),
];

const LABEL_MM: f64 = 1.6;

/// Text no wider than `room`: the height shrinks to fit, down to `LABEL_MM`.
fn fitted_height(font: &SheetFont, text: &str, height: f64, room: f64) -> f64 {
    let width = font.width_mm(text, height);
    if width <= room {
        height
    } else {
        (height * room / width).max(LABEL_MM)
    }
}

/// Draws the sheet frame and the title block with `labels` and `values`.
pub fn draw_frame_and_title_block(
    page: &mut Page,
    format: SheetFormat,
    labels: &BTreeMap<TitleField, String>,
    values: &TitleBlock,
) {
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
            if let Some(value) = values.get(field).filter(|value| !value.trim().is_empty()) {
                let wanted = match (prominent, height > 10.0) {
                    (true, true) => 4.5,
                    (true, false) => 3.0,
                    (false, _) => 2.5,
                };
                let size = fitted_height(font, value, wanted, width - 3.0);
                page.marks.push(Mark::Text {
                    at: [x + 1.5, y + height - (height - 2.4 - size) / 2.0 - 0.6],
                    text: value.trim().to_owned(),
                    height_mm: size,
                    anchor: Anchor::Start,
                    angle_deg: 0.0,
                    bold: *prominent,
                });
            }
            x += width;
        }
        y += height;
    }
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

    #[test]
    fn a_long_value_shrinks_to_fit_its_cell() {
        let mut page = Page {
            size_mm: SheetFormat::A3.size_mm(),
            marks: Vec::new(),
        };
        let long = "Rodinný dom s podkrovím a garážou, prístavba a nadstavba".repeat(2);
        let values = TitleBlock::from([(TitleField::Project, long.clone())]);
        draw_frame_and_title_block(&mut page, SheetFormat::A3, &BTreeMap::new(), &values);
        let font = SheetFont::standard();
        let fitted = page
            .marks
            .iter()
            .find_map(|mark| match mark {
                Mark::Text {
                    text, height_mm, ..
                } if *text == long => Some(*height_mm),
                _ => None,
            })
            .expect("the project is written");
        assert!(font.width_mm(&long, fitted) <= 92.0 || fitted <= LABEL_MM);
    }
}
