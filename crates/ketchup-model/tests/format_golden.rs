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
use ketchup_model::document::DocumentStore;
use ketchup_model::persistence::{self, ContainerData, LoadOutcome};

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

/// The current format as Save writes it: the current revision alone.
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/persistence/format104.bin"
);

/// Canonical digest of the current revision each fixture was written with.
const DIGEST_102: &str = "32004c2fb9d7739d";
const DIGEST_103: &str = "695c01140f69ddb9";
const DIGEST_104: &str = "36c397ed20d2ec76";

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
fn format_102_and_103_files_with_history_open_at_their_current_revision() {
    opened(FIXTURE_102, 102, DIGEST_102);
    opened(FIXTURE_103, 103, DIGEST_103);
}

#[test]
fn a_format_104_file_with_history_opens_at_its_current_revision_and_saves_the_golden() {
    let (document, container) = opened(FIXTURE_104, persistence::CURRENT_SCHEMA, DIGEST_104);
    assert_eq!(document.current().underlays().count(), 1);
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
    let (reopened, container) = opened(GOLDEN, persistence::CURRENT_SCHEMA, DIGEST_104);
    assert_eq!(
        persistence::save_document_store_current_snapshot(&reopened, &container).unwrap(),
        golden
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
