use ketchup_app::renderer::{
    DerivedRenderCache, GpuInstancedRenderer, InstancedRenderPlan, RENDER_BACKEND_WGPU_V1,
    RENDER_EVALUATOR_V1, RENDER_PLAN_SCHEMA_V1,
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
    renderer.prepare(&device, &queue, &plan, world_to_clip);
    renderer.prepare(&device, &queue, &rebuilt, world_to_clip);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Ketchup M16 instanced render encoder"),
    });
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
        "M16_10K adapter={:?} plan_ms={:.3} gpu_submit_wait_ms={:.3} total_ms={:.3} candidates={} bounds_tested={} draw_calls={} geometry_uploads={} instances={}",
        adapter_info,
        plan_elapsed.as_secs_f64() * 1_000.0,
        gpu_elapsed.as_secs_f64() * 1_000.0,
        total_started.elapsed().as_secs_f64() * 1_000.0,
        spatial_stats.candidate_count,
        spatial_stats.bounds_tested,
        gpu_stats.draw_calls,
        gpu_stats.geometry_uploads,
        gpu_stats.instances_drawn,
    );
}
