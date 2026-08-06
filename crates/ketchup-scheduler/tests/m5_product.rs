use ketchup_core::beam_m4ae::BeamWorkspace;
use ketchup_core::beam_m5::{BeamNotchFaceRole, HalfLapParticipant};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::path::PathBuf;

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

    assert_eq!(products.packages.len(), 13);
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

    let stale_packages = products.packages.values().cloned().collect::<Vec<_>>();
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
