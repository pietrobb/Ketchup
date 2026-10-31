//! File > Import DXF/DWG in the window: a `.dwg` drawing opens the DXF review
//! with the DWG version named, and confirming it imports the profiles in one
//! Undo step.
use crate::app::dwg_import::LAYERED_DRAWING_DXF;
use crate::tests::shell::step_until;
use crate::*;
use egui_kittest::kittest::Queryable as _;

/// Review SEC-5 and DR-3: in the window a DWG file is read on a thread, in the
/// program worker, while the window shows that it is busy. A file that declares
/// a 300 GiB section ends with a refusal in the status line, not with the
/// window, and the next file opens its review.
#[test]
fn the_window_reads_a_dwg_file_on_a_thread_and_outlives_a_hostile_one() {
    let directory = tempfile::tempdir().unwrap();
    let dwg = ketchup_dwg::dxf_to_dwg(LAYERED_DRAWING_DXF.as_bytes()).unwrap();
    let (hostile, plan) = (
        directory.path().join("hostile.dwg"),
        directory.path().join("plan.dwg"),
    );
    std::fs::write(
        &hostile,
        ketchup_dwg::testing::with_declared_header_section(&dwg, 300 << 30),
    )
    .unwrap();
    std::fs::write(&plan, &dwg).unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Dxf, &hostile)
        .queue_import(ImportFormat::Dxf, &plan);
    let app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    let undo = harness.state().undo_step_count();

    for path in [&hostile, &plan] {
        // The command as the File menu dispatches it, inside a frame.
        let context = harness.ctx.clone();
        let app = harness.state_mut();
        app.file.activity.context = Some(context);
        app.dispatch_file_command(AppCommand::ImportDrawingDxf);
        assert!(
            app.file.activity.dwg_reading.is_some(),
            "{}",
            path.display()
        );
        assert!(app.modal.get::<PendingDxfImport>().is_none());
        let label = app.file_activity_label().expect("busy");
        harness.step();
        // Shown while the file is read, unless it was read within the frame.
        assert!(
            harness
                .query_all(egui_kittest::kittest::by().label(&label))
                .next()
                .is_some()
                || harness.state().file.activity.dwg_reading.is_none()
        );
        step_until(
            &mut harness,
            Duration::from_secs(60),
            "the DWG file was not read",
            |app| app.file.activity.dwg_reading.is_none(),
        );
    }

    let app = harness.state();
    let pending = app
        .modal
        .get::<PendingDxfImport>()
        .unwrap_or_else(|| panic!("no review: {}", app.action_digest()));
    assert_eq!(pending.plan.source.path, plan);
    assert_eq!(pending.plan.review.profiles().len(), 3);
    assert_eq!(app.undo_step_count(), undo);
}

/// A DWG read has no deadline: while it runs, the window shows how far it is estimated to
/// be and how long is left, from the file's size, then that it takes longer than estimated;
/// Cancel stops it.
#[test]
fn a_long_dwg_read_shows_its_estimated_progress_and_cancel_stops_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.dwg");
    // 16 MB: estimated at 2 s.
    std::fs::File::create(&path)
        .unwrap()
        .set_len(16_000_000)
        .unwrap();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(
            |context, app: &mut KetchupApp| app.ui(context),
            KetchupApp::new(),
        );
    harness.step();
    let (token, reader_token) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let context = harness.ctx.clone();
    harness
        .state_mut()
        .queue_dwg_read(path, &context, move |_, cancel| {
            token.send(cancel.clone()).unwrap();
            let _ = released.recv();
            Err(ImportError::failed(ImportFormat::Dxf, "cancelled"))
        });
    let reader_token = reader_token.recv_timeout(Duration::from_secs(10)).unwrap();
    // The read's clock is fixed, so slow frames under load change nothing shown.
    let shown = |harness: &mut egui_kittest::Harness<'_, KetchupApp>, elapsed: u64| {
        let task = harness
            .state_mut()
            .file
            .activity
            .dwg_reading
            .as_mut()
            .unwrap();
        task.progress.fixed_elapsed = Some(Duration::from_secs(elapsed));
        harness.step();
        let (fraction, text) = harness.state().drawing_read_estimate().unwrap();
        assert!(
            harness
                .query_all(egui_kittest::kittest::by().label(&text))
                .next()
                .is_some(),
            "{text}"
        );
        (fraction, text)
    };
    let (fraction, text) = shown(&mut harness, 1);
    assert_eq!(fraction, 0.5);
    let estimate = harness.state().catalog.format(
        "file-activity-read-estimate",
        &BTreeMap::from([("percent", "50".to_owned()), ("seconds", "1".to_owned())]),
    );
    assert_eq!(text, estimate);
    let (fraction, text) = shown(&mut harness, 3);
    assert_eq!(fraction, 0.99);
    let overdue = harness.state().catalog.format(
        "file-activity-read-overdue",
        &BTreeMap::from([("seconds", "3".to_owned())]),
    );
    assert_eq!(text, overdue);

    harness
        .get_by_label(&harness.state().catalog.text("action-cancel"))
        .click();
    harness.step();
    assert!(harness.state().file.activity.dwg_reading.is_none());
    assert!(reader_token.is_cancelled(), "the reader saw the cancel");
    drop(release);
}

/// A DWG drawing whose DXF has more group-code pairs than a drawing import reads is
/// refused in the worker, so the window never reads its whole DXF only to refuse it
/// on the pair count (review 2026-10-21 P3). A file past the import's own limit
/// ([`ketchup_model::import::MAX_DXF_PAIRS`], some 120 MB of DXF) is too large for a
/// test, so the worker is given a smaller one.
#[test]
fn a_dwg_with_more_pairs_than_an_import_reads_is_refused_before_the_window_reads_its_dxf() {
    let mut dxf = String::from("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n");
    dxf.push_str("0\nSECTION\n2\nENTITIES\n");
    // Six pairs a line and more for its handle and layer: over 100,000 pairs.
    for i in 0..20_000 {
        dxf.push_str(&format!("0\nLINE\n8\n0\n10\n{i}\n20\n0\n11\n{i}\n21\n1\n"));
    }
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    let dwg = ketchup_dwg::dxf_to_dwg(dxf.as_bytes()).unwrap();
    let Err(error) = ketchup_application::program_isolation::convert_dwg(
        &dwg,
        100_000,
        &ketchup_program::CancelToken::default(),
    ) else {
        panic!("the window read the DXF of a drawing an import refuses on its pairs");
    };
    let message = error.to_string();
    assert!(message.contains("100000 group-code pairs"), "{message}");
    assert!(message.contains("as an underlay"), "{message}");
}

/// The refusal of the hostile file stays in the status line until the next
/// file is chosen; read alone so the line is checked before it is replaced.
#[test]
fn the_window_names_why_a_hostile_dwg_file_was_refused() {
    let directory = tempfile::tempdir().unwrap();
    let dwg = ketchup_dwg::dxf_to_dwg(LAYERED_DRAWING_DXF.as_bytes()).unwrap();
    let hostile = directory.path().join("hostile.dwg");
    std::fs::write(
        &hostile,
        ketchup_dwg::testing::with_declared_header_section(&dwg, 300 << 30),
    )
    .unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &hostile);
    let app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.step();
    let context = harness.ctx.clone();
    let app = harness.state_mut();
    app.file.activity.context = Some(context);
    app.dispatch_file_command(AppCommand::ImportDrawingDxf);
    step_until(
        &mut harness,
        Duration::from_secs(60),
        "the DWG file was not read",
        |app| app.file.activity.dwg_reading.is_none(),
    );
    let app = harness.state();
    assert!(app.digest.contains("hostile.dwg"), "{}", app.digest);
    // macOS does not enforce the worker's memory limit (see process-wrap's
    // JobMemoryLimit), so there the reader refuses the file with its own error.
    #[cfg(not(target_os = "macos"))]
    assert!(app.digest.contains("2 GiB of memory"), "{}", app.digest);
    assert!(app.modal.get::<PendingDxfImport>().is_none());
}

#[test]
fn the_window_reviews_and_imports_a_dwg_drawing() {
    let directory = tempfile::tempdir().unwrap();
    let dwg = directory.path().join("plan.dwg");
    std::fs::write(
        &dwg,
        ketchup_dwg::dxf_to_dwg(LAYERED_DRAWING_DXF.as_bytes()).unwrap(),
    )
    .unwrap();
    let dialogs = dialogs::ScriptedFileDialogs::new().queue_import(ImportFormat::Dxf, &dwg);
    let script = dialogs.clone();
    let mut app = KetchupApp::new().with_dialogs(Box::new(dialogs));
    let undo = app.undo_step_count();

    app.dispatch_file_command(AppCommand::ImportDrawingDxf);
    let [request] = script.import_requests().try_into().unwrap();
    assert_eq!(request.extensions, ["dxf", "dwg"]);
    assert_eq!(
        request.filter_label,
        app.catalog.text("file-filter-dxf-dwg")
    );
    let pending = app
        .modal
        .get::<PendingDxfImport>()
        .unwrap_or_else(|| panic!("no review: {}", app.action_digest()));
    assert_eq!(pending.plan.review.profiles().len(), 3);
    let conversion = pending.plan.dwg.clone().expect("read from a DWG file");
    assert_eq!(conversion.version, ketchup_dwg::WRITTEN_VERSION);
    assert!(conversion.skipped.is_empty());
    assert!(pending.unit_confirmed, "the DWG declares millimetres");

    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    let named = harness.state().catalog.format(
        "dialog-import-dxf-dwg",
        &BTreeMap::from([("version", conversion.version.clone())]),
    );
    harness.get_by_label(&named);
    let confirm = harness.state().catalog.text("dialog-import-dxf-confirm");
    harness.get_by_label(&confirm).click();
    harness.run();

    let app = harness.state();
    assert!(app.modal.get::<PendingDxfImport>().is_none());
    assert_eq!(
        app.digest,
        app.catalog.format(
            "digest-imported-dxf",
            &BTreeMap::from([("path", dwg.display().to_string())]),
        )
    );
    assert_eq!(app.undo_step_count(), undo + 1);
}

/// Review 2026-10-17 F-2: a DWG drawing reads as the DXF it converts to, so a
/// layout's frame (paper space) and a hidden entity are left out and counted
/// there too, not made profiles beside the model's.
#[test]
fn a_dwg_drawing_leaves_out_its_paper_space_and_invisible_entities() {
    use ketchup_model::import::{DxfImportOptions, inspect_dxf};
    let hidden = LAYERED_DRAWING_DXF.replace(
        "0\nENDSEC\n0\nEOF\n",
        concat!(
            "0\nLINE\n8\noutline\n67\n1\n10\n0\n20\n0\n11\n10\n21\n0\n",
            "0\nCIRCLE\n8\nholes\n60\n1\n10\n80\n20\n80\n40\n4\n",
            "0\nENDSEC\n0\nEOF\n",
        ),
    );
    let read = |dxf: &str| {
        let dwg = ketchup_dwg::dxf_to_dwg(dxf.as_bytes()).unwrap();
        let drawing = ketchup_dwg::dwg_to_dxf(&dwg, usize::MAX).unwrap();
        inspect_dxf(&drawing.dxf, DxfImportOptions::new(None)).unwrap()
    };
    let plain = read(LAYERED_DRAWING_DXF);
    let parsed = read(&hidden);
    assert_eq!(parsed.profiles(), plain.profiles());
    let counted = |code: &str| {
        parsed
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == code)
            .map(|diagnostic| diagnostic.count())
            .sum::<u32>()
    };
    assert_eq!(counted("dxf.entity-paper-space"), 1);
    assert_eq!(counted("dxf.entity-invisible"), 1);
}

/// A solid hatch with an island and a hatch of its own pattern lines come through the
/// DWG bridge as an underlay paints and draws them read from the DXF itself.
#[test]
fn a_dwg_drawing_keeps_its_solid_and_patterned_hatches_in_an_underlay() {
    use ketchup_model::import::{DxfImportOptions, DxfUnderlay, read_dxf_underlay};
    let hatched = LAYERED_DRAWING_DXF.replace(
        "0\nENDSEC\n0\nEOF\n",
        concat!(
            // Solid, red ByLayer: a square of 40 mm with a square island of 20 mm.
            "0\nHATCH\n8\ncut\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n2\n",
            "92\n3\n72\n0\n73\n1\n93\n4\n10\n0\n20\n20\n10\n40\n20\n20\n10\n40\n20\n60\n",
            "10\n0\n20\n60\n97\n0\n",
            "92\n2\n72\n0\n73\n1\n93\n4\n10\n10\n20\n30\n10\n30\n20\n30\n10\n30\n20\n50\n",
            "10\n10\n20\n50\n97\n0\n",
            "75\n0\n76\n1\n98\n0\n",
            // Lines at 45° 2 mm apart, of 1 mm dashes, in a square edge path of 20 mm.
            "0\nHATCH\n8\nholes\n10\n0\n20\n0\n30\n0\n2\nMINE\n70\n0\n71\n0\n91\n1\n",
            "92\n1\n93\n4\n72\n1\n10\n100\n20\n0\n11\n120\n21\n0\n",
            "72\n1\n10\n120\n20\n0\n11\n120\n21\n20\n72\n1\n10\n120\n20\n20\n11\n100\n21\n20\n",
            "72\n1\n10\n100\n20\n20\n11\n100\n21\n0\n97\n0\n",
            "75\n0\n76\n1\n52\n45\n41\n1\n77\n0\n78\n1\n",
            "53\n45\n43\n0\n44\n0\n45\n-1.4142135623730951\n46\n1.4142135623730951\n",
            "79\n2\n49\n1\n49\n-1\n98\n0\n",
            "0\nENDSEC\n0\nEOF\n",
        ),
    );
    let read = |dxf: &[u8]| read_dxf_underlay(dxf, DxfImportOptions::new(None)).unwrap();
    // Per layer: the area its fills paint, their color, and how many lines it draws.
    let drawn = |underlay: &DxfUnderlay| {
        underlay
            .layers
            .iter()
            .zip(underlay.lines.layers())
            .map(|(name, layer)| {
                let area = layer
                    .fills()
                    .iter()
                    .flat_map(|fill| fill.triangles())
                    .map(|[a, b, c]| {
                        ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0
                    })
                    .sum::<f64>();
                let colors = layer
                    .fills()
                    .iter()
                    .map(|fill| underlay.lines.style(fill.style()).color)
                    .collect::<Vec<_>>();
                (
                    name.clone(),
                    (area * 1e6).round() / 1e6,
                    colors,
                    layer.segment_count(),
                )
            })
            .collect::<Vec<_>>()
    };
    let direct = read(hatched.as_bytes());
    let dwg = ketchup_dwg::dxf_to_dwg(hatched.as_bytes()).unwrap();
    let bridged = read(&ketchup_dwg::dwg_to_dxf(&dwg, usize::MAX).unwrap().dxf);
    let cut = drawn(&direct)
        .into_iter()
        .find(|(name, ..)| name == "cut")
        .unwrap();
    assert_eq!(
        (cut.1, cut.2.as_slice()),
        (1200.0, &[Some([255, 0, 0])][..])
    );
    assert_eq!(drawn(&bridged), drawn(&direct));
    let skipped = |underlay: &DxfUnderlay| {
        underlay
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code() == "dxf.hatch-skipped")
            .count()
    };
    assert_eq!((skipped(&direct), skipped(&bridged)), (0, 0));
}
