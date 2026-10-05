//! `ketchup-program check FILE.star [--set name=value]... [--model]
//!  [--prelude PRELUDE.star] [--summary] [--diff SAVED.json]`
//!
//! Evaluates a rule program and prints one JSON report on stdout: issues,
//! parameters, cut list, hardware and machining. Exit code 0 when the program
//! evaluates without error-level issues, 1 when it has issues, 2 when it
//! cannot be evaluated.
//!
//! `--prelude` evaluates with that prelude file instead of the built-in one,
//! so an edited prelude is checked without a build. `--summary` prints only
//! counts: parts per layer, the statics check and issues grouped by kind.
//! `--diff` compares that summary with one saved earlier (`--summary >
//! SAVED.json`) and lists what changed.

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::process::ExitCode;

const USAGE: &str = "usage: ketchup-program check FILE.star [--set name=value]... [--model] \
                     [--prelude PRELUDE.star] [--summary] [--diff SAVED.json]";

/// Parts named per issue group in the summary.
const NAMED_PARTS: usize = 5;

/// Characters of the example message per issue group in the summary.
const EXAMPLE_CHARS: usize = 240;

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

struct Options {
    path: String,
    overrides: BTreeMap<String, f64>,
    with_model: bool,
    prelude: Option<String>,
    summary: bool,
    diff: Option<String>,
}

/// Why the command line or an input file could not be used; each carries the
/// argument or path to fix and the underlying cause.
#[derive(Debug)]
enum CliError {
    Usage,
    MissingValue(String),
    SetWithoutEquals(String),
    SetNotNumber {
        name: String,
        value: String,
        source: std::num::ParseFloatError,
    },
    Read {
        path: String,
        source: std::io::Error,
    },
    NotJson {
        path: String,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage => f.write_str(USAGE),
            Self::MissingValue(arg) => write!(f, "{arg} needs a value"),
            Self::SetWithoutEquals(pair) => write!(f, "--set {pair}: expected name=value"),
            Self::SetNotNumber {
                name,
                value,
                source,
            } => write!(f, "--set {name}: {value:?} is not a number ({source})"),
            Self::Read { path, source } => write!(f, "cannot read {path}: {source}"),
            Self::NotJson { path, source } => write!(f, "{path} is not JSON: {source}"),
        }
    }
}

fn parse(args: &[String]) -> Result<Options, CliError> {
    let [command, path, rest @ ..] = args else {
        return Err(CliError::Usage);
    };
    if command != "check" {
        return Err(CliError::Usage);
    }
    let mut options = Options {
        path: path.clone(),
        overrides: BTreeMap::new(),
        with_model: false,
        prelude: None,
        summary: false,
        diff: None,
    };
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        let mut value = || {
            rest.next()
                .cloned()
                .ok_or_else(|| CliError::MissingValue(arg.clone()))
        };
        match arg.as_str() {
            "--model" => options.with_model = true,
            "--summary" => options.summary = true,
            "--prelude" => options.prelude = Some(value()?),
            "--diff" => {
                options.diff = Some(value()?);
                options.summary = true;
            }
            "--set" => {
                let pair = value()?;
                let (name, number) = pair
                    .split_once('=')
                    .ok_or_else(|| CliError::SetWithoutEquals(pair.clone()))?;
                let parsed = number
                    .parse::<f64>()
                    .map_err(|source| CliError::SetNotNumber {
                        name: name.to_owned(),
                        value: number.to_owned(),
                        source,
                    })?;
                options.overrides.insert(name.to_owned(), parsed);
            }
            _ => return Err(CliError::Usage),
        }
    }
    Ok(options)
}

fn read(path: &str) -> Result<String, CliError> {
    std::fs::read_to_string(path).map_err(|source| CliError::Read {
        path: path.to_owned(),
        source,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse(&args) {
        Ok(options) => options,
        Err(CliError::Usage) => return usage(),
        Err(error) => {
            eprintln!("{error}");
            return usage();
        }
    };
    let loaded = (|| {
        if let Some(path) = &options.prelude {
            ketchup_program::use_prelude(path.clone(), read(path)?);
        }
        let saved =
            match &options.diff {
                Some(path) => Some(serde_json::from_str::<Value>(&read(path)?).map_err(
                    |source| CliError::NotJson {
                        path: path.clone(),
                        source,
                    },
                )?),
                None => None,
            };
        Ok::<_, CliError>((read(&options.path)?, saved))
    })();
    let (source, saved) = match loaded {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let (value, code) = match ketchup_program::run(&options.path, &source, &options.overrides) {
        Ok((evaluated, report)) => {
            let code = if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            };
            let mut value = if options.summary {
                summary(&evaluated.model, &report)
            } else {
                serde_json::to_value(&report).expect("report serializes")
            };
            if options.with_model {
                value["model"] = serde_json::to_value(&evaluated.model).expect("model serializes");
            }
            (value, code)
        }
        Err(error) => (json!({"ok": false, "error": error}), ExitCode::from(2)),
    };
    let value = match saved {
        Some(saved) => json!({"summary": value, "changes": changes(&saved, &value)}),
        None => value,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("report serializes")
    );
    code
}

/// Counts that show at a glance what a program makes and what is wrong.
fn summary(model: &ketchup_program::ProgramModel, report: &ketchup_program::Report) -> Value {
    let mut layers: BTreeMap<&str, usize> = BTreeMap::new();
    for part in &model.parts {
        if part.tags.is_empty() {
            *layers.entry("(no layer)").or_default() += 1;
        }
        for tag in &part.tags {
            *layers.entry(tag).or_default() += 1;
        }
    }
    let mut issues = Map::new();
    for issue in &report.issues {
        let group = issues.entry(issue.kind).or_insert_with(|| {
            json!({
                "severity": issue.severity,
                "count": 0,
                "example": shortened(&issue.message),
                "parts": [],
            })
        });
        group["count"] = json!(group["count"].as_u64().unwrap_or(0) + 1);
        let parts = group["parts"].as_array_mut().expect("parts is a list");
        for part in &issue.parts {
            if parts.len() < NAMED_PARTS && !parts.iter().any(|named| named == part) {
                parts.push(json!(part));
            }
        }
    }
    let mut value = json!({
        "ok": report.ok,
        "errors": report.errors,
        "warnings": report.warnings,
        "parts": model.parts.len(),
        "layers": layers,
        "issues": issues,
    });
    if !report.design.is_empty() {
        value["statics"] =
            serde_json::to_value(report.design.summary()).expect("summary serializes");
    }
    if !report.unused_overrides.is_empty() {
        value["unused_overrides"] = json!(report.unused_overrides);
    }
    value
}

/// `message` cut to its first [`EXAMPLE_CHARS`] characters.
fn shortened(message: &str) -> String {
    match message.char_indices().nth(EXAMPLE_CHARS) {
        Some((end, _)) => format!("{}…", &message[..end]),
        None => message.to_owned(),
    }
}

/// Every count, flag or name in `value` by its path; lists of names and the
/// example messages are context, not something to compare.
fn leaves(value: &Value, path: &str, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "example" || (key == "parts" && child.is_array()) {
                    continue;
                }
                let path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                leaves(child, &path, out);
            }
        }
        Value::Array(_) => {}
        leaf => {
            out.insert(path.to_owned(), leaf.clone());
        }
    }
}

/// What differs between a saved summary and the new one, as
/// `{path, before, after}` with `null` for a side that lacks it.
fn changes(saved: &Value, current: &Value) -> Vec<Value> {
    let (mut before, mut after) = (BTreeMap::new(), BTreeMap::new());
    leaves(saved, "", &mut before);
    leaves(current, "", &mut after);
    let paths: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    paths
        .into_iter()
        .filter_map(|path| {
            let (old, new) = (before.get(path), after.get(path));
            (old != new).then(|| json!({"path": path, "before": old, "after": new}))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The only test that sets the prelude: it holds for the whole process.
    #[test]
    fn an_edited_prelude_is_used_and_its_error_names_its_file_and_line() {
        let mut lines: Vec<&str> = ketchup_program::PRELUDE.lines().collect();
        lines.insert(99, "def broken(:");
        assert!(ketchup_program::use_prelude(
            "edited/prelude.star".to_owned(),
            lines.join("\n")
        ));
        let error = ketchup_program::run("box.star", "box(\"a\", (1, 1, 1))\n", &BTreeMap::new())
            .expect_err("the edited prelude does not parse");
        assert_eq!(error.code, "prelude_invalid");
        assert!(
            error.message.contains("edited/prelude.star:100:"),
            "{}",
            error.message
        );
    }

    #[test]
    fn changes_list_only_differing_counts_and_ignore_examples() {
        let saved = json!({"ok": true, "parts": 3, "layers": {"a": 2, "b": 1},
            "issues": {"gap": {"count": 1, "example": "x", "parts": ["p"]}}});
        let current = json!({"ok": false, "parts": 3, "layers": {"a": 3},
            "issues": {"gap": {"count": 1, "example": "y", "parts": ["q"]}}});
        assert_eq!(
            changes(&saved, &current),
            vec![
                json!({"path": "layers.a", "before": 2, "after": 3}),
                json!({"path": "layers.b", "before": 1, "after": null}),
                json!({"path": "ok", "before": true, "after": false}),
            ]
        );
    }
}
