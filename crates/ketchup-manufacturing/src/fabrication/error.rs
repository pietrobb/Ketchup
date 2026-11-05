//! The errors of fabrication exports, and why an export is refused
//! (review 2026-11-04 r19 P3): each refusal carries its cause, so the user is
//! told what to do instead of one text for every cause.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportBlock {
    /// The projection was made from another revision of the document (or by
    /// another evaluator).
    AnotherState,
    /// The projection's own result no longer matches what it records: its
    /// rows, drawings or operations disagree with each other or with its digest.
    ResultMismatch,
    /// A body or row has not passed validation.
    NotValidated,
    /// The export has no part or operation to write.
    NothingToWrite,
    /// A cut of a part could not be described as machining.
    UndescribedCuts,
    /// A part is not one exact body (a mesh, or an unverified result).
    NotExactBody,
    /// The stock is not a rectangular box, or a cut is not one the format
    /// describes, reaches outside the stock or starts inside the material.
    UnsupportedMachining,
    /// A copy is placed by a transform other than a rotation and a move, or
    /// resolves to another definition than its row.
    UnusablePlacement,
    /// A pin joint drills a piece the export does not make.
    UnmadePinHole,
    /// A dimension, count or name is out of what the format can write.
    ValueOutOfRange,
    /// Two pieces would be written under one name (instance, production code
    /// or adapter).
    DuplicateName,
}

impl fmt::Display for ExportBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AnotherState => {
                "the fabrication result was made from another state of the document; evaluate the model again"
            }
            Self::ResultMismatch => {
                "the fabrication result no longer matches the model it was made from; evaluate the model again"
            }
            Self::NotValidated => {
                "a body has not passed validation; check the model and evaluate it again"
            }
            Self::NothingToWrite => "there is no part or machining this file writes",
            Self::UndescribedCuts => {
                "a cut of it could not be described as machining (an unevenly scaled part, a tool that is not a drilling, a pocket or a profile cut, or a cut that starts inside the material: a cavity, or a cut that opens only to a side and is not rectangular); let each cut tool reach past the face it enters"
            }
            Self::NotExactBody => {
                "it is not one exact body (a mesh, or a result not verified); model it as a solid and evaluate it again"
            }
            Self::UnsupportedMachining => {
                "its stock is not a rectangular box, or a cut is not one the format describes, reaches outside the stock or starts inside the material (a cavity, or a cut that opens only to a side and is not rectangular); let each cut tool reach past the face it enters"
            }
            Self::UnusablePlacement => {
                "a copy is not placed by a rotation and a move alone, or is no longer a copy of this part; evaluate the model again"
            }
            Self::UnmadePinHole => {
                "a pin joint drills a piece this file does not make (a purchased or site-built part, or one outside the production scope); make that part or remove the pin joint"
            }
            Self::ValueOutOfRange => {
                "a dimension, count or name is out of what the file can write (zero, too large, or not plain ASCII)"
            }
            Self::DuplicateName => {
                "two pieces would be written under one name; give every piece its own production code"
            }
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GeneralFabricationError {
    ValidationBindingMismatch,
    Classification(ClassificationError),
    TimberMemberRoleMissing,
    TimberMemberRoleAmbiguous,
    InvalidBomMetadata,
    /// A made part with a body has no fabrication role, so no BOM row or
    /// machine file would make it.
    PartWithoutFabricationRole {
        part: String,
    },
    /// A made part's fabrication role is a name no fabrication knows.
    UnknownFabricationRole {
        part: String,
        role: String,
    },
    UnsupportedOrUnavailableGeometry,
    /// `part` has no current exact body or supported geometry to fabricate
    /// from (review 2026-11-01 MF-106).
    PartGeometryUnavailable {
        part: String,
    },
    BodyValidation(GeneralBodyValidationError),
    InvalidGeometry,
    ExactGraph(ExactBRepGraphError),
    InvalidWeldmentGeometry,
    /// No part of the production scope is made with a body, so there is
    /// nothing to fabricate (review 2026-11-04 r19 P3).
    NoMadePart,
    /// No made piece is drilled or pocketed, so a woodWOP package has no
    /// program (review 2026-11-04 r19 P3).
    NoPieceToProgram,
    /// The export is refused for `0` (review 2026-11-04 r19 P3).
    ExportBlocked(ExportBlock),
    UnresolvedInstance(CanonicalError),
    PinJoint(PinJointError),
    /// The chosen BTLx profile strategy cannot cut a contour of this part.
    BtlxProfileRequestUnsupported {
        part: String,
    },
    ScaledPlacement(ScaledPlacement),
    /// The machine file cannot be written for `part`, for `reason`.
    PartExportBlocked {
        part: String,
        reason: ExportBlock,
    },
    /// A production job cannot lay `part`: it is not one exact body with a
    /// stock blank the job can lay (review 2026-11-01 MF-106).
    PartJobBlocked {
        part: String,
    },
    /// A made piece of `part` has no production code, so a production job
    /// cannot name it.
    PartWithoutProductionCode {
        part: String,
    },
    /// A HOMAG program of `part` is named by its piece's production code, and
    /// `code` (or none) is not [`is_homag_program_code`] (review 2026-11-02 MF-120).
    HomagProgramCodeUnusable {
        part: String,
        code: Option<String>,
    },
    /// `part` has no drilling or pocket, so a woodWOP program of it would only
    /// lay its blank (review 2026-11-02 MF-124).
    NothingToMachine {
        part: String,
    },
    /// The vertical pocket tool asked for is not a woodWOP tool number
    /// (1 to 999 999, review 2026-11-04 MF-123).
    PocketToolNumberInvalid {
        tool: u32,
    },
    /// `part` is drilled into both its top and its bottom face, and one
    /// woodWOP setup machines one face and the edges.
    DrilledFromBothFaces {
        part: String,
    },
    /// A hole of `part` cannot be written as a BTLx drilling, for `reason`.
    BtlxDrillingRefused {
        part: String,
        reason: BtlxDrillRefusal,
    },
    /// `format` cannot carry `element` of `part`, for `reason` (the
    /// capability table, [`carriage`]).
    CannotCarry {
        format: ShopFormat,
        part: String,
        element: ShopElement,
        reason: &'static str,
    },
    /// A drilling of `part` does not take the `thread` recorded on its hole
    /// (the hole or the thread changed after it was tapped), so no machine
    /// file, job or drawing may name that thread.
    TappedHoleInvalid {
        part: String,
        thread: String,
        mismatch: TappedHoleMismatch,
    },
    /// One woodWOP program machines one part, and `parts` different parts
    /// are made: the production package writes one program per piece.
    OneProgramPerPart {
        parts: usize,
    },
    /// The production package could not be packed into one ZIP.
    Package(ketchup_model::exact_product::ExactProductError),
}

impl GeneralFabricationError {
    /// Names `part` in a blocked export, so the user knows what to check.
    #[must_use]
    pub fn at_part(self, part: &str) -> Self {
        match self {
            Self::ExportBlocked(reason) => Self::PartExportBlocked {
                part: part.to_owned(),
                reason,
            },
            Self::DrilledFromBothFaces { .. } => Self::DrilledFromBothFaces {
                part: part.to_owned(),
            },
            Self::UnsupportedOrUnavailableGeometry => Self::PartGeometryUnavailable {
                part: part.to_owned(),
            },
            other => other,
        }
    }
}

impl fmt::Display for GeneralFabricationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnresolvedInstance(error) => write!(formatter, "a fabrication instance does not resolve: {error}"),
            Self::PinJoint(error) => write!(formatter, "a pin joint cannot be exported: {error}"),
            Self::ExactGraph(error) => write!(formatter, "fabrication body exact graph is unavailable: {error}"),
            Self::BodyValidation(error) => write!(formatter, "a visible body cannot be validated: {error}"),
            Self::ValidationBindingMismatch => formatter.write_str(
                "general fabrication requires current complete general-body validation coverage",
            ),
            Self::Classification(error) => {
                write!(formatter, "fabrication roles or BOM materials cannot be read: {error}")
            }
            Self::TimberMemberRoleMissing => {
                formatter.write_str("the timber-member fabrication role is missing")
            }
            Self::TimberMemberRoleAmbiguous => {
                formatter.write_str("the timber-member fabrication role is ambiguous")
            }
            Self::InvalidBomMetadata => {
                formatter.write_str("a BOM role or material token is invalid")
            }
            Self::PartWithoutFabricationRole { part } => write!(
                formatter,
                "part \"{part}\" has no fabrication role, so it would be in no BOM row or machine file; make it a member, a manufactured or a purchased item, hide it, or leave it out of the production scope",
            ),
            Self::UnknownFabricationRole { part, role } => write!(
                formatter,
                "part \"{part}\" has the fabrication role \"{role}\", which no fabrication knows; make it a member, a manufactured or a purchased item",
            ),
            Self::UnsupportedOrUnavailableGeometry => formatter.write_str(
                "a visible geometry-bearing occurrence has unsupported or unavailable evidence",
            ),
            Self::PartGeometryUnavailable { part } => write!(
                formatter,
                "part \"{part}\" has no current exact body or supported geometry to fabricate from; evaluate the model again",
            ),
            Self::InvalidGeometry => {
                formatter.write_str("accepted fabrication geometry has invalid local bounds")
            }
            Self::InvalidWeldmentGeometry => formatter.write_str(
                "weldment cut lists require supported straight members and unambiguous joints",
            ),
            Self::NoMadePart => formatter.write_str(
                "there is no part to make: no part in the production scope is a member, a manufactured or a purchased item with a body; give a part a fabrication role, or widen the production scope",
            ),
            Self::NoPieceToProgram => formatter.write_str(
                "there is no piece to program: a woodWOP program is written for each made piece with a drilling or a pocket, and no piece has one; cut the parts from the cut list instead",
            ),
            Self::ExportBlocked(reason) => {
                write!(formatter, "the fabrication export is refused: {reason}")
            }
            Self::BtlxProfileRequestUnsupported { part } => write!(
                formatter,
                "the requested BTLx profile strategy cannot cut a contour of part \"{part}\": edge saw cuts then milling takes only rectangular line contours with at most 32 intermediate cuts; choose the portable FreeContour strategy for this part",
            ),
            Self::ScaledPlacement(error) => write!(formatter, "a part cannot be fabricated: {error}"),
            Self::PartExportBlocked { part, reason } => write!(
                formatter,
                "part \"{part}\" cannot be written to the machine file: {reason}",
            ),
            Self::PartJobBlocked { part } => write!(
                formatter,
                "a production job cannot lay part \"{part}\": it lays one exact body of each part with the stock blank it is cut from, and this part has none it can lay; evaluate the model again",
            ),
            Self::PartWithoutProductionCode { part } => write!(
                formatter,
                "part \"{part}\" has no production code, so a production job cannot name its piece; give every made piece a code and save the document",
            ),
            Self::HomagProgramCodeUnusable { part, code } => {
                match code {
                    None => write!(formatter, "part \"{part}\" has no production code")?,
                    Some(code) => write!(
                        formatter,
                        "part \"{part}\" has the production code \"{code}\""
                    )?,
                }
                write!(
                    formatter,
                    ", and a HOMAG program and its barcode label are named by a code of exactly 12 letters, digits, '-' or '_'; assign production codes (Tools > Assign production codes, or the model operation set_production_code) and save the document",
                )
            }
            Self::NothingToMachine { part } => write!(
                formatter,
                "part \"{part}\" has no drilling or pocket, so a woodWOP program would only lay its blank; give it holes or pockets, or cut it from the cut list",
            ),
            Self::PocketToolNumberInvalid { tool } => write!(
                formatter,
                "vertical pocket tool {tool} is not a woodWOP tool number; give the number of the pocket tool in the machine's tool list, 1 to 999999",
            ),
            Self::BtlxDrillingRefused { part, reason } => write!(
                formatter,
                "a hole of part \"{part}\" cannot be written as a BTLx drilling: {reason}",
            ),
            Self::DrilledFromBothFaces { part } => write!(
                formatter,
                "part \"{part}\" is drilled into both its top and its bottom face, and a woodWOP program machines one face and the edges in one setup; export a production job, or drill the part from one face",
            ),
            Self::CannotCarry {
                format,
                part,
                element,
                reason,
            } => write!(
                formatter,
                "part \"{part}\" has {element}, which {format} cannot carry: {reason}",
            ),
            Self::TappedHoleInvalid {
                part,
                thread,
                mismatch,
            } => write!(
                formatter,
                "part \"{part}\" has a hole that does not take its thread {thread}: {mismatch}; tap the hole again or change the thread",
            ),
            Self::OneProgramPerPart { parts } => write!(
                formatter,
                "{parts} different parts are made, and one HOMAG MPR program machines one part; export the HOMAG MPR to a .zip destination for one program and barcode label per piece, named by its production code",
            ),
            Self::Package(error) => write!(formatter, "the production package was not packed: {error}"),
        }
    }
}

impl std::error::Error for GeneralFabricationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnresolvedInstance(error) => Some(error),
            Self::PinJoint(error) => Some(error),
            Self::ExactGraph(error) => Some(error),
            Self::BodyValidation(error) => Some(error),
            Self::Classification(error) => Some(error),
            Self::Package(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ClassificationError> for GeneralFabricationError {
    fn from(error: ClassificationError) -> Self {
        Self::Classification(error)
    }
}

impl From<GeneralBodyValidationError> for GeneralFabricationError {
    fn from(error: GeneralBodyValidationError) -> Self {
        Self::BodyValidation(error)
    }
}
