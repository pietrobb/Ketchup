//! A face or edge picked on a program-owned part, told in the program's own
//! words: which part, which named face (or the two faces of an edge), and its
//! point and outward normal in the part's frame and in the world, so the AI
//! can use the pick directly in `on`, `hole`, `push_pull` or `fillet`.
use super::*;
use ketchup_model::document::InstancePath;
use ketchup_model::exact_product::{ExactBodyPackage, ExactFaceRole};
use ketchup_model::tolerance::{APPROXIMATION, ROUNDING};
use ketchup_model::topology::{TopologicalElementKind, TopologicalElementRef};
use ketchup_program::{
    frame,
    model::{Part, ProgramModel},
};
mod provenance;

const HINT: &str = "face is the program's name for it (box: x-..z+; profile part: start, end or a \
segment name) for hole/push_pull/cut/contact; on_face is what on(part, target, face=...) takes \
(an axis of the part's own frame, else a world direction); an edge's two faces go to \
fillet/chamfer edges=[[a, b]]";

pub(crate) fn part_name(snapshot: &Snapshot, path: &InstancePath) -> Option<String> {
    ketchup_application::rule_program_part_name(snapshot, path)
}
fn round(values: [f64; 3]) -> [f64; 3] {
    values.map(|value| (value * 1000.0).round() / 1000.0 + 0.0)
}

/// Point and outward unit normal of face `ordinal`, in the part's frame: the
/// centroid of its largest triangle and that triangle's normal.
fn face_sample(package: &ExactBodyPackage, ordinal: u32) -> Option<([f64; 3], [f64; 3])> {
    let reference = package.topological_reference(TopologicalElementKind::Face, ordinal)?;
    let vertex = |index: u32| package.vertices()[index as usize].position_mm;
    package
        .triangles()
        .iter()
        .enumerate()
        .filter(|(index, _)| package.topological_reference_for_triangle(*index) == Some(reference))
        .map(|(_, triangle)| {
            let [a, b, c] = triangle.vertex_indices.map(vertex);
            let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
            let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
            let n = ketchup_geometry::linalg::cross(u, v);
            let centroid = std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.0);
            (ketchup_geometry::linalg::dot(n, n).sqrt(), centroid, n)
        })
        .filter(|(area, ..)| *area > ROUNDING)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(area, centroid, n)| (centroid, n.map(|value| value / area)))
}

/// The name the solid's topology gives a face, in program terms.
fn role(reference: &TopologicalElementRef) -> Option<&str> {
    match reference.producer_element_id.as_str() {
        "" => None,
        role if role == ExactFaceRole::Top.semantic_role() => Some("end"),
        role if role == ExactFaceRole::Bottom.semantic_role() => Some("start"),
        name => Some(name),
    }
}

fn describe_face(
    model: &ProgramModel,
    part: &Part,
    package: &ExactBodyPackage,
    ordinal: u32,
) -> Option<Value> {
    let reference = package.topological_reference(TopologicalElementKind::Face, ordinal)?;
    let (point, normal) = face_sample(package, ordinal)?;
    let normal_world = frame::apply(&part.rotation, normal);
    let on_face = (0..3)
        .find(|axis| normal[*axis].abs() > 1.0 - APPROXIMATION)
        .map_or_else(
            || json!(round(normal_world)),
            |axis| {
                json!(format!(
                    "{}{}",
                    ["x", "y", "z"][axis],
                    if normal[axis] > 0.0 { "+" } else { "-" }
                ))
            },
        );
    let holes = provenance::describe(model, part, package, ordinal);
    Some(
        json!({"face": part.face_at(point, normal, role(reference)), "holes": holes, "machining_provenance": {"state": if holes.is_empty() { "not_identified" } else { "surface_matches" }, "scope": "finished_hole_operations", "reason": if holes.is_empty() { json!("This face is not matched to a supported drilling operation; it may be an exterior, pocket or boolean surface. No ownership is inferred.") } else { Value::Null }},
            "on_face": on_face,
            "point_local_mm": round(point),
            "normal_local": round(normal),
            "point_world_mm": round(part.to_world(point)),
            "normal_world": round(normal_world),
        }),
    )
}

/// The program part and the two program face names of a picked edge, or
/// `None` when the edge cannot be told in program words.
pub(crate) fn edge_names(
    app: &KetchupApp,
    snapshot: &Snapshot,
    instance_path: &InstancePath,
    reference: &TopologicalElementRef,
) -> Option<(String, [String; 2])> {
    let described = describe(app, snapshot, instance_path, reference)?;
    let [first, second] = described["edge"].as_array()?.as_slice() else {
        return None;
    };
    Some((
        described["part"].as_str()?.to_owned(),
        [first.as_str()?.to_owned(), second.as_str()?.to_owned()],
    ))
}

/// The program's view of the one picked face or edge, or `None` when no
/// program owns the picked part or its solid is not evaluated yet.
pub(super) fn describe(
    app: &KetchupApp,
    snapshot: &Snapshot,
    instance_path: &InstancePath,
    reference: &TopologicalElementRef,
) -> Option<Value> {
    let program = app.document.current_rule_program()?;
    let name = part_name(snapshot, instance_path)?;
    let occurrence = snapshot.resolve_instance_path(instance_path).ok()?;
    // Only the parts' geometry is read, from the window's kept evaluation.
    let evaluated = app.program_evaluations.get(program).ok()?;
    let part = evaluated.model.part(&name)?;
    let package = app
        .exact
        .topology_results
        .get_render(snapshot, occurrence.definition_id)?;
    let ordinal = |kind| {
        package
            .topological_references()
            .iter()
            .filter(|candidate| candidate.kind == kind)
            .position(|candidate| candidate == reference)
            .and_then(|ordinal| u32::try_from(ordinal).ok())
    };
    let mut described = match reference.kind {
        TopologicalElementKind::Face => describe_face(
            &evaluated.model,
            part,
            package,
            ordinal(TopologicalElementKind::Face)?,
        )?,
        TopologicalElementKind::Edge => {
            let edge = ordinal(TopologicalElementKind::Edge)?;
            let faces = package
                .edge_evidence()
                .iter()
                .find(|evidence| evidence.edge_ordinal == edge)?
                .adjacent_face_ordinals
                .iter()
                .map(|face| describe_face(&evaluated.model, part, package, *face))
                .collect::<Option<Vec<_>>>()?;
            let names = faces
                .iter()
                .map(|face| face["face"].as_str())
                .collect::<Option<Vec<_>>>();
            json!({
                "edge": names.filter(|names| names.len() == 2),
                "faces": faces,
            })
        }
        TopologicalElementKind::Vertex => return None,
    };
    described["part"] = json!(part.name);
    described["representation"] = json!("solid_part");
    described["hint"] = json!(HINT);
    Some(described)
}
