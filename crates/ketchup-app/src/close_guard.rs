use crate::KetchupApp;
use eframe::egui;
use ketchup_model::document::DocumentId;

#[derive(Default)]
pub(crate) struct CloseGuard {
    pub(crate) pending: bool,
    approved: Option<CloseIdentity>,
    saving: bool,
    save_succeeded: bool,
    error: Option<String>,
}

#[derive(PartialEq)]
struct CloseIdentity {
    document: DocumentId,
    history: String,
}

impl KetchupApp {
    fn close_identity(&self) -> CloseIdentity {
        CloseIdentity {
            document: self.document.current().document_id(),
            history: self.document.history_digest(),
        }
    }

    pub(crate) fn finish_close_save(&mut self, succeeded: bool) {
        if self.close_guard.saving {
            self.close_guard.save_succeeded = succeeded;
        }
    }

    pub(crate) fn show_close_guard(&mut self, context: &egui::Context) {
        if context.viewport_id() != egui::ViewportId::ROOT || self.file_write_in_progress() {
            return;
        }
        if context.input(|input| input.viewport().close_requested()) {
            // eframe delivers our Close command as a new Close event. Consent is
            // single-use and only covers the work visible when it was granted.
            let approved = self.close_guard.approved.take();
            if !self.is_dirty() || approved.as_ref() == Some(&self.close_identity()) {
                self.close_guard = CloseGuard::default();
                return;
            }
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_guard.pending = true;
        }
        if self.close_guard.saving {
            if self.native_save_in_progress() {
                return;
            }
            self.close_guard.saving = false;
            if self.close_guard.save_succeeded && !self.is_dirty() {
                self.close_guard.pending = false;
                self.close_guard.approved = Some(self.close_identity());
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.close_guard.error = Some(self.digest.clone());
            }
        }
        if !self.close_guard.pending {
            return;
        }

        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("unsaved-close")).show(context, |ui| {
            ui.set_max_width(480.0);
            ui.heading(self.catalog.text("dialog-unsaved-title"));
            ui.label(self.document_title());
            ui.label(self.catalog.text("dialog-close-description"));
            if let Some(error) = &self.close_guard.error {
                ui.label(error);
            }
            ui.horizontal(|ui| {
                save = ui.button(self.catalog.text("file-save")).clicked();
                discard = ui
                    .button(self.catalog.text("dialog-close-discard"))
                    .clicked();
                cancel = ui.button(self.catalog.text("action-cancel")).clicked();
            });
        });
        if cancel || response.should_close() {
            self.close_guard = CloseGuard::default();
        } else if save {
            let Some(path) = self.file.path.clone().or_else(|| self.choose_save_path()) else {
                self.close_guard.error = None;
                return;
            };
            self.close_guard.error = None;
            self.close_guard.saving = true;
            self.close_guard.save_succeeded = false;
            self.start_background_save(path, context);
            context.request_repaint();
        } else if discard {
            self.close_guard.pending = false;
            self.close_guard.approved = Some(self.close_identity());
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
mod tests;
