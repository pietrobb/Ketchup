//! A picture many meshes show is hashed once (review 2026-11-12 R1112-SKP-501: 1 000
//! definitions sharing a 16 MB texture hashed it per mesh, 12.7 s on the window).

use super::json_tests::packed;
use super::*;
use crate::document::DocumentStore;

/// A GLB of `meshes` textured tetrahedra, each its own mesh and node, all showing one
/// picture of `picture` bytes (PNG by its first bytes; the import does not decode it).
fn sharing_one_picture(meshes: usize, picture: usize) -> Vec<u8> {
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
    for uv in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0_f32]] {
        for value in uv {
            binary.extend(value.to_le_bytes());
        }
    }
    for index in [0_u16, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3] {
        binary.extend(index.to_le_bytes());
    }
    let image_at = binary.len();
    binary.extend(b"\x89PNG\r\n\x1a\n");
    binary.resize(image_at + picture, 7);
    while !binary.len().is_multiple_of(4) {
        binary.push(0);
    }
    let length = binary.len();
    let mut chunk = u32::try_from(length).unwrap().to_le_bytes().to_vec();
    chunk.extend(b"BIN\0");
    chunk.extend(binary);
    let primitive = serde_json::json!({
        "attributes": {"POSITION": 0, "TEXCOORD_0": 1},
        "indices": 2,
        "material": 0,
    });
    let json = serde_json::json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": length}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48},
            {"buffer": 0, "byteOffset": 48, "byteLength": 32},
            {"buffer": 0, "byteOffset": 80, "byteLength": 24},
            {"buffer": 0, "byteOffset": image_at, "byteLength": picture},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": FLOAT, "count": 4, "type": "VEC3"},
            {"bufferView": 1, "componentType": FLOAT, "count": 4, "type": "VEC2"},
            {"bufferView": 2, "componentType": UNSIGNED_SHORT, "count": 12, "type": "SCALAR"},
        ],
        "images": [{"bufferView": 3, "mimeType": "image/png"}],
        "textures": [{"source": 0}],
        "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
        "meshes": (0..meshes)
            .map(|_| serde_json::json!({"primitives": [primitive.clone()]}))
            .collect::<Vec<_>>(),
        "nodes": (0..meshes)
            .map(|mesh| serde_json::json!({"mesh": mesh, "translation": [2.0 * mesh as f64, 0.0, 0.0]}))
            .collect::<Vec<_>>(),
        "scenes": [{"nodes": (0..meshes).collect::<Vec<_>>()}],
    });
    packed(&json, &chunk)
}

#[test]
fn a_picture_many_meshes_show_is_hashed_once() {
    let picture = 1 << 20;
    let source = sharing_one_picture(40, picture);
    let document = DocumentStore::new();
    let (batch, hashed) = crate::graph::headless_count_hashed_bytes(|| {
        plan_glb_import(&document.current(), &source, "shared.glb")
    });
    batch.unwrap();
    // The source once for its receipt, the picture once for its blob, and the small rest.
    let bound = u64::try_from(source.len() + picture + (picture / 2)).unwrap();
    assert!(
        hashed < bound,
        "{hashed} bytes hashed for a {picture}-byte picture"
    );
}
