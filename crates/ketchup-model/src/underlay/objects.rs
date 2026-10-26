//! The objects of an underlay's drawing: each entity it draws, with its kind, handle and
//! layer, the insert or dimension that placed it, and where its lines, texts and fills lie
//! in the underlay's layers. An insert keeps the block it places and where, a dimension
//! its measurement, text and points, a hatch its pattern and outline. Nothing here is
//! drawn or snapped to: the layers are; the objects say what their lines are.

use std::ops::Range;

use super::{Reader, UnderlayLinesError, f64_at, push_count};

/// The kinds of entity an object can be, by their DXF names.
const KINDS: [&str; 21] = [
    "LINE",
    "ARC",
    "CIRCLE",
    "ELLIPSE",
    "SPLINE",
    "LWPOLYLINE",
    "POLYLINE",
    "TEXT",
    "MTEXT",
    "ATTDEF",
    "ATTRIB",
    "HATCH",
    "INSERT",
    "DIMENSION",
    "SOLID",
    "TRACE",
    "3DFACE",
    "LEADER",
    "MESH",
    "MPOLYGON",
    "POINT",
];

/// The kind of entity an object is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectKind(u8);

impl ObjectKind {
    /// The kind of the DXF entity `name` (any case), or `None` for one no object is kept
    /// of.
    #[must_use]
    pub fn from_dxf(name: &str) -> Option<Self> {
        KINDS
            .iter()
            .position(|kind| kind.eq_ignore_ascii_case(name))
            .and_then(|index| u8::try_from(index).ok())
            .map(Self)
    }

    /// Its DXF name, upper case.
    #[must_use]
    pub fn name(self) -> &'static str {
        KINDS[usize::from(self.0)]
    }
}

/// A map of the plane onto itself that keeps lines straight: a point `[x, y]` goes to
/// `[m[0][0] x + m[0][1] y + m[0][2], m[1][0] x + m[1][1] y + m[1][2]]`.
pub type Affine = [[f64; 3]; 2];

#[must_use]
pub fn apply(map: &Affine, point: [f64; 2]) -> [f64; 2] {
    [0, 1].map(|row| map[row][0] * point[0] + map[row][1] * point[1] + map[row][2])
}

/// `inner` first, then `outer`.
#[must_use]
pub fn compose(outer: &Affine, inner: &Affine) -> Affine {
    [0, 1].map(|row| {
        [
            outer[row][0] * inner[0][0] + outer[row][1] * inner[1][0],
            outer[row][0] * inner[0][1] + outer[row][1] * inner[1][1],
            outer[row][0] * inner[0][2] + outer[row][1] * inner[1][2] + outer[row][2],
        ]
    })
}

/// Where some of an object's lines, texts and fills lie in one layer of the underlay:
/// ranges of that layer's points ([`super::LayerLines::points`]), texts and fills. A
/// range of points spans whole polylines, or the run of one polyline the object drew.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectPart {
    /// The index of the layer in [`super::UnderlayLines::layers`].
    pub layer: u32,
    pub points: Range<u32>,
    pub texts: Range<u32>,
    pub fills: Range<u32>,
}

impl ObjectPart {
    /// Takes `other` in when it lies on the same layer right after or over this part.
    fn absorb(&mut self, other: &Self) -> bool {
        fn joins(own: &Range<u32>, other: &Range<u32>) -> bool {
            own.is_empty()
                || other.is_empty()
                || (other.start >= own.start && other.start <= own.end)
        }
        fn union(own: &mut Range<u32>, other: &Range<u32>) {
            if Range::is_empty(own) {
                own.clone_from(other);
            } else if !other.is_empty() {
                own.end = own.end.max(other.end);
            }
        }
        let fits = self.layer == other.layer
            && joins(&self.points, &other.points)
            && joins(&self.texts, &other.texts)
            && joins(&self.fills, &other.fills);
        if fits {
            union(&mut self.points, &other.points);
            union(&mut self.texts, &other.texts);
            union(&mut self.fills, &other.fills);
        }
        fits
    }
}

/// What an object holds beyond its kind.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ObjectDetail {
    #[default]
    None,
    /// A placement of a block; the objects it places are its children.
    Insert {
        /// The block's name as the drawing writes it.
        block: String,
        /// From the block's coordinates to the underlay's, in millimetres.
        transform: Affine,
        /// Columns and rows; new imports expand arrays into separate `[1, 1]` objects.
        copies: [u32; 2],
        /// Tag and value of each attribute, in the drawing's order.
        attributes: Vec<(String, String)>,
    },
    /// A dimension; the lines, arrows and text it draws are its children.
    Dimension {
        /// Its type, group 70 of the drawing without its flags: 0 rotated, 1 aligned,
        /// 2 angular, 3 diameter, 4 radius, 5 angular by three points, 6 ordinate.
        dimension_type: u8,
        /// What it measures as the drawing records it: millimetres, degrees for an
        /// angular one; `None` when the drawing records none.
        measurement: Option<f64>,
        /// The text it was given instead of its measurement, empty for none; `<>` in it
        /// stands for the measurement.
        text_override: String,
        /// Its definition points by DXF group: 10 on the dimension line, 11 the middle of
        /// the text, 13 and 14 where the extension lines start, 15 and 16 the points of
        /// an arc, radius or diameter.
        points_mm: Vec<(u8, [f64; 2])>,
    },
    /// A hatch: the area inside an odd number of its rings, painted solid or drawn in a
    /// pattern.
    Hatch {
        /// The pattern's name, `SOLID` for a solid fill.
        pattern: String,
        solid: bool,
        rings: Vec<Vec<[f64; 2]>>,
    },
}

impl ObjectDetail {
    /// The same detail with each of its points mapped by `map`.
    #[must_use]
    pub fn transformed(&self, map: &Affine) -> Self {
        match self {
            Self::None => Self::None,
            Self::Insert {
                block,
                transform,
                copies,
                attributes,
            } => Self::Insert {
                block: block.clone(),
                transform: compose(map, transform),
                copies: *copies,
                attributes: attributes.clone(),
            },
            Self::Dimension {
                dimension_type,
                measurement,
                text_override,
                points_mm,
            } => Self::Dimension {
                dimension_type: *dimension_type,
                measurement: *measurement,
                text_override: text_override.clone(),
                points_mm: points_mm
                    .iter()
                    .map(|(code, point)| (*code, apply(map, *point)))
                    .collect(),
            },
            Self::Hatch {
                pattern,
                solid,
                rings,
            } => Self::Hatch {
                pattern: pattern.clone(),
                solid: *solid,
                rings: rings
                    .iter()
                    .map(|ring| ring.iter().map(|point| apply(map, *point)).collect())
                    .collect(),
            },
        }
    }

    /// Whether every number it holds is finite.
    fn is_finite(&self) -> bool {
        match self {
            Self::None => true,
            Self::Insert { transform, .. } => {
                transform.iter().flatten().all(|value| value.is_finite())
            }
            Self::Dimension {
                measurement,
                points_mm,
                ..
            } => {
                measurement.is_none_or(f64::is_finite)
                    && points_mm
                        .iter()
                        .flat_map(|(_, point)| point)
                        .all(|value| value.is_finite())
            }
            Self::Hatch { rings, .. } => rings
                .iter()
                .flatten()
                .flatten()
                .all(|value| value.is_finite()),
        }
    }
}

/// The area inside an odd number of `rings`, in square millimetres: each ring's own area,
/// added when it lies inside an even number of the others and taken away when inside an
/// odd number.
#[must_use]
pub fn even_odd_area(rings: &[Vec<[f64; 2]>]) -> f64 {
    let own = |ring: &[[f64; 2]]| {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
            .sum::<f64>()
            .abs()
            / 2.0
    };
    rings
        .iter()
        .enumerate()
        .filter(|(_, ring)| ring.len() >= 3)
        .map(|(index, ring)| {
            let depth = rings
                .iter()
                .enumerate()
                .filter(|(other, around)| {
                    *other != index && around.len() >= 3 && ring_contains(around, ring[0])
                })
                .count();
            if depth.is_multiple_of(2) {
                own(ring)
            } else {
                -own(ring)
            }
        })
        .sum()
}

/// Whether `point` lies inside `ring` (even-odd).
#[must_use]
pub fn ring_contains(ring: &[[f64; 2]], point: [f64; 2]) -> bool {
    let mut inside = false;
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < a[0] + (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1])
        {
            inside = !inside;
        }
    }
    inside
}

/// One object of the drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct UnderlayObject {
    pub kind: ObjectKind,
    /// Its handle in the drawing (group 5, hexadecimal there), 0 for none; of an object
    /// an insert placed, the handle of the entity in the block.
    pub handle: u64,
    /// The index in [`UnderlayObjects::layers`] of the layer the drawing puts it on.
    pub layer: u32,
    /// The insert or dimension that placed it, always an object before it.
    pub parent: Option<u32>,
    /// Where what it draws itself lies; what its children draw is theirs.
    pub parts: Vec<ObjectPart>,
    pub detail: ObjectDetail,
    /// Taken out of the drawing in Kečup ([`super::edits`]): it holds nothing and no
    /// question finds it. Never saved in the blob.
    pub removed: bool,
    /// The index in [`super::edits::UnderlayEdits::added`] of the edit that added it;
    /// `None` for an object read from the drawing. Never saved in the blob.
    pub added_by: Option<u32>,
}

impl UnderlayObject {
    /// Adds `part` to where it lies, joined to its last part where they meet.
    pub(super) fn add_part(&mut self, part: ObjectPart) {
        if self.parts.last_mut().is_some_and(|last| last.absorb(&part)) {
            return;
        }
        self.parts.push(part);
    }
}

/// The objects of an underlay and the names of their layers; empty for an underlay saved
/// before objects were kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnderlayObjects {
    /// Every layer an object is on, those that draw nothing of their own too.
    pub layers: Vec<String>,
    pub objects: Vec<UnderlayObject>,
}

/// For one layer of the underlay, where each point, text and fill went when the layer was
/// rebuilt, `u32::MAX` for one left out.
#[derive(Default)]
pub(super) struct LayerMaps {
    pub(super) points: Vec<u32>,
    pub(super) texts: Vec<u32>,
    pub(super) fills: Vec<u32>,
}

impl UnderlayObjects {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// The name of the layer of `object`.
    #[must_use]
    pub fn layer_name(&self, object: &UnderlayObject) -> &str {
        usize::try_from(object.layer)
            .ok()
            .and_then(|index| self.layers.get(index))
            .map_or("", String::as_str)
    }

    /// The index of the layer `name` among [`Self::layers`], added if new.
    pub(super) fn layer_index(&mut self, name: &str) -> u32 {
        let index = match self.layers.iter().position(|layer| layer == name) {
            Some(index) => index,
            None => {
                self.layers.push(name.to_owned());
                self.layers.len() - 1
            }
        };
        u32::try_from(index).expect("an underlay count fits the blob's u32")
    }

    /// Moves every point of the objects' details by `map`.
    pub(super) fn transform(&mut self, map: &Affine) {
        for object in &mut self.objects {
            object.detail = object.detail.transformed(map);
        }
    }

    /// Points each part at where its points, texts and fills went (`maps`, by layer);
    /// what was left out of a part is left out of it.
    pub(super) fn remap(&mut self, maps: &[LayerMaps]) {
        fn moved(range: &Range<u32>, map: &[u32]) -> Range<u32> {
            let kept = map
                .get(index(range.start)..index(range.end).min(map.len()))
                .unwrap_or_default()
                .iter()
                .filter(|new| **new != u32::MAX);
            let (mut first, mut last) = (None, None);
            for new in kept {
                first = first.or(Some(*new));
                last = Some(*new);
            }
            match (first, last) {
                (Some(first), Some(last)) => first..last + 1,
                _ => 0..0,
            }
        }
        for object in &mut self.objects {
            for part in &mut object.parts {
                let Some(map) = maps.get(index(part.layer)) else {
                    continue;
                };
                part.points = moved(&part.points, &map.points);
                part.texts = moved(&part.texts, &map.texts);
                part.fills = moved(&part.fills, &map.fills);
            }
            object.parts.retain(|part| {
                !(part.points.is_empty() && part.texts.is_empty() && part.fills.is_empty())
            });
        }
    }

    pub(super) fn encode(&self, bytes: &mut Vec<u8>) {
        push_count(bytes, self.layers.len());
        for layer in &self.layers {
            push_text(bytes, layer);
        }
        push_count(bytes, self.objects.len());
        for object in &self.objects {
            bytes.push(object.kind.0);
            bytes.extend_from_slice(&object.handle.to_le_bytes());
            bytes.extend_from_slice(&object.layer.to_le_bytes());
            bytes.extend_from_slice(&object.parent.unwrap_or(u32::MAX).to_le_bytes());
            push_count(bytes, object.parts.len());
            for part in &object.parts {
                for value in [
                    part.layer,
                    part.points.start,
                    part.points.end,
                    part.texts.start,
                    part.texts.end,
                    part.fills.start,
                    part.fills.end,
                ] {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            encode_detail(&object.detail, bytes);
        }
    }

    /// Reads what [`Self::encode`] wrote for an underlay whose layers hold `sizes`
    /// points, texts and fills each.
    pub(super) fn decode(
        reader: &mut Reader<'_>,
        sizes: &[[usize; 3]],
    ) -> Result<Self, UnderlayLinesError> {
        let mut layers = Vec::new();
        for _ in 0..reader.count()? {
            layers.push(read_text(reader)?);
        }
        let mut objects = Vec::new();
        for own in 0..reader.count()? {
            let kind = take_u8(reader)?;
            let mut handle = [0; 8];
            handle.copy_from_slice(reader.take(1, 8)?);
            let handle = u64::from_le_bytes(handle);
            let layer = take_u32(reader)?;
            let parent = take_u32(reader)?;
            let invalid = usize::from(kind) >= KINDS.len()
                || index(layer) >= layers.len()
                || (parent != u32::MAX && index(parent) >= own);
            if invalid {
                return Err(UnderlayLinesError::InvalidObject);
            }
            let mut parts = Vec::new();
            for _ in 0..reader.count()? {
                let values = [(); 7].map(|()| take_u32(reader));
                let [layer, p0, p1, t0, t1, f0, f1] = values;
                let part = ObjectPart {
                    layer: layer?,
                    points: p0?..p1?,
                    texts: t0?..t1?,
                    fills: f0?..f1?,
                };
                let size = sizes
                    .get(index(part.layer))
                    .ok_or(UnderlayLinesError::InvalidObject)?;
                let within = |range: &Range<u32>, size: usize| {
                    range.start <= range.end && index(range.end) <= size
                };
                if !(within(&part.points, size[0])
                    && within(&part.texts, size[1])
                    && within(&part.fills, size[2]))
                {
                    return Err(UnderlayLinesError::InvalidObject);
                }
                parts.push(part);
            }
            let detail = decode_detail(reader)?;
            if !detail.is_finite() {
                return Err(UnderlayLinesError::NotFinite);
            }
            objects.push(UnderlayObject {
                kind: ObjectKind(kind),
                handle,
                layer,
                parent: (parent != u32::MAX).then_some(parent),
                parts,
                detail,
                removed: false,
                added_by: None,
            });
        }
        Ok(Self { layers, objects })
    }
}

fn index(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn push_text(bytes: &mut Vec<u8>, text: &str) {
    push_count(bytes, text.len());
    bytes.extend_from_slice(text.as_bytes());
}

fn push_point(bytes: &mut Vec<u8>, point: [f64; 2]) {
    bytes.extend_from_slice(&point[0].to_le_bytes());
    bytes.extend_from_slice(&point[1].to_le_bytes());
}

fn read_text(reader: &mut Reader<'_>) -> Result<String, UnderlayLinesError> {
    let length = reader.count()?;
    Ok(std::str::from_utf8(reader.take(length, 1)?)
        .map_err(UnderlayLinesError::TextNotUtf8)?
        .to_owned())
}

fn take_u8(reader: &mut Reader<'_>) -> Result<u8, UnderlayLinesError> {
    Ok(reader.take(1, 1)?[0])
}

fn take_u32(reader: &mut Reader<'_>) -> Result<u32, UnderlayLinesError> {
    let bytes = reader.take(1, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn take_f64(reader: &mut Reader<'_>) -> Result<f64, UnderlayLinesError> {
    Ok(f64_at(reader.take(1, 8)?))
}

fn take_point(reader: &mut Reader<'_>) -> Result<[f64; 2], UnderlayLinesError> {
    Ok([take_f64(reader)?, take_f64(reader)?])
}

fn encode_detail(detail: &ObjectDetail, bytes: &mut Vec<u8>) {
    match detail {
        ObjectDetail::None => bytes.push(0),
        ObjectDetail::Insert {
            block,
            transform,
            copies,
            attributes,
        } => {
            bytes.push(1);
            push_text(bytes, block);
            for value in transform.iter().flatten() {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for copy in copies {
                bytes.extend_from_slice(&copy.to_le_bytes());
            }
            push_count(bytes, attributes.len());
            for (tag, value) in attributes {
                push_text(bytes, tag);
                push_text(bytes, value);
            }
        }
        ObjectDetail::Dimension {
            dimension_type,
            measurement,
            text_override,
            points_mm,
        } => {
            bytes.push(2);
            bytes.push(*dimension_type);
            bytes.push(u8::from(measurement.is_some()));
            bytes.extend_from_slice(&measurement.unwrap_or(0.0).to_le_bytes());
            push_text(bytes, text_override);
            push_count(bytes, points_mm.len());
            for (code, point) in points_mm {
                bytes.push(*code);
                push_point(bytes, *point);
            }
        }
        ObjectDetail::Hatch {
            pattern,
            solid,
            rings,
        } => {
            bytes.push(3);
            push_text(bytes, pattern);
            bytes.push(u8::from(*solid));
            push_count(bytes, rings.len());
            for ring in rings {
                push_count(bytes, ring.len());
                for point in ring {
                    push_point(bytes, *point);
                }
            }
        }
    }
}

fn decode_detail(reader: &mut Reader<'_>) -> Result<ObjectDetail, UnderlayLinesError> {
    Ok(match take_u8(reader)? {
        0 => ObjectDetail::None,
        1 => {
            let block = read_text(reader)?;
            let mut values = [0.0; 6];
            for value in &mut values {
                *value = take_f64(reader)?;
            }
            let copies = [take_u32(reader)?, take_u32(reader)?];
            let mut attributes = Vec::new();
            for _ in 0..reader.count()? {
                attributes.push((read_text(reader)?, read_text(reader)?));
            }
            ObjectDetail::Insert {
                block,
                transform: [
                    [values[0], values[1], values[2]],
                    [values[3], values[4], values[5]],
                ],
                copies,
                attributes,
            }
        }
        2 => {
            let dimension_type = take_u8(reader)?;
            let measured = take_u8(reader)? != 0;
            let measurement = take_f64(reader)?;
            let text_override = read_text(reader)?;
            let mut points_mm = Vec::new();
            for _ in 0..reader.count()? {
                points_mm.push((take_u8(reader)?, take_point(reader)?));
            }
            ObjectDetail::Dimension {
                dimension_type,
                measurement: measured.then_some(measurement),
                text_override,
                points_mm,
            }
        }
        3 => {
            let pattern = read_text(reader)?;
            let solid = take_u8(reader)? != 0;
            let mut rings = Vec::new();
            for _ in 0..reader.count()? {
                // Pushed one by one: each point checks its own bytes before it is kept.
                let mut ring = Vec::new();
                for _ in 0..reader.count()? {
                    ring.push(take_point(reader)?);
                }
                rings.push(ring);
            }
            ObjectDetail::Hatch {
                pattern,
                solid,
                rings,
            }
        }
        _ => return Err(UnderlayLinesError::InvalidObject),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(low: f64, high: f64) -> Vec<[f64; 2]> {
        vec![[low, low], [high, low], [high, high], [low, high]]
    }

    #[test]
    fn a_hatch_area_takes_its_islands_away_and_adds_areas_in_them() {
        assert!((even_odd_area(&[square(0.0, 100.0)]) - 10_000.0).abs() < 1e-9);
        let island = [square(0.0, 100.0), square(30.0, 70.0)];
        assert!((even_odd_area(&island) - 8_400.0).abs() < 1e-9);
        let nested = [square(0.0, 100.0), square(30.0, 70.0), square(40.0, 60.0)];
        assert!((even_odd_area(&nested) - 8_800.0).abs() < 1e-9);
    }

    #[test]
    fn composed_maps_place_a_point_as_one_after_the_other() {
        let turn: Affine = [[0.0, -1.0, 10.0], [1.0, 0.0, 0.0]];
        let scale: Affine = [[2.0, 0.0, 0.0], [0.0, 2.0, 5.0]];
        let point = [3.0, 4.0];
        assert_eq!(
            apply(&compose(&turn, &scale), point),
            apply(&turn, apply(&scale, point))
        );
    }
}
