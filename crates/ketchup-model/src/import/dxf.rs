use crate::tolerance::{ACCUMULATED_ROUNDING, MAX_COORDINATE_MM, ROUNDING};
use ketchup_geometry::linalg::cross2;
use ketchup_tolerance::limits;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, FeatureKind, OccurrenceId,
    ProfileSegment, ProposalBudget, Snapshot, Transform,
};

use super::{
    ImportContractError, ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat,
    ImportLengthUnit, ImportOutputRef, ImportReceipt, ImportUnitAuthority, ImportUnitDecision,
    MAX_IMPORT_DIAGNOSTICS,
};

pub const DXF_PARSER_ID: &str = "ketchup-dxf";
pub const DXF_PARSER_VERSION: &str = "30";
const MAX_DXF_LINE_BYTES: usize = 512;
/// The most group-code pairs a drawing import reads; a DWG conversion with more is
/// refused in the worker, before the window reads its DXF.
pub const MAX_DXF_PAIRS: usize = limits::DRAWING_PAIRS;
/// Entities read, those placed by block inserts included.
pub const MAX_DXF_ENTITIES: usize = limits::DRAWING_ENTITIES;
/// Profiles one import makes: three commands each and the receipt fit the import's own
/// budget, [`ProposalBudget::DRAWING_IMPORT`], not that of one edit.
pub const MAX_DXF_PROFILES: usize = limits::DRAWING_PROFILES;
const _: () = assert!(3 * MAX_DXF_PROFILES < ProposalBudget::DRAWING_IMPORT.max_commands);
const MAX_DXF_SEGMENTS_PER_PROFILE: usize = 1_024;
const MAX_DXF_BLOCK_DEPTH: usize = 32;
const DXF_GEOMETRY_EPSILON_MM: f64 = ROUNDING;

/// What one reading of a DXF keeps to. A profile import turns what it reads into canonical
/// profiles in the window and keeps their bounded limits. An underlay is read in the program
/// worker, whose memory and time limits bound it, and keeps every line it can read. Either
/// leaves out, and counts, an entity or block it cannot read, and refuses the drawing only
/// when nothing readable is left.
#[derive(Clone, Copy)]
struct DxfReading {
    max_pairs: usize,
    max_entities: usize,
    max_profiles: usize,
    max_segments: usize,
    /// Joins loose lines and arcs into profiles; otherwise each stays one of its own.
    chain: bool,
    /// Reads texts and attributes instead of reporting them unsupported or dropped.
    texts: bool,
    /// Reads the colors, linetypes and lineweights of entities and layers instead of
    /// reporting them ignored.
    styles: bool,
}

const PROFILE_READING: DxfReading = DxfReading {
    max_pairs: MAX_DXF_PAIRS,
    max_entities: MAX_DXF_ENTITIES,
    max_profiles: MAX_DXF_PROFILES,
    max_segments: MAX_DXF_SEGMENTS_PER_PROFILE,
    chain: true,
    texts: false,
    styles: false,
};

/// No limit, yet small enough that the `+ 1` of a check cannot overflow.
const UNBOUNDED: usize = usize::MAX / 2;

const UNDERLAY_READING: DxfReading = DxfReading {
    max_pairs: UNBOUNDED,
    max_entities: UNBOUNDED,
    max_profiles: UNBOUNDED,
    max_segments: UNBOUNDED,
    chain: false,
    texts: true,
    styles: true,
};

mod block;
mod code_page;
mod style;
mod text;
mod underlay;
use block::{BlockDefinition, parse_blocks};
use code_page::{code_page_encoding, code_page_name, dxf_source_text};
use style::EntityStyle;
use text::OrderedText;
pub use underlay::{DxfUnderlay, read_dxf_underlay, read_dxf_underlay_from};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DxfImportOptions {
    unit_if_unitless: Option<ImportLengthUnit>,
}

impl DxfImportOptions {
    #[must_use]
    pub const fn new(unit_if_unitless: Option<ImportLengthUnit>) -> Self {
        Self { unit_if_unitless }
    }

    #[must_use]
    pub const fn unit_if_unitless(self) -> Option<ImportLengthUnit> {
        self.unit_if_unitless
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedDxfProfile {
    layer: String,
    segments: Vec<ProfileSegment>,
    closed: bool,
}

impl ParsedDxfProfile {
    #[must_use]
    pub fn layer(&self) -> &str {
        &self.layer
    }

    #[must_use]
    pub fn segments(&self) -> &[ProfileSegment] {
        &self.segments
    }

    #[must_use]
    pub const fn closed(&self) -> bool {
        self.closed
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedDxf {
    units: ImportUnitDecision,
    insbase_mm: Option<[f64; 3]>,
    layers: Vec<String>,
    profiles: Vec<ParsedDxfProfile>,
    diagnostics: Vec<ImportDiagnostic>,
}

impl ParsedDxf {
    #[must_use]
    pub const fn units(&self) -> ImportUnitDecision {
        self.units
    }

    #[must_use]
    pub const fn insbase_mm(&self) -> Option<[f64; 3]> {
        self.insbase_mm
    }

    #[must_use]
    pub fn layers(&self) -> &[String] {
        &self.layers
    }

    #[must_use]
    pub fn profiles(&self) -> &[ParsedDxfProfile] {
        &self.profiles
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DxfImportError {
    Empty,
    /// The drawing could not be read on (an underlay is read from its file as it goes).
    Read(ReadFailure),
    NonAscii,
    LineTooLong,
    TooManyPairs,
    MalformedPairs,
    MalformedSections,
    MissingEntitiesSection,
    DuplicateSection,
    InvalidHeader,
    UnitsRequired,
    UnsupportedUnits,
    InvalidNumber,
    NotANumber {
        value: String,
        cause: std::num::ParseFloatError,
    },
    NotAnInteger {
        value: String,
        cause: std::num::ParseIntError,
    },
    NonPlanarGeometry,
    CoordinateOutOfRange,
    DegenerateGeometry,
    InvalidBulge,
    InvalidBlock,
    UnsupportedInsertTransform,
    TooManyEntities,
    TooManyProfiles,
    TooManySegments,
    AmbiguousGeometry,
    /// A closed profile encloses no area or crosses itself.
    InvalidProfile,
    NoSupportedGeometry,
    ReportTooLarge,
    InvalidReport(ImportContractError),
}

impl DxfImportError {
    /// Whether it ends the whole reading: the drawing cannot be read on, or a limit of
    /// the reading or of its report is reached. Any other error is the fault of one
    /// entity or block, which is left out and counted.
    const fn ends_reading(&self) -> bool {
        matches!(
            self,
            Self::Read(_)
                | Self::TooManyPairs
                | Self::TooManyEntities
                | Self::TooManyProfiles
                | Self::ReportTooLarge
                | Self::InvalidReport(_)
        )
    }

    /// Why it was left out, the same for every entity left out for it: without the
    /// value that is not a number, which would give each its own report entry.
    fn reason(&self) -> String {
        match self {
            Self::NotANumber { .. } => "DXF contains a value that is not a number".to_owned(),
            Self::NotAnInteger { .. } => "DXF contains a value that is not an integer".to_owned(),
            other => other.to_string(),
        }
    }

    /// Whether reading the same source in another length unit could avoid
    /// this error: only coordinate range and millimetre tolerances scale.
    #[must_use]
    pub const fn depends_on_units(&self) -> bool {
        matches!(
            self,
            Self::CoordinateOutOfRange
                | Self::DegenerateGeometry
                | Self::AmbiguousGeometry
                | Self::NonPlanarGeometry
        )
    }
}

impl fmt::Display for DxfImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotANumber { value, cause } => {
                return write!(formatter, "DXF value {value:?} is not a number: {cause}");
            }
            Self::NotAnInteger { value, cause } => {
                return write!(formatter, "DXF value {value:?} is not an integer: {cause}");
            }
            Self::InvalidReport(cause) => {
                return write!(formatter, "DXF unsupported/loss report is invalid: {cause}");
            }
            Self::Read(failure) => {
                return write!(formatter, "the DXF could not be read on: {}", failure.0);
            }
            Self::TooManyPairs => {
                return write!(
                    formatter,
                    "DXF exceeds the {MAX_DXF_PAIRS} group-code pair limit"
                );
            }
            Self::TooManyEntities => {
                return write!(formatter, "DXF exceeds the {MAX_DXF_ENTITIES} entity limit");
            }
            Self::TooManyProfiles => {
                return write!(
                    formatter,
                    "DXF exceeds the {MAX_DXF_PROFILES} profile limit of one import"
                );
            }
            Self::Empty => "DXF source is empty",
            Self::NonAscii => "DXF must be UTF-8 text, or older than DXF 2007 and in the Windows code page its $DWGCODEPAGE names; binary DXF is unsupported",
            Self::LineTooLong => "DXF contains a line longer than 512 bytes",
            Self::MalformedPairs => "DXF group-code/value pairs are malformed",
            Self::MalformedSections => "DXF SECTION/ENDSEC/EOF structure is malformed",
            Self::MissingEntitiesSection => "DXF has no ENTITIES section",
            Self::DuplicateSection => "DXF contains a duplicate HEADER or ENTITIES section",
            Self::InvalidHeader => {
                "DXF header contains duplicate or malformed authoritative values"
            }
            Self::UnitsRequired => {
                "DXF does not declare supported units; choose the source length unit explicitly"
            }
            Self::UnsupportedUnits => {
                "DXF declares a length unit outside the supported bounded subset"
            }
            Self::InvalidNumber => "DXF contains an invalid finite numeric value",
            Self::NonPlanarGeometry => {
                "DXF contains non-planar Z, elevation, thickness, or OCS geometry"
            }
            Self::CoordinateOutOfRange => {
                "scaled DXF geometry exceeds the canonical ±1,000,000 mm envelope"
            }
            Self::DegenerateGeometry => "DXF contains degenerate zero-length geometry",
            Self::InvalidBulge => "DXF contains an invalid or full-circle polyline bulge",
            Self::InvalidBlock => {
                "DXF contains an invalid, duplicate, cyclic, or undefined block"
            }
            Self::UnsupportedInsertTransform => {
                "DXF INSERT uses unsupported zero, non-uniform curved, or Z scaling"
            }
            Self::TooManySegments => "one DXF chain exceeds the 1,024 segment limit",
            Self::AmbiguousGeometry => {
                "DXF line/arc connectivity is branched, duplicated, or otherwise ambiguous"
            }
            Self::InvalidProfile => {
                "DXF contains a closed profile that encloses no area or crosses itself"
            }
            Self::NoSupportedGeometry => {
                "DXF contains no supported LINE, ARC, CIRCLE, circular ELLIPSE, linear SPLINE, HATCH/MPOLYGON/MESH boundary, SOLID, TRACE, 3DFACE, straight LEADER, POLYLINE/polyface/polygon-mesh, or LWPOLYLINE geometry"
            }
            Self::ReportTooLarge => "DXF unsupported/loss report exceeds the 1,024 entry limit",
        })
    }
}

impl std::error::Error for DxfImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(failure) => Some(failure.0.as_ref()),
            _ => None,
        }
    }
}

/// The error a read of the drawing failed with, shared so the import error stays `Clone`;
/// two are equal when they are of one kind and say the same.
#[derive(Clone, Debug)]
pub struct ReadFailure(pub std::sync::Arc<std::io::Error>);

impl ReadFailure {
    #[must_use]
    pub fn kind(&self) -> std::io::ErrorKind {
        self.0.kind()
    }
}

impl From<std::io::Error> for ReadFailure {
    fn from(error: std::io::Error) -> Self {
        Self(std::sync::Arc::new(error))
    }
}

impl PartialEq for ReadFailure {
    fn eq(&self, other: &Self) -> bool {
        self.0.kind() == other.0.kind() && self.0.to_string() == other.0.to_string()
    }
}

impl Eq for ReadFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DxfImportPlanError {
    Parse(DxfImportError),
    InvalidSourceIdentity(ImportContractError),
    IdSpaceExhausted,
}

impl fmt::Display for DxfImportPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(formatter),
            Self::InvalidSourceIdentity(error) => {
                write!(
                    formatter,
                    "DXF source name or provenance is invalid: {error}"
                )
            }
            Self::IdSpaceExhausted => formatter.write_str("canonical import ID space is exhausted"),
        }
    }
}

impl std::error::Error for DxfImportPlanError {}

impl From<DxfImportError> for DxfImportPlanError {
    fn from(error: DxfImportError) -> Self {
        Self::Parse(error)
    }
}

#[derive(Clone, Copy)]
struct DxfPair<'a> {
    code: i16,
    value: &'a str,
}

/// What a reading reports, and the limits it reads within.
struct DiagnosticCounts {
    entries: BTreeMap<(ImportDiagnosticSeverity, String, Option<String>), u32>,
    reading: DxfReading,
    /// The layers, upper case, that the drawing's author turned off or froze;
    /// a profile import leaves their entities out (review 2026-10-18 MF-13).
    /// An underlay keeps them, with their tags hidden.
    hidden_layers: BTreeSet<String>,
    /// The layers, upper case, the author froze, as an underlay reads them: all a block
    /// placed on one of them holds goes on that layer ([`place_on_frozen_layer`]).
    frozen_layers: BTreeSet<String>,
    /// The code page its `$DWGCODEPAGE` names, ANSI_1252 when it names none: a text's
    /// `%%nnn` is the character nnn of it.
    code_page: &'static encoding_rs::Encoding,
    /// What it left out, by diagnostic code and what (an entity type or a block, and
    /// why): one report entry each, with the count and the first handles or names.
    skipped: BTreeMap<(&'static str, String), Skipped>,
    /// Why the first entity or block it left out could not be read: the refusal when
    /// nothing readable is left.
    first_skip: Option<DxfImportError>,
}

/// Entities or blocks left out for one reason: how many, and the handles or names of the
/// first [`NAMED_SKIPS`], which the report lists.
struct Skipped {
    count: u32,
    label: &'static str,
    names: Vec<String>,
}

/// How many handles or names one report entry of what was left out lists.
const NAMED_SKIPS: usize = 20;

impl DiagnosticCounts {
    const fn new(reading: DxfReading) -> Self {
        Self {
            entries: BTreeMap::new(),
            reading,
            hidden_layers: BTreeSet::new(),
            frozen_layers: BTreeSet::new(),
            code_page: encoding_rs::WINDOWS_1252,
            skipped: BTreeMap::new(),
            first_skip: None,
        }
    }

    /// Counts one entity or block left out under `code`, as `what` (its type or kind and
    /// why), with its handle or name `name` among the first listed, `label` saying which;
    /// `error` is why it could not be read.
    fn skip(
        &mut self,
        code: &'static str,
        what: String,
        named: Option<(&'static str, &str)>,
        error: Option<DxfImportError>,
    ) -> Result<(), DxfImportError> {
        if self.first_skip.is_none() {
            self.first_skip = error;
        }
        let (label, name) = named.unwrap_or(("", ""));
        let skipped = self.skipped.entry((code, what)).or_insert(Skipped {
            count: 0,
            label,
            names: Vec::new(),
        });
        skipped.count = skipped
            .count
            .checked_add(1)
            .ok_or(DxfImportError::ReportTooLarge)?;
        if !name.is_empty()
            && skipped.names.len() < NAMED_SKIPS
            && !skipped.names.iter().any(|known| known == name)
        {
            skipped.names.push(name.to_owned());
        }
        Ok(())
    }

    /// Why nothing readable is left: why the first thing left out could not be read, or
    /// that the drawing holds nothing it reads.
    fn nothing_left(&mut self) -> DxfImportError {
        self.first_skip
            .take()
            .unwrap_or(DxfImportError::NoSupportedGeometry)
    }

    /// Adds one report entry for each reason something was left out: `what`, then the
    /// first handles or names, as many as fit the report's text.
    fn report_skipped(&mut self) -> Result<(), DxfImportError> {
        for ((code, what), skipped) in std::mem::take(&mut self.skipped) {
            let mut subject = what;
            let mut end = subject.len().min(limits::TEXT_BYTES / 2);
            while !subject.is_char_boundary(end) {
                end -= 1;
            }
            subject.truncate(end);
            let mut listed = String::new();
            let mut shown = 0;
            for name in &skipped.names {
                let separator = if listed.is_empty() { "" } else { ", " };
                if subject.len() + listed.len() + separator.len() + name.len() + 32
                    > limits::TEXT_BYTES
                    || name.chars().any(char::is_control)
                {
                    continue;
                }
                listed.push_str(separator);
                listed.push_str(name);
                shown += 1;
            }
            if shown > 0 {
                let first = if (shown as u32) < skipped.count {
                    format!("first {shown} ")
                } else {
                    String::new()
                };
                subject = format!("{subject}; {first}{}: {listed}", skipped.label);
            }
            self.add(
                ImportDiagnosticSeverity::Warning,
                code,
                Some(subject),
                skipped.count,
            )?;
        }
        Ok(())
    }

    fn add(
        &mut self,
        severity: ImportDiagnosticSeverity,
        code: &str,
        subject: Option<String>,
        count: u32,
    ) -> Result<(), DxfImportError> {
        let mut key = (severity, code.to_owned(), subject);
        // A file with very many layers or ignored groups still imports: past
        // the report's size the rest is counted in one entry.
        if !self.entries.contains_key(&key) && self.entries.len() + 1 >= MAX_IMPORT_DIAGNOSTICS {
            key = (
                ImportDiagnosticSeverity::Warning,
                "dxf.diagnostics-omitted".to_owned(),
                None,
            );
        }
        let entry = self.entries.entry(key).or_default();
        *entry = entry
            .checked_add(count)
            .ok_or(DxfImportError::ReportTooLarge)?;
        if self.entries.len() > MAX_IMPORT_DIAGNOSTICS {
            return Err(DxfImportError::ReportTooLarge);
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<ImportDiagnostic>, DxfImportError> {
        self.report_skipped()?;
        self.entries
            .into_iter()
            .map(|((severity, code, subject), count)| {
                ImportDiagnostic::new(severity, code, subject, count)
                    .map_err(DxfImportError::InvalidReport)
            })
            .collect()
    }
}

#[derive(Clone)]
struct LooseSegment {
    layer: String,
    ordinal: usize,
    /// Read for an underlay only.
    style: EntityStyle,
    segment: ProfileSegment,
}

#[derive(Clone)]
struct OrderedProfile {
    layer: String,
    ordinal: usize,
    /// Read for an underlay only.
    style: EntityStyle,
    segments: Vec<ProfileSegment>,
    closed: bool,
}

impl From<LooseSegment> for OrderedProfile {
    fn from(loose: LooseSegment) -> Self {
        Self {
            layer: loose.layer,
            ordinal: loose.ordinal,
            style: loose.style,
            segments: vec![loose.segment],
            closed: false,
        }
    }
}

#[derive(Default)]
struct ParsedEntities {
    loose: Vec<LooseSegment>,
    explicit_profiles: Vec<OrderedProfile>,
    layers: BTreeSet<String>,
    texts: Vec<OrderedText>,
    /// The xrefs it would place, by name, with how often: counted in the report only where
    /// the drawing itself places them, so an xref in a block counts once per insert of it.
    xrefs: BTreeMap<String, u32>,
    /// The handle (group 5) of each entity read here that has one, by its ordinal.
    handles: BTreeMap<usize, String>,
}

impl ParsedEntities {
    /// Names the xrefs it would place in the report, as not loaded.
    fn report_xrefs(&mut self, diagnostics: &mut DiagnosticCounts) -> Result<(), DxfImportError> {
        for (name, count) in std::mem::take(&mut self.xrefs) {
            diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                "dxf.xref-not-loaded",
                Some(name),
                count,
            )?;
        }
        Ok(())
    }

    fn push_texts(&mut self, texts: impl IntoIterator<Item = OrderedText>) {
        for text in texts {
            self.layers.insert(text.layer.clone());
            self.texts.push(text);
        }
    }

    /// How many lines, profiles and texts it holds.
    fn counts(&self) -> [usize; 3] {
        [
            self.loose.len(),
            self.explicit_profiles.len(),
            self.texts.len(),
        ]
    }

    /// Gives what was added since `counts` the style of the entity `record` that drew it;
    /// what an insert placed from its block at `scale` takes the insert's style ByBlock.
    fn restyle(
        &mut self,
        counts: [usize; 3],
        record: &[DxfPair<'_>],
        placed_at: Option<f64>,
    ) -> Result<(), DxfImportError> {
        let style = EntityStyle::read(record)?;
        let layer = parse_layer(record)?;
        let restyle = |own: &mut EntityStyle| {
            *own = match placed_at {
                Some(scale) => own.within(&style, &layer, scale),
                None => style.clone(),
            };
        };
        self.loose[counts[0]..]
            .iter_mut()
            .for_each(|loose| restyle(&mut loose.style));
        self.explicit_profiles[counts[1]..]
            .iter_mut()
            .for_each(|profile| restyle(&mut profile.style));
        self.texts[counts[2]..]
            .iter_mut()
            .for_each(|text| restyle(&mut text.style));
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum UndirectedSegmentKey {
    Line([i64; 2], [i64; 2]),
    CircularArc {
        start: [i64; 2],
        end: [i64; 2],
        center: [i64; 2],
        clockwise: bool,
    },
    CubicBezier {
        start: [i64; 2],
        control_1: [i64; 2],
        control_2: [i64; 2],
        end: [i64; 2],
    },
    /// The spline points, in the direction that starts at the smaller point.
    Spline(Vec<[i64; 2]>),
}

/// Inspect an ASCII DXF of any size, within the profile count limits, without mutating a document.
pub fn inspect_dxf(source: &[u8], options: DxfImportOptions) -> Result<ParsedDxf, DxfImportError> {
    let (units, insbase_mm, mut parsed_entities, mut diagnostics) =
        read_dxf(source, options, PROFILE_READING)?;
    let mut profiles = assemble_loose_segments(
        parsed_entities.loose,
        MAX_DXF_SEGMENTS_PER_PROFILE,
        Some((&parsed_entities.handles, &mut diagnostics)),
    )?;
    profiles.append(&mut parsed_entities.explicit_profiles);
    let profiles = without_invalid_profiles(profiles, &parsed_entities.handles, &mut diagnostics)?;
    let layers = parsed_entities.layers;
    if profiles.len() > MAX_DXF_PROFILES {
        return Err(DxfImportError::TooManyProfiles);
    }
    // Two equal profiles of a layer share every segment, so this also leaves out the
    // second, in one pass over the segments instead of comparing every pair of profiles.
    let mut profiles =
        without_duplicate_segments(profiles, Some((&parsed_entities.handles, &mut diagnostics)))?;
    profiles.sort_by(|left, right| {
        (left.layer.as_str(), left.ordinal).cmp(&(right.layer.as_str(), right.ordinal))
    });
    if profiles.is_empty() {
        return Err(diagnostics.nothing_left());
    }
    let diagnostics = finish_report(diagnostics, &layers, units, [0.0, 0.0])?;
    Ok(ParsedDxf {
        units,
        insbase_mm,
        layers: layers.into_iter().collect(),
        profiles: profiles
            .into_iter()
            .map(|profile| ParsedDxfProfile {
                layer: profile.layer,
                segments: profile.segments,
                closed: profile.closed,
            })
            .collect(),
        diagnostics,
    })
}

/// `profiles` without those a document would refuse, a closed one enclosing no area or
/// crossing itself, which are left out and counted by the handle of their entity.
fn without_invalid_profiles(
    profiles: Vec<OrderedProfile>,
    handles: &BTreeMap<usize, String>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let mut valid = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let kind = FeatureKind::Profile {
            segments: profile.segments,
            closed: profile.closed,
        };
        if crate::document::is_valid_profile_kind(&kind)
            && let FeatureKind::Profile { segments, .. } = kind
        {
            valid.push(OrderedProfile {
                segments,
                ..profile
            });
            continue;
        }
        diagnostics.skip(
            "dxf.profile-invalid",
            DxfImportError::InvalidProfile.reason(),
            handles
                .get(&profile.ordinal)
                .map(|handle| ("handles", handle.as_str())),
            Some(DxfImportError::InvalidProfile),
        )?;
    }
    Ok(valid)
}

type ReadDxf = (
    ImportUnitDecision,
    Option<[f64; 3]>,
    ParsedEntities,
    DiagnosticCounts,
);

/// The units, base point, entities and report of `source`, read as `reading` says.
fn read_dxf(
    source: &[u8],
    options: DxfImportOptions,
    reading: DxfReading,
) -> Result<ReadDxf, DxfImportError> {
    let (text, code_page) = dxf_source_text(source)?;
    let pairs = parse_pairs(&text, reading.max_pairs)?;
    let (sections, mut diagnostics) = parse_sections(&pairs, reading)?;
    if let Some(code_page) = code_page {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.text-code-page",
            Some(code_page),
            1,
        )?;
    }
    let header = sections.get("HEADER").map(Vec::as_slice).unwrap_or(&[]);
    let (units, insbase_mm) = parse_header(header, options, &mut diagnostics)?;
    diagnostics.hidden_layers = hidden_layers(
        sections.get("TABLES").map(Vec::as_slice).unwrap_or(&[]),
        true,
    );
    let mut entity_count = 0;
    let blocks = parse_blocks(
        sections.get("BLOCKS").map(Vec::as_slice).unwrap_or(&[]),
        units,
        &mut entity_count,
        &mut diagnostics,
    )?;
    let entities = sections
        .get("ENTITIES")
        .ok_or(DxfImportError::MissingEntitiesSection)?;
    let mut parsed_entities = ParsedEntities::default();
    parse_entities(
        entities,
        units,
        Some(&blocks),
        &mut entity_count,
        &mut parsed_entities,
        &mut diagnostics,
    )?;
    parsed_entities.report_xrefs(&mut diagnostics)?;
    Ok((units, insbase_mm, parsed_entities, diagnostics))
}

/// The report of a reading with its layers, origin and units.
fn finish_report(
    mut diagnostics: DiagnosticCounts,
    layers: &BTreeSet<String>,
    units: ImportUnitDecision,
    offset_mm: [f64; 2],
) -> Result<Vec<ImportDiagnostic>, DxfImportError> {
    for layer in layers {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.layer",
            Some(layer.clone()),
            1,
        )?;
    }
    if offset_mm == [0.0, 0.0] {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.wcs-origin-preserved",
            None,
            1,
        )?;
    } else {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.wcs-origin-moved",
            Some(format!(
                "x {} m, y {} m",
                offset_mm[0] / 1000.0,
                offset_mm[1] / 1000.0
            )),
            1,
        )?;
    }
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        match units.authority() {
            ImportUnitAuthority::FileDeclared => "dxf.units-file-declared",
            ImportUnitAuthority::UserDeclared => "dxf.units-user-declared",
        },
        Some(unit_name(units.source_unit()).to_owned()),
        1,
    )?;
    diagnostics.finish()
}

/// Build one detached canonical transaction for a reviewed DXF subset import.
pub fn plan_dxf_import(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
    options: DxfImportOptions,
) -> Result<CommandBatch, DxfImportPlanError> {
    let parsed = inspect_dxf(source, options)?;
    let import_id = snapshot
        .next_import_id()
        .ok_or(DxfImportPlanError::IdSpaceExhausted)?;
    let mut next_definition = next_id(snapshot.definitions().map(|item| item.id().0))?;
    let mut next_feature = next_id(snapshot.features().map(|item| item.id().0))?;
    let mut next_occurrence = next_id(snapshot.occurrences().map(|item| item.id().0))?;
    let display_name = source_name
        .strip_suffix(".dxf")
        .or_else(|| source_name.strip_suffix(".DXF"))
        .filter(|name| !name.is_empty())
        .unwrap_or(source_name);
    let mut commands = Vec::with_capacity(parsed.profiles.len() * 3 + 1);
    let mut outputs = Vec::with_capacity(parsed.profiles.len() * 3);
    for (index, profile) in parsed.profiles.iter().enumerate() {
        let definition_id = DefinitionId(next_definition);
        let feature_id = FeatureId(next_feature);
        let occurrence_id = OccurrenceId(next_occurrence);
        next_definition = next_definition
            .checked_add(1)
            .ok_or(DxfImportPlanError::IdSpaceExhausted)?;
        next_feature = next_feature
            .checked_add(1)
            .ok_or(DxfImportPlanError::IdSpaceExhausted)?;
        next_occurrence = next_occurrence
            .checked_add(1)
            .ok_or(DxfImportPlanError::IdSpaceExhausted)?;
        let name = format!("{display_name} · {} · {}", profile.layer, index + 1);
        commands.push(CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: name.clone(),
        });
        commands.push(CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: format!("DXF profile · {}", profile.layer),
            kind: FeatureKind::Profile {
                segments: profile.segments.clone(),
                closed: profile.closed,
            },
        });
        commands.push(CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id,
            name,
            transform: Transform::identity(),
            parent: None,
            tags: Default::default(),
            visible: true,
        });
        outputs.extend([
            ImportOutputRef::Definition(definition_id),
            ImportOutputRef::Feature(feature_id),
            ImportOutputRef::Occurrence(occurrence_id),
        ]);
    }
    outputs.sort_unstable();
    let receipt = ImportReceipt::from_source_bytes(
        import_id,
        ImportFormat::Dxf,
        source,
        source_name,
        parsed.units,
        DXF_PARSER_ID,
        DXF_PARSER_VERSION,
        parsed.diagnostics,
        outputs,
    )
    .map_err(DxfImportPlanError::InvalidSourceIdentity)?;
    commands.push(CanonicalCommand::RecordImport(receipt));
    Ok(CommandBatch::new(commands))
}

fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, DxfImportPlanError> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|id| *id != 0)
        .ok_or(DxfImportPlanError::IdSpaceExhausted)
}

fn parse_pairs(text: &str, max_pairs: usize) -> Result<Vec<DxfPair<'_>>, DxfImportError> {
    // The lines are counted and measured first and refused on the pair count as it is
    // reached, so a large file is never held as a list of its lines, only as its pairs.
    let max_lines = max_pairs.saturating_mul(2).saturating_add(1);
    let mut line_count = 0_usize;
    for line in text.lines() {
        if line.len() > MAX_DXF_LINE_BYTES {
            return Err(DxfImportError::LineTooLong);
        }
        if line_count > max_lines {
            return Err(DxfImportError::TooManyPairs);
        }
        line_count += 1;
    }
    if !line_count.is_multiple_of(2) {
        return Err(DxfImportError::MalformedPairs);
    }
    if line_count / 2 > max_pairs {
        return Err(DxfImportError::TooManyPairs);
    }
    let mut pairs = Vec::with_capacity(line_count / 2);
    let mut lines = text.lines();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        let code = parse_integer::<i16>(code.trim())?;
        // Group 999 is a comment and may stand anywhere.
        if code != 999 {
            pairs.push(DxfPair {
                code,
                value: value.trim(),
            });
        }
    }
    Ok(pairs)
}

fn parse_sections<'a>(
    pairs: &[DxfPair<'a>],
    reading: DxfReading,
) -> Result<(BTreeMap<String, Vec<DxfPair<'a>>>, DiagnosticCounts), DxfImportError> {
    let mut sections = BTreeMap::new();
    let mut diagnostics = DiagnosticCounts::new(reading);
    let mut index = 0;
    let mut saw_eof = false;
    while index < pairs.len() {
        if pairs[index].code == 0 && pairs[index].value.eq_ignore_ascii_case("EOF") {
            saw_eof = true;
            index += 1;
            break;
        }
        if pairs[index].code != 0
            || !pairs[index].value.eq_ignore_ascii_case("SECTION")
            || pairs.get(index + 1).is_none_or(|pair| pair.code != 2)
        {
            return Err(DxfImportError::MalformedSections);
        }
        let name = pairs[index + 1].value.to_ascii_uppercase();
        index += 2;
        let start = index;
        while index < pairs.len()
            && !(pairs[index].code == 0 && pairs[index].value.eq_ignore_ascii_case("ENDSEC"))
        {
            index += 1;
        }
        if index == pairs.len() {
            return Err(DxfImportError::MalformedSections);
        }
        if sections
            .insert(name.clone(), pairs[start..index].to_vec())
            .is_some()
        {
            return Err(DxfImportError::DuplicateSection);
        }
        if name != "HEADER" && name != "BLOCKS" && name != "ENTITIES" {
            diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                "dxf.section-ignored",
                Some(name),
                1,
            )?;
        }
        index += 1;
    }
    if !saw_eof || index != pairs.len() {
        return Err(DxfImportError::MalformedSections);
    }
    Ok((sections, diagnostics))
}

fn parse_header(
    pairs: &[DxfPair<'_>],
    options: DxfImportOptions,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(ImportUnitDecision, Option<[f64; 3]>), DxfImportError> {
    let mut variables: BTreeMap<&str, &[DxfPair<'_>]> = BTreeMap::new();
    let mut index = 0;
    while index < pairs.len() {
        if pairs[index].code != 9 {
            return Err(DxfImportError::InvalidHeader);
        }
        let name = pairs[index].value;
        let start = index + 1;
        index = start;
        while index < pairs.len() && pairs[index].code != 9 {
            index += 1;
        }
        if variables.insert(name, &pairs[start..index]).is_some() {
            return Err(DxfImportError::InvalidHeader);
        }
    }
    if let Some(code_page) = variables
        .get("$DWGCODEPAGE")
        .and_then(|values| values.iter().find(|pair| pair.code == 3))
        .and_then(|pair| code_page_encoding(pair.value.trim().as_bytes()))
    {
        diagnostics.code_page = code_page;
    }
    let declared = match variables.get("$INSUNITS") {
        Some(values) => {
            let value = parse_integer::<i16>(exactly_one(values, 70)?)?;
            match value {
                0 => None,
                1 => Some(ImportLengthUnit::Inch),
                2 => Some(ImportLengthUnit::Foot),
                4 => Some(ImportLengthUnit::Millimetre),
                5 => Some(ImportLengthUnit::Centimetre),
                6 => Some(ImportLengthUnit::Metre),
                _ => return Err(DxfImportError::UnsupportedUnits),
            }
        }
        None => None,
    };
    let units = match declared {
        Some(unit) => ImportUnitDecision::new(unit, ImportUnitAuthority::FileDeclared),
        None => ImportUnitDecision::new(
            options
                .unit_if_unitless()
                .ok_or(DxfImportError::UnitsRequired)?,
            ImportUnitAuthority::UserDeclared,
        ),
    };
    let insbase_mm = variables
        .get("$INSBASE")
        .map(|values| {
            let scale = units.millimetres_per_unit();
            let point = [
                scale_coordinate(parse_number(exactly_one(values, 10)?)?, scale)?,
                scale_coordinate(parse_number(exactly_one(values, 20)?)?, scale)?,
                scale_coordinate(parse_number(exactly_one(values, 30)?)?, scale)?,
            ];
            if point != [0.0, 0.0, 0.0] {
                diagnostics.add(
                    ImportDiagnosticSeverity::Warning,
                    "dxf.insbase-not-applied",
                    Some(format!("{},{},{}", point[0], point[1], point[2])),
                    1,
                )?;
            }
            Ok(point)
        })
        .transpose()?;
    for name in variables.keys().copied() {
        if name != "$INSUNITS"
            && name != "$INSBASE"
            && !(diagnostics.reading.styles && style::STYLE_VARIABLES.contains(&name))
        {
            diagnostics.add(
                ImportDiagnosticSeverity::Info,
                "dxf.header-variable-ignored",
                Some(name.to_owned()),
                1,
            )?;
        }
    }
    Ok((units, insbase_mm))
}

fn exactly_one<'a>(pairs: &'a [DxfPair<'_>], code: i16) -> Result<&'a str, DxfImportError> {
    let mut values = pairs.iter().filter(|pair| pair.code == code);
    let value = values.next().ok_or(DxfImportError::InvalidHeader)?.value;
    if values.next().is_some() {
        return Err(DxfImportError::InvalidHeader);
    }
    Ok(value)
}

fn bump_entity_count(entity_count: &mut usize, max: usize) -> Result<(), DxfImportError> {
    *entity_count = entity_count
        .checked_add(1)
        .ok_or(DxfImportError::TooManyEntities)?;
    if *entity_count > max {
        Err(DxfImportError::TooManyEntities)
    } else {
        Ok(())
    }
}

fn parse_entities(
    pairs: &[DxfPair<'_>],
    units: ImportUnitDecision,
    blocks: Option<&BTreeMap<String, BlockDefinition>>,
    entity_count: &mut usize,
    output: &mut ParsedEntities,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    let mut index = 0;
    let mut ordinal = 0;
    while index < pairs.len() {
        if pairs[index].code != 0 || pairs[index].value.is_empty() {
            return Err(DxfImportError::MalformedSections);
        }
        ordinal += 1;
        bump_entity_count(entity_count, diagnostics.reading.max_entities)?;
        let kind = pairs[index].value.to_ascii_uppercase();
        let start = index + 1;
        index = start;
        while index < pairs.len() && pairs[index].code != 0 {
            index += 1;
        }
        let record = &pairs[start..index];
        let handle = record
            .iter()
            .find(|pair| pair.code == 5)
            .map(|pair| pair.value.trim().to_owned());
        if let Some(handle) = &handle {
            output.handles.insert(ordinal, handle.clone());
        }
        // An entity it cannot read is left out and counted, by its handle, instead of
        // refusing the whole drawing.
        let read = (|| -> Result<(), DxfImportError> {
            if !matches!(kind.as_str(), "ATTRIB" | "SEQEND")
                && let Some((severity, code)) = left_out(record, &diagnostics.hidden_layers)?
            {
                skip_sub_entities(
                    &kind,
                    record,
                    pairs,
                    &mut index,
                    &mut ordinal,
                    entity_count,
                    diagnostics.reading.max_entities,
                )?;
                diagnostics.add(severity, code, Some(kind.clone()), 1)?;
                return Ok(());
            }
            let counts = output.counts();
            match kind.as_str() {
                "LINE" | "ARC" => {
                    let segment = if kind == "LINE" {
                        parse_line(record, units, diagnostics)?
                    } else {
                        parse_arc(record, units, diagnostics)?
                    };
                    output.layers.insert(segment.layer.clone());
                    output.loose.push(LooseSegment { ordinal, ..segment });
                }
                "CIRCLE" => {
                    let profile = parse_circle(record, units, ordinal, diagnostics)?;
                    output.layers.insert(profile.layer.clone());
                    output.explicit_profiles.push(profile);
                }
                "ELLIPSE" => {
                    if let Some(profile) = parse_ellipse(record, units, ordinal, diagnostics)? {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                }
                "SPLINE" => {
                    if let Some(profile) = parse_spline(record, units, ordinal, diagnostics)? {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                }
                "HATCH" => {
                    for profile in parse_hatch(record, units, ordinal, diagnostics)? {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                }
                "MPOLYGON" => {
                    for profile in parse_mpolygon(record, units, ordinal, diagnostics)? {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                }
                "MESH" => {
                    for profile in parse_mesh(record, units, ordinal, diagnostics)? {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                }
                "SOLID" | "TRACE" => {
                    let profile = parse_trace(&kind, record, units, ordinal, diagnostics)?;
                    output.layers.insert(profile.layer.clone());
                    output.explicit_profiles.push(profile);
                }
                "3DFACE" => {
                    let profile = parse_3dface(record, units, ordinal, diagnostics)?;
                    output.layers.insert(profile.layer.clone());
                    output.explicit_profiles.push(profile);
                }
                "LEADER" => {
                    if let Some(profile) = parse_leader(record, units, ordinal, diagnostics)? {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                }
                "INSERT" => {
                    let profile_ordinal = ordinal;
                    let mut attributes = parse_insert_attributes(
                        record,
                        pairs,
                        &mut index,
                        &mut ordinal,
                        entity_count,
                        units,
                        diagnostics,
                    )?;
                    let block_definitions = blocks.ok_or(DxfImportError::InvalidBlock)?;
                    let (mut profiles, mut texts) = parse_insert(
                        record,
                        units,
                        profile_ordinal,
                        block_definitions,
                        entity_count,
                        &mut output.xrefs,
                        diagnostics,
                    )?;
                    texts.append(&mut attributes);
                    place_on_frozen_layer(record, diagnostics, &mut profiles, &mut texts)?;
                    for profile in profiles {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                    output.push_texts(texts);
                }
                "DIMENSION" => {
                    let block_definitions = blocks.ok_or(DxfImportError::InvalidBlock)?;
                    let (mut profiles, mut texts) =
                        parse_dimension(record, units, ordinal, block_definitions, diagnostics)?;
                    place_on_frozen_layer(record, diagnostics, &mut profiles, &mut texts)?;
                    for profile in profiles {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                    output.push_texts(texts);
                }
                "TEXT" | "MTEXT" | "ATTDEF" if diagnostics.reading.texts => {
                    output.push_texts(text::parse_text(&kind, record, units, diagnostics)?);
                }
                "POLYLINE" => {
                    for profile in parse_polyline(
                        record,
                        pairs,
                        &mut index,
                        &mut ordinal,
                        entity_count,
                        units,
                        diagnostics,
                    )? {
                        output.layers.insert(profile.layer.clone());
                        output.explicit_profiles.push(profile);
                    }
                }
                "LWPOLYLINE" => {
                    let profile = parse_lwpolyline(record, units, ordinal, diagnostics)?;
                    output.layers.insert(profile.layer.clone());
                    output.explicit_profiles.push(profile);
                }
                "ATTRIB" | "SEQEND" => return Err(DxfImportError::MalformedPairs),
                _ => diagnostics.add(
                    ImportDiagnosticSeverity::Warning,
                    "dxf.entity-unsupported",
                    Some(kind.clone()),
                    1,
                )?,
            }
            if diagnostics.reading.styles {
                let placed_at = match kind.as_str() {
                    "INSERT" => Some(parse_optional_number(record, 41)?.unwrap_or(1.0).abs()),
                    "DIMENSION" => Some(1.0),
                    _ => None,
                };
                output.restyle(counts, record, placed_at)?;
            }
            Ok(())
        })();
        if let Err(error) = read {
            if error.ends_reading() {
                return Err(error);
            }
            diagnostics.skip(
                "dxf.entity-unreadable",
                format!("{kind}: {}", error.reason()),
                handle.as_deref().map(|handle| ("handles", handle)),
                Some(error),
            )?;
        }
        // Many INSERTs of a block each within the limit must not build millions
        // of profiles before the whole drawing is checked.
        if output.explicit_profiles.len() > diagnostics.reading.max_profiles {
            return Err(DxfImportError::TooManyProfiles);
        }
    }
    Ok(())
}

/// Why an entity is no part of the model, if it is not: it belongs to a
/// layout (paper space, group 67 = 1), such as a frame or a title block, or
/// its author hid it (group 60 = 1, review 2026-10-17 F-2) or the layer it is
/// on (turned off or frozen, review 2026-10-18 MF-13). Each is left out and
/// counted, never made a profile.
fn left_out(
    record: &[DxfPair<'_>],
    hidden_layers: &BTreeSet<String>,
) -> Result<Option<(ImportDiagnosticSeverity, &'static str)>, DxfImportError> {
    Ok(if parse_optional_i64(record, 67)? == Some(1) {
        Some((ImportDiagnosticSeverity::Info, "dxf.entity-paper-space"))
    } else if parse_optional_i64(record, 60)? == Some(1) {
        Some((ImportDiagnosticSeverity::Warning, "dxf.entity-invisible"))
    } else if parse_optional_text(record, 8)?
        .is_some_and(|layer| hidden_layers.contains(&layer.to_ascii_uppercase()))
    {
        Some((ImportDiagnosticSeverity::Warning, "dxf.entity-layer-hidden"))
    } else {
        None
    })
}

/// The layers, upper case, of the TABLES section `tables` that are frozen (group 70
/// bit 1), and with `turned_off` those turned off (a negative color, group 62) too; the
/// author sees none of their entities (review 2026-10-18 MF-13).
fn hidden_layers(tables: &[DxfPair<'_>], turned_off: bool) -> BTreeSet<String> {
    let mut hidden = BTreeSet::new();
    let mut table = String::new();
    let mut index = 0;
    while index < tables.len() {
        let kind = tables[index].value;
        let start = index + 1;
        index = start;
        while index < tables.len() && tables[index].code != 0 {
            index += 1;
        }
        let record = &tables[start..index];
        let group = |code| {
            record
                .iter()
                .find(|pair| pair.code == code)
                .map(|pair| pair.value.trim())
        };
        let integer = |code| group(code).and_then(|value| value.parse::<i64>().ok());
        if kind.eq_ignore_ascii_case("TABLE") {
            table = group(2).unwrap_or_default().to_ascii_uppercase();
        } else if table == "LAYER"
            && kind.eq_ignore_ascii_case("LAYER")
            && ((turned_off && integer(62).is_some_and(|color| color < 0))
                || integer(70).is_some_and(|flags| flags & 1 != 0))
            && let Some(name) = group(2)
        {
            hidden.insert(name.to_ascii_uppercase());
        }
    }
    hidden
}

/// Puts all that the block of `record`, an INSERT or a DIMENSION, places on the
/// placement's layer when the drawing froze it, its nested blocks and attributes too:
/// freezing that layer hides the whole placement, not only what the block holds on layer
/// 0 (turning it off hides only that). An underlay keeps it there, hidden with that
/// layer's tag (capability review of review 2026-10-21 P3).
fn place_on_frozen_layer(
    record: &[DxfPair<'_>],
    diagnostics: &DiagnosticCounts,
    profiles: &mut [OrderedProfile],
    texts: &mut [OrderedText],
) -> Result<(), DxfImportError> {
    let layer = parse_layer(record)?;
    if diagnostics
        .frozen_layers
        .contains(&layer.to_ascii_uppercase())
    {
        for profile in profiles {
            profile.layer.clone_from(&layer);
        }
        for text in texts {
            text.layer.clone_from(&layer);
        }
    }
    Ok(())
}

/// Steps over what follows an entity left out up to its SEQEND: the
/// vertices of a POLYLINE, the attributes of an INSERT that has them.
fn skip_sub_entities(
    kind: &str,
    record: &[DxfPair<'_>],
    pairs: &[DxfPair<'_>],
    index: &mut usize,
    ordinal: &mut usize,
    entity_count: &mut usize,
    max_entities: usize,
) -> Result<(), DxfImportError> {
    let followed = match kind {
        "POLYLINE" => true,
        "INSERT" => parse_optional_i64(record, 66)? == Some(1),
        _ => false,
    };
    if !followed {
        return Ok(());
    }
    loop {
        if *index >= pairs.len() || pairs[*index].code != 0 || pairs[*index].value.is_empty() {
            return Err(DxfImportError::MalformedPairs);
        }
        *ordinal += 1;
        bump_entity_count(entity_count, max_entities)?;
        let ended = pairs[*index].value.eq_ignore_ascii_case("SEQEND");
        *index += 1;
        while *index < pairs.len() && pairs[*index].code != 0 {
            *index += 1;
        }
        if ended {
            break;
        }
    }
    Ok(())
}

fn parse_line(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<LooseSegment, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "LINE",
        record,
        &[5, 8, 10, 20, 30, 11, 21, 31, 100, 210, 220, 230],
        diagnostics,
    )?;
    let scale = units.millimetres_per_unit();
    let start = parse_xy(record, 10, 20, scale)?;
    let end = parse_xy(record, 11, 21, scale)?;
    ensure_distinct(start, end)?;
    Ok(LooseSegment {
        layer,
        ordinal: 0,
        style: EntityStyle::default(),
        segment: ProfileSegment::Line {
            start_mm: start,
            end_mm: end,
        },
    })
}

fn parse_circle(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<OrderedProfile, DxfImportError> {
    let layer = parse_layer(record)?;
    let mirrored = ensure_planar_ocs(record)?;
    report_ignored_groups(
        "CIRCLE",
        record,
        &[5, 8, 10, 20, 30, 39, 40, 100, 210, 220, 230],
        diagnostics,
    )?;
    let scale = units.millimetres_per_unit();
    let center = parse_xy(record, 10, 20, scale)?;
    let radius = scale_coordinate(parse_required_number(record, 40)?, scale)?;
    if radius <= DXF_GEOMETRY_EPSILON_MM {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let right = [normalize_coordinate(center[0] + radius)?, center[1]];
    let left = [normalize_coordinate(center[0] - radius)?, center[1]];
    normalize_coordinate(center[1] + radius)?;
    normalize_coordinate(center[1] - radius)?;
    ensure_distinct(right, left)?;
    let segments = vec![
        ProfileSegment::CircularArc {
            start_mm: right,
            end_mm: left,
            center_mm: center,
            clockwise: false,
        },
        ProfileSegment::CircularArc {
            start_mm: left,
            end_mm: right,
            center_mm: center,
            clockwise: false,
        },
    ];
    Ok(OrderedProfile {
        layer,
        ordinal,
        style: EntityStyle::default(),
        segments: if mirrored {
            segments.into_iter().map(mirror_ocs).collect()
        } else {
            segments
        },
        closed: true,
    })
}

fn parse_ellipse(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Option<OrderedProfile>, DxfImportError> {
    let ratio = parse_required_number(record, 40)?;
    if ratio <= 0.0 || ratio > 1.0 {
        return Err(DxfImportError::DegenerateGeometry);
    }
    if ratio != 1.0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.entity-unsupported",
            Some("ELLIPSE".to_owned()),
            1,
        )?;
        return Ok(None);
    }

    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "ELLIPSE",
        record,
        &[5, 8, 10, 20, 30, 11, 21, 31, 40, 41, 42, 100, 210, 220, 230],
        diagnostics,
    )?;
    let scale = units.millimetres_per_unit();
    let center = parse_xy(record, 10, 20, scale)?;
    let major_axis = [
        scale_coordinate(parse_required_number(record, 11)?, scale)?,
        scale_coordinate(parse_required_number(record, 21)?, scale)?,
    ];
    let radius = major_axis[0].hypot(major_axis[1]);
    if radius <= DXF_GEOMETRY_EPSILON_MM {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let start_parameter = parse_optional_number(record, 41)?.unwrap_or(0.0);
    let end_parameter = parse_optional_number(record, 42)?.unwrap_or(std::f64::consts::TAU);
    // An end below the start runs on through parameter 0, as DXF defines it.
    let mut sweep = end_parameter - start_parameter;
    if sweep < 0.0 {
        sweep += std::f64::consts::TAU;
    }
    if start_parameter < 0.0 || end_parameter > std::f64::consts::TAU || sweep <= f64::EPSILON {
        return Err(DxfImportError::AmbiguousGeometry);
    }

    let point_at = |parameter: f64| -> Result<[f64; 2], DxfImportError> {
        let (sine, cosine) = deterministic_sin_cos_degrees(parameter.to_degrees());
        Ok([
            normalize_coordinate(center[0] + major_axis[0] * cosine - major_axis[1] * sine)?,
            normalize_coordinate(center[1] + major_axis[1] * cosine + major_axis[0] * sine)?,
        ])
    };
    let start = point_at(start_parameter)?;
    let full = start_parameter == 0.0 && end_parameter == std::f64::consts::TAU;
    let (segments, closed) = if full {
        for direction in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
            normalize_coordinate(center[0] + radius * direction[0])?;
            normalize_coordinate(center[1] + radius * direction[1])?;
        }
        let opposite = point_at(std::f64::consts::PI)?;
        ensure_distinct(start, opposite)?;
        (
            vec![
                ProfileSegment::CircularArc {
                    start_mm: start,
                    end_mm: opposite,
                    center_mm: center,
                    clockwise: false,
                },
                ProfileSegment::CircularArc {
                    start_mm: opposite,
                    end_mm: start,
                    center_mm: center,
                    clockwise: false,
                },
            ],
            true,
        )
    } else {
        let end = point_at(end_parameter)?;
        ensure_distinct(start, end)?;
        validate_arc_sweep_envelope(
            start,
            end,
            center,
            radius,
            false,
            sweep > std::f64::consts::PI,
        )?;
        (
            vec![ProfileSegment::CircularArc {
                start_mm: start,
                end_mm: end,
                center_mm: center,
                clockwise: false,
            }],
            false,
        )
    };
    Ok(Some(OrderedProfile {
        layer,
        ordinal,
        style: EntityStyle::default(),
        segments,
        closed,
    }))
}

fn parse_spline_points(
    record: &[DxfPair<'_>],
    x_code: i16,
    y_code: i16,
    z_code: i16,
) -> Result<Vec<[f64; 2]>, DxfImportError> {
    let mut points: Vec<(f64, Option<f64>, bool)> = Vec::new();
    for pair in record {
        if pair.code == x_code {
            points.push((parse_number(pair.value)?, None, false));
        } else if pair.code == y_code {
            let point = points.last_mut().ok_or(DxfImportError::MalformedPairs)?;
            if point.1.replace(parse_number(pair.value)?).is_some() {
                return Err(DxfImportError::MalformedPairs);
            }
        } else if pair.code == z_code {
            let point = points.last_mut().ok_or(DxfImportError::MalformedPairs)?;
            if point.2 {
                return Err(DxfImportError::MalformedPairs);
            }
            if parse_number(pair.value)? != 0.0 {
                return Err(DxfImportError::NonPlanarGeometry);
            }
            point.2 = true;
        }
    }
    points
        .into_iter()
        .map(|(x, y, _)| Ok([x, y.ok_or(DxfImportError::MalformedPairs)?]))
        .collect()
}

fn parse_spline(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Option<OrderedProfile>, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "SPLINE",
        record,
        &[
            5, 8, 10, 20, 30, 11, 21, 31, 12, 22, 32, 13, 23, 33, 40, 41, 42, 43, 44, 70, 71, 72,
            73, 74, 100, 210, 220, 230,
        ],
        diagnostics,
    )?;
    for pair in record {
        if matches!(
            pair.code,
            10 | 20
                | 30
                | 11
                | 21
                | 31
                | 12
                | 22
                | 32
                | 13
                | 23
                | 33
                | 40
                | 41
                | 42
                | 43
                | 44
                | 210
                | 220
                | 230
        ) {
            parse_number(pair.value)?;
        }
    }

    let flags = parse_required_i64(record, 70)?;
    if flags < 0 || flags & !31 != 0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let degree = parse_required_i64(record, 71)?;
    let declared_knots = parse_required_i64(record, 72)?;
    let declared_controls = parse_required_i64(record, 73)?;
    let declared_fit_points = parse_required_i64(record, 74)?;
    if degree < 1 || declared_knots < 1 || declared_controls < 2 || declared_fit_points < 0 {
        return Err(DxfImportError::MalformedPairs);
    }
    let degree = usize::try_from(degree)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let declared_knots = usize::try_from(declared_knots)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let declared_controls = usize::try_from(declared_controls)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let declared_fit_points = usize::try_from(declared_fit_points)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    if declared_controls > diagnostics.reading.max_segments + 1 {
        return Err(DxfImportError::TooManySegments);
    }
    if degree >= declared_controls {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let expected_knots = declared_controls
        .checked_add(degree)
        .and_then(|count| count.checked_add(1))
        .ok_or(DxfImportError::TooManySegments)?;
    if declared_knots != expected_knots {
        return Err(DxfImportError::MalformedPairs);
    }

    let knots = record
        .iter()
        .filter(|pair| pair.code == 40)
        .map(|pair| parse_number(pair.value))
        .collect::<Result<Vec<_>, _>>()?;
    let controls = parse_spline_points(record, 10, 20, 30)?;
    let fit_points = parse_spline_points(record, 11, 21, 31)?;
    let start_tangents = parse_spline_points(record, 12, 22, 32)?;
    let end_tangents = parse_spline_points(record, 13, 23, 33)?;
    if start_tangents.len() > 1 || end_tangents.len() > 1 {
        return Err(DxfImportError::MalformedPairs);
    }
    if knots.len() != declared_knots
        || controls.len() != declared_controls
        || fit_points.len() != declared_fit_points
    {
        return Err(DxfImportError::MalformedPairs);
    }
    if knots.windows(2).any(|pair| pair[0] > pair[1]) || knots.first() >= knots.last() {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    for tolerance_code in [42, 43, 44] {
        if parse_optional_number(record, tolerance_code)?.is_some_and(|value| value < 0.0) {
            return Err(DxfImportError::AmbiguousGeometry);
        }
    }

    let weights = record
        .iter()
        .filter(|pair| pair.code == 41)
        .map(|pair| parse_number(pair.value))
        .collect::<Result<Vec<_>, _>>()?;
    if (!weights.is_empty() && weights.len() != controls.len())
        || weights.iter().any(|weight| *weight <= 0.0)
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let has_fit_tangent = !start_tangents.is_empty() || !end_tangents.is_empty();
    if degree != 1
        || flags & (2 | 4) != 0
        || !weights.is_empty()
        || !fit_points.is_empty()
        || has_fit_tangent
    {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.entity-unsupported",
            Some("SPLINE".to_owned()),
            1,
        )?;
        return Ok(None);
    }

    if knots[0] != knots[1]
        || knots[knots.len() - 2] != knots[knots.len() - 1]
        || knots[1..knots.len() - 1]
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let closed = flags & 1 != 0;
    if closed && controls.len() < 3 {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let vertices = controls
        .into_iter()
        .map(|[x, y]| PolylineVertex {
            x,
            y: Some(y),
            bulge: None,
        })
        .collect();
    build_polyline_profile(layer, ordinal, vertices, closed, units, diagnostics).map(Some)
}

fn parse_hatch(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "HATCH",
        record,
        &[
            2, 5, 8, 10, 11, 12, 13, 20, 21, 22, 23, 30, 40, 41, 42, 47, 50, 51, 52, 62, 63, 70,
            71, 72, 73, 75, 76, 77, 78, 79, 91, 92, 93, 94, 95, 96, 97, 98, 100, 210, 220, 230,
            330,
        ],
        diagnostics,
    )?;

    let solid_fill = parse_required_i64(record, 70)?;
    let associative = parse_optional_i64(record, 71)?.unwrap_or(0);
    if !matches!(solid_fill, 0 | 1) || !matches!(associative, 0 | 1) {
        return Err(DxfImportError::MalformedPairs);
    }
    let profiles =
        parse_polygon_boundaries(record, units, ordinal, &layer, "HATCH", true, diagnostics)?;
    if !profiles.is_empty() {
        report_hatch_losses(&layer, associative, diagnostics)?;
        report_polygon_topology_loss(
            "dxf.hatch-boundary-topology-dropped",
            &layer,
            profiles.len(),
            diagnostics,
        )?;
    }
    Ok(profiles)
}

fn parse_mpolygon(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "MPOLYGON",
        record,
        &[
            2, 5, 8, 10, 11, 20, 21, 30, 40, 41, 42, 47, 50, 51, 52, 62, 63, 70, 71, 72, 73, 75,
            76, 77, 78, 79, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 210, 220, 230, 330, 420, 421,
            450, 451, 452, 453, 460, 461, 462, 463, 470,
        ],
        diagnostics,
    )?;

    let header_end = record
        .iter()
        .position(|pair| pair.code == 92)
        .ok_or(DxfImportError::MalformedPairs)?;
    let header = &record[..header_end];
    let version = parse_required_i64(header, 70)?;
    let solid_fill = parse_required_i64(header, 71)?;
    if version != 1 || !matches!(solid_fill, 0 | 1) {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    if parse_required_number(header, 10)? != 0.0 || parse_required_number(header, 20)? != 0.0 {
        return Err(DxfImportError::NonPlanarGeometry);
    }

    let trailing = polygon_trailing_record(record)?;
    let annotated = parse_optional_i64(trailing, 73)?.unwrap_or(0);
    if !matches!(annotated, 0 | 1) {
        return Err(DxfImportError::MalformedPairs);
    }
    let offset_x = parse_optional_number(trailing, 11)?;
    let offset_y = parse_optional_number(trailing, 21)?;
    if offset_x.is_some() != offset_y.is_some() {
        return Err(DxfImportError::MalformedPairs);
    }
    if offset_x.unwrap_or(0.0) != 0.0 || offset_y.unwrap_or(0.0) != 0.0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let degenerated_loops = parse_optional_i64(trailing, 99)?.unwrap_or(0);
    if degenerated_loops < 0 {
        return Err(DxfImportError::MalformedPairs);
    }
    if degenerated_loops != 0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }

    let profiles = parse_polygon_boundaries(
        record,
        units,
        ordinal,
        &layer,
        "MPOLYGON",
        false,
        diagnostics,
    )?;
    if !profiles.is_empty() {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.mpolygon-fill-dropped",
            Some(layer.clone()),
            1,
        )?;
        if annotated == 1 {
            diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                "dxf.mpolygon-annotation-dropped",
                Some(layer.clone()),
                1,
            )?;
        }
        report_polygon_topology_loss(
            "dxf.mpolygon-boundary-topology-dropped",
            &layer,
            profiles.len(),
            diagnostics,
        )?;
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.mpolygon-geometry",
            Some(layer),
            u32::try_from(profiles.len())
                .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManyProfiles)?,
        )?;
    }
    Ok(profiles)
}

fn parse_polygon_boundaries(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    layer: &str,
    entity_name: &str,
    allow_source_handles: bool,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let path_count = parse_required_i64(record, 91)?;
    if path_count < 1 {
        return Err(DxfImportError::MalformedPairs);
    }
    let path_count = usize::try_from(path_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManyProfiles)?;
    if path_count > diagnostics.reading.max_profiles {
        return Err(DxfImportError::TooManyProfiles);
    }

    let path_starts = record
        .iter()
        .enumerate()
        .filter(|(_, pair)| pair.code == 92)
        .collect::<Vec<_>>();
    if path_starts.len() != path_count {
        return Err(DxfImportError::MalformedPairs);
    }

    let mut profiles = Vec::with_capacity(path_count);
    for (path_index, (path_start, path_type)) in path_starts.iter().copied().enumerate() {
        let path_type = parse_integer::<i64>(path_type.value)?;
        if path_type < 0 {
            return Err(DxfImportError::MalformedPairs);
        }
        if path_type & !19 != 0 {
            diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                "dxf.entity-unsupported",
                Some(entity_name.to_owned()),
                1,
            )?;
            return Ok(Vec::new());
        }

        let record_end = path_starts
            .get(path_index + 1)
            .map_or(record.len(), |(next_start, _)| *next_start);
        let path_record = &record[path_start + 1..record_end];
        let source_count_index = path_record
            .iter()
            .rposition(|pair| pair.code == 97)
            .ok_or(DxfImportError::MalformedPairs)?;
        let source_count = parse_integer::<i64>(path_record[source_count_index].value)?;
        if source_count < 0 {
            return Err(DxfImportError::MalformedPairs);
        }
        let source_count = usize::try_from(source_count)
            .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
        if !allow_source_handles && source_count != 0 {
            return Err(DxfImportError::AmbiguousGeometry);
        }
        let source_end = source_count_index
            .checked_add(1)
            .and_then(|index| index.checked_add(source_count))
            .ok_or(DxfImportError::MalformedPairs)?;
        if source_end > path_record.len()
            || path_record[source_count_index + 1..source_end]
                .iter()
                .any(|pair| pair.code != 330)
        {
            return Err(DxfImportError::MalformedPairs);
        }

        let path = &path_record[..source_count_index];
        let profile_ordinal = ordinal
            .checked_add(path_index)
            .ok_or(DxfImportError::TooManyProfiles)?;
        let Some(profile) =
            parse_hatch_path(path, path_type, layer, profile_ordinal, units, diagnostics)?
        else {
            diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                "dxf.entity-unsupported",
                Some(entity_name.to_owned()),
                1,
            )?;
            return Ok(Vec::new());
        };
        profiles.push(profile);
    }

    Ok(profiles)
}

fn polygon_trailing_record<'a>(
    record: &'a [DxfPair<'a>],
) -> Result<&'a [DxfPair<'a>], DxfImportError> {
    let path_start = record
        .iter()
        .rposition(|pair| pair.code == 92)
        .ok_or(DxfImportError::MalformedPairs)?;
    let path_record = &record[path_start + 1..];
    let source_count_index = path_record
        .iter()
        .rposition(|pair| pair.code == 97)
        .ok_or(DxfImportError::MalformedPairs)?;
    let source_count = parse_integer::<usize>(path_record[source_count_index].value)?;
    let source_end = source_count_index
        .checked_add(1)
        .and_then(|index| index.checked_add(source_count))
        .ok_or(DxfImportError::MalformedPairs)?;
    if source_end > path_record.len()
        || path_record[source_count_index + 1..source_end]
            .iter()
            .any(|pair| pair.code != 330)
    {
        return Err(DxfImportError::MalformedPairs);
    }
    Ok(&path_record[source_end..])
}

fn report_polygon_topology_loss(
    code: &str,
    layer: &str,
    path_count: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    if path_count > 1 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            code,
            Some(layer.to_owned()),
            u32::try_from(path_count)
                .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManyProfiles)?,
        )?;
    }
    Ok(())
}

fn parse_hatch_path(
    path: &[DxfPair<'_>],
    path_type: i64,
    layer: &str,
    ordinal: usize,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Option<OrderedProfile>, DxfImportError> {
    if path_type & 2 == 0 {
        return parse_hatch_edge_path(
            path,
            layer,
            ordinal,
            units,
            diagnostics.reading.max_segments,
        );
    }

    let has_bulge = parse_required_i64(path, 72)?;
    let closed = parse_required_i64(path, 73)?;
    let declared_count = parse_required_i64(path, 93)?;
    if !matches!(has_bulge, 0 | 1) {
        return Err(DxfImportError::MalformedPairs);
    }
    if closed != 1 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    if declared_count < 3 {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let declared_count = usize::try_from(declared_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManySegments)?;
    if declared_count > diagnostics.reading.max_segments {
        return Err(DxfImportError::TooManySegments);
    }

    let vertex_start = path
        .iter()
        .position(|pair| pair.code == 93)
        .ok_or(DxfImportError::MalformedPairs)?
        + 1;
    let mut vertices = Vec::with_capacity(declared_count);
    for pair in &path[vertex_start..] {
        match pair.code {
            10 => vertices.push(PolylineVertex {
                x: parse_number(pair.value)?,
                y: None,
                bulge: None,
            }),
            20 => {
                let vertex = vertices.last_mut().ok_or(DxfImportError::MalformedPairs)?;
                if vertex.y.replace(parse_number(pair.value)?).is_some() {
                    return Err(DxfImportError::MalformedPairs);
                }
            }
            42 => {
                let vertex = vertices.last_mut().ok_or(DxfImportError::MalformedPairs)?;
                let bulge = parse_number(pair.value)?;
                if vertex.bulge.replace(bulge).is_some() {
                    return Err(DxfImportError::MalformedPairs);
                }
                if has_bulge == 0 && bulge != 0.0 {
                    return Err(DxfImportError::InvalidBulge);
                }
            }
            _ => {}
        }
    }
    if vertices.len() != declared_count || vertices.iter().any(|vertex| vertex.y.is_none()) {
        return Err(DxfImportError::MalformedPairs);
    }

    build_polyline_profile(
        layer.to_owned(),
        ordinal,
        vertices,
        true,
        units,
        diagnostics,
    )
    .map(Some)
}

fn report_hatch_losses(
    layer: &str,
    associative: i64,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    diagnostics.add(
        ImportDiagnosticSeverity::Warning,
        "dxf.hatch-fill-dropped",
        Some(layer.to_owned()),
        1,
    )?;
    if associative == 1 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.hatch-associativity-dropped",
            Some(layer.to_owned()),
            1,
        )?;
    }
    Ok(())
}

fn parse_hatch_edge_path(
    path: &[DxfPair<'_>],
    layer: &str,
    ordinal: usize,
    units: ImportUnitDecision,
    max_segments: usize,
) -> Result<Option<OrderedProfile>, DxfImportError> {
    let declared_count = parse_required_i64(path, 93)?;
    if declared_count < 1 {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let declared_count = usize::try_from(declared_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManySegments)?;
    if declared_count > max_segments {
        return Err(DxfImportError::TooManySegments);
    }
    let count_index = path
        .iter()
        .position(|pair| pair.code == 93)
        .ok_or(DxfImportError::MalformedPairs)?;
    let edge_data = &path[count_index + 1..];
    if edge_data.first().is_none_or(|pair| pair.code != 72) {
        return Err(DxfImportError::MalformedPairs);
    }
    let edge_starts = edge_data
        .iter()
        .enumerate()
        .filter_map(|(index, pair)| (pair.code == 72).then_some(index))
        .collect::<Vec<_>>();
    if edge_starts.len() != declared_count {
        return Err(DxfImportError::MalformedPairs);
    }

    let scale = units.millimetres_per_unit();
    let mut loose = Vec::with_capacity(declared_count);
    let mut unique = BTreeSet::new();
    for (edge_index, start_index) in edge_starts.iter().copied().enumerate() {
        let end_index = edge_starts
            .get(edge_index + 1)
            .copied()
            .unwrap_or(edge_data.len());
        let edge = &edge_data[start_index..end_index];
        let edge_type = parse_required_i64(edge, 72)?;
        let segments = match edge_type {
            1 => {
                if edge
                    .iter()
                    .any(|pair| !matches!(pair.code, 72 | 10 | 20 | 11 | 21))
                {
                    return Err(DxfImportError::MalformedPairs);
                }
                let start_mm = parse_xy(edge, 10, 20, scale)?;
                let end_mm = parse_xy(edge, 11, 21, scale)?;
                ensure_distinct(start_mm, end_mm)?;
                vec![ProfileSegment::Line { start_mm, end_mm }]
            }
            2 => {
                if edge
                    .iter()
                    .any(|pair| !matches!(pair.code, 72 | 10 | 20 | 40 | 50 | 51 | 73))
                {
                    return Err(DxfImportError::MalformedPairs);
                }
                let center_mm = parse_xy(edge, 10, 20, scale)?;
                let radius = scale_coordinate(parse_required_number(edge, 40)?, scale)?;
                if radius <= DXF_GEOMETRY_EPSILON_MM {
                    return Err(DxfImportError::DegenerateGeometry);
                }
                let counterclockwise = parse_required_i64(edge, 73)?;
                if !matches!(counterclockwise, 0 | 1) {
                    return Err(DxfImportError::MalformedPairs);
                }
                let clockwise = counterclockwise == 0;
                let start_angle = parse_required_number(edge, 50)?.rem_euclid(360.0);
                let end_angle = parse_required_number(edge, 51)?.rem_euclid(360.0);
                let sweep = if clockwise {
                    (start_angle - end_angle).rem_euclid(360.0)
                } else {
                    (end_angle - start_angle).rem_euclid(360.0)
                };
                let point_at = |angle| -> Result<[f64; 2], DxfImportError> {
                    let (sin, cos) = deterministic_sin_cos_degrees(angle);
                    Ok([
                        normalize_coordinate(center_mm[0] + radius * cos)?,
                        normalize_coordinate(center_mm[1] + radius * sin)?,
                    ])
                };
                let start_mm = point_at(start_angle)?;
                if sweep <= f64::EPSILON {
                    for direction in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
                        normalize_coordinate(center_mm[0] + radius * direction[0])?;
                        normalize_coordinate(center_mm[1] + radius * direction[1])?;
                    }
                    let opposite_mm = point_at(start_angle + 180.0)?;
                    ensure_distinct(start_mm, opposite_mm)?;
                    vec![
                        ProfileSegment::CircularArc {
                            start_mm,
                            end_mm: opposite_mm,
                            center_mm,
                            clockwise,
                        },
                        ProfileSegment::CircularArc {
                            start_mm: opposite_mm,
                            end_mm: start_mm,
                            center_mm,
                            clockwise,
                        },
                    ]
                } else {
                    let end_mm = point_at(end_angle)?;
                    ensure_distinct(start_mm, end_mm)?;
                    validate_arc_sweep_envelope(
                        start_mm,
                        end_mm,
                        center_mm,
                        radius,
                        clockwise,
                        sweep > 180.0,
                    )?;
                    vec![ProfileSegment::CircularArc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    }]
                }
            }
            3 => {
                if edge
                    .iter()
                    .any(|pair| !matches!(pair.code, 72 | 10 | 20 | 11 | 21 | 40 | 50 | 51 | 73))
                {
                    return Err(DxfImportError::MalformedPairs);
                }
                let center_mm = parse_xy(edge, 10, 20, scale)?;
                let major_axis_mm = [
                    scale_coordinate(parse_required_number(edge, 11)?, scale)?,
                    scale_coordinate(parse_required_number(edge, 21)?, scale)?,
                ];
                let radius = major_axis_mm[0].hypot(major_axis_mm[1]);
                if radius <= DXF_GEOMETRY_EPSILON_MM {
                    return Err(DxfImportError::DegenerateGeometry);
                }
                let ratio = parse_required_number(edge, 40)?;
                if ratio <= 0.0 || ratio > 1.0 {
                    return Err(DxfImportError::DegenerateGeometry);
                }
                if ratio != 1.0 {
                    return Ok(None);
                }
                let counterclockwise = parse_required_i64(edge, 73)?;
                if !matches!(counterclockwise, 0 | 1) {
                    return Err(DxfImportError::MalformedPairs);
                }
                let clockwise = counterclockwise == 0;
                let start_angle = parse_required_number(edge, 50)?.rem_euclid(360.0);
                let end_angle = parse_required_number(edge, 51)?.rem_euclid(360.0);
                let sweep = if clockwise {
                    (start_angle - end_angle).rem_euclid(360.0)
                } else {
                    (end_angle - start_angle).rem_euclid(360.0)
                };
                let point_at = |angle| -> Result<[f64; 2], DxfImportError> {
                    let (sin, cos) = deterministic_sin_cos_degrees(angle);
                    Ok([
                        normalize_coordinate(
                            center_mm[0] + major_axis_mm[0] * cos - major_axis_mm[1] * sin,
                        )?,
                        normalize_coordinate(
                            center_mm[1] + major_axis_mm[1] * cos + major_axis_mm[0] * sin,
                        )?,
                    ])
                };
                let start_mm = point_at(start_angle)?;
                if sweep <= f64::EPSILON {
                    for direction in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
                        normalize_coordinate(center_mm[0] + radius * direction[0])?;
                        normalize_coordinate(center_mm[1] + radius * direction[1])?;
                    }
                    let opposite_mm = point_at(start_angle + 180.0)?;
                    ensure_distinct(start_mm, opposite_mm)?;
                    vec![
                        ProfileSegment::CircularArc {
                            start_mm,
                            end_mm: opposite_mm,
                            center_mm,
                            clockwise,
                        },
                        ProfileSegment::CircularArc {
                            start_mm: opposite_mm,
                            end_mm: start_mm,
                            center_mm,
                            clockwise,
                        },
                    ]
                } else {
                    let end_mm = point_at(end_angle)?;
                    ensure_distinct(start_mm, end_mm)?;
                    validate_arc_sweep_envelope(
                        start_mm,
                        end_mm,
                        center_mm,
                        radius,
                        clockwise,
                        sweep > 180.0,
                    )?;
                    vec![ProfileSegment::CircularArc {
                        start_mm,
                        end_mm,
                        center_mm,
                        clockwise,
                    }]
                }
            }
            4 => {
                if edge.iter().any(|pair| {
                    !matches!(
                        pair.code,
                        72 | 94
                            | 73
                            | 74
                            | 95
                            | 96
                            | 40
                            | 10
                            | 20
                            | 42
                            | 97
                            | 11
                            | 21
                            | 12
                            | 22
                            | 13
                            | 23
                    )
                }) {
                    return Err(DxfImportError::MalformedPairs);
                }
                let Some(segments) = parse_hatch_linear_spline_edge(edge, scale, max_segments)?
                else {
                    return Ok(None);
                };
                segments
            }
            _ => return Err(DxfImportError::MalformedPairs),
        };

        for segment in segments {
            if loose.len() >= max_segments {
                return Err(DxfImportError::TooManySegments);
            }
            let start = point_key(segment.start_mm());
            let end = point_key(segment.end_mm());
            let (first, second, reversed) = if start <= end {
                (start, end, false)
            } else {
                (end, start, true)
            };
            let key = match &segment {
                ProfileSegment::Line { .. } => (0, first, second, [0, 0], false),
                ProfileSegment::CircularArc {
                    center_mm,
                    clockwise,
                    ..
                } => (
                    1,
                    first,
                    second,
                    point_key(*center_mm),
                    if reversed { !*clockwise } else { *clockwise },
                ),
                ProfileSegment::CubicBezier { .. } | ProfileSegment::Spline { .. } => {
                    return Err(DxfImportError::AmbiguousGeometry);
                }
            };
            if !unique.insert(key) {
                return Err(DxfImportError::AmbiguousGeometry);
            }
            loose.push(LooseSegment {
                layer: layer.to_owned(),
                ordinal: loose.len(),
                style: EntityStyle::default(),
                segment,
            });
        }
    }

    let mut profiles = assemble_loose_segments(loose, max_segments, None)?;
    if profiles.len() != 1 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let mut profile = profiles.pop().expect("one HATCH edge-path profile");
    if !profile.closed {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    profile.ordinal = ordinal;
    Ok(Some(profile))
}

fn parse_hatch_linear_spline_edge(
    edge: &[DxfPair<'_>],
    scale: f64,
    max_segments: usize,
) -> Result<Option<Vec<ProfileSegment>>, DxfImportError> {
    let degree = parse_required_i64(edge, 94)?;
    let rational = parse_required_i64(edge, 73)?;
    let periodic = parse_required_i64(edge, 74)?;
    let declared_knots = parse_required_i64(edge, 95)?;
    let declared_controls = parse_required_i64(edge, 96)?;
    let declared_fit_points = parse_required_i64(edge, 97)?;
    if degree < 1
        || !matches!(rational, 0 | 1)
        || !matches!(periodic, 0 | 1)
        || declared_knots < 1
        || declared_controls < 2
        || declared_fit_points < 0
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let degree = usize::try_from(degree)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let declared_knots = usize::try_from(declared_knots)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let declared_controls = usize::try_from(declared_controls)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let declared_fit_points = usize::try_from(declared_fit_points)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    if declared_controls > max_segments + 1 {
        return Err(DxfImportError::TooManySegments);
    }
    if degree >= declared_controls {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let expected_knots = declared_controls
        .checked_add(degree)
        .and_then(|count| count.checked_add(1))
        .ok_or(DxfImportError::TooManySegments)?;
    if declared_knots != expected_knots {
        return Err(DxfImportError::MalformedPairs);
    }

    let knots = edge
        .iter()
        .filter(|pair| pair.code == 40)
        .map(|pair| parse_number(pair.value))
        .collect::<Result<Vec<_>, _>>()?;
    let controls = parse_spline_points(edge, 10, 20, 30)?;
    let fit_points = parse_spline_points(edge, 11, 21, 31)?;
    let start_tangents = parse_spline_points(edge, 12, 22, 32)?;
    let end_tangents = parse_spline_points(edge, 13, 23, 33)?;
    let weights = edge
        .iter()
        .filter(|pair| pair.code == 42)
        .map(|pair| parse_number(pair.value))
        .collect::<Result<Vec<_>, _>>()?;
    if knots.len() != declared_knots
        || controls.len() != declared_controls
        || fit_points.len() != declared_fit_points
        || start_tangents.len() > 1
        || end_tangents.len() > 1
        || (!weights.is_empty() && weights.len() != controls.len())
        || weights.iter().any(|weight| *weight <= 0.0)
    {
        return Err(DxfImportError::MalformedPairs);
    }
    if knots.windows(2).any(|pair| pair[0] > pair[1]) || knots.first() >= knots.last() {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    if degree != 1
        || rational != 0
        || periodic != 0
        || !weights.is_empty()
        || !fit_points.is_empty()
        || !start_tangents.is_empty()
        || !end_tangents.is_empty()
    {
        return Ok(None);
    }
    if knots[0] != knots[1]
        || knots[knots.len() - 2] != knots[knots.len() - 1]
        || knots[1..knots.len() - 1]
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }

    controls
        .windows(2)
        .map(|points| {
            let start_mm = [
                scale_coordinate(points[0][0], scale)?,
                scale_coordinate(points[0][1], scale)?,
            ];
            let end_mm = [
                scale_coordinate(points[1][0], scale)?,
                scale_coordinate(points[1][1], scale)?,
            ];
            ensure_distinct(start_mm, end_mm)?;
            Ok(ProfileSegment::Line { start_mm, end_mm })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn parse_mesh(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "MESH",
        record,
        &[5, 8, 10, 20, 30, 71, 72, 90, 91, 92, 93, 94, 95, 100, 140],
        diagnostics,
    )?;

    let marker_positions = [92_i16, 93, 94, 95]
        .into_iter()
        .map(|code| {
            let mut matches = record
                .iter()
                .enumerate()
                .filter(|(_, pair)| pair.code == code);
            let position = matches
                .next()
                .map(|(position, _)| position)
                .ok_or(DxfImportError::MalformedPairs)?;
            if matches.next().is_some() {
                return Err(DxfImportError::MalformedPairs);
            }
            Ok(position)
        })
        .collect::<Result<Vec<_>, DxfImportError>>()?;
    let [vertices_marker, faces_marker, edges_marker, creases_marker] = marker_positions.as_slice()
    else {
        unreachable!("four MESH markers were requested")
    };
    let (vertices_marker, faces_marker, edges_marker, creases_marker) = (
        *vertices_marker,
        *faces_marker,
        *edges_marker,
        *creases_marker,
    );
    if !(vertices_marker < faces_marker
        && faces_marker < edges_marker
        && edges_marker < creases_marker)
    {
        return Err(DxfImportError::MalformedPairs);
    }

    let header = &record[..vertices_marker];
    let version = parse_required_i64(header, 71)?;
    let blend_crease = parse_required_i64(header, 72)?;
    let subdivision_level = parse_required_i64(header, 91)?;
    if version != 2 || !matches!(blend_crease, 0 | 1) || subdivision_level != 0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }

    let parse_marker_count = |position: usize| -> Result<usize, DxfImportError> {
        let value = parse_integer::<i64>(record[position].value)?;
        if value < 0 {
            return Err(DxfImportError::MalformedPairs);
        }
        usize::try_from(value)
            .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)
    };
    let declared_vertices = parse_marker_count(vertices_marker)?;
    if !(3..=diagnostics.reading.max_pairs / 3).contains(&declared_vertices) {
        return Err(DxfImportError::MalformedPairs);
    }
    let vertex_record = &record[vertices_marker + 1..faces_marker];
    if vertex_record.len() != declared_vertices * 3
        || vertex_record
            .iter()
            .enumerate()
            .any(|(index, pair)| pair.code != [10, 20, 30][index % 3])
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let scale = units.millimetres_per_unit();
    let mut vertices = Vec::with_capacity(declared_vertices);
    let mut unique_vertices = BTreeSet::new();
    for coordinates in vertex_record.chunks_exact(3) {
        let z = parse_number(coordinates[2].value)?;
        if z != 0.0 {
            return Err(DxfImportError::NonPlanarGeometry);
        }
        let point = [
            scale_coordinate(parse_number(coordinates[0].value)?, scale)?,
            scale_coordinate(parse_number(coordinates[1].value)?, scale)?,
        ];
        if !unique_vertices.insert(point_key(point)) {
            return Err(DxfImportError::AmbiguousGeometry);
        }
        vertices.push(point);
    }

    let declared_face_values = parse_marker_count(faces_marker)?;
    let face_record = &record[faces_marker + 1..edges_marker];
    if declared_face_values == 0
        || face_record.len() != declared_face_values
        || face_record.iter().any(|pair| pair.code != 90)
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let face_values = face_record
        .iter()
        .map(|pair| parse_integer::<i64>(pair.value))
        .collect::<Result<Vec<_>, _>>()?;
    let mut faces = Vec::new();
    let mut cursor = 0;
    while cursor < face_values.len() {
        let declared_face_vertices = face_values[cursor];
        cursor += 1;
        if declared_face_vertices < 3 {
            return Err(DxfImportError::DegenerateGeometry);
        }
        let declared_face_vertices = usize::try_from(declared_face_vertices)
            .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
        if declared_face_vertices > diagnostics.reading.max_segments
            || cursor + declared_face_vertices > face_values.len()
        {
            return Err(
                if declared_face_vertices > diagnostics.reading.max_segments {
                    DxfImportError::TooManySegments
                } else {
                    DxfImportError::MalformedPairs
                },
            );
        }
        let mut face = Vec::with_capacity(declared_face_vertices);
        let mut unique_indices = BTreeSet::new();
        for raw_index in &face_values[cursor..cursor + declared_face_vertices] {
            if *raw_index < 0 {
                return Err(DxfImportError::MalformedPairs);
            }
            let index = usize::try_from(*raw_index)
                .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
            if index >= vertices.len() || !unique_indices.insert(index) {
                return Err(DxfImportError::AmbiguousGeometry);
            }
            face.push(index);
        }
        cursor += declared_face_vertices;
        let points = face
            .iter()
            .map(|index| vertices[*index])
            .collect::<Vec<_>>();
        validate_mesh_face(&points)?;
        faces.push(face);
    }
    if faces.is_empty() {
        return Err(DxfImportError::MalformedPairs);
    }

    let explicit_edges = parse_marker_count(edges_marker)?;
    if explicit_edges != 0 || edges_marker + 1 != creases_marker {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let creases = parse_marker_count(creases_marker)?;
    let trailing = &record[creases_marker + 1..];
    if creases != 0
        || !(trailing.is_empty()
            || (trailing.len() == 1
                && trailing[0].code == 90
                && trailing[0].value.parse::<i64>().ok() == Some(0)))
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }

    build_mesh_boundary_profiles(
        layer,
        ordinal,
        vertices,
        faces,
        "dxf.mesh-face-topology-dropped",
        "dxf.mesh-boundary-geometry",
        diagnostics,
    )
}

fn build_mesh_boundary_profiles(
    layer: String,
    ordinal: usize,
    vertices: Vec<[f64; 2]>,
    faces: Vec<Vec<usize>>,
    topology_loss_code: &str,
    boundary_code: &str,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let mut edges = BTreeMap::<_, Vec<_>>::new();
    let mut edge_ordinal = 0;
    for face in &faces {
        for (&start, &end) in face
            .iter()
            .zip(face.iter().cycle().skip(1))
            .take(face.len())
        {
            edge_ordinal += 1;
            let key = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            let incidents = edges.entry(key).or_default();
            incidents.push((start, end, edge_ordinal));
            if incidents.len() > 2 {
                return Err(DxfImportError::AmbiguousGeometry);
            }
        }
    }
    for incidents in edges.values() {
        if incidents.len() == 2
            && !(incidents[0].0 == incidents[1].1 && incidents[0].1 == incidents[1].0)
        {
            return Err(DxfImportError::AmbiguousGeometry);
        }
    }
    let mut boundary_edges = edges
        .values()
        .filter_map(|incidents| (incidents.len() == 1).then_some(incidents[0]))
        .collect::<Vec<_>>();
    boundary_edges.sort_by_key(|(_, _, edge_ordinal)| *edge_ordinal);
    if boundary_edges.is_empty() {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let loose = boundary_edges
        .into_iter()
        .map(|(start, end, edge_ordinal)| LooseSegment {
            layer: layer.clone(),
            ordinal: edge_ordinal,
            style: EntityStyle::default(),
            segment: ProfileSegment::Line {
                start_mm: vertices[start],
                end_mm: vertices[end],
            },
        })
        .collect();
    let mut profiles = assemble_loose_segments(loose, diagnostics.reading.max_segments, None)?;
    if profiles.iter().any(|profile| !profile.closed) {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    for profile in &mut profiles {
        profile.ordinal = ordinal;
    }
    diagnostics.add(
        ImportDiagnosticSeverity::Warning,
        topology_loss_code,
        Some(layer.clone()),
        u32::try_from(faces.len())
            .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManyProfiles)?,
    )?;
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        boundary_code,
        Some(layer),
        u32::try_from(profiles.len())
            .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManyProfiles)?,
    )?;
    Ok(profiles)
}

fn validate_mesh_face(points: &[[f64; 2]]) -> Result<(), DxfImportError> {
    for left in 0..points.len() {
        for right in left + 1..points.len() {
            if (points[left][0] - points[right][0]).hypot(points[left][1] - points[right][1])
                <= DXF_GEOMETRY_EPSILON_MM
            {
                return Err(DxfImportError::AmbiguousGeometry);
            }
        }
    }
    for first in 0..points.len() {
        let first_next = (first + 1) % points.len();
        for second in first + 1..points.len() {
            let second_next = (second + 1) % points.len();
            if first_next == second || second_next == first {
                continue;
            }
            if line_segments_intersect(
                points[first],
                points[first_next],
                points[second],
                points[second_next],
            ) {
                return Err(DxfImportError::AmbiguousGeometry);
            }
        }
    }
    let twice_area = points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| cross2(start, end))
        .sum::<f64>();
    if twice_area.abs() <= DXF_GEOMETRY_EPSILON_MM * DXF_GEOMETRY_EPSILON_MM {
        return Err(DxfImportError::DegenerateGeometry);
    }
    Ok(())
}

fn parse_trace(
    entity: &str,
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<OrderedProfile, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        entity,
        record,
        &[
            5, 8, 10, 20, 30, 11, 21, 31, 12, 22, 32, 13, 23, 33, 39, 100, 210, 220, 230,
        ],
        diagnostics,
    )?;
    let scale = units.millimetres_per_unit();
    let native = [
        parse_xy(record, 10, 20, scale)?,
        parse_xy(record, 11, 21, scale)?,
        parse_xy(record, 12, 22, scale)?,
        parse_xy(record, 13, 23, scale)?,
    ];
    let mut points = vec![native[0], native[1], native[3], native[2]];
    if points[2] == points[3] {
        points.pop();
    }
    validate_simple_line_polygon(&points)?;
    let segments = points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .map(|(start_mm, end_mm)| ProfileSegment::Line { start_mm, end_mm })
        .collect();
    Ok(OrderedProfile {
        layer,
        ordinal,
        style: EntityStyle::default(),
        segments,
        closed: true,
    })
}

fn parse_leader(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Option<OrderedProfile>, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "LEADER",
        record,
        &[
            3, 5, 8, 10, 20, 30, 40, 41, 71, 72, 73, 74, 75, 76, 77, 100, 210, 211, 212, 213, 220,
            221, 222, 223, 230, 231, 232, 233, 340,
        ],
        diagnostics,
    )?;
    for pair in record {
        if matches!(
            pair.code,
            10 | 20
                | 30
                | 40
                | 41
                | 210
                | 211
                | 212
                | 213
                | 220
                | 221
                | 222
                | 223
                | 230
                | 231
                | 232
                | 233
        ) {
            let value = parse_number(pair.value)?;
            if matches!(pair.code, 231..=233) && value != 0.0 {
                return Err(DxfImportError::NonPlanarGeometry);
            }
        }
    }
    for code in [40, 41] {
        if parse_optional_number(record, code)?.is_some_and(|value| value < 0.0) {
            return Err(DxfImportError::MalformedPairs);
        }
    }

    let arrowhead = parse_required_i64(record, 71)?;
    let path_type = parse_required_i64(record, 72)?;
    let creation = parse_required_i64(record, 73)?;
    let hook_direction = parse_required_i64(record, 74)?;
    let hookline = parse_required_i64(record, 75)?;
    let declared_vertices = parse_required_i64(record, 76)?;
    if !matches!(arrowhead, 0 | 1)
        || !matches!(path_type, 0 | 1)
        || !matches!(creation, 0..=3)
        || !matches!(hook_direction, 0 | 1)
        || !matches!(hookline, 0 | 1)
        || declared_vertices < 2
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let declared_vertices = usize::try_from(declared_vertices)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    if declared_vertices > diagnostics.reading.max_segments + 1 {
        return Err(DxfImportError::TooManySegments);
    }
    let points = parse_spline_points(record, 10, 20, 30)?;
    if points.len() != declared_vertices {
        return Err(DxfImportError::MalformedPairs);
    }
    let scale = units.millimetres_per_unit();
    let points = points
        .into_iter()
        .map(|[x, y]| Ok([scale_coordinate(x, scale)?, scale_coordinate(y, scale)?]))
        .collect::<Result<Vec<_>, DxfImportError>>()?;
    for pair in points.windows(2) {
        ensure_distinct(pair[0], pair[1])?;
    }

    if path_type != 0 || hookline != 0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.entity-unsupported",
            Some("LEADER".to_owned()),
            1,
        )?;
        return Ok(None);
    }
    let segments = points
        .windows(2)
        .map(|pair| ProfileSegment::Line {
            start_mm: pair[0],
            end_mm: pair[1],
        })
        .collect();
    if arrowhead == 1 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.leader-arrowhead-dropped",
            None,
            1,
        )?;
    }
    diagnostics.add(
        ImportDiagnosticSeverity::Warning,
        "dxf.leader-semantics-dropped",
        None,
        1,
    )?;
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        "dxf.leader-geometry",
        None,
        1,
    )?;
    Ok(Some(OrderedProfile {
        layer,
        ordinal,
        style: EntityStyle::default(),
        segments,
        closed: false,
    }))
}

fn parse_3dface(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<OrderedProfile, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "3DFACE",
        record,
        &[
            5, 8, 10, 20, 30, 11, 21, 31, 12, 22, 32, 13, 23, 33, 70, 100,
        ],
        diagnostics,
    )?;
    if parse_optional_i64(record, 70)?.unwrap_or(0) != 0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let scale = units.millimetres_per_unit();
    let mut points = vec![
        parse_xy(record, 10, 20, scale)?,
        parse_xy(record, 11, 21, scale)?,
        parse_xy(record, 12, 22, scale)?,
        parse_xy(record, 13, 23, scale)?,
    ];
    if points[2] == points[3] {
        points.pop();
    }
    validate_simple_line_polygon(&points)?;
    let segments = points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .map(|(start_mm, end_mm)| ProfileSegment::Line { start_mm, end_mm })
        .collect();
    Ok(OrderedProfile {
        layer,
        ordinal,
        style: EntityStyle::default(),
        segments,
        closed: true,
    })
}

fn parse_arc(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<LooseSegment, DxfImportError> {
    let layer = parse_layer(record)?;
    let mirrored = ensure_planar_ocs(record)?;
    report_ignored_groups(
        "ARC",
        record,
        &[5, 8, 10, 20, 30, 40, 50, 51, 100, 210, 220, 230],
        diagnostics,
    )?;
    let scale = units.millimetres_per_unit();
    let center = parse_xy(record, 10, 20, scale)?;
    let radius = scale_coordinate(parse_required_number(record, 40)?, scale)?;
    if radius <= DXF_GEOMETRY_EPSILON_MM {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let start_angle = parse_required_number(record, 50)?.rem_euclid(360.0);
    let end_angle = parse_required_number(record, 51)?.rem_euclid(360.0);
    let sweep = (end_angle - start_angle).rem_euclid(360.0);
    if sweep <= f64::EPSILON || (360.0 - sweep) <= f64::EPSILON {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let (start_sin, start_cos) = deterministic_sin_cos_degrees(start_angle);
    let (end_sin, end_cos) = deterministic_sin_cos_degrees(end_angle);
    let start = [
        normalize_coordinate(center[0] + radius * start_cos)?,
        normalize_coordinate(center[1] + radius * start_sin)?,
    ];
    let end = [
        normalize_coordinate(center[0] + radius * end_cos)?,
        normalize_coordinate(center[1] + radius * end_sin)?,
    ];
    validate_arc_sweep_envelope(start, end, center, radius, false, sweep > 180.0)?;
    ensure_distinct(start, end)?;
    let segment = ProfileSegment::CircularArc {
        start_mm: start,
        end_mm: end,
        center_mm: center,
        clockwise: false,
    };
    Ok(LooseSegment {
        layer,
        ordinal: 0,
        style: EntityStyle::default(),
        segment: if mirrored {
            mirror_ocs(segment)
        } else {
            segment
        },
    })
}

fn parse_dimension(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    blocks: &BTreeMap<String, BlockDefinition>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(Vec<OrderedProfile>, Vec<OrderedText>), DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "DIMENSION",
        record,
        &[
            1, 2, 3, 5, 8, 10, 11, 12, 13, 14, 15, 16, 20, 21, 22, 23, 24, 25, 26, 30, 31, 32, 33,
            34, 35, 36, 40, 41, 42, 50, 51, 52, 53, 70, 71, 72, 100, 210, 220, 230, 280,
        ],
        diagnostics,
    )?;
    let dimension_type = parse_required_i64(record, 70)?;
    if !(0..=255).contains(&dimension_type) {
        return Err(DxfImportError::MalformedPairs);
    }
    let name = parse_required_text(record, 2)?;
    let key = block_name_key(name)?;
    if !key.starts_with("*D") {
        return Err(DxfImportError::InvalidBlock);
    }
    // A dimension whose drawn block is missing or empty is left out and
    // counted; it no longer refuses the whole drawing (review 2026-10-18 MF-13).
    let Some(block) = blocks.get(&key).filter(|block| !block.is_empty()) else {
        diagnostics.skip(
            "dxf.dimension-block-missing",
            "DIMENSION whose drawn block is undefined, unreadable or empty".to_owned(),
            Some(("blocks", name)),
            Some(DxfImportError::InvalidBlock),
        )?;
        return Ok((Vec::new(), Vec::new()));
    };
    if block.base_mm != [0.0, 0.0] {
        return Err(DxfImportError::InvalidBlock);
    }
    let clone_x = parse_optional_number(record, 12)?;
    let clone_y = parse_optional_number(record, 22)?;
    let clone_offset_mm = match (clone_x, clone_y) {
        (None, None) => [0.0, 0.0],
        (Some(x), Some(y)) => {
            let scale = units.millimetres_per_unit();
            [scale_coordinate(x, scale)?, scale_coordinate(y, scale)?]
        }
        _ => return Err(DxfImportError::MalformedPairs),
    };
    let profiles = block
        .profiles
        .iter()
        .map(|profile| {
            Ok(OrderedProfile {
                layer: if profile.layer == "0" {
                    layer.clone()
                } else {
                    profile.layer.clone()
                },
                ordinal,
                style: profile.style.placed_on(&layer),
                segments: profile
                    .segments
                    .iter()
                    .map(|segment| {
                        transform_segment(
                            segment,
                            [0.0, 0.0],
                            clone_offset_mm,
                            [1.0, 1.0],
                            0.0,
                            1.0,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                closed: profile.closed,
            })
        })
        .collect::<Result<Vec<_>, DxfImportError>>()?;
    diagnostics.add(
        ImportDiagnosticSeverity::Warning,
        "dxf.dimension-semantics-dropped",
        Some(name.to_owned()),
        1,
    )?;
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        "dxf.dimension-graphics",
        Some(name.to_owned()),
        1,
    )?;
    let texts = text::place_texts(
        &block.texts,
        &layer,
        [0.0, 0.0],
        clone_offset_mm,
        [1.0, 1.0],
        0.0,
        1.0,
    )?;
    Ok((profiles, texts))
}

fn add_xrefs<'a>(
    xrefs: &mut BTreeMap<String, u32>,
    placed: impl IntoIterator<Item = (&'a str, u32)>,
) {
    for (name, count) in placed {
        let total = xrefs.entry(name.to_owned()).or_default();
        *total = total.saturating_add(count);
    }
}

fn parse_insert(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    blocks: &BTreeMap<String, BlockDefinition>,
    entity_count: &mut usize,
    xrefs: &mut BTreeMap<String, u32>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(Vec<OrderedProfile>, Vec<OrderedText>), DxfImportError> {
    let layer = parse_layer(record)?;
    let mirrored = ensure_planar_ocs(record)?;
    report_ignored_groups(
        "INSERT",
        record,
        &[
            5, 8, 2, 10, 20, 30, 41, 42, 43, 44, 45, 50, 66, 70, 71, 100, 210, 220, 230,
        ],
        diagnostics,
    )?;
    let name = parse_required_text(record, 2)?;
    let block = blocks
        .get(&block_name_key(name)?)
        .ok_or(DxfImportError::InvalidBlock)?;
    if block.xref {
        add_xrefs(xrefs, [(name, 1)]);
        return Ok((Vec::new(), Vec::new()));
    }
    add_xrefs(
        xrefs,
        block
            .xrefs
            .iter()
            .map(|(name, count)| (name.as_str(), *count)),
    );
    if block.is_empty() {
        // An empty block, or one holding only counted xrefs, places nothing;
        // it does not refuse the rest of the drawing.
        return Ok((Vec::new(), Vec::new()));
    }
    // In an object coordinate system with extrusion (0, 0, -1) the insert is the
    // mirror image of its upright one: its x axis is world -x, so it is placed at -x,
    // turned the other way, its x scale and its column spacing reversed.
    // `0.0 - x` and not `-x`, so that 0 stays +0 and joins bit for bit.
    let flip = |value: f64| if mirrored { 0.0 - value } else { value };
    let scale_x = flip(parse_optional_number(record, 41)?.unwrap_or(1.0));
    let scale_y = parse_optional_number(record, 42)?.unwrap_or(1.0);
    // The block is planar, so its z scale (a uniform 3D scale, as Revit writes) leaves it
    // in the plane; only a zero one collapses it.
    let scale_z = parse_optional_number(record, 43)?.unwrap_or(1.0);
    let rotation_degrees = flip(parse_optional_number(record, 50)?.unwrap_or(0.0));
    let column_count = parse_optional_i64(record, 70)?.unwrap_or(1);
    let row_count = parse_optional_i64(record, 71)?.unwrap_or(1);
    if scale_x == 0.0 || scale_y == 0.0 || scale_z == 0.0 || column_count <= 0 || row_count <= 0 {
        return Err(DxfImportError::UnsupportedInsertTransform);
    }
    let column_count = usize::try_from(column_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::UnsupportedInsertTransform)?;
    let row_count = usize::try_from(row_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::UnsupportedInsertTransform)?;
    let placement_count = row_count
        .checked_mul(column_count)
        .ok_or(DxfImportError::TooManyEntities)?;
    if placement_count > diagnostics.reading.max_entities {
        return Err(DxfImportError::TooManyEntities);
    }
    let unit_scale = units.millimetres_per_unit();
    let column_spacing_mm = scale_coordinate(
        flip(parse_optional_number(record, 44)?.unwrap_or(0.0)),
        unit_scale,
    )?;
    let row_spacing_mm = scale_coordinate(
        parse_optional_number(record, 45)?.unwrap_or(0.0),
        unit_scale,
    )?;
    if (column_count > 1 && column_spacing_mm.abs() <= DXF_GEOMETRY_EPSILON_MM)
        || (row_count > 1 && row_spacing_mm.abs() <= DXF_GEOMETRY_EPSILON_MM)
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    if scale_x.abs() != scale_y.abs()
        && block.profiles.iter().any(|profile| {
            profile
                .segments
                .iter()
                .any(|segment| matches!(segment, ProfileSegment::CircularArc { .. }))
        })
    {
        return Err(DxfImportError::UnsupportedInsertTransform);
    }
    let expanded_profile_count = placement_count
        .checked_mul(block.profiles.len())
        .ok_or(DxfImportError::TooManyProfiles)?;
    if expanded_profile_count > diagnostics.reading.max_profiles {
        return Err(DxfImportError::TooManyProfiles);
    }
    *entity_count = entity_count
        .checked_add(placement_count - 1)
        .ok_or(DxfImportError::TooManyEntities)?;
    if *entity_count > diagnostics.reading.max_entities {
        return Err(DxfImportError::TooManyEntities);
    }
    let [insertion_x, insertion_y] = parse_xy(record, 10, 20, unit_scale)?;
    let insertion_mm = [flip(insertion_x), insertion_y];
    let (rotation_sin, rotation_cos) = deterministic_sin_cos_degrees(rotation_degrees);
    let mut profiles = Vec::with_capacity(expanded_profile_count);
    let mut texts = Vec::new();
    for row in 0..row_count {
        for column in 0..column_count {
            let array_offset_mm = [
                column as f64 * column_spacing_mm,
                row as f64 * row_spacing_mm,
            ];
            let array_insertion_mm = [
                normalize_coordinate(
                    insertion_mm[0] + array_offset_mm[0] * rotation_cos
                        - array_offset_mm[1] * rotation_sin,
                )?,
                normalize_coordinate(
                    insertion_mm[1]
                        + array_offset_mm[0] * rotation_sin
                        + array_offset_mm[1] * rotation_cos,
                )?,
            ];
            for profile in &block.profiles {
                profiles.push(OrderedProfile {
                    layer: if profile.layer == "0" {
                        layer.clone()
                    } else {
                        profile.layer.clone()
                    },
                    ordinal,
                    style: profile.style.placed_on(&layer),
                    segments: profile
                        .segments
                        .iter()
                        .map(|segment| {
                            transform_segment(
                                segment,
                                block.base_mm,
                                array_insertion_mm,
                                [scale_x, scale_y],
                                rotation_sin,
                                rotation_cos,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    closed: profile.closed,
                });
            }
            texts.append(&mut text::place_texts(
                &block.texts,
                &layer,
                block.base_mm,
                array_insertion_mm,
                [scale_x, scale_y],
                rotation_sin,
                rotation_cos,
            )?);
        }
    }
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        "dxf.block-insert",
        Some(name.to_owned()),
        u32::try_from(placement_count)
            .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManyEntities)?,
    )?;
    Ok((profiles, texts))
}

fn parse_insert_attributes(
    insert_record: &[DxfPair<'_>],
    pairs: &[DxfPair<'_>],
    index: &mut usize,
    ordinal: &mut usize,
    entity_count: &mut usize,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedText>, DxfImportError> {
    match parse_optional_i64(insert_record, 66)? {
        None | Some(0) => return Ok(Vec::new()),
        Some(1) => {}
        Some(_) => return Err(DxfImportError::MalformedPairs),
    }
    let block_name = parse_required_text(insert_record, 2)?;
    let insert_layer = parse_layer(insert_record)?;
    let mut attribute_count = 0_u32;
    let mut texts = Vec::new();
    loop {
        if *index >= pairs.len() || pairs[*index].code != 0 || pairs[*index].value.is_empty() {
            return Err(DxfImportError::MalformedPairs);
        }
        *ordinal += 1;
        bump_entity_count(entity_count, diagnostics.reading.max_entities)?;
        let kind = pairs[*index].value.to_ascii_uppercase();
        let start = *index + 1;
        *index = start;
        while *index < pairs.len() && pairs[*index].code != 0 {
            *index += 1;
        }
        let record = &pairs[start..*index];
        match kind.as_str() {
            "ATTRIB" => {
                validate_insert_attribute(record)?;
                attribute_count = attribute_count
                    .checked_add(1)
                    .ok_or(DxfImportError::TooManyEntities)?;
                if diagnostics.reading.texts {
                    // An attribute it cannot read is counted; the insert's lines stay.
                    let read = text::parse_text("ATTRIB", record, units, diagnostics)
                        .and_then(|text| Ok((text, EntityStyle::read(record)?)));
                    match read {
                        Ok((text, style)) => texts.extend(text.map(|mut text| {
                            if text.layer == "0" {
                                text.layer.clone_from(&insert_layer);
                            }
                            text.style = style;
                            text
                        })),
                        Err(error) => diagnostics.add(
                            ImportDiagnosticSeverity::Warning,
                            "dxf.entity-unreadable",
                            Some(format!("ATTRIB: {error}")),
                            1,
                        )?,
                    }
                }
            }
            "SEQEND" => {
                if attribute_count == 0 {
                    return Err(DxfImportError::MalformedPairs);
                }
                parse_optional_layer(record)?;
                report_ignored_groups("SEQEND", record, &[5, 8, 100], diagnostics)?;
                break;
            }
            _ => return Err(DxfImportError::MalformedPairs),
        }
    }
    if !diagnostics.reading.texts {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.insert-attributes-dropped",
            Some(block_name.to_owned()),
            attribute_count,
        )?;
    }
    Ok(texts)
}

fn validate_insert_attribute(record: &[DxfPair<'_>]) -> Result<(), DxfImportError> {
    let mut tags = record.iter().filter(|pair| pair.code == 2);
    let tag = tags.next().ok_or(DxfImportError::MalformedPairs)?.value;
    if tags.next().is_some()
        || tag.is_empty()
        || tag.len() > 255
        || tag.chars().any(char::is_control)
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let mut values = record.iter().filter(|pair| pair.code == 1);
    values.next().ok_or(DxfImportError::MalformedPairs)?;
    if values.next().is_some() {
        return Err(DxfImportError::MalformedPairs);
    }
    parse_optional_layer(record)?;
    Ok(())
}

/// A point of a block where an INSERT places it: from the block's base, scaled, turned and
/// moved to the insertion point.
fn place_point(
    point: [f64; 2],
    base_mm: [f64; 2],
    insertion_mm: [f64; 2],
    scale: [f64; 2],
    rotation_sin: f64,
    rotation_cos: f64,
) -> Result<[f64; 2], DxfImportError> {
    let relative = [
        (point[0] - base_mm[0]) * scale[0],
        (point[1] - base_mm[1]) * scale[1],
    ];
    Ok([
        normalize_coordinate(
            insertion_mm[0] + relative[0] * rotation_cos - relative[1] * rotation_sin,
        )?,
        normalize_coordinate(
            insertion_mm[1] + relative[0] * rotation_sin + relative[1] * rotation_cos,
        )?,
    ])
}

fn transform_segment(
    segment: &ProfileSegment,
    base_mm: [f64; 2],
    insertion_mm: [f64; 2],
    scale: [f64; 2],
    rotation_sin: f64,
    rotation_cos: f64,
) -> Result<ProfileSegment, DxfImportError> {
    let transform = |point: [f64; 2]| {
        place_point(
            point,
            base_mm,
            insertion_mm,
            scale,
            rotation_sin,
            rotation_cos,
        )
    };
    match segment {
        ProfileSegment::Line { start_mm, end_mm } => {
            let start_mm = transform(*start_mm)?;
            let end_mm = transform(*end_mm)?;
            ensure_distinct(start_mm, end_mm)?;
            Ok(ProfileSegment::Line { start_mm, end_mm })
        }
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => {
            let start_mm = transform(*start_mm)?;
            let end_mm = transform(*end_mm)?;
            let center_mm = transform(*center_mm)?;
            ensure_distinct(start_mm, end_mm)?;
            let start_vector = [start_mm[0] - center_mm[0], start_mm[1] - center_mm[1]];
            let end_vector = [end_mm[0] - center_mm[0], end_mm[1] - center_mm[1]];
            let radius = start_vector[0].hypot(start_vector[1]);
            let transformed_clockwise = *clockwise ^ (scale[0] * scale[1] < 0.0);
            let major = if transformed_clockwise {
                cross2(start_vector, end_vector) > 0.0
            } else {
                cross2(start_vector, end_vector) < 0.0
            };
            validate_arc_sweep_envelope(
                start_mm,
                end_mm,
                center_mm,
                radius,
                transformed_clockwise,
                major,
            )?;
            Ok(ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise: transformed_clockwise,
            })
        }
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => Ok(ProfileSegment::CubicBezier {
            start_mm: transform(*start_mm)?,
            control_1_mm: transform(*control_1_mm)?,
            control_2_mm: transform(*control_2_mm)?,
            end_mm: transform(*end_mm)?,
        }),
        ProfileSegment::Spline { points_mm } => Ok(ProfileSegment::Spline {
            points_mm: points_mm
                .iter()
                .map(|point| transform(*point))
                .collect::<Result<_, _>>()?,
        }),
    }
}

fn parse_polyline(
    record: &[DxfPair<'_>],
    pairs: &[DxfPair<'_>],
    index: &mut usize,
    ordinal: &mut usize,
    entity_count: &mut usize,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let profile_ordinal = *ordinal;
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "POLYLINE",
        record,
        &[
            5, 8, 10, 20, 30, 39, 66, 70, 71, 72, 73, 74, 75, 100, 210, 220, 230,
        ],
        diagnostics,
    )?;
    if parse_optional_i64(record, 66)?.is_some_and(|value| value != 1) {
        return Err(DxfImportError::MalformedPairs);
    }
    if parse_optional_number(record, 10)?.is_some_and(|value| value != 0.0)
        || parse_optional_number(record, 20)?.is_some_and(|value| value != 0.0)
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let flags = parse_optional_i64(record, 70)?.unwrap_or(0);
    if flags & !(1 | 8 | 16 | 32 | 64 | 128) != 0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let polygon_mesh = flags & 16 != 0;
    let polyface = flags & 64 != 0;
    if (polygon_mesh && (polyface || flags & 8 != 0))
        || (polyface && flags & (1 | 8 | 16 | 32) != 0)
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let has_mesh_metadata = record.iter().any(|pair| matches!(pair.code, 71..=75));
    if (!polygon_mesh && !polyface && has_mesh_metadata)
        || (polyface && record.iter().any(|pair| matches!(pair.code, 73..=75)))
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let three_dimensional = flags & 8 != 0;
    if flags & 128 != 0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.polyline-generation-flag-ignored",
            Some(layer.clone()),
            1,
        )?;
    }
    let closed = flags & 1 != 0;
    let mut vertices = Vec::new();
    let mut polygon_mesh_vertices = Vec::new();
    let mut polyface_vertices = Vec::new();
    let mut polyface_faces = Vec::new();
    let mut saw_polyface_face = false;
    let mut saw_seqend = false;
    while *index < pairs.len() {
        if pairs[*index].code != 0 || pairs[*index].value.is_empty() {
            return Err(DxfImportError::MalformedPairs);
        }
        *ordinal += 1;
        bump_entity_count(entity_count, diagnostics.reading.max_entities)?;
        let kind = pairs[*index].value.to_ascii_uppercase();
        let start = *index + 1;
        *index = start;
        while *index < pairs.len() && pairs[*index].code != 0 {
            *index += 1;
        }
        let child = &pairs[start..*index];
        match kind.as_str() {
            "VERTEX" if polygon_mesh => {
                polygon_mesh_vertices.push(parse_polygon_mesh_vertex(child, &layer, diagnostics)?);
            }
            "VERTEX" if polyface => match parse_polyface_vertex(child, &layer, diagnostics)? {
                PolyfaceRecord::Coordinate(point) => {
                    if saw_polyface_face {
                        return Err(DxfImportError::MalformedPairs);
                    }
                    polyface_vertices.push(point);
                }
                PolyfaceRecord::Face { indices, hidden } => {
                    saw_polyface_face = true;
                    if hidden > 0 {
                        diagnostics.add(
                            ImportDiagnosticSeverity::Warning,
                            "dxf.polyface-invisible-edge-dropped",
                            Some(layer.clone()),
                            hidden,
                        )?;
                    }
                    polyface_faces.push(indices);
                }
            },
            "VERTEX" => {
                if vertices.len() > diagnostics.reading.max_segments {
                    return Err(DxfImportError::TooManySegments);
                }
                vertices.push(parse_polyline_vertex(
                    child,
                    &layer,
                    three_dimensional,
                    diagnostics,
                )?);
            }
            "SEQEND" => {
                ensure_matching_optional_layer(child, &layer)?;
                report_ignored_groups("SEQEND", child, &[5, 8, 100], diagnostics)?;
                saw_seqend = true;
                break;
            }
            _ => return Err(DxfImportError::MalformedPairs),
        }
    }
    if !saw_seqend {
        return Err(DxfImportError::MalformedPairs);
    }
    if polygon_mesh {
        return finish_polygon_mesh(
            record,
            layer,
            profile_ordinal,
            polygon_mesh_vertices,
            flags,
            units,
            diagnostics,
        );
    }
    if polyface {
        return finish_polyface(
            record,
            layer,
            profile_ordinal,
            polyface_vertices,
            polyface_faces,
            units,
            diagnostics,
        );
    }
    if vertices.len() < 2 {
        return Err(DxfImportError::MalformedPairs);
    }
    Ok(vec![build_polyline_profile(
        layer,
        profile_ordinal,
        vertices,
        closed,
        units,
        diagnostics,
    )?])
}

fn parse_polygon_mesh_vertex(
    record: &[DxfPair<'_>],
    layer: &str,
    diagnostics: &mut DiagnosticCounts,
) -> Result<[f64; 2], DxfImportError> {
    ensure_matching_optional_layer(record, layer)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "VERTEX",
        record,
        &[5, 8, 10, 20, 30, 42, 70, 100, 210, 220, 230],
        diagnostics,
    )?;
    if parse_required_i64(record, 70)? != 64 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    if parse_optional_number(record, 42)?.is_some_and(|bulge| bulge != 0.0) {
        return Err(DxfImportError::InvalidBulge);
    }
    Ok([
        parse_required_number(record, 10)?,
        parse_required_number(record, 20)?,
    ])
}

fn finish_polygon_mesh(
    header: &[DxfPair<'_>],
    layer: String,
    ordinal: usize,
    native_vertices: Vec<[f64; 2]>,
    flags: i64,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    if parse_optional_i64(header, 73)?.unwrap_or(0) != 0
        || parse_optional_i64(header, 74)?.unwrap_or(0) != 0
        || parse_optional_i64(header, 75)?.unwrap_or(0) != 0
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let m_count = parse_required_i64(header, 71)?;
    let n_count = parse_required_i64(header, 72)?;
    if m_count < 2 || n_count < 2 {
        return Err(DxfImportError::MalformedPairs);
    }
    let m_count = usize::try_from(m_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let n_count = usize::try_from(n_count)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let vertex_count = m_count
        .checked_mul(n_count)
        .ok_or(DxfImportError::TooManyEntities)?;
    if vertex_count > diagnostics.reading.max_entities {
        return Err(DxfImportError::TooManyEntities);
    }
    if native_vertices.len() != vertex_count {
        return Err(DxfImportError::MalformedPairs);
    }

    let m_closed = flags & 1 != 0;
    let n_closed = flags & 32 != 0;
    if m_closed && n_closed {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let max_boundary_segments = if m_closed {
        m_count
    } else if n_closed {
        n_count
    } else {
        2_usize
            .checked_mul(
                m_count
                    .checked_sub(1)
                    .and_then(|m| n_count.checked_sub(1).and_then(|n| m.checked_add(n)))
                    .ok_or(DxfImportError::TooManySegments)?,
            )
            .ok_or(DxfImportError::TooManySegments)?
    };
    if max_boundary_segments > diagnostics.reading.max_segments {
        return Err(DxfImportError::TooManySegments);
    }

    let scale = units.millimetres_per_unit();
    let mut vertices = Vec::with_capacity(native_vertices.len());
    let mut unique_vertices = BTreeSet::new();
    for point in native_vertices {
        let point = [
            scale_coordinate(point[0], scale)?,
            scale_coordinate(point[1], scale)?,
        ];
        if !unique_vertices.insert(point_key(point)) {
            return Err(DxfImportError::AmbiguousGeometry);
        }
        vertices.push(point);
    }

    let m_steps = if m_closed { m_count } else { m_count - 1 };
    let n_steps = if n_closed { n_count } else { n_count - 1 };
    let mut faces = Vec::with_capacity(m_steps * n_steps);
    for m in 0..m_steps {
        let next_m = (m + 1) % m_count;
        for n in 0..n_steps {
            let next_n = (n + 1) % n_count;
            let face = vec![
                m * n_count + n,
                next_m * n_count + n,
                next_m * n_count + next_n,
                m * n_count + next_n,
            ];
            let points = face
                .iter()
                .map(|index| vertices[*index])
                .collect::<Vec<_>>();
            validate_mesh_face(&points)?;
            faces.push(face);
        }
    }
    build_mesh_boundary_profiles(
        layer,
        ordinal,
        vertices,
        faces,
        "dxf.polygon-mesh-surface-topology-dropped",
        "dxf.polygon-mesh-boundary-geometry",
        diagnostics,
    )
}

enum PolyfaceRecord {
    Coordinate([f64; 2]),
    Face { indices: Vec<usize>, hidden: u32 },
}

fn parse_polyface_vertex(
    record: &[DxfPair<'_>],
    layer: &str,
    diagnostics: &mut DiagnosticCounts,
) -> Result<PolyfaceRecord, DxfImportError> {
    ensure_matching_optional_layer(record, layer)?;
    report_ignored_groups(
        "VERTEX",
        record,
        &[5, 8, 10, 20, 30, 70, 71, 72, 73, 74, 100],
        diagnostics,
    )?;
    let flags = parse_required_i64(record, 70)?;
    match flags {
        64 | 192 => {
            ensure_planar(record)?;
            if record.iter().any(|pair| matches!(pair.code, 71..=74)) {
                return Err(DxfImportError::MalformedPairs);
            }
            Ok(PolyfaceRecord::Coordinate([
                parse_required_number(record, 10)?,
                parse_required_number(record, 20)?,
            ]))
        }
        128 => {
            if parse_optional_number(record, 10)?.is_some_and(|value| value != 0.0)
                || parse_optional_number(record, 20)?.is_some_and(|value| value != 0.0)
                || parse_optional_number(record, 30)?.is_some_and(|value| value != 0.0)
            {
                return Err(DxfImportError::AmbiguousGeometry);
            }
            let raw_indices = [71_i16, 72, 73, 74]
                .into_iter()
                .map(|code| parse_optional_i64(record, code))
                .collect::<Result<Vec<_>, _>>()?;
            let mut indices = Vec::with_capacity(4);
            let mut hidden = 0_u32;
            let mut ended = false;
            for raw_index in raw_indices {
                let raw_index = raw_index.unwrap_or(0);
                if raw_index == 0 {
                    ended = true;
                    continue;
                }
                if ended {
                    return Err(DxfImportError::MalformedPairs);
                }
                if raw_index < 0 {
                    hidden += 1;
                }
                let one_based = raw_index
                    .checked_abs()
                    .ok_or(DxfImportError::MalformedPairs)?;
                let one_based = usize::try_from(one_based)
                    .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
                indices.push(
                    one_based
                        .checked_sub(1)
                        .ok_or(DxfImportError::MalformedPairs)?,
                );
            }
            if !matches!(indices.len(), 3 | 4) {
                return Err(DxfImportError::MalformedPairs);
            }
            Ok(PolyfaceRecord::Face { indices, hidden })
        }
        _ => Err(DxfImportError::AmbiguousGeometry),
    }
}

fn finish_polyface(
    header: &[DxfPair<'_>],
    layer: String,
    ordinal: usize,
    native_vertices: Vec<[f64; 2]>,
    faces: Vec<Vec<usize>>,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let declared_vertices = parse_required_i64(header, 71)?;
    let declared_faces = parse_required_i64(header, 72)?;
    if declared_vertices < 3 || declared_faces < 1 {
        return Err(DxfImportError::MalformedPairs);
    }
    let declared_vertices = usize::try_from(declared_vertices)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    let declared_faces = usize::try_from(declared_faces)
        .map_err(|_: std::num::TryFromIntError| DxfImportError::MalformedPairs)?;
    if native_vertices.len() != declared_vertices || faces.len() != declared_faces {
        return Err(DxfImportError::MalformedPairs);
    }
    let scale = units.millimetres_per_unit();
    let mut vertices = Vec::with_capacity(native_vertices.len());
    let mut unique_vertices = BTreeSet::new();
    for point in native_vertices {
        let point = [
            scale_coordinate(point[0], scale)?,
            scale_coordinate(point[1], scale)?,
        ];
        if !unique_vertices.insert(point_key(point)) {
            return Err(DxfImportError::AmbiguousGeometry);
        }
        vertices.push(point);
    }
    for face in &faces {
        let mut unique_indices = BTreeSet::new();
        for index in face {
            if *index >= vertices.len() || !unique_indices.insert(*index) {
                return Err(DxfImportError::AmbiguousGeometry);
            }
        }
        let points = face
            .iter()
            .map(|index| vertices[*index])
            .collect::<Vec<_>>();
        validate_mesh_face(&points)?;
    }
    build_mesh_boundary_profiles(
        layer,
        ordinal,
        vertices,
        faces,
        "dxf.polyface-face-topology-dropped",
        "dxf.polyface-boundary-geometry",
        diagnostics,
    )
}

fn parse_polyline_vertex(
    record: &[DxfPair<'_>],
    layer: &str,
    three_dimensional: bool,
    diagnostics: &mut DiagnosticCounts,
) -> Result<PolylineVertex, DxfImportError> {
    ensure_matching_optional_layer(record, layer)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "VERTEX",
        record,
        &[5, 8, 10, 20, 30, 42, 70, 100, 210, 220, 230],
        diagnostics,
    )?;
    let flags = parse_optional_i64(record, 70)?.unwrap_or(0);
    if flags != if three_dimensional { 32 } else { 0 } {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let bulge = parse_optional_number(record, 42)?;
    if three_dimensional && bulge.is_some_and(|value| value != 0.0) {
        return Err(DxfImportError::InvalidBulge);
    }
    Ok(PolylineVertex {
        x: parse_required_number(record, 10)?,
        y: Some(parse_required_number(record, 20)?),
        bulge: if three_dimensional { None } else { bulge },
    })
}

#[derive(Clone, Copy)]
struct PolylineVertex {
    x: f64,
    y: Option<f64>,
    bulge: Option<f64>,
}

fn parse_lwpolyline(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<OrderedProfile, DxfImportError> {
    let layer = parse_layer(record)?;
    let mirrored = ensure_planar_ocs(record)?;
    report_ignored_groups(
        "LWPOLYLINE",
        record,
        &[5, 8, 10, 20, 30, 38, 39, 42, 70, 90, 100, 210, 220, 230],
        diagnostics,
    )?;
    let declared_count = parse_required_i64(record, 90)?;
    if declared_count < 2 || declared_count as usize > diagnostics.reading.max_segments + 1 {
        return Err(DxfImportError::TooManySegments);
    }
    let flags = parse_optional_i64(record, 70)?.unwrap_or(0);
    if flags & !129 != 0 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    if flags & 128 != 0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.polyline-generation-flag-ignored",
            Some(layer.clone()),
            1,
        )?;
    }
    let closed = flags & 1 != 0;
    let mut vertices = Vec::new();
    for pair in record {
        match pair.code {
            10 => vertices.push(PolylineVertex {
                x: parse_number(pair.value)?,
                y: None,
                bulge: None,
            }),
            20 => {
                let vertex = vertices.last_mut().ok_or(DxfImportError::MalformedPairs)?;
                if vertex.y.replace(parse_number(pair.value)?).is_some() {
                    return Err(DxfImportError::MalformedPairs);
                }
            }
            42 => {
                let vertex = vertices.last_mut().ok_or(DxfImportError::MalformedPairs)?;
                if vertex.bulge.is_some() {
                    return Err(DxfImportError::MalformedPairs);
                }
                vertex.bulge = Some(parse_number(pair.value)?);
            }
            _ => {}
        }
    }
    if vertices.len() != declared_count as usize || vertices.iter().any(|vertex| vertex.y.is_none())
    {
        return Err(DxfImportError::MalformedPairs);
    }
    let mut profile = build_polyline_profile(layer, ordinal, vertices, closed, units, diagnostics)?;
    if mirrored {
        profile.segments = profile.segments.into_iter().map(mirror_ocs).collect();
    }
    Ok(profile)
}

fn build_polyline_profile(
    layer: String,
    ordinal: usize,
    vertices: Vec<PolylineVertex>,
    closed: bool,
    units: ImportUnitDecision,
    diagnostics: &mut DiagnosticCounts,
) -> Result<OrderedProfile, DxfImportError> {
    let scale = units.millimetres_per_unit();
    let points = vertices
        .iter()
        .map(|vertex| {
            Ok([
                scale_coordinate(vertex.x, scale)?,
                scale_coordinate(vertex.y.expect("validated polyline Y"), scale)?,
            ])
        })
        .collect::<Result<Vec<_>, DxfImportError>>()?;
    let read = points.len();
    let (points, vertices) = without_repeated_vertices(points, vertices, closed);
    let removed = u32::try_from(read - points.len())
        .map_err(|_: std::num::TryFromIntError| DxfImportError::TooManySegments)?;
    if removed > 0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.repeated-vertex-removed",
            Some(layer.clone()),
            removed,
        )?;
    }
    if points.len() < 2 {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let edge_count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    if edge_count > diagnostics.reading.max_segments {
        return Err(DxfImportError::TooManySegments);
    }
    if !closed
        && vertices
            .last()
            .and_then(|vertex| vertex.bulge)
            .is_some_and(|bulge| bulge != 0.0)
    {
        diagnostics.add(
            ImportDiagnosticSeverity::Warning,
            "dxf.open-polyline-terminal-bulge-ignored",
            Some(layer.clone()),
            1,
        )?;
    }
    let mut segments = Vec::with_capacity(edge_count);
    for index in 0..edge_count {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        ensure_distinct(start, end)?;
        segments.push(segment_from_bulge(
            start,
            end,
            vertices[index].bulge.unwrap_or(0.0),
        )?);
    }
    Ok(OrderedProfile {
        layer,
        ordinal,
        style: EntityStyle::default(),
        segments,
        closed,
    })
}

fn segment_from_bulge(
    start: [f64; 2],
    end: [f64; 2],
    bulge: f64,
) -> Result<ProfileSegment, DxfImportError> {
    if bulge == 0.0 {
        return Ok(ProfileSegment::Line {
            start_mm: start,
            end_mm: end,
        });
    }
    if !bulge.is_finite() {
        return Err(DxfImportError::InvalidBulge);
    }
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let chord_squared = dx * dx + dy * dy;
    if !chord_squared.is_finite()
        || chord_squared <= DXF_GEOMETRY_EPSILON_MM * DXF_GEOMETRY_EPSILON_MM
    {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let center_factor = (1.0 - bulge * bulge) / (4.0 * bulge);
    let center = [
        normalize_coordinate((start[0] + end[0]) * 0.5 - dy * center_factor)?,
        normalize_coordinate((start[1] + end[1]) * 0.5 + dx * center_factor)?,
    ];
    let radius_dx = start[0] - center[0];
    let radius_dy = start[1] - center[1];
    let radius_squared = radius_dx * radius_dx + radius_dy * radius_dy;
    if !radius_squared.is_finite()
        || radius_squared <= DXF_GEOMETRY_EPSILON_MM * DXF_GEOMETRY_EPSILON_MM
    {
        return Err(DxfImportError::InvalidBulge);
    }
    validate_arc_sweep_envelope(
        start,
        end,
        center,
        radius_squared.sqrt(),
        bulge < 0.0,
        bulge.abs() > 1.0,
    )?;
    Ok(ProfileSegment::CircularArc {
        start_mm: start,
        end_mm: end,
        center_mm: center,
        clockwise: bulge < 0.0,
    })
}

fn parse_required_text<'a>(
    record: &'a [DxfPair<'_>],
    code: i16,
) -> Result<&'a str, DxfImportError> {
    parse_optional_text(record, code)?.ok_or(DxfImportError::InvalidBlock)
}

fn parse_optional_text<'a>(
    record: &'a [DxfPair<'_>],
    code: i16,
) -> Result<Option<&'a str>, DxfImportError> {
    let mut values = record.iter().filter(|pair| pair.code == code);
    let value = values.next().map(|pair| pair.value);
    if values.next().is_some() {
        return Err(DxfImportError::InvalidBlock);
    }
    Ok(value)
}

/// The key of a block name: in capitals, each `\U+XXXX` read as its character (a DXF
/// older than 2007 writes so a character outside its code page, `Profondit\U+00E0`).
/// No other backslash, no slash and no control character belongs in a name.
fn block_name_key(name: &str) -> Result<String, DxfImportError> {
    let mut key = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(at) = rest.find('\\') {
        key.push_str(&rest[..at]);
        let character = rest[at + 1..]
            .strip_prefix("U+")
            .and_then(|code| code.get(..4))
            .filter(|hex| hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32)
            .ok_or(DxfImportError::InvalidBlock)?;
        key.push(character);
        rest = &rest[at + "\\U+XXXX".len()..];
    }
    key.push_str(rest);
    if key.is_empty()
        || name.len() > 255
        || key.chars().any(char::is_control)
        || key.contains(['/', '\\'])
    {
        return Err(DxfImportError::InvalidBlock);
    }
    Ok(key.to_ascii_uppercase())
}

fn parse_layer(record: &[DxfPair<'_>]) -> Result<String, DxfImportError> {
    Ok(parse_optional_layer(record)?.unwrap_or_else(|| "0".to_owned()))
}

fn parse_optional_layer(record: &[DxfPair<'_>]) -> Result<Option<String>, DxfImportError> {
    let mut values = record.iter().filter(|pair| pair.code == 8);
    let layer = values.next().map(|pair| pair.value);
    if values.next().is_some() {
        return Err(DxfImportError::MalformedPairs);
    }
    layer
        .map(|layer| {
            if layer.is_empty()
                || layer.len() > 255
                || layer.chars().any(char::is_control)
                || layer.contains(['/', '\\'])
            {
                return Err(DxfImportError::MalformedPairs);
            }
            Ok(layer.to_owned())
        })
        .transpose()
}

fn ensure_matching_optional_layer(
    record: &[DxfPair<'_>],
    expected: &str,
) -> Result<(), DxfImportError> {
    if parse_optional_layer(record)?.is_some_and(|layer| layer != expected) {
        Err(DxfImportError::AmbiguousGeometry)
    } else {
        Ok(())
    }
}

fn ensure_planar(record: &[DxfPair<'_>]) -> Result<(), DxfImportError> {
    for pair in record {
        match pair.code {
            30 | 31 | 32 | 33 | 38 | 39 if parse_number(pair.value)? != 0.0 => {
                return Err(DxfImportError::NonPlanarGeometry);
            }
            210 | 220 if parse_number(pair.value)? != 0.0 => {
                return Err(DxfImportError::NonPlanarGeometry);
            }
            230 if parse_number(pair.value)? != 1.0 => {
                return Err(DxfImportError::NonPlanarGeometry);
            }
            _ => {}
        }
    }
    Ok(())
}

/// Like `ensure_planar` for an entity whose points are in its object coordinate
/// system: extrusion (0, 0, -1), which AutoCAD's Mirror leaves on arcs, circles
/// and polylines, is accepted. Returns whether the entity is so mirrored.
fn ensure_planar_ocs(record: &[DxfPair<'_>]) -> Result<bool, DxfImportError> {
    let mirrored = record
        .iter()
        .rev()
        .find(|pair| pair.code == 230)
        .map(|pair| parse_number(pair.value))
        .transpose()?
        == Some(-1.0);
    if mirrored {
        let upright = record
            .iter()
            .filter(|pair| pair.code != 230)
            .copied()
            .collect::<Vec<_>>();
        ensure_planar(&upright)?;
    } else {
        ensure_planar(record)?;
    }
    Ok(mirrored)
}

/// A segment from an object coordinate system with extrusion (0, 0, -1) in world
/// coordinates: the arbitrary axis algorithm turns its x axis to world -x.
fn mirror_ocs(segment: ProfileSegment) -> ProfileSegment {
    // `0.0 - x` and not `-x`, so that 0 stays +0 and joins bit for bit.
    let flip = |point: [f64; 2]| [0.0 - point[0], point[1]];
    match segment {
        ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
            start_mm: flip(start_mm),
            end_mm: flip(end_mm),
        },
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => ProfileSegment::CircularArc {
            start_mm: flip(start_mm),
            end_mm: flip(end_mm),
            center_mm: flip(center_mm),
            clockwise: !clockwise,
        },
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => ProfileSegment::CubicBezier {
            start_mm: flip(start_mm),
            control_1_mm: flip(control_1_mm),
            control_2_mm: flip(control_2_mm),
            end_mm: flip(end_mm),
        },
        ProfileSegment::Spline { points_mm } => ProfileSegment::Spline {
            points_mm: points_mm.into_iter().map(flip).collect(),
        },
    }
}

fn report_ignored_groups(
    entity: &str,
    record: &[DxfPair<'_>],
    accepted: &[i16],
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    for pair in record {
        let style = diagnostics.reading.styles && style::STYLE_GROUPS.contains(&pair.code);
        if !accepted.contains(&pair.code) && !style {
            diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                "dxf.entity-group-ignored",
                Some(format!("{entity}:{}", pair.code)),
                1,
            )?;
        }
    }
    Ok(())
}

fn parse_xy(
    record: &[DxfPair<'_>],
    x_code: i16,
    y_code: i16,
    scale: f64,
) -> Result<[f64; 2], DxfImportError> {
    Ok([
        scale_coordinate(parse_required_number(record, x_code)?, scale)?,
        scale_coordinate(parse_required_number(record, y_code)?, scale)?,
    ])
}

fn parse_required_number(record: &[DxfPair<'_>], code: i16) -> Result<f64, DxfImportError> {
    parse_optional_number(record, code)?.ok_or(DxfImportError::MalformedPairs)
}

fn parse_optional_number(record: &[DxfPair<'_>], code: i16) -> Result<Option<f64>, DxfImportError> {
    let mut values = record.iter().filter(|pair| pair.code == code);
    let value = values.next().map(|pair| pair.value);
    if values.next().is_some() {
        return Err(DxfImportError::MalformedPairs);
    }
    value.map(parse_number).transpose()
}

fn parse_required_i64(record: &[DxfPair<'_>], code: i16) -> Result<i64, DxfImportError> {
    parse_optional_i64(record, code)?.ok_or(DxfImportError::MalformedPairs)
}

fn parse_optional_i64(record: &[DxfPair<'_>], code: i16) -> Result<Option<i64>, DxfImportError> {
    let mut values = record.iter().filter(|pair| pair.code == code);
    let value = values.next().map(|pair| pair.value);
    if values.next().is_some() {
        return Err(DxfImportError::MalformedPairs);
    }
    value.map(parse_integer::<i64>).transpose()
}

fn parse_number(value: &str) -> Result<f64, DxfImportError> {
    let number = value
        .parse::<f64>()
        .map_err(|cause| DxfImportError::NotANumber {
            value: value.to_owned(),
            cause,
        })?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(DxfImportError::InvalidNumber)
    }
}

fn parse_integer<T: std::str::FromStr<Err = std::num::ParseIntError>>(
    value: &str,
) -> Result<T, DxfImportError> {
    value.parse().map_err(|cause| DxfImportError::NotAnInteger {
        value: value.to_owned(),
        cause,
    })
}

fn scale_coordinate(value: f64, scale: f64) -> Result<f64, DxfImportError> {
    normalize_coordinate(value * scale)
}

fn deterministic_sin_cos_degrees(degrees: f64) -> (f64, f64) {
    let normalized = degrees.rem_euclid(360.0);
    // `rem_euclid` of a tiny negative angle rounds up to exactly 360.
    let normalized = if normalized >= 360.0 { 0.0 } else { normalized };
    let quadrant = (normalized / 90.0).floor() as u8;
    let offset = normalized - f64::from(quadrant) * 90.0;
    let reflected = offset > 45.0;
    let reduced = if reflected { 90.0 - offset } else { offset };
    let radians = reduced * (std::f64::consts::PI / 180.0);
    let squared = radians * radians;
    let mut sine_polynomial = -1.0 / 121_645_100_408_832_000.0;
    sine_polynomial = 1.0 / 355_687_428_096_000.0 + squared * sine_polynomial;
    sine_polynomial = -1.0 / 1_307_674_368_000.0 + squared * sine_polynomial;
    sine_polynomial = 1.0 / 6_227_020_800.0 + squared * sine_polynomial;
    sine_polynomial = -1.0 / 39_916_800.0 + squared * sine_polynomial;
    sine_polynomial = 1.0 / 362_880.0 + squared * sine_polynomial;
    sine_polynomial = -1.0 / 5_040.0 + squared * sine_polynomial;
    sine_polynomial = 1.0 / 120.0 + squared * sine_polynomial;
    sine_polynomial = -1.0 / 6.0 + squared * sine_polynomial;
    let sine = radians * (1.0 + squared * sine_polynomial);

    let mut cosine_polynomial = -1.0 / 6_402_373_705_728_000.0;
    cosine_polynomial = 1.0 / 20_922_789_888_000.0 + squared * cosine_polynomial;
    cosine_polynomial = -1.0 / 87_178_291_200.0 + squared * cosine_polynomial;
    cosine_polynomial = 1.0 / 479_001_600.0 + squared * cosine_polynomial;
    cosine_polynomial = -1.0 / 3_628_800.0 + squared * cosine_polynomial;
    cosine_polynomial = 1.0 / 40_320.0 + squared * cosine_polynomial;
    cosine_polynomial = -1.0 / 720.0 + squared * cosine_polynomial;
    cosine_polynomial = 1.0 / 24.0 + squared * cosine_polynomial;
    cosine_polynomial = -1.0 / 2.0 + squared * cosine_polynomial;
    let cosine = 1.0 + squared * cosine_polynomial;
    let (sine, cosine) = if reflected {
        (cosine, sine)
    } else {
        (sine, cosine)
    };
    match quadrant {
        0 => (sine, cosine),
        1 => (cosine, -sine),
        2 => (-sine, -cosine),
        _ => (-cosine, sine),
    }
}

thread_local! {
    /// The largest coordinate a reading on this thread keeps: the canonical envelope,
    /// except while an underlay is read ([`AnyFiniteCoordinate`]).
    static COORDINATE_LIMIT_MM: std::cell::Cell<f64> = const {
        std::cell::Cell::new(MAX_COORDINATE_MM)
    };
}

/// While it lives, a reading on this thread keeps every finite coordinate: an underlay's
/// lines are moved into the envelope after they are read
/// ([`crate::underlay::UnderlayLines::move_near_origin`]).
struct AnyFiniteCoordinate(f64);

impl AnyFiniteCoordinate {
    fn new() -> Self {
        Self(COORDINATE_LIMIT_MM.replace(f64::MAX))
    }
}

impl Drop for AnyFiniteCoordinate {
    fn drop(&mut self) {
        COORDINATE_LIMIT_MM.set(self.0);
    }
}

fn normalize_coordinate(value: f64) -> Result<f64, DxfImportError> {
    if !value.is_finite() || value.abs() > COORDINATE_LIMIT_MM.get() {
        return Err(DxfImportError::CoordinateOutOfRange);
    }
    Ok(if value == 0.0 { 0.0 } else { value })
}

/// `points`, read from `vertices`, without a point at the one before it, nor, of a closed
/// polyline, a last point at the first: the zero-length edge between them is dropped, and
/// the kept point's edge onwards takes the bulge of the dropped point's.
fn without_repeated_vertices(
    points: Vec<[f64; 2]>,
    vertices: Vec<PolylineVertex>,
    closed: bool,
) -> (Vec<[f64; 2]>, Vec<PolylineVertex>) {
    let repeated = |left: [f64; 2], right: [f64; 2]| ensure_distinct(left, right).is_err();
    let mut kept_points: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    let mut kept_vertices: Vec<PolylineVertex> = Vec::with_capacity(vertices.len());
    for (point, vertex) in points.into_iter().zip(vertices) {
        if let (Some(&last), Some(kept)) = (kept_points.last(), kept_vertices.last_mut())
            && repeated(last, point)
        {
            kept.bulge = vertex.bulge;
            continue;
        }
        kept_points.push(point);
        kept_vertices.push(vertex);
    }
    while closed
        && kept_points.len() > 1
        && repeated(kept_points[0], kept_points[kept_points.len() - 1])
    {
        kept_points.pop();
        kept_vertices.pop();
    }
    (kept_points, kept_vertices)
}

fn ensure_distinct(start: [f64; 2], end: [f64; 2]) -> Result<(), DxfImportError> {
    if (start[0] - end[0]).hypot(start[1] - end[1]) <= DXF_GEOMETRY_EPSILON_MM {
        Err(DxfImportError::DegenerateGeometry)
    } else {
        Ok(())
    }
}

fn validate_simple_line_polygon(points: &[[f64; 2]]) -> Result<(), DxfImportError> {
    if !(3..=4).contains(&points.len()) {
        return Err(DxfImportError::DegenerateGeometry);
    }
    for left in 0..points.len() {
        for right in left + 1..points.len() {
            if (points[left][0] - points[right][0]).hypot(points[left][1] - points[right][1])
                <= DXF_GEOMETRY_EPSILON_MM
            {
                return Err(DxfImportError::AmbiguousGeometry);
            }
        }
    }
    if points.len() == 4
        && (line_segments_intersect(points[0], points[1], points[2], points[3])
            || line_segments_intersect(points[1], points[2], points[3], points[0]))
    {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    let twice_area = points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .map(|(start, end)| cross2(start, end))
        .sum::<f64>();
    if twice_area.abs() <= DXF_GEOMETRY_EPSILON_MM * DXF_GEOMETRY_EPSILON_MM {
        return Err(DxfImportError::DegenerateGeometry);
    }
    Ok(())
}

fn line_segments_intersect(
    first_start: [f64; 2],
    first_end: [f64; 2],
    second_start: [f64; 2],
    second_end: [f64; 2],
) -> bool {
    let orientation = |start: [f64; 2], end: [f64; 2], point: [f64; 2]| {
        cross2(
            [end[0] - start[0], end[1] - start[1]],
            [point[0] - start[0], point[1] - start[1]],
        )
    };
    let tolerance = DXF_GEOMETRY_EPSILON_MM * DXF_GEOMETRY_EPSILON_MM;
    let first_to_second_start = orientation(first_start, first_end, second_start);
    let first_to_second_end = orientation(first_start, first_end, second_end);
    let second_to_first_start = orientation(second_start, second_end, first_start);
    let second_to_first_end = orientation(second_start, second_end, first_end);
    if [
        first_to_second_start,
        first_to_second_end,
        second_to_first_start,
        second_to_first_end,
    ]
    .iter()
    .any(|value| value.abs() <= tolerance)
    {
        return true;
    }
    (first_to_second_start.is_sign_positive() != first_to_second_end.is_sign_positive())
        && (second_to_first_start.is_sign_positive() != second_to_first_end.is_sign_positive())
}

fn validate_arc_sweep_envelope(
    start: [f64; 2],
    end: [f64; 2],
    center: [f64; 2],
    radius: f64,
    clockwise: bool,
    major: bool,
) -> Result<(), DxfImportError> {
    let start_vector = [start[0] - center[0], start[1] - center[1]];
    let end_vector = [end[0] - center[0], end[1] - center[1]];
    for direction in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
        if directed_arc_contains(
            start_vector,
            end_vector,
            direction,
            clockwise,
            major,
            radius,
        ) {
            normalize_coordinate(center[0] + radius * direction[0])?;
            normalize_coordinate(center[1] + radius * direction[1])?;
        }
    }
    Ok(())
}

fn directed_arc_contains(
    start: [f64; 2],
    end: [f64; 2],
    direction: [f64; 2],
    clockwise: bool,
    major: bool,
    radius: f64,
) -> bool {
    let (from, to) = if clockwise {
        (end, start)
    } else {
        (start, end)
    };
    let tolerance = DXF_GEOMETRY_EPSILON_MM * radius.max(1.0);
    if major {
        !(cross2(to, direction) > tolerance && cross2(direction, from) > tolerance)
    } else {
        cross2(from, direction) >= -tolerance && cross2(direction, to) >= -tolerance
    }
}

fn arc_endpoint_buckets(segments: &[LooseSegment]) -> BTreeMap<[i64; 2], Vec<[f64; 2]>> {
    let mut arc_endpoints: BTreeMap<[i64; 2], Vec<[f64; 2]>> = BTreeMap::new();
    for segment in segments {
        if matches!(segment.segment, ProfileSegment::CircularArc { .. }) {
            for point in [segment.segment.start_mm(), segment.segment.end_mm()] {
                arc_endpoints
                    .entry(connectivity_bucket(point))
                    .or_default()
                    .push(point);
            }
        }
    }
    arc_endpoints
}

/// Snaps the ends of `segments` that lie within the tolerance of an arc end onto it, and
/// takes out, in their order, those whose ends it cannot snap unambiguously, each with why.
fn snap_lines_to_unambiguous_arc_endpoints(
    segments: &mut Vec<LooseSegment>,
) -> Vec<(LooseSegment, DxfImportError)> {
    // Arc ends come from a centre, a radius and angles, so two arcs with
    // different centres rarely meet bit for bit. Two ends within the tolerance
    // become one of them, the same one for both.
    let mut removed = Vec::new();
    let arc_endpoints = arc_endpoint_buckets(segments);
    let mut failed = BTreeMap::new();
    for (index, segment) in segments.iter_mut().enumerate() {
        if let ProfileSegment::CircularArc {
            start_mm, end_mm, ..
        } = &mut segment.segment
        {
            let snapped = shared_arc_endpoint(*start_mm, &arc_endpoints).and_then(|start| {
                let end = shared_arc_endpoint(*end_mm, &arc_endpoints)?;
                ensure_distinct(start, end)?;
                Ok((start, end))
            });
            match snapped {
                Ok((start, end)) => (*start_mm, *end_mm) = (start, end),
                Err(error) => {
                    failed.insert(index, error);
                }
            }
        }
    }
    take_out(segments, failed, &mut removed);
    let arc_endpoints = arc_endpoint_buckets(segments);
    let mut failed = BTreeMap::new();
    for (index, segment) in segments.iter_mut().enumerate() {
        let ProfileSegment::Line { start_mm, end_mm } = &mut segment.segment else {
            continue;
        };
        let snapped = unambiguous_arc_endpoint(*start_mm, &arc_endpoints).and_then(|start| {
            let end = unambiguous_arc_endpoint(*end_mm, &arc_endpoints)?;
            ensure_distinct(start, end)?;
            Ok((start, end))
        });
        match snapped {
            Ok((start, end)) => (*start_mm, *end_mm) = (start, end),
            Err(error) => {
                failed.insert(index, error);
            }
        }
    }
    take_out(segments, failed, &mut removed);
    removed
}

/// Moves the `segments` at the indices of `failed` to `removed`, each with its error.
fn take_out(
    segments: &mut Vec<LooseSegment>,
    mut failed: BTreeMap<usize, DxfImportError>,
    removed: &mut Vec<(LooseSegment, DxfImportError)>,
) {
    if failed.is_empty() {
        return;
    }
    let mut kept = Vec::with_capacity(segments.len() - failed.len());
    for (index, segment) in std::mem::take(segments).into_iter().enumerate() {
        match failed.remove(&index) {
            Some(error) => removed.push((segment, error)),
            None => kept.push(segment),
        }
    }
    *segments = kept;
}

/// The point an arc end shares with at most one other arc end near it.
fn shared_arc_endpoint(
    point: [f64; 2],
    arc_endpoints: &BTreeMap<[i64; 2], Vec<[f64; 2]>>,
) -> Result<[f64; 2], DxfImportError> {
    let near = nearby_arc_endpoints(point, arc_endpoints);
    if near.len() > 2 {
        return Err(DxfImportError::AmbiguousGeometry);
    }
    Ok(near.into_values().next().unwrap_or(point))
}

fn nearby_arc_endpoints(
    point: [f64; 2],
    arc_endpoints: &BTreeMap<[i64; 2], Vec<[f64; 2]>>,
) -> BTreeMap<[u64; 2], [f64; 2]> {
    let bucket = connectivity_bucket(point);
    let mut candidates = BTreeMap::new();
    for x_offset in -1..=1 {
        for y_offset in -1..=1 {
            let nearby = [bucket[0] + x_offset, bucket[1] + y_offset];
            for candidate in arc_endpoints.get(&nearby).into_iter().flatten() {
                if (point[0] - candidate[0]).hypot(point[1] - candidate[1])
                    <= DXF_GEOMETRY_EPSILON_MM
                {
                    candidates.insert(point_key(*candidate), *candidate);
                }
            }
        }
    }
    candidates
}

fn unambiguous_arc_endpoint(
    point: [f64; 2],
    arc_endpoints: &BTreeMap<[i64; 2], Vec<[f64; 2]>>,
) -> Result<[f64; 2], DxfImportError> {
    let candidates = nearby_arc_endpoints(point, arc_endpoints);
    match candidates.len() {
        0 => Ok(point),
        1 => Ok(*candidates.values().next().expect("one endpoint candidate")),
        _ => Err(DxfImportError::AmbiguousGeometry),
    }
}

fn connectivity_bucket(point: [f64; 2]) -> [i64; 2] {
    [
        (point[0] / DXF_GEOMETRY_EPSILON_MM).floor() as i64,
        (point[1] / DXF_GEOMETRY_EPSILON_MM).floor() as i64,
    ]
}

/// Where a drawing or block reports what it leaves out of its profiles: the handles of its
/// entities by ordinal, and its report. Without one, the first ambiguity refuses the whole
/// reading, as within one hatch.
type LeaveOut<'a> = Option<(&'a BTreeMap<usize, String>, &'a mut DiagnosticCounts)>;

/// Leaves out what the entities of `ordinals` drew, counted by handle as `detail` of
/// `error`; refuses with `error` when there is nowhere to report it.
fn leave_out(
    report: &mut LeaveOut<'_>,
    ordinals: impl IntoIterator<Item = usize>,
    detail: &str,
    error: DxfImportError,
) -> Result<(), DxfImportError> {
    let mut ordinals = ordinals.into_iter().peekable();
    if ordinals.peek().is_none() {
        return Ok(());
    }
    let Some((handles, diagnostics)) = report else {
        return Err(error);
    };
    let what = format!("{} ({detail})", error.reason());
    for ordinal in ordinals {
        diagnostics.skip(
            "dxf.profile-ambiguous",
            what.clone(),
            handles
                .get(&ordinal)
                .map(|handle| ("handles", handle.as_str())),
            Some(error.clone()),
        )?;
    }
    Ok(())
}

/// The profiles `loose` chains into, layer by layer. With a `report`, a line it cannot
/// place unambiguously is left out and counted: a repeated line alone, and every line of
/// a run that branches or does not chain; without one, it refuses them all.
fn assemble_loose_segments(
    loose: Vec<LooseSegment>,
    max_segments: usize,
    mut report: LeaveOut<'_>,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let mut by_layer: BTreeMap<String, Vec<LooseSegment>> = BTreeMap::new();
    for segment in loose {
        by_layer
            .entry(segment.layer.clone())
            .or_default()
            .push(segment);
    }
    let mut profiles = Vec::new();
    for (layer, mut segments) in by_layer {
        for (segment, error) in snap_lines_to_unambiguous_arc_endpoints(&mut segments) {
            leave_out(
                &mut report,
                [segment.ordinal],
                "an end near arc ends",
                error,
            )?;
        }
        let mut duplicate_lines = BTreeSet::new();
        let mut repeated = Vec::new();
        segments.retain(|segment| {
            if let ProfileSegment::Line { .. } = segment.segment {
                let start = point_key(segment.segment.start_mm());
                let end = point_key(segment.segment.end_mm());
                let key = if start <= end {
                    (start, end)
                } else {
                    (end, start)
                };
                if !duplicate_lines.insert(key) {
                    repeated.push(segment.ordinal);
                    return false;
                }
            }
            true
        });
        leave_out(
            &mut report,
            repeated,
            "a repeated line",
            DxfImportError::AmbiguousGeometry,
        )?;
        let mut endpoints: BTreeMap<[u64; 2], Vec<usize>> = BTreeMap::new();
        for (index, segment) in segments.iter().enumerate() {
            endpoints
                .entry(point_key(segment.segment.start_mm()))
                .or_default()
                .push(index);
            endpoints
                .entry(point_key(segment.segment.end_mm()))
                .or_default()
                .push(index);
        }
        let mut unused = (0..segments.len()).collect::<BTreeSet<_>>();
        // Seeds in ordinal order, and open ends looked up among the
        // component's own points: scanning every segment per profile made a
        // file of many separate segments quadratic.
        let mut seeds = (0..segments.len()).collect::<Vec<_>>();
        seeds.sort_by_key(|index| segments[*index].ordinal);
        for seed in seeds {
            if !unused.contains(&seed) {
                continue;
            }
            let mut component = BTreeSet::new();
            let mut queue = VecDeque::from([seed]);
            while let Some(index) = queue.pop_front() {
                if !component.insert(index) {
                    continue;
                }
                for point in [
                    segments[index].segment.start_mm(),
                    segments[index].segment.end_mm(),
                ] {
                    if let Some(incident) = endpoints.get(&point_key(point)) {
                        queue.extend(incident.iter().copied());
                    }
                }
            }
            for index in &component {
                unused.remove(index);
            }
            let ordinal = component
                .iter()
                .map(|index| segments[*index].ordinal)
                .min()
                .expect("non-empty component");
            let (ordered, closed) =
                match chain_component(&segments, &endpoints, &component, seed, max_segments) {
                    Ok(chained) => chained,
                    Err((error, detail)) => {
                        leave_out(
                            &mut report,
                            component.iter().map(|index| segments[*index].ordinal),
                            detail,
                            error,
                        )?;
                        continue;
                    }
                };
            profiles.push(OrderedProfile {
                layer: layer.clone(),
                ordinal,
                style: EntityStyle::default(),
                segments: ordered,
                closed,
            });
        }
    }
    Ok(profiles)
}

/// The segments of `component`, a connected run of `segments` that `endpoints` indexes by
/// end, in order from its smaller open end (or from the start of `seed` when it closes),
/// and whether it closes; or why they do not chain into one profile.
fn chain_component(
    segments: &[LooseSegment],
    endpoints: &BTreeMap<[u64; 2], Vec<usize>>,
    component: &BTreeSet<usize>,
    seed: usize,
    max_segments: usize,
) -> Result<(Vec<ProfileSegment>, bool), (DxfImportError, &'static str)> {
    let ambiguous = |detail| (DxfImportError::AmbiguousGeometry, detail);
    let component_points = component
        .iter()
        .flat_map(|index| {
            [
                point_key(segments[*index].segment.start_mm()),
                point_key(segments[*index].segment.end_mm()),
            ]
        })
        .collect::<BTreeSet<_>>();
    let degree = |point: &[u64; 2]| {
        endpoints[point]
            .iter()
            .filter(|index| component.contains(index))
            .count()
    };
    if component_points.iter().any(|point| degree(point) > 2) {
        return Err(ambiguous("lines that branch"));
    }
    if component.len() > max_segments {
        return Err((
            DxfImportError::TooManySegments,
            "a run of too many segments",
        ));
    }
    let degree_one = component_points
        .into_iter()
        .filter(|point| degree(point) == 1)
        .collect::<Vec<_>>();
    let closed = match degree_one.len() {
        0 => true,
        2 => false,
        _ => return Err(ambiguous("lines that do not chain")),
    };
    let start_point = if closed {
        point_key(segments[seed].segment.start_mm())
    } else {
        *degree_one.iter().min().expect("two open endpoints")
    };
    let mut unused = component.clone();
    let mut current = start_point;
    let mut ordered = Vec::with_capacity(component.len());
    while !unused.is_empty() {
        let Some(index) = endpoints
            .get(&current)
            .into_iter()
            .flatten()
            .filter(|index| unused.contains(index))
            .copied()
            .min_by_key(|index| segments[*index].ordinal)
        else {
            return Err(ambiguous("lines that do not chain"));
        };
        let segment = &segments[index].segment;
        let oriented = if point_key(segment.start_mm()) == current {
            segment.clone()
        } else {
            reverse_segment(segment)
        };
        current = point_key(oriented.end_mm());
        ordered.push(oriented);
        unused.remove(&index);
    }
    if (closed && current != start_point) || (!closed && !degree_one.contains(&current)) {
        return Err(ambiguous("lines that do not chain"));
    }
    Ok((ordered, closed))
}

/// `profiles` without each that repeats a segment of its layer, of an earlier profile or
/// of its own, left out and counted in `report` (refused without one).
fn without_duplicate_segments(
    profiles: Vec<OrderedProfile>,
    mut report: LeaveOut<'_>,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let mut seen = BTreeSet::new();
    let mut kept = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let mut own = BTreeSet::new();
        let repeats = profile.segments.iter().any(|segment| {
            let key = (profile.layer.clone(), undirected_segment_key(segment));
            seen.contains(&key) || !own.insert(key)
        });
        if repeats {
            leave_out(
                &mut report,
                [profile.ordinal],
                "a segment another profile of its layer has",
                DxfImportError::AmbiguousGeometry,
            )?;
            continue;
        }
        seen.extend(own);
        kept.push(profile);
    }
    Ok(kept)
}

fn undirected_segment_key(segment: &ProfileSegment) -> UndirectedSegmentKey {
    let start = duplicate_point_key(segment.start_mm());
    let end = duplicate_point_key(segment.end_mm());
    match segment {
        ProfileSegment::Line { .. } => {
            if start <= end {
                UndirectedSegmentKey::Line(start, end)
            } else {
                UndirectedSegmentKey::Line(end, start)
            }
        }
        ProfileSegment::CircularArc {
            center_mm,
            clockwise,
            ..
        } => {
            if start <= end {
                UndirectedSegmentKey::CircularArc {
                    start,
                    end,
                    center: duplicate_point_key(*center_mm),
                    clockwise: *clockwise,
                }
            } else {
                UndirectedSegmentKey::CircularArc {
                    start: end,
                    end: start,
                    center: duplicate_point_key(*center_mm),
                    clockwise: !clockwise,
                }
            }
        }
        ProfileSegment::CubicBezier {
            control_1_mm,
            control_2_mm,
            ..
        } => {
            if start <= end {
                UndirectedSegmentKey::CubicBezier {
                    start,
                    control_1: duplicate_point_key(*control_1_mm),
                    control_2: duplicate_point_key(*control_2_mm),
                    end,
                }
            } else {
                UndirectedSegmentKey::CubicBezier {
                    start: end,
                    control_1: duplicate_point_key(*control_2_mm),
                    control_2: duplicate_point_key(*control_1_mm),
                    end: start,
                }
            }
        }
        ProfileSegment::Spline { points_mm } => {
            let mut points = points_mm
                .iter()
                .map(|point| duplicate_point_key(*point))
                .collect::<Vec<_>>();
            if start > end {
                points.reverse();
            }
            UndirectedSegmentKey::Spline(points)
        }
    }
}

fn point_key(point: [f64; 2]) -> [u64; 2] {
    [point[0].to_bits(), point[1].to_bits()]
}

fn duplicate_point_key(point: [f64; 2]) -> [i64; 2] {
    const DUPLICATE_GRID_MM: f64 = ACCUMULATED_ROUNDING;
    [
        (point[0] / DUPLICATE_GRID_MM).round() as i64,
        (point[1] / DUPLICATE_GRID_MM).round() as i64,
    ]
}

fn reverse_segment(segment: &ProfileSegment) -> ProfileSegment {
    match segment {
        ProfileSegment::Line { start_mm, end_mm } => ProfileSegment::Line {
            start_mm: *end_mm,
            end_mm: *start_mm,
        },
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => ProfileSegment::CircularArc {
            start_mm: *end_mm,
            end_mm: *start_mm,
            center_mm: *center_mm,
            clockwise: !clockwise,
        },
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => ProfileSegment::CubicBezier {
            start_mm: *end_mm,
            control_1_mm: *control_2_mm,
            control_2_mm: *control_1_mm,
            end_mm: *start_mm,
        },
        ProfileSegment::Spline { points_mm } => ProfileSegment::Spline {
            points_mm: points_mm.iter().rev().copied().collect(),
        },
    }
}

const fn unit_name(unit: ImportLengthUnit) -> &'static str {
    match unit {
        ImportLengthUnit::Millimetre => "millimetre",
        ImportLengthUnit::Centimetre => "centimetre",
        ImportLengthUnit::Metre => "metre",
        ImportLengthUnit::Inch => "inch",
        ImportLengthUnit::Foot => "foot",
    }
}
