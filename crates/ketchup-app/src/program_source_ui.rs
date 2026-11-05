//! The dock's Program page: the whole Starlark program of a program-owned
//! model, coloured as code, with its functions as chips to jump to and the
//! lines that build the selected part highlighted; then the program's
//! parameters and variants.

mod starlark_source;
#[cfg(test)]
mod tests;

use super::*;
use crate::program_evaluation::Lookup;
use crate::theme::CodeColors;
use ketchup_application::SourceLines;
use ketchup_model::document::RuleProgramSource;
use starlark_source::{Function, Token};

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
    code: CodeView,
}

/// The program's lines coloured once per source, and where they sit at the
/// dock's width, so that a frame lays out only the lines in view.
#[derive(Clone)]
struct CodeView {
    lines: Vec<CodeLine>,
    /// Each line's width unwrapped, with the screen's points per pixel it
    /// was measured at.
    widths: Option<(u32, Vec<f32>)>,
    rows: Option<CodeRows>,
}

#[derive(Clone)]
struct CodeLine {
    /// The line in the source, without its line break.
    text: std::ops::Range<usize>,
    spans: Vec<(std::ops::Range<usize>, Token)>,
}

/// Where the lines sit at one width.
#[derive(Clone)]
struct CodeRows {
    /// The code's width and the screen's points per pixel, bit for bit.
    key: (u32, u32),
    /// Where the code starts right of the marker and number.
    gutter: f32,
    /// The width code wraps at.
    wrap: f32,
    /// The height of one row of code.
    row_height: f32,
    /// Each line's top below the first, then the bottom of the last.
    tops: Vec<f32>,
}

impl CodeView {
    fn of(source: &str) -> Self {
        let mut open = None;
        let mut start = 0;
        let lines = source
            .split_inclusive('\n')
            .map(|piece| {
                let line = piece
                    .strip_suffix('\n')
                    .map_or(piece, |line| line.strip_suffix('\r').unwrap_or(line));
                let text = start..start + line.len();
                start += piece.len();
                CodeLine {
                    spans: starlark_source::highlight_line(line, &mut open),
                    text,
                }
            })
            .collect();
        Self {
            lines,
            widths: None,
            rows: None,
        }
    }
}

/// The font code is set in.
fn code_font() -> egui::FontId {
    egui::FontId::monospace(SHELL_MONO_SIZE)
}

/// Where the lines of `code` sit at `width`. No line is laid out as text for
/// it: a line as wide as the code fits is one row, and where a longer one
/// breaks is counted from its letters' widths, so dragging the dock's edge,
/// which changes the width every frame, lays out only the lines in view
/// (review 2026-11-04 r19).
fn code_rows<'a>(
    code: &'a mut CodeView,
    ui: &egui::Ui,
    source: &str,
    width: f32,
) -> (&'a CodeRows, &'a [CodeLine]) {
    let points = ui.ctx().pixels_per_point().to_bits();
    let key = (width.to_bits(), points);
    if code.rows.as_ref().is_some_and(|rows| rows.key != key) {
        code.rows = None;
    }
    let CodeView {
        lines,
        widths,
        rows,
    } = code;
    let rows = rows.get_or_insert_with(|| {
        let gutter = [false, true]
            .into_iter()
            .map(|defining| {
                let job = gutter_job(lines.len(), defining, Color32::WHITE, Color32::WHITE);
                ui.painter().layout_job(job).size()
            })
            .fold(Vec2::ZERO, Vec2::max);
        let wrap = code_wrap_width(ui, width, gutter.x);
        ui.fonts(|fonts| {
            let mut fonts = fonts.lock();
            let font = fonts.fonts.font(&code_font());
            let row_height = font.row_height().max(gutter.y);
            if widths.as_ref().is_none_or(|(at, _)| *at != points) {
                let measured = lines
                    .iter()
                    .map(|line| {
                        source[line.text.clone()]
                            .chars()
                            .map(|c| font.glyph_width(c))
                            .sum()
                    })
                    .collect();
                *widths = Some((points, measured));
            }
            let measured = widths.as_ref().map_or(&[][..], |(_, widths)| widths);
            let mut top = 0.0;
            let mut tops = Vec::with_capacity(lines.len() + 1);
            tops.push(top);
            for (line, line_width) in lines.iter().zip(measured) {
                let rows = if *line_width <= wrap {
                    1
                } else {
                    row_starts(&source[line.text.clone()], wrap, |c| font.glyph_width(c)).len() + 1
                };
                top += std::iter::repeat_n(row_height, rows).sum::<f32>();
                tops.push(top);
            }
            CodeRows {
                key,
                gutter: gutter.x,
                wrap,
                row_height,
                tops,
            }
        })
    });
    (rows, lines)
}

/// Where `text` breaks into rows no wider than `wrap`: the byte offset each
/// row after the first starts at. A row breaks after its last space, and
/// inside a word only when the word alone is wider than a row; a space at
/// the end of a row may overhang it.
fn row_starts(text: &str, wrap: f32, mut advance: impl FnMut(char) -> f32) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut row = 0;
    let mut x = 0.0;
    // Just after the row's last space, and the row's width up to there.
    let mut after_space: Option<(usize, f32)> = None;
    for (at, c) in text.char_indices() {
        let width = advance(c);
        if c.is_whitespace() {
            x += width;
            after_space = Some((at + c.len_utf8(), x));
            continue;
        }
        if x + width > wrap && at > row {
            if let Some((start, before)) = after_space.take().filter(|(start, _)| *start > row) {
                starts.push(start);
                row = start;
                x -= before;
            }
            if x + width > wrap && at > row {
                starts.push(at);
                row = at;
                x = 0.0;
            }
        }
        x += width;
    }
    starts
}

/// A line's marker and number.
fn gutter_job(
    number: usize,
    defining: bool,
    marker_color: Color32,
    number_color: Color32,
) -> egui::text::LayoutJob {
    let font = egui::FontId::monospace(SHELL_MONO_SIZE);
    let mut gutter = egui::text::LayoutJob::default();
    let marker = if defining { DEFINING_LINE_MARKER } else { " " };
    gutter.append(
        marker,
        6.0,
        egui::TextFormat::simple(font.clone(), marker_color),
    );
    gutter.append(
        &format!("{number:>4}  "),
        0.0,
        egui::TextFormat::simple(font, number_color),
    );
    gutter
}

/// The width code wraps at: clear of the floating scroll bar; words break
/// only when one alone overflows.
fn code_wrap_width(ui: &egui::Ui, width: f32, gutter: f32) -> f32 {
    let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
    (width - gutter - bar).max(SHELL_MONO_SIZE * 8.0)
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
        // Taken out and put back, not copied: the source may be a megabyte.
        let kept = ui
            .data_mut(|data| data.remove_temp::<Option<ProgramSourceView>>(id))
            .flatten()
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
                    code: CodeView::of(&program.source),
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
            &mut view.code,
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
        ui.data_mut(|data| data.insert_temp(id, Some(view)));
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
    code: &mut CodeView,
    lines: &[SourceLines],
    scroll: CodeScroll,
    height: f32,
) {
    let colors = palette.code();
    let defines = |number: usize| {
        lines
            .iter()
            .any(|lines| (lines.first..=lines.last).contains(&number))
    };
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
                .show_viewport(ui, |ui, viewport| {
                    let width = ui.available_width();
                    let content = ui.max_rect();
                    let (rows, code_lines) = code_rows(code, ui, source, width);
                    let count = rows.tops.len() - 1;
                    ui.set_height(rows.tops[count]);
                    let row = |index: usize| {
                        Rect::from_min_max(
                            egui::pos2(content.left(), content.top() + rows.tops[index]),
                            egui::pos2(
                                content.left() + width,
                                content.top() + rows.tops[index + 1],
                            ),
                        )
                    };
                    // Only vertically: the line numbers stay in view.
                    let target = |index: usize| {
                        let row = row(index);
                        Rect::from_x_y_ranges(row.left()..=row.left() + 1.0, row.y_range())
                    };
                    if scroll.to_selection
                        && let Some(first) = (1..=count).find(|number| defines(*number))
                    {
                        ui.scroll_to_rect(target(first - 1), Some(egui::Align::Center));
                    }
                    if let Some(number) =
                        scroll.to_line.filter(|number| (1..=count).contains(number))
                    {
                        ui.scroll_to_rect(target(number - 1), Some(egui::Align::Min));
                    }
                    let first = rows
                        .tops
                        .partition_point(|top| *top <= viewport.min.y)
                        .saturating_sub(1);
                    let end = rows
                        .tops
                        .partition_point(|top| *top < viewport.max.y)
                        .min(count);
                    let line_id = |number: usize| ui.id().with(("program-line", number));
                    let breaks: Vec<Vec<usize>> = ui.fonts(|fonts| {
                        let mut fonts = fonts.lock();
                        let font = fonts.fonts.font(&code_font());
                        code_lines[first..end]
                            .iter()
                            .map(|line| {
                                row_starts(&source[line.text.clone()], rows.wrap, |c| {
                                    font.glyph_width(c)
                                })
                            })
                            .collect()
                    });
                    for (index, starts) in (first..end).zip(breaks) {
                        let number = index + 1;
                        let line = &code_lines[index];
                        let text = &source[line.text.clone()];
                        let row = row(index);
                        let defining = defines(number);
                        if defining {
                            ui.painter().rect_filled(row, 0.0, palette.accent_wash(34));
                            ui.painter().rect_filled(
                                Rect::from_min_size(row.min, Vec2::new(3.0, row.height())),
                                0.0,
                                palette.accent,
                            );
                        }
                        let response = ui.interact(row, line_id(number), Sense::hover());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Label,
                                true,
                                program_source_line_label(number, text, defining),
                            )
                        });
                        paint_code_line(
                            ui,
                            palette,
                            colors,
                            CodeRow {
                                number,
                                text,
                                spans: &line.spans,
                                starts: &starts,
                                defining,
                            },
                            row.min,
                            rows,
                        );
                    }
                    // A screen reader reads the whole program, not only the
                    // lines in view; the lines out of view are named for it
                    // alone, and only while one is listening (review
                    // 2026-11-04 r19).
                    for index in (0..first).chain(end..count) {
                        let number = index + 1;
                        let row = row(index);
                        let named = ui.ctx().accesskit_node_builder(line_id(number), |node| {
                            let text = &source[code_lines[index].text.clone()];
                            node.set_role(egui::accesskit::Role::Label);
                            node.set_value(program_source_line_label(
                                number,
                                text,
                                defines(number),
                            ));
                            node.set_bounds(egui::accesskit::Rect {
                                x0: row.min.x.into(),
                                y0: row.min.y.into(),
                                x1: row.max.x.into(),
                                y1: row.max.y.into(),
                            });
                        });
                        if named.is_none() {
                            break;
                        }
                    }
                });
        });
}

/// One line as it is painted.
struct CodeRow<'a> {
    number: usize,
    text: &'a str,
    spans: &'a [(std::ops::Range<usize>, Token)],
    /// Where in `text` each row after the first starts.
    starts: &'a [usize],
    defining: bool,
}

/// One line of code at `top_left`: marker and number, then the source
/// coloured by token in the rows `code_rows` counted for it, each under the
/// line's first character, so a narrow dock never cuts a line off.
fn paint_code_line(
    ui: &egui::Ui,
    palette: Palette,
    colors: CodeColors,
    line: CodeRow<'_>,
    top_left: egui::Pos2,
    rows: &CodeRows,
) {
    let format = |color| egui::TextFormat::simple(code_font(), color);
    let number_color = if line.defining {
        palette.text
    } else {
        palette.faint
    };
    let gutter = gutter_job(line.number, line.defining, palette.accent, number_color);
    let painter = ui.painter();
    painter.galley(top_left, painter.layout_job(gutter), palette.text);
    let ends = line.starts.iter().copied().chain([line.text.len()]);
    let mut start = 0;
    let mut below = 0.0;
    for end in ends {
        let mut code = egui::text::LayoutJob::default();
        for (range, token) in line.spans {
            let piece = range.start.max(start)..range.end.min(end);
            if piece.is_empty() {
                continue;
            }
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
            code.append(&line.text[piece], 0.0, format(color));
        }
        painter.galley(
            top_left + Vec2::new(rows.gutter, below),
            painter.layout_job(code),
            palette.text,
        );
        start = end;
        below += rows.row_height;
    }
}
