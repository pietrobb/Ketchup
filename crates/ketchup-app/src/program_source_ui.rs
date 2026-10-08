use super::*;
use crate::program_evaluation::Lookup;
use ketchup_application::SourceLines;
use ketchup_model::document::RuleProgramSource;

/// Program evaluated once per source revision, so selecting parts stays cheap.
#[derive(Clone)]
struct ProgramSourceView {
    source: RuleProgramSource,
    parts: Result<BTreeMap<String, Vec<SourceLines>>, ketchup_program::ProgramError>,
    /// Part whose first line has already been scrolled into view.
    scrolled_to: Option<String>,
}

/// Marks lines that define the selected part, also for screen readers.
pub(super) const DEFINING_LINE_MARKER: &str = "›";

pub(super) fn program_source_line_label(number: usize, line: &str, defining: bool) -> String {
    let marker = if defining { DEFINING_LINE_MARKER } else { " " };
    format!("{marker}{number:>4}  {line}")
}

fn line_ranges(lines: &[SourceLines]) -> String {
    lines
        .iter()
        .map(|lines| {
            if lines.first == lines.last {
                lines.first.to_string()
            } else {
                format!("{}–{}", lines.first, lines.last)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl KetchupApp {
    /// Shows the whole Starlark program of a program-owned model and highlights
    /// the lines that create or change the selected part. Models without a
    /// program show nothing, so the dock does not grow for them.
    pub(super) fn show_program_source(&mut self, ui: &mut egui::Ui) {
        let Some(program) = self.document.current_rule_program() else {
            return;
        };
        let part = self
            .selected_instance_paths()
            .first()
            .and_then(|path| live_bridge::program_pick::part_name(&self.document.current(), path));
        egui::CollapsingHeader::new(self.catalog.text("program-source-title"))
            .id_salt("program-source")
            .default_open(true)
            .show(ui, |ui| {
                let id = ui.id().with("program-source-view");
                let kept = ui
                    .data_mut(|data| data.get_temp::<ProgramSourceView>(id))
                    .filter(|view| &view.source == program);
                let mut view = match kept {
                    Some(view) => view,
                    None => match self.program_evaluations.try_get(program) {
                        Lookup::Pending => {
                            ui.weak(self.catalog.text("status-program-planning"));
                            return;
                        }
                        Lookup::Ready(evaluation) => ProgramSourceView {
                            source: program.clone(),
                            parts: evaluation.map(|evaluated| evaluated.part_sources.clone()),
                            scrolled_to: None,
                        },
                    },
                };
                match &view.parts {
                    Err(error) => {
                        ui.label(self.catalog.format(
                            "program-source-error",
                            &BTreeMap::from([("error", error.message.clone())]),
                        ));
                    }
                    Ok(parts) => {
                        let lines = match &part {
                            None => {
                                ui.weak(self.catalog.text("program-source-select-hint"));
                                &[][..]
                            }
                            Some(part) => match parts.get(part) {
                                None => {
                                    ui.label(self.catalog.format(
                                        "program-source-part-missing",
                                        &BTreeMap::from([("part", part.clone())]),
                                    ));
                                    &[][..]
                                }
                                Some(lines) => {
                                    ui.label(self.catalog.format(
                                        "program-source-lines",
                                        &BTreeMap::from([
                                            ("part", part.clone()),
                                            ("lines", line_ranges(lines)),
                                        ]),
                                    ));
                                    lines.as_slice()
                                }
                            },
                        };
                        let scroll = !lines.is_empty() && view.scrolled_to != part;
                        show_source_lines(ui, &view.source.source, lines, scroll);
                        view.scrolled_to.clone_from(&part);
                    }
                }
                ui.data_mut(|data| data.insert_temp(id, view));
            });
        ui.separator();
    }
}

fn show_source_lines(ui: &mut egui::Ui, source: &str, lines: &[SourceLines], scroll: bool) {
    let highlight = ui.visuals().selection.bg_fill;
    let highlight_text = ui.visuals().strong_text_color();
    egui::ScrollArea::both()
        .id_salt("program-source-scroll")
        .max_height(360.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let mut scrolled = !scroll;
            for (index, line) in source.lines().enumerate() {
                let number = index + 1;
                let defining = lines
                    .iter()
                    .any(|lines| (lines.first..=lines.last).contains(&number));
                let mut text =
                    egui::RichText::new(program_source_line_label(number, line, defining))
                        .monospace();
                if defining {
                    text = text.background_color(highlight).color(highlight_text);
                } else {
                    text = text.weak();
                }
                let response = ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
                if defining && !scrolled {
                    response.scroll_to_me(Some(egui::Align::Center));
                    scrolled = true;
                }
            }
        });
}
