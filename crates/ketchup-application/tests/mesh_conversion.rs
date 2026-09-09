use std::time::{Duration, Instant};

use ketchup_application::evaluation::exact_worker_candidates;
use ketchup_application::mesh_conversion::{
    MeshConversionError, MeshConversionTaskEvent, commit_mesh_conversion, prepare_mesh_conversion,
    start_mesh_conversion, verify_mesh_conversion,
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

fn dense_cylinder(side_count: usize, ring_count: usize, radius: f64, height: f64) -> MeshBodySpec {
    let vertices_per_layer = side_count * ring_count + 1;
    let mut vertices_mm = Vec::with_capacity(vertices_per_layer * 2);
    for z in [0.0, height] {
        for ring in 0..ring_count {
            let ring_radius = radius * (ring_count - ring) as f64 / ring_count as f64;
            vertices_mm.extend(
                regular_polygon(side_count, ring_radius)
                    .into_iter()
                    .map(|point| [point[0], point[1], z]),
            );
        }
        vertices_mm.push([0.0, 0.0, z]);
    }
    let high_offset = vertices_per_layer as u32;
    let mut triangles = Vec::new();
    for ring in 0..ring_count - 1 {
        let outer = ring * side_count;
        let inner = (ring + 1) * side_count;
        for index in 0..side_count {
            let next = (index + 1) % side_count;
            let [outer_a, outer_b, inner_a, inner_b] = [
                (outer + index) as u32,
                (outer + next) as u32,
                (inner + index) as u32,
                (inner + next) as u32,
            ];
            triangles.extend([
                [outer_a, inner_b, outer_b],
                [outer_a, inner_a, inner_b],
                [
                    high_offset + outer_a,
                    high_offset + outer_b,
                    high_offset + inner_b,
                ],
                [
                    high_offset + outer_a,
                    high_offset + inner_b,
                    high_offset + inner_a,
                ],
            ]);
        }
    }
    let inner = (ring_count - 1) * side_count;
    let center = (vertices_per_layer - 1) as u32;
    for index in 0..side_count {
        let next = (index + 1) % side_count;
        let [a, b] = [(inner + index) as u32, (inner + next) as u32];
        triangles.extend([
            [center, b, a],
            [high_offset + center, high_offset + a, high_offset + b],
            [index as u32, next as u32, high_offset + next as u32],
            [
                index as u32,
                high_offset + next as u32,
                high_offset + index as u32,
            ],
        ]);
    }
    MeshBodySpec {
        schema: MESH_BODY_SCHEMA_V1.to_owned(),
        vertices_mm,
        triangles,
        authority: MeshAuthority::Authored {
            provenance: "dense-mesh-conversion-test".to_owned(),
        },
    }
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
    let mut invalid_index = package.clone();
    invalid_index.triangles[0].vertex_indices[0] = u32::MAX;
    assert!(matches!(
        verify_mesh_conversion(&plan, invalid_index),
        Err(MeshConversionError::ExactVerificationMismatch)
    ));
    let mut non_finite = package.clone();
    non_finite.vertices[0].position_mm[0] = f64::NAN;
    assert!(matches!(
        verify_mesh_conversion(&plan, non_finite),
        Err(MeshConversionError::ExactVerificationMismatch)
    ));
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
fn large_cylinder_mesh_verifies_beyond_the_former_pairwise_limit() {
    let mesh = dense_cylinder(64, 126, 10.0, 25.0);
    let source_triangle_count = mesh.triangles.len();
    let document = document(mesh);
    let plan = prepare_mesh_conversion(&document, MESH, 0.2).unwrap();
    let exact = worker().evaluate_exact_brep_graph(plan.graph()).unwrap();
    assert!(source_triangle_count * exact.triangles.len() > 4_000_000);

    let verification = verify_mesh_conversion(&plan, exact).unwrap();

    assert!(verification.max_source_to_exact_mm() <= plan.tolerance_mm());
    assert!(verification.max_exact_to_source_mm() <= plan.tolerance_mm());
    assert_eq!(document.visible_undo_steps(), 1);
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

    let mut other_document = self::document(prism(
        &[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
        8.0,
    ));
    let other_plan = prepare_mesh_conversion(&other_document, MESH, 1.0e-6).unwrap();
    assert_eq!(plan.graph().graph_digest, other_plan.graph().graph_digest);
    assert_ne!(
        document.current().document_id(),
        other_document.current().document_id()
    );
    let other_before = (
        other_document.current().canonical_digest(),
        other_document.visible_undo_steps(),
    );
    assert!(matches!(
        commit_mesh_conversion(&mut other_document, &other_plan, verified.clone()),
        Err(MeshConversionError::Stale)
    ));
    assert_eq!(
        (
            other_document.current().canonical_digest(),
            other_document.visible_undo_steps(),
        ),
        other_before
    );

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

#[test]
fn background_task_reports_monotonic_progress_without_mutating_source() {
    let document = document(prism(
        &[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
        8.0,
    ));
    let before = document.current().canonical_digest();
    let task = start_mesh_conversion(
        &document,
        MESH,
        1.0e-6,
        exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .expect("build ketchup-exact-worker before this test"),
        Duration::from_secs(5),
        || {},
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut completed = Vec::new();
    loop {
        match task.poll() {
            Ok(MeshConversionTaskEvent::Progress(progress)) => completed.push(progress.completed),
            Ok(MeshConversionTaskEvent::Finished(result)) => {
                (*result).unwrap();
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => panic!("background conversion did not finish"),
        }
    }
    assert_eq!(completed, vec![0, 1, 2]);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 1);
}

#[test]
fn background_wait_does_not_join_a_blocked_completion_callback() {
    let document = document(prism(
        &[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
        8.0,
    ));
    let (release_sender, release_receiver) = std::sync::mpsc::channel::<()>();
    let (started_sender, started_receiver) = std::sync::mpsc::channel();
    let task = start_mesh_conversion(
        &document,
        MESH,
        1.0e-6,
        exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .expect("build ketchup-exact-worker before this test"),
        Duration::from_secs(5),
        move || {
            let _ = started_sender.send(());
            let _ = release_receiver.recv();
        },
    )
    .unwrap();
    let (result_sender, result_receiver) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let _ = result_sender.send(task.wait(Duration::from_secs(5)).is_ok());
    });

    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let completed_promptly = result_receiver.recv_timeout(Duration::from_millis(500));
    drop(release_sender);
    waiter.join().unwrap();

    assert_eq!(completed_promptly, Ok(true));
}

#[test]
fn background_task_commits_on_the_monotonic_revision_after_undo_branch() {
    let mut document = document(prism(
        &[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
        8.0,
    ));
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OCCURRENCE,
                transform: Transform::from_translation(1.0, 0.0, 0.0).unwrap(),
            },
        ]))
        .unwrap();
    document.undo().unwrap();
    let source_digest = document.current().canonical_digest();
    let next_revision_id = document.next_revision_id();
    assert!(next_revision_id > document.current().revision_id() + 1);

    let task = start_mesh_conversion(
        &document,
        MESH,
        1.0e-6,
        exact_worker_candidates()
            .into_iter()
            .find(|path| path.is_file())
            .expect("build ketchup-exact-worker before this test"),
        Duration::from_secs(5),
        || {},
    )
    .unwrap();
    let prepared = task.wait(Duration::from_secs(5)).unwrap();
    assert_eq!(document.current().canonical_digest(), source_digest);

    let package =
        commit_mesh_conversion(&mut document, &prepared.plan, prepared.verification).unwrap();
    assert_eq!(document.current().revision_id(), next_revision_id);
    assert!(package.is_current(&document.current()));
    assert_eq!(document.visible_redo_steps(), 0);
}

#[test]
fn background_task_cancel_and_timeout_fail_closed() {
    for (timeout, cancel, expected) in [
        (Duration::from_secs(5), true, MeshConversionError::Cancelled),
        (Duration::ZERO, false, MeshConversionError::TimedOut),
    ] {
        let document = document(prism(
            &[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]],
            8.0,
        ));
        let before = document.current().canonical_digest();
        let task = start_mesh_conversion(
            &document,
            MESH,
            1.0e-6,
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
                .expect("build ketchup-exact-worker before this test"),
            timeout,
            || {},
        )
        .unwrap();
        if cancel {
            task.cancel();
        }
        assert!(matches!(task.wait(Duration::from_secs(5)), Err(error) if error == expected));
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), 1);
    }
}
