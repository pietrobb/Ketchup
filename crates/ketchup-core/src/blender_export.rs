use crate::document::{
    DefinitionId, FeatureId, GroupId, InstancePathStep, LocalGroupKey, LocalOccurrenceKey,
    SceneOccurrence, Snapshot, Transform,
};
use crate::exact_product::{ExactBodyPackage, ExactBodyView, ExactProductError};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const GLB_MAGIC: u32 = 0x4654_6c67;
const GLB_JSON_CHUNK: u32 = 0x4e4f_534a;
const GLB_BIN_CHUNK: u32 = 0x004e_4942;
const GLTF_ARRAY_BUFFER: u32 = 34_962;
const GLTF_ELEMENT_ARRAY_BUFFER: u32 = 34_963;
const GLTF_FLOAT: u32 = 5_126;
const GLTF_UNSIGNED_INT: u32 = 5_125;
const MILLIMETRES_PER_METRE: f64 = 1_000.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactGlbExport {
    pub glb: Vec<u8>,
    pub loss_report: String,
}

#[derive(Clone, Copy)]
pub struct ExactGlbInstance<'a> {
    pub package: &'a ExactBodyPackage,
    pub occurrence: &'a SceneOccurrence,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct GeometryKey {
    definition_id: DefinitionId,
    producer_feature_id: FeatureId,
    result_fingerprint: String,
}

struct GeometryLayout {
    position_accessor: usize,
    index_accessor: usize,
}

struct SceneNode {
    name: String,
    matrix: [f64; 16],
    mesh: Option<usize>,
    children: Vec<usize>,
    extras: Value,
}

impl SceneNode {
    fn into_json(self) -> Value {
        let mut node = json!({
            "name": self.name,
            "matrix": self.matrix,
            "extras": self.extras
        });
        if let Some(mesh) = self.mesh {
            node["mesh"] = json!(mesh);
        }
        if !self.children.is_empty() {
            node["children"] = json!(self.children);
        }
        node
    }
}

pub fn exact_model_glb_export(
    snapshot: &Snapshot,
    instances: &[ExactGlbInstance<'_>],
) -> Result<ExactGlbExport, ExactProductError> {
    if instances.is_empty() {
        return Err(ExactProductError::EmptyModelExport);
    }
    if instances
        .iter()
        .any(|instance| !instance.occurrence.visible || !instance.package.is_current(snapshot))
    {
        return Err(ExactProductError::StaleResult);
    }

    let mut binary = Vec::new();
    let mut buffer_views = Vec::<Value>::new();
    let mut accessors = Vec::<Value>::new();
    let mut geometries = BTreeMap::<GeometryKey, GeometryLayout>::new();
    for instance in instances {
        let key = geometry_key(instance.package);
        if geometries.contains_key(&key) {
            continue;
        }
        let layout = append_geometry(
            instance.package,
            &mut binary,
            &mut buffer_views,
            &mut accessors,
        )?;
        geometries.insert(key, layout);
    }

    let mut material_indices = BTreeMap::<[u8; 3], usize>::new();
    let mut materials = Vec::<Value>::new();
    let mut mesh_indices = BTreeMap::<(GeometryKey, Option<[u8; 3]>), usize>::new();
    let mut meshes = Vec::<Value>::new();
    let mut instance_meshes = Vec::with_capacity(instances.len());
    for instance in instances {
        let geometry_key = geometry_key(instance.package);
        let mesh_key = (geometry_key.clone(), instance.occurrence.color());
        let mesh_index = if let Some(index) = mesh_indices.get(&mesh_key) {
            *index
        } else {
            let geometry = &geometries[&geometry_key];
            let mut primitive = json!({
                "attributes": {"POSITION": geometry.position_accessor},
                "indices": geometry.index_accessor,
                "mode": 4
            });
            if let Some(color) = instance.occurrence.color() {
                let material_index = *material_indices.entry(color).or_insert_with(|| {
                    let index = materials.len();
                    materials.push(material_json(color));
                    index
                });
                primitive["material"] = json!(material_index);
            }
            let index = meshes.len();
            meshes.push(json!({
                "name": instance.occurrence.definition_name,
                "primitives": [primitive]
            }));
            mesh_indices.insert(mesh_key, index);
            index
        };
        instance_meshes.push(mesh_index);
    }

    let (nodes, root_nodes) = build_scene_nodes(snapshot, instances, &instance_meshes)?;
    let nodes = nodes
        .into_iter()
        .map(SceneNode::into_json)
        .collect::<Vec<_>>();
    let mut document = json!({
        "asset": {
            "version": "2.0",
            "generator": "Ketchup",
            "extras": {
                "ketchupSourceDigest": snapshot.canonical_digest(),
                "ketchupSourceUnit": "millimetre",
                "unit": "metre",
                "upAxis": "Y"
            }
        },
        "scene": 0,
        "scenes": [{"name": "Ketchup model", "nodes": root_nodes}],
        "nodes": nodes,
        "meshes": meshes,
        "buffers": [{"byteLength": binary.len()}],
        "bufferViews": buffer_views,
        "accessors": accessors
    });
    if !materials.is_empty() {
        document["materials"] = Value::Array(materials);
    }
    let glb = encode_glb(&document, &binary)?;
    let loss_report = format!(
        "authority=accepted exact OCCT B-Rep\nformat=glTF 2.0 binary (GLB)\nconversion=current-visible-exact-model-to-instanced-mesh-scene\nunit_conversion=millimetres to metres\naxis_conversion=Ketchup Z-up to glTF Y-up\nhierarchy=canonical global groups, component occurrences, local groups, and nested occurrences\nblender_background_import_verified=false\neditability_loss=canonical features, rules, dimensions, constraints, and Undo history are not preserved\ntopology_loss=exact topology, analytic surfaces, and durable face identity are not preserved\ntolerance_loss=geometry is approximated by each accepted tessellation under its source tolerance profile\nsource_digest={}\noccurrence_body_count={}\nunique_geometry_count={}\nmesh_count={}\n",
        snapshot.canonical_digest(),
        instances.len(),
        geometries.len(),
        mesh_indices.len(),
    );
    Ok(ExactGlbExport { glb, loss_report })
}

fn build_scene_nodes(
    snapshot: &Snapshot,
    instances: &[ExactGlbInstance<'_>],
    instance_meshes: &[usize],
) -> Result<(Vec<SceneNode>, Vec<usize>), ExactProductError> {
    let mut nodes = Vec::new();
    let mut root_nodes = Vec::new();
    let mut hierarchy = BTreeMap::<String, usize>::new();
    let identity = gltf_matrix(Transform::identity())?;
    for (instance, mesh_index) in instances.iter().zip(instance_meshes) {
        let root_id = instance.occurrence.instance_path.root_occurrence();
        let root_occurrence = snapshot
            .occurrence(root_id)
            .ok_or(ExactProductError::InvalidMeshExport)?;
        let root_key = format!("occurrence:{}", root_id.0);
        let root_index = if let Some(index) = hierarchy.get(&root_key) {
            *index
        } else {
            let parent = root_occurrence
                .parent()
                .map(|group_id| {
                    ensure_global_group(
                        snapshot,
                        group_id,
                        &mut nodes,
                        &mut root_nodes,
                        &mut hierarchy,
                    )
                })
                .transpose()?;
            let index = push_scene_node(
                &mut nodes,
                SceneNode {
                    name: root_occurrence.name().to_owned(),
                    matrix: gltf_matrix(root_occurrence.transform())?,
                    mesh: None,
                    children: Vec::new(),
                    extras: json!({
                        "ketchupEntity": "occurrence",
                        "ketchupDefinition": snapshot
                            .definition(root_occurrence.definition_id())
                            .ok_or(ExactProductError::InvalidMeshExport)?
                            .name(),
                        "ketchupInstancePath": root_key
                    }),
                },
            );
            attach_scene_node(&mut nodes, &mut root_nodes, parent, index);
            hierarchy.insert(root_key.clone(), index);
            index
        };

        let mut owner_definition_id = root_occurrence.definition_id();
        let mut parent_index = root_index;
        let mut path_key = root_key;
        for step in instance.occurrence.instance_path.steps() {
            match step {
                InstancePathStep::Group(local_id) => {
                    path_key.push_str(&format!("/group:{}", local_id.0));
                    let key = LocalGroupKey {
                        definition_id: owner_definition_id,
                        local_id: *local_id,
                    };
                    let local = snapshot
                        .local_group(key)
                        .ok_or(ExactProductError::InvalidMeshExport)?;
                    parent_index = ensure_local_scene_node(
                        &mut nodes,
                        &mut hierarchy,
                        parent_index,
                        &path_key,
                        local.name(),
                        local.transform(),
                        json!({
                            "ketchupEntity": "group",
                            "ketchupInstancePath": path_key
                        }),
                    )?;
                }
                InstancePathStep::Occurrence(local_id) => {
                    path_key.push_str(&format!("/occurrence:{}", local_id.0));
                    let key = LocalOccurrenceKey {
                        definition_id: owner_definition_id,
                        local_id: *local_id,
                    };
                    let local = snapshot
                        .local_occurrence(key)
                        .ok_or(ExactProductError::InvalidMeshExport)?;
                    let definition = snapshot
                        .definition(local.definition_id())
                        .ok_or(ExactProductError::InvalidMeshExport)?;
                    parent_index = ensure_local_scene_node(
                        &mut nodes,
                        &mut hierarchy,
                        parent_index,
                        &path_key,
                        local.name(),
                        local.transform(),
                        json!({
                            "ketchupEntity": "occurrence",
                            "ketchupDefinition": definition.name(),
                            "ketchupInstancePath": path_key
                        }),
                    )?;
                    owner_definition_id = local.definition_id();
                }
            }
        }
        if owner_definition_id != instance.occurrence.definition_id {
            return Err(ExactProductError::InvalidMeshExport);
        }
        if nodes[parent_index].mesh.is_none() {
            nodes[parent_index].mesh = Some(*mesh_index);
            nodes[parent_index].extras["ketchupProducerFeatureId"] =
                json!(instance.package.producer_feature_id().0);
        } else {
            let body_index = push_scene_node(
                &mut nodes,
                SceneNode {
                    name: format!(
                        "{} body {}",
                        instance.occurrence.definition_name,
                        instance.package.producer_feature_id().0
                    ),
                    matrix: identity,
                    mesh: Some(*mesh_index),
                    children: Vec::new(),
                    extras: json!({
                        "ketchupEntity": "body",
                        "ketchupProducerFeatureId": instance.package.producer_feature_id().0
                    }),
                },
            );
            attach_scene_node(&mut nodes, &mut root_nodes, Some(parent_index), body_index);
        }
    }
    Ok((nodes, root_nodes))
}

fn ensure_global_group(
    snapshot: &Snapshot,
    group_id: GroupId,
    nodes: &mut Vec<SceneNode>,
    root_nodes: &mut Vec<usize>,
    hierarchy: &mut BTreeMap<String, usize>,
) -> Result<usize, ExactProductError> {
    let key = format!("group:{}", group_id.0);
    if let Some(index) = hierarchy.get(&key) {
        return Ok(*index);
    }
    let group = snapshot
        .group(group_id)
        .ok_or(ExactProductError::InvalidMeshExport)?;
    let parent = group
        .parent()
        .map(|parent_id| ensure_global_group(snapshot, parent_id, nodes, root_nodes, hierarchy))
        .transpose()?;
    let index = push_scene_node(
        nodes,
        SceneNode {
            name: group.name().to_owned(),
            matrix: gltf_matrix(group.transform())?,
            mesh: None,
            children: Vec::new(),
            extras: json!({"ketchupEntity": "group", "ketchupGroupId": group_id.0}),
        },
    );
    attach_scene_node(nodes, root_nodes, parent, index);
    hierarchy.insert(key, index);
    Ok(index)
}

fn ensure_local_scene_node(
    nodes: &mut Vec<SceneNode>,
    hierarchy: &mut BTreeMap<String, usize>,
    parent_index: usize,
    key: &str,
    name: &str,
    transform: Transform,
    extras: Value,
) -> Result<usize, ExactProductError> {
    if let Some(index) = hierarchy.get(key) {
        return Ok(*index);
    }
    let index = push_scene_node(
        nodes,
        SceneNode {
            name: name.to_owned(),
            matrix: gltf_matrix(transform)?,
            mesh: None,
            children: Vec::new(),
            extras,
        },
    );
    nodes[parent_index].children.push(index);
    hierarchy.insert(key.to_owned(), index);
    Ok(index)
}

fn push_scene_node(nodes: &mut Vec<SceneNode>, node: SceneNode) -> usize {
    let index = nodes.len();
    nodes.push(node);
    index
}

fn attach_scene_node(
    nodes: &mut [SceneNode],
    root_nodes: &mut Vec<usize>,
    parent: Option<usize>,
    index: usize,
) {
    if let Some(parent) = parent {
        nodes[parent].children.push(index);
    } else {
        root_nodes.push(index);
    }
}

fn geometry_key(package: &ExactBodyPackage) -> GeometryKey {
    GeometryKey {
        definition_id: package.definition_id(),
        producer_feature_id: package.producer_feature_id(),
        result_fingerprint: package.result_fingerprint().to_owned(),
    }
}

fn append_geometry(
    package: &ExactBodyPackage,
    binary: &mut Vec<u8>,
    buffer_views: &mut Vec<Value>,
    accessors: &mut Vec<Value>,
) -> Result<GeometryLayout, ExactProductError> {
    if package.vertices().is_empty() || package.triangles().is_empty() {
        return Err(ExactProductError::InvalidMeshExport);
    }
    align_to_four(binary, 0);
    let position_offset = binary.len();
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for vertex in package.vertices() {
        let point = ketchup_point_to_gltf(vertex.position_mm)?;
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
            binary.extend_from_slice(&point[axis].to_le_bytes());
        }
    }
    let position_length = binary.len() - position_offset;
    let position_view = buffer_views.len();
    buffer_views.push(json!({
        "buffer": 0,
        "byteOffset": position_offset,
        "byteLength": position_length,
        "target": GLTF_ARRAY_BUFFER
    }));
    let position_accessor = accessors.len();
    accessors.push(json!({
        "bufferView": position_view,
        "byteOffset": 0,
        "componentType": GLTF_FLOAT,
        "count": package.vertices().len(),
        "type": "VEC3",
        "min": minimum,
        "max": maximum
    }));

    align_to_four(binary, 0);
    let index_offset = binary.len();
    for triangle in package.triangles() {
        for index in triangle.vertex_indices {
            if index as usize >= package.vertices().len() {
                return Err(ExactProductError::InvalidMeshExport);
            }
            binary.extend_from_slice(&index.to_le_bytes());
        }
    }
    let index_length = binary.len() - index_offset;
    let index_view = buffer_views.len();
    buffer_views.push(json!({
        "buffer": 0,
        "byteOffset": index_offset,
        "byteLength": index_length,
        "target": GLTF_ELEMENT_ARRAY_BUFFER
    }));
    let index_accessor = accessors.len();
    accessors.push(json!({
        "bufferView": index_view,
        "byteOffset": 0,
        "componentType": GLTF_UNSIGNED_INT,
        "count": package.triangles().len() * 3,
        "type": "SCALAR"
    }));
    Ok(GeometryLayout {
        position_accessor,
        index_accessor,
    })
}

fn ketchup_point_to_gltf(point_mm: [f64; 3]) -> Result<[f32; 3], ExactProductError> {
    let point = [
        point_mm[0] / MILLIMETRES_PER_METRE,
        point_mm[2] / MILLIMETRES_PER_METRE,
        -point_mm[1] / MILLIMETRES_PER_METRE,
    ];
    if point.iter().any(|value| !value.is_finite()) {
        return Err(ExactProductError::InvalidMeshExport);
    }
    Ok(point.map(|value| value as f32))
}

fn gltf_matrix(transform: Transform) -> Result<[f64; 16], ExactProductError> {
    let conversion = [
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let inverse = [
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut converted = multiply_matrix(multiply_matrix(conversion, *transform.matrix()), inverse);
    converted[3] /= MILLIMETRES_PER_METRE;
    converted[7] /= MILLIMETRES_PER_METRE;
    converted[11] /= MILLIMETRES_PER_METRE;
    let determinant = converted[0] * (converted[5] * converted[10] - converted[6] * converted[9])
        - converted[1] * (converted[4] * converted[10] - converted[6] * converted[8])
        + converted[2] * (converted[4] * converted[9] - converted[5] * converted[8]);
    if converted.iter().any(|value| !value.is_finite()) || determinant.abs() <= f64::EPSILON {
        return Err(ExactProductError::InvalidMeshExport);
    }
    let mut column_major = [0.0; 16];
    for row in 0..4 {
        for column in 0..4 {
            column_major[column * 4 + row] = converted[row * 4 + column];
        }
    }
    Ok(column_major)
}

fn multiply_matrix(left: [f64; 16], right: [f64; 16]) -> [f64; 16] {
    let mut result = [0.0; 16];
    for row in 0..4 {
        for column in 0..4 {
            result[row * 4 + column] = (0..4)
                .map(|index| left[row * 4 + index] * right[index * 4 + column])
                .sum();
        }
    }
    result
}

fn material_json(color: [u8; 3]) -> Value {
    let [red, green, blue] = color.map(srgb_channel_to_linear);
    json!({
        "name": format!("Ketchup #{:02X}{:02X}{:02X}", color[0], color[1], color[2]),
        "pbrMetallicRoughness": {
            "baseColorFactor": [red, green, blue, 1.0],
            "metallicFactor": 0.0,
            "roughnessFactor": 0.8
        }
    })
}

fn srgb_channel_to_linear(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn encode_glb(document: &Value, binary: &[u8]) -> Result<Vec<u8>, ExactProductError> {
    let mut json_bytes =
        serde_json::to_vec(document).map_err(|_| ExactProductError::InvalidMeshExport)?;
    align_to_four(&mut json_bytes, b' ');
    let mut binary = binary.to_vec();
    align_to_four(&mut binary, 0);
    let total_length = 12_usize
        .checked_add(8)
        .and_then(|length| length.checked_add(json_bytes.len()))
        .and_then(|length| length.checked_add(8))
        .and_then(|length| length.checked_add(binary.len()))
        .and_then(|length| u32::try_from(length).ok())
        .ok_or(ExactProductError::InvalidMeshExport)?;
    let json_length =
        u32::try_from(json_bytes.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
    let binary_length =
        u32::try_from(binary.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
    let mut glb = Vec::with_capacity(total_length as usize);
    glb.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&total_length.to_le_bytes());
    glb.extend_from_slice(&json_length.to_le_bytes());
    glb.extend_from_slice(&GLB_JSON_CHUNK.to_le_bytes());
    glb.extend_from_slice(&json_bytes);
    glb.extend_from_slice(&binary_length.to_le_bytes());
    glb.extend_from_slice(&GLB_BIN_CHUNK.to_le_bytes());
    glb.extend_from_slice(&binary);
    Ok(glb)
}

fn align_to_four(bytes: &mut Vec<u8>, padding: u8) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(padding);
    }
}
