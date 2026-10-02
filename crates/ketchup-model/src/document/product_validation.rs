use super::*;
use ketchup_tolerance::limits;

pub(super) fn ensure_product_id(id: u64) -> Result<(), CanonicalError> {
    if id == 0 {
        Err(CanonicalError::ReservedProductId)
    } else {
        Ok(())
    }
}

pub(super) fn ensure_name(name: &str) -> Result<(), CanonicalError> {
    if name.trim().is_empty() {
        Err(CanonicalError::EmptyProductName)
    } else {
        Ok(())
    }
}

pub(super) fn validate_transform(transform: Transform) -> Result<(), CanonicalError> {
    Transform::from_matrix(transform.matrix).map(|_| ())
}

pub(super) fn validate_product(product: &ProductModel) -> Result<(), CanonicalError> {
    validate_product_with_drawing_sources(product, true)
}

pub(super) fn validate_product_with_drawing_sources(
    product: &ProductModel,
    validate_drawing_sources: bool,
) -> Result<(), CanonicalError> {
    validate_product_change(None, product, validate_drawing_sources)
}

/// Validates `product`, derived from `before`, which passed validation already. A check whose
/// inputs `product` kept passes again, so only the checks [`ProductChange`] names run; without
/// `before` every check runs.
pub(super) fn validate_product_change(
    before: Option<&ProductModel>,
    product: &ProductModel,
    validate_drawing_sources: bool,
) -> Result<(), CanonicalError> {
    let change = ProductChange::between(before, product);
    ensure_product_id(product.document_id.0)?;
    validate_document_entries(product, validate_drawing_sources, change.definitions)?;
    if change.definitions {
        validate_definitions(product)?;
        validate_features(product)?;
        validate_feature_parameters(product)?;
    }
    match change.occurrences_to_check(product) {
        Some(changed) => validate_occurrences(product, changed.into_iter())?,
        None => validate_occurrences(product, product.occurrences.values())?,
    }
    if change.groups {
        validate_groups(product)?;
    }
    if change.definitions {
        validate_local_structure(product)?;
    }
    Ok(())
}

/// What a candidate product changed, as far as validation cares, relative to the validated
/// product it was derived from. Revisions share unchanged entries, so a kept entry is the
/// same allocation and comparing two products costs one pointer comparison per entry.
struct ProductChange<'a> {
    before: Option<&'a ProductModel>,
    /// Whether any input of the definition checks changed: those checks read only the fields
    /// compared in [`definition_inputs_unchanged`].
    definitions: bool,
    groups: bool,
}

impl<'a> ProductChange<'a> {
    fn between(before: Option<&'a ProductModel>, product: &ProductModel) -> Self {
        Self {
            before,
            definitions: before.is_none_or(|before| !definition_inputs_unchanged(before, product)),
            groups: before.is_none_or(|before| !same_entries(&before.groups, &product.groups)),
        }
    }

    /// The occurrences to check when only some of them can fail: an occurrence's checks read
    /// the occurrence and which definitions, groups and tags exist. `None` means all of them.
    fn occurrences_to_check<'p>(
        &self,
        product: &'p ProductModel,
    ) -> Option<Vec<&'p Arc<Occurrence>>> {
        let before = self.before?;
        (same_keys(&before.definitions, &product.definitions)
            && same_keys(&before.groups, &product.groups)
            && same_keys(&before.tags, &product.tags))
        .then(|| {
            product
                .occurrences
                .iter()
                .filter(|(id, occurrence)| {
                    !before
                        .occurrences
                        .get(id)
                        .is_some_and(|kept| Arc::ptr_eq(kept, occurrence))
                })
                .map(|(_, occurrence)| occurrence)
                .collect()
        })
    }
}

/// Every field the checks of definitions, features, their parameters and local structure
/// read: [`validate_definitions`], [`validate_features`], [`validate_feature_parameters`],
/// [`validate_local_structure`] and the feature dependency graph.
fn definition_inputs_unchanged(before: &ProductModel, product: &ProductModel) -> bool {
    before.document_id == product.document_id
        && before.tolerance == product.tolerance
        && before.body_feature_suppression == product.body_feature_suppression
        && same_entries(&before.definitions, &product.definitions)
        && same_entries(&before.features, &product.features)
        && same_entries(&before.local_groups, &product.local_groups)
        && same_entries(&before.local_occurrences, &product.local_occurrences)
        && same_entries(&before.evaluator_nodes, &product.evaluator_nodes)
        && same_entries(
            &before.feature_parameter_bindings,
            &product.feature_parameter_bindings,
        )
        && same_entries(
            &before.feature_parameter_provenance,
            &product.feature_parameter_provenance,
        )
        && same_entries(
            &before.exact_reference_evidence,
            &product.exact_reference_evidence,
        )
        && same_entries(&before.import_receipts, &product.import_receipts)
        && same_entries(&before.tags, &product.tags)
}

fn same_entries<K: Ord, V>(before: &BTreeMap<K, Arc<V>>, after: &BTreeMap<K, Arc<V>>) -> bool {
    before.len() == after.len()
        && before
            .iter()
            .zip(after)
            .all(|((before_key, before), (after_key, after))| {
                before_key == after_key && Arc::ptr_eq(before, after)
            })
}

fn same_keys<K: Ord, V>(before: &BTreeMap<K, V>, after: &BTreeMap<K, V>) -> bool {
    before.len() == after.len() && before.keys().eq(after.keys())
}

/// Checks every entry outside definitions, occurrences and groups, in the order the full
/// validation reports them; the feature dependency graph sits among them and is checked
/// only when `definitions_changed`.
fn validate_document_entries(
    product: &ProductModel,
    validate_drawing_sources: bool,
    definitions_changed: bool,
) -> Result<(), CanonicalError> {
    let mut assigned_codes = BTreeSet::new();
    for (path, code) in &product.production_codes {
        validate_production_code(code)?;
        if !assigned_codes.insert(code.to_ascii_uppercase()) {
            return Err(CanonicalError::DuplicateProductionCode(code.clone()));
        }
        if production_code_identity(product, path).is_none() {
            return Err(CanonicalError::InvalidProductionCodePath(path.clone()));
        }
    }
    if product
        .instance_transform_overrides
        .iter()
        .any(|(path, transform)| {
            path.is_root()
                || !matches!(path.steps().last(), Some(InstancePathStep::Occurrence(_)))
                || validate_transform(*transform).is_err()
                || resolve_product_instance_path(product, path).is_none()
        })
    {
        return Err(CanonicalError::InvalidInstancePath);
    }
    if let Some(id) = product
        .grounded_occurrences
        .iter()
        .find(|id| !product.occurrences.contains_key(id))
    {
        return Err(CanonicalError::OccurrenceNotFound(*id));
    }
    for (id, mate) in &product.assembly_mates {
        if *id != mate.id() {
            return Err(CanonicalError::InvalidAssemblyMate(*id));
        }
        validate_assembly_mate(product, mate, false)?;
    }
    for (id, joint) in &product.assembly_joints {
        if *id != joint.id() {
            return Err(CanonicalError::InvalidAssemblyJoint(*id));
        }
        validate_assembly_joint(product, joint)?;
    }
    for (id, coupling) in &product.assembly_motion_couplings {
        if *id != coupling.id() {
            return Err(CanonicalError::InvalidAssemblyMotionCoupling(*id));
        }
        validate_assembly_motion_coupling(product, coupling)?;
    }
    for (id, interface) in &product.mechanical_interfaces {
        if *id != interface.id() {
            return Err(CanonicalError::InvalidMechanicalInterface(*id));
        }
        validate_mechanical_interface(product, interface)?;
    }
    for (id, condition) in &product.mechanical_conditions {
        if *id != condition.id() {
            return Err(CanonicalError::InvalidMechanicalCondition(*id));
        }
        validate_mechanical_condition(product, condition)?;
    }
    for (id, study) in &product.assembly_motion_studies {
        if *id != study.id() {
            return Err(CanonicalError::InvalidAssemblyMotionStudy(*id));
        }
        validate_assembly_motion_study(product, study)?;
    }
    for (id, sheet) in &product.drawing_sheets {
        if *id != sheet.id() {
            return Err(CanonicalError::Drawing(DrawingError::InvalidSheet));
        }
        if validate_drawing_sources {
            validate_drawing_sheet(product, sheet)?;
        }
    }
    for (id, plan) in &product.cam_plans {
        if *id != plan.id() {
            return Err(CanonicalError::Cam(CamError::InvalidPlan));
        }
        plan.validate_structure().map_err(CanonicalError::Cam)?;
    }
    let snapshot = std::cell::OnceCell::new();
    let snapshot = || {
        snapshot.get_or_init(|| Snapshot {
            revision_id: 0,
            product: Arc::new(product.clone()),
        })
    };
    for (id, joint) in &product.pin_joints {
        if *id != joint.id {
            return Err(CanonicalError::PinJoint(PinJointError::InvalidJointId));
        }
        project_pin_joint_contract(snapshot(), joint).map_err(CanonicalError::PinJoint)?;
    }
    if let Some(recipe) = product.assembly_recipe.as_deref() {
        recipe
            .audit(snapshot())
            .map_err(CanonicalError::AssemblyRecipe)?;
    }
    if definitions_changed {
        FeatureDependencyGraph::from_product(product)?;
    }
    for (id, joint) in &product.joints {
        if *id != joint.id() || !joint.volume().has_positive_volume() {
            return Err(CanonicalError::Prismatic(PrismaticError::EmptyVolume));
        }
    }
    for (id, space) in &product.spaces {
        if *id != space.id() || !space.volume().has_positive_volume() {
            return Err(CanonicalError::Space(SpaceError::InvalidVolume));
        }
        for adjacent_id in space.adjacent_to() {
            let adjacent = product
                .spaces
                .get(adjacent_id)
                .ok_or(CanonicalError::Space(SpaceError::MissingSpace))?;
            if !adjacent.adjacent_to().contains(id) {
                return Err(CanonicalError::Space(SpaceError::AsymmetricAdjacency));
            }
        }
        if space
            .accessible_to()
            .iter()
            .any(|target| !product.spaces.contains_key(target))
        {
            return Err(CanonicalError::Space(SpaceError::MissingSpace));
        }
    }
    for (id, clearance) in &product.clearance_volumes {
        if *id != clearance.id() || !clearance.volume().has_positive_volume() {
            return Err(CanonicalError::Space(SpaceError::InvalidVolume));
        }
        let owner_is_valid = match clearance.owner() {
            ClearanceOwner::Occurrence(path) => snapshot().resolve_instance_path(path).is_ok(),
            ClearanceOwner::Space(space_id) => product.spaces.contains_key(space_id),
        };
        if !owner_is_valid {
            return Err(CanonicalError::Space(SpaceError::InvalidOwner));
        }
    }
    for (id, dimension) in &product.persistent_dimensions {
        if *id != dimension.id {
            return Err(CanonicalError::InvalidPersistentDimensionTarget);
        }
        validate_persistent_dimension(dimension)?;
    }
    for tag in product.tags.values() {
        ensure_product_id(tag.id.0)?;
        ensure_name(&tag.name)?;
    }
    for (id, collection) in &product.collections {
        if *id != collection.id {
            return Err(CanonicalError::CollectionNotFound(*id));
        }
        ensure_product_id(collection.id.0)?;
        ensure_name(&collection.name)?;
        for occurrence_id in &collection.occurrence_ids {
            if !product.occurrences.contains_key(occurrence_id) {
                return Err(CanonicalError::OccurrenceNotFound(*occurrence_id));
            }
        }
    }
    for (id, receipt) in &product.import_receipts {
        if *id != receipt.id() {
            return Err(CanonicalError::InvalidImportReceipt(
                ImportContractError::InvalidIdentity,
            ));
        }
        receipt
            .validate()
            .map_err(CanonicalError::InvalidImportReceipt)?;
    }
    Ok(())
}

fn validate_definitions(product: &ProductModel) -> Result<(), CanonicalError> {
    for definition in product.definitions.values() {
        ensure_product_id(definition.id.0)?;
        ensure_name(&definition.name)?;
        if definition.bodies.is_empty()
            || !definition.bodies.contains_key(&definition.active_body_id)
        {
            return Err(CanonicalError::BodyNotFound(
                definition.id,
                definition.active_body_id,
            ));
        }
        for (id, body) in &definition.bodies {
            if *id != body.id {
                return Err(CanonicalError::BodyNotFound(definition.id, *id));
            }
            ensure_product_id(body.id.0)?;
            ensure_name(&body.name)?;
            if let Some(feature_id) = body.consumed_by {
                let feature = product
                    .features
                    .get(&feature_id)
                    .ok_or(CanonicalError::InvalidBodyAuthoringPlan)?;
                let ownership = definition
                    .feature_body_ownership
                    .get(&feature_id)
                    .ok_or(CanonicalError::InvalidBodyAuthoringPlan)?;
                if feature.definition_id != definition.id
                    || !matches!(feature.kind, FeatureKind::Boolean { .. })
                    || !ownership.input_body_ids.contains(id)
                    || ownership.output_body_id == Some(*id)
                    || definition.active_body_id == *id
                {
                    return Err(CanonicalError::InvalidBodyAuthoringPlan);
                }
            }
        }
        let mut seen = BTreeSet::new();
        for feature_id in &definition.feature_ids {
            if !seen.insert(*feature_id) {
                return Err(CanonicalError::InvalidFeatureOwnership(*feature_id));
            }
            let feature = product
                .features
                .get(feature_id)
                .ok_or(CanonicalError::FeatureNotFound(*feature_id))?;
            if feature.definition_id != definition.id {
                return Err(CanonicalError::InvalidFeatureOwnership(*feature_id));
            }
            let ownership = definition
                .feature_body_ownership
                .get(feature_id)
                .ok_or(CanonicalError::InvalidBodyOwnership(*feature_id))?;
            if ownership
                .input_body_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
                || ownership
                    .input_body_ids
                    .iter()
                    .chain(ownership.output_body_id.iter())
                    .any(|id| !definition.bodies.contains_key(id))
                || feature.kind.produces_body() != ownership.output_body_id.is_some()
                || inferred_feature_body_ownership(product, definition, &feature.kind)?
                    .input_body_ids
                    != ownership.input_body_ids
            {
                return Err(CanonicalError::InvalidBodyOwnership(*feature_id));
            }
        }
        if definition.feature_body_ownership.len() != definition.feature_ids.len() {
            return Err(CanonicalError::InvalidFeatureOwnership(
                definition
                    .feature_body_ownership
                    .keys()
                    .find(|id| !seen.contains(id))
                    .cloned()
                    .unwrap_or(FeatureId(0)),
            ));
        }
        validate_body_dependency_graph(definition)?;
        let mut local_ids = BTreeSet::new();
        for local_id in &definition.local_group_ids {
            if !local_ids.insert(local_id.0)
                || !product.local_groups.contains_key(&LocalGroupKey {
                    definition_id: definition.id,
                    local_id: *local_id,
                })
            {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
        for local_id in &definition.local_occurrence_ids {
            if !local_ids.insert(local_id.0)
                || !product.local_occurrences.contains_key(&LocalOccurrenceKey {
                    definition_id: definition.id,
                    local_id: *local_id,
                })
            {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
    }
    Ok(())
}

fn validate_features(product: &ProductModel) -> Result<(), CanonicalError> {
    for feature in product.features.values() {
        ensure_product_id(feature.id.0)?;
        ensure_name(&feature.name)?;
        validate_feature_kind(&feature.kind, product.tolerance.linear_mm())?;
        let definition = product
            .definitions
            .get(&feature.definition_id)
            .ok_or(CanonicalError::DefinitionNotFound(feature.definition_id))?;
        if !definition.feature_ids.contains(&feature.id) {
            return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
        }
        match feature.kind.clone() {
            FeatureKind::Pad(spec) => {
                if !pad_inputs_are_valid(product, feature.definition_id, &spec)? {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Revolve { profile, .. } => {
                let profile_feature = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                let supported_profile = match &profile_feature.kind {
                    FeatureKind::Profile { closed: true, .. } => true,
                    FeatureKind::Sketch(sketch) => sketch
                        .solved_regions()
                        .is_ok_and(|regions| regions.len() == 1),
                    _ => false,
                };
                if profile_feature.definition_id != feature.definition_id || !supported_profile {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Shell { .. }
            | FeatureKind::EdgeFinish { .. }
            | FeatureKind::FaceOffset { .. } => {
                validate_topological_feature_context(
                    product.document_id,
                    feature.definition_id,
                    &feature.kind,
                )?;
                let (target, references) = feature
                    .kind
                    .topological_picks()
                    .expect("topology features pick a target");
                validate_topological_target(product, definition, feature.id, target, &references)?;
            }
            FeatureKind::PlanarOffset { profile, distance } => {
                let source = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let source_precedes_offset = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == profile)
                    .is_some_and(|position| position < feature_position);
                let distance = distance.millimetres();
                let rectangle = source
                    .kind
                    .polygon_points()
                    .filter(|points| is_axis_aligned_rectangle(points));
                let valid_bounds = match (&source.kind, rectangle) {
                    (_, Some(points_mm)) => {
                        let output_bounds = [
                            points_mm[0][0] - distance,
                            points_mm[0][1] - distance,
                            points_mm[2][0] + distance,
                            points_mm[2][1] + distance,
                        ];
                        output_bounds.into_iter().all(|coordinate| {
                            coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE_MM
                        }) && output_bounds[2] - output_bounds[0] >= limits::MIN_LENGTH_MM
                            && output_bounds[3] - output_bounds[1] >= limits::MIN_LENGTH_MM
                    }
                    (FeatureKind::Profile { segments, closed }, None) => {
                        distance.abs() <= MAX_COORDINATE_MM
                            && exact_planar_offset_profile(segments, *closed, product.tolerance)
                                .is_some_and(|profile| {
                                    let bounds = profile.bounds_bits.map(f64::from_bits);
                                    let Some(margin) =
                                        profile.max_planar_offset_displacement_mm(distance)
                                    else {
                                        return false;
                                    };
                                    let output_envelope = if distance > 0.0 {
                                        [
                                            bounds[0] - margin,
                                            bounds[1] - margin,
                                            bounds[2] + margin,
                                            bounds[3] + margin,
                                        ]
                                    } else {
                                        bounds
                                    };
                                    let minimum_displacement = distance.abs();
                                    let cannot_statically_collapse = distance > 0.0
                                        || bounds[2] - bounds[0] > 2.0 * minimum_displacement
                                            && bounds[3] - bounds[1] > 2.0 * minimum_displacement;
                                    output_envelope.into_iter().all(|coordinate| {
                                        coordinate.is_finite()
                                            && coordinate.abs() <= MAX_COORDINATE_MM
                                    }) && cannot_statically_collapse
                                })
                    }
                    (FeatureKind::Sketch(spec), None) => {
                        spec.solved_regions().ok().is_some_and(|regions| {
                            let [region] = regions.as_slice() else {
                                return false;
                            };
                            accepts_planar_offset_solved_region(region, distance)
                        })
                    }
                    _ => false,
                };
                if source.definition_id != feature.definition_id
                    || !source_precedes_offset
                    || !valid_bounds
                {
                    return Err(CanonicalError::InvalidPlanarOffset);
                }
            }
            FeatureKind::Sweep { profile, path, .. } => {
                let profile_source = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                let path_source = product
                    .features
                    .get(&path)
                    .ok_or(CanonicalError::FeatureNotFound(path))?;
                let valid_inputs = ValidatedSweepInputs::with_workplane_frames(
                    product.tolerance,
                    &profile_source.kind,
                    &path_source.kind,
                    |workplane| {
                        product.features.get(&workplane).and_then(|feature| {
                            if let FeatureKind::Workplane(spec) = &feature.kind {
                                Some(spec.frame)
                            } else {
                                None
                            }
                        })
                    },
                )
                .is_some();
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let sources_precede_sweep = [profile, path].into_iter().all(|source_id| {
                    definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == source_id)
                        .is_some_and(|position| position < feature_position)
                });
                if profile_source.definition_id != feature.definition_id
                    || path_source.definition_id != feature.definition_id
                    || !valid_inputs
                    || !sources_precede_sweep
                {
                    return Err(CanonicalError::InvalidSweep);
                }
            }
            FeatureKind::WeldmentMember(spec) => {
                let profile_source = product
                    .features
                    .get(&spec.profile)
                    .ok_or(CanonicalError::FeatureNotFound(spec.profile))?;
                let path_source = product
                    .features
                    .get(&spec.path)
                    .ok_or(CanonicalError::FeatureNotFound(spec.path))?;
                let valid_profile = ValidatedSweepProfile::from_feature_kind(
                    &profile_source.kind,
                    product.tolerance,
                )
                .is_some();
                let valid_path = matches!(
                    &path_source.kind,
                    FeatureKind::SpatialPath { segments }
                        if is_valid_spatial_sweep_path(segments, product.tolerance.linear_mm())
                            && spatial_sweep_bounds_are_valid(&profile_source.kind, segments, product.tolerance.linear_mm())
                );
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let sources_precede_member =
                    [spec.profile, spec.path].into_iter().all(|source_id| {
                        definition
                            .feature_ids
                            .iter()
                            .position(|candidate| *candidate == source_id)
                            .is_some_and(|position| position < feature_position)
                    });
                if profile_source.definition_id != feature.definition_id
                    || path_source.definition_id != feature.definition_id
                    || !valid_profile
                    || !valid_path
                    || !sources_precede_member
                {
                    return Err(CanonicalError::InvalidWeldmentMember);
                }
            }
            FeatureKind::WeldmentJoint(spec) => {
                let member = |id: FeatureId| -> Option<(&Feature, [f64; 3], [f64; 3])> {
                    let member = product.features.get(&id)?.as_ref();
                    let FeatureKind::WeldmentMember(member_spec) = &member.kind else {
                        return None;
                    };
                    let path = product.features.get(&member_spec.path)?;
                    let FeatureKind::SpatialPath { segments } = &path.kind else {
                        return None;
                    };
                    let [SpatialPathSegment::Line { start_mm, end_mm }] = segments.as_slice()
                    else {
                        return None;
                    };
                    Some((member, *start_mm, *end_mm))
                };
                let Some((first, first_start, first_end)) = member(spec.first_member) else {
                    return Err(CanonicalError::InvalidWeldmentJoint);
                };
                let Some((second, second_start, second_end)) = member(spec.second_member) else {
                    return Err(CanonicalError::InvalidWeldmentJoint);
                };
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let members_precede_joint = [spec.first_member, spec.second_member]
                    .into_iter()
                    .all(|member_id| {
                        definition
                            .feature_ids
                            .iter()
                            .position(|candidate| *candidate == member_id)
                            .is_some_and(|position| position < feature_position)
                    });
                let distance = |left: [f64; 3], right: [f64; 3]| {
                    ((left[0] - right[0]).powi(2)
                        + (left[1] - right[1]).powi(2)
                        + (left[2] - right[2]).powi(2))
                    .sqrt()
                };
                let endpoint_pairs = [
                    (first_start, first_end, second_start, second_end),
                    (first_start, first_end, second_end, second_start),
                    (first_end, first_start, second_start, second_end),
                    (first_end, first_start, second_end, second_start),
                ];
                let matching = endpoint_pairs
                    .into_iter()
                    .filter(|(joint_first, _, joint_second, _)| {
                        distance(*joint_first, *joint_second) <= PROFILE_EPSILON_MM
                    })
                    .collect::<Vec<_>>();
                let Some((joint, first_far, _, second_far)) = matching.first().copied() else {
                    return Err(CanonicalError::InvalidWeldmentJoint);
                };
                let first_length = distance(joint, first_far);
                let second_length = distance(joint, second_far);
                let first_direction = [
                    (first_far[0] - joint[0]) / first_length,
                    (first_far[1] - joint[1]) / first_length,
                    (first_far[2] - joint[2]) / first_length,
                ];
                let second_direction = [
                    (second_far[0] - joint[0]) / second_length,
                    (second_far[1] - joint[1]) / second_length,
                    (second_far[2] - joint[2]) / second_length,
                ];
                let direction_dot = first_direction[0] * second_direction[0]
                    + first_direction[1] * second_direction[1]
                    + first_direction[2] * second_direction[2];
                if first.definition_id != feature.definition_id
                    || second.definition_id != feature.definition_id
                    || !members_precede_joint
                    || matching.len() != 1
                    || !first_length.is_finite()
                    || !second_length.is_finite()
                    || first_length < limits::MIN_LENGTH_MM
                    || second_length < limits::MIN_LENGTH_MM
                    || !direction_dot.is_finite()
                    || direction_dot.abs() > 0.996_194_698_091_745_5
                {
                    return Err(CanonicalError::InvalidWeldmentJoint);
                }
            }
            FeatureKind::SurfaceBody(SurfaceBodySpec::Planar { profile }) => {
                let source = product
                    .features
                    .get(&profile)
                    .ok_or(CanonicalError::FeatureNotFound(profile))?;
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let source_precedes_surface = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == profile)
                    .is_some_and(|position| position < feature_position);
                let valid_profile = matches!(
                    &source.kind,
                    FeatureKind::Profile {
                        segments,
                        closed: true,
                    } if segments.len() >= 2
                ) || matches!(
                    &source.kind,
                    FeatureKind::Sketch(sketch)
                        if sketch.solved_regions().is_ok_and(|regions| regions.len() == 1)
                );
                if source.definition_id != feature.definition_id
                    || !source_precedes_surface
                    || !valid_profile
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::SurfaceTrim { target, cutter } => {
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let valid_surface_operand = |operand: FeatureId| {
                    product.features.get(&operand).is_some_and(|source| {
                        source.definition_id == feature.definition_id
                            && source.kind.body_kind() == Some(BodyKind::Surface)
                            && definition
                                .feature_ids
                                .iter()
                                .position(|candidate| *candidate == operand)
                                .is_some_and(|position| position < feature_position)
                    })
                };
                if target == cutter
                    || !valid_surface_operand(target)
                    || !valid_surface_operand(cutter)
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::SurfaceExtend { target, .. }
            | FeatureKind::SurfaceThicken { target, .. } => {
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let valid_target = product.features.get(&target).is_some_and(|source| {
                    source.definition_id == feature.definition_id
                        && source.kind.body_kind() == Some(BodyKind::Surface)
                        && definition
                            .feature_ids
                            .iter()
                            .position(|candidate| *candidate == target)
                            .is_some_and(|position| position < feature_position)
                });
                if !valid_target {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::SurfaceKnit { surfaces, .. } => {
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let valid_surface = |surface: FeatureId| {
                    product.features.get(&surface).is_some_and(|source| {
                        source.definition_id == feature.definition_id
                            && source.kind.body_kind() == Some(BodyKind::Surface)
                            && definition
                                .feature_ids
                                .iter()
                                .position(|candidate| *candidate == surface)
                                .is_some_and(|position| position < feature_position)
                    })
                };
                if !surfaces.iter().copied().all(valid_surface) {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Loft {
                sections,
                guide,
                continuity,
            }
            | FeatureKind::SurfaceBody(SurfaceBodySpec::Loft {
                sections,
                guide,
                continuity,
            }) => {
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                for section in sections {
                    let profile = product
                        .features
                        .get(&section.profile)
                        .ok_or(CanonicalError::FeatureNotFound(section.profile))?;
                    let source_precedes_loft = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == section.profile)
                        .is_some_and(|position| position < feature_position);
                    let valid_profile = match &profile.kind {
                        FeatureKind::Sketch(sketch) => sketch
                            .solved_regions()
                            .is_ok_and(|regions| regions.len() == 1),
                        kind => kind.closed_spline_points().is_some_and(|points| {
                            points.len() <= MAX_EXACT_BREP_LOFT_CONTROL_POINTS
                        }),
                    };
                    if profile.definition_id != feature.definition_id
                        || !valid_profile
                        || !source_precedes_loft
                    {
                        return Err(CanonicalError::InvalidLoft);
                    }
                }
                if let Some(guide) = guide {
                    let guide_feature = product
                        .features
                        .get(&guide)
                        .ok_or(CanonicalError::FeatureNotFound(guide))?;
                    let guide_precedes_loft = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == guide)
                        .is_some_and(|position| position < feature_position);
                    if continuity == LoftContinuity::Curvature
                        || guide_feature.definition_id != feature.definition_id
                        || !matches!(guide_feature.kind, FeatureKind::SpatialPath { .. })
                        || !guide_precedes_loft
                    {
                        return Err(CanonicalError::InvalidLoft);
                    }
                }
            }
            FeatureKind::Boolean { target, tool, .. } => {
                let target_feature = product
                    .features
                    .get(&target)
                    .ok_or(CanonicalError::FeatureNotFound(target))?;
                let tool_feature = product
                    .features
                    .get(&tool)
                    .ok_or(CanonicalError::FeatureNotFound(tool))?;
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let inputs_precede_boolean = [target, tool].into_iter().all(|input| {
                    definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == input)
                        .is_some_and(|position| position < feature_position)
                });
                if target == tool
                    || target_feature.definition_id != feature.definition_id
                    || tool_feature.definition_id != feature.definition_id
                    || !feature_kind_is_solid(&target_feature.kind)
                    || !feature_kind_is_solid(&tool_feature.kind)
                    || !inputs_precede_boolean
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::ImportedExactBody(spec) => {
                let receipt_is_invalid =
                    product
                        .import_receipts
                        .get(&spec.import_id)
                        .is_none_or(|receipt| {
                            !matches!(receipt.format(), ImportFormat::Step | ImportFormat::Iges)
                                || receipt.source_sha256() != &spec.source_sha256
                                || receipt.source_byte_len() != spec.source_byte_len
                        });
                if receipt_is_invalid {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::RigidTransform { target, .. } => {
                let target_feature = product
                    .features
                    .get(&target)
                    .ok_or(CanonicalError::FeatureNotFound(target))?;
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let target_precedes = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == target)
                    .is_some_and(|position| position < feature_position);
                if target_feature.definition_id != feature.definition_id
                    || !feature_kind_is_solid(&target_feature.kind)
                    || !target_precedes
                {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::MeshBody(spec) => {
                let authority_is_invalid = match &spec.authority {
                    MeshAuthority::ExactConversion(conversion) => {
                        conversion.destination_definition_id != feature.definition_id
                            || conversion.destination_feature_id != feature.id
                    }
                    MeshAuthority::ImportedStl { import_id } => product
                        .import_receipts
                        .get(import_id)
                        .is_none_or(|receipt| receipt.format() != ImportFormat::Stl),
                    MeshAuthority::ImportedSketchupScene { import_id } => product
                        .import_receipts
                        .get(import_id)
                        .is_none_or(|receipt| receipt.format() != ImportFormat::SketchupScene),
                    MeshAuthority::ImportedGlb { import_id } => product
                        .import_receipts
                        .get(import_id)
                        .is_none_or(|receipt| receipt.format() != ImportFormat::Glb),
                    MeshAuthority::Authored { .. } => false,
                };
                if definition.feature_ids.as_slice() != [feature.id] || authority_is_invalid {
                    return Err(CanonicalError::InvalidFeatureOwnership(feature.id));
                }
            }
            FeatureKind::Workplane(spec) => match &spec.support {
                WorkplaneSupport::Free | WorkplaneSupport::Principal(_) => {}
                WorkplaneSupport::PlanarFace { reference, health } => {
                    let producer = product.features.get(&reference.producer_feature_id).ok_or(
                        CanonicalError::Sketch(SketchError::InvalidPlanarFaceSupport),
                    )?;
                    let profile = product.features.get(&reference.profile_feature_id).ok_or(
                        CanonicalError::Sketch(SketchError::InvalidPlanarFaceSupport),
                    )?;
                    let feature_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == feature.id)
                        .expect("validated definition contains feature");
                    let producer_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == reference.producer_feature_id);
                    let profile_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == reference.profile_feature_id);
                    let evidence = product
                        .exact_reference_evidence
                        .get(&reference.lineage_digest);
                    let evidence_and_frame_are_valid = match health {
                        WorkplaneSupportHealth::Resolved => {
                            evidence.is_some_and(|evidence| evidence.as_ref() == reference.as_ref())
                                && supported_planar_face_frame(product, reference).is_some_and(
                                    |face| {
                                        lies_on_planar_face(
                                            spec.frame,
                                            face,
                                            product.tolerance.linear_mm(),
                                        )
                                    },
                                )
                        }
                        WorkplaneSupportHealth::Ambiguous
                        | WorkplaneSupportHealth::Lost
                        | WorkplaneSupportHealth::Stale => evidence.is_none(),
                    };
                    if !evidence_and_frame_are_valid
                        || reference.document_id != product.document_id
                        || reference.definition_id != feature.definition_id
                        || producer.definition_id != feature.definition_id
                        || profile.definition_id != feature.definition_id
                        || !feature_kind_is_solid(&producer.kind)
                        || producer_position.is_none_or(|position| position >= feature_position)
                        || profile_position.is_none_or(|position| {
                            producer_position.is_none_or(|producer| position >= producer)
                        })
                    {
                        return Err(CanonicalError::Sketch(
                            SketchError::InvalidPlanarFaceSupport,
                        ));
                    }
                }
                WorkplaneSupport::ConstructionPlane { feature: support } => {
                    let support_feature = product.features.get(support).ok_or(
                        CanonicalError::Sketch(SketchError::MissingWorkplaneSupport(*support)),
                    )?;
                    let FeatureKind::ConstructionPlane {
                        origin_mm,
                        normal,
                        x_direction,
                    } = &support_feature.kind
                    else {
                        return Err(CanonicalError::Sketch(
                            SketchError::MissingWorkplaneSupport(*support),
                        ));
                    };
                    let feature_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == feature.id)
                        .expect("validated definition contains feature");
                    let support_precedes = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == *support)
                        .is_some_and(|position| position < feature_position);
                    let expected_frame =
                        WorkplaneFrame::from_construction_plane(*origin_mm, *normal, *x_direction)
                            .map_err(CanonicalError::Sketch)?;
                    if support_feature.definition_id != feature.definition_id
                        || !support_precedes
                        || spec.frame != expected_frame
                    {
                        return Err(CanonicalError::Sketch(SketchError::WorkplaneCycle(
                            feature.id,
                        )));
                    }
                }
                WorkplaneSupport::Offset { base, distance } => {
                    let base_feature = product.features.get(base).ok_or(CanonicalError::Sketch(
                        SketchError::MissingWorkplaneSupport(*base),
                    ))?;
                    let FeatureKind::Workplane(base_spec) = &base_feature.kind else {
                        return Err(CanonicalError::Sketch(
                            SketchError::MissingWorkplaneSupport(*base),
                        ));
                    };
                    let feature_position = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == feature.id)
                        .expect("validated definition contains feature");
                    let base_precedes = definition
                        .feature_ids
                        .iter()
                        .position(|candidate| *candidate == *base)
                        .is_some_and(|position| position < feature_position);
                    if base_feature.definition_id != feature.definition_id
                        || !base_precedes
                        || spec.frame != base_spec.frame.offset(distance.millimetres())
                    {
                        return Err(CanonicalError::Sketch(SketchError::WorkplaneCycle(
                            feature.id,
                        )));
                    }
                }
            },
            FeatureKind::Sketch(spec) => {
                let workplane =
                    product
                        .features
                        .get(&spec.workplane)
                        .ok_or(CanonicalError::Sketch(
                            SketchError::MissingWorkplaneSupport(spec.workplane),
                        ))?;
                let feature_position = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == feature.id)
                    .expect("validated definition contains feature");
                let workplane_precedes = definition
                    .feature_ids
                    .iter()
                    .position(|candidate| *candidate == spec.workplane)
                    .is_some_and(|position| position < feature_position);
                if workplane.definition_id != feature.definition_id
                    || !matches!(workplane.kind, FeatureKind::Workplane(_))
                    || !workplane_precedes
                {
                    return Err(CanonicalError::Sketch(
                        SketchError::MissingWorkplaneSupport(spec.workplane),
                    ));
                }
            }
            FeatureKind::Profile { .. }
            | FeatureKind::SheetMetal(_)
            | FeatureKind::SpatialPath { .. }
            | FeatureKind::ConstructionPoint { .. }
            | FeatureKind::ConstructionAxis { .. }
            | FeatureKind::ConstructionPlane { .. } => {}
        }
    }
    Ok(())
}

fn validate_feature_parameters(product: &ProductModel) -> Result<(), CanonicalError> {
    for (target, binding) in &product.feature_parameter_bindings {
        let feature = product
            .features
            .get(&target.feature_id)
            .ok_or(CanonicalError::FeatureNotFound(target.feature_id))?;
        if binding.target != *target
            || !feature_supports_parameter_target(&feature.kind, target)
            || feature_parameter_value_bits(product, target).is_none()
            || resolve_derived_identity(&product.evaluator_nodes, &binding.derived_from)
                != SlotResolution::Resolved
        {
            return Err(CanonicalError::InvalidFeatureParameterBinding(
                target.clone(),
            ));
        }
    }
    for (target, provenance) in &product.feature_parameter_provenance {
        if !product.feature_parameter_bindings.contains_key(target)
            || provenance.input_digest.is_empty()
            || provenance.result_digest.is_empty()
            || provenance.identity.validate().is_err()
        {
            return Err(CanonicalError::InvalidFeatureParameterBinding(
                target.clone(),
            ));
        }
    }
    Ok(())
}

fn validate_occurrences<'a>(
    product: &ProductModel,
    occurrences: impl Iterator<Item = &'a Arc<Occurrence>>,
) -> Result<(), CanonicalError> {
    for occurrence in occurrences {
        ensure_product_id(occurrence.id.0)?;
        ensure_name(&occurrence.name)?;
        validate_transform(occurrence.transform)?;
        if !product.definitions.contains_key(&occurrence.definition_id) {
            return Err(CanonicalError::DefinitionNotFound(occurrence.definition_id));
        }
        if let Some(parent) = occurrence.parent
            && !product.groups.contains_key(&parent)
        {
            return Err(CanonicalError::GroupNotFound(parent));
        }
        if let Some(tag) = occurrence.tag
            && !product.tags.contains_key(&tag)
        {
            return Err(CanonicalError::TagNotFound(tag));
        }
    }
    Ok(())
}

fn validate_groups(product: &ProductModel) -> Result<(), CanonicalError> {
    for group in product.groups.values() {
        ensure_product_id(group.id.0)?;
        ensure_name(&group.name)?;
        validate_transform(group.transform)?;
        if let Some(parent) = group.parent
            && !product.groups.contains_key(&parent)
        {
            return Err(CanonicalError::GroupNotFound(parent));
        }
        let mut visiting = BTreeSet::new();
        let mut cursor = Some(group.id);
        while let Some(group_id) = cursor {
            if !visiting.insert(group_id) {
                return Err(CanonicalError::GroupCycle(group_id));
            }
            cursor = product.groups[&group_id].parent;
        }
    }
    Ok(())
}

fn validate_local_structure(product: &ProductModel) -> Result<(), CanonicalError> {
    for (key, group) in &product.local_groups {
        if key != &group.key
            || !product.definitions.contains_key(&key.definition_id)
            || !product.definitions[&key.definition_id]
                .local_group_ids
                .contains(&key.local_id)
        {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        if let Some(parent) = group.parent {
            let parent_key = LocalGroupKey {
                definition_id: key.definition_id,
                local_id: parent,
            };
            if !product.local_groups.contains_key(&parent_key) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
        let mut visiting = BTreeSet::new();
        let mut cursor = Some(key.local_id);
        while let Some(local_id) = cursor {
            if !visiting.insert(local_id) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
            cursor = product.local_groups[&LocalGroupKey {
                definition_id: key.definition_id,
                local_id,
            }]
                .parent;
        }
    }
    for (key, occurrence) in &product.local_occurrences {
        if key != &occurrence.key
            || !product.definitions.contains_key(&key.definition_id)
            || !product.definitions[&key.definition_id]
                .local_occurrence_ids
                .contains(&key.local_id)
            || !product.definitions.contains_key(&occurrence.definition_id)
        {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        if let Some(parent) = occurrence.parent {
            let parent_key = LocalGroupKey {
                definition_id: key.definition_id,
                local_id: parent,
            };
            if !product.local_groups.contains_key(&parent_key) {
                return Err(CanonicalError::InvalidLocalGraph);
            }
        }
        if let Some(tag) = occurrence.tag
            && !product.tags.contains_key(&tag)
        {
            return Err(CanonicalError::TagNotFound(tag));
        }
    }
    let feature_graph = FeatureDependencyGraph::from_product(product)?;
    for ((definition_id, body_id), suppressed) in &product.body_feature_suppression {
        if suppressed.is_empty() {
            return Err(CanonicalError::InvalidFeatureSuppression(
                *definition_id,
                *body_id,
            ));
        }
        let ordered =
            ordered_body_feature_history(product, *definition_id, *body_id, &feature_graph)?;
        let ordered_suppressed = ordered
            .into_iter()
            .filter(|id| suppressed.contains(id))
            .collect::<Vec<_>>();
        if ordered_suppressed.len() != suppressed.len() {
            return Err(CanonicalError::InvalidFeatureSuppression(
                *definition_id,
                *body_id,
            ));
        }
        validate_body_feature_suppression(
            product,
            *definition_id,
            *body_id,
            &ordered_suppressed,
            &feature_graph,
        )?;
    }
    validate_definition_ownership_graph(product)
}

pub(super) fn validate_definition_ownership_graph(
    product: &ProductModel,
) -> Result<(), CanonicalError> {
    fn visit(
        definition_id: DefinitionId,
        product: &ProductModel,
        visiting: &mut BTreeSet<DefinitionId>,
        visited: &mut BTreeSet<DefinitionId>,
    ) -> Result<(), CanonicalError> {
        if visited.contains(&definition_id) {
            return Ok(());
        }
        if !visiting.insert(definition_id) {
            return Err(CanonicalError::InvalidLocalGraph);
        }
        let definition = &product.definitions[&definition_id];
        for local_id in &definition.local_occurrence_ids {
            let target = product.local_occurrences[&LocalOccurrenceKey {
                definition_id,
                local_id: *local_id,
            }]
                .definition_id;
            visit(target, product, visiting, visited)?;
        }
        visiting.remove(&definition_id);
        visited.insert(definition_id);
        Ok(())
    }

    let mut visited = BTreeSet::new();
    for definition_id in product.definitions.keys().cloned() {
        visit(definition_id, product, &mut BTreeSet::new(), &mut visited)?;
    }
    Ok(())
}

#[cfg(test)]
mod incremental_equivalence_tests {
    use super::*;
    use ketchup_geometry::sketch::{
        FeatureDirection, FeatureExtent, PadOperation, PadProfile, PadSpec, PrincipalPlane,
        SketchEntity, SketchEntityId, WorkplaneSpec,
    };

    type Mutation = (&'static str, fn(&mut ProductModel));

    fn dimension(value: f64) -> Dimension {
        Dimension::new(value.to_string(), value).unwrap()
    }

    /// A valid product with two part definitions, tags, nested groups and occurrences.
    fn product() -> ProductModel {
        let mut commands = vec![
            CanonicalCommand::CreateTag {
                id: TagId(1),
                name: "shelves".into(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: TagId(2),
                name: "doors".into(),
                visible: true,
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(1),
                name: "cabinet".into(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(2),
                name: "drawer".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(1)),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "width".into(),
                dimension: dimension(400.0),
                dependencies: vec![],
            },
        ];
        for definition in 1..=2_u64 {
            let base = definition * 10;
            let width = 300.0 + definition as f64;
            let points = [[0.0, 0.0], [width, 0.0], [width, 200.0], [0.0, 200.0]];
            let sketch = SketchSpec {
                workplane: FeatureId(base),
                entities: (0..4)
                    .map(|i| SketchEntity::Line {
                        id: SketchEntityId(i as u64 + 1),
                        start_mm: points[i],
                        end_mm: points[(i + 1) % 4],
                    })
                    .collect(),
                constraints: vec![],
            };
            let region = sketch.solved_regions().unwrap()[0].id;
            commands.push(CanonicalCommand::CreateDefinition {
                id: DefinitionId(definition),
                name: format!("part {definition}"),
            });
            for (id, kind) in [
                (
                    base,
                    FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
                ),
                (base + 1, FeatureKind::Sketch(sketch)),
                (
                    base + 2,
                    FeatureKind::Pad(PadSpec {
                        profile: PadProfile::SketchRegion {
                            sketch: FeatureId(base + 1),
                            region,
                        },
                        direction: FeatureDirection::AlongNormal,
                        extent: FeatureExtent::Blind(dimension(18.0)),
                        operation: PadOperation::NewBody,
                    }),
                ),
            ] {
                commands.push(CanonicalCommand::CreateFeature {
                    id: FeatureId(id),
                    definition_id: DefinitionId(definition),
                    name: format!("feature {id}"),
                    kind,
                });
            }
        }
        for id in 1..=4_u64 {
            commands.push(CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(id),
                definition_id: DefinitionId(1 + id % 2),
                name: format!("part {id}"),
                transform: Transform::from_translation(id as f64 * 500.0, 0.0, 0.0).unwrap(),
                parent: (id > 1).then_some(GroupId(1 + id % 2)),
                tag: (id > 2).then_some(TagId(1 + id % 2)),
                visible: true,
            });
        }
        let mut store = DocumentStore::new();
        store.apply_batch(&CommandBatch::new(commands)).unwrap();
        (*store.current().product).clone()
    }

    /// Edits the entry under `key` if an earlier edit kept it.
    fn edit<K: Ord, V: Clone>(map: &mut BTreeMap<K, Arc<V>>, key: K, change: impl FnOnce(&mut V)) {
        if let Some(entry) = map.get_mut(&key) {
            change(Arc::make_mut(entry));
        }
    }

    /// Edits as `apply_batch` makes them: a kept entry stays the same allocation, an edited
    /// one is copied on write. Each is valid or names one way the product can break.
    const MUTATIONS: &[Mutation] = &[
        ("move an occurrence", |p| {
            edit(&mut p.occurrences, OccurrenceId(2), |o| {
                o.transform = Transform::from_translation(7.0, 8.0, 9.0).unwrap();
            });
        }),
        ("rename an occurrence", |p| {
            edit(&mut p.occurrences, OccurrenceId(3), |o| {
                o.name = "renamed".into()
            });
        }),
        ("empty occurrence name", |p| {
            edit(&mut p.occurrences, OccurrenceId(3), |o| o.name = " ".into());
        }),
        ("singular occurrence transform", |p| {
            edit(&mut p.occurrences, OccurrenceId(4), |o| {
                o.transform.matrix = [0.0; 16]
            });
        }),
        ("occurrence uses a missing definition", |p| {
            edit(&mut p.occurrences, OccurrenceId(1), |o| {
                o.definition_id = DefinitionId(99)
            });
        }),
        ("occurrence repointed to the other definition", |p| {
            edit(&mut p.occurrences, OccurrenceId(1), |o| {
                o.definition_id = DefinitionId(2)
            });
        }),
        ("occurrence in a missing group", |p| {
            edit(&mut p.occurrences, OccurrenceId(2), |o| {
                o.parent = Some(GroupId(99))
            });
        }),
        ("occurrence with a missing tag", |p| {
            edit(&mut p.occurrences, OccurrenceId(2), |o| {
                o.tag = Some(TagId(99))
            });
        }),
        ("add an occurrence", |p| {
            let mut added = (*p.occurrences[&OccurrenceId(2)]).clone();
            added.id = OccurrenceId(5);
            p.occurrences.insert(added.id, Arc::new(added));
        }),
        ("remove an occurrence", |p| {
            p.occurrences.remove(&OccurrenceId(4));
        }),
        ("remove a used definition", |p| {
            p.definitions.remove(&DefinitionId(2));
        }),
        ("remove a used tag", |p| {
            p.tags.remove(&TagId(1));
        }),
        ("remove a used group", |p| {
            p.groups.remove(&GroupId(2));
        }),
        ("group cycle", |p| {
            edit(&mut p.groups, GroupId(1), |g| g.parent = Some(GroupId(2)));
        }),
        ("empty group name", |p| {
            edit(&mut p.groups, GroupId(2), |g| g.name = String::new());
        }),
        ("empty definition name", |p| {
            edit(&mut p.definitions, DefinitionId(1), |d| {
                d.name = String::new()
            });
        }),
        ("empty feature name", |p| {
            edit(&mut p.features, FeatureId(12), |f| f.name = String::new());
        }),
        ("feature moved to the other definition", |p| {
            edit(&mut p.features, FeatureId(12), |f| {
                f.definition_id = DefinitionId(2)
            });
        }),
        ("tolerance change", |p| {
            p.tolerance = TolerancePolicy::default()
        }),
        ("rename a tag", |p| {
            edit(&mut p.tags, TagId(2), |t| t.name = "fronts".into());
        }),
    ];

    /// Validating only what an edit changed accepts and refuses exactly what validating the
    /// whole product does, with the same error, for every edit and every pair of edits.
    #[test]
    fn changed_entries_validation_matches_full_validation() {
        let before = product();
        validate_product(&before).unwrap();
        let mut refused = 0;
        for (index, (first, apply_first)) in MUTATIONS.iter().enumerate() {
            for (second, apply_second) in
                std::iter::once(&("nothing else", (|_| {}) as fn(&mut ProductModel)))
                    .chain(&MUTATIONS[index + 1..])
            {
                let mut product = before.clone();
                apply_first(&mut product);
                apply_second(&mut product);
                let full = validate_product(&product);
                refused += usize::from(full.is_err());
                assert_eq!(
                    validate_product_change(Some(&before), &product, true),
                    full,
                    "{first} + {second}"
                );
            }
        }
        assert!(
            refused > MUTATIONS.len(),
            "the edits must exercise refusals"
        );
    }
}
