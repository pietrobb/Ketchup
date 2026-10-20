//! Underlays: drawings shown under the model to trace, snap to and measure.
//!
//! An underlay is no part. Nothing that makes, counts, checks or exports parts sees it: it
//! is no occurrence, so cut lists, hardware, takeoffs, collisions, masses, production
//! exports and validators never meet it. The document keeps its name, its layers and the
//! tags that show them; its lines are one blob of the document's container
//! ([`UnderlayLines::encode`]), shared by every revision, so an Undo history does not copy
//! them.

use std::collections::BTreeSet;
use std::fmt;

use ketchup_geometry::typed_id;

use crate::document::{CanonicalCommand, CommandBatch, ProfileSegment, Snapshot, TagId};

typed_id!(UnderlayId);

/// A drawing under the model, shown while its tag and the tag of a layer are visible.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Underlay {
    pub id: UnderlayId,
    pub name: String,
    /// Shows or hides the whole underlay.
    pub tag: TagId,
    /// In the order of the layers of its lines.
    pub layers: Vec<UnderlayLayer>,
    /// SHA-256 of [`UnderlayLines::encode`], a blob of the document's container.
    pub lines_sha256: [u8; 32],
}

impl Underlay {
    /// Every tag the underlay holds: its own and its layers'.
    pub fn tags(&self) -> impl Iterator<Item = TagId> + '_ {
        std::iter::once(self.tag).chain(self.layers.iter().map(|layer| layer.tag))
    }

    /// The hex name of the container blob that holds its lines.
    #[must_use]
    pub fn lines_blob(&self) -> String {
        self.lines_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// One layer of the drawing, shown while its own tag is visible.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnderlayLayer {
    pub name: String,
    pub tag: TagId,
}

/// The lines of an underlay: per layer, polylines in the XY plane in millimetres.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnderlayLines {
    layers: Vec<LayerLines>,
}

/// The polylines of one layer: their points one after another, and where each ends.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerLines {
    points_mm: Vec<[f64; 2]>,
    /// The index in `points_mm` one past the last point of each polyline.
    ends: Vec<usize>,
}

impl LayerLines {
    pub fn polylines(&self) -> impl Iterator<Item = &[[f64; 2]]> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts
            .zip(self.ends.iter().copied())
            .map(|(start, end)| &self.points_mm[start..end])
    }

    /// Every straight piece of every polyline.
    pub fn segments(&self) -> impl Iterator<Item = [[f64; 2]; 2]> + '_ {
        self.polylines()
            .flat_map(|points| points.windows(2).map(|pair| [pair[0], pair[1]]))
    }

    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.points_mm.len() - self.ends.len()
    }
}

const MAGIC: &[u8; 8] = b"KUNDRLY1";

/// Why the bytes of a container blob are not underlay lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnderlayLinesError {
    /// The bytes do not start as underlay lines do.
    NotUnderlayLines,
    /// The bytes end before the counts they declare.
    Truncated,
    /// Bytes are left over after the last layer.
    TrailingBytes,
    /// A polyline of fewer than two points, or ends out of order.
    InvalidPolyline,
    /// A coordinate that is not a finite number.
    NotFinite,
    /// The blob holds another number of layers than the underlay names.
    LayerCount { blob: usize, underlay: usize },
}

impl fmt::Display for UnderlayLinesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUnderlayLines => formatter.write_str("the blob is not underlay lines"),
            Self::Truncated => formatter.write_str("the underlay lines end too early"),
            Self::TrailingBytes => formatter.write_str("bytes follow the last underlay layer"),
            Self::InvalidPolyline => {
                formatter.write_str("an underlay polyline has fewer than two points")
            }
            Self::NotFinite => formatter.write_str("an underlay point is not a finite number"),
            Self::LayerCount { blob, underlay } => write!(
                formatter,
                "the underlay lines hold {blob} layers, the underlay names {underlay}"
            ),
        }
    }
}

impl std::error::Error for UnderlayLinesError {}

impl UnderlayLines {
    pub fn layers(&self) -> &[LayerLines] {
        &self.layers
    }

    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.layers.iter().map(LayerLines::segment_count).sum()
    }

    /// The smallest and largest corner of every point, or `None` without points.
    #[must_use]
    pub fn bounds_mm(&self) -> Option<[[f64; 2]; 2]> {
        let mut points = self.layers.iter().flat_map(|layer| &layer.points_mm);
        let first = *points.next()?;
        Some(points.fold([first, first], |[low, high], point| {
            [
                [low[0].min(point[0]), low[1].min(point[1])],
                [high[0].max(point[0]), high[1].max(point[1])],
            ]
        }))
    }

    /// The bytes of the container blob: little-endian counts, ends and coordinates.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let capacity = MAGIC.len()
            + 4
            + self
                .layers
                .iter()
                .map(|layer| 8 + layer.ends.len() * 4 + layer.points_mm.len() * 16)
                .sum::<usize>();
        let mut bytes = Vec::with_capacity(capacity);
        bytes.extend_from_slice(MAGIC);
        push_count(&mut bytes, self.layers.len());
        for layer in &self.layers {
            push_count(&mut bytes, layer.ends.len());
            push_count(&mut bytes, layer.points_mm.len());
            for end in &layer.ends {
                push_count(&mut bytes, *end);
            }
            for point in &layer.points_mm {
                bytes.extend_from_slice(&point[0].to_le_bytes());
                bytes.extend_from_slice(&point[1].to_le_bytes());
            }
        }
        bytes
    }

    /// Reads what [`Self::encode`] wrote; every count is checked against the bytes
    /// before anything is allocated for it.
    pub fn decode(bytes: &[u8]) -> Result<Self, UnderlayLinesError> {
        let mut reader = Reader(
            bytes
                .strip_prefix(MAGIC.as_slice())
                .ok_or(UnderlayLinesError::NotUnderlayLines)?,
        );
        let layer_count = reader.count()?;
        let mut layers = Vec::new();
        for _ in 0..layer_count {
            let polyline_count = reader.count()?;
            let point_count = reader.count()?;
            let ends = (0..polyline_count)
                .map(|_| reader.count())
                .collect::<Result<Vec<_>, _>>()?;
            let points_mm = reader
                .take(point_count, 16)?
                .chunks_exact(16)
                .map(|point| [f64_at(&point[..8]), f64_at(&point[8..])])
                .collect::<Vec<_>>();
            let mut start = 0;
            for end in &ends {
                if *end < start + 2 {
                    return Err(UnderlayLinesError::InvalidPolyline);
                }
                start = *end;
            }
            if start != points_mm.len() {
                return Err(UnderlayLinesError::InvalidPolyline);
            }
            if points_mm.iter().flatten().any(|value| !value.is_finite()) {
                return Err(UnderlayLinesError::NotFinite);
            }
            layers.push(LayerLines { points_mm, ends });
        }
        if !reader.0.is_empty() {
            return Err(UnderlayLinesError::TrailingBytes);
        }
        Ok(Self { layers })
    }
}

fn push_count(bytes: &mut Vec<u8>, count: usize) {
    let count = u32::try_from(count).expect("an underlay count fits the blob's u32");
    bytes.extend_from_slice(&count.to_le_bytes());
}

fn f64_at(bytes: &[u8]) -> f64 {
    let mut array = [0; 8];
    array.copy_from_slice(bytes);
    f64::from_le_bytes(array)
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn count(&mut self) -> Result<usize, UnderlayLinesError> {
        let bytes = self.take(1, 4)?;
        usize::try_from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .map_err(|_: std::num::TryFromIntError| UnderlayLinesError::Truncated)
    }

    fn take(&mut self, count: usize, size: usize) -> Result<&'a [u8], UnderlayLinesError> {
        let length = count
            .checked_mul(size)
            .filter(|length| *length <= self.0.len())
            .ok_or(UnderlayLinesError::Truncated)?;
        let (taken, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(taken)
    }
}

/// Collects drawing segments into the polylines of named layers.
#[derive(Default)]
pub struct UnderlayLinesBuilder {
    names: Vec<String>,
    layers: Vec<LayerLines>,
    /// Per layer, the loose lines and arcs [`Self::finish`] joins into polylines.
    loose: Vec<LayerLines>,
}

/// The largest distance between an arc and the chords drawn for it.
const ARC_CHORD_TOLERANCE_MM: f64 = 0.1;
/// Chords of a whole circle at most, however large it is.
const MAX_CIRCLE_CHORDS: f64 = 1024.0;
/// Chords of one cubic Bézier piece.
const BEZIER_CHORDS: u32 = 16;

impl UnderlayLinesBuilder {
    /// Adds one connected run of `segments` to `layer` as one polyline.
    pub fn add(&mut self, layer: &str, segments: &[ProfileSegment]) {
        let index = self.layer_index(layer);
        push_polyline(&mut self.layers[index], segments);
    }

    /// Adds a loose line or arc of `layer`, joined with the others it shares an end with.
    pub fn add_loose(&mut self, layer: &str, segment: &ProfileSegment) {
        let index = self.layer_index(layer);
        push_polyline(&mut self.loose[index], std::slice::from_ref(segment));
    }

    fn layer_index(&mut self, layer: &str) -> usize {
        if let Some(index) = self.names.iter().position(|name| name == layer) {
            return index;
        }
        self.names.push(layer.to_owned());
        self.layers.push(LayerLines::default());
        self.loose.push(LayerLines::default());
        self.layers.len() - 1
    }

    /// The layer names in the order of the lines' layers, and the lines.
    #[must_use]
    pub fn finish(mut self) -> (Vec<String>, UnderlayLines) {
        for (lines, loose) in self.layers.iter_mut().zip(&self.loose) {
            join_loose(loose, lines);
        }
        (
            self.names,
            UnderlayLines {
                layers: self.layers,
            },
        )
    }
}

/// Appends `segments`, one connected run, to `lines` as one polyline.
fn push_polyline(lines: &mut LayerLines, segments: &[ProfileSegment]) {
    let Some(first) = segments.first() else {
        return;
    };
    let start = lines.points_mm.len();
    lines.points_mm.push(first.start_mm());
    for segment in segments {
        if lines.points_mm.last() != Some(&segment.start_mm()) {
            lines.points_mm.push(segment.start_mm());
        }
        push_segment(&mut lines.points_mm, segment);
    }
    if lines.points_mm.len() - start < 2 {
        lines.points_mm.truncate(start);
        return;
    }
    lines.ends.push(lines.points_mm.len());
}

/// Appends the polylines of `loose` to `lines`, each run of them that meets end to end at
/// points no third one touches joined into one polyline. Points meet when they are equal.
fn join_loose(loose: &LayerLines, lines: &mut LayerLines) {
    let pieces = loose.polylines().collect::<Vec<_>>();
    let key = |point: [f64; 2]| [point[0].to_bits(), point[1].to_bits()];
    // At each point, the pieces that end there: 2 × piece, + 1 for its last point.
    let mut ends_at = std::collections::HashMap::<[u64; 2], Vec<usize>>::new();
    for (index, piece) in pieces.iter().enumerate() {
        ends_at.entry(key(piece[0])).or_default().push(2 * index);
        ends_at
            .entry(key(piece[piece.len() - 1]))
            .or_default()
            .push(2 * index + 1);
    }
    let mut used = vec![false; pieces.len()];
    let walk = |first: usize, used: &mut [bool], lines: &mut LayerLines| {
        let mut end = first;
        used[end / 2] = true;
        push_oriented(&mut lines.points_mm, pieces[end / 2], end % 2 == 1, false);
        loop {
            let tail = pieces[end / 2][if end.is_multiple_of(2) {
                pieces[end / 2].len() - 1
            } else {
                0
            }];
            let meeting = &ends_at[&key(tail)];
            if meeting.len() != 2 {
                break;
            }
            let Some(&next) = meeting.iter().find(|next| !used[**next / 2]) else {
                break;
            };
            used[next / 2] = true;
            push_oriented(&mut lines.points_mm, pieces[next / 2], next % 2 == 1, true);
            end = next;
        }
        lines.ends.push(lines.points_mm.len());
    };
    // Runs start where no two pieces meet alone; what is left are closed rings.
    for index in 0..pieces.len() {
        for end in [2 * index, 2 * index + 1] {
            let point = pieces[index][if end.is_multiple_of(2) {
                0
            } else {
                pieces[index].len() - 1
            }];
            if !used[index] && ends_at[&key(point)].len() != 2 {
                walk(end, &mut used, lines);
            }
        }
    }
    for index in 0..pieces.len() {
        if !used[index] {
            walk(2 * index, &mut used, lines);
        }
    }
}

/// Appends `piece`, from its last point back if `reversed`, without its first point if
/// `joined`, since the run already ends there.
fn push_oriented(points: &mut Vec<[f64; 2]>, piece: &[[f64; 2]], reversed: bool, joined: bool) {
    let skip = usize::from(joined);
    if reversed {
        points.extend(piece.iter().rev().skip(skip));
    } else {
        points.extend(piece.iter().skip(skip));
    }
}

/// Appends the points after the start of `segment`.
fn push_segment(points: &mut Vec<[f64; 2]>, segment: &ProfileSegment) {
    match segment {
        ProfileSegment::Line { end_mm, .. } => points.push(*end_mm),
        ProfileSegment::CircularArc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => {
            let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
            let start = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
            let end = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
            let tau = std::f64::consts::TAU;
            let mut sweep = if *clockwise {
                -(start - end).rem_euclid(tau)
            } else {
                (end - start).rem_euclid(tau)
            };
            if sweep == 0.0 {
                sweep = if *clockwise { -tau } else { tau };
            }
            let step = if radius > ARC_CHORD_TOLERANCE_MM {
                2.0 * (1.0 - ARC_CHORD_TOLERANCE_MM / radius).acos()
            } else {
                tau
            };
            let chords = (sweep.abs() / step.max(tau / MAX_CIRCLE_CHORDS)).ceil();
            let mut index = 1.0;
            while index < chords {
                let angle = start + sweep * index / chords;
                points.push([
                    center_mm[0] + radius * angle.cos(),
                    center_mm[1] + radius * angle.sin(),
                ]);
                index += 1.0;
            }
            points.push(*end_mm);
        }
        ProfileSegment::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
        } => {
            for index in 1..BEZIER_CHORDS {
                let t = f64::from(index) / f64::from(BEZIER_CHORDS);
                let u = 1.0 - t;
                let weights = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                let controls = [start_mm, control_1_mm, control_2_mm, end_mm];
                points.push([0, 1].map(|axis| {
                    weights
                        .iter()
                        .zip(controls)
                        .map(|(weight, control)| weight * control[axis])
                        .sum()
                }));
            }
            points.push(*end_mm);
        }
        ProfileSegment::Spline { points_mm } => points.extend(points_mm.iter().skip(1)),
    }
}

/// Why an underlay cannot be added or removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnderlayPlanError {
    /// No tag or underlay id is left.
    IdSpaceExhausted,
    /// The document has no underlay of this id.
    NotFound(UnderlayId),
}

impl fmt::Display for UnderlayPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdSpaceExhausted => formatter.write_str("no tag or underlay id is left"),
            Self::NotFound(id) => write!(formatter, "the document has no underlay {}", id.0),
        }
    }
}

impl std::error::Error for UnderlayPlanError {}

/// One batch, so one Undo step, that adds an underlay named `name` with a tag of its own
/// and one per layer, its lines being the container blob `lines_sha256`.
pub fn plan_underlay(
    snapshot: &Snapshot,
    name: &str,
    layer_names: &[String],
    lines_sha256: [u8; 32],
) -> Result<(CommandBatch, UnderlayId), UnderlayPlanError> {
    let next = |ids: &mut dyn Iterator<Item = u64>| {
        ids.max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(UnderlayPlanError::IdSpaceExhausted)
    };
    let id = UnderlayId(next(
        &mut snapshot.underlays().map(|underlay| underlay.id.0),
    )?);
    let mut next_tag = next(&mut snapshot.tags().map(|tag| tag.id().0))?;
    let mut new_tag = |commands: &mut Vec<CanonicalCommand>, name: String| {
        let id = TagId(next_tag);
        next_tag = next_tag
            .checked_add(1)
            .ok_or(UnderlayPlanError::IdSpaceExhausted)?;
        commands.push(CanonicalCommand::CreateTag {
            id,
            name,
            visible: true,
        });
        Ok(id)
    };
    let mut commands = Vec::with_capacity(layer_names.len() + 2);
    let tag = new_tag(&mut commands, name.to_owned())?;
    let layers = layer_names
        .iter()
        .map(|layer| {
            Ok(UnderlayLayer {
                name: layer.clone(),
                tag: new_tag(&mut commands, format!("{name} · {layer}"))?,
            })
        })
        .collect::<Result<Vec<_>, UnderlayPlanError>>()?;
    commands.push(CanonicalCommand::UpsertUnderlay(Underlay {
        id,
        name: name.to_owned(),
        tag,
        layers,
        lines_sha256,
    }));
    Ok((CommandBatch::new(commands), id))
}

/// One batch, so one Undo step, that removes the underlay `id` and the tags it alone held.
pub fn plan_underlay_removal(
    snapshot: &Snapshot,
    id: UnderlayId,
) -> Result<CommandBatch, UnderlayPlanError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayPlanError::NotFound(id))?;
    let held_elsewhere = snapshot
        .occurrences()
        .flat_map(|occurrence| occurrence.tags().iter().copied())
        .chain(
            snapshot
                .underlays()
                .filter(|other| other.id != id)
                .flat_map(|other| other.tags().collect::<Vec<_>>()),
        )
        .collect::<BTreeSet<_>>();
    let mut commands = vec![CanonicalCommand::DeleteUnderlay { id }];
    commands.extend(
        underlay
            .tags()
            .filter(|tag| !held_elsewhere.contains(tag))
            .map(|id| CanonicalCommand::DeleteTag { id }),
    );
    Ok(CommandBatch::new(commands))
}
