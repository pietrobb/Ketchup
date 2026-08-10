mod harness;
use harness::Shell;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::beam_m4ae::{BEAM_RULE_NODE, BeamValidationVerdict, ZONE1_GAP_NODE};
use ketchup_core::document::HighRiskClass;
use ketchup_core::graph::sha256_hex;
use ketchup_core::validation::{PRISMATIC_VALIDATOR_CONTRACT_V1, ValidationState};

#[test]
fn visible_beam_panel_loads_and_refreshes_canonical_slice() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().load_beam_m4ae());
    shell.settle();
    let baseline = shell.app().beam_groove_positions().unwrap();
    assert_eq!(baseline.len(), 12);
    assert_eq!((baseline[0].start_mm, baseline[11].end_mm), (210.0, 6660.0));
    let identities = shell
        .app()
        .beam_slice()
        .unwrap()
        .pieces
        .iter()
        .map(|p| p.identity.clone())
        .collect::<Vec<_>>();
    let bom_rev = shell.app().beam_bom().unwrap().generated_for_revision;
    assert_eq!(shell.app().beam_full_bom().unwrap().rows.len(), 2);
    assert_eq!(
        shell.app().beam_full_bom().unwrap().rows[0].material_key,
        "ketchup.material.timber.unspecified.v1"
    );
    assert_eq!(
        shell.app().beam_dimension_sheet().unwrap().chains[0].grouped_labels,
        ["415 × 6", "408 × 5", "400"]
    );
    assert_eq!(
        shell.app().beam_validation_report().unwrap().state,
        ValidationState::Passed
    );
    assert_eq!(
        shell
            .app()
            .beam_validation_report()
            .unwrap()
            .invocation
            .contract_id,
        PRISMATIC_VALIDATOR_CONTRACT_V1
    );
    assert!(shell.app_mut().set_beam_zone1_gap_mm(420.0));
    shell.settle();
    assert_ne!(
        shell.app().beam_groove_positions().unwrap()[1].start_mm,
        785.0
    );
    assert_ne!(
        shell.app().beam_bom().unwrap().generated_for_revision,
        bom_rev
    );
    assert!(shell.app().beam_validation_is_green());
    assert_eq!(
        shell.app().beam_dimension_sheet().unwrap().chains[0].grouped_labels,
        ["420 × 6", "408 × 5", "400"]
    );
    assert!(shell.app().beam_last_change().unwrap().bom_regenerated);
    assert!(
        shell
            .app()
            .beam_last_change()
            .unwrap()
            .dimensions_regenerated
    );
    assert!(shell.app().beam_last_change().unwrap().validator_ran);
    assert_eq!(
        shell.app().beam_slice().unwrap().validation,
        BeamValidationVerdict::Green
    );
    assert_eq!(
        shell
            .app()
            .beam_slice()
            .unwrap()
            .pieces
            .iter()
            .map(|p| p.identity.clone())
            .collect::<Vec<_>>(),
        identities
    );
    assert_eq!(
        shell
            .app()
            .beam_last_change()
            .unwrap()
            .recomputed_nodes
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![ZONE1_GAP_NODE, BEAM_RULE_NODE]
    );
}

#[test]
fn running_app_accepts_worker_notches_and_exports_current_piece_outputs() {
    let executable_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let current = std::env::current_exe().unwrap();
    let executable = current
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(executable_name);
    assert!(
        executable.is_file(),
        "build the workspace all-targets so the exact worker is present at {}",
        executable.display()
    );

    let dialogs = ScriptedFileDialogs::new()
        .queue_refused_high_risk()
        .queue_high_risk_approval(41)
        .queue_refused_high_risk()
        .queue_high_risk_approval(42);
    let mut shell = Shell::with_dialogs(dialogs.clone());
    shell.app_mut().connect_exact_worker(&executable).unwrap();
    assert!(shell.app_mut().load_beam_m4ae());
    for _ in 0..100 {
        shell.settle();
        if shell.app().beam_m5_products().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let products = shell
        .app()
        .beam_m5_products()
        .expect("the running app must accept current M5 worker products");
    assert_eq!(products.packages.len(), 13);
    assert_eq!(shell.app().beam_exact_body_count(), 13);
    assert_eq!(products.stable_reference_count(), 48);
    assert_eq!(products.manufacturing.operations.len(), 24);
    let first_piece = products.packages.keys().next().unwrap().clone();
    let lineages = products
        .packages
        .values()
        .flat_map(|package| package.references.iter())
        .map(|reference| reference.lineage_digest.clone())
        .collect::<Vec<_>>();

    let directory = tempfile::tempdir().unwrap();
    let drawing_path = directory.path().join("beam-a.svg");
    let manufacturing_path = directory.path().join("beam-a.kfm");
    let mesh_path = directory.path().join("beam-piece.obj");
    let loss_path = mesh_path.with_extension("obj.loss.txt");
    let original_manufacturing = b"existing manufacturing export must survive refusal";
    let original_mesh = b"existing mesh must survive refusal";
    let original_loss = b"existing loss report must survive refusal";
    std::fs::write(&manufacturing_path, original_manufacturing).unwrap();
    std::fs::write(&mesh_path, original_mesh).unwrap();
    std::fs::write(&loss_path, original_loss).unwrap();
    let active_digest = shell.app().canonical_digest();
    let active_revision = shell.app().document_revision();
    let active_can_undo = shell.app().can_undo();
    let beam_revision = shell.app().beam_slice().unwrap().revision_id;
    assert!(shell.app_mut().export_beam_drawing_to(&drawing_path));
    assert!(
        !shell
            .app_mut()
            .export_beam_manufacturing_to(&manufacturing_path)
    );
    assert_eq!(
        std::fs::read(&manufacturing_path).unwrap(),
        original_manufacturing
    );
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert!(
        shell
            .app_mut()
            .export_beam_manufacturing_to(&manufacturing_path)
    );
    let manufacturing_receipt = shell
        .app()
        .last_side_effect_receipt()
        .expect("approved manufacturing export returns an authorization receipt");
    assert_eq!(manufacturing_receipt.approving_human(), 41);
    assert_eq!(manufacturing_receipt.revision_id(), beam_revision);
    assert_eq!(
        manufacturing_receipt.operation(),
        "release-manufacturing-export-with-warnings"
    );
    assert_eq!(
        manufacturing_receipt.scope().class(),
        HighRiskClass::ReleaseManufacturingExportWithWarnings
    );
    assert_eq!(
        manufacturing_receipt.scope().path(),
        Some(manufacturing_path.display().to_string().as_str())
    );
    assert_eq!(
        manufacturing_receipt.payload_digest(),
        sha256_hex(&std::fs::read(&manufacturing_path).unwrap())
    );
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert_eq!(shell.app().document_revision(), active_revision);
    assert_eq!(shell.app().can_undo(), active_can_undo);
    assert_eq!(shell.app().beam_slice().unwrap().revision_id, beam_revision);
    assert!(
        !shell
            .app_mut()
            .export_beam_piece_mesh_to(&first_piece, &mesh_path)
    );
    assert_eq!(std::fs::read(&mesh_path).unwrap(), original_mesh);
    assert_eq!(std::fs::read(&loss_path).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert_eq!(shell.app().document_revision(), active_revision);
    assert_eq!(shell.app().can_undo(), active_can_undo);
    assert_eq!(shell.app().beam_slice().unwrap().revision_id, beam_revision);

    assert!(
        shell
            .app_mut()
            .export_beam_piece_mesh_to(&first_piece, &mesh_path)
    );
    let receipt = shell
        .app()
        .last_side_effect_receipt()
        .expect("approved Beam lossy export returns an authorization receipt");
    assert_eq!(receipt.approving_human(), 42);
    assert_eq!(receipt.revision_id(), beam_revision);
    assert_eq!(receipt.operation(), "export-lossy-obj-with-loss-report");
    assert_eq!(receipt.scope().class(), HighRiskClass::LossyConversion);
    assert_eq!(
        receipt.scope().path(),
        Some(mesh_path.display().to_string().as_str())
    );
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert_eq!(shell.app().document_revision(), active_revision);
    assert_eq!(shell.app().can_undo(), active_can_undo);
    assert_eq!(shell.app().beam_slice().unwrap().revision_id, beam_revision);
    assert_eq!(dialogs.high_risk_prompts().len(), 4);
    assert!(dialogs.high_risk_prompts()[0].contains("Payload SHA-256:"));
    assert!(
        dialogs.high_risk_prompts()[0].contains("manufacturing export with unresolved warnings")
    );
    assert!(dialogs.high_risk_prompts()[2].contains("lossy exact-to-mesh conversion"));
    let drawing = std::fs::read_to_string(drawing_path).unwrap();
    let manufacturing = std::fs::read_to_string(&manufacturing_path).unwrap();
    assert!(drawing.contains("ketchup.beam-drawing-svg.v1"));
    assert!(drawing.contains("beam-a/longitudinal-outline"));
    assert!(manufacturing.starts_with("ketchup.beam-manufacturing-export.v1\n"));
    assert_eq!(manufacturing.matches("operation=").count(), 24);
    let mesh = std::fs::read_to_string(&mesh_path).unwrap();
    let loss = std::fs::read_to_string(mesh_path.with_extension("obj.loss.txt")).unwrap();
    assert!(mesh.starts_with("# Ketchup exact body OBJ\n"));
    assert!(loss.contains("producer_piece_key="));

    assert!(shell.app_mut().set_beam_zone1_gap_mm(420.0));
    assert!(shell.app().beam_m5_products().is_none());
    assert_eq!(shell.app().beam_exact_body_count(), 0);
    assert!(
        !shell
            .app_mut()
            .export_beam_manufacturing_to(&manufacturing_path)
    );
    assert!(
        !shell
            .app_mut()
            .export_beam_piece_mesh_to(&first_piece, &mesh_path)
    );
    for _ in 0..100 {
        shell.settle();
        if shell.app().beam_m5_products().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let changed = shell
        .app()
        .beam_m5_products()
        .expect("the app must accept recomputed M5 products");
    assert_eq!(shell.app().beam_exact_body_count(), 13);
    assert_eq!(changed.stable_reference_count(), 48);
    assert_eq!(
        changed
            .packages
            .values()
            .flat_map(|package| package.references.iter())
            .map(|reference| reference.lineage_digest.clone())
            .collect::<Vec<_>>(),
        lineages
    );
}
