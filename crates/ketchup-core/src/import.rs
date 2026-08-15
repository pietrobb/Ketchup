use std::fmt;

use crate::document::{DefinitionId, FeatureId, OccurrenceId};
use crate::graph::sha256_bytes;

pub const IMPORT_RECEIPT_SCHEMA_V1: &str = "ketchup.import-receipt.v1";
pub const MAX_IMPORT_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_IMPORT_DIAGNOSTICS: usize = 1_024;
pub const MAX_IMPORT_OUTPUTS: usize = 1_024;
const MAX_IMPORT_TEXT_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ImportId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportFormat {
    Stl,
    Dxf,
    Step,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportLengthUnit {
    Millimetre,
    Centimetre,
    Metre,
    Inch,
    Foot,
}

impl ImportLengthUnit {
    #[must_use]
    pub const fn millimetres_per_unit(self) -> f64 {
        match self {
            Self::Millimetre => 1.0,
            Self::Centimetre => 10.0,
            Self::Metre => 1_000.0,
            Self::Inch => 25.4,
            Self::Foot => 304.8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportUnitAuthority {
    FileDeclared,
    UserDeclared,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportUnitDecision {
    source_unit: ImportLengthUnit,
    authority: ImportUnitAuthority,
}

impl ImportUnitDecision {
    #[must_use]
    pub const fn new(source_unit: ImportLengthUnit, authority: ImportUnitAuthority) -> Self {
        Self {
            source_unit,
            authority,
        }
    }

    #[must_use]
    pub const fn source_unit(self) -> ImportLengthUnit {
        self.source_unit
    }

    #[must_use]
    pub const fn authority(self) -> ImportUnitAuthority {
        self.authority
    }

    #[must_use]
    pub fn millimetres_per_unit(self) -> f64 {
        self.source_unit.millimetres_per_unit()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ImportDiagnosticSeverity {
    Info,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ImportDiagnostic {
    severity: ImportDiagnosticSeverity,
    code: String,
    subject: Option<String>,
    count: u32,
}

impl ImportDiagnostic {
    pub fn new(
        severity: ImportDiagnosticSeverity,
        code: impl Into<String>,
        subject: Option<String>,
        count: u32,
    ) -> Result<Self, ImportContractError> {
        let code = code.into();
        validate_text(&code)?;
        if subject
            .as_deref()
            .is_some_and(|value| validate_text(value).is_err())
            || count == 0
        {
            return Err(ImportContractError::InvalidDiagnostic);
        }
        Ok(Self {
            severity,
            code,
            subject,
            count,
        })
    }

    #[must_use]
    pub const fn severity(&self) -> ImportDiagnosticSeverity {
        self.severity
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }

    #[must_use]
    pub const fn count(&self) -> u32 {
        self.count
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ImportOutputRef {
    Definition(DefinitionId),
    Feature(FeatureId),
    Occurrence(OccurrenceId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportReceipt {
    schema: String,
    id: ImportId,
    format: ImportFormat,
    source_sha256: [u8; 32],
    source_byte_len: u64,
    source_name: String,
    units: ImportUnitDecision,
    parser_id: String,
    parser_version: String,
    diagnostics: Vec<ImportDiagnostic>,
    outputs: Vec<ImportOutputRef>,
}

impl ImportReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ImportId,
        format: ImportFormat,
        source_sha256: [u8; 32],
        source_byte_len: u64,
        source_name: impl Into<String>,
        units: ImportUnitDecision,
        parser_id: impl Into<String>,
        parser_version: impl Into<String>,
        diagnostics: Vec<ImportDiagnostic>,
        outputs: Vec<ImportOutputRef>,
    ) -> Result<Self, ImportContractError> {
        let receipt = Self {
            schema: IMPORT_RECEIPT_SCHEMA_V1.to_owned(),
            id,
            format,
            source_sha256,
            source_byte_len,
            source_name: source_name.into(),
            units,
            parser_id: parser_id.into(),
            parser_version: parser_version.into(),
            diagnostics,
            outputs,
        };
        receipt.validate()?;
        Ok(receipt)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_source_bytes(
        id: ImportId,
        format: ImportFormat,
        source_bytes: &[u8],
        source_name: impl Into<String>,
        units: ImportUnitDecision,
        parser_id: impl Into<String>,
        parser_version: impl Into<String>,
        diagnostics: Vec<ImportDiagnostic>,
        outputs: Vec<ImportOutputRef>,
    ) -> Result<Self, ImportContractError> {
        Self::new(
            id,
            format,
            sha256_bytes(source_bytes),
            source_bytes.len() as u64,
            source_name,
            units,
            parser_id,
            parser_version,
            diagnostics,
            outputs,
        )
    }

    pub fn validate(&self) -> Result<(), ImportContractError> {
        if self.schema != IMPORT_RECEIPT_SCHEMA_V1 || self.id.0 == 0 {
            return Err(ImportContractError::InvalidIdentity);
        }
        if self.source_byte_len == 0 || self.source_byte_len > MAX_IMPORT_SOURCE_BYTES {
            return Err(ImportContractError::InvalidSource);
        }
        validate_text(&self.source_name)?;
        if self.source_name.contains(['/', '\\']) {
            return Err(ImportContractError::InvalidSource);
        }
        validate_text(&self.parser_id)?;
        validate_text(&self.parser_version)?;
        if self.diagnostics.len() > MAX_IMPORT_DIAGNOSTICS
            || self.diagnostics.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(ImportContractError::DiagnosticsNotCanonical);
        }
        if self.outputs.is_empty()
            || self.outputs.len() > MAX_IMPORT_OUTPUTS
            || self.outputs.windows(2).any(|pair| pair[0] >= pair[1])
            || self.outputs.iter().any(|output| match output {
                ImportOutputRef::Definition(id) => id.0 == 0,
                ImportOutputRef::Feature(id) => id.0 == 0,
                ImportOutputRef::Occurrence(id) => id.0 == 0,
            })
        {
            return Err(ImportContractError::OutputsNotCanonical);
        }
        Ok(())
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn id(&self) -> ImportId {
        self.id
    }

    #[must_use]
    pub const fn format(&self) -> ImportFormat {
        self.format
    }

    #[must_use]
    pub const fn source_sha256(&self) -> &[u8; 32] {
        &self.source_sha256
    }

    #[must_use]
    pub const fn source_byte_len(&self) -> u64 {
        self.source_byte_len
    }

    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    #[must_use]
    pub const fn units(&self) -> ImportUnitDecision {
        self.units
    }

    #[must_use]
    pub fn parser_id(&self) -> &str {
        &self.parser_id
    }

    #[must_use]
    pub fn parser_version(&self) -> &str {
        &self.parser_version
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn outputs(&self) -> &[ImportOutputRef] {
        &self.outputs
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportContractError {
    InvalidIdentity,
    InvalidSource,
    InvalidText,
    InvalidDiagnostic,
    DiagnosticsNotCanonical,
    OutputsNotCanonical,
}

impl fmt::Display for ImportContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "import identity is invalid",
            Self::InvalidSource => "import source identity is invalid",
            Self::InvalidText => "import metadata text is invalid",
            Self::InvalidDiagnostic => "import diagnostic is invalid",
            Self::DiagnosticsNotCanonical => "import diagnostics are not canonical",
            Self::OutputsNotCanonical => "import outputs are not canonical",
        })
    }
}

impl std::error::Error for ImportContractError {}

fn validate_text(value: &str) -> Result<(), ImportContractError> {
    if value.is_empty()
        || value.len() > MAX_IMPORT_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        Err(ImportContractError::InvalidText)
    } else {
        Ok(())
    }
}
