//! Exact surface bodies: a planar face bounded by a profile and the smoothest
//! surface spanning a closed loop of spatial curves, also through the inner
//! curves of a network.

use super::{
    exact_brep_boundary_segments, exact_brep_graph_error, exact_brep_planar_loop,
    exact_brep_profile_error, exact_brep_spatial_path_segment, transform_local_to_profile_frame,
};
use ketchup_exact::PlanarProfileSegment;
use ketchup_exact::{ExactBackend, ExactOpOutput, GeometryError, PlanarProfileLoop};
use ketchup_model::exact_brep_graph::{
    ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry, ExactBRepProfileId,
    ExactBRepSpatialPath,
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
