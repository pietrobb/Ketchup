//! `ketchup-viewer --benchmark <file>`: how fast this machine's Viewer opens
//! a package, draws it while orbiting, picks a component and switches scenes,
//! measured without a window on the GPU the window would use (V9). Peak
//! memory is read from outside the process by
//! `scripts/windows/benchmark-viewer.ps1`.

use super::ViewerApp;
use crate::{
    camera::Camera,
    gpu::{MeshRenderer, RenderTarget, RenderView},
};
use ketchup_rejection::{Rejection, RejectionPhase};
use num_traits::ToPrimitive;
use std::{
    fmt::Write as _,
    path::Path,
    time::{Duration, Instant},
};

/// Pointer pixels the camera turns per frame, a steady drag.
const ORBIT_PX_PER_FRAME: f32 = 4.0;
/// Pick positions per axis over the middle of the view, in device coordinates.
const PICKS: [f32; 5] = [-0.8, -0.4, 0.0, 0.4, 0.8];

/// What one run measured; every duration waits until the GPU finished.
#[derive(Debug)]
pub struct Benchmark {
    pub package_bytes: u64,
    pub components: usize,
    pub scenes: usize,
    /// Reading and checking the file as the Open button does, without the GPU.
    pub open: Duration,
    pub upload: Duration,
    pub size: [u32; 2],
    pub frames: usize,
    pub frame_median: Duration,
    pub frame_worst: Duration,
    pub pick_median: Duration,
    /// The slowest switch to a saved scene and its first frame; `None`
    /// without scenes.
    pub scene_worst: Option<Duration>,
}

impl Benchmark {
    /// Orbit frames per second from the median frame.
    pub fn fps(&self) -> f64 {
        1.0 / self.frame_median.as_secs_f64().max(f64::MIN_POSITIVE)
    }

    /// One measurement per line, for people and the benchmark script.
    pub fn report(&self) -> String {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let mut text = String::new();
        let mut line = |args: std::fmt::Arguments<'_>| {
            text.write_fmt(args).ok();
            text.push('\n');
        };
        line(format_args!(
            "package: {} bytes, {} components, {} scenes",
            self.package_bytes, self.components, self.scenes
        ));
        line(format_args!("open: {:.1} ms", ms(self.open)));
        line(format_args!("GPU upload: {:.1} ms", ms(self.upload)));
        line(format_args!(
            "orbit: {:.1} fps at {}x{} ({} frames, median {:.2} ms, worst {:.2} ms)",
            self.fps(),
            self.size[0],
            self.size[1],
            self.frames,
            ms(self.frame_median),
            ms(self.frame_worst)
        ));
        line(format_args!("pick: median {:.3} ms", ms(self.pick_median)));
        match self.scene_worst {
            Some(worst) => line(format_args!("scene switch: worst {:.1} ms", ms(worst))),
            None => line(format_args!("scene switch: no scenes")),
        }
        text
    }
}

/// Open `path`, upload it and draw `frames` orbit frames of `size` pixels on
/// `device`, then pick over the view and switch to every saved scene.
pub fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
    size: [u32; 2],
    frames: usize,
) -> Result<Benchmark, Rejection> {
    let package_bytes = std::fs::metadata(path)
        .map_err(|error| super::file_error(path, error))?
        .len();
    let mut app = ViewerApp::new(None);
    let started = Instant::now();
    app.open_path(path)?;
    let open = started.elapsed();
    let Some(model) = &app.model else {
        return Err(
            Rejection::new("viewer.benchmark.package", RejectionPhase::Request)
                .target(path.display().to_string())
                .reason("A comments file names a package; the benchmark needs the package itself.")
                .fix_hint("Benchmark the .ketchup-view file."),
        );
    };
    let started = Instant::now();
    let renderer = MeshRenderer::new(device, model)?;
    finish(device)?;
    let upload = started.elapsed();
    let target = RenderTarget::new(device, size);
    let mut camera = app.camera.clone().unwrap_or_else(|| Camera::fit(model));
    let mut frame_times = Vec::with_capacity(frames);
    for _ in 0..frames.max(1) {
        camera.orbit([ORBIT_PX_PER_FRAME, 0.0]);
        let started = Instant::now();
        renderer.render(device, queue, &target, &camera, &view(&app));
        finish(device)?;
        frame_times.push(started.elapsed());
    }
    let aspect = size[0].to_f32().expect("view width") / size[1].max(1).to_f32().expect("height");
    let mut pick_times = Vec::with_capacity(PICKS.len() * PICKS.len());
    for x in PICKS {
        for y in PICKS {
            let started = Instant::now();
            let hit = camera.pick(model, [x, y], aspect, &app.visible, app.section.as_ref());
            pick_times.push(started.elapsed());
            std::hint::black_box(hit);
        }
    }
    pick_times.sort_unstable();
    let components = model.manifest().occurrences.len();
    let scenes = model.manifest().scenes.len();
    let mut scene_worst = None;
    for index in 0..scenes {
        let started = Instant::now();
        app.activate_scene(index)?;
        let camera = app.camera.clone().expect("a scene shows its camera");
        renderer.render(device, queue, &target, &camera, &view(&app));
        finish(device)?;
        let took = started.elapsed();
        scene_worst = Some(scene_worst.map_or(took, |worst: Duration| worst.max(took)));
    }
    frame_times.sort_unstable();
    Ok(Benchmark {
        package_bytes,
        components,
        scenes,
        open,
        upload,
        size,
        frames: frame_times.len(),
        frame_median: median(&frame_times),
        frame_worst: *frame_times.last().expect("at least one frame"),
        pick_median: median(&pick_times),
        scene_worst,
    })
}

/// The frame as the window would draw it now.
fn view(app: &ViewerApp) -> RenderView<'_> {
    RenderView {
        selected: app.selected,
        visible: &app.visible,
        style: &app.style,
        section: app.section.as_ref(),
        guides: app.guides,
    }
}

/// Wait until the GPU finished everything submitted.
fn finish(device: &wgpu::Device) -> Result<(), Rejection> {
    device
        .poll(wgpu::PollType::Wait)
        .map(drop)
        .map_err(|error| {
            Rejection::new("viewer.benchmark.gpu", RejectionPhase::Io)
                .reason("The GPU did not finish the benchmark frame.")
                .fix_hint("Retry; if it persists, update the graphics driver.")
                .caused_by(error)
        })
}

/// The middle of sorted `times`.
fn median(times: &[Duration]) -> Duration {
    times[times.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn kitchen() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../viewer/samples/kitchen.ketchup-view")
    }

    #[test]
    fn benchmark_of_the_kitchen_sample_times_open_upload_orbit_pick_and_scenes() {
        let (device, queue) = ketchup_test_gpu::gpu_device();
        let path = kitchen();
        let result = run(&device, &queue, &path, [320, 240], 8).expect("benchmark");
        let bytes = std::fs::metadata(&path).expect("sample").len();
        assert_eq!(result.package_bytes, bytes);
        assert_eq!((result.components, result.scenes), (20, 1));
        assert_eq!((result.size, result.frames), ([320, 240], 8));
        assert!(result.frame_median <= result.frame_worst);
        assert!(result.fps().is_finite() && result.fps() > 0.0);
        assert!(
            result.scene_worst.is_some(),
            "the sample's scene was switched to"
        );
        let report = result.report();
        for line in ["package", "open", "GPU upload", "orbit", "pick", "scene"] {
            assert!(report.contains(line), "report names {line}:\n{report}");
        }
    }

    #[test]
    fn a_missing_package_is_refused_as_the_open_button_refuses_it() {
        let (device, queue) = ketchup_test_gpu::gpu_device();
        let missing = kitchen().with_file_name("no-such-model.ketchup-view");
        let error =
            run(&device, &queue, &missing, [64, 64], 1).expect_err("a missing file is refused");
        assert_eq!(error.code(), "viewer.file.open");
    }
}
