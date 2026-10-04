//! Linear, rectangular and circular patterns and move/copy arrays of the selection.

use crate::*;

impl KetchupApp {
    pub(crate) fn linear_pattern_source_plan(&self) -> Option<LinearPatternSourcePlan> {
        if self.selection.selected_group.is_some() {
            return None;
        }
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_path = (occurrence_paths.len() == 1)
            .then(|| occurrence_paths.first().expect("one occurrence exists"))?;
        let occurrence_id = occurrence_path
            .is_root()
            .then(|| occurrence_path.root_occurrence())?;
        self.linear_pattern_source_plan_for_occurrence(occurrence_id)
    }

    pub(crate) fn rectangular_pattern_source_plan(&self) -> Option<RectangularPatternSourcePlan> {
        if self.selection.selected_group.is_some() {
            return None;
        }
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_path = (occurrence_paths.len() == 1)
            .then(|| occurrence_paths.first().expect("one occurrence exists"))?;
        let occurrence_id = occurrence_path
            .is_root()
            .then(|| occurrence_path.root_occurrence())?;
        self.rectangular_pattern_source_plan_for_occurrence(occurrence_id)
    }

    pub(crate) fn circular_pattern_source_plan(&self) -> Option<CircularPatternSourcePlan> {
        if self.selection.selected_group.is_some() {
            return None;
        }
        let occurrence_paths = self.selected_instance_paths();
        let occurrence_path = (occurrence_paths.len() == 1)
            .then(|| occurrence_paths.first().expect("one occurrence exists"))?;
        let occurrence_id = occurrence_path
            .is_root()
            .then(|| occurrence_path.root_occurrence())?;
        self.circular_pattern_source_plan_for_occurrence(occurrence_id)
    }

    pub(crate) fn apply_current_move_copy_array(
        &mut self,
        mode: MoveCopyArrayMode,
        count: usize,
    ) -> bool {
        let Some((_, previous)) = self.current_move_copy_correction() else {
            return false;
        };
        self.apply_move_copy_array(previous, mode, count)
    }

    pub(crate) fn apply_move_copy_array(
        &mut self,
        previous: MoveCopyCorrection,
        mode: MoveCopyArrayMode,
        count: usize,
    ) -> bool {
        if !(1..=MAX_PATTERN_COUNT).contains(&count) || previous.source_occurrence_ids.is_empty() {
            return false;
        }
        let Ok(parent) = self.document.tip_replacement_parent() else {
            return false;
        };
        let base = parent.snapshot();
        let Some(expected_first_id) = base
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .map(OccurrenceId)
        else {
            return false;
        };
        if expected_first_id != previous.first_copy_occurrence_id {
            return false;
        }
        let source_count = previous.source_occurrence_ids.len();
        let mut commands = Vec::new();
        let mut created_per_definition = BTreeMap::<DefinitionId, usize>::new();
        for copy_index in 1..=count {
            let factor = match mode {
                MoveCopyArrayMode::Multiply => copy_index as f64,
                MoveCopyArrayMode::Divide => copy_index as f64 / count as f64,
            };
            let delta_mm = previous.delta_mm * factor;
            for (source_index, source_id) in
                previous.source_occurrence_ids.iter().copied().enumerate()
            {
                let Some(source) = base.occurrence(source_id) else {
                    return false;
                };
                let Some(definition) = base.definition(source.definition_id()) else {
                    return false;
                };
                let Some(offset) = (copy_index - 1)
                    .checked_mul(source_count)
                    .and_then(|offset| offset.checked_add(source_index))
                else {
                    return false;
                };
                let Some(id) = previous
                    .first_copy_occurrence_id
                    .0
                    .checked_add(offset as u64)
                    .map(OccurrenceId)
                else {
                    return false;
                };
                let Some(transform) =
                    translated_in_parent_space(base, source.parent(), source.transform(), delta_mm)
                else {
                    return false;
                };
                let existing = base
                    .scene_query()
                    .into_iter()
                    .filter(|item| item.definition_id == source.definition_id())
                    .count();
                let created = created_per_definition
                    .entry(source.definition_id())
                    .or_default();
                *created += 1;
                commands.push(CanonicalCommand::CreateOccurrence {
                    id,
                    definition_id: source.definition_id(),
                    name: self.catalog.format(
                        "model-copy-occurrence",
                        &BTreeMap::from([
                            ("name", definition.name().to_owned()),
                            ("number", (existing + *created).to_string()),
                        ]),
                    ),
                    transform,
                    parent: source.parent(),
                    tags: source.tags().clone(),
                    visible: source.visible(),
                });
                if let Some(color) = source.color() {
                    commands.push(CanonicalCommand::SetOccurrenceColor {
                        id,
                        color: Some(color),
                    });
                }
            }
        }
        let Ok(proposal) = self.document.prepare_tip_replacement_proposal(
            &parent,
            CommandBatch::new(commands),
            ProposalContext::canonical_preview(),
        ) else {
            return false;
        };
        if self
            .complete_mutation_with_work_recovery(|document| {
                document.commit_tip_replacement_proposal(&proposal)
            })
            .is_err()
        {
            return false;
        }
        let selected_copy_occurrence_ids = (0..source_count)
            .map(|source_index| {
                OccurrenceId(
                    previous.first_copy_occurrence_id.0
                        + ((count - 1) * source_count + source_index) as u64,
                )
            })
            .collect::<BTreeSet<_>>();
        let selected_copy_occurrence_id = OccurrenceId(
            previous.first_copy_occurrence_id.0
                + ((count - 1) * source_count + previous.primary_source_index) as u64,
        );
        let Some(primary_source) =
            base.occurrence(previous.source_occurrence_ids[previous.primary_source_index])
        else {
            return false;
        };
        self.selection.select_exact(
            SelectionId {
                definition_id: primary_source.definition_id(),
                instance_path: InstancePath::root(selected_copy_occurrence_id),
                element: previous.element.clone(),
            },
            false,
        );
        self.selection.occurrences.extend(
            selected_copy_occurrence_ids
                .iter()
                .copied()
                .map(InstancePath::root),
        );
        self.record_transform_correction(
            CorrectionSelection::Occurrences {
                occurrence_ids: selected_copy_occurrence_ids,
                primary_occurrence_id: Some(selected_copy_occurrence_id),
            },
            CorrectionOperation::MoveCopy(MoveCopyCorrection {
                array_mode: mode,
                array_count: count,
                ..previous
            }),
        );
        self.status_key = "status-object-copied";
        self.digest = self.catalog.format(
            match mode {
                MoveCopyArrayMode::Multiply => "digest-copy-array-multiplied",
                MoveCopyArrayMode::Divide => "digest-copy-array-divided",
            },
            &BTreeMap::from([("count", count.to_string())]),
        );
        true
    }

    pub(crate) fn linear_pattern_plan(
        &self,
        source: &LinearPatternSourcePlan,
        axis: Axis,
        spacing_mm: f64,
        count: usize,
    ) -> Option<LinearPatternPlan> {
        if self
            .linear_pattern_source_plan_for_occurrence(source.occurrence_id)
            .as_ref()
            != Some(source)
            || !(2..=MAX_PATTERN_COUNT).contains(&count)
            || !spacing_mm.is_finite()
            || spacing_mm.abs() <= f64::EPSILON
        {
            return None;
        }
        let mut commands = Vec::with_capacity(count - 1);
        for index in 1..count {
            let id = OccurrenceId(
                source
                    .next_occurrence_id
                    .0
                    .checked_add((index - 1) as u64)?,
            );
            let offset_mm = spacing_mm * index as f64;
            if !offset_mm.is_finite() {
                return None;
            }
            let transform =
                translated_transform(source.source_transform, axis_vector(axis, offset_mm)).ok()?;
            commands.push(CanonicalCommand::CreateOccurrence {
                id,
                definition_id: source.definition_id,
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", source.definition_name.clone()),
                        (
                            "number",
                            (source.existing_definition_occurrence_count + index).to_string(),
                        ),
                    ]),
                ),
                transform,
                parent: source.source_parent,
                tags: source.source_tags.clone(),
                visible: source.source_visible,
            });
            if let Some(color) = source.source_color {
                commands.push(CanonicalCommand::SetOccurrenceColor {
                    id,
                    color: Some(color),
                });
            }
        }
        Some(LinearPatternPlan {
            source: source.clone(),
            axis,
            spacing_mm,
            count,
            commands,
        })
    }

    pub(crate) fn begin_linear_pattern(&mut self) {
        let Some(source) = self.linear_pattern_source_plan() else {
            return;
        };
        self.tool_preview.close::<OccurrenceOperationPreview>();
        self.modal.open(PendingLinearPattern {
            source,
            axis: Axis::X,
            spacing: "100".to_owned(),
            count: "2".to_owned(),
            preview_plan: None,
        });
    }

    #[must_use]
    pub fn linear_pattern_visible(&self) -> bool {
        self.modal.get::<PendingLinearPattern>().is_some()
    }

    #[must_use]
    pub fn linear_pattern_inputs(&self) -> Option<(Axis, &str, &str)> {
        self.modal.get::<PendingLinearPattern>().map(|pending| {
            (
                pending.axis,
                pending.spacing.as_str(),
                pending.count.as_str(),
            )
        })
    }

    pub(crate) fn linear_pattern_binding_is_current(&self, pending: &PendingLinearPattern) -> bool {
        self.linear_pattern_source_plan_for_occurrence(pending.source.occurrence_id)
            .as_ref()
            == Some(&pending.source)
    }

    #[must_use]
    pub fn linear_pattern_preview_is_current(&self) -> bool {
        self.modal
            .get::<PendingLinearPattern>()
            .is_some_and(|pending| {
                let (Ok(spacing_mm), Ok(count)) = (
                    pending.spacing.trim().parse::<f64>(),
                    pending.count.trim().parse::<usize>(),
                ) else {
                    return false;
                };
                let Some(plan) =
                    self.linear_pattern_plan(&pending.source, pending.axis, spacing_mm, count)
                else {
                    return false;
                };
                self.linear_pattern_binding_is_current(pending)
                    && pending.preview_plan.as_ref() == Some(&plan)
                    && self
                        .tool_preview
                        .get::<OccurrenceOperationPreview>()
                        .is_some_and(|preview| {
                            self.has_occurrence_operation_preview()
                                && preview.source_revision == plan.source.source_revision
                                && preview.batch.commands() == plan.commands.as_slice()
                        })
            })
    }

    pub fn preview_pending_linear_pattern(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingLinearPattern>().cloned() else {
            return false;
        };
        let (Ok(spacing_mm), Ok(count)) = (
            pending.spacing.trim().parse::<f64>(),
            pending.count.trim().parse::<usize>(),
        ) else {
            return false;
        };
        let Some(plan) = self.linear_pattern_plan(&pending.source, pending.axis, spacing_mm, count)
        else {
            return false;
        };
        if let Some(current) = self.modal.get_mut::<PendingLinearPattern>() {
            current.preview_plan = None;
        }
        if !self.preview_linear_pattern_plan(plan.clone()) {
            return false;
        }
        if let Some(current) = self.modal.get_mut::<PendingLinearPattern>() {
            current.preview_plan = Some(plan);
        }
        true
    }

    pub fn confirm_linear_pattern(&mut self) -> bool {
        let Some(plan) = self
            .modal
            .get::<PendingLinearPattern>()
            .and_then(|pending| pending.preview_plan.clone())
        else {
            return false;
        };
        if !self.linear_pattern_preview_is_current() || !self.apply_linear_pattern_plan(plan) {
            return false;
        }
        self.modal.close::<PendingLinearPattern>();
        true
    }

    pub fn preview_linear_pattern(
        &mut self,
        source_id: OccurrenceId,
        axis: Axis,
        spacing_mm: f64,
        count: usize,
    ) -> bool {
        let source = match self.modal.get::<PendingLinearPattern>() {
            Some(pending) if pending.source.occurrence_id == source_id => pending.source.clone(),
            Some(_) => return false,
            None => {
                let Some(source) = self.linear_pattern_source_plan_for_occurrence(source_id) else {
                    return false;
                };
                source
            }
        };
        let Some(plan) = self.linear_pattern_plan(&source, axis, spacing_mm, count) else {
            return false;
        };
        self.preview_linear_pattern_plan(plan)
    }

    pub(crate) fn linear_pattern_preview_boxes(
        &self,
        plan: &LinearPatternPlan,
    ) -> Option<BTreeMap<OccurrenceId, RenderBox>> {
        let source_box = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == InstancePath::root(plan.source.occurrence_id))?;
        let mut boxes = BTreeMap::new();
        for (command_index, command) in plan
            .commands
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::CreateOccurrence { .. }))
            .enumerate()
        {
            let CanonicalCommand::CreateOccurrence { id, .. } = command else {
                return None;
            };
            let offset_mm = plan.spacing_mm * (command_index + 1) as f64;
            if !offset_mm.is_finite() {
                return None;
            }
            let delta_mm = axis_vector(plan.axis, offset_mm);
            let mut preview_box = source_box.clone();
            preview_box.instance_path = InstancePath::root(*id);
            preview_box.origin_mm += delta_mm;
            boxes.insert(*id, preview_box);
        }
        Some(boxes)
    }

    pub(crate) fn preview_linear_pattern_plan(&mut self, plan: LinearPatternPlan) -> bool {
        if self
            .linear_pattern_plan(&plan.source, plan.axis, plan.spacing_mm, plan.count)
            .as_ref()
            != Some(&plan)
        {
            return false;
        }
        let Some(boxes) = self.linear_pattern_preview_boxes(&plan) else {
            return false;
        };
        let batch = CommandBatch::new(plan.commands.clone());
        self.tool_preview.open(OccurrenceOperationPreview {
            source_revision: plan.source.source_revision,
            command_digest: batch.digest(),
            batch,
            boxes,
            hidden_occurrences: BTreeSet::new(),
            selection_after: None,
            committed_digest_key: "digest-linear-pattern-committed",
            canonical_plan: Some(OccurrenceCanonicalPreviewPlan::LinearPattern(plan.clone())),
            solid_tool_plan: None,
        });
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            "digest-linear-pattern-live",
            &BTreeMap::from([
                ("axis", alignment_axis_label(plan.axis).to_owned()),
                ("count", plan.count.to_string()),
                ("spacing", format_height(plan.spacing_mm)),
            ]),
        );
        true
    }

    pub(crate) fn apply_linear_pattern_plan(&mut self, plan: LinearPatternPlan) -> bool {
        if self
            .linear_pattern_plan(&plan.source, plan.axis, plan.spacing_mm, plan.count)
            .as_ref()
            != Some(&plan)
            || !self
                .tool_preview
                .get::<OccurrenceOperationPreview>()
                .is_some_and(|preview| {
                    self.has_occurrence_operation_preview()
                        && preview.source_revision == plan.source.source_revision
                        && preview.batch.commands() == plan.commands.as_slice()
                })
        {
            return false;
        }
        self.confirm_occurrence_operation_preview()
    }

    pub(crate) fn rectangular_pattern_plan(
        &self,
        source: &RectangularPatternSourcePlan,
        spec: RectangularPatternSpec,
    ) -> Option<RectangularPatternPlan> {
        if self
            .rectangular_pattern_source_plan_for_occurrence(source.occurrence_id)
            .as_ref()
            != Some(source)
        {
            return None;
        }
        let RectangularPatternSpec {
            primary_axis,
            primary_spacing_mm,
            primary_count,
            secondary_axis,
            secondary_spacing_mm,
            secondary_count,
        } = spec;
        let total_count = primary_count.checked_mul(secondary_count)?;
        if primary_axis == secondary_axis
            || !(2..=MAX_PATTERN_COUNT).contains(&primary_count)
            || !(2..=MAX_PATTERN_COUNT).contains(&secondary_count)
            || total_count > MAX_PATTERN_COUNT
            || !primary_spacing_mm.is_finite()
            || primary_spacing_mm.abs() <= f64::EPSILON
            || !secondary_spacing_mm.is_finite()
            || secondary_spacing_mm.abs() <= f64::EPSILON
        {
            return None;
        }
        let mut commands = Vec::with_capacity(total_count - 1);
        let mut created = 0usize;
        for primary_index in 0..primary_count {
            for secondary_index in 0..secondary_count {
                if primary_index == 0 && secondary_index == 0 {
                    continue;
                }
                let primary_offset = primary_spacing_mm * primary_index as f64;
                let secondary_offset = secondary_spacing_mm * secondary_index as f64;
                if !primary_offset.is_finite() || !secondary_offset.is_finite() {
                    return None;
                }
                let delta_mm = axis_vector(primary_axis, primary_offset)
                    + axis_vector(secondary_axis, secondary_offset);
                let transform = translated_transform(source.source_transform, delta_mm).ok()?;
                let id = OccurrenceId(source.next_occurrence_id.0.checked_add(created as u64)?);
                created += 1;
                commands.push(CanonicalCommand::CreateOccurrence {
                    id,
                    definition_id: source.definition_id,
                    name: self.catalog.format(
                        "model-copy-occurrence",
                        &BTreeMap::from([
                            ("name", source.definition_name.clone()),
                            (
                                "number",
                                (source.existing_definition_occurrence_count + created).to_string(),
                            ),
                        ]),
                    ),
                    transform,
                    parent: source.source_parent,
                    tags: source.source_tags.clone(),
                    visible: source.source_visible,
                });
                if let Some(color) = source.source_color {
                    commands.push(CanonicalCommand::SetOccurrenceColor {
                        id,
                        color: Some(color),
                    });
                }
            }
        }
        Some(RectangularPatternPlan {
            source: source.clone(),
            primary_axis,
            primary_spacing_mm,
            primary_count,
            secondary_axis,
            secondary_spacing_mm,
            secondary_count,
            commands,
        })
    }

    pub(crate) fn begin_rectangular_pattern(&mut self) {
        let Some(source) = self.rectangular_pattern_source_plan() else {
            return;
        };
        self.tool_preview.close::<OccurrenceOperationPreview>();
        self.modal.open(PendingRectangularPattern {
            source,
            primary_axis: Axis::X,
            primary_spacing: "100".to_owned(),
            primary_count: "2".to_owned(),
            secondary_axis: Axis::Y,
            secondary_spacing: "100".to_owned(),
            secondary_count: "2".to_owned(),
            preview_plan: None,
        });
    }

    #[must_use]
    pub fn rectangular_pattern_visible(&self) -> bool {
        self.modal.get::<PendingRectangularPattern>().is_some()
    }

    #[must_use]
    pub fn rectangular_pattern_inputs(&self) -> Option<(Axis, &str, &str, Axis, &str, &str)> {
        self.modal
            .get::<PendingRectangularPattern>()
            .map(|pending| {
                (
                    pending.primary_axis,
                    pending.primary_spacing.as_str(),
                    pending.primary_count.as_str(),
                    pending.secondary_axis,
                    pending.secondary_spacing.as_str(),
                    pending.secondary_count.as_str(),
                )
            })
    }

    pub(crate) fn rectangular_pattern_binding_is_current(
        &self,
        pending: &PendingRectangularPattern,
    ) -> bool {
        self.rectangular_pattern_source_plan_for_occurrence(pending.source.occurrence_id)
            .as_ref()
            == Some(&pending.source)
    }

    #[must_use]
    pub fn rectangular_pattern_preview_is_current(&self) -> bool {
        self.modal
            .get::<PendingRectangularPattern>()
            .is_some_and(|pending| {
                let (
                    Ok(primary_spacing_mm),
                    Ok(primary_count),
                    Ok(secondary_spacing_mm),
                    Ok(secondary_count),
                ) = (
                    pending.primary_spacing.trim().parse::<f64>(),
                    pending.primary_count.trim().parse::<usize>(),
                    pending.secondary_spacing.trim().parse::<f64>(),
                    pending.secondary_count.trim().parse::<usize>(),
                )
                else {
                    return false;
                };
                let Some(plan) = self.rectangular_pattern_plan(
                    &pending.source,
                    RectangularPatternSpec {
                        primary_axis: pending.primary_axis,
                        primary_spacing_mm,
                        primary_count,
                        secondary_axis: pending.secondary_axis,
                        secondary_spacing_mm,
                        secondary_count,
                    },
                ) else {
                    return false;
                };
                self.rectangular_pattern_binding_is_current(pending)
                    && pending.preview_plan.as_ref() == Some(&plan)
                    && self
                        .tool_preview
                        .get::<OccurrenceOperationPreview>()
                        .is_some_and(|preview| {
                            self.has_occurrence_operation_preview()
                                && preview.source_revision == plan.source.source_revision
                                && preview.batch.commands() == plan.commands.as_slice()
                        })
            })
    }

    pub fn preview_pending_rectangular_pattern(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingRectangularPattern>().cloned() else {
            return false;
        };
        let (
            Ok(primary_spacing_mm),
            Ok(primary_count),
            Ok(secondary_spacing_mm),
            Ok(secondary_count),
        ) = (
            pending.primary_spacing.trim().parse::<f64>(),
            pending.primary_count.trim().parse::<usize>(),
            pending.secondary_spacing.trim().parse::<f64>(),
            pending.secondary_count.trim().parse::<usize>(),
        )
        else {
            return false;
        };
        let Some(plan) = self.rectangular_pattern_plan(
            &pending.source,
            RectangularPatternSpec {
                primary_axis: pending.primary_axis,
                primary_spacing_mm,
                primary_count,
                secondary_axis: pending.secondary_axis,
                secondary_spacing_mm,
                secondary_count,
            },
        ) else {
            return false;
        };
        if let Some(current) = self.modal.get_mut::<PendingRectangularPattern>() {
            current.preview_plan = None;
        }
        if !self.preview_rectangular_pattern_plan(plan.clone()) {
            return false;
        }
        if let Some(current) = self.modal.get_mut::<PendingRectangularPattern>() {
            current.preview_plan = Some(plan);
        }
        true
    }

    pub fn confirm_rectangular_pattern(&mut self) -> bool {
        let Some(plan) = self
            .modal
            .get::<PendingRectangularPattern>()
            .and_then(|pending| pending.preview_plan.clone())
        else {
            return false;
        };
        if !self.rectangular_pattern_preview_is_current()
            || !self.apply_rectangular_pattern_plan(plan)
        {
            return false;
        }
        self.modal.close::<PendingRectangularPattern>();
        true
    }

    pub fn preview_rectangular_pattern(
        &mut self,
        source_id: OccurrenceId,
        spec: RectangularPatternSpec,
    ) -> bool {
        let source = match self.modal.get::<PendingRectangularPattern>() {
            Some(pending) if pending.source.occurrence_id == source_id => pending.source.clone(),
            Some(_) => return false,
            None => {
                let Some(source) = self.rectangular_pattern_source_plan_for_occurrence(source_id)
                else {
                    return false;
                };
                source
            }
        };
        let Some(plan) = self.rectangular_pattern_plan(&source, spec) else {
            return false;
        };
        self.preview_rectangular_pattern_plan(plan)
    }

    pub(crate) fn rectangular_pattern_preview_boxes(
        &self,
        plan: &RectangularPatternPlan,
    ) -> Option<BTreeMap<OccurrenceId, RenderBox>> {
        let source_box = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == InstancePath::root(plan.source.occurrence_id))?;
        let mut boxes = BTreeMap::new();
        let mut commands = plan
            .commands
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::CreateOccurrence { .. }));
        for primary_index in 0..plan.primary_count {
            for secondary_index in 0..plan.secondary_count {
                if primary_index == 0 && secondary_index == 0 {
                    continue;
                }
                let Some(CanonicalCommand::CreateOccurrence { id, .. }) = commands.next() else {
                    return None;
                };
                let primary_offset = plan.primary_spacing_mm * primary_index as f64;
                let secondary_offset = plan.secondary_spacing_mm * secondary_index as f64;
                if !primary_offset.is_finite() || !secondary_offset.is_finite() {
                    return None;
                }
                let delta_mm = axis_vector(plan.primary_axis, primary_offset)
                    + axis_vector(plan.secondary_axis, secondary_offset);
                let mut preview_box = source_box.clone();
                preview_box.instance_path = InstancePath::root(*id);
                preview_box.origin_mm += delta_mm;
                boxes.insert(*id, preview_box);
            }
        }
        (commands.next().is_none()).then_some(boxes)
    }

    pub(crate) fn preview_rectangular_pattern_plan(
        &mut self,
        plan: RectangularPatternPlan,
    ) -> bool {
        if self
            .rectangular_pattern_plan(
                &plan.source,
                RectangularPatternSpec {
                    primary_axis: plan.primary_axis,
                    primary_spacing_mm: plan.primary_spacing_mm,
                    primary_count: plan.primary_count,
                    secondary_axis: plan.secondary_axis,
                    secondary_spacing_mm: plan.secondary_spacing_mm,
                    secondary_count: plan.secondary_count,
                },
            )
            .as_ref()
            != Some(&plan)
        {
            return false;
        }
        let Some(boxes) = self.rectangular_pattern_preview_boxes(&plan) else {
            return false;
        };
        let batch = CommandBatch::new(plan.commands.clone());
        self.tool_preview.open(OccurrenceOperationPreview {
            source_revision: plan.source.source_revision,
            command_digest: batch.digest(),
            batch,
            boxes,
            hidden_occurrences: BTreeSet::new(),
            selection_after: None,
            committed_digest_key: "digest-rectangular-pattern-committed",
            canonical_plan: Some(OccurrenceCanonicalPreviewPlan::RectangularPattern(
                plan.clone(),
            )),
            solid_tool_plan: None,
        });
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            "digest-rectangular-pattern-live",
            &BTreeMap::from([
                (
                    "count",
                    (plan.primary_count * plan.secondary_count).to_string(),
                ),
                (
                    "primary-axis",
                    alignment_axis_label(plan.primary_axis).to_owned(),
                ),
                ("primary-spacing", format_height(plan.primary_spacing_mm)),
                (
                    "secondary-axis",
                    alignment_axis_label(plan.secondary_axis).to_owned(),
                ),
                (
                    "secondary-spacing",
                    format_height(plan.secondary_spacing_mm),
                ),
            ]),
        );
        true
    }

    pub(crate) fn apply_rectangular_pattern_plan(&mut self, plan: RectangularPatternPlan) -> bool {
        if self
            .rectangular_pattern_plan(
                &plan.source,
                RectangularPatternSpec {
                    primary_axis: plan.primary_axis,
                    primary_spacing_mm: plan.primary_spacing_mm,
                    primary_count: plan.primary_count,
                    secondary_axis: plan.secondary_axis,
                    secondary_spacing_mm: plan.secondary_spacing_mm,
                    secondary_count: plan.secondary_count,
                },
            )
            .as_ref()
            != Some(&plan)
            || !self
                .tool_preview
                .get::<OccurrenceOperationPreview>()
                .is_some_and(|preview| {
                    self.has_occurrence_operation_preview()
                        && preview.source_revision == plan.source.source_revision
                        && preview.batch.commands() == plan.commands.as_slice()
                })
        {
            return false;
        }
        self.confirm_occurrence_operation_preview()
    }

    pub(crate) fn circular_pattern_plan(
        &self,
        source: &CircularPatternSourcePlan,
        axis: Axis,
        centre_mm: Vec3,
        angle_step_degrees: f64,
        count: usize,
    ) -> Option<CircularPatternPlan> {
        if self
            .circular_pattern_source_plan_for_occurrence(source.occurrence_id)
            .as_ref()
            != Some(source)
            || !(2..=MAX_PATTERN_COUNT).contains(&count)
            || !centre_mm.x.is_finite()
            || !centre_mm.y.is_finite()
            || !centre_mm.z.is_finite()
            || !angle_step_degrees.is_finite()
            || !(0.01..=360.0).contains(&angle_step_degrees.abs())
        {
            return None;
        }
        let mut commands = Vec::with_capacity(count - 1);
        for index in 1..count {
            let id = OccurrenceId(
                source
                    .next_occurrence_id
                    .0
                    .checked_add((index - 1) as u64)?,
            );
            let rotation =
                world_rotation_transform(centre_mm, axis, angle_step_degrees * index as f64)
                    .ok()?;
            commands.push(CanonicalCommand::CreateOccurrence {
                id,
                definition_id: source.definition_id,
                name: self.catalog.format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", source.definition_name.clone()),
                        (
                            "number",
                            (source.existing_definition_occurrence_count + index).to_string(),
                        ),
                    ]),
                ),
                transform: rotation.compose(source.source_transform),
                parent: source.source_parent,
                tags: source.source_tags.clone(),
                visible: source.source_visible,
            });
            if let Some(color) = source.source_color {
                commands.push(CanonicalCommand::SetOccurrenceColor {
                    id,
                    color: Some(color),
                });
            }
        }
        Some(CircularPatternPlan {
            source: source.clone(),
            axis,
            centre_mm,
            angle_step_degrees,
            count,
            commands,
        })
    }

    pub(crate) fn begin_circular_pattern(&mut self) {
        let Some(source) = self.circular_pattern_source_plan() else {
            return;
        };
        self.tool_preview.close::<OccurrenceOperationPreview>();
        self.modal.open(PendingCircularPattern {
            source,
            axis: Axis::Z,
            centre_x: "0".to_owned(),
            centre_y: "0".to_owned(),
            centre_z: "0".to_owned(),
            angle: "90".to_owned(),
            count: "4".to_owned(),
            preview_plan: None,
        });
    }

    #[must_use]
    pub fn circular_pattern_visible(&self) -> bool {
        self.modal.get::<PendingCircularPattern>().is_some()
    }

    #[must_use]
    pub fn circular_pattern_inputs(&self) -> Option<(Axis, &str, &str, &str, &str, &str)> {
        self.modal.get::<PendingCircularPattern>().map(|pending| {
            (
                pending.axis,
                pending.centre_x.as_str(),
                pending.centre_y.as_str(),
                pending.centre_z.as_str(),
                pending.angle.as_str(),
                pending.count.as_str(),
            )
        })
    }

    pub(crate) fn circular_pattern_binding_is_current(
        &self,
        pending: &PendingCircularPattern,
    ) -> bool {
        self.circular_pattern_source_plan_for_occurrence(pending.source.occurrence_id)
            .as_ref()
            == Some(&pending.source)
    }

    #[must_use]
    pub fn circular_pattern_preview_is_current(&self) -> bool {
        self.modal
            .get::<PendingCircularPattern>()
            .is_some_and(|pending| {
                let (Ok(centre_x), Ok(centre_y), Ok(centre_z), Ok(angle), Ok(count)) = (
                    pending.centre_x.trim().parse::<f64>(),
                    pending.centre_y.trim().parse::<f64>(),
                    pending.centre_z.trim().parse::<f64>(),
                    pending.angle.trim().parse::<f64>(),
                    pending.count.trim().parse::<usize>(),
                ) else {
                    return false;
                };
                let Some(plan) = self.circular_pattern_plan(
                    &pending.source,
                    pending.axis,
                    Vec3::new(centre_x, centre_y, centre_z),
                    angle,
                    count,
                ) else {
                    return false;
                };
                self.circular_pattern_binding_is_current(pending)
                    && pending.preview_plan.as_ref() == Some(&plan)
                    && self
                        .tool_preview
                        .get::<OccurrenceOperationPreview>()
                        .is_some_and(|preview| {
                            self.has_occurrence_operation_preview()
                                && preview.source_revision == plan.source.source_revision
                                && preview.batch.commands() == plan.commands.as_slice()
                        })
            })
    }

    pub fn preview_pending_circular_pattern(&mut self) -> bool {
        let Some(pending) = self.modal.get::<PendingCircularPattern>().cloned() else {
            return false;
        };
        let (Ok(centre_x), Ok(centre_y), Ok(centre_z), Ok(angle), Ok(count)) = (
            pending.centre_x.trim().parse::<f64>(),
            pending.centre_y.trim().parse::<f64>(),
            pending.centre_z.trim().parse::<f64>(),
            pending.angle.trim().parse::<f64>(),
            pending.count.trim().parse::<usize>(),
        ) else {
            return false;
        };
        let Some(plan) = self.circular_pattern_plan(
            &pending.source,
            pending.axis,
            Vec3::new(centre_x, centre_y, centre_z),
            angle,
            count,
        ) else {
            return false;
        };
        if let Some(current) = self.modal.get_mut::<PendingCircularPattern>() {
            current.preview_plan = None;
        }
        if !self.preview_circular_pattern_plan(plan.clone()) {
            return false;
        }
        if let Some(current) = self.modal.get_mut::<PendingCircularPattern>() {
            current.preview_plan = Some(plan);
        }
        true
    }

    pub fn confirm_circular_pattern(&mut self) -> bool {
        let Some(plan) = self
            .modal
            .get::<PendingCircularPattern>()
            .and_then(|pending| pending.preview_plan.clone())
        else {
            return false;
        };
        if !self.circular_pattern_preview_is_current() || !self.apply_circular_pattern_plan(plan) {
            return false;
        }
        self.modal.close::<PendingCircularPattern>();
        true
    }

    pub fn preview_circular_pattern(
        &mut self,
        source_id: OccurrenceId,
        axis: Axis,
        centre_mm: Vec3,
        angle_step_degrees: f64,
        count: usize,
    ) -> bool {
        let source = match self.modal.get::<PendingCircularPattern>() {
            Some(pending) if pending.source.occurrence_id == source_id => pending.source.clone(),
            Some(_) => return false,
            None => {
                let Some(source) = self.circular_pattern_source_plan_for_occurrence(source_id)
                else {
                    return false;
                };
                source
            }
        };
        let Some(plan) =
            self.circular_pattern_plan(&source, axis, centre_mm, angle_step_degrees, count)
        else {
            return false;
        };
        self.preview_circular_pattern_plan(plan)
    }

    pub(crate) fn circular_pattern_preview_boxes(
        &self,
        plan: &CircularPatternPlan,
    ) -> Option<BTreeMap<OccurrenceId, RenderBox>> {
        let source_box = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == InstancePath::root(plan.source.occurrence_id))?;
        let mut boxes = BTreeMap::new();
        let mut commands = plan
            .commands
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::CreateOccurrence { .. }));
        for index in 1..plan.count {
            let Some(CanonicalCommand::CreateOccurrence { id, .. }) = commands.next() else {
                return None;
            };
            let rotation = world_rotation_transform(
                plan.centre_mm,
                plan.axis,
                plan.angle_step_degrees * index as f64,
            )
            .ok()?;
            let corners = box_corners(
                source_box.size_mm.x,
                source_box.size_mm.y,
                source_box.size_mm.z,
            )
            .map(|corner| transform_model_point(rotation, corner + source_box.origin_mm));
            let (minimum, maximum) = corners.into_iter().fold(
                (
                    Vec3::new(f64::MAX, f64::MAX, f64::MAX),
                    Vec3::new(f64::MIN, f64::MIN, f64::MIN),
                ),
                |(minimum, maximum), corner| {
                    (
                        Vec3::new(
                            minimum.x.min(corner.x),
                            minimum.y.min(corner.y),
                            minimum.z.min(corner.z),
                        ),
                        Vec3::new(
                            maximum.x.max(corner.x),
                            maximum.y.max(corner.y),
                            maximum.z.max(corner.z),
                        ),
                    )
                },
            );
            let mut preview_box = source_box.clone();
            preview_box.instance_path = InstancePath::root(*id);
            preview_box.origin_mm = minimum;
            preview_box.size_mm = maximum - minimum;
            boxes.insert(*id, preview_box);
        }
        (commands.next().is_none()).then_some(boxes)
    }

    pub(crate) fn preview_circular_pattern_plan(&mut self, plan: CircularPatternPlan) -> bool {
        if self
            .circular_pattern_plan(
                &plan.source,
                plan.axis,
                plan.centre_mm,
                plan.angle_step_degrees,
                plan.count,
            )
            .as_ref()
            != Some(&plan)
        {
            return false;
        }
        let Some(boxes) = self.circular_pattern_preview_boxes(&plan) else {
            return false;
        };
        let batch = CommandBatch::new(plan.commands.clone());
        self.tool_preview.open(OccurrenceOperationPreview {
            source_revision: plan.source.source_revision,
            command_digest: batch.digest(),
            batch,
            boxes,
            hidden_occurrences: BTreeSet::new(),
            selection_after: None,
            committed_digest_key: "digest-circular-pattern-committed",
            canonical_plan: Some(OccurrenceCanonicalPreviewPlan::CircularPattern(
                plan.clone(),
            )),
            solid_tool_plan: None,
        });
        self.status_key = "status-preview";
        self.digest = self.catalog.format(
            "digest-circular-pattern-live",
            &BTreeMap::from([
                ("angle", format_angle(plan.angle_step_degrees)),
                ("axis", alignment_axis_label(plan.axis).to_owned()),
                ("count", plan.count.to_string()),
                (
                    "centre",
                    format!(
                        "{}, {}, {}",
                        format_height(plan.centre_mm.x),
                        format_height(plan.centre_mm.y),
                        format_height(plan.centre_mm.z)
                    ),
                ),
            ]),
        );
        true
    }

    pub(crate) fn apply_circular_pattern_plan(&mut self, plan: CircularPatternPlan) -> bool {
        if self
            .circular_pattern_plan(
                &plan.source,
                plan.axis,
                plan.centre_mm,
                plan.angle_step_degrees,
                plan.count,
            )
            .as_ref()
            != Some(&plan)
            || !self
                .tool_preview
                .get::<OccurrenceOperationPreview>()
                .is_some_and(|preview| {
                    self.has_occurrence_operation_preview()
                        && preview.source_revision == plan.source.source_revision
                        && preview.batch.commands() == plan.commands.as_slice()
                })
        {
            return false;
        }
        self.confirm_occurrence_operation_preview()
    }

    pub(crate) fn show_linear_pattern_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingLinearPattern>() else {
            return;
        };
        let previous_axis = pending.axis;
        let previous_spacing = pending.spacing.clone();
        let previous_count = pending.count.clone();
        let mut axis = previous_axis;
        let mut spacing = previous_spacing.clone();
        let mut count = previous_count.clone();
        let parse = |axis: Axis, spacing: &str, count: &str| {
            let spacing = spacing.trim().parse::<f64>().ok()?;
            let count = count.trim().parse::<usize>().ok()?;
            (spacing.is_finite()
                && spacing.abs() > f64::EPSILON
                && (2..=MAX_PATTERN_COUNT).contains(&count))
            .then_some((axis, spacing, count))
        };
        let mut current = parse(axis, &spacing, &count);
        let binding_is_current = self.linear_pattern_binding_is_current(pending);
        let preview_is_current = self.linear_pattern_preview_is_current();
        let mut open = true;
        let mut preview = false;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-linear-pattern-title"))
            .id(egui::Id::new("linear-pattern"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.text("dialog-linear-pattern-axis"));
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut axis,
                        Axis::X,
                        self.catalog.text("dialog-linear-pattern-axis-x"),
                    );
                    ui.selectable_value(
                        &mut axis,
                        Axis::Y,
                        self.catalog.text("dialog-linear-pattern-axis-y"),
                    );
                    ui.selectable_value(
                        &mut axis,
                        Axis::Z,
                        self.catalog.text("dialog-linear-pattern-axis-z"),
                    );
                });
                let spacing_label = self.catalog.text("dialog-linear-pattern-spacing");
                ui.label(&spacing_label);
                let spacing_input = ui.add(egui::TextEdit::singleline(&mut spacing));
                spacing_input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &spacing_label)
                });
                let count_label = self.catalog.text("dialog-linear-pattern-count");
                ui.label(&count_label);
                let count_input = ui.add(egui::TextEdit::singleline(&mut count));
                count_input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &count_label)
                });
                current = parse(axis, &spacing, &count);
                ui.separator();
                ui.horizontal(|ui| {
                    preview = ui
                        .add_enabled(
                            binding_is_current && current.is_some(),
                            egui::Button::new(self.catalog.text("dialog-linear-pattern-preview")),
                        )
                        .clicked();
                    confirm = ui
                        .add_enabled(
                            preview_is_current,
                            egui::Button::new(self.catalog.text("dialog-linear-pattern-confirm")),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-linear-pattern-cancel"))
                        .clicked();
                });
            });
        let changed =
            axis != previous_axis || spacing != previous_spacing || count != previous_count;
        if let Some(pending) = self.modal.get_mut::<PendingLinearPattern>() {
            pending.axis = axis;
            pending.spacing = spacing;
            pending.count = count;
            if changed {
                pending.preview_plan = None;
            }
        }
        if changed {
            self.tool_preview.close::<OccurrenceOperationPreview>();
        }
        if cancel || !open {
            self.modal.close::<PendingLinearPattern>();
            self.tool_preview.close::<OccurrenceOperationPreview>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if preview {
            self.preview_pending_linear_pattern();
        } else if confirm {
            self.confirm_linear_pattern();
        }
    }

    pub(crate) fn show_rectangular_pattern_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingRectangularPattern>() else {
            return;
        };
        let binding_is_current = self.rectangular_pattern_binding_is_current(pending);
        let previous_primary_axis = pending.primary_axis;
        let previous_primary_spacing = pending.primary_spacing.clone();
        let previous_primary_count = pending.primary_count.clone();
        let previous_secondary_axis = pending.secondary_axis;
        let previous_secondary_spacing = pending.secondary_spacing.clone();
        let previous_secondary_count = pending.secondary_count.clone();
        let mut primary_axis = previous_primary_axis;
        let mut primary_spacing = previous_primary_spacing.clone();
        let mut primary_count = previous_primary_count.clone();
        let mut secondary_axis = previous_secondary_axis;
        let mut secondary_spacing = previous_secondary_spacing.clone();
        let mut secondary_count = previous_secondary_count.clone();
        let parse = |primary_axis: Axis,
                     primary_spacing: &str,
                     primary_count: &str,
                     secondary_axis: Axis,
                     secondary_spacing: &str,
                     secondary_count: &str| {
            let primary_spacing = primary_spacing.trim().parse::<f64>().ok()?;
            let primary_count = primary_count.trim().parse::<usize>().ok()?;
            let secondary_spacing = secondary_spacing.trim().parse::<f64>().ok()?;
            let secondary_count = secondary_count.trim().parse::<usize>().ok()?;
            let total_count = primary_count.checked_mul(secondary_count)?;
            (primary_axis != secondary_axis
                && primary_spacing.is_finite()
                && primary_spacing.abs() > f64::EPSILON
                && secondary_spacing.is_finite()
                && secondary_spacing.abs() > f64::EPSILON
                && (2..=MAX_PATTERN_COUNT).contains(&primary_count)
                && (2..=MAX_PATTERN_COUNT).contains(&secondary_count)
                && total_count <= MAX_PATTERN_COUNT)
                .then_some((
                    primary_axis,
                    primary_spacing,
                    primary_count,
                    secondary_axis,
                    secondary_spacing,
                    secondary_count,
                ))
        };
        let mut current = parse(
            primary_axis,
            &primary_spacing,
            &primary_count,
            secondary_axis,
            &secondary_spacing,
            &secondary_count,
        );
        let preview_is_current = self.rectangular_pattern_preview_is_current();
        let mut open = true;
        let mut preview = false;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-rectangular-pattern-title"))
            .id(egui::Id::new("rectangular-pattern"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                for (axis, label_key) in [
                    (&mut primary_axis, "dialog-rectangular-pattern-primary-axis"),
                    (
                        &mut secondary_axis,
                        "dialog-rectangular-pattern-secondary-axis",
                    ),
                ] {
                    ui.label(self.catalog.text(label_key));
                    ui.horizontal(|ui| {
                        for (candidate, key) in [
                            (Axis::X, "dialog-rectangular-pattern-axis-x"),
                            (Axis::Y, "dialog-rectangular-pattern-axis-y"),
                            (Axis::Z, "dialog-rectangular-pattern-axis-z"),
                        ] {
                            ui.selectable_value(axis, candidate, self.catalog.text(key));
                        }
                    });
                }
                for (value, key) in [
                    (
                        &mut primary_spacing,
                        "dialog-rectangular-pattern-primary-spacing",
                    ),
                    (
                        &mut primary_count,
                        "dialog-rectangular-pattern-primary-count",
                    ),
                    (
                        &mut secondary_spacing,
                        "dialog-rectangular-pattern-secondary-spacing",
                    ),
                    (
                        &mut secondary_count,
                        "dialog-rectangular-pattern-secondary-count",
                    ),
                ] {
                    let label = self.catalog.text(key);
                    ui.label(&label);
                    let input = ui.add(egui::TextEdit::singleline(value));
                    input.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
                    });
                }
                current = parse(
                    primary_axis,
                    &primary_spacing,
                    &primary_count,
                    secondary_axis,
                    &secondary_spacing,
                    &secondary_count,
                );
                ui.separator();
                ui.horizontal(|ui| {
                    preview = ui
                        .add_enabled(
                            binding_is_current && current.is_some(),
                            egui::Button::new(
                                self.catalog.text("dialog-rectangular-pattern-preview"),
                            ),
                        )
                        .clicked();
                    confirm = ui
                        .add_enabled(
                            preview_is_current,
                            egui::Button::new(
                                self.catalog.text("dialog-rectangular-pattern-confirm"),
                            ),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-rectangular-pattern-cancel"))
                        .clicked();
                });
            });
        let changed = primary_axis != previous_primary_axis
            || primary_spacing != previous_primary_spacing
            || primary_count != previous_primary_count
            || secondary_axis != previous_secondary_axis
            || secondary_spacing != previous_secondary_spacing
            || secondary_count != previous_secondary_count;
        if let Some(pending) = self.modal.get_mut::<PendingRectangularPattern>() {
            pending.primary_axis = primary_axis;
            pending.primary_spacing = primary_spacing;
            pending.primary_count = primary_count;
            pending.secondary_axis = secondary_axis;
            pending.secondary_spacing = secondary_spacing;
            pending.secondary_count = secondary_count;
            if changed {
                pending.preview_plan = None;
            }
        }
        if changed {
            self.tool_preview.close::<OccurrenceOperationPreview>();
        }
        if cancel || !open {
            self.modal.close::<PendingRectangularPattern>();
            self.tool_preview.close::<OccurrenceOperationPreview>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if preview {
            self.preview_pending_rectangular_pattern();
        } else if confirm {
            self.confirm_rectangular_pattern();
        }
    }

    pub(crate) fn show_circular_pattern_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingCircularPattern>() else {
            return;
        };
        let binding_is_current = self.circular_pattern_binding_is_current(pending);
        let previous_axis = pending.axis;
        let previous_centre_x = pending.centre_x.clone();
        let previous_centre_y = pending.centre_y.clone();
        let previous_centre_z = pending.centre_z.clone();
        let previous_angle = pending.angle.clone();
        let previous_count = pending.count.clone();
        let mut axis = previous_axis;
        let mut centre_x = previous_centre_x.clone();
        let mut centre_y = previous_centre_y.clone();
        let mut centre_z = previous_centre_z.clone();
        let mut angle = previous_angle.clone();
        let mut count = previous_count.clone();
        let parse = |axis: Axis, x: &str, y: &str, z: &str, angle: &str, count: &str| {
            let centre = Vec3::new(
                x.trim().parse::<f64>().ok()?,
                y.trim().parse::<f64>().ok()?,
                z.trim().parse::<f64>().ok()?,
            );
            let angle = angle.trim().parse::<f64>().ok()?;
            let count = count.trim().parse::<usize>().ok()?;
            (centre.x.is_finite()
                && centre.y.is_finite()
                && centre.z.is_finite()
                && angle.is_finite()
                && (0.01..=360.0).contains(&angle.abs())
                && (2..=MAX_PATTERN_COUNT).contains(&count))
            .then_some((axis, centre, angle, count))
        };
        let mut current = parse(axis, &centre_x, &centre_y, &centre_z, &angle, &count);
        let preview_is_current = self.circular_pattern_preview_is_current();
        let mut open = true;
        let mut preview = false;
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-circular-pattern-title"))
            .id(egui::Id::new("circular-pattern"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.text("dialog-circular-pattern-axis"));
                ui.horizontal(|ui| {
                    for (candidate, key) in [
                        (Axis::X, "dialog-circular-pattern-axis-x"),
                        (Axis::Y, "dialog-circular-pattern-axis-y"),
                        (Axis::Z, "dialog-circular-pattern-axis-z"),
                    ] {
                        ui.selectable_value(&mut axis, candidate, self.catalog.text(key));
                    }
                });
                for (value, key) in [
                    (&mut centre_x, "dialog-circular-pattern-centre-x"),
                    (&mut centre_y, "dialog-circular-pattern-centre-y"),
                    (&mut centre_z, "dialog-circular-pattern-centre-z"),
                ] {
                    let label = self.catalog.text(key);
                    ui.label(&label);
                    let input = ui.add(egui::TextEdit::singleline(value));
                    input.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
                    });
                }
                let angle_label = self.catalog.text("dialog-circular-pattern-angle");
                ui.label(&angle_label);
                let angle_input = ui.add(egui::TextEdit::singleline(&mut angle));
                angle_input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &angle_label)
                });
                let count_label = self.catalog.text("dialog-circular-pattern-count");
                ui.label(&count_label);
                let count_input = ui.add(egui::TextEdit::singleline(&mut count));
                count_input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &count_label)
                });
                current = parse(axis, &centre_x, &centre_y, &centre_z, &angle, &count);
                ui.separator();
                ui.horizontal(|ui| {
                    preview = ui
                        .add_enabled(
                            binding_is_current && current.is_some(),
                            egui::Button::new(self.catalog.text("dialog-circular-pattern-preview")),
                        )
                        .clicked();
                    confirm = ui
                        .add_enabled(
                            preview_is_current,
                            egui::Button::new(self.catalog.text("dialog-circular-pattern-confirm")),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-circular-pattern-cancel"))
                        .clicked();
                });
            });
        let changed = axis != previous_axis
            || centre_x != previous_centre_x
            || centre_y != previous_centre_y
            || centre_z != previous_centre_z
            || angle != previous_angle
            || count != previous_count;
        if let Some(pending) = self.modal.get_mut::<PendingCircularPattern>() {
            pending.axis = axis;
            pending.centre_x = centre_x;
            pending.centre_y = centre_y;
            pending.centre_z = centre_z;
            pending.angle = angle;
            pending.count = count;
            if changed {
                pending.preview_plan = None;
            }
        }
        if changed {
            self.tool_preview.close::<OccurrenceOperationPreview>();
        }
        if cancel || !open {
            self.modal.close::<PendingCircularPattern>();
            self.tool_preview.close::<OccurrenceOperationPreview>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if preview {
            self.preview_pending_circular_pattern();
        } else if confirm {
            self.confirm_circular_pattern();
        }
    }
}
