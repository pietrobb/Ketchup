//! The thread of a tapped hole in the feature history: another library size
//! or depth, or none, as one undo step. The document refuses a thread its hole
//! does not take, by the tapping rule every export checks again.

use super::*;
use ketchup_model::document::HoleThread;

impl KetchupApp {
    /// The thread field of the selected feature when it is a tapped hole,
    /// filled with its thread as the document holds it now.
    pub(super) fn hole_thread_editor(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        feature_id: FeatureId,
    ) -> Option<FeatureHistoryUiAction> {
        let (_, thread) = snapshot.hole_threads().find(|(id, _)| *id == feature_id)?;
        let source = (snapshot.revision_id(), feature_id);
        if self.feature_history.thread_source != Some(source) {
            self.feature_history.thread_input =
                format!("{} {}", thread.size, format_height(thread.depth_mm));
            self.feature_history.thread_source = Some(source);
        }
        let label = self.catalog.text("feature-history-thread");
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.feature_history.thread_input).hint_text(&label),
        );
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label));
        show_value_refusal(
            ui,
            &self.catalog,
            &self.feature_history.thread_input,
            self.value_box.decimal,
        );
        let mut action = None;
        if ui
            .button(self.catalog.text("feature-history-change-thread"))
            .clicked()
        {
            action = Some(FeatureHistoryUiAction::SetHoleThread {
                feature_id,
                remove: false,
            });
        }
        if ui
            .button(self.catalog.text("feature-history-remove-thread"))
            .clicked()
        {
            action = Some(FeatureHistoryUiAction::SetHoleThread {
                feature_id,
                remove: true,
            });
        }
        action
    }

    /// Records the typed thread ("M8 12") on the tapped hole `feature_id`, or
    /// `remove`s its thread and leaves the hole drilled, as one undo step.
    pub(super) fn set_hole_thread(&mut self, feature_id: FeatureId, remove: bool) {
        let snapshot = self.document.current();
        let Some((_, current)) = snapshot.hole_threads().find(|(id, _)| *id == feature_id) else {
            return;
        };
        let thread = if remove {
            None
        } else {
            let Some(thread) = typed_thread(
                &self.feature_history.thread_input,
                current,
                self.value_box.decimal,
            ) else {
                self.digest = self.catalog.text("digest-hole-thread-invalid");
                return;
            };
            Some(thread)
        };
        let label = thread.as_ref().map(HoleThread::label);
        let batch = CommandBatch::new(vec![CanonicalCommand::SetHoleThread {
            id: feature_id,
            thread,
        }]);
        self.digest = match (self.apply_batch_with_work_recovery(&batch), label) {
            (Ok(()), Some(thread)) => self.catalog.format(
                "digest-hole-thread-changed",
                &BTreeMap::from([("thread", thread)]),
            ),
            (Ok(()), None) => self.catalog.text("digest-hole-thread-removed"),
            (Err(error), _) => self.catalog.format(
                "digest-hole-thread-refused",
                &BTreeMap::from([("reason", error.to_string())]),
            ),
        };
    }
}

/// The library thread `input` names with its depth, e.g. "M8 12", keeping
/// whether the `current` thread is modelled.
fn typed_thread(
    input: &str,
    current: &HoleThread,
    decimal: crate::length_input::DecimalSeparator,
) -> Option<HoleThread> {
    let thread = crate::app::hole::hole_spec(input, decimal)?.thread?;
    let pitch_mm = ketchup_program::library_metric_threads()
        .ok()?
        .get(&thread.size)?
        .pitch_mm;
    Some(HoleThread {
        size: thread.size,
        pitch_mm,
        depth_mm: thread.depth_mm,
        modeled: current.modeled,
    })
}
