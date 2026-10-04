//! Project drawings of what is visible: floor plan, sections and elevations on
//! one SVG sheet, drawn from the exact solids. Hidden layers are left out, so
//! turning off the concept layer draws the construction.

use crate::*;
use ketchup_manufacturing::project_drawings::{
    DrawingSolid, ProjectDrawingError, ProjectSheet, ProjectSheetOptions, ProjectView,
    default_plan_cut_z, project_sheet,
};

/// Colour of a solid with no colour of its own.
const DEFAULT_SOLID_COLOR: [u8; 3] = [190, 190, 190];

/// Why the project drawings could not be made or written.
#[derive(Debug)]
pub(crate) enum ProjectDrawingsError {
    Drawing(ProjectDrawingError),
    Write(std::io::Error),
}

impl KetchupApp {
    /// Every visible occurrence with a published exact solid, in world millimetres.
    fn visible_drawing_solids(&self, snapshot: &Snapshot) -> Vec<DrawingSolid> {
        let exact = self.exact.results.render_by_definition(snapshot);
        snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter_map(|occurrence| {
                let package = exact.get(&occurrence.definition_id)?;
                let vertices = package.vertices();
                let world = |index: u32| {
                    let vertex = vertices.get(usize::try_from(index).ok()?)?;
                    Some(occurrence.transform.transform_point(vertex.position_mm))
                };
                let triangles = package
                    .triangles()
                    .iter()
                    .filter_map(|triangle| {
                        let [a, b, c] = triangle.vertex_indices;
                        Some([world(a)?, world(b)?, world(c)?])
                    })
                    .collect();
                Some(DrawingSolid {
                    name: occurrence.occurrence_name.clone(),
                    color: occurrence.color.unwrap_or(DEFAULT_SOLID_COLOR),
                    triangles,
                })
            })
            .collect()
    }

    /// The sheet of the visible exact solids, titled after the document.
    pub(crate) fn project_drawings(&self) -> Result<ProjectSheet, ProjectDrawingError> {
        let snapshot = self.document.current();
        let solids = self.visible_drawing_solids(&snapshot);
        let cut = default_plan_cut_z(&solids).ok_or(ProjectDrawingError::Empty)?;
        let title = self
            .file
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .map_or_else(
                || {
                    self.document
                        .current_rule_program()
                        .map_or_else(|| "Kečup".to_owned(), |program| program.file_name.clone())
                },
                str::to_owned,
            );
        let view_titles = ProjectView::ALL
            .into_iter()
            .map(|view| {
                let title = match view {
                    ProjectView::Plan => self.catalog.format(
                        "drawings-view-plan",
                        &BTreeMap::from([("height", format!("{cut:.0}"))]),
                    ),
                    ProjectView::LongitudinalSection => {
                        self.catalog.text("drawings-view-longitudinal")
                    }
                    ProjectView::CrossSection => self.catalog.text("drawings-view-cross"),
                    ProjectView::Front => self.catalog.text("drawings-view-front"),
                    ProjectView::Back => self.catalog.text("drawings-view-back"),
                    ProjectView::Left => self.catalog.text("drawings-view-left"),
                    ProjectView::Right => self.catalog.text("drawings-view-right"),
                };
                (view, title)
            })
            .collect();
        project_sheet(
            &solids,
            &ProjectSheetOptions {
                title,
                view_titles,
                plan_cut_z_mm: Some(cut),
            },
        )
    }

    pub(crate) fn project_drawings_error_text(&self, error: &ProjectDrawingsError) -> String {
        match error {
            ProjectDrawingsError::Drawing(ProjectDrawingError::Empty) => {
                self.catalog.text("drawings-unavailable")
            }
            ProjectDrawingsError::Write(error) => error.to_string(),
        }
    }

    /// Writes the project drawings of what is visible as one SVG sheet.
    pub fn export_project_drawings_to(&mut self, path: &Path) -> bool {
        let result = self
            .project_drawings()
            .map_err(ProjectDrawingsError::Drawing)
            .and_then(|sheet| {
                std::fs::write(path, &sheet.svg)
                    .map(|()| sheet.scale)
                    .map_err(ProjectDrawingsError::Write)
            });
        let path = path.display().to_string();
        self.digest = match &result {
            Ok(scale) => self.catalog.format(
                "digest-exported-project-drawings",
                &BTreeMap::from([("path", path), ("scale", scale.to_string())]),
            ),
            Err(error) => self.catalog.format(
                "error-export-project-drawings",
                &BTreeMap::from([
                    ("path", path),
                    ("reason", self.project_drawings_error_text(error)),
                ]),
            ),
        };
        result.is_ok()
    }
}
