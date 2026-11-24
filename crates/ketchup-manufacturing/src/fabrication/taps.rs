//! Export checks of tapped holes, and the support report of a BTLx file:
//! what it carries and what it loses.

use super::*;
use ketchup_geometry::tapping::TappedHole;

impl GeneralFabricationProjection {
    /// The support report written beside a BTLx file made with `options`:
    /// the profile strategy, what a machine may not make, the losses of
    /// [`Self::btlx_loss_report`] and the drill lines of
    /// [`Self::btlx_drill_report`]. The BTLx export and the workshop package
    /// write this one report (review 2026-11-10 MF-400).
    ///
    /// # Errors
    /// The gate's refusal, as [`Self::btlx_2_3_1_export`] gives it.
    pub fn btlx_support_report(
        &self,
        snapshot: &Snapshot,
        options: BtlxExportOptions,
    ) -> Result<String, GeneralFabricationError> {
        let (selected_profile_request, intermediate_saw_cuts, selected_profile_contours) =
            match options.profile_processing_request {
                BtlxProfileProcessingRequest::PortableFreeContour => (
                    "portable FreeContour",
                    "not applicable".to_owned(),
                    "closed simple line/arc contours",
                ),
                BtlxProfileProcessingRequest::EdgeSawCutsThenMillContour {
                    intermediate_saw_cuts,
                } => (
                    "edge SawContour cuts, then MillContour",
                    intermediate_saw_cuts.to_string(),
                    "rectangles sawn on their edges then milled; contours with an arc milled whole",
                ),
            };
        Ok(format!(
            "schema=ketchup.btlx-support-report.v1\nsource_digest={}\nformat=BTLx 2.3.1\nstock=rectangular straight timber\ndrilling=circular\nportable_profile_removals=pocket, through-cut, extruded boolean-cut\nportable_profile_contours=closed simple line/arc contours\ndefault_profile_request=edge SawContour cuts, then MillContour\nselected_profile_request={selected_profile_request}\nintermediate_saw_cuts={intermediate_saw_cuts}\nselected_profile_contours={selected_profile_contours}\nunsupported=straight-edged non-rectangular profile with the edge-saw request; non-rectangular stock; other machining operations\nconcrete_importer_verified=false\nmachine_execution_order_guaranteed=false\n{}{}",
            snapshot.canonical_digest(),
            self.btlx_loss_report(snapshot)?,
            self.btlx_drill_report(snapshot, options.largest_drill_mm)
        ))
    }

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
