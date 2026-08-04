use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    OccurrenceId, Transform,
};
use ketchup_core::exact_product::{
    EXACT_THROUGH_CUT_EVALUATOR_V1, ExactFaceRole, ExactRectangleRequest,
};
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
const CUT_PROFILE: FeatureId = FeatureId(14);
const THROUGH_CUT: FeatureId = FeatureId(15);

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

#[test]
fn scheduler_evaluates_bounded_through_cut_with_seven_role_evidences() {
    let mut supervisor = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let mut document = through_cut_document(100.0, 60.0, 18.0, [30.0, 20.0, 20.0, 15.0]);
    let snapshot = document.current();
    let request = ExactRectangleRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
    let cut = request.through_cut.as_ref().unwrap();
    assert_eq!(cut.feature_id, THROUGH_CUT);
    assert_eq!(cut.profile_feature_id, CUT_PROFILE);
    assert_eq!(request.producer_feature_id(), THROUGH_CUT);
    assert_eq!(request.evaluator(), EXACT_THROUGH_CUT_EVALUATOR_V1);

    let package = supervisor.evaluate_rectangle(&request).unwrap();
    assert!(package.is_current(&snapshot));
    assert_eq!(package.identity.producer_feature_id, THROUGH_CUT);
    assert_eq!(package.identity.evaluator, EXACT_THROUGH_CUT_EVALUATOR_V1);
    assert_eq!(package.bounds_mm, [[0.0, 0.0, 0.0], [100.0, 60.0, 18.0]]);
    assert_eq!(package.vertices.len(), 16);
    assert_eq!(package.triangles.len(), 32);
    let mut edge_use = BTreeMap::<(u32, u32), usize>::new();
    let mut signed_volume_mm3 = 0.0;
    for triangle in &package.triangles {
        let [a, b, c] = triangle
            .vertex_indices
            .map(|index| package.vertices[index as usize].position_mm);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        assert!(cross.into_iter().map(|value| value * value).sum::<f64>() > 1.0e-12);
        signed_volume_mm3 += (a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
        for [first, second] in [
            [triangle.vertex_indices[0], triangle.vertex_indices[1]],
            [triangle.vertex_indices[1], triangle.vertex_indices[2]],
            [triangle.vertex_indices[2], triangle.vertex_indices[0]],
        ] {
            *edge_use
                .entry((first.min(second), first.max(second)))
                .or_default() += 1;
        }
    }
    assert!(edge_use.values().all(|count| *count == 2));
    assert!((signed_volume_mm3 - 102_600.0).abs() < 1.0e-6);
    assert_eq!(package.references.len(), 7);

    let packages = BTreeMap::from([(DEFINITION, Arc::new(package.clone()))]);
    let projection = ExactInteractionProjection::from_snapshot(&snapshot, &packages);
    let hole_ray = Ray::new(Vec3::new(40.0, 27.5, 30.0), Vec3::new(0.0, 0.0, -1.0)).unwrap();
    assert!(projection.exact_pick(hole_ray).is_none());
    let cut_wall_ray = Ray::new(Vec3::new(40.0, 27.5, 9.0), Vec3::new(-1.0, 0.0, 0.0)).unwrap();
    assert_eq!(
        projection
            .exact_pick(cut_wall_ray)
            .and_then(|hit| hit.target.body.role()),
        Some(ExactFaceRole::CutWest)
    );

    for role in [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
        ExactFaceRole::CutWest,
        ExactFaceRole::CutEast,
        ExactFaceRole::CutSouth,
        ExactFaceRole::CutNorth,
    ] {
        let matching = package
            .references
            .iter()
            .filter(|reference| reference.role() == Some(role))
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "expected one durable {role:?} evidence");
        assert_eq!(matching[0].producer_feature_id, THROUGH_CUT);
        assert_eq!(
            matching[0].profile_feature_id,
            match role {
                ExactFaceRole::Top | ExactFaceRole::Bottom | ExactFaceRole::East => PROFILE,
                ExactFaceRole::CutWest
                | ExactFaceRole::CutEast
                | ExactFaceRole::CutSouth
                | ExactFaceRole::CutNorth => CUT_PROFILE,
            }
        );
        assert!(matching[0].has_valid_lineage());
    }

    for reference in package.references.clone() {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }
    let loaded =
        ketchup_core::persistence::load(&ketchup_core::persistence::save(&document.current()))
            .unwrap();
    assert_eq!(loaded.source_schema(), 5);
    let reopened = match loaded {
        ketchup_core::persistence::LoadOutcome::Editable { document, .. } => document,
        ketchup_core::persistence::LoadOutcome::ReviewOnly(_) => {
            panic!("schema-5 through-cut evidence must reopen editable")
        }
    };
    assert_eq!(
        reopened.current().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(reopened.current().exact_reference_evidence().count(), 7);
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

fn through_cut_document(width: f64, depth: f64, height: f64, cut: [f64; 4]) -> DocumentStore {
    let mut document = rectangle_document(width, depth, height);
    let [x, y, cut_width, cut_depth] = cut;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: CUT_PROFILE,
                definition_id: DEFINITION,
                name: "Through-cut profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [x, y],
                        [x + cut_width, y],
                        [x + cut_width, y + cut_depth],
                        [x, y + cut_depth],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: THROUGH_CUT,
                definition_id: DEFINITION,
                name: "Bounded through-cut".to_owned(),
                kind: FeatureKind::ThroughCut {
                    target: EXTRUSION,
                    profile: CUT_PROFILE,
                },
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
        ExactFaceRole::CutWest
        | ExactFaceRole::CutEast
        | ExactFaceRole::CutSouth
        | ExactFaceRole::CutNorth => panic!("cut roles are outside the extrusion-only C1b corpus"),
    };
    Ray::new(origin, direction).unwrap()
}

fn worker_path() -> &'static str {
    env!("CARGO_BIN_EXE_ketchup-exact-worker")
}
