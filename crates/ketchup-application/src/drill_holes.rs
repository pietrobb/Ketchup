//! Which points of a planar part face take a `drill_holes` hole, judged on the
//! exact body the host evaluated: its face evidence and its tessellation.
//!
//! A hole is admitted when its whole circle lies on the named face and its wall
//! stays inside the part, so it always removes material and never notches or
//! splits the part. Everything else is refused by name.

use crate::diagnostics::{AssistantPlanningResult, assistant_planning_rejection};
use crate::topology::assistant_topology_references;
use ketchup_assistant::sidecar::AssistantPartHole;
use ketchup_geometry::linalg::{point_segment_distance2, signed_outline_distance};
use ketchup_model::document::{DefinitionId, FeatureId, Snapshot};
use ketchup_model::exact_product::{ExactBRepGraphPackage, ExactBodyPackage, ExactResultRegistry};
use ketchup_model::tolerance::ROUNDING;
use ketchup_model::topology::TopologicalElementKind;

const OPERATION: &str = "drill_holes";

type Point2 = [f64; 2];

/// One `drill_holes` operation: `diameter_mm` holes at `points_local_mm` of
/// one face of body `target`, `depth_mm` deep or `through`.
pub(crate) struct Drill<'a> {
    pub definition_id: DefinitionId,
    pub name: &'a str,
    pub target: FeatureId,
    pub face_reference_id: &'a str,
    pub points_local_mm: &'a [[f64; 3]],
    pub diameter_mm: f64,
    pub depth_mm: Option<f64>,
    pub through: bool,
}

/// The holes `drill` makes, entering at its points along the inward face
/// normal, and the body bounds they are planned against.
///
/// # Errors
/// The target or face is not current exact evidence, the face is curved,
/// oblique or inside the body's bounds, or a hole is off the face, would break
/// out of the part, or would remove nothing.
pub(crate) fn admit_drill_holes(
    snapshot: &Snapshot,
    topology: &ExactResultRegistry,
    drill: &Drill<'_>,
) -> AssistantPlanningResult<(Vec<AssistantPartHole>, [[f64; 3]; 2])> {
    let Drill {
        definition_id,
        target,
        face_reference_id,
        points_local_mm,
        diameter_mm,
        depth_mm,
        through,
        ..
    } = *drill;
    let target_name = format!("feature:{}", target.0);
    let reject = |code, invariant: String, hint| {
        assistant_planning_rejection(code, OPERATION, &target_name, invariant, hint)
    };
    let packages = topology.body_values(snapshot).unwrap_or_default();
    let package = packages
        .values()
        .find_map(|package| match package.as_ref() {
            ExactBodyPackage::Graph(graph)
                if graph.identity.definition_id == definition_id
                    && graph.identity.producer_feature_id == target =>
            {
                Some(graph)
            }
            _ => None,
        })
        .ok_or_else(|| {
            reject(
                "planning.cad_feature_input_unsupported",
                "The drilled target is not a current exact body of the definition.".to_owned(),
                "Target an evaluated body feature of the part and use its listed face references.",
            )
        })?;
    let issued = assistant_topology_references(snapshot, topology, TopologicalElementKind::Face)
        .into_iter()
        .filter(|reference| {
            reference.definition_id == definition_id
                && reference.producer_feature_id == target
                && reference.lineage_digest == face_reference_id
        })
        .count();
    let ordinal = package
        .topological_references
        .iter()
        .filter(|reference| reference.kind == TopologicalElementKind::Face)
        .position(|reference| reference.lineage_digest == face_reference_id)
        .and_then(|ordinal| u32::try_from(ordinal).ok());
    let evidence = ordinal.filter(|_| issued == 1).and_then(|ordinal| {
        package
            .face_evidence
            .iter()
            .find(|evidence| evidence.face_ordinal == ordinal)
    });
    let (Some(ordinal), Some(evidence)) = (ordinal, evidence) else {
        return Err(reject(
            "planning.cad_topology_reference_unavailable",
            "The drilled face reference is not a unique current host-issued face of the target."
                .to_owned(),
            "Refresh the document context and use only listed topology_face_references for this target.",
        ));
    };
    if evidence.surface_kind != "plane" {
        return Err(reject(
            "planning.cad_hole_face_not_planar",
            format!(
                "The face is a {} surface; holes are drilled into planar faces only.",
                evidence.surface_kind
            ),
            "Drill into a planar face of the part.",
        ));
    }
    let normal = evidence.unit_normal;
    let Some(axis) = (0..3).find(|&axis| {
        (normal[axis].abs() - 1.0).abs() <= ROUNDING
            && (0..3).all(|other| other == axis || normal[other].abs() <= ROUNDING)
    }) else {
        return Err(reject(
            "planning.cad_hole_face_oblique",
            "The face is oblique to the part's axes; holes are drilled square into faces along a part axis only.".to_owned(),
            "Drill into a face whose normal runs along a part axis.",
        ));
    };
    let mut inward = [0.0; 3];
    inward[axis] = -normal[axis].signum();
    let bounds = vertex_bounds(package);
    let plane = evidence.centroid_mm[axis];
    let tolerance = snapshot.tolerance().linear_mm();
    let outer = if inward[axis] > 0.0 {
        bounds[0][axis]
    } else {
        bounds[1][axis]
    };
    if (plane - outer).abs() > tolerance {
        return Err(reject(
            "planning.cad_hole_face_inner",
            "The face lies inside the part's bounds (a step, pocket floor or inner wall); holes are drilled into outer bounding faces only.".to_owned(),
            "Drill into a face on the outside of the part.",
        ));
    }
    let extent = bounds[1][axis] - bounds[0][axis];
    let body = Body {
        package,
        axis,
        plane,
        side: inward[axis],
        tolerance,
    };
    let face: Vec<[Point2; 3]> = body
        .triangles()
        .filter(|(face, _)| *face == Some(ordinal))
        .map(|(_, triangle)| triangle.map(|[_, u, v]| [u, v]))
        .collect();
    let outline = face.iter().flatten().fold(
        [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
        |[min, max], corner| {
            [
                [min[0].min(corner[0]), min[1].min(corner[1])],
                [max[0].max(corner[0]), max[1].max(corner[1])],
            ]
        },
    );
    let (u, v) = body.plane_axes();
    let radius = diameter_mm * 0.5;
    let reach = depth_mm.unwrap_or(extent) + tolerance;
    let mut holes = Vec::with_capacity(points_local_mm.len());
    for (index, point) in points_local_mm.iter().enumerate() {
        let id = (index + 1).to_string();
        let off_face = |invariant: &str| {
            reject(
                "planning.cad_hole_point_off_face",
                format!("Hole {id}: {invariant}"),
                "Put every hole point on the named face, its whole circle inside the face.",
            )
        };
        if (point[axis] - plane).abs() > tolerance {
            return Err(off_face(
                "the point is not in the plane of the face, so it is not on the face.",
            ));
        }
        let centre = [point[u], point[v]];
        let on_face = face
            .iter()
            .any(|triangle| polygon_distance(centre, triangle) <= tolerance);
        if on_face {
            // Through: crossing parallel faces (a cavity, the far side) is the
            // hole's way out; any wall within the circle notches or splits.
            let breaks_out = if through {
                body.hits(centre, radius, extent, true)
            } else {
                body.hits(centre, radius, reach, false)
            };
            if breaks_out {
                return Err(reject(
                    "planning.cad_hole_breaks_out",
                    format!(
                        "Hole {id}: its wall would leave the part (at the edge of the face, into a cavity or through the far side), which notches the part or splits it."
                    ),
                    "Keep every hole circle and wall inside the part, or drill through for a hole that leaves it at the far side.",
                ));
            }
        } else {
            let within = (0..2)
                .all(|index| (outline[0][index]..=outline[1][index]).contains(&centre[index]));
            if within && !body.hits(centre, radius, reach, false) {
                return Err(reject(
                    "planning.cad_hole_removes_nothing",
                    format!(
                        "Hole {id}: the part has no material under it (for example an existing hole), so it would remove nothing."
                    ),
                    "Put every hole point on material of the named face.",
                ));
            }
            return Err(off_face("the point is outside the face."));
        }
        let mut entry = *point;
        entry[axis] = plane;
        holes.push(AssistantPartHole {
            id,
            entry_local_mm: entry,
            inward_unit_local: inward,
            diameter_mm,
            depth_mm: depth_mm.unwrap_or(extent),
            through,
        });
    }
    Ok((holes, bounds))
}

/// The tessellated body in hole coordinates: depth `t` along the inward axis
/// from the face plane, `u`, `v` across it.
struct Body<'a> {
    package: &'a ExactBRepGraphPackage,
    axis: usize,
    plane: f64,
    side: f64,
    tolerance: f64,
}

impl Body<'_> {
    fn plane_axes(&self) -> (usize, usize) {
        ((self.axis + 1) % 3, (self.axis + 2) % 3)
    }

    /// Every triangle as `[t, u, v]` corners, with the face ordinal it belongs to.
    fn triangles(&self) -> impl Iterator<Item = (Option<u32>, [[f64; 3]; 3])> + '_ {
        let (u, v) = self.plane_axes();
        let vertices = &self.package.vertices;
        self.package
            .triangles
            .iter()
            .enumerate()
            .map(move |(index, triangle)| {
                let corners = triangle.vertex_indices.map(|vertex| {
                    let position = usize::try_from(vertex)
                        .ok()
                        .and_then(|vertex| vertices.get(vertex))
                        .map_or([f64::NAN; 3], |vertex| vertex.position_mm);
                    [
                        self.side * (position[self.axis] - self.plane),
                        position[u],
                        position[v],
                    ]
                });
                let face = self.package.triangle_face_ordinals.get(index).copied();
                (face, corners)
            })
    }

    /// Whether the body's boundary comes within `radius` of the hole axis at
    /// `centre` between just below the face and depth `reach`; faces parallel
    /// to the entry face are skipped when `walls_only`.
    fn hits(&self, centre: Point2, radius: f64, reach: f64, walls_only: bool) -> bool {
        let tolerance = self.tolerance;
        self.triangles().any(|(_, corners)| {
            if walls_only
                && corners
                    .iter()
                    .all(|corner| (corner[0] - corners[0][0]).abs() <= tolerance)
            {
                return false;
            }
            let clipped = clip(&clip(&corners, |t| t - tolerance), |t| reach - t);
            let projected: Vec<Point2> = clipped.iter().map(|[_, u, v]| [*u, *v]).collect();
            polygon_distance(centre, &projected) < radius + tolerance
        })
    }
}

fn vertex_bounds(package: &ExactBRepGraphPackage) -> [[f64; 3]; 2] {
    package.vertices.iter().fold(
        [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]],
        |[min, max], vertex| {
            let position = vertex.position_mm;
            [
                std::array::from_fn(|axis| min[axis].min(position[axis])),
                std::array::from_fn(|axis| max[axis].max(position[axis])),
            ]
        },
    )
}

/// The part of a convex polygon of `[t, u, v]` corners where `inside(t) >= 0`.
fn clip(polygon: &[[f64; 3]], inside: impl Fn(f64) -> f64) -> Vec<[f64; 3]> {
    let mut kept = Vec::with_capacity(polygon.len() + 1);
    for (index, &corner) in polygon.iter().enumerate() {
        let next = polygon[(index + 1) % polygon.len()];
        let (here, there) = (inside(corner[0]), inside(next[0]));
        if here >= 0.0 {
            kept.push(corner);
        }
        if (here >= 0.0) != (there >= 0.0) {
            let share = here / (here - there);
            kept.push(std::array::from_fn(|axis| {
                corner[axis] + (next[axis] - corner[axis]) * share
            }));
        }
    }
    kept
}

/// Distance from `point` to a polygon, zero inside; one or two corners are a
/// point or a segment, and a polygon seen edge-on has only its edges.
fn polygon_distance(point: Point2, polygon: &[Point2]) -> f64 {
    match polygon {
        [] => f64::INFINITY,
        [corner] => point_segment_distance2(point, *corner, *corner),
        _ => signed_outline_distance(polygon, point)
            .map_or(f64::INFINITY, |distance| distance.max(0.0)),
    }
}
