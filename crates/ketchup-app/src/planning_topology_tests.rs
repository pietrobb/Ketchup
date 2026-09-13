use super::*;

fn finishes(app: &KetchupApp) -> Vec<AssistantCadBodyFeature> {
    let snapshot = app.document.current();
    let reference = |kind| {
        ketchup_application::topology::assistant_topology_references(
            &snapshot,
            &app.topology_results,
            kind,
        )
        .into_iter()
        .find(|reference| reference.producer_feature_id == FeatureId(2))
        .unwrap()
        .lineage_digest
        .clone()
    };
    vec![
        AssistantCadBodyFeature::TopologyShell {
            target_feature_id: 2,
            removed_face_reference_ids: vec![reference(TopologicalElementKind::Face)],
            thickness_mm: 1.0,
            direction: ketchup_core::assistant_sidecar::AssistantCadShellDirection::Inward,
        },
        AssistantCadBodyFeature::TopologyFillet {
            target_feature_id: 2,
            edge_reference_ids: vec![reference(TopologicalElementKind::Edge)],
            radius_mm: 1.0,
            radius_stations: Vec::new(),
        },
        AssistantCadBodyFeature::TopologyChamfer {
            target_feature_id: 2,
            edge_reference_ids: vec![reference(TopologicalElementKind::Edge)],
            distance_mm: 1.0,
            mode: Default::default(),
            side_face_reference_ids: Vec::new(),
        },
    ]
}

fn prefix() -> AssistantCadEditOperation {
    AssistantCadEditOperation::SetColor {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![1],
        },
        color: Some([31, 47, 59]),
    }
}

fn program(feature: AssistantCadBodyFeature) -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![
            prefix(),
            AssistantCadEditOperation::AppendFeature {
                definition_id: INITIAL_BOX_DEFINITION.0,
                name: "Finish after prefix".into(),
                feature,
            },
        ],
    }
}

#[test]
fn prefix_preserves_current_topology_for_all_finishes_and_one_undo_step() {
    for finish_index in 0..3 {
        let mut app = KetchupApp::new();
        install_initial_graph_result(&mut app);
        let before = app.live_bridge_stamp();
        let history = app.undo_step_count();
        let references = app.topology_results.values().cloned().collect::<Vec<_>>();
        let proposal = app
            .derive_assistant_cad_edit_proposal(&program(finishes(&app)[finish_index].clone()))
            .unwrap();
        assert_eq!(app.live_bridge_stamp(), before);
        assert_eq!(app.undo_step_count(), history);
        assert!(app.topology_results.is_bound_to(&app.document.current()));
        assert_eq!(
            app.topology_results.values().cloned().collect::<Vec<_>>(),
            references
        );
        app.document.commit_proposal(&proposal).unwrap();
        let committed = app.document.current();
        assert_eq!(app.undo_step_count(), history + 1);
        assert!(
            ExactBRepGraph::from_snapshot(&committed, INITIAL_BOX_DEFINITION, FeatureId(3)).is_ok()
        );
        assert!(app.undo());
        assert_eq!(
            app.document.current().canonical_digest(),
            before.canonical_digest
        );
        assert!(app.redo());
        assert_eq!(
            app.document.current().canonical_digest(),
            committed.canonical_digest()
        );
    }
}

#[test]
fn prefix_rejects_changed_producer_geometry_and_stale_registry() {
    for finish_index in 0..3 {
        for stale_registry in [false, true] {
            let mut app = KetchupApp::new();
            install_initial_graph_result(&mut app);
            let mut input = program(finishes(&app)[finish_index].clone());
            if stale_registry {
                app.document
                    .apply_batch(&CommandBatch::new(vec![
                        CanonicalCommand::SetOccurrenceColor {
                            id: OccurrenceId(1),
                            color: Some([9, 8, 7]),
                        },
                    ]))
                    .unwrap();
                assert!(!app.topology_results.is_bound_to(&app.document.current()));
            } else {
                input.operations[0] = AssistantCadEditOperation::SetDimension {
                    feature_id: 2,
                    constraint_id: None,
                    value_mm: 73.0,
                };
            }
            let before = app.live_bridge_stamp();
            let history = app.undo_step_count();
            let error = app.derive_assistant_cad_edit_proposal(&input).unwrap_err();
            assert_eq!(error.code, "planning.cad_topology_reference_unavailable");
            assert_eq!(app.live_bridge_stamp(), before);
            assert_eq!(app.undo_step_count(), history);
        }
    }
}

#[test]
fn prefix_never_authorizes_forged_or_duplicate_references() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let before = app.live_bridge_stamp();
    let history = app.undo_step_count();
    for feature in finishes(&app) {
        for duplicate in [false, true] {
            let mut feature = feature.clone();
            let ids = match &mut feature {
                AssistantCadBodyFeature::TopologyShell {
                    removed_face_reference_ids,
                    ..
                } => removed_face_reference_ids,
                AssistantCadBodyFeature::TopologyFillet {
                    edge_reference_ids, ..
                }
                | AssistantCadBodyFeature::TopologyChamfer {
                    edge_reference_ids, ..
                } => edge_reference_ids,
                _ => unreachable!(),
            };
            if duplicate {
                ids.push(ids[0].clone());
            } else {
                ids[0] = "f".repeat(64);
            }
            assert!(
                app.derive_assistant_cad_edit_proposal(&program(feature))
                    .is_err()
            );
            assert_eq!(app.live_bridge_stamp(), before);
            assert_eq!(app.undo_step_count(), history);
        }
    }
}
