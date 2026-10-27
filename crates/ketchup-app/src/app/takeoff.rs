//! Material takeoff of the program parts: a window with the totals and a CSV
//! export. A program that declares `production(tags=…)` counts exactly those
//! parts, shown or hidden; otherwise parts on hidden layers are not counted,
//! so turning off the concept layer counts only the construction.

use crate::app_state::{TakeoffCache, TakeoffError};
use crate::*;
use ketchup_program::ProgramModel;
use ketchup_program::bom::{bar_schedule_table, bom, hardware_table};
use ketchup_program::cut_list::cut_list_table;
use ketchup_program::cutting::cutting_table;
use ketchup_program::quote::quote_table;
use ketchup_program::table::DecimalSeparator;
use ketchup_program::takeoff::{Takeoff, material_takeoff};

/// A list of the program's parts the takeoff window exports as a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartList {
    CutList,
    Hardware,
    BarSchedule,
    CuttingPlan,
    Quote,
}

impl PartList {
    /// The middle of its message keys, e.g. `digest-exported-hardware`.
    const fn key(self) -> &'static str {
        match self {
            Self::CutList => "cut-list",
            Self::Hardware => "hardware",
            Self::BarSchedule => "bar-schedule",
            Self::CuttingPlan => "cutting-plan",
            Self::Quote => "quote",
        }
    }
}

impl KetchupApp {
    /// The counted program parts by program name, each with its published
    /// exact solid volume when there is one: with a production scope every
    /// part (the model keeps those in scope), else the visible ones.
    fn takeoff_parts(
        &self,
        snapshot: &Snapshot,
        model: &ProgramModel,
        counted: impl Fn(&InstancePath) -> bool,
    ) -> BTreeMap<String, Option<f64>> {
        let exact = self.exact.results.render_by_definition(snapshot);
        let every = !model.production_scope.is_empty();
        snapshot
            .scene_query()
            .into_iter()
            .filter(|part| (every || part.visible) && counted(&part.instance_path))
            .filter_map(|part| {
                let name =
                    ketchup_application::rule_program_part_name(snapshot, &part.instance_path)?;
                let volume = exact
                    .get(&part.definition_id)
                    .map(|package| match package.as_ref() {
                        ExactBodyPackage::Graph(graph) => graph.volume_mm3,
                        ExactBodyPackage::Imported(imported) => imported.volume_mm3,
                    });
                Some((name, volume))
            })
            .collect()
    }

    /// The material takeoff of the current program's visible parts, or why
    /// there is none.
    pub(crate) fn material_takeoff(&self) -> Result<Arc<Takeoff>, TakeoffError> {
        let Some(program) = self.document.current_rule_program() else {
            return Err(TakeoffError::NoProgram);
        };
        let snapshot = self.document.current();
        let key = (
            snapshot.document_id(),
            snapshot.revision_id(),
            self.exact.results.contents_stamp(),
        );
        let mut cache = self.takeoff.cache.borrow_mut();
        let same_program = cache.as_ref().is_some_and(|cached| {
            cached.program.0 == program.file_name
                && cached.program.1 == program.source
                && cached.program.2 == program.overrides
        });
        if !same_program {
            // The window asks every frame; it says it is planning until the
            // program's background evaluation is ready.
            let crate::program_evaluation::Lookup::Ready(evaluation) =
                self.program_evaluations.try_get(program)
            else {
                return Err(TakeoffError::Planning);
            };
            let model = evaluation.map(|evaluated| Arc::new(evaluated.model.clone()));
            *cache = Some(TakeoffCache {
                program: (
                    program.file_name.clone(),
                    program.source.clone(),
                    program.overrides.clone(),
                ),
                model,
                key,
                takeoff: Err(TakeoffError::NoProgram),
            });
        }
        let cached = cache.as_mut().expect("the cache was just filled");
        if !same_program || cached.key != key {
            cached.key = key;
            cached.takeoff = cached
                .model
                .as_ref()
                .map_err(|error| TakeoffError::Program(error.clone()))
                .map(|model| {
                    Arc::new(material_takeoff(
                        model,
                        &self.takeoff_parts(&snapshot, model, |_| true),
                    ))
                });
        }
        cached.takeoff.clone()
    }

    /// The material takeoff of the selected visible program parts: a summary
    /// of one wall or one floor of a large model.
    pub(crate) fn selected_material_takeoff(&self) -> Result<Takeoff, TakeoffError> {
        self.material_takeoff()?;
        let selected = self.selected_instance_paths();
        let snapshot = self.document.current();
        let cache = self.takeoff.cache.borrow();
        let model = cache
            .as_ref()
            .and_then(|cached| cached.model.as_ref().ok())
            .ok_or(TakeoffError::NoProgram)?;
        let counted = self.takeoff_parts(&snapshot, model, |path| {
            selected.iter().any(|selected| {
                selected == path
                    || (selected.steps().is_empty()
                        && selected.root_occurrence() == path.root_occurrence())
            })
        });
        Ok(material_takeoff(model, &counted))
    }

    /// Writes the takeoff of the counted parts as CSV with the system's
    /// decimal separator.
    pub fn export_material_takeoff_to(&mut self, path: &Path) -> bool {
        let decimal = self.csv_decimal();
        let result = self.material_takeoff().and_then(|takeoff| {
            let csv = ketchup_application::program_variants::takeoff_table(&takeoff).csv(decimal);
            write_atomically(path, csv.as_bytes())
                .map_err(|error| TakeoffError::Write(Arc::new(error)))
        });
        let (key, mut arguments) = match &result {
            Ok(()) => ("digest-exported-material-takeoff", BTreeMap::new()),
            Err(reason) => (
                "error-export-material-takeoff",
                BTreeMap::from([("reason", self.takeoff_error_text(reason))]),
            ),
        };
        arguments.insert("path", path.display().to_string());
        self.digest = self.catalog.format(key, &arguments);
        result.is_ok()
    }

    /// How a CSV from the window writes numbers: with the separator its value
    /// fields read (the system's), never the interface language's (UI-9).
    fn csv_decimal(&self) -> DecimalSeparator {
        match self.value_box.decimal {
            crate::length_input::DecimalSeparator::Point => DecimalSeparator::Point,
            crate::length_input::DecimalSeparator::Comma => DecimalSeparator::Comma,
        }
    }

    /// Writes `list` as a table: XLSX for a `.xlsx` path, otherwise CSV with
    /// the system's decimal separator. The cut list has the
    /// counted parts, the hardware list, the bar schedule and the cutting plan
    /// are the rows the program report gives (MCP report sections `hardware`,
    /// `bar_schedule`, `cutting`).
    pub(crate) fn export_list_to(&mut self, list: PartList, path: &Path) -> bool {
        let result = self.material_takeoff().and_then(|_| {
            let snapshot = self.document.current();
            let table = {
                let cache = self.takeoff.cache.borrow();
                let model = cache
                    .as_ref()
                    .and_then(|cached| cached.model.as_ref().ok())
                    .ok_or(TakeoffError::NoProgram)?;
                match list {
                    PartList::CutList => {
                        let counted = self.takeoff_parts(&snapshot, model, |_| true);
                        cut_list_table(model, |name| counted.contains_key(name))
                    }
                    PartList::Hardware => hardware_table(&bom(model).hardware),
                    PartList::BarSchedule => bar_schedule_table(&bom(model).bar_schedule),
                    PartList::CuttingPlan => cutting_table(&bom(model).cutting),
                    PartList::Quote => {
                        quote_table(&bom(model).quote.ok_or(TakeoffError::NoPrices)?)
                    }
                }
            };
            let xlsx = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"));
            let bytes = if xlsx {
                table.xlsx()
            } else {
                table.csv(self.csv_decimal()).into_bytes()
            };
            write_atomically(path, &bytes).map_err(|error| TakeoffError::Write(Arc::new(error)))
        });
        let (key, mut arguments) = match &result {
            Ok(()) => (format!("digest-exported-{}", list.key()), BTreeMap::new()),
            Err(reason) => (
                format!("error-export-{}", list.key()),
                BTreeMap::from([("reason", self.takeoff_error_text(reason))]),
            ),
        };
        arguments.insert("path", path.display().to_string());
        self.digest = self.catalog.format(&key, &arguments);
        result.is_ok()
    }

    pub(crate) fn show_material_takeoff_window(&mut self, context: &egui::Context) {
        if !self.takeoff.open {
            return;
        }
        let takeoff = self.material_takeoff();
        let mut open = true;
        let mut export = None;
        egui::Window::new(self.catalog.text("window-material-takeoff"))
            .open(&mut open)
            .default_size([720.0, 480.0])
            .show(context, |ui| match &takeoff {
                Err(reason) => {
                    ui.label(self.takeoff_error_text(reason));
                }
                Ok(takeoff) => {
                    ui.label(self.catalog.format(
                        "takeoff-summary",
                        &BTreeMap::from([
                            ("counted", takeoff.counted_parts.to_string()),
                            ("excluded", takeoff.excluded_parts.to_string()),
                            ("exact", takeoff.exact_volume_parts.to_string()),
                        ]),
                    ));
                    if takeoff.outside_program_parts > 0 {
                        ui.label(self.catalog.format(
                            "takeoff-outside-program",
                            &BTreeMap::from([("count", takeoff.outside_program_parts.to_string())]),
                        ));
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(self.catalog.text("takeoff-export-csv")).clicked() {
                            export = Some(None);
                        }
                        for list in [
                            PartList::CutList,
                            PartList::Hardware,
                            PartList::BarSchedule,
                            PartList::CuttingPlan,
                            PartList::Quote,
                        ] {
                            for extension in ["csv", "xlsx"] {
                                let label = format!("takeoff-export-{}-{extension}", list.key());
                                if ui.button(self.catalog.text(&label)).clicked() {
                                    export = Some(Some((list, extension)));
                                }
                            }
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.takeoff_material_grid(ui, takeoff);
                        ui.separator();
                        self.takeoff_row_grid(ui, takeoff);
                    });
                }
            });
        if !open {
            self.takeoff.open = false;
        }
        match export {
            Some(None) => {
                if let Some(path) = self.choose_export_path("csv") {
                    self.export_material_takeoff_to(&path);
                }
            }
            Some(Some((list, extension))) => {
                if let Some(path) = self.choose_export_path(extension) {
                    self.export_list_to(list, &path);
                }
            }
            None => {}
        }
    }

    pub(crate) fn takeoff_error_text(&self, error: &TakeoffError) -> String {
        match error {
            TakeoffError::NoProgram => self.catalog.text("takeoff-no-program"),
            TakeoffError::Planning => self.catalog.text("status-program-planning"),
            TakeoffError::NoPrices => self.catalog.text("takeoff-no-prices"),
            TakeoffError::Program(error) => error.to_string(),
            TakeoffError::Write(error) => error.to_string(),
        }
    }

    fn takeoff_header(&self, ui: &mut egui::Ui, keys: &[&str]) {
        for key in keys {
            ui.strong(self.catalog.text(key));
        }
        ui.end_row();
    }

    fn takeoff_material_grid(&self, ui: &mut egui::Ui, takeoff: &Takeoff) {
        ui.heading(self.catalog.text("takeoff-materials"));
        egui::Grid::new("takeoff-materials")
            .striped(true)
            .show(ui, |ui| {
                self.takeoff_header(
                    ui,
                    &[
                        "takeoff-material",
                        "takeoff-count",
                        "takeoff-length",
                        "takeoff-area",
                        "takeoff-volume",
                    ],
                );
                for total in &takeoff.materials {
                    ui.label(&total.material);
                    ui.label(total.count.to_string());
                    ui.label(format!("{:.2}", total.length_m));
                    ui.label(format!("{:.2}", total.area_m2));
                    ui.label(self.takeoff_volume(total.volume_m3, total.volume_basis));
                    ui.end_row();
                }
            });
    }

    fn takeoff_row_grid(&self, ui: &mut egui::Ui, takeoff: &Takeoff) {
        ui.heading(self.catalog.text("takeoff-rows"));
        egui::Grid::new("takeoff-rows")
            .striped(true)
            .show(ui, |ui| {
                self.takeoff_header(
                    ui,
                    &[
                        "takeoff-category",
                        "takeoff-material",
                        "takeoff-section",
                        "takeoff-count",
                        "takeoff-length",
                        "takeoff-area",
                        "takeoff-volume",
                    ],
                );
                for row in &takeoff.rows {
                    ui.label(&row.category);
                    ui.label(&row.material);
                    ui.label(format!("{} × {}", row.section_mm[0], row.section_mm[1]));
                    ui.label(row.count.to_string());
                    ui.label(format!("{:.2}", row.length_m));
                    ui.label(format!("{:.2}", row.area_m2));
                    ui.label(self.takeoff_volume(row.volume_m3, row.volume_basis));
                    ui.end_row();
                }
            });
    }

    fn takeoff_volume(
        &self,
        volume_m3: f64,
        basis: ketchup_program::takeoff::VolumeBasis,
    ) -> String {
        format!(
            "{volume_m3:.3} ({})",
            self.catalog
                .text(&format!("takeoff-basis-{}", basis.as_str()))
        )
    }
}

/// Write through a temporary file beside `path`, so an interrupted export
/// never leaves a half-written takeoff in place of the previous one.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
