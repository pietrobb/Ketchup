//! `inspect action=underlay_snap`: the endpoint, midpoint and nearest point of the shown
//! underlay lines near a point, and the length of the line or arc nearest it, from the
//! same snap index the window's drawing tools and Measure use.

use super::*;
use ketchup_model::underlay::UnderlayId;
use ketchup_model::underlay::snap::{UnderlaySnap, UnderlaySnapKind};

/// How far from the point a snap is looked for without `radius_mm`.
const DEFAULT_RADIUS_MM: f64 = 10.0;
/// The farthest a snap is looked for: past it a client measures, it does not snap.
const MAX_RADIUS_MM: f64 = 10_000.0;

impl LiveBridge {
    pub(super) fn underlay_snap(
        app: &KetchupApp,
        request: &Request,
    ) -> Result<Value, &'static str> {
        let Request::UnderlaySnap {
            expected,
            point_mm,
            radius_mm,
            underlay_id,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        let radius_mm = radius_mm.unwrap_or(DEFAULT_RADIUS_MM);
        if !point_mm.iter().all(|value| value.is_finite())
            || !(0.0..=MAX_RADIUS_MM).contains(&radius_mm)
        {
            return Err("invalid_params");
        }
        let snapshot = app.document.current();
        let only = underlay_id.map(UnderlayId);
        if only.is_some_and(|id| snapshot.underlay(id).is_none()) {
            return Err("entity_not_found");
        }
        let snaps = app
            .underlay_snaps_near(&snapshot, *point_mm, radius_mm)
            .into_iter()
            .filter(|(id, _)| only.is_none_or(|only| only == *id))
            .collect::<Vec<_>>();
        let best = |kind| {
            snaps
                .iter()
                .filter(|(_, snap)| snap.kind == kind)
                .min_by(|a, b| a.1.distance_mm.total_cmp(&b.1.distance_mm))
                .copied()
        };
        let described = |found: Option<(UnderlayId, UnderlaySnap)>| {
            found.map_or(Value::Null, |(id, snap)| describe(&snapshot, id, &snap))
        };
        let nearest = best(UnderlaySnapKind::Nearest);
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
        Ok(json!({
            "point_mm": point_mm,
            "radius_mm": radius_mm,
            "endpoint": described(best(UnderlaySnapKind::Endpoint)),
            "midpoint": described(best(UnderlaySnapKind::Midpoint)),
            "nearest": described(nearest),
            "line": line,
        }))
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
