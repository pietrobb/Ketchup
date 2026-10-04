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
        // An incremental plan that selects no producers (e.g. a color or grounding
        // edit) changed no geometry: the baseline stays valid and nothing is missing.
        let unchanged_geometry = matches!(
            &self.selection,
            ExactEvaluationSelection::Scoped(producers) if producers.is_empty()
        );
        self.complete = unchanged_geometry
            || !self.producers.is_empty()
                && self
                    .producers
                    .iter()
                    .all(|entry| entry.render.is_evaluated());
        self.topology_complete = unchanged_geometry
            || !self.producers.is_empty()
                && self
                    .producers
                    .iter()
                    .all(|entry| entry.topology.is_evaluated());
        if self.producers.is_empty() && !unchanged_geometry && self.not_evaluated.is_none() {
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
pub type ExactEvaluationResult = Result<ExactEvaluationProducts, ExactEvaluationError>;

/// Why an exact evaluation task produced nothing that can be published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactEvaluationError {
    Cancelled,
    TimedOut,
    WorkerDisconnected,
    /// The products belong to another canonical state than the one being published.
    Stale,
    Package(ExactProductError),
    /// The document refused the exact reference evidence; `reference` names it when
    /// a single reference was refused.
    ReferenceEvidence {
        reference: Option<String>,
        error: ReferenceEvidenceError,
    },
}

impl std::fmt::Display for ExactEvaluationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("exact evaluation cancelled"),
            Self::TimedOut => f.write_str("exact evaluation timed out"),
            Self::WorkerDisconnected => f.write_str("exact evaluation worker disconnected"),
            Self::Stale => f.write_str("exact evaluation products are stale"),
            Self::Package(error) => write!(f, "exact package cannot be registered: {error}"),
            Self::ReferenceEvidence {
                reference: Some(reference),
                error,
            } => write!(f, "{error}: {reference}"),
            Self::ReferenceEvidence {
                reference: None,
                error,
            } => error.fmt(f),
        }
    }
}

impl std::error::Error for ExactEvaluationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Package(error) => Some(error),
            Self::ReferenceEvidence { error, .. } => Some(error),
            Self::Cancelled | Self::TimedOut | Self::WorkerDisconnected | Self::Stale => None,
        }
    }
}
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
            return Err(ExactEvaluationError::Cancelled);
        }
        match self.receiver.recv_timeout(timeout) {
            Ok(result) if !self.cancelled.load(Ordering::Acquire) => result,
            Ok(_) => Err(ExactEvaluationError::Cancelled),
            Err(RecvTimeoutError::Timeout) => {
                self.cancel();
                Err(ExactEvaluationError::TimedOut)
            }
            Err(RecvTimeoutError::Disconnected) => Err(ExactEvaluationError::WorkerDisconnected),
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
) -> Result<(ExactResultRegistry, ExactResultRegistry, EvaluationReport), ExactEvaluationError> {
    if task.cancelled.load(Ordering::Acquire) {
        return Err(ExactEvaluationError::Cancelled);
    }
    if products.source != task.source || products.source != exact_source(snapshot) {
        return Err(ExactEvaluationError::Stale);
    }
    let mut results = ExactResultRegistry::carried_forward(snapshot, render);
    let mut topology_results = ExactResultRegistry::carried_forward(snapshot, topology);
    for package in products.render_packages {
        results
            .insert_current(snapshot, package)
            .map_err(ExactEvaluationError::Package)?;
    }
    for package in products.topology_packages {
        topology_results
            .insert_current(snapshot, package)
            .map_err(ExactEvaluationError::Package)?;
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
) -> Result<EvaluationReport, ExactEvaluationError> {
    let snapshot = document.current();
    let (results, topology_results, report) =
        materialize_exact_products(&snapshot, render, topology, task, products)?;
    document.try_canonical_transaction(
        |document| {
            document
                .register_exact_references(
                    results.values().flat_map(|package| package.references()),
                )
                .map_err(
                    |(reference, error)| ExactEvaluationError::ReferenceEvidence {
                        reference: Some(format!(
                            "role={}, source={}, profile={}, producer={}",
                            reference.semantic_role,
                            reference.source_element_id,
                            reference.profile_feature_id.0,
                            reference.producer_feature_id.0
                        )),
                        error,
                    },
                )?;
            document
                .register_exact_reference_evidence(&results)
                .map_err(|error| ExactEvaluationError::ReferenceEvidence {
                    reference: None,
                    error,
                })
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
    use ketchup_geometry::sketch::{
        PrincipalPlane, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport, WorkplaneSupportHealth,
    };
    use ketchup_model::document::{
        CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
        FeatureKind, OccurrenceId, Transform,
    };
    use ketchup_model::exact_product::{ExactBodyPackage, ExactFaceRole, ExactResultRegistry};
    use ketchup_model::testing::box_package;

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
                    kind: FeatureKind::polygon(&[
                        [0.0, 0.0],
                        [20.0, 0.0],
                        [20.0, 10.0],
                        [0.0, 10.0],
                    ]),
                },
                CanonicalCommand::CreateFeature {
                    id: EXTRUSION,
                    definition_id: DEFINITION,
                    name: "Extrusion".into(),
                    kind: FeatureKind::extrusion(PROFILE, Dimension::from_decimal("5").unwrap()),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(20),
                    definition_id: DEFINITION,
                    name: "Part occurrence".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tags: Default::default(),
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

    /// Showing or hiding a tag leaves every exact input as it was: evidence stays,
    /// graphs carry over, products are reused and republishing them changes nothing.
    #[test]
    fn tag_visibility_keeps_exact_state_without_republication() {
        use ketchup_model::document::TagId;
        use ketchup_model::exact_brep_graph::ExactBRepGraph;
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
                    kind: FeatureKind::polygon(&[
                        [0.0, 0.0],
                        [20.0, 0.0],
                        [20.0, 10.0],
                        [0.0, 10.0],
                    ]),
                },
                CanonicalCommand::CreateFeature {
                    id: EXTRUSION,
                    definition_id: DEFINITION,
                    name: "Extrusion".into(),
                    kind: FeatureKind::extrusion(PROFILE, Dimension::from_decimal("5").unwrap()),
                },
                CanonicalCommand::CreateTag {
                    id: TagId(1),
                    name: "koncept".into(),
                    visible: true,
                },
            ]))
            .unwrap();
        let before = document.current();
        let package = Arc::new(package(&before, "box", &[ExactFaceRole::Top]));
        document
            .register_exact_references(package.references())
            .unwrap();
        let render = ExactResultRegistry::accept(&document.current(), [package]).unwrap();
        let evidence = document.current().exact_reference_evidence().count();
        assert_eq!(evidence, 1);

        let revision = document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetTagVisibility {
                    id: TagId(1),
                    visible: false,
                },
            ]))
            .unwrap();
        let toggled = document.current();
        assert_eq!(toggled.exact_reference_evidence().count(), evidence);
        assert_eq!(
            *toggled.exact_brep_graph(DEFINITION, EXTRUSION).unwrap(),
            ExactBRepGraph::from_snapshot(&toggled, DEFINITION, EXTRUSION).unwrap()
        );
        let carried = ExactResultRegistry::carried_forward(&toggled, &render);
        let package = carried.values().next().unwrap();
        assert!(package.is_current(&toggled));
        let again = ExactResultRegistry::carried_forward(&toggled, &carried);
        assert!(Arc::ptr_eq(package, again.values().next().unwrap()));

        document
            .register_exact_references(package.references())
            .unwrap();
        document
            .register_exact_reference_evidence(&carried)
            .unwrap();
        assert!(std::ptr::eq(
            revision.as_ref(),
            document.revision_history().last().unwrap()
        ));
    }
}
