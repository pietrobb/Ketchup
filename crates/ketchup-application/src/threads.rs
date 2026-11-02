//! Where the threads of a document run, as cylinders in their definition's
//! frame: every tapped hole the document records, and the outside thread of
//! every program part that names one (`thread`, `thread_from_mm`,
//! `thread_to_mm` along its local z). A cosmetic thread stays a plain
//! cylinder that a view draws the thread on; a modelled one is cut into it.

use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Snapshot, metric_nominal_diameter,
};
use ketchup_model::exact_brep_graph::{ExactBRepGraph, ExactBRepGraphError, ExactBRepOperation};
use ketchup_model::exact_product::ExactBRepGraphPackage;
use ketchup_model::tolerance::ROUNDING;
use ketchup_scheduler::WorkerError;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub use ketchup_model::document::ThreadCylinder;

/// Why the body of a modelled thread could not be made cosmetic.
#[derive(Debug)]
pub enum CosmeticThreadBodyError {
    Graph(ExactBRepGraphError),
    /// The document holds no imported source with this SHA-256.
    ImportedSourceMissing(String),
    Unavailable(crate::worker_pool::ExactWorkerUnavailable),
    Worker(WorkerError),
}

impl fmt::Display for CosmeticThreadBodyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(error) => {
                write!(formatter, "the graph without threads is invalid: {error}")
            }
            Self::ImportedSourceMissing(sha256) => {
                write!(formatter, "imported source {sha256} is missing")
            }
            Self::Unavailable(error) => error.fmt(formatter),
            Self::Worker(error) => write!(formatter, "exact worker failed: {error}"),
        }
    }
}

impl std::error::Error for CosmeticThreadBodyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Graph(error) => Some(error),
            Self::ImportedSourceMissing(_) => None,
            Self::Unavailable(error) => Some(error),
            Self::Worker(error) => Some(error),
        }
    }
}

/// What `package`'s body is drawn from with its modelled threads cosmetic, so
/// a drawing shows them by ISO 6410: its graph without the thread grooves and
/// the imported sources that graph reads. Made where the document is, without
/// a worker; [`Self::evaluate`] then waits for one wherever the caller can let
/// it be cancelled (review 2026-11-01 UI-100, UI-11).
#[derive(Clone, Debug)]
pub struct CosmeticThreadGraph {
    graph: ExactBRepGraph,
    sources: Vec<Arc<Vec<u8>>>,
}

/// The cosmetic-thread graph of `package`'s body, or `None` when the body
/// models no thread. `blobs` are the document's imported sources by SHA-256.
///
/// # Errors
/// The graph without grooves is invalid or an imported source is missing.
pub fn cosmetic_thread_graph(
    package: &ExactBRepGraphPackage,
    blobs: &ketchup_model::persistence::Blobs,
) -> Result<Option<CosmeticThreadGraph>, CosmeticThreadBodyError> {
    let Some(graph) = package
        .graph
        .without_thread_grooves()
        .map_err(CosmeticThreadBodyError::Graph)?
    else {
        return Ok(None);
    };
    let mut hashes = Vec::new();
    for node in &graph.nodes {
        if let ExactBRepOperation::ImportedExact { source_sha256, .. } = &node.operation {
            let hash = source_sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if !hashes.contains(&hash) {
                hashes.push(hash);
            }
        }
    }
    let sources = hashes
        .into_iter()
        .map(|hash| {
            blobs
                .get(&hash)
                .cloned()
                .ok_or(CosmeticThreadBodyError::ImportedSourceMissing(hash))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(CosmeticThreadGraph { graph, sources }))
}

impl CosmeticThreadGraph {
    /// The body without its thread grooves, evaluated by a pooled exact worker
    /// at `executable`, whose output cache already holds the nodes before them;
    /// `cancelled` stops the worker.
    ///
    /// # Errors
    /// No exact worker evaluates it, or it was cancelled.
    pub fn evaluate(
        &self,
        executable: Option<&Path>,
        cancelled: &AtomicBool,
    ) -> Result<ExactBRepGraphPackage, CosmeticThreadBodyError> {
        let sources = self
            .sources
            .iter()
            .map(|source| source.as_slice())
            .collect::<Vec<_>>();
        let mut worker = crate::worker_pool::checkout(executable, cancelled)
            .map_err(CosmeticThreadBodyError::Unavailable)?;
        let body = worker
            .evaluate_exact_brep_graph_with_imported_sources_and_cancellation(
                &self.graph,
                &sources,
                cancelled,
            )
            .map_err(CosmeticThreadBodyError::Worker)?;
        worker.release();
        Ok(body)
    }
}

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
