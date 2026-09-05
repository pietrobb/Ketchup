use ketchup_app::renderer::{
    DerivedRenderCache, GpuFrameDescriptor, GpuInstancedRenderer, InstancedRenderPlan,
    RENDER_BACKEND_WGPU_V1, RENDER_EVALUATOR_V1, RENDER_PLAN_SCHEMA_V1,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DerivedIdentity, Dimension, DocumentStore,
    FeatureId, FeatureKind, InstancePath, NodeId, OccurrenceId, SlotPath, SlotSegment, Transform,
};
use ketchup_core::exact_product::ExactResultRegistry;
use ketchup_interaction::projection::CanonicalInteractionProjection;
use ketchup_interaction::{Ray, Vec3};
use ketchup_scheduler::AcceptanceIdentity;
use ketchup_scheduler::general::{
    CompletionOutcome, GeneralJobScheduler, JobKind, JobPolicy, JobRequest, ScheduleOutcome,
};
use std::time::Instant;

const DEFINITION: DefinitionId = DefinitionId(1);
const PROFILE: FeatureId = FeatureId(1);
const BODY: FeatureId = FeatureId(2);
const OCCURRENCES: usize = 10_000;

fn grid_transform(index: usize) -> Transform {
    Transform::from_translation(
        (index % 100) as f64 * 20.0,
        (index / 100) as f64 * 20.0,
        0.0,
    )
    .unwrap()
}

fn product_document() -> DocumentStore {
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: DEFINITION,
            name: "M16 product definition".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: PROFILE,
            definition_id: DEFINITION,
            name: "Profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: BODY,
            definition_id: DEFINITION,
            name: "Extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: PROFILE,
                height: Dimension::from_decimal("10").unwrap(),
            },
        },
    ];
    commands.extend(
        (0..OCCURRENCES).map(|index| CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(index as u64 + 1),
            definition_id: DEFINITION,
            name: format!("Occurrence {}", index + 1),
            transform: grid_transform(index),
            parent: None,
            tag: None,
            visible: true,
        }),
    );
    let mut store = DocumentStore::new();
    store.apply_batch(&CommandBatch::new(commands)).unwrap();
    store
}

#[test]
fn component_render_instances_preserve_child_colors_and_reset_root_override() {
    use ketchup_core::document::GroupId;
    let mut store = product_document();
    let group = GroupId(1);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateGroup {
                id: group,
                name: "Colored pair".to_owned(),
                transform: Transform::identity(),
                parent: None,
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(1),
                parent: Some(group),
            },
            CanonicalCommand::SetOccurrenceParent {
                id: OccurrenceId(2),
                parent: Some(group),
            },
            CanonicalCommand::SetOccurrenceColor {
                id: OccurrenceId(1),
                color: Some([210, 30, 20]),
            },
            CanonicalCommand::SetOccurrenceColor {
                id: OccurrenceId(2),
                color: Some([20, 40, 210]),
            },
        ]))
        .unwrap();
    let mut cache = DerivedRenderCache::default();
    let colors = |plan: &InstancedRenderPlan| {
        let mut result = plan
            .batches()
            .iter()
            .flat_map(|batch| batch.instances.iter())
            .filter_map(|instance| instance.color)
            .collect::<Vec<_>>();
        result.sort();
        result
    };
    let before = InstancedRenderPlan::from_snapshot(
        &store.current(),
        &ExactResultRegistry::default(),
        &mut cache,
    );
    let expected = vec![[20, 40, 210], [210, 30, 20]];
    assert_eq!(colors(&before), expected);
    let root = store
        .convert_group_to_component(group, "Colored component")
        .unwrap()
        .component_occurrence_id;
    let converted = InstancedRenderPlan::from_snapshot(
        &store.current(),
        &ExactResultRegistry::default(),
        &mut cache,
    );
    assert_eq!(converted.instance_count(), OCCURRENCES);
    assert_eq!(colors(&converted), expected);
    assert!(std::sync::Arc::ptr_eq(
        &before.batches()[0].geometry,
        &converted.batches()[0].geometry
    ));
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceColor {
                id: root,
                color: Some([40, 200, 60]),
            },
        ]))
        .unwrap();
    let overridden = InstancedRenderPlan::from_snapshot(
        &store.current(),
        &ExactResultRegistry::default(),
        &mut cache,
    );
    assert_eq!(colors(&overridden), vec![[40, 200, 60]; 2]);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceColor {
                id: root,
                color: None,
            },
        ]))
        .unwrap();
    let reset = InstancedRenderPlan::from_snapshot(
        &store.current(),
        &ExactResultRegistry::default(),
        &mut cache,
    );
    assert_eq!(colors(&reset), expected);
    assert!(std::sync::Arc::ptr_eq(
        &before.batches()[0].geometry,
        &reset.batches()[0].geometry
    ));
}

fn mesh_acceptance(snapshot: &ketchup_core::document::Snapshot) -> AcceptanceIdentity {
    let node_id = NodeId(BODY.0);
    let slot_path = SlotPath::new(vec![
        SlotSegment::new(node_id, "mesh", "instanced-render").unwrap(),
    ])
    .unwrap();
    AcceptanceIdentity {
        document_scope: snapshot.document_id().0,
        derived_identity: DerivedIdentity::new(node_id, slot_path).unwrap(),
        input_digest: snapshot.canonical_digest(),
        evaluator: RENDER_EVALUATOR_V1.to_owned(),
        backend: Some(RENDER_BACKEND_WGPU_V1.to_owned()),
        schema: RENDER_PLAN_SCHEMA_V1.to_owned(),
        tolerance: ketchup_core::document::TOLERANCE_PROFILE_V1.to_owned(),
    }
}

#[test]
fn real_ten_thousand_occurrence_product_uses_one_scheduled_mesh_one_bvh_and_one_wgpu_draw() {
    let total_started = Instant::now();
    let mut store = product_document();
    let snapshot = store.current();

    let acceptance = mesh_acceptance(&snapshot);
    let mut scheduler = GeneralJobScheduler::new(1_048_576);
    scheduler
        .advance_revision(snapshot.revision_id(), [])
        .unwrap();
    let handle = match scheduler
        .schedule(JobRequest {
            node_id: NodeId(BODY.0),
            acceptance: acceptance.clone(),
            kind: JobKind::Mesh,
            policy: JobPolicy::NO_RESTART,
        })
        .unwrap()
    {
        ScheduleOutcome::Queued(handle) => handle,
        ScheduleOutcome::CacheHit(_) => panic!("first mesh request must execute"),
    };
    scheduler.start(handle.id).unwrap();
    assert_eq!(
        scheduler.complete(handle.id, "m16-one-shared-box-mesh", 240),
        Ok(CompletionOutcome::Current)
    );
    assert!(matches!(
        scheduler
            .schedule(JobRequest {
                node_id: NodeId(BODY.0),
                acceptance,
                kind: JobKind::Mesh,
                policy: JobPolicy::NO_RESTART,
            })
            .unwrap(),
        ScheduleOutcome::CacheHit(_)
    ));

    let plan_started = Instant::now();
    let mut render_cache = DerivedRenderCache::default();
    let plan = InstancedRenderPlan::from_snapshot(
        &snapshot,
        &ExactResultRegistry::default(),
        &mut render_cache,
    );
    let plan_elapsed = plan_started.elapsed();
    assert!(plan.is_current(&snapshot));
    assert_eq!(plan.geometry_count(), 1);
    assert_eq!(plan.instance_count(), OCCURRENCES);
    assert_eq!(render_cache.stats().geometry_entries, 1);
    let rebuilt = InstancedRenderPlan::from_snapshot(
        &snapshot,
        &ExactResultRegistry::default(),
        &mut render_cache,
    );
    assert_eq!(rebuilt.instance_count(), OCCURRENCES);
    assert_eq!(render_cache.stats().geometry_hits, 1);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceColor {
                id: OccurrenceId(1),
                color: Some([200, 50, 20]),
            },
            CanonicalCommand::SetOccurrenceColor {
                id: OccurrenceId(2),
                color: Some([20, 80, 210]),
            },
        ]))
        .unwrap();
    let colored = InstancedRenderPlan::from_snapshot(
        &store.current(),
        &ExactResultRegistry::default(),
        &mut render_cache,
    );
    assert_eq!(colored.geometry_count(), 1);
    assert_eq!(colored.batches()[0].instances[0].color, Some([200, 50, 20]));
    assert_eq!(colored.batches()[0].instances[1].color, Some([20, 80, 210]));
    assert!(
        colored.batches()[0].instances[2..]
            .iter()
            .all(|instance| instance.color.is_none())
    );
    assert_eq!(render_cache.stats().geometry_misses, 1);
    assert!(std::sync::Arc::ptr_eq(
        &plan.batches()[0].geometry,
        &colored.batches()[0].geometry
    ));
    assert_eq!(
        plan.batches()[0].instances[0].transform,
        colored.batches()[0].instances[0].transform
    );
    assert!(store.undo().is_some());
    let reset = InstancedRenderPlan::from_snapshot(
        &store.current(),
        &ExactResultRegistry::default(),
        &mut render_cache,
    );
    assert!(
        reset.batches()[0]
            .instances
            .iter()
            .all(|instance| instance.color.is_none())
    );
    assert!(std::sync::Arc::ptr_eq(
        &colored.batches()[0].geometry,
        &reset.batches()[0].geometry
    ));
    let cached_plan = std::sync::Arc::new(rebuilt.clone());
    let orbit_started = Instant::now();
    for _ in 0..10_000 {
        assert!(cached_plan.is_same_revision(&snapshot));
        assert_eq!(cached_plan.instance_count(), OCCURRENCES);
    }
    let cached_orbit_elapsed = orbit_started.elapsed();
    assert!(
        cached_orbit_elapsed.as_millis() < 50,
        "{cached_orbit_elapsed:?}"
    );

    let target = OCCURRENCES - 1;
    let target_placement = grid_transform(target);
    let target_transform = target_placement.matrix();
    let ray = Ray::new(
        Vec3::new(target_transform[3] + 2.0, target_transform[7] + 2.0, 30.0),
        Vec3::new(0.0, 0.0, -1.0),
    )
    .unwrap();
    let scene = CanonicalInteractionProjection::from_snapshot(&snapshot)
        .scene()
        .unwrap();
    let (hit, spatial_stats) = scene.exact_pick_with_stats(ray, 0.01);
    assert_eq!(
        hit.unwrap().primary.reference.instance_path,
        InstancePath::root(OccurrenceId(OCCURRENCES as u64))
    );
    assert_eq!(spatial_stats.indexed_items, OCCURRENCES);
    assert!(spatial_stats.candidate_count <= 4, "{spatial_stats:?}");
    assert!(
        spatial_stats.bounds_tested < OCCURRENCES / 8,
        "{spatial_stats:?}"
    );

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("M16 product proof requires a hardware DX12 adapter");
    let adapter_info = adapter.get_info();
    assert_eq!(adapter_info.backend, wgpu::Backend::Dx12);
    assert!(!matches!(
        adapter_info.device_type,
        wgpu::DeviceType::Cpu | wgpu::DeviceType::VirtualGpu
    ));
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("Ketchup M16 10k product proof"),
        ..Default::default()
    }))
    .unwrap();
    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Ketchup M16 offscreen target"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut renderer = GpuInstancedRenderer::new(&device, format);
    let gpu_started = Instant::now();
    let world_to_clip = [
        0.001, 0.0, 0.0, 0.0, 0.0, 0.001, 0.0, 0.0, 0.0, 0.0, 0.001, 0.0, -1.0, -1.0, 0.0, 1.0,
    ];
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Ketchup M16 instanced render encoder"),
    });
    let frame = GpuFrameDescriptor {
        world_to_clip,
        view_depth: [0.0, 0.0, 1.0, 0.0],
        framebuffer_size: [64, 64],
        viewport: [0, 0, 64, 64],
    };
    renderer.prepare(&device, &queue, &mut encoder, &plan, frame);
    renderer.prepare(&device, &queue, &mut encoder, &colored, frame);
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Ketchup M16 instanced render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        renderer.paint(&mut pass);
    }
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::WaitForSubmissionIndex(submission))
        .unwrap();
    let gpu_elapsed = gpu_started.elapsed();
    assert!(
        pollster::block_on(device.pop_error_scope()).is_none(),
        "wgpu validation rejected the product draw"
    );
    let gpu_stats = renderer.stats();
    assert_eq!(gpu_stats.gpu_geometry_entries, 1);
    assert_eq!(gpu_stats.geometry_uploads, 1);
    assert_eq!(gpu_stats.geometry_cache_hits, 1);
    assert_eq!(gpu_stats.instance_uploads, 2);
    assert_eq!(gpu_stats.draw_calls, 1);
    assert_eq!(gpu_stats.instances_drawn, OCCURRENCES as u64);

    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceVisibility {
                id: OccurrenceId(1),
                visible: false,
            },
        ]))
        .unwrap();
    let changed = store.current();
    assert!(!plan.is_current(&changed));
    let changed_plan = InstancedRenderPlan::from_snapshot(
        &changed,
        &ExactResultRegistry::default(),
        &mut render_cache,
    );
    assert!(changed_plan.is_current(&changed));
    assert_eq!(changed_plan.instance_count(), OCCURRENCES - 1);
    assert_eq!(render_cache.stats().geometry_entries, 1);

    eprintln!(
        "M16_10K adapter={:?} plan_ms={:.3} cached_orbit_10k_checks_ms={:.3} gpu_submit_wait_ms={:.3} total_ms={:.3} candidates={} bounds_tested={} draw_calls={} geometry_uploads={} instances={}",
        adapter_info,
        plan_elapsed.as_secs_f64() * 1_000.0,
        cached_orbit_elapsed.as_secs_f64() * 1_000.0,
        gpu_elapsed.as_secs_f64() * 1_000.0,
        total_started.elapsed().as_secs_f64() * 1_000.0,
        spatial_stats.candidate_count,
        spatial_stats.bounds_tested,
        gpu_stats.draw_calls,
        gpu_stats.geometry_uploads,
        gpu_stats.instances_drawn,
    );
}

/// Real offscreen DX12 work, not egui_kittest or a GPU timestamp benchmark.
/// Timings include camera/prepare/encode, queue submission and GPU completion;
/// document evaluation, pipeline warmup and readback are deliberately separate.
#[test]
fn garden_studio_hardware_gpu_camera_frames() {
    use ketchup_application::{DocumentSession, SessionSettings};
    use std::time::Duration;

    const BODIES: usize = 93;
    const FRAMES: usize = 20;
    const WARMUP: usize = 3;
    const WIDTH: u32 = 1600;
    const HEIGHT: u32 = 1000;
    let setup_started = Instant::now();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/garden-studio.ketchup");
    let mut session = DocumentSession::open(&fixture, SessionSettings::default()).unwrap();
    session
        .evaluate()
        .expect("garden studio needs real exact evaluation");
    let snapshot = session.snapshot();
    assert_eq!(snapshot.occurrences().count(), BODIES);
    assert_eq!(
        session.exact_results().render_values(&snapshot).count(),
        BODIES
    );
    let mut cache = DerivedRenderCache::default();
    let plan = InstancedRenderPlan::from_snapshot(&snapshot, session.exact_results(), &mut cache);
    assert!(plan.matches_exact_results(&snapshot, session.exact_results()));
    assert_eq!(plan.instance_count(), BODIES);
    let triangles: usize = plan
        .batches()
        .iter()
        .map(|batch| {
            assert!(batch.geometry.vertex_count() > 0);
            assert!(batch.geometry.index_count() > 0);
            assert_eq!(batch.geometry.index_count() % 3, 0);
            batch.geometry.index_count() / 3 * batch.instances.len()
        })
        .sum();
    assert_eq!(
        triangles, 1100,
        "the actual exact house must reach the GPU plan"
    );

    // Fit the evaluated vertices after each plan instance's row-major transform.
    // A bounding sphere keeps the entire ~8 m house (in mm) inside every orbit,
    // with correct 1600:1000 aspect and wgpu's [0, 1] clip-depth range.
    let exact = session.exact_results().render_by_definition(&snapshot);
    let mut points = Vec::<[f32; 3]>::new();
    for batch in plan.batches() {
        let package = exact
            .get(&batch.definition_id)
            .expect("no box/mesh fallback");
        assert_eq!(batch.geometry.index_count(), package.triangles().len() * 3);
        for instance in &batch.instances {
            let m = instance.transform;
            for vertex in package.vertices() {
                let p = vertex.position_mm.map(|v| v as f32);
                points.push(std::array::from_fn(|row| {
                    m[row * 4] * p[0]
                        + m[row * 4 + 1] * p[1]
                        + m[row * 4 + 2] * p[2]
                        + m[row * 4 + 3]
                }));
            }
        }
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in &points {
        for axis in 0..3 {
            assert!(p[axis].is_finite());
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    let center: [f32; 3] = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
    let radius = (0..3)
        .map(|i| ((max[i] - min[i]) * 0.5).powi(2))
        .sum::<f32>()
        .sqrt();
    assert!(
        radius > 1000.0 && radius < 20_000.0,
        "unexpected mm bounds: {min:?}..{max:?}"
    );
    let dot = |a: [f32; 3], b: [f32; 3]| a.into_iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let camera = |frame: usize| {
        let angle = 0.6 + frame as f32 * 0.025;
        let (sin, cos) = angle.sin_cos();
        let (se, ce) = 0.55_f32.sin_cos();
        let right = [-sin, cos, 0.0];
        let up = [-se * cos, -se * sin, ce];
        let forward = [-ce * cos, -ce * sin, -se];
        let x = right.map(|v| v / (radius * 1.15 * WIDTH as f32 / HEIGHT as f32));
        let y = up.map(|v| v / (radius * 1.15));
        let z = forward.map(|v| v / (radius * 4.0));
        GpuFrameDescriptor {
            // Column-major for WGSL, unlike the row-major instance transforms.
            world_to_clip: [
                x[0],
                y[0],
                z[0],
                0.0,
                x[1],
                y[1],
                z[1],
                0.0,
                x[2],
                y[2],
                z[2],
                0.0,
                -dot(x, center),
                -dot(y, center),
                0.5 - dot(z, center),
                1.0,
            ],
            view_depth: [
                forward[0],
                forward[1],
                forward[2],
                radius * 2.0 - dot(forward, center),
            ],
            framebuffer_size: [WIDTH, HEIGHT],
            viewport: [0, 0, WIDTH, HEIGHT],
        }
    };
    for frame in 0..WARMUP + FRAMES {
        let m = camera(frame).world_to_clip;
        if frame > 0 {
            assert_ne!(m, camera(frame - 1).world_to_clip);
        }
        for p in &points {
            let clip: [f32; 3] = std::array::from_fn(|row| {
                m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row]
            });
            assert!(
                clip[0].abs() < 1.0 && clip[1].abs() < 1.0 && clip[2] > 0.0 && clip[2] < 1.0,
                "house clipped: {clip:?}"
            );
        }
    }

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("garden regression requires a physical DX12 GPU, never software fallback");
    let adapter_info = adapter.get_info();
    assert_eq!(adapter_info.backend, wgpu::Backend::Dx12);
    assert!(matches!(
        adapter_info.device_type,
        wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu
    ));
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("Garden studio hardware regression"),
        ..Default::default()
    }))
    .unwrap();
    device.on_uncaptured_error(Box::new(|error| panic!("uncaptured wgpu error: {error}")));
    for filter in [
        wgpu::ErrorFilter::OutOfMemory,
        wgpu::ErrorFilter::Internal,
        wgpu::ErrorFilter::Validation,
    ] {
        device.push_error_scope(filter);
    }
    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Garden studio 1600x1000 offscreen target"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut renderer = GpuInstancedRenderer::new(&device, format);
    // One synchronous frame at a time: no enqueue-only or CPU-only substitute.
    let render_frame = |renderer: &mut GpuInstancedRenderer, frame: usize| {
        let started = Instant::now();
        assert!(plan.is_same_revision(&snapshot));
        let descriptor = camera(frame);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Garden studio camera frame"),
        });
        renderer.prepare(&device, &queue, &mut encoder, &plan, descriptor);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Garden studio color pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            renderer.paint(&mut pass);
        }
        let command = encoder.finish();
        let cpu = started.elapsed();
        let submit_started = Instant::now();
        let submission = queue.submit([command]);
        let submit = submit_started.elapsed();
        let wait_started = Instant::now();
        device
            .poll(wgpu::PollType::WaitForSubmissionIndex(submission))
            .unwrap();
        let wait = wait_started.elapsed();
        [cpu, submit, wait, started.elapsed()].map(|d| d.as_secs_f64() * 1000.0)
    };
    for frame in 0..WARMUP {
        render_frame(&mut renderer, frame);
    }
    let warm = renderer.stats();
    assert!(warm.geometry_uploads > 0);
    assert_eq!(warm.instances_drawn, (WARMUP * BODIES) as u64);
    let setup_ms = setup_started.elapsed().as_secs_f64() * 1000.0;
    let mut samples = Vec::with_capacity(FRAMES);
    let measured_started = Instant::now();
    for frame in 0..FRAMES {
        let before = renderer.stats();
        samples.push(render_frame(&mut renderer, WARMUP + frame));
        let after = renderer.stats();
        assert_eq!(
            after.geometry_uploads, warm.geometry_uploads,
            "camera-only geometry reupload"
        );
        assert_eq!(after.gpu_geometry_entries, warm.gpu_geometry_entries);
        assert_eq!(
            after.instances_drawn - before.instances_drawn,
            BODIES as u64
        );
        assert_eq!(
            after.draw_calls - before.draw_calls,
            plan.batches().len() as u64
        );
    }
    let measured = measured_started.elapsed();
    let stats = renderer.stats();
    assert_eq!(
        stats.instances_drawn - warm.instances_drawn,
        (FRAMES * BODIES) as u64
    );

    // Outside the timing: prove that submitted geometry actually shaded pixels.
    // 1600 * 4 = 6400 is already a multiple of COPY_BYTES_PER_ROW_ALIGNMENT.
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Garden studio pixel proof"),
        size: u64::from(WIDTH * HEIGHT * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
        },
        extent,
    );
    let submission = queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap()
        });
    device
        .poll(wgpu::PollType::WaitForSubmissionIndex(submission))
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    let pixels = readback.slice(..).get_mapped_range();
    let shaded_pixels = pixels
        .chunks_exact(4)
        .filter(|p| p[..3] != [0, 0, 0])
        .count();
    assert!(
        shaded_pixels > 10_000,
        "blank or implausibly tiny house: {shaded_pixels} pixels"
    );
    drop(pixels);
    readback.unmap();
    for _ in 0..3 {
        let error = pollster::block_on(device.pop_error_scope());
        assert!(
            error.is_none(),
            "wgpu rejected garden studio work: {error:?}"
        );
    }
    eprintln!(
        "GARDEN_GPU adapter={adapter_info:?} debug={} size={WIDTH}x{HEIGHT} bodies={BODIES} triangles={triangles} bounds_mm={min:?}..{max:?} setup_including_warmup_ms={setup_ms:.3}",
        cfg!(debug_assertions)
    );
    for (frame, sample) in samples.iter().enumerate() {
        eprintln!(
            "GARDEN_GPU frame={frame:02} cpu_ms={:.3} submit_ms={:.3} wait_ms={:.3} total_ms={:.3}",
            sample[0], sample[1], sample[2], sample[3]
        );
    }
    for (column, name) in ["cpu", "submit", "wait", "cpu_submit_wait"]
        .into_iter()
        .enumerate()
    {
        let mut sorted: Vec<f64> = samples.iter().map(|s| s[column]).collect();
        sorted.sort_by(f64::total_cmp);
        let total: f64 = sorted.iter().sum();
        // Nearest-rank p95: the 19th ordered sample out of 20.
        let p95 = sorted[(FRAMES * 95).div_ceil(100) - 1];
        eprintln!(
            "GARDEN_GPU {name} total_ms={total:.3} mean_ms={:.3} p95_ms={p95:.3}",
            total / FRAMES as f64
        );
    }
    eprintln!(
        "GARDEN_GPU warmed_frames={FRAMES} wall_ms={:.3} geometry_uploads_before={} after={} instances_drawn={} color_draw_calls={} shaded_pixels={shaded_pixels} wgpu_errors=0",
        measured.as_secs_f64() * 1000.0,
        warm.geometry_uploads,
        stats.geometry_uploads,
        stats.instances_drawn - warm.instances_drawn,
        stats.draw_calls - warm.draw_calls
    );
    assert!(
        measured < Duration::from_secs(2),
        "20 warmed hardware frames exceeded the generous 2s regression budget: {measured:?}"
    );
}
