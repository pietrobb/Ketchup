use crate::tolerance::MAX_COORDINATE_MM;
use std::fmt;

/// A length as the user wrote it and its value in millimetres.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Dimension {
    source_token: String,
    millimetres: f64,
}

impl Dimension {
    pub fn new(source_token: impl Into<String>, millimetres: f64) -> Result<Self, DimensionError> {
        let source_token = source_token.into();
        if source_token.trim().is_empty() {
            return Err(DimensionError::EmptySourceToken);
        }
        if !millimetres.is_finite() || millimetres.abs() > MAX_COORDINATE_MM {
            return Err(DimensionError::OutsideEnvelope);
        }
        Ok(Self {
            source_token,
            millimetres,
        })
    }

    pub fn from_decimal(source_token: impl Into<String>) -> Result<Self, DimensionError> {
        let source_token = source_token.into();
        let millimetres = source_token
            .parse::<f64>()
            .map_err(|_: std::num::ParseFloatError| DimensionError::InvalidDecimalToken)?;
        Self::new(source_token, millimetres)
    }

    #[must_use]
    pub fn source_token(&self) -> &str {
        &self.source_token
    }

    #[must_use]
    pub const fn millimetres(&self) -> f64 {
        self.millimetres
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DimensionError {
    EmptySourceToken,
    InvalidDecimalToken,
    OutsideEnvelope,
}

impl fmt::Display for DimensionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptySourceToken => "dimension source token is empty",
            Self::InvalidDecimalToken => "dimension source token is not decimal",
            Self::OutsideEnvelope => "dimension is outside the canonical coordinate envelope",
        })
    }
}

impl std::error::Error for DimensionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_keeps_its_source_token_and_rejects_bad_input() {
        let dimension = Dimension::from_decimal("12.5").unwrap();
        assert_eq!(
            (dimension.source_token(), dimension.millimetres()),
            ("12.5", 12.5)
        );
        assert_eq!(
            Dimension::new(" ", 1.0),
            Err(DimensionError::EmptySourceToken)
        );
        assert_eq!(
            Dimension::from_decimal("12 mm"),
            Err(DimensionError::InvalidDecimalToken)
        );
        assert_eq!(
            Dimension::new("far", MAX_COORDINATE_MM * 2.0),
            Err(DimensionError::OutsideEnvelope)
        );
        assert_eq!(
            Dimension::new("nan", f64::NAN),
            Err(DimensionError::OutsideEnvelope)
        );
    }
}
