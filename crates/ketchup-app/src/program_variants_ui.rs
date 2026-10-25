//! The dock's program sections. PROGRAM PARAMETERS: each `param()` with its
//! value, range and description; a value typed and confirmed with Enter is the
//! `set_params` apply. VARIANTS: the program's saved variants with the one its
//! parameters have now, Use and Delete per variant, Save for the current values
//! and Export of all variants. The program evaluation is asked without
//! waiting: while it or an action is being computed the sections say so and
//! their fields and buttons wait, never the frame.

use super::*;
use crate::length_input::{DecimalSeparator, read_length, read_number};
use crate::live_bridge::variant_window::VariantAction;
use crate::program_evaluation::Lookup;
use ketchup_application::program_variants::active_variant;
use ketchup_program::model::Param;

fn values_text(values: &BTreeMap<String, f64>) -> String {
    values
        .iter()
        .map(|(parameter, value)| format!("{parameter} {value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A number as the window's fields read it back, with at most six decimals, so a third
/// shows as `0.333333`, not with the noise of its binary fraction (review 2026-10-24 UI-29).
pub(crate) fn number_text(value: f64, decimal: DecimalSeparator) -> String {
    let rounded = format!("{value:.6}");
    let text = match rounded.trim_end_matches('0').trim_end_matches('.') {
        "-0" => "0".to_owned(),
        text => text.to_owned(),
    };
    match decimal {
        DecimalSeparator::Comma => text.replace('.', ","),
        DecimalSeparator::Point => text,
    }
}

impl KetchupApp {
    pub(crate) fn show_variants_section(&mut self, ui: &mut egui::Ui) {
        let Some(program) = self.document.current_rule_program().cloned() else {
            return;
        };
        let evaluation = self.program_evaluations.try_get(&program);
        let planning = matches!(evaluation, Lookup::Pending);
        match evaluation {
            Lookup::Ready(Ok(evaluated)) => {
                self.live.variants.params = evaluated.model.params.clone()
            }
            Lookup::Pending => {}
            _ => self.live.variants.params.clear(),
        }
        let params = self.live.variants.params.clone();
        if params.is_empty() && program.variants.is_empty() {
            return;
        }
        let idle = !planning && !self.live.variants.is_running();
        let mut action = self.show_params(ui, &params, idle);
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("dock-variants"));
        if !idle && params.is_empty() {
            ui.label(self.catalog.text("variants-pending"));
        }
        let active = active_variant(&program, &params);
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

    /// One field per parameter; Enter in a field, or leaving it changed, asks
    /// for its value, in millimetres unless the parameter takes no unit. Text
    /// that is no value changes nothing and says why.
    fn show_params(
        &mut self,
        ui: &mut egui::Ui,
        params: &[Param],
        idle: bool,
    ) -> Option<VariantAction> {
        if params.is_empty() {
            return None;
        }
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("dock-params"));
        if !idle {
            ui.label(self.catalog.text("variants-pending"));
        }
        let decimal = self.value_box.decimal;
        let mut action = None;
        for param in params {
            let named = BTreeMap::from([("name", param.name.clone())]);
            let label = self.catalog.format("params-field", &named);
            let range = self.range_text(param, decimal);
            let shown = number_text(param.value, decimal);
            let drafted = match &self.live.variants.draft {
                Some((name, draft)) if *name == param.name => Some(draft.clone()),
                _ => None,
            };
            // The field being typed in stays while the dock waits, so its text is not
            // lost (review 2026-10-24 UI-29); the others wait.
            let enabled = idle || drafted.is_some();
            let mut text = drafted.clone().unwrap_or_else(|| shown.clone());
            let field = ui
                .horizontal(|ui| {
                    ui.label(&param.name);
                    let field = egui::TextEdit::singleline(&mut text).desired_width(90.0);
                    let response = ui.add_enabled(enabled, field);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, &label)
                    });
                    if let Some(range) = range {
                        ui.label(range);
                    }
                    response
                })
                .inner;
            if !param.doc.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(&param.doc).weak()).wrap());
            }
            // Text confirmed while the dock waited, set once it is free.
            let queued = drafted.is_some() && !field.has_focus() && !field.lost_focus();
            if field.has_focus() {
                self.live.variants.draft = Some((param.name.clone(), text));
            } else if field.lost_focus() || (queued && idle) {
                // Enter, or leaving a changed field, sets the value; Escape
                // leaves it (review 2026-10-24 WB-27).
                let escape = !queued && ui.input(|input| input.key_pressed(egui::Key::Escape));
                if !escape && !idle {
                    self.live.variants.draft = Some((param.name.clone(), text));
                    continue;
                }
                self.live.variants.draft = None;
                if !escape && text != shown {
                    let read = if param.plain {
                        read_number(&text, decimal)
                    } else {
                        read_length(&text, decimal)
                    };
                    match read {
                        Ok(value) => {
                            action = Some(VariantAction::SetParam(param.name.clone(), value))
                        }
                        Err(refusal) => {
                            let arguments = BTreeMap::from([
                                ("name", param.name.clone()),
                                ("reason", self.catalog.text(refusal.message_key())),
                            ]);
                            self.live.variants.outcome =
                                Some(self.catalog.format("params-refused", &arguments));
                        }
                    }
                }
            }
        }
        action
    }

    /// The declared bounds of a parameter, if it has any.
    fn range_text(&self, param: &Param, decimal: DecimalSeparator) -> Option<String> {
        let number = |value: f64| number_text(value, decimal);
        let (key, arguments) = match (param.min, param.max) {
            (Some(min), Some(max)) => (
                "params-range",
                vec![("min", number(min)), ("max", number(max))],
            ),
            (Some(min), None) => ("params-at-least", vec![("min", number(min))]),
            (None, Some(max)) => ("params-at-most", vec![("max", number(max))]),
            (None, None) => return None,
        };
        Some(self.catalog.format(key, &arguments.into_iter().collect()))
    }
}
