//! Shared asynchronous execution and publication. No GUI event loop is required.
use super::*;

pub type ExactSource = (DocumentId, u64, String);
pub fn exact_source(snapshot: &Snapshot) -> ExactSource {
    (
        snapshot.document_id(),
        snapshot.revision_id(),
        snapshot.canonical_digest(),
    )
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProducerKey {
    pub definition_id: DefinitionId,
    pub feature_id: FeatureId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceStatus {
    Current,
    Evaluated,
    Failed { reason: String },
    NotEvaluated { reason: String },
}
impl EvidenceStatus {
    pub fn is_evaluated(&self) -> bool {
        matches!(self, Self::Current | Self::Evaluated)
    }
    pub(super) fn not_evaluated(reason: &str) -> Self {
        Self::NotEvaluated {
            reason: reason.to_owned(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProducerCoverage {
    pub key: ProducerKey,
    pub render: EvidenceStatus,
    pub topology: EvidenceStatus,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationReport {
    pub source: ExactSource,
    pub producers: Vec<ProducerCoverage>,
    /// Empty geometry is not a global exact-evaluation pass.
    pub complete: bool,
    pub topology_complete: bool,
    pub not_evaluated: Option<String>,
}
impl EvaluationReport {
    pub(super) fn finish(&mut self) {
        self.complete = !self.producers.is_empty()
            && self
                .producers
                .iter()
                .all(|entry| entry.render.is_evaluated());
        self.topology_complete = !self.producers.is_empty()
            && self
                .producers
                .iter()
                .all(|entry| entry.topology.is_evaluated());
        if self.producers.is_empty() && self.not_evaluated.is_none() {
            self.not_evaluated = Some("no exact producers selected".into());
        }
    }
}
pub struct ExactEvaluationProducts {
    pub(super) source: ExactSource,
    pub(super) render_packages: Vec<Arc<ExactBodyPackage>>,
    pub(super) topology_packages: Vec<Arc<ExactBodyPackage>>,
    pub(super) report: EvaluationReport,
}
pub type ExactEvaluationResult = Result<ExactEvaluationProducts, String>;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExactEvaluationProgress {
    pub total_producers: usize,
    pub completed_producers: usize,
    pub reused_producers: usize,
    pub active_producer: Option<ProducerKey>,
    pub active_elapsed_ms: Option<u64>,
}
pub struct ExactEvaluationTask {
    pub source: ExactSource,
    pub cancelled: Arc<AtomicBool>,
    pub finished: Arc<AtomicBool>,
    pub(super) receiver: Receiver<ExactEvaluationResult>,
    pub(super) total_producers: usize,
    pub(super) completed_producers: Arc<AtomicUsize>,
    pub(super) reused_producers: usize,
    pub(super) active_producer: Arc<Mutex<Option<(ProducerKey, Instant)>>>,
}
impl Drop for ExactEvaluationTask {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl ExactEvaluationTask {
    pub fn progress(&self) -> ExactEvaluationProgress {
        let active = *self
            .active_producer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ExactEvaluationProgress {
            total_producers: self.total_producers,
            completed_producers: self.completed_producers.load(Ordering::Acquire),
            reused_producers: self.reused_producers,
            active_producer: active.map(|(key, _)| key),
            active_elapsed_ms: active
                .map(|(_, started)| started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn poll(&self) -> Result<ExactEvaluationResult, TryRecvError> {
        self.receiver.try_recv()
    }
    pub fn wait(&self, timeout: Duration) -> ExactEvaluationResult {
        if self.cancelled.load(Ordering::Acquire) {
            return Err("exact evaluation cancelled".into());
        }
        match self.receiver.recv_timeout(timeout) {
            Ok(result) if !self.cancelled.load(Ordering::Acquire) => result,
            Ok(_) => Err("exact evaluation cancelled".into()),
            Err(RecvTimeoutError::Timeout) => {
                self.cancel();
                Err("exact evaluation timed out".into())
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err("exact evaluation worker disconnected".into())
            }
        }
    }
}

/// Re-check every product against the current canonical snapshot, also across Undo/Redo.
pub fn rebind_exact_results(
    snapshot: &Snapshot,
    render: &mut ExactResultRegistry,
    topology: &mut ExactResultRegistry,
) {
    if !render.is_bound_to(snapshot) {
        *render = ExactResultRegistry::carried_forward(snapshot, render);
    }
    if !topology.is_bound_to(snapshot) {
        *topology = ExactResultRegistry::carried_forward(snapshot, topology);
    }
}

/// Publish only snapshot-bound, uncancelled products. Evidence registration is sequential,
/// not a transaction; canonical content and the Undo stack are never edited here.
pub fn publish_exact_products(
    document: &mut DocumentStore,
    render: &mut ExactResultRegistry,
    topology: &mut ExactResultRegistry,
    task: &ExactEvaluationTask,
    products: ExactEvaluationProducts,
) -> Result<EvaluationReport, String> {
    let snapshot = document.current();
    if task.cancelled.load(Ordering::Acquire)
        || products.source != task.source
        || products.source != exact_source(&snapshot)
    {
        return Err("stale or cancelled exact evaluation".into());
    }
    let mut results = ExactResultRegistry::carried_forward(&snapshot, render);
    let mut topology_results = ExactResultRegistry::carried_forward(&snapshot, topology);
    for package in products.render_packages {
        results
            .insert_current(&snapshot, package)
            .map_err(|error| error.to_string())?;
    }
    for package in products.topology_packages {
        topology_results
            .insert_current(&snapshot, package)
            .map_err(|error| error.to_string())?;
    }
    let references = results
        .values()
        .flat_map(|package| package.references())
        .cloned()
        .collect::<Vec<_>>();
    for reference in references {
        let identity = format!(
            "role={}, source={}, profile={}, producer={}",
            reference.semantic_role,
            reference.source_element_id,
            reference.profile_feature_id.0,
            reference.producer_feature_id.0
        );
        document
            .register_exact_reference_evidence(reference)
            .map_err(|error| format!("{error}: {identity}"))?;
    }
    document
        .register_exact_reference_evidence(&results)
        .map_err(|error| error.to_string())?;
    *render = results;
    *topology = topology_results;
    Ok(products.report)
}
