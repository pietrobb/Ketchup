//! Shared asynchronous execution and publication. No GUI event loop is required.
use super::*;

pub type ExactSource = ExactProducerEvidenceContext;
pub fn exact_source(snapshot: &Snapshot) -> ExactSource {
    ExactProducerEvidenceContext::from_snapshot(snapshot)
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
pub enum ExactEvaluationSelection {
    Full,
    Scoped(BTreeSet<ProducerKey>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationReport {
    pub source: ExactSource,
    pub selection: ExactEvaluationSelection,
    pub producers: Vec<ProducerCoverage>,
    /// Empty geometry is not a global exact-evaluation pass.
    pub complete: bool,
    pub topology_complete: bool,
    pub not_evaluated: Option<String>,
}
impl EvaluationReport {
    pub fn establishes_full_baseline(&self) -> bool {
        self.selection == ExactEvaluationSelection::Full && self.complete && self.topology_complete
    }

    pub fn needs_retry(&self) -> bool {
        !self.complete
            || self
                .producers
                .iter()
                .any(|entry| matches!(entry.topology, EvidenceStatus::Failed { .. }))
    }

    pub(super) fn finish(&mut self) {
        // With no producers selected nothing is missing: an incremental plan
        // that changed no geometry (a color or grounding edit) keeps its
        // baseline, and a document without exact geometry has nothing to evaluate.
        self.complete = self
            .producers
            .iter()
            .all(|entry| entry.render.is_evaluated());
        self.topology_complete = self
            .producers
            .iter()
            .all(|entry| entry.topology.is_evaluated());
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
    pub selection: ExactEvaluationSelection,
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

/// Materialize snapshot-bound exact products without publishing canonical or GUI state.
/// This is the pre-publication boundary used by guarded candidate validation.
pub fn materialize_exact_products(
    snapshot: &Snapshot,
    render: &ExactResultRegistry,
    topology: &ExactResultRegistry,
    task: &ExactEvaluationTask,
    products: ExactEvaluationProducts,
) -> Result<(ExactResultRegistry, ExactResultRegistry, EvaluationReport), String> {
    if task.cancelled.load(Ordering::Acquire)
        || products.source != task.source
        || products.source != exact_source(snapshot)
    {
        return Err("stale or cancelled exact evaluation".into());
    }
    let mut results = ExactResultRegistry::carried_forward(snapshot, render);
    let mut topology_results = ExactResultRegistry::carried_forward(snapshot, topology);
    for package in products.render_packages {
        results
            .insert_current(snapshot, package)
            .map_err(|error| error.to_string())?;
    }
    for package in products.topology_packages {
        topology_results
            .insert_current(snapshot, package)
            .map_err(|error| error.to_string())?;
    }
    Ok((results, topology_results, products.report))
}

/// Publish only snapshot-bound, uncancelled products. Evidence registration is atomic;
/// canonical content and the Undo stack are never edited here.
pub fn publish_exact_products(
    document: &mut DocumentStore,
    render: &mut ExactResultRegistry,
    topology: &mut ExactResultRegistry,
    task: &ExactEvaluationTask,
    products: ExactEvaluationProducts,
) -> Result<EvaluationReport, String> {
    let snapshot = document.current();
    let (results, topology_results, report) =
        materialize_exact_products(&snapshot, render, topology, task, products)?;
    let references = results
        .values()
        .flat_map(|package| package.references())
        .cloned()
        .collect::<Vec<_>>();
    document.try_canonical_transaction(
        |document| {
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
                .map_err(|error| error.to_string())
        },
        |_| Ok(()),
    )?;
    *render = results;
    *topology = topology_results;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_core::document::{
        CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
        FeatureKind, OccurrenceId, Transform,
    };
    use ketchup_core::exact_product::{ExactBodyPackage, ExactFaceRole, ExactResultRegistry};
    use ketchup_core::sketch::{
        PrincipalPlane, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport, WorkplaneSupportHealth,
    };
    use ketchup_core::testing::box_package;

    const DEFINITION: DefinitionId = DefinitionId(1);
    const PROFILE: FeatureId = FeatureId(10);
    const EXTRUSION: FeatureId = FeatureId(11);
    const FACE_PLANE: FeatureId = FeatureId(12);

    fn package(
        snapshot: &Snapshot,
        fingerprint: &str,
        roles: &[ExactFaceRole],
    ) -> ExactBodyPackage {
        box_package(snapshot, DEFINITION, EXTRUSION, fingerprint, roles).unwrap()
    }

    fn seed() -> DocumentStore {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Part".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: PROFILE,
                    definition_id: DEFINITION,
                    name: "Profile".into(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: EXTRUSION,
                    definition_id: DEFINITION,
                    name: "Extrusion".into(),
                    kind: FeatureKind::Extrusion {
                        profile: PROFILE,
                        height: Dimension::from_decimal("5").unwrap(),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(20),
                    definition_id: DEFINITION,
                    name: "Part occurrence".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        let anchor =
            package(&document.current(), "anchor", &[ExactFaceRole::Top]).references()[0].clone();
        document
            .register_exact_reference_evidence(anchor.clone())
            .unwrap();
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id: FACE_PLANE,
                definition_id: DEFINITION,
                name: "Face plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(anchor),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(5.0),
                }),
            }]))
            .unwrap();
        document
    }

    #[test]
    fn exact_publication_is_atomic_when_later_reference_conflicts() {
        let mut document = seed();
        let snapshot = document.current();
        let before = snapshot
            .exact_reference_evidence()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(before.len(), 1);
        let package = package(
            &snapshot,
            "conflicting",
            &[
                ExactFaceRole::Bottom,
                ExactFaceRole::LinearSide,
                ExactFaceRole::Top,
            ],
        );
        let source = exact_source(&snapshot);
        let (_sender, receiver) = mpsc::channel();
        let task = ExactEvaluationTask {
            source: source.clone(),
            selection: ExactEvaluationSelection::Full,
            cancelled: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(true)),
            receiver,
            total_producers: 1,
            completed_producers: Arc::new(AtomicUsize::new(1)),
            reused_producers: 0,
            active_producer: Arc::new(Mutex::new(None)),
        };
        let products = ExactEvaluationProducts {
            source: source.clone(),
            render_packages: vec![Arc::new(package)],
            topology_packages: Vec::new(),
            report: EvaluationReport {
                source,
                selection: ExactEvaluationSelection::Full,
                producers: Vec::new(),
                complete: true,
                topology_complete: false,
                not_evaluated: None,
            },
        };
        let mut render = ExactResultRegistry::default();
        let mut topology = ExactResultRegistry::default();

        publish_exact_products(&mut document, &mut render, &mut topology, &task, products)
            .unwrap_err();

        assert_eq!(
            document
                .current()
                .exact_reference_evidence()
                .cloned()
                .collect::<Vec<_>>(),
            before
        );
        assert!(render.is_empty());
        assert!(topology.is_empty());
    }
}
