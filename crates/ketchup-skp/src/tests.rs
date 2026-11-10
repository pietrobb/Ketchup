use super::*;
use openskp::create::{
    DefinitionOptions, FaceOptions, GuideOptions, InstanceOptions, LayerOptions, Point3, create,
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
