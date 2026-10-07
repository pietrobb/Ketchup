use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use ketchup_geometry::linalg;
use ketchup_interaction::projection::CanonicalInteractionProjection;
use ketchup_model::document::{
    DefinitionId, DocumentId, FeatureId, FeatureKind, InstancePath, Snapshot, Transform,
};
use ketchup_model::exact_product::{ExactBodyPackage, ExactResultRegistry};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use wgpu::util::DeviceExt as _;

pub const RENDER_PLAN_SCHEMA_V1: &str = "ketchup.render-plan.v1";
pub const RENDER_EVALUATOR_V1: &str = "ketchup.renderer.instanced.v1";
pub const RENDER_BACKEND_WGPU_V1: &str = "ketchup.renderer.wgpu.v1";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderCacheStats {
    pub geometry_entries: usize,
    pub geometry_hits: u64,
    pub geometry_misses: u64,
}

#[derive(Default)]
pub struct DerivedRenderCache {
    geometries: BTreeMap<String, Arc<RenderGeometry>>,
    hits: u64,
    misses: u64,
}

impl DerivedRenderCache {
    #[must_use]
    pub fn stats(&self) -> RenderCacheStats {
        RenderCacheStats {
            geometry_entries: self.geometries.len(),
            geometry_hits: self.hits,
            geometry_misses: self.misses,
        }
    }

    fn geometry(&mut self, source: GeometrySource) -> Arc<RenderGeometry> {
        if let Some(geometry) = self.geometries.get(&source.fingerprint) {
            self.hits += 1;
            return Arc::clone(geometry);
        }
        let fingerprint = source.fingerprint.clone();
        let geometry = Arc::new(RenderGeometry {
            fingerprint: source.fingerprint,
            vertices: source.vertices,
            indices: source.indices,
            edge_vertices: source.edge_vertices,
        });
        self.geometries.insert(fingerprint, Arc::clone(&geometry));
        self.misses += 1;
        geometry
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderInstance {
    pub transform: [f32; 16],
    /// Optional persisted sRGB bytes; absent preserves the legacy shaded material.
    pub color: Option<[u8; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RenderVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

/// One corner of the screen-space quad that draws a feature edge: both segment
/// ends, and where along (0 or 1) and to which side (-1 or 1) this corner lies.
#[derive(Clone, Copy, Debug, PartialEq)]
struct EdgeVertex {
    start: [f32; 3],
    end: [f32; 3],
    corner: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct RenderGeometry {
    fingerprint: String,
    vertices: Vec<RenderVertex>,
    indices: Vec<u32>,
    edge_vertices: Vec<EdgeVertex>,
}

impl RenderGeometry {
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    #[must_use]
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }
}

#[derive(Clone, Debug)]
pub struct RenderBatch {
    pub definition_id: DefinitionId,
    pub geometry: Arc<RenderGeometry>,
    pub instances: Vec<RenderInstance>,
}

#[derive(Clone, Debug)]
pub struct InstancedRenderPlan {
    /// Unique per built plan, so the GPU side can keep its uploads for as long
    /// as the same plan is painted (every camera-only frame).
    id: u64,
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    exact_contents_stamp: u64,
    batches: Vec<RenderBatch>,
    /// The instance path of every instance, parallel to `batches`.
    instance_paths: Arc<Vec<Vec<InstancePath>>>,
}

fn next_plan_id() -> u64 {
    static NEXT_PLAN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT_PLAN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl InstancedRenderPlan {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        exact_results: &ExactResultRegistry,
        cache: &mut DerivedRenderCache,
    ) -> Self {
        let projection = CanonicalInteractionProjection::from_snapshot(snapshot);
        let colors = snapshot
            .scene_query()
            .into_iter()
            .map(|occurrence| {
                let color = occurrence.color();
                (occurrence.instance_path, color)
            })
            .collect::<BTreeMap<_, _>>();
        // Matched once: each match re-derives every package's body ownership.
        let mut exact_by_definition = BTreeMap::<DefinitionId, Vec<&Arc<ExactBodyPackage>>>::new();
        for package in exact_results.render_values(snapshot) {
            exact_by_definition
                .entry(package.definition_id())
                .or_default()
                .push(package);
        }
        let mut batches =
            BTreeMap::<(DefinitionId, String), (RenderBatch, Vec<InstancePath>)>::new();
        let mut definition_geometries =
            BTreeMap::<DefinitionId, Vec<(String, Arc<RenderGeometry>)>>::new();
        for occurrence in projection
            .occurrences()
            .iter()
            .filter(|occurrence| occurrence.visible)
        {
            let definition_id = occurrence.body.definition_id;
            if let std::collections::btree_map::Entry::Vacant(entry) =
                definition_geometries.entry(definition_id)
            {
                let sources = geometry_sources(
                    snapshot,
                    exact_by_definition
                        .get(&definition_id)
                        .map_or(&[][..], Vec::as_slice),
                    definition_id,
                );
                if sources.is_empty() {
                    continue;
                }
                entry.insert(
                    sources
                        .into_iter()
                        .map(|source| {
                            let fingerprint = source.fingerprint.clone();
                            let geometry = cache.geometry(source);
                            (fingerprint, geometry)
                        })
                        .collect(),
                );
            }
            let Some(geometries) = definition_geometries.get(&definition_id) else {
                continue;
            };
            for (fingerprint, geometry) in geometries {
                let key = (definition_id, fingerprint.clone());
                let geometry = Arc::clone(geometry);
                let (batch, paths) = batches.entry(key).or_insert_with(|| {
                    (
                        RenderBatch {
                            definition_id,
                            geometry,
                            instances: Vec::new(),
                        },
                        Vec::new(),
                    )
                });
                batch.instances.push(RenderInstance {
                    color: colors.get(&occurrence.instance_path).copied().flatten(),
                    transform: transform_f32(occurrence.canonical_world_transform),
                });
                paths.push(occurrence.instance_path.clone());
            }
        }
        let (batches, instance_paths): (Vec<_>, Vec<_>) = batches.into_values().unzip();
        cache.geometries.retain(|fingerprint, _| {
            batches
                .iter()
                .any(|batch| batch.geometry.fingerprint() == fingerprint)
        });
        Self {
            id: next_plan_id(),
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            exact_contents_stamp: exact_results.contents_stamp(),
            batches,
            instance_paths: Arc::new(instance_paths),
        }
    }

    /// This plan with the instances at `transform_overrides` placed there, for
    /// the live preview of a Move, Rotate or Scale. The geometry stays shared,
    /// so a preview frame does not rebuild the meshes of the whole document.
    #[must_use]
    pub fn with_transform_overrides(
        &self,
        transform_overrides: &BTreeMap<InstancePath, Transform>,
    ) -> Self {
        let mut plan = self.clone();
        plan.id = next_plan_id();
        for (batch, paths) in plan.batches.iter_mut().zip(self.instance_paths.iter()) {
            for (instance, path) in batch.instances.iter_mut().zip(paths) {
                if let Some(transform) = transform_overrides.get(path) {
                    instance.transform = transform_f32(*transform);
                }
            }
        }
        plan
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        RENDER_PLAN_SCHEMA_V1
    }

    #[must_use]
    pub const fn evaluator(&self) -> &'static str {
        RENDER_EVALUATOR_V1
    }

    #[must_use]
    pub const fn backend(&self) -> &'static str {
        RENDER_BACKEND_WGPU_V1
    }

    #[must_use]
    pub fn is_same_revision(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id() && self.source_revision == snapshot.revision_id()
    }

    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.is_same_revision(snapshot) && self.source_digest == snapshot.canonical_digest()
    }

    /// Whether this plan was built from exactly the exact products `snapshot`
    /// can paint right now.
    ///
    /// Exact products arrive from the isolated worker, or are rebound to a new
    /// revision, after the revision they belong to is already committed, so a
    /// plan can be same-revision and yet miss a body that has since become
    /// paintable. Only products that are current for `snapshot` count, because
    /// those are exactly the ones the plan draws geometry from.
    #[must_use]
    pub fn matches_exact_results(
        &self,
        snapshot: &Snapshot,
        exact_results: &ExactResultRegistry,
    ) -> bool {
        self.is_current(snapshot) && self.exact_contents_stamp == exact_results.contents_stamp()
    }

    #[must_use]
    pub fn batches(&self) -> &[RenderBatch] {
        &self.batches
    }

    #[must_use]
    pub fn geometry_count(&self) -> usize {
        self.batches.len()
    }

    #[must_use]
    pub fn instance_count(&self) -> usize {
        self.batches.iter().map(|batch| batch.instances.len()).sum()
    }
}

struct GeometrySource {
    fingerprint: String,
    vertices: Vec<RenderVertex>,
    indices: Vec<u32>,
    edge_vertices: Vec<EdgeVertex>,
}

pub(crate) type PlanarProfileMesh = (Vec<[f64; 3]>, Vec<[u32; 3]>);

fn geometry_sources(
    snapshot: &Snapshot,
    exact_packages: &[&Arc<ExactBodyPackage>],
    definition_id: DefinitionId,
) -> Vec<GeometrySource> {
    let exact = exact_packages
        .iter()
        .map(|package| {
            let positions = package
                .vertices()
                .iter()
                .map(|vertex| vertex.position_mm.map(|value| value as f32))
                .collect::<Vec<_>>();
            let triangles = package
                .triangles()
                .iter()
                .map(|triangle| triangle.vertex_indices)
                .collect::<Vec<_>>();
            let face_groups = package
                .triangles()
                .iter()
                .map(|triangle| triangle.face_role)
                .collect::<Vec<_>>();
            build_render_geometry("exact", &positions, &triangles, &face_groups)
        })
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return exact;
    }

    let Some(definition) = snapshot.definition(definition_id) else {
        return Vec::new();
    };
    if definition
        .feature_ids()
        .last()
        .and_then(|id| snapshot.feature(*id))
        .is_some_and(|feature| matches!(feature.kind(), FeatureKind::Profile { closed: false, .. }))
    {
        return Vec::new();
    }
    for feature_id in definition.feature_ids() {
        let Some(feature) = snapshot.feature(*feature_id) else {
            return Vec::new();
        };
        if let FeatureKind::MeshBody(mesh) = feature.kind() {
            let positions = mesh
                .vertices_mm
                .iter()
                .map(|vertex| vertex.map(|value| value as f32))
                .collect::<Vec<_>>();
            return vec![build_render_geometry(
                "canonical-mesh",
                &positions,
                &mesh.triangles,
                &vec![None::<u8>; mesh.triangles.len()],
            )];
        }
    }
    if let Some((positions, triangles)) =
        canonical_definition_fallback_mesh(snapshot, definition_id)
    {
        let positions = positions
            .into_iter()
            .map(|vertex| vertex.map(|value| value as f32))
            .collect::<Vec<_>>();
        return vec![build_render_geometry(
            "canonical-feature-mesh",
            &positions,
            &triangles,
            &vec![None::<u8>; triangles.len()],
        )];
    }

    let projection = CanonicalInteractionProjection::from_snapshot(snapshot);
    let Some(occurrence) = projection
        .occurrences()
        .iter()
        .find(|occurrence| occurrence.body.definition_id == definition_id)
    else {
        return Vec::new();
    };
    let Some(local_box) = occurrence.local_box else {
        return Vec::new();
    };
    let min = local_box.origin_mm;
    let max = local_box.origin_mm + local_box.size_mm;
    let positions = vec![
        [min.x as f32, min.y as f32, min.z as f32],
        [max.x as f32, min.y as f32, min.z as f32],
        [min.x as f32, max.y as f32, min.z as f32],
        [max.x as f32, max.y as f32, min.z as f32],
        [min.x as f32, min.y as f32, max.z as f32],
        [max.x as f32, min.y as f32, max.z as f32],
        [min.x as f32, max.y as f32, max.z as f32],
        [max.x as f32, max.y as f32, max.z as f32],
    ];
    let triangles = vec![
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 4],
        [1, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    vec![build_render_geometry(
        "canonical-box",
        &positions,
        &triangles,
        &[
            Some(0_u8),
            Some(0),
            Some(1),
            Some(1),
            Some(2),
            Some(2),
            Some(3),
            Some(3),
            Some(4),
            Some(4),
            Some(5),
            Some(5),
        ],
    )]
}

pub(crate) fn canonical_profile_feature_mesh(
    snapshot: &Snapshot,
    feature_id: FeatureId,
) -> Option<PlanarProfileMesh> {
    ketchup_interaction::mesh_projection::canonical_profile_feature_mesh(snapshot, feature_id)
        .map(|(_, positions, triangles)| (positions, triangles))
}

pub(crate) fn canonical_definition_fallback_mesh(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Option<PlanarProfileMesh> {
    ketchup_interaction::profile_surface::canonical_definition_surface(snapshot, definition_id)
        .map(|(_, positions, triangles)| (positions, triangles))
}

pub(crate) fn extrude_planar_profile_mesh(
    mesh: PlanarProfileMesh,
    signed_extent_mm: f64,
) -> Option<PlanarProfileMesh> {
    extrude_planar_profile_mesh_along(mesh, [0.0, 0.0, signed_extent_mm])
}

pub(crate) fn extrude_planar_profile_mesh_along(
    (positions, face_triangles): PlanarProfileMesh,
    offset_mm: [f64; 3],
) -> Option<PlanarProfileMesh> {
    ketchup_interaction::profile_surface::extrude_profile_surface(
        (positions, face_triangles),
        offset_mm,
    )
}

pub(crate) fn feature_edges<G: Copy + Ord>(
    positions: &[[f32; 3]],
    triangles: &[[u32; 3]],
    face_groups: &[Option<G>],
) -> BTreeSet<[u32; 2]> {
    feature_edge_triangles(positions, triangles, face_groups)
        .into_iter()
        .map(|(edge, _)| edge)
        .collect()
}

/// The feature edges of a mesh, each paired with the triangles that use it.
///
/// Overlay painting needs the owning triangles to name the picked face, and
/// rescanning the whole triangle list per edge is quadratic: on an imported
/// mesh of fifty thousand triangles that alone stalls the frame for seconds.
pub(crate) fn feature_edge_triangles<G: Copy + Ord>(
    positions: &[[f32; 3]],
    triangles: &[[u32; 3]],
    face_groups: &[Option<G>],
) -> Vec<([u32; 2], Vec<u32>)> {
    assert_eq!(triangles.len(), face_groups.len());
    let normals = triangles
        .iter()
        .map(|triangle| triangle_normal(positions, *triangle))
        .collect::<Vec<_>>();
    let mut edge_uses = BTreeMap::<[u32; 2], Vec<u32>>::new();
    for (index, triangle) in triangles.iter().enumerate() {
        for [first, second] in [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ] {
            edge_uses
                .entry(ordered_edge(first, second))
                .or_default()
                .push(index as u32);
        }
    }
    edge_uses
        .into_iter()
        .filter(|(_, uses)| {
            let first = uses[0] as usize;
            if uses.len() == 1 {
                true
            } else if uses
                .iter()
                .all(|index| face_groups[*index as usize].is_some())
            {
                uses.iter()
                    .skip(1)
                    .any(|index| face_groups[*index as usize] != face_groups[first])
            } else {
                uses.iter().skip(1).any(|index| {
                    linalg::dot(
                        normals[first].map(f64::from),
                        normals[*index as usize].map(f64::from),
                    ) < 0.95
                })
            }
        })
        .collect()
}

fn build_render_geometry<G: Copy + Ord>(
    kind: &str,
    positions: &[[f32; 3]],
    triangles: &[[u32; 3]],
    face_groups: &[Option<G>],
) -> GeometrySource {
    let feature_edges = feature_edges(positions, triangles, face_groups);
    let face_normals = triangles
        .iter()
        .map(|triangle| triangle_normal(positions, *triangle))
        .collect::<Vec<_>>();
    let mut grouped_vertex_normals = BTreeMap::<(u32, G), [f32; 3]>::new();
    for ((triangle, face_group), normal) in triangles.iter().zip(face_groups).zip(&face_normals) {
        let Some(face_group) = face_group else {
            continue;
        };
        for index in triangle {
            let sum = grouped_vertex_normals
                .entry((*index, *face_group))
                .or_insert([0.0; 3]);
            for axis in 0..3 {
                sum[axis] += normal[axis];
            }
        }
    }

    let mut vertices = Vec::with_capacity(triangles.len() * 3);
    for ((triangle, face_group), face_normal) in
        triangles.iter().zip(face_groups).zip(&face_normals)
    {
        for index in triangle {
            let normal = face_group
                .and_then(|group| grouped_vertex_normals.get(&(*index, group)).copied())
                .map_or(*face_normal, normalize);
            vertices.push(RenderVertex {
                position: positions[*index as usize],
                normal,
            });
        }
    }
    let indices = (0..vertices.len() as u32).collect::<Vec<_>>();
    let edge_vertices = edge_quads(positions, &feature_edges);
    GeometrySource {
        fingerprint: geometry_fingerprint(kind, &vertices, &indices, &edge_vertices),
        vertices,
        indices,
        edge_vertices,
    }
}

/// Each feature edge as its own screen-space quad (two triangles). Edges are drawn
/// as lines rather than inside the face triangles: a face triangulation is a fan of
/// thin slivers along notched outlines, and a line confined to the triangle owning
/// the edge came out dotted wherever the sliver narrowed below a pixel. Faces mesh
/// separately, so the edge two faces share appears twice and is drawn once.
fn edge_quads(positions: &[[f32; 3]], feature_edges: &BTreeSet<[u32; 2]>) -> Vec<EdgeVertex> {
    let mut segments = BTreeSet::new();
    for [first, second] in feature_edges {
        let ends = [positions[*first as usize], positions[*second as usize]];
        let [start, end] = ends.map(|point| point.map(f32::to_bits));
        match start.cmp(&end) {
            std::cmp::Ordering::Less => segments.insert([start, end]),
            std::cmp::Ordering::Greater => segments.insert([end, start]),
            std::cmp::Ordering::Equal => false,
        };
    }
    segments
        .into_iter()
        .flat_map(|segment| {
            let [start, end] = segment.map(|point| point.map(f32::from_bits));
            [
                [0.0, -1.0],
                [1.0, -1.0],
                [1.0, 1.0],
                [0.0, -1.0],
                [1.0, 1.0],
                [0.0, 1.0],
            ]
            .map(|corner| EdgeVertex { start, end, corner })
        })
        .collect()
}

fn ordered_edge(first: u32, second: u32) -> [u32; 2] {
    if first <= second {
        [first, second]
    } else {
        [second, first]
    }
}

fn triangle_normal(positions: &[[f32; 3]], triangle: [u32; 3]) -> [f32; 3] {
    let [first, second, third] = triangle.map(|index| positions[index as usize].map(f64::from));
    unit(linalg::cross(
        linalg::sub(second, first),
        linalg::sub(third, first),
    ))
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    unit(vector.map(f64::from))
}

/// The unit vector in GPU single precision; a degenerate vector stays as is.
#[allow(clippy::cast_possible_truncation)]
fn unit(vector: [f64; 3]) -> [f32; 3] {
    linalg::normalize(vector)
        .unwrap_or(vector)
        .map(|value| value as f32)
}

fn geometry_fingerprint(
    kind: &str,
    vertices: &[RenderVertex],
    indices: &[u32],
    edge_vertices: &[EdgeVertex],
) -> String {
    use std::fmt::Write as _;
    let mut fingerprint = String::with_capacity(
        kind.len() + vertices.len() * 54 + indices.len() * 9 + edge_vertices.len() / 6 * 54,
    );
    fingerprint.push_str(kind);
    for vertex in vertices {
        for value in vertex.position.into_iter().chain(vertex.normal) {
            let _ = write!(fingerprint, ":{:08x}", value.to_bits());
        }
    }
    for index in indices {
        let _ = write!(fingerprint, ":{index:08x}");
    }
    fingerprint.push_str(":edges");
    for edge in edge_vertices.iter().step_by(6) {
        for value in edge.start.into_iter().chain(edge.end) {
            let _ = write!(fingerprint, ":{:08x}", value.to_bits());
        }
    }
    fingerprint
}

fn transform_f32(transform: Transform) -> [f32; 16] {
    transform.matrix().map(|value| value as f32)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GpuRenderStats {
    pub gpu_geometry_entries: usize,
    pub geometry_uploads: u64,
    pub geometry_cache_hits: u64,
    pub instance_uploads: u64,
    pub draw_calls: u64,
    pub instances_drawn: u64,
}

struct GpuGeometry {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    edge_buffer: Option<wgpu::Buffer>,
    edge_vertex_count: u32,
}

struct PreparedBatch {
    fingerprint: String,
    instances: std::ops::Range<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct GpuFrameDescriptor {
    pub world_to_clip: [f32; 16],
    pub view_depth: [f32; 4],
    /// Section plane as unit normal and offset; fragments with `normal·p > offset` are
    /// cut away. All zero shows everything.
    pub section: [f32; 4],
    pub framebuffer_size: [u32; 2],
    pub viewport: [u32; 4],
}

pub struct GpuInstancedRenderer {
    depth_pipeline: wgpu::RenderPipeline,
    color_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    scene_bind_group: wgpu::BindGroup,
    scene_bind_group_layout: wgpu::BindGroupLayout,
    depth_buffer: wgpu::Buffer,
    depth_capacity: u64,
    depth_target: wgpu::TextureView,
    depth_target_size: [u32; 2],
    target_format: wgpu::TextureFormat,
    geometries: BTreeMap<String, GpuGeometry>,
    prepared: Vec<PreparedBatch>,
    /// All instances of the prepared plan, one range per batch.
    instance_buffer: Option<wgpu::Buffer>,
    prepared_plan: Option<u64>,
    stats: GpuRenderStats,
}

fn create_depth_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Ketchup per-pixel scene depths"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_depth_target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: [u32; 2],
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Ketchup scene depth-pass target"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_scene_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    camera_buffer: &wgpu::Buffer,
    depth_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Ketchup scene bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: depth_buffer.as_entire_binding(),
            },
        ],
    })
}

impl GpuInstancedRenderer {
    #[must_use]
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ketchup instanced scene shader"),
            source: wgpu::ShaderSource::Wgsl(INSTANCED_SHADER.into()),
        });
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ketchup camera uniform"),
            size: 112,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Ketchup scene bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let depth_buffer = create_depth_buffer(device, 4);
        let scene_bind_group = create_scene_bind_group(
            device,
            &scene_bind_group_layout,
            &camera_buffer,
            &depth_buffer,
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Ketchup instanced scene pipeline layout"),
            bind_group_layouts: &[&scene_bind_group_layout],
            push_constant_ranges: &[],
        });
        let vertex_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let edge_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let instance_attributes = wgpu::vertex_attr_array![4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4];
        let surface_layout = wgpu::VertexBufferLayout {
            array_stride: 24,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vertex_attributes,
        };
        let edge_layout = wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &edge_attributes,
        };
        let create_pipeline = |label: &'static str,
                               vertex_entry: &'static str,
                               vertex_layout: wgpu::VertexBufferLayout<'_>,
                               fragment_entry: &'static str,
                               write_mask| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex_entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[
                        vertex_layout,
                        wgpu::VertexBufferLayout {
                            array_stride: 80,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &instance_attributes,
                        },
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment_entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask,
                    })],
                }),
                // Back faces are culled in the shader unless a section plane shows them.
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let depth_pipeline = create_pipeline(
            "Ketchup scene depth pipeline",
            "vs_main",
            surface_layout.clone(),
            "fs_depth",
            wgpu::ColorWrites::empty(),
        );
        let color_pipeline = create_pipeline(
            "Ketchup scene color pipeline",
            "vs_main",
            surface_layout,
            "fs_main",
            wgpu::ColorWrites::ALL,
        );
        let edge_pipeline = create_pipeline(
            "Ketchup scene edge pipeline",
            "vs_edge",
            edge_layout,
            "fs_edge",
            wgpu::ColorWrites::ALL,
        );
        let depth_target_size = [1, 1];
        let depth_target = create_depth_target(device, target_format, depth_target_size);
        Self {
            depth_pipeline,
            color_pipeline,
            edge_pipeline,
            camera_buffer,
            scene_bind_group,
            scene_bind_group_layout,
            depth_buffer,
            depth_capacity: 4,
            depth_target,
            depth_target_size,
            target_format,
            geometries: BTreeMap::new(),
            prepared: Vec::new(),
            instance_buffer: None,
            prepared_plan: None,
            stats: GpuRenderStats::default(),
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        plan: &InstancedRenderPlan,
        frame: GpuFrameDescriptor,
    ) {
        let framebuffer_size = frame.framebuffer_size.map(|value| value.max(1));
        // Two depths per pixel: the exact nearest surface, and the same pushed back by
        // its slope, against which the edge lines are tested.
        let required_depth_bytes =
            u64::from(framebuffer_size[0]) * u64::from(framebuffer_size[1]) * 8;
        if required_depth_bytes > self.depth_capacity {
            self.depth_buffer = create_depth_buffer(device, required_depth_bytes);
            self.scene_bind_group = create_scene_bind_group(
                device,
                &self.scene_bind_group_layout,
                &self.camera_buffer,
                &self.depth_buffer,
            );
            self.depth_capacity = required_depth_bytes;
        }
        if framebuffer_size != self.depth_target_size {
            self.depth_target = create_depth_target(device, self.target_format, framebuffer_size);
            self.depth_target_size = framebuffer_size;
        }
        encoder.clear_buffer(&self.depth_buffer, 0, None);
        let mut camera_uniform = f32_bytes(&frame.world_to_clip);
        camera_uniform.extend(f32_bytes(&frame.view_depth));
        camera_uniform.extend(f32_bytes(&frame.section));
        let viewport = frame.viewport;
        camera_uniform.extend(u32_bytes(&[
            framebuffer_size[0],
            framebuffer_size[1],
            viewport[2].max(1),
            viewport[3].max(1),
        ]));
        queue.write_buffer(&self.camera_buffer, 0, &camera_uniform);
        if self.prepared_plan != Some(plan.id) {
            self.upload_plan(device, plan);
        }
        if viewport[2] > 0 && viewport[3] > 0 {
            let mut depth_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Ketchup scene depth preparation"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.depth_target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            depth_pass.set_viewport(
                viewport[0] as f32,
                viewport[1] as f32,
                viewport[2] as f32,
                viewport[3] as f32,
                0.0,
                1.0,
            );
            depth_pass.set_scissor_rect(viewport[0], viewport[1], viewport[2], viewport[3]);
            depth_pass.set_bind_group(0, &self.scene_bind_group, &[]);
            depth_pass.set_pipeline(&self.depth_pipeline);
            self.draw_batches(&mut depth_pass);
        }
    }

    /// Upload what a newly built plan needs: geometry not yet on the GPU and
    /// one buffer with every instance. Camera-only frames reuse both.
    fn upload_plan(&mut self, device: &wgpu::Device, plan: &InstancedRenderPlan) {
        self.prepared.clear();
        let mut instances = Vec::new();
        for batch in plan.batches() {
            let fingerprint = batch.geometry.fingerprint().to_owned();
            if self.geometries.contains_key(&fingerprint) {
                self.stats.geometry_cache_hits += 1;
            } else {
                let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Ketchup derived geometry vertices"),
                    contents: &vertex_bytes(&batch.geometry.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Ketchup derived geometry indices"),
                    contents: &u32_bytes(&batch.geometry.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                let edges = &batch.geometry.edge_vertices;
                let edge_buffer = (!edges.is_empty()).then(|| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Ketchup derived geometry edges"),
                        contents: &edge_vertex_bytes(edges),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                });
                self.geometries.insert(
                    fingerprint.clone(),
                    GpuGeometry {
                        vertex_buffer,
                        index_buffer,
                        index_count: batch.geometry.indices.len() as u32,
                        edge_buffer,
                        edge_vertex_count: edges.len() as u32,
                    },
                );
                self.stats.geometry_uploads += 1;
            }
            let first = instances.len() as u32;
            instances.extend_from_slice(&batch.instances);
            self.prepared.push(PreparedBatch {
                fingerprint,
                instances: first..instances.len() as u32,
            });
            self.stats.instance_uploads += 1;
        }
        let used = self
            .prepared
            .iter()
            .map(|batch| batch.fingerprint.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        self.geometries
            .retain(|fingerprint, _| used.contains(fingerprint.as_str()));
        self.stats.gpu_geometry_entries = self.geometries.len();
        self.instance_buffer = (!instances.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Ketchup derived instance transforms"),
                contents: &instance_bytes(&instances),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        self.prepared_plan = Some(plan.id);
    }

    fn draw_batches(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(instance_buffer) = &self.instance_buffer else {
            return;
        };
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        for prepared in &self.prepared {
            let geometry = &self.geometries[&prepared.fingerprint];
            pass.set_vertex_buffer(0, geometry.vertex_buffer.slice(..));
            pass.set_index_buffer(geometry.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..geometry.index_count, 0, prepared.instances.clone());
        }
    }

    /// Faces, then their edge lines over them.
    fn draw_scene(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.scene_bind_group, &[]);
        pass.set_pipeline(&self.color_pipeline);
        self.draw_batches(pass);
        let Some(instance_buffer) = &self.instance_buffer else {
            return;
        };
        pass.set_pipeline(&self.edge_pipeline);
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        for prepared in &self.prepared {
            let geometry = &self.geometries[&prepared.fingerprint];
            if let Some(edge_buffer) = &geometry.edge_buffer {
                pass.set_vertex_buffer(0, edge_buffer.slice(..));
                pass.draw(0..geometry.edge_vertex_count, prepared.instances.clone());
            }
        }
    }

    pub fn paint(&mut self, render_pass: &mut wgpu::RenderPass<'_>) {
        self.draw_scene(render_pass);
        if self.instance_buffer.is_some() {
            for prepared in &self.prepared {
                self.stats.draw_calls += 1;
                self.stats.instances_drawn += u64::from(prepared.instances.len() as u32);
            }
        }
    }

    #[must_use]
    pub fn stats(&self) -> GpuRenderStats {
        self.stats
    }
}

#[derive(Clone)]
pub struct ScenePaintCallback {
    plan: Arc<InstancedRenderPlan>,
    viewport: eframe::egui::Rect,
    world_to_clip: [f32; 16],
    view_depth: [f32; 4],
    section: [f32; 4],
}

impl ScenePaintCallback {
    #[must_use]
    pub fn new(
        plan: Arc<InstancedRenderPlan>,
        viewport: eframe::egui::Rect,
        world_to_clip: [f32; 16],
        view_depth: [f32; 4],
    ) -> Self {
        Self {
            plan,
            viewport,
            world_to_clip,
            view_depth,
            section: [0.0; 4],
        }
    }

    /// Cut the scene open along a section plane (see [`GpuFrameDescriptor::section`]).
    #[must_use]
    pub fn with_section(mut self, section: [f32; 4]) -> Self {
        self.section = section;
        self
    }
}

impl CallbackTrait for ScenePaintCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(renderer) = callback_resources.get_mut::<GpuInstancedRenderer>() {
            let info = eframe::egui::PaintCallbackInfo {
                viewport: self.viewport,
                clip_rect: self.viewport,
                pixels_per_point: screen_descriptor.pixels_per_point,
                screen_size_px: screen_descriptor.size_in_pixels,
            };
            let viewport = info.viewport_in_pixels();
            renderer.prepare(
                device,
                queue,
                egui_encoder,
                &self.plan,
                GpuFrameDescriptor {
                    world_to_clip: self.world_to_clip,
                    view_depth: self.view_depth,
                    section: self.section,
                    framebuffer_size: screen_descriptor.size_in_pixels,
                    viewport: [
                        u32::try_from(viewport.left_px).unwrap_or(0),
                        u32::try_from(viewport.top_px).unwrap_or(0),
                        u32::try_from(viewport.width_px).unwrap_or(0),
                        u32::try_from(viewport.height_px).unwrap_or(0),
                    ],
                },
            );
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        if let Some(renderer) = callback_resources.get::<GpuInstancedRenderer>() {
            renderer.draw_scene(render_pass);
        }
    }
}

fn vertex_bytes(vertices: &[RenderVertex]) -> Vec<u8> {
    vertices
        .iter()
        .flat_map(|vertex| [vertex.position, vertex.normal].into_iter().flatten())
        .flat_map(f32::to_ne_bytes)
        .collect()
}

fn edge_vertex_bytes(vertices: &[EdgeVertex]) -> Vec<u8> {
    vertices
        .iter()
        .flat_map(|vertex| {
            vertex
                .start
                .into_iter()
                .chain(vertex.end)
                .chain(vertex.corner)
        })
        .flat_map(f32::to_ne_bytes)
        .collect()
}

fn instance_bytes(instances: &[RenderInstance]) -> Vec<u8> {
    instances
        .iter()
        .flat_map(|instance| {
            // Convert sRGB to linear exactly once per instance, not per vertex.
            let color = instance.color.map_or([0.0; 4], |rgb| {
                let linear = rgb.map(|byte| {
                    let value = f32::from(byte) / 255.0;
                    if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
                });
                [linear[0], linear[1], linear[2], 1.0]
            });
            row_major_to_columns(instance.transform)
                .into_iter()
                .chain(color)
        })
        .flat_map(f32::to_ne_bytes)
        .collect()
}

fn row_major_to_columns(matrix: [f32; 16]) -> [f32; 16] {
    [
        matrix[0], matrix[4], matrix[8], matrix[12], matrix[1], matrix[5], matrix[9], matrix[13],
        matrix[2], matrix[6], matrix[10], matrix[14], matrix[3], matrix[7], matrix[11], matrix[15],
    ]
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

fn u32_bytes(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

const INSTANCED_SHADER: &str = r#"
struct Camera {
    world_to_clip: mat4x4<f32>,
    view_depth: vec4<f32>,
    section: vec4<f32>,
    framebuffer_size: vec2<u32>,
    viewport_size: vec2<u32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

// Two entries per pixel: [2i] the nearest surface depth, [2i + 1] the nearest
// surface depth pushed back by its slope, for the edge lines.
@group(0) @binding(1)
var<storage, read_write> pixel_depths: array<atomic<u32>>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
    @location(8) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) view_depth: f32,
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) world_position: vec3<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    let world_position = model * vec4<f32>(input.position, 1.0);
    var output: VertexOutput;
    output.clip_position = camera.world_to_clip * world_position;
    output.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    output.color = input.color;
    output.view_depth = dot(camera.view_depth, world_position);
    output.world_position = world_position.xyz;
    return output;
}

fn section_open() -> bool {
    return dot(camera.section.xyz, camera.section.xyz) > 0.5;
}

fn cut_away(world_position: vec3<f32>) -> bool {
    return section_open() && dot(camera.section.xyz, world_position) > camera.section.w;
}

// Cut away by the section plane, or a back face that only an open cut shows.
fn hidden(input: VertexOutput, front_facing: bool) -> bool {
    if !section_open() {
        return !front_facing;
    }
    return cut_away(input.world_position);
}

fn pixel_depth_index(clip_position: vec4<f32>) -> u32 {
    let pixel = vec2<u32>(clip_position.xy);
    return 2u * (pixel.y * camera.framebuffer_size.x + pixel.x);
}

fn depth_priority(view_depth: f32) -> u32 {
    let bits = bitcast<u32>(view_depth);
    let ordered = select(
        bits ^ 0x80000000u,
        ~bits,
        (bits & 0x80000000u) != 0u,
    );
    return ~ordered;
}

@fragment
fn fs_depth(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    if hidden(input, front_facing) {
        discard;
    }
    let index = pixel_depth_index(input.clip_position);
    atomicMax(&pixel_depths[index], depth_priority(input.view_depth));
    // An edge line is up to 1.5 px from the edge it draws, where an adjacent face
    // already lies deeper by its depth slope; two pixels of slope plus a rounding
    // margin keep the edge in front of its own faces but behind any other surface.
    // not a tolerance: f32 depth rounding margin of the edge pass.
    let slack = 2.0 * fwidth(input.view_depth) + max(abs(input.view_depth) * 4e-6, 1e-3);
    atomicMax(&pixel_depths[index + 1u], depth_priority(input.view_depth + slack));
    return vec4<f32>(0.0);
}

@fragment
fn fs_main(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    if hidden(input, front_facing) {
        discard;
    }
    let depth = atomicLoad(&pixel_depths[pixel_depth_index(input.clip_position)]);
    if depth_priority(input.view_depth) != depth {
        discard;
    }
    if !front_facing {
        // The inside of a solid seen through the cut marks the section.
        return vec4<f32>(0.84, 0.29, 0.25, 1.0);
    }
    let light_direction = normalize(vec3<f32>(-0.35, -0.45, 0.82));
    let diffuse = 0.62 + 0.38 * max(dot(normalize(input.world_normal), light_direction), 0.0);
    let face_color = mix(vec3<f32>(0.36, 0.42, 0.50) * diffuse, input.color.rgb, input.color.a);
    return vec4<f32>(face_color, 1.0);
}

struct EdgeInput {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) corner: vec2<f32>,
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
};

struct EdgeOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) view_depth: f32,
    // Pixels along the segment from its start, and across it from its centre line.
    @location(2) @interpolate(linear) line: vec2<f32>,
    @location(3) @interpolate(flat) length: f32,
};

// Half the quad width in pixels: the line core plus its anti-aliased rim.
const EDGE_REACH: f32 = 1.5;

@vertex
fn vs_edge(input: EdgeInput) -> EdgeOutput {
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    var world_start = model * vec4<f32>(input.start, 1.0);
    var world_end = model * vec4<f32>(input.end, 1.0);
    var clip_start = camera.world_to_clip * world_start;
    var clip_end = camera.world_to_clip * world_end;
    var output: EdgeOutput;
    // In perspective, keep only the part of the segment in front of the eye.
    // not a tolerance: clip-space w below which a point is behind the eye.
    let near = 1e-6;
    if clip_start.w < near && clip_end.w < near {
        output.clip_position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return output;
    }
    if clip_start.w < near {
        let t = (near - clip_start.w) / (clip_end.w - clip_start.w);
        clip_start = mix(clip_start, clip_end, t);
        world_start = mix(world_start, world_end, t);
    } else if clip_end.w < near {
        let t = (near - clip_end.w) / (clip_start.w - clip_end.w);
        clip_end = mix(clip_end, clip_start, t);
        world_end = mix(world_end, world_start, t);
    }
    let pixels = vec2<f32>(camera.viewport_size) * 0.5;
    let delta = clip_end.xy / clip_end.w * pixels - clip_start.xy / clip_start.w * pixels;
    let span = length(delta);
    // not a tolerance: a segment shorter than this many pixels has no direction.
    let direction = select(vec2<f32>(1.0, 0.0), delta / span, span > 1e-6);
    let normal = vec2<f32>(-direction.y, direction.x);
    let along = input.corner.x;
    let across = input.corner.y * EDGE_REACH;
    let outward = 2.0 * along - 1.0;
    let clip = mix(clip_start, clip_end, along);
    let offset = (direction * outward * EDGE_REACH + normal * across) / pixels;
    let world = mix(world_start, world_end, along);
    output.clip_position = vec4<f32>(clip.xy + offset * clip.w, clip.zw);
    output.world_position = world.xyz;
    output.view_depth = dot(camera.view_depth, world);
    output.line = vec2<f32>(along * span + outward * EDGE_REACH, across);
    output.length = span;
    return output;
}

@fragment
fn fs_edge(input: EdgeOutput) -> @location(0) vec4<f32> {
    if cut_away(input.world_position) {
        discard;
    }
    // Behind the nearest surface (beyond the slack of its own slope): hidden.
    let surface = atomicLoad(&pixel_depths[pixel_depth_index(input.clip_position) + 1u]);
    if depth_priority(input.view_depth) < surface {
        discard;
    }
    let past_end = max(max(-input.line.x, input.line.x - input.length), 0.0);
    let distance = length(vec2<f32>(past_end, input.line.y));
    let alpha = 1.0 - smoothstep(0.8, EDGE_REACH, distance);
    if alpha <= 0.0 {
        discard;
    }
    return vec4<f32>(0.71, 0.75, 0.81, alpha);
}
"#;

#[cfg(test)]
mod tests {
    use super::{RenderInstance, extrude_planar_profile_mesh, feature_edges, instance_bytes};
    use ketchup_interaction::mesh_projection::segment_profile_mesh;
    use ketchup_model::document::ProfileSegment;
    use ketchup_model::exact_product::ExactFaceRole;
    use std::collections::BTreeSet;

    const POSITIONS: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    const TRIANGLES: [[u32; 3]; 2] = [[0, 1, 2], [0, 2, 3]];

    #[test]
    fn instance_color_bytes_keep_transform_stride_and_linear_srgb() {
        let instances = [None, Some([0, 128, 255])].map(|color| RenderInstance {
            transform: [0.0; 16],
            color,
        });
        let bytes = instance_bytes(&instances);
        assert_eq!(bytes.len(), 160);
        let floats = bytes
            .chunks_exact(4)
            .map(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(&floats[16..20], &[0.0; 4]);
        assert_eq!(floats[36], 0.0);
        assert!((floats[37] - 0.2158605).abs() < 0.000001);
        assert_eq!(&floats[38..40], &[1.0, 1.0]);
    }

    #[test]
    fn same_cad_face_keeps_only_quad_boundary() {
        let edges = feature_edges(
            &POSITIONS,
            &TRIANGLES,
            &[Some(ExactFaceRole::Top), Some(ExactFaceRole::Top)],
        );

        assert_eq!(edges, BTreeSet::from([[0, 1], [0, 3], [1, 2], [2, 3]]));
        assert!(!edges.contains(&[0, 2]));
    }

    #[test]
    fn different_cad_faces_keep_their_shared_edge() {
        let edges = feature_edges(
            &POSITIONS,
            &TRIANGLES,
            &[Some(ExactFaceRole::Top), Some(ExactFaceRole::LinearSide)],
        );

        assert_eq!(edges.len(), 5);
        assert!(edges.contains(&[0, 2]));
    }

    #[test]
    fn ungrouped_coplanar_mesh_hides_its_triangulation_diagonal() {
        let edges = feature_edges(&POSITIONS, &TRIANGLES, &[None::<u8>, None]);

        assert_eq!(edges, BTreeSet::from([[0, 1], [0, 3], [1, 2], [2, 3]]));
    }

    #[test]
    fn circular_profile_mesh_follows_the_circle_instead_of_its_bounds() {
        let segments = [
            ProfileSegment::CircularArc {
                start_mm: [10.0, 0.0],
                end_mm: [-10.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
            ProfileSegment::CircularArc {
                start_mm: [-10.0, 0.0],
                end_mm: [10.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
        ];

        let (positions, triangles) = segment_profile_mesh(&segments).unwrap();

        assert_eq!(positions.len(), 64);
        assert_eq!(triangles.len(), 62);
        assert!(positions.iter().all(|point| {
            ((point[0].hypot(point[1]) - 10.0).abs() <= 1.0e-9) && point[2] == 0.0
        }));
    }

    #[test]
    fn circular_profile_extrusion_preview_has_curved_caps_and_side_walls() {
        let segments = [
            ProfileSegment::CircularArc {
                start_mm: [10.0, 0.0],
                end_mm: [-10.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
            ProfileSegment::CircularArc {
                start_mm: [-10.0, 0.0],
                end_mm: [10.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
        ];
        let planar = segment_profile_mesh(&segments).unwrap();

        let (positions, triangles) = extrude_planar_profile_mesh(planar, 30.0).unwrap();

        assert_eq!(positions.len(), 128);
        assert_eq!(triangles.len(), 252);
        assert_eq!(
            positions
                .iter()
                .map(|position| position[2])
                .fold(f64::INFINITY, f64::min),
            0.0
        );
        assert_eq!(
            positions
                .iter()
                .map(|position| position[2])
                .fold(f64::NEG_INFINITY, f64::max),
            30.0
        );
        assert!(positions.iter().all(|point| {
            (point[0].hypot(point[1]) - 10.0).abs() <= 1.0e-9
                && (point[2] == 0.0 || point[2] == 30.0)
        }));
        assert!(triangles.iter().any(|triangle| {
            let z = triangle.map(|index| positions[index as usize][2]);
            z.contains(&0.0) && z.contains(&30.0)
        }));
    }

    #[test]
    fn arc_profile_mesh_follows_the_arc_and_its_closing_chord() {
        let segments = [
            ProfileSegment::CircularArc {
                start_mm: [-10.0, 0.0],
                end_mm: [10.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: true,
            },
            ProfileSegment::Line {
                start_mm: [10.0, 0.0],
                end_mm: [-10.0, 0.0],
            },
        ];

        let (positions, triangles) = segment_profile_mesh(&segments).unwrap();

        assert_eq!(positions.len(), 33);
        assert_eq!(triangles.len(), 31);
        assert!(positions.iter().any(|point| point[1] > 9.9));
        assert!(positions.iter().all(|point| point[1] >= -1.0e-9));
    }
}
