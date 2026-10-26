//! `inspect action=underlay_objects`: what an underlay's drawing is made of, as objects
//! rather than lines: counts by kind, block and layer, a page of the objects a filter keeps,
//! and one object in full (an insert's block, placement and attributes, a dimension's
//! measurement and points, a hatch's outline and area, the lines and texts it draws).
//! Points are document millimetres, with `drawing_*` in the drawing's own coordinates.

use super::*;
use ketchup_geometry::linalg::cross2;
use ketchup_model::underlay::object_query::{ObjectFilter, ObjectIndex};
use ketchup_model::underlay::objects::{ObjectDetail, UnderlayObject, apply, even_odd_area};
use ketchup_model::underlay::{Underlay, UnderlayLines};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;
/// The most blocks the counts name, most placed first.
const MAX_BLOCKS: usize = 100;
/// The most points one object's detail gives of its lines and rings.
const MAX_DETAIL_POINTS: usize = 4_000;
const MAX_CHILDREN: usize = 500;
const MAX_TOUCHING: usize = 200;
/// The most characters a row quotes of the text it draws.
const ROW_TEXT_CHARS: usize = 120;

impl LiveBridge {
    pub(super) fn underlay_objects(
        app: &KetchupApp,
        request: &Request,
    ) -> Result<Value, &'static str> {
        let Request::UnderlayObjects {
            expected,
            underlay_id,
            object_id,
            object_kind,
            layer,
            block,
            text,
            within_mm,
            parent_id,
            top_level,
            limit,
            offset,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        let snapshot = app.document.current();
        let underlay = super::underlay_regions::one_underlay(&snapshot, *underlay_id)?;
        let lines = app
            .cached_underlay_lines(underlay)
            .ok_or("entity_not_found")?;
        let index = ObjectIndex::new(&lines);
        let place = Placed { underlay };
        if let Some(id) = object_id {
            index.object(*id).ok_or("entity_not_found")?;
            return Ok(json!({
                "underlay_id": underlay.id.0,
                "object": object_detail(&index, &lines, &place, *id),
            }));
        }
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit)
            || within_mm.is_some_and(|area| !area.iter().flatten().all(|value| value.is_finite()))
        {
            return Err("invalid_params");
        }
        if parent_id.is_some_and(|parent| index.object(parent).is_none()) {
            return Err("entity_not_found");
        }
        let filter = ObjectFilter {
            kind: object_kind.clone(),
            layer: layer.clone(),
            block: block.clone(),
            text: text.clone(),
            within_mm: within_mm.map(|area| place.local_area(area)),
            parent: *parent_id,
            // Without a parent, the top level unless asked otherwise: the objects the
            // drawing places, not the lines inside its blocks.
            top_level: top_level.unwrap_or(true),
        };
        let found = index.find(&filter);
        let offset = offset.unwrap_or(0);
        let rows = found
            .iter()
            .skip(offset)
            .take(limit)
            .map(|id| object_row(&index, &lines, &place, *id))
            .collect::<Vec<_>>();
        let next = offset + rows.len();
        let mut answer = json!({
            "underlay_id": underlay.id.0,
            "name": underlay.name,
            "objects_total": index.present(),
            "matched": found.len(),
            "objects": rows,
            "next_offset": (next < found.len()).then_some(next),
        });
        // The counts once, with the first page of everything: a narrowed question asks
        // about its objects, not the drawing again.
        let narrowed = object_kind.is_some()
            || layer.is_some()
            || block.is_some()
            || text.is_some()
            || within_mm.is_some()
            || parent_id.is_some();
        if !narrowed && offset == 0 {
            let mut blocks = index.block_counts().into_iter().collect::<Vec<_>>();
            blocks.sort_by(|a, b| b.1[1].cmp(&a.1[1]).then(a.0.cmp(b.0)));
            let fields = answer.as_object_mut().expect("a JSON object");
            fields.insert("blocks_total".into(), json!(blocks.len()));
            blocks.truncate(MAX_BLOCKS);
            // [top level, in all] per kind and per block.
            fields.insert("kinds".into(), json!(index.kind_counts()));
            fields.insert(
                "blocks".into(),
                blocks
                    .into_iter()
                    .map(|(name, [top, all])| json!({"block": name, "top_level": top, "all": all}))
                    .collect(),
            );
            fields.insert("layers".into(), json!(index.layer_counts()));
        }
        Ok(answer)
    }
}

/// From the underlay's lines to the document and to the drawing's own coordinates.
pub(super) struct Placed<'a> {
    pub(super) underlay: &'a Underlay,
}

impl Placed<'_> {
    fn document(&self, local: [f64; 2]) -> [f64; 2] {
        self.underlay.to_document(local)
    }

    fn drawing(&self, local: [f64; 2]) -> [f64; 2] {
        let offset = self.underlay.offset_mm();
        [local[0] + offset[0], local[1] + offset[1]]
    }

    fn area_scale(&self) -> f64 {
        self.underlay.scale().powi(2)
    }

    /// The box of the underlay's lines that holds the document box `area`.
    pub(super) fn local_area(&self, area: [[f64; 2]; 2]) -> [[f64; 2]; 2] {
        let [low, high] = area;
        bounds_of_points(
            [
                [low[0], low[1]],
                [high[0], low[1]],
                [low[0], high[1]],
                [high[0], high[1]],
            ]
            .map(|corner| self.underlay.to_local(corner)),
        )
    }

    /// The document box that holds the local box `bounds`.
    fn document_bounds(&self, bounds: [[f64; 2]; 2]) -> [[f64; 2]; 2] {
        let [low, high] = bounds;
        bounds_of_points(
            [
                [low[0], low[1]],
                [high[0], low[1]],
                [low[0], high[1]],
                [high[0], high[1]],
            ]
            .map(|corner| self.document(corner)),
        )
    }
}

fn bounds_of_points(points: [[f64; 2]; 4]) -> [[f64; 2]; 2] {
    let mut bounds = [points[0], points[0]];
    for point in points {
        for axis in 0..2 {
            bounds[0][axis] = bounds[0][axis].min(point[axis]);
            bounds[1][axis] = bounds[1][axis].max(point[axis]);
        }
    }
    bounds
}

fn object_of<'a>(index: &ObjectIndex<'a>, id: u32) -> &'a UnderlayObject {
    index.object(id).expect("an object the index found")
}

/// One object in a list: what it is, where, and what it says.
fn object_row(
    index: &ObjectIndex<'_>,
    lines: &UnderlayLines,
    place: &Placed<'_>,
    id: u32,
) -> Value {
    let object = object_of(index, id);
    let texts = index.texts_below(id);
    let mut quoted = texts
        .iter()
        .map(|text| text.text.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    if let Some((cut, _)) = quoted.char_indices().nth(ROW_TEXT_CHARS) {
        quoted.truncate(cut);
        quoted.push('…');
    }
    let mut row = json!({
        "id": id,
        "kind": object.kind.name(),
        "layer": lines.objects().layer_name(object),
        "handle": handle(object.handle),
        "parent": object.parent,
        "children": index.children(id).len(),
        "bounds_mm": index.bounds(id).map(|bounds| place.document_bounds(bounds)),
        "text": (!quoted.is_empty()).then_some(quoted),
    });
    let fields = row.as_object_mut().expect("a JSON object");
    if texts.len() == 1 {
        let centres = crate::renderer::underlay::text::letter_centres(texts[0]);
        if let Some(first) = centres.first() {
            let bounds = centres.iter().fold([*first, *first], |mut bounds, point| {
                for axis in 0..2 {
                    bounds[0][axis] = bounds[0][axis].min(point[axis]);
                    bounds[1][axis] = bounds[1][axis].max(point[axis]);
                }
                bounds
            });
            let centre = [0, 1].map(|axis| (bounds[0][axis] + bounds[1][axis]) / 2.0);
            fields.insert("text_point_mm".into(), json!(place.document(centre)));
            fields.insert(
                "text_anchor_mm".into(),
                json!(place.document(texts[0].anchor_mm)),
            );
        }
    }
    match &object.detail {
        ObjectDetail::Insert {
            block,
            attributes,
            transform,
            ..
        } => {
            fields.insert("block".into(), json!(block));
            if !attributes.is_empty() {
                fields.insert("attributes".into(), attributes_json(attributes));
            }
            // Where and how it places the block, so one page gives every placement.
            let origin = apply(transform, [0.0, 0.0]);
            let x_axis = [transform[0][0], transform[1][0]];
            let y_axis = [transform[0][1], transform[1][1]];
            fields.insert("insertion_mm".into(), json!(place.document(origin)));
            fields.insert(
                "rotation_deg".into(),
                json!(x_axis[1].atan2(x_axis[0]).to_degrees()),
            );
            fields.insert(
                "scale".into(),
                json!([x_axis[0].hypot(x_axis[1]), y_axis[0].hypot(y_axis[1])]),
            );
            fields.insert("mirrored".into(), json!(cross2(x_axis, y_axis) < 0.0));
        }
        ObjectDetail::Dimension { measurement, .. } => {
            fields.insert("measurement".into(), json!(measurement));
        }
        ObjectDetail::Hatch { pattern, rings, .. } => {
            fields.insert("pattern".into(), json!(pattern));
            fields.insert(
                "area_m2".into(),
                json!(even_odd_area(rings) * place.area_scale() / 1e6),
            );
        }
        ObjectDetail::None => {}
    }
    row
}

/// One object in full.
fn object_detail(
    index: &ObjectIndex<'_>,
    lines: &UnderlayLines,
    place: &Placed<'_>,
    id: u32,
) -> Value {
    let object = object_of(index, id);
    let mut row = object_row(index, lines, place, id);
    let fields = row.as_object_mut().expect("a JSON object");
    fields.remove("text");
    let children = index.children(id);
    fields.insert(
        "child_ids".into(),
        json!(&children[..children.len().min(MAX_CHILDREN)]),
    );
    fields.insert(
        "drawing_bounds_mm".into(),
        json!(
            index
                .bounds(id)
                .map(|[low, high]| [place.drawing(low), place.drawing(high)])
        ),
    );
    let mut budget = MAX_DETAIL_POINTS;
    let mut take = |points: &[[f64; 2]]| {
        let taken = points.len().min(budget);
        budget -= taken;
        points[..taken]
            .iter()
            .map(|point| place.document(*point))
            .collect::<Vec<_>>()
    };
    match &object.detail {
        ObjectDetail::Insert {
            transform, copies, ..
        } => {
            let origin = apply(transform, [0.0, 0.0]);
            fields.insert("drawing_insertion_mm".into(), json!(place.drawing(origin)));
            fields.insert("copies".into(), json!(copies));
        }
        ObjectDetail::Dimension {
            dimension_type,
            text_override,
            points_mm,
            ..
        } => {
            fields.insert(
                "dimension_type".into(),
                json!(dimension_type_name(*dimension_type)),
            );
            fields.insert("text_override".into(), json!(text_override));
            fields.insert(
                "points".into(),
                points_mm
                    .iter()
                    .map(|(code, point)| {
                        json!({"group": code, "point_mm": place.document(*point), "drawing_mm": place.drawing(*point)})
                    })
                    .collect::<Value>(),
            );
        }
        ObjectDetail::Hatch { solid, rings, .. } => {
            fields.insert("solid".into(), json!(solid));
            fields.insert(
                "rings_mm".into(),
                rings.iter().map(|ring| take(ring)).collect::<Value>(),
            );
        }
        ObjectDetail::None => {}
    }
    fields.insert(
        "texts".into(),
        index
            .own_texts(id)
            .iter()
            .map(|text| json!({"text": text.text, "anchor_mm": place.document(text.anchor_mm), "height_mm": text.height_mm * place.underlay.scale()}))
            .collect::<Value>(),
    );
    let polylines = index.own_polylines(id);
    let point_total = polylines.iter().map(|run| run.len()).sum::<usize>();
    fields.insert(
        "polylines_mm".into(),
        polylines.iter().map(|run| take(run)).collect::<Value>(),
    );
    fields.insert(
        "length_mm".into(),
        json!(
            polylines
                .iter()
                .flat_map(|run| run.windows(2))
                .map(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]))
                .sum::<f64>()
                * place.underlay.scale()
        ),
    );
    let touching = index.touching(
        id,
        ketchup_model::underlay::regions::DEFAULT_JOIN_MM / place.underlay.scale(),
    );
    fields.insert(
        "touching".into(),
        json!(&touching[..touching.len().min(MAX_TOUCHING)]),
    );
    fields.insert("touching_total".into(), json!(touching.len()));
    fields.insert("points_total".into(), json!(point_total));
    fields.insert("points_truncated".into(), json!(budget == 0));
    row
}

fn handle(handle: u64) -> Option<String> {
    (handle != 0).then(|| format!("{handle:X}"))
}

fn attributes_json(attributes: &[(String, String)]) -> Value {
    attributes
        .iter()
        .map(|(tag, value)| json!({"tag": tag, "value": value}))
        .collect()
}

fn dimension_type_name(dimension_type: u8) -> &'static str {
    match dimension_type {
        0 => "rotated",
        1 => "aligned",
        2 => "angular",
        3 => "diameter",
        4 => "radius",
        5 => "angular_3_point",
        6 => "ordinate",
        _ => "unknown",
    }
}
