use ketchup_core::assembly_recipe::{
    ASSEMBLY_RECIPE_SCHEMA_V1, AssemblyRecipe, AssemblyRecipeCompileError, AssemblyRecipeError,
    RecipeDimensionAnchor, RecipeEditScope, RecipeFaceRef, RecipeJoinery, RecipeKey,
    RecipeParameter, RecipeParameterUnit, RecipePartAdoption, RecipePartMobility, RecipePatchNode,
    RecipeRelation, RecipeRelationKind, RecipeSemanticChange, RecipeSemanticPatch,
    RecognizedRecipeFeatureKind, compile_assembly_recipe_patch,
};
use ketchup_core::document::{
    CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension, FeatureId,
    FeatureKind, FeatureParameterTarget, GroupId, InstancePath, OccurrenceId, ParameterValueType,
    TagId, Transform,
};
use ketchup_core::joinery::{
    DowelJointContract, DowelJointFace, DowelJointId, DowelPhysicalHolePair, StandardDowel,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    SketchEntity, SketchEntityId, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
};
use std::collections::BTreeMap;

fn key(value: &str) -> RecipeKey {
    RecipeKey::new(value).unwrap()
}

fn panel_commands(
    definition_id: u64,
    profile_id: u64,
    extrusion_id: u64,
    occurrence_id: u64,
    name: &str,
    tag: Option<TagId>,
) -> Vec<CanonicalCommand> {
    vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(definition_id),
            name: name.to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(profile_id),
            definition_id: DefinitionId(definition_id),
            name: format!("{name} profile"),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [600.0, 0.0], [600.0, 400.0], [0.0, 400.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(extrusion_id),
            definition_id: DefinitionId(definition_id),
            name: format!("{name} extrusion"),
            kind: FeatureKind::Extrusion {
                profile: FeatureId(profile_id),
                height: Dimension::new("19", 19.0).unwrap(),
            },
        },
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(occurrence_id),
            definition_id: DefinitionId(definition_id),
            name: format!("{name} occurrence"),
            transform: Transform::identity(),
            parent: None,
            tag,
            visible: true,
        },
    ]
}

fn dowel_joint() -> DowelJointContract {
    DowelJointContract {
        id: DowelJointId(7),
        name: "rear-top-row".to_owned(),
        first: DowelJointFace {
            instance_path: InstancePath::root(OccurrenceId(1)),
            face_origin_local_mm: [0.0, 0.0, 0.0],
            inward_unit_local: [0.0, 0.0, -1.0],
            bounds_min_local_mm: [0.0, 0.0, -19.0],
            bounds_max_local_mm: [600.0, 400.0, 0.0],
        },
        second: DowelJointFace {
            instance_path: InstancePath::root(OccurrenceId(2)),
            face_origin_local_mm: [0.0, 0.0, 0.0],
            inward_unit_local: [0.0, 0.0, 1.0],
            bounds_min_local_mm: [0.0, 0.0, 0.0],
            bounds_max_local_mm: [600.0, 400.0, 19.0],
        },
        first_center_local_mm: [50.0, 20.0, 0.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count: 3,
        spacing_mm: 32.0,
        dowel: StandardDowel::D8x30.symmetric_spec(),
        physical_hole_pairs: None,
    }
}

fn adoption_part(
    part_key: &str,
    occurrence_id: u64,
    _definition_id: u64,
    profile_id: u64,
    extrusion_id: u64,
    height_mm: f64,
    scope: RecipeEditScope,
) -> RecipePartAdoption {
    let mut parameters = BTreeMap::new();
    parameters.insert(
        key("height"),
        RecipeParameter {
            value: height_mm,
            unit: RecipeParameterUnit::Millimetres,
            target: Some(
                FeatureParameterTarget::new(
                    FeatureId(extrusion_id),
                    "height",
                    ParameterValueType::Length,
                )
                .unwrap(),
            ),
        },
    );
    RecipePartAdoption {
        key: key(part_key),
        instance_path: InstancePath::root(OccurrenceId(occurrence_id)),
        mobility: if part_key == "rear" {
            RecipePartMobility::Movable
        } else {
            RecipePartMobility::Fixed
        },
        edit_scope: scope,
        parameters,
        features: vec![
            (
                key(&format!("{part_key}/profile")),
                FeatureId(profile_id),
                RecognizedRecipeFeatureKind::Profile,
            ),
            (
                key(&format!("{part_key}/solid")),
                FeatureId(extrusion_id),
                RecognizedRecipeFeatureKind::Extrusion,
            ),
        ],
    }
}

#[test]
fn rectangular_sketch_pad_dimensions_preserve_anchors_and_entity_identity() {
    use ketchup_core::sketch::{FeatureDirection, FeatureExtent, PadSpec};
    for angle in [0.0_f64, 37.0] {
        let (sine, cosine) = angle.to_radians().sin_cos();
        let frame =
            WorkplaneFrame::from_axes([5.0, 7.0, 11.0], [cosine, sine, 0.0], [-sine, cosine, 0.0])
                .unwrap();
        for (feature_id, path, current, next, axis) in [
            (2, "bounds.width", 40.0, 52.0, frame.x_axis),
            (2, "bounds.height", 30.0, 45.0, frame.y_axis),
            (3, "extent.distance", 6.0, 10.0, frame.normal),
        ] {
            for (anchor, fraction) in [
                (RecipeDimensionAnchor::Minimum, 0.0),
                (RecipeDimensionAnchor::Centre, 0.5),
                (RecipeDimensionAnchor::Maximum, 1.0),
            ] {
                let corners = [[10.0, 20.0], [50.0, 20.0], [50.0, 50.0], [10.0, 50.0]];
                let sketch = SketchSpec {
                    workplane: FeatureId(1),
                    entities: (0..4)
                        .map(|index| SketchEntity::Line {
                            id: SketchEntityId(index as u64 + 1),
                            start_mm: corners[index],
                            end_mm: corners[(index + 1) % 4],
                        })
                        .collect(),
                    constraints: vec![],
                };
                let region = sketch.solved_regions().unwrap()[0].id;
                let mut document = ketchup_core::document::DocumentStore::new();
                document
                    .apply_batch(&CommandBatch::new(vec![
                        CanonicalCommand::CreateDefinition {
                            id: DefinitionId(1),
                            name: "Panel".into(),
                        },
                        CanonicalCommand::CreateFeature {
                            id: FeatureId(1),
                            definition_id: DefinitionId(1),
                            name: "Frame".into(),
                            kind: FeatureKind::Workplane(WorkplaneSpec {
                                support: WorkplaneSupport::Free,
                                frame,
                            }),
                        },
                        CanonicalCommand::CreateFeature {
                            id: FeatureId(2),
                            definition_id: DefinitionId(1),
                            name: "Rectangle".into(),
                            kind: FeatureKind::Sketch(sketch),
                        },
                        CanonicalCommand::CreateFeature {
                            id: FeatureId(3),
                            definition_id: DefinitionId(1),
                            name: "Solid".into(),
                            kind: FeatureKind::Pad(PadSpec {
                                sketch: FeatureId(2),
                                region,
                                direction: FeatureDirection::AlongNormal,
                                extent: FeatureExtent::Blind(Dimension::new("6", 6.0).unwrap()),
                            }),
                        },
                        CanonicalCommand::CreateOccurrence {
                            id: OccurrenceId(1),
                            definition_id: DefinitionId(1),
                            name: "Panel".into(),
                            transform: Transform::identity(),
                            parent: None,
                            tag: None,
                            visible: true,
                        },
                    ]))
                    .unwrap();
                let target = FeatureParameterTarget::new(
                    FeatureId(feature_id),
                    path,
                    ParameterValueType::Length,
                )
                .unwrap();
                let recipe = AssemblyRecipe::adopt(
                    &document.current(),
                    key("assembly"),
                    vec![RecipePartAdoption {
                        key: key("panel"),
                        instance_path: InstancePath::root(OccurrenceId(1)),
                        mobility: RecipePartMobility::Movable,
                        edit_scope: RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(
                            1,
                        ))),
                        parameters: BTreeMap::from([(
                            key("size"),
                            RecipeParameter {
                                value: current,
                                unit: RecipeParameterUnit::Millimetres,
                                target: Some(target.clone()),
                            },
                        )]),
                        features: vec![
                            (
                                key("panel/frame"),
                                FeatureId(1),
                                RecognizedRecipeFeatureKind::Workplane,
                            ),
                            (
                                key("panel/sketch"),
                                FeatureId(2),
                                RecognizedRecipeFeatureKind::Sketch,
                            ),
                            (
                                key("panel/pad"),
                                FeatureId(3),
                                RecognizedRecipeFeatureKind::Pad,
                            ),
                        ],
                    }],
                    vec![],
                    vec![],
                )
                .unwrap();
                document
                    .apply_batch(&CommandBatch::new(vec![
                        CanonicalCommand::SetAssemblyRecipe(recipe),
                    ]))
                    .unwrap();
                let before = document.current();
                let patch = RecipeSemanticPatch {
                    nodes: vec![RecipePatchNode {
                        key: key("resize"),
                        dependencies: vec![],
                        change: RecipeSemanticChange::SetParameter {
                            part: key("panel"),
                            parameter: key("size"),
                            value: next,
                            anchor,
                        },
                    }],
                };
                let result = compile_assembly_recipe_patch(&before, &patch).unwrap();
                document
                    .apply_batch(result.batch.as_ref().unwrap())
                    .unwrap();
                let after = document.current();
                assert_eq!(after.feature_parameter_value(&target), Some(next));
                assert_eq!(
                    after
                        .features()
                        .map(|feature| feature.id())
                        .collect::<Vec<_>>(),
                    vec![FeatureId(1), FeatureId(2), FeatureId(3)]
                );
                let transform = after.occurrence(OccurrenceId(1)).unwrap().transform();
                for (coordinate, direction) in axis.iter().enumerate() {
                    assert!(
                        (transform.matrix()[coordinate * 4 + 3]
                            - direction * (current - next) * fraction)
                            .abs()
                            < 1.0e-9
                    );
                }
                let FeatureKind::Sketch(actual) = after.feature(FeatureId(2)).unwrap().kind()
                else {
                    panic!("sketch replaced");
                };
                let mut expected = corners;
                if path == "bounds.width" {
                    expected[1][0] = 62.0;
                    expected[2][0] = 62.0;
                }
                if path == "bounds.height" {
                    expected[2][1] = 65.0;
                    expected[3][1] = 65.0;
                }
                for (index, entity) in actual.entities.iter().enumerate() {
                    assert_eq!(
                        *entity,
                        SketchEntity::Line {
                            id: SketchEntityId(index as u64 + 1),
                            start_mm: expected[index],
                            end_mm: expected[(index + 1) % 4]
                        }
                    );
                }
                assert_eq!(actual.solved_regions().unwrap()[0].id, region);
                assert!(
                    compile_assembly_recipe_patch(&after, &patch)
                        .unwrap()
                        .batch
                        .is_none()
                );
                document.undo().unwrap();
                assert_eq!(
                    document.current().canonical_digest(),
                    before.canonical_digest()
                );
                document.redo().unwrap();
                assert_eq!(
                    document.current().canonical_digest(),
                    after.canonical_digest()
                );
            }
        }
    }
}

#[test]
fn sketch_bounds_parameters_reject_constraints_and_nonrectangular_profiles() {
    use ketchup_core::document::ParameterPath;
    use ketchup_core::sketch::{SketchConstraint, SketchConstraintId, SketchConstraintKind};
    let corners = [[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]];
    let rectangle = SketchSpec {
        workplane: FeatureId(1),
        entities: (0..4)
            .map(|index| SketchEntity::Line {
                id: SketchEntityId(index as u64 + 1),
                start_mm: corners[index],
                end_mm: corners[(index + 1) % 4],
            })
            .collect(),
        constraints: vec![],
    };
    let mut constrained = rectangle.clone();
    constrained.constraints.push(SketchConstraint {
        id: SketchConstraintId(1),
        kind: SketchConstraintKind::Horizontal {
            entity: SketchEntityId(1),
        },
    });
    let mut open = rectangle.clone();
    if let SketchEntity::Line { end_mm, .. } = &mut open.entities[3] {
        *end_mm = [0.0, 1.0];
    }
    let mut duplicate = rectangle.clone();
    duplicate.entities[3] = SketchEntity::Line {
        id: SketchEntityId(4),
        start_mm: corners[0],
        end_mm: corners[1],
    };
    let mut trapezoid = rectangle.clone();
    if let SketchEntity::Line { end_mm, .. } = &mut trapezoid.entities[1] {
        *end_mm = [35.0, 30.0];
    }
    if let SketchEntity::Line { start_mm, .. } = &mut trapezoid.entities[2] {
        *start_mm = [35.0, 30.0];
    }
    for spec in [constrained, open, duplicate, trapezoid] {
        let kind = FeatureKind::Sketch(spec);
        assert!(
            kind.parameter_descriptors()
                .iter()
                .all(|parameter| !parameter.path().as_str().starts_with("bounds."))
        );
        assert_eq!(
            kind.parameter_value(&ParameterPath::new("bounds.height").unwrap()),
            None
        );
    }
    let mut reordered = rectangle.clone();
    reordered.entities.reverse();
    for entity in &mut reordered.entities {
        if let SketchEntity::Line {
            start_mm, end_mm, ..
        } = entity
        {
            std::mem::swap(start_mm, end_mm);
        }
    }
    assert_eq!(
        FeatureKind::Sketch(reordered)
            .parameter_value(&ParameterPath::new("bounds.height").unwrap()),
        Some(30.0)
    );
}

fn populated_document() -> ketchup_core::document::DocumentStore {
    let mut document = ketchup_core::document::DocumentStore::new();
    let mut commands = vec![CanonicalCommand::CreateTag {
        id: TagId(1),
        name: "Joinery".to_owned(),
        visible: true,
    }];
    commands.extend(panel_commands(1, 1, 2, 1, "Rear", Some(TagId(1))));
    commands.extend(panel_commands(2, 3, 4, 2, "Top", None));
    commands.extend([
        CanonicalCommand::SetOccurrenceColor {
            id: OccurrenceId(1),
            color: Some([120, 80, 40]),
        },
        CanonicalCommand::UpsertDowelJoint(dowel_joint()),
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

fn adopt_recipe(snapshot: &ketchup_core::document::Snapshot) -> AssemblyRecipe {
    let height = |id| match snapshot.feature(FeatureId(id)).unwrap().kind() {
        FeatureKind::Extrusion { height, .. } => height.millimetres(),
        _ => panic!("recipe panel extrusion is missing"),
    };
    AssemblyRecipe::adopt(
        snapshot,
        key("cabinet"),
        vec![
            adoption_part(
                "rear",
                1,
                1,
                1,
                2,
                height(2),
                RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
            ),
            adoption_part(
                "top",
                2,
                2,
                3,
                4,
                height(4),
                RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(2))),
            ),
        ],
        vec![RecipeRelation {
            key: key("rear_top_contact"),
            kind: RecipeRelationKind::Contact,
            first: RecipeFaceRef {
                part: key("rear"),
                role: "extrusion.top".to_owned(),
            },
            second: RecipeFaceRef {
                part: key("top"),
                role: "extrusion.bottom".to_owned(),
            },
        }],
        vec![RecipeJoinery {
            key: key("rear_top_dowels"),
            first_part: key("rear"),
            second_part: key("top"),
            dowel_joint_id: DowelJointId(7),
        }],
    )
    .unwrap()
}

#[test]
fn recipe_round_trip_preserves_keys_geometry_appearance_joinery_and_history() {
    let mut document = populated_document();
    document.discard_history_before_current();
    let geometry_digest = document.current().canonical_digest();
    let recipe = adopt_recipe(&document.current());
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe.clone()),
        ]))
        .unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    let expected_digest = document.current().canonical_digest();
    assert_ne!(expected_digest, geometry_digest);

    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let loaded = persistence::load(&bytes).unwrap();
    let mut reopened = loaded.into_editable().ok().unwrap();
    let snapshot = reopened.current();
    assert_eq!(snapshot.assembly_recipe(), Some(&recipe));
    assert_eq!(snapshot.canonical_digest(), expected_digest);
    assert_eq!(
        snapshot.occurrence(OccurrenceId(1)).unwrap().tag(),
        Some(TagId(1))
    );
    assert_eq!(
        snapshot.occurrence(OccurrenceId(1)).unwrap().color(),
        Some([120, 80, 40])
    );
    assert_eq!(snapshot.dowel_joint(DowelJointId(7)), Some(&dowel_joint()));
    assert_eq!(
        snapshot.feature(FeatureId(2)).unwrap().kind(),
        &FeatureKind::Extrusion {
            profile: FeatureId(1),
            height: Dimension::new("19", 19.0).unwrap(),
        }
    );

    assert_eq!(reopened.visible_undo_steps(), 1);
    reopened.undo().unwrap();
    assert!(reopened.current().assembly_recipe().is_none());
    assert_eq!(reopened.current().canonical_digest(), geometry_digest);
    reopened.redo().unwrap();
    assert_eq!(reopened.current().assembly_recipe(), Some(&recipe));
}

#[test]
fn manual_owned_feature_change_fails_closed_but_atomic_recipe_update_succeeds() {
    let mut document = populated_document();
    let recipe = adopt_recipe(&document.current());
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    document.discard_history_before_current();
    let before = document.current().canonical_digest();

    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(2),
                dimension: Dimension::new("20", 20.0).unwrap(),
            },
        ]))
        .err()
        .unwrap();
    assert!(matches!(
        error,
        CanonicalError::AssemblyRecipe(AssemblyRecipeError::InvalidParameter(key))
            if key.as_str() == "rear"
    ));
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);

    let candidate = document
        .current()
        .preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::ClearAssemblyRecipe,
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(2),
                dimension: Dimension::new("20", 20.0).unwrap(),
            },
        ]))
        .unwrap();
    let updated_recipe = adopt_recipe(&candidate);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::ClearAssemblyRecipe,
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(2),
                dimension: Dimension::new("20", 20.0).unwrap(),
            },
            CanonicalCommand::SetAssemblyRecipe(updated_recipe),
        ]))
        .unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(
        document.current().feature(FeatureId(2)).unwrap().kind(),
        &FeatureKind::Extrusion {
            profile: FeatureId(1),
            height: Dimension::new("20", 20.0).unwrap(),
        }
    );
    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), before);
}

#[test]
fn make_unique_remaps_only_selected_recipe_part_and_does_not_bless_stale_ownership() {
    use ketchup_core::document::CloneDefinitionPlan;
    let mut document = populated_document();
    let mut joint = dowel_joint();
    joint.first.face_origin_local_mm = [0.0, 0.0, 19.0];
    joint.first.bounds_min_local_mm = [0.0, 0.0, 0.0];
    joint.first.bounds_max_local_mm = [600.0, 400.0, 19.0];
    joint.first_center_local_mm = [50.0, 20.0, 19.0];
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_translation(0.0, 0.0, -19.0).unwrap(),
            },
            CanonicalCommand::UpsertDowelJoint(joint),
        ]))
        .unwrap();
    let recipe = adopt_recipe(&document.current());
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe.clone()),
        ]))
        .unwrap();
    let before = document.current();
    let undo = document.visible_undo_steps();
    let clone = CanonicalCommand::CloneDefinitionAndRepoint(CloneDefinitionPlan::new(
        OccurrenceId(1),
        DefinitionId(1),
        DefinitionId(3),
        "Independent rear".to_owned(),
        vec![(FeatureId(1), FeatureId(5)), (FeatureId(2), FeatureId(6))],
    ));
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: FeatureId(2),
                dimension: Dimension::new("20", 20.0).unwrap(),
            },
            clone.clone(),
        ]))
        .err()
        .unwrap();
    assert!(matches!(
        error,
        CanonicalError::AssemblyRecipe(AssemblyRecipeError::OwnedFeatureConflict(_))
    ));
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(document.current().revision_id(), before.revision_id());
    assert_eq!(document.visible_undo_steps(), undo);

    document
        .apply_batch(&CommandBatch::new(vec![clone]))
        .unwrap();
    let after = document.current();
    let remapped = after.assembly_recipe().unwrap();
    remapped.audit(&after).unwrap();
    assert_eq!(
        remapped.relations().collect::<Vec<_>>(),
        recipe.relations().collect::<Vec<_>>()
    );
    assert_eq!(
        remapped.joinery().collect::<Vec<_>>(),
        recipe.joinery().collect::<Vec<_>>()
    );
    assert_eq!(
        remapped.parts().find(|part| part.key == key("top")),
        recipe.parts().find(|part| part.key == key("top"))
    );
    for owned in recipe
        .owned_features()
        .filter(|owned| owned.part == key("top"))
    {
        assert_eq!(
            remapped.owned_features().find(|item| item.key == owned.key),
            Some(owned)
        );
    }
    assert_eq!(
        after.dowel_joint(DowelJointId(7)),
        before.dowel_joint(DowelJointId(7))
    );
    let compiled = compile_assembly_recipe_patch(
        &after,
        &RecipeSemanticPatch {
            nodes: vec![RecipePatchNode {
                key: key("resize-independent"),
                dependencies: vec![],
                change: RecipeSemanticChange::SetParameter {
                    part: key("rear"),
                    parameter: key("height"),
                    value: 20.0,
                    anchor: RecipeDimensionAnchor::Maximum,
                },
            }],
        },
    )
    .unwrap();
    document.apply_batch(&compiled.batch.unwrap()).unwrap();
    assert_eq!(
        document.current().feature(FeatureId(2)),
        before.feature(FeatureId(2))
    );
    assert_eq!(
        document.current().feature_parameter_value(
            &FeatureParameterTarget::new(FeatureId(6), "height", ParameterValueType::Length)
                .unwrap()
        ),
        Some(20.0)
    );
}

#[test]
fn shared_definition_requires_explicit_shared_scope_for_owned_features() {
    let mut document = ketchup_core::document::DocumentStore::new();
    let mut commands = panel_commands(1, 1, 2, 1, "Shared", None);
    commands.push(CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(2),
        definition_id: DefinitionId(1),
        name: "Shared second".to_owned(),
        transform: Transform::from_translation(700.0, 0.0, 0.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    });
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    let occurrence_scoped = AssemblyRecipe::adopt(
        &document.current(),
        key("shared_test"),
        vec![adoption_part(
            "first",
            1,
            1,
            1,
            2,
            19.0,
            RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
        )],
        vec![],
        vec![],
    );
    assert!(matches!(
        occurrence_scoped,
        Err(AssemblyRecipeError::InvalidEditScope(part)) if part.as_str() == "first"
    ));

    let shared = AssemblyRecipe::adopt(
        &document.current(),
        key("shared_test"),
        vec![adoption_part(
            "first",
            1,
            1,
            1,
            2,
            19.0,
            RecipeEditScope::SharedDefinition(DefinitionId(1)),
        )],
        vec![],
        vec![],
    )
    .unwrap();
    assert_eq!(
        shared.parts().next().unwrap().edit_scope,
        RecipeEditScope::SharedDefinition(DefinitionId(1))
    );
}

#[test]
fn adoption_rejects_unknown_version_and_unrecognized_or_partial_feature_sets() {
    let document = populated_document();
    let snapshot = document.current();
    let unknown = AssemblyRecipe::new(
        "ketchup.assembly-recipe.v999",
        key("future"),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    );
    assert_eq!(
        unknown,
        Err(AssemblyRecipeError::UnsupportedVersion(
            "ketchup.assembly-recipe.v999".to_owned()
        ))
    );

    let mut partial = adoption_part(
        "rear",
        1,
        1,
        1,
        2,
        19.0,
        RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
    );
    partial.features.pop();
    assert!(matches!(
        AssemblyRecipe::adopt(&snapshot, key("partial"), vec![partial], vec![], vec![]),
        Err(AssemblyRecipeError::UnrecognizedFeatureSet(part)) if part.as_str() == "rear"
    ));

    let mut wrong_kind = adoption_part(
        "rear",
        1,
        1,
        1,
        2,
        19.0,
        RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
    );
    wrong_kind.features[1].2 = RecognizedRecipeFeatureKind::Pocket;
    assert_eq!(
        AssemblyRecipe::adopt(
            &snapshot,
            key("wrong_kind"),
            vec![wrong_kind],
            vec![],
            vec![]
        ),
        Err(AssemblyRecipeError::UnrecognizedFeature(FeatureId(2)))
    );

    let mut foreign_parameter = adoption_part(
        "rear",
        1,
        1,
        1,
        2,
        19.0,
        RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
    );
    foreign_parameter
        .parameters
        .get_mut(&key("height"))
        .unwrap()
        .target = Some(
        FeatureParameterTarget::new(FeatureId(4), "height", ParameterValueType::Length).unwrap(),
    );
    assert!(matches!(
        AssemblyRecipe::adopt(
            &snapshot,
            key("foreign_parameter"),
            vec![foreign_parameter],
            vec![],
            vec![]
        ),
        Err(AssemblyRecipeError::InvalidParameter(part)) if part.as_str() == "rear"
    ));
}

#[test]
fn adoption_rejects_a_joinery_id_whose_endpoints_do_not_match_declared_parts() {
    let mut document = populated_document();
    document
        .apply_batch(&CommandBatch::new(panel_commands(
            3, 5, 6, 3, "Third", None,
        )))
        .unwrap();
    let snapshot = document.current();
    let result = AssemblyRecipe::adopt(
        &snapshot,
        key("wrong_joinery"),
        vec![
            adoption_part(
                "top",
                2,
                2,
                3,
                4,
                19.0,
                RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(2))),
            ),
            adoption_part(
                "third",
                3,
                3,
                5,
                6,
                19.0,
                RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(3))),
            ),
        ],
        vec![],
        vec![RecipeJoinery {
            key: key("top_third"),
            first_part: key("top"),
            second_part: key("third"),
            dowel_joint_id: DowelJointId(7),
        }],
    );
    assert!(matches!(
        result,
        Err(AssemblyRecipeError::UnresolvedJoinery(item)) if item.as_str() == "top_third"
    ));
}

#[test]
fn document_without_recipe_round_trips_without_inventing_one() {
    let document = populated_document();
    let snapshot = document.current();
    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert!(reopened.snapshot().assembly_recipe().is_none());
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(ASSEMBLY_RECIPE_SCHEMA_V1, "ketchup.assembly-recipe.v1");
}

fn height_patch(value: f64, anchor: RecipeDimensionAnchor) -> RecipeSemanticPatch {
    RecipeSemanticPatch {
        nodes: vec![RecipePatchNode {
            key: key("resize_rear"),
            dependencies: vec![],
            change: RecipeSemanticChange::SetParameter {
                part: key("rear"),
                parameter: key("height"),
                value,
                anchor,
            },
        }],
    }
}

#[test]
fn semantic_compiler_is_a_true_no_op_and_round_trips_a_maximum_anchored_dimension() {
    let mut document = ketchup_core::document::DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(panel_commands(1, 1, 2, 1, "Rear", None)))
        .unwrap();
    let recipe = AssemblyRecipe::adopt(
        &document.current(),
        key("single_panel"),
        vec![adoption_part(
            "rear",
            1,
            1,
            1,
            2,
            19.0,
            RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
        )],
        vec![],
        vec![],
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    document.discard_history_before_current();
    let original_digest = document.current().canonical_digest();
    let original_revision = document.current().revision_id();

    let no_op = compile_assembly_recipe_patch(
        &document.current(),
        &height_patch(19.0, RecipeDimensionAnchor::Maximum),
    )
    .unwrap();
    assert!(no_op.batch.is_none());
    assert!(no_op.affected_parts.is_empty());
    assert_eq!(document.current().revision_id(), original_revision);
    assert_eq!(document.visible_undo_steps(), 0);

    let expanded = compile_assembly_recipe_patch(
        &document.current(),
        &height_patch(29.0, RecipeDimensionAnchor::Maximum),
    )
    .unwrap();
    let batch = expanded.batch.unwrap();
    assert_eq!(batch.commands().len(), 4);
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(
        document.current().feature_parameter_value(
            &FeatureParameterTarget::new(FeatureId(2), "height", ParameterValueType::Length)
                .unwrap()
        ),
        Some(29.0)
    );
    assert_eq!(
        document
            .current()
            .resolve_instance_path(&InstancePath::root(OccurrenceId(1)))
            .unwrap()
            .local_transform
            .matrix()[11],
        -10.0
    );

    let restored = compile_assembly_recipe_patch(
        &document.current(),
        &height_patch(19.0, RecipeDimensionAnchor::Maximum),
    )
    .unwrap();
    document.apply_batch(&restored.batch.unwrap()).unwrap();
    assert_eq!(document.current().canonical_digest(), original_digest);
    assert_eq!(
        document
            .current()
            .resolve_instance_path(&InstancePath::root(OccurrenceId(1)))
            .unwrap()
            .local_transform,
        Transform::identity()
    );
}

#[test]
fn semantic_compiler_rejects_cycles_conflicts_and_unsupported_parameters_before_publication() {
    let mut document = populated_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(adopt_recipe(&document.current())),
        ]))
        .unwrap();
    document.discard_history_before_current();
    let before = document.current().canonical_digest();
    let node = |node_key: &str, dependency: &str, value| RecipePatchNode {
        key: key(node_key),
        dependencies: vec![key(dependency)],
        change: RecipeSemanticChange::SetParameter {
            part: key("rear"),
            parameter: key("height"),
            value,
            anchor: RecipeDimensionAnchor::Minimum,
        },
    };
    let cycle = RecipeSemanticPatch {
        nodes: vec![node("first", "second", 20.0), node("second", "first", 21.0)],
    };
    assert!(matches!(
        compile_assembly_recipe_patch(&document.current(), &cycle),
        Err(AssemblyRecipeCompileError::DependencyCycle(node)) if node.as_str() == "first"
    ));

    let conflict = RecipeSemanticPatch {
        nodes: vec![
            RecipePatchNode {
                dependencies: vec![],
                ..node("first", "second", 20.0)
            },
            RecipePatchNode {
                dependencies: vec![key("first")],
                ..node("second", "first", 21.0)
            },
        ],
    };
    assert!(matches!(
        compile_assembly_recipe_patch(&document.current(), &conflict),
        Err(AssemblyRecipeCompileError::ConflictingWrites { part, parameter })
            if part.as_str() == "rear" && parameter.as_str() == "height"
    ));
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 0);
}

fn rotated_contact_document() -> ketchup_core::document::DocumentStore {
    let mut document = ketchup_core::document::DocumentStore::new();
    let mut commands = panel_commands(1, 1, 2, 1, "Rear", None);
    commands.extend(panel_commands(2, 3, 4, 2, "Top", None));
    commands.extend(panel_commands(3, 5, 6, 3, "Independent", None));
    commands.extend([
        CanonicalCommand::CreateGroup {
            id: GroupId(10),
            name: "Rotated assembly".to_owned(),
            transform: Transform::from_matrix([
                0.0, 0.0, 1.0, 100.0, 0.0, 1.0, 0.0, 200.0, -1.0, 0.0, 0.0, 300.0, 0.0, 0.0, 0.0,
                1.0,
            ])
            .unwrap(),
            parent: None,
        },
        CanonicalCommand::CreateGroup {
            id: GroupId(11),
            name: "Nested assembly".to_owned(),
            transform: Transform::from_translation(10.0, 20.0, 30.0).unwrap(),
            parent: Some(GroupId(10)),
        },
        CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(2),
            transform: Transform::from_translation(0.0, 0.0, 29.0).unwrap(),
        },
        CanonicalCommand::SetOccurrenceParent {
            id: OccurrenceId(1),
            parent: Some(GroupId(11)),
        },
        CanonicalCommand::SetOccurrenceParent {
            id: OccurrenceId(2),
            parent: Some(GroupId(11)),
        },
    ]);
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let recipe = AssemblyRecipe::adopt(
        &document.current(),
        key("rotated_contact"),
        vec![
            adoption_part(
                "rear",
                1,
                1,
                1,
                2,
                19.0,
                RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
            ),
            adoption_part(
                "top",
                2,
                2,
                3,
                4,
                19.0,
                RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(2))),
            ),
        ],
        vec![RecipeRelation {
            key: key("rear_top_contact"),
            kind: RecipeRelationKind::Contact,
            first: RecipeFaceRef {
                part: key("rear"),
                role: "bounds.z.maximum".to_owned(),
            },
            second: RecipeFaceRef {
                part: key("top"),
                role: "bounds.z.minimum".to_owned(),
            },
        }],
        vec![],
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn extend_rear_to_top_patch() -> RecipeSemanticPatch {
    RecipeSemanticPatch {
        nodes: vec![RecipePatchNode {
            key: key("extend_rear"),
            dependencies: vec![],
            change: RecipeSemanticChange::ExtendUntilContact {
                part: key("rear"),
                parameter: key("height"),
                relation: key("rear_top_contact"),
                anchor: RecipeDimensionAnchor::Minimum,
            },
        }],
    }
}

#[test]
fn independent_semantic_edits_commute_and_repeat_without_geometry_or_identity_drift() {
    for (rear_height, top_height) in [(29.0, 39.0), (9.0, 27.0)] {
        let nodes = vec![
            RecipePatchNode {
                key: key("resize_rear"),
                dependencies: vec![],
                change: RecipeSemanticChange::SetParameter {
                    part: key("rear"),
                    parameter: key("height"),
                    value: rear_height,
                    anchor: RecipeDimensionAnchor::Maximum,
                },
            },
            RecipePatchNode {
                key: key("resize_top"),
                dependencies: vec![],
                change: RecipeSemanticChange::SetParameter {
                    part: key("top"),
                    parameter: key("height"),
                    value: top_height,
                    anchor: RecipeDimensionAnchor::Centre,
                },
            },
        ];
        let baseline = persistence::save_document_store(
            &rotated_contact_document(),
            &persistence::ContainerData::default(),
        )
        .unwrap();
        let mut expected_digest = None;
        for (reverse, separate_batches) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let mut document = persistence::load(&baseline)
                .unwrap()
                .into_editable()
                .ok()
                .unwrap();
            let before = document.current();
            let mut ordered = nodes.clone();
            if reverse {
                ordered.reverse();
            }
            let patches = if separate_batches {
                ordered
                    .into_iter()
                    .map(|node| RecipeSemanticPatch { nodes: vec![node] })
                    .collect()
            } else {
                vec![RecipeSemanticPatch { nodes: ordered }]
            };
            for patch in &patches {
                let compiled = compile_assembly_recipe_patch(&document.current(), patch).unwrap();
                document.apply_batch(&compiled.batch.unwrap()).unwrap();
            }
            let after = document.current();
            // Independent oracle: the fixture maps local (x,y,z) to (130+z,220+y,290-x).
            // Rear keeps its maximum at world X=149; top keeps its centre at X=168.5.
            for (occurrence, extrusion, height, minimum_x, maximum_x, local_z) in [
                (
                    1,
                    2,
                    rear_height,
                    149.0 - rear_height,
                    149.0,
                    19.0 - rear_height,
                ),
                (
                    2,
                    4,
                    top_height,
                    168.5 - top_height / 2.0,
                    168.5 + top_height / 2.0,
                    38.5 - top_height / 2.0,
                ),
            ] {
                let FeatureKind::Extrusion {
                    height: actual_height,
                    ..
                } = after.feature(FeatureId(extrusion)).unwrap().kind()
                else {
                    panic!("panel extrusion was replaced")
                };
                assert_eq!(actual_height.millimetres(), height);
                let instance = after
                    .resolve_instance_path(&InstancePath::root(OccurrenceId(occurrence)))
                    .unwrap();
                assert_eq!(
                    instance.local_transform,
                    Transform::from_translation(0.0, 0.0, local_z).unwrap()
                );
                let matrix = instance.world_transform.matrix();
                assert_eq!(
                    matrix,
                    &[
                        0.0, 0.0, 1.0, minimum_x, 0.0, 1.0, 0.0, 220.0, -1.0, 0.0, 0.0, 290.0, 0.0,
                        0.0, 0.0, 1.0
                    ]
                );
                let FeatureKind::Profile { points_mm } =
                    after.feature(FeatureId(extrusion - 1)).unwrap().kind()
                else {
                    panic!("panel profile was replaced")
                };
                assert_eq!(
                    points_mm,
                    &vec![[0.0, 0.0], [600.0, 0.0], [600.0, 400.0], [0.0, 400.0]]
                );
                let mut bounds_min = [f64::INFINITY; 3];
                let mut bounds_max = [f64::NEG_INFINITY; 3];
                for [x, y] in points_mm {
                    for z in [0.0, actual_height.millimetres()] {
                        for axis in 0..3 {
                            let row = axis * 4;
                            let world = matrix[row] * x
                                + matrix[row + 1] * y
                                + matrix[row + 2] * z
                                + matrix[row + 3];
                            bounds_min[axis] = bounds_min[axis].min(world);
                            bounds_max[axis] = bounds_max[axis].max(world);
                        }
                    }
                }
                assert_eq!(bounds_min, [minimum_x, 220.0, -310.0]);
                assert_eq!(bounds_max, [maximum_x, 620.0, 290.0]);
            }
            assert_eq!(after.scene_query().len(), 3);
            for id in 1..=3 {
                assert_eq!(
                    after.definition(DefinitionId(id)).unwrap().feature_ids(),
                    before.definition(DefinitionId(id)).unwrap().feature_ids()
                );
            }
            for id in [1, 3, 5, 6] {
                assert_eq!(after.feature(FeatureId(id)), before.feature(FeatureId(id)));
            }
            assert_eq!(
                after.occurrence(OccurrenceId(3)),
                before.occurrence(OccurrenceId(3))
            );
            assert_eq!(after.assembly_recipe().unwrap().parts().count(), 2);
            assert_eq!(
                document.visible_undo_steps(),
                if separate_batches { 2 } else { 1 }
            );
            let digest = after.canonical_digest();
            if let Some(expected) = &expected_digest {
                assert_eq!(&digest, expected);
            } else {
                expected_digest = Some(digest.clone());
            }
            let revision = after.revision_id();
            for _ in 0..3 {
                let repeated = compile_assembly_recipe_patch(
                    &document.current(),
                    &RecipeSemanticPatch {
                        nodes: nodes.clone(),
                    },
                )
                .unwrap();
                assert!(repeated.batch.is_none());
                assert!(repeated.affected_parts.is_empty());
                assert_eq!(document.current().canonical_digest(), digest);
                assert_eq!(document.current().revision_id(), revision);
                assert_eq!(
                    document.visible_undo_steps(),
                    if separate_batches { 2 } else { 1 }
                );
            }
        }
    }
}

#[test]
fn extend_until_contact_handles_nested_rotation_and_matches_a_clean_compile() {
    let mut incremental = rotated_contact_document();
    let baseline =
        persistence::save_document_store(&incremental, &persistence::ContainerData::default())
            .unwrap();
    let independent_before = incremental
        .current()
        .occurrence(OccurrenceId(3))
        .unwrap()
        .clone();
    let compilation =
        compile_assembly_recipe_patch(&incremental.current(), &extend_rear_to_top_patch()).unwrap();
    assert_eq!(compilation.affected_parts, [key("rear")].into());
    incremental
        .apply_batch(&compilation.batch.unwrap())
        .unwrap();
    assert_eq!(incremental.visible_undo_steps(), 1);
    assert_eq!(
        incremental.current().feature_parameter_value(
            &FeatureParameterTarget::new(FeatureId(2), "height", ParameterValueType::Length)
                .unwrap()
        ),
        Some(29.0)
    );
    assert_eq!(
        incremental.current().occurrence(OccurrenceId(3)).unwrap(),
        &independent_before
    );
    let rear = incremental
        .current()
        .resolve_instance_path(&InstancePath::root(OccurrenceId(1)))
        .unwrap();
    let top = incremental
        .current()
        .resolve_instance_path(&InstancePath::root(OccurrenceId(2)))
        .unwrap();
    let transform_point = |transform: Transform, point: [f64; 3]| {
        let matrix = transform.matrix();
        [
            matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2] + matrix[3],
            matrix[4] * point[0] + matrix[5] * point[1] + matrix[6] * point[2] + matrix[7],
            matrix[8] * point[0] + matrix[9] * point[1] + matrix[10] * point[2] + matrix[11],
        ]
    };
    assert_eq!(
        transform_point(rear.world_transform, [300.0, 200.0, 29.0]),
        transform_point(top.world_transform, [300.0, 200.0, 0.0])
    );

    let repeated =
        compile_assembly_recipe_patch(&incremental.current(), &extend_rear_to_top_patch()).unwrap();
    assert!(repeated.batch.is_none());
    assert!(repeated.affected_parts.is_empty());
    assert_eq!(incremental.visible_undo_steps(), 1);

    let mut clean = persistence::load(&baseline)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let direct = compile_assembly_recipe_patch(
        &clean.current(),
        &height_patch(29.0, RecipeDimensionAnchor::Minimum),
    )
    .unwrap();
    clean.apply_batch(&direct.batch.unwrap()).unwrap();
    assert_eq!(
        incremental.current().assembly_recipe(),
        clean.current().assembly_recipe()
    );
    assert_eq!(
        incremental.current().scene_query(),
        clean.current().scene_query()
    );
    for feature_id in 1..=6 {
        assert_eq!(
            incremental.current().feature(FeatureId(feature_id)),
            clean.current().feature(FeatureId(feature_id))
        );
    }
    assert_eq!(
        incremental.current().canonical_digest(),
        clean.current().canonical_digest()
    );
}

fn contact_document(target_x_mm: f64, moving_role: &str) -> ketchup_core::document::DocumentStore {
    let mut document = ketchup_core::document::DocumentStore::new();
    let mut commands = panel_commands(1, 1, 2, 1, "Rear", None);
    commands.extend(panel_commands(2, 3, 4, 2, "Top", None));
    commands.push(CanonicalCommand::SetOccurrenceTransform {
        id: OccurrenceId(2),
        transform: Transform::from_translation(target_x_mm, 0.0, 29.0).unwrap(),
    });
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let recipe = AssemblyRecipe::adopt(
        &document.current(),
        key("contact_diagnostic"),
        vec![
            adoption_part(
                "rear",
                1,
                1,
                1,
                2,
                19.0,
                RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
            ),
            adoption_part(
                "top",
                2,
                2,
                3,
                4,
                19.0,
                RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(2))),
            ),
        ],
        vec![RecipeRelation {
            key: key("rear_top_contact"),
            kind: RecipeRelationKind::Contact,
            first: RecipeFaceRef {
                part: key("rear"),
                role: moving_role.to_owned(),
            },
            second: RecipeFaceRef {
                part: key("top"),
                role: "bounds.z.minimum".to_owned(),
            },
        }],
        vec![],
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

#[test]
fn extend_until_contact_rejects_non_overlapping_faces_without_mutation() {
    let document = contact_document(1_000.0, "bounds.z.maximum");
    let digest = document.current().canonical_digest();
    assert!(matches!(
        compile_assembly_recipe_patch(&document.current(), &extend_rear_to_top_patch()),
        Err(AssemblyRecipeCompileError::AmbiguousContact(relation))
            if relation.as_str() == "rear_top_contact"
    ));
    assert_eq!(document.current().canonical_digest(), digest);
    assert_eq!(document.visible_undo_steps(), 0);
}

#[test]
fn extend_until_contact_rejects_an_unsupported_face_role_without_mutation() {
    let document = contact_document(0.0, "named.top");
    let digest = document.current().canonical_digest();
    assert!(matches!(
        compile_assembly_recipe_patch(&document.current(), &extend_rear_to_top_patch()),
        Err(AssemblyRecipeCompileError::UnsupportedFaceRole(role)) if role == "named.top"
    ));
    assert_eq!(document.current().canonical_digest(), digest);
    assert_eq!(document.visible_undo_steps(), 0);
}

fn physical_hole_workplane(
    id: u64,
    definition_id: u64,
    name: &str,
    origin_mm: [f64; 3],
    normal_z: f64,
) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id: FeatureId(id),
        definition_id: DefinitionId(definition_id),
        name: name.to_owned(),
        kind: FeatureKind::Workplane(WorkplaneSpec {
            support: WorkplaneSupport::Free,
            frame: WorkplaneFrame {
                origin_mm,
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, normal_z, 0.0],
                normal: [0.0, 0.0, normal_z],
            },
        }),
    }
}

fn physical_hole_sketch(
    id: u64,
    definition_id: u64,
    workplane_id: u64,
    name: &str,
) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id: FeatureId(id),
        definition_id: DefinitionId(definition_id),
        name: name.to_owned(),
        kind: FeatureKind::Sketch(SketchSpec {
            workplane: FeatureId(workplane_id),
            entities: vec![SketchEntity::Circle {
                id: SketchEntityId(1),
                center_mm: [0.0, 0.0],
                radius_mm: 4.0,
            }],
            constraints: vec![],
        }),
    }
}

fn physical_hole_pocket(
    id: u64,
    definition_id: u64,
    target_id: u64,
    profile_id: u64,
    name: &str,
) -> CanonicalCommand {
    CanonicalCommand::CreateFeature {
        id: FeatureId(id),
        definition_id: DefinitionId(definition_id),
        name: name.to_owned(),
        kind: FeatureKind::Pocket {
            target: FeatureId(target_id),
            profile: FeatureId(profile_id),
            depth: Dimension::new("16", 16.0).unwrap(),
        },
    }
}

fn dependent_joinery_document() -> ketchup_core::document::DocumentStore {
    dependent_joinery_document_with_physical_holes(false)
}

fn dependent_joinery_document_with_physical_holes(
    physical_holes: bool,
) -> ketchup_core::document::DocumentStore {
    let mut document = ketchup_core::document::DocumentStore::new();
    let mut commands = panel_commands(1, 1, 2, 1, "Rear", None);
    commands.extend(panel_commands(2, 3, 4, 2, "Top", None));
    commands.extend([
        CanonicalCommand::SetFeatureDimension {
            id: FeatureId(4),
            dimension: Dimension::new("40", 40.0).unwrap(),
        },
        CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(2),
            transform: Transform::from_translation(0.0, 0.0, 19.0).unwrap(),
        },
    ]);
    if physical_holes {
        commands.extend([
            physical_hole_workplane(5, 1, "Rear hole plane", [50.0, 20.0, 19.0], -1.0),
            physical_hole_sketch(6, 1, 5, "Rear hole sketch"),
            physical_hole_pocket(7, 1, 2, 6, "Rear hole pocket"),
            physical_hole_workplane(8, 2, "Top hole plane", [50.0, 20.0, 0.0], 1.0),
            physical_hole_sketch(9, 2, 8, "Top hole sketch"),
            physical_hole_pocket(10, 2, 4, 9, "Top hole pocket"),
        ]);
    }
    commands.push(CanonicalCommand::UpsertDowelJoint(DowelJointContract {
        id: DowelJointId(9),
        name: "dependent-row".to_owned(),
        first: DowelJointFace {
            instance_path: InstancePath::root(OccurrenceId(1)),
            face_origin_local_mm: [0.0, 0.0, 19.0],
            inward_unit_local: [0.0, 0.0, -1.0],
            bounds_min_local_mm: [0.0, 0.0, 0.0],
            bounds_max_local_mm: [600.0, 400.0, 19.0],
        },
        second: DowelJointFace {
            instance_path: InstancePath::root(OccurrenceId(2)),
            face_origin_local_mm: [0.0, 0.0, 0.0],
            inward_unit_local: [0.0, 0.0, 1.0],
            bounds_min_local_mm: [0.0, 0.0, 0.0],
            bounds_max_local_mm: [600.0, 400.0, 40.0],
        },
        first_center_local_mm: [50.0, 20.0, 19.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count: if physical_holes { 1 } else { 3 },
        spacing_mm: 32.0,
        dowel: StandardDowel::D8x30.symmetric_spec(),
        physical_hole_pairs: physical_holes.then(|| {
            vec![DowelPhysicalHolePair {
                first_pocket_feature_id: FeatureId(7),
                second_pocket_feature_id: FeatureId(10),
            }]
        }),
    }));
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let mut rear = adoption_part(
        "rear",
        1,
        1,
        1,
        2,
        19.0,
        RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(1))),
    );
    let mut top = adoption_part(
        "top",
        2,
        2,
        3,
        4,
        40.0,
        RecipeEditScope::Occurrence(InstancePath::root(OccurrenceId(2))),
    );
    if physical_holes {
        rear.features.extend([
            (
                key("rear/hole_plane"),
                FeatureId(5),
                RecognizedRecipeFeatureKind::Workplane,
            ),
            (
                key("rear/hole_sketch"),
                FeatureId(6),
                RecognizedRecipeFeatureKind::Sketch,
            ),
            (
                key("rear/hole_pocket"),
                FeatureId(7),
                RecognizedRecipeFeatureKind::Pocket,
            ),
        ]);
        top.features.extend([
            (
                key("top/hole_plane"),
                FeatureId(8),
                RecognizedRecipeFeatureKind::Workplane,
            ),
            (
                key("top/hole_sketch"),
                FeatureId(9),
                RecognizedRecipeFeatureKind::Sketch,
            ),
            (
                key("top/hole_pocket"),
                FeatureId(10),
                RecognizedRecipeFeatureKind::Pocket,
            ),
        ]);
    }
    let recipe = AssemblyRecipe::adopt(
        &document.current(),
        key("dependent_joinery"),
        vec![rear, top],
        vec![RecipeRelation {
            key: key("rear_top_contact"),
            kind: RecipeRelationKind::Contact,
            first: RecipeFaceRef {
                part: key("rear"),
                role: "bounds.z.maximum".to_owned(),
            },
            second: RecipeFaceRef {
                part: key("top"),
                role: "bounds.z.minimum".to_owned(),
            },
        }],
        vec![RecipeJoinery {
            key: key("rear_top_dowels"),
            first_part: key("rear"),
            second_part: key("top"),
            dowel_joint_id: DowelJointId(9),
        }],
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn dependent_joinery_patch() -> RecipeSemanticPatch {
    RecipeSemanticPatch {
        nodes: vec![
            RecipePatchNode {
                key: key("move_target"),
                dependencies: vec![],
                change: RecipeSemanticChange::SetParameter {
                    part: key("top"),
                    parameter: key("height"),
                    value: 30.0,
                    anchor: RecipeDimensionAnchor::Maximum,
                },
            },
            RecipePatchNode {
                key: key("extend_rear"),
                dependencies: vec![key("move_target")],
                change: RecipeSemanticChange::ExtendUntilContact {
                    part: key("rear"),
                    parameter: key("height"),
                    relation: key("rear_top_contact"),
                    anchor: RecipeDimensionAnchor::Minimum,
                },
            },
        ],
    }
}

#[test]
fn dependent_joinery_is_rebound_with_the_same_identity_in_one_undo_step() {
    let mut document = dependent_joinery_document();
    let original_digest = document.current().canonical_digest();
    let patch = dependent_joinery_patch();
    let compilation = compile_assembly_recipe_patch(&document.current(), &patch).unwrap();
    assert_eq!(compilation.affected_parts, [key("rear"), key("top")].into());
    document.apply_batch(&compilation.batch.unwrap()).unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    let snapshot = document.current();
    let joint = snapshot.dowel_joint(DowelJointId(9)).unwrap();
    assert_eq!(joint.first.face_origin_local_mm[2], 29.0);
    assert_eq!(joint.first_center_local_mm[2], 29.0);
    assert_eq!(joint.first.bounds_max_local_mm[2], 29.0);
    assert_eq!(joint.second.bounds_max_local_mm[2], 30.0);
    assert_eq!(document.current().scene_query().len(), 2);
    assert_eq!(
        document
            .current()
            .definition(DefinitionId(1))
            .unwrap()
            .feature_ids()
            .len(),
        2
    );
    assert_eq!(
        document
            .current()
            .definition(DefinitionId(2))
            .unwrap()
            .feature_ids()
            .len(),
        2
    );
    ketchup_core::joinery::project_dowel_joint_contract(&document.current(), joint).unwrap();

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), original_digest);
    assert_eq!(
        document
            .current()
            .dowel_joint(DowelJointId(9))
            .unwrap()
            .first_center_local_mm[2],
        19.0
    );
    document.redo().unwrap();
    assert_eq!(
        document
            .current()
            .dowel_joint(DowelJointId(9))
            .unwrap()
            .first_center_local_mm[2],
        29.0
    );
}

#[test]
fn dependent_physical_holes_follow_rebound_joinery_without_changing_identity() {
    let mut document = dependent_joinery_document_with_physical_holes(true);
    let original_digest = document.current().canonical_digest();
    let compilation =
        compile_assembly_recipe_patch(&document.current(), &dependent_joinery_patch()).unwrap();
    let batch = compilation.batch.unwrap();
    assert_eq!(
        batch
            .commands()
            .iter()
            .filter(|command| matches!(
                command,
                CanonicalCommand::SetFeatureParameter { target, .. }
                    if target.feature_id == FeatureId(5)
                        && target.path.as_str() == "frame.origin.z"
            ))
            .count(),
        1
    );
    assert!(
        !batch
            .commands()
            .iter()
            .any(|command| matches!(command, CanonicalCommand::DeleteFeature { .. }))
    );

    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    let snapshot = document.current();
    let joint = snapshot.dowel_joint(DowelJointId(9)).unwrap();
    assert_eq!(joint.id, DowelJointId(9));
    assert_eq!(
        joint.physical_hole_pairs,
        Some(vec![DowelPhysicalHolePair {
            first_pocket_feature_id: FeatureId(7),
            second_pocket_feature_id: FeatureId(10),
        }])
    );
    let FeatureKind::Workplane(rear_hole_plane) = snapshot.feature(FeatureId(5)).unwrap().kind()
    else {
        panic!("rear hole workplane was replaced")
    };
    assert_eq!(rear_hole_plane.frame.origin_mm, [50.0, 20.0, 29.0]);
    for _ in 0..3 {
        let repeated =
            compile_assembly_recipe_patch(&document.current(), &dependent_joinery_patch()).unwrap();
        assert!(repeated.batch.is_none());
        assert!(repeated.affected_parts.is_empty());
        assert_eq!(
            document.current().canonical_digest(),
            snapshot.canonical_digest()
        );
        assert_eq!(document.current().revision_id(), snapshot.revision_id());
        assert_eq!(document.current().dowel_joints().count(), 1);
        assert_eq!(
            document
                .current()
                .definition(DefinitionId(1))
                .unwrap()
                .feature_ids()
                .len(),
            5
        );
        assert_eq!(
            document
                .current()
                .definition(DefinitionId(2))
                .unwrap()
                .feature_ids()
                .len(),
            5
        );
        assert_eq!(document.visible_undo_steps(), 1);
    }
    let projection = ketchup_core::joinery::project_dowel_joint_contract(&snapshot, joint).unwrap();
    assert_eq!(projection.pairs.len(), 1);
    assert_eq!(
        projection.pairs[0]
            .physical_probe_coincidence
            .unwrap()
            .maximum_endpoint_error_mm,
        0.0
    );
    for feature_id in 1..=10 {
        assert!(snapshot.feature(FeatureId(feature_id)).is_some());
    }
    snapshot
        .assembly_recipe()
        .unwrap()
        .audit(&snapshot)
        .unwrap();

    document.undo().unwrap();
    assert_eq!(document.current().canonical_digest(), original_digest);
    document.redo().unwrap();
    let redone = document.current();
    ketchup_core::joinery::project_dowel_joint_contract(
        &redone,
        redone.dowel_joint(DowelJointId(9)).unwrap(),
    )
    .unwrap();
}

#[test]
fn physical_joinery_recipe_and_probe_evidence_survive_history_round_trip() {
    let mut document = dependent_joinery_document_with_physical_holes(true);
    let before_digest = document.current().canonical_digest();
    let compilation =
        compile_assembly_recipe_patch(&document.current(), &dependent_joinery_patch()).unwrap();
    document.apply_batch(&compilation.batch.unwrap()).unwrap();
    let after = document.current();
    let after_digest = after.canonical_digest();
    let expected_recipe = after.assembly_recipe().unwrap().clone();
    let expected_joint = after.dowel_joint(DowelJointId(9)).unwrap().clone();

    let bytes = persistence::save_document_store(&document, &persistence::ContainerData::default())
        .unwrap();
    let mut reopened = persistence::load(&bytes)
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    let reopened_snapshot = reopened.current();
    assert_eq!(reopened_snapshot.canonical_digest(), after_digest);
    assert_eq!(reopened_snapshot.assembly_recipe(), Some(&expected_recipe));
    assert_eq!(
        reopened_snapshot.dowel_joint(DowelJointId(9)),
        Some(&expected_joint)
    );
    let projection = ketchup_core::joinery::project_dowel_joint_contract(
        &reopened_snapshot,
        reopened_snapshot.dowel_joint(DowelJointId(9)).unwrap(),
    )
    .unwrap();
    assert_eq!(projection.pairs.len(), 1);
    assert_eq!(
        projection.pairs[0]
            .physical_probe_coincidence
            .unwrap()
            .maximum_endpoint_error_mm,
        0.0
    );

    assert_eq!(reopened.visible_undo_steps(), 1);
    assert_eq!(reopened.undo().unwrap().canonical_digest(), before_digest);
    assert_eq!(reopened.visible_redo_steps(), 1);
    assert_eq!(reopened.redo().unwrap().canonical_digest(), after_digest);
    let redone = reopened.current();
    let redone_projection = ketchup_core::joinery::project_dowel_joint_contract(
        &redone,
        redone.dowel_joint(DowelJointId(9)).unwrap(),
    )
    .unwrap();
    assert!(
        redone_projection.pairs[0]
            .physical_probe_coincidence
            .is_some()
    );
}

#[test]
fn shared_definition_anchor_moves_every_instance_without_changing_identity() {
    let mut document = ketchup_core::document::DocumentStore::new();
    let mut commands = panel_commands(1, 1, 2, 1, "Shared", None);
    commands.push(CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(2),
        definition_id: DefinitionId(1),
        name: "Shared second".to_owned(),
        transform: Transform::from_translation(100.0, 0.0, 30.0).unwrap(),
        parent: None,
        tag: None,
        visible: true,
    });
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let recipe = AssemblyRecipe::adopt(
        &document.current(),
        key("shared_test"),
        vec![adoption_part(
            "rear",
            1,
            1,
            1,
            2,
            19.0,
            RecipeEditScope::SharedDefinition(DefinitionId(1)),
        )],
        vec![],
        vec![],
    )
    .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetAssemblyRecipe(recipe),
        ]))
        .unwrap();
    let compilation = compile_assembly_recipe_patch(
        &document.current(),
        &height_patch(29.0, RecipeDimensionAnchor::Maximum),
    )
    .unwrap();
    document.apply_batch(&compilation.batch.unwrap()).unwrap();

    assert!(document.current().definition(DefinitionId(1)).is_some());
    assert_eq!(document.current().scene_query().len(), 2);
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .transform()
            .matrix()[11],
        -10.0
    );
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(2))
            .unwrap()
            .transform()
            .matrix()[11],
        20.0
    );
    document
        .current()
        .assembly_recipe()
        .unwrap()
        .audit(&document.current())
        .unwrap();
}
