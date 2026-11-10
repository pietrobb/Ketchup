//! SketchUp files read in the program worker: the reader trusts sizes a file declares and
//! recurses over its nesting, so a hostile file costs that worker, not the caller, and a
//! normal file reads exactly as it does in process.
use ketchup_program::CancelToken;
use ketchup_scheduler::program_worker::{ProgramWorker, SkpConversionError, SkpRefusal};

fn worker() -> ProgramWorker {
    ProgramWorker::new(env!("CARGO_BIN_EXE_ketchup-program-worker")).unwrap()
}

fn sketchup_2025_file() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ketchup-skp/fixtures/SU_File.skp"
    ))
    .unwrap()
}

#[test]
fn a_skp_reads_in_the_worker_as_it_reads_in_process() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let skp = sketchup_2025_file();
    let local = ketchup_skp::skp_to_glb(&skp, usize::MAX).unwrap();
    let isolated = worker
        .convert_skp(&skp, usize::MAX, &CancelToken::default())
        .unwrap();
    assert_eq!(isolated.glb, local.glb);
    assert_eq!(isolated.version, local.version);
    assert_eq!(isolated.verified, local.verified);
    assert_eq!(isolated.left_out, local.left_out);
    let first = worker.process_id().unwrap();

    // A refusal is the reader's answer, with its cause from the worker; the worker stays.
    let error = worker
        .convert_skp(b"not a SketchUp file", usize::MAX, &CancelToken::default())
        .unwrap_err();
    let SkpConversionError::Refused(SkpRefusal::Reader(rejection)) = &error else {
        panic!("{error:?}");
    };
    assert_eq!(rejection.code(), "skp.unreadable");
    let local = ketchup_skp::skp_to_glb(b"not a SketchUp file", usize::MAX).unwrap_err();
    assert_eq!(rejection.causes()[0], local.to_string(), "{error:?}");
    assert_eq!(worker.process_id(), Some(first));

    let error = worker
        .convert_skp(&skp, 100, &CancelToken::default())
        .unwrap_err();
    assert!(
        matches!(
            error,
            SkpConversionError::Refused(SkpRefusal::GlbTooLarge { limit: 100, .. })
        ),
        "{error:?}"
    );
}
