//! A `.dwg` drawing imports as the DXF that `ketchup-dwg` converts it to: the
//! review, the plan and the bounded DXF limits are those of a DXF import. The
//! sealed source stays the DWG file's own bytes, so publication still checks
//! that the file on disk is the one reviewed, and the conversion runs again from
//! it. What the DWG reader could not decode is listed in the review, never
//! dropped silently.

use crate::*;
use std::borrow::Cow;

/// What reading a DWG file as DXF reported.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DwgConversion {
    /// The file's version code, for example `AC1032`.
    pub(crate) version: String,
    /// Records the DWG reader could not decode and left out, most frequent first.
    pub(crate) skipped: Vec<ketchup_dwg::SkippedRecords>,
}

pub(crate) fn is_dwg_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dwg"))
}

/// The DXF a drawing import reads from `source`, the bytes of `path`: those
/// bytes, or for a `.dwg` file the DXF they convert to, refused when it would
/// be larger than a DXF file the import reads.
pub(crate) fn drawing_dxf<'a>(
    path: &Path,
    source: &'a [u8],
) -> Result<(Cow<'a, [u8]>, Option<DwgConversion>), ImportError> {
    if !is_dwg_path(path) {
        return Ok((Cow::Borrowed(source), None));
    }
    // The DWG file and its DXF are each bounded like a DXF file.
    let limit = ImportFormat::Dxf.source_limit_bytes();
    let drawing =
        ketchup_dwg::dwg_to_dxf(source, limit, usize::try_from(limit).unwrap_or(usize::MAX))
            .map_err(|error| ImportError::failed(ImportFormat::Dxf, error))?;
    Ok((
        Cow::Owned(drawing.dxf),
        Some(DwgConversion {
            version: drawing.version,
            skipped: drawing.skipped,
        }),
    ))
}

/// A millimetre drawing on three declared layers: an open line and arc, a
/// closed polyline with a bulge and a circle.
#[cfg(test)]
pub(crate) const LAYERED_DRAWING_DXF: &str = concat!(
    "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
    "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n3\n",
    "0\nLAYER\n2\noutline\n70\n0\n62\n7\n6\nCONTINUOUS\n",
    "0\nLAYER\n2\ncut\n70\n0\n62\n1\n6\nCONTINUOUS\n",
    "0\nLAYER\n2\nholes\n70\n0\n62\n3\n6\nCONTINUOUS\n",
    "0\nENDTAB\n0\nENDSEC\n",
    "0\nSECTION\n2\nENTITIES\n",
    "0\nLINE\n8\noutline\n10\n0\n20\n0\n11\n10\n21\n0\n",
    "0\nARC\n8\noutline\n10\n10\n20\n10\n40\n10\n50\n270\n51\n0\n",
    "0\nLWPOLYLINE\n8\ncut\n90\n4\n70\n1\n",
    "10\n0\n20\n0\n42\n0\n10\n10\n20\n0\n42\n1\n10\n10\n20\n10\n42\n0\n10\n0\n20\n10\n42\n0\n",
    "0\nCIRCLE\n8\nholes\n10\n50\n20\n50\n40\n7.5\n",
    "0\nENDSEC\n0\nEOF\n",
);
