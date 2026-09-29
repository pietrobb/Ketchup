//! The document and feature digests as computed up to native schema 97.
//!
//! Frozen: files written before the serde snapshot format store these digests (a mesh
//! converted from an exact body names its source revision, an assembly recipe names the
//! features it owns). Loading such a file computes both digests once and replaces every
//! stored old value by the current one. Only the legacy load path calls this module.

use super::stable_digest::{digest_feature, digest_snapshot};
use super::*;

/// A product read from a file written before native schema 98, with its stored digests
/// replaced by current ones.
pub(crate) struct MigratedDigests {
    pub(crate) product: ProductModel,
    /// The document digest the writing version computed.
    pub(crate) source_digest: String,
}

/// Replaces every stored digest of the old definition by the current one.
///
/// `earlier` maps the old document digests of revisions already loaded from the same file
/// to their current digests; this revision's pair is added to it. `to_current` rewrites
/// what the old definition named differently; it runs after the old digests are taken.
pub(crate) fn migrate_stored_digests(
    product: ProductModel,
    point_profiles: &BTreeSet<FeatureId>,
    earlier: &mut BTreeMap<String, String>,
    to_current: impl FnOnce(&ProductModel) -> Result<ProductModel, ciborium::value::Error>,
) -> Result<MigratedDigests, ciborium::value::Error> {
    let source_digest = document_digest(&product, point_profiles);
    let migrated = replace_stored_text(&to_current(&product)?, earlier)?;
    let features = product
        .features
        .values()
        .zip(migrated.features.values())
        .map(|(source, current)| {
            (
                feature_digest(source, point_profiles),
                digest_feature(current),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let product = replace_stored_text(&migrated, &features)?;
    let current = digest_snapshot(&Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    });
    earlier.insert(source_digest.clone(), current);
    Ok(MigratedDigests {
        product,
        source_digest,
    })
}

fn replace_stored_text(
    product: &ProductModel,
    replacements: &BTreeMap<String, String>,
) -> Result<ProductModel, ciborium::value::Error> {
    fn visit(value: &mut ciborium::Value, replacements: &BTreeMap<String, String>) {
        match value {
            ciborium::Value::Text(text) => {
                if let Some(replacement) = replacements.get(text.as_str()) {
                    text.clone_from(replacement);
                }
            }
            ciborium::Value::Array(items) => {
                for item in items {
                    visit(item, replacements);
                }
            }
            ciborium::Value::Map(entries) => {
                for (key, item) in entries {
                    visit(key, replacements);
                    visit(item, replacements);
                }
            }
            ciborium::Value::Tag(_, item) => visit(item, replacements),
            _ => {}
        }
    }
    if replacements.is_empty() {
        return Ok(product.clone());
    }
    let mut value = ciborium::Value::serialized(product)?;
    visit(&mut value, replacements);
    value.deserialized()
}

fn document_digest(product: &ProductModel, point_profiles: &BTreeSet<FeatureId>) -> String {
    let mut digest = DigestV3::new();
    digest.bytes(b"ketchup.document.v3");
    digest.u64(product.document_id.0);
    digest.byte(match product.units {
        UnitSystem::Millimetres => 1,
    });
    digest.u64(product.evaluator_nodes.len() as u64);
    for node in product.evaluator_nodes.values() {
        digest.node(node);
    }
    digest.u64(product.overrides.len() as u64);
    for value in product.overrides.values() {
        digest.canonical_override(value);
    }
    digest.u64(product.feature_parameter_bindings.len() as u64);
    for binding in product.feature_parameter_bindings.values() {
        digest.feature_parameter_binding(binding);
    }
    if !product.feature_parameter_provenance.is_empty() {
        digest.bytes(b"feature-parameter-provenance.v1");
        digest.u64(product.feature_parameter_provenance.len() as u64);
        for (target, provenance) in &product.feature_parameter_provenance {
            digest.feature_parameter_target(target);
            digest.feature_parameter_provenance(provenance);
        }
    }
    digest.u64(product.joints.len() as u64);
    for joint in product.joints.values() {
        digest.joint(joint);
    }
    digest.u64(product.spaces.len() as u64);
    for space in product.spaces.values() {
        digest.space(space);
    }
    digest.u64(product.clearance_volumes.len() as u64);
    for clearance in product.clearance_volumes.values() {
        digest.clearance_volume(clearance);
    }
    if !product.cam_plans.is_empty() {
        digest.bytes(b"canonical-cam-plans.v1");
        digest.u64(product.cam_plans.len() as u64);
        for plan in product.cam_plans.values() {
            digest.cam_plan(plan);
        }
    }
    if !product.dowel_joints.is_empty() {
        digest.bytes(b"canonical-dowel-joints.v1");
        digest.u64(product.dowel_joints.len() as u64);
        for joint in product.dowel_joints.values() {
            digest.dowel_joint(joint);
        }
    }
    if let Some(recipe) = product.assembly_recipe.as_deref() {
        digest.bytes(b"canonical-assembly-recipe.v1");
        digest.assembly_recipe(recipe, &product.features);
    }
    digest.u64(product.persistent_dimensions.len() as u64);
    for dimension in product.persistent_dimensions.values() {
        digest.persistent_dimension(dimension);
    }
    digest.u64(product.tags.len() as u64);
    for tag in product.tags.values() {
        digest.tag(tag);
    }
    digest.u64(product.classification_dimensions.len() as u64);
    for dimension in product.classification_dimensions.values() {
        digest.u64(dimension.id.0);
        digest.bytes(dimension.name.as_bytes());
        digest.u64(dimension.categories.len() as u64);
        for category in dimension.categories.values() {
            digest.u64(category.id.0);
            digest.bytes(category.name.as_bytes());
        }
    }
    digest.u64(product.classification_assignments.len() as u64);
    for ((occurrence_id, dimension_id), category_id) in &product.classification_assignments {
        digest.u64(occurrence_id.0);
        digest.u64(dimension_id.0);
        digest.u64(category_id.0);
    }
    digest.u64(product.collections.len() as u64);
    for collection in product.collections.values() {
        digest.collection(collection);
    }
    digest.u64(product.import_receipts.len() as u64);
    for receipt in product.import_receipts.values() {
        digest.import_receipt(receipt);
    }
    digest.u64(product.definitions.len() as u64);
    for definition in product.definitions.values() {
        digest.definition(definition);
    }
    digest.u64(product.features.len() as u64);
    for feature in product.features.values() {
        digest.feature(feature, point_profiles);
    }
    digest.u64(product.body_feature_suppression.len() as u64);
    for ((definition_id, body_id), suppressed) in &product.body_feature_suppression {
        digest.u64(definition_id.0);
        digest.u64(body_id.0);
        digest.u64(suppressed.len() as u64);
        for feature_id in suppressed {
            digest.u64(feature_id.0);
        }
    }
    digest.u64(product.occurrences.len() as u64);
    for occurrence in product.occurrences.values() {
        digest.occurrence(occurrence);
    }
    digest.u64(product.grounded_occurrences.len() as u64);
    for occurrence_id in &product.grounded_occurrences {
        digest.u64(occurrence_id.0);
    }
    digest.u64(product.assembly_mates.len() as u64);
    for mate in product.assembly_mates.values() {
        digest.assembly_mate(mate);
    }
    if !product.assembly_joints.is_empty() {
        digest.bytes(b"canonical-assembly-joints.v1");
        digest.u64(product.assembly_joints.len() as u64);
        for joint in product.assembly_joints.values() {
            digest.assembly_joint(joint);
        }
    }
    if !product.assembly_motion_couplings.is_empty() {
        digest.bytes(b"canonical-assembly-motion-couplings.v1");
        digest.u64(product.assembly_motion_couplings.len() as u64);
        for coupling in product.assembly_motion_couplings.values() {
            digest.assembly_motion_coupling(coupling);
        }
    }
    if !product.mechanical_interfaces.is_empty() {
        digest.bytes(b"canonical-mechanical-interfaces.v1");
        digest.u64(product.mechanical_interfaces.len() as u64);
        for interface in product.mechanical_interfaces.values() {
            digest.mechanical_interface(interface);
        }
    }
    if !product.mechanical_conditions.is_empty() {
        digest.bytes(b"canonical-mechanical-conditions.v1");
        digest.u64(product.mechanical_conditions.len() as u64);
        for condition in product.mechanical_conditions.values() {
            digest.mechanical_condition(condition);
        }
    }
    if !product.assembly_motion_studies.is_empty() {
        digest.bytes(b"canonical-assembly-motion-studies.v1");
        digest.u64(product.assembly_motion_studies.len() as u64);
        for study in product.assembly_motion_studies.values() {
            digest.assembly_motion_study(study);
        }
    }
    if !product.drawing_sheets.is_empty() {
        digest.bytes(b"canonical-drawing-sheets.v1");
        digest.u64(product.drawing_sheets.len() as u64);
        for sheet in product.drawing_sheets.values() {
            digest.drawing_sheet(sheet);
        }
    }
    digest.u64(product.groups.len() as u64);
    for group in product.groups.values() {
        digest.group(group);
    }
    digest.u64(product.local_groups.len() as u64);
    for group in product.local_groups.values() {
        digest.local_group(group);
    }
    digest.u64(product.local_occurrences.len() as u64);
    for occurrence in product.local_occurrences.values() {
        digest.local_occurrence(occurrence);
    }
    if !product.instance_transform_overrides.is_empty() {
        digest.bytes(b"canonical-instance-transform-overrides.v1");
        digest.u64(product.instance_transform_overrides.len() as u64);
        for (path, transform) in &product.instance_transform_overrides {
            digest.instance_path(path);
            digest.transform(*transform);
        }
    }
    if !product.production_codes.is_empty() {
        digest.bytes(b"canonical-production-codes.v1");
        digest.u64(product.production_codes.len() as u64);
        for (path, code) in &product.production_codes {
            digest.instance_path(path);
            digest.bytes(code.as_bytes());
        }
    }
    digest.finish()
}

fn feature_digest(feature: &Feature, point_profiles: &BTreeSet<FeatureId>) -> String {
    let mut digest = DigestV3::new();
    digest.bytes(b"ketchup.feature.v1");
    digest.feature(feature, point_profiles);
    digest.finish()
}

struct DigestV3(u64);

impl DigestV3 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    const fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(Self::PRIME);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        for byte in bytes {
            self.byte(*byte);
        }
    }

    fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    fn node(&mut self, node: &EvaluatorNode) {
        self.bytes(&node.canonical_spec_bytes());
    }

    fn slot_path(&mut self, path: &SlotPath) {
        self.u64(path.segments().len() as u64);
        for segment in path.segments() {
            self.u64(segment.producer_rule_id.0);
            self.bytes(segment.output_port.as_bytes());
            self.bytes(segment.semantic_key.as_bytes());
        }
    }

    fn canonical_override(&mut self, value: &CanonicalOverride) {
        self.u64(value.id);
        self.u64(value.target.root_rule_node_id.0);
        self.slot_path(&value.target.slot_path);
        self.bytes(value.parameter.as_bytes());
        self.u64(value.value_bits);
        match value.health {
            SlotResolution::Resolved => self.byte(1),
            SlotResolution::Ambiguous { segment_index } => {
                self.byte(2);
                self.u64(segment_index as u64);
            }
            SlotResolution::Lost { segment_index } => {
                self.byte(3);
                self.u64(segment_index as u64);
            }
        }
    }

    fn feature_parameter_target(&mut self, target: &FeatureParameterTarget) {
        self.u64(target.feature_id.0);
        self.bytes(target.path.as_str().as_bytes());
        self.byte(match target.value_type {
            ParameterValueType::Length => 1,
            ParameterValueType::Angle => 2,
            ParameterValueType::Scalar => 3,
        });
    }

    fn feature_parameter_binding(&mut self, binding: &FeatureParameterBinding) {
        self.feature_parameter_target(&binding.target);
        self.u64(binding.derived_from.root_rule_node_id.0);
        self.slot_path(&binding.derived_from.slot_path);
    }

    fn feature_parameter_provenance(&mut self, provenance: &FeatureParameterProvenance) {
        self.bytes(provenance.identity.evaluator.as_bytes());
        self.bytes(provenance.identity.schema.as_bytes());
        self.bytes(provenance.identity.tolerance.as_bytes());
        if let Some(backend) = provenance.identity.backend.as_ref() {
            self.byte(1);
            self.bytes(backend.as_bytes());
        } else {
            self.byte(0);
        }
        self.bytes(provenance.input_digest.as_bytes());
        self.bytes(provenance.result_digest.as_bytes());
        self.u64(provenance.applied_value_bits);
    }

    fn joint(&mut self, joint: &CanonicalJoint) {
        self.u64(joint.id().0);
        self.u64(joint.participant_a().root_rule_node_id.0);
        self.slot_path(&joint.participant_a().slot_path);
        self.u64(joint.participant_b().root_rule_node_id.0);
        self.slot_path(&joint.participant_b().slot_path);
        for value in joint.volume().min().into_iter().chain(joint.volume().max()) {
            self.u64(value.to_bits());
        }
    }

    fn space(&mut self, space: &CanonicalSpace) {
        self.u64(space.id().0);
        self.bytes(space.purpose().as_bytes());
        for value in space.volume().min().into_iter().chain(space.volume().max()) {
            self.u64(value.to_bits());
        }
        self.u64(space.adjacent_to().len() as u64);
        for id in space.adjacent_to() {
            self.u64(id.0);
        }
        self.u64(space.accessible_to().len() as u64);
        for id in space.accessible_to() {
            self.u64(id.0);
        }
    }

    fn clearance_volume(&mut self, clearance: &CanonicalClearanceVolume) {
        self.u64(clearance.id().0);
        match clearance.owner() {
            ClearanceOwner::Occurrence(path) => {
                self.byte(1);
                self.u64(path.root_occurrence().0);
                self.u64(path.steps().len() as u64);
                for step in path.steps() {
                    match step {
                        InstancePathStep::Group(id) => {
                            self.byte(1);
                            self.u64(id.0);
                        }
                        InstancePathStep::Occurrence(id) => {
                            self.byte(2);
                            self.u64(id.0);
                        }
                    }
                }
            }
            ClearanceOwner::Space(id) => {
                self.byte(2);
                self.u64(id.0);
            }
        }
        self.bytes(clearance.reason().as_bytes());
        for value in clearance
            .volume()
            .min()
            .into_iter()
            .chain(clearance.volume().max())
        {
            self.u64(value.to_bits());
        }
        self.byte(match clearance.coordinate_frame() {
            ClearanceCoordinateFrame::World => 1,
        });
        self.u64(clearance.tolerance().epsilon_mm().to_bits());
        self.byte(match clearance.severity() {
            ClearanceSeverity::Advisory => 1,
            ClearanceSeverity::Required => 2,
        });
        if let Some(identity) = clearance.derived_from() {
            self.byte(1);
            self.u64(identity.root_rule_node_id.0);
            self.slot_path(&identity.slot_path);
        } else {
            self.byte(0);
        }
    }

    fn cam_plan(&mut self, plan: &crate::cam::CamPlan) {
        self.u64(plan.id().0);
        self.bytes(plan.name().as_bytes());
        self.byte(match plan.units() {
            crate::cam::CamUnits::Millimetres => 1,
        });
        self.u64(plan.target().definition_id.0);
        self.u64(plan.target().feature_id.0);
        self.bytes(plan.target().exact_graph_digest.as_bytes());
        for value in plan
            .stock()
            .minimum_mm
            .into_iter()
            .chain(plan.stock().maximum_mm)
        {
            self.u64(value.to_bits());
        }
        self.u64(u64::from(plan.tool().number));
        self.byte(match plan.tool().kind {
            crate::cam::CamToolKind::FlatEndMill => 1,
            crate::cam::CamToolKind::BallEndMill => 2,
            crate::cam::CamToolKind::Drill => 3,
        });
        for value in [
            plan.tool().diameter_mm,
            plan.tool().flute_length_mm,
            plan.tool().overall_length_mm,
            plan.tool().holder_diameter_mm,
            plan.tool().holder_length_mm,
            plan.tool().feed_mm_per_min,
            plan.tool().plunge_mm_per_min,
        ] {
            self.u64(value.to_bits());
        }
        self.u64(u64::from(plan.tool().spindle_rpm));
        self.byte(match plan.setup().work_offset {
            crate::cam::CamWorkOffset::G54 => 54,
            crate::cam::CamWorkOffset::G55 => 55,
            crate::cam::CamWorkOffset::G56 => 56,
            crate::cam::CamWorkOffset::G57 => 57,
            crate::cam::CamWorkOffset::G58 => 58,
            crate::cam::CamWorkOffset::G59 => 59,
        });
        for value in plan
            .setup()
            .origin_mm
            .into_iter()
            .chain(plan.setup().x_axis)
            .chain(plan.setup().y_axis)
            .chain([plan.setup().safe_height_mm])
        {
            self.u64(value.to_bits());
        }
        for value in [
            plan.cut_parameters().maximum_stepdown_mm,
            plan.cut_parameters().stepover_ratio,
            plan.cut_parameters().radial_allowance_mm,
            plan.cut_parameters().axial_allowance_mm,
        ] {
            self.u64(value.to_bits());
        }
    }

    fn dowel_joint(&mut self, joint: &crate::joinery::DowelJointContract) {
        self.u64(joint.id.0);
        self.bytes(joint.name.as_bytes());
        for side in [&joint.first, &joint.second] {
            self.instance_path(&side.instance_path);
            for value in side
                .face_origin_local_mm
                .into_iter()
                .chain(side.inward_unit_local)
                .chain(side.bounds_min_local_mm)
                .chain(side.bounds_max_local_mm)
            {
                self.u64(value.to_bits());
            }
        }
        for value in joint
            .first_center_local_mm
            .into_iter()
            .chain(joint.row_unit_first_local)
            .chain([
                joint.spacing_mm,
                joint.dowel.diameter_mm,
                joint.dowel.length_mm,
                joint.dowel.first_insertion_mm,
                joint.dowel.second_insertion_mm,
                joint.dowel.bottom_clearance_mm,
            ])
        {
            self.u64(value.to_bits());
        }
        self.u64(u64::from(joint.count));
        if !joint.pair_offsets_first_local_mm.is_empty() {
            self.u64(joint.pair_offsets_first_local_mm.len() as u64);
            for offset in &joint.pair_offsets_first_local_mm {
                for value in offset {
                    self.u64(value.to_bits());
                }
            }
        }
        match &joint.physical_hole_pairs {
            None => self.byte(0),
            Some(pairs) => {
                self.byte(1);
                self.u64(pairs.len() as u64);
                for pair in pairs {
                    self.u64(pair.first_pocket_feature_id.0);
                    self.u64(pair.second_pocket_feature_id.0);
                }
            }
        }
    }

    fn assembly_recipe(
        &mut self,
        recipe: &crate::assembly_recipe::AssemblyRecipe,
        features: &BTreeMap<FeatureId, Arc<Feature>>,
    ) {
        use crate::assembly_recipe::{
            RecipeEditScope, RecipeParameterUnit, RecipePartMobility, RecipeRelationKind,
            RecognizedRecipeFeatureKind,
        };

        self.bytes(recipe.schema.as_bytes());
        self.bytes(recipe.key.as_str().as_bytes());
        self.u64(recipe.parts.len() as u64);
        for part in recipe.parts.values() {
            self.bytes(part.key.as_str().as_bytes());
            self.instance_path(&part.instance_path);
            self.u64(part.definition_id.0);
            self.transform(part.placement);
            self.byte(match part.mobility {
                RecipePartMobility::Fixed => 1,
                RecipePartMobility::Movable => 2,
            });
            match &part.edit_scope {
                RecipeEditScope::Occurrence(path) => {
                    self.byte(1);
                    self.instance_path(path);
                }
                RecipeEditScope::SharedDefinition(id) => {
                    self.byte(2);
                    self.u64(id.0);
                }
            }
            self.u64(part.parameters.len() as u64);
            for (key, parameter) in &part.parameters {
                self.bytes(key.as_str().as_bytes());
                self.u64(parameter.value.to_bits());
                self.byte(match parameter.unit {
                    RecipeParameterUnit::Millimetres => 1,
                    RecipeParameterUnit::Degrees => 2,
                    RecipeParameterUnit::Scalar => 3,
                });
                match &parameter.target {
                    Some(target) => {
                        self.byte(1);
                        self.feature_parameter_target(target);
                    }
                    None => self.byte(0),
                }
            }
        }
        self.u64(recipe.relations.len() as u64);
        for relation in recipe.relations.values() {
            self.bytes(relation.key.as_str().as_bytes());
            self.byte(match relation.kind {
                RecipeRelationKind::Contact => 1,
                RecipeRelationKind::Coincident => 2,
            });
            for face in [&relation.first, &relation.second] {
                self.bytes(face.part.as_str().as_bytes());
                self.bytes(face.role.as_bytes());
            }
        }
        self.u64(recipe.joinery.len() as u64);
        for joinery in recipe.joinery.values() {
            self.bytes(joinery.key.as_str().as_bytes());
            self.bytes(joinery.first_part.as_str().as_bytes());
            self.bytes(joinery.second_part.as_str().as_bytes());
            self.u64(joinery.dowel_joint_id.0);
        }
        self.u64(recipe.owned_features.len() as u64);
        for owned in recipe.owned_features.values() {
            self.bytes(owned.key.as_str().as_bytes());
            self.bytes(owned.part.as_str().as_bytes());
            self.u64(owned.feature_id.0);
            self.byte(match owned.kind {
                RecognizedRecipeFeatureKind::Profile => 1,
                RecognizedRecipeFeatureKind::Pad => {
                    use crate::sketch::{PadOperation, PadProfile};
                    // The old recipe told the four pad records apart by kind.
                    match features.get(&owned.feature_id).map(|feature| &feature.kind) {
                        Some(FeatureKind::Pad(spec)) => match (spec.profile, &spec.operation) {
                            (PadProfile::Feature(_), PadOperation::NewBody) => 2,
                            (PadProfile::Feature(_), PadOperation::Cut { .. }) => 3,
                            (PadProfile::SketchRegion { .. }, PadOperation::Cut { .. }) => 6,
                            (PadProfile::SketchRegion { .. }, PadOperation::NewBody) => 7,
                        },
                        _ => 7,
                    }
                }
                RecognizedRecipeFeatureKind::Workplane => 4,
                RecognizedRecipeFeatureKind::Sketch => 5,
            });
            self.bytes(owned.canonical_fingerprint.as_bytes());
        }
    }

    fn persistent_dimension(&mut self, dimension: &PersistentDimension) {
        self.u64(dimension.id.0);
        self.bytes(dimension.name.as_bytes());
        match &dimension.target {
            PersistentDimensionTarget::FeatureParameter(target) => {
                self.byte(1);
                self.feature_parameter_target(target);
            }
            PersistentDimensionTarget::DerivedOutput(target) => {
                self.byte(2);
                self.u64(target.root_rule_node_id.0);
                self.slot_path(&target.slot_path);
            }
            PersistentDimensionTarget::ExactFeatureParameter {
                definition_id,
                producer_feature_id,
                semantic_role,
                source_element_id,
                path,
                value_type,
            } => {
                self.byte(3);
                self.u64(definition_id.0);
                self.feature_parameter_target(&FeatureParameterTarget {
                    feature_id: *producer_feature_id,
                    path: path.clone(),
                    value_type: *value_type,
                });
                self.bytes(semantic_role.as_bytes());
                self.bytes(source_element_id.as_bytes());
            }
        }
        self.byte(match dimension.presentation.unit {
            DimensionDisplayUnit::Millimetres => 1,
            DimensionDisplayUnit::Centimetres => 2,
            DimensionDisplayUnit::Inches => 3,
        });
        self.byte(dimension.presentation.decimal_places);
    }

    fn tag(&mut self, tag: &Tag) {
        self.u64(tag.id.0);
        self.bytes(tag.name.as_bytes());
        self.byte(u8::from(tag.visible));
    }

    fn collection(&mut self, collection: &Collection) {
        self.u64(collection.id.0);
        self.bytes(collection.name.as_bytes());
        self.u64(collection.occurrence_ids.len() as u64);
        for occurrence_id in &collection.occurrence_ids {
            self.u64(occurrence_id.0);
        }
    }

    fn import_receipt(&mut self, receipt: &ImportReceipt) {
        self.bytes(receipt.schema().as_bytes());
        self.u64(receipt.id().0);
        self.byte(match receipt.format() {
            ImportFormat::Stl => 1,
            ImportFormat::Dxf => 2,
            ImportFormat::Step => 3,
            ImportFormat::SketchupScene => 4,
            ImportFormat::Glb => 5,
            ImportFormat::Iges => 6,
        });
        self.bytes(receipt.source_sha256());
        self.u64(receipt.source_byte_len());
        self.bytes(receipt.source_name().as_bytes());
        self.byte(match receipt.units().source_unit() {
            ImportLengthUnit::Millimetre => 1,
            ImportLengthUnit::Centimetre => 2,
            ImportLengthUnit::Metre => 3,
            ImportLengthUnit::Inch => 4,
            ImportLengthUnit::Foot => 5,
        });
        self.byte(match receipt.units().authority() {
            ImportUnitAuthority::FileDeclared => 1,
            ImportUnitAuthority::UserDeclared => 2,
        });
        self.bytes(receipt.parser_id().as_bytes());
        self.bytes(receipt.parser_version().as_bytes());
        self.u64(receipt.diagnostics().len() as u64);
        for diagnostic in receipt.diagnostics() {
            self.byte(match diagnostic.severity() {
                ImportDiagnosticSeverity::Info => 1,
                ImportDiagnosticSeverity::Warning => 2,
            });
            self.bytes(diagnostic.code().as_bytes());
            match diagnostic.subject() {
                Some(subject) => {
                    self.byte(1);
                    self.bytes(subject.as_bytes());
                }
                None => self.byte(0),
            }
            self.u64(u64::from(diagnostic.count()));
        }
        self.u64(receipt.outputs().len() as u64);
        for output in receipt.outputs() {
            match output {
                ImportOutputRef::Definition(id) => {
                    self.byte(1);
                    self.u64(id.0);
                }
                ImportOutputRef::Feature(id) => {
                    self.byte(2);
                    self.u64(id.0);
                }
                ImportOutputRef::Occurrence(id) => {
                    self.byte(3);
                    self.u64(id.0);
                }
                ImportOutputRef::Group(id) => {
                    self.byte(4);
                    self.u64(id.0);
                }
            }
        }
    }

    fn transform(&mut self, transform: Transform) {
        for value in transform.matrix {
            self.u64(value.to_bits());
        }
    }

    fn instance_path(&mut self, path: &InstancePath) {
        self.u64(path.root_occurrence().0);
        self.u64(path.steps().len() as u64);
        for step in path.steps() {
            match step {
                InstancePathStep::Group(id) => {
                    self.byte(1);
                    self.u64(id.0);
                }
                InstancePathStep::Occurrence(id) => {
                    self.byte(2);
                    self.u64(id.0);
                }
            }
        }
    }

    fn definition(&mut self, definition: &Definition) {
        self.u64(definition.id.0);
        self.bytes(definition.name.as_bytes());
        self.u64(definition.feature_ids.len() as u64);
        for feature_id in &definition.feature_ids {
            self.u64(feature_id.0);
        }
        self.u64(definition.bodies.len() as u64);
        for body in definition.bodies.values() {
            self.u64(body.id.0);
            self.bytes(body.name.as_bytes());
            self.byte(u8::from(body.visible));
            self.optional_id(body.consumed_by.map(|id| id.0));
        }
        self.u64(definition.active_body_id.0);
        self.u64(definition.feature_body_ownership.len() as u64);
        for (feature_id, ownership) in &definition.feature_body_ownership {
            self.u64(feature_id.0);
            self.u64(ownership.input_body_ids.len() as u64);
            for body_id in &ownership.input_body_ids {
                self.u64(body_id.0);
            }
            self.optional_id(ownership.output_body_id.map(|body_id| body_id.0));
        }
        self.u64(definition.local_group_ids.len() as u64);
        for id in &definition.local_group_ids {
            self.u64(id.0);
        }
        self.u64(definition.local_occurrence_ids.len() as u64);
        for id in &definition.local_occurrence_ids {
            self.u64(id.0);
        }
    }

    fn sketch_point_ref(&mut self, reference: crate::sketch::SketchPointRef) {
        self.u64(reference.entity.0);
        self.byte(match reference.point {
            SketchPointKind::Start => 1,
            SketchPointKind::End => 2,
            SketchPointKind::Center => 3,
            SketchPointKind::Control1 => 4,
            SketchPointKind::Control2 => 5,
        });
    }

    fn body_subshape_reference(&mut self, reference: &BodySubshapeRef) {
        self.u64(reference.document_id.0);
        self.u64(reference.definition_id.0);
        self.u64(reference.profile_feature_id.0);
        self.u64(reference.producer_feature_id.0);
        self.bytes(reference.semantic_role.as_bytes());
        self.bytes(reference.source_element_id.as_bytes());
        self.bytes(reference.expected_type.as_bytes());
        self.u64(u64::from(reference.expected_cardinality));
        self.byte(match reference.stability {
            crate::exact_product::ReferenceStability::Guaranteed => 1,
        });
        self.bytes(reference.lineage_digest.as_bytes());
    }

    fn profile_face(&mut self, face: &ProfileFaceReference) {
        match face {
            ProfileFaceReference::Start => self.byte(1),
            ProfileFaceReference::End => self.byte(2),
            ProfileFaceReference::Segment {
                entity_id,
                source_name,
            } => {
                self.byte(3);
                self.u64(*entity_id);
                self.bytes(source_name.as_bytes());
            }
            ProfileFaceReference::NamedResult(name) => {
                self.byte(4);
                self.bytes(name.as_bytes());
            }
        }
    }

    fn topological_reference(&mut self, reference: &TopologicalElementRef) {
        self.bytes(
            &reference
                .to_bytes()
                .expect("validated topological feature reference is serializable"),
        );
    }

    fn feature_direction(&mut self, direction: crate::sketch::FeatureDirection) {
        match direction {
            crate::sketch::FeatureDirection::AlongNormal => self.byte(1),
            crate::sketch::FeatureDirection::OppositeNormal => self.byte(2),
            crate::sketch::FeatureDirection::Vector(vector) => {
                self.byte(3);
                for component in vector {
                    self.u64(component.to_bits());
                }
            }
        }
    }

    fn feature_extent_end(&mut self, end: &crate::sketch::FeatureExtentEnd) {
        match end {
            crate::sketch::FeatureExtentEnd::Blind(distance) => {
                self.byte(1);
                self.bytes(distance.source_token().as_bytes());
                self.u64(distance.millimetres().to_bits());
            }
            crate::sketch::FeatureExtentEnd::ThroughAll => self.byte(2),
            crate::sketch::FeatureExtentEnd::UpToFace(reference) => {
                self.byte(3);
                self.body_subshape_reference(reference);
            }
        }
    }

    /// The legacy reader builds a pad only from the old extrusion, pad, sketch pocket,
    /// through cut and pocket records; each keeps the encoding of its old record.
    fn pad(&mut self, spec: &crate::sketch::PadSpec) {
        use crate::sketch::{CutStart, FeatureExtent, PadOperation, PadProfile};
        match (&spec.profile, &spec.operation, &spec.extent) {
            (PadProfile::Feature(profile), PadOperation::NewBody, FeatureExtent::Blind(height)) => {
                self.byte(2);
                self.u64(profile.0);
                self.bytes(height.source_token().as_bytes());
                self.u64(height.millimetres().to_bits());
            }
            (
                PadProfile::Feature(profile),
                PadOperation::Cut { target, .. },
                FeatureExtent::Blind(depth),
            ) => {
                self.byte(10);
                self.u64(target.0);
                self.u64(profile.0);
                self.bytes(depth.source_token().as_bytes());
                self.u64(depth.millimetres().to_bits());
            }
            (PadProfile::Feature(profile), PadOperation::Cut { target, .. }, _) => {
                self.byte(3);
                self.u64(target.0);
                self.u64(profile.0);
            }
            (profile, PadOperation::Cut { target, start }, _) => {
                self.byte(20);
                self.u64(target.0);
                self.pad_sketch_region(*profile);
                self.feature_direction(spec.direction);
                self.feature_extent(&spec.extent);
                if let CutStart::Support(support) = start {
                    self.body_subshape_reference(support);
                }
            }
            (profile, PadOperation::NewBody, _) => {
                self.byte(19);
                self.pad_sketch_region(*profile);
                self.feature_direction(spec.direction);
                self.feature_extent(&spec.extent);
            }
        }
    }

    fn pad_sketch_region(&mut self, profile: crate::sketch::PadProfile) {
        self.u64(profile.feature_id().0);
        self.u64(match profile {
            crate::sketch::PadProfile::SketchRegion { region, .. } => region.0,
            crate::sketch::PadProfile::Feature(_) => 0,
        });
    }

    fn feature_extent(&mut self, extent: &crate::sketch::FeatureExtent) {
        match extent {
            crate::sketch::FeatureExtent::Blind(distance) => {
                self.byte(1);
                self.bytes(distance.source_token().as_bytes());
                self.u64(distance.millimetres().to_bits());
            }
            crate::sketch::FeatureExtent::ThroughAll => self.byte(2),
            crate::sketch::FeatureExtent::UpToFace(reference) => {
                self.byte(3);
                self.body_subshape_reference(reference);
            }
            crate::sketch::FeatureExtent::Symmetric(distance) => {
                self.byte(4);
                self.bytes(distance.source_token().as_bytes());
                self.u64(distance.millimetres().to_bits());
            }
            crate::sketch::FeatureExtent::Bidirectional { along, opposite } => {
                self.byte(5);
                self.feature_extent_end(along);
                self.feature_extent_end(opposite);
            }
        }
    }

    /// `point_profile`: the feature was stored as a profile of corner points, which
    /// reads as a closed chain of lines.
    fn feature_kind(&mut self, kind: &FeatureKind, point_profile: bool) {
        match kind {
            FeatureKind::Workplane(spec) => {
                self.byte(17);
                match &spec.support {
                    WorkplaneSupport::Free => self.byte(4),
                    WorkplaneSupport::Principal(plane) => {
                        self.byte(1);
                        self.byte(match plane {
                            PrincipalPlane::Xy => 1,
                            PrincipalPlane::Yz => 2,
                            PrincipalPlane::Xz => 3,
                        });
                    }
                    WorkplaneSupport::Offset { base, distance } => {
                        self.byte(2);
                        self.u64(base.0);
                        self.bytes(distance.source_token().as_bytes());
                        self.u64(distance.millimetres().to_bits());
                    }
                    WorkplaneSupport::PlanarFace { reference, .. } => {
                        self.byte(3);
                        self.body_subshape_reference(reference);
                    }
                    WorkplaneSupport::ConstructionPlane { feature } => {
                        self.byte(5);
                        self.u64(feature.0);
                    }
                }
                if !matches!(&spec.support, WorkplaneSupport::PlanarFace { .. }) {
                    for coordinate in spec
                        .frame
                        .origin_mm
                        .iter()
                        .chain(spec.frame.x_axis.iter())
                        .chain(spec.frame.y_axis.iter())
                        .chain(spec.frame.normal.iter())
                    {
                        self.u64(coordinate.to_bits());
                    }
                }
            }
            FeatureKind::Sketch(spec) => {
                self.byte(18);
                self.u64(spec.workplane.0);
                self.u64(spec.entities.len() as u64);
                for entity in &spec.entities {
                    match entity {
                        SketchEntity::Line {
                            id,
                            start_mm,
                            end_mm,
                        } => {
                            self.byte(1);
                            self.u64(id.0);
                            for point in [start_mm, end_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                        }
                        SketchEntity::Arc {
                            id,
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        } => {
                            self.byte(2);
                            self.u64(id.0);
                            for point in [start_mm, end_mm, center_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                            self.byte(u8::from(*clockwise));
                        }
                        SketchEntity::Circle {
                            id,
                            center_mm,
                            radius_mm,
                        } => {
                            self.byte(3);
                            self.u64(id.0);
                            self.u64(center_mm[0].to_bits());
                            self.u64(center_mm[1].to_bits());
                            self.u64(radius_mm.to_bits());
                        }
                        SketchEntity::CubicBezier {
                            id,
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                        } => {
                            self.byte(4);
                            self.u64(id.0);
                            for point in [start_mm, control_1_mm, control_2_mm, end_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                        }
                    }
                }
                self.u64(spec.constraints.len() as u64);
                for constraint in &spec.constraints {
                    self.u64(constraint.id.0);
                    match &constraint.kind {
                        SketchConstraintKind::Horizontal { entity } => {
                            self.byte(1);
                            self.u64(entity.0);
                        }
                        SketchConstraintKind::Vertical { entity } => {
                            self.byte(2);
                            self.u64(entity.0);
                        }
                        SketchConstraintKind::Coincident { a, b } => {
                            self.byte(3);
                            self.sketch_point_ref(*a);
                            self.sketch_point_ref(*b);
                        }
                        SketchConstraintKind::Distance { a, b, value } => {
                            self.byte(4);
                            self.sketch_point_ref(*a);
                            self.sketch_point_ref(*b);
                            self.bytes(value.source_token().as_bytes());
                            self.u64(value.millimetres().to_bits());
                        }
                        SketchConstraintKind::Radius { entity, value } => {
                            self.byte(5);
                            self.u64(entity.0);
                            self.bytes(value.source_token().as_bytes());
                            self.u64(value.millimetres().to_bits());
                        }
                        SketchConstraintKind::FixedPoint { point, position_mm } => {
                            self.byte(6);
                            self.sketch_point_ref(*point);
                            self.u64(position_mm[0].to_bits());
                            self.u64(position_mm[1].to_bits());
                        }
                        SketchConstraintKind::Parallel { a, b } => {
                            self.byte(7);
                            self.u64(a.0);
                            self.u64(b.0);
                        }
                        SketchConstraintKind::Perpendicular { a, b } => {
                            self.byte(8);
                            self.u64(a.0);
                            self.u64(b.0);
                        }
                        SketchConstraintKind::Tangent { a, b } => {
                            self.byte(9);
                            self.u64(a.0);
                            self.u64(b.0);
                        }
                        SketchConstraintKind::Angle {
                            a,
                            b,
                            angle_degrees,
                        } => {
                            self.byte(10);
                            self.u64(a.0);
                            self.u64(b.0);
                            self.u64(angle_degrees.to_bits());
                        }
                        SketchConstraintKind::Equal { a, b } => {
                            self.byte(11);
                            self.u64(a.0);
                            self.u64(b.0);
                        }
                        SketchConstraintKind::Symmetric { a, b, axis } => {
                            self.byte(12);
                            self.sketch_point_ref(*a);
                            self.sketch_point_ref(*b);
                            self.u64(axis.0);
                        }
                        SketchConstraintKind::Concentric { a, b } => {
                            self.byte(13);
                            self.u64(a.0);
                            self.u64(b.0);
                        }
                        SketchConstraintKind::Collinear { a, b } => {
                            self.byte(14);
                            self.u64(a.0);
                            self.u64(b.0);
                        }
                        SketchConstraintKind::Midpoint { point, line } => {
                            self.byte(15);
                            self.sketch_point_ref(*point);
                            self.u64(line.0);
                        }
                        SketchConstraintKind::PointOnCurve { point, curve } => {
                            self.byte(16);
                            self.sketch_point_ref(*point);
                            self.u64(curve.0);
                        }
                        SketchConstraintKind::Projection {
                            entity,
                            source_feature,
                            source_entity,
                            ..
                        } => {
                            self.byte(17);
                            self.u64(entity.0);
                            self.u64(source_feature.0);
                            self.u64(source_entity.0);
                        }
                        SketchConstraintKind::Construction { entity } => {
                            self.byte(18);
                            self.u64(entity.0);
                        }
                    }
                }
            }
            FeatureKind::Profile { segments, .. } if point_profile => {
                self.byte(1);
                self.u64(segments.len() as u64);
                for segment in segments {
                    let point = segment.start_mm();
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            // Old records stored a closed spline as its own feature kind.
            FeatureKind::Profile { .. } if let Some(points) = kind.closed_spline_points() => {
                self.byte(14);
                self.u64(points.len() as u64);
                for point in points {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            FeatureKind::Profile { segments, closed } => {
                self.byte(11);
                self.byte(u8::from(*closed));
                self.u64(segments.len() as u64);
                for segment in segments {
                    match segment {
                        ProfileSegment::Line { start_mm, end_mm } => {
                            self.byte(1);
                            for point in [start_mm, end_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                        }
                        ProfileSegment::CircularArc {
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        } => {
                            self.byte(2);
                            for point in [start_mm, end_mm, center_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                            self.byte(u8::from(*clockwise));
                        }
                        ProfileSegment::CubicBezier {
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                        } => {
                            self.byte(3);
                            for point in [start_mm, control_1_mm, control_2_mm, end_mm] {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                        }
                        // Never in an old record; hashed for completeness only.
                        ProfileSegment::Spline { points_mm } => {
                            self.byte(4);
                            self.u64(points_mm.len() as u64);
                            for point in points_mm {
                                self.u64(point[0].to_bits());
                                self.u64(point[1].to_bits());
                            }
                        }
                    }
                }
            }
            FeatureKind::ConstructionPoint { position_mm } => {
                self.byte(26);
                for coordinate in position_mm {
                    self.u64(coordinate.to_bits());
                }
            }
            FeatureKind::ConstructionAxis {
                origin_mm,
                direction,
            } => {
                self.byte(27);
                for coordinate in origin_mm.iter().chain(direction) {
                    self.u64(coordinate.to_bits());
                }
            }
            FeatureKind::ConstructionPlane {
                origin_mm,
                normal,
                x_direction,
            } => {
                self.byte(28);
                for coordinate in origin_mm.iter().chain(normal).chain(x_direction) {
                    self.u64(coordinate.to_bits());
                }
            }
            FeatureKind::SpatialPath { segments } => {
                self.byte(25);
                self.u64(segments.len() as u64);
                for segment in segments {
                    match segment {
                        SpatialPathSegment::Line { start_mm, end_mm } => {
                            self.byte(1);
                            for point in [start_mm, end_mm] {
                                for coordinate in point {
                                    self.u64(coordinate.to_bits());
                                }
                            }
                        }
                        SpatialPathSegment::CircularArc {
                            start_mm,
                            end_mm,
                            center_mm,
                            normal,
                            clockwise,
                        } => {
                            self.byte(2);
                            for point in [start_mm, end_mm, center_mm, normal] {
                                for coordinate in point {
                                    self.u64(coordinate.to_bits());
                                }
                            }
                            self.byte(u8::from(*clockwise));
                        }
                        SpatialPathSegment::CubicBezier {
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                        } => {
                            self.byte(3);
                            for point in [start_mm, control_1_mm, control_2_mm, end_mm] {
                                for coordinate in point {
                                    self.u64(coordinate.to_bits());
                                }
                            }
                        }
                    }
                }
            }
            FeatureKind::Pad(spec) => self.pad(spec),
            FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => {
                self.byte(8);
                self.byte(match operation {
                    BooleanOperation::Cut => 1,
                    BooleanOperation::Union => 2,
                    BooleanOperation::Intersect => 3,
                    BooleanOperation::Split => 4,
                });
                self.u64(target.0);
                self.u64(tool.0);
            }
            FeatureKind::PlanarOffset { profile, distance } => {
                self.byte(12);
                self.u64(profile.0);
                self.bytes(distance.source_token.as_bytes());
                self.u64(distance.millimetres.to_bits());
            }
            FeatureKind::Sweep { profile, path } => {
                self.byte(13);
                self.u64(profile.0);
                self.u64(path.0);
            }
            FeatureKind::WeldmentMember(spec) => {
                self.byte(30);
                self.u64(spec.profile.0);
                self.u64(spec.path.0);
                self.u64(spec.orientation_degrees.to_bits());
            }
            FeatureKind::WeldmentJoint(spec) => {
                self.byte(31);
                self.u64(spec.first_member.0);
                self.u64(spec.second_member.0);
                self.byte(match spec.policy {
                    crate::document::WeldmentJointPolicy::Butt => 1,
                    crate::document::WeldmentJointPolicy::Miter => 2,
                });
                self.byte(match spec.primary {
                    crate::document::WeldmentJointPrimary::First => 1,
                    crate::document::WeldmentJointPrimary::Second => 2,
                });
            }
            FeatureKind::SurfaceBody(spec) => {
                self.byte(32);
                match spec {
                    crate::document::SurfaceBodySpec::Planar { profile } => {
                        self.byte(1);
                        self.u64(profile.0);
                    }
                    crate::document::SurfaceBodySpec::Loft {
                        sections,
                        guide,
                        continuity,
                    } => {
                        self.byte(2);
                        self.u64(sections.len() as u64);
                        for section in sections {
                            self.u64(section.profile.0);
                            self.u64(section.elevation_mm.to_bits());
                        }
                        self.u64(guide.map_or(0, |guide| guide.0));
                        self.byte(match continuity {
                            crate::document::LoftContinuity::Position => 1,
                            crate::document::LoftContinuity::Tangent => 2,
                            crate::document::LoftContinuity::Curvature => 3,
                        });
                    }
                }
            }
            FeatureKind::SurfaceTrim { target, cutter } => {
                self.byte(33);
                self.u64(target.0);
                self.u64(cutter.0);
            }
            FeatureKind::SurfaceExtend { target, distance } => {
                self.byte(34);
                self.u64(target.0);
                self.bytes(distance.source_token.as_bytes());
                self.u64(distance.millimetres.to_bits());
            }
            FeatureKind::SurfaceKnit {
                surfaces,
                tolerance,
                make_solid,
            } => {
                self.byte(35);
                self.u64(surfaces.len() as u64);
                for surface in surfaces {
                    self.u64(surface.0);
                }
                self.bytes(tolerance.source_token.as_bytes());
                self.u64(tolerance.millimetres.to_bits());
                self.byte(u8::from(*make_solid));
            }
            FeatureKind::SurfaceThicken {
                target,
                thickness,
                direction,
            } => {
                self.byte(36);
                self.u64(target.0);
                self.bytes(thickness.source_token.as_bytes());
                self.u64(thickness.millimetres.to_bits());
                self.byte(match direction {
                    ShellDirection::Inward => 1,
                    ShellDirection::Outward => 2,
                    ShellDirection::Symmetric => 3,
                });
            }
            FeatureKind::Loft {
                sections,
                guide,
                continuity,
            } => {
                self.byte(15);
                self.u64(sections.len() as u64);
                for section in sections {
                    self.u64(section.profile.0);
                    self.u64(section.elevation_mm.to_bits());
                }
                self.u64(guide.map_or(0, |guide| guide.0));
                self.byte(match continuity {
                    crate::document::LoftContinuity::Position => 1,
                    crate::document::LoftContinuity::Tangent => 2,
                    crate::document::LoftContinuity::Curvature => 3,
                });
            }
            FeatureKind::Revolve {
                profile,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => {
                self.byte(4);
                self.u64(profile.0);
                for coordinate in axis_start_mm.iter().chain(axis_end_mm) {
                    self.u64(coordinate.to_bits());
                }
                self.u64(angle_degrees.to_bits());
            }
            FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
                direction,
            } => {
                // Version 3 kept recorded and named faces in two lists.
                let recorded = removed_faces
                    .iter()
                    .filter_map(FaceRef::topological)
                    .collect::<Vec<_>>();
                let profile_faces = removed_faces
                    .iter()
                    .filter_map(FaceRef::named)
                    .collect::<Vec<_>>();
                self.byte(21);
                self.u64(target.0);
                self.u64(recorded.len() as u64);
                for reference in recorded {
                    self.topological_reference(reference);
                }
                self.bytes(thickness.source_token.as_bytes());
                self.u64(thickness.millimetres.to_bits());
                self.byte(match direction {
                    ShellDirection::Inward => 1,
                    ShellDirection::Outward => 2,
                    ShellDirection::Symmetric => 3,
                });
                // Absent for shells opened by topology, so their digests stay.
                if !profile_faces.is_empty() {
                    self.u64(profile_faces.len() as u64);
                    for face in profile_faces {
                        self.profile_face(face);
                    }
                }
            }
            FeatureKind::EdgeFinish {
                target,
                edges,
                kind,
                amount,
                fillet_radius_stations,
                chamfer_mode,
                chamfer_edge_sides,
            } => {
                let recorded = edges
                    .iter()
                    .filter_map(EdgeRef::topological)
                    .collect::<Vec<_>>();
                let profile_edges = edges.iter().filter_map(EdgeRef::named).collect::<Vec<_>>();
                self.byte(22);
                self.u64(target.0);
                self.u64(recorded.len() as u64);
                for reference in recorded {
                    self.topological_reference(reference);
                }
                self.u64(profile_edges.len() as u64);
                for edge in profile_edges {
                    self.profile_face(&edge.first);
                    self.profile_face(&edge.second);
                }
                self.byte(match kind {
                    EdgeFinishKind::Fillet => 1,
                    EdgeFinishKind::Chamfer => 2,
                });
                self.bytes(amount.source_token.as_bytes());
                self.u64(amount.millimetres.to_bits());
                self.u64(fillet_radius_stations.len() as u64);
                for station in fillet_radius_stations {
                    self.u64(station.position.to_bits());
                    self.bytes(station.radius.source_token.as_bytes());
                    self.u64(station.radius.millimetres.to_bits());
                }
                match chamfer_mode {
                    ChamferMode::Symmetric => self.byte(1),
                    ChamferMode::TwoDistance { second_distance } => {
                        self.byte(2);
                        self.bytes(second_distance.source_token.as_bytes());
                        self.u64(second_distance.millimetres.to_bits());
                    }
                    ChamferMode::DistanceAngle { angle_degrees } => {
                        self.byte(3);
                        self.u64(angle_degrees.to_bits());
                    }
                }
                self.u64(chamfer_edge_sides.len() as u64);
                for selection in chamfer_edge_sides {
                    self.topological_reference(&selection.edge);
                    self.topological_reference(&selection.side_face);
                }
            }
            FeatureKind::FaceOffset {
                target,
                face,
                distance,
            } => {
                self.byte(23);
                self.u64(target.0);
                match face {
                    FaceRef::Topological(face) => {
                        self.byte(1);
                        self.topological_reference(face);
                    }
                    FaceRef::Named(face) => {
                        self.byte(2);
                        self.profile_face(face);
                    }
                }
                self.bytes(distance.source_token.as_bytes());
                self.u64(distance.millimetres.to_bits());
            }
            FeatureKind::RigidTransform { target, transform } => {
                self.byte(24);
                self.u64(target.0);
                for value in transform.matrix() {
                    self.u64(value.to_bits());
                }
            }
            FeatureKind::SheetMetal(spec) => {
                self.byte(29);
                for dimension in [&spec.width, &spec.depth, &spec.thickness] {
                    self.bytes(dimension.source_token().as_bytes());
                    self.u64(dimension.millimetres().to_bits());
                }
                self.u64(spec.k_factor.to_bits());
                self.u64(spec.flanges.len() as u64);
                for flange in &spec.flanges {
                    self.byte(match flange.edge {
                        crate::sheet_metal::SheetMetalEdge::MinX => 1,
                        crate::sheet_metal::SheetMetalEdge::MaxX => 2,
                        crate::sheet_metal::SheetMetalEdge::MinY => 3,
                        crate::sheet_metal::SheetMetalEdge::MaxY => 4,
                    });
                    self.bytes(flange.length.source_token().as_bytes());
                    self.u64(flange.length.millimetres().to_bits());
                    self.u64(flange.angle_degrees.to_bits());
                    self.bytes(flange.inner_radius.source_token().as_bytes());
                    self.u64(flange.inner_radius.millimetres().to_bits());
                }
            }
            FeatureKind::ImportedExactBody(spec) => {
                self.byte(16);
                self.bytes(spec.schema.as_bytes());
                self.u64(spec.import_id.0);
                self.bytes(&spec.source_sha256);
                self.u64(spec.source_byte_len);
                match spec.source_part_index {
                    Some(index) => {
                        self.byte(1);
                        self.u64(u64::from(index));
                    }
                    None => self.byte(0),
                }
                self.bytes(spec.result_fingerprint.as_bytes());
                self.u64(u64::from(spec.solid_count));
                if let Some(topology_counts) = spec.topology_counts {
                    self.byte(1);
                    for count in topology_counts {
                        self.u64(u64::from(count));
                    }
                }
                if spec.schema == IMPORTED_EXACT_BODY_SCHEMA_V3 {
                    self.byte(match spec.body_kind {
                        BodyKind::Solid => 1,
                        BodyKind::Surface => 2,
                    });
                    self.u64(spec.area_mm2.to_bits());
                }
                self.u64(spec.volume_mm3.to_bits());
                for coordinate in spec.bounds_mm.iter().flatten() {
                    self.u64(coordinate.to_bits());
                }
                self.bytes(spec.backend.as_bytes());
                self.bytes(spec.tolerance.as_bytes());
            }
            FeatureKind::MeshBody(spec) => {
                self.byte(9);
                self.bytes(spec.schema.as_bytes());
                self.u64(spec.vertices_mm.len() as u64);
                for vertex in &spec.vertices_mm {
                    for coordinate in vertex {
                        self.u64(coordinate.to_bits());
                    }
                }
                self.u64(spec.triangles.len() as u64);
                for triangle in &spec.triangles {
                    for index in triangle {
                        self.u64(u64::from(*index));
                    }
                }
                match &spec.authority {
                    MeshAuthority::Authored { provenance } => {
                        self.byte(1);
                        self.bytes(provenance.as_bytes());
                    }
                    MeshAuthority::ImportedStl { import_id } => {
                        self.byte(3);
                        self.u64(import_id.0);
                    }
                    MeshAuthority::ImportedSketchupScene { import_id } => {
                        self.byte(4);
                        self.u64(import_id.0);
                    }
                    MeshAuthority::ImportedGlb { import_id } => {
                        self.byte(5);
                        self.u64(import_id.0);
                    }
                    MeshAuthority::ExactConversion(conversion) => {
                        self.byte(2);
                        self.u64(conversion.source_document_id.0);
                        self.u64(conversion.source_revision);
                        self.bytes(conversion.source_digest.as_bytes());
                        self.u64(conversion.source_definition_id.0);
                        self.u64(conversion.source_feature_id.0);
                        self.bytes(conversion.source_result_fingerprint.as_bytes());
                        self.bytes(conversion.source_evaluator.as_bytes());
                        self.bytes(conversion.source_backend.as_bytes());
                        self.bytes(conversion.source_tolerance.as_bytes());
                        self.bytes(conversion.tessellation_tolerance.as_bytes());
                        self.u64(conversion.destination_definition_id.0);
                        self.u64(conversion.destination_feature_id.0);
                        self.u64(conversion.unsupported_semantics.len() as u64);
                        for semantic in &conversion.unsupported_semantics {
                            self.bytes(semantic.as_bytes());
                        }
                        self.byte(match conversion.exact_reference_consequence {
                            ExactReferenceConversionConsequence::Lost => 1,
                        });
                    }
                }
            }
        }
    }

    fn feature(&mut self, feature: &Feature, point_profiles: &BTreeSet<FeatureId>) {
        self.u64(feature.id.0);
        self.u64(feature.definition_id.0);
        self.bytes(feature.name.as_bytes());
        self.feature_kind(&feature.kind, point_profiles.contains(&feature.id));
    }

    fn assembly_mate(&mut self, mate: &AssemblyMate) {
        self.u64(mate.id().0);
        for endpoint in [mate.endpoint_a(), mate.endpoint_b()] {
            self.instance_path(endpoint.instance_path());
            self.body_subshape_reference(endpoint.reference());
            match endpoint.attachment() {
                crate::assembly::AssemblyMateAttachment::ReferenceOnly(_) => self.byte(1),
                crate::assembly::AssemblyMateAttachment::PlanarFace(attachment) => {
                    self.byte(2);
                    for value in attachment
                        .local_origin_mm()
                        .into_iter()
                        .chain(attachment.local_unit_normal())
                    {
                        self.u64(value.to_bits());
                    }
                }
                crate::assembly::AssemblyMateAttachment::Axial(attachment) => {
                    self.byte(3);
                    self.byte(match attachment.kind() {
                        crate::assembly::AxialAttachmentKind::Axis => 1,
                        crate::assembly::AxialAttachmentKind::CylindricalFace => 2,
                    });
                    for value in attachment
                        .local_origin_mm()
                        .into_iter()
                        .chain(attachment.local_unit_direction())
                    {
                        self.u64(value.to_bits());
                    }
                }
            }
            match endpoint.health() {
                AssemblyReferenceHealth::Resolved => self.byte(1),
                AssemblyReferenceHealth::Ambiguous { candidate_count } => {
                    self.byte(2);
                    self.u64(u64::from(candidate_count));
                }
                AssemblyReferenceHealth::Lost => self.byte(3),
                AssemblyReferenceHealth::Broken => self.byte(4),
            }
        }
        match mate.kind() {
            AssemblyMateKind::CoincidentPlanar {
                offset_mm,
                reversed,
            } => {
                self.byte(1);
                self.u64(offset_mm.to_bits());
                self.byte(u8::from(reversed));
            }
            AssemblyMateKind::ConcentricAxial { reversed } => {
                self.byte(2);
                self.byte(u8::from(reversed));
            }
            AssemblyMateKind::Distance { distance_mm } => {
                self.byte(3);
                self.u64(distance_mm.to_bits());
            }
            AssemblyMateKind::Angle { angle_degrees } => {
                self.byte(4);
                self.u64(angle_degrees.to_bits());
            }
        }
    }

    fn assembly_joint(&mut self, joint: &AssemblyJoint) {
        self.bytes(joint.schema().as_bytes());
        self.u64(joint.id().0);
        self.instance_path(joint.parent_instance_path());
        self.instance_path(joint.child_instance_path());
        self.assembly_joint_kind(joint.kind());
    }

    fn assembly_joint_kind(&mut self, kind: AssemblyJointKind) {
        match kind {
            AssemblyJointKind::Fixed => self.byte(1),
            AssemblyJointKind::Revolute {
                axis,
                limits,
                position_degrees,
            } => {
                self.byte(2);
                self.assembly_joint_axis(axis);
                self.assembly_joint_limits(limits);
                self.u64(position_degrees.to_bits());
            }
            AssemblyJointKind::Prismatic {
                axis,
                limits,
                position_mm,
            } => {
                self.byte(3);
                self.assembly_joint_axis(axis);
                self.assembly_joint_limits(limits);
                self.u64(position_mm.to_bits());
            }
            AssemblyJointKind::Helical {
                axis,
                limits,
                lead_mm_per_revolution,
                position_degrees,
            } => {
                self.byte(4);
                self.assembly_joint_axis(axis);
                self.assembly_joint_limits(limits);
                self.u64(lead_mm_per_revolution.to_bits());
                self.u64(position_degrees.to_bits());
            }
        }
    }

    fn assembly_joint_axis(&mut self, axis: crate::assembly_joint::AssemblyJointAxis) {
        for value in axis.direction_in_parent() {
            self.u64(value.to_bits());
        }
        for value in axis.pivot_in_parent_mm() {
            self.u64(value.to_bits());
        }
    }

    fn assembly_joint_limits(&mut self, limits: Option<AssemblyJointLimits>) {
        if let Some(limits) = limits {
            self.byte(1);
            self.u64(limits.min().to_bits());
            self.u64(limits.max().to_bits());
        } else {
            self.byte(0);
        }
    }

    fn mechanical_interface(&mut self, interface: &MechanicalInterface) {
        use crate::mechanical_contract::MechanicalRole;

        self.bytes(interface.schema().as_bytes());
        self.u64(interface.id().0);
        self.u64(interface.occurrence_id().0);
        self.byte(match interface.role() {
            MechanicalRole::Mounting => 1,
            MechanicalRole::Support => 2,
            MechanicalRole::Guide => 3,
        });
        self.u64(u64::from(interface.face_ordinal()));
        self.bytes(interface.geometry_fingerprint().as_bytes());
        let frame = interface.frame();
        for value in frame.origin_mm() {
            self.u64(value.to_bits());
        }
        for value in frame.normal() {
            self.u64(value.to_bits());
        }
        self.u64(frame.area_mm2().to_bits());
        for corner in frame.bounds_mm() {
            for value in corner {
                self.u64(value.to_bits());
            }
        }
    }

    fn mechanical_condition(&mut self, condition: &MechanicalCondition) {
        use crate::mechanical_contract::MechanicalAxisAlignment;

        self.bytes(condition.schema().as_bytes());
        self.u64(condition.id().0);
        match condition.kind() {
            MechanicalConditionKind::PlanarContact {
                first,
                second,
                offset_mm,
                tolerance_mm,
            } => {
                self.byte(1);
                self.u64(first.0);
                self.u64(second.0);
                self.u64(offset_mm.to_bits());
                self.u64(tolerance_mm.to_bits());
            }
            MechanicalConditionKind::Support {
                supported,
                supporting,
                tolerance_mm,
            } => {
                self.byte(2);
                self.u64(supported.0);
                self.u64(supporting.0);
                self.u64(tolerance_mm.to_bits());
            }
            MechanicalConditionKind::JointAxisAlignment {
                joint_id,
                interface,
                alignment,
                tolerance_degrees,
            } => {
                self.byte(3);
                self.u64(joint_id.0);
                self.u64(interface.0);
                self.byte(match alignment {
                    MechanicalAxisAlignment::Parallel => 1,
                    MechanicalAxisAlignment::Perpendicular => 2,
                });
                self.u64(tolerance_degrees.to_bits());
            }
            MechanicalConditionKind::JointTravel {
                joint_id,
                minimum,
                maximum,
            } => {
                self.byte(4);
                self.u64(joint_id.0);
                self.u64(minimum.to_bits());
                self.u64(maximum.to_bits());
            }
        }
    }

    fn assembly_motion_coupling(&mut self, coupling: &AssemblyMotionCoupling) {
        use crate::mechanical_coupling::{
            AssemblyMotionDirection, AssemblyTransmissionKind, GearMeshKind, ScrewHandedness,
        };

        self.bytes(coupling.schema().as_bytes());
        self.u64(coupling.id().0);
        self.u64(coupling.input_joint_id().0);
        self.u64(coupling.output_joint_id().0);
        self.u64(coupling.input_reference_position().to_bits());
        self.u64(coupling.output_reference_position().to_bits());
        match coupling.transmission() {
            AssemblyTransmissionKind::GearPair {
                input_teeth,
                output_teeth,
                mesh,
            } => {
                self.byte(1);
                self.u64(u64::from(input_teeth));
                self.u64(u64::from(output_teeth));
                self.byte(match mesh {
                    GearMeshKind::External => 1,
                    GearMeshKind::Internal => 2,
                });
            }
            AssemblyTransmissionKind::Belt {
                input_pitch_diameter_mm,
                output_pitch_diameter_mm,
                crossed,
            } => {
                self.byte(2);
                self.u64(input_pitch_diameter_mm.to_bits());
                self.u64(output_pitch_diameter_mm.to_bits());
                self.byte(u8::from(crossed));
            }
            AssemblyTransmissionKind::Chain {
                input_sprocket_teeth,
                output_sprocket_teeth,
            } => {
                self.byte(3);
                self.u64(u64::from(input_sprocket_teeth));
                self.u64(u64::from(output_sprocket_teeth));
            }
            AssemblyTransmissionKind::RackAndPinion {
                pinion_pitch_diameter_mm,
                direction,
            } => {
                self.byte(4);
                self.u64(pinion_pitch_diameter_mm.to_bits());
                self.byte(match direction {
                    AssemblyMotionDirection::Same => 1,
                    AssemblyMotionDirection::Opposite => 2,
                });
            }
            AssemblyTransmissionKind::LeadScrew {
                lead_mm_per_revolution,
                handedness,
            } => {
                self.byte(5);
                self.u64(lead_mm_per_revolution.to_bits());
                self.byte(match handedness {
                    ScrewHandedness::Right => 1,
                    ScrewHandedness::Left => 2,
                });
            }
        }
    }

    fn assembly_motion_study(&mut self, study: &AssemblyMotionStudy) {
        self.bytes(study.schema().as_bytes());
        self.u64(study.id().0);
        self.bytes(study.name().as_bytes());
        self.u64(study.drivers().len() as u64);
        for driver in study.drivers() {
            self.u64(driver.joint_id().0);
            self.u64(driver.position().to_bits());
        }
    }

    fn drawing_sheet(&mut self, sheet: &DrawingSheet) {
        self.bytes(sheet.schema().as_bytes());
        self.u64(sheet.id().0);
        self.bytes(sheet.name().as_bytes());
        match sheet.source() {
            DrawingSource::Definition(id) => {
                self.byte(1);
                self.u64(id.0);
            }
            DrawingSource::RigidAssembly { occurrence_ids } => {
                self.byte(2);
                self.u64(occurrence_ids.len() as u64);
                for id in occurrence_ids {
                    self.u64(id.0);
                }
            }
            DrawingSource::RigidAssemblyInstances { instance_paths } => {
                self.byte(3);
                self.u64(instance_paths.len() as u64);
                for path in instance_paths {
                    self.instance_path(path);
                }
            }
        }
        let page = sheet.page();
        self.bytes(page.size().stable_name().as_bytes());
        self.bytes(page.orientation().stable_name().as_bytes());
        self.u64(u64::from(page.scale().numerator()));
        self.u64(u64::from(page.scale().denominator()));
        for margin in page.margins().values_mm() {
            self.u64(u64::from(margin));
        }
        let title_block = sheet.title_block();
        self.bytes(title_block.title().as_bytes());
        self.bytes(title_block.drawing_number().as_bytes());
        self.bytes(title_block.revision().as_bytes());
        self.bytes(title_block.author().as_bytes());
        self.byte(u8::from(title_block.is_parametric()));
        self.u64(sheet.views().len() as u64);
        for view in sheet.views() {
            self.bytes(view.stable_name().as_bytes());
        }
        self.u64(sheet.linear_dimensions().len() as u64);
        for dimension in sheet.linear_dimensions() {
            self.u64(dimension.id().0);
            self.bytes(dimension.view_stable_name().as_bytes());
            self.bytes(dimension.source_line_id().as_bytes());
            self.u64(dimension.offset_page_mm().to_bits());
            match dimension.tolerance() {
                DrawingDimensionTolerance::None => self.byte(0),
                DrawingDimensionTolerance::Symmetric { deviation_bits } => {
                    self.byte(1);
                    self.u64(deviation_bits);
                }
                DrawingDimensionTolerance::Bilateral {
                    upper_bits,
                    lower_bits,
                } => {
                    self.byte(2);
                    self.u64(upper_bits);
                    self.u64(lower_bits);
                }
            }
        }
        self.u64(sheet.angular_dimensions().len() as u64);
        for dimension in sheet.angular_dimensions() {
            self.u64(dimension.id().0);
            self.bytes(dimension.view_stable_name().as_bytes());
            for source in dimension.source_line_ids() {
                self.bytes(source.as_bytes());
            }
            self.u64(dimension.arc_radius_page_mm().to_bits());
            match dimension.tolerance() {
                DrawingDimensionTolerance::None => self.byte(0),
                DrawingDimensionTolerance::Symmetric { deviation_bits } => {
                    self.byte(1);
                    self.u64(deviation_bits);
                }
                DrawingDimensionTolerance::Bilateral {
                    upper_bits,
                    lower_bits,
                } => {
                    self.byte(2);
                    self.u64(upper_bits);
                    self.u64(lower_bits);
                }
            }
        }
        self.u64(sheet.circular_dimensions().len() as u64);
        for dimension in sheet.circular_dimensions() {
            self.u64(dimension.id().0);
            self.bytes(dimension.view_stable_name().as_bytes());
            self.bytes(dimension.source_circle_id().as_bytes());
            self.byte(match dimension.kind() {
                DrawingCircularDimensionKind::Radius => 1,
                DrawingCircularDimensionKind::Diameter => 2,
            });
            self.u64(dimension.leader_angle_degrees().to_bits());
            self.u64(dimension.offset_page_mm().to_bits());
            match dimension.tolerance() {
                DrawingDimensionTolerance::None => self.byte(0),
                DrawingDimensionTolerance::Symmetric { deviation_bits } => {
                    self.byte(1);
                    self.u64(deviation_bits);
                }
                DrawingDimensionTolerance::Bilateral {
                    upper_bits,
                    lower_bits,
                } => {
                    self.byte(2);
                    self.u64(upper_bits);
                    self.u64(lower_bits);
                }
            }
        }
        self.u64(sheet.datum_symbols().len() as u64);
        for datum in sheet.datum_symbols() {
            self.u64(datum.id().0);
            self.bytes(datum.view_stable_name().as_bytes());
            self.bytes(datum.source_line_id().as_bytes());
            self.bytes(datum.label().as_bytes());
            for coordinate in datum.offset_page_mm() {
                self.u64(coordinate.to_bits());
            }
        }
        self.u64(sheet.feature_control_frames().len() as u64);
        for frame in sheet.feature_control_frames() {
            self.u64(frame.id().0);
            self.bytes(frame.view_stable_name().as_bytes());
            self.bytes(frame.source_line_id().as_bytes());
            self.byte(match frame.characteristic() {
                DrawingGeometricCharacteristic::Straightness => 1,
                DrawingGeometricCharacteristic::Flatness => 2,
                DrawingGeometricCharacteristic::Circularity => 3,
                DrawingGeometricCharacteristic::Cylindricity => 4,
                DrawingGeometricCharacteristic::ProfileOfLine => 5,
                DrawingGeometricCharacteristic::ProfileOfSurface => 6,
                DrawingGeometricCharacteristic::Angularity => 7,
                DrawingGeometricCharacteristic::Perpendicularity => 8,
                DrawingGeometricCharacteristic::Parallelism => 9,
                DrawingGeometricCharacteristic::Position => 10,
                DrawingGeometricCharacteristic::Concentricity => 11,
                DrawingGeometricCharacteristic::Symmetry => 12,
                DrawingGeometricCharacteristic::CircularRunout => 13,
                DrawingGeometricCharacteristic::TotalRunout => 14,
            });
            self.u64(frame.tolerance_mm().to_bits());
            self.byte(u8::from(frame.diameter_zone()));
            self.byte(match frame.material_condition() {
                DrawingMaterialCondition::None => 0,
                DrawingMaterialCondition::MaximumMaterial => 1,
                DrawingMaterialCondition::LeastMaterial => 2,
                DrawingMaterialCondition::RegardlessOfFeatureSize => 3,
            });
            self.u64(frame.datum_references().len() as u64);
            for reference in frame.datum_references() {
                self.bytes(reference.label().as_bytes());
                self.byte(match reference.material_condition() {
                    DrawingMaterialCondition::None => 0,
                    DrawingMaterialCondition::MaximumMaterial => 1,
                    DrawingMaterialCondition::LeastMaterial => 2,
                    DrawingMaterialCondition::RegardlessOfFeatureSize => 3,
                });
            }
            for coordinate in frame.offset_page_mm() {
                self.u64(coordinate.to_bits());
            }
        }
        self.u64(sheet.bom_balloons().len() as u64);
        for balloon in sheet.bom_balloons() {
            self.u64(balloon.id().0);
            self.bytes(balloon.view_stable_name().as_bytes());
            self.instance_path(balloon.instance_path());
            self.u64(u64::from(balloon.position()));
            for coordinate in balloon.offset_page_mm() {
                self.u64(coordinate.to_bits());
            }
        }
        self.u64(sheet.notes().len() as u64);
        for note in sheet.notes() {
            self.u64(note.id().0);
            for coordinate in note.position_page_mm() {
                self.u64(coordinate.to_bits());
            }
            self.bytes(note.text_template().as_bytes());
        }
    }

    fn occurrence(&mut self, occurrence: &Occurrence) {
        self.u64(occurrence.id.0);
        self.u64(occurrence.definition_id.0);
        self.bytes(occurrence.name.as_bytes());
        self.transform(occurrence.transform);
        self.optional_id(occurrence.parent.map(|id| id.0));
        self.optional_id(occurrence.tag.map(|id| id.0));
        self.byte(u8::from(occurrence.visible));
        self.byte(u8::from(occurrence.color.is_some()));
        if let Some(color) = occurrence.color {
            for channel in color {
                self.byte(channel);
            }
        }
    }

    fn group(&mut self, group: &Group) {
        self.u64(group.id.0);
        self.bytes(group.name.as_bytes());
        self.transform(group.transform);
        self.optional_id(group.parent.map(|id| id.0));
    }

    fn local_group(&mut self, group: &LocalGroup) {
        self.u64(group.key.definition_id.0);
        self.u64(group.key.local_id.0);
        self.bytes(group.name.as_bytes());
        self.transform(group.transform);
        self.optional_id(group.parent.map(|id| id.0));
    }

    fn local_occurrence(&mut self, occurrence: &LocalOccurrence) {
        self.u64(occurrence.key.definition_id.0);
        self.u64(occurrence.key.local_id.0);
        self.u64(occurrence.definition_id.0);
        self.bytes(occurrence.name.as_bytes());
        self.transform(occurrence.transform);
        self.optional_id(occurrence.parent.map(|id| id.0));
        self.optional_id(occurrence.tag.map(|id| id.0));
        self.byte(u8::from(occurrence.visible));
        self.byte(u8::from(occurrence.color.is_some()));
        if let Some(color) = occurrence.color {
            for channel in color {
                self.byte(channel);
            }
        }
    }

    fn optional_id(&mut self, id: Option<u64>) {
        match id {
            Some(id) => {
                self.byte(1);
                self.u64(id);
            }
            None => self.byte(0),
        }
    }

    fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}
