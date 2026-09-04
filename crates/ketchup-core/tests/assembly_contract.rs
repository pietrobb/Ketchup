use ketchup_core::assembly::{
    AssemblyDofStatus, AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
    AssemblyRecomputePublishError, AssemblyRecomputeStatus, AssemblyReferenceHealth,
    AssemblySolvePublishError, AssemblySolveStatus, AssemblySolverPolicy, AxialAttachment,
    AxialAttachmentKind, PlanarFaceAttachment, recompute_rigid_assembly, solve_rigid_assembly,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentId,
    DocumentStore, FeatureId, FeatureKind, GroupId, OccurrenceId, ProfileSegment,
    ProposalCommitError, ProposalPrepareError, TagId, Transform,
};
use ketchup_core::exact_product::{
    ExactAxialAttachmentInput, ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest,
    ExactPlanarFaceAttachmentInput, ExactResultRegistry, build_box_render_package,
    build_box_render_package_with_attachments, build_box_render_package_with_typed_attachments,
    canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use ketchup_core::state_view::encode_semantic_state;
use std::sync::Arc;

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(2);
const EXTRUSION: FeatureId = FeatureId(3);
const FIRST: OccurrenceId = OccurrenceId(10);
const SECOND: OccurrenceId = OccurrenceId(11);
const MATE: AssemblyMateId = AssemblyMateId(20);

#[derive(Debug, Eq, PartialEq)]
struct StoreStamp {
    revision_id: u64,
    digest: String,
    revision_count: usize,
    undo_steps: usize,
    redo_steps: usize,
}

fn store_stamp(document: &DocumentStore) -> StoreStamp {
    StoreStamp {
        revision_id: document.current().revision_id(),
        digest: document.current().canonical_digest(),
        revision_count: document.revision_count(),
        undo_steps: document.visible_undo_steps(),
        redo_steps: document.visible_redo_steps(),
    }
}

fn current_exact_package(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
) -> Arc<ExactBodyPackage> {
    current_exact_package_with_attachments(
        snapshot,
        fingerprint,
        &[
            ExactPlanarFaceAttachmentInput {
                role: ExactFaceRole::Top,
                local_origin_mm: [0.0; 3],
                local_unit_normal: [0.0, 0.0, 1.0],
            },
            ExactPlanarFaceAttachmentInput {
                role: ExactFaceRole::Bottom,
                local_origin_mm: [0.0; 3],
                local_unit_normal: [0.0, 0.0, -1.0],
            },
            ExactPlanarFaceAttachmentInput {
                role: ExactFaceRole::East,
                local_origin_mm: [0.0; 3],
                local_unit_normal: [1.0, 0.0, 0.0],
            },
        ],
    )
}

fn current_exact_package_without_attachments(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
) -> Arc<ExactBodyPackage> {
    current_exact_package_with_attachments(snapshot, fingerprint, &[])
}

fn current_exact_package_with_attachments(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
    attachments: &[ExactPlanarFaceAttachmentInput],
) -> Arc<ExactBodyPackage> {
    let request = ExactFeatureChainRequest::from_snapshot(snapshot, DEFINITION).unwrap();
    let FeatureKind::Extrusion { height, .. } = snapshot.feature(EXTRUSION).unwrap().kind() else {
        panic!("expected extrusion");
    };
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
                EXTRUSION,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}:{fingerprint}"),
        )
    });
    Arc::new(
        build_box_render_package_with_attachments(
            &request,
            format!("exact-input:{fingerprint}"),
            fingerprint.to_owned(),
            "occt".into(),
            "r0".into(),
            [[0.0, 0.0, 0.0], [10.0, 10.0, height.millimetres()]],
            evidence,
            attachments,
        )
        .unwrap()
        .into(),
    )
}

fn current_exact_package_with_axial_attachment(
    snapshot: &ketchup_core::document::Snapshot,
    fingerprint: &str,
    attachment: Option<ExactAxialAttachmentInput>,
) -> Arc<ExactBodyPackage> {
    let request = ExactFeatureChainRequest::from_snapshot(snapshot, DEFINITION).unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::CircleSide,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                snapshot.document_id(),
                EXTRUSION,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}:{fingerprint}"),
        )
    });
    Arc::new(
        build_box_render_package_with_typed_attachments(
            &request,
            format!("exact-input:{fingerprint}"),
            fingerprint.to_owned(),
            "occt".into(),
            "r0".into(),
            request.expected_bounds_mm(),
            evidence,
            &[],
            attachment.as_slice(),
        )
        .unwrap()
        .into(),
    )
}

fn seeded_circle_document() -> (DocumentStore, ketchup_core::exact_product::BodySubshapeRef) {
    let center = [5.0, 5.0];
    let radius = 5.0;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Assembly cylinder".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Circle profile".into(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::CircularArc {
                            start_mm: [center[0] + radius, center[1]],
                            end_mm: [center[0] - radius, center[1]],
                            center_mm: center,
                            clockwise: false,
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [center[0] - radius, center[1]],
                            end_mm: [center[0] + radius, center[1]],
                            center_mm: center,
                            clockwise: false,
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Cylinder".into(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Cylinder A".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Cylinder B".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let package = current_exact_package_with_axial_attachment(
        &snapshot,
        "initial-axis",
        Some(ExactAxialAttachmentInput {
            role: ExactFaceRole::CircleSide,
            kind: AxialAttachmentKind::CylindricalFace,
            local_origin_mm: [center[0], center[1], 0.0],
            local_unit_direction: [0.0, 0.0, 1.0],
        }),
    );
    (
        document,
        package
            .reference(ExactFaceRole::CircleSide)
            .unwrap()
            .clone(),
    )
}

fn seeded_document() -> (
    DocumentStore,
    ketchup_core::exact_product::BodySubshapeRef,
    ketchup_core::exact_product::BodySubshapeRef,
    ketchup_core::exact_product::BodySubshapeRef,
) {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Assembly part".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Extrusion".into(),
                kind: FeatureKind::Extrusion {
                    profile: PROFILE,
                    height: Dimension::from_decimal("10").unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Part A".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Part B".into(),
                transform: Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let request = ExactFeatureChainRequest::from_snapshot(&snapshot, DEFINITION).unwrap();
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
                EXTRUSION,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}"),
        )
    });
    let package = build_box_render_package(
        &request,
        "exact-input".into(),
        "result".into(),
        "occt".into(),
        "r0".into(),
        [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        evidence,
    )
    .unwrap();
    (
        document,
        package.reference(ExactFaceRole::Top).unwrap().clone(),
        package.reference(ExactFaceRole::Bottom).unwrap().clone(),
        package.reference(ExactFaceRole::East).unwrap().clone(),
    )
}

fn canonical_planar_endpoint(
    occurrence_id: OccurrenceId,
    reference: ketchup_core::exact_product::BodySubshapeRef,
) -> AssemblyMateEndpoint {
    let geometry = match reference.role() {
        Some(ExactFaceRole::Top) => Some(([0.0; 3], [0.0, 0.0, 1.0])),
        Some(ExactFaceRole::Bottom) => Some(([0.0; 3], [0.0, 0.0, -1.0])),
        Some(ExactFaceRole::East) => Some(([0.0; 3], [1.0, 0.0, 0.0])),
        _ => None,
    };
    match geometry
        .and_then(|(origin, normal)| PlanarFaceAttachment::new(reference.clone(), origin, normal))
    {
        Some(attachment) => AssemblyMateEndpoint::resolved_planar_face(occurrence_id, attachment),
        None => AssemblyMateEndpoint::resolved(occurrence_id, reference),
    }
}

fn cylindrical_reference(
    mut reference: ketchup_core::exact_product::BodySubshapeRef,
) -> ketchup_core::exact_product::BodySubshapeRef {
    let role = ExactFaceRole::CircleSide;
    reference.semantic_role = role.semantic_role().to_owned();
    reference.source_element_id = role.source_element_id().to_owned();
    reference.expected_type = role.expected_type().to_owned();
    reference.lineage_digest = canonical_reference_lineage_digest(
        reference.document_id,
        reference.producer_feature_id,
        &reference.semantic_role,
        &reference.source_element_id,
        &reference.expected_type,
    );
    reference
}

fn coincident_mate(
    id: AssemblyMateId,
    a: AssemblyMateEndpoint,
    b: AssemblyMateEndpoint,
) -> AssemblyMate {
    AssemblyMate::new(
        id,
        a,
        b,
        AssemblyMateKind::CoincidentPlanar {
            offset_mm: 0.0,
            reversed: false,
        },
    )
}

fn planar_endpoint(
    occurrence_id: OccurrenceId,
    reference: ketchup_core::exact_product::BodySubshapeRef,
    local_origin_mm: [f64; 3],
    local_unit_normal: [f64; 3],
) -> AssemblyMateEndpoint {
    AssemblyMateEndpoint::resolved_planar_face(
        occurrence_id,
        PlanarFaceAttachment::new(reference, local_origin_mm, local_unit_normal).unwrap(),
    )
}

#[test]
fn typed_planar_face_endpoints_are_bit_exact_schema_50_state_and_history() {
    assert_eq!(persistence::CURRENT_SCHEMA, 51);
    let (mut document, top, bottom, _east) = seeded_document();
    let before_digest = document.current().canonical_digest();
    let origin_a = [1.25, -0.0, 3.5];
    let normal_a = [0.0, -1.0, 0.0];
    let origin_b = [-2.5, 4.0, 10.0];
    let normal_b = [0.0, 1.0, 0.0];
    let mate = coincident_mate(
        MATE,
        planar_endpoint(FIRST, top, origin_a, normal_a),
        planar_endpoint(SECOND, bottom, origin_b, normal_b),
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(mate.clone()),
        ]))
        .unwrap();

    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    assert_ne!(committed_digest, before_digest);
    let committed_mate = committed.assembly_mate(MATE).unwrap();
    let attachment_a = committed_mate
        .endpoint_a()
        .planar_face_attachment()
        .unwrap();
    let attachment_b = committed_mate
        .endpoint_b()
        .planar_face_attachment()
        .unwrap();
    assert_eq!(
        attachment_a.local_origin_mm().map(f64::to_bits),
        origin_a.map(f64::to_bits)
    );
    assert_eq!(
        attachment_a.local_unit_normal().map(f64::to_bits),
        normal_a.map(f64::to_bits)
    );
    assert_eq!(
        attachment_b.local_origin_mm().map(f64::to_bits),
        origin_b.map(f64::to_bits)
    );
    assert_eq!(
        attachment_b.local_unit_normal().map(f64::to_bits),
        normal_b.map(f64::to_bits)
    );

    let state = encode_semantic_state(&committed).complete_v1().to_owned();
    assert!(state.contains("assembly_mate.20.endpoint_a.attachment=planar_face"));
    assert!(state.contains(
        "assembly_mate.20.endpoint_a.origin_f64_bits=3ff4000000000000,8000000000000000,400c000000000000"
    ));
    assert!(state.contains(
        "assembly_mate.20.endpoint_a.normal_f64_bits=0000000000000000,bff0000000000000,0000000000000000"
    ));
    assert!(state.contains("assembly_mate.20.endpoint_b.attachment=planar_face"));
    assert!(state.contains(
        "assembly_mate.20.endpoint_b.origin_f64_bits=c004000000000000,4010000000000000,4024000000000000"
    ));
    assert!(state.contains(
        "assembly_mate.20.endpoint_b.normal_f64_bits=0000000000000000,3ff0000000000000,0000000000000000"
    ));

    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    assert_eq!(reopened.source_schema(), 51);
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        state
    );
    assert_eq!(reopened.snapshot().assembly_mate(MATE), Some(&mate));

    assert_eq!(document.undo().unwrap().canonical_digest(), before_digest);
    assert!(document.current().assembly_mate(MATE).is_none());
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
    assert_eq!(document.current().assembly_mate(MATE), Some(&mate));
    assert_eq!(
        encode_semantic_state(&document.current()).complete_v1(),
        state
    );
}

#[test]
fn typed_axial_endpoints_are_bit_exact_ignore_labels_transform_origins_and_round_trip() {
    let (mut document, top, bottom, _east) = seeded_document();
    let top = cylindrical_reference(top);
    let bottom = cylindrical_reference(bottom);
    let origin_a = [2.0, -0.0, 4.0];
    let origin_b = [1.0, 2.0, 5.0];
    let direction_a = [1.0, 0.0, 0.0];
    let direction_b = [0.0, 1.0, 0.0];
    let mate = AssemblyMate::new(
        MATE,
        AssemblyMateEndpoint::resolved_axial(
            FIRST,
            AxialAttachment::new(
                top,
                AxialAttachmentKind::CylindricalFace,
                origin_a,
                direction_a,
            )
            .unwrap(),
        ),
        AssemblyMateEndpoint::resolved_axial(
            SECOND,
            AxialAttachment::new(
                bottom,
                AxialAttachmentKind::CylindricalFace,
                origin_b,
                direction_b,
            )
            .unwrap(),
        ),
        AssemblyMateKind::ConcentricAxial { reversed: true },
    );
    let before = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: SECOND,
                transform: Transform::from_matrix([
                    0.0, -1.0, 0.0, 20.0, 1.0, 0.0, 0.0, 5.0, 0.0, 0.0, 1.0, 7.0, 0.0, 0.0, 0.0,
                    1.0,
                ])
                .unwrap(),
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(mate.clone()),
        ]))
        .unwrap();
    let committed = document.current();
    let digest = committed.canonical_digest();
    assert_ne!(digest, before);
    let state = encode_semantic_state(&committed).complete_v1().to_owned();
    assert!(state.contains("endpoint_a.attachment=axial,kind=cylindrical_face"));
    assert!(
        state.contains(
            "endpoint_a.origin_f64_bits=4000000000000000,8000000000000000,4010000000000000"
        )
    );
    assert!(state.contains(
        "endpoint_b.direction_f64_bits=0000000000000000,3ff0000000000000,0000000000000000"
    ));
    let solved = solve_rigid_assembly(&committed, AssemblySolverPolicy::default()).unwrap();
    let solved_transform = solved.occurrence(SECOND).unwrap().transform();
    let m = solved_transform.matrix();
    let world_b_origin = [
        m[0] * origin_b[0] + m[1] * origin_b[1] + m[2] * origin_b[2] + m[3],
        m[4] * origin_b[0] + m[5] * origin_b[1] + m[6] * origin_b[2] + m[7],
        m[8] * origin_b[0] + m[9] * origin_b[1] + m[10] * origin_b[2] + m[11],
    ];
    let world_b_direction = [
        m[0] * direction_b[0] + m[1] * direction_b[1] + m[2] * direction_b[2],
        m[4] * direction_b[0] + m[5] * direction_b[1] + m[6] * direction_b[2],
        m[8] * direction_b[0] + m[9] * direction_b[1] + m[10] * direction_b[2],
    ];
    assert_near(world_b_origin[1], origin_a[1]);
    assert_near(world_b_origin[2], origin_a[2]);
    for axis in 0..3 {
        assert_near(world_b_direction[axis], -direction_a[axis]);
    }
    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    assert_eq!(reopened.source_schema(), 51);
    assert_eq!(reopened.snapshot().canonical_digest(), digest);
    assert_eq!(reopened.snapshot().assembly_mate(MATE), Some(&mate));
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        state
    );
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), digest);
}

#[test]
fn typed_planar_solver_transforms_local_frames_and_ignores_semantic_role_text() {
    let (mut document, mut top, mut bottom, _east) = seeded_document();
    top.semantic_role = ExactFaceRole::Bottom.semantic_role().to_owned();
    top.lineage_digest = canonical_reference_lineage_digest(
        top.document_id,
        top.producer_feature_id,
        &top.semantic_role,
        &top.source_element_id,
        &top.expected_type,
    );
    bottom.semantic_role = ExactFaceRole::Top.semantic_role().to_owned();
    bottom.lineage_digest = canonical_reference_lineage_digest(
        bottom.document_id,
        bottom.producer_feature_id,
        &bottom.semantic_role,
        &bottom.source_element_id,
        &bottom.expected_type,
    );
    let quarter_turn = Transform::from_matrix([
        0.0, -1.0, 0.0, 20.0, 1.0, 0.0, 0.0, 5.0, 0.0, 0.0, 1.0, 7.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: SECOND,
                transform: quarter_turn,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(coincident_mate(
                MATE,
                planar_endpoint(FIRST, top, [2.0, 3.0, 4.0], [1.0, 0.0, 0.0]),
                planar_endpoint(SECOND, bottom, [0.0, 1.0, 2.0], [0.0, 1.0, 0.0]),
            )),
        ]))
        .unwrap();

    let solved =
        solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default()).unwrap();
    assert_eq!(solved.status(), AssemblySolveStatus::UnderConstrained);
    assert!(solved.maximum_residual() <= AssemblySolverPolicy::default().linear_tolerance_mm);
    let solved_transform = solved.occurrence(SECOND).unwrap().transform();
    let matrix = solved_transform.matrix();
    for (actual, expected) in matrix.iter().copied().zip([
        0.0, -1.0, 0.0, 3.0, 1.0, 0.0, 0.0, 5.0, 0.0, 0.0, 1.0, 7.0, 0.0, 0.0, 0.0, 1.0,
    ]) {
        assert_near(actual, expected);
    }
    let world_origin_a = [2.0, 3.0, 4.0];
    let world_origin_b = [matrix[3] - 1.0, matrix[7], matrix[11] + 2.0];
    assert_near(world_origin_b[0], world_origin_a[0]);
    let world_normal_a = [1.0, 0.0, 0.0];
    let world_normal_b = [-1.0, 0.0, 0.0];
    for axis in 0..3 {
        assert_near(world_normal_b[axis], -world_normal_a[axis]);
    }
}

#[test]
fn recompute_refreshes_identity_bound_typed_attachments_and_fails_closed_without_evidence() {
    let (mut document, top, bottom, _east) = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(coincident_mate(
                MATE,
                planar_endpoint(FIRST, top, [5.0, 5.0, 10.0], [0.0, 0.0, 1.0]),
                planar_endpoint(SECOND, bottom, [5.0, 5.0, 0.0], [0.0, 0.0, -1.0]),
            )),
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("12").unwrap(),
            },
        ]))
        .unwrap();

    let source = document.current();
    let refreshed_inputs = [
        ExactPlanarFaceAttachmentInput {
            role: ExactFaceRole::Top,
            local_origin_mm: [6.25, 4.5, 12.0],
            local_unit_normal: [0.0, 0.0, 1.0],
        },
        ExactPlanarFaceAttachmentInput {
            role: ExactFaceRole::Bottom,
            local_origin_mm: [6.25, 4.5, 0.0],
            local_unit_normal: [0.0, 0.0, -1.0],
        },
    ];
    let registry = ExactResultRegistry::accept(
        &source,
        [current_exact_package_with_attachments(
            &source,
            "attachment-refresh",
            &refreshed_inputs,
        )],
    )
    .unwrap();
    let recomputed =
        recompute_rigid_assembly(&document, &registry, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(recomputed.status(), AssemblyRecomputeStatus::Solved);
    let refreshed_mate = &recomputed.mates()[0];
    let refreshed_a = refreshed_mate
        .endpoint_a()
        .planar_face_attachment()
        .unwrap();
    let refreshed_b = refreshed_mate
        .endpoint_b()
        .planar_face_attachment()
        .unwrap();
    assert_eq!(
        refreshed_a.local_origin_mm().map(f64::to_bits),
        refreshed_inputs[0].local_origin_mm.map(f64::to_bits)
    );
    assert_eq!(
        refreshed_a.local_unit_normal().map(f64::to_bits),
        refreshed_inputs[0].local_unit_normal.map(f64::to_bits)
    );
    assert_eq!(
        refreshed_b.local_origin_mm().map(f64::to_bits),
        refreshed_inputs[1].local_origin_mm.map(f64::to_bits)
    );
    assert_eq!(
        refreshed_b.local_unit_normal().map(f64::to_bits),
        refreshed_inputs[1].local_unit_normal.map(f64::to_bits)
    );
    document
        .commit_proposal(&recomputed.prepare_publication(&document).unwrap())
        .unwrap();
    assert_eq!(
        document
            .current()
            .assembly_mate(MATE)
            .unwrap()
            .endpoint_a()
            .planar_face_attachment()
            .unwrap()
            .local_origin_mm()
            .map(f64::to_bits),
        refreshed_inputs[0].local_origin_mm.map(f64::to_bits)
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("14").unwrap(),
            },
        ]))
        .unwrap();
    let current = document.current();
    let registry_without_attachments = ExactResultRegistry::accept(
        &current,
        [current_exact_package_without_attachments(
            &current,
            "attachment-evidence-missing",
        )],
    )
    .unwrap();
    let broken = recompute_rigid_assembly(
        &document,
        &registry_without_attachments,
        AssemblySolverPolicy::default(),
    )
    .unwrap();
    assert_eq!(broken.status(), AssemblyRecomputeStatus::Broken);
    assert!(broken.solve().is_none());
    assert_eq!(
        broken.mates()[0].endpoint_a().health(),
        AssemblyReferenceHealth::Broken
    );
    assert!(
        broken.mates()[0]
            .endpoint_a()
            .planar_face_attachment()
            .is_none()
    );
}

#[test]
fn recompute_refreshes_typed_axial_geometry_and_breaks_without_axis_evidence() {
    let (mut document, circle_side) = seeded_circle_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                MATE,
                AssemblyMateEndpoint::resolved_axial(
                    FIRST,
                    AxialAttachment::cylindrical_face(
                        circle_side.clone(),
                        [1.0, 2.0, 3.0],
                        [0.0, 0.0, 1.0],
                    )
                    .unwrap(),
                ),
                AssemblyMateEndpoint::resolved_axial(
                    SECOND,
                    AxialAttachment::cylindrical_face(
                        circle_side,
                        [4.0, 5.0, 6.0],
                        [0.0, 1.0, 0.0],
                    )
                    .unwrap(),
                ),
                AssemblyMateKind::ConcentricAxial { reversed: false },
            )),
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("12").unwrap(),
            },
        ]))
        .unwrap();

    let source = document.current();
    let refreshed = ExactAxialAttachmentInput {
        role: ExactFaceRole::CircleSide,
        kind: AxialAttachmentKind::CylindricalFace,
        local_origin_mm: [6.25, 4.5, 0.0],
        local_unit_direction: [1.0, 0.0, 0.0],
    };
    let registry = ExactResultRegistry::accept(
        &source,
        [current_exact_package_with_axial_attachment(
            &source,
            "axial-attachment-refresh",
            Some(refreshed),
        )],
    )
    .unwrap();
    let recomputed =
        recompute_rigid_assembly(&document, &registry, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(recomputed.status(), AssemblyRecomputeStatus::Solved);
    for endpoint in [
        recomputed.mates()[0].endpoint_a(),
        recomputed.mates()[0].endpoint_b(),
    ] {
        let attachment = endpoint.axial_attachment().unwrap();
        assert_eq!(
            attachment.local_origin_mm().map(f64::to_bits),
            refreshed.local_origin_mm.map(f64::to_bits)
        );
        assert_eq!(
            attachment.local_unit_direction().map(f64::to_bits),
            refreshed.local_unit_direction.map(f64::to_bits)
        );
    }

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("14").unwrap(),
            },
        ]))
        .unwrap();
    let current = document.current();
    let registry_without_axis = ExactResultRegistry::accept(
        &current,
        [current_exact_package_with_axial_attachment(
            &current,
            "axial-attachment-missing",
            None,
        )],
    )
    .unwrap();
    let broken = recompute_rigid_assembly(
        &document,
        &registry_without_axis,
        AssemblySolverPolicy::default(),
    )
    .unwrap();
    assert_eq!(broken.status(), AssemblyRecomputeStatus::Broken);
    assert!(broken.solve().is_none());
    assert_eq!(
        broken.mates()[0].endpoint_a().health(),
        AssemblyReferenceHealth::Broken
    );
    assert!(broken.mates()[0].endpoint_a().axial_attachment().is_none());
}

#[test]
fn assembly_contract_is_reviewed_atomic_undoable_and_losslessly_persistent() {
    let (mut document, top, bottom, _east) = seeded_document();
    let before = document.current();
    let before_digest = before.canonical_digest();
    let before_undo = document.visible_undo_steps();
    let mate = coincident_mate(
        MATE,
        canonical_planar_endpoint(FIRST, top),
        canonical_planar_endpoint(SECOND, bottom),
    );
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(mate.clone()),
        ]))
        .unwrap();
    assert_eq!(document.current().canonical_digest(), before_digest);
    document.commit_proposal(&proposal).unwrap();

    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    assert_ne!(committed_digest, before_digest);
    assert_eq!(document.visible_undo_steps(), before_undo + 1);
    assert!(committed.occurrence_is_grounded(FIRST));
    assert_eq!(committed.assembly_mate(MATE), Some(&mate));
    let grounded_dof = committed.assembly_dof_diagnostic(FIRST).unwrap();
    assert_eq!(grounded_dof.status(), AssemblyDofStatus::Grounded);
    assert_eq!(grounded_dof.remaining_dof(), Some(0));
    assert_eq!(grounded_dof.incident_mate_ids(), &[MATE]);
    let pending_dof = committed.assembly_dof_diagnostic(SECOND).unwrap();
    assert_eq!(pending_dof.status(), AssemblyDofStatus::PendingSolve);
    assert_eq!(pending_dof.remaining_dof(), None);
    assert_eq!(pending_dof.incident_mate_ids(), &[MATE]);
    let state = encode_semantic_state(&committed);
    assert!(state.complete_v1().contains("occurrence.10.grounded=true"));
    assert!(
        state
            .complete_v1()
            .contains("occurrence.10.dof=status:grounded,remaining:0,incident_mates:[20]")
    );
    assert!(
        state.complete_v1().contains(
            "occurrence.11.dof=status:pending_solve,remaining:unknown,incident_mates:[20]"
        )
    );
    assert!(
        state
            .complete_v1()
            .contains("assembly_mate.20.kind=coincident_planar")
    );

    let reopened = persistence::load(&persistence::save(&committed)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        state.complete_v1()
    );
    assert!(reopened.snapshot().occurrence_is_grounded(FIRST));
    assert_eq!(reopened.snapshot().assembly_mate(MATE), Some(&mate));
    assert_eq!(document.undo().unwrap().canonical_digest(), before_digest);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}

#[test]
fn stale_unresolved_duplicate_and_in_use_requests_fail_without_partial_state() {
    let (mut document, top, bottom, _east) = seeded_document();
    let mate = coincident_mate(
        MATE,
        canonical_planar_endpoint(FIRST, top.clone()),
        canonical_planar_endpoint(SECOND, bottom.clone()),
    );
    let stale_delete = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence { id: FIRST },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(mate.clone()),
        ]))
        .unwrap();
    let before_stale_delete = store_stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale_delete),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store_stamp(&document), before_stale_delete);

    let stale = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyMateKind {
                id: MATE,
                kind: AssemblyMateKind::Distance { distance_mm: 5.0 },
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: SECOND,
                visible: false,
            },
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(30),
                name: "Assembly root".into(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::CreateGroup {
                id: GroupId(31),
                name: "Assembly child".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(30)),
            },
        ]))
        .unwrap();
    let before_stale_edit = store_stamp(&document);
    assert!(matches!(
        document.commit_proposal(&stale),
        Err(ProposalCommitError::Stale(_))
    ));
    assert_eq!(store_stamp(&document), before_stale_edit);

    let mut wrong_owner = top.clone();
    wrong_owner.definition_id = DefinitionId(999);
    let mut wrong_document = top.clone();
    wrong_document.document_id = DocumentId(999);
    let invalid = [
        coincident_mate(
            AssemblyMateId(21),
            AssemblyMateEndpoint::lost(FIRST, top.clone()),
            canonical_planar_endpoint(SECOND, bottom.clone()),
        ),
        coincident_mate(
            AssemblyMateId(22),
            AssemblyMateEndpoint::ambiguous(FIRST, top.clone(), 2),
            canonical_planar_endpoint(SECOND, bottom.clone()),
        ),
        coincident_mate(
            AssemblyMateId(23),
            canonical_planar_endpoint(FIRST, wrong_owner),
            canonical_planar_endpoint(SECOND, bottom.clone()),
        ),
        coincident_mate(
            AssemblyMateId(24),
            canonical_planar_endpoint(FIRST, wrong_document),
            canonical_planar_endpoint(SECOND, bottom.clone()),
        ),
    ];
    for candidate in invalid {
        let before = store_stamp(&document);
        assert!(matches!(
            document.apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceGrounded {
                    id: FIRST,
                    grounded: true,
                },
                CanonicalCommand::CreateAssemblyMate(candidate.clone()),
            ])),
            Err(CanonicalError::InvalidAssemblyMate(id)) if id == candidate.id()
        ));
        assert_eq!(store_stamp(&document), before);
        assert!(!document.current().occurrence_is_grounded(FIRST));
    }
    let before_duplicate = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(mate),
        ])),
        Err(CanonicalError::AssemblyMateAlreadyExists(MATE))
    ));
    assert_eq!(store_stamp(&document), before_duplicate);
    assert!(!document.current().occurrence_is_grounded(FIRST));
    let before_in_use = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence { id: FIRST }
        ])),
        Err(CanonicalError::OccurrenceInAssemblyMate(FIRST))
    ));
    assert_eq!(store_stamp(&document), before_in_use);
    let before_cycle = store_stamp(&document);
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::SetGroupParent {
                id: GroupId(30),
                parent: Some(GroupId(31)),
            },
        ])),
        Err(CanonicalError::GroupCycle(GroupId(30)))
    ));
    assert_eq!(store_stamp(&document), before_cycle);
    assert!(!document.current().occurrence_is_grounded(FIRST));
}

fn assert_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-5,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn rigid_solver_covers_supported_mates_with_fixed_policy_and_explicit_dof() {
    let cases = [
        (
            AssemblyMateKind::CoincidentPlanar {
                offset_mm: 0.0,
                reversed: false,
            },
            Transform::from_translation(20.0, 0.0, 7.0).unwrap(),
            0_u8,
        ),
        (
            AssemblyMateKind::ConcentricAxial { reversed: false },
            Transform::from_translation(20.0, 8.0, 7.0).unwrap(),
            1_u8,
        ),
        (
            AssemblyMateKind::Distance { distance_mm: 5.0 },
            Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
            2_u8,
        ),
        (
            AssemblyMateKind::Angle {
                angle_degrees: 60.0,
            },
            Transform::identity(),
            3_u8,
        ),
    ];

    for (kind, initial, case) in cases {
        let (mut document, top, bottom, east) = seeded_document();
        let endpoint_b_reference = match case {
            0 => bottom,
            3 => east,
            _ => top.clone(),
        };
        let (endpoint_a, endpoint_b) = if case == 1 {
            (
                AssemblyMateEndpoint::resolved_axial(
                    FIRST,
                    AxialAttachment::new(
                        cylindrical_reference(top),
                        AxialAttachmentKind::CylindricalFace,
                        [0.0; 3],
                        [0.0, 0.0, 1.0],
                    )
                    .unwrap(),
                ),
                AssemblyMateEndpoint::resolved_axial(
                    SECOND,
                    AxialAttachment::new(
                        cylindrical_reference(endpoint_b_reference),
                        AxialAttachmentKind::CylindricalFace,
                        [0.0; 3],
                        [0.0, 0.0, 1.0],
                    )
                    .unwrap(),
                ),
            )
        } else {
            (
                canonical_planar_endpoint(FIRST, top),
                canonical_planar_endpoint(SECOND, endpoint_b_reference),
            )
        };
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: SECOND,
                    transform: initial,
                },
                CanonicalCommand::SetOccurrenceGrounded {
                    id: FIRST,
                    grounded: true,
                },
                CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                    MATE, endpoint_a, endpoint_b, kind,
                )),
            ]))
            .unwrap();
        let source = document.current();
        let policy = AssemblySolverPolicy::default();
        let solved = solve_rigid_assembly(&source, policy).unwrap();
        assert_eq!(solved.schema(), "ketchup.rigid-assembly-solver.v1");
        assert_eq!(solved.iterations(), policy.max_iterations);
        assert_eq!(solved.status(), AssemblySolveStatus::UnderConstrained);
        assert!(solved.maximum_residual() <= policy.linear_tolerance_mm);
        assert!(solved.remaining_dof() > 0);
        assert_eq!(solved.occurrence(FIRST).unwrap().remaining_dof(), 0);
        assert!(solved.occurrence(FIRST).unwrap().grounded());
        assert_eq!(
            solved.occurrence(FIRST).unwrap().transform(),
            source.occurrence(FIRST).unwrap().transform()
        );
        let solved_transform = solved.occurrence(SECOND).unwrap().transform();
        let matrix = solved_transform.matrix();
        match case {
            0 => assert_near(matrix[11], 0.0),
            1 => {
                assert_near(matrix[3], 0.0);
                assert_near(matrix[7], 0.0);
                assert_near(matrix[11], 7.0);
            }
            2 => assert_near(matrix[3], 5.0),
            3 => assert_near(matrix[8], 60.0_f64.to_radians().cos()),
            _ => unreachable!(),
        }
    }
}

#[test]
fn rigid_solver_is_fully_constrained_and_publication_is_reviewed_and_stale_safe() {
    let (mut document, top, bottom, east) = seeded_document();
    let quarter_turn = Transform::from_matrix([
        0.0, -1.0, 0.0, 20.0, 1.0, 0.0, 0.0, 5.0, 0.0, 0.0, 1.0, 7.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: SECOND,
                transform: quarter_turn,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(coincident_mate(
                AssemblyMateId(20),
                canonical_planar_endpoint(FIRST, top.clone()),
                canonical_planar_endpoint(SECOND, bottom),
            )),
            CanonicalCommand::CreateAssemblyMate(coincident_mate(
                AssemblyMateId(21),
                canonical_planar_endpoint(FIRST, east.clone()),
                canonical_planar_endpoint(SECOND, east),
            )),
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AssemblyMateId(22),
                canonical_planar_endpoint(FIRST, top.clone()),
                canonical_planar_endpoint(SECOND, top),
                AssemblyMateKind::Distance { distance_mm: 5.0 },
            )),
        ]))
        .unwrap();

    let source = document.current();
    let solved = solve_rigid_assembly(&source, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(
        solved.status(),
        AssemblySolveStatus::FullyConstrained,
        "{solved:#?}"
    );
    assert_eq!(solved.remaining_dof(), 0);
    assert!(solved.conflicting_mate_ids().is_empty());
    let stale_proposal = solved.prepare_publication(&document).unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: TagId(99),
            name: "Unrelated edit".into(),
            visible: true,
        }]))
        .unwrap();
    let before_stale_commit = store_stamp(&document);
    assert!(document.commit_proposal(&stale_proposal).is_err());
    assert_eq!(store_stamp(&document), before_stale_commit);
    assert!(matches!(
        solved.prepare_publication(&document),
        Err(AssemblySolvePublishError::Stale)
    ));

    let current_solve =
        solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default()).unwrap();
    let proposal = current_solve.prepare_publication(&document).unwrap();
    let before_publish_undo = document.visible_undo_steps();
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), before_publish_undo + 1);
}

#[test]
fn conflicting_and_redundant_mates_are_deterministic_and_never_publish_failure() {
    let (mut document, top, _bottom, _east) = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AssemblyMateId(20),
                canonical_planar_endpoint(FIRST, top.clone()),
                canonical_planar_endpoint(SECOND, top.clone()),
                AssemblyMateKind::Distance { distance_mm: 5.0 },
            )),
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                AssemblyMateId(21),
                canonical_planar_endpoint(FIRST, top.clone()),
                canonical_planar_endpoint(SECOND, top),
                AssemblyMateKind::Distance { distance_mm: 10.0 },
            )),
        ]))
        .unwrap();
    let source = document.current();
    let before = store_stamp(&document);
    let failed = solve_rigid_assembly(&source, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(failed.status(), AssemblySolveStatus::OverConstrained);
    assert_eq!(
        failed.conflicting_mate_ids(),
        &[AssemblyMateId(20), AssemblyMateId(21)]
    );
    assert!(matches!(
        failed.publication_batch(&source),
        Err(AssemblySolvePublishError::SolveNotConverged(
            AssemblySolveStatus::OverConstrained
        ))
    ));
    assert_eq!(store_stamp(&document), before);
    let registry = ExactResultRegistry::accept(
        &source,
        [current_exact_package(&source, "over-constrained")],
    )
    .unwrap();
    let failed_recompute =
        recompute_rigid_assembly(&document, &registry, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(
        failed_recompute.status(),
        AssemblyRecomputeStatus::OverConstrained
    );
    assert!(matches!(
        failed_recompute.prepare_publication(&document),
        Err(AssemblyRecomputePublishError::SolveNotPublishable(
            AssemblyRecomputeStatus::OverConstrained
        ))
    ));
    assert_eq!(store_stamp(&document), before);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyMateKind {
                id: AssemblyMateId(21),
                kind: AssemblyMateKind::Distance { distance_mm: 5.0 },
            },
        ]))
        .unwrap();
    let redundant =
        solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default()).unwrap();
    assert_eq!(redundant.status(), AssemblySolveStatus::UnderConstrained);
    assert_eq!(redundant.redundant_mate_ids(), &[AssemblyMateId(21)]);
}

#[test]
fn three_occurrence_chain_is_permutation_deterministic_bounded_and_branch_safe() {
    let third = OccurrenceId(12);
    let unrelated = OccurrenceId(13);
    let unrelated_transform = Transform::from_translation(0.0, 30.0, 0.0).unwrap();
    let mut baseline = None;

    for reverse_mates in [false, true] {
        let (mut document, top, _bottom, _east) = seeded_document();
        let mut mates = [
            AssemblyMate::new(
                AssemblyMateId(20),
                canonical_planar_endpoint(FIRST, top.clone()),
                canonical_planar_endpoint(SECOND, top.clone()),
                AssemblyMateKind::Distance { distance_mm: 5.0 },
            ),
            AssemblyMate::new(
                AssemblyMateId(21),
                canonical_planar_endpoint(SECOND, top.clone()),
                canonical_planar_endpoint(third, top),
                AssemblyMateKind::Distance { distance_mm: 7.0 },
            ),
        ];
        if reverse_mates {
            mates.reverse();
        }
        let mut commands = vec![
            CanonicalCommand::CreateOccurrence {
                id: third,
                definition_id: DEFINITION,
                name: "Part C".into(),
                transform: Transform::from_translation(40.0, 0.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: unrelated,
                definition_id: DEFINITION,
                name: "Unrelated branch".into(),
                transform: unrelated_transform,
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
        ];
        commands.extend(mates.into_iter().map(CanonicalCommand::CreateAssemblyMate));
        document.apply_batch(&CommandBatch::new(commands)).unwrap();

        let source = document.current();
        let bounded = solve_rigid_assembly(
            &source,
            AssemblySolverPolicy {
                max_iterations: 1,
                ..AssemblySolverPolicy::default()
            },
        )
        .unwrap();
        assert_eq!(bounded.iterations(), 1);

        let solved = solve_rigid_assembly(&source, AssemblySolverPolicy::default()).unwrap();
        assert_eq!(solved.status(), AssemblySolveStatus::UnderConstrained);
        assert_eq!(
            solved.iterations(),
            AssemblySolverPolicy::default().max_iterations
        );
        assert_eq!(solved.remaining_dof(), 16);
        assert!(source.occurrence_is_grounded(FIRST));
        assert_eq!(
            solved
                .occurrences()
                .iter()
                .map(|occurrence| (
                    occurrence.occurrence_id(),
                    occurrence.remaining_dof(),
                    occurrence.grounded(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (FIRST, 0, true),
                (SECOND, 5, false),
                (third, 5, false),
                (unrelated, 6, false),
            ]
        );
        assert_eq!(solved.occurrence(FIRST).unwrap().remaining_dof(), 0);
        assert_eq!(solved.occurrence(SECOND).unwrap().remaining_dof(), 5);
        assert_eq!(solved.occurrence(third).unwrap().remaining_dof(), 5);
        assert_eq!(solved.occurrence(unrelated).unwrap().remaining_dof(), 6);

        let second_transform = solved.occurrence(SECOND).unwrap().transform();
        let second_matrix = second_transform.matrix();
        assert_near(second_matrix[3], 5.0);
        assert_near(second_matrix[7], 0.0);
        assert_near(second_matrix[11], 0.0);
        let third_transform = solved.occurrence(third).unwrap().transform();
        let third_matrix = third_transform.matrix();
        assert_near(third_matrix[3], 12.0);
        assert_near(third_matrix[7], 0.0);
        assert_near(third_matrix[11], 0.0);
        assert_eq!(
            solved.occurrence(unrelated).unwrap().transform(),
            unrelated_transform
        );

        let outcome = solved
            .occurrences()
            .iter()
            .map(|occurrence| {
                (
                    occurrence.occurrence_id(),
                    occurrence.transform(),
                    occurrence.remaining_dof(),
                )
            })
            .collect::<Vec<_>>();
        if let Some(expected) = &baseline {
            assert_eq!(&outcome, expected);
        } else {
            baseline = Some(outcome);
        }

        let proposal = solved.prepare_publication(&document).unwrap();
        let undo_before = document.visible_undo_steps();
        document.commit_proposal(&proposal).unwrap();
        assert_eq!(document.visible_undo_steps(), undo_before + 1);
        assert_eq!(
            document
                .current()
                .occurrence(unrelated)
                .unwrap()
                .transform(),
            unrelated_transform
        );
    }
}

#[test]
fn assembly_recompute_rebinds_current_topology_and_persists_fail_closed_diagnostics() {
    let unrelated = OccurrenceId(12);
    let unrelated_transform = Transform::from_translation(0.0, 30.0, 0.0).unwrap();
    let (mut document, top, _bottom, _east) = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: unrelated,
                definition_id: DEFINITION,
                name: "Unrelated".into(),
                transform: unrelated_transform,
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                MATE,
                canonical_planar_endpoint(FIRST, top.clone()),
                canonical_planar_endpoint(SECOND, top),
                AssemblyMateKind::Distance { distance_mm: 5.0 },
            )),
        ]))
        .unwrap();
    let initial =
        solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default()).unwrap();
    let proposal = initial.prepare_publication(&document).unwrap();
    document.commit_proposal(&proposal).unwrap();
    let solved_transform = document.current().occurrence(SECOND).unwrap().transform();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("12").unwrap(),
            },
        ]))
        .unwrap();
    let source = document.current();
    let old_lineage = source
        .assembly_mate(MATE)
        .unwrap()
        .endpoint_a()
        .reference()
        .lineage_digest
        .clone();
    let registry = ExactResultRegistry::accept(
        &source,
        [current_exact_package(&source, "after-height-edit")],
    )
    .unwrap();
    let recomputed =
        recompute_rigid_assembly(&document, &registry, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(recomputed.status(), AssemblyRecomputeStatus::Solved);
    assert_eq!(recomputed.source_revision(), source.revision_id());
    assert_eq!(recomputed.source_digest(), source.canonical_digest());
    assert_eq!(
        recomputed.mates()[0]
            .endpoint_a()
            .reference()
            .lineage_digest,
        old_lineage
    );
    assert_ne!(
        recomputed.mates()[0]
            .endpoint_a()
            .reference()
            .canonical_input_digest,
        source
            .assembly_mate(MATE)
            .unwrap()
            .endpoint_a()
            .reference()
            .canonical_input_digest
    );
    let undo_before = document.visible_undo_steps();
    let proposal = recomputed.prepare_publication(&document).unwrap();
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before + 1);
    assert_eq!(
        document
            .current()
            .occurrence(unrelated)
            .unwrap()
            .transform(),
        unrelated_transform
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("14").unwrap(),
            },
        ]))
        .unwrap();
    let before_lost_transform = document.current().occurrence(SECOND).unwrap().transform();
    let lost = recompute_rigid_assembly(
        &document,
        &ExactResultRegistry::default(),
        AssemblySolverPolicy::default(),
    )
    .unwrap();
    assert_eq!(lost.status(), AssemblyRecomputeStatus::Lost);
    assert!(lost.solve().is_none());
    let proposal = lost.prepare_publication(&document).unwrap();
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(
        document
            .current()
            .assembly_mate(MATE)
            .unwrap()
            .endpoint_a()
            .health(),
        AssemblyReferenceHealth::Lost
    );
    assert_eq!(
        document.current().occurrence(SECOND).unwrap().transform(),
        before_lost_transform
    );

    let source = document.current();
    let mut ambiguous_registry = ExactResultRegistry::default();
    ambiguous_registry
        .insert_current(&source, current_exact_package(&source, "candidate-a"))
        .unwrap();
    ambiguous_registry
        .insert_current(&source, current_exact_package(&source, "candidate-b"))
        .unwrap();
    let ambiguous = recompute_rigid_assembly(
        &document,
        &ambiguous_registry,
        AssemblySolverPolicy::default(),
    )
    .unwrap();
    assert_eq!(ambiguous.status(), AssemblyRecomputeStatus::Ambiguous);
    let proposal = ambiguous.prepare_publication(&document).unwrap();
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(
        document
            .current()
            .assembly_mate(MATE)
            .unwrap()
            .endpoint_a()
            .health(),
        AssemblyReferenceHealth::Ambiguous { candidate_count: 2 }
    );

    let current_mate = document.current().assembly_mate(MATE).unwrap().clone();
    let mut incompatible = current_mate.endpoint_a().reference().clone();
    incompatible.evaluator = "incompatible-evaluator".into();
    let broken_anchor = AssemblyMate::new(
        MATE,
        canonical_planar_endpoint(FIRST, incompatible),
        current_mate.endpoint_b().clone(),
        current_mate.kind(),
    );
    let proposal = document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::RebindAssemblyMate(broken_anchor),
        ]))
        .unwrap();
    document.commit_proposal(&proposal).unwrap();
    let source = document.current();
    let registry =
        ExactResultRegistry::accept(&source, [current_exact_package(&source, "broken-envelope")])
            .unwrap();
    let broken =
        recompute_rigid_assembly(&document, &registry, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(broken.status(), AssemblyRecomputeStatus::Broken);
    let proposal = broken.prepare_publication(&document).unwrap();
    document.commit_proposal(&proposal).unwrap();
    assert_eq!(
        document
            .current()
            .assembly_mate(MATE)
            .unwrap()
            .endpoint_a()
            .health(),
        AssemblyReferenceHealth::Broken
    );
    assert_eq!(
        document.current().occurrence(SECOND).unwrap().transform(),
        solved_transform
    );
    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(
        reopened
            .snapshot()
            .assembly_mate(MATE)
            .unwrap()
            .endpoint_a()
            .health(),
        AssemblyReferenceHealth::Broken
    );
    assert!(
        encode_semantic_state(&reopened.snapshot())
            .complete_v1()
            .contains("assembly_mate.20.endpoint_a.health=broken")
    );

    let source = document.current();
    let registry =
        ExactResultRegistry::accept(&source, [current_exact_package(&source, "stale-result")])
            .unwrap();
    let stale =
        recompute_rigid_assembly(&document, &registry, AssemblySolverPolicy::default()).unwrap();
    let stale_proposal = stale.prepare_publication(&document).unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: TagId(100),
            name: "Unrelated edit".into(),
            visible: true,
        }]))
        .unwrap();
    let before_stale = store_stamp(&document);
    let stale_error = match document.commit_proposal(&stale_proposal) {
        Ok(_) => panic!("stale assembly recompute unexpectedly committed"),
        Err(error) => error,
    };
    assert!(
        matches!(
            stale_error,
            ProposalCommitError::Stale(_)
                | ProposalCommitError::Canonical(CanonicalError::StaleAssemblySolve)
                | ProposalCommitError::Preparation(ProposalPrepareError::Canonical(
                    CanonicalError::StaleAssemblySolve
                ))
        ),
        "unexpected stale error: {stale_error:?}"
    );
    assert_eq!(store_stamp(&document), before_stale);
    assert!(matches!(
        stale.publication_batch(&document.current()),
        Err(AssemblyRecomputePublishError::Stale)
    ));
    assert_eq!(
        document
            .current()
            .occurrence(unrelated)
            .unwrap()
            .transform(),
        unrelated_transform
    );
}

#[test]
fn assembly_recompute_round_trips_rebind_and_controlled_topology_loss() {
    let unrelated = OccurrenceId(12);
    let unrelated_transform = Transform::from_translation(0.0, 30.0, 0.0).unwrap();
    let (mut document, top, _bottom, _east) = seeded_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: unrelated,
                definition_id: DEFINITION,
                name: "Unrelated".into(),
                transform: unrelated_transform,
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: FIRST,
                grounded: true,
            },
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                MATE,
                canonical_planar_endpoint(FIRST, top.clone()),
                canonical_planar_endpoint(SECOND, top),
                AssemblyMateKind::Distance { distance_mm: 5.0 },
            )),
        ]))
        .unwrap();
    let initial =
        solve_rigid_assembly(&document.current(), AssemblySolverPolicy::default()).unwrap();
    document
        .commit_proposal(&initial.prepare_publication(&document).unwrap())
        .unwrap();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("12").unwrap(),
            },
        ]))
        .unwrap();
    let before_rebind = document.current();
    let before_rebind_state = encode_semantic_state(&before_rebind)
        .complete_v1()
        .to_owned();
    let before_rebind_digest = before_rebind.canonical_digest();
    let before_rebind_reference = before_rebind
        .assembly_mate(MATE)
        .unwrap()
        .endpoint_a()
        .reference()
        .clone();
    let registry = ExactResultRegistry::accept(
        &before_rebind,
        [current_exact_package(&before_rebind, "verify-rebind")],
    )
    .unwrap();
    let recomputed =
        recompute_rigid_assembly(&document, &registry, AssemblySolverPolicy::default()).unwrap();
    assert_eq!(recomputed.status(), AssemblyRecomputeStatus::Solved);
    assert_eq!(recomputed.source_revision(), before_rebind.revision_id());
    assert_eq!(recomputed.source_digest(), before_rebind_digest);

    let undo_before = document.visible_undo_steps();
    document
        .commit_proposal(&recomputed.prepare_publication(&document).unwrap())
        .unwrap();
    let rebound = document.current();
    let rebound_digest = rebound.canonical_digest();
    let rebound_state = encode_semantic_state(&rebound).complete_v1().to_owned();
    let rebound_reference = rebound
        .assembly_mate(MATE)
        .unwrap()
        .endpoint_a()
        .reference();
    assert_eq!(document.visible_undo_steps(), undo_before + 1);
    assert_eq!(
        rebound_reference.lineage_digest,
        before_rebind_reference.lineage_digest
    );
    assert_ne!(
        rebound_reference.canonical_input_digest,
        before_rebind_reference.canonical_input_digest
    );
    assert_eq!(
        rebound.occurrence(unrelated).unwrap().transform(),
        unrelated_transform
    );
    let reopened = persistence::load(&persistence::save(&rebound)).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), rebound_digest);
    assert_eq!(
        encode_semantic_state(&reopened.snapshot()).complete_v1(),
        rebound_state
    );

    assert_eq!(
        document.undo().unwrap().canonical_digest(),
        before_rebind_digest
    );
    assert_eq!(
        encode_semantic_state(&document.current()).complete_v1(),
        before_rebind_state
    );
    assert_eq!(document.redo().unwrap().canonical_digest(), rebound_digest);
    assert_eq!(
        encode_semantic_state(&document.current()).complete_v1(),
        rebound_state
    );

    let mut reopened_document = match reopened.into_editable() {
        Ok(document) => document,
        Err(_) => panic!("assembly recompute save/open unexpectedly requires review"),
    };
    let reopened_snapshot = reopened_document.current();
    let reopened_registry = ExactResultRegistry::accept(
        &reopened_snapshot,
        [current_exact_package(&reopened_snapshot, "verify-rebind")],
    )
    .unwrap();
    let reopened_recompute = recompute_rigid_assembly(
        &reopened_document,
        &reopened_registry,
        AssemblySolverPolicy::default(),
    )
    .unwrap();
    assert_eq!(reopened_recompute.status(), AssemblyRecomputeStatus::Solved);
    assert!(matches!(
        reopened_recompute.publication_batch(&reopened_snapshot),
        Err(AssemblyRecomputePublishError::NoCanonicalChanges)
    ));

    let dimension_edit = reopened_document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: EXTRUSION,
                dimension: Dimension::from_decimal("14").unwrap(),
            },
        ]))
        .unwrap();
    reopened_document.commit_proposal(&dimension_edit).unwrap();
    let before_topology_change = reopened_document.current();
    let current_mate = before_topology_change.assembly_mate(MATE).unwrap().clone();
    let removed_role = ExactFaceRole::PocketFloor;
    let mut removed_reference = current_mate.endpoint_a().reference().clone();
    removed_reference.semantic_role = removed_role.semantic_role().into();
    removed_reference.source_element_id = removed_role.source_element_id().into();
    removed_reference.expected_type = removed_role.expected_type().into();
    removed_reference.lineage_digest = canonical_reference_lineage_digest(
        before_topology_change.document_id(),
        EXTRUSION,
        removed_role.semantic_role(),
        removed_role.source_element_id(),
        removed_role.expected_type(),
    );
    let topology_edit = reopened_document
        .prepare_proposal(CommandBatch::new(vec![
            CanonicalCommand::RebindAssemblyMate(AssemblyMate::new(
                MATE,
                canonical_planar_endpoint(FIRST, removed_reference),
                current_mate.endpoint_b().clone(),
                current_mate.kind(),
            )),
        ]))
        .unwrap();
    reopened_document.commit_proposal(&topology_edit).unwrap();
    let topology_changed = reopened_document.current();
    let topology_changed_digest = topology_changed.canonical_digest();
    let retained_transform = topology_changed.occurrence(SECOND).unwrap().transform();
    let topology_registry = ExactResultRegistry::accept(
        &topology_changed,
        [current_exact_package(&topology_changed, "top-role-removed")],
    )
    .unwrap();
    let lost = recompute_rigid_assembly(
        &reopened_document,
        &topology_registry,
        AssemblySolverPolicy::default(),
    )
    .unwrap();
    assert_eq!(lost.status(), AssemblyRecomputeStatus::Lost);
    assert!(lost.solve().is_none());
    let lost_undo_before = reopened_document.visible_undo_steps();
    reopened_document
        .commit_proposal(&lost.prepare_publication(&reopened_document).unwrap())
        .unwrap();
    let lost_snapshot = reopened_document.current();
    let lost_digest = lost_snapshot.canonical_digest();
    assert_eq!(reopened_document.visible_undo_steps(), lost_undo_before + 1);
    assert_eq!(
        lost_snapshot
            .assembly_mate(MATE)
            .unwrap()
            .endpoint_a()
            .health(),
        AssemblyReferenceHealth::Lost
    );
    assert_eq!(
        lost_snapshot.occurrence(SECOND).unwrap().transform(),
        retained_transform
    );
    assert_eq!(
        lost_snapshot.occurrence(unrelated).unwrap().transform(),
        unrelated_transform
    );
    let reopened_lost = persistence::load(&persistence::save(&lost_snapshot)).unwrap();
    assert_eq!(reopened_lost.snapshot().canonical_digest(), lost_digest);
    assert_eq!(
        reopened_document.undo().unwrap().canonical_digest(),
        topology_changed_digest
    );
    assert_eq!(
        reopened_document.redo().unwrap().canonical_digest(),
        lost_digest
    );
}
