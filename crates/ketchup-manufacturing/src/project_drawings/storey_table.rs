//! The table of storeys against the model (review 2026-11-09 MF-300): a table
//! the model contradicts printed convincing but wrong level marks for the
//! building office, so it is refused; and the storeys found, which fill a new
//! table for the designer to correct.

use super::{
    DrawingSolid, Storey, bounds, floor_levels, ground_floor_of, storeys_on, upward_levels,
};
use crate::title_block::StoreyLevels;

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
    /// No face of the model looks up within [`NEAR_FLOOR_MM`] of its finished
    /// floor.
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
                "has no face of the model looking up within {NEAR_FLOOR_MM} mm of it"
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
    let faces = upward_levels(solids)
        .into_iter()
        .map(|(height, _)| height)
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
            .any(|face| (face - floor).abs() <= NEAR_FLOOR_MM)
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
    /// The upward levels large enough to be a floor, lowest first.
    pub floors_mm: Vec<f64>,
    /// The top of the model.
    pub top_mm: f64,
}

impl ModelFloors {
    /// The floors of `solids`; none with no solid.
    #[must_use]
    pub fn of(solids: &[DrawingSolid]) -> Option<Self> {
        let (_, high) = bounds(solids)?;
        Some(Self {
            floors_mm: floor_levels(solids),
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
        storeys_on(&self.floors_mm, self.top_mm, above_floor_mm)
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
