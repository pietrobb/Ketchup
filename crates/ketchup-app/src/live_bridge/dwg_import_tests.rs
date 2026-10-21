//! `file action=import` of a `.dwg` drawing: it imports as the profiles of its
//! DXF conversion in one Undo step, the review names the DWG version and what
//! the DWG reader left out, and a file that is no DWG or is too large is
//! refused without changing the document.

use super::*;
use crate::app::dwg_import::{LAYERED_DRAWING_DXF, read_dwg};
use crate::live_bridge::program_check::ProgramCheckJob;
use crate::live_bridge::program_dry_run::QueuedReply;
use ketchup_model::import::ImportFormat;
use ketchup_program::CancelToken;

fn layered_dwg(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(
        &path,
        ketchup_dwg::dxf_to_dwg(LAYERED_DRAWING_DXF.as_bytes()).unwrap(),
    )
    .unwrap();
    path
}

fn importing_dwg(bridge: &LiveBridge) -> bool {
    matches!(
        bridge.program_check_job,
        Some(ProgramCheckJob::DwgImport(_))
    )
}

fn import(path: &Path) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: false,
    }
}

fn import_underlay(path: &Path) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
        as_underlay: true,
    }
}

fn export_profiles(path: &Path) -> Request {
    Request::Export {
        expected: None,
        path: path.display().to_string(),
        export_format: "dxf".to_owned(),
    }
}

#[test]
fn a_dwg_drawing_imports_as_the_profiles_of_its_dxf_in_one_undo_step() {
    let directory = tempfile::tempdir().unwrap();
    let (dxf, dwg) = (
        directory.path().join("plan.dxf"),
        directory.path().join("plan.dwg"),
    );
    std::fs::write(&dxf, LAYERED_DRAWING_DXF).unwrap();
    std::fs::write(
        &dwg,
        ketchup_dwg::dxf_to_dwg(LAYERED_DRAWING_DXF.as_bytes()).unwrap(),
    )
    .unwrap();

    let mut imported = Vec::new();
    for (index, source) in [&dxf, &dwg].into_iter().enumerate() {
        let (mut app, mut bridge) = setup();
        let undo = app.undo_step_count();
        let answer = bridge
            .execute(&mut app, import(source), false)
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        assert_eq!(answer["imported"], true, "{answer}");
        assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
        let profiles = directory.path().join(format!("profiles-{index}.dxf"));
        bridge
            .execute(&mut app, export_profiles(&profiles), false)
            .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
        imported.push((answer["review"].clone(), std::fs::read(&profiles).unwrap()));
    }
    let [(from_dxf, dxf_profiles), (from_dwg, dwg_profiles)] = imported.as_slice() else {
        unreachable!()
    };

    assert_eq!(from_dxf["dwg"], Value::Null);
    assert_eq!(
        from_dwg["dwg"],
        json!({"version": ketchup_dwg::WRITTEN_VERSION, "skipped_records": []})
    );
    for field in [
        "profiles",
        "layers",
        "layers_total",
        "unit",
        "unit_from_file",
    ] {
        assert_eq!(from_dwg[field], from_dxf[field], "{field}");
    }
    assert_eq!(from_dwg["profiles"], 3);
    assert_eq!(from_dwg["layers"], json!(["cut", "holes", "outline"]));
    assert_eq!(from_dwg["unit"], "mm");
    assert_eq!(from_dwg["unit_from_file"], true);
    assert_eq!(
        dwg_profiles, dxf_profiles,
        "the same profiles, written out again"
    );
}

#[test]
fn a_file_that_is_no_dwg_is_refused_and_changes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let garbage = directory.path().join("garbage.dwg");
    std::fs::write(&garbage, b"AC1032 but no drawing follows").unwrap();
    let (mut app, mut bridge) = setup();
    let (digest, undo) = (app.canonical_digest(), app.undo_step_count());

    assert_eq!(
        bridge.execute(&mut app, import(&garbage), false),
        Err("import_failed")
    );
    let error = app
        .import_file(ImportFormat::Dxf, &garbage, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("DWG file"), "{error}");
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo);
}

/// Review SEC-5: a DWG file whose section map declares 300 GiB. Read in the
/// window's process the reader aborts it on the allocation; read in the program
/// worker it is refused, and the next file imports.
#[test]
fn a_dwg_declaring_a_huge_section_is_refused_and_the_window_lives_on() {
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
    let (mut app, mut bridge) = setup();
    let (digest, undo) = (app.canonical_digest(), app.undo_step_count());

    assert_eq!(
        bridge.execute(&mut app, import(&hostile), false),
        Err("import_failed")
    );
    let error = app
        .import_file(ImportFormat::Dxf, &hostile, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("2 GiB of memory"), "{error}");
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo);

    let answer = bridge
        .execute(&mut app, import(&plan), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    assert_eq!(answer["review"]["profiles"], 3, "{answer}");
    assert_eq!(app.undo_step_count(), undo + 1);
}

/// Review DR-3: a client's DWG import does not read the file on the window's
/// thread. The poll that takes the request only starts a job and returns; the
/// window keeps polling, names the import as busy, and imports in a later poll.
#[test]
fn a_clients_dwg_import_reads_the_file_off_the_windows_thread() {
    let directory = tempfile::tempdir().unwrap();
    let dwg = layered_dwg(directory.path(), "plan.dwg");
    let mut wire = Wire::new();
    wire.call(Request::Status {});
    let undo = wire.app.undo_step_count();

    wire.send(import(&dwg), false);
    let mut reader = wire.stream.try_clone().unwrap();
    let (sender, answers) = mpsc::channel();
    let reading = std::thread::spawn(move || {
        let mut header = [0; 4];
        reader.read_exact(&mut header).unwrap();
        let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
        reader.read_exact(&mut bytes).unwrap();
        sender
            .send(serde_json::from_slice::<Response>(&bytes).unwrap())
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    while !importing_dwg(wire.app.live.bridge.as_ref().unwrap()) {
        assert!(
            Instant::now() < deadline,
            "the import was never a job: the window read the file itself"
        );
        assert!(answers.try_recv().is_err(), "answered without a job");
        wire.app.poll_live_bridge(&wire.context);
        std::thread::yield_now();
    }
    let bridge = wire.app.live.bridge.as_ref().unwrap();
    let busy = serde_json::to_value(bridge.busy_jobs()).unwrap();
    assert_eq!(busy[0]["target"], "import", "{busy}");
    assert_eq!(wire.app.undo_step_count(), undo, "nothing imported yet");

    let answer = loop {
        if let Ok(answer) = answers.try_recv() {
            break answer;
        }
        assert!(Instant::now() < deadline, "the import did not finish");
        wire.app.poll_live_bridge(&wire.context);
        std::thread::yield_now();
    };
    reading.join().unwrap();
    assert!(answer.ok, "{answer:?}");
    let result = answer.result.unwrap();
    assert_eq!(result["review"]["profiles"], 3, "{result}");
    assert_eq!(
        result["review"]["dwg"]["version"],
        ketchup_dwg::WRITTEN_VERSION
    );
    assert_eq!(wire.app.undo_step_count(), undo + 1, "one Undo step");
}

/// A cancelled DWG import is answered at once, stops its worker and changes
/// nothing.
#[test]
fn a_cancelled_dwg_import_changes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let dwg = layered_dwg(directory.path(), "plan.dwg");
    let (mut app, mut bridge) = setup();
    let (digest, undo) = (app.canonical_digest(), app.undo_step_count());
    let context = egui::Context::default();
    let (reply, answers) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let queued = QueuedReply {
        id: 1,
        reply,
        cancelled: Arc::clone(&cancelled),
    };

    bridge.start_dwg_import(&mut app, &context, queued, import(&dwg), false);
    assert!(importing_dwg(&bridge));
    cancelled.store(true, Ordering::Release);
    bridge.poll_program_check_job(&mut app, &context);

    let answer = answers.try_recv().expect("answered in the same poll");
    assert_eq!(answer.error.as_deref(), Some("request_cancelled"));
    assert!(bridge.program_check_job.is_none());
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo);
}

/// A conversion an open review or an import still holds is never dropped from
/// the cache, so publishing it does not read the file again on the window's
/// thread, however many other DWG files were read meanwhile.
#[test]
fn a_held_dwg_conversion_is_not_read_again() {
    let drawing = |radius: &str| {
        let dxf = LAYERED_DRAWING_DXF.replace("40\n7.5\n", &format!("40\n{radius}\n"));
        ketchup_dwg::dxf_to_dwg(dxf.as_bytes()).unwrap()
    };
    let cancel = CancelToken::default();
    let reviewed = drawing("7.25");
    let held = read_dwg(&reviewed, &cancel).unwrap();
    for radius in ["7.125", "7.0625", "7.03125"] {
        read_dwg(&drawing(radius), &cancel).unwrap();
    }

    let again = read_dwg(&reviewed, &cancel).unwrap();
    assert!(Arc::ptr_eq(&held, &again), "read again in the worker");
}

/// `file action=import as_underlay=true`: a DWG drawing is kept as an underlay in one Undo
/// step and adds no part; the summary names its tags, `tag_visibility` hides one layer and
/// `remove_underlay` takes it out again in one more step.
#[test]
fn a_dwg_drawing_imports_as_an_underlay_shown_by_its_tags_and_is_removed_again() {
    let directory = tempfile::tempdir().unwrap();
    let dwg = layered_dwg(directory.path(), "plan.dwg");
    let (mut app, mut bridge) = setup();
    let (undo, occurrences) = (
        app.undo_step_count(),
        app.document.current().occurrences().count(),
    );

    let answer = bridge
        .execute(&mut app, import_underlay(&dwg), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));

    assert_eq!(answer["imported"], true, "{answer}");
    // In the order the drawing first draws on them.
    assert_eq!(
        answer["underlay"]["layers"],
        json!(["outline", "cut", "holes"])
    );
    assert_eq!(
        answer["underlay"]["dwg_version"],
        ketchup_dwg::WRITTEN_VERSION
    );
    assert_eq!(app.undo_step_count(), undo + 1, "one Undo step");
    assert_eq!(app.document.current().occurrences().count(), occurrences);
    let summary = bridge
        .execute(&mut app, Request::Summary {}, false)
        .unwrap();
    let underlay = &summary["underlays"][0];
    assert_eq!(underlay["id"], answer["underlay_id"], "{summary}");
    let layer_tag = underlay["layers"][0]["tag"]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{summary}"))
        .to_owned();
    let hide = Request::TagVisibility {
        expected: None,
        name: layer_tag.clone(),
        visible: false,
    };
    bridge.execute(&mut app, hide, false).unwrap();
    assert!(app.underlay_draws()[0].visible.len() == 2);

    let remove = Request::RemoveUnderlay {
        expected: None,
        underlay_id: answer["underlay_id"].as_u64().unwrap(),
    };
    let removed = bridge.execute(&mut app, remove, false).unwrap();
    assert_eq!(removed["removed"], true);
    assert_eq!(app.undo_step_count(), undo + 3);
    assert_eq!(app.document.current().underlays().count(), 0);
    let summary = bridge
        .execute(&mut app, Request::Summary {}, false)
        .unwrap();
    assert!(
        !summary["tags"].to_string().contains(&layer_tag),
        "{summary}"
    );
}

#[test]
fn only_a_drawing_imports_as_an_underlay() {
    let directory = tempfile::tempdir().unwrap();
    let stl = directory.path().join("part.stl");
    std::fs::write(&stl, "solid empty\nendsolid empty\n").unwrap();
    let (mut app, mut bridge) = setup();
    let digest = app.canonical_digest();
    let code = bridge
        .execute(&mut app, import_underlay(&stl), false)
        .unwrap_err();
    assert_eq!(code, "invalid_params");
    assert_eq!(app.canonical_digest(), digest);
}

/// `inspect action=underlay_snap`: a drawing in S-JTSK metres, moved near the origin on
/// import, snaps at the end, middle and line of its line and where another crosses it,
/// names the point in the drawing's own coordinates, measures the line and between two
/// snaps; a hidden layer gives nothing.
#[test]
fn a_client_snaps_to_and_measures_an_underlay_line_in_its_drawing_coordinates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("site.dxf");
    let site = concat!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n6\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n",
        "0\nLINE\n8\nparcel\n10\n-500000\n20\n-1200000\n11\n-499990\n21\n-1200000\n",
        // Across it 4 m from its start.
        "0\nLINE\n8\nparcel\n10\n-499996\n20\n-1200001\n11\n-499996\n21\n-1199999\n",
        "0\nENDSEC\n0\nEOF\n",
    );
    std::fs::write(&path, site).unwrap();
    let (mut app, mut bridge) = setup();
    let answer = bridge
        .execute(&mut app, import_underlay(&path), false)
        .unwrap_or_else(|code| panic!("{code}: {}", app.action_digest()));
    let offset: [f64; 2] = serde_json::from_value(answer["underlay"]["offset_mm"].clone()).unwrap();
    // Built on the job's thread that read the drawing: there without waiting.
    assert!(app.underlay_snaps_built());
    // After the move the 10 m line starts here.
    let start = [-500_000_000.0 - offset[0], -1_200_000_000.0 - offset[1]];
    let snap = |point_mm: [f64; 2], radius_mm: Option<f64>| Request::UnderlaySnap {
        expected: None,
        point_mm,
        to_mm: None,
        radius_mm,
        underlay_id: None,
    };
    let digest = app.canonical_digest();

    let near_middle = [start[0] + 5_000.0, start[1] + 3.0];
    let found = bridge
        .execute(&mut app, snap(near_middle, None), false)
        .unwrap();
    assert_eq!(
        found["midpoint"]["point_mm"],
        json!([start[0] + 5_000.0, start[1]]),
        "{found}"
    );
    assert_eq!(
        found["midpoint"]["drawing_mm"],
        json!([-499_995_000.0, -1_200_000_000.0])
    );
    assert_eq!(found["midpoint"]["layer"], "parcel");
    assert_eq!(found["snaps_ready"], true);
    assert_eq!(found["nearest"]["distance_mm"], 3.0);
    assert_eq!(found["endpoint"], Value::Null, "the ends are 5 m away");
    assert_eq!(found["line"]["length_mm"], 10_000.0);
    assert_eq!(found["line"]["start_mm"], json!(start));

    let found = bridge
        .execute(
            &mut app,
            snap([start[0] - 4.0, start[1] + 3.0], None),
            false,
        )
        .unwrap();
    assert_eq!(found["endpoint"]["point_mm"], json!(start));
    assert_eq!(found["endpoint"]["distance_mm"], 5.0);

    // From near the crossing to near the start: the crossing and the start, 4 m apart.
    let cross = [start[0] + 4_000.0, start[1]];
    let measured = bridge
        .execute(
            &mut app,
            Request::UnderlaySnap {
                expected: None,
                point_mm: [cross[0] + 2.0, cross[1] + 1.0],
                to_mm: Some([start[0] - 4.0, start[1] + 3.0]),
                radius_mm: None,
                underlay_id: None,
            },
            false,
        )
        .unwrap();
    assert_eq!(
        measured["intersection"]["point_mm"],
        json!(cross),
        "{measured}"
    );
    assert_eq!(
        measured["intersection"]["drawing_mm"],
        json!([-499_996_000.0, -1_200_000_000.0])
    );
    assert_eq!(measured["measure"]["from"]["snap"], "intersection");
    assert_eq!(measured["measure"]["to"]["snap"], "endpoint");
    assert_eq!(measured["measure"]["to"]["point_mm"], json!(start));
    assert_eq!(measured["measure"]["distance_mm"], 4_000.0);
    assert_eq!(app.canonical_digest(), digest, "a snap changes nothing");

    let far = bridge
        .execute(&mut app, snap(near_middle, Some(2.0)), false)
        .unwrap();
    assert_eq!(far["nearest"], Value::Null);
    assert_eq!(far["line"], Value::Null);
    let refused = bridge.execute(&mut app, snap(near_middle, Some(-1.0)), false);
    assert_eq!(refused.unwrap_err(), "invalid_params");
    let missing = Request::UnderlaySnap {
        expected: None,
        point_mm: near_middle,
        to_mm: None,
        radius_mm: None,
        underlay_id: Some(99),
    };
    assert_eq!(
        bridge.execute(&mut app, missing, false).unwrap_err(),
        "entity_not_found"
    );

    let underlay = app.document.current().underlays().next().unwrap().clone();
    assert!(app.set_tag_visibility(underlay.layers[0].tag, false));
    let hidden = bridge
        .execute(&mut app, snap(near_middle, None), false)
        .unwrap();
    assert_eq!(hidden["nearest"], Value::Null);
}
