use ketchup_core::blender_export::{ExactGlbInstance, exact_model_glb_export};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    GroupId, OccurrenceId, Transform,
};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFaceRole, ExactFeatureChainRequest, ExactProductError,
    build_box_render_package, canonical_reference_lineage_digest,
};
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
