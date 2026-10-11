//! Android entry point of the Kečup Viewer: opens packages and comment files
//! other apps hand over or the customer picks, and the package placed in the
//! app's external data directory by the development install.

#[cfg(target_os = "android")]
mod files;

/// Called by android-activity when the app starts.
#[cfg(target_os = "android")]
#[allow(unsafe_code)] // android-activity resolves this unique Rust-ABI entry point.
#[unsafe(no_mangle)]
fn android_main(android_app: android_activity::AndroidApp) {
    // Android discards stdout/stderr; logcat is the only place errors show.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("KetchupViewer"),
    );
    std::panic::set_hook(Box::new(|info| log::error!("{info}")));
    let path = android_app
        .external_data_path()
        .map(|path| path.join("model.ketchup-view"));
    let mut options = eframe::NativeOptions {
        android_app: Some(android_app.clone()),
        renderer: eframe::Renderer::Wgpu,
        run_and_return: false,
        ..Default::default()
    };
    let defaults = eframe::egui_wgpu::WgpuSetupCreateNew::default();
    let setup = eframe::egui_wgpu::WgpuSetupCreateNew {
        instance_descriptor: eframe::wgpu::InstanceDescriptor {
            backends: backend(),
            ..defaults.instance_descriptor.clone()
        },
        // Phone GPUs often allow less than wgpu's desktop defaults (e.g. 16 KiB
        // uniform buffers); the Viewer fits within what the device reports.
        device_descriptor: std::sync::Arc::new(|adapter| eframe::wgpu::DeviceDescriptor {
            label: Some("Kečup Viewer device"),
            required_limits: adapter.limits(),
            ..Default::default()
        }),
        ..defaults
    };
    options.wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(setup);
    let result = eframe::run_native(
        "Kečup Viewer",
        options,
        Box::new(move |cc| {
            let mut app = ketchup_viewer::app::ViewerApp::new(cc.wgpu_render_state.clone())
                .with_host_sharing();
            if let Some(path) = path
                && path.exists()
                && let Err(error) = app.open_path(&path)
            {
                log::error!("{error}; {}", error.fix_hint_text());
            }
            Ok(Box::new(AndroidViewer {
                app,
                files: files::Files::new(android_app.clone()),
                android_app,
                measured_for: None,
                bars: [0; 4],
                keyboard: 0,
            }))
        }),
    );
    if let Err(error) = result {
        log::error!("Kečup Viewer Android startup failed: {error}");
    }
}

/// The Viewer with Android's file traffic, kept out of the status bar,
/// gesture bar, camera cutout and on-screen keyboard: apps targeting
/// Android 15+ are drawn edge to edge, under all of them, and a
/// NativeActivity window does not shrink for the keyboard, which would
/// otherwise cover the comment being typed.
#[cfg(target_os = "android")]
struct AndroidViewer {
    app: ketchup_viewer::app::ViewerApp,
    files: files::Files,
    android_app: android_activity::AndroidApp,
    /// Screen size the bar insets were read for; rotation reads them again.
    measured_for: Option<eframe::egui::Vec2>,
    /// System bar and cutout insets in pixels as [left, top, right, bottom].
    bars: [i32; 4],
    /// Height of the on-screen keyboard in pixels, 0 when closed.
    keyboard: i32,
}

#[cfg(target_os = "android")]
impl eframe::App for AndroidViewer {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        use eframe::egui;
        let (screen, pixels_per_point) =
            ctx.input(|input| (input.screen_rect().size(), input.pixels_per_point()));
        let typing = ctx.wants_keyboard_input();
        let measured = self.measured_for == Some(screen);
        if !measured || typing || self.keyboard > 0 {
            match system_insets(&self.android_app) {
                Ok((bars, keyboard)) => {
                    // Insets arrive once the window is laid out; keep asking until then.
                    if !measured && (bars != [0; 4] || ctx.cumulative_frame_nr() > 60) {
                        log::info!("System bar insets {bars:?} px for screen {screen:?} pt");
                        self.measured_for = Some(screen);
                    }
                    if typing && self.keyboard > 0 && keyboard == 0 {
                        // Closed with Back: release the field so a tap opens it again.
                        ctx.memory_mut(|memory| memory.stop_text_input());
                    }
                    self.bars = bars;
                    self.keyboard = keyboard;
                }
                Err(error) => {
                    if !measured {
                        log::error!("Reading the system bar insets failed: {error}");
                        self.measured_for = Some(screen);
                    }
                }
            }
        }
        // The keyboard slides in after the field is focused; follow it.
        if typing || self.keyboard > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut insets = self.bars;
        insets[3] = insets[3].max(self.keyboard);
        let [left, top, right, bottom] = insets.map(|px| px.max(0) as f32 / pixels_per_point);
        let frame_fill = egui::Frame::NONE.fill(ctx.style().visuals.panel_fill);
        if top > 0.0 {
            egui::TopBottomPanel::top("safe_area_top")
                .exact_height(top)
                .frame(frame_fill)
                .show_separator_line(false)
                .show(ctx, |_| {});
        }
        if bottom > 0.0 {
            egui::TopBottomPanel::bottom("safe_area_bottom")
                .exact_height(bottom)
                .frame(frame_fill)
                .show_separator_line(false)
                .show(ctx, |_| {});
        }
        if left > 0.0 {
            egui::SidePanel::left("safe_area_left")
                .exact_width(left)
                .resizable(false)
                .frame(frame_fill)
                .show_separator_line(false)
                .show(ctx, |_| {});
        }
        if right > 0.0 {
            egui::SidePanel::right("safe_area_right")
                .exact_width(right)
                .resizable(false)
                .frame(frame_fill)
                .show_separator_line(false)
                .show(ctx, |_| {});
        }
        self.files.receive(&mut self.app);
        self.app.update(ctx, frame);
        for request in self.app.take_file_requests() {
            if let Err(error) = self.files.request(&mut self.app, request) {
                self.app.report(error);
            }
        }
        // The picker's answer comes back with the activity, not as input.
        if self.files.waiting {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

/// Vulkan, unless the only Vulkan driver is the emulator's gfxstream, which
/// crashes inside vkQueueSubmit; then OpenGL ES. A window can carry only one
/// API's surface, so the choice is made before the window surface exists.
#[cfg(target_os = "android")]
fn backend() -> eframe::wgpu::Backends {
    use eframe::wgpu;
    let probe = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        flags: wgpu::InstanceFlags::empty(),
        ..Default::default()
    });
    let usable_vulkan = probe
        .enumerate_adapters(wgpu::Backends::VULKAN)
        .iter()
        .any(|adapter| adapter.get_info().driver != "gfxstream");
    let chosen = if usable_vulkan {
        wgpu::Backends::VULKAN
    } else {
        wgpu::Backends::GL
    };
    log::info!("GPU backend: {chosen:?}");
    chosen
}

/// System bar and display cutout insets in pixels as [left, top, right, bottom],
/// and the on-screen keyboard height in pixels. Below Android 15 the window is
/// neither drawn under the bars nor covered by the keyboard, so all are zero.
#[cfg(target_os = "android")]
fn system_insets(
    android_app: &android_activity::AndroidApp,
) -> Result<([i32; 4], i32), jni::errors::Error> {
    use jni::{jni_sig, jni_str};
    files::with_activity(android_app, |env, activity| {
        let sdk = env
            .get_static_field(
                jni_str!("android/os/Build$VERSION"),
                jni_str!("SDK_INT"),
                jni_sig!("I"),
            )?
            .i()?;
        if sdk < 35 {
            return Ok(([0; 4], 0));
        }
        let window = env
            .call_method(
                activity,
                jni_str!("getWindow"),
                jni_sig!("()Landroid/view/Window;"),
                &[],
            )?
            .l()?;
        let decor = env
            .call_method(
                &window,
                jni_str!("getDecorView"),
                jni_sig!("()Landroid/view/View;"),
                &[],
            )?
            .l()?;
        let insets = env
            .call_method(
                &decor,
                jni_str!("getRootWindowInsets"),
                jni_sig!("()Landroid/view/WindowInsets;"),
                &[],
            )?
            .l()?;
        if insets.is_null() {
            return Ok(([0; 4], 0));
        }
        let mut kind = |names: &[&jni::strings::JNIStr]| -> jni::errors::Result<i32> {
            let mut kind = 0;
            for name in names {
                kind |= env
                    .call_static_method(
                        jni_str!("android/view/WindowInsets$Type"),
                        name,
                        jni_sig!("()I"),
                        &[],
                    )?
                    .i()?;
            }
            Ok(kind)
        };
        let bars_kind = kind(&[jni_str!("systemBars"), jni_str!("displayCutout")])?;
        let keyboard_kind = kind(&[jni_str!("ime")])?;
        let mut sides = |kind: i32| -> jni::errors::Result<[i32; 4]> {
            let edges = env
                .call_method(
                    &insets,
                    jni_str!("getInsets"),
                    jni_sig!("(I)Landroid/graphics/Insets;"),
                    &[kind.into()],
                )?
                .l()?;
            let mut sides = [0; 4];
            for (side, name) in sides.iter_mut().zip([
                jni_str!("left"),
                jni_str!("top"),
                jni_str!("right"),
                jni_str!("bottom"),
            ]) {
                *side = env.get_field(&edges, name, jni_sig!("I"))?.i()?;
            }
            Ok(sides)
        };
        let bars = sides(bars_kind)?;
        let keyboard = sides(keyboard_kind)?[3];
        Ok((bars, keyboard))
    })
}
