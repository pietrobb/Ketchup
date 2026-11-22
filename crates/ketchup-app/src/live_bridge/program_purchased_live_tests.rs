//! Live checks on Hettich STEP files that are not in the repository. Every
//! test is ignored and returns at once unless `KETCHUP_HETTICH_DIR` names the
//! folder that holds the `hettich` subfolder; exports go to the temp folder.
use super::*;
use ketchup_model::document::FeatureKind;

fn hettich_dir() -> Option<String> {
    std::env::var("KETCHUP_HETTICH_DIR")
        .ok()
        .filter(|dir| !dir.is_empty())
}

fn skipped() -> bool {
    let skipped = hettich_dir().is_none();
    if skipped {
        eprintln!("KETCHUP_HETTICH_DIR is not set; skipped");
    }
    skipped
}

fn source(file: &str, part: Option<&str>) -> String {
    let part = part.map_or(String::new(), |p| format!("part = '{p}', "));
    format!(
        "door_h = param('door_h', 2000)\n\
         n = 5\n\
         for i in range(n):\n\
         \x20   z = 100 + i * (door_h - 200) / (n - 1)\n\
         \x20   purchased('hinge %d' % i, '{file}', '9237903', 'Hettich', {part}\n\
         \x20             at = (18, 0, z), z = (-1, 0, 0), x = (0, 0, 1))\n\
         board('door', (600, 18, door_h))\n"
    )
}

fn apply(source: &str, folder: bool, door_h: f64) -> Request {
    Request::ApplyProgram {
        expected: None,
        source: source.to_owned(),
        overrides: Some(BTreeMap::from([("door_h".to_owned(), door_h)])),
        file_name: Some("predsien_purchased.star".to_owned()),
        replace_document: true,
        source_folder: if folder { hettich_dir() } else { None },
    }
}

fn hinges(app: &KetchupApp) -> Vec<(String, f64, [[f64; 3]; 2])> {
    let snapshot = app.document.current();
    let mut out: Vec<_> = snapshot
        .occurrences()
        .filter(|o| o.name().starts_with("hinge"))
        .map(|o| {
            let d = snapshot.definition(o.definition_id()).unwrap();
            let f = snapshot.feature(d.feature_ids()[0]).unwrap();
            let FeatureKind::ImportedExactBody(spec) = f.kind() else {
                panic!("not imported")
            };
            (
                o.name().to_owned(),
                o.transform().matrix()[11],
                spec.bounds_mm,
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn collisions(applied: &Value) -> Vec<String> {
    applied["report"]["issues"]
        .as_array()
        .map(|issues| {
            issues
                .iter()
                .filter(|issue| issue["kind"] == "collision")
                .map(|issue| issue["parts"].to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn settled(wire: &mut Wire, result: &Value) -> (Value, Vec<Value>) {
    if result["exact_collisions"]["not_evaluated"][0]["reason"] != "exact_check_running" {
        let issues = result["report"]["issues"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        return (result["exact_collisions"].clone(), issues);
    }
    let (mut issues, mut offset) = (Vec::new(), 0);
    loop {
        let expected = wire.app.live_bridge_stamp();
        let response = wire.answer_within(
            Request::ProgramReport {
                expected,
                section: ReportSection::Issues,
                offset,
                limit: 50,
            },
            Duration::from_secs(600),
        );
        if response.error.as_deref() == Some("busy") {
            continue;
        }
        let page = response.result.unwrap();
        issues.extend(page["rows"].as_array().unwrap().iter().cloned());
        match page["next_offset"].as_u64() {
            Some(next) => offset = usize::try_from(next).unwrap(),
            None => return (page["exact_collisions"].clone(), issues),
        }
    }
}

fn run(tag: &str, file: &str, part: Option<&str>) {
    if skipped() {
        return;
    }
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|p| p.is_file())
        .unwrap();
    let mut app = KetchupApp::new();
    app.connect_exact_worker(worker).unwrap();
    app.selection.clear();
    let mut wire = Wire::connect(app);
    let source = source(file, part);
    let t = Instant::now();
    let first = wire.answer_within(apply(&source, true, 2000.0), Duration::from_secs(600));
    eprintln!("[{tag}] first apply {:?}: ok={}", t.elapsed(), first.ok);
    if !first.ok {
        eprintln!("[{tag}] FULL {first:#?}");
    }
    let Some(first) = first.result.filter(|_| first.ok) else {
        eprintln!("[{tag}] ERROR {:#?}", first.error);
        return;
    };
    for h in hinges(&wire.app) {
        eprintln!("[{tag}] {} z={} bounds={:?}", h.0, h.1, h.2);
    }
    eprintln!(
        "[{tag}] receipts={} program_kept={} geometry_evaluated={} keys={:?}",
        wire.app.document.current().import_receipts().count(),
        wire.app.document.current_rule_program().is_some(),
        first["geometry_evaluated"],
        first
            .as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>())
    );
    let t = Instant::now();
    let (exact, issues) = settled(&mut wire, &first);
    let pairs: Vec<String> = issues
        .iter()
        .filter(|issue| issue["kind"] == "collision")
        .map(|issue| issue["parts"].to_string())
        .collect();
    eprintln!(
        "[{tag}] exact check settled {:?}: {exact} collisions={pairs:?} (answer had {:?})",
        t.elapsed(),
        collisions(&first)
    );
    let t = Instant::now();
    let moved = wire.answer_within(apply(&source, false, 2300.0), Duration::from_secs(600));
    eprintln!("[{tag}] door 2300 apply {:?} ok={}", t.elapsed(), moved.ok);
    for h in hinges(&wire.app) {
        eprintln!("[{tag}] {} z={}", h.0, h.1);
    }
    let model = ketchup_program::run(
        "predsien_purchased.star",
        &source,
        &BTreeMap::from([("door_h".to_owned(), 2300.0)]),
    )
    .unwrap()
    .0
    .model;
    let bom = ketchup_program::bom::bom(&model);
    for row in &bom.hardware {
        eprintln!("[{tag}] hardware {} x{}", row.item, row.count);
    }
    eprintln!(
        "[{tag}] cut list {:?}",
        bom.cut_list
            .iter()
            .flat_map(|r| &r.parts)
            .collect::<Vec<_>>()
    );
    crate::drawn_shape::tests::evaluate_exact(&mut wire.app);
    let out = std::env::temp_dir().join("hettich_live");
    std::fs::create_dir_all(&out).unwrap();
    for (format, extension) in [("step", "step"), ("glb", "glb")] {
        let target = out.join(format!("{tag}.{extension}"));
        let t = Instant::now();
        let exported = wire.answer_within(
            Request::Export(crate::live_bridge::interchange::ExportRequest {
                expected: None,
                path: target.display().to_string(),
                export_format: format.to_owned(),
                drawing_scale: None,
                image: None,
                cam_dialect: None,
                cam_operations: None,
                overwrite: true,
            }),
            Duration::from_secs(600),
        );
        let bytes = std::fs::read(&target).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        let named = ["hinge 0", "hinge 4", "door"]
            .iter()
            .filter(|part| text.contains(*part))
            .count();
        eprintln!(
            "[{tag}] export {format} ok={} {:?} bytes={} names={named}/3",
            exported.ok,
            t.elapsed(),
            bytes.len()
        );
    }
}

/// The predsien case: 5 Sensys hinges of the konfig file and 17 more parts of
/// the wingline file, 22 purchased() in one apply.
#[test]
#[ignore]
fn live_hettich_hall_22() {
    if skipped() {
        return;
    }
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|p| p.is_file())
        .unwrap();
    let (mut app, mut bridge) = setup();
    app.connect_exact_worker(worker).unwrap();
    let wingline = "hettich/wingline_l_A_osadene_v6_fix.stp";
    // The names of the wingline file's parts, from the error a missing one gives.
    let probe = format!(
        "door_h = param('door_h', 2000)\npurchased('x', '{wingline}', '1', 'Hettich', part = 'no such part')\n"
    );
    let started = Instant::now();
    assert!(
        bridge
            .execute(&mut app, apply(&probe, true, 2000.0), false)
            .is_err()
    );
    let reason = take_error_details().unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_owned();
    eprintln!("[hall] probe {:?}", started.elapsed());
    let listed = reason.split("its parts are ").nth(1).unwrap();
    let names: Vec<String> = listed
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|name| *name != "SOLID" && !name.contains("Sensys"))
        .map(str::to_owned)
        .collect();
    // The error lists 20 names; two parts are placed twice to make 17.
    let names: Vec<String> = names.iter().cycle().take(17).cloned().collect();
    // The probe's read does not count: the apply starts in a window that read nothing.
    bridge.program_folder = Default::default();
    eprintln!("[hall] {} wingline parts: {names:?}", names.len());
    let mut source = String::from(
        "door_h = param('door_h', 2000)\n\
         for i in range(5):\n\
         \x20   purchased('sensys %d' % i, 'hettich/konfig/9339516_9282527_9339481++9339507+9278742.stp',\n\
         \x20             '9073605', 'Hettich', part = '9073605_Sensys_hinge',\n\
         \x20             at = (18, 0, 100 + i * (door_h - 200) / 4), z = (-1, 0, 0), x = (0, 0, 1))\n",
    );
    for (i, name) in names.iter().enumerate() {
        source.push_str(&format!(
            "purchased('wl {i}', '{wingline}', '{i}', 'Hettich', part = {name:?}, at = (0, 0, door_h - 2000))\n"
        ));
    }
    source.push_str("board('door', (600, 18, door_h))\n");
    let reads = |bridge: &LiveBridge| bridge.program_folder.worker_reads();
    let before = reads(&bridge);
    let timed = |app: &mut KetchupApp, bridge: &mut LiveBridge, tag: &str, door_h: f64| {
        let started = Instant::now();
        let result = bridge.execute(app, apply(&source, true, door_h), false);
        let elapsed = started.elapsed();
        if result.is_err() {
            eprintln!("[hall] {tag} ERROR {:#}", take_error_details().unwrap());
        }
        let parts = app
            .document
            .current()
            .occurrences()
            .filter(|o| o.name().starts_with("sensys") || o.name().starts_with("wl "))
            .count();
        eprintln!(
            "[hall] {tag} door_h={door_h}: {elapsed:?}, bought parts {parts}, worker reads so far {}",
            reads(bridge) - before
        );
    };
    timed(&mut app, &mut bridge, "first apply", 2000.0);
    timed(&mut app, &mut bridge, "re-apply", 2300.0);
    timed(&mut app, &mut bridge, "re-apply", 2100.0);
    bridge.program_folder = Default::default();
    let before = reads(&bridge);
    let started = Instant::now();
    assert!(
        bridge
            .execute(&mut app, apply(&source, true, 2200.0), false)
            .is_ok()
    );
    eprintln!(
        "[hall] fresh window re-apply: {:?}, worker reads {}",
        started.elapsed(),
        reads(&bridge) - before
    );
}

/// 22 Sensys hinges of the konfig file by part=, the same program as the
/// before-cache probe on c5196661.
#[test]
#[ignore]
fn live_hettich_22_sensys() {
    if skipped() {
        return;
    }
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|p| p.is_file())
        .unwrap();
    let (mut app, mut bridge) = setup();
    app.connect_exact_worker(worker).unwrap();
    let source = "door_h = param('door_h', 2000)\n\
         for i in range(22):\n\
         \x20   purchased('sensys %d' % i, 'hettich/konfig/9339516_9282527_9339481++9339507+9278742.stp',\n\
         \x20             '9073605', 'Hettich', part = '9073605_Sensys_hinge',\n\
         \x20             at = (18, 0, 100 + i * (door_h - 200) / 21), z = (-1, 0, 0), x = (0, 0, 1))\n\
         board('door', (600, 18, door_h))\n";
    for (tag, door_h) in [
        ("first apply", 2000.0),
        ("re-apply", 2300.0),
        ("re-apply", 2100.0),
    ] {
        let before = bridge.program_folder.worker_reads();
        let started = Instant::now();
        let result = bridge.execute(&mut app, apply(source, true, door_h), false);
        let elapsed = started.elapsed();
        if result.is_err() {
            eprintln!("[after] {tag} ERROR {:#}", take_error_details().unwrap());
        }
        let parts = app
            .document
            .current()
            .occurrences()
            .filter(|o| o.name().starts_with("sensys"))
            .count();
        eprintln!(
            "[after] {tag} door_h={door_h}: {elapsed:?}, bought parts {parts}, reads {}",
            bridge.program_folder.worker_reads() - before
        );
    }
}

#[test]
#[ignore]
fn live_hettich_set() {
    run("set", "hettich/9237903.stp", None);
}

#[test]
#[ignore]
fn live_hettich_konfig() {
    run(
        "konfig",
        "hettich/konfig/9339516_9282527_9339481++9339507+9278742.stp",
        Some("9073605_Sensys_hinge"),
    );
}

#[test]
#[ignore]
fn live_hettich_push_to_move() {
    run(
        "v6",
        "hettich/wingline_l_A_osadene_v6.stp",
        Some("Hettich 9329888 Push to move jednotka"),
    );
}

#[test]
#[ignore]
fn live_hettich_push_to_move_fix() {
    run(
        "v6fix",
        "hettich/wingline_l_A_osadene_v6_fix.stp",
        Some("Hettich 9329888 Push to move jednotka"),
    );
}
