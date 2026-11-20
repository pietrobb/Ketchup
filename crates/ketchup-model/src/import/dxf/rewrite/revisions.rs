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
    /// The entities of the earlier that the later draws alike, in the same place, under
    /// another handle: ArchiCAD, Revit and Allplan number entities anew on each export
    /// (review 2026-11-10 og:R1110-UL-401), so they are neither removed nor added.
    pub renumbered: usize,
    /// The top-level entities of either without a handle, which nothing tells apart
    /// from another of their kind in a drawing changed since, so they are not compared.
    pub without_handle: usize,
    /// How many top-level entities with a handle the earlier and the later have.
    pub with_handle: [usize; 2],
}

impl RevisionComparison {
    /// Whether the two cannot be compared entity by entity: one of them, or both, has no
    /// entity with a handle, so every entity of one would read as removed and every one of
    /// the other as added (UL-401).
    #[must_use]
    pub fn lacks_handles(&self) -> bool {
        self.with_handle.contains(&0)
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
        with_handle: [then.len(), now.len()],
        ..RevisionComparison::default()
    };
    let mut removed = Vec::new();
    for (key, &id) in &then {
        let Some(&found) = now.get(key) else {
            removed.push(id);
            continue;
        };
        let difference = compare::difference_between(
            earlier.lines,
            id,
            earlier.offset_mm,
            later.lines,
            found,
            later.offset_mm,
        )
        .or_else(|| {
            compare::restyled(earlier.lines, id, later.lines, found).then_some(Difference::Style)
        });
        match difference {
            Some(difference) => comparison
                .changed
                .push(entity(later, found, Some(difference))),
            None => comparison.unchanged += 1,
        }
    }
    let added = now
        .iter()
        .filter(|(key, _)| !then.contains_key(key))
        .map(|(_, &id)| id);
    let (renumbered, added) = pair_renumbered(earlier, &removed, later, added);
    comparison.renumbered = renumbered.len();
    comparison.removed = removed
        .iter()
        .filter(|id| !renumbered.contains(id))
        .map(|&id| entity(earlier, id, None))
        .collect();
    comparison.added = added
        .into_iter()
        .map(|id| entity(later, id, None))
        .collect();
    comparison
}

/// Of `removed`, objects of `earlier`, those that an object of `added`, of `later`,
/// draws alike in the same place under another handle; and the objects of `added` left.
/// Each is looked for among those of its kind where it lies, not among all.
fn pair_renumbered(
    earlier: Revision<'_>,
    removed: &[usize],
    later: Revision<'_>,
    added: impl Iterator<Item = usize>,
) -> (std::collections::BTreeSet<usize>, Vec<usize>) {
    let kind = |lines: &UnderlayLines, id: usize| lines.objects().objects[id].kind.name();
    let mut by_place = BTreeMap::<_, Vec<usize>>::new();
    for &id in removed {
        let place = compare::place_mm(earlier.lines, id, earlier.offset_mm);
        by_place
            .entry((kind(earlier.lines, id), place))
            .or_default()
            .push(id);
    }
    let mut paired = std::collections::BTreeSet::new();
    let mut left = Vec::new();
    for found in added {
        let place = compare::place_mm(later.lines, found, later.offset_mm);
        let candidates = by_place.get_mut(&(kind(later.lines, found), place));
        let alike = candidates.and_then(|candidates| {
            let at = candidates.iter().position(|&id| {
                compare::difference_between(
                    earlier.lines,
                    id,
                    earlier.offset_mm,
                    later.lines,
                    found,
                    later.offset_mm,
                )
                .is_none()
                    && !compare::restyled(earlier.lines, id, later.lines, found)
            })?;
            Some(candidates.swap_remove(at))
        });
        match alike {
            Some(id) => {
                paired.insert(id);
            }
            None => left.push(found),
        }
    }
    (paired, left)
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
