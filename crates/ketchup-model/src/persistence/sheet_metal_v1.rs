//! Sheet metal as files stored it before snapshot format 101: a `width` x `depth`
//! rectangle with at most one flange on each of its sides, named `MinX`, `MaxX`, `MinY` and
//! `MaxY`. It reads as the rectangle's corners counter-clockwise from the origin with a bend
//! on the matching edge: `MinY` is edge 0, `MaxX` 1, `MaxY` 2 and `MinX` 3.

use crate::document::Dimension;
use crate::sheet_metal::{SheetMetalBend, SheetMetalSpec};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SheetMetalV1 {
    pub(crate) width: Dimension,
    pub(crate) depth: Dimension,
    pub(crate) thickness: Dimension,
    pub(crate) k_factor: f64,
    pub(crate) flanges: Vec<FlangeV1>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FlangeV1 {
    pub(crate) edge: String,
    pub(crate) length: Dimension,
    pub(crate) angle_degrees: f64,
    pub(crate) inner_radius: Dimension,
}

/// The old side names in their old order, each with the rectangle edge it became.
const SIDES: [(&str, usize); 4] = [("MinX", 3), ("MaxX", 1), ("MinY", 0), ("MaxY", 2)];

/// The side a binary file stored as its 1-based position in the old order.
pub(crate) fn stored_side(code: u8) -> Option<String> {
    let (name, _) = SIDES.get(usize::from(code).checked_sub(1)?)?;
    Some((*name).to_owned())
}

impl FlangeV1 {
    /// The position of the side in the old order `MinX`, `MaxX`, `MinY`, `MaxY`.
    pub(crate) fn side(&self) -> Option<usize> {
        SIDES.iter().position(|(name, _)| *name == self.edge)
    }
}

impl SheetMetalV1 {
    /// The current spec and, for each old flange in order, the index of its bend.
    pub(crate) fn to_current(&self) -> Option<(SheetMetalSpec, Vec<usize>)> {
        let (width, depth) = (self.width.millimetres(), self.depth.millimetres());
        let mut bends = self
            .flanges
            .iter()
            .enumerate()
            .map(|(old, flange)| {
                let edge = SIDES[flange.side()?].1;
                Some((
                    old,
                    SheetMetalBend {
                        parent: None,
                        edge,
                        length: flange.length.clone(),
                        angle_degrees: flange.angle_degrees,
                        inner_radius: flange.inner_radius.clone(),
                    },
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        bends.sort_by_key(|(_, bend)| bend.edge);
        let mut bend_of_flange = vec![0; bends.len()];
        for (index, (old, _)) in bends.iter().enumerate() {
            bend_of_flange[*old] = index;
        }
        let spec = SheetMetalSpec {
            base_mm: vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]],
            thickness: self.thickness.clone(),
            k_factor: self.k_factor,
            bends: bends.into_iter().map(|(_, bend)| bend).collect(),
        };
        Some((spec, bend_of_flange))
    }
}

/// The parameter path of the bend an old flange path `flanges.N.x` became.
pub(crate) fn bend_path(path: &str, bend_of_flange: &[usize]) -> Option<String> {
    let (index, rest) = path.strip_prefix("flanges.")?.split_once('.')?;
    let bend = bend_of_flange.get(index.parse::<usize>().ok()?)?;
    Some(format!("bends.{bend}.{rest}"))
}

/// The old flange path a bend path `bends.N.x` was read from.
pub(crate) fn flange_path(path: &str, bend_of_flange: &[usize]) -> Option<String> {
    let (index, rest) = path.strip_prefix("bends.")?.split_once('.')?;
    let index = index.parse::<usize>().ok()?;
    let flange = bend_of_flange.iter().position(|bend| *bend == index)?;
    Some(format!("flanges.{flange}.{rest}"))
}
