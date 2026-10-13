//! Where the threads of a document run, as cylinders in their definition's
//! frame: every tapped hole the document records, and the outside thread of
//! every program part that names one (`thread`, `thread_from_mm`,
//! `thread_to_mm` along its local z). A cosmetic thread stays a plain
//! cylinder that a view draws the thread on; a modelled one is cut into it.

use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Snapshot, metric_nominal_diameter,
};
use ketchup_model::tolerance::ROUNDING;
use std::collections::{BTreeMap, BTreeSet};

pub use ketchup_model::document::ThreadCylinder;

/// Every thread of `snapshot`; the outside threads of program parts only
/// with the program's `evaluated` model.
#[must_use]
pub fn thread_cylinders(
    snapshot: &Snapshot,
    evaluated: Option<&ketchup_program::Evaluated>,
) -> Vec<ThreadCylinder> {
    let mut threads = snapshot
        .hole_threads()
        .filter_map(|(id, _)| snapshot.tapped_hole(id))
        .collect::<Vec<_>>();
    let Some(evaluated) = evaluated else {
        return threads;
    };
    let modeled = snapshot
        .shaft_threads()
        .filter_map(|(id, _)| snapshot.feature(id))
        .map(|feature| feature.definition_id())
        .collect::<BTreeSet<_>>();
    let parts = evaluated.model.parts_by_name();
    let mut seen = BTreeSet::new();
    for occurrence in snapshot.scene_query() {
        if !seen.insert(occurrence.definition_id) {
            continue;
        }
        let Some(part) = crate::rule_program_part_name(snapshot, &occurrence.instance_path)
            .and_then(|name| parts.get(name.as_str()).copied())
        else {
            continue;
        };
        threads.extend(
            outside_thread(occurrence.definition_id, &part.attributes).map(|thread| {
                ThreadCylinder {
                    modeled: modeled.contains(&thread.definition_id),
                    ..thread
                }
            }),
        );
    }
    threads
}

/// The batch that models the threads of `snapshot` (`modeled`) or draws them
/// cosmetic where a thread does not say, with the bolts and rods of the
/// program `model` that owns the document. A program keeps owning it.
#[must_use]
pub fn modeled_threads_batch(
    snapshot: &Snapshot,
    model: Option<&ketchup_program::ProgramModel>,
    modeled: bool,
) -> CommandBatch {
    let mut commands = vec![CanonicalCommand::SetModeledThreads { modeled }];
    crate::rule_threads::append(snapshot, model, modeled, &mut commands);
    CommandBatch::new(commands)
}

/// The outside thread `attributes` name along local z: size and span.
pub(crate) fn outside_thread(
    definition_id: DefinitionId,
    attributes: &BTreeMap<String, String>,
) -> Option<ThreadCylinder> {
    let size = attributes.get("thread")?;
    let bound = |key: &str| attributes.get(key)?.parse::<f64>().ok();
    let (from, to) = (bound("thread_from_mm")?, bound("thread_to_mm")?);
    let pitch_mm = ketchup_program::eval::library_metric_threads()
        .ok()?
        .get(size)?
        .pitch_mm;
    let nominal_diameter_mm = metric_nominal_diameter(size)?;
    (to - from > ROUNDING).then(|| ThreadCylinder {
        definition_id,
        size: size.clone(),
        pitch_mm,
        nominal_diameter_mm,
        surface_diameter_mm: nominal_diameter_mm,
        start_mm: [0.0, 0.0, from],
        direction: [0.0, 0.0, 1.0],
        length_mm: to - from,
        internal: false,
        modeled: false,
    })
}
