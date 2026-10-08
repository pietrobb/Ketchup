//! The window's one evaluation of its program per source and overrides.
//!
//! Publishing a program seeds it with the evaluation the edit already made. Any
//! other change of the document's program (open, undo, redo) is evaluated on a
//! background thread, so the tools that read program parts (Fillet/Chamfer edge
//! names, Push/Pull of a drawn shape) never evaluate a house program on the UI
//! thread nor wait for one: they ask with `try_get` and show that they are
//! planning until it is ready. Only answers owed in the same call (a live-bridge
//! request, the built-in assistant) wait, with `get_blocking`.
use ketchup_model::document::RuleProgramSource;
use ketchup_program::{Evaluated, ProgramError};
use std::cell::{Cell, RefCell};
use std::sync::{Arc, mpsc};

/// A program evaluation as the tools see it: the error is kept, never `None`.
pub(crate) type Evaluation = Result<Arc<Evaluated>, ProgramError>;

/// What a tool finds when it asks for an evaluation without waiting.
pub(crate) enum Lookup {
    Ready(Evaluation),
    /// Being evaluated on a background thread; the window repaints when it is
    /// done, so the tool says it is planning and asks again.
    Pending,
}

#[derive(Default)]
pub(crate) struct ProgramEvaluations {
    ready: RefCell<Option<(RuleProgramSource, Evaluation)>>,
    pending: RefCell<Option<(RuleProgramSource, mpsc::Receiver<Evaluation>)>>,
    repaint: RefCell<Option<eframe::egui::Context>>,
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
        *self.repaint.borrow_mut() = Some(repaint.clone());
        self.start(source);
    }

    /// The evaluation of `source` without waiting: the kept one, or `Pending`
    /// while a background thread evaluates it (started now if none does).
    pub(crate) fn try_get(&self, source: &RuleProgramSource) -> Lookup {
        if self.start(source) {
            return Lookup::Pending;
        }
        match self.ready.borrow().as_ref() {
            Some((ready, evaluation)) if ready == source => Lookup::Ready(evaluation.clone()),
            _ => Lookup::Pending,
        }
    }

    /// The evaluation of `source`, waiting for it when it is still running.
    /// Only for answers owed in the same call (a live-bridge request, the
    /// built-in assistant); tools driven by the pointer or a key use
    /// [`Self::try_get`].
    pub(crate) fn get_blocking(&self, source: &RuleProgramSource) -> Evaluation {
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

    /// Makes `source` pending until the test sends its evaluation, as after an
    /// Open or Undo of a program that takes long to evaluate. A tool that
    /// waited for it would hang the test.
    #[cfg(test)]
    pub(crate) fn hold(&self, source: &RuleProgramSource) -> mpsc::SyncSender<Evaluation> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.ready.borrow_mut().take();
        *self.pending.borrow_mut() = Some((source.clone(), receiver));
        sender
    }

    /// Evaluates `source` the way the background thread does.
    #[cfg(test)]
    pub(crate) fn evaluate_for_test(source: &RuleProgramSource) -> Evaluation {
        evaluate(source)
    }

    /// How many evaluations ran on the calling (UI) thread.
    #[cfg(test)]
    pub(crate) fn on_ui_thread(&self) -> usize {
        self.on_ui_thread.get()
    }

    /// Polls the background thread and starts one for `source` unless it is
    /// ready or already running. True while `source` is being evaluated.
    fn start(&self, source: &RuleProgramSource) -> bool {
        self.poll();
        if self.is_ready(source) {
            return false;
        }
        if self
            .pending
            .borrow()
            .as_ref()
            .is_some_and(|(pending, _)| pending == source)
        {
            return true;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_source = source.clone();
        let repaint = self.repaint.borrow().clone();
        let spawned = std::thread::Builder::new()
            .name("ketchup-program-evaluation".into())
            .spawn(move || {
                let _ = sender.send(evaluate(&worker_source));
                if let Some(repaint) = repaint {
                    repaint.request_repaint();
                }
            });
        if spawned.is_ok() {
            *self.pending.borrow_mut() = Some((source.clone(), receiver));
            return true;
        }
        // Without a thread the evaluation happens here, once.
        self.on_ui_thread.set(self.on_ui_thread.get() + 1);
        *self.ready.borrow_mut() = Some((source.clone(), evaluate(source)));
        false
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
    use std::time::{Duration, Instant};

    fn program(text: &str) -> RuleProgramSource {
        RuleProgramSource {
            file_name: "model.star".into(),
            source: text.into(),
            overrides: Default::default(),
        }
    }

    fn ready(lookup: Lookup) -> Option<Evaluation> {
        match lookup {
            Lookup::Ready(evaluation) => Some(evaluation),
            Lookup::Pending => None,
        }
    }

    /// Asks without waiting until the background evaluation is kept.
    fn settle(evaluations: &ProgramEvaluations, source: &RuleProgramSource) -> Evaluation {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(evaluation) = ready(evaluations.try_get(source)) {
                return evaluation;
            }
            assert!(Instant::now() < deadline, "the evaluation never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn asking_for_a_house_being_evaluated_does_not_wait_for_it() {
        let house = RuleProgramSource {
            file_name: "tiny-house.star".into(),
            source: include_str!("../../../examples/programs/tiny-house.star").into(),
            overrides: Default::default(),
        };
        // What a whole evaluation of the house costs on this machine.
        let started = Instant::now();
        assert!(ProgramEvaluations::default().get_blocking(&house).is_ok());
        let whole = started.elapsed();

        // Right after Open the frame warms the program and a tool asks at once.
        let evaluations = ProgramEvaluations::default();
        evaluations.warm(&house, &eframe::egui::Context::default());
        let started = Instant::now();
        let first = evaluations.try_get(&house);
        let asked = started.elapsed();
        assert!(matches!(first, Lookup::Pending));
        assert!(
            asked < Duration::from_millis(20) && asked * 10 < whole,
            "asking took {asked:?}; the whole evaluation takes {whole:?}"
        );

        // The tool asks again on later frames and gets the house, evaluated once
        // and never on the asking thread.
        let evaluated = settle(&evaluations, &house).unwrap();
        assert!(!evaluated.part_sources.is_empty());
        assert_eq!(evaluations.on_ui_thread(), 0);

        // Waiting (the bridge's same-call answer, the old tool path) costs
        // about the whole evaluation, which no tool may spend in a frame.
        let waiting = ProgramEvaluations::default();
        waiting.warm(&house, &eframe::egui::Context::default());
        let started = Instant::now();
        assert!(waiting.get_blocking(&house).is_ok());
        let waited = started.elapsed();
        assert_eq!(waiting.on_ui_thread(), 0);
        eprintln!("house: whole {whole:?}, asked {asked:?}, waited after warm {waited:?}");
    }

    #[test]
    fn a_program_nobody_warmed_is_evaluated_in_the_background_too() {
        let evaluations = ProgramEvaluations::default();
        let block = program("box(\"block\", (100, 60, 40))\n");
        assert!(matches!(evaluations.try_get(&block), Lookup::Pending));
        assert!(
            settle(&evaluations, &block)
                .unwrap()
                .model
                .part("block")
                .is_some()
        );
        assert_eq!(evaluations.on_ui_thread(), 0);

        // Another program replaces it; an answer owed at once evaluates it here.
        let other = program("box(\"other\", (10, 10, 10))\n");
        assert!(
            evaluations
                .get_blocking(&other)
                .unwrap()
                .model
                .part("other")
                .is_some()
        );
        assert!(ready(evaluations.try_get(&other)).is_some_and(|evaluation| evaluation.is_ok()));
        assert_eq!(evaluations.on_ui_thread(), 1);
    }

    #[test]
    fn a_failing_program_keeps_its_error() {
        let evaluations = ProgramEvaluations::default();
        let broken = program("box(\"block\", (100, 60, 40))\nfail(\"the wall is gone\")\n");
        let error = settle(&evaluations, &broken).unwrap_err();
        assert!(error.to_string().contains("the wall is gone"), "{error}");
        assert!(evaluations.get_blocking(&broken).is_err());
        assert_eq!(evaluations.on_ui_thread(), 0);
    }
}
