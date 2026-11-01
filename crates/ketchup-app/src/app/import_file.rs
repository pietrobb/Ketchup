//! Import of a file named by its path, without the review window: a client (an AI
//! connected over MCP) gets the review summary in the answer instead. It prepares
//! the same preview plan and commits it through the same import as the window,
//! as one Undo step.

use crate::*;
use serde_json::{Value, json};

/// The import format a file extension stands for.
pub(crate) fn import_format(path: &Path) -> Option<ImportFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "step" | "stp" => ImportFormat::Step,
        "iges" | "igs" => ImportFormat::Iges,
        "stl" => ImportFormat::Stl,
        "dxf" | "dwg" => ImportFormat::Dxf,
        "glb" => ImportFormat::Glb,
        "kscene" => ImportFormat::SketchupScene,
        _ => return None,
    })
}

const UNITS: [(&str, ImportLengthUnit); 5] = [
    ("mm", ImportLengthUnit::Millimetre),
    ("cm", ImportLengthUnit::Centimetre),
    ("m", ImportLengthUnit::Metre),
    ("in", ImportLengthUnit::Inch),
    ("ft", ImportLengthUnit::Foot),
];

pub(crate) fn parse_unit(name: &str) -> Option<ImportLengthUnit> {
    UNITS
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, unit)| *unit)
}

/// At most this many entries of a review list go into an import answer.
const SUMMARY_ITEMS: usize = 20;

/// The first entries of `items` and their total count. The import has already
/// happened when the answer is written, so a large review must never push it
/// over the response limit, which would leave only `applied: true`.
pub(crate) fn bounded<T: serde::Serialize>(items: &[T]) -> (Value, usize) {
    (json!(&items[..items.len().min(SUMMARY_ITEMS)]), items.len())
}

fn unit_name(unit: ImportLengthUnit) -> &'static str {
    UNITS
        .iter()
        .find(|(_, candidate)| *candidate == unit)
        .map_or("mm", |(name, _)| name)
}

impl KetchupApp {
    /// Imports `path` as `format`. `unit` is the length unit of an STL file
    /// (millimetres when omitted) or of a DXF file that declares none; a unit the
    /// DXF file declares itself always wins.
    pub(crate) fn import_file(
        &mut self,
        format: ImportFormat,
        path: &Path,
        unit: Option<ImportLengthUnit>,
    ) -> Result<Value, ImportError> {
        let snapshot = self.document.current();
        match format {
            ImportFormat::Step => {
                let source = ImportSourcePlan::seal(
                    format,
                    path.to_owned(),
                    Self::read_step_source(path)?,
                    (),
                    &snapshot,
                );
                let plan = self.prepare_step_import_preview_plan(source)?;
                let parts = plan.evidence.placed_parts().map(|part| &part.exact);
                let (refused, refused_total) = bounded(&plan.evidence.refusals());
                let summary = json!({
                    "solids": parts.clone().map(|exact| exact.solid_count).sum::<u32>(),
                    "volume_mm3": parts.map(|exact| exact.volume_mm3).sum::<f64>(),
                    "refused": refused,
                    "refused_total": refused_total,
                });
                self.import_step_from(&PendingStepImport {
                    plan,
                    invalidated: false,
                })?;
                Ok(summary)
            }
            ImportFormat::Iges => {
                let source = ImportSourcePlan::seal(
                    format,
                    path.to_owned(),
                    Self::read_iges_source(path)?,
                    (),
                    &snapshot,
                );
                let plan = self.prepare_iges_import_preview_plan(source)?;
                let parts = plan.evidence.parts.iter().map(|part| &part.exact);
                let (refused, refused_total) = bounded(&plan.evidence.refusals());
                let summary = json!({
                    "solids": parts.clone().map(|exact| exact.solid_count).sum::<u32>(),
                    "volume_mm3": parts.map(|exact| exact.volume_mm3).sum::<f64>(),
                    "refused": refused,
                    "refused_total": refused_total,
                });
                self.import_iges_from(&PendingIgesImport {
                    plan,
                    invalidated: false,
                })?;
                Ok(summary)
            }
            ImportFormat::Stl => {
                let unit = unit.unwrap_or(ImportLengthUnit::Millimetre);
                let source = ImportSourcePlan::seal(
                    format,
                    path.to_owned(),
                    read_import_source(format, path)?,
                    unit,
                    &snapshot,
                );
                let plan = self.prepare_stl_import_preview_plan(source)?;
                let (diagnostics, diagnostics_total) = bounded(plan.review.diagnostics());
                let summary = json!({
                    "triangles": plan.review.triangles().len(),
                    "unit": unit_name(unit),
                    "diagnostics": diagnostics,
                    "diagnostics_total": diagnostics_total,
                });
                self.import_stl_from(&PendingStlImport {
                    plan,
                    review_error: None,
                    invalidated: false,
                })?;
                Ok(summary)
            }
            ImportFormat::Dxf => self.import_drawing(path, Self::read_dxf_source(path)?, unit),
            ImportFormat::SketchupScene => {
                let source = ImportSourcePlan::seal(
                    format,
                    path.to_owned(),
                    read_import_source(format, path)?,
                    (),
                    &snapshot,
                );
                let plan = self.prepare_sketchup_scene_import_preview_plan(source)?;
                let (diagnostics, diagnostics_total) = bounded(plan.review.diagnostics());
                let summary = json!({
                    "definitions": plan.review.definition_count(),
                    "instances": plan.review.instance_count(),
                    "triangles": plan.review.triangle_count(),
                    "diagnostics": diagnostics,
                    "diagnostics_total": diagnostics_total,
                });
                self.import_sketchup_scene_from(&PendingSketchupSceneImport {
                    plan,
                    invalidated: false,
                })?;
                Ok(summary)
            }
            ImportFormat::Glb => self.import_glb_file(path),
        }
    }

    /// Imports the drawing `bytes`, the bytes of the `.dxf` or `.dwg` file at
    /// `path`, like [`Self::import_file`].
    pub(crate) fn import_drawing(
        &mut self,
        path: &Path,
        bytes: Vec<u8>,
        unit: Option<ImportLengthUnit>,
    ) -> Result<Value, ImportError> {
        let format = ImportFormat::Dxf;
        let snapshot = self.document.current();
        let units = {
            let (dxf, _) = crate::app::dwg_import::drawing_dxf(path, &bytes)?;
            Self::inspect_dxf_for_review(&dxf)?.units()
        };
        let unit = if units.authority() == ImportUnitAuthority::FileDeclared {
            units.source_unit()
        } else {
            unit.ok_or_else(|| {
                ImportError::failed(
                    format,
                    "the DXF file declares no length unit; pass unit as mm, cm, m, in or ft",
                )
            })?
        };
        let plan = self.prepare_dxf_import_preview_plan(ImportSourcePlan::seal(
            format,
            path.to_owned(),
            bytes,
            unit,
            &snapshot,
        ))?;
        let (layers, layers_total) = bounded(plan.review.layers());
        let (diagnostics, diagnostics_total) = bounded(plan.review.diagnostics());
        let summary = json!({
            "profiles": plan.review.profiles().len(),
            "layers": layers,
            "layers_total": layers_total,
            "unit": unit_name(unit),
            "unit_from_file": units.authority() == ImportUnitAuthority::FileDeclared,
            "diagnostics": diagnostics,
            "diagnostics_total": diagnostics_total,
            "dwg": plan.dwg.as_ref().map(|dwg| json!({
                "version": dwg.version,
                "skipped_records": dwg.skipped.iter().map(|skipped| json!({
                    "record_type": skipped.record_type,
                    "count": skipped.count,
                })).collect::<Vec<_>>(),
            })),
        });
        self.import_dxf_from(&PendingDxfImport {
            plan,
            unit_confirmed: true,
            review_error: None,
            invalidated: false,
        })?;
        Ok(summary)
    }
}
