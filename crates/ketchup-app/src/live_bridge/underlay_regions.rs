//! `inspect action=underlay_regions` and `underlay_parallel`: what an underlay's lines
//! form rather than what they are: the closed regions the lines of its shown (or named)
//! layers enclose, each with its area, holes and the texts inside it, and the pairs of
//! lines that run side by side at a distance. Hatch pattern lines and dimensions are left
//! out: they draw on areas, they do not bound them.

use super::*;
use ketchup_model::underlay::object_query::ObjectIndex;
use ketchup_model::underlay::objects::ObjectDetail;
use ketchup_model::underlay::regions::{
    DEFAULT_JOIN_MM, MAX_JOIN_MM, Region, find_figures, find_regions, parallel_pairs,
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
            closing_segments_mm,
            include_outline,
            whole,
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
        let mut segments = pieces.iter().map(|piece| piece.ends).collect::<Vec<_>>();
        add_closures(&mut segments, underlay, closing_segments_mm)?;
        let scale = underlay.scale();
        // The tolerance is document millimetres; the lines are the underlay's.
        let mut regions = if *whole {
            find_figures(&segments, tolerance / scale)
        } else {
            find_regions(&segments, tolerance / scale)
        }
        .map_err(search_too_large)?;
        let area_scale = scale * scale;
        regions.retain(|region| region.area_mm2 * area_scale / 1e6 >= min_area_m2);
        let (texts, ambiguous_texts) =
            texts_in(&lines, &chosen_text_layers(&snapshot, underlay), &regions);
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
        let mut answer = json!({
            "underlay_id": underlay.id.0,
            "layers": chosen.iter().map(|index| &underlay.layers[*index].name).collect::<Vec<_>>(),
            "pieces": segments.len(),
            "tolerance_mm": tolerance,
            "closing_segments_mm": closing_segments_mm,
            "matched": order.len(),
            "regions": rows,
            "ambiguous_texts": ambiguous_texts,
            "interpretation": "Geometric regions, not classified rooms. Text association uses rendered letter centres; verify boundaries and layers before modeling.",
            "next_offset": (next < order.len()).then_some(next),
        });
        if let Some(warning) = objectless_warning(&lines) {
            answer["warnings"] = json!([warning]);
        }
        Ok(answer)
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

/// The innermost of `regions` of `underlay` ([`closed_regions`]) that holds the document
/// point `point`, in the underlay's millimetres.
pub(super) fn region_at(
    underlay: &Underlay,
    regions: Vec<Region>,
    point: [f64; 2],
) -> Result<Region, &'static str> {
    if !point.iter().all(|value| value.is_finite()) {
        return Err("invalid_params");
    }
    region_holding(underlay, regions, point).ok_or_else(|| {
        super::program_access::invalid(
            "point_mm",
            "No closed region of the chosen layers holds the point.",
            "Ask inspect action=underlay_regions with point_mm, or name the layers whose lines close around it.",
        )
    })
}

/// The innermost of `regions` of `underlay` that holds the document point `point`, if one
/// does: no region there is no refusal.
pub(super) fn region_holding(
    underlay: &Underlay,
    regions: Vec<Region>,
    point: [f64; 2],
) -> Option<Region> {
    let local = underlay.to_local(point);
    regions
        .into_iter()
        .filter(|region| region.contains(local))
        .min_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2))
}

/// Review 2026-10-26 UO-41: the room that `found`, a region of [`closed_regions`] of the
/// shown layers of `underlay`, stands for when no layers are named, and the layers of its
/// walls (none when no drawn line bounds it). A stamp frame or a fitting drawn
/// on layers of their own inside a room is no hole in it, and a point or a label inside one
/// of them is in the room: the room is holed only where the layers of its walls close
/// inside it.
pub(super) fn walled(
    snapshot: &Snapshot,
    underlay: &Underlay,
    lines: &UnderlayLines,
    lookup: Lookup,
    found: Region,
) -> Result<(Region, Vec<usize>), &'static str> {
    let mut regions = closed_regions(snapshot, underlay, lines, None, lookup, false)?;
    let Some(mut at) = regions.iter().position(|region| *region == found) else {
        return Ok((found, Vec::new()));
    };
    let pieces = pieces(
        lines,
        &chosen_text_layers(snapshot, underlay),
        lookup.within,
    );
    let tolerance = lookup.tolerance_mm.unwrap_or(DEFAULT_JOIN_MM) / underlay.scale();
    let edge = |ring: &[[f64; 2]]| ring_layers(ring, &pieces, tolerance);
    let mut layers = edge(&regions[at].outline);
    for _ in 0..regions.len() {
        let outline = &regions[at].outline;
        let Some(around) = regions.iter().position(|region| {
            region
                .holes
                .iter()
                .any(|hole| same_ring(hole, outline, tolerance))
        }) else {
            break;
        };
        let walls = edge(&regions[around].outline);
        if walls.is_empty() || walls.iter().any(|layer| layers.contains(layer)) {
            break;
        }
        at = around;
        layers = walls;
    }
    let mut room = regions.swap_remove(at);
    if !layers.is_empty() {
        room.holes
            .retain(|hole| edge(hole).iter().any(|layer| layers.contains(layer)));
        room.area_mm2 = room
            .holes
            .iter()
            .fold(ring_area(&room.outline), |area, hole| {
                area - ring_area(hole)
            });
    }
    Ok((room, layers))
}

/// The layers of the room ([`walled`]) at the document point `point` or labelled `label`
/// among the shown layers, when there is one.
pub(super) fn room_layers(
    snapshot: &Snapshot,
    underlay: &Underlay,
    lines: &UnderlayLines,
    point: Option<[f64; 2]>,
    label: Option<&str>,
    lookup: Lookup,
) -> Option<Vec<usize>> {
    let found = match (point, label) {
        (Some(point), None) if point.iter().all(|value| value.is_finite()) => {
            let regions = closed_regions(snapshot, underlay, lines, None, lookup, false).ok()?;
            region_holding(underlay, regions, point)?
        }
        (None, Some(label)) => {
            region_labelled(snapshot, underlay, lines, None, None, label, lookup).ok()?
        }
        _ => return None,
    };
    let (_, layers) = walled(snapshot, underlay, lines, lookup, found).ok()?;
    (!layers.is_empty()).then_some(layers)
}

/// The layers of `pieces` that run along an edge of `ring`, within `tolerance`.
fn ring_layers(ring: &[[f64; 2]], pieces: &[Piece], tolerance: f64) -> Vec<usize> {
    let mut layers = Vec::new();
    for (index, start) in ring.iter().enumerate() {
        let end = ring[(index + 1) % ring.len()];
        let along = [end[0] - start[0], end[1] - start[1]];
        let length = along[0].hypot(along[1]);
        if length <= tolerance {
            continue;
        }
        let unit = [along[0] / length, along[1] / length];
        let low = [
            start[0].min(end[0]) - tolerance,
            start[1].min(end[1]) - tolerance,
        ];
        let high = [
            start[0].max(end[0]) + tolerance,
            start[1].max(end[1]) + tolerance,
        ];
        for piece in pieces {
            let [a, b] = piece.ends;
            if layers.contains(&piece.layer)
                || a[0].max(b[0]) < low[0]
                || a[0].min(b[0]) > high[0]
                || a[1].max(b[1]) < low[1]
                || a[1].min(b[1]) > high[1]
            {
                continue;
            }
            let offset = |p: [f64; 2]| [p[0] - start[0], p[1] - start[1]];
            let across = |p: [f64; 2]| (offset(p)[0] * unit[1] - offset(p)[1] * unit[0]).abs();
            let at = |p: [f64; 2]| offset(p)[0] * unit[0] + offset(p)[1] * unit[1];
            let (from, to) = (at(a).min(at(b)), at(a).max(at(b)));
            if across(a) <= tolerance
                && across(b) <= tolerance
                && to.min(length) - from.max(0.0) > tolerance
            {
                layers.push(piece.layer);
            }
        }
    }
    layers
}

/// Whether the rings `a` and `b` close round the same place: the same bounds and area,
/// within `tolerance`.
fn same_ring(a: &[[f64; 2]], b: &[[f64; 2]], tolerance: f64) -> bool {
    let bounds = |ring: &[[f64; 2]]| {
        ring.iter().fold(
            [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
            |[low, high], point| {
                [
                    [low[0].min(point[0]), low[1].min(point[1])],
                    [high[0].max(point[0]), high[1].max(point[1])],
                ]
            },
        )
    };
    let ([a_low, a_high], [b_low, b_high]) = (bounds(a), bounds(b));
    let close = |x: f64, y: f64| (x - y).abs() <= tolerance;
    let span = (a_high[0] - a_low[0]) + (a_high[1] - a_low[1]);
    (0..2).all(|axis| close(a_low[axis], b_low[axis]) && close(a_high[axis], b_high[axis]))
        && (ring_area(a) - ring_area(b)).abs() <= 2.0 * tolerance * span
}

/// The area a ring closes round, whichever way it turns.
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let twice = (0..ring.len())
        .map(|index| {
            let [x0, y0] = ring[index];
            let [x1, y1] = ring[(index + 1) % ring.len()];
            x0 * y1 - x1 * y0
        })
        .sum::<f64>();
    twice.abs() / 2.0
}

/// The one region of the shown (or named) layers whose texts (on the shown layers)
/// include `label`: a text equal to it, ignoring case and line breaks, or failing that
/// one containing it. Several regions or none are an error naming what was found.
pub(super) fn region_labelled(
    snapshot: &Snapshot,
    underlay: &Underlay,
    lines: &UnderlayLines,
    layers: Option<&[String]>,
    text_layers: Option<&[String]>,
    label: &str,
    lookup: Lookup,
) -> Result<Region, &'static str> {
    let wanted = folded(label);
    if wanted.is_empty() {
        return Err(super::program_access::invalid(
            "label",
            "The label is empty.",
            "Give the text written inside the region, e.g. \"1.02 WC\".",
        ));
    }
    let mut regions = closed_regions(snapshot, underlay, lines, layers, lookup, false)?;
    // Review 2026-10-26 UO-44: the layers a binding read its label from, shown or not.
    let text_layers = match text_layers {
        Some(names) => underlay
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| {
                names
                    .iter()
                    .any(|name| layer.name.eq_ignore_ascii_case(name))
            })
            .map(|(index, _)| index)
            .collect(),
        None => chosen_text_layers(snapshot, underlay),
    };
    let (texts, _) = texts_in(lines, &text_layers, &regions);
    let holding = |matches: &dyn Fn(&str) -> bool| {
        (0..regions.len())
            .filter(|index| texts[*index].iter().any(|text| matches(&folded(text))))
            .collect::<Vec<_>>()
    };
    let mut found = holding(&|text| text == wanted);
    if found.is_empty() {
        found = holding(&|text| text.contains(&wanted));
    }
    match found.as_slice() {
        [index] => Ok(regions.swap_remove(*index)),
        [] => Err(super::program_access::invalid(
            "label",
            format!("No closed region of the chosen layers holds a text {label:?} entirely."),
            "Check the text with inspect action=underlay_objects text=…; if the room is open at a doorway, pass closing_segments_mm across it; or name the layers whose lines close around it.",
        )),
        several => {
            let scale = underlay.scale();
            let candidates = several
                .iter()
                .take(ROW_TEXTS)
                .map(|index| {
                    format!(
                        "{:.2} m² holding {:?}",
                        regions[*index].area_mm2 * scale * scale / 1e6,
                        texts[*index]
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            Err(super::program_access::invalid(
                "label",
                format!(
                    "{} regions hold a text {label:?}: {candidates}.",
                    several.len()
                ),
                "Give the whole text of one label, or point_mm inside the intended region.",
            ))
        }
    }
}

/// Review 2026-10-26 UO-42: what to say of the regions of `lines` imported before Kečup
/// kept the objects of a drawing, when hatch and dimension lines cannot be left out.
pub(super) fn objectless_warning(lines: &UnderlayLines) -> Option<&'static str> {
    let drawn = lines
        .layers()
        .iter()
        .any(|layer| layer.polylines().next().is_some());
    (drawn && lines.objects().is_empty()).then_some(
        "This underlay was imported before Kečup kept the objects of a drawing, so hatch and dimension lines are not left out and may split its regions; import it again.",
    )
}

/// Lower case, every run of white space (line breaks included) one space.
fn folded(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The closed regions of the shown (or named) layers of `underlay`, in its millimetres;
/// `whole`, its figures instead: each outer edge whatever lines split it inside
/// ([`find_figures`]).
pub(super) fn closed_regions(
    snapshot: &Snapshot,
    underlay: &Underlay,
    lines: &UnderlayLines,
    layers: Option<&[String]>,
    lookup: Lookup,
    whole: bool,
) -> Result<Vec<Region>, &'static str> {
    found_regions(region_search(
        snapshot, underlay, lines, layers, lookup, whole,
    )?)
}

/// The search [`closed_regions`] makes, without making it.
pub(super) fn region_search(
    snapshot: &Snapshot,
    underlay: &Underlay,
    lines: &UnderlayLines,
    layers: Option<&[String]>,
    lookup: Lookup,
    whole: bool,
) -> Result<Search, &'static str> {
    let tolerance = lookup.tolerance_mm.unwrap_or(DEFAULT_JOIN_MM);
    if !(tolerance > 0.0 && tolerance <= MAX_JOIN_MM) {
        return Err("invalid_params");
    }
    let chosen = chosen_layers(snapshot, underlay, layers)?;
    let pieces = pieces(lines, &chosen, lookup.within);
    let mut segments = pieces.iter().map(|piece| piece.ends).collect::<Vec<_>>();
    add_closures(&mut segments, underlay, lookup.closing_segments_mm)?;
    Ok(Search {
        segments,
        tolerance: (tolerance / underlay.scale()).to_bits(),
        whole,
    })
}

/// How a region is looked for besides its layers.
#[derive(Clone, Copy, Default)]
pub(super) struct Lookup<'a> {
    /// The distance, in document millimetres, within which line ends join.
    pub tolerance_mm: Option<f64>,
    /// Segments, in document millimetres, that close gaps such as doorways.
    pub closing_segments_mm: &'a [[[f64; 2]; 2]],
    /// The box, in the underlay's millimetres, the lines looked at meet: a part of a plan too
    /// large to search whole (review 2026-10-26 UO-46).
    pub within: Option<[[f64; 2]; 2]>,
}

/// The refusal of a search of more lines than one takes: what to narrow it with (review
/// 2026-10-26 UO-46).
fn search_too_large(error: ketchup_model::underlay::regions::RegionError) -> &'static str {
    record_rejection(
        &rejected("invalid_params")
            .target("within_mm")
            .reason(format!("Too many lines to search for regions: {error}."))
            .fix_hint("Pass within_mm, a document box [[x, y], [x, y]] around the area, or name only the layers whose lines bound it.")
            .caused_by(error),
        json!({}),
    );
    "invalid_params"
}

/// The regions, or the figures, `search` finds, the same search found once.
fn found_regions(search: Search) -> Result<Vec<Region>, &'static str> {
    let mut found = FOUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(at) = found.iter().position(|(earlier, _)| *earlier == search) {
        let entry = found.remove(at);
        let regions = entry.1.clone();
        found.push(entry);
        return regions.map_err(search_too_large);
    }
    drop(found);
    #[cfg(test)]
    SEARCHED_HERE.with(|searched| searched.set(searched.get() + 1));
    let tolerance = f64::from_bits(search.tolerance);
    let regions = if search.whole {
        find_figures(&search.segments, tolerance)
    } else {
        find_regions(&search.segments, tolerance)
    };
    let mut found = FOUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let kept = FOUND_KEPT
        + FLAT_DRAWINGS.load(std::sync::atomic::Ordering::Relaxed)
        + FOLLOWED_SEARCHES.load(std::sync::atomic::Ordering::Relaxed);
    while found.len() >= kept {
        drop(found.remove(0));
    }
    found.push((search, regions.clone()));
    drop(found);
    regions.map_err(search_too_large)
}

/// What regions were found in: the pieces in the underlay's millimetres, the join
/// tolerance there and whether figures were wanted.
#[derive(Clone, PartialEq)]
pub(super) struct Search {
    segments: Vec<[[f64; 2]; 2]>,
    tolerance: u64,
    whole: bool,
}

/// The regions a search found, or why it found none.
type Searched = Result<Vec<Region>, ketchup_model::underlay::regions::RegionError>;

/// How many searches [`FOUND`] keeps besides one per drawing lying flat.
const FOUND_KEPT: usize = 4;

/// The drawings lying flat whose regions a click of Pull area waits for at once: [`FOUND`]
/// keeps one search more for each, or with a plan per floor each search found pushed out
/// another and the click waited for ever (review 2026-10-26 UO-40).
static FLAT_DRAWINGS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Keeps the regions of `count` drawings lying flat found together.
pub(crate) fn keep_found_for(count: usize) {
    FLAT_DRAWINGS.store(count, std::sync::atomic::Ordering::Relaxed);
}

/// The areas of the window's program that follow a drawing: [`FOUND`] keeps one search
/// more for each, or with areas found in more ways than it kept each move of the drawing
/// found all their regions again (review 2026-10-29 UO-40).
static FOLLOWED_SEARCHES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Keeps the regions of `count` areas that follow a drawing found together.
pub(super) fn keep_found_for_followed(count: usize) {
    FOLLOWED_SEARCHES.store(count, std::sync::atomic::Ordering::Relaxed);
}

/// The regions of the latest searches, or why they were not found, the last used last:
/// moving, turning or fitting an underlay found the same regions of the same lines again
/// for every section that follows it (review 2026-10-26 UO-40).
static FOUND: std::sync::Mutex<Vec<(Search, Searched)>> = std::sync::Mutex::new(Vec::new());

/// The searches [`search_ahead`] runs now.
static SEARCHING: std::sync::Mutex<Vec<Search>> = std::sync::Mutex::new(Vec::new());

/// Starts finding the closed regions of the shown layers of `underlay` on a thread of its
/// own, the search a click of Pull area makes, so the click takes them found instead of
/// stalling the window on a large plan (review 2026-10-26 UO-40).
pub(crate) fn search_ahead(
    snapshot: &Snapshot,
    underlay: &Underlay,
    lines: std::sync::Arc<UnderlayLines>,
) {
    let Ok(chosen) = chosen_layers(snapshot, underlay, None) else {
        return;
    };
    let tolerance = (DEFAULT_JOIN_MM / underlay.scale()).to_bits();
    spawn_ahead(move || {
        if let Ahead::Claimed(search) = claim(shown_search(&lines, &chosen, tolerance)) {
            run_ahead(search);
        }
    });
}

/// Whether the regions a click of Pull area finds in `underlay` are found, or why not.
/// When neither, they are being found off the UI thread: this starts that search unless it
/// runs already, so the click waits for it instead of searching again (review 2026-10-26
/// UO-40).
pub(crate) fn regions_ready(
    snapshot: &Snapshot,
    underlay: &Underlay,
    lines: &UnderlayLines,
) -> bool {
    let Ok(chosen) = chosen_layers(snapshot, underlay, None) else {
        // The click says why.
        return true;
    };
    let tolerance = (DEFAULT_JOIN_MM / underlay.scale()).to_bits();
    search_ready(shown_search(lines, &chosen, tolerance))
}

/// Whether the regions `search` finds are found, or why not; when neither, they are being
/// found off the UI thread, started here unless they are already.
pub(super) fn search_ready(search: Search) -> bool {
    match claim(search) {
        Ahead::Found => true,
        Ahead::Running => false,
        Ahead::Claimed(search) => {
            spawn_ahead(move || run_ahead(search));
            false
        }
    }
}

/// Where a search stands.
enum Ahead {
    Found,
    Running,
    /// Neither, now recorded in [`SEARCHING`] for the caller to run.
    Claimed(Search),
}

fn claim(search: Search) -> Ahead {
    let mut found = FOUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut searching = SEARCHING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(at) = found.iter().position(|(earlier, _)| *earlier == search) {
        // Used last: a waiting click keeps it over searches made since.
        let entry = found.remove(at);
        found.push(entry);
        return Ahead::Found;
    }
    if searching.contains(&search) {
        return Ahead::Running;
    }
    searching.push(Search {
        segments: search.segments.clone(),
        ..search
    });
    Ahead::Claimed(search)
}

/// Finds the regions of `search`, claimed, into [`FOUND`].
fn run_ahead(search: Search) {
    let _ = found_regions(search.clone());
    SEARCHING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|running| *running != search);
}

/// A test's hold on the searches its thread starts ahead: each waits until it is let go.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct HoldAhead {
    held: std::sync::Mutex<bool>,
    let_go: std::sync::Condvar,
}

#[cfg(test)]
impl HoldAhead {
    /// Holds the searches the calling thread starts ahead from now on.
    pub(crate) fn hold() -> std::sync::Arc<Self> {
        let hold = std::sync::Arc::new(Self {
            held: std::sync::Mutex::new(true),
            let_go: std::sync::Condvar::new(),
        });
        HOLD_AHEAD.with(|held| *held.borrow_mut() = Some(hold.clone()));
        hold
    }

    pub(crate) fn let_go(&self) {
        *self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        self.let_go.notify_all();
    }

    fn wait(&self) {
        let held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(
            self.let_go
                .wait_while(held, |held| *held)
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }
}

#[cfg(test)]
thread_local! {
    static HOLD_AHEAD: std::cell::RefCell<Option<std::sync::Arc<HoldAhead>>> =
        const { std::cell::RefCell::new(None) };
    static SEARCHED_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many searches for regions the calling thread has run itself.
#[cfg(test)]
pub(crate) fn searches_on_this_thread() -> usize {
    SEARCHED_HERE.with(std::cell::Cell::get)
}

/// Runs `search` on a thread of its own.
fn spawn_ahead(search: impl FnOnce() + Send + 'static) {
    #[cfg(test)]
    let hold = HOLD_AHEAD.with(|hold| hold.borrow().clone());
    std::thread::spawn(move || {
        #[cfg(test)]
        if let Some(hold) = hold {
            hold.wait();
        }
        search();
    });
}

/// The search for the regions of `layers` of `lines` joined within `tolerance`, as
/// [`closed_regions`] makes it with no closures.
fn shown_search(lines: &UnderlayLines, layers: &[usize], tolerance: u64) -> Search {
    Search {
        segments: pieces(lines, layers, None)
            .iter()
            .map(|piece| piece.ends)
            .collect(),
        tolerance,
        whole: false,
    }
}

/// Whether the regions of the shown layers of `underlay` are found, without finding them.
#[cfg(test)]
pub(crate) fn found_ahead(snapshot: &Snapshot, underlay: &Underlay, lines: &UnderlayLines) -> bool {
    let Ok(chosen) = chosen_layers(snapshot, underlay, None) else {
        return false;
    };
    let search = shown_search(
        lines,
        &chosen,
        (DEFAULT_JOIN_MM / underlay.scale()).to_bits(),
    );
    FOUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|(earlier, _)| *earlier == search)
}

fn add_closures(
    segments: &mut Vec<[[f64; 2]; 2]>,
    underlay: &Underlay,
    closures: &[[[f64; 2]; 2]],
) -> Result<(), &'static str> {
    if closures.len() > MAX_LIMIT
        || closures
            .iter()
            .flatten()
            .flatten()
            .any(|value| !value.is_finite())
    {
        return Err(super::program_access::invalid(
            "closing_segments_mm",
            "Closing segments must contain finite document coordinates, at most 100 segments.",
            "Supply only the explicit boundary gaps to close, as pairs of [x, y] endpoints.",
        ));
    }
    segments.extend(
        closures
            .iter()
            .map(|ends| ends.map(|point| underlay.to_local(point))),
    );
    Ok(())
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
pub(super) fn chosen_layers(
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

/// The names of the layers shown, whose texts are what a region says.
pub(super) fn text_layer_names(snapshot: &Snapshot, underlay: &Underlay) -> Vec<String> {
    chosen_text_layers(snapshot, underlay)
        .into_iter()
        .map(|index| underlay.layers[index].name.clone())
        .collect()
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

pub(super) fn local_area(underlay: &Underlay, area: [[f64; 2]; 2]) -> [[f64; 2]; 2] {
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

/// Associate only when every visible letter centre lies in the same region.
/// A text crossing boundaries is evidence for several candidates, not a region name.
fn texts_in(
    lines: &UnderlayLines,
    layers: &[usize],
    regions: &[Region],
) -> (Vec<Vec<String>>, Vec<Value>) {
    let mut texts = vec![Vec::new(); regions.len()];
    let mut ambiguous = Vec::new();
    for &layer in layers {
        let Some(drawn) = lines.layers().get(layer) else {
            continue;
        };
        for text in drawn.texts() {
            let centres = crate::renderer::underlay::text::letter_centres(text);
            let mut candidates = std::collections::BTreeSet::new();
            let mut outside = false;
            for point in centres {
                let innermost = regions
                    .iter()
                    .enumerate()
                    .filter(|(_, region)| region.contains(point))
                    .min_by(|a, b| a.1.area_mm2.total_cmp(&b.1.area_mm2))
                    .map(|(index, _)| index);
                if let Some(index) = innermost {
                    candidates.insert(index);
                } else {
                    outside = true;
                }
            }
            if candidates.len() == 1 && !outside {
                if let Some(index) = candidates.first() {
                    texts[*index].push(text.text.clone());
                }
            } else if !candidates.is_empty() {
                ambiguous.push(json!({"text": text.text, "candidate_regions": candidates, "partly_outside": outside}));
            }
        }
    }
    (texts, ambiguous)
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
