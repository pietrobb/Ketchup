use ketchup_core::document::{DocumentStore, FeatureKind, MeshAuthority};
use ketchup_core::import::{
    ImportFormat, ImportLengthUnit, ImportUnitAuthority, SketchupSceneImportError,
    inspect_sketchup_scene, plan_sketchup_scene_import,
};
use ketchup_core::persistence;
use serde_json::json;

fn shared_tetrahedron_scene() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": "ketchup.sketchup-scene.v1",
        "units": "inch",
        "definitions": [{
            "id": "component:cube-guid:solid:1",
            "name": "Shared tetrahedron",
            "vertices": [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0]
            ],
            "triangles": [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]
        }],
        "instances": [
            {
                "definition": "component:cube-guid:solid:1",
                "name": "Left",
                "transform": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                "visible": true
            },
            {
                "definition": "component:cube-guid:solid:1",
                "name": "Right",
                "transform": [1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                "visible": true
            }
        ],
        "metadata": {
            "material_assignments": 3,
            "textures": 1,
            "tags": 2,
            "scenes": 1,
            "unsupported_entities": 4
        }
    }))
    .unwrap()
}

#[test]
fn sketchup_scene_preserves_shared_instances_and_is_one_persistent_undo_step() {
    let source = shared_tetrahedron_scene();
    let review = inspect_sketchup_scene(&source).unwrap();
    assert_eq!(review.definition_count(), 1);
    assert_eq!(review.instance_count(), 2);
    assert_eq!(review.triangle_count(), 4);
    assert_eq!(review.diagnostics().len(), 6);

    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    let batch =
        plan_sketchup_scene_import(&document.current(), &source, "assembly.kscene").unwrap();
    let repeated =
        plan_sketchup_scene_import(&document.current(), &source, "assembly.kscene").unwrap();
    assert_eq!(batch.digest(), repeated.digest());

    let proposal = document.prepare_proposal(batch).unwrap();
    assert_eq!(document.current().canonical_digest(), before);
    document.commit_verified_proposal(&proposal).unwrap();
    let committed = document.current();
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(committed.definitions().count(), 1);
    assert_eq!(committed.occurrences().count(), 2);
    let receipt = committed.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::SketchupScene);
    assert_eq!(receipt.units().source_unit(), ImportLengthUnit::Inch);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );

    let mesh = committed
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        mesh.authority,
        MeshAuthority::ImportedSketchupScene {
            import_id: receipt.id()
        }
    );
    assert_eq!(mesh.vertices_mm[1], [25.4, 0.0, 0.0]);
    let right = committed
        .occurrences()
        .find(|occurrence| occurrence.name() == "Right")
        .unwrap();
    assert_eq!(right.transform().matrix()[3], 50.8);

    let committed_digest = committed.canonical_digest();
    let encoded = persistence::save(&committed);
    assert_eq!(
        u16::from_le_bytes([encoded[10], encoded[11]]),
        persistence::CURRENT_SCHEMA
    );
    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), committed_digest);
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(
        document.redo().unwrap().canonical_digest(),
        committed_digest
    );
}

#[test]
fn schema_30_sketchup_document_remains_losslessly_loadable() {
    let source = shared_tetrahedron_scene();
    let mut document = DocumentStore::new();
    let batch =
        plan_sketchup_scene_import(&document.current(), &source, "schema-30.kscene").unwrap();
    document.apply_batch(&batch).unwrap();
    let snapshot = document.current();
    let mut encoded = persistence::save(&snapshot);
    let manifest_length = u32::from_le_bytes(encoded[12..16].try_into().unwrap()) as usize;
    let payload_offset = 16 + manifest_length;
    let body_contract_bytes = 4 + snapshot
        .definitions()
        .map(|definition| {
            8 + 4
                + definition
                    .bodies()
                    .map(|body| {
                        8 + 4
                            + body.name().len()
                            + 1
                            + 1
                            + usize::from(body.consumed_by().is_some()) * 8
                    })
                    .sum::<usize>()
                + 8
                + 4
                + definition
                    .feature_ids()
                    .iter()
                    .filter_map(|feature_id| {
                        definition
                            .feature_body_ownership(*feature_id)
                            .map(|ownership| {
                                8 + 4
                                    + ownership.input_body_ids().len() * 8
                                    + 1
                                    + usize::from(ownership.output_body_id().is_some()) * 8
                            })
                    })
                    .sum::<usize>()
        })
        .sum::<usize>();
    encoded.truncate(encoded.len() - 16 - body_contract_bytes);
    encoded[10..12].copy_from_slice(&30_u16.to_le_bytes());
    let payload_length = (encoded.len() - payload_offset) as u64;
    encoded[16..24].copy_from_slice(&payload_length.to_le_bytes());
    let checksum = ketchup_core::graph::sha256_bytes(&encoded[payload_offset..]);
    encoded[24..56].copy_from_slice(&checksum);

    let reopened = persistence::load(&encoded).unwrap();
    assert_eq!(reopened.source_schema(), 30);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        snapshot.canonical_digest()
    );
}

#[test]
fn sketchup_scene_parser_refuses_unknown_schema_dangling_instances_and_open_meshes() {
    let source = shared_tetrahedron_scene();
    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    value["schema"] = json!("unknown");
    assert!(matches!(
        inspect_sketchup_scene(&serde_json::to_vec(&value).unwrap()),
        Err(SketchupSceneImportError::UnsupportedSchema)
    ));

    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    value["instances"][0]["definition"] = json!("missing");
    assert!(matches!(
        inspect_sketchup_scene(&serde_json::to_vec(&value).unwrap()),
        Err(SketchupSceneImportError::MissingDefinition)
    ));

    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    value["instances"][0]["transform"] = json!([
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0
    ]);
    assert!(matches!(
        inspect_sketchup_scene(&serde_json::to_vec(&value).unwrap()),
        Err(SketchupSceneImportError::InvalidTransform)
    ));

    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    value["instances"][0]["transform"][0] = json!(1.0e100);
    assert!(matches!(
        inspect_sketchup_scene(&serde_json::to_vec(&value).unwrap()),
        Err(SketchupSceneImportError::InvalidTransform)
    ));

    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    value["definitions"][0]["vertices"] = json!([
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [2.0, 2.0, 2.0]
    ]);
    value["definitions"][0]["triangles"] = json!([[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 4]]);
    let source = serde_json::to_vec(&value).unwrap();
    let document = DocumentStore::new();
    let batch = plan_sketchup_scene_import(&document.current(), &source, "open.kscene").unwrap();
    assert!(document.prepare_proposal(batch).is_err());
    assert_eq!(document.visible_undo_steps(), 0);
}

#[test]
fn sketchup_scene_parser_reports_bounded_quota_failures() {
    let source = shared_tetrahedron_scene();

    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    let instance = value["instances"][0].clone();
    let instances = value["instances"].as_array_mut().unwrap();
    instances.resize(513, instance);
    assert!(matches!(
        inspect_sketchup_scene(&serde_json::to_vec(&value).unwrap()),
        Err(SketchupSceneImportError::TooManyInstances)
    ));

    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    let definition = value["definitions"][0].clone();
    let definitions = value["definitions"].as_array_mut().unwrap();
    definitions.resize(129, definition);
    assert!(matches!(
        inspect_sketchup_scene(&serde_json::to_vec(&value).unwrap()),
        Err(SketchupSceneImportError::TooManyDefinitions)
    ));

    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    value["definitions"][0]["vertices"] =
        serde_json::to_value(vec![[0.0_f64; 3]; 100_001]).unwrap();
    assert!(matches!(
        inspect_sketchup_scene(&serde_json::to_vec(&value).unwrap()),
        Err(SketchupSceneImportError::TooManyVertices)
    ));

    let mut value: serde_json::Value = serde_json::from_slice(&source).unwrap();
    value["definitions"][0]["triangles"] =
        serde_json::to_value(vec![[0_u32, 1, 2]; 200_001]).unwrap();
    assert!(matches!(
        inspect_sketchup_scene(&serde_json::to_vec(&value).unwrap()),
        Err(SketchupSceneImportError::TooManyTriangles)
    ));
}
