//! Two revisions of a drawing compared by handle (review 2026-11-08 §5.3): an architect
//! sends revision B of a plan the model was traced over revision A of. Each top-level
//! entity of B is matched with the entity of A of the same handle and kind: one in both
//! that draws otherwise is changed, one only in B added, one only in A removed. What they
//! draw makes an overlay, an underlay of three layers in their own colors over the
//! drawing, so the changes are seen where they lie.

use std::collections::BTreeMap;

use super::compare::{self, Difference};
use crate::underlay::style::LineStyle;
use crate::underlay::text::UnderlayText;
use crate::underlay::{UnderlayLines, UnderlayLinesBuilder};

/// One entity changed, added or removed between the revisions.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct RevisedEntity {
    /// Its handle as the drawing writes it, hexadecimal.
    pub handle: String,
    /// Its DXF entity name.
    pub kind: &'static str,
    pub layer: String,
    /// How a changed one draws otherwise, the first difference found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difference: Option<Difference>,
    /// Where it lies, to the millimetre, in the drawing's own coordinates; none where
    /// it draws nothing.
    pub at_mm: Option<[i64; 2]>,
}

/// What changed from one revision of a drawing to the next, entity by entity.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct RevisionComparison {
    pub changed: Vec<RevisedEntity>,
    pub added: Vec<RevisedEntity>,
    pub removed: Vec<RevisedEntity>,
    /// The entities both have that draw alike.
    pub unchanged: usize,
    /// The top-level entities of either without a handle, which nothing tells apart
    /// from another of their kind in a drawing changed since, so they are not compared.
    pub without_handle: usize,
}

impl RevisionComparison {
    /// Whether nothing was compared: neither drawing has an entity with a handle.
    #[must_use]
    pub fn compared_nothing(&self) -> bool {
        self.changed.is_empty()
            && self.added.is_empty()
            && self.removed.is_empty()
            && self.unchanged == 0
    }
}

/// A drawing's lines and what they were moved by from its own coordinates.
#[derive(Clone, Copy)]
pub struct Revision<'a> {
    pub lines: &'a UnderlayLines,
    pub offset_mm: [f64; 2],
}

/// The top-level objects of `lines` that have a handle, by handle and kind, and how many
/// have none.
fn handled(lines: &UnderlayLines) -> (BTreeMap<(u64, &'static str), usize>, usize) {
    let mut tops = BTreeMap::new();
    let mut without = 0;
    for (id, object) in lines.objects().objects.iter().enumerate() {
        if object.parent.is_some() || object.removed || object.added_by.is_some() {
            continue;
        }
        if object.handle == 0 {
            without += 1;
        } else {
            tops.entry((object.handle, object.kind.name()))
                .or_insert(id);
        }
    }
    (tops, without)
}

fn entity(revision: Revision<'_>, id: usize, difference: Option<Difference>) -> RevisedEntity {
    let objects = revision.lines.objects();
    let object = &objects.objects[id];
    RevisedEntity {
        handle: format!("{:X}", object.handle),
        kind: object.kind.name(),
        layer: objects.layer_name(object).to_owned(),
        difference,
        at_mm: compare::place_mm(revision.lines, id, revision.offset_mm),
    }
}

/// What changed from `earlier` to `later`, by the handles of their entities.
#[must_use]
pub fn compare_revisions(earlier: Revision<'_>, later: Revision<'_>) -> RevisionComparison {
    let ((then, then_without), (now, now_without)) = (handled(earlier.lines), handled(later.lines));
    let mut comparison = RevisionComparison {
        without_handle: then_without + now_without,
        ..RevisionComparison::default()
    };
    for (key, &id) in &then {
        let Some(&found) = now.get(key) else {
            comparison.removed.push(entity(earlier, id, None));
            continue;
        };
        match compare::difference_between(
            earlier.lines,
            id,
            earlier.offset_mm,
            later.lines,
            found,
            later.offset_mm,
        ) {
            Some(difference) => comparison
                .changed
                .push(entity(later, found, Some(difference))),
            None => comparison.unchanged += 1,
        }
    }
    for (key, &id) in &now {
        if !then.contains_key(key) {
            comparison.added.push(entity(later, id, None));
        }
    }
    comparison
}

/// The names of the overlay's layers, in the window's language.
#[derive(Clone, Debug)]
pub struct OverlayLayers {
    pub changed: String,
    pub added: String,
    pub removed: String,
}

/// The colors of the overlay's layers: changed orange, added green, removed red.
const CHANGED: [u8; 3] = [230, 120, 0];
const ADDED: [u8; 3] = [0, 160, 60];
const REMOVED: [u8; 3] = [210, 30, 30];
/// 0.5 mm on paper, heavier than the drawing's own lines.
const OVERLAY_WEIGHT_HMM: u16 = 50;

/// The overlay of `comparison` in the coordinates of `later`'s lines: what each changed
/// and added entity draws in `later`, and each removed one drew in `earlier`, on the layer
/// named for it, as objects of their handles.
#[must_use]
pub fn revision_overlay(
    earlier: Revision<'_>,
    later: Revision<'_>,
    comparison: &RevisionComparison,
    layers: &OverlayLayers,
) -> (Vec<String>, UnderlayLines) {
    let mut builder = UnderlayLinesBuilder::default();
    // The earlier drawing's points where the later's lines have them.
    let shift = [0, 1].map(|axis| earlier.offset_mm[axis] - later.offset_mm[axis]);
    let groups = [
        (
            &comparison.changed,
            later,
            [0.0, 0.0],
            &layers.changed,
            CHANGED,
        ),
        (&comparison.added, later, [0.0, 0.0], &layers.added, ADDED),
        (
            &comparison.removed,
            earlier,
            shift,
            &layers.removed,
            REMOVED,
        ),
    ];
    for (entities, revision, shift, layer, color) in groups {
        builder.set_style(&LineStyle {
            color: Some(color),
            weight_hmm: OVERLAY_WEIGHT_HMM,
            pattern_mm: Vec::new(),
        });
        let (tops, _) = handled(revision.lines);
        for revised in entities.iter() {
            let Some(&id) = u64::from_str_radix(&revised.handle, 16)
                .ok()
                .and_then(|handle| tops.get(&(handle, revised.kind)))
            else {
                continue;
            };
            let object = &revision.lines.objects().objects[id];
            let copy =
                builder.add_object(object.kind, object.handle, layer, None, Default::default());
            builder.set_object(Some(copy));
            copy_drawn(&mut builder, revision.lines, id, layer, shift);
        }
    }
    builder.set_object(None);
    builder.finish()
}

/// Adds what object `id` of `lines`, and every object it placed, draws to `layer`,
/// moved by `shift`.
fn copy_drawn(
    builder: &mut UnderlayLinesBuilder,
    lines: &UnderlayLines,
    id: usize,
    layer: &str,
    shift: [f64; 2],
) {
    let moved = |point: [f64; 2]| [point[0] + shift[0], point[1] + shift[1]];
    for drawn in compare::drawn_parts(lines, id) {
        for run in drawn.lines {
            let points = run.iter().copied().map(moved).collect::<Vec<_>>();
            builder.add_polyline(layer, &points);
        }
        for text in drawn.texts {
            builder.add_text(
                layer,
                UnderlayText {
                    anchor_mm: moved(text.anchor_mm),
                    ..text
                },
            );
        }
        for rings in drawn.fills {
            let rings = rings
                .iter()
                .map(|ring| ring.iter().copied().map(moved).collect())
                .collect::<Vec<_>>();
            builder.add_fill(layer, &rings);
        }
    }
}
