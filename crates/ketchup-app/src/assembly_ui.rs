use super::*;
use ketchup_core::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
    AssemblyRecomputePublishError, AssemblyRecomputeStatus, AssemblySolveResult,
    AssemblySolveStatus, AssemblySolverPolicy, recompute_rigid_assembly, solve_rigid_assembly,
};
use ketchup_core::exact_product::BodySubshapeRef;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MateKindChoice {
    #[default]
    CoincidentPlanar,
    ConcentricAxial,
    Distance,
    Angle,
}

impl MateKindChoice {
    const ALL: [Self; 4] = [
        Self::CoincidentPlanar,
        Self::ConcentricAxial,
        Self::Distance,
        Self::Angle,
    ];

    const fn label_key(self) -> &'static str {
        match self {
            Self::CoincidentPlanar => "assembly-mate-coincident",
            Self::ConcentricAxial => "assembly-mate-concentric",
            Self::Distance => "assembly-mate-distance",
            Self::Angle => "assembly-mate-angle",
        }
    }

    const fn needs_value(self) -> bool {
        !matches!(self, Self::ConcentricAxial)
    }

    const fn requires_planar_faces(self) -> bool {
        !matches!(self, Self::ConcentricAxial)
    }
}

struct AssemblyProposalPreview {
    proposal: Proposal,
    action: String,
    clear_occurrence_name: bool,
}

#[derive(Default)]
pub(super) struct AssemblyEditorState {
    definition: Option<DefinitionId>,
    occurrence_name: String,
    endpoint_a: Option<OccurrenceId>,
    endpoint_b: Option<OccurrenceId>,
    reference_a: Option<String>,
    reference_b: Option<String>,
    kind: MateKindChoice,
    value_input: String,
    reversed: bool,
    selected_mate: Option<AssemblyMateId>,
    preview: Option<AssemblyProposalPreview>,
    solve_result: Option<AssemblySolveResult>,
}

enum AssemblyUiAction {
    Insert,
    Ground(OccurrenceId, bool),
    SelectEndpointA(OccurrenceId),
    SelectEndpointB(OccurrenceId),
    EditMate(AssemblyMateId),
    RemoveMate(AssemblyMateId),
    PreviewMate,
    Solve,
}

impl KetchupApp {
    fn assembly_references(
        &self,
        snapshot: &Snapshot,
        occurrence_id: OccurrenceId,
        kind: MateKindChoice,
    ) -> Vec<BodySubshapeRef> {
        let Some(occurrence) = snapshot.occurrence(occurrence_id) else {
            return Vec::new();
        };
        let Some(package) = self
            .exact_results
            .get_render(snapshot, occurrence.definition_id())
        else {
            return Vec::new();
        };
        let mut references = package
            .references()
            .iter()
            .filter(|reference| {
                !kind.requires_planar_faces() || reference.expected_type == "planar_face"
            })
            .cloned()
            .collect::<Vec<_>>();
        references.sort_by(|left, right| {
            left.semantic_role
                .cmp(&right.semantic_role)
                .then(left.source_element_id.cmp(&right.source_element_id))
                .then(left.lineage_digest.cmp(&right.lineage_digest))
        });
        references
    }

    fn reconcile_assembly_editor(&mut self, snapshot: &Snapshot) {
        let definitions = snapshot
            .definitions()
            .map(|definition| definition.id())
            .collect::<Vec<_>>();
        if self
            .assembly_editor
            .definition
            .is_none_or(|id| !definitions.contains(&id))
        {
            self.assembly_editor.definition = definitions.first().copied();
            self.assembly_editor.occurrence_name.clear();
        }
        let occurrences = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id())
            .collect::<Vec<_>>();
        if self
            .assembly_editor
            .endpoint_a
            .is_none_or(|id| !occurrences.contains(&id))
        {
            self.assembly_editor.endpoint_a = occurrences.first().copied();
            self.assembly_editor.reference_a = None;
        }
        if self
            .assembly_editor
            .endpoint_b
            .is_none_or(|id| !occurrences.contains(&id))
        {
            self.assembly_editor.endpoint_b = occurrences.get(1).copied();
            self.assembly_editor.reference_b = None;
        }
        if self
            .assembly_editor
            .selected_mate
            .is_some_and(|id| snapshot.assembly_mate(id).is_none())
        {
            self.assembly_editor.selected_mate = None;
        }
        if self.assembly_editor.value_input.is_empty() {
            self.assembly_editor.value_input = "0".to_owned();
        }

        let kind = self.assembly_editor.kind;
        let references_a = self
            .assembly_editor
            .endpoint_a
            .map_or_else(Vec::new, |id| self.assembly_references(snapshot, id, kind));
        if self
            .assembly_editor
            .reference_a
            .as_ref()
            .is_none_or(|digest| {
                !references_a
                    .iter()
                    .any(|reference| &reference.lineage_digest == digest)
            })
        {
            self.assembly_editor.reference_a = references_a
                .first()
                .map(|reference| reference.lineage_digest.clone());
        }
        let references_b = self
            .assembly_editor
            .endpoint_b
            .map_or_else(Vec::new, |id| self.assembly_references(snapshot, id, kind));
        if self
            .assembly_editor
            .reference_b
            .as_ref()
            .is_none_or(|digest| {
                !references_b
                    .iter()
                    .any(|reference| &reference.lineage_digest == digest)
            })
        {
            self.assembly_editor.reference_b = references_b
                .first()
                .map(|reference| reference.lineage_digest.clone());
        }
    }

    fn assembly_error(&mut self, reason: impl ToString) {
        self.digest = self.catalog.format(
            "assembly-error",
            &BTreeMap::from([("reason", reason.to_string())]),
        );
    }

    fn prepare_assembly_preview(
        &mut self,
        batch: CommandBatch,
        action_key: &'static str,
        clear_occurrence_name: bool,
    ) -> bool {
        let proposal = match self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
        {
            Ok(proposal) => proposal,
            Err(error) => {
                self.assembly_error(error);
                return false;
            }
        };
        let preview_snapshot = match self.document.preview_batch(proposal.batch()) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.assembly_error(error);
                return false;
            }
        };
        let solve_result = if preview_snapshot.assembly_mates().next().is_some() {
            match solve_rigid_assembly(&preview_snapshot, AssemblySolverPolicy::default()) {
                Ok(result)
                    if matches!(
                        result.status(),
                        AssemblySolveStatus::UnderConstrained
                            | AssemblySolveStatus::FullyConstrained
                    ) =>
                {
                    Some(result)
                }
                Ok(result) => {
                    self.assembly_editor.solve_result = Some(result.clone());
                    self.assembly_error(self.catalog.text("assembly-error-solve-refused"));
                    return false;
                }
                Err(error) => {
                    self.assembly_error(error);
                    return false;
                }
            }
        } else {
            None
        };
        let action = self.catalog.text(action_key);
        self.assembly_editor.preview = Some(AssemblyProposalPreview {
            proposal,
            action: action.clone(),
            clear_occurrence_name,
        });
        self.assembly_editor.solve_result = solve_result;
        self.digest = self.catalog.format(
            "assembly-preview-ready",
            &BTreeMap::from([("action", action)]),
        );
        true
    }

    fn preview_insert_occurrence(&mut self) -> bool {
        let snapshot = self.document.current();
        let Some(definition_id) = self.assembly_editor.definition else {
            self.assembly_error(self.catalog.text("assembly-error-definition"));
            return false;
        };
        if snapshot.definition(definition_id).is_none() {
            self.assembly_error(self.catalog.text("assembly-error-definition"));
            return false;
        }
        let name = self.assembly_editor.occurrence_name.trim().to_owned();
        if name.is_empty() {
            self.assembly_error(self.catalog.text("assembly-error-name"));
            return false;
        }
        let id = OccurrenceId(
            snapshot
                .occurrences()
                .map(|occurrence| occurrence.id().0)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let x = snapshot.occurrences().count() as f64 * 25.0;
        let transform = Transform::from_translation(x, 0.0, 0.0)
            .expect("bounded insertion translation is finite");
        self.prepare_assembly_preview(
            CommandBatch::new(vec![CanonicalCommand::CreateOccurrence {
                id,
                definition_id,
                name,
                transform,
                parent: None,
                tag: None,
                visible: true,
            }]),
            "assembly-action-insert",
            true,
        )
    }

    fn preview_ground_occurrence(&mut self, id: OccurrenceId, grounded: bool) -> bool {
        self.prepare_assembly_preview(
            CommandBatch::new(vec![CanonicalCommand::SetOccurrenceGrounded {
                id,
                grounded,
            }]),
            if grounded {
                "assembly-action-ground"
            } else {
                "assembly-action-unground"
            },
            false,
        )
    }

    fn selected_assembly_reference(
        &self,
        snapshot: &Snapshot,
        occurrence_id: OccurrenceId,
        digest: &str,
    ) -> Option<BodySubshapeRef> {
        self.assembly_references(snapshot, occurrence_id, self.assembly_editor.kind)
            .into_iter()
            .find(|reference| reference.lineage_digest == digest)
    }

    fn editor_mate_kind(&self) -> Option<AssemblyMateKind> {
        let value = self
            .assembly_editor
            .value_input
            .trim()
            .parse::<f64>()
            .ok()?;
        let kind = match self.assembly_editor.kind {
            MateKindChoice::CoincidentPlanar => AssemblyMateKind::CoincidentPlanar {
                offset_mm: value,
                reversed: self.assembly_editor.reversed,
            },
            MateKindChoice::ConcentricAxial => AssemblyMateKind::ConcentricAxial {
                reversed: self.assembly_editor.reversed,
            },
            MateKindChoice::Distance => AssemblyMateKind::Distance { distance_mm: value },
            MateKindChoice::Angle => AssemblyMateKind::Angle {
                angle_degrees: value,
            },
        };
        kind.is_valid().then_some(kind)
    }

    fn preview_editor_mate(&mut self) -> bool {
        let snapshot = self.document.current();
        let (Some(occurrence_a), Some(occurrence_b)) = (
            self.assembly_editor.endpoint_a,
            self.assembly_editor.endpoint_b,
        ) else {
            self.assembly_error(self.catalog.text("assembly-error-endpoints"));
            return false;
        };
        if occurrence_a == occurrence_b {
            self.assembly_error(self.catalog.text("assembly-error-endpoints"));
            return false;
        }
        let (Some(reference_a_digest), Some(reference_b_digest)) = (
            self.assembly_editor.reference_a.as_deref(),
            self.assembly_editor.reference_b.as_deref(),
        ) else {
            self.assembly_error(self.catalog.text("assembly-error-references"));
            return false;
        };
        let Some(reference_a) =
            self.selected_assembly_reference(&snapshot, occurrence_a, reference_a_digest)
        else {
            self.assembly_error(self.catalog.text("assembly-error-references"));
            return false;
        };
        let Some(reference_b) =
            self.selected_assembly_reference(&snapshot, occurrence_b, reference_b_digest)
        else {
            self.assembly_error(self.catalog.text("assembly-error-references"));
            return false;
        };
        let Some(kind) = self.editor_mate_kind() else {
            self.assembly_error(self.catalog.text("assembly-error-value"));
            return false;
        };
        let id = self.assembly_editor.selected_mate.unwrap_or_else(|| {
            AssemblyMateId(
                snapshot
                    .assembly_mates()
                    .map(|mate| mate.id().0)
                    .max()
                    .unwrap_or(0)
                    + 1,
            )
        });
        let mate = AssemblyMate::new(
            id,
            AssemblyMateEndpoint::resolved(occurrence_a, reference_a),
            AssemblyMateEndpoint::resolved(occurrence_b, reference_b),
            kind,
        );
        let commands = if let Some(existing) = snapshot.assembly_mate(id) {
            if existing.endpoint_a() == mate.endpoint_a()
                && existing.endpoint_b() == mate.endpoint_b()
            {
                vec![CanonicalCommand::SetAssemblyMateKind { id, kind }]
            } else {
                vec![
                    CanonicalCommand::DeleteAssemblyMate { id },
                    CanonicalCommand::CreateAssemblyMate(mate),
                ]
            }
        } else {
            vec![CanonicalCommand::CreateAssemblyMate(mate)]
        };
        self.prepare_assembly_preview(
            CommandBatch::new(commands),
            if self.assembly_editor.selected_mate.is_some() {
                "assembly-action-edit-mate"
            } else {
                "assembly-action-create-mate"
            },
            false,
        )
    }

    fn edit_assembly_mate(&mut self, id: AssemblyMateId) {
        let snapshot = self.document.current();
        let Some(mate) = snapshot.assembly_mate(id) else {
            return;
        };
        self.assembly_editor.selected_mate = Some(id);
        self.assembly_editor.endpoint_a = Some(mate.endpoint_a().occurrence_id());
        self.assembly_editor.endpoint_b = Some(mate.endpoint_b().occurrence_id());
        self.assembly_editor.reference_a =
            Some(mate.endpoint_a().reference().lineage_digest.clone());
        self.assembly_editor.reference_b =
            Some(mate.endpoint_b().reference().lineage_digest.clone());
        match mate.kind() {
            AssemblyMateKind::CoincidentPlanar {
                offset_mm,
                reversed,
            } => {
                self.assembly_editor.kind = MateKindChoice::CoincidentPlanar;
                self.assembly_editor.value_input = format_height(offset_mm);
                self.assembly_editor.reversed = reversed;
            }
            AssemblyMateKind::ConcentricAxial { reversed } => {
                self.assembly_editor.kind = MateKindChoice::ConcentricAxial;
                self.assembly_editor.value_input = "0".to_owned();
                self.assembly_editor.reversed = reversed;
            }
            AssemblyMateKind::Distance { distance_mm } => {
                self.assembly_editor.kind = MateKindChoice::Distance;
                self.assembly_editor.value_input = format_height(distance_mm);
                self.assembly_editor.reversed = false;
            }
            AssemblyMateKind::Angle { angle_degrees } => {
                self.assembly_editor.kind = MateKindChoice::Angle;
                self.assembly_editor.value_input = format_height(angle_degrees);
                self.assembly_editor.reversed = false;
            }
        }
    }

    fn preview_remove_assembly_mate(&mut self, id: AssemblyMateId) -> bool {
        self.prepare_assembly_preview(
            CommandBatch::new(vec![CanonicalCommand::DeleteAssemblyMate { id }]),
            "assembly-action-remove-mate",
            false,
        )
    }

    fn preview_assembly_solve(&mut self) -> bool {
        let recomputed = match recompute_rigid_assembly(
            &self.document,
            &self.exact_results,
            AssemblySolverPolicy::default(),
        ) {
            Ok(result) => result,
            Err(error) => {
                self.assembly_error(error);
                return false;
            }
        };
        self.assembly_editor.solve_result = recomputed.solve().cloned();
        if recomputed.status() != AssemblyRecomputeStatus::Solved {
            self.assembly_error(self.catalog.text("assembly-error-solve-refused"));
            return false;
        }
        let proposal = match recomputed.prepare_publication(&self.document) {
            Ok(proposal) => proposal,
            Err(AssemblyRecomputePublishError::NoCanonicalChanges) => {
                self.digest = self.catalog.text("assembly-solve-current");
                return true;
            }
            Err(error) => {
                self.assembly_error(error);
                return false;
            }
        };
        let action = self.catalog.text("assembly-action-solve");
        self.assembly_editor.preview = Some(AssemblyProposalPreview {
            proposal,
            action: action.clone(),
            clear_occurrence_name: false,
        });
        self.digest = self.catalog.format(
            "assembly-preview-ready",
            &BTreeMap::from([("action", action)]),
        );
        true
    }

    fn confirm_assembly_preview(&mut self) -> bool {
        let Some(preview) = self.assembly_editor.preview.take() else {
            return false;
        };
        match self.document.commit_verified_proposal(&preview.proposal) {
            Ok(_) => {
                if preview.clear_occurrence_name {
                    self.assembly_editor.occurrence_name.clear();
                }
                self.assembly_editor.selected_mate = None;
                self.assembly_editor.solve_result = None;
                self.reconcile_selection();
                self.digest = self.catalog.format(
                    "assembly-committed",
                    &BTreeMap::from([("action", preview.action)]),
                );
                true
            }
            Err(error) => {
                self.assembly_error(error);
                false
            }
        }
    }

    fn cancel_assembly_preview(&mut self) {
        self.assembly_editor.preview = None;
        self.assembly_editor.solve_result = None;
        self.digest = self.catalog.text("assembly-cancelled");
    }

    fn show_assembly_solve_diagnostic(&self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let Some(result) = self.assembly_editor.solve_result.as_ref() else {
            return;
        };
        let source_is_current = result.document_id() == snapshot.document_id()
            && result.source_revision() == snapshot.revision_id()
            && result.source_digest() == snapshot.canonical_digest();
        let status = self.catalog.text(match result.status() {
            AssemblySolveStatus::UnderConstrained => "assembly-solve-under",
            AssemblySolveStatus::FullyConstrained => "assembly-solve-fully",
            AssemblySolveStatus::OverConstrained => "assembly-solve-over",
            AssemblySolveStatus::Failed => "assembly-solve-failed",
        });
        ui.label(self.catalog.format(
            "assembly-solve-summary",
            &BTreeMap::from([
                ("status", status),
                ("dof", result.remaining_dof().to_string()),
                ("current", source_is_current.to_string()),
            ]),
        ));
    }

    pub(super) fn show_assembly_editor(&mut self, ui: &mut egui::Ui) {
        let title = self.catalog.text("assembly-title");
        egui::CollapsingHeader::new(title)
            .id_salt("assembly-editor")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("assembly-editor-scroll")
                    .max_height(700.0)
                    .show(ui, |ui| self.show_assembly_editor_content(ui));
            });
        ui.separator();
    }

    fn show_assembly_editor_content(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.document.current();
        self.reconcile_assembly_editor(&snapshot);

        if let Some(preview) = self.assembly_editor.preview.as_ref() {
            ui.label(self.catalog.format(
                "assembly-preview-observational",
                &BTreeMap::from([("action", preview.action.clone())]),
            ));
            self.show_assembly_solve_diagnostic(ui, &snapshot);
            let confirm = ui
                .button(self.catalog.text("assembly-confirm-preview"))
                .clicked();
            let cancel = ui
                .button(self.catalog.text("assembly-cancel-preview"))
                .clicked();
            if confirm {
                self.confirm_assembly_preview();
            } else if cancel {
                self.cancel_assembly_preview();
            }
            ui.separator();
            return;
        }

        let definitions = snapshot
            .definitions()
            .map(|definition| (definition.id(), definition.name().to_owned()))
            .collect::<Vec<_>>();
        if let Some(mut selected) = self.assembly_editor.definition {
            let label = self.catalog.text("assembly-definition");
            let selected_name = definitions
                .iter()
                .find(|(id, _)| *id == selected)
                .map_or_else(|| selected.0.to_string(), |(_, name)| name.clone());
            let response = egui::ComboBox::from_id_salt("assembly-definition")
                .width(ui.available_width())
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    for (id, name) in &definitions {
                        ui.selectable_value(&mut selected, *id, name);
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
            });
            if self.assembly_editor.definition != Some(selected) {
                self.assembly_editor.definition = Some(selected);
                self.assembly_editor.occurrence_name.clear();
            }
        }
        if self.assembly_editor.occurrence_name.is_empty()
            && let Some(definition_id) = self.assembly_editor.definition
            && let Some(definition) = snapshot.definition(definition_id)
        {
            let count = snapshot
                .occurrences()
                .filter(|occurrence| occurrence.definition_id() == definition_id)
                .count()
                + 1;
            self.assembly_editor.occurrence_name = self.catalog.format(
                "assembly-default-occurrence-name",
                &BTreeMap::from([
                    ("definition", definition.name().to_owned()),
                    ("number", count.to_string()),
                ]),
            );
        }
        let name_label = self.catalog.text("assembly-occurrence-name");
        let name = ui.add(
            egui::TextEdit::singleline(&mut self.assembly_editor.occurrence_name)
                .hint_text(&name_label),
        );
        name.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &name_label)
        });
        let mut action = ui
            .button(self.catalog.text("assembly-preview-insert"))
            .clicked()
            .then_some(AssemblyUiAction::Insert);

        ui.label(self.catalog.text("assembly-occurrences"));
        for occurrence in snapshot.occurrences() {
            let id = occurrence.id();
            let grounded = snapshot.occurrence_is_grounded(id);
            let remaining = self
                .assembly_editor
                .solve_result
                .as_ref()
                .and_then(|result| result.occurrence(id))
                .map_or_else(
                    || {
                        if grounded {
                            "0".to_owned()
                        } else {
                            self.catalog.text("assembly-dof-pending")
                        }
                    },
                    |solved| solved.remaining_dof().to_string(),
                );
            ui.label(self.catalog.format(
                "assembly-occurrence-row",
                &BTreeMap::from([
                    ("name", occurrence.name().to_owned()),
                    ("id", id.0.to_string()),
                    ("dof", remaining),
                ]),
            ));
            ui.horizontal_wrapped(|ui| {
                let ground_key = if grounded {
                    "assembly-preview-unground"
                } else {
                    "assembly-preview-ground"
                };
                let ground_label = self.catalog.format(
                    ground_key,
                    &BTreeMap::from([("name", occurrence.name().to_owned())]),
                );
                if ui.button(ground_label).clicked() {
                    action = Some(AssemblyUiAction::Ground(id, !grounded));
                }
                let endpoint_a = self.catalog.format(
                    "assembly-use-endpoint-a",
                    &BTreeMap::from([("name", occurrence.name().to_owned())]),
                );
                if ui.button(endpoint_a).clicked() {
                    action = Some(AssemblyUiAction::SelectEndpointA(id));
                }
                let endpoint_b = self.catalog.format(
                    "assembly-use-endpoint-b",
                    &BTreeMap::from([("name", occurrence.name().to_owned())]),
                );
                if ui.button(endpoint_b).clicked() {
                    action = Some(AssemblyUiAction::SelectEndpointB(id));
                }
            });
        }

        let previous_kind = self.assembly_editor.kind;
        let kind_label = self.catalog.text("assembly-mate-kind");
        let kind_response = egui::ComboBox::from_id_salt("assembly-mate-kind")
            .width(ui.available_width())
            .selected_text(self.catalog.text(self.assembly_editor.kind.label_key()))
            .show_ui(ui, |ui| {
                for kind in MateKindChoice::ALL {
                    ui.selectable_value(
                        &mut self.assembly_editor.kind,
                        kind,
                        self.catalog.text(kind.label_key()),
                    );
                }
            });
        kind_response.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &kind_label)
        });
        if previous_kind != self.assembly_editor.kind {
            self.assembly_editor.reference_a = None;
            self.assembly_editor.reference_b = None;
        }
        if self.assembly_editor.kind.needs_value() {
            let value_label = self.catalog.text(match self.assembly_editor.kind {
                MateKindChoice::Angle => "assembly-angle-value",
                _ => "assembly-distance-value",
            });
            let value = ui.add(
                egui::TextEdit::singleline(&mut self.assembly_editor.value_input)
                    .hint_text(&value_label),
            );
            value.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &value_label)
            });
        }
        if matches!(
            self.assembly_editor.kind,
            MateKindChoice::CoincidentPlanar | MateKindChoice::ConcentricAxial
        ) {
            ui.checkbox(
                &mut self.assembly_editor.reversed,
                self.catalog.text("assembly-reversed"),
            );
        }

        let references_a = self.assembly_editor.endpoint_a.map_or_else(Vec::new, |id| {
            self.assembly_references(&snapshot, id, self.assembly_editor.kind)
        });
        let references_b = self.assembly_editor.endpoint_b.map_or_else(Vec::new, |id| {
            self.assembly_references(&snapshot, id, self.assembly_editor.kind)
        });
        for (side, references, selected) in [
            ("a", &references_a, &mut self.assembly_editor.reference_a),
            ("b", &references_b, &mut self.assembly_editor.reference_b),
        ] {
            let label_key = if side == "a" {
                "assembly-reference-a"
            } else {
                "assembly-reference-b"
            };
            let label = self.catalog.text(label_key);
            let selected_text = selected
                .as_ref()
                .and_then(|digest| {
                    references
                        .iter()
                        .find(|reference| &reference.lineage_digest == digest)
                })
                .map_or_else(
                    || self.catalog.text("assembly-reference-unavailable"),
                    |reference| reference.semantic_role.clone(),
                );
            let response = egui::ComboBox::from_id_salt(("assembly-reference", side))
                .width(ui.available_width())
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for reference in references {
                        ui.selectable_value(
                            selected,
                            Some(reference.lineage_digest.clone()),
                            &reference.semantic_role,
                        );
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label)
            });
        }

        let can_preview_mate = self.assembly_editor.endpoint_a.is_some()
            && self.assembly_editor.endpoint_b.is_some()
            && self.assembly_editor.reference_a.is_some()
            && self.assembly_editor.reference_b.is_some();
        if ui
            .add_enabled(
                can_preview_mate,
                egui::Button::new(self.catalog.text(
                    if self.assembly_editor.selected_mate.is_some() {
                        "assembly-preview-update-mate"
                    } else {
                        "assembly-preview-create-mate"
                    },
                )),
            )
            .clicked()
        {
            action = Some(AssemblyUiAction::PreviewMate);
        }

        for mate in snapshot.assembly_mates() {
            ui.horizontal_wrapped(|ui| {
                ui.label(self.catalog.format(
                    "assembly-mate-row",
                    &BTreeMap::from([("id", mate.id().0.to_string())]),
                ));
                if ui
                    .button(self.catalog.format(
                        "assembly-edit-mate",
                        &BTreeMap::from([("id", mate.id().0.to_string())]),
                    ))
                    .clicked()
                {
                    action = Some(AssemblyUiAction::EditMate(mate.id()));
                }
                if ui
                    .button(self.catalog.format(
                        "assembly-remove-mate",
                        &BTreeMap::from([("id", mate.id().0.to_string())]),
                    ))
                    .clicked()
                {
                    action = Some(AssemblyUiAction::RemoveMate(mate.id()));
                }
            });
        }
        if ui
            .add_enabled(
                snapshot.assembly_mates().next().is_some(),
                egui::Button::new(self.catalog.text("assembly-preview-solve")),
            )
            .clicked()
        {
            action = Some(AssemblyUiAction::Solve);
        }
        self.show_assembly_solve_diagnostic(ui, &snapshot);
        ui.separator();

        match action {
            Some(AssemblyUiAction::Insert) => {
                self.preview_insert_occurrence();
            }
            Some(AssemblyUiAction::Ground(id, grounded)) => {
                self.preview_ground_occurrence(id, grounded);
            }
            Some(AssemblyUiAction::SelectEndpointA(id)) => {
                self.assembly_editor.endpoint_a = Some(id);
                self.assembly_editor.reference_a = None;
            }
            Some(AssemblyUiAction::SelectEndpointB(id)) => {
                self.assembly_editor.endpoint_b = Some(id);
                self.assembly_editor.reference_b = None;
            }
            Some(AssemblyUiAction::EditMate(id)) => self.edit_assembly_mate(id),
            Some(AssemblyUiAction::RemoveMate(id)) => {
                self.preview_remove_assembly_mate(id);
            }
            Some(AssemblyUiAction::PreviewMate) => {
                self.preview_editor_mate();
            }
            Some(AssemblyUiAction::Solve) => {
                self.preview_assembly_solve();
            }
            None => {}
        }
    }

    #[must_use]
    pub fn assembly_preview_pending(&self) -> bool {
        self.assembly_editor.preview.is_some()
    }

    #[must_use]
    pub fn assembly_mate_count(&self) -> usize {
        self.document.current().assembly_mates().count()
    }

    #[must_use]
    pub fn grounded_occurrence_count(&self) -> usize {
        self.document.current().grounded_occurrences().count()
    }

    #[must_use]
    pub fn assembly_endpoint_references_ready(&self) -> (bool, bool) {
        let snapshot = self.document.current();
        (
            self.assembly_editor.endpoint_a.is_some()
                && self.assembly_editor.reference_a.is_some()
                && self.assembly_editor.endpoint_a.is_some_and(|id| {
                    !self
                        .assembly_references(&snapshot, id, self.assembly_editor.kind)
                        .is_empty()
                }),
            self.assembly_editor.endpoint_b.is_some()
                && self.assembly_editor.reference_b.is_some()
                && self.assembly_editor.endpoint_b.is_some_and(|id| {
                    !self
                        .assembly_references(&snapshot, id, self.assembly_editor.kind)
                        .is_empty()
                }),
        )
    }

    #[must_use]
    pub fn assembly_solve_status(&self) -> Option<AssemblySolveStatus> {
        self.assembly_editor
            .solve_result
            .as_ref()
            .map(AssemblySolveResult::status)
    }
}
