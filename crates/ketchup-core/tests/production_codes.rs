use ketchup_core::document::*;
use ketchup_core::persistence;

fn occurrence(id: u64) -> CanonicalCommand {
    CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(id),
        definition_id: DefinitionId(1),
        name: format!("Part {id}"),
        transform: Transform::identity(),
        parent: None,
        tag: None,
        visible: true,
    }
}
fn fixture() -> DocumentStore {
    let mut doc = DocumentStore::new();
    doc.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateDefinition {
            id: DefinitionId(1),
            name: "Plate".into(),
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(10),
            definition_id: DefinitionId(1),
            name: "Profile".into(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0., 0.], [50., 0.], [50., 30.], [0., 30.]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: FeatureId(11),
            definition_id: DefinitionId(1),
            name: "Solid".into(),
            kind: FeatureKind::Extrusion {
                profile: FeatureId(10),
                height: Dimension::from_decimal("10").unwrap(),
            },
        },
        occurrence(20),
        occurrence(21),
    ]))
    .unwrap();
    doc
}
fn set(id: u64, code: Option<&str>) -> CanonicalCommand {
    CanonicalCommand::SetProductionCode {
        instance_path: InstancePath::root(OccurrenceId(id)),
        code: code.map(str::to_owned),
    }
}
#[test]
fn roundtrip_digest_undo_redo_and_copy_isolation() {
    let mut doc = fixture();
    let before = doc.current().canonical_digest();
    doc.apply_batch(&CommandBatch::new(vec![set(20, Some("PLATE_001-A"))]))
        .unwrap();
    let assigned = doc.current().canonical_digest();
    assert_ne!(before, assigned);
    assert_eq!(doc.current().production_codes().count(), 1);
    assert_eq!(
        doc.current()
            .production_code(&InstancePath::root(OccurrenceId(21))),
        None
    );
    let bytes = persistence::save(&doc.current());
    assert_eq!(u16::from_le_bytes(bytes[10..12].try_into().unwrap()), 91);
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), assigned);
    assert_eq!(
        reopened
            .snapshot()
            .production_code(&InstancePath::root(OccurrenceId(20))),
        Some("PLATE_001-A")
    );
    assert_eq!(doc.undo().unwrap().canonical_digest(), before);
    assert_eq!(doc.redo().unwrap().canonical_digest(), assigned);
    doc.apply_batch(&CommandBatch::new(vec![occurrence(22)]))
        .unwrap();
    assert_eq!(
        doc.current()
            .production_code(&InstancePath::root(OccurrenceId(22))),
        None
    );
}
#[test]
fn nested_copies_have_independent_persistent_codes() {
    let mut doc = fixture();
    doc.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateGroup {
            id: GroupId(30),
            name: "Assembly".into(),
            transform: Transform::identity(),
            parent: None,
        },
        CanonicalCommand::SetOccurrenceParent {
            id: OccurrenceId(20),
            parent: Some(GroupId(30)),
        },
        CanonicalCommand::SetOccurrenceParent {
            id: OccurrenceId(21),
            parent: Some(GroupId(30)),
        },
    ]))
    .unwrap();
    let assembly = doc
        .convert_group_to_component(GroupId(30), "Assembly")
        .unwrap();
    doc.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(1000),
            definition_id: assembly.component_definition_id,
            name: "Copy".into(),
            transform: Transform::from_translation(100., 0., 0.).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        },
    ]))
    .unwrap();
    let paths = doc
        .current()
        .scene_query()
        .into_iter()
        .filter(|item| item.definition_id == DefinitionId(1))
        .map(|item| item.instance_path)
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 4);
    assert!(paths.iter().all(|path| !path.is_root()));
    doc.apply_batch(&CommandBatch::new(
        paths
            .iter()
            .enumerate()
            .map(|(i, path)| CanonicalCommand::SetProductionCode {
                instance_path: path.clone(),
                code: Some(format!("{i:012}")),
            })
            .collect(),
    ))
    .unwrap();
    let reopened = persistence::load(&persistence::save(&doc.current())).unwrap();
    for (i, path) in paths.iter().enumerate() {
        assert_eq!(
            reopened.snapshot().production_code(path),
            Some(format!("{i:012}").as_str())
        );
    }
    assert!(
        doc.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence {
                id: OccurrenceId(1000)
            }
        ]))
        .is_err()
    );
}

#[test]
fn persisted_duplicate_and_invalid_codes_are_rejected_even_with_valid_checksum() {
    let mut doc = fixture();
    doc.apply_batch(&CommandBatch::new(vec![
        set(20, Some("FIRST0000001")),
        set(21, Some("OTHER0000002")),
    ]))
    .unwrap();
    let original = persistence::save(&doc.current());
    for replacement in [b"FIRST0000001".as_slice(), b"lower0000002".as_slice()] {
        let mut bytes = original.clone();
        let offset = bytes
            .windows(12)
            .position(|window| window == b"OTHER0000002")
            .unwrap();
        bytes[offset..offset + 12].copy_from_slice(replacement);
        let manifest_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let checksum = ketchup_core::graph::sha256_bytes(&bytes[16 + manifest_length..]);
        bytes[24..56].copy_from_slice(&checksum);
        assert!(persistence::load(&bytes).is_err());
    }
}

#[test]
fn invalid_duplicate_and_document_local_uniqueness() {
    let mut doc = fixture();
    for code in ["", "a", "A B", "A/B", "é", &"X".repeat(65)] {
        assert!(matches!(
            doc.apply_batch(&CommandBatch::new(vec![set(20, Some(code))])),
            Err(CanonicalError::InvalidProductionCode)
        ));
    }
    doc.apply_batch(&CommandBatch::new(vec![set(20, Some(&"X".repeat(64)))]))
        .unwrap();
    let before = doc.current().canonical_digest();
    assert!(matches!(
        doc.apply_batch(&CommandBatch::new(vec![set(21, Some(&"X".repeat(64)))])),
        Err(CanonicalError::DuplicateProductionCode(_))
    ));
    assert_eq!(before, doc.current().canonical_digest());
    assert!(matches!(
        doc.apply_batch(&CommandBatch::new(vec![set(999, Some("MISSING"))])),
        Err(CanonicalError::InvalidProductionCodePath(_))
    ));
    fixture()
        .apply_batch(&CommandBatch::new(vec![set(20, Some(&"X".repeat(64)))]))
        .unwrap();
    doc.apply_batch(&CommandBatch::new(vec![set(999, None), set(999, None)]))
        .unwrap();
    assert_eq!(before, doc.current().canonical_digest());
}
#[test]
fn topology_edits_fail_atomically_but_transforms_and_explicit_clear_work() {
    let mut doc = fixture();
    doc.apply_batch(&CommandBatch::new(vec![set(20, Some("PART"))]))
        .unwrap();
    let before = doc.current().canonical_digest();
    for commands in [
        vec![CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(20),
        }],
        vec![
            CanonicalCommand::DeleteOccurrence {
                id: OccurrenceId(20),
            },
            occurrence(20),
        ],
    ] {
        assert!(matches!(
            doc.apply_batch(&CommandBatch::new(commands)),
            Err(CanonicalError::InvalidProductionCodePath(_))
        ));
        assert_eq!(before, doc.current().canonical_digest());
    }
    doc.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(20),
            transform: Transform::from_translation(1., 2., 3.).unwrap(),
        },
    ]))
    .unwrap();
    assert_eq!(
        doc.current()
            .production_code(&InstancePath::root(OccurrenceId(20))),
        Some("PART")
    );
    doc.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::DeleteOccurrence {
            id: OccurrenceId(20),
        },
        set(20, None),
    ]))
    .unwrap();
    assert_eq!(doc.current().production_codes().count(), 0);
    assert_eq!(doc.undo().unwrap().production_codes().count(), 1);
}
