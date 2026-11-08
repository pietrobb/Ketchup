//! Why an edited drawing is not written into its source, with the facts of each refusal
//! as fields and a text saying what to do.

use std::fmt;

use super::super::{DxfImportError, MAX_DXF_LINE_CHARS};
use super::Difference;

/// Why the edited drawing was not written.
#[derive(Debug)]
pub enum EditedDxfError {
    /// The source drawing does not read as an underlay.
    Read(DxfImportError),
    Refused(DxfEditRefusal),
}

/// Why the changes do not go into the source drawing, with its facts as fields.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DxfEditRefusal {
    /// The source drawing is not the one the underlay was read from: an entity of it is
    /// missing or of another kind.
    NotTheSource,
    /// The source has every entity the underlay was read from, but one moved, stretched
    /// or copied, or every one, reads as other lines: changed in the file since, or read
    /// differently by this Kečup than by the one that imported it (review 2026-10-29
    /// CD-64). Entities not edited that read differently are written as the file has them
    /// (review 2026-11-02 CD-120).
    ReadDifferently,
    /// The source's groups do not pair up into sections and entities.
    Malformed,
    /// The source is larger than its places are kept in: 4 GiB (review 2026-10-29 CD-63).
    TooLarge,
    /// A changed object has no handle, and its entity is not told by its order either.
    NoHandle { object: u32 },
    /// No entity of the source has the handle of a changed object.
    HandleMissing { object: u32, handle: u64 },
    /// A changed object is of a kind, or lies in a plane, that is not mapped yet.
    NotMappable { object: u32, kind: String },
    /// The entity of an object edited in Kečup reads otherwise in the file now (changed
    /// there since it was imported, or read differently by this Kečup), while another
    /// reads as it did: where it lay (`was_mm`) and lies now
    /// (`now_mm`), in the underlay's millimetres. `object` is the one edited, `handle`
    /// and `kind` the entity of the file that holds it, `handle` 0 in a drawing without
    /// handles (review 2026-11-04 r19 CD-163: the
    /// refusal named neither the object nor what to do but import again, losing every
    /// edit).
    ChangedInFile {
        object: u32,
        handle: u64,
        kind: String,
        edit: EditOfObject,
        difference: Difference,
        was_mm: Option<[i64; 2]>,
        now_mm: Option<[i64; 2]>,
    },
    /// The entity of an object edited in Kečup is no longer in the file: deleted there
    /// since the import, where it lay at `was_mm`. One not edited is left out as the file
    /// has it (review 2026-11-04 r19 P3).
    DeletedInFile {
        object: u32,
        handle: u64,
        kind: String,
        edit: EditOfObject,
        was_mm: Option<[i64; 2]>,
    },
    /// More edits than one refused, each of the others' kinds, all named at once
    /// (review 2026-11-06 P3: only the first was, so N of them took N rounds).
    Several(Vec<DxfEditRefusal>),
    /// A text of entity `kind` (`handle` 0 without one), group `code`, would be
    /// `characters` long written as the drawing is `written`, longer than a DXF line
    /// holds. Only an MTEXT's text is split into pieces (review 2026-11-07 UL-220).
    TextTooLong {
        kind: String,
        handle: u64,
        code: i32,
        characters: usize,
        written: TextWritten,
    },
    /// A text drawn in Kečup, beginning `starts`, would be `characters` long written in
    /// a DXF 12 drawing, longer than a DXF line holds, which has no MTEXT to cut it into
    /// pieces (review 2026-11-07 r26 UL-240: the refusal named the handle Kečup had given
    /// the text, found in no file of the user's). In a later drawing, a text drawn or
    /// copied in Kečup, `written` as the drawing is (review 2026-11-08 CD-241: it too was
    /// named by the handle Kečup gave it, and told to save the source as DXF 2007 and
    /// import that, which loses every edit).
    DrawnTextTooLong {
        starts: String,
        characters: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        written: Option<TextWritten>,
    },
}

/// How a drawing writes the characters of its texts, which decides what makes one too
/// long and what helps (review 2026-11-07 r26 P3: every drawing was told of its code
/// page and DXF 2007, a DXF 2007 one and one naming no code page too).
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TextWritten {
    /// Each character as read: DXF 2007 or later (UTF-8), or a drawing naming no code
    /// page; saving it otherwise does not make a text shorter.
    AsRead,
    /// Before DXF 2007, in the code page it is read in, with what the page has not as
    /// `\U+XXXX`.
    CodePage,
    /// Before DXF 2007, with each character beyond ASCII as `\U+XXXX`, as its header
    /// names a code page its texts are not read in.
    Escaped,
}

impl TextWritten {
    /// How a text is written so, said after its length.
    fn how(self) -> &'static str {
        match self {
            Self::AsRead => "",
            Self::CodePage => {
                " written in its code page, with each character the page has not as \\U+XXXX"
            }
            Self::Escaped => {
                " with each character beyond ASCII written as \\U+XXXX, as its header names a \
                 code page its texts are not in"
            }
        }
    }
}

impl DxfEditRefusal {
    /// `refusals` as one: None for none, itself for one, else [`Self::Several`].
    pub(super) fn all(mut refusals: Vec<Self>) -> Option<Self> {
        match refusals.len() {
            0 => None,
            1 => refusals.pop(),
            _ => Some(Self::Several(refusals)),
        }
    }

    /// The objects whose edits it refuses, as their entities changed in the file since,
    /// cannot be told without a handle, have a handle the file has not, or are of a kind
    /// not moved in a DXF, so each edit alone can be discarded (review 2026-11-08 CD-242:
    /// one without a handle had no Discard, and removing it, as told, was refused alike).
    #[must_use]
    pub fn refused_edits(&self) -> Vec<u32> {
        match self {
            Self::ChangedInFile { object, .. }
            | Self::DeletedInFile { object, .. }
            | Self::NoHandle { object }
            | Self::HandleMissing { object, .. }
            | Self::NotMappable { object, .. } => {
                vec![*object]
            }
            Self::Several(refusals) => refusals.iter().flat_map(Self::refused_edits).collect(),
            _ => Vec::new(),
        }
    }
}

/// What was done in Kečup to an object of the underlay.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EditOfObject {
    Moved,
    Stretched,
    Removed,
    Copied,
}

impl EditOfObject {
    fn done(self) -> &'static str {
        match self {
            Self::Moved => "moved",
            Self::Stretched => "stretched",
            Self::Removed => "taken out",
            Self::Copied => "copied",
        }
    }
}

impl Difference {
    fn what(self) -> &'static str {
        match self {
            Self::Layer => "layer is another",
            Self::Lines => "lines lie elsewhere or run otherwise",
            Self::Text => "text lies elsewhere, reads otherwise or is set otherwise",
            Self::Fill => "fill covers another area",
        }
    }
}

/// The entity of kind `kind` and handle `handle`, said.
fn entity(kind: &str, handle: u64) -> String {
    // A DXF handle is never 0: the drawing has none (R12).
    if handle == 0 {
        format!("a {kind} without a handle")
    } else {
        format!("the {kind} of handle {handle:X}")
    }
}

/// `x, y` of a place in millimetres, or that it has none.
fn at(place: Option<[i64; 2]>) -> String {
    place.map_or_else(|| "nowhere".to_owned(), |[x, y]| format!("{x}, {y}"))
}

impl From<DxfEditRefusal> for EditedDxfError {
    fn from(refusal: DxfEditRefusal) -> Self {
        Self::Refused(refusal)
    }
}

impl fmt::Display for EditedDxfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "the source drawing does not read: {error}"),
            Self::Refused(refusal) => refusal.fmt(formatter),
        }
    }
}

impl std::error::Error for EditedDxfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Refused(refusal) => Some(refusal),
        }
    }
}

impl fmt::Display for DxfEditRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTheSource => formatter.write_str(
                "the file is not the drawing the underlay was read from: its entities differ",
            ),
            Self::ReadDifferently => formatter.write_str(
                "the file has the entities the underlay was read from, but the ones moved or \
                 copied in Kečup, or all of them, read as other lines now: either they were \
                 changed in the file since it was imported, or this Kečup reads drawings \
                 differently from the one that imported it; import the drawing again to \
                 write it with changes",
            ),
            Self::Malformed => formatter
                .write_str("the source drawing's groups do not pair up into sections and entities"),
            Self::TooLarge => formatter.write_str("the source drawing is larger than 4 GiB"),
            Self::NoHandle { object } => write!(
                formatter,
                "object {object} has no handle in the drawing, and its entity cannot be told by \
                 its place among the drawing's entities, as the file has gained, lost or \
                 reordered entities since; undo that edit of object {object} (taking it out \
                 is refused alike) to write the drawing with the others, or import the \
                 drawing again"
            ),
            Self::HandleMissing { object, handle } => write!(
                formatter,
                "no entity of the drawing has the handle {handle:X} of object {object}"
            ),
            Self::NotMappable { object, kind } => write!(
                formatter,
                "object {object} is a {kind} that is not moved or copied in a DXF yet; take it \
                 out and draw it again, or leave it where it was read"
            ),
            Self::ChangedInFile {
                object,
                handle,
                kind,
                edit,
                difference,
                was_mm,
                now_mm,
            } => {
                // Where it lies is said apart from what differs (review 2026-11-06 P3).
                let place = if was_mm == now_mm {
                    format!("it still lies at {} mm", at(*was_mm))
                } else {
                    format!(
                        "it lay at {} mm and lies at {} mm now",
                        at(*was_mm),
                        at(*now_mm)
                    )
                };
                write!(
                    formatter,
                    "object {object}, of {}, was {} in Kečup, but its {} in the file now, \
                     changed there since it was imported or read differently by this Kečup; \
                     {place}; undo that edit of object {object} to write the drawing with the \
                     others, or import the drawing again",
                    entity(kind, *handle),
                    edit.done(),
                    difference.what(),
                )
            }
            Self::DeletedInFile {
                object,
                handle,
                kind,
                edit,
                was_mm,
            } => write!(
                formatter,
                "object {object}, of {}, was {} in Kečup, but that entity is no longer in the \
                 file, deleted there since it was imported; it lay at {} mm; undo that edit of \
                 object {object} to write the drawing with the others, or import the drawing \
                 again",
                entity(kind, *handle),
                edit.done(),
                at(*was_mm)
            ),
            // The first few, as a long list reaches the status line and an MCP reply
            // (review 2026-11-06 r23 CD-201); `refused_edits` has them all.
            Self::Several(refusals) => {
                write!(formatter, "{} edits are refused: ", refusals.len())?;
                ketchup_rejection::write_listed(
                    formatter,
                    refusals
                        .iter()
                        .enumerate()
                        .map(|(index, refusal)| format!("{}) {refusal}", index + 1)),
                    "; ",
                )
            }
            Self::TextTooLong {
                kind,
                handle,
                code,
                characters,
                written,
            } => {
                // Imported again, the drawing starts over: the advice says so (review
                // 2026-11-08 CD-241).
                let or = if *written == TextWritten::AsRead {
                    ""
                } else {
                    ", or save the drawing as DXF 2007 or later in the program that made it \
                     and import that, which starts the edits made in Kečup over"
                };
                write!(
                    formatter,
                    "the text in group {code} of {} would be {characters} characters long{}, \
                     longer than the {} a DXF line holds; shorten that text or take it out{or}",
                    entity(kind, *handle),
                    written.how(),
                    MAX_DXF_LINE_CHARS
                )
            }
            Self::DrawnTextTooLong {
                starts,
                characters,
                written: None,
            } => write!(
                formatter,
                "the text \"{starts}…\" drawn in Kečup would be {characters} characters long \
                 in this DXF 12 drawing, longer than the {} a DXF line holds, and DXF 12 has \
                 no multi-line text to cut it into; shorten it or draw it as several texts",
                MAX_DXF_LINE_CHARS
            ),
            Self::DrawnTextTooLong {
                starts,
                characters,
                written: Some(written),
            } => write!(
                formatter,
                "the text \"{starts}…\" drawn or copied in Kečup would be {characters} \
                 characters long{}, longer than the {} a DXF line holds; shorten it, draw it as \
                 several texts or take it out",
                written.how(),
                MAX_DXF_LINE_CHARS
            ),
        }
    }
}

impl std::error::Error for DxfEditRefusal {}
