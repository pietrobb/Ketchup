use super::*;
use ketchup_tolerance::limits;

mod commands;
pub(super) mod feature_annotations;

use feature_annotations::forget_feature_annotations;

fn create_evaluator_input(
    product: &mut ProductModel,
    id: NodeId,
    name: &str,
    dimension: &Dimension,
    dependencies: &[NodeId],
) -> Result<(), CanonicalError> {
    if product.evaluator_nodes.contains_key(&id) {
        return Err(CanonicalError::NodeAlreadyExists(id));
    }
    for dependency in dependencies {
        if !product.evaluator_nodes.contains_key(dependency) {
            return Err(CanonicalError::MissingDependency(*dependency));
        }
    }
    let node = EvaluatorNode::parameter(
        id,
        name.to_owned(),
        dimension.clone(),
        dependencies.to_vec(),
    )
    .map_err(CanonicalError::Graph)?;
    product.evaluator_nodes.insert(id, Arc::new(node));
    Ok(())
}

/// The face references a feature is anchored to: the face a workplane lies on,
/// and the faces a pad's extent reaches and its cut starts from.
fn anchored_references(kind: &FeatureKind) -> Vec<&BodySubshapeRef> {
    match kind {
        FeatureKind::Workplane(WorkplaneSpec {
            support: WorkplaneSupport::PlanarFace { reference, .. },
            ..
        }) => vec![reference.as_ref()],
        FeatureKind::Pad(spec) => spec.references(),
        _ => Vec::new(),
    }
}

/// The lineages of every face reference some feature of `product` is anchored to.
fn anchored_lineages(product: &ProductModel) -> BTreeSet<&str> {
    product
        .features
        .values()
        .flat_map(|feature| anchored_references(&feature.kind))
        .map(|reference| reference.lineage_digest.as_str())
        .collect()
}

/// Marks stale the face references whose producer changed and drops evidence no
/// feature is anchored to any more. Tag visibility and saved views leave every
/// exact input as it was, so their evidence stays current.
fn refresh_reference_lineages(product: &mut ProductModel, current: &Snapshot, view_only: bool) {
    let mut stale_reference_lineages = BTreeSet::new();
    for feature in product.features.values() {
        for reference in anchored_references(&feature.kind) {
            let mut producer_dependencies = BTreeSet::new();
            add_feature_dependency_closure(
                current,
                reference.producer_feature_id,
                &mut producer_dependencies,
            );
            let producer_inputs_unchanged = producer_dependencies.iter().all(|dependency| {
                let AuthoritativeDependency::Feature(id) = dependency else {
                    return true;
                };
                current
                    .feature(*id)
                    .zip(product.features.get(id))
                    .is_some_and(|(before, after)| before.kind() == &after.kind)
            });
            if !producer_inputs_unchanged {
                stale_reference_lineages.insert(reference.lineage_digest.clone());
            }
        }
    }
    for lineage in &stale_reference_lineages {
        set_planar_face_reference_health(product, lineage, WorkplaneSupportHealth::Stale);
        product.exact_reference_evidence.remove(lineage);
    }
    if !view_only {
        let anchored = anchored_lineages(product)
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        product
            .exact_reference_evidence
            .retain(|lineage, _| anchored.contains(lineage));
    }
}

/// Stores the feature graph a revision was checked with; after a view-only change the
/// exact graphs of `previous` carry over instead of being compiled again.
/// The features a batch changed and the dependency graph of the new features. A
/// feature kept by the batch is the same `Arc`, so it is not compared by value, and
/// a batch that keeps every feature (a layer or colour change) reuses the graph.
fn feature_changes(
    current: &Snapshot,
    product: &ProductModel,
) -> Result<(BTreeSet<FeatureId>, Arc<FeatureDependencyGraph>), CanonicalError> {
    let (before, after) = (&current.product.features, &product.features);
    // A thread that turns modelled or cosmetic rebuilds the body it is cut into.
    let threads = threads::thread_changed_features(&current.product, product);
    if super::product_validation::same_entries(before, after) {
        return Ok((threads, current.feature_dependency_graph()?));
    }
    let mut changed: BTreeSet<_> = before
        .keys()
        .chain(after.keys())
        .filter(|id| match (before.get(id), after.get(id)) {
            (Some(before), Some(after)) => !Arc::ptr_eq(before, after) && before != after,
            (before, after) => before.is_some() != after.is_some(),
        })
        .copied()
        .collect();
    changed.extend(threads);
    Ok((
        changed,
        Arc::new(FeatureDependencyGraph::from_product(product)?),
    ))
}

fn keep_exact_caches(
    product: &mut ProductModel,
    previous: &ProductModel,
    feature_graph: Arc<FeatureDependencyGraph>,
    view_only: bool,
) {
    let _ = product.exact_graphs.dependencies.set(Ok(feature_graph));
    if view_only {
        product.exact_graphs.inherit_graphs(&previous.exact_graphs);
    }
}

pub struct DocumentStore {
    pub(super) revisions: Vec<Arc<Revision>>,
    pub(super) cursor: usize,
    pub(super) next_revision_id: u64,
    pub(super) mutation_epoch: u64,
    pub(super) evaluation_registry: BTreeMap<DerivedResultKey, DerivedResultEvent>,
    pub(super) human_confirmation_policy: Option<Box<HumanConfirmationPolicy>>,
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentStore {
    /// Applies a canonical mutation and keeps it only when its external finalizer succeeds.
    /// Rollback restores the complete history and process-local policy state, but deliberately
    /// leaves the mutation epoch advanced so in-flight derived work cannot publish stale results.
    pub fn try_canonical_transaction<T, E>(
        &mut self,
        mutate: impl FnOnce(&mut Self) -> Result<T, E>,
        finalize: impl FnOnce(&Self) -> Result<(), E>,
    ) -> Result<T, E> {
        let previous_revisions = self.revisions.clone();
        let previous_cursor = self.cursor;
        let previous_next_revision_id = self.next_revision_id;
        let previous_registry = self.evaluation_registry.clone();
        let previous_confirmation_policy = self.human_confirmation_policy.clone();
        let result = mutate(self).and_then(|value| finalize(self).map(|()| value));
        if result.is_err() {
            self.revisions = previous_revisions;
            self.cursor = previous_cursor;
            self.next_revision_id = previous_next_revision_id;
            self.evaluation_registry = previous_registry;
            self.human_confirmation_policy = previous_confirmation_policy;
        }
        result
    }

    // Process-local epoch is not snapshot/history state; rollback must never restore it.
    pub fn mutation_epoch(&self) -> u64 {
        self.mutation_epoch
    }

    #[must_use]
    pub const fn next_revision_id(&self) -> u64 {
        self.next_revision_id
    }

    pub(super) fn fresh_mutation_epoch() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("document mutation epoch exhausted")
    }
    #[must_use]
    pub fn new() -> Self {
        Self::from_product(0, ProductModel::default())
            .expect("an empty canonical document is valid")
    }

    pub fn configure_human_confirmation_policy(
        &mut self,
        verifying_key: [u8; 32],
        epoch: u64,
    ) -> Result<(), HumanConfirmationError> {
        if epoch == 0
            || self
                .human_confirmation_policy
                .as_ref()
                .is_some_and(|current| epoch <= current.epoch)
        {
            return Err(HumanConfirmationError::PolicyEpochInvalid);
        }
        let verifying_key = VerifyingKey::from_bytes(&verifying_key).map_err(
            |_: ed25519_dalek::SignatureError| HumanConfirmationError::InvalidVerifyingKey,
        )?;
        self.human_confirmation_policy = Some(Box::new(HumanConfirmationPolicy {
            verifying_key,
            epoch,
            consumed_signatures: BTreeSet::new(),
        }));
        Ok(())
    }

    pub(crate) fn from_product(
        revision_id: u64,
        product: ProductModel,
    ) -> Result<Self, CanonicalError> {
        validate_graph(&product.evaluator_nodes)?;
        validate_overrides(&product)?;
        validate_product(&product)?;
        validate_sketch_projections(&product)?;
        let next_revision_id = revision_id
            .checked_add(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let feature_states = FeatureDependencyGraph::from_product(&product)?
            .evaluation_states(&BTreeSet::new(), &BTreeSet::new());
        let snapshot = Snapshot {
            revision_id,
            product: Arc::new(product),
        };
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot,
            batch_digest: String::new(),
            origin: RevisionOrigin::Initial,
            checkpoint: None,
            rule_program: None,
            recomputed_nodes: BTreeSet::new(),
            dirty_features: BTreeSet::new(),
            feature_states,
            evaluation: None,
        });
        Ok(Self {
            revisions: vec![revision],
            cursor: 0,
            next_revision_id,
            mutation_epoch: Self::fresh_mutation_epoch(),
            evaluation_registry: BTreeMap::new(),
            human_confirmation_policy: None,
        })
    }

    /// The revision of a store `from_product` just built, as `from_revision_history`
    /// takes it.
    pub(crate) fn validated_current(&self) -> ValidatedSnapshot {
        let revision = &self.revisions[self.cursor];
        ValidatedSnapshot {
            snapshot: revision.snapshot.clone(),
            feature_states: revision.feature_states.clone(),
        }
    }

    #[must_use]
    pub fn current(&self) -> Snapshot {
        self.revisions[self.cursor].snapshot.clone()
    }

    /// An independent planning view sharing immutable history, without confirmation authority.
    #[must_use]
    pub fn fork_for_planning(&self) -> Self {
        Self {
            revisions: self.revisions.clone(),
            cursor: self.cursor,
            next_revision_id: self.next_revision_id,
            mutation_epoch: Self::fresh_mutation_epoch(),
            evaluation_registry: self.evaluation_registry.clone(),
            human_confirmation_policy: None,
        }
    }

    /// A store whose one revision is `snapshot`, sharing its product instead of
    /// copying or re-reading it: what an exact evaluation of a preview
    /// publishes into.
    pub fn from_snapshot(snapshot: &Snapshot) -> Result<Self, CanonicalError> {
        let next_revision_id = snapshot
            .revision_id
            .checked_add(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let feature_states = FeatureDependencyGraph::from_product(&snapshot.product)?
            .evaluation_states(&BTreeSet::new(), &BTreeSet::new());
        let revision = Arc::new(Revision {
            id: snapshot.revision_id,
            snapshot: snapshot.clone(),
            batch_digest: String::new(),
            origin: RevisionOrigin::Initial,
            checkpoint: None,
            rule_program: None,
            recomputed_nodes: BTreeSet::new(),
            dirty_features: BTreeSet::new(),
            feature_states,
            evaluation: None,
        });
        Ok(Self {
            revisions: vec![revision],
            cursor: 0,
            next_revision_id,
            mutation_epoch: Self::fresh_mutation_epoch(),
            evaluation_registry: BTreeMap::new(),
            human_confirmation_policy: None,
        })
    }

    #[must_use]
    pub fn current_rule_program(&self) -> Option<&RuleProgramSource> {
        self.revisions[self.cursor].rule_program()
    }

    /// The program an edit in the Undo history detached: `None` while a program
    /// owns the document or when none ever did. Undo back to it restores it.
    #[must_use]
    pub fn detached_rule_program(&self) -> Option<&RuleProgramSource> {
        if self.current_rule_program().is_some() {
            return None;
        }
        self.revisions[..self.cursor]
            .iter()
            .rev()
            .find_map(|revision| revision.rule_program())
    }

    fn validate_rule_program_source(source: &RuleProgramSource) -> Result<(), RuleProgramError> {
        source.validate()?;
        let bytes = serde_json::to_vec(source)
            .expect("strings and finite numbers always encode")
            .len();
        if bytes > MAX_RULE_PROGRAM_BYTES {
            return Err(RuleProgramError::TooLarge { bytes });
        }
        Ok(())
    }

    /// Makes `revision` current, discarding the Redo branch and keeping at most
    /// [`limits::UNDO_REVISIONS`] earlier revisions to step back to.
    fn push_revision(&mut self, revision: Arc<Revision>) {
        self.revisions.truncate(self.cursor + 1);
        self.revisions.push(revision);
        self.cursor += 1;
        let excess = self.cursor.saturating_sub(limits::UNDO_REVISIONS);
        if excess > 0 {
            self.revisions.drain(..excess);
            self.cursor -= excess;
        }
    }

    /// Gives the one revision of a store just opened without its history the
    /// program the file stores beside the document.
    pub(crate) fn restore_opened_rule_program(&mut self, source: Arc<RuleProgramSource>) {
        debug_assert_eq!(self.revisions.len(), 1);
        Arc::make_mut(&mut self.revisions[self.cursor]).rule_program =
            Some(BoundRuleProgram::new(source));
    }

    /// Associates the source with the current newly committed revision, within the same transaction.
    /// A later manual revision does not inherit it.
    pub fn bind_rule_program(&mut self, source: RuleProgramSource) -> Result<(), CanonicalError> {
        if self.cursor == 0 || self.cursor + 1 != self.revisions.len() {
            return Err(RuleProgramError::NoNewHeadRevision.into());
        }
        if self.revisions[self.cursor].rule_program.is_some() {
            return Err(RuleProgramError::AlreadyBound.into());
        }
        Self::validate_rule_program_source(&source)?;
        Arc::make_mut(&mut self.revisions[self.cursor]).rule_program =
            Some(BoundRuleProgram::new(Arc::new(source)));
        Ok(())
    }

    /// Publishes a source-only edit as a canonical revision without changing any geometry or IDs.
    /// Only an already source-owned document can be edited this way.
    pub fn replace_rule_program_source(
        &mut self,
        source: RuleProgramSource,
    ) -> Result<(), CanonicalError> {
        if self.current_rule_program().is_none() {
            return Err(RuleProgramError::NotProgramOwned.into());
        }
        Self::validate_rule_program_source(&source)?;
        if self.current_rule_program() == Some(&source) {
            return Ok(());
        }
        let revision_id = self.next_revision_id;
        let next_revision_id = revision_id
            .checked_add(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let current = &self.revisions[self.cursor];
        let program = BoundRuleProgram::new(Arc::new(source));
        let batch_digest = program.digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot: Snapshot {
                revision_id,
                product: Arc::clone(&current.snapshot.product),
            },
            batch_digest,
            origin: RevisionOrigin::Principal(ProposalPrincipal::ManualClient),
            checkpoint: None,
            rule_program: Some(program),
            recomputed_nodes: BTreeSet::new(),
            dirty_features: BTreeSet::new(),
            feature_states: current.feature_states.clone(),
            evaluation: current.evaluation.clone(),
        });
        self.push_revision(revision);
        self.next_revision_id = next_revision_id;
        self.mutation_epoch = Self::fresh_mutation_epoch();
        Ok(())
    }

    pub(crate) fn from_revision_history(
        revisions: Vec<StoredRevision>,
        cursor: usize,
        next_revision_id: u64,
    ) -> Self {
        // A file written before the shared Undo limit can hold more revisions than an
        // editing session keeps; restore only those within the limit around the cursor.
        let first = cursor.saturating_sub(limits::UNDO_REVISIONS);
        let cursor = cursor - first;
        let revisions = revisions
            .into_iter()
            .skip(first)
            .take(cursor + limits::UNDO_REVISIONS + 1);
        let mut restored = Vec::with_capacity(revisions.len());
        // Revisions that hold one program share its Arc; digest each program once.
        let mut bound = BTreeMap::<*const RuleProgramSource, BoundRuleProgram>::new();
        for (validated, batch_digest, origin, checkpoint, rule_program) in revisions {
            let rule_program = rule_program.map(|source| {
                bound
                    .entry(Arc::as_ptr(&source))
                    .or_insert_with(|| BoundRuleProgram::new(source))
                    .clone()
            });
            let ValidatedSnapshot {
                snapshot,
                feature_states,
            } = validated;
            restored.push(Arc::new(Revision {
                id: snapshot.revision_id(),
                snapshot,
                batch_digest,
                origin,
                checkpoint,
                rule_program,
                recomputed_nodes: BTreeSet::new(),
                dirty_features: BTreeSet::new(),
                feature_states,
                evaluation: None,
            }));
        }
        Self {
            revisions: restored,
            cursor,
            next_revision_id,
            mutation_epoch: Self::fresh_mutation_epoch(),
            evaluation_registry: BTreeMap::new(),
            human_confirmation_policy: None,
        }
    }

    pub fn register_exact_reference_evidence(
        &mut self,
        evidence: impl Into<ExactReferenceEvidence>,
    ) -> Result<(), ReferenceEvidenceError> {
        match evidence.into() {
            ExactReferenceEvidence::Reference(reference) => {
                let current = &self.revisions[self.cursor];
                if !reference.has_valid_lineage() {
                    return Err(ReferenceEvidenceError::InvalidLineage);
                }
                if reference.document_id != current.snapshot.document_id() {
                    return Err(ReferenceEvidenceError::WrongDocument);
                }
                let producer = current
                    .snapshot
                    .feature(reference.producer_feature_id)
                    .ok_or(ReferenceEvidenceError::ProducerNotFound)?;
                if producer.definition_id() != reference.definition_id {
                    return Err(ReferenceEvidenceError::ProducerDefinitionMismatch);
                }
                let context = ExactProducerEvidenceContext::from_snapshot(&current.snapshot);
                let matches_request = ExactProducerCompilation::from_snapshot(
                    &current.snapshot,
                    &context,
                    reference.definition_id,
                    reference.producer_feature_id,
                )
                .is_ok_and(|producer| producer.matches_reference(&reference));
                if !matches_request {
                    return Err(ReferenceEvidenceError::InvalidLineage);
                }
                let event = DerivedResultEvent {
                    document_id: current.snapshot.document_id(),
                    revision_id: current.snapshot.revision_id(),
                    canonical_digest: current.snapshot.canonical_digest(),
                    classification: DerivedResultClassification::Current,
                    payload: DerivedResultPayload::ExactReference(*reference),
                };
                if !self.register_derived_result(event) {
                    return Err(ReferenceEvidenceError::WrongDocument);
                }
            }
            ExactReferenceEvidence::Registry(results) => {
                let current = &self.revisions[self.cursor];
                let rebinds = current
                    .snapshot
                    .features()
                    .filter_map(|feature| match feature.kind() {
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::PlanarFace { reference, .. },
                            ..
                        }) => Some(ExactReferenceRebind {
                            lineage_digest: reference.lineage_digest.clone(),
                            resolution: results.resolve_reference(&current.snapshot, reference),
                        }),
                        _ => None,
                    })
                    .collect();
                let event = DerivedResultEvent {
                    document_id: current.snapshot.document_id(),
                    revision_id: current.snapshot.revision_id(),
                    canonical_digest: current.snapshot.canonical_digest(),
                    classification: DerivedResultClassification::Current,
                    payload: DerivedResultPayload::ExactReferenceRebinds(rebinds),
                };
                if !self.register_derived_result(event) {
                    return Err(ReferenceEvidenceError::InvalidLineage);
                }
            }
        }
        Ok(())
    }

    /// Registers many exact references as one change of the current revision, with
    /// the same checks as registering each alone.
    ///
    /// Registering them one by one cloned the whole product per reference, which
    /// dropped its cached digest and exact graphs; a 395-part model took ~6 s.
    pub fn register_exact_references<'r>(
        &mut self,
        references: impl IntoIterator<Item = &'r BodySubshapeRef>,
    ) -> Result<(), (&'r BodySubshapeRef, ReferenceEvidenceError)> {
        let current = Arc::clone(&self.revisions[self.cursor]);
        let snapshot = &current.snapshot;
        let context = ExactProducerEvidenceContext::from_snapshot(snapshot);
        let is_current = |reference: &BodySubshapeRef| {
            ExactProducerCompilation::from_snapshot(
                snapshot,
                &context,
                reference.definition_id,
                reference.producer_feature_id,
            )
            .is_ok_and(|producer| producer.matches_reference(reference))
        };
        let mut product: Option<ProductModel> = None;
        for reference in references {
            if !reference.has_valid_lineage() {
                return Err((reference, ReferenceEvidenceError::InvalidLineage));
            }
            if reference.document_id != snapshot.document_id() {
                return Err((reference, ReferenceEvidenceError::WrongDocument));
            }
            let producer = snapshot
                .feature(reference.producer_feature_id)
                .ok_or((reference, ReferenceEvidenceError::ProducerNotFound))?;
            if producer.definition_id() != reference.definition_id {
                return Err((
                    reference,
                    ReferenceEvidenceError::ProducerDefinitionMismatch,
                ));
            }
            if !is_current(reference) {
                return Err((reference, ReferenceEvidenceError::InvalidLineage));
            }
            // Evidence already held, with no feature anchored to it, would change nothing:
            // a carried-forward result must not copy the product and drop its caches.
            let held = product.as_ref().unwrap_or(snapshot.product.as_ref());
            if held
                .exact_reference_evidence
                .get(&reference.lineage_digest)
                .is_some_and(|evidence| evidence.as_ref() == reference)
                && !anchored_lineages(held).contains(reference.lineage_digest.as_str())
            {
                continue;
            }
            let product = product.get_or_insert_with(|| snapshot.product.as_ref().clone());
            let conflicts_with_anchor = product.features.values().any(|feature| {
                matches!(
                    &feature.kind,
                    FeatureKind::Workplane(WorkplaneSpec {
                        support: WorkplaneSupport::PlanarFace {
                            reference: support,
                            ..
                        },
                        ..
                    }) if support.lineage_digest == reference.lineage_digest
                        && (support.definition_id != reference.definition_id
                            || support.profile_feature_id != reference.profile_feature_id
                            || support.producer_feature_id != reference.producer_feature_id
                            || support.semantic_role != reference.semantic_role
                            || support.source_element_id != reference.source_element_id
                            || support.expected_type != reference.expected_type
                            || (is_current(support) && support.as_ref() != reference))
                )
            });
            if conflicts_with_anchor {
                return Err((reference, ReferenceEvidenceError::WrongDocument));
            }
            product.exact_reference_evidence.insert(
                reference.lineage_digest.clone(),
                Arc::new(reference.clone()),
            );
            if rebind_planar_face_reference(product, reference).is_err() {
                return Err((reference, ReferenceEvidenceError::WrongDocument));
            }
        }
        if let Some(mut product) = product {
            // Evidence feeds neither the exact graphs nor the dependency graph; while
            // no anchored feature was rebound they stay valid for the new product.
            let features_kept = product.features.len() == snapshot.product.features.len()
                && product.features.iter().zip(&snapshot.product.features).all(
                    |((id, after), (before_id, before))| {
                        id == before_id && Arc::ptr_eq(after, before)
                    },
                );
            if features_kept {
                product.exact_graphs = snapshot.product.exact_graphs.carried_over();
            }
            self.revisions[self.cursor] = Arc::new(Revision {
                id: current.id,
                snapshot: Snapshot {
                    revision_id: current.snapshot.revision_id,
                    product: Arc::new(product),
                },
                batch_digest: current.batch_digest.clone(),
                origin: current.origin,
                checkpoint: current.checkpoint.clone(),
                rule_program: current.rule_program.clone(),
                recomputed_nodes: current.recomputed_nodes.clone(),
                dirty_features: current.dirty_features.clone(),
                feature_states: current.feature_states.clone(),
                evaluation: current.evaluation.clone(),
            });
        }
        Ok(())
    }

    pub(super) fn register_derived_result(&mut self, event: DerivedResultEvent) -> bool {
        let current = &self.revisions[self.cursor];
        if event.document_id != current.snapshot.document_id()
            || event.revision_id != current.snapshot.revision_id()
            || event.canonical_digest != current.snapshot.canonical_digest()
        {
            return false;
        }
        match &event.payload {
            DerivedResultPayload::Evaluation(key)
                if key.document_id != event.document_id || key.revision_id != event.revision_id =>
            {
                return false;
            }
            DerivedResultPayload::ExactReference(reference)
                if reference.document_id != event.document_id =>
            {
                return false;
            }
            // Nothing to rebind leaves the revision, and its cached derivations, as is.
            DerivedResultPayload::ExactReferenceRebinds(rebinds) if rebinds.is_empty() => {
                return true;
            }
            _ => {}
        }
        let evidence_context = ExactProducerEvidenceContext::from_snapshot(&current.snapshot);
        let reference_is_current = |reference: &BodySubshapeRef| {
            ExactProducerCompilation::from_snapshot(
                &current.snapshot,
                &evidence_context,
                reference.definition_id,
                reference.producer_feature_id,
            )
            .is_ok_and(|producer| producer.matches_reference(reference))
        };
        match &event.payload {
            DerivedResultPayload::Evaluation(key) => {
                self.evaluation_registry.insert(key.clone(), event);
            }
            DerivedResultPayload::ExactReference(reference) => {
                let conflicts_with_anchor = current.snapshot.features().any(|feature| {
                    matches!(
                        feature.kind(),
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::PlanarFace {
                                reference: support,
                                ..
                            },
                            ..
                        }) if support.lineage_digest == reference.lineage_digest
                            && (support.definition_id != reference.definition_id
                                || support.profile_feature_id != reference.profile_feature_id
                                || support.producer_feature_id != reference.producer_feature_id
                                || support.semantic_role != reference.semantic_role
                                || support.source_element_id != reference.source_element_id
                                || support.expected_type != reference.expected_type
                                || (reference_is_current(support) && support.as_ref() != reference))
                    )
                });
                if conflicts_with_anchor {
                    return false;
                }
                let mut product = current.snapshot.product.as_ref().clone();
                product.exact_reference_evidence.insert(
                    reference.lineage_digest.clone(),
                    Arc::new(reference.clone()),
                );
                if rebind_planar_face_reference(&mut product, reference).is_err() {
                    return false;
                }
                self.revisions[self.cursor] = Arc::new(Revision {
                    id: current.id,
                    snapshot: Snapshot {
                        revision_id: current.snapshot.revision_id,
                        product: Arc::new(product),
                    },
                    batch_digest: current.batch_digest.clone(),
                    origin: current.origin,
                    checkpoint: current.checkpoint.clone(),
                    rule_program: current.rule_program.clone(),
                    recomputed_nodes: current.recomputed_nodes.clone(),
                    dirty_features: current.dirty_features.clone(),
                    feature_states: current.feature_states.clone(),
                    evaluation: current.evaluation.clone(),
                });
            }
            DerivedResultPayload::ExactReferenceRebinds(rebinds) => {
                let mut product = current.snapshot.product.as_ref().clone();
                for rebind in rebinds {
                    match &rebind.resolution {
                        ExactReferenceResolution::Resolved { reference } => {
                            if !reference_is_current(reference)
                                || reference.lineage_digest != rebind.lineage_digest
                            {
                                return false;
                            }
                            product.exact_reference_evidence.insert(
                                rebind.lineage_digest.clone(),
                                Arc::new(reference.as_ref().clone()),
                            );
                            if rebind_planar_face_reference(&mut product, reference).is_err() {
                                return false;
                            }
                        }
                        ExactReferenceResolution::Ambiguous { .. } => {
                            product
                                .exact_reference_evidence
                                .remove(&rebind.lineage_digest);
                            set_planar_face_reference_health(
                                &mut product,
                                &rebind.lineage_digest,
                                WorkplaneSupportHealth::Ambiguous,
                            );
                        }
                        ExactReferenceResolution::Lost => {
                            product
                                .exact_reference_evidence
                                .remove(&rebind.lineage_digest);
                            set_planar_face_reference_health(
                                &mut product,
                                &rebind.lineage_digest,
                                WorkplaneSupportHealth::Lost,
                            );
                        }
                        ExactReferenceResolution::Quarantined { .. } => {
                            product
                                .exact_reference_evidence
                                .remove(&rebind.lineage_digest);
                            set_planar_face_reference_health(
                                &mut product,
                                &rebind.lineage_digest,
                                WorkplaneSupportHealth::Stale,
                            );
                        }
                    }
                }
                if validate_product(&product).is_err() {
                    return false;
                }
                self.revisions[self.cursor] = Arc::new(Revision {
                    id: current.id,
                    snapshot: Snapshot {
                        revision_id: current.snapshot.revision_id,
                        product: Arc::new(product),
                    },
                    batch_digest: current.batch_digest.clone(),
                    origin: current.origin,
                    checkpoint: current.checkpoint.clone(),
                    rule_program: current.rule_program.clone(),
                    recomputed_nodes: current.recomputed_nodes.clone(),
                    dirty_features: current.dirty_features.clone(),
                    feature_states: current.feature_states.clone(),
                    evaluation: current.evaluation.clone(),
                });
            }
        }
        true
    }

    #[must_use]
    pub fn revision_count(&self) -> usize {
        self.revisions.len()
    }

    pub fn revision_history(&self) -> impl ExactSizeIterator<Item = &Revision> {
        self.revisions.iter().map(Arc::as_ref)
    }

    #[must_use]
    pub const fn history_cursor(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn history_digest(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"ketchup.revision-history.v2");
        digest.update((self.cursor as u64).to_le_bytes());
        for revision in self.revisions.iter().take(self.cursor + 1) {
            digest.update(revision.id.to_le_bytes());
            digest.update(revision.snapshot.canonical_digest().as_bytes());
            digest.update(revision.batch_digest.as_bytes());
            if let Some(program) = revision.rule_program.as_ref() {
                digest.update([1]);
                digest.update(program.digest);
            }
            match revision.origin {
                RevisionOrigin::Initial => digest.update([0]),
                RevisionOrigin::Principal(principal) => {
                    digest.update([1]);
                    update_revision_principal_digest(&mut digest, principal);
                }
                RevisionOrigin::Rollback {
                    principal,
                    target_revision,
                } => {
                    digest.update([2]);
                    update_revision_principal_digest(&mut digest, principal);
                    digest.update(target_revision.to_le_bytes());
                }
            }
            if let Some(checkpoint) = revision.checkpoint.as_ref() {
                digest.update([1]);
                digest.update((checkpoint.len() as u64).to_le_bytes());
                digest.update(checkpoint.as_bytes());
            } else {
                digest.update([0]);
            }
        }
        format!("{:x}", digest.finalize())[..16].to_owned()
    }

    #[must_use]
    pub fn revision_catalog(&self) -> Vec<RevisionCatalogEntry> {
        self.revisions
            .iter()
            .enumerate()
            .map(|(index, revision)| RevisionCatalogEntry {
                revision_id: revision.id,
                canonical_digest: revision.snapshot.canonical_digest(),
                batch_digest: revision.batch_digest.clone(),
                origin: revision.origin,
                checkpoint: revision.checkpoint.clone(),
                current: index == self.cursor,
            })
            .collect()
    }

    pub fn create_checkpoint(
        &mut self,
        expected_revision: u64,
        expected_digest: &str,
        name: &str,
    ) -> Result<(), RevisionHistoryError> {
        let current = &self.revisions[self.cursor];
        if current.id != expected_revision || current.snapshot.canonical_digest() != expected_digest
        {
            return Err(RevisionHistoryError::Stale);
        }
        if name.is_empty()
            || name.len() > 80
            || name.trim() != name
            || name.chars().any(char::is_control)
        {
            return Err(RevisionHistoryError::InvalidCheckpointName);
        }
        if self.revisions.iter().any(|revision| {
            revision.checkpoint.as_deref() == Some(name) && revision.id != expected_revision
        }) {
            return Err(RevisionHistoryError::DuplicateCheckpointName);
        }
        let mut replacement = current.as_ref().clone();
        replacement.checkpoint = Some(name.to_owned());
        self.revisions[self.cursor] = Arc::new(replacement);
        self.mutation_epoch = Self::fresh_mutation_epoch();
        Ok(())
    }

    pub fn compare_revisions(
        &self,
        before_revision: u64,
        after_revision: u64,
    ) -> Result<RevisionDiff, RevisionHistoryError> {
        let before = self
            .revisions
            .iter()
            .find(|revision| revision.id == before_revision)
            .ok_or(RevisionHistoryError::RevisionNotFound(before_revision))?;
        let after = self
            .revisions
            .iter()
            .find(|revision| revision.id == after_revision)
            .ok_or(RevisionHistoryError::RevisionNotFound(after_revision))?;
        let before_digest = before.snapshot.canonical_digest();
        let after_digest = after.snapshot.canonical_digest();
        let mut changes = Vec::new();
        let counts = [
            (
                "definitions",
                before.snapshot.product.definitions.len(),
                after.snapshot.product.definitions.len(),
            ),
            (
                "features",
                before.snapshot.product.features.len(),
                after.snapshot.product.features.len(),
            ),
            (
                "occurrences",
                before.snapshot.product.occurrences.len(),
                after.snapshot.product.occurrences.len(),
            ),
            (
                "groups",
                before.snapshot.product.groups.len(),
                after.snapshot.product.groups.len(),
            ),
            (
                "tags",
                before.snapshot.product.tags.len(),
                after.snapshot.product.tags.len(),
            ),
            (
                "collections",
                before.snapshot.product.collections.len(),
                after.snapshot.product.collections.len(),
            ),
            (
                "assembly-mates",
                before.snapshot.product.assembly_mates.len(),
                after.snapshot.product.assembly_mates.len(),
            ),
            (
                "assembly-joints",
                before.snapshot.product.assembly_joints.len(),
                after.snapshot.product.assembly_joints.len(),
            ),
            (
                "drawing-sheets",
                before.snapshot.product.drawing_sheets.len(),
                after.snapshot.product.drawing_sheets.len(),
            ),
            (
                "imports",
                before.snapshot.product.import_receipts.len(),
                after.snapshot.product.import_receipts.len(),
            ),
        ];
        changes.extend(
            counts
                .into_iter()
                .filter(|(_, before_count, after_count)| before_count != after_count)
                .map(|(structure, before_count, after_count)| RevisionDiffEntry {
                    structure,
                    before_count,
                    after_count,
                }),
        );
        if before_digest != after_digest {
            changes.push(RevisionDiffEntry {
                structure: "canonical-document",
                before_count: 1,
                after_count: 1,
            });
        }
        Ok(RevisionDiff {
            before_revision,
            after_revision,
            before_digest,
            after_digest,
            changes,
        })
    }

    pub fn rollback_to_revision(
        &mut self,
        expected_revision: u64,
        expected_digest: &str,
        target_revision: u64,
        principal: ProposalPrincipal,
    ) -> Result<Arc<Revision>, RevisionHistoryError> {
        let current = &self.revisions[self.cursor];
        if current.id != expected_revision || current.snapshot.canonical_digest() != expected_digest
        {
            return Err(RevisionHistoryError::Stale);
        }
        if matches!(
            principal,
            ProposalPrincipal::Human(0) | ProposalPrincipal::Plugin(0)
        ) {
            return Err(RevisionHistoryError::InvalidPrincipal);
        }
        let target = self
            .revisions
            .iter()
            .find(|revision| revision.id == target_revision)
            .cloned()
            .ok_or(RevisionHistoryError::RevisionNotFound(target_revision))?;
        if target.snapshot.canonical_digest() == current.snapshot.canonical_digest()
            && target.rule_program == current.rule_program
        {
            return Err(RevisionHistoryError::NoOpRollback);
        }
        let revision_id = self.next_revision_id;
        let following_revision_id = revision_id
            .checked_add(1)
            .ok_or(RevisionHistoryError::RevisionExhausted)?;
        let evidence = format!(
            "ketchup.rollback.v1:{target_revision}:{}",
            target.snapshot.canonical_digest()
        );
        let operation_digest = format!("{:x}", Sha256::digest(evidence.as_bytes()));
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot: Snapshot {
                revision_id,
                product: Arc::clone(&target.snapshot.product),
            },
            batch_digest: operation_digest[..16].to_owned(),
            origin: RevisionOrigin::Rollback {
                principal,
                target_revision,
            },
            checkpoint: None,
            rule_program: target.rule_program.clone(),
            recomputed_nodes: BTreeSet::new(),
            dirty_features: target.snapshot.product.features.keys().copied().collect(),
            feature_states: target.feature_states.clone(),
            evaluation: None,
        });
        self.push_revision(Arc::clone(&revision));
        self.next_revision_id = following_revision_id;
        self.mutation_epoch = Self::fresh_mutation_epoch();
        self.evaluation_registry.clear();
        Ok(revision)
    }

    #[must_use]
    pub const fn visible_undo_steps(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn visible_redo_steps(&self) -> usize {
        self.revisions.len() - self.cursor - 1
    }

    pub fn discard_history_before_current(&mut self) {
        let current = Arc::clone(&self.revisions[self.cursor]);
        self.revisions.clear();
        self.revisions.push(current);
        self.cursor = 0;
    }

    pub fn validate_batch(&self, batch: &CommandBatch) -> Result<(), CanonicalError> {
        self.preview_batch(batch).map(|_| ())
    }

    pub fn preview_batch(&self, batch: &CommandBatch) -> Result<Snapshot, CanonicalError> {
        let snapshot = self.current();
        let mut candidate =
            Self::from_product(snapshot.revision_id(), snapshot.product.as_ref().clone())?;
        candidate.apply_batch(batch)?;
        Ok(candidate.current())
    }

    pub fn preview_dependency_staging_batch(
        &self,
        batch: &CommandBatch,
    ) -> Result<Snapshot, CanonicalError> {
        let snapshot = self.current();
        let mut candidate =
            Self::from_product(snapshot.revision_id(), snapshot.product.as_ref().clone())?;
        candidate.apply_batch_with_origin_and_validation(
            batch,
            RevisionOrigin::Principal(ProposalPrincipal::ManualClient),
            false,
        )?;
        Ok(candidate.current())
    }

    pub fn apply_batch(&mut self, batch: &CommandBatch) -> Result<Arc<Revision>, CanonicalError> {
        self.apply_batch_with_origin(
            batch,
            RevisionOrigin::Principal(ProposalPrincipal::ManualClient),
        )
    }

    pub(super) fn apply_batch_with_origin(
        &mut self,
        batch: &CommandBatch,
        origin: RevisionOrigin,
    ) -> Result<Arc<Revision>, CanonicalError> {
        self.apply_batch_with_origin_and_validation(batch, origin, true)
    }

    /// Showing or hiding a tag or a part, keeping a saved view or switching
    /// the document's threads between modelled and cosmetic keeps the program
    /// owning the model; any other edit detaches it. A program's own bolts
    /// following its mode are its own edit, which binds its source anew.
    fn rule_program_kept_by(&self, batch: &CommandBatch) -> Option<BoundRuleProgram> {
        let switches_mode = batch
            .commands
            .iter()
            .any(|command| matches!(command, CanonicalCommand::SetModeledThreads { .. }));
        batch
            .commands
            .iter()
            .all(|command| command.is_view_only() || (switches_mode && command.switches_threads()))
            .then(|| self.revisions[self.cursor].rule_program.clone())
            .flatten()
    }

    pub(super) fn apply_batch_with_origin_and_validation(
        &mut self,
        batch: &CommandBatch,
        origin: RevisionOrigin,
        validate_drawing_sources: bool,
    ) -> Result<Arc<Revision>, CanonicalError> {
        if batch.schema != COMMAND_SCHEMA_V1 {
            return Err(CanonicalError::UnsupportedCommandSchema);
        }
        if batch.commands.is_empty() {
            return Err(CanonicalError::EmptyCommandBatch);
        }

        let current = self.current();
        let view_only = batch.commands.iter().all(CanonicalCommand::is_view_only);
        // Evidence no feature is anchored to yet stays through the commands: a
        // feature this batch creates may anchor it. What is still unanchored after
        // them is dropped by `refresh_reference_lineages`.
        let product = current.product.as_ref().clone();
        let production_anchors = product
            .production_codes
            .keys()
            .map(|path| (path.clone(), production_code_identity(&product, path)))
            .collect();
        let mut edit = commands::BatchEdit {
            previous_evaluation: self.revisions[self.cursor].evaluation.clone(),
            current,
            product,
            explicit_dirty_features: BTreeSet::new(),
            evaluation_identity: EvaluationIdentity::default(),
            production_anchors,
            rebound_production_paths: BTreeSet::new(),
        };
        for command in &batch.commands {
            edit.apply(command)?;
        }
        let commands::BatchEdit {
            current,
            mut product,
            explicit_dirty_features,
            evaluation_identity,
            previous_evaluation,
            rebound_production_paths,
            ..
        } = edit;
        if let Some(path) = rebound_production_paths.first() {
            return Err(CanonicalError::InvalidProductionCodePath(path.clone()));
        }

        refresh_supported_planar_face_frames(&mut product, Some(&current))?;
        refresh_reference_lineages(&mut product, &current, view_only);

        support::prune_grounded_instances(&mut product);
        contact_joint::prune(&mut product);
        joint::prune(&mut product);
        refresh_sketch_projections(&mut product)?;
        validate_graph(&product.evaluator_nodes)?;
        refresh_override_health(&mut product);
        validate_overrides(&product)?;
        // Showing or hiding parts changes nothing the checks of the model read.
        if view_only {
            validate_saved_views(&product)?;
        } else {
            validate_product_change(Some(&current.product), &product, validate_drawing_sources)?;
        }
        validate_sketch_projections(&product)?;
        validate_assembly_joint_motion_publication(&current, &product, batch)?;
        let revision_id = self.next_revision_id;
        let following_revision_id = revision_id
            .checked_add(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let mut evaluator_roots = BTreeSet::new();
        for command in &batch.commands {
            match command {
                CanonicalCommand::CreateEvaluatorNode { id, .. }
                | CanonicalCommand::SetEvaluatorDimension { id, .. }
                | CanonicalCommand::RenameEvaluatorNode { id, .. }
                | CanonicalCommand::CreateExpressionNode { id, .. }
                | CanonicalCommand::CreateRuleNode { id, .. }
                | CanonicalCommand::SetNodeExpression { id, .. }
                | CanonicalCommand::SetRuleOutputs { id, .. } => {
                    evaluator_roots.insert(*id);
                }
                CanonicalCommand::RecomputeFeatureParameters { scope, .. } => match scope {
                    FeatureParameterRecomputeScope::All => evaluator_roots.extend(
                        product
                            .feature_parameter_bindings
                            .values()
                            .map(|binding| binding.derived_from.root_rule_node_id),
                    ),
                    FeatureParameterRecomputeScope::AffectedBy(nodes) => {
                        evaluator_roots.extend(nodes);
                    }
                },
                _ => {}
            }
        }
        let recomputed_nodes = dependent_closure(&product.evaluator_nodes, &evaluator_roots);
        let evaluation = evaluate_affected(
            &product.evaluator_nodes,
            &evaluation_identity,
            previous_evaluation.as_ref(),
            &recomputed_nodes,
        )
        .map_err(CanonicalError::Graph)?;
        let (changed_features, feature_graph) = feature_changes(&current, &product)?;
        let dirty_features = feature_graph.dependent_closure(
            changed_features
                .iter()
                .chain(&explicit_dirty_features)
                .cloned()
                .filter(|id| product.features.contains_key(id)),
        );
        let feature_states = feature_graph.evaluation_states(&dirty_features, &BTreeSet::new());
        let rule_program = self.rule_program_kept_by(batch);
        keep_exact_caches(&mut product, &current.product, feature_graph, view_only);
        let snapshot = Snapshot {
            revision_id,
            product: Arc::new(product),
        };
        let revision = Arc::new(Revision {
            id: revision_id,
            snapshot,
            batch_digest: batch.digest(),
            origin,
            checkpoint: None,
            rule_program,
            recomputed_nodes,
            dirty_features,
            feature_states,
            evaluation: Some(evaluation),
        });

        self.mutation_epoch = Self::fresh_mutation_epoch();
        self.push_revision(Arc::clone(&revision));
        self.next_revision_id = following_revision_id;
        Ok(revision)
    }

    pub fn make_unique(
        &mut self,
        occurrence_id: OccurrenceId,
        new_definition_name: impl Into<String>,
    ) -> Result<Arc<Revision>, CanonicalError> {
        let snapshot = self.current();
        let occurrence = snapshot
            .occurrence(occurrence_id)
            .ok_or(CanonicalError::OccurrenceNotFound(occurrence_id))?;
        let source = snapshot
            .definition(occurrence.definition_id)
            .ok_or(CanonicalError::DefinitionNotFound(occurrence.definition_id))?;
        let new_definition_id =
            DefinitionId(next_id(snapshot.definitions().map(|item| item.id.0))?);
        let mut next_feature_id = next_id(snapshot.features().map(|item| item.id.0))?;
        let feature_id_map = source
            .feature_ids
            .iter()
            .map(|source_id| {
                let mapped = FeatureId(next_feature_id);
                next_feature_id += 1;
                (*source_id, mapped)
            })
            .collect();
        let plan = CloneDefinitionPlan {
            occurrence_id,
            source_definition_id: occurrence.definition_id,
            new_definition_id,
            new_definition_name: new_definition_name.into(),
            feature_id_map,
        };
        self.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CloneDefinitionAndRepoint(plan),
        ]))
    }

    pub fn convert_group_to_component(
        &mut self,
        group_id: GroupId,
        component_name: impl Into<String>,
    ) -> Result<ConvertGroupToComponentResult, CanonicalError> {
        let snapshot = self.current();
        if snapshot.group(group_id).is_none() {
            return Err(CanonicalError::GroupNotFound(group_id));
        }
        let plan = ConvertGroupPlan {
            group_id,
            new_definition_id: DefinitionId(next_id(snapshot.definitions().map(|item| item.id.0))?),
            new_occurrence_id: OccurrenceId(next_id(snapshot.occurrences().map(|item| item.id.0))?),
            component_name: component_name.into(),
        };
        let mappings = conversion_mappings(snapshot.product(), &plan)?;
        let component_definition_id = plan.new_definition_id;
        let component_occurrence_id = plan.new_occurrence_id;
        let revision = self.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::ConvertGroupToComponent(plan),
        ]))?;
        Ok(ConvertGroupToComponentResult {
            revision,
            component_definition_id,
            component_occurrence_id,
            mappings,
        })
    }

    pub fn register_evaluation(
        &mut self,
        root_rule_node_id: NodeId,
        slot_path: SlotPath,
        report: &EvaluationReport,
    ) -> Result<DerivedResultEvent, CanonicalError> {
        let snapshot = self.current();
        let Some(root_node) = snapshot.evaluator_node(root_rule_node_id) else {
            return Err(CanonicalError::NodeNotFound(root_rule_node_id));
        };
        if !matches!(root_node.kind(), EvaluatorNodeKind::Rule { .. }) {
            return Err(CanonicalError::WrongNodeKind(root_rule_node_id));
        }
        let target = DerivedIdentity::new(root_rule_node_id, slot_path.clone())
            .map_err(CanonicalError::from)?;
        if resolve_derived_identity(&snapshot.product.evaluator_nodes, &target)
            != SlotResolution::Resolved
        {
            return Err(CanonicalError::UnresolvedDerivedOutput);
        }
        if report.document_id != Some(snapshot.document_id())
            || report.revision_id != Some(snapshot.revision_id())
            || report.canonical_digest.as_deref() != Some(snapshot.canonical_digest().as_str())
        {
            return Err(CanonicalError::EvaluationEnvelopeMismatch);
        }
        let expected = snapshot.evaluate(&report.identity)?;
        let supplied_node = report
            .node(root_rule_node_id)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        let expected_node = expected
            .node(root_rule_node_id)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        if supplied_node != expected_node {
            return Err(CanonicalError::EvaluationEvidenceMismatch);
        }
        if !matches!(supplied_node.status, EvaluationStatus::Evaluated(_)) {
            return Err(CanonicalError::FailedEvaluation(root_rule_node_id));
        }
        let output = report
            .outputs
            .get(&target)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        let expected_output = expected
            .outputs
            .get(&target)
            .ok_or(CanonicalError::EvaluationEvidenceMismatch)?;
        if output != expected_output {
            return Err(CanonicalError::EvaluationEvidenceMismatch);
        }
        let key = DerivedResultKey {
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            root_rule_node_id,
            slot_path,
            input_digest: output.input_digest.clone(),
            result_digest: output.result_digest.clone(),
            evaluator: report.identity.evaluator.clone(),
            backend: report.identity.backend.clone(),
            schema: report.identity.schema.clone(),
            tolerance: report.identity.tolerance.clone(),
        };
        let event = DerivedResultEvent {
            document_id: snapshot.document_id(),
            revision_id: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            classification: DerivedResultClassification::Current,
            payload: DerivedResultPayload::Evaluation(key),
        };
        if !self.register_derived_result(event.clone()) {
            return Err(CanonicalError::EvaluationEnvelopeMismatch);
        }
        Ok(event)
    }

    #[must_use]
    pub fn evaluation_registry_len(&self) -> usize {
        self.evaluation_registry.len()
    }

    pub fn undo(&mut self) -> Option<Snapshot> {
        if self.cursor == 0 {
            return None;
        }
        self.mutation_epoch = Self::fresh_mutation_epoch();
        self.cursor -= 1;
        Some(self.current())
    }

    pub fn redo(&mut self) -> Option<Snapshot> {
        if self.cursor + 1 >= self.revisions.len() {
            return None;
        }
        self.mutation_epoch = Self::fresh_mutation_epoch();
        self.cursor += 1;
        Some(self.current())
    }

    /// Captures the immediate parent of the current last tip for correction planning.
    pub fn tip_replacement_parent(
        &self,
    ) -> Result<TipReplacementParent, TipReplacementProposalError> {
        if self.cursor == 0 {
            return Err(TipReplacementProposalError::NoCurrentTip);
        }
        if self.cursor + 1 != self.revisions.len() {
            return Err(TipReplacementProposalError::RedoBranch);
        }
        let superseded = &self.revisions[self.cursor].snapshot;
        let corrected_revision =
            superseded
                .revision_id
                .checked_add(1)
                .ok_or(TipReplacementProposalError::Canonical(
                    CanonicalError::RevisionExhausted,
                ))?;
        if self.next_revision_id != corrected_revision {
            return Err(TipReplacementProposalError::Stale);
        }
        let parent = self.revisions[self.cursor - 1].snapshot.clone();
        Ok(TipReplacementParent {
            snapshot: parent.clone(),
            document_id: superseded.document_id(),
            parent_revision: parent.revision_id(),
            parent_digest: parent.canonical_digest(),
            superseded_revision: superseded.revision_id(),
            superseded_digest: superseded.canonical_digest(),
        })
    }

    /// Prepares a correction against a previously guarded immediate-parent snapshot.
    pub fn prepare_tip_replacement_proposal(
        &self,
        parent: &TipReplacementParent,
        batch: CommandBatch,
        context: ProposalContext,
    ) -> Result<TipReplacementProposal, TipReplacementProposalError> {
        validate_confirmation_requirement(&context)
            .map_err(TipReplacementProposalError::Preparation)?;
        let current_parent = self.validate_tip_replacement_envelope(
            parent.document_id,
            parent.parent_revision,
            &parent.parent_digest,
            parent.superseded_revision,
            &parent.superseded_digest,
            parent.superseded_revision.checked_add(1).ok_or(
                TipReplacementProposalError::Canonical(CanonicalError::RevisionExhausted),
            )?,
        )?;
        if parent.snapshot.document_id() != parent.document_id
            || parent.snapshot.revision_id() != parent.parent_revision
            || parent.snapshot.canonical_digest() != parent.parent_digest
            || parent.snapshot.canonical_digest() != current_parent.canonical_digest()
        {
            return Err(TipReplacementProposalError::Stale);
        }

        let authoritative_dependencies = authoritative_dependencies(&current_parent, &batch);
        let authoritative_writes = authoritative_writes(&current_parent, &batch);
        let cost = ProposalCost {
            commands: batch.commands.len(),
            read_dependencies: authoritative_dependencies.len(),
            write_targets: authoritative_writes.len(),
        };
        validate_proposal_budget(context.requested_budget, cost)
            .map_err(TipReplacementProposalError::Preparation)?;
        let corrected_revision = parent.superseded_revision + 1;
        let (_, authoritative_diff, intended_result_digest) = tip_replacement_candidate(
            &current_parent,
            corrected_revision,
            &batch,
            &authoritative_writes,
            context.goal.clone(),
        )
        .map_err(TipReplacementProposalError::Preparation)?;
        Ok(TipReplacementProposal {
            document_id: parent.document_id,
            parent_revision: parent.parent_revision,
            parent_digest: parent.parent_digest.clone(),
            superseded_revision: parent.superseded_revision,
            superseded_digest: parent.superseded_digest.clone(),
            corrected_revision,
            command_digest: batch.digest(),
            dependency_digest: dependency_digest(&current_parent, &authoritative_dependencies),
            authoritative_dependencies,
            authoritative_writes,
            authoritative_diff,
            intended_result_digest,
            principal: context.principal,
            goal: context.goal,
            assumptions: context.assumptions,
            risk: context.risk,
            confirmation: context.confirmation,
            requested_budget: context.requested_budget,
            cost,
            batch,
        })
    }

    /// Replays and verifies a correction without changing document state.
    pub fn preview_tip_replacement_proposal(
        &self,
        proposal: &TipReplacementProposal,
    ) -> Result<Snapshot, TipReplacementProposalError> {
        self.verify_tip_replacement_proposal(proposal)
    }

    /// Atomically replaces the exact current last tip with its verified correction.
    pub fn commit_tip_replacement_proposal(
        &mut self,
        proposal: &TipReplacementProposal,
    ) -> Result<Arc<Revision>, TipReplacementProposalError> {
        if matches!(proposal.risk, ProposalRisk::High(_)) {
            return Err(TipReplacementProposalError::HumanApprovalRequired);
        }
        let expected = self.verify_tip_replacement_proposal(proposal)?;
        let parent = self.revisions[self.cursor - 1].snapshot.clone();
        let previous_revisions = self.revisions.clone();
        let previous_cursor = self.cursor;
        let previous_next_revision_id = self.next_revision_id;
        let previous_registry = self.evaluation_registry.clone();

        self.revisions.pop();
        self.cursor -= 1;
        let result = self
            .apply_batch(&proposal.batch)
            .map_err(TipReplacementProposalError::Canonical)
            .and_then(|revision| {
                let actual_diff: Vec<_> = proposal
                    .authoritative_writes
                    .iter()
                    .cloned()
                    .map(|target| ProposalDiffEntry {
                        target: target.clone(),
                        before: proposal_value(&parent, target.clone(), proposal.goal.clone()),
                        after: proposal_value(revision.snapshot(), target, proposal.goal.clone()),
                    })
                    .collect();
                let actual_result_digest =
                    dependency_digest(revision.snapshot(), &proposal.authoritative_writes);
                if revision.id() != proposal.corrected_revision
                    || revision.batch_digest() != proposal.command_digest
                    || actual_diff != proposal.authoritative_diff
                    || actual_result_digest != proposal.intended_result_digest
                    || revision.snapshot().canonical_digest() != expected.canonical_digest()
                {
                    return Err(TipReplacementProposalError::VerificationMismatch);
                }
                Ok(revision)
            });

        match result {
            Ok(revision) => {
                self.evaluation_registry
                    .retain(|key, _| key.revision_id != proposal.superseded_revision);
                Ok(revision)
            }
            Err(error) => {
                self.revisions = previous_revisions;
                self.cursor = previous_cursor;
                self.next_revision_id = previous_next_revision_id;
                self.evaluation_registry = previous_registry;
                Err(error)
            }
        }
    }

    pub(super) fn validate_tip_replacement_envelope(
        &self,
        document_id: DocumentId,
        parent_revision: u64,
        parent_digest: &str,
        superseded_revision: u64,
        superseded_digest: &str,
        corrected_revision: u64,
    ) -> Result<Snapshot, TipReplacementProposalError> {
        if self.cursor == 0 {
            return Err(TipReplacementProposalError::NoCurrentTip);
        }
        if self.cursor + 1 != self.revisions.len() {
            return Err(TipReplacementProposalError::RedoBranch);
        }
        let current = &self.revisions[self.cursor].snapshot;
        let parent = &self.revisions[self.cursor - 1].snapshot;
        if current.document_id() != document_id
            || current.revision_id() != superseded_revision
            || current.canonical_digest() != superseded_digest
            || parent.document_id() != document_id
            || parent.revision_id() != parent_revision
            || parent.canonical_digest() != parent_digest
            || superseded_revision.checked_add(1) != Some(corrected_revision)
            || self.next_revision_id != corrected_revision
        {
            return Err(TipReplacementProposalError::Stale);
        }
        Ok(parent.clone())
    }

    pub(super) fn verify_tip_replacement_proposal(
        &self,
        proposal: &TipReplacementProposal,
    ) -> Result<Snapshot, TipReplacementProposalError> {
        let parent = self.validate_tip_replacement_envelope(
            proposal.document_id,
            proposal.parent_revision,
            &proposal.parent_digest,
            proposal.superseded_revision,
            &proposal.superseded_digest,
            proposal.corrected_revision,
        )?;
        if proposal.command_digest != proposal.batch.digest() {
            return Err(TipReplacementProposalError::VerificationMismatch);
        }
        let authoritative_dependencies = authoritative_dependencies(&parent, &proposal.batch);
        let authoritative_writes = authoritative_writes(&parent, &proposal.batch);
        let cost = ProposalCost {
            commands: proposal.batch.commands.len(),
            read_dependencies: authoritative_dependencies.len(),
            write_targets: authoritative_writes.len(),
        };
        if authoritative_dependencies != proposal.authoritative_dependencies
            || authoritative_writes != proposal.authoritative_writes
            || dependency_digest(&parent, &authoritative_dependencies) != proposal.dependency_digest
            || cost != proposal.cost
        {
            return Err(TipReplacementProposalError::VerificationMismatch);
        }
        validate_proposal_budget(proposal.requested_budget, cost)
            .map_err(TipReplacementProposalError::Preparation)?;
        validate_confirmation_requirement(&ProposalContext {
            principal: proposal.principal,
            goal: proposal.goal.clone(),
            assumptions: proposal.assumptions.clone(),
            risk: proposal.risk,
            confirmation: proposal.confirmation.clone(),
            requested_budget: proposal.requested_budget,
        })
        .map_err(TipReplacementProposalError::Preparation)?;
        let (preview, authoritative_diff, intended_result_digest) = tip_replacement_candidate(
            &parent,
            proposal.corrected_revision,
            &proposal.batch,
            &authoritative_writes,
            proposal.goal.clone(),
        )
        .map_err(TipReplacementProposalError::Preparation)?;
        if authoritative_diff != proposal.authoritative_diff
            || intended_result_digest != proposal.intended_result_digest
        {
            return Err(TipReplacementProposalError::VerificationMismatch);
        }
        Ok(preview)
    }

    pub fn plan_pad(
        &self,
        id: FeatureId,
        definition_id: DefinitionId,
        name: impl Into<String>,
        spec: PadSpec,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        self.prepare_proposal_with_context(
            CommandBatch::new(vec![CanonicalCommand::CreateFeature {
                id,
                definition_id,
                name: name.into(),
                kind: FeatureKind::Pad(spec),
            }]),
            context,
        )
    }

    pub fn plan_new_body_feature(
        &self,
        plan: NewBodyFeaturePlan,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        if !matches!(
            plan.feature_kind,
            FeatureKind::Pad(PadSpec {
                operation: PadOperation::NewBody,
                ..
            })
        ) {
            return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
        }
        let snapshot = self.current();
        for dependency in plan.feature_kind.authoritative_dependencies() {
            let feature = snapshot
                .feature(dependency)
                .ok_or(CanonicalError::FeatureNotFound(dependency))?;
            if feature.definition_id() != plan.definition_id
                || !feature_references_are_resolved(
                    snapshot.product(),
                    dependency,
                    &mut BTreeSet::new(),
                )
            {
                return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
            }
        }
        self.prepare_proposal_with_context(
            CommandBatch::new(vec![
                CanonicalCommand::CreateBody {
                    definition_id: plan.definition_id,
                    id: plan.body_id,
                    name: plan.body_name,
                    visible: true,
                },
                CanonicalCommand::SetActiveBody {
                    definition_id: plan.definition_id,
                    id: plan.body_id,
                },
                CanonicalCommand::CreateFeature {
                    id: plan.feature_id,
                    definition_id: plan.definition_id,
                    name: plan.feature_name,
                    kind: plan.feature_kind,
                },
            ]),
            context,
        )
    }

    pub fn plan_multibody_boolean(
        &self,
        plan: MultiBodyBooleanPlan,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        if plan.target_body_id == plan.tool_body_id
            || plan.target_feature_id == plan.tool_feature_id
            || plan.operation == BooleanOperation::Split
        {
            return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
        }
        let snapshot = self.current();
        let definition = snapshot
            .definition(plan.definition_id)
            .ok_or(CanonicalError::DefinitionNotFound(plan.definition_id))?;
        for body_id in [plan.target_body_id, plan.tool_body_id] {
            let body = definition
                .body(body_id)
                .ok_or(CanonicalError::BodyNotFound(plan.definition_id, body_id))?;
            if body.consumed_by().is_some() {
                return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
            }
        }
        for (feature_id, body_id) in [
            (plan.target_feature_id, plan.target_body_id),
            (plan.tool_feature_id, plan.tool_body_id),
        ] {
            let feature = snapshot
                .feature(feature_id)
                .ok_or(CanonicalError::FeatureNotFound(feature_id))?;
            if feature.definition_id() != plan.definition_id
                || !matches!(
                    feature.kind(),
                    FeatureKind::Pad(PadSpec {
                        operation: PadOperation::NewBody,
                        ..
                    })
                )
                || definition
                    .feature_body_ownership(feature_id)
                    .and_then(FeatureBodyOwnership::output_body_id)
                    != Some(body_id)
                || !feature_references_are_resolved(
                    snapshot.product(),
                    feature_id,
                    &mut BTreeSet::new(),
                )
                || definition
                    .feature_ids()
                    .iter()
                    .cloned()
                    .any(|dependent_id| {
                        dependent_id != feature_id
                            && snapshot.feature(dependent_id).is_some_and(|dependent| {
                                dependent.kind().dependencies().contains(&feature_id)
                                    && definition
                                        .feature_body_ownership(dependent_id)
                                        .and_then(FeatureBodyOwnership::output_body_id)
                                        == Some(body_id)
                            })
                    })
            {
                return Err(CanonicalError::InvalidBodyAuthoringPlan.into());
            }
        }
        let mut commands = vec![
            CanonicalCommand::SetActiveBody {
                definition_id: plan.definition_id,
                id: plan.target_body_id,
            },
            CanonicalCommand::CreateFeature {
                id: plan.result_feature_id,
                definition_id: plan.definition_id,
                name: plan.result_feature_name,
                kind: FeatureKind::Boolean {
                    operation: plan.operation,
                    target: plan.target_feature_id,
                    tool: plan.tool_feature_id,
                },
            },
        ];
        if plan.tool_policy == ToolBodyPolicy::Consume {
            commands.push(CanonicalCommand::ConsumeBody {
                definition_id: plan.definition_id,
                id: plan.tool_body_id,
                by_feature_id: plan.result_feature_id,
            });
        }
        self.prepare_proposal_with_context(CommandBatch::new(commands), context)
    }

    pub fn plan_body_command(
        &self,
        command: CanonicalCommand,
    ) -> Result<Proposal, ProposalPrepareError> {
        if !matches!(
            command,
            CanonicalCommand::CreateBody { .. }
                | CanonicalCommand::DeleteBody { .. }
                | CanonicalCommand::RenameBody { .. }
                | CanonicalCommand::SetActiveBody { .. }
                | CanonicalCommand::SetBodyVisibility { .. }
                | CanonicalCommand::ConsumeBody { .. }
                | CanonicalCommand::SetFeatureBodyOwnership { .. }
        ) {
            return Err(ProposalPrepareError::Canonical(
                CanonicalError::InvalidBodyCommand,
            ));
        }
        self.prepare_proposal(CommandBatch::new(vec![command]))
    }

    pub fn prepare_proposal(&self, batch: CommandBatch) -> Result<Proposal, ProposalPrepareError> {
        self.prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
    }

    pub fn prepare_proposal_with_context(
        &self,
        batch: CommandBatch,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        self.prepare_proposal_with_context_and_validation(batch, context, true)
    }

    pub(crate) fn prepare_dependency_staging_proposal_with_context(
        &self,
        batch: CommandBatch,
        context: ProposalContext,
    ) -> Result<Proposal, ProposalPrepareError> {
        self.prepare_proposal_with_context_and_validation(batch, context, false)
    }

    pub(super) fn prepare_proposal_with_context_and_validation(
        &self,
        batch: CommandBatch,
        context: ProposalContext,
        validate_drawing_sources: bool,
    ) -> Result<Proposal, ProposalPrepareError> {
        validate_confirmation_requirement(&context)?;
        let snapshot = self.current();
        let authoritative_dependencies = authoritative_dependencies(&snapshot, &batch);
        let authoritative_writes = authoritative_writes(&snapshot, &batch);
        let cost = ProposalCost {
            commands: batch.commands.len(),
            read_dependencies: authoritative_dependencies.len(),
            write_targets: authoritative_writes.len(),
        };
        validate_proposal_budget(context.requested_budget, cost)?;
        let (authoritative_diff, intended_result_digest) = proposal_candidate_with_validation(
            &snapshot,
            &batch,
            &authoritative_writes,
            context.goal.clone(),
            validate_drawing_sources,
        )?;
        Ok(Proposal {
            document_id: snapshot.document_id(),
            provenance_revision: snapshot.revision_id,
            provenance_digest: snapshot.canonical_digest(),
            command_digest: batch.digest(),
            dependency_digest: dependency_digest(&snapshot, &authoritative_dependencies),
            authoritative_dependencies,
            authoritative_writes,
            authoritative_diff,
            intended_result_digest,
            principal: context.principal,
            goal: context.goal,
            assumptions: context.assumptions,
            risk: context.risk,
            confirmation: context.confirmation,
            requested_budget: context.requested_budget,
            cost,
            batch,
        })
    }

    pub fn prepare_high_risk_side_effect(
        &self,
        operation: &str,
        principal: ProposalPrincipal,
        scope: HighRiskScope,
        payload: &[u8],
    ) -> Result<SideEffectProposal, HumanConfirmationError> {
        if operation.is_empty()
            || operation.len() > 128
            || operation.chars().any(char::is_control)
            || payload.is_empty()
        {
            return Err(HumanConfirmationError::InvalidSideEffectEvidence);
        }
        validate_high_risk_requester(principal)?;
        let snapshot = self.current();
        let payload_digest = format!("{:x}", Sha256::digest(payload));
        let mut evidence = Vec::new();
        push_confirmation_field(&mut evidence, b"ketchup.side-effect-proposal.v1");
        push_confirmation_field(&mut evidence, operation.as_bytes());
        push_confirmation_u64(&mut evidence, snapshot.document_id().0);
        push_confirmation_u64(&mut evidence, snapshot.revision_id);
        push_confirmation_field(&mut evidence, snapshot.canonical_digest().as_bytes());
        push_confirmation_principal(&mut evidence, principal);
        push_confirmation_scope(&mut evidence, &scope);
        push_confirmation_field(&mut evidence, payload_digest.as_bytes());
        let operation_digest = format!("{:x}", Sha256::digest(&evidence));
        Ok(SideEffectProposal {
            document_id: snapshot.document_id(),
            provenance_revision: snapshot.revision_id,
            provenance_digest: snapshot.canonical_digest(),
            operation: operation.to_owned(),
            operation_digest,
            payload_digest,
            principal,
            scope,
        })
    }

    #[must_use]
    pub fn validate_proposal(&self, proposal: &Proposal) -> ProposalValidity {
        let snapshot = self.current();
        let envelope_matches = proposal.document_id == snapshot.document_id();
        let command_matches = proposal.command_digest == proposal.batch.digest();
        let dependencies_match = proposal.dependency_digest
            == dependency_digest(&snapshot, &proposal.authoritative_dependencies);
        if envelope_matches && command_matches && dependencies_match {
            ProposalValidity::Valid {
                evaluated_revision: snapshot.revision_id,
            }
        } else {
            ProposalValidity::Stale {
                provenance_revision: proposal.provenance_revision,
                current_revision: snapshot.revision_id,
            }
        }
    }

    /// Replays and verifies a standard-risk proposal in an isolated store without
    /// changing canonical history, the mutation epoch, or Undo/Redo state.
    pub fn preview_verified_proposal(
        &self,
        proposal: &Proposal,
    ) -> Result<Snapshot, ProposalCommitError> {
        if matches!(proposal.risk, ProposalRisk::High(_)) {
            return Err(ProposalCommitError::HumanApprovalRequired);
        }
        self.verify_proposal_candidate(proposal)
    }

    pub fn commit_proposal(
        &mut self,
        proposal: &Proposal,
    ) -> Result<Arc<Revision>, ProposalCommitError> {
        self.commit_verified_proposal(proposal)
            .map(|committed| committed.revision)
    }

    pub fn commit_verified_proposal(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, ProposalCommitError> {
        if matches!(proposal.risk, ProposalRisk::High(_)) {
            return Err(ProposalCommitError::HumanApprovalRequired);
        }
        self.commit_verified_proposal_inner(proposal)
    }

    pub fn commit_high_risk_proposal(
        &mut self,
        proposal: &Proposal,
        approval: &HumanApprovalToken,
        now_ms: u64,
    ) -> Result<VerifiedProposalCommit, ProposalCommitError> {
        let ProposalRisk::High(risk_class) = proposal.risk else {
            return Err(ProposalCommitError::HumanApprovalUnexpected);
        };
        let ProposalConfirmation::HumanOnly(scope) = &proposal.confirmation else {
            return Err(ProposalCommitError::HumanApprovalInvalid);
        };
        let signature = approval.signature;
        {
            let policy = self
                .human_confirmation_policy
                .as_ref()
                .ok_or(ProposalCommitError::HumanApprovalPolicyUnavailable)?;
            if policy.epoch != approval.policy_epoch {
                return Err(ProposalCommitError::HumanApprovalPolicyStale);
            }
            if policy.consumed_signatures.contains(&signature) {
                return Err(ProposalCommitError::HumanApprovalReplayed);
            }
            if approval.approving_human == 0
                || approval.requester != proposal.principal
                || approval.document_id != proposal.document_id
                || approval.revision_id != proposal.provenance_revision
                || approval.provenance_digest != proposal.provenance_digest
                || approval.dependency_digest != proposal.dependency_digest
                || approval.command_digest != proposal.command_digest
                || approval.result_digest != proposal.intended_result_digest
                || approval.scope != *scope
                || approval.scope.class != risk_class
                || now_ms < approval.issued_at_ms
                || now_ms > approval.expires_at_ms
            {
                return Err(ProposalCommitError::HumanApprovalInvalid);
            }
            policy
                .verifying_key
                .verify(
                    &approval.signing_payload(),
                    &Signature::from_bytes(&approval.signature),
                )
                .map_err(|_: ed25519_dalek::SignatureError| {
                    ProposalCommitError::HumanApprovalInvalid
                })?;
        }
        let committed = self.commit_verified_proposal_inner(proposal)?;
        self.human_confirmation_policy
            .as_mut()
            .expect("confirmation policy was verified before commit")
            .consumed_signatures
            .insert(signature);
        Ok(committed)
    }

    pub fn authorize_high_risk_side_effect(
        &mut self,
        proposal: &SideEffectProposal,
        approval: &SideEffectApprovalToken,
        now_ms: u64,
    ) -> Result<SideEffectAuthorizationReceipt, SideEffectAuthorizationError> {
        let signature = approval.signature;
        let snapshot = self.current();
        {
            let policy = self
                .human_confirmation_policy
                .as_ref()
                .ok_or(SideEffectAuthorizationError::PolicyUnavailable)?;
            if policy.epoch != approval.policy_epoch {
                return Err(SideEffectAuthorizationError::PolicyStale);
            }
            if policy.consumed_signatures.contains(&signature) {
                return Err(SideEffectAuthorizationError::Replayed);
            }
            if approval.approving_human == 0
                || approval.requester != proposal.principal
                || approval.document_id != proposal.document_id
                || approval.revision_id != proposal.provenance_revision
                || approval.provenance_digest != proposal.provenance_digest
                || approval.operation_digest != proposal.operation_digest
                || approval.payload_digest != proposal.payload_digest
                || approval.scope != proposal.scope
                || snapshot.document_id() != proposal.document_id
                || snapshot.revision_id != proposal.provenance_revision
                || snapshot.canonical_digest() != proposal.provenance_digest
                || now_ms < approval.issued_at_ms
                || now_ms > approval.expires_at_ms
            {
                return Err(SideEffectAuthorizationError::Invalid);
            }
            policy
                .verifying_key
                .verify(
                    &approval.signing_payload(),
                    &Signature::from_bytes(&approval.signature),
                )
                .map_err(|_: ed25519_dalek::SignatureError| {
                    SideEffectAuthorizationError::Invalid
                })?;
        }
        self.human_confirmation_policy
            .as_mut()
            .expect("side-effect policy was verified before authorization")
            .consumed_signatures
            .insert(signature);
        Ok(SideEffectAuthorizationReceipt {
            approving_human: approval.approving_human,
            document_id: proposal.document_id,
            revision_id: proposal.provenance_revision,
            operation: proposal.operation.clone(),
            operation_digest: proposal.operation_digest.clone(),
            payload_digest: proposal.payload_digest.clone(),
            scope: proposal.scope.clone(),
            policy_epoch: approval.policy_epoch,
            authorized_at_ms: now_ms,
        })
    }

    pub(super) fn verify_proposal_candidate(
        &self,
        proposal: &Proposal,
    ) -> Result<Snapshot, ProposalCommitError> {
        if let stale @ ProposalValidity::Stale { .. } = self.validate_proposal(proposal) {
            return Err(ProposalCommitError::Stale(stale));
        }
        let current = self.current();
        let (candidate, diff, intended_result_digest) = proposal_candidate_snapshot(
            &current,
            &proposal.batch,
            &proposal.authoritative_writes,
            proposal.goal.clone(),
        )
        .map_err(ProposalCommitError::Preparation)?;
        if diff != proposal.authoritative_diff
            || intended_result_digest != proposal.intended_result_digest
        {
            return Err(ProposalCommitError::VerificationMismatch);
        }
        Ok(candidate)
    }

    pub(super) fn commit_verified_proposal_inner(
        &mut self,
        proposal: &Proposal,
    ) -> Result<VerifiedProposalCommit, ProposalCommitError> {
        self.verify_proposal_candidate(proposal)?;
        let previous_revisions = self.revisions.clone();
        let previous_cursor = self.cursor;
        let previous_next_revision_id = self.next_revision_id;
        let previous_registry = self.evaluation_registry.clone();
        let revision = self
            .apply_batch_with_origin(
                &proposal.batch,
                RevisionOrigin::Principal(proposal.principal),
            )
            .map_err(ProposalCommitError::Canonical)?;
        let actual_result_digest =
            dependency_digest(revision.snapshot(), &proposal.authoritative_writes);
        if revision.batch_digest() != proposal.command_digest
            || actual_result_digest != proposal.intended_result_digest
        {
            self.revisions = previous_revisions;
            self.cursor = previous_cursor;
            self.next_revision_id = previous_next_revision_id;
            self.evaluation_registry = previous_registry;
            return Err(ProposalCommitError::VerificationMismatch);
        }
        Ok(VerifiedProposalCommit {
            revision,
            command_digest: proposal.command_digest.clone(),
            result_digest: actual_result_digest,
            verified_writes: proposal.authoritative_writes.clone(),
        })
    }
}
