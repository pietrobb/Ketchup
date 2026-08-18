use super::*;
use ketchup_core::sketch::PrincipalPlane;

#[cfg(debug_assertions)]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeadlessFaceWorkflowFailure {
    FailedEvaluation,
    Ambiguous,
    Lost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FaceWorkflowDatum {
    Xy,
    Xz,
    Yz,
}

impl FaceWorkflowDatum {
    const ALL: [Self; 3] = [Self::Xy, Self::Xz, Self::Yz];

    const fn label_key(self) -> &'static str {
        match self {
            Self::Xy => "face-workflow-datum-xy",
            Self::Xz => "face-workflow-datum-xz",
            Self::Yz => "face-workflow-datum-yz",
        }
    }

    pub(super) const fn plane(self) -> PrincipalPlane {
        match self {
            Self::Xy => PrincipalPlane::Xy,
            Self::Xz => PrincipalPlane::Xz,
            Self::Yz => PrincipalPlane::Yz,
        }
    }
}

#[derive(Debug)]
pub(super) struct FaceWorkflowUiState {
    datum: FaceWorkflowDatum,
    snaps_enabled: bool,
    xray_preview: bool,
    #[cfg(debug_assertions)]
    headless_failure: Option<HeadlessFaceWorkflowFailure>,
}

impl Default for FaceWorkflowUiState {
    fn default() -> Self {
        Self {
            datum: FaceWorkflowDatum::Xy,
            snaps_enabled: true,
            xray_preview: false,
            #[cfg(debug_assertions)]
            headless_failure: None,
        }
    }
}

impl FaceWorkflowUiState {
    pub(super) const fn snaps_enabled(&self) -> bool {
        self.snaps_enabled
    }

    pub(super) const fn xray_preview(&self) -> bool {
        self.xray_preview
    }

    pub(super) fn set_xray_preview(&mut self, active: bool) {
        self.xray_preview = active;
    }

    #[cfg(debug_assertions)]
    pub(super) fn take_headless_failure(&mut self) -> Option<HeadlessFaceWorkflowFailure> {
        self.headless_failure.take()
    }
}

impl KetchupApp {
    pub(super) fn show_face_workflow_ui(&mut self, ui: &mut egui::Ui) {
        if !matches!(
            self.active_tool,
            ActiveTool::Rectangle | ActiveTool::PushPull
        ) {
            return;
        }
        let title = self.catalog.text("face-workflow-title");
        let datum_label = self.catalog.text("face-workflow-datum");
        let snap_label = self.catalog.text("face-workflow-snaps");
        let target = self.hover_readout();
        let diagnostic = self.digest.clone();
        let xray = self.face_workflow.xray_preview;
        let overlap = self.hovered_overlap_choice();
        let mut datum = self.face_workflow.datum;
        let mut snaps_enabled = self.face_workflow.snaps_enabled;

        egui::CollapsingHeader::new(title)
            .id_salt("face-workflow")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(datum_label);
                ui.horizontal(|ui| {
                    for choice in FaceWorkflowDatum::ALL {
                        ui.radio_value(&mut datum, choice, self.catalog.text(choice.label_key()));
                    }
                });
                ui.checkbox(&mut snaps_enabled, snap_label);
                ui.label(self.catalog.format(
                    "face-workflow-target",
                    &BTreeMap::from([("target", target)]),
                ));
                if xray {
                    let (index, count) = overlap.unwrap_or((0, 0));
                    ui.label(self.catalog.format(
                        "face-workflow-xray",
                        &BTreeMap::from([
                            ("index", (index + 1).to_string()),
                            ("count", count.to_string()),
                        ]),
                    ));
                } else {
                    ui.label(self.catalog.text("face-workflow-pick-through-hint"));
                }
                ui.label(self.catalog.format(
                    "face-workflow-diagnostic",
                    &BTreeMap::from([("message", diagnostic)]),
                ));
            });

        if datum != self.face_workflow.datum {
            self.face_workflow.datum = datum;
            self.digest = self.catalog.format(
                "digest-face-workflow-datum",
                &BTreeMap::from([("datum", self.catalog.text(datum.label_key()))]),
            );
        }
        if snaps_enabled != self.face_workflow.snaps_enabled {
            self.face_workflow.snaps_enabled = snaps_enabled;
            self.snap_tracker.clear();
            self.hover_snap = None;
            self.digest = self.catalog.text(if snaps_enabled {
                "digest-face-workflow-snaps-on"
            } else {
                "digest-face-workflow-snaps-off"
            });
        }
    }

    #[must_use]
    pub fn face_workflow_datum(&self) -> PrincipalPlane {
        self.face_workflow.datum.plane()
    }

    #[must_use]
    pub const fn face_workflow_snaps_enabled(&self) -> bool {
        self.face_workflow.snaps_enabled
    }

    #[must_use]
    pub const fn face_workflow_xray_active(&self) -> bool {
        self.face_workflow.xray_preview
    }

    #[must_use]
    pub fn face_workflow_target_feedback(&self) -> String {
        self.hover_readout()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn arm_headless_face_workflow_failure(&mut self, failure: HeadlessFaceWorkflowFailure) {
        self.face_workflow.headless_failure = Some(failure);
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn seed_headless_face_workflow_last_valid_output(&mut self) -> Result<(), String> {
        use ketchup_core::exact_product::{
            build_box_render_package, canonical_reference_lineage_digest,
        };

        let snapshot = self.document.current();
        let request = ExactFeatureChainRequest::from_snapshot(&snapshot, INITIAL_BOX_DEFINITION)
            .map_err(|error| error.to_string())?;
        let evidence = [
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::East,
        ]
        .map(|role| {
            (
                role,
                canonical_reference_lineage_digest(
                    request.document_id,
                    request.producer_feature_id(),
                    role.semantic_role(),
                    role.source_element_id(),
                    role.expected_type(),
                ),
                format!("headless-geometry-{role:?}"),
            )
        });
        let package = build_box_render_package(
            &request,
            "headless-exact-input".to_owned(),
            "headless-last-valid-result".to_owned(),
            "headless-backend".to_owned(),
            "headless-tolerance".to_owned(),
            [[0.0; 3], request.dimensions_mm()],
            evidence,
        )
        .map(ExactBodyPackage::from)
        .map(Arc::new)
        .map_err(|error| error.to_string())?;
        if let Some(task) = self.exact_task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        self.exact_results =
            ExactResultRegistry::accept(&snapshot, [package]).map_err(|error| error.to_string())?;
        self.exact_worker_attempted = true;
        self.exact_worker_path = None;
        self.exact_source = Some((
            snapshot.document_id(),
            snapshot.revision_id(),
            snapshot.canonical_digest(),
        ));
        Ok(())
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub fn headless_face_workflow_exact_output_stamp(&self) -> u64 {
        self.exact_results.contents_stamp()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub fn headless_face_workflow_exact_output_fingerprints(&self) -> Vec<String> {
        self.exact_results
            .values()
            .map(|package| package.result_key().result_fingerprint)
            .collect()
    }
}
