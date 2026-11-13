//! File > Export > Presentation image: a still PNG of the visible model for a client or a
//! quotation, with the background, size and see-through materials the user chose. The modelling
//! viewport is unchanged. The image is made on a thread, for the window and a client alike, so
//! the window keeps painting; cancelling stops the thread within a row of samples (review
//! 2026-11-12 R1112-IMG-502: a client's image froze the window for seconds and a cancelled one
//! kept rendering).

use crate::*;
use ketchup_manufacturing::blender_export::MeshGlbInstance;
use ketchup_manufacturing::presentation::{
    PresentationError, PresentationLabel, PresentationOptions, part_labels, presentation_scene,
    render_presentation_until,
};
use ketchup_model::annotation::DecimalMark;
use ketchup_model::drawing::BomItems;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// What the thread making a presentation image sends back: its PNG, or why there is none.
pub(crate) type MadePresentation = Result<Vec<u8>, ExportError>;

/// How a presentation image looks: the render options, how opaque the parts of each named
/// material are, in percent (100 = opaque; parts of other materials are opaque), and its labels:
/// each component's name when `part_labels`, and the texts a client places.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PresentationRequest {
    pub(crate) options: PresentationOptions,
    pub(crate) opacity_percent: BTreeMap<String, u8>,
    pub(crate) part_labels: bool,
    pub(crate) labels: Vec<PresentationLabel>,
}

/// The one presentation image a window makes at a time, for itself or for a client. It is free
/// only once the thread of the last image has ended, not as soon as that image is cancelled, so
/// asking and cancelling again does not pile threads up (review 2026-11-13 R1113-IMG-600).
#[derive(Default)]
pub(crate) struct PresentationSlot(Arc<()>);

impl PresentationSlot {
    /// A presentation thread is still running.
    pub(crate) fn busy(&self) -> bool {
        Arc::strong_count(&self.0) > 1
    }

    /// Holds the slot as a running thread would, until the result is dropped.
    #[cfg(test)]
    pub(crate) fn clone_held(&self) -> Arc<()> {
        Arc::clone(&self.0)
    }
}

/// A presentation image being made on its own thread.
pub(crate) struct PresentationTask {
    path: PathBuf,
    /// Set by Cancel; the thread stops at its next row.
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<MadePresentation>,
}

/// The visible meshes of a snapshot with the opacity (0–255) of each.
pub(crate) struct PresentationParts {
    snapshot: Snapshot,
    /// The document's blobs, for the pictures textured faces show.
    blobs: ketchup_model::persistence::Blobs,
    scene: Vec<CurrentVisibleMesh>,
    opacity: Vec<u8>,
    /// How the dimensions write a decimal, as the drawings do (review 2026-11-12 P3).
    decimal_mark: DecimalMark,
}

impl PresentationParts {
    /// Makes the PNG of these parts as `request` says on a thread of its own, which stops once
    /// `cancelled` is set and holds `slot` until it ends; what it made comes on the receiver,
    /// and `repaint` is asked to paint. Refused while `slot` is held.
    pub(crate) fn spawn(
        self,
        request: PresentationRequest,
        cancelled: Arc<AtomicBool>,
        repaint: egui::Context,
        slot: &PresentationSlot,
    ) -> Result<Receiver<MadePresentation>, ExportError> {
        if slot.busy() {
            return Err(presentation_busy());
        }
        let held = Arc::clone(&slot.0);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let made = self.make(&request, &cancelled);
            drop(held);
            if sender.send(made).is_ok() {
                repaint.request_repaint();
            }
        });
        Ok(receiver)
    }

    /// The PNG of these parts as `request` says. A panic in the renderer fails this image
    /// instead of the window (R1112-IMG-500).
    fn make(&self, request: &PresentationRequest, cancelled: &AtomicBool) -> MadePresentation {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.render(request, cancelled)
        }))
        .unwrap_or_else(|payload| {
            Err(ExportError::failed(format!(
                "the presentation renderer stopped: {}",
                crate::bounded_parser::panic_message(payload.as_ref())
            )))
        })
    }

    fn render(&self, request: &PresentationRequest, cancelled: &AtomicBool) -> MadePresentation {
        let instances = self
            .scene
            .iter()
            .map(|body| MeshGlbInstance {
                source: body.source.as_export_source(),
                occurrence: &body.occurrence,
            })
            .collect::<Vec<_>>();
        let opacity = self
            .scene
            .iter()
            .zip(&self.opacity)
            .map(|(body, opacity)| (&body.occurrence.instance_path, *opacity))
            .collect::<BTreeMap<_, _>>();
        let mut scene = presentation_scene(&self.snapshot, &instances, &self.blobs, |occurrence| {
            opacity
                .get(&occurrence.instance_path)
                .copied()
                .unwrap_or(u8::MAX)
        })
        .map_err(presentation_refused)?;
        if request.part_labels {
            scene.labels = part_labels(&instances);
        }
        scene.labels.extend(request.labels.iter().cloned());
        let options = PresentationOptions {
            decimal_mark: self.decimal_mark,
            ..request.options
        };
        render_presentation_until(&scene, &options, cancelled)
            .and_then(|image| image.png())
            .map_err(presentation_refused)
    }
}

/// Why the renderer made no image, as a refusal that says how to get one.
pub(super) fn presentation_refused(error: PresentationError) -> ExportError {
    let hint = match &error {
        PresentationError::Size { .. } | PresentationError::TooManyPixels { .. } => {
            "Make the image smaller; its longest side and its pixels are limited."
        }
        PresentationError::LabelAway { .. } | PresentationError::LabelPoint { .. } => {
            "Place each label at a point on or near the model."
        }
        PresentationError::NonFiniteGeometry { .. } => {
            "Move the model near the origin; a presentation image draws coordinates within a million kilometres of it."
        }
        PresentationError::Cancelled => "Export the image again.",
        PresentationError::Supersample { .. } => "Smooth edges with 1 to 4 samples per pixel side.",
        PresentationError::ViewDirection => "View the model from a direction, not from its centre.",
        PresentationError::EmptyModel => {
            "Show at least one part with a surface; hidden parts and tags are left out."
        }
        PresentationError::TooManyTriangles { .. } => {
            "Hide some parts, or show fewer detailed imported meshes, and export again."
        }
        PresentationError::LabelText { .. } => {
            "Give each label text on one line, within the limit of characters."
        }
        PresentationError::LabelLetter { .. } => {
            "Write the label in Latin, Greek or Cyrillic letters or emoji; other scripts cannot be drawn yet."
        }
        PresentationError::TooManyLabels { .. } => {
            "Turn off the part labels or name fewer labels; part labels count with yours."
        }
        PresentationError::TextureReference { .. } => {
            "Import the textured mesh again; it names a picture or texture coordinate it does not have."
        }
        // Not the user's to change: the built-in font, a stored picture, the model's data
        // or the PNG writer failed.
        PresentationError::Font
        | PresentationError::Texture { .. }
        | PresentationError::Source(_)
        | PresentationError::Png(_) => return ExportError::failed(error),
    };
    Rejection::new("presentation.image", RejectionPhase::Validation)
        .reason(error.to_string())
        .fix_hint(hint)
        .into()
}

/// Another presentation image's thread has not ended yet.
pub(crate) fn presentation_busy() -> ExportError {
    Rejection::new("presentation.busy", RejectionPhase::Validation)
        .reason("another presentation image is still being made or stopping")
        .fix_hint("Export the image again once the other one has finished or stopped.")
        .into()
}

/// Percent opaque as a channel value, 100 % = 255.
fn opacity_of(percent: u8) -> u8 {
    u8::try_from(u16::from(percent.min(100)) * 255 / 100).unwrap_or(u8::MAX)
}

impl KetchupApp {
    /// The visible parts of the current model with the opacity of their material. A material
    /// `request` names that no visible part is of is refused, so a misspelt one is not ignored.
    pub(crate) fn presentation_parts(
        &self,
        request: &PresentationRequest,
    ) -> Result<PresentationParts, ExportError> {
        let snapshot = self.document.current();
        let scene = self.current_visible_mesh_scene(&snapshot)?;
        let items = BomItems::new(&snapshot).map_err(ExportError::failed)?;
        let mut used = BTreeSet::new();
        let mut opacity = Vec::with_capacity(scene.len());
        for body in &scene {
            let item = items
                .item(&body.occurrence.instance_path)
                .map_err(ExportError::failed)?;
            let material = items.material_name(&item).map_err(ExportError::failed)?;
            let percent = material.and_then(|name| {
                let percent = request.opacity_percent.get(name)?;
                used.insert(name.to_owned());
                Some(*percent)
            });
            opacity.push(percent.map_or(u8::MAX, opacity_of));
        }
        let unknown = request
            .opacity_percent
            .keys()
            .filter(|name| !used.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(
                Rejection::new("presentation.material", RejectionPhase::Validation)
                    .target(unknown.join(", "))
                    .reason("no visible part is of this material")
                    .fix_hint(
                        "Name the materials of visible parts as the bill of materials lists them.",
                    )
                    .into(),
            );
        }
        Ok(PresentationParts {
            snapshot,
            blobs: self.file.container_data.blobs().clone(),
            scene,
            opacity,
            decimal_mark: crate::app::project_drawings::sheet_decimal_mark(&self.catalog),
        })
    }

    /// Starts a client's image on a thread that stops once `cancelled` is set; the live bridge
    /// hands what it made to [`Self::finish_client_presentation`]. A refusal before the thread
    /// starts is shown in the window too, as one after it is.
    pub(crate) fn start_client_presentation(
        &mut self,
        path: &Path,
        request: &PresentationRequest,
        cancelled: Arc<AtomicBool>,
        context: &egui::Context,
    ) -> Result<Receiver<MadePresentation>, ExportError> {
        self.side_effect_receipts.clear();
        let started = presentation_destination(path)
            .and_then(|()| self.presentation_parts(request))
            .and_then(|parts| {
                parts.spawn(
                    request.clone(),
                    cancelled,
                    context.clone(),
                    &self.file.activity.presentation_slot,
                )
            });
        if let Err(error) = &started {
            self.show_export_outcome("presentation", path, Err(error));
        }
        started
    }

    /// Writes the image a client's thread `made` as that client's export; with `overwrite` it
    /// may replace a file once the user agrees in the window.
    pub(crate) fn finish_client_presentation(
        &mut self,
        path: &Path,
        made: MadePresentation,
        cancelled: &Arc<AtomicBool>,
        overwrite: bool,
    ) -> Result<Written, ExportError> {
        self.live.client_export = Some(Arc::clone(cancelled));
        self.live.client_overwrite = overwrite;
        let result = made.and_then(|png| self.publish_presentation(path, &png));
        self.live.client_export = None;
        self.live.client_overwrite = false;
        self.report_export_outcome("presentation", path, result)
    }

    /// Starts making the image on a thread; [`Self::poll_presentation_export`] writes it. A
    /// refusal before the thread starts is shown and returned.
    pub(crate) fn start_presentation_export(
        &mut self,
        path: PathBuf,
        request: &PresentationRequest,
        context: &egui::Context,
    ) -> Result<(), ExportError> {
        self.side_effect_receipts.clear();
        let cancelled = Arc::new(AtomicBool::new(false));
        let receiver = presentation_destination(&path)
            .and_then(|()| self.presentation_parts(request))
            .and_then(|parts| {
                parts.spawn(
                    request.clone(),
                    Arc::clone(&cancelled),
                    context.clone(),
                    &self.file.activity.presentation_slot,
                )
            });
        let receiver = match receiver {
            Ok(receiver) => receiver,
            Err(error) => {
                self.show_export_outcome("presentation", &path, Err(&error));
                return Err(error);
            }
        };
        self.file.activity.presentation_export = Some(PresentationTask {
            path,
            cancelled,
            receiver,
        });
        context.request_repaint();
        Ok(())
    }

    /// Stops the image's thread at its next row; nothing is written.
    pub(super) fn cancel_presentation_export(&mut self) {
        if let Some(task) = self.file.activity.presentation_export.take() {
            task.cancelled.store(true, Ordering::Release);
            self.digest = self.catalog.text("digest-cancelled");
        }
    }

    pub(super) fn poll_presentation_export(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.presentation_export.take() else {
            return;
        };
        let made = match task.receiver.try_recv() {
            Ok(made) => made,
            Err(TryRecvError::Empty) => {
                self.file.activity.presentation_export = Some(task);
                return;
            }
            Err(error @ TryRecvError::Disconnected) => Err(ExportError::failed(error)),
        };
        let result = made.and_then(|png| self.publish_presentation(&task.path, &png));
        self.show_export_outcome("presentation", &task.path, result.as_ref());
        context.request_repaint();
    }

    /// Writes the image. A picture loses nothing of the model, so only replacing a file asks
    /// the user; a client's image replaces one only with `overwrite` and once the user agrees
    /// (review 2026-11-12 P3: `overwrite` was ignored).
    fn publish_presentation(&mut self, path: &Path, png: &[u8]) -> Result<Written, ExportError> {
        let existing = export_target_sha256(path)?;
        if existing.is_some() {
            if self.live.client_export.is_some() && !self.live.client_overwrite {
                return Err(ExportError::Destination {
                    requirement: "a new file name; a client's image replaces an existing file only with overwrite and once the user agrees",
                });
            }
            let title = self.catalog.text("dialog-export-overwrite-title");
            let risk = self.catalog.text("dialog-export-overwrite-risk");
            self.authorize_path_side_effect(
                HighRiskClass::Overwrite,
                "overwrite-presentation-image",
                &title,
                &risk,
                path,
                png,
            )?;
        }
        write_export_artifact_if_unchanged(path, png, existing.as_deref())?;
        Ok(Written::alone(path))
    }
}

fn presentation_destination(path: &Path) -> Result<(), ExportError> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        Ok(())
    } else {
        Err(ExportError::Destination {
            requirement: "an explicit .png destination",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R1112-IMG-502: Cancel only dropped the receiver and the thread rendered on; now it sets
    /// the flag the thread stops at, and a stopped image is no file.
    #[test]
    fn cancel_in_the_window_stops_the_images_thread() {
        let mut app = KetchupApp::new();
        crate::tests::install_initial_graph_result(&mut app);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cancelled.png");
        let context = egui::Context::default();
        app.start_presentation_export(path.clone(), &PresentationRequest::default(), &context)
            .unwrap();
        let cancelled = Arc::clone(
            &app.file
                .activity
                .presentation_export
                .as_ref()
                .unwrap()
                .cancelled,
        );
        app.cancel_presentation_export();
        assert!(cancelled.load(Ordering::Acquire));
        assert!(app.file.activity.presentation_export.is_none());

        let parts = app
            .presentation_parts(&PresentationRequest::default())
            .unwrap();
        let made = parts
            .spawn(
                PresentationRequest::default(),
                Arc::new(AtomicBool::new(true)),
                context,
                &PresentationSlot::default(),
            )
            .unwrap()
            .recv()
            .unwrap();
        let refused = made.expect_err("a cancelled image is not made").to_string();
        assert!(refused.contains("cancelled"), "{refused}");
        assert!(!path.exists());
    }

    /// R1113 P3: too many labels, an empty model, too many triangles or a broken texture
    /// reference all fell to a general `export_failed` that says nothing of what to change.
    #[test]
    fn every_refusal_the_user_can_act_on_says_what_to_change() {
        for error in [
            PresentationError::TooManyLabels {
                labels: 400,
                limit: 200,
            },
            PresentationError::EmptyModel,
            PresentationError::TooManyTriangles {
                triangles: 5_000_000,
                limit: 4_000_000,
            },
            PresentationError::TextureReference { triangle: 3 },
            PresentationError::LabelText {
                label: 2,
                limit: 200,
            },
            PresentationError::LabelLetter {
                label: 1,
                letter: '倉',
            },
            PresentationError::ViewDirection,
            PresentationError::Supersample { supersample: 9 },
        ] {
            let shown = error.to_string();
            let ExportError::Failed(cause) = presentation_refused(error) else {
                panic!("{shown} is not refused with its cause");
            };
            let rejection = cause
                .downcast_ref::<Rejection>()
                .unwrap_or_else(|| panic!("{shown} fell to a general failure"));
            assert_eq!(rejection.code(), "presentation.image", "{shown}");
            assert_eq!(rejection.reason_text(), shown);
            assert!(!rejection.fix_hint_text().is_empty(), "{shown}");
        }
    }

    /// R1113-IMG-600: a cancelled image freed its place at once while its thread ran on, so
    /// asking again started a second thread. Now a second image waits for the first thread.
    #[test]
    fn a_second_image_waits_until_the_first_ones_thread_has_ended() {
        let mut app = KetchupApp::new();
        crate::tests::install_initial_graph_result(&mut app);
        let slot = PresentationSlot::default();
        let running = Arc::clone(&slot.0);
        assert!(slot.busy());
        let parts = |app: &KetchupApp| {
            app.presentation_parts(&PresentationRequest::default())
                .unwrap()
        };
        let spawn = |app: &KetchupApp| {
            parts(app).spawn(
                PresentationRequest::default(),
                Arc::new(AtomicBool::new(false)),
                egui::Context::default(),
                &slot,
            )
        };
        let refused = spawn(&app).expect_err("refused while a thread runs");
        assert!(
            refused.to_string().contains("still being made"),
            "{refused}"
        );
        drop(running);
        let made = spawn(&app).unwrap().recv().unwrap();
        assert!(made.is_ok(), "{:?}", made.err());
        assert!(!slot.busy(), "the ended thread let its place go");

        // The window's image waits for a thread still running, and says why.
        let directory = tempfile::tempdir().unwrap();
        let context = egui::Context::default();
        let held = app.file.activity.presentation_slot.clone_held();
        let start = |app: &mut KetchupApp, name: &str| {
            app.start_presentation_export(
                directory.path().join(name),
                &PresentationRequest::default(),
                &context,
            )
        };
        assert!(start(&mut app, "second.png").is_err());
        assert!(app.file.activity.presentation_export.is_none());
        assert!(app.digest.contains("still being made"), "{}", app.digest);
        drop(held);
        start(&mut app, "free.png").unwrap();
        assert!(app.file.activity.presentation_export.is_some());
    }
}
