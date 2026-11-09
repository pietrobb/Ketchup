//! `ketchup-program check FILE.star [--set name=value]... [--model]
//!  [--prelude PRELUDE.star] [--summary] [--diff SAVED.json] [--calculation sk|cz]`
//!
//! Evaluates a rule program and prints one JSON report on stdout: issues,
//! parameters, cut list, hardware and machining. Exit code 0 when the program
//! evaluates without error-level issues and its statics, if any, are verified;
//! 1 when it has error-level issues or a member or joint fails; 3 when nothing
//! fails but some member or joint is not verified or some load reaches no
//! member; 2 when it cannot be evaluated. A `--set` name the program has no
//! parameter of is said on stderr with the parameter it is likely meant for.
//!
//! `--prelude` evaluates with that prelude file instead of the built-in one,
//! so an edited prelude is checked without a build. `--summary` prints only
//! counts: parts per layer, the statics check and issues grouped by kind.
//! `--diff` compares that summary with one saved earlier (`--summary >
//! SAVED.json`) and lists what changed. `--calculation sk` (or `cz`) prints
//! the printable structural calculation in Slovak (Czech) as HTML instead of
//! the JSON report, with the same exit code.

use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::process::ExitCode;

const USAGE: &str = "usage: ketchup-program check FILE.star [--set name=value]... [--model] \
                     [--prelude PRELUDE.star] [--summary] [--diff SAVED.json] \
                     [--calculation sk|cz]";

/// Parts named per issue group in the summary.
const NAMED_PARTS: usize = 5;

/// Characters of the example message per issue group in the summary.
const EXAMPLE_CHARS: usize = 240;

/// What a refused command line prints: the usage only when the command itself
/// is wrong, not under a bad `--set` (review 2026-11-06 r23 P3: the whole usage
/// buried the one line that said what was wrong).
fn complaint(error: &CliError) -> String {
    error.to_string()
}

struct Options {
    path: String,
    overrides: BTreeMap<String, f64>,
    with_model: bool,
    prelude: Option<String>,
    summary: bool,
    diff: Option<String>,
    calculation: Option<ketchup_program::calculation::Language>,
}

/// Why the command line or an input file could not be used; each carries the
/// argument or path to fix and the underlying cause.
#[derive(Debug)]
enum CliError {
    Usage,
    MissingValue(String),
    Language(String),
    SetWithoutEquals(String),
    SetNotNumber {
        name: String,
        value: String,
        source: std::num::ParseFloatError,
    },
    SetNotFinite {
        name: String,
        value: String,
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
            Self::Language(code) => {
                write!(f, "--calculation {code}: expected sk or cz")
            }
            Self::SetWithoutEquals(pair) => write!(f, "--set {pair}: expected name=value"),
            Self::SetNotNumber {
                name,
                value,
                source,
            } => write!(f, "--set {name}: {value:?} is not a number ({source})"),
            Self::SetNotFinite { name, value } => {
                write!(f, "--set {name}: {value:?} is not a finite number")
            }
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
        calculation: None,
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
            "--calculation" => {
                let code = value()?;
                options.calculation = Some(
                    ketchup_program::calculation::Language::from_code(&code)
                        .ok_or(CliError::Language(code))?,
                );
            }
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
                if !parsed.is_finite() {
                    return Err(CliError::SetNotFinite {
                        name: name.to_owned(),
                        value: number.to_owned(),
                    });
                }
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
        Err(error) => {
            eprintln!("{}", complaint(&error));
            return ExitCode::from(2);
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
            let names: Vec<&str> = report
                .params
                .iter()
                .map(|param| param.name.as_str())
                .collect();
            for line in unused_set_lines(&report.unused_overrides, &names) {
                eprintln!("{line}");
            }
            let code = match (report.ok, report.statics()) {
                (false, _) | (_, ketchup_program::Statics::Failed) => ExitCode::from(1),
                (true, ketchup_program::Statics::NotVerified) => ExitCode::from(3),
                _ => ExitCode::SUCCESS,
            };
            if let Some(language) = options.calculation {
                print!(
                    "{}",
                    ketchup_program::calculation::document(&options.path, &report, language)
                );
                return code;
            }
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
        "statics_verdict": report.statics(),
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

/// Parameters named when a `--set` name is like none of them.
const LISTED_PARAMS: usize = 12;

/// One line per `--set` name the program has no parameter of, naming the
/// parameter it shares its longest start with (three letters or more), or else
/// the program's parameters (review 2026-11-08 PG-245).
fn unused_set_lines(unused_overrides: &[String], names: &[&str]) -> Vec<String> {
    unused_overrides
        .iter()
        .map(|unused| {
            let shared = |name: &&str| {
                unused
                    .chars()
                    .zip(name.chars())
                    .take_while(|(a, b)| a == b)
                    .count()
            };
            let nearest = names
                .iter()
                .filter(|name| shared(name) >= 3)
                .max_by_key(|name| shared(name));
            let said = match (nearest, names.is_empty()) {
                (Some(name), _) => format!("did you mean {name}?"),
                (None, true) => "it has none".to_owned(),
                (None, false) => {
                    let mut listed = names[..names.len().min(LISTED_PARAMS)].join(", ");
                    if names.len() > LISTED_PARAMS {
                        listed.push_str(", …");
                    }
                    format!("it has {listed}")
                }
            };
            format!("--set {unused}: the program has no parameter {unused}; {said}")
        })
        .collect()
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

    /// Review 2026-11-06 PG-182: `--set f=nan` passed every range check, as
    /// comparisons with NaN are false, and the program failed later naming no
    /// parameter.
    #[test]
    fn set_refuses_a_number_that_is_not_finite_by_its_name() {
        for value in ["nan", "inf", "-inf", "1e999"] {
            let args = ["check", "a.star", "--set", &format!("f={value}")].map(str::to_owned);
            let error = parse(&args).err().expect(value).to_string();
            assert!(error.starts_with("--set f:"), "{error}");
            assert!(error.contains("finite"), "{error}");
        }
    }

    /// Review 2026-11-06 r23 P3: a bad `--set` printed the whole usage after
    /// its one line.
    #[test]
    fn a_bad_set_says_what_is_wrong_without_the_usage() {
        for pair in ["n", "n=two", "n=nan"] {
            let args = ["check", "a.star", "--set", pair].map(str::to_owned);
            let said = complaint(&parse(&args).err().expect(pair));
            assert!(said.starts_with("--set "), "{said}");
            assert!(!said.contains("usage:"), "{said}");
            assert_eq!(said.lines().count(), 1, "{said}");
        }
        let said = complaint(&parse(&["run".to_owned()]).err().expect("no command"));
        assert!(said.starts_with("usage:"), "{said}");
    }

    /// Review 2026-11-08 PG-245: `--set snowsk=1.5` ended with exit code 3 and
    /// said nothing; the name stood only in the JSON's `unused_overrides`.
    #[test]
    fn a_set_of_no_parameter_is_said_with_the_nearest_name() {
        let unused = ["q".to_owned(), "snowsk".to_owned()];
        assert_eq!(
            unused_set_lines(&unused, &["snow_kn", "wind_kn"]),
            vec![
                "--set q: the program has no parameter q; it has snow_kn, wind_kn".to_owned(),
                "--set snowsk: the program has no parameter snowsk; did you mean snow_kn?"
                    .to_owned(),
            ]
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
