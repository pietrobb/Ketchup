use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{Map, Value};

use super::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportLengthUnit, ImportOutputRef,
    ImportReceipt, ImportUnitAuthority, ImportUnitDecision, MAX_IMPORT_OUTPUTS,
};
use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, FeatureKind, GroupId,
    MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec, OccurrenceId, Snapshot, Transform,
};

pub const GLB_PARSER_ID: &str = "ketchup-glb";
pub const GLB_PARSER_VERSION: &str = "1";
pub const MAX_GLB_SOURCE_BYTES: u64 = 32 * 1024 * 1024;

const GLB_MAGIC: u32 = 0x4654_6c67;
const JSON_CHUNK: u32 = 0x4e4f_534a;
const BIN_CHUNK: u32 = 0x004e_4942;
const BYTE: u64 = 5_120;
const UNSIGNED_BYTE: u64 = 5_121;
const SHORT: u64 = 5_122;
const UNSIGNED_SHORT: u64 = 5_123;
const UNSIGNED_INT: u64 = 5_125;
const FLOAT: u64 = 5_126;
const MAX_MESHES: usize = 512;
const MAX_PRIMITIVES: usize = 512;
const MAX_NODES: usize = 512;
const MAX_TOTAL_VERTICES: usize = 200_000;
const MAX_TOTAL_TRIANGLES: usize = 400_000;
const MAX_GLB_COMMANDS: usize = 1_024;
const MAX_VERTICES_PER_PRIMITIVE: usize = 100_000;
const MAX_TRIANGLES_PER_PRIMITIVE: usize = 200_000;
const MAX_TEXT_BYTES: usize = 1_024;
const MAX_ABS_MM: f64 = 1_000_000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedGlbScene {
    primitives: Vec<ParsedPrimitive>,
    nodes: Vec<ParsedNode>,
    traversal: Vec<usize>,
    diagnostics: Vec<ImportDiagnostic>,
    instance_count: usize,
    triangle_count: usize,
}

impl ParsedGlbScene {
    #[must_use]
    pub fn mesh_primitive_count(&self) -> usize {
        self.primitives.len()
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.traversal.len()
    }

    #[must_use]
    pub const fn instance_count(&self) -> usize {
        self.instance_count
    }

    #[must_use]
    pub const fn triangle_count(&self) -> usize {
        self.triangle_count
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedPrimitive {
    name: String,
    vertices_mm: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedNode {
    name: String,
    transform: Transform,
    children: Vec<usize>,
    primitives: Vec<usize>,
}

#[derive(Clone, Copy)]
struct BufferView {
    offset: usize,
    length: usize,
    stride: Option<usize>,
}

#[derive(Clone, Copy)]
struct Accessor {
    view: usize,
    offset: usize,
    component_type: u64,
    count: usize,
    kind: AccessorKind,
}

type ParsedMeshes = (Vec<ParsedPrimitive>, Vec<Vec<usize>>, usize);
type IndexedGeometry = (Vec<[f64; 3]>, Vec<[u32; 3]>);

struct SceneSelection {
    nodes: BTreeSet<usize>,
    meshes: BTreeSet<usize>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum AccessorKind {
    Scalar,
    Vec3,
    Other(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GlbImportError {
    Empty,
    SourceTooLarge,
    InvalidContainer,
    InvalidJson,
    UnsupportedVersion,
    UnsupportedFeature,
    EmptyScene,
    TooManyMeshes,
    TooManyNodes,
    TooManyVertices,
    TooManyTriangles,
    TooManyOutputs,
    InvalidReference,
    InvalidAccessor,
    InvalidGeometry,
    InvalidTransform,
    InvalidHierarchy,
    InvalidText,
    InvalidSourceIdentity,
    IdSpaceExhausted,
}

impl fmt::Display for GlbImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "GLB source is empty",
            Self::SourceTooLarge => "GLB source exceeds the bounded 32 MiB envelope",
            Self::InvalidContainer => "GLB container or chunk layout is malformed",
            Self::InvalidJson => "GLB JSON document is malformed",
            Self::UnsupportedVersion => "GLB must contain a glTF 2.0 document",
            Self::UnsupportedFeature => "GLB uses a feature outside the safe mesh import subset",
            Self::EmptyScene => "GLB selected scene contains no importable mesh instances",
            Self::TooManyMeshes => "GLB exceeds the bounded mesh primitive envelope",
            Self::TooManyNodes => "GLB exceeds the bounded scene-node envelope",
            Self::TooManyVertices => "GLB exceeds the bounded vertex envelope",
            Self::TooManyTriangles => "GLB exceeds the bounded triangle envelope",
            Self::TooManyOutputs => "GLB expands beyond the bounded canonical output envelope",
            Self::InvalidReference => "GLB contains an out-of-range object reference",
            Self::InvalidAccessor => "GLB contains an invalid or unsupported accessor",
            Self::InvalidGeometry => "GLB contains invalid solid mesh geometry",
            Self::InvalidTransform => "GLB contains a non-finite or degenerate node transform",
            Self::InvalidHierarchy => "GLB selected scene is not a finite single-parent tree",
            Self::InvalidText => "GLB contains invalid identity text",
            Self::InvalidSourceIdentity => "GLB source name or provenance is invalid",
            Self::IdSpaceExhausted => "canonical import ID space is exhausted",
        })
    }
}

impl std::error::Error for GlbImportError {}

pub fn inspect_glb(source: &[u8]) -> Result<ParsedGlbScene, GlbImportError> {
    if source.is_empty() {
        return Err(GlbImportError::Empty);
    }
    if source.len() as u64 > MAX_GLB_SOURCE_BYTES {
        return Err(GlbImportError::SourceTooLarge);
    }
    let (json_bytes, binary) = parse_container(source)?;
    let root: Value =
        serde_json::from_slice(json_bytes).map_err(|_| GlbImportError::InvalidJson)?;
    let root = object(&root)?;
    validate_asset(root)?;
    reject_required_extensions(root)?;

    let buffers = array_required(root, "buffers")?;
    if buffers.len() != 1 {
        return Err(GlbImportError::UnsupportedFeature);
    }
    let buffer = object(&buffers[0])?;
    if buffer.contains_key("uri") {
        return Err(GlbImportError::UnsupportedFeature);
    }
    let declared_binary_length = usize_value(buffer, "byteLength")?;
    if declared_binary_length == 0
        || declared_binary_length > binary.len()
        || binary.len() - declared_binary_length > 3
        || binary[declared_binary_length..]
            .iter()
            .any(|byte| *byte != 0)
    {
        return Err(GlbImportError::InvalidContainer);
    }
    let binary = &binary[..declared_binary_length];
    let buffer_views = parse_buffer_views(root, binary.len())?;
    let accessors = parse_accessors(root, &buffer_views, binary.len())?;

    let selection = select_scene(root)?;
    let materials = optional_array(root, "materials")?;
    let mut diagnostic_counts = BTreeMap::<&'static str, u32>::new();
    record_document_losses(root, &mut diagnostic_counts)?;
    let material_colors = parse_materials(materials, &mut diagnostic_counts)?;
    let (primitives, mesh_primitives, total_triangles) = parse_meshes(
        root,
        binary,
        &buffer_views,
        &accessors,
        &material_colors,
        &selection.meshes,
        &mut diagnostic_counts,
    )?;
    let (nodes, traversal, instance_count) = parse_nodes(
        root,
        &mesh_primitives,
        &selection.nodes,
        &mut diagnostic_counts,
    )?;
    if instance_count == 0 {
        return Err(GlbImportError::EmptyScene);
    }
    let output_count = primitives
        .len()
        .checked_mul(2)
        .and_then(|count| count.checked_add(traversal.len()))
        .and_then(|count| count.checked_add(instance_count))
        .ok_or(GlbImportError::TooManyOutputs)?;
    let colored_instances = traversal.iter().try_fold(0_usize, |count, node_index| {
        let colored = nodes[*node_index]
            .primitives
            .iter()
            .filter(|primitive| primitives[**primitive].color.is_some())
            .count();
        count
            .checked_add(colored)
            .ok_or(GlbImportError::TooManyOutputs)
    })?;
    let command_count = output_count
        .checked_add(colored_instances)
        .and_then(|count| count.checked_add(1))
        .ok_or(GlbImportError::TooManyOutputs)?;
    if output_count > MAX_IMPORT_OUTPUTS || command_count > MAX_GLB_COMMANDS {
        return Err(GlbImportError::TooManyOutputs);
    }

    bump(
        &mut diagnostic_counts,
        "glb_mesh_hierarchy_transforms_and_instances_preserved",
        u32::try_from(instance_count).map_err(|_| GlbImportError::TooManyNodes)?,
    )?;
    let mut diagnostics = diagnostic_counts
        .into_iter()
        .map(|(code, count)| {
            let severity = if code == "glb_mesh_hierarchy_transforms_and_instances_preserved" {
                ImportDiagnosticSeverity::Info
            } else {
                ImportDiagnosticSeverity::Warning
            };
            ImportDiagnostic::new(severity, code, None, count)
                .map_err(|_| GlbImportError::InvalidGeometry)
        })
        .collect::<Result<Vec<_>, _>>()?;
    diagnostics.sort();

    Ok(ParsedGlbScene {
        primitives,
        nodes,
        traversal,
        diagnostics,
        instance_count,
        triangle_count: total_triangles,
    })
}

pub fn plan_glb_import(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
) -> Result<CommandBatch, GlbImportError> {
    validate_text(source_name)?;
    if source_name.contains(['/', '\\']) {
        return Err(GlbImportError::InvalidSourceIdentity);
    }
    let scene = inspect_glb(source)?;
    let import_id = snapshot
        .next_import_id()
        .map_err(|_| GlbImportError::IdSpaceExhausted)?;
    let definition_start = next_id(snapshot.definitions().map(|item| item.id().0))?;
    let feature_start = next_id(snapshot.features().map(|item| item.id().0))?;
    let occurrence_start = next_id(snapshot.occurrences().map(|item| item.id().0))?;
    let group_start = next_id(snapshot.groups().map(|item| item.id().0))?;

    let mut commands = Vec::new();
    let mut outputs = Vec::new();
    let mut definition_ids = Vec::with_capacity(scene.primitives.len());
    for (offset, primitive) in scene.primitives.iter().enumerate() {
        let offset = u64::try_from(offset).map_err(|_| GlbImportError::IdSpaceExhausted)?;
        let definition_id = DefinitionId(
            definition_start
                .checked_add(offset)
                .ok_or(GlbImportError::IdSpaceExhausted)?,
        );
        let feature_id = FeatureId(
            feature_start
                .checked_add(offset)
                .ok_or(GlbImportError::IdSpaceExhausted)?,
        );
        definition_ids.push(definition_id);
        outputs.push(ImportOutputRef::Definition(definition_id));
        outputs.push(ImportOutputRef::Feature(feature_id));
        commands.push(CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: primitive.name.clone(),
        });
        commands.push(CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: "GLB mesh".to_owned(),
            kind: FeatureKind::MeshBody(MeshBodySpec {
                schema: MESH_BODY_SCHEMA_V1.to_owned(),
                vertices_mm: primitive.vertices_mm.clone(),
                triangles: primitive.triangles.clone(),
                authority: MeshAuthority::ImportedGlb { import_id },
            }),
        });
    }

    let mut group_ids = vec![None; scene.nodes.len()];
    for (offset, node_index) in scene.traversal.iter().copied().enumerate() {
        let id = GroupId(
            group_start
                .checked_add(u64::try_from(offset).map_err(|_| GlbImportError::IdSpaceExhausted)?)
                .ok_or(GlbImportError::IdSpaceExhausted)?,
        );
        group_ids[node_index] = Some(id);
    }
    let mut parent_by_node = vec![None; scene.nodes.len()];
    for parent_index in &scene.traversal {
        for child in &scene.nodes[*parent_index].children {
            parent_by_node[*child] = Some(*parent_index);
        }
    }
    for node_index in &scene.traversal {
        let node = &scene.nodes[*node_index];
        let id = group_ids[*node_index].ok_or(GlbImportError::InvalidHierarchy)?;
        let parent = parent_by_node[*node_index]
            .map(|index| group_ids[index].expect("parent precedes child in validated traversal"));
        outputs.push(ImportOutputRef::Group(id));
        commands.push(CanonicalCommand::CreateGroup {
            id,
            name: node.name.clone(),
            transform: node.transform,
            parent,
        });
    }

    let mut occurrence_offset = 0_u64;
    for node_index in &scene.traversal {
        let node = &scene.nodes[*node_index];
        let parent = group_ids[*node_index].ok_or(GlbImportError::InvalidHierarchy)?;
        for primitive_index in &node.primitives {
            let occurrence_id = OccurrenceId(
                occurrence_start
                    .checked_add(occurrence_offset)
                    .ok_or(GlbImportError::IdSpaceExhausted)?,
            );
            occurrence_offset = occurrence_offset
                .checked_add(1)
                .ok_or(GlbImportError::IdSpaceExhausted)?;
            let primitive = &scene.primitives[*primitive_index];
            outputs.push(ImportOutputRef::Occurrence(occurrence_id));
            commands.push(CanonicalCommand::CreateOccurrence {
                id: occurrence_id,
                definition_id: definition_ids[*primitive_index],
                name: if node.primitives.len() == 1 {
                    node.name.clone()
                } else {
                    format!("{} / {}", node.name, primitive.name)
                },
                transform: Transform::identity(),
                parent: Some(parent),
                tag: None,
                visible: true,
            });
            if let Some(color) = primitive.color {
                commands.push(CanonicalCommand::SetOccurrenceColor {
                    id: occurrence_id,
                    color: Some(color),
                });
            }
        }
    }
    outputs.sort();
    let receipt = ImportReceipt::from_source_bytes(
        import_id,
        ImportFormat::Glb,
        source,
        source_name,
        ImportUnitDecision::new(ImportLengthUnit::Metre, ImportUnitAuthority::FileDeclared),
        GLB_PARSER_ID,
        GLB_PARSER_VERSION,
        scene.diagnostics,
        outputs,
    )
    .map_err(|_| GlbImportError::InvalidSourceIdentity)?;
    commands.push(CanonicalCommand::RecordImport(receipt));
    let batch = CommandBatch::new(commands);
    snapshot
        .preview_batch(&batch)
        .map_err(|_| GlbImportError::InvalidGeometry)?;
    Ok(batch)
}

fn parse_container(source: &[u8]) -> Result<(&[u8], &[u8]), GlbImportError> {
    if source.len() < 20 {
        return Err(GlbImportError::InvalidContainer);
    }
    if read_u32(source, 0)? != GLB_MAGIC {
        return Err(GlbImportError::InvalidContainer);
    }
    if read_u32(source, 4)? != 2 {
        return Err(GlbImportError::UnsupportedVersion);
    }
    if usize::try_from(read_u32(source, 8)?).ok() != Some(source.len()) {
        return Err(GlbImportError::InvalidContainer);
    }
    let mut offset = 12_usize;
    let mut json = None;
    let mut binary = None;
    let mut chunk_index = 0_usize;
    while offset < source.len() {
        let header_end = offset
            .checked_add(8)
            .ok_or(GlbImportError::InvalidContainer)?;
        if header_end > source.len() {
            return Err(GlbImportError::InvalidContainer);
        }
        let length = usize::try_from(read_u32(source, offset)?)
            .map_err(|_| GlbImportError::InvalidContainer)?;
        let kind = read_u32(source, offset + 4)?;
        if !length.is_multiple_of(4) {
            return Err(GlbImportError::InvalidContainer);
        }
        let start = header_end;
        let end = start
            .checked_add(length)
            .ok_or(GlbImportError::InvalidContainer)?;
        if end > source.len() {
            return Err(GlbImportError::InvalidContainer);
        }
        match kind {
            JSON_CHUNK if chunk_index == 0 && json.is_none() => json = Some(&source[start..end]),
            BIN_CHUNK if json.is_some() && binary.is_none() => binary = Some(&source[start..end]),
            _ => return Err(GlbImportError::InvalidContainer),
        }
        offset = end;
        chunk_index += 1;
    }
    if offset != source.len() {
        return Err(GlbImportError::InvalidContainer);
    }
    Ok((
        json.ok_or(GlbImportError::InvalidContainer)?,
        binary.ok_or(GlbImportError::InvalidContainer)?,
    ))
}
fn parse_buffer_views(
    root: &Map<String, Value>,
    binary_length: usize,
) -> Result<Vec<BufferView>, GlbImportError> {
    let values = array_required(root, "bufferViews")?;
    if values.len() > MAX_PRIMITIVES * 4 {
        return Err(GlbImportError::TooManyMeshes);
    }
    values
        .iter()
        .map(|value| {
            let value = object(value)?;
            if usize_value(value, "buffer")? != 0 {
                return Err(GlbImportError::UnsupportedFeature);
            }
            let offset = optional_usize(value, "byteOffset")?.unwrap_or(0);
            let length = usize_value(value, "byteLength")?;
            let stride = optional_usize(value, "byteStride")?;
            if length == 0
                || stride.is_some_and(|stride| !(4..=252).contains(&stride))
                || offset
                    .checked_add(length)
                    .is_none_or(|end| end > binary_length)
            {
                return Err(GlbImportError::InvalidAccessor);
            }
            Ok(BufferView {
                offset,
                length,
                stride,
            })
        })
        .collect()
}

fn parse_accessors(
    root: &Map<String, Value>,
    views: &[BufferView],
    binary_length: usize,
) -> Result<Vec<Accessor>, GlbImportError> {
    let values = array_required(root, "accessors")?;
    if values.len() > MAX_PRIMITIVES * 2 {
        return Err(GlbImportError::TooManyMeshes);
    }
    values
        .iter()
        .map(|value| {
            let value = object(value)?;
            if value.contains_key("sparse")
                || value
                    .get("normalized")
                    .is_some_and(|value| !value.is_boolean())
            {
                return Err(GlbImportError::UnsupportedFeature);
            }
            let view = usize_value(value, "bufferView")?;
            let buffer_view = views.get(view).ok_or(GlbImportError::InvalidReference)?;
            let offset = optional_usize(value, "byteOffset")?.unwrap_or(0);
            let component_type = u64_value(value, "componentType")?;
            let count = usize_value(value, "count")?;
            let kind = match string_value(value, "type")? {
                "SCALAR" => AccessorKind::Scalar,
                "VEC3" => AccessorKind::Vec3,
                "VEC2" => AccessorKind::Other(2),
                "VEC4" | "MAT2" => AccessorKind::Other(4),
                "MAT3" => AccessorKind::Other(9),
                "MAT4" => AccessorKind::Other(16),
                _ => return Err(GlbImportError::InvalidAccessor),
            };
            if count == 0 {
                return Err(GlbImportError::InvalidAccessor);
            }
            let component_size = component_size(component_type)?;
            let components = match kind {
                AccessorKind::Scalar => 1,
                AccessorKind::Vec3 => 3,
                AccessorKind::Other(components) => components,
            };
            let element_size = component_size
                .checked_mul(components)
                .ok_or(GlbImportError::InvalidAccessor)?;
            let stride = buffer_view.stride.unwrap_or(element_size);
            let start = buffer_view
                .offset
                .checked_add(offset)
                .ok_or(GlbImportError::InvalidAccessor)?;
            let end = count
                .checked_sub(1)
                .and_then(|last| last.checked_mul(stride))
                .and_then(|last| start.checked_add(last))
                .and_then(|last| last.checked_add(element_size))
                .ok_or(GlbImportError::InvalidAccessor)?;
            let view_end = buffer_view
                .offset
                .checked_add(buffer_view.length)
                .ok_or(GlbImportError::InvalidAccessor)?;
            if offset % component_size != 0
                || stride < element_size
                || stride % component_size != 0
                || end > view_end
                || end > binary_length
            {
                return Err(GlbImportError::InvalidAccessor);
            }
            Ok(Accessor {
                view,
                offset,
                component_type,
                count,
                kind,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn parse_meshes(
    root: &Map<String, Value>,
    binary: &[u8],
    views: &[BufferView],
    accessors: &[Accessor],
    material_colors: &[Option<[u8; 3]>],
    selected_meshes: &BTreeSet<usize>,
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<ParsedMeshes, GlbImportError> {
    let meshes = array_required(root, "meshes")?;
    if meshes.is_empty() {
        return Err(GlbImportError::EmptyScene);
    }
    if meshes.len() > MAX_MESHES {
        return Err(GlbImportError::TooManyMeshes);
    }
    if selected_meshes.iter().any(|index| *index >= meshes.len()) {
        return Err(GlbImportError::InvalidReference);
    }
    if meshes.len() > selected_meshes.len() {
        bump(
            diagnostics,
            "glb_unselected_meshes_ignored",
            u32::try_from(meshes.len() - selected_meshes.len())
                .map_err(|_| GlbImportError::TooManyMeshes)?,
        )?;
    }
    let mut parsed = Vec::new();
    let mut mesh_primitives = Vec::with_capacity(meshes.len());
    let mut total_vertices = 0_usize;
    let mut total_triangles = 0_usize;
    for (mesh_index, mesh) in meshes.iter().enumerate() {
        if !selected_meshes.contains(&mesh_index) {
            mesh_primitives.push(Vec::new());
            continue;
        }
        let mesh = object(mesh)?;
        let mesh_name = optional_name(mesh, "name", "GLB mesh", mesh_index)?;
        let primitives = array_required(mesh, "primitives")?;
        if primitives.is_empty()
            || parsed
                .len()
                .checked_add(primitives.len())
                .is_none_or(|count| count > MAX_PRIMITIVES)
        {
            return Err(GlbImportError::TooManyMeshes);
        }
        let mut primitive_indices = Vec::with_capacity(primitives.len());
        for (primitive_index, primitive) in primitives.iter().enumerate() {
            let primitive = object(primitive)?;
            if primitive.contains_key("targets") || primitive.contains_key("extensions") {
                return Err(GlbImportError::UnsupportedFeature);
            }
            if optional_u64(primitive, "mode")?.unwrap_or(4) != 4 {
                return Err(GlbImportError::UnsupportedFeature);
            }
            let attributes = object(
                primitive
                    .get("attributes")
                    .ok_or(GlbImportError::InvalidReference)?,
            )?;
            let position_accessor = usize_value(attributes, "POSITION")?;
            if attributes.keys().any(|name| name != "POSITION") {
                bump(diagnostics, "glb_vertex_attributes_ignored", 1)?;
            }
            let index_accessor = usize_value(primitive, "indices")?;
            let vertices_mm = decode_positions(position_accessor, binary, views, accessors)?;
            let triangles =
                decode_triangles(index_accessor, vertices_mm.len(), binary, views, accessors)?;
            let (vertices_mm, triangles) = weld_indexed_geometry(vertices_mm, triangles)?;
            total_vertices = total_vertices
                .checked_add(vertices_mm.len())
                .ok_or(GlbImportError::TooManyVertices)?;
            total_triangles = total_triangles
                .checked_add(triangles.len())
                .ok_or(GlbImportError::TooManyTriangles)?;
            if total_vertices > MAX_TOTAL_VERTICES {
                return Err(GlbImportError::TooManyVertices);
            }
            if total_triangles > MAX_TOTAL_TRIANGLES {
                return Err(GlbImportError::TooManyTriangles);
            }
            let color = optional_usize(primitive, "material")?
                .map(|index| {
                    material_colors
                        .get(index)
                        .copied()
                        .ok_or(GlbImportError::InvalidReference)
                })
                .transpose()?
                .flatten();
            let name = if primitives.len() == 1 {
                mesh_name.clone()
            } else {
                let name = format!("{mesh_name} / primitive {}", primitive_index + 1);
                validate_text(&name)?;
                name
            };
            primitive_indices.push(parsed.len());
            parsed.push(ParsedPrimitive {
                name,
                vertices_mm,
                triangles,
                color,
            });
        }
        mesh_primitives.push(primitive_indices);
    }
    Ok((parsed, mesh_primitives, total_triangles))
}

fn weld_indexed_geometry(
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
) -> Result<IndexedGeometry, GlbImportError> {
    let mut index_by_position = BTreeMap::<[u64; 3], u32>::new();
    let mut welded_vertices = Vec::new();
    let mut welded_triangles = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        let mut welded = [0_u32; 3];
        for (slot, source_index) in triangle.into_iter().enumerate() {
            let point = *vertices
                .get(source_index as usize)
                .ok_or(GlbImportError::InvalidGeometry)?;
            let key = point.map(|value| {
                if value == 0.0 {
                    0.0_f64.to_bits()
                } else {
                    value.to_bits()
                }
            });
            welded[slot] = if let Some(index) = index_by_position.get(&key) {
                *index
            } else {
                let index = u32::try_from(welded_vertices.len())
                    .map_err(|_| GlbImportError::TooManyVertices)?;
                welded_vertices.push(point);
                index_by_position.insert(key, index);
                index
            };
        }
        if welded[0] == welded[1] || welded[1] == welded[2] || welded[0] == welded[2] {
            return Err(GlbImportError::InvalidGeometry);
        }
        welded_triangles.push(welded);
    }
    if welded_vertices.len() < 4 {
        return Err(GlbImportError::InvalidGeometry);
    }
    Ok((welded_vertices, welded_triangles))
}

fn decode_positions(
    accessor_index: usize,
    binary: &[u8],
    views: &[BufferView],
    accessors: &[Accessor],
) -> Result<Vec<[f64; 3]>, GlbImportError> {
    let accessor = accessors
        .get(accessor_index)
        .ok_or(GlbImportError::InvalidReference)?;
    if accessor.kind != AccessorKind::Vec3
        || accessor.component_type != FLOAT
        || accessor.count > MAX_VERTICES_PER_PRIMITIVE
        || accessor.count < 4
    {
        return Err(GlbImportError::InvalidAccessor);
    }
    let view = views
        .get(accessor.view)
        .ok_or(GlbImportError::InvalidReference)?;
    let stride = view.stride.unwrap_or(12);
    let start = view
        .offset
        .checked_add(accessor.offset)
        .ok_or(GlbImportError::InvalidAccessor)?;
    let mut vertices = Vec::with_capacity(accessor.count);
    for index in 0..accessor.count {
        let offset = start
            .checked_add(
                index
                    .checked_mul(stride)
                    .ok_or(GlbImportError::InvalidAccessor)?,
            )
            .ok_or(GlbImportError::InvalidAccessor)?;
        let gltf = [
            read_f32(binary, offset)?,
            read_f32(binary, offset + 4)?,
            read_f32(binary, offset + 8)?,
        ];
        let point = [
            f64::from(gltf[0]) * 1_000.0,
            -f64::from(gltf[2]) * 1_000.0,
            f64::from(gltf[1]) * 1_000.0,
        ];
        if point
            .iter()
            .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_ABS_MM)
        {
            return Err(GlbImportError::InvalidGeometry);
        }
        vertices.push(point);
    }
    Ok(vertices)
}

fn decode_triangles(
    accessor_index: usize,
    vertex_count: usize,
    binary: &[u8],
    views: &[BufferView],
    accessors: &[Accessor],
) -> Result<Vec<[u32; 3]>, GlbImportError> {
    let accessor = accessors
        .get(accessor_index)
        .ok_or(GlbImportError::InvalidReference)?;
    if accessor.kind != AccessorKind::Scalar
        || !matches!(
            accessor.component_type,
            UNSIGNED_BYTE | UNSIGNED_SHORT | UNSIGNED_INT
        )
        || accessor.count % 3 != 0
        || !(12..=MAX_TRIANGLES_PER_PRIMITIVE * 3).contains(&accessor.count)
    {
        return Err(GlbImportError::InvalidAccessor);
    }
    let view = views
        .get(accessor.view)
        .ok_or(GlbImportError::InvalidReference)?;
    let width = component_size(accessor.component_type)?;
    let stride = view.stride.unwrap_or(width);
    let start = view
        .offset
        .checked_add(accessor.offset)
        .ok_or(GlbImportError::InvalidAccessor)?;
    let mut indices = Vec::with_capacity(accessor.count);
    for index in 0..accessor.count {
        let offset = start
            .checked_add(
                index
                    .checked_mul(stride)
                    .ok_or(GlbImportError::InvalidAccessor)?,
            )
            .ok_or(GlbImportError::InvalidAccessor)?;
        let value = match accessor.component_type {
            UNSIGNED_BYTE => u32::from(*binary.get(offset).ok_or(GlbImportError::InvalidAccessor)?),
            UNSIGNED_SHORT => u32::from(read_u16(binary, offset)?),
            UNSIGNED_INT => read_u32(binary, offset)?,
            _ => return Err(GlbImportError::InvalidAccessor),
        };
        if value as usize >= vertex_count {
            return Err(GlbImportError::InvalidGeometry);
        }
        indices.push(value);
    }
    Ok(indices
        .chunks_exact(3)
        .map(|triangle| [triangle[0], triangle[1], triangle[2]])
        .collect())
}

fn select_scene(root: &Map<String, Value>) -> Result<SceneSelection, GlbImportError> {
    let nodes = array_required(root, "nodes")?;
    if nodes.is_empty() || nodes.len() > MAX_NODES {
        return Err(if nodes.is_empty() {
            GlbImportError::EmptyScene
        } else {
            GlbImportError::TooManyNodes
        });
    }
    let scenes = array_required(root, "scenes")?;
    if scenes.is_empty() {
        return Err(GlbImportError::EmptyScene);
    }
    let scene_index = optional_usize(root, "scene")?.unwrap_or(0);
    let scene = object(
        scenes
            .get(scene_index)
            .ok_or(GlbImportError::InvalidReference)?,
    )?;
    let roots = array_required(scene, "nodes")?
        .iter()
        .map(value_as_usize)
        .collect::<Result<Vec<_>, _>>()?;
    if roots.is_empty()
        || roots.iter().any(|root| *root >= nodes.len())
        || roots.iter().copied().collect::<BTreeSet<_>>().len() != roots.len()
    {
        return Err(GlbImportError::InvalidHierarchy);
    }
    let mut state = vec![0_u8; nodes.len()];
    let mut selected = BTreeSet::new();
    for root in roots {
        visit_raw_node(root, nodes, &mut state, &mut selected)?;
    }
    let mut meshes = BTreeSet::new();
    for index in &selected {
        let node = object(&nodes[*index])?;
        if let Some(mesh) = optional_usize(node, "mesh")? {
            meshes.insert(mesh);
        }
    }
    Ok(SceneSelection {
        nodes: selected,
        meshes,
    })
}

fn visit_raw_node(
    index: usize,
    nodes: &[Value],
    state: &mut [u8],
    selected: &mut BTreeSet<usize>,
) -> Result<(), GlbImportError> {
    match state.get(index).copied() {
        Some(0) => {}
        Some(_) => return Err(GlbImportError::InvalidHierarchy),
        None => return Err(GlbImportError::InvalidReference),
    }
    state[index] = 1;
    selected.insert(index);
    let node = object(&nodes[index])?;
    let children = optional_array(node, "children")?
        .iter()
        .map(value_as_usize)
        .collect::<Result<Vec<_>, _>>()?;
    if children.iter().copied().collect::<BTreeSet<_>>().len() != children.len()
        || children
            .iter()
            .any(|child| *child >= nodes.len() || *child == index)
    {
        return Err(GlbImportError::InvalidHierarchy);
    }
    for child in children {
        visit_raw_node(child, nodes, state, selected)?;
    }
    state[index] = 2;
    Ok(())
}

fn parse_nodes(
    root: &Map<String, Value>,
    mesh_primitives: &[Vec<usize>],
    selected_nodes: &BTreeSet<usize>,
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<(Vec<ParsedNode>, Vec<usize>, usize), GlbImportError> {
    let values = array_required(root, "nodes")?;
    if values.is_empty() || values.len() > MAX_NODES {
        return Err(if values.is_empty() {
            GlbImportError::EmptyScene
        } else {
            GlbImportError::TooManyNodes
        });
    }
    let mut nodes = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        if !selected_nodes.contains(&index) {
            nodes.push(ParsedNode {
                name: "Unselected GLB node".to_owned(),
                transform: Transform::identity(),
                children: Vec::new(),
                primitives: Vec::new(),
            });
            continue;
        }
        let value = object(value)?;
        if value.contains_key("skin") || value.contains_key("weights") {
            return Err(GlbImportError::UnsupportedFeature);
        }
        if value.contains_key("camera") {
            bump(diagnostics, "glb_cameras_ignored", 1)?;
        }
        let name = optional_name(value, "name", "GLB node", index)?;
        let transform = parse_node_transform(value)?;
        let children = optional_array(value, "children")?
            .iter()
            .map(value_as_usize)
            .collect::<Result<Vec<_>, _>>()?;
        let unique_children = children.iter().copied().collect::<BTreeSet<_>>();
        if unique_children.len() != children.len()
            || children
                .iter()
                .any(|child| *child >= values.len() || *child == index)
        {
            return Err(GlbImportError::InvalidHierarchy);
        }
        let primitives = optional_usize(value, "mesh")?
            .map(|mesh| {
                mesh_primitives
                    .get(mesh)
                    .cloned()
                    .ok_or(GlbImportError::InvalidReference)
            })
            .transpose()?
            .unwrap_or_default();
        nodes.push(ParsedNode {
            name,
            transform,
            children,
            primitives,
        });
    }

    let scenes = array_required(root, "scenes")?;
    if scenes.is_empty() {
        return Err(GlbImportError::EmptyScene);
    }
    let scene_index = optional_usize(root, "scene")?.unwrap_or(0);
    let scene = object(
        scenes
            .get(scene_index)
            .ok_or(GlbImportError::InvalidReference)?,
    )?;
    let roots = array_required(scene, "nodes")?
        .iter()
        .map(value_as_usize)
        .collect::<Result<Vec<_>, _>>()?;
    if roots.is_empty()
        || roots.iter().any(|root| *root >= nodes.len())
        || roots.iter().copied().collect::<BTreeSet<_>>().len() != roots.len()
    {
        return Err(GlbImportError::InvalidHierarchy);
    }
    let mut state = vec![0_u8; nodes.len()];
    let mut traversal = Vec::new();
    for root in roots {
        visit_node(root, &nodes, &mut state, &mut traversal)?;
    }
    let ignored = nodes.len() - traversal.len();
    if ignored > 0 {
        bump(
            diagnostics,
            "glb_unselected_nodes_ignored",
            u32::try_from(ignored).map_err(|_| GlbImportError::TooManyNodes)?,
        )?;
    }
    let instance_count = traversal.iter().try_fold(0_usize, |count, index| {
        count
            .checked_add(nodes[*index].primitives.len())
            .ok_or(GlbImportError::TooManyNodes)
    })?;
    Ok((nodes, traversal, instance_count))
}

fn visit_node(
    index: usize,
    nodes: &[ParsedNode],
    state: &mut [u8],
    traversal: &mut Vec<usize>,
) -> Result<(), GlbImportError> {
    match state.get(index).copied() {
        Some(0) => {}
        Some(_) => return Err(GlbImportError::InvalidHierarchy),
        None => return Err(GlbImportError::InvalidReference),
    }
    state[index] = 1;
    traversal.push(index);
    for child in &nodes[index].children {
        visit_node(*child, nodes, state, traversal)?;
    }
    state[index] = 2;
    Ok(())
}

fn parse_node_transform(node: &Map<String, Value>) -> Result<Transform, GlbImportError> {
    let has_matrix = node.contains_key("matrix");
    let has_trs = ["translation", "rotation", "scale"]
        .iter()
        .any(|name| node.contains_key(*name));
    if has_matrix && has_trs {
        return Err(GlbImportError::InvalidTransform);
    }
    let gltf = if has_matrix {
        let values = number_array(node, "matrix", 16)?;
        let mut row_major = [0.0; 16];
        for row in 0..4 {
            for column in 0..4 {
                row_major[row * 4 + column] = values[column * 4 + row];
            }
        }
        row_major
    } else {
        let translation = optional_number_array(node, "translation", 3)?
            .map(|values| [values[0], values[1], values[2]])
            .unwrap_or([0.0; 3]);
        let rotation = optional_number_array(node, "rotation", 4)?
            .map(|values| [values[0], values[1], values[2], values[3]])
            .unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let scale = optional_number_array(node, "scale", 3)?
            .map(|values| [values[0], values[1], values[2]])
            .unwrap_or([1.0; 3]);
        trs_matrix(translation, rotation, scale)?
    };
    gltf_transform_to_ketchup(gltf)
}

fn trs_matrix(
    translation: [f64; 3],
    quaternion: [f64; 4],
    scale: [f64; 3],
) -> Result<[f64; 16], GlbImportError> {
    if translation
        .iter()
        .chain(quaternion.iter())
        .chain(scale.iter())
        .any(|value| !value.is_finite())
        || scale.iter().any(|value| value.abs() <= f64::EPSILON)
    {
        return Err(GlbImportError::InvalidTransform);
    }
    let [x, y, z, w] = quaternion;
    let norm = x * x + y * y + z * z + w * w;
    if !norm.is_finite() || (norm - 1.0).abs() > 1.0e-6 {
        return Err(GlbImportError::InvalidTransform);
    }
    let rotation = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y - z * w),
        2.0 * (x * z + y * w),
        0.0,
        2.0 * (x * y + z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z - x * w),
        0.0,
        2.0 * (x * z - y * w),
        2.0 * (y * z + x * w),
        1.0 - 2.0 * (x * x + y * y),
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    let mut matrix = rotation;
    for row in 0..3 {
        for column in 0..3 {
            matrix[row * 4 + column] *= scale[column];
        }
    }
    matrix[3] = translation[0];
    matrix[7] = translation[1];
    matrix[11] = translation[2];
    Ok(matrix)
}

fn gltf_transform_to_ketchup(gltf: [f64; 16]) -> Result<Transform, GlbImportError> {
    if gltf.iter().any(|value| !value.is_finite())
        || gltf[12].abs() > f64::EPSILON
        || gltf[13].abs() > f64::EPSILON
        || gltf[14].abs() > f64::EPSILON
        || (gltf[15] - 1.0).abs() > f64::EPSILON
    {
        return Err(GlbImportError::InvalidTransform);
    }
    let to_gltf = [
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let to_ketchup = [
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut matrix = multiply_matrix(multiply_matrix(to_ketchup, gltf), to_gltf);
    matrix[3] *= 1_000.0;
    matrix[7] *= 1_000.0;
    matrix[11] *= 1_000.0;
    let determinant = matrix[0] * (matrix[5] * matrix[10] - matrix[6] * matrix[9])
        - matrix[1] * (matrix[4] * matrix[10] - matrix[6] * matrix[8])
        + matrix[2] * (matrix[4] * matrix[9] - matrix[5] * matrix[8]);
    if matrix
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
        || determinant.abs() <= f64::EPSILON
    {
        return Err(GlbImportError::InvalidTransform);
    }
    Transform::from_matrix(matrix).map_err(|_| GlbImportError::InvalidTransform)
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

fn validate_asset(root: &Map<String, Value>) -> Result<(), GlbImportError> {
    let asset = object(
        root.get("asset")
            .ok_or(GlbImportError::UnsupportedVersion)?,
    )?;
    if parse_version(string_value(asset, "version")?)? != (2, 0) {
        return Err(GlbImportError::UnsupportedVersion);
    }
    if let Some(minimum) = asset.get("minVersion") {
        let minimum = minimum.as_str().ok_or(GlbImportError::UnsupportedVersion)?;
        if parse_version(minimum)? > (2, 0) {
            return Err(GlbImportError::UnsupportedVersion);
        }
    }
    Ok(())
}

fn parse_version(value: &str) -> Result<(u32, u32), GlbImportError> {
    let mut parts = value.split('.');
    let major = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .ok_or(GlbImportError::UnsupportedVersion)?;
    let minor = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .ok_or(GlbImportError::UnsupportedVersion)?;
    if parts.next().is_some() {
        return Err(GlbImportError::UnsupportedVersion);
    }
    Ok((major, minor))
}

fn reject_required_extensions(root: &Map<String, Value>) -> Result<(), GlbImportError> {
    let required = optional_array(root, "extensionsRequired")?;
    if required.is_empty() {
        Ok(())
    } else {
        Err(GlbImportError::UnsupportedFeature)
    }
}

fn record_document_losses(
    root: &Map<String, Value>,
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<(), GlbImportError> {
    for (name, code) in [
        ("animations", "glb_animations_ignored"),
        ("cameras", "glb_camera_definitions_ignored"),
        ("images", "glb_images_ignored"),
        ("textures", "glb_textures_ignored"),
        ("samplers", "glb_samplers_ignored"),
    ] {
        let values = optional_array(root, name)?;
        if !values.is_empty() {
            bump(
                diagnostics,
                code,
                u32::try_from(values.len()).map_err(|_| GlbImportError::UnsupportedFeature)?,
            )?;
        }
    }
    let used = optional_array(root, "extensionsUsed")?;
    if !used.is_empty() {
        bump(
            diagnostics,
            "glb_optional_extensions_ignored",
            u32::try_from(used.len()).map_err(|_| GlbImportError::UnsupportedFeature)?,
        )?;
    }
    Ok(())
}

fn parse_materials(
    materials: &[Value],
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<Vec<Option<[u8; 3]>>, GlbImportError> {
    materials
        .iter()
        .map(|material| {
            let material = object(material)?;
            if material
                .get("alphaMode")
                .is_some_and(|mode| mode.as_str() != Some("OPAQUE"))
            {
                return Err(GlbImportError::UnsupportedFeature);
            }
            if material
                .get("doubleSided")
                .is_some_and(|value| value != &Value::Bool(false))
            {
                bump(diagnostics, "glb_double_sided_material_ignored", 1)?;
            }
            if material.contains_key("extensions") {
                bump(diagnostics, "glb_material_extensions_ignored", 1)?;
            }
            for field in ["normalTexture", "occlusionTexture", "emissiveTexture"] {
                if material.contains_key(field) {
                    bump(diagnostics, "glb_material_textures_ignored", 1)?;
                }
            }
            if material.contains_key("emissiveFactor") {
                bump(diagnostics, "glb_emissive_color_ignored", 1)?;
            }
            let pbr = material
                .get("pbrMetallicRoughness")
                .map(object)
                .transpose()?;
            if pbr.is_some_and(|pbr| pbr.contains_key("baseColorTexture")) {
                bump(diagnostics, "glb_material_textures_ignored", 1)?;
            }
            let factor = pbr
                .and_then(|pbr| pbr.get("baseColorFactor"))
                .map(|value| exact_number_array(value, 4))
                .transpose()?
                .unwrap_or_else(|| vec![1.0; 4]);
            if factor
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
                || (factor[3] - 1.0).abs() > f64::EPSILON
            {
                return Err(GlbImportError::UnsupportedFeature);
            }
            Ok(Some([
                linear_to_srgb_byte(factor[0]),
                linear_to_srgb_byte(factor[1]),
                linear_to_srgb_byte(factor[2]),
            ]))
        })
        .collect()
}

fn linear_to_srgb_byte(value: f64) -> u8 {
    let srgb = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn component_size(component_type: u64) -> Result<usize, GlbImportError> {
    match component_type {
        BYTE | UNSIGNED_BYTE => Ok(1),
        SHORT | UNSIGNED_SHORT => Ok(2),
        FLOAT | UNSIGNED_INT => Ok(4),
        _ => Err(GlbImportError::InvalidAccessor),
    }
}

fn object(value: &Value) -> Result<&Map<String, Value>, GlbImportError> {
    value.as_object().ok_or(GlbImportError::InvalidJson)
}

fn array_required<'a>(
    object: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a [Value], GlbImportError> {
    object
        .get(name)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or(GlbImportError::InvalidJson)
}

fn optional_array<'a>(
    object: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a [Value], GlbImportError> {
    match object.get(name) {
        None => Ok(&[]),
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or(GlbImportError::InvalidJson),
    }
}

fn string_value<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str, GlbImportError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or(GlbImportError::InvalidJson)
}

fn u64_value(object: &Map<String, Value>, name: &str) -> Result<u64, GlbImportError> {
    object
        .get(name)
        .and_then(Value::as_u64)
        .ok_or(GlbImportError::InvalidJson)
}

fn optional_u64(object: &Map<String, Value>, name: &str) -> Result<Option<u64>, GlbImportError> {
    object
        .get(name)
        .map(|value| value.as_u64().ok_or(GlbImportError::InvalidJson))
        .transpose()
}

fn usize_value(object: &Map<String, Value>, name: &str) -> Result<usize, GlbImportError> {
    usize::try_from(u64_value(object, name)?).map_err(|_| GlbImportError::InvalidJson)
}

fn optional_usize(
    object: &Map<String, Value>,
    name: &str,
) -> Result<Option<usize>, GlbImportError> {
    optional_u64(object, name)?
        .map(|value| usize::try_from(value).map_err(|_| GlbImportError::InvalidJson))
        .transpose()
}

fn value_as_usize(value: &Value) -> Result<usize, GlbImportError> {
    value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(GlbImportError::InvalidJson)
}

fn number_array(
    object: &Map<String, Value>,
    name: &str,
    length: usize,
) -> Result<Vec<f64>, GlbImportError> {
    exact_number_array(object.get(name).ok_or(GlbImportError::InvalidJson)?, length)
}

fn optional_number_array(
    object: &Map<String, Value>,
    name: &str,
    length: usize,
) -> Result<Option<Vec<f64>>, GlbImportError> {
    object
        .get(name)
        .map(|value| exact_number_array(value, length))
        .transpose()
}

fn exact_number_array(value: &Value, length: usize) -> Result<Vec<f64>, GlbImportError> {
    let values = value.as_array().ok_or(GlbImportError::InvalidJson)?;
    if values.len() != length {
        return Err(GlbImportError::InvalidJson);
    }
    values
        .iter()
        .map(|value| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or(GlbImportError::InvalidJson)
        })
        .collect()
}

fn optional_name(
    object: &Map<String, Value>,
    name: &str,
    fallback: &str,
    index: usize,
) -> Result<String, GlbImportError> {
    let value = match object.get(name) {
        None => format!("{fallback} {}", index + 1),
        Some(Value::String(value)) if value.is_empty() => format!("{fallback} {}", index + 1),
        Some(Value::String(value)) => value.clone(),
        Some(_) => return Err(GlbImportError::InvalidJson),
    };
    validate_text(&value)?;
    Ok(value)
}

fn validate_text(value: &str) -> Result<(), GlbImportError> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        Err(GlbImportError::InvalidText)
    } else {
        Ok(())
    }
}

fn bump(
    diagnostics: &mut BTreeMap<&'static str, u32>,
    code: &'static str,
    count: u32,
) -> Result<(), GlbImportError> {
    if count == 0 {
        return Ok(());
    }
    let value = diagnostics.entry(code).or_default();
    *value = value
        .checked_add(count)
        .ok_or(GlbImportError::UnsupportedFeature)?;
    Ok(())
}

fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, GlbImportError> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|id| *id != 0)
        .ok_or(GlbImportError::IdSpaceExhausted)
}

fn read_u16(source: &[u8], offset: usize) -> Result<u16, GlbImportError> {
    let bytes = source
        .get(offset..offset + 2)
        .ok_or(GlbImportError::InvalidAccessor)?;
    Ok(u16::from_le_bytes(
        bytes.try_into().expect("two-byte checked slice"),
    ))
}

fn read_u32(source: &[u8], offset: usize) -> Result<u32, GlbImportError> {
    let bytes = source
        .get(offset..offset + 4)
        .ok_or(GlbImportError::InvalidContainer)?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("four-byte checked slice"),
    ))
}

fn read_f32(source: &[u8], offset: usize) -> Result<f32, GlbImportError> {
    let bytes = source
        .get(offset..offset + 4)
        .ok_or(GlbImportError::InvalidAccessor)?;
    Ok(f32::from_le_bytes(
        bytes.try_into().expect("four-byte checked slice"),
    ))
}
