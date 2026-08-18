use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind, OccurrenceId, ProposalCommitError, Transform,
};
use ketchup_core::drawing::{
    DrawingAuthoringError, DrawingError, DrawingSheet, DrawingSheetId, DrawingSource,
    OrthographicViewKind, prepare_create_drawing_sheet, prepare_delete_drawing_sheet,
    prepare_edit_drawing_sheet, project_orthographic_drawing,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactResultRegistry,
    build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(2);
const EXTRUSION: FeatureId = FeatureId(3);
const SHEET: DrawingSheetId = DrawingSheetId(10);

#[derive(Debug, Eq, PartialEq)]
struct StoreStamp {
    revision: u64,
    digest: String,
    revisions: usize,
    undo: usize,
    redo: usize,
}

fn stamp(document: &DocumentStore) -> StoreStamp {
    StoreStamp {
        revision: document.current().revision_id(),
        digest: document.current().canonical_digest(),
        revisions: document.revision_count(),
        undo: document.visible_undo_steps(),
        redo: document.visible_redo_steps(),
    }
}

fn seeded_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Drawing part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("30").unwrap(),
                },
            },
        ]))
        .unwrap();
    document
}

fn exact_package_for(
    snapshot: &ketchup_core::document::Snapshot,
    producer_feature_id: FeatureId,
    fingerprint: &str,
    bounds: [[f64; 3]; 2],
) -> Arc<ExactBodyPackage> {
    let request = ExactFeatureChainRequest::terminal_body_requests(snapshot, DEFINITION)
        .unwrap()
        .into_values()
        .find(|request| request.producer_feature_id() == producer_feature_id)
        .unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                snapshot.document_id(),
                producer_feature_id,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}:{fingerprint}"),
        )
    });
    Arc::new(
        build_box_render_package(
            &request,
            format!("exact-input:{fingerprint}"),
            fingerprint.into(),
            "occt".into(),
            "r0".into(),
            bounds,
            evidence,
        )
        .unwrap()
        .into(),
    )
}

fn exact_package(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
    bounds: [[f64; 3]; 2],
) -> Arc<ExactBodyPackage> {
    exact_package_for(snapshot, EXTRUSION, fingerprint, bounds)
}

fn registry(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
    bounds: [[f64; 3]; 2],
) -> ExactResultRegistry {
    ExactResultRegistry::accept(snapshot, [exact_package(snapshot, fingerprint, bounds)]).unwrap()
}

fn sheet(name: &str) -> DrawingSheet {
    DrawingSheet::new(SHEET, name, DrawingSource::Definition(DEFINITION)).unwrap()
}

#[test]
fn canonical_sheet_create_edit_delete_is_reviewed_undoable_and_persisted() {
    let mut document = seeded_document();
    let exact = registry(
        &document.current(),
        "initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let before_create = stamp(&document);
    let (create, drawing) = prepare_create_drawing_sheet(&document, &exact, sheet("A3")).unwrap();
    assert_eq!(stamp(&document), before_create);
    assert_eq!(drawing.views.len(), 3);
    assert_eq!(
        drawing
            .views
            .iter()
            .map(|view| view.kind)
            .collect::<Vec<_>>(),
        vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
        ]
    );
    assert!(
        drawing
            .views
            .iter()
            .all(|view| !view.visible_lines.is_empty())
    );
    assert!(drawing.views.iter().all(|view| {
        view.bounds_mm[0][0].is_finite()
            && view.bounds_mm[0][1].is_finite()
            && view.bounds_mm[1][0] > view.bounds_mm[0][0]
            && view.bounds_mm[1][1] > view.bounds_mm[0][1]
    }));
    document.commit_proposal(&create).unwrap();
    assert_eq!(document.visible_undo_steps(), before_create.undo + 1);
    assert_eq!(document.current().drawing_sheet(SHEET), Some(&sheet("A3")));

    let saved = persistence::save(&document.current());
    let mut reopened = persistence::load(&saved)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&sheet("A3")));
    let reopened_exact = registry(
        &reopened.current(),
        "reopened",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let reopened_drawing = project_orthographic_drawing(
        &reopened.current(),
        &reopened_exact,
        reopened.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(
        reopened_drawing.stable_source_identity,
        drawing.stable_source_identity
    );
    assert_eq!(reopened_drawing.views, drawing.views);

    let edit_before = reopened.visible_undo_steps();
    let (edit, edited_output) =
        prepare_edit_drawing_sheet(&reopened, &reopened_exact, sheet("A4")).unwrap();
    reopened.commit_proposal(&edit).unwrap();
    assert_eq!(reopened.visible_undo_steps(), edit_before + 1);
    assert_eq!(edited_output.views, reopened_drawing.views);
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&sheet("A4")));
    reopened.undo().unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&sheet("A3")));
    reopened.redo().unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&sheet("A4")));

    let delete = prepare_delete_drawing_sheet(&reopened, SHEET).unwrap();
    let delete_before = reopened.visible_undo_steps();
    reopened.commit_proposal(&delete).unwrap();
    assert_eq!(reopened.visible_undo_steps(), delete_before + 1);
    assert!(reopened.current().drawing_sheet(SHEET).is_none());
    reopened.undo().unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&sheet("A4")));
}

#[test]
fn under_constrained_assembly_source_is_rejected_without_mutation() {
    let mut document = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(20),
                definition_id: DEFINITION,
                name: "Fixed".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(21),
                definition_id: DEFINITION,
                name: "Free".into(),
                transform: Transform::from_translation(50.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(20),
                grounded: true,
            },
        ]))
        .unwrap();
    let exact = registry(
        &document.current(),
        "under-constrained",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let assembly_sheet = DrawingSheet::new(
        SHEET,
        "Assembly",
        DrawingSource::RigidAssembly {
            occurrence_ids: vec![OccurrenceId(20), OccurrenceId(21)],
        },
    )
    .unwrap();
    let before = stamp(&document);
    assert!(matches!(
        prepare_create_drawing_sheet(&document, &exact, assembly_sheet),
        Err(DrawingAuthoringError::Drawing(DrawingError::SourceNotRigid))
    ));
    assert_eq!(stamp(&document), before);
}

#[test]
fn rigid_assembly_source_projects_fixed_occurrences_in_world_space() {
    let mut document = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(20),
                definition_id: DEFINITION,
                name: "Left".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(21),
                definition_id: DEFINITION,
                name: "Right".into(),
                transform: Transform::from_translation(50.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(20),
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(21),
                grounded: true,
            },
        ]))
        .unwrap();
    let exact = registry(
        &document.current(),
        "assembly",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let assembly_sheet = DrawingSheet::new(
        SHEET,
        "Assembly",
        DrawingSource::RigidAssembly {
            occurrence_ids: vec![OccurrenceId(20), OccurrenceId(21)],
        },
    )
    .unwrap();
    let (proposal, drawing) =
        prepare_create_drawing_sheet(&document, &exact, assembly_sheet).unwrap();
    document.commit_proposal(&proposal).unwrap();
    assert!(drawing.is_current(&document.current()));
    assert!(
        drawing
            .stable_source_identity
            .starts_with("assembly:20,21/")
    );
    let front = drawing
        .views
        .iter()
        .find(|view| view.kind == OrthographicViewKind::Front)
        .unwrap();
    assert_eq!(front.bounds_mm[0][0], 0.0);
    assert_eq!(front.bounds_mm[1][0], 70.0);
}

#[test]
fn model_edit_recomputes_associative_output_with_stable_source_identity() {
    let mut document = seeded_document();
    let initial_registry = registry(
        &document.current(),
        "initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let (proposal, initial) =
        prepare_create_drawing_sheet(&document, &initial_registry, sheet("A3")).unwrap();
    document.commit_proposal(&proposal).unwrap();

    let edit = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("60").unwrap(),
            },
        ]))
        .unwrap();
    document.commit_proposal(&edit).unwrap();
    assert!(!initial.is_current(&document.current()));
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &initial_registry,
            document.current().drawing_sheet(SHEET).unwrap(),
        ),
        Err(DrawingError::SourceStale)
    );

    let current_registry = registry(
        &document.current(),
        "edited",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 60.0]],
    );
    let recomputed = project_orthographic_drawing(
        &document.current(),
        &current_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(
        recomputed.stable_source_identity,
        initial.stable_source_identity
    );
    assert_ne!(recomputed.result_digest, initial.result_digest);
    assert_ne!(recomputed.views, initial.views);
}

#[test]
fn model_edit_recompute_is_deterministic_across_undo_redo_and_save_open() {
    let mut document = seeded_document();
    let initial_registry = registry(
        &document.current(),
        "initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let (create, initial) =
        prepare_create_drawing_sheet(&document, &initial_registry, sheet("A3")).unwrap();
    document.commit_proposal(&create).unwrap();

    let initial_view_ids = initial
        .views
        .iter()
        .map(|view| view.stable_view_id.clone())
        .collect::<Vec<_>>();
    let initial_line_ids = initial
        .views
        .iter()
        .map(|view| {
            view.visible_lines
                .iter()
                .map(|line| line.stable_line_id.clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        initial_view_ids,
        vec![
            "sheet-10/view-front",
            "sheet-10/view-top",
            "sheet-10/view-right",
        ]
    );
    assert_eq!(
        initial
            .views
            .iter()
            .map(|view| view.bounds_mm)
            .collect::<Vec<_>>(),
        vec![
            [[0.0, 0.0], [20.0, 30.0]],
            [[0.0, 0.0], [20.0, 10.0]],
            [[0.0, 0.0], [10.0, 30.0]],
        ]
    );

    let edit = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("60").unwrap(),
            },
        ]))
        .unwrap();
    document.commit_proposal(&edit).unwrap();
    let edited_registry = registry(
        &document.current(),
        "edited",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 60.0]],
    );
    let edited = project_orthographic_drawing(
        &document.current(),
        &edited_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &edited_registry,
            document.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        edited
    );
    assert_eq!(
        edited
            .views
            .iter()
            .map(|view| view.stable_view_id.clone())
            .collect::<Vec<_>>(),
        initial_view_ids
    );
    assert_eq!(
        edited
            .views
            .iter()
            .map(|view| {
                view.visible_lines
                    .iter()
                    .map(|line| line.stable_line_id.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>(),
        initial_line_ids
    );
    assert_eq!(
        edited
            .views
            .iter()
            .map(|view| view.bounds_mm)
            .collect::<Vec<_>>(),
        vec![
            [[0.0, 0.0], [20.0, 60.0]],
            [[0.0, 0.0], [20.0, 10.0]],
            [[0.0, 0.0], [10.0, 60.0]],
        ]
    );

    document.undo().unwrap();
    let undo_registry = registry(
        &document.current(),
        "initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &undo_registry,
            document.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        initial
    );

    document.redo().unwrap();
    let redo_registry = registry(
        &document.current(),
        "edited",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 60.0]],
    );
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &redo_registry,
            document.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        edited
    );

    let saved = persistence::save(&document.current());
    let reopened = persistence::load(&saved)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_registry = registry(
        &reopened.current(),
        "edited",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 60.0]],
    );
    assert_eq!(
        project_orthographic_drawing(
            &reopened.current(),
            &reopened_registry,
            reopened.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        edited
    );
}

#[test]
fn stale_failed_ambiguous_lost_and_stale_confirmation_leave_state_unchanged() {
    let mut document = seeded_document();
    let initial_registry = registry(
        &document.current(),
        "initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let (create, accepted_output) =
        prepare_create_drawing_sheet(&document, &initial_registry, sheet("A3")).unwrap();
    document.commit_proposal(&create).unwrap();
    let post_create_registry =
        ExactResultRegistry::carried_forward(&document.current(), &initial_registry);

    let stale_edit = prepare_edit_drawing_sheet(&document, &post_create_registry, sheet("A4"))
        .unwrap()
        .0;
    let unrelated = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: DEFINITION,
                name: "Renamed".into(),
            },
        ]))
        .unwrap();
    document.commit_proposal(&unrelated).unwrap();
    let before_stale_confirm = stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale_edit),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stamp(&document), before_stale_confirm);

    let current = document.current();
    let current_package =
        exact_package(&current, "current-a", [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]]);
    let current_registry =
        ExactResultRegistry::accept(&current, [Arc::clone(&current_package)]).unwrap();
    let accepted_current_output = project_orthographic_drawing(
        &current,
        &current_registry,
        current.drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    let second_package =
        exact_package(&current, "current-b", [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]]);
    let conflicting =
        ExactResultRegistry::accept(&current, [current_package, second_package]).unwrap();
    let before_failures = stamp(&document);
    assert_eq!(
        project_orthographic_drawing(
            &current,
            &ExactResultRegistry::default(),
            current.drawing_sheet(SHEET).unwrap(),
        ),
        Err(DrawingError::SourceFailed)
    );
    assert_eq!(
        project_orthographic_drawing(
            &current,
            &conflicting,
            current.drawing_sheet(SHEET).unwrap(),
        ),
        Err(DrawingError::SourceStale)
    );

    let mut multibody = seeded_document();
    let create_body = multibody
        .plan_body_command(CanonicalCommand::CreateBody {
            definition_id: DEFINITION,
            id: BodyId(2),
            name: "Second body".into(),
            visible: true,
        })
        .unwrap();
    multibody.commit_proposal(&create_body).unwrap();
    let activate_body = multibody
        .plan_body_command(CanonicalCommand::SetActiveBody {
            definition_id: DEFINITION,
            id: BodyId(2),
        })
        .unwrap();
    multibody.commit_proposal(&activate_body).unwrap();
    let second_body_feature = multibody
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DEFINITION,
                name: "Second profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(5),
                definition_id: DEFINITION,
                name: "Second extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(4),
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
        ]))
        .unwrap();
    multibody.commit_proposal(&second_body_feature).unwrap();
    let multibody_snapshot = multibody.current();
    let multibody_registry = ExactResultRegistry::accept(
        &multibody_snapshot,
        [
            exact_package_for(
                &multibody_snapshot,
                EXTRUSION,
                "body-a",
                [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
            ),
            exact_package_for(
                &multibody_snapshot,
                FeatureId(5),
                "body-b",
                [[0.0, 0.0, 0.0], [5.0, 5.0, 10.0]],
            ),
        ],
    )
    .unwrap();
    let before_ambiguous = stamp(&multibody);
    assert_eq!(
        project_orthographic_drawing(
            &multibody_snapshot,
            &multibody_registry,
            &sheet("Ambiguous multi-body source"),
        ),
        Err(DrawingError::SourceAmbiguous)
    );
    assert_eq!(stamp(&multibody), before_ambiguous);

    let lost = DrawingSheet::new(
        DrawingSheetId(99),
        "Lost",
        DrawingSource::Definition(DefinitionId(999)),
    )
    .unwrap();
    assert_eq!(
        project_orthographic_drawing(&current, &conflicting, &lost),
        Err(DrawingError::SourceLost)
    );
    assert_eq!(stamp(&document), before_failures);
    assert_eq!(
        project_orthographic_drawing(
            &current,
            &current_registry,
            current.drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        accepted_current_output
    );
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &initial_registry,
            document.current().drawing_sheet(SHEET).unwrap(),
        ),
        Err(DrawingError::SourceStale)
    );
    assert!(!accepted_output.is_current(&document.current()));
}
