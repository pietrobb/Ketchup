mod json_input;
mod protocol;
use ketchup_application::SessionSettings;
use std::{io, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("ketchup-headless: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut stdio = false;
    let mut worker = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            eprintln!(
                "Usage: ketchup-headless --stdio [--worker PATH]\nJSON-lines ketchup.headless.v1 on stdin/stdout; diagnostics on stderr.\nDefault worker: sibling ketchup-exact-worker. No GUI or assistant sidecar.\nMutations require expected_revision + expected_digest from state.\nEach apply is one atomic CAD program, not a whole-script transaction."
            );
            return Ok(());
        } else if arg == "--stdio" && !stdio {
            stdio = true;
        } else if arg == "--worker" && worker.is_none() {
            worker = Some(PathBuf::from(args.next().ok_or("--worker requires PATH")?));
        } else {
            return Err(format!("unknown or repeated argument: {}", arg.to_string_lossy()).into());
        }
    }
    if !stdio {
        return Err("--stdio is required (use --help)".into());
    }
    let worker = worker
        .or_else(|| std::env::var_os("KETCHUP_EXACT_WORKER").map(PathBuf::from))
        .or_else(|| {
            std::env::current_exe().ok().and_then(|exe| {
                exe.parent().map(|p| {
                    p.join(if cfg!(windows) {
                        "ketchup-exact-worker.exe"
                    } else {
                        "ketchup-exact-worker"
                    })
                })
            })
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
