//! The dock's VARIANTS section: the program's saved variants with the one its
//! parameters have now, Use and Delete per variant, Save for the current values
//! and Export of all variants. The program evaluation is asked without
//! waiting: while it or an action is being computed the section says so and
//! its buttons wait, never the frame.

use super::*;
use crate::live_bridge::variant_window::VariantAction;
use crate::program_evaluation::Lookup;
use ketchup_application::program_variants::active_variant;

fn values_text(values: &BTreeMap<String, f64>) -> String {
    values
        .iter()
        .map(|(parameter, value)| format!("{parameter} {value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl KetchupApp {
    pub(crate) fn show_variants_section(&mut self, ui: &mut egui::Ui) {
        let Some(program) = self.document.current_rule_program().cloned() else {
            return;
        };
        let evaluation = self.program_evaluations.try_get(&program);
        let planning = matches!(evaluation, Lookup::Pending);
        let params = match evaluation {
            Lookup::Ready(Ok(evaluated)) => evaluated.model.params.clone(),
            _ => Vec::new(),
        };
        if params.is_empty() && program.variants.is_empty() {
            return;
        }
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("dock-variants"));
        let idle = !planning && !self.live.variants.is_running();
        if !idle {
            ui.label(self.catalog.text("variants-pending"));
        }
        let active = active_variant(&program, &params);
        let mut action = None;
        for (name, values) in &program.variants {
            let row = BTreeMap::from([("name", name.clone()), ("values", values_text(values))]);
            let named = BTreeMap::from([("name", name.clone())]);
            ui.horizontal_wrapped(|ui| {
                let _ = ui.selectable_label(
                    active == Some(name.as_str()),
                    self.catalog.format("variants-row", &row),
                );
                let use_button = egui::Button::new(self.catalog.format("variants-use", &named));
                if ui
                    .add_enabled(idle && active != Some(name.as_str()), use_button)
                    .clicked()
                {
                    action = Some(VariantAction::Use(name.clone()));
                }
                let delete = egui::Button::new(self.catalog.format("variants-delete", &named));
                if ui.add_enabled(idle, delete).clicked() {
                    action = Some(VariantAction::Delete(name.clone()));
                }
            });
        }
        let label = self.catalog.text("variants-name");
        ui.horizontal(|ui| {
            ui.label(&label);
            ui.add(egui::TextEdit::singleline(&mut self.live.variants.name).desired_width(90.0))
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, &label)
                });
        });
        let name = self.live.variants.name.trim().to_owned();
        let save = egui::Button::new(self.catalog.text("variants-save"));
        if ui
            .add_enabled(idle && !params.is_empty() && !name.is_empty(), save)
            .clicked()
        {
            action = Some(VariantAction::Save(name));
        }
        let export = egui::Button::new(self.catalog.text("variants-export"));
        if ui
            .add_enabled(idle && !program.variants.is_empty(), export)
            .clicked()
            && let Some(path) = self.choose_export_path("xlsx")
        {
            action = Some(VariantAction::Export(path));
        }
        if let Some(outcome) = &self.live.variants.outcome {
            ui.add(egui::Label::new(egui::RichText::new(outcome).weak()).wrap());
        }
        if let Some(action) = action {
            if matches!(action, VariantAction::Save(_)) {
                self.live.variants.name.clear();
            }
            self.start_variant_action(ui.ctx(), action);
        }
    }
}
