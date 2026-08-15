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
pub const DXF_PARSER_VERSION: &str = "1";
pub const MAX_DXF_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DXF_LINE_BYTES: usize = 512;
const MAX_DXF_PAIRS: usize = 100_000;
const MAX_DXF_ENTITIES: usize = 10_000;
const MAX_DXF_PROFILES: usize = (ProposalBudget::HOST_MAX.max_commands - 1) / 3;
const MAX_DXF_SEGMENTS_PER_PROFILE: usize = 1_024;
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
            Self::InvalidBulge => "DXF contains an invalid or full-circle LWPOLYLINE bulge",
            Self::TooManyEntities => "DXF exceeds the 10,000 entity limit",
            Self::TooManyProfiles => "DXF exceeds the bounded 170 canonical profile limit",
            Self::TooManySegments => "one DXF chain exceeds the 1,024 segment limit",
            Self::AmbiguousGeometry => {
                "DXF line/arc connectivity is branched, duplicated, or otherwise ambiguous"
            }
            Self::NoSupportedGeometry => {
                "DXF contains no supported LINE, ARC, or LWPOLYLINE geometry"
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum UndirectedSegmentKey {
    Line([i64; 2], [i64; 2]),
    CircularArc {
        start: [i64; 2],
        end: [i64; 2],
        center: [i64; 2],
        clockwise: bool,
    },
}

/// Inspect a bounded ASCII DXF without mutating a document.
pub fn inspect_dxf(source: &[u8], options: DxfImportOptions) -> Result<ParsedDxf, DxfImportError> {
    let pairs = parse_pairs(source)?;
    let (sections, mut diagnostics) = parse_sections(&pairs)?;
    let header = sections.get("HEADER").map(Vec::as_slice).unwrap_or(&[]);
    let (units, insbase_mm) = parse_header(header, options, &mut diagnostics)?;
    let entities = sections
        .get("ENTITIES")
        .ok_or(DxfImportError::MissingEntitiesSection)?;
    let mut loose = Vec::new();
    let mut explicit_profiles = Vec::new();
    let mut layers = BTreeSet::new();
    parse_entities(
        entities,
        units,
        &mut loose,
        &mut explicit_profiles,
        &mut layers,
        &mut diagnostics,
    )?;
    let mut profiles = assemble_loose_segments(loose)?;
    profiles.append(&mut explicit_profiles);
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
        if name != "HEADER" && name != "ENTITIES" {
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

fn parse_entities(
    pairs: &[DxfPair<'_>],
    units: ImportUnitDecision,
    loose: &mut Vec<LooseSegment>,
    explicit_profiles: &mut Vec<OrderedProfile>,
    layers: &mut BTreeSet<String>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    let mut index = 0;
    let mut ordinal = 0;
    while index < pairs.len() {
        if pairs[index].code != 0 || pairs[index].value.is_empty() {
            return Err(DxfImportError::MalformedSections);
        }
        ordinal += 1;
        if ordinal > MAX_DXF_ENTITIES {
            return Err(DxfImportError::TooManyEntities);
        }
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
                layers.insert(segment.layer.clone());
                loose.push(LooseSegment {
                    layer: segment.layer,
                    ordinal,
                    segment: segment.segment,
                });
            }
            "ARC" => {
                let segment = parse_arc(record, units, diagnostics)?;
                layers.insert(segment.layer.clone());
                loose.push(LooseSegment {
                    layer: segment.layer,
                    ordinal,
                    segment: segment.segment,
                });
            }
            "LWPOLYLINE" => {
                let profile = parse_lwpolyline(record, units, ordinal, diagnostics)?;
                layers.insert(profile.layer.clone());
                explicit_profiles.push(profile);
            }
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

fn parse_layer(record: &[DxfPair<'_>]) -> Result<String, DxfImportError> {
    let values = record
        .iter()
        .filter(|pair| pair.code == 8)
        .map(|pair| pair.value)
        .collect::<Vec<_>>();
    let layer = match values.as_slice() {
        [] => "0",
        [value] => *value,
        _ => return Err(DxfImportError::MalformedPairs),
    };
    if layer.is_empty()
        || layer.len() > 255
        || layer.chars().any(char::is_control)
        || layer.contains(['/', '\\'])
    {
        return Err(DxfImportError::MalformedPairs);
    }
    Ok(layer.to_owned())
}

fn ensure_planar(record: &[DxfPair<'_>]) -> Result<(), DxfImportError> {
    for pair in record {
        match pair.code {
            30 | 31 | 38 | 39 if parse_number(pair.value)? != 0.0 => {
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
    let mut values = record.iter().filter(|pair| pair.code == code);
    let value = values.next().ok_or(DxfImportError::MalformedPairs)?.value;
    if values.next().is_some() {
        return Err(DxfImportError::MalformedPairs);
    }
    parse_number(value)
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
