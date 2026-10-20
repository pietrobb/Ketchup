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

const FIXTURE_103: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/history-format103.bin"
);

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/history-format104.bin"
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
const REVISION_DIGESTS_103: &[&str] = &[
    "a2bfa71d0ad7e6b3",
    "6cd3c1af4f1f9c26",
    "41e384da97b177b7",
    "ceba4f5a02d6379c",
    "bd01bc074ad79469",
    "0c7e44701364923c",
    "38598be34ea8becd",
    "695c01140f69ddb9",
];

/// Canonical digest of every revision, oldest first, as format 104 wrote them: the
/// revisions of format 103 and one that adds an underlay.
const REVISION_DIGESTS: &[&str] = &[
    "48cb24a587a5e1a4",
    "47c446366c7309af",
    "e3e22fe400ae01f6",
    "d06a1acc853fae01",
    "fe70edd3b0e829c4",
    "a1ab9927b9ea3ca5",
    "ba55554281ad83f6",
    "9b4fe31a50c9b5d6",
    "05ed4c9169d7b1ae",
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
    assert_eq!(audit.history_discarded(), None);
    // A release recorded under this digest must still verify (FMT-1).
    assert_eq!(
        audit.source_canonical_digest,
        *REVISION_DIGESTS_102.last().unwrap()
    );
    assert_eq!(revision_digests(&document), REVISION_DIGESTS_102);
}

#[test]
fn a_format_103_file_with_history_opens_with_every_revision_it_was_written_with() {
    let LoadOutcome::Editable {
        document, audit, ..
    } = persistence::load(&std::fs::read(FIXTURE_103).unwrap()).unwrap()
    else {
        panic!("the golden is editable")
    };
    assert_eq!(audit.source_schema, 103);
    assert_eq!(audit.history_discarded(), None);
    // A release recorded under this digest must still verify (FMT-1).
    assert_eq!(
        audit.source_canonical_digest,
        *REVISION_DIGESTS_103.last().unwrap()
    );
    assert_eq!(revision_digests(&document), REVISION_DIGESTS_103);
}

/// [`written`] and a revision that adds an underlay, with its lines in the container.
fn written_104() -> (DocumentStore, ContainerData) {
    use ketchup_model::underlay::{UnderlayLinesBuilder, plan_underlay};
    let mut document = written();
    let mut builder = UnderlayLinesBuilder::default();
    builder.add(
        "stena",
        &[ProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [100.0, 0.0],
        }],
    );
    let (layers, lines) = builder.finish();
    let bytes = lines.encode();
    let (batch, _) = plan_underlay(
        &document.current(),
        "Pôdorys",
        &layers,
        ketchup_model::graph::sha256_bytes(&bytes),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let mut container = ContainerData::default();
    container.insert_import_blob(bytes).unwrap();
    (document, container)
}

#[test]
fn a_format_104_file_with_history_opens_with_every_revision_and_saves_the_same_bytes() {
    if ketchup_test_env::update_golden() {
        let (document, container) = written_104();
        let bytes = persistence::save_document_store(&document, &container).unwrap();
        std::fs::write(FIXTURE, bytes).unwrap();
        panic!(
            "wrote {FIXTURE}; record REVISION_DIGESTS = {:?}",
            revision_digests(&document)
        );
    }
    let bytes = std::fs::read(FIXTURE).unwrap();
    let LoadOutcome::Editable {
        document,
        audit,
        container_data,
        ..
    } = persistence::load(&bytes).unwrap()
    else {
        panic!("the golden is editable")
    };
    assert_eq!(audit.source_schema, persistence::CURRENT_SCHEMA);
    assert_eq!(audit.history_discarded(), None);
    assert_eq!(revision_digests(&document), REVISION_DIGESTS);
    assert_eq!(document.current().underlays().count(), 1);
    assert_eq!(
        persistence::save_document_store(&document, &container_data).unwrap(),
        bytes,
        "the saved form changed; see the module comment"
    );
}

/// Every field of the model that is left out of the file while it has its default, as
/// `file: field` under `src/`. Format 103 shipped in Kečup 0.1.0, which drops a field it
/// does not know and loses it at its next Save, so a new optional field of the saved
/// document needs a new format number like any other new field (PS-4): `underlays`
/// came with format 104.
const OPTIONAL_FIELDS: &[&str] = &[
    "document.rs: contact_joints",
    "document.rs: floor_z_mm",
    "document.rs: grounded_instances",
    "document.rs: hole_threads",
    "document.rs: modeled_threads",
    "document.rs: saved_views",
    "document.rs: shaft_threads",
    "document.rs: tolerance",
    "document.rs: underlays",
    "document/entities.rs: modeled",
    "document/entities.rs: section",
    "document/entities.rs: up",
    "document/revision.rs: variants",
    "exact_brep_graph.rs: chamfer_edge_sides",
    "exact_brep_graph.rs: crest_bits",
    "exact_brep_graph.rs: fillet_radius_stations",
    "exact_brep_graph.rs: guide",
    "exact_brep_graph.rs: guide",
    "exact_brep_graph.rs: name",
    "exact_brep_graph.rs: profile_edges",
    "exact_brep_graph.rs: profile_faces",
    "exact_brep_graph.rs: segment_entity_ids",
    "exact_brep_graph.rs: tolerance",
    "exact_brep_graph.rs: tolerance",
    "exact_brep_graph.rs: tool_name",
    "exact_brep_graph.rs: tool_name",
    "exact_brep_graph.rs: trim_bits",
    "exact_brep_graph.rs: up_bits",
];

#[test]
fn no_optional_field_is_added_without_a_new_format_number() {
    fn scan(directory: &std::path::Path, root: &std::path::Path, found: &mut Vec<String>) {
        let mut entries = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                scan(&path, root, found);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let file = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let source = std::fs::read_to_string(&path).unwrap();
            let lines = source.lines().collect::<Vec<_>>();
            for (index, line) in lines.iter().enumerate() {
                if !line.contains("skip_serializing_if") || line.trim_start().starts_with("//") {
                    continue;
                }
                let field = lines[index + 1..]
                    .iter()
                    .map(|line| line.trim_start())
                    .filter(|line| !line.starts_with('#'))
                    .find_map(field_name)
                    .unwrap_or_else(|| panic!("no field after {file}:{}", index + 1));
                found.push(format!("{file}: {field}"));
            }
        }
    }
    /// The name of the field `line` declares, as in `pub(crate) name: Type`.
    fn field_name(line: &str) -> Option<&str> {
        let line = line
            .strip_prefix("pub(crate) ")
            .or_else(|| line.strip_prefix("pub "))
            .unwrap_or(line);
        let line = line.strip_prefix("r#").unwrap_or(line);
        let (name, _) = line.split_once(':')?;
        let name = name.trim_end();
        let identifier = name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
        (!name.is_empty() && identifier).then_some(name)
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    scan(&root, &root, &mut found);
    found.sort();
    assert_eq!(
        found, OPTIONAL_FIELDS,
        "the optional fields of the model changed. A field of the saved document left out \
         at its default still needs a new format number, because Kečup 0.1.0 would drop it \
         (see the snapshot_codec module comment); list a field outside the document here"
    );
}
