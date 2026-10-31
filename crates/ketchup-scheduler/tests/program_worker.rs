//! Programs evaluated and checked in the program worker: the review's runaway
//! programs and models end with an error instead of taking the process down or
//! running for minutes, and a normal program and its report come back bit for
//! bit as in process.
use ketchup_program::{CancelToken, ExactPair, ExactShapes, evaluate, refine, report};
use ketchup_scheduler::program_worker::{CheckDeadline, ProgramWorker};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

const BOX: &str = "box(\"a\", (10, 20, 30))\n";

fn checks() -> CheckDeadline {
    CheckDeadline::start()
}

fn worker() -> ProgramWorker {
    ProgramWorker::new(env!("CARGO_BIN_EXE_ketchup-program-worker")).unwrap()
}

fn run(
    worker: &mut ProgramWorker,
    source: &str,
) -> Result<ketchup_program::Evaluated, ketchup_program::ProgramError> {
    worker.evaluate(
        "test.star",
        source,
        &BTreeMap::new(),
        &CancelToken::default(),
    )
}

#[test]
fn memory_past_the_limit_ends_the_worker_not_the_caller() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    // `str.replace` builds its result outside the interpreter's heap limit (review PRG-M1).
    let error = run(&mut worker, "s = \"ab\" * 50000\nt = s.replace(\"\", s)\n").unwrap_err();
    assert_eq!(error.code, "memory_limit", "{error}");
    assert_eq!(run(&mut worker, BOX).unwrap().model.parts.len(), 1);
}

/// Unix has no job object: the worker's memory limit is its own data limit,
/// the private writable memory it maps, on Linux (review 2026-10-15 PRG-1,
/// PRG-3, PRG-6; RLIMIT_DATA instead of RLIMIT_AS since review 2026-10-26 EX-40).
#[cfg(target_os = "linux")]
#[test]
fn the_worker_runs_under_its_data_limit_on_linux() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    run(&mut worker, BOX).unwrap();
    let pid = worker.process_id().unwrap();
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
    let data = limits
        .lines()
        .find(|line| line.starts_with("Max data size"))
        .unwrap();
    let bytes = ketchup_scheduler::program_worker::PROGRAM_WORKER_MEMORY_BYTES.to_string();
    assert!(data.contains(&bytes), "{data}");
}

#[test]
fn a_builtin_loop_past_the_deadline_is_killed() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker().with_deadline(Duration::from_secs(2));
    // One builtin call: the interpreter never reaches its next clock check (review PRG-M2).
    let error = run(&mut worker, "x = max(range(2147483647))\n").unwrap_err();
    assert_eq!(error.code, "time_limit", "{error}");
    assert!(error.message.contains("longer than 2 s"), "{error}");
    assert_eq!(worker.process_id(), None);
    assert_eq!(run(&mut worker, BOX).unwrap().model.parts.len(), 1);
}

#[test]
fn cancel_kills_a_running_program_at_once() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    run(&mut worker, BOX).unwrap();
    let first = worker.process_id().unwrap();
    let cancel = CancelToken::default();
    let (sender, outcome) = std::sync::mpsc::channel();
    let evaluation = {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            let result = worker.evaluate(
                "spin.star",
                "x = max(range(2147483647))\n",
                &BTreeMap::new(),
                &cancel,
            );
            sender.send(result).unwrap();
            worker
        })
    };
    // Still running: the loop takes far longer than this.
    assert!(outcome.recv_timeout(Duration::from_millis(300)).is_err());
    cancel.cancel();
    // Cancelled, not stopped at the worker's own deadline later.
    let error = outcome.recv().unwrap().unwrap_err();
    assert_eq!(error.code, "cancelled", "{error}");
    let mut worker = evaluation.join().unwrap();
    assert_eq!(run(&mut worker, BOX).unwrap().model.parts.len(), 1);
    assert_ne!(worker.process_id(), Some(first));
}

#[test]
fn the_worker_stays_between_programs_and_is_replaced_after_a_kill() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker().with_deadline(Duration::from_secs(1));
    run(&mut worker, BOX).unwrap();
    let first = worker.process_id().unwrap();
    // A program error is the program's answer, not the worker's death.
    assert_eq!(run(&mut worker, "box(").unwrap_err().code, "syntax_error");
    assert_eq!(
        run(&mut worker, "fail(\"no\")\n").unwrap_err().code,
        "evaluation_error"
    );
    run(&mut worker, BOX).unwrap();
    assert_eq!(worker.process_id(), Some(first));
    assert_eq!(
        run(&mut worker, "x = max(range(2147483647))\n")
            .unwrap_err()
            .code,
        "time_limit"
    );
    assert_eq!(worker.process_id(), None);
    run(&mut worker, BOX).unwrap();
    assert_ne!(worker.process_id(), Some(first));
}

/// Checking 400 boxes that all overlap takes over a minute; the worker is
/// stopped at its deadline and the next request starts a new one (review PRG-1).
#[test]
fn a_report_past_the_deadline_is_killed() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let evaluated = run(
        &mut worker,
        "for i in range(400):\n    box(\"b%d\" % i, (100, 100, 100), at=(0, i * 0.01, 0))\n",
    )
    .unwrap();
    let deadline = CheckDeadline::after(Duration::from_secs(2));
    let error = worker
        .report(&evaluated, deadline, &CancelToken::default())
        .unwrap_err();
    assert_eq!(error.code, "time_limit", "{error}");
    assert!(
        error.message.starts_with("checking the program's model")
            && error.message.contains("longer than 2 s"),
        "{error}"
    );
    assert_eq!(worker.process_id(), None);
    let small = run(&mut worker, BOX).unwrap();
    assert!(
        worker
            .report(&small, checks(), &CancelToken::default())
            .unwrap()
            .ok
    );
}

/// Refining a report with exact answers repeats its checks, so it is stopped
/// at the same deadline instead of running in the caller (review 2026-10-15 N-4).
#[test]
fn a_refine_past_the_deadline_is_killed() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let evaluated = run(
        &mut worker,
        "for i in range(400):\n    box(\"b%d\" % i, (100, 100, 100), at=(0, i * 0.01, 0))\n",
    )
    .unwrap();
    let deadline = CheckDeadline::after(Duration::from_secs(2));
    let error = worker
        .refine(
            &evaluated.model,
            &ExactShapes::default(),
            deadline,
            &CancelToken::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, "time_limit", "{error}");
    assert!(
        error.message.starts_with("checking the program's model")
            && error.message.contains("longer than 2 s"),
        "{error}"
    );
    assert_eq!(worker.process_id(), None);
    let small = run(&mut worker, BOX).unwrap();
    let refined = worker
        .refine(
            &small.model,
            &ExactShapes::default(),
            checks(),
            &CancelToken::default(),
        )
        .unwrap();
    assert_eq!(refined, refine(&small.model, &ExactShapes::default()));
}

/// Evaluation, report and refine of one program share one deadline: a model
/// that is cheap to evaluate but costly to check stops at one predictable time,
/// not after a fresh deadline for every step (review 2026-10-15 S3). Once the
/// report has used it up, later steps are refused before a worker starts.
#[test]
fn evaluation_report_and_refine_share_one_deadline() {
    let _turn = crate::integration_support::file_turn();
    // Checking 400 boxes that all overlap takes over a minute.
    let source =
        "for i in range(400):\n    box(\"b%d\" % i, (100, 100, 100), at=(0, i * 0.01, 0))\n";
    let evaluated = evaluate("test.star", source, &BTreeMap::new()).unwrap();
    let mut worker = worker();
    let cancel = CancelToken::default();
    let deadline = CheckDeadline::after(Duration::from_secs(3));
    let error = worker
        .run("test.star", source, &BTreeMap::new(), deadline, &cancel)
        .expect_err("the report stops at the deadline of the evaluation");
    let stopped = |error: &ketchup_program::ProgramError| {
        assert_eq!(error.code, "time_limit", "{error}");
        assert!(
            error.message.starts_with("checking the program's model")
                && error.message.contains("longer than 3 s"),
            "{error}"
        );
    };
    stopped(&error);
    assert_eq!(worker.process_id(), None);
    // A fresh deadline per step would start a worker and check for a minute.
    let error = worker
        .refine(&evaluated.model, &ExactShapes::default(), deadline, &cancel)
        .unwrap_err();
    stopped(&error);
    assert_eq!(worker.process_id(), None);
    let error = worker.report(&evaluated, deadline, &cancel).unwrap_err();
    stopped(&error);
    assert_eq!(worker.process_id(), None);
}

const MOTION: &str = r#"
p=param("position",40)
a=box("anchor",(10,10,10),at=(-100,0,0))
b=box("b",(10,20,30),at=(110,20,30))
hole(b,"z+",at=(4,5),diameter=2,depth=6)
c=component("child",[b])
i=instance("inner",c,at=(100,0,0),x=(0,1,0))
g=component("parent",[c,i])
j=instance("copy",g,at=(400,0,0))
joint(a,g,kind="motion",motion=slide((0,2,0),0,100),position=p)
rotate(a,axis=(0,0,1),angle=0.1)
check(False,"anchor too close",parts=[a],hint="move it",severity="warning")
print("done")
"#;

/// The two house programs are the large-house documents (AGENTS.md Â§6).
const LARGE: [&str; 2] = ["house-project.star", "tiny-house.star"];

fn example_programs(large: bool) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut programs = Vec::new();
    for directory in ["examples/programs", "crates/ketchup-app/tests/fixtures"] {
        for entry in std::fs::read_dir(root.join(directory)).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(".star") && LARGE.contains(&name.as_str()) == large {
                programs.push((name.clone(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    programs
}

#[test]
fn every_example_comes_back_bit_for_bit() {
    let _turn = crate::integration_support::file_turn();
    let mut programs = example_programs(false);
    assert!(programs.len() >= 7);
    programs.push(("motion.star".to_owned(), MOTION.to_owned()));
    assert_round_trip(programs);
}

#[test]
#[ignore = "large-house: runs in full suite"]
fn large_house_programs_come_back_bit_for_bit() {
    let _turn = crate::integration_support::file_turn();
    let programs = example_programs(true);
    assert_eq!(programs.len(), LARGE.len());
    assert_round_trip(programs);
}

fn assert_round_trip(programs: Vec<(String, String)>) {
    let overrides = BTreeMap::from([("not_a_param".to_owned(), 1.0)]);
    let mut worker = worker();
    for (name, source) in programs {
        let local = evaluate(&name, &source, &overrides);
        let isolated = worker.evaluate(&name, &source, &overrides, &CancelToken::default());
        // Debug spells out -0.0 and NaN, which PartialEq would let through.
        assert_eq!(format!("{isolated:?}"), format!("{local:?}"), "{name}");
        let local = local.unwrap_or_else(|error| panic!("{name}: {error}"));
        let checked = worker.report(&local, checks(), &CancelToken::default());
        assert_eq!(
            format!("{checked:?}"),
            format!("{:?}", Ok::<_, ()>(report(&local))),
            "{name}"
        );
        // The first two parts touch on their solids, whatever their boxes say.
        let mut exact = ExactShapes::default();
        if let [a, b, ..] = local.model.parts.as_slice() {
            let touching = ExactPair {
                distance_mm: Some(0.0),
                ..ExactPair::default()
            };
            exact.insert(&a.name, &b.name, touching);
        }
        let refined = worker.refine(&local.model, &exact, checks(), &CancelToken::default());
        assert_eq!(
            format!("{refined:?}"),
            format!("{:?}", Ok::<_, ()>(refine(&local.model, &exact))),
            "{name}"
        );
    }
}
