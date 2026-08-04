use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    OccurrenceId, Transform,
};
use ketchup_core::exact_product::{ExactFaceRole, ExactRectangleRequest};
use ketchup_exact::{
    ExactBackend, RectangleExtrudeSpec, ReferenceResolution, StabilityClass,
    capture_guaranteed_references, resolve_subshape_reference,
};
use ketchup_interaction::exact_projection::ExactInteractionProjection;
use ketchup_interaction::{Ray, Vec3};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::BTreeMap;
use std::sync::Arc;

const PROFILE: FeatureId = FeatureId(11);
const EXTRUSION: FeatureId = FeatureId(12);
const DEFINITION: DefinitionId = DefinitionId(10);

#[test]
fn preregistered_c1b_product_corpus_has_zero_wrong_identities_and_survives_save_open() {
    let corpus = include_str!("fixtures/c1b/rectangle-v1.tsv");
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut observed_cases = 0;
    let mut observed_roles = 0;

    for line in corpus
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
    {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 4, "malformed preregistered C1b row: {line}");
        let case_id = fields[0];
        let width = fields[1].parse::<f64>().unwrap();
        let depth = fields[2].parse::<f64>().unwrap();
        let height = fields[3].parse::<f64>().unwrap();
        let mut document = rectangle_document(width, depth, height);
        let snapshot = document.current();
        let request = ExactRectangleRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
        let package = supervisor.evaluate_rectangle(&request).unwrap();
        assert!(package.is_current(&snapshot));
        assert_eq!(package.vertices.len(), 8);
        assert_eq!(package.triangles.len(), 12);

        let direct = ExactBackend::new()
            .extrude_rectangle(RectangleExtrudeSpec {
                width_mm: width,
                depth_mm: depth,
                height_mm: height,
            })
            .unwrap();
        let direct_references = capture_guaranteed_references(
            &direct,
            &snapshot.document_id().0.to_string(),
            &EXTRUSION.0.to_string(),
        )
        .unwrap();
        let packages = BTreeMap::from([(DEFINITION, Arc::new(package.clone()))]);
        let projection = ExactInteractionProjection::from_snapshot(&snapshot, &packages);
        assert_eq!(projection.occurrence_count(), 1);

        for role in [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
        ] {
            let hit = projection
                .exact_pick(ray_for(role, width, depth, height))
                .unwrap_or_else(|| panic!("{case_id}: exact pick missed {role:?}"));
            assert_eq!(hit.target.body.role(), Some(role), "{case_id}");
            assert!(hit.target.body.has_valid_lineage(), "{case_id}");
            let direct_reference = direct_references
                .iter()
                .find(|reference| reference.semantic_role == role.semantic_role())
                .unwrap();
            assert_eq!(
                hit.target.body.document_id.0.to_string(),
                direct_reference.document_id,
                "{case_id}: document provenance differs for {role:?}"
            );
            assert_eq!(
                hit.target.body.producer_feature_id.0.to_string(),
                direct_reference.producer_feature_id,
                "{case_id}: producer provenance differs for {role:?}"
            );
            assert_eq!(
                hit.target.body.semantic_role,
                direct_reference.semantic_role
            );
            assert_eq!(
                hit.target.body.source_element_id,
                direct_reference.source_element_id
            );
            assert_eq!(
                hit.target.body.expected_type,
                direct_reference.expected_type
            );
            assert_eq!(direct_reference.stability_class, StabilityClass::Guaranteed);
            assert_eq!(
                hit.target.body.backend, direct_reference.backend_fingerprint,
                "{case_id}: backend provenance differs for {role:?}"
            );
            assert_eq!(
                hit.target.body.lineage_digest, direct_reference.lineage_digest,
                "{case_id}: canonical lineage differs for {role:?}"
            );
            let ReferenceResolution::Resolved { face_ordinal, .. } =
                resolve_subshape_reference(direct_reference, &direct)
            else {
                panic!("{case_id}: direct resolver did not resolve {role:?}");
            };
            let direct_face = direct
                .body
                .topology
                .faces
                .iter()
                .find(|face| face.ordinal == face_ordinal)
                .unwrap();
            assert_eq!(
                hit.target.body.corroborating_geometry_fingerprint,
                direct_face.geometric_fingerprint,
                "{case_id}: interaction and direct resolver disagree for {role:?}"
            );
            observed_roles += 1;
        }

        let before_revision = snapshot.revision_id();
        let before_digest = snapshot.canonical_digest();
        let before_undo = document.visible_undo_steps();
        for reference in package.references.clone() {
            document
                .register_exact_reference_evidence(reference)
                .unwrap();
        }
        assert_eq!(document.current().revision_id(), before_revision);
        assert_eq!(document.current().canonical_digest(), before_digest);
        assert_eq!(document.visible_undo_steps(), before_undo);

        let bytes = ketchup_core::persistence::save(&document.current());
        let reopened = match ketchup_core::persistence::load(&bytes).unwrap() {
            ketchup_core::persistence::LoadOutcome::Editable { document, .. } => document,
            ketchup_core::persistence::LoadOutcome::ReviewOnly(_) => {
                panic!("{case_id}: current exact reference evidence must open editable")
            }
        };
        assert_eq!(reopened.current().canonical_digest(), before_digest);
        let mut reopened_references = reopened
            .current()
            .exact_reference_evidence()
            .cloned()
            .collect::<Vec<_>>();
        let mut expected_references = package.references.clone();
        reopened_references.sort_by(|left, right| left.lineage_digest.cmp(&right.lineage_digest));
        expected_references.sort_by(|left, right| left.lineage_digest.cmp(&right.lineage_digest));
        assert_eq!(reopened_references, expected_references, "{case_id}");
        observed_cases += 1;
    }

    assert_eq!(observed_cases, 9);
    assert_eq!(observed_roles, 27);
}

fn rectangle_document(width: f64, depth: f64, height: f64) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "C1b rectangle".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Exact extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::new(height.to_string(), height).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(13),
                definition_id: DEFINITION,
                name: "C1b occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn ray_for(role: ExactFaceRole, width: f64, depth: f64, height: f64) -> Ray {
    let (origin, direction) = match role {
        ExactFaceRole::Top => (
            Vec3::new(width / 2.0, depth / 2.0, height + 10.0),
            Vec3::new(0.0, 0.0, -1.0),
        ),
        ExactFaceRole::Bottom => (
            Vec3::new(width / 2.0, depth / 2.0, -10.0),
            Vec3::new(0.0, 0.0, 1.0),
        ),
        ExactFaceRole::East => (
            Vec3::new(width + 10.0, depth / 2.0, height / 2.0),
            Vec3::new(-1.0, 0.0, 0.0),
        ),
    };
    Ray::new(origin, direction).unwrap()
}

fn worker_path() -> &'static str {
    env!("CARGO_BIN_EXE_ketchup-exact-worker")
}
