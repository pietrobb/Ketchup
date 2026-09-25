use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    GroupId, OccurrenceId, Transform,
};
use ketchup_core::exact_product::{ExactBodyPackage, ExactFaceRole, ExactProductError};
use ketchup_core::testing::box_package;
use ketchup_core::three_mf_export::{
    ExactThreeMfInstance, MAX_THREE_MF_EXPORT_INSTANCES, exact_model_three_mf_export,
};
use std::collections::BTreeMap;

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(2);
const EXTRUSION: FeatureId = FeatureId(3);
const FIRST: OccurrenceId = OccurrenceId(10);
const SECOND: OccurrenceId = OccurrenceId(11);

fn seeded_document() -> DocumentStore {
    seeded_document_with_definition_name("Reusable <beam>")
}

fn seeded_document_with_definition_name(definition_name: &str) -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: definition_name.into(),
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
            CanonicalCommand::CreateGroup {
                id: GroupId(20),
                name: "Frame & parts".into(),
                transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                parent: None,
            },
            CanonicalCommand::CreateOccurrence {
                id: FIRST,
                definition_id: DEFINITION,
                name: "Beam A".into(),
                transform: Transform::identity(),
                parent: Some(GroupId(20)),
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: SECOND,
                definition_id: DEFINITION,
                name: "Beam B".into(),
                transform: Transform::from_translation(20.0, 30.0, 40.0).unwrap(),
                parent: Some(GroupId(20)),
                tag: None,
                visible: true,
            },
            CanonicalCommand::SetOccurrenceColor {
                id: FIRST,
                color: Some([255, 0, 0]),
            },
            CanonicalCommand::SetOccurrenceColor {
                id: SECOND,
                color: Some([255, 0, 0]),
            },
        ]))
        .unwrap();
    document
}

fn current_package(snapshot: &ketchup_core::document::Snapshot) -> ExactBodyPackage {
    box_package(
        snapshot,
        DEFINITION,
        EXTRUSION,
        "shared-result",
        &[
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::LinearSide,
        ],
    )
    .unwrap()
}

fn stored_zip_entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut entries = BTreeMap::new();
    let mut cursor = 0;
    while bytes.get(cursor..cursor + 4) == Some(&0x0403_4b50_u32.to_le_bytes()) {
        let size = u32::from_le_bytes(bytes[cursor + 18..cursor + 22].try_into().unwrap()) as usize;
        let name_len =
            u16::from_le_bytes(bytes[cursor + 26..cursor + 28].try_into().unwrap()) as usize;
        let extra_len =
            u16::from_le_bytes(bytes[cursor + 28..cursor + 30].try_into().unwrap()) as usize;
        let name_start = cursor + 30;
        let data_start = name_start + name_len + extra_len;
        let name = String::from_utf8(bytes[name_start..name_start + name_len].to_vec()).unwrap();
        entries.insert(name, bytes[data_start..data_start + size].to_vec());
        cursor = data_start + size;
    }
    assert_eq!(&bytes[cursor..cursor + 4], &0x0201_4b50_u32.to_le_bytes());
    assert_eq!(
        &bytes[bytes.len() - 22..bytes.len() - 18],
        &0x0605_4b50_u32.to_le_bytes()
    );
    entries
}

#[test]
fn three_mf_is_deterministic_and_preserves_units_hierarchy_transforms_and_color() {
    let document = seeded_document();
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrences = snapshot.scene_query();
    let instances = occurrences
        .iter()
        .map(|occurrence| ExactThreeMfInstance {
            package: &package,
            occurrence,
        })
        .collect::<Vec<_>>();

    let first = exact_model_three_mf_export(&snapshot, &instances).unwrap();
    let second = exact_model_three_mf_export(&snapshot, &instances).unwrap();
    assert_eq!(first, second);
    let entries = stored_zip_entries(&first.three_mf);
    assert_eq!(
        entries.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["3D/3dmodel.model", "[Content_Types].xml", "_rels/.rels"]
    );
    let model = String::from_utf8(entries["3D/3dmodel.model"].clone()).unwrap();
    assert!(model.contains("<model unit=\"millimeter\""));
    assert!(model.contains("name=\"Frame &amp; parts\""));
    assert!(model.contains("name=\"Reusable &lt;beam&gt; body 3\""));
    assert!(model.contains("displaycolor=\"#FF0000FF\""));
    assert!(model.contains("100.00000000000000000 0.00000000000000000 0.00000000000000000"));
    assert!(model.contains("20.00000000000000000 30.00000000000000000 40.00000000000000000"));
    assert_eq!(
        model.matches("<mesh>").count(),
        1,
        "shared colored geometry"
    );
    assert_eq!(model.matches("<item ").count(), 1, "one grouped build root");
    assert_eq!(model.matches("<triangle ").count(), 12);
    assert!(first.loss_report.contains("format=3MF Core 1.3 package"));
    assert!(first.loss_report.contains("unique_mesh_resource_count=1"));
    assert!(first.loss_report.contains("occurrence_body_count=2"));
    assert!(
        first
            .loss_report
            .contains(&format!("source_digest={}", snapshot.canonical_digest()))
    );
}

#[test]
fn three_mf_refuses_empty_hidden_and_stale_inputs_without_mutation() {
    let mut document = seeded_document();
    let snapshot = document.current();
    let package = current_package(&snapshot);
    assert_eq!(
        exact_model_three_mf_export(&snapshot, &[]).unwrap_err(),
        ExactProductError::EmptyModelExport
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: FIRST,
                visible: false,
            },
        ]))
        .unwrap();
    let current = document.current();
    let hidden = current
        .scene_query()
        .into_iter()
        .find(|occurrence| occurrence.occurrence_id == FIRST)
        .unwrap();
    assert_eq!(
        exact_model_three_mf_export(
            &current,
            &[ExactThreeMfInstance {
                package: &package,
                occurrence: &hidden,
            }]
        )
        .unwrap_err(),
        ExactProductError::StaleResult
    );
    assert_eq!(
        document.current().canonical_digest(),
        current.canonical_digest()
    );
}

#[test]
fn three_mf_refuses_excessive_instances_before_encoding() {
    let document = seeded_document();
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrence = snapshot.scene_query().remove(0);
    let instances = vec![
        ExactThreeMfInstance {
            package: &package,
            occurrence: &occurrence,
        };
        MAX_THREE_MF_EXPORT_INSTANCES + 1
    ];

    assert_eq!(
        exact_model_three_mf_export(&snapshot, &instances).unwrap_err(),
        ExactProductError::ExportResourceLimit
    );
}

#[test]
fn three_mf_refuses_package_occurrence_definition_mismatch() {
    let document = seeded_document();
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let mut occurrence = snapshot.scene_query().remove(0);
    occurrence.definition_id = DefinitionId(999);

    assert_eq!(
        exact_model_three_mf_export(
            &snapshot,
            &[ExactThreeMfInstance {
                package: &package,
                occurrence: &occurrence,
            }],
        )
        .unwrap_err(),
        ExactProductError::InvalidMeshExport
    );
}

#[test]
fn three_mf_refuses_xml_1_0_forbidden_name_characters() {
    let document = seeded_document_with_definition_name("invalid\u{1}name");
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrence = snapshot.scene_query().remove(0);

    assert_eq!(
        exact_model_three_mf_export(
            &snapshot,
            &[ExactThreeMfInstance {
                package: &package,
                occurrence: &occurrence,
            }],
        )
        .unwrap_err(),
        ExactProductError::InvalidMeshExport
    );
}
