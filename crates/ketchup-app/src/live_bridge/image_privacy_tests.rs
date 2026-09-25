//! Real offscreen GPU privacy proof; no Screenshot event supplies image pixels.
use super::*;
use egui_kittest::Harness;
use std::sync::atomic::AtomicUsize;

static GPU_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_test_guard() -> std::sync::MutexGuard<'static, ()> {
    GPU_TEST.lock().unwrap_or_else(|error| error.into_inner())
}

fn settle_exact(h: &mut Harness<'_, KetchupApp>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().exact_task.is_some() {
        assert!(Instant::now() < deadline, "exact scene did not settle");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn queue_with_options(
    h: &mut Harness<'_, KetchupApp>,
    session: u64,
    framing: ImageFraming,
    detail_target: Option<ImageDetailTarget>,
    max_side_px: u32,
) -> mpsc::Receiver<Response> {
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let mut bridge = app.live_bridge.take().unwrap();
    bridge.session = session;
    let (reply, rx) = mpsc::sync_channel(1);
    bridge.request_image(
        app,
        &ctx,
        Queued {
            session,
            id: session,
            request: Request::Image {
                expected: Some(app.live_bridge_stamp()),
                image_protocol_version: IMAGE_PROTOCOL_VERSION,
                capture_mode: CaptureMode::Offscreen,
                max_side_px,
                framing,
                detail_target,
            },
            connection_closed: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            reply,
        },
    );
    app.live_bridge = Some(bridge);
    rx
}

fn queue_with_framing(
    h: &mut Harness<'_, KetchupApp>,
    session: u64,
    framing: ImageFraming,
) -> mpsc::Receiver<Response> {
    queue_with_options(h, session, framing, None, MIN_IMAGE_SIDE_PX)
}

fn queue(h: &mut Harness<'_, KetchupApp>, session: u64) -> mpsc::Receiver<Response> {
    queue_with_framing(h, session, ImageFraming::Viewport)
}
fn response(h: &mut Harness<'_, KetchupApp>, rx: &mpsc::Receiver<Response>) -> Response {
    for _ in 0..250 {
        h.step();
        if let Ok(response) = rx.try_recv() {
            return response;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no bounded image response");
}

fn render_private_capture(h: &mut Harness<'_, KetchupApp>, description: &str) {
    for _ in 0..10 {
        if h.render().is_ok() {
            return;
        }
        h.ctx.request_repaint();
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no bounded GPU frame for {description}");
}
#[test]
fn isolated_pixels_exclude_late_transformed_and_same_layer_sentinels() {
    let _gpu = gpu_test_guard();
    let mut app = KetchupApp::new();
    app.set_assistant_workspace_mode(crate::AssistantWorkspaceMode::Dock);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app);
    h.ctx.style_mut(|style| style.animation_time = 0.0);
    for _ in 0..30 {
        h.step();
    }
    settle_exact(&mut h);
    h.render().expect("initialize actual offscreen renderer");
    let ctx = h.ctx.clone();
    h.state_mut().enable_live_bridge(&ctx).unwrap();
    let mode = Arc::new(AtomicUsize::new(0));
    let draw_mode = mode.clone();
    let rect = h.state().viewport_rect().unwrap().shrink(20.0);
    // Registered after the app: only the capture frame carries this sentinel.
    ctx.on_end_pass(
        "private capture-frame sentinel",
        Arc::new(move |ctx| {
            let mode = draw_mode.load(Ordering::Acquire);
            if mode == 0 {
                return;
            }
            let layer = if mode == 3 {
                egui::LayerId::background()
            } else {
                egui::LayerId::new(egui::Order::Foreground, egui::Id::new("private sentinel"))
            };
            let offset = if mode == 2 {
                egui::vec2(4000.0, 0.0)
            } else {
                egui::Vec2::ZERO
            };
            ctx.set_transform_layer(layer, egui::emath::TSTransform::from_translation(-offset));
            egui::Painter::new(ctx.clone(), layer, egui::Rect::EVERYTHING)
                .with_clip_rect(egui::Rect::EVERYTHING)
                .rect_filled(
                    rect.translate(offset),
                    0.0,
                    egui::Color32::from_rgb(255, 0, 255),
                );
        }),
    );
    let mut baseline = None;
    for variant in 0..4 {
        let rx = queue(&mut h, variant as u64 + 1);
        mode.store(variant, Ordering::Release);
        h.step();
        let capture_pass = h
            .state()
            .live_bridge
            .as_ref()
            .unwrap()
            .image
            .pending
            .as_ref()
            .unwrap()
            .capture
            .as_ref()
            .unwrap()
            .0
            .pass;
        let rendered = h
            .render()
            .expect("execute production isolated callback on real wgpu");
        if variant != 0 {
            assert!(
                rendered.pixels().any(|p| p.0 == [255, 0, 255, 255]),
                "sentinel variant {variant} must actually paint in the GUI"
            );
        }
        mode.store(0, Ordering::Release);
        let reply = response(&mut h, &rx);
        assert!(reply.ok, "{reply:?}");
        let image = reply.result.unwrap();
        assert_eq!(image["capture_pass"], capture_pass);
        assert_eq!(
            image["stamp"],
            serde_json::to_value(h.state().live_bridge_stamp()).unwrap()
        );
        assert_eq!(image["render"]["source"], "isolated_cad_target");
        assert_eq!(image["render"]["geometry_complete"], false);
        let data = image["data"].as_str().unwrap().to_owned();
        assert!(data.len() > 1000, "real bounded PNG, not empty metadata");
        if let Some(baseline) = &baseline {
            assert_eq!(&data, baseline, "GUI sentinel changed isolated CAD pixels");
        } else {
            baseline = Some(data);
        }
    }
}
#[test]
fn discarded_capture_and_session_replacement_never_reuse_authority() {
    let _gpu = gpu_test_guard();
    let mut app = KetchupApp::new();
    app.set_assistant_workspace_mode(crate::AssistantWorkspaceMode::Dock);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app);
    for _ in 0..20 {
        h.step();
    }
    let ctx = h.ctx.clone();
    h.state_mut().enable_live_bridge(&ctx).unwrap();
    let old = queue(&mut h, 1);
    let discard = Arc::new(AtomicBool::new(true));
    let flag = discard.clone();
    ctx.on_end_pass(
        "discard capture once",
        Arc::new(move |ctx| {
            if flag.swap(false, Ordering::AcqRel) {
                ctx.request_discard("privacy regression");
            }
        }),
    );
    h.step(); // No renderer runs the discarded pass.
    let bridge = h.state_mut().live_bridge.as_mut().unwrap();
    bridge
        .image
        .pending
        .as_ref()
        .unwrap()
        .queued
        .cancelled
        .store(true, Ordering::Release);
    let new = queue(&mut h, 2); // Before any intervening finish: must not inherit busy.
    assert!(
        h.state()
            .live_bridge
            .as_ref()
            .unwrap()
            .image
            .pending
            .is_some()
    );
    assert!(matches!(new.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(matches!(
        old.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    let bridge = h.state_mut().live_bridge.as_mut().unwrap();
    bridge.image.revoke();
    assert!(bridge.image.pending.is_none());
    assert!(matches!(
        new.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}

fn native_harness_with_size(ppp: f32, size: egui::Vec2) -> Harness<'static, KetchupApp> {
    let mut app = KetchupApp::new();
    app.set_assistant_workspace_mode(crate::AssistantWorkspaceMode::Dock);
    // Select the same ScenePaintCallback branch as the native app. No scene
    // resources are installed in the outer harness renderer: only the private
    // production target creates them. No desktop window or input is involved.
    app.wgpu_target_format = Some(wgpu::TextureFormat::Rgba8Unorm);
    app.grid_axes_visible = false;
    app.white_background_visible = true;
    app.shadows_visible = false;
    app.fog_visible = false;
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(ppp)
        .build_state(|ctx, app: &mut KetchupApp| app.ui(ctx), app);
    h.ctx.style_mut(|style| style.animation_time = 0.0);
    for _ in 0..30 {
        h.step();
    }
    settle_exact(&mut h);
    h.render()
        .expect("native offscreen wgpu is required, not skipped");
    let ctx = h.ctx.clone();
    h.state_mut().enable_live_bridge(&ctx).unwrap();
    h
}

fn native_harness(ppp: f32) -> Harness<'static, KetchupApp> {
    native_harness_with_size(ppp, egui::vec2(1200.0, 800.0))
}

fn capture<'a>(h: &'a Harness<'_, KetchupApp>) -> &'a (Painted, target::Readback, bool) {
    h.state()
        .live_bridge
        .as_ref()
        .unwrap()
        .image
        .pending
        .as_ref()
        .unwrap()
        .capture
        .as_ref()
        .expect("capture must be scheduled")
}

fn native_pixel_proof(ppp: f32) -> Value {
    let mut h = native_harness(ppp);
    let stamp = h.state().live_bridge_stamp();
    let plan = h.state().render_plan.as_ref().unwrap();
    assert_eq!(plan.batches().len(), 1, "built-in 100 x 60 x 20 mm box");
    assert_eq!(plan.batches()[0].instances.len(), 1);
    assert_eq!(plan.batches()[0].geometry.index_count(), 36);
    assert!(h.state().selection.primary.is_none());

    // Consume a real private-target readback as the full-resolution oracle.
    // This request is explicitly revoked afterwards; a separate normal request
    // below must deliver the matching thumbnail through finish_image.
    let oracle_rx = queue(&mut h, 1);
    h.step();
    assert_eq!(capture(&h).0.callbacks, 1);
    let outer = h
        .render()
        .expect("execute actual ScenePaintCallback and map_async");
    let pixels = (0..250)
        .find_map(|_| {
            let result = capture(&h).1.take().expect("GPU readback must succeed");
            if result.is_none() {
                std::thread::sleep(Duration::from_millis(10));
            }
            result
        })
        .expect("real private GPU pixels must arrive");
    let painted = &capture(&h).0;
    assert_eq!(pixels.pass, painted.pass);
    assert_eq!(
        pixels.nonce,
        h.state()
            .live_bridge
            .as_ref()
            .unwrap()
            .image
            .pending
            .as_ref()
            .unwrap()
            .nonce
    );
    assert_eq!(painted.ppp, ppp);
    assert_eq!(
        pixels.image.size,
        [(1200.0 * ppp) as usize, (800.0 * ppp) as usize]
    );

    // An interior point of the default box's top face, not an axis/grid pixel.
    // The outer GUI is white here because it lacks scene callback resources;
    // the independent private renderer must shade actual model triangles.
    let top = h
        .state()
        .project_to_screen(crate::Vec3::new(50.0, 30.0, 20.0), painted.rect);
    let sx = (top.x * ppp).floor() as usize;
    let sy = (top.y * ppp).floor() as usize;
    assert!(painted.rect.shrink(4.0).contains(top));
    for y in sy - 2..=sy + 2 {
        for x in sx - 2..=sx + 2 {
            assert_eq!(
                outer.get_pixel(x as u32, y as u32).0,
                [255; 4],
                "reference has no model at {x},{y}"
            );
            let rgba = pixels.image.pixels[y * pixels.image.size[0] + x].to_array();
            assert_eq!(rgba[3], 255);
            assert!(
                rgba[..3].iter().all(|c| *c < 245),
                "box face must differ from background: {rgba:?}"
            );
        }
    }
    let rect = painted.rect.shrink(2.0);
    let x0 = (rect.min.x * ppp).ceil() as usize;
    let y0 = (rect.min.y * ppp).ceil() as usize;
    let sw = (rect.max.x * ppp).floor() as usize - x0;
    let sh = (rect.max.y * ppp).floor() as usize - y0;
    let w = (f64::from(MIN_IMAGE_SIDE_PX) * sw as f64 / sw.max(sh) as f64).floor() as usize;
    let height = (f64::from(MIN_IMAGE_SIDE_PX) * sh as f64 / sw.max(sh) as f64).floor() as usize;
    let mut rgb = Vec::new();
    for y in 0..height {
        for x in 0..w {
            let source_x = x0 + ((x as f64 + 0.5) * sw as f64 / w as f64).floor() as usize;
            let source_y = y0 + ((y as f64 + 0.5) * sh as f64 / height as f64).floor() as usize;
            let rgba = pixels.image.pixels[source_y * pixels.image.size[0] + source_x].to_array();
            assert_eq!(rgba[3], 255);
            rgb.extend_from_slice(&rgba[..3]);
        }
    }
    let shaded = rgb
        .chunks_exact(3)
        .filter(|p| p.iter().all(|c| *c < 245))
        .count();
    let white = rgb.chunks_exact(3).filter(|p| *p == [255; 3]).count();
    assert!(
        shaded > 20,
        "model must survive thumbnail sampling: {shaded}"
    );
    assert!(
        white > 20,
        "thumbnail must also contain background: {white}"
    );
    let expected_png = base64(&png_rgb(w, height, &rgb));
    h.state_mut().live_bridge.as_mut().unwrap().image.revoke();
    assert!(matches!(
        oracle_rx.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));

    let rx = queue(&mut h, 2);
    h.step();
    let pass = capture(&h).0.pass;
    h.render()
        .expect("render normal response using native scene callback");
    let reply = response(&mut h, &rx);
    assert!(reply.ok, "{reply:?}");
    assert_eq!(reply.stamp, Some(stamp.clone()));
    assert!(serde_json::to_vec(&reply).unwrap().len() <= MAX_IMAGE_FRAME_BYTES);
    let image = reply.result.unwrap();
    assert_eq!(
        image["data"], expected_png,
        "every thumbnail sample must match the actual GPU source"
    );
    assert_eq!(image["stamp"], serde_json::to_value(&stamp).unwrap());
    assert_eq!(h.state().live_bridge_stamp(), stamp);
    assert_eq!(image["capture_pass"], pass);
    assert_eq!(image["source_size_px"], json!(pixels.image.size));
    assert_eq!(image["crop_px"], json!([x0, y0, sw, sh]));
    assert_eq!(image["pixels_per_point"], ppp);
    assert_eq!(image["width"], w);
    assert_eq!(image["height"], height);
    assert_eq!(image["sampling"], "nearest_center");
    assert_eq!(image["requested_max_side_px"], MIN_IMAGE_SIDE_PX);
    assert_eq!(image["render"]["scene_callbacks"], 1);
    assert_eq!(image["render"]["callback_correlated"], true);
    assert_eq!(image["render"]["source"], "isolated_cad_target");
    assert_eq!(image["render"]["gui_overlays_included"], false);
    assert_eq!(image["render"]["geometry_complete"], false);
    assert_eq!(
        image["render"]["completeness"],
        "display_only_not_geometry_validation"
    );
    println!(
        "native ScenePaintCallback ppp={ppp}: source={:?}, crop={:?}, thumbnail={w}x{height}, shaded={shaded}, white={white}, 25 projected face pixels differ from outer background; all {} RGB samples match",
        pixels.image.size,
        [x0, y0, sw, sh],
        w * height
    );
    image
}

/// A client that asks for Zoom Fit and an image right after the model changed
/// (e.g. right after launch) must receive the framed view, not the camera
/// that existed before the fit could be applied.
#[test]
fn image_requested_during_a_pending_zoom_fit_shows_the_framed_camera() {
    let _gpu = gpu_test_guard();
    let mut h = native_harness(1.0);
    assert!(h.state_mut().create_box());
    // The new box's exact evaluation starts on the next frame; let it land so
    // the only thing the image waits for is the pending fit.
    h.step();
    settle_exact(&mut h);
    h.state_mut().zoom = 0.05;
    h.state_mut().zoom_fit_pending = true;
    let rx = queue(&mut h, 41);
    h.step();
    assert!(!h.state().zoom_fit_pending);
    render_private_capture(&mut h, "image after a deferred zoom fit");
    let response = response(&mut h, &rx);
    assert!(response.ok, "{:?}", response.error);
    assert!(!h.state().zoom_fit_pending);
    let framed_zoom = f64::from(h.state().zoom);
    let result = response.result.unwrap();
    let image_zoom = result["view"]["zoom"].as_f64().unwrap();
    assert!(
        (image_zoom - framed_zoom).abs() < 1e-6 && image_zoom > 0.05,
        "image zoom {image_zoom} must be the framed zoom {framed_zoom}"
    );
}

#[test]
fn native_scene_callback_draws_default_box_pixels() {
    let _gpu = gpu_test_guard();
    native_pixel_proof(1.0);
}

#[test]
fn selection_framing_crops_real_cad_pixels_without_mutating_view_or_selection() {
    let _gpu = gpu_test_guard();
    let mut h = native_harness(1.0);
    h.state_mut().selection.clear();
    h.state_mut()
        .selection
        .select_occurrence(OccurrenceId(1), false);
    let stamp = h.state().live_bridge_stamp();
    let camera = h.state().camera_view_state();
    let selection = h.state().selection.occurrences.clone();
    let primary = h.state().selection.primary.clone();

    let selection_rx = queue_with_framing(&mut h, 1, ImageFraming::Selection);
    if h.state()
        .live_bridge
        .as_ref()
        .unwrap()
        .image
        .pending
        .is_none()
    {
        panic!(
            "selection framing request was rejected: {:?}",
            selection_rx.try_recv().unwrap()
        );
    }
    h.step();
    assert!(capture(&h).0.crop.width() < capture(&h).0.rect.width());
    h.render()
        .expect("render selection-framed private CAD viewport");
    let framed = response(&mut h, &selection_rx).result.unwrap();

    assert_eq!(framed["framing"]["mode"], "selection");
    assert_eq!(framed["framing"]["occurrence_ids"], json!([1]));
    assert_eq!(framed["thumbnail"], false);
    let source = framed["source_size_px"].as_array().unwrap();
    let crop = framed["crop_px"].as_array().unwrap();
    assert!(
        crop[2].as_u64().unwrap() < source[0].as_u64().unwrap()
            || crop[3].as_u64().unwrap() < source[1].as_u64().unwrap(),
        "selection framing must crop the real CAD target"
    );
    assert_eq!(h.state().live_bridge_stamp(), stamp);
    assert_eq!(h.state().camera_view_state(), camera);
    assert_eq!(h.state().selection.occurrences, selection);
    assert_eq!(h.state().selection.primary, primary);
}

#[test]
fn host_topology_detail_framing_crops_real_pixels_without_gui_selection() {
    let _gpu = gpu_test_guard();
    let mut h = native_harness_with_size(1.0, egui::vec2(2200.0, 1200.0));
    let viewport_rx =
        queue_with_options(&mut h, 40, ImageFraming::Viewport, None, MAX_IMAGE_SIDE_PX);
    h.step();
    render_private_capture(&mut h, "1600 px private CAD target");
    let viewport = response(&mut h, &viewport_rx).result.unwrap();
    assert_eq!(viewport["requested_max_side_px"], MAX_IMAGE_SIDE_PX);
    assert_eq!(
        viewport["width"]
            .as_u64()
            .unwrap()
            .max(viewport["height"].as_u64().unwrap()),
        u64::from(MAX_IMAGE_SIDE_PX)
    );
    assert_eq!(viewport["source_size_px"], json!([2200, 1200]));
    assert_eq!(viewport["render"]["source"], "isolated_cad_target");
    assert_eq!(viewport["render"]["gui_overlays_included"], false);

    crate::tests::install_initial_graph_result(h.state_mut());
    h.state_mut().selection.clear();
    for _ in 0..3 {
        h.step();
    }
    let (entity_id, reference_id) = {
        let app = h.state();
        let page = app
            .live_bridge
            .as_ref()
            .unwrap()
            .query
            .page_with_topology(
                &app.document.current(),
                &app.topology_results,
                &PageRequest {
                    kind: EntityKind::Edges,
                    limit: 10,
                    search: String::new(),
                    definition_id: Some(1),
                    tag_id: None,
                    classification_dimension_id: None,
                    classification_category_id: None,
                    world_bounds_mm: None,
                    cursor: None,
                },
            )
            .unwrap();
        let edge = page["items"].as_array().unwrap().first().unwrap();
        (
            edge["id"].as_u64().unwrap(),
            edge["reference_id"].as_str().unwrap().to_owned(),
        )
    };
    let stamp = h.state().live_bridge_stamp();
    let camera = h.state().camera_view_state();
    let rx = queue_with_options(
        &mut h,
        41,
        ImageFraming::DetailSelection,
        Some(ImageDetailTarget {
            occurrence_id: 1,
            kind: EntityKind::Edges,
            entity_id,
        }),
        MAX_IMAGE_SIDE_PX,
    );
    h.step();
    assert!(capture(&h).0.crop.width() < capture(&h).0.rect.width());
    render_private_capture(&mut h, "host-resolved edge detail");
    let framed = response(&mut h, &rx).result.unwrap();

    assert_eq!(framed["framing"]["mode"], "detail_selection");
    assert_eq!(framed["framing"]["occurrence_ids"], json!([1]));
    assert_eq!(framed["framing"]["detail"]["kind"], "edge");
    assert_eq!(framed["framing"]["detail"]["entity_id"], entity_id);
    assert_eq!(framed["framing"]["detail"]["reference_id"], reference_id);
    assert_eq!(framed["requested_max_side_px"], MAX_IMAGE_SIDE_PX);
    assert_eq!(framed["render"]["source"], "isolated_cad_target");
    assert_eq!(framed["render"]["gui_overlays_included"], false);
    assert_eq!(h.state().live_bridge_stamp(), stamp);
    assert_eq!(h.state().camera_view_state(), camera);
    assert!(h.state().selection.occurrences.is_empty());
    assert!(h.state().selection.primary.is_none());
}

#[test]
fn native_scene_callback_hidpi_source_crop_and_samples_are_consistent() {
    let _gpu = gpu_test_guard();
    let one = native_pixel_proof(1.0);
    let two = native_pixel_proof(2.0);
    assert_eq!(one["width"], two["width"]);
    assert_eq!(one["height"], two["height"]);
    for axis in 0..2 {
        assert_eq!(
            two["source_size_px"][axis].as_u64().unwrap(),
            2 * one["source_size_px"][axis].as_u64().unwrap()
        );
    }
    for component in 0..4 {
        let a = one["crop_px"][component].as_i64().unwrap();
        let b = two["crop_px"][component].as_i64().unwrap();
        assert!(
            (b - 2 * a).abs() <= 2,
            "only inward pixel rounding may differ"
        );
    }
    // Do not assert cross-DPI byte identity: rasterization/AA sample locations
    // legitimately differ. Each DPI independently matches its real GPU source.
}

#[test]
fn pending_native_capture_rejects_exact_registry_replacement_without_document_mutation() {
    let _gpu = gpu_test_guard();
    use ketchup_core::exact_brep_graph::ExactBRepGraph;
    use ketchup_core::exact_product::{
        ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence, ExactBodyPackage,
    };
    use ketchup_core::import::{StepImportMesh, StepMeshTriangle};

    let mut h = native_harness(1.0);
    let snapshot = h.state().document.current();
    let graph =
        ExactBRepGraph::from_snapshot(&snapshot, crate::DefinitionId(1), crate::FeatureId(2))
            .unwrap();
    // Provided package fixture for the built-in box, using the existing headless
    // publication hook. This tests registry identity, NOT worker execution,
    // kernel correctness, topology validation, or an exact-geometry claim.
    let mesh = StepImportMesh {
        vertices_mm: vec![
            [0.0, 0.0, 0.0],
            [100.0, 0.0, 0.0],
            [0.0, 60.0, 0.0],
            [100.0, 60.0, 0.0],
            [0.0, 0.0, 20.0],
            [100.0, 0.0, 20.0],
            [0.0, 60.0, 20.0],
            [100.0, 60.0, 20.0],
        ],
        triangles: [
            ([0, 2, 1], 0),
            ([1, 2, 3], 0),
            ([4, 5, 6], 1),
            ([5, 7, 6], 1),
            ([0, 1, 4], 2),
            ([1, 5, 4], 2),
            ([2, 6, 3], 3),
            ([3, 6, 7], 3),
            ([0, 4, 2], 4),
            ([2, 4, 6], 4),
            ([1, 3, 5], 5),
            ([3, 7, 5], 5),
        ]
        .into_iter()
        .map(|(vertex_indices, face_ordinal)| StepMeshTriangle {
            vertex_indices,
            face_ordinal,
        })
        .collect(),
    };
    let package = |fingerprint: &str| {
        ExactBodyPackage::Graph(
            ExactBRepGraphPackage::from_worker_evidence(
                &graph,
                ExactBRepGraphWorkerEvidence {
                    exact_input_digest: "image-registry-guard-fixture-input".into(),
                    result_fingerprint: fingerprint.into(),
                    volume_mm3: 120_000.0,
                    area_mm2: 0.0,
                    topology_counts: [8, 12, 6, 1, 1],
                    wire_count: None,
                    bounds_mm: [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
                    backend: "image-registry-guard-test-fixture.v1".into(),
                    tolerance: "1e-7-mm".into(),
                    faces: Vec::new(),
                    edges: Vec::new(),
                },
                &mesh,
            )
            .unwrap(),
        )
    };
    assert!(
        h.state_mut()
            .headless_install_exact_package(package("image-fixture-a"))
    );
    h.step();
    let before = VisualState::read(h.state()).unwrap();
    let rx = queue(&mut h, 1);
    h.step();
    assert_eq!(capture(&h).0.callbacks, 1);
    assert_eq!(capture(&h).0.state.exact, before.exact);
    h.render()
        .expect("submit native GPU capture before registry replacement");
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    // Same geometry, same document/revision/epoch, new exact result identity,
    // published while asynchronous GPU capture is pending consumption.
    assert!(
        h.state_mut()
            .headless_install_exact_package(package("image-fixture-b"))
    );
    let after = VisualState::read(h.state()).unwrap();
    assert_eq!(before.stamp, after.stamp);
    assert_eq!(before.camera, after.camera);
    assert_eq!(before.exact_complete, after.exact_complete);
    assert_eq!(before.evaluating, after.evaluating);
    assert_ne!(before.exact, after.exact);
    let reply = response(&mut h, &rx);
    assert!(
        !reply.ok,
        "replaced exact registry must invalidate already-submitted pixels"
    );
    assert_eq!(reply.error.as_deref(), Some("stale_image"));
    assert!(reply.result.is_none());
    assert!(
        h.state()
            .live_bridge
            .as_ref()
            .unwrap()
            .image
            .pending
            .is_none()
    );
    assert_eq!(h.state().live_bridge_stamp(), before.stamp);
    println!(
        "pending native capture rejected stale_image after exact contents stamp {} -> {}; document stamp unchanged",
        before.exact, after.exact
    );
}
