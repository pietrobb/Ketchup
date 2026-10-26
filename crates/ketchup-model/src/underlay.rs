//! Underlays: drawings shown under the model to trace, snap to and measure.
//!
//! An underlay is no part. Nothing that makes, counts, checks or exports parts sees it: it
//! is no occurrence, so cut lists, hardware, takeoffs, collisions, masses, production
//! exports and validators never meet it. The document keeps its name, its layers and the
//! tags that show them; its lines are one blob of the document's container
//! ([`UnderlayLines::encode`]), shared by every revision, so an Undo history does not copy
//! them. The texts of a layer travel with its lines; snapping only meets the lines. Each
//! polyline and text names the style it is drawn in ([`style::LineStyle`]). The areas a
//! drawing paints solid are fills of a layer ([`fill::UnderlayFill`]), drawn under its
//! lines. What the lines are — the entities, inserts, dimensions and hatches of the
//! drawing — its objects say ([`objects::UnderlayObjects`]).

pub mod drawn;
pub mod edits;
pub mod fill;
pub mod line_edit;
pub mod object_query;
pub mod objects;
pub mod placement;
pub mod regions;
pub mod snap;
pub mod style;
pub mod text;

use std::collections::BTreeSet;
use std::fmt;

use ketchup_geometry::typed_id;
use ketchup_tolerance::limits;

use crate::document::{CanonicalCommand, CommandBatch, ProfileSegment, Snapshot, TagId, Transform};
use crate::tolerance::{MAX_COORDINATE_MM, UNDERLAY_ARC_CHORD_MM};
use fill::UnderlayFill;
use objects::{LayerMaps, ObjectDetail, ObjectKind, ObjectPart, UnderlayObject, UnderlayObjects};
use style::LineStyle;
use text::UnderlayText;

typed_id!(UnderlayId);

/// A drawing under the model, shown while its tag and the tag of a layer are visible.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Underlay {
    pub id: UnderlayId,
    pub name: String,
    /// Shows or hides the whole underlay.
    pub tag: TagId,
    /// In the order of the layers of its lines.
    pub layers: Vec<UnderlayLayer>,
    /// SHA-256 of [`UnderlayLines::encode`], a blob of the document's container.
    pub lines_sha256: [u8; 32],
    /// The bits of the offset in millimetres its lines were moved by
    /// ([`UnderlayLines::move_near_origin`]); added back it gives the drawing's own
    /// coordinates.
    pub offset_mm_bits: [u64; 2],
    /// Where its lines lie in the document, as an occurrence's transform places its part:
    /// a turn about Z, a uniform scale and a move in the XY plane
    /// ([`placement::fit_two_points`]); none until it is placed.
    #[serde(
        default = "Transform::identity",
        skip_serializing_if = "placement::is_identity"
    )]
    pub placement: Transform,
    /// What was changed of its drawing in Kečup; its blob stays as it was read.
    #[serde(default, skip_serializing_if = "edits::UnderlayEdits::is_empty")]
    pub edits: edits::UnderlayEdits,
}

impl Underlay {
    /// Every tag the underlay holds: its own and its layers'.
    pub fn tags(&self) -> impl Iterator<Item = TagId> + '_ {
        std::iter::once(self.tag).chain(self.layers.iter().map(|layer| layer.tag))
    }

    #[must_use]
    pub fn offset_mm(&self) -> [f64; 2] {
        self.offset_mm_bits.map(f64::from_bits)
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

/// The lines of an underlay: per layer, polylines in the XY plane in millimetres; and the
/// styles they are drawn in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnderlayLines {
    layers: Vec<LayerLines>,
    styles: Vec<LineStyle>,
    objects: UnderlayObjects,
}

/// The polylines of one layer: their points one after another, where each ends and its
/// style; and the layer's texts and fills.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerLines {
    points_mm: Vec<[f64; 2]>,
    /// The index in `points_mm` one past the last point of each polyline.
    ends: Vec<usize>,
    /// The index of each polyline's style in [`UnderlayLines::style`].
    styles: Vec<u32>,
    texts: Vec<UnderlayText>,
    fills: Vec<UnderlayFill>,
}

impl LayerLines {
    pub fn texts(&self) -> &[UnderlayText] {
        &self.texts
    }

    /// The areas it paints solid, drawn under its lines.
    pub fn fills(&self) -> &[UnderlayFill] {
        &self.fills
    }

    /// Every polyline with the index of its style.
    pub fn styled_polylines(&self) -> impl Iterator<Item = (u32, &[[f64; 2]])> {
        self.styles.iter().copied().zip(self.polylines())
    }

    /// The points of its lines and fills and the anchors of its texts.
    fn points_and_anchors(&self) -> impl Iterator<Item = [f64; 2]> + '_ {
        let anchors = self.texts.iter().map(|text| text.anchor_mm);
        let fills = self.fills.iter().flat_map(|fill| fill.points_mm.iter());
        self.points_mm.iter().chain(fills).copied().chain(anchors)
    }

    /// The points of all its polylines, one after another.
    #[must_use]
    pub fn points_mm(&self) -> &[[f64; 2]] {
        &self.points_mm
    }

    /// The index in [`Self::points_mm`] one past the last point of each polyline.
    #[must_use]
    pub fn polyline_ends(&self) -> &[usize] {
        &self.ends
    }

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

    /// The smallest and largest corner of its points, its fills' and its text anchors, or
    /// `None` without any.
    #[must_use]
    pub fn bounds_mm(&self) -> Option<[[f64; 2]; 2]> {
        point_bounds(self.points_and_anchors())
    }
}

/// The smallest and largest corner of `points`, or `None` without points.
fn point_bounds(mut points: impl Iterator<Item = [f64; 2]>) -> Option<[[f64; 2]; 2]> {
    let first = points.next()?;
    Some(points.fold([first, first], |[low, high], point| {
        [
            [low[0].min(point[0]), low[1].min(point[1])],
            [high[0].max(point[0]), high[1].max(point[1])],
        ]
    }))
}

/// The versions of the blob, read all: lines only (1); each layer's texts after its points
/// (2); the styles first, and the style of each polyline and text (3); the bytes of 3
/// deflated after their length (4, written without fills); of 3 with each layer's
/// fills after its texts (5, written with fills and no objects); and of 5 with the
/// objects after the layers (6, written with objects).
const MAGICS: [&[u8; 8]; 6] = [
    b"KUNDRLY1",
    b"KUNDRLY2",
    b"KUNDRLY3",
    b"KUNDRLY4",
    b"KUNDRLY5",
    b"KUNDRLY6",
];

/// The deflate level of a written blob: the default, which keeps a 12 MB drawing to a
/// third of its bytes in about a third of a second.
const DEFLATE_LEVEL: u8 = 6;

/// How many times its deflated bytes a blob's body may be. Underlay lines deflate to
/// about a third; deflate itself allows about a thousand times, which a hostile blob
/// would use to ask for gigabytes.
const MAX_INFLATION: usize = 128;

fn inflates_plausibly(body: usize, deflated: usize) -> bool {
    body <= deflated.saturating_mul(MAX_INFLATION)
}

/// Why the bytes of a container blob are not underlay lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnderlayLinesError {
    /// The bytes do not start as underlay lines do.
    NotUnderlayLines,
    /// The deflated bytes cannot be inflated, or inflate past the length they declare.
    Inflate(miniz_oxide::inflate::TINFLStatus),
    /// The length declared is more than [`MAX_INFLATION`] times the deflated bytes.
    Inflated { length: usize, deflated: usize },
    /// The length declared, or that of lines to be written, is more than
    /// [`limits::UNDERLAY_LINES_BYTES`].
    TooLarge { length: usize },
    /// The bytes end before the counts they declare.
    Truncated,
    /// Bytes are left over after the last layer.
    TrailingBytes,
    /// A polyline of fewer than two points, or ends out of order.
    InvalidPolyline,
    /// A coordinate that is not a finite number.
    NotFinite,
    /// A text whose height, width or alignment cannot be drawn.
    InvalidText,
    /// A text whose bytes are not UTF-8.
    TextNotUtf8(std::str::Utf8Error),
    /// A style with no dash to draw, or a polyline or text naming no style.
    InvalidStyle,
    /// The blob holds another number of layers than the underlay names.
    LayerCount { blob: usize, underlay: usize },
    /// Filling it crosses edges more often than a drawing's entity budget allows.
    FillTooLarge(usize),
    /// An object of an unknown kind, on a layer or with a parent the blob does not hold,
    /// or pointing past the lines, texts or fills of its layer.
    InvalidObject,
}

impl fmt::Display for UnderlayLinesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUnderlayLines => formatter.write_str("the blob is not underlay lines"),
            Self::Inflate(status) => {
                write!(
                    formatter,
                    "the compressed underlay lines are damaged ({status:?})"
                )
            }
            Self::Inflated { length, deflated } => write!(
                formatter,
                "the underlay lines declare {length} bytes, more than {MAX_INFLATION} times their {deflated} compressed bytes; the file is damaged"
            ),
            Self::TooLarge { length } => write!(
                formatter,
                "the underlay lines take {length} bytes, more than the {} MiB an underlay may hold",
                limits::UNDERLAY_LINES_BYTES >> 20
            ),
            Self::Truncated => formatter.write_str("the underlay lines end too early"),
            Self::TrailingBytes => formatter.write_str("bytes follow the last underlay layer"),
            Self::InvalidPolyline => {
                formatter.write_str("an underlay polyline has fewer than two points")
            }
            Self::NotFinite => formatter.write_str("an underlay point is not a finite number"),
            Self::InvalidText => formatter
                .write_str("an underlay text has no height or width, or an unknown alignment"),
            Self::TextNotUtf8(error) => write!(formatter, "an underlay text is not UTF-8: {error}"),
            Self::InvalidStyle => {
                formatter.write_str("an underlay line style is invalid or names no style")
            }
            Self::LayerCount { blob, underlay } => write!(
                formatter,
                "the underlay lines hold {blob} layers, the underlay names {underlay}"
            ),
            Self::FillTooLarge(crossings) => write!(
                formatter,
                "filling the underlay crosses its edges {crossings} times, more than {}",
                limits::DRAWING_ENTITIES
            ),
            Self::InvalidObject => formatter.write_str(
                "an underlay object has an unknown kind, layer or parent, or points past its layer's lines",
            ),
        }
    }
}

impl std::error::Error for UnderlayLinesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TextNotUtf8(error) => Some(error),
            _ => None,
        }
    }
}

impl UnderlayLines {
    pub fn layers(&self) -> &[LayerLines] {
        &self.layers
    }

    /// The styles its polylines and texts name; an underlay saved before styles has none,
    /// and every index names [`Self::style`] continuous in its own color.
    pub fn styles(&self) -> &[LineStyle] {
        &self.styles
    }

    /// The entities, inserts, dimensions and hatches its lines, texts and fills were drawn
    /// by; none for an underlay saved before objects were kept.
    #[must_use]
    pub fn objects(&self) -> &UnderlayObjects {
        &self.objects
    }

    #[must_use]
    pub fn style(&self, index: u32) -> &LineStyle {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.styles.get(index))
            .unwrap_or(&style::CONTINUOUS)
    }

    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.layers.iter().map(LayerLines::segment_count).sum()
    }

    /// The layer and first point of straight piece `segment`, counted across the layers'
    /// polylines in order as [`snap::UnderlaySnapIndex`] counts them.
    #[must_use]
    pub fn segment_start(&self, mut segment: usize) -> Option<(usize, usize)> {
        for (index, layer) in self.layers.iter().enumerate() {
            let count = layer.segment_count();
            if segment >= count {
                segment -= count;
                continue;
            }
            // Polyline `p` ends at `ends[p]` and holds the pieces before `ends[p] - p - 1`.
            let (mut low, mut high) = (0, layer.ends.len());
            while low < high {
                let middle = (low + high) / 2;
                if layer.ends[middle] - middle - 1 <= segment {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            return Some((index, segment + low));
        }
        None
    }

    #[must_use]
    pub fn text_count(&self) -> usize {
        self.layers.iter().map(|layer| layer.texts.len()).sum()
    }

    #[must_use]
    pub fn fill_count(&self) -> usize {
        self.layers.iter().map(|layer| layer.fills.len()).sum()
    }

    /// The smallest and largest corner of every point of its lines and fills, or `None`
    /// without points.
    #[must_use]
    pub fn bounds_mm(&self) -> Option<[[f64; 2]; 2]> {
        point_bounds(self.layers.iter().flat_map(|layer| {
            let fills = layer.fills.iter().flat_map(|fill| fill.points_mm.iter());
            layer.points_mm.iter().chain(fills).copied()
        }))
    }

    /// Moves lines that leave the canonical ±1,000 m envelope near the origin, so that a
    /// drawing in national grid coordinates (S-JTSK: hundreds of kilometres) is kept
    /// instead of left out. The lines move by the median of their points, rounded to whole
    /// metres, so that a stray line far away does not decide where the drawing lands.
    /// Texts move with the lines, by their anchors, and count in the median too.
    /// Returns that offset, which added back gives the drawing's own coordinates, and what
    /// was left out because even moved it is more than [`UNDERLAY_EXTENT_MM`] away.
    pub fn move_near_origin(&mut self) -> MovedNearOrigin {
        let near = |point: &[f64; 2], extent: f64| point.iter().all(|value| value.abs() <= extent);
        let inside = |point: &[f64; 2]| near(point, UNDERLAY_EXTENT_MM);
        let points = LayerLines::points_and_anchors;
        if self
            .layers
            .iter()
            .flat_map(points)
            .all(|point| near(&point, MAX_COORDINATE_MM))
        {
            return MovedNearOrigin::default();
        }
        let offset_mm = [0, 1].map(|axis| {
            let mut values = self
                .layers
                .iter()
                .flat_map(|layer| points(layer).map(|point| point[axis]))
                .collect::<Vec<_>>();
            let middle = values.len() / 2;
            let (_, median, _) = values.select_nth_unstable_by(middle, f64::total_cmp);
            // `+ 0.0` makes a -0.0 of a median just below zero 0.0.
            (*median / 1000.0).round() * 1000.0 + 0.0
        });
        let before = (self.segment_count(), self.text_count(), self.fill_count());
        let mut maps = Vec::with_capacity(self.layers.len());
        for layer in &mut self.layers {
            let mut map = LayerMaps::default();
            let mut kept = LayerLines::default();
            for mut text in std::mem::take(&mut layer.texts) {
                text.anchor_mm = [0, 1].map(|axis| text.anchor_mm[axis] - offset_mm[axis]);
                map.texts
                    .push(kept_at(inside(&text.anchor_mm), kept.texts.len()));
                if inside(&text.anchor_mm) {
                    kept.texts.push(text);
                }
            }
            // A fill with a point outside is left out whole: what is left of it would
            // paint another area.
            for mut fill in std::mem::take(&mut layer.fills) {
                for point in &mut fill.points_mm {
                    *point = [point[0] - offset_mm[0], point[1] - offset_mm[1]];
                }
                let all_inside = fill.points_mm.iter().all(inside);
                map.fills.push(kept_at(all_inside, kept.fills.len()));
                if all_inside {
                    kept.fills.push(fill);
                }
            }
            for (style, polyline) in layer.styled_polylines() {
                // A point outside ends the polyline; the pieces on both sides of it are
                // left out.
                let mut run = Vec::new();
                for point in polyline {
                    let moved = [point[0] - offset_mm[0], point[1] - offset_mm[1]];
                    if inside(&moved) {
                        run.push(moved);
                    } else {
                        push_run(&mut kept, &mut run, style, &mut map.points);
                        map.points.push(u32::MAX);
                    }
                }
                push_run(&mut kept, &mut run, style, &mut map.points);
            }
            *layer = kept;
            maps.push(map);
        }
        self.objects.remap(&maps);
        self.objects
            .transform(&[[1.0, 0.0, -offset_mm[0]], [0.0, 1.0, -offset_mm[1]]]);
        MovedNearOrigin {
            offset_mm,
            segments_outside: before.0 - self.segment_count(),
            texts_outside: before.1 - self.text_count(),
            fills_outside: before.2 - self.fill_count(),
        }
    }

    /// The bytes of the container blob: the length of the body, then the body deflated
    /// (zlib) — little-endian counts, ends, coordinates and style indices, the styles
    /// before the layers, the objects after them. Lines without objects are written as
    /// version 5, without fills either as version 4, which older readers read too.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (with_objects, with_fills, body) = self.encode_whole_body();
        Self::deflate(with_objects, with_fills, &body)
    }

    /// What [`Self::encode`] writes, or why [`Self::decode`] would not read it back: lines
    /// past [`limits::UNDERLAY_LINES_BYTES`] are refused before they are deflated.
    pub fn encode_readable(&self) -> Result<Vec<u8>, UnderlayLinesError> {
        let (with_objects, with_fills, body) = self.encode_whole_body();
        if body.len() > limits::UNDERLAY_LINES_BYTES {
            return Err(UnderlayLinesError::TooLarge { length: body.len() });
        }
        Ok(Self::deflate(with_objects, with_fills, &body))
    }

    fn encode_whole_body(&self) -> (bool, bool, Vec<u8>) {
        let with_objects = !self.objects.is_empty();
        let with_fills = with_objects || self.fill_count() > 0;
        let mut body = self.encode_body(with_fills);
        if with_objects {
            self.objects.encode(&mut body);
        }
        (with_objects, with_fills, body)
    }

    fn deflate(with_objects: bool, with_fills: bool, body: &[u8]) -> Vec<u8> {
        let mut deflated = miniz_oxide::deflate::compress_to_vec_zlib(body, DEFLATE_LEVEL);
        if !inflates_plausibly(body.len(), deflated.len()) {
            // Stored, as `decode` refuses a body more than [`MAX_INFLATION`] times its
            // deflated bytes.
            deflated = miniz_oxide::deflate::compress_to_vec_zlib(body, 0);
        }
        let magic = MAGICS[match (with_objects, with_fills) {
            (true, _) => 5,
            (false, true) => 4,
            (false, false) => 3,
        }];
        let mut bytes = Vec::with_capacity(magic.len() + 8 + deflated.len());
        bytes.extend_from_slice(magic);
        let length = u64::try_from(body.len()).expect("a blob's length fits u64");
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&deflated);
        bytes
    }

    /// The bytes version 3 wrote after its magic, and `with_fills` each layer's fills
    /// after its texts.
    fn encode_body(&self, with_fills: bool) -> Vec<u8> {
        let capacity = 8 + self
            .layers
            .iter()
            .map(|layer| 12 + layer.ends.len() * 8 + layer.points_mm.len() * 16)
            .sum::<usize>();
        let mut bytes = Vec::with_capacity(capacity);
        push_count(&mut bytes, self.styles.len());
        for style in &self.styles {
            style.encode(&mut bytes);
        }
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
            for style in &layer.styles {
                bytes.extend_from_slice(&style.to_le_bytes());
            }
            push_count(&mut bytes, layer.texts.len());
            for text in &layer.texts {
                text.encode(&mut bytes);
            }
            if with_fills {
                push_count(&mut bytes, layer.fills.len());
                for fill in &layer.fills {
                    fill.encode(&mut bytes);
                }
            }
        }
        bytes
    }

    /// Reads what [`Self::encode`] wrote, or an older version of it; every count is
    /// checked against the bytes before anything is allocated for it.
    pub fn decode(bytes: &[u8]) -> Result<Self, UnderlayLinesError> {
        let (version, rest) = MAGICS
            .iter()
            .enumerate()
            .find_map(|(index, magic)| Some((index + 1, bytes.strip_prefix(magic.as_slice())?)))
            .ok_or(UnderlayLinesError::NotUnderlayLines)?;
        if version < 4 {
            return Self::decode_body(version, rest);
        }
        let (length, deflated) = rest
            .split_first_chunk::<8>()
            .ok_or(UnderlayLinesError::Truncated)?;
        let length = usize::try_from(u64::from_le_bytes(*length))
            .map_err(|_: std::num::TryFromIntError| UnderlayLinesError::Truncated)?;
        // Inflated no further than the length declared, so a damaged or hostile blob
        // allocates no more than it says it holds; and a length far past what its
        // bytes could hold is refused before anything is inflated, so a blob of a few
        // MB cannot ask for gigabytes (review 2026-10-24 MD-22), nor one of some 16 MB
        // past what an underlay may hold.
        if length > limits::UNDERLAY_LINES_BYTES {
            return Err(UnderlayLinesError::TooLarge { length });
        }
        if !inflates_plausibly(length, deflated.len()) {
            return Err(UnderlayLinesError::Inflated {
                length,
                deflated: deflated.len(),
            });
        }
        let body = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(deflated, length)
            .map_err(|error| match error.status {
                miniz_oxide::inflate::TINFLStatus::FailedCannotMakeProgress => {
                    UnderlayLinesError::Truncated
                }
                status => UnderlayLinesError::Inflate(status),
            })?;
        if body.len() != length {
            return Err(UnderlayLinesError::Truncated);
        }
        // Version 4 deflates the body of 3; 5 that of 3 with fills; 6 that of 5 with
        // objects.
        Self::decode_body(if version == 4 { 3 } else { version }, &body)
    }

    /// The bytes after the magic of `version`, uncompressed.
    fn decode_body(version: usize, rest: &[u8]) -> Result<Self, UnderlayLinesError> {
        let mut reader = Reader(rest);
        let mut styles = Vec::new();
        if version >= 3 {
            // Pushed one by one: each style checks its own bytes before it is kept.
            for _ in 0..reader.count()? {
                styles.push(LineStyle::decode(&mut reader)?);
            }
        }
        let named = |style: u32| {
            usize::try_from(style).is_ok_and(|style| style < styles.len())
                || (version < 3 && style == 0)
        };
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
            let polyline_styles = if version >= 3 {
                reader
                    .take(ends.len(), 4)?
                    .chunks_exact(4)
                    .map(|style| u32::from_le_bytes([style[0], style[1], style[2], style[3]]))
                    .collect()
            } else {
                vec![0; ends.len()]
            };
            let mut texts = Vec::new();
            if version >= 2 {
                // Pushed one by one: each text checks its own bytes before it is kept.
                for _ in 0..reader.count()? {
                    texts.push(UnderlayText::decode(&mut reader, version >= 3)?);
                }
            }
            let mut fills = Vec::new();
            if version >= 5 {
                for _ in 0..reader.count()? {
                    fills.push(UnderlayFill::decode(&mut reader)?);
                }
            }
            let mut indices = polyline_styles
                .iter()
                .chain(texts.iter().map(|text| &text.style))
                .chain(fills.iter().map(|fill| &fill.style));
            if !indices.all(|style| named(*style)) {
                return Err(UnderlayLinesError::InvalidStyle);
            }
            layers.push(LayerLines {
                points_mm,
                ends,
                styles: polyline_styles,
                texts,
                fills,
            });
        }
        let objects = if version >= 6 {
            let sizes = layers
                .iter()
                .map(|layer| [layer.points_mm.len(), layer.texts.len(), layer.fills.len()])
                .collect::<Vec<_>>();
            UnderlayObjects::decode(&mut reader, &sizes)?
        } else {
            UnderlayObjects::default()
        };
        if !reader.0.is_empty() {
            return Err(UnderlayLinesError::TrailingBytes);
        }
        // An import keeps the work of filling within the drawing's budget; a blob that
        // needs more was not written by one.
        let crossings = layers
            .iter()
            .flat_map(|layer| &layer.fills)
            .fold(0_usize, |sum, fill| {
                sum.saturating_add(fill.sweep_crossings())
            });
        if crossings > limits::DRAWING_ENTITIES {
            return Err(UnderlayLinesError::FillTooLarge(crossings));
        }
        Ok(Self {
            layers,
            styles,
            objects,
        })
    }
}

/// What [`UnderlayLines::move_near_origin`] moved the drawing by, and what it left out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MovedNearOrigin {
    pub offset_mm: [f64; 2],
    /// Straight pieces that even moved leave the envelope.
    pub segments_outside: usize,
    /// Texts whose anchor even moved leaves the envelope.
    pub texts_outside: usize,
    /// Fills of which a point even moved leaves the envelope.
    pub fills_outside: usize,
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

/// Collects drawing segments into the polylines of named layers, each in the style set
/// last.
#[derive(Default)]
pub struct UnderlayLinesBuilder {
    names: Vec<String>,
    layers: Vec<LayerLines>,
    /// Per layer, the loose lines and arcs [`Self::finish`] joins into polylines.
    loose: Vec<LayerLines>,
    styles: Vec<LineStyle>,
    /// The index in `styles` of what is added next.
    style: u32,
    objects: UnderlayObjects,
    /// The object what is added next belongs to.
    object: Option<u32>,
    /// Per layer, the object of each loose polyline.
    loose_objects: Vec<Vec<Option<u32>>>,
}

/// How far from the middle a drawing moved near the origin keeps its lines and texts:
/// ±10 km, the cadastral map of a whole municipality. Its points are f64 and need no
/// model envelope; the window draws each as a high and a low f32 part taken from the eye,
/// so they stay exact out there too.
pub const UNDERLAY_EXTENT_MM: f64 = 10_000_000.0;

/// Chords of a whole circle at most, however large it is: enough for the chords of an
/// arc of up to 87 km radius to stay within [`UNDERLAY_ARC_CHORD_MM`] of it, far past
/// the [`UNDERLAY_EXTENT_MM`] an underlay keeps around its middle.
const MAX_CIRCLE_CHORDS: f64 = 65_536.0;
/// Chords of one cubic Bézier piece.
const BEZIER_CHORDS: u32 = 16;

impl UnderlayLinesBuilder {
    /// The style of the lines and texts added after; until set, a continuous line in the
    /// underlay's own color.
    pub fn set_style(&mut self, style: &LineStyle) {
        // Index 0 stays what was added before a style was set.
        if self.styles.is_empty() {
            self.styles.push(LineStyle::default());
        }
        if self.styles.get(index(self.style)) == Some(style) {
            return;
        }
        let at = match self.styles.iter().position(|kept| kept == style) {
            Some(at) => at,
            None => {
                self.styles.push(style.clone());
                self.styles.len() - 1
            }
        };
        self.style = u32::try_from(at).expect("an underlay count fits the blob's u32");
    }

    fn dashed(&self) -> bool {
        self.styles
            .get(index(self.style))
            .is_some_and(LineStyle::is_dashed)
    }

    /// Adds one connected run of `segments` to `layer` as one polyline; dashed, as one
    /// polyline per segment, since a drawing starts the pattern anew at each.
    pub fn add(&mut self, layer: &str, segments: &[ProfileSegment]) {
        let index = self.layer_index(layer);
        let before = self.sizes(index);
        if self.dashed() {
            for segment in segments {
                push_polyline(
                    &mut self.layers[index],
                    std::slice::from_ref(segment),
                    self.style,
                );
            }
        } else {
            push_polyline(&mut self.layers[index], segments, self.style);
        }
        self.claim(index, before);
    }

    /// Adds a loose line or arc of `layer`, joined with the others of its style it shares
    /// an end with unless dashed.
    pub fn add_loose(&mut self, layer: &str, segment: &ProfileSegment) {
        if self.dashed() {
            self.add(layer, std::slice::from_ref(segment));
        } else {
            let index = self.layer_index(layer);
            let loose = &mut self.loose[index];
            let before = loose.ends.len();
            push_polyline(loose, std::slice::from_ref(segment), self.style);
            if loose.ends.len() > before {
                self.loose_objects[index].push(self.object);
            }
        }
    }

    /// Adds straight pieces of `layer` that each stay a polyline of their own, as the
    /// dashes of a hatch's pattern do.
    pub fn add_pieces(&mut self, layer: &str, pieces: &[[[f64; 2]; 2]]) {
        let index = self.layer_index(layer);
        let before = self.sizes(index);
        let lines = &mut self.layers[index];
        for piece in pieces {
            lines.points_mm.extend_from_slice(piece);
            lines.ends.push(lines.points_mm.len());
            lines.styles.push(self.style);
        }
        self.claim(index, before);
    }

    /// Adds a fill of `layer`: what lies inside an odd number of `rings`, each closed and
    /// of at least three points.
    pub fn add_fill(&mut self, layer: &str, rings: &[Vec<[f64; 2]>]) {
        let index = self.layer_index(layer);
        let before = self.sizes(index);
        let fill = UnderlayFill::new(self.style, rings);
        if !fill.ends.is_empty() {
            self.layers[index].fills.push(fill);
        }
        self.claim(index, before);
    }

    /// Adds a text of `layer`.
    pub fn add_text(&mut self, layer: &str, mut text: UnderlayText) {
        let index = self.layer_index(layer);
        let before = self.sizes(index);
        text.style = self.style;
        self.layers[index].texts.push(text);
        self.claim(index, before);
    }

    /// Adds an object of the drawing of `kind` on `layer`, placed by `parent` (an object
    /// added before), and returns its index.
    pub fn add_object(
        &mut self,
        kind: ObjectKind,
        handle: u64,
        layer: &str,
        parent: Option<u32>,
        detail: ObjectDetail,
    ) -> u32 {
        let layer = self.objects.layer_index(layer);
        self.objects.objects.push(UnderlayObject {
            kind,
            handle,
            layer,
            parent,
            parts: Vec::new(),
            detail,
            removed: false,
            added_by: None,
        });
        u32::try_from(self.objects.objects.len() - 1)
            .expect("an underlay count fits the blob's u32")
    }

    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.objects.len()
    }

    /// The object the lines, texts and fills added after belong to; none until set.
    pub fn set_object(&mut self, object: Option<u32>) {
        self.object = object;
    }

    /// How many points, texts and fills layer `index` holds.
    fn sizes(&self, index: usize) -> [usize; 3] {
        let layer = &self.layers[index];
        [layer.points_mm.len(), layer.texts.len(), layer.fills.len()]
    }

    /// Gives what layer `index` gained since it held `before` to the current object.
    fn claim(&mut self, index: usize, before: [usize; 3]) {
        let after = self.sizes(index);
        if let Some(object) = self.object {
            add_part(&mut self.objects, object, index, before, after);
        }
    }

    fn layer_index(&mut self, layer: &str) -> usize {
        if let Some(index) = self.names.iter().position(|name| name == layer) {
            return index;
        }
        self.names.push(layer.to_owned());
        self.layers.push(LayerLines::default());
        self.loose.push(LayerLines::default());
        self.loose_objects.push(Vec::new());
        self.layers.len() - 1
    }

    /// The layer names in the order of the lines' layers, and the lines.
    #[must_use]
    pub fn finish(mut self) -> (Vec<String>, UnderlayLines) {
        for (index, (lines, loose)) in self.layers.iter_mut().zip(&self.loose).enumerate() {
            let mut placed = Vec::new();
            join_loose(loose, lines, &mut placed);
            for (piece, points) in placed {
                if let Some(object) = self.loose_objects[index][piece] {
                    let texts = lines.texts.len();
                    let fills = lines.fills.len();
                    add_part(
                        &mut self.objects,
                        object,
                        index,
                        [points.start, texts, fills],
                        [points.end, texts, fills],
                    );
                }
            }
        }
        if self.styles.is_empty() {
            self.styles.push(LineStyle::default());
        }
        (
            self.names,
            UnderlayLines {
                layers: self.layers,
                styles: self.styles,
                objects: self.objects,
            },
        )
    }
}

/// Gives `object` the points, texts and fills of layer `layer` from `before` to `after`.
fn add_part(
    objects: &mut UnderlayObjects,
    object: u32,
    layer: usize,
    before: [usize; 3],
    after: [usize; 3],
) {
    let count = |value: usize| u32::try_from(value).expect("an underlay count fits the blob's u32");
    if before == after {
        return;
    }
    if let Some(object) = objects.objects.get_mut(index(object)) {
        object.add_part(ObjectPart {
            layer: count(layer),
            points: count(before[0])..count(after[0]),
            texts: count(before[1])..count(after[1]),
            fills: count(before[2])..count(after[2]),
        });
    }
}

/// A style index as an index into the styles.
fn index(style: u32) -> usize {
    usize::try_from(style).unwrap_or(usize::MAX)
}

/// Appends `segments`, one connected run, to `lines` as one polyline of `style`.
fn push_polyline(lines: &mut LayerLines, segments: &[ProfileSegment], style: u32) {
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
    lines.styles.push(style);
}

/// Appends `run` to `lines` as one polyline of `style` if it has a piece, and empties it;
/// `map` gets where each of its points went, `u32::MAX` if it was left out.
fn push_run(lines: &mut LayerLines, run: &mut Vec<[f64; 2]>, style: u32, map: &mut Vec<u32>) {
    let kept = run.len() >= 2;
    let start = lines.points_mm.len();
    map.extend((0..run.len()).map(|offset| kept_at(kept, start + offset)));
    if kept {
        lines.points_mm.append(run);
        lines.ends.push(lines.points_mm.len());
        lines.styles.push(style);
    }
    run.clear();
}

/// Where a point, text or fill went: `at` if it was `kept`, else `u32::MAX`.
fn kept_at(kept: bool, at: usize) -> u32 {
    if kept {
        u32::try_from(at).expect("an underlay count fits the blob's u32")
    } else {
        u32::MAX
    }
}

/// Appends the polylines of `loose` to `lines`, each run of them of one style that meets
/// end to end at points no third one of it touches joined into one polyline. Points meet
/// when they are equal. `placed` gets the points of `lines` each piece became, by its
/// index in `loose`.
fn join_loose(
    loose: &LayerLines,
    lines: &mut LayerLines,
    placed: &mut Vec<(usize, std::ops::Range<usize>)>,
) {
    let pieces = loose.polylines().collect::<Vec<_>>();
    let key = |piece: usize, point: [f64; 2]| {
        (
            loose.styles[piece],
            [point[0].to_bits(), point[1].to_bits()],
        )
    };
    // At each point, the pieces that end there: 2 × piece, + 1 for its last point.
    let mut ends_at = std::collections::HashMap::<(u32, [u64; 2]), Vec<usize>>::new();
    for (index, piece) in pieces.iter().enumerate() {
        ends_at
            .entry(key(index, piece[0]))
            .or_default()
            .push(2 * index);
        ends_at
            .entry(key(index, piece[piece.len() - 1]))
            .or_default()
            .push(2 * index + 1);
    }
    let mut used = vec![false; pieces.len()];
    let mut walk = |first: usize, used: &mut [bool], lines: &mut LayerLines| {
        let mut end = first;
        used[end / 2] = true;
        let start = lines.points_mm.len();
        push_oriented(&mut lines.points_mm, pieces[end / 2], end % 2 == 1, false);
        placed.push((end / 2, start..lines.points_mm.len()));
        loop {
            let tail = pieces[end / 2][if end.is_multiple_of(2) {
                pieces[end / 2].len() - 1
            } else {
                0
            }];
            let meeting = &ends_at[&key(end / 2, tail)];
            if meeting.len() != 2 {
                break;
            }
            let Some(&next) = meeting.iter().find(|next| !used[**next / 2]) else {
                break;
            };
            used[next / 2] = true;
            // It starts at the point the run ends at.
            let start = lines.points_mm.len() - 1;
            push_oriented(&mut lines.points_mm, pieces[next / 2], next % 2 == 1, true);
            placed.push((next / 2, start..lines.points_mm.len()));
            end = next;
        }
        lines.ends.push(lines.points_mm.len());
        lines.styles.push(loose.styles[first / 2]);
    };
    // Runs start where no two pieces meet alone; what is left are closed rings.
    for index in 0..pieces.len() {
        for end in [2 * index, 2 * index + 1] {
            let point = pieces[index][if end.is_multiple_of(2) {
                0
            } else {
                pieces[index].len() - 1
            }];
            if !used[index] && ends_at[&key(index, point)].len() != 2 {
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

/// The angle an arc of `radius` turns by from chord to chord at most: its chords stay
/// within [`UNDERLAY_ARC_CHORD_MM`] of it, and a whole circle has at most
/// [`MAX_CIRCLE_CHORDS`] of them.
pub(crate) fn arc_chord_step(radius: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let step = if radius > UNDERLAY_ARC_CHORD_MM {
        2.0 * (1.0 - UNDERLAY_ARC_CHORD_MM / radius).acos()
    } else {
        tau
    };
    step.max(tau / MAX_CIRCLE_CHORDS)
}

/// Appends the points after the start of `segment`.
pub(crate) fn push_segment(points: &mut Vec<[f64; 2]>, segment: &ProfileSegment) {
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
            let chords = (sweep.abs() / arc_chord_step(radius)).ceil();
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
/// and one per layer, its lines being the container blob `lines_sha256`, moved by
/// `offset_mm` from the drawing's own coordinates and placed where that puts it beside the
/// first underlay that was moved too ([`placement::geo_placement`]). The tags of
/// `hidden_layers`, the layers
/// the drawing turned off or froze, start hidden.
pub fn plan_underlay(
    snapshot: &Snapshot,
    name: &str,
    layer_names: &[String],
    hidden_layers: &[String],
    lines_sha256: [u8; 32],
    offset_mm: [f64; 2],
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
    let mut new_tag = |commands: &mut Vec<CanonicalCommand>, name: String, visible: bool| {
        let id = TagId(next_tag);
        next_tag = next_tag
            .checked_add(1)
            .ok_or(UnderlayPlanError::IdSpaceExhausted)?;
        commands.push(CanonicalCommand::CreateTag { id, name, visible });
        Ok(id)
    };
    let mut commands = Vec::with_capacity(layer_names.len() + 2);
    let tag = new_tag(&mut commands, name.to_owned(), true)?;
    let layers = layer_names
        .iter()
        .map(|layer| {
            let visible = !hidden_layers.contains(layer);
            Ok(UnderlayLayer {
                name: layer.clone(),
                tag: new_tag(&mut commands, format!("{name} · {layer}"), visible)?,
            })
        })
        .collect::<Result<Vec<_>, UnderlayPlanError>>()?;
    commands.push(CanonicalCommand::UpsertUnderlay(Underlay {
        id,
        name: name.to_owned(),
        tag,
        layers,
        lines_sha256,
        offset_mm_bits: offset_mm.map(f64::to_bits),
        placement: placement::geo_placement(snapshot, offset_mm),
        edits: edits::UnderlayEdits::default(),
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
