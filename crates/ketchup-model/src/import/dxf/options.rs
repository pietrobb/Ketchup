//! How a DXF drawing is read: the unit of one that declares none, the geo origin an
//! underlay moves by and the code page the user chose.

use super::{DxfCodePage, ImportLengthUnit};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DxfImportOptions {
    unit_if_unitless: Option<ImportLengthUnit>,
    /// The bits of the document's geo origin an underlay moves by when it can.
    geo_origin_mm_bits: Option<[u64; 2]>,
    /// The code page the user chose to read a drawing before DXF 2007 in.
    code_page: Option<DxfCodePage>,
}

impl DxfImportOptions {
    #[must_use]
    pub const fn new(unit_if_unitless: Option<ImportLengthUnit>) -> Self {
        Self {
            unit_if_unitless,
            geo_origin_mm_bits: None,
            code_page: None,
        }
    }

    /// The drawing's text is read in `code_page`, whatever its header names; a drawing of
    /// DXF 2007 or later is refused it ([`super::DxfImportError::CodePageOfUtf8Drawing`]).
    #[must_use]
    pub const fn with_code_page(self, code_page: Option<DxfCodePage>) -> Self {
        Self { code_page, ..self }
    }

    #[must_use]
    pub const fn code_page(self) -> Option<DxfCodePage> {
        self.code_page
    }

    /// Read as an underlay, the drawing moves by `origin_mm` when that leaves out no more
    /// of it than its own median ([`crate::underlay::placement::geo_origin`]).
    #[must_use]
    pub fn with_geo_origin(self, origin_mm: Option<[f64; 2]>) -> Self {
        Self {
            geo_origin_mm_bits: origin_mm.map(|origin| origin.map(f64::to_bits)),
            ..self
        }
    }

    #[must_use]
    pub const fn unit_if_unitless(self) -> Option<ImportLengthUnit> {
        self.unit_if_unitless
    }

    #[must_use]
    pub fn geo_origin_mm(self) -> Option<[f64; 2]> {
        self.geo_origin_mm_bits.map(|bits| bits.map(f64::from_bits))
    }
}
