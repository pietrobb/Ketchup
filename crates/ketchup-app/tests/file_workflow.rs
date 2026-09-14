//! The G6 File workflow replayed through the designed shell, offscreen.
//!
//! Every step is a real click on a real menu item of the real user interface;
//! only the operating system file dialogs are answered from a script. Outcomes
//! are read from document state and from the disk, never from painted text.

mod harness;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use std::time::Duration;

use eframe::egui::{Key, Pos2, accesskit::Role};
use harness::{ScriptedAssistantTransport, Shell};
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_core::assistant_sidecar::{
    AssistantCadBodyFeature, AssistantCadEditOperation, AssistantCadEditProgram,
    AssistantCadFeatureReference, AssistantCadParameterValueType, AssistantCadProgramFeatureOutput,
    AssistantCadProgramFeatureReference, AssistantCadSheetMetalEdge, AssistantCadSheetMetalFlange,
    AssistantCadWeldmentJointPolicy, AssistantCadWeldmentJointPrimary, AssistantCamToolKind,
    AssistantCamWorkOffset, AssistantChatResult, AssistantFeaReviewRequest,
};
use ketchup_core::document::{
    CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
    DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind, NodeId, OccurrenceId,
    ProfileSegment, SpatialPathSegment, Transform,
};
use ketchup_core::graph::sha256_bytes;
use ketchup_core::import::{
    DxfImportOptions, ImportFormat, ImportLengthUnit, ImportUnitAuthority, MAX_DXF_SOURCE_BYTES,
    MAX_STEP_SOURCE_BYTES, MAX_STL_SOURCE_BYTES, inspect_dxf,
};
use ketchup_core::mesh_recognition::{MeshRecognition, recognize_mesh_body};
use ketchup_interaction::Vec3;
use ketchup_scheduler::ExactWorkerSupervisor;

fn compose_two_shared_occurrences(shell: &mut Shell) {
    shell.click_at(shell.viewport_rect().center());
    assert!(
        shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)),
        "composing the model must add a second occurrence"
    );
    shell.settle();
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().definition_count(), 1);
}

fn lossy_legacy_document() -> Vec<u8> {
    let mut bytes = b"KETCHUPDOC".to_vec();
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&7_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&42_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.push(b'x');
    bytes.extend_from_slice(&3.5_f64.to_bits().to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes
}

/// The digest the shell would report for `key`, resolved through its own
/// catalog so a translation change cannot break the assertion.
fn digest_starts_like(shell: &Shell, key: &str) -> bool {
    let template = shell.catalog().text(key);
    let prefix: String = template.chars().take_while(|c| *c != '{').collect();
    !prefix.trim().is_empty() && shell.app().action_digest().starts_with(prefix.trim_end())
}

fn wait_for_visible_label(shell: &mut Shell, label: &str) {
    for _ in 0..300 {
        shell.settle();
        if shell.has_visible_label(label) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for visible label {label:?}");
}

static EXACT_FILE_EXPORT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Eq, PartialEq)]
struct CanonicalState {
    revision: u64,
    digest: String,
    can_undo: bool,
    can_redo: bool,
    undo_steps: usize,
    redo_steps: usize,
    dirty: bool,
    definitions: usize,
    features: usize,
    occurrences: usize,
    mesh_bodies: usize,
    import_receipts: usize,
}

fn canonical_state(shell: &Shell) -> CanonicalState {
    CanonicalState {
        revision: shell.app().document_revision(),
        digest: shell.app().canonical_digest(),
        can_undo: shell.app().can_undo(),
        can_redo: shell.app().can_redo(),
        undo_steps: shell.app().undo_step_count(),
        redo_steps: shell.app().redo_step_count(),
        dirty: shell.app().is_dirty(),
        definitions: shell.app().definition_count(),
        features: shell.app().feature_count(),
        occurrences: shell.app().occurrence_count(),
        mesh_bodies: shell.app().mesh_body_count(),
        import_receipts: shell.app().import_receipt_count(),
    }
}

fn reachable_history_digests(shell: &mut Shell) -> (Vec<String>, Vec<String>) {
    let initial = canonical_state(shell);
    let mut undo = Vec::with_capacity(initial.undo_steps);
    for _ in 0..initial.undo_steps {
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        undo.push(shell.app().canonical_digest());
    }
    for _ in 0..initial.undo_steps {
        shell.click_menu_command("menu-edit", AppCommand::Redo);
    }

    let mut redo = Vec::with_capacity(initial.redo_steps);
    for _ in 0..initial.redo_steps {
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        redo.push(shell.app().canonical_digest());
    }
    for _ in 0..initial.redo_steps {
        shell.click_menu_command("menu-edit", AppCommand::Undo);
    }
    assert_eq!(canonical_state(shell), initial);
    (undo, redo)
}

fn assert_state_and_history_unchanged(
    shell: &mut Shell,
    expected_state: &CanonicalState,
    expected_history: &(Vec<String>, Vec<String>),
) {
    assert_eq!(&canonical_state(shell), expected_state);
    assert_eq!(&reachable_history_digests(shell), expected_history);
}

fn exact_worker_path() -> PathBuf {
    let name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let colocated = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(name);
    if colocated.is_file() {
        colocated
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug")
            .join(name)
    }
}

fn imported_exact_feature_id(shell: &Shell) -> FeatureId {
    shell
        .app()
        .document_snapshot()
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::ImportedExactBody(_)))
        .map(ketchup_core::document::Feature::id)
        .expect("the confirmed STEP import must create one imported exact body feature")
}

fn wait_for_hovered_pick(shell: &mut Shell, position: Pos2) {
    for _ in 0..100 {
        shell.move_pointer(position);
        if shell.app().hovered_selection().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "the current painted exact body must become pickable: producers={:?}, triangles={}",
        shell.app().exact_current_producer_ids(),
        shell.app().instanced_scene_triangle_count()
    );
}

fn wait_for_current_exact_body(shell: &mut Shell) {
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_render_body_count() == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        shell.app().exact_render_body_count(),
        1,
        "the real worker must publish current exact evidence within two seconds"
    );
}

fn arrange_one_visible_and_one_hidden_occurrence_with_redo(shell: &mut Shell) {
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    shell.click_menu_command("menu-edit", AppCommand::Copy);
    shell.click_menu_command("menu-edit", AppCommand::Paste);
    shell.click_menu_command("menu-view", AppCommand::Hide);
    shell.click_menu_command("menu-edit", AppCommand::Paste);
    shell.click_menu_command("menu-edit", AppCommand::Undo);

    assert_eq!(shell.app().active_box_count(), 2);
    assert!(shell.app().can_undo());
    assert!(shell.app().can_redo());
    assert!(shell.app().is_dirty());
}

fn ascii_stl_facet_count(bytes: &[u8]) -> usize {
    assert!(bytes.is_ascii(), "File export must be ASCII STL");
    let text = std::str::from_utf8(bytes).unwrap();
    assert!(
        text.lines()
            .next()
            .is_some_and(|line| line.starts_with("solid ")),
        "ASCII STL must start with a solid declaration"
    );
    assert!(
        text.lines()
            .next_back()
            .is_some_and(|line| line.starts_with("endsolid ")),
        "ASCII STL must end with a matching solid declaration"
    );
    let facets = text
        .lines()
        .filter(|line| line.trim_start().starts_with("facet normal "))
        .count();
    assert_eq!(
        text.lines()
            .filter(|line| line.trim_start().starts_with("vertex "))
            .count(),
        facets * 3
    );
    facets
}

fn valid_binary_tetrahedron() -> Vec<u8> {
    let facets: [([[f32; 3]; 3], [f32; 3]); 4] = [
        (
            [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            [0.0, 0.0, -1.0],
        ),
        (
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            [0.0, -1.0, 0.0],
        ),
        (
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            [-1.0, 0.0, 0.0],
        ),
        (
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [1.0, 1.0, 1.0],
        ),
    ];
    let mut bytes = vec![0_u8; 80];
    bytes[..21].copy_from_slice(b"deterministic binary ");
    bytes.extend_from_slice(&(facets.len() as u32).to_le_bytes());
    for (vertices, normal) in facets {
        for value in normal.into_iter().chain(vertices.into_iter().flatten()) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&0_u16.to_le_bytes());
    }
    bytes
}

fn valid_ascii_tetrahedron() -> &'static [u8] {
    b"solid tetrahedron\n\
 facet normal 0 0 -1\n  outer loop\n   vertex 0 0 0\n   vertex 0 1 0\n   vertex 1 0 0\n  endloop\n endfacet\n\
 facet normal 0 -1 0\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
 facet normal -1 0 0\n  outer loop\n   vertex 0 0 0\n   vertex 0 0 1\n   vertex 0 1 0\n  endloop\n endfacet\n\
 facet normal 1 1 1\n  outer loop\n   vertex 1 0 0\n   vertex 0 1 0\n   vertex 0 0 1\n  endloop\n endfacet\n\
endsolid tetrahedron\n"
}

fn valid_dxf_subset() -> &'static [u8] {
    b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n8\n0\n2\nwrapper\n3\nwrapper\n70\n2\n10\n0\n20\n0\n30\n0\n\
0\nATTDEF\n8\nlabels\n2\nPART\n1\nunknown\n10\n0\n20\n0\n\
0\nINSERT\n8\n0\n2\nstamp\n10\n0\n20\n0\n\
0\nENDBLK\n8\n0\n\
0\nBLOCK\n8\n0\n2\nstamp\n3\nstamp\n70\n1\n10\n0\n20\n0\n30\n0\n\
0\nLWPOLYLINE\n8\n0\n90\n3\n70\n1\n10\n0\n20\n0\n10\n4\n20\n0\n10\n0\n20\n4\n\
0\nSOLID\n8\nfill\n10\n5\n20\n0\n11\n7\n21\n0\n12\n5\n22\n2\n13\n7\n23\n2\n\
0\nENDBLK\n8\n0\n\
0\nBLOCK\n8\n0\n2\n*D1\n70\n1\n10\n0\n20\n0\n\
0\nLINE\n8\n0\n10\n160\n20\n0\n11\n164\n21\n0\n\
0\nTEXT\n8\n0\n10\n162\n20\n1\n1\n4.00\n\
0\nENDBLK\n8\n0\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\noutline\n10\n0\n20\n0\n11\n10\n21\n0\n\
0\nARC\n8\noutline\n10\n10\n20\n10\n40\n10\n50\n270\n51\n0\n\
0\nLWPOLYLINE\n8\ncut\n90\n3\n70\n1\n10\n0\n20\n0\n42\n0\n10\n10\n20\n0\n42\n1\n10\n0\n20\n10\n42\n0\n\
0\nCIRCLE\n8\nbores\n10\n30\n20\n20\n40\n5\n\
0\nPOLYLINE\n8\nlegacy\n66\n1\n70\n1\n0\nVERTEX\n10\n40\n20\n0\n0\nVERTEX\n10\n50\n20\n0\n0\nVERTEX\n10\n50\n20\n10\n0\nVERTEX\n10\n40\n20\n10\n0\nSEQEND\n8\nlegacy\n\
0\nINSERT\n8\nsymbols\n2\nwrapper\n10\n60\n20\n0\n41\n-2\n42\n3\n50\n90\n70\n2\n44\n10\n66\n1\n\
0\nATTRIB\n8\nlabels\n2\nPART\n1\nA-01\n10\n60\n20\n0\n\
0\nSEQEND\n8\nlabels\n\
0\nTRACE\n8\nfill\n10\n80\n20\n0\n11\n84\n21\n0\n12\n80\n22\n3\n13\n80\n23\n3\n\
0\n3DFACE\n8\nfacets\n10\n90\n20\n0\n11\n94\n21\n0\n12\n94\n22\n3\n13\n90\n23\n3\n70\n0\n\
0\nPOLYLINE\n8\npaths-3d\n66\n1\n70\n9\n\
0\nVERTEX\n70\n32\n10\n100\n20\n0\n30\n0\n\
0\nVERTEX\n70\n32\n10\n104\n20\n0\n30\n0\n\
0\nVERTEX\n70\n32\n10\n100\n20\n3\n30\n0\n\
0\nSEQEND\n8\npaths-3d\n\
0\nELLIPSE\n8\narcs\n10\n110\n20\n5\n11\n0\n21\n2\n40\n1\n\
0\nSPLINE\n8\nsplines\n70\n24\n71\n1\n72\n5\n73\n3\n74\n0\n40\n0\n40\n0\n40\n1\n40\n2\n40\n2\n10\n120\n20\n0\n10\n124\n20\n0\n10\n124\n20\n3\n\
0\nHATCH\n8\nhatches\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n2\n92\n1\n93\n4\n72\n1\n10\n130\n20\n0\n11\n134\n21\n0\n72\n3\n10\n134\n20\n1.5\n11\n1.5\n21\n0\n40\n1\n50\n270\n51\n90\n73\n1\n72\n1\n10\n134\n20\n3\n11\n130\n21\n3\n72\n1\n10\n130\n20\n3\n11\n130\n21\n0\n97\n0\n92\n2\n72\n0\n73\n1\n93\n3\n10\n131\n20\n1\n10\n132\n20\n1\n10\n131\n20\n2\n97\n0\n75\n0\n76\n1\n98\n0\n\
0\nHATCH\n8\nhatch-splines\n2\nSOLID\n70\n1\n71\n0\n91\n1\n92\n1\n93\n3\n72\n4\n94\n1\n73\n0\n74\n0\n95\n5\n96\n3\n40\n0\n40\n0\n40\n1\n40\n2\n40\n2\n10\n140\n20\n0\n10\n144\n20\n0\n10\n144\n20\n3\n97\n0\n72\n1\n10\n144\n20\n3\n11\n140\n21\n3\n72\n1\n10\n140\n20\n3\n11\n140\n21\n0\n97\n0\n\
0\nHATCH\n8\nhatch-patterned\n2\nANSI31\n70\n0\n71\n0\n91\n1\n92\n3\n72\n0\n73\n1\n93\n4\n10\n150\n20\n0\n10\n154\n20\n0\n10\n154\n20\n3\n10\n150\n20\n3\n97\n0\n75\n0\n76\n1\n52\n45\n41\n1\n77\n0\n78\n1\n53\n45\n43\n0\n44\n0\n45\n0\n46\n1\n79\n0\n98\n0\n\
0\nMPOLYGON\n8\nmpolygons\n70\n1\n10\n0\n20\n0\n30\n0\n71\n1\n91\n2\n92\n3\n72\n0\n73\n1\n93\n4\n10\n160\n20\n10\n10\n164\n20\n10\n10\n164\n20\n13\n10\n160\n20\n13\n97\n0\n92\n2\n72\n0\n73\n1\n93\n3\n10\n161\n20\n11\n10\n162\n20\n11\n10\n161\n20\n12\n97\n0\n76\n1\n73\n1\n11\n0\n21\n0\n99\n0\n\
0\nDIMENSION\n8\ndimensions\n2\n*D1\n3\nSTANDARD\n10\n0\n20\n0\n12\n5\n22\n2\n70\n32\n\
0\nLEADER\n8\nleaders\n3\nSTANDARD\n71\n1\n72\n0\n73\n0\n74\n0\n75\n0\n76\n3\n10\n170\n20\n0\n30\n0\n10\n174\n20\n0\n30\n0\n10\n174\n20\n3\n30\n0\n\
0\nMESH\n8\nmeshes\n71\n2\n72\n0\n91\n0\n92\n4\n10\n180\n20\n0\n30\n0\n10\n184\n20\n0\n30\n0\n10\n184\n20\n3\n30\n0\n10\n180\n20\n3\n30\n0\n93\n8\n90\n3\n90\n0\n90\n1\n90\n2\n90\n3\n90\n0\n90\n2\n90\n3\n94\n0\n95\n0\n90\n0\n\
0\nPOLYLINE\n8\npolyfaces\n66\n1\n70\n64\n71\n4\n72\n2\n0\nVERTEX\n8\npolyfaces\n70\n192\n10\n190\n20\n0\n30\n0\n0\nVERTEX\n8\npolyfaces\n70\n192\n10\n194\n20\n0\n30\n0\n0\nVERTEX\n8\npolyfaces\n70\n192\n10\n194\n20\n3\n30\n0\n0\nVERTEX\n8\npolyfaces\n70\n192\n10\n190\n20\n3\n30\n0\n0\nVERTEX\n8\npolyfaces\n70\n128\n71\n1\n72\n2\n73\n-3\n0\nVERTEX\n8\npolyfaces\n70\n128\n71\n1\n72\n3\n73\n4\n0\nSEQEND\n8\npolyfaces\n\
0\nPOLYLINE\n8\npolygon-meshes\n66\n1\n70\n16\n71\n2\n72\n2\n0\nVERTEX\n8\npolygon-meshes\n70\n64\n10\n200\n20\n0\n30\n0\n0\nVERTEX\n8\npolygon-meshes\n70\n64\n10\n200\n20\n3\n30\n0\n0\nVERTEX\n8\npolygon-meshes\n70\n64\n10\n204\n20\n0\n30\n0\n0\nVERTEX\n8\npolygon-meshes\n70\n64\n10\n204\n20\n3\n30\n0\n0\nSEQEND\n8\npolygon-meshes\n\
0\nTEXT\n8\nnotes\n10\n0\n20\n0\n1\nunsupported\n\
0\nENDSEC\n0\nEOF\n"
}

fn valid_sketchup_scene_bridge() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "ketchup.sketchup-scene.v1",
        "units": "inch",
        "definitions": [{
            "id": "component:shared:solid:1",
            "name": "Shared solid",
            "vertices": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            "triangles": [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]
        }],
        "instances": [
            {
                "definition": "component:shared:solid:1",
                "name": "First",
                "transform": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                "visible": true
            },
            {
                "definition": "component:shared:solid:1",
                "name": "Second",
                "transform": [1.0, 0.0, 0.0, 2.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                "visible": true
            }
        ],
        "metadata": {
            "material_assignments": 1,
            "textures": 0,
            "tags": 1,
            "scenes": 0,
            "unsupported_entities": 0
        }
    }))
    .unwrap()
}

fn assert_persisted_stl(
    document_path: &Path,
    source: &[u8],
    unit: ImportLengthUnit,
    encoding_diagnostic: &str,
) {
    let loaded = ketchup_core::persistence::load_file(document_path).unwrap();
    let snapshot = loaded.snapshot();
    let receipt = snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(source));
    assert_eq!(receipt.source_byte_len(), source.len() as u64);
    assert_eq!(receipt.units().source_unit(), unit);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::UserDeclared
    );
    assert!(
        receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == encoding_diagnostic)
    );
    let mesh = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(mesh) => Some(mesh),
            _ => None,
        })
        .unwrap();
    let max_coordinate = mesh
        .vertices_mm
        .iter()
        .flatten()
        .copied()
        .fold(0.0_f64, f64::max);
    assert_eq!(max_coordinate, unit.millimetres_per_unit());
}

#[test]
fn assistant_sheet_metal_reaches_exact_worker_and_file_export_through_accesskit() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("sheet-metal-source.ketchup");
    let flat_pattern = directory.path().join("sheet-metal-flat.dxf");
    let bend_table = flat_pattern.with_extension("bends.csv");
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(80),
                name: "Assistant sheet metal".into(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(80),
                definition_id: DefinitionId(80),
                name: "Assistant sheet metal".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    std::fs::write(
        &source,
        ketchup_core::persistence::save_document_store(
            &fixture,
            &ketchup_core::persistence::ContainerData::default(),
        )
        .unwrap(),
    )
    .unwrap();

    let request = "Create a 100 by 50 millimetre sheet-metal bracket with opposite bends.";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the exact sheet-metal feature.".into(),
            model_intent: None,
        },
    )]));
    transport.queue_cad_edit_program(
        request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::AppendFeature {
                definition_id: 80,
                name: "Opposite flanges".into(),
                feature: AssistantCadBodyFeature::SheetMetal {
                    width_mm: 100.0,
                    depth_mm: 50.0,
                    thickness_mm: 2.0,
                    k_factor: 0.4,
                    flanges: vec![
                        AssistantCadSheetMetalFlange {
                            edge: AssistantCadSheetMetalEdge::MinX,
                            length_mm: 20.0,
                            angle_degrees: 90.0,
                            inner_radius_mm: 3.0,
                        },
                        AssistantCadSheetMetalFlange {
                            edge: AssistantCadSheetMetalEdge::MaxX,
                            length_mm: 30.0,
                            angle_degrees: -45.0,
                            inner_radius_mm: 3.0,
                        },
                    ],
                },
            }],
        },
    );
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_export(&flat_pattern)
        .always_confirm_high_risk_as(405);
    let mut shell = Shell::with_dialogs_and_assistant_transport(dialogs, transport);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(Key::Enter);
    let confirm = shell.catalog().text("assistant-confirm");
    wait_for_visible_label(&mut shell, &confirm);
    assert_eq!(shell.app().document_snapshot().features().count(), 0);
    shell.click_row(&confirm);

    let feature_id = shell
        .app()
        .document_snapshot()
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::SheetMetal(_)))
        .map(ketchup_core::document::Feature::id)
        .expect("confirmed Assistant proposal must create one canonical sheet-metal feature");
    wait_for_current_exact_body(&mut shell);
    assert_eq!(shell.app().exact_current_producer_ids(), [feature_id]);
    let before_export = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportSheetMetalManufacturing);
    assert!(flat_pattern.is_file(), "{}", shell.app().action_digest());
    assert!(bend_table.is_file());
    let parsed = inspect_dxf(
        &std::fs::read(&flat_pattern).unwrap(),
        DxfImportOptions::new(None),
    )
    .unwrap();
    assert!(!parsed.profiles().is_empty());
    let bends = std::fs::read_to_string(&bend_table).unwrap();
    assert!(bends.contains("min-x,up,90,3,"));
    assert!(bends.contains("max-x,down,-45,3,"));
    assert_state_and_history_unchanged(&mut shell, &before_export, &before_history);
}

#[test]
fn assistant_cam_setup_reviews_exact_simulation_and_exports_through_accesskit() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cam-source.ketchup");
    let output = directory.path().join("reviewed-facing.nc");
    let definition = DefinitionId(95);
    let profile = FeatureId(950);
    let solid = FeatureId(951);
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "CAM target".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "20x10 profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "20x10x5 target".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::new("5", 5.0).unwrap(),
                },
            },
        ]))
        .unwrap();
    std::fs::write(
        &source,
        ketchup_core::persistence::save_document_store(
            &fixture,
            &ketchup_core::persistence::ContainerData::default(),
        )
        .unwrap(),
    )
    .unwrap();

    let request = "Set up a reviewed 2.5D facing operation.";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the canonical CAM setup.".into(),
            model_intent: None,
        },
    )]));
    transport.queue_cad_edit_program(
        request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::UpsertCamPlan {
                plan_id: 95,
                name: "Reviewed facing".into(),
                target_definition_id: definition.0,
                target_feature_id: solid.0,
                stock_minimum_mm: [0.0, 0.0, 0.0],
                stock_maximum_mm: [20.0, 10.0, 7.0],
                tool_number: 1,
                tool_kind: AssistantCamToolKind::FlatEndMill,
                tool_diameter_mm: 2.0,
                flute_length_mm: 5.0,
                overall_length_mm: 10.0,
                holder_diameter_mm: 6.0,
                holder_length_mm: 10.0,
                spindle_rpm: 10_000,
                feed_mm_per_min: 600.0,
                plunge_mm_per_min: 200.0,
                work_offset: AssistantCamWorkOffset::G54,
                origin_mm: [0.0, 0.0, 0.0],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 1.0, 0.0],
                safe_height_mm: 10.0,
                maximum_stepdown_mm: 2.0,
                stepover_ratio: 0.5,
                radial_allowance_mm: 0.0,
                axial_allowance_mm: 0.0,
            }],
        },
    );
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_export(&output);
    let mut shell = Shell::with_dialogs_and_assistant_transport(dialogs, transport);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let baseline = canonical_state(&shell);

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(Key::Enter);
    let assistant_confirm = shell.catalog().text("assistant-confirm");
    wait_for_visible_label(&mut shell, &assistant_confirm);
    shell.click_row(&assistant_confirm);
    assert!(
        shell
            .app()
            .document_snapshot()
            .cam_plan(ketchup_core::cam::CamPlanId(95))
            .is_some()
    );
    assert_eq!(shell.app().undo_step_count(), baseline.undo_steps + 1);
    let setup_state = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ReviewCamExport);
    let cancel = shell.catalog().text("dialog-cancel");
    wait_for_visible_label(&mut shell, &cancel);
    shell.click_button_label(&cancel);
    assert!(!output.exists());
    assert_eq!(canonical_state(&shell), setup_state);

    shell.click_menu_command("menu-file", AppCommand::ReviewCamExport);
    let preview = shell.catalog().text("cam-review-preview");
    shell.click_button_label(&preview);
    assert!(shell.has_visible_label("mm: G54"));
    assert!(shell.app().action_digest().contains("safe"));
    assert_eq!(canonical_state(&shell), setup_state);
    let confirm = shell.catalog().text("cam-review-confirm-export");
    shell.click_button_label(&confirm);
    assert!(output.is_file(), "{}", shell.app().action_digest());
    let artifact = std::fs::read_to_string(&output).unwrap();
    assert!(artifact.contains("\nG21\nG90\nG17\nG54\nT1 M6\nS10000 M3\n"));
    assert_eq!(canonical_state(&shell), setup_state);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), baseline.digest);
    assert!(
        shell
            .app()
            .document_snapshot()
            .cam_plan(ketchup_core::cam::CamPlanId(95))
            .is_none()
    );
}

#[test]
fn static_fea_review_runs_through_offscreen_accesskit_without_mutation() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("fea-source.ketchup");
    let definition = DefinitionId(71);
    let profile = FeatureId(72);
    let solid = FeatureId(73);
    let occurrence = OccurrenceId(74);
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "FEA cylinder".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "Circular section".into(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![
                        ProfileSegment::CircularArc {
                            start_mm: [10.0, 0.0],
                            end_mm: [-10.0, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [-10.0, 0.0],
                            end_mm: [10.0, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "Cylinder".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::new("20", 20.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: "Cylinder instance".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    std::fs::write(
        &source,
        ketchup_core::persistence::save_document_store(
            &fixture,
            &ketchup_core::persistence::ContainerData::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let request = "Set up a converged static FEA review for the selected cylinder.";
    let transport = Arc::new(ScriptedAssistantTransport::new([(
        request.to_owned(),
        AssistantChatResult {
            message: "Review the exact FEA setup, then explicitly confirm the solve.".into(),
            model_intent: None,
        },
    )]));
    transport.queue_fea_review(
        request,
        AssistantFeaReviewRequest {
            definition_id: definition.0,
            feature_id: solid.0,
            occurrence_id: occurrence.0,
            case_id: "assistant-cylinder-static".into(),
            youngs_modulus_mpa: 200_000.0,
            poisson_ratio: 0.3,
            yield_strength_mpa: 250.0,
            constrained_face_ordinals: vec![0],
            loaded_face_ordinal: 1,
            traction_local_n_per_mm2: [0.0, 0.0, -1.0],
            coarse_deflection_mm: 0.3,
            fine_deflection_mm: 0.12,
        },
    );
    let dialogs = ScriptedFileDialogs::new().queue_open(&source);
    let mut shell = Shell::with_dialogs_and_assistant_transport(dialogs, transport);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    assert!(shell.app_mut().headless_select_occurrence(occurrence));
    shell.settle();
    let baseline = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ReviewStaticFea);
    let cancel = shell.catalog().text("dialog-cancel");
    wait_for_visible_label(&mut shell, &cancel);
    shell.click_button_label(&cancel);
    assert_eq!(canonical_state(&shell), baseline);

    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(request);
    shell.press_key(Key::Enter);
    let run = shell.catalog().text("fea-review-run-confirmed");
    wait_for_visible_label(&mut shell, &run);
    let assistant_baseline = canonical_state(&shell);
    assert_eq!(assistant_baseline.revision, baseline.revision);
    assert_eq!(assistant_baseline.digest, baseline.digest);
    assert_eq!(assistant_baseline.undo_steps, baseline.undo_steps);
    assert_eq!(assistant_baseline.redo_steps, baseline.redo_steps);
    shell.click_button_label(&run);
    assert!(
        shell.app().action_digest().contains("without changing")
            || shell.app().action_digest().contains("bez zmeny"),
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(canonical_state(&shell), assistant_baseline);
}

#[test]
fn local_pdm_root_child_and_verified_open_run_through_offscreen_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("pdm-source.ketchup");
    let repository = directory.path().join(".ketchup-pdm");
    let definition = DefinitionId(301);
    let profile = FeatureId(302);
    let solid = FeatureId(303);
    let occurrence = OccurrenceId(304);
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "Released bracket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: profile,
                definition_id: definition,
                name: "80x40 profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: solid,
                definition_id: definition,
                name: "80x40x12 bracket".into(),
                kind: FeatureKind::Extrusion {
                    profile,
                    height: Dimension::new("12", 12.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: "Bracket instance".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    std::fs::write(
        &source,
        ketchup_core::persistence::save_document_store(
            &fixture,
            &ketchup_core::persistence::ContainerData::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let dialogs = ScriptedFileDialogs::new().queue_open(&source);
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let root_state = canonical_state(&shell);
    let root_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ReviewLocalPdm);
    let cancel = shell.catalog().text("dialog-cancel");
    wait_for_visible_label(&mut shell, &cancel);
    shell.click_button_label(&cancel);
    assert!(!repository.exists());
    assert_state_and_history_unchanged(&mut shell, &root_state, &root_history);

    shell.click_menu_command("menu-file", AppCommand::ReviewLocalPdm);
    let confirm = shell.catalog().text("pdm-confirm-create");
    wait_for_visible_label(&mut shell, &confirm);
    shell.click_button_label(&confirm);
    assert!(digest_starts_like(&shell, "pdm-release-created"));
    assert_eq!(
        std::fs::read_dir(repository.join("releases"))
            .unwrap()
            .count(),
        1
    );
    assert_state_and_history_unchanged(&mut shell, &root_state, &root_history);
    shell.click_button_label(&cancel);

    assert!(shell.app_mut().headless_select_occurrence(occurrence));
    assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));
    shell.settle();
    let child_state = canonical_state(&shell);
    let child_history = reachable_history_digests(&mut shell);
    assert_ne!(child_state.digest, root_state.digest);

    shell.click_menu_command("menu-file", AppCommand::ReviewLocalPdm);
    let refresh = shell.catalog().text("pdm-catalog-refresh");
    wait_for_visible_label(&mut shell, &refresh);
    shell.click_button_label(&refresh);
    assert!(digest_starts_like(&shell, "pdm-catalog-ready"));
    shell.click_button_label(&confirm);
    assert!(digest_starts_like(&shell, "pdm-release-created"));
    assert_eq!(
        std::fs::read_dir(repository.join("releases"))
            .unwrap()
            .count(),
        2
    );
    assert_state_and_history_unchanged(&mut shell, &child_state, &child_history);

    let verify = shell.catalog().text("pdm-open-release");
    shell.click_button_label(&verify);
    assert!(digest_starts_like(&shell, "pdm-open-verified"));
    assert_state_and_history_unchanged(&mut shell, &child_state, &child_history);
}

#[test]
fn assistant_weldment_recomputes_and_exports_cut_list_through_accesskit() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("weldment-source.ketchup");
    let cut_list = directory.path().join("weldment-cut-list.csv");
    let drawing = cut_list.with_extension("svg");
    let mut fixture = DocumentStore::new();
    fixture
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(80),
                name: "Assistant weldment".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(80),
                definition_id: DefinitionId(80),
                name: "Shared square profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(81),
                definition_id: DefinitionId(80),
                name: "Horizontal path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [-100.0, 0.0, 0.0],
                        end_mm: [0.0, 0.0, 0.0],
                    }],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(82),
                definition_id: DefinitionId(80),
                name: "Vertical path".into(),
                kind: FeatureKind::SpatialPath {
                    segments: vec![SpatialPathSegment::Line {
                        start_mm: [0.0, 0.0, 0.0],
                        end_mm: [0.0, 100.0, 0.0],
                    }],
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(80),
                definition_id: DefinitionId(80),
                name: "Assistant weldment".into(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(200),
                name: ketchup_core::fabrication::FABRICATION_ROLE_DIMENSION_V1.into(),
                categories: vec![(
                    ClassificationCategoryId(201),
                    ketchup_core::fabrication::MANUFACTURED_ITEM_ROLE_V1.into(),
                )],
            },
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(210),
                name: ketchup_core::fabrication::MATERIAL_DIMENSION_V1.into(),
                categories: vec![(
                    ClassificationCategoryId(211),
                    "ketchup.material.steel.s355.v1".into(),
                )],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(80),
                dimension_id: ClassificationDimensionId(200),
                category_id: Some(ClassificationCategoryId(201)),
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(80),
                dimension_id: ClassificationDimensionId(210),
                category_id: Some(ClassificationCategoryId(211)),
            },
        ]))
        .unwrap();
    std::fs::write(
        &source,
        ketchup_core::persistence::save_document_store(
            &fixture,
            &ketchup_core::persistence::ContainerData::default(),
        )
        .unwrap(),
    )
    .unwrap();

    let create_request = "Create two square-section weldment members and miter their corner.";
    let resize_request = "Change the shared weldment profile width to 6 millimetres.";
    let transport = Arc::new(ScriptedAssistantTransport::new([
        (
            create_request.to_owned(),
            AssistantChatResult {
                message: "Review the exact weldment members and miter joint.".into(),
                model_intent: None,
            },
        ),
        (
            resize_request.to_owned(),
            AssistantChatResult {
                message: "Review the associative profile change.".into(),
                model_intent: None,
            },
        ),
    ]));
    let body_output = |operation_index| {
        AssistantCadFeatureReference::ProgramOutput(AssistantCadProgramFeatureReference {
            operation_index,
            output: AssistantCadProgramFeatureOutput::BodyFeature,
        })
    };
    transport.queue_cad_edit_program(
        create_request,
        AssistantCadEditProgram {
            operations: vec![
                AssistantCadEditOperation::AppendFeature {
                    definition_id: 80,
                    name: "Horizontal member".into(),
                    feature: AssistantCadBodyFeature::WeldmentMember {
                        profile_feature_id: 80,
                        path_feature_id: 81,
                        orientation_degrees: 0.0,
                    },
                },
                AssistantCadEditOperation::AppendFeature {
                    definition_id: 80,
                    name: "Vertical member".into(),
                    feature: AssistantCadBodyFeature::WeldmentMember {
                        profile_feature_id: 80,
                        path_feature_id: 82,
                        orientation_degrees: 0.0,
                    },
                },
                AssistantCadEditOperation::AppendFeature {
                    definition_id: 80,
                    name: "Miter corner".into(),
                    feature: AssistantCadBodyFeature::WeldmentJoint {
                        first_member_id: body_output(0),
                        second_member_id: body_output(1),
                        policy: AssistantCadWeldmentJointPolicy::Miter,
                        primary: AssistantCadWeldmentJointPrimary::First,
                    },
                },
            ],
        },
    );
    transport.queue_cad_edit_program(
        resize_request,
        AssistantCadEditProgram {
            operations: vec![AssistantCadEditOperation::SetFeatureParameter {
                feature_id: 80,
                parameter_path: "bounds.width".into(),
                value_type: AssistantCadParameterValueType::Length,
                value: 6.0,
            }],
        },
    );
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&source)
        .queue_export(&cut_list)
        .queue_export(&cut_list)
        .queue_refused_high_risk()
        .always_confirm_high_risk_as(406);
    let mut shell = Shell::with_dialogs_and_assistant_transport(dialogs, transport);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    let input = shell.catalog().text("assistant-input-hint");
    let confirm = shell.catalog().text("assistant-confirm");
    shell.focus_text_input(&input);
    shell.type_text(create_request);
    shell.press_key(Key::Enter);
    wait_for_visible_label(&mut shell, &confirm);
    shell.click_row(&confirm);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::WeldmentMember(_)))
            .count(),
        2,
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::WeldmentJoint(_)))
            .count(),
        1,
        "{}",
        shell.app().action_digest()
    );
    wait_for_current_exact_body(&mut shell);
    let joint_id = shell
        .app()
        .document_snapshot()
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::WeldmentJoint(_)))
        .map(ketchup_core::document::Feature::id)
        .unwrap();
    assert_eq!(shell.app().exact_current_producer_ids(), [joint_id]);
    let before = shell
        .app_mut()
        .current_general_fabrication_projection()
        .unwrap()
        .weldment
        .unwrap();
    let stable_rows = before
        .rows
        .iter()
        .map(|row| row.stable_row_id.clone())
        .collect::<Vec<_>>();
    let before_digest = before.cut_list_envelope.source_digest;

    shell.focus_text_input(&input);
    shell.type_text(resize_request);
    shell.press_key(Key::Enter);
    wait_for_visible_label(&mut shell, &confirm);
    shell.click_row(&confirm);
    wait_for_current_exact_body(&mut shell);
    assert_eq!(shell.app().exact_current_producer_ids(), [joint_id]);
    let after = shell
        .app_mut()
        .current_general_fabrication_projection()
        .unwrap()
        .weldment
        .unwrap();
    assert_eq!(
        after
            .rows
            .iter()
            .map(|row| row.stable_row_id.clone())
            .collect::<Vec<_>>(),
        stable_rows
    );
    assert_ne!(after.cut_list_envelope.source_digest, before_digest);
    let before_export = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportWeldmentCutList);
    assert!(!cut_list.exists());
    assert!(!drawing.exists());
    assert!(shell.app().last_side_effect_receipt().is_none());
    shell.click_menu_command("menu-file", AppCommand::ExportWeldmentCutList);
    assert!(cut_list.is_file(), "{}", shell.app().action_digest());
    assert!(drawing.is_file());
    let csv = std::fs::read_to_string(&cut_list).unwrap();
    let svg = std::fs::read_to_string(&drawing).unwrap();
    assert!(csv.contains(ketchup_core::fabrication::WELDMENT_CUT_LIST_EXPORT_V1));
    assert!(csv.contains("position=1"));
    assert!(csv.contains("position=2"));
    assert!(csv.contains("material=ketchup.material.steel.s355.v1"));
    assert!(svg.contains(ketchup_core::fabrication::WELDMENT_DRAWING_SVG_V1));
    assert_eq!(
        shell.app().last_side_effect_receipt().unwrap().operation(),
        "release-weldment-cut-list-and-drawing"
    );
    assert_state_and_history_unchanged(&mut shell, &before_export, &before_history);
}

#[test]
fn save_as_then_new_then_open_restores_the_same_canonical_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("composed.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());

    compose_two_shared_occurrences(&mut shell);
    let composed_digest = shell.app().canonical_digest();
    let composed_revision = shell.app().document_revision();
    assert!(shell.app().is_dirty(), "an edited document must be dirty");

    shell.click_menu_command("menu-file", AppCommand::SaveAs);

    assert!(path.is_file(), "Save As must write the document to disk");
    let inspection = ketchup_app::inspect_native_document(&path).unwrap();
    assert_eq!(
        inspection.schema_version,
        ketchup_core::persistence::CURRENT_SCHEMA
    );
    assert_eq!(inspection.definitions, 1);
    assert_eq!(inspection.root_occurrences, 2);
    assert_eq!(inspection.profiles, 1);
    assert_eq!(inspection.extrusions, 1);
    assert_eq!(inspection.profile_extrusion_definitions, 1);
    assert_eq!(inspection.visible_profile_extrusion_root_occurrences, 2);
    assert_eq!(inspection.canonical_digest, composed_digest);
    assert_eq!(
        inspection.container_sha256,
        ketchup_core::graph::sha256_hex(&std::fs::read(&path).unwrap())
    );
    assert!(!shell.app().is_dirty(), "a saved document must be clean");
    assert_eq!(shell.app().document_path(), Some(path.as_path()));

    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "New must replace the composed model with an empty document"
    );
    assert_eq!(shell.app().document_path(), None);

    shell.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(
        shell.app().canonical_digest(),
        composed_digest,
        "a reopened document must keep its IDs, hierarchy, transforms, \
         parameters, and sharing"
    );
    assert_eq!(shell.app().document_revision(), composed_revision);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().definition_count(), 1);
    assert!(
        !shell.app().is_dirty(),
        "a freshly opened document is clean"
    );
    assert_eq!(
        script.suggested_names(),
        vec!["Untitled.ketchup".to_owned()],
        "Save As must propose the current document name"
    );
}

#[test]
fn save_open_preserves_the_complete_undo_redo_history_through_accesskit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .queue_open(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    let expected = canonical_state(&shell);
    let expected_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    for cycle in 0..3 {
        shell.click_menu_command("menu-file", AppCommand::New);
        shell.click_menu_command("menu-file", AppCommand::Open);

        let reopened = canonical_state(&shell);
        assert_eq!(reopened.revision, expected.revision, "cycle {cycle}");
        assert_eq!(reopened.digest, expected.digest, "cycle {cycle}");
        assert_eq!(reopened.undo_steps, expected.undo_steps, "cycle {cycle}");
        assert_eq!(reopened.redo_steps, expected.redo_steps, "cycle {cycle}");
        assert!(!reopened.dirty, "cycle {cycle}");
        assert_eq!(
            reachable_history_digests(&mut shell),
            expected_history,
            "cycle {cycle}"
        );

        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), expected_history.1[0]);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), expected.digest);
        if cycle < 2 {
            shell.click_menu_command("menu-file", AppCommand::Save);
        }
    }
}

#[test]
fn corrupt_persisted_history_fails_open_without_replacing_the_active_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("corrupt-history.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    let active = canonical_state(&shell);
    let mut corrupt = std::fs::read(&path).unwrap();
    *corrupt.last_mut().unwrap() ^= 0xff;
    std::fs::write(&path, corrupt).unwrap();

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(canonical_state(&shell), active);
    assert!(digest_starts_like(&shell, "error-open-document"));
}

#[test]
fn recovered_open_warns_with_the_actual_source_and_requires_save_as() {
    let directory = tempfile::tempdir().unwrap();
    let requested = directory.path().join("damaged.ketchup");
    let recovery = requested.with_extension("ketchup.recovery");
    let destination = directory.path().join("recovered-copy.ketchup");

    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&requested));
    compose_two_shared_occurrences(&mut author);
    let expected = author.app().canonical_digest();
    author.click_menu_command("menu-file", AppCommand::SaveAs);
    std::fs::copy(&requested, &recovery).unwrap();
    std::fs::write(&requested, b"corrupt primary").unwrap();

    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&requested)
        .queue_save(&destination)
        .always_discard();
    let mut recovered = Shell::with_dialogs(dialogs);
    recovered.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(recovered.app().canonical_digest(), expected);
    assert_eq!(recovered.app().document_path(), None);
    assert_eq!(
        recovered.app().recovery_requested_path(),
        Some(requested.as_path())
    );
    assert_eq!(
        recovered.app().recovery_source_path(),
        Some(recovery.as_path())
    );
    assert!(recovered.app().is_dirty());
    assert!(digest_starts_like(&recovered, "digest-opened-recovery"));
    assert!(
        recovered
            .app()
            .action_digest()
            .contains(&recovery.display().to_string())
    );
    assert!(
        recovered
            .app()
            .action_digest()
            .contains(&requested.display().to_string())
    );

    recovered.click_menu_command("menu-file", AppCommand::Save);
    assert_eq!(recovered.app().document_path(), Some(destination.as_path()));
    assert_eq!(recovered.app().recovery_requested_path(), None);
    assert_eq!(recovered.app().recovery_source_path(), None);
    assert!(!recovered.app().is_dirty());
    assert_eq!(std::fs::read(&requested).unwrap(), b"corrupt primary");
    assert_eq!(
        ketchup_core::persistence::load_file(&destination)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        expected
    );
}

#[test]
fn native_document_inspection_counts_only_visible_modeled_root_occurrences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("hidden-modeled-root.ketchup");
    let script = ScriptedFileDialogs::new().queue_save(&path);
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    assert!(shell.app_mut().set_selection_visibility(false));
    shell.click_menu_command("menu-file", AppCommand::SaveAs);

    let inspection = ketchup_app::inspect_native_document(&path).unwrap();
    assert_eq!(inspection.root_occurrences, 2);
    assert_eq!(inspection.profile_extrusion_definitions, 1);
    assert_eq!(inspection.visible_profile_extrusion_root_occurrences, 1);
}

#[test]
fn native_document_inspection_rejects_an_oversized_sparse_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("oversized-inspection.ketchup");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(ketchup_core::persistence::MAX_NATIVE_DOCUMENT_BYTES as u64 + 1)
        .unwrap();

    let error = ketchup_app::inspect_native_document(&path).unwrap_err();
    assert!(
        error.contains("document exceeds a resource limit"),
        "{error}"
    );
}

#[test]
fn native_document_inspection_hashes_the_recovered_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recovered-inspection.ketchup");
    let recovery = path.with_extension("ketchup.recovery");
    let script = ScriptedFileDialogs::new().queue_save(&path);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    std::fs::copy(&path, &recovery).unwrap();
    let recovery_bytes = std::fs::read(&recovery).unwrap();
    std::fs::write(&path, b"corrupt primary").unwrap();

    let inspection = ketchup_app::inspect_native_document(&path).unwrap();
    assert_eq!(
        inspection.container_sha256,
        ketchup_core::graph::sha256_hex(&recovery_bytes)
    );
}

#[test]
fn confirmed_legacy_migration_writes_and_activates_only_a_new_copy() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy.ketchup");
    let destination = directory.path().join("migrated.ketchup");
    let occupied_destination = directory.path().join("occupied.ketchup");
    let replay_destination = directory.path().join("replayed.ketchup");
    let source_bytes = lossy_legacy_document();
    std::fs::write(&source, &source_bytes).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&source)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    let active_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(
        shell.app().has_review_candidate(),
        "{}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&source)
    );
    assert!(shell.app().has_review_candidate());
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(directory.path())
    );
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);

    std::fs::write(&source, b"tampered after review").unwrap();
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!destination.exists());
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);

    std::fs::File::create(&source)
        .unwrap()
        .set_len(ketchup_core::persistence::MAX_NATIVE_DOCUMENT_BYTES as u64 + 1)
        .unwrap();
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!destination.exists());
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);
    std::fs::write(&source, &source_bytes).unwrap();

    let occupied_bytes = b"preserve existing migration destination";
    std::fs::write(&occupied_destination, occupied_bytes).unwrap();
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&occupied_destination)
    );
    assert_eq!(
        std::fs::read(&occupied_destination).unwrap(),
        occupied_bytes
    );
    assert!(shell.app().has_review_candidate());
    assert_eq!(shell.app().canonical_digest(), active_digest);

    assert!(
        shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!shell.app().has_review_candidate());
    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&replay_destination)
    );
    assert!(!replay_destination.exists());
    assert_eq!(shell.app().document_path(), Some(destination.as_path()));
    assert_eq!(shell.app().document_revision(), 8);
    assert!(!shell.app().is_dirty());
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert_eq!(
        ketchup_core::persistence::load_file(&source)
            .unwrap()
            .disposition(),
        ketchup_core::persistence::LoadDisposition::ReviewOnly
    );
    assert_eq!(
        ketchup_core::persistence::load_file(&destination)
            .unwrap()
            .disposition(),
        ketchup_core::persistence::LoadDisposition::EditableLossless
    );
}

#[test]
fn recovered_migration_review_revalidates_the_recovery_source() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("recovered-legacy.ketchup");
    let recovery = source.with_extension("ketchup.recovery");
    let destination = directory.path().join("recovered-migrated.ketchup");
    std::fs::write(&source, b"corrupt primary").unwrap();
    std::fs::write(&recovery, lossy_legacy_document()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&source)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(shell.app().has_review_candidate());
    std::fs::File::create(&recovery)
        .unwrap()
        .set_len(ketchup_core::persistence::MAX_NATIVE_DOCUMENT_BYTES as u64 + 1)
        .unwrap();

    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!destination.exists());
    assert!(shell.app().has_review_candidate());
}

#[test]
fn legacy_migration_review_rejects_a_stale_active_document_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("legacy-stale.ketchup");
    let destination = directory.path().join("stale-migration.ketchup");
    std::fs::write(&source, lossy_legacy_document()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&source)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    shell.click_menu_command("menu-file", AppCommand::Open);
    assert!(shell.app().has_review_candidate());
    assert!(shell.app_mut().create_box());
    let stale_state = canonical_state(&shell);

    assert!(
        !shell
            .app_mut()
            .confirm_review_candidate_migration_to(&destination)
    );
    assert!(!destination.exists());
    assert!(shell.app().has_review_candidate());
    assert_eq!(canonical_state(&shell), stale_state);
}

#[test]
fn optional_unknown_extension_survives_app_open_edit_and_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("extended.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard()
        .always_confirm_high_risk_as(7);
    let mut shell = Shell::with_dialogs(script);
    shell.click_menu_command("menu-file", AppCommand::Save);

    let snapshot = ketchup_core::persistence::load_file(&path)
        .unwrap()
        .snapshot();
    let mut sidecars = ketchup_core::persistence::ContainerData::default();
    sidecars
        .insert_extension(
            ketchup_core::persistence::ExtensionEntry::new(
                "org.example.optional",
                "opaque.bin",
                false,
                vec![7, 8, 9],
            )
            .unwrap(),
        )
        .unwrap();
    std::fs::write(
        &path,
        ketchup_core::persistence::save_container(&snapshot, &sidecars).unwrap(),
    )
    .unwrap();

    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    shell.click_menu_command("menu-file", AppCommand::Save);

    let reopened = ketchup_core::persistence::load_file(&path).unwrap();
    let extension = reopened.container_data().extensions().next().unwrap();
    assert_eq!(extension.namespace(), "org.example.optional");
    assert_eq!(extension.path(), "opaque.bin");
    assert_eq!(extension.bytes(), &[7, 8, 9]);
    assert!(!extension.required());
}

#[test]
fn save_writes_to_the_known_path_without_asking_again() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resaved.ketchup");
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .always_confirm_high_risk_as(8);
    let mut shell = Shell::with_dialogs(script.clone());

    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(path.is_file());

    shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0));
    shell.settle();
    let edited_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Save);

    assert_eq!(
        script.suggested_names().len(),
        1,
        "a document with a path must be saved without a second dialog"
    );
    let on_disk = ketchup_core::persistence::load_file(&path)
        .expect("the saved document reloads")
        .snapshot()
        .canonical_digest();
    assert_eq!(
        on_disk, edited_digest,
        "Save must persist the current state of the document"
    );
    assert!(!shell.app().is_dirty());
}

#[test]
fn dirty_gui_document_recovers_after_crash_and_requires_save_as() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("crashed.ketchup");
    let recovered_copy = directory.path().join("recovered.ketchup");
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    author.click_menu_command("menu-file", AppCommand::Save);
    let clean_digest = author.app().canonical_digest();

    let mut crashed = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&primary));
    crashed.click_menu_command("menu-file", AppCommand::Open);
    assert!(crashed.app_mut().create_box());
    crashed.settle();
    let dirty_digest = crashed.app().canonical_digest();
    let dirty_undo = crashed.app().undo_step_count();
    assert!(ketchup_core::persistence::work_recovery_path(&primary).is_file());
    drop(crashed);

    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&primary)
        .queue_save(&recovered_copy);
    let mut recovered = Shell::with_dialogs(dialogs);
    recovered.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(recovered.app().canonical_digest(), dirty_digest);
    assert_eq!(recovered.app().undo_step_count(), dirty_undo);
    assert_eq!(recovered.app().document_path(), None);
    assert!(recovered.app().is_dirty());
    assert_eq!(
        recovered.app().recovery_source_path(),
        Some(ketchup_core::persistence::work_recovery_path(&primary).as_path())
    );
    assert_eq!(
        ketchup_core::persistence::load(&std::fs::read(&primary).unwrap())
            .unwrap()
            .snapshot()
            .canonical_digest(),
        clean_digest
    );

    recovered.click_menu_command("menu-file", AppCommand::Save);
    assert_eq!(
        recovered.app().document_path(),
        Some(recovered_copy.as_path())
    );
    assert!(!recovered.app().is_dirty());
    assert!(!ketchup_core::persistence::work_recovery_path(&primary).exists());
}

#[test]
fn gui_retries_failed_post_save_work_recovery_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("retry-save-cleanup.ketchup");
    let recovery = ketchup_core::persistence::work_recovery_path(&primary);
    let mut shell = Shell::with_dialogs(
        ScriptedFileDialogs::new()
            .queue_save(&primary)
            .always_confirm_high_risk_as(8),
    );
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(shell.app_mut().create_box());
    shell.settle();
    let owned_checkpoint = std::fs::read(&recovery).unwrap();

    std::fs::remove_file(&recovery).unwrap();
    std::fs::create_dir(&recovery).unwrap();
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(!shell.app().is_dirty());

    std::fs::remove_dir(&recovery).unwrap();
    std::fs::write(&recovery, owned_checkpoint).unwrap();
    shell.settle();
    assert!(!recovery.exists());
}

#[test]
fn gui_undo_and_redo_roll_back_when_work_recovery_checkpoint_fails() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("transactional-undo.ketchup");
    let recovery = ketchup_core::persistence::work_recovery_path(&primary);
    let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(shell.app_mut().create_box());
    assert!(shell.app_mut().create_box());
    shell.settle();
    assert!(recovery.is_file());

    let before = canonical_state(&shell);
    std::fs::remove_file(&recovery).unwrap();
    std::fs::create_dir(&recovery).unwrap();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-save-document"));

    std::fs::remove_dir(&recovery).unwrap();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert!(recovery.is_file());
    let undone = canonical_state(&shell);
    assert_eq!(
        ketchup_core::persistence::load_file(&primary)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        undone.digest
    );
    std::fs::remove_file(&recovery).unwrap();
    std::fs::create_dir(&recovery).unwrap();
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(canonical_state(&shell), undone);
    assert!(digest_starts_like(&shell, "error-save-document"));
}

#[test]
fn gui_canonical_edit_rolls_back_when_work_recovery_checkpoint_fails() {
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("transactional-edit.ketchup");
    let recovery = ketchup_core::persistence::work_recovery_path(&primary);
    let mut shell = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&primary));
    shell.click_menu_command("menu-file", AppCommand::Save);
    let before = canonical_state(&shell);

    std::fs::create_dir(&recovery).unwrap();
    assert!(!shell.app_mut().create_box());
    shell.settle();

    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-save-document"));
}

#[test]
fn step_import_publishes_its_blob_in_the_same_work_recovery_transaction() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let directory = tempfile::tempdir().unwrap();
    let primary = directory.path().join("step-import-recovery.ketchup");
    let recovery = ketchup_core::persistence::work_recovery_path(&primary);
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&primary)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source);
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let baseline = canonical_state(&shell);

    std::fs::create_dir(&recovery).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert_eq!(canonical_state(&shell), baseline);
    std::fs::remove_dir(&recovery).unwrap();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));

    assert_eq!(shell.app().import_receipt_count(), 1);
    assert!(recovery.is_file());
    let recovered = ketchup_core::persistence::load_file_with_source(&primary).unwrap();
    assert_eq!(recovered.source_path(), recovery);
    assert_eq!(recovered.outcome().snapshot().import_receipts().count(), 1);
}

#[test]
fn gui_save_as_cleanup_preserves_another_windows_newer_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let shared = directory.path().join("shared-recovery.ketchup");
    let alternate = directory.path().join("first-copy.ketchup");
    let recovery = ketchup_core::persistence::work_recovery_path(&shared);
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&shared));
    author.click_menu_command("menu-file", AppCommand::Save);

    let mut first = Shell::with_dialogs(
        ScriptedFileDialogs::new()
            .queue_open(&shared)
            .queue_save(&alternate),
    );
    let mut second = Shell::with_dialogs(ScriptedFileDialogs::new().queue_open(&shared));
    first.click_menu_command("menu-file", AppCommand::Open);
    second.click_menu_command("menu-file", AppCommand::Open);
    assert!(first.app_mut().create_box());
    first.settle();
    assert!(second.app_mut().create_box());
    assert!(second.app_mut().create_box());
    second.settle();
    let second_digest = second.app().canonical_digest();
    let newer_checkpoint = std::fs::read(&recovery).unwrap();

    first.click_menu_command("menu-file", AppCommand::SaveAs);

    assert_eq!(first.app().document_path(), Some(alternate.as_path()));
    assert_eq!(std::fs::read(&recovery).unwrap(), newer_checkpoint);
    let recovered = ketchup_core::persistence::load_file_with_source(&shared).unwrap();
    assert_eq!(recovered.source_path(), recovery);
    assert_eq!(
        recovered.outcome().snapshot().canonical_digest(),
        second_digest
    );
}

#[test]
fn two_open_windows_detect_external_save_and_offer_safe_save_as() {
    let directory = tempfile::tempdir().unwrap();
    let shared = directory.path().join("shared.ketchup");
    let alternate = directory.path().join("second-copy.ketchup");
    let mut author = Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&shared));
    author.click_menu_command("menu-file", AppCommand::Save);

    let first_dialogs = ScriptedFileDialogs::new()
        .queue_open(&shared)
        .always_confirm_high_risk_as(31);
    let second_dialogs = ScriptedFileDialogs::new()
        .queue_open(&shared)
        .queue_save(&alternate)
        .always_confirm_high_risk_as(32);
    let mut first = Shell::with_dialogs(first_dialogs);
    let mut second = Shell::with_dialogs(second_dialogs.clone());
    first.click_menu_command("menu-file", AppCommand::Open);
    second.click_menu_command("menu-file", AppCommand::Open);

    assert!(first.app_mut().create_box());
    first.click_menu_command("menu-file", AppCommand::Save);
    let first_bytes = std::fs::read(&shared).unwrap();
    let first_digest = first.app().canonical_digest();

    let stale_state = canonical_state(&second);
    assert!(!second.app_mut().create_box());

    assert_eq!(std::fs::read(&shared).unwrap(), first_bytes);
    assert!(!second.app().is_dirty());
    assert!(digest_starts_like(&second, "error-save-document"));
    assert!(
        second
            .app()
            .action_digest()
            .contains("changed outside this session")
    );
    assert_eq!(canonical_state(&second), stale_state);
    assert_eq!(
        second_dialogs.high_risk_prompts().len(),
        0,
        "a stale file must be rejected before requesting overwrite consent"
    );

    second.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(!second.app().is_dirty());
    assert_eq!(second.app().document_path(), Some(alternate.as_path()));
    assert!(second.app_mut().create_box());
    second.click_menu_command("menu-file", AppCommand::Save);
    let independent_digest = second.app().canonical_digest();
    assert_eq!(
        ketchup_core::persistence::load_file(&shared)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        first_digest
    );
    assert_eq!(
        ketchup_core::persistence::load_file(&alternate)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        independent_digest
    );
}

#[test]
fn save_as_requires_explicit_consent_before_preserving_current_revision_without_full_history() {
    let directory = tempfile::tempdir().unwrap();
    let boundary = directory.path().join("history-boundary.ketchup");
    let saved = directory.path().join("current-only.ketchup");
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "history counter".to_owned(),
                dimension: Dimension::new("0", 0.0).unwrap(),
                dependencies: Vec::new(),
            },
        ]))
        .unwrap();
    for value in 1..4_095_u64 {
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetEvaluatorDimension {
                    id: NodeId(1),
                    dimension: Dimension::new(value.to_string(), value as f64).unwrap(),
                },
            ]))
            .unwrap();
    }
    assert_eq!(document.revision_count(), 4_096);
    ketchup_core::persistence::save_atomic_document_store_with_container(
        &boundary,
        &document,
        &ketchup_core::persistence::ContainerData::default(),
    )
    .unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_open(&boundary)
        .queue_open(&saved)
        .queue_save(&saved)
        .queue_save(&saved)
        .queue_history_truncation_approval(false)
        .queue_history_truncation_approval(true);
    let mut shell = Shell::with_dialogs(script.clone());
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().undo_step_count(), 4_095);
    assert!(shell.app_mut().create_box());
    shell.settle();
    let expected_digest = shell.app().canonical_digest();
    let expected_revision = shell.app().document_revision();
    let expected_undo_steps = shell.app().undo_step_count();

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(!saved.exists());
    assert!(shell.app().is_dirty());
    assert_eq!(shell.app().canonical_digest(), expected_digest);
    assert_eq!(shell.app().undo_step_count(), expected_undo_steps);
    assert_eq!(script.history_truncation_prompts().len(), 1);
    assert!(script.history_truncation_prompts()[0].contains("4096"));

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    assert!(!shell.app().is_dirty());
    assert_eq!(shell.app().canonical_digest(), expected_digest);
    assert_eq!(shell.app().document_revision(), expected_revision);
    assert_eq!(shell.app().undo_step_count(), 0);
    assert_eq!(shell.app().redo_step_count(), 0);
    assert_eq!(script.history_truncation_prompts().len(), 2);
    assert!(digest_starts_like(
        &shell,
        "digest-saved-document-current-only"
    ));

    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), expected_digest);
    assert_eq!(shell.app().document_revision(), expected_revision);
    assert_eq!(shell.app().undo_step_count(), 0);
    assert_eq!(shell.app().redo_step_count(), 0);
    assert!(!shell.app().is_dirty());
    assert!(shell.app_mut().create_box());
    assert!(shell.app().document_revision() > expected_revision);
}

#[test]
fn overwrite_save_requires_payload_bound_human_receipt_before_disk_write() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.ketchup");
    let original = b"existing file must survive refusal".to_vec();
    std::fs::write(&path, &original).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_save(&path)
        .queue_refused_high_risk()
        .queue_high_risk_approval(42);
    let mut shell = Shell::with_dialogs(script.clone());
    compose_two_shared_occurrences(&mut shell);
    let canonical = shell.app().canonical_digest();
    let revision = shell.app().document_revision();

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(shell.app().is_dirty());
    assert!(shell.app().last_side_effect_receipt().is_none());

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    let receipt = shell
        .app()
        .last_side_effect_receipt()
        .expect("approved overwrite returns an authorization receipt");
    assert_eq!(receipt.approving_human(), 42);
    assert_eq!(receipt.revision_id(), revision);
    assert_eq!(receipt.operation(), "overwrite-native-document");
    assert_eq!(
        receipt.scope().path(),
        Some(path.display().to_string().as_str())
    );
    assert_eq!(shell.app().canonical_digest(), canonical);
    assert_eq!(shell.app().document_revision(), revision);
    assert!(!shell.app().is_dirty());
    assert_eq!(script.high_risk_prompts().len(), 2);
    assert!(script.high_risk_prompts()[0].contains("Payload SHA-256:"));
    assert_eq!(
        ketchup_core::persistence::load_file(&path)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        canonical
    );
}

#[test]
fn a_failed_open_leaves_the_active_document_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.ketchup");
    std::fs::write(&malformed, b"not a ketchup document").unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_open(&malformed)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    let before = shell.app().canonical_digest();
    let revision = shell.app().document_revision();

    shell.click_menu_command("menu-file", AppCommand::Open);

    assert_eq!(
        shell.app().canonical_digest(),
        before,
        "a failed Open must not replace the active document"
    );
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().document_path(), None);
    assert!(
        digest_starts_like(&shell, "error-open-document"),
        "a failed Open must report the localized open error, digest was {:?}",
        shell.app().action_digest()
    );
}

#[test]
fn a_failed_save_keeps_the_document_dirty_and_reports_the_reason() {
    let directory = tempfile::tempdir().unwrap();
    let script = ScriptedFileDialogs::new().queue_save(directory.path());
    let mut shell = Shell::with_dialogs(script);

    compose_two_shared_occurrences(&mut shell);
    let before = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::Save);

    assert_eq!(shell.app().canonical_digest(), before);
    assert!(
        shell.app().is_dirty(),
        "a failed Save must leave the document unsaved"
    );
    assert_eq!(shell.app().document_path(), None);
    assert!(
        digest_starts_like(&shell, "error-save-document"),
        "a failed Save must report the localized save error, digest was {:?}",
        shell.app().action_digest()
    );
}

#[test]
fn discarding_unsaved_work_is_confirmed_before_new_replaces_it() {
    let refused = ScriptedFileDialogs::new();
    let mut shell = Shell::with_dialogs(refused.clone());
    compose_two_shared_occurrences(&mut shell);
    let composed = shell.app().canonical_digest();

    shell.click_menu_command("menu-file", AppCommand::New);

    assert_eq!(refused.discard_prompts(), 1, "the shell must ask first");
    assert_eq!(
        shell.app().canonical_digest(),
        composed,
        "a refused prompt must keep the composed model"
    );
    assert_eq!(shell.app().active_box_count(), 2);
}

#[test]
fn file_menu_exports_fail_closed_when_current_exact_evidence_is_unavailable() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let stl = directory.path().join("preserved.stl");
    let stl_loss = stl.with_extension("stl.loss.txt");
    let step = directory.path().join("preserved.step");
    let step_loss = step.with_extension("step.loss.txt");
    let unavailable_worker = directory.path().join("not-an-exact-worker");
    let original_stl = b"preserve STL when exact evidence is unavailable";
    let original_stl_loss = b"preserve STL loss report when exact evidence is unavailable";
    let original_step = b"preserve STEP when exact evidence is unavailable";
    let original_step_loss = b"preserve STEP loss report when exact evidence is unavailable";
    std::fs::write(&stl, original_stl).unwrap();
    std::fs::write(&stl_loss, original_stl_loss).unwrap();
    std::fs::write(&step, original_step).unwrap();
    std::fs::write(&step_loss, original_step_loss).unwrap();
    std::fs::write(&unavailable_worker, b"not an executable worker").unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_export(&stl)
        .queue_export(&step)
        .always_confirm_high_risk_as(91);
    let mut shell = Shell::with_dialogs(script.clone());
    shell
        .app_mut()
        .connect_exact_worker(&unavailable_worker)
        .unwrap();
    shell.settle();
    assert_eq!(shell.app().exact_render_body_count(), 0);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);

    assert_eq!(std::fs::read(&stl).unwrap(), original_stl);
    assert_eq!(std::fs::read(&stl_loss).unwrap(), original_stl_loss);
    assert_eq!(std::fs::read(&step).unwrap(), original_step);
    assert_eq!(std::fs::read(&step_loss).unwrap(), original_step_loss);
    assert!(
        script.high_risk_prompts().is_empty(),
        "missing current evidence must be rejected before lossy confirmation"
    );
    assert!(shell.app().last_side_effect_receipt().is_none());
}

#[test]
fn file_menu_exports_current_visible_exact_model_without_mutating_canonical_state() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let protected_stl = directory.path().join("protected.stl");
    let protected_loss = protected_stl.with_extension("stl.loss.txt");
    let first_stl = directory.path().join("visible-a.stl");
    let second_stl = directory.path().join("visible-b.stl");
    let step = directory.path().join("visible.step");
    let script = ScriptedFileDialogs::new()
        .queue_cancelled_export()
        .queue_cancelled_export()
        .queue_export(&protected_stl)
        .queue_export(&protected_stl)
        .queue_export(&first_stl)
        .queue_export(&second_stl)
        .queue_export(&step)
        .queue_refused_high_risk()
        .queue_high_risk_approval(40)
        .queue_high_risk_approval(41)
        .queue_high_risk_approval(42)
        .queue_high_risk_approval(43)
        .queue_high_risk_approval(44)
        .queue_high_risk_approval(45);
    let mut shell = Shell::with_dialogs(script.clone());
    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        0,
        "cancelling either export dialog must not create an artifact"
    );
    assert!(script.high_risk_prompts().is_empty());

    let original_stl = b"existing STL must survive refusal";
    let original_loss = b"existing STL loss report must survive refusal";
    std::fs::write(&protected_stl, original_stl).unwrap();
    std::fs::write(&protected_loss, original_loss).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(std::fs::read(&protected_stl).unwrap(), original_stl);
    assert_eq!(std::fs::read(&protected_loss).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_eq!(canonical_state(&shell), before);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_ne!(std::fs::read(&protected_stl).unwrap(), original_stl);
    assert_ne!(std::fs::read(&protected_loss).unwrap(), original_loss);
    assert_eq!(canonical_state(&shell), before);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    assert_eq!(canonical_state(&shell), before);
    let first_stl_bytes = std::fs::read(&first_stl).unwrap();
    let second_stl_bytes = std::fs::read(&second_stl).unwrap();
    assert_eq!(
        first_stl_bytes, second_stl_bytes,
        "the same visible exact model must produce byte-identical STL"
    );
    assert_eq!(
        ascii_stl_facet_count(&first_stl_bytes),
        12,
        "the hidden shared occurrence must not contribute facets"
    );
    let stl_loss = std::fs::read_to_string(first_stl.with_extension("stl.loss.txt")).unwrap();
    assert!(stl_loss.contains("format=ASCII STL"));
    assert!(stl_loss.contains("editability_loss="));
    assert!(stl_loss.contains("topology_loss="));
    assert!(stl_loss.contains(&format!("source_digest={}", before.digest)));

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert!(
        step.is_file(),
        "STEP export failed with digest {:?}",
        shell.app().action_digest()
    );
    assert!(
        std::fs::read_to_string(&step)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    let step_loss = std::fs::read_to_string(step.with_extension("step.loss.txt")).unwrap();
    assert!(step_loss.contains("format=ISO 10303 STEP"));
    assert!(step_loss.contains("editability_loss="));
    assert!(step_loss.contains(&format!("source_digest={}", before.digest)));

    let requests = script.export_requests();
    assert_eq!(requests.len(), 7);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.extension.as_str())
            .collect::<Vec<_>>(),
        vec!["stl", "step", "stl", "stl", "stl", "stl", "step"]
    );
    for request in &requests {
        assert!(
            request
                .filter_label
                .to_ascii_lowercase()
                .contains(request.extension.as_str()),
            "the recorded filter must identify its format: {request:?}"
        );
        assert_eq!(
            request.suggested_name,
            format!("Untitled.{}", request.extension)
        );
    }
    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 7);
    assert!(prompts[2].contains(&protected_stl.display().to_string()));
    assert!(prompts[3].contains(&protected_loss.display().to_string()));
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("Payload SHA-256:"))
    );
}

#[test]
fn file_menu_three_mf_export_is_localized_atomic_and_non_mutating() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let three_mf = directory.path().join("protected.3mf");
    let loss = three_mf.with_extension("3mf.loss.txt");
    let original_model = b"preserve 3MF until overwrite approval";
    let original_loss = b"preserve 3MF loss report until overwrite approval";
    std::fs::write(&three_mf, original_model).unwrap();
    std::fs::write(&loss, original_loss).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_export(&three_mf)
        .queue_export(&three_mf)
        .queue_refused_high_risk()
        .queue_high_risk_approval(121)
        .queue_high_risk_approval(122)
        .queue_high_risk_approval(123);
    let mut shell = Shell::with_dialogs(script.clone());
    arrange_one_visible_and_one_hidden_occurrence_with_redo(&mut shell);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportPrintThreeMf);
    assert_eq!(std::fs::read(&three_mf).unwrap(), original_model);
    assert_eq!(std::fs::read(&loss).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    shell.click_menu_command("menu-file", AppCommand::ExportPrintThreeMf);
    let package = std::fs::read(&three_mf).unwrap();
    assert_eq!(&package[..4], b"PK\x03\x04");
    assert!(
        package
            .windows(b"3D/3dmodel.model".len())
            .any(|window| window == b"3D/3dmodel.model")
    );
    assert!(
        package
            .windows(b"unit=\"millimeter\"".len())
            .any(|window| window == b"unit=\"millimeter\"")
    );
    let report = std::fs::read_to_string(&loss).unwrap();
    assert!(report.contains("format=3MF Core 1.3 package"));
    assert!(report.contains("hierarchy=canonical global groups"));
    assert!(report.contains(&format!("source_digest={}", before.digest)));
    assert!(digest_starts_like(&shell, "digest-exported-3mf"));
    assert!(shell.app().last_side_effect_receipt().is_some());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    let requests = script.export_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        request.extension == "3mf"
            && request.filter_label.to_ascii_lowercase().contains("3mf")
            && request.suggested_name == "Untitled.3mf"
    }));
    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 4);
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("Payload SHA-256:"))
    );
}

#[test]
fn file_menu_round_trips_mixed_exact_and_imported_mesh_scene_to_stl_glb_and_three_mf() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let directory = tempfile::tempdir().unwrap();
    let stl = directory.path().join("mixed.stl");
    let glb = directory.path().join("mixed.glb");
    let three_mf = directory.path().join("mixed.3mf");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Glb, &fixture)
        .queue_export(&stl)
        .queue_export(&glb)
        .queue_export(&three_mf)
        .always_confirm_high_risk_as(131);
    let mut shell = Shell::with_dialogs(script.clone());
    compose_two_shared_occurrences(&mut shell);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportMeshStl);
    shell.click_menu_command("menu-file", AppCommand::ExportBlenderGlb);
    shell.click_menu_command("menu-file", AppCommand::ExportPrintThreeMf);

    let stl_bytes = std::fs::read(&stl).unwrap();
    assert!(ascii_stl_facet_count(&stl_bytes) > 1_000);
    let stl_report = std::fs::read_to_string(stl.with_extension("stl.loss.txt")).unwrap();
    assert!(stl_report.contains("canonical_mesh_occurrence_count=140"));
    assert!(stl_report.contains("color_loss=STL does not preserve occurrence colors"));

    let glb_bytes = std::fs::read(&glb).unwrap();
    let glb_scene = ketchup_core::import::inspect_glb(&glb_bytes).unwrap();
    assert_eq!(glb_scene.instance_count(), 142);
    let mut round_trip = DocumentStore::new();
    let batch = ketchup_core::import::plan_glb_import(
        &round_trip.current(),
        &glb_bytes,
        "mixed-round-trip.glb",
    )
    .unwrap();
    round_trip.apply_batch(&batch).unwrap();
    let round_trip_digest = round_trip.current().canonical_digest();
    let persisted = ketchup_core::persistence::save(&round_trip.current());
    assert_eq!(
        ketchup_core::persistence::load(&persisted)
            .unwrap()
            .snapshot()
            .canonical_digest(),
        round_trip_digest
    );
    let glb_report = std::fs::read_to_string(glb.with_extension("glb.loss.txt")).unwrap();
    assert!(glb_report.contains("canonical_mesh_occurrence_count=140"));
    assert!(glb_report.contains("materials=resolved occurrence sRGB colors"));

    let three_mf_bytes = std::fs::read(&three_mf).unwrap();
    assert_eq!(&three_mf_bytes[..4], b"PK\x03\x04");
    assert!(
        three_mf_bytes
            .windows(b"displaycolor=\"#".len())
            .any(|bytes| bytes == b"displaycolor=\"#")
    );
    let three_mf_report = std::fs::read_to_string(three_mf.with_extension("3mf.loss.txt")).unwrap();
    assert!(three_mf_report.contains("canonical_mesh_occurrence_count=140"));
    assert!(three_mf_report.contains("materials=resolved occurrence sRGB colors"));

    assert_eq!(script.high_risk_prompts().len(), 3);
    assert!(shell.app().last_side_effect_receipt().is_some());
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_menu_step_export_requires_bound_overwrite_approval_without_canonical_mutation() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let step = directory.path().join("protected.step");
    let loss = step.with_extension("step.loss.txt");
    let original_step = b"preserve STEP until overwrite approval";
    let original_loss = b"preserve STEP loss report until overwrite approval";
    std::fs::write(&step, original_step).unwrap();
    std::fs::write(&loss, original_loss).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_export(&step)
        .queue_export(&step)
        .queue_refused_high_risk()
        .queue_high_risk_approval(111)
        .queue_high_risk_approval(112)
        .queue_high_risk_approval(113);
    let mut shell = Shell::with_dialogs(script.clone());
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_current_exact_body(&mut shell);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(std::fs::read(&step).unwrap(), original_step);
    assert_eq!(std::fs::read(&loss).unwrap(), original_loss);
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_ne!(std::fs::read(&step).unwrap(), original_step);
    assert_ne!(std::fs::read(&loss).unwrap(), original_loss);
    assert!(
        std::fs::read_to_string(&step)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    assert!(shell.app().last_side_effect_receipt().is_some());
    assert_state_and_history_unchanged(&mut shell, &before, &history);

    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 4);
    assert!(
        prompts
            .iter()
            .all(|prompt| prompt.contains("Payload SHA-256:"))
    );
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains(&step.display().to_string()))
    );
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt.contains(&loss.display().to_string()))
    );
}

#[test]
fn file_menu_iges_export_and_localized_import_are_bound_atomic_and_renderable() {
    let _serial = EXACT_FILE_EXPORT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    let step_source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let import_source = directory.path().join("source.iges");
    let cancelled = AtomicBool::new(false);
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    inspector
        .convert_step_to_iges_with_cancellation(&step_source, &import_source, &cancelled)
        .unwrap();
    let source_bytes = std::fs::read(&import_source).unwrap();
    let source_evidence = inspector
        .inspect_iges_import_with_cancellation(
            &import_source,
            &ketchup_core::graph::sha256_hex(&source_bytes),
            &cancelled,
        )
        .unwrap();

    let exported = directory.path().join("protected.iges");
    let exported_loss = exported.with_extension("iges.loss.txt");
    let original_export = b"preserve IGES until overwrite approval";
    let original_loss = b"preserve IGES loss report until overwrite approval";
    std::fs::write(&exported, original_export).unwrap();
    std::fs::write(&exported_loss, original_loss).unwrap();
    let export_dialogs = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &step_source)
        .queue_export(&exported)
        .queue_export(&exported)
        .queue_refused_high_risk()
        .queue_high_risk_approval(201)
        .queue_high_risk_approval(202)
        .queue_high_risk_approval(203);
    let mut export_shell = Shell::with_dialogs(export_dialogs.clone());
    export_shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    export_shell.click_at(export_shell.viewport_rect().center());
    assert!(export_shell.app_mut().delete_selected());
    export_shell.settle();
    export_shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    export_shell.click_button_label(&export_shell.catalog().text("dialog-import-step-confirm"));
    let export_feature = imported_exact_feature_id(&export_shell);
    for _ in 0..100 {
        export_shell.settle();
        if export_shell.app().exact_current_producer_ids() == [export_feature] {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        export_shell.app().exact_current_producer_ids(),
        [export_feature]
    );
    let export_before = canonical_state(&export_shell);
    let export_history = reachable_history_digests(&mut export_shell);
    for _ in 0..100 {
        export_shell.settle();
        if export_shell.app().exact_current_producer_ids() == [export_feature] {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        export_shell.app().exact_current_producer_ids(),
        [export_feature]
    );

    export_shell.click_menu_command("menu-file", AppCommand::ExportExactIges);
    assert_eq!(std::fs::read(&exported).unwrap(), original_export);
    assert_eq!(std::fs::read(&exported_loss).unwrap(), original_loss);
    assert_state_and_history_unchanged(&mut export_shell, &export_before, &export_history);
    for _ in 0..100 {
        export_shell.settle();
        if export_shell.app().exact_current_producer_ids() == [export_feature] {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        export_shell.app().exact_current_producer_ids(),
        [export_feature]
    );

    export_shell.click_menu_command("menu-file", AppCommand::ExportExactIges);
    let exported_bytes = std::fs::read(&exported).unwrap();
    assert_ne!(
        exported_bytes,
        original_export,
        "IGES export failed with digest {:?}",
        export_shell.app().action_digest()
    );
    let report = std::fs::read_to_string(&exported_loss).unwrap();
    assert!(report.contains("format=IGES 5.3"));
    assert!(report.contains("assembly_loss="));
    assert!(report.contains(&format!("source_digest={}", export_before.digest)));
    let exported_evidence = inspector
        .inspect_iges_import_with_cancellation(
            &exported,
            &ketchup_core::graph::sha256_hex(&exported_bytes),
            &cancelled,
        )
        .unwrap();
    assert_eq!(exported_evidence.source_unit, ImportLengthUnit::Millimetre);
    assert_eq!(exported_evidence.solid_count, 1);
    assert_state_and_history_unchanged(&mut export_shell, &export_before, &export_history);
    assert_eq!(export_dialogs.high_risk_prompts().len(), 4);

    let import_dialogs = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Iges, &import_source)
        .queue_import(ImportFormat::Iges, &import_source);
    let mut import_shell = Shell::with_catalog_and_dialogs(
        ketchup_interaction::LocaleCatalog::slovak(),
        import_dialogs.clone(),
    );
    import_shell.app_mut().enable_headless_instanced_scene();
    import_shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    import_shell.click_at(import_shell.viewport_rect().center());
    assert!(import_shell.app_mut().delete_selected());
    import_shell.settle();
    let import_before = canonical_state(&import_shell);
    let import_history = reachable_history_digests(&mut import_shell);

    import_shell.click_menu_command("menu-file", AppCommand::ImportExactIges);
    let confirm = import_shell.catalog().text("dialog-import-iges-confirm");
    assert!(import_shell.has_role_and_label(Role::Button, &confirm));
    std::fs::write(&import_source, b"changed after IGES preview").unwrap();
    import_shell.click_button_label(&confirm);
    assert!(digest_starts_like(&import_shell, "error-import-iges"));
    assert_state_and_history_unchanged(&mut import_shell, &import_before, &import_history);

    std::fs::write(&import_source, &source_bytes).unwrap();
    import_shell.click_menu_command("menu-file", AppCommand::ImportExactIges);
    import_shell.click_button_label(&confirm);
    assert_eq!(
        import_shell.app().document_revision(),
        import_before.revision + 1
    );
    assert_eq!(import_shell.app().import_receipt_count(), 1);
    assert!(digest_starts_like(&import_shell, "digest-imported-iges"));
    let imported_snapshot = import_shell.app().document_snapshot();
    let receipt = imported_snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Iges)
        .unwrap();
    assert_eq!(receipt.units().source_unit(), source_evidence.source_unit);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    let imported_feature = imported_exact_feature_id(&import_shell);
    for _ in 0..100 {
        import_shell.settle();
        if import_shell.app().exact_current_producer_ids() == [imported_feature]
            && import_shell.app().instanced_scene_triangle_count() > 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(import_shell.app().exact_render_body_count(), 1);
    assert!(import_shell.app().exact_render_triangle_count() > 0);
    assert_eq!(
        import_dialogs.import_requests(),
        vec![
            ketchup_app::dialogs::ImportDialogRequestRecord {
                format: ImportFormat::Iges,
                filter_label: import_shell.catalog().text("file-filter-iges"),
                extensions: vec!["iges".to_owned(), "igs".to_owned()],
            },
            ketchup_app::dialogs::ImportDialogRequestRecord {
                format: ImportFormat::Iges,
                filter_label: import_shell.catalog().text("file-filter-iges"),
                extensions: vec!["iges".to_owned(), "igs".to_owned()],
            },
        ]
    );
}

#[test]
fn file_import_sketchup_scene_preserves_shared_instances_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let source = valid_sketchup_scene_bridge();
    let path = directory.path().join("shared.kscene");
    let document_path = directory.path().join("shared.ketchup");
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::SketchupScene, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportSketchupScene);
    shell.click_button_label(&shell.catalog().text("dialog-import-sketchup-scene-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 1);
    assert_eq!(shell.app().feature_count(), before.features + 1);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 2);
    assert_eq!(shell.app().mesh_body_count(), before.mesh_bodies + 1);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    assert!(digest_starts_like(&shell, "digest-imported-sketchup-scene"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::SketchupScene,
            filter_label: shell.catalog().text("file-filter-sketchup-scene"),
            extensions: vec!["kscene".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let snapshot = loaded.snapshot();
    let receipt = snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::SketchupScene);
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.units().source_unit(), ImportLengthUnit::Inch);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert_eq!(snapshot.definitions().count(), before.definitions + 1);
    assert_eq!(snapshot.occurrences().count(), before.occurrences + 2);
    let imported_mesh = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(mesh)
                if matches!(
                    mesh.authority,
                    ketchup_core::document::MeshAuthority::ImportedSketchupScene { .. }
                ) =>
            {
                Some(mesh)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(imported_mesh.vertices_mm[1], [25.4, 0.0, 0.0]);
}

#[test]
fn file_import_blender_glb_reviews_and_commits_one_mesh_scene_offscreen() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let source = std::fs::read(&source_path).unwrap();
    let review = ketchup_core::import::inspect_glb(&source).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Glb, &source_path);
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(
        shell.app().definition_count(),
        before.definitions + review.mesh_primitive_count()
    );
    assert_eq!(
        shell.app().occurrence_count(),
        before.occurrences + review.instance_count()
    );
    assert_eq!(
        shell.app().mesh_body_count(),
        before.mesh_bodies + review.mesh_primitive_count()
    );
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    assert!(digest_starts_like(&shell, "digest-imported-glb"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Glb,
            filter_label: shell.catalog().text("file-filter-glb"),
            extensions: vec!["glb".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .import_receipts()
            .find(|receipt| receipt.format() == ImportFormat::Glb)
            .unwrap()
            .source_sha256(),
        &sha256_bytes(&source)
    );
}

#[test]
fn file_import_blender_glb_cancel_source_change_and_stale_review_do_not_mutate() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let original = std::fs::read(&fixture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.glb");
    std::fs::write(&source, &original).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Glb, &source)
        .queue_import(ImportFormat::Glb, &source)
        .queue_import(ImportFormat::Glb, &source);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    let mut replacement = original.clone();
    replacement[0] ^= 1;
    std::fs::write(&source, replacement).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    assert!(digest_starts_like(&shell, "error-import-glb"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    std::fs::write(&source, original).unwrap();
    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    assert!(digest_starts_like(&shell, "error-import-glb"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);
}

#[test]
fn blender_mesh_exact_conversion_requires_review_and_one_confirmed_undo_step() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Glb, &source);
    let mut shell = Shell::with_dialogs(script);
    shell
        .app_mut()
        .set_assistant_workspace_mode(ketchup_app::AssistantWorkspaceMode::Tab);
    shell
        .app_mut()
        .headless_force_exact_worker_path(exact_worker_path());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    let snapshot = shell.app().document_snapshot();
    let recognized = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::MeshBody(mesh)
                if matches!(
                    recognize_mesh_body(mesh, 0.25),
                    MeshRecognition::Candidate { .. }
                ) =>
            {
                Some((feature.id(), feature.definition_id()))
            }
            _ => None,
        })
        .expect("the real Blender fixture must contain a supported simple mesh");
    let occurrence_id = snapshot
        .occurrences()
        .find(|occurrence| occurrence.definition_id() == recognized.1)
        .expect("the recognized Blender mesh must have a placed occurrence")
        .id();
    drop(snapshot);
    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
    shell.settle();
    let imported = canonical_state(&shell);
    let imported_history = reachable_history_digests(&mut shell);

    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
    shell.settle();
    shell.click_menu_command("menu-model", AppCommand::ConvertSelectedMeshToExact);
    assert_eq!(canonical_state(&shell), imported);
    assert!(
        shell.has_visible_label(&shell.catalog().text("dialog-mesh-conversion-title")),
        "{}",
        shell.app().action_digest()
    );
    shell.click_button_label(&shell.catalog().text("dialog-mesh-conversion-cancel"));
    assert_state_and_history_unchanged(&mut shell, &imported, &imported_history);

    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
    shell.settle();
    shell.click_menu_command("menu-model", AppCommand::ConvertSelectedMeshToExact);
    assert!(shell.app_mut().create_box());
    assert!(shell.app_mut().undo());
    let after_aba = canonical_state(&shell);
    let after_aba_history = reachable_history_digests(&mut shell);
    assert!(!shell.has_visible_label(&shell.catalog().text("dialog-mesh-conversion-confirm")));
    assert_state_and_history_unchanged(&mut shell, &after_aba, &after_aba_history);

    assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
    shell.settle();
    shell.click_menu_command("menu-model", AppCommand::ConvertSelectedMeshToExact);
    assert_eq!(canonical_state(&shell), after_aba);
    let confirm_label = shell.catalog().text("dialog-mesh-conversion-confirm");
    wait_for_visible_label(&mut shell, &confirm_label);
    shell.click_button_label(&confirm_label);
    assert!(shell.app().document_revision() > after_aba.revision);
    assert_eq!(shell.app().mesh_body_count(), after_aba.mesh_bodies - 1);
    assert_eq!(shell.app().undo_step_count(), after_aba.undo_steps + 1);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(recognized.0)
            .expect("converted feature keeps the source feature ID")
            .kind(),
        FeatureKind::Profile { .. } | FeatureKind::Workplane(_)
    ));

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), after_aba.digest);
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(recognized.0)
            .expect("Undo restores the source mesh")
            .kind(),
        FeatureKind::MeshBody(_)
    ));
}

#[test]
fn imported_blender_mesh_blocks_exact_and_btlx_exports_before_side_effects() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/blender/garden-studio-colored.glb");
    let directory = tempfile::tempdir().unwrap();
    let step = directory.path().join("blocked.step");
    let btlx = directory.path().join("blocked.btlx");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Glb, &source)
        .queue_export(&step)
        .queue_export(&btlx)
        .always_confirm_high_risk_as(91);
    let mut shell = Shell::with_dialogs(script.clone());

    shell.click_menu_command("menu-file", AppCommand::ImportBlenderGlb);
    shell.click_button_label(&shell.catalog().text("dialog-import-glb-confirm"));
    let imported = canonical_state(&shell);
    let imported_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert!(digest_starts_like(&shell, "error-export-step"));
    assert!(
        shell.app().action_digest().contains("mesh body"),
        "{}",
        shell.app().action_digest()
    );
    assert!(!step.exists());
    assert_state_and_history_unchanged(&mut shell, &imported, &imported_history);

    shell.click_menu_command("menu-file", AppCommand::ExportHundeggerBtlx);
    assert!(digest_starts_like(&shell, "error-export-btlx"));
    assert!(shell.app().action_digest().contains("mesh body"));
    assert!(!btlx.exists());
    assert!(!btlx.with_extension("btlx.support.txt").exists());
    assert!(script.high_risk_prompts().is_empty());
    assert!(shell.app().last_side_effect_receipt().is_none());
    assert_state_and_history_unchanged(&mut shell, &imported, &imported_history);
}

#[test]
fn file_import_sketchup_scene_rejects_source_substitution_after_review() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reviewed.kscene");
    std::fs::write(&path, valid_sketchup_scene_bridge()).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::SketchupScene, &path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportSketchupScene);
    std::fs::write(&path, b"{}" as &[u8]).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-sketchup-scene-confirm"));

    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-import-sketchup-scene"));
}

#[test]
fn file_import_stl_commits_one_canonical_mesh_transaction_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetrahedron.stl");
    let document_path = directory.path().join("imported.ketchup");
    std::fs::write(&path, valid_ascii_tetrahedron()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);
    let before_definitions = shell.app().definition_count();

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));

    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert_eq!(shell.app().definition_count(), before_definitions + 1);
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert!(shell.app().can_undo());
    assert!(digest_starts_like(&shell, "digest-imported-stl"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Stl,
            filter_label: shell.catalog().text("file-filter-stl"),
            extensions: vec!["stl".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().mesh_body_count(), 0);
    assert_eq!(shell.app().import_receipt_count(), 0);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);

    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    assert!(!shell.app().is_dirty());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert_persisted_stl(
        &document_path,
        valid_ascii_tetrahedron(),
        ImportLengthUnit::Millimetre,
        "stl.ascii",
    );
}

#[test]
fn file_import_dxf_reviews_and_commits_one_canonical_profile_transaction_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.dxf");
    let document_path = directory.path().join("imported-dxf.ketchup");
    std::fs::write(&path, valid_dxf_subset()).unwrap();
    assert_eq!(
        inspect_dxf(valid_dxf_subset(), DxfImportOptions::new(None))
            .unwrap()
            .profiles()
            .len(),
        24
    );
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);
    let before_box_count = shell.app().active_box_count();

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(
        shell.app().document_revision() == before.revision + 1,
        "DXF import did not commit: {}",
        shell.app().action_digest()
    );
    assert!(shell.app().definition_count() >= 24);
    assert!(shell.app().feature_count() >= 24);
    assert!(shell.app().occurrence_count() >= 24);
    assert!(shell.app().active_box_count() >= before_box_count + 24);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert!(digest_starts_like(&shell, "digest-imported-dxf"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Dxf,
            filter_label: shell.catalog().text("file-filter-dxf"),
            extensions: vec!["dxf".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().definition_count(), before.definitions);
    assert_eq!(shell.app().feature_count(), before.features);
    assert_eq!(shell.app().occurrence_count(), before.occurrences);
    assert_eq!(shell.app().import_receipt_count(), before.import_receipts);
    assert!(shell.app().can_redo());
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.format(), ImportFormat::Dxf);
    assert_eq!(
        receipt.parser_version(),
        ketchup_core::import::DXF_PARSER_VERSION
    );
    assert_eq!(receipt.outputs().len(), 24 * 3);
    assert_eq!(receipt.source_sha256(), &sha256_bytes(valid_dxf_subset()));
    assert_eq!(receipt.source_byte_len(), valid_dxf_subset().len() as u64);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported" && diagnostic.subject() == Some("TEXT")
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.entity-unsupported"
            && diagnostic.subject() == Some("ATTDEF")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.block-insert"
            && diagnostic.subject() == Some("stamp")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.insert-attributes-dropped"
            && diagnostic.subject() == Some("wrapper")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.dimension-semantics-dropped"
            && diagnostic.subject() == Some("*D1")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.dimension-graphics"
            && diagnostic.subject() == Some("*D1")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-boundary-topology-dropped"
            && diagnostic.subject() == Some("hatches")
            && diagnostic.count() == 2
    }));
    for code in [
        "dxf.mpolygon-fill-dropped",
        "dxf.mpolygon-annotation-dropped",
        "dxf.mpolygon-boundary-topology-dropped",
        "dxf.mpolygon-geometry",
    ] {
        assert!(receipt.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == code && diagnostic.subject() == Some("mpolygons")
        }));
    }
    for (subject, codes) in [
        (
            "meshes",
            [
                "dxf.mesh-face-topology-dropped",
                "dxf.mesh-boundary-geometry",
            ],
        ),
        (
            "polyfaces",
            [
                "dxf.polyface-face-topology-dropped",
                "dxf.polyface-boundary-geometry",
            ],
        ),
        (
            "polygon-meshes",
            [
                "dxf.polygon-mesh-surface-topology-dropped",
                "dxf.polygon-mesh-boundary-geometry",
            ],
        ),
    ] {
        for code in codes {
            assert!(receipt.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == code && diagnostic.subject() == Some(subject)
            }));
        }
    }
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.polyface-invisible-edge-dropped"
            && diagnostic.subject() == Some("polyfaces")
            && diagnostic.count() == 1
    }));
    for code in [
        "dxf.leader-arrowhead-dropped",
        "dxf.leader-semantics-dropped",
        "dxf.leader-geometry",
    ] {
        assert!(
            receipt
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.code() == code && diagnostic.count() == 1 })
        );
    }
    assert_eq!(
        loaded_snapshot
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::SegmentProfile { .. }))
            .count(),
        24
    );
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 4
            && segments[0].start_mm() == [200.0, 0.0]
            && segments[0].end_mm() == [204.0, 0.0]
            && segments[1].end_mm() == [204.0, 3.0]
            && segments[2].end_mm() == [200.0, 3.0]
            && segments[3].end_mm() == [200.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        !*closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [170.0, 0.0],
                        end_mm: [174.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [174.0, 0.0],
                        end_mm: [174.0, 3.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        !*closed
            && matches!(
                segments.as_slice(),
                [ProfileSegment::Line {
                    start_mm: [165.0, 2.0],
                    end_mm: [169.0, 2.0]
                }]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 3
            && segments[0].start_mm() == [80.0, 0.0]
            && segments[0].end_mm() == [84.0, 0.0]
            && segments[1].end_mm() == [80.0, 3.0]
            && segments[2].end_mm() == [80.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 4
            && segments[0].start_mm() == [60.0, -10.0]
            && segments[0].end_mm() == [60.0, -14.0]
            && segments[1].end_mm() == [54.0, -14.0]
            && segments[3].end_mm() == [60.0, -10.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 4
            && segments[0].start_mm() == [90.0, 0.0]
            && segments[0].end_mm() == [94.0, 0.0]
            && segments[1].end_mm() == [94.0, 3.0]
            && segments[2].end_mm() == [90.0, 3.0]
            && segments[3].end_mm() == [90.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [100.0, 0.0],
                        end_mm: [104.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [104.0, 0.0],
                        end_mm: [100.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [100.0, 3.0],
                        end_mm: [100.0, 0.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        matches!(
            segments.as_slice(),
            [
                ProfileSegment::CircularArc {
                    start_mm: [35.0, 20.0],
                    end_mm: [25.0, 20.0],
                    center_mm: [30.0, 20.0],
                    clockwise: false,
                },
                ProfileSegment::CircularArc {
                    start_mm: [25.0, 20.0],
                    end_mm: [35.0, 20.0],
                    center_mm: [30.0, 20.0],
                    clockwise: false,
                }
            ] if *closed
        )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 4
            && segments[0].start_mm() == [40.0, 0.0]
            && segments[0].end_mm() == [50.0, 0.0]
            && segments[3].end_mm() == [40.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 3
            && segments[0].start_mm() == [60.0, 0.0]
            && segments[0].end_mm() == [60.0, -8.0]
            && segments[1].end_mm() == [48.0, 0.0]
            && segments[2].end_mm() == [60.0, 0.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && segments.len() == 3
            && segments[0].start_mm() == [60.0, 10.0]
            && segments[0].end_mm() == [60.0, 2.0]
            && segments[1].end_mm() == [48.0, 10.0]
            && segments[2].end_mm() == [60.0, 10.0]
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        matches!(
            segments.as_slice(),
            [
                ProfileSegment::CircularArc {
                    start_mm: [110.0, 7.0],
                    end_mm: [110.0, 3.0],
                    center_mm: [110.0, 5.0],
                    clockwise: false,
                },
                ProfileSegment::CircularArc {
                    start_mm: [110.0, 3.0],
                    end_mm: [110.0, 7.0],
                    center_mm: [110.0, 5.0],
                    clockwise: false,
                }
            ] if *closed
        )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        !*closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [120.0, 0.0],
                        end_mm: [124.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [124.0, 0.0],
                        end_mm: [124.0, 3.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [130.0, 0.0],
                        end_mm: [134.0, 0.0]
                    },
                    ProfileSegment::CircularArc {
                        start_mm: [134.0, 0.0],
                        end_mm: [134.0, 3.0],
                        center_mm: [134.0, 1.5],
                        clockwise: false
                    },
                    ProfileSegment::Line {
                        start_mm: [134.0, 3.0],
                        end_mm: [130.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [130.0, 3.0],
                        end_mm: [130.0, 0.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [140.0, 0.0],
                        end_mm: [144.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [144.0, 0.0],
                        end_mm: [144.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [144.0, 3.0],
                        end_mm: [140.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [140.0, 3.0],
                        end_mm: [140.0, 0.0]
                    }
                ]
            )
    }));
    assert!(loaded_snapshot.features().any(|feature| {
        let FeatureKind::SegmentProfile { segments, closed } = feature.kind() else {
            return false;
        };
        *closed
            && matches!(
                segments.as_slice(),
                [
                    ProfileSegment::Line {
                        start_mm: [150.0, 0.0],
                        end_mm: [154.0, 0.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [154.0, 0.0],
                        end_mm: [154.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [154.0, 3.0],
                        end_mm: [150.0, 3.0]
                    },
                    ProfileSegment::Line {
                        start_mm: [150.0, 3.0],
                        end_mm: [150.0, 0.0]
                    }
                ]
            )
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-fill-dropped"
            && diagnostic.subject() == Some("hatch-patterned")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.hatch-fill-dropped"
            && diagnostic.subject() == Some("hatches")
            && diagnostic.count() == 1
    }));
    assert!(receipt.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == "dxf.block-insert"
            && diagnostic.subject() == Some("wrapper")
            && diagnostic.count() == 2
    }));
}

#[test]
fn file_menu_dxf_export_is_layered_authorized_atomic_and_non_mutating_offscreen() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("profiles.dxf");
    let exported_path = directory.path().join("round-trip.dxf");
    let report_path = exported_path.with_extension("dxf.loss.txt");
    std::fs::write(&source_path, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &source_path)
        .queue_export(&exported_path)
        .queue_export(&exported_path)
        .always_confirm_high_risk_as(117);
    let mut shell = Shell::with_dialogs(script.clone());
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    let before_export = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ExportDrawingDxf);
    assert_eq!(canonical_state(&shell), before_export);
    let first_dxf = std::fs::read(&exported_path).unwrap();
    let first_report = std::fs::read_to_string(&report_path).unwrap();
    let inspected = inspect_dxf(&first_dxf, DxfImportOptions::new(None)).unwrap();
    assert!(inspected.profiles().len() >= 24);
    assert!(inspected.layers().iter().any(|layer| layer == "bores"));
    assert!(inspected.layers().iter().any(|layer| layer == "hatches"));
    assert!(first_report.contains("schema=ketchup.dxf-profile-export.v1"));
    assert!(first_report.contains("unit=millimetre"));
    assert!(first_report.contains("dwg=unsupported"));

    std::fs::write(&exported_path, b"stale primary").unwrap();
    std::fs::write(&report_path, b"stale report").unwrap();
    shell.click_menu_command("menu-file", AppCommand::ExportDrawingDxf);
    assert_eq!(canonical_state(&shell), before_export);
    assert_eq!(std::fs::read(&exported_path).unwrap(), first_dxf);
    assert_eq!(std::fs::read_to_string(&report_path).unwrap(), first_report);
    assert_eq!(
        script.export_requests(),
        vec![
            ketchup_app::dialogs::ExportRequestRecord {
                filter_label: shell.catalog().text("file-filter-dxf"),
                extension: "dxf".to_owned(),
                suggested_name: "Untitled.dxf".to_owned(),
            },
            ketchup_app::dialogs::ExportRequestRecord {
                filter_label: shell.catalog().text("file-filter-dxf"),
                extension: "dxf".to_owned(),
                suggested_name: "Untitled.dxf".to_owned(),
            },
        ]
    );
    let prompts = script.high_risk_prompts();
    assert_eq!(prompts.len(), 4);
    assert!(prompts[0].contains(&shell.catalog().text("dialog-export-dxf-title")));
    assert!(prompts[0].contains(&shell.catalog().text("dialog-export-dxf-risk")));
    assert!(prompts[2].contains(&shell.catalog().text("dialog-export-overwrite-title")));
    assert!(prompts[3].contains(&shell.catalog().text("dialog-export-overwrite-title")));
    assert!(digest_starts_like(&shell, "digest-exported-dxf"));
}

#[test]
fn file_menu_refuses_native_dwg_without_preview_authorization_or_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let renamed_source = directory.path().join("renamed.dwg");
    let destination = directory.path().join("misleading.dwg");
    std::fs::write(&renamed_source, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &renamed_source)
        .queue_export(&destination)
        .always_confirm_high_risk_as(118);
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-import-dxf"));

    shell.click_menu_command("menu-file", AppCommand::ExportDrawingDxf);
    assert_eq!(canonical_state(&shell), before);
    assert!(digest_starts_like(&shell, "error-export-dxf"));
    assert!(!destination.exists());
    assert!(!destination.with_extension("dxf.loss.txt").exists());
    assert!(script.high_risk_prompts().is_empty());
}

#[test]
fn exact_exchange_refuses_native_solidworks_and_parasolid_without_preview_or_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let source = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/self-authored-box.step"),
    )
    .unwrap();
    let solidworks_source = directory.path().join("part.sldprt");
    let parasolid_source = directory.path().join("assembly.x_t");
    let solidworks_destination = directory.path().join("export.sldasm");
    let parasolid_destination = directory.path().join("export.x_b");
    std::fs::write(&solidworks_source, &source).unwrap();
    std::fs::write(&parasolid_source, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &solidworks_source)
        .queue_import(ImportFormat::Iges, &parasolid_source)
        .queue_export(&solidworks_destination)
        .queue_export(&parasolid_destination)
        .always_confirm_high_risk_as(121);
    let mut shell = Shell::with_dialogs(script.clone());
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert!(shell.app().action_digest().contains("SolidWorks"));

    shell.click_menu_command("menu-file", AppCommand::ImportExactIges);
    assert_eq!(canonical_state(&shell), before);
    assert!(shell.app().action_digest().contains("Parasolid"));

    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before);
    assert!(shell.app().action_digest().contains("SolidWorks"));

    shell.click_menu_command("menu-file", AppCommand::ExportExactIges);
    assert_eq!(canonical_state(&shell), before);
    assert!(shell.app().action_digest().contains("Parasolid"));

    assert!(!solidworks_destination.exists());
    assert!(!parasolid_destination.exists());
    assert!(script.high_risk_prompts().is_empty());
}

#[test]
fn exact_worker_derives_step_units_from_representation_context() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let original = std::fs::read_to_string(&fixture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    for (name, source, expected) in [
        (
            "millimetre",
            original.replace(
                "END-ISO-10303-21;",
                "/* misleading SI_UNIT(.CENTI.,.METRE.) comment */\nEND-ISO-10303-21;",
            ),
            ImportLengthUnit::Millimetre,
        ),
        (
            "centimetre",
            original.replace(".MILLI.", ".CENTI."),
            ImportLengthUnit::Centimetre,
        ),
        (
            "metre",
            original.replace(".MILLI.", "$"),
            ImportLengthUnit::Metre,
        ),
    ] {
        let path = directory.path().join(format!("{name}.step"));
        std::fs::write(&path, source.as_bytes()).unwrap();
        let evidence = inspector
            .inspect_step_import_with_cancellation(
                &path,
                &ketchup_core::graph::sha256_hex(source.as_bytes()),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(evidence.source_unit, expected);
    }
}

#[test]
fn file_import_exact_step_commits_undoes_and_persists_source_blob_offscreen() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let source = std::fs::read(&source_path).unwrap();
    let source_hash = ketchup_core::graph::sha256_hex(&source);
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let source_evidence = inspector
        .inspect_step_import_with_cancellation(&source_path, &source_hash, &AtomicBool::new(false))
        .unwrap();
    let repeated_evidence = inspector
        .inspect_step_import_with_cancellation(&source_path, &source_hash, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(repeated_evidence, source_evidence);
    assert!(!source_evidence.backend.is_empty());
    assert!(!source_evidence.tolerance.is_empty());
    let directory = tempfile::tempdir().unwrap();
    let document_path = directory.path().join("imported-step.ketchup");
    let exported_path = directory.path().join("reexported-step.step");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .queue_export(&exported_path)
        .always_confirm_high_risk_as(91)
        .always_discard();
    let mut shell = Shell::with_dialogs(script.clone());
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the focused STEP workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    let imported_feature_id = imported_exact_feature_id(&shell);

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 1);
    assert_eq!(shell.app().feature_count(), before.features + 1);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 1);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    assert!(digest_starts_like(&shell, "digest-imported-step"));
    assert_eq!(
        script.import_requests(),
        vec![ketchup_app::dialogs::ImportDialogRequestRecord {
            format: ImportFormat::Step,
            filter_label: shell.catalog().text("file-filter-step"),
            extensions: vec!["step".to_owned(), "stp".to_owned()],
        }]
    );

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    assert_eq!(shell.app().import_receipt_count(), before.import_receipts);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );

    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_current_producer_ids() == [imported_feature_id]
            && shell.app().instanced_scene_triangle_count() > 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(shell.app().exact_render_body_count(), 1);
    assert!(
        shell.app().exact_render_triangle_count() > 0,
        "an imported STEP body without triangles is invisible and unpickable"
    );
    shell.settle();
    assert!(
        shell.app().instanced_scene_triangle_count() > 0,
        "an exact product that arrives after its revision must still reach the painted scene"
    );
    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let viewport = shell.viewport_rect();
    let screen = shell.app().project_to_screen(centre, viewport);
    shell.click_at(screen);
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "an imported STEP body must be pickable where it is painted"
    );
    let before_export = canonical_state(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before_export);
    assert!(
        exported_path.is_file(),
        "STEP re-export failed with digest {:?}",
        shell.app().action_digest()
    );
    assert!(
        std::fs::read_to_string(&exported_path)
            .unwrap()
            .starts_with("ISO-10303-21;")
    );
    let exported = std::fs::read(&exported_path).unwrap();
    let exported_evidence = inspector
        .inspect_step_import_with_cancellation(
            &exported_path,
            &ketchup_core::graph::sha256_hex(&exported),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(exported_evidence.source_unit, source_evidence.source_unit);
    assert_eq!(exported_evidence.solid_count, source_evidence.solid_count);
    assert!((exported_evidence.volume_mm3 - source_evidence.volume_mm3).abs() < 1.0e-6);
    for axis in 0..3 {
        assert!(
            (exported_evidence.bounds_mm[0][axis] - source_evidence.bounds_mm[0][axis]).abs()
                < 1.0e-6
        );
        assert!(
            (exported_evidence.bounds_mm[1][axis] - source_evidence.bounds_mm[1][axis]).abs()
                < 1.0e-6
        );
    }

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let snapshot = loaded.snapshot();
    let receipt = snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Step)
        .unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.source_byte_len(), source.len() as u64);
    assert_eq!(receipt.units().source_unit(), source_evidence.source_unit);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::FileDeclared
    );
    assert_eq!(receipt.parser_id(), ketchup_core::import::STEP_PARSER_ID);
    assert_eq!(
        receipt.parser_version(),
        ketchup_core::import::STEP_XDE_PARSER_VERSION
    );
    let spec = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some(spec),
            _ => None,
        })
        .unwrap();
    assert_eq!(spec.source_sha256, sha256_bytes(&source));
    assert_eq!(spec.source_byte_len, source.len() as u64);
    assert_eq!(spec.source_part_index, Some(0));
    let hash = ketchup_core::graph::sha256_hex(&source);
    assert_eq!(loaded.container_data().blobs().get(&hash), Some(&source));
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
}

#[test]
fn file_import_exact_step_preserves_a_real_nested_repeated_xde_assembly_offscreen() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/independent-xde-assembly.step");
    let source = std::fs::read(&source_path).unwrap();
    let source_hash = ketchup_core::graph::sha256_hex(&source);
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let evidence = inspector
        .inspect_step_xde_import_with_cancellation(
            &source_path,
            &source_hash,
            &AtomicBool::new(false),
        )
        .unwrap();
    let source_geometry = inspector
        .inspect_step_import_with_cancellation(&source_path, &source_hash, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(evidence.parts.len(), 2);
    assert_eq!(evidence.nodes.len(), 5);
    assert_eq!(
        evidence
            .nodes
            .iter()
            .filter(|node| node.part_index.is_some())
            .count(),
        3
    );

    let directory = tempfile::tempdir().unwrap();
    let document_path = directory.path().join("imported-xde-assembly.ketchup");
    let exported_path = directory.path().join("roundtrip-xde-assembly.step");
    let exported_iges_path = directory.path().join("flat-xde-assembly.iges");
    let assistant_query = "Inspect the imported exchange model without editing it";
    let assistant_transport = Arc::new(ScriptedAssistantTransport::new([(
        assistant_query.to_owned(),
        AssistantChatResult {
            message: "The imported exact model was inspected without mutation.".to_owned(),
            model_intent: None,
        },
    )]));
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .queue_export(&exported_path)
        .queue_export(&exported_iges_path)
        .always_confirm_high_risk_as(117)
        .always_discard();
    let mut shell =
        Shell::with_dialogs_and_assistant_transport(script, assistant_transport.clone());
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the XDE assembly workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();
    let before = canonical_state(&shell);
    let before_groups = shell.app().group_count();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 2);
    assert_eq!(shell.app().feature_count(), before.features + 2);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 3);
    assert_eq!(shell.app().group_count(), before_groups + 2);
    assert_eq!(
        shell.app().import_receipt_count(),
        before.import_receipts + 1
    );
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);

    let snapshot = shell.app().document_snapshot();
    let root = snapshot
        .groups()
        .find(|group| group.name() == "Fixture root assembly")
        .unwrap();
    let carriage = snapshot
        .groups()
        .find(|group| group.name() == "Carriage nested instance")
        .unwrap();
    assert_eq!(root.parent(), None);
    assert_eq!(carriage.parent(), Some(root.id()));
    assert_eq!(carriage.transform().matrix()[7], 50.0);
    let left = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Bracket left instance")
        .unwrap();
    let right = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Bracket right instance")
        .unwrap();
    let pin = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == "Pin root instance")
        .unwrap();
    assert_eq!(left.parent(), Some(carriage.id()));
    assert_eq!(right.parent(), Some(carriage.id()));
    assert_eq!(pin.parent(), Some(root.id()));
    assert_eq!(left.definition_id(), right.definition_id());
    assert_ne!(left.definition_id(), pin.definition_id());
    assert_eq!(left.color(), Some([255, 0, 0]));
    assert_eq!(right.color(), Some([0, 255, 0]));
    assert_eq!(pin.color(), Some([0, 0, 255]));
    assert_eq!(right.transform().matrix()[3], 40.0);
    assert_eq!(pin.transform().matrix()[3], 20.0);
    assert_eq!(pin.transform().matrix()[7], 10.0);
    assert_eq!(pin.transform().matrix()[11], 5.0);
    let mut imported_features = snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::ImportedExactBody(spec) => Some((feature.id(), spec.source_part_index)),
            _ => None,
        })
        .collect::<Vec<_>>();
    imported_features.sort_unstable();
    assert_eq!(
        imported_features
            .iter()
            .map(|(_, part_index)| *part_index)
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1)]
    );
    let mut expected_producers = imported_features
        .iter()
        .map(|(feature_id, _)| *feature_id)
        .collect::<Vec<_>>();
    expected_producers.sort_unstable();
    drop(snapshot);

    for _ in 0..150 {
        shell.settle();
        let mut producers = shell.app().exact_current_producer_ids();
        producers.sort_unstable();
        if producers == expected_producers && shell.app().instanced_scene_triangle_count() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut producers = shell.app().exact_current_producer_ids();
    producers.sort_unstable();
    assert_eq!(producers, expected_producers);
    assert_eq!(shell.app().exact_render_body_count(), 2);
    assert!(shell.app().exact_render_triangle_count() > 0);
    assert!(
        shell.app().instanced_scene_triangle_count() > shell.app().exact_render_triangle_count(),
        "the shared bracket definition must be instanced twice in the painted scene"
    );
    let screen = shell
        .app()
        .project_to_screen(Vec3::new(45.0, 60.0, 15.0), shell.viewport_rect());
    shell.click_at(screen);
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_eq!(shell.app().definition_count(), before.definitions + 2);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 3);
    assert_eq!(shell.app().group_count(), before_groups + 2);
    for _ in 0..150 {
        shell.settle();
        let mut producers = shell.app().exact_current_producer_ids();
        producers.sort_unstable();
        if producers == expected_producers && shell.app().instanced_scene_triangle_count() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut reopened_producers = shell.app().exact_current_producer_ids();
    reopened_producers.sort_unstable();
    assert_eq!(reopened_producers, expected_producers);

    let assistant_context = shell.app().assistant_context();
    assert_eq!(assistant_context["state_view"]["complete"], true);
    let assistant_state = assistant_context["state_view"]["content"].as_str().unwrap();
    assert_eq!(assistant_state.matches("source_format:step").count(), 2);
    assert!(assistant_state.contains("source_part_index:0"));
    assert!(assistant_state.contains("source_part_index:1"));
    assert_eq!(
        assistant_state
            .matches("source_parametric_history:unavailable")
            .count(),
        2
    );
    assert_eq!(
        assistant_state,
        ketchup_core::state_view::encode_semantic_state(&shell.app().document_snapshot())
            .agent_v1()
    );

    let before_assistant = canonical_state(&shell);
    let input = shell.catalog().text("assistant-input-hint");
    shell.focus_text_input(&input);
    shell.type_text(assistant_query);
    shell.press_key(Key::Enter);
    for _ in 0..300 {
        shell.settle();
        if !assistant_transport.contexts().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let provider_contexts = assistant_transport.contexts();
    assert_eq!(provider_contexts.len(), 1);
    let interoperability = &provider_contexts[0]["interoperability"];
    assert_eq!(interoperability["complete"], true);
    assert_eq!(interoperability["import_count"], 1);
    assert_eq!(interoperability["imports"][0]["format"], "step");
    assert_eq!(
        interoperability["imports"][0]["source_parametric_history"],
        "unavailable"
    );
    assert_eq!(
        interoperability["imports"][0]["canonical_editability"],
        "exact_brep_and_occurrence_metadata"
    );
    assert!(
        interoperability["imports"][0]["diagnostic_codes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "step_parametric_reconstruction_unavailable")
    );
    assert_eq!(assistant_transport.remaining_responses(), 0);
    assert_eq!(shell.app().document_revision(), before_assistant.revision);
    assert_eq!(shell.app().canonical_digest(), before_assistant.digest);
    assert_eq!(shell.app().undo_step_count(), before_assistant.undo_steps);
    assert_eq!(shell.app().redo_step_count(), before_assistant.redo_steps);

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let loaded_receipt = loaded_snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Step)
        .unwrap();
    assert_eq!(loaded_receipt.source_sha256(), &sha256_bytes(&source));
    assert!(
        loaded_receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "step_hierarchy_preserved")
    );
    assert!(
        loaded_receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "step_color_metadata_preserved")
    );
    assert!(
        loaded_receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "step_name_metadata_preserved")
    );
    assert!(
        loaded_receipt
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "step_parametric_reconstruction_unavailable")
    );
    assert_eq!(
        loaded.container_data().blobs().get(&source_hash),
        Some(&source)
    );

    let before_export = canonical_state(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before_export);
    assert!(
        exported_path.is_file(),
        "XDE assembly export failed with digest {:?}",
        shell.app().action_digest()
    );
    let exported = std::fs::read(&exported_path).unwrap();
    let exported_hash = ketchup_core::graph::sha256_hex(&exported);
    let roundtrip = inspector
        .inspect_step_xde_import_with_cancellation(
            &exported_path,
            &exported_hash,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(roundtrip.parts.len(), 2);
    assert_eq!(
        roundtrip
            .parts
            .iter()
            .map(|part| part.name.as_str())
            .collect::<Vec<_>>(),
        evidence
            .parts
            .iter()
            .map(|part| part.name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(roundtrip.nodes.len(), 6);
    assert_eq!(
        roundtrip
            .nodes
            .iter()
            .filter(|node| node.part_index.is_some())
            .count(),
        3
    );
    assert_eq!(
        roundtrip
            .nodes
            .iter()
            .filter_map(|node| node.part_index)
            .collect::<Vec<_>>(),
        vec![0, 0, 1]
    );
    let roundtrip_node = |name: &str| {
        roundtrip
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap()
    };
    let root_node = roundtrip_node("Fixture root assembly");
    let carriage_node = roundtrip_node("Carriage nested instance");
    let left_node = roundtrip_node("Bracket left instance");
    let right_node = roundtrip_node("Bracket right instance");
    let pin_node = roundtrip_node("Pin root instance");
    assert_eq!(carriage_node.parent_id, Some(root_node.id));
    assert_eq!(left_node.parent_id, Some(carriage_node.id));
    assert_eq!(right_node.parent_id, Some(carriage_node.id));
    assert_eq!(pin_node.parent_id, Some(root_node.id));
    assert_eq!(left_node.color, Some([255, 0, 0]));
    assert_eq!(right_node.color, Some([0, 255, 0]));
    assert_eq!(pin_node.color, Some([0, 0, 255]));
    assert_eq!(carriage_node.transform.matrix()[7], 50.0);
    assert_eq!(right_node.transform.matrix()[3], 40.0);
    assert_eq!(pin_node.transform.matrix()[3], 20.0);
    assert_eq!(pin_node.transform.matrix()[7], 10.0);
    assert_eq!(pin_node.transform.matrix()[11], 5.0);
    let roundtrip_geometry = inspector
        .inspect_step_import_with_cancellation(
            &exported_path,
            &exported_hash,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(roundtrip_geometry.solid_count, source_geometry.solid_count);
    assert!((roundtrip_geometry.volume_mm3 - source_geometry.volume_mm3).abs() < 1.0e-6);
    for axis in 0..3 {
        assert!(
            (roundtrip_geometry.bounds_mm[0][axis] - source_geometry.bounds_mm[0][axis]).abs()
                < 1.0e-6
        );
        assert!(
            (roundtrip_geometry.bounds_mm[1][axis] - source_geometry.bounds_mm[1][axis]).abs()
                < 1.0e-6
        );
    }
    assert!(
        std::fs::read_to_string(exported_path.with_extension("step.loss.txt"))
            .unwrap()
            .contains("repeated part definitions")
    );

    shell.click_menu_command("menu-file", AppCommand::ExportExactIges);
    assert_eq!(canonical_state(&shell), before_export);
    let exported_iges = std::fs::read(&exported_iges_path).unwrap();
    let iges_roundtrip = inspector
        .inspect_iges_xde_import_with_cancellation(
            &exported_iges_path,
            &ketchup_core::graph::sha256_hex(&exported_iges),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(iges_roundtrip.parts.len(), 3);
    assert_eq!(iges_roundtrip.nodes.len(), 3);
    assert!(
        iges_roundtrip
            .nodes
            .iter()
            .all(|node| node.transform == Transform::identity())
    );
    assert_eq!(
        iges_roundtrip
            .nodes
            .iter()
            .map(|node| (node.name.as_str(), node.color))
            .collect::<Vec<_>>(),
        vec![
            ("Bracket left instance", Some([255, 0, 0])),
            ("Bracket right instance", Some([0, 255, 0])),
            ("Pin root instance", Some([0, 0, 255])),
        ]
    );
    assert_eq!(
        iges_roundtrip
            .parts
            .iter()
            .map(|part| part.exact.solid_count)
            .sum::<u32>(),
        source_geometry.solid_count
    );
    assert!(
        (iges_roundtrip
            .parts
            .iter()
            .map(|part| part.exact.volume_mm3)
            .sum::<f64>()
            - source_geometry.volume_mm3)
            .abs()
            < 1.0e-6
    );
    let iges_loss =
        std::fs::read_to_string(exported_iges_path.with_extension("iges.loss.txt")).unwrap();
    assert!(iges_loss.contains("occurrence names, sRGB colors"));
    assert!(iges_loss.contains("hierarchy, local transforms, and repeated shared definitions"));
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
}

#[test]
fn moving_an_imported_step_body_keeps_it_painted_and_drops_it_when_the_import_is_undone() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .always_confirm_high_risk_as(103);
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the imported STEP move workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    let imported_feature_id = imported_exact_feature_id(&shell);
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_current_producer_ids() == [imported_feature_id]
            && shell.app().instanced_scene_triangle_count() > 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let painted = shell.app().instanced_scene_triangle_count();
    assert!(painted > 0);

    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let span = (bounds[1][0] - bounds[0][0]).max(bounds[1][1] - bounds[0][1]);
    let interaction_point = centre + Vec3::new(span * 0.17, span * 0.09, 0.0);
    let viewport = shell.viewport_rect();
    let interaction_screen = shell.app().project_to_screen(interaction_point, viewport);
    wait_for_hovered_pick(&mut shell, interaction_screen);
    shell.click_at(interaction_screen);
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    // A move publishes a new revision without touching the import, and the
    // isolated worker needs seconds to re-derive the body. The body must stay
    // painted in the very next frame instead of blinking out until it catches up.
    // The move goes through a real viewport drag: a drag ends its preview in the
    // same frame that commits the revision, so the scene plan is rebuilt at the
    // moment the carried-forward products are still bound to the old revision.
    let offset = Vec3::new(40.0, 0.0, 0.0);
    shell.click_command(AppCommand::Move);
    let from = shell.app().project_to_screen(interaction_point, viewport);
    let to = shell
        .app()
        .project_to_screen(interaction_point + offset, viewport);
    // Every single frame of the gesture counts, not just the settled result: the
    // frame that commits the revision paints before the next one can repair it,
    // and a shell only repaints on demand, so one blank frame stays on screen.
    let mut blank_frames = 0;
    shell.drag_observing(from, to, |app| {
        if app.instanced_scene_triangle_count() == 0 {
            blank_frames += 1;
        }
    });
    assert_eq!(
        blank_frames, 0,
        "no frame of a move may paint an empty scene"
    );
    shell.settle();
    assert_eq!(
        shell.app().exact_render_body_count(),
        1,
        "moving an imported STEP body must not invalidate it"
    );
    assert_eq!(
        shell.app().instanced_scene_triangle_count(),
        painted,
        "an imported STEP body must stay painted across a move"
    );
    let viewport = shell.viewport_rect();
    let moved = shell
        .app()
        .project_to_screen(interaction_point + offset, viewport);
    shell.click_at(moved);
    assert!(
        shell.app().hovered_selection().is_some(),
        "a moved imported STEP body must be hoverable where it is painted"
    );
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "a moved imported STEP body must be pickable where it is painted"
    );

    // Move only starts on a hovered occurrence, so a pick projection that stops
    // following the carried-forward products leaves the body painted but dead:
    // the first move works and no later one does.
    let before_second_move = shell.app().document_revision();
    shell.click_command(AppCommand::Move);
    let further = shell
        .app()
        .project_to_screen(interaction_point + offset + offset, viewport);
    shell.drag(moved, further);
    shell.settle();
    assert_eq!(
        shell.app().document_revision(),
        before_second_move + 1,
        "a second move of an imported STEP body must still commit: {:?}",
        shell.app().action_digest()
    );

    // Carrying the product forward must stay fail-closed: undoing the import
    // removes the feature it was derived from, so it must disappear at once.
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.settle();
    assert_eq!(
        shell.app().exact_render_body_count(),
        0,
        "an imported body whose feature is gone must not be carried forward"
    );
    assert_eq!(shell.app().instanced_scene_triangle_count(), 0);
}

#[test]
fn rotating_an_imported_step_body_keeps_it_painted_pickable_and_turnable_again() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .always_confirm_high_risk_as(103);
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the imported STEP rotate workflow requires the real exact worker");
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    let imported_feature_id = imported_exact_feature_id(&shell);
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_current_producer_ids() == [imported_feature_id]
            && shell.app().instanced_scene_triangle_count() > 0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let painted = shell.app().instanced_scene_triangle_count();
    assert!(painted > 0);

    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let span = (bounds[1][0] - bounds[0][0]).max(bounds[1][1] - bounds[0][1]);
    let interaction_point = centre + Vec3::new(span * 0.17, span * 0.09, 0.0);
    let viewport = shell.viewport_rect();
    let interaction_screen = shell.app().project_to_screen(interaction_point, viewport);
    wait_for_hovered_pick(&mut shell, interaction_screen);
    shell.click_at(interaction_screen);
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    // An imported body carries no canonical box, so its pivot has to come from
    // the painted exact geometry. Turning it is a pure occurrence transform:
    // the accepted product stays valid and the worker is never asked again.
    shell.click_command(AppCommand::Rotate);
    let before_steps = shell.app().undo_step_count();
    let from = shell
        .app()
        .project_to_screen(centre + Vec3::new(span * 0.4, 0.0, 0.0), viewport);
    let to = shell
        .app()
        .project_to_screen(centre + Vec3::new(0.0, span * 0.4, 0.0), viewport);
    let mut blank_frames = 0;
    shell.drag_observing(from, to, |app| {
        if app.instanced_scene_triangle_count() == 0 {
            blank_frames += 1;
        }
    });
    assert_eq!(
        blank_frames, 0,
        "no frame of a rotation may paint an empty scene"
    );
    shell.settle();
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "one Rotate drag must be exactly one undo step: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().exact_render_body_count(),
        1,
        "rotating an imported STEP body must not invalidate it"
    );
    assert_eq!(
        shell.app().instanced_scene_triangle_count(),
        painted,
        "an imported STEP body must stay painted across a rotation"
    );

    // The body turned about its own centre, so it is still under the same point
    // and a second rotation must be able to start there.
    let viewport = shell.viewport_rect();
    let interaction_screen = shell.app().project_to_screen(interaction_point, viewport);
    wait_for_hovered_pick(&mut shell, interaction_screen);
    shell.click_at(interaction_screen);
    assert!(
        shell.app().hovered_selection().is_some(),
        "a rotated imported STEP body must be hoverable where it is painted"
    );
    let before_steps = shell.app().undo_step_count();
    shell.click_command(AppCommand::Rotate);
    shell.drag(from, to);
    shell.settle();
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "a second rotation of an imported STEP body must still commit: {:?}",
        shell.app().action_digest()
    );
}

#[test]
fn file_import_transformed_multi_solid_step_round_trips_through_save_open_and_occt() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("transformed-multi-solid.step");
    let document_path = directory.path().join("transformed-multi-solid.ketchup");
    let exported_path = directory
        .path()
        .join("transformed-multi-solid-reexport.step");

    let source_script = ScriptedFileDialogs::new()
        .queue_export(&source_path)
        .always_confirm_high_risk_as(101);
    let mut source_shell = Shell::with_dialogs(source_script);
    source_shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    compose_two_shared_occurrences(&mut source_shell);
    for _ in 0..100 {
        source_shell.settle();
        if source_shell.app().exact_render_body_count() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(source_shell.app().exact_render_body_count(), 1);
    source_shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert!(
        source_path.is_file(),
        "multi-solid STEP source export failed with digest {:?}",
        source_shell.app().action_digest()
    );

    let source = std::fs::read(&source_path).unwrap();
    let mut inspector = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let source_evidence = inspector
        .inspect_step_import_with_cancellation(
            &source_path,
            &ketchup_core::graph::sha256_hex(&source),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(source_evidence.solid_count, 2);
    assert!(source_evidence.bounds_mm[1][0] > 150.0);
    assert!(source_evidence.bounds_mm[1][1] > 25.0);
    let source_xde_evidence = inspector
        .inspect_step_xde_import_with_cancellation(
            &source_path,
            &ketchup_core::graph::sha256_hex(&source),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(source_xde_evidence.parts.len(), 1);
    assert_eq!(source_xde_evidence.nodes.len(), 3);
    assert_eq!(
        source_xde_evidence
            .nodes
            .iter()
            .filter_map(|node| node.part_index)
            .collect::<Vec<_>>(),
        vec![0, 0]
    );

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .queue_export(&exported_path)
        .always_confirm_high_risk_as(102)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    shell.app_mut().enable_headless_instanced_scene();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    let mut imported_feature_ids = shell
        .app()
        .document_snapshot()
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::ImportedExactBody(_)))
        .map(ketchup_core::document::Feature::id)
        .collect::<Vec<_>>();
    imported_feature_ids.sort_unstable();
    assert_eq!(imported_feature_ids.len(), 1);
    assert_eq!(shell.app().definition_count(), before.definitions + 1);
    assert_eq!(shell.app().feature_count(), before.features + 1);
    assert_eq!(shell.app().occurrence_count(), before.occurrences + 2);
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    assert_eq!(shell.app().canonical_digest(), before.digest);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(shell.app().canonical_digest(), imported_digest);

    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    for _ in 0..100 {
        shell.settle();
        let mut producers = shell.app().exact_current_producer_ids();
        producers.sort_unstable();
        if producers == imported_feature_ids && shell.app().instanced_scene_triangle_count() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut producers = shell.app().exact_current_producer_ids();
    producers.sort_unstable();
    assert_eq!(producers, imported_feature_ids);
    assert_eq!(shell.app().exact_render_body_count(), 1);
    shell.settle();
    assert!(
        shell.app().instanced_scene_triangle_count() > 0,
        "reopened shared STEP occurrences must reach the painted scene"
    );
    // XDE preserves two occurrences of one shared exact part definition.
    let bounds = shell.app().exact_render_bounds()[0];
    let centre = Vec3::new(
        (bounds[0][0] + bounds[1][0]) * 0.5,
        (bounds[0][1] + bounds[1][1]) * 0.5,
        (bounds[0][2] + bounds[1][2]) * 0.5,
    );
    let viewport = shell.viewport_rect();
    let screen = shell.app().project_to_screen(centre, viewport);
    shell.click_at(screen);
    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "a reopened multi-solid STEP body must be pickable where it is painted"
    );

    let before_export = canonical_state(&shell);
    shell.click_menu_command("menu-file", AppCommand::ExportExactStep);
    assert_eq!(canonical_state(&shell), before_export);
    assert!(
        exported_path.is_file(),
        "multi-solid STEP re-export failed with digest {:?}",
        shell.app().action_digest()
    );
    let exported = std::fs::read(&exported_path).unwrap();
    let exported_evidence = inspector
        .inspect_step_import_with_cancellation(
            &exported_path,
            &ketchup_core::graph::sha256_hex(&exported),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(exported_evidence.source_unit, source_evidence.source_unit);
    assert_eq!(exported_evidence.solid_count, source_evidence.solid_count);
    assert!((exported_evidence.volume_mm3 - source_evidence.volume_mm3).abs() < 1.0e-6);
    for axis in 0..3 {
        assert!(
            (exported_evidence.bounds_mm[0][axis] - source_evidence.bounds_mm[0][axis]).abs()
                < 1.0e-6
        );
        assert!(
            (exported_evidence.bounds_mm[1][axis] - source_evidence.bounds_mm[1][axis]).abs()
                < 1.0e-6
        );
    }

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot
        .import_receipts()
        .find(|receipt| receipt.format() == ImportFormat::Step)
        .unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.units().source_unit(), source_evidence.source_unit);
    assert_eq!(std::fs::read(&source_path).unwrap(), source);
}

#[test]
fn file_import_step_cancel_worker_source_stale_and_oversize_refuse_without_mutation() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let original = std::fs::read(&fixture).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.step");
    let corrupt = directory.path().join("corrupt.step");
    let unsupported_unit = directory.path().join("unsupported-unit.step");
    let oversized = directory.path().join("oversized.step");
    let replacement = directory.path().join("replacement.ketchup");
    let unavailable_worker = directory.path().join("unavailable-worker");
    std::fs::write(&source, &original).unwrap();
    std::fs::write(&corrupt, b"not an ISO-10303-21 exchange file").unwrap();
    let unsupported_source = String::from_utf8(original.clone())
        .unwrap()
        .replace(".MILLI.", ".MICRO.")
        .into_bytes();
    assert_ne!(unsupported_source, original);
    std::fs::write(&unsupported_unit, unsupported_source).unwrap();
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(MAX_STEP_SOURCE_BYTES + 1)
        .unwrap();
    std::fs::write(&unavailable_worker, b"not an exact worker").unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Step)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &corrupt)
        .queue_import(ImportFormat::Step, &unsupported_unit)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &source)
        .queue_import(ImportFormat::Step, &oversized)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell
        .app_mut()
        .connect_exact_worker(&unavailable_worker)
        .unwrap();
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    let mut changed = original.clone();
    changed[0] ^= 1;
    std::fs::write(&source, &changed).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
    std::fs::write(&source, &original).unwrap();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    for _ in 0..2 {
        let before_refusal = canonical_state(&shell);
        let before_refusal_history = reachable_history_digests(&mut shell);
        shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
        assert!(digest_starts_like(&shell, "error-import-step"));
        assert_state_and_history_unchanged(&mut shell, &before_refusal, &before_refusal_history);
    }

    shell.click_at(shell.viewport_rect().center());
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_change_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(
        &mut shell,
        &after_document_change,
        &after_document_change_history,
    );

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);

    let before_oversize = canonical_state(&shell);
    let before_oversize_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    assert!(digest_starts_like(&shell, "error-import-step"));
    assert_state_and_history_unchanged(&mut shell, &before_oversize, &before_oversize_history);
    assert_eq!(std::fs::read(&source).unwrap(), original);
}

#[test]
fn file_import_unitless_dxf_requires_and_persists_explicit_user_units() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unitless.dxf");
    let document_path = directory.path().join("unitless.ketchup");
    let source = std::str::from_utf8(valid_dxf_subset())
        .unwrap()
        .replace("9\n$INSUNITS\n70\n4\n", "")
        .into_bytes();
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &path)
        .queue_save(&document_path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert_eq!(canonical_state(&shell), before);

    shell.click_role_and_label(Role::RadioButton, &shell.catalog().text("unit-centimetre"));
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    shell.click_menu_command("menu-file", AppCommand::Save);

    let loaded = ketchup_core::persistence::load_file(&document_path).unwrap();
    let loaded_snapshot = loaded.snapshot();
    let receipt = loaded_snapshot.import_receipts().next().unwrap();
    assert_eq!(receipt.source_sha256(), &sha256_bytes(&source));
    assert_eq!(receipt.units().source_unit(), ImportLengthUnit::Centimetre);
    assert_eq!(
        receipt.units().authority(),
        ImportUnitAuthority::UserDeclared
    );
}

#[test]
fn file_import_dxf_dialog_and_review_cancel_leave_canonical_state_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles.dxf");
    std::fs::write(&path, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Dxf)
        .queue_import(ImportFormat::Dxf, &path);
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    assert_state_and_history_unchanged(&mut shell, &before, &history);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_dxf_refusals_preserve_exact_state_and_reachable_history() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.dxf");
    let ambiguous = directory.path().join("ambiguous.dxf");
    let oversized = directory.path().join("oversized.dxf");
    std::fs::write(
        &malformed,
        b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0\n20\n0\n11\n1\n",
    )
    .unwrap();
    std::fs::write(
        &ambiguous,
        b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n10\n0\n20\n0\n11\n1\n21\n0\n\
0\nLINE\n10\n0\n20\n0\n11\n0\n21\n1\n\
0\nLINE\n10\n0\n20\n0\n11\n-1\n21\n0\n\
0\nENDSEC\n0\nEOF\n",
    )
    .unwrap();
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(MAX_DXF_SOURCE_BYTES + 1)
        .unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &malformed)
        .queue_import(ImportFormat::Dxf, &ambiguous)
        .queue_import(ImportFormat::Dxf, &oversized);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);
    assert!(before.redo_steps > 0);

    for _ in 0..3 {
        shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
        assert!(digest_starts_like(&shell, "error-import-dxf"));
        assert_state_and_history_unchanged(&mut shell, &before, &history);
    }
}

#[test]
fn file_import_dxf_rejects_source_document_history_and_open_staleness() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    let replacement = directory.path().join("replacement.ketchup");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_import(ImportFormat::Dxf, &source)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    let before_source_change = canonical_state(&shell);
    let before_source_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    let mut same_length_change = valid_dxf_subset().to_vec();
    same_length_change[0] = b'1';
    std::fs::write(&source, same_length_change).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before_source_change, &before_source_history);

    std::fs::write(&source, valid_dxf_subset()).unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_document_change, &after_document_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));
    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);
}

#[test]
fn file_import_dxf_undo_redo_round_trip_cannot_restore_confirmability() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &source);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    assert_eq!(canonical_state(&shell), before);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_dxf_edit_context_navigation_invalidates_review() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.dxf");
    std::fs::write(&source, valid_dxf_subset()).unwrap();
    let script = ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &source);
    let mut shell = Shell::with_dialogs(script);
    let solid = shell.top_face_centre(1);
    shell.click_at(solid);
    shell.double_click_at(solid);
    assert_eq!(shell.app().edit_context_depth(), 1);
    let before = canonical_state(&shell);
    let history = reachable_history_digests(&mut shell);

    shell.click_menu_command("menu-file", AppCommand::ImportDrawingDxf);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().edit_context_depth(), 0);
    shell.click_button_label(&shell.catalog().text("dialog-import-dxf-confirm"));

    assert!(digest_starts_like(&shell, "error-import-dxf"));
    assert_state_and_history_unchanged(&mut shell, &before, &history);
}

#[test]
fn file_import_binary_stl_commits_through_the_same_headless_workflow() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tetrahedron-binary.stl");
    let document_path = directory.path().join("binary-import.ketchup");
    let source = valid_binary_tetrahedron();
    std::fs::write(&path, &source).unwrap();
    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);
    let before = canonical_state(&shell);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));

    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().mesh_body_count(), 1);
    assert_eq!(shell.app().import_receipt_count(), 1);
    assert!(digest_starts_like(&shell, "digest-imported-stl"));

    let imported_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert_persisted_stl(
        &document_path,
        &source,
        ImportLengthUnit::Millimetre,
        "stl.binary",
    );
}

#[test]
fn file_import_stl_review_applies_and_persists_every_declared_unit() {
    let directory = tempfile::tempdir().unwrap();
    for (index, unit, label_key) in [
        (0, ImportLengthUnit::Millimetre, "unit-millimetre"),
        (1, ImportLengthUnit::Centimetre, "unit-centimetre"),
        (2, ImportLengthUnit::Metre, "unit-metre"),
        (3, ImportLengthUnit::Inch, "unit-inch"),
        (4, ImportLengthUnit::Foot, "unit-foot"),
    ] {
        let source_path = directory.path().join(format!("unit-{index}.stl"));
        let document_path = directory.path().join(format!("unit-{index}.ketchup"));
        std::fs::write(&source_path, valid_ascii_tetrahedron()).unwrap();
        let script = ScriptedFileDialogs::new()
            .queue_import(ImportFormat::Stl, &source_path)
            .queue_save(&document_path);
        let mut shell = Shell::with_dialogs(script);

        shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
        shell.click_role_and_label(Role::RadioButton, &shell.catalog().text(label_key));
        shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
        shell.click_menu_command("menu-file", AppCommand::Save);

        assert_persisted_stl(&document_path, valid_ascii_tetrahedron(), unit, "stl.ascii");
    }
}

#[test]
fn file_import_stl_cancel_and_every_refusal_leave_canonical_state_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let valid = directory.path().join("valid.stl");
    let zero_facets = directory.path().join("zero-facets.stl");
    let malformed = directory.path().join("truncated-binary.stl");
    let non_manifold = directory.path().join("open-shell.stl");
    let oversized = directory.path().join("oversized.stl");
    std::fs::write(&valid, valid_ascii_tetrahedron()).unwrap();
    let mut zero_facet_source = vec![0_u8; 80];
    zero_facet_source.extend_from_slice(&0_u32.to_le_bytes());
    std::fs::write(&zero_facets, zero_facet_source).unwrap();
    let mut truncated_binary = valid_binary_tetrahedron();
    truncated_binary.pop();
    std::fs::write(&malformed, truncated_binary).unwrap();
    let last_facet = std::str::from_utf8(valid_ascii_tetrahedron())
        .unwrap()
        .find("facet normal 1 1 1")
        .unwrap();
    let mut open_shell = valid_ascii_tetrahedron()[..last_facet].to_vec();
    open_shell.extend_from_slice(b"endsolid tetrahedron\n");
    std::fs::write(&non_manifold, open_shell).unwrap();
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(MAX_STL_SOURCE_BYTES + 1)
        .unwrap();

    let script = ScriptedFileDialogs::new()
        .queue_cancelled_import(ImportFormat::Stl)
        .queue_import(ImportFormat::Stl, &valid)
        .queue_import(ImportFormat::Stl, &zero_facets)
        .queue_import(ImportFormat::Stl, &malformed)
        .queue_import(ImportFormat::Stl, &non_manifold)
        .queue_import(ImportFormat::Stl, &oversized);
    let mut shell = Shell::with_dialogs(script);
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let before = canonical_state(&shell);
    let before_history = reachable_history_digests(&mut shell);
    assert!(before.redo_steps > 0);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-cancel"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before, &before_history);
}

#[test]
fn file_import_stl_rejects_source_document_history_and_open_staleness() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("reviewed.stl");
    let replacement = directory.path().join("replacement.ketchup");
    std::fs::write(&source, valid_ascii_tetrahedron()).unwrap();
    let mut replacement_shell =
        Shell::with_dialogs(ScriptedFileDialogs::new().queue_save(&replacement));
    replacement_shell.click_menu_command("menu-file", AppCommand::Save);

    let script = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_import(ImportFormat::Stl, &source)
        .queue_open(&replacement)
        .always_discard();
    let mut shell = Shell::with_dialogs(script);

    let before_source_change = canonical_state(&shell);
    let before_source_history = reachable_history_digests(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    let mut same_length_change = valid_ascii_tetrahedron().to_vec();
    same_length_change[0] = b'S';
    std::fs::write(&source, same_length_change).unwrap();
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &before_source_change, &before_source_history);

    std::fs::write(&source, valid_ascii_tetrahedron()).unwrap();
    compose_two_shared_occurrences(&mut shell);
    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    assert!(shell.app_mut().move_selected(Vec3::new(10.0, 0.0, 0.0)));
    let after_document_change = canonical_state(&shell);
    let after_document_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_document_change, &after_document_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-edit", AppCommand::Undo);
    let after_undo = canonical_state(&shell);
    let after_undo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_undo, &after_undo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-edit", AppCommand::Redo);
    let after_redo = canonical_state(&shell);
    let after_redo_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_redo, &after_redo_history);

    shell.click_menu_command("menu-file", AppCommand::ImportMeshStl);
    shell.click_menu_command("menu-file", AppCommand::Open);
    let after_open = canonical_state(&shell);
    let after_open_history = reachable_history_digests(&mut shell);
    shell.click_button_label(&shell.catalog().text("dialog-import-stl-confirm"));
    assert!(digest_starts_like(&shell, "error-import-stl"));
    assert_state_and_history_unchanged(&mut shell, &after_open, &after_open_history);
}
