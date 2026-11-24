//! The checks of a surface, fillet or chamfer request. Each refusal names the
//! field and the rule it breaks (R1120ac-EXA-902): before, a weight of zero, an
//! empty move, a seam kept twice, a radius under 0.01 mm and stations out of
//! order all answered "assistant CAD body feature is invalid".

use super::{
    AssistantCadChamferMode, AssistantCadFeatureReference, AssistantCadFilletRadiusStation,
    AssistantCadSurfaceBodySource, AssistantCadSurfaceSeam, assistant_cad_vector_is_bounded, loft,
};
use crate::request_invalid::{AssistantRequestInvalid, AssistantRequestProblem};
use ketchup_model::graph::is_sha256_hex;
use ketchup_model::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
use ketchup_tolerance::limits;
use std::collections::BTreeSet;
use std::ops::RangeInclusive;

pub(super) type Checked = Result<(), AssistantRequestInvalid>;

const SUBJECT: &str = "CAD body feature";
/// A fillet radius or chamfer distance, in mm.
const EDGE_SIZE_MM: RangeInclusive<f64> = limits::MIN_LENGTH_MM..=100_000.0;

/// Ok when `holds`, else `field` refused for breaking `rule`.
pub(super) fn rule(holds: bool, field: impl ToString, rule: &'static str) -> Checked {
    refuse(holds, field, AssistantRequestProblem::Violates(rule))
}

/// Ok when `count` is at most `limit`, else `field` refused for exceeding it.
pub(super) fn at_most(count: usize, limit: usize, field: &str) -> Checked {
    refuse(
        count <= limit,
        field,
        AssistantRequestProblem::ExceedsLimit(limit),
    )
}

fn refuse(holds: bool, field: impl ToString, problem: AssistantRequestProblem) -> Checked {
    if holds {
        Ok(())
    } else {
        Err(AssistantRequestInvalid::new(SUBJECT, problem).item(field))
    }
}

pub(super) fn reference(reference: AssistantCadFeatureReference, field: &str) -> Checked {
    rule(
        reference.validate().is_ok(),
        field,
        "it must name a feature: a nonzero ID or an earlier operation's output",
    )
}

fn length(value: f64, field: &str) -> Checked {
    rule(
        value.is_finite() && (limits::MIN_LENGTH_MM..=super::MAX_COORDINATE_MM).contains(&value),
        field,
        "it must be a finite length from 0.01 mm to the coordinate limit",
    )
}

pub(super) fn surface_body(source: &AssistantCadSurfaceBodySource) -> Checked {
    match source {
        AssistantCadSurfaceBodySource::Planar { profile_feature_id } => {
            reference(*profile_feature_id, "profile_feature_id")
        }
        AssistantCadSurfaceBodySource::Loft {
            sections,
            guide_feature_id,
            continuity,
            rail_feature_id,
        } => loft::check(sections, *guide_feature_id, *rail_feature_id, *continuity),
        AssistantCadSurfaceBodySource::Boundary { segments, inner } => {
            rule(
                segments.len() >= 2,
                "segments",
                "a closed loop needs at least 2 segments",
            )?;
            at_most(segments.len(), limits::PATH_SEGMENTS, "segments")?;
            at_most(inner.len(), limits::PATH_SEGMENTS, "inner")
        }
        AssistantCadSurfaceBodySource::Blend {
            first_feature_id,
            second_feature_id,
            ..
        } => {
            reference(*first_feature_id, "first_feature_id")?;
            reference(*second_feature_id, "second_feature_id")?;
            rule(
                first_feature_id != second_feature_id,
                "second_feature_id",
                "a blend spans two different surfaces",
            )
        }
        AssistantCadSurfaceBodySource::ControlPoint {
            target_feature_id,
            offset_mm,
            weight,
            keep,
            ..
        } => control_point(*target_feature_id, *offset_mm, *weight, keep),
    }
}

fn control_point(
    target: AssistantCadFeatureReference,
    offset_mm: [f64; 3],
    weight: Option<f64>,
    keep: &[AssistantCadSurfaceSeam],
) -> Checked {
    reference(target, "target_feature_id")?;
    rule(
        assistant_cad_vector_is_bounded(offset_mm),
        "offset_mm",
        "each value must be finite and within the coordinate limit",
    )?;
    rule(
        weight.is_none_or(|weight| {
            (1.0 / limits::NURBS_WEIGHT_SPAN..=limits::NURBS_WEIGHT_SPAN).contains(&weight)
        }),
        "weight",
        "a weight must lie between 0.000001 and 1000000; 1 leaves the point as plain as its \
         neighbours",
    )?;
    rule(
        weight.is_some() || offset_mm.iter().any(|value| *value != 0.0),
        "offset_mm",
        "the move changes nothing: give a nonzero offset_mm or a weight",
    )?;
    at_most(keep.len(), limits::FEATURE_REFERENCES, "keep")?;
    for (index, seam) in keep.iter().enumerate() {
        let field = format!("keep[{index}]");
        reference(seam.surface_feature_id, &field)?;
        rule(
            seam.surface_feature_id != target,
            &field,
            "the surface that moves cannot keep a seam with itself; name a neighbouring surface",
        )?;
        rule(
            keep[..index]
                .iter()
                .all(|other| other.surface_feature_id != seam.surface_feature_id),
            &field,
            "this surface is already kept; name each neighbour once",
        )?;
    }
    Ok(())
}

pub(super) fn surface_trim(
    target: AssistantCadFeatureReference,
    cutter: AssistantCadFeatureReference,
) -> Checked {
    reference(target, "target_feature_id")?;
    reference(cutter, "cutter_feature_id")?;
    rule(
        target != cutter,
        "cutter_feature_id",
        "a surface cannot trim itself",
    )
}

pub(super) fn surface_extend(target: AssistantCadFeatureReference, distance_mm: f64) -> Checked {
    reference(target, "target_feature_id")?;
    length(distance_mm, "distance_mm")
}

pub(super) fn surface_knit(
    surfaces: &[AssistantCadFeatureReference],
    tolerance_mm: f64,
) -> Checked {
    rule(
        surfaces.len() >= 2,
        "surface_feature_ids",
        "a knit joins at least 2 surfaces",
    )?;
    at_most(surfaces.len(), limits::KNIT_SURFACES, "surface_feature_ids")?;
    for (index, surface) in surfaces.iter().enumerate() {
        let field = format!("surface_feature_ids[{index}]");
        reference(*surface, &field)?;
        rule(
            !surfaces[..index].contains(surface),
            &field,
            "this surface is already named; name each once",
        )?;
    }
    rule(
        tolerance_mm.is_finite() && (DEFAULT_LINEAR_TOLERANCE_MM..=10.0).contains(&tolerance_mm),
        "tolerance_mm",
        "it must be from the linear tolerance to 10 mm",
    )
}

pub(super) fn surface_thicken(target: AssistantCadFeatureReference, thickness_mm: f64) -> Checked {
    reference(target, "target_feature_id")?;
    length(thickness_mm, "thickness_mm")
}

/// 1 to the most distinct edge (or face) references, each a SHA-256 hex.
fn edge_references(ids: &[String], field: &str) -> Checked {
    rule(!ids.is_empty(), field, "name at least one edge")?;
    at_most(ids.len(), limits::FEATURE_REFERENCES, field)?;
    rule(
        ids.iter().all(|id| is_sha256_hex(id)),
        field,
        "each must be a reference ID (64 hex digits) from inspect query kind=edges",
    )?;
    rule(
        ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
        field,
        "an edge is named twice; name each once",
    )
}

fn edge_size(value: f64, field: &str) -> Checked {
    rule(
        value.is_finite() && EDGE_SIZE_MM.contains(&value),
        field,
        "it must be from 0.01 mm to 100000 mm",
    )
}

pub(super) fn fillet(
    target_feature_id: u64,
    edge_reference_ids: &[String],
    radius_mm: f64,
    stations: &[AssistantCadFilletRadiusStation],
) -> Checked {
    rule(
        target_feature_id != 0,
        "target_feature_id",
        "it must be a nonzero feature ID",
    )?;
    edge_references(edge_reference_ids, "edge_reference_ids")?;
    edge_size(radius_mm, "radius_mm")?;
    at_most(
        stations.len(),
        limits::FILLET_RADIUS_STATIONS,
        "radius_stations",
    )?;
    for (index, station) in stations.iter().enumerate() {
        rule(
            station.position.is_finite() && station.position > 0.0 && station.position <= 1.0,
            format!("radius_stations[{index}].position"),
            "a station lies along the edge, after 0 and at most 1",
        )?;
        edge_size(
            station.radius_mm,
            &format!("radius_stations[{index}].radius_mm"),
        )?;
        rule(
            index == 0 || stations[index - 1].position < station.position,
            format!("radius_stations[{index}].position"),
            "stations must follow the edge: each position greater than the one before",
        )?;
    }
    rule(
        stations
            .last()
            .is_none_or(|station| station.position == 1.0),
        "radius_stations",
        "the last station must be at position 1, the end of the edge",
    )
}

pub(super) fn chamfer(
    target_feature_id: u64,
    edge_reference_ids: &[String],
    distance_mm: f64,
    mode: AssistantCadChamferMode,
    side_face_reference_ids: &[String],
) -> Checked {
    rule(
        target_feature_id != 0,
        "target_feature_id",
        "it must be a nonzero feature ID",
    )?;
    edge_references(edge_reference_ids, "edge_reference_ids")?;
    edge_size(distance_mm, "distance_mm")?;
    let sides = || {
        rule(
            side_face_reference_ids.len() == edge_reference_ids.len(),
            "side_face_reference_ids",
            "name one side face for each edge, in the same order",
        )?;
        rule(
            side_face_reference_ids.iter().all(|id| is_sha256_hex(id)),
            "side_face_reference_ids",
            "each must be a reference ID (64 hex digits) from inspect query kind=faces",
        )
    };
    match mode {
        AssistantCadChamferMode::Symmetric => rule(
            side_face_reference_ids.is_empty(),
            "side_face_reference_ids",
            "a symmetric chamfer takes no side faces",
        ),
        AssistantCadChamferMode::TwoDistance { second_distance_mm } => {
            edge_size(second_distance_mm, "second_distance_mm")?;
            sides()
        }
        AssistantCadChamferMode::DistanceAngle { angle_degrees } => {
            rule(
                angle_degrees.is_finite() && angle_degrees > 0.1 && angle_degrees < 89.9,
                "angle_degrees",
                "it must lie between 0.1 and 89.9 degrees",
            )?;
            sides()
        }
    }
}
