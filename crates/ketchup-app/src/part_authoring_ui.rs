use super::*;
use ketchup_core::document::BodyId;
#[cfg(debug_assertions)]
use ketchup_core::exact_product::ExactProductError;
use ketchup_core::release_capstone::ReleaseCapstoneContract;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, PadSpec, PocketSpec, SketchConstraint, SketchConstraintId,
    SketchConstraintKind, SketchEntity, SketchEntityId, SketchPointKind, SketchPointRef,
    SketchSpec, WorkplaneSupportHealth,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PartAuthoringStage {
    Plate,
    PocketSupport,
    Pocket,
    Fasteners,
}

#[derive(Clone, Debug)]
struct PartAuthoringPreview {
    proposal: Proposal,
    stage: PartAuthoringStage,
    document_id: DocumentId,
    revision: u64,
    canonical_digest: String,
}

#[derive(Debug)]
pub(super) struct PartAuthoringUiState {
    plate_dimensions: String,
    pocket_dimensions: String,
    fastener_dimensions: String,
    preview: Option<PartAuthoringPreview>,
}

impl Default for PartAuthoringUiState {
    fn default() -> Self {
        Self {
            plate_dimensions: "120,80,8".to_owned(),
            pocket_dimensions: "20,4".to_owned(),
            fastener_dimensions: "12,30,24".to_owned(),
            preview: None,
        }
    }
}

impl KetchupApp {
    pub(super) fn show_part_authoring_ui(&mut self, ui: &mut egui::Ui) {
        let title = self.catalog.text("part-authoring-title");
        egui::CollapsingHeader::new(title)
            .id_salt("part-authoring")
            .default_open(false)
            .show(ui, |ui| {
                let stage = self.part_authoring_stage();
                if let Some(stage) = stage {
                    let (label_key, input) = match stage {
                        PartAuthoringStage::Plate => (
                            "part-authoring-plate-dimensions",
                            &mut self.part_authoring.plate_dimensions,
                        ),
                        PartAuthoringStage::PocketSupport | PartAuthoringStage::Pocket => (
                            "part-authoring-pocket-dimensions",
                            &mut self.part_authoring.pocket_dimensions,
                        ),
                        PartAuthoringStage::Fasteners => (
                            "part-authoring-fastener-dimensions",
                            &mut self.part_authoring.fastener_dimensions,
                        ),
                    };
                    ui.label(self.catalog.text(label_key));
                    ui.text_edit_singleline(input);
                } else {
                    ui.label(self.catalog.text("part-authoring-complete"));
                }

                if let Some(preview) = self.part_authoring.preview.as_ref() {
                    ui.label(self.catalog.format(
                        "part-authoring-preview-summary",
                        &BTreeMap::from([(
                            "count",
                            preview.proposal.batch().commands().len().to_string(),
                        )]),
                    ));
                    let confirm = ui
                        .button(self.catalog.text("part-authoring-confirm"))
                        .clicked();
                    let cancel = ui
                        .button(self.catalog.text("part-authoring-cancel"))
                        .clicked();
                    if confirm {
                        self.confirm_part_authoring_preview();
                    } else if cancel {
                        self.cancel_part_authoring_preview();
                    }
                } else if stage.is_some()
                    && ui
                        .button(self.catalog.text("part-authoring-preview"))
                        .clicked()
                {
                    self.prepare_part_authoring_preview();
                }
            });
    }

    fn part_authoring_stage(&self) -> Option<PartAuthoringStage> {
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let snapshot = self.document.current();
        if snapshot.definition(contract.plate_definition_id).is_none() {
            Some(PartAuthoringStage::Plate)
        } else if snapshot
            .feature(contract.plate_feature_ids.face_workplane)
            .is_none()
        {
            Some(PartAuthoringStage::PocketSupport)
        } else if snapshot
            .feature(contract.plate_feature_ids.pocket)
            .is_none()
        {
            Some(PartAuthoringStage::Pocket)
        } else if snapshot.definition(contract.shared_definition_id).is_none()
            || snapshot
                .definition(contract.replacement_definition_id)
                .is_none()
        {
            Some(PartAuthoringStage::Fasteners)
        } else {
            None
        }
    }

    fn prepare_part_authoring_preview(&mut self) -> bool {
        let Some(stage) = self.part_authoring_stage() else {
            return false;
        };
        let batch = match stage {
            PartAuthoringStage::Plate => self.plan_capstone_plate(),
            PartAuthoringStage::PocketSupport => self.plan_capstone_pocket_support(),
            PartAuthoringStage::Pocket => self.plan_capstone_pocket(),
            PartAuthoringStage::Fasteners => self.plan_capstone_fasteners(),
        };
        let Some(batch) = batch else {
            self.digest = self.catalog.text("part-authoring-invalid");
            return false;
        };
        let proposal = match self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
        {
            Ok(proposal) => proposal,
            Err(error) => {
                self.digest = format!("{}: {error}", self.catalog.text("part-authoring-invalid"));
                return false;
            }
        };
        if let Err(error) = self.document.preview_batch(proposal.batch()) {
            self.digest = format!("{}: {error}", self.catalog.text("part-authoring-invalid"));
            return false;
        }
        let snapshot = self.document.current();
        self.part_authoring.preview = Some(PartAuthoringPreview {
            proposal,
            stage,
            document_id: snapshot.document_id(),
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
        });
        self.digest = self.catalog.text("part-authoring-preview-ready");
        true
    }

    fn confirm_part_authoring_preview(&mut self) -> bool {
        let Some(preview) = self.part_authoring.preview.take() else {
            return false;
        };
        let snapshot = self.document.current();
        if snapshot.document_id() != preview.document_id
            || snapshot.revision_id() != preview.revision
            || snapshot.canonical_digest() != preview.canonical_digest
        {
            self.digest = self.catalog.text("error-preview-stale");
            return false;
        }
        match self.document.commit_verified_proposal(&preview.proposal) {
            Ok(_) => {
                self.clear_ephemeral_edit_state();
                self.selection = SelectionState::default();
                self.render_plan = None;
                self.exact_source = None;
                self.digest = self.catalog.text(match preview.stage {
                    PartAuthoringStage::Plate => "part-authoring-plate-committed",
                    PartAuthoringStage::PocketSupport => "part-authoring-pocket-support-committed",
                    PartAuthoringStage::Pocket => "part-authoring-pocket-committed",
                    PartAuthoringStage::Fasteners => "part-authoring-fasteners-committed",
                });
                true
            }
            Err(_) => {
                self.digest = self.catalog.text("error-preview-stale");
                false
            }
        }
    }

    pub(super) fn cancel_part_authoring_preview(&mut self) {
        self.part_authoring.preview = None;
        self.digest = self.catalog.text("digest-cancelled");
    }

    pub(super) fn part_authoring_preview_pending(&self) -> bool {
        self.part_authoring.preview.is_some()
    }

    fn plan_capstone_plate(&self) -> Option<CommandBatch> {
        let [length, width, height] =
            parse_positive_dimensions::<3>(&self.part_authoring.plate_dimensions)?;
        let snapshot = self.document.current();
        if snapshot.definitions().count() != 1
            || snapshot.definition(INITIAL_BOX_DEFINITION).is_none()
            || snapshot.occurrences().count() != 1
            || snapshot.occurrence(OccurrenceId(1)).is_none()
        {
            return None;
        }
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let ids = contract.plate_feature_ids;
        let sketch = fixed_rectangle(ids.principal_workplane, [0.0, 0.0], length, width);
        let region = sketch.solved_regions().ok()?.first()?.id;
        Some(CommandBatch::new(vec![
            CanonicalCommand::DeleteOccurrence {
                id: OccurrenceId(1),
            },
            CanonicalCommand::DeleteDefinition {
                id: INITIAL_BOX_DEFINITION,
            },
            CanonicalCommand::CreateDefinition {
                id: contract.plate_definition_id,
                name: self.catalog.text("part-authoring-plate-name"),
            },
            CanonicalCommand::CreateBody {
                definition_id: contract.plate_definition_id,
                id: contract.plate_body_id,
                name: self.catalog.text("part-authoring-plate-body"),
                visible: true,
            },
            CanonicalCommand::SetActiveBody {
                definition_id: contract.plate_definition_id,
                id: contract.plate_body_id,
            },
            CanonicalCommand::DeleteBody {
                definition_id: contract.plate_definition_id,
                id: BodyId(1),
            },
            CanonicalCommand::CreateFeature {
                id: ids.principal_workplane,
                definition_id: contract.plate_definition_id,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: ids.base_sketch,
                definition_id: contract.plate_definition_id,
                name: self.catalog.text("part-authoring-plate-sketch"),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: ids.pad,
                definition_id: contract.plate_definition_id,
                name: self.catalog.text("part-authoring-plate-pad"),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: ids.base_sketch,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(height)?),
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: contract.plate_occurrence_id,
                definition_id: contract.plate_definition_id,
                name: self.catalog.text("part-authoring-plate-name"),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
    }

    fn plan_capstone_pocket_support(&self) -> Option<CommandBatch> {
        let [width, depth] =
            parse_positive_dimensions::<2>(&self.part_authoring.pocket_dimensions)?;
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let ids = contract.plate_feature_ids;
        let snapshot = self.document.current();
        if snapshot.occurrence_effectively_visible(contract.plate_occurrence_id) != Some(true) {
            return None;
        }
        let FeatureKind::Pad(pad) = snapshot.feature(ids.pad)?.kind() else {
            return None;
        };
        if depth >= pad.extent.distance().millimetres() {
            return None;
        }
        let request = ExactFeatureChainRequest::from_snapshot_for_producer(
            &snapshot,
            contract.plate_definition_id,
            ids.pad,
        )
        .ok()?;
        let plate_length = f64::from_bits(request.width_bits);
        let plate_width = f64::from_bits(request.depth_bits);
        if width >= plate_length || width >= plate_width {
            return None;
        }
        let exact = self
            .exact_results
            .get_render(&snapshot, contract.plate_definition_id)?;
        let top = exact.reference(ExactFaceRole::Top)?;
        let top = snapshot
            .exact_reference_by_lineage(&top.lineage_digest)?
            .clone();
        let face_frame = snapshot.resolved_planar_face_workplane_frame(&top)?;
        let sketch = fixed_rectangle(
            ids.face_workplane,
            [plate_length * 0.5, plate_width * 0.5],
            width,
            width,
        );
        Some(CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: ids.face_workplane,
                definition_id: contract.plate_definition_id,
                name: self.catalog.text("part-authoring-face-workplane"),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: face_frame,
                }),
            },
            CanonicalCommand::CreateFeature {
                id: ids.pocket_sketch,
                definition_id: contract.plate_definition_id,
                name: self.catalog.text("part-authoring-pocket-sketch"),
                kind: FeatureKind::Sketch(sketch),
            },
        ]))
    }

    fn plan_capstone_pocket(&self) -> Option<CommandBatch> {
        let [_, depth] = parse_positive_dimensions::<2>(&self.part_authoring.pocket_dimensions)?;
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let ids = contract.plate_feature_ids;
        let snapshot = self.document.current();
        let FeatureKind::Pad(pad) = snapshot.feature(ids.pad)?.kind() else {
            return None;
        };
        if depth >= pad.extent.distance().millimetres() {
            return None;
        }
        let FeatureKind::Workplane(workplane) = snapshot.feature(ids.face_workplane)?.kind() else {
            return None;
        };
        let WorkplaneSupport::PlanarFace { reference, health } = &workplane.support else {
            return None;
        };
        if *health != WorkplaneSupportHealth::Resolved {
            return None;
        }
        let FeatureKind::Sketch(sketch) = snapshot.feature(ids.pocket_sketch)?.kind() else {
            return None;
        };
        let region = sketch.solved_regions().ok()?.first()?.id;
        Some(CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: ids.pocket,
            definition_id: contract.plate_definition_id,
            name: self.catalog.text("part-authoring-pocket"),
            kind: FeatureKind::SketchPocket(PocketSpec {
                target: ids.pad,
                sketch: ids.pocket_sketch,
                region,
                support: reference.clone(),
                direction: FeatureDirection::OppositeNormal,
                extent: FeatureExtent::Blind(dimension(depth)?),
            }),
        }]))
    }

    fn plan_capstone_fasteners(&self) -> Option<CommandBatch> {
        let [diameter, shared_height, target_height] =
            parse_positive_dimensions::<3>(&self.part_authoring.fastener_dimensions)?;
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let snapshot = self.document.current();
        if snapshot
            .feature(contract.plate_feature_ids.pocket)
            .is_none()
            || snapshot.definition(contract.shared_definition_id).is_some()
            || snapshot
                .definition(contract.replacement_definition_id)
                .is_some()
        {
            return None;
        }
        let mut commands = Vec::new();
        add_fastener(
            &mut commands,
            &self.catalog,
            (
                contract.shared_definition_id,
                contract.shared_body_id,
                contract.shared_feature_ids,
            ),
            [diameter, shared_height],
            "part-authoring-shared-fastener",
        )?;
        add_fastener(
            &mut commands,
            &self.catalog,
            (
                contract.replacement_definition_id,
                contract.replacement_body_id,
                contract.replacement_feature_ids,
            ),
            [diameter, target_height],
            "part-authoring-target-fastener",
        )?;
        commands.extend([
            CanonicalCommand::CreateOccurrence {
                id: contract.first_shared_occurrence_id,
                definition_id: contract.shared_definition_id,
                name: self.catalog.text("part-authoring-fastener-a"),
                transform: Transform::from_translation(-30.0, 0.0, 8.0).ok()?,
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateOccurrence {
                id: contract.second_shared_occurrence_id,
                definition_id: contract.shared_definition_id,
                name: self.catalog.text("part-authoring-fastener-b"),
                transform: Transform::from_translation(30.0, 0.0, 8.0).ok()?,
                parent: None,
                tag: None,
                visible: true,
            },
        ]);
        Some(CommandBatch::new(commands))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub fn headless_part_authoring_step_ready(&self) -> bool {
        match self.part_authoring_stage() {
            Some(PartAuthoringStage::Plate) => self.plan_capstone_plate().is_some(),
            Some(PartAuthoringStage::PocketSupport) => {
                self.plan_capstone_pocket_support().is_some()
            }
            Some(PartAuthoringStage::Pocket) => self.plan_capstone_pocket().is_some(),
            Some(PartAuthoringStage::Fasteners) => self.plan_capstone_fasteners().is_some(),
            None => true,
        }
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub fn headless_part_authoring_proposal_parity(&self) -> bool {
        let Some(stage) = self.part_authoring_stage() else {
            return true;
        };
        let batch = match stage {
            PartAuthoringStage::Plate => self.plan_capstone_plate(),
            PartAuthoringStage::PocketSupport => self.plan_capstone_pocket_support(),
            PartAuthoringStage::Pocket => self.plan_capstone_pocket(),
            PartAuthoringStage::Fasteners => self.plan_capstone_fasteners(),
        };
        let Some(batch) = batch else {
            return false;
        };
        let Ok(manual) = self
            .document
            .prepare_proposal_with_context(batch.clone(), ProposalContext::canonical_preview())
        else {
            return false;
        };
        let Ok(assistant) = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::local_assistant_model())
        else {
            return false;
        };
        manual.principal() == ProposalPrincipal::ManualClient
            && assistant.principal() == ProposalPrincipal::LocalAssistant
            && manual.batch() == assistant.batch()
            && manual.command_digest() == assistant.command_digest()
            && manual.intended_result_digest() == assistant.intended_result_digest()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn make_headless_part_authoring_preview_stale(&mut self) -> bool {
        if self.part_authoring.preview.is_none()
            || self
                .document
                .current()
                .definition(INITIAL_BOX_DEFINITION)
                .is_none()
        {
            return false;
        }
        self.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::RenameDefinition {
                    id: INITIAL_BOX_DEFINITION,
                    name: "Intervening edit".to_owned(),
                },
            ]))
            .is_ok()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub fn headless_part_authoring_unsupported_profile_refused(&self) -> bool {
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let ids = contract.plate_feature_ids;
        let snapshot = self.document.current();
        let Some(FeatureKind::Workplane(workplane)) = snapshot
            .feature(ids.face_workplane)
            .map(|feature| feature.kind())
        else {
            return false;
        };
        let WorkplaneSupport::PlanarFace { reference, health } = &workplane.support else {
            return false;
        };
        if *health != WorkplaneSupportHealth::Resolved {
            return false;
        }
        let sketch = fixed_circle(ids.face_workplane, 10.0);
        let Some(region) = sketch
            .solved_regions()
            .ok()
            .and_then(|regions| regions.first().cloned())
        else {
            return false;
        };
        let Ok(candidate) = self.document.preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature {
                id: ids.pocket_sketch,
            },
            CanonicalCommand::CreateFeature {
                id: ids.pocket_sketch,
                definition_id: contract.plate_definition_id,
                name: self.catalog.text("part-authoring-pocket-sketch"),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: ids.pocket,
                definition_id: contract.plate_definition_id,
                name: self.catalog.text("part-authoring-pocket"),
                kind: FeatureKind::SketchPocket(PocketSpec {
                    target: ids.pad,
                    sketch: ids.pocket_sketch,
                    region: region.id,
                    support: reference.clone(),
                    direction: FeatureDirection::OppositeNormal,
                    extent: FeatureExtent::Blind(dimension(4.0).expect("constant is valid")),
                }),
            },
        ])) else {
            return false;
        };
        matches!(
            ExactFeatureChainRequest::from_snapshot_for_producer(
                &candidate,
                contract.plate_definition_id,
                ids.pocket,
            ),
            Err(ExactProductError::UnsupportedProfile)
        )
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    #[must_use]
    pub fn headless_part_authoring_subshape_lineage(&self) -> Vec<String> {
        let snapshot = self.document.current();
        let mut lineage = self
            .exact_results
            .render_values(&snapshot)
            .flat_map(|package| package.references())
            .map(|reference| {
                format!(
                    "{}:{}:{}:{}:{}",
                    reference.definition_id.0,
                    reference.producer_feature_id.0,
                    reference.semantic_role,
                    reference.source_element_id,
                    reference.lineage_digest
                )
            })
            .collect::<Vec<_>>();
        lineage.sort();
        lineage
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn set_headless_part_authoring_reference_health(
        &mut self,
        health: WorkplaneSupportHealth,
    ) -> bool {
        let contract = ReleaseCapstoneContract::mechanical_plate_fixture();
        let snapshot = self.document.current();
        let Some(package) = self
            .exact_results
            .get_render(&snapshot, contract.plate_definition_id)
            .map(|package| package.as_ref().clone())
        else {
            return false;
        };
        let registry = match health {
            WorkplaneSupportHealth::Resolved => self.exact_results.clone(),
            WorkplaneSupportHealth::Lost => ExactResultRegistry::default(),
            WorkplaneSupportHealth::Ambiguous => {
                let mut incompatible = package.clone();
                let ExactBodyPackage::Rectangle(incompatible) = &mut incompatible else {
                    return false;
                };
                incompatible.identity.backend.push_str("-ambiguous");
                for reference in &mut incompatible.references {
                    reference.backend = incompatible.identity.backend.clone();
                }
                let Ok(registry) = ExactResultRegistry::accept(
                    &snapshot,
                    [
                        Arc::new(package),
                        Arc::new(ExactBodyPackage::Rectangle(incompatible.clone())),
                    ],
                ) else {
                    return false;
                };
                registry
            }
            WorkplaneSupportHealth::Stale => return false,
        };
        self.document
            .register_exact_reference_evidence(&registry)
            .is_ok()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn set_headless_part_authoring_dimensions(&mut self, value: &str) {
        match self.part_authoring_stage() {
            Some(PartAuthoringStage::Plate) => {
                self.part_authoring.plate_dimensions = value.to_owned();
            }
            Some(PartAuthoringStage::PocketSupport | PartAuthoringStage::Pocket) => {
                self.part_authoring.pocket_dimensions = value.to_owned();
            }
            Some(PartAuthoringStage::Fasteners) => {
                self.part_authoring.fastener_dimensions = value.to_owned();
            }
            None => {}
        }
    }
}

fn parse_positive_dimensions<const N: usize>(value: &str) -> Option<[f64; N]> {
    let values = value
        .split([',', ';', 'x', 'X'])
        .map(str::trim)
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let values: [f64; N] = values.try_into().ok()?;
    values
        .iter()
        .all(|value| value.is_finite() && *value > 0.01)
        .then_some(values)
}

fn dimension(value: f64) -> Option<Dimension> {
    Dimension::new(value.to_string(), value).ok()
}

fn point(entity: u64, point: SketchPointKind) -> SketchPointRef {
    SketchPointRef {
        entity: SketchEntityId(entity),
        point,
    }
}

fn fixed_rectangle(
    workplane: FeatureId,
    center_mm: [f64; 2],
    length: f64,
    width: f64,
) -> SketchSpec {
    let corners = [
        [center_mm[0] - length * 0.5, center_mm[1] - width * 0.5],
        [center_mm[0] + length * 0.5, center_mm[1] - width * 0.5],
        [center_mm[0] + length * 0.5, center_mm[1] + width * 0.5],
        [center_mm[0] - length * 0.5, center_mm[1] + width * 0.5],
    ];
    let entities = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: corners[index],
            end_mm: corners[(index + 1) % 4],
        })
        .collect();
    let mut constraints = Vec::new();
    for index in 0..4 {
        let entity = index as u64 + 1;
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 1),
            kind: SketchConstraintKind::FixedPoint {
                point: point(entity, SketchPointKind::Start),
                position_mm: corners[index],
            },
        });
        constraints.push(SketchConstraint {
            id: SketchConstraintId(index as u64 * 2 + 2),
            kind: SketchConstraintKind::FixedPoint {
                point: point(entity, SketchPointKind::End),
                position_mm: corners[(index + 1) % 4],
            },
        });
    }
    SketchSpec {
        workplane,
        entities,
        constraints,
    }
}

fn fixed_circle(workplane: FeatureId, radius: f64) -> SketchSpec {
    SketchSpec {
        workplane,
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [0.0, 0.0],
            radius_mm: radius,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: dimension(radius).expect("validated radius is canonical"),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: point(1, SketchPointKind::Center),
                    position_mm: [0.0, 0.0],
                },
            },
        ],
    }
}

fn add_fastener(
    commands: &mut Vec<CanonicalCommand>,
    catalog: &LocaleCatalog,
    identity: (
        DefinitionId,
        BodyId,
        ketchup_core::release_capstone::CapstoneFastenerFeatureIds,
    ),
    dimensions: [f64; 2],
    name_key: &str,
) -> Option<()> {
    let (definition, body, ids) = identity;
    let [diameter, height] = dimensions;
    let sketch = fixed_circle(ids.principal_workplane, diameter * 0.5);
    let region = sketch.solved_regions().ok()?.first()?.id;
    let name = catalog.text(name_key);
    commands.extend([
        CanonicalCommand::CreateDefinition {
            id: definition,
            name: name.clone(),
        },
        CanonicalCommand::CreateBody {
            definition_id: definition,
            id: body,
            name: catalog.format(
                "part-authoring-fastener-body",
                &BTreeMap::from([("name", name.clone())]),
            ),
            visible: true,
        },
        CanonicalCommand::SetActiveBody {
            definition_id: definition,
            id: body,
        },
        CanonicalCommand::DeleteBody {
            definition_id: definition,
            id: BodyId(1),
        },
        CanonicalCommand::CreateFeature {
            id: ids.principal_workplane,
            definition_id: definition,
            name: "XY".to_owned(),
            kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
        },
        CanonicalCommand::CreateFeature {
            id: ids.sketch,
            definition_id: definition,
            name: catalog.text("part-authoring-fastener-sketch"),
            kind: FeatureKind::Sketch(sketch),
        },
        CanonicalCommand::CreateFeature {
            id: ids.pad,
            definition_id: definition,
            name: catalog.text("part-authoring-fastener-pad"),
            kind: FeatureKind::Pad(PadSpec {
                sketch: ids.sketch,
                region,
                direction: FeatureDirection::AlongNormal,
                extent: FeatureExtent::Blind(dimension(height)?),
            }),
        },
    ]);
    Some(())
}
