//! Canonical digests of document content.
//!
//! A digest hashes the serde form of a value, the same definition the native file stores,
//! so a new field takes part in every digest without further code. A saved field that is
//! evidence of an evaluation or is derived from other fields is not document identity: it
//! declares `#[serde(serialize_with = "crate::document::derived")]` (see
//! [`ketchup_geometry::derived`]) and hashes as a unit.

use super::*;
use serde::Serialize;

pub(super) fn digest_snapshot(snapshot: &Snapshot) -> String {
    digest_product(snapshot.product.as_ref())
}

/// Document digest of `product`, the product model or a value with its serde form.
pub(crate) fn digest_product(product: &impl Serialize) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.document.v4");
    digest.value(product);
    digest.finish()
}

pub(super) fn digest_feature(feature: &Feature) -> String {
    let mut digest = StableDigest::new();
    digest.bytes(b"ketchup.feature.v2");
    digest.value(feature);
    digest.finish()
}

/// 64-bit FNV-1a over labelled CBOR values.
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
        for byte in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
            self.byte(*byte);
        }
    }

    /// Hashes the identity of `value`: its serde form without derived fields.
    pub(super) fn value(&mut self, value: &impl Serialize) {
        identity_form(|| ciborium::into_writer(value, &mut *self))
            .expect("document values serialize");
    }

    /// Hashes what `dependency` names in `product`, or its absence.
    pub(super) fn authoritative_dependency(
        &mut self,
        product: &ProductModel,
        dependency: &AuthoritativeDependency,
    ) {
        use AuthoritativeDependency as D;
        self.value(dependency);
        match dependency {
            D::EvaluatorNode(id) => self.value(&product.evaluator_nodes.get(id)),
            D::Override(id) => self.value(&product.overrides.get(id)),
            D::FeatureParameterBinding(target) => {
                self.value(&product.feature_parameter_bindings.get(target));
            }
            D::Joint(id) => self.value(&product.joints.get(id)),
            D::Space(id) => self.value(&product.spaces.get(id)),
            D::ClearanceVolume(id) => self.value(&product.clearance_volumes.get(id)),
            D::CamPlan(id) => self.value(&product.cam_plans.get(id)),
            D::PinJoint(id) => self.value(&product.pin_joints.get(id)),
            D::Tolerance => self.value(&product.tolerance),
            D::FloorHeight => self.value(&product.floor_z_mm),
            D::GroundedInstances => self.value(&product.grounded_instances),
            D::ProductionCodes => self.value(&product.production_codes),
            D::AssemblyRecipe => self.value(&product.assembly_recipe),
            D::PersistentDimension(id) => self.value(&product.persistent_dimensions.get(id)),
            D::Tag(id) => self.value(&product.tags.get(id)),
            D::ClassificationDimension(id) => {
                self.value(&product.classification_dimensions.get(id));
            }
            D::OccurrenceClassification(occurrence_id, dimension_id) => self.value(
                &product
                    .classification_assignments
                    .get(&(*occurrence_id, *dimension_id)),
            ),
            D::Collection(id) => self.value(&product.collections.get(id)),
            D::Import(id) => self.value(&product.import_receipts.get(id)),
            D::Definition(id) => self.value(&product.definitions.get(id)),
            D::Feature(id) => self.value(&product.features.get(id)),
            D::BodyFeatureSuppression(definition_id, body_id) => self.value(
                &product
                    .body_feature_suppression
                    .get(&(*definition_id, *body_id)),
            ),
            D::Occurrence(id) => self.value(&product.occurrences.get(id)),
            D::GroundedOccurrence(id) => self.value(&product.grounded_occurrences.contains(id)),
            D::AssemblyMate(id) => self.value(&product.assembly_mates.get(id)),
            D::AssemblyJoint(id) => self.value(&product.assembly_joints.get(id)),
            D::AssemblyMotionCoupling(id) => {
                self.value(&product.assembly_motion_couplings.get(id));
            }
            D::AssemblyMotionStudy(id) => self.value(&product.assembly_motion_studies.get(id)),
            D::MechanicalInterface(id) => self.value(&product.mechanical_interfaces.get(id)),
            D::MechanicalCondition(id) => self.value(&product.mechanical_conditions.get(id)),
            D::DrawingSheet(id) => self.value(&product.drawing_sheets.get(id)),
            D::Group(id) => self.value(&product.groups.get(id)),
            D::LocalGroup(key) => self.value(&product.local_groups.get(key)),
            D::LocalOccurrence(key) => self.value(&product.local_occurrences.get(key)),
            D::DefinitionUsers(id) => self.value(&(
                product
                    .occurrences
                    .values()
                    .filter(|occurrence| occurrence.definition_id == *id)
                    .map(|occurrence| occurrence.id)
                    .collect::<Vec<_>>(),
                product
                    .local_occurrences
                    .values()
                    .filter(|occurrence| occurrence.definition_id == *id)
                    .map(|occurrence| occurrence.key)
                    .collect::<Vec<_>>(),
            )),
            D::FeatureUsers(id) => self.value(
                &product
                    .features
                    .values()
                    .filter(|feature| feature.kind.authoritative_dependencies().contains(id))
                    .map(|feature| feature.id)
                    .collect::<Vec<_>>(),
            ),
            D::FeatureParameterBindings(id) => self.value(
                &product
                    .feature_parameter_bindings
                    .values()
                    .filter(|binding| binding.target.feature_id == *id)
                    .collect::<Vec<_>>(),
            ),
            D::GroupChildren(id) => self.value(&(
                product
                    .groups
                    .values()
                    .filter(|group| group.parent == Some(*id))
                    .map(|group| group.id)
                    .collect::<Vec<_>>(),
                product
                    .occurrences
                    .values()
                    .filter(|occurrence| occurrence.parent == Some(*id))
                    .map(|occurrence| occurrence.id)
                    .collect::<Vec<_>>(),
            )),
            D::GroupSubtree(root) => {
                let groups = product
                    .groups
                    .iter()
                    .filter(|(id, _)| group_is_descendant(product, *root, **id))
                    .collect::<BTreeMap<_, _>>();
                let occurrences = product
                    .occurrences
                    .values()
                    .filter(|occurrence| {
                        occurrence
                            .parent
                            .is_some_and(|parent| groups.contains_key(&parent))
                    })
                    .collect::<Vec<_>>();
                self.value(&(groups, occurrences));
            }
            D::OccurrenceCollections(id) => self.value(&(
                product.grounded_occurrences.contains(id),
                product
                    .collections
                    .values()
                    .filter(|collection| collection.occurrence_ids.contains(id))
                    .collect::<Vec<_>>(),
                product
                    .assembly_mates
                    .values()
                    .filter(|mate| {
                        mate.endpoint_a().occurrence_id() == *id
                            || mate.endpoint_b().occurrence_id() == *id
                    })
                    .collect::<Vec<_>>(),
                product
                    .pin_joints
                    .values()
                    .filter(|joint| {
                        joint.first.instance_path.root_occurrence() == *id
                            || joint.second.instance_path.root_occurrence() == *id
                    })
                    .collect::<Vec<_>>(),
            )),
        }
    }

    pub(super) fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}

impl std::io::Write for StableDigest {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for byte in bytes {
            self.byte(*byte);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exact_product::{BodySubshapeRef, ReferenceStability};
    use ketchup_geometry::sketch::{WorkplaneFrame, WorkplaneSupportHealth};

    fn identity(value: &impl Serialize) -> String {
        let mut digest = StableDigest::new();
        digest.value(value);
        digest.finish()
    }

    fn saved(value: &impl Serialize) -> Vec<u8> {
        let mut bytes = Vec::new();
        ciborium::into_writer(value, &mut bytes).unwrap();
        bytes
    }

    fn face(result_fingerprint: &str) -> BodySubshapeRef {
        BodySubshapeRef {
            schema: "schema".to_owned(),
            document_id: DocumentId(1),
            definition_id: DefinitionId(2),
            profile_feature_id: FeatureId(3),
            producer_feature_id: FeatureId(4),
            semantic_role: "cap_end".to_owned(),
            source_element_id: "face".to_owned(),
            expected_type: "face".to_owned(),
            expected_cardinality: 1,
            stability: ReferenceStability::Guaranteed,
            canonical_input_digest: "input".to_owned(),
            exact_input_digest: "exact".to_owned(),
            result_fingerprint: result_fingerprint.to_owned(),
            evaluator: "evaluator".to_owned(),
            backend: "backend".to_owned(),
            tolerance: "tolerance".to_owned(),
            lineage_digest: "lineage".to_owned(),
            corroborating_geometry_fingerprint: "geometry".to_owned(),
        }
    }

    fn workplane(support: WorkplaneSupport, origin_z: f64) -> WorkplaneSpec {
        WorkplaneSpec {
            support,
            frame: WorkplaneFrame {
                origin_mm: [0.0, 0.0, origin_z],
                ..WorkplaneFrame::principal(ketchup_geometry::sketch::PrincipalPlane::Xy)
            },
        }
    }

    #[test]
    fn evaluation_evidence_is_saved_but_does_not_identify_the_value() {
        let (first, second) = (face("result-a"), face("result-b"));
        assert_eq!(identity(&first), identity(&second));
        assert_ne!(saved(&first), saved(&second));
        let renamed = BodySubshapeRef {
            semantic_role: "cap_start".to_owned(),
            ..face("result-a")
        };
        assert_ne!(identity(&first), identity(&renamed));
    }

    #[test]
    fn hashing_leaves_later_saving_complete() {
        let reference = face("result-a");
        let before = saved(&reference);
        identity(&reference);
        assert_eq!(saved(&reference), before);
    }

    #[test]
    fn only_a_face_supported_workplane_takes_its_frame_from_evaluation() {
        let on_face = |health, origin_z| {
            workplane(
                WorkplaneSupport::PlanarFace {
                    reference: Box::new(face("result-a")),
                    health,
                },
                origin_z,
            )
        };
        assert_eq!(
            identity(&on_face(WorkplaneSupportHealth::Resolved, 0.0)),
            identity(&on_face(WorkplaneSupportHealth::Stale, 5.0))
        );
        assert_ne!(
            saved(&on_face(WorkplaneSupportHealth::Resolved, 0.0)),
            saved(&on_face(WorkplaneSupportHealth::Resolved, 5.0))
        );
        assert_ne!(
            identity(&workplane(WorkplaneSupport::Free, 0.0)),
            identity(&workplane(WorkplaneSupport::Free, 5.0))
        );
    }
}
