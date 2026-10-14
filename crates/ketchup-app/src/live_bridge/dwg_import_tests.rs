//! `file action=import` of a `.dwg` drawing: it imports as the profiles of its
//! DXF conversion in one Undo step, the review names the DWG version and what
//! the DWG reader left out, and a file that is no DWG or is too large is
//! refused without changing the document.

use super::*;
use crate::app::dwg_import::LAYERED_DRAWING_DXF;
use ketchup_model::import::ImportFormat;

fn import(path: &Path) -> Request {
    Request::Import {
        expected: None,
        path: path.display().to_string(),
        unit: None,
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
fn a_file_that_is_no_dwg_or_too_large_is_refused_and_changes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let garbage = directory.path().join("garbage.dwg");
    std::fs::write(&garbage, b"AC1032 but no drawing follows").unwrap();
    let large = directory.path().join("large.dwg");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(u64::try_from(ketchup_dwg::MAX_DWG_SOURCE_BYTES).unwrap() + 1)
        .unwrap();
    let (mut app, mut bridge) = setup();
    let (digest, undo) = (app.canonical_digest(), app.undo_step_count());

    for (path, reason) in [(&garbage, "DWG file"), (&large, "exceeds the bounded")] {
        assert_eq!(
            bridge.execute(&mut app, import(path), false),
            Err("import_failed"),
            "{}",
            path.display()
        );
        let error = app
            .import_file(ImportFormat::Dxf, path, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains(reason), "{}: {error}", path.display());
        assert_eq!(app.canonical_digest(), digest);
        assert_eq!(app.undo_step_count(), undo);
    }
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
