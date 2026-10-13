//! Reconcile the modelled outside threads of program parts with what the
//! program and the document say: a bolt or rod's thread is modelled when its
//! `modeled_thread=` says so, else when the document models threads. Only a
//! modelled thread has a record, on the feature that makes the part's body,
//! so a cosmetic document keeps none. A bolt in a tapped hole of its size on
//! its axis takes that hole's phase, so the two mesh at any depth and turn.

use crate::rule_program_part_name;
use crate::threads::outside_thread;
use ketchup_geometry::linalg::Vec3;
use ketchup_geometry::thread_profile::ThreadPhase;
use ketchup_model::document::{
    CanonicalCommand, DefinitionId, FeatureId, ShaftThread, Snapshot, ThreadCylinder, Transform,
};
use ketchup_model::tolerance::ROUNDING;
use ketchup_program::ProgramModel;
use std::collections::BTreeMap;

/// The program attribute a part's `modeled_thread=` sets: "true" or "false".
const MODELED_THREAD: &str = "modeled_thread";

/// Whether reconciling can change anything: some part names a thread, or the
/// document records a modelled one.
pub(crate) fn needed(snapshot: &Snapshot, model: &ProgramModel) -> bool {
    snapshot.shaft_threads().next().is_some()
        || model
            .parts
            .iter()
            .any(|part| part.attributes.contains_key("thread"))
}

/// Appends to `commands` what makes the modelled outside threads of
/// `snapshot` those of `model` while the document models threads where they
/// do not say (`document_modeled`).
pub(crate) fn append(
    snapshot: &Snapshot,
    model: Option<&ProgramModel>,
    document_modeled: bool,
    commands: &mut Vec<CanonicalCommand>,
) {
    let wanted = model
        .map(|model| wanted(snapshot, model, document_modeled))
        .unwrap_or_default();
    let recorded = snapshot.shaft_threads().collect::<BTreeMap<_, _>>();
    for (id, thread) in &wanted {
        if recorded.get(id) != Some(&thread) {
            commands.push(CanonicalCommand::SetShaftThread {
                id: *id,
                thread: Some(thread.clone()),
            });
        }
    }
    for id in recorded.keys().filter(|id| !wanted.contains_key(id)) {
        commands.push(CanonicalCommand::SetShaftThread {
            id: *id,
            thread: None,
        });
    }
}

/// The modelled outside threads `model` asks for, by the feature that makes
/// each threaded part's body.
fn wanted(
    snapshot: &Snapshot,
    model: &ProgramModel,
    document_modeled: bool,
) -> BTreeMap<FeatureId, ShaftThread> {
    let parts = model.parts_by_name();
    let scene = snapshot.scene_query();
    let placements = |definition_id: DefinitionId| {
        scene
            .iter()
            .filter(|occurrence| occurrence.definition_id == definition_id)
            .map(|occurrence| occurrence.transform)
            .collect::<Vec<_>>()
    };
    let holes = snapshot
        .hole_threads()
        .filter_map(|(id, _)| snapshot.tapped_hole(id))
        .collect::<Vec<_>>();
    let mut wanted = BTreeMap::new();
    for occurrence in &scene {
        let Some(part) = rule_program_part_name(snapshot, &occurrence.instance_path)
            .and_then(|name| parts.get(name.as_str()).copied())
        else {
            continue;
        };
        let modeled = match part.attributes.get(MODELED_THREAD).map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            _ => document_modeled,
        };
        let Some(thread) =
            outside_thread(occurrence.definition_id, &part.attributes).filter(|_| modeled)
        else {
            continue;
        };
        let Some(&feature_id) = snapshot
            .definition(occurrence.definition_id)
            .and_then(|definition| definition.feature_ids().last())
        else {
            continue;
        };
        if wanted.contains_key(&feature_id) {
            continue;
        }
        let phase = match placements(occurrence.definition_id).as_slice() {
            [bolt] => holes
                .iter()
                .find_map(|hole| hole_phase(snapshot, hole, &thread, *bolt, &placements)),
            _ => None,
        }
        .or_else(|| ThreadPhase::in_own_part(thread.start_mm.into(), thread.direction.into()));
        let Some(phase) = phase else {
            continue;
        };
        wanted.insert(
            feature_id,
            ShaftThread {
                size: thread.size.clone(),
                pitch_mm: thread.pitch_mm,
                surface_diameter_mm: thread.surface_diameter_mm,
                start_mm: thread.start_mm,
                direction: thread.direction,
                length_mm: thread.length_mm,
                phase_origin_mm: phase.origin.to_array(),
                phase_reference: phase.reference.to_array(),
            },
        );
    }
    wanted
}

/// The phase of `hole` carried into the coordinates of the bolt `thread`
/// placed at `bolt`, when the hole is tapped with its size on its axis and
/// its part is placed once.
fn hole_phase(
    snapshot: &Snapshot,
    hole: &ThreadCylinder,
    thread: &ThreadCylinder,
    bolt: Transform,
    placements: &impl Fn(DefinitionId) -> Vec<Transform>,
) -> Option<ThreadPhase> {
    if hole.size != thread.size {
        return None;
    }
    let [part] = placements(hole.definition_id)[..] else {
        return None;
    };
    let bolt_in_hole = part.affine().invert().ok()?.compose(bolt.affine());
    let origin = bolt_in_hole.transform_point(thread.start_mm.into());
    let axis = bolt_in_hole.transform_vector(thread.direction.into());
    let hole_axis = Vec3::from(hole.direction);
    let off_axis = origin - Vec3::from(hole.start_mm);
    let tolerance = snapshot.tolerance().linear_mm();
    if axis.cross(hole_axis).length() > ROUNDING || off_axis.cross(hole_axis).length() > tolerance {
        return None;
    }
    let phase = ThreadPhase::in_own_part(hole.start_mm.into(), hole_axis)?.in_part(bolt_in_hole)?;
    // The phase runs along the bolt's own direction: a right-handed helix is
    // the same helix about either sense of its axis.
    Some(ThreadPhase {
        axis: thread.direction.into(),
        ..phase
    })
}
