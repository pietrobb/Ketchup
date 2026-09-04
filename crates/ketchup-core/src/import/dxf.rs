use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, FeatureKind, OccurrenceId,
    ProfileSegment, ProposalBudget, Snapshot, Transform,
};

use super::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportLengthUnit, ImportOutputRef,
    ImportReceipt, ImportUnitAuthority, ImportUnitDecision, MAX_IMPORT_DIAGNOSTICS,
};

pub const DXF_PARSER_ID: &str = "ketchup-dxf";
pub const DXF_PARSER_VERSION: &str = "27";
pub const MAX_DXF_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DXF_LINE_BYTES: usize = 512;
const MAX_DXF_PAIRS: usize = 100_000;
const MAX_DXF_ENTITIES: usize = 10_000;
const MAX_DXF_PROFILES: usize = (ProposalBudget::HOST_MAX.max_commands - 1) / 3;
const MAX_DXF_SEGMENTS_PER_PROFILE: usize = 1_024;
const MAX_DXF_BLOCK_DEPTH: usize = 32;
const MAX_DXF_ABS_MM: f64 = 1_000_000.0;
const DXF_GEOMETRY_EPSILON_MM: f64 = 1.0e-9;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DxfImportError {
    Empty,
    SourceTooLarge,
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
    NoSupportedGeometry,
    ReportTooLarge,
}

impl fmt::Display for DxfImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "DXF source is empty",
            Self::SourceTooLarge => "DXF source exceeds the bounded 8 MiB envelope",
            Self::NonAscii => "DXF must use the bounded ASCII encoding; binary DXF is unsupported",
            Self::LineTooLong => "DXF contains a line longer than 512 bytes",
            Self::TooManyPairs => "DXF exceeds the 100,000 group-code pair limit",
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
                "DXF contains an invalid, duplicate, cyclic, empty, or undefined block"
            }
            Self::UnsupportedInsertTransform => {
                "DXF INSERT uses unsupported zero, non-uniform curved, or Z scaling"
            }
            Self::TooManyEntities => "DXF exceeds the 10,000 entity limit",
            Self::TooManyProfiles => "DXF exceeds the bounded 170 canonical profile limit",
            Self::TooManySegments => "one DXF chain exceeds the 1,024 segment limit",
            Self::AmbiguousGeometry => {
                "DXF line/arc connectivity is branched, duplicated, or otherwise ambiguous"
            }
            Self::NoSupportedGeometry => {
                "DXF contains no supported LINE, ARC, CIRCLE, circular ELLIPSE, linear SPLINE, HATCH/MPOLYGON/MESH boundary, SOLID, TRACE, 3DFACE, straight LEADER, POLYLINE/polyface/polygon-mesh, or LWPOLYLINE geometry"
            }
            Self::ReportTooLarge => "DXF unsupported/loss report exceeds the 1,024 entry limit",
        })
    }
}

impl std::error::Error for DxfImportError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DxfImportPlanError {
    Parse(DxfImportError),
    InvalidSourceIdentity,
    IdSpaceExhausted,
}

impl fmt::Display for DxfImportPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(formatter),
            Self::InvalidSourceIdentity => {
                formatter.write_str("DXF source name or provenance is invalid")
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

#[derive(Default)]
struct DiagnosticCounts {
    entries: BTreeMap<(ImportDiagnosticSeverity, String, Option<String>), u32>,
}

impl DiagnosticCounts {
    fn add(
        &mut self,
        severity: ImportDiagnosticSeverity,
        code: &str,
        subject: Option<String>,
        count: u32,
    ) -> Result<(), DxfImportError> {
        let key = (severity, code.to_owned(), subject);
        let entry = self.entries.entry(key).or_default();
        *entry = entry
            .checked_add(count)
            .ok_or(DxfImportError::ReportTooLarge)?;
        if self.entries.len() > MAX_IMPORT_DIAGNOSTICS {
            return Err(DxfImportError::ReportTooLarge);
        }
        Ok(())
    }

    fn finish(self) -> Result<Vec<ImportDiagnostic>, DxfImportError> {
        self.entries
            .into_iter()
            .map(|((severity, code, subject), count)| {
                ImportDiagnostic::new(severity, code, subject, count)
                    .map_err(|_| DxfImportError::ReportTooLarge)
            })
            .collect()
    }
}

#[derive(Clone)]
struct LooseSegment {
    layer: String,
    ordinal: usize,
    segment: ProfileSegment,
}

#[derive(Clone)]
struct OrderedProfile {
    layer: String,
    ordinal: usize,
    segments: Vec<ProfileSegment>,
    closed: bool,
}

#[derive(Clone)]
struct BlockDefinition {
    base_mm: [f64; 2],
    profiles: Vec<OrderedProfile>,
}

#[derive(Clone)]
struct RawBlockDefinition<'a> {
    base_mm: [f64; 2],
    entities: Vec<DxfPair<'a>>,
}

struct BlockResolution<'a> {
    blocks: &'a mut BTreeMap<String, BlockDefinition>,
    resolving: &'a mut BTreeSet<String>,
    entity_count: &'a mut usize,
    diagnostics: &'a mut DiagnosticCounts,
}

#[derive(Default)]
struct ParsedEntities {
    loose: Vec<LooseSegment>,
    explicit_profiles: Vec<OrderedProfile>,
    layers: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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
}

/// Inspect a bounded ASCII DXF without mutating a document.
pub fn inspect_dxf(source: &[u8], options: DxfImportOptions) -> Result<ParsedDxf, DxfImportError> {
    let pairs = parse_pairs(source)?;
    let (sections, mut diagnostics) = parse_sections(&pairs)?;
    let header = sections.get("HEADER").map(Vec::as_slice).unwrap_or(&[]);
    let (units, insbase_mm) = parse_header(header, options, &mut diagnostics)?;
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
    let mut profiles = assemble_loose_segments(parsed_entities.loose)?;
    profiles.append(&mut parsed_entities.explicit_profiles);
    let layers = parsed_entities.layers;
    reject_duplicate_segments(&profiles)?;
    for left in 0..profiles.len() {
        if profiles[left + 1..]
            .iter()
            .any(|right| profiles_are_duplicate(&profiles[left], right))
        {
            return Err(DxfImportError::AmbiguousGeometry);
        }
    }
    profiles.sort_by(|left, right| {
        (left.layer.as_str(), left.ordinal).cmp(&(right.layer.as_str(), right.ordinal))
    });
    if profiles.is_empty() {
        return Err(DxfImportError::NoSupportedGeometry);
    }
    if profiles.len() > MAX_DXF_PROFILES {
        return Err(DxfImportError::TooManyProfiles);
    }
    for layer in &layers {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.layer",
            Some(layer.clone()),
            1,
        )?;
    }
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        "dxf.wcs-origin-preserved",
        None,
        1,
    )?;
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        match units.authority() {
            ImportUnitAuthority::FileDeclared => "dxf.units-file-declared",
            ImportUnitAuthority::UserDeclared => "dxf.units-user-declared",
        },
        Some(unit_name(units.source_unit()).to_owned()),
        1,
    )?;
    let diagnostics = diagnostics.finish()?;
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
        .map_err(|_| DxfImportPlanError::IdSpaceExhausted)?;
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
            kind: FeatureKind::SegmentProfile {
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
            tag: None,
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
    .map_err(|_| DxfImportPlanError::InvalidSourceIdentity)?;
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

fn parse_pairs(source: &[u8]) -> Result<Vec<DxfPair<'_>>, DxfImportError> {
    if source.is_empty() {
        return Err(DxfImportError::Empty);
    }
    if source.len() as u64 > MAX_DXF_SOURCE_BYTES {
        return Err(DxfImportError::SourceTooLarge);
    }
    if !source.is_ascii() {
        return Err(DxfImportError::NonAscii);
    }
    let text = std::str::from_utf8(source).map_err(|_| DxfImportError::NonAscii)?;
    let lines = text.lines().collect::<Vec<_>>();
    if lines.iter().any(|line| line.len() > MAX_DXF_LINE_BYTES) {
        return Err(DxfImportError::LineTooLong);
    }
    if lines.len() % 2 != 0 {
        return Err(DxfImportError::MalformedPairs);
    }
    if lines.len() / 2 > MAX_DXF_PAIRS {
        return Err(DxfImportError::TooManyPairs);
    }
    lines
        .chunks_exact(2)
        .map(|pair| {
            let code = pair[0]
                .trim()
                .parse::<i16>()
                .map_err(|_| DxfImportError::MalformedPairs)?;
            Ok(DxfPair {
                code,
                value: pair[1].trim(),
            })
        })
        .collect()
}

fn parse_sections<'a>(
    pairs: &[DxfPair<'a>],
) -> Result<(BTreeMap<String, Vec<DxfPair<'a>>>, DiagnosticCounts), DxfImportError> {
    let mut sections = BTreeMap::new();
    let mut diagnostics = DiagnosticCounts::default();
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
    let declared = match variables.get("$INSUNITS") {
        Some(values) => {
            let value = exactly_one(values, 70)?
                .parse::<i16>()
                .map_err(|_| DxfImportError::InvalidHeader)?;
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
        if name != "$INSUNITS" && name != "$INSBASE" {
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

fn parse_blocks<'a>(
    pairs: &[DxfPair<'a>],
    units: ImportUnitDecision,
    entity_count: &mut usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<BTreeMap<String, BlockDefinition>, DxfImportError> {
    let mut raw_blocks = BTreeMap::new();
    let mut index = 0;
    while index < pairs.len() {
        if pairs[index].code != 0 || !pairs[index].value.eq_ignore_ascii_case("BLOCK") {
            return Err(DxfImportError::InvalidBlock);
        }
        bump_entity_count(entity_count)?;
        let header_start = index + 1;
        index = header_start;
        while index < pairs.len() && pairs[index].code != 0 {
            index += 1;
        }
        let header = &pairs[header_start..index];
        ensure_planar(header)?;
        report_ignored_groups(
            "BLOCK",
            header,
            &[5, 8, 2, 3, 10, 20, 30, 70, 100],
            diagnostics,
        )?;
        parse_layer(header)?;
        let flags = parse_optional_i64(header, 70)?.unwrap_or(0);
        if flags & !3 != 0 {
            return Err(DxfImportError::InvalidBlock);
        }
        let name = parse_required_text(header, 2)?;
        if parse_optional_text(header, 3)?.is_some_and(|alternate| alternate != name) {
            return Err(DxfImportError::InvalidBlock);
        }
        let key = block_name_key(name)?;
        let base_mm = parse_xy(header, 10, 20, units.millimetres_per_unit())?;

        let entities_start = index;
        while index < pairs.len() {
            if pairs[index].code != 0 || pairs[index].value.is_empty() {
                return Err(DxfImportError::InvalidBlock);
            }
            if pairs[index].value.eq_ignore_ascii_case("BLOCK") {
                return Err(DxfImportError::InvalidBlock);
            }
            if pairs[index].value.eq_ignore_ascii_case("ENDBLK") {
                break;
            }
            index += 1;
            while index < pairs.len() && pairs[index].code != 0 {
                index += 1;
            }
        }
        if index == pairs.len() {
            return Err(DxfImportError::InvalidBlock);
        }
        let entities = pairs[entities_start..index].to_vec();
        let end_start = index + 1;
        index = end_start;
        while index < pairs.len() && pairs[index].code != 0 {
            index += 1;
        }
        let end_record = &pairs[end_start..index];
        bump_entity_count(entity_count)?;
        parse_optional_layer(end_record)?;
        report_ignored_groups("ENDBLK", end_record, &[5, 8, 100], diagnostics)?;

        if raw_blocks
            .insert(key, RawBlockDefinition { base_mm, entities })
            .is_some()
        {
            return Err(DxfImportError::InvalidBlock);
        }
    }

    let keys = raw_blocks.keys().cloned().collect::<Vec<_>>();
    let mut blocks = BTreeMap::new();
    let mut resolving = BTreeSet::new();
    {
        let mut resolution = BlockResolution {
            blocks: &mut blocks,
            resolving: &mut resolving,
            entity_count,
            diagnostics,
        };
        for key in keys {
            resolve_block(&key, &raw_blocks, 0, units, &mut resolution)?;
        }
    }
    Ok(blocks)
}

fn resolve_block<'a>(
    key: &str,
    raw_blocks: &BTreeMap<String, RawBlockDefinition<'a>>,
    depth: usize,
    units: ImportUnitDecision,
    resolution: &mut BlockResolution<'_>,
) -> Result<(), DxfImportError> {
    if resolution.blocks.contains_key(key) {
        return Ok(());
    }
    if depth >= MAX_DXF_BLOCK_DEPTH || !resolution.resolving.insert(key.to_owned()) {
        return Err(DxfImportError::InvalidBlock);
    }
    let raw = raw_blocks
        .get(key)
        .cloned()
        .ok_or(DxfImportError::InvalidBlock)?;
    for dependency in block_dependencies(&raw.entities)? {
        resolve_block(&dependency, raw_blocks, depth + 1, units, resolution)?;
    }

    let mut parsed_entities = ParsedEntities::default();
    parse_entities(
        &raw.entities,
        units,
        Some(resolution.blocks),
        resolution.entity_count,
        &mut parsed_entities,
        resolution.diagnostics,
    )?;
    let mut profiles = assemble_loose_segments(parsed_entities.loose)?;
    profiles.append(&mut parsed_entities.explicit_profiles);
    reject_duplicate_segments(&profiles)?;
    if profiles.len() > MAX_DXF_PROFILES {
        return Err(DxfImportError::TooManyProfiles);
    }
    resolution.blocks.insert(
        key.to_owned(),
        BlockDefinition {
            base_mm: raw.base_mm,
            profiles,
        },
    );
    resolution.resolving.remove(key);
    Ok(())
}

fn block_dependencies(pairs: &[DxfPair<'_>]) -> Result<Vec<String>, DxfImportError> {
    let mut dependencies = Vec::new();
    let mut index = 0;
    while index < pairs.len() {
        if pairs[index].code != 0 || pairs[index].value.is_empty() {
            return Err(DxfImportError::InvalidBlock);
        }
        let kind = pairs[index].value;
        let start = index + 1;
        index = start;
        while index < pairs.len() && pairs[index].code != 0 {
            index += 1;
        }
        if kind.eq_ignore_ascii_case("INSERT") || kind.eq_ignore_ascii_case("DIMENSION") {
            dependencies.push(block_name_key(parse_required_text(
                &pairs[start..index],
                2,
            )?)?);
        }
    }
    Ok(dependencies)
}

fn bump_entity_count(entity_count: &mut usize) -> Result<(), DxfImportError> {
    *entity_count = entity_count
        .checked_add(1)
        .ok_or(DxfImportError::TooManyEntities)?;
    if *entity_count > MAX_DXF_ENTITIES {
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
        bump_entity_count(entity_count)?;
        let kind = pairs[index].value.to_ascii_uppercase();
        let start = index + 1;
        index = start;
        while index < pairs.len() && pairs[index].code != 0 {
            index += 1;
        }
        let record = &pairs[start..index];
        match kind.as_str() {
            "LINE" => {
                let segment = parse_line(record, units, diagnostics)?;
                output.layers.insert(segment.layer.clone());
                output.loose.push(LooseSegment {
                    layer: segment.layer,
                    ordinal,
                    segment: segment.segment,
                });
            }
            "ARC" => {
                let segment = parse_arc(record, units, diagnostics)?;
                output.layers.insert(segment.layer.clone());
                output.loose.push(LooseSegment {
                    layer: segment.layer,
                    ordinal,
                    segment: segment.segment,
                });
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
                parse_insert_attributes(
                    record,
                    pairs,
                    &mut index,
                    &mut ordinal,
                    entity_count,
                    diagnostics,
                )?;
                let block_definitions = blocks.ok_or(DxfImportError::InvalidBlock)?;
                for profile in parse_insert(
                    record,
                    units,
                    profile_ordinal,
                    block_definitions,
                    entity_count,
                    diagnostics,
                )? {
                    output.layers.insert(profile.layer.clone());
                    output.explicit_profiles.push(profile);
                }
            }
            "DIMENSION" => {
                let block_definitions = blocks.ok_or(DxfImportError::InvalidBlock)?;
                for profile in
                    parse_dimension(record, units, ordinal, block_definitions, diagnostics)?
                {
                    output.layers.insert(profile.layer.clone());
                    output.explicit_profiles.push(profile);
                }
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
                Some(kind),
                1,
            )?,
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
    ensure_planar(record)?;
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
    Ok(OrderedProfile {
        layer,
        ordinal,
        segments: vec![
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
        ],
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
    let sweep = end_parameter - start_parameter;
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
    let degree = usize::try_from(degree).map_err(|_| DxfImportError::MalformedPairs)?;
    let declared_knots =
        usize::try_from(declared_knots).map_err(|_| DxfImportError::MalformedPairs)?;
    let declared_controls =
        usize::try_from(declared_controls).map_err(|_| DxfImportError::MalformedPairs)?;
    let declared_fit_points =
        usize::try_from(declared_fit_points).map_err(|_| DxfImportError::MalformedPairs)?;
    if declared_controls > MAX_DXF_SEGMENTS_PER_PROFILE + 1 {
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
            u32::try_from(profiles.len()).map_err(|_| DxfImportError::TooManyProfiles)?,
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
    let path_count = usize::try_from(path_count).map_err(|_| DxfImportError::TooManyProfiles)?;
    if path_count > MAX_DXF_PROFILES {
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
        let path_type = path_type
            .value
            .parse::<i64>()
            .map_err(|_| DxfImportError::MalformedPairs)?;
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
        let source_count = path_record[source_count_index]
            .value
            .parse::<i64>()
            .map_err(|_| DxfImportError::MalformedPairs)?;
        if source_count < 0 {
            return Err(DxfImportError::MalformedPairs);
        }
        let source_count =
            usize::try_from(source_count).map_err(|_| DxfImportError::MalformedPairs)?;
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
    let source_count = path_record[source_count_index]
        .value
        .parse::<usize>()
        .map_err(|_| DxfImportError::MalformedPairs)?;
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
            u32::try_from(path_count).map_err(|_| DxfImportError::TooManyProfiles)?,
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
        return parse_hatch_edge_path(path, layer, ordinal, units);
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
    let declared_count =
        usize::try_from(declared_count).map_err(|_| DxfImportError::TooManySegments)?;
    if declared_count > MAX_DXF_SEGMENTS_PER_PROFILE {
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
) -> Result<Option<OrderedProfile>, DxfImportError> {
    let declared_count = parse_required_i64(path, 93)?;
    if declared_count < 1 {
        return Err(DxfImportError::DegenerateGeometry);
    }
    let declared_count =
        usize::try_from(declared_count).map_err(|_| DxfImportError::TooManySegments)?;
    if declared_count > MAX_DXF_SEGMENTS_PER_PROFILE {
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
                let Some(segments) = parse_hatch_linear_spline_edge(edge, scale)? else {
                    return Ok(None);
                };
                segments
            }
            _ => return Err(DxfImportError::MalformedPairs),
        };

        for segment in segments {
            if loose.len() >= MAX_DXF_SEGMENTS_PER_PROFILE {
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
                ProfileSegment::CubicBezier { .. } => {
                    return Err(DxfImportError::AmbiguousGeometry);
                }
            };
            if !unique.insert(key) {
                return Err(DxfImportError::AmbiguousGeometry);
            }
            loose.push(LooseSegment {
                layer: layer.to_owned(),
                ordinal: loose.len(),
                segment,
            });
        }
    }

    let mut profiles = assemble_loose_segments(loose)?;
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
    let degree = usize::try_from(degree).map_err(|_| DxfImportError::MalformedPairs)?;
    let declared_knots =
        usize::try_from(declared_knots).map_err(|_| DxfImportError::MalformedPairs)?;
    let declared_controls =
        usize::try_from(declared_controls).map_err(|_| DxfImportError::MalformedPairs)?;
    let declared_fit_points =
        usize::try_from(declared_fit_points).map_err(|_| DxfImportError::MalformedPairs)?;
    if declared_controls > MAX_DXF_SEGMENTS_PER_PROFILE + 1 {
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
        let value = record[position]
            .value
            .parse::<i64>()
            .map_err(|_| DxfImportError::MalformedPairs)?;
        if value < 0 {
            return Err(DxfImportError::MalformedPairs);
        }
        usize::try_from(value).map_err(|_| DxfImportError::MalformedPairs)
    };
    let declared_vertices = parse_marker_count(vertices_marker)?;
    if !(3..=MAX_DXF_PAIRS / 3).contains(&declared_vertices) {
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
        .map(|pair| {
            pair.value
                .parse::<i64>()
                .map_err(|_| DxfImportError::MalformedPairs)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut faces = Vec::new();
    let mut cursor = 0;
    while cursor < face_values.len() {
        let declared_face_vertices = face_values[cursor];
        cursor += 1;
        if declared_face_vertices < 3 {
            return Err(DxfImportError::DegenerateGeometry);
        }
        let declared_face_vertices =
            usize::try_from(declared_face_vertices).map_err(|_| DxfImportError::MalformedPairs)?;
        if declared_face_vertices > MAX_DXF_SEGMENTS_PER_PROFILE
            || cursor + declared_face_vertices > face_values.len()
        {
            return Err(if declared_face_vertices > MAX_DXF_SEGMENTS_PER_PROFILE {
                DxfImportError::TooManySegments
            } else {
                DxfImportError::MalformedPairs
            });
        }
        let mut face = Vec::with_capacity(declared_face_vertices);
        let mut unique_indices = BTreeSet::new();
        for raw_index in &face_values[cursor..cursor + declared_face_vertices] {
            if *raw_index < 0 {
                return Err(DxfImportError::MalformedPairs);
            }
            let index = usize::try_from(*raw_index).map_err(|_| DxfImportError::MalformedPairs)?;
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
            segment: ProfileSegment::Line {
                start_mm: vertices[start],
                end_mm: vertices[end],
            },
        })
        .collect();
    let mut profiles = assemble_loose_segments(loose)?;
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
        u32::try_from(faces.len()).map_err(|_| DxfImportError::TooManyProfiles)?,
    )?;
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        boundary_code,
        Some(layer),
        u32::try_from(profiles.len()).map_err(|_| DxfImportError::TooManyProfiles)?,
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
        .map(|(start, end)| cross(start, end))
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
    let declared_vertices =
        usize::try_from(declared_vertices).map_err(|_| DxfImportError::MalformedPairs)?;
    if declared_vertices > MAX_DXF_SEGMENTS_PER_PROFILE + 1 {
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
    ensure_planar(record)?;
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
    Ok(LooseSegment {
        layer,
        ordinal: 0,
        segment: ProfileSegment::CircularArc {
            start_mm: start,
            end_mm: end,
            center_mm: center,
            clockwise: false,
        },
    })
}

fn parse_dimension(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    blocks: &BTreeMap<String, BlockDefinition>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
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
    let block = blocks
        .get(&key)
        .filter(|block| !block.profiles.is_empty() && block.base_mm == [0.0, 0.0])
        .ok_or(DxfImportError::InvalidBlock)?;
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
    Ok(profiles)
}

fn parse_insert(
    record: &[DxfPair<'_>],
    units: ImportUnitDecision,
    ordinal: usize,
    blocks: &BTreeMap<String, BlockDefinition>,
    entity_count: &mut usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Vec<OrderedProfile>, DxfImportError> {
    let layer = parse_layer(record)?;
    ensure_planar(record)?;
    report_ignored_groups(
        "INSERT",
        record,
        &[
            5, 8, 2, 10, 20, 30, 41, 42, 43, 44, 45, 50, 66, 70, 71, 100, 210, 220, 230,
        ],
        diagnostics,
    )?;
    let scale_x = parse_optional_number(record, 41)?.unwrap_or(1.0);
    let scale_y = parse_optional_number(record, 42)?.unwrap_or(1.0);
    let scale_z = parse_optional_number(record, 43)?.unwrap_or(1.0);
    let rotation_degrees = parse_optional_number(record, 50)?.unwrap_or(0.0);
    let column_count = parse_optional_i64(record, 70)?.unwrap_or(1);
    let row_count = parse_optional_i64(record, 71)?.unwrap_or(1);
    if scale_x == 0.0 || scale_y == 0.0 || scale_z != 1.0 || column_count <= 0 || row_count <= 0 {
        return Err(DxfImportError::UnsupportedInsertTransform);
    }
    let column_count =
        usize::try_from(column_count).map_err(|_| DxfImportError::UnsupportedInsertTransform)?;
    let row_count =
        usize::try_from(row_count).map_err(|_| DxfImportError::UnsupportedInsertTransform)?;
    let placement_count = row_count
        .checked_mul(column_count)
        .ok_or(DxfImportError::TooManyEntities)?;
    if placement_count > MAX_DXF_ENTITIES {
        return Err(DxfImportError::TooManyEntities);
    }
    let unit_scale = units.millimetres_per_unit();
    let column_spacing_mm = scale_coordinate(
        parse_optional_number(record, 44)?.unwrap_or(0.0),
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
    let name = parse_required_text(record, 2)?;
    let block = blocks
        .get(&block_name_key(name)?)
        .filter(|block| !block.profiles.is_empty())
        .ok_or(DxfImportError::InvalidBlock)?;
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
    if expanded_profile_count > MAX_DXF_PROFILES {
        return Err(DxfImportError::TooManyProfiles);
    }
    *entity_count = entity_count
        .checked_add(placement_count - 1)
        .ok_or(DxfImportError::TooManyEntities)?;
    if *entity_count > MAX_DXF_ENTITIES {
        return Err(DxfImportError::TooManyEntities);
    }
    let insertion_mm = parse_xy(record, 10, 20, unit_scale)?;
    let (rotation_sin, rotation_cos) = deterministic_sin_cos_degrees(rotation_degrees);
    let mut profiles = Vec::with_capacity(expanded_profile_count);
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
        }
    }
    diagnostics.add(
        ImportDiagnosticSeverity::Info,
        "dxf.block-insert",
        Some(name.to_owned()),
        u32::try_from(placement_count).map_err(|_| DxfImportError::TooManyEntities)?,
    )?;
    Ok(profiles)
}

fn parse_insert_attributes(
    insert_record: &[DxfPair<'_>],
    pairs: &[DxfPair<'_>],
    index: &mut usize,
    ordinal: &mut usize,
    entity_count: &mut usize,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    match parse_optional_i64(insert_record, 66)? {
        None | Some(0) => return Ok(()),
        Some(1) => {}
        Some(_) => return Err(DxfImportError::MalformedPairs),
    }
    let block_name = parse_required_text(insert_record, 2)?;
    let mut attribute_count = 0_u32;
    loop {
        if *index >= pairs.len() || pairs[*index].code != 0 || pairs[*index].value.is_empty() {
            return Err(DxfImportError::MalformedPairs);
        }
        *ordinal += 1;
        bump_entity_count(entity_count)?;
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
    diagnostics.add(
        ImportDiagnosticSeverity::Warning,
        "dxf.insert-attributes-dropped",
        Some(block_name.to_owned()),
        attribute_count,
    )?;
    Ok(())
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

fn transform_segment(
    segment: &ProfileSegment,
    base_mm: [f64; 2],
    insertion_mm: [f64; 2],
    scale: [f64; 2],
    rotation_sin: f64,
    rotation_cos: f64,
) -> Result<ProfileSegment, DxfImportError> {
    let transform = |point: [f64; 2]| {
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
                cross(start_vector, end_vector) > 0.0
            } else {
                cross(start_vector, end_vector) < 0.0
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
        bump_entity_count(entity_count)?;
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
                if vertices.len() > MAX_DXF_SEGMENTS_PER_PROFILE {
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
    let m_count = usize::try_from(m_count).map_err(|_| DxfImportError::MalformedPairs)?;
    let n_count = usize::try_from(n_count).map_err(|_| DxfImportError::MalformedPairs)?;
    let vertex_count = m_count
        .checked_mul(n_count)
        .ok_or(DxfImportError::TooManyEntities)?;
    if vertex_count > MAX_DXF_ENTITIES {
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
    if max_boundary_segments > MAX_DXF_SEGMENTS_PER_PROFILE {
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
                let one_based =
                    usize::try_from(one_based).map_err(|_| DxfImportError::MalformedPairs)?;
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
    let declared_vertices =
        usize::try_from(declared_vertices).map_err(|_| DxfImportError::MalformedPairs)?;
    let declared_faces =
        usize::try_from(declared_faces).map_err(|_| DxfImportError::MalformedPairs)?;
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
    ensure_planar(record)?;
    report_ignored_groups(
        "LWPOLYLINE",
        record,
        &[5, 8, 10, 20, 30, 38, 39, 42, 70, 90, 100, 210, 220, 230],
        diagnostics,
    )?;
    let declared_count = parse_required_i64(record, 90)?;
    if declared_count < 2 || declared_count as usize > MAX_DXF_SEGMENTS_PER_PROFILE + 1 {
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
    build_polyline_profile(layer, ordinal, vertices, closed, units, diagnostics)
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
    let edge_count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    if edge_count > MAX_DXF_SEGMENTS_PER_PROFILE {
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

fn block_name_key(name: &str) -> Result<String, DxfImportError> {
    if name.is_empty()
        || name.len() > 255
        || name.chars().any(char::is_control)
        || name.contains(['/', '\\'])
    {
        return Err(DxfImportError::InvalidBlock);
    }
    Ok(name.to_ascii_uppercase())
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

fn report_ignored_groups(
    entity: &str,
    record: &[DxfPair<'_>],
    accepted: &[i16],
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    for pair in record {
        if !accepted.contains(&pair.code) {
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
    value
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| DxfImportError::MalformedPairs)
        })
        .transpose()
}

fn parse_number(value: &str) -> Result<f64, DxfImportError> {
    let value = value
        .parse::<f64>()
        .map_err(|_| DxfImportError::InvalidNumber)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(DxfImportError::InvalidNumber)
    }
}

fn scale_coordinate(value: f64, scale: f64) -> Result<f64, DxfImportError> {
    normalize_coordinate(value * scale)
}

fn deterministic_sin_cos_degrees(degrees: f64) -> (f64, f64) {
    let normalized = degrees.rem_euclid(360.0);
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
        3 => (-cosine, sine),
        _ => unreachable!("angle was reduced to one turn"),
    }
}

fn normalize_coordinate(value: f64) -> Result<f64, DxfImportError> {
    if !value.is_finite() || value.abs() > MAX_DXF_ABS_MM {
        return Err(DxfImportError::CoordinateOutOfRange);
    }
    Ok(if value == 0.0 { 0.0 } else { value })
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
        .map(|(start, end)| cross(start, end))
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
        cross(
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
        !(cross(to, direction) > tolerance && cross(direction, from) > tolerance)
    } else {
        cross(from, direction) >= -tolerance && cross(direction, to) >= -tolerance
    }
}

fn cross(left: [f64; 2], right: [f64; 2]) -> f64 {
    left[0] * right[1] - left[1] * right[0]
}

fn snap_lines_to_unambiguous_arc_endpoints(
    segments: &mut [LooseSegment],
) -> Result<(), DxfImportError> {
    let mut arc_endpoints: BTreeMap<[i64; 2], Vec<[f64; 2]>> = BTreeMap::new();
    for segment in segments.iter() {
        if matches!(segment.segment, ProfileSegment::CircularArc { .. }) {
            for point in [segment.segment.start_mm(), segment.segment.end_mm()] {
                arc_endpoints
                    .entry(connectivity_bucket(point))
                    .or_default()
                    .push(point);
            }
        }
    }
    for segment in segments.iter_mut() {
        let ProfileSegment::Line { start_mm, end_mm } = &mut segment.segment else {
            continue;
        };
        *start_mm = unambiguous_arc_endpoint(*start_mm, &arc_endpoints)?;
        *end_mm = unambiguous_arc_endpoint(*end_mm, &arc_endpoints)?;
        ensure_distinct(*start_mm, *end_mm)?;
    }
    Ok(())
}

fn unambiguous_arc_endpoint(
    point: [f64; 2],
    arc_endpoints: &BTreeMap<[i64; 2], Vec<[f64; 2]>>,
) -> Result<[f64; 2], DxfImportError> {
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

fn assemble_loose_segments(
    loose: Vec<LooseSegment>,
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
        snap_lines_to_unambiguous_arc_endpoints(&mut segments)?;
        let mut endpoints: BTreeMap<[u64; 2], Vec<usize>> = BTreeMap::new();
        let mut duplicate_lines = BTreeSet::new();
        for (index, segment) in segments.iter().enumerate() {
            let start = point_key(segment.segment.start_mm());
            let end = point_key(segment.segment.end_mm());
            endpoints.entry(start).or_default().push(index);
            endpoints.entry(end).or_default().push(index);
            if let ProfileSegment::Line { .. } = segment.segment {
                let key = if start <= end {
                    (start, end)
                } else {
                    (end, start)
                };
                if !duplicate_lines.insert(key) {
                    return Err(DxfImportError::AmbiguousGeometry);
                }
            }
        }
        if endpoints.values().any(|incident| incident.len() > 2) {
            return Err(DxfImportError::AmbiguousGeometry);
        }
        let mut unused = (0..segments.len()).collect::<BTreeSet<_>>();
        while let Some(seed) = unused
            .iter()
            .min_by_key(|index| segments[**index].ordinal)
            .copied()
        {
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
            let degree_one = endpoints
                .iter()
                .filter(|(_, incident)| {
                    incident
                        .iter()
                        .filter(|index| component.contains(index))
                        .count()
                        == 1
                })
                .map(|(point, _)| *point)
                .collect::<Vec<_>>();
            let closed = match degree_one.len() {
                0 => true,
                2 => false,
                _ => return Err(DxfImportError::AmbiguousGeometry),
            };
            let start_point = if closed {
                point_key(segments[seed].segment.start_mm())
            } else {
                *degree_one.iter().min().expect("two open endpoints")
            };
            let mut current = start_point;
            let mut ordered = Vec::with_capacity(component.len());
            while ordered.len() < component.len() {
                let mut candidates = endpoints
                    .get(&current)
                    .into_iter()
                    .flatten()
                    .filter(|index| component.contains(index) && unused.contains(index))
                    .copied()
                    .collect::<Vec<_>>();
                candidates.sort_by_key(|index| segments[*index].ordinal);
                let Some(index) = candidates.first().copied() else {
                    return Err(DxfImportError::AmbiguousGeometry);
                };
                let segment = &segments[index].segment;
                let oriented = if point_key(segment.start_mm()) == current {
                    segment.clone()
                } else if point_key(segment.end_mm()) == current {
                    reverse_segment(segment)
                } else {
                    return Err(DxfImportError::AmbiguousGeometry);
                };
                current = point_key(oriented.end_mm());
                ordered.push(oriented);
                unused.remove(&index);
            }
            if (closed && current != start_point)
                || (!closed && !degree_one.contains(&current))
                || ordered.len() > MAX_DXF_SEGMENTS_PER_PROFILE
            {
                return Err(if ordered.len() > MAX_DXF_SEGMENTS_PER_PROFILE {
                    DxfImportError::TooManySegments
                } else {
                    DxfImportError::AmbiguousGeometry
                });
            }
            profiles.push(OrderedProfile {
                layer: layer.clone(),
                ordinal: component
                    .iter()
                    .map(|index| segments[*index].ordinal)
                    .min()
                    .expect("non-empty component"),
                segments: ordered,
                closed,
            });
        }
    }
    Ok(profiles)
}

fn reject_duplicate_segments(profiles: &[OrderedProfile]) -> Result<(), DxfImportError> {
    let mut seen = BTreeSet::new();
    for profile in profiles {
        for segment in &profile.segments {
            if !seen.insert((profile.layer.clone(), undirected_segment_key(segment))) {
                return Err(DxfImportError::AmbiguousGeometry);
            }
        }
    }
    Ok(())
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
    }
}

fn profiles_are_duplicate(left: &OrderedProfile, right: &OrderedProfile) -> bool {
    if left.layer != right.layer
        || left.closed != right.closed
        || left.segments.len() != right.segments.len()
    {
        return false;
    }
    let reversed = right
        .segments
        .iter()
        .rev()
        .map(reverse_segment)
        .collect::<Vec<_>>();
    if left.closed {
        cyclic_segments_equal(&left.segments, &right.segments)
            || cyclic_segments_equal(&left.segments, &reversed)
    } else {
        left.segments == right.segments || left.segments == reversed
    }
}

fn cyclic_segments_equal(left: &[ProfileSegment], right: &[ProfileSegment]) -> bool {
    (0..right.len()).any(|offset| {
        left.iter()
            .enumerate()
            .all(|(index, segment)| segment == &right[(index + offset) % right.len()])
    })
}

fn point_key(point: [f64; 2]) -> [u64; 2] {
    [point[0].to_bits(), point[1].to_bits()]
}

fn duplicate_point_key(point: [f64; 2]) -> [i64; 2] {
    const DUPLICATE_GRID_MM: f64 = 1.0e-8;
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
