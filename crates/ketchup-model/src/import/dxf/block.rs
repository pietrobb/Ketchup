//! The BLOCKS section: each block read once into the profiles and texts its inserts
//! place; an xref or a block of one is not loaded.

use super::*;

#[derive(Clone)]
pub(super) struct BlockDefinition {
    pub(super) base_mm: [f64; 2],
    pub(super) profiles: Vec<OrderedProfile>,
    pub(super) texts: Vec<OrderedText>,
    /// Read for an underlay only.
    pub(super) fills: Vec<OrderedFill>,
    /// An xref or a block of one, not loaded: its inserts are named in the report.
    pub(super) xref: bool,
    /// The xrefs it places, its own blocks' among them, by name with how often: each
    /// insert of it counts them once more.
    pub(super) xrefs: BTreeMap<String, u32>,
    /// Read for an underlay only: its entities, in its own coordinates; the objects of its
    /// profiles, texts and fills index them.
    pub(super) objects: Vec<super::objects::SourceObject>,
    /// How many entities it places, those of the blocks it places included: each insert
    /// counts them again against the reading's entity budget.
    pub(super) entities: usize,
}

impl BlockDefinition {
    pub(super) fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.texts.is_empty() && self.fills.is_empty()
    }
}

#[derive(Clone)]
struct RawBlockDefinition<'a> {
    /// As the drawing writes it; its key is upper case.
    name: String,
    base_mm: [f64; 2],
    entities: Vec<DxfPair<'a>>,
    xref: bool,
}

struct BlockResolution<'a> {
    blocks: &'a mut BTreeMap<String, BlockDefinition>,
    resolving: &'a mut BTreeSet<String>,
    /// The blocks that could not be resolved, with why: each is read and reported once.
    failed: BTreeMap<String, DxfImportError>,
    entity_count: &'a mut usize,
    diagnostics: &'a mut DiagnosticCounts,
}

/// The keys of the blocks that the INSERTs and DIMENSIONs of a section name, seen one
/// pair at a time.
#[derive(Default)]
pub(super) struct InsertedBlocks {
    keys: BTreeSet<String>,
    placing: bool,
}

impl InsertedBlocks {
    pub(super) fn see(&mut self, code: i16, value: &str) {
        if code == 0 {
            self.placing =
                value.eq_ignore_ascii_case("INSERT") || value.eq_ignore_ascii_case("DIMENSION");
        } else if code == 2
            && self.placing
            && let Ok(key) = block_name_key(value)
        {
            self.keys.insert(key);
        }
    }

    pub(super) fn of(pairs: &[DxfPair<'_>]) -> Self {
        let mut inserted = Self::default();
        for pair in pairs {
            inserted.see(pair.code, pair.value);
        }
        inserted
    }

    fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
}

pub(super) fn parse_blocks<'a>(
    pairs: &[DxfPair<'a>],
    inserted: &InsertedBlocks,
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
        bump_entity_count(entity_count, diagnostics.reading.max_entities)?;
        let header_start = index + 1;
        index = header_start;
        while index < pairs.len() && pairs[index].code != 0 {
            index += 1;
        }
        let header = &pairs[header_start..index];
        let mut read_header = || {
            ensure_planar(header)?;
            report_ignored_groups(
                "BLOCK",
                header,
                &[5, 8, 2, 3, 10, 20, 30, 70, 100],
                diagnostics,
            )?;
            parse_layer(header)?;
            // Bits 4, 8, 16 and 32 make an xref or a block of one, which is not loaded;
            // bit 64 is ignored on input.
            let flags = parse_optional_i64(header, 70)?.unwrap_or(0);
            if flags & !(1 | 2 | 4 | 8 | 16 | 32 | 64) != 0 {
                return Err(DxfImportError::InvalidBlock);
            }
            let xref = flags & (4 | 8 | 16 | 32) != 0;
            let name = parse_required_text(header, 2)?;
            if parse_optional_text(header, 3)?.is_some_and(|alternate| alternate != name) {
                return Err(DxfImportError::InvalidBlock);
            }
            let key = block_name_key(name)?;
            let base_mm = parse_xy(header, 10, 20, units.millimetres_per_unit())?;
            Ok((key, base_mm, xref))
        };
        let read = read_header();

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
        bump_entity_count(entity_count, diagnostics.reading.max_entities)?;
        let read = read.and_then(|block| {
            parse_optional_layer(end_record)?;
            report_ignored_groups("ENDBLK", end_record, &[5, 8, 100], diagnostics)?;
            Ok(block)
        });
        let name = || {
            parse_optional_text(header, 2)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        match read {
            Ok((key, _, _)) if raw_blocks.contains_key(&key) => {
                skip_block(name(), DxfImportError::InvalidBlock, diagnostics)?;
            }
            Ok((key, base_mm, xref)) => {
                raw_blocks.insert(
                    key,
                    RawBlockDefinition {
                        name: name().to_owned(),
                        base_mm,
                        entities,
                        xref,
                    },
                );
            }
            Err(error) => skip_block(name(), error, diagnostics)?,
        }
    }

    // Only the blocks the drawing's entities place are read, with the blocks they
    // place: a block nothing inserts would otherwise cost its whole expansion
    // (review 2026-10-24 UL-24).
    let keys = (raw_blocks.keys())
        .filter(|key| inserted.contains(key))
        .cloned()
        .collect::<Vec<_>>();
    let mut blocks = BTreeMap::new();
    let mut resolving = BTreeSet::new();
    {
        let mut resolution = BlockResolution {
            blocks: &mut blocks,
            resolving: &mut resolving,
            failed: BTreeMap::new(),
            entity_count,
            diagnostics,
        };
        for key in keys {
            if let Err(error) = resolve_block(&key, &raw_blocks, 0, units, &mut resolution) {
                skip_block(&raw_blocks[&key].name, error, resolution.diagnostics)?;
            }
        }
    }
    Ok(blocks)
}

/// A block it cannot read is left out and counted by why, with its name; its inserts and
/// dimensions are then left out as entities it cannot read. A limit of the reading still
/// ends it.
fn skip_block(
    name: &str,
    error: DxfImportError,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    if error.ends_reading() {
        return Err(error);
    }
    diagnostics.skip(
        "dxf.block-unreadable",
        error.reason(),
        Some(("blocks", name)),
        Some(error),
    )
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
    if let Some(error) = resolution.failed.get(key) {
        return Err(error.clone());
    }
    let raw = raw_blocks
        .get(key)
        .cloned()
        .ok_or(DxfImportError::InvalidBlock)?;
    if depth >= MAX_DXF_BLOCK_DEPTH || !resolution.resolving.insert(key.to_owned()) {
        return Err(DxfImportError::InvalidBlock);
    }
    let resolved = resolve_raw_block(key, raw, raw_blocks, depth, units, resolution);
    resolution.resolving.remove(key);
    if let Err(error) = &resolved {
        resolution.failed.insert(key.to_owned(), error.clone());
    }
    resolved
}

/// [`resolve_block`] of the block `key`, read as `raw`, while it is being resolved.
fn resolve_raw_block<'a>(
    key: &str,
    raw: RawBlockDefinition<'a>,
    raw_blocks: &BTreeMap<String, RawBlockDefinition<'a>>,
    depth: usize,
    units: ImportUnitDecision,
    resolution: &mut BlockResolution<'_>,
) -> Result<(), DxfImportError> {
    if raw.xref {
        resolution.blocks.insert(
            key.to_owned(),
            BlockDefinition {
                base_mm: raw.base_mm,
                profiles: Vec::new(),
                texts: Vec::new(),
                fills: Vec::new(),
                xref: true,
                xrefs: BTreeMap::new(),
                objects: Vec::new(),
                entities: 0,
            },
        );
        return Ok(());
    }
    for dependency in block_dependencies(&raw.entities)? {
        // An insert or dimension of a block that cannot be resolved here (missing, too
        // deep, or one inserting itself) is left out as an entity it cannot read.
        if let Err(error) = resolve_block(&dependency, raw_blocks, depth + 1, units, resolution)
            && error.ends_reading()
        {
            return Err(error);
        }
    }

    let mut parsed_entities = ParsedEntities::default();
    // What its hatches draw counts where an insert places it, and what it leaves out is
    // reported in its own coordinates, under its name.
    let pieces_left = resolution.diagnostics.fill_pieces_left;
    let outer = resolution.diagnostics.block.replace(raw.name.clone());
    let counted_before = *resolution.entity_count;
    let parsed = parse_entities(
        &raw.entities,
        units,
        Some(resolution.blocks),
        resolution.entity_count,
        &mut parsed_entities,
        resolution.diagnostics,
    );
    resolution.diagnostics.fill_pieces_left = pieces_left;
    resolution.diagnostics.block = outer;
    parsed?;
    let entities = *resolution.entity_count - counted_before;
    let reading = resolution.diagnostics.reading;
    let mut profiles = if reading.chain {
        assemble_loose_segments(
            parsed_entities.loose,
            reading.max_segments,
            Some((
                &parsed_entities.handles,
                &mut *resolution.diagnostics,
                Some(raw.name.as_str()),
            )),
        )?
    } else {
        parsed_entities
            .loose
            .into_iter()
            .map(OrderedProfile::from)
            .collect()
    };
    profiles.append(&mut parsed_entities.explicit_profiles);
    if reading.chain {
        profiles = without_duplicate_segments(
            profiles,
            Some((
                &parsed_entities.handles,
                &mut *resolution.diagnostics,
                Some(raw.name.as_str()),
            )),
        )?;
    }
    if profiles.len() > reading.max_profiles {
        return Err(DxfImportError::TooManyProfiles);
    }
    resolution.blocks.insert(
        key.to_owned(),
        BlockDefinition {
            base_mm: raw.base_mm,
            profiles,
            texts: parsed_entities.texts,
            fills: parsed_entities.fills,
            xref: false,
            xrefs: parsed_entities.xrefs,
            objects: parsed_entities.objects,
            entities,
        },
    );
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
