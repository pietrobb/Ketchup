//! `program action=export_variants`: the cut list and the material takeoff of
//! every saved variant, each evaluated with its own values on a worker thread.
//! The window's document, its Undo history and its parameters stay as they are.
#[cfg(test)]
#[path = "program_variant_export_tests.rs"]
mod tests;

use super::program_check::ProgramCheckJob;
use super::program_dry_run::QueuedReply;
use super::program_variants::owned_program;
use super::*;
use ketchup_application::program_variants::{
    VariantExport, combined_table, evaluate_variants, takeoff_table,
};
use ketchup_model::document::RuleProgramSource;
use ketchup_program::CancelToken;
use ketchup_program::table::{DecimalSeparator, Table};

pub(super) struct VariantExportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    cancel: CancelToken,
    receiver: mpsc::Receiver<Result<Value, Failed>>,
}

/// Why the worker wrote nothing; the answer is made on the UI thread, which
/// keeps the error details.
#[derive(Debug)]
enum Failed {
    Cancelled,
    NoneEvaluates(Vec<Value>),
    Write(std::io::Error),
}

/// Where the two tables go and how numbers are written.
struct Targets {
    cut_list: PathBuf,
    takeoff: PathBuf,
    xlsx: bool,
    decimal: DecimalSeparator,
}

impl Targets {
    fn bytes(&self, table: &Table) -> Vec<u8> {
        if self.xlsx {
            table.xlsx()
        } else {
            table.csv(self.decimal).into_bytes()
        }
    }
}

/// A new file at `path`: written beside it first, and never in place of a
/// file that appeared meanwhile.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    Ok(())
}

fn summary(variant: &VariantExport) -> Value {
    match &variant.result {
        Ok((cut_list, takeoff)) => json!({"name": variant.name, "params": variant.params,
            "cut_list_rows": cut_list.rows.len(), "parts": takeoff.counted_parts,
            "materials": takeoff.materials}),
        Err(error) => {
            json!({"name": variant.name, "params": variant.params, "error": format!("{error}")})
        }
    }
}

fn export(
    program: &RuleProgramSource,
    targets: &Targets,
    cancel: &CancelToken,
) -> Result<Value, Failed> {
    let variants = evaluate_variants(program, cancel).ok_or(Failed::Cancelled)?;
    let evaluated = variants
        .iter()
        .filter_map(|variant| {
            let (cut_list, takeoff) = variant.result.as_ref().ok()?;
            Some((variant.name.as_str(), cut_list, takeoff_table(takeoff)))
        })
        .collect::<Vec<_>>();
    let summaries = variants.iter().map(summary).collect::<Vec<_>>();
    if evaluated.is_empty() {
        return Err(Failed::NoneEvaluates(summaries));
    }
    let cut_list = combined_table(
        &evaluated
            .iter()
            .map(|(name, cut_list, _)| (*name, *cut_list))
            .collect::<Vec<_>>(),
    );
    let takeoff = combined_table(
        &evaluated
            .iter()
            .map(|(name, _, takeoff)| (*name, takeoff))
            .collect::<Vec<_>>(),
    );
    if cancel.is_cancelled() {
        return Err(Failed::Cancelled);
    }
    write_new(&targets.cut_list, &targets.bytes(&cut_list)).map_err(Failed::Write)?;
    if let Err(error) = write_new(&targets.takeoff, &targets.bytes(&takeoff)) {
        // Both tables or neither: a lone cut list would look like a whole export.
        let _ = std::fs::remove_file(&targets.cut_list);
        return Err(Failed::Write(error));
    }
    Ok(json!({
        "exported": evaluated.len(),
        "failed": variants.len() - evaluated.len(),
        "variants": summaries,
        "cut_list_path": targets.cut_list,
        "takeoff_path": targets.takeoff,
        "basis": "all_program_parts",
        "canonical_mutation": false,
        "hint": "Each variant is evaluated with its own values apart from the window: the document, its parameters and Undo history are unchanged. All parts of each variant count (layers hidden in the window do not apply); sizes are blanks.",
    }))
}

fn answer(result: Result<Value, Failed>) -> Result<Value, &'static str> {
    result.map_err(|failed| match failed {
        Failed::Cancelled => failure(
            "request_cancelled",
            "The export was cancelled; no file was written.",
            json!({}),
        ),
        Failed::NoneEvaluates(variants) => failure(
            "program_rejected",
            "No variant of the program evaluates, so nothing was written.",
            json!({"variants": variants}),
        ),
        Failed::Write(error) => failed_because("export_write_failed", error),
    })
}

impl LiveBridge {
    /// The program, its targets and that it has variants to export.
    fn variant_export(
        app: &KetchupApp,
        request: Request,
    ) -> Result<(RuleProgramSource, Targets), &'static str> {
        let Request::ExportProgramVariants { expected, path } = request else {
            return Err("invalid_request");
        };
        Self::guard(app, &Some(expected))?;
        let program = owned_program(app)?;
        if program.variants.is_empty() {
            return Err(program_access::invalid(
                "name",
                "The program has no variants to export.",
                "Save one first with action=save_variant.",
            ));
        }
        let target = Path::new(&path);
        let extension = target
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        let (Some(stem), Some(extension @ ("csv" | "xlsx"))) = (
            target.file_stem().and_then(|stem| stem.to_str()),
            extension.as_deref(),
        ) else {
            return Err(failure(
                "invalid_path",
                "The variants are written to a local absolute path ending in .csv or .xlsx.",
                json!({"path": path}),
            ));
        };
        let takeoff = target.with_file_name(format!("{stem}.takeoff.{extension}"));
        if !drawings::local_absolute_path(&path) {
            return Err(failure(
                "invalid_path",
                "The variants are written to a local absolute path (no network share, device or stream).",
                json!({"path": path}),
            ));
        }
        // No one confirms an overwrite on this path, so an existing file is never replaced.
        if let Some(existing) = [target, takeoff.as_path()]
            .into_iter()
            .find(|path| std::fs::symlink_metadata(path).is_ok())
        {
            return Err(failure(
                "export_target_exists",
                "A file already exists at this path; export the variants to a new file name.",
                json!({"path": existing}),
            ));
        }
        let decimal = match app.language {
            Some(crate::language::UiLanguage::English) => DecimalSeparator::Point,
            Some(crate::language::UiLanguage::Slovak) | None => DecimalSeparator::Comma,
        };
        let targets = Targets {
            cut_list: target.to_path_buf(),
            takeoff,
            xlsx: extension == "xlsx",
            decimal,
        };
        Ok((program, targets))
    }

    /// The synchronous export used by the in-process harness.
    pub(super) fn export_variants_now(
        app: &KetchupApp,
        request: Request,
    ) -> Result<Value, &'static str> {
        let (program, targets) = Self::variant_export(app, request)?;
        answer(export(&program, &targets, &CancelToken::default()))
    }

    pub(super) fn start_queued_variant_export(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        queued: QueuedReply,
        request: Request,
    ) {
        let QueuedReply {
            id,
            reply,
            cancelled,
        } = queued;
        take_error_details();
        if self.program_check_job.is_some() {
            let busy = failure(
                "busy",
                "A program is already planning or checking geometry.",
                json!({}),
            );
            Self::reply(app, id, &reply, Err(busy));
            return;
        }
        let (program, targets) = match Self::variant_export(app, request) {
            Ok(export) => export,
            Err(code) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
        let cancel = CancelToken::default();
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-program-variant-export".into())
            .spawn(move || {
                let _ = sender.try_send(export(&program, &targets, &worker_cancel));
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            let error = failed_because("job_worker_unavailable", error);
            Self::reply(app, id, &reply, Err(error));
            return;
        }
        self.program_check_job = Some(ProgramCheckJob::VariantExport(VariantExportJob {
            id,
            reply,
            cancelled,
            cancel,
            receiver,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_variant_export(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        job: VariantExportJob,
    ) {
        if job.cancelled.load(Ordering::Acquire) {
            job.cancel.cancel();
        }
        let result = match job.receiver.try_recv() {
            Ok(result) => {
                take_error_details();
                answer(result)
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job = Some(ProgramCheckJob::VariantExport(job));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(error) => Err(failed_because("program_worker_disconnected", error)),
        };
        Self::reply(app, job.id, &job.reply, result);
    }
}
