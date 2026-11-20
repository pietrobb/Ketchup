//! A component stretched with SketchUp's Scale tool imports as rigid placements of a
//! stretched definition of its own (review 2026-11-19 og SKP-800).

use super::json_tests::packed;
use super::*;
use crate::document::{DocumentStore, ScaledPlacement};
use std::f64::consts::FRAC_1_SQRT_2;

/// A GLB of one tetrahedron mesh with `corners` in metres, under the scene's `nodes`;
/// the roots are the nodes `roots`.
fn scene(corners: [[f32; 3]; 4], nodes: &Value, roots: &Value) -> Vec<u8> {
    let mut binary = Vec::new();
    for value in corners.iter().flatten() {
        binary.extend(value.to_le_bytes());
    }
    for index in [0_u16, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3] {
        binary.extend(index.to_le_bytes());
    }
    let length = binary.len();
    let mut chunk = u32::try_from(length).unwrap().to_le_bytes().to_vec();
    chunk.extend(b"BIN\0");
    chunk.extend(binary);
    let json = serde_json::json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": length}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48},
            {"buffer": 0, "byteOffset": 48, "byteLength": 24},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": FLOAT, "count": 4, "type": "VEC3"},
            {"bufferView": 1, "componentType": UNSIGNED_SHORT, "count": 12, "type": "SCALAR"},
        ],
        "meshes": [{"name": "Board", "primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "nodes": nodes,
        "scenes": [{"nodes": roots}],
    });
    packed(&json, &chunk)
}

const UNIT: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
];

/// A board group `transform` over its part, beside a plain placement of the same mesh.
fn board_beside_plain(corners: [[f32; 3]; 4], transform: &Value) -> Vec<u8> {
    let mut board = transform.clone();
    board["name"] = serde_json::json!("Stretched board");
    board["children"] = serde_json::json!([1]);
    scene(
        corners,
        &serde_json::json!([
            board,
            {"name": "Part", "mesh": 0},
            {"name": "Plain board", "mesh": 0, "translation": [0.0, 5.0, 0.0]},
        ]),
        &serde_json::json!([0, 2]),
    )
}

/// The world transform of each node of `scene`.
fn worlds(scene: &ParsedGlbScene) -> Vec<Affine3> {
    let mut worlds = vec![Affine3::IDENTITY; scene.nodes.len()];
    for node in &scene.traversal {
        let parent = scene
            .traversal
            .iter()
            .find(|index| scene.nodes[**index].children.contains(node));
        worlds[*node] = parent
            .map_or(Affine3::IDENTITY, |parent| worlds[*parent])
            .compose(scene.nodes[*node].transform.affine());
    }
    worlds
}

/// The corners of the scene's parts where they are placed, to the millimetre.
fn placed_corners(scene: &ParsedGlbScene) -> Vec<[i64; 3]> {
    let worlds = worlds(scene);
    let mut corners = Vec::new();
    for node in &scene.traversal {
        let node = *node;
        for (primitive, placement) in scene.nodes[node]
            .primitives
            .iter()
            .zip(&scene.nodes[node].placements)
        {
            let placed = worlds[node].compose(placement.affine());
            for point in &scene.primitives[*primitive].vertices_mm {
                let point = placed.transform_point(Vec3::from(*point));
                corners.push([point.x, point.y, point.z].map(|value| value.round() as i64));
            }
        }
    }
    corners.sort_unstable();
    corners
}

/// A board stretched ten times its length, turned or mirrored too, lies where it did,
/// each group and part only moved, turned or mirrored, its stretched geometry a
/// definition of its own beside the plain board's, with a note. The workshop export
/// refused the whole model for the scaled placement.
#[test]
fn a_component_stretched_with_scale_imports_rigid_with_its_own_stretched_definition() {
    let turn = [0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2];
    for (stretched, unstretched, prestretched_by) in [
        (
            serde_json::json!({"scale": [10.0, 1.0, 1.0]}),
            serde_json::json!({}),
            [10.0_f32, 1.0, 1.0],
        ),
        (
            serde_json::json!({"rotation": turn, "scale": [1.0, 3.0, 0.5], "translation": [2.0, 0.0, 0.0]}),
            serde_json::json!({"rotation": turn, "translation": [2.0, 0.0, 0.0]}),
            [1.0, 3.0, 0.5],
        ),
        (
            serde_json::json!({"scale": [-2.0, 1.0, 1.0]}),
            serde_json::json!({"scale": [-1.0, 1.0, 1.0]}),
            [2.0, 1.0, 1.0],
        ),
    ] {
        let scene = inspect_glb(&board_beside_plain(UNIT, &stretched)).unwrap();
        for node in &scene.traversal {
            let node = &scene.nodes[*node];
            assert!(node.transform.is_rigid(), "{stretched}: {}", node.name);
            assert!(
                node.placements.iter().all(|placement| placement.is_rigid()),
                "{stretched}: {}",
                node.name
            );
        }
        let worlds = worlds(&scene);
        assert!(
            worlds
                .iter()
                .all(|world| world.linear.is_orthonormal(1.0e-9))
        );

        // Where it lies is where the same board, its mesh drawn stretched, lies.
        let drawn =
            UNIT.map(|corner| std::array::from_fn(|axis| corner[axis] * prestretched_by[axis]));
        let mut expected =
            placed_corners(&inspect_glb(&board_beside_plain(drawn, &unstretched)).unwrap());
        let plain = placed_corners(
            &inspect_glb(&board_beside_plain(UNIT, &serde_json::json!({}))).unwrap(),
        );
        // The plain board of the stretched scene keeps the unit mesh.
        expected.retain(|corner| corner[2] < 4_000);
        expected.extend(plain.iter().filter(|corner| corner[2] >= 4_000));
        expected.sort_unstable();
        assert_eq!(placed_corners(&scene), expected, "{stretched}");

        assert_eq!(scene.mesh_primitive_count(), 2, "{stretched}");
        let note = scene
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == "glb_scaled_parts_baked_into_own_definitions")
            .unwrap_or_else(|| panic!("{stretched}: {:?}", scene.diagnostics()));
        assert_eq!(note.count(), 1, "{stretched}");

        let mut document = DocumentStore::new();
        let batch = plan_glb_import(
            &document.current(),
            &board_beside_plain(UNIT, &stretched),
            "board.glb",
        )
        .unwrap();
        document.apply_batch(&batch).unwrap();
        let snapshot = document.current();
        assert_eq!(snapshot.occurrences().count(), 2);
        for occurrence in snapshot.occurrences() {
            let world = snapshot
                .world_transform_for_occurrence(occurrence.id())
                .unwrap();
            assert_eq!(
                ScaledPlacement::check(occurrence.id(), world),
                Ok(()),
                "{stretched}"
            );
        }
    }
}

/// A GLB of one mesh, a strip of `triangles` triangles, placed once under each of
/// `stretches` along x.
fn stretched_strips(triangles: u32, stretches: &[f64]) -> Vec<u8> {
    let vertices = triangles + 2;
    let mut binary = Vec::new();
    for vertex in 0..vertices {
        let point = [vertex as f32 * 0.001, (vertex % 2) as f32 * 0.001, 0.0];
        for value in point {
            binary.extend(value.to_le_bytes());
        }
    }
    let positions = binary.len();
    for triangle in 0..triangles {
        let (first, second) = if triangle % 2 == 0 {
            (triangle, triangle + 1)
        } else {
            (triangle + 1, triangle)
        };
        for index in [first, second, triangle + 2] {
            binary.extend(index.to_le_bytes());
        }
    }
    let length = binary.len();
    let mut chunk = u32::try_from(length).unwrap().to_le_bytes().to_vec();
    chunk.extend(b"BIN\0");
    chunk.extend(binary);
    let nodes = stretches
        .iter()
        .enumerate()
        .map(|(index, along)| {
            serde_json::json!({
                "name": format!("Board {index}"),
                "mesh": 0,
                "scale": [along, 1.0, 1.0],
                "translation": [0.0, index as f64, 0.0],
            })
        })
        .collect::<Vec<_>>();
    let json = serde_json::json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": length}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": positions},
            {"buffer": 0, "byteOffset": positions, "byteLength": length - positions},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": FLOAT, "count": vertices, "type": "VEC3"},
            {"bufferView": 1, "componentType": UNSIGNED_INT, "count": 3 * triangles, "type": "SCALAR"},
        ],
        "meshes": [{"name": "Board", "primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "nodes": nodes,
        "scenes": [{"nodes": (0..stretches.len()).collect::<Vec<_>>()}],
    });
    packed(&json, &chunk)
}

/// R1120ac-SKP-900: many parts stretched each a little differently are refused by the
/// placed triangles before any stretched copy is made. Each (mesh, stretch) copied the
/// whole mesh first: 1 000 boards of 179 000 triangles ran the window out of memory,
/// while the same boards unstretched were refused cleanly.
#[test]
fn many_parts_stretched_differently_are_refused_before_their_copies_are_made() {
    let triangles = 60_000;
    let boards = MAX_PLACED_TRIANGLES / triangles + 1;
    let stretches = (1..=boards)
        .map(|board| 1.0 + 0.01 * board as f64)
        .collect::<Vec<_>>();
    let source = stretched_strips(u32::try_from(triangles).unwrap(), &stretches);
    assert_eq!(
        inspect_glb(&source).unwrap_err(),
        GlbImportError::TooLarge(GlbExcess {
            measure: GlbMeasure::PlacedTriangles,
            at_least: boards * triangles,
            limit: MAX_PLACED_TRIANGLES,
        })
    );
}

/// Stretched copies that alone hold more than one import stores are refused while
/// copying: the copies are made one by one only while they fit.
#[test]
fn stretched_copies_over_what_one_import_stores_are_refused() {
    let triangles = 60_000;
    let vertices = triangles + 2;
    let boards = MAX_TOTAL_VERTICES / vertices + 1;
    let stretches = (1..=boards)
        .map(|board| 1.0 + 0.01 * board as f64)
        .collect::<Vec<_>>();
    let source = stretched_strips(u32::try_from(triangles).unwrap(), &stretches);
    assert_eq!(
        inspect_glb(&source).unwrap_err(),
        GlbImportError::TooLarge(GlbExcess {
            measure: GlbMeasure::Vertices,
            at_least: boards * vertices,
            limit: MAX_TOTAL_VERTICES,
        })
    );
    // Fewer, they import, each its own definition and the unstretched mesh dropped.
    let fit = stretched_strips(u32::try_from(triangles).unwrap(), &stretches[..boards - 1]);
    assert_eq!(
        inspect_glb(&fit).unwrap().mesh_primitive_count(),
        boards - 1
    );
}

/// Two parts stretched alike share their stretched definition; a scene without a
/// stretch keeps one definition and notes nothing.
#[test]
fn parts_stretched_alike_share_one_definition_and_an_unstretched_scene_notes_nothing() {
    let stretch = serde_json::json!([10.0, 1.0, 1.0]);
    let twice = scene(
        UNIT,
        &serde_json::json!([
            {"name": "A", "mesh": 0, "scale": stretch},
            {"name": "B", "mesh": 0, "scale": stretch, "translation": [0.0, 5.0, 0.0]},
        ]),
        &serde_json::json!([0, 1]),
    );
    let scene_twice = inspect_glb(&twice).unwrap();
    assert_eq!(scene_twice.mesh_primitive_count(), 1);
    let note = scene_twice
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == "glb_scaled_parts_baked_into_own_definitions")
        .unwrap();
    assert_eq!(note.count(), 2);

    let plain = inspect_glb(&board_beside_plain(UNIT, &serde_json::json!({}))).unwrap();
    assert_eq!(plain.mesh_primitive_count(), 1);
    assert!(
        plain
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code() != "glb_scaled_parts_baked_into_own_definitions")
    );
}
