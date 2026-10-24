//! The objects of a drawing read as an underlay ([`crate::underlay::objects`]): each entity
//! read is one, with its kind, handle and layer, and claims the lines, texts and fills it
//! drew. An insert or a dimension also places the objects of its block as its children,
//! their details carried to where it places them. What an object holds beyond that is
//! read from its record and never refuses the entity: a value it cannot read is left out.

use super::style::UNCLAIMED;
use super::*;
use crate::underlay::objects::{Affine, ObjectDetail, ObjectKind};

/// An object as read, in the coordinates of what it was read in (the drawing or a block).
#[derive(Clone)]
pub(super) struct SourceObject {
    pub(super) kind: ObjectKind,
    pub(super) handle: u64,
    pub(super) layer: String,
    /// An object read before it with it.
    pub(super) parent: Option<u32>,
    pub(super) detail: ObjectDetail,
}

/// Where an insert places the points of its block, as [`super::place_point`] does: from
/// `base_mm`, scaled, turned and moved to `insertion_mm`.
pub(super) fn placement(
    base_mm: [f64; 2],
    insertion_mm: [f64; 2],
    scale: [f64; 2],
    rotation_sin: f64,
    rotation_cos: f64,
) -> Affine {
    let linear = [
        [rotation_cos * scale[0], -rotation_sin * scale[1]],
        [rotation_sin * scale[0], rotation_cos * scale[1]],
    ];
    [0, 1].map(|row| {
        let moved = linear[row][0] * base_mm[0] + linear[row][1] * base_mm[1];
        [linear[row][0], linear[row][1], insertion_mm[row] - moved]
    })
}

/// Makes the entity of `kind` just read — its groups and what follows it up to its SEQEND
/// in `span` — an object of `output`, and gives it what it drew since `counts`; what an
/// insert or dimension placed from its block goes to the objects of the block, placed as
/// its children.
pub(super) fn claim(
    output: &mut ParsedEntities,
    counts: [usize; 4],
    kind: &str,
    span: &[DxfPair<'_>],
    blocks: Option<&BTreeMap<String, BlockDefinition>>,
    units: ImportUnitDecision,
) -> Result<(), DxfImportError> {
    let placement = output.placement.take();
    let Some(object_kind) = ObjectKind::from_dxf(kind) else {
        return Ok(());
    };
    let places = matches!(kind, "INSERT" | "DIMENSION");
    if output.counts() == counts && !places {
        return Ok(());
    }
    let record_end = span
        .iter()
        .position(|pair| pair.code == 0)
        .unwrap_or(span.len());
    let (record, following) = span.split_at(record_end);
    let text = |code| parse_optional_text(record, code).ok().flatten();
    let handle = text(5)
        .and_then(|handle| u64::from_str_radix(handle.trim(), 16).ok())
        .unwrap_or(0);
    let detail = match kind {
        "INSERT" => insert_detail(record, following, placement),
        "DIMENSION" => dimension_detail(record, units),
        "HATCH" => ObjectDetail::Hatch {
            pattern: text(2).unwrap_or_default().to_owned(),
            solid: parse_optional_i64(record, 70).ok().flatten() == Some(1),
            rings: output
                .fills
                .get_mut(counts[3])
                .map(|fill| std::mem::take(&mut fill.outline))
                .unwrap_or_default(),
        },
        _ => ObjectDetail::None,
    };
    let own = count(output.objects.len())?;
    output.objects.push(SourceObject {
        kind: object_kind,
        handle,
        layer: parse_layer(record)?,
        parent: None,
        detail,
    });
    let children = own + 1;
    let block = text(2)
        .and_then(|name| block_name_key(name).ok())
        .and_then(|key| blocks?.get(&key));
    if let (true, Some(map), Some(block)) = (places, placement, block) {
        for child in &block.objects {
            output.objects.push(SourceObject {
                parent: Some(child.parent.map_or(own, |parent| parent + children)),
                detail: child.detail.transformed(&map),
                ..child.clone()
            });
        }
    }
    let assign = |object: &mut u32| {
        *object = if *object == UNCLAIMED {
            own
        } else {
            *object + children
        };
    };
    output.loose[counts[0]..]
        .iter_mut()
        .for_each(|loose| assign(&mut loose.style.object));
    output.explicit_profiles[counts[1]..]
        .iter_mut()
        .for_each(|profile| assign(&mut profile.style.object));
    output.texts[counts[2]..]
        .iter_mut()
        .for_each(|text| assign(&mut text.style.object));
    output.fills[counts[3]..]
        .iter_mut()
        .for_each(|fill| assign(&mut fill.style.object));
    Ok(())
}

fn count(value: usize) -> Result<u32, DxfImportError> {
    u32::try_from(value).map_err(|_: std::num::TryFromIntError| DxfImportError::TooManyEntities)
}

/// The block an INSERT `record` places at `placement`, how often, and the tags and values
/// of the ATTRIBs that follow it.
fn insert_detail(
    record: &[DxfPair<'_>],
    following: &[DxfPair<'_>],
    placement: Option<Affine>,
) -> ObjectDetail {
    let copies = [70, 71].map(|code| {
        parse_optional_i64(record, code)
            .ok()
            .flatten()
            .and_then(|copies| u32::try_from(copies).ok())
            .unwrap_or(1)
    });
    let mut attributes = Vec::new();
    let mut rest = following;
    while let Some((head, tail)) = rest.split_first() {
        let end = tail
            .iter()
            .position(|pair| pair.code == 0)
            .unwrap_or(tail.len());
        let (attribute, after) = tail.split_at(end);
        if head.value.eq_ignore_ascii_case("ATTRIB") {
            let value = |code| {
                parse_optional_text(attribute, code)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
                    .to_owned()
            };
            attributes.push((value(2), value(1)));
        }
        rest = after;
    }
    ObjectDetail::Insert {
        block: parse_optional_text(record, 2)
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_owned(),
        transform: placement.unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        copies,
        attributes,
    }
}

/// The type, measurement, text and definition points of a DIMENSION `record`.
fn dimension_detail(record: &[DxfPair<'_>], units: ImportUnitDecision) -> ObjectDetail {
    let number = |code| parse_optional_number(record, code).ok().flatten();
    let scale = units.millimetres_per_unit();
    let dimension_type = parse_optional_i64(record, 70)
        .ok()
        .flatten()
        .and_then(|flags| u8::try_from(flags & 7).ok())
        .unwrap_or(0);
    let angular = matches!(dimension_type, 2 | 5);
    // The drawing records an angle in radians.
    let measurement = number(42).map(|value| {
        if angular {
            value.to_degrees()
        } else {
            value * scale
        }
    });
    let points_mm = [10_u8, 11, 13, 14, 15, 16]
        .into_iter()
        .filter_map(|code| {
            let x = number(i16::from(code))?;
            let y = number(i16::from(code) + 10)?;
            Some((code, [x * scale, y * scale]))
        })
        .collect();
    ObjectDetail::Dimension {
        dimension_type,
        measurement,
        text_override: parse_optional_text(record, 1)
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_owned(),
        points_mm,
    }
}
