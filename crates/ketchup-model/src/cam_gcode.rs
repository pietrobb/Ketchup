//! The G-code of a CNC controller as data.
//!
//! A dialect lists the lines a program starts and ends with and one template per line
//! kind, in which `{name}` stands for a value of the program. One renderer writes every
//! dialect and one reader reads it back, so a controller is added by a library entry
//! (`library/cam-gcode-dialects.json`) or by the caller's own JSON, never by code.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::cam::{
    CAM_POSTPROCESSOR_SCHEMA_V1, CamPostprocessedMotion, CamPostprocessedMotionKind,
    CamPostprocessedProgram, CamPostprocessorError, malformed,
};

pub const CAM_GCODE_DIALECT_SCHEMA_V1: &str = "ketchup.cam-gcode-dialect.v1";

const LIBRARY: &str = include_str!("../library/cam-gcode-dialects.json");

/// The identifier of the JSON form of a program, which no G-code dialect may take.
pub(crate) const CONTROLLER_NEUTRAL_JSON: &str = "controller_neutral_json";

/// Whether a whole number keeps its decimal point (`10.`) or drops it (`10`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CamDecimalPoint {
    Always,
    WhenFractional,
}

/// Sequence numbers in front of every line between the start and end lines; after
/// `last` they start again at `first`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CamLineNumbers {
    pub prefix: String,
    pub first: u32,
    pub step: u32,
    pub last: u32,
}

impl CamLineNumbers {
    fn next(&self, number: u32) -> u32 {
        match number.checked_add(self.step) {
            Some(next) if next <= self.last => next,
            _ => self.first,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MotionSource {
    rapid: String,
    retract: String,
    plunge: String,
    cut: String,
    arc_clockwise: String,
    arc_counterclockwise: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DialectSource {
    schema: String,
    id: String,
    name: String,
    sources: Vec<String>,
    decimals: u8,
    decimal_point: CamDecimalPoint,
    uppercase: bool,
    line_numbers: Option<CamLineNumbers>,
    start: Vec<String>,
    header: Vec<String>,
    motions: MotionSource,
    footer: Vec<String>,
    end: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Field {
    PlanDigest,
    ToolpathDigest,
    SimulationFingerprint,
    WorkOffset,
    Tool,
    SpindleRpm,
    CuttingFeed,
    PlungeFeed,
    SafeZ,
    X,
    Y,
    Z,
    I,
    J,
    Feed,
}

const FIELDS: [(&str, Field); 15] = [
    ("plan_digest", Field::PlanDigest),
    ("toolpath_digest", Field::ToolpathDigest),
    ("simulation_fingerprint", Field::SimulationFingerprint),
    ("work_offset", Field::WorkOffset),
    ("tool", Field::Tool),
    ("spindle_rpm", Field::SpindleRpm),
    ("cutting_feed", Field::CuttingFeed),
    ("plunge_feed", Field::PlungeFeed),
    ("safe_z", Field::SafeZ),
    ("x", Field::X),
    ("y", Field::Y),
    ("z", Field::Z),
    ("i", Field::I),
    ("j", Field::J),
    ("feed", Field::Feed),
];

const HEADER_FIELDS: [Field; 9] = [
    Field::PlanDigest,
    Field::ToolpathDigest,
    Field::SimulationFingerprint,
    Field::WorkOffset,
    Field::Tool,
    Field::SpindleRpm,
    Field::CuttingFeed,
    Field::PlungeFeed,
    Field::SafeZ,
];

const MOTION_KINDS: [CamPostprocessedMotionKind; 6] = [
    CamPostprocessedMotionKind::Rapid,
    CamPostprocessedMotionKind::Retract,
    CamPostprocessedMotionKind::Plunge,
    CamPostprocessedMotionKind::Cut,
    CamPostprocessedMotionKind::ArcClockwise,
    CamPostprocessedMotionKind::ArcCounterclockwise,
];

impl Field {
    fn name(self) -> &'static str {
        FIELDS
            .iter()
            .find(|(_, field)| *field == self)
            .map_or("value", |(name, _)| name)
    }
}

fn motion_name(kind: CamPostprocessedMotionKind) -> &'static str {
    match kind {
        CamPostprocessedMotionKind::Rapid => "rapid",
        CamPostprocessedMotionKind::Retract => "retract",
        CamPostprocessedMotionKind::Plunge => "plunge",
        CamPostprocessedMotionKind::Cut => "cut",
        CamPostprocessedMotionKind::ArcClockwise => "arc_clockwise",
        CamPostprocessedMotionKind::ArcCounterclockwise => "arc_counterclockwise",
    }
}

fn motion_fields(kind: CamPostprocessedMotionKind) -> &'static [Field] {
    match kind {
        CamPostprocessedMotionKind::Rapid | CamPostprocessedMotionKind::Retract => {
            &[Field::X, Field::Y, Field::Z]
        }
        CamPostprocessedMotionKind::Plunge | CamPostprocessedMotionKind::Cut => {
            &[Field::X, Field::Y, Field::Z, Field::Feed]
        }
        CamPostprocessedMotionKind::ArcClockwise
        | CamPostprocessedMotionKind::ArcCounterclockwise => &[
            Field::X,
            Field::Y,
            Field::Z,
            Field::I,
            Field::J,
            Field::Feed,
        ],
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Segment {
    Text(String),
    Value(Field),
}

#[derive(Clone, Debug, PartialEq)]
struct Template(Vec<Segment>);

impl Template {
    fn fields(&self) -> impl Iterator<Item = Field> + '_ {
        self.0.iter().filter_map(|segment| match segment {
            Segment::Value(field) => Some(*field),
            Segment::Text(_) => None,
        })
    }

    fn texts(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter_map(|segment| match segment {
            Segment::Text(text) => Some(text.as_str()),
            Segment::Value(_) => None,
        })
    }

    /// The source form, `G1 X{x} ...`, for messages.
    fn source(&self) -> String {
        self.0
            .iter()
            .map(|segment| match segment {
                Segment::Text(text) => text.clone(),
                Segment::Value(field) => format!("{{{}}}", field.name()),
            })
            .collect()
    }

    /// The values of `line` when it has this template's form.
    fn capture<'a>(&self, line: &'a str) -> Option<Vec<(Field, &'a str)>> {
        let mut rest = line;
        let mut values = Vec::new();
        let mut segments = self.0.iter().peekable();
        while let Some(segment) = segments.next() {
            match segment {
                Segment::Text(text) => rest = rest.strip_prefix(text.as_str())?,
                Segment::Value(field) => {
                    let end = match segments.peek() {
                        Some(Segment::Text(next)) => rest.find(next.as_str())?,
                        _ => rest.len(),
                    };
                    let (value, tail) = rest.split_at(end);
                    if value.is_empty() {
                        return None;
                    }
                    values.push((*field, value));
                    rest = tail;
                }
            }
        }
        rest.is_empty().then_some(values)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct MotionTemplates {
    rapid: Template,
    retract: Template,
    plunge: Template,
    cut: Template,
    arc_clockwise: Template,
    arc_counterclockwise: Template,
}

impl MotionTemplates {
    fn get(&self, kind: CamPostprocessedMotionKind) -> &Template {
        match kind {
            CamPostprocessedMotionKind::Rapid => &self.rapid,
            CamPostprocessedMotionKind::Retract => &self.retract,
            CamPostprocessedMotionKind::Plunge => &self.plunge,
            CamPostprocessedMotionKind::Cut => &self.cut,
            CamPostprocessedMotionKind::ArcClockwise => &self.arc_clockwise,
            CamPostprocessedMotionKind::ArcCounterclockwise => &self.arc_counterclockwise,
        }
    }
}

/// Why a dialect was refused; a refused dialect writes no program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CamGCodeDialectError {
    /// The JSON does not have the dialect's shape.
    Json {
        line: usize,
        column: usize,
        category: serde_json::error::Category,
    },
    Schema {
        found: String,
    },
    UnknownLibraryDialect {
        id: String,
    },
    DuplicateLibraryDialect {
        id: String,
    },
    Field {
        field: &'static str,
        reason: &'static str,
    },
    /// A line of a section; `line` is 1-based, `None` for the section as a whole.
    Template {
        section: &'static str,
        line: Option<usize>,
        reason: &'static str,
    },
    /// A motion of the first kind would read back as the second.
    Ambiguous {
        first: CamPostprocessedMotionKind,
        second: CamPostprocessedMotionKind,
    },
}

impl CamGCodeDialectError {
    fn json(error: &serde_json::Error) -> Self {
        Self::Json {
            line: error.line(),
            column: error.column(),
            category: error.classify(),
        }
    }
}

impl std::fmt::Display for CamGCodeDialectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json {
                line,
                column,
                category,
            } => write!(
                formatter,
                "the G-code dialect is not valid dialect JSON ({category:?} at line {line}, column {column})"
            ),
            Self::Schema { found } => write!(
                formatter,
                "the G-code dialect schema is {found:?}, not {CAM_GCODE_DIALECT_SCHEMA_V1}"
            ),
            Self::UnknownLibraryDialect { id } => {
                write!(formatter, "the library has no postprocessor dialect {id:?}")
            }
            Self::DuplicateLibraryDialect { id } => {
                write!(formatter, "the library has two dialects named {id:?}")
            }
            Self::Field { field, reason } => {
                write!(formatter, "the G-code dialect's {field} {reason}")
            }
            Self::Template {
                section,
                line: Some(line),
                reason,
            } => write!(
                formatter,
                "line {line} of the G-code dialect's {section} {reason}"
            ),
            Self::Template {
                section,
                line: None,
                reason,
            } => write!(formatter, "the G-code dialect's {section} {reason}"),
            Self::Ambiguous { first, second } => write!(
                formatter,
                "a {} motion of the G-code dialect would read back as a {} motion",
                motion_name(*first),
                motion_name(*second)
            ),
        }
    }
}

impl std::error::Error for CamGCodeDialectError {}

/// A controller's G-code: the fixed lines, a template per line kind and how numbers
/// are written. Built only through validation, so every dialect reads back what it
/// writes.
#[derive(Clone, Debug, PartialEq)]
pub struct CamGCodeDialect {
    id: String,
    name: String,
    sources: Vec<String>,
    decimals: u8,
    decimal_point: CamDecimalPoint,
    uppercase: bool,
    line_numbers: Option<CamLineNumbers>,
    start: Vec<String>,
    header: Vec<Template>,
    motions: MotionTemplates,
    footer: Vec<String>,
    end: Vec<String>,
}

impl CamGCodeDialect {
    /// A dialect from its JSON description, refused unless it reads back what it writes.
    pub fn from_json(json: &str) -> Result<Self, CamGCodeDialectError> {
        let source: DialectSource =
            serde_json::from_str(json).map_err(|error| CamGCodeDialectError::json(&error))?;
        Self::from_source(source)
    }

    /// The dialects shipped with Ketchup.
    pub fn library() -> Result<Vec<Self>, CamGCodeDialectError> {
        let sources: Vec<DialectSource> =
            serde_json::from_str(LIBRARY).map_err(|error| CamGCodeDialectError::json(&error))?;
        let mut dialects: Vec<Self> = Vec::with_capacity(sources.len());
        for source in sources {
            let dialect = Self::from_source(source)?;
            if dialects.iter().any(|known| known.id == dialect.id) {
                return Err(CamGCodeDialectError::DuplicateLibraryDialect { id: dialect.id });
            }
            dialects.push(dialect);
        }
        Ok(dialects)
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the dialect's conventions come from.
    #[must_use]
    pub fn sources(&self) -> &[String] {
        &self.sources
    }

    /// The step the dialect writes lengths and feeds in.
    #[must_use]
    pub fn resolution_mm(&self) -> f64 {
        1.0 / self.scale()
    }

    pub(crate) fn scale(&self) -> f64 {
        10f64.powi(i32::from(self.decimals))
    }

    fn from_source(source: DialectSource) -> Result<Self, CamGCodeDialectError> {
        let field = |field, reason| CamGCodeDialectError::Field { field, reason };
        if source.schema != CAM_GCODE_DIALECT_SCHEMA_V1 {
            return Err(CamGCodeDialectError::Schema {
                found: source.schema,
            });
        }
        if source.id.is_empty()
            || source.id == CONTROLLER_NEUTRAL_JSON
            || !source
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(field(
                "id",
                "must be lower-case letters, digits and '_', and not the JSON form's id",
            ));
        }
        if source.name.trim().is_empty() {
            return Err(field("name", "is empty"));
        }
        if source.sources.iter().all(|text| text.trim().is_empty()) {
            return Err(field("sources", "name no source for the conventions"));
        }
        if !(1..=9).contains(&source.decimals) {
            return Err(field("decimals", "must be 1 to 9"));
        }
        if let Some(numbers) = &source.line_numbers
            && (numbers.prefix.is_empty()
                || !numbers.prefix.chars().all(|c| c.is_ascii_alphabetic())
                || (source.uppercase && numbers.prefix.chars().any(|c| c.is_ascii_lowercase()))
                || numbers.step == 0
                || numbers.first > numbers.last)
        {
            return Err(field(
                "line_numbers",
                "need a letter prefix, a step above zero and first not above last",
            ));
        }
        let uppercase = source.uppercase;
        let fixed = |section, lines: &[String]| -> Result<Vec<String>, CamGCodeDialectError> {
            for (index, line) in lines.iter().enumerate() {
                template(line, section, index, &[], uppercase)?;
            }
            Ok(lines.to_vec())
        };
        let start = fixed("start", &source.start)?;
        let footer = fixed("footer", &source.footer)?;
        let end = fixed("end", &source.end)?;
        let header = source
            .header
            .iter()
            .enumerate()
            .map(|(index, line)| template(line, "header", index, &HEADER_FIELDS, uppercase))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(missing) = HEADER_FIELDS.iter().find(|needed| {
            !header
                .iter()
                .any(|line| line.fields().any(|f| f == **needed))
        }) {
            return Err(CamGCodeDialectError::Template {
                section: "header",
                line: None,
                reason: missing_reason(*missing),
            });
        }
        let motion = |kind, text: &str| -> Result<Template, CamGCodeDialectError> {
            let section = motion_name(kind);
            let fields = motion_fields(kind);
            let parsed = template(text, section, 0, fields, uppercase)?;
            if let Some(missing) = fields
                .iter()
                .find(|needed| !parsed.fields().any(|f| f == **needed))
            {
                return Err(CamGCodeDialectError::Template {
                    section,
                    line: None,
                    reason: missing_reason(*missing),
                });
            }
            Ok(parsed)
        };
        let motions = MotionTemplates {
            rapid: motion(CamPostprocessedMotionKind::Rapid, &source.motions.rapid)?,
            retract: motion(CamPostprocessedMotionKind::Retract, &source.motions.retract)?,
            plunge: motion(CamPostprocessedMotionKind::Plunge, &source.motions.plunge)?,
            cut: motion(CamPostprocessedMotionKind::Cut, &source.motions.cut)?,
            arc_clockwise: motion(
                CamPostprocessedMotionKind::ArcClockwise,
                &source.motions.arc_clockwise,
            )?,
            arc_counterclockwise: motion(
                CamPostprocessedMotionKind::ArcCounterclockwise,
                &source.motions.arc_counterclockwise,
            )?,
        };
        let dialect = Self {
            id: source.id,
            name: source.name,
            sources: source.sources,
            decimals: source.decimals,
            decimal_point: source.decimal_point,
            uppercase,
            line_numbers: source.line_numbers,
            start,
            header,
            motions,
            footer,
            end,
        };
        dialect.refuse_ambiguous_motions()?;
        Ok(dialect)
    }

    /// Every motion line must read back as its own kind only.
    fn refuse_ambiguous_motions(&self) -> Result<(), CamGCodeDialectError> {
        for first in MOTION_KINDS {
            let sample = CamPostprocessedMotion {
                kind: first,
                end_mm: [-12.5, 3.0, -0.25],
                center_offset_mm: Some([4.75, -6.0]),
                feed_mm_per_min: Some(1200.0),
            };
            let line = self
                .motions
                .get(first)
                .0
                .iter()
                .map(|segment| match segment {
                    Segment::Text(text) => Some(text.clone()),
                    Segment::Value(field) => self.motion_value(*field, &sample),
                })
                .collect::<Option<String>>()
                .ok_or(CamGCodeDialectError::Template {
                    section: motion_name(first),
                    line: None,
                    reason: "names a value its motion does not have",
                })?;
            for second in MOTION_KINDS {
                if second != first && self.motions.get(second).capture(&line).is_some() {
                    return Err(CamGCodeDialectError::Ambiguous { first, second });
                }
            }
        }
        Ok(())
    }

    fn number(&self, value: f64) -> String {
        let mut text = format!("{value:.*}", usize::from(self.decimals));
        if text.contains('.') {
            let kept = text.trim_end_matches('0').len();
            text.truncate(kept);
            if text.ends_with('.') && self.decimal_point == CamDecimalPoint::WhenFractional {
                text.pop();
            }
        }
        match text.strip_prefix('-') {
            Some(magnitude) if magnitude.chars().all(|c| c == '0' || c == '.') => {
                magnitude.to_owned()
            }
            _ => text,
        }
    }

    fn digest(&self, digest: &str) -> String {
        if self.uppercase {
            digest.to_ascii_uppercase()
        } else {
            digest.to_owned()
        }
    }

    fn header_value(&self, field: Field, program: &CamPostprocessedProgram) -> Option<String> {
        Some(match field {
            Field::PlanDigest => self.digest(&program.plan_digest),
            Field::ToolpathDigest => self.digest(&program.toolpath_digest),
            Field::SimulationFingerprint => self.digest(&program.simulation_fingerprint),
            Field::WorkOffset => program.work_offset.clone(),
            Field::Tool => program.tool_number.to_string(),
            Field::SpindleRpm => program.spindle_rpm.to_string(),
            Field::CuttingFeed => self.number(program.cutting_feed_mm_per_min),
            Field::PlungeFeed => self.number(program.plunge_feed_mm_per_min),
            Field::SafeZ => self.number(program.safe_retract_z_mm),
            Field::X | Field::Y | Field::Z | Field::I | Field::J | Field::Feed => return None,
        })
    }

    fn motion_value(&self, field: Field, motion: &CamPostprocessedMotion) -> Option<String> {
        let [x, y, z] = motion.end_mm;
        match field {
            Field::X => Some(self.number(x)),
            Field::Y => Some(self.number(y)),
            Field::Z => Some(self.number(z)),
            Field::I => motion.center_offset_mm.map(|[i, _]| self.number(i)),
            Field::J => motion.center_offset_mm.map(|[_, j]| self.number(j)),
            Field::Feed => motion.feed_mm_per_min.map(|feed| self.number(feed)),
            _ => None,
        }
    }

    fn fill(
        template: &Template,
        value: impl Fn(Field) -> Option<String>,
    ) -> Result<String, CamPostprocessorError> {
        let mut line = String::new();
        for segment in &template.0 {
            match segment {
                Segment::Text(text) => line.push_str(text),
                Segment::Value(field) => line.push_str(&value(*field).ok_or_else(|| {
                    malformed(format!("the program has no {} for this line", field.name()))
                })?),
            }
        }
        Ok(line)
    }

    pub(crate) fn render(
        &self,
        program: &CamPostprocessedProgram,
    ) -> Result<String, CamPostprocessorError> {
        let mut numbered =
            Vec::with_capacity(self.header.len() + program.motions.len() + self.footer.len());
        for template in &self.header {
            numbered.push(Self::fill(template, |field| {
                self.header_value(field, program)
            })?);
        }
        for motion in &program.motions {
            numbered.push(Self::fill(self.motions.get(motion.kind), |field| {
                self.motion_value(field, motion)
            })?);
        }
        numbered.extend(self.footer.iter().cloned());
        let mut lines = self.start.clone();
        let mut sequence = self
            .line_numbers
            .as_ref()
            .map(|numbers| (numbers, numbers.first));
        for line in numbered {
            match &mut sequence {
                Some((numbers, number)) => {
                    lines.push(format!("{}{number} {line}", numbers.prefix));
                    *number = numbers.next(*number);
                }
                None => lines.push(line),
            }
        }
        lines.extend(self.end.iter().cloned());
        let mut text = lines.join("\n");
        text.push('\n');
        Ok(text)
    }

    pub(crate) fn parse(
        &self,
        bytes: &[u8],
    ) -> Result<CamPostprocessedProgram, CamPostprocessorError> {
        let text = std::str::from_utf8(bytes).map_err(|error| {
            malformed(format!(
                "the program is not UTF-8 after byte {}",
                error.valid_up_to()
            ))
        })?;
        let body = text
            .strip_suffix('\n')
            .ok_or_else(|| malformed("the program does not end with a line break"))?;
        let lines = body.split('\n').collect::<Vec<_>>();
        let fixed = self.start.len() + self.header.len() + self.footer.len() + self.end.len();
        if lines.len() <= fixed {
            return Err(malformed("the program has no motions"));
        }
        let (start, rest) = lines.split_at(self.start.len());
        let (numbered, end) = rest.split_at(rest.len() - self.end.len());
        if start != self.start.as_slice() || end != self.end.as_slice() {
            return Err(malformed(format!(
                "the program does not start with {:?} and end with {:?}",
                self.start, self.end
            )));
        }
        let mut sequence = self
            .line_numbers
            .as_ref()
            .map(|numbers| (numbers, numbers.first));
        let mut stripped = Vec::with_capacity(numbered.len());
        for line in numbered {
            match &mut sequence {
                Some((numbers, number)) => {
                    let prefix = format!("{}{number} ", numbers.prefix);
                    stripped.push(line.strip_prefix(prefix.as_str()).ok_or_else(|| {
                        malformed(format!(
                            "line {line:?} does not start with sequence number {prefix:?}"
                        ))
                    })?);
                    *number = numbers.next(*number);
                }
                None => stripped.push(*line),
            }
        }
        let (header, rest) = stripped.split_at(self.header.len());
        let (motion_lines, footer) = rest.split_at(rest.len() - self.footer.len());
        if footer != self.footer.as_slice() {
            return Err(malformed(format!(
                "the program does not end its motions with {:?}",
                self.footer
            )));
        }
        let mut values = BTreeMap::new();
        for (template, line) in self.header.iter().zip(header) {
            let captured = template.capture(line).ok_or_else(|| {
                malformed(format!(
                    "line {line:?} is not the dialect's {:?}",
                    template.source()
                ))
            })?;
            for (field, value) in captured {
                if *values.entry(field).or_insert(value) != value {
                    return Err(malformed(format!(
                        "the program gives two values for {}",
                        field.name()
                    )));
                }
            }
        }
        let header_value = |field: Field| {
            values
                .get(&field)
                .copied()
                .ok_or_else(|| malformed(format!("the program has no {}", field.name())))
        };
        let motions = motion_lines
            .iter()
            .map(|line| self.parse_motion(line))
            .collect::<Result<Vec<_>, _>>()?;
        let program = CamPostprocessedProgram {
            schema: CAM_POSTPROCESSOR_SCHEMA_V1.to_owned(),
            plan_digest: header_value(Field::PlanDigest)?.to_ascii_lowercase(),
            toolpath_digest: header_value(Field::ToolpathDigest)?.to_ascii_lowercase(),
            simulation_fingerprint: header_value(Field::SimulationFingerprint)?
                .to_ascii_lowercase(),
            units: "mm".to_owned(),
            work_offset: header_value(Field::WorkOffset)?.to_owned(),
            tool_number: parse_whole(header_value(Field::Tool)?)?,
            spindle_rpm: parse_whole(header_value(Field::SpindleRpm)?)?,
            cutting_feed_mm_per_min: parse_number(header_value(Field::CuttingFeed)?)?,
            plunge_feed_mm_per_min: parse_number(header_value(Field::PlungeFeed)?)?,
            safe_retract_z_mm: parse_number(header_value(Field::SafeZ)?)?,
            motions,
        };
        if self.render(&program)? != text {
            return Err(malformed(
                "the program is not written the way the dialect writes it",
            ));
        }
        Ok(program)
    }

    fn parse_motion(&self, line: &str) -> Result<CamPostprocessedMotion, CamPostprocessorError> {
        let matches = MOTION_KINDS
            .iter()
            .filter_map(|kind| {
                self.motions
                    .get(*kind)
                    .capture(line)
                    .map(|values| (*kind, values))
            })
            .collect::<Vec<_>>();
        let [(kind, captured)] = matches.as_slice() else {
            return Err(malformed(format!(
                "line {line:?} is not exactly one motion of the dialect"
            )));
        };
        let mut values = BTreeMap::new();
        for (field, value) in captured {
            values.insert(*field, parse_number(value)?);
        }
        let value = |field: Field| {
            values
                .get(&field)
                .copied()
                .ok_or_else(|| malformed(format!("line {line:?} has no {}", field.name())))
        };
        let is_arc = matches!(
            kind,
            CamPostprocessedMotionKind::ArcClockwise
                | CamPostprocessedMotionKind::ArcCounterclockwise
        );
        let has_feed = !matches!(
            kind,
            CamPostprocessedMotionKind::Rapid | CamPostprocessedMotionKind::Retract
        );
        Ok(CamPostprocessedMotion {
            kind: *kind,
            end_mm: [value(Field::X)?, value(Field::Y)?, value(Field::Z)?],
            center_offset_mm: if is_arc {
                Some([value(Field::I)?, value(Field::J)?])
            } else {
                None
            },
            feed_mm_per_min: if has_feed {
                Some(value(Field::Feed)?)
            } else {
                None
            },
        })
    }
}

fn missing_reason(field: Field) -> &'static str {
    match field {
        Field::PlanDigest => "lacks {plan_digest}",
        Field::ToolpathDigest => "lacks {toolpath_digest}",
        Field::SimulationFingerprint => "lacks {simulation_fingerprint}",
        Field::WorkOffset => "lacks {work_offset}",
        Field::Tool => "lacks {tool}",
        Field::SpindleRpm => "lacks {spindle_rpm}",
        Field::CuttingFeed => "lacks {cutting_feed}",
        Field::PlungeFeed => "lacks {plunge_feed}",
        Field::SafeZ => "lacks {safe_z}",
        Field::X => "lacks {x}",
        Field::Y => "lacks {y}",
        Field::Z => "lacks {z}",
        Field::I => "lacks {i}",
        Field::J => "lacks {j}",
        Field::Feed => "lacks {feed}",
    }
}

/// One line of a section; `allowed` are the values it may hold.
fn template(
    text: &str,
    section: &'static str,
    index: usize,
    allowed: &[Field],
    uppercase: bool,
) -> Result<Template, CamGCodeDialectError> {
    let error = |reason| CamGCodeDialectError::Template {
        section,
        line: Some(index + 1),
        reason,
    };
    if text.trim().is_empty() || text.contains(['\n', '\r']) {
        return Err(error("must be one line that is not blank"));
    }
    if text.len() > ketchup_tolerance::limits::TEXT_BYTES {
        return Err(error("is longer than a text may be"));
    }
    let mut segments = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        if !before.is_empty() {
            segments.push(Segment::Text(before.to_owned()));
        }
        let close = after
            .find('}')
            .ok_or_else(|| error("has a '{' without its '}'"))?;
        let name = after
            .get(1..close)
            .ok_or_else(|| error("has a '{' without its '}'"))?;
        let field = FIELDS
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, field)| *field)
            .ok_or_else(|| error("names a value the dialect format does not know"))?;
        if !allowed.contains(&field) {
            return Err(error("names a value this line cannot hold"));
        }
        if matches!(segments.last(), Some(Segment::Value(_))) {
            return Err(error("has two values with no text between them"));
        }
        segments.push(Segment::Value(field));
        // `}` is one byte, so the text after it starts on a character boundary.
        rest = after.split_at(close + 1).1;
    }
    if !rest.is_empty() {
        segments.push(Segment::Text(rest.to_owned()));
    }
    let parsed = Template(segments);
    if parsed.texts().any(|text| text.contains('}')) {
        return Err(error("has a '}' without its '{'"));
    }
    if uppercase
        && parsed
            .texts()
            .any(|text| text.chars().any(|c| c.is_ascii_lowercase()))
    {
        return Err(error("has lower-case letters in an upper-case dialect"));
    }
    for pair in parsed.0.windows(2) {
        if let [Segment::Value(_), Segment::Text(text)] = pair
            && text.starts_with(|c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        {
            return Err(error(
                "has text that would read as part of the value before it",
            ));
        }
    }
    Ok(parsed)
}

fn parse_number(text: &str) -> Result<f64, CamPostprocessorError> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(malformed(format!("{text:?} is not a number")));
    }
    text.parse()
        .map_err(|_not_a_number: std::num::ParseFloatError| {
            malformed(format!("{text:?} is not a number"))
        })
}

fn parse_whole(text: &str) -> Result<u32, CamPostprocessorError> {
    text.parse()
        .map_err(|_not_a_whole_number: std::num::ParseIntError| {
            malformed(format!("{text:?} is not an unsigned integer"))
        })
}
