use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, FeatureId, FeatureKind,
    FeatureParameterTarget, InstancePath, ParameterValueType, Snapshot, Transform,
};
use crate::joinery::{
    DowelHole, DowelJointContract, DowelJointFace, DowelJointId, project_dowel_joint_contract,
};
use crate::sketch::{FeatureExtent, SketchEntity, WorkplaneFrame, WorkplaneSupport};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const ASSEMBLY_RECIPE_SCHEMA_V1: &str = "ketchup.assembly-recipe.v1";
const MAX_RECIPE_ENTRIES: usize = 16_384;
const MAX_RECIPE_KEY_BYTES: usize = 128;
const MAX_FACE_ROLE_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RecipeKey(String);

impl RecipeKey {
    pub fn new(value: impl Into<String>) -> Result<Self, AssemblyRecipeError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_RECIPE_KEY_BYTES
            || value.split('/').any(|segment| {
                segment.is_empty()
                    || !segment.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'_' | b'-')
                    })
            })
        {
            return Err(AssemblyRecipeError::InvalidKey(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipePartMobility {
    Fixed,
    Movable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecipeEditScope {
    Occurrence(InstancePath),
    SharedDefinition(DefinitionId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipeParameterUnit {
    Millimetres,
    Degrees,
    Scalar,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecipeParameter {
    pub value: f64,
    pub unit: RecipeParameterUnit,
    pub target: Option<FeatureParameterTarget>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecipePart {
    pub key: RecipeKey,
    pub instance_path: InstancePath,
    pub definition_id: DefinitionId,
    pub placement: Transform,
    pub mobility: RecipePartMobility,
    pub edit_scope: RecipeEditScope,
    pub parameters: BTreeMap<RecipeKey, RecipeParameter>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeFaceRef {
    pub part: RecipeKey,
    pub role: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipeRelationKind {
    Contact,
    Coincident,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeRelation {
    pub key: RecipeKey,
    pub kind: RecipeRelationKind,
    pub first: RecipeFaceRef,
    pub second: RecipeFaceRef,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeJoinery {
    pub key: RecipeKey,
    pub first_part: RecipeKey,
    pub second_part: RecipeKey,
    pub dowel_joint_id: DowelJointId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecognizedRecipeFeatureKind {
    Profile,
    Extrusion,
    Pad,
    Pocket,
    Workplane,
    Sketch,
    SketchPocket,
}

impl RecognizedRecipeFeatureKind {
    #[must_use]
    pub fn matches(self, kind: &FeatureKind) -> bool {
        matches!(
            (self, kind),
            (Self::Profile, FeatureKind::Profile { .. })
                | (Self::Profile, FeatureKind::SegmentProfile { .. })
                | (Self::Extrusion, FeatureKind::Extrusion { .. })
                | (Self::Pad, FeatureKind::Pad(_))
                | (Self::Pocket, FeatureKind::Pocket { .. })
                | (Self::Workplane, FeatureKind::Workplane(_))
                | (Self::Sketch, FeatureKind::Sketch(_))
                | (Self::SketchPocket, FeatureKind::SketchPocket(_))
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeOwnedFeature {
    pub key: RecipeKey,
    pub part: RecipeKey,
    pub feature_id: FeatureId,
    pub kind: RecognizedRecipeFeatureKind,
    pub canonical_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecipePartAdoption {
    pub key: RecipeKey,
    pub instance_path: InstancePath,
    pub mobility: RecipePartMobility,
    pub edit_scope: RecipeEditScope,
    pub parameters: BTreeMap<RecipeKey, RecipeParameter>,
    pub features: Vec<(RecipeKey, FeatureId, RecognizedRecipeFeatureKind)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyRecipe {
    pub(crate) schema: String,
    pub(crate) key: RecipeKey,
    pub(crate) parts: BTreeMap<RecipeKey, RecipePart>,
    pub(crate) relations: BTreeMap<RecipeKey, RecipeRelation>,
    pub(crate) joinery: BTreeMap<RecipeKey, RecipeJoinery>,
    pub(crate) owned_features: BTreeMap<RecipeKey, RecipeOwnedFeature>,
}

impl AssemblyRecipe {
    pub fn new(
        schema: impl Into<String>,
        key: RecipeKey,
        parts: BTreeMap<RecipeKey, RecipePart>,
        relations: BTreeMap<RecipeKey, RecipeRelation>,
        joinery: BTreeMap<RecipeKey, RecipeJoinery>,
        owned_features: BTreeMap<RecipeKey, RecipeOwnedFeature>,
    ) -> Result<Self, AssemblyRecipeError> {
        let recipe = Self {
            schema: schema.into(),
            key,
            parts,
            relations,
            joinery,
            owned_features,
        };
        recipe.validate_structure()?;
        Ok(recipe)
    }

    pub fn adopt(
        snapshot: &Snapshot,
        key: RecipeKey,
        part_requests: Vec<RecipePartAdoption>,
        relations: Vec<RecipeRelation>,
        joinery: Vec<RecipeJoinery>,
    ) -> Result<Self, AssemblyRecipeError> {
        if part_requests.is_empty() || part_requests.len() > MAX_RECIPE_ENTRIES {
            return Err(AssemblyRecipeError::ResourceLimit);
        }
        let mut parts = BTreeMap::new();
        let mut owned_features = BTreeMap::new();
        for request in part_requests {
            let resolved = snapshot
                .resolve_instance_path(&request.instance_path)
                .map_err(|_| AssemblyRecipeError::UnresolvedPart(request.key.clone()))?;
            if request.edit_scope != RecipeEditScope::Occurrence(request.instance_path.clone())
                && request.edit_scope != RecipeEditScope::SharedDefinition(resolved.definition_id)
            {
                return Err(AssemblyRecipeError::InvalidEditScope(request.key));
            }
            let definition = snapshot
                .definition(resolved.definition_id)
                .ok_or_else(|| AssemblyRecipeError::UnresolvedPart(request.key.clone()))?;
            let declared = request
                .features
                .iter()
                .map(|(_, id, _)| *id)
                .collect::<BTreeSet<_>>();
            if declared.len() != request.features.len()
                || definition
                    .feature_ids()
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    != declared
            {
                return Err(AssemblyRecipeError::UnrecognizedFeatureSet(request.key));
            }
            for (feature_key, feature_id, kind) in request.features {
                let feature = snapshot
                    .feature(feature_id)
                    .ok_or(AssemblyRecipeError::UnrecognizedFeature(feature_id))?;
                if feature.definition_id() != resolved.definition_id
                    || !kind.matches(feature.kind())
                {
                    return Err(AssemblyRecipeError::UnrecognizedFeature(feature_id));
                }
                let owned = RecipeOwnedFeature {
                    key: feature_key.clone(),
                    part: request.key.clone(),
                    feature_id,
                    kind,
                    canonical_fingerprint: snapshot
                        .feature_canonical_fingerprint(feature_id)
                        .ok_or(AssemblyRecipeError::UnrecognizedFeature(feature_id))?,
                };
                if owned_features.insert(feature_key.clone(), owned).is_some() {
                    return Err(AssemblyRecipeError::DuplicateKey(feature_key));
                }
            }
            let part = RecipePart {
                key: request.key.clone(),
                instance_path: request.instance_path,
                definition_id: resolved.definition_id,
                placement: resolved.local_transform,
                mobility: request.mobility,
                edit_scope: request.edit_scope,
                parameters: request.parameters,
            };
            if parts.insert(request.key.clone(), part).is_some() {
                return Err(AssemblyRecipeError::DuplicateKey(request.key));
            }
        }
        let relations = keyed(relations, |value| &value.key)?;
        let joinery = keyed(joinery, |value| &value.key)?;
        let recipe = Self::new(
            ASSEMBLY_RECIPE_SCHEMA_V1,
            key,
            parts,
            relations,
            joinery,
            owned_features,
        )?;
        recipe.audit(snapshot)?;
        Ok(recipe)
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub fn key(&self) -> &RecipeKey {
        &self.key
    }

    pub fn parts(&self) -> impl Iterator<Item = &RecipePart> {
        self.parts.values()
    }

    pub fn relations(&self) -> impl Iterator<Item = &RecipeRelation> {
        self.relations.values()
    }

    pub fn joinery(&self) -> impl Iterator<Item = &RecipeJoinery> {
        self.joinery.values()
    }

    pub fn owned_features(&self) -> impl Iterator<Item = &RecipeOwnedFeature> {
        self.owned_features.values()
    }

    pub fn audit(&self, snapshot: &Snapshot) -> Result<(), AssemblyRecipeError> {
        self.validate_structure()?;
        for part in self.parts.values() {
            let resolved = snapshot
                .resolve_instance_path(&part.instance_path)
                .map_err(|_| AssemblyRecipeError::UnresolvedPart(part.key.clone()))?;
            if resolved.definition_id != part.definition_id
                || resolved.local_transform != part.placement
            {
                return Err(AssemblyRecipeError::PartChanged(part.key.clone()));
            }
            match &part.edit_scope {
                RecipeEditScope::Occurrence(path) if path == &part.instance_path => {
                    let owns_definition_features = self
                        .owned_features
                        .values()
                        .any(|owned| owned.part == part.key);
                    let shared_occurrences = snapshot
                        .scene_query()
                        .into_iter()
                        .filter(|occurrence| occurrence.definition_id == part.definition_id)
                        .count();
                    if owns_definition_features && shared_occurrences > 1 {
                        return Err(AssemblyRecipeError::InvalidEditScope(part.key.clone()));
                    }
                }
                RecipeEditScope::SharedDefinition(id) if *id == part.definition_id => {}
                _ => return Err(AssemblyRecipeError::InvalidEditScope(part.key.clone())),
            }
            for parameter in part.parameters.values() {
                if !parameter.value.is_finite() {
                    return Err(AssemblyRecipeError::InvalidParameter(part.key.clone()));
                }
                if let Some(target) = &parameter.target
                    && snapshot.feature_parameter_value(target).map(f64::to_bits)
                        != Some(parameter.value.to_bits())
                {
                    return Err(AssemblyRecipeError::InvalidParameter(part.key.clone()));
                }
            }
        }
        for owned in self.owned_features.values() {
            let part = &self.parts[&owned.part];
            let feature = snapshot
                .feature(owned.feature_id)
                .ok_or(AssemblyRecipeError::OwnedFeatureConflict(owned.key.clone()))?;
            if feature.definition_id() != part.definition_id
                || !owned.kind.matches(feature.kind())
                || snapshot
                    .feature_canonical_fingerprint(owned.feature_id)
                    .as_deref()
                    != Some(owned.canonical_fingerprint.as_str())
            {
                return Err(AssemblyRecipeError::OwnedFeatureConflict(owned.key.clone()));
            }
        }
        for item in self.joinery.values() {
            let joint = snapshot
                .dowel_joint(item.dowel_joint_id)
                .ok_or_else(|| AssemblyRecipeError::UnresolvedJoinery(item.key.clone()))?;
            let first = &self.parts[&item.first_part].instance_path;
            let second = &self.parts[&item.second_part].instance_path;
            let matches_declared_parts = (joint.first.instance_path == *first
                && joint.second.instance_path == *second)
                || (joint.first.instance_path == *second && joint.second.instance_path == *first);
            if !matches_declared_parts {
                return Err(AssemblyRecipeError::UnresolvedJoinery(item.key.clone()));
            }
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), AssemblyRecipeError> {
        if self.schema != ASSEMBLY_RECIPE_SCHEMA_V1 {
            return Err(AssemblyRecipeError::UnsupportedVersion(self.schema.clone()));
        }
        let total = self
            .parts
            .len()
            .saturating_add(self.relations.len())
            .saturating_add(self.joinery.len())
            .saturating_add(self.owned_features.len());
        if self.parts.is_empty() || total > MAX_RECIPE_ENTRIES {
            return Err(AssemblyRecipeError::ResourceLimit);
        }
        let mut part_paths = BTreeSet::new();
        for (key, part) in &self.parts {
            if key != &part.key {
                return Err(AssemblyRecipeError::NonCanonicalKey(key.clone()));
            }
            if !part_paths.insert(part.instance_path.clone()) {
                return Err(AssemblyRecipeError::InvalidEditScope(key.clone()));
            }
            for parameter in part.parameters.keys() {
                RecipeKey::new(parameter.as_str())?;
            }
        }
        for (key, relation) in &self.relations {
            if key != &relation.key
                || relation.first.part == relation.second.part
                || !self.parts.contains_key(&relation.first.part)
                || !self.parts.contains_key(&relation.second.part)
                || !valid_face_role(&relation.first.role)
                || !valid_face_role(&relation.second.role)
            {
                return Err(AssemblyRecipeError::InvalidRelation(key.clone()));
            }
        }
        let mut joinery_ids = BTreeSet::new();
        for (key, item) in &self.joinery {
            if key != &item.key
                || item.first_part == item.second_part
                || !self.parts.contains_key(&item.first_part)
                || !self.parts.contains_key(&item.second_part)
                || !joinery_ids.insert(item.dowel_joint_id)
            {
                return Err(AssemblyRecipeError::InvalidJoinery(key.clone()));
            }
        }
        let mut owned_feature_ids = BTreeSet::new();
        for (key, owned) in &self.owned_features {
            if key != &owned.key
                || !self.parts.contains_key(&owned.part)
                || !owned_feature_ids.insert(owned.feature_id)
            {
                return Err(AssemblyRecipeError::InvalidOwnership(key.clone()));
            }
        }
        for part in self.parts.values() {
            for parameter in part.parameters.values() {
                if let Some(target) = &parameter.target
                    && !self.owned_features.values().any(|owned| {
                        owned.part == part.key && owned.feature_id == target.feature_id
                    })
                {
                    return Err(AssemblyRecipeError::InvalidParameter(part.key.clone()));
                }
            }
        }
        Ok(())
    }
}

fn keyed<T: Clone>(
    values: Vec<T>,
    key: impl Fn(&T) -> &RecipeKey,
) -> Result<BTreeMap<RecipeKey, T>, AssemblyRecipeError> {
    let mut result = BTreeMap::new();
    for value in values {
        let item_key = key(&value).clone();
        if result.insert(item_key.clone(), value).is_some() {
            return Err(AssemblyRecipeError::DuplicateKey(item_key));
        }
    }
    Ok(result)
}

fn valid_face_role(role: &str) -> bool {
    !role.is_empty()
        && role.len() <= MAX_FACE_ROLE_BYTES
        && role.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipeDimensionAnchor {
    Minimum,
    Centre,
    Maximum,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RecipeSemanticChange {
    SetParameter {
        part: RecipeKey,
        parameter: RecipeKey,
        value: f64,
        anchor: RecipeDimensionAnchor,
    },
    ExtendUntilContact {
        part: RecipeKey,
        parameter: RecipeKey,
        relation: RecipeKey,
        anchor: RecipeDimensionAnchor,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecipePatchNode {
    pub key: RecipeKey,
    pub dependencies: Vec<RecipeKey>,
    pub change: RecipeSemanticChange,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipeSemanticPatch {
    pub nodes: Vec<RecipePatchNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyRecipeCompilation {
    pub batch: Option<CommandBatch>,
    pub affected_parts: BTreeSet<RecipeKey>,
}

pub fn compile_assembly_recipe_patch(
    snapshot: &Snapshot,
    patch: &RecipeSemanticPatch,
) -> Result<AssemblyRecipeCompilation, AssemblyRecipeCompileError> {
    let recipe = snapshot
        .assembly_recipe()
        .ok_or(AssemblyRecipeCompileError::RecipeMissing)?;
    recipe
        .audit(snapshot)
        .map_err(AssemblyRecipeCompileError::RecipeInvalid)?;
    if patch.nodes.len() > MAX_RECIPE_ENTRIES {
        return Err(AssemblyRecipeCompileError::ResourceLimit);
    }

    let ordered = topological_patch_order(patch)?;
    let mut targeted_parameters = BTreeSet::new();
    let mut commands = Vec::new();
    let mut affected_parts = BTreeSet::new();
    let mut updated_recipe = recipe.clone();
    let mut path_transforms = BTreeMap::<InstancePath, Transform>::new();

    for node_index in ordered {
        let node = &patch.nodes[node_index];
        let (part, parameter, value, anchor) = match &node.change {
            RecipeSemanticChange::SetParameter {
                part,
                parameter,
                value,
                anchor,
            } => (part, parameter, *value, *anchor),
            RecipeSemanticChange::ExtendUntilContact {
                part,
                parameter,
                relation,
                anchor,
            } => {
                let working =
                    preview_recipe_geometry(snapshot, recipe, &commands, &path_transforms)?;
                let value = solve_extend_until_contact(
                    &working,
                    &updated_recipe,
                    part,
                    parameter,
                    relation,
                    *anchor,
                )?;
                (part, parameter, value, *anchor)
            }
        };
        if !value.is_finite() {
            return Err(AssemblyRecipeCompileError::InvalidValue(node.key.clone()));
        }
        if !targeted_parameters.insert((part.clone(), parameter.clone())) {
            return Err(AssemblyRecipeCompileError::ConflictingWrites {
                part: part.clone(),
                parameter: parameter.clone(),
            });
        }
        let recipe_part = recipe
            .parts
            .get(part)
            .ok_or_else(|| AssemblyRecipeCompileError::PartMissing(part.clone()))?;
        let recipe_parameter = recipe_part.parameters.get(parameter).ok_or_else(|| {
            AssemblyRecipeCompileError::ParameterMissing {
                part: part.clone(),
                parameter: parameter.clone(),
            }
        })?;
        let target = recipe_parameter.target.as_ref().ok_or_else(|| {
            AssemblyRecipeCompileError::UnsupportedParameter {
                part: part.clone(),
                parameter: parameter.clone(),
            }
        })?;
        let current = snapshot.feature_parameter_value(target).ok_or_else(|| {
            AssemblyRecipeCompileError::UnsupportedParameter {
                part: part.clone(),
                parameter: parameter.clone(),
            }
        })?;
        if current.to_bits() == value.to_bits() {
            continue;
        }
        let local_axis = supported_parameter_axis(snapshot, target).ok_or_else(|| {
            AssemblyRecipeCompileError::UnsupportedGeometry {
                part: part.clone(),
                parameter: parameter.clone(),
            }
        })?;
        if value <= 1.0e-9 {
            return Err(AssemblyRecipeCompileError::InvalidValue(node.key.clone()));
        }
        let anchor_offset = match anchor {
            RecipeDimensionAnchor::Minimum => 0.0,
            RecipeDimensionAnchor::Centre => (current - value) * 0.5,
            RecipeDimensionAnchor::Maximum => current - value,
        };
        if anchor_offset != 0.0 {
            for path in affected_instance_paths(snapshot, recipe_part) {
                let base_transform = path_transforms.get(&path).copied().unwrap_or(
                    snapshot
                        .resolve_instance_path(&path)
                        .map_err(|_| AssemblyRecipeCompileError::PartChanged(part.clone()))?
                        .local_transform,
                );
                let translated = translate_in_local_axis(base_transform, local_axis, anchor_offset)
                    .ok_or_else(|| AssemblyRecipeCompileError::UnsupportedGeometry {
                        part: part.clone(),
                        parameter: parameter.clone(),
                    })?;
                path_transforms.insert(path, translated);
            }
        }
        commands.push(CanonicalCommand::SetFeatureParameter {
            target: target.clone(),
            dimension: Dimension::new(value.to_string(), value)
                .map_err(|_| AssemblyRecipeCompileError::InvalidValue(node.key.clone()))?,
        });
        updated_recipe
            .parts
            .get_mut(part)
            .expect("validated recipe part exists")
            .parameters
            .get_mut(parameter)
            .expect("validated recipe parameter exists")
            .value = value;
        affected_parts.insert(part.clone());
    }

    if commands.is_empty() {
        return Ok(AssemblyRecipeCompilation {
            batch: None,
            affected_parts,
        });
    }

    let geometry_candidate =
        preview_recipe_geometry(snapshot, recipe, &commands, &path_transforms)?;
    let JoineryRebind {
        physical_hole_commands,
        updates: joinery_updates,
    } = rebind_affected_joinery(snapshot, &geometry_candidate, recipe, &affected_parts)?;

    commands.insert(0, CanonicalCommand::ClearAssemblyRecipe);
    for (_, contract) in &joinery_updates {
        commands.insert(1, CanonicalCommand::DeleteDowelJoint { id: contract.id });
    }
    if !path_transforms.is_empty() {
        let mut root_transforms = BTreeMap::new();
        let mut instance_transforms = BTreeMap::new();
        for (path, transform) in path_transforms {
            if path.is_root() {
                root_transforms.insert(path.root_occurrence(), transform);
            } else {
                instance_transforms.insert(path, transform);
            }
        }
        commands.push(CanonicalCommand::ApplyAssemblySolve {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            transforms: root_transforms.into_iter().collect(),
            instance_transforms: instance_transforms.into_iter().collect(),
        });
    }
    commands.extend(physical_hole_commands);
    commands.extend(
        joinery_updates
            .into_iter()
            .map(|(_, contract)| CanonicalCommand::UpsertDowelJoint(contract)),
    );

    let candidate = snapshot
        .preview_batch(&CommandBatch::new(commands.clone()))
        .map_err(AssemblyRecipeCompileError::Canonical)?;
    for part in updated_recipe.parts.values_mut() {
        part.placement = candidate
            .resolve_instance_path(&part.instance_path)
            .map_err(|_| AssemblyRecipeCompileError::PartChanged(part.key.clone()))?
            .local_transform;
    }
    for owned in updated_recipe.owned_features.values_mut() {
        owned.canonical_fingerprint = candidate
            .feature_canonical_fingerprint(owned.feature_id)
            .ok_or_else(|| AssemblyRecipeCompileError::OwnedFeatureChanged(owned.key.clone()))?;
    }
    updated_recipe
        .audit(&candidate)
        .map_err(AssemblyRecipeCompileError::RecipeInvalid)?;
    commands.push(CanonicalCommand::SetAssemblyRecipe(updated_recipe));

    Ok(AssemblyRecipeCompilation {
        batch: Some(CommandBatch::new(commands)),
        affected_parts,
    })
}

fn topological_patch_order(
    patch: &RecipeSemanticPatch,
) -> Result<Vec<usize>, AssemblyRecipeCompileError> {
    let mut indices = BTreeMap::new();
    for (index, node) in patch.nodes.iter().enumerate() {
        if indices.insert(node.key.clone(), index).is_some() {
            return Err(AssemblyRecipeCompileError::DuplicateNode(node.key.clone()));
        }
    }
    let mut indegree = vec![0usize; patch.nodes.len()];
    let mut dependents = vec![BTreeSet::new(); patch.nodes.len()];
    for (index, node) in patch.nodes.iter().enumerate() {
        let mut unique = BTreeSet::new();
        for dependency in &node.dependencies {
            let dependency_index = *indices
                .get(dependency)
                .ok_or_else(|| AssemblyRecipeCompileError::DependencyMissing(dependency.clone()))?;
            if !unique.insert(dependency_index) {
                return Err(AssemblyRecipeCompileError::DuplicateDependency(
                    node.key.clone(),
                ));
            }
            indegree[index] += 1;
            dependents[dependency_index].insert(index);
        }
    }
    let mut ready = indegree
        .iter()
        .enumerate()
        .filter_map(|(index, degree)| {
            (*degree == 0).then_some((patch.nodes[index].key.clone(), index))
        })
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(patch.nodes.len());
    while let Some((_, index)) = ready.pop_first() {
        ordered.push(index);
        for dependent in dependents[index].iter().copied() {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                ready.insert((patch.nodes[dependent].key.clone(), dependent));
            }
        }
    }
    if ordered.len() != patch.nodes.len() {
        let node = indegree
            .iter()
            .enumerate()
            .filter_map(|(index, degree)| (*degree != 0).then_some(patch.nodes[index].key.clone()))
            .min()
            .expect("a rejected dependency graph contains a node");
        return Err(AssemblyRecipeCompileError::DependencyCycle(node));
    }
    Ok(ordered)
}

fn preview_recipe_geometry(
    snapshot: &Snapshot,
    recipe: &AssemblyRecipe,
    commands: &[CanonicalCommand],
    path_transforms: &BTreeMap<InstancePath, Transform>,
) -> Result<Snapshot, AssemblyRecipeCompileError> {
    let mut preview = vec![CanonicalCommand::ClearAssemblyRecipe];
    preview.extend(
        recipe
            .joinery
            .values()
            .map(|item| CanonicalCommand::DeleteDowelJoint {
                id: item.dowel_joint_id,
            }),
    );
    preview.extend_from_slice(commands);
    if !path_transforms.is_empty() {
        let mut root_transforms = BTreeMap::new();
        let mut instance_transforms = BTreeMap::new();
        for (path, transform) in path_transforms {
            if path.is_root() {
                root_transforms.insert(path.root_occurrence(), *transform);
            } else {
                instance_transforms.insert(path.clone(), *transform);
            }
        }
        preview.push(CanonicalCommand::ApplyAssemblySolve {
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            transforms: root_transforms.into_iter().collect(),
            instance_transforms: instance_transforms.into_iter().collect(),
        });
    }
    snapshot
        .preview_batch(&CommandBatch::new(preview))
        .map_err(AssemblyRecipeCompileError::Canonical)
}

struct JoineryRebind {
    physical_hole_commands: Vec<CanonicalCommand>,
    updates: Vec<(RecipeKey, DowelJointContract)>,
}

fn rebind_affected_joinery(
    snapshot: &Snapshot,
    candidate: &Snapshot,
    recipe: &AssemblyRecipe,
    affected_parts: &BTreeSet<RecipeKey>,
) -> Result<JoineryRebind, AssemblyRecipeCompileError> {
    let mut physical_hole_parameters = BTreeMap::<FeatureParameterTarget, f64>::new();
    let mut updates = Vec::new();
    for item in recipe.joinery.values() {
        if !affected_parts.contains(&item.first_part) && !affected_parts.contains(&item.second_part)
        {
            continue;
        }
        let original = snapshot.dowel_joint(item.dowel_joint_id).ok_or_else(|| {
            AssemblyRecipeCompileError::RecipeInvalid(AssemblyRecipeError::UnresolvedJoinery(
                item.key.clone(),
            ))
        })?;
        let mut updated = original.clone();
        let first_part = &recipe.parts[&item.first_part];
        let second_part = &recipe.parts[&item.second_part];
        let (contract_first_part, contract_second_part) = if original.first.instance_path
            == first_part.instance_path
            && original.second.instance_path == second_part.instance_path
        {
            (first_part, second_part)
        } else {
            (second_part, first_part)
        };
        let first_bounds = supported_part_bounds(candidate, recipe, contract_first_part)
            .ok_or_else(|| {
                AssemblyRecipeCompileError::UnsupportedDependentJoinery(item.key.clone())
            })?;
        let second_bounds = supported_part_bounds(candidate, recipe, contract_second_part)
            .ok_or_else(|| {
                AssemblyRecipeCompileError::UnsupportedDependentJoinery(item.key.clone())
            })?;
        let first_shift = rebind_joint_face(&mut updated.first, first_bounds).ok_or_else(|| {
            AssemblyRecipeCompileError::UnsupportedDependentJoinery(item.key.clone())
        })?;
        rebind_joint_face(&mut updated.second, second_bounds).ok_or_else(|| {
            AssemblyRecipeCompileError::UnsupportedDependentJoinery(item.key.clone())
        })?;
        if first_shift != 0.0 {
            let axis = inward_axis(updated.first.inward_unit_local).ok_or_else(|| {
                AssemblyRecipeCompileError::UnsupportedDependentJoinery(item.key.clone())
            })?;
            updated.first_center_local_mm[axis] += first_shift;
        }
        if let Some(bindings) = &updated.physical_hole_pairs {
            let mut geometric_contract = updated.clone();
            geometric_contract.physical_hole_pairs = None;
            let projection =
                project_dowel_joint_contract(candidate, &geometric_contract).map_err(|_| {
                    AssemblyRecipeCompileError::UnsupportedDependentJoinery(item.key.clone())
                })?;
            if bindings.len() != projection.pairs.len() {
                return Err(AssemblyRecipeCompileError::UnsupportedDependentJoinery(
                    item.key.clone(),
                ));
            }
            for (binding, pair) in bindings.iter().zip(&projection.pairs) {
                collect_physical_hole_updates(
                    candidate,
                    recipe,
                    contract_first_part,
                    binding.first_pocket_feature_id,
                    &pair.first,
                    &item.key,
                    &mut physical_hole_parameters,
                )?;
                collect_physical_hole_updates(
                    candidate,
                    recipe,
                    contract_second_part,
                    binding.second_pocket_feature_id,
                    &pair.second,
                    &item.key,
                    &mut physical_hole_parameters,
                )?;
            }
        }
        if &updated != original {
            updates.push((item.key.clone(), updated));
        }
    }
    let commands = physical_hole_parameters
        .into_iter()
        .map(|(target, value)| {
            Ok(CanonicalCommand::SetFeatureParameter {
                target,
                dimension: Dimension::new(value.to_string(), value)
                    .map_err(AssemblyRecipeCompileError::Canonical)?,
            })
        })
        .collect::<Result<Vec<_>, AssemblyRecipeCompileError>>()?;
    Ok(JoineryRebind {
        physical_hole_commands: commands,
        updates,
    })
}

fn collect_physical_hole_updates(
    snapshot: &Snapshot,
    recipe: &AssemblyRecipe,
    part: &RecipePart,
    pocket_feature_id: FeatureId,
    expected: &DowelHole,
    joinery_key: &RecipeKey,
    parameters: &mut BTreeMap<FeatureParameterTarget, f64>,
) -> Result<(), AssemblyRecipeCompileError> {
    let unsupported =
        || AssemblyRecipeCompileError::UnsupportedDependentJoinery(joinery_key.clone());
    let owns = |feature_id, kind| {
        recipe.owned_features.values().any(|owned| {
            owned.part == part.key && owned.feature_id == feature_id && owned.kind == kind
        })
    };
    let pocket = snapshot
        .feature(pocket_feature_id)
        .ok_or_else(unsupported)?;
    let FeatureKind::Pocket { profile, .. } = pocket.kind() else {
        return Err(unsupported());
    };
    if pocket.definition_id() != part.definition_id
        || !owns(pocket_feature_id, RecognizedRecipeFeatureKind::Pocket)
    {
        return Err(unsupported());
    }
    let sketch = snapshot.feature(*profile).ok_or_else(unsupported)?;
    let FeatureKind::Sketch(spec) = sketch.kind() else {
        return Err(unsupported());
    };
    let [SketchEntity::Circle { center_mm, .. }] = spec.entities.as_slice() else {
        return Err(unsupported());
    };
    if sketch.definition_id() != part.definition_id
        || !owns(*profile, RecognizedRecipeFeatureKind::Sketch)
    {
        return Err(unsupported());
    }
    let workplane = snapshot.feature(spec.workplane).ok_or_else(unsupported)?;
    let FeatureKind::Workplane(workplane_spec) = workplane.kind() else {
        return Err(unsupported());
    };
    if workplane.definition_id() != part.definition_id
        || !matches!(workplane_spec.support, WorkplaneSupport::Free)
        || !owns(spec.workplane, RecognizedRecipeFeatureKind::Workplane)
    {
        return Err(unsupported());
    }
    let frame = workplane_spec.frame;
    let observed_entry: [f64; 3] = std::array::from_fn(|axis| {
        frame.origin_mm[axis]
            + frame.x_axis[axis] * center_mm[0]
            + frame.y_axis[axis] * center_mm[1]
    });
    let desired_origin: [f64; 3] = std::array::from_fn(|axis| {
        frame.origin_mm[axis] + expected.entry_local_mm[axis] - observed_entry[axis]
    });
    for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
        if (desired_origin[axis] - frame.origin_mm[axis]).abs() <= 1.0e-8 {
            continue;
        }
        let target = FeatureParameterTarget::new(
            spec.workplane,
            format!("frame.origin.{name}"),
            ParameterValueType::Length,
        )
        .expect("static physical-hole workplane parameter path is valid");
        if parameters
            .insert(target, desired_origin[axis])
            .is_some_and(|existing| existing.to_bits() != desired_origin[axis].to_bits())
        {
            return Err(unsupported());
        }
    }
    Ok(())
}

fn rebind_joint_face(face: &mut DowelJointFace, bounds: SupportedPartBounds) -> Option<f64> {
    let axis = inward_axis(face.inward_unit_local)?;
    let old_coordinate = face.face_origin_local_mm[axis];
    let expected_old = if face.inward_unit_local[axis] > 0.0 {
        face.bounds_min_local_mm[axis]
    } else {
        face.bounds_max_local_mm[axis]
    };
    if (old_coordinate - expected_old).abs() > 1.0e-8 {
        return None;
    }
    let new_coordinate = if face.inward_unit_local[axis] > 0.0 {
        bounds.minimum[axis]
    } else {
        bounds.maximum[axis]
    };
    face.face_origin_local_mm[axis] = new_coordinate;
    face.bounds_min_local_mm = bounds.minimum;
    face.bounds_max_local_mm = bounds.maximum;
    Some(new_coordinate - old_coordinate)
}

fn inward_axis(vector: [f64; 3]) -> Option<usize> {
    let axis = (0..3).find(|axis| (vector[*axis].abs() - 1.0).abs() <= 1.0e-8)?;
    (0..3)
        .all(|other| other == axis || vector[other].abs() <= 1.0e-8)
        .then_some(axis)
}

#[derive(Clone, Copy)]
struct SupportedPartBounds {
    minimum: [f64; 3],
    maximum: [f64; 3],
}

#[derive(Clone, Copy)]
struct SupportedFace {
    axis: usize,
    maximum: bool,
    point_local: [f64; 3],
    normal_local: [f64; 3],
}

fn solve_extend_until_contact(
    snapshot: &Snapshot,
    recipe: &AssemblyRecipe,
    part: &RecipeKey,
    parameter: &RecipeKey,
    relation_key: &RecipeKey,
    anchor: RecipeDimensionAnchor,
) -> Result<f64, AssemblyRecipeCompileError> {
    let relation = recipe
        .relations
        .get(relation_key)
        .ok_or_else(|| AssemblyRecipeCompileError::RelationMissing(relation_key.clone()))?;
    if relation.kind != RecipeRelationKind::Contact {
        return Err(AssemblyRecipeCompileError::UnsupportedRelation(
            relation_key.clone(),
        ));
    }
    let (moving_ref, target_ref) = if relation.first.part == *part {
        (&relation.first, &relation.second)
    } else if relation.second.part == *part {
        (&relation.second, &relation.first)
    } else {
        return Err(AssemblyRecipeCompileError::PartNotInRelation {
            part: part.clone(),
            relation: relation_key.clone(),
        });
    };
    let moving_part = recipe
        .parts
        .get(part)
        .ok_or_else(|| AssemblyRecipeCompileError::PartMissing(part.clone()))?;
    let target_part = recipe
        .parts
        .get(&target_ref.part)
        .ok_or_else(|| AssemblyRecipeCompileError::PartMissing(target_ref.part.clone()))?;
    let recipe_parameter = moving_part.parameters.get(parameter).ok_or_else(|| {
        AssemblyRecipeCompileError::ParameterMissing {
            part: part.clone(),
            parameter: parameter.clone(),
        }
    })?;
    let parameter_target = recipe_parameter.target.as_ref().ok_or_else(|| {
        AssemblyRecipeCompileError::UnsupportedParameter {
            part: part.clone(),
            parameter: parameter.clone(),
        }
    })?;
    let parameter_axis = supported_parameter_axis(snapshot, parameter_target)
        .and_then(axis_index)
        .ok_or_else(|| AssemblyRecipeCompileError::UnsupportedGeometry {
            part: part.clone(),
            parameter: parameter.clone(),
        })?;
    let moving_bounds = supported_part_bounds(snapshot, recipe, moving_part).ok_or_else(|| {
        AssemblyRecipeCompileError::UnsupportedGeometry {
            part: part.clone(),
            parameter: parameter.clone(),
        }
    })?;
    let target_bounds = supported_part_bounds(snapshot, recipe, target_part).ok_or_else(|| {
        AssemblyRecipeCompileError::UnsupportedGeometry {
            part: target_ref.part.clone(),
            parameter: parameter.clone(),
        }
    })?;
    let moving_face = supported_face(moving_bounds, &moving_ref.role)
        .ok_or_else(|| AssemblyRecipeCompileError::UnsupportedFaceRole(moving_ref.role.clone()))?;
    let target_face = supported_face(target_bounds, &target_ref.role)
        .ok_or_else(|| AssemblyRecipeCompileError::UnsupportedFaceRole(target_ref.role.clone()))?;
    if moving_face.axis != parameter_axis {
        return Err(AssemblyRecipeCompileError::UnsupportedGeometry {
            part: part.clone(),
            parameter: parameter.clone(),
        });
    }
    let moving_resolved = snapshot
        .resolve_instance_path(&moving_part.instance_path)
        .map_err(|_| AssemblyRecipeCompileError::PartChanged(part.clone()))?;
    let target_resolved = snapshot
        .resolve_instance_path(&target_part.instance_path)
        .map_err(|_| AssemblyRecipeCompileError::PartChanged(target_ref.part.clone()))?;
    let moving_normal = transform_vector(moving_resolved.world_transform, moving_face.normal_local);
    let target_normal = transform_vector(target_resolved.world_transform, target_face.normal_local);
    if (dot(moving_normal, target_normal) + 1.0).abs() > 1.0e-8
        || !faces_overlap(
            moving_resolved.world_transform,
            moving_bounds,
            moving_face,
            target_resolved.world_transform,
            target_bounds,
            target_face,
        )
    {
        return Err(AssemblyRecipeCompileError::AmbiguousContact(
            relation_key.clone(),
        ));
    }
    let moving_point = transform_point(moving_resolved.world_transform, moving_face.point_local);
    let target_point = transform_point(target_resolved.world_transform, target_face.point_local);
    let positive_axis = transform_vector(
        moving_resolved.world_transform,
        std::array::from_fn(|axis| (axis == parameter_axis) as u8 as f64),
    );
    let displacement = dot(subtract(target_point, moving_point), positive_axis);
    let coefficient = match (moving_face.maximum, anchor) {
        (true, RecipeDimensionAnchor::Minimum) => 1.0,
        (true, RecipeDimensionAnchor::Centre) => 0.5,
        (true, RecipeDimensionAnchor::Maximum) => 0.0,
        (false, RecipeDimensionAnchor::Minimum) => 0.0,
        (false, RecipeDimensionAnchor::Centre) => -0.5,
        (false, RecipeDimensionAnchor::Maximum) => -1.0,
    };
    if coefficient == 0.0 {
        if displacement.abs() <= 1.0e-8 {
            return snapshot
                .feature_parameter_value(parameter_target)
                .ok_or_else(|| AssemblyRecipeCompileError::UnsupportedParameter {
                    part: part.clone(),
                    parameter: parameter.clone(),
                });
        }
        return Err(AssemblyRecipeCompileError::ContactUnreachable(
            relation_key.clone(),
        ));
    }
    let current = snapshot
        .feature_parameter_value(parameter_target)
        .ok_or_else(|| AssemblyRecipeCompileError::UnsupportedParameter {
            part: part.clone(),
            parameter: parameter.clone(),
        })?;
    let value = current + displacement / coefficient;
    if !value.is_finite() || value <= 1.0e-9 {
        return Err(AssemblyRecipeCompileError::ContactUnreachable(
            relation_key.clone(),
        ));
    }
    Ok(value)
}

fn supported_rectangle_sketch(
    snapshot: &Snapshot,
    sketch_id: FeatureId,
) -> Option<([[f64; 2]; 2], WorkplaneFrame)> {
    let FeatureKind::Sketch(spec) = snapshot.feature(sketch_id)?.kind() else {
        return None;
    };
    let bounds = spec.rectangle_bounds()?;
    let FeatureKind::Workplane(workplane) = snapshot.feature(spec.workplane)?.kind() else {
        return None;
    };
    if !matches!(
        workplane.support,
        WorkplaneSupport::Free | WorkplaneSupport::Principal(_)
    ) {
        return None;
    }
    Some((bounds, workplane.frame))
}

fn supported_part_bounds(
    snapshot: &Snapshot,
    recipe: &AssemblyRecipe,
    part: &RecipePart,
) -> Option<SupportedPartBounds> {
    let owned = recipe
        .owned_features
        .values()
        .filter(|owned| owned.part == part.key)
        .map(|owned| owned.feature_id)
        .collect::<BTreeSet<_>>();
    let mut result = None;
    for feature_id in &owned {
        let bounds = match snapshot.feature(*feature_id)?.kind() {
            FeatureKind::Extrusion { profile, height } => {
                if !owned.contains(profile) || height.millimetres() <= 0.0 {
                    return None;
                }
                let FeatureKind::Profile { points_mm } = snapshot.feature(*profile)?.kind() else {
                    return None;
                };
                let [minimum, maximum] = rectangle_bounds(points_mm)?;
                SupportedPartBounds {
                    minimum: [minimum[0], minimum[1], 0.0],
                    maximum: [maximum[0], maximum[1], height.millimetres()],
                }
            }
            FeatureKind::Pad(spec) => {
                if !owned.contains(&spec.sketch) {
                    return None;
                }
                let ([minimum, maximum], frame) =
                    supported_rectangle_sketch(snapshot, spec.sketch)?;
                let FeatureKind::Sketch(sketch) = snapshot.feature(spec.sketch)?.kind() else {
                    return None;
                };
                if !owned.contains(&sketch.workplane)
                    || sketch.solved_regions().ok()?.as_slice().first()?.id != spec.region
                    || [frame.x_axis, frame.y_axis, frame.normal]
                        .into_iter()
                        .any(|axis| axis_index(axis.map(f64::abs)).is_none())
                {
                    return None;
                }
                let height = spec.extent.blind_distance()?.millimetres();
                let direction = spec.direction.vector(frame.normal)?;
                if direction != frame.normal && direction != frame.normal.map(|value| -value) {
                    return None;
                }
                let mut bounds = SupportedPartBounds {
                    minimum: [f64::INFINITY; 3],
                    maximum: [f64::NEG_INFINITY; 3],
                };
                for x in [minimum[0], maximum[0]] {
                    for y in [minimum[1], maximum[1]] {
                        for depth in [0.0, height] {
                            for axis in 0..3 {
                                let coordinate = frame.origin_mm[axis]
                                    + x * frame.x_axis[axis]
                                    + y * frame.y_axis[axis]
                                    + depth * direction[axis];
                                bounds.minimum[axis] = bounds.minimum[axis].min(coordinate);
                                bounds.maximum[axis] = bounds.maximum[axis].max(coordinate);
                            }
                        }
                    }
                }
                bounds
            }
            _ => continue,
        };
        if result.replace(bounds).is_some() {
            return None;
        }
    }
    result
}

fn rectangle_bounds(points: &[[f64; 2]]) -> Option<[[f64; 2]; 2]> {
    let [first, second, third, fourth] = points else {
        return None;
    };
    if first[0] < second[0]
        && first[1] == second[1]
        && second[0] == third[0]
        && second[1] < third[1]
        && third[1] == fourth[1]
        && fourth[0] == first[0]
    {
        Some([*first, *third])
    } else {
        None
    }
}

fn supported_face(bounds: SupportedPartBounds, role: &str) -> Option<SupportedFace> {
    let (axis, maximum) = match role {
        "bounds.x.minimum" => (0, false),
        "bounds.x.maximum" => (0, true),
        "bounds.y.minimum" => (1, false),
        "bounds.y.maximum" => (1, true),
        "bounds.z.minimum" | "extrusion.bottom" => (2, false),
        "bounds.z.maximum" | "extrusion.top" => (2, true),
        _ => return None,
    };
    let mut point_local =
        std::array::from_fn(|index| (bounds.minimum[index] + bounds.maximum[index]) * 0.5);
    point_local[axis] = if maximum {
        bounds.maximum[axis]
    } else {
        bounds.minimum[axis]
    };
    let mut normal_local = [0.0; 3];
    normal_local[axis] = if maximum { 1.0 } else { -1.0 };
    Some(SupportedFace {
        axis,
        maximum,
        point_local,
        normal_local,
    })
}

fn faces_overlap(
    moving_transform: Transform,
    moving_bounds: SupportedPartBounds,
    moving_face: SupportedFace,
    target_transform: Transform,
    target_bounds: SupportedPartBounds,
    target_face: SupportedFace,
) -> bool {
    let moving_axes = [0, 1, 2]
        .into_iter()
        .filter(|axis| *axis != moving_face.axis)
        .collect::<Vec<_>>();
    let target_axes = [0, 1, 2]
        .into_iter()
        .filter(|axis| *axis != target_face.axis)
        .collect::<Vec<_>>();
    let moving_world_axes = moving_axes
        .iter()
        .map(|axis| transform_vector(moving_transform, unit_axis(*axis)))
        .collect::<Vec<_>>();
    let target_world_axes = target_axes
        .iter()
        .map(|axis| transform_vector(target_transform, unit_axis(*axis)))
        .collect::<Vec<_>>();
    if target_world_axes.iter().any(|target_axis| {
        moving_world_axes
            .iter()
            .filter(|moving_axis| dot(**moving_axis, *target_axis).abs() >= 1.0 - 1.0e-8)
            .count()
            != 1
    }) {
        return false;
    }
    let moving_corners = face_corners(moving_bounds, moving_face)
        .map(|point| transform_point(moving_transform, point));
    let target_corners = face_corners(target_bounds, target_face)
        .map(|point| transform_point(target_transform, point));
    moving_world_axes.into_iter().all(|axis| {
        let moving_interval = projection_interval(moving_corners, axis);
        let target_interval = projection_interval(target_corners, axis);
        moving_interval[1].min(target_interval[1]) - moving_interval[0].max(target_interval[0])
            > 1.0e-8
    })
}

fn face_corners(bounds: SupportedPartBounds, face: SupportedFace) -> [[f64; 3]; 4] {
    let tangents = [0, 1, 2]
        .into_iter()
        .filter(|axis| *axis != face.axis)
        .collect::<Vec<_>>();
    std::array::from_fn(|index| {
        let mut point = face.point_local;
        point[tangents[0]] = if index & 1 == 0 {
            bounds.minimum[tangents[0]]
        } else {
            bounds.maximum[tangents[0]]
        };
        point[tangents[1]] = if index & 2 == 0 {
            bounds.minimum[tangents[1]]
        } else {
            bounds.maximum[tangents[1]]
        };
        point
    })
}

fn projection_interval(points: [[f64; 3]; 4], axis: [f64; 3]) -> [f64; 2] {
    points.into_iter().fold(
        [f64::INFINITY, f64::NEG_INFINITY],
        |[minimum, maximum], point| {
            let projection = dot(point, axis);
            [minimum.min(projection), maximum.max(projection)]
        },
    )
}

fn axis_index(axis: [f64; 3]) -> Option<usize> {
    (0..3).find(|index| {
        axis[*index] == 1.0 && (0..3).all(|other| other == *index || axis[other] == 0.0)
    })
}

fn unit_axis(axis: usize) -> [f64; 3] {
    std::array::from_fn(|index| (index == axis) as u8 as f64)
}

fn transform_point(transform: Transform, point: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
        matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
    ]
}

fn transform_vector(transform: Transform, vector: [f64; 3]) -> [f64; 3] {
    let matrix = transform.matrix();
    [
        matrix[0] * vector[0] + matrix[1] * vector[1] + matrix[2] * vector[2],
        matrix[4] * vector[0] + matrix[5] * vector[1] + matrix[6] * vector[2],
        matrix[8] * vector[0] + matrix[9] * vector[1] + matrix[10] * vector[2],
    ]
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| left[axis] - right[axis])
}

fn supported_parameter_axis(
    snapshot: &Snapshot,
    target: &FeatureParameterTarget,
) -> Option<[f64; 3]> {
    let feature = snapshot.feature(target.feature_id)?;
    match (feature.kind(), target.path.as_str()) {
        (FeatureKind::Profile { .. }, "bounds.width") => Some([1.0, 0.0, 0.0]),
        (FeatureKind::Profile { .. }, "bounds.height") => Some([0.0, 1.0, 0.0]),
        (FeatureKind::Extrusion { .. }, "height") => Some([0.0, 0.0, 1.0]),
        (FeatureKind::Sketch(_), "bounds.width" | "bounds.height") => {
            let (_, frame) = supported_rectangle_sketch(snapshot, target.feature_id)?;
            Some(if target.path.as_str() == "bounds.width" {
                frame.x_axis
            } else {
                frame.y_axis
            })
        }
        (FeatureKind::Pad(spec), "extent.distance")
            if matches!(spec.extent, FeatureExtent::Blind(_)) =>
        {
            let (_, frame) = supported_rectangle_sketch(snapshot, spec.sketch)?;
            let direction = spec.direction.vector(frame.normal)?;
            ((dot(direction, frame.normal).abs() - 1.0).abs() <= 1.0e-8).then_some(direction)
        }
        _ => None,
    }
}

fn affected_instance_paths(snapshot: &Snapshot, part: &RecipePart) -> Vec<InstancePath> {
    match &part.edit_scope {
        RecipeEditScope::Occurrence(path) => vec![path.clone()],
        RecipeEditScope::SharedDefinition(definition_id) => snapshot
            .scene_query()
            .into_iter()
            .filter_map(|occurrence| {
                (occurrence.definition_id == *definition_id).then_some(occurrence.instance_path)
            })
            .collect(),
    }
}

fn translate_in_local_axis(
    transform: Transform,
    axis: [f64; 3],
    distance: f64,
) -> Option<Transform> {
    let mut matrix = *transform.matrix();
    for row in 0..3 {
        matrix[row * 4 + 3] += distance
            * (matrix[row * 4] * axis[0]
                + matrix[row * 4 + 1] * axis[1]
                + matrix[row * 4 + 2] * axis[2]);
    }
    Transform::from_matrix(matrix).ok()
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssemblyRecipeCompileError {
    RecipeMissing,
    RecipeInvalid(AssemblyRecipeError),
    ResourceLimit,
    DuplicateNode(RecipeKey),
    DependencyMissing(RecipeKey),
    DuplicateDependency(RecipeKey),
    DependencyCycle(RecipeKey),
    PartMissing(RecipeKey),
    PartChanged(RecipeKey),
    ParameterMissing {
        part: RecipeKey,
        parameter: RecipeKey,
    },
    UnsupportedParameter {
        part: RecipeKey,
        parameter: RecipeKey,
    },
    UnsupportedGeometry {
        part: RecipeKey,
        parameter: RecipeKey,
    },
    ConflictingWrites {
        part: RecipeKey,
        parameter: RecipeKey,
    },
    RelationMissing(RecipeKey),
    UnsupportedRelation(RecipeKey),
    PartNotInRelation {
        part: RecipeKey,
        relation: RecipeKey,
    },
    UnsupportedFaceRole(String),
    AmbiguousContact(RecipeKey),
    ContactUnreachable(RecipeKey),
    UnsupportedDependentJoinery(RecipeKey),
    InvalidValue(RecipeKey),
    OwnedFeatureChanged(RecipeKey),
    Canonical(crate::document::CanonicalError),
}

impl fmt::Display for AssemblyRecipeCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "assembly recipe compile failed: {self:?}")
    }
}

impl std::error::Error for AssemblyRecipeCompileError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssemblyRecipeError {
    UnsupportedVersion(String),
    InvalidKey(String),
    DuplicateKey(RecipeKey),
    NonCanonicalKey(RecipeKey),
    ResourceLimit,
    UnresolvedPart(RecipeKey),
    PartChanged(RecipeKey),
    InvalidEditScope(RecipeKey),
    InvalidParameter(RecipeKey),
    InvalidRelation(RecipeKey),
    InvalidJoinery(RecipeKey),
    InvalidOwnership(RecipeKey),
    UnrecognizedFeatureSet(RecipeKey),
    UnrecognizedFeature(FeatureId),
    OwnedFeatureConflict(RecipeKey),
    UnresolvedJoinery(RecipeKey),
}

impl fmt::Display for AssemblyRecipeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "assembly recipe is invalid: {self:?}")
    }
}

impl std::error::Error for AssemblyRecipeError {}
