//! Shop drawings: the parts a program gives the attribute [`SHEET`] are drawn
//! together on a sheet of that name (library `buildup()` gives it to each
//! piece of a build-up), seen along their local z with their local x to the
//! right. Identical parts share a position number: the same material and
//! blank size as on the cut list, made alike (outline, holes, pockets and
//! cuts), turned at most. A mirror image is a position of its own naming the
//! one it mirrors.

use crate::bom::key;
use crate::frame;
use crate::model::{Part, ProgramModel, ProgramOperation, ProgramPartBody, ProgramProfileSegment};
use serde::Serialize;

/// The part attribute naming the shop drawing a part is drawn on.
pub const SHEET: &str = "drawing:sheet";

/// The identical parts of one position.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShopPosition {
    pub material: String,
    /// The blank, longest first, as the cut list gives it.
    pub dimensions_mm: [f64; 3],
    pub parts: Vec<String>,
    /// The number of the position whose parts these mirror: the same made
    /// the other hand, which no turning makes alike.
    pub mirror_of: Option<usize>,
}

/// One sheet: its name, the world directions its view shows to the right and
/// up (the first part's local x and y, not necessarily unit) and its
/// positions, numbered from 1 in the order they first come.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShopDrawing {
    pub name: String,
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub positions: Vec<ShopPosition>,
}

/// The shop drawings of the parts in production, in the order they first come.
#[must_use]
pub fn shop_drawings(model: &ProgramModel) -> Vec<ShopDrawing> {
    let mut drawings: Vec<ShopDrawing> = Vec::new();
    // Per drawing, how the parts of each position are made.
    let mut made_keys: Vec<Vec<Making>> = Vec::new();
    for part in model.parts.iter().filter(|part| model.in_production(part)) {
        let Some(name) = part.attributes.get(SHEET) else {
            continue;
        };
        let index = drawings
            .iter()
            .position(|drawing| drawing.name == *name)
            .unwrap_or_else(|| {
                drawings.push(ShopDrawing {
                    name: name.clone(),
                    right: frame::axis(&part.rotation, 0),
                    up: frame::axis(&part.rotation, 1),
                    positions: Vec::new(),
                });
                drawings.len() - 1
            });
        let (min, max) = part.local_bounds();
        let mut dimensions: [f64; 3] = std::array::from_fn(|axis| max[axis] - min[axis]);
        dimensions.sort_by(|left, right| right.total_cmp(left));
        let material = part
            .material
            .clone()
            .unwrap_or_else(|| "unspecified".to_owned());
        let made = making(part);
        while made_keys.len() <= index {
            made_keys.push(Vec::new());
        }
        let positions = &mut drawings[index].positions;
        let alike = |made_as: &dyn Fn(&Making) -> &Keys| {
            positions
                .iter()
                .zip(&made_keys[index])
                .position(|(position, other)| {
                    position.material == material
                        && position.dimensions_mm.map(key) == dimensions.map(key)
                        && other.turned == *made_as(&made)
                })
        };
        match alike(&|made| &made.turned) {
            Some(found) => positions[found].parts.push(part.name.clone()),
            None => {
                let mirror_of = alike(&|made| &made.mirrored).map(|found| found + 1);
                positions.push(ShopPosition {
                    material,
                    dimensions_mm: dimensions,
                    parts: vec![part.name.clone()],
                    mirror_of,
                });
                made_keys[index].push(made);
            }
        }
    }
    drawings
}

/// Tagged points (mm keys) that tell how a part is made.
type Keys = Vec<(String, [i64; 3])>;

/// How a part is made, and how its mirror image is.
struct Making {
    turned: Keys,
    mirrored: Keys,
}

/// How a part is made, for telling positions apart: the corners of its body
/// and the places of its holes, pockets and cuts in its own frame from the
/// corner of its bounds, the same for the part turned any way (review
/// 2026-10-29 PG-62: members notched on both sides, on one side and on none
/// shared one position by material and blank alone). Turning flips an even
/// number of axes; an odd number makes the mirror image, the other hand
/// (review 2026-10-31 PG-83).
fn making(part: &Part) -> Making {
    let (min, max) = part.local_bounds();
    let mut points: Vec<(String, [f64; 3])> = Vec::new();
    match &part.body {
        ProgramPartBody::Extrusion {
            segments,
            distance_mm,
            ..
        } => outline(&mut points, "corner", segments, 0.0, *distance_mm),
        other => points.push((serde_json::to_string(other).unwrap_or_default(), [0.0; 3])),
    }
    for operation in &part.operations {
        match operation {
            ProgramOperation::Hole(hole) => {
                // The thread tells a tapped hole from a plain bore (review 2026-10-31 P3).
                let thread = hole.thread.as_ref().map(|thread| {
                    (
                        thread.size.as_str(),
                        key(thread.pitch_mm),
                        key(thread.depth_mm),
                    )
                });
                let tag = format!(
                    "hole {} {} {} {thread:?}",
                    key(hole.diameter_mm),
                    key(hole.depth_mm),
                    hole.through
                );
                let bottom = std::array::from_fn(|axis| {
                    hole.entry_mm[axis] + hole.inward[axis] * hole.depth_mm
                });
                points.push((tag.clone(), hole.entry_mm));
                points.push((tag, bottom));
            }
            ProgramOperation::Pocket(pocket) => {
                let [width, height] = pocket.extent_mm();
                let corner = pocket.corner_mm;
                let along = |direction: [f64; 3], by: f64| -> [f64; 3] {
                    std::array::from_fn(|axis| corner[axis] + direction[axis] * by)
                };
                for point in [
                    corner,
                    along(pocket.u, width),
                    along(pocket.v, height),
                    along(pocket.inward, pocket.depth_mm),
                ] {
                    points.push(("pocket".to_owned(), point));
                }
            }
            ProgramOperation::Cut(cut) => {
                let low = cut.depth_mm.map_or(min[2], |depth| max[2] - depth);
                outline(&mut points, "cut", &cut.segments, low, max[2]);
            }
            other => points.push((serde_json::to_string(other).unwrap_or_default(), [0.0; 3])),
        }
    }
    let flipped = |odd: u32| {
        (0..8_usize)
            .filter(|flips| flips.count_ones() % 2 == odd)
            .map(|mirror| {
                let mut keys = points
                    .iter()
                    .map(|(tag, point)| {
                        let at = std::array::from_fn(|axis| {
                            let value = if mirror & (1 << axis) == 0 {
                                point[axis]
                            } else {
                                min[axis] + max[axis] - point[axis]
                            };
                            key(value - min[axis])
                        });
                        (tag.clone(), at)
                    })
                    .collect::<Vec<_>>();
                keys.sort_unstable();
                keys.dedup();
                keys
            })
            .min()
            .unwrap_or_default()
    };
    Making {
        turned: flipped(0),
        mirrored: flipped(1),
    }
}

/// The corners (and arc centres and curve control points) of a profile,
/// tagged `tag`, at the heights `low` and `high` of its local z.
fn outline(
    points: &mut Vec<(String, [f64; 3])>,
    tag: &str,
    segments: &[ProgramProfileSegment],
    low: f64,
    high: f64,
) {
    for segment in segments {
        let [x, y] = segment.start_mm;
        points.push((tag.to_owned(), [x, y, low]));
        points.push((tag.to_owned(), [x, y, high]));
        if let Some(arc) = segment.arc {
            let [x, y] = arc.center_mm;
            points.push(([tag, " arc"].concat(), [x, y, low]));
        }
        for [x, y] in segment.bezier.into_iter().flatten() {
            points.push(([tag, " curve"].concat(), [x, y, low]));
        }
    }
}
