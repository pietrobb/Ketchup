//! STL, 3MF, GLB and DAE exports of the visible model. From the File menu the file and
//! its loss report are made on a thread, so the window keeps painting and the file activity
//! indication can cancel; consent is asked and the file written when they come back, so a
//! cancelled export writes nothing (review 2026-10-24 UI-28).

use crate::*;
use ketchup_manufacturing::blender_export::model_dae_export;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MeshFormat {
    Stl,
    ThreeMf,
    Glb,
    Dae,
}

/// A mesh file's bytes and its loss report.
pub(crate) struct MadeMesh {
    bytes: Vec<u8>,
    loss_report: String,
}

impl MeshFormat {
    pub(crate) fn of_command(command: AppCommand) -> Option<Self> {
        match command {
            AppCommand::ExportMeshStl => Some(Self::Stl),
            AppCommand::ExportPrintThreeMf => Some(Self::ThreeMf),
            AppCommand::ExportBlenderGlb => Some(Self::Glb),
            AppCommand::ExportSketchupDae => Some(Self::Dae),
            _ => None,
        }
    }

    pub(crate) fn command(self) -> AppCommand {
        match self {
            Self::Stl => AppCommand::ExportMeshStl,
            Self::ThreeMf => AppCommand::ExportPrintThreeMf,
            Self::Glb => AppCommand::ExportBlenderGlb,
            Self::Dae => AppCommand::ExportSketchupDae,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Stl => "stl",
            Self::ThreeMf => "3mf",
            Self::Glb => "glb",
            Self::Dae => "dae",
        }
    }

    fn evidence_domain(self) -> &'static [u8] {
        match self {
            Self::Stl => b"ketchup.current-model-stl-export.v1",
            Self::ThreeMf => b"ketchup.current-model-3mf-export.v1",
            Self::Glb => b"ketchup.current-model-blender-glb-export.v1",
            Self::Dae => b"ketchup.current-model-dae-export.v1",
        }
    }

    fn consent(self) -> ExportConsent<'static> {
        let lossy = |operation, overwrite_operations| ExportConsent {
            class: HighRiskClass::LossyConversion,
            operation,
            title_key: "dialog-export-lossy-title",
            risk_key: "dialog-export-lossy-risk",
            overwrite_title_key: "dialog-export-overwrite-title",
            overwrite_operations,
        };
        match self {
            Self::Stl => lossy(
                "export-current-model-stl-with-loss-report",
                [
                    "overwrite-current-model-stl-export",
                    "overwrite-current-model-stl-loss-report",
                ],
            ),
            Self::ThreeMf => lossy(
                "export-current-model-3mf-with-loss-report",
                [
                    "overwrite-current-model-3mf-export",
                    "overwrite-current-model-3mf-loss-report",
                ],
            ),
            Self::Glb => ExportConsent {
                title_key: "dialog-export-blender-title",
                risk_key: "dialog-export-blender-risk",
                ..lossy(
                    "export-current-model-blender-glb-with-loss-report",
                    [
                        "overwrite-current-model-blender-glb-export",
                        "overwrite-current-model-blender-glb-loss-report",
                    ],
                )
            },
            Self::Dae => ExportConsent {
                title_key: "dialog-export-sketchup-title",
                risk_key: "dialog-export-sketchup-risk",
                overwrite_title_key: "dialog-export-sketchup-overwrite-title",
                ..lossy(
                    "export-current-model-dae-with-loss-report",
                    [
                        "overwrite-current-model-dae-export",
                        "overwrite-current-model-dae-loss-report",
                    ],
                )
            },
        }
    }

    /// The file of `scene`, the visible meshes of `snapshot`.
    fn make(
        self,
        snapshot: &Snapshot,
        scene: &[CurrentVisibleMesh],
    ) -> Result<MadeMesh, ExportError> {
        match self {
            Self::Stl => {
                if scene.iter().any(|body| {
                    matches!(
                        body.source,
                        CurrentVisibleMeshSource::Exact(ref package)
                            if matches!(package.as_ref(), ExactBodyPackage::Imported(_))
                    )
                }) {
                    return Err(ExportError::UnsupportedContent {
                        reason: "imported STEP bounds proxies cannot be exported as STL",
                    });
                }
                let bodies = scene
                    .iter()
                    .map(|body| MeshExportBody {
                        source: body.source.as_export_source(),
                        transform: body.occurrence.transform,
                    })
                    .collect::<Vec<_>>();
                let bundle = model_stl_export(snapshot, &bodies).map_err(ExportError::failed)?;
                Ok(MadeMesh {
                    bytes: bundle.mesh_stl.into_bytes(),
                    loss_report: bundle.loss_report,
                })
            }
            Self::ThreeMf => {
                let instances = scene
                    .iter()
                    .map(|body| MeshThreeMfInstance {
                        source: body.source.as_export_source(),
                        occurrence: &body.occurrence,
                    })
                    .collect::<Vec<_>>();
                let bundle =
                    model_three_mf_export(snapshot, &instances).map_err(ExportError::failed)?;
                Ok(MadeMesh {
                    bytes: bundle.three_mf,
                    loss_report: bundle.loss_report,
                })
            }
            Self::Glb | Self::Dae => {
                let instances = scene
                    .iter()
                    .map(|body| MeshGlbInstance {
                        source: body.source.as_export_source(),
                        occurrence: &body.occurrence,
                    })
                    .collect::<Vec<_>>();
                if self == Self::Glb {
                    let bundle =
                        model_glb_export(snapshot, &instances).map_err(ExportError::failed)?;
                    Ok(MadeMesh {
                        bytes: bundle.glb,
                        loss_report: bundle.loss_report,
                    })
                } else {
                    let bundle =
                        model_dae_export(snapshot, &instances).map_err(ExportError::failed)?;
                    Ok(MadeMesh {
                        bytes: bundle.dae.into_bytes(),
                        loss_report: bundle.loss_report,
                    })
                }
            }
        }
    }
}

/// A mesh export whose file is being made on its own thread.
pub(crate) struct MeshExportTask {
    format: MeshFormat,
    path: PathBuf,
    receiver: Receiver<Result<MadeMesh, ExportError>>,
}

impl MeshExportTask {
    pub(crate) fn format(&self) -> MeshFormat {
        self.format
    }
}

impl KetchupApp {
    /// Makes the file and writes it, waiting; the live bridge and a window without a frame
    /// take this path.
    pub(crate) fn export_mesh_to(
        &mut self,
        format: MeshFormat,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let result = self
            .current_visible_mesh_scene(&snapshot)
            .and_then(|scene| format.make(&snapshot, &scene))
            .and_then(|made| self.publish_mesh_export(format, path, &made));
        self.report_export_outcome(format.key(), path, result)
    }

    /// Starts making the file on a thread; [`Self::poll_mesh_export`] writes it.
    pub(super) fn start_mesh_export(
        &mut self,
        format: MeshFormat,
        path: PathBuf,
        context: &egui::Context,
    ) {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        let scene = match self.current_visible_mesh_scene(&snapshot) {
            Ok(scene) => scene,
            Err(error) => {
                self.report_export_outcome(format.key(), &path, Err(error))
                    .ok();
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let repaint = context.clone();
        std::thread::spawn(move || {
            if sender.send(format.make(&snapshot, &scene)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.mesh_export = Some(MeshExportTask {
            format,
            path,
            receiver,
        });
        context.request_repaint();
    }

    /// Drops the export; its thread finishes, but nothing is written.
    pub(super) fn cancel_mesh_export(&mut self) {
        if self.file.activity.mesh_export.take().is_some() {
            self.digest = self.catalog.text("digest-cancelled");
        }
    }

    pub(super) fn poll_mesh_export(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.mesh_export.as_ref() else {
            return;
        };
        let made = match task.receiver.try_recv() {
            Ok(made) => made,
            Err(TryRecvError::Empty) => return,
            Err(error @ TryRecvError::Disconnected) => Err(ExportError::failed(error)),
        };
        let task = self
            .file
            .activity
            .mesh_export
            .take()
            .expect("polled export");
        let result = made.and_then(|made| self.publish_mesh_export(task.format, &task.path, &made));
        self.report_export_outcome(task.format.key(), &task.path, result)
            .ok();
        context.request_repaint();
    }

    /// Asks for consent and writes the file with its loss report.
    fn publish_mesh_export(
        &mut self,
        format: MeshFormat,
        path: &Path,
        made: &MadeMesh,
    ) -> Result<Written, ExportError> {
        let report_path = path.with_extension(format!("{}.loss.txt", format.key()));
        let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
        let evidence = super::export::export_bundle_evidence(
            format.evidence_domain(),
            path,
            &made.bytes,
            &report_path,
            made.loss_report.as_bytes(),
        );
        self.authorize_export_bundle(
            format.consent(),
            path,
            &report_path,
            &precondition,
            &evidence,
        )?;
        write_export_bundle(
            path,
            &made.bytes,
            &report_path,
            made.loss_report.as_bytes(),
            &precondition,
        )?;
        Ok(Written::with_loss_report(path, &report_path))
    }
}
