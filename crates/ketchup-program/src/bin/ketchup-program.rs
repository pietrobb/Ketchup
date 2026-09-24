//! `ketchup-program check FILE.star [--set name=value]... [--model]`
//!
//! Evaluates a rule program and prints one JSON report on stdout: issues,
//! parameters, cut list, hardware and machining. Exit code 0 when the program
//! evaluates without error-level issues, 1 when it has issues, 2 when it
//! cannot be evaluated.

use std::collections::BTreeMap;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: ketchup-program check FILE.star [--set name=value]... [--model]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command, path, rest @ ..] = args.as_slice() else {
        return usage();
    };
    if command != "check" {
        return usage();
    }
    let mut overrides = BTreeMap::new();
    let mut with_model = false;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--model" => with_model = true,
            "--set" => {
                let Some((name, value)) = rest.get(index + 1).and_then(|pair| pair.split_once('='))
                else {
                    return usage();
                };
                let Ok(value) = value.parse::<f64>() else {
                    eprintln!("--set {name}: {value:?} is not a number");
                    return ExitCode::from(2);
                };
                overrides.insert(name.to_owned(), value);
                index += 1;
            }
            _ => return usage(),
        }
        index += 1;
    }
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("cannot read {path}: {error}");
            return ExitCode::from(2);
        }
    };
    match ketchup_program::run(path, &source, &overrides) {
        Ok((evaluated, report)) => {
            let mut value = serde_json::to_value(&report).expect("report serializes");
            if with_model {
                value["model"] = serde_json::to_value(&evaluated.model).expect("model serializes");
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("report serializes")
            );
            if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            let value = serde_json::json!({"ok": false, "error": error});
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("error serializes")
            );
            ExitCode::from(2)
        }
    }
}
