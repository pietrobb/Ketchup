use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedInstance {
    pub definition_id: DefinitionId,
    pub local_transform: Transform,
    pub parent_world_transform: Transform,
    pub world_transform: Transform,
}

/// Why the categories of one named classification dimension cannot be read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClassificationError {
    /// A required dimension with this name does not exist.
    DimensionMissing { name: String },
    /// More than one dimension has this name.
    DimensionAmbiguous { name: String },
    /// An occurrence is assigned a category its dimension does not define.
    CategoryMissing {
        dimension_id: ClassificationDimensionId,
        category_id: ClassificationCategoryId,
    },
}

impl fmt::Display for ClassificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionMissing { name } => {
                write!(formatter, "classification dimension {name:?} is missing")
            }
            Self::DimensionAmbiguous { name } => write!(
                formatter,
                "classification dimension {name:?} is ambiguous: more than one dimension has this name"
            ),
            Self::CategoryMissing {
                dimension_id,
                category_id,
            } => write!(
                formatter,
                "classification category {} is missing from dimension {}",
                category_id.0, dimension_id.0
            ),
        }
    }
}

impl std::error::Error for ClassificationError {}

#[derive(Clone)]
pub struct Snapshot {
    pub(super) revision_id: u64,
    pub(super) product: Arc<ProductModel>,
}

impl Snapshot {
    #[must_use]
    pub fn production_code(&self, path: &InstancePath) -> Option<&str> {
        self.product.production_codes.get(path).map(String::as_str)
    }

    pub fn production_codes(&self) -> impl Iterator<Item = (&InstancePath, &str)> {
        self.product
            .production_codes
            .iter()
            .map(|(path, code)| (path, code.as_str()))
    }

    pub fn preview_batch(&self, batch: &CommandBatch) -> Result<Self, CanonicalError> {
        self.preview_batch_at_revision(
            batch,
            self.revision_id
                .checked_add(1)
                .ok_or(CanonicalError::RevisionExhausted)?,
        )
    }

    pub fn preview_batch_at_revision(
        &self,
        batch: &CommandBatch,
        revision_id: u64,
    ) -> Result<Self, CanonicalError> {
        let base_revision = revision_id
            .checked_sub(1)
            .ok_or(CanonicalError::RevisionExhausted)?;
        let mut candidate =
            DocumentStore::from_product(base_revision, self.product.as_ref().clone())?;
        candidate.apply_batch(batch)?;
        Ok(candidate.current())
    }

    #[must_use]
    pub const fn revision_id(&self) -> u64 {
        self.revision_id
    }

    #[must_use]
    pub fn evaluator_node(&self, id: NodeId) -> Option<&EvaluatorNode> {
        self.product.evaluator_nodes.get(&id).map(Arc::as_ref)
    }

    pub fn evaluator_nodes(&self) -> impl Iterator<Item = &EvaluatorNode> {
        self.product.evaluator_nodes.values().map(Arc::as_ref)
    }

    pub fn evaluator_node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.product.evaluator_nodes.keys().cloned()
    }

    #[must_use]
    pub fn evaluator_node_count(&self) -> usize {
        self.product.evaluator_nodes.len()
    }

    #[must_use]
    pub fn shares_evaluator_node_with(&self, other: &Self, id: NodeId) -> bool {
        match (
            self.product.evaluator_nodes.get(&id),
            other.product.evaluator_nodes.get(&id),
        ) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub fn overrides(&self) -> impl Iterator<Item = &CanonicalOverride> {
        self.product.overrides.values().map(Arc::as_ref)
    }

    pub fn feature_parameter_bindings(&self) -> impl Iterator<Item = &FeatureParameterBinding> {
        self.product
            .feature_parameter_bindings
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn feature_parameter_binding(
        &self,
        target: &FeatureParameterTarget,
    ) -> Option<&FeatureParameterBinding> {
        self.product
            .feature_parameter_bindings
            .get(target)
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn has_feature_parameter(&self, target: &FeatureParameterTarget) -> bool {
        feature_parameter_dimension(&self.product, target).is_some()
    }

    #[must_use]
    pub fn feature_parameter_value(&self, target: &FeatureParameterTarget) -> Option<f64> {
        feature_parameter_dimension(&self.product, target).map(|value| value.millimetres())
    }

    #[must_use]
    pub fn feature_parameter_provenance(
        &self,
        target: &FeatureParameterTarget,
    ) -> Option<&FeatureParameterProvenance> {
        self.product
            .feature_parameter_provenance
            .get(target)
            .map(Arc::as_ref)
    }

    pub fn audit_feature_parameter_freshness(
        &self,
        identity: &EvaluationIdentity,
    ) -> Result<Vec<FeatureParameterFreshnessAudit>, CanonicalError> {
        let report = evaluate_graph(&self.product.evaluator_nodes, identity)
            .map_err(CanonicalError::Graph)?;
        self.product
            .feature_parameter_bindings
            .values()
            .map(|binding| {
                let freshness =
                    audit_feature_parameter_binding(&self.product, binding, identity, &report);
                Ok(FeatureParameterFreshnessAudit {
                    target: binding.target.clone(),
                    freshness,
                })
            })
            .collect()
    }

    #[must_use]
    pub fn override_by_id(&self, id: u64) -> Option<&CanonicalOverride> {
        self.product.overrides.get(&id).map(Arc::as_ref)
    }

    pub fn joints(&self) -> impl Iterator<Item = &CanonicalJoint> {
        self.product.joints.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn joint(&self, id: JointId) -> Option<&CanonicalJoint> {
        self.product.joints.get(&id).map(Arc::as_ref)
    }

    pub fn spaces(&self) -> impl Iterator<Item = &CanonicalSpace> {
        self.product.spaces.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn space(&self, id: SpaceId) -> Option<&CanonicalSpace> {
        self.product.spaces.get(&id).map(Arc::as_ref)
    }

    pub fn clearance_volumes(&self) -> impl Iterator<Item = &CanonicalClearanceVolume> {
        self.product.clearance_volumes.values().map(Arc::as_ref)
    }

    pub fn cam_plans(&self) -> impl Iterator<Item = &CamPlan> {
        self.product.cam_plans.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn cam_plan(&self, id: CamPlanId) -> Option<&CamPlan> {
        self.product.cam_plans.get(&id).map(Arc::as_ref)
    }

    pub fn pin_joints(&self) -> impl Iterator<Item = &PinJointContract> {
        self.product.pin_joints.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn pin_joint(&self, id: PinJointId) -> Option<&PinJointContract> {
        self.product.pin_joints.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_recipe(&self) -> Option<&AssemblyRecipe> {
        self.product.assembly_recipe.as_deref()
    }

    #[must_use]
    pub fn feature_canonical_fingerprint(&self, id: FeatureId) -> Option<String> {
        self.feature(id).map(digest_feature)
    }

    #[must_use]
    pub fn clearance_volume(&self, id: ClearanceVolumeId) -> Option<&CanonicalClearanceVolume> {
        self.product.clearance_volumes.get(&id).map(Arc::as_ref)
    }

    pub fn exact_reference_evidence(&self) -> impl Iterator<Item = &BodySubshapeRef> {
        self.product
            .exact_reference_evidence
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn exact_reference_by_lineage(&self, lineage_digest: &str) -> Option<&BodySubshapeRef> {
        self.product
            .exact_reference_evidence
            .get(lineage_digest)
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn resolved_planar_face_workplane_frame(
        &self,
        reference: &BodySubshapeRef,
    ) -> Option<WorkplaneFrame> {
        self.exact_reference_by_lineage(&reference.lineage_digest)
            .filter(|evidence| *evidence == reference)?;
        supported_planar_face_frame(&self.product, reference)
    }

    pub fn persistent_dimensions(&self) -> impl Iterator<Item = &PersistentDimension> {
        self.product.persistent_dimensions.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn persistent_dimension(&self, id: PersistentDimensionId) -> Option<&PersistentDimension> {
        self.product.persistent_dimensions.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn project_persistent_dimension(
        &self,
        id: PersistentDimensionId,
    ) -> Option<PersistentDimensionProjection> {
        let dimension = self.persistent_dimension(id)?;
        let (health, millimetres) = resolve_persistent_dimension(self.product(), dimension);
        let display_value =
            millimetres.map(|value| dimension.presentation.unit.from_millimetres(value));
        let display_text = display_value.map(|value| {
            format!(
                "{value:.precision$} {}",
                dimension.presentation.unit.label(),
                precision = usize::from(dimension.presentation.decimal_places)
            )
        });
        Some(PersistentDimensionProjection {
            id,
            health,
            millimetres,
            display_value,
            display_text,
        })
    }

    #[must_use]
    pub fn resolve_slot(&self, identity: &DerivedIdentity) -> SlotResolution {
        resolve_derived_identity(&self.product.evaluator_nodes, identity)
    }

    pub fn evaluate(
        &self,
        identity: &EvaluationIdentity,
    ) -> Result<EvaluationReport, CanonicalError> {
        let mut report = evaluate_graph(&self.product.evaluator_nodes, identity)
            .map_err(CanonicalError::Graph)?;
        report.document_id = Some(self.document_id());
        report.revision_id = Some(self.revision_id());
        report.canonical_digest = Some(self.canonical_digest());
        Ok(report)
    }

    /// The exact B-Rep graph of one body producer, compiled at most once per
    /// snapshot; `None` when the producer cannot be compiled.
    #[must_use]
    pub fn exact_brep_graph(
        &self,
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
    ) -> Option<Arc<ExactBRepGraph>> {
        let key = (self.revision_id, definition_id, producer_feature_id);
        if let Some(graph) = self.product.exact_graphs.graphs.lock().ok()?.get(&key) {
            return graph.clone();
        }
        let inherited = self
            .product
            .exact_graphs
            .inherited
            .lock()
            .ok()?
            .get(&(definition_id, producer_feature_id))
            .cloned();
        let graph = match inherited {
            Some(graph) => Some(Arc::new(graph.rebased_to(self))),
            None => ExactBRepGraph::from_snapshot(self, definition_id, producer_feature_id)
                .ok()
                .map(Arc::new),
        };
        self.product
            .exact_graphs
            .graphs
            .lock()
            .ok()?
            .insert(key, graph.clone());
        graph
    }

    #[must_use]
    pub fn canonical_digest(&self) -> String {
        self.product
            .canonical_digest
            .0
            .get_or_init(|| digest_snapshot(self))
            .clone()
    }

    /// Whether `value` is a digest as [`Self::canonical_digest`] writes it: one 64-bit
    /// hash as sixteen lower-case hex digits.
    #[must_use]
    pub fn is_canonical_digest(value: &str) -> bool {
        value.len() == 2 * size_of::<u64>()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    #[must_use]
    pub fn document_id(&self) -> DocumentId {
        self.product.document_id
    }

    /// The feature dependency graph, built at most once per snapshot.
    pub fn feature_dependency_graph(&self) -> Result<Arc<FeatureDependencyGraph>, CanonicalError> {
        self.product
            .exact_graphs
            .dependencies
            .get_or_init(|| FeatureDependencyGraph::from_product(&self.product).map(Arc::new))
            .clone()
    }

    pub fn solid_tool_feature_clone_count(
        &self,
        feature_id: FeatureId,
    ) -> Result<usize, CanonicalError> {
        exact_solid_tool_dependency_closure_for_snapshot(self, feature_id)
            .map(|closure| closure.len())
    }

    pub fn solid_tool_result_feature_count(
        &self,
        target_feature_id: FeatureId,
        tool_feature_id: FeatureId,
    ) -> Result<usize, CanonicalError> {
        solid_tool_result_feature_count(&self.product, target_feature_id, tool_feature_id)
    }

    #[must_use]
    pub fn units(&self) -> UnitSystem {
        self.product.units
    }

    /// The model tolerance every check of this document uses.
    #[must_use]
    pub fn tolerance(&self) -> TolerancePolicy {
        self.product.tolerance
    }

    #[must_use]
    pub fn tag(&self, id: TagId) -> Option<&Tag> {
        self.product.tags.get(&id).map(Arc::as_ref)
    }

    pub fn tags(&self) -> impl Iterator<Item = &Tag> {
        self.product.tags.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn saved_view(&self, id: SavedViewId) -> Option<&SavedView> {
        self.product.saved_views.get(&id).map(Arc::as_ref)
    }

    pub fn saved_views(&self) -> impl Iterator<Item = &SavedView> {
        self.product.saved_views.values().map(Arc::as_ref)
    }

    pub fn occurrences_with_tag(&self, id: TagId) -> impl Iterator<Item = &Occurrence> {
        self.product
            .occurrences
            .values()
            .filter(move |occurrence| occurrence.tags.contains(&id))
            .map(Arc::as_ref)
    }

    /// Whether a part with `tags` is shown: hiding any one of its tags hides it.
    #[must_use]
    pub fn tags_visible(&self, tags: &BTreeSet<TagId>) -> bool {
        all_tags_visible(&self.product.tags, tags)
    }

    #[must_use]
    pub fn classification_dimension(
        &self,
        id: ClassificationDimensionId,
    ) -> Option<&ClassificationDimension> {
        self.product
            .classification_dimensions
            .get(&id)
            .map(Arc::as_ref)
    }

    pub fn classification_dimensions(&self) -> impl Iterator<Item = &ClassificationDimension> {
        self.product
            .classification_dimensions
            .values()
            .map(Arc::as_ref)
    }

    /// The one dimension called `name`, or `None` when no dimension has that name.
    pub fn classification_dimension_named(
        &self,
        name: &str,
    ) -> Result<Option<&ClassificationDimension>, ClassificationError> {
        let mut named = self
            .classification_dimensions()
            .filter(|dimension| dimension.name() == name);
        match (named.next(), named.next()) {
            (dimension, None) => Ok(dimension),
            (_, Some(_)) => Err(ClassificationError::DimensionAmbiguous {
                name: name.to_owned(),
            }),
        }
    }

    /// Like [`Snapshot::classification_dimension_named`], but the dimension must exist.
    pub fn required_classification_dimension(
        &self,
        name: &str,
    ) -> Result<&ClassificationDimension, ClassificationError> {
        self.classification_dimension_named(name)?.ok_or_else(|| {
            ClassificationError::DimensionMissing {
                name: name.to_owned(),
            }
        })
    }

    /// The category name every classified occurrence takes in `dimension`.
    pub fn occurrence_category_names<'a>(
        &'a self,
        dimension: &'a ClassificationDimension,
    ) -> impl Iterator<
        Item = Result<(OccurrenceId, ClassificationCategoryId, &'a str), ClassificationError>,
    > + 'a {
        self.occurrences().filter_map(move |occurrence| {
            let category_id = self.occurrence_classification(occurrence.id(), dimension.id())?;
            Some(
                dimension
                    .category(category_id)
                    .map(|category| (occurrence.id(), category_id, category.name()))
                    .ok_or(ClassificationError::CategoryMissing {
                        dimension_id: dimension.id(),
                        category_id,
                    }),
            )
        })
    }

    #[must_use]
    pub fn occurrence_classification(
        &self,
        occurrence_id: OccurrenceId,
        dimension_id: ClassificationDimensionId,
    ) -> Option<ClassificationCategoryId> {
        self.product
            .classification_assignments
            .get(&(occurrence_id, dimension_id))
            .cloned()
    }

    pub fn occurrence_classifications(
        &self,
        occurrence_id: OccurrenceId,
    ) -> impl Iterator<Item = (ClassificationDimensionId, ClassificationCategoryId)> + '_ {
        self.product
            .classification_assignments
            .range(
                (occurrence_id, ClassificationDimensionId(0))
                    ..=(occurrence_id, ClassificationDimensionId(u64::MAX)),
            )
            .map(|((_, dimension_id), category_id)| (*dimension_id, *category_id))
    }

    #[must_use]
    pub fn collection(&self, id: CollectionId) -> Option<&Collection> {
        self.product.collections.get(&id).map(Arc::as_ref)
    }

    pub fn collections(&self) -> impl Iterator<Item = &Collection> {
        self.product.collections.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn import_receipt(&self, id: ImportId) -> Option<&ImportReceipt> {
        self.product.import_receipts.get(&id).map(Arc::as_ref)
    }

    pub fn import_receipts(&self) -> impl Iterator<Item = &ImportReceipt> {
        self.product.import_receipts.values().map(Arc::as_ref)
    }

    /// The next free import ID, or `None` when the import ID space is exhausted.
    #[must_use]
    pub fn next_import_id(&self) -> Option<ImportId> {
        let last = self.product.import_receipts.keys().map(|id| id.0).max();
        last.unwrap_or(0).checked_add(1).map(ImportId)
    }

    pub fn occurrences_in_collection(&self, id: CollectionId) -> impl Iterator<Item = &Occurrence> {
        self.product
            .collections
            .get(&id)
            .into_iter()
            .flat_map(|collection| collection.occurrence_ids.iter())
            .filter_map(|occurrence_id| self.product.occurrences.get(occurrence_id))
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn occurrence_effectively_visible(&self, id: OccurrenceId) -> Option<bool> {
        let occurrence = self.occurrence(id)?;
        Some(occurrence.visible && self.tags_visible(&occurrence.tags))
    }

    #[must_use]
    pub fn definition(&self, id: DefinitionId) -> Option<&Definition> {
        self.product.definitions.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.product.features.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn suppressed_feature_ids(
        &self,
        definition_id: DefinitionId,
        body_id: BodyId,
    ) -> Option<&BTreeSet<FeatureId>> {
        self.product
            .body_feature_suppression
            .get(&(definition_id, body_id))
    }

    #[must_use]
    pub fn feature_is_suppressed(&self, id: FeatureId) -> bool {
        self.product
            .body_feature_suppression
            .values()
            .any(|suppressed| suppressed.contains(&id))
    }

    #[must_use]
    pub fn occurrence(&self, id: OccurrenceId) -> Option<&Occurrence> {
        self.product.occurrences.get(&id).map(Arc::as_ref)
    }

    #[must_use]
    pub fn occurrence_is_grounded(&self, id: OccurrenceId) -> bool {
        self.product.grounded_occurrences.contains(&id)
    }

    pub fn grounded_occurrences(&self) -> impl Iterator<Item = OccurrenceId> + '_ {
        self.product.grounded_occurrences.iter().cloned()
    }

    #[must_use]
    pub fn assembly_mate(&self, id: AssemblyMateId) -> Option<&AssemblyMate> {
        self.product.assembly_mates.get(&id).map(Arc::as_ref)
    }

    pub fn assembly_mates(&self) -> impl Iterator<Item = &AssemblyMate> {
        self.product.assembly_mates.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_joint(&self, id: AssemblyJointId) -> Option<&AssemblyJoint> {
        self.product.assembly_joints.get(&id).map(Arc::as_ref)
    }

    pub fn assembly_joints(&self) -> impl Iterator<Item = &AssemblyJoint> {
        self.product.assembly_joints.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_motion_coupling(
        &self,
        id: AssemblyMotionCouplingId,
    ) -> Option<&AssemblyMotionCoupling> {
        self.product
            .assembly_motion_couplings
            .get(&id)
            .map(Arc::as_ref)
    }

    pub fn assembly_motion_couplings(&self) -> impl Iterator<Item = &AssemblyMotionCoupling> {
        self.product
            .assembly_motion_couplings
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn mechanical_interface(&self, id: MechanicalInterfaceId) -> Option<&MechanicalInterface> {
        self.product.mechanical_interfaces.get(&id).map(Arc::as_ref)
    }

    pub fn mechanical_interfaces(&self) -> impl Iterator<Item = &MechanicalInterface> {
        self.product.mechanical_interfaces.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn mechanical_condition(&self, id: MechanicalConditionId) -> Option<&MechanicalCondition> {
        self.product.mechanical_conditions.get(&id).map(Arc::as_ref)
    }

    pub fn mechanical_conditions(&self) -> impl Iterator<Item = &MechanicalCondition> {
        self.product.mechanical_conditions.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_motion_study(&self, id: AssemblyMotionStudyId) -> Option<&AssemblyMotionStudy> {
        self.product
            .assembly_motion_studies
            .get(&id)
            .map(Arc::as_ref)
    }

    pub fn assembly_motion_studies(&self) -> impl Iterator<Item = &AssemblyMotionStudy> {
        self.product
            .assembly_motion_studies
            .values()
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn drawing_sheet(&self, id: DrawingSheetId) -> Option<&DrawingSheet> {
        self.product.drawing_sheets.get(&id).map(Arc::as_ref)
    }

    pub fn drawing_sheets(&self) -> impl Iterator<Item = &DrawingSheet> {
        self.product.drawing_sheets.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn assembly_dof_diagnostic(&self, id: OccurrenceId) -> Option<AssemblyDofDiagnostic> {
        self.product.occurrences.contains_key(&id).then(|| {
            let grounded = self.occurrence_is_grounded(id);
            AssemblyDofDiagnostic {
                occurrence_id: id,
                status: if grounded {
                    AssemblyDofStatus::Grounded
                } else {
                    AssemblyDofStatus::PendingSolve
                },
                remaining_dof: grounded.then_some(0),
                incident_mate_ids: self
                    .product
                    .assembly_mates
                    .values()
                    .filter(|mate| {
                        mate.endpoint_a().occurrence_id() == id
                            || mate.endpoint_b().occurrence_id() == id
                    })
                    .map(|mate| mate.id())
                    .collect(),
            }
        })
    }

    #[must_use]
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.product.groups.get(&id).map(Arc::as_ref)
    }

    pub fn definitions(&self) -> impl Iterator<Item = &Definition> {
        self.product.definitions.values().map(Arc::as_ref)
    }

    pub fn features(&self) -> impl Iterator<Item = &Feature> {
        self.product.features.values().map(Arc::as_ref)
    }

    pub fn occurrences(&self) -> impl Iterator<Item = &Occurrence> {
        self.product.occurrences.values().map(Arc::as_ref)
    }

    pub fn groups(&self) -> impl Iterator<Item = &Group> {
        self.product.groups.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn local_occurrence(&self, key: LocalOccurrenceKey) -> Option<&LocalOccurrence> {
        self.product.local_occurrences.get(&key).map(Arc::as_ref)
    }

    #[must_use]
    pub fn local_group(&self, key: LocalGroupKey) -> Option<&LocalGroup> {
        self.product.local_groups.get(&key).map(Arc::as_ref)
    }

    pub fn local_occurrences(&self) -> impl Iterator<Item = &LocalOccurrence> {
        self.product.local_occurrences.values().map(Arc::as_ref)
    }

    pub fn local_groups(&self) -> impl Iterator<Item = &LocalGroup> {
        self.product.local_groups.values().map(Arc::as_ref)
    }

    #[must_use]
    pub fn scene_query(&self) -> Vec<SceneOccurrence> {
        self.scene_query_bounded(usize::MAX, usize::MAX, usize::MAX)
            .expect("unbounded scene query cannot exceed its limits")
    }

    pub fn scene_query_bounded(
        &self,
        max_occurrences: usize,
        max_path_steps: usize,
        max_text_bytes: usize,
    ) -> Result<Vec<SceneOccurrence>, SceneQueryBudgetExceeded> {
        let mut occurrences = Vec::new();
        let mut text_bytes = 0usize;
        for occurrence in self.product.occurrences.values() {
            if occurrences.len() >= max_occurrences {
                return Err(SceneQueryBudgetExceeded {
                    kind: SceneQueryBudgetKind::Occurrences,
                    limit: max_occurrences,
                    observed_at_least: max_occurrences.saturating_add(1),
                });
            }
            let definition = &self.product.definitions[&occurrence.definition_id];
            consume_scene_text_budget(
                &mut text_bytes,
                occurrence.name.len().saturating_add(definition.name.len()),
                max_text_bytes,
            )?;
            let instance_path = InstancePath::root(occurrence.id);
            let world_transform =
                self.world_transform_for_occurrence_bounded(occurrence.id, max_path_steps)?;
            let visible = self
                .occurrence_effectively_visible(occurrence.id)
                .expect("validated occurrence is queryable");
            occurrences.push(SceneOccurrence {
                occurrence_id: occurrence.id,
                instance_path: instance_path.clone(),
                definition_id: definition.id,
                occurrence_name: occurrence.name.clone(),
                definition_name: definition.name.clone(),
                transform: world_transform,
                parent: occurrence.parent,
                local_parent: None,
                visible,
                color: occurrence.color,
                shared_occurrence_count: 0,
            });
            project_local_occurrences_bounded(
                &self.product,
                occurrence.id,
                definition.id,
                &instance_path,
                world_transform,
                visible,
                occurrence.color,
                &mut occurrences,
                max_occurrences,
                max_path_steps,
                &mut text_bytes,
                max_text_bytes,
            )?;
        }

        let mut sharing = BTreeMap::<DefinitionId, usize>::new();
        for occurrence in &occurrences {
            *sharing.entry(occurrence.definition_id).or_default() += 1;
        }
        for occurrence in &mut occurrences {
            occurrence.shared_occurrence_count = sharing[&occurrence.definition_id];
        }
        Ok(occurrences)
    }

    pub fn bind_scene_query(
        &self,
        context: SceneQueryContext,
    ) -> Result<BoundSceneQuery, SceneQueryError> {
        let context_is_valid = match &context {
            SceneQueryContext::Group(group_id) => self.group(*group_id).is_some(),
            SceneQueryContext::Definition {
                definition_id,
                instance_path,
            } => {
                self.resolve_instance_path(instance_path)
                    .is_ok_and(|resolved| resolved.definition_id == *definition_id)
                    && self.scene_query().into_iter().any(|occurrence| {
                        occurrence.instance_path == *instance_path && occurrence.visible
                    })
            }
        };
        if !context_is_valid {
            return Err(SceneQueryError::InvalidContext);
        }
        Ok(BoundSceneQuery {
            document_id: self.document_id(),
            source_revision: self.revision_id(),
            source_digest: self.canonical_digest(),
            context,
        })
    }

    pub fn scene_query_in(
        &self,
        query: &BoundSceneQuery,
    ) -> Result<Vec<SceneOccurrence>, SceneQueryError> {
        if query.document_id != self.document_id()
            || query.source_revision != self.revision_id()
            || query.source_digest != self.canonical_digest()
        {
            return Err(SceneQueryError::SnapshotMismatch);
        }
        let mut occurrences = self.scene_query();
        occurrences.retain(|occurrence| {
            occurrence.visible
                && match &query.context {
                    SceneQueryContext::Group(group_id) => {
                        occurrence.instance_path.is_root()
                            && self
                                .occurrence(occurrence.instance_path.root_occurrence())
                                .is_some_and(|root| root.parent() == Some(*group_id))
                    }
                    SceneQueryContext::Definition {
                        definition_id,
                        instance_path,
                    } => {
                        self.resolve_instance_path(instance_path)
                            .is_ok_and(|resolved| resolved.definition_id == *definition_id)
                            && occurrence.instance_path.root_occurrence()
                                == instance_path.root_occurrence()
                            && occurrence
                                .instance_path
                                .steps()
                                .starts_with(instance_path.steps())
                            && occurrence.instance_path.steps()[instance_path.steps().len()..]
                                .iter()
                                .filter(|step| matches!(step, InstancePathStep::Occurrence(_)))
                                .count()
                                <= 1
                    }
                }
        });
        let mut sharing = BTreeMap::<DefinitionId, usize>::new();
        for occurrence in &occurrences {
            *sharing.entry(occurrence.definition_id).or_default() += 1;
        }
        for occurrence in &mut occurrences {
            occurrence.shared_occurrence_count = sharing[&occurrence.definition_id];
        }
        Ok(occurrences)
    }

    pub fn resolve_instance_path(
        &self,
        path: &InstancePath,
    ) -> Result<ResolvedInstance, CanonicalError> {
        let root = self
            .occurrence(path.root_occurrence())
            .ok_or(CanonicalError::InvalidInstancePath)?;
        let mut definition_id = root.definition_id;
        let mut local_transform = root.transform;
        let mut parent_world_transform = root
            .parent
            .map_or(Some(Transform::identity()), |parent| {
                self.world_transform_for_group(parent)
            })
            .ok_or(CanonicalError::InvalidInstancePath)?;
        let mut transform = parent_world_transform.compose(local_transform);
        let mut resolved_path = InstancePath::root(path.root_occurrence());
        let mut parent = None;
        for step in &path.steps {
            match *step {
                InstancePathStep::Group(local_id) => {
                    let group = self
                        .local_group(LocalGroupKey {
                            definition_id,
                            local_id,
                        })
                        .ok_or(CanonicalError::InvalidInstancePath)?;
                    if group.parent != parent {
                        return Err(CanonicalError::InvalidInstancePath);
                    }
                    resolved_path = resolved_path.with_step(*step);
                    parent_world_transform = transform;
                    local_transform = group.transform;
                    transform = parent_world_transform.compose(local_transform);
                    parent = Some(local_id);
                }
                InstancePathStep::Occurrence(local_id) => {
                    let occurrence = self
                        .local_occurrence(LocalOccurrenceKey {
                            definition_id,
                            local_id,
                        })
                        .ok_or(CanonicalError::InvalidInstancePath)?;
                    if occurrence.parent != parent {
                        return Err(CanonicalError::InvalidInstancePath);
                    }
                    resolved_path = resolved_path.with_step(*step);
                    parent_world_transform = transform;
                    local_transform = self
                        .product
                        .instance_transform_overrides
                        .get(&resolved_path)
                        .copied()
                        .unwrap_or(occurrence.transform);
                    transform = parent_world_transform.compose(local_transform);
                    definition_id = occurrence.definition_id;
                    parent = None;
                }
            }
        }
        Ok(ResolvedInstance {
            definition_id,
            local_transform,
            parent_world_transform,
            world_transform: transform,
        })
    }

    #[must_use]
    pub fn world_transform_for_group(&self, id: GroupId) -> Option<Transform> {
        let mut lineage = Vec::new();
        let mut cursor = Some(id);
        while let Some(group_id) = cursor {
            let group = self.group(group_id)?;
            lineage.push(group_id);
            cursor = group.parent;
        }
        lineage.reverse();
        Some(
            lineage
                .into_iter()
                .fold(Transform::identity(), |transform, group_id| {
                    transform.compose(self.product.groups[&group_id].transform)
                }),
        )
    }

    pub(super) fn world_transform_for_occurrence_bounded(
        &self,
        id: OccurrenceId,
        max_group_steps: usize,
    ) -> Result<Transform, SceneQueryBudgetExceeded> {
        let occurrence = self
            .occurrence(id)
            .expect("validated occurrence is queryable");
        let mut lineage = Vec::new();
        let mut cursor = occurrence.parent;
        while let Some(group_id) = cursor {
            if lineage.len() >= max_group_steps {
                return Err(SceneQueryBudgetExceeded {
                    kind: SceneQueryBudgetKind::PathSteps,
                    limit: max_group_steps,
                    observed_at_least: max_group_steps.saturating_add(1),
                });
            }
            lineage.push(group_id);
            cursor = self.product.groups[&group_id].parent;
        }
        lineage.reverse();
        let parent_transform = lineage
            .into_iter()
            .fold(Transform::identity(), |transform, group_id| {
                transform.compose(self.product.groups[&group_id].transform)
            });
        Ok(parent_transform.compose(occurrence.transform))
    }

    #[must_use]
    pub fn world_transform_for_occurrence(&self, id: OccurrenceId) -> Option<Transform> {
        let occurrence = self.occurrence(id)?;
        let parent_transform = occurrence
            .parent
            .map_or(Some(Transform::identity()), |parent| {
                self.world_transform_for_group(parent)
            })?;
        Some(parent_transform.compose(occurrence.transform))
    }

    pub(crate) fn product(&self) -> &ProductModel {
        &self.product
    }
}

pub(super) fn consume_scene_text_budget(
    used: &mut usize,
    additional: usize,
    limit: usize,
) -> Result<(), SceneQueryBudgetExceeded> {
    let next = used.saturating_add(additional);
    if next > limit {
        return Err(SceneQueryBudgetExceeded {
            kind: SceneQueryBudgetKind::TextBytes,
            limit,
            observed_at_least: next,
        });
    }
    *used = next;
    Ok(())
}

// Recursive traversal carries its explicit budgets and accumulated scene state together.
#[allow(clippy::too_many_arguments)]
pub(super) fn project_local_occurrences_bounded(
    product: &ProductModel,
    root_occurrence_id: OccurrenceId,
    owner_definition_id: DefinitionId,
    owner_path: &InstancePath,
    owner_transform: Transform,
    owner_visible: bool,
    owner_color: Option<[u8; 3]>,
    output: &mut Vec<SceneOccurrence>,
    max_occurrences: usize,
    max_path_steps: usize,
    text_bytes: &mut usize,
    max_text_bytes: usize,
) -> Result<(), SceneQueryBudgetExceeded> {
    let definition = &product.definitions[&owner_definition_id];
    for local_id in &definition.local_occurrence_ids {
        let local = &product.local_occurrences[&LocalOccurrenceKey {
            definition_id: owner_definition_id,
            local_id: *local_id,
        }];
        let mut group_lineage = Vec::new();
        let mut parent = local.parent;
        while let Some(local_group_id) = parent {
            if owner_path.steps().len().saturating_add(group_lineage.len()) >= max_path_steps {
                return Err(SceneQueryBudgetExceeded {
                    kind: SceneQueryBudgetKind::PathSteps,
                    limit: max_path_steps,
                    observed_at_least: max_path_steps.saturating_add(1),
                });
            }
            let group = &product.local_groups[&LocalGroupKey {
                definition_id: owner_definition_id,
                local_id: local_group_id,
            }];
            group_lineage.push(local_group_id);
            parent = group.parent;
        }
        group_lineage.reverse();
        let next_path_steps = owner_path
            .steps()
            .len()
            .saturating_add(group_lineage.len())
            .saturating_add(1);
        if next_path_steps > max_path_steps {
            return Err(SceneQueryBudgetExceeded {
                kind: SceneQueryBudgetKind::PathSteps,
                limit: max_path_steps,
                observed_at_least: next_path_steps,
            });
        }
        let target_definition = &product.definitions[&local.definition_id];
        consume_scene_text_budget(
            text_bytes,
            local
                .name
                .len()
                .saturating_add(target_definition.name.len()),
            max_text_bytes,
        )?;

        let mut path = owner_path.clone();
        let mut world_transform = owner_transform;
        for local_group_id in group_lineage {
            let group = &product.local_groups[&LocalGroupKey {
                definition_id: owner_definition_id,
                local_id: local_group_id,
            }];
            path = path.with_step(InstancePathStep::Group(local_group_id));
            world_transform = world_transform.compose(group.transform);
        }
        path = path.with_step(InstancePathStep::Occurrence(*local_id));
        world_transform = world_transform.compose(
            product
                .instance_transform_overrides
                .get(&path)
                .copied()
                .unwrap_or(local.transform),
        );
        let visible =
            owner_visible && local.visible && all_tags_visible(&product.tags, &local.tags);
        let color = owner_color.or(local.color);
        if output.len() >= max_occurrences {
            return Err(SceneQueryBudgetExceeded {
                kind: SceneQueryBudgetKind::Occurrences,
                limit: max_occurrences,
                observed_at_least: max_occurrences.saturating_add(1),
            });
        }
        output.push(SceneOccurrence {
            occurrence_id: root_occurrence_id,
            instance_path: path.clone(),
            definition_id: local.definition_id,
            occurrence_name: local.name.clone(),
            definition_name: target_definition.name.clone(),
            transform: world_transform,
            parent: None,
            local_parent: local.parent,
            visible,
            color,
            shared_occurrence_count: 0,
        });
        project_local_occurrences_bounded(
            product,
            root_occurrence_id,
            local.definition_id,
            &path,
            world_transform,
            visible,
            color,
            output,
            max_occurrences,
            max_path_steps,
            text_bytes,
            max_text_bytes,
        )?;
    }
    Ok(())
}

/// A part is shown only while every tag it belongs to is visible.
pub(crate) fn all_tags_visible(tags: &BTreeMap<TagId, Arc<Tag>>, ids: &BTreeSet<TagId>) -> bool {
    ids.iter()
        .all(|id| tags.get(id).is_none_or(|tag| tag.visible))
}
