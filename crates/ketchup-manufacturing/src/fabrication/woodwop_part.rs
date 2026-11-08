//! The macros of the woodWOP program of one part, shared by the program of one
//! part and each program of the production package (review 2026-11-08 MF-140).

use super::*;

/// The stock frame and macros of the woodWOP program of one part: its
/// drillings and pockets and the holes of its logical pin joints, laid as
/// [`woodwop_setup`] lays them. The program of one part and each program of
/// the production package are written by it alike (review 2026-11-08 MF-140:
/// the program of one part left the pin holes out).
pub(super) fn woodwop_part_macros(
    stock_frame: WoodwopStockFrame,
    machining: &[&GeneralManufacturingOperation],
    holes: &[&PinHole],
    pocket_tool: Option<u32>,
) -> Result<(WoodwopStockFrame, Vec<String>), GeneralFabricationError> {
    woodwop_setup(
        stock_frame,
        &machining_normals(
            machining.iter().copied(),
            holes.iter().map(|hole| hole.inward_unit_local),
        ),
        |stock_frame| {
            let mut macros = machining
                .iter()
                .map(|operation| match operation.kind {
                    GeneralManufacturingKind::CircularDrill => {
                        woodwop_drilling_macro(&operation.machining, stock_frame)
                    }
                    GeneralManufacturingKind::ProfileCut => pocket_tool.and_then(|tool| {
                        woodwop_vertical_pocket_macro(&operation.machining, stock_frame, tool)
                    }),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            for hole in holes {
                macros.push(woodwop_pin_macro(hole, stock_frame)?);
            }
            Some(macros)
        },
    )
}

/// Whether two copies are drilled for pin joints alike, in their own frames.
pub(super) fn same_pin_holes(first: &[&PinHole], other: &[&PinHole]) -> bool {
    let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= ROUNDING);
    first.len() == other.len()
        && first.iter().all(|hole| {
            other.iter().any(|copy| {
                close(&hole.entry_local_mm, &copy.entry_local_mm)
                    && close(&hole.inward_unit_local, &copy.inward_unit_local)
                    && close(
                        &[hole.diameter_mm, hole.depth_mm],
                        &[copy.diameter_mm, copy.depth_mm],
                    )
            })
        })
}
