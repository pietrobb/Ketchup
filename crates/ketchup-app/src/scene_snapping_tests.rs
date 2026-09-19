use super::*;

fn viewport() -> Rect {
    Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0))
}

#[test]
fn own_polygon_snap_is_shared_by_hover_rectangle_and_point_input() {
    for points in [
        vec![[0.0, 0.0], [80.0, 0.0], [80.0, 60.0], [0.0, 60.0]],
        vec![[0.0, 0.0], [80.0, 0.0], [25.0, 60.0]],
    ] {
        let mut app = KetchupApp::new();
        app.new_document();
        let origin = Vec3::new(30.0, 25.0, 0.0);
        let corner = origin + Vec3::new(points[2][0], points[2][1], 0.0);
        assert!(app.create_profile_at(origin, points));
        let rect = viewport();
        for (point, kind) in [
            (corner, SnapKind::Endpoint),
            (
                (origin + Vec3::new(80.0, 0.0, 0.0) + corner) * 0.5,
                SnapKind::Midpoint,
            ),
        ] {
            let pointer = app.project(point, rect) + Vec2::new(2.0, 1.0);
            for tool in [
                ActiveTool::Rectangle,
                ActiveTool::Line,
                ActiveTool::Circle,
                ActiveTool::Arc,
            ] {
                app.active_tool = tool;
                app.update_viewport_inference(Some(pointer), rect);
                assert_eq!(app.hovered_snap_kind(), Some(kind), "{tool:?} {point:?}");
                assert_eq!(app.hovered_snap_position(), Some(point));
                let input = if tool == ActiveTool::Rectangle {
                    app.rectangle_input_point(pointer, rect)
                } else {
                    app.viewport_point_at_screen(pointer, rect, 0.0)
                };
                assert_eq!(input, Some(point), "hover must equal input");
            }
        }
    }
}

#[test]
fn rectangle_acquires_world_axis_away_from_origin() {
    let mut app = KetchupApp::new();
    app.new_document();
    app.active_tool = ActiveTool::Rectangle;
    let rect = viewport();
    for point in [Vec3::new(110.0, 0.0, 0.0), Vec3::new(0.0, 110.0, 0.0)] {
        let pointer = app.project(point, rect) + Vec2::new(2.0, 1.0);
        let actual = app.rectangle_input_point(pointer, rect).unwrap();
        assert!(
            actual.x.abs() < 1e-8 || actual.y.abs() < 1e-8,
            "axis input {actual:?}"
        );
        assert!(actual.distance(point) < 5.0);
    }
    for plane in [PrincipalPlane::Xy, PrincipalPlane::Xz, PrincipalPlane::Yz] {
        let frame = WorkplaneFrame::principal(plane);
        let d = frame.y_axis;
        let point = Vec3::new(d[0], d[1], d[2]) * 110.0;
        let pointer = app.project(point, rect) + Vec2::new(2.0, 1.0);
        let (actual, axis) = app
            .datum_snap_at_screen(pointer, rect, Some(frame))
            .unwrap();
        assert!(axis.is_some());
        assert!(rectangle_snapping::point_in_frame(actual, frame));
        assert!(cross(actual, point).length() < 1e-7);
    }
    app.active_tool = ActiveTool::Line;
    let z_pointer = app.project(Vec3::new(0.0, 0.0, 110.0), rect) + Vec2::new(2.0, 1.0);
    let on_z = app.viewport_point_at_screen(z_pointer, rect, 0.0).unwrap();
    assert!(on_z.x.abs() < 1e-8 && on_z.y.abs() < 1e-8 && (on_z.z - 110.0).abs() < 5.0);
    app.face_workflow.set_snaps_enabled(false);
    assert!(
        app.datum_snap_at_screen(app.project(Vec3::new(100.0, 0.0, 0.0), rect), rect, None)
            .is_none()
    );
}

#[test]
fn generic_snap_cache_follows_transform_visibility_and_history() {
    let mut app = KetchupApp::new();
    app.new_document();
    assert!(app.create_profile_at(Vec3::ZERO, vec![[0.0, 0.0], [80.0, 0.0], [25.0, 60.0]]));
    let rect = viewport();
    let corner = Vec3::new(25.0, 60.0, 0.0);
    let check = |app: &KetchupApp, p: Vec3| {
        app.scene_snap_at_screen(app.project(p, rect), rect, 8.0, None)
            .unwrap()
            .position_mm
    };
    assert_eq!(check(&app, corner), corner);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(1),
                transform: Transform::from_translation(40.0, 20.0, 15.0).unwrap(),
            },
        ]))
        .unwrap();
    let moved = corner + Vec3::new(40.0, 20.0, 15.0);
    assert_eq!(check(&app, moved), moved);
    app.document.undo().unwrap();
    assert_eq!(check(&app, corner), corner);
    app.document.redo().unwrap();
    assert_eq!(check(&app, moved), moved);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    assert!(
        app.scene_snap_at_screen(app.project(moved, rect), rect, 8.0, None)
            .is_none()
    );
    app.document.undo().unwrap();
    assert_eq!(check(&app, moved), moved);
}

#[test]
fn exact_prism_uses_native_edges_not_bounding_box_or_triangle_diagonals() {
    let mut app = planar_push_pull::tests::prism(
        &[[0.0, 0.0], [80.0, 0.0], [25.0, 60.0]],
        Transform::identity(),
    );
    let rect = viewport();
    app.active_tool = ActiveTool::Rectangle;
    for (point, kind) in [
        (Vec3::new(25.0, 60.0, 12.0), SnapKind::Endpoint),
        (Vec3::new(52.5, 30.0, 12.0), SnapKind::Midpoint),
    ] {
        let pointer = app.project(point, rect) + Vec2::new(2.0, 1.0);
        let snap = app.rectangle_snap_at_screen(pointer, rect).unwrap();
        assert_eq!(snap.kind, kind);
        assert!(snap.position_mm.distance(point) < 1e-7);
        assert!(
            app.rectangle_input_point(pointer, rect)
                .unwrap()
                .distance(point)
                < 1e-7
        );
    }
    let phantom = Vec3::new(80.0, 60.0, 12.0);
    assert!(
        app.scene_snap_at_screen(app.project(phantom, rect), rect, 8.0, None)
            .is_none()
    );
}

#[test]
fn curve_tessellation_never_becomes_endpoint_or_midpoint_and_center_survives_extrusion() {
    let mut app = KetchupApp::new();
    app.new_document();
    let center = Vec3::new(50.0, 40.0, 0.0);
    assert!(app.complete_circle(center, 25.0, Vec3::new(1.0, 0.0, 0.0)));
    let snapshot = app.document.current();
    let profile = snapshot.features().last().unwrap();
    app.document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(100),
            definition_id: profile.definition_id(),
            name: "Extrude".into(),
            kind: FeatureKind::Extrusion {
                profile: profile.id(),
                height: Dimension::new("15", 15.0).unwrap(),
            },
        }]))
        .unwrap();
    let rect = viewport();
    app.active_tool = ActiveTool::Rectangle;
    for z in [0.0, 15.0] {
        let c = center + Vec3::new(0.0, 0.0, z);
        let f = WorkplaneFrame::principal(PrincipalPlane::Xy).offset(z);
        let snap = app
            .scene_snap_at_screen(app.project(c, rect), rect, 8.0, Some(f))
            .unwrap();
        assert_eq!(snap.kind, SnapKind::Center);
        assert!(snap.position_mm.distance(c) < 1e-7);
        let angle = 0.31_f64;
        let p = c + Vec3::new(25.0 * angle.cos(), 25.0 * angle.sin(), 0.0);
        let snap = app
            .scene_snap_at_screen(app.project(p, rect), rect, 8.0, Some(f))
            .unwrap();
        assert_eq!(snap.kind, SnapKind::Edge);
        assert!(
            (snap.position_mm.distance(c) - 25.0).abs() < 1e-8,
            "circle snap must not lie on a tessellation chord"
        );
    }
}

#[test]
fn constrained_circle_snap_uses_solved_geometry_on_yz() {
    use ketchup_core::sketch::{
        SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntityId,
        SketchPointKind, SketchPointRef, SketchSpec, WorkplaneSpec,
    };
    let mut app = KetchupApp::new();
    app.new_document();
    let sketch = SketchSpec {
        workplane: FeatureId(1),
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [10.0, 20.0],
            radius_mm: 15.0,
        }],
        constraints: vec![SketchConstraint {
            id: SketchConstraintId(1),
            kind: SketchConstraintKind::FixedPoint {
                point: SketchPointRef {
                    entity: SketchEntityId(1),
                    point: SketchPointKind::Center,
                },
                position_mm: [40.0, 50.0],
            },
        }],
    };
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "YZ circle".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Circle".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Circle".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    let rect = viewport();
    for x in [0.0] {
        let point = Vec3::new(x, 40.0, 50.0);
        let snap = app
            .scene_snap_at_screen(app.project(point, rect), rect, 8.0, None)
            .unwrap();
        assert_eq!(snap.kind, SnapKind::Center);
        assert!(snap.position_mm.distance(point) < 1e-7);
    }
}
