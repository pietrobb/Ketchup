use std::collections::BTreeSet;

use ketchup_application::model_query::{EntityKind, ModelQuery};
use ketchup_application::plan_assistant_cad_edit_program as plan;
use ketchup_core::assistant_sidecar::*;
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, DocumentStore,
    FeatureId, FeatureKind, OccurrenceId, SpatialPathSegment, Transform,
};
use ketchup_core::exact_brep_graph::{
    EXACT_BREP_GRAPH_SCHEMA_V14, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry,
};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_core::persistence::{ContainerData, LoadOutcome, load, save_document_store};
use ketchup_core::sketch::{
    PrincipalPlane, SketchEntity, SketchEntityId, SketchSpec, WorkplaneSpec,
};

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
fn public_cubic_bezier_profile_plans_as_an_exact_editable_part() {
    let document = DocumentStore::new();
    let input = program(vec![AssistantCadEditOperation::CreatePart {
        name: "Bezier enclosure".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![
            AssistantSketchEntity::CubicBezier {
                id: 1,
                start_mm: [-20.0, 0.0],
                control_1_mm: [-20.0, 15.0],
                control_2_mm: [20.0, 15.0],
                end_mm: [20.0, 0.0],
            },
            AssistantSketchEntity::CubicBezier {
                id: 2,
                start_mm: [20.0, 0.0],
                control_1_mm: [20.0, -15.0],
                control_2_mm: [-20.0, -15.0],
                end_mm: [-20.0, 0.0],
            },
        ],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 8.0 },
        translation_mm: [0.0; 3],
        rotation: None,
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let FeatureKind::Sketch(sketch) = preview.feature(FeatureId(2)).unwrap().kind() else {
        panic!("planned profile must remain an editable sketch");
    };
    assert_eq!(sketch.entities.len(), 2);
    assert!(sketch.entities.iter().all(|entity| matches!(
        entity,
        ketchup_core::sketch::SketchEntity::CubicBezier { .. }
    )));
    assert!(ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), FeatureId(3)).is_ok());
}

#[test]
fn public_large_ellipse_has_a_visible_bounded_deviation_and_reopens_exactly() {
    let mut document = DocumentStore::new();
    let baseline_digest = document.current().canonical_digest();
    let ellipse = |radius_x_mm, radius_y_mm, maximum_deviation_mm| {
        program(vec![AssistantCadEditOperation::CreatePart {
            name: "Bounded ellipse".into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Ellipse {
                segment_ids: [1, 2, 3, 4],
                center_mm: [0.0, 0.0],
                radius_x_mm,
                radius_y_mm,
                rotation_degrees: 32.0,
                maximum_deviation_mm,
            }],
            constraints: Vec::new(),
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 8.0 },
            translation_mm: [0.0; 3],
            rotation: None,
        }])
    };
    let large = ellipse(200_000.0, 80_000.0, 55.0);
    let serialized = serde_json::to_value(&large).unwrap();
    assert_eq!(
        serialized["operations"][0]["entities"][0]["maximum_deviation_mm"],
        55.0
    );
    assert_eq!(large.validate(), Ok(()));
    let mut too_strict = serialized;
    too_strict["operations"][0]["entities"][0]["maximum_deviation_mm"] = serde_json::json!(54.0);
    let too_strict: AssistantCadEditProgram = serde_json::from_value(too_strict).unwrap();
    assert!(too_strict.validate().is_err());
    assert_eq!(document.current().canonical_digest(), baseline_digest);

    let input = ellipse(24.0, 9.6, 0.007);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let FeatureKind::Sketch(sketch) = preview.feature(FeatureId(2)).unwrap().kind() else {
        panic!("planned ellipse must remain an editable sketch");
    };
    assert_eq!(sketch.entities.len(), 4);
    let (sin, cos) = 32.0_f64.to_radians().sin_cos();
    let mut maximum_sampled_deviation = 0.0_f64;
    for entity in &sketch.entities {
        let ketchup_core::sketch::SketchEntity::CubicBezier {
            start_mm,
            control_1_mm,
            control_2_mm,
            end_mm,
            ..
        } = entity
        else {
            panic!("ellipse approximation must remain editable cubic geometry");
        };
        for step in 0..=4_096 {
            let parameter = f64::from(step) / 4_096.0;
            let inverse = 1.0 - parameter;
            let point = [0, 1].map(|axis| {
                inverse.powi(3) * start_mm[axis]
                    + 3.0 * inverse.powi(2) * parameter * control_1_mm[axis]
                    + 3.0 * inverse * parameter.powi(2) * control_2_mm[axis]
                    + parameter.powi(3) * end_mm[axis]
            });
            let local = [
                cos * point[0] + sin * point[1],
                -sin * point[0] + cos * point[1],
            ];
            let normalized_radius = (local[0] / 24.0).hypot(local[1] / 9.6);
            let projected = [local[0] / normalized_radius, local[1] / normalized_radius];
            maximum_sampled_deviation = maximum_sampled_deviation.max(
                (point[0] - (cos * projected[0] - sin * projected[1]))
                    .hypot(point[1] - (sin * projected[0] + cos * projected[1])),
            );
        }
    }
    assert!(maximum_sampled_deviation <= 0.007);
    let large_radius_deviation = maximum_sampled_deviation * (200_000.0 / 24.0);
    assert!(large_radius_deviation > 50.0);
    assert!(large_radius_deviation <= 55.0);
    let regions = sketch.solved_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].entity_ids.len(), 4);
    assert!(ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), FeatureId(3)).is_ok());

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_ne!(committed_digest, baseline_digest);
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), baseline_digest);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("bounded ellipse must reopen as an editable document");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
}

#[test]
fn public_rotated_rounded_rectangle_plans_as_an_exact_editable_profile() {
    let document = DocumentStore::new();
    let input = program(vec![AssistantCadEditOperation::CreatePart {
        name: "Rotated rounded rectangle".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::RoundedRectangle {
            segment_ids: [1, 2, 3, 4, 5, 6, 7, 8],
            center_mm: [-4.0, 6.0],
            width_mm: 48.0,
            height_mm: 28.0,
            corner_radius_mm: 6.0,
            rotation_degrees: 27.0,
        }],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 9.0 },
        translation_mm: [0.0; 3],
        rotation: None,
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let FeatureKind::Sketch(sketch) = preview.feature(FeatureId(2)).unwrap().kind() else {
        panic!("planned rounded rectangle must remain an editable sketch");
    };
    assert_eq!(sketch.entities.len(), 8);
    assert_eq!(
        sketch
            .entities
            .iter()
            .filter(|entity| matches!(entity, ketchup_core::sketch::SketchEntity::Line { .. }))
            .count(),
        4
    );
    assert_eq!(
        sketch
            .entities
            .iter()
            .filter(|entity| matches!(entity, ketchup_core::sketch::SketchEntity::Arc { .. }))
            .count(),
        4
    );
    let regions = sketch.solved_regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].entity_ids.len(), 8);
    assert!(ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), FeatureId(3)).is_ok());
}

#[test]
fn public_profile_copies_expand_to_transformed_editable_closed_profiles() {
    let document = seeded();
    let copy =
        |entity_ids, translation_mm, rotation_degrees, uniform_scale| AssistantSketchProfileCopy {
            entity_ids,
            translation_mm,
            rotation_degrees,
            uniform_scale,
        };
    let input = program(vec![AssistantCadEditOperation::CreateSketch {
        definition_id: 1,
        name: "Transformed profile copies".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::ProfileCopies {
            source_entities: vec![AssistantSketchEntity::RoundedRectangle {
                segment_ids: [1, 2, 3, 4, 5, 6, 7, 8],
                center_mm: [0.0, 0.0],
                width_mm: 20.0,
                height_mm: 12.0,
                corner_radius_mm: 2.0,
                rotation_degrees: 0.0,
            }],
            copies: vec![
                copy((11..=18).collect(), [-18.0, 4.0], 25.0, 1.0),
                copy((21..=28).collect(), [18.0, -5.0], -30.0, 0.7),
            ],
        }],
        constraints: Vec::new(),
    }]);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let FeatureKind::Sketch(sketch) = preview.feature(FeatureId(5)).unwrap().kind() else {
        panic!("profile copies must remain one editable sketch");
    };
    assert_eq!(sketch.entities.len(), 16);
    assert_eq!(sketch.solved_regions().unwrap().len(), 2);
    assert_eq!(
        sketch
            .entities
            .iter()
            .map(|entity| entity.id().0)
            .collect::<Vec<_>>(),
        (11..=18).chain(21..=28).collect::<Vec<_>>()
    );
    assert!(sketch.entities.iter().all(|entity| matches!(
        entity,
        ketchup_core::sketch::SketchEntity::Line { .. }
            | ketchup_core::sketch::SketchEntity::Arc { .. }
    )));

    let exact_document = DocumentStore::new();
    let exact_input = program(vec![AssistantCadEditOperation::CreatePart {
        name: "Transformed exact profile".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::ProfileCopies {
            source_entities: vec![AssistantSketchEntity::RoundedRectangle {
                segment_ids: [1, 2, 3, 4, 5, 6, 7, 8],
                center_mm: [0.0, 0.0],
                width_mm: 20.0,
                height_mm: 12.0,
                corner_radius_mm: 2.0,
                rotation_degrees: 0.0,
            }],
            copies: vec![copy((31..=38).collect(), [7.0, -3.0], 40.0, 1.25)],
        }],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 8.0 },
        translation_mm: [0.0; 3],
        rotation: None,
    }]);
    let exact_batch = plan(
        &exact_document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &exact_input,
    )
    .unwrap();
    let exact_preview = exact_document.preview_batch(&exact_batch).unwrap();
    assert!(ExactBRepGraph::from_snapshot(&exact_preview, DefinitionId(1), FeatureId(3)).is_ok());
}

#[test]
fn parametric_sketch_outputs_build_a_complex_loft_graph_without_manual_points() {
    let mut document = DocumentStore::new();
    let baseline_digest = document.current().canonical_digest();
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let definition = output(0, AssistantCadProgramFeatureOutput::Definition);
    let sketch = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(output(
            operation_index,
            AssistantCadProgramFeatureOutput::SketchFeature,
        ))
    };
    let input = program(vec![
        part(),
        AssistantCadEditOperation::CreateProgramSketch {
            definition,
            name: "Dimensioned circular section".into(),
            workplane: AssistantWorkplaneSpec::Frame {
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 1.0, 0.0],
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 13.0,
            }],
            constraints: vec![AssistantSketchConstraint::Radius {
                id: 1,
                entity_id: 1,
                value_mm: 13.0,
            }],
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition,
            name: "Rounded middle section".into(),
            workplane: AssistantWorkplaneSpec::Frame {
                origin_mm: [2.0, -1.0, -8.0],
                x_axis: [0.866_025_403_784, 0.5, 0.0],
                y_axis: [-0.5, 0.866_025_403_784, 0.0],
            },
            entities: vec![AssistantSketchEntity::RoundedRectangle {
                segment_ids: [1, 2, 3, 4, 5, 6, 7, 8],
                center_mm: [0.0, 0.0],
                width_mm: 30.0,
                height_mm: 18.0,
                corner_radius_mm: 4.0,
                rotation_degrees: 18.0,
            }],
            constraints: Vec::new(),
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition,
            name: "Transformed elliptic section".into(),
            workplane: AssistantWorkplaneSpec::Frame {
                origin_mm: [4.0, 2.0, -15.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 0.939_692_620_786, 0.342_020_143_326],
            },
            entities: vec![AssistantSketchEntity::ProfileCopies {
                source_entities: vec![AssistantSketchEntity::Ellipse {
                    segment_ids: [1, 2, 3, 4],
                    center_mm: [0.0, 0.0],
                    radius_x_mm: 9.0,
                    radius_y_mm: 6.0,
                    rotation_degrees: 0.0,
                    maximum_deviation_mm: 0.003,
                }],
                copies: vec![AssistantSketchProfileCopy {
                    entity_ids: vec![11, 12, 13, 14],
                    translation_mm: [3.0, -2.0],
                    rotation_degrees: -27.0,
                    uniform_scale: 0.85,
                }],
            }],
            constraints: Vec::new(),
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Parametric multi-section loft".into(),
            feature: AssistantCadBodyFeature::Loft {
                sections: vec![
                    AssistantCadLoftSection {
                        profile_feature_id: sketch(1),
                        elevation_mm: 0.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: sketch(2),
                        elevation_mm: 28.0,
                    },
                    AssistantCadLoftSection {
                        profile_feature_id: sketch(3),
                        elevation_mm: 55.0,
                    },
                ],
            },
        },
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let mut holed = input.clone();
    let AssistantCadEditOperation::CreateProgramSketch {
        entities,
        constraints,
        ..
    } = &mut holed.operations[1]
    else {
        unreachable!("the first Loft section is a program sketch")
    };
    *entities = vec![
        AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 13.0,
        },
        AssistantSketchEntity::Circle {
            id: 2,
            center_mm: [0.0, 0.0],
            radius_mm: 4.0,
        },
    ];
    constraints.clear();
    assert!(
        plan(
            &DocumentStore::new(),
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &holed,
        )
        .is_err(),
        "Loft sections with holes must fail closed during public planning"
    );
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let loft_feature = preview
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::Loft { .. }))
        .unwrap();
    let graph =
        ExactBRepGraph::from_snapshot(&preview, DefinitionId(1), loft_feature.id()).unwrap();
    assert_eq!(graph.schema, EXACT_BREP_GRAPH_SCHEMA_V14);
    assert!(
        graph
            .profiles
            .iter()
            .map(|profile| profile.frame_bits)
            .collect::<BTreeSet<_>>()
            .len()
            == 3
    );
    assert!(matches!(
        graph.nodes[0].operation,
        ExactBRepOperation::Loft { ref sections } if sections.len() == 3
    ));
    assert_eq!(graph.profiles.len(), 3);
    assert!(matches!(
        graph.profiles[0].geometry,
        ExactBRepPlanarGeometry::Circle { .. }
    ));
    assert!(matches!(
        &graph.profiles[1].geometry,
        ExactBRepPlanarGeometry::Boundary { closed: true, segments } if segments.len() == 8
    ));
    assert!(matches!(
        &graph.profiles[2].geometry,
        ExactBRepPlanarGeometry::Boundary { closed: true, segments } if segments.len() == 4
    ));

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_ne!(committed_digest, baseline_digest);
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), baseline_digest);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("framed Loft must reopen as an editable document");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    let reopened = document.current();
    let reopened_loft = reopened
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::Loft { .. }))
        .unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened, DefinitionId(1), reopened_loft.id())
            .unwrap()
            .schema,
        EXACT_BREP_GRAPH_SCHEMA_V14
    );
}

#[test]
fn public_helix_and_thread_program_is_serializable_atomic_and_exact() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![
        AssistantCadEditOperation::CreateHelix {
            name: "Arbitrary axis helix".into(),
            parameters: AssistantHelixParameters {
                axis: AssistantAxisSpec::TwoPoints {
                    start_mm: [4.0, -3.0, 2.0],
                    end_mm: [5.0, -1.0, 5.0],
                },
                radius_mm: 7.0,
                pitch_mm: 4.5,
                turns: 2.25,
                start_angle_degrees: 37.0,
                handedness: AssistantHelixHandedness::Left,
            },
        },
        AssistantCadEditOperation::CreateThread {
            name: "Arbitrary axis thread".into(),
            parameters: AssistantThreadParameters {
                helix: AssistantHelixParameters {
                    axis: AssistantAxisSpec::OriginDirection {
                        origin_mm: [28.0, 0.0, 0.0],
                        direction: [0.35, 0.2, 1.0],
                    },
                    radius_mm: 8.0,
                    pitch_mm: 6.0,
                    turns: 2.0,
                    start_angle_degrees: 15.0,
                    handedness: AssistantHelixHandedness::Right,
                },
                profile_radius_mm: 0.65,
                profile: AssistantThreadProfile::V,
            },
        },
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 10);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 2);
    assert_eq!(candidate.occurrences().count(), 2);
    assert_eq!(candidate.features().count(), 6);
    let path_lengths = candidate
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::SpatialPath { segments } => Some(segments.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(path_lengths, vec![9, 8]);
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(3)).unwrap();
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(2), FeatureId(6)).unwrap();

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(committed.revision_id(), baseline.revision_id() + 1);
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
fn helix_path_is_a_persistent_non_solid_construction_feature() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateHelixPath {
        name: "Construction helix".into(),
        parameters: AssistantHelixParameters {
            axis: AssistantAxisSpec::OriginDirection {
                origin_mm: [4.0, -3.0, 2.0],
                direction: [1.0, 2.0, 3.0],
            },
            radius_mm: 7.0,
            pitch_mm: 4.5,
            turns: 2.25,
            start_angle_degrees: 37.0,
            handedness: AssistantHelixHandedness::Left,
        },
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::SpatialPath { segments } if segments.len() == 9
    ));
    assert!(!candidate.features().any(|feature| matches!(
        feature.kind(),
        FeatureKind::Sweep { .. } | FeatureKind::SegmentProfile { .. }
    )));

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native construction path must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::SpatialPath { segments } if segments.len() == 9
    ));
}

#[test]
fn helix_resolves_existing_and_same_program_construction_axes() {
    let parameters = |axis| AssistantHelixParameters {
        axis,
        radius_mm: 7.0,
        pitch_mm: 4.5,
        turns: 2.25,
        start_angle_degrees: 37.0,
        handedness: AssistantHelixHandedness::Left,
    };
    let construction_output = AssistantCadProgramFeatureReference {
        operation_index: 0,
        output: AssistantCadProgramFeatureOutput::ConstructionFeature,
    };
    let same_program = program(vec![
        AssistantCadEditOperation::CreateConstructionAxis {
            name: "Referenced axis".into(),
            origin_mm: [4.0, -3.0, 2.0],
            direction: [1.0, 2.0, 3.0],
        },
        AssistantCadEditOperation::CreateHelixPath {
            name: "Referenced Helix".into(),
            parameters: parameters(AssistantAxisSpec::ConstructionAxis {
                axis: AssistantCadFeatureReference::ProgramOutput(construction_output),
            }),
        },
    ]);
    let same_program: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&same_program).unwrap()).unwrap();
    let empty = DocumentStore::new();
    let batch = plan(
        &empty,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &same_program,
    )
    .unwrap();
    let candidate = empty.preview_batch(&batch).unwrap();
    let FeatureKind::SpatialPath {
        segments: same_program_segments,
    } = candidate.feature(FeatureId(2)).unwrap().kind()
    else {
        panic!("referenced axis must produce a spatial Helix path");
    };

    let mut existing = DocumentStore::new();
    let axis_program = program(vec![AssistantCadEditOperation::CreateConstructionAxis {
        name: "Existing axis".into(),
        origin_mm: [4.0, -3.0, 2.0],
        direction: [1.0, 2.0, 3.0],
    }]);
    let axis_batch = plan(
        &existing,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &axis_program,
    )
    .unwrap();
    existing.apply_batch(&axis_batch).unwrap();
    let existing_program = program(vec![AssistantCadEditOperation::CreateHelixPath {
        name: "Existing-axis Helix".into(),
        parameters: parameters(AssistantAxisSpec::ConstructionAxis {
            axis: AssistantCadFeatureReference::Existing(1),
        }),
    }]);
    let existing_batch = plan(
        &existing,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &existing_program,
    )
    .unwrap();
    let existing_candidate = existing.preview_batch(&existing_batch).unwrap();
    let FeatureKind::SpatialPath {
        segments: existing_segments,
    } = existing_candidate.feature(FeatureId(2)).unwrap().kind()
    else {
        panic!("existing axis must produce a spatial Helix path");
    };
    assert_eq!(same_program_segments, existing_segments);
    assert_eq!(
        same_program_segments,
        &parameters(AssistantAxisSpec::OriginDirection {
            origin_mm: [4.0, -3.0, 2.0],
            direction: [1.0, 2.0, 3.0],
        })
        .spatial_path_segments()
        .unwrap()
    );

    let wrong_kind = program(vec![
        AssistantCadEditOperation::CreateConstructionPoint {
            name: "Not an axis".into(),
            position_mm: [0.0; 3],
        },
        AssistantCadEditOperation::CreateHelixPath {
            name: "Invalid Helix".into(),
            parameters: parameters(AssistantAxisSpec::ConstructionAxis {
                axis: AssistantCadFeatureReference::ProgramOutput(construction_output),
            }),
        },
    ]);
    assert!(wrong_kind.validate().is_err());

    let mut wrong_existing = DocumentStore::new();
    let point_program = program(vec![AssistantCadEditOperation::CreateConstructionPoint {
        name: "Existing point".into(),
        position_mm: [0.0; 3],
    }]);
    let point_batch = plan(
        &wrong_existing,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &point_program,
    )
    .unwrap();
    wrong_existing.apply_batch(&point_batch).unwrap();
    let invalid_existing = program(vec![AssistantCadEditOperation::CreateHelixPath {
        name: "Invalid existing-axis Helix".into(),
        parameters: parameters(AssistantAxisSpec::ConstructionAxis {
            axis: AssistantCadFeatureReference::Existing(1),
        }),
    }]);
    assert!(
        plan(
            &wrong_existing,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &invalid_existing,
        )
        .is_err()
    );
}

#[test]
fn revolve_resolves_shared_world_axis_in_arbitrary_workplane() {
    let workplane = AssistantWorkplaneSpec::Frame {
        origin_mm: [3.0, -2.0, 5.0],
        x_axis: [0.0, 1.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
    };
    let revolve_part = |axis| AssistantCadEditOperation::CreatePart {
        name: "Arbitrary-axis revolve".into(),
        workplane: workplane.clone(),
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [5.0, 0.0],
            radius_mm: 1.0,
        }],
        constraints: vec![AssistantSketchConstraint::Radius {
            id: 1,
            entity_id: 1,
            value_mm: 1.0,
        }],
        feature: AssistantCadPartFeature::Revolve {
            axis,
            angle_degrees: 270.0,
        },
        translation_mm: [0.0; 3],
        rotation: None,
    };
    let direct = program(vec![revolve_part(AssistantAxisSpec::TwoPoints {
        start_mm: [3.0, -2.0, 5.0],
        end_mm: [3.0, -2.0, 8.0],
    })]);
    let direct_batch = plan(
        &DocumentStore::new(),
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &direct,
    )
    .unwrap();
    let direct_candidate = DocumentStore::new().preview_batch(&direct_batch).unwrap();
    let FeatureKind::Revolve {
        axis_start_mm: direct_start,
        axis_end_mm: direct_end,
        ..
    } = direct_candidate.feature(FeatureId(3)).unwrap().kind()
    else {
        panic!("shared 3D axis must create Revolve");
    };
    assert_eq!((*direct_start, *direct_end), ([0.0, 0.0], [0.0, 1.0]));
    assert!(
        ExactBRepGraph::from_snapshot(&direct_candidate, DefinitionId(1), FeatureId(3)).is_ok()
    );

    let axis_output = AssistantCadProgramFeatureReference {
        operation_index: 0,
        output: AssistantCadProgramFeatureOutput::ConstructionFeature,
    };
    let referenced = program(vec![
        AssistantCadEditOperation::CreateConstructionAxis {
            name: "Referenced revolve axis".into(),
            origin_mm: [3.0, -2.0, 5.0],
            direction: [0.0, 0.0, 3.0],
        },
        revolve_part(AssistantAxisSpec::ConstructionAxis {
            axis: AssistantCadFeatureReference::ProgramOutput(axis_output),
        }),
    ]);
    let referenced: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&referenced).unwrap()).unwrap();
    let referenced_document = DocumentStore::new();
    let referenced_batch = plan(
        &referenced_document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &referenced,
    )
    .unwrap();
    let referenced_candidate = referenced_document
        .preview_batch(&referenced_batch)
        .unwrap();
    let FeatureKind::Revolve {
        axis_start_mm: referenced_start,
        axis_end_mm: referenced_end,
        ..
    } = referenced_candidate.feature(FeatureId(4)).unwrap().kind()
    else {
        panic!("referenced construction axis must create Revolve");
    };
    assert_eq!(
        (*referenced_start, *referenced_end),
        (*direct_start, *direct_end)
    );

    let off_plane = program(vec![revolve_part(AssistantAxisSpec::OriginDirection {
        origin_mm: [4.0, -2.0, 5.0],
        direction: [0.0, 0.0, 1.0],
    })]);
    assert!(
        plan(
            &DocumentStore::new(),
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &off_plane,
        )
        .is_err()
    );
}

#[test]
fn arbitrary_spatial_path_is_a_persistent_non_solid_3d_curve() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateSpatialPath {
        name: "Mixed construction curve".into(),
        segments: vec![
            AssistantSpatialPathSegment::Line {
                start_mm: [0.0, 0.0, 0.0],
                end_mm: [10.0, 0.0, 0.0],
            },
            AssistantSpatialPathSegment::CircularArc {
                start_mm: [10.0, 0.0, 0.0],
                end_mm: [20.0, 10.0, 0.0],
                center_mm: [10.0, 10.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                clockwise: false,
            },
            AssistantSpatialPathSegment::CubicBezier {
                start_mm: [20.0, 10.0, 0.0],
                control_1_mm: [20.0, 15.0, 0.0],
                control_2_mm: [20.0, 20.0, 5.0],
                end_mm: [25.0, 25.0, 10.0],
            },
        ],
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::SpatialPath { segments }
            if matches!(
                segments.as_slice(),
                [
                    SpatialPathSegment::Line { .. },
                    SpatialPathSegment::CircularArc { .. },
                    SpatialPathSegment::CubicBezier { .. }
                ]
            )
    ));

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native 3D construction curve must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
}

#[test]
fn construction_point_is_persistent_inspectable_and_non_solid() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateConstructionPoint {
        name: "Datum point".into(),
        position_mm: [12.5, -3.25, 7.0],
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionPoint { position_mm }
            if *position_mm == [12.5, -3.25, 7.0]
    ));
    let detail = ModelQuery::default()
        .detail(&candidate, EntityKind::Features, 1)
        .unwrap();
    assert_eq!(detail["item"]["kind"], "ConstructionPoint");
    assert_eq!(detail["item"]["construction"]["type"], "point");
    assert_eq!(
        detail["item"]["construction"]["position_mm"],
        serde_json::json!([12.5, -3.25, 7.0])
    );
    assert_eq!(
        detail["item"]["construction"]["coordinate_space"],
        "definition_mm"
    );

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native construction point must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionPoint { position_mm }
            if *position_mm == [12.5, -3.25, 7.0]
    ));

    assert!(
        program(vec![AssistantCadEditOperation::CreateConstructionPoint {
            name: "Invalid point".into(),
            position_mm: [f64::NAN, 0.0, 0.0],
        }])
        .validate()
        .is_err()
    );
}

#[test]
fn construction_axis_is_persistent_inspectable_and_non_solid() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateConstructionAxis {
        name: "Datum axis".into(),
        origin_mm: [4.0, -2.0, 9.5],
        direction: [1.0, 2.0, 3.0],
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionAxis {
            origin_mm,
            direction
        } if *origin_mm == [4.0, -2.0, 9.5] && *direction == [1.0, 2.0, 3.0]
    ));
    let detail = ModelQuery::default()
        .detail(&candidate, EntityKind::Features, 1)
        .unwrap();
    assert_eq!(detail["item"]["kind"], "ConstructionAxis");
    assert_eq!(detail["item"]["construction"]["type"], "axis");
    assert_eq!(
        detail["item"]["construction"]["origin_mm"],
        serde_json::json!([4.0, -2.0, 9.5])
    );
    assert_eq!(
        detail["item"]["construction"]["direction"],
        serde_json::json!([1.0, 2.0, 3.0])
    );
    assert_eq!(
        detail["item"]["construction"]["coordinate_space"],
        "definition_mm"
    );

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native construction axis must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionAxis {
            origin_mm,
            direction
        } if *origin_mm == [4.0, -2.0, 9.5] && *direction == [1.0, 2.0, 3.0]
    ));

    assert!(
        program(vec![AssistantCadEditOperation::CreateConstructionAxis {
            name: "Invalid axis".into(),
            origin_mm: [0.0; 3],
            direction: [0.0; 3],
        }])
        .validate()
        .is_err()
    );
}

#[test]
fn construction_plane_is_persistent_inspectable_and_non_solid() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::CreateConstructionPlane {
        name: "Datum plane".into(),
        origin_mm: [4.0, -2.0, 9.5],
        normal: [0.0, 0.0, 1.0],
        x_direction: [1.0, 0.0, 0.0],
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    let candidate = document.preview_batch(&batch).unwrap();
    assert_eq!(candidate.definitions().count(), 1);
    assert_eq!(candidate.occurrences().count(), 1);
    assert_eq!(candidate.features().count(), 1);
    assert!(matches!(
        candidate.feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionPlane {
            origin_mm,
            normal,
            x_direction,
        } if *origin_mm == [4.0, -2.0, 9.5]
            && *normal == [0.0, 0.0, 1.0]
            && *x_direction == [1.0, 0.0, 0.0]
    ));
    let detail = ModelQuery::default()
        .detail(&candidate, EntityKind::Features, 1)
        .unwrap();
    assert_eq!(detail["item"]["kind"], "ConstructionPlane");
    assert_eq!(detail["item"]["construction"]["type"], "plane");
    assert_eq!(
        detail["item"]["construction"]["origin_mm"],
        serde_json::json!([4.0, -2.0, 9.5])
    );
    assert_eq!(
        detail["item"]["construction"]["normal"],
        serde_json::json!([0.0, 0.0, 1.0])
    );
    assert_eq!(
        detail["item"]["construction"]["x_direction"],
        serde_json::json!([1.0, 0.0, 0.0])
    );
    assert_eq!(
        detail["item"]["construction"]["coordinate_space"],
        "definition_mm"
    );

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("native construction plane must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(1)).unwrap().kind(),
        FeatureKind::ConstructionPlane {
            origin_mm,
            normal,
            x_direction,
        } if *origin_mm == [4.0, -2.0, 9.5]
            && *normal == [0.0, 0.0, 1.0]
            && *x_direction == [1.0, 0.0, 0.0]
    ));

    assert!(
        program(vec![AssistantCadEditOperation::CreateConstructionPlane {
            name: "Invalid plane".into(),
            origin_mm: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            x_direction: [0.0, 0.0, 1.0],
        }])
        .validate()
        .is_err()
    );
}

#[test]
fn program_sketch_persistently_references_typed_construction_plane_output() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let plane_output = AssistantCadProgramFeatureReference {
        operation_index: 0,
        output: AssistantCadProgramFeatureOutput::ConstructionFeature,
    };
    let definition_output = AssistantCadProgramFeatureReference {
        operation_index: 0,
        output: AssistantCadProgramFeatureOutput::Definition,
    };
    let input = program(vec![
        AssistantCadEditOperation::CreateConstructionPlane {
            name: "Oblique datum plane".into(),
            origin_mm: [4.0, -2.0, 9.5],
            normal: [0.0, 0.0, 2.0],
            x_direction: [3.0, 0.0, 0.0],
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition: definition_output,
            name: "Referenced plane sketch".into(),
            workplane: AssistantWorkplaneSpec::ConstructionPlane {
                plane: AssistantCadFeatureReference::ProgramOutput(plane_output),
            },
            entities: vec![AssistantSketchEntity::Line {
                id: 1,
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 0.0],
            }],
            constraints: Vec::new(),
        },
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 5);
    let candidate = document.preview_batch(&batch).unwrap();
    let FeatureKind::Workplane(workplane) = candidate.feature(FeatureId(2)).unwrap().kind() else {
        panic!("downstream sketch must own a workplane");
    };
    assert!(matches!(
        workplane.support,
        ketchup_core::sketch::WorkplaneSupport::ConstructionPlane {
            feature: FeatureId(1)
        }
    ));
    assert_eq!(workplane.frame.origin_mm, [4.0, -2.0, 9.5]);
    assert_eq!(workplane.frame.x_axis, [1.0, 0.0, 0.0]);
    assert_eq!(workplane.frame.y_axis, [0.0, 1.0, 0.0]);
    assert_eq!(workplane.frame.normal, [0.0, 0.0, 1.0]);
    assert_eq!(
        candidate
            .feature(FeatureId(2))
            .unwrap()
            .kind()
            .dependencies(),
        [FeatureId(1)].into_iter().collect()
    );
    assert!(!candidate.features().any(|feature| matches!(
        feature.kind(),
        FeatureKind::Pad(_) | FeatureKind::Sweep { .. }
    )));

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(
        document
            .preview_batch(&CommandBatch::new(vec![CanonicalCommand::DeleteFeature {
                id: FeatureId(1),
            }]))
            .is_err(),
        "the referenced construction plane must not be deletable"
    );

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("construction-plane sketch reference must reopen losslessly");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(matches!(
        document.current().feature(FeatureId(2)).unwrap().kind(),
        FeatureKind::Workplane(ketchup_core::sketch::WorkplaneSpec {
            support: ketchup_core::sketch::WorkplaneSupport::ConstructionPlane {
                feature: FeatureId(1)
            },
            ..
        })
    ));

    let invalid = program(vec![
        AssistantCadEditOperation::CreateConstructionAxis {
            name: "Not a plane".into(),
            origin_mm: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        },
        AssistantCadEditOperation::CreateProgramSketch {
            definition: definition_output,
            name: "Invalid referenced sketch".into(),
            workplane: AssistantWorkplaneSpec::ConstructionPlane {
                plane: AssistantCadFeatureReference::ProgramOutput(plane_output),
            },
            entities: vec![AssistantSketchEntity::Line {
                id: 1,
                start_mm: [0.0, 0.0],
                end_mm: [10.0, 0.0],
            }],
            constraints: Vec::new(),
        },
    ]);
    assert!(invalid.validate().is_err());
}

#[test]
fn same_program_boolean_resolves_host_assigned_body_outputs_atomically() {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Boolean chain".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Target profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Target body".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("10", 10.0).unwrap(),
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(3),
                definition_id: DefinitionId(1),
                name: "First opening".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[2.0, 2.0], [12.0, 2.0], [12.0, 12.0], [2.0, 12.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: DefinitionId(1),
                name: "Second opening".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[20.0, 10.0], [30.0, 10.0], [30.0, 20.0], [20.0, 20.0]],
                },
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let earlier_body = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(AssistantCadProgramFeatureReference {
            operation_index,
            output: AssistantCadProgramFeatureOutput::BodyFeature,
        })
    };
    let input = program(vec![
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "First pocket body".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2,
                profile_feature_id: 3,
                depth_mm: 5.0,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Second pocket body".into(),
            feature: AssistantCadBodyFeature::Pocket {
                target_feature_id: 2,
                profile_feature_id: 4,
                depth_mm: 5.0,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Chained Boolean".into(),
            feature: AssistantCadBodyFeature::Boolean {
                operation: AssistantCadBooleanOperation::Intersect,
                target_feature_id: earlier_body(0),
                tool_feature_id: earlier_body(1),
            },
        },
    ]);

    let mut guessed_output = input.clone();
    let AssistantCadEditOperation::AppendFeature {
        feature: AssistantCadBodyFeature::Boolean {
            target_feature_id, ..
        },
        ..
    } = &mut guessed_output.operations[2]
    else {
        unreachable!("the third operation is the chained Boolean")
    };
    *target_feature_id = 5.into();
    let rejection = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &guessed_output,
    )
    .unwrap_err();
    assert_eq!(rejection.code, "canonical.feature_not_found");
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);

    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 3);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);
    let candidate = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        candidate.feature(FeatureId(7)).unwrap().kind(),
        FeatureKind::Boolean {
            target: FeatureId(5),
            tool: FeatureId(6),
            ..
        }
    ));
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(7)).unwrap();

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
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
fn create_part_sketch_and_pocket_resolve_typed_program_outputs_atomically() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let input = program(vec![
        part(),
        AssistantCadEditOperation::CreateProgramSketch {
            definition: output(0, AssistantCadProgramFeatureOutput::Definition),
            name: "Opening profile".into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 4.0,
            }],
            constraints: Vec::new(),
        },
        AssistantCadEditOperation::AppendProgramPocket {
            definition: output(0, AssistantCadProgramFeatureOutput::Definition),
            name: "Opening".into(),
            target_feature: output(0, AssistantCadProgramFeatureOutput::BodyFeature),
            profile_feature: output(1, AssistantCadProgramFeatureOutput::SketchFeature),
            depth_mm: 10.0,
        },
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(batch.commands().len(), 8);
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);

    let candidate = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        candidate.feature(FeatureId(6)).unwrap().kind(),
        FeatureKind::Pocket {
            target: FeatureId(3),
            profile: FeatureId(5),
            ..
        }
    ));
    ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(6)).unwrap();

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
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
fn typed_program_outputs_reject_forward_and_cross_kind_references_without_mutation() {
    let document = DocumentStore::new();
    let baseline = document.current();
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let sketch = |definition| AssistantCadEditOperation::CreateProgramSketch {
        definition,
        name: "Opening profile".into(),
        workplane: AssistantWorkplaneSpec::Principal {
            plane: AssistantPrincipalPlane::Xy,
        },
        entities: vec![AssistantSketchEntity::Circle {
            id: 1,
            center_mm: [0.0, 0.0],
            radius_mm: 4.0,
        }],
        constraints: Vec::new(),
    };
    let pocket = |target_feature, profile_feature| AssistantCadEditOperation::AppendProgramPocket {
        definition: output(0, AssistantCadProgramFeatureOutput::Definition),
        name: "Opening".into(),
        target_feature,
        profile_feature,
        depth_mm: 10.0,
    };
    let invalid_programs = [
        program(vec![
            part(),
            sketch(output(0, AssistantCadProgramFeatureOutput::BodyFeature)),
        ]),
        program(vec![
            part(),
            sketch(output(0, AssistantCadProgramFeatureOutput::Definition)),
            pocket(
                output(0, AssistantCadProgramFeatureOutput::BodyFeature),
                output(0, AssistantCadProgramFeatureOutput::BodyFeature),
            ),
        ]),
        program(vec![
            part(),
            sketch(output(0, AssistantCadProgramFeatureOutput::Definition)),
            pocket(
                output(2, AssistantCadProgramFeatureOutput::BodyFeature),
                output(1, AssistantCadProgramFeatureOutput::SketchFeature),
            ),
        ]),
        program(vec![
            part(),
            AssistantCadEditOperation::CreateProgramSketch {
                definition: output(0, AssistantCadProgramFeatureOutput::Definition),
                name: "Guessed workplane".into(),
                workplane: AssistantWorkplaneSpec::Offset {
                    base_feature_id: 1,
                    distance_mm: 1.0,
                },
                entities: vec![AssistantSketchEntity::Circle {
                    id: 1,
                    center_mm: [0.0, 0.0],
                    radius_mm: 4.0,
                }],
                constraints: Vec::new(),
            },
        ]),
    ];
    for input in invalid_programs {
        let rejection = plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &input,
        )
        .unwrap_err();
        assert_eq!(rejection.phase, AssistantRejectionPhase::IntentValidation);
        assert_eq!(rejection.code, "intent.cad_edit_program_invalid");
    }
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);
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
fn circular_pattern_uses_the_shared_arbitrary_axis_atomically() {
    let direct = program(vec![AssistantCadEditOperation::CircularPattern {
        selector: explicit(1),
        instances: 4,
        axis: AssistantAxisSpec::OriginDirection {
            origin_mm: [1.0, 2.0, 3.0],
            direction: [1.0, 1.0, 0.0],
        },
        angle_step_degrees: 90.0,
    }]);
    let two_points = program(vec![AssistantCadEditOperation::CircularPattern {
        selector: explicit(1),
        instances: 4,
        axis: AssistantAxisSpec::TwoPoints {
            start_mm: [1.0, 2.0, 3.0],
            end_mm: [2.0, 3.0, 3.0],
        },
        angle_step_degrees: 90.0,
    }]);
    let mut document = seeded();
    let before = document.current();
    let undo = document.visible_undo_steps();
    let registry = ExactResultRegistry::default();
    let direct_batch = plan(&document, &BTreeSet::new(), &registry, &direct).unwrap();
    let points_batch = plan(&document, &BTreeSet::new(), &registry, &two_points).unwrap();
    assert_eq!(direct_batch.commands(), points_batch.commands());
    assert_eq!(direct_batch.commands().len(), 3);

    document.apply_batch(&direct_batch).unwrap();
    let patterned = document.current();
    assert_eq!(patterned.occurrences().count(), 4);
    let first_copy = patterned.occurrence(OccurrenceId(2)).unwrap().transform();
    let expected_translation = [7.828_427_124_746_19, 3.171_572_875_253_81, 3.0];
    for (index, expected) in [3, 7, 11].into_iter().zip(expected_translation) {
        assert!((first_copy.matrix()[index] - expected).abs() < 1.0e-9);
    }
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        patterned.canonical_digest()
    );

    let duplicate = program(vec![AssistantCadEditOperation::CircularPattern {
        selector: explicit(1),
        instances: 3,
        axis: AssistantAxisSpec::OriginDirection {
            origin_mm: [0.0; 3],
            direction: [0.0, 0.0, 1.0],
        },
        angle_step_degrees: 180.0,
    }]);
    let error = plan(&document, &BTreeSet::new(), &registry, &duplicate).unwrap_err();
    assert_eq!(error.phase, AssistantRejectionPhase::IntentValidation);
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
fn public_framed_sketch_output_builds_an_exact_editable_planar_offset() {
    let mut document = DocumentStore::new();
    let part_program = program(vec![AssistantCadEditOperation::CreatePart {
        name: "Framed cubic profile".into(),
        workplane: AssistantWorkplaneSpec::Frame {
            origin_mm: [30.0, -20.0, 15.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
        },
        entities: vec![
            AssistantSketchEntity::CubicBezier {
                id: 1,
                start_mm: [-20.0, 0.0],
                control_1_mm: [-20.0, 15.0],
                control_2_mm: [20.0, 15.0],
                end_mm: [20.0, 0.0],
            },
            AssistantSketchEntity::CubicBezier {
                id: 2,
                start_mm: [20.0, 0.0],
                control_1_mm: [20.0, -15.0],
                control_2_mm: [-20.0, -15.0],
                end_mm: [-20.0, 0.0],
            },
        ],
        constraints: Vec::new(),
        feature: AssistantCadPartFeature::Extrusion { distance_mm: 8.0 },
        translation_mm: [0.0; 3],
        rotation: None,
    }]);
    let registry = ExactResultRegistry::default();
    let part_batch = plan(&document, &BTreeSet::new(), &registry, &part_program).unwrap();
    document.apply_batch(&part_batch).unwrap();
    let baseline_digest = document.current().canonical_digest();
    let offset_program = |distance_mm| {
        program(vec![AssistantCadEditOperation::AppendFeature {
            definition_id: 1,
            name: "Framed planar offset".into(),
            feature: AssistantCadBodyFeature::PlanarOffset {
                profile_feature_id: 2,
                distance_mm,
            },
        }])
    };
    let input = offset_program(3.0);
    let encoded = serde_json::to_vec(&input).unwrap();
    let decoded: AssistantCadEditProgram = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, input);

    let batch = plan(&document, &BTreeSet::new(), &registry, &decoded).unwrap();
    let candidate = document.preview_batch(&batch).unwrap();
    let graph = ExactBRepGraph::from_snapshot(&candidate, DefinitionId(1), FeatureId(4)).unwrap();
    assert!(graph.terminal_is_planar_offset());
    assert_eq!(graph.profiles[0].source_feature_id, 2);
    assert_eq!(graph.profiles[0].frame_bits[0], 30.0_f64.to_bits());
    assert_eq!(graph.profiles[0].frame_bits[1], (-20.0_f64).to_bits());
    assert_eq!(graph.profiles[0].frame_bits[2], 15.0_f64.to_bits());
    let bounds = graph.producer_bounds_mm().unwrap().unwrap();
    assert_eq!(bounds[0][0], 30.0);
    assert_eq!(bounds[1][0], 30.0);
    assert!(bounds[0][1] < -40.0 && bounds[1][1] > 0.0);
    assert!(bounds[0][2] < 15.0 && bounds[1][2] > 15.0);

    let error = plan(
        &document,
        &BTreeSet::new(),
        &registry,
        &offset_program(-21.0),
    )
    .unwrap_err();
    assert_eq!(error.code, "planning.cad_feature_input_unsupported");
    assert_eq!(document.current().canonical_digest(), baseline_digest);

    document.apply_batch(&batch).unwrap();
    let committed_digest = document.current().canonical_digest();
    assert_eq!(document.visible_undo_steps(), 2);
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), baseline_digest);
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("framed Planar Offset must reopen as an editable document");
    };
    assert_eq!(document.current().canonical_digest(), committed_digest);
    assert!(
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(1), FeatureId(4),).is_ok()
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

#[test]
fn color_program_is_atomic_and_copy_pattern_mirror_preserve_accumulated_color() {
    let mut document = seeded();
    let registry = ExactResultRegistry::default();
    let color = Some([12, 128, 255]);
    let input = program(vec![
        AssistantCadEditOperation::SetColor {
            selector: explicit(1),
            color,
        },
        AssistantCadEditOperation::Copy {
            selector: explicit(1),
            translation_mm: [30.0, 0.0, 0.0],
        },
        AssistantCadEditOperation::LinearPattern {
            selector: explicit(1),
            instances: 3,
            step_mm: [0.0, 30.0, 0.0],
        },
        AssistantCadEditOperation::Mirror {
            selector: explicit(1),
            plane_origin_mm: [0.0; 3],
            plane_normal: [1.0, 0.0, 0.0],
        },
    ]);
    let before = document.current();
    let batch = plan(&document, &BTreeSet::new(), &registry, &input).unwrap();
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.current().occurrences().count(), 5);
    assert!(document.current().occurrences().all(|o| o.color() == color));
    let colored = document.current().canonical_digest();
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), colored);
    let selection = BTreeSet::from([OccurrenceId(1)]);
    let reset = program(vec![AssistantCadEditOperation::SetColor {
        selector: AssistantCadEntitySelector::CurrentSelection {},
        color: None,
    }]);
    document
        .apply_batch(&plan(&document, &selection, &registry, &reset).unwrap())
        .unwrap();
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .color(),
        None
    );
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(2))
            .unwrap()
            .color(),
        color
    );
    let baseline = document.current().canonical_digest();
    for ids in [vec![], vec![1, 1], vec![1, 999]] {
        let invalid = program(vec![AssistantCadEditOperation::SetColor {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: ids,
            },
            color,
        }]);
        assert!(plan(&document, &selection, &registry, &invalid).is_err());
        assert_eq!(document.current().canonical_digest(), baseline);
    }
}

#[test]
fn color_wire_rejects_non_rgb_bytes() {
    for color in [
        "[-1,0,0]",
        "[256,0,0]",
        "[1.5,0,0]",
        "[true,0,0]",
        "[1,2]",
        "[1,2,3,4]",
        "\"red\"",
    ] {
        let json = format!(
            r#"{{"operations":[{{"operation":"set_color","selector":{{"type":"occurrences","occurrence_ids":[1]}},"color":{color}}}]}}"#
        );
        assert!(
            serde_json::from_str::<AssistantCadEditProgram>(&json).is_err(),
            "{json}"
        );
    }
}

#[test]
fn public_program_sweeps_a_general_sketch_profile_along_a_spatial_path_atomically() {
    let definition = DefinitionId(1);
    let workplane = FeatureId(1);
    let profile = FeatureId(2);
    let path = FeatureId(3);
    let profile_sketch = SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [-2.0, -2.0],
                end_mm: [2.0, -2.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(2),
                start_mm: [2.0, -2.0],
                end_mm: [2.0, 2.0],
                center_mm: [2.0, 0.0],
                clockwise: false,
            },
            SketchEntity::CubicBezier {
                id: SketchEntityId(3),
                start_mm: [2.0, 2.0],
                control_1_mm: [0.5, 3.0],
                control_2_mm: [-0.5, 3.0],
                end_mm: [-2.0, 2.0],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [-2.0, 2.0],
                end_mm: [-2.0, -2.0],
            },
        ],
        constraints: Vec::new(),
    };
    let spatial_segments = vec![
        SpatialPathSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [0.0, 0.0, 20.0],
        },
        SpatialPathSegment::CircularArc {
            start_mm: [0.0, 0.0, 20.0],
            end_mm: [10.0, 0.0, 30.0],
            center_mm: [10.0, 0.0, 20.0],
            normal: [0.0, 1.0, 0.0],
            clockwise: false,
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [10.0, 0.0, 30.0],
            control_1_mm: [15.0, 0.0, 30.0],
            control_2_mm: [20.0, 5.0, 35.0],
            end_mm: [25.0, 10.0, 40.0],
        },
    ];
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "General sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "Profile plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Line arc cubic profile".into(),
                kind: FeatureKind::Sketch(profile_sketch.clone()),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "3D path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: spatial_segments.clone(),
                },
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: definition.0,
        name: "Public general sweep".into(),
        feature: AssistantCadBodyFeature::Sweep {
            profile_feature_id: profile.0,
            path_feature_id: path.0,
        },
    }]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    let sweep = FeatureId(4);
    let graph = ExactBRepGraph::from_snapshot(&preview, definition, sweep).unwrap();
    assert!(matches!(
        &graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SpatialSweep { path, .. } if path.segments.len() == 3
    ));
    assert!(matches!(
        &graph.profiles[0].geometry,
        ExactBRepPlanarGeometry::Boundary { closed: true, segments } if segments.len() == 4
    ));

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(document.visible_undo_steps(), 2);
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
    let bytes = save_document_store(&document, &ContainerData::default()).unwrap();
    let LoadOutcome::Editable { document, .. } = load(&bytes).unwrap() else {
        panic!("general Sketch/SpatialPath Sweep must reopen losslessly");
    };
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );

    let mut sketch_path_document = DocumentStore::new();
    sketch_path_document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Sketch path sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "Profile plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Line arc cubic profile".into(),
                kind: FeatureKind::Sketch(profile_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "Path plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xz)),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(4),
                definition_id: definition,
                name: "2D curved path".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: path,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [0.0, 0.0],
                            end_mm: [0.0, 20.0],
                        },
                        SketchEntity::Arc {
                            id: SketchEntityId(2),
                            start_mm: [0.0, 20.0],
                            end_mm: [10.0, 30.0],
                            center_mm: [10.0, 20.0],
                            clockwise: true,
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(3),
                            start_mm: [10.0, 30.0],
                            control_1_mm: [15.0, 30.0],
                            control_2_mm: [20.0, 35.0],
                            end_mm: [25.0, 40.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
        ]))
        .unwrap();
    let sketch_path_program = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: definition.0,
        name: "Public Sketch path sweep".into(),
        feature: AssistantCadBodyFeature::Sweep {
            profile_feature_id: profile.0,
            path_feature_id: 4,
        },
    }]);
    let sketch_path_batch = plan(
        &sketch_path_document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &sketch_path_program,
    )
    .unwrap();
    let sketch_path_preview = sketch_path_document
        .preview_batch(&sketch_path_batch)
        .unwrap();
    let sketch_path_graph =
        ExactBRepGraph::from_snapshot(&sketch_path_preview, definition, FeatureId(5)).unwrap();
    assert!(matches!(
        &sketch_path_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::SpatialSweep { path, .. } if path.segments.len() == 3
    ));

    let mut invalid = DocumentStore::new();
    invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Invalid sweep".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "Profile plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Open profile".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane,
                    entities: vec![SketchEntity::Line {
                        id: SketchEntityId(1),
                        start_mm: [-2.0, 0.0],
                        end_mm: [2.0, 0.0],
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: path,
                definition_id: definition,
                name: "3D path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: spatial_segments,
                },
            },
        ]))
        .unwrap();
    let invalid_baseline = invalid.current();
    assert!(
        plan(
            &invalid,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &input,
        )
        .is_err()
    );
    assert_eq!(
        invalid.current().canonical_digest(),
        invalid_baseline.canonical_digest()
    );
}
