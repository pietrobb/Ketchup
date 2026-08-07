use ketchup_core::beam_m4ae::BeamWorkspace;
use ketchup_core::beam_m5::{BeamNotchFaceRole, HalfLapParticipant};
use ketchup_core::document::Transform;
use ketchup_core::exact_product::{ExactBodyView, ExactProductError, ExactResultRegistry};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn worker_backed_half_laps_produce_durable_refs_drawing_and_export() {
    let mut workspace = BeamWorkspace::load().unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_ketchup-exact-worker"));
    let mut worker = ExactWorkerSupervisor::spawn(executable).unwrap();

    let requests = workspace.m5_requests().unwrap();
    assert_eq!(requests.len(), 13);
    let packages = requests
        .iter()
        .map(|request| worker.evaluate_beam_piece(request).unwrap())
        .collect::<Vec<_>>();
    let products = workspace.accept_m5_packages(packages).unwrap();
    let snapshot = workspace.snapshot();
    let registry =
        ExactResultRegistry::accept_beam(&snapshot, products.packages.values().cloned()).unwrap();

    assert_eq!(products.packages.len(), 13);
    assert_eq!(registry.beam_len(), 13);
    assert!(products.packages.iter().all(|(piece, package)| {
        registry
            .get_beam(piece)
            .is_some_and(|registered| Arc::ptr_eq(registered, package))
    }));
    let first_package = products.packages.values().next().unwrap();
    assert!(Arc::ptr_eq(
        registry
            .get_beam_result(&first_package.result_key())
            .unwrap(),
        first_package
    ));
    let mesh = first_package.mesh_export(Transform::identity());
    assert!(mesh.mesh_obj.starts_with("# Ketchup exact body OBJ\n"));
    assert!(mesh.mesh_obj.contains("\ng unreferenced\n"));
    assert!(mesh.loss_report.contains("producer_piece_key="));
    let mut invalid = first_package.as_ref().clone();
    invalid.references[0].backend = "tampered".to_owned();
    assert!(matches!(
        ExactResultRegistry::accept_beam(&snapshot, [Arc::new(invalid)]),
        Err(ExactProductError::InvalidWorkerEvidence)
    ));
    let mut duplicate_registry = ExactResultRegistry::default();
    duplicate_registry
        .insert_current_beam(&snapshot, Arc::clone(first_package))
        .unwrap();
    assert!(matches!(
        duplicate_registry.insert_current_beam(&snapshot, Arc::clone(first_package)),
        Err(ExactProductError::DuplicateDerivedResult { .. })
    ));
    let mut conflicting = first_package.as_ref().clone();
    conflicting
        .identity
        .result_fingerprint
        .push_str("-alternate");
    for reference in &mut conflicting.references {
        reference.result_fingerprint = conflicting.identity.result_fingerprint.clone();
    }
    assert!(conflicting.has_valid_registry_evidence());
    assert!(matches!(
        duplicate_registry.insert_current_beam(&snapshot, Arc::new(conflicting)),
        Err(ExactProductError::DuplicateDerivedResult { .. })
    ));
    assert_eq!(products.stable_reference_count(), 48);
    assert_eq!(products.manufacturing.operations.len(), 24);
    assert!(products.packages.values().all(|package| {
        package.triangles.iter().all(|triangle| {
            let points = triangle
                .vertex_indices
                .map(|index| package.vertices[index as usize].position_mm);
            (0..3).any(|axis| {
                points[0][axis] == points[1][axis] && points[1][axis] == points[2][axis]
            })
        })
    }));
    assert_eq!(
        products
            .manufacturing
            .operations
            .iter()
            .filter(|operation| operation.participant == HalfLapParticipant::A)
            .count(),
        12
    );
    assert!(products.packages.values().all(|package| {
        package
            .references
            .iter()
            .all(|reference| reference.has_valid_lineage())
    }));
    assert!(products.packages.values().any(|package| {
        package.references.iter().any(|reference| {
            reference.participant == HalfLapParticipant::A
                && reference.role == BeamNotchFaceRole::WestWall
        })
    }));
    let svg = String::from_utf8(products.drawing_svg().unwrap()).unwrap();
    assert!(svg.contains("beam-a/longitudinal-outline"));
    let manufacturing = String::from_utf8(products.manufacturing_export().unwrap()).unwrap();
    assert!(manufacturing.starts_with("ketchup.beam-manufacturing-export.v1\n"));
    assert_eq!(manufacturing.matches("operation=").count(), 24);
    let mut quarantined = products.clone();
    quarantined.manufacturing.operations[0]
        .contact_face
        .lineage_digest = "quarantined".to_owned();
    assert!(quarantined.manufacturing_export().is_err());

    let stale_packages = products
        .packages
        .values()
        .map(|package| package.as_ref().clone())
        .collect::<Vec<_>>();
    let old_lineages = products
        .packages
        .values()
        .flat_map(|package| package.references.iter())
        .map(|reference| {
            (
                reference.joint_id,
                reference.participant,
                reference.role,
                reference.lineage_digest.clone(),
            )
        })
        .collect::<Vec<_>>();
    workspace.set_zone1_gap_mm(420.0).unwrap();
    assert!(workspace.accept_m5_packages(stale_packages).is_err());
    assert!(matches!(
        ExactResultRegistry::accept_beam(
            &workspace.snapshot(),
            products.packages.values().cloned()
        ),
        Err(ExactProductError::StaleResult)
    ));
    let changed_requests = workspace.m5_requests().unwrap();
    let changed_packages = changed_requests
        .iter()
        .map(|request| worker.evaluate_beam_piece(request).unwrap())
        .collect::<Vec<_>>();
    let changed = workspace.accept_m5_packages(changed_packages).unwrap();
    let changed_lineages = changed
        .packages
        .values()
        .flat_map(|package| package.references.iter())
        .map(|reference| {
            (
                reference.joint_id,
                reference.participant,
                reference.role,
                reference.lineage_digest.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(old_lineages, changed_lineages);
    assert_eq!(
        changed.manufacturing.envelope.source_revision,
        workspace.slice().revision_id
    );
}
