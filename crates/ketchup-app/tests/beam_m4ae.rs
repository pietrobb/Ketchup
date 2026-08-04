mod harness;
use harness::Shell;
use ketchup_core::beam_m4ae::{BEAM_RULE_NODE, BeamValidationVerdict, ZONE1_GAP_NODE};
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
