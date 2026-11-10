//! Export checks of tapped holes.

use super::*;
use ketchup_geometry::tapping::TappedHole;

impl GeneralFabricationProjection {
    /// The support report lines that name what a BTLx file loses, as the BTLx
    /// gate finds it. Of the threads: BTLx 2.3.1 has no tapping processing, so
    /// a thread is only its drilling's comment and user attributes, which a
    /// machine may ignore and drill the core hole alone. Of the pin joints: the
    /// holes in a part the shop does not make (review 2026-11-09 MF-305).
    /// `tapped_drillings=0` and `unmade_pin_holes=0` alone when nothing is lost.
    ///
    /// # Errors
    /// The gate's refusal, as [`Self::btlx_2_3_1_export`] gives it.
    pub fn btlx_loss_report(&self, snapshot: &Snapshot) -> Result<String, GeneralFabricationError> {
        let mut tapped = Vec::new();
        let mut unmade = Vec::new();
        for loss in self.admit(snapshot, ShopFormat::Btlx)? {
            match loss.element {
                ShopElement::Thread(label) => tapped.push(format!("{}: {label}", loss.part)),
                ShopElement::UnmadePinHole(joint) => {
                    unmade.push(format!("{} ({joint})", loss.part))
                }
                _ => {}
            }
        }
        let mut report = format!("tapped_drillings={}\n", tapped.len());
        if !tapped.is_empty() {
            report.push_str(&format!(
                "thread_loss={}: {}\n",
                super::capabilities::BTLX_THREAD_LOSS,
                tapped.join("; ")
            ));
        }
        report.push_str(&format!("unmade_pin_holes={}\n", unmade.len()));
        if !unmade.is_empty() {
            report.push_str(&format!(
                "unmade_pin_hole_loss={}: {}\n",
                super::capabilities::UNMADE_PIN_HOLE,
                unmade.join("; ")
            ));
        }
        Ok(report)
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
