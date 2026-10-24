//! `inspect action=underlay_regions` and `underlay_parallel`: what an underlay's lines
//! form rather than what they are: the closed regions the lines of its shown (or named)
//! layers enclose, each with its area, holes and the texts inside it, and the pairs of
//! lines that run side by side at a distance. Hatch pattern lines and dimensions are left
//! out: they draw on areas, they do not bound them.

use super::*;
use ketchup_model::underlay::object_query::ObjectIndex;
use ketchup_model::underlay::objects::ObjectDetail;
use ketchup_model::underlay::regions::{
    DEFAULT_JOIN_MM, MAX_JOIN_MM, Region, find_regions, parallel_pairs,
};
use ketchup_model::underlay::{Underlay, UnderlayId, UnderlayLines};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;
/// Regions smaller than this are left out unless `min_area_m2` says otherwise.
const DEFAULT_MIN_AREA_M2: f64 = 0.1;
/// The most texts a region row quotes.
const ROW_TEXTS: usize = 8;
const MAX_OUTLINE_POINTS: usize = 4_000;

/// One straight piece, in the underlay's millimetres, with its layer.
struct Piece {
    ends: [[f64; 2]; 2],
    layer: usize,
    /// Its first point among the layer's.
    point: usize,
}

impl LiveBridge {
    pub(super) fn underlay_regions(
        app: &KetchupApp,
        request: &Request,
    ) -> Result<Value, &'static str> {
        let Request::UnderlayRegions {
            expected,
            underlay_id,
            layers,
            within_mm,
            point_mm,
            min_area_m2,
            tolerance_mm,
            include_outline,
            limit,
            offset,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        let tolerance = tolerance_mm.unwrap_or(DEFAULT_JOIN_MM);
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        let min_area_m2 = min_area_m2.unwrap_or(DEFAULT_MIN_AREA_M2);
        if !(tolerance > 0.0 && tolerance <= MAX_JOIN_MM)
            || !(1..=MAX_LIMIT).contains(&limit)
            || !min_area_m2.is_finite()
            || point_mm.is_some_and(|point| !point.iter().all(|value| value.is_finite()))
        {
            return Err("invalid_params");
        }
        let snapshot = app.document.current();
        let underlay = one_underlay(&snapshot, *underlay_id)?;
        let lines = app
            .cached_underlay_lines(underlay)
            .ok_or("entity_not_found")?;
        let chosen = chosen_layers(&snapshot, underlay, layers.as_deref())?;
        let pieces = pieces(
            &lines,
            &chosen,
            within_mm.map(|area| local_area(underlay, area)),
        );
        let segments = pieces.iter().map(|piece| piece.ends).collect::<Vec<_>>();
        let scale = underlay.scale();
        // The tolerance is document millimetres; the lines are the underlay's.
        let mut regions = find_regions(&segments, tolerance / scale)
            .map_err(|error| failed_because("invalid_params", error))?;
        let area_scale = scale * scale;
        regions.retain(|region| region.area_mm2 * area_scale / 1e6 >= min_area_m2);
        let texts = texts_in(&lines, &chosen_text_layers(&snapshot, underlay), &regions);
        let local_point = point_mm.map(|point| underlay.to_local(point));
        let mut order = (0..regions.len()).collect::<Vec<_>>();
        if let Some(point) = local_point {
            // The innermost region holding the point first.
            order.retain(|index| regions[*index].contains(point));
            order.sort_by(|a, b| regions[*a].area_mm2.total_cmp(&regions[*b].area_mm2));
        }
        let offset = offset.unwrap_or(0);
        let rows = order
            .iter()
            .skip(offset)
            .take(limit)
            .map(|index| {
                region_row(
                    underlay,
                    *index,
                    &regions[*index],
                    &texts[*index],
                    include_outline.unwrap_or(point_mm.is_some()),
                )
            })
            .collect::<Vec<_>>();
        let next = offset + rows.len();
        Ok(json!({
            "underlay_id": underlay.id.0,
            "layers": chosen.iter().map(|index| &underlay.layers[*index].name).collect::<Vec<_>>(),
            "pieces": segments.len(),
            "tolerance_mm": tolerance,
            "matched": order.len(),
            "regions": rows,
            "next_offset": (next < order.len()).then_some(next),
        }))
    }

    pub(super) fn underlay_parallel(
        app: &KetchupApp,
        request: &Request,
    ) -> Result<Value, &'static str> {
        let Request::UnderlayParallel {
            expected,
            underlay_id,
            layers,
            within_mm,
            min_gap_mm,
            max_gap_mm,
            min_overlap_mm,
            angle_deg,
            limit,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        let (min_gap, max_gap) = (min_gap_mm.unwrap_or(1.0), max_gap_mm.unwrap_or(1_000.0));
        let min_overlap = min_overlap_mm.unwrap_or(100.0);
        let angle = angle_deg.unwrap_or(1.0);
        if !(1..=MAX_LIMIT).contains(&limit)
            || !(min_gap >= 0.0 && min_gap <= max_gap && max_gap <= 10_000.0)
            || !(min_overlap >= 0.0 && min_overlap.is_finite())
            || !(0.0..=45.0).contains(&angle)
        {
            return Err("invalid_params");
        }
        let snapshot = app.document.current();
        let underlay = one_underlay(&snapshot, *underlay_id)?;
        let lines = app
            .cached_underlay_lines(underlay)
            .ok_or("entity_not_found")?;
        let chosen = chosen_layers(&snapshot, underlay, layers.as_deref())?;
        let pieces = pieces(
            &lines,
            &chosen,
            within_mm.map(|area| local_area(underlay, area)),
        );
        let segments = pieces.iter().map(|piece| piece.ends).collect::<Vec<_>>();
        let scale = underlay.scale();
        // Counted past the page, so the histogram covers every pair up to a bound.
        let pairs = parallel_pairs(
            &segments,
            min_gap / scale,
            max_gap / scale,
            angle,
            min_overlap / scale,
            limit.max(20_000),
        );
        // Per gap rounded to the millimetre: how many pairs and how far they run alongside.
        let mut by_gap = std::collections::BTreeMap::<String, (usize, f64)>::new();
        for pair in &pairs {
            let entry = by_gap
                .entry(format!("{:.0}", pair.gap_mm * scale))
                .or_default();
            entry.0 += 1;
            entry.1 += pair.overlap_mm * scale / 1_000.0;
        }
        let mut gaps = by_gap.into_iter().collect::<Vec<_>>();
        gaps.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
        gaps.truncate(30);
        let gaps = gaps
            .into_iter()
            .map(|(gap, (count, metres))| json!({"gap_mm": gap.parse::<f64>().unwrap_or(0.0), "pairs": count, "alongside_m": metres}))
            .collect::<Vec<_>>();
        let index = ObjectIndex::new(&lines);
        let piece_json = |index_of: usize| {
            let piece = &pieces[index_of];
            json!({
                "object_id": index.object_drawing(piece.layer, piece.point),
                "layer": underlay.layers[piece.layer].name,
                "from_mm": underlay.to_document(piece.ends[0]),
                "to_mm": underlay.to_document(piece.ends[1]),
            })
        };
        Ok(json!({
            "underlay_id": underlay.id.0,
            "layers": chosen.iter().map(|index| &underlay.layers[*index].name).collect::<Vec<_>>(),
            "pieces": segments.len(),
            "pairs_found": pairs.len(),
            "gaps": gaps,
            "pairs": pairs.iter().take(limit).map(|pair| json!({
                "gap_mm": pair.gap_mm * scale,
                "alongside_mm": pair.overlap_mm * scale,
                "a": piece_json(pair.a),
                "b": piece_json(pair.b),
            })).collect::<Vec<_>>(),
        }))
    }
}

/// The underlay `id` names, or the document's only one.
pub(super) fn one_underlay(
    snapshot: &Snapshot,
    id: Option<u64>,
) -> Result<&Underlay, &'static str> {
    match id {
        Some(id) => snapshot.underlay(UnderlayId(id)).ok_or("entity_not_found"),
        None => {
            let mut all = snapshot.underlays();
            match (all.next(), all.next()) {
                (Some(only), None) => Ok(only),
                (None, _) => Err("entity_not_found"),
                (Some(_), Some(_)) => Err("invalid_params"),
            }
        }
    }
}

/// The indices of the layers named (any case), or of every shown one.
fn chosen_layers(
    snapshot: &Snapshot,
    underlay: &Underlay,
    names: Option<&[String]>,
) -> Result<Vec<usize>, &'static str> {
    match names {
        Some(names) => names
            .iter()
            .map(|name| {
                underlay
                    .layers
                    .iter()
                    .position(|layer| layer.name.eq_ignore_ascii_case(name))
                    .ok_or("entity_not_found")
            })
            .collect(),
        None => Ok(chosen_text_layers(snapshot, underlay)),
    }
}

/// The layers shown: the texts on them are what a region says.
fn chosen_text_layers(snapshot: &Snapshot, underlay: &Underlay) -> Vec<usize> {
    let shown = |tag| snapshot.tag(tag).is_some_and(|tag| tag.visible());
    if !shown(underlay.tag) {
        return Vec::new();
    }
    (0..underlay.layers.len())
        .filter(|index| shown(underlay.layers[*index].tag))
        .collect()
}

fn local_area(underlay: &Underlay, area: [[f64; 2]; 2]) -> [[f64; 2]; 2] {
    let [low, high] = area;
    let corners = [
        [low[0], low[1]],
        [high[0], low[1]],
        [low[0], high[1]],
        [high[0], high[1]],
    ]
    .map(|corner| underlay.to_local(corner));
    let mut bounds = [corners[0], corners[0]];
    for corner in corners {
        bounds[0] = [bounds[0][0].min(corner[0]), bounds[0][1].min(corner[1])];
        bounds[1] = [bounds[1][0].max(corner[0]), bounds[1][1].max(corner[1])];
    }
    bounds
}

/// The straight pieces of `layers` that meet `area`, without those a hatch draws or a
/// dimension.
fn pieces(lines: &UnderlayLines, layers: &[usize], area: Option<[[f64; 2]; 2]>) -> Vec<Piece> {
    let objects = &lines.objects().objects;
    // Whether each object is a hatch or dimension or lies below one.
    let mut annotation = vec![false; objects.len()];
    for (index, object) in objects.iter().enumerate() {
        let own = matches!(
            object.detail,
            ObjectDetail::Hatch { .. } | ObjectDetail::Dimension { .. }
        );
        let inherited = object
            .parent
            .and_then(|parent| annotation.get(usize::try_from(parent).ok()?).copied())
            .unwrap_or(false);
        annotation[index] = own || inherited;
    }
    let mut skipped = vec![Vec::new(); lines.layers().len()];
    for (object, _) in objects.iter().zip(&annotation).filter(|(_, skip)| **skip) {
        for part in &object.parts {
            if let Some(ranges) = usize::try_from(part.layer)
                .ok()
                .and_then(|layer| skipped.get_mut(layer))
            {
                ranges.push(part.points.clone());
            }
        }
    }
    let meets = |[a, b]: [[f64; 2]; 2]| {
        area.is_none_or(|[low, high]| {
            a[0].min(b[0]) <= high[0]
                && a[0].max(b[0]) >= low[0]
                && a[1].min(b[1]) <= high[1]
                && a[1].max(b[1]) >= low[1]
        })
    };
    let mut pieces = Vec::new();
    for &layer in layers {
        let Some(drawn) = lines.layers().get(layer) else {
            continue;
        };
        let points = drawn.points_mm();
        let skip = |point: usize| {
            skipped[layer].iter().any(|range| {
                let start = usize::try_from(range.start).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end).unwrap_or(0);
                point >= start && point + 1 < end
            })
        };
        let mut start = 0;
        for &end in drawn.polyline_ends() {
            for point in start..end.saturating_sub(1) {
                let ends = [points[point], points[point + 1]];
                if !skip(point) && meets(ends) {
                    pieces.push(Piece { ends, layer, point });
                }
            }
            start = end;
        }
    }
    pieces
}

/// For each region, the texts of `layers` whose anchor it is the innermost region of.
fn texts_in(lines: &UnderlayLines, layers: &[usize], regions: &[Region]) -> Vec<Vec<String>> {
    let mut texts = vec![Vec::new(); regions.len()];
    for &layer in layers {
        let Some(drawn) = lines.layers().get(layer) else {
            continue;
        };
        for text in drawn.texts() {
            let innermost = regions
                .iter()
                .enumerate()
                .filter(|(_, region)| region.contains(text.anchor_mm))
                .min_by(|a, b| a.1.area_mm2.total_cmp(&b.1.area_mm2))
                .map(|(index, _)| index);
            if let Some(index) = innermost {
                texts[index].push(text.text.clone());
            }
        }
    }
    texts
}

fn region_row(
    underlay: &Underlay,
    index: usize,
    region: &Region,
    texts: &[String],
    outline: bool,
) -> Value {
    let scale = underlay.scale();
    let [low, high] = region.bounds;
    let corners = [
        [low[0], low[1]],
        [high[0], high[1]],
        [low[0], high[1]],
        [high[0], low[1]],
    ]
    .map(|corner| underlay.to_document(corner));
    let mut bounds = [corners[0], corners[0]];
    for corner in corners {
        bounds[0] = [bounds[0][0].min(corner[0]), bounds[0][1].min(corner[1])];
        bounds[1] = [bounds[1][0].max(corner[0]), bounds[1][1].max(corner[1])];
    }
    let perimeter = region
        .outline
        .iter()
        .zip(region.outline.iter().cycle().skip(1))
        .map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1]))
        .sum::<f64>()
        * scale;
    let mut row = json!({
        "region": index,
        "area_m2": region.area_mm2 * scale * scale / 1e6,
        "perimeter_mm": perimeter,
        "bounds_mm": bounds,
        "corners": region.outline.len(),
        "holes": region.holes.len(),
        "texts": &texts[..texts.len().min(ROW_TEXTS)],
        "texts_total": texts.len(),
    });
    if outline {
        let mut budget = MAX_OUTLINE_POINTS;
        let mut take = |ring: &[[f64; 2]]| {
            let taken = ring.len().min(budget);
            budget -= taken;
            ring[..taken]
                .iter()
                .map(|point| underlay.to_document(*point))
                .collect::<Vec<_>>()
        };
        let fields = row.as_object_mut().expect("a JSON object");
        fields.insert("outline_mm".into(), json!(take(&region.outline)));
        fields.insert(
            "holes_mm".into(),
            region
                .holes
                .iter()
                .map(|hole| take(hole))
                .collect::<Value>(),
        );
        fields.insert("outline_truncated".into(), json!(budget == 0));
    }
    row
}
