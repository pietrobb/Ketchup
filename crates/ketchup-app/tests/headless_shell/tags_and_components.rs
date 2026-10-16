use super::*;

#[test]
fn delete_is_localized_atomic_group_aware_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Delete),
            shell.catalog().text("action-delete")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Delete));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(shell.app().command_is_enabled(AppCommand::Delete));
        let before_delete = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.press_key(Key::Delete);

        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-deleted",
                &BTreeMap::from([("count", "2".to_owned())]),
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Delete));
        let deleted = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().canonical_digest(), before_delete);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), deleted);

        let mut group_shell = Shell::with_catalog(catalog.clone());
        group_shell.click_at(group_shell.top_face_centre(1));
        group_shell.click_menu_command("menu-edit", AppCommand::Copy);
        group_shell.click_menu_command("menu-edit", AppCommand::Paste);
        group_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        group_shell.click_menu_command("menu-model", AppCommand::Group);
        assert_eq!(group_shell.app().group_count(), 1);
        assert!(group_shell.app().command_is_enabled(AppCommand::Delete));
        let grouped = group_shell.app().canonical_digest();
        let grouped_revision = group_shell.app().document_revision();
        let grouped_undo_steps = group_shell.app().undo_step_count();

        group_shell.click_menu_command("menu-edit", AppCommand::Delete);

        assert_eq!(group_shell.app().occurrence_count(), 0);
        assert_eq!(group_shell.app().group_count(), 0);
        assert_eq!(group_shell.app().document_revision(), grouped_revision + 1);
        assert_eq!(group_shell.app().undo_step_count(), grouped_undo_steps + 1);
        assert_eq!(
            group_shell.app().action_digest(),
            group_shell.catalog().format(
                "digest-deleted",
                &BTreeMap::from([("count", "2".to_owned())]),
            )
        );
        let group_deleted = group_shell.app().canonical_digest();
        group_shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(group_shell.app().occurrence_count(), 2);
        assert_eq!(group_shell.app().group_count(), 1);
        assert_eq!(group_shell.app().canonical_digest(), grouped);
        group_shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(group_shell.app().occurrence_count(), 0);
        assert_eq!(group_shell.app().group_count(), 0);
        assert_eq!(group_shell.app().canonical_digest(), group_deleted);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(!context_shell.app().command_is_enabled(AppCommand::Delete));
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-edit", AppCommand::Delete);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn group_ungroup_is_localized_atomic_and_context_bound() {
    for (locale_index, catalog) in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Group),
            shell.catalog().text("model-group")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::Ungroup),
            shell.catalog().text("model-ungroup")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Group));
        assert!(!shell.app().command_is_enabled(AppCommand::Ungroup));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Group));
        let ungrouped = shell.app().canonical_digest();
        let group_revision = shell.app().document_revision();
        let group_undo_steps = shell.app().undo_step_count();

        if locale_index == 0 {
            shell.key(Key::G, ctrl());
        } else {
            shell.click_menu_command("menu-model", AppCommand::Group);
        }

        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().document_revision(), group_revision + 1);
        assert_eq!(shell.app().undo_step_count(), group_undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-grouped",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Group));
        assert!(shell.app().command_is_enabled(AppCommand::Ungroup));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let grouped = shell.app().canonical_digest();
        let ungroup_revision = shell.app().document_revision();
        let ungroup_undo_steps = shell.app().undo_step_count();

        if locale_index == 0 {
            shell.click_menu_command("menu-model", AppCommand::Ungroup);
        } else {
            shell.key(Key::G, harness::ctrl_shift());
        }

        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().document_revision(), ungroup_revision + 1);
        assert_eq!(shell.app().undo_step_count(), ungroup_undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-ungrouped",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert!(shell.app().command_is_enabled(AppCommand::Group));
        assert!(!shell.app().command_is_enabled(AppCommand::Ungroup));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let regrouped = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().canonical_digest(), grouped);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().canonical_digest(), ungrouped);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().canonical_digest(), regrouped);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(!context_shell.app().command_is_enabled(AppCommand::Group));
        assert!(!context_shell.app().command_is_enabled(AppCommand::Ungroup));
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-model", AppCommand::Group);
        context_shell.click_menu_command("menu-model", AppCommand::Ungroup);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn make_component_directly_from_shift_selected_objects_is_atomic() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    shell.click_menu_command("menu-edit", AppCommand::Copy);
    shell.click_menu_command("menu-edit", AppCommand::Paste);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    shell.click_at(shell.top_face_centre(1));
    shell.click_at_with(shell.top_face_centre(2), shift());

    assert_eq!(shell.app().selected_occurrence_count(), 2);
    assert!(shell.app().command_is_enabled(AppCommand::Group));
    assert!(shell.app().command_is_enabled(AppCommand::MakeComponent));
    let before = shell.app().canonical_digest();
    let revision = shell.app().document_revision();
    let undo_steps = shell.app().undo_step_count();

    shell.click_menu_command("menu-model", AppCommand::MakeComponent);

    assert_eq!(shell.app().group_count(), 0);
    assert_eq!(shell.app().occurrence_count(), 1);
    assert_eq!(shell.app().definition_count(), 2);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(3)));
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
    let converted = shell.app().canonical_digest();

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().group_count(), 0);
    assert_eq!(shell.app().occurrence_count(), 2);
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().group_count(), 0);
    assert_eq!(shell.app().occurrence_count(), 1);
    assert_eq!(shell.app().canonical_digest(), converted);
}

#[test]
fn nested_group_component_context_is_visible_shared_and_selection_safe() {
    let mut shell = Shell::new();
    let outside = shell.catalog().text("viewport-context-outside");
    assert!(shell.has_visible_label(&outside));

    shell.click_at(shell.top_face_centre(1));
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-model", AppCommand::Group);
    assert_eq!(shell.app().group_count(), 1);
    assert_eq!(shell.app().definition_count(), 1);

    shell.double_click_at(shell.top_face_centre(1));
    assert_eq!(shell.app().edit_context_depth(), 1);
    let group_context = shell.app().edit_context_readout();
    assert!(group_context.contains("Model > Group 1"), "{group_context}");
    assert!(shell.has_visible_label(&group_context));

    shell.double_click_at(shell.top_face_centre(1));
    assert_eq!(shell.app().edit_context_depth(), 2);
    let component_context = shell.app().edit_context_readout();
    assert!(
        component_context.contains("Model > Group 1 > Box-1"),
        "{component_context}"
    );
    assert!(shell.has_visible_label(&component_context));
    let shared = shell.catalog().format(
        "viewport-context-shared",
        &BTreeMap::from([("count", "2".to_owned())]),
    );
    assert!(shell.has_visible_label(&shared));
    assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));

    let (origin, size) = shell.app().occurrence_box_geometry(2).unwrap();
    let external_midpoint = shell
        .app()
        .viewport_position(origin + Vec3::new(size.x * 0.5, 0.0, size.z))
        .unwrap();
    shell.move_pointer(external_midpoint);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Midpoint));
    assert!(shell.app().hovered_selection().is_none());
    shell.click_at(shell.top_face_centre(2));
    assert_eq!(shell.app().selected_occurrence_count(), 0);

    shell.press_key(Key::Escape);
    assert_eq!(shell.app().edit_context_depth(), 1);
    assert!(shell.has_visible_label(&group_context));
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().edit_context_depth(), 0);
    assert!(shell.has_visible_label(&outside));

    // With the exact body shown a viewport click picks the part inside the
    // group, so the group itself is selected in the outliner.
    wait_for_one_exact_body(&mut shell);
    shell.click_row(&shell.catalog().format(
        "outliner-group",
        &BTreeMap::from([("name", "Group 1".to_owned()), ("count", "2".to_owned())]),
    ));
    assert!(shell.app().command_is_enabled(AppCommand::Ungroup));
    shell.click_menu_command("menu-model", AppCommand::Ungroup);
    shell.click_at(shell.top_face_centre(2));
    assert!(shell.app().command_is_enabled(AppCommand::MakeUnique));
    let shared_definition = shell.app().occurrence_definition_id(OccurrenceId(1));
    shell.click_menu_command("menu-model", AppCommand::MakeUnique);
    assert_ne!(
        shell.app().occurrence_definition_id(OccurrenceId(2)),
        shared_definition
    );

    shell.double_click_at(shell.top_face_centre(2));
    assert_eq!(shell.app().edit_context_depth(), 1);
    let unique = shell.catalog().format(
        "viewport-context-unique",
        &BTreeMap::from([("count", "1".to_owned())]),
    );
    assert!(shell.has_visible_label(&shell.app().edit_context_readout()));
    assert!(shell.has_visible_label(&unique));
}

#[test]
fn make_component_is_localized_atomic_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::MakeComponent),
            shell.catalog().text("model-make-component")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeComponent));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::Group);
        assert!(shell.app().command_is_enabled(AppCommand::MakeComponent));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let grouped = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        let component_name = shell.catalog().format(
            "model-component-name",
            &BTreeMap::from([("number", "1".to_owned())]),
        );

        shell.click_menu_command("menu-model", AppCommand::MakeComponent);

        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().occurrence_count(), 1);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(3)));
        assert!(!shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-made-component",
                &BTreeMap::from([("name", component_name), ("count", "2".to_owned()),])
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeComponent));
        let converted = shell.app().canonical_digest();

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().canonical_digest(), grouped);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), converted);
        assert!(!shell.app().command_is_enabled(AppCommand::Paste));

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::MakeComponent)
        );
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-model", AppCommand::MakeComponent);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn select_all_instances_is_localized_context_bound_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::SelectAllInstances),
            shell.catalog().text("model-select-all-instances")
        );

        shell.click_at(shell.top_face_centre(1));
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::SelectAllInstances)
        );
        let single_revision = shell.app().document_revision();
        let single_digest = shell.app().canonical_digest();
        let single_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), single_revision);
        assert_eq!(shell.app().canonical_digest(), single_digest);
        assert_eq!(shell.app().undo_step_count(), single_undo_steps);

        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().occurrence_count(), 3);
        assert_ne!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            shell.app().occurrence_definition_id(OccurrenceId(3))
        );
        shell.settle();
        let definition_name = shell.catalog().format(
            "model-default-box",
            &BTreeMap::from([("number", "1".to_owned())]),
        );
        let component_heading = shell.catalog().format(
            "outliner-component",
            &BTreeMap::from([
                ("name", definition_name),
                ("count", "2".to_owned()),
                ("dimensions", "100 × 60 × 20".to_owned()),
            ]),
        );
        shell.click_row(&component_heading);
        let peer_name = shell.app().occurrence_name(OccurrenceId(2)).unwrap();
        let peer_row = shell.catalog().format(
            "outliner-instance",
            &BTreeMap::from([("visibility", "◉".to_owned()), ("name", peer_name)]),
        );
        shell.click_row(&peer_row);
        shell.settle();
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::SelectAllInstances)
        );
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);

        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(3)));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-definition",
                &BTreeMap::from([
                    (
                        "name",
                        shell.app().definition_name(DefinitionId(1)).unwrap(),
                    ),
                    ("count", "2".to_owned()),
                ])
            )
        );
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::SelectAllInstances)
        );
        let action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), action_digest);
    }
}

#[test]
fn assign_tag_is_localized_atomic_context_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(700);
        let temporary_tag = TagId(701);
        for (target, name) in [(tag, "Hardware"), (temporary_tag, "Temporary")] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        shell.settle();
        assert_eq!(
            shell.app().command_label(AppCommand::AssignTag),
            shell.catalog().text("model-assign-tag")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::AssignTag));

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::AssignTag));
        let cancel_revision = shell.app().document_revision();
        let cancel_digest = shell.app().canonical_digest();
        let cancel_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        assert!(shell.app().tag_assignment_visible());
        assert_eq!(shell.app().tag_assignment_input(), Some(BTreeSet::new()));
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-cancel"),
        );
        assert!(!shell.app().tag_assignment_visible());
        assert_eq!(shell.app().document_revision(), cancel_revision);
        assert_eq!(shell.app().canonical_digest(), cancel_digest);
        assert_eq!(shell.app().undo_step_count(), cancel_undo_steps);

        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        shell.click_command(AppCommand::AssignTag);
        assert!(shell.app().tag_assignment_visible());
        shell.click_role_and_label(Role::CheckBox, "Hardware");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );

        assert!(!shell.app().tag_assignment_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), Some(tag));
        let after = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = after.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-assigned-tag",
                &BTreeMap::from([("count", "2".to_owned()), ("tag", "Hardware".to_owned()),])
            )
        );

        let tagged_revision = shell.app().document_revision();
        let tagged_digest = shell.app().canonical_digest();
        let tagged_undo_steps = shell.app().undo_step_count();
        let tagged_action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        assert!(shell.app().tag_assignment_visible());
        assert_eq!(shell.app().document_revision(), tagged_revision);
        assert_eq!(shell.app().canonical_digest(), tagged_digest);
        assert_eq!(shell.app().undo_step_count(), tagged_undo_steps);
        assert_eq!(shell.app().action_digest(), tagged_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell.click_row(&shell.catalog().text("dialog-assign-tag-untagged"));
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        assert_eq!(first_tag(&shell, OccurrenceId(1)), None);
        assert_eq!(first_tag(&shell, OccurrenceId(2)), None);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), Some(tag));

        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_row(&shell.catalog().text("dialog-assign-tag-untagged"));
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_assignment());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), Some(tag));
        shell.settle();
        assert!(!shell.app().tag_assignment_visible());
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_role_and_label(Role::CheckBox, "Temporary");
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::DeleteTag {
                tag_id: temporary_tag.0,
            }
        ));
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_assignment());
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), Some(tag));
        shell.settle();
        assert!(!shell.app().tag_assignment_visible());

        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        let stale_revision = shell.app().document_revision();
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::RenameDefinition {
                definition_id: DefinitionId(1).0,
                name: "Intervening definition".to_owned(),
            }
        ));
        let intervening_revision = shell.app().document_revision();
        assert!(intervening_revision > stale_revision);
        shell.settle();
        assert!(!shell.app().tag_assignment_visible());
        assert_eq!(shell.app().document_revision(), intervening_revision);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), Some(tag));
    }
}

#[test]
fn tags_panel_create_is_localized_canonical_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.settle();
        let create_label = shell.catalog().text("tags-create");
        let confirm_label = shell.catalog().text("dialog-create-tag-confirm");
        let cancel_label = shell.catalog().text("dialog-create-tag-cancel");
        let name_label = shell.catalog().text("dialog-create-tag-name");
        assert!(shell.has_role_and_label(Role::Button, &create_label));

        let initial = shell.app().document_snapshot();
        let occurrence = initial.occurrence(OccurrenceId(1)).unwrap();
        let preserved = (
            occurrence.name().to_owned(),
            occurrence.definition_id(),
            occurrence.transform(),
            occurrence.parent(),
            occurrence.tags().first().copied(),
            occurrence.visible(),
        );
        let initial_revision = shell.app().document_revision();
        let initial_digest = shell.app().canonical_digest();
        let initial_undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &create_label);
        assert!(shell.app().tag_creation_visible());
        shell.click_role_and_label(Role::Button, &cancel_label);
        assert!(!shell.app().tag_creation_visible());
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps);

        shell.click_role_and_label(Role::Button, &create_label);
        assert!(!shell.app_mut().confirm_tag_creation());
        assert_eq!(shell.app().document_revision(), initial_revision);
        shell.focus_text_input(&name_label);
        shell.type_text("Hardware");
        assert_eq!(shell.app().tag_creation_input(), Some("Hardware"));
        shell.click_role_and_label(Role::Button, &confirm_label);

        assert!(!shell.app().tag_creation_visible());
        assert_eq!(shell.app().document_revision(), initial_revision + 1);
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);
        let created = shell
            .app()
            .document_snapshot()
            .tags()
            .find(|tag| tag.name() == "Hardware")
            .map(|tag| (tag.id(), tag.visible()));
        assert_eq!(created, Some((TagId(1), true)));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-created-tag",
                &BTreeMap::from([("name", "Hardware".to_owned())]),
            )
        );
        let after = shell.app().document_snapshot();
        let occurrence = after.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            ),
            preserved
        );

        let created_revision = shell.app().document_revision();
        let created_digest = shell.app().canonical_digest();
        let created_undo_steps = shell.app().undo_step_count();
        let created_action_digest = shell.app().action_digest().to_owned();
        let created_selection = shell.app().selected_occurrence_count();
        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Hardware");
        assert!(!shell.app_mut().confirm_tag_creation());
        assert_eq!(shell.app().document_revision(), created_revision);
        assert_eq!(shell.app().canonical_digest(), created_digest);
        assert_eq!(shell.app().undo_step_count(), created_undo_steps);
        assert_eq!(shell.app().action_digest(), created_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), created_selection);
        shell.click_role_and_label(Role::Button, &cancel_label);

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Namespace drift");
        assert!(create_tag(
            shell.app_mut(),
            TagId(850),
            "Intervening tag".to_owned(),
            false
        ));
        shell.settle();
        assert!(!shell.app().tag_creation_visible());
        assert!(
            shell
                .app()
                .document_snapshot()
                .tags()
                .all(|tag| tag.name() != "Namespace drift")
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(TagId(850)), None);

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Stale tag");
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::RenameDefinition {
                definition_id: DefinitionId(1).0,
                name: "Intervening definition".to_owned(),
            }
        ));
        shell.settle();
        assert!(!shell.app().tag_creation_visible());
        assert!(
            shell
                .app()
                .document_snapshot()
                .tags()
                .all(|tag| tag.name() != "Stale tag")
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert!(shell.app().document_snapshot().tags().next().is_none());
        let restored = shell.app().document_snapshot();
        let occurrence = restored.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            ),
            preserved
        );
    }
}

#[test]
fn tags_panel_create_from_selection_is_localized_atomic_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let create_label = shell.catalog().text("tags-create-from-selection");
        let confirm_label = shell
            .catalog()
            .text("dialog-create-tag-from-selection-confirm");
        let cancel_label = shell.catalog().text("dialog-create-tag-cancel");
        let name_label = shell.catalog().text("dialog-create-tag-name");
        let other_tag = TagId(1401);
        assert!(create_tag(
            shell.app_mut(),
            other_tag,
            "Other".to_owned(),
            true
        ));
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert!(set_occurrence_tag(
            shell.app_mut(),
            OccurrenceId(1),
            Some(other_tag)
        ));
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &create_label));
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_role_and_label(Role::Button, &create_label);
        assert!(!shell.app().tag_creation_visible());
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 3);

        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(Role::Button, &create_label);
        assert!(shell.app().tag_creation_visible());
        shell.focus_text_input(&name_label);
        shell.type_text("Hardware");
        shell.click_role_and_label(Role::Button, &confirm_label);

        let created_tag = TagId(1402);
        assert!(!shell.app().tag_creation_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        assert_eq!(shell.app().tag_visibility(created_tag), Some(true));
        let assigned = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = assigned.occurrence(id).unwrap();
            // The new tag is added; a part keeps the tags it already had.
            let expected_tags = preserved[index]
                .4
                .into_iter()
                .chain([created_tag])
                .collect::<BTreeSet<_>>();
            assert_eq!(occurrence.tags(), &expected_tags);
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                (
                    preserved[index].0.clone(),
                    preserved[index].1,
                    preserved[index].2,
                    preserved[index].3,
                    preserved[index].5,
                )
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-created-tag-from-selection",
                &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "3".to_owned()),]),
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(created_tag), None);
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                restored.occurrence(id).unwrap().tags().first().copied(),
                preserved[index].4
            );
        }

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Other");
        let unchanged_revision = shell.app().document_revision();
        let unchanged_digest = shell.app().canonical_digest();
        let unchanged_undo_steps = shell.app().undo_step_count();
        assert!(!shell.app_mut().confirm_tag_creation());
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        shell.click_role_and_label(Role::Button, &cancel_label);

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Selection drift");
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_creation());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        shell.settle();
        assert!(!shell.app().tag_creation_visible());
        assert!(
            shell
                .app()
                .document_snapshot()
                .tags()
                .all(|tag| tag.name() != "Selection drift")
        );
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Stale tag");
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::RenameDefinition {
                definition_id: DefinitionId(1).0,
                name: "Intervening definition".to_owned(),
            }
        ));
        shell.settle();
        assert!(!shell.app().tag_creation_visible());
        assert!(
            shell
                .app()
                .document_snapshot()
                .tags()
                .all(|tag| tag.name() != "Stale tag")
        );
    }
}

#[test]
fn tags_panel_delete_unused_is_localized_context_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let unused_tag = TagId(801);
        let used_tag = TagId(802);
        for (target, name) in [(unused_tag, "Unused"), (used_tag, "Used")] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_role_and_label(Role::CheckBox, "Used");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        shell.settle();

        let unused_delete = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Unused".to_owned())]),
        );
        let used_delete = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Used".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &unused_delete));
        assert!(shell.has_role_and_label(Role::Button, &used_delete));

        let baseline = shell.app().document_snapshot();
        let occurrence = baseline.occurrence(OccurrenceId(1)).unwrap();
        let preserved_occurrence = (
            occurrence.name().to_owned(),
            occurrence.definition_id(),
            occurrence.transform(),
            occurrence.parent(),
            occurrence.tags().first().copied(),
            occurrence.visible(),
        );
        let baseline_revision = shell.app().document_revision();
        let baseline_digest = shell.app().canonical_digest();
        let baseline_undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &unused_delete);
        assert!(shell.app().tag_deletion_visible());
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-cancel"),
        );
        assert!(!shell.app().tag_deletion_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);

        shell.click_role_and_label(Role::Button, &unused_delete);
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::RenameDefinition {
                definition_id: DefinitionId(1).0,
                name: "Intervening definition".to_owned(),
            }
        ));
        shell.settle();
        assert!(!shell.app().tag_deletion_visible());
        assert!(shell.app().document_snapshot().tag(unused_tag).is_some());
        shell.click_menu_command("menu-edit", AppCommand::Undo);

        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(Role::Button, &unused_delete);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-confirm"),
        );

        assert!(!shell.app().tag_deletion_visible());
        assert!(shell.app().document_revision() > revision);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        let deleted = shell.app().document_snapshot();
        assert!(deleted.tag(unused_tag).is_none());
        assert_eq!(deleted.tag(used_tag).unwrap().name(), "Used");
        assert!(deleted.tag(used_tag).unwrap().visible());
        let occurrence = deleted.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            ),
            preserved_occurrence
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-deleted-tag",
                &BTreeMap::from([("name", "Unused".to_owned())]),
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        let restored = shell.app().document_snapshot();
        assert_eq!(restored.tag(unused_tag).unwrap().name(), "Unused");
        assert!(restored.tag(unused_tag).unwrap().visible());
        assert_eq!(restored.tag(used_tag).unwrap().name(), "Used");
        let occurrence = restored.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            ),
            preserved_occurrence
        );
    }
}

#[test]
fn tags_panel_delete_used_exact_plan_is_localized_atomic_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(825);
        assert!(create_tag(
            shell.app_mut(),
            tag,
            "Hardware".to_owned(),
            true
        ));
        assert!(shell.app_mut().create_box());
        for target in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);

        let delete_label = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        let baseline = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = baseline.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.visible(),
            )
        });
        let baseline_revision = shell.app().document_revision();
        let baseline_digest = shell.app().canonical_digest();
        let baseline_undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &delete_label);
        assert!(shell.app().tag_deletion_visible());
        assert!(shell.has_visible_label(&shell.catalog().format(
            "dialog-delete-used-tag-message",
            &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "2".to_owned()),]),
        )));
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-cancel"),
        );
        assert!(!shell.app().tag_deletion_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);

        shell.click_role_and_label(Role::Button, &delete_label);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-confirm"),
        );

        assert!(!shell.app().tag_deletion_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision + 1);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        let deleted = shell.app().document_snapshot();
        assert!(deleted.tag(tag).is_none());
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = deleted.occurrence(id).unwrap();
            assert_eq!(occurrence.tags().first().copied(), None);
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-deleted-used-tag",
                &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "2".to_owned()),]),
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        let restored = shell.app().document_snapshot();
        assert_eq!(restored.tag(tag).unwrap().name(), "Hardware");
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert_eq!(
                restored.occurrence(id).unwrap().tags().first().copied(),
                Some(tag)
            );
        }
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-edit", AppCommand::Redo);
        let redone = shell.app().document_snapshot();
        assert!(redone.tag(tag).is_none());
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert_eq!(redone.occurrence(id).unwrap().tags().first().copied(), None);
        }
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert!(shell.app().document_snapshot().tag(tag).is_some());

        shell.click_role_and_label(Role::Button, &delete_label);
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::RenameDefinition {
                definition_id: DefinitionId(1).0,
                name: "Intervening definition".to_owned(),
            }
        ));
        shell.settle();
        assert!(!shell.app().tag_deletion_visible());
        let stale = shell.app().document_snapshot();
        assert!(stale.tag(tag).is_some());
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert_eq!(
                stale.occurrence(id).unwrap().tags().first().copied(),
                Some(tag)
            );
        }
    }
}

#[test]
fn tags_panel_deletes_a_layer_held_only_inside_a_component_in_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let local_tag = TagId(826);
        let missing_tag = TagId(827);
        for (target, name) in [(local_tag, "Local"), (missing_tag, "Missing")] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        assert!(shell.app_mut().create_box());
        assert!(set_occurrence_tag(
            shell.app_mut(),
            OccurrenceId(1),
            Some(local_tag)
        ));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(shell.app_mut().group_selected());
        assert!(shell.app_mut().make_component());
        shell.settle();

        let local_delete = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Local".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &local_delete));
        let holds_local_tag = |snapshot: &ketchup_model::document::Snapshot| {
            snapshot
                .local_occurrences()
                .any(|occurrence| occurrence.tags().contains(&local_tag))
        };
        assert!(holds_local_tag(&shell.app().document_snapshot()));
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(Role::Button, &local_delete);
        assert!(shell.app().tag_deletion_visible());
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-confirm"),
        );
        assert!(!shell.app().tag_deletion_visible());
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        let deleted = shell.app().document_snapshot();
        assert!(deleted.tag(local_tag).is_none());
        assert!(!holds_local_tag(&deleted));
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        let restored = shell.app().document_snapshot();
        assert!(restored.tag(local_tag).is_some());
        assert!(holds_local_tag(&restored));
        assert_eq!(shell.app().canonical_digest(), digest);

        let missing_delete = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Missing".to_owned())]),
        );
        shell.click_role_and_label(Role::Button, &missing_delete);
        assert!(shell.app().tag_deletion_visible());
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::DeleteTag {
                tag_id: missing_tag.0,
            }
        ));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let action_digest = shell.app().action_digest().to_owned();
        let selected_occurrences = shell.app().selected_occurrence_count();
        assert!(!shell.app_mut().confirm_tag_deletion());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), action_digest);
        assert_eq!(
            shell.app().selected_occurrence_count(),
            selected_occurrences
        );
        shell.settle();
        assert!(!shell.app().tag_deletion_visible());
    }
}

#[test]
fn tags_panel_clear_assignments_is_localized_atomic_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let used_tag = TagId(850);
        let unused_tag = TagId(851);
        for (target, name) in [(used_tag, "Hardware"), (unused_tag, "Unused")] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        for target in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(used_tag)));
        }
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::SetOccurrenceVisibility {
                occurrence_id: OccurrenceId(2).0,
                visible: false,
            }
        ));
        shell.settle();

        let clear_used = shell.catalog().format(
            "tags-clear",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        let clear_unused = shell.catalog().format(
            "tags-clear",
            &BTreeMap::from([("name", "Unused".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &clear_used));
        assert!(shell.has_role_and_label(Role::Button, &clear_unused));

        let unused_revision = shell.app().document_revision();
        let unused_digest = shell.app().canonical_digest();
        let unused_undo_steps = shell.app().undo_step_count();
        let unused_action_digest = shell.app().action_digest().to_owned();
        shell.click_role_and_label(Role::Button, &clear_unused);
        assert!(!shell.app().tag_clear_visible());
        assert_eq!(shell.app().document_revision(), unused_revision);
        assert_eq!(shell.app().canonical_digest(), unused_digest);
        assert_eq!(shell.app().undo_step_count(), unused_undo_steps);
        assert_eq!(shell.app().action_digest(), unused_action_digest);

        shell.click_role_and_label(Role::Button, &clear_used);
        assert!(shell.app().tag_clear_visible());
        let cancel_revision = shell.app().document_revision();
        let cancel_digest = shell.app().canonical_digest();
        let cancel_undo_steps = shell.app().undo_step_count();
        let cancel_action_digest = shell.app().action_digest().to_owned();
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-clear-tag-cancel"),
        );
        assert!(!shell.app().tag_clear_visible());
        assert_eq!(shell.app().document_revision(), cancel_revision);
        assert_eq!(shell.app().canonical_digest(), cancel_digest);
        assert_eq!(shell.app().undo_step_count(), cancel_undo_steps);
        assert_eq!(shell.app().action_digest(), cancel_action_digest);

        shell.click_role_and_label(Role::Button, &clear_used);
        assert!(create_tag(
            shell.app_mut(),
            TagId(854),
            "Namespace drift".to_owned(),
            false
        ));
        let namespace_revision = shell.app().document_revision();
        let namespace_digest = shell.app().canonical_digest();
        let namespace_undo_steps = shell.app().undo_step_count();
        let namespace_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_clear());
        assert_eq!(shell.app().document_revision(), namespace_revision);
        assert_eq!(shell.app().canonical_digest(), namespace_digest);
        assert_eq!(shell.app().undo_step_count(), namespace_undo_steps);
        assert_eq!(shell.app().action_digest(), namespace_action_digest);
        shell.settle();
        assert!(!shell.app().tag_clear_visible());
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.settle();

        shell.click_role_and_label(Role::Button, &clear_used);
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::RenameDefinition {
                definition_id: DefinitionId(1).0,
                name: "Intervening definition".to_owned(),
            }
        ));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_clear());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.settle();
        assert!(!shell.app().tag_clear_visible());
        shell.click_menu_command("menu-edit", AppCommand::Undo);

        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(Role::Button, &clear_used);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-clear-tag-confirm"),
        );

        assert!(!shell.app().tag_clear_visible());
        assert!(shell.app().document_revision() > revision);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        let cleared = shell.app().document_snapshot();
        let tag = cleared.tag(used_tag).unwrap();
        assert_eq!(tag.name(), "Hardware");
        assert!(tag.visible());
        assert!(cleared.tag(unused_tag).is_some());
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = cleared.occurrence(id).unwrap();
            assert_eq!(occurrence.tags().first().copied(), None);
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-cleared-tag",
                &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "2".to_owned()),])
            )
        );
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &clear_used));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(used_tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), Some(used_tag));
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .tag(used_tag)
                .unwrap()
                .name(),
            "Hardware"
        );
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), None);
        assert_eq!(first_tag(&shell, OccurrenceId(2)), None);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(used_tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), Some(used_tag));

        let missing_tag = TagId(852);
        assert!(create_tag(
            shell.app_mut(),
            missing_tag,
            "Missing".to_owned(),
            true
        ));
        assert!(set_occurrence_tag(
            shell.app_mut(),
            OccurrenceId(1),
            Some(missing_tag)
        ));
        shell.settle();
        let clear_missing = shell.catalog().format(
            "tags-clear",
            &BTreeMap::from([("name", "Missing".to_owned())]),
        );
        shell.click_role_and_label(Role::Button, &clear_missing);
        assert!(shell.app().tag_clear_visible());
        assert!(set_occurrence_tag(shell.app_mut(), OccurrenceId(1), None));
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::DeleteTag {
                tag_id: missing_tag.0,
            }
        ));
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_clear());
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
        shell.settle();
        assert!(!shell.app().tag_clear_visible());
    }
}

#[test]
fn tags_panel_rename_is_localized_canonical_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(901);
        let other_tag = TagId(902);
        for (target, name) in [(tag, "Hardware"), (other_tag, "Other")] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_role_and_label(Role::CheckBox, "Hardware");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        shell.settle();

        let rename_label = shell.catalog().format(
            "tags-rename",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        let name_label = shell.catalog().text("dialog-rename-tag-name");
        let confirm_label = shell.catalog().text("dialog-rename-tag-confirm");
        let cancel_label = shell.catalog().text("dialog-rename-tag-cancel");
        assert!(shell.has_role_and_label(Role::Button, &rename_label));

        let baseline = shell.app().document_snapshot();
        let occurrence = baseline.occurrence(OccurrenceId(1)).unwrap();
        let preserved_occurrence = (
            occurrence.name().to_owned(),
            occurrence.definition_id(),
            occurrence.transform(),
            occurrence.parent(),
            occurrence.tags().first().copied(),
            occurrence.visible(),
        );
        let baseline_revision = shell.app().document_revision();
        let baseline_digest = shell.app().canonical_digest();
        let baseline_undo_steps = shell.app().undo_step_count();
        let baseline_selection = shell.app().selected_occurrence_count();

        shell.click_role_and_label(Role::Button, &rename_label);
        assert!(shell.app().tag_rename_visible());
        assert_eq!(shell.app().tag_rename_input(), Some("Hardware"));
        shell.click_role_and_label(Role::Button, &cancel_label);
        assert!(!shell.app().tag_rename_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
        let invalid_action_digest = shell.app().action_digest().to_owned();

        shell.click_role_and_label(Role::Button, &rename_label);
        assert!(!shell.app_mut().confirm_tag_rename());
        shell.focus_text_input(&name_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Other");
        assert!(!shell.app_mut().confirm_tag_rename());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
        assert_eq!(shell.app().action_digest(), invalid_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), baseline_selection);
        shell.focus_text_input(&name_label);
        shell.key(Key::A, ctrl());
        shell.type_text("  Mechanical  ");
        shell.click_role_and_label(Role::Button, &confirm_label);

        assert!(!shell.app().tag_rename_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision + 1);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps + 1);
        let renamed = shell.app().document_snapshot();
        let renamed_tag = renamed.tag(tag).unwrap();
        assert_eq!(renamed_tag.name(), "Mechanical");
        assert!(renamed_tag.visible());
        assert_eq!(renamed.tag(other_tag).unwrap().name(), "Other");
        let occurrence = renamed.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            ),
            preserved_occurrence
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-renamed-tag",
                &BTreeMap::from([
                    ("old_name", "Hardware".to_owned()),
                    ("name", "Mechanical".to_owned()),
                ]),
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        let restored = shell.app().document_snapshot();
        assert_eq!(restored.tag(tag).unwrap().name(), "Hardware");
        assert!(restored.tag(tag).unwrap().visible());
        assert_eq!(
            restored
                .occurrence(OccurrenceId(1))
                .unwrap()
                .tags()
                .first()
                .copied(),
            Some(tag)
        );
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        let redone = shell.app().document_snapshot();
        assert_eq!(redone.tag(tag).unwrap().name(), "Mechanical");
        assert!(redone.tag(tag).unwrap().visible());
        assert_eq!(
            redone
                .occurrence(OccurrenceId(1))
                .unwrap()
                .tags()
                .first()
                .copied(),
            Some(tag)
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell.app().document_snapshot().tag(tag).unwrap().name(),
            "Hardware"
        );

        let other_rename_label = shell.catalog().format(
            "tags-rename",
            &BTreeMap::from([("name", "Other".to_owned())]),
        );
        shell.click_role_and_label(Role::Button, &other_rename_label);
        assert!(shell.app().tag_rename_visible());
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::DeleteTag {
                tag_id: other_tag.0
            }
        ));
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        let missing_selection = shell.app().selected_occurrence_count();
        assert!(!shell.app_mut().confirm_tag_rename());
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), missing_selection);
        assert!(shell.app().document_snapshot().tag(other_tag).is_none());
        shell.settle();
        assert!(!shell.app().tag_rename_visible());
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .tag(other_tag)
                .unwrap()
                .name(),
            "Other"
        );

        shell.click_role_and_label(Role::Button, &rename_label);
        shell.focus_text_input(&name_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Stale name");
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::RenameDefinition {
                definition_id: DefinitionId(1).0,
                name: "Intervening definition".to_owned(),
            }
        ));
        shell.settle();
        assert!(!shell.app().tag_rename_visible());
        assert_eq!(
            shell.app().document_snapshot().tag(tag).unwrap().name(),
            "Hardware"
        );
    }
}

#[test]
fn tags_panel_visibility_toggle_is_localized_canonical_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(701);
        assert!(create_tag(
            shell.app_mut(),
            tag,
            "Hardware".to_owned(),
            true
        ));
        hold_exact_bodies_back(&mut shell);
        shell.settle();

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        // The exact body arrives while the dialog is open; it must stay open.
        shell
            .app_mut()
            .connect_exact_worker(exact_worker_path())
            .unwrap();
        wait_for_one_exact_body(&mut shell);
        shell.click_role_and_label(Role::CheckBox, "Hardware");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        shell.settle();
        let row = shell.catalog().format(
            "tags-row",
            &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "1".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::CheckBox, &row));
        assert_eq!(shell.app().tag_visibility(tag), Some(true));

        let before = shell.app().document_snapshot();
        let occurrence = before.occurrence(OccurrenceId(1)).unwrap();
        let preserved = (
            occurrence.definition_id(),
            occurrence.transform(),
            occurrence.parent(),
            occurrence.tags().first().copied(),
            occurrence.visible(),
        );
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::CheckBox, &row);

        assert_eq!(shell.app().tag_visibility(tag), Some(false));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        let hidden = shell.app().document_snapshot();
        let hidden_occurrence = hidden.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                hidden_occurrence.definition_id(),
                hidden_occurrence.transform(),
                hidden_occurrence.parent(),
                hidden_occurrence.tags().first().copied(),
                hidden_occurrence.visible(),
            ),
            preserved
        );
        assert_eq!(hidden.tag(tag).unwrap().name(), "Hardware");
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-tag-visibility",
                &BTreeMap::from([
                    ("name", "Hardware".to_owned()),
                    ("visibility", shell.catalog().text("visibility-hidden"),),
                ])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(tag), Some(true));
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .tags()
                .first()
                .copied(),
            Some(tag)
        );

        let unchanged_revision = shell.app().document_revision();
        let unchanged_digest = shell.app().canonical_digest();
        let unchanged_undo_steps = shell.app().undo_step_count();
        assert!(!shell.app_mut().set_tag_visibility(tag, true));
        assert!(!shell.app_mut().set_tag_visibility(TagId(999), false));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
    }
}

#[test]
fn a_part_in_two_tags_is_hidden_while_either_tag_is_hidden() {
    let mut shell = Shell::with_catalog(LocaleCatalog::english());
    let (roof, concept) = (TagId(711), TagId(712));
    for (target, name) in [(roof, "Roof"), (concept, "Concept")] {
        assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
    }
    shell.settle();

    let part_on_screen = shell.top_face_centre(1);
    shell.click_at(part_on_screen);
    let undo_steps = shell.app().undo_step_count();
    shell.click_menu_command("menu-model", AppCommand::AssignTag);
    shell.click_role_and_label(Role::CheckBox, "Roof");
    shell.click_role_and_label(Role::CheckBox, "Concept");
    shell.click_role_and_label(
        Role::Button,
        &shell.catalog().text("dialog-assign-tag-confirm"),
    );
    assert_eq!(
        shell.app().occurrence_tags(OccurrenceId(1)),
        BTreeSet::from([roof, concept])
    );
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
    shell.settle();
    let row = |name: &str| {
        shell.catalog().format(
            "tags-row",
            &BTreeMap::from([("name", name.to_owned()), ("count", "1".to_owned())]),
        )
    };
    let (roof_row, concept_row) = (row("Roof"), row("Concept"));
    let shown = |shell: &Shell| {
        shell
            .app()
            .document_snapshot()
            .occurrence_effectively_visible(OccurrenceId(1))
    };
    assert_eq!(shown(&shell), Some(true));

    for hidden_row in [&roof_row, &concept_row] {
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(Role::CheckBox, hidden_row);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shown(&shell), Some(false));
        // A hidden part cannot be picked in the viewport.
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(part_on_screen);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        shell.click_role_and_label(Role::CheckBox, hidden_row);
        assert_eq!(shown(&shell), Some(true));
    }

    shell.click_role_and_label(Role::CheckBox, &roof_row);
    shell.click_role_and_label(Role::CheckBox, &concept_row);
    assert_eq!(shown(&shell), Some(false));
    shell.click_role_and_label(Role::CheckBox, &concept_row);
    assert_eq!(shown(&shell), Some(false), "Roof alone still hides it");
    for _ in 0..3 {
        shell.click_menu_command("menu-edit", AppCommand::Undo);
    }
    assert_eq!(shown(&shell), Some(true));
}

#[test]
fn saved_views_panel_saves_shows_renames_and_deletes_a_view() {
    use ketchup_app::ViewFlag;
    let mut shell = Shell::with_catalog(LocaleCatalog::english());
    let roof = TagId(721);
    assert!(create_tag(shell.app_mut(), roof, "Roof".to_owned(), true));
    shell.settle();
    let catalog = shell.catalog().clone();
    let name_label = catalog.text("saved-views-name");
    let save_label = catalog.text("saved-views-save");
    let named =
        |key: &str, name: &str| catalog.format(key, &BTreeMap::from([("name", name.to_owned())]));

    shell.focus_text_input(&name_label);
    shell.type_text("Concept");
    shell.click_role_and_label(Role::Button, &save_label);
    assert!(shell.app_mut().set_tag_visibility(roof, false));
    shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Xray));
    shell.focus_text_input(&name_label);
    shell.type_text("No roof");
    let undo_steps = shell.app().undo_step_count();
    shell.click_role_and_label(Role::Button, &save_label);
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
    let views = |shell: &Shell| {
        shell
            .app()
            .saved_views()
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
    };
    assert_eq!(views(&shell), ["Concept", "No roof"]);

    shell.click_role_and_label(Role::Button, &named("saved-views-activate", "Concept"));
    assert_eq!(shell.app().tag_visibility(roof), Some(true));
    assert!(!shell.app().view_visible(ViewFlag::Xray));
    shell.click_role_and_label(Role::Button, &named("saved-views-activate", "No roof"));
    assert_eq!(shell.app().tag_visibility(roof), Some(false));
    assert!(shell.app().view_visible(ViewFlag::Xray));

    shell.focus_text_input(&name_label);
    shell.type_text("Without roof");
    shell.click_role_and_label(Role::Button, &named("saved-views-rename", "No roof"));
    assert_eq!(views(&shell), ["Concept", "Without roof"]);
    shell.click_role_and_label(Role::Button, &named("saved-views-delete", "Concept"));
    assert_eq!(views(&shell), ["Without roof"]);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(views(&shell), ["Concept", "Without roof"]);
}

#[test]
fn section_controls_cut_turn_and_flip_and_a_saved_view_keeps_the_cut() {
    let mut shell = Shell::with_catalog(LocaleCatalog::english());
    shell.settle();
    let catalog = shell.catalog().clone();
    let undo_steps = shell.app().undo_step_count();

    shell.click_role_and_label(Role::CheckBox, &catalog.text("section-open"));
    let horizontal = shell.app().section().expect("the checkbox opens a cut");
    assert_eq!(horizontal.normal, [0.0, 0.0, 1.0]);

    let across_x = catalog.format("section-axis", &BTreeMap::from([("axis", "X".to_owned())]));
    shell.click_role_and_label(Role::Button, &across_x);
    assert_eq!(shell.app().section().unwrap().normal, [1.0, 0.0, 0.0]);
    shell.click_role_and_label(Role::Button, &catalog.text("section-flip"));
    assert_eq!(shell.app().section().unwrap().normal, [-1.0, 0.0, 0.0]);
    assert_eq!(
        shell.app().undo_step_count(),
        undo_steps,
        "a cut changes only the viewport"
    );

    shell.focus_text_input(&catalog.text("saved-views-name"));
    shell.type_text("Cut");
    shell.click_role_and_label(Role::Button, &catalog.text("saved-views-save"));
    let cut = shell.app().section();
    shell.click_role_and_label(Role::CheckBox, &catalog.text("section-open"));
    assert_eq!(shell.app().section(), None);
    shell.click_role_and_label(
        Role::Button,
        &catalog.format(
            "saved-views-activate",
            &BTreeMap::from([("name", "Cut".to_owned())]),
        ),
    );
    assert_eq!(shell.app().section(), cut);
}

#[test]
fn tags_panel_bulk_visibility_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.settle();
        let show_all_label = shell.catalog().text("tags-show-all");
        let hide_all_label = shell.catalog().text("tags-hide-all");
        assert!(shell.has_role_and_label(Role::Button, &show_all_label));
        assert!(shell.has_role_and_label(Role::Button, &hide_all_label));
        shell.settle();
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().set_all_tag_visibility(true));
        assert!(!shell.app_mut().set_all_tag_visibility(false));
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        let visible_tag = TagId(801);
        let hidden_tag = TagId(802);
        for (target, name, visible) in [
            (visible_tag, "Hardware", true),
            (hidden_tag, "Hidden", false),
        ] {
            assert!(create_tag(
                shell.app_mut(),
                target,
                name.to_owned(),
                visible
            ));
        }
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), visible_tag),
            (OccurrenceId(2), hidden_tag),
        ] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        shell.settle();

        assert!(shell.has_role_and_label(Role::Button, &show_all_label));
        assert!(shell.has_role_and_label(Role::Button, &hide_all_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &show_all_label);

        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        let shown = shell.app().document_snapshot();
        assert_eq!(shown.tag(visible_tag).unwrap().name(), "Hardware");
        assert_eq!(shown.tag(hidden_tag).unwrap().name(), "Hidden");
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = shown.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-all-tags-visibility",
                &BTreeMap::from([
                    ("count", "1".to_owned()),
                    ("visibility", shell.catalog().text("visibility-shown")),
                ])
            )
        );
        let shown_revision = shell.app().document_revision();
        let shown_digest = shell.app().canonical_digest();
        let shown_undo_steps = shell.app().undo_step_count();
        let shown_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().set_all_tag_visibility(true));
        assert_eq!(shell.app().document_revision(), shown_revision);
        assert_eq!(shell.app().canonical_digest(), shown_digest);
        assert_eq!(shell.app().undo_step_count(), shown_undo_steps);
        assert_eq!(shell.app().action_digest(), shown_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_all_label));
        assert!(shell.has_role_and_label(Role::Button, &hide_all_label));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        shell.click_role_and_label(Role::Button, &hide_all_label);

        assert_eq!(shell.app().tag_visibility(visible_tag), Some(false));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-all-tags-visibility",
                &BTreeMap::from([
                    ("count", "1".to_owned()),
                    ("visibility", shell.catalog().text("visibility-hidden")),
                ])
            )
        );
        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();
        let hidden_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().set_all_tag_visibility(false));
        assert_eq!(shell.app().document_revision(), hidden_revision);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps);
        assert_eq!(shell.app().action_digest(), hidden_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_all_label));
        assert!(shell.has_role_and_label(Role::Button, &hide_all_label));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = restored.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
    }
}

#[test]
fn tags_panel_invert_visibility_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().invert_tag_visibility());
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
        shell.settle();
        let empty_invert_label = shell.catalog().text("tags-invert-visibility");
        assert!(shell.has_role_and_label(Role::Button, &empty_invert_label));
        shell.settle();

        let visible_tag = TagId(851);
        let hidden_tag = TagId(852);
        for (target, name, visible) in [
            (visible_tag, "Hardware", true),
            (hidden_tag, "Hidden", false),
        ] {
            assert!(create_tag(
                shell.app_mut(),
                target,
                name.to_owned(),
                visible
            ));
        }
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), visible_tag),
            (OccurrenceId(2), hidden_tag),
        ] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        shell.click_at(shell.top_face_centre(1));
        shell.settle();

        let invert_label = shell.catalog().text("tags-invert-visibility");
        assert!(shell.has_role_and_label(Role::Button, &invert_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &invert_label);

        assert_eq!(shell.app().tag_visibility(visible_tag), Some(false));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        let inverted = shell.app().document_snapshot();
        assert_eq!(inverted.tag(visible_tag).unwrap().name(), "Hardware");
        assert_eq!(inverted.tag(hidden_tag).unwrap().name(), "Hidden");
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = inverted.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-inverted-tags-visibility",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = restored.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
    }
}

#[test]
fn tags_panel_isolate_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let target_tag = TagId(901);
        let visible_tag = TagId(902);
        let hidden_tag = TagId(903);
        for (target, name, visible) in [
            (target_tag, "Hardware", false),
            (visible_tag, "Visible", true),
            (hidden_tag, "Hidden", false),
        ] {
            assert!(create_tag(
                shell.app_mut(),
                target,
                name.to_owned(),
                visible
            ));
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), target_tag),
            (OccurrenceId(2), visible_tag),
            (OccurrenceId(3), hidden_tag),
        ] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        shell.click_at(shell.top_face_centre(2));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        shell.settle();

        let isolate_label = shell.catalog().format(
            "tags-isolate",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &isolate_label);

        assert_eq!(shell.app().tag_visibility(target_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(false));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let isolated = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = isolated.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-isolated-tag",
                &BTreeMap::from([("count", "2".to_owned()), ("name", "Hardware".to_owned()),])
            )
        );

        let isolated_revision = shell.app().document_revision();
        let isolated_digest = shell.app().canonical_digest();
        let isolated_undo_steps = shell.app().undo_step_count();
        let isolated_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().isolate_tag(target_tag));
        assert!(!shell.app_mut().isolate_tag(TagId(9999)));
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        assert_eq!(shell.app().document_revision(), isolated_revision);
        assert_eq!(shell.app().canonical_digest(), isolated_digest);
        assert_eq!(shell.app().undo_step_count(), isolated_undo_steps);
        assert_eq!(shell.app().action_digest(), isolated_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(target_tag), Some(false));
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = restored.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
    }
}

#[test]
fn tags_panel_isolate_selection_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let selected_a = TagId(951);
        let selected_b = TagId(952);
        let other = TagId(953);
        for (target, name) in [
            (selected_a, "Hardware"),
            (selected_b, "Fasteners"),
            (other, "Other"),
        ] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), selected_a),
            (OccurrenceId(2), selected_b),
            (OccurrenceId(3), other),
        ] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app_mut().set_tag_visibility(selected_b, false));
        shell.settle();

        let isolate_label = shell.catalog().text("tags-isolate-selection");
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &isolate_label);

        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(true));
        assert_eq!(shell.app().tag_visibility(other), Some(false));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let isolated = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = isolated.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-isolated-selected-tags",
                &BTreeMap::from([("count", "2".to_owned()), ("tags", "2".to_owned()),])
            )
        );

        let isolated_revision = shell.app().document_revision();
        let isolated_digest = shell.app().canonical_digest();
        let isolated_undo_steps = shell.app().undo_step_count();
        let isolated_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().isolate_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        assert_eq!(shell.app().document_revision(), isolated_revision);
        assert_eq!(shell.app().canonical_digest(), isolated_digest);
        assert_eq!(shell.app().undo_step_count(), isolated_undo_steps);
        assert_eq!(shell.app().action_digest(), isolated_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = restored.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().isolate_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        assert!(shell.app_mut().create_box());
        let untagged_revision = shell.app().document_revision();
        let untagged_digest = shell.app().canonical_digest();
        let untagged_undo_steps = shell.app().undo_step_count();
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().isolate_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        assert_eq!(shell.app().document_revision(), untagged_revision);
        assert_eq!(shell.app().canonical_digest(), untagged_digest);
        assert_eq!(shell.app().undo_step_count(), untagged_undo_steps);
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
    }
}

#[test]
fn tags_panel_hide_selection_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let selected_a = TagId(961);
        let selected_b = TagId(962);
        let other = TagId(963);
        for (target, name) in [
            (selected_a, "Hardware"),
            (selected_b, "Fasteners"),
            (other, "Other"),
        ] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), selected_a),
            (OccurrenceId(2), selected_b),
            (OccurrenceId(3), other),
        ] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell.settle();

        let hide_label = shell.catalog().text("tags-hide-selection");
        assert!(shell.has_role_and_label(Role::Button, &hide_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &hide_label);

        assert_eq!(shell.app().tag_visibility(selected_a), Some(false));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let hidden = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = hidden.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-hidden-selected-tags",
                &BTreeMap::from([("count", "2".to_owned()), ("tags", "2".to_owned()),])
            )
        );

        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();
        let hidden_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().hide_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &hide_label));
        assert_eq!(shell.app().document_revision(), hidden_revision);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps);
        assert_eq!(shell.app().action_digest(), hidden_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(true));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &hide_label));
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().hide_selected_tags());
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        assert!(shell.app_mut().create_box());
        let untagged_revision = shell.app().document_revision();
        let untagged_digest = shell.app().canonical_digest();
        let untagged_undo_steps = shell.app().undo_step_count();
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().hide_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &hide_label));
        assert_eq!(shell.app().document_revision(), untagged_revision);
        assert_eq!(shell.app().canonical_digest(), untagged_digest);
        assert_eq!(shell.app().undo_step_count(), untagged_undo_steps);
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
    }
}

#[test]
fn tags_panel_show_selection_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let selected_a = TagId(964);
        let selected_b = TagId(965);
        let other = TagId(966);
        for (target, name) in [
            (selected_a, "Hardware"),
            (selected_b, "Fasteners"),
            (other, "Other"),
        ] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), selected_a),
            (OccurrenceId(2), selected_b),
            (OccurrenceId(3), other),
        ] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell.settle();

        let hide_label = shell.catalog().text("tags-hide-selection");
        shell.click_role_and_label(Role::Button, &hide_label);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(false));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));

        let show_label = shell.catalog().text("tags-show-selection");
        assert!(shell.has_role_and_label(Role::Button, &show_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tags().first().copied(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &show_label);

        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(true));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let shown = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = shown.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tags().first().copied(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-shown-selected-tags",
                &BTreeMap::from([("count", "2".to_owned()), ("tags", "2".to_owned()),])
            )
        );

        let shown_revision = shell.app().document_revision();
        let shown_digest = shell.app().canonical_digest();
        let shown_undo_steps = shell.app().undo_step_count();
        let shown_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().show_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_label));
        assert_eq!(shell.app().document_revision(), shown_revision);
        assert_eq!(shell.app().canonical_digest(), shown_digest);
        assert_eq!(shell.app().undo_step_count(), shown_undo_steps);
        assert_eq!(shell.app().action_digest(), shown_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(false));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_label));
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().show_selected_tags());
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        assert!(shell.app_mut().create_box());
        let untagged_revision = shell.app().document_revision();
        let untagged_digest = shell.app().canonical_digest();
        let untagged_undo_steps = shell.app().undo_step_count();
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().show_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_label));
        assert_eq!(shell.app().document_revision(), untagged_revision);
        assert_eq!(shell.app().canonical_digest(), untagged_digest);
        assert_eq!(shell.app().undo_step_count(), untagged_undo_steps);
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
    }
}
