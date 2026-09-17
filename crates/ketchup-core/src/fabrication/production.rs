use super::*;
use serde_json::{Value, json};

pub const PRODUCTION_JOB_V2: &str = "ketchup.production-job.v2";

/// Explicitly selected, trusted postprocessor. No machine is selected by default.
pub trait ProductionAdapter {
    fn id(&self) -> &str;
    fn render(
        &self,
        projection: &GeneralFabricationProjection,
        snapshot: &Snapshot,
    ) -> Result<Value, GeneralFabricationError>;
}

pub struct HomagWoodwopAdapter {
    pub options: WoodwopMprOptions,
}

impl ProductionAdapter for HomagWoodwopAdapter {
    fn id(&self) -> &str {
        "homag-woodwop4"
    }

    fn render(
        &self,
        projection: &GeneralFabricationProjection,
        snapshot: &Snapshot,
    ) -> Result<Value, GeneralFabricationError> {
        let programs = projection.woodwop_mpr_4_0_setup_a_programs(snapshot, self.options)?;
        programs
            .into_iter()
            .map(|(path, mpr)| {
                let code = snapshot
                    .production_code(&path)
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                Ok(json!({
                    "filename": format!("{code}.mpr"),
                    "part_code": code, "setup_id": "A", "code": code,
                    "content": String::from_utf8(mpr).expect("woodWOP output is ASCII"),
                }))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array)
    }
}

#[must_use]
pub fn instance_path_value(path: &InstancePath) -> Value {
    json!({
        "root_occurrence_id": path.root_occurrence().0,
        "steps": path.steps().iter().map(|step| match step {
            InstancePathStep::Group(id) => json!({"kind": "group", "id": id.0}),
            InstancePathStep::Occurrence(id) => json!({"kind": "occurrence", "id": id.0}),
        }).collect::<Vec<_>>()
    })
}

impl GeneralFabricationProjection {
    /// One physical piece per entry, with definition-local machining coordinates.
    /// dimensions_mm are descending stock-frame bounding extents, not necessarily a cut piece.
    /// stock_frame maps those axes and their minimum corner to definition-local mm;
    /// its sorted axes may be left-handed. Only rectangular_prism denotes a rectangular blank.
    /// Original profiles and machining frames remain unchanged in operations.
    /// Setup A initially groups all machining; only a chosen postprocessor can validate
    /// that it fits one fixture. No opposite-face orientation or toolpath is inferred.
    pub fn production_job(
        &self,
        snapshot: &Snapshot,
        adapters: &[&dyn ProductionAdapter],
    ) -> Result<Value, GeneralFabricationError> {
        self.bom_export(snapshot)?;
        self.manufacturing_export(snapshot)?;
        let mut parts = Vec::new();
        let mut paths = BTreeSet::new();
        let mut codes = BTreeSet::new();
        let mut matched_operations = BTreeSet::new();
        let mut dowels = BTreeMap::<InstancePath, Vec<Value>>::new();
        for joint in snapshot.dowel_joints() {
            let projection = project_dowel_joint_contract(snapshot, joint)
                .map_err(|_| GeneralFabricationError::ExportBlocked)?;
            for pair in projection.pairs {
                for hole in [pair.first, pair.second] {
                    dowels
                        .entry(hole.instance_path.clone())
                        .or_default()
                        .push(json!({
                            "kind": "dowel_drill", "joint_id": joint.id.0,
                            "hole_id": hole.stable_hole_id,
                            "entry_local_mm": hole.entry_local_mm,
                            "inward_unit_local": hole.inward_unit_local,
                            "diameter_mm": hole.diameter_mm, "depth_mm": hole.depth_mm,
                        }));
                }
            }
        }
        for row in self
            .bom
            .rows
            .iter()
            .filter(|row| row.item_kind != GeneralBomItemKind::Purchased)
        {
            let GeneralBodySource::Exact(source) = &row.source else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            if row.quantity == 0 || row.quantity != row.instances.len() {
                return Err(GeneralFabricationError::ExportBlocked);
            }
            let matching = self
                .manufacturing
                .operations
                .iter()
                .enumerate()
                .filter(|(_, operation)| {
                    operation.definition_id == row.definition_id && operation.source == *source
                })
                .collect::<Vec<_>>();
            let Some((_, stock)) = matching.first() else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            if stock.kind != GeneralManufacturingKind::Stock || !stock.semantic_inputs.is_empty() {
                return Err(GeneralFabricationError::ExportBlocked);
            }
            let GeneralMachiningGeometry::TimberStock {
                frame,
                cross_section,
                start_mm,
                length_axis,
                length_mm,
                cross_section_width_mm,
                cross_section_height_mm,
            } = &stock.machining
            else {
                return Err(GeneralFabricationError::ExportBlocked);
            };
            let stock_dimensions = [
                *cross_section_width_mm,
                *cross_section_height_mm,
                *length_mm,
            ];
            let mut order = [0, 1, 2];
            order.sort_by(|left, right| {
                stock_dimensions[*right]
                    .total_cmp(&stock_dimensions[*left])
                    .then(left.cmp(right))
            });
            let dimensions_mm = order.map(|axis| stock_dimensions[axis]);
            let axes = [frame.x_axis, frame.y_axis, *length_axis];
            let mut minimum = [f64::INFINITY; 2];
            for segment in cross_section {
                let GeneralMachiningSegment::Line { start_mm, end_mm } = segment else {
                    return Err(GeneralFabricationError::ExportBlocked);
                };
                for axis in 0..2 {
                    minimum[axis] = minimum[axis].min(start_mm[axis]).min(end_mm[axis]);
                }
            }
            let origin_mm: [f64; 3] = std::array::from_fn(|axis| {
                start_mm[axis] + frame.x_axis[axis] * minimum[0] + frame.y_axis[axis] * minimum[1]
            });
            let stock_shape = if rectangular_stock_profile_dimensions(
                cross_section,
                *length_mm,
                *cross_section_width_mm,
                *cross_section_height_mm,
            )
            .is_some()
            {
                "rectangular_prism"
            } else {
                "profile_extrusion"
            };
            for (index, _) in &matching {
                matched_operations.insert(*index);
            }
            for path in &row.instances {
                let code = snapshot
                    .production_code(path)
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                if !paths.insert(path.clone()) || !codes.insert(code.to_owned()) {
                    return Err(GeneralFabricationError::ExportBlocked);
                }
                let resolved = snapshot
                    .resolve_instance_path(path)
                    .map_err(|_| GeneralFabricationError::ExportBlocked)?;
                if resolved.definition_id != row.definition_id
                    || !is_production_transform(resolved.world_transform)
                {
                    return Err(GeneralFabricationError::ExportBlocked);
                }
                let definition = snapshot
                    .definition(row.definition_id)
                    .ok_or(GeneralFabricationError::ExportBlocked)?;
                let operations = matching
                    .iter()
                    .map(|(_, operation)| {
                        json!({
                            "operation_id": operation.stable_operation_id,
                            "kind": operation.kind.token(),
                            "geometry": operation.machining,
                        })
                    })
                    .collect::<Vec<_>>();
                let holes = dowels.remove(path).unwrap_or_default();
                let setups = if operations.len() > 1 || !holes.is_empty() {
                    vec![json!({
                        "id": "A", "code": code,
                        "operation_ids": operations.iter().skip(1)
                            .map(|op| op["operation_id"].clone()).collect::<Vec<_>>(),
                        "dowel_hole_ids": holes.iter()
                            .map(|hole| hole["hole_id"].clone()).collect::<Vec<_>>(),
                    })]
                } else {
                    vec![]
                };
                parts.push(json!({
                    "instance_path": instance_path_value(path),
                    "definition_id": row.definition_id.0,
                    "code": code, "name": definition.name(),
                    "material_key": row.material_key,
                    "dimensions_mm": dimensions_mm,
                    "stock_frame": {"origin_mm": origin_mm, "axes": order.map(|axis| axes[axis])},
                    "stock_shape": stock_shape,
                    "coordinate_frame": "definition_local_mm",
                    "operations": operations,
                    "dowel_holes": holes,
                    "machining_setups": setups,
                }));
            }
        }
        if parts.is_empty()
            || !dowels.is_empty()
            || matched_operations.len() != self.manufacturing.operations.len()
        {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        let mut outputs = serde_json::Map::new();
        for adapter in adapters {
            if adapter.id().is_empty() || outputs.contains_key(adapter.id()) {
                return Err(GeneralFabricationError::ExportBlocked);
            }
            outputs.insert(adapter.id().to_owned(), adapter.render(self, snapshot)?);
        }
        Ok(json!({
            "schema": PRODUCTION_JOB_V2,
            "document_id": snapshot.document_id().0,
            "source_revision": snapshot.revision_id(),
            "source_digest": snapshot.canonical_digest(),
            "parts": parts, "outputs": outputs,
        }))
    }
}
