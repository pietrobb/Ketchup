//! Faces that close no solid import as sheets (U4c, 304dd342), tested here so the check
//! runs in every job, not only in the one that builds OCCT (R1110-TST-403).

use super::json_tests::packed;
use super::*;
use crate::document::DocumentStore;

/// A GLB of one mesh of `triangles` over the corners of a tetrahedron, in metres.
fn mesh_of(triangles: &[[u16; 3]]) -> Vec<u8> {
    let mut binary = Vec::new();
    for corner in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0_f32],
    ] {
        for value in corner {
            binary.extend(value.to_le_bytes());
        }
    }
    for index in triangles.iter().flatten() {
        binary.extend(index.to_le_bytes());
    }
    while binary.len() % 4 != 0 {
        binary.push(0);
    }
    let indices = triangles.len() * 3;
    let mut chunk = u32::try_from(binary.len()).unwrap().to_le_bytes().to_vec();
    chunk.extend(b"BIN\0");
    let length = binary.len();
    chunk.extend(binary);
    let json = serde_json::json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": length}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48},
            {"buffer": 0, "byteOffset": 48, "byteLength": indices * 2},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": FLOAT, "count": 4, "type": "VEC3"},
            {"bufferView": 1, "componentType": UNSIGNED_SHORT, "count": indices, "type": "SCALAR"},
        ],
        "meshes": [{"name": "Face", "primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "nodes": [{"name": "Face", "mesh": 0}],
        "scenes": [{"nodes": [0]}],
    });
    packed(&json, &chunk)
}

fn codes(scene: &ParsedGlbScene) -> Vec<&str> {
    scene
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code())
        .collect()
}

/// Two faces on one edge and a box without its top come in as sheets, counted in the
/// review; before 304dd342 they were left out, and a file of only them was refused as
/// having nothing to import.
#[test]
fn faces_that_close_no_solid_import_as_sheets() {
    for triangles in [
        &[[0, 1, 2], [0, 2, 3]][..],
        &[[0, 2, 1], [0, 1, 3], [0, 3, 2]][..],
    ] {
        let source = mesh_of(triangles);
        let scene = inspect_glb(&source).unwrap();
        assert_eq!(scene.mesh_primitive_count(), 1, "{triangles:?}");
        assert!(scene.primitives[0].sheet, "{triangles:?}");
        let codes = codes(&scene);
        assert!(codes.contains(&"glb_open_surfaces_imported"), "{codes:?}");
        assert!(
            !codes.contains(&"glb_non_solid_pieces_skipped"),
            "{codes:?}"
        );

        let mut document = DocumentStore::new();
        let batch = plan_glb_import(&document.current(), &source, "face.glb").unwrap();
        document.apply_batch(&batch).unwrap();
        assert_eq!(document.current().occurrences().count(), 1);
    }
}

/// A closed tetrahedron stays a solid.
#[test]
fn a_closed_mesh_stays_a_solid() {
    let scene = inspect_glb(&mesh_of(&[[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]])).unwrap();
    assert_eq!(scene.mesh_primitive_count(), 1);
    assert!(!scene.primitives[0].sheet);
    assert!(!codes(&scene).contains(&"glb_open_surfaces_imported"));
}

/// A closed tetrahedron wound any way, a face or all of them inside out, imports as a solid
/// facing out, turned by its geometry (review 2026-11-13 R1113-SKP-600: a SketchUp box
/// with a face wound against the others came as no solid).
#[test]
fn a_shell_wound_any_way_imports_as_a_solid_facing_out() {
    for triangles in [
        [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 3, 2]],
        [[0, 1, 2], [0, 3, 1], [0, 2, 3], [1, 3, 2]],
        [[0, 1, 2], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
    ] {
        let scene = inspect_glb(&mesh_of(&triangles)).unwrap();
        assert_eq!(scene.mesh_primitive_count(), 1, "{triangles:?}");
        assert!(!scene.primitives[0].sheet, "{triangles:?}");
        assert!(is_solid(&scene.primitives[0]), "{triangles:?}");
        let codes = codes(&scene);
        assert!(!codes.contains(&"glb_open_surfaces_imported"), "{codes:?}");
    }
}
