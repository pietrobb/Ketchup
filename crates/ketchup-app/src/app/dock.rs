//! The right dock: the selection on top, then one page at a time — layers
//! and structure, the program, checks, views and AI — side by side as tabs,
//! so each page gets the dock's full height instead of a stack of sections.

use crate::program_evaluation::Lookup;
use crate::selection_measures_ui::SelectionMeasures;
use crate::theme::{self, Icon};
use crate::*;

/// One page of the right dock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockTab {
    Layers,
    Program,
    Checks,
    Views,
    Ai,
}

impl DockTab {
    pub const ALL: [Self; 5] = [
        Self::Layers,
        Self::Program,
        Self::Checks,
        Self::Views,
        Self::Ai,
    ];

    /// Localization key of the tab's name.
    #[must_use]
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Layers => "dock-tab-layers",
            Self::Program => "dock-tab-program",
            Self::Checks => "dock-tab-checks",
            Self::Views => "dock-tab-views",
            Self::Ai => "dock-tab-ai",
        }
    }

    const fn icon(self) -> Icon {
        match self {
            Self::Layers => Icon::Layers,
            Self::Program => Icon::Code,
            Self::Checks => Icon::Checklist,
            Self::Views => Icon::Camera,
            Self::Ai => Icon::Plug,
        }
    }
}

const TAB_HEIGHT: f32 = 50.0;

/// The size of one part along its own axes, else of the selection's world box.
fn shown_size(measures: &SelectionMeasures) -> [f64; 3] {
    measures.own_size_mm.unwrap_or(measures.world_size_mm)
}

fn millimetres(value: f64) -> String {
    let text = format!("{value:.1}");
    text.strip_suffix(".0").map_or(text.clone(), str::to_owned)
}

impl KetchupApp {
    /// The page the dock shows: the one the user picked; otherwise the
    /// program, or how the model is built when the manual CAD tools are on
    /// and the model has no layers yet, and the layers for everything else.
    #[must_use]
    pub fn dock_tab(&self) -> DockTab {
        self.dock.tab.unwrap_or_else(|| {
            let program = self.document.current_rule_program().is_some();
            let layered = self.document.current().tags().next().is_some();
            if program || (self.manual_cad_tools_visible() && !layered) {
                DockTab::Program
            } else {
                DockTab::Layers
            }
        })
    }

    pub fn set_dock_tab(&mut self, tab: DockTab) {
        self.dock.tab = Some(tab);
    }

    pub(crate) fn show_right_dock(&mut self, ui: &mut egui::Ui) {
        self.show_selection_card(ui);
        self.show_dock_tabs(ui);
        ui.add_space(8.0);
        let tab = self.dock_tab();
        if tab == DockTab::Program && self.document.current_rule_program().is_some() {
            // The code sizes itself to the dock, so the page scrolls on its own.
            self.show_program_tab(ui);
            return;
        }
        dock_scroll_area().show(ui, |ui| {
            self.show_tool_options(ui);
            match tab {
                DockTab::Layers => {
                    self.show_layers_section(ui);
                    self.show_occurrence_color_editor(ui);
                    self.show_outliner(ui);
                }
                DockTab::Program => {
                    if !self.manual_cad_tools_visible() {
                        ui.weak(self.catalog.text("dock-program-none"));
                    }
                    self.show_parameter_editor(ui);
                    self.show_manual_cad_tools(ui);
                }
                DockTab::Checks => self.show_validator_content(ui),
                DockTab::Views => {
                    self.saved_views_ui(ui);
                    self.section_ui(ui);
                    ui.separator();
                    self.show_viewer_notes(ui);
                    self.show_viewer_dimensions(ui);
                    self.show_received_comments(ui);
                }
                DockTab::Ai => self.show_connect_ai(ui),
            }
        });
    }

    /// Options of the tool in hand; whichever page is open.
    pub(crate) fn show_tool_options(&mut self, ui: &mut egui::Ui) {
        self.show_face_workflow_ui(ui);
        self.show_helix_tool(ui);
    }

    /// The selected part: its name, layers, where the program builds it and
    /// its size, then its position, volume and mass. Nothing when nothing
    /// measurable is selected, so the tabs stay at the top.
    fn show_selection_card(&mut self, ui: &mut egui::Ui) {
        let Some(measures) = self.selection_measures() else {
            return;
        };
        let palette = self.palette();
        let snapshot = self.document.current();
        let paths = self.selected_instance_paths();
        let first = paths.first();
        let part = first.and_then(|path| live_bridge::program_pick::part_name(&snapshot, path));
        let occurrence = first.and_then(|path| snapshot.occurrence(path.root_occurrence()));
        let name = if measures.parts > 1 {
            self.catalog.format(
                "selection-measures-parts",
                &BTreeMap::from([("count", measures.parts.to_string())]),
            )
        } else {
            part.clone()
                .or_else(|| occurrence.map(|occurrence| occurrence.name().to_owned()))
                .unwrap_or_default()
        };
        let tags = occurrence
            .map(|occurrence| {
                occurrence
                    .tags()
                    .iter()
                    .filter_map(|id| snapshot.tag(*id))
                    .map(|tag| tag.name().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let source = part.as_ref().and_then(|part| self.program_part_lines(part));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(self.catalog.text("selection-measures-title").to_uppercase())
                    .size(SHELL_SECTION_SIZE)
                    .color(palette.dim),
            );
            ui.label(egui::RichText::new(&name).strong().size(SHELL_TITLE_SIZE));
            for tag in &tags {
                tag_chip(ui, palette, tag);
            }
            if let Some(source) = source {
                let link = ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{source} →"))
                            .monospace()
                            .color(palette.accent),
                    )
                    .sense(Sense::click()),
                );
                if link
                    .on_hover_text(self.catalog.text("selection-source-hint"))
                    .clicked()
                {
                    self.dock.tab = Some(DockTab::Program);
                    self.dock.reveal_program_lines = true;
                }
            }
        });
        ui.add_space(4.0);
        let size = shown_size(&measures);
        ui.columns(3, |columns| {
            for (column, (key, value)) in columns.iter_mut().zip(
                ["selection-size-w", "selection-size-d", "selection-size-h"]
                    .into_iter()
                    .zip(size),
            ) {
                size_tile(
                    column,
                    palette,
                    &self.catalog.text(key),
                    &millimetres(value),
                );
            }
        });
        for line in self.selection_detail_lines(&measures) {
            ui.add(
                egui::Label::new(egui::RichText::new(line).small().color(palette.dim)).truncate(),
            );
        }
        ui.add_space(6.0);
        ui.separator();
    }

    /// `file.star:11–14` for the first lines that build `part`, once the
    /// program is evaluated.
    fn program_part_lines(&self, part: &str) -> Option<String> {
        let program = self.document.current_rule_program()?;
        let Lookup::Ready(Ok(evaluated)) = self.program_evaluations.try_get(program) else {
            return None;
        };
        let lines = evaluated.part_sources.get(part)?.first()?;
        Some(if lines.first == lines.last {
            format!("{}:{}", program.file_name, lines.first)
        } else {
            format!("{}:{}–{}", program.file_name, lines.first, lines.last)
        })
    }

    fn show_dock_tabs(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        let current = self.dock_tab();
        let issues = self
            .validator_report()
            .map_or(0, |report| report.issue_count);
        let connected = self.live.consent_attached || self.live.launch_attached;
        let width = ui.available_width() / DockTab::ALL.len() as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for tab in DockTab::ALL {
                let label = self.catalog.text(tab.label_key());
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, TAB_HEIGHT), Sense::click());
                let selected = tab == current;
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        selected,
                        &label,
                    )
                });
                let painter = ui.painter();
                if selected {
                    painter.rect_filled(rect, 0.0, palette.surface);
                    painter.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(rect.left(), rect.bottom() - 2.0),
                            rect.right_bottom(),
                        ),
                        0.0,
                        palette.accent,
                    );
                } else if response.hovered() {
                    painter.rect_filled(rect, 0.0, palette.surface_raised);
                }
                let ink = if selected || response.hovered() {
                    palette.text
                } else {
                    palette.dim
                };
                let icon = Rect::from_center_size(
                    Pos2::new(rect.center().x, rect.top() + 17.0),
                    Vec2::splat(18.0),
                );
                theme::paint_icon(
                    painter,
                    icon,
                    tab.icon(),
                    if selected { palette.accent } else { ink },
                    palette.accent,
                    1.6,
                );
                painter.text(
                    Pos2::new(rect.center().x, rect.top() + 37.0),
                    egui::Align2::CENTER_CENTER,
                    &label,
                    egui::FontId::proportional(SHELL_SMALL_SIZE + 0.5),
                    ink,
                );
                if tab == DockTab::Checks && issues > 0 {
                    let centre = icon.right_top() + Vec2::new(3.0, 1.0);
                    painter.circle_filled(centre, 7.5, palette.accent);
                    painter.text(
                        centre,
                        egui::Align2::CENTER_CENTER,
                        issues.min(99).to_string(),
                        egui::FontId::proportional(9.5),
                        palette.accent_ink,
                    );
                }
                if tab == DockTab::Ai {
                    let dot = if connected {
                        palette.code().ok
                    } else {
                        palette.faint
                    };
                    painter.circle_filled(icon.right_top() + Vec2::new(3.0, 2.0), 3.5, dot);
                }
                if response.clicked() {
                    self.dock.tab = Some(tab);
                }
            }
        });
        let line = ui.cursor().top();
        ui.painter().hline(
            ui.max_rect().x_range(),
            line,
            Stroke::new(1.0_f32, palette.line),
        );
    }
}

fn tag_chip(ui: &mut egui::Ui, palette: theme::Palette, name: &str) {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0_f32, palette.line))
        .corner_radius(egui::CornerRadius::same(5))
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter()
                    .rect_filled(dot, egui::CornerRadius::same(2), palette.accent);
                ui.label(egui::RichText::new(name).small());
            });
        });
}

fn size_tile(ui: &mut egui::Ui, palette: theme::Palette, axis: &str, value: &str) {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(Stroke::new(1.0_f32, palette.line))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(9, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(axis).monospace().color(palette.faint));
                ui.label(
                    egui::RichText::new(value)
                        .monospace()
                        .strong()
                        .size(SHELL_TITLE_SIZE),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new("mm").small().color(palette.faint));
                });
            });
        });
}
