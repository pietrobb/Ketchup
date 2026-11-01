//! Review 2026-10-31 MCP-80: a bridge handler that runs a worker with a token nobody keeps
//! cannot be cancelled and holds the window for the worker's whole budget. Every
//! `CancelToken::default()` outside tests is either kept by the job it starts (cloned for
//! the worker on the next line) or sits on a path the socket never takes: the window
//! queues those requests as jobs, and only the in-process `execute` reaches them.

use std::path::Path;

/// (file, function, request the socket queues instead).
const ONLY_IN_PROCESS: &[(&str, &str, &str)] = &[
    ("interchange.rs", "import_file", "dwg_import::imports_dwg"),
    (
        "program_access.rs",
        "program_report_page",
        "Request::ProgramReport",
    ),
    (
        "program_variant_export.rs",
        "export_variants_now",
        "Request::ExportProgramVariants",
    ),
];

fn enclosing_function(lines: &[&str], index: usize) -> Option<String> {
    lines[..index].iter().rev().find_map(|line| {
        let (_, rest) = line.split_once("fn ")?;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    })
}

#[test]
fn bridge_handlers_never_run_a_worker_with_a_token_nobody_keeps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![root.join("live_bridge.rs")];
    for entry in std::fs::read_dir(root.join("live_bridge")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name.ends_with(".rs") && !name.contains("tests") {
            files.push(path);
        }
    }
    let dispatch = std::fs::read_to_string(root.join("live_bridge.rs")).unwrap();
    let mut offenders = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("CancelToken::default()") {
                continue;
            }
            let kept = line.trim_start().starts_with("let cancel =")
                && lines
                    .get(index + 1)
                    .is_some_and(|next| next.contains("cancel.clone()"));
            if kept {
                continue;
            }
            let function = enclosing_function(&lines, index).unwrap_or_default();
            match ONLY_IN_PROCESS
                .iter()
                .find(|(f, name, _)| *f == file && *name == function)
            {
                Some((_, _, queued)) => assert!(
                    dispatch.contains(queued),
                    "{file}:{} {function}: the socket no longer queues {queued}",
                    index + 1
                ),
                None => offenders.push(format!("{file}:{} in {function}", index + 1)),
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a worker no one can cancel: {offenders:?}"
    );
}
