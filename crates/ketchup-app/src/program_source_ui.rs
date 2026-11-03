//! The dock's Program page: the whole Starlark program of a program-owned
//! model, coloured as code, with its functions as chips to jump to and the
//! lines that build the selected part highlighted; then the program's
//! parameters and variants.

use super::*;
use crate::program_evaluation::Lookup;
use crate::starlark_source::{self, Function, Token};
use crate::theme::CodeColors;
use ketchup_application::SourceLines;
use ketchup_model::document::RuleProgramSource;

/// Program evaluated once per source revision, so selecting parts stays cheap.
#[derive(Clone)]
struct ProgramSourceView {
    source: RuleProgramSource,
    parts: Result<BTreeMap<String, Vec<SourceLines>>, ketchup_program::ProgramError>,
    functions: Vec<Function>,
    /// Part whose first line has already been scrolled into view.
    scrolled_to: Option<String>,
    /// The line a function chip asked to bring to the top.
    jump_to: Option<usize>,
}

/// Marks lines that define the selected part, also for screen readers.
pub(super) const DEFINING_LINE_MARKER: &str = "›";

/// What a code line's label reads: marker, number and source.
pub(super) fn program_source_line_label(number: usize, line: &str, defining: bool) -> String {
    let marker = if defining { DEFINING_LINE_MARKER } else { " " };
    format!("{marker}{number:>4}  {line}")
}

fn line_ranges(lines: &[SourceLines]) -> String {
    let mut ranges = lines
        .iter()
        .map(|lines| (lines.first, lines.last))
        .collect::<Vec<_>>();
    ranges.dedup();
    ranges
        .into_iter()
        .map(|(first, last)| {
            if first == last {
                first.to_string()
            } else {
                format!("{first}–{last}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Height kept under the code: its frame's margins and its status line.
const FOOTER_HEIGHT: f32 = 48.0;

impl KetchupApp {
    /// The Program page of a program-owned model; it scrolls as a whole, the
    /// code filling what the dock shows of it.
    pub(super) fn show_program_tab(&mut self, ui: &mut egui::Ui) {
        let viewport = ui.available_height();
        dock_scroll_area().show(ui, |ui| {
            let top = ui.min_rect().top();
            self.show_tool_options(ui);
            self.show_program_source(ui, top + viewport);
            self.show_variants_section(ui);
            self.show_parameter_editor(ui);
            self.show_manual_cad_tools(ui);
        });
    }

    /// The program with the selected part's lines highlighted; a long one
    /// reaches down to `bottom`, the end of the dock, a short one leaves the
    /// variants and parameters in view.
    fn show_program_source(&mut self, ui: &mut egui::Ui, bottom: f32) {
        let Some(program) = self.document.current_rule_program() else {
            return;
        };
        let palette = self.palette();
        let part = self
            .selected_instance_paths()
            .first()
            .and_then(|path| live_bridge::program_pick::part_name(&self.document.current(), path));
        let id = egui::Id::new("program-source-view");
        let kept = ui
            .data_mut(|data| data.get_temp::<ProgramSourceView>(id))
            .filter(|view| &view.source == program);
        self.program_title(ui, palette, program);
        let mut view = match kept {
            Some(view) => view,
            None => match self.program_evaluations.try_get(program) {
                Lookup::Pending => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak(self.catalog.text("status-program-planning"));
                    });
                    return;
                }
                Lookup::Ready(evaluation) => ProgramSourceView {
                    source: program.clone(),
                    parts: evaluation.map(|evaluated| evaluated.part_sources.clone()),
                    functions: starlark_source::functions(&program.source),
                    scrolled_to: None,
                    jump_to: None,
                },
            },
        };
        let colors = palette.code();
        let lines = match &view.parts {
            Err(error) => {
                status_banner(
                    ui,
                    colors.error,
                    "⚠",
                    &self.catalog.format(
                        "program-source-error",
                        &BTreeMap::from([("error", error.message.clone())]),
                    ),
                    None,
                );
                Vec::new()
            }
            Ok(parts) => {
                let revision = self.catalog.format(
                    "program-source-revision",
                    &BTreeMap::from([(
                        "revision",
                        self.document.current().revision_id().to_string(),
                    )]),
                );
                status_banner(
                    ui,
                    colors.ok,
                    "✔",
                    &self.catalog.text("program-source-current"),
                    Some(&revision),
                );
                match &part {
                    None => {
                        ui.weak(self.catalog.text("program-source-select-hint"));
                        Vec::new()
                    }
                    Some(part) => match parts.get(part) {
                        None => {
                            ui.label(self.catalog.format(
                                "program-source-part-missing",
                                &BTreeMap::from([("part", part.clone())]),
                            ));
                            Vec::new()
                        }
                        Some(lines) => {
                            ui.label(
                                egui::RichText::new(self.catalog.format(
                                    "program-source-lines",
                                    &BTreeMap::from([
                                        ("part", part.clone()),
                                        ("lines", line_ranges(lines)),
                                    ]),
                                ))
                                .color(palette.dim),
                            );
                            lines.clone()
                        }
                    },
                }
            }
        };
        let current = lines.first().and_then(|first| {
            view.functions
                .iter()
                .find(|function| (function.first..=function.last).contains(&first.first))
        });
        if let Some(line) = function_chips(ui, palette, &view.functions, current) {
            view.jump_to = Some(line);
        }
        let reveal = std::mem::take(&mut self.dock.reveal_program_lines);
        let scroll = !lines.is_empty() && (reveal || view.scrolled_to != part);
        ui.add_space(4.0);
        let height = (bottom - ui.cursor().top() - FOOTER_HEIGHT).max(240.0);
        let jump = view.jump_to.take();
        show_code(
            ui,
            palette,
            &view.source.source,
            &lines,
            CodeScroll {
                to_selection: scroll,
                to_line: jump,
            },
            height,
        );
        let footer = match current {
            Some(function) => format!("{}()", function.name),
            None => view.source.file_name.clone(),
        };
        let parts = view.parts.as_ref().map_or(0, BTreeMap::len);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(footer)
                    .monospace()
                    .small()
                    .color(palette.accent),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(self.catalog.format(
                        "program-source-parts",
                        &BTreeMap::from([("count", parts.to_string())]),
                    ))
                    .monospace()
                    .small()
                    .color(palette.dim),
                );
            });
        });
        view.scrolled_to.clone_from(&part);
        ui.data_mut(|data| data.insert_temp(id, view));
    }

    /// `Starlark  file.star` with a button that copies the program.
    fn program_title(&self, ui: &mut egui::Ui, palette: Palette, program: &RuleProgramSource) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(self.catalog.text("program-source-title"))
                    .strong()
                    .size(SHELL_TITLE_SIZE),
            );
            ui.label(
                egui::RichText::new(&program.file_name)
                    .monospace()
                    .color(palette.dim),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let name = self.catalog.text("program-source-copy");
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                let painter = ui.painter();
                let corner = egui::CornerRadius::same(6);
                painter.rect_filled(
                    rect,
                    corner,
                    if response.hovered() {
                        palette.surface_raised
                    } else {
                        palette.surface
                    },
                );
                painter.rect_stroke(
                    rect,
                    corner,
                    Stroke::new(1.0_f32, palette.line),
                    egui::StrokeKind::Inside,
                );
                theme::paint_icon(
                    painter,
                    shrink_to_icon(rect, 15.0),
                    Icon::Copy,
                    palette.dim,
                    palette.dim,
                    1.5,
                );
                name_widget(&response, true, &name);
                if response.on_hover_text(&name).clicked() {
                    ui.ctx().copy_text(program.source.clone());
                }
            });
        });
        ui.add_space(4.0);
    }
}

/// A one-line status on a tinted band: `glyph text` and, right-aligned,
/// `detail`.
fn status_banner(ui: &mut egui::Ui, color: Color32, glyph: &str, text: &str, detail: Option<&str>) {
    let [red, green, blue, _] = color.to_array();
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(red, green, blue, 28))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(red, green, blue, 90),
        ))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(glyph).color(color));
                ui.add(egui::Label::new(text).wrap());
                if let Some(detail) = detail {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(detail).monospace().small().color(color));
                    });
                }
            });
        });
    ui.add_space(4.0);
}

/// One chip per top-level function, `current` marked; returns the first
/// line of the chip clicked.
fn function_chips(
    ui: &mut egui::Ui,
    palette: Palette,
    functions: &[Function],
    current: Option<&Function>,
) -> Option<usize> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 5.0);
        for function in functions {
            let selected = current == Some(function);
            let mut job = egui::text::LayoutJob::default();
            let mono = egui::FontId::monospace(SHELL_MONO_SIZE);
            job.append(
                &format!("{}()", function.name),
                0.0,
                egui::TextFormat::simple(
                    mono.clone(),
                    if selected {
                        palette.accent
                    } else {
                        palette.text
                    },
                ),
            );
            job.append(
                &if function.first == function.last {
                    function.first.to_string()
                } else {
                    format!("{}–{}", function.first, function.last)
                },
                6.0,
                egui::TextFormat::simple(egui::FontId::monospace(SHELL_SMALL_SIZE), palette.faint),
            );
            let button = egui::Button::new(job)
                .fill(if selected {
                    palette.accent_wash(36)
                } else {
                    palette.surface
                })
                .stroke(Stroke::new(
                    1.0_f32,
                    if selected {
                        palette.accent
                    } else {
                        palette.line
                    },
                ))
                .corner_radius(egui::CornerRadius::same(5));
            if ui.add(button).clicked() {
                clicked = Some(function.first);
            }
        }
    });
    clicked
}

/// Where the code view scrolls this frame.
struct CodeScroll {
    /// The selected part's first line, to the middle.
    to_selection: bool,
    /// A function's first line, to the top.
    to_line: Option<usize>,
}

fn show_code(
    ui: &mut egui::Ui,
    palette: Palette,
    source: &str,
    lines: &[SourceLines],
    scroll: CodeScroll,
    height: f32,
) {
    let colors = palette.code();
    egui::Frame::new()
        .fill(palette.bg)
        .stroke(Stroke::new(1.0_f32, palette.line))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(0, 6))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("program-source-scroll")
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let left = ui.max_rect().left();
                    let mut to_selection = scroll.to_selection;
                    let mut open = None;
                    for (index, line) in source.lines().enumerate() {
                        let number = index + 1;
                        let defining = lines
                            .iter()
                            .any(|lines| (lines.first..=lines.last).contains(&number));
                        let background = ui.painter().add(egui::Shape::Noop);
                        let response =
                            code_line(ui, palette, colors, number, line, defining, &mut open);
                        if defining {
                            let row = Rect::from_x_y_ranges(
                                left..=ui.max_rect().right(),
                                response.rect.y_range(),
                            );
                            ui.painter().set(
                                background,
                                egui::Shape::rect_filled(row, 0.0, palette.accent_wash(34)),
                            );
                            ui.painter().rect_filled(
                                Rect::from_min_size(row.min, Vec2::new(3.0, row.height())),
                                0.0,
                                palette.accent,
                            );
                        }
                        // Only vertically: the line numbers stay in view.
                        let target =
                            Rect::from_x_y_ranges(left..=left + 1.0, response.rect.y_range());
                        if defining && to_selection {
                            ui.scroll_to_rect(target, Some(egui::Align::Center));
                            to_selection = false;
                        }
                        if scroll.to_line == Some(number) {
                            ui.scroll_to_rect(target, Some(egui::Align::Min));
                        }
                    }
                });
        });
}

/// One line of code: marker and number, then the source coloured by token
/// and wrapped to the dock under its own first character, so a narrow dock
/// never cuts a line off. Screen readers get the whole line as one label.
fn code_line(
    ui: &mut egui::Ui,
    palette: Palette,
    colors: CodeColors,
    number: usize,
    line: &str,
    defining: bool,
    open: &mut Option<&'static str>,
) -> egui::Response {
    let font = egui::FontId::monospace(SHELL_MONO_SIZE);
    let format = |color| egui::TextFormat::simple(font.clone(), color);
    let mut gutter = egui::text::LayoutJob::default();
    let marker = if defining { DEFINING_LINE_MARKER } else { " " };
    gutter.append(marker, 6.0, format(palette.accent));
    gutter.append(
        &format!("{number:>4}  "),
        0.0,
        format(if defining {
            palette.text
        } else {
            palette.faint
        }),
    );
    let gutter = ui.painter().layout_job(gutter);
    let mut code = egui::text::LayoutJob::default();
    for (range, token) in starlark_source::highlight_line(line, open) {
        let color = match token {
            Token::Plain => palette.text,
            Token::Keyword => colors.keyword,
            Token::Constant => colors.constant,
            Token::String => colors.string,
            Token::Number => colors.number,
            Token::Comment => colors.comment,
            Token::Call => colors.call,
            Token::Definition => colors.definition,
        };
        code.append(&line[range], 0.0, format(color));
    }
    let width = ui.available_width();
    // Clear of the floating scroll bar; words break only when one alone overflows.
    let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
    code.wrap.max_width = (width - gutter.size().x - bar).max(SHELL_MONO_SIZE * 8.0);
    let code = ui.painter().layout_job(code);
    let size = Vec2::new(width, gutter.size().y.max(code.size().y));
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            true,
            program_source_line_label(number, line, defining),
        )
    });
    let painter = ui.painter();
    let code_left = rect.min + Vec2::new(gutter.size().x, 0.0);
    painter.galley(rect.min, gutter, palette.text);
    painter.galley(code_left, code, palette.text);
    response
}
