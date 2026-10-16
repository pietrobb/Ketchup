//! One command of a batch applied to the product the batch edits, dispatched by
//! the family of model state it changes.
use super::*;

mod annotations;
mod assembly_relations;
mod definitions_and_bodies;
mod evaluation;
mod features;
mod occurrences;
mod organization;
mod sketch_constraints;
mod sketch_entities;

/// The product a batch edits, with what its commands record along the way.
pub(super) struct BatchEdit {
    /// The revision the batch applies to.
    pub(super) current: Snapshot,
    pub(super) product: ProductModel,
    pub(super) explicit_dirty_features: BTreeSet<FeatureId>,
    pub(super) evaluation_identity: EvaluationIdentity,
    pub(super) previous_evaluation: Option<EvaluationReport>,
    /// The definitions each coded instance path resolved to before the batch.
    pub(super) production_anchors: BTreeMap<InstancePath, Option<Vec<DefinitionId>>>,
    /// Coded paths a command made resolve to other definitions.
    pub(super) rebound_production_paths: BTreeSet<InstancePath>,
}

impl BatchEdit {
    pub(super) fn apply(&mut self, command: &CanonicalCommand) -> Result<(), CanonicalError> {
        match command {
            CanonicalCommand::SetFloorHeight { .. }
            | CanonicalCommand::SetContactJoints { .. }
            | CanonicalCommand::SetGroundedInstances { .. }
            | CanonicalCommand::SetOccurrenceGrounded { .. }
            | CanonicalCommand::SetTolerance { .. }
            | CanonicalCommand::SetProductionCode { .. }
            | CanonicalCommand::CreateEvaluatorNode { .. }
            | CanonicalCommand::DeleteEvaluatorNode { .. }
            | CanonicalCommand::SetEvaluatorDimension { .. }
            | CanonicalCommand::RenameEvaluatorNode { .. }
            | CanonicalCommand::CreateExpressionNode { .. }
            | CanonicalCommand::CreateRuleNode { .. }
            | CanonicalCommand::SetNodeExpression { .. }
            | CanonicalCommand::SetRuleOutputs { .. }
            | CanonicalCommand::UpsertOverride(..)
            | CanonicalCommand::DeleteOverride { .. }
            | CanonicalCommand::UpsertFeatureParameterBinding(..)
            | CanonicalCommand::DeleteFeatureParameterBinding { .. }
            | CanonicalCommand::RecomputeFeatureParameters { .. } => {
                evaluation::apply(self, command)?
            }
            CanonicalCommand::UpsertJoint(..)
            | CanonicalCommand::DeleteJoint { .. }
            | CanonicalCommand::UpsertSpace(..)
            | CanonicalCommand::DeleteSpace { .. }
            | CanonicalCommand::UpsertClearanceVolume(..)
            | CanonicalCommand::DeleteClearanceVolume { .. }
            | CanonicalCommand::UpsertCamPlan(..)
            | CanonicalCommand::DeleteCamPlan { .. }
            | CanonicalCommand::UpsertPinJoint(..)
            | CanonicalCommand::DeletePinJoint { .. }
            | CanonicalCommand::SetAssemblyRecipe(..)
            | CanonicalCommand::ClearAssemblyRecipe
            | CanonicalCommand::UpsertPersistentDimension(..)
            | CanonicalCommand::DeletePersistentDimension { .. }
            | CanonicalCommand::CreateDrawingSheet(..)
            | CanonicalCommand::UpdateDrawingSheet(..)
            | CanonicalCommand::DeleteDrawingSheet { .. } => annotations::apply(self, command)?,
            CanonicalCommand::CreateTag { .. }
            | CanonicalCommand::DeleteTag { .. }
            | CanonicalCommand::UpsertSavedView(..)
            | CanonicalCommand::DeleteSavedView { .. }
            | CanonicalCommand::SetTagVisibility { .. }
            | CanonicalCommand::SetTagName { .. }
            | CanonicalCommand::UpsertClassificationDimension { .. }
            | CanonicalCommand::SetOccurrenceClassification { .. }
            | CanonicalCommand::CreateCollection { .. }
            | CanonicalCommand::DeleteCollection { .. }
            | CanonicalCommand::SetCollectionOccurrences { .. }
            | CanonicalCommand::RecordImport(..) => organization::apply(self, command)?,
            CanonicalCommand::CreateDefinition { .. }
            | CanonicalCommand::DeleteDefinition { .. }
            | CanonicalCommand::RenameDefinition { .. }
            | CanonicalCommand::CreateBody { .. }
            | CanonicalCommand::DeleteBody { .. }
            | CanonicalCommand::RenameBody { .. }
            | CanonicalCommand::SetActiveBody { .. }
            | CanonicalCommand::SetBodyVisibility { .. }
            | CanonicalCommand::ConsumeBody { .. }
            | CanonicalCommand::SetFeatureBodyOwnership { .. }
            | CanonicalCommand::SetBodyFeatureSuppression { .. } => {
                definitions_and_bodies::apply(self, command)?
            }
            CanonicalCommand::CreateFeature { .. }
            | CanonicalCommand::DeleteFeature { .. }
            | CanonicalCommand::SetFeatureDimension { .. }
            | CanonicalCommand::SetFeatureParameter { .. }
            | CanonicalCommand::SetHoleThread { .. }
            | CanonicalCommand::SetModeledThreads { .. }
            | CanonicalCommand::SetShaftThread { .. } => features::apply(self, command)?,
            CanonicalCommand::CreateSketchConstraint { .. }
            | CanonicalCommand::ReplaceSketchConstraint { .. }
            | CanonicalCommand::DeleteSketchConstraint { .. }
            | CanonicalCommand::SetSketchConstraintDimension { .. } => {
                sketch_constraints::apply(self, command)?
            }
            CanonicalCommand::SplitSketchEntity { .. }
            | CanonicalCommand::JoinSketchEntities { .. }
            | CanonicalCommand::TrimSketchEntity { .. }
            | CanonicalCommand::ExtendSketchEntity { .. }
            | CanonicalCommand::OffsetSketchEntity { .. }
            | CanonicalCommand::ProjectSketchEntity { .. }
            | CanonicalCommand::SetSketchEntityConstruction { .. }
            | CanonicalCommand::TranslateProfile { .. }
            | CanonicalCommand::SetProfilePoints { .. } => sketch_entities::apply(self, command)?,
            CanonicalCommand::CreateOccurrence { .. }
            | CanonicalCommand::DeleteOccurrence { .. }
            | CanonicalCommand::SetOccurrenceTransform { .. }
            | CanonicalCommand::CreateLocalOccurrence { .. }
            | CanonicalCommand::DeleteLocalOccurrence { .. }
            | CanonicalCommand::RepointLocalOccurrence { .. }
            | CanonicalCommand::SetLocalOccurrenceParent { .. }
            | CanonicalCommand::SetLocalOccurrenceTransform { .. }
            | CanonicalCommand::RenameLocalOccurrence { .. }
            | CanonicalCommand::RenameEntity { .. }
            | CanonicalCommand::SetOccurrenceColor { .. }
            | CanonicalCommand::SetLocalOccurrenceColor { .. }
            | CanonicalCommand::SetLocalOccurrenceTags { .. }
            | CanonicalCommand::SetOccurrenceVisibility { .. }
            | CanonicalCommand::SetOccurrenceTags { .. }
            | CanonicalCommand::RepointOccurrence { .. }
            | CanonicalCommand::SetOccurrenceParent { .. }
            | CanonicalCommand::CreateGroup { .. }
            | CanonicalCommand::DeleteGroup { .. }
            | CanonicalCommand::CreateLocalGroup { .. }
            | CanonicalCommand::DeleteLocalGroup { .. }
            | CanonicalCommand::SetLocalGroupTransform { .. }
            | CanonicalCommand::SetLocalGroupParent { .. }
            | CanonicalCommand::SetGroupTransform { .. }
            | CanonicalCommand::SetGroupParent { .. }
            | CanonicalCommand::CloneDefinitionAndRepoint(..)
            | CanonicalCommand::ConvertGroupToComponent(..)
            | CanonicalCommand::ApplySolidTool(..) => occurrences::apply(self, command)?,
            CanonicalCommand::GuardAssemblyRecompute { .. }
            | CanonicalCommand::ApplyAssemblySolve { .. }
            | CanonicalCommand::CreateAssemblyMate(..)
            | CanonicalCommand::RebindAssemblyMate(..)
            | CanonicalCommand::SetAssemblyMateKind { .. }
            | CanonicalCommand::DeleteAssemblyMate { .. }
            | CanonicalCommand::CreateAssemblyJoint(..)
            | CanonicalCommand::SetAssemblyJointKind { .. }
            | CanonicalCommand::SetAssemblyJointPosition { .. }
            | CanonicalCommand::SetAssemblyJointLimits { .. }
            | CanonicalCommand::DeleteAssemblyJoint { .. }
            | CanonicalCommand::CreateAssemblyMotionCoupling(..)
            | CanonicalCommand::UpdateAssemblyMotionCoupling(..)
            | CanonicalCommand::DeleteAssemblyMotionCoupling { .. }
            | CanonicalCommand::CreateAssemblyMotionStudy(..)
            | CanonicalCommand::UpdateAssemblyMotionStudy(..)
            | CanonicalCommand::DeleteAssemblyMotionStudy { .. }
            | CanonicalCommand::CreateMechanicalInterface(..)
            | CanonicalCommand::UpdateMechanicalInterface(..)
            | CanonicalCommand::DeleteMechanicalInterface { .. }
            | CanonicalCommand::CreateMechanicalCondition(..)
            | CanonicalCommand::UpdateMechanicalCondition(..)
            | CanonicalCommand::DeleteMechanicalCondition { .. } => {
                assembly_relations::apply(self, command)?
            }
        }
        for (path, identity) in &self.production_anchors {
            if production_code_identity(&self.product, path).as_ref() != identity.as_ref() {
                self.rebound_production_paths.insert(path.clone());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Family = fn(&mut BatchEdit, &CanonicalCommand) -> Result<(), CanonicalError>;

    /// Review 2026-10-15 status PS-11: every command family ended in `_ => {}`, so a
    /// command the dispatcher sent to the wrong family changed nothing and the batch
    /// still succeeded.
    #[test]
    fn a_command_sent_to_a_family_that_does_not_apply_it_is_refused() {
        let current = DocumentStore::new().current();
        let mut edit = BatchEdit {
            product: current.product.as_ref().clone(),
            current,
            explicit_dirty_features: BTreeSet::new(),
            evaluation_identity: EvaluationIdentity::default(),
            previous_evaluation: None,
            production_anchors: BTreeMap::new(),
            rebound_production_paths: BTreeSet::new(),
        };
        let tag = CanonicalCommand::DeleteTag { id: TagId(1) };
        let recipe = CanonicalCommand::ClearAssemblyRecipe;
        let families: [(&str, Family, &CanonicalCommand); 9] = [
            ("annotation", annotations::apply, &tag),
            ("assembly relation", assembly_relations::apply, &recipe),
            (
                "definition and body",
                definitions_and_bodies::apply,
                &recipe,
            ),
            ("evaluation", evaluation::apply, &recipe),
            ("feature", features::apply, &recipe),
            ("occurrence", occurrences::apply, &recipe),
            ("organization", organization::apply, &recipe),
            ("sketch constraint", sketch_constraints::apply, &recipe),
            ("sketch entity", sketch_entities::apply, &recipe),
        ];
        for (family, apply, command) in families {
            assert_eq!(
                apply(&mut edit, command),
                Err(CanonicalError::CommandOutsideFamily(family)),
                "{family}"
            );
        }
    }
}
