//! Changes made in Kečup to an underlay's drawing: objects taken out of it. The blob of the
//! drawing stays as it was read, shared by every revision; the edits are a few numbers the
//! underlay carries, so the Undo step of an edit copies no lines. [`UnderlayLines::edited`]
//! gives the drawing with them, and everything that draws, snaps to or asks about an
//! underlay's lines reads that.

use std::collections::BTreeSet;

use super::objects::LayerMaps;
use super::{LayerLines, Underlay, UnderlayLines, UnderlayPlanError, kept_at};
use crate::document::{CanonicalCommand, CommandBatch, Snapshot};

/// What was changed of an underlay's drawing.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnderlayEdits {
    /// The top-level objects taken out, by their index among the objects of the lines; an
    /// insert or dimension takes out what it placed with it.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub removed_objects: BTreeSet<u32>,
}

impl UnderlayEdits {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.removed_objects.is_empty()
    }

    /// The bytes that name the edits in [`Underlay::lines_key`].
    fn key_bytes(&self) -> Vec<u8> {
        let mut bytes = b"removed".to_vec();
        for object in &self.removed_objects {
            bytes.extend_from_slice(&object.to_le_bytes());
        }
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
        Ok(UnderlayLines::decode(bytes)?.edited(&self.edits))
    }
}

impl UnderlayLines {
    /// These lines with `edits`: the lines, texts and fills of a removed object and of
    /// all it placed left out, the polylines they were part of split where they were.
    /// A removed object keeps its index, so the others keep theirs, but it holds nothing
    /// and is [`super::objects::UnderlayObject::removed`].
    #[must_use]
    pub fn edited(mut self, edits: &UnderlayEdits) -> Self {
        if edits.is_empty() {
            return self;
        }
        let objects = &self.objects.objects;
        let mut removed = vec![false; objects.len()];
        for (index, object) in objects.iter().enumerate() {
            let by_id = u32::try_from(index).is_ok_and(|id| edits.removed_objects.contains(&id));
            let by_parent = object
                .parent
                .and_then(|parent| removed.get(super::index(parent)).copied())
                .unwrap_or(false);
            removed[index] = by_id || by_parent;
        }
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
        for (object, _) in objects.iter().zip(&removed).filter(|(_, gone)| **gone) {
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
        for (object, gone) in self.objects.objects.iter_mut().zip(removed) {
            if gone {
                object.parts.clear();
                object.removed = true;
            }
        }
        self
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
        let object = objects
            .get(super::index(*object_id))
            .filter(|object| !object.removed)
            .ok_or(UnderlayObjectError::NotFound(*object_id))?;
        if let Some(mut parent) = object.parent {
            while let Some(up) = objects.get(super::index(parent)).and_then(|p| p.parent) {
                parent = up;
            }
            return Err(UnderlayObjectError::Placed {
                object: *object_id,
                parent,
            });
        }
        edits.removed_objects.insert(*object_id);
    }
    Ok(CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(
        Underlay {
            edits,
            ..underlay.clone()
        },
    )]))
}
