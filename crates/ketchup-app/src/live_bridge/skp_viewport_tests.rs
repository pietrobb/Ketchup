//! The 3D view shows the pictures of an imported SketchUp file on its faces, drawn by the
//! scene renderer on the GPU into an image of its own.

use super::*;
use crate::renderer::{GpuFrameDescriptor, GpuInstancedRenderer};
use eframe::egui::{Pos2, Rect, Vec2};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../ketchup-skp/fixtures")
            .join(name),
    )
    .unwrap()
}

/// The 3D view of `app`, `size` pixels framing everything shown, as RGBA rows from the top.
fn viewport_pixels(app: &mut KetchupApp, side: [u16; 2]) -> Vec<u8> {
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(side[0].into(), side[1].into()));
    let size = side.map(u32::from);
    app.camera.viewport_rect = Some(rect);
    app.zoom_fit();
    let snapshot = app.document_snapshot();
    let plan = app.rebuild_render_plan(&snapshot);
    let (_, _, forward) = app.camera_basis();
    let (device, queue) = ketchup_test_gpu::gpu_device();
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = GpuInstancedRenderer::new(&device, &queue, format);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport test target"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let stride = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("viewport test readback"),
        size: u64::from(stride) * u64::from(size[1]),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    renderer.prepare(
        &device,
        &queue,
        &mut encoder,
        &plan,
        GpuFrameDescriptor {
            world_to_clip: app.world_to_clip(rect),
            view_depth: [forward.x as f32, forward.y as f32, forward.z as f32, 0.0],
            section: [0.0; 4],
            framebuffer_size: size,
            viewport: [0, 0, size[0], size[1]],
        },
    );
    {
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewport test pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        renderer.paint(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |result| {
        result.expect("readback maps");
    });
    device.poll(wgpu::PollType::Wait).expect("GPU finishes");
    assert!(pollster::block_on(device.pop_error_scope()).is_none());
    let mapped = buffer.slice(..).get_mapped_range();
    mapped
        .chunks_exact(stride as usize)
        .flat_map(|row| row[..usize::from(side[0]) * 4].to_vec())
        .collect()
}

/// The textured box of a SketchUp file shows its picture's red and white squares in the 3D
/// view. Before, every face showed one flat colour there.
#[test]
fn a_texture_shows_in_the_3d_view() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("textured.skp");
    std::fs::write(&path, fixture("textured_box.skp")).unwrap();
    let (mut app, mut bridge) = setup();
    let hide = app
        .document_snapshot()
        .occurrences()
        .map(|occurrence| CanonicalCommand::SetOccurrenceVisibility {
            id: occurrence.id(),
            visible: false,
        })
        .collect();
    app.document.apply_batch(&CommandBatch::new(hide)).unwrap();
    bridge
        .execute(
            &mut app,
            Request::Import {
                expected: None,
                path: path.display().to_string(),
                unit: None,
                as_underlay: false,
                revision_of: None,
                code_page: None,
            },
            false,
        )
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));

    // The box also brings the back side of its faces, in SketchUp's pink (255, 155, 155),
    // as a sheet on the same faces: from outside the picture shows, as in SketchUp.
    let rgba = viewport_pixels(&mut app, [240, 160]);
    let plan = app.render.plan.clone().unwrap();
    assert!(
        plan.batches()
            .iter()
            .any(|batch| batch.geometry.picture_blob().is_some()),
        "a geometry with the picture"
    );
    // Shading scales a face's colour, so a flat colour keeps one green-to-red ratio; the
    // pink one is 0.61. The 2 × 2 picture blends its squares between pixel centres, so
    // only a part of each red square stays clearly red.
    let ratios = rgba
        .chunks(4)
        .filter(|pixel| pixel[3] == u8::MAX && pixel[0] > 60)
        .map(|pixel| f64::from(pixel[1]) / f64::from(pixel[0]))
        .collect::<Vec<_>>();
    let red = ratios.iter().filter(|ratio| **ratio < 0.6).count();
    let white = ratios.iter().filter(|ratio| **ratio > 0.8).count();
    assert!(
        ratios.len() > 2_000 && red * 40 > ratios.len() && white * 20 > ratios.len(),
        "{red} red and {white} white of {} model pixels",
        ratios.len()
    );
}
