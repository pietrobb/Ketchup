use crate::{
    camera::Camera,
    gpu::{MeshRenderer, RenderTarget, RenderView},
    model::Model,
};
use ketchup_view_format::{DisplayStyle, Section};

fn pixels(device: &wgpu::Device, queue: &wgpu::Queue, target: &RenderTarget) -> Vec<u8> {
    target.read_rgba(device, queue).expect("readback")
}

#[test]
fn package_instances_visibility_selection_edges_and_section_change_gpu_pixels() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU adapter required, never skip rendering proof");
    eprintln!("Viewer GPU probe: {:?}", adapter.get_info());
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("GPU device");
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let package = crate::app::tests::scene_package();
    let model = Model::from_package(package).expect("package");
    let renderer = MeshRenderer::new(&device, &model).expect("upload");
    let target = RenderTarget::new(&device, [64, 64]);
    let mut camera =
        Camera::from_scene(&model, &model.manifest().scenes[0].camera).expect("camera");
    let mut view = RenderView {
        selected: None,
        visible: &[true, false],
        style: &DisplayStyle::Shaded,
        section: None,
    };
    renderer.render(&device, &queue, &target, &camera, &view);
    let before = pixels(&device, &queue, &target);
    let background = &before[0..4];
    let painted = |p: &[u8]| p.chunks_exact(4).filter(|p| *p != background).count();
    assert!(
        painted(&before) > 100,
        "source-placed first triangle must be visible"
    );
    view.selected = Some(0);
    renderer.render(&device, &queue, &target, &camera, &view);
    let highlighted = pixels(&device, &queue, &target);
    assert_ne!(before, highlighted);
    assert!(
        highlighted.chunks_exact(4).any(|p| p[0] > p[2] + 50),
        "selection is orange"
    );
    view.selected = Some(1);
    renderer.render(&device, &queue, &target, &camera, &view);
    assert_eq!(
        before,
        pixels(&device, &queue, &target),
        "a hidden same-name instance must not highlight the other"
    );
    view.selected = None;
    view.visible = &[false, false];
    renderer.render(&device, &queue, &target, &camera, &view);
    assert_eq!(painted(&pixels(&device, &queue, &target)), 0);
    view.visible = &[true, false];
    view.style = &DisplayStyle::ShadedEdges;
    renderer.render(&device, &queue, &target, &camera, &view);
    assert_ne!(
        before,
        pixels(&device, &queue, &target),
        "display edges must affect pixels"
    );
    view.style = &DisplayStyle::Wireframe;
    renderer.render(&device, &queue, &target, &camera, &view);
    assert!(
        painted(&pixels(&device, &queue, &target)) < painted(&before),
        "wireframe has no filled interior"
    );
    view.style = &DisplayStyle::Shaded;
    let section = Section {
        normal: [1.0, 0.0, 0.0],
        offset_mm: 88.0,
    };
    view.section = Some(&section);
    renderer.render(&device, &queue, &target, &camera, &view);
    let cut = painted(&pixels(&device, &queue, &target));
    assert!(
        cut > 0 && cut < painted(&before),
        "world-space section removes only the positive side"
    );
    view.section = None;
    camera = Camera::from_scene(&model, &model.manifest().scenes[1].camera).expect("second camera");
    view.visible = &[false, true];
    renderer.render(&device, &queue, &target, &camera, &view);
    let second = pixels(&device, &queue, &target);
    assert!(
        painted(&second) > 100,
        "second instance shares upload but has its own placement"
    );
    let mut reversed = Model::from_package(crate::app::tests::scene_package()).expect("model");
    reversed.bodies.reverse();
    let reordered = MeshRenderer::new(&device, &reversed).expect("upload");
    reordered.render(&device, &queue, &target, &camera, &view);
    assert_eq!(
        second,
        pixels(&device, &queue, &target),
        "body traversal order must not change render"
    );
    camera.orbit([50.0, 30.0]);
    renderer.render(&device, &queue, &target, &camera, &view);
    assert_ne!(second, pixels(&device, &queue, &target));
    let mut oversized = Model::from_package(crate::app::tests::scene_package()).expect("model");
    oversized.bodies = vec![oversized.bodies[0].clone(); 50_001];
    assert!(
        MeshRenderer::new(&device, &oversized).is_err(),
        "instance primitive budget checked before GPU allocation"
    );
    assert!(pollster::block_on(device.pop_error_scope()).is_none());
}
