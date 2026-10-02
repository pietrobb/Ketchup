//! Real egui_kittest/wgpu isolated CAD pixels, not a rasterizer double.
//! Run with cargo test -p ketchup-app --test live_bridge_image.
//! No native window, physical input, production launcher or provider-delivery claim.

use eframe::egui::{self, ColorImage, Event, ViewportCommand, ViewportId, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable as _};
use ketchup_app::{AppCommand, AssistantWorkspaceMode, KetchupApp, live_bridge::*};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
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
fn send_request_version_mode(
    h: &Harness<'_, KetchupApp>,
    image_protocol_version: u32,
    capture_mode: CaptureMode,
) -> TcpStream {
    let credentials = h.state().live_bridge_credentials().unwrap();
    let mut socket = TcpStream::connect(credentials.address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    let body = serde_json::to_vec(&Envelope {
        version: 1,
        id: 1,
        token: credentials.token,
        request: Request::Image(ImageRequest {
            expected: Some(h.state().live_bridge_stamp()),
            image_protocol_version,
            capture_mode,
            max_side_px: MIN_IMAGE_SIDE_PX,
            framing: ketchup_app::live_bridge::ImageFraming::Viewport,
            detail_target: None,
        }),
    })
    .unwrap();
    socket
        .write_all(&(body.len() as u32).to_be_bytes())
        .unwrap();
    socket.write_all(&body).unwrap();
    socket
}
fn send_request_mode(h: &Harness<'_, KetchupApp>, capture_mode: CaptureMode) -> TcpStream {
    send_request_version_mode(h, IMAGE_PROTOCOL_VERSION, capture_mode)
}
fn send_request(h: &Harness<'_, KetchupApp>) -> TcpStream {
    send_request_mode(h, CaptureMode::Offscreen)
}
fn request_version_mode(
    h: &Harness<'_, KetchupApp>,
    image_protocol_version: u32,
    capture_mode: CaptureMode,
) -> mpsc::Receiver<Response> {
    let mut socket = send_request_version_mode(h, image_protocol_version, capture_mode);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut header = [0; 4];
        socket.read_exact(&mut header).unwrap();
        let size = u32::from_be_bytes(header) as usize;
        assert!(size > 0 && size <= MAX_IMAGE_FRAME_BYTES);
        let mut body = vec![0; size];
        socket.read_exact(&mut body).unwrap();
        tx.send(serde_json::from_slice(&body).unwrap()).unwrap();
    });
    rx
}
fn request_mode(
    h: &Harness<'_, KetchupApp>,
    capture_mode: CaptureMode,
) -> mpsc::Receiver<Response> {
    request_version_mode(h, IMAGE_PROTOCOL_VERSION, capture_mode)
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
/// An error answer carries only its rejection, never pixels.
fn is_pixel_free_rejection(response: &Response) -> bool {
    response.result.as_ref().is_some_and(|result| {
        result["code"].as_str() == response.error.as_deref() && result.get("data").is_none()
    })
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
    assert!(serde_json::to_vec(&response).unwrap().len() <= MAX_IMAGE_FRAME_BYTES);
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
    assert!(png.len() > 57 && png.len() < MAX_IMAGE_FRAME_BYTES);
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let (w, height) = (
        value["width"].as_u64().unwrap() as usize,
        value["height"].as_u64().unwrap() as usize,
    );
    assert!(w > 0 && height > 0);
    assert_eq!(w.max(height), MIN_IMAGE_SIDE_PX as usize);
    assert_eq!(value["requested_max_side_px"], MIN_IMAGE_SIDE_PX);
    assert_eq!(value["source_size_px"], serde_json::json!([1600, 1000]));
    assert_eq!(value["sampling"], "nearest_center");
    assert_eq!(value["thumbnail"], true);
    assert_eq!(value["image_protocol_version"], IMAGE_PROTOCOL_VERSION);
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
    let mut zlib = Vec::new();
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
            b"IDAT" => zlib.extend_from_slice(&png[offset + 8..offset + 8 + n]),
            _ => {}
        }
        offset += n + 12;
    }
    assert_eq!(offset, png.len());
    assert_eq!(&zlib[..2], &[0x78, 0x01]);
    let checksum_offset = zlib.len() - 4;
    let mut cursor = 2;
    let mut block_count = 0;
    let mut scanlines = Vec::new();
    loop {
        let final_block = match zlib[cursor] {
            0 => false,
            1 => true,
            flags => panic!("unsupported stored DEFLATE flags {flags}"),
        };
        cursor += 1;
        let len = u16::from_le_bytes(zlib[cursor..cursor + 2].try_into().unwrap()) as usize;
        let inverse = u16::from_le_bytes(zlib[cursor + 2..cursor + 4].try_into().unwrap());
        assert_eq!(inverse, !(len as u16));
        cursor += 4;
        scanlines.extend_from_slice(&zlib[cursor..cursor + len]);
        cursor += len;
        block_count += 1;
        if final_block {
            break;
        }
    }
    assert!(
        block_count > 1,
        "512 px image must exercise multiple DEFLATE blocks"
    );
    assert_eq!(cursor, checksum_offset);
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &scanlines {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    assert_eq!(&zlib[checksum_offset..], &((b << 16) | a).to_be_bytes());
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
fn unsupported_image_protocol_is_rejected_before_callback_scheduling() {
    let _gpu = (
        crate::integration_support::file_turn(),
        GPU_TEST.lock().unwrap_or_else(|error| error.into_inner()),
    );
    let mut h = harness();
    let rx = request_version_mode(&h, IMAGE_PROTOCOL_VERSION - 1, CaptureMode::Offscreen);
    let response = wait_response(&mut h, rx);
    assert!(
        !response.ok && is_pixel_free_rejection(&response),
        "{response:?}"
    );
    assert_eq!(
        response.error.as_deref(),
        Some("unsupported_image_protocol")
    );
    assert!(!has_callback(&h));
}

#[test]
fn isolated_pixels_are_bounded_stamped_private_and_view_dependent() {
    let _gpu = (
        crate::integration_support::file_turn(),
        GPU_TEST.lock().unwrap_or_else(|error| error.into_inner()),
    );
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
        let _gpu = (
            crate::integration_support::file_turn(),
            GPU_TEST.lock().unwrap_or_else(|error| error.into_inner()),
        );
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
            assert!(
                !response.ok && is_pixel_free_rejection(&response),
                "{response:?}"
            );
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
        let _gpu = (
            crate::integration_support::file_turn(),
            GPU_TEST.lock().unwrap_or_else(|error| error.into_inner()),
        );
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
            !response.ok && is_pixel_free_rejection(&response),
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
    let _gpu = (
        crate::integration_support::file_turn(),
        GPU_TEST.lock().unwrap_or_else(|error| error.into_inner()),
    );
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
    let _gpu = (
        crate::integration_support::file_turn(),
        GPU_TEST.lock().unwrap_or_else(|error| error.into_inner()),
    );
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
