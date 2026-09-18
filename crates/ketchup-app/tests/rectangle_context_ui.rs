mod harness;

use eframe::egui::{Key, accesskit::Role};
use harness::Shell;
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_application::transforms::world_axis_rotation_transform;
use ketchup_core::{
    document::{
        CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId,
        FeatureKind, GroupId, OccurrenceId, Transform,
    },
    exact_brep_graph::ExactBRepGraph,
    persistence,
    sketch::{PrincipalPlane, SketchEntity},
};
use ketchup_interaction::Vec3;

fn tool(shell: &mut Shell, plane: PrincipalPlane) {
    shell.click_command(AppCommand::Rectangle);
    shell.click_role_and_label(Role::Button, &shell.catalog().text("face-workflow-title"));
    shell.click_role_and_label(
        Role::RadioButton,
        &shell.catalog().text(match plane {
            PrincipalPlane::Xy => "face-workflow-datum-xy",
            PrincipalPlane::Xz => "face-workflow-datum-xz",
            PrincipalPlane::Yz => "face-workflow-datum-yz",
        }),
    );
}

fn transform(t: Transform, p: Vec3) -> Vec3 {
    let m = t.matrix();
    Vec3::new(
        m[0] * p.x + m[1] * p.y + m[2] * p.z + m[3],
        m[4] * p.x + m[5] * p.y + m[6] * p.z + m[7],
        m[8] * p.x + m[9] * p.y + m[10] * p.z + m[11],
    )
}

fn local_corners(shell: &Shell) -> (DefinitionId, Vec<Vec3>) {
    let snapshot = shell.app().document_snapshot();
    let feature = snapshot.features().last().unwrap();
    let FeatureKind::Sketch(sketch) = feature.kind() else {
        panic!("rectangle sketch missing: {:?}", feature.kind())
    };
    let FeatureKind::Workplane(plane) = snapshot.feature(sketch.workplane).unwrap().kind() else {
        panic!("workplane missing")
    };
    let f = plane.frame;
    let points = sketch
        .entities
        .iter()
        .map(|entity| {
            let SketchEntity::Line { start_mm: p, .. } = entity else {
                panic!("line expected")
            };
            Vec3::new(
                f.origin_mm[0] + f.x_axis[0] * p[0] + f.y_axis[0] * p[1],
                f.origin_mm[1] + f.x_axis[1] * p[0] + f.y_axis[1] * p[1],
                f.origin_mm[2] + f.x_axis[2] * p[0] + f.y_axis[2] * p[1],
            )
        })
        .collect();
    (feature.definition_id(), points)
}

fn assert_corner(points: &[Vec3], expected: Vec3) {
    assert!(
        points.iter().any(|p| {
            let d = *p - expected;
            d.x.abs() < 1e-6 && d.y.abs() < 1e-6 && d.z.abs() < 1e-6
        }),
        "missing {expected:?} in {points:?}"
    );
}

fn draw(shell: &mut Shell, plane: PrincipalPlane) -> [Vec3; 4] {
    tool(shell, plane);
    let start = Vec3::ZERO;
    let end = match plane {
        PrincipalPlane::Xy => Vec3::new(30.0, 15.0, 0.0),
        PrincipalPlane::Xz => Vec3::new(30.0, 0.0, 15.0),
        PrincipalPlane::Yz => Vec3::new(0.0, 30.0, 15.0),
    };
    let before = shell.app().canonical_digest();
    let steps = shell.app().undo_step_count();
    for cancel in [true, false] {
        shell.click_command(AppCommand::Rectangle);
        let a = shell.app().viewport_position(start).unwrap();
        let b = shell.app().viewport_position(end).unwrap();
        assert!(shell.viewport_rect().contains(a));
        assert!(shell.viewport_rect().contains(b));
        shell.click_at(a);
        shell.move_pointer(b);
        assert_eq!(shell.app().canonical_digest(), before);
        assert_eq!(shell.app().undo_step_count(), steps);
        if cancel {
            shell.press_key(Key::Escape);
            assert_eq!(shell.app().canonical_digest(), before);
        } else {
            shell.type_text("30,15");
            shell.press_key(Key::Enter);
        }
    }
    assert_eq!(
        shell.app().undo_step_count(),
        steps + 1,
        "{}",
        shell.app().action_digest()
    );
    let after = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), after);
    [
        start,
        if plane == PrincipalPlane::Xy {
            Vec3::new(end.x, 0.0, 0.0)
        } else {
            Vec3::new(end.x, end.y, 0.0)
        },
        end,
        if plane == PrincipalPlane::Xy {
            Vec3::new(0.0, end.y, 0.0)
        } else {
            Vec3::new(0.0, 0.0, end.z)
        },
    ]
}

#[test]
fn empty_document_datum_rectangle_has_exact_corners_and_one_persisted_undo() {
    for plane in [PrincipalPlane::Xz, PrincipalPlane::Yz] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty-rectangle.ketchup");
        let mut shell = Shell::with_dialogs(
            ScriptedFileDialogs::new()
                .queue_save(&path)
                .queue_open(&path)
                .always_discard(),
        );
        shell.click_menu_command("menu-file", AppCommand::New);
        assert_eq!(shell.app().document_snapshot().definitions().count(), 0);
        let expected = draw(&mut shell, plane);
        let (definition, points) = local_corners(&shell);
        let snapshot = shell.app().document_snapshot();
        assert_eq!(snapshot.occurrences().count(), 1);
        let occurrence = snapshot.occurrences().next().unwrap();
        assert_eq!(occurrence.definition_id(), definition);
        let world = snapshot
            .world_transform_for_occurrence(occurrence.id())
            .unwrap();
        let points: Vec<_> = points.into_iter().map(|p| transform(world, p)).collect();
        for p in expected {
            assert_corner(&points, p);
        }
        let digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-file", AppCommand::SaveAs);
        shell.click_menu_command("menu-file", AppCommand::New);
        shell.click_menu_command("menu-file", AppCommand::Open);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().document_snapshot().occurrences().count(), 1);
    }
}

fn fixture(path: &std::path::Path) -> Transform {
    let mut document = DocumentStore::new();
    let outer = Transform::from_translation(30.0, -40.0, 20.0)
        .unwrap()
        .compose(
            world_axis_rotation_transform(Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0), 90.0).unwrap(),
        );
    let inner = Transform::from_translation(10.0, 20.0, 30.0)
        .unwrap()
        .compose(
            world_axis_rotation_transform(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 90.0).unwrap(),
        );
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Unrelated first definition".into(),
        },
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(2),
            name: "Shared part".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(1),
            definition_id: DefinitionId(2),
            name: "Profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(2),
            definition_id: DefinitionId(2),
            name: "Body".into(),
            kind: FeatureKind::Extrusion {
                profile: FeatureId(1),
                height: Dimension::from_decimal("20").unwrap(),
            },
        },
        CanonicalCommand::CreateGroup {
            id: GroupId(10),
            name: "Outer".into(),
            parent: None,
            transform: outer,
        },
        CanonicalCommand::CreateGroup {
            id: GroupId(20),
            name: "Inner".into(),
            parent: Some(GroupId(10)),
            transform: inner,
        },
    ];
    for (id, t) in [
        (1, Transform::identity()),
        (2, Transform::from_translation(180.0, 90.0, 50.0).unwrap()),
    ] {
        commands.push(CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(2),
            name: format!("Shared {id}"),
            transform: t,
            parent: Some(GroupId(20)),
            tag: None,
            visible: true,
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    persistence::save_atomic(path, &document.current()).unwrap();
    outer.compose(inner)
}
#[test]
fn group_datum_rectangle_creates_a_separate_part_in_parent_space() {
    for plane in [PrincipalPlane::Xz, PrincipalPlane::Yz] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("group.ketchup");
        fixture(&path);
        let mut shell = Shell::with_dialogs(
            ScriptedFileDialogs::new()
                .queue_open(&path)
                .always_discard(),
        );
        shell.click_menu_command("menu-file", AppCommand::Open);
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        let centre = shell.top_face_centre(1);
        shell.double_click_at(centre);
        assert_eq!(shell.app().edit_context_depth(), 1);
        let before = shell.app().document_snapshot();
        let expected = draw(&mut shell, plane);
        let (definition, local) = local_corners(&shell);
        let snapshot = shell.app().document_snapshot();
        assert_eq!(snapshot.definitions().count(), 3);
        let created = snapshot
            .occurrences()
            .find(|o| o.definition_id() == definition)
            .unwrap();
        assert_eq!(created.parent(), Some(GroupId(20)));
        let world = snapshot
            .world_transform_for_occurrence(created.id())
            .unwrap();
        let points: Vec<_> = local.into_iter().map(|p| transform(world, p)).collect();
        for p in expected {
            assert_corner(&points, p);
        }
        for feature in before.features() {
            assert_eq!(
                snapshot.feature(feature.id()).unwrap().kind(),
                feature.kind()
            );
        }
    }
}
#[test]
fn standalone_datum_rectangle_can_be_hovered_and_push_pulled() {
    for plane in [PrincipalPlane::Xz, PrincipalPlane::Yz] {
        let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().always_discard());
        shell.click_menu_command("menu-file", AppCommand::New);
        let points = draw(&mut shell, plane);
        let centre = (points[0] + points[2]) * 0.5;
        shell.click_command(AppCommand::Select);
        let screen = shell.app().viewport_position(centre).unwrap();
        shell.move_pointer(screen);
        assert!(
            shell.app().hovered_selection().is_some(),
            "standalone {plane:?} rectangle must expose its face for selection"
        );
        shell.click_at(screen);
        let before = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.press_key(Key::P);
        shell.type_text("10");
        assert_eq!(shell.app().value_input(), "10");
        shell.press_key(Key::Enter);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        let snapshot = shell.app().document_snapshot();
        let producer = snapshot.features().last().unwrap();
        assert!(
            matches!(producer.kind(), FeatureKind::Pad(_)),
            "rectangle must support Push/Pull: {}",
            shell.app().action_digest()
        );
        let graph =
            ExactBRepGraph::from_snapshot(&snapshot, producer.definition_id(), producer.id())
                .unwrap();
        assert_eq!(
            graph.producer_bounds_mm().unwrap().unwrap(),
            match plane {
                PrincipalPlane::Xz => [[0.0, -10.0, 0.0], [30.0, 0.0, 15.0]],
                PrincipalPlane::Yz => [[0.0, 0.0, 0.0], [10.0, 30.0, 15.0]],
                PrincipalPlane::Xy => unreachable!(),
            }
        );
        let after = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), after);
    }
}

#[test]
fn nested_shared_definition_rectangle_uses_active_context_and_world_to_local_frame() {
    for plane in [PrincipalPlane::Xz, PrincipalPlane::Yz] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("nested-source.ketchup");
        let saved = dir.path().join("nested-saved.ketchup");
        let world = fixture(&source);
        let mut shell = Shell::with_dialogs(
            ScriptedFileDialogs::new()
                .queue_open(&source)
                .queue_save(&saved)
                .queue_open(&saved)
                .always_discard(),
        );
        shell.click_menu_command("menu-file", AppCommand::Open);
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        let centre = shell.top_face_centre(1);
        shell.double_click_at(centre);
        assert_eq!(shell.app().edit_context_depth(), 1);
        shell.double_click_at(centre);
        assert_eq!(shell.app().edit_context_depth(), 2);
        let expected = draw(&mut shell, plane);
        let (definition, local) = local_corners(&shell);
        assert_eq!(
            definition,
            DefinitionId(2),
            "must edit active shared definition, not first definition"
        );
        let snapshot = shell.app().document_snapshot();
        assert_eq!(snapshot.definitions().count(), 2);
        assert_eq!(snapshot.occurrences().count(), 2);
        assert_eq!(
            snapshot
                .features()
                .filter(|f| f.definition_id() == DefinitionId(1))
                .count(),
            0
        );
        let inverse = world.rigid_inverse().unwrap();
        for p in expected {
            assert_corner(&local, transform(inverse, p));
        }
        let actual: Vec<_> = local.iter().map(|p| transform(world, *p)).collect();
        for p in expected {
            assert_corner(&actual, p);
        }
        let second = snapshot
            .world_transform_for_occurrence(OccurrenceId(2))
            .unwrap();
        let shared: Vec<_> = local.iter().map(|p| transform(second, *p)).collect();
        for p in expected {
            assert_corner(&shared, transform(second, transform(inverse, p)));
        }
        let digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-file", AppCommand::SaveAs);
        shell.click_menu_command("menu-file", AppCommand::New);
        shell.click_menu_command("menu-file", AppCommand::Open);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(local_corners(&shell), (definition, local));
    }
}
