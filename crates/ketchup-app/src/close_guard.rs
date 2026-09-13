use crate::{KetchupApp, assistant_conversation_digest};
use eframe::egui;
use ketchup_core::document::DocumentId;

#[derive(Default)]
pub(crate) struct CloseGuard {
    pub(crate) pending: bool,
    approved: Option<CloseIdentity>,
    error: Option<String>,
}

#[derive(PartialEq)]
struct CloseIdentity {
    document: DocumentId,
    history: String,
    conversation: String,
}

impl KetchupApp {
    fn close_identity(&self) -> CloseIdentity {
        CloseIdentity {
            document: self.document.current().document_id(),
            history: self.document.history_digest(),
            conversation: assistant_conversation_digest(&self.assistant_messages),
        }
    }

    pub(crate) fn show_close_guard(&mut self, context: &egui::Context) {
        if context.viewport_id() != egui::ViewportId::ROOT {
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
            let Some(path) = self
                .document_path
                .clone()
                .or_else(|| self.choose_save_path())
            else {
                self.close_guard.error = None;
                return;
            };
            if !self.save_document_to(&path) {
                self.close_guard.error = Some(self.digest.clone());
            } else {
                self.close_guard.pending = false;
                self.close_guard.approved = Some(self.close_identity());
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if discard {
            self.close_guard.pending = false;
            self.close_guard.approved = Some(self.close_identity());
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
mod tests;
