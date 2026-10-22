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

#[derive(Debug)]
enum UsageError {
    StdioRequired,
    WorkerPathMissing,
    WorkerPathInvalid(OsString, io::Error),
    UnknownOrRepeated(OsString),
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StdioRequired => formatter.write_str("--stdio is required (use --help)"),
            Self::WorkerPathMissing => formatter.write_str("--worker requires PATH"),
            Self::WorkerPathInvalid(path, error) => {
                write!(formatter, "--worker {}: {error}", path.to_string_lossy())
            }
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

impl std::error::Error for UsageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WorkerPathInvalid(_, error) => Some(error),
            _ => None,
        }
    }
}

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
            let path = args.next().ok_or(UsageError::WorkerPathMissing)?;
            // The scheduler runs only a worker named by an absolute path.
            worker = Some(
                std::path::absolute(&path)
                    .map_err(|error| UsageError::WorkerPathInvalid(path, error))?,
            );
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
        assert!(matches!(parse(&[]), Err(UsageError::StdioRequired)));
        assert!(matches!(
            parse(&["--stdio", "--worker"]),
            Err(UsageError::WorkerPathMissing)
        ));
        assert!(matches!(
            parse(&["--stdio", "--stdio"]),
            Err(UsageError::UnknownOrRepeated(arg)) if arg == "--stdio"
        ));
        // The scheduler runs only a worker named by an absolute path.
        assert_eq!(
            parse(&["--stdio", "--worker", "w"]).unwrap(),
            Some(Some(std::env::current_dir().unwrap().join("w")))
        );
        assert!(matches!(
            parse(&["--stdio", "--worker", ""]),
            Err(UsageError::WorkerPathInvalid(..))
        ));
        assert!(matches!(parse(&["--stdio", "--help"]), Ok(None)));
    }
}
