use crate::document::{
    DefinitionId, FeatureId, GroupId, InstancePathStep, LocalGroupKey, LocalOccurrenceKey,
    SceneOccurrence, Snapshot, Transform,
};
use crate::exact_product::{ExactBodyPackage, ExactProductError, MeshExportSource};
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

pub const MAX_THREE_MF_EXPORT_INSTANCES: usize = 8_000;
const MAX_THREE_MF_EXPORT_VERTICES: usize = 2_000_000;
const MAX_THREE_MF_EXPORT_TRIANGLES: usize = 4_000_000;
const MAX_THREE_MF_EXPORT_XML_BYTES: usize = 256 * 1024 * 1024;
const MAX_THREE_MF_EXPORT_ARCHIVE_BYTES: usize = 257 * 1024 * 1024;

#[derive(Clone, Copy)]
struct ThreeMfExportLimits {
    instances: usize,
    vertices: usize,
    triangles: usize,
    xml_bytes: usize,
    archive_bytes: usize,
}

const THREE_MF_EXPORT_LIMITS: ThreeMfExportLimits = ThreeMfExportLimits {
    instances: MAX_THREE_MF_EXPORT_INSTANCES,
    vertices: MAX_THREE_MF_EXPORT_VERTICES,
    triangles: MAX_THREE_MF_EXPORT_TRIANGLES,
    xml_bytes: MAX_THREE_MF_EXPORT_XML_BYTES,
    archive_bytes: MAX_THREE_MF_EXPORT_ARCHIVE_BYTES,
};

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#;
const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactThreeMfExport {
    pub three_mf: Vec<u8>,
    pub loss_report: String,
}

#[derive(Clone, Copy)]
pub struct ExactThreeMfInstance<'a> {
    pub package: &'a ExactBodyPackage,
    pub occurrence: &'a SceneOccurrence,
}

#[derive(Clone, Copy)]
pub struct MeshThreeMfInstance<'a> {
    pub source: MeshExportSource<'a>,
    pub occurrence: &'a SceneOccurrence,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MeshKey {
    definition_id: DefinitionId,
    producer_feature_id: FeatureId,
    result_fingerprint: String,
    color: Option<[u8; 3]>,
}

#[derive(Clone)]
struct AssemblyNode {
    name: String,
    transform: Transform,
    meshes: Vec<MeshKey>,
    children: Vec<usize>,
}

pub fn exact_model_three_mf_export(
    snapshot: &Snapshot,
    instances: &[ExactThreeMfInstance<'_>],
) -> Result<ExactThreeMfExport, ExactProductError> {
    let instances = instances
        .iter()
        .map(|instance| MeshThreeMfInstance {
            source: MeshExportSource::Exact(instance.package),
            occurrence: instance.occurrence,
        })
        .collect::<Vec<_>>();
    model_three_mf_export(snapshot, &instances)
}

pub fn model_three_mf_export(
    snapshot: &Snapshot,
    instances: &[MeshThreeMfInstance<'_>],
) -> Result<ExactThreeMfExport, ExactProductError> {
    model_three_mf_export_with_limits(snapshot, instances, THREE_MF_EXPORT_LIMITS)
}

fn model_three_mf_export_with_limits(
    snapshot: &Snapshot,
    instances: &[MeshThreeMfInstance<'_>],
    limits: ThreeMfExportLimits,
) -> Result<ExactThreeMfExport, ExactProductError> {
    if instances.is_empty() {
        return Err(ExactProductError::EmptyModelExport);
    }
    if instances.len() > limits.instances {
        return Err(ExactProductError::ExportResourceLimit);
    }
    if instances
        .iter()
        .any(|instance| !instance.occurrence.visible || !instance.source.is_current(snapshot))
    {
        return Err(ExactProductError::StaleResult);
    }
    if instances
        .iter()
        .any(|instance| instance.source.definition_id() != instance.occurrence.definition_id)
    {
        return Err(ExactProductError::InvalidMeshExport);
    }

    let mut packages = BTreeMap::<MeshKey, MeshExportSource<'_>>::new();
    let mut vertex_count = 0_usize;
    let mut triangle_count = 0_usize;
    for instance in instances {
        let key = mesh_key(instance);
        if packages.contains_key(&key) {
            continue;
        }
        vertex_count = add_with_limit(
            vertex_count,
            instance.source.vertex_count(),
            limits.vertices,
        )?;
        triangle_count = add_with_limit(
            triangle_count,
            instance.source.triangle_count(),
            limits.triangles,
        )?;
        validate_mesh(instance.source)?;
        packages.insert(key, instance.source);
    }
    let (nodes, roots) = build_assembly(snapshot, instances)?;

    let colors = packages
        .keys()
        .filter_map(|key| key.color)
        .collect::<std::collections::BTreeSet<_>>();
    let material_indices = colors
        .iter()
        .copied()
        .enumerate()
        .map(|(index, color)| (color, index))
        .collect::<BTreeMap<_, _>>();

    let mut next_id = if colors.is_empty() { 1_u32 } else { 2_u32 };
    let mesh_ids = packages
        .keys()
        .cloned()
        .map(|key| {
            let id = next_id;
            next_id += 1;
            (key, id)
        })
        .collect::<BTreeMap<_, _>>();

    let mut node_ids = BTreeMap::<usize, u32>::new();
    let mut node_order = Vec::new();
    for root in &roots {
        allocate_node_ids(*root, &nodes, &mut node_ids, &mut node_order, &mut next_id)?;
    }

    let model = encode_model(
        snapshot,
        &packages,
        &mesh_ids,
        &material_indices,
        &nodes,
        &node_ids,
        &node_order,
        &roots,
        limits.xml_bytes,
    )?;
    let three_mf = encode_package(
        &[
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", ROOT_RELATIONSHIPS.as_bytes()),
            ("3D/3dmodel.model", model.as_bytes()),
        ],
        limits.archive_bytes,
    )?;
    let canonical_mesh_count = instances
        .iter()
        .filter(|instance| instance.source.is_canonical())
        .count();
    let loss_report = format!(
        "authority=validated exact tessellations and canonical mesh bodies\nformat=3MF Core 1.3 package\nconversion=current-visible-model-to-instanced-print-mesh\nunit=millimeter\naxis=Ketchup Z-up preserved\nhierarchy=canonical global groups, component occurrences, local groups, and nested occurrences\nmaterials=resolved occurrence sRGB colors\neditability_loss=canonical features, rules, dimensions, constraints, and Undo history are not preserved\ntopology_loss=exact topology, analytic surfaces, and durable face identity are not preserved\ntolerance_loss=exact geometry uses its accepted tessellation; canonical mesh vertices are preserved\nsource_digest={}\noccurrence_body_count={}\ncanonical_mesh_occurrence_count={canonical_mesh_count}\nunique_mesh_resource_count={}\nassembly_object_count={}\nresource_vertex_limit={}\nresource_triangle_limit={}\n",
        snapshot.canonical_digest(),
        instances.len(),
        packages.len(),
        nodes.len(),
        limits.vertices,
        limits.triangles,
    );
    Ok(ExactThreeMfExport {
        three_mf,
        loss_report,
    })
}

fn mesh_key(instance: &MeshThreeMfInstance<'_>) -> MeshKey {
    MeshKey {
        definition_id: instance.source.definition_id(),
        producer_feature_id: instance.source.producer_feature_id(),
        result_fingerprint: instance.source.identity(),
        color: instance.occurrence.color(),
    }
}

fn add_with_limit(
    current: usize,
    additional: usize,
    limit: usize,
) -> Result<usize, ExactProductError> {
    current
        .checked_add(additional)
        .filter(|total| *total <= limit)
        .ok_or(ExactProductError::ExportResourceLimit)
}

fn validate_mesh(source: MeshExportSource<'_>) -> Result<(), ExactProductError> {
    if source.vertex_count() == 0 || source.triangle_count() == 0 {
        return Err(ExactProductError::InvalidMeshExport);
    }
    if (0..source.vertex_count()).any(|index| {
        source
            .vertex_position_mm(index)
            .is_none_or(|point| point.into_iter().any(|value| !value.is_finite()))
    }) || (0..source.triangle_count()).any(|index| {
        source.triangle_indices(index).is_none_or(|triangle| {
            triangle
                .into_iter()
                .any(|vertex| vertex as usize >= source.vertex_count())
        })
    }) {
        return Err(ExactProductError::InvalidMeshExport);
    }
    Ok(())
}

fn build_assembly(
    snapshot: &Snapshot,
    instances: &[MeshThreeMfInstance<'_>],
) -> Result<(Vec<AssemblyNode>, Vec<usize>), ExactProductError> {
    let mut nodes = Vec::new();
    let mut roots = Vec::new();
    let mut hierarchy = BTreeMap::<String, usize>::new();
    for instance in instances {
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
                    ensure_global_group(snapshot, group_id, &mut nodes, &mut roots, &mut hierarchy)
                })
                .transpose()?;
            let index = nodes.len();
            nodes.push(AssemblyNode {
                name: root_occurrence.name().to_owned(),
                transform: root_occurrence.transform(),
                meshes: Vec::new(),
                children: Vec::new(),
            });
            attach_node(&mut nodes, &mut roots, parent, index);
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
                    let local = snapshot
                        .local_group(LocalGroupKey {
                            definition_id: owner_definition_id,
                            local_id: *local_id,
                        })
                        .ok_or(ExactProductError::InvalidMeshExport)?;
                    parent_index = ensure_local_node(
                        &mut nodes,
                        &mut hierarchy,
                        parent_index,
                        &path_key,
                        local.name(),
                        local.transform(),
                    );
                }
                InstancePathStep::Occurrence(local_id) => {
                    path_key.push_str(&format!("/occurrence:{}", local_id.0));
                    let local = snapshot
                        .local_occurrence(LocalOccurrenceKey {
                            definition_id: owner_definition_id,
                            local_id: *local_id,
                        })
                        .ok_or(ExactProductError::InvalidMeshExport)?;
                    parent_index = ensure_local_node(
                        &mut nodes,
                        &mut hierarchy,
                        parent_index,
                        &path_key,
                        local.name(),
                        local.transform(),
                    );
                    owner_definition_id = local.definition_id();
                }
            }
        }
        if owner_definition_id != instance.occurrence.definition_id {
            return Err(ExactProductError::InvalidMeshExport);
        }
        nodes[parent_index].meshes.push(mesh_key(instance));
    }
    Ok((nodes, roots))
}

fn ensure_global_group(
    snapshot: &Snapshot,
    group_id: GroupId,
    nodes: &mut Vec<AssemblyNode>,
    roots: &mut Vec<usize>,
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
        .map(|parent_id| ensure_global_group(snapshot, parent_id, nodes, roots, hierarchy))
        .transpose()?;
    let index = nodes.len();
    nodes.push(AssemblyNode {
        name: group.name().to_owned(),
        transform: group.transform(),
        meshes: Vec::new(),
        children: Vec::new(),
    });
    attach_node(nodes, roots, parent, index);
    hierarchy.insert(key, index);
    Ok(index)
}

fn ensure_local_node(
    nodes: &mut Vec<AssemblyNode>,
    hierarchy: &mut BTreeMap<String, usize>,
    parent: usize,
    key: &str,
    name: &str,
    transform: Transform,
) -> usize {
    if let Some(index) = hierarchy.get(key) {
        return *index;
    }
    let index = nodes.len();
    nodes.push(AssemblyNode {
        name: name.to_owned(),
        transform,
        meshes: Vec::new(),
        children: Vec::new(),
    });
    nodes[parent].children.push(index);
    hierarchy.insert(key.to_owned(), index);
    index
}

fn attach_node(
    nodes: &mut [AssemblyNode],
    roots: &mut Vec<usize>,
    parent: Option<usize>,
    index: usize,
) {
    if let Some(parent) = parent {
        nodes[parent].children.push(index);
    } else {
        roots.push(index);
    }
}

fn allocate_node_ids(
    node_index: usize,
    nodes: &[AssemblyNode],
    ids: &mut BTreeMap<usize, u32>,
    order: &mut Vec<usize>,
    next_id: &mut u32,
) -> Result<(), ExactProductError> {
    if ids.contains_key(&node_index) {
        return Ok(());
    }
    for child in &nodes[node_index].children {
        allocate_node_ids(*child, nodes, ids, order, next_id)?;
    }
    if nodes[node_index].meshes.is_empty() && nodes[node_index].children.is_empty() {
        return Err(ExactProductError::InvalidMeshExport);
    }
    let id = *next_id;
    *next_id = next_id
        .checked_add(1)
        .ok_or(ExactProductError::InvalidMeshExport)?;
    ids.insert(node_index, id);
    order.push(node_index);
    Ok(())
}

struct BoundedString {
    value: String,
    limit: usize,
}

impl BoundedString {
    fn new(limit: usize) -> Self {
        Self {
            value: String::with_capacity(limit.min(1024 * 1024)),
            limit,
        }
    }

    fn into_inner(self) -> String {
        self.value
    }
}

impl fmt::Write for BoundedString {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self
            .value
            .len()
            .checked_add(value.len())
            .is_none_or(|length| length > self.limit)
        {
            return Err(fmt::Error);
        }
        self.value.push_str(value);
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_model(
    snapshot: &Snapshot,
    packages: &BTreeMap<MeshKey, MeshExportSource<'_>>,
    mesh_ids: &BTreeMap<MeshKey, u32>,
    material_indices: &BTreeMap<[u8; 3], usize>,
    nodes: &[AssemblyNode],
    node_ids: &BTreeMap<usize, u32>,
    node_order: &[usize],
    roots: &[usize],
    xml_limit: usize,
) -> Result<String, ExactProductError> {
    let mut xml = BoundedString::new(xml_limit);
    write!(
        xml,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n  <metadata name=\"Title\">Ketchup model</metadata>\n  <metadata name=\"KetchupSourceDigest\">{}</metadata>\n  <resources>\n",
        snapshot.canonical_digest()
    )
    .map_err(|_| ExactProductError::ExportResourceLimit)?;
    if !material_indices.is_empty() {
        xml.write_str("    <basematerials id=\"1\">\n")
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        for color in material_indices.keys() {
            writeln!(
                xml,
                "      <base name=\"Ketchup #{:02X}{:02X}{:02X}\" displaycolor=\"#{:02X}{:02X}{:02X}FF\"/>",
                color[0], color[1], color[2], color[0], color[1], color[2]
            )
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        }
        xml.write_str("    </basematerials>\n")
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
    }
    for (key, package) in packages {
        let id = mesh_ids[key];
        write!(xml, "    <object id=\"{id}\" type=\"model\" name=\"")
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        write_xml_escaped(
            &mut xml,
            snapshot
                .definition(key.definition_id)
                .ok_or(ExactProductError::InvalidMeshExport)?
                .name(),
        )?;
        writeln!(xml, " body {}\">", key.producer_feature_id.0)
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        xml.write_str("      <mesh>\n        <vertices>\n")
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        for index in 0..package.vertex_count() {
            let vertex = package
                .vertex_position_mm(index)
                .ok_or(ExactProductError::InvalidMeshExport)?;
            writeln!(
                xml,
                "          <vertex x=\"{:.17}\" y=\"{:.17}\" z=\"{:.17}\"/>",
                vertex[0], vertex[1], vertex[2]
            )
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        }
        xml.write_str("        </vertices>\n        <triangles>\n")
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        for index in 0..package.triangle_count() {
            let [v1, v2, v3] = package
                .triangle_indices(index)
                .ok_or(ExactProductError::InvalidMeshExport)?;
            if let Some(color) = key.color {
                let material = material_indices[&color];
                writeln!(
                    xml,
                    "          <triangle v1=\"{v1}\" v2=\"{v2}\" v3=\"{v3}\" pid=\"1\" p1=\"{material}\" p2=\"{material}\" p3=\"{material}\"/>"
                )
                .map_err(|_| ExactProductError::ExportResourceLimit)?;
            } else {
                writeln!(
                    xml,
                    "          <triangle v1=\"{v1}\" v2=\"{v2}\" v3=\"{v3}\"/>"
                )
                .map_err(|_| ExactProductError::ExportResourceLimit)?;
            }
        }
        xml.write_str("        </triangles>\n      </mesh>\n    </object>\n")
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
    }
    for node_index in node_order {
        let node = &nodes[*node_index];
        write!(
            xml,
            "    <object id=\"{}\" type=\"model\" name=\"",
            node_ids[node_index]
        )
        .map_err(|_| ExactProductError::ExportResourceLimit)?;
        write_xml_escaped(&mut xml, &node.name)?;
        xml.write_str("\">\n      <components>\n")
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        for mesh in &node.meshes {
            writeln!(xml, "        <component objectid=\"{}\"/>", mesh_ids[mesh])
                .map_err(|_| ExactProductError::ExportResourceLimit)?;
        }
        for child in &node.children {
            writeln!(
                xml,
                "        <component objectid=\"{}\" transform=\"{}\"/>",
                node_ids[child],
                transform_3mf(nodes[*child].transform)?
            )
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
        }
        xml.write_str("      </components>\n    </object>\n")
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
    }
    xml.write_str("  </resources>\n  <build>\n")
        .map_err(|_| ExactProductError::ExportResourceLimit)?;
    for root in roots {
        writeln!(
            xml,
            "    <item objectid=\"{}\" transform=\"{}\"/>",
            node_ids[root],
            transform_3mf(nodes[*root].transform)?
        )
        .map_err(|_| ExactProductError::ExportResourceLimit)?;
    }
    xml.write_str("  </build>\n</model>\n")
        .map_err(|_| ExactProductError::ExportResourceLimit)?;
    Ok(xml.into_inner())
}

fn transform_3mf(transform: Transform) -> Result<String, ExactProductError> {
    let matrix = transform.matrix();
    let determinant = matrix[0] * (matrix[5] * matrix[10] - matrix[6] * matrix[9])
        - matrix[1] * (matrix[4] * matrix[10] - matrix[6] * matrix[8])
        + matrix[2] * (matrix[4] * matrix[9] - matrix[5] * matrix[8]);
    if matrix.iter().any(|value| !value.is_finite()) || determinant.abs() <= f64::EPSILON {
        return Err(ExactProductError::InvalidMeshExport);
    }
    Ok(format!(
        "{:.17} {:.17} {:.17} {:.17} {:.17} {:.17} {:.17} {:.17} {:.17} {:.17} {:.17} {:.17}",
        matrix[0],
        matrix[1],
        matrix[2],
        matrix[4],
        matrix[5],
        matrix[6],
        matrix[8],
        matrix[9],
        matrix[10],
        matrix[3],
        matrix[7],
        matrix[11]
    ))
}

fn write_xml_escaped(xml: &mut BoundedString, value: &str) -> Result<(), ExactProductError> {
    for character in value.chars() {
        let mut encoded = [0_u8; 4];
        let escaped = match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&apos;",
            '\u{9}'
            | '\u{A}'
            | '\u{D}'
            | '\u{20}'..='\u{D7FF}'
            | '\u{E000}'..='\u{FFFD}'
            | '\u{10000}'..='\u{10FFFF}' => character.encode_utf8(&mut encoded),
            _ => return Err(ExactProductError::InvalidMeshExport),
        };
        xml.write_str(escaped)
            .map_err(|_| ExactProductError::ExportResourceLimit)?;
    }
    Ok(())
}

fn encode_package(
    entries: &[(&str, &[u8])],
    archive_limit: usize,
) -> Result<Vec<u8>, ExactProductError> {
    struct CentralEntry<'a> {
        name: &'a str,
        crc: u32,
        size: u32,
        offset: u32,
    }
    u16::try_from(entries.len()).map_err(|_| ExactProductError::ExportResourceLimit)?;
    let archive_size = entries.iter().try_fold(22_usize, |total, (name, bytes)| {
        u16::try_from(name.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
        u32::try_from(bytes.len()).map_err(|_| ExactProductError::ExportResourceLimit)?;
        let entry_size = 76_usize
            .checked_add(name.len().saturating_mul(2))
            .and_then(|size| size.checked_add(bytes.len()))
            .ok_or(ExactProductError::ExportResourceLimit)?;
        add_with_limit(total, entry_size, archive_limit)
    })?;
    let mut archive = Vec::with_capacity(archive_size);
    let mut central = Vec::with_capacity(entries.len());
    for (name, bytes) in entries {
        let name_bytes = name.as_bytes();
        let name_len =
            u16::try_from(name_bytes.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
        let size = u32::try_from(bytes.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
        let offset =
            u32::try_from(archive.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
        let crc = crc32(bytes);
        archive.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        archive.extend_from_slice(&20_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&crc.to_le_bytes());
        archive.extend_from_slice(&size.to_le_bytes());
        archive.extend_from_slice(&size.to_le_bytes());
        archive.extend_from_slice(&name_len.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(name_bytes);
        archive.extend_from_slice(bytes);
        central.push(CentralEntry {
            name,
            crc,
            size,
            offset,
        });
    }
    let central_offset =
        u32::try_from(archive.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
    for entry in &central {
        let name = entry.name.as_bytes();
        let name_len =
            u16::try_from(name.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
        archive.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        archive.extend_from_slice(&20_u16.to_le_bytes());
        archive.extend_from_slice(&20_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&entry.crc.to_le_bytes());
        archive.extend_from_slice(&entry.size.to_le_bytes());
        archive.extend_from_slice(&entry.size.to_le_bytes());
        archive.extend_from_slice(&name_len.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u16.to_le_bytes());
        archive.extend_from_slice(&0_u32.to_le_bytes());
        archive.extend_from_slice(&entry.offset.to_le_bytes());
        archive.extend_from_slice(name);
    }
    let central_size = u32::try_from(archive.len())
        .map_err(|_| ExactProductError::InvalidMeshExport)?
        .checked_sub(central_offset)
        .ok_or(ExactProductError::InvalidMeshExport)?;
    let count = u16::try_from(central.len()).map_err(|_| ExactProductError::InvalidMeshExport)?;
    archive.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    archive.extend_from_slice(&0_u16.to_le_bytes());
    archive.extend_from_slice(&0_u16.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&central_size.to_le_bytes());
    archive.extend_from_slice(&central_offset.to_le_bytes());
    archive.extend_from_slice(&0_u16.to_le_bytes());
    debug_assert_eq!(archive.len(), archive_size);
    Ok(archive)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_resource_counts_are_checked_without_overflow() {
        assert_eq!(add_with_limit(2, 3, 5), Ok(5));
        assert_eq!(
            add_with_limit(2, 4, 5),
            Err(ExactProductError::ExportResourceLimit)
        );
        assert_eq!(
            add_with_limit(usize::MAX, 1, usize::MAX),
            Err(ExactProductError::ExportResourceLimit)
        );
    }

    #[test]
    fn bounded_xml_refuses_expansion_and_forbidden_characters() {
        let mut xml = BoundedString::new(4);
        assert_eq!(
            write_xml_escaped(&mut xml, "&"),
            Err(ExactProductError::ExportResourceLimit)
        );

        let mut xml = BoundedString::new(32);
        assert_eq!(
            write_xml_escaped(&mut xml, "bad\u{1}name"),
            Err(ExactProductError::InvalidMeshExport)
        );
    }

    #[test]
    fn archive_size_is_checked_before_allocation() {
        let entries = [("a", b"x".as_slice())];
        assert_eq!(
            encode_package(&entries, 100),
            Err(ExactProductError::ExportResourceLimit)
        );
        assert_eq!(encode_package(&entries, 101).unwrap().len(), 101);
    }
}
