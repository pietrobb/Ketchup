//! A DXF drawing written back with what was changed of it in Kečup as an underlay
//! ([`crate::underlay::edits`]). The source file's own text is kept byte for byte: its
//! header, tables, styles, blocks and every entity not changed. The entities taken out are
//! left out, the moved ones are mapped where they lie (a hatch by its boundary, pattern and
//! seeds; a dimension with a block of its own drawn as it lies), and after the drawing's own come
//! the copies, with handles of their own, and the shapes drawn, as LINE, LWPOLYLINE, ARC,
//! CIRCLE and TEXT on their layers. The source must be the drawing the underlay was read
//! from: it is read again, and its lines must be the underlay's.

use std::collections::BTreeSet;
use std::fmt;
use std::io::Cursor;
use std::ops::Range;

use super::code_page::CodePage;
use super::text::unicode_escapes;
use super::underlay::text_encoding;
use super::{DxfImportError, DxfImportOptions, read_dxf_underlay};
use crate::import::{ImportLengthUnit, ImportUnitAuthority};
use crate::tolerance::NEGLIGIBLE;
use crate::underlay::UnderlayLines;
use crate::underlay::drawn::{AddedObject, DrawnShape, readable_rotation};
use crate::underlay::edits::UnderlayEdits;
use crate::underlay::objects::{Affine, ObjectKind, UnderlayObject, apply};

/// The units a drawing that declares none may have been read in.
const UNITS: [ImportLengthUnit; 9] = [
    ImportLengthUnit::Millimetre,
    ImportLengthUnit::Centimetre,
    ImportLengthUnit::Decimetre,
    ImportLengthUnit::Metre,
    ImportLengthUnit::Kilometre,
    ImportLengthUnit::Inch,
    ImportLengthUnit::Foot,
    ImportLengthUnit::Yard,
    ImportLengthUnit::Mile,
];

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
    /// The source has every entity the underlay was read from, but they read as other
    /// lines: changed in the file since, or read differently by this Kečup than by the
    /// one that imported it (review 2026-10-29 CD-64).
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
                "the file has the entities the underlay was read from, but they read as other \
                 lines now: either they were changed in the file since it was imported, or \
                 this Kečup reads drawings differently from the one that imported it; import \
                 the drawing again to write it with changes",
            ),
            Self::Malformed => formatter
                .write_str("the source drawing's groups do not pair up into sections and entities"),
            Self::TooLarge => formatter.write_str("the source drawing is larger than 4 GiB"),
            Self::NoHandle { object } => write!(
                formatter,
                "object {object} has no handle in the drawing, and its entity cannot be told by \
                 its place among the drawing's entities"
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
        }
    }
}

impl std::error::Error for DxfEditRefusal {}

/// `source`, the drawing `lines` were read from with `options`, written with `edits`. A
/// drawing that declares no unit may have been read in another unit than `options` gives:
/// the one its lines match is taken.
pub fn write_edited_dxf(
    source: &[u8],
    options: DxfImportOptions,
    lines: &UnderlayLines,
    edits: &UnderlayEdits,
) -> Result<Vec<u8>, EditedDxfError> {
    // The read is let go before the drawing is parsed, so the two are not held at once
    // (review 2026-10-29 CD-63).
    let frame = frame_of(source, options, lines)?;
    let encoding = text_encoding(&mut Cursor::new(source))
        .map_err(EditedDxfError::Read)?
        .map(|(encoding, ..)| encoding);
    let drawing = Drawing::parse(source, encoding)?;
    let Some(frame) = frame else {
        // The file is blamed only when its entities are not those the lines were read
        // from (review 2026-10-29 CD-64).
        return Err(if drawing.has_entities_of(&lines.objects().objects) {
            DxfEditRefusal::ReadDifferently
        } else {
            DxfEditRefusal::NotTheSource
        }
        .into());
    };
    drawing.write(lines, edits, &frame)
}

/// How `lines` lie against the units of `source`, when reading it gives them: as `options`
/// read it, or for a drawing that declares no unit, in the unit its lines are scaled by
/// against that read. At most two reads, not one in each unit (review 2026-10-29 CD-63).
fn frame_of(
    source: &[u8],
    options: DxfImportOptions,
    lines: &UnderlayLines,
) -> Result<Option<Frame>, EditedDxfError> {
    let read = read_dxf_underlay(source, options).map_err(EditedDxfError::Read)?;
    let frame = |read: &super::underlay::DxfUnderlay| Frame {
        mm_per_unit: read.units.millimetres_per_unit(),
        offset_mm: read.offset_mm,
    };
    if read.lines == *lines {
        return Ok(Some(frame(&read)));
    }
    if read.units.authority() != ImportUnitAuthority::UserDeclared {
        return Ok(None);
    }
    let Some(unit) = unit_scaled_to(&read, lines) else {
        return Ok(None);
    };
    drop(read);
    let other = read_dxf_underlay(
        source,
        DxfImportOptions::new(Some(unit)).with_geo_origin(options.geo_origin_mm()),
    )
    .map_err(EditedDxfError::Read)?;
    Ok((other.lines == *lines).then(|| frame(&other)))
}

/// The unit of [`UNITS`], other than the one `read` was read in, whose scale against it
/// is nearest to that of `lines`, as the size of their bounds gives it: unlike where they
/// lie, their size does not hang on what they were moved by.
fn unit_scaled_to(
    read: &super::underlay::DxfUnderlay,
    lines: &UnderlayLines,
) -> Option<ImportLengthUnit> {
    let size = |lines: &UnderlayLines| {
        lines
            .bounds_mm()
            .map(|[low, high]| (high[0] - low[0]).max(high[1] - low[1]))
    };
    let (found, wanted) = (size(&read.lines)?, size(lines)?);
    let read_mm = read.units.millimetres_per_unit();
    let mm_per_unit = wanted / found * read_mm;
    if !(mm_per_unit.is_finite() && mm_per_unit > 0.0) {
        return None;
    }
    UNITS
        .into_iter()
        .filter(|unit| unit.millimetres_per_unit() != read_mm)
        .min_by(|a, b| {
            let off =
                |unit: &ImportLengthUnit| (unit.millimetres_per_unit() / mm_per_unit).ln().abs();
            off(a).total_cmp(&off(b))
        })
}

/// How the underlay's millimetres lie against the drawing's own units.
struct Frame {
    mm_per_unit: f64,
    offset_mm: [f64; 2],
}

impl Frame {
    /// The point of the underlay's lines `point` in the drawing's units.
    fn point(&self, point: [f64; 2]) -> [f64; 2] {
        [0, 1].map(|axis| (point[axis] + self.offset_mm[axis]) / self.mm_per_unit)
    }

    /// `map`, of the underlay's lines, as the map of the drawing's units.
    fn map(&self, map: &Affine) -> Affine {
        let [[a, b, x], [c, d, y]] = *map;
        let [ox, oy] = self.offset_mm;
        let k = self.mm_per_unit;
        [
            [a, b, (x + ox - (a * ox + b * oy)) / k],
            [c, d, (y + oy - (c * ox + d * oy)) / k],
        ]
    }
}

/// One group of the source: its code, and where its two lines lie. The places are kept in
/// 32 bits, as a drawing holds millions of pairs (review 2026-10-29 CD-63).
struct Pair {
    code: i32,
    /// The start of its code line.
    start: u32,
    /// Its value line, without the line end.
    value: [u32; 2],
    /// Past its value line's end.
    end: u32,
}

impl Pair {
    fn start(&self) -> usize {
        place(self.start)
    }

    fn value(&self) -> Range<usize> {
        place(self.value[0])..place(self.value[1])
    }

    fn end(&self) -> usize {
        place(self.end)
    }
}

/// A place kept in 32 bits, as the `usize` it was made from.
fn place(at: u32) -> usize {
    usize::try_from(at).unwrap_or(usize::MAX)
}

/// An entity of the ENTITIES section with the vertices or attributes that follow it: the
/// range of its pairs, and where each of its records starts with that record's name.
struct Entity {
    pairs: Range<usize>,
    records: Vec<(usize, String)>,
    handle: Option<u64>,
}

/// The parts of the source that are written anew.
struct Drawing<'a> {
    source: &'a [u8],
    eol: &'static [u8],
    pairs: Vec<Pair>,
    entities: Vec<Entity>,
    /// The pair `ENDSEC` closing the ENTITIES section.
    entities_end: usize,
    /// The value pair of `$HANDSEED`.
    handseed: Option<usize>,
    /// The handle after every handle of the drawing.
    next_handle: Option<u64>,
    /// Whether it is DXF 13 or later: entities carry subclass markers and their owner.
    subclassed: bool,
    /// Whether its texts are UTF-8: DXF 2007 or later.
    utf8: bool,
    /// The owner (group 330) of the entities of model space.
    owner: Option<Vec<u8>>,
    /// Each layer name as read, with the bytes the drawing writes it in.
    layers: Vec<(String, Vec<u8>)>,
    /// The blocks of the BLOCKS section.
    blocks: Vec<BlockAt>,
    /// The pair `ENDSEC` closing the BLOCKS section.
    blocks_end: Option<usize>,
    /// The pair `ENDTAB` closing the BLOCK_RECORD table, with the table's handle.
    block_records: Option<(usize, Option<Vec<u8>>)>,
    /// The number after the highest of the drawing's `*D<n>` blocks.
    next_dimension_block: u64,
    /// Where each record of the OBJECTS section starts, and the pair `ENDSEC` closing it.
    objects: Vec<usize>,
    objects_end: Option<usize>,
}

/// A block of the BLOCKS section: the pair `BLOCK` it starts at, its name upper case, the
/// handle of its block record (group 330), and its pair `ENDBLK`.
struct BlockAt {
    start: usize,
    name: String,
    owner: Option<Vec<u8>>,
    endblk: Option<usize>,
}

/// What mapped dimensions add besides their own entity: their drawn blocks, and the
/// records of those blocks.
struct NewBlocks {
    blocks: Vec<u8>,
    records: Vec<u8>,
    next_dimension: u64,
}

impl<'a> Drawing<'a> {
    fn parse(source: &'a [u8], encoding: Option<CodePage>) -> Result<Self, EditedDxfError> {
        let pairs = pairs(source)?;
        let eol: &'static [u8] = if source
            .iter()
            .position(|byte| *byte == b'\n')
            .is_some_and(|end| end > 0 && source[end - 1] == b'\r')
        {
            b"\r\n"
        } else {
            b"\n"
        };
        let value = |pair: &Pair| source[pair.value()].trim_ascii();
        let name = |pair: &Pair| String::from_utf8_lossy(value(pair)).to_ascii_uppercase();
        let decode = |bytes: &[u8]| {
            let text = encoding
                .unwrap_or(CodePage::Standard(encoding_rs::UTF_8))
                .decode(bytes)?;
            Some(unicode_escapes(text.trim()).into_owned())
        };
        let mut drawing = Self {
            source,
            eol,
            pairs: Vec::new(),
            entities: Vec::new(),
            entities_end: 0,
            handseed: None,
            next_handle: None,
            subclassed: false,
            utf8: false,
            owner: None,
            layers: Vec::new(),
            blocks: Vec::new(),
            blocks_end: None,
            block_records: None,
            next_dimension_block: 1,
            objects: Vec::new(),
            objects_end: None,
        };
        let mut section = String::new();
        let mut record = String::new();
        let mut table = (String::new(), None::<Vec<u8>>);
        let mut max_handle = None::<u64>;
        let mut version = Vec::new();
        let mut index = 0;
        while index < pairs.len() {
            let pair = &pairs[index];
            match pair.code {
                0 if name(pair) == "SECTION" => {
                    let next = pairs.get(index + 1).ok_or(DxfEditRefusal::Malformed)?;
                    section = name(next);
                    index += 2;
                    if section == "ENTITIES" {
                        index = drawing.parse_entities(&pairs, index)?;
                    }
                    continue;
                }
                0 => {
                    record = name(pair);
                    match (section.as_str(), record.as_str()) {
                        ("BLOCKS", "BLOCK") => drawing.blocks.push(BlockAt {
                            start: index,
                            name: String::new(),
                            owner: None,
                            endblk: None,
                        }),
                        ("BLOCKS", "ENDBLK") => {
                            if let Some(block) = drawing.blocks.last_mut() {
                                block.endblk = Some(index);
                            }
                        }
                        ("BLOCKS", "ENDSEC") => drawing.blocks_end = Some(index),
                        ("OBJECTS", "ENDSEC") => drawing.objects_end = Some(index),
                        // Not the `EOF` past it: the last record would take in `ENDSEC`.
                        ("OBJECTS", _) if drawing.objects_end.is_none() => {
                            drawing.objects.push(index);
                        }
                        ("TABLES", "ENDTAB") if table.0 == "BLOCK_RECORD" => {
                            drawing.block_records = Some((index, table.1.take()));
                        }
                        _ => {}
                    }
                }
                2 | 5 if section == "TABLES" && record == "TABLE" => {
                    if pair.code == 2 {
                        table = (name(pair), None);
                    } else {
                        table.1 = Some(value(pair).to_vec());
                    }
                }
                2 | 330 if section == "BLOCKS" && record == "BLOCK" => {
                    if let Some(block) = drawing.blocks.last_mut() {
                        if pair.code == 2 && block.name.is_empty() {
                            block.name = name(pair);
                        } else if pair.code == 330 && block.owner.is_none() {
                            block.owner = Some(value(pair).to_vec());
                        }
                    }
                }
                9 if section == "HEADER" => {
                    let next = pairs.get(index + 1).ok_or(DxfEditRefusal::Malformed)?;
                    match name(pair).as_str() {
                        "$HANDSEED" => drawing.handseed = Some(index + 1),
                        "$ACADVER" => version = value(next).to_ascii_uppercase(),
                        _ => {}
                    }
                }
                2 if section == "TABLES" && record == "LAYER" => {
                    if let Some(name) = decode(value(pair)) {
                        drawing.layers.push((name, value(pair).to_vec()));
                    }
                }
                _ => {}
            }
            if matches!(pair.code, 5 | 105)
                && let Some(handle) = hex(value(pair))
            {
                max_handle = max_handle.max(Some(handle));
            }
            index += 1;
        }
        if drawing.entities_end == 0 {
            return Err(DxfEditRefusal::Malformed.into());
        }
        for entity in &drawing.entities {
            // Past the handles of its VERTEX, ATTRIB and SEQEND too, not only its own.
            for pair in &pairs[entity.pairs.clone()] {
                if matches!(pair.code, 5 | 105)
                    && let Some(handle) = hex(value(pair))
                {
                    max_handle = max_handle.max(Some(handle));
                }
                if pair.code == 8
                    && let Some(name) = decode(value(pair))
                    && !drawing.layers.iter().any(|(known, _)| *known == name)
                {
                    drawing.layers.push((name, value(pair).to_vec()));
                }
            }
        }
        let seed = drawing.handseed.and_then(|index| hex(value(&pairs[index])));
        drawing.next_handle = match (seed, max_handle) {
            (None, None) => None,
            (seed, max) => Some(seed.unwrap_or(0).max(max.map_or(1, |max| max + 1))),
        };
        drawing.next_dimension_block = drawing
            .blocks
            .iter()
            .filter_map(|block| block.name.strip_prefix("*D")?.parse::<u64>().ok())
            .max()
            .map_or(1, |last| last.saturating_add(1));
        drawing.subclassed = version.as_slice() >= b"AC1012".as_slice();
        drawing.utf8 = version.as_slice() >= b"AC1021".as_slice();
        drawing.owner = drawing.entities.iter().find_map(|entity| {
            let record =
                &pairs[entity.pairs.start..entity.records.get(1).map_or(entity.pairs.end, |r| r.0)];
            let paper = record
                .iter()
                .any(|pair| pair.code == 67 && value(pair) == b"1");
            (!paper)
                .then(|| record.iter().find(|pair| pair.code == 330))
                .flatten()
                .map(|pair| value(pair).to_vec())
        });
        drawing.pairs = pairs;
        Ok(drawing)
    }

    /// Reads the entities from pair `index` on, past the section's start, to its end;
    /// returns the index past its `ENDSEC`.
    fn parse_entities(
        &mut self,
        pairs: &[Pair],
        mut index: usize,
    ) -> Result<usize, EditedDxfError> {
        let value = |pair: &Pair| self.source[pair.value()].trim_ascii();
        let name = |pair: &Pair| String::from_utf8_lossy(value(pair)).to_ascii_uppercase();
        let mut current: Option<Entity> = None;
        while index < pairs.len() {
            let pair = &pairs[index];
            if pair.code != 0 {
                if current.is_none() {
                    return Err(DxfEditRefusal::Malformed.into());
                }
                if pair.code == 5
                    && let Some(entity) = current.as_mut()
                    && entity.records.len() == 1
                    && entity.handle.is_none()
                {
                    entity.handle = hex(value(pair));
                }
                index += 1;
                continue;
            }
            let record = name(pair);
            let follows = current.as_ref().is_some_and(|entity| {
                let primary = entity.records[0].1.as_str();
                matches!(record.as_str(), "VERTEX" | "SEQEND")
                    || (record == "ATTRIB" && primary == "INSERT")
            });
            if follows && let Some(entity) = current.as_mut() {
                entity.records.push((index, record));
                index += 1;
                continue;
            }
            if let Some(mut entity) = current.take() {
                entity.pairs.end = index;
                self.entities.push(entity);
            }
            if record == "ENDSEC" {
                self.entities_end = index;
                return Ok(index + 1);
            }
            current = Some(Entity {
                pairs: index..index,
                records: vec![(index, record)],
                handle: None,
            });
            index += 1;
        }
        Err(DxfEditRefusal::Malformed.into())
    }

    /// Whether it has the entity of each top-level object of `objects`: one of its handle
    /// and kind, or for a drawing without handles one in its place
    /// ([`Self::entities_by_order`]).
    fn has_entities_of(&self, objects: &[UnderlayObject]) -> bool {
        let mut tops = objects
            .iter()
            .filter(|object| object.parent.is_none())
            .peekable();
        if tops.peek().is_none() {
            return false;
        }
        if tops.clone().all(|object| object.handle == 0) {
            return self.entities_by_order(objects).is_some();
        }
        tops.filter(|object| object.handle != 0).all(|object| {
            let mut found = self
                .entities
                .iter()
                .filter(|entity| entity.handle == Some(object.handle));
            matches!(
                (found.next(), found.next()),
                (Some(entity), None) if entity.records[0].1 == object.kind.name()
            )
        })
    }

    /// For a drawing without handles (DXF R12, Kečup's own sheets), the entity of each
    /// top-level object of `objects` by their order: the n-th is the n-th entity of model
    /// space of a kind objects are kept of. `None` unless both are without handles and each
    /// object has the kind of its entity, one for one (review 2026-10-29 CD-66).
    fn entities_by_order(&self, objects: &[UnderlayObject]) -> Option<Vec<Option<usize>>> {
        if self.entities.iter().any(|entity| entity.handle.is_some()) {
            return None;
        }
        let kinds = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| {
                let head = entity
                    .records
                    .get(1)
                    .map_or(entity.pairs.end, |record| record.0);
                let paper = self.pairs[entity.pairs.start..head]
                    .iter()
                    .any(|pair| pair.code == 67 && self.source[pair.value()].trim_ascii() == b"1");
                !paper && ObjectKind::from_dxf(&entity.records[0].1).is_some()
            })
            .map(|(index, entity)| (index, entity.records[0].1.as_str()));
        let mut entities = vec![None; objects.len()];
        let mut kinds = kinds.fuse();
        for (index, object) in objects.iter().enumerate() {
            if object.parent.is_some() {
                continue;
            }
            if object.handle != 0 {
                return None;
            }
            let (entity, kind) = kinds.next()?;
            if kind != object.kind.name() {
                return None;
            }
            entities[index] = Some(entity);
        }
        kinds.next().is_none().then_some(entities)
    }

    fn value(&self, index: usize) -> &[u8] {
        self.source[self.pairs[index].value()].trim_ascii()
    }

    fn number(&self, index: usize) -> Result<f64, EditedDxfError> {
        std::str::from_utf8(self.value(index))
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .ok_or(DxfEditRefusal::Malformed.into())
    }

    /// The value of an integer group (a flag, a count, a kind), as the import read it.
    fn integer(&self, index: usize) -> Result<i64, EditedDxfError> {
        std::str::from_utf8(self.value(index))
            .ok()
            .and_then(|text| text.parse::<i64>().ok())
            .ok_or(DxfEditRefusal::Malformed.into())
    }

    fn write(
        &self,
        lines: &UnderlayLines,
        edits: &UnderlayEdits,
        frame: &Frame,
    ) -> Result<Vec<u8>, EditedDxfError> {
        let objects = &lines.objects().objects;
        let read_count = u32::try_from(objects.len()).unwrap_or(u32::MAX);
        let by_order = self.entities_by_order(objects);
        let entity_of = |object: u32| -> Result<usize, EditedDxfError> {
            let index = usize::try_from(object).unwrap_or(usize::MAX);
            let read = objects
                .get(index)
                .filter(|read| read.parent.is_none())
                .ok_or(DxfEditRefusal::NoHandle { object })?;
            if read.handle == 0 {
                return by_order
                    .as_ref()
                    .and_then(|entities| entities.get(index).copied().flatten())
                    .ok_or(DxfEditRefusal::NoHandle { object }.into());
            }
            let mut found = self
                .entities
                .iter()
                .enumerate()
                .filter(|(_, entity)| entity.handle == Some(read.handle));
            match (found.next(), found.next()) {
                (Some((index, _)), None) => Ok(index),
                _ => Err(DxfEditRefusal::HandleMissing {
                    object,
                    handle: read.handle,
                }
                .into()),
            }
        };
        // Per entity of the drawing: left out, mapped, or kept.
        let mut replaced = vec![None::<Option<Vec<u8>>>; self.entities.len()];
        for &object in edits.removed_objects.iter().filter(|id| **id < read_count) {
            replaced[entity_of(object)?] = Some(None);
        }
        let removed: BTreeSet<u64> = self
            .entities
            .iter()
            .zip(&replaced)
            .filter(|(_, replaced)| matches!(replaced, Some(None)))
            .flat_map(|(entity, _)| entity.pairs.clone())
            .filter(|&index| self.pairs[index].code == 5)
            .filter_map(|index| hex(self.value(index)))
            .collect();
        let (objects_edits, removed) = self.objects_without(removed);
        // The entities kept as they are, without what they named of the removed.
        for (entity, replaced) in self.entities.iter().zip(&mut replaced) {
            if replaced.is_none() {
                *replaced = self.without_removed(entity, &removed).map(Some);
            }
        }
        let mut handles = self.next_handle;
        let mut blocks = NewBlocks {
            blocks: Vec::new(),
            records: Vec::new(),
            next_dimension: self.next_dimension_block,
        };
        let moved = edits.moved_objects.iter().map(|moved| (moved, false));
        let stretched = edits.stretched_objects.iter().map(|moved| (moved, true));
        for ((&object, map), stretch) in moved.chain(stretched) {
            if edits.removed_objects.contains(&object) {
                continue;
            }
            let entity = entity_of(object)?;
            let map = Mapping {
                map: frame.map(map),
                stretch,
                removed: &removed,
                vertices_minus_z: false,
            };
            let mapped = self.mapped(object, entity, &map, false, &mut handles, &mut blocks)?;
            replaced[entity] = Some(Some(mapped));
        }
        // How many objects each object of the drawing is with all it placed: a copy of it
        // takes as many ids.
        let mut tree = vec![1_u32; objects.len()];
        for (index, object) in objects.iter().enumerate().rev() {
            if let Some(parent) = object
                .parent
                .and_then(|parent| usize::try_from(parent).ok())
                && parent < index
            {
                tree[parent] = tree[parent].saturating_add(tree[index]);
            }
        }
        let mut added = Vec::new();
        let mut id = read_count;
        for edit in &edits.added {
            let first = id;
            id = id.saturating_add(
                edit.copy_of()
                    .and_then(|of| usize::try_from(of).ok())
                    .and_then(|of| tree.get(of).copied())
                    .unwrap_or(1),
            );
            if edits.removed_objects.contains(&first) {
                continue;
            }
            match edit {
                AddedObject::Copy { of, map } | AddedObject::Stretched { of, map } => {
                    let entity = entity_of(*of)?;
                    let map = Mapping {
                        map: frame.map(map),
                        stretch: matches!(edit, AddedObject::Stretched { .. }),
                        removed: &removed,
                        vertices_minus_z: false,
                    };
                    added.extend(self.mapped(
                        *of,
                        entity,
                        &map,
                        true,
                        &mut handles,
                        &mut blocks,
                    )?);
                }
                AddedObject::Drawn { layer, shape } => {
                    added.extend(self.drawn(layer, shape, frame, &mut handles));
                }
            }
        }
        let mut out = Vec::with_capacity(self.source.len() + added.len());
        let mut copied = 0;
        let mut splice = |out: &mut Vec<u8>, range: Range<usize>, with: &[u8]| {
            out.extend_from_slice(&self.source[copied..range.start]);
            out.extend_from_slice(with);
            copied = range.end;
        };
        let mut edits_at = Vec::new();
        if let (Some(index), Some(next)) = (self.handseed, handles)
            && handles != self.next_handle
        {
            let mut value = format!("{next:X}").into_bytes();
            value.extend_from_slice(self.eol);
            let pair = &self.pairs[index];
            edits_at.push((pair.value().start..pair.end(), value));
        }
        edits_at.extend(objects_edits);
        for (entity, replaced) in self.entities.iter().zip(replaced) {
            if let Some(replaced) = replaced {
                let range =
                    self.pairs[entity.pairs.start].start()..self.pairs[entity.pairs.end - 1].end();
                edits_at.push((range, replaced.unwrap_or_default()));
            }
        }
        let end = self.pairs[self.entities_end].start();
        edits_at.push((end..end, added));
        // A dimension's block is found only in the BLOCKS section, so when blocks were
        // added there is one.
        if let Some(index) = self.blocks_end
            && !blocks.blocks.is_empty()
        {
            let at = self.pairs[index].start();
            edits_at.push((at..at, blocks.blocks));
        }
        if let Some((index, _)) = self.block_records
            && !blocks.records.is_empty()
        {
            let at = self.pairs[index].start();
            edits_at.push((at..at, blocks.records));
        }
        edits_at.sort_by_key(|(range, _)| range.start);
        for (range, with) in edits_at {
            splice(&mut out, range, &with);
        }
        out.extend_from_slice(&self.source[copied..]);
        Ok(out)
    }

    /// The OBJECTS section rewritten without the records `removed` (by handle) owned, as
    /// a removed entity's extension dictionary, and without every reference to them: a
    /// group's member, a reactor, a dictionary entry with its key, a draw-order entry with
    /// its sort handle (review 2026-10-29 CD-60). A dimension association measuring a
    /// removed entity goes too (CD-68). Each item replaces a byte range; the handles
    /// removed come back with those of the records left out.
    fn objects_without(&self, mut removed: BTreeSet<u64>) -> (ObjectEdits, BTreeSet<u64>) {
        let Some(end) = self.objects_end else {
            return (Vec::new(), removed);
        };
        if removed.is_empty() {
            return (Vec::new(), removed);
        }
        let records: Vec<Range<usize>> = self
            .objects
            .iter()
            .enumerate()
            .map(|(at, &start)| start..self.objects.get(at + 1).copied().unwrap_or(end))
            .collect();
        let handle_of = |record: &Range<usize>, code: i32| -> Option<u64> {
            let mut in_group = false;
            record.clone().find_map(|index| {
                let pair = &self.pairs[index];
                if pair.code == 102 {
                    in_group = self.value(index).starts_with(b"{");
                }
                (!in_group && pair.code == code)
                    .then(|| hex(self.value(index)))
                    .flatten()
            })
        };
        let mut dropped = vec![false; records.len()];
        loop {
            let mut changed = false;
            for (record, dropped) in records.iter().zip(&mut dropped) {
                let measures_removed = || {
                    self.value(record.start).eq_ignore_ascii_case(b"DIMASSOC")
                        && record.clone().any(|index| {
                            self.pairs[index].code == 331
                                && hex(self.value(index)).is_some_and(|h| removed.contains(&h))
                        })
                };
                if !*dropped
                    && (handle_of(record, 330).is_some_and(|owner| removed.contains(&owner))
                        || measures_removed())
                {
                    *dropped = true;
                    changed = true;
                    removed.extend(handle_of(record, 5));
                }
            }
            if !changed {
                break;
            }
        }
        let mut edits = Vec::new();
        for (record, dropped) in records.iter().zip(dropped) {
            let bytes = self.pairs[record.start].start()..self.pairs[record.end - 1].end();
            if dropped {
                edits.push((bytes, Vec::new()));
                continue;
            }
            let mut skip = vec![false; record.len()];
            for (at, index) in record.clone().enumerate() {
                let code = self.pairs[index].code;
                if !(330..=369).contains(&code)
                    || !hex(self.value(index)).is_some_and(|handle| removed.contains(&handle))
                {
                    continue;
                }
                skip[at] = true;
                let previous = index.checked_sub(1).filter(|i| *i > record.start);
                if let Some(previous) = previous
                    && matches!(code, 350 | 360)
                    && self.pairs[previous].code == 3
                {
                    skip[previous - record.start] = true;
                }
                if code == 331 && self.pairs.get(index + 1).is_some_and(|next| next.code == 5) {
                    skip[at + 1] = true;
                }
            }
            if skip.iter().any(|skip| *skip) {
                let mut kept = Vec::new();
                for (index, skip) in record.clone().zip(skip) {
                    if !skip {
                        let pair = &self.pairs[index];
                        kept.extend_from_slice(&self.source[pair.start()..pair.end()]);
                    }
                }
                edits.push((bytes, kept));
            }
        }
        (edits, removed)
    }

    /// The entity `entity`, of object `object`, mapped by `map`, of the drawing's units;
    /// with `copy` a copy, with handles of its own from `handles`. A dimension draws a
    /// block mapped as it is, added to `blocks`.
    fn mapped(
        &self,
        object: u32,
        entity: usize,
        map: &Mapping,
        copy: bool,
        handles: &mut Option<u64>,
        blocks: &mut NewBlocks,
    ) -> Result<Vec<u8>, EditedDxfError> {
        let entity = &self.entities[entity];
        let mut out = Vec::new();
        let primary = if copy { take_handle(handles) } else { None };
        let mut map = *map;
        for (index, (start, record)) in entity.records.iter().enumerate() {
            let end = entity
                .records
                .get(index + 1)
                .map_or(entity.pairs.end, |next| next.0);
            if index == 0 && record == "POLYLINE" {
                map.vertices_minus_z = self.extrusion_z(*start..end)?.is_some_and(|z| z < 0.0);
            }
            let own = match (copy, index) {
                (false, _) => None,
                (true, 0) => primary,
                (true, _) => take_handle(handles),
            };
            let mut mapped = self.mapped_record(object, record, *start..end, &map)?;
            self.drop_references(record, &mut mapped, map.removed);
            if record == "DIMENSION" {
                self.dimension_block(object, &mut mapped, &map.map, handles, blocks)?;
            }
            let copy = copy.then_some((own, primary, entity.handle));
            self.write_record(&mut out, &mapped, copy);
        }
        Ok(out)
    }

    /// The entity `entity` as read without what it names of the `removed` handles, if it
    /// names any (review 2026-10-29 CD-68).
    fn without_removed(&self, entity: &Entity, removed: &BTreeSet<u64>) -> Option<Vec<u8>> {
        let names = entity.pairs.clone().any(|index| {
            (330..=369).contains(&self.pairs[index].code)
                && hex(self.value(index)).is_some_and(|handle| removed.contains(&handle))
        });
        if !names {
            return None;
        }
        let mut out = Vec::new();
        for (index, (start, record)) in entity.records.iter().enumerate() {
            let end = entity
                .records
                .get(index + 1)
                .map_or(entity.pairs.end, |next| next.0);
            let mut pairs = (*start..end).map(Out::Raw).collect::<Vec<_>>();
            self.drop_references(record, &mut pairs, removed);
            self.write_record(&mut out, &pairs, None);
        }
        Some(out)
    }

    /// Leaves out of `record` (one record of kind `kind`) each reference to the `removed`
    /// handles, as a reactor, and a reactor group left empty. A hatch bounded by a removed
    /// entity keeps its fill: its count of boundary objects goes down, and with none left
    /// it is no longer associative (review 2026-10-29 CD-68).
    fn drop_references(&self, kind: &str, record: &mut [Out], removed: &BTreeSet<u64>) {
        let hatch = kind == "HATCH";
        let mut sources = None::<(usize, usize, u64)>;
        let mut associative = None::<(usize, usize)>;
        let (mut dropped, mut kept) = (false, false);
        let mut group = None::<usize>;
        for at in 0..record.len() {
            let index = match &record[at] {
                Out::Raw(index) | Out::Set(index, _) => *index,
                Out::New(..) | Out::Drop => continue,
            };
            let code = self.pairs[index].code;
            let value = self.value(index);
            match code {
                102 if value.starts_with(b"{") => group = Some(at),
                102 => {
                    // A group of which every member was left out goes with them.
                    if let Some(start) = group.take()
                        && record[start + 1..at]
                            .iter()
                            .all(|out| matches!(out, Out::Drop))
                        && at > start + 1
                    {
                        record[start] = Out::Drop;
                        record[at] = Out::Drop;
                    }
                }
                71 if hatch => associative = Some((at, index)),
                97 if hatch => {
                    let count = std::str::from_utf8(value)
                        .ok()
                        .and_then(|count| count.trim().parse().ok());
                    sources = count.map(|count| (at, index, count));
                }
                _ => {}
            }
            if !(330..=369).contains(&code) {
                continue;
            }
            let Some(handle) = hex(value) else {
                continue;
            };
            let source = hatch && code == 330 && sources.is_some();
            if !removed.contains(&handle) {
                kept |= source;
                continue;
            }
            record[at] = Out::Drop;
            dropped |= source;
            if source && let Some((at, index, count)) = sources.as_mut() {
                *count = count.saturating_sub(1);
                record[*at] = Out::Set(*index, count.to_string());
            }
        }
        if dropped
            && !kept
            && let Some((at, index)) = associative
        {
            record[at] = Out::Set(index, "0".to_owned());
        }
    }

    /// The block the dimension `record` draws, mapped by `map` as a new block of its own
    /// added to `blocks`, which `record` then names: a block name says where it is drawn,
    /// so neither a moved nor a copied dimension can keep the one it was read with.
    fn dimension_block(
        &self,
        object: u32,
        record: &mut [Out],
        map: &Affine,
        handles: &mut Option<u64>,
        blocks: &mut NewBlocks,
    ) -> Result<(), EditedDxfError> {
        let not_mappable = || DxfEditRefusal::NotMappable {
            object,
            kind: "DIMENSION".to_owned(),
        };
        let (at, index) = record
            .iter()
            .enumerate()
            .find_map(|(at, out)| match out {
                Out::Raw(index) if self.pairs[*index].code == 2 => Some((at, *index)),
                _ => None,
            })
            .ok_or_else(not_mappable)?;
        let wanted = String::from_utf8_lossy(self.value(index)).to_ascii_uppercase();
        let block = self
            .blocks
            .iter()
            .find(|block| block.name == wanted)
            .ok_or_else(not_mappable)?;
        let endblk = block.endblk.ok_or(DxfEditRefusal::Malformed)?;
        let next_record = |from: usize| {
            (from + 1..self.pairs.len())
                .find(|index| self.pairs[*index].code == 0)
                .ok_or(DxfEditRefusal::Malformed)
        };
        let name = format!("*D{}", blocks.next_dimension);
        blocks.next_dimension = blocks.next_dimension.saturating_add(1);
        let old_owner = block.owner.as_deref().and_then(hex);
        // Its block record, in the drawings that have them.
        let new_owner = match &self.block_records {
            Some(_) if self.subclassed => take_handle(handles),
            _ => None,
        };
        if let (Some(handle), Some((_, table))) = (new_owner, &self.block_records) {
            let out = &mut blocks.records;
            self.push_pair(out, 0, b"BLOCK_RECORD");
            self.push_pair(out, 5, format!("{handle:X}").as_bytes());
            if let Some(table) = table {
                self.push_pair(out, 330, table);
            }
            self.push_pair(out, 100, b"AcDbSymbolTableRecord");
            self.push_pair(out, 100, b"AcDbBlockTableRecord");
            self.push_pair(out, 2, name.as_bytes());
        }
        let copy = |handles: &mut Option<u64>| Some((take_handle(handles), new_owner, old_owner));
        // The BLOCK, named anew; its entities mapped; its ENDBLK.
        let header = block.start..next_record(block.start)?;
        let named = header
            .clone()
            .map(|index| match self.pairs[index].code {
                2 | 3 => Out::Set(index, name.clone()),
                _ => Out::Raw(index),
            })
            .collect::<Vec<_>>();
        self.write_record(&mut blocks.blocks, &named, copy(handles));
        let mut start = header.end;
        let mut mapping = Mapping {
            map: *map,
            stretch: false,
            removed: &BTreeSet::new(),
            vertices_minus_z: false,
        };
        while start < endblk {
            let end = next_record(start)?;
            let kind = String::from_utf8_lossy(self.value(start)).to_ascii_uppercase();
            if kind == "POLYLINE" {
                mapping.vertices_minus_z = self.extrusion_z(start..end)?.is_some_and(|z| z < 0.0);
            }
            let mapped = self.mapped_record(object, &kind, start..end, &mapping)?;
            self.write_record(&mut blocks.blocks, &mapped, copy(handles));
            start = end;
        }
        let tail = (endblk..next_record(endblk)?)
            .map(Out::Raw)
            .collect::<Vec<_>>();
        self.write_record(&mut blocks.blocks, &tail, copy(handles));
        record[at] = Out::Set(index, name);
        Ok(())
    }

    /// Writes the pair of `code` and `value` to `out`.
    fn push_pair(&self, out: &mut Vec<u8>, code: i32, value: &[u8]) {
        out.extend_from_slice(format!("{code:>3}").as_bytes());
        out.extend_from_slice(self.eol);
        out.extend_from_slice(value);
        out.extend_from_slice(self.eol);
    }

    /// The Z of the extrusion of the record of pairs `pairs`, if it has one, as an entity
    /// flat in the plan has: ±1 (review 2026-10-29 CD-69).
    fn extrusion_z(&self, pairs: Range<usize>) -> Result<Option<f64>, EditedDxfError> {
        let mut z = None;
        for index in pairs.take_while(|index| self.pairs[*index].code < 1000) {
            match self.pairs[index].code {
                210 | 220 if self.number(index)?.abs() > NEGLIGIBLE => {
                    return Ok(Some(f64::NAN));
                }
                230 => z = Some(self.number(index)?),
                _ => {}
            }
        }
        Ok(z)
    }

    /// Whether the record `record` of pairs `pairs` has its points in an object coordinate
    /// system of extrusion −Z, where x is the world's −x; a VERTEX has that of its
    /// POLYLINE, `vertices_minus_z`. An entity whose points are the world's has none; one
    /// of another extrusion is refused, as the import refuses it.
    fn in_mirrored_ocs(
        &self,
        object: u32,
        record: &str,
        pairs: Range<usize>,
        vertices_minus_z: bool,
    ) -> Result<bool, EditedDxfError> {
        let not_mappable = || DxfEditRefusal::NotMappable {
            object,
            kind: record.to_owned(),
        };
        let minus_z = match self.extrusion_z(pairs)? {
            None => record == "VERTEX" && vertices_minus_z,
            Some(z) if (z - 1.0).abs() <= NEGLIGIBLE => false,
            Some(z) if (z + 1.0).abs() <= NEGLIGIBLE => true,
            Some(_) => return Err(not_mappable().into()),
        };
        match record {
            _ if !minus_z => Ok(false),
            "LINE" | "POINT" | "ELLIPSE" | "SPLINE" | "MTEXT" => Ok(false),
            "CIRCLE" | "ARC" | "LWPOLYLINE" | "POLYLINE" | "VERTEX" | "TEXT" | "ATTRIB"
            | "ATTDEF" | "INSERT" => Ok(true),
            _ => Err(not_mappable().into()),
        }
    }

    /// The pairs of one record mapped by `map`.
    fn mapped_record(
        &self,
        object: u32,
        record: &str,
        pairs: Range<usize>,
        mapping: &Mapping,
    ) -> Result<Vec<Out>, EditedDxfError> {
        let not_mappable = || DxfEditRefusal::NotMappable {
            object,
            kind: record.to_owned(),
        };
        let rules = Rules::of(record).ok_or_else(not_mappable)?;
        if mapping.stretch && !STRETCHED_RECORDS.contains(&record) {
            return Err(not_mappable().into());
        }
        let ocs = self.in_mirrored_ocs(object, record, pairs.clone(), mapping.vertices_minus_z)?;
        let flipped;
        let map = if ocs {
            // Mapped in the object coordinate system: its x is the world's −x.
            let [[a, b, x], [c, d, y]] = mapping.map;
            flipped = [[a, 0.0 - b, 0.0 - x], [0.0 - c, d, y]];
            &flipped
        } else {
            &mapping.map
        };
        // A text reads from left to right in the world, not in its own system.
        let readable = |rotation: f64| {
            use std::f64::consts::PI;
            if ocs {
                PI - readable_rotation(&mapping.map, PI - rotation)
            } else {
                readable_rotation(map, rotation)
            }
        };
        let [[a, b, _], [c, d, _]] = *map;
        let scale = (a * d - b * c).abs().sqrt();
        // A stretched line keeps its widths.
        let length_scale = if mapping.stretch { 1.0 } else { scale };
        let mirrored = a * d - b * c < 0.0;
        let turn_deg = c.atan2(a).to_degrees();
        let indices = pairs.clone().collect::<Vec<_>>();
        // Groups of extended data are the application's own.
        let own = indices
            .iter()
            .position(|index| self.pairs[*index].code >= 1000)
            .map_or(&indices[..], |end| &indices[..end]);
        let code_of = |index: usize| self.pairs[index].code;
        let number_of = |code: i32| -> Result<Option<f64>, EditedDxfError> {
            own.iter()
                .find(|index| code_of(**index) == code)
                .map(|index| self.number(*index))
                .transpose()
        };
        let dimension_kind = || -> Result<i64, EditedDxfError> {
            own.iter()
                .find(|index| code_of(**index) == 70)
                .map_or(Ok(0), |index| self.integer(*index))
        };
        if rules.insert && mirrored {
            let many = |code| number_of(code).map(|count| count.is_some_and(|count| count > 1.0));
            if many(70)? || many(71)? {
                return Err(not_mappable().into());
            }
        }
        let mut out = indices
            .iter()
            .map(|index| Out::Raw(*index))
            .collect::<Vec<_>>();
        let position = |code: i32| own.iter().position(|index| code_of(*index) == code);
        let mut skip = 0;
        for (at, &index) in own.iter().enumerate() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let code = code_of(index);
            let next = own
                .get(at + 1)
                .copied()
                .filter(|next| code_of(*next) == code + 10);
            if rules.points.contains(&code) || rules.vectors.contains(&code) {
                let next = next.ok_or(DxfEditRefusal::Malformed)?;
                let point = [self.number(index)?, self.number(next)?];
                let [x, y] = if rules.points.contains(&code) {
                    apply(map, point)
                } else {
                    [a * point[0] + b * point[1], c * point[0] + d * point[1]]
                };
                let [x, y] = if rules.direction == Some(code) {
                    let turned = readable(point[1].atan2(point[0]));
                    let length = x.hypot(y);
                    [length * turned.cos(), length * turned.sin()]
                } else {
                    [x, y]
                };
                out[at] = Out::Set(index, number(x));
                out[at + 1] = Out::Set(next, number(y));
                skip = 1;
            } else if rules.lengths.contains(&code) {
                out[at] = Out::Set(index, number(self.number(index)? * length_scale));
            } else if code == 42 && rules.bulge && mirrored {
                out[at] = Out::Set(index, number(0.0 - self.number(index)?));
            } else if rules.angles.contains(&code) {
                let angle = turned(self.number(index)?, turn_deg, mirrored);
                out[at] = Out::Set(index, number(angle));
            }
        }
        if rules.dimension
            && let Some(at) = position(42)
            && !matches!(dimension_kind()? & 7, 2 | 5)
        {
            out[at] = Out::Set(own[at], number(self.number(own[at])? * scale));
        }
        if rules.hatch {
            self.map_hatch(own, &mut out, map, turn_deg, mirrored, scale)?;
        }
        if rules.arc
            && let (Some(start), Some(end)) = (position(50), position(51))
        {
            let [from, to] = [start, end].map(|at| self.number(own[at]));
            let (from, to) = (from?, to?);
            let (from, to) = if mirrored {
                (turn_deg - to, turn_deg - from)
            } else {
                (turn_deg + from, turn_deg + to)
            };
            out[start] = Out::Set(own[start], number(from.rem_euclid(360.0)));
            out[end] = Out::Set(own[end], number(to.rem_euclid(360.0)));
        }
        if rules.ellipse
            && mirrored
            && let (Some(start), Some(end)) = (position(41), position(42))
        {
            let tau = std::f64::consts::TAU;
            let [from, to] = [start, end].map(|at| self.number(own[at]));
            let (from, to) = (from?, to?);
            out[start] = Out::Set(own[start], number((0.0 - to).rem_euclid(tau)));
            out[end] = Out::Set(own[end], number((0.0 - from).rem_euclid(tau)));
        }
        // A group the record lacks goes after the group `after`, in the order set.
        let mut new = Vec::new();
        let mut set = |out: &mut Vec<Out>, code: i32, after: i32, value: f64| {
            if let Some(at) = position(code) {
                out[at] = Out::Set(own[at], number(value));
            } else if let Some(at) = position(after) {
                new.push((at, Out::New(code, number(value))));
            }
        };
        if let Some(code) = rules.rotation
            && !(rules.direction.is_some() && position(11).is_some())
        {
            let rotation = number_of(code)?.unwrap_or(0.0).to_radians();
            let turned = readable(rotation).to_degrees().rem_euclid(360.0);
            set(&mut out, code, 40, turned);
        }
        if rules.insert {
            let sx = number_of(41)?.unwrap_or(1.0);
            let sy = number_of(42)?.unwrap_or(1.0);
            let rotation = number_of(50)?.unwrap_or(0.0);
            let (rotation, sy) = if mirrored {
                (turn_deg - rotation, 0.0 - sy)
            } else {
                (turn_deg + rotation, sy)
            };
            let anchor = if position(30).is_some() { 30 } else { 20 };
            set(&mut out, 41, anchor, sx * scale);
            set(&mut out, 42, anchor, sy * scale);
            set(&mut out, 50, anchor, rotation.rem_euclid(360.0));
        }
        // From the last place back, so the places before stay where they are.
        new.sort_by_key(|(at, _)| *at);
        for (at, pair) in new.into_iter().rev() {
            out.insert(at + 1, pair);
        }
        Ok(out)
    }

    /// The groups of a HATCH or MPOLYGON, `own`, mapped by `map` into `out`, by where they
    /// stand: the boundary paths (polylines with their bulges, or lines, arcs, elliptical
    /// arcs and splines), then the pattern, its lines and the seed points. The elevation
    /// before the paths stays. Mapped, it no longer follows the entities it was made from:
    /// it is written as not associative, without their handles.
    fn map_hatch(
        &self,
        own: &[usize],
        out: &mut [Out],
        map: &Affine,
        turn: f64,
        mirrored: bool,
        scale: f64,
    ) -> Result<(), EditedDxfError> {
        let code = |at: usize| self.pairs[own[at]].code;
        let value = |at: usize| self.number(own[at]);
        let integer = |at: usize| self.integer(own[at]);
        let set = |out: &mut [Out], at: usize, value: f64| {
            out[at] = Out::Set(own[at], number(value));
        };
        // The point (or with `linear` the direction) whose X is at `at` and Y at `at + 1`
        // of group `y`.
        let xy = |out: &mut [Out], at: usize, y: i32, linear: bool| {
            if at + 1 >= own.len() || code(at + 1) != y {
                return Err(EditedDxfError::from(DxfEditRefusal::Malformed));
            }
            let point = [value(at)?, value(at + 1)?];
            let [x, y] = if linear {
                let [[a, b, _], [c, d, _]] = *map;
                [a * point[0] + b * point[1], c * point[0] + d * point[1]]
            } else {
                apply(map, point)
            };
            set(out, at, x);
            set(out, at + 1, y);
            Ok(())
        };
        let starts = (0..own.len())
            .filter(|at| code(*at) == 92)
            .collect::<Vec<_>>();
        let first = *starts.first().ok_or(DxfEditRefusal::Malformed)?;
        if let Some(at) = (0..first).find(|at| code(*at) == 71) {
            set(out, at, 0.0);
        }
        let mut trailing = own.len();
        for (path, &start) in starts.iter().enumerate() {
            let end = starts.get(path + 1).copied().unwrap_or(own.len());
            let sources = (start + 1..end)
                .rev()
                .find(|at| code(*at) == 97)
                .ok_or(DxfEditRefusal::Malformed)?;
            let count = integer(sources)?;
            let source_end = sources + 1 + usize::try_from(count).unwrap_or(usize::MAX);
            if source_end > end || (sources + 1..source_end).any(|at| code(at) != 330) {
                return Err(DxfEditRefusal::Malformed.into());
            }
            set(out, sources, 0.0);
            out[sources + 1..source_end].fill_with(|| Out::Drop);
            if path + 1 == starts.len() {
                trailing = source_end;
            }
            let body = start + 1..sources;
            if integer(start)? & 2 != 0 {
                for at in body {
                    match code(at) {
                        10 => xy(out, at, 20, false)?,
                        42 if mirrored => set(out, at, 0.0 - value(at)?),
                        _ => {}
                    }
                }
                continue;
            }
            let edges = body
                .clone()
                .filter(|at| code(*at) == 72)
                .chain([sources])
                .collect::<Vec<_>>();
            for edge in edges.windows(2) {
                let edge = edge[0]..edge[1];
                let kind = integer(edge.start)?;
                let find = |wanted: i32| edge.clone().find(|at| code(*at) == wanted);
                for at in edge.clone() {
                    match (kind, code(at)) {
                        (1..=4, 10) | (1 | 4, 11) => xy(out, at, code(at) + 10, false)?,
                        (3, 11) | (4, 12 | 13) => xy(out, at, code(at) + 10, true)?,
                        (2, 40) => set(out, at, value(at)? * scale),
                        _ => {}
                    }
                }
                if matches!(kind, 2 | 3)
                    && let (Some(start), Some(end)) = (find(50), find(51))
                {
                    // Counterclockwise from `from` to `to`; a clockwise edge (73 = 0) gives
                    // the angles of its mirror image.
                    let counterclockwise = find(73).map_or(Ok(1.0), value)? != 0.0;
                    let (from, to) = (value(start)?, value(end)?);
                    let (from, to) = if counterclockwise {
                        (from, to)
                    } else {
                        (0.0 - to, 0.0 - from)
                    };
                    // An arc's angles lie in the plane; an elliptical arc's along its axis.
                    let (from, to) = match (kind, mirrored) {
                        (2, false) => (turn + from, turn + to),
                        (2, true) => (turn - to, turn - from),
                        (_, false) => (from, to),
                        (_, true) => (0.0 - to, 0.0 - from),
                    };
                    let (from, to) = if counterclockwise {
                        (from, to)
                    } else {
                        (0.0 - to, 0.0 - from)
                    };
                    set(out, start, from.rem_euclid(360.0));
                    set(out, end, to.rem_euclid(360.0));
                }
            }
        }
        for at in trailing..own.len() {
            match code(at) {
                52 | 53 => set(out, at, turned(value(at)?, turn, mirrored)),
                41 | 49 => set(out, at, value(at)? * scale),
                43 => xy(out, at, 44, false)?,
                45 => xy(out, at, 46, true)?,
                10 => xy(out, at, 20, false)?,
                11 => xy(out, at, 21, true)?,
                460 => {
                    let turn = turn.to_radians();
                    let angle = value(at)?;
                    set(out, at, if mirrored { turn - angle } else { turn + angle });
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Writes the pairs `record` to `out`; a copy (`Some((own, primary, original))`) takes
    /// the handle `own`, and the primary's handle where it named the original's, and
    /// leaves out the reactors and extension dictionary that are the original's.
    fn write_record(
        &self,
        out: &mut Vec<u8>,
        record: &[Out],
        copy: Option<(Option<u64>, Option<u64>, Option<u64>)>,
    ) {
        let mut in_group = false;
        for pair in record {
            let (index, value) = match pair {
                Out::Raw(index) => (*index, None),
                Out::Set(index, value) => (*index, Some(value.as_bytes())),
                Out::Drop => continue,
                Out::New(code, value) => {
                    out.extend_from_slice(format!("{code:>3}").as_bytes());
                    out.extend_from_slice(self.eol);
                    out.extend_from_slice(value.as_bytes());
                    out.extend_from_slice(self.eol);
                    continue;
                }
            };
            let pair = &self.pairs[index];
            let raw = self.value(index);
            let mut value = value.map(<[u8]>::to_vec);
            if let Some((own, primary, original)) = copy {
                if pair.code == 102 {
                    in_group = raw.starts_with(b"{");
                    continue;
                }
                if in_group || pair.code == 360 {
                    continue;
                }
                match pair.code {
                    5 => match own {
                        Some(handle) => value = Some(format!("{handle:X}").into_bytes()),
                        None => continue,
                    },
                    330 if original.is_some() && hex(raw) == original => {
                        if let Some(handle) = primary {
                            value = Some(format!("{handle:X}").into_bytes());
                        }
                    }
                    _ => {}
                }
            }
            match value {
                Some(value) => {
                    out.extend_from_slice(&self.source[pair.start()..pair.value().start]);
                    out.extend_from_slice(&value);
                    out.extend_from_slice(self.eol);
                }
                None => out.extend_from_slice(&self.source[pair.start()..pair.end()]),
            }
        }
    }

    /// The entity of `shape`, drawn on `layer`, in the drawing's units.
    fn drawn(
        &self,
        layer: &str,
        shape: &DrawnShape,
        frame: &Frame,
        handles: &mut Option<u64>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut pair = |code: i32, value: &[u8]| {
            out.extend_from_slice(format!("{code:>3}").as_bytes());
            out.extend_from_slice(self.eol);
            out.extend_from_slice(value);
            out.extend_from_slice(self.eol);
        };
        let layer_bytes = self
            .layers
            .iter()
            .find(|(name, _)| name == layer)
            .map_or_else(|| self.text(layer), |(_, raw)| raw.clone());
        let head = |pair: &mut dyn FnMut(i32, &[u8]), kind: &str, handles: &mut Option<u64>| {
            pair(0, kind.as_bytes());
            if let Some(handle) = take_handle(handles) {
                pair(5, format!("{handle:X}").as_bytes());
            }
            if self.subclassed {
                if let Some(owner) = &self.owner {
                    pair(330, owner);
                }
                pair(100, b"AcDbEntity");
            }
            pair(8, &layer_bytes);
        };
        let subclass = |pair: &mut dyn FnMut(i32, &[u8]), name: &str| {
            if self.subclassed {
                pair(100, name.as_bytes());
            }
        };
        let point = |pair: &mut dyn FnMut(i32, &[u8]), code: i32, at: [f64; 2]| {
            let [x, y] = frame.point(at);
            pair(code, number(x).as_bytes());
            pair(code + 10, number(y).as_bytes());
            pair(code + 20, b"0");
        };
        let length = |value: f64| number(value / frame.mm_per_unit);
        match shape {
            DrawnShape::Polyline { points_mm, closed } if points_mm.len() == 2 && !closed => {
                head(&mut pair, "LINE", handles);
                subclass(&mut pair, "AcDbLine");
                point(&mut pair, 10, points_mm[0]);
                point(&mut pair, 11, points_mm[1]);
            }
            DrawnShape::Polyline { points_mm, closed } if self.subclassed => {
                head(&mut pair, "LWPOLYLINE", handles);
                subclass(&mut pair, "AcDbPolyline");
                pair(90, points_mm.len().to_string().as_bytes());
                pair(70, if *closed { b"1" } else { b"0" });
                for at in points_mm {
                    let [x, y] = frame.point(*at);
                    pair(10, number(x).as_bytes());
                    pair(20, number(y).as_bytes());
                }
            }
            DrawnShape::Polyline { points_mm, closed } => {
                head(&mut pair, "POLYLINE", handles);
                pair(66, b"1");
                pair(70, if *closed { b"1" } else { b"0" });
                point(
                    &mut pair,
                    10,
                    [0.0 - frame.offset_mm[0], 0.0 - frame.offset_mm[1]],
                );
                for at in points_mm {
                    head(&mut pair, "VERTEX", handles);
                    point(&mut pair, 10, *at);
                }
                head(&mut pair, "SEQEND", handles);
            }
            DrawnShape::Arc {
                center_mm,
                radius_mm,
                start_deg,
                end_deg,
            } => {
                head(&mut pair, "ARC", handles);
                subclass(&mut pair, "AcDbCircle");
                point(&mut pair, 10, *center_mm);
                pair(40, length(*radius_mm).as_bytes());
                subclass(&mut pair, "AcDbArc");
                pair(50, number(start_deg.rem_euclid(360.0)).as_bytes());
                pair(51, number(end_deg.rem_euclid(360.0)).as_bytes());
            }
            DrawnShape::Circle {
                center_mm,
                radius_mm,
            } => {
                head(&mut pair, "CIRCLE", handles);
                subclass(&mut pair, "AcDbCircle");
                point(&mut pair, 10, *center_mm);
                pair(40, length(*radius_mm).as_bytes());
            }
            DrawnShape::Text {
                anchor_mm,
                height_mm,
                rotation_deg,
                text,
            } => {
                head(&mut pair, "TEXT", handles);
                subclass(&mut pair, "AcDbText");
                point(&mut pair, 10, *anchor_mm);
                pair(40, length(*height_mm).as_bytes());
                pair(1, &self.text(text));
                pair(50, number(rotation_deg.rem_euclid(360.0)).as_bytes());
                subclass(&mut pair, "AcDbText");
            }
        }
        out
    }

    /// `text` as the drawing writes a text: UTF-8 from DXF 2007 on, else what is not
    /// ASCII as `\U+XXXX`; a `%%` that would be read as a special character is kept as
    /// written, and what would end the line is a space.
    fn text(&self, text: &str) -> Vec<u8> {
        let mut out = String::with_capacity(text.len());
        let percent = text.contains("%%");
        for character in text.chars() {
            match character {
                '%' if percent => out.push_str("%%%"),
                character if character.is_control() => out.push(' '),
                character if character.is_ascii() || self.utf8 => out.push(character),
                character => match u16::try_from(u32::from(character)) {
                    Ok(unit) => out.push_str(&format!("\\U+{unit:04X}")),
                    Err(_) => out.push('?'),
                },
            }
        }
        out.into_bytes()
    }
}

/// Byte ranges of the source and what replaces each.
type ObjectEdits = Vec<(Range<usize>, Vec<u8>)>;

/// One pair written of a record: as read, with a new value, new, or left out.
enum Out {
    Raw(usize),
    Set(usize, String),
    New(i32, String),
    Drop,
}

/// A map of an entity, of the drawing's units; `stretch` when it makes a straight line
/// shorter or longer, so that its widths stay as they are. The entity written no longer
/// names the handles `removed`. With `vertices_minus_z` the VERTEX records mapped are of a
/// POLYLINE of extrusion −Z.
#[derive(Clone, Copy)]
struct Mapping<'r> {
    map: Affine,
    stretch: bool,
    removed: &'r BTreeSet<u64>,
    vertices_minus_z: bool,
}

/// The records of the straight lines that are stretched ([`UnderlayEdits::stretched_objects`]).
const STRETCHED_RECORDS: [&str; 5] = ["LINE", "LWPOLYLINE", "POLYLINE", "VERTEX", "SEQEND"];

/// What of a record a map changes, by group code.
struct Rules {
    /// The X groups of points; Y is the group 10 after.
    points: &'static [i32],
    /// The X groups of directions, turned but not moved.
    vectors: &'static [i32],
    /// Lengths, scaled.
    lengths: &'static [i32],
    /// Group 42 is a bulge, negated by a mirror.
    bulge: bool,
    /// Groups 50 and 51 are the angles of an arc.
    arc: bool,
    /// Groups 41 and 42 are the parameters of an ellipse's arc.
    ellipse: bool,
    /// The group of a text's rotation in degrees, kept readable.
    rotation: Option<i32>,
    /// The vector group of a text's direction, which decides over its rotation.
    direction: Option<i32>,
    /// An INSERT: groups 41, 42 and 50 are its scales and rotation.
    insert: bool,
    /// Angles in degrees of directions in the plane, turned and mirrored with it.
    angles: &'static [i32],
    /// A HATCH or MPOLYGON: what its groups are depends on where they stand.
    hatch: bool,
    /// A DIMENSION: group 42 is its measurement, and it draws a block of its own.
    dimension: bool,
}

impl Rules {
    fn of(record: &str) -> Option<Self> {
        let plain = Self {
            points: &[],
            vectors: &[],
            lengths: &[],
            bulge: false,
            arc: false,
            ellipse: false,
            rotation: None,
            direction: None,
            insert: false,
            angles: &[],
            hatch: false,
            dimension: false,
        };
        Some(match record {
            "LINE" => Self {
                points: &[10, 11],
                ..plain
            },
            "POINT" => Self {
                points: &[10],
                ..plain
            },
            "CIRCLE" => Self {
                points: &[10],
                lengths: &[40],
                ..plain
            },
            "ARC" => Self {
                points: &[10],
                lengths: &[40],
                arc: true,
                ..plain
            },
            "LWPOLYLINE" => Self {
                points: &[10],
                lengths: &[40, 41, 43],
                bulge: true,
                ..plain
            },
            "POLYLINE" => Self {
                lengths: &[40, 41],
                ..plain
            },
            "VERTEX" => Self {
                points: &[10],
                lengths: &[40, 41],
                bulge: true,
                ..plain
            },
            "SEQEND" => plain,
            "TEXT" | "ATTRIB" | "ATTDEF" => Self {
                points: &[10, 11],
                lengths: &[40],
                rotation: Some(50),
                ..plain
            },
            "MTEXT" => Self {
                points: &[10],
                vectors: &[11],
                lengths: &[40, 41, 42, 43],
                rotation: Some(50),
                direction: Some(11),
                ..plain
            },
            "INSERT" => Self {
                points: &[10],
                lengths: &[44, 45],
                insert: true,
                ..plain
            },
            "ELLIPSE" => Self {
                points: &[10],
                vectors: &[11],
                ellipse: true,
                ..plain
            },
            "SPLINE" => Self {
                points: &[10, 11],
                vectors: &[12, 13],
                ..plain
            },
            "SOLID" | "TRACE" | "3DFACE" => Self {
                points: &[10, 11, 12, 13],
                ..plain
            },
            "LEADER" => Self {
                points: &[10],
                vectors: &[211, 212, 213],
                lengths: &[40, 41],
                ..plain
            },
            "MESH" => Self {
                points: &[10],
                ..plain
            },
            "HATCH" | "MPOLYGON" => Self {
                hatch: true,
                ..plain
            },
            "DIMENSION" => Self {
                points: &[10, 11, 13, 14, 15, 16],
                vectors: &[12],
                angles: &[50, 53],
                dimension: true,
                ..plain
            },
            _ => return None,
        })
    }
}

/// The direction at `degrees` in the plane, turned by `turn` and mirrored when `mirrored`.
fn turned(degrees: f64, turn: f64, mirrored: bool) -> f64 {
    if mirrored {
        turn - degrees
    } else {
        turn + degrees
    }
    .rem_euclid(360.0)
}

/// The next of `handles`, which moves on past it.
fn take_handle(handles: &mut Option<u64>) -> Option<u64> {
    let handle = (*handles)?;
    *handles = handle.checked_add(1);
    Some(handle)
}

fn hex(value: &[u8]) -> Option<u64> {
    u64::from_str_radix(std::str::from_utf8(value).ok()?, 16).ok()
}

/// `value` as a DXF number: the shortest text that reads back as it, without `-0`.
fn number(value: f64) -> String {
    let value = if value == 0.0 { 0.0 } else { value };
    format!("{value}")
}

/// The group pairs of `source`, line by line, in one pass.
fn pairs(source: &[u8]) -> Result<Vec<Pair>, EditedDxfError> {
    // A line from `start`: where it starts, where its content ends, and past its end.
    let line = |start: usize| {
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(source.len(), |at| start + at);
        let content_end = if end > start && source[end - 1] == b'\r' {
            end - 1
        } else {
            end
        };
        (start, content_end, (end + 1).min(source.len()))
    };
    let place = |at: usize| {
        u32::try_from(at).map_err(|_: std::num::TryFromIntError| DxfEditRefusal::TooLarge)
    };
    let mut pairs = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let (code_start, code_end, next) = line(start);
        if next >= source.len() {
            // A last empty line after the end of file is no pair.
            if source[code_start..code_end].trim_ascii().is_empty() {
                break;
            }
            return Err(DxfEditRefusal::Malformed.into());
        }
        let (value_start, value_end, after) = line(next);
        let code = std::str::from_utf8(source[code_start..code_end].trim_ascii())
            .ok()
            .and_then(|code| code.parse::<i32>().ok())
            .ok_or(DxfEditRefusal::Malformed)?;
        pairs.push(Pair {
            code,
            start: place(code_start)?,
            value: [place(value_start)?, place(value_end)?],
            end: place(after)?,
        });
        start = after;
    }
    Ok(pairs)
}
