//! Real egui_kittest/wgpu isolated CAD pixels, not a rasterizer double.
//! Run with cargo test -p ketchup-app --test live_bridge_image.
//! Cross-language proof requires KETCHUP_LIVE_PYTHON with anthropic installed.
//! No native window, physical input, production launcher or provider-delivery claim.

use eframe::egui::{self, ColorImage, Event, ViewportCommand, ViewportId, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable as _};
use ketchup_app::{AppCommand, AssistantWorkspaceMode, KetchupApp, live_bridge::*};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static GPU_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn harness() -> Harness<'static, KetchupApp> {
    let mut app = KetchupApp::new();
    app.set_assistant_workspace_mode(AssistantWorkspaceMode::Dock);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app);
    h.ctx.style_mut(|s| s.animation_time = 0.0);
    for _ in 0..30 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    h.render()
        .expect("initialize real offscreen wgpu before request deadline");
    // This host uses egui CAD shapes. Its only native callback is isolated capture.
    assert!(!has_callback(&h));
    let ctx = h.ctx.clone();
    h.state_mut().enable_live_bridge(&ctx).unwrap();
    h
}
fn send_request_mode(h: &Harness<'_, KetchupApp>, capture_mode: CaptureMode) -> TcpStream {
    let credentials = h.state().live_bridge_credentials().unwrap();
    let mut socket = TcpStream::connect(credentials.address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    let body = serde_json::to_vec(&Envelope {
        version: 1,
        id: 1,
        token: credentials.token,
        request: Request::Image {
            expected: h.state().live_bridge_stamp(),
            capture_mode,
        },
    })
    .unwrap();
    socket
        .write_all(&(body.len() as u32).to_be_bytes())
        .unwrap();
    socket.write_all(&body).unwrap();
    socket
}
fn send_request(h: &Harness<'_, KetchupApp>) -> TcpStream {
    send_request_mode(h, CaptureMode::Offscreen)
}
fn request_mode(
    h: &Harness<'_, KetchupApp>,
    capture_mode: CaptureMode,
) -> mpsc::Receiver<Response> {
    let mut socket = send_request_mode(h, capture_mode);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut header = [0; 4];
        socket.read_exact(&mut header).unwrap();
        let size = u32::from_be_bytes(header) as usize;
        assert!(size > 0 && size <= MAX_FRAME_BYTES);
        let mut body = vec![0; size];
        socket.read_exact(&mut body).unwrap();
        tx.send(serde_json::from_slice(&body).unwrap()).unwrap();
    });
    rx
}
fn request(h: &Harness<'_, KetchupApp>) -> mpsc::Receiver<Response> {
    request_mode(h, CaptureMode::Offscreen)
}
fn has_callback(h: &Harness<'_, KetchupApp>) -> bool {
    fn contains(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Callback(_) => true,
            egui::Shape::Vec(shapes) => shapes.iter().any(contains),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|shape| contains(&shape.shape))
}
fn assert_no_screenshot_command(h: &Harness<'_, KetchupApp>) {
    assert!(
        h.output().viewport_output.values().all(|output| output
            .commands
            .iter()
            .all(|command| !matches!(command, ViewportCommand::Screenshot(_)))),
        "GUI Screenshot must never be image authority"
    );
}
fn wait_callback(h: &mut Harness<'_, KetchupApp>) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        h.step();
        assert_no_screenshot_command(h);
        if has_callback(h) {
            return h.ctx.cumulative_pass_nr() - 1;
        }
        assert!(
            Instant::now() < deadline,
            "isolated GPU callback not scheduled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn wait_response(h: &mut Harness<'_, KetchupApp>, rx: mpsc::Receiver<Response>) -> Response {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        h.step();
        assert_no_screenshot_command(h);
        // The scheduled output was already rendered (or deliberately discarded).
        assert!(
            !has_callback(h),
            "no silent rescheduling or old-frame fallback"
        );
        match rx.try_recv() {
            Ok(response) => return response,
            Err(mpsc::TryRecvError::Disconnected) => panic!("response worker closed"),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        assert!(Instant::now() < deadline, "bounded image response required");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn capture(h: &mut Harness<'_, KetchupApp>) -> serde_json::Value {
    let stamp = h.state().live_bridge_stamp();
    let rx = request(h);
    let pass = wait_callback(h);
    h.render()
        .expect("service native wgpu callback for this exact pass");
    let response = wait_response(h, rx);
    assert!(response.ok, "{response:?}");
    assert_eq!(response.stamp, Some(stamp.clone()));
    assert_eq!(h.state().live_bridge_stamp(), stamp);
    assert!(serde_json::to_vec(&response).unwrap().len() <= MAX_FRAME_BYTES);
    let value = response.result.unwrap();
    assert_eq!(value["stamp"], serde_json::to_value(stamp).unwrap());
    assert_eq!(value["capture_pass"], pass);
    assert_pixels(
        &decode64(value["data"].as_str().unwrap()),
        &value,
        h.state().viewport_rect().unwrap(),
    );
    value
}
fn decode64(s: &str) -> Vec<u8> {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::new();
    let (mut bits, mut count) = (0u32, 0);
    for c in s.bytes().filter(|c| *c != b'=') {
        bits = (bits << 6) | table.iter().position(|x| *x == c).unwrap() as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    bytes
}
fn assert_pixels(png: &[u8], value: &serde_json::Value, rect: egui::Rect) {
    assert!(png.len() > 57 && png.len() < MAX_FRAME_BYTES);
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let (w, height) = (
        value["width"].as_u64().unwrap() as usize,
        value["height"].as_u64().unwrap() as usize,
    );
    assert!(w > 0 && w <= 64 && height > 0 && height <= 64);
    assert_eq!(value["source_size_px"], serde_json::json!([1600, 1000]));
    assert_eq!(value["sampling"], "nearest_center");
    assert_eq!(value["thumbnail"], true);
    assert_eq!(value["render"]["source"], "isolated_cad_target");
    assert_eq!(value["render"]["gui_overlays_included"], false);
    assert_eq!(value["capture_mode"], "offscreen");
    assert_eq!(value["render"]["render_correlated"], true);
    assert_eq!(value["render"]["callback_correlated"], true);
    assert_eq!(value["render"]["viewport_visibility_required"], false);
    assert_eq!(value["render"]["viewport_unoccluded"], false);
    assert_eq!(value["render"]["geometry_complete"], false);
    assert_eq!(
        value["render"]["completeness"],
        "display_only_not_geometry_validation"
    );
    // Independently inspect this encoder's stored DEFLATE scanlines. These are
    // isolated pixels, NOT expected to match samples from the GUI framebuffer.
    let mut offset = 8;
    let mut scanlines = Vec::new();
    while offset < png.len() {
        let n = u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap()) as usize;
        match &png[offset + 4..offset + 8] {
            b"IHDR" => {
                assert_eq!(
                    u32::from_be_bytes(png[offset + 8..offset + 12].try_into().unwrap()) as usize,
                    w
                );
                assert_eq!(
                    u32::from_be_bytes(png[offset + 12..offset + 16].try_into().unwrap()) as usize,
                    height
                );
                assert_eq!(&png[offset + 16..offset + 21], &[8, 2, 0, 0, 0]);
            }
            b"IDAT" => {
                let z = &png[offset + 8..offset + 8 + n];
                assert_eq!(&z[..3], &[0x78, 1, 1]);
                let len = u16::from_le_bytes(z[3..5].try_into().unwrap()) as usize;
                assert_eq!(
                    u16::from_le_bytes(z[5..7].try_into().unwrap()),
                    !(len as u16)
                );
                scanlines.extend_from_slice(&z[7..7 + len]);
            }
            _ => {}
        }
        offset += n + 12;
    }
    assert_eq!(offset, png.len());
    assert_eq!(scanlines.len(), height * (w * 3 + 1));
    let crop: Vec<usize> = value["crop_px"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as usize)
        .collect();
    let ppp = value["pixels_per_point"].as_f64().unwrap() as f32;
    assert_eq!(crop.len(), 4);
    assert!(crop[0] as f32 > rect.min.x * ppp && crop[1] as f32 > rect.min.y * ppp);
    assert!(
        ((crop[0] + crop[2]) as f32) < rect.max.x * ppp
            && ((crop[1] + crop[3]) as f32) < rect.max.y * ppp
    );
    let mut colors = std::collections::BTreeSet::new();
    for row in scanlines.chunks_exact(w * 3 + 1) {
        assert_eq!(row[0], 0);
        for pixel in row[1..].chunks_exact(3) {
            colors.insert([pixel[0], pixel[1], pixel[2]]);
        }
    }
    assert!(
        colors.len() > 16,
        "nonzero, nonuniform real CAD pixels required"
    );
    assert!(
        !colors.contains(&[255, 0, 255]),
        "GUI sentinel leaked into CAD output"
    );
    println!(
        "isolated wgpu pixels: {} PNG bytes, {}x{}, {} colors",
        png.len(),
        w,
        height,
        colors.len()
    );
}
fn queue_command(h: &mut Harness<'_, KetchupApp>, command: AppCommand) {
    let label = h.state().command_label(command);
    h.query_all_by_role_and_label(Role::Button, &label)
        .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("accessible command required")
        .click_accesskit();
    h.step();
    h.step();
}
#[test]
fn isolated_pixels_are_bounded_stamped_private_and_view_dependent() {
    let _gpu = GPU_TEST.lock().unwrap_or_else(|error| error.into_inner());
    let mut h = harness();
    let initial = h.state().live_bridge_stamp();
    let baseline = capture(&mut h);
    let sentinel = Arc::new(AtomicBool::new(true));
    let flag = sentinel.clone();
    h.ctx.on_end_pass(
        "full GUI sentinel",
        Arc::new(move |ctx| {
            if flag.load(Ordering::Acquire) {
                egui::Painter::new(ctx.clone(), egui::LayerId::debug(), egui::Rect::EVERYTHING)
                    .rect_filled(ctx.screen_rect(), 0.0, egui::Color32::from_rgb(255, 0, 255));
            }
        }),
    );
    let rx = request(&h);
    let pass = wait_callback(&mut h);
    let gui = h
        .render()
        .expect("render GUI sentinel and separate CAD callback");
    assert!(
        gui.pixels().all(|pixel| pixel.0 == [255, 0, 255, 255]),
        "full GUI really carries sentinel"
    );
    sentinel.store(false, Ordering::Release); // It disappears before readback is consumed.
    let response = wait_response(&mut h, rx);
    assert!(response.ok, "{response:?}");
    assert_eq!(response.stamp, Some(initial.clone()));
    let isolated = response.result.unwrap();
    assert_eq!(isolated["capture_pass"], pass);
    assert_eq!(isolated["stamp"], serde_json::to_value(&initial).unwrap());
    assert_eq!(
        isolated["data"], baseline["data"],
        "late GUI paint must not affect any isolated byte"
    );
    queue_command(&mut h, AppCommand::ViewTop);
    let top = capture(&mut h);
    assert_eq!(h.state().live_bridge_stamp(), initial);
    assert_ne!(
        top["view"], baseline["view"],
        "AccessKit changed the camera"
    );
    assert_ne!(
        top["data"], baseline["data"],
        "pixels must respond to CAD view, not just a fixed gradient"
    );
}
#[test]
fn screenshots_are_ignored_and_unserviced_gpu_callback_times_out() {
    for service_gpu in [false, true] {
        let _gpu = GPU_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let mut h = harness();
        let rx = request(&h);
        let pass = wait_callback(&mut h);
        if service_gpu {
            h.render()
                .expect("only native GPU callback can supply image pixels");
            h.render().expect("duplicate native prepare is harmless");
        }
        // Missing Screenshot is already covered by capture(). Foreign, duplicate,
        // malformed/transparent GUI events cannot supply or invalidate authority.
        for viewport_id in [ViewportId::ROOT, ViewportId::from_hash_of("foreign")] {
            let event = Event::Screenshot {
                viewport_id,
                user_data: egui::UserData::new(0u64),
                image: Arc::new(ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT])),
            };
            h.input_mut().events.extend([event.clone(), event]);
        }
        let response = wait_response(&mut h, rx);
        if service_gpu {
            assert!(response.ok, "{response:?}");
            let value = response.result.unwrap();
            assert_eq!(value["capture_pass"], pass);
            assert_pixels(
                &decode64(value["data"].as_str().unwrap()),
                &value,
                h.state().viewport_rect().unwrap(),
            );
        } else {
            assert!(!response.ok && response.result.is_none(), "{response:?}");
            assert_eq!(response.error.as_deref(), Some("image_timeout"));
            // Lost/discarded output must not poison the next capture.
            capture(&mut h);
        }
    }
}
#[test]
fn pending_gpu_capture_rejects_precise_hidden_and_stale_states() {
    for (case, code) in [
        ("camera", "stale_image"),
        ("hidden", "hidden_viewport"),
        ("document", "stale_document"),
    ] {
        let _gpu = GPU_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let mut h = harness();
        let initial = h.state().live_bridge_stamp();
        let rx = request_mode(
            &h,
            if case == "hidden" {
                CaptureMode::VisibleViewport
            } else {
                CaptureMode::Offscreen
            },
        );
        wait_callback(&mut h);
        // Do not render: change state before delivery, never fabricate private GPU data.
        match case {
            "camera" => queue_command(&mut h, AppCommand::ViewTop),
            "hidden" => h
                .state_mut()
                .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab),
            "document" => {
                assert!(h.state_mut().create_box());
                assert_eq!(
                    h.state().live_bridge_stamp().document_id,
                    initial.document_id
                );
                assert!(h.state().live_bridge_stamp().mutation_epoch > initial.mutation_epoch);
                assert_ne!(
                    h.state().live_bridge_stamp().canonical_digest,
                    initial.canonical_digest
                );
                assert!(h.state().live_bridge_credentials().is_some());
            }
            _ => unreachable!(),
        }
        let response = wait_response(&mut h, rx);
        assert!(
            !response.ok && response.result.is_none(),
            "{case}: {response:?}"
        );
        assert_eq!(
            response.error.as_deref(),
            Some(code),
            "{case}: {response:?}"
        );
    }
}
#[test]
fn offscreen_capture_works_while_the_gui_canvas_is_not_visible() {
    let _gpu = GPU_TEST.lock().unwrap_or_else(|error| error.into_inner());
    let mut h = harness();
    h.state_mut()
        .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
    let stamp = h.state().live_bridge_stamp();
    let rx = request_mode(&h, CaptureMode::Offscreen);
    let pass = wait_callback(&mut h);
    h.render()
        .expect("service private GPU target without a visible CAD canvas");
    let response = wait_response(&mut h, rx);
    assert!(response.ok, "{response:?}");
    assert_eq!(response.stamp, Some(stamp.clone()));
    let value = response.result.unwrap();
    assert_eq!(value["stamp"], serde_json::to_value(stamp).unwrap());
    assert_eq!(value["capture_pass"], pass);
    assert_eq!(value["capture_mode"], "offscreen");
    assert_eq!(value["render"]["render_correlated"], true);
    assert_eq!(value["render"]["viewport_visibility_required"], false);
    assert_eq!(value["render"]["viewport_unoccluded"], false);
    assert_pixels(
        &decode64(value["data"].as_str().unwrap()),
        &value,
        h.state().viewport_rect().unwrap(),
    );
}
#[test]
fn disconnected_capture_is_revoked_before_reconnect() {
    let _gpu = GPU_TEST.lock().unwrap_or_else(|error| error.into_inner());
    let mut h = harness();
    let stamp = h.state().live_bridge_stamp();
    let mut old = send_request(&h);
    wait_callback(&mut h);
    old.shutdown(Shutdown::Write).unwrap();
    old.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(
        old.read(&mut [0u8; 1]).unwrap(),
        0,
        "cancelled session closes without image response"
    );
    // Execute retained output after transport cancellation. No public debug hook.
    h.render().expect("cancelled callback is safe to service");
    drop(old);
    capture(&mut h); // No stale busy flag or old stamp reused by the new session.
    assert_eq!(h.state().live_bridge_stamp(), stamp);
    queue_command(&mut h, AppCommand::ViewTop);
    capture(&mut h);
}

// Never Debug or relay arbitrary child output: stdin owns the private attachment.
struct Python(Child);
impl Drop for Python {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn isolated_frame_reaches_registered_python_image_tool_and_new_png() {
    let Some(python) = std::env::var_os("KETCHUP_LIVE_PYTHON") else {
        eprintln!("SKIP: set KETCHUP_LIVE_PYTHON to Python 3.11+ with anthropic installed");
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let destination = root.join("artifacts/live-view").join(format!(
        "s4c-isolated-{}-{}.png",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    assert!(destination.is_absolute() && !destination.exists());
    let _gpu = GPU_TEST.lock().unwrap_or_else(|error| error.into_inner());
    let mut h = harness();
    // Independent raw TCP response on the SAME stable CAD state, before the
    // persistent Python session attaches. Never substitute synthetic source pixels.
    let raw = capture(&mut h);
    let raw_png = decode64(raw["data"].as_str().unwrap());
    let credentials = h.state().live_bridge_credentials().unwrap();
    let initial = h.state().live_bridge_stamp();
    let count = h.state().document_snapshot().occurrences().count();
    let history = h.state().undo_step_count();
    let mut child = Python(
        Command::new(python)
            .arg("-B")
            .arg("-u")
            .arg(root.join("tests/live_bridge_image_client.py"))
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start explicitly configured Python + anthropic"),
    );
    let mut input = child.0.stdin.take().unwrap();
    let mut attachment = serde_json::to_vec(&serde_json::json!({
        "address": credentials.address.to_string(), "token": credentials.token, "image_path": destination,
    })).expect("encode private attachment");
    attachment.push(b'\n');
    assert!(
        input.write_all(&attachment).is_ok(),
        "write private attachment"
    );
    attachment.fill(0);
    drop(attachment);
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let out_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout.take(1024 * 1024)).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.take(1024 * 1024).read_to_end(&mut bytes);
        (result.is_ok(), bytes)
    });
    let mut capture_pass = None;
    for checkpoint in [
        "initial",
        "plan_guarded",
        "captured",
        "create_only",
        "disconnected",
    ] {
        let deadline = Instant::now() + Duration::from_secs(45);
        let line = loop {
            match rx.try_recv() {
                Ok(Ok(line)) => break line,
                Ok(Err(_)) => panic!("Python output read failed at {checkpoint}"),
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("Python exited before {checkpoint}")
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "Python checkpoint deadline: {checkpoint}"
            );
            h.step();
            assert_no_screenshot_command(&h);
            if has_callback(&h) {
                assert_eq!(
                    checkpoint, "captured",
                    "unexpected capture during guard/cleanup"
                );
                assert!(
                    capture_pass.is_none(),
                    "no silent retry or cached-frame fallback"
                );
                capture_pass = Some(h.ctx.cumulative_pass_nr() - 1);
                assert_eq!(h.state().live_bridge_stamp(), initial);
            }
            // Actual Harness.render services production native callbacks. Its GUI
            // return image is deliberately NOT used as the CAD source reference.
            h.render()
                .expect("service isolated wgpu callback during Python TCP call");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(line.len() <= MAX_FRAME_BYTES, "oversized checkpoint");
        assert!(
            !line.contains(&credentials.token),
            "credential leaked in Python stdout"
        );
        assert!(
            !line.contains(&credentials.address.to_string()),
            "endpoint leaked in Python stdout"
        );
        let event: serde_json::Value =
            serde_json::from_str(&line).unwrap_or_else(|_| panic!("invalid sanitized checkpoint"));
        assert!(
            event["checkpoint"].as_str() == Some(checkpoint),
            "Python helper failed or checkpoint out of order at {checkpoint}"
        );
        assert_eq!(event["stamp"], serde_json::to_value(&initial).unwrap());
        assert_eq!(
            h.state().live_bridge_stamp(),
            initial,
            "image must not mutate GUI store"
        );
        assert_eq!(h.state().document_snapshot().occurrences().count(), count);
        assert_eq!(h.state().undo_step_count(), history);
        if checkpoint == "captured" {
            let receipt = &event["receipt"];
            let value = &receipt["result"];
            assert_eq!(receipt["ok"], true);
            assert_eq!(receipt["stamp"], event["stamp"]);
            assert_eq!(value["stamp"], event["stamp"]);
            assert!(value.get("data").is_none());
            let artifact = &value["artifact"];
            assert_eq!(Path::new(artifact["path"].as_str().unwrap()), destination);
            assert_eq!(artifact["artifact_saved"], true);
            assert_eq!(artifact["visual_delivery"], "unverified");
            assert_eq!(artifact["geometry_evaluated"], false);
            let png = std::fs::read(&destination).expect("accepted image MUST exist");
            assert_eq!(artifact["byte_count"].as_u64().unwrap(), png.len() as u64);
            assert_eq!(
                value["capture_pass"],
                capture_pass.expect("actual native GPU callback required")
            );
            assert!(
                value["capture_pass"].as_u64().unwrap() > raw["capture_pass"].as_u64().unwrap()
            );
            assert_pixels(&png, value, h.state().viewport_rect().unwrap());
            assert_eq!(
                png, raw_png,
                "skill artifact must equal raw Rust isolated PNG on stable CAD state"
            );
            for field in [
                "stamp",
                "view",
                "selection",
                "render",
                "crop_px",
                "source_size_px",
            ] {
                assert_eq!(
                    value[field], raw[field],
                    "stable isolated metadata: {field}"
                );
            }
            println!(
                "S4c isolated artifact: {} (SHA-256 {})",
                destination.display(),
                artifact["sha256"].as_str().unwrap()
            );
        } else if capture_pass.is_none() {
            assert!(!destination.exists(), "guard wrote an artifact");
        }
        assert!(
            input.write_all(b"continue\n").is_ok(),
            "checkpoint acknowledgement failed"
        );
    }
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("wait for Python") {
            break status;
        }
        assert!(Instant::now() < deadline, "Python exit deadline");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    };
    out_thread.join().expect("stdout collector panicked");
    let (read_ok, stderr) = err_thread.join().expect("stderr collector panicked");
    assert!(read_ok, "Python stderr read failed");
    assert!(
        !stderr
            .windows(credentials.token.len())
            .any(|w| w == credentials.token.as_bytes()),
        "credential leaked in Python stderr"
    );
    assert!(
        !stderr
            .windows(credentials.address.to_string().len())
            .any(|w| w == credentials.address.to_string().as_bytes()),
        "endpoint leaked in Python stderr"
    );
    assert!(
        stderr.is_empty(),
        "Python emitted unexpected stderr (suppressed)"
    );
    assert!(
        rx.try_recv().is_err(),
        "unexpected trailing stdout (suppressed)"
    );
    assert!(status.success(), "Python helper failed (output suppressed)");
    h.step();
    assert_eq!(h.state().live_bridge_stamp(), initial);
    // Human UI remains usable after the non-owning skill/socket and helper close.
    queue_command(&mut h, AppCommand::ViewTop);
    assert_eq!(h.state().live_bridge_stamp(), initial);
    let top = capture(&mut h);
    assert_ne!(top["view"], raw["view"]);
    assert_ne!(top["data"], raw["data"]);
    assert_eq!(h.state().live_bridge_stamp(), initial);
    assert_eq!(h.state().undo_step_count(), history);
}
