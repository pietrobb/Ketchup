//! Changes made in Kečup to an underlay's drawing: objects taken out of it, moved, and
//! drawn or copied into it ([`super::drawn`]). The blob of the drawing stays as it was
//! read, shared by every revision; the edits are a few numbers and shapes the underlay
//! carries, so the Undo step of an edit copies no lines. [`UnderlayLines::edited`] gives
//! the drawing with them, and everything that draws, snaps to or asks about an underlay's
//! lines reads that.

use std::collections::{BTreeMap, BTreeSet};

use super::drawn::{AddedObject, DrawnShape, is_similarity, then};
use super::objects::{Affine, LayerMaps, ObjectDetail, ObjectKind, UnderlayObject};
use super::{LayerLines, Underlay, UnderlayLayer, UnderlayLines, UnderlayPlanError, kept_at};
use crate::document::{CanonicalCommand, CommandBatch, Snapshot};

/// What was changed of an underlay's drawing. Objects added get the ids after the
/// drawing's own, in the order of `added`, so an id stays what it names.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnderlayEdits {
    /// The top-level objects taken out, by their index among the objects of the lines; an
    /// insert or dimension takes out what it placed with it.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub removed_objects: BTreeSet<u32>,
    /// Top-level objects of the drawing as read, by id, with the map from where they were
    /// read to where they are: a move, turn, mirror or uniform scale.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub moved_objects: BTreeMap<u32, Affine>,
    /// What was drawn or copied into the drawing, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<AddedObject>,
}

impl UnderlayEdits {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.removed_objects.is_empty() && self.moved_objects.is_empty() && self.added.is_empty()
    }

    /// Whether every shape can be drawn and every map is a similarity.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.moved_objects.values().all(is_similarity)
            && self.added.iter().all(|added| match added {
                AddedObject::Drawn { shape, .. } => shape.is_valid(),
                AddedObject::Copy { map, .. } => is_similarity(map),
            })
    }

    /// The bytes that name the edits in [`Underlay::lines_key`].
    fn key_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        ciborium::into_writer(self, &mut bytes).expect("edits serialize into memory");
        bytes
    }
}

impl Underlay {
    /// What names its lines as they are drawn: the blob's SHA-256 while nothing was
    /// changed, else the SHA-256 of that and its edits. Caches of decoded, drawn and
    /// snapped lines are kept under it.
    #[must_use]
    pub fn lines_key(&self) -> [u8; 32] {
        if self.edits.is_empty() {
            return self.lines_sha256;
        }
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(self.lines_sha256);
        hasher.update(self.edits.key_bytes());
        hasher.finalize().into()
    }

    /// Its lines as they are drawn: the blob `bytes` decoded, with its edits.
    pub fn read_lines(&self, bytes: &[u8]) -> Result<UnderlayLines, super::UnderlayLinesError> {
        Ok(UnderlayLines::decode(bytes)?.edited(&self.edits, &self.layers))
    }
}

impl UnderlayLines {
    /// These lines with `edits`, the lines' layers named by `layers`: the lines, texts and
    /// fills of a removed object and of all it placed left out, the polylines they were
    /// part of split where they were; a moved object's put where it was moved; and the
    /// added objects after the drawing's own. A removed object keeps its index, so the
    /// others keep theirs, but it holds nothing and is [`UnderlayObject::removed`].
    #[must_use]
    pub fn edited(mut self, edits: &UnderlayEdits, layers: &[UnderlayLayer]) -> Self {
        if edits.is_empty() {
            return self;
        }
        let children = self.children_index();
        let moved = edits
            .moved_objects
            .iter()
            .filter(|(id, _)| !edits.removed_objects.contains(id))
            .filter_map(|(id, map)| Some((self.capture(*id, &children)?, *map)))
            .collect::<Vec<_>>();
        let copies = edits
            .added
            .iter()
            .map(|added| match added {
                AddedObject::Copy { of, .. } => self.capture(*of, &children),
                AddedObject::Drawn { .. } => None,
            })
            .collect::<Vec<_>>();
        let removed = self.with_children(|id| edits.removed_objects.contains(&id));
        let moving = self.with_children(|id| {
            edits.moved_objects.contains_key(&id) && !edits.removed_objects.contains(&id)
        });
        let gone = removed
            .iter()
            .zip(&moving)
            .map(|(removed, moving)| *removed || *moving)
            .collect::<Vec<_>>();
        self.cut(&gone);
        for (object, removed) in self.objects.objects.iter_mut().zip(removed) {
            object.removed = removed;
        }
        for (captured, map) in &moved {
            self.put(captured, map, true, None, false);
        }
        for (added_by, (added, copy)) in edits.added.iter().zip(&copies).enumerate() {
            let added_by = u32::try_from(added_by).expect("an edit count fits u32");
            let first = u32::try_from(self.objects.objects.len()).unwrap_or(u32::MAX);
            let removed = edits.removed_objects.contains(&first);
            match (added, copy) {
                (AddedObject::Drawn { layer, shape }, _) => {
                    let lines_layer = layers.iter().position(|known| known.name == *layer);
                    let object_layer = self.objects.layer_index(layer);
                    self.draw(
                        lines_layer.filter(|_| !removed),
                        object_layer,
                        shape,
                        added_by,
                    );
                }
                (AddedObject::Copy { map, .. }, Some(captured)) => {
                    self.put(captured, map, false, Some(added_by), removed);
                }
                // A copy of no object of the drawing still takes its id, holding nothing.
                (AddedObject::Copy { .. }, None) => {
                    self.objects.objects.push(UnderlayObject {
                        kind: ObjectKind::from_dxf("INSERT").expect("a kind objects are kept of"),
                        handle: 0,
                        layer: 0,
                        parent: None,
                        parts: Vec::new(),
                        detail: ObjectDetail::None,
                        removed: true,
                        added_by: Some(added_by),
                    });
                    continue;
                }
            }
            if removed && let Some(object) = self.objects.objects.get_mut(super::index(first)) {
                object.removed = true;
            }
        }
        self
    }

    /// For each object, whether it or an object that placed it is one `named`.
    fn with_children(&self, named: impl Fn(u32) -> bool) -> Vec<bool> {
        let objects = &self.objects.objects;
        let mut marks = vec![false; objects.len()];
        for (index, object) in objects.iter().enumerate() {
            let by_id = u32::try_from(index).is_ok_and(&named);
            let by_parent = object
                .parent
                .and_then(|parent| marks.get(super::index(parent)).copied())
                .unwrap_or(false);
            marks[index] = by_id || by_parent;
        }
        marks
    }

    /// Takes out the lines, texts and fills of each object marked in `gone`, which then
    /// holds nothing.
    fn cut(&mut self, gone: &[bool]) {
        let objects = &self.objects.objects;
        let removed = gone;
        // Per layer: which straight pieces (from point k to k + 1), texts and fills go.
        let mut cut = self
            .layers
            .iter()
            .map(|layer| {
                (
                    vec![false; layer.points_mm.len()],
                    vec![false; layer.texts.len()],
                    vec![false; layer.fills.len()],
                )
            })
            .collect::<Vec<_>>();
        for (object, _) in objects.iter().zip(removed).filter(|(_, gone)| **gone) {
            for part in &object.parts {
                let Some((pieces, texts, fills)) = cut.get_mut(super::index(part.layer)) else {
                    continue;
                };
                let range = |range: &std::ops::Range<u32>| {
                    super::index(range.start)..super::index(range.end)
                };
                let points = range(&part.points);
                let last_piece = points.end.saturating_sub(1).min(pieces.len());
                pieces[points.start.min(last_piece)..last_piece].fill(true);
                for (marks, range) in [(texts, range(&part.texts)), (fills, range(&part.fills))] {
                    let end = range.end.min(marks.len());
                    marks[range.start.min(end)..end].fill(true);
                }
            }
        }
        let mut maps = Vec::with_capacity(self.layers.len());
        for (layer, (pieces, texts, fills)) in self.layers.iter_mut().zip(&cut) {
            let (kept, map) = without(layer, pieces, texts, fills);
            *layer = kept;
            maps.push(map);
        }
        self.objects.remap(&maps);
        for (object, gone) in self.objects.objects.iter_mut().zip(gone) {
            if *gone {
                object.parts.clear();
            }
        }
    }
}

/// `layer` without the straight pieces, texts and fills marked in `pieces`, `texts` and
/// `fills`, and where each of its points, texts and fills went.
fn without(
    layer: &LayerLines,
    pieces: &[bool],
    texts: &[bool],
    fills: &[bool],
) -> (LayerLines, LayerMaps) {
    let mut kept = LayerLines::default();
    let mut map = LayerMaps {
        points: vec![u32::MAX; layer.points_mm.len()],
        ..LayerMaps::default()
    };
    let mut start = 0;
    for (&end, &style) in layer.ends.iter().zip(&layer.styles) {
        let mut run_start = None;
        let flush = |kept: &mut LayerLines, run_start: &mut Option<usize>| {
            if let Some(first) = run_start.take()
                && kept.points_mm.len() - first >= 2
            {
                kept.ends.push(kept.points_mm.len());
                kept.styles.push(style);
            }
        };
        for (piece, &gone) in pieces.iter().enumerate().take(end - 1).skip(start) {
            if gone {
                flush(&mut kept, &mut run_start);
                continue;
            }
            if run_start.is_none() {
                run_start = Some(kept.points_mm.len());
                map.points[piece] = kept_at(true, kept.points_mm.len());
                kept.points_mm.push(layer.points_mm[piece]);
            }
            map.points[piece + 1] = kept_at(true, kept.points_mm.len());
            kept.points_mm.push(layer.points_mm[piece + 1]);
        }
        flush(&mut kept, &mut run_start);
        start = end;
    }
    for (text, gone) in layer.texts.iter().zip(texts) {
        map.texts.push(kept_at(!gone, kept.texts.len()));
        if !gone {
            kept.texts.push(text.clone());
        }
    }
    for (fill, gone) in layer.fills.iter().zip(fills) {
        map.fills.push(kept_at(!gone, kept.fills.len()));
        if !gone {
            kept.fills.push(fill.clone());
        }
    }
    (kept, map)
}

/// Why objects of an underlay cannot be removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnderlayObjectError {
    /// The document has no underlay of this id, or no id is left.
    Plan(UnderlayPlanError),
    /// The underlay was read before its objects were kept: its lines are not objects.
    NoObjects,
    /// The drawing has no object of this id, or it was removed already.
    NotFound(u32),
    /// The object is part of an insert or dimension, the top-level object `parent`;
    /// removing it alone would change every placement of a block.
    Placed { object: u32, parent: u32 },
    /// No object was named.
    Empty,
    /// The underlay has no layer of this name.
    UnknownLayer(String),
    /// The shape at this index of those given cannot be drawn: a number not finite, a
    /// radius or height not above zero, too few points or a text without letters.
    InvalidShape(usize),
    /// The map is not a move, turn, mirror or uniform scale above zero.
    InvalidMap,
}

impl std::fmt::Display for UnderlayObjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(formatter),
            Self::NoObjects => formatter.write_str(
                "the drawing was imported before Kečup kept its objects; import it again to edit it",
            ),
            Self::NotFound(id) => write!(formatter, "the drawing has no object {id}"),
            Self::Placed { object, parent } => write!(
                formatter,
                "object {object} is part of object {parent}; remove {parent} instead"
            ),
            Self::Empty => formatter.write_str("no object was named"),
            Self::UnknownLayer(layer) => write!(formatter, "the drawing has no layer {layer:?}"),
            Self::InvalidShape(index) => write!(
                formatter,
                "shape {index} cannot be drawn: it needs finite numbers, a radius or height above zero, two points or more (three when closed) and a text with letters"
            ),
            Self::InvalidMap => formatter.write_str(
                "the map must move, turn, mirror or scale uniformly by more than zero",
            ),
        }
    }
}

impl std::error::Error for UnderlayObjectError {}

/// One batch, so one Undo step, that takes the top-level objects `ids` out of the drawing
/// of underlay `id`, whose lines as drawn now are `lines`; ids named twice count once.
pub fn plan_underlay_object_removal(
    snapshot: &Snapshot,
    id: super::UnderlayId,
    lines: &UnderlayLines,
    ids: &[u32],
) -> Result<CommandBatch, UnderlayObjectError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayObjectError::Plan(UnderlayPlanError::NotFound(id)))?;
    let objects = &lines.objects().objects;
    if objects.is_empty() {
        return Err(UnderlayObjectError::NoObjects);
    }
    if ids.is_empty() {
        return Err(UnderlayObjectError::Empty);
    }
    let mut edits = underlay.edits.clone();
    for object_id in ids {
        top_level(objects, *object_id)?;
        edits.removed_objects.insert(*object_id);
    }
    Ok(with_edits(underlay, edits))
}

/// One batch, so one Undo step, that draws `shapes` on the layer named `layer` of the
/// drawing of underlay `id`, each a new object.
pub fn plan_underlay_drawing(
    snapshot: &Snapshot,
    id: super::UnderlayId,
    layer: &str,
    shapes: Vec<DrawnShape>,
) -> Result<CommandBatch, UnderlayObjectError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayObjectError::Plan(UnderlayPlanError::NotFound(id)))?;
    if shapes.is_empty() {
        return Err(UnderlayObjectError::Empty);
    }
    if !underlay.layers.iter().any(|known| known.name == layer) {
        return Err(UnderlayObjectError::UnknownLayer(layer.to_owned()));
    }
    if let Some(index) = shapes.iter().position(|shape| !shape.is_valid()) {
        return Err(UnderlayObjectError::InvalidShape(index));
    }
    let mut edits = underlay.edits.clone();
    edits
        .added
        .extend(shapes.into_iter().map(|shape| AddedObject::Drawn {
            layer: layer.to_owned(),
            shape,
        }));
    Ok(with_edits(underlay, edits))
}

/// One batch, so one Undo step, that moves the top-level objects `ids` of the drawing of
/// underlay `id`, whose lines as drawn now are `lines`, by `map` (a move, turn, mirror or
/// uniform scale), or adds copies of them so moved when `copy`.
pub fn plan_underlay_object_transform(
    snapshot: &Snapshot,
    id: super::UnderlayId,
    lines: &UnderlayLines,
    ids: &[u32],
    map: &Affine,
    copy: bool,
) -> Result<CommandBatch, UnderlayObjectError> {
    let underlay = snapshot
        .underlay(id)
        .ok_or(UnderlayObjectError::Plan(UnderlayPlanError::NotFound(id)))?;
    let objects = &lines.objects().objects;
    if objects.is_empty() {
        return Err(UnderlayObjectError::NoObjects);
    }
    if ids.is_empty() {
        return Err(UnderlayObjectError::Empty);
    }
    if !is_similarity(map) {
        return Err(UnderlayObjectError::InvalidMap);
    }
    let mut edits = underlay.edits.clone();
    let unique = ids.iter().copied().collect::<BTreeSet<_>>();
    for object_id in unique {
        let object = top_level(objects, object_id)?;
        // What the object is now, as an added object or a map of a read one.
        let current = match object.added_by {
            Some(added_by) => edits
                .added
                .get(super::index(added_by))
                .cloned()
                .ok_or(UnderlayObjectError::NotFound(object_id))?,
            None => AddedObject::Copy {
                of: object_id,
                map: edits
                    .moved_objects
                    .get(&object_id)
                    .copied()
                    .unwrap_or(IDENTITY),
            },
        };
        let mapped = match current {
            AddedObject::Drawn { layer, shape } => AddedObject::Drawn {
                layer,
                shape: shape.transformed(map),
            },
            AddedObject::Copy { of, map: before } => AddedObject::Copy {
                of,
                map: then(&before, map),
            },
        };
        if copy {
            edits.added.push(mapped);
        } else if let Some(added_by) = object.added_by {
            edits.added[super::index(added_by)] = mapped;
        } else if let AddedObject::Copy { map, .. } = mapped {
            edits.moved_objects.insert(object_id, map);
        }
    }
    Ok(with_edits(underlay, edits))
}

const IDENTITY: Affine = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

/// Object `id` of `objects` when it is there and placed by none.
fn top_level(objects: &[UnderlayObject], id: u32) -> Result<&UnderlayObject, UnderlayObjectError> {
    let object = objects
        .get(super::index(id))
        .filter(|object| !object.removed)
        .ok_or(UnderlayObjectError::NotFound(id))?;
    if let Some(mut parent) = object.parent {
        while let Some(up) = objects.get(super::index(parent)).and_then(|p| p.parent) {
            parent = up;
        }
        return Err(UnderlayObjectError::Placed { object: id, parent });
    }
    Ok(object)
}

fn with_edits(underlay: &Underlay, edits: UnderlayEdits) -> CommandBatch {
    CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(Underlay {
        edits,
        ..underlay.clone()
    })])
}
