use crate::tolerance::{APPROXIMATION, MAX_COORDINATE_MM};
use ketchup_geometry::linalg::{Affine3, Mat3, Vec3};
use ketchup_tolerance::limits;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{Map, Value};

use super::{
    ImportContractError, ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat,
    ImportLengthUnit, ImportOutputRef, ImportReceipt, ImportUnitAuthority, ImportUnitDecision,
    MAX_IMPORT_RECEIPT_OUTPUTS,
};
use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, FeatureKind, GroupId,
    MESH_BODY_SCHEMA_V1, MESH_SHEET_SCHEMA_V1, MeshAuthority, MeshBodySpec, MeshImage, MeshLook,
    OccurrenceId, ProposalBudget, Snapshot, TexturedTriangle, Transform,
};

mod excess;
mod json;
mod tags;
mod textures;

use textures::{GlbImage, MaterialLook};

use excess::too_large;
pub use excess::{GlbExcess, GlbMeasure};
pub use tags::{GlbNodeTags, GlbTag, MAX_GLB_TAGS};

pub const GLB_PARSER_ID: &str = "ketchup-glb";
pub const GLB_PARSER_VERSION: &str = "1";

/// glTF is Y-up in metres, Kečup Z-up in millimetres: the Kečup point
/// `(x, y, z)` mm is the glTF point `(x, z, −y) / 1000`.
pub const GLTF_FROM_KETCHUP_AXES: Mat3 =
    Mat3::from_rows([[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]]);
pub const MILLIMETRES_PER_METRE: f64 = 1_000.0;

#[must_use]
pub fn gltf_point_from_ketchup(point_mm: Vec3) -> Vec3 {
    GLTF_FROM_KETCHUP_AXES.mul_vec(point_mm) / MILLIMETRES_PER_METRE
}

#[must_use]
pub fn gltf_transform_from_ketchup(transform: Affine3) -> Affine3 {
    Affine3 {
        linear: GLTF_FROM_KETCHUP_AXES * transform.linear * GLTF_FROM_KETCHUP_AXES.transpose(),
        translation: gltf_point_from_ketchup(transform.translation),
    }
}

#[must_use]
pub fn ketchup_transform_from_gltf(transform: Affine3) -> Affine3 {
    let ketchup_from_gltf = GLTF_FROM_KETCHUP_AXES.transpose();
    Affine3 {
        linear: ketchup_from_gltf * transform.linear * GLTF_FROM_KETCHUP_AXES,
        translation: ketchup_from_gltf.mul_vec(transform.translation) * MILLIMETRES_PER_METRE,
    }
}

const GLB_MAGIC: u32 = 0x4654_6c67;
const JSON_CHUNK: u32 = 0x4e4f_534a;
const BIN_CHUNK: u32 = 0x004e_4942;
const BYTE: u64 = 5_120;
const UNSIGNED_BYTE: u64 = 5_121;
const SHORT: u64 = 5_122;
const UNSIGNED_SHORT: u64 = 5_123;
const UNSIGNED_INT: u64 = 5_125;
const FLOAT: u64 = 5_126;
// What one import may cost is its geometry and its commands, not how many groups hold
// them: a SketchUp model of a small hall has thousands of groups and some thousands of
// triangles. The counts of nodes and meshes only keep the parse bounded, as many as the
// profiles one drawing import makes.
const MAX_MESHES: usize = limits::DRAWING_PROFILES;
const MAX_PRIMITIVES: usize = limits::DRAWING_PROFILES;
const MAX_NODES: usize = limits::DRAWING_PROFILES;
/// Most nodes on the path from a scene root to a node, so walking the tree stays
/// within the stack.
const MAX_NODE_DEPTH: usize = limits::INSTANCE_PATH_STEPS;
/// Farthest a GLB may place a vertex or a node from the origin of its frame, in mm: a
/// georeferenced component (an IFC element brought into SketchUp) keeps its geometry
/// kilometres away and its placement brings it back. What the import adds stays within
/// [`MAX_COORDINATE_MM`].
const MAX_SOURCE_COORDINATE_MM: f64 = 1e10;
const MAX_TOTAL_VERTICES: usize = 200_000;
const MAX_TOTAL_TRIANGLES: usize = 400_000;
/// Most triangles the placed parts show together: what an export of them writes.
const MAX_PLACED_TRIANGLES: usize = limits::EXPORT_TRIANGLES;
/// Commands of one import, one Undo step: the budget of a drawing import.
const MAX_GLB_COMMANDS: usize = ProposalBudget::DRAWING_IMPORT.max_commands;

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedGlbScene {
    primitives: Vec<ParsedPrimitive>,
    nodes: Vec<ParsedNode>,
    traversal: Vec<usize>,
    diagnostics: Vec<ImportDiagnostic>,
    instance_count: usize,
    triangle_count: usize,
    /// The GLB's images; those its meshes show come into the document as blobs.
    images: Vec<Option<GlbImage>>,
}

impl ParsedGlbScene {
    /// The bytes of each picture the imported meshes show, once: the blobs the document
    /// needs beside the import's commands ([`crate::document::MeshImage::blob`]).
    #[must_use]
    pub fn shown_images(&self) -> Vec<std::sync::Arc<Vec<u8>>> {
        self.shown_image_indices()
            .into_iter()
            .filter_map(|index| self.images[index].as_ref())
            .map(|image| std::sync::Arc::clone(&image.bytes))
            .collect()
    }

    fn shown_image_indices(&self) -> BTreeSet<usize> {
        self.primitives
            .iter()
            .filter_map(|primitive| primitive.look.as_ref())
            .flatten()
            .flatten()
            .map(TexturedTriangle::image_index)
            .collect()
    }

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

    /// The scene with the `notes` of the reader that wrote the GLB (the version of a
    /// SketchUp file and what its conversion left out) among its diagnostics.
    #[must_use]
    pub fn with_notes(mut self, notes: &[ImportDiagnostic]) -> Self {
        self.diagnostics.extend_from_slice(notes);
        self.diagnostics.sort();
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedPrimitive {
    name: String,
    /// Relative to [`Self::origin_mm`].
    vertices_mm: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    color: Option<[u8; 3]>,
    /// The picture each triangle shows, by the index of the GLB's image.
    look: Option<Vec<Option<TexturedTriangle>>>,
    /// Closes no solid: imported as a sheet (see [`MESH_SHEET_SCHEMA_V1`]).
    sheet: bool,
    /// Where the mesh's own origin moved to, in its frame: zero, or the centre of a mesh
    /// lying beyond the model space there (see [`recentre`]).
    origin_mm: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedNode {
    name: String,
    /// The node's transform as the GLB gives it.
    local: Affine3,
    /// The group's transform as imported (see [`place_nodes`]).
    transform: Transform,
    /// The transform of the part each of [`Self::primitives`] imports as.
    placements: Vec<Transform>,
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
    InvalidContainer,
    InvalidJson,
    UnsupportedVersion,
    UnsupportedFeature,
    EmptyScene,
    /// The model holds more of something than one import takes.
    TooLarge(GlbExcess),
    InvalidReference,
    InvalidAccessor,
    InvalidGeometry,
    InvalidTransform,
    InvalidHierarchy,
    InvalidText,
    InvalidSourceIdentity(ImportContractError),
    MalformedJson(String),
    InvalidReport(ImportContractError),
    RejectedByDocument(crate::document::CanonicalError),
    IdSpaceExhausted,
}

impl fmt::Display for GlbImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "GLB source is empty",
            Self::InvalidContainer => "GLB container or chunk layout is malformed",
            Self::InvalidJson => "GLB JSON document is malformed",
            Self::UnsupportedVersion => "GLB must contain a glTF 2.0 document",
            Self::UnsupportedFeature => "GLB uses a feature outside the safe mesh import subset",
            Self::EmptyScene => "GLB selected scene contains no importable mesh instances",
            Self::TooLarge(excess) => return excess.fmt(formatter),
            Self::InvalidReference => "GLB contains an out-of-range object reference",
            Self::InvalidAccessor => "GLB contains an invalid or unsupported accessor",
            Self::InvalidGeometry => "GLB contains invalid solid mesh geometry",
            Self::InvalidTransform => "GLB contains a non-finite or degenerate node transform",
            Self::InvalidHierarchy => "GLB selected scene is not a finite single-parent tree",
            Self::InvalidText => "GLB contains invalid identity text",
            Self::InvalidSourceIdentity(error) => {
                return write!(
                    formatter,
                    "GLB source name or provenance is invalid: {error}"
                );
            }
            Self::MalformedJson(reason) => {
                return write!(formatter, "GLB JSON document is malformed: {reason}");
            }
            Self::InvalidReport(error) => {
                return write!(formatter, "GLB import report is invalid: {error}");
            }
            Self::RejectedByDocument(error) => {
                return write!(formatter, "GLB result is rejected by the document: {error}");
            }
            Self::IdSpaceExhausted => "canonical import ID space is exhausted",
        })
    }
}

impl std::error::Error for GlbImportError {}

pub fn inspect_glb(source: &[u8]) -> Result<ParsedGlbScene, GlbImportError> {
    if source.is_empty() {
        return Err(GlbImportError::Empty);
    }
    let (json_bytes, binary) = parse_container(source)?;
    let json::GlbJson { root, counts } = json::read(json_bytes).map_err(|error| match error {
        json::ReadError::TooLarge => too_large(
            GlbMeasure::Description,
            json::MAX_JSON_VALUES,
            json::MAX_JSON_VALUES,
        ),
        json::ReadError::NotObject => GlbImportError::InvalidJson,
        json::ReadError::Malformed(error) => GlbImportError::MalformedJson(error.to_string()),
    })?;
    let root = &root;
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
    record_document_losses(&counts, &mut diagnostic_counts)?;
    let mut images = textures::parse_images(root, binary, &buffer_views)?;
    let material_looks = parse_materials(root, materials, &images, &mut diagnostic_counts)?;
    textures::record_unshown(root, &material_looks, &mut diagnostic_counts)?;
    let (primitives, mesh_primitives, total_triangles) = parse_meshes(
        root,
        binary,
        &buffer_views,
        &accessors,
        &material_looks,
        &selection.meshes,
        &mut images,
        &mut diagnostic_counts,
    )?;
    let (mut nodes, traversal, instance_count) = parse_nodes(
        root,
        &mesh_primitives,
        &selection.nodes,
        &mut diagnostic_counts,
    )?;
    if instance_count == 0 {
        return Err(GlbImportError::EmptyScene);
    }
    place_nodes(&mut nodes, &traversal, &primitives)?;
    let placed_triangles = traversal.iter().fold(0_usize, |count, node_index| {
        nodes[*node_index]
            .primitives
            .iter()
            .fold(count, |count, primitive| {
                count.saturating_add(primitives[*primitive].triangles.len())
            })
    });
    if placed_triangles > MAX_PLACED_TRIANGLES {
        return Err(too_large(
            GlbMeasure::PlacedTriangles,
            placed_triangles,
            MAX_PLACED_TRIANGLES,
        ));
    }
    let output_count = primitives
        .len()
        .saturating_mul(2)
        .saturating_add(traversal.len())
        .saturating_add(instance_count);
    if output_count > MAX_IMPORT_RECEIPT_OUTPUTS {
        return Err(too_large(
            GlbMeasure::Outputs,
            output_count,
            MAX_IMPORT_RECEIPT_OUTPUTS,
        ));
    }
    let colored_instances = traversal.iter().fold(0_usize, |count, node_index| {
        count.saturating_add(
            nodes[*node_index]
                .primitives
                .iter()
                .filter(|primitive| primitives[**primitive].color.is_some())
                .count(),
        )
    });
    let command_count = output_count
        .saturating_add(colored_instances)
        .saturating_add(1);
    if command_count > MAX_GLB_COMMANDS {
        return Err(too_large(
            GlbMeasure::Commands,
            command_count,
            MAX_GLB_COMMANDS,
        ));
    }

    bump(
        &mut diagnostic_counts,
        "glb_mesh_hierarchy_transforms_and_instances_preserved",
        u32::try_from(instance_count).map_err(|_out_of_range: std::num::TryFromIntError| {
            too_large(GlbMeasure::Nodes, instance_count, MAX_NODES)
        })?,
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
                .map_err(GlbImportError::InvalidReport)
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
        images,
    })
}

pub fn plan_glb_import(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
) -> Result<CommandBatch, GlbImportError> {
    plan_glb_import_noting(snapshot, source, source_name, &[])
}

/// [`plan_glb_import`] of a GLB another reader wrote, whose receipt records that
/// reader's `notes` with the GLB import's own diagnostics.
pub fn plan_glb_import_noting(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
    notes: &[ImportDiagnostic],
) -> Result<CommandBatch, GlbImportError> {
    plan_glb_import_tagged(
        snapshot,
        source,
        source_name,
        notes,
        &GlbNodeTags::default(),
    )
}

/// [`plan_glb_import_noting`] whose parts carry the `tags` of their node and of every node
/// above it; a tag the document already names is that tag, any other is created.
pub fn plan_glb_import_tagged(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
    notes: &[ImportDiagnostic],
    tags: &GlbNodeTags,
) -> Result<CommandBatch, GlbImportError> {
    validate_text(source_name)?;
    if source_name.contains(['/', '\\']) {
        return Err(GlbImportError::InvalidSourceIdentity(
            ImportContractError::InvalidSource,
        ));
    }
    let scene = inspect_glb(source)?.with_notes(notes);
    let import_id = snapshot
        .next_import_id()
        .ok_or(GlbImportError::IdSpaceExhausted)?;
    let definition_start = next_id(snapshot.definitions().map(|item| item.id().0))?;
    let feature_start = next_id(snapshot.features().map(|item| item.id().0))?;
    let occurrence_start = next_id(snapshot.occurrences().map(|item| item.id().0))?;
    let group_start = next_id(snapshot.groups().map(|item| item.id().0))?;

    let mut commands = Vec::new();
    let mut outputs = Vec::new();
    let mut definition_ids = Vec::with_capacity(scene.primitives.len());
    for (offset, primitive) in scene.primitives.iter().enumerate() {
        let offset = u64::try_from(offset)
            .map_err(|_out_of_range: std::num::TryFromIntError| GlbImportError::IdSpaceExhausted)?;
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
                look: primitive
                    .look
                    .as_deref()
                    .map(|look| mesh_look(look, &scene.images))
                    .transpose()?,
                schema: if primitive.sheet {
                    MESH_SHEET_SCHEMA_V1
                } else {
                    MESH_BODY_SCHEMA_V1
                }
                .to_owned(),
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
                .checked_add(u64::try_from(offset).map_err(
                    |_out_of_range: std::num::TryFromIntError| GlbImportError::IdSpaceExhausted,
                )?)
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

    let planned_tags = tags::plan_tags(snapshot, tags, &scene.nodes, &scene.traversal)?;
    commands.extend(planned_tags.commands);
    let mut occurrence_offset = 0_u64;
    for node_index in &scene.traversal {
        let node = &scene.nodes[*node_index];
        let parent = group_ids[*node_index].ok_or(GlbImportError::InvalidHierarchy)?;
        for (primitive_index, placement) in node.primitives.iter().zip(&node.placements) {
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
                transform: *placement,
                parent: Some(parent),
                tags: planned_tags.by_node[*node_index].clone(),
                visible: !planned_tags.hidden_by_node[*node_index],
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
    .map_err(GlbImportError::InvalidSourceIdentity)?;
    commands.push(CanonicalCommand::RecordImport(receipt));
    let batch = CommandBatch::new(commands);
    snapshot
        .preview_batch(&batch)
        .map_err(GlbImportError::RejectedByDocument)?;
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
            .map_err(|_out_of_range: std::num::TryFromIntError| GlbImportError::InvalidContainer)?;
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
            BIN_CHUNK if chunk_index == 1 => binary = Some(&source[start..end]),
            // The GLB specification has readers skip chunks of unknown types.
            JSON_CHUNK | BIN_CHUNK => return Err(GlbImportError::InvalidContainer),
            _ if chunk_index > 0 => {}
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
        return Err(too_large(
            GlbMeasure::BufferViews,
            values.len(),
            MAX_PRIMITIVES * 4,
        ));
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
        return Err(too_large(
            GlbMeasure::Accessors,
            values.len(),
            MAX_PRIMITIVES * 2,
        ));
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
    material_looks: &[MaterialLook],
    selected_meshes: &BTreeSet<usize>,
    images: &mut Vec<Option<GlbImage>>,
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<ParsedMeshes, GlbImportError> {
    let meshes = array_required(root, "meshes")?;
    if meshes.is_empty() {
        return Err(GlbImportError::EmptyScene);
    }
    if meshes.len() > MAX_MESHES {
        return Err(too_large(GlbMeasure::Meshes, meshes.len(), MAX_MESHES));
    }
    if selected_meshes.iter().any(|index| *index >= meshes.len()) {
        return Err(GlbImportError::InvalidReference);
    }
    if meshes.len() > selected_meshes.len() {
        bump(
            diagnostics,
            "glb_unselected_meshes_ignored",
            u32::try_from(meshes.len() - selected_meshes.len()).map_err(
                |_out_of_range: std::num::TryFromIntError| {
                    too_large(GlbMeasure::Meshes, meshes.len(), MAX_MESHES)
                },
            )?,
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
        // glTF requires a mesh to hold a primitive.
        if primitives.is_empty() {
            return Err(GlbImportError::InvalidJson);
        }
        let parts = parsed.len().saturating_add(primitives.len());
        if parts > MAX_PRIMITIVES {
            return Err(too_large(GlbMeasure::MeshParts, parts, MAX_PRIMITIVES));
        }
        let mut primitive_indices = Vec::with_capacity(primitives.len());
        let mut pieces = Vec::with_capacity(primitives.len());
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
            let material = optional_usize(primitive, "material")?
                .map(|index| {
                    material_looks
                        .get(index)
                        .copied()
                        .ok_or(GlbImportError::InvalidReference)
                })
                .transpose()?;
            let index_accessor = usize_value(primitive, "indices")?;
            let vertices_mm = decode_positions(position_accessor, binary, views, accessors)?;
            let triangles =
                decode_triangles(index_accessor, vertices_mm.len(), binary, views, accessors)?;
            let (look, uv_attribute) = match material.and_then(|material| material.texture) {
                Some(texture) => {
                    let name = format!("TEXCOORD_{}", texture.set);
                    match optional_usize(attributes, &name)? {
                        Some(accessor) => {
                            let uvs = textures::decode_uvs(
                                accessor,
                                vertices_mm.len(),
                                binary,
                                views,
                                accessors,
                            )?;
                            let look =
                                textures::textured_triangles(texture.image, &uvs, &triangles)?;
                            (Some(look), Some(name))
                        }
                        None => {
                            bump(diagnostics, "glb_material_textures_ignored", 1)?;
                            (None, None)
                        }
                    }
                }
                None => (None, None),
            };
            if attributes
                .keys()
                .any(|name| name != "POSITION" && Some(name) != uv_attribute.as_ref())
            {
                bump(diagnostics, "glb_vertex_attributes_ignored", 1)?;
            }
            let (vertices_mm, triangles) = weld_indexed_geometry(vertices_mm, triangles)?;
            total_vertices = total_vertices.saturating_add(vertices_mm.len());
            total_triangles = total_triangles.saturating_add(triangles.len());
            if total_vertices > MAX_TOTAL_VERTICES {
                return Err(too_large(
                    GlbMeasure::Vertices,
                    total_vertices,
                    MAX_TOTAL_VERTICES,
                ));
            }
            if total_triangles > MAX_TOTAL_TRIANGLES {
                return Err(too_large(
                    GlbMeasure::Triangles,
                    total_triangles,
                    MAX_TOTAL_TRIANGLES,
                ));
            }
            let name = if primitives.len() == 1 {
                mesh_name.clone()
            } else {
                let name = format!("{mesh_name} / primitive {}", primitive_index + 1);
                validate_text(&name)?;
                name
            };
            pieces.push(ParsedPrimitive {
                name,
                vertices_mm,
                triangles,
                color: material.and_then(|material| material.color),
                look,
                sheet: false,
                origin_mm: [0.0; 3],
            });
        }
        let origin_mm = recentre(&mut pieces)?;
        for shell in closed_shells(&mesh_name, pieces, images, diagnostics)? {
            primitive_indices.push(parsed.len());
            parsed.push(ParsedPrimitive { origin_mm, ..shell });
        }
        mesh_primitives.push(primitive_indices);
    }
    Ok((parsed, mesh_primitives, total_triangles))
}

/// The bodies of one mesh. glTF holds one primitive per material, so a closed
/// Blender object with two materials arrives as open pieces, and a pane of glass
/// or a SketchUp face is an open surface. Closed primitives stay as they are;
/// the open ones are joined, and when that closes no solid each open piece
/// comes as a sheet of its own material. Only a piece that is not even a valid
/// sheet, such as one of degenerate triangles, is left out and counted.
fn closed_shells(
    mesh_name: &str,
    primitives: Vec<ParsedPrimitive>,
    images: &mut Vec<Option<GlbImage>>,
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<Vec<ParsedPrimitive>, GlbImportError> {
    let (mut shells, open): (Vec<_>, Vec<_>) = primitives.into_iter().partition(is_solid);
    let Some(first) = open.first() else {
        return Ok(shells);
    };
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for piece in &open {
        let offset =
            u32::try_from(vertices.len()).map_err(|_out_of_range: std::num::TryFromIntError| {
                too_large(GlbMeasure::Vertices, vertices.len(), MAX_TOTAL_VERTICES)
            })?;
        vertices.extend_from_slice(&piece.vertices_mm);
        triangles.extend(
            piece
                .triangles
                .iter()
                .map(|triangle| triangle.map(|index| index + offset)),
        );
    }
    let joined = weld_indexed_geometry(vertices, triangles)
        .ok()
        .map(|(vertices_mm, triangles)| ParsedPrimitive {
            name: if shells.is_empty() {
                mesh_name.to_owned()
            } else {
                first.name.clone()
            },
            vertices_mm,
            triangles,
            color: first.color,
            look: None,
            sheet: false,
            origin_mm: first.origin_mm,
        })
        .filter(is_solid)
        .map(|joined| {
            Ok::<_, GlbImportError>(ParsedPrimitive {
                look: joined_look(&open, first.color, images)?,
                ..joined
            })
        })
        .transpose()?;
    let count = |pieces: usize| {
        u32::try_from(pieces).map_err(|_out_of_range: std::num::TryFromIntError| {
            too_large(GlbMeasure::MeshParts, pieces, MAX_PRIMITIVES)
        })
    };
    if let Some(joined) = joined {
        bump(
            diagnostics,
            "glb_material_pieces_joined",
            count(open.len())?,
        )?;
        shells.push(joined);
        return Ok(shells);
    }
    let (sheets, invalid): (Vec<_>, Vec<_>) = open
        .into_iter()
        .map(|piece| ParsedPrimitive {
            sheet: true,
            ..piece
        })
        .partition(is_valid);
    if !sheets.is_empty() {
        bump(
            diagnostics,
            "glb_open_surfaces_imported",
            count(sheets.len())?,
        )?;
    }
    if !invalid.is_empty() {
        bump(
            diagnostics,
            "glb_non_solid_pieces_skipped",
            count(invalid.len())?,
        )?;
    }
    shells.extend(sheets);
    Ok(shells)
}

/// The look of the body joined from `pieces`, its colour `color`: each piece keeps its
/// picture, and a piece without one of another colour shows that colour as a picture of
/// one pixel. The triangles stay in the pieces' order.
fn joined_look(
    pieces: &[ParsedPrimitive],
    color: Option<[u8; 3]>,
    images: &mut Vec<Option<GlbImage>>,
) -> Result<Option<Vec<Option<TexturedTriangle>>>, GlbImportError> {
    let painted = |piece: &ParsedPrimitive| piece.color.filter(|own| Some(*own) != color);
    if pieces
        .iter()
        .all(|piece| piece.look.is_none() && painted(piece).is_none())
    {
        return Ok(None);
    }
    let mut look = Vec::new();
    for piece in pieces {
        match (&piece.look, painted(piece)) {
            (Some(own), _) => look.extend_from_slice(own),
            (None, Some(own)) => {
                let image = textures::colour_picture(own, images)?;
                look.extend(
                    std::iter::repeat_n(
                        TexturedTriangle {
                            image,
                            uv: [[0.5; 2]; 3],
                        },
                        piece.triangles.len(),
                    )
                    .map(Some),
                );
            }
            (None, None) => look.extend(std::iter::repeat_n(None, piece.triangles.len())),
        }
    }
    Ok(Some(look))
}

/// Moves the origin of a mesh lying beyond the model space to the centre of its pieces,
/// rounded to the millimetre, and answers that origin; a mesh within it stays, at zero.
fn recentre(pieces: &mut [ParsedPrimitive]) -> Result<[f64; 3], GlbImportError> {
    let within = |pieces: &[ParsedPrimitive]| {
        pieces
            .iter()
            .flat_map(|piece| piece.vertices_mm.iter().flatten())
            .all(|coordinate| coordinate.abs() <= MAX_COORDINATE_MM)
    };
    if within(pieces) {
        return Ok([0.0; 3]);
    }
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for point in pieces.iter().flat_map(|piece| piece.vertices_mm.iter()) {
        for axis in 0..3 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    let origin: [f64; 3] = std::array::from_fn(|axis| f64::midpoint(low[axis], high[axis]).round());
    for point in pieces
        .iter_mut()
        .flat_map(|piece| piece.vertices_mm.iter_mut())
    {
        for axis in 0..3 {
            point[axis] -= origin[axis];
        }
    }
    if !within(pieces) {
        return Err(GlbImportError::InvalidGeometry);
    }
    Ok(origin)
}

/// The [`MeshLook`] of triangles that show the GLB's `images` by their index: each
/// picture once, by its blob, and the triangles by their place in that list.
fn mesh_look(
    triangles: &[Option<TexturedTriangle>],
    images: &[Option<GlbImage>],
) -> Result<MeshLook, GlbImportError> {
    let shown: Vec<u32> = triangles
        .iter()
        .flatten()
        .map(|triangle| triangle.image)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let images = shown
        .iter()
        .map(|index| {
            let image = images
                .get(usize::try_from(*index).unwrap_or(usize::MAX))
                .and_then(Option::as_ref)
                .ok_or(GlbImportError::InvalidReference)?;
            Ok(MeshImage {
                blob: image.blob().to_owned(),
                mime_type: image.mime_type.to_owned(),
            })
        })
        .collect::<Result<_, GlbImportError>>()?;
    let places: BTreeMap<u32, u32> = shown.iter().copied().zip(0..).collect();
    let triangles = triangles
        .iter()
        .map(|triangle| {
            triangle
                .map(|triangle| {
                    Ok(TexturedTriangle {
                        image: *places
                            .get(&triangle.image)
                            .ok_or(GlbImportError::InvalidReference)?,
                        ..triangle
                    })
                })
                .transpose()
        })
        .collect::<Result<_, GlbImportError>>()?;
    Ok(MeshLook { images, triangles })
}

fn is_solid(primitive: &ParsedPrimitive) -> bool {
    !primitive.sheet && is_valid(primitive)
}

fn is_valid(primitive: &ParsedPrimitive) -> bool {
    crate::document::validate_mesh_body(&MeshBodySpec {
        look: None,
        schema: if primitive.sheet {
            MESH_SHEET_SCHEMA_V1
        } else {
            MESH_BODY_SCHEMA_V1
        }
        .to_owned(),
        vertices_mm: primitive.vertices_mm.clone(),
        triangles: primitive.triangles.clone(),
        authority: MeshAuthority::Authored {
            provenance: GLB_PARSER_ID.to_owned(),
        },
    })
    .is_ok()
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
                let index = u32::try_from(welded_vertices.len()).map_err(
                    |_out_of_range: std::num::TryFromIntError| {
                        too_large(
                            GlbMeasure::Vertices,
                            welded_vertices.len(),
                            MAX_TOTAL_VERTICES,
                        )
                    },
                )?;
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
        || accessor.count > limits::MESH_VERTICES
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
        if point.iter().any(|coordinate| {
            !coordinate.is_finite() || coordinate.abs() > MAX_SOURCE_COORDINATE_MM
        }) {
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
        || !(3..=limits::MESH_TRIANGLES * 3).contains(&accessor.count)
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
    if nodes.is_empty() {
        return Err(GlbImportError::EmptyScene);
    }
    if nodes.len() > MAX_NODES {
        return Err(too_large(GlbMeasure::Nodes, nodes.len(), MAX_NODES));
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
        visit_raw_node(root, 1, nodes, &mut state, &mut selected)?;
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

/// Selects the node at `index`, `depth` nodes from its scene root, and those under it.
fn visit_raw_node(
    index: usize,
    depth: usize,
    nodes: &[Value],
    state: &mut [u8],
    selected: &mut BTreeSet<usize>,
) -> Result<(), GlbImportError> {
    if depth > MAX_NODE_DEPTH {
        return Err(too_large(GlbMeasure::Nesting, depth, MAX_NODE_DEPTH));
    }
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
        visit_raw_node(child, depth + 1, nodes, state, selected)?;
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
    if values.is_empty() {
        return Err(GlbImportError::EmptyScene);
    }
    if values.len() > MAX_NODES {
        return Err(too_large(GlbMeasure::Nodes, values.len(), MAX_NODES));
    }
    let mut nodes = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        if !selected_nodes.contains(&index) {
            nodes.push(ParsedNode {
                name: "Unselected GLB node".to_owned(),
                local: Affine3::IDENTITY,
                transform: Transform::identity(),
                placements: Vec::new(),
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
        let local = parse_node_transform(value)?;
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
            local,
            transform: Transform::identity(),
            placements: Vec::new(),
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
            u32::try_from(ignored).map_err(|_out_of_range: std::num::TryFromIntError| {
                too_large(GlbMeasure::Nodes, nodes.len(), MAX_NODES)
            })?,
        )?;
    }
    let instance_count = traversal.iter().fold(0_usize, |count, index| {
        count.saturating_add(nodes[*index].primitives.len())
    });
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

fn parse_node_transform(node: &Map<String, Value>) -> Result<Affine3, GlbImportError> {
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
    if !norm.is_finite() || (norm - 1.0).abs() > APPROXIMATION {
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

/// The transform in Kečup's axes and millimetres, its move within
/// [`MAX_SOURCE_COORDINATE_MM`]; [`place_nodes`] brings it within the model space.
fn gltf_transform_to_ketchup(gltf: [f64; 16]) -> Result<Affine3, GlbImportError> {
    if gltf.iter().any(|value| !value.is_finite())
        || gltf[12].abs() > f64::EPSILON
        || gltf[13].abs() > f64::EPSILON
        || gltf[14].abs() > f64::EPSILON
        || (gltf[15] - 1.0).abs() > f64::EPSILON
    {
        return Err(GlbImportError::InvalidTransform);
    }
    let affine = ketchup_transform_from_gltf(Affine3::from_row_major(gltf));
    if affine
        .linear
        .rows
        .iter()
        .flatten()
        .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        || [
            affine.translation.x,
            affine.translation.y,
            affine.translation.z,
        ]
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_SOURCE_COORDINATE_MM)
        || affine.linear.is_singular()
    {
        return Err(GlbImportError::InvalidTransform);
    }
    Ok(affine)
}

/// The document transform of `affine`, a move within the model space.
fn document_transform(affine: Affine3) -> Result<Transform, GlbImportError> {
    if affine
        .to_row_major()
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
    {
        return Err(GlbImportError::InvalidTransform);
    }
    Transform::from_affine(affine).map_err(GlbImportError::RejectedByDocument)
}

fn translation(offset: Vec3) -> Affine3 {
    Affine3 {
        linear: Mat3::IDENTITY,
        translation: offset,
    }
}

/// Sets the transforms the groups and parts of the scene import with. A node keeps its
/// own transform where the geometry under it, its transform and its parts' fit the
/// model space. Otherwise its frame moves to the centre of the geometry under it, as a
/// georeferenced component needs:
/// its group then holds the move from its parent's centre to its own, and each part
/// the move from there to its mesh's origin. Placed in the model, nothing moves.
fn place_nodes(
    nodes: &mut [ParsedNode],
    traversal: &[usize],
    primitives: &[ParsedPrimitive],
) -> Result<(), GlbImportError> {
    let primitive_bounds = primitives
        .iter()
        .map(|primitive| {
            let origin = Vec3::new(
                primitive.origin_mm[0],
                primitive.origin_mm[1],
                primitive.origin_mm[2],
            );
            bounds(
                primitive
                    .vertices_mm
                    .iter()
                    .map(|point| Vec3::new(point[0], point[1], point[2]) + origin),
            )
        })
        .collect::<Vec<_>>();
    // The box of the geometry under each node, in its frame; children come later in
    // the traversal, so they are boxed first.
    let mut node_bounds = vec![None; nodes.len()];
    for index in traversal.iter().rev() {
        let node = &nodes[*index];
        let own = node
            .primitives
            .iter()
            .filter_map(|primitive| primitive_bounds[*primitive])
            .flat_map(corners);
        let below = node
            .children
            .iter()
            .filter_map(|child| node_bounds[*child].map(|bounds| (nodes[*child].local, bounds)))
            .flat_map(|(local, bounds)| {
                corners(bounds).map(|corner| local.transform_point(corner))
            });
        node_bounds[*index] = bounds(own.chain(below));
    }
    let mut parent_by_node = vec![None; nodes.len()];
    for index in traversal {
        for child in &nodes[*index].children {
            parent_by_node[*child] = Some(*index);
        }
    }
    let mut pivots = vec![Vec3::ZERO; nodes.len()];
    for index in traversal {
        let parent_pivot = parent_by_node[*index].map_or(Vec3::ZERO, |parent| pivots[parent]);
        let node = &nodes[*index];
        let place = |pivot: Vec3| -> Result<(Transform, Vec<Transform>), GlbImportError> {
            let transform = document_transform(
                translation(-parent_pivot)
                    .compose(node.local)
                    .compose(translation(pivot)),
            )?;
            let placements = node
                .primitives
                .iter()
                .map(|primitive| {
                    let origin = primitives[*primitive].origin_mm;
                    document_transform(translation(
                        Vec3::new(origin[0], origin[1], origin[2]) - pivot,
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((transform, placements))
        };
        let within = node_bounds[*index].is_none_or(|(low, high): (Vec3, Vec3)| {
            [low.x, low.y, low.z, high.x, high.y, high.z]
                .iter()
                .all(|value| value.abs() <= MAX_COORDINATE_MM)
        });
        let at_origin = if within { place(Vec3::ZERO).ok() } else { None };
        let (pivot, (transform, placements)) = match at_origin {
            Some(placed) => (Vec3::ZERO, placed),
            None => {
                let pivot = node_bounds[*index].map_or(Vec3::ZERO, |(low, high)| {
                    Vec3::new(
                        f64::midpoint(low.x, high.x).round(),
                        f64::midpoint(low.y, high.y).round(),
                        f64::midpoint(low.z, high.z).round(),
                    )
                });
                (pivot, place(pivot)?)
            }
        };
        pivots[*index] = pivot;
        nodes[*index].transform = transform;
        nodes[*index].placements = placements;
    }
    Ok(())
}

fn bounds(points: impl Iterator<Item = Vec3>) -> Option<(Vec3, Vec3)> {
    points.fold(None, |bounds, point| {
        Some(bounds.map_or((point, point), |(low, high): (Vec3, Vec3)| {
            (
                Vec3::new(low.x.min(point.x), low.y.min(point.y), low.z.min(point.z)),
                Vec3::new(
                    high.x.max(point.x),
                    high.y.max(point.y),
                    high.z.max(point.z),
                ),
            )
        }))
    })
}

fn corners((low, high): (Vec3, Vec3)) -> [Vec3; 8] {
    std::array::from_fn(|corner| {
        Vec3::new(
            if corner & 1 == 0 { low.x } else { high.x },
            if corner & 2 == 0 { low.y } else { high.y },
            if corner & 4 == 0 { low.z } else { high.z },
        )
    })
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

/// Names what the import leaves out, from the length of each root array it only counts
/// ([`json::COUNTED_KEYS`]).
fn record_document_losses(
    counts: &BTreeMap<&'static str, usize>,
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<(), GlbImportError> {
    for (name, code) in [
        ("animations", "glb_animations_ignored"),
        ("cameras", "glb_camera_definitions_ignored"),
        ("extensionsUsed", "glb_optional_extensions_ignored"),
    ] {
        let count = counts.get(name).copied().unwrap_or(0);
        if count > 0 {
            bump(
                diagnostics,
                code,
                u32::try_from(count).map_err(|_out_of_range: std::num::TryFromIntError| {
                    GlbImportError::UnsupportedFeature
                })?,
            )?;
        }
    }
    Ok(())
}

fn parse_materials(
    root: &Map<String, Value>,
    materials: &[Value],
    images: &[Option<GlbImage>],
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<Vec<MaterialLook>, GlbImportError> {
    materials
        .iter()
        .map(|material| {
            let material = object(material)?;
            let mode = match material.get("alphaMode").map(Value::as_str) {
                None => None,
                Some(Some(mode @ ("OPAQUE" | "MASK" | "BLEND"))) => Some(mode),
                Some(_) => return Err(GlbImportError::UnsupportedFeature),
            };
            let cutoff = material
                .get("alphaCutoff")
                .map(|value| {
                    value
                        .as_f64()
                        .filter(|cutoff| cutoff.is_finite() && *cutoff >= 0.0)
                        .ok_or(GlbImportError::InvalidJson)
                })
                .transpose()?
                .unwrap_or(0.5);
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
            let texture = match pbr.and_then(|pbr| pbr.get("baseColorTexture")) {
                Some(info) => {
                    let texture = textures::base_color_texture(root, info, images, diagnostics)?;
                    if texture.is_none() {
                        bump(diagnostics, "glb_material_textures_ignored", 1)?;
                    }
                    texture
                }
                None => None,
            };
            let factor = pbr
                .and_then(|pbr| pbr.get("baseColorFactor"))
                .map(|value| exact_number_array(value, 4))
                .transpose()?
                .unwrap_or_else(|| vec![1.0; 4]);
            if factor
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
            {
                return Err(GlbImportError::UnsupportedFeature);
            }
            // Transparency changes only how a solid looks, so a glass or
            // cut-out material imports as an opaque colour with a note.
            if textures::shows_transparency(mode, cutoff, factor[3], texture, images) {
                bump(diagnostics, "glb_material_transparency_ignored", 1)?;
            }
            Ok(MaterialLook {
                color: Some([
                    linear_to_srgb_byte(factor[0]),
                    linear_to_srgb_byte(factor[1]),
                    linear_to_srgb_byte(factor[2]),
                ]),
                texture,
            })
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
    usize::try_from(u64_value(object, name)?)
        .map_err(|_out_of_range: std::num::TryFromIntError| GlbImportError::InvalidJson)
}

fn optional_usize(
    object: &Map<String, Value>,
    name: &str,
) -> Result<Option<usize>, GlbImportError> {
    optional_u64(object, name)?
        .map(|value| {
            usize::try_from(value)
                .map_err(|_out_of_range: std::num::TryFromIntError| GlbImportError::InvalidJson)
        })
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
    if value.is_empty() || value.len() > limits::TEXT_BYTES || value.chars().any(char::is_control) {
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

#[cfg(test)]
mod json_tests {
    use super::*;

    /// The JSON and the binary chunk of the committed Blender GLB.
    pub(super) fn sample() -> (Value, Vec<u8>) {
        let seed = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../artifacts/blender/garden-studio-colored.glb"
        ))
        .unwrap();
        let length = usize::try_from(u32::from_le_bytes(seed[12..16].try_into().unwrap())).unwrap();
        let json = serde_json::from_slice(&seed[20..20 + length]).unwrap();
        (json, seed[20 + length..].to_vec())
    }

    pub(super) fn packed(json: &Value, binary: &[u8]) -> Vec<u8> {
        let mut json = serde_json::to_vec(json).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let total = 12 + 8 + json.len() + binary.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
        glb.extend_from_slice(&u32::try_from(json.len()).unwrap().to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(binary);
        glb
    }

    /// Review 2026-10-21 P3: the JSON of a GLB was kept whole as `serde_json::Value`, 14
    /// to 27 times its size. Data the import never reads, `extras` however large, and
    /// arrays it only counts now read as without it; what it reads is bounded, so a file
    /// holding more than an importable GLB can is refused by name, not read whole.
    #[test]
    fn unread_json_costs_nothing_and_read_json_is_bounded() {
        let (json, binary) = sample();
        let scene = inspect_glb(&packed(&json, &binary)).unwrap();
        let many = |value: Value| Value::Array(vec![value; 300_000]);

        let mut unread = json.clone();
        unread["extras"] = many(Value::from(0));
        unread["nodes"][0]["extras"] = many(Value::from("x"));
        unread["meshes"][0]["primitives"][0]["extras"] = many(Value::from(0.5));
        assert_eq!(inspect_glb(&packed(&unread, &binary)), Ok(scene.clone()));

        let mut counted = json.clone();
        counted["animations"] = many(serde_json::json!({"channels": [], "samplers": []}));
        let counted = inspect_glb(&packed(&counted, &binary)).unwrap();
        assert_eq!(counted.instance_count, scene.instance_count);
        assert!(counted.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "glb_animations_ignored" && diagnostic.count() == 300_000
        }));

        let mut read = json;
        read["materials"]
            .as_array_mut()
            .unwrap()
            .extend(vec![serde_json::json!({}); json::MAX_JSON_VALUES]);
        // Counted, with what to do, as any other model too large for one import (review
        // 2026-11-12 P3: a SketchUp model of some 60 000 placements got a bare JSON error).
        let refused = inspect_glb(&packed(&read, &binary)).unwrap_err();
        assert_eq!(
            refused,
            too_large(
                GlbMeasure::Description,
                json::MAX_JSON_VALUES,
                json::MAX_JSON_VALUES
            )
        );
        let message = refused.to_string();
        assert!(
            message.contains("one import takes at most 1048576"),
            "{message}"
        );
        assert!(message.contains("Purge Unused"), "{message}");
    }

    /// A glass material (a base colour with alpha below 1) imports as its opaque colour with
    /// a note, as a cut-out or blended one does; it refused the whole file before, so a
    /// SketchUp model with one window pane did not import at all.
    #[test]
    fn a_translucent_material_imports_as_its_opaque_colour_with_a_note() {
        let (json, binary) = sample();
        let opaque = inspect_glb(&packed(&json, &binary)).unwrap();
        let mut glass = json;
        for material in glass["materials"].as_array_mut().unwrap() {
            material["alphaMode"] = Value::from("BLEND");
            if let Some(factor) = material
                .pointer_mut("/pbrMetallicRoughness/baseColorFactor")
                .and_then(Value::as_array_mut)
            {
                factor[3] = Value::from(0.4);
            }
        }
        let glass = inspect_glb(&packed(&glass, &binary)).unwrap();
        let colours = |scene: &ParsedGlbScene| {
            scene
                .primitives
                .iter()
                .map(|primitive| primitive.color)
                .collect::<Vec<_>>()
        };
        assert_eq!(colours(&glass), colours(&opaque));
        assert!(colours(&glass).iter().any(Option::is_some));
        assert!(glass.diagnostics().iter().any(|diagnostic| {
            diagnostic.code() == "glb_material_transparency_ignored" && diagnostic.count() > 0
        }));
    }
}

#[cfg(test)]
mod gltf_axes_tests {
    use super::*;

    fn close(left: Affine3, right: Affine3) -> bool {
        left.to_row_major()
            .iter()
            .zip(right.to_row_major())
            .all(|(a, b)| (a - b).abs() <= 1.0e-9)
    }

    #[test]
    fn gltf_axes_round_trip_points_transforms_and_inverses() {
        // A rotation about z, a non-uniform scale and a translation in mm.
        let transform = Affine3::from_row_major([
            0.0, -2.0, 0.0, 10.0, 1.0, 0.0, 0.0, -5.0, 0.0, 0.0, 3.0, 7.0, 0.0, 0.0, 0.0, 1.0,
        ]);
        let gltf = gltf_transform_from_ketchup(transform);
        assert!(close(ketchup_transform_from_gltf(gltf), transform));
        let point = Vec3::new(4.0, 6.0, -2.0);
        let mapped = gltf.transform_point(gltf_point_from_ketchup(point));
        let expected = gltf_point_from_ketchup(transform.transform_point(point));
        assert!(mapped.distance(expected) < 1.0e-12);
        assert!(close(
            gltf_transform_from_ketchup(transform.invert().unwrap()),
            gltf.invert().unwrap()
        ));
        // Kečup up (+Z) is glTF up (+Y), and 1 000 mm is 1 m.
        let up = gltf_point_from_ketchup(Vec3::new(0.0, 0.0, MILLIMETRES_PER_METRE));
        assert!(up.distance(Vec3::Y) < 1.0e-12);
    }
}

#[cfg(test)]
mod placement_tests {
    use super::json_tests::packed;
    use super::*;

    /// A GLB of one tetrahedron, its corners `offset` metres along glTF X from the
    /// origin of its mesh, under a group whose node is `group`.
    fn tetrahedron(offset: f32, group: Value) -> Vec<u8> {
        let mut binary = Vec::new();
        for corner in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0_f32],
        ] {
            binary.extend((corner[0] + offset).to_le_bytes());
            binary.extend(corner[1].to_le_bytes());
            binary.extend(corner[2].to_le_bytes());
        }
        for index in [0_u16, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3] {
            binary.extend(index.to_le_bytes());
        }
        let length = binary.len();
        let mut chunk = u32::try_from(length).unwrap().to_le_bytes().to_vec();
        chunk.extend(b"BIN\0");
        chunk.extend(binary);
        let mut group = group;
        group["name"] = serde_json::json!("IFC element");
        group["children"] = serde_json::json!([1]);
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
            "meshes": [{"name": "Proxy", "primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
            "nodes": [group, {"name": "Proxy", "mesh": 0}],
            "scenes": [{"nodes": [0]}],
        });
        packed(&json, &chunk)
    }

    /// The corners of the scene's parts where they are placed, to the millimetre: a
    /// float32 corner 4 km out holds a quarter of one.
    fn placed_corners(source: &[u8]) -> Vec<[i64; 3]> {
        let scene = inspect_glb(source).unwrap();
        let mut corners = Vec::new();
        for node in &scene.traversal {
            let mut world = scene.nodes[*node].transform.affine();
            let mut parent = *node;
            while let Some(above) = scene
                .traversal
                .iter()
                .find(|index| scene.nodes[**index].children.contains(&parent))
            {
                world = scene.nodes[*above].transform.affine().compose(world);
                parent = *above;
            }
            for (primitive, placement) in scene.nodes[*node]
                .primitives
                .iter()
                .zip(&scene.nodes[*node].placements)
            {
                let placed = world.compose(placement.affine());
                for point in &scene.primitives[*primitive].vertices_mm {
                    let point = placed.transform_point(Vec3::from(*point));
                    corners.push([point.x, point.y, point.z].map(|value| value.round() as i64));
                }
            }
        }
        corners.sort_unstable();
        corners
    }

    /// An IFC element of a SketchUp terrace keeps its geometry 4 km from its origin, and
    /// its group moves it back by as much. Placed, it lies in the model: it imports where
    /// it is, each part and group within the model space. It was refused as invalid
    /// geometry.
    #[test]
    fn a_georeferenced_component_imports_where_it_is_placed() {
        let far = 4_063.6_f32;
        let near = placed_corners(&tetrahedron(0.0, serde_json::json!({})));
        let moved_back = tetrahedron(
            far,
            serde_json::json!({"translation": [-f64::from(far), 0.0, 0.0]}),
        );
        assert_eq!(placed_corners(&moved_back), near);
        let scaled_back = tetrahedron(
            far,
            serde_json::json!({
                "translation": [-f64::from(far) * 0.1, 0.0, 0.0],
                "scale": [0.1, 0.1, 0.1],
            }),
        );
        let scaled = placed_corners(&tetrahedron(
            0.0,
            serde_json::json!({"scale": [0.1, 0.1, 0.1]}),
        ));
        assert_eq!(placed_corners(&scaled_back), scaled);

        // Geometry placed beyond the model space is still refused.
        assert_eq!(
            inspect_glb(&tetrahedron(far, serde_json::json!({}))),
            Err(GlbImportError::InvalidTransform)
        );
    }
}

#[cfg(test)]
mod open_surface_tests;

#[cfg(test)]
mod shared_picture_tests;
