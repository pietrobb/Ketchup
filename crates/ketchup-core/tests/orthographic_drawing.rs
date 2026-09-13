use ketchup_core::document::{
    BodyId, CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
    FeatureKind, OccurrenceId, ProposalCommitError, Transform,
};
use ketchup_core::drawing::{
    DrawingAngularDimension, DrawingAnnotations, DrawingAuthoringError, DrawingCircularDimension,
    DrawingCircularDimensionKind, DrawingDatumId, DrawingDatumReference, DrawingDatumSymbol,
    DrawingDimensionId, DrawingDimensionTolerance, DrawingError, DrawingFeatureControlFrame,
    DrawingFeatureControlFrameId, DrawingGeometricCharacteristic, DrawingLinearDimension,
    DrawingMargins, DrawingMaterialCondition, DrawingNote, DrawingNoteId, DrawingPageOrientation,
    DrawingPageSize, DrawingPageTemplate, DrawingScale, DrawingSheet, DrawingSheetId,
    DrawingSource, DrawingTitleBlock, OrthographicViewKind, prepare_create_drawing_sheet,
    prepare_delete_drawing_sheet, prepare_edit_drawing_sheet, project_orthographic_drawing,
};
use ketchup_core::drawing_export::{DrawingExportError, export_drawing};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    ExactBRepGraphEdgeEvidence, ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence,
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactProductError,
    ExactResultRegistry, build_box_render_package, canonical_reference_lineage_digest,
};
use ketchup_core::import::{StepImportMesh, StepMeshTriangle};
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

fn graph_registry_with_circle(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
    radius_mm: f64,
) -> ExactResultRegistry {
    let graph = ExactBRepGraph::from_snapshot(snapshot, DEFINITION, EXTRUSION).unwrap();
    let bounds_mm = graph.producer_bounds_mm().unwrap().unwrap();
    let [minimum, maximum] = bounds_mm;
    let center = [10.0, 0.0, (minimum[2] + maximum[2]) * 0.5];
    let mesh = StepImportMesh {
        vertices_mm: vec![
            [minimum[0], minimum[1], minimum[2]],
            [maximum[0], minimum[1], minimum[2]],
            [maximum[0], maximum[1], minimum[2]],
            [minimum[0], maximum[1], minimum[2]],
            [minimum[0], minimum[1], maximum[2]],
            [maximum[0], minimum[1], maximum[2]],
            [maximum[0], maximum[1], maximum[2]],
            [minimum[0], maximum[1], maximum[2]],
        ],
        triangles: [
            ([0, 2, 1], 0),
            ([0, 3, 2], 0),
            ([4, 5, 6], 1),
            ([4, 6, 7], 1),
            ([0, 1, 5], 2),
            ([0, 5, 4], 2),
            ([1, 2, 6], 3),
            ([1, 6, 5], 3),
            ([2, 3, 7], 4),
            ([2, 7, 6], 4),
            ([3, 0, 4], 5),
            ([3, 4, 7], 5),
        ]
        .into_iter()
        .map(|(vertex_indices, face_ordinal)| StepMeshTriangle {
            vertex_indices,
            face_ordinal,
        })
        .collect(),
    };
    let package = ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            exact_input_digest: format!("typed-dimension-input:{fingerprint}"),
            result_fingerprint: fingerprint.into(),
            volume_mm3: (maximum[0] - minimum[0])
                * (maximum[1] - minimum[1])
                * (maximum[2] - minimum[2]),
            area_mm2: 0.0,
            topology_counts: [8, 12, 6, 1, 1],
            wire_count: None,
            bounds_mm,
            backend: "typed-dimension-exact".into(),
            tolerance: "1e-7-mm".into(),
            faces: Vec::new(),
            edges: vec![ExactBRepGraphEdgeEvidence {
                edge_ordinal: 0,
                curve_kind: "circle".into(),
                length_mm: std::f64::consts::TAU * radius_mm,
                centroid_mm: center,
                bounds_mm: [
                    [center[0] - radius_mm, center[1], center[2] - radius_mm],
                    [center[0] + radius_mm, center[1], center[2] + radius_mm],
                ],
                closed: true,
                circle_radius_mm: Some(radius_mm),
                axis_origin_mm: Some(center),
                unit_axis_direction: Some([0.0, -1.0, 0.0]),
                adjacent_face_ordinals: Vec::new(),
            }],
        },
        &mesh,
    )
    .unwrap();
    ExactResultRegistry::accept(snapshot, [Arc::new(ExactBodyPackage::from(package))]).unwrap()
}

fn sheet(name: &str) -> DrawingSheet {
    DrawingSheet::new(SHEET, name, DrawingSource::Definition(DEFINITION)).unwrap()
}

fn production_sheet(revision: &str) -> DrawingSheet {
    DrawingSheet::with_contract(
        SHEET,
        "Manufacturing sheet",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::new(
            DrawingPageSize::A4,
            DrawingPageOrientation::Portrait,
            DrawingScale::new(1, 2).unwrap(),
            DrawingMargins::new(20, 10, 10, 10),
        )
        .unwrap(),
        DrawingTitleBlock::new("Drawing part", "DWG-001", revision, "Kečup").unwrap(),
    )
    .unwrap()
}

fn contract_digests(
    document: &DocumentStore,
    exact: &ExactResultRegistry,
    page: DrawingPageTemplate,
    title_block: DrawingTitleBlock,
) -> (String, String) {
    let sheet = DrawingSheet::with_contract(
        SHEET,
        "Digest sheet",
        DrawingSource::Definition(DEFINITION),
        page,
        title_block,
    )
    .unwrap();
    let layout_digest = project_orthographic_drawing(&document.current(), exact, &sheet)
        .unwrap()
        .layout
        .digest;
    let candidate = document
        .preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDrawingSheet(sheet),
        ]))
        .unwrap();
    (layout_digest, candidate.canonical_digest())
}

fn matches_segment(
    line: &ketchup_core::drawing::ProjectedVisibleLine,
    first: [f64; 2],
    second: [f64; 2],
) -> bool {
    let close = |left: [f64; 2], right: [f64; 2]| {
        (left[0] - right[0]).abs() <= 1.0e-9 && (left[1] - right[1]).abs() <= 1.0e-9
    };
    (close(line.start_mm, first) && close(line.end_mm, second))
        || (close(line.start_mm, second) && close(line.end_mm, first))
}

fn rendered_svg_node_bounds(tree: &resvg::usvg::Tree, id: &str) -> [f32; 4] {
    let bounds = tree.node_by_id(id).unwrap().abs_stroke_bounding_box();
    [bounds.x(), bounds.y(), bounds.right(), bounds.bottom()]
}

fn overlapping_pair_with_views(
    rear_transform: Transform,
    front_transform: Transform,
    views: Vec<OrthographicViewKind>,
) -> ketchup_core::drawing::OrthographicDrawing {
    let mut document = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(20),
                definition_id: DEFINITION,
                name: "Rear".into(),
                transform: rear_transform,
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(21),
                definition_id: DEFINITION,
                name: "Front".into(),
                transform: front_transform,
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
        "overlap",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let assembly_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Overlap",
        DrawingSource::RigidAssembly {
            occurrence_ids: vec![OccurrenceId(20), OccurrenceId(21)],
        },
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Overlap", "", "", "").unwrap(),
        views,
    )
    .unwrap();
    project_orthographic_drawing(&document.current(), &exact, &assembly_sheet).unwrap()
}

fn overlapping_pair(
    rear_transform: Transform,
    front_transform: Transform,
) -> ketchup_core::drawing::OrthographicDrawing {
    overlapping_pair_with_views(
        rear_transform,
        front_transform,
        vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
        ],
    )
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
    assert_eq!(
        drawing.schema,
        ketchup_core::drawing::ORTHOGRAPHIC_LINEWORK_SCHEMA_V2
    );
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
        view.visible_lines
            .iter()
            .chain(&view.hidden_lines)
            .all(|line| {
                let dx = line.end_mm[0] - line.start_mm[0];
                let dy = line.end_mm[1] - line.start_mm[1];
                dx * dx + dy * dy > 1.0e-12
            })
    }));
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
fn production_page_contract_is_deterministic_undoable_and_persisted() {
    let mut document = seeded_document();
    let exact = registry(
        &document.current(),
        "page-contract",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let legacy_default =
        project_orthographic_drawing(&document.current(), &exact, &sheet("Default")).unwrap();
    let initial_sheet = production_sheet("A");
    let (create, initial) =
        prepare_create_drawing_sheet(&document, &exact, initial_sheet.clone()).unwrap();
    let repeated =
        project_orthographic_drawing(&document.current(), &exact, &initial_sheet).unwrap();

    assert_eq!(initial.views, legacy_default.views);
    assert_eq!(initial.result_digest, legacy_default.result_digest);
    assert_eq!(initial.layout, repeated.layout);
    assert_eq!(initial.layout.digest.len(), 64);
    assert_eq!(
        initial.layout.digest,
        "646863f6a6f0a0c40e440108f53ec02618cbc75ffb0cfd849fb92b1a674c11cd"
    );
    assert_eq!(initial.layout.page_size_mm, [210.0, 297.0]);
    assert_eq!(
        initial.layout.border_bounds_mm,
        [[20.0, 10.0], [200.0, 287.0]]
    );
    assert_eq!(
        initial.layout.title_block_bounds_mm,
        [[20.0, 10.0], [200.0, 46.0]]
    );
    assert_eq!(
        initial
            .layout
            .view_placements
            .iter()
            .map(|placement| placement.kind)
            .collect::<Vec<_>>(),
        vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
        ]
    );
    assert!(initial.layout.view_placements.iter().all(|placement| {
        placement.page_bounds_mm[0][0] >= initial.layout.border_bounds_mm[0][0]
            && placement.page_bounds_mm[0][1] >= initial.layout.title_block_bounds_mm[1][1]
            && placement.page_bounds_mm[1][0] <= initial.layout.border_bounds_mm[1][0]
            && placement.page_bounds_mm[1][1] <= initial.layout.border_bounds_mm[1][1]
    }));

    document.commit_proposal(&create).unwrap();
    let exact = ExactResultRegistry::carried_forward(&document.current(), &exact);
    let edited_sheet = production_sheet("B");
    let (edit, edited) =
        prepare_edit_drawing_sheet(&document, &exact, edited_sheet.clone()).unwrap();
    assert_eq!(edited.views, initial.views);
    assert_eq!(edited.result_digest, initial.result_digest);
    assert_ne!(edited.layout.digest, initial.layout.digest);
    document.commit_proposal(&edit).unwrap();
    assert_eq!(document.current().drawing_sheet(SHEET), Some(&edited_sheet));

    document.undo().unwrap();
    assert_eq!(
        document.current().drawing_sheet(SHEET),
        Some(&initial_sheet)
    );
    document.redo().unwrap();
    assert_eq!(document.current().drawing_sheet(SHEET), Some(&edited_sheet));

    let reopened = persistence::load(&persistence::save(&document.current()))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&edited_sheet));
    assert_eq!(
        reopened.current().canonical_digest(),
        document.current().canonical_digest()
    );
}

#[test]
fn every_page_and_title_field_changes_layout_and_canonical_digests() {
    let document = seeded_document();
    let exact = registry(
        &document.current(),
        "digest-fields",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let page = |size, orientation, numerator, denominator, margins| {
        DrawingPageTemplate::new(
            size,
            orientation,
            DrawingScale::new(numerator, denominator).unwrap(),
            margins,
        )
        .unwrap()
    };
    let title = |name, number, revision, author| {
        DrawingTitleBlock::new(name, number, revision, author).unwrap()
    };
    let margins = DrawingMargins::new(20, 10, 10, 10);
    let baseline = contract_digests(
        &document,
        &exact,
        page(
            DrawingPageSize::A4,
            DrawingPageOrientation::Portrait,
            1,
            2,
            margins,
        ),
        title("Drawing part", "DWG-001", "A", "Kečup"),
    );
    let variants = [
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A3,
                DrawingPageOrientation::Portrait,
                1,
                2,
                margins,
            ),
            title("Drawing part", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Landscape,
                1,
                2,
                margins,
            ),
            title("Drawing part", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                3,
                margins,
            ),
            title("Drawing part", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                2,
                3,
                margins,
            ),
            title("Drawing part", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                2,
                DrawingMargins::new(21, 10, 10, 10),
            ),
            title("Drawing part", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                2,
                DrawingMargins::new(20, 11, 10, 10),
            ),
            title("Drawing part", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                2,
                DrawingMargins::new(20, 10, 11, 10),
            ),
            title("Drawing part", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                2,
                DrawingMargins::new(20, 10, 10, 11),
            ),
            title("Drawing part", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                2,
                margins,
            ),
            title("Changed title", "DWG-001", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                2,
                margins,
            ),
            title("Drawing part", "DWG-002", "A", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                2,
                margins,
            ),
            title("Drawing part", "DWG-001", "B", "Kečup"),
        ),
        contract_digests(
            &document,
            &exact,
            page(
                DrawingPageSize::A4,
                DrawingPageOrientation::Portrait,
                1,
                2,
                margins,
            ),
            title("Drawing part", "DWG-001", "A", "Other"),
        ),
    ];
    assert!(variants.iter().all(|value| value.0 != baseline.0));
    assert!(variants.iter().all(|value| value.1 != baseline.1));
    assert_eq!(
        variants
            .iter()
            .map(|value| &value.0)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        variants.len()
    );
    assert_eq!(
        variants
            .iter()
            .map(|value| &value.1)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        variants.len()
    );
}

#[test]
fn invalid_page_scale_text_and_overflow_fail_closed() {
    assert_eq!(
        [
            DrawingPageSize::A0,
            DrawingPageSize::A1,
            DrawingPageSize::A2,
            DrawingPageSize::A3,
            DrawingPageSize::A4,
        ]
        .map(DrawingPageSize::portrait_dimensions_mm),
        [[841, 1189], [594, 841], [420, 594], [297, 420], [210, 297]]
    );
    assert_eq!(
        DrawingScale::new(2, 4).unwrap(),
        DrawingScale::new(1, 2).unwrap()
    );
    assert_eq!(DrawingScale::new(0, 1), Err(DrawingError::InvalidScale));
    assert_eq!(
        DrawingScale::new(1_000_001, 1),
        Err(DrawingError::InvalidScale)
    );
    assert_eq!(
        DrawingPageTemplate::new(
            DrawingPageSize::A4,
            DrawingPageOrientation::Portrait,
            DrawingScale::default(),
            DrawingMargins::new(100, 105, 10, 10),
        ),
        Err(DrawingError::InvalidPageTemplate)
    );
    assert_eq!(
        DrawingTitleBlock::new("Invalid\nTitle", "", "", ""),
        Err(DrawingError::InvalidTitleBlock)
    );
    assert_eq!(
        DrawingTitleBlock::new("x".repeat(257), "", "", ""),
        Err(DrawingError::InvalidTitleBlock)
    );

    let document = seeded_document();
    let exact = registry(
        &document.current(),
        "overflow",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let oversized = DrawingSheet::with_contract(
        SHEET,
        "Overflow",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::new(
            DrawingPageSize::A4,
            DrawingPageOrientation::Portrait,
            DrawingScale::new(1_000_000, 1).unwrap(),
            DrawingMargins::default(),
        )
        .unwrap(),
        DrawingTitleBlock::new("Overflow", "", "", "").unwrap(),
    )
    .unwrap();
    let before = stamp(&document);
    assert_eq!(
        project_orthographic_drawing(&document.current(), &exact, &oversized),
        Err(DrawingError::LayoutOverflow)
    );
    assert_eq!(stamp(&document), before);
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
fn overlapping_bodies_split_and_classify_hidden_linework_deterministically() {
    let drawing = overlapping_pair(
        Transform::from_translation(0.0, 20.0, 0.0).unwrap(),
        Transform::from_translation(5.0, 0.0, 10.0).unwrap(),
    );
    let repeated = overlapping_pair(
        Transform::from_translation(0.0, 20.0, 0.0).unwrap(),
        Transform::from_translation(5.0, 0.0, 10.0).unwrap(),
    );
    assert_eq!(
        repeated.stable_source_identity,
        drawing.stable_source_identity
    );
    assert_eq!(repeated.result_digest, drawing.result_digest);
    assert_eq!(repeated.views, drawing.views);
    let front = drawing
        .views
        .iter()
        .find(|view| view.kind == OrthographicViewKind::Front)
        .unwrap();
    assert!(front.visible_lines.iter().any(|line| matches_segment(
        line,
        [20.0, 0.0],
        [20.0, 10.0]
    )));
    assert!(front.hidden_lines.iter().any(|line| matches_segment(
        line,
        [20.0, 10.0],
        [20.0, 30.0]
    )));
    assert!(
        front
            .visible_lines
            .iter()
            .chain(&front.hidden_lines)
            .all(|line| {
                let horizontal = (line.start_mm[1] - line.end_mm[1]).abs() <= 1.0e-9;
                let vertical = (line.start_mm[0] - line.end_mm[0]).abs() <= 1.0e-9;
                horizontal ^ vertical
            })
    );
}

#[test]
fn auxiliary_view_classifies_rotated_occluded_bodies_deterministically() {
    let auxiliary = OrthographicViewKind::auxiliary([1.0, -1.0, 1.0], [0.0, 0.0, 1.0]).unwrap();
    let rear = Transform::from_matrix([
        0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    let front = Transform::from_matrix([
        0.0, -1.0, 0.0, 12.0, 1.0, 0.0, 0.0, -8.0, 0.0, 0.0, 1.0, 10.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    let drawing = overlapping_pair_with_views(rear, front, vec![auxiliary]);
    let repeated = overlapping_pair_with_views(rear, front, vec![auxiliary]);

    assert_eq!(repeated.result_digest, drawing.result_digest);
    assert_eq!(repeated.views, drawing.views);
    let view = drawing.views.first().unwrap();
    assert_eq!(view.kind, auxiliary);
    assert!(!view.visible_lines.is_empty());
    assert!(!view.hidden_lines.is_empty());
    assert!(
        view.visible_lines
            .iter()
            .chain(&view.hidden_lines)
            .any(|line| {
                let delta = [
                    line.end_mm[0] - line.start_mm[0],
                    line.end_mm[1] - line.start_mm[1],
                ];
                delta[0].abs() > 1.0e-9 && delta[1].abs() > 1.0e-9
            })
    );
}

#[test]
fn section_view_clips_front_geometry_and_emits_deterministic_cut_contours() {
    let document = seeded_document();
    let exact = registry(
        &document.current(),
        "section",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let section = OrthographicViewKind::section([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], -5.0).unwrap();
    let section_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Section",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Section", "", "", "").unwrap(),
        vec![section],
    )
    .unwrap();

    let drawing =
        project_orthographic_drawing(&document.current(), &exact, &section_sheet).unwrap();
    let repeated =
        project_orthographic_drawing(&document.current(), &exact, &section_sheet).unwrap();
    assert_eq!(repeated.result_digest, drawing.result_digest);
    assert_eq!(repeated.views, drawing.views);
    let view = drawing.views.first().unwrap();
    let cut_lines = view
        .visible_lines
        .iter()
        .filter(|line| line.stable_line_id.contains("/section:"))
        .collect::<Vec<_>>();
    assert!(!cut_lines.is_empty());
    assert!(view.hidden_lines.is_empty());
    assert_eq!(view.bounds_mm, [[0.0, 0.0], [20.0, 30.0]]);
    assert!(cut_lines.iter().all(|line| {
        line.start_mm
            .into_iter()
            .chain(line.end_mm)
            .all(|coordinate| coordinate.is_finite())
    }));
}

#[test]
fn section_view_reveals_an_internal_opening_hidden_by_front_geometry() {
    let mut document = seeded_document();
    let transforms = [
        (0.0, 20.0, 0.0),
        (40.0, 20.0, 0.0),
        (20.0, 20.0, -20.0),
        (20.0, 20.0, 20.0),
        (0.0, 0.0, -10.0),
        (20.0, 0.0, -10.0),
        (40.0, 0.0, -10.0),
        (0.0, 0.0, 20.0),
        (20.0, 0.0, 20.0),
        (40.0, 0.0, 20.0),
    ];
    let mut commands = Vec::new();
    let mut occurrence_ids = Vec::new();
    for (offset, (x, y, z)) in transforms.into_iter().enumerate() {
        let id = OccurrenceId(100 + offset as u64);
        occurrence_ids.push(id);
        commands.push(CanonicalCommand::CreateOccurrence {
            id,
            definition_id: DEFINITION,
            name: format!("Opening member {offset}"),
            transform: Transform::from_translation(x, y, z).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        });
        commands.push(CanonicalCommand::SetOccurrenceGrounded { id, grounded: true });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let exact = registry(
        &document.current(),
        "internal-opening",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let section = OrthographicViewKind::section([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], -15.0).unwrap();
    let opening_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Internal opening",
        DrawingSource::RigidAssembly { occurrence_ids },
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Internal opening", "", "", "").unwrap(),
        vec![OrthographicViewKind::Front, section],
    )
    .unwrap();

    let drawing =
        project_orthographic_drawing(&document.current(), &exact, &opening_sheet).unwrap();
    let front = &drawing.views[0];
    let cut = &drawing.views[1];
    assert!(front.hidden_lines.iter().any(|line| matches_segment(
        line,
        [20.0, 10.0],
        [40.0, 10.0]
    )));
    assert!(
        cut.visible_lines
            .iter()
            .any(|line| matches_segment(line, [20.0, 10.0], [40.0, 10.0]))
    );
}

#[test]
fn empty_section_fails_closed_without_preparing_a_mutation() {
    let document = seeded_document();
    let exact = registry(
        &document.current(),
        "empty-section",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let section = OrthographicViewKind::section([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], -100.0).unwrap();
    let section_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Empty section",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Empty section", "", "", "").unwrap(),
        vec![section],
    )
    .unwrap();
    let before = stamp(&document);

    assert!(matches!(
        prepare_create_drawing_sheet(&document, &exact, section_sheet),
        Err(DrawingAuthoringError::Drawing(DrawingError::SourceFailed))
    ));
    assert_eq!(stamp(&document), before);
}

#[test]
fn top_and_right_views_use_their_positive_camera_depth_direction() {
    let top_drawing = overlapping_pair(
        Transform::identity(),
        Transform::from_translation(5.0, 5.0, 40.0).unwrap(),
    );
    let top = top_drawing
        .views
        .iter()
        .find(|view| view.kind == OrthographicViewKind::Top)
        .unwrap();
    assert!(
        top.visible_lines
            .iter()
            .any(|line| matches_segment(line, [20.0, 0.0], [20.0, 5.0]))
    );
    assert!(
        top.hidden_lines
            .iter()
            .any(|line| matches_segment(line, [20.0, 5.0], [20.0, 10.0]))
    );

    let right_drawing = overlapping_pair(
        Transform::identity(),
        Transform::from_translation(40.0, 5.0, 10.0).unwrap(),
    );
    let right = right_drawing
        .views
        .iter()
        .find(|view| view.kind == OrthographicViewKind::Right)
        .unwrap();
    assert!(right.visible_lines.iter().any(|line| matches_segment(
        line,
        [10.0, 0.0],
        [10.0, 10.0]
    )));
    assert!(right.hidden_lines.iter().any(|line| matches_segment(
        line,
        [10.0, 10.0],
        [10.0, 30.0]
    )));
}

#[test]
fn custom_views_are_reviewed_undoable_and_persisted() {
    let mut document = seeded_document();
    let exact = registry(
        &document.current(),
        "custom-views",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let initial_sheet = sheet("Default views");
    let (create, _) =
        prepare_create_drawing_sheet(&document, &exact, initial_sheet.clone()).unwrap();
    document.commit_proposal(&create).unwrap();
    let exact = ExactResultRegistry::carried_forward(&document.current(), &exact);
    let secondary_sheet = DrawingSheet::new(
        DrawingSheetId(11),
        "Secondary sheet",
        DrawingSource::Definition(DEFINITION),
    )
    .unwrap();
    let (create_secondary, _) =
        prepare_create_drawing_sheet(&document, &exact, secondary_sheet.clone()).unwrap();
    document.commit_proposal(&create_secondary).unwrap();
    let exact = ExactResultRegistry::carried_forward(&document.current(), &exact);
    let auxiliary = OrthographicViewKind::auxiliary([2.0, -3.0, 4.0], [0.0, 0.0, 1.0]).unwrap();
    let section = OrthographicViewKind::section([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], -5.0).unwrap();
    let detail = OrthographicViewKind::detail(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0],
        8.0,
        DrawingScale::new(2, 1).unwrap(),
    )
    .unwrap();
    let custom_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Custom views",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Custom views", "", "", "").unwrap(),
        vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
            OrthographicViewKind::Isometric,
            auxiliary,
            section,
            detail,
        ],
    )
    .unwrap();
    let before_edit = stamp(&document);
    let (edit, projected) =
        prepare_edit_drawing_sheet(&document, &exact, custom_sheet.clone()).unwrap();
    assert_eq!(stamp(&document), before_edit);
    assert_eq!(projected.views.len(), 7);
    let isometric = projected
        .views
        .iter()
        .find(|view| view.kind == OrthographicViewKind::Isometric)
        .unwrap();
    assert!(isometric.visible_lines.iter().any(|line| {
        let delta = [
            line.end_mm[0] - line.start_mm[0],
            line.end_mm[1] - line.start_mm[1],
        ];
        delta[0].abs() > 1.0e-9 && delta[1].abs() > 1.0e-9
    }));
    let auxiliary_view = projected
        .views
        .iter()
        .find(|view| view.kind == auxiliary)
        .unwrap();
    assert!(auxiliary_view.stable_view_id.contains("view-auxiliary-"));
    assert_ne!(auxiliary_view.bounds_mm, isometric.bounds_mm);
    let section_view = projected
        .views
        .iter()
        .find(|view| view.kind == section)
        .unwrap();
    assert!(section_view.stable_view_id.contains("view-section-"));
    assert!(
        section_view
            .visible_lines
            .iter()
            .any(|line| line.stable_line_id.contains("/section:"))
    );
    let detail_view = projected
        .views
        .iter()
        .find(|view| view.kind == detail)
        .unwrap();
    assert_eq!(detail_view.bounds_mm, [[0.0, 0.0], [8.0, 8.0]]);
    let detail_placement = projected
        .layout
        .view_placements
        .iter()
        .find(|placement| placement.kind == detail)
        .unwrap();
    assert_eq!(
        detail_placement.page_bounds_mm[1][0] - detail_placement.page_bounds_mm[0][0],
        16.0
    );

    document.commit_proposal(&edit).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.current().drawing_sheet(SHEET), Some(&custom_sheet));
    document.undo().unwrap();
    assert_eq!(
        document.current().drawing_sheet(SHEET),
        Some(&initial_sheet)
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let saved = persistence::save(&document.current());
    let reopened = persistence::load(&saved)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&custom_sheet));
    assert_eq!(
        reopened.current().drawing_sheet(DrawingSheetId(11)),
        Some(&secondary_sheet)
    );
    assert_eq!(persistence::save(&reopened.current()), saved);
    let reopened_exact = registry(
        &reopened.current(),
        "custom-views",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let repeated = project_orthographic_drawing(
        &reopened.current(),
        &reopened_exact,
        reopened.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(repeated.result_digest, projected.result_digest);
    assert_eq!(repeated.views, projected.views);
}

#[test]
fn invalid_auxiliary_and_view_sets_fail_closed() {
    assert_eq!(
        OrthographicViewKind::auxiliary([0.0; 3], [0.0, 0.0, 1.0]),
        Err(DrawingError::InvalidView)
    );
    assert_eq!(
        OrthographicViewKind::auxiliary([1.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
        Err(DrawingError::InvalidView)
    );
    assert_eq!(
        OrthographicViewKind::section([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], f64::NAN),
        Err(DrawingError::InvalidView)
    );
    assert_eq!(
        OrthographicViewKind::section([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], 1.0e12 + 1.0),
        Err(DrawingError::InvalidView)
    );
    assert_eq!(
        OrthographicViewKind::detail(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [f64::NAN, 0.0],
            1.0,
            DrawingScale::default(),
        ),
        Err(DrawingError::InvalidView)
    );
    assert_eq!(
        OrthographicViewKind::detail(
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0],
            0.0,
            DrawingScale::default(),
        ),
        Err(DrawingError::InvalidView)
    );
    let make_sheet = |views| {
        DrawingSheet::with_contract_and_views(
            SHEET,
            "Invalid views",
            DrawingSource::Definition(DEFINITION),
            DrawingPageTemplate::default(),
            DrawingTitleBlock::new("Invalid views", "", "", "").unwrap(),
            views,
        )
    };
    assert_eq!(make_sheet(Vec::new()), Err(DrawingError::InvalidView));
    assert_eq!(
        make_sheet(vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Front,
        ]),
        Err(DrawingError::InvalidView)
    );
    let excessive_views = (0..33)
        .map(|index| {
            OrthographicViewKind::detail(
                [0.0, -1.0, 0.0],
                [0.0, 0.0, 1.0],
                [f64::from(index), 0.0],
                1.0,
                DrawingScale::default(),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(make_sheet(excessive_views), Err(DrawingError::InvalidView));
}

#[test]
fn empty_detail_crop_fails_without_mutation() {
    let document = seeded_document();
    let exact = registry(
        &document.current(),
        "empty-detail",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let detail = OrthographicViewKind::detail(
        [0.0, -1.0, 0.0],
        [0.0, 0.0, 1.0],
        [100.0, 100.0],
        1.0,
        DrawingScale::new(2, 1).unwrap(),
    )
    .unwrap();
    let detail_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Empty detail",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Empty detail", "", "", "").unwrap(),
        vec![detail],
    )
    .unwrap();
    let before = stamp(&document);

    assert!(matches!(
        prepare_create_drawing_sheet(&document, &exact, detail_sheet),
        Err(DrawingAuthoringError::Drawing(DrawingError::SourceFailed))
    ));
    assert_eq!(stamp(&document), before);
}

#[test]
fn typed_dimensions_follow_exact_graph_model_change_undo_redo_and_save_open() {
    let mut document = seeded_document();
    let initial_registry = graph_registry_with_circle(&document.current(), "typed-initial", 5.0);
    let plain_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Typed exact dimensions",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Typed exact dimensions", "TD-001", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
    )
    .unwrap();
    let plain =
        project_orthographic_drawing(&document.current(), &initial_registry, &plain_sheet).unwrap();
    let circle_id = plain.views[0].circles[0].stable_circle_id.clone();
    let perpendicular = plain.views[0]
        .visible_lines
        .iter()
        .enumerate()
        .find_map(|(left_index, left)| {
            let left_delta = [
                left.end_mm[0] - left.start_mm[0],
                left.end_mm[1] - left.start_mm[1],
            ];
            plain.views[0]
                .visible_lines
                .iter()
                .skip(left_index + 1)
                .find_map(|right| {
                    let right_delta = [
                        right.end_mm[0] - right.start_mm[0],
                        right.end_mm[1] - right.start_mm[1],
                    ];
                    ((left_delta[0] * right_delta[0] + left_delta[1] * right_delta[1]).abs()
                        <= 1.0e-9)
                        .then(|| [left.stable_line_id.clone(), right.stable_line_id.clone()])
                })
        })
        .unwrap();
    let typed_sheet = DrawingSheet::with_contract_views_and_annotations(
        SHEET,
        "Typed exact dimensions",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Typed exact dimensions", "TD-001", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::with_typed_dimensions(
            Vec::new(),
            vec![
                DrawingAngularDimension::new(
                    DrawingDimensionId(1),
                    OrthographicViewKind::Front,
                    perpendicular,
                    8.0,
                    DrawingDimensionTolerance::None,
                )
                .unwrap(),
            ],
            vec![
                DrawingCircularDimension::new(
                    DrawingDimensionId(2),
                    OrthographicViewKind::Front,
                    circle_id.clone(),
                    DrawingCircularDimensionKind::Radius,
                    45.0,
                    6.0,
                    DrawingDimensionTolerance::None,
                )
                .unwrap(),
                DrawingCircularDimension::new(
                    DrawingDimensionId(3),
                    OrthographicViewKind::Front,
                    circle_id,
                    DrawingCircularDimensionKind::Diameter,
                    135.0,
                    6.0,
                    DrawingDimensionTolerance::symmetric(0.1).unwrap(),
                )
                .unwrap(),
            ],
            Vec::new(),
        ),
    )
    .unwrap();
    let (create, initial) =
        prepare_create_drawing_sheet(&document, &initial_registry, typed_sheet).unwrap();
    assert_eq!(initial.layout.angular_dimensions[0].label, "90°");
    assert_eq!(initial.layout.circular_dimensions[0].label, "R5 mm");
    assert_eq!(initial.layout.circular_dimensions[1].label, "⌀10 ±0.1 mm");
    document.commit_proposal(&create).unwrap();

    let edit = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("60").unwrap(),
            },
        ]))
        .unwrap();
    document.commit_proposal(&edit).unwrap();
    let edited_registry = graph_registry_with_circle(&document.current(), "typed-edited", 7.0);
    let edited = project_orthographic_drawing(
        &document.current(),
        &edited_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(edited.layout.angular_dimensions[0].label, "90°");
    assert_eq!(edited.layout.circular_dimensions[0].label, "R7 mm");
    assert_eq!(edited.layout.circular_dimensions[1].label, "⌀14 ±0.1 mm");
    assert_ne!(edited.layout.digest, initial.layout.digest);

    document.undo().unwrap();
    let undo_registry = graph_registry_with_circle(&document.current(), "typed-initial", 5.0);
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
    let reopened = persistence::load(&persistence::save(&document.current()))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_registry = graph_registry_with_circle(&reopened.current(), "typed-edited", 7.0);
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
fn gdt_datums_follow_exact_model_change_consent_undo_and_save_open() {
    let mut document = seeded_document();
    let initial_registry = graph_registry_with_circle(&document.current(), "gdt-initial", 5.0);
    let plain_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "GD&T exact",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("GD&T exact", "GDT-001", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
    )
    .unwrap();
    let plain =
        project_orthographic_drawing(&document.current(), &initial_registry, &plain_sheet).unwrap();
    let source_line_id = plain.views[0].visible_lines[0].stable_line_id.clone();
    let make_sheet = |tolerance_mm| {
        DrawingSheet::with_contract_views_and_annotations(
            SHEET,
            "GD&T exact",
            DrawingSource::Definition(DEFINITION),
            DrawingPageTemplate::default(),
            DrawingTitleBlock::new("GD&T exact", "GDT-001", "A", "Kečup").unwrap(),
            vec![OrthographicViewKind::Front],
            DrawingAnnotations::with_manufacturing_annotations(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![
                    DrawingDatumSymbol::new(
                        DrawingDatumId(1),
                        OrthographicViewKind::Front,
                        source_line_id.clone(),
                        "A",
                        [15.0, 15.0],
                    )
                    .unwrap(),
                ],
                vec![
                    DrawingFeatureControlFrame::new(
                        DrawingFeatureControlFrameId(1),
                        OrthographicViewKind::Front,
                        source_line_id.clone(),
                        DrawingGeometricCharacteristic::Perpendicularity,
                        tolerance_mm,
                        false,
                        DrawingMaterialCondition::RegardlessOfFeatureSize,
                        vec![
                            DrawingDatumReference::new("A", DrawingMaterialCondition::None)
                                .unwrap(),
                        ],
                        [25.0, 25.0],
                    )
                    .unwrap(),
                ],
                Vec::new(),
            ),
        )
        .unwrap()
    };

    let before = stamp(&document);
    let (create, initial) =
        prepare_create_drawing_sheet(&document, &initial_registry, make_sheet(0.05)).unwrap();
    assert_eq!(
        stamp(&document),
        before,
        "preview must remain observational"
    );
    assert_eq!(initial.layout.datum_symbols[0].label, "A");
    assert_eq!(
        initial.layout.feature_control_frames[0].label,
        "PERPENDICULARITY | 0.05 RFS | A"
    );
    document.commit_proposal(&create).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), before.undo + 1);
    document.undo().unwrap();
    assert!(document.current().drawing_sheet(SHEET).is_none());
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let current_registry =
        ExactResultRegistry::carried_forward(&document.current(), &initial_registry);
    let stale_edit = prepare_edit_drawing_sheet(&document, &current_registry, make_sheet(0.1))
        .unwrap()
        .0;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("60").unwrap(),
            },
        ]))
        .unwrap();
    let before_stale_confirm = stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale_edit),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(stamp(&document), before_stale_confirm);
    assert_eq!(
        document
            .current()
            .drawing_sheet(SHEET)
            .unwrap()
            .feature_control_frames()[0]
            .tolerance_mm(),
        0.05
    );

    let edited_registry = graph_registry_with_circle(&document.current(), "gdt-edited", 7.0);
    let edited = project_orthographic_drawing(
        &document.current(),
        &edited_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(
        edited.layout.datum_symbols[0].source_line_id,
        source_line_id
    );
    assert_eq!(edited.layout.feature_control_frames[0].tolerance_mm, 0.05);
    assert_ne!(edited.layout.digest, initial.layout.digest);

    let reopened = persistence::load(&persistence::save(&document.current()))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_registry = graph_registry_with_circle(&reopened.current(), "gdt-edited", 7.0);
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
fn associative_toleranced_dimensions_recompute_across_undo_redo_and_save_open() {
    let mut document = seeded_document();
    let initial_registry = registry(
        &document.current(),
        "dimension-initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let plain_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Dimensioned",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Dimensioned", "DIM-001", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
    )
    .unwrap();
    let plain =
        project_orthographic_drawing(&document.current(), &initial_registry, &plain_sheet).unwrap();
    let source_line_id = plain.views[0]
        .visible_lines
        .iter()
        .find(|line| {
            let delta = [
                line.end_mm[0] - line.start_mm[0],
                line.end_mm[1] - line.start_mm[1],
            ];
            (delta[0] * delta[0] + delta[1] * delta[1] - 900.0).abs() <= 1.0e-9
        })
        .unwrap()
        .stable_line_id
        .clone();
    let symmetric_tolerance = DrawingDimensionTolerance::symmetric(0.2).unwrap();
    let bilateral_tolerance = DrawingDimensionTolerance::bilateral(0.3, 0.1).unwrap();
    let dimensions = vec![
        DrawingLinearDimension::with_tolerance(
            DrawingDimensionId(1),
            OrthographicViewKind::Front,
            source_line_id.clone(),
            8.0,
            symmetric_tolerance,
        )
        .unwrap(),
        DrawingLinearDimension::with_tolerance(
            DrawingDimensionId(2),
            OrthographicViewKind::Front,
            source_line_id.clone(),
            16.0,
            bilateral_tolerance,
        )
        .unwrap(),
    ];
    let dimensioned_sheet = DrawingSheet::with_contract_views_and_dimensions(
        SHEET,
        "Dimensioned",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Dimensioned", "DIM-001", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
        dimensions,
    )
    .unwrap();
    let digest_without_dimension = document
        .preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDrawingSheet(plain_sheet),
        ]))
        .unwrap()
        .canonical_digest();
    let (create, initial) =
        prepare_create_drawing_sheet(&document, &initial_registry, dimensioned_sheet).unwrap();
    assert_ne!(create.intended_result_digest(), digest_without_dimension);
    assert_eq!(initial.layout.linear_dimensions.len(), 2);
    assert!(
        initial
            .layout
            .linear_dimensions
            .iter()
            .all(|dimension| dimension.source_line_id == source_line_id)
    );
    assert_eq!(initial.layout.linear_dimensions[0].value_mm, 30.0);
    assert_eq!(
        initial.layout.linear_dimensions[0].tolerance,
        symmetric_tolerance
    );
    assert_eq!(initial.layout.linear_dimensions[0].label, "30 ±0.2 mm");
    assert_eq!(initial.layout.linear_dimensions[1].value_mm, 30.0);
    assert_eq!(
        initial.layout.linear_dimensions[1].tolerance,
        bilateral_tolerance
    );
    assert_eq!(initial.layout.linear_dimensions[1].label, "30 +0.3/-0.1 mm");
    assert_ne!(initial.layout.digest, plain.layout.digest);
    document.commit_proposal(&create).unwrap();

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
        "dimension-edited",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 60.0]],
    );
    let edited = project_orthographic_drawing(
        &document.current(),
        &edited_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(edited.layout.linear_dimensions[0].value_mm, 60.0);
    assert_eq!(
        edited.layout.linear_dimensions[0].tolerance,
        symmetric_tolerance
    );
    assert_eq!(edited.layout.linear_dimensions[0].label, "60 ±0.2 mm");
    assert_eq!(edited.layout.linear_dimensions[1].value_mm, 60.0);
    assert_eq!(
        edited.layout.linear_dimensions[1].tolerance,
        bilateral_tolerance
    );
    assert_eq!(edited.layout.linear_dimensions[1].label, "60 +0.3/-0.1 mm");
    assert_eq!(
        edited.layout.linear_dimensions[0].source_line_id,
        initial.layout.linear_dimensions[0].source_line_id
    );

    document.undo().unwrap();
    let undo_registry = registry(
        &document.current(),
        "dimension-initial",
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
        "dimension-edited",
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

    let reopened = persistence::load(&persistence::save(&document.current()))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_registry = registry(
        &reopened.current(),
        "dimension-edited",
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
fn associative_notes_and_title_block_follow_source_metadata_across_undo_redo_and_save_open() {
    let mut document = seeded_document();
    let initial_registry = registry(
        &document.current(),
        "annotation-initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        SHEET,
        "Manufacturing",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::parametric("{source_name}", "Sheet {sheet_name}", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::new(
            Vec::new(),
            vec![
                DrawingNote::new(
                    DrawingNoteId(1),
                    [30.0, 60.0],
                    "Manufacture {source_name} from {sheet_name}",
                )
                .unwrap(),
            ],
        ),
    )
    .unwrap();
    let (create, initial) =
        prepare_create_drawing_sheet(&document, &initial_registry, sheet.clone()).unwrap();
    assert_eq!(initial.layout.title_block.title(), "Drawing part");
    assert_eq!(
        initial.layout.title_block.drawing_number(),
        "Sheet Manufacturing"
    );
    assert_eq!(
        initial.layout.notes[0].text,
        "Manufacture Drawing part from Manufacturing"
    );
    assert_eq!(initial.layout.notes[0].stable_note_id, "sheet-10/note-1");
    document.commit_proposal(&create).unwrap();

    let rename = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: DEFINITION,
                name: "Renamed part".into(),
            },
        ]))
        .unwrap();
    document.commit_proposal(&rename).unwrap();
    let renamed_registry = registry(
        &document.current(),
        "annotation-renamed",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let renamed = project_orthographic_drawing(
        &document.current(),
        &renamed_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(renamed.layout.title_block.title(), "Renamed part");
    assert_eq!(
        renamed.layout.notes[0].text,
        "Manufacture Renamed part from Manufacturing"
    );
    assert_ne!(renamed.layout.digest, initial.layout.digest);

    document.undo().unwrap();
    let undo_registry = registry(
        &document.current(),
        "annotation-initial",
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
        "annotation-renamed",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    assert_eq!(
        project_orthographic_drawing(
            &document.current(),
            &redo_registry,
            document.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        renamed
    );

    let saved = persistence::save(&document.current());
    let reopened = persistence::load(&saved)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET), Some(&sheet));
    assert_eq!(persistence::save(&reopened.current()), saved);
    let reopened_registry = registry(
        &reopened.current(),
        "annotation-renamed",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    assert_eq!(
        project_orthographic_drawing(
            &reopened.current(),
            &reopened_registry,
            reopened.current().drawing_sheet(SHEET).unwrap(),
        )
        .unwrap(),
        renamed
    );
}

#[test]
fn svg_text_renders_at_viewbox_scale_without_annotation_collisions() {
    let mut document = seeded_document();
    let exact = registry(
        &document.current(),
        "svg-text-scale",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let plain_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "SVG text scale",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Title", "Number", "Revision", "Author").unwrap(),
        vec![OrthographicViewKind::Front],
    )
    .unwrap();
    let plain = project_orthographic_drawing(&document.current(), &exact, &plain_sheet).unwrap();
    let source_line_id = plain.views[0]
        .visible_lines
        .iter()
        .find(|line| {
            let delta = [
                line.end_mm[0] - line.start_mm[0],
                line.end_mm[1] - line.start_mm[1],
            ];
            (delta[0] * delta[0] + delta[1] * delta[1] - 900.0).abs() <= 1.0e-9
        })
        .unwrap()
        .stable_line_id
        .clone();
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        SHEET,
        "SVG text scale",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Title", "Number", "Revision", "Author").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::new(
            vec![
                DrawingLinearDimension::new(
                    DrawingDimensionId(1),
                    OrthographicViewKind::Front,
                    source_line_id,
                    8.0,
                )
                .unwrap(),
            ],
            vec![],
        ),
    )
    .unwrap();
    let (create, drawing) = prepare_create_drawing_sheet(&document, &exact, sheet).unwrap();
    document.commit_proposal(&create).unwrap();
    let exported = export_drawing(&document.current(), &drawing).unwrap();

    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let font_family = options
        .fontdb
        .faces()
        .next()
        .and_then(|face| face.families.first())
        .map(|(family, _)| family.clone())
        .unwrap();
    options.fontdb_mut().set_sans_serif_family(font_family);
    let tree = resvg::usvg::Tree::from_data(exported.svg(), &options).unwrap();
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    assert!(pixmap.data().chunks_exact(4).any(|pixel| pixel[3] != 0));

    let pixels_per_mm = tree.size().height() / drawing.layout.page_size_mm[1] as f32;
    let max_text_height = 5.0 * pixels_per_mm;
    let dimension = rendered_svg_node_bounds(&tree, "sheet-10/dimension-1");
    assert!(dimension[3] - dimension[1] < max_text_height);

    let title_border = rendered_svg_node_bounds(&tree, "title-block-2");
    let title_rows = [
        rendered_svg_node_bounds(&tree, "title-block/title"),
        rendered_svg_node_bounds(&tree, "title-block/drawing-number"),
        rendered_svg_node_bounds(&tree, "title-block/revision"),
        rendered_svg_node_bounds(&tree, "title-block/author"),
    ];
    assert!(title_rows[0][1] > title_border[3]);
    assert!(
        title_rows
            .iter()
            .all(|bounds| bounds[3] - bounds[1] < max_text_height)
    );
    assert!(title_rows.windows(2).all(|rows| rows[0][3] < rows[1][1]));
}

#[test]
fn production_exports_are_stable_across_recompute_undo_redo_and_save_open() {
    let mut document = seeded_document();
    let initial_registry = registry(
        &document.current(),
        "export-initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let plain_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Export",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Export", "", "", "").unwrap(),
        vec![OrthographicViewKind::Front],
    )
    .unwrap();
    let plain =
        project_orthographic_drawing(&document.current(), &initial_registry, &plain_sheet).unwrap();
    let source_line_id = plain.views[0]
        .visible_lines
        .iter()
        .find(|line| {
            let delta = [
                line.end_mm[0] - line.start_mm[0],
                line.end_mm[1] - line.start_mm[1],
            ];
            (delta[0] * delta[0] + delta[1] * delta[1] - 900.0).abs() <= 1.0e-9
        })
        .unwrap()
        .stable_line_id
        .clone();
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        SHEET,
        "Export",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::parametric("Výkres súčiastky", "ČV-⌀10", "Ž", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::new(
            vec![
                DrawingLinearDimension::with_tolerance(
                    DrawingDimensionId(1),
                    OrthographicViewKind::Front,
                    source_line_id,
                    8.0,
                    DrawingDimensionTolerance::symmetric(0.2).unwrap(),
                )
                .unwrap(),
            ],
            vec![
                DrawingNote::new(
                    DrawingNoteId(1),
                    [30.0, 60.0],
                    "Priemer ⌀10 mm; hĺbka 5 mm; tolerancia ±0.2 mm",
                )
                .unwrap(),
            ],
        ),
    )
    .unwrap();
    let (create, initial_drawing) =
        prepare_create_drawing_sheet(&document, &initial_registry, sheet).unwrap();
    document.commit_proposal(&create).unwrap();
    let initial = export_drawing(&document.current(), &initial_drawing).unwrap();
    assert_eq!(
        export_drawing(&document.current(), &initial_drawing).unwrap(),
        initial
    );
    let svg = std::str::from_utf8(initial.svg()).unwrap();
    assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(svg.contains("class=\"VISIBLE\""));
    assert!(svg.contains("class=\"HIDDEN\""));
    assert!(svg.contains("class=\"DIMENSION\""));
    assert!(svg.contains("30 ±0.2 mm"));
    assert!(svg.contains("Priemer ⌀10 mm; hĺbka 5 mm; tolerancia ±0.2 mm"));
    assert!(svg.contains(">Výkres súčiastky</text>"));
    let dxf = std::str::from_utf8(initial.dxf()).unwrap();
    assert!(dxf.contains("$INSUNITS\n70\n4"));
    assert!(dxf.contains("8\nVISIBLE"));
    assert!(dxf.contains("8\nHIDDEN\n6\nHIDDEN"));
    assert!(dxf.contains("8\nDIMENSION"));
    assert!(dxf.contains("0\nTABLE\n2\nAPPID\n70\n1\n0\nAPPID\n2\nKETCHUP\n70\n0\n0\nENDTAB"));
    let dxf_lines = dxf.lines().collect::<Vec<_>>();
    for (index, _) in dxf_lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == "1000")
    {
        assert_eq!(&dxf_lines[index - 2..index], &["1001", "KETCHUP"]);
    }
    assert!(dxf.contains("30 ±0.2 mm"));
    let pdf = std::str::from_utf8(initial.pdf()).unwrap();
    assert!(pdf.starts_with("%PDF-1.7"));
    assert!(pdf.contains("/Encoding 6 0 R /ToUnicode 7 0 R"));
    assert!(pdf.contains("<93> <010D>"));
    assert!(pdf.contains("<A2> <00B1>"));
    assert!(pdf.contains("<A3> <2300>"));
    assert!(pdf.contains("<333020A2302E32206D6D> Tj"));
    assert!(pdf.contains("<4B65937570> Tj"));
    assert!(!pdf.contains('?'));
    assert!(pdf.ends_with("%%EOF\n"));

    let edit = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("60").unwrap(),
            },
        ]))
        .unwrap();
    document.commit_proposal(&edit).unwrap();
    assert_eq!(
        export_drawing(&document.current(), &initial_drawing),
        Err(DrawingExportError::StaleDrawing)
    );
    let edited_registry = registry(
        &document.current(),
        "export-edited",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 60.0]],
    );
    let edited_drawing = project_orthographic_drawing(
        &document.current(),
        &edited_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    let edited = export_drawing(&document.current(), &edited_drawing).unwrap();
    assert_ne!(edited, initial);
    assert!(
        std::str::from_utf8(edited.svg())
            .unwrap()
            .contains("60 ±0.2 mm")
    );

    document.undo().unwrap();
    let undo_registry = registry(
        &document.current(),
        "export-initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let undo_drawing = project_orthographic_drawing(
        &document.current(),
        &undo_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(
        export_drawing(&document.current(), &undo_drawing).unwrap(),
        initial
    );

    document.redo().unwrap();
    let redo_registry = registry(
        &document.current(),
        "export-edited",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 60.0]],
    );
    let redo_drawing = project_orthographic_drawing(
        &document.current(),
        &redo_registry,
        document.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(
        export_drawing(&document.current(), &redo_drawing).unwrap(),
        edited
    );

    let reopened = persistence::load(&persistence::save(&document.current()))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_registry = registry(
        &reopened.current(),
        "export-edited",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 60.0]],
    );
    let reopened_drawing = project_orthographic_drawing(
        &reopened.current(),
        &reopened_registry,
        reopened.current().drawing_sheet(SHEET).unwrap(),
    )
    .unwrap();
    assert_eq!(
        export_drawing(&reopened.current(), &reopened_drawing).unwrap(),
        edited
    );
}

#[test]
fn pdf_export_rejects_text_outside_its_declared_font_repertoire() {
    let mut document = seeded_document();
    let exact = registry(
        &document.current(),
        "unsupported-pdf-text",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Unsupported PDF text",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Drawing 🚫", "", "", "").unwrap(),
        vec![OrthographicViewKind::Front],
    )
    .unwrap();
    let (create, drawing) = prepare_create_drawing_sheet(&document, &exact, sheet).unwrap();
    document.commit_proposal(&create).unwrap();
    let before = stamp(&document);

    assert_eq!(
        export_drawing(&document.current(), &drawing),
        Err(DrawingExportError::UnsupportedPdfText)
    );
    assert_eq!(stamp(&document), before);
}

#[test]
fn stale_malformed_and_excessive_exports_fail_without_mutation() {
    let mut document = seeded_document();
    let exact = registry(
        &document.current(),
        "export-negative",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let (create, drawing) =
        prepare_create_drawing_sheet(&document, &exact, sheet("Export")).unwrap();
    document.commit_proposal(&create).unwrap();
    let before = stamp(&document);

    let mut malformed = drawing.clone();
    malformed.layout.digest = "corrupt".into();
    assert_eq!(
        export_drawing(&document.current(), &malformed),
        Err(DrawingExportError::InvalidDrawing)
    );

    let mut excessive = drawing.clone();
    let line = excessive.views[0].visible_lines[0].clone();
    excessive.views[0].visible_lines.resize(100_001, line);
    assert_eq!(
        export_drawing(&document.current(), &excessive),
        Err(DrawingExportError::ResourceLimit)
    );
    assert_eq!(stamp(&document), before);
}

#[test]
fn invalid_and_excessive_annotations_fail_without_mutation() {
    assert_eq!(
        DrawingTitleBlock::parametric("{unknown}", "", "", ""),
        Err(DrawingError::InvalidTitleBlock)
    );
    assert_eq!(
        DrawingNote::new(DrawingNoteId(0), [30.0, 60.0], "Note"),
        Err(DrawingError::InvalidAnnotation)
    );
    assert_eq!(
        DrawingNote::new(DrawingNoteId(1), [f64::NAN, 60.0], "Note"),
        Err(DrawingError::InvalidAnnotation)
    );
    assert_eq!(
        DrawingNote::new(DrawingNoteId(1), [30.0, 60.0], "{unknown}"),
        Err(DrawingError::InvalidAnnotation)
    );

    let document = seeded_document();
    let exact = registry(
        &document.current(),
        "annotation-errors",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let outside = DrawingSheet::with_contract_views_and_annotations(
        SHEET,
        "Outside note",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Outside note", "", "", "").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::new(
            Vec::new(),
            vec![DrawingNote::new(DrawingNoteId(1), [1.0, 1.0], "Note").unwrap()],
        ),
    )
    .unwrap();
    let before = stamp(&document);
    assert_eq!(
        project_orthographic_drawing(&document.current(), &exact, &outside),
        Err(DrawingError::LayoutOverflow)
    );
    assert!(matches!(
        prepare_create_drawing_sheet(&document, &exact, outside),
        Err(DrawingAuthoringError::Drawing(DrawingError::LayoutOverflow))
    ));
    assert_eq!(stamp(&document), before);

    let notes = (1..=257)
        .map(|id| DrawingNote::new(DrawingNoteId(id), [30.0, 60.0], "Note").unwrap())
        .collect();
    assert_eq!(
        DrawingSheet::with_contract_views_and_annotations(
            SHEET,
            "Excessive notes",
            DrawingSource::Definition(DEFINITION),
            DrawingPageTemplate::default(),
            DrawingTitleBlock::new("Excessive notes", "", "", "").unwrap(),
            vec![OrthographicViewKind::Front],
            DrawingAnnotations::new(Vec::new(), notes),
        ),
        Err(DrawingError::InvalidAnnotation)
    );
    assert_eq!(stamp(&document), before);
}

#[test]
fn invalid_lost_and_excessive_linear_dimensions_fail_without_mutation() {
    let document = seeded_document();
    let exact = registry(
        &document.current(),
        "dimension-errors",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    assert_eq!(
        DrawingLinearDimension::new(
            DrawingDimensionId(1),
            OrthographicViewKind::Front,
            "sheet-10/view-front/edge",
            0.0,
        ),
        Err(DrawingError::InvalidDimension)
    );
    for tolerance in [
        DrawingDimensionTolerance::symmetric(0.0),
        DrawingDimensionTolerance::symmetric(f64::NAN),
        DrawingDimensionTolerance::symmetric(1.0e12 + 1.0),
        DrawingDimensionTolerance::bilateral(0.0, 0.0),
        DrawingDimensionTolerance::bilateral(0.1, -0.1),
    ] {
        assert_eq!(tolerance, Err(DrawingError::InvalidDimension));
    }
    let lost = DrawingLinearDimension::new(
        DrawingDimensionId(1),
        OrthographicViewKind::Front,
        "sheet-10/view-front/missing-edge",
        8.0,
    )
    .unwrap();
    let lost_sheet = DrawingSheet::with_contract_views_and_dimensions(
        SHEET,
        "Lost dimension",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Lost dimension", "", "", "").unwrap(),
        vec![OrthographicViewKind::Front],
        vec![lost],
    )
    .unwrap();
    let before = stamp(&document);
    assert_eq!(
        project_orthographic_drawing(&document.current(), &exact, &lost_sheet),
        Err(DrawingError::DimensionSourceLost)
    );
    assert!(matches!(
        prepare_create_drawing_sheet(&document, &exact, lost_sheet),
        Err(DrawingAuthoringError::Drawing(
            DrawingError::DimensionSourceLost
        ))
    ));
    assert_eq!(stamp(&document), before);

    let dimensions = (1..=257)
        .map(|id| {
            DrawingLinearDimension::new(
                DrawingDimensionId(id),
                OrthographicViewKind::Front,
                "sheet-10/view-front/missing-edge",
                8.0,
            )
            .unwrap()
        })
        .collect();
    assert_eq!(
        DrawingSheet::with_contract_views_and_dimensions(
            SHEET,
            "Excessive dimensions",
            DrawingSource::Definition(DEFINITION),
            DrawingPageTemplate::default(),
            DrawingTitleBlock::new("Excessive dimensions", "", "", "").unwrap(),
            vec![OrthographicViewKind::Front],
            dimensions,
        ),
        Err(DrawingError::InvalidDimension)
    );
    assert_eq!(stamp(&document), before);
}

#[test]
fn malformed_geometry_is_rejected_without_mutation() {
    let document = seeded_document();
    let snapshot = document.current();
    let mut malformed = (*exact_package(
        &snapshot,
        "malformed",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    ))
    .clone();
    match &mut malformed {
        ExactBodyPackage::Rectangle(package) => {
            package.triangles[0].vertex_indices[0] = u32::MAX;
        }
        _ => unreachable!(),
    }
    let before = stamp(&document);
    assert!(matches!(
        ExactResultRegistry::accept(&snapshot, [Arc::new(malformed)]),
        Err(ExactProductError::StaleResult)
    ));
    assert_eq!(stamp(&document), before);
}

#[test]
fn excessive_occlusion_work_is_rejected_without_mutation() {
    let mut document = seeded_document();
    let mut commands = Vec::new();
    let mut occurrence_ids = Vec::new();
    for offset in 0..170_u64 {
        let id = OccurrenceId(1_000 + offset);
        occurrence_ids.push(id);
        commands.push(CanonicalCommand::CreateOccurrence {
            id,
            definition_id: DEFINITION,
            name: format!("Bounded {offset}"),
            transform: Transform::from_translation(offset as f64 * 30.0, 0.0, 0.0).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        });
        commands.push(CanonicalCommand::SetOccurrenceGrounded { id, grounded: true });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let exact = registry(
        &document.current(),
        "bounded",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let bounded_sheet = DrawingSheet::new(
        SHEET,
        "Bounded",
        DrawingSource::RigidAssembly { occurrence_ids },
    )
    .unwrap();
    let before = stamp(&document);
    assert_eq!(
        project_orthographic_drawing(&document.current(), &exact, &bounded_sheet),
        Err(DrawingError::ResourceLimit)
    );
    assert_eq!(stamp(&document), before);
}

#[test]
fn model_edit_recomputes_associative_output_with_stable_source_identity() {
    let mut document = seeded_document();
    let initial_registry = registry(
        &document.current(),
        "initial",
        [[0.0, 0.0, 0.0], [20.0, 10.0, 30.0]],
    );
    let seven_view_sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Seven-view associative sheet",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Seven-view associative sheet", "", "", "").unwrap(),
        vec![
            OrthographicViewKind::Front,
            OrthographicViewKind::Top,
            OrthographicViewKind::Right,
            OrthographicViewKind::Isometric,
            OrthographicViewKind::auxiliary([2.0, -3.0, 4.0], [0.0, 0.0, 1.0]).unwrap(),
            OrthographicViewKind::section([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], -5.0).unwrap(),
            OrthographicViewKind::detail(
                [0.0, -1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0],
                8.0,
                DrawingScale::new(2, 1).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let (proposal, initial) =
        prepare_create_drawing_sheet(&document, &initial_registry, seven_view_sheet).unwrap();
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(initial.views.len(), 7);

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
