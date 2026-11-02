//! Review 2026-10-31 MCP-80, 2026-11-01 MCP-101: code that runs a worker with a cancel
//! flag nobody keeps cannot be cancelled and holds the window for the worker's whole
//! budget. In the bridge and in the window's helpers it calls (`app/`), every such flag
//! outside tests, a `CancelToken::default()` or an `AtomicBool::new(false)` that is not
//! shared through an `Arc`, is either kept by the job it starts (cloned for the worker on
//! the next line) or named below with why nothing waits on it unseen.

use std::path::Path;

/// (file, function, request the socket queues as a job instead): the window queues those
/// requests as jobs, and only the in-process `execute` reaches these.
const ONLY_IN_PROCESS: &[(&str, &str, &str)] = &[
    (
        "live_bridge/interchange.rs",
        "import_file",
        "dwg_import::imports_as_job",
    ),
    (
        "app/import_file.rs",
        "import_file",
        "dwg_import::imports_as_job",
    ),
    (
        "live_bridge/drawings.rs",
        "export_drawings",
        "project_drawings_job::exports_project_drawings",
    ),
    (
        "live_bridge/program_access.rs",
        "program_report_page",
        "Request::ProgramReport",
    ),
    (
        "live_bridge/program_variant_export.rs",
        "export_variants_now",
        "Request::ExportProgramVariants",
    ),
];

/// (file, function, why no one waits on it unseen).
const NOT_WAITED_ON: &[(&str, &str, &str)] = &[
    // Without a frame (a harness), nothing can show the work or cancel it; with
    // one the same function starts a thread with a kept token.
    (
        "app/exact_import_activity.rs",
        "start_exact_import_read",
        "no frame",
    ),
    ("app/underlay.rs", "start_underlay_read", "no frame"),
    (
        "app/underlay_export.rs",
        "export_selected_underlay",
        "no frame",
    ),
    // The job that read the DWG holds its conversion, so this finds it cached.
    ("app/dwg_import.rs", "drawing_dxf", "held conversion"),
    // Window dialogs the bridge never reaches; they run in the frame of their button.
    ("app/document.rs", "show_pdm_review_window", "window dialog"),
    ("app/export.rs", "show_cam_export_window", "window dialog"),
    (
        "app/fea_review.rs",
        "show_fea_review_window",
        "window dialog",
    ),
];

/// The function `lines[index]` is in, and whether it is compiled for tests only.
fn enclosing_function(lines: &[&str], index: usize) -> Option<(String, bool)> {
    let at = lines[..=index]
        .iter()
        .rposition(|line| line.contains("fn ") && !line.trim_start().starts_with("//"))?;
    let (_, rest) = lines[at].split_once("fn ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let attributes = lines[..at]
        .iter()
        .rev()
        .take_while(|line| {
            let line = line.trim_start();
            line.starts_with("#[") || line.starts_with("///")
        })
        .any(|line| line.contains("#[cfg(test)]"));
    (!name.is_empty()).then_some((name, attributes))
}

/// The lines before an inline `#[cfg(test)] mod … {`, the code built outside tests.
fn outside_tests(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines
        .windows(2)
        .position(|pair| {
            pair[0].trim() == "#[cfg(test)]"
                && pair[1].trim_start().contains("mod ")
                && pair[1].trim_end().ends_with('{')
        })
        .unwrap_or(lines.len());
    lines[..end].to_vec()
}

/// A flag on `line` that no one else can set.
fn unkept_flag(line: &str) -> bool {
    line.contains("CancelToken::default()")
        || (line.contains("AtomicBool::new(false)")
            && !line.contains("Arc::new(AtomicBool::new(false))"))
        || line.contains("&Arc::new(AtomicBool::new(false))")
}

#[test]
fn bridge_and_window_helpers_never_run_a_worker_with_a_flag_nobody_keeps() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![
        ("live_bridge.rs".to_owned(), root.join("live_bridge.rs")),
        ("app.rs".to_owned(), root.join("app.rs")),
    ];
    for directory in ["live_bridge", "app"] {
        for entry in std::fs::read_dir(root.join(directory)).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.ends_with(".rs") && !name.contains("tests") {
                files.push((format!("{directory}/{name}"), path));
            }
        }
    }
    let dispatch = std::fs::read_to_string(root.join("live_bridge.rs")).unwrap();
    let mut offenders = Vec::new();
    let mut used = Vec::new();
    for (file, path) in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let lines = outside_tests(&text);
        for (index, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") || !unkept_flag(line) {
                continue;
            }
            // Struct fields a thread shares by reference to their owner.
            if line.trim().split_once(": ").is_some_and(|(field, value)| {
                field.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && value == "AtomicBool::new(false),"
            }) {
                continue;
            }
            let kept = line.trim_start().starts_with("let cancel =")
                && lines
                    .get(index + 1)
                    .is_some_and(|next| next.contains("cancel.clone()"));
            if kept {
                continue;
            }
            let Some((function, test_only)) = enclosing_function(&lines, index) else {
                offenders.push(format!("{file}:{}", index + 1));
                continue;
            };
            if test_only {
                continue;
            }
            if let Some((_, _, queued)) = ONLY_IN_PROCESS
                .iter()
                .find(|(f, name, _)| *f == file && *name == function)
            {
                assert!(
                    dispatch.contains(queued),
                    "{file}:{} {function}: the socket no longer queues {queued}",
                    index + 1
                );
                used.push((file.clone(), function));
            } else if NOT_WAITED_ON
                .iter()
                .any(|(f, name, _)| *f == file && *name == function)
            {
                used.push((file.clone(), function));
            } else {
                offenders.push(format!("{file}:{} in {function}", index + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a worker no one can cancel: {offenders:?}"
    );
    // An exception that no longer has a flag to excuse is taken off the list.
    for (file, function, _) in ONLY_IN_PROCESS.iter().chain(NOT_WAITED_ON) {
        assert!(
            used.iter().any(|(f, name)| f == file && name == function),
            "{file} {function} keeps no unkept flag any more; remove it from the list"
        );
    }
}
