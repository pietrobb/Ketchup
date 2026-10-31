//! DWG files read in the program worker: a file that declares a section of
//! hundreds of gigabytes ends that worker instead of the process that asked
//! (review SEC-5), and a normal file reads exactly as it does in process.
use ketchup_program::CancelToken;
use ketchup_scheduler::program_worker::{DwgConversionError, DwgRefusal, ProgramWorker};

/// A millimetre drawing of a line and a circle.
const DRAWING_DXF: &str = concat!(
    "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
    "0\nSECTION\n2\nENTITIES\n",
    "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n",
    "0\nCIRCLE\n8\n0\n10\n50\n20\n50\n40\n7.5\n",
    "0\nENDSEC\n0\nEOF\n",
);

fn worker() -> ProgramWorker {
    ProgramWorker::new(env!("CARGO_BIN_EXE_ketchup-program-worker")).unwrap()
}

fn drawing_dwg() -> Vec<u8> {
    ketchup_dwg::dxf_to_dwg(DRAWING_DXF.as_bytes()).unwrap()
}

#[test]
fn a_dwg_declaring_a_huge_section_ends_the_worker_not_the_caller() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let dwg = drawing_dwg();
    // Read in this process, the reader would abort it on the 300 GiB allocation.
    let hostile = ketchup_dwg::testing::with_declared_header_section(&dwg, 300 << 30);
    let error = worker
        .convert_dwg(&hostile, usize::MAX, &CancelToken::default())
        .unwrap_err();
    assert_eq!(error, DwgConversionError::MemoryLimit, "{error}");
    assert!(error.to_string().contains("2 GiB"), "{error}");
    assert_eq!(worker.process_id(), None);

    let read = worker
        .convert_dwg(&dwg, usize::MAX, &CancelToken::default())
        .unwrap();
    assert_eq!(read.version, ketchup_dwg::WRITTEN_VERSION);
}

#[test]
fn a_dwg_reads_in_the_worker_as_it_reads_in_process() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let dwg = drawing_dwg();
    let local = ketchup_dwg::dwg_to_dxf(&dwg, usize::MAX).unwrap();
    let isolated = worker
        .convert_dwg(&dwg, usize::MAX, &CancelToken::default())
        .unwrap();
    assert_eq!(isolated.dxf, local.dxf);
    assert_eq!(isolated.version, local.version);
    assert_eq!(isolated.skipped, local.skipped);
    let first = worker.process_id().unwrap();

    // A refusal is the reader's answer; the worker stays.
    let error = worker
        .convert_dwg(
            b"AC1032 and nothing else",
            usize::MAX,
            &CancelToken::default(),
        )
        .unwrap_err();
    assert!(
        matches!(
            error,
            DwgConversionError::Refused(DwgRefusal::NothingRead { .. })
        ),
        "{error:?}"
    );
    assert_eq!(worker.process_id(), Some(first));
}

/// The DXF of a drawing larger than one 16 MiB reply frame comes back whole: the
/// worker writes it to a file instead of the reply.
#[test]
fn a_dwg_whose_dxf_is_larger_than_a_reply_frame_comes_back_whole() {
    let _turn = crate::integration_support::file_turn();
    let mut dxf = String::from("0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n");
    dxf.push_str("0\nSECTION\n2\nENTITIES\n");
    for i in 0..LINES {
        dxf.push_str(&format!("0\nLINE\n8\n0\n10\n{i}\n20\n0\n11\n{i}\n21\n1\n"));
    }
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    let dwg = ketchup_dwg::dxf_to_dwg(dxf.as_bytes()).unwrap();
    let read = worker()
        .convert_dwg(&dwg, usize::MAX, &CancelToken::default())
        .unwrap();
    assert!(
        read.dxf.len() > ketchup_scheduler::protocol::MAX_REPLY_FRAME_BYTES,
        "{} bytes",
        read.dxf.len()
    );
    assert_eq!(
        read.dxf,
        ketchup_dwg::dwg_to_dxf(&dwg, usize::MAX).unwrap().dxf
    );
}

/// About 139 bytes of DXF each: 20 MiB of DXF.
const LINES: usize = 150_000;

/// The pairs of a DXF text, code and value trimmed.
fn pairs(dxf: &str) -> Vec<(&str, &str)> {
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    lines
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

/// Whether a record `kind` named `name` is in the DXF pairs: a table entry or an object.
fn has_record(pairs: &[(&str, &str)], kind: &str, name: &str) -> bool {
    let mut record = "";
    pairs.iter().any(|&(code, value)| {
        if code == "0" {
            record = value;
        }
        record == kind && code == "2" && value == name
    })
}

/// What a drawing written back from [`ketchup_dwg::testing::styled_drawing_dwg`] keeps of it.
fn assert_whole_drawing(dxf: &str) {
    let pairs = pairs(dxf);
    for (kind, name) in [
        ("STYLE", "ARIAL"),
        ("DIMSTYLE", "MYDIM"),
        ("APPID", "MYAPP"),
    ] {
        assert!(has_record(&pairs, kind, name), "{kind} {name} is kept");
    }
    assert!(
        pairs
            .windows(2)
            .any(|two| two == [("1001", "MYAPP"), ("1000", "wall-1")]),
        "the line's extended data is kept"
    );
    assert!(
        pairs.contains(&("0", "LAYOUT")),
        "the objects section keeps the layouts"
    );
    assert!(pairs.contains(&("3", "WALL1")), "the group is kept");
    assert_references_resolve(&pairs);
}

/// The members (340) of the drawing's groups.
fn group_members<'a>(pairs: &[(&str, &'a str)]) -> Vec<&'a str> {
    let mut record = "";
    pairs
        .iter()
        .filter_map(|&(code, value)| {
            if code == "0" {
                record = value;
            }
            (record == "GROUP" && code == "340").then_some(value)
        })
        .collect()
}

/// Every owner, reactor and member reference (330, 340, 360) names a record of the drawing.
fn assert_references_resolve(pairs: &[(&str, &str)]) {
    let handles: std::collections::BTreeSet<&str> = pairs
        .iter()
        .filter(|(code, _)| matches!(*code, "5" | "105"))
        .map(|&(_, value)| value)
        .collect();
    for &(code, value) in pairs {
        if matches!(code, "330" | "340" | "360") && value != "0" {
            assert!(handles.contains(value), "{code} {value} names no record");
        }
    }
}

/// Review 2026-10-29 CD-60: a drawing imported from DWG is written back whole, as DXF and as
/// DWG: its text and dimension styles, applications with their extended data and the
/// objects the import does not read stay, not only the layers and linetypes.
#[test]
fn a_dwg_drawing_written_back_keeps_its_styles_extended_data_and_objects() {
    use ketchup_model::import::ImportLengthUnit;
    use ketchup_scheduler::protocol::{UnderlayInput, UnderlayWriteInput};

    let _turn = crate::integration_support::file_turn();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("plan.dwg");
    std::fs::write(&source, ketchup_dwg::testing::styled_drawing_dwg()).unwrap();
    let lines = directory.path().join("lines.bin");
    let mut worker = worker();
    let layers = worker
        .read_underlay(
            UnderlayInput {
                source: source.clone(),
                dwg: true,
                unit_if_unitless: Some(ImportLengthUnit::Millimetre),
                geo_origin_mm: None,
                output: lines.clone(),
            },
            &CancelToken::default(),
        )
        .unwrap()
        .layers
        .len();
    let mut write = |output: &std::path::Path| {
        worker
            .write_underlay(
                UnderlayWriteInput {
                    source: source.clone(),
                    dwg: true,
                    unit_if_unitless: Some(ImportLengthUnit::Millimetre),
                    geo_origin_mm: None,
                    lines: lines.clone(),
                    layers,
                    edits: ketchup_model::underlay::edits::UnderlayEdits::default(),
                    output: output.to_owned(),
                },
                std::time::Duration::from_secs(120),
                &CancelToken::default(),
            )
            .unwrap()
    };

    let dxf = directory.path().join("written.dxf");
    write(&dxf);
    assert_whole_drawing(&std::fs::read_to_string(&dxf).unwrap());

    let dwg = directory.path().join("written.dwg");
    write(&dwg);
    let back = ketchup_dwg::dwg_to_whole_dxf(&std::fs::read(&dwg).unwrap(), usize::MAX).unwrap();
    assert_whole_drawing(&String::from_utf8(back.dxf).unwrap());
}

/// Review 2026-10-29 CD-60, its probe: a line of group `WALL1` deleted in Kečup leaves the
/// group with the other line, and no reference in the drawing names the deleted one.
#[test]
fn a_line_deleted_from_a_dwg_group_leaves_the_group_and_no_dangling_reference() {
    use ketchup_model::import::ImportLengthUnit;
    use ketchup_scheduler::protocol::{UnderlayInput, UnderlayWriteInput};

    let _turn = crate::integration_support::file_turn();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("plan.dwg");
    std::fs::write(&source, ketchup_dwg::testing::styled_drawing_dwg()).unwrap();
    let lines = directory.path().join("lines.bin");
    let mut worker = worker();
    let layers = worker
        .read_underlay(
            UnderlayInput {
                source: source.clone(),
                dwg: true,
                unit_if_unitless: Some(ImportLengthUnit::Millimetre),
                geo_origin_mm: None,
                output: lines.clone(),
            },
            &CancelToken::default(),
        )
        .unwrap()
        .layers
        .len();
    let mut edits = ketchup_model::underlay::edits::UnderlayEdits::default();
    edits.removed_objects.insert(0);
    let mut write = |output: &std::path::Path| {
        worker
            .write_underlay(
                UnderlayWriteInput {
                    source: source.clone(),
                    dwg: true,
                    unit_if_unitless: Some(ImportLengthUnit::Millimetre),
                    geo_origin_mm: None,
                    lines: lines.clone(),
                    layers,
                    edits: edits.clone(),
                    output: output.to_owned(),
                },
                std::time::Duration::from_secs(120),
                &CancelToken::default(),
            )
            .unwrap()
    };
    let assert_one_member = |dxf: &str| {
        let pairs = pairs(dxf);
        assert!(
            !pairs.contains(&("1000", "wall-1")),
            "the first line is deleted"
        );
        assert!(pairs.contains(&("3", "WALL1")), "the group stays");
        assert_eq!(
            group_members(&pairs).len(),
            1,
            "{:?}",
            group_members(&pairs)
        );
        assert_references_resolve(&pairs);
    };

    let dxf = directory.path().join("written.dxf");
    write(&dxf);
    assert_one_member(&std::fs::read_to_string(&dxf).unwrap());

    let dwg = directory.path().join("written.dwg");
    write(&dwg);
    let back = ketchup_dwg::dwg_to_whole_dxf(&std::fs::read(&dwg).unwrap(), usize::MAX).unwrap();
    assert_one_member(&String::from_utf8(back.dxf).unwrap());
}

/// Review 2026-10-29 CD-71: a drawing with an application's own entity, which a DWG cannot
/// hold without data of that application, is written as DWG without it and says so; before,
/// that one entity refused the whole export.
#[test]
fn a_drawing_with_a_custom_entity_is_written_as_dwg_without_it_and_says_so() {
    use ketchup_model::import::ImportLengthUnit;
    use ketchup_scheduler::protocol::{UnderlayInput, UnderlayWriteInput};

    let _turn = crate::integration_support::file_turn();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("plan.dxf");
    let custom = DRAWING_DXF.replace(
        "0\nENDSEC\n0\nEOF",
        "0\nACME_WIDGET\n8\n0\n100\nAcmeWidget\n10\n1\n20\n2\n30\n0\n0\nENDSEC\n0\nEOF",
    );
    std::fs::write(&source, custom).unwrap();
    let lines = directory.path().join("lines.bin");
    let mut worker = worker();
    let layers = worker
        .read_underlay(
            UnderlayInput {
                source: source.clone(),
                dwg: false,
                unit_if_unitless: Some(ImportLengthUnit::Millimetre),
                geo_origin_mm: None,
                output: lines.clone(),
            },
            &CancelToken::default(),
        )
        .unwrap()
        .layers
        .len();
    let output = directory.path().join("written.dwg");
    let written = worker
        .write_underlay(
            UnderlayWriteInput {
                source,
                dwg: false,
                unit_if_unitless: Some(ImportLengthUnit::Millimetre),
                geo_origin_mm: None,
                lines,
                layers,
                edits: ketchup_model::underlay::edits::UnderlayEdits::default(),
                output: output.clone(),
            },
            std::time::Duration::from_secs(120),
            &CancelToken::default(),
        )
        .unwrap();
    assert_eq!(written.skipped, [(String::from("ACME_WIDGET"), 1)]);
    let back = ketchup_dwg::dwg_to_dxf(&std::fs::read(&output).unwrap(), usize::MAX).unwrap();
    let text = String::from_utf8(back.dxf).unwrap();
    assert!(text.contains("LINE") && text.contains("CIRCLE"), "{text}");
}
