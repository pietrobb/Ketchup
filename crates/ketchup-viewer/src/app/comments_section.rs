//! The Comments section: write a comment on what is shown, point at parts with
//! arrows, restore a comment's view, and save the comments for the author.

use super::{FileRequest, ViewerApp};
use eframe::egui;
#[cfg(any(target_os = "windows", target_os = "macos"))]
use ketchup_rejection::{Rejection, RejectionPhase};
#[cfg(any(target_os = "windows", target_os = "macos"))]
use std::{io::Write, path::Path};

enum Action {
    Show(u64),
    Edit(u64),
    Delete(u64),
}

impl ViewerApp {
    /// Comments on the opened package; the model itself stays read-only.
    pub(super) fn comments(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let t = |key| language.text(key);
        let (Some(_), Some(review)) = (&self.model, &mut self.review) else {
            return;
        };
        ui.separator();
        ui.heading(t("viewer-comments"));
        if review.unsaved() {
            ui.small(t("viewer-comments-unsaved"));
        }
        if let Some(draft) = &mut review.draft {
            let label = ui.label(t("viewer-comment-author"));
            ui.add(egui::TextEdit::singleline(&mut draft.author))
                .labelled_by(label.id);
            let label = ui.label(t("viewer-comment-text"));
            ui.add(egui::TextEdit::multiline(&mut draft.text).desired_rows(3))
                .labelled_by(label.id);
            ui.label(language.format(
                "viewer-comment-arrows-count",
                &[("count", &draft.arrows.len().to_string())],
            ));
            match draft.placing {
                Some(None) => {
                    ui.colored_label(egui::Color32::LIGHT_BLUE, t("viewer-comment-arrow-tip"));
                }
                Some(Some(_)) => {
                    ui.colored_label(egui::Color32::LIGHT_BLUE, t("viewer-comment-arrow-tail"));
                }
                None => {}
            }
            let mut finish = None;
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("viewer-comment-arrow")).clicked() {
                    draft.placing = Some(None);
                }
                if !draft.arrows.is_empty() && ui.button(t("viewer-comment-clear-arrows")).clicked()
                {
                    draft.arrows.clear();
                }
                let can_save = !draft.text.trim().is_empty();
                if ui
                    .add_enabled(can_save, egui::Button::new(t("viewer-comment-save")))
                    .clicked()
                {
                    finish = Some(true);
                }
                if ui.button(t("viewer-comment-cancel")).clicked() {
                    finish = Some(false);
                }
            });
            match finish {
                Some(true) => {
                    review.commit();
                }
                Some(false) => review.draft = None,
                None => {}
            }
            return;
        }
        let mut action = None;
        let add = ui.button(t("viewer-comment-add")).clicked();
        if review.comments.comments.is_empty() {
            ui.small(t("viewer-comments-none"));
        }
        for comment in &review.comments.comments {
            let active = review.active == Some(comment.id);
            egui::Frame::group(ui.style())
                .stroke(if active {
                    egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(64, 200, 255))
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                })
                .show(ui, |ui| {
                    if let Some(author) = &comment.author {
                        ui.small(author);
                    }
                    ui.label(&comment.text);
                    ui.horizontal_wrapped(|ui| {
                        if ui.small_button(t("viewer-comment-show")).clicked() {
                            action = Some(Action::Show(comment.id));
                        }
                        if ui.small_button(t("viewer-comment-edit")).clicked() {
                            action = Some(Action::Edit(comment.id));
                        }
                        if ui.small_button(t("viewer-comment-delete")).clicked() {
                            action = Some(Action::Delete(comment.id));
                        }
                    });
                });
        }
        if add && let Some((view, selected)) = self.current_view() {
            self.start_comment(view, selected);
        }
        let result = match action {
            Some(Action::Show(id)) => self.show_comment(id),
            Some(Action::Edit(id)) => self.show_comment(id).map(|()| {
                if let Some(review) = &mut self.review {
                    review.edit(id);
                }
            }),
            Some(Action::Delete(id)) => {
                if let Some(review) = &mut self.review {
                    review.delete(id);
                }
                Ok(())
            }
            None => Ok(()),
        };
        if let Err(error) = result {
            self.error = Some(error);
        }
        if self.host_files.is_some() {
            self.host_comment_files(ui);
        } else {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            self.comment_files(ui);
        }
    }

    /// Comment file buttons whose files the host picks or downloads.
    fn host_comment_files(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let t = |key| language.text(key);
        ui.horizontal_wrapped(|ui| {
            if self.host_sharing && ui.button(t("viewer-comments-share")).clicked() {
                self.request_file(FileRequest::ShareComments);
            }
            for (key, request) in [
                ("viewer-comments-open", FileRequest::OpenComments),
                ("viewer-comments-save", FileRequest::SaveComments),
                ("viewer-comments-save-copy", FileRequest::SaveCommentedCopy),
            ] {
                // Opened comments replace the current ones.
                if ui.button(t(key)).clicked()
                    && (request != FileRequest::OpenComments || self.may_discard_comments())
                {
                    self.request_file(request);
                }
            }
        });
    }

    /// Start a new comment on the view now shown.
    fn start_comment(
        &mut self,
        view: ketchup_view_format::CommentView,
        selected: Option<ketchup_view_format::InstancePath>,
    ) {
        if let Some(review) = &mut self.review {
            review.start(view, selected);
        }
    }

    /// Desktop buttons that open and save comment files.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn comment_files(&mut self, ui: &mut egui::Ui) {
        let language = self.language;
        let t = |key| language.text(key);
        ui.horizontal_wrapped(|ui| {
            let result = if ui.button(t("viewer-comments-open")).clicked()
                && self.may_discard_comments()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(t("viewer-comments-filter"), &["ketchup-comments"])
                    .pick_file()
            {
                std::fs::File::open(&path)
                    .map_err(|error| file_error("viewer.comments.open", &path, error))
                    .and_then(|file| self.open_comments(file))
            } else if ui.button(t("viewer-comments-save")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(t("viewer-comments-filter"), &["ketchup-comments"])
                    .save_file()
            {
                write_atomically(&path, |file| self.save_comments(file))
                    .map(|written| self.comments_written(written))
            } else if ui.button(t("viewer-comments-save-copy")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter(t("viewer-file-filter"), &["ketchup-view"])
                    .save_file()
            {
                write_atomically(&path, |file| self.save_commented_copy(file))
                    .map(|written| self.comments_written(written))
            } else {
                Ok(())
            };
            if let Err(error) = result {
                self.error = Some(error);
            }
        });
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
/// Write `path` through a sibling temporary file, so an interrupted save
/// never leaves a half-written file in place of an earlier one.
pub(crate) fn write_atomically<T>(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<T, Rejection>,
) -> Result<T, Rejection> {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".part");
    let temporary = path.with_file_name(name);
    let mut file = std::fs::File::create(&temporary)
        .map_err(|error| file_error("viewer.comments.save", &temporary, error))?;
    let written = write(&mut file).and_then(|value| {
        file.flush()
            .and_then(|()| file.sync_all())
            .map(|()| value)
            .map_err(|error| file_error("viewer.comments.save", &temporary, error))
    });
    drop(file);
    let renamed = written.and_then(|value| {
        std::fs::rename(&temporary, path)
            .map(|()| value)
            .map_err(|error| file_error("viewer.comments.save", path, error))
    });
    if renamed.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    renamed
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn file_error(code: &'static str, path: &Path, error: std::io::Error) -> Rejection {
    Rejection::new(code, RejectionPhase::Io)
        .target(path.display().to_string())
        .reason("The comments file could not be read or written.")
        .fix_hint("Choose an accessible location and try again.")
        .caused_by(error)
}
