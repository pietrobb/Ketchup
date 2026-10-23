//! A DXF drawing read as an underlay: every line, arc and curve it draws, by layer, as
//! polylines, without making profiles of them, its texts and its hatches
//! ([`super::hatch`]), each in the color, linetype and lineweight the drawing gives it
//! ([`super::style`]), seen from above: heights,
//! elevations and thicknesses are dropped and counted once. An entity it cannot read is
//! left out and counted in the report. A drawing whose coordinates leave the canonical
//! envelope, such as one in national grid coordinates, is moved near the origin by an
//! offset the report names and the underlay keeps.
//!
//! The drawing is read line by line, never whole: a first pass finds its text encoding, a
//! second checks its sections and keeps its header, tables and blocks, and a third reads its
//! entities one at a time (with the vertices or attributes that follow one) straight into
//! the lines. Memory grows with the lines kept, not with the size of the file.

use super::*;
use crate::underlay::style::LineStyle;
use crate::underlay::{UnderlayLines, UnderlayLinesBuilder};
use std::io::{BufRead, Seek, SeekFrom};
use std::ops::Range;

/// The lines of a DXF drawing and what reading it reported.
#[derive(Clone, Debug, PartialEq)]
pub struct DxfUnderlay {
    pub units: ImportUnitDecision,
    /// The names of the layers of `lines`, in their order.
    pub layers: Vec<String>,
    /// The layers of `layers` the drawing turned off or froze: kept, but shown only when
    /// asked to.
    pub hidden_layers: Vec<String>,
    pub lines: UnderlayLines,
    /// What `lines` were moved by from the drawing's own coordinates
    /// ([`UnderlayLines::move_near_origin`]).
    pub offset_mm: [f64; 2],
    pub diagnostics: Vec<ImportDiagnostic>,
}

/// Reads every drawn entity of `source` into underlay lines, without the limits of a
/// profile import; run it where memory and time are bounded, in the program worker.
pub fn read_dxf_underlay(
    source: &[u8],
    options: DxfImportOptions,
) -> Result<DxfUnderlay, DxfImportError> {
    read_dxf_underlay_from(std::io::Cursor::new(source), options)
}

/// [`read_dxf_underlay`] of a drawing read line by line from `source`, such as a buffered
/// file, which is read three times from its start.
pub fn read_dxf_underlay_from(
    mut source: impl BufRead + Seek,
    options: DxfImportOptions,
) -> Result<DxfUnderlay, DxfImportError> {
    let _any_coordinate = AnyFiniteCoordinate::new();
    let code_page = text_encoding(&mut source)?;
    let encoding = code_page.as_ref().map(|(encoding, _)| *encoding);
    let mut diagnostics = DiagnosticCounts::new(UNDERLAY_READING);
    if let Some((_, name)) = code_page {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.text-code-page",
            Some(name),
            1,
        )?;
    }
    let sections = read_sections(PairReader::new(&mut source, encoding)?, &mut diagnostics)?;
    let header = sections.header.pairs();
    let (units, _) = parse_header(&header, options, &mut diagnostics)?;
    let styles =
        style::DrawingStyles::read(&sections.tables.pairs(), &header, units, &mut diagnostics)?;
    diagnostics.frozen_layers = hidden_layers(&sections.tables.pairs(), false);
    let mut entity_count = 0;
    let blocks = parse_blocks(
        &sections.blocks.pairs(),
        units,
        &mut entity_count,
        &mut diagnostics,
    )?;
    if !sections.entities {
        return Err(DxfImportError::MissingEntitiesSection);
    }
    let mut reader = PairReader::new(&mut source, encoding)?;
    let mut builder = UnderlayLinesBuilder::default();
    let mut layers = BTreeSet::new();
    let mut entity = PairBuffer::default();
    let mut read_entity = |entity: &mut PairBuffer, diagnostics: &mut DiagnosticCounts| {
        let mut parsed = ParsedEntities::default();
        parse_entities(
            &entity.pairs(),
            units,
            Some(&blocks),
            &mut entity_count,
            &mut parsed,
            diagnostics,
        )?;
        parsed.report_xrefs(diagnostics)?;
        for loose in &parsed.loose {
            builder.set_style(&styles.line_style(&loose.style, &loose.layer, diagnostics)?);
            builder.add_loose(&loose.layer, &loose.segment);
        }
        for profile in &parsed.explicit_profiles {
            builder.set_style(&styles.line_style(&profile.style, &profile.layer, diagnostics)?);
            builder.add(&profile.layer, &profile.segments);
        }
        for text in parsed.texts.drain(..) {
            builder.set_style(&styles.line_style(&text.style, &text.layer, diagnostics)?);
            builder.add_text(&text.layer, text.text);
        }
        for fill in &parsed.fills {
            // A hatch is painted, and its pattern drawn, continuous: its own dashes
            // are the pattern's.
            let style = styles.line_style(&fill.style, &fill.layer, diagnostics)?;
            builder.set_style(&LineStyle {
                pattern_mm: Vec::new(),
                ..style
            });
            builder.add_fill(&fill.layer, &fill.rings);
            builder.add_pieces(&fill.layer, &fill.pattern);
        }
        layers.append(&mut parsed.layers);
        entity.clear();
        Ok::<_, DxfImportError>(())
    };
    reader.skip_to_section("ENTITIES")?;
    reader.in_drawing = true;
    let mut value = String::new();
    // An entity ends where the next begins, unless it waits for its SEQEND: a POLYLINE
    // always, an INSERT whose own record has group 66 = 1 (as the profile import reads
    // them, every entity up to the SEQEND belongs to it).
    let (mut awaits_seqend, mut in_head) = (false, false);
    loop {
        let code = reader
            .next_pair(&mut value)?
            .ok_or(DxfImportError::MalformedSections)?;
        if code == 0 && value.eq_ignore_ascii_case("ENDSEC") {
            break;
        }
        if code == 0 && awaits_seqend {
            awaits_seqend = !value.eq_ignore_ascii_case("SEQEND");
            in_head = false;
        } else if code == 0 {
            if !entity.is_empty() {
                read_entity(&mut entity, &mut diagnostics)?;
            }
            awaits_seqend = value.eq_ignore_ascii_case("POLYLINE");
            in_head = true;
        } else if code == 66 && in_head && entity.is_head("INSERT") {
            // Waiting too long only joins whole entities into one read; ending too early
            // would cut one.
            awaits_seqend |= value.parse::<i64>() == Ok(1);
        }
        entity.push_flat(code, &value);
    }
    if !entity.is_empty() {
        read_entity(&mut entity, &mut diagnostics)?;
    }
    let flattened = entity.flattened + sections.blocks.flattened;
    if flattened > 0 {
        diagnostics.add(
            ImportDiagnosticSeverity::Info,
            "dxf.height-flattened",
            None,
            u32::try_from(flattened).unwrap_or(u32::MAX),
        )?;
    }
    let (layer_names, mut lines) = builder.finish();
    let moved = lines.move_near_origin();
    let offset_mm = moved.offset_mm;
    for (code, outside) in [
        ("dxf.lines-outside-envelope", moved.segments_outside),
        ("dxf.texts-outside-envelope", moved.texts_outside),
        ("dxf.fills-outside-envelope", moved.fills_outside),
    ] {
        if outside > 0 {
            diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                code,
                None,
                u32::try_from(outside).unwrap_or(u32::MAX),
            )?;
        }
    }
    if lines.segment_count() == 0 && lines.text_count() == 0 && lines.fill_count() == 0 {
        return Err(diagnostics.nothing_left());
    }
    let diagnostics = finish_report(diagnostics, &layers, units, offset_mm)?;
    let hidden = hidden_layers(&sections.tables.pairs(), true);
    let hidden_layers = layer_names
        .iter()
        .filter(|layer| hidden.contains(&layer.to_ascii_uppercase()))
        .cloned()
        .collect();
    Ok(DxfUnderlay {
        units,
        hidden_layers,
        layers: layer_names,
        lines,
        offset_mm,
        diagnostics,
    })
}

/// Group codes and values, the values in one text so a section or an entity costs a
/// few allocations, not one per value.
#[derive(Default)]
struct PairBuffer {
    text: String,
    pairs: Vec<(i16, Range<usize>)>,
    /// Whether the entity being pushed had a height dropped.
    flattening: bool,
    /// How many entities had a Z, an elevation or a thickness dropped.
    flattened: usize,
}

impl PairBuffer {
    fn push(&mut self, code: i16, value: &str) {
        let start = self.text.len();
        self.text.push_str(value);
        self.pairs.push((code, start..self.text.len()));
    }

    /// [`Self::push`] of an entity seen from above: an underlay is a plan, so a Z
    /// coordinate (groups 30-37), an elevation (38) or a thickness (39) is read as 0,
    /// as a surveyed drawing in national grid coordinates carries one on nearly every
    /// line. An extrusion direction other than ±Z is still refused by the entity.
    fn push_flat(&mut self, code: i16, value: &str) {
        // The vertices and attributes that follow an entity are part of it.
        let follows = ["VERTEX", "ATTRIB", "SEQEND"]
            .iter()
            .any(|kind| value.eq_ignore_ascii_case(kind));
        if code == 0 && !follows {
            self.flattening = false;
        }
        let height = (30..=39).contains(&code)
            && value
                .trim()
                .parse::<f64>()
                .is_ok_and(|height| height != 0.0);
        if !height {
            return self.push(code, value);
        }
        if !self.flattening {
            self.flattening = true;
            self.flattened += 1;
        }
        self.push(code, "0");
    }

    fn pairs(&self) -> Vec<DxfPair<'_>> {
        self.pairs
            .iter()
            .map(|(code, range)| DxfPair {
                code: *code,
                value: &self.text[range.clone()],
            })
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Whether the buffer holds an entity of `kind`.
    fn is_head(&self, kind: &str) -> bool {
        self.pairs.first().is_some_and(|(code, range)| {
            *code == 0 && self.text[range.clone()].eq_ignore_ascii_case(kind)
        })
    }

    fn clear(&mut self) {
        self.text.clear();
        self.pairs.clear();
        self.flattening = false;
    }
}

/// What the sections of a drawing hold that its entities need.
struct Sections {
    header: PairBuffer,
    /// The layers and linetypes their styles name.
    tables: PairBuffer,
    blocks: PairBuffer,
    entities: bool,
}

/// Checks the SECTION/ENDSEC/EOF structure as a profile import does, keeping the header,
/// the tables and the blocks.
fn read_sections(
    mut reader: PairReader<'_, impl BufRead>,
    diagnostics: &mut DiagnosticCounts,
) -> Result<Sections, DxfImportError> {
    let mut sections = Sections {
        header: PairBuffer::default(),
        tables: PairBuffer::default(),
        blocks: PairBuffer::default(),
        entities: false,
    };
    let mut names = BTreeSet::new();
    let mut value = String::new();
    loop {
        let code = reader
            .next_pair(&mut value)?
            .ok_or(DxfImportError::MalformedSections)?;
        if code == 0 && value.eq_ignore_ascii_case("EOF") {
            break;
        }
        if code != 0
            || !value.eq_ignore_ascii_case("SECTION")
            || reader.next_pair(&mut value)? != Some(2)
        {
            return Err(DxfImportError::MalformedSections);
        }
        let name = value.to_ascii_uppercase();
        reader.in_drawing = name != "HEADER";
        let mut kept = match name.as_str() {
            "HEADER" => Some(&mut sections.header),
            "TABLES" => Some(&mut sections.tables),
            "BLOCKS" => Some(&mut sections.blocks),
            _ => None,
        };
        loop {
            let code = reader
                .next_pair(&mut value)?
                .ok_or(DxfImportError::MalformedSections)?;
            if code == 0 && value.eq_ignore_ascii_case("ENDSEC") {
                break;
            }
            match kept.as_mut() {
                Some(blocks) if name == "BLOCKS" => blocks.push_flat(code, &value),
                Some(kept) => kept.push(code, &value),
                None => {}
            }
        }
        if !names.insert(name.clone()) {
            return Err(DxfImportError::DuplicateSection);
        }
        match name.as_str() {
            "ENTITIES" => sections.entities = true,
            "HEADER" | "TABLES" | "BLOCKS" => {}
            _ => diagnostics.add(
                ImportDiagnosticSeverity::Warning,
                "dxf.section-ignored",
                Some(name),
                1,
            )?,
        }
    }
    if reader.next_pair(&mut value)?.is_some() {
        return Err(DxfImportError::MalformedSections);
    }
    Ok(sections)
}

/// The encoding of `source`'s text, as the profile import decides it for the whole file:
/// UTF-8 (`None`) when all of it is, otherwise the Windows code page its `$DWGCODEPAGE`
/// names when it is older than DXF 2007, with that name.
fn text_encoding(
    source: &mut (impl BufRead + Seek),
) -> Result<Option<(&'static encoding_rs::Encoding, String)>, DxfImportError> {
    source.seek(SeekFrom::Start(0)).map_err(read_error)?;
    let mut line = Vec::new();
    let (mut utf8, mut empty) = (true, true);
    // A header variable's value is the second line after its name.
    let mut acadver = HeaderValue::default();
    let mut code_page = HeaderValue::default();
    while read_line(source, &mut line)? {
        empty = false;
        utf8 &= std::str::from_utf8(&line).is_ok();
        let trimmed = line.trim_ascii();
        acadver.see(trimmed, b"$ACADVER");
        code_page.see(trimmed, b"$DWGCODEPAGE");
    }
    if empty {
        return Err(DxfImportError::Empty);
    }
    if utf8 {
        return Ok(None);
    }
    acadver
        .value
        .is_none_or(|version| version.as_slice() < b"AC1021".as_slice())
        .then_some(code_page.value)
        .flatten()
        .and_then(|name| Some((code_page_encoding(&name)?, code_page_name(&name))))
        .map(Some)
        .ok_or(DxfImportError::NonAscii)
}

/// The value of the first header variable of a name, seen line by line.
#[derive(Default)]
struct HeaderValue {
    /// Lines until the value, once the name was seen.
    until: Option<u8>,
    value: Option<Vec<u8>>,
}

impl HeaderValue {
    fn see(&mut self, line: &[u8], name: &[u8]) {
        match self.until {
            None if self.value.is_none() && line == name => self.until = Some(2),
            Some(1) => {
                self.value = Some(line.to_vec());
                self.until = Some(0);
            }
            Some(left) if left > 1 => self.until = Some(left - 1),
            _ => {}
        }
    }
}

/// Reads group-code/value pairs line by line in the drawing's text encoding.
struct PairReader<'a, R> {
    source: &'a mut R,
    encoding: Option<&'static encoding_rs::Encoding>,
    line: Vec<u8>,
    /// Past the header: in the blocks or entities, whose texts keep their spaces.
    in_drawing: bool,
}

impl<'a, R: BufRead + Seek> PairReader<'a, R> {
    fn new(
        source: &'a mut R,
        encoding: Option<&'static encoding_rs::Encoding>,
    ) -> Result<Self, DxfImportError> {
        source.seek(SeekFrom::Start(0)).map_err(read_error)?;
        Ok(Self {
            source,
            encoding,
            line: Vec::new(),
            in_drawing: false,
        })
    }
}

impl<R: BufRead> PairReader<'_, R> {
    /// The next pair's code, its value in `value`; `None` at the end of the drawing.
    /// Comments (group 999) are passed over. A value is trimmed, but out of the header
    /// the text of a text (groups 1 and 3) is not: its spaces are part of it, and an MTEXT
    /// is cut into pieces wherever 250 characters end, at a space too.
    fn next_pair(&mut self, value: &mut String) -> Result<Option<i16>, DxfImportError> {
        loop {
            if !self.next_line(value)? {
                return Ok(None);
            }
            let code = parse_integer::<i16>(value.trim())?;
            if !self.next_line(value)? {
                return Err(DxfImportError::MalformedPairs);
            }
            let trimmed = value.trim();
            if trimmed.len() != value.len() && !(self.in_drawing && matches!(code, 1 | 3)) {
                *value = trimmed.to_owned();
            }
            if code != 999 {
                return Ok(Some(code));
            }
        }
    }

    fn next_line(&mut self, text: &mut String) -> Result<bool, DxfImportError> {
        if !read_line(self.source, &mut self.line)? {
            return Ok(false);
        }
        text.clear();
        text.push_str(
            &self
                .encoding
                .unwrap_or(encoding_rs::UTF_8)
                .decode_without_bom_handling_and_without_replacement(&self.line)
                .ok_or(DxfImportError::NonAscii)?,
        );
        if text.len() > MAX_DXF_LINE_BYTES {
            return Err(DxfImportError::LineTooLong);
        }
        Ok(true)
    }

    /// Reads on past the start of the section `name`, in a drawing whose sections were
    /// checked.
    fn skip_to_section(&mut self, name: &str) -> Result<(), DxfImportError> {
        let mut value = String::new();
        while let Some(code) = self.next_pair(&mut value)? {
            if code == 0 && value.eq_ignore_ascii_case("SECTION") {
                self.next_pair(&mut value)?;
                if value.eq_ignore_ascii_case(name) {
                    return Ok(());
                }
                loop {
                    match self.next_pair(&mut value)? {
                        Some(0) if value.eq_ignore_ascii_case("ENDSEC") => break,
                        Some(_) => {}
                        None => return Err(DxfImportError::MalformedSections),
                    }
                }
            }
        }
        Err(DxfImportError::MissingEntitiesSection)
    }
}

/// Reads the next line of `source` into `line` without its line break, as `str::lines`
/// splits; `false` at the end. A line longer than any a DXF may hold is refused before it
/// is read on, whatever its text encoding (decoding never shortens it).
fn read_line(source: &mut impl BufRead, line: &mut Vec<u8>) -> Result<bool, DxfImportError> {
    line.clear();
    loop {
        let available = source.fill_buf().map_err(read_error)?;
        if available.is_empty() {
            return Ok(!line.is_empty());
        }
        let (take, ended) = match available.iter().position(|byte| *byte == b'\n') {
            Some(at) => (at + 1, true),
            None => (available.len(), false),
        };
        // The line break, `\r\n`, is not part of the line.
        if line.len() + take > MAX_DXF_LINE_BYTES + 2 {
            return Err(DxfImportError::LineTooLong);
        }
        line.extend_from_slice(&available[..take]);
        source.consume(take);
        if ended {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(true);
        }
    }
}

fn read_error(error: std::io::Error) -> DxfImportError {
    DxfImportError::Read(error.into())
}
