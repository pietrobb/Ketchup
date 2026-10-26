//! Questions about the objects of an underlay ([`super::objects`]): what each holds
//! (children, bounds, its own lines and texts), which object drew a line, and which
//! objects match a kind, layer, block, text or area. Everything is in the underlay's own
//! millimetres, before its placement.

use std::collections::BTreeMap;

use ketchup_geometry::linalg::dot2;

use super::UnderlayLines;
use super::objects::{ObjectDetail, ObjectPart, UnderlayObject};
use super::text::UnderlayText;

/// The children and bounds of every object of an underlay, worked out once.
pub struct ObjectIndex<'a> {
    lines: &'a UnderlayLines,
    children: Vec<Vec<u32>>,
    /// Of what it and all below it draw, `None` for an object that draws nothing.
    bounds: Vec<Option<[[f64; 2]; 2]>>,
}

/// What objects to list: each given field must match.
#[derive(Clone, Debug, Default)]
pub struct ObjectFilter {
    /// A DXF kind, `INSERT`, `LINE`, …, in any case.
    pub kind: Option<String>,
    /// A layer name, in any case.
    pub layer: Option<String>,
    /// Part of the name of the block an insert places, in any case.
    pub block: Option<String>,
    /// Part of a text it draws or below it, or of an attribute value, in any case.
    pub text: Option<String>,
    /// Only objects whose bounds meet this rectangle, `[[x, y], [x, y]]`.
    pub within_mm: Option<[[f64; 2]; 2]>,
    /// Only the children of this object; with `None`, `top_level` decides.
    pub parent: Option<u32>,
    /// Only objects nothing placed, when no parent is given.
    pub top_level: bool,
}

impl<'a> ObjectIndex<'a> {
    #[must_use]
    pub fn new(lines: &'a UnderlayLines) -> Self {
        let objects = &lines.objects().objects;
        let mut children = vec![Vec::new(); objects.len()];
        let mut bounds = objects
            .iter()
            .map(|object| own_bounds(lines, object))
            .collect::<Vec<_>>();
        // A parent comes before its children, so walking back folds each child's bounds,
        // already holding its own children's, into its parent's.
        for (index, object) in objects.iter().enumerate().rev() {
            if let Some(parent) = object.parent.map(as_index) {
                children[parent].push(u32::try_from(index).unwrap_or(u32::MAX));
                bounds[parent] = merge(bounds[parent], bounds[index]);
            }
        }
        for list in &mut children {
            list.reverse();
        }
        Self {
            lines,
            children,
            bounds,
        }
    }

    /// The object `id`, `None` for one that was removed ([`super::edits`]).
    #[must_use]
    pub fn object(&self, id: u32) -> Option<&'a UnderlayObject> {
        self.lines
            .objects()
            .objects
            .get(as_index(id))
            .filter(|object| !object.removed)
    }

    /// How many ids there are, removed objects' too.
    #[must_use]
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// How many objects there are, removed ones left out.
    #[must_use]
    pub fn present(&self) -> usize {
        self.live().count()
    }

    fn live(&self) -> impl Iterator<Item = &'a UnderlayObject> {
        self.lines
            .objects()
            .objects
            .iter()
            .filter(|object| !object.removed)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    #[must_use]
    pub fn children(&self, id: u32) -> &[u32] {
        self.children.get(as_index(id)).map_or(&[], Vec::as_slice)
    }

    /// The bounds of all it and the objects below it draw.
    #[must_use]
    pub fn bounds(&self, id: u32) -> Option<[[f64; 2]; 2]> {
        self.bounds.get(as_index(id)).copied().flatten()
    }

    /// The polylines it draws itself: each run of points of one polyline in its parts.
    #[must_use]
    pub fn own_polylines(&self, id: u32) -> Vec<&'a [[f64; 2]]> {
        let Some(object) = self.object(id) else {
            return Vec::new();
        };
        let mut runs = Vec::new();
        for part in &object.parts {
            let Some(layer) = self.lines.layers().get(as_index(part.layer)) else {
                continue;
            };
            let (start, end) = (as_index(part.points.start), as_index(part.points.end));
            let mut run_start = start;
            // A polyline ends inside the range where one of its ends lies.
            for polyline_end in layer.polyline_ends() {
                if *polyline_end > run_start && *polyline_end < end {
                    runs.push(&layer.points_mm()[run_start..*polyline_end]);
                    run_start = *polyline_end;
                }
            }
            if end > run_start {
                runs.push(&layer.points_mm()[run_start..end]);
            }
        }
        runs.retain(|run| run.len() >= 2);
        runs
    }

    /// The texts it draws itself.
    #[must_use]
    pub fn own_texts(&self, id: u32) -> Vec<&'a UnderlayText> {
        let Some(object) = self.object(id) else {
            return Vec::new();
        };
        object
            .parts
            .iter()
            .filter_map(|part| {
                let layer = self.lines.layers().get(as_index(part.layer))?;
                layer
                    .texts()
                    .get(as_index(part.texts.start)..as_index(part.texts.end))
            })
            .flatten()
            .collect()
    }

    /// The texts it and the objects below it draw, in order.
    #[must_use]
    pub fn texts_below(&self, id: u32) -> Vec<&'a UnderlayText> {
        let mut texts = self.own_texts(id);
        for child in self.children(id) {
            texts.extend(self.texts_below(*child));
        }
        texts
    }

    /// The object that drew the straight piece from point `point` to the next of layer
    /// `layer`: the one whose own lines hold both.
    #[must_use]
    pub fn object_drawing(&self, layer: usize, point: usize) -> Option<u32> {
        self.lines
            .objects()
            .objects
            .iter()
            .position(|object| {
                object.parts.iter().any(|part| {
                    as_index(part.layer) == layer
                        && as_index(part.points.start) <= point
                        && point + 1 < as_index(part.points.end)
                })
            })
            .and_then(|index| u32::try_from(index).ok())
    }

    /// The object drawn last of those that draw a text `text_holds` says holds `point_mm`
    /// on a layer `shown` keeps, or else of those that paint a fill there: a click on a
    /// label inside a hatched room picks the label. Texts and fills draw no straight
    /// pieces, so [`Self::object_drawing`] never finds them.
    #[must_use]
    pub fn object_with_area_at(
        &self,
        point_mm: [f64; 2],
        shown: impl Fn(usize) -> bool,
        text_holds: impl Fn(&UnderlayText, [f64; 2]) -> bool,
    ) -> Option<u32> {
        let objects = &self.lines.objects().objects;
        let layers = self.lines.layers();
        let hit = |holds: &dyn Fn(&super::LayerLines, &ObjectPart) -> bool| {
            objects
                .iter()
                .rposition(|object| {
                    !object.removed
                        && object.parts.iter().any(|part| {
                            let layer = as_index(part.layer);
                            shown(layer)
                                && layers.get(layer).is_some_and(|lines| holds(lines, part))
                        })
                })
                .and_then(|index| u32::try_from(index).ok())
        };
        let range = |range: &std::ops::Range<u32>| as_index(range.start)..as_index(range.end);
        hit(&|layer, part| {
            layer
                .texts()
                .get(range(&part.texts))
                .is_some_and(|texts| texts.iter().any(|text| text_holds(text, point_mm)))
        })
        .or_else(|| {
            hit(&|layer, part| {
                layer
                    .fills()
                    .get(range(&part.fills))
                    .is_some_and(|fills| fills.iter().any(|fill| fill.contains(point_mm)))
            })
        })
    }

    /// The ids of the objects `filter` keeps, in order.
    #[must_use]
    pub fn find(&self, filter: &ObjectFilter) -> Vec<u32> {
        let objects = &self.lines.objects().objects;
        let lower = |text: &Option<String>| text.as_ref().map(|text| text.to_lowercase());
        let (block, text) = (lower(&filter.block), lower(&filter.text));
        (0..objects.len())
            .filter_map(|index| u32::try_from(index).ok())
            .filter(|id| {
                let object = &objects[as_index(*id)];
                if object.removed {
                    return false;
                }
                let placed_right = match filter.parent {
                    Some(parent) => object.parent == Some(parent),
                    None => !filter.top_level || object.parent.is_none(),
                };
                placed_right
                    && filter
                        .kind
                        .as_ref()
                        .is_none_or(|kind| kind.eq_ignore_ascii_case(object.kind.name()))
                    && filter.layer.as_ref().is_none_or(|layer| {
                        layer.eq_ignore_ascii_case(self.lines.objects().layer_name(object))
                    })
                    && block.as_ref().is_none_or(|block| {
                        matches!(&object.detail, ObjectDetail::Insert { block: name, .. }
                            if name.to_lowercase().contains(block))
                    })
                    && filter
                        .within_mm
                        .is_none_or(|area| self.bounds(*id).is_some_and(|own| meets(own, area)))
                    && text.as_ref().is_none_or(|text| self.holds_text(*id, text))
            })
            .collect()
    }

    /// Whether it or an object below it draws a text holding `lower`, or it has an
    /// attribute whose value does.
    fn holds_text(&self, id: u32, lower: &str) -> bool {
        let attribute = self.object(id).is_some_and(|object| {
            matches!(&object.detail, ObjectDetail::Insert { attributes, .. }
                if attributes.iter().any(|(_, value)| value.to_lowercase().contains(lower)))
        });
        attribute
            || self
                .texts_below(id)
                .iter()
                .any(|text| text.text.to_lowercase().contains(lower))
    }

    /// The object at the top of the chain of objects that placed `id`.
    #[must_use]
    pub fn top_level_of(&self, mut id: u32) -> u32 {
        while let Some(parent) = self.object(id).and_then(|object| object.parent) {
            id = parent;
        }
        id
    }

    /// The top-level objects whose lines meet those of `id` and of the objects below it:
    /// a corner or line end of one within `tolerance` of a line of the other, or a
    /// dimension's definition point on a line. This is how lines join into a drawing and
    /// which lines a dimension measures. Hatches are left out: they fill areas, they do
    /// not join lines.
    #[must_use]
    pub fn touching(&self, id: u32, tolerance: f64) -> Vec<u32> {
        let top = self.top_level_of(id);
        let (points, segments) = self.joints(&self.below(id));
        let Some([low, high]) = self.bounds(id) else {
            return Vec::new();
        };
        let area = [
            [low[0] - tolerance, low[1] - tolerance],
            [high[0] + tolerance, high[1] + tolerance],
        ];
        let mut found = std::collections::BTreeSet::new();
        for other in 0..self.len() {
            let Ok(other) = u32::try_from(other) else {
                break;
            };
            let other_top = self.top_level_of(other);
            if other_top == top
                || found.contains(&other_top)
                || !self.bounds(other).is_some_and(|bounds| meets(bounds, area))
            {
                continue;
            }
            let (other_points, other_segments) = self.joints(&[other]);
            let meet = |points: &[[f64; 2]], segments: &[[[f64; 2]; 2]]| {
                points.iter().any(|point| {
                    segments
                        .iter()
                        .any(|[a, b]| within(*point, *a, *b, tolerance))
                })
            };
            if meet(&other_points, &segments) || meet(&points, &other_segments) {
                found.insert(other_top);
            }
        }
        found.into_iter().collect()
    }

    /// `id` and every object below it.
    fn below(&self, id: u32) -> Vec<u32> {
        let mut all = vec![id];
        let mut next = 0;
        while let Some(current) = all.get(next).copied() {
            all.extend_from_slice(self.children(current));
            next += 1;
        }
        all
    }

    /// The points where lines of `ids` may join others (corners, line ends and the
    /// definition points of dimensions) and their straight pieces; nothing of a hatch.
    fn joints(&self, ids: &[u32]) -> (Vec<[f64; 2]>, Vec<[[f64; 2]; 2]>) {
        let (mut points, mut segments) = (Vec::new(), Vec::new());
        for id in ids {
            match self.object(*id).map(|object| &object.detail) {
                None | Some(ObjectDetail::Hatch { .. }) => continue,
                Some(ObjectDetail::Dimension { points_mm, .. }) => {
                    points.extend(points_mm.iter().map(|(_, point)| *point));
                }
                Some(ObjectDetail::Insert { .. } | ObjectDetail::None) => {}
            }
            for run in self.own_polylines(*id) {
                points.extend_from_slice(run);
                segments.extend(run.windows(2).map(|pair| [pair[0], pair[1]]));
            }
        }
        (points, segments)
    }

    /// How many objects of each kind there are, top level and in all.
    #[must_use]
    pub fn kind_counts(&self) -> BTreeMap<&'static str, [usize; 2]> {
        let mut counts = BTreeMap::new();
        for object in self.live() {
            let count = counts.entry(object.kind.name()).or_insert([0, 0]);
            count[0] += usize::from(object.parent.is_none());
            count[1] += 1;
        }
        counts
    }

    /// Per block, how often an insert places it at the top level and in all.
    #[must_use]
    pub fn block_counts(&self) -> BTreeMap<&'a str, [usize; 2]> {
        let mut counts = BTreeMap::new();
        for object in self.live() {
            if let ObjectDetail::Insert { block, .. } = &object.detail {
                let count = counts.entry(block.as_str()).or_insert([0, 0]);
                count[0] += usize::from(object.parent.is_none());
                count[1] += 1;
            }
        }
        counts
    }

    /// Per layer, how many objects the drawing puts on it.
    #[must_use]
    pub fn layer_counts(&self) -> BTreeMap<&'a str, usize> {
        let objects = self.lines.objects();
        let mut counts = BTreeMap::new();
        for object in self.live() {
            *counts.entry(objects.layer_name(object)).or_insert(0) += 1;
        }
        counts
    }
}

fn as_index(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn merge(left: Option<[[f64; 2]; 2]>, right: Option<[[f64; 2]; 2]>) -> Option<[[f64; 2]; 2]> {
    match (left, right) {
        (Some([low, high]), Some([other_low, other_high])) => Some([
            [low[0].min(other_low[0]), low[1].min(other_low[1])],
            [high[0].max(other_high[0]), high[1].max(other_high[1])],
        ]),
        (one, None) | (None, one) => one,
    }
}

/// Whether `point` lies within `tolerance` of the piece from `a` to `b`.
fn within(point: [f64; 2], a: [f64; 2], b: [f64; 2], tolerance: f64) -> bool {
    let along = [b[0] - a[0], b[1] - a[1]];
    let from_a = [point[0] - a[0], point[1] - a[1]];
    let length2 = dot2(along, along);
    let t = if length2 > 0.0 {
        (dot2(from_a, along) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (from_a[0] - along[0] * t).hypot(from_a[1] - along[1] * t) <= tolerance
}

/// Whether two rectangles share a point.
fn meets(own: [[f64; 2]; 2], area: [[f64; 2]; 2]) -> bool {
    let [low, high] = area;
    let low = [low[0].min(high[0]), low[1].min(high[1])];
    let high = [area[0][0].max(area[1][0]), area[0][1].max(area[1][1])];
    own[0][0] <= high[0] && own[1][0] >= low[0] && own[0][1] <= high[1] && own[1][1] >= low[1]
}

/// The bounds of what `object` draws itself: its points, fills, text anchors and the
/// points of its detail.
fn own_bounds(lines: &UnderlayLines, object: &UnderlayObject) -> Option<[[f64; 2]; 2]> {
    if object.removed {
        return None;
    }
    let mut bounds = None;
    let mut add = |point: [f64; 2]| bounds = merge(bounds, Some([point, point]));
    for part in &object.parts {
        let Some(layer) = lines.layers().get(as_index(part.layer)) else {
            continue;
        };
        let range = |range: &std::ops::Range<u32>| as_index(range.start)..as_index(range.end);
        layer
            .points_mm()
            .get(range(&part.points))
            .into_iter()
            .flatten()
            .for_each(|point| add(*point));
        for fill in layer.fills().get(range(&part.fills)).into_iter().flatten() {
            fill.points_mm().iter().for_each(|point| add(*point));
        }
        for text in layer.texts().get(range(&part.texts)).into_iter().flatten() {
            add(text.anchor_mm);
        }
    }
    match &object.detail {
        ObjectDetail::Hatch { rings, .. } => rings.iter().flatten().for_each(|point| add(*point)),
        ObjectDetail::Dimension { points_mm, .. } => {
            points_mm.iter().for_each(|(_, point)| add(*point));
        }
        ObjectDetail::Insert { .. } | ObjectDetail::None => {}
    }
    bounds
}
