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
    /// What a placement of it builds but for its arcs: the points of its lines and
    /// curves, its texts and the pieces of its fills.
    pieces: usize,
    /// The radius and turn of each of its arcs.
    arcs: Vec<[f64; 2]>,
}

impl BlockDefinition {
    fn new(
        base_mm: [f64; 2],
        profiles: Vec<OrderedProfile>,
        texts: Vec<OrderedText>,
        fills: Vec<OrderedFill>,
    ) -> Self {
        let segments = || profiles.iter().flat_map(|profile| &profile.segments);
        let fill_pieces = |fill: &OrderedFill| {
            fill.pattern.len() + fill.rings.iter().map(Vec::len).sum::<usize>()
        };
        let pieces = (segments().map(crate::underlay::straight_points))
            .chain(fills.iter().map(fill_pieces))
            .fold(texts.len(), usize::saturating_add);
        let arcs = segments()
            .filter_map(crate::underlay::arc_turn)
            .map(|[radius, _, sweep]| [radius, sweep])
            .collect();
        Self {
            base_mm,
            profiles,
            texts,
            fills,
            xref: false,
            xrefs: BTreeMap::new(),
            objects: Vec::new(),
            entities: 0,
            pieces,
            arcs,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.texts.is_empty() && self.fills.is_empty()
    }

    /// What one placement of it scaled by `scale` builds: the points of its lines and
    /// curves, the chords of its arcs at the radius they are placed at, its texts and the
    /// pieces of its fills (review 2026-10-26 UL-40).
    pub(super) fn placed_size(&self, scale: f64) -> usize {
        (self.arcs.iter())
            .map(|[radius, sweep]| crate::underlay::arc_chords(radius * scale, *sweep))
            .fold(self.pieces, usize::saturating_add)
    }

    /// At most [`Self::placed_size`] at any scale: each line and arc builds a piece at
    /// least.
    pub(super) fn least_placed_size(&self) -> usize {
        self.pieces.saturating_add(self.arcs.len())
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
    /// [`nesting_heights`] of the drawing's blocks.
    heights: BTreeMap<String, usize>,
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
    {
        let mut resolution = BlockResolution {
            blocks: &mut blocks,
            heights: nesting_heights(&raw_blocks),
            failed: BTreeMap::new(),
            entity_count,
            diagnostics,
        };
        for key in keys {
            if let Err(error) = resolve_block(&key, &raw_blocks, units, &mut resolution) {
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

/// How many blocks deep each block nests, itself included: 1 for a block that places
/// none. An insert of a block the drawing does not define, or of one that places the
/// inserting block back, does not count; resolving refuses those. Counted over the whole
/// drawing, so the limit on it does not depend on the block the reading starts at
/// (review 2026-10-24 UL-28), and without recursion, however long a chain is.
fn nesting_heights(
    raw_blocks: &BTreeMap<String, RawBlockDefinition<'_>>,
) -> BTreeMap<String, usize> {
    let placed: BTreeMap<&str, Vec<String>> = (raw_blocks.iter())
        .map(|(key, raw)| {
            let placed = if raw.xref {
                Vec::new()
            } else {
                block_dependencies(&raw.entities).unwrap_or_default()
            };
            (key.as_str(), placed)
        })
        .collect();
    let mut heights = BTreeMap::<String, usize>::new();
    let mut on_path = BTreeSet::new();
    for &root in placed.keys() {
        if heights.contains_key(root) {
            continue;
        }
        on_path.insert(root);
        let mut stack = vec![(root, 0)];
        while let Some(&mut (key, ref mut next)) = stack.last_mut() {
            let children = &placed[key];
            if let Some(child) = children.get(*next) {
                *next += 1;
                let child = child.as_str();
                if placed.contains_key(child)
                    && !heights.contains_key(child)
                    && on_path.insert(child)
                {
                    stack.push((child, 0));
                }
                continue;
            }
            let below = (children.iter())
                .filter_map(|child| heights.get(child.as_str()))
                .max();
            heights.insert(key.to_owned(), below.map_or(1, |below| below + 1));
            on_path.remove(key);
            stack.pop();
        }
    }
    heights
}

fn resolve_block<'a>(
    key: &str,
    raw_blocks: &BTreeMap<String, RawBlockDefinition<'a>>,
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
    let resolved = if resolution.heights.get(key) > Some(&MAX_DXF_BLOCK_DEPTH) {
        Err(DxfImportError::BlocksTooDeep)
    } else {
        resolve_raw_block(key, raw, raw_blocks, units, resolution)
    };
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
    units: ImportUnitDecision,
    resolution: &mut BlockResolution<'_>,
) -> Result<(), DxfImportError> {
    if raw.xref {
        resolution.blocks.insert(
            key.to_owned(),
            BlockDefinition {
                xref: true,
                ..BlockDefinition::new(raw.base_mm, Vec::new(), Vec::new(), Vec::new())
            },
        );
        return Ok(());
    }
    let height = resolution.heights.get(key).copied().unwrap_or(1);
    for dependency in block_dependencies(&raw.entities)? {
        // An insert or dimension of a block that cannot be resolved here (missing, too
        // deep, or one inserting itself) is left out as an entity it cannot read. A block
        // that nests no less deep than this one places it back: each block resolved
        // nests less deep than the one placing it, so resolving recurses at most
        // MAX_DXF_BLOCK_DEPTH deep.
        let resolved = if resolution.heights.get(&dependency) >= Some(&height) {
            Err(DxfImportError::InvalidBlock)
        } else {
            resolve_block(&dependency, raw_blocks, units, resolution)
        };
        if let Err(error) = resolved
            && error.ends_reading()
        {
            return Err(error);
        }
    }

    let mut parsed_entities = ParsedEntities::default();
    // What its hatches draw counts where an insert places it, and what it leaves out is
    // reported in its own coordinates, under its name. What it places inside it counts
    // against what the blocks may hold, not against what the drawing's inserts place:
    // counted there too, a reference placed once was refused for twice its size (review
    // 2026-10-26 UL-42).
    let pieces_left = resolution.diagnostics.fill_pieces_left;
    let diagnostics = &mut *resolution.diagnostics;
    std::mem::swap(&mut diagnostics.placed_left, &mut diagnostics.held_left);
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
    let diagnostics = &mut *resolution.diagnostics;
    std::mem::swap(&mut diagnostics.placed_left, &mut diagnostics.held_left);
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
            xrefs: parsed_entities.xrefs,
            objects: parsed_entities.objects,
            entities,
            ..BlockDefinition::new(
                raw.base_mm,
                profiles,
                parsed_entities.texts,
                parsed_entities.fills,
            )
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
