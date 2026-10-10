use ketchup_manufacturing::blender_export::MeshGlbInstance;
use ketchup_manufacturing::viewer_export::{
    DefinitionMetadata, DesktopCameraContext, OccurrenceMetadata, ViewerExportData,
    model_viewer_export,
};
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    InstancePath, InstancePathStep, LocalGroupId, LocalGroupKey, LocalOccurrenceId,
    LocalOccurrenceKey, OccurrenceId, Transform,
};
use ketchup_model::exact_product::MeshExportSource;
use ketchup_model::testing::box_package;
use ketchup_view_format as view;
use std::collections::BTreeMap;

fn nested_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Reusable part".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(2),
            definition_id: DefinitionId(1),
            name: "Profile".into(),
            kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(3),
            definition_id: DefinitionId(1),
            name: "Solid".into(),
            kind: FeatureKind::extrusion(FeatureId(2), Dimension::from_decimal("10").unwrap()),
        },
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(5),
            name: "Assembly".into(),
        },
        CanonicalCommand::CreateLocalGroup {
            key: LocalGroupKey {
                definition_id: DefinitionId(5),
                local_id: LocalGroupId(6),
            },
            name: "Inner group".into(),
            transform: Transform::from_translation(5.0, 7.0, 9.0).unwrap(),
            parent: None,
        },
        CanonicalCommand::CreateLocalOccurrence {
            key: LocalOccurrenceKey {
                definition_id: DefinitionId(5),
                local_id: LocalOccurrenceId(7),
            },
            definition_id: DefinitionId(1),
            name: "Inner part".into(),
            transform: Transform::identity(),
            parent: Some(LocalGroupId(6)),
            tags: Default::default(),
            visible: true,
        },
    ];
    let angle = std::f64::consts::FRAC_PI_4;
    for (id, x) in [(10, 100.0), (11, 300.0)] {
        commands.push(CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(5),
            name: format!("Assembly {id}"),
            transform: Transform::from_matrix([
                angle.cos(),
                -angle.sin(),
                0.0,
                x,
                angle.sin(),
                angle.cos(),
                0.0,
                200.0,
                0.0,
                0.0,
                1.0,
                50.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ])
            .unwrap(),
            parent: None,
            tags: Default::default(),
            visible: true,
        });
    }
    commands.push(CanonicalCommand::SetOccurrenceColor {
        id: OccurrenceId(10),
        color: Some([12, 34, 56]),
    });
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

fn source_path(root: u64) -> InstancePath {
    InstancePath::root(OccurrenceId(root))
        .with_step(InstancePathStep::Group(LocalGroupId(6)))
        .with_step(InstancePathStep::Occurrence(LocalOccurrenceId(7)))
}

fn view_path(root: u64) -> view::InstancePath {
    view::InstancePath {
        root_occurrence_id: root,
        steps: vec![
            view::PathStep::Group {
                owner_definition_id: 5,
                local_id: 6,
            },
            view::PathStep::Occurrence {
                owner_definition_id: 5,
                local_id: 7,
            },
        ],
    }
}

fn roundtrip(package: &view::Package) -> view::Package {
    let mut bytes = Vec::new();
    package.write(&mut bytes).unwrap();
    view::Package::read(bytes.as_slice()).unwrap()
}

#[test]
fn manual_export_keeps_nested_names_colors_and_local_not_world_bounds() {
    let document = nested_document();
    let snapshot = document.current();
    let body = box_package(&snapshot, DefinitionId(1), FeatureId(3), "current", &[]).unwrap();
    let occurrences = snapshot.scene_query();
    let instances: Vec<_> = occurrences
        .iter()
        .filter(|item| item.definition_id == DefinitionId(1))
        .map(|occurrence| MeshGlbInstance {
            source: MeshExportSource::Exact(&body),
            occurrence,
        })
        .collect();
    let export = model_viewer_export(&snapshot, &instances, &ViewerExportData::default()).unwrap();
    let reopened = roundtrip(&export.package);
    assert_eq!(reopened, export.package);
    assert_eq!(
        reopened.manifest.occurrences.len(),
        4,
        "both ancestors and children"
    );
    let part = reopened
        .manifest
        .definitions
        .iter()
        .find(|item| item.id == 1)
        .unwrap();
    let [min_mm, max_mm] = body.bounds_mm();
    assert_eq!(
        part.local_dimensions,
        Some(view::LocalDimensions::LocalBounds { min_mm, max_mm })
    );
    assert_eq!(part.name, "Reusable part");
    assert!(part.material.is_none());
    let ancestor = reopened
        .manifest
        .definitions
        .iter()
        .find(|item| item.id == 5)
        .unwrap();
    assert_eq!(ancestor.name, "Assembly");
    assert!(ancestor.local_dimensions.is_none());
    assert!(ancestor.material.is_none());
    let inner = reopened
        .manifest
        .occurrences
        .iter()
        .find(|item| item.path == view_path(10))
        .unwrap();
    assert_eq!(inner.name, "Inner part");
    assert_eq!(inner.color_srgb, Some([12, 34, 56]));
    let geometry = reopened.geometry().expect("renderer geometry");
    let body_node = geometry
        .traversal
        .iter()
        .copied()
        .find(|&i| {
            (i == inner.glb_node || geometry.nodes[i].parent == Some(inner.glb_node))
                && geometry.nodes[i].mesh.is_some()
        })
        .expect("body at or below nested occurrence");
    let matrix = geometry.nodes[body_node].world_matrix;
    let actual_mm = [
        matrix[12] * 1000.0,
        -matrix[14] * 1000.0,
        matrix[13] * 1000.0,
    ];
    let expected_mm = snapshot
        .resolve_instance_path(&source_path(10))
        .unwrap()
        .world_transform
        .transform_point([0.0; 3]);
    for (actual, expected) in actual_mm.into_iter().zip(expected_mm) {
        assert!((actual - expected).abs() < 1e-8);
    }
    let mesh = geometry.nodes[body_node].mesh.expect("mesh");
    assert_eq!(
        geometry.meshes[mesh][0].topology,
        view::PrimitiveTopology::Triangles
    );
    assert_eq!(geometry.meshes[mesh][0].indices().len(), 36);
    let other = reopened
        .manifest
        .occurrences
        .iter()
        .find(|item| item.path == view_path(11))
        .unwrap();
    assert!(other.color_srgb.is_none());
    assert!(
        reopened
            .manifest
            .occurrences
            .iter()
            .any(|item| item.name == "Assembly 10")
    );
    assert_eq!(
        snapshot.document_id().0,
        reopened.manifest.source.document_id
    );
    assert_eq!(snapshot.revision_id(), reopened.manifest.source.revision);
    assert_eq!(
        document
            .current()
            .resolve_instance_path(&source_path(10))
            .unwrap()
            .world_transform,
        snapshot
            .resolve_instance_path(&source_path(10))
            .unwrap()
            .world_transform
    );
    assert!(export.loss_report.contains("topology_loss="));
}

#[test]
fn authored_definition_and_instance_notes_scenes_and_units_survive_export() {
    let document = nested_document();
    let snapshot = document.current();
    let body = box_package(&snapshot, DefinitionId(1), FeatureId(3), "current", &[]).unwrap();
    let occurrences = snapshot.scene_query();
    let instances: Vec<_> = occurrences
        .iter()
        .filter(|item| item.definition_id == DefinitionId(1))
        .map(|occurrence| MeshGlbInstance {
            source: MeshExportSource::Exact(&body),
            occurrence,
        })
        .collect();
    let mut data = ViewerExportData::default();
    data.definitions.insert(
        DefinitionId(1),
        DefinitionMetadata {
            note: Some("Shared note — hrúbka".into()),
            material: Some("HPL".into()),
            authored_size_mm: Some([10.0, 20.0, 30.0]),
        },
    );
    data.occurrences.insert(
        source_path(10),
        OccurrenceMetadata {
            note: Some("Only this instance".into()),
            material: Some("Painted HPL".into()),
            attributes: BTreeMap::from([("texture_axis".into(), "z".into())]),
        },
    );
    data.scenes.push(view::Scene {
        id: 1,
        name: "Detail".into(),
        camera: view::Camera {
            eye_mm: [200.0, -300.0, 100.0],
            target_mm: [100.0, 200.0, 50.0],
            up: [0.0, 0.0, 1.0],
            projection: view::Projection::Orthographic {
                short_span_mm: 400.0,
            },
        },
        hidden: vec![view::InstancePath {
            root_occurrence_id: 11,
            steps: vec![],
        }],
        style: view::DisplayStyle::ShadedEdges,
        section: Some(view::Section {
            normal: [0.0, 0.0, 1.0],
            offset_mm: 55.0,
        }),
        visible_dimensions: vec![4],
    });
    data.dimensions.push(view::Dimension {
        id: 4,
        from: view::Anchor {
            occurrence: Some(view_path(10)),
            point_mm: [0.0; 3],
        },
        to: view::Anchor {
            occurrence: Some(view_path(10)),
            point_mm: [10.0, 0.0, 0.0],
        },
        label_offset_mm: [0.0, 3.0, 0.0],
        value_mm: 10.0,
        display_unit: view::LengthUnit::Inch,
    });
    data.start_scene = Some(1);
    let export = model_viewer_export(&snapshot, &instances, &data).unwrap();
    let reopened = roundtrip(&export.package);
    let part = reopened
        .manifest
        .definitions
        .iter()
        .find(|item| item.id == 1)
        .unwrap();
    assert_eq!(part.note.as_deref(), Some("Shared note — hrúbka"));
    assert_eq!(part.material.as_deref(), Some("HPL"));
    assert_eq!(
        part.local_dimensions,
        Some(view::LocalDimensions::AuthoredAxes {
            size_mm: [10.0, 20.0, 30.0]
        })
    );
    let first = reopened
        .manifest
        .occurrences
        .iter()
        .find(|item| item.path == view_path(10))
        .unwrap();
    let second = reopened
        .manifest
        .occurrences
        .iter()
        .find(|item| item.path == view_path(11))
        .unwrap();
    assert_eq!(first.note.as_deref(), Some("Only this instance"));
    assert_eq!(first.material.as_deref(), Some("Painted HPL"));
    assert_eq!(
        first.attributes.get("texture_axis").map(String::as_str),
        Some("z")
    );
    assert!(second.note.is_none());
    assert!(second.material.is_none());
    assert_eq!(reopened.manifest.scenes, data.scenes);
    assert_eq!(reopened.manifest.dimensions, data.dimensions);
    assert_eq!(reopened.manifest.start_scene, Some(1));
}

fn glb_document(glb: &[u8]) -> serde_json::Value {
    let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&glb[20..20 + length]).unwrap()
}

fn glb_indices(glb: &[u8], doc: &serde_json::Value, accessor: usize) -> Vec<u32> {
    let json_length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let accessor = &doc["accessors"][accessor];
    let view = &doc["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    let offset = 28
        + json_length
        + view["byteOffset"].as_u64().unwrap() as usize
        + accessor["byteOffset"].as_u64().unwrap() as usize;
    let count = accessor["count"].as_u64().unwrap() as usize;
    glb[offset..offset + count * 4]
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
        .collect()
}

#[test]
fn nested_export_shares_twelve_face_boundary_edges_without_triangle_diagonals() {
    let document = nested_document();
    let snapshot = document.current();
    let body = box_package(&snapshot, DefinitionId(1), FeatureId(3), "current", &[]).unwrap();
    let occurrences = snapshot.scene_query();
    let instances: Vec<_> = occurrences
        .iter()
        .filter(|item| item.definition_id == DefinitionId(1))
        .map(|occurrence| MeshGlbInstance {
            source: MeshExportSource::Exact(&body),
            occurrence,
        })
        .collect();
    let export = model_viewer_export(&snapshot, &instances, &ViewerExportData::default()).unwrap();
    let reopened = roundtrip(&export.package);
    let doc = glb_document(&reopened.glb);
    let meshes = doc["meshes"].as_array().unwrap();
    assert_eq!(meshes.len(), 2, "colors differ, geometry stays shared");
    let first = &meshes[0]["primitives"];
    let second = &meshes[1]["primitives"];
    assert_eq!(first[1]["mode"], 1);
    assert_eq!(first[1]["indices"], second[1]["indices"]);
    assert_eq!(
        first[1]["attributes"]["POSITION"],
        first[0]["attributes"]["POSITION"]
    );
    assert_eq!(
        first[1]["attributes"]["POSITION"],
        second[1]["attributes"]["POSITION"]
    );
    let indices = glb_indices(
        &reopened.glb,
        &doc,
        first[1]["indices"].as_u64().unwrap() as usize,
    );
    assert_eq!(
        indices.len(),
        24,
        "twelve edges, not eighteen triangulation edges"
    );
    let mut unique = std::collections::BTreeSet::new();
    for pair in indices.chunks_exact(2) {
        assert!(unique.insert([pair[0], pair[1]]));
        let a = body.vertices()[pair[0] as usize].position_mm;
        let b = body.vertices()[pair[1] as usize].position_mm;
        assert_eq!(
            (0..3).filter(|&axis| a[axis] != b[axis]).count(),
            1,
            "no face diagonal exported"
        );
    }
    let original =
        ketchup_manufacturing::blender_export::model_glb_export(&snapshot, &instances).unwrap();
    assert!(
        glb_document(&original.glb)["meshes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|mesh| mesh["primitives"].as_array().unwrap().len() == 1)
    );
}

#[test]
fn canonical_mesh_edges_hide_coplanar_diagonal_and_keep_a_fold() {
    use ketchup_model::document::{MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec};
    let fixture = nested_document();
    let body = box_package(
        &fixture.current(),
        DefinitionId(1),
        FeatureId(3),
        "mesh source",
        &[],
    )
    .unwrap();
    for (last_z, expected) in [(10.0, 12), (20.0, 13)] {
        let mut vertices: Vec<_> = body
            .vertices()
            .iter()
            .map(|vertex| vertex.position_mm)
            .collect();
        vertices[7][2] = last_z;
        let mesh = MeshBodySpec {
            schema: MESH_BODY_SCHEMA_V1.into(),
            vertices_mm: vertices,
            triangles: body
                .triangles()
                .iter()
                .map(|triangle| triangle.vertex_indices)
                .collect(),
            authority: MeshAuthority::Authored {
                provenance: "test mesh".into(),
            },
        };
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Surface".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(2),
                    definition_id: DefinitionId(1),
                    name: "Mesh".into(),
                    kind: FeatureKind::MeshBody(mesh.clone()),
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(10),
                    definition_id: DefinitionId(1),
                    name: "Surface instance".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tags: Default::default(),
                    visible: true,
                },
            ]))
            .unwrap();
        let snapshot = document.current();
        let occurrences = snapshot.scene_query();
        let instances = [MeshGlbInstance {
            source: MeshExportSource::Canonical {
                definition_id: DefinitionId(1),
                producer_feature_id: FeatureId(2),
                mesh: &mesh,
            },
            occurrence: &occurrences[0],
        }];
        let export =
            model_viewer_export(&snapshot, &instances, &ViewerExportData::default()).unwrap();
        let reopened = roundtrip(&export.package);
        let doc = glb_document(&reopened.glb);
        let primitive = &doc["meshes"][0]["primitives"][1];
        let indices = glb_indices(
            &reopened.glb,
            &doc,
            primitive["indices"].as_u64().unwrap() as usize,
        );
        assert_eq!(indices.len(), expected * 2);
        assert_eq!(
            indices.chunks_exact(2).any(|edge| edge == [4, 7]),
            last_z != 10.0
        );
    }
}

#[test]
fn invalid_authored_dimensions_and_foreign_scene_references_are_refused() {
    let document = nested_document();
    let snapshot = document.current();
    let body = box_package(&snapshot, DefinitionId(1), FeatureId(3), "current", &[]).unwrap();
    let occurrences = snapshot.scene_query();
    let instances: Vec<_> = occurrences
        .iter()
        .filter(|item| item.definition_id == DefinitionId(1))
        .map(|occurrence| MeshGlbInstance {
            source: MeshExportSource::Exact(&body),
            occurrence,
        })
        .collect();
    for size in [[f64::NAN, 10.0, 10.0], [-1.0, 10.0, 10.0]] {
        let mut data = ViewerExportData::default();
        data.definitions.insert(
            DefinitionId(1),
            DefinitionMetadata {
                authored_size_mm: Some(size),
                ..Default::default()
            },
        );
        assert!(model_viewer_export(&snapshot, &instances, &data).is_err());
    }
    let data = ViewerExportData {
        start_scene: Some(999),
        ..Default::default()
    };
    assert!(model_viewer_export(&snapshot, &instances, &data).is_err());
    let data = ViewerExportData {
        dimensions: vec![view::Dimension {
            id: 4,
            from: view::Anchor {
                occurrence: Some(view_path(999)),
                point_mm: [0.0; 3],
            },
            to: view::Anchor {
                occurrence: None,
                point_mm: [1.0; 3],
            },
            label_offset_mm: [0.0; 3],
            value_mm: 1.0,
            display_unit: view::LengthUnit::Millimetre,
        }],
        ..Default::default()
    };
    assert!(model_viewer_export(&snapshot, &instances, &data).is_err());
}

#[test]
fn changed_source_never_exports_old_nested_placements() {
    let mut document = nested_document();
    let old = document.current();
    let body = box_package(&old, DefinitionId(1), FeatureId(3), "current", &[]).unwrap();
    let occurrences = old.scene_query();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: OccurrenceId(10),
                transform: Transform::from_translation(900.0, 800.0, 700.0).unwrap(),
            },
        ]))
        .unwrap();
    let instances: Vec<_> = occurrences
        .iter()
        .filter(|item| item.definition_id == DefinitionId(1))
        .map(|occurrence| MeshGlbInstance {
            source: MeshExportSource::Exact(&body),
            occurrence,
        })
        .collect();
    let result = model_viewer_export(
        &document.current(),
        &instances,
        &ViewerExportData::default(),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("exported stale placements"),
    };
    assert!(
        !error.causes().is_empty(),
        "preserves geometry export cause"
    );
}

fn desktop_camera(parallel: bool) -> ketchup_model::document::SavedCamera {
    ketchup_model::document::SavedCamera {
        parallel,
        yaw_rad: 0.0,
        pitch_rad: std::f64::consts::FRAC_PI_2,
        target_z_mm: 40.0,
        zoom: 2.0,
        pan: [120.0, -60.0],
    }
}

fn camera_context() -> DesktopCameraContext {
    DesktopCameraContext {
        viewport_points: [1200.0, 600.0],
        orbit_target_xy_mm: [20.0, 30.0],
        points_per_mm: 600.0 * 2.0 / 420.0,
        eye_distance_mm: 500.0,
    }
}

// Independent pinhole/parallel projection of the serialized world-space camera.
fn portable_screen(camera: &view::Camera, point: [f64; 3], size: [f64; 2]) -> [f64; 2] {
    use ketchup_geometry::linalg::Vec3;
    let eye = Vec3::from(camera.eye_mm);
    let forward = Vec3::from(camera.target_mm) - eye;
    let forward = forward / forward.length();
    let right = forward.cross(Vec3::from(camera.up));
    let right = right / right.length();
    let up = right.cross(forward);
    let relative = Vec3::from(point) - eye;
    let frame = camera.framing(size[0] / size[1]).unwrap();
    let (extent, shift) = match camera.projection {
        view::Projection::Orthographic { .. } => (frame, [0.0; 2]),
        view::Projection::Perspective {
            lens_shift_short, ..
        } => {
            let depth = relative.dot(forward);
            (
                [2.0 * frame[0] * depth, 2.0 * frame[1] * depth],
                lens_shift_short,
            )
        }
    };
    let short = size[0].min(size[1]);
    [
        relative.dot(right) * size[0] / extent[0] + shift[0] * short,
        relative.dot(up) * size[1] / extent[1] + shift[1] * short,
    ]
}

#[test]
fn panned_camera_preserves_projection_at_multiple_depths_and_phone_framing() {
    let context = camera_context();
    for parallel in [true, false] {
        let camera = context.camera(&desktop_camera(parallel)).unwrap();
        for point in [[57.0, 13.0, 86.0], [-3.0, 120.0, 11.0], [20.0, 30.0, 40.0]] {
            let depth = 530.0 - point[1];
            let scale = if parallel {
                context.points_per_mm
            } else {
                context.points_per_mm * 500.0 / depth
            };
            let expected = [
                (point[0] - 20.0) * scale + 120.0,
                -(point[2] - 40.0) * scale + 60.0,
            ];
            for size in [[1200.0, 600.0], [360.0, 780.0]] {
                let actual = portable_screen(&camera, point, size);
                let ratio = size[0].min(size[1]) / 600.0;
                for axis in 0..2 {
                    assert!(
                        (actual[axis] - expected[axis] * ratio).abs() < 1e-9,
                        "{parallel} {point:?} {size:?}: {actual:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn camera_conversion_refuses_missing_or_invalid_viewport_context() {
    let saved = desktop_camera(false);
    let valid = camera_context();
    for context in [
        DesktopCameraContext {
            viewport_points: [0.0, 600.0],
            ..valid
        },
        DesktopCameraContext {
            viewport_points: [1200.0, f64::NAN],
            ..valid
        },
        DesktopCameraContext {
            points_per_mm: 0.0,
            ..valid
        },
        DesktopCameraContext {
            eye_distance_mm: f64::INFINITY,
            ..valid
        },
        DesktopCameraContext {
            eye_distance_mm: -1.0,
            ..valid
        },
    ] {
        assert!(context.camera(&saved).is_err());
    }
    for camera in [
        ketchup_model::document::SavedCamera {
            pan: [f64::NAN, 0.0],
            ..saved
        },
        ketchup_model::document::SavedCamera {
            yaw_rad: f64::NAN,
            ..saved
        },
        ketchup_model::document::SavedCamera {
            zoom: -1.0,
            ..saved
        },
    ] {
        assert!(valid.camera(&camera).is_err());
    }
}

#[test]
fn saved_scene_export_maps_root_and_shared_nested_layers_without_mutating_model() {
    use ketchup_model::document::{SavedView, SavedViewId, SectionPlane, TagId};
    let mut document = nested_document();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: TagId(21),
                name: "Root layer".into(),
                visible: true,
            },
            CanonicalCommand::CreateTag {
                id: TagId(22),
                name: "Nested layer".into(),
                visible: false,
            },
            CanonicalCommand::SetOccurrenceTags {
                id: OccurrenceId(10),
                tags: [TagId(21)].into(),
            },
            CanonicalCommand::SetLocalOccurrenceTags {
                key: LocalOccurrenceKey {
                    definition_id: DefinitionId(5),
                    local_id: LocalOccurrenceId(7),
                },
                tags: [TagId(22)].into(),
            },
            CanonicalCommand::UpsertSavedView(SavedView {
                id: SavedViewId(31),
                name: "Root hidden".into(),
                camera: desktop_camera(true),
                style: ["edges".into(), "xray".into()].into(),
                hidden_tags: [TagId(21)].into(),
                section: SectionPlane::new([2.0, 3.0, 55.0], [0.0, 0.0, 1.0]),
            }),
            CanonicalCommand::UpsertSavedView(SavedView {
                id: SavedViewId(32),
                name: "Nested hidden".into(),
                camera: desktop_camera(false),
                style: ["wireframe".into()].into(),
                hidden_tags: [TagId(22)].into(),
                section: None,
            }),
        ]))
        .unwrap();
    let snapshot = document.current();
    let body = box_package(&snapshot, DefinitionId(1), FeatureId(3), "current", &[]).unwrap();
    let occurrences = snapshot.scene_query();
    let instances: Vec<_> = occurrences
        .iter()
        .filter(|item| item.definition_id == DefinitionId(1))
        .map(|occurrence| MeshGlbInstance {
            source: MeshExportSource::Exact(&body),
            occurrence,
        })
        .collect();
    let data = ViewerExportData {
        saved_views: [
            (SavedViewId(31), camera_context()),
            (SavedViewId(32), camera_context()),
        ]
        .into(),
        start_scene: Some(31),
        ..Default::default()
    };
    assert!(
        ketchup_manufacturing::blender_export::model_glb_export(&snapshot, &instances).is_err(),
        "the original GLB export still refuses hidden geometry"
    );
    let export = model_viewer_export(&snapshot, &instances, &data).unwrap();
    let reopened = roundtrip(&export.package);
    assert_eq!(reopened, export.package);
    let scenes = &reopened.manifest.scenes;
    assert_eq!(scenes.len(), 2);
    assert_eq!(scenes[0].name, "Root hidden");
    assert_eq!(scenes[0].style, view::DisplayStyle::ShadedEdges);
    assert_eq!(
        scenes[0].hidden,
        vec![
            view::InstancePath {
                root_occurrence_id: 10,
                steps: vec![]
            },
            view_path(10),
        ]
    );
    assert_eq!(
        scenes[0].section,
        Some(view::Section {
            normal: [0.0, 0.0, 1.0],
            offset_mm: 55.0
        })
    );
    assert_eq!(scenes[1].style, view::DisplayStyle::Wireframe);
    assert_eq!(scenes[1].hidden, vec![view_path(10), view_path(11)]);
    assert!(scenes[1].section.is_none());
    assert_eq!(
        scenes[1].camera,
        camera_context().camera(&desktop_camera(false)).unwrap()
    );
    assert!(export.loss_report.contains("omitted_display_flags=xray"));
    assert!(!document.current().tag(TagId(22)).unwrap().visible());
    assert_eq!(
        document.current().saved_view(SavedViewId(31)).unwrap(),
        snapshot.saved_view(SavedViewId(31)).unwrap()
    );
    let data = ViewerExportData {
        saved_views: [(SavedViewId(999), camera_context())].into(),
        ..Default::default()
    };
    assert!(model_viewer_export(&snapshot, &instances, &data).is_err());
}
