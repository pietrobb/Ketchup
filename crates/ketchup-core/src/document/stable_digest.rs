use super::*;

pub(super) fn digest_snapshot(snapshot: &Snapshot) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.document.v3");
    digest.u64(snapshot.product.document_id.0);
    digest.byte(match snapshot.product.units {
        UnitSystem::Millimetres => 1,
    });
    digest.u64(snapshot.product.evaluator_nodes.len() as u64);
    for node in snapshot.product.evaluator_nodes.values() {
        digest.node(node);
    }
    digest.u64(snapshot.product.overrides.len() as u64);
    for value in snapshot.product.overrides.values() {
        digest.canonical_override(value);
    }
    digest.u64(snapshot.product.feature_parameter_bindings.len() as u64);
    for binding in snapshot.product.feature_parameter_bindings.values() {
        digest.feature_parameter_binding(binding);
    }
    if !snapshot.product.feature_parameter_provenance.is_empty() {
        digest.bytes(b"feature-parameter-provenance.v1");
        digest.u64(snapshot.product.feature_parameter_provenance.len() as u64);
        for (target, provenance) in &snapshot.product.feature_parameter_provenance {
            digest.feature_parameter_target(target);
            digest.feature_parameter_provenance(provenance);
        }
    }
    digest.u64(snapshot.product.joints.len() as u64);
    for joint in snapshot.product.joints.values() {
        digest.joint(joint);
    }
    digest.u64(snapshot.product.spaces.len() as u64);
    for space in snapshot.product.spaces.values() {
        digest.space(space);
    }
    digest.u64(snapshot.product.clearance_volumes.len() as u64);
    for clearance in snapshot.product.clearance_volumes.values() {
        digest.clearance_volume(clearance);
    }
    if !snapshot.product.cam_plans.is_empty() {
        digest.bytes(b"canonical-cam-plans.v1");
        digest.u64(snapshot.product.cam_plans.len() as u64);
        for plan in snapshot.product.cam_plans.values() {
            digest.cam_plan(plan);
        }
    }
    if !snapshot.product.dowel_joints.is_empty() {
        digest.bytes(b"canonical-dowel-joints.v1");
        digest.u64(snapshot.product.dowel_joints.len() as u64);
        for joint in snapshot.product.dowel_joints.values() {
            digest.dowel_joint(joint);
        }
    }
    if let Some(recipe) = snapshot.product.assembly_recipe.as_deref() {
        digest.bytes(b"canonical-assembly-recipe.v1");
        digest.assembly_recipe(recipe);
    }
    digest.u64(snapshot.product.persistent_dimensions.len() as u64);
    for dimension in snapshot.product.persistent_dimensions.values() {
        digest.persistent_dimension(dimension);
    }
    digest.u64(snapshot.product.tags.len() as u64);
    for tag in snapshot.product.tags.values() {
        digest.tag(tag);
    }
    digest.u64(snapshot.product.classification_dimensions.len() as u64);
    for dimension in snapshot.product.classification_dimensions.values() {
        digest.u64(dimension.id.0);
        digest.bytes(dimension.name.as_bytes());
        digest.u64(dimension.categories.len() as u64);
        for category in dimension.categories.values() {
            digest.u64(category.id.0);
            digest.bytes(category.name.as_bytes());
        }
    }
    digest.u64(snapshot.product.classification_assignments.len() as u64);
    for ((occurrence_id, dimension_id), category_id) in &snapshot.product.classification_assignments
    {
        digest.u64(occurrence_id.0);
        digest.u64(dimension_id.0);
        digest.u64(category_id.0);
    }
    digest.u64(snapshot.product.collections.len() as u64);
    for collection in snapshot.product.collections.values() {
        digest.collection(collection);
    }
    digest.u64(snapshot.product.import_receipts.len() as u64);
    for receipt in snapshot.product.import_receipts.values() {
        digest.import_receipt(receipt);
    }
    digest.u64(snapshot.product.definitions.len() as u64);
    for definition in snapshot.product.definitions.values() {
        digest.definition(definition);
    }
    digest.u64(snapshot.product.features.len() as u64);
    for feature in snapshot.product.features.values() {
        digest.feature(feature);
    }
    digest.u64(snapshot.product.body_feature_suppression.len() as u64);
    for ((definition_id, body_id), suppressed) in &snapshot.product.body_feature_suppression {
        digest.u64(definition_id.0);
        digest.u64(body_id.0);
        digest.u64(suppressed.len() as u64);
        for feature_id in suppressed {
            digest.u64(feature_id.0);
        }
    }
    digest.u64(snapshot.product.occurrences.len() as u64);
    for occurrence in snapshot.product.occurrences.values() {
        digest.occurrence(occurrence);
    }
    digest.u64(snapshot.product.grounded_occurrences.len() as u64);
    for occurrence_id in &snapshot.product.grounded_occurrences {
        digest.u64(occurrence_id.0);
    }
    digest.u64(snapshot.product.assembly_mates.len() as u64);
    for mate in snapshot.product.assembly_mates.values() {
        digest.assembly_mate(mate);
    }
    if !snapshot.product.assembly_joints.is_empty() {
        digest.bytes(b"canonical-assembly-joints.v1");
        digest.u64(snapshot.product.assembly_joints.len() as u64);
        for joint in snapshot.product.assembly_joints.values() {
            digest.assembly_joint(joint);
        }
    }
    if !snapshot.product.assembly_motion_couplings.is_empty() {
        digest.bytes(b"canonical-assembly-motion-couplings.v1");
        digest.u64(snapshot.product.assembly_motion_couplings.len() as u64);
        for coupling in snapshot.product.assembly_motion_couplings.values() {
            digest.assembly_motion_coupling(coupling);
        }
    }
    if !snapshot.product.mechanical_interfaces.is_empty() {
        digest.bytes(b"canonical-mechanical-interfaces.v1");
        digest.u64(snapshot.product.mechanical_interfaces.len() as u64);
        for interface in snapshot.product.mechanical_interfaces.values() {
            digest.mechanical_interface(interface);
        }
    }
    if !snapshot.product.mechanical_conditions.is_empty() {
        digest.bytes(b"canonical-mechanical-conditions.v1");
        digest.u64(snapshot.product.mechanical_conditions.len() as u64);
        for condition in snapshot.product.mechanical_conditions.values() {
            digest.mechanical_condition(condition);
        }
    }
    if !snapshot.product.assembly_motion_studies.is_empty() {
        digest.bytes(b"canonical-assembly-motion-studies.v1");
        digest.u64(snapshot.product.assembly_motion_studies.len() as u64);
        for study in snapshot.product.assembly_motion_studies.values() {
            digest.assembly_motion_study(study);
        }
    }
    if !snapshot.product.drawing_sheets.is_empty() {
        digest.bytes(b"canonical-drawing-sheets.v1");
        digest.u64(snapshot.product.drawing_sheets.len() as u64);
        for sheet in snapshot.product.drawing_sheets.values() {
            digest.drawing_sheet(sheet);
        }
    }
    digest.u64(snapshot.product.groups.len() as u64);
    for group in snapshot.product.groups.values() {
        digest.group(group);
    }
    digest.u64(snapshot.product.local_groups.len() as u64);
    for group in snapshot.product.local_groups.values() {
        digest.local_group(group);
    }
    digest.u64(snapshot.product.local_occurrences.len() as u64);
    for occurrence in snapshot.product.local_occurrences.values() {
        digest.local_occurrence(occurrence);
    }
    if !snapshot.product.instance_transform_overrides.is_empty() {
        digest.bytes(b"canonical-instance-transform-overrides.v1");
        digest.u64(snapshot.product.instance_transform_overrides.len() as u64);
        for (path, transform) in &snapshot.product.instance_transform_overrides {
            digest.instance_path(path);
            digest.transform(*transform);
        }
    }
    if !snapshot.product.production_codes.is_empty() {
        digest.bytes(b"canonical-production-codes.v1");
        digest.u64(snapshot.product.production_codes.len() as u64);
        for (path, code) in &snapshot.product.production_codes {
            digest.instance_path(path);
            digest.bytes(code.as_bytes());
        }
    }
    digest.finish()
}

pub(super) fn digest_feature(feature: &Feature) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.feature.v1");
    digest.feature(feature);
    digest.finish()
}

pub(super) struct StableDigest(u64);

impl StableDigest {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub(super) const fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(Self::PRIME);
    }

    pub(super) fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        for byte in bytes {
            self.byte(*byte);
        }
    }

    pub(super) fn u64(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.byte(byte);
        }
    }

    fn node(&mut self, node: &EvaluatorNode) {
        self.bytes(&node.canonical_spec_bytes());
    }

    fn ports(&mut self, ports: &[PortSpec]) {
        self.u64(ports.len() as u64);
        for port in ports {
            self.bytes(port.name().as_bytes());
            self.byte(match port.value_type() {
                ValueType::Number => 1,
            });
        }
    }
    fn rule_outputs(&mut self, outputs: &[RuleOutput]) {
        self.u64(outputs.len() as u64);
        let mut stack = outputs.iter().rev().collect::<Vec<_>>();
        while let Some(output) = stack.pop() {
            let segment = output.segment();
            self.u64(segment.producer_rule_id.0);
            self.bytes(segment.output_port.as_bytes());
            self.bytes(segment.semantic_key.as_bytes());
            self.u64(output.children().len() as u64);
            stack.extend(output.children().iter().rev());
        }
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

    fn assembly_recipe(&mut self, recipe: &crate::assembly_recipe::AssemblyRecipe) {
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
                RecognizedRecipeFeatureKind::Extrusion => 2,
                RecognizedRecipeFeatureKind::Pad => 7,
                RecognizedRecipeFeatureKind::Pocket => 3,
                RecognizedRecipeFeatureKind::Workplane => 4,
                RecognizedRecipeFeatureKind::Sketch => 5,
                RecognizedRecipeFeatureKind::SketchPocket => 6,
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

    fn sketch_constraint(&mut self, constraint: &SketchConstraint) {
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

    fn feature_kind(&mut self, kind: &FeatureKind) {
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
            FeatureKind::Profile { points_mm } => {
                self.byte(1);
                self.u64(points_mm.len() as u64);
                for point in points_mm {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            FeatureKind::SegmentProfile { segments, closed } => {
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
            FeatureKind::SplineProfile { control_points_mm } => {
                self.byte(14);
                self.u64(control_points_mm.len() as u64);
                for point in control_points_mm {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            FeatureKind::Extrusion { profile, height } => {
                self.byte(2);
                self.u64(profile.0);
                self.bytes(height.source_token.as_bytes());
                self.u64(height.millimetres.to_bits());
            }
            FeatureKind::Pad(spec) => {
                self.byte(19);
                self.u64(spec.sketch.0);
                self.u64(spec.region.0);
                self.feature_direction(spec.direction);
                self.feature_extent(&spec.extent);
            }
            FeatureKind::SketchPocket(spec) => {
                self.byte(20);
                self.u64(spec.target.0);
                self.u64(spec.sketch.0);
                self.u64(spec.region.0);
                self.feature_direction(spec.direction);
                self.feature_extent(&spec.extent);
                self.body_subshape_reference(&spec.support);
            }
            FeatureKind::ThroughCut { target, profile } => {
                self.byte(3);
                self.u64(target.0);
                self.u64(profile.0);
            }
            FeatureKind::Pocket {
                target,
                profile,
                depth,
            } => {
                self.byte(10);
                self.u64(target.0);
                self.u64(profile.0);
                self.bytes(depth.source_token.as_bytes());
                self.u64(depth.millimetres.to_bits());
            }
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
            FeatureKind::BottleProfileControl {
                profile,
                body_radius,
                body_height,
                shoulder_rise,
            } => {
                self.byte(6);
                self.u64(profile.0);
                for dimension in [body_radius, body_height, shoulder_rise] {
                    self.bytes(dimension.source_token.as_bytes());
                    self.u64(dimension.millimetres.to_bits());
                }
            }
            FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
            } => {
                self.byte(5);
                self.u64(target.0);
                self.u64(removed_faces.len() as u64);
                for role in removed_faces {
                    self.bytes(role.as_str().as_bytes());
                }
                self.bytes(thickness.source_token.as_bytes());
                self.u64(thickness.millimetres.to_bits());
            }
            FeatureKind::BottleEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => {
                self.byte(7);
                self.u64(target.0);
                self.u64(edges.len() as u64);
                for role in edges {
                    self.bytes(role.as_str().as_bytes());
                }
                self.byte(match kind {
                    EdgeFinishKind::Fillet => 1,
                    EdgeFinishKind::Chamfer => 2,
                });
                self.bytes(amount.source_token.as_bytes());
                self.u64(amount.millimetres.to_bits());
            }
            FeatureKind::TopologyShell {
                target,
                removed_faces,
                thickness,
                direction,
            } => {
                self.byte(21);
                self.u64(target.0);
                self.u64(removed_faces.len() as u64);
                for reference in removed_faces {
                    self.topological_reference(reference);
                }
                self.bytes(thickness.source_token.as_bytes());
                self.u64(thickness.millimetres.to_bits());
                self.byte(match direction {
                    ShellDirection::Inward => 1,
                    ShellDirection::Outward => 2,
                    ShellDirection::Symmetric => 3,
                });
            }
            FeatureKind::TopologyEdgeFinish {
                target,
                edges,
                kind,
                amount,
                fillet_radius_stations,
                chamfer_mode,
                chamfer_edge_sides,
            } => {
                self.byte(22);
                self.u64(target.0);
                self.u64(edges.len() as u64);
                for reference in edges {
                    self.topological_reference(reference);
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
            FeatureKind::TopologyFaceOffset {
                target,
                face,
                distance,
            } => {
                self.byte(23);
                self.u64(target.0);
                self.topological_reference(face);
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

    fn feature(&mut self, feature: &Feature) {
        self.u64(feature.id.0);
        self.u64(feature.definition_id.0);
        self.bytes(feature.name.as_bytes());
        self.feature_kind(&feature.kind);
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

    pub(super) fn authoritative_dependency(
        &mut self,
        product: &ProductModel,
        dependency: AuthoritativeDependency,
    ) {
        match dependency {
            AuthoritativeDependency::EvaluatorNode(id) => {
                self.byte(1);
                self.u64(id.0);
                if let Some(node) = product.evaluator_nodes.get(&id) {
                    self.byte(1);
                    self.node(node);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Override(id) => {
                self.byte(12);
                self.u64(id);
                if let Some(value) = product.overrides.get(&id) {
                    self.byte(1);
                    self.canonical_override(value);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::FeatureParameterBinding(target) => {
                self.byte(14);
                self.feature_parameter_target(&target);
                if let Some(binding) = product.feature_parameter_bindings.get(&target) {
                    self.byte(1);
                    self.feature_parameter_binding(binding);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Joint(id) => {
                self.byte(13);
                self.u64(id.0);
                if let Some(joint) = product.joints.get(&id) {
                    self.byte(1);
                    self.joint(joint);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Space(id) => {
                self.byte(19);
                self.u64(id.0);
                if let Some(space) = product.spaces.get(&id) {
                    self.byte(1);
                    self.space(space);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::ClearanceVolume(id) => {
                self.byte(20);
                self.u64(id.0);
                if let Some(clearance) = product.clearance_volumes.get(&id) {
                    self.byte(1);
                    self.clearance_volume(clearance);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::CamPlan(id) => {
                self.byte(39);
                self.u64(id.0);
                if let Some(plan) = product.cam_plans.get(&id) {
                    self.byte(1);
                    self.cam_plan(plan);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::DowelJoint(id) => {
                self.byte(40);
                self.u64(id.0);
                if let Some(joint) = product.dowel_joints.get(&id) {
                    self.byte(1);
                    self.dowel_joint(joint);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::PersistentDimension(id) => {
                self.byte(15);
                self.u64(id.0);
                if let Some(dimension) = product.persistent_dimensions.get(&id) {
                    self.byte(1);
                    self.persistent_dimension(dimension);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Tag(id) => {
                self.byte(16);
                self.u64(id.0);
                if let Some(tag) = product.tags.get(&id) {
                    self.byte(1);
                    self.tag(tag);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::ClassificationDimension(id) => {
                self.byte(27);
                self.u64(id.0);
                if let Some(dimension) = product.classification_dimensions.get(&id) {
                    self.byte(1);
                    self.bytes(dimension.name.as_bytes());
                    self.u64(dimension.categories.len() as u64);
                    for category in dimension.categories.values() {
                        self.u64(category.id.0);
                        self.bytes(category.name.as_bytes());
                    }
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::OccurrenceClassification(occurrence_id, dimension_id) => {
                self.byte(28);
                self.u64(occurrence_id.0);
                self.u64(dimension_id.0);
                self.optional_id(
                    product
                        .classification_assignments
                        .get(&(occurrence_id, dimension_id))
                        .map(|id| id.0),
                );
            }
            AuthoritativeDependency::Collection(id) => {
                self.byte(17);
                self.u64(id.0);
                if let Some(collection) = product.collections.get(&id) {
                    self.byte(1);
                    self.collection(collection);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Import(id) => {
                self.byte(22);
                self.u64(id.0);
                if let Some(receipt) = product.import_receipts.get(&id) {
                    self.byte(1);
                    self.import_receipt(receipt);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Definition(id) => {
                self.byte(2);
                self.u64(id.0);
                if let Some(definition) = product.definitions.get(&id) {
                    self.byte(1);
                    self.definition(definition);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Feature(id) => {
                self.byte(3);
                self.u64(id.0);
                if let Some(feature) = product.features.get(&id) {
                    self.byte(1);
                    self.feature(feature);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::BodyFeatureSuppression(definition_id, body_id) => {
                self.byte(26);
                self.u64(definition_id.0);
                self.u64(body_id.0);
                if let Some(suppressed) = product
                    .body_feature_suppression
                    .get(&(definition_id, body_id))
                {
                    self.byte(1);
                    self.u64(suppressed.len() as u64);
                    for feature_id in suppressed {
                        self.u64(feature_id.0);
                    }
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::ProductionCodes => {
                self.bytes(b"production-codes.v1");
                self.u64(product.production_codes.len() as u64);
                for (path, code) in &product.production_codes {
                    self.instance_path(path);
                    self.bytes(code.as_bytes());
                }
            }
            AuthoritativeDependency::AssemblyRecipe => {
                self.bytes(b"assembly-recipe-dependency.v1");
                if let Some(recipe) = product.assembly_recipe.as_deref() {
                    self.byte(1);
                    self.assembly_recipe(recipe);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Occurrence(id) => {
                self.byte(4);
                self.u64(id.0);
                if let Some(occurrence) = product.occurrences.get(&id) {
                    self.byte(1);
                    self.occurrence(occurrence);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::GroundedOccurrence(id) => {
                self.byte(23);
                self.u64(id.0);
                self.byte(u8::from(product.grounded_occurrences.contains(&id)));
            }
            AuthoritativeDependency::AssemblyMate(id) => {
                self.byte(24);
                self.u64(id.0);
                if let Some(mate) = product.assembly_mates.get(&id) {
                    self.byte(1);
                    self.assembly_mate(mate);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::AssemblyJoint(id) => {
                self.byte(34);
                self.u64(id.0);
                if let Some(joint) = product.assembly_joints.get(&id) {
                    self.byte(1);
                    self.assembly_joint(joint);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::AssemblyMotionCoupling(id) => {
                self.byte(36);
                self.u64(id.0);
                if let Some(coupling) = product.assembly_motion_couplings.get(&id) {
                    self.byte(1);
                    self.assembly_motion_coupling(coupling);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::MechanicalInterface(id) => {
                self.byte(37);
                self.u64(id.0);
                if let Some(interface) = product.mechanical_interfaces.get(&id) {
                    self.byte(1);
                    self.mechanical_interface(interface);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::MechanicalCondition(id) => {
                self.byte(38);
                self.u64(id.0);
                if let Some(condition) = product.mechanical_conditions.get(&id) {
                    self.byte(1);
                    self.mechanical_condition(condition);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::AssemblyMotionStudy(id) => {
                self.byte(35);
                self.u64(id.0);
                if let Some(study) = product.assembly_motion_studies.get(&id) {
                    self.byte(1);
                    self.assembly_motion_study(study);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::DrawingSheet(id) => {
                self.byte(25);
                self.u64(id.0);
                if let Some(sheet) = product.drawing_sheets.get(&id) {
                    self.byte(1);
                    self.drawing_sheet(sheet);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::Group(id) => {
                self.byte(5);
                self.u64(id.0);
                if let Some(group) = product.groups.get(&id) {
                    self.byte(1);
                    self.group(group);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::LocalGroup(key) => {
                self.byte(6);
                self.u64(key.definition_id.0);
                self.u64(key.local_id.0);
                if let Some(group) = product.local_groups.get(&key) {
                    self.byte(1);
                    self.local_group(group);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::LocalOccurrence(key) => {
                self.byte(7);
                self.u64(key.definition_id.0);
                self.u64(key.local_id.0);
                if let Some(occurrence) = product.local_occurrences.get(&key) {
                    self.byte(1);
                    self.local_occurrence(occurrence);
                } else {
                    self.byte(0);
                }
            }
            AuthoritativeDependency::DefinitionUsers(id) => {
                self.byte(8);
                self.u64(id.0);
                let world_users = product
                    .occurrences
                    .values()
                    .filter(|occurrence| occurrence.definition_id == id)
                    .map(|occurrence| occurrence.id)
                    .collect::<Vec<_>>();
                self.u64(world_users.len() as u64);
                for occurrence_id in world_users {
                    self.u64(occurrence_id.0);
                }
                let local_users = product
                    .local_occurrences
                    .values()
                    .filter(|occurrence| occurrence.definition_id == id)
                    .map(|occurrence| occurrence.key)
                    .collect::<Vec<_>>();
                self.u64(local_users.len() as u64);
                for key in local_users {
                    self.u64(key.definition_id.0);
                    self.u64(key.local_id.0);
                }
            }
            AuthoritativeDependency::FeatureUsers(id) => {
                self.byte(9);
                self.u64(id.0);
                let users = product
                    .features
                    .values()
                    .filter_map(|feature| match feature.kind {
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::Offset { base, .. },
                            ..
                        }) if base == id => Some(feature.id),
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::ConstructionPlane { feature: support },
                            ..
                        }) if support == id => Some(feature.id),
                        FeatureKind::Workplane(WorkplaneSpec {
                            support: WorkplaneSupport::PlanarFace { ref reference, .. },
                            ..
                        }) if reference.producer_feature_id == id => Some(feature.id),
                        FeatureKind::Sketch(ref spec) if spec.workplane == id => Some(feature.id),
                        FeatureKind::Extrusion { profile, .. }
                        | FeatureKind::PlanarOffset { profile, .. }
                            if profile == id =>
                        {
                            Some(feature.id)
                        }
                        FeatureKind::ThroughCut { target, profile }
                            if target == id || profile == id =>
                        {
                            Some(feature.id)
                        }
                        FeatureKind::Sweep { profile, path } if profile == id || path == id => {
                            Some(feature.id)
                        }
                        FeatureKind::WeldmentMember(ref spec)
                            if spec.profile == id || spec.path == id =>
                        {
                            Some(feature.id)
                        }
                        FeatureKind::WeldmentJoint(ref spec)
                            if spec.first_member == id || spec.second_member == id =>
                        {
                            Some(feature.id)
                        }
                        FeatureKind::Loft { ref sections, .. }
                            if sections.iter().any(|section| section.profile == id) =>
                        {
                            Some(feature.id)
                        }
                        FeatureKind::Boolean { target, tool, .. } if target == id || tool == id => {
                            Some(feature.id)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                self.u64(users.len() as u64);
                for feature_id in users {
                    self.u64(feature_id.0);
                }
            }
            AuthoritativeDependency::FeatureParameterBindings(id) => {
                self.byte(21);
                self.u64(id.0);
                let bindings = product
                    .feature_parameter_bindings
                    .values()
                    .filter(|binding| binding.target.feature_id == id)
                    .collect::<Vec<_>>();
                self.u64(bindings.len() as u64);
                for binding in bindings {
                    self.feature_parameter_binding(binding);
                }
            }
            AuthoritativeDependency::GroupChildren(id) => {
                self.byte(10);
                self.u64(id.0);
                let group_children = product
                    .groups
                    .values()
                    .filter(|group| group.parent == Some(id))
                    .map(|group| group.id)
                    .collect::<Vec<_>>();
                self.u64(group_children.len() as u64);
                for group_id in group_children {
                    self.u64(group_id.0);
                }
                let occurrence_children = product
                    .occurrences
                    .values()
                    .filter(|occurrence| occurrence.parent == Some(id))
                    .map(|occurrence| occurrence.id)
                    .collect::<Vec<_>>();
                self.u64(occurrence_children.len() as u64);
                for occurrence_id in occurrence_children {
                    self.u64(occurrence_id.0);
                }
            }
            AuthoritativeDependency::GroupSubtree(root) => {
                self.byte(11);
                self.u64(root.0);
                let descendants = product
                    .groups
                    .keys()
                    .cloned()
                    .filter(|id| group_is_descendant(product, root, *id))
                    .collect::<BTreeSet<_>>();
                self.u64(descendants.len() as u64);
                for id in &descendants {
                    self.group(&product.groups[id]);
                }
                let occurrences = product
                    .occurrences
                    .values()
                    .filter(|occurrence| {
                        occurrence
                            .parent
                            .is_some_and(|parent| descendants.contains(&parent))
                    })
                    .collect::<Vec<_>>();
                self.u64(occurrences.len() as u64);
                for occurrence in occurrences {
                    self.occurrence(occurrence);
                }
            }
            AuthoritativeDependency::OccurrenceCollections(id) => {
                self.byte(18);
                self.u64(id.0);
                self.byte(u8::from(product.grounded_occurrences.contains(&id)));
                let collections = product
                    .collections
                    .values()
                    .filter(|collection| collection.occurrence_ids.contains(&id))
                    .collect::<Vec<_>>();
                self.u64(collections.len() as u64);
                for collection in collections {
                    self.collection(collection);
                }
                let mates = product
                    .assembly_mates
                    .values()
                    .filter(|mate| {
                        mate.endpoint_a().occurrence_id() == id
                            || mate.endpoint_b().occurrence_id() == id
                    })
                    .collect::<Vec<_>>();
                self.u64(mates.len() as u64);
                for mate in mates {
                    self.assembly_mate(mate);
                }
                let dowel_joints = product
                    .dowel_joints
                    .values()
                    .filter(|joint| {
                        joint.first.instance_path.root_occurrence() == id
                            || joint.second.instance_path.root_occurrence() == id
                    })
                    .collect::<Vec<_>>();
                self.u64(dowel_joints.len() as u64);
                for joint in dowel_joints {
                    self.dowel_joint(joint);
                }
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

    pub(super) fn command(&mut self, command: &CanonicalCommand) {
        match command {
            CanonicalCommand::SetProductionCode {
                instance_path,
                code,
            } => {
                self.bytes(b"set-production-code.v1");
                self.instance_path(instance_path);
                self.byte(u8::from(code.is_some()));
                if let Some(code) = code {
                    self.bytes(code.as_bytes());
                }
            }
            CanonicalCommand::CreateEvaluatorNode {
                id,
                name,
                dimension,
                dependencies,
            } => {
                self.byte(1);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
                self.u64(dependencies.len() as u64);
                for dependency in dependencies {
                    self.u64(dependency.0);
                }
            }
            CanonicalCommand::SetEvaluatorDimension { id, dimension } => {
                self.byte(2);
                self.u64(id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::RenameEvaluatorNode { id, name } => {
                self.byte(3);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::RecordImport(receipt) => {
                self.byte(50);
                self.import_receipt(receipt);
            }
            CanonicalCommand::CreateDefinition { id, name } => {
                self.byte(10);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::DeleteDefinition { id } => {
                self.byte(11);
                self.u64(id.0);
            }
            CanonicalCommand::RenameDefinition { id, name } => {
                self.byte(12);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::CreateBody {
                definition_id,
                id,
                name,
                visible,
            } => {
                self.byte(60);
                self.u64(definition_id.0);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::DeleteBody { definition_id, id } => {
                self.byte(61);
                self.u64(definition_id.0);
                self.u64(id.0);
            }
            CanonicalCommand::RenameBody {
                definition_id,
                id,
                name,
            } => {
                self.byte(62);
                self.u64(definition_id.0);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::SetActiveBody { definition_id, id } => {
                self.byte(63);
                self.u64(definition_id.0);
                self.u64(id.0);
            }
            CanonicalCommand::SetBodyVisibility {
                definition_id,
                id,
                visible,
            } => {
                self.byte(64);
                self.u64(definition_id.0);
                self.u64(id.0);
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::ConsumeBody {
                definition_id,
                id,
                by_feature_id,
            } => {
                self.byte(66);
                self.u64(definition_id.0);
                self.u64(id.0);
                self.u64(by_feature_id.0);
            }
            CanonicalCommand::SetFeatureBodyOwnership { id, ownership } => {
                self.byte(65);
                self.u64(id.0);
                self.u64(ownership.input_body_ids.len() as u64);
                for body_id in &ownership.input_body_ids {
                    self.u64(body_id.0);
                }
                self.optional_id(ownership.output_body_id.map(|body_id| body_id.0));
            }
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id,
                body_id,
                suppressed_feature_ids,
            } => {
                self.byte(68);
                self.u64(definition_id.0);
                self.u64(body_id.0);
                self.u64(suppressed_feature_ids.len() as u64);
                for feature_id in suppressed_feature_ids {
                    self.u64(feature_id.0);
                }
            }
            CanonicalCommand::CreateFeature {
                id,
                definition_id,
                name,
                kind,
            } => {
                self.byte(13);
                self.u64(id.0);
                self.u64(definition_id.0);
                self.bytes(name.as_bytes());
                self.feature_kind(kind);
            }
            CanonicalCommand::DeleteFeature { id } => {
                self.byte(14);
                self.u64(id.0);
            }
            CanonicalCommand::SetFeatureDimension { id, dimension } => {
                self.byte(15);
                self.u64(id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::SetFeatureParameter { target, dimension } => {
                self.byte(101);
                self.u64(target.feature_id.0);
                self.bytes(target.path.as_str().as_bytes());
                self.byte(match target.value_type {
                    ParameterValueType::Length => 1,
                    ParameterValueType::Angle => 2,
                    ParameterValueType::Scalar => 3,
                });
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::CreateSketchConstraint { id, constraint } => {
                self.byte(98);
                self.u64(id.0);
                self.sketch_constraint(constraint);
            }
            CanonicalCommand::ReplaceSketchConstraint { id, constraint } => {
                self.byte(99);
                self.u64(id.0);
                self.sketch_constraint(constraint);
            }
            CanonicalCommand::DeleteSketchConstraint { id, constraint_id } => {
                self.byte(100);
                self.u64(id.0);
                self.u64(constraint_id.0);
            }
            CanonicalCommand::SetSketchConstraintDimension {
                id,
                constraint_id,
                dimension,
            } => {
                self.byte(67);
                self.u64(id.0);
                self.u64(constraint_id.0);
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::SplitSketchEntity {
                id,
                entity_id,
                new_entity_id,
                parameter,
                joint_constraint_ids,
            } => {
                self.byte(91);
                self.u64(id.0);
                self.u64(entity_id.0);
                self.u64(new_entity_id.0);
                self.u64(parameter.to_bits());
                self.u64(joint_constraint_ids.len() as u64);
                for constraint_id in joint_constraint_ids {
                    self.u64(constraint_id.0);
                }
            }
            CanonicalCommand::JoinSketchEntities {
                id,
                source_entity_id,
                source_endpoint,
                consumed_entity_id,
                consumed_endpoint,
            } => {
                self.byte(97);
                self.u64(id.0);
                self.u64(source_entity_id.0);
                self.byte(match source_endpoint {
                    SketchPointKind::Start => 1,
                    SketchPointKind::End => 2,
                    SketchPointKind::Center => 3,
                    SketchPointKind::Control1 => 4,
                    SketchPointKind::Control2 => 5,
                });
                self.u64(consumed_entity_id.0);
                self.byte(match consumed_endpoint {
                    SketchPointKind::Start => 1,
                    SketchPointKind::End => 2,
                    SketchPointKind::Center => 3,
                    SketchPointKind::Control1 => 4,
                    SketchPointKind::Control2 => 5,
                });
            }
            CanonicalCommand::TrimSketchEntity {
                id,
                entity_id,
                start_parameter,
                end_parameter,
            } => {
                self.byte(92);
                self.u64(id.0);
                self.u64(entity_id.0);
                self.u64(start_parameter.to_bits());
                self.u64(end_parameter.to_bits());
            }
            CanonicalCommand::ExtendSketchEntity {
                id,
                entity_id,
                endpoint,
                parameter,
            } => {
                self.byte(93);
                self.u64(id.0);
                self.u64(entity_id.0);
                self.byte(match endpoint {
                    SketchPointKind::Start => 1,
                    SketchPointKind::End => 2,
                    SketchPointKind::Center => 3,
                    SketchPointKind::Control1 => 4,
                    SketchPointKind::Control2 => 5,
                });
                self.u64(parameter.to_bits());
            }
            CanonicalCommand::OffsetSketchEntity {
                id,
                entity_id,
                new_entity_id,
                distance_mm,
                side,
            } => {
                self.byte(94);
                self.u64(id.0);
                self.u64(entity_id.0);
                self.u64(new_entity_id.0);
                self.u64(distance_mm.to_bits());
                self.byte(match side {
                    SketchOffsetSide::Left => 1,
                    SketchOffsetSide::Right => 2,
                });
            }
            CanonicalCommand::ProjectSketchEntity {
                id,
                source_feature_id,
                source_entity_id,
                new_entity_id,
                projection_constraint_id,
            } => {
                self.byte(95);
                self.u64(id.0);
                self.u64(source_feature_id.0);
                self.u64(source_entity_id.0);
                self.u64(new_entity_id.0);
                self.u64(projection_constraint_id.0);
            }
            CanonicalCommand::SetSketchEntityConstruction {
                id,
                entity_id,
                construction,
                construction_constraint_id,
            } => {
                self.byte(96);
                self.u64(id.0);
                self.u64(entity_id.0);
                self.byte(u8::from(*construction));
                self.u64(construction_constraint_id.0);
            }
            CanonicalCommand::TranslateProfile { id, delta_mm } => {
                self.byte(71);
                self.u64(id.0);
                self.u64(delta_mm[0].to_bits());
                self.u64(delta_mm[1].to_bits());
            }
            CanonicalCommand::SetBottleControlDimension {
                id,
                control,
                dimension,
            } => {
                self.byte(31);
                self.u64(id.0);
                self.byte(match control {
                    BottleControlDimension::BodyRadius => 1,
                    BottleControlDimension::BodyHeight => 2,
                    BottleControlDimension::ShoulderRise => 3,
                });
                self.bytes(dimension.source_token.as_bytes());
                self.u64(dimension.millimetres.to_bits());
            }
            CanonicalCommand::SetBottleEdgeFinishKind { id, kind } => {
                self.byte(32);
                self.u64(id.0);
                self.byte(match kind {
                    EdgeFinishKind::Fillet => 1,
                    EdgeFinishKind::Chamfer => 2,
                });
            }
            CanonicalCommand::SetProfilePoints { id, points_mm } => {
                self.byte(27);
                self.u64(id.0);
                self.u64(points_mm.len() as u64);
                for point in points_mm {
                    self.u64(point[0].to_bits());
                    self.u64(point[1].to_bits());
                }
            }
            CanonicalCommand::CreateOccurrence {
                id,
                definition_id,
                name,
                transform,
                parent,
                tag,
                visible,
            } => {
                self.byte(16);
                self.u64(id.0);
                self.u64(definition_id.0);
                self.bytes(name.as_bytes());
                self.transform(*transform);
                self.optional_id(parent.map(|id| id.0));
                self.optional_id(tag.map(|id| id.0));
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::DeleteOccurrence { id } => {
                self.byte(17);
                self.u64(id.0);
            }
            CanonicalCommand::SetOccurrenceTransform { id, transform } => {
                self.byte(18);
                self.u64(id.0);
                self.transform(*transform);
            }
            CanonicalCommand::RenameEntity { id, name } => {
                self.byte(69);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::GuardAssemblyRecompute {
                source_revision,
                source_digest,
            } => {
                self.byte(56);
                self.u64(*source_revision);
                self.bytes(source_digest.as_bytes());
            }
            CanonicalCommand::ApplyAssemblySolve {
                source_revision,
                source_digest,
                transforms,
                instance_transforms,
            } => {
                self.byte(54);
                self.u64(*source_revision);
                self.bytes(source_digest.as_bytes());
                self.u64(transforms.len() as u64);
                for (id, transform) in transforms {
                    self.u64(id.0);
                    self.transform(*transform);
                }
                self.u64(instance_transforms.len() as u64);
                for (path, transform) in instance_transforms {
                    self.instance_path(path);
                    self.transform(*transform);
                }
            }
            CanonicalCommand::SetOccurrenceGrounded { id, grounded } => {
                self.byte(50);
                self.u64(id.0);
                self.byte(u8::from(*grounded));
            }
            CanonicalCommand::CreateAssemblyMate(mate) => {
                self.byte(51);
                self.assembly_mate(mate);
            }
            CanonicalCommand::RebindAssemblyMate(mate) => {
                self.byte(55);
                self.assembly_mate(mate);
            }
            CanonicalCommand::SetAssemblyMateKind { id, kind } => {
                self.byte(52);
                self.u64(id.0);
                match kind {
                    AssemblyMateKind::CoincidentPlanar {
                        offset_mm,
                        reversed,
                    } => {
                        self.byte(1);
                        self.u64(offset_mm.to_bits());
                        self.byte(u8::from(*reversed));
                    }
                    AssemblyMateKind::ConcentricAxial { reversed } => {
                        self.byte(2);
                        self.byte(u8::from(*reversed));
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
            CanonicalCommand::DeleteAssemblyMate { id } => {
                self.byte(53);
                self.u64(id.0);
            }
            CanonicalCommand::CreateAssemblyJoint(joint) => {
                self.byte(74);
                self.assembly_joint(joint);
            }
            CanonicalCommand::SetAssemblyJointKind { id, kind } => {
                self.byte(75);
                self.u64(id.0);
                self.assembly_joint_kind(*kind);
            }
            CanonicalCommand::SetAssemblyJointPosition { id, position } => {
                self.byte(76);
                self.u64(id.0);
                self.u64(position.to_bits());
            }
            CanonicalCommand::SetAssemblyJointLimits { id, limits } => {
                self.byte(77);
                self.u64(id.0);
                self.assembly_joint_limits(*limits);
            }
            CanonicalCommand::DeleteAssemblyJoint { id } => {
                self.byte(78);
                self.u64(id.0);
            }
            CanonicalCommand::CreateAssemblyMotionCoupling(coupling) => {
                self.byte(82);
                self.assembly_motion_coupling(coupling);
            }
            CanonicalCommand::UpdateAssemblyMotionCoupling(coupling) => {
                self.byte(83);
                self.assembly_motion_coupling(coupling);
            }
            CanonicalCommand::DeleteAssemblyMotionCoupling { id } => {
                self.byte(84);
                self.u64(id.0);
            }
            CanonicalCommand::CreateAssemblyMotionStudy(study) => {
                self.byte(79);
                self.assembly_motion_study(study);
            }
            CanonicalCommand::UpdateAssemblyMotionStudy(study) => {
                self.byte(80);
                self.assembly_motion_study(study);
            }
            CanonicalCommand::DeleteAssemblyMotionStudy { id } => {
                self.byte(81);
                self.u64(id.0);
            }
            CanonicalCommand::CreateMechanicalInterface(interface) => {
                self.byte(85);
                self.mechanical_interface(interface);
            }
            CanonicalCommand::UpdateMechanicalInterface(interface) => {
                self.byte(86);
                self.mechanical_interface(interface);
            }
            CanonicalCommand::DeleteMechanicalInterface { id } => {
                self.byte(87);
                self.u64(id.0);
            }
            CanonicalCommand::CreateMechanicalCondition(condition) => {
                self.byte(88);
                self.mechanical_condition(condition);
            }
            CanonicalCommand::UpdateMechanicalCondition(condition) => {
                self.byte(89);
                self.mechanical_condition(condition);
            }
            CanonicalCommand::DeleteMechanicalCondition { id } => {
                self.byte(90);
                self.u64(id.0);
            }
            CanonicalCommand::CreateDrawingSheet(sheet) => {
                self.byte(57);
                self.drawing_sheet(sheet);
            }
            CanonicalCommand::UpdateDrawingSheet(sheet) => {
                self.byte(58);
                self.drawing_sheet(sheet);
            }
            CanonicalCommand::DeleteDrawingSheet { id } => {
                self.byte(59);
                self.u64(id.0);
            }
            CanonicalCommand::SetOccurrenceColor { id, color } => {
                self.bytes(b"set-occurrence-color");
                self.u64(id.0);
                self.byte(u8::from(color.is_some()));
                if let Some(color) = color {
                    for channel in color {
                        self.byte(*channel);
                    }
                }
            }
            CanonicalCommand::SetOccurrenceVisibility { id, visible } => {
                self.byte(19);
                self.u64(id.0);
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::SetOccurrenceTag { id, tag } => {
                self.byte(41);
                self.u64(id.0);
                self.optional_id(tag.map(|id| id.0));
            }
            CanonicalCommand::RepointOccurrence { id, definition_id } => {
                self.byte(20);
                self.u64(id.0);
                self.u64(definition_id.0);
            }
            CanonicalCommand::SetOccurrenceParent { id, parent } => {
                self.byte(21);
                self.u64(id.0);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::CreateGroup {
                id,
                name,
                transform,
                parent,
            } => {
                self.byte(22);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.transform(*transform);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::DeleteGroup { id } => {
                self.byte(23);
                self.u64(id.0);
            }
            CanonicalCommand::SetGroupTransform { id, transform } => {
                self.byte(24);
                self.u64(id.0);
                self.transform(*transform);
            }
            CanonicalCommand::SetGroupParent { id, parent } => {
                self.byte(25);
                self.u64(id.0);
                self.optional_id(parent.map(|id| id.0));
            }
            CanonicalCommand::CloneDefinitionAndRepoint(plan) => {
                self.byte(26);
                self.u64(plan.occurrence_id.0);
                self.u64(plan.source_definition_id.0);
                self.u64(plan.new_definition_id.0);
                self.bytes(plan.new_definition_name.as_bytes());
                self.u64(plan.feature_id_map.len() as u64);
                for (source_id, new_id) in &plan.feature_id_map {
                    self.u64(source_id.0);
                    self.u64(new_id.0);
                }
            }
            CanonicalCommand::ConvertGroupToComponent(plan) => {
                self.byte(28);
                self.u64(plan.group_id.0);
                self.u64(plan.new_definition_id.0);
                self.u64(plan.new_occurrence_id.0);
                self.bytes(plan.component_name.as_bytes());
            }
            CanonicalCommand::ApplySolidTool(plan) => {
                self.byte(49);
                self.byte(match plan.operation {
                    BooleanOperation::Cut => 1,
                    BooleanOperation::Union => 2,
                    BooleanOperation::Intersect => 3,
                    BooleanOperation::Split => 4,
                });
                self.u64(plan.target_occurrence_id.0);
                self.u64(plan.target_feature_id.0);
                self.u64(plan.tool_occurrence_id.0);
                self.u64(plan.tool_feature_id.0);
                self.u64(plan.result_definition_id.0);
                self.u64(plan.result_feature_ids.len() as u64);
                for id in &plan.result_feature_ids {
                    self.u64(id.0);
                }
                self.bytes(plan.result_definition_name.as_bytes());
                self.bytes(plan.result_feature_name.as_bytes());
                self.byte(u8::from(plan.keep_tool));
            }
            CanonicalCommand::CreateExpressionNode {
                id,
                name,
                expression,
            } => {
                self.byte(4);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(expression.as_bytes());
            }
            CanonicalCommand::CreateRuleNode {
                id,
                name,
                expression,
                input_ports,
                output_ports,
                outputs,
                override_parameters,
            } => {
                self.byte(5);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.bytes(expression.as_bytes());
                self.ports(input_ports);
                self.ports(output_ports);
                self.rule_outputs(outputs);
                self.u64(override_parameters.len() as u64);
                for parameter in override_parameters {
                    self.bytes(parameter.name().as_bytes());
                    self.byte(match parameter.merge_policy() {
                        OverrideMergePolicy::Replace => 1,
                    });
                }
            }
            CanonicalCommand::SetNodeExpression { id, expression } => {
                self.byte(6);
                self.u64(id.0);
                self.bytes(expression.as_bytes());
            }
            CanonicalCommand::SetRuleOutputs { id, outputs } => {
                self.byte(7);
                self.u64(id.0);
                self.rule_outputs(outputs);
            }
            CanonicalCommand::UpsertOverride(value) => {
                self.byte(8);
                self.canonical_override(value);
            }
            CanonicalCommand::DeleteOverride { id } => {
                self.byte(9);
                self.u64(*id);
            }
            CanonicalCommand::UpsertFeatureParameterBinding(binding) => {
                self.byte(33);
                self.feature_parameter_binding(binding);
            }
            CanonicalCommand::DeleteFeatureParameterBinding { target } => {
                self.byte(34);
                self.feature_parameter_target(target);
            }
            CanonicalCommand::RecomputeFeatureParameters { identity, scope } => {
                self.byte(35);
                self.bytes(identity.evaluator.as_bytes());
                self.bytes(identity.schema.as_bytes());
                self.bytes(identity.tolerance.as_bytes());
                match &identity.backend {
                    Some(backend) => {
                        self.byte(1);
                        self.bytes(backend.as_bytes());
                    }
                    None => self.byte(0),
                }
                match scope {
                    FeatureParameterRecomputeScope::All => self.byte(0),
                    FeatureParameterRecomputeScope::AffectedBy(nodes) => {
                        self.byte(1);
                        self.u64(nodes.len() as u64);
                        for node in nodes {
                            self.u64(node.0);
                        }
                    }
                }
            }
            CanonicalCommand::UpsertJoint(joint) => {
                self.byte(29);
                self.joint(joint);
            }
            CanonicalCommand::DeleteJoint { id } => {
                self.byte(30);
                self.u64(id.0);
            }
            CanonicalCommand::UpsertSpace(space) => {
                self.byte(45);
                self.space(space);
            }
            CanonicalCommand::DeleteSpace { id } => {
                self.byte(46);
                self.u64(id.0);
            }
            CanonicalCommand::UpsertClearanceVolume(clearance) => {
                self.byte(47);
                self.clearance_volume(clearance);
            }
            CanonicalCommand::DeleteClearanceVolume { id } => {
                self.byte(48);
                self.u64(id.0);
            }
            CanonicalCommand::UpsertCamPlan(plan) => {
                self.byte(112);
                self.cam_plan(plan);
            }
            CanonicalCommand::DeleteCamPlan { id } => {
                self.byte(113);
                self.u64(id.0);
            }
            CanonicalCommand::UpsertDowelJoint(joint) => {
                self.byte(114);
                self.dowel_joint(joint);
            }
            CanonicalCommand::DeleteDowelJoint { id } => {
                self.byte(115);
                self.u64(id.0);
            }
            CanonicalCommand::SetAssemblyRecipe(recipe) => {
                self.byte(116);
                self.assembly_recipe(recipe);
            }
            CanonicalCommand::ClearAssemblyRecipe => self.byte(117),
            CanonicalCommand::UpsertPersistentDimension(dimension) => {
                self.byte(36);
                self.persistent_dimension(dimension);
            }
            CanonicalCommand::DeletePersistentDimension { id } => {
                self.byte(37);
                self.u64(id.0);
            }
            CanonicalCommand::CreateTag { id, name, visible } => {
                self.byte(38);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::DeleteTag { id } => {
                self.byte(39);
                self.u64(id.0);
            }
            CanonicalCommand::SetTagVisibility { id, visible } => {
                self.byte(40);
                self.u64(id.0);
                self.byte(u8::from(*visible));
            }
            CanonicalCommand::SetTagName { id, name } => {
                self.byte(70);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::UpsertClassificationDimension {
                id,
                name,
                categories,
            } => {
                self.byte(72);
                self.u64(id.0);
                self.bytes(name.as_bytes());
                self.u64(categories.len() as u64);
                for (category_id, category_name) in categories {
                    self.u64(category_id.0);
                    self.bytes(category_name.as_bytes());
                }
            }
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id,
                dimension_id,
                category_id,
            } => {
                self.byte(73);
                self.u64(occurrence_id.0);
                self.u64(dimension_id.0);
                self.optional_id(category_id.map(|id| id.0));
            }
            CanonicalCommand::CreateCollection { id, name } => {
                self.byte(42);
                self.u64(id.0);
                self.bytes(name.as_bytes());
            }
            CanonicalCommand::DeleteCollection { id } => {
                self.byte(43);
                self.u64(id.0);
            }
            CanonicalCommand::SetCollectionOccurrences { id, occurrence_ids } => {
                self.byte(44);
                self.u64(id.0);
                self.u64(occurrence_ids.len() as u64);
                for occurrence_id in occurrence_ids {
                    self.u64(occurrence_id.0);
                }
            }
        }
    }

    pub(super) fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}
