//! The macros of the woodWOP program of one part, shared by the program of one
//! part and each program of the production package (review 2026-11-08 MF-140).

use super::*;

/// The stock frame and macros of the woodWOP program of one part: its
/// drillings and pockets and the holes of its logical pin joints, laid as
/// [`woodwop_setup`] lays them. The program of one part and each program of
/// the production package are written by it alike (review 2026-11-08 MF-140:
/// the program of one part left the pin holes out).
pub(super) fn woodwop_part_macros(
    stock_frame: WoodwopStockFrame,
    machining: &[&GeneralManufacturingOperation],
    holes: &[&PinHole],
    pocket_tool: Option<u32>,
) -> Result<(WoodwopStockFrame, Vec<String>), GeneralFabricationError> {
    woodwop_setup(
        stock_frame,
        &machining_normals(
            machining.iter().copied(),
            holes.iter().map(|hole| hole.inward_unit_local),
        ),
        |stock_frame| {
            let mut macros = machining
                .iter()
                .map(|operation| match operation.kind {
                    GeneralManufacturingKind::CircularDrill => {
                        woodwop_drilling_macro(&operation.machining, stock_frame)
                    }
                    GeneralManufacturingKind::ProfileCut => pocket_tool.and_then(|tool| {
                        woodwop_vertical_pocket_macro(&operation.machining, stock_frame, tool)
                    }),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            for hole in holes {
                macros.push(woodwop_pin_macro(hole, stock_frame)?);
            }
            Some(macros)
        },
    )
}

/// Whether two copies are drilled for pin joints alike, in their own frames.
pub(super) fn same_pin_holes(first: &[&PinHole], other: &[&PinHole]) -> bool {
    let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= ROUNDING);
    first.len() == other.len()
        && first.iter().all(|hole| {
            other.iter().any(|copy| {
                close(&hole.entry_local_mm, &copy.entry_local_mm)
                    && close(&hole.inward_unit_local, &copy.inward_unit_local)
                    && close(
                        &[hole.diameter_mm, hole.depth_mm],
                        &[copy.diameter_mm, copy.depth_mm],
                    )
            })
        })
}

/// The woodWOP programs of each made part, by part name, or why it gets none.
pub(super) type PartPrograms = Vec<(
    String,
    Result<Vec<(InstancePath, Vec<u8>)>, GeneralFabricationError>,
)>;

/// What a partial HOMAG package says it does not program, and why.
pub const HOMAG_PACKAGE_REPORT: &str = "not_programmed.txt";

/// A made part the partial HOMAG package has no program for, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct NotProgrammed {
    pub part: String,
    pub reason: GeneralFabricationError,
}

/// The HOMAG package of the parts that can be programmed and the parts that
/// cannot (review 2026-11-08 §5.2): one part the machine cannot take does not
/// keep the others from the shop.
#[derive(Clone, Debug, PartialEq)]
pub struct HomagPartialPackage {
    pub programs: Vec<HomagProductionProgram>,
    pub not_programmed: Vec<NotProgrammed>,
}

impl HomagPartialPackage {
    /// The programs as one stored ZIP and, when a part has none,
    /// [`HOMAG_PACKAGE_REPORT`] naming each such part with its reason. With
    /// no program at all it is refused for the first part's reason.
    pub fn zip(&self) -> Result<Vec<u8>, GeneralFabricationError> {
        if self.programs.is_empty() {
            return Err(self
                .not_programmed
                .first()
                .map_or(GeneralFabricationError::NoPieceToProgram, |part| {
                    part.reason.clone()
                }));
        }
        let report = (!self.not_programmed.is_empty()).then(|| {
            let one_line = |text: &str| text.replace(['\r', '\n'], " ");
            std::iter::once("Not programmed:\n".to_owned())
                .chain(self.not_programmed.iter().map(|part| {
                    format!(
                        "- {}: {}\n",
                        one_line(&part.part),
                        one_line(&part.reason.to_string())
                    )
                }))
                .collect::<String>()
        });
        programs_zip(&self.programs, report.as_deref())
    }
}

/// The production program of a piece with its label.
pub(super) fn homag_production_program(
    snapshot: &Snapshot,
    instance_path: InstancePath,
    mpr: Vec<u8>,
) -> Result<HomagProductionProgram, GeneralFabricationError> {
    let (program_name, barcode_svg) = homag_program_label(snapshot, &instance_path)?;
    Ok(HomagProductionProgram {
        schema: HOMAG_BHX_PRODUCTION_PACKAGE_V1,
        program_name,
        instance_path,
        mpr,
        barcode_svg,
    })
}

fn programs_zip(
    programs: &[HomagProductionProgram],
    report: Option<&str>,
) -> Result<Vec<u8>, GeneralFabricationError> {
    let names = programs
        .iter()
        .map(|program| {
            [
                format!("{}.mpr", program.program_name),
                format!("{}.svg", program.program_name),
            ]
        })
        .collect::<Vec<_>>();
    let mut entries = programs
        .iter()
        .zip(&names)
        .flat_map(|(program, [mpr, svg])| {
            [
                (mpr.as_str(), program.mpr.as_slice()),
                (svg.as_str(), program.barcode_svg.as_slice()),
            ]
        })
        .collect::<Vec<_>>();
    if let Some(report) = report {
        entries.push((HOMAG_PACKAGE_REPORT, report.as_bytes()));
    }
    crate::three_mf_export::encode_package(&entries, usize::MAX)
        .map_err(GeneralFabricationError::Package)
}

impl GeneralFabricationProjection {
    /// The production package as one stored ZIP: `<code>.mpr` and its
    /// barcode label `<code>.svg` for every piece the package programs.
    pub fn woodwop_mpr_4_0_production_zip(
        &self,
        snapshot: &Snapshot,
        options: WoodwopMprOptions,
    ) -> Result<Vec<u8>, GeneralFabricationError> {
        let programs = self.woodwop_mpr_4_0_production_package(snapshot, options)?;
        if programs.is_empty() {
            return Err(GeneralFabricationError::NoPieceToProgram);
        }
        programs_zip(&programs, None)
    }

    /// The production package of every made part that can be programmed, and
    /// each part that cannot with its reason, including an element the
    /// machine cannot make (a through hole, a thread). Whatever refuses the
    /// whole model (its validation, a pocket tool that is no tool number)
    /// still refuses the package.
    pub fn woodwop_mpr_4_0_partial_production_package(
        &self,
        snapshot: &Snapshot,
        options: WoodwopMprOptions,
    ) -> Result<HomagPartialPackage, GeneralFabricationError> {
        let mut package = HomagPartialPackage {
            programs: Vec::new(),
            not_programmed: Vec::new(),
        };
        for (part, written) in self.woodwop_programs_by_part(snapshot, options, false)? {
            let programs = written.and_then(|programs| {
                programs
                    .into_iter()
                    .map(|(path, mpr)| homag_production_program(snapshot, path, mpr))
                    .collect::<Result<Vec<_>, _>>()
            });
            match programs {
                Ok(programs) => package.programs.extend(programs),
                Err(reason) => package.not_programmed.push(NotProgrammed { part, reason }),
            }
        }
        Ok(package)
    }

    /// The woodWOP programs of each made part, by part name, or why the part
    /// gets none; an error of the whole model refuses them all. With
    /// `whole_model_gate` the first part the machine cannot make refuses them
    /// all too, as the strict package does.
    pub(super) fn woodwop_programs_by_part(
        &self,
        snapshot: &Snapshot,
        options: WoodwopMprOptions,
        whole_model_gate: bool,
    ) -> Result<PartPrograms, GeneralFabricationError> {
        self.bom_export(snapshot)?;
        self.manufacturing_export(snapshot)?;
        if let Some(tool) = options
            .vertical_pocket_tool_number
            .filter(|tool| *tool == 0 || *tool > 999_999)
        {
            return Err(GeneralFabricationError::PocketToolNumberInvalid { tool });
        }
        let format = ShopFormat::Woodwop {
            vertical_pocket_tool: options.vertical_pocket_tool_number.is_some(),
        };
        if whole_model_gate {
            self.admit(snapshot, format)?;
        }
        let pin_holes = capabilities::undrilled_pin_holes(snapshot)?;
        let holes = pin_holes.iter().map(|(_, hole)| hole).collect::<Vec<_>>();
        let mut parts = Vec::new();
        let mut program_paths = BTreeSet::new();
        for row in self
            .bom
            .rows
            .iter()
            .filter(|row| format.writes(row.item_kind))
        {
            let part = snapshot
                .definition(row.definition_id)
                .map_or("", |definition| definition.name());
            let mut write_part =
                || -> Result<Vec<(InstancePath, Vec<u8>)>, GeneralFabricationError> {
                    let mut programs = Vec::new();
                    self.admit_row(snapshot, format, row, &pin_holes)?;
                    let GeneralBodySource::Exact(_) = &row.source else {
                        return Err(GeneralFabricationError::ExportBlocked(
                            ExportBlock::NotExactBody,
                        ));
                    };
                    let matching = self
                        .manufacturing
                        .operations
                        .iter()
                        .filter(|operation| is_row_operation(row, operation))
                        .collect::<Vec<_>>();
                    let [stock, machining @ ..] = matching.as_slice() else {
                        return Err(GeneralFabricationError::ExportBlocked(
                            ExportBlock::ResultMismatch,
                        ));
                    };
                    if stock.kind != GeneralManufacturingKind::Stock
                        || !stock.semantic_inputs.is_empty()
                        || machining.iter().any(|operation| {
                            !matches!(
                                operation.kind,
                                GeneralManufacturingKind::CircularDrill
                                    | GeneralManufacturingKind::ProfileCut
                            )
                        })
                    {
                        return Err(GeneralFabricationError::ExportBlocked(
                            ExportBlock::UnsupportedMachining,
                        ));
                    }
                    // No copy gets a program, so the blank need not be one a
                    // program is set up on (review 2026-11-03 MF-141).
                    if machining.is_empty()
                        && !holes
                            .iter()
                            .any(|hole| row.instances.contains(&hole.instance_path))
                    {
                        return Ok(programs);
                    }
                    let stock_frame = woodwop_stock_frame(&stock.machining).ok_or(
                        GeneralFabricationError::ExportBlocked(ExportBlock::UnsupportedMachining),
                    )?;
                    for instance_path in &row.instances {
                        let resolved = snapshot
                            .resolve_instance_path(instance_path)
                            .map_err(GeneralFabricationError::UnresolvedInstance)?;
                        if resolved.definition_id != row.definition_id
                            || !is_production_transform(resolved.world_transform)
                        {
                            return Err(GeneralFabricationError::ExportBlocked(
                                ExportBlock::UnusablePlacement,
                            ));
                        }
                        let instance_holes = holes
                            .iter()
                            .copied()
                            .filter(|hole| hole.instance_path == *instance_path)
                            .collect::<Vec<_>>();
                        if machining.is_empty() && instance_holes.is_empty() {
                            continue;
                        }
                        let (stock_frame, macros) = woodwop_part_macros(
                            stock_frame,
                            machining,
                            &instance_holes,
                            options.vertical_pocket_tool_number,
                        )?;
                        if !program_paths.insert(instance_path.clone()) {
                            return Err(GeneralFabricationError::ExportBlocked(
                                ExportBlock::DuplicateName,
                            ));
                        }
                        let mpr = woodwop_mpr_output(
                            stock_frame,
                            &macros,
                            HOMAG_BHX_PRODUCTION_PACKAGE_V1,
                        )?;
                        programs.push((instance_path.clone(), mpr));
                    }
                    Ok(programs)
                };
            parts.push((
                part.to_owned(),
                write_part().map_err(|error| error.at_part(part)),
            ));
        }
        Ok(parts)
    }
}
