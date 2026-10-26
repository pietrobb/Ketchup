//! What Kečup draws into an underlay's drawing and how it moves what is there: shapes added
//! on a layer, copies of objects and objects moved, turned or mirrored. The edits keep only
//! the shapes and the maps ([`super::edits::UnderlayEdits`]); here they become lines, texts
//! and fills of the drawing with objects of their own, as the read ones have.

use super::objects::{Affine, ObjectDetail, ObjectKind, ObjectPart, UnderlayObject, apply};
use super::{LayerLines, UnderlayFill, UnderlayLines, UnderlayText, add_part, index, push_segment};
use crate::document::ProfileSegment;

/// A shape drawn into a layer, in the millimetres of the underlay's lines.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum DrawnShape {
    /// Straight pieces through `points_mm`; `closed` joins the last back to the first. Two
    /// points not closed are a LINE, more an LWPOLYLINE.
    #[serde(alias = "line")]
    Polyline {
        points_mm: Vec<[f64; 2]>,
        #[serde(default)]
        closed: bool,
    },
    /// Counterclockwise from `start_deg` to `end_deg`, as a drawing's ARC.
    Arc {
        center_mm: [f64; 2],
        radius_mm: f64,
        start_deg: f64,
        end_deg: f64,
    },
    Circle {
        center_mm: [f64; 2],
        radius_mm: f64,
    },
    /// One line of text on its baseline from `anchor_mm`, letters `height_mm` high.
    Text {
        anchor_mm: [f64; 2],
        height_mm: f64,
        #[serde(default)]
        rotation_deg: f64,
        text: String,
    },
}

/// One thing added to a drawing; each becomes objects after the drawing's own, in order.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "added", rename_all = "snake_case")]
pub enum AddedObject {
    /// A shape on the layer of that name.
    Drawn { layer: String, shape: DrawnShape },
    /// A copy of the drawing's own top-level object `of` and all it placed, as it was read,
    /// mapped by `map`.
    Copy { of: u32, map: Affine },
}

impl DrawnShape {
    /// The kind of object it is.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Polyline { points_mm, closed } if points_mm.len() == 2 && !closed => "LINE",
            Self::Polyline { .. } => "LWPOLYLINE",
            Self::Arc { .. } => "ARC",
            Self::Circle { .. } => "CIRCLE",
            Self::Text { .. } => "TEXT",
        }
    }

    /// Whether it is drawable: finite numbers, a radius and height above zero, a polyline of
    /// two points or more and a text with letters.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let finite = |point: &[f64; 2]| point.iter().all(|value| value.is_finite());
        match self {
            Self::Polyline { points_mm, closed } => {
                points_mm.len() >= if *closed { 3 } else { 2 } && points_mm.iter().all(finite)
            }
            Self::Arc {
                center_mm,
                radius_mm,
                start_deg,
                end_deg,
            } => {
                finite(center_mm)
                    && *radius_mm > 0.0
                    && radius_mm.is_finite()
                    && start_deg.is_finite()
                    && end_deg.is_finite()
            }
            Self::Circle {
                center_mm,
                radius_mm,
            } => finite(center_mm) && *radius_mm > 0.0 && radius_mm.is_finite(),
            Self::Text {
                anchor_mm,
                height_mm,
                rotation_deg,
                text,
            } => {
                finite(anchor_mm)
                    && *height_mm > 0.0
                    && height_mm.is_finite()
                    && rotation_deg.is_finite()
                    && !text.trim().is_empty()
            }
        }
    }

    /// The same shape mapped by `map`, a similarity ([`is_similarity`]).
    #[must_use]
    pub fn transformed(&self, map: &Affine) -> Self {
        let scale = scale_of(map);
        let mirrored = determinant(map) < 0.0;
        let angle_of = |center: [f64; 2], radius: f64, degrees: f64| {
            let point = [
                center[0] + radius * degrees.to_radians().cos(),
                center[1] + radius * degrees.to_radians().sin(),
            ];
            let [center, point] = [center, point].map(|point| apply(map, point));
            (point[1] - center[1])
                .atan2(point[0] - center[0])
                .to_degrees()
        };
        match self {
            Self::Polyline { points_mm, closed } => Self::Polyline {
                points_mm: points_mm.iter().map(|point| apply(map, *point)).collect(),
                closed: *closed,
            },
            Self::Arc {
                center_mm,
                radius_mm,
                start_deg,
                end_deg,
            } => {
                let [start, end] =
                    [*start_deg, *end_deg].map(|angle| angle_of(*center_mm, *radius_mm, angle));
                // A mirror runs the arc the other way: counterclockwise it starts at the
                // image of its end.
                let (start_deg, end_deg) = if mirrored { (end, start) } else { (start, end) };
                Self::Arc {
                    center_mm: apply(map, *center_mm),
                    radius_mm: radius_mm * scale,
                    start_deg,
                    end_deg,
                }
            }
            Self::Circle {
                center_mm,
                radius_mm,
            } => Self::Circle {
                center_mm: apply(map, *center_mm),
                radius_mm: radius_mm * scale,
            },
            Self::Text {
                anchor_mm,
                height_mm,
                rotation_deg,
                text,
            } => Self::Text {
                anchor_mm: apply(map, *anchor_mm),
                height_mm: height_mm * scale,
                rotation_deg: readable_rotation(map, rotation_deg.to_radians()).to_degrees(),
                text: text.clone(),
            },
        }
    }

    /// Its lines and texts.
    fn pieces(&self) -> Vec<Piece> {
        let arc = |center: [f64; 2], radius: f64, start: f64, end: f64| {
            let at = |degrees: f64| {
                [
                    center[0] + radius * degrees.to_radians().cos(),
                    center[1] + radius * degrees.to_radians().sin(),
                ]
            };
            let mut points = vec![at(start)];
            push_segment(
                &mut points,
                &ProfileSegment::CircularArc {
                    start_mm: at(start),
                    end_mm: at(end),
                    center_mm: center,
                    clockwise: false,
                },
            );
            points
        };
        match self {
            Self::Polyline { points_mm, closed } => {
                let mut points = points_mm.clone();
                if *closed {
                    points.push(points_mm[0]);
                }
                vec![Piece::Line(points)]
            }
            Self::Arc {
                center_mm,
                radius_mm,
                start_deg,
                end_deg,
            } => vec![Piece::Line(arc(
                *center_mm, *radius_mm, *start_deg, *end_deg,
            ))],
            Self::Circle {
                center_mm,
                radius_mm,
            } => {
                let mut points = arc(*center_mm, *radius_mm, 0.0, 180.0);
                points.extend(
                    arc(*center_mm, *radius_mm, 180.0, 360.0)
                        .into_iter()
                        .skip(1),
                );
                vec![Piece::Line(points)]
            }
            Self::Text {
                anchor_mm,
                height_mm,
                rotation_deg,
                text,
            } => {
                let mut drawn = UnderlayText::new(text.clone(), *anchor_mm, *height_mm);
                drawn.rotation_rad = rotation_deg.to_radians();
                vec![Piece::Text(drawn)]
            }
        }
    }
}

/// Whether `map` keeps shapes as they are up to size and a mirror: a turn, a uniform
/// scale above zero, a mirror and a move, with finite numbers.
#[must_use]
pub fn is_similarity(map: &Affine) -> bool {
    let [[a, b, x], [c, d, y]] = *map;
    let finite = [a, b, c, d, x, y].iter().all(|value| value.is_finite());
    let scale = a.hypot(c);
    let close = |p: f64, q: f64| (p - q).abs() <= 1e-9 * scale.max(1.0);
    let turn = close(a, d) && close(b, -c);
    let mirror = close(a, -d) && close(b, c);
    finite && scale > 1e-12 && (turn || mirror)
}

/// `outer` after `inner`.
#[must_use]
pub fn then(inner: &Affine, outer: &Affine) -> Affine {
    super::objects::compose(outer, inner)
}

/// The map that undoes `map`; `None` when it flattens the plane.
#[must_use]
pub fn inverse(map: &Affine) -> Option<Affine> {
    let [[a, b, x], [c, d, y]] = *map;
    let det = determinant(map);
    if det.abs() <= f64::MIN_POSITIVE || !det.is_finite() {
        return None;
    }
    let [ia, ib, ic, id] = [d / det, -b / det, -c / det, a / det];
    Some([[ia, ib, -(ia * x + ib * y)], [ic, id, -(ic * x + id * y)]])
}

/// `map`, a map of the document, as the map of the lines that `placement` (from the lines
/// to the document) places: into the document, `map`, and back.
#[must_use]
pub fn in_lines(map: &Affine, placement: &Affine) -> Option<Affine> {
    Some(then(&then(placement, map), &inverse(placement)?))
}

/// Scaled by `scale` and turned counterclockwise by `rotate_deg` about `about`, mirrored
/// across the line through the two points `mirror`, then moved by `by`.
#[must_use]
pub fn similarity(
    by: [f64; 2],
    rotate_deg: f64,
    scale: f64,
    about: [f64; 2],
    mirror: Option<[[f64; 2]; 2]>,
) -> Affine {
    let (sin, cos) = rotate_deg.to_radians().sin_cos();
    let [a, b, c, d] = [scale * cos, -scale * sin, scale * sin, scale * cos];
    let turned = [
        [a, b, about[0] - a * about[0] - b * about[1]],
        [c, d, about[1] - c * about[0] - d * about[1]],
    ];
    let mirrored = match mirror {
        Some([from, to]) => {
            let direction = [to[0] - from[0], to[1] - from[1]];
            let length = direction[0].hypot(direction[1]);
            let [u, v] = [direction[0] / length, direction[1] / length];
            // Reflection across the line through `from` along (u, v).
            let [ma, mb, mc, md] = [u * u - v * v, 2.0 * u * v, 2.0 * u * v, v * v - u * u];
            let reflect = [
                [ma, mb, from[0] - ma * from[0] - mb * from[1]],
                [mc, md, from[1] - mc * from[0] - md * from[1]],
            ];
            then(&turned, &reflect)
        }
        None => turned,
    };
    then(&mirrored, &[[1.0, 0.0, by[0]], [0.0, 1.0, by[1]]])
}

fn determinant(map: &Affine) -> f64 {
    map[0][0] * map[1][1] - map[0][1] * map[1][0]
}

fn scale_of(map: &Affine) -> f64 {
    determinant(map).abs().sqrt()
}

/// The rotation of a text at `rotation` once mapped by `map`: its baseline's image, turned
/// round when a mirror would make it read backward, so it stays readable as a drawing's
/// mirrored text does.
fn readable_rotation(map: &Affine, rotation: f64) -> f64 {
    let direction = [rotation.cos(), rotation.sin()];
    let image = [
        map[0][0] * direction[0] + map[0][1] * direction[1],
        map[1][0] * direction[0] + map[1][1] * direction[1],
    ];
    let angle = image[1].atan2(image[0]);
    if determinant(map) < 0.0 && image[0] < 0.0 {
        angle + std::f64::consts::PI
    } else {
        angle
    }
}

/// What one object draws itself on one layer.
#[derive(Clone, Debug)]
enum Piece {
    Line(Vec<[f64; 2]>),
    Text(UnderlayText),
    Fill(UnderlayFill),
}

/// One object of a captured tree: what it is, and what it draws by layer and style.
#[derive(Clone, Debug)]
struct CapturedObject {
    id: u32,
    kind: ObjectKind,
    layer: u32,
    /// Its parent's index in the captured tree.
    parent: Option<usize>,
    detail: ObjectDetail,
    /// By layer of the lines and style.
    pieces: Vec<(usize, u32, Piece)>,
}

/// A top-level object and all it placed, with what each draws, taken from the lines before
/// they are edited.
#[derive(Clone, Debug)]
pub(super) struct Captured(Vec<CapturedObject>);

impl UnderlayLines {
    /// The children of every object, by its index.
    pub(super) fn children_index(&self) -> Vec<Vec<u32>> {
        let mut children = vec![Vec::new(); self.objects.objects.len()];
        for (id, object) in self.objects.objects.iter().enumerate() {
            if let (Some(parent), Ok(id)) = (object.parent, u32::try_from(id))
                && let Some(siblings) = children.get_mut(index(parent))
            {
                siblings.push(id);
            }
        }
        children
    }

    /// Object `root` and all it placed, with what each draws; `None` for no such object.
    pub(super) fn capture(&self, root: u32, children: &[Vec<u32>]) -> Option<Captured> {
        self.objects.objects.get(index(root))?;
        let mut captured = Vec::new();
        let mut queue = vec![(root, None)];
        while let Some((id, parent)) = queue.pop() {
            let object = &self.objects.objects[index(id)];
            let at = captured.len();
            captured.push(CapturedObject {
                id,
                kind: object.kind,
                layer: object.layer,
                parent,
                detail: object.detail.clone(),
                pieces: object
                    .parts
                    .iter()
                    .flat_map(|part| self.part_pieces(part))
                    .collect(),
            });
            queue.extend(
                children
                    .get(index(id))
                    .into_iter()
                    .flatten()
                    .rev()
                    .map(|child| (*child, Some(at))),
            );
        }
        Some(Captured(captured))
    }

    /// The polylines, texts and fills of `part`, each polyline with its style.
    fn part_pieces(&self, part: &ObjectPart) -> Vec<(usize, u32, Piece)> {
        let layer_index = index(part.layer);
        let Some(layer) = self.layers.get(layer_index) else {
            return Vec::new();
        };
        let mut pieces = Vec::new();
        let [start, end] = [part.points.start, part.points.end].map(index);
        // The first polyline that ends past `start`.
        let mut polyline = layer.ends.partition_point(|&ends| ends <= start);
        while let Some(&polyline_end) = layer.ends.get(polyline) {
            let polyline_start = if polyline == 0 {
                0
            } else {
                layer.ends[polyline - 1]
            };
            if polyline_start >= end {
                break;
            }
            let run = polyline_start.max(start)..polyline_end.min(end);
            if run.len() >= 2 {
                pieces.push((
                    layer_index,
                    layer.styles[polyline],
                    Piece::Line(layer.points_mm[run].to_vec()),
                ));
            }
            polyline += 1;
        }
        let range = |range: &std::ops::Range<u32>, len: usize| {
            index(range.start).min(len)..index(range.end).min(len)
        };
        for text in &layer.texts[range(&part.texts, layer.texts.len())] {
            pieces.push((layer_index, text.style, Piece::Text(text.clone())));
        }
        for fill in &layer.fills[range(&part.fills, layer.fills.len())] {
            pieces.push((layer_index, fill.style, Piece::Fill(fill.clone())));
        }
        pieces
    }

    /// Adds `captured` mapped by `map`: into the objects it was taken from when `keep_ids`,
    /// else as new objects made by edit `added_by`, which hold nothing and are removed when
    /// `removed`.
    pub(super) fn put(
        &mut self,
        captured: &Captured,
        map: &Affine,
        keep_ids: bool,
        added_by: Option<u32>,
        removed: bool,
    ) {
        let mut ids = Vec::with_capacity(captured.0.len());
        for object in &captured.0 {
            let detail = object.detail.transformed(map);
            let id = if keep_ids {
                let kept = &mut self.objects.objects[index(object.id)];
                kept.detail = detail;
                kept.parts.clear();
                object.id
            } else {
                self.push_object(UnderlayObject {
                    kind: object.kind,
                    handle: 0,
                    layer: object.layer,
                    parent: object.parent.map(|parent| ids[parent]),
                    parts: Vec::new(),
                    detail,
                    removed,
                    added_by,
                })
            };
            ids.push(id);
            if removed {
                continue;
            }
            for (layer, style, piece) in &object.pieces {
                self.put_piece(id, *layer, *style, &mapped(piece, map));
            }
        }
    }

    /// Adds `shape` on the lines' layer `layer` and the objects' layer `object_layer` as
    /// a new object made by edit `added_by`; with no layer it holds nothing.
    pub(super) fn draw(
        &mut self,
        layer: Option<usize>,
        object_layer: u32,
        shape: &DrawnShape,
        added_by: u32,
    ) {
        let id = self.push_object(UnderlayObject {
            kind: ObjectKind::from_dxf(shape.kind()).expect("a kind objects are kept of"),
            handle: 0,
            layer: object_layer,
            parent: None,
            parts: Vec::new(),
            detail: ObjectDetail::None,
            removed: false,
            added_by: Some(added_by),
        });
        if let Some(layer) = layer {
            for piece in shape.pieces() {
                self.put_piece(id, layer, 0, &piece);
            }
        }
    }

    fn push_object(&mut self, object: UnderlayObject) -> u32 {
        self.objects.objects.push(object);
        u32::try_from(self.objects.objects.len() - 1)
            .expect("an underlay count fits the blob's u32")
    }

    /// Appends `piece` to layer `layer` in style `style` as part of object `id`.
    fn put_piece(&mut self, id: u32, layer: usize, style: u32, piece: &Piece) {
        if self.layers.len() <= layer {
            self.layers.resize_with(layer + 1, LayerLines::default);
        }
        let style = if index(style) < self.styles.len() {
            style
        } else {
            0
        };
        let lines = &mut self.layers[layer];
        let before = [lines.points_mm.len(), lines.texts.len(), lines.fills.len()];
        match piece {
            Piece::Line(points) => {
                lines.points_mm.extend_from_slice(points);
                lines.ends.push(lines.points_mm.len());
                lines.styles.push(style);
            }
            Piece::Text(text) => lines.texts.push(UnderlayText {
                style,
                ..text.clone()
            }),
            Piece::Fill(fill) => lines.fills.push(UnderlayFill {
                style,
                ..fill.clone()
            }),
        }
        let after = [lines.points_mm.len(), lines.texts.len(), lines.fills.len()];
        add_part(&mut self.objects, id, layer, before, after);
    }
}

/// `piece` mapped by `map`.
fn mapped(piece: &Piece, map: &Affine) -> Piece {
    match piece {
        Piece::Line(points) => Piece::Line(points.iter().map(|point| apply(map, *point)).collect()),
        Piece::Text(text) => Piece::Text(UnderlayText {
            anchor_mm: apply(map, text.anchor_mm),
            height_mm: text.height_mm * scale_of(map),
            wrap_width_mm: text.wrap_width_mm * scale_of(map),
            line_pitch_mm: text.line_pitch_mm * scale_of(map),
            rotation_rad: readable_rotation(map, text.rotation_rad),
            ..text.clone()
        }),
        Piece::Fill(fill) => Piece::Fill(UnderlayFill {
            points_mm: fill
                .points_mm
                .iter()
                .map(|point| apply(map, *point))
                .collect(),
            ..fill.clone()
        }),
    }
}
