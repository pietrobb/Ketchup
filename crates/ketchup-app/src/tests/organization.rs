//! Groups, components, tags, clipboard and selection commands.

use super::*;

#[test]
fn removing_the_last_topological_selection_clears_the_occurrence_selection() {
    let mut app = KetchupApp::new();
    install_initial_graph_result(&mut app);
    let locator = TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: FeatureId(2),
        kind: TopologicalElementKind::Edge,
        ordinal: 2,
    };
    assert!(app.select_topological_locator(locator.clone()));
    assert_eq!(app.selected_occurrence_count(), 1);
    assert!(app.select_topological_locator_additive(locator, true));
    assert_eq!(app.selected_occurrence_count(), 0);
    assert!(!app.command_is_enabled(AppCommand::Fillet));
}

#[test]
fn rename_plans_are_revision_context_command_bound_and_clipboard_preserving() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();

    let occurrence_source = app.occurrence_rename_source_plan().unwrap();
    assert_eq!(occurrence_source.source_revision, app.document_revision());
    assert_eq!(occurrence_source.occurrence_count, 1);
    app.begin_occurrence_rename();
    app.modal.get_mut::<PendingOccurrenceRename>().unwrap().name = "Exact occurrence".to_owned();
    let occurrence_pending = app.modal.get::<PendingOccurrenceRename>().cloned().unwrap();
    let occurrence_plan = app.occurrence_rename_plan(&occurrence_pending).unwrap();
    assert_eq!(
        occurrence_plan.command,
        CanonicalCommand::RenameEntity {
            id: OccurrenceId(1),
            name: "Exact occurrence".to_owned(),
        }
    );

    let mut tampered_occurrence_plan = occurrence_plan.clone();
    tampered_occurrence_plan.command = CanonicalCommand::RenameEntity {
        id: OccurrenceId(1),
        name: "Tampered occurrence".to_owned(),
    };
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    assert!(!app.apply_occurrence_rename_plan(tampered_occurrence_plan));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_occurrence_rename_plan(occurrence_plan.clone()));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);
    app.selection.edit_context.clear();

    assert!(app.apply_occurrence_rename_plan(occurrence_plan));
    assert_eq!(
        app.occurrence_name(OccurrenceId(1)),
        Some("Exact occurrence".to_owned())
    );
    assert_eq!(app.clipboard.occurrences, clipboard);
    assert!(app.undo());

    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.begin_definition_rename();
    app.modal.get_mut::<PendingDefinitionRename>().unwrap().name = "Exact definition".to_owned();
    let definition_pending = app.modal.get::<PendingDefinitionRename>().cloned().unwrap();
    let definition_plan = app.definition_rename_plan(&definition_pending).unwrap();
    assert_eq!(definition_plan.source.instance_count, 1);
    assert_eq!(
        definition_plan.command,
        CanonicalCommand::RenameDefinition {
            id: INITIAL_BOX_DEFINITION,
            name: "Exact definition".to_owned(),
        }
    );

    let mut tampered_definition_plan = definition_plan.clone();
    tampered_definition_plan.source.instance_count += 1;
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    assert!(!app.apply_definition_rename_plan(tampered_definition_plan));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_definition_rename_plan(definition_plan.clone()));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);
    app.selection.edit_context.clear();

    assert!(app.apply_definition_rename_plan(definition_plan));
    assert_eq!(
        app.definition_name(INITIAL_BOX_DEFINITION),
        Some("Exact definition".to_owned())
    );
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn viewport_selection_keeps_group_commands_available() {
    let mut app = KetchupApp::new();
    app.select_all();
    assert!(app.copy_selection_to_clipboard());
    assert!(app.paste_clipboard());
    app.select_all();
    assert!(app.group_selected());

    let target = SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.clear_selection();
    app.select_from_viewport(Some(target), false);

    assert_eq!(app.selection_count(), 2);
    assert!(app.selection.primary.is_none());
    assert!(app.selected_group_id().is_some());
    assert!(app.command_enabled(AppCommand::Ungroup));
    assert!(app.command_enabled(AppCommand::MakeComponent));
}

#[test]
fn group_and_ungroup_fail_closed_for_exact_incomplete_and_stale_selection() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection
        .select_path(InstancePath::root(OccurrenceId(1)), false);
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        true,
    );
    let exact_revision = app.document_revision();
    let exact_digest = app.canonical_digest();
    let exact_undo_steps = app.document.visible_undo_steps();
    assert!(app.command_enabled(AppCommand::Group));
    assert!(app.group_selection_source_plan().is_some());
    assert_eq!(app.document_revision(), exact_revision);
    assert_eq!(app.canonical_digest(), exact_digest);
    assert_eq!(app.document.visible_undo_steps(), exact_undo_steps);

    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    app.selection.primary = Some(SelectionId {
        definition_id: INITIAL_BOX_DEFINITION,
        instance_path: InstancePath::root(OccurrenceId(1)),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    });
    let face_revision = app.document_revision();
    let face_digest = app.canonical_digest();
    let face_undo_steps = app.document.visible_undo_steps();
    assert!(!app.command_enabled(AppCommand::MakeComponent));
    assert!(!app.make_component());
    assert_eq!(app.document_revision(), face_revision);
    assert_eq!(app.canonical_digest(), face_digest);
    assert_eq!(app.document.visible_undo_steps(), face_undo_steps);

    app.selection.primary = None;
    app.selection
        .occurrences
        .remove(&InstancePath::root(OccurrenceId(2)));
    let incomplete_revision = app.document_revision();
    let incomplete_digest = app.canonical_digest();
    let incomplete_undo_steps = app.document.visible_undo_steps();
    assert!(!app.command_enabled(AppCommand::Ungroup));
    assert!(!app.command_enabled(AppCommand::MakeComponent));
    assert!(!app.ungroup_selected());
    assert!(!app.make_component());
    assert_eq!(app.document_revision(), incomplete_revision);
    assert_eq!(app.canonical_digest(), incomplete_digest);
    assert_eq!(app.document.visible_undo_steps(), incomplete_undo_steps);

    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(2),
                parent: None,
            },
            CanonicalCommand::DeleteGroup { id: group_id },
        ]))
        .unwrap();
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.document.visible_undo_steps();
    assert!(!app.command_enabled(AppCommand::Ungroup));
    assert!(!app.command_enabled(AppCommand::MakeComponent));
    assert!(!app.ungroup_selected());
    assert!(!app.make_component());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.document.visible_undo_steps(), stale_undo_steps);
}

#[test]
fn group_and_ungroup_plans_are_revision_bound_exact_and_clipboard_preserving() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();

    let group_plan = app.group_selection_source_plan().unwrap();
    assert_eq!(group_plan.source_revision, app.document_revision());
    assert_eq!(group_plan.occurrence_count, 2);
    assert_eq!(group_plan.commands.len(), 3);
    let mut tampered_group_plan = group_plan.clone();
    tampered_group_plan.occurrence_count += 1;
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    assert!(!app.apply_group_selection_source_plan(tampered_group_plan));
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.create_box());
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_group_selection_source_plan(group_plan));
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    let ungroup_plan = app.ungroup_selection_source_plan().unwrap();
    assert_eq!(ungroup_plan.source_revision, app.document_revision());
    assert_eq!(ungroup_plan.group_id, group_id);
    assert_eq!(ungroup_plan.occurrence_count, 2);
    assert_eq!(ungroup_plan.item_count, 2);

    let mut tampered_ungroup_plan = ungroup_plan.clone();
    tampered_ungroup_plan.commands.pop();
    let grouped_digest = app.canonical_digest();
    let grouped_undo_steps = app.undo_step_count();
    let grouped_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_ungroup_selection_source_plan(tampered_ungroup_plan));
    assert_eq!(app.canonical_digest(), grouped_digest);
    assert_eq!(app.undo_step_count(), grouped_undo_steps);
    assert_eq!(app.action_digest(), grouped_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(group_id));
    assert!(!app.apply_ungroup_selection_source_plan(ungroup_plan));
    assert_eq!(app.canonical_digest(), grouped_digest);
    assert_eq!(app.undo_step_count(), grouped_undo_steps);
    assert_eq!(app.action_digest(), grouped_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);
    app.selection.edit_context.clear();

    assert!(app.ungroup_selected());
    assert_eq!(app.group_count(), 0);
    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn moved_group_behaves_as_one_object_and_explodes_without_geometry_shift() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_all();
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    let ids = [OccurrenceId(1), OccurrenceId(2)];
    let before = ids.map(|id| {
        app.document
            .current()
            .world_transform_for_occurrence(id)
            .unwrap()
    });
    let revision_before_move = app.document_revision();

    assert!(app.move_selected(Vec3::new(40.0, -20.0, 15.0)));
    assert_eq!(app.document_revision(), revision_before_move + 1);
    assert_eq!(app.selection.selected_group, Some(group_id));
    let moved = ids.map(|id| {
        app.document
            .current()
            .world_transform_for_occurrence(id)
            .unwrap()
    });
    for (before, moved) in before.into_iter().zip(moved) {
        assert_eq!(moved.matrix()[3], before.matrix()[3] + 40.0);
        assert_eq!(moved.matrix()[7], before.matrix()[7] - 20.0);
        assert_eq!(moved.matrix()[11], before.matrix()[11] + 15.0);
    }

    let moved = ids.map(|id| {
        app.document
            .current()
            .world_transform_for_occurrence(id)
            .unwrap()
    });
    assert!(app.ungroup_selected());
    assert_eq!(app.group_count(), 0);
    let exploded = ids.map(|id| {
        app.document
            .current()
            .world_transform_for_occurrence(id)
            .unwrap()
    });
    assert_eq!(exploded, moved);
}

#[test]
fn dimensions_panel_creates_categories_and_assigns_independent_values_in_one_undo_step() {
    let mut app = KetchupApp::new();
    app.selection
        .select_path(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.create_classification_dimension("Building side", "Exterior"));
    assert!(app.add_classification_category(ClassificationDimensionId(1), "Interior"));
    assert!(app.create_classification_dimension("Building system", "Structure"));
    assert!(app.assign_selection_to_classification(
        ClassificationDimensionId(1),
        Some(ClassificationCategoryId(2))
    ));
    let undo_before = app.document.visible_undo_steps();
    assert!(app.assign_selection_to_classification(
        ClassificationDimensionId(2),
        Some(ClassificationCategoryId(3))
    ));
    assert_eq!(app.document.visible_undo_steps(), undo_before + 1);
    assert_eq!(
        app.document
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(1)),
        Some(ClassificationCategoryId(2))
    );
    assert_eq!(
        app.document
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(2)),
        Some(ClassificationCategoryId(3))
    );
    app.document.undo().unwrap();
    assert_eq!(
        app.document
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(1)),
        Some(ClassificationCategoryId(2))
    );
    assert_eq!(
        app.document
            .current()
            .occurrence_classification(OccurrenceId(1), ClassificationDimensionId(2)),
        None
    );

    app.assistant.workspace_mode = AssistantWorkspaceMode::Tab;
    app.classification.selected_dimension = Some(ClassificationDimensionId(1));
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    for expected in ["DIMENSIONS", "Interior (1 occurrences)"] {
        assert!(
            harness
                .query_all_by(|node| {
                    !node.is_hidden()
                        && (node.label().as_deref() == Some(expected)
                            || node.value().as_deref() == Some(expected))
                })
                .next()
                .is_some(),
            "missing dimensions panel text: {expected}"
        );
    }
}

#[test]
fn outliner_and_viewport_share_multiselection_without_document_mutation() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let revision = app.document_revision();
    let outliner_ids = app
        .outliner_query()
        .into_iter()
        .flat_map(|definition| definition.occurrences)
        .map(|occurrence| occurrence.instance_path)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        outliner_ids,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );

    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.selection.contains(&InstancePath::root(OccurrenceId(1))));
    assert_eq!(app.selection_count(), 1);

    app.select_from_viewport(
        Some(SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        }),
        true,
    );
    assert_eq!(app.selection_count(), 2);
    assert!(app.selection.contains(&InstancePath::root(OccurrenceId(1))));
    assert!(app.selection.contains(&InstancePath::root(OccurrenceId(2))));

    app.select_from_viewport(
        Some(SelectionId {
            definition_id: DefinitionId(2),
            instance_path: InstancePath::root(OccurrenceId(2)),
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Maximum,
            },
        }),
        true,
    );
    assert_eq!(app.selection_count(), 1);

    app.select_from_viewport(None, false);
    assert_eq!(app.selection_count(), 0);
    app.orbit(Vec2::new(18.0, -9.0));
    assert_eq!(app.document_revision(), revision);
}

#[test]
fn group_and_ungroup_preserve_world_placement_as_atomic_batches() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    let before = app
        .document
        .current()
        .scene_query()
        .into_iter()
        .map(|item| (item.occurrence_id, item.transform))
        .collect::<BTreeMap<_, _>>();
    let undo_steps = app.document.visible_undo_steps();

    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    let grouped = app.document.current();
    assert_eq!(app.document.visible_undo_steps(), undo_steps + 1);
    assert_eq!(grouped.groups().count(), 1);
    assert_eq!(
        grouped.occurrence(OccurrenceId(1)).unwrap().parent(),
        Some(group_id)
    );
    assert_eq!(
        grouped.occurrence(OccurrenceId(2)).unwrap().parent(),
        Some(group_id)
    );
    assert_eq!(
        grouped
            .scene_query()
            .into_iter()
            .map(|item| (item.occurrence_id, item.transform))
            .collect::<BTreeMap<_, _>>(),
        before
    );

    assert!(app.undo());
    assert_eq!(app.document.current().groups().count(), 0);
    assert!(app.redo());
    assert!(app.select_group(group_id));
    assert!(app.ungroup_selected());
    assert_eq!(app.document.current().groups().count(), 0);
    assert_eq!(
        app.document
            .current()
            .scene_query()
            .into_iter()
            .map(|item| (item.occurrence_id, item.transform))
            .collect::<BTreeMap<_, _>>(),
        before
    );
    assert!(app.undo());
    assert_eq!(app.document.current().groups().count(), 1);
    assert!(app.redo());
    assert_eq!(app.document.current().groups().count(), 0);
}

#[test]
fn ungroup_composes_parent_transform_into_occurrences_and_child_groups() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let parent_group = app.selection.selected_group.unwrap();
    app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(4)), true);
    assert!(app.group_selected());
    let child_group = app.selection.selected_group.unwrap();
    let parent_transform = Transform::from_translation(40.0, -20.0, 15.0).unwrap();
    let child_transform = Transform::from_translation(-5.0, 12.0, 3.0).unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetGroupTransform {
                id: parent_group,
                transform: parent_transform,
            },
            CanonicalCommand::SetGroupTransform {
                id: child_group,
                transform: child_transform,
            },
            CanonicalCommand::SetGroupParent {
                id: child_group,
                parent: Some(parent_group),
            },
        ]))
        .unwrap();
    let before = app
        .document
        .current()
        .scene_query()
        .into_iter()
        .map(|item| (item.occurrence_id, item.transform))
        .collect::<BTreeMap<_, _>>();
    assert!(app.select_group(parent_group));
    let undo_steps = app.document.visible_undo_steps();

    assert!(app.ungroup_selected());

    let snapshot = app.document.current();
    assert!(snapshot.group(parent_group).is_none());
    let child = snapshot.group(child_group).unwrap();
    assert_eq!(child.parent(), None);
    assert_eq!(child.transform(), parent_transform.compose(child_transform));
    assert_eq!(app.document.visible_undo_steps(), undo_steps + 1);
    assert_eq!(
        snapshot
            .scene_query()
            .into_iter()
            .map(|item| (item.occurrence_id, item.transform))
            .collect::<BTreeMap<_, _>>(),
        before
    );
    assert!(app.undo());
    assert!(app.document.current().group(parent_group).is_some());
    assert!(app.redo());
    assert!(app.document.current().group(parent_group).is_none());
}

#[test]
fn make_component_converts_a_nested_group_subtree_in_one_undo_step() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let parent_group = app.selection.selected_group.unwrap();
    let child_group = GroupId(10);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: child_group,
                name: "Child".to_owned(),
                transform: Transform::identity(),
                parent: Some(parent_group),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(3),
                parent: Some(child_group),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(4),
                parent: Some(child_group),
            },
        ]))
        .unwrap();
    assert!(app.select_group(parent_group));
    let before = app.canonical_digest();
    let revision = app.document_revision();
    let undo_steps = app.document.visible_undo_steps();
    assert!(app.make_component_source_plan().is_some());

    assert!(app.make_component());

    assert_eq!(app.group_count(), 0);
    assert_eq!(app.occurrence_count(), 1);
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.document.visible_undo_steps(), undo_steps + 1);
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-made-component",
            &BTreeMap::from([
                (
                    "name",
                    app.catalog.format(
                        "model-component-name",
                        &BTreeMap::from([("number", parent_group.0.to_string())]),
                    )
                ),
                ("count", "4".to_owned()),
            ]),
        )
    );
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before);
    assert_eq!(app.group_count(), 2);
    assert_eq!(app.occurrence_count(), 4);
    assert!(app.redo());
    assert_eq!(app.group_count(), 0);
    assert_eq!(app.occurrence_count(), 1);
}

#[test]
fn make_component_plan_rejects_tampering_context_drift_and_staleness_without_side_effects() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    let plan = app.make_component_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(plan.group_id, group_id);
    assert_eq!(plan.occurrence_count, 2);
    assert_eq!(
        plan.occurrence_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    assert_eq!(plan.primary, None);
    assert_eq!(plan.selected_group, Some(group_id));
    assert!(plan.edit_context.is_empty());
    assert_eq!(plan.subtree_occurrence_count, 2);
    assert_eq!(plan.new_definition_id, DefinitionId(3));
    assert_eq!(plan.new_occurrence_id, OccurrenceId(3));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    let mut tampered = plan.clone();
    tampered.group_name.push('!');
    assert!(!app.apply_make_component_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(group_id));
    assert!(!app.apply_make_component_source_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.create_box());
    assert!(app.select_group(group_id));
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_make_component_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn make_component_rejects_a_nested_local_id_collision_without_mutation() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let parent_group = app.selection.selected_group.unwrap();
    app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(4)), true);
    assert!(app.group_selected());
    let child_group = app.selection.selected_group.unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetGroupParent {
            id: child_group,
            parent: Some(parent_group),
        }]))
        .unwrap();
    assert!(app.select_group(parent_group));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.document.visible_undo_steps();

    assert!(app.make_component_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::MakeComponent));
    assert!(!app.make_component());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.document.visible_undo_steps(), undo_steps);
}

#[test]
fn component_copies_keep_distinct_nested_paths_and_composed_world_positions() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let before = app
        .active_boxes()
        .into_iter()
        .map(|item| item.origin_mm)
        .collect::<Vec<_>>();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());

    let converted = app.active_boxes();
    assert_eq!(
        converted
            .iter()
            .map(|item| item.origin_mm)
            .collect::<Vec<_>>(),
        before
    );
    assert!(converted.iter().all(|item| !item.instance_path.is_root()));
    let component_path = app.selected_move_reference().unwrap().instance_path;
    assert!(component_path.is_root());
    assert!(app.copy_selected(Vec3::new(200.0, 0.0, 0.0)));

    let boxes = app.active_boxes();
    let paths = boxes
        .iter()
        .map(|item| item.instance_path.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(paths.len(), 4);
    assert_eq!(
        paths
            .iter()
            .map(InstancePath::root_occurrence)
            .collect::<BTreeSet<_>>()
            .len(),
        2
    );
    let mut expected = before.clone();
    expected.extend(
        before
            .iter()
            .map(|origin| *origin + Vec3::new(200.0, 0.0, 0.0)),
    );
    let actual = boxes.iter().map(|item| item.origin_mm).collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn repeated_multi_level_instance_paths_keep_world_identity_while_root_only_operations_refuse_them()
{
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());

    let first_level_root = app.document.current().occurrences().next().unwrap().id();
    assert!(app.copy_selected(Vec3::new(200.0, 0.0, 0.0)));
    let first_level_roots = app
        .document
        .current()
        .occurrences()
        .map(|occurrence| occurrence.id())
        .collect::<Vec<_>>();
    assert_eq!(first_level_roots.len(), 2);
    assert!(first_level_roots.contains(&first_level_root));

    app.clear_selection();
    app.select_from_outliner(InstancePath::root(first_level_roots[0]), false);
    app.select_from_outliner(InstancePath::root(first_level_roots[1]), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    assert!(app.copy_selected(Vec3::new(500.0, -40.0, 25.0)));

    let snapshot = app.document.current();
    let top_level_roots = snapshot
        .occurrences()
        .map(|occurrence| occurrence.id())
        .collect::<Vec<_>>();
    assert_eq!(top_level_roots.len(), 2);
    let leaves = snapshot
        .scene_query()
        .into_iter()
        .filter(|item| {
            item.instance_path
                .steps()
                .iter()
                .filter(|step| matches!(step, InstancePathStep::Occurrence(_)))
                .count()
                == 2
        })
        .collect::<Vec<_>>();
    assert_eq!(leaves.len(), 8);
    assert_eq!(
        leaves
            .iter()
            .map(|item| item.instance_path.clone())
            .collect::<BTreeSet<_>>()
            .len(),
        8
    );

    let by_root = top_level_roots
        .iter()
        .map(|root| {
            let entries = leaves
                .iter()
                .filter(|item| item.instance_path.root_occurrence() == *root)
                .map(|item| (item.instance_path.steps().to_vec(), item.transform.matrix()))
                .collect::<BTreeMap<_, _>>();
            (*root, entries)
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(by_root[&top_level_roots[0]].len(), 4);
    assert_eq!(by_root[&top_level_roots[1]].len(), 4);
    for (suffix, first) in &by_root[&top_level_roots[0]] {
        let second = by_root[&top_level_roots[1]][suffix];
        assert_eq!(second[3], first[3] + 500.0);
        assert_eq!(second[7], first[7] - 40.0);
        assert_eq!(second[11], first[11] + 25.0);
    }

    drop(snapshot);
    let nested_paths = BTreeSet::from([
        leaves[0].instance_path.clone(),
        leaves
            .iter()
            .find(|item| {
                item.instance_path.root_occurrence() != leaves[0].instance_path.root_occurrence()
                    && item.instance_path.steps() == leaves[0].instance_path.steps()
            })
            .unwrap()
            .instance_path
            .clone(),
    ]);
    app.clear_selection();
    app.selection.occurrences = nested_paths.clone();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert_eq!(
        app.selected_root_occurrence_ids(),
        Err(RootOccurrenceSelectionError::Nested {
            paths: nested_paths
        })
    );
    assert!(!app.prepare_assistant_assembly_joint_from_selection());
    assert!(!app.preview_selection_drawing());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}

#[test]
fn grounded_repeated_nested_instances_create_associative_drawing_with_roundtrip_and_undo() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    assert!(app.copy_selected(Vec3::new(40.0, 0.0, 0.0)));

    let first_level_roots = app
        .document
        .current()
        .occurrences()
        .map(|occurrence| occurrence.id())
        .collect::<Vec<_>>();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(first_level_roots[0]), false);
    app.select_from_outliner(InstancePath::root(first_level_roots[1]), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    assert!(app.copy_selected(Vec3::new(80.0, -20.0, 10.0)));

    let top_level_roots = app
        .document
        .current()
        .occurrences()
        .map(|occurrence| occurrence.id())
        .collect::<Vec<_>>();
    for root in &top_level_roots {
        app.clear_selection();
        app.select_from_outliner(InstancePath::root(*root), false);
        assert!(app.set_selected_occurrence_grounded(true));
    }

    assert!(app.headless_install_exact_package(current_box_package(&app)));
    let leaves = app
        .document
        .current()
        .scene_query()
        .into_iter()
        .filter(|item| {
            item.instance_path
                .steps()
                .iter()
                .filter(|step| matches!(step, InstancePathStep::Occurrence(_)))
                .count()
                == 2
        })
        .collect::<Vec<_>>();
    let first = leaves[0].instance_path.clone();
    let second = leaves
        .iter()
        .find(|item| {
            item.instance_path.root_occurrence() != first.root_occurrence()
                && item.instance_path.steps() == first.steps()
        })
        .unwrap()
        .instance_path
        .clone();
    let mut instance_paths = vec![first, second];
    instance_paths.sort();

    let stale_path = instance_paths[0]
        .clone()
        .with_step(InstancePathStep::Occurrence(
            ketchup_model::document::LocalOccurrenceId(u64::MAX),
        ));
    app.clear_selection();
    app.selection.occurrences.insert(stale_path);
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    assert!(!app.preview_selection_drawing());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);

    app.clear_selection();
    app.selection.occurrences = instance_paths.iter().cloned().collect();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let prepared = app.preview_selection_drawing();
    assert!(prepared, "{}", app.action_digest());
    assert!(app.assembly_preview_pending());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert!(app.confirm_assembly_preview());
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);

    let committed = app.document.current();
    let sheet = committed.drawing_sheets().next().unwrap();
    assert_eq!(
        sheet.source(),
        &ketchup_model::drawing::DrawingSource::RigidAssemblyInstances {
            instance_paths: instance_paths.clone()
        }
    );
    assert_eq!(sheet.bom_balloons().len(), 2);
    assert!(
        sheet
            .bom_balloons()
            .iter()
            .all(|balloon| balloon.position() == 1)
    );
    assert_eq!(
        sheet
            .bom_balloons()
            .iter()
            .map(|balloon| balloon.instance_path().clone())
            .collect::<Vec<_>>(),
        instance_paths
    );
    let committed_digest = committed.canonical_digest();
    let bytes = ketchup_model::persistence::save(&committed);
    let reopened = ketchup_model::persistence::load(&bytes).unwrap();
    assert_eq!(
        reopened.source_schema(),
        ketchup_model::persistence::CURRENT_SCHEMA
    );
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(
        reopened
            .snapshot()
            .drawing_sheets()
            .next()
            .unwrap()
            .source(),
        sheet.source()
    );

    drop(committed);
    assert!(app.undo());
    assert!(app.document.current().drawing_sheets().next().is_none());
    assert!(app.redo());
    assert_eq!(app.document.current().canonical_digest(), committed_digest);
}

#[test]
fn purge_preserves_definitions_referenced_by_nested_component_occurrences() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    assert_eq!(app.definition_count(), 3);
    assert_eq!(app.purgeable_definition_count(), 0);

    assert!(app.delete_selected());
    assert_eq!(app.occurrence_count(), 0);
    assert_eq!(app.purgeable_definition_count(), 0);
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    assert!(!app.purge_unused_definitions());
    assert_eq!(app.definition_count(), 3);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
}

#[test]
fn purge_source_plan_is_exact_stale_safe_and_one_undo_step() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let expected_definition_ids = app
        .document
        .current()
        .definitions()
        .map(|definition| definition.id())
        .collect::<BTreeSet<_>>();
    assert_eq!(expected_definition_ids.len(), 2);
    assert!(app.select_all());
    assert!(app.delete_selected());

    let plan = app.purge_unused_source_plan().unwrap();
    assert_eq!(plan.definition_ids, expected_definition_ids);
    assert_eq!(plan.definition_count, 2);
    let stale_plan = plan.clone();
    assert!(app.undo());
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    assert!(!app.apply_purge_unused_source_plan(stale_plan));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(app.redo());
    let before_purge_steps = app.undo_step_count();
    assert!(app.apply_purge_unused_source_plan(plan));
    assert_eq!(app.definition_count(), 0);
    assert_eq!(app.undo_step_count(), before_purge_steps + 1);
    assert!(app.undo());
    assert_eq!(app.definition_count(), 2);
    assert!(app.redo());
    assert_eq!(app.definition_count(), 0);
}

#[test]
fn nested_edits_require_a_matching_snapshot_bound_context() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    let nested = app.active_boxes()[0].clone();
    let selection = SelectionId {
        definition_id: nested.definition_id,
        instance_path: nested.instance_path.clone(),
        element: ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        },
    };
    app.selection.select_exact(selection.clone(), false);
    let before_revision = app.document_revision();
    let before_digest = app.document.current().canonical_digest();
    app.set_push_pull_distance_input("5");
    assert!(!app.start_preview());
    assert!(!app.move_selected(Vec3::new(10.0, 0.0, 0.0)));
    assert_eq!(app.document_revision(), before_revision);
    assert_eq!(app.document.current().canonical_digest(), before_digest);

    let component_path = InstancePath::root(nested.instance_path.root_occurrence());
    assert!(app.enter_occurrence_context(component_path));
    app.selection.select_exact(selection.clone(), false);
    assert!(app.start_preview());
    assert!(app.preview_action_digest().is_some());
    assert!(app.confirm_preview());
    let committed_digest = app.document.current().canonical_digest();
    assert_ne!(committed_digest, before_digest);
    assert_eq!(app.document_revision(), before_revision + 1);
    assert!(app.undo());
    assert_eq!(app.document.current().canonical_digest(), before_digest);
    assert!(app.redo());
    assert_eq!(app.document.current().canonical_digest(), committed_digest);

    app.selection.select_exact(selection, false);
    app.set_push_pull_distance_input("5");
    assert!(app.start_preview());
    let stale_revision = app.document_revision();
    let stale_digest = app.document.current().canonical_digest();
    assert!(app.exit_edit_context());
    assert!(!app.confirm_preview());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.document.current().canonical_digest(), stale_digest);
}

#[test]
fn edit_context_blocks_selection_leakage_and_exits_one_level_at_a_time() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();

    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert_eq!(app.selection.selected_group, Some(group_id));
    assert!(app.enter_occurrence_context(InstancePath::root(OccurrenceId(1))));
    assert_eq!(
        app.selection.edit_context,
        vec![EditContext::Group(group_id)]
    );

    app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
    assert_eq!(app.selection_count(), 0);
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert_eq!(app.selection_count(), 1);
    assert!(app.enter_occurrence_context(InstancePath::root(OccurrenceId(1))));
    assert!(matches!(
        app.selection.edit_context.last(),
        Some(EditContext::Definition {
            definition_id: DefinitionId(1),
            instance_path,
        }) if *instance_path == InstancePath::root(OccurrenceId(1))
    ));

    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    assert_eq!(app.selection_count(), 0);
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert_eq!(app.selection_count(), 1);
    let tag = TagId(91_001);
    assert!(
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateTag {
                    id: tag,
                    name: "Local guard".to_owned(),
                    visible: true,
                },
                CanonicalCommand::SetOccurrenceTags {
                    id: OccurrenceId(1),
                    tags: [tag].into(),
                },
            ]))
            .is_ok()
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let local_selection = app.selected_occurrence_ids();
    app.begin_tag_creation(Some(local_selection));
    assert!(!app.tag_creation_visible());
    assert!(!app.assign_selection_to_tag(tag));
    assert!(!app.remove_selection_from_tag(tag));
    assert!(!app.isolate_selected_tags());
    assert!(!app.hide_selected_tags());
    assert!(!app.show_selected_tags());
    assert!(!app.invert_selected_tags());
    assert!(!app.select_matching_tags());
    assert!(!app.select_tag_occurrences(tag));
    assert!(!app.select_all_tagged_occurrences());
    assert!(!app.select_untagged_occurrences());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    app.clear_selection();
    assert!(app.exit_edit_context());
    assert_eq!(
        app.selection.edit_context,
        vec![EditContext::Group(group_id)]
    );
    assert!(app.exit_edit_context());
    assert!(app.selection.edit_context.is_empty());
}

#[test]
fn used_local_tag_deletion_fails_closed_without_mutation() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    let tag = TagId(91_002);
    assert!(
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateTag {
                    id: tag,
                    name: "Local delete guard".to_owned(),
                    visible: true,
                },
                CanonicalCommand::SetOccurrenceTags {
                    id: OccurrenceId(1),
                    tags: [tag].into(),
                },
            ]))
            .is_ok()
    );
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    assert!(!app.can_delete_tag(tag));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    app.begin_tag_deletion(tag);
    app.begin_tag_clear(tag);

    assert!(!app.tag_deletion_visible());
    assert!(!app.confirm_tag_deletion());
    assert!(!app.tag_clear_visible());
    assert!(!app.confirm_tag_clear());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert!(app.document.current().tag(tag).is_some());
    assert!(
        app.document
            .current()
            .local_occurrences()
            .any(|occurrence| occurrence.tags().first().copied() == Some(tag))
    );
}

#[test]
fn copy_plan_is_exact_document_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    let plan = app.copy_source_plan().unwrap();
    assert_eq!(
        plan.occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.occurrence_count, 2);
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_copy_source_plan(plan.clone()));

    assert_eq!(
        app.clipboard.occurrences,
        vec![OccurrenceId(1), OccurrenceId(2)]
    );
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    let action_digest = app.action_digest().to_owned();

    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    assert!(app.copy_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Copy));
    assert!(!app.apply_copy_source_plan(plan));
    assert_eq!(
        app.clipboard.occurrences,
        vec![OccurrenceId(1), OccurrenceId(2)]
    );
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);

    app.selection.selected_group = Some(GroupId(999));
    assert!(app.copy_source_plan().is_none());
}

#[test]
fn cut_plan_is_exact_atomic_pasteable_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    let plan = app.cut_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.occurrence_count, 2);
    assert_eq!(plan.clipboard.len(), 2);
    assert_eq!(plan.commands.len(), 2);
    let before_cut = app.canonical_digest();
    let revision = app.document_revision();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_cut_source_plan(plan));

    assert_eq!(app.occurrence_count(), 0);
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(
        app.clipboard.occurrences,
        vec![OccurrenceId(1), OccurrenceId(2)]
    );
    assert_eq!(app.clipboard.cut_occurrences.len(), 2);
    assert!(app.command_enabled(AppCommand::Paste));
    assert!(app.paste_clipboard());
    assert_eq!(app.occurrence_count(), 2);
    assert_eq!(app.selected_occurrence_count(), 2);
    assert!(app.undo());
    assert_eq!(app.occurrence_count(), 0);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_cut);

    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(1)),
        InstancePath::root(OccurrenceId(2)),
    ]);
    let stale_plan = app.cut_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_clipboard = app.clipboard.occurrences.clone();
    assert!(!app.apply_cut_source_plan(stale_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.clipboard.occurrences, stale_clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(app.cut_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Cut));
}

#[test]
fn paste_plan_is_exact_atomic_context_bound_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    assert!(app.copy_selection_to_clipboard());
    let plan = app.paste_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.source_occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.source_occurrence_count, 2);
    assert_eq!(plan.commands.len(), 2);
    assert_eq!(
        plan.pasted
            .iter()
            .map(|(occurrence_id, _)| *occurrence_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([OccurrenceId(3), OccurrenceId(4)])
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_paste_source_plan(plan));

    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.occurrence_count(), 4);
    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.redo());
    assert_eq!(app.occurrence_count(), 4);

    let stale_plan = app.paste_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_paste_source_plan(stale_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(app.paste_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Paste));
    assert!(!app.paste_clipboard());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    app.selection.edit_context.clear();

    app.clipboard.occurrences = vec![OccurrenceId(2), OccurrenceId(1)];
    assert!(app.paste_source_plan().is_none());
    app.clipboard.occurrences = vec![OccurrenceId(1), OccurrenceId(1)];
    assert!(app.paste_source_plan().is_none());
}

#[test]
fn duplicate_plan_is_exact_atomic_clipboard_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    let plan = app.duplicate_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.source_occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.source_occurrence_count, 2);
    assert_eq!(plan.commands.len(), 2);
    assert_eq!(
        plan.duplicated
            .iter()
            .map(|(occurrence_id, _)| *occurrence_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([OccurrenceId(3), OccurrenceId(4)])
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_duplicate_source_plan(plan));

    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.occurrence_count(), 4);
    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(app.clipboard.occurrences, clipboard);
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), digest);
    assert!(app.redo());
    assert_eq!(app.occurrence_count(), 4);
    assert_eq!(app.clipboard.occurrences, clipboard);
    app.select_from_outliner(InstancePath::root(OccurrenceId(3)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(4)), true);

    let stale_plan = app.duplicate_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_duplicate_source_plan(stale_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection.selected_group = Some(GroupId(999));
    assert!(app.duplicate_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Duplicate));
    app.selection.selected_group = None;
    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(app.duplicate_source_plan().is_none());
    assert!(!app.duplicate_selection());
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn delete_plan_is_exact_atomic_clipboard_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    assert!(app.create_box());
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(3)));
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    let plan = app.delete_selection_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(plan.occurrence_count, 2);
    assert!(plan.group_ids.is_empty());
    assert_eq!(plan.group_count, 0);
    assert_eq!(plan.commands.len(), 2);
    let before_delete = app.canonical_digest();
    let revision = app.document_revision();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_delete_selection_source_plan(plan));

    assert_eq!(app.occurrence_count(), 1);
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(app.clipboard.occurrences, clipboard);
    assert!(app.command_enabled(AppCommand::Paste));
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-deleted",
            &BTreeMap::from([("count", "2".to_owned())]),
        )
    );
    assert!(app.undo());
    assert_eq!(app.canonical_digest(), before_delete);
    assert_eq!(app.clipboard.occurrences, clipboard);
    assert!(app.redo());
    assert_eq!(app.occurrence_count(), 1);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.undo());
    app.selection.clear();
    app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(1)),
        InstancePath::root(OccurrenceId(2)),
    ]);
    let stale_plan = app.delete_selection_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_delete_selection_source_plan(stale_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    assert!(app.delete_selection_source_plan().is_none());
    assert!(!app.command_enabled(AppCommand::Delete));
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(app.delete_selection_source_plan().is_none());
    assert!(!app.delete_selected());

    let mut group_app = KetchupApp::new();
    assert!(group_app.create_box());
    group_app.selection.clear();
    group_app.selection.occurrences.extend([
        InstancePath::root(OccurrenceId(2)),
        InstancePath::root(OccurrenceId(1)),
    ]);
    assert!(group_app.group_selected());
    let group_plan = group_app.delete_selection_source_plan().unwrap();
    assert_eq!(group_plan.source_revision, group_app.document_revision());
    assert_eq!(group_plan.occurrence_count, 2);
    assert_eq!(group_plan.group_count, 1);
    assert_eq!(group_plan.commands.len(), 3);
    assert!(group_app.apply_delete_selection_source_plan(group_plan));
    assert_eq!(group_app.occurrence_count(), 0);
    assert_eq!(group_app.group_count(), 0);
    assert!(group_app.undo());
    assert_eq!(group_app.occurrence_count(), 2);
    assert_eq!(group_app.group_count(), 1);
}

#[test]
fn deselect_plan_is_exact_clipboard_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.selection.selected_group = Some(GroupId(999));
    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(777)));
    let edit_context = app.selection.edit_context.clone();
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    app.clipboard.occurrences = vec![OccurrenceId(1)];
    let clipboard = app.clipboard.occurrences.clone();
    let plan = app.deselect_source_plan().unwrap();
    assert_eq!(plan.source_revision, revision);
    assert!(plan.occurrence_paths.is_empty());
    assert_eq!(plan.occurrence_count, 0);
    assert!(plan.primary.is_none());
    assert_eq!(plan.selected_group, Some(GroupId(999)));
    assert_eq!(plan.edit_context, edit_context);
    assert!(app.command_enabled(AppCommand::Deselect));

    assert!(app.clear_selection());

    assert!(app.selection.occurrences.is_empty());
    assert!(app.selection.primary.is_none());
    assert!(app.selection.selected_group.is_none());
    assert_eq!(app.selection.edit_context, edit_context);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.clipboard.occurrences, clipboard);
    assert!(!app.command_enabled(AppCommand::Deselect));
    let action_digest = app.action_digest().to_owned();
    assert!(!app.clear_selection());
    assert_eq!(app.action_digest(), action_digest);

    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    assert!(app.deselect_source_plan().is_some());
    assert!(app.clear_selection());
    assert!(app.selection.occurrences.is_empty());
    assert_eq!(app.selection.edit_context, edit_context);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let mut stale_app = KetchupApp::new();
    stale_app.selection.clear();
    stale_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let stale_revision_plan = stale_app.deselect_source_plan().unwrap();
    assert!(stale_app.create_box());
    let stale_revision = stale_app.document_revision();
    let stale_digest = stale_app.canonical_digest();
    let stale_undo_steps = stale_app.undo_step_count();
    let stale_action_digest = stale_app.action_digest().to_owned();
    let stale_occurrences = stale_app.selection.occurrences.clone();
    let stale_primary = stale_app.selection.primary.clone();
    let stale_selected_group = stale_app.selection.selected_group;
    let stale_edit_context = stale_app.selection.edit_context.clone();
    assert!(!stale_app.apply_deselect_source_plan(stale_revision_plan));
    assert_eq!(stale_app.document_revision(), stale_revision);
    assert_eq!(stale_app.canonical_digest(), stale_digest);
    assert_eq!(stale_app.undo_step_count(), stale_undo_steps);
    assert_eq!(stale_app.action_digest(), stale_action_digest);
    assert_eq!(stale_app.selection.occurrences, stale_occurrences);
    assert_eq!(stale_app.selection.primary, stale_primary);
    assert_eq!(stale_app.selection.selected_group, stale_selected_group);
    assert_eq!(stale_app.selection.edit_context, stale_edit_context);

    stale_app.selection.clear();
    stale_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let stale_selection_plan = stale_app.deselect_source_plan().unwrap();
    stale_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    assert!(!stale_app.apply_deselect_source_plan(stale_selection_plan));
    assert_eq!(stale_app.selected_occurrence_count(), 2);

    stale_app.selection.clear();
    stale_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let stale_context_plan = stale_app.deselect_source_plan().unwrap();
    stale_app
        .selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!stale_app.apply_deselect_source_plan(stale_context_plan));
    assert_eq!(stale_app.selected_occurrence_count(), 1);

    stale_app.selection.edit_context.clear();
    let mut tampered_plan = stale_app.deselect_source_plan().unwrap();
    tampered_plan.occurrence_count += 1;
    assert!(!stale_app.apply_deselect_source_plan(tampered_plan));
    assert_eq!(stale_app.selected_occurrence_count(), 1);
}

#[test]
fn select_all_plan_is_exact_clipboard_preserving_and_stale_safe() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    let copy_plan = app.copy_source_plan().unwrap();
    assert!(app.apply_copy_source_plan(copy_plan));
    let clipboard = app.clipboard.occurrences.clone();
    app.selection.clear();

    let plan = app.select_all_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert!(plan.source_occurrence_paths.is_empty());
    assert!(plan.source_primary.is_none());
    assert!(plan.source_selected_group.is_none());
    assert!(plan.edit_context.is_empty());
    assert_eq!(plan.target_count, 2);
    assert_eq!(
        plan.target_instance_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.apply_select_all_source_plan(plan));

    assert_eq!(app.selected_occurrence_count(), 2);
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.clipboard.occurrences, clipboard);
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-selected-all",
            &BTreeMap::from([("count", "2".to_owned())])
        )
    );
    assert!(app.select_all_source_plan().is_none());
    assert!(!app.select_all());

    app.selection.clear();
    let stale_revision_plan = app.select_all_source_plan().unwrap();
    assert!(app.create_box());
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_select_all_source_plan(stale_revision_plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection.clear();
    let stale_selection_plan = app.select_all_source_plan().unwrap();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    assert!(!app.apply_select_all_source_plan(stale_selection_plan));
    assert_eq!(app.selected_occurrence_count(), 1);

    app.selection.clear();
    let stale_context_plan = app.select_all_source_plan().unwrap();
    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_select_all_source_plan(stale_context_plan));
    assert!(app.selection.occurrences.is_empty());
    assert_eq!(app.clipboard.occurrences, clipboard);

    let mut tampered_app = KetchupApp::new();
    let mut tampered_plan = tampered_app.select_all_source_plan().unwrap();
    tampered_plan.target_count += 1;
    assert!(!tampered_app.apply_select_all_source_plan(tampered_plan));
    assert_eq!(tampered_app.selected_occurrence_count(), 0);
}

#[test]
fn select_all_plan_stays_inside_group_and_definition_contexts() {
    let mut group_app = KetchupApp::new();
    assert!(group_app.create_box());
    assert!(group_app.create_box());
    group_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    group_app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(group_app.group_selected());
    let group_id = group_app.selection.selected_group.unwrap();
    assert!(group_app.enter_group_context(group_id));
    let group_plan = group_app.select_all_source_plan().unwrap();
    assert_eq!(
        group_plan.target_instance_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    let group_revision = group_app.document_revision();
    let group_digest = group_app.canonical_digest();
    let group_undo_steps = group_app.undo_step_count();
    assert!(group_app.select_all());
    assert!(!group_app.command_enabled(AppCommand::SelectAll));
    assert_eq!(group_app.document_revision(), group_revision);
    assert_eq!(group_app.canonical_digest(), group_digest);
    assert_eq!(group_app.undo_step_count(), group_undo_steps);

    let mut definition_app = KetchupApp::new();
    definition_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(definition_app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    definition_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    definition_app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(definition_app.group_selected());
    assert!(definition_app.make_component());
    let component_path = definition_app
        .selection
        .occurrences
        .first()
        .unwrap()
        .clone();
    assert!(definition_app.enter_occurrence_context(component_path.clone()));
    let definition_plan = definition_app.select_all_source_plan().unwrap();
    assert!(definition_plan.target_instance_paths.len() >= 2);
    assert!(
        definition_plan
            .target_instance_paths
            .iter()
            .all(|path| path.root_occurrence() == component_path.root_occurrence())
    );
    assert!(definition_app.select_all());
    assert_eq!(
        definition_app.selected_instance_paths(),
        definition_plan.target_instance_paths
    );
    assert!(!definition_app.command_enabled(AppCommand::SelectAll));
}

#[test]
fn select_all_instances_stays_inside_the_active_group_context() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    assert!(app.copy_selected(Vec3::new(300.0, 0.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    assert!(app.enter_group_context(group_id));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.command_enabled(AppCommand::SelectAllInstances));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    app.dispatch_command(AppCommand::SelectAllInstances);

    assert_eq!(
        app.selected_instance_paths(),
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}

#[test]
fn select_all_instances_rejects_group_and_missing_selections_without_mutation() {
    let mut group_app = KetchupApp::new();
    group_app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(group_app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    let group_id = GroupId(10);
    group_app
        .document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: group_id,
                name: "Single".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(group_id),
            },
        ]))
        .unwrap();
    assert!(group_app.select_group(group_id));
    let revision = group_app.document_revision();
    let digest = group_app.canonical_digest();
    let undo_steps = group_app.undo_step_count();
    let action_digest = group_app.action_digest().to_owned();

    assert!(group_app.select_all_instances_source_plan().is_none());
    assert!(!group_app.command_enabled(AppCommand::SelectAllInstances));
    assert!(!group_app.select_all_instances());
    assert_eq!(group_app.document_revision(), revision);
    assert_eq!(group_app.canonical_digest(), digest);
    assert_eq!(group_app.undo_step_count(), undo_steps);
    assert_eq!(group_app.action_digest(), action_digest);

    let mut missing_app = KetchupApp::new();
    missing_app.selection.clear();
    missing_app
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let action_digest = missing_app.action_digest().to_owned();
    assert!(missing_app.select_all_instances_source_plan().is_none());
    assert!(!missing_app.command_enabled(AppCommand::SelectAllInstances));
    assert!(!missing_app.select_all_instances());
    assert_eq!(missing_app.action_digest(), action_digest);
}

#[test]
fn select_all_instances_plan_rejects_tampering_context_drift_and_staleness_without_side_effects() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    let plan = app.select_all_instances_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.source_instance_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(1))])
    );
    assert_eq!(plan.source_count, 1);
    assert_eq!(plan.source_primary, None);
    assert_eq!(plan.source_selected_group, None);
    assert!(plan.edit_context.is_empty());
    assert_eq!(
        plan.target_instance_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    assert_eq!(plan.target_count, 2);

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let selection = app.selected_instance_paths();
    let mut tampered = plan.clone();
    tampered.target_count += 1;
    assert!(!app.apply_select_all_instances_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    let context_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_select_all_instances_source_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), context_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);
    app.selection.edit_context.pop();

    assert!(app.set_selected_occurrence_grounded(true));
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_select_all_instances_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn grounded_occurrence_plan_is_exact_and_rejects_tampering_context_drift_and_staleness() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    let plan = app.grounded_occurrence_source_plan(true).unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(plan.source_digest, app.canonical_digest());
    assert_eq!(
        plan.occurrence_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(1))])
    );
    assert_eq!(plan.occurrence_count, 1);
    assert_eq!(plan.source_primary, None);
    assert_eq!(plan.source_selected_group, None);
    assert!(plan.edit_context.is_empty());
    assert_eq!(plan.occurrence_id, OccurrenceId(1));
    assert!(!plan.source_grounded);
    assert!(plan.target_grounded);
    assert_eq!(
        plan.command,
        CanonicalCommand::SetOccurrenceGrounded {
            id: OccurrenceId(1),
            grounded: true,
        }
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let selection = app.selected_instance_paths();
    let mut tampered = plan.clone();
    tampered.command = CanonicalCommand::SetOccurrenceGrounded {
        id: OccurrenceId(1),
        grounded: false,
    };
    assert!(!app.apply_grounded_occurrence_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_grounded_occurrence_source_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.set_selected_occurrence_grounded(true));
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_grounded_occurrence_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn selection_visibility_plan_is_exact_and_rejects_tampering_context_drift_and_staleness() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    let plan = app.selection_visibility_source_plan(false).unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(plan.source_digest, app.canonical_digest());
    assert_eq!(
        plan.occurrence_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(1))])
    );
    assert_eq!(plan.occurrence_count, 1);
    assert_eq!(plan.source_primary, None);
    assert_eq!(plan.source_selected_group, None);
    assert!(plan.edit_context.is_empty());
    assert_eq!(
        plan.source_visibility,
        BTreeMap::from([(OccurrenceId(1), true)])
    );
    assert_eq!(
        plan.changed_occurrence_ids,
        BTreeSet::from([OccurrenceId(1)])
    );
    assert!(!plan.target_visible);
    assert_eq!(
        plan.commands,
        vec![CanonicalCommand::SetOccurrenceVisibility {
            id: OccurrenceId(1),
            visible: false,
        }]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let selection = app.selected_instance_paths();
    let mut tampered = plan.clone();
    tampered.commands = vec![CanonicalCommand::SetOccurrenceVisibility {
        id: OccurrenceId(1),
        visible: true,
    }];
    assert!(!app.apply_selection_visibility_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_selection_visibility_source_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.set_selected_occurrence_grounded(true));
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_selection_visibility_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn tag_creation_plan_is_exact_and_rejects_tampering_namespace_selection_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let existing_tag = TagId(700);
    let created_tag = TagId(701);
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
            id: existing_tag,
            name: "Other".to_owned(),
            visible: false,
        }]))
        .unwrap();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let occurrence_ids = BTreeSet::from([OccurrenceId(1)]);
    let selection = app.selected_instance_paths();
    let clipboard = app.clipboard.occurrences.clone();
    let source = app.tag_creation_source_plan(Some(&occurrence_ids)).unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.tags,
        BTreeMap::from([(existing_tag, ("Other".to_owned(), false))])
    );
    assert_eq!(source.id, created_tag);
    assert_eq!(source.occurrence_ids, Some(occurrence_ids));
    let plan = app.tag_creation_plan(&source, "  Hardware  ").unwrap();
    assert_eq!(plan.target_name, "Hardware");
    assert_eq!(
        plan.commands,
        vec![
            CanonicalCommand::CreateTag {
                id: created_tag,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(1),
                tags: [created_tag].into(),
            },
        ]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.commands.pop();
    assert!(!app.apply_tag_creation_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source
        .source
        .tags
        .get_mut(&existing_tag)
        .unwrap()
        .0
        .push('!');
    assert!(!app.apply_tag_creation_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);

    app.clear_selection();
    assert!(!app.apply_tag_creation_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.clipboard.occurrences, clipboard);
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);

    assert!(app.apply_tag_creation_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let created = app.document_snapshot();
    assert_eq!(created.tag(created_tag).unwrap().name(), "Hardware");
    assert!(created.tag(created_tag).unwrap().visible());
    assert_eq!(
        created
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        Some(created_tag)
    );
    assert_eq!(created.tag(existing_tag).unwrap().name(), "Other");
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_creation_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);

    assert!(app.undo());
    assert!(app.document_snapshot().tag(created_tag).is_none());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        None
    );
    assert!(app.redo());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        Some(created_tag)
    );
}

#[test]
fn tag_clear_plan_is_exact_and_rejects_tampering_namespace_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let tag = TagId(698);
    let other_tag = TagId(699);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: tag,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: other_tag,
                name: "Other".to_owned(),
                visible: false,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(1),
                tags: [tag].into(),
            },
        ]))
        .unwrap();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let selection = app.selected_instance_paths();
    let clipboard = app.clipboard.occurrences.clone();
    let source = app.tag_clear_source_plan(tag).unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.tags,
        BTreeMap::from([
            (tag, ("Hardware".to_owned(), true)),
            (other_tag, ("Other".to_owned(), false)),
        ])
    );
    assert_eq!(source.id, tag);
    assert_eq!(source.original_name, "Hardware");
    assert!(source.original_visible);
    assert_eq!(source.occurrence_ids, BTreeSet::from([OccurrenceId(1)]));
    let plan = app.tag_clear_plan(&source).unwrap();
    assert_eq!(
        plan.commands,
        vec![CanonicalCommand::SetOccurrenceTags {
            id: OccurrenceId(1),
            tags: Default::default(),
        }]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.commands = vec![CanonicalCommand::SetOccurrenceTags {
        id: OccurrenceId(1),
        tags: [other_tag].into(),
    }];
    assert!(!app.apply_tag_clear_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source
        .source
        .tags
        .get_mut(&other_tag)
        .unwrap()
        .0
        .push('!');
    assert!(!app.apply_tag_clear_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.apply_tag_clear_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let cleared = app.document_snapshot();
    assert_eq!(cleared.tag(tag).unwrap().name(), "Hardware");
    assert!(cleared.tag(tag).unwrap().visible());
    assert_eq!(
        cleared
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        None
    );
    assert_eq!(cleared.tag(other_tag).unwrap().name(), "Other");
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_clear_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.undo());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        Some(tag)
    );
    assert!(app.redo());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        None
    );
}

#[test]
fn tag_deletion_plan_is_exact_and_rejects_tampering_namespace_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let tag = TagId(700);
    let other_tag = TagId(701);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: tag,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: other_tag,
                name: "Other".to_owned(),
                visible: false,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(1),
                tags: [tag].into(),
            },
        ]))
        .unwrap();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let selection = app.selected_instance_paths();
    let clipboard = app.clipboard.occurrences.clone();
    let source = app.tag_deletion_source_plan(tag).unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.tags,
        BTreeMap::from([
            (tag, ("Hardware".to_owned(), true)),
            (other_tag, ("Other".to_owned(), false)),
        ])
    );
    assert_eq!(source.id, tag);
    assert_eq!(source.original_name, "Hardware");
    assert!(source.original_visible);
    assert_eq!(source.occurrence_ids, BTreeSet::from([OccurrenceId(1)]));
    let plan = app.tag_deletion_plan(&source).unwrap();
    assert_eq!(
        plan.commands,
        vec![
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(1),
                tags: Default::default(),
            },
            CanonicalCommand::DeleteTag { id: tag },
        ]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.commands = vec![CanonicalCommand::DeleteTag { id: tag }];
    assert!(!app.apply_tag_deletion_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source
        .source
        .tags
        .get_mut(&other_tag)
        .unwrap()
        .0
        .push('!');
    assert!(!app.apply_tag_deletion_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.apply_tag_deletion_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let deleted = app.document_snapshot();
    assert!(deleted.tag(tag).is_none());
    assert_eq!(
        deleted
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        None
    );
    assert_eq!(deleted.tag(other_tag).unwrap().name(), "Other");
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_deletion_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.undo());
    assert_eq!(app.document_snapshot().tag(tag).unwrap().name(), "Hardware");
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        Some(tag)
    );
    assert!(app.redo());
    assert!(app.document_snapshot().tag(tag).is_none());
    assert_eq!(
        app.document_snapshot()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        None
    );
}

#[test]
fn tag_rename_plan_is_exact_and_rejects_tampering_namespace_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let tag = TagId(700);
    let other_tag = TagId(701);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: tag,
                name: "Hardware".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: other_tag,
                name: "Other".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(1),
                tags: [tag].into(),
            },
        ]))
        .unwrap();
    app.clear_selection();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let selection = app.selected_instance_paths();
    let clipboard = app.clipboard.occurrences.clone();
    let source = app.tag_rename_source_plan(tag).unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.tags,
        BTreeMap::from([
            (tag, ("Hardware".to_owned(), true)),
            (other_tag, ("Other".to_owned(), true)),
        ])
    );
    assert_eq!(source.id, tag);
    assert_eq!(source.original_name, "Hardware");
    assert!(source.original_visible);
    assert_eq!(source.occurrence_ids, BTreeSet::from([OccurrenceId(1)]));
    assert!(app.tag_rename_plan(&source, "Other").is_none());
    let plan = app.tag_rename_plan(&source, "  Mechanical  ").unwrap();
    assert_eq!(plan.target_name, "Mechanical");
    assert_eq!(
        plan.command,
        CanonicalCommand::SetTagName {
            id: tag,
            name: "Mechanical".to_owned(),
        }
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.command = CanonicalCommand::SetTagName {
        id: tag,
        name: "Tampered".to_owned(),
    };
    assert!(!app.apply_tag_rename_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source
        .source
        .tags
        .get_mut(&other_tag)
        .unwrap()
        .0
        .push('!');
    assert!(!app.apply_tag_rename_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.apply_tag_rename_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    let renamed = app.document_snapshot();
    assert_eq!(renamed.tag(tag).unwrap().name(), "Mechanical");
    assert!(renamed.tag(tag).unwrap().visible());
    assert_eq!(
        renamed
            .occurrence(OccurrenceId(1))
            .unwrap()
            .tags()
            .first()
            .copied(),
        Some(tag)
    );
    assert_eq!(renamed.tag(other_tag).unwrap().name(), "Other");
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_rename_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);
    assert_eq!(app.selected_instance_paths(), selection);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.undo());
    assert_eq!(app.document_snapshot().tag(tag).unwrap().name(), "Hardware");
    assert!(app.redo());
    assert_eq!(
        app.document_snapshot().tag(tag).unwrap().name(),
        "Mechanical"
    );
}

#[test]
fn tag_assignment_plan_is_exact_and_rejects_tampering_context_drift_and_staleness() {
    let mut app = KetchupApp::new();
    let source_tag = TagId(700);
    let target_tag = TagId(701);
    assert!(app.create_box());
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: source_tag,
                name: "Source".to_owned(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: target_tag,
                name: "Target".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(1),
                tags: [source_tag].into(),
            },
        ]))
        .unwrap();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    let source = app.tag_assignment_source_plan().unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(source.source_digest, app.canonical_digest());
    assert_eq!(
        source.occurrence_paths,
        BTreeSet::from([
            InstancePath::root(OccurrenceId(1)),
            InstancePath::root(OccurrenceId(2)),
        ])
    );
    assert_eq!(source.occurrence_count, 2);
    assert_eq!(source.source_primary, None);
    assert_eq!(source.source_selected_group, None);
    assert!(source.edit_context.is_empty());
    assert_eq!(
        source.source_tags,
        BTreeMap::from([
            (OccurrenceId(1), BTreeSet::from([source_tag])),
            (OccurrenceId(2), BTreeSet::new()),
        ])
    );
    assert_eq!(
        source.available_tags.get(&target_tag).map(String::as_str),
        Some("Target")
    );
    assert!(source.initial_tags.is_empty());
    // Both parts end up in both tags; the first keeps the tag it already had.
    let both = BTreeSet::from([source_tag, target_tag]);
    let pending = PendingTagAssignment {
        source,
        target_tags: both.clone(),
    };
    let plan = app.dialog_tag_assignment_plan(&pending).unwrap();
    assert_eq!(
        plan.changed_occurrence_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    assert_eq!(
        plan.commands,
        vec![
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(1),
                tags: both.clone(),
            },
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(2),
                tags: both.clone(),
            },
        ]
    );

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();
    let mut tampered = plan.clone();
    tampered.commands.pop();
    assert!(!app.apply_tag_assignment_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_tag_assignment_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.apply_tag_assignment_plan(plan.clone()));
    assert_eq!(app.document_revision(), revision + 1);
    assert_eq!(app.undo_step_count(), undo_steps + 1);
    assert_eq!(app.occurrence_tags(OccurrenceId(1)), both);
    assert_eq!(app.occurrence_tags(OccurrenceId(2)), both);
    assert_eq!(app.clipboard.occurrences, clipboard);
    let applied_revision = app.document_revision();
    let applied_digest = app.canonical_digest();
    let applied_undo_steps = app.undo_step_count();
    let applied_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_tag_assignment_plan(plan));
    assert_eq!(app.document_revision(), applied_revision);
    assert_eq!(app.canonical_digest(), applied_digest);
    assert_eq!(app.undo_step_count(), applied_undo_steps);
    assert_eq!(app.action_digest(), applied_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.undo());
    assert_eq!(
        app.occurrence_tags(OccurrenceId(1)).first().copied(),
        Some(source_tag)
    );
    assert_eq!(app.occurrence_tags(OccurrenceId(2)).first().copied(), None);
    assert!(app.redo());
    assert_eq!(app.occurrence_tags(OccurrenceId(1)), both);
    assert_eq!(app.occurrence_tags(OccurrenceId(2)), both);
}

#[test]
fn component_replacement_plan_rejects_tampering_context_drift_and_staleness_without_side_effects() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let source = app.component_replacement_source_plan().unwrap();
    assert_eq!(source.source_revision, app.document_revision());
    assert_eq!(
        source.occurrence_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(2))])
    );
    assert_eq!(source.occurrence_count, 1);
    assert_eq!(source.occurrence_id, OccurrenceId(2));
    assert_eq!(source.source_primary, None);
    assert_eq!(source.source_selected_group, None);
    assert!(source.edit_context.is_empty());
    assert_eq!(source.source_definition_id, DefinitionId(2));
    assert_eq!(source.candidate_count, 1);
    assert_eq!(source.initial_target_definition_id, DefinitionId(1));
    assert_eq!(
        source.candidate_definitions.get(&DefinitionId(1)),
        app.definition_name(DefinitionId(1)).as_ref()
    );
    let pending = PendingComponentReplacement {
        source,
        target_definition_id: DefinitionId(1),
    };
    let plan = app.component_replacement_plan(&pending).unwrap();
    assert_eq!(
        plan.command,
        CanonicalCommand::RepointOccurrence {
            id: OccurrenceId(2),
            definition_id: DefinitionId(1),
        }
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    let mut tampered = plan.clone();
    tampered.command = CanonicalCommand::RepointOccurrence {
        id: OccurrenceId(2),
        definition_id: DefinitionId(999),
    };
    assert!(!app.apply_component_replacement_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    let mut tampered_source = plan.clone();
    tampered_source.source.source_definition_name.push('!');
    assert!(!app.apply_component_replacement_plan(tampered_source));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_component_replacement_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_component_replacement_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn make_unique_rejects_a_missing_selected_occurrence_without_mutation() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();

    assert!(app.make_unique_source_plan().is_none());
    assert!(!app.make_unique());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}

#[test]
fn make_unique_plan_rejects_tampering_context_drift_and_staleness_without_side_effects() {
    let mut app = KetchupApp::new();
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    assert!(app.copy_selection_to_clipboard());
    let clipboard = app.clipboard.occurrences.clone();
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let plan = app.make_unique_source_plan().unwrap();
    assert_eq!(plan.source_revision, app.document_revision());
    assert_eq!(
        plan.occurrence_paths,
        BTreeSet::from([InstancePath::root(OccurrenceId(2))])
    );
    assert_eq!(plan.occurrence_id, OccurrenceId(2));
    assert_eq!(plan.source_primary, None);
    assert_eq!(plan.source_selected_group, None);
    assert!(plan.edit_context.is_empty());
    assert_eq!(plan.source_definition_id, DefinitionId(1));
    assert_eq!(plan.visible_peer_count, 2);
    assert_eq!(
        plan.visible_peer_ids,
        BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
    );
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    let mut tampered = plan.clone();
    tampered.command = CloneDefinitionPlan::new(
        OccurrenceId(2),
        DefinitionId(1),
        DefinitionId(999),
        "Tampered".to_owned(),
        Vec::new(),
    );
    assert!(!app.apply_make_unique_source_plan(tampered));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    app.selection
        .edit_context
        .push(EditContext::Group(GroupId(999)));
    assert!(!app.apply_make_unique_source_plan(plan.clone()));
    app.selection.edit_context.clear();
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);

    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    let stale_revision = app.document_revision();
    let stale_digest = app.canonical_digest();
    let stale_undo_steps = app.undo_step_count();
    let stale_action_digest = app.action_digest().to_owned();
    assert!(!app.apply_make_unique_source_plan(plan));
    assert_eq!(app.document_revision(), stale_revision);
    assert_eq!(app.canonical_digest(), stale_digest);
    assert_eq!(app.undo_step_count(), stale_undo_steps);
    assert_eq!(app.action_digest(), stale_action_digest);
    assert_eq!(app.clipboard.occurrences, clipboard);
}

#[test]
fn ground_occurrence_rejects_a_missing_selected_occurrence_without_mutation() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    assert!(app.grounded_occurrence_source_plan(true).is_none());
    assert!(!app.command_enabled(AppCommand::GroundOccurrence));
    assert!(!app.set_selected_occurrence_grounded(true));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
}

#[test]
fn selection_visibility_rejects_a_partly_missing_selection_without_mutation() {
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    app.selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    let action_digest = app.action_digest().to_owned();

    assert!(app.selection_visibility_source_plan(false).is_none());
    assert!(!app.command_enabled(AppCommand::Hide));
    assert!(!app.set_selection_visibility(false));
    assert!(
        app.document
            .current()
            .occurrence(OccurrenceId(1))
            .unwrap()
            .visible()
    );
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
    assert_eq!(app.action_digest(), action_digest);
}

#[test]
fn delete_selection_rejects_missing_and_collection_protected_occurrences() {
    let mut stale = KetchupApp::new();
    stale.selection.clear();
    stale
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(1)));
    stale
        .selection
        .occurrences
        .insert(InstancePath::root(OccurrenceId(999)));
    let stale_revision = stale.document_revision();
    let stale_digest = stale.canonical_digest();
    let stale_undo_steps = stale.undo_step_count();
    let stale_action_digest = stale.action_digest().to_owned();

    assert!(stale.delete_selection_source_plan().is_none());
    assert!(!stale.command_enabled(AppCommand::Delete));
    assert!(!stale.delete_selected());
    assert_eq!(stale.document_revision(), stale_revision);
    assert_eq!(stale.canonical_digest(), stale_digest);
    assert_eq!(stale.undo_step_count(), stale_undo_steps);
    assert_eq!(stale.action_digest(), stale_action_digest);

    let mut protected = KetchupApp::new();
    assert!(
        protected
            .document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateCollection {
                    id: CollectionId(1),
                    name: "Protected".to_owned(),
                },
                CanonicalCommand::SetCollectionOccurrences {
                    id: CollectionId(1),
                    occurrence_ids: vec![OccurrenceId(1)],
                },
            ]))
            .is_ok()
    );
    protected.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    let protected_revision = protected.document_revision();
    let protected_digest = protected.canonical_digest();
    let protected_undo_steps = protected.undo_step_count();
    let protected_action_digest = protected.action_digest().to_owned();

    assert!(protected.delete_selection_source_plan().is_none());
    assert!(!protected.command_enabled(AppCommand::Delete));
    assert!(!protected.delete_selected());
    assert_eq!(protected.document_revision(), protected_revision);
    assert_eq!(protected.canonical_digest(), protected_digest);
    assert_eq!(protected.undo_step_count(), protected_undo_steps);
    assert_eq!(protected.action_digest(), protected_action_digest);
    assert!(
        protected
            .document
            .current()
            .occurrence(OccurrenceId(1))
            .is_some()
    );
}

#[test]
fn organized_component_hierarchy_round_trips_with_stable_identity() {
    let mut app = KetchupApp::new();
    app.selection.select_exact(
        SelectionId {
            definition_id: DefinitionId(1),
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
    assert!(app.copy_selected(Vec3::new(150.0, 0.0, 0.0)));
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    let group_id = app.selection.selected_group.unwrap();
    assert!(app.enter_group_context(group_id));
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), false);
    assert!(app.make_unique());

    let expected = app.document.current();
    let loaded = ketchup_model::persistence::load(&ketchup_model::persistence::save(&expected))
        .unwrap()
        .snapshot();
    assert_eq!(loaded.canonical_digest(), expected.canonical_digest());
    assert_eq!(
        loaded.occurrence(OccurrenceId(1)).unwrap().parent(),
        Some(group_id)
    );
    assert_eq!(
        loaded.occurrence(OccurrenceId(2)).unwrap().parent(),
        Some(group_id)
    );
    assert_ne!(
        loaded.occurrence(OccurrenceId(1)).unwrap().definition_id(),
        loaded.occurrence(OccurrenceId(2)).unwrap().definition_id()
    );
}
