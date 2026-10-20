//! A DXF drawing read as an underlay: every line, arc and curve it draws, by layer, as
//! polylines, without making profiles of them. An entity it cannot read is left out and
//! counted in the report.

use super::*;
use crate::underlay::{UnderlayLines, UnderlayLinesBuilder};

/// The lines of a DXF drawing and what reading it reported.
#[derive(Clone, Debug, PartialEq)]
pub struct DxfUnderlay {
    pub units: ImportUnitDecision,
    /// The names of the layers of `lines`, in their order.
    pub layers: Vec<String>,
    pub lines: UnderlayLines,
    pub diagnostics: Vec<ImportDiagnostic>,
}

/// Reads every drawn entity of `source` into underlay lines, without the limits of a
/// profile import; run it where memory and time are bounded, in the program worker.
pub fn read_dxf_underlay(
    source: &[u8],
    options: DxfImportOptions,
) -> Result<DxfUnderlay, DxfImportError> {
    let (units, _, parsed, diagnostics) = read_dxf(source, options, UNDERLAY_READING)?;
    let mut builder = UnderlayLinesBuilder::default();
    for loose in &parsed.loose {
        builder.add_loose(&loose.layer, &loose.segment);
    }
    for profile in &parsed.explicit_profiles {
        builder.add(&profile.layer, &profile.segments);
    }
    let (layers, lines) = builder.finish();
    if lines.segment_count() == 0 {
        return Err(DxfImportError::NoSupportedGeometry);
    }
    let diagnostics = finish_report(diagnostics, &parsed.layers, units)?;
    Ok(DxfUnderlay {
        units,
        layers,
        lines,
        diagnostics,
    })
}
