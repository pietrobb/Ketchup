use std::collections::BTreeSet;

use ketchup_application::plan_assistant_cad_edit_program as plan;
use ketchup_core::assistant_sidecar::*;
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, OccurrenceId, Transform,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{ExactPlanarOffsetRequest, ExactResultRegistry};

fn part() -> AssistantCadEditOperation {
    AssistantCadEditOperation::CreatePart {
        name: "Editable part".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 12.0,
        }],
        constraints: vec![AssistantSketchConstraint::Radius {
            id: 1,
            entity_id: 1,
            value_mm: 12.0,
        }],
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
        translation_mm: [5.0, 6.0, 7.0],
        rotation: None,
    }
}

fn program(operations: Vec<AssistantCadEditOperation>) -> AssistantCadEditProgram {
    AssistantCadEditProgram { operations }
}

fn explicit(id: u64) -> AssistantCadEntitySelector {
    AssistantCadEntitySelector::Occurrences {
        occurrence_ids: vec![id],
    }
}

fn translate(selector: AssistantCadEntitySelector, delta: [f64; 3]) -> AssistantCadEditOperation {
    AssistantCadEditOperation::Transform {
        selector,
        translation_mm: delta,
        rotation: None,
    }
}

fn seeded() -> DocumentStore {
    let mut document = DocumentStore::new();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![part()]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    document
}

#[test]
fn serializable_create_program_plans_without_gui_and_commits_one_editable_revision() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![part(), part()]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let registry = ExactResultRegistry::default();
    let batch = plan(&document, &BTreeSet::new(), &registry, &input).unwrap();
    assert_eq!(batch.commands().len(), 10);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);
    assert_eq!(
        batch,
        plan(&document, &BTreeSet::new(), &registry, &input).unwrap()
    );
    let preview = document.preview_batch(&batch).unwrap();
    for (definition, feature) in [(1, 3), (2, 6)] {
        assert!(matches!(
            preview.feature(FeatureId(feature)).unwrap().kind(),
            FeatureKind::Pad(_)
        ));
        assert!(
            ExactBRepGraph::from_snapshot(&preview, DefinitionId(definition), FeatureId(feature))
                .is_ok()
        );
    }
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(committed.definitions().count(), 2);
    assert_eq!(committed.occurrences().count(), 2);
    assert_eq!(committed.features().count(), 6);
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn explicit_targets_ignore_selection_and_current_selection_is_borrowed() {
    let document = seeded();
    let registry = ExactResultRegistry::default();
    let input = program(vec![translate(explicit(1), [1.0, 2.0, 3.0])]);
    let explicit_batch = plan(&document, &BTreeSet::new(), &registry, &input).unwrap();
    // Even an unrelated stale selection cannot override explicit targets.
    assert_eq!(
        explicit_batch,
        plan(
            &document,
            &BTreeSet::from([OccurrenceId(999)]),
            &registry,
            &input
        )
        .unwrap()
    );
    let selected_input = program(vec![translate(
        AssistantCadEntitySelector::CurrentSelection {},
        [1.0, 2.0, 3.0],
    )]);
    let selection = BTreeSet::from([OccurrenceId(1)]);
    assert_eq!(
        explicit_batch,
        plan(&document, &selection, &registry, &selected_input).unwrap()
    );
    assert_eq!(selection, BTreeSet::from([OccurrenceId(1)]));
    let error = plan(&document, &BTreeSet::new(), &registry, &selected_input).unwrap_err();
    assert_eq!(error.phase, AssistantRejectionPhase::ProposalPlanning);
    assert_eq!(error.code, "planning.cad_selector_invalid");
}

#[test]
fn missing_and_stale_selection_keep_canonical_diagnostics_without_mutation() {
    let mut document = seeded();
    let registry = ExactResultRegistry::default();
    let stale_selection = BTreeSet::from([OccurrenceId(1)]);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence {
                id: OccurrenceId(1),
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    for (selector, id) in [
        (explicit(999), 999),
        (AssistantCadEntitySelector::CurrentSelection {}, 1),
    ] {
        let error = plan(
            &document,
            &stale_selection,
            &registry,
            &program(vec![translate(selector, [1.0, 0.0, 0.0])]),
        )
        .unwrap_err();
        assert_eq!(error.phase, AssistantRejectionPhase::CanonicalValidation);
        assert_eq!(
            error.code,
            CanonicalError::OccurrenceNotFound(OccurrenceId(id)).code()
        );
        assert_eq!(error.operation, "transform_occurrence");
        assert_eq!(error.target, format!("occurrence:{id}"));
        assert!(error.retryable);
        assert_eq!(error.validate(), Ok(()));
    }
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.current().revision_id(), baseline.revision_id());
    assert_eq!(document.visible_undo_steps(), undo);
}

#[test]
fn transform_copy_pattern_and_mirror_use_accumulated_transforms_atomically() {
    let mut document = seeded();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let input = program(vec![
        translate(explicit(1), [10.0, 0.0, 0.0]),
        AssistantCadEditOperation::Transform {
            selector: explicit(1),
            translation_mm: [0.0; 3],
            rotation: Some(AssistantCadRotation {
                pivot_mm: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                angle_degrees: 90.0,
            }),
        },
        AssistantCadEditOperation::Copy {
            selector: explicit(1),
            translation_mm: [0.0, 20.0, 0.0],
        },
        AssistantCadEditOperation::LinearPattern {
            selector: explicit(1),
            instances: 3,
            step_mm: [25.0, 0.0, 0.0],
        },
        AssistantCadEditOperation::Mirror {
            selector: explicit(1),
            plane_origin_mm: [0.0; 3],
            plane_normal: [1.0, 0.0, 0.0],
        },
    ]);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 6);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    for (id, expected) in [
        (1, [-6.0, 15.0, 7.0]),
        (2, [-6.0, 35.0, 7.0]),
        (3, [19.0, 15.0, 7.0]),
        (4, [44.0, 15.0, 7.0]),
        (5, [6.0, 15.0, 7.0]),
    ] {
        let occurrence = committed.occurrence(OccurrenceId(id)).unwrap();
        assert_eq!(occurrence.definition_id(), DefinitionId(1));
        let transform = occurrence.transform();
        for (index, value) in [3, 7, 11].into_iter().zip(expected) {
            assert!((transform.matrix()[index] - value).abs() < 1e-9);
        }
    }
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
}

#[test]
fn invalid_and_deleted_targets_fail_the_whole_plan() {
    let document = seeded();
    let baseline = document.current();
    let registry = ExactResultRegistry::default();
    let error = plan(&document, &BTreeSet::new(), &registry, &program(vec![])).unwrap_err();
    assert_eq!(error.code, "intent.cad_edit_program_invalid");
    assert_eq!(error.phase, AssistantRejectionPhase::IntentValidation);
    let input = program(vec![
        translate(explicit(1), [100.0, 0.0, 0.0]),
        AssistantCadEditOperation::Delete {
            selector: explicit(1),
            dependency_policy: AssistantCadDeletePolicy::RejectIfReferenced,
        },
        translate(explicit(1), [10.0, 0.0, 0.0]),
    ]);
    let error = plan(&document, &BTreeSet::new(), &registry, &input).unwrap_err();
    assert_eq!(error.code, "planning.cad_target_deleted");
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 1);
}

#[test]
fn canonical_batch_validation_remains_atomic_for_deferred_dimension_errors() {
    let mut document = seeded();
    let baseline = document.current();
    // S1 preserves the planner/DocumentStore validation boundary: dimensions
    // are validated canonically when the returned batch is previewed/applied.
    let input = program(vec![
        translate(explicit(1), [10.0, 0.0, 0.0]),
        AssistantCadEditOperation::SetDimension {
            feature_id: 999,
            constraint_id: None,
            value_mm: 10.0,
        },
    ]);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(
        document.preview_batch(&batch).err().unwrap(),
        CanonicalError::FeatureNotFound(FeatureId(999))
    );
    assert_eq!(
        document.apply_batch(&batch).err().unwrap(),
        CanonicalError::FeatureNotFound(FeatureId(999))
    );
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 1);
}

#[test]
fn exact_planar_offset_preview_gate_is_preserved() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Profile".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Boundary".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]],
                },
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let offset = |distance_mm| {
        program(vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Offset".into(),
            feature: AssistantCadBodyFeature::PlanarOffset {
                profile_feature_id: 1,
                distance_mm,
            },
        }])
    };
    let registry = ExactResultRegistry::default();
    let batch = plan(&document, &BTreeSet::new(), &registry, &offset(-5.0)).unwrap();
    let candidate = document.preview_batch(&batch).unwrap();
    let request = ExactPlanarOffsetRequest::from_snapshot(&candidate, DefinitionId(1)).unwrap();
    assert_eq!(request.offset_feature_id, FeatureId(2));
    assert_eq!(
        request.expected_bounds_mm(),
        [[5.0, 5.0, 0.0], [75.0, 35.0, 0.0]]
    );
    let error = plan(&document, &BTreeSet::new(), &registry, &offset(-25.0)).unwrap_err();
    assert_eq!(error.code, "canonical.invalid_planar_offset");
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn missing_topology_evidence_cannot_authorize_a_finish() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Body".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("30", 30.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Instance".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: 1,
        name: "Finish".into(),
        feature: AssistantCadBodyFeature::TopologyFillet {
            target_feature_id: 2,
            edge_reference_ids: vec!["a".repeat(64)],
            radius_mm: 1.0,
        },
    }]);
    let error = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap_err();
    assert_eq!(error.code, "planning.cad_topology_reference_unavailable");
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}
