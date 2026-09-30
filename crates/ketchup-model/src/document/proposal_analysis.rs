use super::*;

pub(super) fn proposal_candidate_snapshot(
    snapshot: &Snapshot,
    batch: &CommandBatch,
    writes: &BTreeSet<AuthoritativeDependency>,
    goal: ProposalGoal,
) -> Result<(Snapshot, Vec<ProposalDiffEntry>, String), ProposalPrepareError> {
    let mut candidate =
        DocumentStore::from_product(snapshot.revision_id, snapshot.product.as_ref().clone())?;
    let revision = candidate.apply_batch_with_origin(
        batch,
        RevisionOrigin::Principal(ProposalPrincipal::ManualClient),
    )?;
    let after = revision.snapshot();
    let diff = writes
        .iter()
        .cloned()
        .map(|target| ProposalDiffEntry {
            target: target.clone(),
            before: proposal_value(snapshot, target.clone(), goal.clone()),
            after: proposal_value(after, target, goal.clone()),
        })
        .collect();
    Ok((after.clone(), diff, dependency_digest(after, writes)))
}

pub(super) fn proposal_candidate_with_validation(
    snapshot: &Snapshot,
    batch: &CommandBatch,
    writes: &BTreeSet<AuthoritativeDependency>,
    goal: ProposalGoal,
    validate_drawing_sources: bool,
) -> Result<(Vec<ProposalDiffEntry>, String), ProposalPrepareError> {
    let mut candidate =
        DocumentStore::from_product(snapshot.revision_id, snapshot.product.as_ref().clone())?;
    let revision = candidate.apply_batch_with_origin_and_validation(
        batch,
        RevisionOrigin::Principal(ProposalPrincipal::ManualClient),
        validate_drawing_sources,
    )?;
    let after = revision.snapshot();
    let diff = writes
        .iter()
        .cloned()
        .map(|target| ProposalDiffEntry {
            target: target.clone(),
            before: proposal_value(snapshot, target.clone(), goal.clone()),
            after: proposal_value(after, target, goal.clone()),
        })
        .collect();
    Ok((diff, dependency_digest(after, writes)))
}

pub(super) fn tip_replacement_candidate(
    parent: &Snapshot,
    corrected_revision: u64,
    batch: &CommandBatch,
    writes: &BTreeSet<AuthoritativeDependency>,
    goal: ProposalGoal,
) -> Result<(Snapshot, Vec<ProposalDiffEntry>, String), ProposalPrepareError> {
    let mut candidate =
        DocumentStore::from_product(parent.revision_id, parent.product.as_ref().clone())?;
    candidate.next_revision_id = corrected_revision;
    let revision = candidate.apply_batch(batch)?;
    let after = revision.snapshot();
    let diff = writes
        .iter()
        .cloned()
        .map(|target| ProposalDiffEntry {
            target: target.clone(),
            before: proposal_value(parent, target.clone(), goal.clone()),
            after: proposal_value(after, target, goal.clone()),
        })
        .collect();
    Ok((after.clone(), diff, dependency_digest(after, writes)))
}

pub(super) fn proposal_value(
    snapshot: &Snapshot,
    target: AuthoritativeDependency,
    goal: ProposalGoal,
) -> ProposalValue {
    match target {
        AuthoritativeDependency::EvaluatorNode(id) => {
            snapshot
                .evaluator_node(id)
                .map_or(ProposalValue::Missing, |node| {
                    if matches!(goal, ProposalGoal::CreateEvaluatorInput(_)) {
                        node.dimension()
                            .map_or(ProposalValue::Missing, |dimension| {
                                ProposalValue::EvaluatorInputState {
                                    name: node.name().to_owned(),
                                    dimension: dimension.clone(),
                                    dependencies: node.dependencies().to_vec(),
                                }
                            })
                    } else if matches!(goal, ProposalGoal::CreateEvaluatorExpression(_)) {
                        ProposalValue::EvaluatorExpressionState {
                            name: node.name().to_owned(),
                            expression: node.kind().source().to_owned(),
                            dependencies: node.dependencies().to_vec(),
                        }
                    } else if matches!(goal, ProposalGoal::CreateEvaluatorRule(_)) {
                        match node.kind() {
                            EvaluatorNodeKind::Rule {
                                outputs,
                                allowed_parameters,
                                ..
                            } => ProposalValue::EvaluatorRuleState {
                                name: node.name().to_owned(),
                                expression: node.kind().source().to_owned(),
                                dependencies: node.dependencies().to_vec(),
                                input_ports: node.input_ports().to_vec(),
                                output_ports: node.output_ports().to_vec(),
                                outputs: outputs.clone(),
                                override_parameters: allowed_parameters.clone(),
                            },
                            _ => ProposalValue::Missing,
                        }
                    } else if matches!(goal, ProposalGoal::RenameEvaluatorNode(_)) {
                        ProposalValue::Text(node.name().to_owned())
                    } else if matches!(goal, ProposalGoal::SetEvaluatorExpression(_)) {
                        ProposalValue::Text(node.kind().source().to_owned())
                    } else if matches!(goal, ProposalGoal::SetRuleOutputs(_)) {
                        match node.kind() {
                            EvaluatorNodeKind::Rule { outputs, .. } => {
                                ProposalValue::RuleOutputs(outputs.clone())
                            }
                            _ => ProposalValue::Missing,
                        }
                    } else {
                        node.dimension()
                            .cloned()
                            .map_or(ProposalValue::Missing, ProposalValue::Dimension)
                    }
                })
        }
        AuthoritativeDependency::Override(id)
            if matches!(
                goal,
                ProposalGoal::CreateRuleOverride(_) | ProposalGoal::DeleteRuleOverride(_)
            ) =>
        {
            snapshot
                .override_by_id(id)
                .map_or(ProposalValue::Missing, |value| {
                    ProposalValue::RuleOverrideState {
                        target: value.target.clone(),
                        parameter: value.parameter.clone(),
                        value: value.value(),
                        health: value.health.clone(),
                    }
                })
        }
        AuthoritativeDependency::FeatureParameterBinding(target)
            if matches!(
                goal,
                ProposalGoal::CreateFeatureParameterBinding(_)
                    | ProposalGoal::DeleteFeatureParameterBinding(_)
            ) =>
        {
            snapshot
                .feature_parameter_binding(&target)
                .map_or(ProposalValue::Missing, |binding| {
                    ProposalValue::FeatureParameterBindingState {
                        target: binding.target.clone(),
                        derived_from: binding.derived_from.clone(),
                    }
                })
        }
        AuthoritativeDependency::Joint(id)
            if matches!(
                goal,
                ProposalGoal::CreateJoint(_) | ProposalGoal::DeleteJoint(_)
            ) =>
        {
            snapshot
                .joint(id)
                .map_or(ProposalValue::Missing, |joint| ProposalValue::JointState {
                    participant_a: joint.participant_a().clone(),
                    participant_b: joint.participant_b().clone(),
                    volume_min: joint.volume().min(),
                    volume_max: joint.volume().max(),
                })
        }
        AuthoritativeDependency::Space(id)
            if matches!(
                goal,
                ProposalGoal::CreateSpace(_) | ProposalGoal::DeleteSpace(_)
            ) =>
        {
            snapshot
                .space(id)
                .map_or(ProposalValue::Missing, |space| ProposalValue::SpaceState {
                    purpose: space.purpose().to_owned(),
                    volume_min: space.volume().min(),
                    volume_max: space.volume().max(),
                    adjacent_to: space.adjacent_to().to_vec(),
                    accessible_to: space.accessible_to().to_vec(),
                })
        }
        AuthoritativeDependency::ClearanceVolume(id)
            if matches!(
                goal,
                ProposalGoal::CreateClearanceVolume(_) | ProposalGoal::DeleteClearanceVolume(_)
            ) =>
        {
            snapshot
                .clearance_volume(id)
                .map_or(ProposalValue::Missing, |clearance| {
                    ProposalValue::ClearanceVolumeState {
                        owner: clearance.owner().clone(),
                        reason: clearance.reason().to_owned(),
                        volume_min: clearance.volume().min(),
                        volume_max: clearance.volume().max(),
                        coordinate_frame: clearance.coordinate_frame(),
                        tolerance_mm: clearance.tolerance().linear_mm(),
                        severity: clearance.severity(),
                        derived_from: clearance.derived_from().cloned(),
                    }
                })
        }
        AuthoritativeDependency::PersistentDimension(id)
            if matches!(
                goal,
                ProposalGoal::CreatePersistentDimension(_)
                    | ProposalGoal::DeletePersistentDimension(_)
            ) =>
        {
            snapshot
                .persistent_dimension(id)
                .map_or(ProposalValue::Missing, |dimension| {
                    ProposalValue::PersistentDimensionState {
                        name: dimension.name.clone(),
                        target: dimension.target.clone(),
                        presentation: dimension.presentation,
                    }
                })
        }
        AuthoritativeDependency::Feature(id) => {
            snapshot
                .feature(id)
                .map_or(ProposalValue::Missing, |feature| {
                    if let ProposalGoal::RecomputeFeatureParameter(ref parameter) = goal
                        && parameter.feature_id == id
                    {
                        return feature_parameter_dimension(&snapshot.product, parameter)
                            .map_or(ProposalValue::Missing, ProposalValue::Dimension);
                    }
                    match (feature.kind(), feature.kind().polygon_points()) {
                        (_, Some(points_mm))
                            if matches!(goal, ProposalGoal::SetProfilePoints(_)) =>
                        {
                            ProposalValue::ProfilePoints(points_mm)
                        }
                        (_, Some(points_mm))
                            if matches!(
                                goal,
                                ProposalGoal::CreateProfileFeature(_)
                                    | ProposalGoal::DeleteProfileFeature(_)
                                    | ProposalGoal::CloneProfileDefinitionAndRepoint(_)
                            ) =>
                        {
                            ProposalValue::ProfileFeatureState {
                                definition: feature.definition_id(),
                                name: feature.name().to_owned(),
                                points_mm,
                            }
                        }
                        (
                            FeatureKind::Pad(PadSpec {
                                extent: FeatureExtent::Blind(height),
                                ..
                            }),
                            _,
                        ) => ProposalValue::Dimension(height.clone()),
                        _ => ProposalValue::Digest(dependency_digest(
                            snapshot,
                            &BTreeSet::from([target]),
                        )),
                    }
                })
        }
        AuthoritativeDependency::Tag(id) => {
            snapshot
                .tag(id)
                .map_or(ProposalValue::Missing, |tag| match goal {
                    ProposalGoal::SetTagVisibility(_) => ProposalValue::Boolean(tag.visible()),
                    ProposalGoal::CreateTag(_) | ProposalGoal::DeleteTag(_) => {
                        ProposalValue::TagState {
                            name: tag.name().to_owned(),
                            visible: tag.visible(),
                        }
                    }
                    _ => ProposalValue::Digest(dependency_digest(
                        snapshot,
                        &BTreeSet::from([target]),
                    )),
                })
        }
        AuthoritativeDependency::Collection(id)
            if matches!(
                goal,
                ProposalGoal::SetCollectionOccurrences(_)
                    | ProposalGoal::CreateCollection(_)
                    | ProposalGoal::DeleteCollection(_)
            ) =>
        {
            snapshot
                .collection(id)
                .map_or(ProposalValue::Missing, |collection| match goal {
                    ProposalGoal::CreateCollection(_) => {
                        ProposalValue::Text(collection.name().to_owned())
                    }
                    ProposalGoal::SetCollectionOccurrences(_) => {
                        ProposalValue::Occurrences(collection.occurrence_ids().collect())
                    }
                    ProposalGoal::DeleteCollection(_) => ProposalValue::CollectionState {
                        name: collection.name().to_owned(),
                        occurrence_ids: collection.occurrence_ids().collect(),
                    },
                    _ => unreachable!(),
                })
        }
        AuthoritativeDependency::Definition(id) => {
            snapshot
                .definition(id)
                .map_or(ProposalValue::Missing, |definition| {
                    if matches!(
                        goal,
                        ProposalGoal::RenameDefinition(_) | ProposalGoal::CreateDefinition(_)
                    ) {
                        ProposalValue::Text(definition.name().to_owned())
                    } else if matches!(
                        goal,
                        ProposalGoal::DeleteDefinition(_)
                            | ProposalGoal::CloneProfileDefinitionAndRepoint(_)
                            | ProposalGoal::ConvertEmptyGroupToComponent(_)
                    ) {
                        ProposalValue::DefinitionState {
                            name: definition.name().to_owned(),
                            feature_ids: definition.feature_ids().to_vec(),
                            local_occurrence_ids: definition.local_occurrence_ids().to_vec(),
                            local_group_ids: definition.local_group_ids().to_vec(),
                        }
                    } else if matches!(
                        goal,
                        ProposalGoal::CreateProfileFeature(_)
                            | ProposalGoal::DeleteProfileFeature(_)
                    ) {
                        ProposalValue::DefinitionFeatures(definition.feature_ids().to_vec())
                    } else {
                        ProposalValue::Digest(dependency_digest(
                            snapshot,
                            &BTreeSet::from([target]),
                        ))
                    }
                })
        }
        AuthoritativeDependency::Occurrence(id) => {
            snapshot
                .occurrence(id)
                .map_or(ProposalValue::Missing, |occurrence| match goal {
                    ProposalGoal::SetOccurrenceTranslation(_) => {
                        ProposalValue::Transform(occurrence.transform())
                    }
                    ProposalGoal::AtomicMultiCommandEdit(_) => ProposalValue::OccurrenceState {
                        definition: occurrence.definition_id(),
                        name: occurrence.name().to_owned(),
                        transform: occurrence.transform(),
                        parent: occurrence.parent(),
                        tag: occurrence.tag(),
                        visible: occurrence.visible(),
                    },
                    ProposalGoal::SetOccurrenceTag(_) => ProposalValue::Tag(occurrence.tag()),
                    ProposalGoal::RepointOccurrence(_) => {
                        ProposalValue::Definition(occurrence.definition_id())
                    }
                    ProposalGoal::SetOccurrenceParent(_) => {
                        ProposalValue::Group(occurrence.parent())
                    }
                    ProposalGoal::CreateOccurrence(_)
                    | ProposalGoal::DeleteOccurrence(_)
                    | ProposalGoal::CloneProfileDefinitionAndRepoint(_)
                    | ProposalGoal::ConvertEmptyGroupToComponent(_) => {
                        ProposalValue::OccurrenceState {
                            definition: occurrence.definition_id(),
                            name: occurrence.name().to_owned(),
                            transform: occurrence.transform(),
                            parent: occurrence.parent(),
                            tag: occurrence.tag(),
                            visible: occurrence.visible(),
                        }
                    }
                    _ => ProposalValue::Boolean(occurrence.visible()),
                })
        }
        AuthoritativeDependency::GroupSubtree(id)
            if matches!(goal, ProposalGoal::ConvertEmptyGroupToComponent(_)) =>
        {
            snapshot
                .group(id)
                .map_or(ProposalValue::Missing, |group| ProposalValue::GroupState {
                    name: group.name().to_owned(),
                    transform: group.transform(),
                    parent: group.parent(),
                })
        }
        AuthoritativeDependency::Group(id)
            if matches!(
                goal,
                ProposalGoal::SetGroupTranslation(_)
                    | ProposalGoal::SetGroupParent(_)
                    | ProposalGoal::CreateGroup(_)
                    | ProposalGoal::DeleteGroup(_)
            ) =>
        {
            snapshot
                .group(id)
                .map_or(ProposalValue::Missing, |group| match goal {
                    ProposalGoal::SetGroupTranslation(_) => {
                        ProposalValue::Transform(group.transform())
                    }
                    ProposalGoal::SetGroupParent(_) => ProposalValue::Group(group.parent()),
                    ProposalGoal::CreateGroup(_) | ProposalGoal::DeleteGroup(_) => {
                        ProposalValue::GroupState {
                            name: group.name().to_owned(),
                            transform: group.transform(),
                            parent: group.parent(),
                        }
                    }
                    _ => unreachable!(),
                })
        }
        _ => ProposalValue::Digest(dependency_digest(snapshot, &BTreeSet::from([target]))),
    }
}

pub(super) fn feature_parameter_bindings_in_scope<'a>(
    snapshot: &'a Snapshot,
    scope: &FeatureParameterRecomputeScope,
) -> Vec<&'a FeatureParameterBinding> {
    match scope {
        FeatureParameterRecomputeScope::All => snapshot.feature_parameter_bindings().collect(),
        FeatureParameterRecomputeScope::AffectedBy(nodes) => {
            let affected = dependent_closure(&snapshot.product.evaluator_nodes, nodes);
            snapshot
                .feature_parameter_bindings()
                .filter(|binding| affected.contains(&binding.derived_from.root_rule_node_id))
                .collect()
        }
    }
}

pub(super) fn authoritative_writes(
    snapshot: &Snapshot,
    batch: &CommandBatch,
) -> BTreeSet<AuthoritativeDependency> {
    let mut writes = BTreeSet::new();
    for command in &batch.commands {
        match command {
            CanonicalCommand::SetTolerance { .. } => {
                writes.insert(AuthoritativeDependency::Tolerance);
            }
            CanonicalCommand::SetProductionCode { .. } => {
                writes.insert(AuthoritativeDependency::ProductionCodes);
            }
            CanonicalCommand::CreateEvaluatorNode { id, .. }
            | CanonicalCommand::SetEvaluatorDimension { id, .. }
            | CanonicalCommand::RenameEvaluatorNode { id, .. }
            | CanonicalCommand::CreateExpressionNode { id, .. }
            | CanonicalCommand::CreateRuleNode { id, .. }
            | CanonicalCommand::SetNodeExpression { id, .. }
            | CanonicalCommand::SetRuleOutputs { id, .. } => {
                writes.insert(AuthoritativeDependency::EvaluatorNode(*id));
            }
            CanonicalCommand::UpsertOverride(value) => {
                writes.insert(AuthoritativeDependency::Override(value.id));
            }
            CanonicalCommand::DeleteOverride { id } => {
                writes.insert(AuthoritativeDependency::Override(*id));
            }
            CanonicalCommand::UpsertFeatureParameterBinding(binding) => {
                writes.insert(AuthoritativeDependency::FeatureParameterBinding(
                    binding.target.clone(),
                ));
            }
            CanonicalCommand::DeleteFeatureParameterBinding { target } => {
                writes.insert(AuthoritativeDependency::FeatureParameterBinding(
                    target.clone(),
                ));
            }
            CanonicalCommand::RecomputeFeatureParameters { scope, .. } => {
                writes.extend(
                    feature_parameter_bindings_in_scope(snapshot, scope)
                        .into_iter()
                        .map(|binding| AuthoritativeDependency::Feature(binding.target.feature_id)),
                );
            }
            CanonicalCommand::UpsertJoint(joint) => {
                writes.insert(AuthoritativeDependency::Joint(joint.id()));
            }
            CanonicalCommand::DeleteJoint { id } => {
                writes.insert(AuthoritativeDependency::Joint(*id));
            }
            CanonicalCommand::UpsertSpace(space) => {
                writes.insert(AuthoritativeDependency::Space(space.id()));
            }
            CanonicalCommand::DeleteSpace { id } => {
                writes.insert(AuthoritativeDependency::Space(*id));
            }
            CanonicalCommand::UpsertClearanceVolume(clearance) => {
                writes.insert(AuthoritativeDependency::ClearanceVolume(clearance.id()));
            }
            CanonicalCommand::DeleteClearanceVolume { id } => {
                writes.insert(AuthoritativeDependency::ClearanceVolume(*id));
            }
            CanonicalCommand::UpsertCamPlan(plan) => {
                writes.insert(AuthoritativeDependency::CamPlan(plan.id()));
            }
            CanonicalCommand::DeleteCamPlan { id } => {
                writes.insert(AuthoritativeDependency::CamPlan(*id));
            }
            CanonicalCommand::UpsertPinJoint(joint) => {
                writes.insert(AuthoritativeDependency::PinJoint(joint.id));
            }
            CanonicalCommand::DeletePinJoint { id } => {
                writes.insert(AuthoritativeDependency::PinJoint(*id));
            }
            CanonicalCommand::SetAssemblyRecipe(_) | CanonicalCommand::ClearAssemblyRecipe => {
                writes.insert(AuthoritativeDependency::AssemblyRecipe);
            }
            CanonicalCommand::UpsertPersistentDimension(dimension) => {
                writes.insert(AuthoritativeDependency::PersistentDimension(dimension.id));
            }
            CanonicalCommand::DeletePersistentDimension { id } => {
                writes.insert(AuthoritativeDependency::PersistentDimension(*id));
            }
            CanonicalCommand::CreateTag { id, .. }
            | CanonicalCommand::DeleteTag { id }
            | CanonicalCommand::SetTagVisibility { id, .. }
            | CanonicalCommand::SetTagName { id, .. } => {
                writes.insert(AuthoritativeDependency::Tag(*id));
            }
            CanonicalCommand::UpsertClassificationDimension { id, .. } => {
                writes.insert(AuthoritativeDependency::ClassificationDimension(*id));
            }
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id,
                dimension_id,
                ..
            } => {
                writes.insert(AuthoritativeDependency::OccurrenceClassification(
                    *occurrence_id,
                    *dimension_id,
                ));
            }
            CanonicalCommand::CreateCollection { id, .. }
            | CanonicalCommand::DeleteCollection { id }
            | CanonicalCommand::SetCollectionOccurrences { id, .. } => {
                writes.insert(AuthoritativeDependency::Collection(*id));
            }
            CanonicalCommand::RecordImport(receipt) => {
                writes.insert(AuthoritativeDependency::Import(receipt.id()));
            }
            CanonicalCommand::CreateDefinition { id, .. }
            | CanonicalCommand::DeleteDefinition { id }
            | CanonicalCommand::RenameDefinition { id, .. } => {
                writes.insert(AuthoritativeDependency::Definition(*id));
            }
            CanonicalCommand::CreateBody { definition_id, .. }
            | CanonicalCommand::DeleteBody { definition_id, .. }
            | CanonicalCommand::RenameBody { definition_id, .. }
            | CanonicalCommand::SetActiveBody { definition_id, .. }
            | CanonicalCommand::SetBodyVisibility { definition_id, .. }
            | CanonicalCommand::ConsumeBody { definition_id, .. } => {
                writes.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::SetFeatureBodyOwnership { id, .. } => {
                writes.insert(AuthoritativeDependency::Feature(*id));
                if let Some(feature) = snapshot.feature(*id) {
                    writes.insert(AuthoritativeDependency::Definition(feature.definition_id()));
                }
            }
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id,
                body_id,
                ..
            } => {
                writes.insert(AuthoritativeDependency::BodyFeatureSuppression(
                    *definition_id,
                    *body_id,
                ));
            }
            CanonicalCommand::CreateFeature {
                id, definition_id, ..
            } => {
                writes.insert(AuthoritativeDependency::Feature(*id));
                writes.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::DeleteFeature { id } => {
                writes.insert(AuthoritativeDependency::Feature(*id));
                if let Some(feature) = snapshot.feature(*id) {
                    writes.insert(AuthoritativeDependency::Definition(feature.definition_id()));
                }
            }
            CanonicalCommand::SetFeatureParameter { target, .. } => {
                writes.insert(AuthoritativeDependency::Feature(target.feature_id));
            }
            CanonicalCommand::SetFeatureDimension { id, .. }
            | CanonicalCommand::CreateSketchConstraint { id, .. }
            | CanonicalCommand::ReplaceSketchConstraint { id, .. }
            | CanonicalCommand::DeleteSketchConstraint { id, .. }
            | CanonicalCommand::SetSketchConstraintDimension { id, .. }
            | CanonicalCommand::SplitSketchEntity { id, .. }
            | CanonicalCommand::JoinSketchEntities { id, .. }
            | CanonicalCommand::TrimSketchEntity { id, .. }
            | CanonicalCommand::ExtendSketchEntity { id, .. }
            | CanonicalCommand::OffsetSketchEntity { id, .. }
            | CanonicalCommand::ProjectSketchEntity { id, .. }
            | CanonicalCommand::SetSketchEntityConstruction { id, .. }
            | CanonicalCommand::TranslateProfile { id, .. }
            | CanonicalCommand::SetProfilePoints { id, .. } => {
                writes.insert(AuthoritativeDependency::Feature(*id));
            }
            CanonicalCommand::GuardAssemblyRecompute { .. } => {}
            CanonicalCommand::ApplyAssemblySolve {
                transforms,
                instance_transforms,
                ..
            } => {
                writes.extend(
                    transforms
                        .iter()
                        .map(|(id, _)| AuthoritativeDependency::Occurrence(*id)),
                );
                writes.extend(
                    instance_transforms.iter().map(|(path, _)| {
                        AuthoritativeDependency::Occurrence(path.root_occurrence())
                    }),
                );
            }
            CanonicalCommand::SetOccurrenceGrounded { id, .. } => {
                writes.insert(AuthoritativeDependency::GroundedOccurrence(*id));
            }
            CanonicalCommand::CreateAssemblyMate(mate)
            | CanonicalCommand::RebindAssemblyMate(mate) => {
                writes.insert(AuthoritativeDependency::AssemblyMate(mate.id()));
            }
            CanonicalCommand::SetAssemblyMateKind { id, .. }
            | CanonicalCommand::DeleteAssemblyMate { id } => {
                writes.insert(AuthoritativeDependency::AssemblyMate(*id));
            }
            CanonicalCommand::CreateAssemblyJoint(joint) => {
                writes.insert(AuthoritativeDependency::AssemblyJoint(joint.id()));
            }
            CanonicalCommand::SetAssemblyJointKind { id, .. }
            | CanonicalCommand::SetAssemblyJointPosition { id, .. }
            | CanonicalCommand::SetAssemblyJointLimits { id, .. }
            | CanonicalCommand::DeleteAssemblyJoint { id } => {
                writes.insert(AuthoritativeDependency::AssemblyJoint(*id));
            }
            CanonicalCommand::CreateAssemblyMotionCoupling(coupling)
            | CanonicalCommand::UpdateAssemblyMotionCoupling(coupling) => {
                writes.insert(AuthoritativeDependency::AssemblyMotionCoupling(
                    coupling.id(),
                ));
            }
            CanonicalCommand::DeleteAssemblyMotionCoupling { id } => {
                writes.insert(AuthoritativeDependency::AssemblyMotionCoupling(*id));
            }
            CanonicalCommand::CreateAssemblyMotionStudy(study)
            | CanonicalCommand::UpdateAssemblyMotionStudy(study) => {
                writes.insert(AuthoritativeDependency::AssemblyMotionStudy(study.id()));
            }
            CanonicalCommand::DeleteAssemblyMotionStudy { id } => {
                writes.insert(AuthoritativeDependency::AssemblyMotionStudy(*id));
            }
            CanonicalCommand::CreateMechanicalInterface(interface)
            | CanonicalCommand::UpdateMechanicalInterface(interface) => {
                writes.insert(AuthoritativeDependency::MechanicalInterface(interface.id()));
            }
            CanonicalCommand::DeleteMechanicalInterface { id } => {
                writes.insert(AuthoritativeDependency::MechanicalInterface(*id));
            }
            CanonicalCommand::CreateMechanicalCondition(condition)
            | CanonicalCommand::UpdateMechanicalCondition(condition) => {
                writes.insert(AuthoritativeDependency::MechanicalCondition(condition.id()));
            }
            CanonicalCommand::DeleteMechanicalCondition { id } => {
                writes.insert(AuthoritativeDependency::MechanicalCondition(*id));
            }
            CanonicalCommand::CreateDrawingSheet(sheet)
            | CanonicalCommand::UpdateDrawingSheet(sheet) => {
                writes.insert(AuthoritativeDependency::DrawingSheet(sheet.id()));
            }
            CanonicalCommand::DeleteDrawingSheet { id } => {
                writes.insert(AuthoritativeDependency::DrawingSheet(*id));
            }
            CanonicalCommand::CreateOccurrence { id, .. }
            | CanonicalCommand::DeleteOccurrence { id }
            | CanonicalCommand::SetOccurrenceTransform { id, .. }
            | CanonicalCommand::RenameEntity { id, .. }
            | CanonicalCommand::SetOccurrenceColor { id, .. }
            | CanonicalCommand::SetOccurrenceVisibility { id, .. }
            | CanonicalCommand::SetOccurrenceTag { id, .. }
            | CanonicalCommand::RepointOccurrence { id, .. }
            | CanonicalCommand::SetOccurrenceParent { id, .. } => {
                writes.insert(AuthoritativeDependency::Occurrence(*id));
            }
            CanonicalCommand::CreateGroup { id, .. }
            | CanonicalCommand::DeleteGroup { id }
            | CanonicalCommand::SetGroupTransform { id, .. }
            | CanonicalCommand::SetGroupParent { id, .. } => {
                writes.insert(AuthoritativeDependency::Group(*id));
            }
            CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                let path = InstancePath::root(plan.occurrence_id);
                for joint in snapshot.pin_joints().filter(|joint| {
                    joint.physical_hole_pairs.is_some()
                        && (joint.first.instance_path == path || joint.second.instance_path == path)
                }) {
                    writes.insert(AuthoritativeDependency::PinJoint(joint.id));
                }
                if snapshot
                    .assembly_recipe()
                    .is_some_and(|recipe| recipe.parts().any(|part| part.instance_path == path))
                {
                    writes.insert(AuthoritativeDependency::AssemblyRecipe);
                }
                writes.insert(AuthoritativeDependency::Occurrence(plan.occurrence_id));
                writes.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                for (_, new_id) in &plan.feature_id_map {
                    writes.insert(AuthoritativeDependency::Feature(*new_id));
                }
            }
            CanonicalCommand::ConvertGroupToComponent(plan) => {
                writes.insert(AuthoritativeDependency::GroupSubtree(plan.group_id));
                writes.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                writes.insert(AuthoritativeDependency::Occurrence(plan.new_occurrence_id));
            }
            CanonicalCommand::ApplySolidTool(plan) => {
                writes.insert(AuthoritativeDependency::Occurrence(
                    plan.target_occurrence_id,
                ));
                writes.insert(AuthoritativeDependency::Occurrence(plan.tool_occurrence_id));
                writes.insert(AuthoritativeDependency::Definition(
                    plan.result_definition_id,
                ));
                writes.extend(
                    plan.result_feature_ids
                        .iter()
                        .cloned()
                        .map(AuthoritativeDependency::Feature),
                );
            }
        }
    }
    writes
}

pub(super) fn authoritative_dependencies(
    snapshot: &Snapshot,
    batch: &CommandBatch,
) -> BTreeSet<AuthoritativeDependency> {
    let mut dependencies = BTreeSet::new();
    for command in &batch.commands {
        match command {
            CanonicalCommand::SetTolerance { .. } => {
                dependencies.insert(AuthoritativeDependency::Tolerance);
            }
            CanonicalCommand::SetProductionCode { instance_path, .. } => {
                dependencies.insert(AuthoritativeDependency::ProductionCodes);
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    instance_path.root_occurrence(),
                ));
                if let Some(owners) = production_code_identity(&snapshot.product, instance_path) {
                    dependencies
                        .extend(owners.into_iter().map(AuthoritativeDependency::Definition));
                }
            }
            CanonicalCommand::CreateEvaluatorNode {
                id,
                dependencies: node_dependencies,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
                for dependency in node_dependencies {
                    add_evaluator_dependency_closure(snapshot, *dependency, &mut dependencies);
                }
            }
            CanonicalCommand::SetEvaluatorDimension { id, .. }
            | CanonicalCommand::RenameEvaluatorNode { id, .. } => {
                add_evaluator_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::RecordImport(receipt) => {
                dependencies.insert(AuthoritativeDependency::Import(receipt.id()));
                for output in receipt.outputs() {
                    dependencies.insert(match output {
                        ImportOutputRef::Definition(id) => AuthoritativeDependency::Definition(*id),
                        ImportOutputRef::Feature(id) => AuthoritativeDependency::Feature(*id),
                        ImportOutputRef::Occurrence(id) => AuthoritativeDependency::Occurrence(*id),
                        ImportOutputRef::Group(id) => AuthoritativeDependency::Group(*id),
                    });
                }
            }
            CanonicalCommand::CreateDefinition { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
            }
            CanonicalCommand::DeleteDefinition { id } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
                dependencies.insert(AuthoritativeDependency::DefinitionUsers(*id));
            }
            CanonicalCommand::RenameDefinition { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Definition(*id));
            }
            CanonicalCommand::CreateBody { definition_id, .. }
            | CanonicalCommand::DeleteBody { definition_id, .. }
            | CanonicalCommand::RenameBody { definition_id, .. }
            | CanonicalCommand::SetActiveBody { definition_id, .. }
            | CanonicalCommand::SetBodyVisibility { definition_id, .. } => {
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::ConsumeBody {
                definition_id,
                by_feature_id,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                add_feature_dependency_closure(snapshot, *by_feature_id, &mut dependencies);
            }
            CanonicalCommand::SetFeatureBodyOwnership { id, .. } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
                if let Some(feature) = snapshot.feature(*id) {
                    dependencies
                        .insert(AuthoritativeDependency::Definition(feature.definition_id()));
                }
            }
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id,
                body_id,
                suppressed_feature_ids,
            } => {
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                dependencies.insert(AuthoritativeDependency::BodyFeatureSuppression(
                    *definition_id,
                    *body_id,
                ));
                for feature_id in suppressed_feature_ids {
                    add_feature_dependency_closure(snapshot, *feature_id, &mut dependencies);
                }
                if let Some(current) = snapshot.suppressed_feature_ids(*definition_id, *body_id) {
                    for feature_id in current {
                        add_feature_dependency_closure(snapshot, *feature_id, &mut dependencies);
                    }
                }
            }
            CanonicalCommand::CreateFeature {
                id,
                definition_id,
                kind,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Feature(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                match kind {
                    FeatureKind::Workplane(spec) => match &spec.support {
                        WorkplaneSupport::Free | WorkplaneSupport::Principal(_) => {}
                        WorkplaneSupport::Offset { base, .. }
                        | WorkplaneSupport::ConstructionPlane { feature: base } => {
                            add_feature_dependency_closure(snapshot, *base, &mut dependencies);
                        }
                        WorkplaneSupport::PlanarFace { reference, .. } => {
                            add_feature_dependency_closure(
                                snapshot,
                                reference.producer_feature_id,
                                &mut dependencies,
                            );
                        }
                    },
                    FeatureKind::Sketch(spec) => {
                        add_feature_dependency_closure(snapshot, spec.workplane, &mut dependencies);
                        for constraint in &spec.constraints {
                            if let SketchConstraintKind::Projection { source_feature, .. } =
                                constraint.kind
                            {
                                add_feature_dependency_closure(
                                    snapshot,
                                    source_feature,
                                    &mut dependencies,
                                );
                            }
                        }
                    }
                    FeatureKind::Pad(spec) => {
                        if let Some(target) = spec.operation.target() {
                            add_feature_dependency_closure(snapshot, target, &mut dependencies);
                        }
                        add_feature_dependency_closure(
                            snapshot,
                            spec.profile.feature_id(),
                            &mut dependencies,
                        );
                    }
                    FeatureKind::Revolve { profile, .. }
                    | FeatureKind::PlanarOffset { profile, .. }
                    | FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile }) => {
                        add_feature_dependency_closure(snapshot, *profile, &mut dependencies);
                    }
                    FeatureKind::Sweep { profile, path } => {
                        add_feature_dependency_closure(snapshot, *profile, &mut dependencies);
                        add_feature_dependency_closure(snapshot, *path, &mut dependencies);
                    }
                    FeatureKind::WeldmentMember(spec) => {
                        add_feature_dependency_closure(snapshot, spec.profile, &mut dependencies);
                        add_feature_dependency_closure(snapshot, spec.path, &mut dependencies);
                    }
                    FeatureKind::WeldmentJoint(spec) => {
                        add_feature_dependency_closure(
                            snapshot,
                            spec.first_member,
                            &mut dependencies,
                        );
                        add_feature_dependency_closure(
                            snapshot,
                            spec.second_member,
                            &mut dependencies,
                        );
                    }
                    FeatureKind::Loft {
                        sections, guide, ..
                    }
                    | FeatureKind::SurfaceBody(SurfaceBodySpec::Loft {
                        sections, guide, ..
                    }) => {
                        for source in sections.iter().map(|section| section.profile).chain(*guide) {
                            add_feature_dependency_closure(snapshot, source, &mut dependencies);
                        }
                    }
                    FeatureKind::Boolean { target, tool, .. }
                    | FeatureKind::SurfaceTrim {
                        target,
                        cutter: tool,
                    } => {
                        add_feature_dependency_closure(snapshot, *target, &mut dependencies);
                        add_feature_dependency_closure(snapshot, *tool, &mut dependencies);
                    }
                    FeatureKind::Shell { target, .. }
                    | FeatureKind::EdgeFinish { target, .. }
                    | FeatureKind::FaceOffset { target, .. }
                    | FeatureKind::SurfaceExtend { target, .. }
                    | FeatureKind::SurfaceThicken { target, .. }
                    | FeatureKind::RigidTransform { target, .. } => {
                        add_feature_dependency_closure(snapshot, *target, &mut dependencies);
                    }
                    FeatureKind::SurfaceKnit { surfaces, .. } => {
                        for surface in surfaces {
                            add_feature_dependency_closure(snapshot, *surface, &mut dependencies);
                        }
                    }
                    FeatureKind::Profile { .. }
                    | FeatureKind::SheetMetal(_)
                    | FeatureKind::SpatialPath { .. }
                    | FeatureKind::ConstructionPoint { .. }
                    | FeatureKind::ConstructionAxis { .. }
                    | FeatureKind::ConstructionPlane { .. }
                    | FeatureKind::ImportedExactBody(_)
                    | FeatureKind::MeshBody(_) => {}
                }
            }
            CanonicalCommand::DeleteFeature { id } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
                dependencies.insert(AuthoritativeDependency::FeatureUsers(*id));
            }
            CanonicalCommand::ProjectSketchEntity {
                id,
                source_feature_id,
                ..
            } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
                add_feature_dependency_closure(snapshot, *source_feature_id, &mut dependencies);
            }
            CanonicalCommand::SetFeatureParameter { target, .. } => {
                add_feature_dependency_closure(snapshot, target.feature_id, &mut dependencies);
            }
            CanonicalCommand::SetFeatureDimension { id, .. }
            | CanonicalCommand::CreateSketchConstraint { id, .. }
            | CanonicalCommand::ReplaceSketchConstraint { id, .. }
            | CanonicalCommand::DeleteSketchConstraint { id, .. }
            | CanonicalCommand::SetSketchConstraintDimension { id, .. }
            | CanonicalCommand::SplitSketchEntity { id, .. }
            | CanonicalCommand::JoinSketchEntities { id, .. }
            | CanonicalCommand::TrimSketchEntity { id, .. }
            | CanonicalCommand::ExtendSketchEntity { id, .. }
            | CanonicalCommand::OffsetSketchEntity { id, .. }
            | CanonicalCommand::SetSketchEntityConstruction { id, .. }
            | CanonicalCommand::TranslateProfile { id, .. }
            | CanonicalCommand::SetProfilePoints { id, .. } => {
                add_feature_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::GuardAssemblyRecompute { .. } => {
                dependencies.extend(
                    snapshot
                        .occurrences()
                        .map(|occurrence| AuthoritativeDependency::Occurrence(occurrence.id())),
                );
                dependencies.extend(
                    snapshot
                        .grounded_occurrences()
                        .map(AuthoritativeDependency::GroundedOccurrence),
                );
                for mate in snapshot.assembly_mates() {
                    dependencies.insert(AuthoritativeDependency::AssemblyMate(mate.id()));
                    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
                        add_feature_dependency_closure(
                            snapshot,
                            endpoint.reference().producer_feature_id,
                            &mut dependencies,
                        );
                    }
                }
                dependencies.extend(
                    snapshot
                        .assembly_joints()
                        .map(|joint| AuthoritativeDependency::AssemblyJoint(joint.id())),
                );
                dependencies.extend(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| AuthoritativeDependency::AssemblyMotionStudy(study.id())),
                );
                dependencies.extend(snapshot.assembly_motion_couplings().map(|coupling| {
                    AuthoritativeDependency::AssemblyMotionCoupling(coupling.id())
                }));
            }
            CanonicalCommand::ApplyAssemblySolve {
                transforms,
                instance_transforms,
                ..
            } => {
                dependencies.extend(
                    transforms
                        .iter()
                        .map(|(id, _)| AuthoritativeDependency::Occurrence(*id)),
                );
                dependencies.extend(
                    instance_transforms.iter().map(|(path, _)| {
                        AuthoritativeDependency::Occurrence(path.root_occurrence())
                    }),
                );
                dependencies.extend(
                    snapshot
                        .grounded_occurrences()
                        .map(AuthoritativeDependency::GroundedOccurrence),
                );
                for mate in snapshot.assembly_mates() {
                    dependencies.insert(AuthoritativeDependency::AssemblyMate(mate.id()));
                    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
                        dependencies.insert(AuthoritativeDependency::Occurrence(
                            endpoint.occurrence_id(),
                        ));
                        add_feature_dependency_closure(
                            snapshot,
                            endpoint.reference().producer_feature_id,
                            &mut dependencies,
                        );
                    }
                }
                dependencies.extend(
                    snapshot
                        .assembly_joints()
                        .map(|joint| AuthoritativeDependency::AssemblyJoint(joint.id())),
                );
                dependencies.extend(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| AuthoritativeDependency::AssemblyMotionStudy(study.id())),
                );
                dependencies.extend(snapshot.assembly_motion_couplings().map(|coupling| {
                    AuthoritativeDependency::AssemblyMotionCoupling(coupling.id())
                }));
            }
            CanonicalCommand::SetOccurrenceGrounded { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::GroundedOccurrence(*id));
            }
            CanonicalCommand::CreateAssemblyMate(mate)
            | CanonicalCommand::RebindAssemblyMate(mate) => {
                dependencies.insert(AuthoritativeDependency::AssemblyMate(mate.id()));
                for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
                    dependencies.insert(AuthoritativeDependency::Occurrence(
                        endpoint.occurrence_id(),
                    ));
                    add_feature_dependency_closure(
                        snapshot,
                        endpoint.reference().profile_feature_id,
                        &mut dependencies,
                    );
                    add_feature_dependency_closure(
                        snapshot,
                        endpoint.reference().producer_feature_id,
                        &mut dependencies,
                    );
                }
            }
            CanonicalCommand::SetAssemblyMateKind { id, .. } => {
                dependencies.insert(AuthoritativeDependency::AssemblyMate(*id));
                if let Some(mate) = snapshot.assembly_mate(*id) {
                    for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
                        dependencies.insert(AuthoritativeDependency::Occurrence(
                            endpoint.occurrence_id(),
                        ));
                        add_feature_dependency_closure(
                            snapshot,
                            endpoint.reference().producer_feature_id,
                            &mut dependencies,
                        );
                    }
                }
            }
            CanonicalCommand::DeleteAssemblyMate { id } => {
                dependencies.insert(AuthoritativeDependency::AssemblyMate(*id));
            }
            CanonicalCommand::CreateAssemblyJoint(joint) => {
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(joint.id()));
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    joint.parent_occurrence_id(),
                ));
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    joint.child_occurrence_id(),
                ));
                dependencies.extend(
                    snapshot
                        .assembly_joints()
                        .map(|joint| AuthoritativeDependency::AssemblyJoint(joint.id())),
                );
            }
            CanonicalCommand::SetAssemblyJointKind { id, .. }
            | CanonicalCommand::SetAssemblyJointPosition { id, .. }
            | CanonicalCommand::SetAssemblyJointLimits { id, .. } => {
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(*id));
                if let Some(joint) = snapshot.assembly_joint(*id) {
                    dependencies.insert(AuthoritativeDependency::Occurrence(
                        joint.parent_occurrence_id(),
                    ));
                    dependencies.insert(AuthoritativeDependency::Occurrence(
                        joint.child_occurrence_id(),
                    ));
                }
                dependencies.extend(
                    snapshot
                        .assembly_joints()
                        .map(|joint| AuthoritativeDependency::AssemblyJoint(joint.id())),
                );
                dependencies.extend(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| AuthoritativeDependency::AssemblyMotionStudy(study.id())),
                );
                dependencies.extend(snapshot.assembly_motion_couplings().map(|coupling| {
                    AuthoritativeDependency::AssemblyMotionCoupling(coupling.id())
                }));
            }
            CanonicalCommand::DeleteAssemblyJoint { id } => {
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(*id));
                dependencies.extend(
                    snapshot
                        .assembly_motion_studies()
                        .map(|study| AuthoritativeDependency::AssemblyMotionStudy(study.id())),
                );
                dependencies.extend(snapshot.assembly_motion_couplings().map(|coupling| {
                    AuthoritativeDependency::AssemblyMotionCoupling(coupling.id())
                }));
            }
            CanonicalCommand::CreateAssemblyMotionCoupling(coupling)
            | CanonicalCommand::UpdateAssemblyMotionCoupling(coupling) => {
                dependencies.insert(AuthoritativeDependency::AssemblyMotionCoupling(
                    coupling.id(),
                ));
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(
                    coupling.input_joint_id(),
                ));
                dependencies.insert(AuthoritativeDependency::AssemblyJoint(
                    coupling.output_joint_id(),
                ));
                dependencies.extend(
                    snapshot
                        .assembly_motion_couplings()
                        .map(|value| AuthoritativeDependency::AssemblyMotionCoupling(value.id())),
                );
            }
            CanonicalCommand::DeleteAssemblyMotionCoupling { id } => {
                dependencies.insert(AuthoritativeDependency::AssemblyMotionCoupling(*id));
            }
            CanonicalCommand::CreateAssemblyMotionStudy(study)
            | CanonicalCommand::UpdateAssemblyMotionStudy(study) => {
                dependencies.insert(AuthoritativeDependency::AssemblyMotionStudy(study.id()));
                dependencies.extend(
                    study
                        .drivers()
                        .iter()
                        .map(|driver| AuthoritativeDependency::AssemblyJoint(driver.joint_id())),
                );
            }
            CanonicalCommand::DeleteAssemblyMotionStudy { id } => {
                dependencies.insert(AuthoritativeDependency::AssemblyMotionStudy(*id));
            }
            CanonicalCommand::CreateMechanicalInterface(interface)
            | CanonicalCommand::UpdateMechanicalInterface(interface) => {
                dependencies.insert(AuthoritativeDependency::MechanicalInterface(interface.id()));
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    interface.occurrence_id(),
                ));
            }
            CanonicalCommand::DeleteMechanicalInterface { id } => {
                dependencies.insert(AuthoritativeDependency::MechanicalInterface(*id));
                dependencies.extend(
                    snapshot.mechanical_conditions().map(|condition| {
                        AuthoritativeDependency::MechanicalCondition(condition.id())
                    }),
                );
            }
            CanonicalCommand::CreateMechanicalCondition(condition)
            | CanonicalCommand::UpdateMechanicalCondition(condition) => {
                dependencies.insert(AuthoritativeDependency::MechanicalCondition(condition.id()));
                dependencies.extend(
                    condition
                        .kind()
                        .interfaces()
                        .into_iter()
                        .map(AuthoritativeDependency::MechanicalInterface),
                );
                dependencies.extend(
                    condition
                        .kind()
                        .joint_id()
                        .map(AuthoritativeDependency::AssemblyJoint),
                );
            }
            CanonicalCommand::DeleteMechanicalCondition { id } => {
                dependencies.insert(AuthoritativeDependency::MechanicalCondition(*id));
            }
            CanonicalCommand::CreateDrawingSheet(sheet)
            | CanonicalCommand::UpdateDrawingSheet(sheet) => {
                dependencies.insert(AuthoritativeDependency::DrawingSheet(sheet.id()));
                match sheet.source() {
                    DrawingSource::Definition(id) => {
                        dependencies.insert(AuthoritativeDependency::Definition(*id));
                        if let Some(definition) = snapshot.definition(*id) {
                            dependencies.extend(
                                definition
                                    .feature_ids()
                                    .iter()
                                    .copied()
                                    .map(AuthoritativeDependency::Feature),
                            );
                        }
                    }
                    DrawingSource::RigidAssembly { occurrence_ids } => {
                        for occurrence_id in occurrence_ids {
                            if let Some(occurrence) = snapshot.occurrence(*occurrence_id)
                                && let Some(definition) =
                                    snapshot.definition(occurrence.definition_id())
                            {
                                dependencies.insert(AuthoritativeDependency::Definition(
                                    occurrence.definition_id(),
                                ));
                                dependencies.extend(
                                    definition
                                        .feature_ids()
                                        .iter()
                                        .copied()
                                        .map(AuthoritativeDependency::Feature),
                                );
                            }
                        }
                        dependencies.extend(
                            occurrence_ids
                                .iter()
                                .cloned()
                                .map(AuthoritativeDependency::Occurrence),
                        );
                        dependencies.extend(
                            occurrence_ids
                                .iter()
                                .cloned()
                                .map(AuthoritativeDependency::GroundedOccurrence),
                        );
                        dependencies.extend(
                            snapshot
                                .assembly_mates()
                                .filter(|mate| {
                                    occurrence_ids.contains(&mate.endpoint_a().occurrence_id())
                                        || occurrence_ids
                                            .contains(&mate.endpoint_b().occurrence_id())
                                })
                                .map(|mate| AuthoritativeDependency::AssemblyMate(mate.id())),
                        );
                    }
                    DrawingSource::RigidAssemblyInstances { instance_paths } => {
                        dependencies.extend(instance_paths.iter().map(|path| {
                            AuthoritativeDependency::Occurrence(path.root_occurrence())
                        }));
                        dependencies.extend(instance_paths.iter().map(|path| {
                            AuthoritativeDependency::GroundedOccurrence(path.root_occurrence())
                        }));
                        for path in instance_paths {
                            if let Some(root) = snapshot.occurrence(path.root_occurrence()) {
                                dependencies.insert(AuthoritativeDependency::Definition(
                                    root.definition_id(),
                                ));
                                if let Some(definition) = snapshot.definition(root.definition_id())
                                {
                                    dependencies.extend(
                                        definition
                                            .feature_ids()
                                            .iter()
                                            .copied()
                                            .map(AuthoritativeDependency::Feature),
                                    );
                                }
                            }
                            let mut prefix = InstancePath::root(path.root_occurrence());
                            for step in path.steps() {
                                prefix = prefix.with_step(*step);
                                if matches!(step, InstancePathStep::Occurrence(_))
                                    && let Ok(resolved) = snapshot.resolve_instance_path(&prefix)
                                {
                                    dependencies.insert(AuthoritativeDependency::Definition(
                                        resolved.definition_id,
                                    ));
                                    if let Some(definition) =
                                        snapshot.definition(resolved.definition_id)
                                    {
                                        dependencies.extend(
                                            definition
                                                .feature_ids()
                                                .iter()
                                                .copied()
                                                .map(AuthoritativeDependency::Feature),
                                        );
                                    }
                                }
                            }
                        }
                        dependencies.extend(
                            snapshot
                                .assembly_mates()
                                .filter(|mate| {
                                    instance_paths.contains(mate.endpoint_a().instance_path())
                                        || instance_paths
                                            .contains(mate.endpoint_b().instance_path())
                                })
                                .map(|mate| AuthoritativeDependency::AssemblyMate(mate.id())),
                        );
                    }
                }
            }
            CanonicalCommand::DeleteDrawingSheet { id } => {
                dependencies.insert(AuthoritativeDependency::DrawingSheet(*id));
            }
            CanonicalCommand::CreateOccurrence {
                id,
                definition_id,
                parent,
                tag,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
                if let Some(tag_id) = tag {
                    dependencies.insert(AuthoritativeDependency::Tag(*tag_id));
                }
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::DeleteOccurrence { id } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::OccurrenceCollections(*id));
            }
            CanonicalCommand::SetOccurrenceTransform { id, .. }
            | CanonicalCommand::RenameEntity { id, .. }
            | CanonicalCommand::SetOccurrenceColor { id, .. }
            | CanonicalCommand::SetOccurrenceVisibility { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
            }
            CanonicalCommand::SetOccurrenceTag { id, tag } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                if let Some(tag_id) = tag {
                    dependencies.insert(AuthoritativeDependency::Tag(*tag_id));
                }
            }
            CanonicalCommand::RepointOccurrence { id, definition_id } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                dependencies.insert(AuthoritativeDependency::Definition(*definition_id));
            }
            CanonicalCommand::SetOccurrenceParent { id, parent } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::CreateGroup { id, parent, .. } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::DeleteGroup { id } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                dependencies.insert(AuthoritativeDependency::GroupChildren(*id));
            }
            CanonicalCommand::SetGroupTransform { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
            }
            CanonicalCommand::SetGroupParent { id, parent } => {
                dependencies.insert(AuthoritativeDependency::Group(*id));
                add_group_ancestry(snapshot, *parent, &mut dependencies);
            }
            CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                dependencies.insert(AuthoritativeDependency::OccurrenceCollections(
                    plan.occurrence_id,
                ));
                dependencies.insert(AuthoritativeDependency::AssemblyRecipe);
                let path = InstancePath::root(plan.occurrence_id);
                for joint in snapshot.pin_joints().filter(|joint| {
                    joint.physical_hole_pairs.is_some()
                        && (joint.first.instance_path == path || joint.second.instance_path == path)
                }) {
                    dependencies.insert(AuthoritativeDependency::PinJoint(joint.id));
                }
                dependencies.insert(AuthoritativeDependency::Occurrence(plan.occurrence_id));
                dependencies.insert(AuthoritativeDependency::Definition(
                    plan.source_definition_id,
                ));
                dependencies.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                for (source_id, new_id) in &plan.feature_id_map {
                    add_feature_dependency_closure(snapshot, *source_id, &mut dependencies);
                    dependencies.insert(AuthoritativeDependency::FeatureParameterBindings(
                        *source_id,
                    ));
                    dependencies.insert(AuthoritativeDependency::Feature(*new_id));
                }
                if let Some(definition) = snapshot.definition(plan.source_definition_id) {
                    for local_id in definition.local_group_ids() {
                        dependencies.insert(AuthoritativeDependency::LocalGroup(LocalGroupKey {
                            definition_id: plan.source_definition_id,
                            local_id: *local_id,
                        }));
                    }
                    for local_id in definition.local_occurrence_ids() {
                        dependencies.insert(AuthoritativeDependency::LocalOccurrence(
                            LocalOccurrenceKey {
                                definition_id: plan.source_definition_id,
                                local_id: *local_id,
                            },
                        ));
                    }
                }
            }
            CanonicalCommand::ConvertGroupToComponent(plan) => {
                dependencies.insert(AuthoritativeDependency::GroupSubtree(plan.group_id));
                dependencies.insert(AuthoritativeDependency::Definition(plan.new_definition_id));
                dependencies.insert(AuthoritativeDependency::Occurrence(plan.new_occurrence_id));
            }
            CanonicalCommand::ApplySolidTool(plan) => {
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    plan.target_occurrence_id,
                ));
                dependencies.insert(AuthoritativeDependency::Occurrence(plan.tool_occurrence_id));
                dependencies.insert(AuthoritativeDependency::OccurrenceCollections(
                    plan.tool_occurrence_id,
                ));
                add_feature_dependency_closure(snapshot, plan.target_feature_id, &mut dependencies);
                add_feature_dependency_closure(snapshot, plan.tool_feature_id, &mut dependencies);
                dependencies.insert(AuthoritativeDependency::Definition(
                    plan.result_definition_id,
                ));
                dependencies.extend(
                    plan.result_feature_ids
                        .iter()
                        .cloned()
                        .map(AuthoritativeDependency::Feature),
                );
            }
            CanonicalCommand::CreateExpressionNode { id, expression, .. } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
                if let Ok(expression) = ExpressionAst::parse(expression) {
                    for dependency in expression.dependencies() {
                        add_evaluator_dependency_closure(snapshot, dependency, &mut dependencies);
                    }
                }
            }
            CanonicalCommand::CreateRuleNode { id, expression, .. } => {
                dependencies.insert(AuthoritativeDependency::EvaluatorNode(*id));
                if let Ok(expression) = ExpressionAst::parse(expression) {
                    for dependency in expression.dependencies() {
                        add_evaluator_dependency_closure(snapshot, dependency, &mut dependencies);
                    }
                }
            }
            CanonicalCommand::SetNodeExpression { id, expression } => {
                add_evaluator_dependency_closure(snapshot, *id, &mut dependencies);
                if let Ok(expression) = ExpressionAst::parse(expression) {
                    for dependency in expression.dependencies() {
                        add_evaluator_dependency_closure(snapshot, dependency, &mut dependencies);
                    }
                }
            }
            CanonicalCommand::SetRuleOutputs { id, .. } => {
                add_evaluator_dependency_closure(snapshot, *id, &mut dependencies);
            }
            CanonicalCommand::UpsertOverride(value) => {
                dependencies.insert(AuthoritativeDependency::Override(value.id));
                add_evaluator_dependency_closure(
                    snapshot,
                    value.target.root_rule_node_id,
                    &mut dependencies,
                );
            }
            CanonicalCommand::DeleteOverride { id } => {
                dependencies.insert(AuthoritativeDependency::Override(*id));
            }
            CanonicalCommand::UpsertFeatureParameterBinding(binding) => {
                dependencies.insert(AuthoritativeDependency::FeatureParameterBinding(
                    binding.target.clone(),
                ));
                add_feature_dependency_closure(
                    snapshot,
                    binding.target.feature_id,
                    &mut dependencies,
                );
                add_evaluator_dependency_closure(
                    snapshot,
                    binding.derived_from.root_rule_node_id,
                    &mut dependencies,
                );
            }
            CanonicalCommand::DeleteFeatureParameterBinding { target } => {
                dependencies.insert(AuthoritativeDependency::FeatureParameterBinding(
                    target.clone(),
                ));
            }
            CanonicalCommand::RecomputeFeatureParameters { scope, .. } => {
                for binding in feature_parameter_bindings_in_scope(snapshot, scope) {
                    dependencies.insert(AuthoritativeDependency::FeatureParameterBinding(
                        binding.target.clone(),
                    ));
                    add_feature_dependency_closure(
                        snapshot,
                        binding.target.feature_id,
                        &mut dependencies,
                    );
                    add_evaluator_dependency_closure(
                        snapshot,
                        binding.derived_from.root_rule_node_id,
                        &mut dependencies,
                    );
                }
            }
            CanonicalCommand::UpsertJoint(joint) => {
                dependencies.insert(AuthoritativeDependency::Joint(joint.id()));
                add_evaluator_dependency_closure(
                    snapshot,
                    joint.participant_a().root_rule_node_id,
                    &mut dependencies,
                );
                add_evaluator_dependency_closure(
                    snapshot,
                    joint.participant_b().root_rule_node_id,
                    &mut dependencies,
                );
            }
            CanonicalCommand::DeleteJoint { id } => {
                dependencies.insert(AuthoritativeDependency::Joint(*id));
            }
            CanonicalCommand::UpsertSpace(space) => {
                dependencies.insert(AuthoritativeDependency::Space(space.id()));
                dependencies.extend(
                    space
                        .adjacent_to()
                        .iter()
                        .chain(space.accessible_to())
                        .cloned()
                        .map(AuthoritativeDependency::Space),
                );
            }
            CanonicalCommand::DeleteSpace { id } => {
                dependencies.insert(AuthoritativeDependency::Space(*id));
            }
            CanonicalCommand::UpsertClearanceVolume(clearance) => {
                dependencies.insert(AuthoritativeDependency::ClearanceVolume(clearance.id()));
                match clearance.owner() {
                    ClearanceOwner::Occurrence(path) => {
                        dependencies
                            .insert(AuthoritativeDependency::Occurrence(path.root_occurrence()));
                    }
                    ClearanceOwner::Space(id) => {
                        dependencies.insert(AuthoritativeDependency::Space(*id));
                    }
                }
                if let Some(identity) = clearance.derived_from() {
                    add_evaluator_dependency_closure(
                        snapshot,
                        identity.root_rule_node_id,
                        &mut dependencies,
                    );
                }
            }
            CanonicalCommand::DeleteClearanceVolume { id } => {
                dependencies.insert(AuthoritativeDependency::ClearanceVolume(*id));
            }
            CanonicalCommand::UpsertCamPlan(plan) => {
                dependencies.insert(AuthoritativeDependency::CamPlan(plan.id()));
                add_feature_dependency_closure(
                    snapshot,
                    plan.target().feature_id,
                    &mut dependencies,
                );
            }
            CanonicalCommand::DeleteCamPlan { id } => {
                dependencies.insert(AuthoritativeDependency::CamPlan(*id));
            }
            CanonicalCommand::UpsertPinJoint(joint) => {
                dependencies.insert(AuthoritativeDependency::PinJoint(joint.id));
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    joint.first.instance_path.root_occurrence(),
                ));
                dependencies.insert(AuthoritativeDependency::Occurrence(
                    joint.second.instance_path.root_occurrence(),
                ));
            }
            CanonicalCommand::DeletePinJoint { id } => {
                dependencies.insert(AuthoritativeDependency::PinJoint(*id));
            }
            CanonicalCommand::SetAssemblyRecipe(recipe) => {
                dependencies.insert(AuthoritativeDependency::AssemblyRecipe);
                dependencies.extend(recipe.parts.values().flat_map(|part| {
                    [
                        AuthoritativeDependency::Occurrence(part.instance_path.root_occurrence()),
                        AuthoritativeDependency::Definition(part.definition_id),
                    ]
                }));
                dependencies.extend(
                    recipe
                        .owned_features
                        .values()
                        .map(|owned| AuthoritativeDependency::Feature(owned.feature_id)),
                );
                dependencies.extend(
                    recipe
                        .joinery
                        .values()
                        .map(|item| AuthoritativeDependency::PinJoint(item.pin_joint_id)),
                );
            }
            CanonicalCommand::ClearAssemblyRecipe => {
                dependencies.insert(AuthoritativeDependency::AssemblyRecipe);
            }
            CanonicalCommand::UpsertPersistentDimension(dimension) => {
                dependencies.insert(AuthoritativeDependency::PersistentDimension(dimension.id));
                match &dimension.target {
                    PersistentDimensionTarget::FeatureParameter(target) => {
                        add_feature_dependency_closure(
                            snapshot,
                            target.feature_id,
                            &mut dependencies,
                        );
                    }
                    PersistentDimensionTarget::DerivedOutput(target) => {
                        add_evaluator_dependency_closure(
                            snapshot,
                            target.root_rule_node_id,
                            &mut dependencies,
                        );
                    }
                    PersistentDimensionTarget::ExactFeatureParameter { .. } => {}
                }
            }
            CanonicalCommand::DeletePersistentDimension { id } => {
                dependencies.insert(AuthoritativeDependency::PersistentDimension(*id));
            }
            CanonicalCommand::CreateTag { id, .. }
            | CanonicalCommand::DeleteTag { id }
            | CanonicalCommand::SetTagVisibility { id, .. }
            | CanonicalCommand::SetTagName { id, .. } => {
                dependencies.insert(AuthoritativeDependency::Tag(*id));
            }
            CanonicalCommand::UpsertClassificationDimension { id, .. } => {
                dependencies.insert(AuthoritativeDependency::ClassificationDimension(*id));
                dependencies.extend(
                    snapshot
                        .product
                        .classification_assignments
                        .keys()
                        .filter(|(_, dimension_id)| dimension_id == id)
                        .map(|(occurrence_id, dimension_id)| {
                            AuthoritativeDependency::OccurrenceClassification(
                                *occurrence_id,
                                *dimension_id,
                            )
                        }),
                );
            }
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id,
                dimension_id,
                ..
            } => {
                dependencies.insert(AuthoritativeDependency::Occurrence(*occurrence_id));
                dependencies.insert(AuthoritativeDependency::ClassificationDimension(
                    *dimension_id,
                ));
                dependencies.insert(AuthoritativeDependency::OccurrenceClassification(
                    *occurrence_id,
                    *dimension_id,
                ));
            }
            CanonicalCommand::CreateCollection { id, .. }
            | CanonicalCommand::DeleteCollection { id } => {
                dependencies.insert(AuthoritativeDependency::Collection(*id));
            }
            CanonicalCommand::SetCollectionOccurrences { id, occurrence_ids } => {
                dependencies.insert(AuthoritativeDependency::Collection(*id));
                dependencies.extend(
                    occurrence_ids
                        .iter()
                        .cloned()
                        .map(AuthoritativeDependency::Occurrence),
                );
            }
        }
    }
    dependencies
}

pub(super) fn add_evaluator_dependency_closure(
    snapshot: &Snapshot,
    id: NodeId,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    if !dependencies.insert(AuthoritativeDependency::EvaluatorNode(id)) {
        return;
    }
    if let Some(node) = snapshot.evaluator_node(id) {
        for dependency in node.dependencies() {
            add_evaluator_dependency_closure(snapshot, *dependency, dependencies);
        }
    }
}

pub(super) fn add_feature_dependency_closure(
    snapshot: &Snapshot,
    id: FeatureId,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    if !dependencies.insert(AuthoritativeDependency::Feature(id)) {
        return;
    }
    if let Some(feature) = snapshot.feature(id) {
        dependencies.insert(AuthoritativeDependency::Definition(feature.definition_id()));
        match feature.kind() {
            FeatureKind::Workplane(spec) => match &spec.support {
                WorkplaneSupport::Free | WorkplaneSupport::Principal(_) => {}
                WorkplaneSupport::Offset { base, .. }
                | WorkplaneSupport::ConstructionPlane { feature: base } => {
                    add_feature_dependency_closure(snapshot, *base, dependencies);
                }
                WorkplaneSupport::PlanarFace { reference, .. } => {
                    add_feature_dependency_closure(
                        snapshot,
                        reference.producer_feature_id,
                        dependencies,
                    );
                }
            },
            FeatureKind::Sketch(spec) => {
                add_feature_dependency_closure(snapshot, spec.workplane, dependencies);
            }
            FeatureKind::Pad(spec) => {
                if let Some(target) = spec.operation.target() {
                    add_feature_dependency_closure(snapshot, target, dependencies);
                }
                add_feature_dependency_closure(snapshot, spec.profile.feature_id(), dependencies);
            }
            FeatureKind::Revolve { profile, .. }
            | FeatureKind::PlanarOffset { profile, .. }
            | FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile }) => {
                add_feature_dependency_closure(snapshot, *profile, dependencies);
            }
            FeatureKind::Sweep { profile, path } => {
                add_feature_dependency_closure(snapshot, *profile, dependencies);
                add_feature_dependency_closure(snapshot, *path, dependencies);
            }
            FeatureKind::WeldmentMember(spec) => {
                add_feature_dependency_closure(snapshot, spec.profile, dependencies);
                add_feature_dependency_closure(snapshot, spec.path, dependencies);
            }
            FeatureKind::WeldmentJoint(spec) => {
                add_feature_dependency_closure(snapshot, spec.first_member, dependencies);
                add_feature_dependency_closure(snapshot, spec.second_member, dependencies);
            }
            FeatureKind::Loft {
                sections, guide, ..
            }
            | FeatureKind::SurfaceBody(SurfaceBodySpec::Loft {
                sections, guide, ..
            }) => {
                for source in sections.iter().map(|section| section.profile).chain(*guide) {
                    add_feature_dependency_closure(snapshot, source, dependencies);
                }
            }
            FeatureKind::Boolean { target, tool, .. }
            | FeatureKind::SurfaceTrim {
                target,
                cutter: tool,
            } => {
                add_feature_dependency_closure(snapshot, *target, dependencies);
                add_feature_dependency_closure(snapshot, *tool, dependencies);
            }
            FeatureKind::Shell { target, .. }
            | FeatureKind::EdgeFinish { target, .. }
            | FeatureKind::FaceOffset { target, .. }
            | FeatureKind::SurfaceExtend { target, .. }
            | FeatureKind::SurfaceThicken { target, .. }
            | FeatureKind::RigidTransform { target, .. } => {
                add_feature_dependency_closure(snapshot, *target, dependencies);
            }
            FeatureKind::SurfaceKnit { surfaces, .. } => {
                for surface in surfaces {
                    add_feature_dependency_closure(snapshot, *surface, dependencies);
                }
            }
            FeatureKind::Profile { .. }
            | FeatureKind::SheetMetal(_)
            | FeatureKind::SpatialPath { .. }
            | FeatureKind::ConstructionPoint { .. }
            | FeatureKind::ConstructionAxis { .. }
            | FeatureKind::ConstructionPlane { .. }
            | FeatureKind::ImportedExactBody(_)
            | FeatureKind::MeshBody(_) => {}
        }
    }
}

pub(super) fn add_group_ancestry(
    snapshot: &Snapshot,
    mut group_id: Option<GroupId>,
    dependencies: &mut BTreeSet<AuthoritativeDependency>,
) {
    while let Some(id) = group_id {
        if !dependencies.insert(AuthoritativeDependency::Group(id)) {
            break;
        }
        group_id = snapshot.group(id).and_then(Group::parent);
    }
}

pub(super) fn dependency_digest(
    snapshot: &Snapshot,
    dependencies: &BTreeSet<AuthoritativeDependency>,
) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.authoritative-dependencies.v2");
    digest.value(&dependencies.len());
    for dependency in dependencies {
        digest.authoritative_dependency(snapshot.product(), dependency);
    }
    digest.finish()
}

pub(super) fn dependent_closure(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
    changed: &BTreeSet<NodeId>,
) -> BTreeSet<NodeId> {
    let mut closure = changed.clone();
    loop {
        let before = closure.len();
        for (id, node) in nodes {
            if node
                .dependencies
                .iter()
                .any(|dependency| closure.contains(dependency))
            {
                closure.insert(*id);
            }
        }
        if closure.len() == before {
            return closure;
        }
    }
}

pub(crate) fn validate_graph(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
) -> Result<(), CanonicalError> {
    validate_typed_graph(nodes).map_err(CanonicalError::Graph)
}
