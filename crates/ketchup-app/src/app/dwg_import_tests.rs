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
/// on the pair count (review 2026-10-21 P3; Levoca's DXF is 134 MB, 8.7 million pairs).
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
    let Err(error) =
        crate::app::dwg_import::read_dwg(&dwg, &ketchup_program::CancelToken::default())
    else {
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
