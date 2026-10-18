//! Hole tool: a click on a planar face of an exact part drills one round hole
//! there, the same `drill_holes` edit an MCP client sends.

use crate::length_input::{DecimalSeparator, VALUE_SEPARATORS, parse_length_list};
use crate::live_bridge::window_edit::Repick;
use crate::*;
use ketchup_assistant::sidecar::AssistantHoleThread;

/// The hole the value box asks for: "6" (Ø6 through), "6 10" (Ø6, 10 deep) or
/// "M8 12" (the fastener library's M8 tap drill, threaded 12 deep and drilled
/// its blind-hole runout deeper, 18.2).
#[derive(Debug, PartialEq)]
pub(crate) struct HoleSpec {
    pub diameter_mm: f64,
    pub depth_mm: Option<f64>,
    pub thread: Option<AssistantHoleThread>,
}

/// Each length is read like every value of the window, so `M8 1 cm` and
/// `6 mm; 12` work: a thread size ends at the first space or separator and the
/// rest is its depth; a diameter and depth are separated by `;`, `x` or `:`, or
/// by the one space where both sides are lengths.
pub(crate) fn hole_spec(input: &str, decimal: DecimalSeparator) -> Option<HoleSpec> {
    let input = input.trim();
    let positive = |text: &str| parse_distance_mm(text, decimal).filter(|mm| *mm > 0.0);
    if input.starts_with(['M', 'm']) {
        let end = input
            .find(|c: char| c.is_whitespace() || VALUE_SEPARATORS.contains(&c))
            .unwrap_or(input.len());
        let size = input[..end].to_uppercase();
        let depth = input[end..].trim_start();
        let depth = depth.strip_prefix(VALUE_SEPARATORS).unwrap_or(depth);
        let metric = *ketchup_program::library_metric_threads().ok()?.get(&size)?;
        let depth_mm = positive(depth)?;
        return Some(HoleSpec {
            diameter_mm: metric.tap_drill_mm,
            depth_mm: Some(metric.blind_hole_depth_mm(depth_mm)),
            thread: Some(AssistantHoleThread {
                size,
                depth_mm,
                modeled_thread: None,
            }),
        });
    }
    let (diameter, depth) = match parse_length_list(input, decimal, 1..=2).as_deref() {
        Some(&[diameter]) => (diameter, None),
        Some(&[diameter, depth]) => (diameter, Some(depth)),
        _ => {
            let (diameter, depth) = split_at_one_space(input, decimal)?;
            (diameter, Some(depth))
        }
    };
    Some(HoleSpec {
        diameter_mm: Some(diameter).filter(|mm| *mm > 0.0)?,
        depth_mm: match depth {
            Some(depth) => Some(Some(depth).filter(|mm| *mm > 0.0)?),
            None => None,
        },
        thread: None,
    })
}

/// The two lengths of `6 10` or `6 mm 1 cm`: the space where the text on
/// either side is a length, refused when two spaces read differently.
fn split_at_one_space(input: &str, decimal: DecimalSeparator) -> Option<(f64, f64)> {
    let mut found = None;
    for (at, _) in input.char_indices().filter(|(_, c)| c.is_whitespace()) {
        let (Some(left), Some(right)) = (
            parse_distance_mm(&input[..at], decimal),
            parse_distance_mm(&input[at..], decimal),
        ) else {
            continue;
        };
        if found
            .replace((left, right))
            .is_some_and(|other| other != (left, right))
        {
            return None;
        }
    }
    found
}

impl KetchupApp {
    /// Hands a hole at the exact face under `pointer` to the bridge as the MCP
    /// `drill_holes` edit: one Undo step or a named refusal.
    pub(crate) fn drill_hole_at_screen(&mut self, pointer: Pos2, rect: Rect) -> bool {
        let Some(spec) = hole_spec(&self.value_box.input, self.value_box.decimal) else {
            self.digest = self.catalog.text("digest-hole-invalid-size");
            return false;
        };
        let Some((ray, operation)) = self
            .view_ray(pointer, rect)
            .and_then(|ray| Some((ray, self.hole_on_ray(ray, &spec)?)))
        else {
            self.digest = self.catalog.text("digest-hole-face-required");
            return false;
        };
        self.queue_window_edit(
            operation,
            "digest-hole-created",
            Some(Repick::Hole { ray, spec }),
        );
        true
    }

    /// The hole `spec` at the exact face `ray` hits first, as the document is
    /// now. A hole clicked while an earlier one was checked is picked again on
    /// the document that one left: its face then belongs to the new hole.
    pub(crate) fn hole_on_ray(
        &self,
        ray: Ray,
        spec: &HoleSpec,
    ) -> Option<AssistantCadEditOperation> {
        let (reference, point_local_mm) = self.face_on_ray(ray)?;
        Some(AssistantCadEditOperation::DrillHoles {
            definition_id: reference.definition_id.0,
            name: self.catalog.text("model-hole-feature"),
            target_feature_id: reference.producer_feature_id.0,
            face_reference_id: reference.lineage_digest,
            points_local_mm: vec![point_local_mm],
            diameter_mm: spec.diameter_mm,
            depth_mm: spec.depth_mm,
            through: spec.depth_mm.is_none(),
            thread: spec.thread.clone(),
        })
    }

    /// The exact face nearest along the view ray that the section keeps, and
    /// the hit point in its part's coordinates.
    pub(super) fn face_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
    ) -> Option<(TopologicalElementRef, [f64; 3])> {
        self.face_on_ray(self.view_ray(pointer, rect)?)
    }

    fn face_on_ray(&self, ray: Ray) -> Option<(TopologicalElementRef, [f64; 3])> {
        let snapshot = self.document.current();
        let topology_results = self.topology_results_for_snapshot(&snapshot)?;
        let hit = ExactInteractionProjection::from_snapshot(&snapshot, topology_results)
            .exact_surface_picks(ray)
            .into_iter()
            .find(|hit| self.section_keeps(hit.position_mm))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::length_input::DecimalSeparator::{Comma, Point};

    fn plain(diameter_mm: f64, depth_mm: Option<f64>) -> Option<HoleSpec> {
        Some(HoleSpec {
            diameter_mm,
            depth_mm,
            thread: None,
        })
    }

    /// Review 2026-10-17 P3-G: a depth with its unit after a thread size
    /// was split at its space and refused.
    #[test]
    fn a_thread_depth_is_one_length_with_its_unit() {
        let m8 = hole_spec("M8 10", Point);
        assert_eq!(m8.as_ref().unwrap().thread.as_ref().unwrap().depth_mm, 10.0);
        for input in ["M8 1 cm", "m8 1cm", "M8;10", "M8 x 10 mm", "M8 2*5"] {
            assert_eq!(hole_spec(input, Point), m8, "{input}");
        }
        let half = hole_spec("M8 0,5 cm", Comma).unwrap().thread.unwrap();
        assert_eq!(half.depth_mm, 5.0);
        for input in ["M8", "M8 0", "M8 1 cm 2", "M7 12", "M8 1,5"] {
            assert_eq!(hole_spec(input, Point), None, "{input}");
        }
    }

    #[test]
    fn a_diameter_and_depth_are_two_lengths() {
        assert_eq!(hole_spec("6", Point), plain(6.0, None));
        assert_eq!(hole_spec("6 mm", Point), plain(6.0, None));
        for input in ["6 10", "6;10", "6x10", "6 mm 1 cm", "6  10", "3*2 1cm"] {
            assert_eq!(hole_spec(input, Point), plain(6.0, Some(10.0)), "{input}");
        }
        assert_eq!(hole_spec("6,5 10", Comma), plain(6.5, Some(10.0)));
        for input in ["0", "6 0", "-6 10", "6 10 2", "6,10", ""] {
            assert_eq!(hole_spec(input, Point), None, "{input}");
        }
    }
}
