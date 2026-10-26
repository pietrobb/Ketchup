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
use ketchup_model::underlay::drawn::{AddedObject, DrawnShape};
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

/// Format 105 as Save wrote it: an underlay and annotations.
const SAVED_105: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/format105.bin"
);

/// The current format as Save writes it: the current revision alone.
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/format106.bin"
);

/// Canonical digest of the current revision each fixture was written with.
const DIGEST_102: &str = "32004c2fb9d7739d";
const DIGEST_103: &str = "695c01140f69ddb9";
const DIGEST_104: &str = "36c397ed20d2ec76";
const DIGEST_105: &str = "fdcd5e8c616c1323";
const DIGEST_106: &str = "29680d90129178f6";

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

/// Review 2026-10-21 P3: Save wrote the newest format for every document, so Kečup 0.1.0
/// and 0.2.0, which read format 103, refused every file saved since. A document is saved
/// in the oldest format that holds it: without underlays and annotations as format 103,
/// the very snapshot bytes 0.2.0 itself saved, with an underlay as format 104; with
/// annotations 105; with an edited underlay 106 (the golden).
#[test]
fn a_document_is_saved_in_the_oldest_format_that_holds_it() {
    for (path, schema, digest) in [
        (RELEASE_0_2_0, 103, DIGEST_103),
        (FIXTURE_103, 103, DIGEST_103),
        (SAVED_104, 104, DIGEST_104),
        (SAVED_105, 105, DIGEST_105),
    ] {
        let (document, _) = opened(path, schema, digest);
        let snapshot = persistence::save(&document.current());
        assert_eq!(snapshot[10..12], schema.to_le_bytes(), "{path}");
        let file = std::fs::read(path).unwrap();
        assert!(
            file.windows(snapshot.len())
                .any(|stored| stored == snapshot),
            "{path} does not hold the snapshot saved now"
        );
    }
}

/// The format Save writes for a document with one DXF import of `lines` lines,
/// its source padded with comments to `padding` more bytes.
fn saved_format_of_a_dxf_import(lines: usize, padding: usize) -> u16 {
    use ketchup_model::import::{DxfImportOptions, plan_dxf_import};
    let entities = (0..lines)
        .map(|line| {
            let y = 10 * line;
            format!("0\nLINE\n8\n0\n10\n0\n20\n{y}\n11\n100\n21\n{y}\n")
        })
        .collect::<String>();
    let comment = format!("999\n{}\n", "x".repeat(200));
    let comments = comment.repeat(padding.div_ceil(comment.len()));
    let source = format!(
        "{comments}0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    let mut document = DocumentStore::new();
    let batch = plan_dxf_import(
        &document.current(),
        source.as_bytes(),
        "lines.dxf",
        DxfImportOptions::new(None),
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let snapshot = persistence::save(&document.current());
    u16::from_le_bytes([snapshot[10], snapshot[11]])
}

/// Review 2026-10-24 MD-20: format 103 was chosen for any document without underlays
/// and annotations, so a DXF import of more than 341 profiles (a receipt of more than
/// 1 024 outputs) or of a source over 8 MiB was written as 103, which Kečup 0.1.0 and
/// 0.2.0 refuse as damaged and open their older `.recovery` instead. Such a document
/// is written as 104, which they refuse cleanly as newer; one within their limits
/// stays 103.
#[test]
fn a_document_beyond_what_kecup_0_2_0_reads_is_not_written_as_format_103() {
    assert_eq!(saved_format_of_a_dxf_import(3, 0), 103);
    assert_eq!(saved_format_of_a_dxf_import(341, 0), 103);
    assert_eq!(saved_format_of_a_dxf_import(400, 0), 104);
    assert_eq!(saved_format_of_a_dxf_import(3, 9 * 1024 * 1024), 104);
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

/// Format 105 adds annotations: the format 104 fixture with them saves as the format 105
/// file.
#[test]
fn a_format_104_document_with_annotations_saves_the_format_105_file() {
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
    assert_eq!(saved, std::fs::read(SAVED_105).unwrap());
    let (reopened, _) = opened(SAVED_105, 105, DIGEST_105);
    assert_eq!(reopened.current().annotations(), annotations());
}

/// Format 106 adds the edits of an underlay's drawing: the format 105 file with object 0
/// of its drawing removed, object 1 moved, a line drawn and a copy of object 1 added.
#[test]
fn a_format_106_file_with_an_edited_underlay_saves_the_golden() {
    let (mut document, container) = opened(SAVED_105, 105, DIGEST_105);
    let mut underlay = document.current().underlays().next().unwrap().clone();
    let moved = [[0.0, -1.0, 10.0], [1.0, 0.0, 20.0]];
    underlay.edits.removed_objects.insert(0);
    underlay.edits.moved_objects.insert(1, moved);
    underlay.edits.added = vec![
        AddedObject::Drawn {
            layer: underlay.layers[0].name.clone(),
            shape: DrawnShape::Polyline {
                points_mm: vec![[0.0, 0.0], [250.0, 0.0]],
                closed: false,
            },
        },
        AddedObject::Copy { of: 1, map: moved },
    ];
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::UpsertUnderlay(
            underlay.clone(),
        )]))
        .unwrap();
    assert_eq!(document.current().canonical_digest(), DIGEST_106);
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
    let (reopened, container) = opened(GOLDEN, persistence::CURRENT_SCHEMA, DIGEST_106);
    assert_eq!(reopened.current().underlays().next(), Some(&underlay));
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
/// underlay's `placement` came with format 104, `annotations` with 105, an underlay's
/// `edits` with 106.
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
    "underlay.rs: edits",
    "underlay.rs: placement",
    "underlay/edits.rs: added",
    "underlay/edits.rs: moved_objects",
    "underlay/edits.rs: removed_objects",
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
