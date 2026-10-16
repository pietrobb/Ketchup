//! Why a stored payload (a CBOR snapshot, a program's JSON) did not decode, as fields:
//! which part of the document, what kind of failure and where in its bytes.
use std::fmt;
use std::io;

use crate::document::FeatureId;

/// The part of a document whose payload did not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadSection {
    /// The CBOR snapshot of a revision (`document.bin` or a history chunk).
    Snapshot,
    /// The format-100 sheet-metal record of this feature.
    SheetMetal(FeatureId),
    /// The JSON of a revision's rule program.
    RuleProgram,
    /// The digests a pre-serde document stored, rewritten to the current form.
    LegacyDigests,
}

/// Where in a payload a decoding failure was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadLocation {
    /// Byte offset into the CBOR payload.
    Byte(usize),
    /// One-based line and column of the JSON text.
    Text { line: usize, column: usize },
    /// The decoder did not say where.
    Unknown,
}

/// What went wrong decoding a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadError {
    /// Reading the bytes failed with this kind of error.
    Read(io::ErrorKind),
    /// The bytes end inside a value.
    Truncated,
    /// The bytes are not well-formed CBOR or JSON.
    Syntax(PayloadLocation),
    /// The bytes are well formed, but a value does not fit the model. `detail` is
    /// what the decoder says about it (the field or value and what was expected); serde
    /// reports that only as text.
    Data { at: PayloadLocation, detail: String },
    /// Values nest deeper than the reader allows.
    TooDeep,
    /// A sheet-metal flange names a side the format does not have.
    UnknownFlangeSide,
}

impl From<ciborium::de::Error<io::Error>> for PayloadError {
    fn from(error: ciborium::de::Error<io::Error>) -> Self {
        use ciborium::de::Error;
        match error {
            Error::Io(error) if error.kind() == io::ErrorKind::UnexpectedEof => Self::Truncated,
            Error::Io(error) => Self::Read(error.kind()),
            Error::Syntax(offset) => Self::Syntax(PayloadLocation::Byte(offset)),
            Error::Semantic(offset, detail) => Self::Data {
                at: offset.map_or(PayloadLocation::Unknown, PayloadLocation::Byte),
                detail,
            },
            Error::RecursionLimitExceeded => Self::TooDeep,
        }
    }
}

impl From<ciborium::value::Error> for PayloadError {
    fn from(error: ciborium::value::Error) -> Self {
        let ciborium::value::Error::Custom(detail) = error;
        Self::Data {
            at: PayloadLocation::Unknown,
            detail,
        }
    }
}

impl From<serde_json::Error> for PayloadError {
    fn from(error: serde_json::Error) -> Self {
        use serde_json::error::Category;
        let at = match (error.line(), error.column()) {
            (0, _) => PayloadLocation::Unknown,
            (line, column) => PayloadLocation::Text { line, column },
        };
        match error.classify() {
            Category::Io => Self::Read(error.io_error_kind().unwrap_or(io::ErrorKind::Other)),
            Category::Eof => Self::Truncated,
            Category::Syntax => Self::Syntax(at),
            Category::Data => {
                // serde_json appends the location to its message; it is kept in `at`.
                let message = error.to_string();
                let detail = match at {
                    PayloadLocation::Text { line, column } => message
                        .strip_suffix(&format!(" at line {line} column {column}"))
                        .map(str::to_owned)
                        .unwrap_or(message),
                    _ => message,
                };
                Self::Data { at, detail }
            }
        }
    }
}

impl fmt::Display for PayloadSection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot => formatter.write_str("the model snapshot"),
            Self::SheetMetal(feature) => {
                write!(formatter, "the sheet-metal record of feature {}", feature.0)
            }
            Self::RuleProgram => formatter.write_str("a revision's rule program"),
            Self::LegacyDigests => formatter.write_str("the digests of an older document"),
        }
    }
}

impl fmt::Display for PayloadLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Byte(offset) => write!(formatter, " at byte {offset}"),
            Self::Text { line, column } => write!(formatter, " at line {line}, column {column}"),
            Self::Unknown => Ok(()),
        }
    }
}

impl fmt::Display for PayloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(kind) => write!(formatter, "could not be read ({kind})"),
            Self::Truncated => formatter.write_str("ends inside a value"),
            Self::Syntax(at) => write!(formatter, "is not well formed{at}"),
            Self::Data { at, detail } => {
                write!(formatter, "holds a value the model rejects{at}: {detail}")
            }
            Self::TooDeep => formatter.write_str("nests values deeper than a document may"),
            Self::UnknownFlangeSide => {
                formatter.write_str("names a flange side the format does not have")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::{PersistenceError, decode_rule_program, snapshot_codec};

    fn snapshot_error(payload: &[u8]) -> PersistenceError {
        let mut body = crate::graph::sha256_bytes(payload).to_vec();
        body.extend_from_slice(payload);
        match snapshot_codec::decode(snapshot_codec::SNAPSHOT_FORMAT, &body) {
            Err(error) => error,
            Ok(_) => panic!("{payload:?} decoded as a snapshot"),
        }
    }

    /// Review PS-8: a payload that does not decode says which part of the document
    /// failed, how, and where in its bytes, as fields rather than a sentence.
    #[test]
    fn a_payload_that_does_not_decode_names_its_section_kind_and_place() {
        // 0x1c uses a reserved length code, so the snapshot is malformed at byte 0.
        assert_eq!(
            snapshot_error(&[0x1c]),
            PersistenceError::InvalidPayload {
                section: PayloadSection::Snapshot,
                error: PayloadError::Syntax(PayloadLocation::Byte(0)),
            }
        );
        // A map of one entry whose key text ends after its header ends early.
        assert_eq!(
            snapshot_error(&[0xa1, 0x68]),
            PersistenceError::InvalidPayload {
                section: PayloadSection::Snapshot,
                error: PayloadError::Truncated,
            }
        );
        // Well-formed CBOR that is not a snapshot is rejected by the model, with
        // serde's reason.
        assert!(matches!(
            snapshot_error(&[0x01]),
            PersistenceError::InvalidPayload {
                section: PayloadSection::Snapshot,
                error: PayloadError::Data { detail, .. },
            } if detail.contains("integer")
        ));

        // A program's JSON names the line and column of malformed text.
        assert!(matches!(
            decode_rule_program(b"{\n  \"file_name\" 5\n}").unwrap_err(),
            PersistenceError::InvalidPayload {
                section: PayloadSection::RuleProgram,
                error: PayloadError::Syntax(PayloadLocation::Text { line: 2, .. }),
            }
        ));
        let wrong_type = decode_rule_program(b"{\"file_name\": 5}").unwrap_err();
        assert!(matches!(
            &wrong_type,
            PersistenceError::InvalidPayload {
                section: PayloadSection::RuleProgram,
                error: PayloadError::Data { detail, .. },
            } if detail.contains("expected a string")
        ));
        // A located JSON reason keeps the place in fields and does not repeat it.
        assert_eq!(
            PayloadError::from(serde_json::from_str::<String>("\n5").unwrap_err()),
            PayloadError::Data {
                at: PayloadLocation::Text { line: 2, column: 1 },
                detail: "invalid type: integer `5`, expected a string".to_owned(),
            }
        );
        assert_eq!(
            decode_rule_program(b"{\"file_name\": ").unwrap_err(),
            PersistenceError::InvalidPayload {
                section: PayloadSection::RuleProgram,
                error: PayloadError::Truncated,
            }
        );
        assert!(
            wrong_type
                .to_string()
                .starts_with("a revision's rule program of the document holds a value"),
            "{wrong_type}"
        );
    }
}
