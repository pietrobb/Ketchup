//! Customer comments returned on a Viewer export, shown to the author: the
//! list, the customer's camera and the arrows at the parts they meant.
//! Comments never edit the model; parts are matched by the project identity
//! and full occurrence paths only, never by names or similar geometry.

use crate::*;
use ketchup_manufacturing::viewer_export::{DesktopView, received, view};
use std::io::BufReader;

/// Received comments, merged across imports by export and comment ID.
#[derive(Default)]
pub(crate) struct ReceivedComments {
    pub(crate) entries: Vec<ReceivedComment>,
    /// The comment shown in the viewport.
    pub(crate) active: Option<usize>,
}

/// One customer comment and the export it was written on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReceivedComment {
    pub(crate) package_sha256: String,
    pub(crate) source: view::Source,
    pub(crate) comment: view::Comment,
}

/// What one import changed in the list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommentsImport {
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
}

/// The shown comment in viewport points.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CommentOverlay {
    pub(crate) arrows: Vec<[Pos2; 2]>,
    pub(crate) label: Option<(Pos2, String)>,
}

/// How a received comment relates to the open model now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CommentStatus {
    pub(crate) changed: bool,
    pub(crate) missing: usize,
}

impl ReceivedComments {
    fn merge(&mut self, comments: view::Comments) -> CommentsImport {
        let mut counts = CommentsImport::default();
        for comment in comments.comments {
            let entry = ReceivedComment {
                package_sha256: comments.export.package_sha256.clone(),
                source: comments.export.source.clone(),
                comment,
            };
            match self.entries.iter_mut().find(|known| {
                known.package_sha256 == entry.package_sha256 && known.comment.id == entry.comment.id
            }) {
                Some(known) if *known == entry => counts.unchanged += 1,
                Some(known) => {
                    *known = entry;
                    counts.updated += 1;
                }
                None => {
                    self.entries.push(entry);
                    counts.added += 1;
                }
            }
        }
        counts
    }
}

impl KetchupApp {
    /// File > Open Viewer Comments: pick a `.ketchup-comments` file or a
    /// commented `.ketchup-view` and list its comments.
    pub(crate) fn open_viewer_comments(&mut self) {
        let label = self.catalog.text("file-filter-viewer-comments");
        let Some(path) = self
            .dialogs
            .pick_review_path(&label, &["ketchup-comments", "ketchup-view"])
        else {
            return;
        };
        self.import_viewer_comments_from(&path).ok();
    }

    /// Read the customer's comments at `path` into the list. Comments on
    /// another project are refused; importing the same file again changes
    /// nothing. The model and its Undo history are never touched.
    pub fn import_viewer_comments_from(
        &mut self,
        path: &Path,
    ) -> Result<CommentsImport, Rejection> {
        let result = (|| -> Result<_, Rejection> {
            let comments = read_comments(path)?;
            let state = received::export_state(&self.document.current(), &comments)?;
            Ok((self.viewer_prep.received.merge(comments), state))
        })();
        match result {
            Ok((counts, state)) => {
                let key = if state == received::ExportState::Current {
                    "digest-viewer-comments-opened"
                } else {
                    "digest-viewer-comments-opened-changed"
                };
                self.digest = self.catalog.format(
                    key,
                    &BTreeMap::from([
                        ("added", counts.added.to_string()),
                        ("updated", counts.updated.to_string()),
                        ("unchanged", counts.unchanged.to_string()),
                    ]),
                );
                Ok(counts)
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "digest-viewer-comments-failed",
                    &BTreeMap::from([("reason", error.reason_text().to_owned())]),
                );
                Err(error)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn received_viewer_comments(&self) -> &[ReceivedComment] {
        &self.viewer_prep.received.entries
    }

    pub(crate) fn received_comment_status(
        &self,
        snapshot: &Snapshot,
        entry: &ReceivedComment,
    ) -> CommentStatus {
        CommentStatus {
            changed: entry.source.canonical_digest != snapshot.canonical_digest(),
            missing: received::missing_parts(snapshot, &entry.comment).len(),
        }
    }

    /// Show comment `index` as the customer saw it: their camera, display
    /// style and section, with the commented part selected. Camera, display
    /// and selection only; parts no longer in the model are not guessed.
    pub fn show_received_viewer_comment(&mut self, index: usize) -> bool {
        let Some(entry) = self.viewer_prep.received.entries.get(index).cloned() else {
            return false;
        };
        let Some(rect) = self.camera.viewport_rect else {
            return false;
        };
        let viewport = [f64::from(rect.width()), f64::from(rect.height())];
        let orbit = [BOX_WIDTH_MM * 0.5, BOX_DEPTH_MM * 0.5];
        let Ok(desktop) = DesktopView::of(&entry.comment.view.camera, viewport, orbit) else {
            return false;
        };
        let before = self.camera_view_state();
        self.set_saved_camera(&ketchup_model::document::SavedCamera {
            parallel: desktop.parallel,
            yaw_rad: desktop.yaw_rad,
            pitch_rad: desktop.pitch_rad,
            target_z_mm: desktop.target_z_mm,
            zoom: desktop.points_per_mm * 420.0 / viewport[0].min(viewport[1]),
            pan: desktop.pan,
        });
        let (wireframe, edges) = match entry.comment.view.style {
            view::DisplayStyle::Shaded => (false, false),
            view::DisplayStyle::ShadedEdges => (false, true),
            view::DisplayStyle::Wireframe => (true, false),
        };
        self.view.set(ViewFlag::Wireframe, wireframe);
        self.view.set(ViewFlag::Edges, edges);
        self.section = entry.comment.view.section.as_ref().and_then(|section| {
            let squared: f64 = section.normal.iter().map(|v| v * v).sum();
            let point = section
                .normal
                .map(|v| v * section.offset_mm / squared.sqrt());
            SectionPlane::new(point, section.normal)
        });
        self.saved_views_ui.active = None;
        self.refresh_camera_distance();
        self.remember_camera_change(before);
        let snapshot = self.document.current();
        let selected = entry
            .comment
            .selected
            .as_ref()
            .and_then(|path| received::model_path(&snapshot, path));
        drop(snapshot);
        match selected {
            Some(path) => self.select_from_outliner(path, false),
            None => {
                self.clear_selection();
            }
        }
        self.viewer_prep.received.active = Some(index);
        true
    }

    /// Where the shown comment's arrows and text land in `rect`: arrows end at
    /// their parts where those are now; the text sits at the first arrow end
    /// on screen, else at the commented part. Missing parts get no arrow.
    pub(crate) fn received_comment_overlay(&self, rect: Rect) -> Option<CommentOverlay> {
        let entry = self
            .viewer_prep
            .received
            .active
            .and_then(|index| self.viewer_prep.received.entries.get(index))?;
        let snapshot = self.document.current();
        let arrows: Vec<[Pos2; 2]> = entry
            .comment
            .arrows
            .iter()
            .filter_map(|arrow| {
                let tail = received::anchor_point(&snapshot, &arrow.tail)?;
                let tip = received::anchor_point(&snapshot, &arrow.tip)?;
                self.project_visible_segment([Vec3::from(tail), Vec3::from(tip)], rect)
            })
            .collect();
        let on_screen = arrows
            .iter()
            .flatten()
            .copied()
            .find(|at| rect.contains(*at));
        let label_at = on_screen.or_else(|| {
            let part = received::model_path(&snapshot, entry.comment.selected.as_ref()?)?;
            let item = self
                .active_boxes()
                .into_iter()
                .find(|item| item.instance_path == part)?;
            Some(self.project(item.origin_mm + item.size_mm * 0.5, rect))
        });
        let text = match &entry.comment.author {
            Some(author) => format!("{author}: {}", entry.comment.text),
            None => entry.comment.text.clone(),
        };
        Some(CommentOverlay {
            arrows,
            label: label_at
                .filter(|at| rect.contains(*at))
                .map(|at| (at, text)),
        })
    }

    /// The shown comment's arrows and text at the part it refers to.
    pub(crate) fn paint_received_comment(&self, painter: &egui::Painter, rect: Rect) {
        let Some(overlay) = self.received_comment_overlay(rect) else {
            return;
        };
        let stroke = Stroke::new(2.4_f32, Color32::from_rgb(255, 140, 60));
        for [tail, tip] in overlay.arrows {
            painter.line_segment([tail, tip], stroke);
            let back = (tail - tip).normalized() * 12.0;
            let side = Vec2::new(-back.y, back.x) * 0.5;
            painter.line_segment([tip, tip + back + side], stroke);
            painter.line_segment([tip, tip + back - side], stroke);
        }
        let Some((at, text)) = overlay.label else {
            return;
        };
        let galley = painter.layout(
            text,
            egui::FontId::proportional(14.0),
            Color32::WHITE,
            (rect.width() * 0.4).max(160.0),
        );
        let frame = Rect::from_min_size(at + Vec2::splat(8.0), galley.size()).expand(6.0);
        painter.rect_filled(frame, 4.0, Color32::from_rgba_unmultiplied(40, 30, 20, 230));
        painter.rect_stroke(frame, 4.0, stroke, egui::StrokeKind::Inside);
        painter.galley(frame.min + Vec2::splat(6.0), galley, Color32::WHITE);
    }

    /// The received comments, newest import last, each with its state.
    pub(crate) fn show_received_comments(&mut self, ui: &mut egui::Ui) {
        if self.viewer_prep.received.entries.is_empty() {
            return;
        }
        let snapshot = self.document.current();
        let rows: Vec<_> = self
            .viewer_prep
            .received
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.clone(),
                    self.received_comment_status(&snapshot, entry),
                )
            })
            .collect();
        drop(snapshot);
        let mut show = None;
        egui::CollapsingHeader::new(self.catalog.text("viewer-received-comments-title"))
            .id_salt("viewer-received-comments")
            .default_open(true)
            .show(ui, |ui| {
                for (index, (entry, status)) in rows.iter().enumerate() {
                    let active = self.viewer_prep.received.active == Some(index);
                    ui.horizontal_wrapped(|ui| {
                        let author = entry.comment.author.clone().unwrap_or_else(|| {
                            self.catalog.text("viewer-received-comment-anonymous")
                        });
                        if ui.selectable_label(active, author).clicked() {
                            show = Some(index);
                        }
                        if status.missing > 0 {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                self.catalog.format(
                                    "viewer-received-comment-missing",
                                    &BTreeMap::from([("count", status.missing.to_string())]),
                                ),
                            );
                        } else if status.changed {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                self.catalog.text("viewer-received-comment-changed"),
                            );
                        }
                    });
                    ui.label(&entry.comment.text);
                    ui.add_space(4.0);
                }
            });
        if let Some(index) = show {
            self.show_received_viewer_comment(index);
        }
        ui.separator();
    }
}

fn read_comments(path: &Path) -> Result<view::Comments, Rejection> {
    let file = std::fs::File::open(path).map_err(|error| {
        Rejection::new(
            "viewer.comments.read",
            ketchup_rejection::RejectionPhase::Request,
        )
        .reason("The comments file could not be opened.")
        .caused_by(error)
    })?;
    let reader = BufReader::new(file);
    let package = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("ketchup-view"));
    if !package {
        return view::Comments::read_unbound(reader);
    }
    view::Package::read_with_comments(reader)?.1.ok_or_else(|| {
        Rejection::new(
            "viewer.comments.none",
            ketchup_rejection::RejectionPhase::Request,
        )
        .reason("This Viewer package carries no comments.")
        .fix_hint(
            "Open the copy the customer saved with comments, or their .ketchup-comments file.",
        )
    })
}
