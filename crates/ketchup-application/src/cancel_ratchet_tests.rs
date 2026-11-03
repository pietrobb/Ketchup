//! Review 2026-11-02 MCP-101: a `CancelToken::default()` here runs a program
//! worker that no request can end, so a client that gives up a request still
//! holds the worker for the rest of its time (a cancelled `check_program` held
//! it 45 s). The window's ratchet does not see this crate, so this one does:
//! every such token outside tests is named below with why only its caller
//! waits on it, on its own thread; a function a request runs takes the
//! request's token instead.

use std::path::{Path, PathBuf};

/// (file, function, why only its caller waits on it).
const UNCANCELLED: &[(&str, &str, &str)] = &[
    ("rule_program.rs", "plan_rule_program", "synchronous plan"),
    ("rule_program.rs", "plan_rule_program_from", "window edit"),
    ("rule_program.rs", "apply_rule_program_by", "session call"),
    ("rule_program.rs", "evaluate", "window edit of parts"),
];

/// The function `lines[index]` is in.
fn enclosing_function(lines: &[&str], index: usize) -> Option<String> {
    let at = lines[..=index]
        .iter()
        .rposition(|line| line.contains("fn ") && !line.trim_start().starts_with("//"))?;
    let (_, rest) = lines[at].split_once("fn ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
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

fn sources(directory: &Path, root: &Path, files: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, root, files);
            continue;
        }
        let name = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if name.ends_with(".rs") && !name.contains("tests") {
            files.push((name, path));
        }
    }
}

#[test]
fn no_worker_runs_with_a_cancel_token_no_request_holds() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &root, &mut files);
    let mut offenders = Vec::new();
    let mut used = Vec::new();
    for (file, path) in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let lines = outside_tests(&text);
        for (index, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("//") || !line.contains("CancelToken::default()") {
                continue;
            }
            let function = enclosing_function(&lines, index).unwrap_or_default();
            if UNCANCELLED
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
        "a worker no request can cancel: {offenders:?}"
    );
    for (file, function, _) in UNCANCELLED {
        assert!(
            used.iter().any(|(f, name)| f == file && name == function),
            "{file} {function} keeps no default token any more; remove it from the list"
        );
    }
}
