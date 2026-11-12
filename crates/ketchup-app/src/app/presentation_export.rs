//! File > Export > Presentation image: a still PNG of the visible model for a client or a
//! quotation, with the background, size and see-through materials the user chose. The modelling
//! viewport is unchanged. From the window the image is made on a thread, so the window keeps
//! painting and the file activity indication can cancel; a client's request waits for it.

use crate::*;
use ketchup_manufacturing::blender_export::MeshGlbInstance;
use ketchup_manufacturing::presentation::{
    PresentationLabel, PresentationOptions, part_labels, presentation_scene, render_presentation,
};
use ketchup_model::drawing::BomItems;
use std::sync::mpsc::{self, Receiver, TryRecvError};

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

/// A presentation image being made on its own thread.
pub(crate) struct PresentationTask {
    path: PathBuf,
    receiver: Receiver<Result<Vec<u8>, ExportError>>,
}

/// The visible meshes of a snapshot with the opacity (0–255) of each.
pub(crate) struct PresentationParts {
    snapshot: Snapshot,
    /// The document's blobs, for the pictures textured faces show.
    blobs: ketchup_model::persistence::Blobs,
    scene: Vec<CurrentVisibleMesh>,
    opacity: Vec<u8>,
}

impl PresentationParts {
    /// The PNG of these parts as `request` says. A panic in the renderer fails this image
    /// instead of the window (R1112-IMG-500).
    fn make(&self, request: &PresentationRequest) -> Result<Vec<u8>, ExportError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.render(request)))
            .unwrap_or_else(|payload| {
                Err(ExportError::failed(format!(
                    "the presentation renderer stopped: {}",
                    crate::bounded_parser::panic_message(payload.as_ref())
                )))
            })
    }

    fn render(&self, request: &PresentationRequest) -> Result<Vec<u8>, ExportError> {
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
        .map_err(ExportError::failed)?;
        if request.part_labels {
            scene.labels = part_labels(&instances);
        }
        scene.labels.extend(request.labels.iter().cloned());
        render_presentation(&scene, &request.options)
            .and_then(|image| image.png())
            .map_err(ExportError::failed)
    }
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
        })
    }

    /// Makes the image and writes it, waiting; the live bridge takes this path.
    pub(crate) fn export_presentation_image_to(
        &mut self,
        path: &Path,
        request: &PresentationRequest,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let result = presentation_destination(path)
            .and_then(|()| self.presentation_parts(request))
            .and_then(|parts| parts.make(request))
            .and_then(|png| self.publish_presentation(path, &png));
        self.report_export_outcome("presentation", path, result)
    }

    /// Starts making the image on a thread; [`Self::poll_presentation_export`] writes it.
    pub(crate) fn start_presentation_export(
        &mut self,
        path: PathBuf,
        request: &PresentationRequest,
        context: &egui::Context,
    ) {
        self.side_effect_receipts.clear();
        let parts = presentation_destination(&path).and_then(|()| self.presentation_parts(request));
        let parts = match parts {
            Ok(parts) => parts,
            Err(error) => {
                self.show_export_outcome("presentation", &path, Err(&error));
                return;
            }
        };
        let request = request.clone();
        let (sender, receiver) = mpsc::channel();
        let repaint = context.clone();
        std::thread::spawn(move || {
            if sender.send(parts.make(&request)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.presentation_export = Some(PresentationTask { path, receiver });
        context.request_repaint();
    }

    /// Drops the image; its thread finishes, but nothing is written.
    pub(super) fn cancel_presentation_export(&mut self) {
        if self.file.activity.presentation_export.take().is_some() {
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
    /// the user; a client never replaces one.
    fn publish_presentation(&mut self, path: &Path, png: &[u8]) -> Result<Written, ExportError> {
        let existing = export_target_sha256(path)?;
        if existing.is_some() {
            if self.live.client_export.is_some() {
                return Err(ExportError::Destination {
                    requirement: "a new file name; a client export never replaces an existing file",
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
