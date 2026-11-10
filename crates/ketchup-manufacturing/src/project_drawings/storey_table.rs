//! The table of storeys against the model (review 2026-11-09 MF-300): a table
//! the model contradicts printed convincing but wrong level marks for the
//! building office, so it is refused; and the storeys found, which fill a new
//! table for the designer to correct.

use super::{
    DrawingSolid, FLOOR_SHARE_OF_LARGEST_LEVEL, Storey, bounds, floor_levels, ground_floor_of,
    plan_overlap, storeys_on, unit_normal, upward_levels, winds_inward,
};
use crate::title_block::StoreyLevels;
use ketchup_geometry::linalg::{cross, sub};
use ketchup_model::tolerance::ROUNDING;
use std::sync::atomic::AtomicBool;

/// How far from a finished floor the model has an upward face at least: a
/// floor finish not modelled lies on its structure, a thicker gap is a table
/// that does not describe the model (not a tolerance: a floor build-up).
pub const NEAR_FLOOR_MM: f64 = 50.0;

/// Why a row of the table of storeys does not describe the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreyTableProblem {
    /// Its structure is over its finished floor, which lies on it.
    StructureAboveFloor,
    /// Its finished floor is that of the storey given (from 1): one plan twice.
    SameFloorAs(usize),
    /// Its finished floor is under the model or not under its top: its plan
    /// would cut nothing.
    OutsideModel,
    /// No face of the model to stand on, one looking up with no solid lying
    /// on all of it, is under its finished floor by [`NEAR_FLOOR_MM`] at most
    /// (a finish lies on its face, not under it).
    NoFloorInModel,
}

/// A row of the table of storeys the model contradicts: the export of the
/// drawings is refused rather than print its levels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreyTableError {
    /// The row, from 1 as the table lists it.
    pub storey: usize,
    pub levels: StoreyLevels,
    pub problem: StoreyTableProblem,
}

impl std::fmt::Display for StoreyTableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            storey,
            levels,
            problem,
        } = self;
        let floor = levels.finished_floor_mm;
        write!(
            formatter,
            "storey {storey} of the table of storeys, its finished floor at {floor} mm, "
        )?;
        match problem {
            StoreyTableProblem::StructureAboveFloor => write!(
                formatter,
                "has its structure above it at {} mm",
                levels.structural_mm
            ),
            StoreyTableProblem::SameFloorAs(other) => {
                write!(formatter, "is the floor of storey {other} again")
            }
            StoreyTableProblem::OutsideModel => {
                formatter.write_str("is under the model or not under its top")
            }
            StoreyTableProblem::NoFloorInModel => write!(
                formatter,
                "has no face of the model to stand on up to {NEAR_FLOOR_MM} mm under it"
            ),
        }
    }
}

impl std::error::Error for StoreyTableError {}

impl From<StoreyTableError> for super::ProjectDrawingError {
    fn from(error: StoreyTableError) -> Self {
        Self::StoreyTable(error)
    }
}

/// Refuses the first row of `table`, in its order, that `solids` contradict;
/// passes any table with no solid to check against (annotations alone).
pub(super) fn check(
    table: &[StoreyLevels],
    solids: &[DrawingSolid],
) -> Result<(), StoreyTableError> {
    contradicted(table, solids).map_or(Ok(()), Err)
}

fn contradicted(table: &[StoreyLevels], solids: &[DrawingSolid]) -> Option<StoreyTableError> {
    let (low, high) = bounds(solids)?;
    let faces = standing_levels(solids)
        .into_iter()
        .filter(|level| level.free_area > level.area * ROUNDING)
        .map(|level| level.height_mm)
        .collect::<Vec<_>>();
    table.iter().enumerate().find_map(|(index, levels)| {
        let floor = f64::from(levels.finished_floor_mm);
        let problem = if levels.structural_mm > levels.finished_floor_mm {
            Some(StoreyTableProblem::StructureAboveFloor)
        } else if let Some(other) = table[..index]
            .iter()
            .position(|below| below.finished_floor_mm == levels.finished_floor_mm)
        {
            Some(StoreyTableProblem::SameFloorAs(other + 1))
        } else if floor < low[2] - NEAR_FLOOR_MM || floor >= high[2] {
            Some(StoreyTableProblem::OutsideModel)
        } else if !faces
            .iter()
            .any(|face| (0.0..=NEAR_FLOOR_MM).contains(&(floor - face)))
        {
            Some(StoreyTableProblem::NoFloorInModel)
        } else {
            None
        };
        problem.map(|problem| StoreyTableError {
            storey: index + 1,
            levels: *levels,
            problem,
        })
    })
}

/// The floors of a model as found from its faces, kept to find its ground
/// floor and its storeys at any plan cut.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelFloors {
    /// The upward levels large enough to be a floor, lowest first, as
    /// [`super::ground_floor`] finds ±0,000 among them.
    pub floors_mm: Vec<f64>,
    /// The floors a storey stands on, lowest first: those of `floors_mm`
    /// measured by what of them no solid lies on, so the top of a ceiling's
    /// lining under the next layer is not the floor of the storey above, the
    /// top of the last layer is (capability review of PR #54).
    pub storey_floors_mm: Vec<f64>,
    /// The top of the model.
    pub top_mm: f64,
}

impl ModelFloors {
    /// The floors of `solids`; none with no solid.
    #[must_use]
    pub fn of(solids: &[DrawingSolid]) -> Option<Self> {
        let (_, high) = bounds(solids)?;
        let standing = standing_levels(solids);
        let largest = standing
            .iter()
            .map(|level| level.free_area)
            .fold(0.0, f64::max);
        Some(Self {
            floors_mm: floor_levels(solids),
            storey_floors_mm: standing
                .into_iter()
                .filter(|level| level.free_area >= largest * FLOOR_SHARE_OF_LARGEST_LEVEL)
                .map(|level| level.height_mm)
                .collect(),
            top_mm: high[2],
        })
    }

    /// Where ±0,000 is by default (see [`super::ground_floor`]).
    #[must_use]
    pub fn ground(&self) -> f64 {
        ground_floor_of(&self.floors_mm)
    }

    /// The storeys found, cut `above_floor_mm` above their floors, as rows of
    /// a table of storeys, each structure at its floor; none for a model with
    /// no floor that far below its top.
    #[must_use]
    pub fn table(&self, above_floor_mm: f64) -> Vec<StoreyLevels> {
        storeys_on(&self.storey_floors_mm, self.top_mm, above_floor_mm)
            .iter()
            .filter_map(|Storey { floor_mm, .. }| {
                // Whole millimetres, as the faces are found; none past i32.
                let floor = floor_mm.and_then(|mm| format!("{mm:.0}").parse::<i32>().ok())?;
                Some(StoreyLevels {
                    finished_floor_mm: floor,
                    structural_mm: floor,
                })
            })
            .collect()
    }
}

/// An upward level of the model and how much of it no solid lies on.
struct StandingLevel {
    height_mm: f64,
    area: f64,
    free_area: f64,
}

/// The upward levels of `solids`, lowest first, each less what lies under the
/// underside of a solid at its height: the top of a layer another lies on,
/// or of a slab under a wall, is no place to stand.
fn standing_levels(solids: &[DrawingSolid]) -> Vec<StandingLevel> {
    // Each underside facing down, its mean height and its triangle in plan
    // counter-clockwise, as `upward_levels` keeps the faces looking up.
    let mut undersides = Vec::new();
    for solid in solids {
        let outward = if winds_inward(solid) { -1.0 } else { 1.0 };
        for triangle in &solid.triangles {
            let [a, b, c] = triangle;
            let Some(normal) = unit_normal(triangle) else {
                continue;
            };
            if normal[2] * outward > -(1.0 - ROUNDING) {
                continue;
            }
            let mut plan = [[a[0], a[1]], [b[0], b[1]], [c[0], c[1]]];
            if cross(sub(*b, *a), sub(*c, *a))[2] < 0.0 {
                plan.swap(1, 2);
            }
            undersides.push(((a[2] + b[2] + c[2]) / 3.0, plan));
        }
    }
    // Nothing can stop it: it is found once, on a worker, for the window.
    let never = AtomicBool::new(false);
    upward_levels(solids)
        .into_iter()
        .map(|(height_mm, level)| {
            // At the same whole millimetre `upward_levels` rounds its levels to.
            let over = undersides
                .iter()
                .filter(|(mean, _)| (mean.round() - height_mm).abs() < 0.5)
                .map(|(_, plan)| *plan)
                .collect::<Vec<_>>();
            let covered = plan_overlap::area_over(&level.triangles, &over, &never).unwrap_or(0.0);
            StandingLevel {
                height_mm,
                area: level.area,
                free_area: level.area - covered,
            }
        })
        .collect()
}
