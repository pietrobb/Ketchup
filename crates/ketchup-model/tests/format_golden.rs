//! Snapshot formats, committed as written (MD-3).
//!
//! The saved serde form of the product model is the file format. A file of any format,
//! including one an older Kečup wrote with its Undo history, must open at its current
//! revision with the digest it was written with and no Undo steps. Saving the current
//! format must give the bytes of its golden. When that test fails because the bytes
//! changed, the change altered the format: give it a new number and its conversion in
//! `snapshot_codec::decode`, keep the golden as the fixture of its format and add a
//! golden of the new format. Regenerate only for a new format with
//! `KETCHUP_UPDATE_GOLDEN=1`.
use ketchup_model::annotation::{Annotation, AnnotationMark};
use ketchup_model::document::{CanonicalCommand, CommandBatch, DocumentStore, TagId};
use ketchup_model::persistence::{self, ContainerData, LoadNotice, LoadOutcome};
use std::collections::BTreeSet;

const FIXTURE_102: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/history-format102.bin"
);

const FIXTURE_103: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/history-format103.bin"
);

const FIXTURE_104: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/history-format104.bin"
);

/// Format 104 as Save wrote it: the current revision alone.
const SAVED_104: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/format104.bin"
);

/// Format 103 as Save in the released Kečup 0.2.0 wrote it, for a document a program
/// owns (`width` overridden to 800, variant `1000`). Written by 0.2.0's own code.
const RELEASE_0_2_0: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/release-0.2.0.bin"
);

/// The current format as Save writes it: the current revision alone.
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/format105.bin"
);

/// Canonical digest of the current revision each fixture was written with.
const DIGEST_102: &str = "32004c2fb9d7739d";
const DIGEST_103: &str = "695c01140f69ddb9";
const DIGEST_104: &str = "36c397ed20d2ec76";
const DIGEST_105: &str = "fdcd5e8c616c1323";

/// The document of a fixture, checked to open at `digest` with no Undo steps.
fn opened(path: &str, schema: u16, digest: &str) -> (DocumentStore, ContainerData) {
    let LoadOutcome::Editable {
        document,
        audit,
        container_data,
        ..
    } = persistence::load(&std::fs::read(path).unwrap()).unwrap()
    else {
        panic!("the fixture is editable")
    };
    assert_eq!(audit.source_schema, schema);
    // A release recorded under this digest must still verify (FMT-1).
    assert_eq!(audit.source_canonical_digest, digest);
    assert_eq!(document.current().canonical_digest(), digest);
    assert_eq!(document.visible_undo_steps(), 0);
    (document, container_data)
}

#[test]
fn format_102_to_104_files_open_at_their_current_revision() {
    opened(FIXTURE_102, 102, DIGEST_102);
    opened(FIXTURE_103, 103, DIGEST_103);
    for path in [FIXTURE_104, SAVED_104] {
        let (document, _) = opened(path, 104, DIGEST_104);
        assert_eq!(document.current().underlays().count(), 1);
        assert!(document.current().annotations().is_empty());
    }
}

/// A file an earlier Kečup saved with its Undo steps opens without them, as every file
/// does, and says so; a file saved now holds none and says nothing (review 2026-10-21 P3).
#[test]
fn a_file_saved_with_undo_steps_opens_without_them_and_says_so() {
    let audit = |path: &str| {
        let LoadOutcome::Editable {
            document, audit, ..
        } = persistence::load(&std::fs::read(path).unwrap()).unwrap()
        else {
            panic!("the fixture is editable")
        };
        assert_eq!(document.visible_undo_steps(), 0);
        audit
    };
    let opened = audit(FIXTURE_104);
    assert!(
        matches!(
            opened.notices[..],
            [LoadNotice::UndoStepsNotRestored { steps }] if steps > 0
        ),
        "{:?}",
        opened.notices
    );
    assert_eq!(opened.history_discarded(), None);
    assert!(audit(SAVED_104).notices.is_empty());
}

#[test]
fn a_file_saved_by_the_previous_release_opens_with_its_program_and_variants() {
    let (document, _) = opened(RELEASE_0_2_0, 103, DIGEST_103);
    let program = document
        .current_rule_program()
        .expect("the program owns it");
    assert_eq!(program.file_name, "cabinet.star");
    assert_eq!(program.source, "w = param('width', 600)\n");
    assert_eq!(program.overrides.get("width"), Some(&800.0));
    assert_eq!(program.variants.len(), 1);
    assert_eq!(program.variants["1000"].get("width"), Some(&1000.0));
}

/// One annotation of each kind, shown by the tag [`ANNOTATION_TAG`].
fn annotations() -> Vec<Annotation> {
    let tags = BTreeSet::from([TagId(ANNOTATION_TAG)]);
    let annotation = |name: &str, mark| Annotation {
        name: name.to_owned(),
        tags: tags.clone(),
        mark,
    };
    vec![
        annotation(
            "S-01",
            AnnotationMark::Text {
                at_mm: [100.0, 200.0, 5.0],
                text: "S-01".to_owned(),
                height_mm: 3.5,
            },
        ),
        annotation(
            "S-01..S-02",
            AnnotationMark::Dimension {
                from_mm: [0.0, 0.0, 5.0],
                to_mm: [1944.4, 0.0, 5.0],
                offset_mm: -800.0,
                height_mm: 2.5,
            },
        ),
        annotation(
            "anchor",
            AnnotationMark::Leader {
                point_mm: [0.0, 0.0, 5.0],
                at_mm: [-500.0, -500.0, 5.0],
                text: "M24 anchor".to_owned(),
                height_mm: 2.5,
            },
        ),
        annotation(
            "outline",
            AnnotationMark::Line {
                points_mm: vec![[0.0; 3], [1944.4, 0.0, 0.0], [1944.4, 900.0, 0.0]],
                closed: true,
            },
        ),
    ]
}

const ANNOTATION_TAG: u64 = 1_000;

#[test]
fn a_format_105_file_with_annotations_saves_the_golden() {
    let (mut document, container) = opened(FIXTURE_104, 104, DIGEST_104);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateTag {
                id: TagId(ANNOTATION_TAG),
                name: "annotations".to_owned(),
                visible: true,
            },
            CanonicalCommand::SetAnnotations {
                annotations: annotations(),
            },
        ]))
        .unwrap();
    assert_eq!(document.current().canonical_digest(), DIGEST_105);
    let saved = persistence::save_document_store_current_snapshot(&document, &container).unwrap();
    if ketchup_test_env::update_golden() {
        std::fs::write(GOLDEN, saved).unwrap();
        panic!("wrote {GOLDEN}");
    }
    let golden = std::fs::read(GOLDEN).unwrap();
    assert_eq!(
        saved, golden,
        "the saved form changed; see the module comment"
    );
    let (reopened, container) = opened(GOLDEN, persistence::CURRENT_SCHEMA, DIGEST_105);
    assert_eq!(reopened.current().annotations(), annotations());
    assert_eq!(
        persistence::save_document_store_current_snapshot(&reopened, &container).unwrap(),
        golden
    );
}

/// Every field of the model that is left out of the file while it has its default, as
/// `file: field` under `src/`. Format 103 shipped in Kečup 0.1.0, which drops a field it
/// does not know and loses it at its next Save, so a new optional field of the saved
/// document needs a new format number like any other new field (PS-4): `underlays` and an
/// underlay's `placement` came with format 104, `annotations` with 105.
const OPTIONAL_FIELDS: &[&str] = &[
    "document.rs: annotations",
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
    "exact_brep_graph.rs: source_format",
    "exact_brep_graph.rs: source_part_index",
    "exact_brep_graph.rs: tolerance",
    "exact_brep_graph.rs: tolerance",
    "exact_brep_graph.rs: tool_name",
    "exact_brep_graph.rs: tool_name",
    "exact_brep_graph.rs: trim_bits",
    "exact_brep_graph.rs: up_bits",
    "underlay.rs: placement",
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
