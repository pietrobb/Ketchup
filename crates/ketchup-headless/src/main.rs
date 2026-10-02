mod json_input;
mod protocol;
use ketchup_application::SessionSettings;
use std::{ffi::OsString, io, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("ketchup-headless: {error}");
        std::process::exit(1);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum UsageError {
    StdioRequired,
    WorkerPathMissing,
    UnknownOrRepeated(OsString),
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StdioRequired => formatter.write_str("--stdio is required (use --help)"),
            Self::WorkerPathMissing => formatter.write_str("--worker requires PATH"),
            Self::UnknownOrRepeated(arg) => {
                write!(
                    formatter,
                    "unknown or repeated argument: {}",
                    arg.to_string_lossy()
                )
            }
        }
    }
}

impl std::error::Error for UsageError {}

/// The worker path the arguments name, `Ok(None)` when they ask for help.
fn parse_args(
    mut args: impl Iterator<Item = OsString>,
) -> Result<Option<Option<PathBuf>>, UsageError> {
    let mut stdio = false;
    let mut worker = None;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok(None);
        } else if arg == "--stdio" && !stdio {
            stdio = true;
        } else if arg == "--worker" && worker.is_none() {
            worker = Some(PathBuf::from(
                args.next().ok_or(UsageError::WorkerPathMissing)?,
            ));
        } else {
            return Err(UsageError::UnknownOrRepeated(arg));
        }
    }
    if !stdio {
        return Err(UsageError::StdioRequired);
    }
    Ok(Some(worker))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(worker) = parse_args(std::env::args_os().skip(1))? else {
        eprintln!(
            "Usage: ketchup-headless --stdio [--worker PATH]\nJSON-lines ketchup.headless.v1 on stdin/stdout; diagnostics on stderr.\nDefault worker: sibling ketchup-exact-worker. No GUI or assistant sidecar.\nMutations may carry expected_revision / expected_digest / expected_mutation_epoch; supplied fields must match.\nEach apply is one atomic CAD program, not a whole-script transaction."
        );
        return Ok(());
    };
    let worker = worker
        .or_else(ketchup_application::evaluation::configured_exact_worker)
        .or_else(|| {
            ketchup_application::evaluation::exact_worker_candidates()
                .into_iter()
                .next()
        });
    let settings = SessionSettings {
        exact_worker_path: worker,
        ..SessionSettings::default()
    };
    protocol::serve(
        io::stdin().lock(),
        io::stdout().lock(),
        protocol::Server::new(settings),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{UsageError, parse_args};
    use std::{ffi::OsString, path::PathBuf};

    fn parse(args: &[&str]) -> Result<Option<Option<PathBuf>>, UsageError> {
        parse_args(args.iter().map(OsString::from))
    }

    #[test]
    fn usage_errors_name_the_offending_argument() {
        assert_eq!(parse(&[]), Err(UsageError::StdioRequired));
        assert_eq!(
            parse(&["--stdio", "--worker"]),
            Err(UsageError::WorkerPathMissing)
        );
        assert_eq!(
            parse(&["--stdio", "--stdio"]),
            Err(UsageError::UnknownOrRepeated("--stdio".into()))
        );
        assert_eq!(
            parse(&["--stdio", "--worker", "w"]),
            Ok(Some(Some(PathBuf::from("w"))))
        );
        assert_eq!(parse(&["--stdio", "--help"]), Ok(None));
    }
}
