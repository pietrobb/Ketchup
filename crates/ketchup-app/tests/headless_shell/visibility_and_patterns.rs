use super::*;

#[test]
fn tags_panel_invert_selection_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let selected_a = TagId(967);
        let selected_b = TagId(968);
        let other = TagId(969);
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

        let invert_label = shell.catalog().text("tags-invert-selection");
        assert!(shell.has_role_and_label(Role::Button, &invert_label));
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

        shell.click_role_and_label(Role::Button, &invert_label);

        assert_eq!(shell.app().tag_visibility(selected_a), Some(false));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(true));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let inverted = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
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
                "digest-inverted-selected-tags",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &invert_label));
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().invert_selected_tags());
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        assert!(shell.app_mut().create_box());
        let untagged_revision = shell.app().document_revision();
        let untagged_digest = shell.app().canonical_digest();
        let untagged_undo_steps = shell.app().undo_step_count();
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().invert_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &invert_label));
        assert_eq!(shell.app().document_revision(), untagged_revision);
        assert_eq!(shell.app().canonical_digest(), untagged_digest);
        assert_eq!(shell.app().undo_step_count(), untagged_undo_steps);
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
    }
}

#[test]
fn tags_panel_select_matching_is_localized_ephemeral_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let hardware = TagId(971);
        let fasteners = TagId(972);
        for (target, name) in [(hardware, "Hardware"), (fasteners, "Fasteners")] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        for _ in 0..5 {
            assert!(shell.app_mut().create_box());
        }
        assert_eq!(shell.app().occurrence_count(), 6);
        for (target, tag) in [
            (OccurrenceId(1), hardware),
            (OccurrenceId(2), hardware),
            (OccurrenceId(3), fasteners),
            (OccurrenceId(4), fasteners),
            (OccurrenceId(5), fasteners),
        ] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::SetOccurrenceVisibility {
                occurrence_id: OccurrenceId(4).0,
                visible: false,
            }
        ));
        assert!(shell.app_mut().select_tag_occurrences(hardware));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(set_occurrence_tag(
            shell.app_mut(),
            OccurrenceId(2),
            Some(fasteners)
        ));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell.settle();

        let select_label = shell.catalog().text("tags-select-matching");
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 4);
        for id in [
            OccurrenceId(1),
            OccurrenceId(2),
            OccurrenceId(3),
            OccurrenceId(5),
        ] {
            assert!(shell.app().occurrence_is_selected(id));
        }
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(4)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(6)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-matching-tags",
                &BTreeMap::from([("count", "4".to_owned()), ("tags", "2".to_owned()),])
            )
        );

        let selected_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_matching_tags());
        assert_eq!(shell.app().selected_occurrence_count(), 4);
        assert_eq!(shell.app().action_digest(), selected_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        assert!(shell.app_mut().set_all_tag_visibility(false));
        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();
        let hidden_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_matching_tags());
        assert_eq!(shell.app().selected_occurrence_count(), 4);
        assert_eq!(shell.app().document_revision(), hidden_revision);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps);
        assert_eq!(shell.app().action_digest(), hidden_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(hardware), Some(true));
        assert_eq!(shell.app().tag_visibility(fasteners), Some(true));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_matching_tags());
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        assert!(shell.app_mut().select_untagged_occurrences());
        assert!(shell.app().occurrence_is_selected(OccurrenceId(6)));
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_matching_tags());
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(6)));
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));
    }
}

#[test]
fn tags_panel_select_all_tagged_is_localized_ephemeral_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let hardware = TagId(981);
        let fasteners = TagId(982);
        let hidden = TagId(983);
        for (target, name, visible) in [
            (hardware, "Hardware", true),
            (fasteners, "Fasteners", true),
            (hidden, "Hidden", false),
        ] {
            assert!(create_tag(
                shell.app_mut(),
                target,
                name.to_owned(),
                visible
            ));
        }
        for _ in 0..5 {
            assert!(shell.app_mut().create_box());
        }
        assert_eq!(shell.app().occurrence_count(), 6);
        for (target, tag) in [
            (OccurrenceId(1), hardware),
            (OccurrenceId(2), hardware),
            (OccurrenceId(3), fasteners),
            (OccurrenceId(4), fasteners),
            (OccurrenceId(5), hidden),
        ] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::SetOccurrenceVisibility {
                occurrence_id: OccurrenceId(4).0,
                visible: false,
            }
        ));
        shell.settle();

        let select_label = shell.catalog().text("tags-select-all-tagged");
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 3);
        for id in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)] {
            assert!(shell.app().occurrence_is_selected(id));
        }
        for id in [OccurrenceId(4), OccurrenceId(5), OccurrenceId(6)] {
            assert!(!shell.app().occurrence_is_selected(id));
        }
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-all-tagged",
                &BTreeMap::from([("count", "3".to_owned())])
            )
        );

        let selected_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_all_tagged_occurrences());
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), selected_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        assert!(shell.app_mut().set_tag_visibility(hardware, false));
        assert!(shell.app_mut().set_tag_visibility(fasteners, false));
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_all_tagged_occurrences());
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));
    }
}

#[test]
fn tags_panel_select_tagged_is_localized_ephemeral_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(1001);
        let empty_tag = TagId(1002);
        for (target, name) in [(tag, "Hardware"), (empty_tag, "Empty")] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().occurrence_count(), 3);
        for target in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(set_occurrence_tag(shell.app_mut(), target, Some(tag)));
        }
        shell.settle();

        let select_label = shell.catalog().format(
            "tags-select",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        // The pasted box hides the new one's top face beside the dock.
        assert!(shell.app_mut().headless_select_occurrence(OccurrenceId(3)));
        shell.settle();
        assert!(shell.app().occurrence_is_selected(OccurrenceId(3)));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(3)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-tag",
                &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "2".to_owned()),])
            )
        );

        let selection_digest = shell.app().action_digest().to_owned();
        shell.click_role_and_label(Role::Button, &select_label);
        assert!(!shell.app_mut().select_tag_occurrences(tag));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().action_digest(), selection_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        assert!(!shell.app_mut().select_tag_occurrences(empty_tag));
        assert!(!shell.app_mut().select_tag_occurrences(TagId(9999)));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().action_digest(), selection_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        assert!(shell.app_mut().set_tag_visibility(tag, false));
        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();
        let hidden_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_tag_occurrences(tag));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(shell.app().document_revision(), hidden_revision);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps);
        assert_eq!(shell.app().action_digest(), hidden_action_digest);
    }
}

#[test]
fn tags_panel_select_untagged_is_localized_ephemeral_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(1101);
        assert!(create_tag(
            shell.app_mut(),
            tag,
            "Hardware".to_owned(),
            true
        ));
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().occurrence_count(), 3);
        assert!(set_occurrence_tag(
            shell.app_mut(),
            OccurrenceId(1),
            Some(tag)
        ));
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::SetOccurrenceVisibility {
                occurrence_id: OccurrenceId(3).0,
                visible: false,
            }
        ));
        shell.settle();

        let select_label = shell.catalog().text("tags-select-untagged");
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(3)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-untagged",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );

        let selected_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_untagged_occurrences());
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), selected_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        assert!(set_occurrence_tag(
            shell.app_mut(),
            OccurrenceId(2),
            Some(tag)
        ));
        let tagged_revision = shell.app().document_revision();
        let tagged_digest = shell.app().canonical_digest();
        let tagged_undo_steps = shell.app().undo_step_count();
        let tagged_action_digest = shell.app().action_digest().to_owned();
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(shell.app().document_revision(), tagged_revision);
        assert_eq!(shell.app().canonical_digest(), tagged_digest);
        assert_eq!(shell.app().undo_step_count(), tagged_undo_steps);
        assert_eq!(shell.app().action_digest(), tagged_action_digest);
    }
}

#[test]
fn tags_panel_assign_selection_is_localized_canonical_context_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(1201);
        assert!(create_tag(
            shell.app_mut(),
            tag,
            "Hardware".to_owned(),
            true
        ));
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().occurrence_count(), 3);
        assert!(set_occurrence_tag(
            shell.app_mut(),
            OccurrenceId(1),
            Some(tag)
        ));
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 3);

        let assign_label = shell.catalog().format(
            "tags-assign-selection",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &assign_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
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

        shell.click_role_and_label(Role::Button, &assign_label);

        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        let assigned = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = assigned.occurrence(id).unwrap();
            assert_eq!(occurrence.tags().first().copied(), Some(tag));
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
        assert!(shell.has_role_and_label(Role::Button, &assign_label));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), None);
        assert_eq!(first_tag(&shell, OccurrenceId(3)), None);
        shell.click_role_and_label(Role::Button, &assign_label);
        let unchanged_revision = shell.app().document_revision();
        let unchanged_digest = shell.app().canonical_digest();
        let unchanged_undo_steps = shell.app().undo_step_count();
        let unchanged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().assign_selection_to_tag(tag));
        assert!(!shell.app_mut().assign_selection_to_tag(TagId(9999)));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        assert_eq!(shell.app().action_digest(), unchanged_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert!(shell.has_role_and_label(Role::Button, &assign_label));
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().assign_selection_to_tag(tag));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
    }
}

#[test]
fn tags_panel_remove_selection_is_localized_canonical_context_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(1301);
        let other_tag = TagId(1302);
        for (target, name) in [(tag, "Hardware"), (other_tag, "Other")] {
            assert!(create_tag(shell.app_mut(), target, name.to_owned(), true));
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        for (target, assigned_tag) in [
            (OccurrenceId(1), tag),
            (OccurrenceId(2), tag),
            (OccurrenceId(3), other_tag),
        ] {
            assert!(set_occurrence_tag(
                shell.app_mut(),
                target,
                Some(assigned_tag)
            ));
        }
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 3);

        let remove_label = shell.catalog().format(
            "tags-remove-selection",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        let remove_other_label = shell.catalog().format(
            "tags-remove-selection",
            &BTreeMap::from([("name", "Other".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &remove_label));
        assert!(shell.has_role_and_label(Role::Button, &remove_other_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
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

        shell.click_role_and_label(Role::Button, &remove_label);

        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        let removed = shell.app().document_snapshot();
        assert_eq!(
            removed
                .occurrence(OccurrenceId(1))
                .unwrap()
                .tags()
                .first()
                .copied(),
            None
        );
        assert_eq!(
            removed
                .occurrence(OccurrenceId(2))
                .unwrap()
                .tags()
                .first()
                .copied(),
            None
        );
        assert_eq!(
            removed
                .occurrence(OccurrenceId(3))
                .unwrap()
                .tags()
                .first()
                .copied(),
            Some(other_tag)
        );
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = removed.occurrence(id).unwrap();
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
                "digest-removed-selected-tag",
                &BTreeMap::from([("count", "2".to_owned()), ("tag", "Hardware".to_owned()),])
            )
        );
        assert!(shell.has_role_and_label(Role::Button, &remove_label));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(first_tag(&shell, OccurrenceId(1)), Some(tag));
        assert_eq!(first_tag(&shell, OccurrenceId(2)), Some(tag));
        assert_eq!(first_tag(&shell, OccurrenceId(3)), Some(other_tag));
        shell.click_role_and_label(Role::Button, &remove_label);
        assert!(shell.has_role_and_label(Role::Button, &remove_label));
        let unchanged_revision = shell.app().document_revision();
        let unchanged_digest = shell.app().canonical_digest();
        let unchanged_undo_steps = shell.app().undo_step_count();
        let unchanged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().remove_selection_from_tag(tag));
        assert!(!shell.app_mut().remove_selection_from_tag(TagId(9999)));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        assert_eq!(shell.app().action_digest(), unchanged_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert!(shell.has_role_and_label(Role::Button, &remove_label));
        assert!(shell.has_role_and_label(Role::Button, &remove_other_label));
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().remove_selection_from_tag(other_tag));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
    }
}

#[test]
fn hide_unhide_selection_is_localized_exact_state_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Hide),
            shell.catalog().text("model-hide")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::Unhide),
            shell.catalog().text("model-unhide")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Hide));
        assert!(!shell.app().command_is_enabled(AppCommand::Unhide));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Hide));
        assert!(!shell.app().command_is_enabled(AppCommand::Unhide));
        let visible_digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::Hide);
        let hidden_digest = shell.app().canonical_digest();
        assert_ne!(hidden_digest, visible_digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                !shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-hidden",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Hide));
        assert!(shell.app().command_is_enabled(AppCommand::Unhide));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), visible_digest);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        shell.click_menu_command("menu-view", AppCommand::Unhide);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 2);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-unhidden",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );

        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(harness::apply_structure(
            shell.app_mut(),
            AssistantStructureChange::SetOccurrenceVisibility {
                occurrence_id: OccurrenceId(1).0,
                visible: false,
            }
        ));
        assert!(
            !shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .visible()
        );
        assert!(
            shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(2))
                .unwrap()
                .visible()
        );
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Hide));
        assert!(shell.app().command_is_enabled(AppCommand::Unhide));
        let mixed_digest = shell.app().canonical_digest();
        let mixed_undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::Hide);
        assert_eq!(shell.app().undo_step_count(), mixed_undo_steps + 1);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                !shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-hidden",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), mixed_digest);
        assert!(
            !shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .visible()
        );
        assert!(
            shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(2))
                .unwrap()
                .visible()
        );
        shell.click_menu_command("menu-view", AppCommand::Unhide);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-unhidden",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );

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
        assert!(!context_shell.app().command_is_enabled(AppCommand::Hide));
        assert!(!context_shell.app().command_is_enabled(AppCommand::Unhide));
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        assert!(!context_shell.app_mut().set_selection_visibility(false));
        assert!(!context_shell.app_mut().set_selection_visibility(true));
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn hide_others_is_localized_root_scope_exact_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::HideOthers),
            shell.catalog().text("model-hide-others")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::HideOthers));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(shell.top_face_centre(1));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().command_is_enabled(AppCommand::HideOthers));
        let visible_revision = shell.app().document_revision();
        let visible_digest = shell.app().canonical_digest();
        let visible_undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::HideOthers));
        shell.click_command(AppCommand::HideOthers);

        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), visible_revision + 1);
        assert_eq!(shell.app().undo_step_count(), visible_undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(
            shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .visible()
        );
        assert!(
            !shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(2))
                .unwrap()
                .visible()
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-hidden-others",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );
        let isolated_digest = shell.app().canonical_digest();
        assert!(!shell.app().command_is_enabled(AppCommand::HideOthers));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), visible_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), isolated_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.click_menu_command("menu-model", AppCommand::HideOthers);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), isolated_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.double_click_at(shell.top_face_centre(1));
        assert_eq!(shell.app().edit_context_depth(), 1);
        assert!(!shell.app().command_is_enabled(AppCommand::HideOthers));
        let context_revision = shell.app().document_revision();
        let context_digest = shell.app().canonical_digest();
        let context_undo_steps = shell.app().undo_step_count();
        let context_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().hide_others());
        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), context_revision);
        assert_eq!(shell.app().canonical_digest(), context_digest);
        assert_eq!(shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn unhide_all_is_localized_root_scope_exact_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::UnhideAll),
            shell.catalog().text("model-unhide-all")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::UnhideAll));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        shell.click_menu_command("menu-view", AppCommand::Hide);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert!(shell.app().command_is_enabled(AppCommand::UnhideAll));
        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::UnhideAll));
        shell.click_command(AppCommand::UnhideAll);

        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), hidden_revision + 1);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-unhidden-all",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );
        let visible_digest = shell.app().canonical_digest();

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), visible_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.click_menu_command("menu-model", AppCommand::UnhideAll);
        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), visible_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.double_click_at(shell.top_face_centre(1));
        assert_eq!(shell.app().edit_context_depth(), 1);
        assert!(!shell.app().command_is_enabled(AppCommand::UnhideAll));
        let context_revision = shell.app().document_revision();
        let context_digest = shell.app().canonical_digest();
        let context_undo_steps = shell.app().undo_step_count();
        let context_action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-model", AppCommand::UnhideAll);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), context_revision);
        assert_eq!(shell.app().canonical_digest(), context_digest);
        assert_eq!(shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn ground_occurrence_is_localized_state_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::GroundOccurrence),
            shell.catalog().text("model-ground-occurrence")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::UngroundOccurrence),
            shell.catalog().text("model-unground-occurrence")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::GroundOccurrence));
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );

        let initial_revision = shell.app().document_revision();
        let initial_digest = shell.app().canonical_digest();
        let initial_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::GroundOccurrence);
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps);

        // The face picked on the exact body the user sees; a box face picked
        // earlier moves to it when the body arrives (TST-3).
        wait_for_one_exact_body(&mut shell);
        shell.click_at(shell.top_face_centre(1));
        let selected_reference = shell
            .app()
            .selected_reference()
            .expect("the exact root occurrence is primary");
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(selected_reference.instance_path.is_root());
        assert!(shell.app().command_is_enabled(AppCommand::GroundOccurrence));
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );
        shell.click_menu_command("menu-model", AppCommand::GroundOccurrence);
        let grounded_digest = shell.app().canonical_digest();
        assert_eq!(shell.app().grounded_occurrence_count(), 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(
            shell.app().selected_reference(),
            Some(selected_reference.clone())
        );
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-grounded-occurrence")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::GroundOccurrence));
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );

        shell.click_menu_command("menu-model", AppCommand::UngroundOccurrence);
        let ungrounded_digest = shell.app().canonical_digest();
        assert_eq!(shell.app().grounded_occurrence_count(), 0);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(shell.app().selected_reference(), Some(selected_reference));
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 2);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-ungrounded-occurrence")
        );
        assert!(shell.app().command_is_enabled(AppCommand::GroundOccurrence));
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().grounded_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), grounded_digest);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().grounded_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().grounded_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), grounded_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().grounded_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), ungrounded_digest);

        let mut multi_shell = Shell::with_catalog(catalog.clone());
        multi_shell.click_at(multi_shell.viewport_rect().center());
        assert!(
            multi_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        multi_shell.settle();
        multi_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        multi_shell.click_at_with(multi_shell.top_face_centre(1), shift());
        assert_eq!(multi_shell.app().selected_occurrence_count(), 2);
        assert!(
            !multi_shell
                .app()
                .command_is_enabled(AppCommand::GroundOccurrence)
        );
        assert!(
            !multi_shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );
        let multi_revision = multi_shell.app().document_revision();
        let multi_digest = multi_shell.app().canonical_digest();
        let multi_undo_steps = multi_shell.app().undo_step_count();
        multi_shell.click_menu_command("menu-model", AppCommand::GroundOccurrence);
        multi_shell.click_menu_command("menu-model", AppCommand::UngroundOccurrence);
        assert_eq!(multi_shell.app().grounded_occurrence_count(), 0);
        assert_eq!(multi_shell.app().document_revision(), multi_revision);
        assert_eq!(multi_shell.app().canonical_digest(), multi_digest);
        assert_eq!(multi_shell.app().undo_step_count(), multi_undo_steps);

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
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::GroundOccurrence)
        );
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        context_shell.click_menu_command("menu-model", AppCommand::GroundOccurrence);
        context_shell.click_menu_command("menu-model", AppCommand::UngroundOccurrence);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
    }
}

#[test]
fn occurrence_align_is_localized_previewed_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::AlignOccurrences),
            shell.catalog().text("model-align-occurrences")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::AlignOccurrences));

        shell.click_at(shell.top_face_centre(1));
        assert!(
            shell
                .app_mut()
                .copy_selected(Vec3::new(300.0, 300.0, 100.0))
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
        let reference_name = shell
            .app()
            .occurrence_name(OccurrenceId(1))
            .expect("the reference occurrence exists");
        let reference_row = shell.catalog().format(
            "outliner-instance",
            &BTreeMap::from([("visibility", "◉".to_owned()), ("name", reference_name)]),
        );
        shell.click_row(&reference_row);
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        assert_eq!(
            shell.app().occurrence_align_inputs(),
            Some((OccurrenceId(2), OccurrenceId(1), Axis::X, AlignMode::Center))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-cancel"),
        );
        assert!(!shell.app().occurrence_align_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-axis-y"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-minimum"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(2)),
            Some((Vec3::new(300.0, 0.0, 100.0), Vec3::new(100.0, 60.0, 20.0)))
        );

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-axis-z"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-preview"),
        );
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(2)),
            Some((Vec3::new(300.0, 300.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-confirm"),
        );
        assert!(!shell.app().occurrence_align_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-align-committed")
        );
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(1)),
            Some(DefinitionId(1))
        );
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(DefinitionId(1))
        );
        let aligned_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), aligned_digest);

        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_align());
        assert!(!shell.app_mut().confirm_occurrence_align());
        assert!(shell.app().occurrence_align_visible());
        assert!(!shell.app().occurrence_align_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        assert!(shell.app_mut().preview_pending_occurrence_align());
        assert!(shell.app().occurrence_align_preview_is_current());
        assert!(
            shell
                .app_mut()
                .preview_linear_pattern(OccurrenceId(1), Axis::X, 100.0, 2,)
        );
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        assert!(!shell.app().occurrence_align_preview_is_current());
        assert!(!shell.app_mut().confirm_occurrence_align());
        assert!(shell.app().occurrence_align_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_align());
        assert!(!shell.app_mut().confirm_occurrence_align());
        assert!(shell.app().occurrence_align_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        let (moving_id, reference_id, _, _) = shell.app().occurrence_align_inputs().unwrap();
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_align());
        assert!(!shell.app_mut().confirm_occurrence_align());
        assert!(shell.app().occurrence_align_visible());
        assert_eq!(shell.app().occurrence_definition_id(moving_id), None);
        assert_eq!(shell.app().occurrence_definition_id(reference_id), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn distribute_occurrences_is_localized_previewed_even_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::DistributeOccurrences),
            shell.catalog().text("model-distribute-occurrences")
        );
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::DistributeOccurrences)
        );
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert!(!shell.app().occurrence_distribution_visible());

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));
        assert!(shell.app_mut().copy_selected(Vec3::new(400.0, 0.0, 0.0)));
        assert!(shell.app_mut().copy_selected(Vec3::new(400.0, 0.0, 0.0)));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::DistributeOccurrences)
        );
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert_eq!(shell.app().occurrence_distribution_axis(), Some(Axis::X));
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );
        assert!(!shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-axis-y"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-axis-x"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(2)),
            Some((Vec3::new(300.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(3)),
            Some((Vec3::new(600.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );

        assert!(shell.app_mut().create_box());
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        assert!(!shell.app_mut().confirm_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-confirm"),
        );
        assert!(!shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), revision + 2);
        assert_eq!(shell.app().occurrence_count(), 4);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-distribute-committed")
        );
        let origins = (1..=4)
            .map(|id| shell.app().occurrence_box_geometry(id).unwrap().0.x)
            .collect::<Vec<_>>();
        assert_eq!(origins, vec![0.0, 300.0, 600.0, 900.0]);
        let distributed_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), distributed_digest);

        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_at(shell.top_face_centre(2));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app_mut().move_selected(Vec3::new(100.0, 0.0, 0.0)));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::DistributeOccurrences)
        );
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_distribution());
        assert!(!shell.app_mut().confirm_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_visible());
        assert!(!shell.app().occurrence_distribution_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert!(
            shell.app_mut().preview_pending_occurrence_distribution(),
            "status={}, selected={}, boxes={:?}",
            shell.app().action_digest(),
            shell.app().selected_occurrence_count(),
            (1..=4)
                .map(|id| shell.app().occurrence_box_geometry(id))
                .collect::<Vec<_>>()
        );
        assert!(shell.app().occurrence_distribution_preview_is_current());
        assert!(
            shell
                .app_mut()
                .preview_linear_pattern(OccurrenceId(1), Axis::X, 100.0, 2,)
        );
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        let foreign_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app().occurrence_distribution_preview_is_current());
        assert!(!shell.app_mut().confirm_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        assert_eq!(shell.app().action_digest(), foreign_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_distribution());
        assert!(!shell.app_mut().confirm_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert!(shell.app_mut().delete_selected());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_distribution());
        assert!(!shell.app_mut().confirm_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn distribute_equal_gaps_is_localized_previewed_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app_mut().copy_selected(Vec3::new(200.0, 0.0, 0.0)));
        assert!(shell.app_mut().rotate_selected(90.0));
        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app_mut().copy_selected(Vec3::new(500.0, 0.0, 0.0)));
        assert!(shell.app_mut().copy_selected(Vec3::new(300.0, 0.0, 0.0)));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let first = shell.app().occurrence_box_geometry(1).unwrap();
        let last = shell.app().occurrence_box_geometry(4).unwrap();

        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert_eq!(
            shell.app().occurrence_distribution_mode(),
            Some(DistributionMode::Centers)
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-equal-gaps"),
        );
        assert_eq!(
            shell.app().occurrence_distribution_mode(),
            Some(DistributionMode::EqualGaps)
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        let second = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .unwrap();
        let third = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(3))
            .unwrap();
        let gaps = [
            second.0.x - (first.0.x + first.1.x),
            third.0.x - (second.0.x + second.1.x),
            last.0.x - (third.0.x + third.1.x),
        ];
        assert!(gaps.iter().all(|gap| (*gap - gaps[0]).abs() < 1.0e-9));

        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-centers"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-equal-gaps"),
        );
        assert_eq!(
            shell.app().occurrence_distribution_mode(),
            Some(DistributionMode::EqualGaps)
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert!(shell.app().occurrence_distribution_preview_is_current());
        shell.settle();
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-confirm"),
        );
        assert!(!shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-distribute-committed")
        );
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(DefinitionId(1))
        );
        let distributed_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), distributed_digest);
    }
}

#[test]
fn distribute_equal_gaps_rejects_an_overlapping_span_without_mutation() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    assert!(shell.app_mut().copy_selected(Vec3::new(10.0, 0.0, 0.0)));
    assert!(shell.app_mut().copy_selected(Vec3::new(10.0, 0.0, 0.0)));
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    assert!(!shell.app_mut().preview_distribute_occurrences(
        &BTreeSet::from([OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]),
        Axis::X,
        DistributionMode::EqualGaps,
    ));
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    let mut stale_shell = Shell::new();
    stale_shell.click_at(stale_shell.top_face_centre(1));
    assert!(
        stale_shell
            .app_mut()
            .copy_selected(Vec3::new(150.0, 0.0, 0.0))
    );
    assert!(
        stale_shell
            .app_mut()
            .copy_selected(Vec3::new(250.0, 0.0, 0.0))
    );
    stale_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    assert!(stale_shell.app_mut().preview_distribute_occurrences(
        &BTreeSet::from([OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]),
        Axis::X,
        DistributionMode::EqualGaps,
    ));
    assert!(stale_shell.app_mut().create_box());
    let stale_revision = stale_shell.app().document_revision();
    let stale_digest = stale_shell.app().canonical_digest();
    assert!(!stale_shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(stale_shell.app().document_revision(), stale_revision);
    assert_eq!(stale_shell.app().canonical_digest(), stale_digest);
}

#[test]
fn linear_pattern_is_localized_previewed_shared_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::LinearPattern),
            shell.catalog().text("model-linear-pattern")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::LinearPattern));
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert!(!shell.app().linear_pattern_visible());

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::LinearPattern));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert_eq!(
            shell.app().linear_pattern_inputs(),
            Some((Axis::X, "100", "2"))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-cancel"),
        );
        assert!(!shell.app().linear_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        let spacing_label = shell.catalog().text("dialog-linear-pattern-spacing");
        shell.focus_text_input(&spacing_label);
        shell.key(Key::A, ctrl());
        shell.press_key(Key::Backspace);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-preview"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);

        shell.focus_text_input(&spacing_label);
        shell.type_text("125");
        let count_label = shell.catalog().text("dialog-linear-pattern-count");
        shell.focus_text_input(&count_label);
        shell.key(Key::A, ctrl());
        shell.type_text("4");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-preview"),
        );
        assert!(shell.app().linear_pattern_visible());
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(4)),
            Some((Vec3::new(375.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-confirm"),
        );
        assert!(!shell.app().linear_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().occurrence_count(), 4);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-linear-pattern-committed")
        );
        for occurrence_id in 1..=4 {
            assert_eq!(
                shell
                    .app()
                    .occurrence_definition_id(OccurrenceId(occurrence_id)),
                Some(DefinitionId(1))
            );
        }
        // The planner's one copy name rule, the same as for an MCP pattern.
        let definition_name = shell.app().definition_name(DefinitionId(1)).unwrap();
        for occurrence_id in 2..=4 {
            assert_eq!(
                shell.app().occurrence_name(OccurrenceId(occurrence_id)),
                Some(format!("{definition_name} {occurrence_id}"))
            );
            assert_eq!(
                shell.app().occurrence_box_geometry(occurrence_id),
                Some((
                    Vec3::new(125.0 * (occurrence_id - 1) as f64, 0.0, 0.0),
                    Vec3::new(100.0, 60.0, 20.0),
                ))
            );
        }
        let patterned_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().occurrence_count(), 1);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), patterned_digest);
        assert_eq!(shell.app().occurrence_count(), 4);

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::LinearPattern));
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_linear_pattern());
        assert!(!shell.app_mut().confirm_linear_pattern());
        assert!(shell.app().linear_pattern_visible());
        assert!(!shell.app().linear_pattern_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert!(shell.app_mut().preview_pending_linear_pattern());
        assert!(shell.app().linear_pattern_preview_is_current());
        assert!(shell.app_mut().preview_rectangular_pattern(
            OccurrenceId(1),
            RectangularPatternSpec {
                primary_axis: Axis::X,
                primary_spacing_mm: 200.0,
                primary_count: 2,
                secondary_axis: Axis::Y,
                secondary_spacing_mm: 200.0,
                secondary_count: 2,
            },
        ));
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        let foreign_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app().linear_pattern_preview_is_current());
        assert!(!shell.app_mut().confirm_linear_pattern());
        assert!(shell.app().linear_pattern_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        assert_eq!(shell.app().action_digest(), foreign_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_linear_pattern());
        assert!(!shell.app_mut().confirm_linear_pattern());
        assert!(shell.app().linear_pattern_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-cancel"),
        );

        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert!(shell.app_mut().delete_selected());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_linear_pattern());
        assert!(!shell.app_mut().confirm_linear_pattern());
        assert!(shell.app().linear_pattern_visible());
        assert_eq!(shell.app().occurrence_definition_id(OccurrenceId(1)), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

/// Review 2026-10-17 P3-G: where the decimal separator is a point, a dialog
/// field with a comma says why under it, in the window's language, and stops
/// saying so once the value is written with a point.
#[test]
fn a_dialog_field_with_a_comma_says_why_in_the_windows_language() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    shell.click_at(shell.top_face_centre(1));
    let why = shell.catalog().text("value-comma-not-decimal");
    for (command, field, cancel) in [
        (
            AppCommand::LinearPattern,
            "dialog-linear-pattern-spacing",
            "dialog-linear-pattern-cancel",
        ),
        (
            AppCommand::RectangularPattern,
            "dialog-rectangular-pattern-secondary-spacing",
            "dialog-rectangular-pattern-cancel",
        ),
        (
            AppCommand::CircularPattern,
            "dialog-circular-pattern-angle",
            "dialog-circular-pattern-cancel",
        ),
    ] {
        shell.click_menu_command("menu-model", command);
        assert!(!shell.has_visible_label(&why), "{command:?}");
        set_dialog_field(&mut shell, field, "12,5");
        assert!(shell.has_visible_label(&why), "{command:?} says why");
        set_dialog_field(&mut shell, field, "12.5");
        assert!(!shell.has_visible_label(&why), "{command:?}");
        let cancel = shell.catalog().text(cancel);
        shell.click_role_and_label(Role::Button, &cancel);
    }
}

#[test]
fn every_pattern_dialog_refuses_513_new_copies_and_makes_512() {
    let mut shell = Shell::with_catalog(LocaleCatalog::english());
    shell.click_at(shell.top_face_centre(1));
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    let limit = shell.catalog().format(
        "dialog-pattern-copy-limit",
        &BTreeMap::from([("limit", "512".to_owned())]),
    );
    let refused = copy_limit_exceeded(&shell, 513);
    assert!(
        refused.contains("513") && refused.contains("512"),
        "{refused}"
    );

    // Each dialog shows the limit, names it when 513 new copies are typed and
    // offers no preview of them; nothing in the document changes.
    for (command, count_fields, preview, cancel) in [
        (
            AppCommand::LinearPattern,
            &[("dialog-linear-pattern-count", "514")][..],
            "dialog-linear-pattern-preview",
            "dialog-linear-pattern-cancel",
        ),
        (
            AppCommand::RectangularPattern,
            &[
                ("dialog-rectangular-pattern-primary-count", "2"),
                ("dialog-rectangular-pattern-secondary-count", "257"),
            ][..],
            "dialog-rectangular-pattern-preview",
            "dialog-rectangular-pattern-cancel",
        ),
        (
            AppCommand::CircularPattern,
            &[("dialog-circular-pattern-count", "514")][..],
            "dialog-circular-pattern-preview",
            "dialog-circular-pattern-cancel",
        ),
    ] {
        shell.click_menu_command("menu-model", command);
        assert!(
            shell.has_visible_label(&limit),
            "{command:?} shows the limit"
        );
        for (key, value) in count_fields {
            set_dialog_field(&mut shell, key, value);
        }
        assert!(shell.has_visible_label(&refused), "{command:?} names 512");
        let preview = shell.catalog().text(preview);
        assert!(
            !shell.button_is_enabled(&preview),
            "{command:?} previews 513 copies"
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        let cancel = shell.catalog().text(cancel);
        shell.click_role_and_label(Role::Button, &cancel);
    }
    shell.click_menu_command("menu-model", AppCommand::LinearPattern);
    set_dialog_field(&mut shell, "dialog-linear-pattern-count", "514");
    assert!(!shell.app_mut().preview_pending_linear_pattern());
    assert!(!shell.app_mut().confirm_linear_pattern());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    // 512 new copies is within the limit: exactly 512 new parts, one Undo step.
    set_dialog_field(&mut shell, "dialog-linear-pattern-spacing", "120");
    set_dialog_field(&mut shell, "dialog-linear-pattern-count", "513");
    assert!(!shell.has_visible_label(&refused));
    let preview = shell.catalog().text("dialog-linear-pattern-preview");
    shell.click_role_and_label(Role::Button, &preview);
    assert!(shell.app().has_occurrence_operation_preview());
    let confirm = shell.catalog().text("dialog-linear-pattern-confirm");
    shell.click_role_and_label(Role::Button, &confirm);
    shell.wait_for_digest("digest-linear-pattern-committed");
    assert_eq!(shell.app().occurrence_count(), 513);
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(513),
        Some((
            Vec3::new(120.0 * 512.0, 0.0, 0.0),
            Vec3::new(100.0, 60.0, 20.0)
        ))
    );
    let definition_name = shell.app().definition_name(DefinitionId(1)).unwrap();
    assert_eq!(
        shell.app().occurrence_name(OccurrenceId(513)),
        Some(format!("{definition_name} 513"))
    );
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().occurrence_count(), 1);
}

#[test]
fn rectangular_pattern_is_localized_previewed_shared_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::RectangularPattern),
            shell.catalog().text("model-rectangular-pattern")
        );
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::RectangularPattern)
        );
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert!(!shell.app().rectangular_pattern_visible());

        shell.click_at(shell.top_face_centre(1));
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::RectangularPattern)
        );
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert_eq!(
            shell.app().rectangular_pattern_inputs(),
            Some((Axis::X, "100", "2", Axis::Y, "100", "2"))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-cancel"),
        );
        assert!(!shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        for (key, value) in [
            ("dialog-rectangular-pattern-primary-spacing", "125"),
            ("dialog-rectangular-pattern-primary-count", "3"),
            ("dialog-rectangular-pattern-secondary-spacing", "75"),
        ] {
            let label = shell.catalog().text(key);
            shell.focus_text_input(&label);
            shell.key(Key::A, ctrl());
            shell.type_text(value);
        }
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(2)),
            Some((Vec3::new(0.0, 75.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(6)),
            Some((Vec3::new(250.0, 75.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );

        let primary_spacing = shell
            .catalog()
            .text("dialog-rectangular-pattern-primary-spacing");
        shell.focus_text_input(&primary_spacing);
        shell.key(Key::A, ctrl());
        shell.type_text("150");
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(6)),
            Some((Vec3::new(300.0, 75.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-confirm"),
        );
        assert!(!shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().occurrence_count(), 6);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-rectangular-pattern-committed")
        );
        for occurrence_id in 1..=6 {
            assert_eq!(
                shell
                    .app()
                    .occurrence_definition_id(OccurrenceId(occurrence_id)),
                Some(DefinitionId(1))
            );
        }
        let patterned_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().occurrence_count(), 1);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), patterned_digest);
        assert_eq!(shell.app().occurrence_count(), 6);

        shell.click_at(shell.top_face_centre(1));
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::RectangularPattern)
        );
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_rectangular_pattern());
        assert!(!shell.app_mut().confirm_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_visible());
        assert!(!shell.app().rectangular_pattern_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert!(shell.app_mut().preview_pending_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_preview_is_current());
        assert!(
            shell
                .app_mut()
                .preview_linear_pattern(OccurrenceId(1), Axis::X, 100.0, 2,)
        );
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        let foreign_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app().rectangular_pattern_preview_is_current());
        assert!(!shell.app_mut().confirm_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        assert_eq!(shell.app().action_digest(), foreign_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_rectangular_pattern());
        assert!(!shell.app_mut().confirm_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-cancel"),
        );

        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert!(shell.app_mut().delete_selected());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_rectangular_pattern());
        assert!(!shell.app_mut().confirm_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().occurrence_definition_id(OccurrenceId(1)), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn rectangular_pattern_rejects_invalid_and_stale_inputs_without_mutation() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    assert!(!shell.app_mut().preview_rectangular_pattern(
        OccurrenceId(1),
        RectangularPatternSpec {
            primary_axis: Axis::X,
            primary_spacing_mm: 100.0,
            primary_count: 2,
            secondary_axis: Axis::X,
            secondary_spacing_mm: 100.0,
            secondary_count: 2,
        },
    ));
    assert!(!shell.app_mut().preview_rectangular_pattern(
        OccurrenceId(1),
        RectangularPatternSpec {
            primary_axis: Axis::X,
            primary_spacing_mm: 100.0,
            primary_count: 101,
            secondary_axis: Axis::Y,
            secondary_spacing_mm: 100.0,
            secondary_count: 100,
        },
    ));
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    assert!(shell.app_mut().preview_rectangular_pattern(
        OccurrenceId(1),
        RectangularPatternSpec {
            primary_axis: Axis::X,
            primary_spacing_mm: 100.0,
            primary_count: 2,
            secondary_axis: Axis::Y,
            secondary_spacing_mm: 100.0,
            secondary_count: 2,
        },
    ));
    assert!(shell.app_mut().create_box());
    let stale_revision = shell.app().document_revision();
    let stale_digest = shell.app().canonical_digest();
    let stale_undo_steps = shell.app().undo_step_count();
    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), stale_revision);
    assert_eq!(shell.app().canonical_digest(), stale_digest);
    assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
}

#[test]
fn circular_pattern_is_localized_previewed_shared_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::CircularPattern),
            shell.catalog().text("model-circular-pattern")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::CircularPattern));
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert!(!shell.app().circular_pattern_visible());

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::CircularPattern));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert_eq!(
            shell.app().circular_pattern_inputs(),
            Some((Axis::Z, "0", "0", "0", "90", "4"))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-cancel"),
        );
        assert!(!shell.app().circular_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        let angle_label = shell.catalog().text("dialog-circular-pattern-angle");
        shell.focus_text_input(&angle_label);
        shell.key(Key::A, ctrl());
        shell.press_key(Key::Backspace);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-preview"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);

        shell.focus_text_input(&angle_label);
        shell.type_text("90");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-preview"),
        );
        assert!(shell.app().circular_pattern_visible());
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .expect("the first rotated occurrence is previewed");
        assert!((origin.x + 60.0).abs() < 1.0e-9);
        assert!(origin.y.abs() < 1.0e-9);
        assert!(origin.z.abs() < 1.0e-9);
        assert!((size.x - 60.0).abs() < 1.0e-9);
        assert!((size.y - 100.0).abs() < 1.0e-9);
        assert!((size.z - 20.0).abs() < 1.0e-9);

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-axis-x"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-preview"),
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .expect("the X-axis occurrence is previewed");
        assert!(origin.x.abs() < 1.0e-9);
        assert!((origin.y + 20.0).abs() < 1.0e-9);
        assert!(origin.z.abs() < 1.0e-9);
        assert!((size.x - 100.0).abs() < 1.0e-9);
        assert!((size.y - 20.0).abs() < 1.0e-9);
        assert!((size.z - 60.0).abs() < 1.0e-9);

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-axis-z"),
        );
        let centre_x_label = shell.catalog().text("dialog-circular-pattern-centre-x");
        shell.focus_text_input(&centre_x_label);
        shell.key(Key::A, ctrl());
        shell.type_text("100");
        assert!(!shell.app().has_occurrence_operation_preview());
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-preview"),
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .expect("the offset-center occurrence is previewed");
        assert!((origin.x - 40.0).abs() < 1.0e-9);
        assert!((origin.y + 100.0).abs() < 1.0e-9);
        assert!(origin.z.abs() < 1.0e-9);
        assert!((size.x - 60.0).abs() < 1.0e-9);
        assert!((size.y - 100.0).abs() < 1.0e-9);
        assert!((size.z - 20.0).abs() < 1.0e-9);

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-confirm"),
        );
        assert!(!shell.app().circular_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().occurrence_count(), 4);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-circular-pattern-committed")
        );
        for occurrence_id in 1..=4 {
            assert_eq!(
                shell
                    .app()
                    .occurrence_definition_id(OccurrenceId(occurrence_id)),
                Some(DefinitionId(1))
            );
        }
        let patterned_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().occurrence_count(), 1);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), patterned_digest);
        assert_eq!(shell.app().occurrence_count(), 4);

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::CircularPattern));
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_circular_pattern());
        assert!(!shell.app_mut().confirm_circular_pattern());
        assert!(shell.app().circular_pattern_visible());
        assert!(!shell.app().circular_pattern_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert!(shell.app_mut().preview_pending_circular_pattern());
        assert!(shell.app().circular_pattern_preview_is_current());
        assert!(
            shell
                .app_mut()
                .preview_linear_pattern(OccurrenceId(1), Axis::X, 100.0, 2,)
        );
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        let foreign_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app().circular_pattern_preview_is_current());
        assert!(!shell.app_mut().confirm_circular_pattern());
        assert!(shell.app().circular_pattern_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        assert_eq!(shell.app().action_digest(), foreign_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_circular_pattern());
        assert!(!shell.app_mut().confirm_circular_pattern());
        assert!(shell.app().circular_pattern_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-cancel"),
        );

        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert!(shell.app_mut().delete_selected());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_circular_pattern());
        assert!(!shell.app_mut().confirm_circular_pattern());
        assert!(shell.app().circular_pattern_visible());
        assert_eq!(shell.app().occurrence_definition_id(OccurrenceId(1)), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }

    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let digest = shell.app().canonical_digest();
    assert!(shell.app_mut().preview_circular_pattern(
        OccurrenceId(1),
        Axis::Z,
        Vec3::new(0.0, 0.0, 0.0),
        45.0,
        3,
    ));
    assert!(shell.app_mut().copy_selected(Vec3::new(200.0, 0.0, 0.0)));
    let changed_digest = shell.app().canonical_digest();
    assert_ne!(changed_digest, digest);
    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    assert_eq!(shell.app().occurrence_count(), 2);
}

#[test]
fn rename_definition_is_localized_validated_shared_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let definition_id = DefinitionId(1);
        let original_name = shell
            .app()
            .definition_name(definition_id)
            .expect("the initial definition exists");
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(1)),
            shell.app().occurrence_definition_id(OccurrenceId(2))
        );
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        assert_eq!(
            shell.app().command_label(AppCommand::RenameDefinition),
            shell.catalog().text("model-rename-definition")
        );
        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        assert!(shell.app().rename_definition_visible());
        assert_eq!(
            shell.app().rename_definition_input(),
            Some(original_name.as_str())
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-cancel"),
        );
        assert!(!shell.app().rename_definition_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        shell.focus_text_input(&shell.catalog().text("dialog-rename-definition-name"));
        shell.key(Key::A, ctrl());
        shell.type_text("Selection drift");
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_definition_rename());
        assert!(shell.app().rename_definition_visible());
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-cancel"),
        );
        shell.click_at(shell.top_face_centre(1));

        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        let input_label = shell.catalog().text("dialog-rename-definition-name");
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.press_key(Key::Backspace);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-confirm"),
        );
        assert!(shell.app().rename_definition_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        let renamed = "Shared housing";
        shell.focus_text_input(&input_label);
        shell.type_text(renamed);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-confirm"),
        );
        assert!(!shell.app().rename_definition_visible());
        assert_eq!(
            shell.app().definition_name(definition_id),
            Some(renamed.to_owned())
        );
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-renamed-definition",
                &BTreeMap::from([("name", renamed.to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell.app().definition_name(definition_id),
            Some(original_name.clone())
        );
        assert_eq!(shell.app().occurrence_count(), 2);

        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Stale definition");
        assert!(shell.app_mut().create_box());
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_definition_rename());
        assert!(shell.app().rename_definition_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        assert_eq!(
            shell.app().definition_name(definition_id),
            Some(original_name.clone())
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-cancel"),
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Missing definition");
        assert!(shell.app_mut().delete_selected());
        assert!(shell.app_mut().purge_unused_definitions());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_definition_rename());
        assert!(shell.app().rename_definition_visible());
        assert_eq!(shell.app().definition_name(definition_id), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn purge_unused_definitions_is_localized_safe_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().occurrence_count(), 1);
        assert_eq!(shell.app().purgeable_definition_count(), 0);
        assert_eq!(
            shell.app().command_label(AppCommand::PurgeUnused),
            shell.catalog().text("model-purge-unused")
        );

        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().occurrence_count(), 2);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().purgeable_definition_count(), 2);
        let after_delete_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", AppCommand::PurgeUnused);
        assert_eq!(shell.app().definition_count(), 0);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().purgeable_definition_count(), 0);
        assert_eq!(shell.app().undo_step_count(), after_delete_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-purged-unused",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().purgeable_definition_count(), 2);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().definition_count(), 0);
        assert_eq!(shell.app().occurrence_count(), 0);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().purgeable_definition_count(), 0);

        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::PurgeUnused);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn a_screen_reader_finds_each_line_of_text_inside_the_widget_that_shows_it() {
    for (locale, catalog) in [
        ("en-US", LocaleCatalog::english()),
        ("sk-SK", LocaleCatalog::slovak()),
        ("pseudo", LocaleCatalog::pseudo()),
    ] {
        let shell = Shell::with_catalog(catalog);
        let outside = shell.text_runs_outside_their_widget();
        assert!(
            outside.is_empty(),
            "{locale}: text a screen reader finds outside its widget (text, line, widget): {outside:?}"
        );
    }
}

#[test]
fn localized_shells_fit_and_support_screen_reader_keyboard_focus() {
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0));
    for (locale, catalog) in [
        ("en-US", LocaleCatalog::english()),
        ("sk-SK", LocaleCatalog::slovak()),
        ("pseudo", LocaleCatalog::pseudo()),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert!(shell.viewport_rect().area() > 0.0, "{locale} viewport");

        for key in [
            "menu-file",
            "menu-edit",
            "menu-view",
            "menu-draw",
            "menu-tools",
            "menu-model",
            "menu-window",
            "menu-help",
        ] {
            let rect = shell.menu_rect(key);
            assert!(rect.area() > 0.0, "{locale} {key} has no layout box");
            assert!(
                screen.contains(rect.min) && screen.contains(rect.max),
                "{locale} {key} overflows the 1600 x 1000 acceptance viewport: {rect:?}"
            );
        }

        let visible_rects = shell.visible_accesskit_rects();
        assert!(visible_rects.len() > 20, "{locale} accessibility tree");
        for (node, rect) in visible_rects {
            let frame_tolerance = 1.5;
            assert!(
                rect.min.x >= screen.min.x - frame_tolerance
                    && rect.min.y >= screen.min.y - frame_tolerance
                    && rect.max.x <= screen.max.x + frame_tolerance
                    && rect.max.y <= screen.max.y + frame_tolerance,
                "{locale} publishes {node} outside the acceptance viewport: {rect:?}"
            );
        }

        for command in [
            AppCommand::Select,
            AppCommand::Rectangle,
            AppCommand::Circle,
            AppCommand::Arc,
            AppCommand::PushPull,
            AppCommand::Move,
            AppCommand::Measure,
            AppCommand::Orbit,
            AppCommand::Pan,
        ] {
            assert!(
                shell.offers(command),
                "{locale} must expose {command:?} by its localized AccessKit name"
            );
        }

        shell.focus_command(AppCommand::Rectangle);
        assert!(
            shell.command_is_focused(AppCommand::Rectangle),
            "{locale} AccessKit focus action must reach Rectangle"
        );
        let expected_digest = shell.catalog().format(
            "digest-tool-active",
            &BTreeMap::from([("tool", shell.catalog().text("tool-rectangle"))]),
        );
        shell.press_key(Key::Enter);
        assert_eq!(shell.app().action_digest(), expected_digest, "{locale}");
    }
}
