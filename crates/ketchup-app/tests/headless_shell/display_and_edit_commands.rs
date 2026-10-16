use super::*;

#[test]
fn the_designed_shell_lays_itself_out_without_a_window() {
    let shell = Shell::new();

    assert!(
        shell.viewport_rect().area() > 0.0,
        "the viewport must be laid out"
    );
    assert!(
        shell.offers(AppCommand::Select),
        "the tool rail must offer Select under an accessible name, not a glyph"
    );
    assert!(
        shell.offers(AppCommand::Move),
        "the tool rail must offer Move under an accessible name, not a glyph"
    );
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn window_menu_toggles_outliner_and_tags_without_mutating_the_document() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.settle();
        let outliner = shell.catalog().text("dock-outliner");
        let tags = shell.catalog().text("dock-tags");
        assert!(shell.app().outliner_visible());
        assert!(shell.app().tags_visible());
        assert!(shell.has_visible_label(&outliner));
        assert!(shell.has_visible_label(&tags));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.open_menu("menu-window");
        shell.click_role_and_label(Role::CheckBox, &outliner);
        shell.press_key(Key::Escape);
        assert!(!shell.app().outliner_visible());
        assert!(!shell.has_visible_label(&outliner));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.open_menu("menu-window");
        shell.click_role_and_label(Role::CheckBox, &tags);
        shell.press_key(Key::Escape);
        assert!(!shell.app().tags_visible());
        assert!(!shell.has_visible_label(&tags));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-file", AppCommand::New);
        assert!(!shell.app().outliner_visible());
        assert!(!shell.app().tags_visible());

        shell.open_menu("menu-window");
        shell.click_role_and_label(Role::CheckBox, &outliner);
        shell.press_key(Key::Escape);
        shell.open_menu("menu-window");
        shell.click_role_and_label(Role::CheckBox, &tags);
        shell.press_key(Key::Escape);
        assert!(shell.app().outliner_visible());
        assert!(shell.app().tags_visible());
        assert!(shell.has_visible_label(&outliner));
        assert!(shell.has_visible_label(&tags));
    }
}

/// Review 2026-10-09 §5.1 (1): the Slovak translation existed but nothing in
/// the window could switch to it. Window > Language switches the whole
/// interface at once, and back, without touching the document.
#[test]
fn ctrl_k_searches_commands_in_slovak_or_english_and_enter_runs_the_highlighted_one() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .set_ui_language(ketchup_app::language::UiLanguage::Slovak);
    shell.settle();
    let slovak = LocaleCatalog::slovak();

    // An English alias finds the Slovak command; its row shows the shortcut.
    shell.key(Key::K, ctrl());
    shell.type_text("shortcuts");
    let matches = shell.app().command_search_matches("shortcuts");
    assert_eq!(
        matches.first().map(|row| row.0),
        Some(AppCommand::Shortcuts)
    );
    assert!(shell.has_visible_label(&slovak.text("help-shortcuts")));
    assert!(shell.has_visible_label("F1"));

    // Arrows move the highlight; Enter runs the highlighted command.
    shell.key(Key::A, ctrl());
    shell.type_text("zoom");
    let matches = shell.app().command_search_matches("zoom");
    assert!(matches.len() >= 2, "{matches:?}");
    assert!(shell.button_is_selected(&matches[0].1));
    shell.key(Key::ArrowDown, Modifiers::NONE);
    assert!(shell.button_is_selected(&matches[1].1));
    assert!(!shell.button_is_selected(&matches[0].1));
    shell.key(Key::A, ctrl());
    shell.type_text("shortcuts");
    shell.key(Key::Enter, Modifiers::NONE);
    shell.settle();
    assert!(shell.has_visible_label(&slovak.text("shortcuts-title")));

    // A command that cannot run now says why, and Enter does not run it.
    let undo_steps = shell.app().undo_step_count();
    let reason = slovak.text("command-unavailable-nothing-to-undo");
    shell.key(Key::K, ctrl());
    shell.key(Key::A, ctrl());
    shell.type_text("undo");
    assert!(!shell.app().command_is_enabled(AppCommand::Undo));
    assert!(shell.has_visible_label(&reason));
    shell.key(Key::Enter, Modifiers::NONE);
    shell.settle();
    assert_eq!(shell.app().action_digest(), reason);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

#[test]
fn window_language_menu_switches_the_interface_to_slovak_and_back() {
    let mut shell = Shell::new();
    assert_eq!(
        shell.app().ui_language(),
        Some(ketchup_app::language::UiLanguage::English)
    );
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    let english_file = LocaleCatalog::english().text("menu-file");
    let slovak_file = LocaleCatalog::slovak().text("menu-file");
    assert_ne!(english_file, slovak_file);
    assert!(shell.has_visible_label(&english_file));

    for (name, language, visible, gone) in [
        (
            "Slovenčina",
            ketchup_app::language::UiLanguage::Slovak,
            &slovak_file,
            &english_file,
        ),
        (
            "English",
            ketchup_app::language::UiLanguage::English,
            &english_file,
            &slovak_file,
        ),
    ] {
        shell.open_menu("menu-window");
        // egui names a submenu button by its text and the arrow it draws.
        let submenu = format!("{} ⏵", shell.catalog().text("menu-language"));
        shell.click_button_label(&submenu);
        shell.settle();
        shell.click_role_and_label(Role::RadioButton, name);
        shell.settle();
        assert_eq!(shell.app().ui_language(), Some(language));
        assert!(shell.has_visible_label(visible), "{name}");
        assert!(!shell.has_visible_label(gone), "{name}");
        assert_eq!(
            shell.app().action_digest(),
            language.catalog().text("digest-language-changed")
        );
    }
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
}

/// A document with layers must show them at the top of the dock, above the
/// assistant and the outliner, so toggling a layer needs no scrolling.
#[test]
fn layers_lead_the_dock_once_the_document_has_a_layer() {
    let mut shell = Shell::new();
    let layers = shell.catalog().text("dock-tags");
    let outliner = shell.catalog().text("dock-outliner");
    let top_of = |shell: &Shell, label: &str| {
        shell
            .visible_accesskit_rects()
            .into_iter()
            .find(|(node, _)| {
                node.contains(&format!("label={label:?}"))
                    || node.contains(&format!("value={label:?}"))
            })
            .map(|(_, rect)| rect.top())
    };
    // Without layers the panel stays at the bottom, where it does not push the
    // assistant out of view.
    assert!(top_of(&shell, &layers).is_none_or(|layers_top| {
        top_of(&shell, &outliner).is_some_and(|outliner_top| layers_top > outliner_top)
    }));

    assert!(create_tag(
        shell.app_mut(),
        TagId(700),
        "Konštrukcia".to_owned(),
        true
    ));
    shell.settle();
    assert!(shell.app().document_snapshot().tag(TagId(700)).is_some());
    let layers_top = top_of(&shell, &layers).expect("layers header is in view");
    assert!(top_of(&shell, &outliner).is_none_or(|outliner_top| layers_top < outliner_top));
    assert!(layers_top < shell.viewport_rect().center().y);
}

#[test]
fn home_view_is_localized_accessible_framed_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::HomeView),
            shell.catalog().text("view-home")
        );

        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(1_000.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let projection = shell.app().projection_mode();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::HomeView));
        shell.click_command(AppCommand::HomeView);
        assert_eq!(
            shell.app().camera_orientation(),
            (-std::f32::consts::FRAC_PI_4, -3.0_f32.recip().sqrt().acos())
        );
        let home_zoom = shell.app().camera_zoom();
        let rect = shell.viewport_rect();
        for occurrence in [1, 2] {
            let (origin, size) = shell.app().occurrence_box_geometry(occurrence).unwrap();
            let center = shell.app().project_to_screen(
                origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5),
                rect,
            );
            assert!(rect.contains(center));
        }

        shell.click_menu_command("menu-view", AppCommand::ViewTop);
        shell.click_menu_command("menu-view", AppCommand::ZoomIn);
        shell.click_menu_command("menu-view", AppCommand::HomeView);
        assert_eq!(
            shell.app().camera_orientation(),
            (-std::f32::consts::FRAC_PI_4, -3.0_f32.recip().sqrt().acos())
        );
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);

        shell.click_menu_command("menu-view", AppCommand::ViewFront);
        shell.click_menu_command("menu-view", AppCommand::ZoomIn);
        shell.press_key(Key::Home);
        assert_eq!(
            shell.app().camera_orientation(),
            (-std::f32::consts::FRAC_PI_4, -3.0_f32.recip().sqrt().acos())
        );
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-home-view",
                &BTreeMap::from([("count", "2".to_owned())]),
            )
        );
        assert_eq!(shell.app().projection_mode(), projection);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn previous_view_is_localized_swappable_gesture_aware_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::PreviousView),
            shell.catalog().text("view-previous")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(1_000.0, 25.0, 0.0)));
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-view", AppCommand::ViewFront);
        shell.click_menu_command("menu-view", AppCommand::ViewProjection);
        shell.click_menu_command("menu-view", AppCommand::ZoomSelection);

        let rect = shell.viewport_rect();
        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let probe = origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5);
        let remembered_orientation = shell.app().camera_orientation();
        let remembered_projection = shell.app().projection_mode();
        let remembered_zoom = shell.app().camera_zoom();
        let remembered_probe = shell.app().project_to_screen(probe, rect);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::HomeView);
        let home_orientation = shell.app().camera_orientation();
        let home_projection = shell.app().projection_mode();
        let home_zoom = shell.app().camera_zoom();
        let home_probe = shell.app().project_to_screen(probe, rect);

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::PreviousView));
        shell.click_command(AppCommand::PreviousView);
        assert_eq!(shell.app().camera_orientation(), remembered_orientation);
        assert_eq!(shell.app().projection_mode(), remembered_projection);
        assert!((shell.app().camera_zoom() - remembered_zoom).abs() < 1.0e-5);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(remembered_probe)
                < 0.01
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-previous-view")
        );

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert_eq!(shell.app().camera_orientation(), home_orientation);
        assert_eq!(shell.app().projection_mode(), home_projection);
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(home_probe)
                < 0.01
        );

        shell.click_command(AppCommand::Orbit);
        shell.drag(rect.center(), rect.center() + Vec2::new(80.0, 40.0));
        assert_ne!(shell.app().camera_orientation(), home_orientation);
        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert_eq!(shell.app().camera_orientation(), home_orientation);

        shell.click_command(AppCommand::Pan);
        shell.drag(rect.center(), rect.center() + Vec2::new(80.0, 40.0));
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(home_probe)
                > 50.0
        );
        assert_eq!(shell.app().camera_orientation(), home_orientation);
        assert_eq!(shell.app().projection_mode(), home_projection);
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);
        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(home_probe)
                < 0.01
        );

        shell.scroll_at(rect.center(), 120.0);
        assert_ne!(shell.app().camera_zoom(), home_zoom);
        shell.app_mut().previous_view();
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn grid_axes_toggle_is_localized_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::GridAxes)),
            shell.catalog().text("view-grid-axes")
        );
        assert!(shell.app().view_visible(ViewFlag::GridAxes));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::View(ViewFlag::GridAxes)));
        shell.click_command(AppCommand::View(ViewFlag::GridAxes));
        assert!(!shell.app().view_visible(ViewFlag::GridAxes));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-grid-axes-hidden")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(shell.app().view_visible(ViewFlag::GridAxes));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-previous-view")
        );

        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::GridAxes));
        assert!(!shell.app().view_visible(ViewFlag::GridAxes));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::GridAxes));
        assert!(shell.app().view_visible(ViewFlag::GridAxes));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-grid-axes-shown")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn xray_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::View(ViewFlag::Xray)),
            shell.catalog().text("view-xray")
        );
        assert!(!shell.app().view_visible(ViewFlag::Xray));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Xray)));
        shell.click_command(AppCommand::View(ViewFlag::Xray));
        assert!(shell.app().view_visible(ViewFlag::Xray));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-xray-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Xray));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Xray));
        assert!(shell.app().view_visible(ViewFlag::Xray));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Xray));
        assert!(!shell.app().view_visible(ViewFlag::Xray));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-xray-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn white_background_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::WhiteBackground)),
            shell.catalog().text("view-white-background")
        );
        assert!(!shell.app().view_visible(ViewFlag::WhiteBackground));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::WhiteBackground)));
        shell.click_command(AppCommand::View(ViewFlag::WhiteBackground));
        assert!(shell.app().view_visible(ViewFlag::WhiteBackground));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-white-background-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::WhiteBackground));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::WhiteBackground));
        assert!(shell.app().view_visible(ViewFlag::WhiteBackground));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::WhiteBackground));
        assert!(!shell.app().view_visible(ViewFlag::WhiteBackground));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-white-background-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn shadows_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Shadows)),
            shell.catalog().text("view-shadows")
        );
        assert!(!shell.app().view_visible(ViewFlag::Shadows));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Shadows)));
        shell.click_command(AppCommand::View(ViewFlag::Shadows));
        assert!(shell.app().view_visible(ViewFlag::Shadows));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-shadows-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Shadows));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Shadows));
        assert!(shell.app().view_visible(ViewFlag::Shadows));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Shadows));
        assert!(!shell.app().view_visible(ViewFlag::Shadows));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-shadows-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn fog_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::View(ViewFlag::Fog)),
            shell.catalog().text("view-fog")
        );
        assert!(!shell.app().view_visible(ViewFlag::Fog));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Fog)));
        shell.click_command(AppCommand::View(ViewFlag::Fog));
        assert!(shell.app().view_visible(ViewFlag::Fog));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-fog-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Fog));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Fog));
        assert!(shell.app().view_visible(ViewFlag::Fog));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Fog));
        assert!(!shell.app().view_visible(ViewFlag::Fog));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-fog-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn shaded_command_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewShaded),
            shell.catalog().text("view-shaded")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::ViewShaded));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Xray));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Edges));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Wireframe));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Monochrome));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::HiddenLine));
        assert!(shell.app().view_visible(ViewFlag::Xray));
        assert!(!shell.app().view_visible(ViewFlag::Edges));
        assert!(shell.app().view_visible(ViewFlag::Wireframe));
        assert!(shell.app().view_visible(ViewFlag::Monochrome));
        assert!(shell.app().view_visible(ViewFlag::HiddenLine));

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewShaded));
        shell.click_command(AppCommand::ViewShaded);
        assert!(!shell.app().view_visible(ViewFlag::Wireframe));
        assert!(!shell.app().view_visible(ViewFlag::Monochrome));
        assert!(!shell.app().view_visible(ViewFlag::HiddenLine));
        assert!(shell.app().view_visible(ViewFlag::Xray));
        assert!(!shell.app().view_visible(ViewFlag::Edges));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-shaded-restored")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(shell.app().view_visible(ViewFlag::Wireframe));
        assert!(shell.app().view_visible(ViewFlag::Monochrome));
        assert!(shell.app().view_visible(ViewFlag::HiddenLine));
        assert!(shell.app().view_visible(ViewFlag::Xray));
        assert!(!shell.app().view_visible(ViewFlag::Edges));
        shell.click_menu_command("menu-view", AppCommand::ViewShaded);
        assert!(!shell.app().view_visible(ViewFlag::Wireframe));
        assert!(!shell.app().view_visible(ViewFlag::Monochrome));
        assert!(!shell.app().view_visible(ViewFlag::HiddenLine));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn wireframe_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Wireframe)),
            shell.catalog().text("view-wireframe")
        );
        assert!(!shell.app().view_visible(ViewFlag::Wireframe));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Wireframe)));
        shell.click_command(AppCommand::View(ViewFlag::Wireframe));
        assert!(shell.app().view_visible(ViewFlag::Wireframe));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-wireframe-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Wireframe));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Wireframe));
        assert!(shell.app().view_visible(ViewFlag::Wireframe));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Wireframe));
        assert!(!shell.app().view_visible(ViewFlag::Wireframe));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-wireframe-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn monochrome_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Monochrome)),
            shell.catalog().text("view-monochrome")
        );
        assert!(!shell.app().view_visible(ViewFlag::Monochrome));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Monochrome)));
        shell.click_command(AppCommand::View(ViewFlag::Monochrome));
        assert!(shell.app().view_visible(ViewFlag::Monochrome));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-monochrome-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Monochrome));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Monochrome));
        assert!(shell.app().view_visible(ViewFlag::Monochrome));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Monochrome));
        assert!(!shell.app().view_visible(ViewFlag::Monochrome));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-monochrome-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn hidden_line_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::HiddenLine)),
            shell.catalog().text("view-hidden-line")
        );
        assert!(!shell.app().view_visible(ViewFlag::HiddenLine));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::HiddenLine)));
        shell.click_command(AppCommand::View(ViewFlag::HiddenLine));
        assert!(shell.app().view_visible(ViewFlag::HiddenLine));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-hidden-line-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::HiddenLine));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::HiddenLine));
        assert!(shell.app().view_visible(ViewFlag::HiddenLine));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::HiddenLine));
        assert!(!shell.app().view_visible(ViewFlag::HiddenLine));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-hidden-line-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn edges_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::View(ViewFlag::Edges)),
            shell.catalog().text("view-edges")
        );
        assert!(shell.app().view_visible(ViewFlag::Edges));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Edges)));
        shell.click_command(AppCommand::View(ViewFlag::Edges));
        assert!(!shell.app().view_visible(ViewFlag::Edges));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-edges-hidden")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(shell.app().view_visible(ViewFlag::Edges));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Edges));
        assert!(!shell.app().view_visible(ViewFlag::Edges));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Edges));
        assert!(shell.app().view_visible(ViewFlag::Edges));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-edges-shown")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn profiles_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Profiles)),
            shell.catalog().text("view-profiles")
        );
        assert!(!shell.app().view_visible(ViewFlag::Profiles));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Profiles)));
        shell.click_command(AppCommand::View(ViewFlag::Profiles));
        assert!(shell.app().view_visible(ViewFlag::Profiles));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-profiles-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Profiles));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Profiles));
        assert!(shell.app().view_visible(ViewFlag::Profiles));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Profiles));
        assert!(!shell.app().view_visible(ViewFlag::Profiles));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-profiles-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn halos_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::View(ViewFlag::Halos)),
            shell.catalog().text("view-halos")
        );
        assert!(!shell.app().view_visible(ViewFlag::Halos));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Halos)));
        shell.click_command(AppCommand::View(ViewFlag::Halos));
        assert!(shell.app().view_visible(ViewFlag::Halos));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-halos-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Halos));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Halos));
        assert!(shell.app().view_visible(ViewFlag::Halos));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Halos));
        assert!(!shell.app().view_visible(ViewFlag::Halos));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-halos-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn depth_cue_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::DepthCue)),
            shell.catalog().text("view-depth-cue")
        );
        assert!(!shell.app().view_visible(ViewFlag::DepthCue));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::DepthCue)));
        shell.click_command(AppCommand::View(ViewFlag::DepthCue));
        assert!(shell.app().view_visible(ViewFlag::DepthCue));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-depth-cue-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::DepthCue));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::DepthCue));
        assert!(shell.app().view_visible(ViewFlag::DepthCue));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::DepthCue));
        assert!(!shell.app().view_visible(ViewFlag::DepthCue));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-depth-cue-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn fade_distant_edges_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::FadeDistantEdges)),
            shell.catalog().text("view-fade-distant-edges")
        );
        assert!(!shell.app().view_visible(ViewFlag::FadeDistantEdges));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::FadeDistantEdges)));
        shell.click_command(AppCommand::View(ViewFlag::FadeDistantEdges));
        assert!(shell.app().view_visible(ViewFlag::FadeDistantEdges));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-fade-distant-edges-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::FadeDistantEdges));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::FadeDistantEdges));
        assert!(shell.app().view_visible(ViewFlag::FadeDistantEdges));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::FadeDistantEdges));
        assert!(!shell.app().view_visible(ViewFlag::FadeDistantEdges));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-fade-distant-edges-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn high_contrast_edges_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::HighContrastEdges)),
            shell.catalog().text("view-high-contrast-edges")
        );
        assert!(!shell.app().view_visible(ViewFlag::HighContrastEdges));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::HighContrastEdges)));
        shell.click_command(AppCommand::View(ViewFlag::HighContrastEdges));
        assert!(shell.app().view_visible(ViewFlag::HighContrastEdges));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-high-contrast-edges-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::HighContrastEdges));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::HighContrastEdges));
        assert!(shell.app().view_visible(ViewFlag::HighContrastEdges));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::HighContrastEdges));
        assert!(!shell.app().view_visible(ViewFlag::HighContrastEdges));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-high-contrast-edges-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn selection_halo_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::SelectionHalo)),
            shell.catalog().text("view-selection-halo")
        );
        assert!(!shell.app().view_visible(ViewFlag::SelectionHalo));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::SelectionHalo)));
        shell.click_command(AppCommand::View(ViewFlag::SelectionHalo));
        assert!(shell.app().view_visible(ViewFlag::SelectionHalo));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-selection-halo-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::SelectionHalo));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::SelectionHalo));
        assert!(shell.app().view_visible(ViewFlag::SelectionHalo));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::SelectionHalo));
        assert!(!shell.app().view_visible(ViewFlag::SelectionHalo));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-selection-halo-hidden")
        );
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn endpoints_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Endpoints)),
            shell.catalog().text("view-endpoints")
        );
        assert!(!shell.app().view_visible(ViewFlag::Endpoints));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Endpoints)));
        shell.click_command(AppCommand::View(ViewFlag::Endpoints));
        assert!(shell.app().view_visible(ViewFlag::Endpoints));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-endpoints-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Endpoints));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Endpoints));
        assert!(shell.app().view_visible(ViewFlag::Endpoints));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Endpoints));
        assert!(!shell.app().view_visible(ViewFlag::Endpoints));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-endpoints-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn midpoints_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Midpoints)),
            shell.catalog().text("view-midpoints")
        );
        assert!(!shell.app().view_visible(ViewFlag::Midpoints));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Midpoints)));
        shell.click_command(AppCommand::View(ViewFlag::Midpoints));
        assert!(shell.app().view_visible(ViewFlag::Midpoints));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-midpoints-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Midpoints));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Midpoints));
        assert!(shell.app().view_visible(ViewFlag::Midpoints));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Midpoints));
        assert!(!shell.app().view_visible(ViewFlag::Midpoints));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-midpoints-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn extensions_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Extensions)),
            shell.catalog().text("view-extensions")
        );
        assert!(!shell.app().view_visible(ViewFlag::Extensions));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Extensions)));
        shell.click_command(AppCommand::View(ViewFlag::Extensions));
        assert!(shell.app().view_visible(ViewFlag::Extensions));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-extensions-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Extensions));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Extensions));
        assert!(shell.app().view_visible(ViewFlag::Extensions));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Extensions));
        assert!(!shell.app().view_visible(ViewFlag::Extensions));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-extensions-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn jitter_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Jitter)),
            shell.catalog().text("view-jitter")
        );
        assert!(!shell.app().view_visible(ViewFlag::Jitter));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Jitter)));
        shell.click_command(AppCommand::View(ViewFlag::Jitter));
        assert!(shell.app().view_visible(ViewFlag::Jitter));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-jitter-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Jitter));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Jitter));
        assert!(shell.app().view_visible(ViewFlag::Jitter));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Jitter));
        assert!(!shell.app().view_visible(ViewFlag::Jitter));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-jitter-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn dashes_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::Dashes)),
            shell.catalog().text("view-dashes")
        );
        assert!(!shell.app().view_visible(ViewFlag::Dashes));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::Dashes)));
        shell.click_command(AppCommand::View(ViewFlag::Dashes));
        assert!(shell.app().view_visible(ViewFlag::Dashes));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-dashes-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::Dashes));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Dashes));
        assert!(shell.app().view_visible(ViewFlag::Dashes));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::Dashes));
        assert!(!shell.app().view_visible(ViewFlag::Dashes));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-dashes-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn color_by_axis_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::ColorByAxis)),
            shell.catalog().text("view-color-by-axis")
        );
        assert!(!shell.app().view_visible(ViewFlag::ColorByAxis));
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::ColorByAxis)));
        shell.click_command(AppCommand::View(ViewFlag::ColorByAxis));
        assert!(shell.app().view_visible(ViewFlag::ColorByAxis));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-color-by-axis-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::ColorByAxis));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::ColorByAxis));
        assert!(shell.app().view_visible(ViewFlag::ColorByAxis));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::ColorByAxis));
        assert!(!shell.app().view_visible(ViewFlag::ColorByAxis));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-color-by-axis-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn hidden_objects_toggle_is_localized_non_interactive_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell
                .app()
                .command_label(AppCommand::View(ViewFlag::HiddenObjects)),
            shell.catalog().text("view-hidden-objects")
        );
        assert!(!shell.app().view_visible(ViewFlag::HiddenObjects));
        assert_eq!(shell.app().hidden_ghost_count(), 0);
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::View(ViewFlag::HiddenObjects))
        );

        let hidden_point = shell.top_face_centre(1);
        shell.click_at(hidden_point);
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        shell.click_menu_command("menu-view", AppCommand::Hide);
        assert_eq!(shell.app().hidden_occurrence_count(), 2);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::View(ViewFlag::HiddenObjects))
        );

        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(hidden_point);
        assert!(shell.offers(AppCommand::View(ViewFlag::HiddenObjects)));
        shell.click_command(AppCommand::View(ViewFlag::HiddenObjects));
        assert!(shell.app().view_visible(ViewFlag::HiddenObjects));
        assert_eq!(shell.app().hidden_ghost_count(), 2);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-hidden-objects-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_at(hidden_point);
        assert_eq!(shell.app().selected_occurrence_count(), 0);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().view_visible(ViewFlag::HiddenObjects));
        assert_eq!(shell.app().hidden_ghost_count(), 0);

        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::HiddenObjects));
        assert!(shell.app().view_visible(ViewFlag::HiddenObjects));
        shell.click_menu_command("menu-view", AppCommand::View(ViewFlag::HiddenObjects));
        assert!(!shell.app().view_visible(ViewFlag::HiddenObjects));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-hidden-objects-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn center_selection_is_localized_accessible_centered_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::CenterSelection),
            shell.catalog().text("view-center-selection")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::CenterSelection));

        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(1_000.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ViewFront);
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::CenterSelection));

        shell.open_menu("menu-view");
        assert!(shell.offers(AppCommand::CenterSelection));
        shell.press_key(Key::Escape);

        let rect = shell.viewport_rect();
        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let selected_center = origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5);
        assert!(
            shell
                .app()
                .project_to_screen(selected_center, rect)
                .distance(rect.center())
                > 1.0
        );
        let zoom = shell.app().camera_zoom();
        let orientation = shell.app().camera_orientation();
        let projection = shell.app().projection_mode();
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::CenterSelection));
        shell.click_command(AppCommand::CenterSelection);
        let centered_distance = shell
            .app()
            .project_to_screen(selected_center, rect)
            .distance(rect.center());
        assert!(
            centered_distance < 0.01,
            "selection centre remained {centered_distance} px from the viewport centre: {}",
            shell.app().action_digest()
        );
        assert!((shell.app().camera_zoom() - zoom).abs() < 1.0e-5);
        assert_eq!(shell.app().camera_orientation(), orientation);
        assert_eq!(shell.app().projection_mode(), projection);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-center-selection",
                &BTreeMap::from([("count", "1".to_owned())]),
            )
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn zoom_window_is_localized_bounded_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ZoomWindow),
            shell.catalog().text("view-zoom-window")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.open_menu("menu-view");
        assert!(shell.offers(AppCommand::ZoomWindow));
        shell.press_key(Key::Escape);

        let rect = shell.viewport_rect();
        let zoom = shell.app().camera_zoom();
        let orientation = shell.app().camera_orientation();
        let projection = shell.app().projection_mode();
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::ZoomWindow);
        shell.drag(rect.center(), rect.center() + Vec2::new(3.0, 3.0));
        assert!((shell.app().camera_zoom() - zoom).abs() < 1.0e-5);
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));
        shell.press_key(Key::Escape);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-cancelled")
        );

        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let probe = origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5);
        let probe_before = shell.app().project_to_screen(probe, rect);
        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::ZoomWindow));
        shell.click_command(AppCommand::ZoomWindow);
        shell.drag(
            probe_before - Vec2::new(80.0, 60.0),
            probe_before + Vec2::new(80.0, 60.0),
        );

        assert!(shell.app().camera_zoom() > zoom);
        assert_eq!(shell.app().camera_orientation(), orientation);
        assert_eq!(shell.app().projection_mode(), projection);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(rect.center())
                < 0.01
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-zoom-window")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));
        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!((shell.app().camera_zoom() - zoom).abs() < 1.0e-5);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(probe_before)
                < 0.01
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn standard_orthographic_views_are_localized_accessible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        for (command, key) in [
            (AppCommand::ViewTop, "view-top"),
            (AppCommand::ViewBottom, "view-bottom"),
            (AppCommand::ViewFront, "view-front"),
            (AppCommand::ViewBack, "view-back"),
            (AppCommand::ViewRight, "view-right"),
            (AppCommand::ViewLeft, "view-left"),
        ] {
            assert_eq!(
                shell.app().command_label(command),
                shell.catalog().text(key)
            );
            assert!(shell.offers(command));
            shell.click_command(command);
            assert_eq!(
                shell.app().action_digest(),
                shell.catalog().format(
                    "digest-view-changed",
                    &BTreeMap::from([("view", shell.catalog().text(key))]),
                )
            );
            shell.click_menu_command("menu-view", command);
            assert_eq!(
                shell.app().action_digest(),
                shell.catalog().format(
                    "digest-view-changed",
                    &BTreeMap::from([("view", shell.catalog().text(key))]),
                )
            );
            assert_eq!(shell.app().document_revision(), revision);
            assert_eq!(shell.app().canonical_digest(), digest);
            assert_eq!(shell.app().undo_step_count(), undo_steps);
        }
    }
}

#[test]
fn zoom_selection_is_localized_selection_bound_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ZoomSelection),
            shell.catalog().text("view-zoom-selection")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::ZoomSelection));

        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(1_000.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        let all_zoom = shell.app().camera_zoom();
        shell.click_at(shell.top_face_centre(1));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().command_is_enabled(AppCommand::ZoomSelection));

        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-view", AppCommand::ZoomSelection);

        assert!(shell.app().camera_zoom() > all_zoom * 2.0);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-zoom-selection",
                &BTreeMap::from([("count", "1".to_owned())]),
            )
        );
        let rect = shell.viewport_rect();
        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let selected_center = shell.app().project_to_screen(
            origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5),
            rect,
        );
        assert!(selected_center.distance(rect.center()) < rect.width() * 0.1);
        let (origin, size) = shell.app().occurrence_box_geometry(2).unwrap();
        let unselected_center = shell.app().project_to_screen(
            origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5),
            rect,
        );
        assert!(!rect.expand(5.0).contains(unselected_center));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert!(!shell.app().command_is_enabled(AppCommand::ZoomSelection));
    }
}

#[test]
fn zoom_steps_are_localized_keyboard_accessible_bounded_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ZoomIn),
            shell.catalog().text("view-zoom-in")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::ZoomOut),
            shell.catalog().text("view-zoom-out")
        );
        shell.open_menu("menu-view");
        assert!(shell.offers(AppCommand::ZoomIn));
        assert!(shell.offers(AppCommand::ZoomOut));
        shell.press_key(Key::Escape);

        let projection = shell.app().projection_mode();
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let initial_zoom = shell.app().camera_zoom();

        shell.click_menu_command("menu-view", AppCommand::ZoomIn);
        assert!((shell.app().camera_zoom() - initial_zoom * 1.25).abs() < 1.0e-5);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-zoom-in")
        );
        shell.click_menu_command("menu-view", AppCommand::ZoomOut);
        assert!((shell.app().camera_zoom() - initial_zoom).abs() < 1.0e-5);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-zoom-out")
        );

        shell.key(Key::Plus, ctrl());
        assert!((shell.app().camera_zoom() - initial_zoom * 1.25).abs() < 1.0e-5);
        shell.key(Key::Minus, ctrl());
        assert!((shell.app().camera_zoom() - initial_zoom).abs() < 1.0e-5);

        assert_eq!(shell.app().projection_mode(), projection);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn status_bar_shows_the_exact_build_version() {
    let shell = Shell::new();

    assert!(shell.has_visible_label(&format!("v{}", KetchupApp::build_version())));
}

#[test]
fn help_about_is_localized_informative_and_presentation_only() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let version = shell.catalog().format(
            "about-version",
            &BTreeMap::from([("version", KetchupApp::build_version().to_owned())]),
        );
        let license = shell.catalog().format(
            "about-license",
            &BTreeMap::from([("license", env!("CARGO_PKG_LICENSE").to_owned())]),
        );
        let close = shell.catalog().text("about-close");

        assert_eq!(
            shell.app().command_label(AppCommand::About),
            shell.catalog().text("help-about")
        );
        shell.click_menu_command("menu-help", AppCommand::About);
        assert!(shell.app().about_visible());
        assert!(shell.has_visible_label(&version));
        assert!(shell.has_visible_label(&license));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_role_and_label(Role::Button, &close);
        assert!(!shell.app().about_visible());
        shell.click_menu_command("menu-help", AppCommand::About);
        assert!(shell.app().about_visible());
        shell.key(Key::N, ctrl());
        assert!(shell.app().about_visible());
        assert!(shell.has_visible_label(&version));
        assert!(shell.has_visible_label(&license));
    }
}

#[test]
fn rename_occurrence_is_localized_selection_bound_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let occurrence_id = OccurrenceId(1);
        let original_name = shell
            .app()
            .occurrence_name(occurrence_id)
            .expect("the initial occurrence exists");
        assert_eq!(
            shell.app().command_label(AppCommand::RenameOccurrence),
            shell.catalog().text("model-rename-occurrence")
        );

        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        assert!(!shell.app().rename_occurrence_visible());

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let input_label = shell.catalog().text("dialog-rename-occurrence-name");
        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(
            shell.app().rename_occurrence_input(),
            Some(original_name.as_str())
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-confirm"),
        );
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-cancel"),
        );
        assert!(!shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().document_revision(), revision);

        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Selection drift");
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_occurrence_rename());
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));

        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.press_key(Key::Backspace);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-confirm"),
        );
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        let renamed = "Housing instance";
        shell.focus_text_input(&input_label);
        shell.type_text(renamed);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-confirm"),
        );
        assert!(!shell.app().rename_occurrence_visible());
        assert_eq!(
            shell.app().occurrence_name(occurrence_id),
            Some(renamed.to_owned())
        );
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-renamed-occurrence",
                &BTreeMap::from([("name", renamed.to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell.app().occurrence_name(occurrence_id),
            Some(original_name.clone())
        );

        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Stale rename");
        assert!(shell.app_mut().create_box());
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_occurrence_rename());
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        assert_eq!(
            shell.app().occurrence_name(occurrence_id),
            Some(original_name.clone())
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Missing occurrence");
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_occurrence_rename());
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().occurrence_name(occurrence_id), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn replace_component_is_localized_state_preserving_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ReplaceComponent),
            shell.catalog().text("model-replace-component")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::ReplaceComponent));
        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        assert!(!shell.app().component_replacement_visible());

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert!(shell.app_mut().create_box());
        let occurrence_id = OccurrenceId(2);
        let original_definition = DefinitionId(2);
        let replacement_definition = DefinitionId(1);
        let original_name = shell.app().occurrence_name(occurrence_id).unwrap();
        let original_geometry = shell
            .app()
            .occurrence_box_geometry(occurrence_id.0)
            .unwrap();
        assert_eq!(
            shell.app().occurrence_definition_id(occurrence_id),
            Some(original_definition)
        );
        assert!(shell.app().command_is_enabled(AppCommand::ReplaceComponent));
        assert!(shell.app_mut().set_selected_occurrence_grounded(true));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        assert_eq!(
            shell.app().component_replacement_input(),
            Some((occurrence_id, replacement_definition))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-replace-component-cancel"),
        );
        assert!(!shell.app().component_replacement_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-replace-component-confirm"),
        );
        assert!(!shell.app().component_replacement_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().grounded_occurrence_count(), 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(occurrence_id));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().occurrence_name(occurrence_id),
            Some(original_name)
        );
        assert_eq!(
            shell.app().occurrence_box_geometry(occurrence_id.0),
            Some(original_geometry)
        );
        assert_eq!(
            shell.app().occurrence_definition_id(occurrence_id),
            Some(replacement_definition)
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-replaced-component",
                &BTreeMap::from([(
                    "name",
                    shell.app().definition_name(replacement_definition).unwrap(),
                )]),
            )
        );
        let replaced_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell.app().occurrence_definition_id(occurrence_id),
            Some(original_definition)
        );
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), replaced_digest);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_component_replacement());
        assert!(shell.app().component_replacement_visible());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().occurrence_definition_id(occurrence_id),
            Some(replacement_definition)
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-replace-component-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        let (missing_occurrence, _) = shell.app().component_replacement_input().unwrap();
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_component_replacement());
        assert!(shell.app().component_replacement_visible());
        assert_eq!(
            shell.app().occurrence_definition_id(missing_occurrence),
            None
        );
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn replace_component_rejects_stale_confirmation_without_mutation() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app_mut().create_box());
        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        assert!(shell.app().component_replacement_visible());
        assert!(shell.app_mut().rotate_selected(30.0));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-replace-component-confirm"),
        );
        assert!(shell.app().component_replacement_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(DefinitionId(2))
        );
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
    }
}

#[test]
fn deselect_is_localized_exact_shortcut_parity_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::Deselect),
            shell.catalog().text("action-deselect")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Deselect));

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::Deselect));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.press_key(Key::Escape);

        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-selection-cleared")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert!(!shell.app().command_is_enabled(AppCommand::Deselect));
        let action_digest = shell.app().action_digest().to_owned();

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().action_digest(), action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        let action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().action_digest(), action_digest);

        assert!(shell.app_mut().create_box());
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::Group);
        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Deselect));
        let group_revision = shell.app().document_revision();
        let group_digest = shell.app().canonical_digest();
        let group_undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-edit", AppCommand::Deselect);

        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().document_revision(), group_revision);
        assert_eq!(shell.app().canonical_digest(), group_digest);
        assert_eq!(shell.app().undo_step_count(), group_undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-selection-cleared")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Deselect));
    }
}

#[test]
fn select_all_is_localized_exact_shortcut_parity_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::SelectAll),
            shell.catalog().text("action-select-all")
        );
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert!(shell.app_mut().create_box());
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert!(shell.app().command_is_enabled(AppCommand::SelectAll));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.key(Key::A, ctrl());

        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let expected_action_digest = shell.catalog().format(
            "digest-selected-all",
            &BTreeMap::from([("count", "2".to_owned())]),
        );
        assert_eq!(shell.app().action_digest(), expected_action_digest);
        assert!(!shell.app().command_is_enabled(AppCommand::SelectAll));
        shell.key(Key::A, ctrl());
        assert_eq!(shell.app().action_digest(), expected_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().action_digest(), expected_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 3);
    }
}

#[test]
fn invert_selection_is_localized_scope_exact_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::InvertSelection),
            shell.catalog().text("action-invert-selection")
        );
        assert!(shell.app().command_is_enabled(AppCommand::InvertSelection));
        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::InvertSelection));
        shell.click_command(AppCommand::InvertSelection);

        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-inverted-selection",
                &BTreeMap::from([("count", "1".to_owned())]),
            )
        );

        shell.key(Key::I, ctrl());
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(2)));
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_menu_command("menu-edit", AppCommand::InvertSelection);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell.key(Key::I, ctrl());
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

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
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        context_shell.click_menu_command("menu-edit", AppCommand::InvertSelection);
        assert_eq!(context_shell.app().selected_occurrence_count(), 1);
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.key(Key::I, ctrl());
        assert_eq!(context_shell.app().selected_occurrence_count(), 0);
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
    }
}

#[test]
fn cut_is_localized_atomic_pasteable_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Cut),
            shell.catalog().text("action-cut")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Cut));
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app_mut().create_box());
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Cut));
        let before_cut = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-edit", AppCommand::Cut);

        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-cut-to-clipboard",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Cut));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let cut_digest = shell.app().canonical_digest();
        let cut_action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-edit", AppCommand::Cut);
        assert_eq!(shell.app().canonical_digest(), cut_digest);
        assert_eq!(shell.app().action_digest(), cut_action_digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().canonical_digest(), before_cut);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), cut_digest);

        let paste_revision = shell.app().document_revision();
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().document_revision(), paste_revision + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-pasted-from-clipboard",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), cut_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_count(), 2);

        let mut group_shell = Shell::with_catalog(catalog.clone());
        assert!(group_shell.app_mut().create_box());
        group_shell.settle();
        group_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        group_shell.click_menu_command("menu-model", AppCommand::Group);
        assert!(!group_shell.app().command_is_enabled(AppCommand::Cut));
        let group_revision = group_shell.app().document_revision();
        let group_digest = group_shell.app().canonical_digest();
        group_shell.click_menu_command("menu-edit", AppCommand::Cut);
        assert_eq!(group_shell.app().document_revision(), group_revision);
        assert_eq!(group_shell.app().canonical_digest(), group_digest);

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
        context_shell.click_at(context_shell.top_face_centre(2));
        assert!(!context_shell.app().command_is_enabled(AppCommand::Cut));
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        context_shell.click_menu_command("menu-edit", AppCommand::Cut);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
    }
}

#[test]
fn duplicate_is_localized_atomic_clipboard_preserving_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Duplicate),
            shell.catalog().text("action-duplicate")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Duplicate));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app_mut().create_box());
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Duplicate));
        let before_duplicate = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.key(Key::D, ctrl());

        assert_eq!(shell.app().occurrence_count(), 4);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-duplicated-selection",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        let duplicated = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before_duplicate);
        assert_eq!(shell.app().occurrence_count(), 2);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);

        shell.click_menu_command("menu-edit", AppCommand::Duplicate);
        assert_eq!(shell.app().canonical_digest(), duplicated);
        assert_eq!(shell.app().occurrence_count(), 4);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before_duplicate);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), duplicated);
        assert_eq!(shell.app().occurrence_count(), 4);

        let paste_revision = shell.app().document_revision();
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 5);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), paste_revision + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-pasted-from-clipboard",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );

        let mut group_shell = Shell::with_catalog(catalog.clone());
        assert!(group_shell.app_mut().create_box());
        group_shell.settle();
        group_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        group_shell.click_menu_command("menu-model", AppCommand::Group);
        assert!(!group_shell.app().command_is_enabled(AppCommand::Duplicate));
        let group_revision = group_shell.app().document_revision();
        let group_digest = group_shell.app().canonical_digest();
        let group_undo_steps = group_shell.app().undo_step_count();
        group_shell.click_menu_command("menu-edit", AppCommand::Duplicate);
        assert_eq!(group_shell.app().document_revision(), group_revision);
        assert_eq!(group_shell.app().canonical_digest(), group_digest);
        assert_eq!(group_shell.app().undo_step_count(), group_undo_steps);

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
        context_shell.click_at(context_shell.top_face_centre(2));
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::Duplicate)
        );
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        context_shell.key(Key::D, ctrl());
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
    }
}

#[test]
fn copy_paste_is_localized_atomic_stale_safe_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Copy),
            shell.catalog().text("action-copy")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::Paste),
            shell.catalog().text("action-paste")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Copy));
        assert!(!shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::Copy));
        let copy_revision = shell.app().document_revision();
        let copy_digest = shell.app().canonical_digest();
        let copy_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-copied-to-clipboard",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );
        assert_eq!(shell.app().document_revision(), copy_revision);
        assert_eq!(shell.app().canonical_digest(), copy_digest);
        assert_eq!(shell.app().undo_step_count(), copy_undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert!(shell.app_mut().create_box());
        shell.settle();

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-copied-to-clipboard",
                &BTreeMap::from([("count", "3".to_owned())])
            )
        );
        let before_paste = shell.app().canonical_digest();
        let paste_revision = shell.app().document_revision();
        let paste_undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-edit", AppCommand::Paste);

        assert_eq!(shell.app().occurrence_count(), 6);
        assert_eq!(shell.app().document_revision(), paste_revision + 1);
        assert_eq!(shell.app().undo_step_count(), paste_undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-pasted-from-clipboard",
                &BTreeMap::from([("count", "3".to_owned())])
            )
        );
        let pasted = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_count(), 3);
        assert_eq!(shell.app().canonical_digest(), before_paste);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_count(), 6);
        assert_eq!(shell.app().canonical_digest(), pasted);

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        assert_eq!(shell.app().occurrence_count(), 5);
        assert!(!shell.app().command_is_enabled(AppCommand::Paste));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 5);
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);

        let mut group_shell = Shell::with_catalog(catalog.clone());
        group_shell.click_at(group_shell.top_face_centre(1));
        group_shell.click_menu_command("menu-edit", AppCommand::Copy);
        group_shell.click_menu_command("menu-edit", AppCommand::Paste);
        group_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        group_shell.click_menu_command("menu-model", AppCommand::Group);
        assert_eq!(group_shell.app().group_count(), 1);
        assert!(!group_shell.app().command_is_enabled(AppCommand::Copy));
        let group_digest = group_shell.app().action_digest().to_owned();
        group_shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert_eq!(group_shell.app().action_digest(), group_digest);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.click_at(context_shell.top_face_centre(1));
        context_shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(context_shell.app().command_is_enabled(AppCommand::Paste));
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        assert!(!context_shell.app().command_is_enabled(AppCommand::Copy));
        assert!(!context_shell.app().command_is_enabled(AppCommand::Paste));
        let revision = context_shell.app().document_revision();
        let digest = context_shell.app().canonical_digest();
        let undo_steps = context_shell.app().undo_step_count();
        let action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(context_shell.app().document_revision(), revision);
        assert_eq!(context_shell.app().canonical_digest(), digest);
        assert_eq!(context_shell.app().undo_step_count(), undo_steps);
        assert_eq!(context_shell.app().action_digest(), action_digest);
    }
}

/// Review 2026-10-09 (P3): a greyed-out tool showed nothing on hover.
#[test]
fn an_unavailable_tool_says_so_on_hover() {
    let mut shell = Shell::new();
    assert!(!shell.app().command_is_enabled(AppCommand::PlanarOffset));
    shell.hover(shell.command_rect(AppCommand::PlanarOffset).center());
    let label = shell.app().command_label(AppCommand::PlanarOffset);
    let tooltip = shell.catalog().format(
        "tool-tooltip-unavailable",
        &BTreeMap::from([("tool", label.clone()), ("shortcut", "\u{1}".to_owned())]),
    );
    let (_, why) = tooltip.split_once('\u{1}').unwrap();
    let visible = shell.visible_accesskit_rects();
    assert!(
        visible
            .iter()
            .any(|(text, _)| text.contains(&label) && text.contains(why)),
        "{why} not in {visible:?}"
    );
}

/// Review 2026-10-09 (P3): Ctrl+V pasted the old parts even after text was
/// copied in another program.
#[test]
fn ctrl_v_does_not_paste_old_parts_over_text_copied_elsewhere() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    shell.native_copy();
    assert_eq!(shell.app().occurrence_count(), 1);

    let revision = shell.app().document_revision();
    shell.native_paste_text("a line from a text editor");
    assert_eq!(shell.app().occurrence_count(), 1);
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("digest-clipboard-holds-other-content")
    );

    shell.native_paste();
    assert_eq!(shell.app().occurrence_count(), 2);
}
