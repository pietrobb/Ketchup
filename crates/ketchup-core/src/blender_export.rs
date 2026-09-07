use crate::document::{DefinitionId, FeatureId, SceneOccurrence, Snapshot, Transform};
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
    let mut nodes = Vec::<Value>::new();
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
        nodes.push(json!({
            "name": instance.occurrence.occurrence_name,
            "mesh": mesh_index,
            "matrix": gltf_matrix(instance.occurrence.transform)?,
            "extras": {
                "ketchupDefinition": instance.occurrence.definition_name,
                "ketchupInstancePath": instance_path_label(instance.occurrence),
                "ketchupProducerFeatureId": instance.package.producer_feature_id().0
            }
        }));
    }

    let root_nodes = (0..nodes.len()).collect::<Vec<_>>();
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
        "authority=accepted exact OCCT B-Rep\nformat=glTF 2.0 binary (GLB)\nconversion=current-visible-exact-model-to-instanced-mesh-scene\nunit_conversion=millimetres to metres\naxis_conversion=Ketchup Z-up to glTF Y-up\neditability_loss=canonical features, rules, dimensions, constraints, and Undo history are not preserved\ntopology_loss=exact topology, analytic surfaces, and durable face identity are not preserved\ntolerance_loss=geometry is approximated by each accepted tessellation under its source tolerance profile\nsource_digest={}\noccurrence_body_count={}\nunique_geometry_count={}\nmesh_count={}\n",
        snapshot.canonical_digest(),
        instances.len(),
        geometries.len(),
        mesh_indices.len(),
    );
    Ok(ExactGlbExport { glb, loss_report })
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

fn instance_path_label(occurrence: &SceneOccurrence) -> String {
    let mut label = format!(
        "occurrence:{}",
        occurrence.instance_path.root_occurrence().0
    );
    for step in occurrence.instance_path.steps() {
        use crate::document::InstancePathStep;
        match step {
            InstancePathStep::Group(id) => label.push_str(&format!("/group:{}", id.0)),
            InstancePathStep::Occurrence(id) => {
                label.push_str(&format!("/occurrence:{}", id.0));
            }
        }
    }
    label
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
