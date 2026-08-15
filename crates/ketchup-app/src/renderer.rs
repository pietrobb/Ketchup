use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use ketchup_core::document::{
    DefinitionId, DocumentId, FeatureKind, InstancePath, Snapshot, Transform,
};
use ketchup_core::exact_product::{ExactBodyView, ExactResultRegistry};
use ketchup_interaction::projection::CanonicalInteractionProjection;
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
        });
        self.geometries.insert(fingerprint, Arc::clone(&geometry));
        self.misses += 1;
        geometry
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderInstance {
    pub transform: [f32; 16],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RenderVertex {
    position: [f32; 3],
    normal: [f32; 3],
    barycentric: [f32; 3],
    edge_mask: [f32; 3],
}

#[derive(Clone, Debug)]
pub struct RenderGeometry {
    fingerprint: String,
    vertices: Vec<RenderVertex>,
    indices: Vec<u32>,
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
    document_id: DocumentId,
    source_revision: u64,
    source_digest: String,
    exact_identity: String,
    batches: Vec<RenderBatch>,
}

impl InstancedRenderPlan {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        exact_results: &ExactResultRegistry,
        cache: &mut DerivedRenderCache,
    ) -> Self {
        Self::from_snapshot_with_transform_overrides(
            snapshot,
            exact_results,
            cache,
            &BTreeMap::new(),
        )
    }

    pub fn from_snapshot_with_transform_overrides(
        snapshot: &Snapshot,
        exact_results: &ExactResultRegistry,
        cache: &mut DerivedRenderCache,
        transform_overrides: &BTreeMap<InstancePath, Transform>,
    ) -> Self {
        let projection = CanonicalInteractionProjection::from_snapshot(snapshot);
        let mut batches = BTreeMap::<(DefinitionId, String), RenderBatch>::new();
        let mut definition_geometries =
            BTreeMap::<DefinitionId, (String, Arc<RenderGeometry>)>::new();
        for occurrence in projection
            .occurrences()
            .iter()
            .filter(|occurrence| occurrence.visible)
        {
            let definition_id = occurrence.body.definition_id;
            if let std::collections::btree_map::Entry::Vacant(entry) =
                definition_geometries.entry(definition_id)
            {
                let Some(source) = geometry_source(snapshot, exact_results, definition_id) else {
                    continue;
                };
                let fingerprint = source.fingerprint.clone();
                let geometry = cache.geometry(source);
                entry.insert((fingerprint, geometry));
            }
            let Some((fingerprint, geometry)) = definition_geometries.get(&definition_id) else {
                continue;
            };
            let key = (definition_id, fingerprint.clone());
            let geometry = Arc::clone(geometry);
            batches
                .entry(key)
                .or_insert_with(|| RenderBatch {
                    definition_id,
                    geometry,
                    instances: Vec::new(),
                })
                .instances
                .push(RenderInstance {
                    transform: transform_f32(
                        transform_overrides
                            .get(&occurrence.instance_path)
                            .copied()
                            .unwrap_or(occurrence.canonical_world_transform),
                    ),
                });
        }
        let batches = batches.into_values().collect::<Vec<_>>();
        cache.geometries.retain(|fingerprint, _| {
            batches
                .iter()
                .any(|batch| batch.geometry.fingerprint() == fingerprint)
        });
        Self {
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            exact_identity: exact_identity(snapshot, exact_results),
            batches,
        }
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
        self.exact_identity == exact_identity(snapshot, exact_results)
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

fn exact_identity(snapshot: &Snapshot, exact_results: &ExactResultRegistry) -> String {
    exact_results
        .values()
        .filter(|package| package.is_current(snapshot))
        .map(|package| {
            format!(
                "{}:{}",
                package.definition_id().0,
                package.result_fingerprint()
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

struct GeometrySource {
    fingerprint: String,
    vertices: Vec<RenderVertex>,
    indices: Vec<u32>,
}

fn geometry_source(
    snapshot: &Snapshot,
    exact_results: &ExactResultRegistry,
    definition_id: DefinitionId,
) -> Option<GeometrySource> {
    if let Some(package) = exact_results
        .get(&definition_id)
        .filter(|package| package.is_current(snapshot))
    {
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
        return Some(build_render_geometry(
            "exact",
            &positions,
            &triangles,
            &face_groups,
        ));
    }

    let definition = snapshot.definition(definition_id)?;
    for feature_id in definition.feature_ids() {
        let feature = snapshot.feature(*feature_id)?;
        if let FeatureKind::MeshBody(mesh) = feature.kind() {
            let positions = mesh
                .vertices_mm
                .iter()
                .map(|vertex| vertex.map(|value| value as f32))
                .collect::<Vec<_>>();
            return Some(build_render_geometry(
                "canonical-mesh",
                &positions,
                &mesh.triangles,
                &vec![None::<u8>; mesh.triangles.len()],
            ));
        }
    }

    let occurrence = CanonicalInteractionProjection::from_snapshot(snapshot)
        .occurrences()
        .iter()
        .find(|occurrence| occurrence.body.definition_id == definition_id)?
        .clone();
    let local_box = occurrence.local_box?;
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
    Some(build_render_geometry(
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
    ))
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
                uses.iter()
                    .skip(1)
                    .any(|index| dot(normals[first], normals[*index as usize]).abs() < 0.95)
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
        let edge_mask = [
            feature_edges.contains(&ordered_edge(triangle[1], triangle[2])) as u8 as f32,
            feature_edges.contains(&ordered_edge(triangle[2], triangle[0])) as u8 as f32,
            feature_edges.contains(&ordered_edge(triangle[0], triangle[1])) as u8 as f32,
        ];
        for (corner, index) in triangle.iter().enumerate() {
            let normal = face_group
                .and_then(|group| grouped_vertex_normals.get(&(*index, group)).copied())
                .map_or(*face_normal, normalize);
            let mut barycentric = [0.0; 3];
            barycentric[corner] = 1.0;
            vertices.push(RenderVertex {
                position: positions[*index as usize],
                normal,
                barycentric,
                edge_mask,
            });
        }
    }
    let indices = (0..vertices.len() as u32).collect::<Vec<_>>();
    GeometrySource {
        fingerprint: geometry_fingerprint(kind, &vertices, &indices),
        vertices,
        indices,
    }
}

fn ordered_edge(first: u32, second: u32) -> [u32; 2] {
    if first <= second {
        [first, second]
    } else {
        [second, first]
    }
}

fn triangle_normal(positions: &[[f32; 3]], triangle: [u32; 3]) -> [f32; 3] {
    let first = positions[triangle[0] as usize];
    let second = positions[triangle[1] as usize];
    let third = positions[triangle[2] as usize];
    normalize([
        (second[1] - first[1]) * (third[2] - first[2])
            - (second[2] - first[2]) * (third[1] - first[1]),
        (second[2] - first[2]) * (third[0] - first[0])
            - (second[0] - first[0]) * (third[2] - first[2]),
        (second[0] - first[0]) * (third[1] - first[1])
            - (second[1] - first[1]) * (third[0] - first[0]),
    ])
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = dot(vector, vector).sqrt();
    if length > f32::EPSILON {
        vector.map(|value| value / length)
    } else {
        vector
    }
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn geometry_fingerprint(kind: &str, vertices: &[RenderVertex], indices: &[u32]) -> String {
    let mut fingerprint =
        String::with_capacity(kind.len() + vertices.len() * 108 + indices.len() * 9);
    fingerprint.push_str(kind);
    for vertex in vertices {
        for values in [
            vertex.position,
            vertex.normal,
            vertex.barycentric,
            vertex.edge_mask,
        ] {
            for value in values {
                use std::fmt::Write as _;
                let _ = write!(fingerprint, ":{:08x}", value.to_bits());
            }
        }
    }
    for index in indices {
        use std::fmt::Write as _;
        let _ = write!(fingerprint, ":{index:08x}");
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
}

struct PreparedBatch {
    fingerprint: String,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct GpuFrameDescriptor {
    pub world_to_clip: [f32; 16],
    pub view_depth: [f32; 4],
    pub framebuffer_size: [u32; 2],
    pub viewport: [u32; 4],
}

pub struct GpuInstancedRenderer {
    depth_pipeline: wgpu::RenderPipeline,
    color_pipeline: wgpu::RenderPipeline,
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
            size: 96,
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
        let vertex_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x3];
        let instance_attributes = wgpu::vertex_attr_array![4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4];
        let create_pipeline = |label: &'static str, fragment_entry: &'static str, write_mask| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[
                        wgpu::VertexBufferLayout {
                            array_stride: 48,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &vertex_attributes,
                        },
                        wgpu::VertexBufferLayout {
                            array_stride: 64,
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
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let depth_pipeline = create_pipeline(
            "Ketchup scene depth pipeline",
            "fs_depth",
            wgpu::ColorWrites::empty(),
        );
        let color_pipeline = create_pipeline(
            "Ketchup scene color pipeline",
            "fs_main",
            wgpu::ColorWrites::ALL,
        );
        let depth_target_size = [1, 1];
        let depth_target = create_depth_target(device, target_format, depth_target_size);
        Self {
            depth_pipeline,
            color_pipeline,
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
        let required_depth_bytes =
            u64::from(framebuffer_size[0]) * u64::from(framebuffer_size[1]) * 4;
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
        camera_uniform.extend(u32_bytes(&[framebuffer_size[0], framebuffer_size[1], 0, 0]));
        queue.write_buffer(&self.camera_buffer, 0, &camera_uniform);
        self.prepared.clear();
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
                self.geometries.insert(
                    fingerprint.clone(),
                    GpuGeometry {
                        vertex_buffer,
                        index_buffer,
                        index_count: batch.geometry.indices.len() as u32,
                    },
                );
                self.stats.geometry_uploads += 1;
            }
            let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Ketchup derived instance transforms"),
                contents: &instance_bytes(&batch.instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
            self.prepared.push(PreparedBatch {
                fingerprint,
                instance_buffer,
                instance_count: batch.instances.len() as u32,
            });
            self.stats.instance_uploads += 1;
        }
        self.geometries.retain(|fingerprint, _| {
            self.prepared
                .iter()
                .any(|batch| batch.fingerprint == *fingerprint)
        });
        self.stats.gpu_geometry_entries = self.geometries.len();
        let viewport = frame.viewport;
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
            for prepared in &self.prepared {
                let geometry = &self.geometries[&prepared.fingerprint];
                depth_pass.set_vertex_buffer(0, geometry.vertex_buffer.slice(..));
                depth_pass.set_vertex_buffer(1, prepared.instance_buffer.slice(..));
                depth_pass
                    .set_index_buffer(geometry.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                depth_pass.draw_indexed(0..geometry.index_count, 0, 0..prepared.instance_count);
            }
        }
    }

    pub fn paint(&mut self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_bind_group(0, &self.scene_bind_group, &[]);
        render_pass.set_pipeline(&self.color_pipeline);
        for prepared in &self.prepared {
            let geometry = &self.geometries[&prepared.fingerprint];
            render_pass.set_vertex_buffer(0, geometry.vertex_buffer.slice(..));
            render_pass.set_vertex_buffer(1, prepared.instance_buffer.slice(..));
            render_pass
                .set_index_buffer(geometry.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..geometry.index_count, 0, 0..prepared.instance_count);
            self.stats.draw_calls += 1;
            self.stats.instances_drawn += u64::from(prepared.instance_count);
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
        }
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
            render_pass.set_bind_group(0, &renderer.scene_bind_group, &[]);
            render_pass.set_pipeline(&renderer.color_pipeline);
            for prepared in &renderer.prepared {
                let geometry = &renderer.geometries[&prepared.fingerprint];
                render_pass.set_vertex_buffer(0, geometry.vertex_buffer.slice(..));
                render_pass.set_vertex_buffer(1, prepared.instance_buffer.slice(..));
                render_pass
                    .set_index_buffer(geometry.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..geometry.index_count, 0, 0..prepared.instance_count);
            }
        }
    }
}

fn vertex_bytes(vertices: &[RenderVertex]) -> Vec<u8> {
    vertices
        .iter()
        .flat_map(|vertex| {
            [
                vertex.position,
                vertex.normal,
                vertex.barycentric,
                vertex.edge_mask,
            ]
            .into_iter()
            .flatten()
        })
        .flat_map(f32::to_ne_bytes)
        .collect()
}

fn instance_bytes(instances: &[RenderInstance]) -> Vec<u8> {
    instances
        .iter()
        .flat_map(|instance| row_major_to_columns(instance.transform))
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
    framebuffer_size: vec2<u32>,
    padding: vec2<u32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(0) @binding(1)
var<storage, read_write> pixel_depths: array<atomic<u32>>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) barycentric: vec3<f32>,
    @location(3) edge_mask: vec3<f32>,
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) barycentric: vec3<f32>,
    @location(2) @interpolate(flat) edge_mask: vec3<f32>,
    @location(3) view_depth: f32,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    let world_position = model * vec4<f32>(input.position, 1.0);
    var output: VertexOutput;
    output.clip_position = camera.world_to_clip * world_position;
    output.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    output.barycentric = input.barycentric;
    output.edge_mask = input.edge_mask;
    output.view_depth = dot(camera.view_depth, world_position);
    return output;
}

fn pixel_depth_index(input: VertexOutput) -> u32 {
    let pixel = vec2<u32>(input.clip_position.xy);
    return pixel.y * camera.framebuffer_size.x + pixel.x;
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
fn fs_depth(input: VertexOutput) -> @location(0) vec4<f32> {
    atomicMax(&pixel_depths[pixel_depth_index(input)], depth_priority(input.view_depth));
    return vec4<f32>(0.0);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let depth = atomicLoad(&pixel_depths[pixel_depth_index(input)]);
    if depth_priority(input.view_depth) != depth {
        discard;
    }
    let light_direction = normalize(vec3<f32>(-0.35, -0.45, 0.82));
    let diffuse = 0.62 + 0.38 * max(dot(normalize(input.world_normal), light_direction), 0.0);
    let face_color = vec3<f32>(0.36, 0.42, 0.50) * diffuse;
    let derivative = max(fwidth(input.barycentric), vec3<f32>(0.0001));
    let edge_distance = input.barycentric / derivative;
    let masked_distance = select(
        vec3<f32>(1000000.0),
        edge_distance,
        input.edge_mask > vec3<f32>(0.5),
    );
    let nearest_edge = min(masked_distance.x, min(masked_distance.y, masked_distance.z));
    let edge_alpha = 1.0 - smoothstep(0.8, 1.5, nearest_edge);
    let edge_color = vec3<f32>(0.71, 0.75, 0.81);
    return vec4<f32>(mix(face_color, edge_color, edge_alpha), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::feature_edges;
    use ketchup_core::exact_product::ExactFaceRole;
    use std::collections::BTreeSet;

    const POSITIONS: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    const TRIANGLES: [[u32; 3]; 2] = [[0, 1, 2], [0, 2, 3]];

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
            &[Some(ExactFaceRole::Top), Some(ExactFaceRole::East)],
        );

        assert_eq!(edges.len(), 5);
        assert!(edges.contains(&[0, 2]));
    }

    #[test]
    fn ungrouped_coplanar_mesh_hides_its_triangulation_diagonal() {
        let edges = feature_edges(&POSITIONS, &TRIANGLES, &[None::<u8>, None]);

        assert_eq!(edges, BTreeSet::from([[0, 1], [0, 3], [1, 2], [2, 3]]));
    }
}
