//! The right dock folds to a narrow strip of its tab icons, so a small screen
//! gives its width to the model. The chevron at the end of the dock's tabs folds
//! it; the chevron on the strip, or a tab icon there, opens it again, as does
//! its checkbox in the Window menu.

use crate::theme;
use crate::*;

/// The width of the chevron at the end of the dock's tabs.
pub(crate) const DOCK_FOLD_WIDTH: f32 = 24.0;
const STRIP_WIDTH: f32 = 40.0;

impl KetchupApp {
    /// The right dock, or the strip it is folded to.
    pub(crate) fn show_right_side(&mut self, context: &egui::Context, frame: egui::Frame) {
        if self.window.panes.dock_collapsed {
            egui::SidePanel::right("right-dock-strip")
                .resizable(false)
                .exact_width(STRIP_WIDTH)
                .frame(frame.inner_margin(egui::Margin::symmetric(0, 8)))
                .show(context, |ui| self.show_dock_strip(ui));
            return;
        }
        egui::SidePanel::right("right-dock")
            .resizable(true)
            .default_width(440.0)
            .width_range(380.0..=720.0)
            .frame(frame)
            .show(context, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                self.show_right_dock(ui);
            });
    }

    fn show_dock_strip(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        let current = self.dock_tab();
        ui.spacing_mut().item_spacing.y = 2.0;
        self.dock_fold_button(ui, Vec2::new(STRIP_WIDTH, 30.0));
        ui.add_space(6.0);
        for tab in DockTab::ALL {
            let label = self.catalog.text(tab.label_key());
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(STRIP_WIDTH), Sense::click());
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            let hovered = response.hovered();
            if hovered {
                ui.painter()
                    .rect_filled(rect.shrink(3.0), 4.0, palette.surface_raised);
            }
            let ink = if tab == current {
                palette.accent
            } else if hovered {
                palette.text
            } else {
                palette.dim
            };
            theme::paint_icon(
                ui.painter(),
                Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
                tab.icon(),
                ink,
                palette.accent,
                1.6,
            );
            if response.on_hover_text(&label).clicked() {
                self.window.dock_tab = Some(tab);
                self.window.panes.dock_collapsed = false;
            }
        }
    }

    /// The chevron that folds the dock, or opens it from the strip.
    pub(crate) fn dock_fold_button(&mut self, ui: &mut egui::Ui, size: Vec2) {
        let folded = self.window.panes.dock_collapsed;
        let label = self
            .catalog
            .text(if folded { "dock-unfold" } else { "dock-fold" });
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        let palette = self.palette();
        let ink = if response.hovered() {
            ui.painter()
                .rect_filled(rect.shrink(3.0), 4.0, palette.surface_raised);
            palette.text
        } else {
            palette.dim
        };
        // `›` folds the dock toward the window's edge, `‹` brings it back.
        let centre = rect.center();
        let tip = if folded { -3.0 } else { 3.0 };
        ui.painter().add(egui::Shape::line(
            vec![
                centre + Vec2::new(-tip, -5.0),
                centre + Vec2::new(tip, 0.0),
                centre + Vec2::new(-tip, 5.0),
            ],
            Stroke::new(1.6_f32, ink),
        ));
        if response.on_hover_text(&label).clicked() {
            self.window.panes.dock_collapsed = !folded;
        }
    }
}
