//! Export checks of tapped holes.

use super::*;
use ketchup_geometry::tapping::TappedHole;

/// Refuses a woodWOP program for `part` if one of its drillings is tapped:
/// the drilling macro would make the core hole and lose the thread.
pub(super) fn check_woodwop_taps_nothing(
    machining: &[&GeneralManufacturingOperation],
    part: &str,
) -> Result<(), GeneralFabricationError> {
    match machining
        .iter()
        .find_map(|operation| operation.thread.as_ref())
    {
        Some(thread) => Err(GeneralFabricationError::WoodwopTappedHole {
            part: part.to_owned(),
            thread: thread.label(),
        }),
        None => Ok(()),
    }
}

/// Refuses the drillings of `operations` whose hole no longer takes the
/// thread recorded on it: its size, pitch, tap drill or depth (the tapping
/// rule the hole was made by).
pub(super) fn check_tapped_holes(
    operations: &[GeneralManufacturingOperation],
    snapshot: &Snapshot,
) -> Result<(), GeneralFabricationError> {
    for operation in operations {
        let (
            Some(thread),
            GeneralMachiningGeometry::CircularDrill {
                diameter_mm,
                through,
                start_mm,
                end_mm,
                ..
            },
        ) = (&operation.thread, &operation.machining)
        else {
            continue;
        };
        let hole = TappedHole {
            size: &thread.size,
            pitch_mm: thread.pitch_mm,
            thread_depth_mm: thread.depth_mm,
            hole_diameter_mm: *diameter_mm,
            hole_depth_mm: (end_mm - start_mm).abs(),
            through: *through,
        };
        hole.check(snapshot.tolerance().linear_mm())
            .map_err(|mismatch| GeneralFabricationError::TappedHoleInvalid {
                part: snapshot.definition(operation.definition_id).map_or_else(
                    || operation.definition_id.0.to_string(),
                    |part| part.name().to_owned(),
                ),
                thread: thread.label(),
                mismatch,
            })?;
    }
    Ok(())
}
