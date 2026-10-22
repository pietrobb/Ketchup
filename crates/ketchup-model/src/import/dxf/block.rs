//! The BLOCKS section: each block read once into the profiles and texts its inserts
//! place; an xref or a block of one is not loaded.

use super::*;

#[derive(Clone)]
pub(super) struct BlockDefinition {
    pub(super) base_mm: [f64; 2],
    pub(super) profiles: Vec<OrderedProfile>,
    pub(super) texts: Vec<OrderedText>,
    /// An xref or a block of one, not loaded: its inserts are named in the report.
    pub(super) xref: bool,
}

impl BlockDefinition {
    pub(super) fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.texts.is_empty()
    }
}

#[derive(Clone)]
struct RawBlockDefinition<'a> {
    base_mm: [f64; 2],
    entities: Vec<DxfPair<'a>>,
    xref: bool,
}

struct BlockResolution<'a> {
    blocks: &'a mut BTreeMap<String, BlockDefinition>,
    resolving: &'a mut BTreeSet<String>,
    entity_count: &'a mut usize,
    diagnostics: &'a mut DiagnosticCounts,
}

pub(super) fn parse_blocks<'a>(
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
        match read {
            Ok((key, _, _)) if raw_blocks.contains_key(&key) => {
                skip_block(&key, DxfImportError::InvalidBlock, diagnostics)?;
            }
            Ok((key, base_mm, xref)) => {
                raw_blocks.insert(
                    key,
                    RawBlockDefinition {
                        base_mm,
                        entities,
                        xref,
                    },
                );
            }
            Err(error) => {
                let name = parse_optional_text(header, 2).ok().flatten();
                skip_block(name.unwrap_or_default(), error, diagnostics)?;
            }
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
            if let Err(error) = resolve_block(&key, &raw_blocks, 0, units, &mut resolution) {
                resolution.resolving.clear();
                skip_block(&key, error, resolution.diagnostics)?;
            }
        }
    }
    Ok(blocks)
}

/// A block an underlay cannot read is left out and counted, named with why; its
/// inserts are then left out as entities it cannot read. A profile import refuses the
/// file.
fn skip_block(
    name: &str,
    error: DxfImportError,
    diagnostics: &mut DiagnosticCounts,
) -> Result<(), DxfImportError> {
    if !diagnostics.reading.skip_unreadable {
        return Err(error);
    }
    diagnostics.add(
        ImportDiagnosticSeverity::Warning,
        "dxf.block-unreadable",
        Some(format!("{name}: {error}")),
        1,
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
    let raw = raw_blocks
        .get(key)
        .cloned()
        .ok_or(DxfImportError::InvalidBlock)?;
    if depth >= MAX_DXF_BLOCK_DEPTH || !resolution.resolving.insert(key.to_owned()) {
        return Err(DxfImportError::InvalidBlock);
    }
    if raw.xref {
        resolution.blocks.insert(
            key.to_owned(),
            BlockDefinition {
                base_mm: raw.base_mm,
                profiles: Vec::new(),
                texts: Vec::new(),
                xref: true,
            },
        );
        resolution.resolving.remove(key);
        return Ok(());
    }
    for dependency in block_dependencies(&raw.entities)? {
        // In an underlay, an insert of a block that cannot be resolved here (missing,
        // too deep, or one inserting itself) is left out as an entity it cannot read.
        if let Err(error) = resolve_block(&dependency, raw_blocks, depth + 1, units, resolution)
            && !resolution.diagnostics.reading.skip_unreadable
        {
            return Err(error);
        }
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
    let reading = resolution.diagnostics.reading;
    let mut profiles = if reading.chain {
        assemble_loose_segments(parsed_entities.loose, reading.max_segments)?
    } else {
        parsed_entities
            .loose
            .into_iter()
            .map(OrderedProfile::from)
            .collect()
    };
    profiles.append(&mut parsed_entities.explicit_profiles);
    if reading.chain {
        reject_duplicate_segments(&profiles)?;
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
            xref: false,
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
