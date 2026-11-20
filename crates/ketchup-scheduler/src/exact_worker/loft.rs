//! A Loft node: a solid or an open surface through the sections, along an
//! optional guide and stretched to an optional rail; holes in solid sections
//! are lofted the same way and cut out.

use super::{
    exact_brep_planar_loop, exact_brep_planar_offset_loop, exact_brep_profile_error,
    exact_brep_spatial_path_segment,
};
use ketchup_exact::{
    ExactBackend, ExactBodyBooleanOperation, ExactOpOutput, FramedLoftProfile, FramedLoftSection,
    FramedLoftSpec, GeometryError, GeometryErrorCode, LoftSurfaceContinuity, backend_fingerprint,
};
use ketchup_model::exact_brep_graph::{
    ExactBRepGraph, ExactBRepLoftContinuity, ExactBRepLoftSection, ExactBRepOperation,
    ExactBRepPlanarGeometry, ExactBRepSpatialPath,
};

/// The solid or surface of a Loft node, through its sections along its guide and rail.
pub(super) fn exact_brep_loft_node(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    operation: &ExactBRepOperation,
) -> Result<ExactOpOutput, GeometryError> {
    let (sections, guide, rail, continuity, make_solid) = match operation {
        ExactBRepOperation::Loft {
            sections,
            guide,
            continuity,
            rail,
        } => (sections, guide, rail, *continuity, true),
        ExactBRepOperation::LoftSurface {
            sections,
            guide,
            continuity,
            rail,
        } => (sections, guide, rail, *continuity, false),
        _ => {
            return Err(GeometryError {
                code: GeometryErrorCode::InvalidParameter,
                diagnostic: "the node is not a Loft".to_owned(),
                operation: "loft_framed_profiles",
                input_digest: graph.canonical_input_digest.clone(),
                backend_fingerprint: backend_fingerprint(),
            });
        }
    };
    let paths = [guide.as_ref(), rail.as_ref()];
    exact_brep_loft(backend, graph, sections, paths, continuity, make_solid)
}

pub(super) const fn surface_continuity(
    continuity: ExactBRepLoftContinuity,
) -> LoftSurfaceContinuity {
    match continuity {
        ExactBRepLoftContinuity::Position => LoftSurfaceContinuity::Position,
        ExactBRepLoftContinuity::Tangent => LoftSurfaceContinuity::Tangent,
        ExactBRepLoftContinuity::Curvature => LoftSurfaceContinuity::Curvature,
    }
}

fn exact_brep_loft(
    backend: &ExactBackend,
    graph: &ExactBRepGraph,
    sections: &[ExactBRepLoftSection],
    [guide, rail]: [Option<&ExactBRepSpatialPath>; 2],
    continuity: ExactBRepLoftContinuity,
    make_solid: bool,
) -> Result<ExactOpOutput, GeometryError> {
    let segments = |path: &ExactBRepSpatialPath| {
        path.segments
            .iter()
            .map(exact_brep_spatial_path_segment)
            .collect::<Vec<_>>()
    };
    let guide_segments = guide.map(segments);
    let rail_segments = rail.map(segments);
    let continuity = surface_continuity(continuity);
    let mut outer_sections = Vec::with_capacity(sections.len());
    let mut hole_sections: Option<Vec<Vec<FramedLoftSection>>> = None;
    for section in sections {
        let profile = &graph.profiles[section.profile.0 as usize];
        let (outer, holes) = match &profile.geometry {
            ExactBRepPlanarGeometry::Spline { control_point_bits } => (
                FramedLoftProfile::Spline {
                    control_points_mm: control_point_bits
                        .iter()
                        .map(|point| point.map(f64::from_bits))
                        .collect(),
                },
                Vec::new(),
            ),
            ExactBRepPlanarGeometry::Circle { .. }
            | ExactBRepPlanarGeometry::Boundary { closed: true, .. } => (
                FramedLoftProfile::Planar(exact_brep_planar_offset_loop(profile)?),
                Vec::new(),
            ),
            ExactBRepPlanarGeometry::Region { outer, holes } => (
                FramedLoftProfile::Planar(exact_brep_planar_loop(outer)),
                holes
                    .iter()
                    .map(|hole| FramedLoftProfile::Planar(exact_brep_planar_loop(hole)))
                    .collect(),
            ),
            _ => {
                return Err(exact_brep_profile_error(
                    profile,
                    "exact Loft requires closed line/arc/circle/cubic or spline sections",
                ));
            }
        };
        let elevation_mm = f64::from_bits(section.elevation_bits);
        let frame = profile.frame();
        let hole_groups = hole_sections.get_or_insert_with(|| {
            (0..holes.len())
                .map(|_| Vec::with_capacity(sections.len()))
                .collect()
        });
        if hole_groups.len() != holes.len() {
            return Err(exact_brep_profile_error(
                profile,
                "exact Loft requires the same hole count in every section",
            ));
        }
        outer_sections.push(FramedLoftSection {
            elevation_mm,
            frame,
            profile: outer,
        });
        for (group, hole) in hole_groups.iter_mut().zip(holes) {
            group.push(FramedLoftSection {
                elevation_mm,
                frame,
                profile: hole,
            });
        }
    }
    let spec = FramedLoftSpec {
        sections: outer_sections,
    };
    let guide = guide_segments.as_deref();
    let mut output = match (guide, rail_segments.as_deref()) {
        (Some(guide), Some(rail)) => {
            backend.loft_framed_with_rail(&spec, guide, rail, continuity, make_solid)?
        }
        (None, Some(_)) => {
            return Err(GeometryError {
                code: GeometryErrorCode::InvalidParameter,
                diagnostic: "a Loft rail needs a guide path".to_owned(),
                operation: "loft_framed_profiles",
                input_digest: graph.canonical_input_digest.clone(),
                backend_fingerprint: backend_fingerprint(),
            });
        }
        (guide, None) if make_solid => {
            backend.loft_framed_profiles_with_controls(&spec, guide, continuity)?
        }
        (guide, None) => backend.loft_framed_surface(&spec, guide, continuity)?,
    };
    // Holes are known from the first section on; every section has as many.
    let hole_sections: Vec<_> = hole_sections.into_iter().flatten().collect();
    if (!make_solid || rail.is_some()) && !hole_sections.is_empty() {
        return Err(GeometryError {
            code: GeometryErrorCode::InvalidProfile,
            diagnostic: "Loft holes are not supported on a surface or with a rail".to_owned(),
            operation: "loft_framed_surface",
            input_digest: graph.canonical_input_digest.clone(),
            backend_fingerprint: backend_fingerprint(),
        });
    }
    for sections in hole_sections {
        let tool = backend.loft_framed_profiles_with_controls(
            &FramedLoftSpec { sections },
            guide,
            continuity,
        )?;
        output =
            backend.boolean_bodies(&output.body, &tool.body, ExactBodyBooleanOperation::Cut)?;
    }
    Ok(output)
}
