//! Shop drawings: the parts a program gives the attribute [`SHEET`] are drawn
//! together on a sheet of that name (library `buildup()` gives it to each
//! piece of a build-up), seen along their local z with their local x to the
//! right. Identical parts, by material and blank size as on the cut list,
//! share a position number.

use crate::bom::key;
use crate::frame;
use crate::model::ProgramModel;
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
        let positions = &mut drawings[index].positions;
        match positions.iter_mut().find(|position| {
            position.material == material && position.dimensions_mm.map(key) == dimensions.map(key)
        }) {
            Some(position) => position.parts.push(part.name.clone()),
            None => positions.push(ShopPosition {
                material,
                dimensions_mm: dimensions,
                parts: vec![part.name.clone()],
            }),
        }
    }
    drawings
}
