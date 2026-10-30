//! What a drawing holds with its edits, counted without making them. A copy of an insert
//! places its block again, so a few edits can ask for many times the drawing: 100 copies
//! of an insert of 50 000 lines made 5.1 GB each time they were drawn (review 2026-10-29
//! MD-61). A drawing with its edits is held to what a drawing read may hold: its objects
//! to [`limits::DRAWING_ENTITIES`], the bytes its blob would take to
//! [`limits::UNDERLAY_LINES_BYTES`].

use std::collections::BTreeMap;

use ketchup_tolerance::limits;

use super::drawn::AddedObject;
use super::edits::UnderlayEdits;
use super::fill::UnderlayFill;
use super::objects::{ObjectPart, UnderlayObject};
use super::{LayerLines, UnderlayLines, index};

/// How many objects a drawing holds, and about how many bytes its blob takes for them: a
/// point 16 and at most 6 more for the end and style of its polyline, a text 80 and its
/// letters, a fill its ring ends and points, an object 21 and 28 for each part.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrawingSize {
    pub objects: usize,
    pub bytes: usize,
}

impl DrawingSize {
    /// Whether a drawing read may hold this much.
    #[must_use]
    pub const fn fits(self) -> bool {
        self.objects <= limits::DRAWING_ENTITIES && self.bytes <= limits::UNDERLAY_LINES_BYTES
    }

    pub(super) fn add(&mut self, other: Self) {
        self.objects = self.objects.saturating_add(other.objects);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }
}

const POINT_BYTES: usize = 22;
const TEXT_BYTES: usize = 80;
const OBJECT_BYTES: usize = 21;
const PART_BYTES: usize = 28;

fn fill_bytes(fill: &UnderlayFill) -> usize {
    12 + fill.ends.len() * 4 + fill.points_mm.len() * 16
}

fn layer_bytes(layer: &LayerLines) -> usize {
    layer.points_mm.len() * POINT_BYTES
        + layer
            .texts
            .iter()
            .map(|text| TEXT_BYTES + text.text.len())
            .sum::<usize>()
        + layer.fills.iter().map(fill_bytes).sum::<usize>()
}

impl UnderlayLines {
    /// The size of these lines.
    #[must_use]
    pub fn size(&self) -> DrawingSize {
        let lines = self.layers.iter().map(layer_bytes).sum::<usize>();
        let objects = self
            .objects
            .objects
            .iter()
            .map(|object| OBJECT_BYTES + object.parts.len() * PART_BYTES)
            .sum::<usize>();
        DrawingSize {
            objects: self.objects.objects.len(),
            bytes: lines + objects,
        }
    }

    /// The size of these lines with `edits` ([`Self::edited`]), counted without making
    /// it: each copy adds its object and all it placed again, each drawn shape one object.
    #[must_use]
    pub fn edited_size(&self, edits: &UnderlayEdits) -> DrawingSize {
        let mut size = self.size();
        size.add(self.added_size(&edits.added));
        size
    }

    /// What `added` adds to these lines, a copy counted by what its object holds here.
    pub(super) fn added_size(&self, added: &[AddedObject]) -> DrawingSize {
        let children = self.children_index();
        let mut copied = BTreeMap::new();
        let mut size = DrawingSize::default();
        for added in added {
            size.add(match added {
                AddedObject::Copy { of, .. } => *copied
                    .entry(*of)
                    .or_insert_with(|| self.tree_size(*of, &children)),
                AddedObject::Drawn { .. } => DrawingSize {
                    objects: 1,
                    bytes: OBJECT_BYTES + PART_BYTES,
                },
            });
        }
        size
    }

    /// Object `root` and all it placed.
    fn tree_size(&self, root: u32, children: &[Vec<u32>]) -> DrawingSize {
        let mut size = DrawingSize::default();
        let mut queue = vec![root];
        while let Some(id) = queue.pop() {
            let Some(object) = self.objects.objects.get(index(id)) else {
                continue;
            };
            size.add(DrawingSize {
                objects: 1,
                bytes: self.object_bytes(object),
            });
            queue.extend(children.get(index(id)).into_iter().flatten());
        }
        size
    }

    fn object_bytes(&self, object: &UnderlayObject) -> usize {
        OBJECT_BYTES
            + object
                .parts
                .iter()
                .map(|part| PART_BYTES + self.part_bytes(part))
                .sum::<usize>()
    }

    fn part_bytes(&self, part: &ObjectPart) -> usize {
        let Some(layer) = self.layers.get(index(part.layer)) else {
            return 0;
        };
        let range = |range: &std::ops::Range<u32>, len: usize| {
            index(range.start).min(len)..index(range.end).min(len)
        };
        range(&part.points, layer.points_mm.len()).len() * POINT_BYTES
            + layer.texts[range(&part.texts, layer.texts.len())]
                .iter()
                .map(|text| TEXT_BYTES + text.text.len())
                .sum::<usize>()
            + layer.fills[range(&part.fills, layer.fills.len())]
                .iter()
                .map(fill_bytes)
                .sum::<usize>()
    }
}
