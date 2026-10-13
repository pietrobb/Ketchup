//! Hole tool: a click on a planar face of an exact part drills one round hole
//! there, the same `drill_holes` edit an MCP client sends.

use crate::*;
use ketchup_assistant::sidecar::AssistantHoleThread;

/// The hole the value box asks for: "6" (Ø6 through), "6 10" (Ø6, 10 deep) or
/// "M8 12" (the fastener library's M8 tap drill, threaded and drilled 12 deep).
#[derive(Debug, PartialEq)]
pub(crate) struct HoleSpec {
    pub diameter_mm: f64,
    pub depth_mm: Option<f64>,
    pub thread: Option<AssistantHoleThread>,
}

pub(crate) fn hole_spec(input: &str) -> Option<HoleSpec> {
    let mut words = input
        .split(|c: char| c.is_whitespace() || matches!(c, 'x' | 'X' | '×'))
        .filter(|word| !word.is_empty());
    let size = words.next()?;
    let depth_mm = match words.next() {
        Some(word) => Some(parse_distance_mm(word).filter(|depth| *depth > 0.0)?),
        None => None,
    };
    if words.next().is_some() {
        return None;
    }
    if size.starts_with(['M', 'm']) {
        let size = size.to_uppercase();
        let diameter_mm = ketchup_program::library_metric_threads()
            .ok()?
            .get(&size)?
            .tap_drill_mm;
        let depth_mm = depth_mm?;
        return Some(HoleSpec {
            diameter_mm,
            depth_mm: Some(depth_mm),
            thread: Some(AssistantHoleThread { size, depth_mm }),
        });
    }
    Some(HoleSpec {
        diameter_mm: parse_distance_mm(size).filter(|diameter| *diameter > 0.0)?,
        depth_mm,
        thread: None,
    })
}

impl KetchupApp {
    /// Hands a hole at the exact face under `pointer` to the bridge as the MCP
    /// `drill_holes` edit: one Undo step or a named refusal.
    pub(crate) fn drill_hole_at_screen(&mut self, pointer: Pos2, rect: Rect) -> bool {
        let Some(spec) = hole_spec(&self.value_box.input) else {
            self.digest = self.catalog.text("digest-hole-invalid-size");
            return false;
        };
        let Some((reference, point_local_mm)) = self.hole_face_at_screen(pointer, rect) else {
            self.digest = self.catalog.text("digest-hole-face-required");
            return false;
        };
        let operation = AssistantCadEditOperation::DrillHoles {
            definition_id: reference.definition_id.0,
            name: self.catalog.text("model-hole-feature"),
            target_feature_id: reference.producer_feature_id.0,
            face_reference_id: reference.lineage_digest,
            points_local_mm: vec![point_local_mm],
            diameter_mm: spec.diameter_mm,
            depth_mm: spec.depth_mm,
            through: spec.depth_mm.is_none(),
            thread: spec.thread,
        };
        self.queue_window_edit(operation, "digest-hole-created")
    }

    /// The exact face nearest along the view ray and the hit point in its
    /// part's coordinates.
    fn hole_face_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
    ) -> Option<(TopologicalElementRef, [f64; 3])> {
        let ray = self.view_ray(pointer, rect)?;
        let snapshot = self.document.current();
        let topology_results = self.topology_results_for_snapshot(&snapshot)?;
        let hit = ExactInteractionProjection::from_snapshot(&snapshot, topology_results)
            .exact_surface_picks(ray)
            .into_iter()
            .next()?;
        let face = hit
            .topological_target?
            .resolve_current(&snapshot, topology_results)
            .ok()?;
        if face.reference.kind != TopologicalElementKind::Face {
            return None;
        }
        let to_local = snapshot
            .resolve_instance_path(&hit.instance_path)
            .ok()?
            .world_transform
            .rigid_inverse()?;
        let point = hit.position_mm;
        Some((
            face.reference,
            to_local.transform_point([point.x, point.y, point.z]),
        ))
    }
}
