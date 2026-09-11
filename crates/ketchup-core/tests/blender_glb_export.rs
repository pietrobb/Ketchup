use ketchup_core::blender_export::{
    ExactGlbInstance, MAX_GLB_EXPORT_INSTANCES, exact_model_glb_export,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    GroupId, MeshAuthority, OccurrenceId, Transform,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactProductError,
    MAX_STL_EXPORT_INSTANCES, build_box_render_package, canonical_reference_lineage_digest,
    exact_model_stl_export,
};
use ketchup_core::import::{
    GlbImportError, ImportFormat, ImportLengthUnit, inspect_glb, plan_glb_import,
};
use ketchup_core::mesh_recognition::{
    MeshRecognition, MeshRecognitionCandidate, recognize_mesh_body,
};
use ketchup_core::persistence;
use serde_json::Value;

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(2);
const EXTRUSION: FeatureId = FeatureId(3);
const FIRST: OccurrenceId = OccurrenceId(10);
const SECOND: OccurrenceId = OccurrenceId(11);

fn seeded_document(first_color: Option<[u8; 3]>, second_color: Option<[u8; 3]>) -> DocumentStore {
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "Reusable beam".into(),
        },
        CanonicalCommand::CreateFeature {
            id: PROFILE,
            definition_id: DEFINITION,
            name: "Profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: EXTRUSION,
            definition_id: DEFINITION,
            name: "Extrusion".into(),
            kind: FeatureKind::Extrusion {
                profile: PROFILE,
                height: Dimension::from_decimal("10").unwrap(),
            },
        },
        CanonicalCommand::CreateOccurrence {
            id: FIRST,
            definition_id: DEFINITION,
            name: "Beam A".into(),
            transform: Transform::identity(),
            parent: None,
            tag: None,
            visible: true,
        },
        CanonicalCommand::CreateOccurrence {
            id: SECOND,
            definition_id: DEFINITION,
            name: "Beam B".into(),
            transform: Transform::from_translation(20.0, 30.0, 40.0).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        },
    ];
    if let Some(color) = first_color {
        commands.push(CanonicalCommand::SetOccurrenceColor {
            id: FIRST,
            color: Some(color),
        });
    }
    if let Some(color) = second_color {
        commands.push(CanonicalCommand::SetOccurrenceColor {
            id: SECOND,
            color: Some(color),
        });
    }
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document
}

fn current_package(snapshot: &ketchup_core::document::Snapshot) -> ExactBodyPackage {
    let request = ExactFeatureChainRequest::from_snapshot(snapshot, DEFINITION).unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                snapshot.document_id(),
                EXTRUSION,
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("geometry:{role:?}"),
        )
    });
    build_box_render_package(
        &request,
        "exact-input".into(),
        "shared-result".into(),
        "occt".into(),
        "r0".into(),
        [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        evidence,
    )
    .unwrap()
    .into()
}

fn sample_glb() -> Vec<u8> {
    let document = seeded_document(None, None);
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrences = snapshot.scene_query();
    let instances = occurrences
        .iter()
        .map(|occurrence| ExactGlbInstance {
            package: &package,
            occurrence,
        })
        .collect::<Vec<_>>();
    exact_model_glb_export(&snapshot, &instances).unwrap().glb
}

fn glb_json(glb: &[u8]) -> Value {
    assert_eq!(&glb[0..4], b"glTF");
    assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
    assert_eq!(
        u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize,
        glb.len()
    );
    let json_length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    assert_eq!(&glb[16..20], b"JSON");
    serde_json::from_slice(&glb[20..20 + json_length]).unwrap()
}

fn rewrite_glb_json(glb: &[u8], mutate: impl FnOnce(&mut Value)) -> Vec<u8> {
    let json_length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let binary_header = 20 + json_length;
    let binary_length =
        u32::from_le_bytes(glb[binary_header..binary_header + 4].try_into().unwrap()) as usize;
    assert_eq!(&glb[binary_header + 4..binary_header + 8], b"BIN\0");
    let binary = &glb[binary_header + 8..binary_header + 8 + binary_length];
    let mut json = glb_json(glb);
    mutate(&mut json);
    let mut json = serde_json::to_vec(&json).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut binary = binary.to_vec();
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let total_length = 12 + 8 + json.len() + 8 + binary.len();
    let mut encoded = Vec::with_capacity(total_length);
    encoded.extend_from_slice(b"glTF");
    encoded.extend_from_slice(&2_u32.to_le_bytes());
    encoded.extend_from_slice(&(total_length as u32).to_le_bytes());
    encoded.extend_from_slice(&(json.len() as u32).to_le_bytes());
    encoded.extend_from_slice(b"JSON");
    encoded.extend_from_slice(&json);
    encoded.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    encoded.extend_from_slice(b"BIN\0");
    encoded.extend_from_slice(&binary);
    encoded
}

#[test]
fn glb_preserves_named_transformed_instances_and_reuses_geometry() {
    let document = seeded_document(None, None);
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrences = snapshot.scene_query();
    let instances = occurrences
        .iter()
        .map(|occurrence| ExactGlbInstance {
            package: &package,
            occurrence,
        })
        .collect::<Vec<_>>();

    let first = exact_model_glb_export(&snapshot, &instances).unwrap();
    let second = exact_model_glb_export(&snapshot, &instances).unwrap();
    assert_eq!(first, second);
    let gltf = glb_json(&first.glb);

    assert_eq!(gltf["asset"]["version"], "2.0");
    assert_eq!(gltf["asset"]["extras"]["unit"], "metre");
    assert_eq!(gltf["asset"]["extras"]["upAxis"], "Y");
    assert_eq!(gltf["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(gltf["meshes"].as_array().unwrap().len(), 1);
    assert_eq!(gltf["accessors"].as_array().unwrap().len(), 2);
    assert_eq!(gltf["nodes"][0]["name"], "Beam A");
    assert_eq!(gltf["nodes"][1]["name"], "Beam B");
    assert_eq!(gltf["nodes"][0]["mesh"], 0);
    assert_eq!(gltf["nodes"][1]["mesh"], 0);
    assert_eq!(gltf["nodes"][1]["matrix"][12], 0.02);
    assert_eq!(gltf["nodes"][1]["matrix"][13], 0.04);
    assert_eq!(gltf["nodes"][1]["matrix"][14], -0.03);
    let minimum = gltf["accessors"][0]["min"].as_array().unwrap();
    let maximum = gltf["accessors"][0]["max"].as_array().unwrap();
    assert!((minimum[2].as_f64().unwrap() + 0.01).abs() < 1.0e-8);
    assert!((maximum[0].as_f64().unwrap() - 0.01).abs() < 1.0e-8);
    assert!((maximum[1].as_f64().unwrap() - 0.01).abs() < 1.0e-8);
    assert!(first.loss_report.contains("unique_geometry_count=1"));
    assert!(first.loss_report.contains("occurrence_body_count=2"));
}

#[test]
fn glb_preserves_group_hierarchy_with_local_transforms() {
    let mut document = seeded_document(None, None);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: GroupId(20),
                name: "Frame".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: FIRST,
                parent: Some(GroupId(20)),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: SECOND,
                parent: Some(GroupId(20)),
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrences = snapshot.scene_query();
    let instances = occurrences
        .iter()
        .map(|occurrence| ExactGlbInstance {
            package: &package,
            occurrence,
        })
        .collect::<Vec<_>>();

    let export = exact_model_glb_export(&snapshot, &instances).unwrap();
    let gltf = glb_json(&export.glb);
    assert_eq!(gltf["scenes"][0]["nodes"], serde_json::json!([0]));
    assert_eq!(gltf["nodes"][0]["name"], "Frame");
    assert_eq!(gltf["nodes"][0]["children"], serde_json::json!([1, 2]));
    assert_eq!(gltf["nodes"][0]["matrix"][12], 0.1);
    assert_eq!(gltf["nodes"][1]["name"], "Beam A");
    assert_eq!(gltf["nodes"][1]["matrix"][12], 0.0);
    assert_eq!(gltf["nodes"][2]["name"], "Beam B");
    assert_eq!(gltf["nodes"][2]["matrix"][12], 0.02);
    assert!(
        export
            .loss_report
            .contains("hierarchy=canonical global groups")
    );
}

#[test]
fn glb_preserves_occurrence_colors_without_duplicating_geometry_buffers() {
    let document = seeded_document(Some([255, 0, 0]), Some([0, 0, 255]));
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrences = snapshot.scene_query();
    let instances = occurrences
        .iter()
        .map(|occurrence| ExactGlbInstance {
            package: &package,
            occurrence,
        })
        .collect::<Vec<_>>();

    let export = exact_model_glb_export(&snapshot, &instances).unwrap();
    let gltf = glb_json(&export.glb);
    assert_eq!(gltf["materials"].as_array().unwrap().len(), 2);
    assert_eq!(gltf["meshes"].as_array().unwrap().len(), 2);
    assert_eq!(gltf["accessors"].as_array().unwrap().len(), 2);
    assert_eq!(
        gltf["meshes"][0]["primitives"][0]["attributes"],
        gltf["meshes"][1]["primitives"][0]["attributes"]
    );
    assert_eq!(gltf["materials"][0]["name"], "Ketchup #FF0000");
    assert_eq!(gltf["materials"][1]["name"], "Ketchup #0000FF");
}

#[test]
fn exported_glb_imports_as_one_persistent_undoable_mesh_scene() {
    let source_document = seeded_document(None, None);
    let source_snapshot = source_document.current();
    let package = current_package(&source_snapshot);
    let scene = source_snapshot.scene_query();
    let instances = scene
        .iter()
        .map(|occurrence| ExactGlbInstance {
            package: &package,
            occurrence,
        })
        .collect::<Vec<_>>();
    let export = exact_model_glb_export(&source_snapshot, &instances).unwrap();

    let inspection = inspect_glb(&export.glb).unwrap();
    assert_eq!(inspection.mesh_primitive_count(), 1);
    assert_eq!(inspection.node_count(), 2);
    assert_eq!(inspection.instance_count(), 2);
    assert_eq!(inspection.triangle_count(), package.triangles().len());

    let mut imported = DocumentStore::new();
    let before = imported.current().canonical_digest();
    let batch = plan_glb_import(&imported.current(), &export.glb, "round-trip.glb").unwrap();
    let repeated = plan_glb_import(&imported.current(), &export.glb, "round-trip.glb").unwrap();
    assert_eq!(batch.digest(), repeated.digest());
    let proposal = imported.prepare_proposal(batch).unwrap();
    assert_eq!(imported.current().canonical_digest(), before);
    imported.commit_verified_proposal(&proposal).unwrap();

    let committed = imported.current();
    assert_eq!(imported.visible_undo_steps(), 1);
    assert_eq!(committed.definitions().count(), 1);
    assert_eq!(committed.features().count(), 1);
    assert_eq!(committed.groups().count(), 2);
    assert_eq!(committed.occurrences().count(), 2);
    let receipt = committed.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::Glb);
    assert_eq!(receipt.units().source_unit(), ImportLengthUnit::Metre);
    let mesh = committed
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        mesh.authority,
        MeshAuthority::ImportedGlb {
            import_id: receipt.id()
        }
    );
    assert_eq!(mesh.triangles.len(), package.triangles().len());
    assert!(matches!(
        recognize_mesh_body(mesh, 1.0e-6),
        MeshRecognition::Candidate {
            candidate: MeshRecognitionCandidate::Box(_),
            ..
        }
    ));
    assert!(
        mesh.vertices_mm
            .iter()
            .flatten()
            .all(|value| { value.is_finite() && *value >= -1.0e-6 && *value <= 10.0 + 1.0e-6 })
    );
    let translated = committed
        .groups()
        .find(|group| group.name() == "Beam B")
        .unwrap();
    assert!((translated.transform().matrix()[3] - 20.0).abs() < 1.0e-6);
    assert!((translated.transform().matrix()[7] - 30.0).abs() < 1.0e-6);
    assert!((translated.transform().matrix()[11] - 40.0).abs() < 1.0e-6);

    let committed_digest = committed.canonical_digest();
    let encoded = persistence::save(&committed);
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(imported.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        imported.redo().unwrap().canonical_digest(),
        committed_digest
    );
}

#[test]
fn glb_accepts_ignored_blender_attributes_and_isolates_the_selected_scene() {
    let source = rewrite_glb_json(&sample_glb(), |document| {
        let position = document["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
            .as_u64()
            .unwrap() as usize;
        document["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] =
            serde_json::json!(position);
        let mut texcoords = document["accessors"][position].clone();
        texcoords["type"] = serde_json::json!("VEC2");
        texcoords.as_object_mut().unwrap().remove("min");
        texcoords.as_object_mut().unwrap().remove("max");
        let texcoords_index = document["accessors"].as_array().unwrap().len();
        document["accessors"]
            .as_array_mut()
            .unwrap()
            .push(texcoords);
        document["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] =
            serde_json::json!(texcoords_index);

        let mut ignored_mesh = document["meshes"][0].clone();
        ignored_mesh["name"] = serde_json::json!("Unselected unsupported lines");
        ignored_mesh["primitives"][0]["mode"] = serde_json::json!(1);
        let ignored_mesh_index = document["meshes"].as_array().unwrap().len();
        document["meshes"]
            .as_array_mut()
            .unwrap()
            .push(ignored_mesh);
        document["nodes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "name": "Unselected unsupported node",
                "mesh": ignored_mesh_index
            }));
    });

    let inspection = inspect_glb(&source).unwrap();
    let codes = inspection
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"glb_vertex_attributes_ignored"));
    assert!(codes.contains(&"glb_unselected_meshes_ignored"));
    assert!(codes.contains(&"glb_unselected_nodes_ignored"));
}

#[test]
fn glb_rejects_future_minimum_versions_numerically() {
    let source = rewrite_glb_json(&sample_glb(), |document| {
        document["asset"]["minVersion"] = serde_json::json!("10.0");
    });
    assert_eq!(
        inspect_glb(&source),
        Err(GlbImportError::UnsupportedVersion)
    );
}

#[test]
fn glb_rejects_scene_expansion_beyond_the_canonical_output_envelope() {
    let source = rewrite_glb_json(&sample_glb(), |document| {
        let template = document["nodes"][0].clone();
        while document["nodes"].as_array().unwrap().len() < 512 {
            let index = document["nodes"].as_array().unwrap().len();
            let mut node = template.clone();
            node["name"] = serde_json::json!(format!("Instance {index}"));
            document["nodes"].as_array_mut().unwrap().push(node);
        }
        document["scenes"][0]["nodes"] = serde_json::json!((0..512).collect::<Vec<usize>>());
    });
    assert_eq!(inspect_glb(&source), Err(GlbImportError::TooManyOutputs));
}

#[test]
fn committed_blender_compatible_house_glb_is_importable() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let source = std::fs::read(path).unwrap();
    let inspection = inspect_glb(&source).unwrap();
    assert!(inspection.mesh_primitive_count() >= 100);
    assert_eq!(inspection.instance_count(), 140);
    assert!(inspection.triangle_count() > 1_000);

    let mut document = DocumentStore::new();
    let batch = plan_glb_import(&document.current(), &source, "garden-studio-colored.glb").unwrap();
    document.apply_batch(&batch).unwrap();
    assert_eq!(document.current().occurrences().count(), 140);
}

#[test]
fn glb_rejects_exact_results_from_an_older_snapshot() {
    let mut document = seeded_document(None, None);
    let old_snapshot = document.current();
    let package = current_package(&old_snapshot);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RenameDefinition {
                id: DEFINITION,
                name: "Changed beam".into(),
            },
        ]))
        .unwrap();
    let current = document.current();
    let occurrences = current.scene_query();
    let instances = occurrences
        .iter()
        .map(|occurrence| ExactGlbInstance {
            package: &package,
            occurrence,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        exact_model_glb_export(&current, &instances),
        Err(ExactProductError::StaleResult)
    );
}

#[test]
fn mesh_exports_reject_instance_expansion_beyond_bounded_limits() {
    let document = seeded_document(None, None);
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrence = snapshot.scene_query().remove(0);
    let glb_instances = vec![
        ExactGlbInstance {
            package: &package,
            occurrence: &occurrence,
        };
        MAX_GLB_EXPORT_INSTANCES + 1
    ];
    assert_eq!(
        exact_model_glb_export(&snapshot, &glb_instances),
        Err(ExactProductError::ExportResourceLimit)
    );

    let stl_bodies = vec![(&package, Transform::identity()); MAX_STL_EXPORT_INSTANCES + 1];
    assert_eq!(
        exact_model_stl_export(&snapshot, &stl_bodies),
        Err(ExactProductError::ExportResourceLimit)
    );
}
