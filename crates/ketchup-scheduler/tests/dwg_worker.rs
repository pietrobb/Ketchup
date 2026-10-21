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
        .convert_dwg(&hostile, &CancelToken::default())
        .unwrap_err();
    assert_eq!(error, DwgConversionError::MemoryLimit, "{error}");
    assert!(error.to_string().contains("2 GiB"), "{error}");
    assert_eq!(worker.process_id(), None);

    let read = worker.convert_dwg(&dwg, &CancelToken::default()).unwrap();
    assert_eq!(read.version, ketchup_dwg::WRITTEN_VERSION);
}

#[test]
fn a_dwg_reads_in_the_worker_as_it_reads_in_process() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let dwg = drawing_dwg();
    let local = ketchup_dwg::dwg_to_dxf(&dwg, usize::MAX).unwrap();
    let isolated = worker.convert_dwg(&dwg, &CancelToken::default()).unwrap();
    assert_eq!(isolated.dxf, local.dxf);
    assert_eq!(isolated.version, local.version);
    assert_eq!(isolated.skipped, local.skipped);
    let first = worker.process_id().unwrap();

    // A refusal is the reader's answer; the worker stays.
    let error = worker
        .convert_dwg(b"AC1032 and nothing else", &CancelToken::default())
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
    let read = worker().convert_dwg(&dwg, &CancelToken::default()).unwrap();
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
