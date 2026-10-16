use super::*;

#[test]
fn one_physical_pin_joint_operation_creates_both_hole_rows_atomically() {
    let mut document = DocumentStore::new();
    let panel = |name: &str, translation_mm| {
        cuboid_part(
            name.into(),
            [100.0, 50.0, 18.0],
            Vec::new(),
            Vec::new(),
            translation_mm,
            None,
        )
    };
    let panels = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            panel("Lower panel", [0.0, 0.0, 0.0]),
            panel("Upper panel", [0.0, 0.0, 18.0]),
        ]),
    )
    .unwrap();
    document.apply_batch(&panels).unwrap();
    let baseline = document.current();
    let undo_before = document.visible_undo_steps();
    let face = |occurrence_id, face_z, inward_unit_local| AssistantPinJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        }
        .into(),
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let input = program(vec![AssistantCadEditOperation::CreatePinJoint {
        joint_id: None,
        name: "Physical shelf row".into(),
        first: face(1, 18.0, [0.0, 0.0, -1.0]),
        second: face(2, 0.0, [0.0, 0.0, 1.0]),
        first_center_local_mm: [20.0, 20.0, 18.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count: 3,
        spacing_mm: 25.0,
        pin: AssistantPin {
            diameter_mm: 8.0,
            length_mm: 30.0,
            hole_clearance_mm: 1.0,
        },
        holes: AssistantPinHoles::Drill {
            first_insertion_mm: None,
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
    assert_eq!(
        batch
            .commands()
            .iter()
            .filter(|command| matches!(
                command,
                CanonicalCommand::CreateFeature {
                    kind: FeatureKind::Pad(PadSpec {
                        profile: PadProfile::Feature(_),
                        extent: FeatureExtent::Blind(_),
                        operation: PadOperation::Cut { .. },
                        ..
                    }),
                    ..
                }
            ))
            .count(),
        6
    );
    assert_eq!(
        batch
            .commands()
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::UpsertPinJoint(_)))
            .count(),
        1
    );

    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before + 1);
    let committed = document.current();
    let committed_digest = committed.canonical_digest();
    let joint = committed
        .pin_joint(ketchup_model::pin_joint::PinJointId(1))
        .unwrap();
    let bindings = joint.physical_hole_pairs.as_ref().unwrap();
    assert_eq!(bindings.len(), 3);
    assert!(bindings.iter().all(|binding| {
        matches!(
            committed
                .feature(binding.first_pocket_feature_id)
                .unwrap()
                .kind(),
            FeatureKind::Pad(PadSpec {
                profile: PadProfile::Feature(_),
                extent: FeatureExtent::Blind(_),
                operation: PadOperation::Cut { .. },
                ..
            })
        ) && matches!(
            committed
                .feature(binding.second_pocket_feature_id)
                .unwrap()
                .kind(),
            FeatureKind::Pad(PadSpec {
                profile: PadProfile::Feature(_),
                extent: FeatureExtent::Blind(_),
                operation: PadOperation::Cut { .. },
                ..
            })
        )
    }));
    let projection =
        ketchup_model::pin_joint::project_pin_joint_contract(&committed, joint).unwrap();
    assert_eq!(projection.pairs.len(), 3);
    assert!(projection.pairs.iter().all(|pair| {
        pair.physical_probe_coincidence
            .as_ref()
            .is_some_and(|probe| probe.maximum_endpoint_error_mm == 0.0)
    }));

    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(document.current().canonical_digest(), committed_digest);

    let update = program(vec![AssistantCadEditOperation::CreatePinJoint {
        joint_id: Some(1),
        name: "Physical shelf row".into(),
        first: face(1, 18.0, [0.0, 0.0, -1.0]),
        second: face(2, 0.0, [0.0, 0.0, 1.0]),
        first_center_local_mm: [25.0, 20.0, 18.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count: 2,
        spacing_mm: 30.0,
        pin: AssistantPin {
            diameter_mm: 8.0,
            length_mm: 30.0,
            hole_clearance_mm: 1.0,
        },
        holes: AssistantPinHoles::Drill {
            first_insertion_mm: None,
        },
    }]);
    let update_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &update,
    )
    .unwrap();
    assert_eq!(
        update_batch
            .commands()
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::DeletePinJoint { .. }))
            .count(),
        1
    );
    assert_eq!(
        update_batch
            .commands()
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::DeleteFeature { .. }))
            .count(),
        18
    );
    assert_eq!(
        update_batch
            .commands()
            .iter()
            .filter(|command| matches!(
                command,
                CanonicalCommand::CreateFeature {
                    kind: FeatureKind::Pad(PadSpec {
                        profile: PadProfile::Feature(_),
                        extent: FeatureExtent::Blind(_),
                        operation: PadOperation::Cut { .. },
                        ..
                    }),
                    ..
                }
            ))
            .count(),
        4
    );
    let undo_before_update = document.visible_undo_steps();
    document.apply_batch(&update_batch).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before_update + 1);
    let updated = document.current();
    assert!(
        baseline
            .features()
            .all(|feature| updated.feature(feature.id()).is_some())
    );
    let updated_joint = updated
        .pin_joint(ketchup_model::pin_joint::PinJointId(1))
        .unwrap();
    assert_eq!(updated_joint.physical_hole_pairs.as_ref().unwrap().len(), 2);
    let updated_projection =
        ketchup_model::pin_joint::project_pin_joint_contract(&updated, updated_joint).unwrap();
    assert!(updated_projection.pairs.iter().all(|pair| {
        pair.physical_probe_coincidence
            .as_ref()
            .is_some_and(|probe| probe.maximum_endpoint_error_mm == 0.0)
    }));

    let revision_before_repeat = updated.revision_id();
    let undo_before_repeat = document.visible_undo_steps();
    let repeated = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &update,
    )
    .unwrap();
    assert!(repeated.commands().is_empty());
    assert_eq!(document.current().revision_id(), revision_before_repeat);
    assert_eq!(document.visible_undo_steps(), undo_before_repeat);

    let delete_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::DeletePhysicalPinJoint {
            joint_id: 1,
        }]),
    )
    .unwrap();
    document.apply_batch(&delete_batch).unwrap();
    let deleted = document.current();
    assert!(
        deleted
            .pin_joint(ketchup_model::pin_joint::PinJointId(1))
            .is_none()
    );
    assert_eq!(deleted.features().count(), baseline.features().count());
    assert!(
        baseline
            .features()
            .all(|feature| deleted.feature(feature.id()).is_some())
    );

    document.undo().unwrap();
    let restored = document.current();
    let owned_pocket = restored
        .pin_joint(ketchup_model::pin_joint::PinJointId(1))
        .unwrap()
        .physical_hole_pairs
        .as_ref()
        .unwrap()[0]
        .first_pocket_feature_id;
    let definition_id = restored.feature(owned_pocket).unwrap().definition_id();
    let manual_feature_id = FeatureId(
        restored
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap()
            + 1,
    );
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: manual_feature_id,
            definition_id,
            name: "Manual downstream feature".into(),
            kind: FeatureKind::RigidTransform {
                target: owned_pocket,
                transform: Transform::identity(),
            },
        }]))
        .unwrap();
    let guarded = document.current();
    let guarded_digest = guarded.canonical_digest();
    let guarded_undo = document.visible_undo_steps();
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &program(vec![AssistantCadEditOperation::DeletePhysicalPinJoint {
                joint_id: 1,
            }]),
        )
        .is_err()
    );
    assert_eq!(document.current().canonical_digest(), guarded_digest);
    assert_eq!(document.visible_undo_steps(), guarded_undo);
}

#[test]
fn physical_pin_joint_geometry_regressions_fail_closed_without_mutation() {
    let seed = |lower_holes: Vec<AssistantPartHole>, upper_z: f64| {
        let mut document = DocumentStore::new();
        let panel = |name: &str, translation_mm, holes| {
            cuboid_part(
                name.into(),
                [100.0, 50.0, 18.0],
                holes,
                Vec::new(),
                translation_mm,
                None,
            )
        };
        let batch = plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &program(vec![
                panel("Lower panel", [0.0, 0.0, 0.0], lower_holes),
                panel("Upper panel", [0.0, 0.0, upper_z], Vec::new()),
            ]),
        )
        .unwrap();
        document.apply_batch(&batch).unwrap();
        document
    };
    let face = |occurrence_id, face_z, inward_unit_local| AssistantPinJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        }
        .into(),
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let joint = |second_normal, count, spacing_mm, pin| AssistantCadEditOperation::CreatePinJoint {
        joint_id: None,
        name: "Guarded physical row".into(),
        first: face(1, 18.0, [0.0, 0.0, -1.0]),
        second: face(2, 0.0, second_normal),
        first_center_local_mm: [20.0, 20.0, 18.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count,
        spacing_mm,
        pin,
        holes: AssistantPinHoles::Drill {
            first_insertion_mm: None,
        },
    };
    let assert_rejected_without_mutation =
        |document: &DocumentStore, operation: AssistantCadEditOperation| {
            let before = document.current();
            let undo = document.visible_undo_steps();
            assert!(
                plan(
                    document,
                    &BTreeSet::new(),
                    &ExactResultRegistry::default(),
                    &program(vec![operation]),
                )
                .is_err()
            );
            assert_eq!(
                document.current().canonical_digest(),
                before.canonical_digest()
            );
            assert_eq!(document.visible_undo_steps(), undo);
        };

    let clean = seed(Vec::new(), 18.0);
    assert_rejected_without_mutation(
        &clean,
        joint(
            [0.0, 0.0, -1.0],
            2,
            25.0,
            AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
        ),
    );
    assert_rejected_without_mutation(
        &clean,
        joint(
            [0.0, 0.0, 1.0],
            2,
            7.0,
            AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
        ),
    );
    assert_rejected_without_mutation(
        &clean,
        joint(
            [0.0, 0.0, 1.0],
            1,
            0.0,
            AssistantPin {
                diameter_mm: 8.0,
                length_mm: 40.0,
                hole_clearance_mm: 1.0,
            },
        ),
    );

    let separated = seed(Vec::new(), 20.0);
    assert_rejected_without_mutation(
        &separated,
        joint(
            [0.0, 0.0, 1.0],
            1,
            0.0,
            AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
        ),
    );

    let hardware = AssistantPartHole {
        id: "hinge-hardware".into(),
        entry_local_mm: [20.0, 20.0, 18.0],
        inward_unit_local: [0.0, 0.0, -1.0],
        diameter_mm: 10.0,
        depth_mm: 10.0,
        through: false,
        thread: None,
    };
    let obstructed = seed(vec![hardware], 18.0);
    assert_rejected_without_mutation(
        &obstructed,
        joint(
            [0.0, 0.0, 1.0],
            1,
            0.0,
            AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
        ),
    );

    let mut shallow = seed(Vec::new(), 18.0);
    let create = plan(
        &shallow,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![joint(
            [0.0, 0.0, 1.0],
            1,
            0.0,
            AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
        )]),
    )
    .unwrap();
    shallow.apply_batch(&create).unwrap();
    let committed = shallow.current();
    let first_pocket = committed
        .pin_joint(ketchup_model::pin_joint::PinJointId(1))
        .unwrap()
        .physical_hole_pairs
        .as_ref()
        .unwrap()[0]
        .first_pocket_feature_id;
    let revision_before_shallow = committed.revision_id();
    let undo_before_shallow = shallow.visible_undo_steps();
    let make_shallow = plan(
        &shallow,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::SetFeatureParameter {
            feature_id: first_pocket.0,
            parameter_path: "depth".into(),
            value_type: AssistantCadParameterValueType::Length,
            value: 15.0.into(),
        }]),
    )
    .unwrap();
    assert!(shallow.apply_batch(&make_shallow).is_err());
    assert_eq!(shallow.current().revision_id(), revision_before_shallow);
    assert_eq!(shallow.visible_undo_steps(), undo_before_shallow);
}

#[test]
fn physical_pin_joint_refuses_shared_root_and_nested_definitions_without_mutation() {
    for nested in [false, true] {
        for shared_id in [1, 2] {
            let mut document = DocumentStore::new();
            let panels = plan(
                &document,
                &BTreeSet::new(),
                &ExactResultRegistry::default(),
                &program(
                    vec![0.0, 18.0]
                        .into_iter()
                        .map(|z| {
                            cuboid_part(
                                format!("Panel {z}"),
                                [100.0, 50.0, 18.0],
                                vec![],
                                Vec::new(),
                                [0.0, 0.0, z],
                                None,
                            )
                        })
                        .collect(),
                ),
            )
            .unwrap();
            document.apply_batch(&panels).unwrap();
            let mut commands = Vec::new();
            if nested {
                commands.push(CanonicalCommand::CreateGroup {
                    id: GroupId(1),
                    name: "Nested assembly".into(),
                    transform: Transform::identity(),
                    parent: None,
                });
            }
            commands.push(CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(3),
                definition_id: DefinitionId(shared_id),
                name: "Untouched hidden sibling".into(),
                transform: Transform::from_translation(200.0, 0.0, 0.0).unwrap(),
                parent: nested.then_some(GroupId(1)),
                tags: Default::default(),
                visible: false,
            });
            document.apply_batch(&CommandBatch::new(commands)).unwrap();
            if nested {
                document
                    .convert_group_to_component(GroupId(1), "Nested component")
                    .unwrap();
            }
            let before = document.current();
            let undo = document.visible_undo_steps();
            let redo = document.visible_redo_steps();
            let epoch = document.mutation_epoch();
            let sibling = before
                .scene_query()
                .into_iter()
                .find(|instance| {
                    instance.definition_id == DefinitionId(shared_id)
                        && (instance.instance_path.is_root() == !nested)
                        && instance.occurrence_name == "Untouched hidden sibling"
                })
                .expect("the shared sibling must actually exist");
            assert!(!sibling.visible);
            let face = |id, z, inward_unit_local| AssistantPinJointFace {
                instance_path: AssistantInstancePath {
                    root_occurrence_id: id,
                    steps: vec![],
                }
                .into(),
                face_origin_local_mm: [0.0, 0.0, z],
                inward_unit_local,
                bounds_min_local_mm: [0.0; 3],
                bounds_max_local_mm: [100.0, 50.0, 18.0],
            };
            let rejection = plan(
                &document,
                &BTreeSet::new(),
                &ExactResultRegistry::default(),
                &program(vec![AssistantCadEditOperation::CreatePinJoint {
                    joint_id: None,
                    name: "Must not drill sibling".into(),
                    first: face(1, 18.0, [0.0, 0.0, -1.0]),
                    second: face(2, 0.0, [0.0, 0.0, 1.0]),
                    first_center_local_mm: [20.0, 20.0, 18.0],
                    row_unit_first_local: [1.0, 0.0, 0.0],
                    count: 3,
                    spacing_mm: 25.0,
                    pin: AssistantPin {
                        diameter_mm: 8.0,
                        length_mm: 30.0,
                        hole_clearance_mm: 1.0,
                    },
                    holes: AssistantPinHoles::Drill {
                        first_insertion_mm: None,
                    },
                }]),
            )
            .unwrap_err();
            assert_eq!(rejection.code, "planning.physical_pin_shared_definition");
            assert_eq!(
                document.current().canonical_digest(),
                before.canonical_digest()
            );
            assert_eq!(document.current().revision_id(), before.revision_id());
            assert_eq!(document.mutation_epoch(), epoch);
            assert_eq!(document.visible_undo_steps(), undo);
            assert_eq!(document.visible_redo_steps(), redo);
            assert_eq!(document.current().pin_joints().count(), 0);
        }
    }
}

#[test]
fn physical_pin_joint_supports_both_rotated_sides_and_preserves_existing_work() {
    let mut document = DocumentStore::new();
    let panel = |name: &str, translation_mm, rotation: Option<AssistantCadRotation>, holes| {
        cuboid_part(
            name.into(),
            [100.0, 50.0, 18.0],
            holes,
            Vec::new(),
            translation_mm,
            rotation,
        )
    };
    let rotation = |pivot_mm, angle_degrees| AssistantCadRotation {
        pivot_mm,
        axis: [0.0, 1.0, 0.0],
        angle_degrees,
    };
    let hardware = AssistantPartHole {
        id: "hinge-hardware".into(),
        entry_local_mm: [50.0, 25.0, 18.0],
        inward_unit_local: [0.0, 0.0, -1.0],
        diameter_mm: 6.0,
        depth_mm: 10.0,
        through: false,
        thread: None,
    };
    let panels = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            panel("Back panel", [0.0, 0.0, 0.0], None, vec![hardware]),
            panel("Top panel", [0.0, 0.0, 18.0], None, Vec::new()),
            panel(
                "Left side",
                [0.0, 0.0, 0.0],
                Some(rotation([0.0, 0.0, 0.0], -90.0)),
                Vec::new(),
            ),
            panel(
                "Right side",
                [100.0, 0.0, 100.0],
                Some(rotation([100.0, 0.0, 100.0], 90.0)),
                Vec::new(),
            ),
        ]),
    )
    .unwrap();
    document.apply_batch(&panels).unwrap();
    let face = |occurrence_id, face_origin_local_mm, inward_unit_local| AssistantPinJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        }
        .into(),
        face_origin_local_mm,
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let top_joint = AssistantCadEditOperation::CreatePinJoint {
        joint_id: None,
        name: "Existing top row".into(),
        first: face(1, [0.0, 0.0, 18.0], [0.0, 0.0, -1.0]),
        second: face(2, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        first_center_local_mm: [30.0, 45.0, 18.0],
        row_unit_first_local: [1.0, 0.0, 0.0],
        count: 2,
        spacing_mm: 40.0,
        pin: AssistantPin {
            diameter_mm: 8.0,
            length_mm: 30.0,
            hole_clearance_mm: 1.0,
        },
        holes: AssistantPinHoles::Drill {
            first_insertion_mm: None,
        },
    };
    let top_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![top_joint]),
    )
    .unwrap();
    document.apply_batch(&top_batch).unwrap();
    let before_sides = document.current();
    let preserved_joint = before_sides
        .pin_joint(ketchup_model::pin_joint::PinJointId(1))
        .unwrap()
        .clone();
    let hardware_feature_id = before_sides
        .features()
        .find(|feature| feature.name() == "Back panel hole hinge-hardware pocket")
        .unwrap()
        .id();
    let side_joint = |name: &str, first, second, first_center_local_mm| {
        AssistantCadEditOperation::CreatePinJoint {
            joint_id: None,
            name: name.into(),
            first,
            second,
            first_center_local_mm,
            row_unit_first_local: [0.0, 1.0, 0.0],
            count: 2,
            spacing_mm: 20.0,
            pin: AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
            holes: AssistantPinHoles::Drill {
                first_insertion_mm: None,
            },
        }
    };
    let sides = program(vec![
        side_joint(
            "Left physical row",
            face(1, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            face(3, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            [0.0, 10.0, 9.0],
        ),
        side_joint(
            "Right physical row",
            face(1, [100.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
            face(4, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            [100.0, 10.0, 9.0],
        ),
    ]);
    let side_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &sides,
    )
    .unwrap();
    assert_eq!(
        side_batch
            .commands()
            .iter()
            .filter(|command| matches!(
                command,
                CanonicalCommand::CreateFeature {
                    kind: FeatureKind::Pad(PadSpec {
                        profile: PadProfile::Feature(_),
                        extent: FeatureExtent::Blind(_),
                        operation: PadOperation::Cut { .. },
                        ..
                    }),
                    ..
                }
            ))
            .count(),
        8
    );
    let undo_before_sides = document.visible_undo_steps();
    document.apply_batch(&side_batch).unwrap();
    assert_eq!(document.visible_undo_steps(), undo_before_sides + 1);
    let committed = document.current();
    assert_eq!(committed.pin_joints().count(), 3);
    assert_eq!(
        committed
            .pin_joint(ketchup_model::pin_joint::PinJointId(1))
            .unwrap(),
        &preserved_joint
    );
    assert!(committed.feature(hardware_feature_id).is_some());
    assert_eq!(
        committed
            .features()
            .filter(|feature| matches!(
                feature.kind(),
                FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(_),
                    extent: FeatureExtent::Blind(_),
                    operation: PadOperation::Cut { .. },
                    ..
                })
            ))
            .count(),
        13
    );
    for (joint_id, expected_centers, first_inward) in [
        (
            1,
            [[30.0, 45.0, 18.0], [70.0, 45.0, 18.0]],
            [0.0, 0.0, -1.0],
        ),
        (2, [[0.0, 10.0, 9.0], [0.0, 30.0, 9.0]], [1.0, 0.0, 0.0]),
        (
            3,
            [[100.0, 10.0, 9.0], [100.0, 30.0, 9.0]],
            [-1.0, 0.0, 0.0],
        ),
    ] {
        let joint = committed
            .pin_joint(ketchup_model::pin_joint::PinJointId(joint_id))
            .unwrap();
        let projection =
            ketchup_model::pin_joint::project_pin_joint_contract(&committed, joint).unwrap();
        assert_eq!(projection.pairs.len(), 2);
        assert_eq!(
            projection
                .pairs
                .iter()
                .map(|pair| pair.first.shared_center_world_mm)
                .collect::<Vec<_>>(),
            expected_centers
        );
        for (pair, center) in projection.pairs.iter().zip(expected_centers) {
            assert_eq!(pair.first.diameter_mm, 8.0);
            assert_eq!(pair.second.diameter_mm, 8.0);
            assert_eq!(pair.first.depth_mm, 16.0);
            assert_eq!(pair.second.depth_mm, 16.0);
            let probe = pair.physical_probe_coincidence.unwrap();
            assert!(probe.maximum_endpoint_error_mm <= 1.0e-8);
            // D8x30 manufacturing oracle: 15 mm insertion on each side, not the 16 mm hole depth.
            for (endpoint, insertion) in [(0, 15.0), (1, -15.0)] {
                for axis in 0..3 {
                    let expected = center[axis] + first_inward[axis] * insertion;
                    assert!(
                        (probe.first_probe_endpoints_world_mm[endpoint][axis] - expected).abs()
                            <= 1.0e-8
                    );
                    assert!(
                        (probe.second_probe_endpoints_world_mm[endpoint][axis] - expected).abs()
                            <= 1.0e-8
                    );
                }
            }
        }
    }
}

#[test]
fn named_program_outputs_create_parts_physical_holes_and_joint_in_one_atomic_batch() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let panel = |name: &str, translation_mm, entry_z, inward_unit_local| {
        cuboid_part(
            name.into(),
            [100.0, 50.0, 18.0],
            vec![AssistantPartHole {
                id: "pin-1".into(),
                entry_local_mm: [20.0, 20.0, entry_z],
                inward_unit_local,
                diameter_mm: 8.0,
                depth_mm: 16.0,
                through: false,
                thread: None,
            }],
            Vec::new(),
            translation_mm,
            None,
        )
    };
    let bind = |name: &str, operation_index, output| AssistantCadEditOperation::BindProgramOutput {
        name: name.into(),
        source: AssistantCadProgramFeatureReference {
            operation_index,
            output,
        },
    };
    let output = |operation_index, output| AssistantCadProgramFeatureReference {
        operation_index,
        output,
    };
    let face = |operation_index, face_z, inward_unit_local| AssistantPinJointFace {
        instance_path: AssistantInstanceReference::ProgramOutput(output(
            operation_index,
            AssistantCadProgramFeatureOutput::Occurrence,
        )),
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let hole = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(output(
            operation_index,
            AssistantCadProgramFeatureOutput::BodyFeature,
        ))
    };
    let input = program(vec![
        panel("Lower panel", [0.0, 0.0, 0.0], 18.0, [0.0, 0.0, -1.0]),
        bind("lower", 0, AssistantCadProgramFeatureOutput::Occurrence),
        bind(
            "lower-hole",
            0,
            AssistantCadProgramFeatureOutput::BodyFeature,
        ),
        panel("Upper panel", [0.0, 0.0, 18.0], 0.0, [0.0, 0.0, 1.0]),
        bind("upper", 3, AssistantCadProgramFeatureOutput::Occurrence),
        bind(
            "upper-hole",
            3,
            AssistantCadProgramFeatureOutput::BodyFeature,
        ),
        AssistantCadEditOperation::CreatePinJoint {
            joint_id: None,
            name: "Bound row".into(),
            first: face(0, 18.0, [0.0, 0.0, -1.0]),
            second: face(3, 0.0, [0.0, 0.0, 1.0]),
            first_center_local_mm: [20.0, 20.0, 18.0],
            row_unit_first_local: [1.0, 0.0, 0.0],
            count: 1,
            spacing_mm: 0.0,
            pin: AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
            holes: AssistantPinHoles::Existing {
                pairs: vec![AssistantPinHolePair {
                    first_pocket_feature_id: hole(0),
                    second_pocket_feature_id: hole(3),
                }],
            },
        },
        bind("joint", 6, AssistantCadProgramFeatureOutput::PinJoint),
    ]);
    let input: AssistantCadEditProgram =
        serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();

    let planned = plan_with_outputs(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);
    let repeated = plan_with_outputs(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(repeated.outputs, planned.outputs);
    assert_eq!(repeated.batch.commands(), planned.batch.commands());
    assert!(matches!(
        planned.outputs.get("lower"),
        Some(AssistantCadResolvedProgramOutput::Occurrence(1))
    ));
    assert!(matches!(
        planned.outputs.get("upper"),
        Some(AssistantCadResolvedProgramOutput::Occurrence(2))
    ));
    assert!(matches!(
        planned.outputs.get("lower-hole"),
        Some(AssistantCadResolvedProgramOutput::BodyFeature(_))
    ));
    assert!(matches!(
        planned.outputs.get("upper-hole"),
        Some(AssistantCadResolvedProgramOutput::BodyFeature(_))
    ));
    assert_eq!(
        planned.outputs.get("joint"),
        Some(&AssistantCadResolvedProgramOutput::PinJoint(1))
    );

    document.apply_batch(&planned.batch).unwrap();
    let committed = document.current();
    let joint = committed
        .pin_joint(ketchup_model::pin_joint::PinJointId(1))
        .unwrap();
    assert_eq!(joint.physical_hole_pairs.as_ref().unwrap().len(), 1);
    let projection =
        ketchup_model::pin_joint::project_pin_joint_contract(&committed, joint).unwrap();
    assert_eq!(
        projection.pairs[0]
            .physical_probe_coincidence
            .as_ref()
            .unwrap()
            .maximum_endpoint_error_mm,
        0.0
    );
    assert_eq!(document.visible_undo_steps(), 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn named_program_outputs_reject_duplicates_forward_wrong_types_and_late_failure_without_mutation() {
    let mut document = DocumentStore::new();
    let baseline = document.current();
    let bind = |name: &str, operation_index, output| AssistantCadEditOperation::BindProgramOutput {
        name: name.into(),
        source: AssistantCadProgramFeatureReference {
            operation_index,
            output,
        },
    };
    let duplicate = program(vec![
        part(),
        bind("part", 0, AssistantCadProgramFeatureOutput::Definition),
        bind("part", 0, AssistantCadProgramFeatureOutput::Occurrence),
    ]);
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &duplicate
        )
        .is_err()
    );

    let wrong_type = program(vec![
        part(),
        bind("not-a-joint", 0, AssistantCadProgramFeatureOutput::PinJoint),
    ]);
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &wrong_type,
        )
        .is_err()
    );

    let late_failure = program(vec![
        part(),
        bind("part", 0, AssistantCadProgramFeatureOutput::Occurrence),
        AssistantCadEditOperation::SetDimension {
            feature_id: u64::MAX,
            constraint_id: None,
            value_mm: 20.0,
        },
    ]);
    let late_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &late_failure,
    )
    .unwrap();
    assert!(document.apply_batch(&late_batch).is_err());
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), 0);

    let missing_or_forward: AssistantCadEditProgram = serde_json::from_value(serde_json::json!({
        "operations": [
            {
                "operation": "create_pin_joint",
                "name": "Invalid",
                "first": {
                    "instance_path": {"operation_index": 2, "output": "occurrence"},
                    "face_origin_local_mm": [0.0, 0.0, 18.0],
                    "inward_unit_local": [0.0, 0.0, -1.0],
                    "bounds_min_local_mm": [0.0, 0.0, 0.0],
                    "bounds_max_local_mm": [100.0, 50.0, 18.0]
                },
                "second": {
                    "instance_path": {"operation_index": 1, "output": "occurrence"},
                    "face_origin_local_mm": [0.0, 0.0, 0.0],
                    "inward_unit_local": [0.0, 0.0, 1.0],
                    "bounds_min_local_mm": [0.0, 0.0, 0.0],
                    "bounds_max_local_mm": [100.0, 50.0, 18.0]
                },
                "first_center_local_mm": [20.0, 20.0, 18.0],
                "row_unit_first_local": [1.0, 0.0, 0.0],
                "count": 1,
                "spacing_mm": 0.0,
                "pin": {"diameter_mm": 8.0, "length_mm": 30.0, "hole_clearance_mm": 1.0},
                "holes": {"type": "existing", "pairs": [{
                    "first_pocket_feature_id": {"operation_index": 2, "output": "body_feature"},
                    "second_pocket_feature_id": {"operation_index": 2, "output": "body_feature"}
                }]}
            },
            {
                "operation": "bind_program_output",
                "name": "later",
                "source": {"operation_index": 2, "output": "occurrence"}
            },
            (serde_json::to_value(cuboid_part(
                "Later part".into(),
                [100.0, 50.0, 18.0],
                Vec::new(),
                Vec::new(),
                [0.0, 0.0, 0.0],
                None,
            ))
            .unwrap())
        ]
    }))
    .unwrap();
    assert!(missing_or_forward.validate().is_err());
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn bound_pin_joint_moves_one_paired_hole_and_rejects_invalid_shifts() {
    let mut document = DocumentStore::new();
    let holes = |entry_z, inward_unit_local| {
        [20.0, 45.0, 70.0]
            .into_iter()
            .enumerate()
            .map(|(index, x)| AssistantPartHole {
                id: format!("pin-{}", index + 1),
                entry_local_mm: [x, 20.0, entry_z],
                inward_unit_local,
                diameter_mm: 8.0,
                depth_mm: 16.0,
                through: false,
                thread: None,
            })
            .collect()
    };
    let panel = |name: &str, translation_mm, entry_z, inward_unit_local| {
        cuboid_part(
            name.into(),
            [100.0, 50.0, 18.0],
            holes(entry_z, inward_unit_local),
            Vec::new(),
            translation_mm,
            None,
        )
    };
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            panel("Lower bound panel", [0.0, 0.0, 0.0], 18.0, [0.0, 0.0, -1.0]),
            panel("Upper bound panel", [0.0, 0.0, 18.0], 0.0, [0.0, 0.0, 1.0]),
        ]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let pocket_ids = |snapshot: &Snapshot, definition_id| {
        snapshot
            .definition(definition_id)
            .unwrap()
            .feature_ids()
            .iter()
            .copied()
            .filter(|id| {
                matches!(
                    snapshot.feature(*id).unwrap().kind(),
                    FeatureKind::Pad(PadSpec {
                        profile: PadProfile::Feature(_),
                        extent: FeatureExtent::Blind(_),
                        operation: PadOperation::Cut { .. },
                        ..
                    })
                )
            })
            .collect::<Vec<_>>()
    };
    let before_joint = document.current();
    let first_pockets = pocket_ids(&before_joint, DefinitionId(1));
    let second_pockets = pocket_ids(&before_joint, DefinitionId(2));
    assert_eq!(first_pockets.len(), 3);
    assert_eq!(second_pockets.len(), 3);
    let face = |occurrence_id, face_z, inward_unit_local| AssistantPinJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        }
        .into(),
        face_origin_local_mm: [0.0, 0.0, face_z],
        inward_unit_local,
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let physical_hole_pairs = first_pockets
        .iter()
        .zip(&second_pockets)
        .map(|(first, second)| AssistantPinHolePair {
            first_pocket_feature_id: first.0.into(),
            second_pocket_feature_id: second.0.into(),
        })
        .collect();
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::CreatePinJoint {
            joint_id: None,
            name: "Bound shelf row".into(),
            first: face(1, 18.0, [0.0, 0.0, -1.0]),
            second: face(2, 0.0, [0.0, 0.0, 1.0]),
            first_center_local_mm: [20.0, 20.0, 18.0],
            row_unit_first_local: [1.0, 0.0, 0.0],
            count: 3,
            spacing_mm: 25.0,
            pin: AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
            holes: AssistantPinHoles::Existing {
                pairs: physical_hole_pairs,
            },
        }]),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let bound = document.current();
    let reopened = load(&save(&bound)).unwrap();
    assert_eq!(
        reopened
            .snapshot()
            .pin_joint(ketchup_model::pin_joint::PinJointId(1))
            .unwrap()
            .physical_hole_pairs
            .as_ref()
            .unwrap()
            .len(),
        3
    );
    let relation_page = ModelQuery::default()
        .page(
            &bound,
            &PageRequest {
                kind: EntityKind::Relations,
                limit: 10,
                search: "pin_joint".into(),
                definition_id: None,
                tag_id: None,
                classification_dimension_id: None,
                classification_category_id: None,
                world_bounds_mm: None,
                cursor: None,
                compact: false,
                direction: None,
                min_draft_degrees: None,
            },
        )
        .unwrap();
    let pairs = relation_page["items"][0]["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 3);
    assert!(pairs.iter().all(|pair| {
        pair["physical_probe_coincidence"]["full_length_coincident"] == true
            && pair["physical_probe_coincidence"]["maximum_endpoint_error_mm"] == 0.0
    }));
    let profile_id = match bound.feature(first_pockets[0]).unwrap().kind() {
        FeatureKind::Pad(PadSpec {
            profile: PadProfile::Feature(profile),
            extent: FeatureExtent::Blind(_),
            operation: PadOperation::Cut { .. },
            ..
        }) => *profile,
        _ => unreachable!(),
    };
    let revision_before_invalid_move = bound.revision_id();
    let invalid_move = program(vec![AssistantCadEditOperation::SetFeatureParameter {
        feature_id: profile_id.0,
        parameter_path: "entities.1.center.x".into(),
        value_type: AssistantCadParameterValueType::Length,
        value: 1.0.into(),
    }]);
    let invalid_batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &invalid_move,
    )
    .unwrap();
    assert!(document.apply_batch(&invalid_batch).is_err());
    assert_eq!(
        document.current().revision_id(),
        revision_before_invalid_move
    );
    for offset in [[25.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 0.0, 1.0]] {
        let invalid = program(vec![AssistantCadEditOperation::MovePhysicalPinPair {
            joint_id: 1,
            pair_index: 1,
            offset_first_local_mm: offset,
        }]);
        assert!(
            plan(
                &document,
                &BTreeSet::new(),
                &ExactResultRegistry::default(),
                &invalid
            )
            .is_err()
        );
        assert_eq!(
            document.current().canonical_digest(),
            bound.canonical_digest()
        );
    }
    let move_pair = program(vec![AssistantCadEditOperation::MovePhysicalPinPair {
        joint_id: 1,
        pair_index: 1,
        offset_first_local_mm: [50.0, 0.0, 0.0],
    }]);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &move_pair,
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let moved = document.current();
    let joint = moved
        .pin_joint(ketchup_model::pin_joint::PinJointId(1))
        .unwrap();
    assert_eq!(
        joint.physical_hole_pairs.as_ref().unwrap()[1].first_pocket_feature_id,
        first_pockets[1]
    );
    assert_eq!(
        joint.physical_hole_pairs.as_ref().unwrap()[1].second_pocket_feature_id,
        second_pockets[1]
    );
    let projected = ketchup_model::pin_joint::project_pin_joint_contract(&moved, joint).unwrap();
    assert_eq!(
        projected
            .pairs
            .iter()
            .map(|pair| pair.first.shared_center_world_mm[0])
            .collect::<Vec<_>>(),
        vec![20.0, 95.0, 70.0]
    );
    assert!(
        projected
            .pairs
            .iter()
            .all(|pair| pair.physical_probe_coincidence.is_some())
    );
    let reopened = load(&save(&moved)).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        moved.canonical_digest()
    );
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        bound.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        moved.canonical_digest()
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
    let sketch = |definition| AssistantCadEditOperation::CreateSketch {
        definition_id: AssistantCadFeatureReference::ProgramOutput(definition),
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
    let pocket = |target_feature, profile_feature| AssistantCadEditOperation::AppendFeature {
        definition_id: AssistantCadFeatureReference::ProgramOutput(output(
            0,
            AssistantCadProgramFeatureOutput::Definition,
        )),
        name: "Opening".into(),
        feature: AssistantCadBodyFeature::Pocket {
            target_feature_id: AssistantCadFeatureReference::ProgramOutput(target_feature),
            profile_feature_id: AssistantCadFeatureReference::ProgramOutput(profile_feature),
            depth_mm: 10.0,
        },
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
            AssistantCadEditOperation::CreateSketch {
                definition_id: AssistantCadFeatureReference::ProgramOutput(output(
                    0,
                    AssistantCadProgramFeatureOutput::Definition,
                )),
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
            secondary: None,
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
fn rectangular_pattern_places_numbered_copies_row_by_row_in_one_step() {
    let mut document = seeded();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let registry = ExactResultRegistry::default();
    let rows = |secondary| {
        program(vec![AssistantCadEditOperation::LinearPattern {
            selector: explicit(1),
            instances: 3,
            step_mm: [25.0, 0.0, 0.0],
            secondary,
        }])
    };

    // The old request shape plans exactly the first row of the rectangular one.
    let single = plan(&document, &BTreeSet::new(), &registry, &rows(None)).unwrap();
    let rectangular = rows(Some(AssistantPatternDirection {
        instances: 2,
        step_mm: [0.0, 40.0, 0.0],
    }));
    let batch = plan(&document, &BTreeSet::new(), &registry, &rectangular).unwrap();
    assert_eq!(single.commands().len(), 2);
    assert_eq!(batch.commands().len(), 5);
    assert_eq!(single.commands(), &batch.commands()[..2]);

    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    let source = committed.occurrence(OccurrenceId(1)).unwrap();
    assert_eq!(source.name(), "Editable part");
    for (id, name, expected) in [
        (2, "Editable part 2", [30.0, 6.0, 7.0]),
        (3, "Editable part 3", [55.0, 6.0, 7.0]),
        (4, "Editable part 4", [5.0, 46.0, 7.0]),
        (5, "Editable part 5", [30.0, 46.0, 7.0]),
        (6, "Editable part 6", [55.0, 46.0, 7.0]),
    ] {
        let copy = committed.occurrence(OccurrenceId(id)).unwrap();
        assert_eq!(copy.definition_id(), source.definition_id());
        assert_eq!(copy.name(), name);
        let transform = copy.transform();
        let matrix = transform.matrix();
        for (index, value) in [3, 7, 11].into_iter().zip(expected) {
            assert!(
                (matrix[index] - value).abs() < 1e-9,
                "copy {id}: {matrix:?}"
            );
        }
    }
    assert_eq!(committed.occurrences().count(), 6);
    assert_eq!(document.visible_undo_steps(), undo + 1);
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn a_pattern_makes_at_most_512_new_parts() {
    let mut document = seeded();
    let baseline = document.current();
    let undo = document.visible_undo_steps();
    let registry = ExactResultRegistry::default();
    let row = |instances| {
        program(vec![AssistantCadEditOperation::LinearPattern {
            selector: explicit(1),
            instances,
            step_mm: [30.0, 0.0, 0.0],
            secondary: None,
        }])
    };

    let rejection = plan(&document, &BTreeSet::new(), &registry, &row(514)).unwrap_err();
    assert_eq!(rejection.code, "intent.cad_edit_program_invalid");
    assert!(
        rejection.failed_invariant.contains("limit of 512"),
        "{rejection:?}"
    );
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    assert_eq!(document.visible_undo_steps(), undo);

    let batch = plan(&document, &BTreeSet::new(), &registry, &row(513)).unwrap();
    document.apply_batch(&batch).unwrap();
    let committed = document.current();
    assert_eq!(
        committed.occurrences().count(),
        baseline.occurrences().count() + 512
    );
    let last = committed.occurrence(OccurrenceId(513)).unwrap();
    assert_eq!(last.name(), "Editable part 513");
    assert!((last.transform().matrix()[3] - (5.0 + 512.0 * 30.0)).abs() < 1e-9);
    assert_eq!(document.visible_undo_steps(), undo + 1);
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
        holes: Vec::new(),
        pockets: Vec::new(),
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
            definition_id: 1.into(),
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
    assert_eq!(
        graph.profiles[0].frame().origin.to_array(),
        [30.0, -20.0, 15.0]
    );
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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Extrusion".into(),
                kind: FeatureKind::extrusion(FeatureId(1), Dimension::new("30", 30.0).unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Instance".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let input = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: 1.into(),
        name: "Finish".into(),
        feature: AssistantCadBodyFeature::TopologyFillet {
            target_feature_id: 2,
            edge_reference_ids: vec!["a".repeat(64)],
            radius_mm: 1.0,
            radius_stations: Vec::new(),
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
fn set_grounded_program_grounds_and_ungrounds_selected_occurrences_in_one_undo_step() {
    let mut document = seeded();
    let registry = ExactResultRegistry::default();
    let before = document.current();
    let ground = program(vec![AssistantCadEditOperation::SetGrounded {
        selector: explicit(1),
        grounded: true,
    }]);
    document
        .apply_batch(&plan(&document, &BTreeSet::new(), &registry, &ground).unwrap())
        .unwrap();
    assert!(document.current().occurrence_is_grounded(OccurrenceId(1)));
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    let unground = program(vec![AssistantCadEditOperation::SetGrounded {
        selector: explicit(1),
        grounded: false,
    }]);
    document
        .apply_batch(&plan(&document, &BTreeSet::new(), &registry, &unground).unwrap())
        .unwrap();
    assert!(!document.current().occurrence_is_grounded(OccurrenceId(1)));
    let missing = program(vec![AssistantCadEditOperation::SetGrounded {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![999],
        },
        grounded: true,
    }]);
    assert!(plan(&document, &BTreeSet::new(), &registry, &missing).is_err());
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
            secondary: None,
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
            SketchEntity::Circle {
                id: SketchEntityId(5),
                center_mm: [0.0, 0.0],
                radius_mm: 0.5,
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
        definition_id: definition.0.into(),
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
        ExactBRepPlanarGeometry::Region { outer, holes }
            if matches!(outer, ketchup_model::exact_brep_graph::ExactBRepPlanarLoop::Boundary { segments } if segments.len() == 4)
                && holes.len() == 1
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
        definition_id: definition.0.into(),
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

#[test]
fn public_surface_program_is_typed_atomic_and_refuses_solid_surface_targets() {
    let definition = DefinitionId(1);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Public surfaces".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: definition,
                name: "Outer profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: definition,
                name: "Cutter profile".into(),
                kind: FeatureKind::polygon(&[[5.0, -5.0], [15.0, -5.0], [15.0, 15.0], [5.0, 15.0]]),
            },
        ]))
        .unwrap();
    let baseline = document.current();
    let earlier_body = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(AssistantCadProgramFeatureReference {
            operation_index,
            output: AssistantCadProgramFeatureOutput::BodyFeature,
        })
    };
    let input = program(vec![
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0.into(),
            name: "Planar surface A".into(),
            feature: AssistantCadBodyFeature::SurfaceBody {
                source: AssistantCadSurfaceBodySource::Planar {
                    profile_feature_id: FeatureId(1).0.into(),
                },
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0.into(),
            name: "Planar surface B".into(),
            feature: AssistantCadBodyFeature::SurfaceBody {
                source: AssistantCadSurfaceBodySource::Planar {
                    profile_feature_id: FeatureId(2).0.into(),
                },
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0.into(),
            name: "Trimmed surface".into(),
            feature: AssistantCadBodyFeature::SurfaceTrim {
                target_feature_id: earlier_body(0),
                cutter_feature_id: earlier_body(1),
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0.into(),
            name: "Extended surface".into(),
            feature: AssistantCadBodyFeature::SurfaceExtend {
                target_feature_id: earlier_body(2),
                distance_mm: 2.0,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0.into(),
            name: "Knitted surface".into(),
            feature: AssistantCadBodyFeature::SurfaceKnit {
                surface_feature_ids: vec![earlier_body(3), earlier_body(1)],
                tolerance_mm: 0.001,
                make_solid: false,
            },
        },
        AssistantCadEditOperation::AppendFeature {
            definition_id: definition.0.into(),
            name: "Thickened solid".into(),
            feature: AssistantCadBodyFeature::SurfaceThicken {
                target_feature_id: earlier_body(4),
                thickness_mm: 1.5,
                direction: AssistantCadShellDirection::Symmetric,
            },
        },
    ]);

    assert_eq!(input.validate(), Ok(()));
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    let preview = document.preview_batch(&batch).unwrap();
    assert!(matches!(
        preview.feature(FeatureId(3)).unwrap().kind(),
        FeatureKind::SurfaceBody(SurfaceBodySpec::Planar {
            profile: FeatureId(1)
        })
    ));
    assert!(matches!(
        preview.feature(FeatureId(5)).unwrap().kind(),
        FeatureKind::SurfaceTrim {
            target: FeatureId(3),
            cutter: FeatureId(4)
        }
    ));
    assert!(matches!(
        preview.feature(FeatureId(7)).unwrap().kind(),
        FeatureKind::SurfaceKnit { surfaces, make_solid: false, .. }
            if surfaces == &[FeatureId(4), FeatureId(6)]
    ));
    assert_eq!(
        preview.feature(FeatureId(8)).unwrap().kind().body_kind(),
        Some(BodyKind::Solid)
    );
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), 2);

    let committed = document.current();
    let invalid = program(vec![AssistantCadEditOperation::AppendFeature {
        definition_id: definition.0.into(),
        name: "Invalid solid extension".into(),
        feature: AssistantCadBodyFeature::SurfaceExtend {
            target_feature_id: FeatureId(8).0.into(),
            distance_mm: 1.0,
        },
    }]);
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &invalid,
        )
        .is_err()
    );
    assert_eq!(
        document.current().canonical_digest(),
        committed.canonical_digest()
    );
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        baseline.canonical_digest()
    );
}

#[test]
fn public_assistant_cam_setup_is_exact_bound_atomic_undoable_and_fail_closed() {
    let mut document = seeded();
    let snapshot = document.current();
    let target = snapshot
        .features()
        .find(|feature| feature.kind().body_kind() == Some(BodyKind::Solid))
        .unwrap();
    let definition_id = target.definition_id().0;
    let feature_id = target.id().0;
    let before_digest = snapshot.canonical_digest();
    let before_undo = document.visible_undo_steps();
    let input = program(vec![cam_setup_operation(definition_id, feature_id)]);

    assert_eq!(input.validate(), Ok(()));
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &input,
    )
    .unwrap();
    assert_eq!(document.current().canonical_digest(), before_digest);
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.visible_undo_steps(), before_undo + 1);
    let committed = document.current();
    let cam = committed
        .cam_plan(ketchup_model::cam::CamPlanId(1))
        .expect("Assistant setup must publish one canonical CAM plan")
        .clone();
    assert_eq!(cam.name(), "Reviewed top setup");
    assert_eq!(cam.target().definition_id.0, definition_id);
    assert_eq!(cam.target().feature_id.0, feature_id);
    assert_eq!(cam.tool().spindle_rpm, 12_000);
    assert_eq!(
        cam.setup().work_offset,
        ketchup_model::cam::CamWorkOffset::G54
    );

    assert_eq!(document.undo().unwrap().canonical_digest(), before_digest);
    assert!(
        document
            .current()
            .cam_plan(ketchup_model::cam::CamPlanId(1))
            .is_none()
    );
    document.redo().unwrap();

    let mut invalid = cam_setup_operation(definition_id, feature_id);
    if let AssistantCadEditOperation::UpsertCamPlan { stepover_ratio, .. } = &mut invalid {
        *stepover_ratio = 1.2;
    }
    assert!(
        plan(
            &document,
            &BTreeSet::new(),
            &ExactResultRegistry::default(),
            &program(vec![invalid]),
        )
        .is_err()
    );
    assert_eq!(
        document
            .current()
            .cam_plan(ketchup_model::cam::CamPlanId(1)),
        Some(&cam)
    );
}
