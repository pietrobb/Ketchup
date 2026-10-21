//! `inspect action=underlay_snap`: the endpoint, intersection, midpoint and nearest point
//! of the shown underlay lines near a point, the length of the line or arc nearest it, and
//! the distance between the snaps at two points, from the same snap index the window's
//! drawing tools and Measure use.

use super::*;
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::snap::{UnderlaySnap, UnderlaySnapKind};

/// How far from the point a snap is looked for without `radius_mm`.
const DEFAULT_RADIUS_MM: f64 = 10.0;
/// The farthest a snap is looked for: past it a client measures, it does not snap.
const MAX_RADIUS_MM: f64 = 10_000.0;

/// The order a measured point prefers snaps in, as the window's snaps rank them.
const PREFERRED: [UnderlaySnapKind; 4] = [
    UnderlaySnapKind::Endpoint,
    UnderlaySnapKind::Intersection,
    UnderlaySnapKind::Midpoint,
    UnderlaySnapKind::Nearest,
];

type Found = Vec<(UnderlayId, UnderlaySnap)>;

impl LiveBridge {
    pub(super) fn underlay_snap(
        app: &KetchupApp,
        request: &Request,
    ) -> Result<Value, &'static str> {
        let Request::UnderlaySnap {
            expected,
            point_mm,
            to_mm,
            radius_mm,
            underlay_id,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        let radius_mm = radius_mm.unwrap_or(DEFAULT_RADIUS_MM);
        if !point_mm
            .iter()
            .chain(to_mm.iter().flatten())
            .all(|value| value.is_finite())
            || !(0.0..=MAX_RADIUS_MM).contains(&radius_mm)
        {
            return Err("invalid_params");
        }
        let snapshot = app.document.current();
        let only = underlay_id.map(UnderlayId);
        if only.is_some_and(|id| snapshot.underlay(id).is_none()) {
            return Err("entity_not_found");
        }
        let ready = std::cell::Cell::new(true);
        let near = |point: [f64; 2]| -> Found {
            let (found, all_ready) = app.underlay_snaps_near(&snapshot, point, radius_mm);
            ready.set(ready.get() && all_ready);
            found
                .into_iter()
                .filter(|(id, _)| only.is_none_or(|only| only == *id))
                .collect()
        };
        let snaps = near(*point_mm);
        let described = |found: Option<(UnderlayId, UnderlaySnap)>| {
            found.map_or(Value::Null, |(id, snap)| describe(&snapshot, id, &snap))
        };
        let nearest = best(&snaps, UnderlaySnapKind::Nearest);
        let line = nearest
            .and_then(|(id, snap)| Some((id, app.underlay_piece(id, snap.segment)?)))
            .map_or(Value::Null, |(id, piece)| {
                let underlay = snapshot.underlay(id).expect("the underlay snapped to");
                json!({
                    "underlay_id": id.0,
                    "layer": underlay.layers.get(piece.layer).map(|layer| &layer.name),
                    "length_mm": piece.length_mm,
                    "start_mm": piece.points_mm.first(),
                    "end_mm": piece.points_mm.last(),
                    "points": piece.points_mm.len(),
                })
            });
        let measure = to_mm.map_or(Value::Null, |to_mm| {
            let end = |point: [f64; 2], found: &Found| {
                let snap = preferred(found);
                let at = snap.map_or(point, |(_, snap)| snap.point_mm);
                (
                    at,
                    json!({"point_mm": at, "snap": snap.map(|(_, snap)| kind_name(snap.kind)),
                    "underlay_snap": described(snap)}),
                )
            };
            let (from, from_json) = end(*point_mm, &snaps);
            let (to, to_json) = end(to_mm, &near(to_mm));
            let delta = [to[0] - from[0], to[1] - from[1]];
            json!({
                "from": from_json,
                "to": to_json,
                "delta_mm": delta,
                "distance_mm": delta[0].hypot(delta[1]),
            })
        });
        Ok(json!({
            "point_mm": point_mm,
            "radius_mm": radius_mm,
            "endpoint": described(best(&snaps, UnderlaySnapKind::Endpoint)),
            "intersection": described(best(&snaps, UnderlaySnapKind::Intersection)),
            "midpoint": described(best(&snaps, UnderlaySnapKind::Midpoint)),
            "nearest": described(nearest),
            "line": line,
            "measure": measure,
            // False while a shown underlay's snap index is still being built: ask again.
            "snaps_ready": ready.get(),
        }))
    }
}

/// The nearest snap of `kind`.
fn best(snaps: &Found, kind: UnderlaySnapKind) -> Option<(UnderlayId, UnderlaySnap)> {
    snaps
        .iter()
        .filter(|(_, snap)| snap.kind == kind)
        .min_by(|a, b| a.1.distance_mm.total_cmp(&b.1.distance_mm))
        .copied()
}

/// The snap a measured point takes: the nearest of the most preferred kind found.
fn preferred(snaps: &Found) -> Option<(UnderlayId, UnderlaySnap)> {
    PREFERRED.into_iter().find_map(|kind| best(snaps, kind))
}

fn kind_name(kind: UnderlaySnapKind) -> &'static str {
    match kind {
        UnderlaySnapKind::Endpoint => "endpoint",
        UnderlaySnapKind::Intersection => "intersection",
        UnderlaySnapKind::Midpoint => "midpoint",
        UnderlaySnapKind::Nearest => "nearest",
    }
}

/// A snap with its underlay and layer, and its point in the drawing's own coordinates.
fn describe(snapshot: &Snapshot, id: UnderlayId, snap: &UnderlaySnap) -> Value {
    let underlay = snapshot.underlay(id).expect("the underlay snapped to");
    let offset = underlay.offset_mm();
    json!({
        "underlay_id": id.0,
        "layer": underlay.layers.get(snap.layer).map(|layer| &layer.name),
        "point_mm": snap.point_mm,
        "drawing_mm": [snap.point_mm[0] + offset[0], snap.point_mm[1] + offset[1]],
        "distance_mm": snap.distance_mm,
    })
}
