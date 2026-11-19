//! Exact surface bodies: a planar face bounded by a profile, the smoothest
//! surface spanning a closed loop of spatial curves, also through the inner
//! curves of a network, and surfaces made from earlier ones.

use super::loft::surface_continuity;
use super::{
    exact_brep_boundary_segments, exact_brep_graph_error, exact_brep_planar_loop,
    exact_brep_profile_error, exact_brep_spatial_path_segment, transform_local_to_profile_frame,
};
use ketchup_exact::PlanarProfileSegment;
use ketchup_exact::{
    ExactBackend, ExactOpOutput, GeometryError, PlanarProfileLoop, SurfaceControlPoint,
};
use ketchup_model::exact_brep_graph::{
    ExactBRepGraph, ExactBRepNodeId, ExactBRepOperation, ExactBRepPlanarGeometry,
    ExactBRepProfileId, ExactBRepSpatialPath,
};

pub(super) fn planar_face(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    profile: ExactBRepProfileId,
) -> Result<ExactOpOutput, GeometryError> {
    let profile = &graph.profiles[profile.0 as usize];
    let planar_loop = match &profile.geometry {
        ExactBRepPlanarGeometry::Boundary { .. } => {
            PlanarProfileLoop::Segments(exact_brep_boundary_segments(profile, true)?)
        }
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => {
            let center_mm = center_bits.map(f64::from_bits);
            let radius_mm = f64::from_bits(*radius_bits);
            let positive = [center_mm[0] + radius_mm, center_mm[1]];
            let negative = [center_mm[0] - radius_mm, center_mm[1]];
            PlanarProfileLoop::Segments(vec![
                PlanarProfileSegment::CircularArc {
                    start_mm: positive,
                    end_mm: negative,
                    center_mm,
                    clockwise: false,
                },
                PlanarProfileSegment::CircularArc {
                    start_mm: negative,
                    end_mm: positive,
                    center_mm,
                    clockwise: false,
                },
            ])
        }
        ExactBRepPlanarGeometry::Region { outer, holes } if holes.is_empty() => {
            exact_brep_planar_loop(outer)
        }
        ExactBRepPlanarGeometry::Region { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact planar surface does not yet support profile holes",
            ));
        }
        ExactBRepPlanarGeometry::Spline { .. } => {
            return Err(exact_brep_profile_error(
                profile,
                "exact planar surface requires a closed line/arc/circle/cubic profile",
            ));
        }
    };
    let local = backend.planar_surface_profile(&planar_loop)?;
    transform_local_to_profile_frame(backend, profile, &local)
}

/// The surface of a `BoundarySurface` node through its loop and, when it has
/// them, the inner curves of its network.
pub(super) fn fill_boundary(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    operation: &ExactBRepOperation,
) -> Result<ExactOpOutput, GeometryError> {
    let ExactBRepOperation::BoundarySurface { boundary, inner } = operation else {
        return Err(exact_brep_graph_error(
            graph,
            "the node is not a boundary surface",
        ));
    };
    let segments = |path: &ExactBRepSpatialPath| {
        path.segments
            .iter()
            .map(exact_brep_spatial_path_segment)
            .collect::<Vec<_>>()
    };
    let inner = match inner {
        Some(inner) => segments(inner),
        None => Vec::new(),
    };
    backend.fill_network_surface(&segments(boundary), &inner)
}

/// The surface of a `SurfaceTrim`, `SurfaceBlend`, `SurfaceControlPoint` or
/// `SurfaceExtend` node from the earlier surfaces it is made from.
pub(super) fn derived_surface(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    outputs: &[ExactOpOutput],
    operation: &ExactBRepOperation,
) -> Result<ExactOpOutput, GeometryError> {
    let body = |node: ExactBRepNodeId| &outputs[node.index()].body;
    match operation {
        ExactBRepOperation::SurfaceTrim { target, cutter } => {
            backend.trim_surface(body(*target), body(*cutter))
        }
        ExactBRepOperation::SurfaceExtend {
            target,
            distance_bits,
        } => backend.extend_planar_surface(body(*target), f64::from_bits(*distance_bits)),
        ExactBRepOperation::SurfaceControlPoint {
            target,
            u_index,
            v_index,
            from_bits,
            offset_bits,
            weight_bits,
            keep,
        } => {
            let keep: Vec<_> = keep
                .iter()
                .map(|seam| (body(seam.surface), surface_continuity(seam.continuity)))
                .collect();
            backend.move_surface_control_point(
                body(*target),
                SurfaceControlPoint {
                    u_index: *u_index,
                    v_index: *v_index,
                    from_mm: from_bits.map(f64::from_bits),
                    offset_mm: offset_bits.map(f64::from_bits),
                    weight: weight_bits.map(f64::from_bits),
                },
                &keep,
            )
        }
        ExactBRepOperation::SurfaceBlend {
            first,
            first_continuity,
            second,
            second_continuity,
        } => backend.blend_surfaces(
            body(*first),
            surface_continuity(*first_continuity),
            body(*second),
            surface_continuity(*second_continuity),
        ),
        _ => Err(exact_brep_graph_error(
            graph,
            "the node is not made from earlier surfaces",
        )),
    }
}
