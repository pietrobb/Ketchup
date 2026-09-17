use super::*;
use serde_json::{Value, json};

pub const PRODUCTION_JOB_V1: &str = "ketchup.production-job.v1";

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
        let programs = projection.woodwop_mpr_4_0_production_package(snapshot, self.options)?;
        Ok(Value::Array(
            programs
                .into_iter()
                .map(|program| {
                    json!({
                        "filename": format!("{}.mpr", program.program_name),
                        "code": program.program_name,
                        "content": String::from_utf8(program.mpr).expect("woodWOP output is ASCII"),
                    })
                })
                .collect(),
        ))
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
    /// The stock axis map relates cutting-list L/W/T to these coordinates.
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
            let stock_frame = woodwop_stock_frame(&stock.machining)
                .ok_or(GeneralFabricationError::ExportBlocked)?;
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
                if resolved.definition_id != row.definition_id {
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
                parts.push(json!({
                    "instance_path": instance_path_value(path),
                    "definition_id": row.definition_id.0,
                    "code": code, "name": definition.name(),
                    "material_key": row.material_key,
                    "dimensions_mm": stock_frame.dimensions_mm,
                    "stock_definition_axes": stock_frame.definition_axes,
                    "coordinate_frame": "definition_local_mm",
                    "operations": operations,
                    "dowel_holes": dowels.remove(path).unwrap_or_default(),
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
            "schema": PRODUCTION_JOB_V1,
            "document_id": snapshot.document_id().0,
            "source_revision": snapshot.revision_id(),
            "source_digest": snapshot.canonical_digest(),
            "parts": parts, "outputs": outputs,
        }))
    }
}
