//! Exact curves a view can show only as their tessellated lines, named with
//! the reason, so an export keeps those lines instead of fitting a curve.

use super::{DrawingInstance, DrawingSheetId, OrthographicViewKind, normalized};
use crate::document::Transform;
use crate::exact_product::ExactBRepGraphEdgeEvidence;
use crate::tolerance::ROUNDING;
use ketchup_geometry::linalg::dot;

/// An exact curve the view has only as its tessellated lines, because its
/// projection is neither a circle nor a straight line (or the view is a
/// section). An export keeps those lines and names the curve; it never fits one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedPolylineCurve {
    pub stable_curve_id: String,
    pub curve_kind: String,
    pub reason: &'static str,
}

/// The curves of every instance of a view, ordered by id.
pub(super) fn project_polyline_curves(
    sheet_id: DrawingSheetId,
    kind: OrthographicViewKind,
    instances: &[DrawingInstance],
) -> Vec<ProjectedPolylineCurve> {
    let mut curves = instances
        .iter()
        .flat_map(|instance| {
            project_instance_polyline_curves(
                sheet_id,
                kind,
                instance.token.as_str(),
                instance.transform,
                instance.package.edge_evidence(),
            )
        })
        .collect::<Vec<_>>();
    curves.sort_by(|left, right| left.stable_curve_id.cmp(&right.stable_curve_id));
    curves
}

/// The exact curves of one instance a view can show only as tessellated
/// lines, each with the reason. A circle facing the view is a circle of the
/// view; a circle seen edge-on and a line project to straight lines.
fn project_instance_polyline_curves(
    sheet_id: DrawingSheetId,
    kind: OrthographicViewKind,
    instance_token: &str,
    transform: Transform,
    edges: &[ExactBRepGraphEdgeEvidence],
) -> Vec<ProjectedPolylineCurve> {
    let depth = kind.frame().direction();
    let section = kind.section_depth_mm().is_some();
    edges
        .iter()
        .filter_map(|edge| {
            let reason = match (edge.curve_kind.as_str(), edge.unit_axis_direction) {
                // A degenerated edge is a point: nothing is drawn as lines.
                ("line" | "degenerate", _) => return None,
                ("circle", Some(axis)) if edge.circle_radius_mm.is_some() => {
                    let along = dot(normalized(transform.transform_vector(axis))?, depth).abs();
                    if along <= ROUNDING {
                        return None;
                    }
                    if (along - 1.0).abs() > ROUNDING {
                        "circle_oblique_to_view"
                    } else if section {
                        "section_view"
                    } else {
                        return None;
                    }
                }
                ("circle", _) => "circle_without_evidence",
                _ => "curve_kind_not_exported",
            };
            Some(ProjectedPolylineCurve {
                stable_curve_id: format!(
                    "sheet-{}/view-{}/{}:curve:{}",
                    sheet_id.0,
                    kind.stable_name(),
                    instance_token,
                    edge.edge_ordinal
                ),
                curve_kind: edge.curve_kind.clone(),
                reason,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_curves_a_view_has_only_as_lines_are_named_with_the_reason() {
        let edge = |ordinal: u32, kind: &str, axis: Option<[f64; 3]>| ExactBRepGraphEdgeEvidence {
            edge_ordinal: ordinal,
            curve_kind: kind.to_owned(),
            length_mm: 10.0,
            centroid_mm: [0.0; 3],
            bounds_mm: [[0.0; 3], [1.0; 3]],
            closed: kind == "circle",
            circle_radius_mm: (kind == "circle").then_some(5.0),
            axis_origin_mm: axis.map(|_| [0.0; 3]),
            unit_axis_direction: axis,
            adjacent_face_ordinals: vec![1, 2],
        };
        let edges = [
            edge(1, "line", None),
            edge(2, "circle", Some([0.0, 0.0, 1.0])),
            edge(3, "circle", None),
            edge(4, "bspline", None),
            edge(5, "degenerate", None),
        ];
        let named = |kind: OrthographicViewKind| {
            project_instance_polyline_curves(
                DrawingSheetId(3),
                kind,
                "definition-1",
                Transform::identity(),
                &edges,
            )
            .into_iter()
            .map(|curve| (curve.stable_curve_id, curve.curve_kind, curve.reason))
            .collect::<Vec<_>>()
        };
        let unsupported = |view: &str| {
            [
                (
                    format!("sheet-3/view-{view}/definition-1:curve:3"),
                    "circle".to_owned(),
                    "circle_without_evidence",
                ),
                (
                    format!("sheet-3/view-{view}/definition-1:curve:4"),
                    "bspline".to_owned(),
                    "curve_kind_not_exported",
                ),
            ]
        };
        // The circle faces the top view (a circle there) and is edge-on in the
        // front view (a straight line there): it is never named in either.
        assert_eq!(named(OrthographicViewKind::Top), unsupported("top"));
        assert_eq!(named(OrthographicViewKind::Front), unsupported("front"));
        let iso = named(OrthographicViewKind::Isometric);
        assert_eq!(iso.len(), 3);
        assert_eq!(
            (iso[0].1.as_str(), iso[0].2),
            ("circle", "circle_oblique_to_view")
        );
        let top = OrthographicViewKind::Top.frame();
        let section = OrthographicViewKind::section(top.direction(), top.vertical(), 0.5).unwrap();
        let cut = named(section);
        assert_eq!((cut.len(), cut[0].2), (3, "section_view"));
    }
}
