use super::*;
use ketchup_geometry::linalg::Mat3;
use openskp::create::{
    DefinitionOptions, FaceOptions, GuideOptions, ImageOptions, InstanceOptions, LayerOptions,
    Point3, create,
};
use serde_json::Value;

const LIMIT: usize = 64 << 20;
const METRES_PER_INCH: f64 = 0.0254;

/// The faces of a 10 × 20 × 30 inch box, each wound counter-clockwise seen from outside.
const BOX_FACES: [[Point3; 4]; 6] = [
    [[0., 0., 0.], [0., 20., 0.], [10., 20., 0.], [10., 0., 0.]],
    [
        [0., 0., 30.],
        [10., 0., 30.],
        [10., 20., 30.],
        [0., 20., 30.],
    ],
    [[0., 0., 0.], [10., 0., 0.], [10., 0., 30.], [0., 0., 30.]],
    [
        [0., 20., 0.],
        [0., 20., 30.],
        [10., 20., 30.],
        [10., 20., 0.],
    ],
    [[0., 0., 0.], [0., 0., 30.], [0., 20., 30.], [0., 20., 0.]],
    [
        [10., 0., 0.],
        [10., 20., 0.],
        [10., 20., 30.],
        [10., 0., 30.],
    ],
];

/// A classic-format (SketchUp 2017) file: a red box component placed twice, 100 inches
/// apart, both on the tag "Walls", with a dimension and a guide point beside them.
fn two_boxes() -> Vec<u8> {
    let mut builder = create();
    let red = builder.add_material("Red", [200, 30, 30], None).unwrap();
    let walls = builder
        .add_layer("Walls", &LayerOptions::default())
        .unwrap();
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for face in &BOX_FACES {
        let options = FaceOptions {
            material: Some(red),
            ..FaceOptions::default()
        };
        definition.add_face(face, &options).unwrap();
    }
    let definition = definition.close().unwrap();
    for (name, x) in [("left", 0.0), ("right", 100.0)] {
        let options = InstanceOptions {
            name: Some(name.to_owned()),
            translation: [x, 0.0, 0.0],
            layer: Some(walls),
            ..InstanceOptions::default()
        };
        builder.add_instance(definition, &options).unwrap();
    }
    builder
        .add_dimension([0., 0., 0.], [10., 0., 0.], 5.0)
        .unwrap();
    builder
        .add_construction_point([0., 0., 50.], &GuideOptions::default())
        .unwrap();
    builder.to_bytes().unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// The JSON chunk of a GLB.
fn gltf(glb: &[u8]) -> Value {
    assert_eq!(&glb[..4], b"glTF");
    let length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    serde_json::from_slice(&glb[20..20 + length]).unwrap()
}

/// The translation (metres, glTF axes) of every node that places a mesh, in node order.
fn mesh_placements(document: &Value) -> Vec<[f64; 3]> {
    let nodes = document["nodes"].as_array().unwrap();
    let mut parent = vec![None; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        for child in node["children"].as_array().into_iter().flatten() {
            parent[child.as_u64().unwrap() as usize] = Some(index);
        }
    }
    let translation = |node: &Value| -> [f64; 3] {
        node["matrix"].as_array().map_or([0.0; 3], |matrix| {
            [12, 13, 14].map(|index| matrix[index].as_f64().unwrap())
        })
    };
    (0..nodes.len())
        .filter(|index| nodes[*index].get("mesh").is_some())
        .map(|mut index| {
            // The placements here have no rotation or scale, so translations add up.
            let mut sum = translation(&nodes[index]);
            while let Some(up) = parent[index] {
                let offset = translation(&nodes[up]);
                sum = [0, 1, 2].map(|axis| sum[axis] + offset[axis]);
                index = up;
            }
            sum
        })
        .collect()
}

/// The extent (max − min) of the positions of every mesh primitive, in metres.
fn mesh_extents(document: &Value) -> Vec<[f64; 3]> {
    let accessors = document["accessors"].as_array().unwrap();
    document["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
        .map(|primitive| {
            let accessor =
                &accessors[primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
            [0, 1, 2].map(|axis| {
                accessor["max"][axis].as_f64().unwrap() - accessor["min"][axis].as_f64().unwrap()
            })
        })
        .collect()
}

fn close(left: [f64; 3], right: [f64; 3]) -> bool {
    (0..3).all(|axis| (left[axis] - right[axis]).abs() < 1e-6)
}

#[test]
fn a_component_placed_twice_keeps_its_size_and_both_placements_in_metres() {
    let model = skp_to_glb(&two_boxes(), LIMIT).unwrap();
    assert!(model.version.starts_with("{17."), "{}", model.version);
    assert!(model.verified);
    let document = gltf(&model.glb);
    // One mesh written once, placed twice: SketchUp z is glTF y, SketchUp y is glTF −z.
    let extents = mesh_extents(&document);
    assert!(!extents.is_empty());
    for extent in &extents {
        assert!(close(
            *extent,
            [10.0, 30.0, 20.0].map(|inches| inches * METRES_PER_INCH)
        ));
    }
    let mut placements = mesh_placements(&document);
    placements.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(placements.len(), 2, "{placements:?}");
    assert!(close(placements[0], [0.0; 3]));
    assert!(close(placements[1], [100.0 * METRES_PER_INCH, 0.0, 0.0]));
    // The material's colour travels as the mesh's base colour.
    let colour = &document["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"];
    assert!(colour.as_array().is_some_and(|rgba| rgba.len() == 4));
}

#[test]
fn what_a_glb_cannot_carry_is_counted_not_dropped_silently() {
    let model = skp_to_glb(&two_boxes(), LIMIT).unwrap();
    let count = |code: &str| {
        model
            .left_out
            .iter()
            .find(|left_out| left_out.code == code)
            .map(|left_out| left_out.count)
    };
    assert_eq!(count("skp_face_tags_dropped"), None, "{:?}", model.left_out);
    assert_eq!(count("skp_dimensions_and_text_dropped"), Some(1));
    assert_eq!(count("skp_guides_dropped"), Some(1));
    assert_eq!(count("skp_textures_dropped"), None);
}

/// A 2 × 2 PNG: red and white squares.
fn checker_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[200, 30, 30, 255, 255, 255, 255, 255, 255, 200, 30, 30])
        .unwrap();
    writer.finish().unwrap();
    bytes
}

/// A classic-format box component whose faces are textured with [`checker_png`], one tile
/// covering 5 × 5 inches.
fn textured_box() -> Vec<u8> {
    let folder = tempfile::tempdir().unwrap();
    let image = folder.path().join("checker.png");
    std::fs::write(&image, checker_png()).unwrap();
    let mut builder = create();
    let checker = builder
        .add_texture_material("Checker", &image, Some(5.0), Some(5.0), None)
        .unwrap();
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for face in &BOX_FACES {
        let options = FaceOptions {
            material: Some(checker),
            ..FaceOptions::default()
        };
        definition.add_face(face, &options).unwrap();
    }
    let definition = definition.close().unwrap();
    builder
        .add_instance(definition, &InstanceOptions::default())
        .unwrap();
    builder.to_bytes().unwrap()
}

/// A texture SketchUp colours comes as its picture only, and that is counted (review
/// 2026-11-13 r95 P3: a tinted texture came untinted and nothing said so). The writer
/// makes no coloured texture, so the read file's material is marked as one.
#[test]
fn a_tinted_texture_is_counted_as_its_tint_left_out() {
    let mut model = openskp::parse_skp(&textured_box(), &ParseOptions::default()).unwrap();
    assert!(
        left_out(&model)
            .iter()
            .all(|left_out| left_out.code != "skp_texture_tints_dropped")
    );
    for material in &mut model.materials {
        material.colorized = true;
    }
    assert!(
        left_out(&model).contains(&LeftOut {
            code: "skp_texture_tints_dropped".into(),
            count: 1,
        }),
        "{:?}",
        left_out(&model)
    );
}

/// Asserts that `glb` holds a PNG base-colour texture and that every mesh says where it lies.
fn assert_textured(glb: &[u8]) {
    let document = gltf(glb);
    let materials = &document["materials"];
    let texture = materials
        .as_array()
        .unwrap()
        .iter()
        .find_map(|material| material["pbrMetallicRoughness"]["baseColorTexture"]["index"].as_u64())
        .unwrap_or_else(|| {
            panic!(
                "no base colour texture: {materials} {}",
                document["textures"]
            )
        });
    let image = &document["textures"][texture as usize]["source"];
    let image = &document["images"][image.as_u64().unwrap() as usize];
    assert_eq!(image["mimeType"], "image/png", "{image}");
    assert!(image["bufferView"].is_u64(), "{image}");
    for mesh in document["meshes"].as_array().unwrap() {
        for primitive in mesh["primitives"].as_array().unwrap() {
            assert!(
                primitive["attributes"]["TEXCOORD_0"].is_u64(),
                "{primitive}"
            );
        }
    }
}

#[test]
fn a_texture_comes_with_the_glb_with_its_image_and_where_it_lies_on_each_face() {
    let model = skp_to_glb(&textured_box(), LIMIT).unwrap();
    assert!(
        model
            .left_out
            .iter()
            .all(|left_out| left_out.code != "skp_textures_dropped"),
        "{:?}",
        model.left_out
    );
    assert_textured(&model.glb);
}

/// Two pictures of the same size in bytes, as PNG files of the same pixel size usually
/// are, stay two pictures: before, only their length and first 16 bytes told them apart,
/// and those are the same in every such PNG.
#[test]
fn two_different_pictures_of_the_same_file_size_stay_two() {
    let checker = checker_png();
    let mut inverse = Vec::new();
    let mut encoder = png::Encoder::new(&mut inverse, 2, 2);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[255, 255, 255, 30, 30, 200, 30, 30, 200, 255, 255, 255])
        .unwrap();
    writer.finish().unwrap();
    assert_eq!(checker.len(), inverse.len());
    assert_ne!(checker, inverse);

    let folder = tempfile::tempdir().unwrap();
    let mut builder = create();
    let mut materials = Vec::new();
    for (name, bytes) in [("Red", &checker), ("Blue", &inverse)] {
        let image = folder.path().join(format!("{name}.png"));
        std::fs::write(&image, bytes).unwrap();
        materials.push(
            builder
                .add_texture_material(name, &image, Some(5.0), Some(5.0), None)
                .unwrap(),
        );
    }
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for (index, face) in BOX_FACES.iter().enumerate() {
        let options = FaceOptions {
            material: Some(materials[index % 2]),
            ..FaceOptions::default()
        };
        definition.add_face(face, &options).unwrap();
    }
    let definition = definition.close().unwrap();
    builder
        .add_instance(definition, &InstanceOptions::default())
        .unwrap();
    let model = skp_to_glb(&builder.to_bytes().unwrap(), LIMIT).unwrap();
    let images = gltf(&model.glb)["images"].as_array().unwrap().len();
    assert_eq!(images, 2);
}

#[test]
fn a_placed_image_comes_as_a_textured_rectangle_not_as_left_out() {
    let folder = tempfile::tempdir().unwrap();
    let image = folder.path().join("photo.png");
    std::fs::write(&image, checker_png()).unwrap();
    let mut builder = create();
    builder
        .add_image(&image, 40.0, 30.0, &ImageOptions::default())
        .unwrap();
    let model = skp_to_glb(&builder.to_bytes().unwrap(), LIMIT).unwrap();
    assert!(model.left_out.is_empty(), "{:?}", model.left_out);
    assert_textured(&model.glb);
    let extents = mesh_extents(&gltf(&model.glb));
    // 40 × 30 inches in the SketchUp XY plane: glTF x and −z; the front and back side of the
    // face may each be a primitive.
    assert!(!extents.is_empty());
    for extent in &extents {
        assert!(
            close(
                *extent,
                [40.0 * METRES_PER_INCH, 0.0, 30.0 * METRES_PER_INCH]
            ),
            "{extents:?}"
        );
    }
}

/// The floats of accessor `index` of a GLB with one buffer, its BIN chunk.
fn accessor_floats(glb: &[u8], document: &Value, index: usize) -> Vec<f32> {
    let json_length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let bin = &glb[20 + json_length + 8..];
    let accessor = &document["accessors"][index];
    let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize
        + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
    let components = match accessor["type"].as_str().unwrap() {
        "VEC2" => 2,
        "VEC3" => 3,
        other => panic!("{other}"),
    };
    let count = accessor["count"].as_u64().unwrap() as usize * components;
    bin[start..start + count * 4]
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect()
}

/// glTF texture coordinates run down the picture from its top row, SketchUp's up from its
/// bottom row: a picture placed in SketchUp shows its top row at its far edge, so the GLB
/// gives that edge `v` 0. Before, OpenSKP's SketchUp `v` came as is and every picture was
/// upside down.
#[test]
fn a_picture_comes_the_right_way_up() {
    // Four colours: red top left, green top right, blue bottom left, white bottom right.
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, 2, 2);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[200, 30, 30, 30, 200, 30, 30, 30, 200, 255, 255, 255])
        .unwrap();
    writer.finish().unwrap();
    let folder = tempfile::tempdir().unwrap();
    let image = folder.path().join("two_rows.png");
    std::fs::write(&image, png).unwrap();
    let mut builder = create();
    builder
        .add_image(&image, 40.0, 30.0, &ImageOptions::default())
        .unwrap();
    let model = skp_to_glb(&builder.to_bytes().unwrap(), LIMIT).unwrap();
    let document = gltf(&model.glb);
    // The picture's side; its back side is a plain colour.
    let textured = |primitive: &Value| {
        let material = &document["materials"][primitive["material"].as_u64().unwrap() as usize];
        material["pbrMetallicRoughness"]["baseColorTexture"].is_object()
    };
    let primitives = document["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|mesh| mesh["primitives"].as_array().unwrap().clone())
        .filter(|primitive| textured(primitive))
        .collect::<Vec<_>>();
    assert!(!primitives.is_empty());
    for primitive in primitives {
        let attribute = |name: &str| primitive["attributes"][name].as_u64().unwrap() as usize;
        let positions = accessor_floats(&model.glb, &document, attribute("POSITION"));
        let uvs = accessor_floats(&model.glb, &document, attribute("TEXCOORD_0"));
        // SketchUp's far edge (+Y) is glTF's −Z; seen from above, the picture's left column
        // is at SketchUp's smaller X, glTF's too.
        for (position, uv) in positions.chunks_exact(3).zip(uvs.chunks_exact(2)) {
            let far = position[2] < -15.0 * METRES_PER_INCH as f32;
            let expected = if far { 0.0 } else { 1.0 };
            assert!((uv[1] - expected).abs() < 1e-4, "{position:?} {uv:?}");
            let right = position[0] > 20.0 * METRES_PER_INCH as f32;
            let expected = if right { 1.0 } else { 0.0 };
            assert!((uv[0] - expected).abs() < 1e-4, "{position:?} {uv:?}");
        }
    }
}

/// A [`checker_png`] textured box, one tile per 5 inches, drawn `offset` inches from the origin
/// twice: in a component placed at the origin and straight into the model.
fn far_boxes(offset: Point3) -> Vec<u8> {
    let folder = tempfile::tempdir().unwrap();
    let image = folder.path().join("checker.png");
    std::fs::write(&image, checker_png()).unwrap();
    let mut builder = create();
    let checker = builder
        .add_texture_material("Checker", &image, Some(5.0), Some(5.0), None)
        .unwrap();
    let options = FaceOptions {
        material: Some(checker),
        ..FaceOptions::default()
    };
    let moved = |face: &[Point3; 4]| face.map(|point| [0, 1, 2].map(|k| point[k] + offset[k]));
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for face in &BOX_FACES {
        definition.add_face(&moved(face), &options).unwrap();
    }
    let definition = definition.close().unwrap();
    builder
        .add_instance(definition, &InstanceOptions::default())
        .unwrap();
    for face in &BOX_FACES {
        builder.add_face(&moved(face), &options).unwrap();
    }
    builder.to_bytes().unwrap()
}

/// R1112-SKP-502: positions and texture coordinates are `f32`, written in the frame of their
/// definition. A 254 × 508 mm box 10 km away came as 253.906 × 507.81 mm, 3 000 km away as
/// 250 × 500 mm, and at a UTM northing (~5.4e6 m) positions moved in steps of 0.5 m.
#[test]
fn a_box_far_from_the_origin_keeps_its_size_place_and_picture() {
    // 10 km and 3 000 km east, 5.4e6 m north: whole multiples of the box and of a tile.
    for offset in [
        [393_700.0, 0.0, 0.0],
        [118_110_000.0, 0.0, 0.0],
        [0.0, 212_598_000.0, 0.0],
    ] {
        let model = skp_to_glb(&far_boxes(offset), LIMIT).unwrap();
        let document = gltf(&model.glb);
        let size = [10.0, 30.0, 20.0].map(|inches| inches * METRES_PER_INCH);
        let extents = mesh_extents(&document);
        assert!(!extents.is_empty());
        for extent in &extents {
            assert!(close(*extent, size), "{offset:?}: {extent:?}");
        }
        // Where each box lies: its node's place plus its mesh's own least corner.
        let accessors = &document["accessors"];
        let least: Vec<[f64; 3]> = document["meshes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|mesh| {
                let position = mesh["primitives"][0]["attributes"]["POSITION"].as_u64();
                let min = &accessors[position.unwrap() as usize]["min"];
                [0, 1, 2].map(|axis| min[axis].as_f64().unwrap())
            })
            .collect();
        let placed = mesh_placements(&document);
        assert_eq!(placed.len(), 2, "{offset:?}: {placed:?}");
        let corner =
            [offset[0], offset[2], -offset[1] - 20.0].map(|inches| inches * METRES_PER_INCH);
        let meshes = document["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|node| node["mesh"].as_u64());
        for (place, mesh) in placed.iter().zip(meshes) {
            let at = [0, 1, 2].map(|axis| place[axis] + least[mesh as usize][axis]);
            assert!(close(at, corner), "{offset:?}: {at:?} not {corner:?}");
        }
        // Every corner lies on a tile's corner: the coordinates stay whole and small.
        for mesh in document["meshes"].as_array().unwrap() {
            for primitive in mesh["primitives"].as_array().unwrap() {
                let uvs = primitive["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize;
                for uv in accessor_floats(&model.glb, &document, uvs) {
                    assert!(uv.abs() <= 8.0, "{offset:?}: {uv}");
                    assert!((uv - uv.round()).abs() < 1e-4, "{offset:?}: {uv}");
                }
            }
        }
    }
}

/// A box component placed twice on the tag "Walls" (hidden in the file when `walls_hidden`),
/// one of its faces on the tag "Roof".
fn tagged_boxes(walls_hidden: bool) -> Vec<u8> {
    let mut builder = create();
    let walls = builder
        .add_layer(
            "Walls",
            &LayerOptions {
                hidden: walls_hidden,
                ..LayerOptions::default()
            },
        )
        .unwrap();
    let roof = builder.add_layer("Roof", &LayerOptions::default()).unwrap();
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for (index, face) in BOX_FACES.iter().enumerate() {
        let options = FaceOptions {
            layer: (index == 1).then_some(roof),
            ..FaceOptions::default()
        };
        definition.add_face(face, &options).unwrap();
    }
    let definition = definition.close().unwrap();
    for x in [0.0, 100.0] {
        let options = InstanceOptions {
            translation: [x, 0.0, 0.0],
            layer: Some(walls),
            ..InstanceOptions::default()
        };
        builder.add_instance(definition, &options).unwrap();
    }
    builder.to_bytes().unwrap()
}

/// The tags of the components and groups come with the GLB, by node, with the file's own
/// visibility; a tag on a face inside a component is counted as left out. Before, every tag
/// was flattened.
#[test]
fn the_tags_of_components_and_groups_come_with_the_glb_by_node() {
    for hidden in [false, true] {
        let model = skp_to_glb(&tagged_boxes(hidden), LIMIT).unwrap();
        assert_eq!(
            model.tags,
            [SkpTag {
                name: "Walls".into(),
                visible: !hidden,
            }],
            "only a tag some component or group carries comes"
        );
        let document = gltf(&model.glb);
        let nodes = document["nodes"].as_array().unwrap();
        assert_eq!(model.node_tags.len(), nodes.len());
        for (node, tag) in nodes.iter().zip(&model.node_tags) {
            assert_eq!(
                *tag,
                node.get("mesh").map(|_| 0),
                "each placed box is on Walls, the model's root on no tag"
            );
        }
        let face_tags = model
            .left_out
            .iter()
            .find(|left_out| left_out.code == "skp_face_tags_dropped")
            .map(|left_out| left_out.count);
        assert_eq!(face_tags, Some(1), "{:?}", model.left_out);
    }
}

/// A box component placed twice, at x = 0 shown and at x = 100 hidden (SketchUp's Hide).
fn hidden_boxes() -> Vec<u8> {
    let mut builder = create();
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for face in &BOX_FACES {
        definition.add_face(face, &FaceOptions::default()).unwrap();
    }
    let definition = definition.close().unwrap();
    for (x, hidden) in [(0.0, false), (100.0, true)] {
        let options = InstanceOptions {
            translation: [x, 0.0, 0.0],
            hidden,
            ..InstanceOptions::default()
        };
        builder.add_instance(definition, &options).unwrap();
    }
    builder.to_bytes().unwrap()
}

/// A component or group hidden in SketchUp (Edit > Hide) comes hidden with the GLB, by
/// node (review 2026-11-10 SKP-402). Before, `Instance.hidden` was dropped and the hidden
/// placement imported visible.
#[test]
fn a_hidden_placement_comes_hidden_by_node() {
    let source = hidden_boxes();
    let model = skp_to_glb(&source, LIMIT).unwrap();
    let document = gltf(&model.glb);
    let nodes = document["nodes"].as_array().unwrap();
    assert_eq!(model.node_hidden.len(), nodes.len());
    let placed = nodes
        .iter()
        .zip(&model.node_hidden)
        .filter(|(node, _)| node.get("mesh").is_some())
        // An omitted matrix is the identity; a column-major matrix's x offset is its 13th.
        .map(|(node, hidden)| (node["matrix"][12].as_f64().unwrap_or(0.0) > 0.0, *hidden))
        .collect::<Vec<_>>();
    assert_eq!(placed.len(), 2, "{nodes:?}");
    for (far, hidden) in placed {
        assert_eq!(
            hidden, far,
            "only the box placed hidden at x = 100 is hidden"
        );
    }
    assert!(!model.node_hidden[0], "the model's root is shown");
}

/// Geometry that closes no solid: a component "Panel" of one face, and a component "Tray",
/// the box without its top, 50 inches beside it.
fn open_pieces() -> Vec<u8> {
    let mut builder = create();
    let mut panel = builder
        .add_component_definition("Panel", &DefinitionOptions::default())
        .unwrap();
    panel
        .add_face(&BOX_FACES[0], &FaceOptions::default())
        .unwrap();
    let panel = panel.close().unwrap();
    let mut tray = builder
        .add_component_definition("Tray", &DefinitionOptions::default())
        .unwrap();
    for (index, face) in BOX_FACES.iter().enumerate() {
        if index != 1 {
            tray.add_face(face, &FaceOptions::default()).unwrap();
        }
    }
    let tray = tray.close().unwrap();
    builder
        .add_instance(panel, &InstanceOptions::default())
        .unwrap();
    let beside = InstanceOptions {
        translation: [50.0, 0.0, 0.0],
        ..InstanceOptions::default()
    };
    builder.add_instance(tray, &beside).unwrap();
    builder.to_bytes().unwrap()
}

/// Faces that close no solid are geometry like any other: both open pieces come with the
/// GLB, placed, and nothing is left out.
#[test]
fn faces_that_close_no_solid_come_with_the_glb() {
    let model = skp_to_glb(&open_pieces(), LIMIT).unwrap();
    assert!(model.left_out.is_empty(), "{:?}", model.left_out);
    let mut placements = mesh_placements(&gltf(&model.glb));
    placements.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(placements.len(), 2, "{placements:?}");
    assert!(close(placements[1], [50.0 * METRES_PER_INCH, 0.0, 0.0]));
}

/// A box component painted on its outside only, as SketchUp's paint bucket leaves it: red
/// and blue fronts in turn, every back unpainted, and one face, the bottom, also painted
/// green on its back.
fn outside_painted_box() -> Vec<u8> {
    let mut builder = create();
    let red = builder.add_material("Red", [200, 30, 30], None).unwrap();
    let blue = builder.add_material("Blue", [30, 30, 200], None).unwrap();
    let green = builder.add_material("Green", [30, 200, 30], None).unwrap();
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for (index, face) in BOX_FACES.iter().enumerate() {
        let options = FaceOptions {
            material: Some(if index % 2 == 0 { red } else { blue }),
            back_material: (index == 0).then_some(green),
            ..FaceOptions::default()
        };
        definition.add_face(face, &options).unwrap();
    }
    let definition = definition.close().unwrap();
    builder
        .add_instance(definition, &InstanceOptions::default())
        .unwrap();
    builder.to_bytes().unwrap()
}

/// The triangles of every mesh primitive, by mesh.
fn triangles_by_mesh(document: &Value) -> Vec<u64> {
    let accessors = document["accessors"].as_array().unwrap();
    document["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mesh| {
            mesh["primitives"]
                .as_array()
                .unwrap()
                .iter()
                .map(|primitive| {
                    accessors[primitive["indices"].as_u64().unwrap() as usize]["count"]
                        .as_u64()
                        .unwrap()
                        / 3
                })
                .sum()
        })
        .collect()
}

/// A box painted on its outside only comes as one surface, each face once, wound and painted
/// as its front: its 6 faces are 12 triangles, and each colour is drawn from both sides.
/// Before (review 2026-11-10 SKP-400), every unpainted back came as a second, inside-out
/// copy, which the GLB import took as a sheet beside the solid, or, with two colours, as
/// sheets only, losing the solid. A back painted unlike its front is counted.
#[test]
fn a_box_painted_on_its_outside_only_comes_as_one_surface() {
    let model = skp_to_glb(&outside_painted_box(), LIMIT).unwrap();
    let document = gltf(&model.glb);
    assert_eq!(triangles_by_mesh(&document), [12], "{}", document["meshes"]);
    for material in document["materials"].as_array().unwrap() {
        assert_eq!(material["doubleSided"], true, "{material}");
    }
    assert_eq!(
        model.left_out,
        [LeftOut {
            code: "skp_back_materials_dropped".into(),
            count: 1,
        }]
    );
}

/// A face unpainted on its front and painted on its back, as a face SketchUp shows reversed,
/// keeps the back's colour, written linear (R1113-SKP-601).
#[test]
fn a_face_painted_on_its_back_only_keeps_that_colour() {
    let mut builder = create();
    let green = builder.add_material("Green", [30, 200, 30], None).unwrap();
    let mut definition = builder
        .add_component_definition("Panel", &DefinitionOptions::default())
        .unwrap();
    let options = FaceOptions {
        back_material: Some(green),
        ..FaceOptions::default()
    };
    definition.add_face(&BOX_FACES[0], &options).unwrap();
    let definition = definition.close().unwrap();
    builder
        .add_instance(definition, &InstanceOptions::default())
        .unwrap();
    let model = skp_to_glb(&builder.to_bytes().unwrap(), LIMIT).unwrap();
    assert!(model.left_out.is_empty(), "{:?}", model.left_out);
    let document = gltf(&model.glb);
    assert_eq!(triangles_by_mesh(&document), [2]);
    let colours: Vec<_> = document["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|material| material["pbrMetallicRoughness"]["baseColorFactor"].clone())
        .collect();
    let green =
        [30.0, 200.0, 30.0].map(|channel: f64| ((channel / 255.0 + 0.055) / 1.055).powf(2.4));
    assert_eq!(colours.len(), 1, "{colours:?}");
    for (channel, expected) in colours[0].as_array().unwrap().iter().zip(green) {
        assert!(
            (channel.as_f64().unwrap() - expected).abs() < 1e-6,
            "{colours:?}"
        );
    }
}

/// The signed volume of every mesh primitive of a GLB, in cubic metres: positive for a
/// solid that faces out.
fn signed_volume(glb: &[u8]) -> f64 {
    let document = gltf(glb);
    let json_length = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let bin = &glb[20 + json_length + 8..];
    let mut volume = 0.0;
    for mesh in document["meshes"].as_array().unwrap() {
        for primitive in mesh["primitives"].as_array().unwrap() {
            let positions = accessor_floats(
                glb,
                &document,
                primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
            );
            let accessor = &document["accessors"][primitive["indices"].as_u64().unwrap() as usize];
            let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let start = view["byteOffset"].as_u64().unwrap_or(0) as usize
                + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
            let count = accessor["count"].as_u64().unwrap() as usize;
            let indices: Vec<usize> = match accessor["componentType"].as_u64().unwrap() {
                5123 => bin[start..start + count * 2]
                    .chunks_exact(2)
                    .map(|bytes| u16::from_le_bytes(bytes.try_into().unwrap()) as usize)
                    .collect(),
                5125 => bin[start..start + count * 4]
                    .chunks_exact(4)
                    .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
                    .collect(),
                other => panic!("{other}"),
            };
            // About a point off every face's plane, so that a face wound the wrong way
            // counts, as it would not about a corner of the box.
            let off = [7.0, -5.0, 3.0];
            let point = |index: usize| {
                [0, 1, 2].map(|axis| f64::from(positions[index * 3 + axis]) - off[axis])
            };
            for triangle in indices.chunks_exact(3) {
                let [a, b, c] = [point(triangle[0]), point(triangle[1]), point(triangle[2])];
                volume += Mat3::from_rows([a, b, c]).determinant() / 6.0;
            }
        }
    }
    volume
}

/// A box component whose faces `reversed` SketchUp shows reversed (their backs outside),
/// each face painted with `paint` on the side `inside` names: its back for a face not
/// reversed when painted from inside.
fn painted_box(reversed: &[usize], paint: Option<[u8; 3]>, inside: bool) -> Vec<u8> {
    let mut builder = create();
    let red = paint.map(|colour| builder.add_material("Red", colour, None).unwrap());
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for (index, face) in BOX_FACES.iter().enumerate() {
        let mut face = *face;
        let turned = reversed.contains(&index);
        if turned {
            face.reverse();
        }
        let on_back = inside != turned;
        let options = FaceOptions {
            material: red.filter(|_| !on_back),
            back_material: red.filter(|_| on_back),
            ..FaceOptions::default()
        };
        definition.add_face(&face, &options).unwrap();
    }
    let definition = definition.close().unwrap();
    builder
        .add_instance(definition, &InstanceOptions::default())
        .unwrap();
    builder.to_bytes().unwrap()
}

/// A face is written wound as its front whichever side is painted: the GLB import turns a
/// closed shell to face out by its geometry (review 2026-11-13 R1113-SKP-600). Before, a
/// face painted on its back only was written reversed, so a box painted from inside came
/// inside out, and as no solid.
#[test]
fn a_face_is_wound_as_its_front_whichever_side_is_painted() {
    let box_volume = 10.0 * 20.0 * 30.0 * METRES_PER_INCH.powi(3);
    for inside in [false, true] {
        let model = skp_to_glb(&painted_box(&[], Some([200, 30, 30]), inside), LIMIT).unwrap();
        assert!(model.left_out.is_empty(), "{:?}", model.left_out);
        let volume = signed_volume(&model.glb);
        assert!(
            (volume - box_volume).abs() < box_volume * 1e-4,
            "inside {inside}: {volume} m³, the box is {box_volume} m³"
        );
    }
}

/// glTF's `baseColorFactor` is linear and a SketchUp colour sRGB: red 200, 30, 30 is written
/// as its linear value (review 2026-11-13 R1113-SKP-601: written as 200 / 255, the GLB
/// import, reading it as linear, made it 229, 96, 96).
#[test]
fn a_colour_is_written_linear() {
    let model = skp_to_glb(&painted_box(&[], Some([200, 30, 30]), false), LIMIT).unwrap();
    let document = gltf(&model.glb);
    let factor = &document["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"];
    let linear = |srgb: f64| ((srgb / 255.0 + 0.055) / 1.055).powf(2.4);
    for (channel, expected) in
        factor
            .as_array()
            .unwrap()
            .iter()
            .zip([linear(200.0), linear(30.0), linear(30.0), 1.0])
    {
        assert!(
            (channel.as_f64().unwrap() - expected).abs() < 1e-6,
            "{factor}"
        );
    }
}

/// A box nothing paints, on a tag coloured red, comes in SketchUp's default white, as
/// SketchUp shows it unless "Color by tag" is on (review 2026-11-13 r95 P3: it came in
/// the tag's colour, so the 2013 and 2020 fixtures came red, 255, 84, 84).
#[test]
fn an_unpainted_face_comes_in_the_default_white_not_its_tags_colour() {
    let mut builder = create();
    let walls = builder
        .add_layer(
            "Walls",
            &LayerOptions {
                color: Some([255, 84, 84, 255]),
                ..LayerOptions::default()
            },
        )
        .unwrap();
    let mut definition = builder
        .add_component_definition("Box", &DefinitionOptions::default())
        .unwrap();
    for face in &BOX_FACES {
        let options = FaceOptions {
            layer: Some(walls),
            ..FaceOptions::default()
        };
        definition.add_face(face, &options).unwrap();
    }
    let definition = definition.close().unwrap();
    let options = InstanceOptions {
        layer: Some(walls),
        ..InstanceOptions::default()
    };
    builder.add_instance(definition, &options).unwrap();
    for model in [builder.to_bytes().unwrap(), fixture("nesting_2013.skp")] {
        let document = gltf(&skp_to_glb(&model, LIMIT).unwrap().glb);
        for material in document["materials"].as_array().unwrap() {
            // White is glTF's own default, so it may be left out.
            let factor = &material["pbrMetallicRoughness"]["baseColorFactor"];
            assert!(
                factor.is_null() || *factor == serde_json::json!([1.0, 1.0, 1.0, 1.0]),
                "{material}"
            );
        }
    }
}

/// A box component named `name` placed at `translation`.
fn named_box(name: &str, translation: [f64; 3]) -> Vec<u8> {
    let mut builder = create();
    let mut definition = builder
        .add_component_definition(name, &DefinitionOptions::default())
        .unwrap();
    for face in &BOX_FACES {
        definition.add_face(face, &FaceOptions::default()).unwrap();
    }
    let definition = definition.close().unwrap();
    let placement = InstanceOptions {
        translation,
        ..InstanceOptions::default()
    };
    builder.add_instance(definition, &placement).unwrap();
    builder.to_bytes().unwrap()
}

/// What the GLB import would refuse in GLB terms is refused here in SketchUp's, or made
/// importable: a model of edges alone, a placement that is no transform, a name with a
/// control character. Before (review 2026-11-12 P3), the first came as a "malformed GLB
/// container", the second as malformed GLB JSON and the third as invalid GLB text.
#[test]
fn what_the_glb_import_would_refuse_is_told_in_sketchups_terms() {
    let mut builder = create();
    builder
        .add_polyline(
            &[[0., 0., 0.], [10., 0., 0.], [10., 10., 0.]],
            &Default::default(),
        )
        .unwrap();
    let edges = skp_to_glb(&builder.to_bytes().unwrap(), LIMIT).unwrap_err();
    assert!(matches!(edges, SkpError::NoFaces), "{edges}");

    let lost = skp_to_glb(&named_box("Lost", [f64::NAN, 0.0, 0.0]), LIMIT).unwrap_err();
    assert!(
        matches!(&lost, SkpError::Placement { name } if name == "Lost"),
        "{lost}"
    );

    let model = skp_to_glb(&named_box("Kitchen\u{7f}Unit", [0.0; 3]), LIMIT).unwrap();
    let document = gltf(&model.glb);
    let names: Vec<_> = document["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .chain(document["meshes"].as_array().unwrap())
        .filter_map(|object| object["name"].as_str())
        .collect();
    assert!(names.contains(&"Kitchen Unit"), "{names:?}");
    assert!(
        names.iter().all(|name| !name.chars().any(char::is_control)),
        "{names:?}"
    );
}

#[test]
fn files_saved_by_sketchup_2013_2017_2020_and_2025_are_read() {
    for (name, major) in [
        ("nesting_2013.skp", 13),
        ("nesting_2017.skp", 17),
        ("nesting_2020.skp", 20),
        ("SU_File.skp", 25),
    ] {
        let model =
            skp_to_glb(&fixture(name), LIMIT).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(major_version(&model.version), Some(major), "{name}");
        assert!(model.verified, "{name}");
        assert!(!mesh_placements(&gltf(&model.glb)).is_empty(), "{name}");
    }
}

#[test]
fn an_unverified_version_is_told_apart() {
    assert_eq!(major_version("{21.0.339}"), Some(21));
    assert!(!VERIFIED_MAJOR_VERSIONS.contains(&21));
    assert_eq!(major_version("unknown"), None);
}

#[test]
fn a_model_larger_than_the_limit_is_refused_with_its_size() {
    let error = skp_to_glb(&fixture("SU_File.skp"), 100).unwrap_err();
    assert!(
        matches!(error, SkpError::GlbTooLarge { limit: 100, size } if size > 100),
        "{error}"
    );
}

#[test]
fn bytes_that_are_not_a_sketchup_file_are_refused() {
    for bytes in [&b""[..], b"not a SketchUp file", &[0xff; 4096]] {
        let error = skp_to_glb(bytes, LIMIT).unwrap_err();
        assert!(
            matches!(
                error,
                SkpError::Read {
                    stage: Stage::ReadSkp,
                    ..
                }
            ),
            "{error}"
        );
    }
}

/// A file cut short anywhere, or with a byte changed, gives an answer: a refusal or a GLB,
/// never a panic out of the reader. A cut file is never read as a smaller model: the reader
/// may stop before a tail that holds no geometry, so it either refuses or gives the whole model.
#[test]
fn damaged_files_are_answered() {
    for name in ["SU_File.skp", "nesting_2017.skp"] {
        let whole = fixture(name);
        let complete = skp_to_glb(&whole, LIMIT).unwrap().glb;
        for cut in (1..32).map(|part| whole.len() * part / 32) {
            if let Ok(model) = skp_to_glb(&whole[..cut], LIMIT) {
                assert!(
                    model.glb == complete,
                    "{name} cut at {cut} read as a smaller model"
                );
            }
        }
        for position in (0..whole.len()).step_by(whole.len() / 97 + 1) {
            let mut damaged = whole.clone();
            damaged[position] ^= 0x5a;
            if let Ok(model) = skp_to_glb(&damaged, LIMIT) {
                assert_eq!(&model.glb[..4], b"glTF", "{name} changed at {position}");
            }
        }
    }
}
