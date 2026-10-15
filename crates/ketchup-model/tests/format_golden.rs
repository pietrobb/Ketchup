//! Snapshot formats with an Undo history, committed as written (MD-3).
//!
//! The saved serde form of the product model is the file format. The fixture of the
//! current format must keep opening with every revision and the digests it was written
//! with, and saving it again must give the same bytes. When that test fails because the
//! bytes changed, the change altered the format: give it a new number and its
//! conversion in `snapshot_codec::decode`, keep the fixture as the golden of its format
//! (drop the byte comparison for it) and add a golden of the new format. Regenerate
//! only for a new format with `KETCHUP_UPDATE_GOLDEN=1`.
use ketchup_geometry::prismatic::{Aabb, CanonicalJoint, JointId};
use ketchup_model::document::*;
use ketchup_model::persistence::{self, ContainerData, LoadOutcome};

const FIXTURE_102: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/history-format102.bin"
);

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/history-format103.bin"
);

/// Canonical digest of every revision, oldest first, as format 102 wrote them.
const REVISION_DIGESTS_102: &[&str] = &[
    "e51eff18704562b7",
    "1ebf22ce3f9227c2",
    "ba84736e268125d3",
    "c6a97bdcf9e1a6d0",
    "ac95765522bbdcd1",
    "37d5c7ca6e51948c",
    "32004c2fb9d7739d",
];

/// Canonical digest of every revision, oldest first, as format 103 wrote them.
const REVISION_DIGESTS: &[&str] = &[
    "a2bfa71d0ad7e6b3",
    "6cd3c1af4f1f9c26",
    "41e384da97b177b7",
    "ceba4f5a02d6379c",
    "bd01bc074ad79469",
    "0c7e44701364923c",
    "38598be34ea8becd",
    "695c01140f69ddb9",
];

fn occurrence(id: u64, name: &str) -> CanonicalCommand {
    CanonicalCommand::CreateOccurrence {
        id: OccurrenceId(id),
        definition_id: DefinitionId(1),
        name: name.into(),
        transform: Transform::from_translation(id as f64 * 60.0, 0.0, 0.0).unwrap(),
        parent: None,
        tags: Default::default(),
        visible: true,
    }
}

/// Seven revisions that touch the product fields most likely to move: support
/// declarations, contact joints, tags, classification, local structure. Format 103
/// adds a joint of two placed parts.
fn written() -> DocumentStore {
    let mut document = DocumentStore::new();
    let batches = vec![
        vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Doska".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(10),
                definition_id: DefinitionId(1),
                name: "Profil".into(),
                kind: FeatureKind::polygon(&[[0., 0.], [50., 0.], [50., 30.], [0., 30.]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DefinitionId(1),
                name: "Teleso".into(),
                kind: FeatureKind::extrusion(FeatureId(10), Dimension::from_decimal("18").unwrap()),
            },
            occurrence(20, "Bok ľavý"),
        ],
        vec![occurrence(21, "Bok pravý")],
        vec![CanonicalCommand::SetFloorHeight { z_mm: Some(-20.0) }],
        vec![CanonicalCommand::SetGroundedInstances {
            paths: [InstancePath::root(OccurrenceId(20))].into(),
        }],
        vec![CanonicalCommand::SetContactJoints {
            joints: vec![ContactJoint {
                name: "spoj".into(),
                parts: [
                    InstancePath::root(OccurrenceId(20)),
                    InstancePath::root(OccurrenceId(21)),
                ],
                max_gap_mm: 0.5,
            }],
        }],
        vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(1),
                name: "Fáza".into(),
                categories: vec![(ClassificationCategoryId(1), "Korpus".into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(21),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
        ],
        vec![CanonicalCommand::UpsertJoint(
            CanonicalJoint::new(
                JointId(1),
                InstancePath::root(OccurrenceId(20)).into(),
                InstancePath::root(OccurrenceId(21)).into(),
                Aabb::new([50., 0., 0.], [60., 30., 18.]).unwrap(),
            )
            .unwrap(),
        )],
    ];
    for commands in batches {
        document.apply_batch(&CommandBatch::new(commands)).unwrap();
    }
    document
}

fn revision_digests(document: &DocumentStore) -> Vec<String> {
    document
        .revision_history()
        .map(|revision| revision.snapshot().canonical_digest())
        .collect()
}

#[test]
fn a_format_102_file_with_history_opens_with_every_revision_it_was_written_with() {
    let LoadOutcome::Editable {
        document, audit, ..
    } = persistence::load(&std::fs::read(FIXTURE_102).unwrap()).unwrap()
    else {
        panic!("the golden is editable")
    };
    assert_eq!(audit.source_schema, 102);
    assert_eq!(audit.history_discarded, None);
    // A release recorded under this digest must still verify (FMT-1).
    assert_eq!(
        audit.source_canonical_digest,
        *REVISION_DIGESTS_102.last().unwrap()
    );
    assert_eq!(revision_digests(&document), REVISION_DIGESTS_102);
}

#[test]
fn a_format_103_file_with_history_opens_with_every_revision_and_saves_the_same_bytes() {
    if ketchup_test_env::update_golden() {
        let document = written();
        let bytes = persistence::save_document_store(&document, &ContainerData::default()).unwrap();
        std::fs::write(FIXTURE, bytes).unwrap();
        panic!(
            "wrote {FIXTURE}; record REVISION_DIGESTS = {:?}",
            revision_digests(&document)
        );
    }
    let bytes = std::fs::read(FIXTURE).unwrap();
    let LoadOutcome::Editable {
        document, audit, ..
    } = persistence::load(&bytes).unwrap()
    else {
        panic!("the golden is editable")
    };
    assert_eq!(audit.source_schema, persistence::CURRENT_SCHEMA);
    assert_eq!(audit.history_discarded, None);
    assert_eq!(revision_digests(&document), REVISION_DIGESTS);
    assert_eq!(
        persistence::save_document_store(&document, &ContainerData::default()).unwrap(),
        with_current_history_format(bytes),
        "the saved form changed; see the module comment"
    );
}

/// The fixture as it saves now: `history.bin` of a history without programs only
/// numbers format 5 (PS-2, programs as chunks) where the fixture has format 4.
fn with_current_history_format(mut bytes: Vec<u8>) -> Vec<u8> {
    use crate::native_persistence::{container_entry_offsets, rehash_container_entry};
    let (checksum, offset, len) = container_entry_offsets(&bytes, "history.bin");
    assert_eq!(bytes[offset + 10..offset + 12], 4_u16.to_le_bytes());
    bytes[offset + 10..offset + 12].copy_from_slice(&5_u16.to_le_bytes());
    rehash_container_entry(&mut bytes, checksum, offset, len);
    bytes
}
