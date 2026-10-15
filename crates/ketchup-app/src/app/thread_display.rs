//! Cosmetic threads in the viewport: a thin helix at the pitch of the thread
//! on every threaded cylinder, drawn with its definition's edges. The
//! geometry stays a plain cylinder.

use crate::program_evaluation::Lookup;
use crate::renderer::{DefinitionLines, FaceColors, single_precision};
use crate::*;
use ketchup_application::threads::{ThreadCylinder, modeled_threads_batch, thread_cylinders};
use ketchup_geometry::linalg;

/// Straight chords per quarter turn of a drawn helix.
const CHORDS_PER_QUARTER_TURN: u32 = 4;

impl KetchupApp {
    /// The instanced plan of `snapshot` with its threads drawn; it waits for
    /// the program's evaluation to draw the outside threads of program parts.
    pub(crate) fn rebuild_render_plan(&mut self, snapshot: &Snapshot) -> Arc<InstancedRenderPlan> {
        let (threads, awaits_program) = match self.document_threads(snapshot) {
            Some(threads) => (threads, false),
            None => (thread_cylinders(snapshot, None), true),
        };
        let lines = thread_lines(&threads);
        let plan = Arc::new(InstancedRenderPlan::from_snapshot_with_lines(
            snapshot,
            &self.exact.results,
            &lines,
            self.view
                .contains(ViewFlag::DraftAnalysis)
                .then_some(super::draft::draft_face_rgb as FaceColors),
            &mut self.render.cache,
        ));
        self.render.plan = Some(Arc::clone(&plan));
        self.render.plan_awaits_program = awaits_program;
        plan
    }

    /// Every thread of `snapshot`, its program parts' included; `None` while
    /// the program is still being evaluated. A program that failed names no
    /// threads of its own.
    pub(crate) fn document_threads(&self, snapshot: &Snapshot) -> Option<Vec<ThreadCylinder>> {
        let evaluated = match self
            .document
            .current_rule_program()
            .map(|program| self.program_evaluations.try_get(program))
        {
            None => None,
            Some(Lookup::Ready(evaluation)) => evaluation.ok(),
            Some(Lookup::Pending) => return None,
        };
        Some(thread_cylinders(snapshot, evaluated.as_deref()))
    }

    /// The View menu's switch between modelled and cosmetic threads.
    pub(crate) fn modeled_threads_checkbox(&mut self, ui: &mut egui::Ui) {
        let mut modeled = self.document.current().modeled_threads();
        let label = self.catalog.text("view-modeled-threads");
        let response = ui
            .checkbox(&mut modeled, &label)
            .on_hover_text(self.catalog.text("hint-modeled-threads"));
        if response.changed() {
            self.set_modeled_threads(modeled);
            ui.close();
        }
    }

    /// Models the document's threads (`modeled`) or draws them cosmetic where
    /// a thread does not say, as one undo step; a program keeps owning the
    /// document and its bolts follow. Waits while the program is evaluated.
    pub fn set_modeled_threads(&mut self, modeled: bool) -> bool {
        let snapshot = self.document.current();
        if snapshot.modeled_threads() == modeled {
            return false;
        }
        let evaluated = match self
            .document
            .current_rule_program()
            .map(|program| self.program_evaluations.try_get(program))
        {
            None => None,
            Some(Lookup::Ready(evaluation)) => evaluation.ok(),
            Some(Lookup::Pending) => {
                self.digest = self.catalog.text("digest-modeled-threads-wait");
                return false;
            }
        };
        let batch = modeled_threads_batch(
            &snapshot,
            evaluated.as_deref().map(|evaluated| &evaluated.model),
            modeled,
        );
        if self.apply_batch_with_work_recovery(&batch).is_err() {
            return false;
        }
        self.digest = self.catalog.text(if modeled {
            "digest-modeled-threads-on"
        } else {
            "digest-modeled-threads-off"
        });
        true
    }

    /// Whether the plan lacks program threads that could be drawn now.
    pub(crate) fn render_plan_misses_program(&self) -> bool {
        self.render.plan_awaits_program
            && self.document.current_rule_program().is_some_and(|program| {
                matches!(self.program_evaluations.try_get(program), Lookup::Ready(_))
            })
    }
}

/// The helix of every cosmetic thread, by definition, in the definition's
/// frame. A modelled thread is in the geometry and needs none.
pub(crate) fn thread_lines(threads: &[ThreadCylinder]) -> DefinitionLines {
    let mut lines = DefinitionLines::new();
    for thread in threads.iter().filter(|thread| !thread.modeled) {
        lines
            .entry(thread.definition_id)
            .or_default()
            .extend(helix(thread));
    }
    lines
}

/// One helix turn per pitch along the thread, on the modelled surface: on a
/// shaft at its radius, in a hole just inside the chords the display mesh
/// cuts across its wall, so the faces in front still hide it.
fn helix(thread: &ThreadCylinder) -> Vec<[[f32; 3]; 2]> {
    let half_chord_turn = ketchup_scheduler::exact_worker::STEP_MESH_ANGULAR_DEFLECTION / 2.0;
    let radius_mm = thread.surface_diameter_mm / 2.0
        * if thread.internal {
            half_chord_turn.cos()
        } else {
            1.0
        };
    let axis = thread.direction;
    let Some(across) = linalg::normalize(linalg::cross(
        axis,
        if axis[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        },
    )) else {
        return Vec::new();
    };
    let side = linalg::cross(axis, across);
    // Points on the exact circle: chords only ever cut inside it.
    let quarter_turns = (4.0 * thread.length_mm / thread.pitch_mm).ceil().max(1.0);
    let chords = quarter_turns * f64::from(CHORDS_PER_QUARTER_TURN);
    let point = |index: f64| {
        let along = thread.length_mm * index / chords;
        let (sin, cos) = (std::f64::consts::TAU * along / thread.pitch_mm).sin_cos();
        single_precision(linalg::add(
            linalg::add(thread.start_mm, linalg::scale(axis, along)),
            linalg::add(
                linalg::scale(across, radius_mm * cos),
                linalg::scale(side, radius_mm * sin),
            ),
        ))
    };
    std::iter::successors(Some(0.0), |index| Some(index + 1.0))
        .take_while(|index| *index < chords)
        .map(|index| [point(index), point(index + 1.0)])
        .collect()
}
