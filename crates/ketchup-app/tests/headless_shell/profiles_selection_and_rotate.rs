use super::*;

#[test]
fn parameter_expression_recomputes_dependents_atomically_and_round_trips_through_the_shell() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("parametric-fixture.ketchup");
    let saved = directory.path().join("parametric-saved.ketchup");
    write_parametric_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [20.0, 30.0, 10.0]]]
    );

    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    assert!(!shell.app().can_undo());
    replace_parameter_expression(&mut shell, "$301 * 2");

    let changed_revision = shell.app().document_revision();
    let changed_digest = shell.app().canonical_digest();
    assert_eq!(changed_revision, initial_revision + 1);
    assert_ne!(changed_digest, initial_digest);
    assert_eq!(
        shell.app().parameter_last_recomputed_nodes(),
        &BTreeSet::from([PARAMETRIC_RULE, PARAMETRIC_DEPENDENT])
    );
    assert!(
        !shell
            .app()
            .parameter_last_recomputed_nodes()
            .contains(&PARAMETRIC_UNRELATED_SOURCE)
    );
    assert!(
        !shell
            .app()
            .parameter_last_recomputed_nodes()
            .contains(&PARAMETRIC_UNRELATED)
    );
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(PARAMETRIC_PROFILE)
            .unwrap()
            .kind().polygon_points(),
        Some(ref points_mm)
            if points_mm == &vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]
    ));
    let report = shell
        .app()
        .document_snapshot()
        .evaluate(&EvaluationIdentity::default())
        .unwrap();
    assert!(matches!(
        report.node(PARAMETRIC_DEPENDENT).unwrap().status,
        EvaluationStatus::Evaluated(value) if (value - 41.0).abs() < f64::EPSILON
    ));
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [40.0, 30.0, 10.0]]]
    );

    replace_parameter_expression(&mut shell, "(");
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    replace_parameter_expression(&mut shell, "$303");
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert_eq!(shell.app().canonical_digest(), changed_digest);

    let input = shell.catalog().text("parameters-expression");
    let apply = shell.catalog().text("parameters-apply");
    shell.focus_text_input(&input);
    shell.key(Key::A, ctrl());
    shell.type_text("$301 * 3");
    shell.click_command(AppCommand::Circle);
    shell.click_at(
        shell
            .app()
            .viewport_position(Vec3::new(10.0, 15.0, 10.0))
            .unwrap(),
    );
    shell.click_at(
        shell
            .app()
            .viewport_position(Vec3::new(15.0, 15.0, 10.0))
            .unwrap(),
    );
    let intervening_revision = shell.app().document_revision();
    let intervening_digest = shell.app().canonical_digest();
    assert_eq!(intervening_revision, changed_revision + 1);
    shell.click_row(&apply);
    assert_eq!(shell.app().document_revision(), intervening_revision);
    assert_eq!(shell.app().canonical_digest(), intervening_digest);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("error-parameter-stale")
    );
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .evaluator_node(PARAMETRIC_RULE)
            .unwrap()
            .kind(),
        EvaluatorNodeKind::Rule { source, .. } if source == "$301 * 2"
    ));
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), changed_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert!(
        !shell.app().can_undo(),
        "the valid edit must be one undo step"
    );
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [20.0, 30.0, 10.0]]]
    );
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [40.0, 30.0, 10.0]]]
    );

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert!(shell.app().can_undo());
    let reopened = shell.app().document_snapshot();
    assert!(matches!(
        reopened.evaluator_node(PARAMETRIC_RULE).unwrap().kind(),
        EvaluatorNodeKind::Rule { source, .. } if source == "$301 * 2"
    ));
    assert!(matches!(
        reopened.feature(PARAMETRIC_PROFILE).unwrap().kind().polygon_points(),
        Some(ref points_mm)
            if points_mm == &vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]
    ));
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [40.0, 30.0, 10.0]]]
    );

    for (locale, catalog) in [
        ("en-US", LocaleCatalog::english()),
        ("sk-SK", LocaleCatalog::slovak()),
        ("pseudo", LocaleCatalog::pseudo()),
    ] {
        let dialogs = ScriptedFileDialogs::new()
            .queue_open(&fixture)
            .always_discard();
        let mut localized = Shell::with_catalog_and_dialogs(catalog, dialogs);
        localized.click_menu_command("menu-file", AppCommand::Open);
        let selector = localized.catalog().text("parameters-node");
        let input = localized.catalog().text("parameters-expression");
        let apply = localized.catalog().text("parameters-apply");
        assert!(
            localized.has_role_and_label(Role::ComboBox, &selector),
            "{locale} selector"
        );
        assert!(
            localized.has_role_and_label(Role::TextInput, &input),
            "{locale} input"
        );
        assert!(
            localized.has_role_and_label(Role::Button, &apply),
            "{locale} apply"
        );
        localized.focus_combo_box(&selector);
        localized.focus_text_input(&input);
    }
}

#[test]
fn every_tool_in_the_rail_is_reachable_by_its_accessible_name() {
    let shell = Shell::new();

    for command in [
        AppCommand::Select,
        AppCommand::Line,
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
            "{command:?} paints an icon and must still expose an accessible name"
        );
    }
}

#[test]
fn circle_center_radius_is_exact_snapped_undoable_and_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("circles.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let center = Vec3::new(35.0, 25.0, 20.0);
    let center_screen = shell.app().viewport_position(center).unwrap();
    let radial_screen = shell
        .app()
        .viewport_position(Vec3::new(55.0, 25.0, 20.0))
        .unwrap();

    shell.click_command(AppCommand::Circle);
    shell.click_at(center_screen);
    shell.move_pointer(radial_screen);

    let (preview_center, preview_radius) = shell.app().circle_preview_geometry().unwrap();
    assert!(
        preview_center.distance(center) < 1.0e-5,
        "preview center {preview_center:?}, expected {center:?}"
    );
    let center = preview_center;
    assert!((preview_radius - 20.0).abs() < 1.0e-5);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.type_text("12.5");
    shell.press_key(Key::Enter);

    let first_digest = shell.app().canonical_digest();
    let (committed_center, committed_radius) = shell.app().latest_circle_geometry().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().circle_profile_count(), 1);
    assert!(committed_center.distance(center) < 1.0e-6);
    assert!((committed_radius - 12.5).abs() < 1.0e-12);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().circle_profile_count(), 0);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_digest);
    assert_eq!(shell.app().circle_profile_count(), 1);

    shell.click_command(AppCommand::Circle);
    let center_screen = shell.app().viewport_position(center).unwrap();
    shell.click_at(center_screen + Vec2::new(3.0, 0.0));
    let (snapped_center, _) = shell.app().circle_preview_geometry().unwrap();
    assert!(
        snapped_center.distance(center) < 1.0e-6,
        "existing circle centre snap"
    );

    let endpoint = Vec3::new(100.0, 60.0, 20.0);
    let endpoint_screen = shell.app().viewport_position(endpoint).unwrap();
    let viewport_center = shell.viewport_rect().center();
    let endpoint_screen = endpoint_screen + (viewport_center - endpoint_screen).normalized() * 2.0;
    shell.move_pointer(endpoint_screen);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    shell.click_at(endpoint_screen);

    let expected_radius = (endpoint.x - center.x).hypot(endpoint.y - center.y);
    let (second_center, second_radius) = shell.app().latest_circle_geometry().unwrap();
    assert_eq!(shell.app().circle_profile_count(), 2);
    assert!(second_center.distance(center) < 1.0e-6);
    assert!((second_radius - expected_radius).abs() < 1.0e-12);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(shell.app().circle_profile_count(), 0);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().circle_profile_count(), 2);
    assert_eq!(
        shell.app().latest_circle_geometry(),
        Some((second_center, second_radius))
    );
}

#[test]
fn arc_endpoint_bulge_is_exact_snapped_undoable_and_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("arcs.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let circle_center = Vec3::new(35.0, 25.0, 20.0);
    let circle_radius = 12.5;
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(circle_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(circle_center + Vec3::new(circle_radius, 0.0, 0.0))
            .unwrap(),
    );
    assert_eq!(shell.app().circle_profile_count(), 1);
    let (circle_center, circle_radius) = shell.app().latest_circle_geometry().unwrap();

    let before_arc_revision = shell.app().document_revision();
    let before_arc_digest = shell.app().canonical_digest();
    shell.click_command(AppCommand::Arc);
    let center_screen = shell.app().viewport_position(circle_center).unwrap();
    shell.move_pointer(center_screen + Vec2::new(3.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Center));
    shell.click_at(center_screen + Vec2::new(3.0, 0.0));

    let midpoint = Vec3::new(50.0, 0.0, 20.0);
    let midpoint_screen = shell.app().viewport_position(midpoint).unwrap();
    shell.move_pointer(midpoint_screen);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Midpoint));
    shell.click_at(midpoint_screen);

    let chord = midpoint - circle_center;
    let chord_length = chord.x.hypot(chord.y);
    let chord_midpoint = (circle_center + midpoint) * 0.5;
    let normal = Vec3::new(-chord.y / chord_length, chord.x / chord_length, 0.0);
    let preview_bulge = chord_midpoint + normal * 8.0;
    shell.move_pointer(shell.app().viewport_position(preview_bulge).unwrap());
    let (preview_start, preview_end, _, _) = shell.app().arc_preview_geometry().unwrap();
    assert!(preview_start.distance(circle_center) < 1.0e-6);
    assert!(preview_end.distance(midpoint) < 1.0e-6);
    assert_eq!(shell.app().document_revision(), before_arc_revision);
    assert_eq!(shell.app().canonical_digest(), before_arc_digest);

    shell.type_text("12.5");
    shell.press_key(Key::Enter);
    let first_arc_digest = shell.app().canonical_digest();
    let (start, end, center, _) = shell.app().latest_arc_geometry().unwrap();
    let exact_bulge = chord_midpoint + normal * 12.5;
    assert_eq!(shell.app().document_revision(), before_arc_revision + 1);
    assert_eq!(shell.app().arc_profile_count(), 1);
    assert!(start.distance(circle_center) < 1.0e-6);
    assert!(end.distance(midpoint) < 1.0e-6);
    assert!((center.distance(start) - center.distance(exact_bulge)).abs() < 1.0e-8);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_arc_digest);
    assert_eq!(shell.app().arc_profile_count(), 0);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_arc_digest);
    assert_eq!(shell.app().arc_profile_count(), 1);

    shell.click_command(AppCommand::Arc);
    let endpoint = Vec3::new(0.0, 0.0, 20.0);
    let endpoint_screen = shell.app().viewport_position(endpoint).unwrap();
    let endpoint_screen =
        endpoint_screen + (shell.viewport_rect().center() - endpoint_screen).normalized() * 2.0;
    shell.move_pointer(endpoint_screen);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    shell.click_at(endpoint_screen);

    let delta = endpoint - circle_center;
    let distance_squared = delta.x * delta.x + delta.y * delta.y;
    let base_scale = circle_radius * circle_radius / distance_squared;
    let tangent_scale = circle_radius * (distance_squared - circle_radius * circle_radius).sqrt()
        / distance_squared;
    let tangent = Vec3::new(
        circle_center.x + delta.x * base_scale - delta.y * tangent_scale,
        circle_center.y + delta.y * base_scale + delta.x * tangent_scale,
        20.0,
    );
    let tangent_screen = shell.app().viewport_position(tangent).unwrap();
    shell.move_pointer(tangent_screen);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Tangent));
    shell.click_at(tangent_screen);

    let tangent_chord = tangent - endpoint;
    let tangent_length = tangent_chord.x.hypot(tangent_chord.y);
    let tangent_midpoint = (endpoint + tangent) * 0.5;
    let tangent_normal = Vec3::new(
        -tangent_chord.y / tangent_length,
        tangent_chord.x / tangent_length,
        0.0,
    );
    shell.click_at(
        shell
            .app()
            .viewport_position(tangent_midpoint + tangent_normal * 6.0)
            .unwrap(),
    );
    assert_eq!(shell.app().arc_profile_count(), 2);
    let (_, snapped_tangent, _, _) = shell.app().latest_arc_geometry().unwrap();
    assert!(snapped_tangent.distance(tangent) < 1.0e-6);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(shell.app().arc_profile_count(), 0);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().circle_profile_count(), 1);
    assert_eq!(shell.app().arc_profile_count(), 2);
}

#[test]
fn empty_document_refuses_push_pull_without_mutation_and_remains_undoable() {
    let mut shell = Shell::new();
    let initial_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-edit", AppCommand::Delete);
    assert_eq!(shell.app().occurrence_count(), 0);
    let empty_revision = shell.app().document_revision();
    let empty_digest = shell.app().canonical_digest();
    let empty_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("15");
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().occurrence_count(), 0);
    assert_eq!(shell.app().document_revision(), empty_revision);
    assert_eq!(shell.app().canonical_digest(), empty_digest);
    assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("error-push-pull-selection-required")
    );

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), empty_digest);
}

#[test]
fn push_pull_without_a_selected_face_never_targets_the_initial_box() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("15");
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().document_height_mm(), 20.0);
    assert!(!shell.app().can_undo());
    assert!(!shell.app().has_drawn_shape_preview());
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("error-push-pull-selection-required")
    );
}

#[test]
fn localized_typed_push_pull_of_a_drawn_circle_mills_the_part_in_one_step() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 1);
    let hole_center = Vec3::new(35.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(hole_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(hole_center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    assert_eq!(shell.app().active_box_count(), 2);
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("-8");
    shell.press_key(Key::Enter);

    let snapshot = shell.app().document_snapshot();
    let definition = snapshot
        .definition(
            snapshot
                .occurrence(OccurrenceId(1))
                .unwrap()
                .definition_id(),
        )
        .unwrap();
    assert!(
        matches!(
            snapshot
                .feature(*definition.feature_ids().last().unwrap())
                .unwrap()
                .kind(),
            FeatureKind::Boolean {
                operation: ketchup_model::document::BooleanOperation::Cut,
                ..
            }
        ),
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "the drawn circle is used up"
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn push_pull_drag_is_continuous_between_grid_inference_targets() {
    let mut shell = Shell::new();
    let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
    let center = origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z);
    let drag_start = shell.app().viewport_position(center).unwrap();
    let projected_up = shell
        .app()
        .viewport_position(center + Vec3::new(0.0, 0.0, 1.0))
        .unwrap();
    let projected_unit = projected_up - drag_start;
    let drag_end = drag_start + projected_unit.normalized() * projected_unit.length() * 7.5;

    shell.click_command(AppCommand::PushPull);
    shell.drag(drag_start, drag_end);

    assert!(
        (shell.app().document_height_mm() - 27.5).abs() < 1.0e-4,
        "Push/Pull must preserve the 7.5 mm pointer distance: {}",
        shell.app().action_digest()
    );
}

#[test]
fn circle_push_pull_drag_renders_a_curved_solid_preview_and_commits_once() {
    let mut shell = Shell::new();
    // The circle is pulled from the box's exact top face.
    wait_for_one_exact_body(&mut shell);
    let center = Vec3::new(75.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let drag_start = shell.app().viewport_position(center).unwrap();
    let projected_up = shell
        .app()
        .viewport_position(center + Vec3::new(0.0, 0.0, 1.0))
        .unwrap();
    let drag_direction = (projected_up - drag_start).normalized();
    shell.click_command(AppCommand::PushPull);

    let mut maximum_render_depth = 0.0_f64;
    shell.drag_observing(drag_start, drag_start + drag_direction * 120.0, |app| {
        maximum_render_depth =
            maximum_render_depth.max(app.push_pull_preview_render_depth_mm().unwrap_or(0.0));
    });

    assert!(
        maximum_render_depth > 1.0,
        "the held Circle drag must render a genuinely extruded profile preview; max depth={maximum_render_depth}, value={}, selection={:?}, revision={}, circle={:?}, host={:?}",
        shell.app().value_input(),
        shell.app().selected_reference(),
        shell.app().document_revision(),
        shell.app().occurrence_box_geometry(2),
        shell.app().occurrence_box_geometry(1),
    );
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_ne!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().occurrence_box_geometry(2).unwrap().1.z > 1.0);
}

#[test]
fn circle_push_pull_negative_correction_directly_creates_a_hole_through_accesskit() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 1);
    let center = Vec3::new(35.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    let profile_revision = shell.app().document_revision();
    let profile_digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("10");
    shell.press_key(Key::Enter);
    let original_revision = shell.app().document_revision();
    let original_digest = shell.app().canonical_digest();
    assert_eq!(original_revision, profile_revision + 1);

    shell.type_text("-20");
    shell.press_key(Key::Enter);

    assert!(!shell.app().has_drawn_shape_preview());
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(
        shell.app().document_revision(),
        original_revision + 1,
        "{}",
        shell.app().action_digest()
    );
    assert_ne!(shell.app().canonical_digest(), original_digest);
    assert_eq!(shell.app().active_box_count(), 1);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().document_revision(), profile_revision);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().circle_profile_count(), 1);
}

#[test]
fn circle_through_hole_moves_by_dragging_its_inner_wall() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 1);
    let center = Vec3::new(35.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    shell.click_command(AppCommand::PushPull);
    shell.type_text("-20");
    shell.press_key(Key::Enter);
    wait_for_exact_bodies(&mut shell, 1);

    let before = shell.app().document_snapshot();
    let occurrence_transform = before.scene_query().into_iter().next().unwrap().transform;
    let profile_id = before
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::Profile {
                segments,
                closed: true,
            } if segments.len() == 2
                && segments.iter().all(|segment| {
                    matches!(
                        segment,
                        ketchup_model::document::ProfileSegment::CircularArc { .. }
                    )
                }) =>
            {
                Some(feature.id())
            }
            _ => None,
        })
        .expect("the through hole must retain its circular driving profile");
    let revision = shell.app().document_revision();
    let undo_steps = shell.app().undo_step_count();

    let inner_wall = shell
        .app()
        .viewport_position(center + Vec3::new(7.0, -7.0, 0.0))
        .unwrap();
    shell.click_command(AppCommand::Move);
    shell.move_pointer(inner_wall);
    shell.click_at(inner_wall);
    assert!(
        !shell.app().move_profile_preview_paths().is_empty(),
        "Move on the circular rim must target the hole profile"
    );
    let value_label = shell.catalog().text("value-label-distance");
    shell.focus_text_input(&value_label);
    shell.key(Key::A, ctrl());
    shell.type_text("5;0;0");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().document_revision(),
        revision + 1,
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
    let moved = shell.app().document_snapshot();
    assert_eq!(
        moved.scene_query().into_iter().next().unwrap().transform,
        occurrence_transform,
        "moving the hole must not move the host box"
    );
    let FeatureKind::Profile { segments, .. } = moved.feature(profile_id).unwrap().kind() else {
        panic!("the moved circular profile must remain canonical")
    };
    assert!(segments.iter().all(|segment| matches!(
        segment,
        ketchup_model::document::ProfileSegment::CircularArc { center_mm, .. }
            if *center_mm == [5.0, 0.0]
    )));

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().document_revision(), revision);
}

#[test]
fn circle_push_pull_creates_an_exact_cylinder_and_circular_hole_with_one_step_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("circle-push-pull.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_exact_bodies(&mut shell, 1);

    let cylinder_center = Vec3::new(75.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(cylinder_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(cylinder_center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    assert_eq!(shell.app().circle_profile_count(), 1);
    assert_eq!(shell.app().active_box_count(), 2);
    let cylinder_selection = shell.app().selected_reference().unwrap();
    assert_eq!(
        cylinder_selection.instance_path.root_occurrence(),
        OccurrenceId(2)
    );
    assert_eq!(
        cylinder_selection.element,
        ketchup_interaction::ElementId::Face {
            axis: Axis::Z,
            side: ketchup_interaction::Side::Maximum,
        }
    );
    assert!(shell.app().occurrence_box_geometry(2).is_some());
    let cylinder_profile_digest = shell.app().canonical_digest();
    let cylinder_profile_revision = shell.app().document_revision();

    shell.click_command(AppCommand::PushPull);
    assert_eq!(
        shell
            .app()
            .selected_reference()
            .unwrap()
            .instance_path
            .root_occurrence(),
        OccurrenceId(2)
    );
    shell.app_mut().set_push_pull_distance_input("30");
    assert!(shell.app_mut().start_preview());
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_BREP_GRAPH_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), cylinder_profile_revision);
    assert_eq!(shell.app().canonical_digest(), cylinder_profile_digest);
    shell.app_mut().cancel_preview();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("30");
    shell.press_key(Key::Enter);
    let cylinder_digest = shell.app().canonical_digest();
    assert_eq!(
        shell.app().document_revision(),
        cylinder_profile_revision + 1
    );
    assert_eq!(shell.app().occurrence_box_geometry(2).unwrap().1.z, 30.0);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), cylinder_profile_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), cylinder_digest);

    let hole_center = Vec3::new(35.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(hole_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(hole_center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    let hole_profile_digest = shell.app().canonical_digest();
    let hole_profile_revision = shell.app().document_revision();
    assert_eq!(shell.app().active_box_count(), 3);

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-20");
    shell.settle();
    assert!(
        shell.app_mut().start_preview(),
        "{}",
        shell.app().action_digest()
    );
    assert!(shell.app().has_drawn_shape_preview());
    assert_eq!(shell.app().document_revision(), hole_profile_revision);
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);
    shell.app_mut().cancel_preview();
    assert!(!shell.app().has_drawn_shape_preview());
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);

    shell.click_command(AppCommand::PushPull);
    shell.type_text("-20");
    shell.press_key(Key::Enter);
    assert!(!shell.app().has_drawn_shape_preview());
    assert!(!shell.app().has_occurrence_operation_preview());
    let hole_digest = shell.app().canonical_digest();
    assert!(shell.app().document_revision() > hole_profile_revision);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().circle_profile_count(), 2);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);
    assert_eq!(shell.app().active_box_count(), 3);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), hole_digest);
    assert_eq!(shell.app().active_box_count(), 2);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), hole_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().circle_profile_count(), 2);
}

#[test]
fn circular_profile_cuts_a_cylindrical_host_as_pocket_and_through_cut() {
    let mut shell = Shell::new();
    let host_center = Vec3::new(75.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(host_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(host_center + Vec3::new(20.0, 0.0, 0.0))
            .unwrap(),
    );
    shell.click_command(AppCommand::PushPull);
    shell.type_text("30");
    shell.press_key(Key::Enter);
    wait_for_exact_bodies(&mut shell, 2);
    let host_triangle_count = shell.app().exact_render_triangle_count();
    let host_bounds = shell.app().occurrence_box_geometry(2).unwrap();
    assert!((host_bounds.0.x - 55.0).abs() < 1.0e-3);
    assert!((host_bounds.0.y - 5.0).abs() < 1.0e-3);
    assert!((host_bounds.0.z - 20.0).abs() < 1.0e-8);
    assert!((host_bounds.1.x - 40.0).abs() < 1.0e-3);
    assert!((host_bounds.1.y - 40.0).abs() < 1.0e-3);
    assert!((host_bounds.1.z - 30.0).abs() < 1.0e-8);

    let hole_center = host_center + Vec3::new(0.0, 0.0, 30.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(hole_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(hole_center + Vec3::new(5.0, 0.0, 0.0))
            .unwrap(),
    );
    let profile_revision = shell.app().document_revision();
    let profile_digest = shell.app().canonical_digest();
    let profile_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("-20");
    shell.press_key(Key::Enter);
    assert!(!shell.app().has_drawn_shape_preview());
    assert!(!shell.app().has_occurrence_operation_preview());
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().document_revision(), profile_revision + 1);
    assert_eq!(shell.app().undo_step_count(), profile_undo_steps + 1);
    assert_eq!(shell.app().occurrence_box_geometry(2).unwrap(), host_bounds);
    assert!(shell.app().exact_render_triangle_count() > host_triangle_count);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().document_revision(), profile_revision);
    assert_eq!(shell.app().undo_step_count(), profile_undo_steps);

    shell.click_command(AppCommand::Select);
    let mesh_projection =
        MeshInteractionProjection::from_snapshot(&shell.app().document_snapshot());
    assert!(
        mesh_projection.contains_occurrence(&InstancePath::root(OccurrenceId(3))),
        "profile occurrence is missing from mesh projection; count={}",
        mesh_projection.occurrence_count()
    );
    let hole_edge = shell
        .app()
        .viewport_position(hole_center + Vec3::new(5.0, 0.0, 0.0))
        .unwrap();
    shell.move_pointer(hole_edge);
    for _ in 0..3 {
        if shell
            .app()
            .hovered_selection()
            .is_some_and(|selection| selection.instance_path.root_occurrence() == OccurrenceId(3))
        {
            break;
        }
        shell.press_key(Key::Tab);
    }
    assert!(
        shell
            .app()
            .hovered_selection()
            .is_some_and(|selection| selection.instance_path.root_occurrence() == OccurrenceId(3)),
        "profile occurrence was not reachable in overlap cycle: hovered={:?}, overlap={:?}",
        shell.app().hovered_selection(),
        shell.app().hovered_overlap_choice()
    );
    shell.click_at(hole_edge);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(3)));
    shell.click_command(AppCommand::PushPull);
    shell.type_text("-30");
    shell.press_key(Key::Enter);
    assert!(!shell.app().has_drawn_shape_preview());
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), profile_revision + 2);
    assert_eq!(shell.app().undo_step_count(), profile_undo_steps + 1);
    wait_for_exact_bodies(&mut shell, 2);
    assert_eq!(shell.app().occurrence_box_geometry(2).unwrap(), host_bounds);
    assert!(shell.app().exact_render_triangle_count() > host_triangle_count);
    let through_cut_digest = shell.app().canonical_digest();
    assert_ne!(through_cut_digest, profile_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().undo_step_count(), profile_undo_steps);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), through_cut_digest);
    assert_eq!(shell.app().occurrence_box_geometry(2).unwrap(), host_bounds);
}

#[test]
fn general_revolve_selects_an_axis_previews_exact_angle_and_commits_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("general-revolve.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path);
    let mut shell = Shell::with_dialogs(dialogs);
    assert!(shell.app_mut().create_closed_polyline(vec![
        [120.0, 0.0],
        [140.0, 0.0],
        [140.0, 30.0],
        [120.0, 30.0],
    ]));
    shell.settle();

    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    shell.open_menu("menu-model");
    assert!(
        shell.offers(AppCommand::Revolve),
        "the selected closed profile must expose Revolve through AccessKit"
    );
    shell.click_command(AppCommand::Revolve);

    let axis_start = shell
        .app()
        .viewport_position(Vec3::new(110.0, 0.0, 0.0))
        .unwrap();
    let axis_end = shell
        .app()
        .viewport_position(Vec3::new(110.0, 30.0, 0.0))
        .unwrap();
    shell.click_at(axis_start);
    assert!(!shell.app().has_revolve_preview());
    shell.click_at(axis_end);
    let (axis_start_mm, axis_end_mm, angle_degrees) =
        shell.app().revolve_preview_parameters().unwrap();
    assert!((axis_start_mm[0] - 110.0).abs() < 1.0e-4);
    assert!(axis_start_mm[1].abs() < 1.0e-4);
    assert!((axis_end_mm[0] - 110.0).abs() < 1.0e-4);
    assert!((axis_end_mm[1] - 30.0).abs() < 1.0e-4);
    assert_eq!(angle_degrees, 360.0);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    // Review 2026-10-17 P3-G: the value is an angle, so `9 cm` revolves
    // nothing (it once made 90°), and `225°` below is 225 degrees.
    shell.type_text("9");
    shell.type_text(" cm");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().revolve_preview_parameters(), None);
    assert_eq!(shell.app().document_revision(), before_revision);

    shell.type_text("225");
    assert_eq!(
        shell.app().revolve_preview_parameters(),
        Some((axis_start_mm, axis_end_mm, 225.0))
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.press_key(Key::Enter);

    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().latest_revolve_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        (committed.1, committed.2, committed.3),
        (axis_start_mm, axis_end_mm, 225.0)
    );
    assert_ne!(committed_digest, before_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().latest_revolve_parameters().is_none());
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().latest_revolve_parameters(), Some(committed));

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert!(shell.app().latest_revolve_parameters().is_none());
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().latest_revolve_parameters(), Some(committed));
}

#[test]
fn planar_offset_previews_signed_exact_bounds_and_commits_from_localized_headless_shell() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    assert!(shell.app_mut().create_closed_polyline(vec![
        [120.0, 0.0],
        [220.0, 0.0],
        [220.0, 60.0],
        [120.0, 60.0],
    ]));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.open_menu("menu-model");
    assert!(
        shell.offers(AppCommand::PlanarOffset),
        "the selected rectangular profile must expose localized Planar Offset through AccessKit"
    );
    shell.click_command(AppCommand::PlanarOffset);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.type_text("-31");
    assert!(shell.app().planar_offset_preview_parameters().is_none());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.key(Key::A, ctrl());
    shell.type_text("-7.5");
    let preview = shell.app().planar_offset_preview_parameters().unwrap();
    assert_eq!(preview.1, -7.5);
    assert_eq!(preview.2, [[127.5, 7.5, 0.0], [212.5, 52.5, 0.0]]);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.press_key(Key::Enter);
    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().latest_planar_offset_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(committed.1, preview.0);
    assert_eq!(committed.2, -7.5);
    assert_ne!(committed_digest, before_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().latest_planar_offset_parameters().is_none());
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(
        shell.app().latest_planar_offset_parameters(),
        Some(committed)
    );
}

#[test]
fn sweep_previews_exact_profile_path_and_commits_from_localized_headless_shell() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    assert!(shell.app_mut().create_sweep_inputs(
        vec![[-5.0, -10.0], [5.0, -10.0], [5.0, 10.0], [-5.0, 10.0]],
        [0.0, 0.0],
        [0.0, 125.0],
    ));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.open_menu("menu-model");
    assert!(
        shell.offers(AppCommand::Sweep),
        "the selected profile/path definition must expose localized Sweep through AccessKit"
    );
    shell.click_command(AppCommand::Sweep);
    let preview = shell.app().sweep_preview_parameters().unwrap();
    assert_eq!(preview.2, [[-5.0, 0.0, -10.0], [5.0, 125.0, 10.0]]);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.press_key(Key::Escape);
    assert!(shell.app().sweep_preview_parameters().is_none());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.click_menu_command("menu-model", AppCommand::Sweep);
    assert_eq!(shell.app().sweep_preview_parameters(), Some(preview));
    shell.press_key(Key::Enter);

    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().latest_sweep_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!((committed.1, committed.2), (preview.0, preview.1));
    assert_ne!(committed_digest, before_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().latest_sweep_parameters().is_none());
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().latest_sweep_parameters(), Some(committed));
}

#[test]
fn spline_profiles_preview_exact_loft_and_commit_from_localized_headless_shell() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    assert!(shell.app_mut().create_loft_inputs(vec![
        (
            vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]],
            0.0,
        ),
        (
            vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
            80.0,
        ),
    ]));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.open_menu("menu-model");
    assert!(
        shell.offers(AppCommand::Loft),
        "the selected ordered spline profiles must expose localized Loft through AccessKit"
    );
    shell.click_command(AppCommand::Loft);
    let preview = shell.app().loft_preview_parameters().unwrap();
    assert_eq!(
        preview
            .0
            .iter()
            .map(|section| section.1)
            .collect::<Vec<_>>(),
        [0.0, 80.0]
    );
    assert_eq!(preview.1, [[-20.0, -10.0, 0.0], [20.0, 10.0, 80.0]]);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.press_key(Key::Escape);
    assert!(shell.app().loft_preview_parameters().is_none());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.click_menu_command("menu-model", AppCommand::Loft);
    assert_eq!(shell.app().loft_preview_parameters(), Some(preview.clone()));
    shell.press_key(Key::Enter);

    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().latest_loft_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(committed.1, preview.0);
    assert_ne!(committed_digest, before_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().latest_loft_parameters().is_none());
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().latest_loft_parameters(), Some(committed));
}

#[test]
fn exact_worker_preserves_assembly_references_while_publishing_general_finish_topology() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let locator = TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: FeatureId(2),
        kind: TopologicalElementKind::Face,
        ordinal: 3,
    };
    let mut prepared = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        shell.settle();
        if shell.app().exact_stable_reference_count() >= 2
            && shell.app_mut().prepare_general_finish(
                locator.clone(),
                GeneralFinishKind::Shell,
                2.0,
            )
        {
            prepared = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(prepared, "{}", shell.app().action_digest());
    assert!(shell.app().exact_stable_reference_count() >= 2);
    let preview = shell.app().general_finish_preview_parameters().unwrap();
    assert_eq!(preview.0, FeatureId(2));
    assert_eq!(preview.1.kind, TopologicalElementKind::Face);
    assert_eq!(preview.1.producer_element_id, "generated-result/face/3");
    assert_eq!(preview.2, GeneralFinishKind::Shell);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
}

#[test]
fn topology_bound_push_pull_edits_a_non_top_planar_face_through_headless_ui() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let mut selected = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        shell.settle();
        if shell
            .app_mut()
            .select_topological_locator(TopologicalPickLocator {
                instance_path: InstancePath::root(OccurrenceId(1)),
                producer_feature_id: FeatureId(2),
                kind: TopologicalElementKind::Face,
                ordinal: 3,
            })
        {
            selected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(selected, "{}", shell.app().action_digest());
    assert_eq!(
        shell.app().selected_reference().unwrap().element,
        ElementId::TopologicalFace(3)
    );
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("5");
    shell.press_key(Key::Enter);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        shell.step();
        if shell.app().document_revision() > before_revision {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(shell.app().document_revision(), before_revision + 1);
    let committed_digest = shell.app().canonical_digest();
    assert_ne!(committed_digest, before_digest);
    assert_eq!(
        shell.app().occurrence_box_geometry(1),
        Some((Vec3::new(-5.0, 0.0, 0.0), Vec3::new(105.0, 60.0, 20.0),))
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
}

#[test]
fn imported_exact_finishes_and_face_push_pull_recompute_through_headless_ui() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let directory = tempfile::tempdir().unwrap();
    let document_path = directory.path().join("imported-face-push-pull.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    wait_for_one_exact_body(&mut shell);
    let snapshot = shell.app().document_snapshot();
    let imported = snapshot
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::ImportedExactBody(_)))
        .unwrap();
    let imported_feature_id = imported.id();
    let imported_definition_id = imported.definition_id();
    let imported_occurrence_id = snapshot
        .occurrences()
        .find(|occurrence| occurrence.definition_id() == imported_definition_id)
        .unwrap()
        .id();
    assert_eq!(
        shell.app().exact_current_producer_ids(),
        [imported_feature_id]
    );
    let imported_digest = shell.app().canonical_digest();
    for (kind, element_kind, ordinal, amount_mm) in [
        (
            GeneralFinishKind::Shell,
            TopologicalElementKind::Face,
            1,
            1.0,
        ),
        (
            GeneralFinishKind::Fillet,
            TopologicalElementKind::Edge,
            0,
            0.75,
        ),
        (
            GeneralFinishKind::Chamfer,
            TopologicalElementKind::Edge,
            5,
            0.75,
        ),
    ] {
        let before_revision = shell.app().document_revision();
        let before_undo_steps = shell.app().undo_step_count();
        assert!(shell.app_mut().prepare_general_finish(
            TopologicalPickLocator {
                instance_path: InstancePath::root(imported_occurrence_id),
                producer_feature_id: imported_feature_id,
                kind: element_kind,
                ordinal,
            },
            kind,
            amount_mm,
        ));
        let preview = shell.app().general_finish_preview_parameters().unwrap();
        assert_eq!(preview.0, imported_feature_id);
        assert_eq!(preview.1.kind, element_kind);
        assert_eq!(preview.1.producer_feature_id, imported_feature_id);
        assert_eq!(preview.2, kind);
        assert_eq!(preview.3, amount_mm);
        assert_eq!(shell.app().document_revision(), before_revision);
        assert_eq!(shell.app().canonical_digest(), imported_digest);
        assert!(shell.app_mut().confirm_general_finish());
        assert!(shell.app().document_revision() > before_revision);
        assert_eq!(shell.app().undo_step_count(), before_undo_steps + 1);
        let committed_digest = shell.app().canonical_digest();
        let generated_feature_id = match kind {
            GeneralFinishKind::Shell => shell.app().latest_topology_shell_parameters().unwrap().0,
            GeneralFinishKind::Fillet | GeneralFinishKind::Chamfer => {
                shell
                    .app()
                    .latest_topology_edge_finish_parameters()
                    .unwrap()
                    .0
            }
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            shell.settle();
            if shell
                .app()
                .exact_current_producer_ids()
                .contains(&generated_feature_id)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            shell
                .app()
                .exact_current_producer_ids()
                .contains(&generated_feature_id),
            "{kind:?}: {}",
            shell.app().action_digest()
        );
        shell.key(Key::Z, ctrl());
        assert_eq!(shell.app().canonical_digest(), imported_digest);
        shell.key(Key::Y, ctrl());
        assert_eq!(shell.app().canonical_digest(), committed_digest);
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            shell.settle();
            if shell
                .app()
                .exact_current_producer_ids()
                .contains(&generated_feature_id)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            shell
                .app()
                .exact_current_producer_ids()
                .contains(&generated_feature_id),
            "{kind:?} did not recompute after Redo"
        );
        shell.key(Key::Z, ctrl());
        assert_eq!(shell.app().canonical_digest(), imported_digest);
    }
    assert!(!shell.app_mut().prepare_general_finish(
        TopologicalPickLocator {
            instance_path: InstancePath::root(imported_occurrence_id),
            producer_feature_id: imported_feature_id,
            kind: TopologicalElementKind::Face,
            ordinal: u32::MAX,
        },
        GeneralFinishKind::Shell,
        1.0,
    ));
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert!(
        shell
            .app_mut()
            .select_topological_locator(TopologicalPickLocator {
                instance_path: InstancePath::root(imported_occurrence_id),
                producer_feature_id: imported_feature_id,
                kind: TopologicalElementKind::Face,
                ordinal: 0,
            })
    );
    let before_revision = shell.app().document_revision();
    let before_undo_steps = shell.app().undo_step_count();
    let before_digest = shell.app().canonical_digest();
    shell.app_mut().set_push_pull_distance_input("5");
    assert!(shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    let _ = shell.app_mut().confirm_preview();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        shell.step();
        if shell.app().document_revision() > before_revision {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(
        shell.app().document_revision() > before_revision,
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().undo_step_count(), before_undo_steps + 1);
    let committed_digest = shell.app().canonical_digest();
    let snapshot = shell.app().document_snapshot();
    let (offset_feature_id, offset_target, offset_face, offset_distance) = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::FaceOffset {
                target,
                face: FaceRef::Topological(face),
                distance,
            } => Some((feature.id(), *target, face, distance.millimetres())),
            _ => None,
        })
        .unwrap();
    assert_eq!(offset_target, imported_feature_id);
    assert_eq!(offset_face.producer_feature_id, imported_feature_id);
    assert_eq!(offset_face.producer_element_id, "imported-result/face/0");
    assert_eq!(offset_distance, 5.0);
    let graph = ExactBRepGraph::from_snapshot(&snapshot, imported_definition_id, offset_feature_id)
        .unwrap();
    let source = std::fs::read(&source_path).unwrap();
    let mut tampered_source = source.clone();
    tampered_source[0] ^= 1;
    let mut direct_worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    assert!(
        direct_worker
            .evaluate_exact_brep_graph_with_imported_source(&graph, &tampered_source)
            .is_err()
    );
    direct_worker
        .evaluate_exact_brep_graph_with_imported_source(&graph, &source)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        shell.settle();
        if shell
            .app()
            .exact_current_producer_ids()
            .contains(&offset_feature_id)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        shell
            .app()
            .exact_current_producer_ids()
            .contains(&offset_feature_id)
    );

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        shell.settle();
        if shell
            .app()
            .exact_current_producer_ids()
            .contains(&offset_feature_id)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        shell
            .app()
            .exact_current_producer_ids()
            .contains(&offset_feature_id)
    );
}

#[test]
fn multi_edge_fillet_is_canonical_stale_safe_atomic_and_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("multi-edge-fillet.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path);
    let mut shell = Shell::with_dialogs(dialogs);
    install_general_finish_graph_result(&mut shell, FeatureId(2));
    let locator = |ordinal| TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: FeatureId(2),
        kind: TopologicalElementKind::Edge,
        ordinal,
    };
    assert!(shell.app_mut().select_topological_locator(locator(9)));
    assert!(
        shell
            .app_mut()
            .select_topological_locator_additive(locator(2), true)
    );
    assert!(!shell.app_mut().select_topological_locator_additive(
        TopologicalPickLocator {
            instance_path: InstancePath::root(OccurrenceId(1)),
            producer_feature_id: FeatureId(2),
            kind: TopologicalElementKind::Face,
            ordinal: 3,
        },
        true,
    ));

    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let before_undo_steps = shell.app().undo_step_count();
    shell.click_menu_command("menu-model", AppCommand::Fillet);
    let preview = shell
        .app()
        .general_finish_preview_selection_parameters()
        .unwrap();
    assert_eq!(preview.0, FeatureId(2));
    assert_eq!(preview.2, GeneralFinishKind::Fillet);
    assert_eq!(preview.1.len(), 2);
    assert_eq!(preview.1[0].producer_element_id, "generated-result/edge/2");
    assert_eq!(preview.1[1].producer_element_id, "generated-result/edge/9");
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    assert!(
        shell
            .app_mut()
            .select_topological_locator_additive(locator(5), true)
    );
    assert!(!shell.app_mut().confirm_general_finish());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo_steps);
    assert!(
        shell
            .app_mut()
            .select_topological_locator_additive(locator(5), true)
    );

    shell.click_menu_command("menu-model", AppCommand::Fillet);
    shell.type_text("1.25");
    shell.press_key(Key::Enter);
    let committed = shell
        .app()
        .latest_topology_edge_finish_set_parameters()
        .unwrap();
    assert_eq!(committed.1, preview.1);
    assert_eq!(committed.2, EdgeFinishKind::Fillet);
    assert_eq!(committed.3, 1.25);
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo_steps + 1);
    let committed_digest = shell.app().canonical_digest();

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert!(
        shell
            .app()
            .latest_topology_edge_finish_set_parameters()
            .is_none()
    );
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(
        shell.app().latest_topology_edge_finish_set_parameters(),
        Some(committed)
    );
}

#[test]
fn general_shell_fillet_and_chamfer_preview_exact_stable_selections_and_persist() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("general-shell-finish.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path);
    let mut shell = Shell::with_dialogs(dialogs);
    install_general_finish_graph_result(&mut shell, FeatureId(2));
    select_general_finish_topology(&mut shell, FeatureId(2), TopologicalElementKind::Face, 3);

    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    shell.open_menu("menu-model");
    assert!(shell.offers(AppCommand::Shell));
    shell.click_command(AppCommand::Shell);
    let shell_preview = shell.app().general_finish_preview_parameters().unwrap();
    assert_eq!(shell_preview.0, FeatureId(2));
    assert_eq!(shell_preview.1.kind, TopologicalElementKind::Face);
    assert_eq!(shell_preview.1.producer_feature_id, FeatureId(2));
    assert_eq!(
        shell_preview.1.producer_element_id,
        "generated-result/face/3"
    );
    assert_eq!(shell_preview.2, GeneralFinishKind::Shell);
    assert_eq!(shell_preview.3, 2.0);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.type_text("2.5");
    assert_eq!(
        shell.app().general_finish_preview_parameters().unwrap().3,
        2.5
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.press_key(Key::Enter);

    let shell_digest = shell.app().canonical_digest();
    let shell_parameters = shell.app().latest_topology_shell_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell_parameters.1, shell_preview.1);
    assert_eq!(shell_parameters.2, 2.5);

    install_general_finish_graph_result(&mut shell, shell_parameters.0);
    let edge_locator = TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: shell_parameters.0,
        kind: TopologicalElementKind::Edge,
        ordinal: 2,
    };
    let assistant_revision = shell.app().document_revision();
    let assistant_digest = shell.app().canonical_digest();
    assert!(
        shell
            .app_mut()
            .prepare_general_finish(edge_locator, GeneralFinishKind::Fillet, 1.25,)
    );
    let fillet_preview = shell.app().general_finish_preview_parameters().unwrap();
    assert_eq!(fillet_preview.0, shell_parameters.0);
    assert_eq!(fillet_preview.1.kind, TopologicalElementKind::Edge);
    assert_eq!(
        fillet_preview.1.producer_element_id,
        "generated-result/edge/2"
    );
    assert_eq!(fillet_preview.2, GeneralFinishKind::Fillet);
    assert_eq!(fillet_preview.3, 1.25);
    assert_eq!(shell.app().document_revision(), assistant_revision);
    assert_eq!(shell.app().canonical_digest(), assistant_digest);
    assert!(shell.app_mut().confirm_general_finish());

    let fillet_digest = shell.app().canonical_digest();
    let fillet = shell
        .app()
        .latest_topology_edge_finish_parameters()
        .unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 2);
    assert_eq!(fillet.1, fillet_preview.1);
    assert_eq!(fillet.2, EdgeFinishKind::Fillet);
    assert_eq!(fillet.3, 1.25);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), shell_digest);
    assert!(
        shell
            .app()
            .latest_topology_edge_finish_parameters()
            .is_none()
    );

    install_general_finish_graph_result(&mut shell, shell_parameters.0);
    select_general_finish_topology(
        &mut shell,
        shell_parameters.0,
        TopologicalElementKind::Edge,
        9,
    );
    shell.click_menu_command("menu-model", AppCommand::Chamfer);
    let chamfer_preview = shell.app().general_finish_preview_parameters().unwrap();
    assert_eq!(
        chamfer_preview.1.producer_element_id,
        "generated-result/edge/9"
    );
    assert_eq!(chamfer_preview.2, GeneralFinishKind::Chamfer);
    shell.type_text("0.75");
    shell.press_key(Key::Enter);
    let chamfer_digest = shell.app().canonical_digest();
    let chamfer = shell
        .app()
        .latest_topology_edge_finish_parameters()
        .unwrap();
    assert_ne!(chamfer_digest, fillet_digest);
    assert_eq!(chamfer.1.producer_element_id, "generated-result/edge/9");
    assert_eq!(chamfer.2, EdgeFinishKind::Chamfer);
    assert_eq!(chamfer.3, 0.75);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), shell_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), chamfer_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert!(shell.app().latest_topology_shell_parameters().is_none());
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), chamfer_digest);
    assert_eq!(
        shell.app().latest_topology_shell_parameters(),
        Some(shell_parameters)
    );
    assert_eq!(
        shell.app().latest_topology_edge_finish_parameters(),
        Some(chamfer)
    );
}

#[test]
fn clicking_the_viewport_selects_the_occurrence() {
    let mut shell = Shell::new();
    let centre = shell.viewport_rect().center();

    shell.click_at(centre);

    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "clicking geometry must select exactly the occurrence under the pointer"
    );
}

#[test]
fn viewport_snap_hysteresis_acquires_retains_and_releases_an_endpoint() {
    let mut shell = Shell::new();
    let endpoint = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();

    shell.move_pointer(endpoint + eframe::egui::Vec2::new(7.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    shell.move_pointer(endpoint + eframe::egui::Vec2::new(10.0, 0.0));
    assert_eq!(
        shell.app().hovered_snap_kind(),
        Some(SnapKind::Endpoint),
        "the release radius must prevent flicker outside the acquire radius"
    );
    shell.move_pointer(endpoint + eframe::egui::Vec2::new(14.0, 0.0));
    assert_ne!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    shell.move_pointer(endpoint + eframe::egui::Vec2::new(10.0, 0.0));
    assert_ne!(
        shell.app().hovered_snap_kind(),
        Some(SnapKind::Endpoint),
        "an unlocked point outside the acquire radius must not reacquire"
    );
    let midpoint_world = Vec3::new(50.0, 0.0, 20.0);
    let midpoint = shell.app().viewport_position(midpoint_world).unwrap();
    shell.move_pointer(midpoint + Vec2::new(7.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Midpoint));
    shell.move_pointer(midpoint + Vec2::new(10.0, 0.0));
    assert_eq!(shell.app().hovered_snap_position(), Some(midpoint_world));
    shell.move_pointer(midpoint + Vec2::new(14.0, 0.0));
    assert_ne!(shell.app().hovered_snap_kind(), Some(SnapKind::Midpoint));
}

#[test]
fn tab_pick_through_selects_the_rear_face_of_one_solid() {
    let mut shell = Shell::new();
    // The faces of the box itself, before any exact body is shown.
    hold_exact_bodies_back(&mut shell);
    let top_centre = shell
        .app()
        .viewport_position(Vec3::new(50.0, 30.0, 20.0))
        .unwrap();
    shell.move_pointer(top_centre);

    assert_eq!(shell.app().hovered_overlap_choice(), Some((0, 2)));
    assert!(matches!(
        shell.app().hovered_selection().unwrap().element,
        ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }
    ));
    shell.press_key(Key::Tab);
    assert_eq!(shell.app().hovered_overlap_choice(), Some((1, 2)));
    assert!(matches!(
        shell.app().hovered_selection().unwrap().element,
        ElementId::Face {
            axis: Axis::Z,
            side: Side::Minimum,
        }
    ));
    shell.click_at(top_centre);
    assert!(matches!(
        shell.app().selected_reference().unwrap().element,
        ElementId::Face {
            axis: Axis::Z,
            side: Side::Minimum,
        }
    ));
}

#[test]
fn shift_click_selects_two_edges_away_from_their_midpoints() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_one_exact_body(&mut shell);
    let first = shell
        .app()
        .viewport_position(Vec3::new(25.0, 0.0, 20.0))
        .unwrap();
    let second = shell
        .app()
        .viewport_position(Vec3::new(100.0, 15.0, 20.0))
        .unwrap();

    shell.move_pointer(first);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Edge));
    shell.click_at(first);
    assert!(
        matches!(
            shell.app().selected_reference().unwrap().element,
            ElementId::TopologicalEdge { .. }
        ),
        "{:?}",
        shell.app().selected_reference()
    );
    shell.move_pointer(second);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Edge));
    shell.click_at_with(second, shift());

    shell.secondary_click_at(second);
    assert!(shell.offers(AppCommand::Fillet));
    assert!(shell.offers(AppCommand::Chamfer));
    shell.click_command(AppCommand::Fillet);
    let preview = shell
        .app()
        .general_finish_preview_selection_parameters()
        .expect("two Shift-clicked edges must open one Fillet preview");
    assert_eq!(preview.1.len(), 2);
}

#[test]
fn shift_click_adds_an_occurrence_without_replacing_the_primary_face() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(180.0, 0.0, 0.0)));
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    // Picked before the solid arrives, the box face would become its exact
    // face when the solid lands, before the Shift-click or not, by machine speed.
    wait_for_one_exact_body(&mut shell);
    shell.click_command(AppCommand::Select);
    shell.click_at(shell.top_face_centre(1));
    let primary = shell
        .app()
        .selected_reference()
        .expect("the first click must retain an exact primary face");
    assert!(matches!(primary.element, ElementId::TopologicalFace(_)));

    shell.click_at_with(shell.top_face_centre(2), shift());

    assert_eq!(shell.app().selected_occurrence_count(), 2);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
    assert_eq!(
        shell.app().selected_reference(),
        Some(primary),
        "an additive click must not replace the exact primary face"
    );
}

#[test]
fn shift_click_removing_the_primary_occurrence_promotes_the_remaining_exact_face() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(180.0, 0.0, 0.0)));
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    wait_for_one_exact_body(&mut shell);
    shell.click_command(AppCommand::Select);
    shell.click_at(shell.top_face_centre(1));
    shell.click_at_with(shell.top_face_centre(2), shift());
    shell.click_at_with(shell.top_face_centre(1), shift());

    assert_eq!(shell.app().selected_occurrence_count(), 1);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
    let primary = shell
        .app()
        .selected_reference()
        .expect("the remaining viewport selection must retain an exact primary face");
    assert_eq!(primary.instance_path, InstancePath::root(OccurrenceId(2)));
    assert!(matches!(primary.element, ElementId::TopologicalFace(_)));
    // It is the exact top face a plain click on that part picks.
    shell.click_at(shell.top_face_centre(2));
    assert_eq!(shell.app().selected_reference(), Some(primary));
}

#[test]
fn shift_click_adds_the_clicked_part_of_a_group_not_the_whole_group() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    for _ in 0..3 {
        assert!(shell.app_mut().copy_selected(Vec3::new(180.0, 0.0, 0.0)));
    }
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-model", AppCommand::Group);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    // The copies share one definition, so one exact body serves all four.
    wait_for_one_exact_body(&mut shell);
    shell.click_command(AppCommand::Select);
    // With exact faces a click picks the part inside the group, not the group.
    shell.click_at(shell.top_face_centre(1));
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    for added in [2, 3] {
        shell.click_at_with(shell.top_face_centre(added), shift());
        assert_eq!(shell.app().selected_occurrence_count(), added as usize);
    }
    for id in [1, 2, 3] {
        assert!(shell.app().occurrence_is_selected(OccurrenceId(id)));
    }
    assert!(!shell.app().occurrence_is_selected(OccurrenceId(4)));

    shell.click_at_with(shell.top_face_centre(2), shift());
    assert_eq!(shell.app().selected_occurrence_count(), 2);
    assert!(!shell.app().occurrence_is_selected(OccurrenceId(2)));
}

#[test]
fn tab_cycles_overlapping_occurrences_and_click_selects_the_visible_choice() {
    let mut shell = Shell::new();
    shell.settle();
    let centre = shell.viewport_rect().center();
    shell.click_at(centre);
    assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));
    assert!(shell.app_mut().move_selected(Vec3::new(-100.0, 0.0, 0.0)));
    wait_for_one_exact_body(&mut shell);
    shell.move_pointer(centre);

    assert_eq!(shell.app().hovered_overlap_choice(), Some((0, 4)));
    assert_eq!(
        shell.app().hovered_selection().unwrap().instance_path,
        InstancePath::root(OccurrenceId(1))
    );
    shell.press_key(Key::Tab);
    assert_eq!(shell.app().hovered_overlap_choice(), Some((1, 4)));
    assert_eq!(
        shell.app().hovered_selection().unwrap().instance_path,
        InstancePath::root(OccurrenceId(2))
    );
    shell.click_at(centre);
    assert_eq!(
        shell.app().selected_reference().unwrap().instance_path,
        InstancePath::root(OccurrenceId(2))
    );
}

#[test]
fn snapped_measurement_updates_the_value_box_without_mutating_the_document() {
    let mut shell = Shell::new();
    shell.click_command(AppCommand::Measure);
    let left = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();
    let right = shell
        .app()
        .viewport_position(Vec3::new(100.0, 0.0, 20.0))
        .unwrap();
    let centre = shell.viewport_rect().center();
    let left = left + (centre - left).normalized() * 2.0;
    let right = right + (centre - right).normalized() * 2.0;
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    shell.click_at(left);
    shell.move_pointer(right);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    assert_eq!(
        shell.app().hovered_snap_position(),
        Some(Vec3::new(100.0, 0.0, 20.0))
    );
    shell.click_at(right);

    assert_eq!(
        shell.app().measured_points(),
        Some((Vec3::new(0.0, 0.0, 20.0), Vec3::new(100.0, 0.0, 20.0),))
    );
    assert_eq!(shell.app().value_input(), "100");
    assert_eq!(shell.app().measured_distance_mm(), Some(100.0));
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn holding_ctrl_over_an_edge_reads_its_whole_length_without_clicking() {
    let mut shell = Shell::new();
    shell.click_command(AppCommand::Measure);
    let midpoint = shell
        .app()
        .viewport_position(Vec3::new(50.0, 0.0, 20.0))
        .unwrap();
    let centre = shell.viewport_rect().center();
    let pointer = midpoint + (centre - midpoint).normalized() * 2.0;
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    shell.move_pointer(pointer);
    assert_eq!(
        shell.app().measured_edge_length_mm(),
        None,
        "without Ctrl the edge is not read"
    );

    shell.set_modifiers(ctrl());
    assert_eq!(shell.app().measured_edge_length_mm(), Some(100.0));
    assert!(
        shell.app().action_digest().contains("100 mm"),
        "the edge length is shown: {}",
        shell.app().action_digest()
    );

    shell.set_modifiers(eframe::egui::Modifiers::NONE);
    assert_eq!(shell.app().measured_edge_length_mm(), None);
    assert_eq!(shell.app().measured_points(), None, "no point was placed");
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn a_click_outside_geometry_clears_the_selection() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(rect.center());
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    shell.click_at(rect.left_top() + eframe::egui::Vec2::new(12.0, 12.0));

    assert_eq!(shell.app().selected_occurrence_count(), 0);
}

#[test]
fn directional_drag_selection_contains_left_to_right_and_crosses_right_to_left() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(180.0, 0.0, 0.0)));
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    shell.click_command(AppCommand::Select);

    let projected_box = |app: &KetchupApp, occurrence_id| {
        let (origin, size) = app.occurrence_box_geometry(occurrence_id).unwrap();
        [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(size.x, 0.0, 0.0),
            Vec3::new(0.0, size.y, 0.0),
            Vec3::new(size.x, size.y, 0.0),
            Vec3::new(0.0, 0.0, size.z),
            Vec3::new(size.x, 0.0, size.z),
            Vec3::new(0.0, size.y, size.z),
            Vec3::new(size.x, size.y, size.z),
        ]
        .into_iter()
        .map(|corner| app.viewport_position(origin + corner).unwrap())
        .map(|point| Rect::from_min_max(point, point))
        .reduce(|left, right| left.union(right))
        .unwrap()
    };
    let first = projected_box(shell.app(), 1);
    let second = projected_box(shell.app(), 2);
    assert!(!first.intersects(second));

    shell.drag(
        first.left_top() - Vec2::splat(16.0),
        first.right_bottom() + Vec2::splat(16.0),
    );
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));

    shell.drag(
        second.right_top() + Vec2::new(16.0, -16.0),
        Pos2::new(second.center().x, second.bottom() + 16.0),
    );
    assert_eq!(shell.app().selected_occurrence_count(), 1);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
}

#[test]
fn shift_drag_selection_adds_without_forgetting_the_primary_face() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(180.0, 0.0, 0.0)));
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    // Picked before the solid arrives, the box face would become its exact
    // face when the solid lands, during the drag or not, by machine speed.
    wait_for_one_exact_body(&mut shell);
    shell.click_command(AppCommand::Select);
    shell.click_at(shell.top_face_centre(1));
    let primary = shell
        .app()
        .selected_reference()
        .expect("the first click must retain an exact primary face");
    assert!(matches!(primary.element, ElementId::TopologicalFace(_)));

    let (origin, size) = shell.app().occurrence_box_geometry(2).unwrap();
    let second = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(size.x, 0.0, 0.0),
        Vec3::new(0.0, size.y, 0.0),
        Vec3::new(size.x, size.y, 0.0),
        Vec3::new(0.0, 0.0, size.z),
        Vec3::new(size.x, 0.0, size.z),
        Vec3::new(0.0, size.y, size.z),
        Vec3::new(size.x, size.y, size.z),
    ]
    .into_iter()
    .map(|corner| shell.app().viewport_position(origin + corner).unwrap())
    .map(|point| Rect::from_min_max(point, point))
    .reduce(|left, right| left.union(right))
    .unwrap();

    shell.drag_with(
        second.left_top() - Vec2::splat(16.0),
        second.right_bottom() + Vec2::splat(16.0),
        shift(),
    );

    assert_eq!(shell.app().selected_occurrence_count(), 2);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
    assert_eq!(
        shell.app().selected_reference(),
        Some(primary),
        "an additive window must not downgrade the existing exact face selection"
    );
}

#[test]
fn empty_history_shortcuts_cancel_an_uncommitted_move() {
    for key in [Key::Z, Key::Y] {
        let mut shell = Shell::new();
        let source = shell.top_face_centre(1);
        let destination = source + Vec2::new(80.0, 0.0);
        let digest = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        assert_eq!(shell.app().undo_step_count(), 0);

        shell.click_command(AppCommand::Move);
        shell.click_at(source);
        shell.move_pointer(destination);
        assert!(shell.app().transform_gesture_active(), "key={key:?}");
        shell.key(key, ctrl());

        assert!(!shell.app().transform_gesture_active(), "key={key:?}");
        assert_eq!(shell.app().canonical_digest(), digest, "key={key:?}");
        assert_eq!(shell.app().document_revision(), revision, "key={key:?}");
    }
}

#[test]
fn empty_history_shortcuts_cancel_an_uncommitted_rotate() {
    for key in [Key::Z, Key::Y] {
        let mut shell = Shell::new();
        let rect = shell.viewport_rect();
        shell.click_at(shell.top_face_centre(1));
        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let centre = origin + size * 0.5;
        let digest = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        assert_eq!(shell.app().undo_step_count(), 0);

        shell.click_command(AppCommand::Rotate);
        shell.click_at(shell.app().project_to_screen(centre, rect));
        assert!(shell.app().transform_gesture_active(), "key={key:?}");
        shell.key(key, ctrl());

        assert!(!shell.app().transform_gesture_active(), "key={key:?}");
        assert_eq!(shell.app().canonical_digest(), digest, "key={key:?}");
        assert_eq!(shell.app().document_revision(), revision, "key={key:?}");
    }
}

#[test]
fn history_shortcuts_cancel_an_uncommitted_transform_before_navigating_history() {
    for (tool, key) in [
        (AppCommand::Move, Key::Z),
        (AppCommand::Move, Key::Y),
        (AppCommand::Rotate, Key::Z),
        (AppCommand::Rotate, Key::Y),
    ] {
        let mut shell = Shell::new();
        assert!(shell.app_mut().create_box());
        if key == Key::Y {
            shell.key(Key::Z, ctrl());
        }
        let digest = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        let redo_steps = shell.app().redo_step_count();
        let source = shell.top_face_centre(1);
        shell.click_at(source);
        shell.click_command(tool);
        match tool {
            AppCommand::Move => {
                shell.click_at(source);
                shell.move_pointer(source + Vec2::new(80.0, 0.0));
            }
            AppCommand::Rotate => {
                let rect = shell.viewport_rect();
                let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
                shell.click_at(shell.app().project_to_screen(origin + size * 0.5, rect));
            }
            _ => unreachable!(),
        }
        assert!(
            shell.app().transform_gesture_active(),
            "tool={tool:?}, key={key:?}"
        );

        shell.key(key, ctrl());

        assert!(
            !shell.app().transform_gesture_active(),
            "tool={tool:?}, key={key:?}"
        );
        assert_eq!(
            shell.app().canonical_digest(),
            digest,
            "tool={tool:?}, key={key:?}"
        );
        assert_eq!(
            shell.app().document_revision(),
            revision,
            "tool={tool:?}, key={key:?}"
        );
        assert_eq!(
            shell.app().undo_step_count(),
            undo_steps,
            "tool={tool:?}, key={key:?}"
        );
        assert_eq!(
            shell.app().redo_step_count(),
            redo_steps,
            "tool={tool:?}, key={key:?}"
        );
    }
}

#[test]
fn switching_tools_cancels_uncommitted_transform_and_copy_toggle() {
    for tool in [AppCommand::Move, AppCommand::Rotate] {
        let mut shell = Shell::new();
        shell.click_at(shell.top_face_centre(1));
        let digest = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        let selected = shell.app().selected_occurrence_count();

        shell.click_command(tool);
        match tool {
            AppCommand::Move => shell.click_at(shell.top_face_centre(1)),
            AppCommand::Rotate => {
                let rect = shell.viewport_rect();
                let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
                shell.click_at(shell.app().project_to_screen(origin + size * 0.5, rect));
            }
            _ => unreachable!(),
        }
        assert!(shell.app().transform_gesture_active(), "tool={tool:?}");
        shell.set_modifiers(ctrl());
        // A bare Ctrl tap toggles copy on release for both transform tools.
        shell.set_modifiers(Modifiers::NONE);
        assert!(
            match tool {
                AppCommand::Move => shell.app().move_copy_mode_active(),
                AppCommand::Rotate => shell.app().rotate_copy_mode_active(),
                _ => unreachable!(),
            },
            "tool={tool:?}"
        );

        shell.click_command(match tool {
            AppCommand::Move => AppCommand::Rotate,
            AppCommand::Rotate => AppCommand::Move,
            _ => unreachable!(),
        });
        shell.set_modifiers(Modifiers::NONE);

        assert!(!shell.app().transform_gesture_active(), "tool={tool:?}");
        assert!(!shell.app().move_copy_mode_active(), "tool={tool:?}");
        assert!(!shell.app().rotate_copy_mode_active(), "tool={tool:?}");
        assert_eq!(shell.app().canonical_digest(), digest, "tool={tool:?}");
        assert_eq!(shell.app().document_revision(), revision, "tool={tool:?}");
        assert_eq!(shell.app().undo_step_count(), undo_steps, "tool={tool:?}");
        assert_eq!(
            shell.app().selected_occurrence_count(),
            selected,
            "tool={tool:?}"
        );
    }
}

#[test]
fn move_axis_constraint_does_not_leak_across_tool_changes() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.press_key(Key::ArrowLeft);
    shell.click_command(AppCommand::Rotate);
    shell.click_command(AppCommand::Move);
    shell.type_text("25");
    shell.press_key(Key::Enter);

    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        origin + Vec3::new(25.0, 0.0, 0.0)
    );
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
}

#[test]
fn rotate_axis_constraint_does_not_leak_across_tool_changes() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Rotate);
    shell.press_key(Key::ArrowRight);
    shell.click_command(AppCommand::Move);
    shell.click_command(AppCommand::Rotate);
    shell.type_text("90");
    shell.press_key(Key::Enter);

    assert_eq!(
        rounded_extents(shell.app().occurrence_box_geometry(1).unwrap().1),
        (60.0, 100.0, 20.0),
        "returning to Rotate must not reuse the previous red-axis lock"
    );
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
}

#[test]
fn move_rejects_an_outliner_selection_change_during_the_gesture() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.settle();
    shell.click_at(shell.top_face_centre(1));
    shell.click_command(AppCommand::Move);
    shell.click_at(shell.top_face_centre(1));
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let second_name = shell
        .app()
        .occurrence_name(OccurrenceId(2))
        .unwrap()
        .trim_end_matches(" #1")
        .to_owned();
    let second_row = shell.catalog().format(
        "outliner-object",
        &BTreeMap::from([
            ("name", second_name),
            ("dimensions", "100 × 60 × 20".to_owned()),
            ("visibility", "◉".to_owned()),
        ]),
    );
    assert!(shell.app().transform_gesture_active());
    let first_origin = shell.app().occurrence_box_geometry(1).unwrap().0;
    let second_origin = shell.app().occurrence_box_geometry(2).unwrap().0;
    shell.click_row(&second_row);

    // Nothing has moved, and the gesture picked up on the first box is no
    // longer what a typed value applies to.
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
    assert!(!shell.app().move_copy_mode_active());
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
    assert!(!shell.app().occurrence_is_selected(OccurrenceId(1)));

    // A distance typed for the gesture is refused as for every tool, never
    // moved along a way the new selection was not shown; typed again it is a
    // fresh move of the new selection alone.
    shell.type_text("25");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("error-preview-stale")
    );
    assert_eq!(shell.app().canonical_digest(), digest);
    shell.type_text("25");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().format(
            "digest-exact-move-applied",
            &BTreeMap::from([("value", "+25 mm".to_owned())]),
        )
    );
    assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        first_origin
    );
    let moved = shell.app().occurrence_box_geometry(2).unwrap().0 - second_origin;
    assert!(
        (moved - Vec3::new(25.0, 0.0, 0.0)).length() < 1e-6,
        "{moved:?}"
    );
}

#[test]
fn rotate_rejects_an_outliner_selection_change_during_the_gesture() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.settle();
    shell.click_at(shell.top_face_centre(1));
    shell.click_command(AppCommand::Rotate);
    shell.click_at(shell.top_face_centre(1));
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();

    let second_name = shell
        .app()
        .occurrence_name(OccurrenceId(2))
        .unwrap()
        .trim_end_matches(" #1")
        .to_owned();
    let second_row = shell.catalog().format(
        "outliner-object",
        &BTreeMap::from([
            ("name", second_name),
            ("dimensions", "100 × 60 × 20".to_owned()),
            ("visibility", "◉".to_owned()),
        ]),
    );
    shell.click_row(&second_row);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
    shell.type_text("45");
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
    assert!(!shell.app().rotate_copy_mode_active());
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
    assert!(!shell.app().occurrence_is_selected(OccurrenceId(1)));
}

#[test]
fn stale_move_pointer_confirmation_ends_copy_toggle_without_mutation() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    shell.click_command(AppCommand::Move);
    let source = shell.top_face_centre(1);
    let destination = source + Vec2::new(80.0, 0.0);
    shell.move_pointer(source);
    shell.set_primary_button(source, Modifiers::NONE, true);
    shell.move_pointer(source + Vec2::new(20.0, 0.0));
    shell.set_modifiers(ctrl());
    shell.set_modifiers(Modifiers::NONE);
    assert!(shell.app().move_copy_mode_active());

    assert!(shell.app_mut().create_box());
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    shell.move_pointer(destination);
    shell.set_primary_button(destination, ctrl(), false);
    shell.set_modifiers(Modifiers::NONE);

    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
    assert!(
        !shell.app().move_copy_mode_active(),
        "rejecting the stale pointer confirmation must end Copy mode"
    );
}

#[test]
fn stale_rotate_pointer_confirmation_ends_copy_toggle_without_mutation() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let source = shell.app().occurrence_box_geometry(1).unwrap();
    let centre = source.0 + source.1 * 0.5;
    shell.click_command(AppCommand::Rotate);
    shell.click_at(shell.app().project_to_screen(centre, rect));
    let from = shell
        .app()
        .project_to_screen(centre + Vec3::new(source.1.x * 0.25, 0.0, 0.0), rect);
    let destination = shell
        .app()
        .project_to_screen(centre + Vec3::new(0.0, source.1.x * 0.25, 0.0), rect);
    shell.move_pointer(from);
    shell.set_primary_button(from, Modifiers::NONE, true);
    shell.move_pointer(from + Vec2::new(10.0, 0.0));
    shell.set_modifiers(ctrl());
    shell.set_modifiers(Modifiers::NONE);
    assert!(shell.app().rotate_copy_mode_active());

    assert!(shell.app_mut().create_box());
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    shell.move_pointer(destination);
    shell.set_primary_button(destination, ctrl(), false);
    shell.set_modifiers(Modifiers::NONE);

    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);
    assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
    assert!(
        !shell.app().rotate_copy_mode_active(),
        "rejecting the stale pointer confirmation must end Rotate-Copy mode"
    );
}

#[test]
fn entering_a_group_context_cancels_an_uncommitted_transform() {
    for tool in [AppCommand::Move, AppCommand::Rotate] {
        let mut shell = Shell::new();
        assert!(shell.app_mut().create_box());
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::Group);
        shell.settle();
        let group_row = shell.catalog().format(
            "outliner-group",
            &BTreeMap::from([("name", "Group 1".to_owned()), ("count", "2".to_owned())]),
        );
        let digest = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        let source = shell.top_face_centre(1);

        shell.click_command(tool);
        shell.click_at(source);
        shell.move_pointer(source + Vec2::new(60.0, 20.0));
        assert!(shell.app().transform_gesture_active(), "tool={tool:?}");
        shell.settle();
        assert!(shell.app().transform_gesture_active(), "tool={tool:?}");

        shell.double_click_row(&group_row);

        assert_eq!(shell.app().edit_context_depth(), 1, "tool={tool:?}");
        assert!(
            !shell.app().transform_gesture_active(),
            "entering a nested context must cancel the stale transform for {tool:?}"
        );
        assert_eq!(shell.app().canonical_digest(), digest, "tool={tool:?}");
        assert_eq!(shell.app().document_revision(), revision, "tool={tool:?}");
        assert_eq!(shell.app().undo_step_count(), undo_steps, "tool={tool:?}");
    }
}

#[test]
fn move_ctrl_after_the_first_click_switches_to_copy() {
    let mut shell = Shell::new();
    let source = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();
    let destination = shell
        .app()
        .viewport_position(Vec3::new(50.0, 0.0, 20.0))
        .unwrap();
    shell.click_command(AppCommand::Move);
    shell.click_at(source);
    let revision = shell.app().document_revision();
    shell.set_modifiers(ctrl());
    shell.set_modifiers(Modifiers::NONE);
    shell.click_at(destination);

    assert_eq!(shell.app().document_revision(), revision + 1);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().definition_count(), 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0,
        Vec3::ZERO,
        "Copy mode must preserve the source occurrence"
    );
    assert!(
        !shell.app().move_copy_mode_active(),
        "Copy mode must end with the gesture so the next Move does not duplicate"
    );
}

#[test]
fn ctrl_does_not_offer_copy_for_a_group_move() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-model", AppCommand::Group);
    let before = [1, 2].map(|id| shell.app().occurrence_box_geometry(id).unwrap().0);
    let source = shell.top_face_centre(1);

    shell.click_command(AppCommand::Move);
    shell.click_at(source);
    assert!(shell.app().transform_gesture_active());
    let move_digest = shell.app().action_digest().to_owned();
    shell.set_modifiers(ctrl());

    assert!(
        !shell.app().move_copy_mode_active(),
        "Ctrl must not offer Copy when the selected group can only be moved"
    );
    assert_eq!(shell.app().action_digest(), move_digest);

    shell.set_modifiers(Modifiers::NONE);
    shell.type_text("25;0;0");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().active_box_count(), 2);
    for (index, id) in [1, 2].into_iter().enumerate() {
        assert_eq!(
            shell.app().occurrence_box_geometry(id).unwrap().0,
            before[index] + Vec3::new(25.0, 0.0, 0.0)
        );
    }
}

#[test]
fn ctrl_does_not_offer_copy_for_a_group_rotate() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-model", AppCommand::Group);
    let source = shell.top_face_centre(1);
    let before_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Rotate);
    shell.click_at(source);
    assert!(shell.app().transform_gesture_active());
    let rotate_digest = shell.app().action_digest().to_owned();
    shell.set_modifiers(ctrl());

    assert!(
        !shell.app().rotate_copy_mode_active(),
        "Ctrl must not offer Rotate-Copy when the selected group can only be rotated"
    );
    assert_eq!(shell.app().action_digest(), rotate_digest);

    shell.set_modifiers(Modifiers::NONE);
    shell.type_text("90");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);
}

#[test]
fn copy_toggle_applies_to_typed_move_and_rotate_during_a_live_gesture() {
    let mut move_shell = Shell::new();
    let move_source_point = move_shell.top_face_centre(1);
    move_shell.click_at(move_source_point);
    let move_source = move_shell.app().occurrence_box_geometry(1).unwrap();
    let move_digest = move_shell.app().canonical_digest();
    let move_steps = move_shell.app().undo_step_count();

    move_shell.click_command(AppCommand::Move);
    move_shell.click_at(move_source_point);
    assert!(move_shell.app().transform_gesture_active());
    move_shell.set_modifiers(ctrl());
    move_shell.set_modifiers(Modifiers::NONE);
    assert!(move_shell.app().move_copy_mode_active());
    move_shell.type_text("25;0;0");
    move_shell.press_key(Key::Enter);

    assert_eq!(move_shell.app().active_box_count(), 2);
    assert_eq!(
        move_shell.app().occurrence_box_geometry(1),
        Some(move_source)
    );
    assert_eq!(
        move_shell.app().occurrence_box_geometry(2).unwrap().0,
        move_source.0 + Vec3::new(25.0, 0.0, 0.0)
    );
    assert_eq!(move_shell.app().undo_step_count(), move_steps + 1);
    move_shell.key(Key::Z, ctrl());
    assert_eq!(move_shell.app().canonical_digest(), move_digest);

    let mut rotate_shell = Shell::new();
    let rotate_centre = rotate_shell.top_face_centre(1);
    rotate_shell.click_at(rotate_centre);
    let rotate_source = rotate_shell.app().occurrence_box_geometry(1).unwrap();
    let rotate_digest = rotate_shell.app().canonical_digest();
    let rotate_steps = rotate_shell.app().undo_step_count();

    rotate_shell.click_command(AppCommand::Rotate);
    rotate_shell.click_at(rotate_centre);
    assert!(rotate_shell.app().transform_gesture_active());
    rotate_shell.set_modifiers(ctrl());
    rotate_shell.set_modifiers(Modifiers::NONE);
    rotate_shell.type_text("90");
    rotate_shell.press_key(Key::Enter);

    assert_eq!(rotate_shell.app().active_box_count(), 2);
    assert_eq!(
        rotate_shell.app().occurrence_box_geometry(1),
        Some(rotate_source)
    );
    assert_eq!(
        rounded_extents(rotate_shell.app().occurrence_box_geometry(2).unwrap().1),
        (60.0, 100.0, 20.0)
    );
    assert_eq!(rotate_shell.app().undo_step_count(), rotate_steps + 1);
    rotate_shell.key(Key::Z, ctrl());
    assert_eq!(rotate_shell.app().canonical_digest(), rotate_digest);
}

#[test]
fn typed_distance_and_vector_correct_the_last_move_copy_in_one_undo_step() {
    let mut shell = Shell::new();
    let source = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();
    let destination = shell
        .app()
        .viewport_position(Vec3::new(50.0, 0.0, 20.0))
        .unwrap();
    let original = shell.app().occurrence_box_geometry(1).unwrap().0;
    let initial_digest = shell.app().canonical_digest();
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.click_at(source);
    shell.click_at_with(destination, ctrl());
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);
    let first_copy = shell.app().occurrence_box_geometry(2).unwrap().0;
    let first_delta = first_copy - original;
    let direction = first_delta * (1.0 / first_delta.length());

    shell.type_text("80");
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().occurrence_box_geometry(1).unwrap().0, original);
    let corrected = shell.app().occurrence_box_geometry(2).unwrap().0;
    assert!(
        (corrected - (original + direction * 80.0)).length() < 1.0e-9,
        "a typed distance must reposition the copy from the original source: {corrected:?}"
    );
    assert_eq!(
        shell.app().undo_step_count(),
        initial_undo_steps + 1,
        "correcting a Move/Copy must replace its history tip"
    );

    shell.type_text("30;40;0");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().occurrence_box_geometry(2).unwrap().0,
        original + Vec3::new(30.0, 40.0, 0.0),
        "an exact vector must replace the previous copy offset"
    );
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn move_copy_array_accepts_x_and_slash_through_the_value_box() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    shell.click_command(AppCommand::Move);
    assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));

    shell.type_text("x5");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().active_box_count(), 6);
    assert_eq!(shell.app().undo_step_count(), 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(6).unwrap().0,
        Vec3::new(500.0, 0.0, 0.0)
    );

    shell.type_text("/5");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().active_box_count(), 6);
    assert_eq!(shell.app().undo_step_count(), 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(2).unwrap().0,
        Vec3::new(20.0, 0.0, 0.0)
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(6).unwrap().0,
        Vec3::new(100.0, 0.0, 0.0)
    );
}

#[test]
fn correcting_move_copy_spacing_preserves_the_array_count_and_mode() {
    let mut shell = Shell::new();
    let source = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();
    let destination = shell
        .app()
        .viewport_position(Vec3::new(100.0, 0.0, 20.0))
        .unwrap();
    let initial_digest = shell.app().canonical_digest();
    let initial_undo_steps = shell.app().undo_step_count();

    shell.click_command(AppCommand::Move);
    shell.click_at(source);
    shell.click_at_with(destination, ctrl());
    shell.type_text("x3");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().active_box_count(), 4);

    shell.type_text("50");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().active_box_count(),
        4,
        "correcting the spacing must not collapse a multiplied Copy array"
    );
    for (id, x) in [(2, 50.0), (3, 100.0), (4, 150.0)] {
        assert!(
            (shell.app().occurrence_box_geometry(id).unwrap().0 - Vec3::new(x, 0.0, 0.0)).length()
                < 0.001
        );
    }

    shell.type_text("/3");
    shell.press_key(Key::Enter);
    shell.type_text("90");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().active_box_count(),
        4,
        "correcting the spacing must not collapse a divided Copy array"
    );
    for (id, x) in [(2, 30.0), (3, 60.0), (4, 90.0)] {
        assert!(
            (shell.app().occurrence_box_geometry(id).unwrap().0 - Vec3::new(x, 0.0, 0.0)).length()
                < 0.001
        );
    }
    assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
}

#[test]
fn copying_an_occurrence_shares_one_definition() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());

    let before = shell.app().document_revision();
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();

    assert_eq!(shell.app().active_box_count(), 2, "a second occurrence");
    assert_eq!(
        shell.app().definition_count(),
        1,
        "Copy must reuse the definition instead of cloning it"
    );
    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "one gesture must produce exactly one canonical revision"
    );
}
