use ketchup_application::evaluation::exact_worker_candidates;
use ketchup_application::mesh_conversion::{
    MeshConversionError, commit_mesh_conversion, prepare_mesh_conversion, verify_mesh_conversion,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureId, FeatureKind,
    MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec, OccurrenceId, Transform,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::persistence;
use ketchup_scheduler::ExactWorkerSupervisor;

const DEFINITION: DefinitionId = DefinitionId(1);
const MESH: FeatureId = FeatureId(2);
const OCCURRENCE: OccurrenceId = OccurrenceId(3);

fn prism(profile: &[[f64; 2]], height: f64) -> MeshBodySpec {
    let count = profile.len();
    let mut vertices_mm = profile
        .iter()
        .map(|point| [point[0], point[1], 0.0])
        .collect::<Vec<_>>();
    vertices_mm.extend(profile.iter().map(|point| [point[0], point[1], height]));
    let mut triangles = Vec::new();
    for index in 1..count - 1 {
        triangles.push([0, (index + 1) as u32, index as u32]);
        triangles.push([
            count as u32,
            (count + index) as u32,
            (count + index + 1) as u32,
        ]);
    }
    for index in 0..count {
        let next = (index + 1) % count;
        triangles.push([index as u32, next as u32, (count + next) as u32]);
        triangles.push([index as u32, (count + next) as u32, (count + index) as u32]);
    }
    MeshBodySpec {
        schema: MESH_BODY_SCHEMA_V1.to_owned(),
        vertices_mm,
        triangles,
        authority: MeshAuthority::Authored {
            provenance: "mesh-conversion-test".to_owned(),
        },
    }
}

fn document(mesh: MeshBodySpec) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Imported mesh".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: MESH,
                definition_id: DEFINITION,
                name: "Mesh".to_owned(),
                kind: FeatureKind::MeshBody(mesh),
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Imported object".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document
}

fn worker() -> ExactWorkerSupervisor {
    let executable = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    ExactWorkerSupervisor::spawn(executable).unwrap()
}

fn regular_polygon(count: usize, radius: f64) -> Vec<[f64; 2]> {
    (0..count)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / count as f64;
            [radius * angle.cos(), radius * angle.sin()]
        })
        .collect()
}

#[test]
fn verified_box_conversion_is_atomic_undoable_and_persistent() {
    let mut document = document(prism(
        &[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
        8.0,
    ));
    let before = document.current().canonical_digest();
    let before_revision = document.current().revision_id();
    let before_undo = document.visible_undo_steps();

    let plan = prepare_mesh_conversion(&document, MESH, 1.0e-6).unwrap();
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.current().revision_id(), before_revision);
    assert_eq!(document.visible_undo_steps(), before_undo);

    let package = worker().evaluate_exact_brep_graph(plan.graph()).unwrap();
    let verification = verify_mesh_conversion(&plan, package).unwrap();
    assert!(verification.max_source_to_exact_mm() <= plan.tolerance_mm());
    assert!(verification.max_exact_to_source_mm() <= plan.tolerance_mm());
    let package = commit_mesh_conversion(&mut document, &plan, verification).unwrap();

    let converted = document.current();
    assert_eq!(document.visible_undo_steps(), before_undo + 1);
    assert!(matches!(
        converted.feature(MESH).unwrap().kind(),
        FeatureKind::Profile { .. }
    ));
    assert!(
        converted
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::Extrusion { .. }))
    );
    assert!(
        converted
            .features()
            .any(|feature| matches!(feature.kind(), FeatureKind::RigidTransform { .. }))
    );
    assert!(package.is_current(&converted));
    let converted_digest = converted.canonical_digest();

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert!(matches!(
        document.current().feature(MESH).unwrap().kind(),
        FeatureKind::MeshBody(_)
    ));
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        converted_digest
    );

    let encoded = persistence::save(&document.current());
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), converted_digest);
    let reopened_graph = ExactBRepGraph::from_snapshot(
        &reopened.snapshot(),
        DEFINITION,
        package.identity.producer_feature_id,
    )
    .unwrap();
    assert_eq!(reopened_graph.graph_digest, package.graph.graph_digest);
}

#[test]
fn cylinder_and_general_extrusion_require_real_exact_agreement() {
    let cases = [
        (prism(&regular_polygon(16, 10.0), 25.0), 0.3),
        (
            prism(
                &[
                    [-3.0, -2.0],
                    [2.0, -2.0],
                    [4.0, 0.5],
                    [1.0, 3.0],
                    [-2.0, 2.0],
                ],
                7.0,
            ),
            1.0e-6,
        ),
    ];
    let mut worker = worker();
    for (mesh, tolerance) in cases {
        let mut document = document(mesh);
        let plan = prepare_mesh_conversion(&document, MESH, tolerance).unwrap();
        let exact = worker.evaluate_exact_brep_graph(plan.graph()).unwrap();
        let verified = verify_mesh_conversion(&plan, exact).unwrap();
        commit_mesh_conversion(&mut document, &plan, verified).unwrap();
        assert!(
            document
                .current()
                .features()
                .all(|feature| !matches!(feature.kind(), FeatureKind::MeshBody(_)))
        );
    }
}

#[test]
fn ambiguous_and_stale_plans_never_mutate() {
    let ambiguous = document(prism(&regular_polygon(6, 2.0), 3.0));
    let before = ambiguous.current().canonical_digest();
    assert!(matches!(
        prepare_mesh_conversion(&ambiguous, MESH, 0.3),
        Err(MeshConversionError::Ambiguous(_))
    ));
    assert_eq!(ambiguous.current().canonical_digest(), before);

    let mut document = document(prism(
        &[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
        8.0,
    ));
    let plan = prepare_mesh_conversion(&document, MESH, 1.0e-6).unwrap();
    let exact = worker().evaluate_exact_brep_graph(plan.graph()).unwrap();
    let verified = verify_mesh_conversion(&plan, exact).unwrap();
    let original = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OCCURRENCE,
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), original);
    let state = (
        document.current().revision_id(),
        document.current().canonical_digest(),
        document.visible_undo_steps(),
        document.visible_redo_steps(),
    );
    assert_eq!(
        commit_mesh_conversion(&mut document, &plan, verified),
        Err(MeshConversionError::Stale)
    );
    assert_eq!(
        (
            document.current().revision_id(),
            document.current().canonical_digest(),
            document.visible_undo_steps(),
            document.visible_redo_steps(),
        ),
        state
    );
}
