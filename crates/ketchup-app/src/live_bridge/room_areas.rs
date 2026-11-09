//! `drawing action=rooms` (review 2026-11-08 §5.3): the areas of the rooms of an architect's
//! plan written back into the drawing. For each room, given by a point in it and its name,
//! a solid HATCH of its floor (its holes left out) and its name and area as TEXT in it, and
//! a table of the rooms with their areas and the total beside the drawing, all on one layer
//! as one Undo step, so the drawing exported as DXF or DWG carries them.

use super::*;
use crate::app::underlay_edit::FindRegions;
use crate::app::underlay_extrude::AreaRefusal;
use ketchup_manufacturing::title_block::SheetLanguage;
use ketchup_model::underlay::drawn::DrawnShape;

/// Letters this high in the drawing's millimetres when no height is given: 2 mm at 1:100.
const DEFAULT_TEXT_MM: f64 = 200.0;
/// The most rooms one call writes.
const MAX_ROOMS: usize = 500;

/// The rooms to write into the drawing of `underlay_id`, on its layer `layer`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomsRequest {
    #[serde(default)]
    pub expected: Option<Stamp>,
    #[serde(default)]
    pub underlay_id: Option<u64>,
    pub layer: String,
    pub rooms: Vec<RoomAt>,
    /// Document millimetres.
    #[serde(default)]
    pub text_height_mm: Option<f64>,
    /// The table's top left corner, document millimetres.
    #[serde(default)]
    pub table_at_mm: Option<[f64; 2]>,
    /// `sk`, `cs` or `en`; the sheet language of the document otherwise.
    #[serde(default)]
    pub language: Option<String>,
}

/// A room: a point inside it (document millimetres) and its name.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomAt {
    pub at_mm: [f64; 2],
    pub name: String,
}

/// One room found: its rings in the drawing's millimetres and its area, as written.
struct Found {
    rings: Vec<Vec<[f64; 2]>>,
    area_m2: f64,
    left_out: Option<super::underlay_regions::LeftOut>,
}

impl LiveBridge {
    /// Draws the rooms of `request` into its drawing, or refuses with the cause recorded.
    pub(super) fn drawing_rooms(
        &mut self,
        app: &mut KetchupApp,
        request: &RoomsRequest,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, AreaRefusal> {
        let refused = AreaRefusal::taken;
        Self::guard(app, &request.expected).map_err(refused)?;
        Self::available(app, ui_busy).map_err(refused)?;
        if request.rooms.is_empty() || request.rooms.len() > MAX_ROOMS {
            return Err(refused(program_access::invalid(
                "rooms",
                format!("Give 1 to {MAX_ROOMS} rooms."),
                "Each room is {at_mm: a point inside it, name}.",
            )));
        }
        let language = match request.language.as_deref() {
            None => app
                .sheet_settings()
                .language
                .unwrap_or(SheetLanguage::English),
            Some(code) => SheetLanguage::from_code(code).ok_or_else(|| {
                refused(program_access::invalid(
                    "language",
                    "The language is sk, cs or en.",
                    "Omit it for the sheet language of the document.",
                ))
            })?,
        };
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, request.underlay_id)
            .map_err(refused)?;
        let id = underlay.id;
        let height = request
            .text_height_mm
            .map_or(DEFAULT_TEXT_MM, |height| height / underlay.scale());
        if !(height.is_finite() && height > 0.0) {
            return Err(refused(program_access::invalid(
                "text_height_mm",
                "The height of the letters must be a positive length.",
                "Give it in document millimetres, or omit it for 200 mm of the drawing.",
            )));
        }
        let lines = app
            .cached_underlay_lines(underlay)
            .ok_or_else(|| refused("entity_not_found"))?;
        let first = lines.objects().objects.len();
        let bounds = lines.bounds_mm();
        let to_local = |point: [f64; 2]| underlay.to_local(point);
        let table_at = match request.table_at_mm {
            Some(at) if at.iter().all(|value| value.is_finite()) => to_local(at),
            Some(_) => {
                return Err(refused(program_access::invalid(
                    "table_at_mm",
                    "The corner must be a point of finite numbers.",
                    "Give [x, y] in document millimetres, or omit it for right of the drawing.",
                )));
            }
            None => bounds.map_or([0.0, 0.0], |[_, high]| [high[0] + 4.0 * height, high[1]]),
        };

        let mut found: Vec<Found> = Vec::with_capacity(request.rooms.len());
        for (at, room) in request.rooms.iter().enumerate() {
            let target = format!("rooms[{at}]");
            if room.name.trim().is_empty() {
                return Err(refused(program_access::invalid(
                    &target,
                    "A room needs a name.",
                    "Give each room the name its label in the plan has, e.g. 1.01 Kitchen.",
                )));
            }
            let Some((outline, holes, left_out)) =
                super::underlay_program::underlay_area_outline(app, id, Some(room.at_mm), None)
                    .map_err(refused)?
            else {
                return Err(refused(program_access::invalid(
                    &target,
                    "No closed region of the shown layers holds its point.",
                    "Ask inspect action=underlay_regions with point_mm, or show the layers of the walls.",
                )));
            };
            let rings = std::iter::once(outline)
                .chain(holes)
                .map(|ring| ring.into_iter().map(to_local).collect::<Vec<_>>())
                .collect::<Vec<_>>();
            if let Some(same) = found.iter().position(|other| other.rings[0] == rings[0]) {
                return Err(refused(program_access::invalid(
                    &target,
                    format!("It lies in the same room as rooms[{same}]."),
                    "Give one point in each room.",
                )));
            }
            let area = rings
                .iter()
                .skip(1)
                .fold(ring_area(&rings[0]), |area, hole| area - ring_area(hole));
            found.push(Found {
                rings,
                area_m2: (area / 10_000.0).round() / 100.0,
                left_out,
            });
        }

        let words = Words::of(language);
        let mut shapes = Vec::new();
        let mut rooms = Vec::new();
        for (room, at) in found.iter().zip(&request.rooms) {
            let ids = first + shapes.len();
            let anchor = to_local(at.at_mm);
            shapes.push(DrawnShape::Hatch {
                rings_mm: room.rings.clone(),
            });
            shapes.push(text(anchor, height, at.name.trim().to_owned()));
            shapes.push(text(
                [anchor[0], anchor[1] - 1.5 * height],
                height,
                format!("{} m²", words.number(room.area_m2)),
            ));
            let mut row = json!({
                "name": at.name.trim(),
                "area_m2": room.area_m2,
                "hatch_id": ids,
                "text_ids": [ids + 1, ids + 2],
                "holes": room.rings.len() - 1,
            });
            if let Some(left_out) = &room.left_out {
                row["holes_left_out"] = json!({"count": left_out.count, "layers": left_out.layers});
            }
            rooms.push(row);
        }
        let total = (found.iter().map(|room| room.area_m2).sum::<f64>() * 100.0).round() / 100.0;
        let table_first = first + shapes.len();
        let names = request.rooms.iter().map(|room| room.name.trim());
        shapes.extend(table(&words, table_at, height, names.zip(&found), total));
        let count = shapes.len();

        Self::require_request_authority(cancelled).map_err(refused)?;
        let changes = app
            .draw_into_underlay(id, &request.layer, shapes, FindRegions::Here)
            .map_err(|error| refused(super::drawing_edit::edit_failure(error)))?;
        self.query.invalidate();
        Ok(super::drawing_edit::with_followed(
            json!({
                "rooms": rooms,
                "total_m2": total,
                "table": {
                    "object_ids": (table_first..first + count).collect::<Vec<_>>(),
                    "at_mm": underlay.to_document(table_at),
                },
                "drawn": count,
                "layer": request.layer,
                "language": language.code(),
                "underlay_id": id.0,
                "undo_steps": app.undo_step_count(),
                "dirty": app.is_dirty(),
            }),
            &changes,
        ))
    }
}

fn text(anchor_mm: [f64; 2], height_mm: f64, text: String) -> DrawnShape {
    DrawnShape::Text {
        anchor_mm,
        height_mm,
        rotation_deg: 0.0,
        text,
    }
}

/// `n` as a length factor; the table has at most [`MAX_ROOMS`] + 2 rows.
fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The area inside `ring`, whichever way it runs.
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let mut twice = 0.0;
    for (at, [x, y]) in ring.iter().enumerate() {
        let [next_x, next_y] = ring[(at + 1) % ring.len()];
        twice += x * next_y - next_x * y;
    }
    twice.abs() / 2.0
}

/// The words of the table and how it writes a number, in the sheet language.
struct Words {
    number: &'static str,
    room: &'static str,
    area: &'static str,
    total: &'static str,
    comma: bool,
}

impl Words {
    const fn of(language: SheetLanguage) -> Self {
        match language {
            SheetLanguage::Slovak => Self {
                number: "Č.",
                room: "Miestnosť",
                area: "Plocha [m²]",
                total: "Spolu",
                comma: true,
            },
            SheetLanguage::Czech => Self {
                number: "Č.",
                room: "Místnost",
                area: "Plocha [m²]",
                total: "Celkem",
                comma: true,
            },
            SheetLanguage::English => Self {
                number: "No.",
                room: "Room",
                area: "Area [m²]",
                total: "Total",
                comma: false,
            },
        }
    }

    /// `value` with two decimals and the language's decimal separator.
    fn number(&self, value: f64) -> String {
        let written = format!("{value:.2}");
        if self.comma {
            written.replace('.', ",")
        } else {
            written
        }
    }
}

/// The table of the rooms from its top left corner `at`: a header, a row per room (its
/// number, name and area) and the total, ruled, letters `height` high.
fn table<'a>(
    words: &Words,
    at: [f64; 2],
    height: f64,
    rooms: impl Iterator<Item = (&'a str, &'a Found)>,
    total: f64,
) -> Vec<DrawnShape> {
    let mut rows = vec![[
        words.number.to_owned(),
        words.room.to_owned(),
        words.area.to_owned(),
    ]];
    for (number, (name, room)) in rooms.enumerate() {
        rows.push([
            (number + 1).to_string(),
            name.to_owned(),
            words.number(room.area_m2),
        ]);
    }
    rows.push([String::new(), words.total.to_owned(), words.number(total)]);
    // A letter is taken as 0.8 of the height wide, and a cell keeps half a height clear.
    let width = |column: usize| {
        let longest = rows
            .iter()
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0);
        (count(longest) * 0.8 + 1.0) * height
    };
    let widths = [width(0), width(1), width(2)];
    let [x, top] = at;
    let right = x + widths.iter().sum::<f64>();
    let row_height = 2.0 * height;
    let bottom = top - row_height * count(rows.len());
    let line = |from: [f64; 2], to: [f64; 2]| DrawnShape::Polyline {
        points_mm: vec![from, to],
        closed: false,
    };
    let mut shapes = vec![DrawnShape::Polyline {
        points_mm: vec![[x, top], [right, top], [right, bottom], [x, bottom]],
        closed: true,
    }];
    for row in 1..rows.len() {
        let y = top - row_height * count(row);
        shapes.push(line([x, y], [right, y]));
    }
    let mut column_x = x;
    for width in &widths[..2] {
        column_x += width;
        shapes.push(line([column_x, top], [column_x, bottom]));
    }
    for (row, cells) in rows.into_iter().enumerate() {
        let baseline = top - row_height * count(row + 1) + 0.5 * height;
        let mut cell_x = x;
        for (cell, width) in cells.into_iter().zip(widths) {
            if !cell.is_empty() {
                shapes.push(text([cell_x + 0.5 * height, baseline], height, cell));
            }
            cell_x += width;
        }
    }
    shapes
}
