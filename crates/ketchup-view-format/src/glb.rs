//! The embedded subset emitted by the existing Kečup GLB exporter: shared
//! float POSITION accessors, u32 indices, matrices, colors and a node forest.
//! No textures, extensions, sparse accessors, URIs, animation or executable data.
use crate::{MAX_GLB_BYTES, MAX_MANIFEST_BYTES, reject};
use ketchup_rejection::Rejection;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) mod geometry;

/// The glTF JSON subset the exporter writes; any other field is refused.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    asset: Asset,
    scene: usize,
    scenes: Vec<Scene>,
    nodes: Vec<Node>,
    meshes: Vec<Mesh>,
    buffers: Vec<Buffer>,
    buffer_views: Vec<View>,
    accessors: Vec<Accessor>,
    #[serde(default)]
    materials: Vec<Material>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    version: String,
    generator: Option<String>,
    extras: BTreeMap<String, Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scene {
    name: Option<String>,
    nodes: Vec<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Node {
    name: Option<String>,
    matrix: [f64; 16],
    pub mesh: Option<usize>,
    #[serde(default)]
    pub children: Vec<usize>,
    #[serde(default)]
    pub extras: BTreeMap<String, Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mesh {
    name: Option<String>,
    primitives: Vec<Primitive>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Primitive {
    attributes: Attributes,
    indices: usize,
    mode: u32,
    material: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attributes {
    #[serde(rename = "POSITION")]
    position: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Buffer {
    byte_length: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct View {
    buffer: usize,
    #[serde(default)]
    byte_offset: usize,
    byte_length: usize,
    target: Option<u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Accessor {
    buffer_view: usize,
    #[serde(default)]
    byte_offset: usize,
    component_type: u32,
    count: usize,
    #[serde(rename = "type")]
    kind: String,
    min: Option<Vec<f64>>,
    max: Option<Vec<f64>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Material {
    name: Option<String>,
    pbr_metallic_roughness: Pbr,
    double_sided: Option<bool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Pbr {
    base_color_factor: [f64; 4],
    metallic_factor: f64,
    roughness_factor: f64,
}

/// What the manifest is validated against: the node tree and the source identity.
pub(crate) struct Info {
    pub nodes: Vec<Node>,
    pub source_digest: String,
}

/// Validate the GLB and return what the manifest must agree with.
pub(crate) fn validate(bytes: &[u8]) -> Result<Info, Rejection> {
    let (doc, _) = validated_document(bytes)?;
    Ok(Info {
        source_digest: source_digest(&doc)?,
        nodes: doc.nodes,
    })
}

/// Parse the GLB chunks and check the whole document; returns it with its binary chunk.
fn validated_document(bytes: &[u8]) -> Result<(Document, &[u8]), Rejection> {
    if bytes.len() < 28
        || bytes.len() > MAX_GLB_BYTES
        || &bytes[..4] != b"glTF"
        || word(&bytes[4..8]) != 2
        || usize_word(&bytes[8..12])? != bytes.len()
    {
        return Err(reject("glb.header", "Invalid GLB 2.0 header or length."));
    }
    let json_len = usize_word(&bytes[12..16])?;
    let bin_header = 20_usize
        .checked_add(json_len)
        .filter(|&end| end <= bytes.len() - 8)
        .ok_or_else(|| reject("glb.json", "GLB JSON chunk is truncated."))?;
    if json_len > MAX_MANIFEST_BYTES || json_len % 4 != 0 || word(&bytes[16..20]) != 0x4e4f534a {
        return Err(reject(
            "glb.json",
            "GLB JSON chunk is unsupported or too large.",
        ));
    }
    let bin_len = usize_word(&bytes[bin_header..bin_header + 4])?;
    if bin_len % 4 != 0
        || bin_len != bytes.len() - bin_header - 8
        || word(&bytes[bin_header + 4..bin_header + 8]) != 0x004e4942
    {
        return Err(reject(
            "glb.binary",
            "Invalid GLB binary chunk; extra chunks are not supported.",
        ));
    }
    let doc: Document = serde_json::from_slice(&bytes[20..bin_header]).map_err(|error| {
        reject(
            "glb.json",
            "GLB uses malformed JSON or unsupported features/resources.",
        )
        .caused_by(error)
    })?;
    let binary = &bytes[bin_header + 8..];
    validate_document(&doc, binary)?;
    source_digest(&doc)?;
    Ok((doc, binary))
}

/// The Kečup source identity stored in the asset extras.
fn source_digest(doc: &Document) -> Result<String, Rejection> {
    doc.asset
        .extras
        .get("ketchupSourceDigest")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| reject("glb.asset", "Geometry has no Kečup source identity."))
}

/// Asset, buffers, views, accessors, meshes, materials and the node tree.
fn validate_document(doc: &Document, binary: &[u8]) -> Result<(), Rejection> {
    if doc.asset.extras.iter().any(|(key, value)| {
        !matches!(
            key.as_str(),
            "ketchupSourceDigest" | "ketchupSourceUnit" | "unit" | "upAxis"
        ) || !value.is_string()
    }) {
        return Err(reject(
            "glb.asset.extras",
            "Only Kečup source identity and units are supported.",
        ));
    }
    if doc.asset.version != "2.0"
        || doc.asset.generator.as_deref() != Some("Ketchup")
        || doc.asset.extras.get("unit").and_then(Value::as_str) != Some("metre")
        || doc.asset.extras.get("upAxis").and_then(Value::as_str) != Some("Y")
        || doc
            .asset
            .extras
            .get("ketchupSourceUnit")
            .and_then(Value::as_str)
            != Some("millimetre")
        || doc.scene != 0
        || doc.scenes.len() != 1
        || doc.buffers.len() != 1
        || doc.buffers[0].byte_length > binary.len()
        || binary.len() - doc.buffers[0].byte_length > 3
        || doc.nodes.is_empty()
        || doc.nodes.len() > 100_000
        || doc.meshes.is_empty()
        || doc.meshes.len() > 100_000
        || doc.accessors.len() > 100_000
        || doc.buffer_views.len() > 100_000
    {
        return Err(reject(
            "glb",
            "Unsupported GLB units, scene, buffer or resource counts.",
        ));
    }
    let _scene_name = &doc.scenes[0].name;
    for view in &doc.buffer_views {
        if view.buffer != 0
            || view.byte_offset % 4 != 0
            || view.byte_length == 0
            || !view
                .byte_offset
                .checked_add(view.byte_length)
                .is_some_and(|end| end <= doc.buffers[0].byte_length)
            || view
                .target
                .is_some_and(|target| target != 34962 && target != 34963)
        {
            return Err(reject(
                "glb.bufferViews",
                "Invalid binary buffer range or target.",
            ));
        }
    }
    let mut elements = 0_usize;
    for accessor in &doc.accessors {
        elements = elements
            .checked_add(accessor.count)
            .filter(|&n| n <= 16_000_000)
            .ok_or_else(|| reject("glb.accessors", "Accessor processing limit exceeded."))?;
        let data = accessor_data(doc, accessor, binary)?;
        if accessor.component_type == 5126
            && data.chunks_exact(4).any(|v| {
                let value = f32::from_le_bytes(v.try_into().expect("four bytes"));
                !value.is_finite() || value.abs() > 1e6
            })
        {
            return Err(reject("glb.positions", "Non-finite mesh coordinates."));
        }
    }
    let mut primitive_count = 0_usize;
    let mut processed_indices = 0_usize;
    for mesh in &doc.meshes {
        let _mesh_name = &mesh.name;
        primitive_count += mesh.primitives.len();
        if mesh.primitives.is_empty() || primitive_count > 100_000 {
            return Err(reject("glb.meshes", "Empty mesh or too many primitives."));
        }
        for primitive in &mesh.primitives {
            validate_primitive(doc, primitive, binary, &mut processed_indices)?;
        }
    }
    for material in &doc.materials {
        let _name = &material.name;
        let _double_sided = material.double_sided;
        let pbr = &material.pbr_metallic_roughness;
        if pbr
            .base_color_factor
            .iter()
            .chain([&pbr.metallic_factor, &pbr.roughness_factor])
            .any(|&v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err(reject(
                "glb.materials",
                "Material factors must be finite within [0,1].",
            ));
        }
    }
    validate_tree(doc)
}

/// The bytes of one accessor, after checking type, alignment and bounds.
fn accessor_data<'a>(
    doc: &Document,
    accessor: &Accessor,
    binary: &'a [u8],
) -> Result<&'a [u8], Rejection> {
    let element_bytes = match (accessor.component_type, accessor.kind.as_str()) {
        (5126, "VEC3") => 12,
        (5125, "SCALAR") => 4,
        _ => {
            return Err(reject(
                "glb.accessors",
                "Only float VEC3 positions and u32 indices are supported.",
            ));
        }
    };
    let view = doc
        .buffer_views
        .get(accessor.buffer_view)
        .ok_or_else(|| reject("glb.accessors", "Accessor refers to a missing buffer view."))?;
    let len = accessor
        .count
        .checked_mul(element_bytes)
        .filter(|&len| len > 0)
        .ok_or_else(|| reject("glb.accessors", "Accessor size is invalid."))?;
    if !accessor.byte_offset.is_multiple_of(4)
        || !accessor
            .byte_offset
            .checked_add(len)
            .is_some_and(|end| end <= view.byte_length)
        || accessor
            .min
            .iter()
            .chain(&accessor.max)
            .flatten()
            .any(|v| !v.is_finite())
    {
        return Err(reject(
            "glb.accessors",
            "Accessor exceeds its view or has invalid bounds.",
        ));
    }
    let start = view.byte_offset + accessor.byte_offset;
    Ok(&binary[start..start + len])
}

/// One primitive: supported mode, POSITION and u32 indices within range.
fn validate_primitive(
    doc: &Document,
    primitive: &Primitive,
    binary: &[u8],
    processed_indices: &mut usize,
) -> Result<(), Rejection> {
    let positions = doc
        .accessors
        .get(primitive.attributes.position)
        .filter(|a| a.component_type == 5126 && a.kind == "VEC3")
        .ok_or_else(|| reject("glb.primitive", "Missing position accessor."))?;
    let indices = doc
        .accessors
        .get(primitive.indices)
        .filter(|a| a.component_type == 5125 && a.kind == "SCALAR")
        .ok_or_else(|| reject("glb.primitive", "Missing index accessor."))?;
    let divisor = match primitive.mode {
        1 => 2,
        4 => 3,
        _ => {
            return Err(reject(
                "glb.primitive",
                "Only lines and triangles are supported.",
            ));
        }
    };
    if indices.count % divisor != 0 || primitive.material.is_some_and(|i| i >= doc.materials.len())
    {
        return Err(reject(
            "glb.primitive",
            "Invalid primitive count or material reference.",
        ));
    }
    *processed_indices = processed_indices
        .checked_add(indices.count)
        .filter(|&n| n <= 16_000_000)
        .ok_or_else(|| reject("glb.indices", "Index processing limit exceeded."))?;
    for index in accessor_data(doc, indices, binary)?.chunks_exact(4) {
        if usize_word(index)? >= positions.count {
            return Err(reject(
                "glb.indices",
                "An index refers to a missing vertex.",
            ));
        }
    }
    Ok(())
}

/// Nodes form a forest: one parent each, no cycles, finite matrices.
fn validate_tree(doc: &Document) -> Result<(), Rejection> {
    let mut parents = vec![0_usize; doc.nodes.len()];
    for node in &doc.nodes {
        if node.extras.iter().any(|(key, value)| match key.as_str() {
            "ketchupGroupId"
            | "ketchupProducerFeatureId"
            | "ketchupDefinitionId"
            | "ketchupOwnerDefinitionId" => !value.as_u64().is_some_and(|id| id > 0),
            "ketchupEntity" => !value
                .as_str()
                .is_some_and(|s| matches!(s, "group" | "occurrence" | "body")),
            "ketchupDefinition" | "ketchupInstancePath" => {
                !value.as_str().is_some_and(|s| s.len() <= 4096)
            }
            _ => true,
        }) {
            return Err(reject(
                "glb.nodes.extras",
                "Unsupported or invalid geometry source metadata.",
            ));
        }
        let _name = &node.name;
        if node.matrix.iter().any(|v| !v.is_finite())
            || node.matrix[3] != 0.0
            || node.matrix[7] != 0.0
            || node.matrix[11] != 0.0
            || node.matrix[15] != 1.0
            || node.mesh.is_some_and(|i| i >= doc.meshes.len())
        {
            return Err(reject(
                "glb.nodes",
                "Invalid node matrix or mesh reference.",
            ));
        }
        for &child in &node.children {
            let count = parents
                .get_mut(child)
                .ok_or_else(|| reject("glb.nodes", "Missing child node."))?;
            *count += 1;
            if *count > 1 {
                return Err(reject("glb.nodes", "A node has multiple parents."));
            }
        }
    }
    let mut queue: Vec<usize> = parents
        .iter()
        .enumerate()
        .filter_map(|(i, &p)| (p == 0).then_some(i))
        .collect();
    let mut roots = doc.scenes[0].nodes.clone();
    roots.sort_unstable();
    if roots != queue {
        return Err(reject(
            "glb.scene",
            "Scene roots do not match the node forest.",
        ));
    }
    let mut world: Vec<_> = doc.nodes.iter().map(|node| node.matrix).collect();
    let mut cursor = 0;
    while cursor < queue.len() {
        let parent = queue[cursor];
        for &child in &doc.nodes[parent].children {
            // Column-major glTF matrices; finite local entries can overflow when composed.
            let combined = std::array::from_fn::<_, 16, _>(|i| {
                (0..4)
                    .map(|k| world[parent][k * 4 + i % 4] * doc.nodes[child].matrix[i / 4 * 4 + k])
                    .sum::<f64>()
            });
            if combined.iter().any(|v| !v.is_finite()) {
                return Err(reject(
                    "glb.nodes",
                    "Nested transforms overflow world coordinates.",
                ));
            }
            world[child] = combined;
        }
        queue.extend_from_slice(&doc.nodes[parent].children);
        cursor += 1;
    }
    if queue.len() != doc.nodes.len() {
        return Err(reject("glb.nodes", "Node hierarchy contains a cycle."));
    }
    Ok(())
}

fn word(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("four byte GLB field"))
}
fn usize_word(bytes: &[u8]) -> Result<usize, Rejection> {
    usize::try_from(word(bytes)).map_err(|_: std::num::TryFromIntError| {
        reject("glb.length", "GLB value exceeds address space.")
    })
}
