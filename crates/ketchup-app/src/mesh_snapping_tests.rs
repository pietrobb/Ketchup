use super::*;

fn mesh_app(
    mut vertices_mm: Vec<[f64; 3]>,
    cap: Vec<[u32; 3]>,
    transform: Transform,
) -> KetchupApp {
    let count = vertices_mm.len() as u32;
    let mut boundary = BTreeMap::<[u32; 2], Vec<[u32; 2]>>::new();
    for &[a, b, c] in &cap {
        for [from, to] in [[a, b], [b, c], [c, a]] {
            boundary
                .entry([from.min(to), from.max(to)])
                .or_default()
                .push([from, to]);
        }
    }
    let mut triangles = Vec::new();
    for [a, b, c] in cap {
        triangles.push([c, b, a]);
        triangles.push([a + count, b + count, c + count]);
    }
    for uses in boundary.values().filter(|uses| uses.len() == 1) {
        let [a, b] = uses[0];
        triangles.extend([[a, b, b + count], [a, b + count, a + count]]);
    }
    let top: Vec<_> = vertices_mm
        .iter()
        .map(|p| [p[0], p[1], p[2] + 35.0])
        .collect();
    vertices_mm.extend(top);
    let mut app = KetchupApp::new();
    app.new_document();
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Mesh snap regression".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Mesh".into(),
                kind: FeatureKind::MeshBody(MeshBodySpec {
                    schema: MESH_BODY_SCHEMA_V1.into(),
                    vertices_mm,
                    triangles,
                    authority: MeshAuthority::Authored {
                        provenance: "Snap regression".into(),
                    },
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Mesh".into(),
                transform,
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    app
}

#[test]
fn acute_mesh_corner_keeps_endpoints_and_each_straight_edge_midpoint() {
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0));
    for transform in [
        Transform::identity(),
        Transform::from_translation(30.0, 25.0, 40.0).unwrap(),
    ] {
        let mut app = mesh_app(
            vec![
                [0.0, 0.0, 0.0],
                [120.0, -12.0, 0.0],
                [120.0, 12.0, 0.0],
                [30.0, -3.0, 0.0],
            ],
            vec![[0, 3, 2], [3, 1, 2]],
            transform,
        );
        for tool in [
            ActiveTool::Rectangle,
            ActiveTool::Line,
            ActiveTool::Move,
            ActiveTool::Rotate,
        ] {
            app.active_tool = tool;
            for (local, kind) in [
                (Vec3::ZERO, SnapKind::Endpoint),
                (Vec3::new(120.0, -12.0, 0.0), SnapKind::Endpoint),
                (Vec3::new(120.0, 12.0, 0.0), SnapKind::Endpoint),
                (Vec3::new(60.0, -6.0, 0.0), SnapKind::Midpoint),
                (Vec3::new(60.0, 6.0, 0.0), SnapKind::Midpoint),
                (Vec3::new(120.0, 0.0, 0.0), SnapKind::Midpoint),
                (Vec3::new(30.0, -3.0, 0.0), SnapKind::Edge),
            ] {
                let world = transform_model_point(transform, local);
                let snap = app
                    .scene_snap_at_screen(app.project(world, rect), rect, 0.5, None)
                    .unwrap();
                assert_eq!(snap.kind, kind, "{tool:?} at {world:?}");
                assert!(
                    snap.position_mm.distance(world) < 1e-4,
                    "{snap:?} at {world:?}"
                );
            }
        }
    }
}

#[test]
fn smooth_mesh_boundary_does_not_offer_tessellation_vertices_as_endpoints() {
    let mut vertices = vec![[0.0, 0.0, 0.0]];
    vertices.extend((0..64).map(|i| {
        let angle = f64::from(i) * std::f64::consts::TAU / 64.0;
        [50.0 * angle.cos(), 50.0 * angle.sin(), 0.0]
    }));
    let triangles = (0..64).map(|i| [0, i + 1, (i + 1) % 64 + 1]).collect();
    let sample = vertices[4];
    let app = mesh_app(vertices, triangles, Transform::identity());
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0));
    let point = Vec3::new(sample[0], sample[1], sample[2]);
    let snap = app
        .scene_snap_at_screen(app.project(point, rect), rect, 0.5, None)
        .unwrap();
    assert_eq!(snap.kind, SnapKind::Edge);
    assert!(snap.position_mm.distance(point) < 1e-4);
}

#[test]
fn acute_mesh_crease_is_kept_without_revealing_coplanar_diagonals() {
    let app = mesh_app(
        vec![[0.0, 0.0, 0.0], [120.0, -12.0, 0.0], [120.0, 12.0, 0.0]],
        vec![[0, 1, 2]],
        Transform::identity(),
    );
    let snapshot = app.document.current();
    let FeatureKind::MeshBody(mesh) = snapshot.feature(FeatureId(1)).unwrap().kind() else {
        panic!("expected canonical mesh fixture");
    };
    let triangles = &mesh.triangles;
    let positions: Vec<_> = mesh
        .vertices_mm
        .iter()
        .map(|p| p.map(|v| v as f32))
        .collect();
    for triangles in [
        triangles.clone(),
        triangles.iter().map(|t| [t[2], t[1], t[0]]).collect(),
    ] {
        let edges =
            renderer::feature_edges(&positions, &triangles, &vec![None::<u8>; triangles.len()]);
        assert!(edges.contains(&[0, 3]), "sharp crease missing: {edges:?}");
        assert_eq!(
            edges,
            BTreeSet::from([
                [0, 1],
                [0, 2],
                [1, 2],
                [3, 4],
                [3, 5],
                [4, 5],
                [0, 3],
                [1, 4],
                [2, 5]
            ]),
            "only the nine physical edges, for either consistent winding"
        );
    }
}

#[test]
fn acute_mesh_crease_provides_world_space_midpoint_and_edge_snaps() {
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 900.0));
    for transform in [
        Transform::identity(),
        Transform::from_translation(30.0, 25.0, 40.0).unwrap(),
    ] {
        let mut app = mesh_app(
            vec![[0.0, 0.0, 0.0], [120.0, -12.0, 0.0], [120.0, 12.0, 0.0]],
            vec![[0, 1, 2]],
            transform,
        );
        for tool in [
            ActiveTool::Rectangle,
            ActiveTool::Line,
            ActiveTool::Move,
            ActiveTool::Rotate,
            ActiveTool::PushPull,
        ] {
            app.active_tool = tool;
            for (height, kind) in [(17.5, SnapKind::Midpoint), (8.75, SnapKind::Edge)] {
                let world = transform_model_point(transform, Vec3::new(0.0, 0.0, height));
                let snap = app.scene_snap_at_screen(app.project(world, rect), rect, 0.5, None);
                assert!(
                    snap.is_some(),
                    "missing crease snap for {tool:?} at {world:?}"
                );
                let snap = snap.unwrap();
                assert_eq!(snap.kind, kind, "{tool:?} at {world:?}");
                assert!(
                    snap.position_mm.distance(world) < 1e-4,
                    "{snap:?} at {world:?}"
                );
            }
        }
    }
}
