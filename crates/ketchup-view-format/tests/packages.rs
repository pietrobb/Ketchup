use ketchup_view_format::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn path(root_occurrence_id: u64) -> InstancePath {
    InstancePath {
        root_occurrence_id,
        steps: vec![],
    }
}

fn camera() -> Camera {
    Camera {
        eye_mm: [200.0, -300.0, 200.0],
        target_mm: [0.0; 3],
        up: [0.0, 0.0, 1.0],
        projection: Projection::Orthographic {
            short_span_mm: 400.0,
        },
    }
}

fn manifest(digest: String, document_id: u64, revision: u64) -> Manifest {
    Manifest {
        source: Source {
            document_id,
            revision,
            canonical_digest: digest,
        },
        definitions: vec![Definition {
            id: 1,
            name: "Shared component".into(),
            note: Some("Author's definition note — mäkčeň".into()),
            material: Some("HPL".into()),
            color_srgb: Some([128, 80, 90]),
            local_dimensions: Some(LocalDimensions::AuthoredAxes { size_mm: [10.0; 3] }),
        }],
        occurrences: vec![10, 11]
            .into_iter()
            .enumerate()
            .map(|(glb_node, root)| Occurrence {
                name: format!("Occurrence {root}"),
                path: path(root),
                definition_id: 1,
                glb_node,
                note: (root == 11).then(|| "Instance note".into()),
                material: None,
                color_srgb: None,
                attributes: std::collections::BTreeMap::from([("texture_axis".into(), "x".into())]),
            })
            .collect(),
        scenes: vec![Scene {
            id: 1,
            name: "Detail".into(),
            camera: camera(),
            hidden: vec![path(11)],
            style: DisplayStyle::ShadedEdges,
            section: Some(Section {
                normal: [0.0, 0.0, 1.0],
                offset_mm: 8.0,
            }),
            visible_dimensions: vec![4],
        }],
        dimensions: vec![Dimension {
            id: 4,
            from: Anchor {
                occurrence: Some(path(10)),
                point_mm: [0.0; 3],
            },
            to: Anchor {
                occurrence: Some(path(10)),
                point_mm: [10.0, 0.0, 0.0],
            },
            label_offset_mm: [0.0, 3.0, 0.0],
            value_mm: 10.0,
            display_unit: LengthUnit::Inch,
        }],
        start_scene: Some(1),
    }
}

fn encode_glb(doc: &Value, binary: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(doc).expect("json");
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut data = b"glTF".to_vec();
    data.extend_from_slice(&2_u32.to_le_bytes());
    data.extend_from_slice(
        &u32::try_from(28 + json.len() + binary.len())
            .expect("length")
            .to_le_bytes(),
    );
    data.extend_from_slice(
        &u32::try_from(json.len())
            .expect("json length")
            .to_le_bytes(),
    );
    data.extend_from_slice(b"JSON");
    data.extend(json);
    data.extend_from_slice(
        &u32::try_from(binary.len())
            .expect("binary length")
            .to_le_bytes(),
    );
    data.extend_from_slice(b"BIN\0");
    data.extend_from_slice(binary);
    data
}

fn fixture() -> (Manifest, Value, Vec<u8>) {
    let digest = "a".repeat(16);
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let doc = json!({
        "asset": {"version":"2.0","generator":"Ketchup","extras":{
            "ketchupSourceDigest":digest,"ketchupSourceUnit":"millimetre","unit":"metre","upAxis":"Y"}},
        "scene":0,"scenes":[{"name":"Ketchup model","nodes":[0,1]}],
        "nodes":[{"name":"First","matrix":identity,"mesh":0,"extras":{"ketchupEntity":"occurrence","ketchupDefinitionId":1,"ketchupInstancePath":"occurrence:10"}},
                 {"name":"Second","matrix":identity,"mesh":0,"extras":{"ketchupEntity":"occurrence","ketchupDefinitionId":1,"ketchupInstancePath":"occurrence:11"}}],
        "meshes":[{"name":"Shared","primitives":[{"attributes":{"POSITION":0},"indices":1,"mode":4}]}],
        "buffers":[{"byteLength":48}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36,"target":34962},
                       {"buffer":0,"byteOffset":36,"byteLength":12,"target":34963}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
                     {"bufferView":1,"componentType":5125,"count":3,"type":"SCALAR"}]
    });
    let mut binary = Vec::new();
    for v in [0.0_f32, 0.0, 0.0, 0.01, 0.0, 0.0, 0.0, 0.01, 0.0] {
        binary.extend_from_slice(&v.to_le_bytes());
    }
    for index in [0_u32, 1, 2] {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    (manifest(digest, 22, 5), doc, binary)
}

fn raw_package(manifest: &[u8], glb: &[u8]) -> Vec<u8> {
    let mut bytes = b"KETCHUPVIEW".to_vec();
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(manifest.len())
            .expect("manifest length")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(&u32::try_from(glb.len()).expect("glb length").to_le_bytes());
    let mut checksum = Sha256::new();
    checksum.update(manifest);
    checksum.update(glb);
    bytes.extend_from_slice(&checksum.finalize());
    bytes.extend_from_slice(manifest);
    bytes.extend_from_slice(glb);
    bytes
}

fn bytes(package: &Package) -> Vec<u8> {
    let mut bytes = Vec::new();
    package.write(&mut bytes).expect("write");
    bytes
}

#[test]
fn renderer_decode_borrows_shared_buffers_and_composes_out_of_order_nodes() {
    let (mut manifest, mut doc, mut binary) = fixture();
    let mut first = doc["nodes"][0].clone();
    let mut second = doc["nodes"][1].clone();
    first["children"] = json!([0]);
    first["matrix"] = json!([
        0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.1, 0.2, 0.3, 1.0
    ]);
    second["children"] = json!([2]);
    first.as_object_mut().unwrap().remove("mesh");
    second.as_object_mut().unwrap().remove("mesh");
    let body = json!({"matrix":[1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0,
        0.0,0.0,1.0,0.0, 0.002,0.003,0.004,1.0],"mesh":0});
    doc["nodes"] = json!([body, first, body, second]);
    doc["scenes"][0]["nodes"] = json!([1, 3]);
    manifest.occurrences[0].glb_node = 1;
    manifest.occurrences[1].glb_node = 3;
    for index in [0_u32, 1] {
        binary.extend_from_slice(&index.to_le_bytes());
    }
    doc["buffers"][0]["byteLength"] = json!(56);
    doc["bufferViews"].as_array_mut().unwrap().push(json!({
        "buffer":0,"byteOffset":48,"byteLength":8,"target":34963}));
    doc["accessors"].as_array_mut().unwrap().push(json!({
        "bufferView":2,"componentType":5125,"count":2,"type":"SCALAR"}));
    doc["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(json!({
        "attributes":{"POSITION":0},"indices":2,"mode":1}));
    let package = Package {
        manifest,
        glb: encode_glb(&doc, &binary),
    };
    let saved = bytes(&package);
    let package = Package::read(saved.as_slice()).expect("package");
    let geometry = package.geometry().expect("renderer data");
    assert_eq!(geometry.nodes[0].mesh, geometry.nodes[2].mesh);
    assert_eq!(geometry.nodes[0].parent, Some(1));
    assert_eq!(geometry.traversal, [1, 3, 0, 2]);
    for (actual, expected) in geometry.nodes[0].world_matrix[12..15]
        .iter()
        .zip([0.104, 0.203, 0.298])
    {
        assert!((actual - expected).abs() < 1e-12);
    }
    let triangle = &geometry.meshes[0][0];
    let edge = &geometry.meshes[0][1];
    assert_eq!(triangle.topology, PrimitiveTopology::Triangles);
    assert_eq!(edge.topology, PrimitiveTopology::Lines);
    assert_eq!(
        triangle.position_bytes().as_ptr(),
        edge.position_bytes().as_ptr()
    );
    let start = package.glb.as_ptr() as usize;
    let data = triangle.position_bytes().as_ptr() as usize;
    assert!(data >= start && data + triangle.position_bytes().len() <= start + package.glb.len());
    assert_eq!(
        triangle.positions().collect::<Vec<_>>(),
        [[0.0, 0.0, 0.0], [0.01, 0.0, 0.0], [0.0, 0.01, 0.0]]
    );
    assert_eq!(triangle.indices().collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(edge.indices().collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn renderer_decode_revalidates_mutated_public_package_fields() {
    let (mut manifest, mut doc, binary) = fixture();
    manifest.occurrences[0].glb_node = usize::MAX;
    assert!(
        Package {
            manifest,
            glb: encode_glb(&doc, &binary)
        }
        .geometry()
        .is_err()
    );
    let (manifest, _, _) = fixture();
    doc["buffers"][0]["uri"] = json!("file:///private/model.bin");
    assert!(
        Package {
            manifest: manifest.clone(),
            glb: encode_glb(&doc, &binary)
        }
        .geometry()
        .is_err()
    );
    doc["buffers"][0].as_object_mut().unwrap().remove("uri");
    doc["accessors"][0]["count"] = json!(u32::MAX);
    assert!(
        Package {
            manifest: manifest.clone(),
            glb: encode_glb(&doc, &binary)
        }
        .geometry()
        .is_err()
    );
    doc["accessors"][0]["count"] = json!(3);
    let mut invalid = binary;
    invalid[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(
        Package {
            manifest,
            glb: encode_glb(&doc, &invalid)
        }
        .geometry()
        .is_err()
    );
}

#[test]
fn offline_roundtrip_preserves_scenes_notes_units_and_shared_occurrences() {
    let (manifest, doc, binary) = fixture();
    let package = Package {
        manifest,
        glb: encode_glb(&doc, &binary),
    };
    let saved = bytes(&package);
    let reopened = Package::read(saved.as_slice()).expect("read");
    assert_eq!(reopened, package);
    assert_eq!(
        reopened.manifest.occurrences[1].note.as_deref(),
        Some("Instance note")
    );
    assert_eq!(
        reopened.manifest.dimensions[0].display_unit,
        LengthUnit::Inch
    );
    assert_eq!(reopened.manifest.scenes[0].hidden, vec![path(11)]);
    assert_eq!(
        reopened.manifest.definitions[0].local_dimensions,
        Some(LocalDimensions::AuthoredAxes { size_mm: [10.0; 3] })
    );
}

#[test]
fn aspect_changes_preserve_short_axis_framing_without_pixel_state() {
    let mut camera = camera();
    assert_eq!(camera.framing(2.0).expect("wide"), [800.0, 400.0]);
    assert_eq!(camera.framing(0.5).expect("phone"), [400.0, 800.0]);
    camera.projection = Projection::Perspective {
        short_fov_radians: 1.0,
        lens_shift_short: [0.0; 2],
    };
    let wide = camera.framing(2.0).expect("wide");
    let phone = camera.framing(0.5).expect("phone");
    assert_eq!(wide[1], phone[0]);
    assert_eq!(wide[0], phone[1]);
    assert!(camera.framing(f64::NAN).is_err());
}

#[test]
fn corrupt_truncated_oversized_and_unknown_envelopes_are_refused() {
    let (manifest, doc, binary) = fixture();
    let original = bytes(&Package {
        manifest,
        glb: encode_glb(&doc, &binary),
    });
    for len in 0..original.len() {
        assert!(Package::read(&original[..len]).is_err(), "truncation {len}");
    }
    for index in [0, 11, 21, original.len() - 1] {
        let mut damaged = original.clone();
        damaged[index] ^= 0x80;
        assert!(Package::read(damaged.as_slice()).is_err());
    }
    let mut trailing = original.clone();
    trailing.push(0);
    assert!(Package::read(trailing.as_slice()).is_err());
    let mut oversized = original[..53].to_vec();
    oversized[13..17].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Package::read(oversized.as_slice()).is_err());
    oversized = original[..53].to_vec();
    oversized[17..21].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Package::read(oversized.as_slice()).is_err());
}

#[test]
fn source_references_finite_numbers_and_program_fields_are_checked() {
    let (manifest, doc, binary) = fixture();
    let glb = encode_glb(&doc, &binary);
    for mutate in [
        (|m: &mut Manifest| m.occurrences[0].definition_id = 99) as fn(&mut Manifest),
        |m| m.occurrences[0].glb_node = 99,
        |m| {
            m.occurrences[0]
                .attributes
                .insert("bad\0key".into(), "value".into());
        },
        |m| {
            m.occurrences[0]
                .attributes
                .insert("key".into(), "v".repeat(65_537));
        },
        |m| {
            m.occurrences[0].attributes = (0..1_025)
                .map(|i| (format!("key{i}"), "v".into()))
                .collect();
        },
        |m| m.occurrences[0].path.root_occurrence_id = 99,
        |m| m.scenes[0].hidden = vec![path(99)],
        |m| m.scenes[0].visible_dimensions = vec![99],
        |m| m.start_scene = Some(99),
        |m| m.dimensions[0].from.occurrence = Some(path(99)),
        |m| m.scenes[0].camera.eye_mm[0] = f64::NAN,
        |m| {
            m.scenes[0].camera.projection = Projection::Perspective {
                short_fov_radians: 0.8,
                lens_shift_short: [f64::NAN, 0.0],
            }
        },
        |m| {
            m.scenes[0].camera.projection = Projection::Perspective {
                short_fov_radians: 0.8,
                lens_shift_short: [0.0, 1e10],
            }
        },
        |m| m.source.canonical_digest = "b".repeat(16),
        |m| m.definitions.push(m.definitions[0].clone()),
        |m| {
            let mut same_name = m.definitions[0].clone();
            same_name.id = 2;
            m.definitions.push(same_name);
            m.occurrences[0].definition_id = 2;
        },
    ] {
        let mut invalid = manifest.clone();
        mutate(&mut invalid);
        let wire = raw_package(
            &serde_json::to_vec(&invalid).expect("hostile manifest"),
            &glb,
        );
        assert!(Package::read(wire.as_slice()).is_err());
        assert!(
            Package {
                manifest: invalid,
                glb: glb.clone()
            }
            .write(Vec::new())
            .is_err()
        );
    }
    let mut json = serde_json::to_value(&manifest).expect("manifest");
    json["program"] = json!("load external file");
    let invalid = raw_package(&serde_json::to_vec(&json).expect("json"), &glb);
    assert!(Package::read(invalid.as_slice()).is_err());
}

#[test]
fn glb_rejects_external_resources_overflow_indices_nan_and_hierarchy_cycles() {
    let (manifest, doc, binary) = fixture();
    let json = serde_json::to_vec(&manifest).expect("manifest");
    for mutate in [
        (|v: &mut Value| v["buffers"][0]["uri"] = json!("file:///private.bin")) as fn(&mut Value),
        |v| v["images"] = json!([{"uri":"https://example.invalid/image"}]),
        |v| v["extensionsRequired"] = json!(["unknown"]),
        |v| v["asset"]["extras"]["program"] = json!("executable input"),
        |v| v["nodes"][0]["extras"]["uri"] = json!("file:///private.bin"),
        |v| v["accessors"][0]["count"] = json!(u64::MAX),
        |v| v["bufferViews"][0]["byteOffset"] = json!(u64::MAX),
        |v| v["meshes"][0]["primitives"][0]["indices"] = json!(99),
        |v| {
            v["nodes"][0]["children"] = json!([1]);
            v["nodes"][1]["children"] = json!([0]);
            v["scenes"][0]["nodes"] = json!([]);
        },
    ] {
        let mut invalid = doc.clone();
        mutate(&mut invalid);
        let encoded = raw_package(&json, &encode_glb(&invalid, &binary));
        assert!(Package::read(encoded.as_slice()).is_err());
    }
    for (offset, value) in [(0, f32::NAN.to_le_bytes()), (36, 99_u32.to_le_bytes())] {
        let mut invalid = binary.clone();
        invalid[offset..offset + 4].copy_from_slice(&value);
        let encoded = raw_package(&json, &encode_glb(&doc, &invalid));
        assert!(Package::read(encoded.as_slice()).is_err());
    }
}

#[test]
fn existing_model_glb_export_is_accepted_without_flattening_or_new_identity() {
    use ketchup_manufacturing::blender_export::{ExactGlbInstance, exact_model_glb_export};
    use ketchup_model::document::{
        CanonicalCommand, CommandBatch, DefinitionId, DocumentStore, FeatureId, FeatureKind,
        GroupId, OccurrenceId, Transform,
    };
    use ketchup_model::testing::box_package;
    let mut document = DocumentStore::new();
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Shared component".into(),
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
            kind: FeatureKind::extrusion(
                FeatureId(2),
                ketchup_model::document::Dimension::from_decimal("10").expect("dimension"),
            ),
        },
    ];
    commands.push(CanonicalCommand::CreateGroup {
        id: GroupId(20),
        name: "Container".into(),
        transform: Transform::identity(),
        parent: None,
    });
    for (id, x) in [(10, 0.0), (11, 20.0)] {
        commands.push(CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(1),
            name: "Same occurrence name".into(),
            transform: Transform::from_translation(x, 0.0, 0.0).expect("transform"),
            parent: (id == 10).then_some(GroupId(20)),
            tags: Default::default(),
            visible: true,
        });
    }
    document
        .apply_batch(&CommandBatch::new(commands))
        .expect("document");
    let snapshot = document.current();
    let body = box_package(
        &snapshot,
        DefinitionId(1),
        FeatureId(3),
        "existing-test-result",
        &[],
    )
    .expect("mesh");
    let occurrences = snapshot.scene_query();
    let instances: Vec<_> = occurrences
        .iter()
        .map(|occurrence| ExactGlbInstance {
            package: &body,
            occurrence,
        })
        .collect();
    let export = exact_model_glb_export(&snapshot, &instances).expect("existing exporter");
    let mut manifest = manifest(
        snapshot.canonical_digest(),
        snapshot.document_id().0,
        snapshot.revision_id(),
    );
    for occurrence in &mut manifest.occurrences {
        let source = occurrences
            .iter()
            .find(|item| item.occurrence_id.0 == occurrence.path.root_occurrence_id)
            .expect("source occurrence");
        occurrence.glb_node = export.occurrence_nodes[&source.instance_path];
        occurrence.definition_id = source.definition_id.0;
    }
    let package = Package {
        manifest,
        glb: export.glb,
    };
    let saved = bytes(&package);
    let reopened = Package::read(saved.as_slice()).expect("existing GLB accepted");
    assert_eq!(reopened, package);
    let json_len = usize::try_from(u32::from_le_bytes(
        reopened.glb[12..16].try_into().expect("json length"),
    ))
    .expect("length");
    let json: Value = serde_json::from_slice(&reopened.glb[20..20 + json_len]).expect("glb JSON");
    assert_eq!(json["meshes"].as_array().expect("meshes").len(), 1);
    let first = reopened.manifest.occurrences[0].glb_node;
    let second = reopened.manifest.occurrences[1].glb_node;
    assert_eq!(first, 1);
    assert_eq!(second, 2);
    assert_eq!(json["nodes"][first]["name"], json["nodes"][second]["name"]);
    assert_eq!(json["nodes"][first]["mesh"], json["nodes"][second]["mesh"]);
    assert_eq!(json["nodes"][second]["matrix"][12], json!(0.02));
}

#[test]
fn nested_transforms_paths_and_line_primitives_roundtrip_without_flattening() {
    let (mut manifest, mut doc, binary) = fixture();
    let nested = InstancePath {
        root_occurrence_id: 10,
        steps: vec![
            PathStep::Group {
                owner_definition_id: 5,
                local_id: 6,
            },
            PathStep::Occurrence {
                owner_definition_id: 5,
                local_id: 7,
            },
        ],
    };
    let mut child = doc["nodes"][0].clone();
    child["extras"]["ketchupInstancePath"] = json!(nested.glb_key());
    child["extras"]["ketchupOwnerDefinitionId"] = json!(5);
    doc["nodes"][0]["extras"]["ketchupDefinitionId"] = json!(5);
    let mut owner = manifest.definitions[0].clone();
    owner.id = 5;
    owner.local_dimensions = None;
    manifest.definitions.push(owner);
    child["matrix"][12] = json!(0.05);
    doc["nodes"][0]
        .as_object_mut()
        .expect("node")
        .remove("mesh");
    let mut group = doc["nodes"][0].clone();
    group["children"] = json!([2]);
    group["extras"] = json!({
        "ketchupEntity":"group", "ketchupOwnerDefinitionId":5,
        "ketchupInstancePath":"occurrence:10/group:6"
    });
    doc["nodes"][0]["children"] = json!([3]);
    doc["nodes"]
        .as_array_mut()
        .expect("nodes")
        .extend([child, group]);
    manifest.occurrences[0].glb_node = 2;
    manifest.occurrences[0].path = nested.clone();
    manifest.dimensions[0].from.occurrence = Some(nested.clone());
    manifest.dimensions[0].to.occurrence = Some(nested.clone());
    doc["accessors"]
        .as_array_mut()
        .expect("accessors")
        .push(json!({"bufferView":1,"componentType":5125,"count":2,"type":"SCALAR"}));
    doc["meshes"][0]["primitives"]
        .as_array_mut()
        .expect("primitives")
        .push(json!({"attributes":{"POSITION":0},"indices":2,"mode":1}));
    let mut perspective = manifest.scenes[0].clone();
    perspective.id = 2;
    perspective.camera.projection = Projection::Perspective {
        short_fov_radians: 0.8,
        lens_shift_short: [0.25, -0.1],
    };
    manifest.scenes.push(perspective);
    let package = Package {
        manifest,
        glb: encode_glb(&doc, &binary),
    };
    let encoded = bytes(&package);
    let reopened = Package::read(encoded.as_slice()).expect("nested lines");
    assert_eq!(reopened, package);
    assert_eq!(
        reopened.manifest.dimensions[0].from.occurrence,
        Some(nested)
    );
    assert_eq!(
        reopened.manifest.definitions[0].local_dimensions,
        Some(LocalDimensions::AuthoredAxes { size_mm: [10.0; 3] })
    );
    assert_eq!(reopened.manifest.scenes.len(), 2);
    for mutate in [
        (|v: &mut Value| v["nodes"][2]["extras"]["ketchupDefinitionId"] = json!(5))
            as fn(&mut Value),
        |v| v["nodes"][2]["extras"]["ketchupOwnerDefinitionId"] = json!(1),
        |v| v["accessors"][2]["count"] = json!(3),
        |v| v["meshes"][0]["primitives"][1]["indices"] = json!(99),
        |v| v["meshes"][0]["primitives"][1]["attributes"]["POSITION"] = json!(2),
        |v| v["nodes"][0]["extras"]["ketchupDefinitionId"] = json!(1),
        |v| {
            v["nodes"][0]["children"] = json!([]);
            v["nodes"][1]["children"] = json!([3]);
        },
    ] {
        let mut changed = doc.clone();
        mutate(&mut changed);
        let invalid = raw_package(
            &serde_json::to_vec(&package.manifest).expect("manifest"),
            &encode_glb(&changed, &binary),
        );
        assert!(Package::read(invalid.as_slice()).is_err());
    }
    let mut wrong_owner = package.manifest.clone();
    if let PathStep::Occurrence {
        owner_definition_id,
        ..
    } = &mut wrong_owner.occurrences[0].path.steps[1]
    {
        *owner_definition_id = 1;
    }
    let invalid = raw_package(
        &serde_json::to_vec(&wrong_owner).expect("manifest"),
        &package.glb,
    );
    assert!(Package::read(invalid.as_slice()).is_err());
    doc["nodes"][0]["matrix"][0] = json!(1e200);
    doc["nodes"][2]["matrix"][0] = json!(1e200);
    let overflow = raw_package(
        &serde_json::to_vec(&package.manifest).expect("manifest"),
        &encode_glb(&doc, &binary),
    );
    assert!(Package::read(overflow.as_slice()).is_err());
}
