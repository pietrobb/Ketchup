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
    /// stock_frame maps those axes and a corner of the blank to definition-local mm;
    /// it is right-handed, the frame of the woodWOP programs. Only rectangular_prism
    /// denotes a rectangular blank.
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
        self.admit(snapshot, ShopFormat::ProductionJob)?;
        let mut parts = Vec::new();
        let mut paths = BTreeSet::new();
        let mut codes = BTreeSet::new();
        let mut matched_operations = BTreeSet::new();
        let mut pins = BTreeMap::<InstancePath, Vec<Value>>::new();
        for (joint, hole) in super::capabilities::undrilled_pin_holes(snapshot)? {
            pins.entry(hole.instance_path.clone())
                .or_default()
                .push(json!({
                    "kind": "pin_drill", "joint_id": joint.id.0,
                    "hole_id": hole.stable_hole_id,
                    "entry_local_mm": hole.entry_local_mm,
                    "inward_unit_local": hole.inward_unit_local,
                    "diameter_mm": hole.diameter_mm, "depth_mm": hole.depth_mm,
                }));
        }
        for row in self
            .bom
            .rows
            .iter()
            .filter(|row| ShopFormat::ProductionJob.writes(row.item_kind))
        {
            let part = snapshot
                .definition(row.definition_id)
                .ok_or(GeneralFabricationError::ExportBlocked)?
                .name();
            let (matching, frame) = row_stock(row, &self.manufacturing.operations)?;
            let JobStockFrame {
                origin_mm,
                axes: stock_axes,
                dimensions_mm,
                rectangular,
            } = frame;
            let stock_shape = if rectangular {
                "rectangular_prism"
            } else {
                "profile_extrusion"
            };
            for (index, _) in &matching {
                matched_operations.insert(*index);
            }
            for path in &row.instances {
                let code = snapshot.production_code(path).ok_or_else(|| {
                    GeneralFabricationError::PartWithoutProductionCode {
                        part: part.to_owned(),
                    }
                })?;
                if !paths.insert(path.clone()) || !codes.insert(code.to_owned()) {
                    return Err(GeneralFabricationError::ExportBlocked);
                }
                let resolved = snapshot
                    .resolve_instance_path(path)
                    .map_err(GeneralFabricationError::UnresolvedInstance)?;
                if resolved.definition_id != row.definition_id
                    || !is_production_transform(resolved.world_transform)
                {
                    return Err(GeneralFabricationError::ExportBlocked);
                }
                let operations = matching
                    .iter()
                    .map(|(_, operation)| {
                        let mut value = json!({
                            "operation_id": operation.stable_operation_id,
                            "kind": operation.kind.token(),
                            "geometry": operation.machining,
                        });
                        if let Some(thread) = &operation.thread {
                            value["thread"] = json!({
                                "size": thread.size, "pitch_mm": thread.pitch_mm,
                                "depth_mm": thread.depth_mm,
                            });
                        }
                        value
                    })
                    .collect::<Vec<_>>();
                let holes = pins.remove(path).unwrap_or_default();
                let setups = if operations.len() > 1 || !holes.is_empty() {
                    vec![json!({
                        "id": "A", "code": code,
                        "operation_ids": operations.iter().skip(1)
                            .map(|op| op["operation_id"].clone()).collect::<Vec<_>>(),
                        "pin_hole_ids": holes.iter()
                            .map(|hole| hole["hole_id"].clone()).collect::<Vec<_>>(),
                    })]
                } else {
                    vec![]
                };
                parts.push(json!({
                    "instance_path": instance_path_value(path),
                    "definition_id": row.definition_id.0,
                    "code": code, "name": part,
                    "material_key": row.material_key,
                    "dimensions_mm": dimensions_mm,
                    "stock_frame": {"origin_mm": origin_mm, "axes": stock_axes},
                    "stock_shape": stock_shape,
                    "coordinate_frame": "definition_local_mm",
                    "operations": operations,
                    "pin_holes": holes,
                    "machining_setups": setups,
                }));
            }
        }
        if parts.is_empty()
            || !pins.is_empty()
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

/// A row's operations with their index in the projection's operation list.
pub(super) type RowOperations<'a> = Vec<(usize, &'a GeneralManufacturingOperation)>;

/// The operations that machine `row`, its stock first, and the frame of
/// that stock. These refusals guard the projection's own consistency, which
/// the exports a production job runs first already check, so they stay the
/// general ExportBlocked: PartExportBlocked gives a machine file's reasons
/// (stock shape, cut reach), not the job's.
pub(super) fn row_stock<'a>(
    row: &GeneralBomRow,
    operations: &'a [GeneralManufacturingOperation],
) -> Result<(RowOperations<'a>, JobStockFrame), GeneralFabricationError> {
    let blocked = || GeneralFabricationError::ExportBlocked;
    if !matches!(row.source, GeneralBodySource::Exact(_))
        || row.quantity == 0
        || row.quantity != row.instances.len()
    {
        return Err(blocked());
    }
    let matching = operations
        .iter()
        .enumerate()
        .filter(|(_, operation)| is_row_operation(row, operation))
        .collect::<Vec<_>>();
    let Some((_, stock)) = matching.first() else {
        return Err(blocked());
    };
    if stock.kind != GeneralManufacturingKind::Stock || !stock.semantic_inputs.is_empty() {
        return Err(blocked());
    }
    let frame = job_stock_frame(&stock.machining).ok_or_else(blocked)?;
    Ok((matching, frame))
}

/// The frame a production job gives a piece: its blank's axes from the
/// longest to the shortest extent, from a corner of the blank, in
/// definition-local mm. Sorting the axes may reflect them; like the woodWOP
/// program, the second axis then runs from the far edge, so the frame stays
/// right-handed and a postprocessor following it machines the part, not its
/// mirror image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct JobStockFrame {
    pub origin_mm: [f64; 3],
    pub axes: [[f64; 3]; 3],
    pub dimensions_mm: [f64; 3],
    /// The blank is a rectangular prism, not a profile extrusion.
    pub rectangular: bool,
}

pub(super) fn job_stock_frame(stock: &GeneralMachiningGeometry) -> Option<JobStockFrame> {
    let GeneralMachiningGeometry::PrismaticStock {
        frame,
        cross_section,
        start_mm,
        length_axis,
        length_mm,
        cross_section_width_mm,
        cross_section_height_mm,
    } = stock
    else {
        return None;
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
    let minimum = profile_minimum(cross_section)?;
    let mut origin_mm: [f64; 3] = std::array::from_fn(|axis| {
        start_mm[axis] + frame.x_axis[axis] * minimum[0] + frame.y_axis[axis] * minimum[1]
    });
    let mut stock_axes = order.map(|axis| axes[axis]);
    let [x, y, z] = stock_axes;
    if dot(x, cross(y, z)) < 0.0 {
        for axis in 0..3 {
            origin_mm[axis] += stock_axes[1][axis] * dimensions_mm[1];
            stock_axes[1][axis] = -stock_axes[1][axis];
        }
    }
    Some(JobStockFrame {
        origin_mm,
        axes: stock_axes,
        dimensions_mm,
        rectangular: rectangular_stock_profile(
            cross_section,
            *length_mm,
            *cross_section_width_mm,
            *cross_section_height_mm,
        )
        .is_some(),
    })
}
