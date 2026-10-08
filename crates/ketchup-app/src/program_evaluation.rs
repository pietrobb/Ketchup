//! The window's one evaluation of its program per source and overrides.
//!
//! Publishing a program seeds it with the evaluation the edit already made. Any
//! other change of the document's program (open, undo, redo) is evaluated on a
//! background thread, so the tools that read program parts (Fillet/Chamfer edge
//! names, the program view of a pick, Push/Pull, takeoff, the source view) find
//! it ready instead of evaluating a house program on the UI thread.
use ketchup_model::document::RuleProgramSource;
use ketchup_program::{Evaluated, ProgramError};
use std::cell::{Cell, RefCell};
use std::sync::{Arc, mpsc};

/// A program evaluation as the tools see it: the error is kept, never `None`.
pub(crate) type Evaluation = Result<Arc<Evaluated>, ProgramError>;

#[derive(Default)]
pub(crate) struct ProgramEvaluations {
    ready: RefCell<Option<(RuleProgramSource, Evaluation)>>,
    pending: RefCell<Option<(RuleProgramSource, mpsc::Receiver<Evaluation>)>>,
    on_ui_thread: Cell<usize>,
}

fn evaluate(source: &RuleProgramSource) -> Evaluation {
    ketchup_program::evaluate(&source.file_name, &source.source, &source.overrides).map(Arc::new)
}

impl ProgramEvaluations {
    /// Keeps an evaluation the caller already made of `source`.
    pub(crate) fn seed(&self, source: &RuleProgramSource, evaluated: &Evaluated) {
        *self.ready.borrow_mut() = Some((source.clone(), Ok(Arc::new(evaluated.clone()))));
        self.pending
            .borrow_mut()
            .take_if(|(pending, _)| pending == source);
    }

    /// Starts evaluating `source` on a background thread unless it is ready or
    /// already being evaluated; `repaint` is woken when it finishes.
    pub(crate) fn warm(&self, source: &RuleProgramSource, repaint: &eframe::egui::Context) {
        self.poll();
        if self.is_ready(source)
            || self
                .pending
                .borrow()
                .as_ref()
                .is_some_and(|(pending, _)| pending == source)
        {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_source = source.clone();
        let repaint = repaint.clone();
        let spawned = std::thread::Builder::new()
            .name("ketchup-program-evaluation".into())
            .spawn(move || {
                let _ = sender.send(evaluate(&worker_source));
                repaint.request_repaint();
            });
        // Without a thread the evaluation simply happens when a tool asks.
        if spawned.is_ok() {
            *self.pending.borrow_mut() = Some((source.clone(), receiver));
        }
    }

    /// The evaluation of `source`: the kept one, the background one (waiting
    /// for it if it is still running), or one made now on the calling thread.
    pub(crate) fn get(&self, source: &RuleProgramSource) -> Evaluation {
        self.poll();
        if let Some((ready, evaluation)) = self.ready.borrow().as_ref()
            && ready == source
        {
            return evaluation.clone();
        }
        let waited = self
            .pending
            .borrow_mut()
            .take_if(|(pending, _)| pending == source)
            .and_then(|(_, receiver)| receiver.recv().ok());
        let evaluation = waited.unwrap_or_else(|| {
            self.on_ui_thread.set(self.on_ui_thread.get() + 1);
            evaluate(source)
        });
        *self.ready.borrow_mut() = Some((source.clone(), evaluation.clone()));
        evaluation
    }

    /// How many evaluations ran on the calling (UI) thread.
    #[cfg(test)]
    pub(crate) fn on_ui_thread(&self) -> usize {
        self.on_ui_thread.get()
    }

    fn is_ready(&self, source: &RuleProgramSource) -> bool {
        self.ready
            .borrow()
            .as_ref()
            .is_some_and(|(ready, _)| ready == source)
    }

    fn poll(&self) {
        let finished = {
            let pending = self.pending.borrow();
            match pending.as_ref().map(|(_, receiver)| receiver.try_recv()) {
                Some(Ok(evaluation)) => Some(Some(evaluation)),
                Some(Err(mpsc::TryRecvError::Disconnected)) => Some(None),
                Some(Err(mpsc::TryRecvError::Empty)) | None => None,
            }
        };
        if let Some(evaluation) = finished
            && let Some((source, _)) = self.pending.borrow_mut().take()
            && let Some(evaluation) = evaluation
        {
            *self.ready.borrow_mut() = Some((source, evaluation));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(text: &str) -> RuleProgramSource {
        RuleProgramSource {
            file_name: "model.star".into(),
            source: text.into(),
            overrides: Default::default(),
        }
    }

    #[test]
    fn a_warmed_program_is_evaluated_off_the_calling_thread_and_kept() {
        let evaluations = ProgramEvaluations::default();
        let block = program("box(\"block\", (100, 60, 40))\n");
        evaluations.warm(&block, &eframe::egui::Context::default());
        assert!(
            evaluations
                .get(&block)
                .unwrap()
                .model
                .part("block")
                .is_some()
        );
        assert!(evaluations.get(&block).is_ok());
        assert_eq!(evaluations.on_ui_thread(), 0);

        // Another program replaces it; asking without warming evaluates once.
        let other = program("box(\"other\", (10, 10, 10))\n");
        assert!(
            evaluations
                .get(&other)
                .unwrap()
                .model
                .part("other")
                .is_some()
        );
        assert!(evaluations.get(&other).is_ok());
        assert_eq!(evaluations.on_ui_thread(), 1);
    }

    #[test]
    fn a_failing_program_keeps_its_error() {
        let evaluations = ProgramEvaluations::default();
        let broken = program("box(\"block\", (100, 60, 40))\nfail(\"the wall is gone\")\n");
        let error = evaluations.get(&broken).unwrap_err();
        assert!(error.to_string().contains("the wall is gone"), "{error}");
        assert!(evaluations.get(&broken).is_err());
        assert_eq!(evaluations.on_ui_thread(), 1);
    }
}
