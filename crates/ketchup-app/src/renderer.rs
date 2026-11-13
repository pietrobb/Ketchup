use crate::delivery::Deliver as _;
use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use ketchup_geometry::linalg;
use ketchup_interaction::projection::CanonicalInteractionProjection;
use ketchup_manufacturing::presentation::{PresentationTexture, TextureError};
use ketchup_model::document::{
    DefinitionId, DocumentId, FeatureId, FeatureKind, InstancePath, Snapshot, Transform,
};
use ketchup_model::document::{MeshImage, MeshLook};
use ketchup_model::exact_product::{ExactBodyPackage, ExactResultRegistry};
use ketchup_model::persistence::Blobs;
use ketchup_rejection::{Reason, dropped};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use wgpu::util::DeviceExt as _;

pub mod underlay;

#[cfg(test)]
mod picture_queue_tests;

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
    /// Pictures by blob, decoded on a thread of their own (review 2026-11-12 R1112-SKP-500:
    /// a large one froze the window for seconds).
    pictures: BTreeMap<String, PictureState>,
    /// Pictures to decode, to the thread that decodes them one after another.
    decoder: Option<mpsc::Sender<PictureJob>>,
    /// Decoded pictures by blob, from that thread.
    decoded: Option<mpsc::Receiver<DecodedPicture>>,
    /// Set when the cache goes with its document: the thread decodes none of the
    /// pictures still queued.
    cancelled: Arc<AtomicBool>,
    hits: u64,
    misses: u64,
}

impl Drop for DerivedRenderCache {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// A picture to decode and its bytes, if the document has them.
type PictureJob = (MeshImage, Option<Arc<Vec<u8>>>);
/// A decoded picture by its blob; `None` when it cannot be shown.
type DecodedPicture = (String, Option<Arc<RenderPicture>>);

/// Decodes the `jobs` one after another until they end or are `cancelled`. A picture
/// whose decoding panics shows plain; the ones after it still decode.
fn decode_pictures(
    jobs: &mpsc::Receiver<PictureJob>,
    sender: &mpsc::Sender<DecodedPicture>,
    cancelled: &AtomicBool,
    decode: impl Fn(&MeshImage, Option<&Vec<u8>>) -> Option<Arc<RenderPicture>>,
) {
    for (image, bytes) in jobs {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        let picture = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            decode(&image, bytes.as_deref())
        }))
        .unwrap_or_else(|payload| {
            ketchup_rejection::log!(
                "the picture {} is shown as a plain face: its decoding broke: {}",
                image.blob,
                crate::bounded_parser::panic_message(payload.as_ref())
            );
            None
        });
        sender.deliver((image.blob, picture));
    }
}

/// Where the decoding of a picture is.
enum PictureState {
    Decoding,
    /// `None` when the blob cannot be shown as a picture.
    Decoded(Option<Arc<RenderPicture>>),
}

/// Marks the fingerprint of a geometry drawn without the picture it waits for, so the GPU
/// keeps it apart from the one drawn with it.
const AWAITING_PICTURE: &str = ":awaiting-picture";

impl DerivedRenderCache {
    #[must_use]
    pub fn stats(&self) -> RenderCacheStats {
        RenderCacheStats {
            geometry_entries: self.geometries.len(),
            geometry_hits: self.hits,
            geometry_misses: self.misses,
        }
    }

    /// Takes the pictures decoded since; whether any was, so a plan drawn without them is
    /// out of date.
    pub(crate) fn receive_pictures(&mut self) -> bool {
        let Some(decoded) = &self.decoded else {
            return false;
        };
        let mut received = false;
        loop {
            match decoded.try_recv() {
                Ok((blob, picture)) => {
                    if let Some(state) = self.pictures.get_mut(&blob) {
                        *state = PictureState::Decoded(picture);
                        received = true;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // The thread is gone; what it had not decoded is shown plain.
                    ketchup_rejection::log!("the thread that decodes pictures stopped");
                    for state in self.pictures.values_mut() {
                        if matches!(state, PictureState::Decoding) {
                            *state = PictureState::Decoded(None);
                            received = true;
                        }
                    }
                    self.decoder = None;
                    self.decoded = None;
                    break;
                }
            }
        }
        received
    }

    /// Whether a picture is still being decoded.
    pub(crate) fn decoding_pictures(&self) -> bool {
        self.pictures
            .values()
            .any(|state| matches!(state, PictureState::Decoding))
    }

    /// Waits until every picture asked for is decoded.
    #[cfg(test)]
    pub(crate) fn wait_for_pictures(&mut self) {
        while self.decoding_pictures() {
            let Some(decoded) = &self.decoded else {
                return;
            };
            match decoded.recv() {
                Ok((blob, picture)) => {
                    if let Some(state) = self.pictures.get_mut(&blob) {
                        *state = PictureState::Decoded(picture);
                    }
                }
                Err(mpsc::RecvError) => {
                    self.receive_pictures();
                }
            }
        }
    }

    /// The decoded picture `image` names; `None` while it is being decoded.
    fn picture(&mut self, image: &MeshImage, blobs: &Blobs) -> Option<Option<Arc<RenderPicture>>> {
        if let Some(state) = self.pictures.get(&image.blob) {
            return match state {
                PictureState::Decoding => None,
                PictureState::Decoded(picture) => Some(picture.clone()),
            };
        }
        let job = (image.clone(), blobs.get(&image.blob).cloned());
        let sent = match &self.decoder {
            Some(decoder) => decoder.send(job).map_err(|mpsc::SendError(job)| job),
            None => Err(job),
        };
        if let Err(job) = sent
            && let Err(error) = self.start_decoder(job)
        {
            // No thread to decode it on: shown in the face's own colour.
            ketchup_rejection::log!(
                "the picture {} is shown as a plain face: {error}",
                image.blob
            );
            self.pictures
                .insert(image.blob.clone(), PictureState::Decoded(None));
            return Some(None);
        }
        self.pictures
            .insert(image.blob.clone(), PictureState::Decoding);
        None
    }

    /// Starts the thread that decodes pictures with `job` as its first.
    fn start_decoder(&mut self, job: PictureJob) -> std::io::Result<()> {
        let (decoder, jobs) = mpsc::channel::<PictureJob>();
        let (sender, decoded) = mpsc::channel();
        let cancelled = Arc::clone(&self.cancelled);
        std::thread::Builder::new()
            .name("ketchup-pictures".into())
            .spawn(move || decode_pictures(&jobs, &sender, &cancelled, RenderPicture::decode))?;
        // Cannot fail: the thread holds the receiver until this sender is dropped.
        dropped(decoder.send(job), Reason::NobodyWaiting);
        self.decoder = Some(decoder);
        self.decoded = Some(decoded);
        Ok(())
    }

    fn geometry(&mut self, source: GeometrySource, blobs: &Blobs) -> Arc<RenderGeometry> {
        if let Some(geometry) = self.geometries.get(&source.fingerprint) {
            self.hits += 1;
            return Arc::clone(geometry);
        }
        let mut fingerprint = source.fingerprint;
        let mut vertices = source.vertices;
        let mut awaiting = false;
        let picture = source.picture.and_then(|image| {
            let picture = self.picture(&image, blobs).unwrap_or_else(|| {
                awaiting = true;
                None
            });
            if picture.is_none() {
                // Shown in the face's own colour instead, for now or for good.
                for vertex in &mut vertices {
                    vertex.picture[2] = 0.0;
                }
            }
            picture
        });
        self.misses += 1;
        if awaiting {
            // Built again with the picture once it is decoded.
            fingerprint.push_str(AWAITING_PICTURE);
        }
        let geometry = Arc::new(RenderGeometry {
            fingerprint: fingerprint.clone(),
            vertices,
            indices: source.indices,
            edge_vertices: source.edge_vertices,
            picture,
        });
        if !awaiting {
            self.geometries.insert(fingerprint, Arc::clone(&geometry));
        }
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
    /// Linear RGBA painted over the part's own color; alpha 0 paints none.
    color: [f32; 4],
    /// The point of the geometry's picture here (glTF `u`, `v`), 1 when the triangle
    /// shows it (else 0), and 1 on the turned copy [`two_sided`] makes of a sheet's
    /// triangle (else 0).
    picture: [f32; 4],
}

/// A picture an imported mesh shows, decoded for the GPU: opaque sRGB RGBA rows from the
/// top, at most [`ketchup_manufacturing::presentation::MAX_TEXTURE_SIDE`] pixels a side.
#[derive(Debug)]
pub struct RenderPicture {
    blob: String,
    size: [u32; 2],
    rgba: Vec<u8>,
}

impl RenderPicture {
    /// The picture `image` names, of `bytes`; `None`, said in the log, when it cannot be read.
    fn decode(image: &MeshImage, bytes: Option<&Vec<u8>>) -> Option<Arc<Self>> {
        let decoded = bytes
            .ok_or(TextureError::Missing)
            .and_then(|bytes| PresentationTexture::decode(bytes, &image.mime_type));
        let picture = match decoded {
            Ok(picture) => picture,
            Err(error) => {
                ketchup_rejection::log!(
                    "the picture {} is shown as a plain face: {error}",
                    image.blob
                );
                return None;
            }
        };
        // Both sides are at most `MAX_TEXTURE_SIDE`.
        let [Ok(width), Ok(height)] = picture.size().map(u32::try_from) else {
            ketchup_rejection::log!("the picture {} is too large to show", image.blob);
            return None;
        };
        Some(Arc::new(Self {
            blob: image.blob.clone(),
            size: [width, height],
            rgba: picture.rgba(),
        }))
    }
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
    /// The picture the triangles with a picture weight show.
    picture: Option<Arc<RenderPicture>>,
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

    /// The blob of the picture its textured triangles show.
    #[must_use]
    pub fn picture_blob(&self) -> Option<&str> {
        self.picture.as_ref().map(|picture| picture.blob.as_str())
    }

    /// The color each triangle paints over its part's own, in linear RGBA.
    pub fn triangle_colors(&self) -> impl Iterator<Item = Option<[f32; 4]>> + '_ {
        self.vertices
            .iter()
            .step_by(3)
            .map(|vertex| (vertex.color[3] > 0.0).then_some(vertex.color))
    }

    /// Every line drawn with the faces: feature edges and decorations.
    pub fn edge_segments(&self) -> impl Iterator<Item = [[f32; 3]; 2]> + '_ {
        self.edge_vertices
            .iter()
            .step_by(6)
            .map(|vertex| [vertex.start, vertex.end])
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
    /// Whether the exact faces were painted by `FaceColors`.
    face_colored: bool,
    batches: Vec<RenderBatch>,
    /// The instance path of every instance, parallel to `batches`.
    instance_paths: Arc<Vec<Vec<InstancePath>>>,
}

fn next_plan_id() -> u64 {
    static NEXT_PLAN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT_PLAN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Lines a definition draws with its feature edges, in its own frame: the
/// cosmetic threads on its cylinders.
pub type DefinitionLines = BTreeMap<DefinitionId, Vec<[[f32; 3]; 2]>>;

/// The sRGB color of an exact package's faces by face ordinal, painted over
/// the part's own color; a face without one keeps it.
pub type FaceColors = fn(&ExactBodyPackage) -> BTreeMap<u32, [u8; 3]>;

impl InstancedRenderPlan {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        exact_results: &ExactResultRegistry,
        cache: &mut DerivedRenderCache,
    ) -> Self {
        Self::from_snapshot_with_lines(
            snapshot,
            exact_results,
            &DefinitionLines::new(),
            None,
            &Blobs::new(),
            cache,
        )
    }

    /// The plan of `snapshot` whose definitions also draw their `lines`, whose
    /// exact faces are painted by `face_colors` and whose imported meshes show
    /// their pictures, read from the document's `blobs`.
    pub fn from_snapshot_with_lines(
        snapshot: &Snapshot,
        exact_results: &ExactResultRegistry,
        lines: &DefinitionLines,
        face_colors: Option<FaceColors>,
        blobs: &Blobs,
        cache: &mut DerivedRenderCache,
    ) -> Self {
        cache.receive_pictures();
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
                    lines.get(&definition_id).map_or(&[][..], Vec::as_slice),
                    face_colors,
                );
                if sources.is_empty() {
                    continue;
                }
                entry.insert(
                    sources
                        .into_iter()
                        .map(|source| {
                            let fingerprint = source.fingerprint.clone();
                            let geometry = cache.geometry(source, blobs);
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
        let shown = cache
            .geometries
            .values()
            .filter_map(|geometry| {
                geometry
                    .picture
                    .as_ref()
                    .map(|picture| picture.blob.clone())
            })
            .collect::<BTreeSet<_>>();
        // One being decoded is kept until it is: a plan drawn without it waits for it.
        cache
            .pictures
            .retain(|blob, state| matches!(state, PictureState::Decoding) || shown.contains(blob));
        Self {
            id: next_plan_id(),
            document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            exact_contents_stamp: exact_results.contents_stamp(),
            face_colored: face_colors.is_some(),
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

    /// Whether this plan painted its exact faces by face colors.
    #[must_use]
    pub const fn face_colored(&self) -> bool {
        self.face_colored
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
    /// The picture its triangles show, decoded only when the geometry is not cached.
    picture: Option<MeshImage>,
}

pub(crate) type PlanarProfileMesh = (Vec<[f64; 3]>, Vec<[u32; 3]>);

/// The geometries of `definition_id`; the first also draws `lines`.
fn geometry_sources(
    snapshot: &Snapshot,
    exact_packages: &[&Arc<ExactBodyPackage>],
    definition_id: DefinitionId,
    lines: &[[[f32; 3]; 2]],
    face_colors: Option<FaceColors>,
) -> Vec<GeometrySource> {
    let exact = exact_packages
        .iter()
        .enumerate()
        .map(|(index, package)| {
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
            let lines = if index == 0 { lines } else { &[] };
            let triangle_colors = face_colors.map_or_else(Vec::new, |face_colors| {
                let colors = face_colors(package);
                (0..triangles.len())
                    .map(|triangle| {
                        package
                            .triangle_face_ordinal(triangle)
                            .and_then(|ordinal| colors.get(&ordinal).copied())
                    })
                    .collect()
            });
            build_render_geometry(
                "exact",
                &positions,
                &triangles,
                &face_groups,
                &triangle_colors,
                lines,
            )
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
            let mut geometry = build_render_geometry(
                "canonical-mesh",
                &positions,
                &mesh.triangles,
                &vec![None::<u8>; mesh.triangles.len()],
                &[],
                lines,
            );
            if let Some(look) = &mesh.look {
                geometry = with_pictures(geometry, look);
            }
            let (geometry, kind) = if mesh.is_sheet() {
                (two_sided(geometry, "canonical-sheet"), "canonical-sheet")
            } else {
                (geometry, "canonical-mesh")
            };
            return match &mesh.look {
                Some(look) => split_by_picture(geometry, look, kind),
                None => vec![geometry],
            };
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
            &[],
            lines,
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
        &[],
        lines,
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
    triangle_colors: &[Option<[u8; 3]>],
    lines: &[[[f32; 3]; 2]],
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
    for (triangle_index, ((triangle, face_group), face_normal)) in triangles
        .iter()
        .zip(face_groups)
        .zip(&face_normals)
        .enumerate()
    {
        let color = triangle_colors
            .get(triangle_index)
            .copied()
            .flatten()
            .map_or([0.0; 4], linear_rgba);
        for index in triangle {
            let normal = face_group
                .and_then(|group| grouped_vertex_normals.get(&(*index, group)).copied())
                .map_or(*face_normal, normalize);
            vertices.push(RenderVertex {
                position: positions[*index as usize],
                normal,
                color,
                picture: [0.0; 4],
            });
        }
    }
    let indices = (0..vertices.len() as u32).collect::<Vec<_>>();
    let edge_vertices = edge_quads(positions, &feature_edges, lines);
    GeometrySource {
        fingerprint: geometry_fingerprint(kind, &vertices, &indices, &edge_vertices),
        vertices,
        indices,
        edge_vertices,
        picture: None,
    }
}

/// `geometry`, one triangle after another as [`build_render_geometry`] makes them, with
/// the picture `look` gives each: where its corners lie on it.
fn with_pictures(mut geometry: GeometrySource, look: &MeshLook) -> GeometrySource {
    for (triangle, textured) in geometry.vertices.chunks_exact_mut(3).zip(&look.triangles) {
        let Some(textured) = textured.filter(|textured| textured.image_index() < look.images.len())
        else {
            continue;
        };
        for (vertex, [u, v]) in triangle.iter_mut().zip(textured.uv) {
            vertex.picture = [u, v, 1.0, vertex.picture[3]];
        }
    }
    geometry
}

/// `geometry` split by the picture each triangle shows (`images`, one per front triangle;
/// a sheet's back triangles follow its front ones and show the same): first the
/// triangles without one, with every edge, then one geometry per picture. The picture of
/// each is named in its fingerprint.
fn split_by_picture(geometry: GeometrySource, look: &MeshLook, kind: &str) -> Vec<GeometrySource> {
    let front = look.triangles.len().max(1);
    let mut groups = BTreeMap::<Option<usize>, Vec<RenderVertex>>::new();
    for (index, triangle) in geometry.vertices.chunks_exact(3).enumerate() {
        let image = look
            .triangles
            .get(index % front)
            .copied()
            .flatten()
            .map(|textured| textured.image_index())
            .filter(|image| *image < look.images.len() && triangle[0].picture[2] > 0.0);
        groups.entry(image).or_default().extend_from_slice(triangle);
    }
    let mut edge_vertices = geometry.edge_vertices;
    groups
        .into_iter()
        .map(|(image, vertices)| {
            let picture = image.and_then(|image| look.images.get(image)).cloned();
            let kind = picture.as_ref().map_or_else(
                || kind.to_owned(),
                |picture| format!("{kind}:{}", picture.blob),
            );
            let indices = (0..).take(vertices.len()).collect::<Vec<_>>();
            let edge_vertices = std::mem::take(&mut edge_vertices);
            GeometrySource {
                fingerprint: geometry_fingerprint(&kind, &vertices, &indices, &edge_vertices),
                vertices,
                indices,
                edge_vertices,
                picture,
            }
        })
        .collect()
}

/// A sheet seen from behind too: each triangle again, turned the other way, so
/// that leaving out back faces, the inside of a solid, still shows it. Its edges
/// stay those of the sheet. A turned copy gives way to any surface that faces the
/// eye where it lies: a SketchUp face's back side comes as a sheet of its own, and
/// from in front the face's own side shows, as in SketchUp.
fn two_sided(mut geometry: GeometrySource, kind: &str) -> GeometrySource {
    let back = geometry
        .vertices
        .chunks_exact(3)
        .flat_map(|triangle| {
            [triangle[0], triangle[2], triangle[1]].map(|vertex| {
                let [u, v, shown, _] = vertex.picture;
                RenderVertex {
                    normal: vertex.normal.map(|value| -value),
                    picture: [u, v, shown, 1.0],
                    ..vertex
                }
            })
        })
        .collect::<Vec<_>>();
    geometry.vertices.extend(back);
    geometry.indices = (0..).take(geometry.vertices.len()).collect();
    geometry.fingerprint = geometry_fingerprint(
        kind,
        &geometry.vertices,
        &geometry.indices,
        &geometry.edge_vertices,
    );
    geometry
}

/// Each feature edge as its own screen-space quad (two triangles). Edges are drawn
/// as lines rather than inside the face triangles: a face triangulation is a fan of
/// thin slivers along notched outlines, and a line confined to the triangle owning
/// the edge came out dotted wherever the sliver narrowed below a pixel. Faces mesh
/// separately, so the edge two faces share appears twice and is drawn once.
/// `lines` are drawn the same way.
fn edge_quads(
    positions: &[[f32; 3]],
    feature_edges: &BTreeSet<[u32; 2]>,
    lines: &[[[f32; 3]; 2]],
) -> Vec<EdgeVertex> {
    let mut segments = BTreeSet::new();
    let edge_ends = feature_edges
        .iter()
        .map(|[first, second]| [positions[*first as usize], positions[*second as usize]]);
    for ends in edge_ends.chain(lines.iter().copied()) {
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
fn unit(vector: [f64; 3]) -> [f32; 3] {
    single_precision(linalg::normalize(vector).unwrap_or(vector))
}

/// `point` in GPU single precision, rounded to the nearest `f32`.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn single_precision(point: [f64; 3]) -> [f32; 3] {
    point.map(|value| value as f32)
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
            write!(fingerprint, ":{:08x}", value.to_bits()).expect("a String takes every write");
        }
    }
    for index in indices {
        write!(fingerprint, ":{index:08x}").expect("a String takes every write");
    }
    if vertices.iter().any(|vertex| vertex.picture[2] > 0.0) {
        fingerprint.push_str(":pictures");
        for vertex in vertices {
            for value in vertex.picture {
                fingerprint.push_str(&format!(":{:08x}", value.to_bits()));
            }
        }
    }
    if vertices.iter().any(|vertex| vertex.color[3] > 0.0) {
        fingerprint.push_str(":colors");
        for vertex in vertices.iter().step_by(3) {
            for value in vertex.color {
                write!(fingerprint, ":{:08x}", value.to_bits())
                    .expect("a String takes every write");
            }
        }
    }
    fingerprint.push_str(":edges");
    for edge in edge_vertices.iter().step_by(6) {
        for value in edge.start.into_iter().chain(edge.end) {
            write!(fingerprint, ":{:08x}", value.to_bits()).expect("a String takes every write");
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
    /// The blob of the picture it shows, a key of `GpuInstancedRenderer::pictures`.
    picture: Option<String>,
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
    picture_bind_group_layout: wgpu::BindGroupLayout,
    picture_sampler: wgpu::Sampler,
    /// What a geometry without a picture binds: one white pixel it never shows.
    no_picture: wgpu::BindGroup,
    /// The pictures of the prepared plan by blob.
    pictures: BTreeMap<String, wgpu::BindGroup>,
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

/// The bind group of a picture of `size` pixels, opaque sRGB RGBA rows from the top. A
/// picture comes averaged down to at most `MAX_TEXTURE_SIDE`, so one level is drawn.
fn create_picture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    size: [u32; 2],
    rgba: &[u8],
) -> wgpu::BindGroup {
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Ketchup mesh picture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * 4),
            rows_per_image: Some(size[1]),
        },
        extent,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Ketchup mesh picture bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
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
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
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
        let picture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Ketchup mesh picture bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        // A picture repeats beyond one tile, as glTF and SketchUp lay it.
        let picture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Ketchup mesh picture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let no_picture = create_picture_bind_group(
            device,
            queue,
            &picture_bind_group_layout,
            &picture_sampler,
            [1, 1],
            &[u8::MAX; 4],
        );
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Ketchup instanced scene pipeline layout"),
            bind_group_layouts: &[&scene_bind_group_layout, &picture_bind_group_layout],
            push_constant_ranges: &[],
        });
        let vertex_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4];
        let edge_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
        let instance_attributes = wgpu::vertex_attr_array![4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4];
        let surface_layout = wgpu::VertexBufferLayout {
            array_stride: 56,
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
            picture_bind_group_layout,
            picture_sampler,
            no_picture,
            pictures: BTreeMap::new(),
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
            self.upload_plan(device, queue, plan);
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
    fn upload_plan(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        plan: &InstancedRenderPlan,
    ) {
        self.prepared.clear();
        let mut instances = Vec::new();
        for batch in plan.batches() {
            if let Some(picture) = &batch.geometry.picture
                && !self.pictures.contains_key(&picture.blob)
            {
                let bind_group = create_picture_bind_group(
                    device,
                    queue,
                    &self.picture_bind_group_layout,
                    &self.picture_sampler,
                    picture.size,
                    &picture.rgba,
                );
                self.pictures.insert(picture.blob.clone(), bind_group);
            }
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
                        picture: batch
                            .geometry
                            .picture
                            .as_ref()
                            .map(|picture| picture.blob.clone()),
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
        let shown = self
            .geometries
            .values()
            .filter_map(|geometry| geometry.picture.as_deref())
            .collect::<std::collections::BTreeSet<_>>();
        self.pictures
            .retain(|blob, _| shown.contains(blob.as_str()));
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
            let picture = geometry
                .picture
                .as_ref()
                .and_then(|blob| self.pictures.get(blob))
                .unwrap_or(&self.no_picture);
            pass.set_bind_group(1, picture, &[]);
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
        .flat_map(|vertex| {
            vertex
                .position
                .into_iter()
                .chain(vertex.normal)
                .chain(vertex.color)
                .chain(vertex.picture)
        })
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
            let color = instance.color.map_or([0.0; 4], linear_rgba);
            row_major_to_columns(instance.transform)
                .into_iter()
                .chain(color)
        })
        .flat_map(f32::to_ne_bytes)
        .collect()
}

/// Opaque sRGB bytes as the linear RGBA the scene shader blends.
#[must_use]
pub fn linear_rgba(rgb: [u8; 3]) -> [f32; 4] {
    let [red, green, blue] = rgb.map(|byte| {
        let value = f32::from(byte) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    [red, green, blue, 1.0]
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

// The picture an imported mesh shows; one white pixel for a geometry without one.
@group(1) @binding(0)
var picture: texture_2d<f32>;
@group(1) @binding(1)
var picture_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // A face color over the part's own (draft analysis); alpha 0 is none.
    @location(2) face_color: vec4<f32>,
    // The point of the picture (u, v), 1 where the triangle shows it, and 1 on the
    // turned copy of a sheet's triangle.
    @location(3) picture: vec4<f32>,
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
    @location(4) picture: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    let world_position = model * vec4<f32>(input.position, 1.0);
    var output: VertexOutput;
    output.clip_position = camera.world_to_clip * world_position;
    output.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    output.color = select(input.color, input.face_color, input.face_color.a > 0.0);
    output.view_depth = dot(camera.view_depth, world_position);
    output.world_position = world_position.xyz;
    output.picture = input.picture;
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

// The depth a surface takes part with: a sheet's turned copy counts as just behind
// where it lies, so a surface that faces the eye there shows instead.
fn surface_depth(input: VertexOutput) -> f32 {
    // not a tolerance: f32 depth rounding margin between coincident surfaces.
    let margin = max(abs(input.view_depth) * 4e-6, 1e-3);
    return input.view_depth + input.picture.w * margin;
}

@fragment
fn fs_depth(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    if hidden(input, front_facing) {
        discard;
    }
    let index = pixel_depth_index(input.clip_position);
    let view_depth = surface_depth(input);
    atomicMax(&pixel_depths[index], depth_priority(view_depth));
    // An edge line is up to 1.5 px from the edge it draws, where an adjacent face
    // already lies deeper by its depth slope; two pixels of slope plus a rounding
    // margin keep the edge in front of its own faces but behind any other surface.
    // not a tolerance: f32 depth rounding margin of the edge pass.
    let slack = 2.0 * fwidth(input.view_depth) + max(abs(view_depth) * 4e-6, 1e-3);
    atomicMax(&pixel_depths[index + 1u], depth_priority(view_depth + slack));
    return vec4<f32>(0.0);
}

@fragment
fn fs_main(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    if hidden(input, front_facing) {
        discard;
    }
    let depth = atomicLoad(&pixel_depths[pixel_depth_index(input.clip_position)]);
    if depth_priority(surface_depth(input)) != depth {
        discard;
    }
    if !front_facing {
        // The inside of a solid seen through the cut marks the section.
        return vec4<f32>(0.84, 0.29, 0.25, 1.0);
    }
    let light_direction = normalize(vec3<f32>(-0.35, -0.45, 0.82));
    let diffuse = 0.62 + 0.38 * max(dot(normalize(input.world_normal), light_direction), 0.0);
    let own_color = mix(vec3<f32>(0.36, 0.42, 0.50) * diffuse, input.color.rgb, input.color.a);
    let texel = textureSampleLevel(picture, picture_sampler, input.picture.xy, 0.0).rgb;
    let face_color = mix(own_color, texel * diffuse, input.picture.z);
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

    /// A sheet's back side is its triangles again, wound and facing the other way, with
    /// the edges of the sheet unchanged, so leaving out back faces still shows it.
    #[test]
    fn a_sheet_is_drawn_from_both_sides_with_its_own_edges() {
        let geometry = || {
            super::build_render_geometry(
                "canonical-mesh",
                &POSITIONS,
                &TRIANGLES,
                &[None::<u8>; 2],
                &[],
                &[],
            )
        };
        let front = geometry();
        let sheet = super::two_sided(geometry(), "canonical-sheet");
        assert_eq!(sheet.vertices.len(), 2 * front.vertices.len());
        assert_eq!(sheet.indices, (0..12).collect::<Vec<u32>>());
        assert_eq!(sheet.edge_vertices, front.edge_vertices);
        assert_ne!(sheet.fingerprint, front.fingerprint);
        for (face, back) in front
            .vertices
            .chunks_exact(3)
            .zip(sheet.vertices[6..].chunks_exact(3))
        {
            assert_eq!(
                [back[0].position, back[1].position, back[2].position],
                [face[0].position, face[2].position, face[1].position]
            );
            for (vertex, turned) in face.iter().zip(back) {
                assert_eq!(turned.normal, vertex.normal.map(|value| -value));
            }
        }
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
