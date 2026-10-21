//! The G6 File workflow replayed through the designed shell, offscreen.
//!
//! Every step is a real click on a real menu item of the real user interface;
//! only the operating system file dialogs are answered from a script. Outcomes
//! are read from document state and from the disk, never from painted text.

use crate::harness;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, atomic::AtomicBool};
use std::time::{Duration, Instant};

use eframe::egui::{Key, Pos2, accesskit::Role};
use harness::Shell;
use ketchup_app::AppCommand;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_assistant::sidecar::{
    AssistantCadBodyFeature, AssistantCadEditOperation, AssistantCadEditProgram,
    AssistantCadFeatureReference, AssistantCadParameterValueType, AssistantCadProgramFeatureOutput,
    AssistantCadProgramFeatureReference, AssistantCadSheetMetalBend,
    AssistantCadWeldmentJointPolicy, AssistantCadWeldmentJointPrimary, AssistantCamToolKind,
    AssistantCamWorkOffset,
};
use ketchup_interaction::Vec3;
use ketchup_model::document::{
    CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
    DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind, OccurrenceId, ProfileSegment,
    SpatialPathSegment, Transform,
};
use ketchup_model::graph::sha256_bytes;
use ketchup_model::import::{
    DxfImportOptions, ImportFormat, ImportLengthUnit, ImportUnitAuthority, MAX_DXF_SOURCE_BYTES,
    MAX_STEP_SOURCE_BYTES, MAX_STL_SOURCE_BYTES, inspect_dxf,
};
use ketchup_model::mesh_recognition::{MeshRecognition, recognize_mesh_body};
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

/// Whether the digest contains every literal part of the `key` message.
fn digest_has(shell: &Shell, key: &str) -> bool {
    let template = shell.catalog().text(key);
    let digest = shell.app().action_digest();
    template
        .split(['{', '}'])
        .step_by(2)
        .all(|literal| digest.contains(literal))
}

fn wait_for_visible_label(shell: &mut Shell, label: &str) {
    assert!(
        shell.wait_until(|shell| {
            shell.settle();
            shell.has_visible_label(label)
        }),
        "timed out waiting for visible label {label:?}"
    );
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
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn imported_exact_feature_id(shell: &Shell) -> FeatureId {
    shell
        .app()
        .document_snapshot()
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::ImportedExactBody(_)))
        .map(ketchup_model::document::Feature::id)
        .expect("the confirmed STEP import must create one imported exact body feature")
}

fn wait_for_hovered_pick(shell: &mut Shell, position: Pos2) {
    let pickable = shell.wait_until(|shell| {
        shell.move_pointer(position);
        shell.app().hovered_selection().is_some()
    });
    assert!(
        pickable,
        "the current painted exact body must become pickable: producers={:?}, triangles={}",
        shell.app().exact_current_producer_ids(),
        shell.app().instanced_scene_triangle_count()
    );
}

/// Apply one typed CAD program the way an MCP client's propose + commit does,
/// and prove it landed as one revision and one undo step.
fn apply_program(shell: &mut Shell, request: &str, program: &AssistantCadEditProgram) {
    let before = canonical_state(shell);
    if let Err(error) = shell.app_mut().headless_apply_cad_program(program) {
        panic!("{request}: the program must apply: {error}");
    }
    shell.step();
    assert_eq!(shell.app().document_revision(), before.revision + 1);
    assert_eq!(shell.app().undo_step_count(), before.undo_steps + 1);
}

fn wait_for_current_exact_body(shell: &mut Shell) {
    shell.wait_until(|shell| {
        shell.settle();
        shell.app().exact_render_body_count() == 1
    });
    assert_eq!(
        shell.app().exact_render_body_count(),
        1,
        "the real worker must publish current exact evidence"
    );
}

/// Wait until exactly `expected_producers` have current exact bodies, one rendered body
/// each with triangles, and the painted scene shows them.
fn wait_for_exact_render(shell: &mut Shell, expected_producers: &[FeatureId]) {
    let mut expected = expected_producers.to_vec();
    expected.sort_unstable();
    let rendered = shell.wait_until(|shell| {
        shell.settle();
        let mut producers = shell.app().exact_current_producer_ids();
        producers.sort_unstable();
        producers == expected
            && shell.app().exact_render_body_count() == expected.len()
            && shell.app().exact_render_triangle_count() > 0
            && shell.app().instanced_scene_triangle_count() > 0
    });
    assert!(
        rendered,
        "waited {:?} for the exact render of {expected:?}: current producers {:?}, \
         rendered bodies {}, exact triangles {}, painted triangles {}",
        harness::WAIT_DEADLINE,
        shell.app().exact_current_producer_ids(),
        shell.app().exact_render_body_count(),
        shell.app().exact_render_triangle_count(),
        shell.app().instanced_scene_triangle_count()
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
    let loaded = ketchup_model::persistence::load_file(document_path).unwrap();
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

mod cad_imports;
mod documents_and_exports;
