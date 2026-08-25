use ketchup_core::document::{
    CanonicalCommand, CommandBatch, Dimension, DocumentStore, FeatureKind, ProposalContext,
    Transform,
};
use ketchup_core::exact_product::{
    ExactFaceRole, ExactFeatureChainRequest, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use ketchup_core::release_capstone::{
    CapstoneOutputKind, CapstoneStage, CapstoneStageEvidence, ReleaseCapstoneContract,
    ReleaseCapstoneContractError,
};
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadPocketOperation, PadSpec, PocketSpec, PrincipalPlane,
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchPointKind, SketchPointRef, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};

fn point(entity: u64, point: SketchPointKind) -> SketchPointRef {
    SketchPointRef {
        entity: SketchEntityId(entity),
        point,
    }
}

fn fixed_rectangle(workplane: ketchup_core::document::FeatureId) -> SketchSpec {
    let corners = [[-60.0, -40.0], [60.0, -40.0], [60.0, 40.0], [-60.0, 40.0]];
    let entities = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: corners[index],
            end_mm: corners[(index + 1) % 4],
        })
        .collect();
    let mut constraints = Vec::new();
    for index in 0..4 {
        let entity = index as u64 + 1;
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 1),
            kind: SketchConstraintKind::FixedPoint {
                point: point(entity, SketchPointKind::Start),
                position_mm: corners[index],
            },
        });
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 2),
            kind: SketchConstraintKind::FixedPoint {
                point: point(entity, SketchPointKind::End),
                position_mm: corners[(index + 1) % 4],
            },
        });
    }
    SketchSpec {
        workplane,
        entities,
        constraints,
    }
}

fn fixed_circle(workplane: ketchup_core::document::FeatureId, radius_mm: f64) -> SketchSpec {
    SketchSpec {
        workplane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 0.0],
            radius_mm,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: Dimension::new(radius_mm.to_string(), radius_mm).unwrap(),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Center),
                    position_mm: [0.0, 0.0],
                },
            },
        ],
    }
}

fn add_fastener_definition(
    commands: &mut Vec<CanonicalCommand>,
    definition: ketchup_core::document::DefinitionId,
    body: ketchup_core::document::BodyId,
    features: ketchup_core::release_capstone::CapstoneFastenerFeatureIds,
    name: &str,
    height_mm: &str,
) {
    let sketch = fixed_circle(features.principal_workplane, 6.0);
    let region = sketch.solved_regions().unwrap()[0].id;
    commands.extend([
        CanonicalCommand::CreateDefinition {
            id: definition,
            name: name.into(),
        },
        CanonicalCommand::CreateBody {
            definition_id: definition,
            id: body,
            name: format!("{name} body"),
            visible: true,
        },
        CanonicalCommand::SetActiveBody {
            definition_id: definition,
            id: body,
        },
        CanonicalCommand::CreateFeature {
            id: features.principal_workplane,
            definition_id: definition,
            name: "XY".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
        },
        CanonicalCommand::CreateFeature {
            id: features.sketch,
            definition_id: definition,
            name: "Fastener circle".into(),
            kind: FeatureKind::Sketch(sketch),
        },
        CanonicalCommand::CreateFeature {
            id: features.pad,
            definition_id: definition,
            name: "Fastener pad".into(),
            kind: FeatureKind::Pad(PadSpec {
                sketch: features.sketch,
                region,
                direction: FeatureDirection::AlongNormal,
                extent: FeatureExtent::Blind(Dimension::from_decimal(height_mm).unwrap()),
            }),
        },
    ]);
}

fn authored_fixture() -> (ReleaseCapstoneContract, DocumentStore) {
    let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
    let plate = contract.plate_feature_ids;
    let base_sketch = fixed_rectangle(plate.principal_workplane);
    let base_region = base_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: contract.plate_definition_id,
                name: "Capstone plate".into(),
            },
            CanonicalCommand::CreateBody {
                definition_id: contract.plate_definition_id,
                id: contract.plate_body_id,
                name: "Plate body".into(),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: contract.plate_definition_id,
                id: contract.plate_body_id,
            },
            CanonicalCommand::CreateFeature {
                id: plate.principal_workplane,
                definition_id: contract.plate_definition_id,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: plate.base_sketch,
                definition_id: contract.plate_definition_id,
                name: "Plate rectangle".into(),
                kind: FeatureKind::Sketch(base_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: plate.pad,
                definition_id: contract.plate_definition_id,
                name: "Plate pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: plate.base_sketch,
                    region: base_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(Dimension::from_decimal("8").unwrap()),
                }),
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let request =
        ExactFeatureChainRequest::from_snapshot(&snapshot, contract.plate_definition_id).unwrap();
    let frame = request.workplane_frame_bits.unwrap().map(f64::from_bits);
    let height_mm = f64::from_bits(request.height_bits);
    let top_frame = WorkplaneFrame {
        origin_mm: [
            frame[0] + frame[9] * height_mm,
            frame[1] + frame[10] * height_mm,
            frame[2] + frame[11] * height_mm,
        ],
        x_axis: [frame[3], frame[4], frame[5]],
        y_axis: [frame[6], frame[7], frame[8]],
        normal: [frame[9], frame[10], frame[11]],
    };
    let reference_evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                snapshot.document_id(),
                plate.pad,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("capstone-plate-{role:?}"),
        )
    });
    let package = build_box_render_package(
        &request,
        "capstone-plate-exact-input".into(),
        "capstone-plate-pad-result".into(),
        "test-occt".into(),
        "r0".into(),
        request.expected_bounds_mm(),
        reference_evidence,
    )
    .unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    for reference in package.references {
        document
            .register_exact_reference_evidence(reference)
            .unwrap();
    }

    assert!(top.has_valid_lineage());
    assert_eq!(
        document
            .current()
            .exact_reference_by_lineage(&top.lineage_digest),
        Some(&top)
    );
    let pocket_sketch = fixed_circle(plate.face_workplane, 10.0);
    let pocket_region = pocket_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: plate.face_workplane,
                definition_id: contract.plate_definition_id,
                name: "Plate top face".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: top_frame,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: plate.pocket_sketch,
                definition_id: contract.plate_definition_id,
                name: "Plate pocket circle".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
        ]))
        .unwrap();
    let pocket = document
        .plan_pad_pocket(
            plate.pocket,
            contract.plate_definition_id,
            "Plate pocket",
            PadPocketOperation::Pocket(PocketSpec {
                target: plate.pad,
                sketch: plate.pocket_sketch,
                region: pocket_region,
                support: Box::new(top),
                direction: FeatureDirection::OppositeNormal,
                extent: FeatureExtent::Blind(Dimension::from_decimal("4").unwrap()),
            }),
            ProposalContext::canonical_preview(),
        )
        .unwrap();
    document.commit_proposal(&pocket).unwrap();

    let mut commands = Vec::new();
    add_fastener_definition(
        &mut commands,
        contract.shared_definition_id,
        contract.shared_body_id,
        contract.shared_feature_ids,
        "Shared fastener",
        "30",
    );
    add_fastener_definition(
        &mut commands,
        contract.replacement_definition_id,
        contract.replacement_body_id,
        contract.replacement_feature_ids,
        "Compatible target fastener",
        "24",
    );
    commands.extend([
        CanonicalCommand::CreateOccurrence {
            id: contract.plate_occurrence_id,
            definition_id: contract.plate_definition_id,
            name: "Plate".into(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::CreateOccurrence {
            id: contract.first_shared_occurrence_id,
            definition_id: contract.shared_definition_id,
            name: "Fastener A".into(),
            transform: Transform::from_translation(-30.0, 0.0, 8.0).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::CreateOccurrence {
            id: contract.second_shared_occurrence_id,
            definition_id: contract.shared_definition_id,
            name: "Fastener B".into(),
            transform: Transform::from_translation(30.0, 0.0, 8.0).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        },
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    (contract, document)
}

fn structural_evidence(contract: &ReleaseCapstoneContract) -> Vec<CapstoneStageEvidence> {
    let revisions = [0, 3, 6, 7, 8, 9, 9];
    contract
        .stages
        .iter()
        .enumerate()
        .map(|(index, requirement)| {
            let evidence_stage = if requirement.stage == CapstoneStage::Reopened {
                CapstoneStage::ComponentReplaced
            } else {
                requirement.stage
            };
            CapstoneStageEvidence {
                stage: requirement.stage,
                document_id: ketchup_core::document::DocumentId(77),
                source_revision: revisions[index],
                source_digest: format!("digest-{evidence_stage:?}"),
                lineage: requirement.lineage.clone(),
                output_fingerprints: requirement
                    .required_outputs
                    .iter()
                    .map(|kind| (*kind, format!("output-{evidence_stage:?}-{kind:?}")))
                    .collect(),
            }
        })
        .collect()
}

#[test]
fn declared_fixture_reconstructs_with_real_feature_roles_and_lossless_save_open() {
    let (contract, document) = authored_fixture();
    let snapshot = document.current();
    let authored = &contract.stages[1].lineage;

    for definition_id in &authored.definition_ids {
        assert!(snapshot.definition(*definition_id).is_some());
    }
    for body_id in &authored.body_ids {
        assert!(authored.definition_ids.iter().any(|definition_id| {
            snapshot
                .definition(*definition_id)
                .unwrap()
                .body(*body_id)
                .is_some()
        }));
    }
    for feature_id in &authored.feature_ids {
        assert!(snapshot.feature(*feature_id).is_some());
    }
    for occurrence_id in &authored.occurrence_ids {
        assert!(snapshot.occurrence(*occurrence_id).is_some());
    }
    assert!(matches!(
        snapshot
            .feature(contract.plate_feature_ids.principal_workplane)
            .unwrap()
            .kind(),
        FeatureKind::Workplane(_)
    ));
    assert!(matches!(
        snapshot
            .feature(contract.plate_feature_ids.base_sketch)
            .unwrap()
            .kind(),
        FeatureKind::Sketch(_)
    ));
    assert!(matches!(
        snapshot
            .feature(contract.plate_feature_ids.pad)
            .unwrap()
            .kind(),
        FeatureKind::Pad(_)
    ));
    assert!(matches!(
        snapshot
            .feature(contract.plate_feature_ids.face_workplane)
            .unwrap()
            .kind(),
        FeatureKind::Workplane(_)
    ));
    assert!(matches!(
        snapshot
            .feature(contract.plate_feature_ids.pocket_sketch)
            .unwrap()
            .kind(),
        FeatureKind::Sketch(_)
    ));
    assert!(matches!(
        snapshot
            .feature(contract.plate_feature_ids.pocket)
            .unwrap()
            .kind(),
        FeatureKind::SketchPocket(_)
    ));
    for features in [
        contract.shared_feature_ids,
        contract.replacement_feature_ids,
    ] {
        assert!(matches!(
            snapshot
                .feature(features.principal_workplane)
                .unwrap()
                .kind(),
            FeatureKind::Workplane(_)
        ));
        assert!(matches!(
            snapshot.feature(features.sketch).unwrap().kind(),
            FeatureKind::Sketch(_)
        ));
        assert!(matches!(
            snapshot.feature(features.pad).unwrap().kind(),
            FeatureKind::Pad(_)
        ));
    }
    assert_eq!(
        snapshot
            .occurrence(contract.first_shared_occurrence_id)
            .unwrap()
            .definition_id(),
        contract.shared_definition_id
    );
    assert_eq!(
        snapshot
            .occurrence(contract.second_shared_occurrence_id)
            .unwrap()
            .definition_id(),
        contract.shared_definition_id
    );

    let bytes = persistence::save(&snapshot);
    let reopened = persistence::load(&bytes).unwrap().snapshot();
    assert_eq!(reopened.document_id(), snapshot.document_id());
    assert_eq!(reopened.revision_id(), snapshot.revision_id());
    assert_eq!(reopened.canonical_digest(), snapshot.canonical_digest());
    assert_eq!(persistence::save(&reopened), bytes);
    for feature_id in &authored.feature_ids {
        assert_eq!(reopened.feature(*feature_id), snapshot.feature(*feature_id));
    }
}

#[test]
fn verifier_replays_permutations_and_invalid_evidence_observationally() {
    let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
    let evidence = structural_evidence(&contract);
    contract.validate_evidence(&evidence).unwrap();

    let mut permuted_contract = contract.clone();
    permuted_contract.required_capabilities.reverse();
    permuted_contract.observational_paths.reverse();
    permuted_contract.refusal_paths.reverse();
    for requirement in &mut permuted_contract.stages {
        requirement.lineage.definition_ids.reverse();
        requirement.lineage.body_ids.reverse();
        requirement.lineage.feature_ids.reverse();
        requirement.lineage.occurrence_ids.reverse();
        requirement.required_outputs.reverse();
    }
    permuted_contract.validate_evidence(&evidence).unwrap();
    assert_eq!(
        permuted_contract.contract_fingerprint(),
        contract.contract_fingerprint()
    );

    let mut permuted = evidence.clone();
    for stage in &mut permuted {
        if !stage.lineage.definition_ids.is_empty() {
            stage.lineage.definition_ids.rotate_left(1);
        }
        stage.lineage.body_ids.reverse();
        stage.lineage.feature_ids.reverse();
        stage.lineage.occurrence_ids.reverse();
        stage.output_fingerprints.reverse();
    }
    contract.validate_evidence(&permuted).unwrap();

    let (contract, document) = authored_fixture();
    let canonical_before = document.current().canonical_digest();
    let revision_before = document.current().revision_id();
    let undo_before = document.visible_undo_steps();
    let redo_before = document.visible_redo_steps();
    let evidence_before = evidence.clone();

    for invalid in 0..13 {
        let mut candidate = evidence.clone();
        let expected = match invalid {
            0 => {
                candidate.pop();
                ReleaseCapstoneContractError::IncompleteEvidence
            }
            1 => {
                candidate[2].document_id = ketchup_core::document::DocumentId(999);
                ReleaseCapstoneContractError::CrossDocument
            }
            2 => {
                candidate[2].source_digest.clear();
                ReleaseCapstoneContractError::InvalidStageLineage(CapstoneStage::AssemblySolved)
            }
            3 => {
                candidate[2].lineage.definition_ids.pop();
                ReleaseCapstoneContractError::InvalidStageLineage(CapstoneStage::AssemblySolved)
            }
            4 => {
                candidate[2].lineage.definition_ids[1] = candidate[2].lineage.definition_ids[0];
                ReleaseCapstoneContractError::InvalidStageLineage(CapstoneStage::AssemblySolved)
            }
            5 => {
                candidate[2].lineage.body_ids[1] = candidate[2].lineage.body_ids[0];
                ReleaseCapstoneContractError::InvalidStageLineage(CapstoneStage::AssemblySolved)
            }
            6 => {
                candidate[2].lineage.feature_ids[1] = candidate[2].lineage.feature_ids[0];
                ReleaseCapstoneContractError::InvalidStageLineage(CapstoneStage::AssemblySolved)
            }
            7 => {
                candidate[2].lineage.occurrence_ids[1] = candidate[2].lineage.occurrence_ids[0];
                ReleaseCapstoneContractError::InvalidStageLineage(CapstoneStage::AssemblySolved)
            }
            8 => {
                candidate[2].output_fingerprints.pop();
                ReleaseCapstoneContractError::InvalidOutputContract(CapstoneStage::AssemblySolved)
            }
            9 => {
                candidate[2].output_fingerprints[1].0 = candidate[2].output_fingerprints[0].0;
                ReleaseCapstoneContractError::InvalidOutputContract(CapstoneStage::AssemblySolved)
            }
            10 => {
                candidate[2].output_fingerprints[0].1.clear();
                ReleaseCapstoneContractError::MissingFingerprint(
                    CapstoneStage::AssemblySolved,
                    CapstoneOutputKind::ExactBrep,
                )
            }
            11 => {
                candidate[6].source_digest = "stale-reopen".into();
                ReleaseCapstoneContractError::CanonicalReplayMismatch
            }
            _ => {
                candidate[6].output_fingerprints[7].1 = "stale-stl".into();
                ReleaseCapstoneContractError::OutputReplayMismatch
            }
        };
        assert_eq!(contract.validate_evidence(&candidate), Err(expected));
        assert_eq!(document.current().canonical_digest(), canonical_before);
        assert_eq!(document.current().revision_id(), revision_before);
        assert_eq!(document.visible_undo_steps(), undo_before);
        assert_eq!(document.visible_redo_steps(), redo_before);
        assert_eq!(evidence, evidence_before);
    }
}
