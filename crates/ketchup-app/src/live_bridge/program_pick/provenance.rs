//! Read-only provenance from the existing program model, not a second ownership store.
use super::*;
use ketchup_geometry::linalg::dot;
use ketchup_program::model::{Hole, ProgramModel};

const MATCH_MM: f64 = 0.001;

fn drilling(part: &Part, hole: &Hole, entry: [f64; 3], inward: [f64; 3]) -> Value {
    json!({
        "part": part.name,
        "id": hole.id,
        "representation": "machining",
        "entry_face": hole.face,
        "entry_local_mm": round(entry),
        "entry_world_mm": round(part.to_world(entry)),
        "inward_local": round(inward),
        "inward_world": round(frame::apply(&part.rotation, inward)),
        "diameter_mm": hole.diameter_mm,
        "depth_mm": hole.depth_mm,
        "through": hole.through,
    })
}

/// Ownership comes only from explicit operation references. Surface matching
/// identifies a bore, but coincident fasteners or opposing bores confer no ownership.
fn connections(model: &ProgramModel, part: &Part, hole: &Hole) -> Vec<Value> {
    model.joints.iter().flat_map(|joint| {
        joint.links.iter().enumerate().filter_map(move |(index, link)| {
            if !link.operations.iter().any(|operation| operation.part == part.name && operation.id == hole.id) {
                return None;
            }
            let counterparts = link.operations.iter()
                .filter(|operation| operation.part != part.name || operation.id != hole.id)
                .filter_map(|operation| {
                    let other = model.part(&operation.part)?;
                    let (hole, (entry, inward)) = other.finished_holes().find(|(hole, _)| hole.id == operation.id)?;
                    Some(drilling(other, hole, entry, inward))
                }).collect::<Vec<_>>();
            Some(json!({
                "name": joint.name,
                "kind": joint.kind,
                "association": "explicit_operation_link",
                "link_index": index,
                "operations": link.operations,
                "counterpart_part": joint.parts.iter().find(|name| **name != part.name),
                "counterpart_holes": counterparts,
                "hardware": {
                    "item": joint.fastener,
                    "representation": if link.hardware_parts.is_empty() { "metadata_only" } else { "linked_solid_parts" },
                    "solid_parts": link.hardware_parts,
                    "solid_part": if link.hardware_parts.len() == 1 { link.hardware_parts.first() } else { None },
                    "solid_link": if link.hardware_parts.is_empty() { "not_declared" } else { "explicit" },
                },
            }))
        })
    }).collect()
}

/// Match a complete tessellated face, not a nearby pick point. Cylinder wall
/// vertices lie on the bore radius; floor vertices lie in its end disk. Normal
/// tests exclude the entry face and exterior cylindrical faces. Multiple matches
/// remain visible instead of silently choosing an owner.
pub(super) fn describe(
    model: &ProgramModel,
    part: &Part,
    package: &ExactBodyPackage,
    ordinal: u32,
) -> Vec<Value> {
    let Some(reference) = package.topological_reference(TopologicalElementKind::Face, ordinal)
    else {
        return Vec::new();
    };
    let Some((point, normal)) = face_sample(package, ordinal) else {
        return Vec::new();
    };
    let vertices = package
        .triangles()
        .iter()
        .enumerate()
        .filter(|(index, _)| package.topological_reference_for_triangle(*index) == Some(reference))
        .flat_map(|(_, triangle)| {
            triangle
                .vertex_indices
                .map(|index| package.vertices()[index as usize].position_mm)
        })
        .collect::<Vec<_>>();
    part.finished_holes()
        .filter_map(|(hole, (entry, inward))| {
            let coordinates = |p: [f64; 3]| {
                let delta = std::array::from_fn(|i| p[i] - entry[i]);
                let depth = dot(delta, inward);
                let radial: [f64; 3] = std::array::from_fn(|i| delta[i] - depth * inward[i]);
                (depth, radial)
            };
            let (_, radial) = coordinates(point);
            let axial = dot(normal, inward);
            let wall = axial.abs() < APPROXIMATION && dot(normal, radial) < 0.0;
            let floor = axial < -1.0 + APPROXIMATION;
            let matches = !vertices.is_empty()
                && (wall || floor)
                && vertices.iter().all(|vertex| {
                    let (depth, radial) = coordinates(*vertex);
                    let radius = dot(radial, radial).sqrt();
                    if wall {
                        depth >= -MATCH_MM
                            && depth <= hole.depth_mm + MATCH_MM
                            && (radius - hole.diameter_mm * 0.5).abs() <= MATCH_MM
                    } else {
                        (depth - hole.depth_mm).abs() <= MATCH_MM
                            && radius <= hole.diameter_mm * 0.5 + MATCH_MM
                    }
                });
            matches.then(|| {
                let mut value = drilling(part, hole, entry, inward);
                value["association"] = json!("finished_hole_surface");
                value["surface"] = json!(if wall { "wall" } else { "floor" });
                let joints = connections(model, part, hole);
                value["ownership"] = json!(if joints.is_empty() {
                    "unowned"
                } else {
                    "explicit"
                });
                value["joints"] = json!(joints);
                value
            })
        })
        .collect()
}
