//! The Settings menu: the interface language. The choice applies at once and
//! the host keeps it for the next start ([`ViewerApp::take_changed_preferences`]).

use super::ViewerApp;
use crate::{preferences::Preferences, text::Language};
use eframe::egui;

impl ViewerApp {
    /// Start with the settings the host kept; `KETCHUP_LANGUAGE` still wins.
    #[must_use]
    pub fn with_preferences(mut self, preferences: Preferences) -> Self {
        self.language = Language::detect(preferences.language);
        self.preferences = preferences;
        self
    }

    /// The settings, once since the customer last changed them, for the host to keep.
    pub fn take_changed_preferences(&mut self) -> Option<Preferences> {
        std::mem::take(&mut self.preferences_changed).then_some(self.preferences)
    }

    pub(super) fn settings_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.t("viewer-settings"), |ui| {
            ui.label(self.t("viewer-language"));
            for language in Language::all() {
                if ui
                    .radio(self.language == language, language.name())
                    .clicked()
                {
                    self.language = language;
                    self.preferences.language = Some(language);
                    self.preferences_changed = true;
                    ui.close();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable as _;

    fn harness(app: ViewerApp) -> egui_kittest::Harness<'static, ViewerApp> {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_state(|ctx, app: &mut ViewerApp| app.show(ctx), app);
        harness.run();
        harness
    }

    #[test]
    fn the_settings_menu_switches_every_text_and_the_decimal_mark_at_once() {
        let slovak = Language::from_tag("sk").expect("Slovak catalog");
        let mut ui = harness(ViewerApp::new(None).with_language(slovak));
        ui.get_by_label("Perspektíva");
        assert_eq!(super::super::millimetres(ui.state().language, 18.5), "18,5");

        ui.get_by_label("Nastavenia").click();
        ui.run();
        ui.get_by_label("English").click();
        ui.run();
        ui.get_by_label("Perspective");
        ui.get_by_label("Open model");
        assert!(ui.query_by_label("Perspektíva").is_none());
        assert_eq!(super::super::millimetres(ui.state().language, 18.5), "18.5");
        let kept = ui.state_mut().take_changed_preferences();
        assert_eq!(kept.and_then(|kept| kept.language), Some(Language::ENGLISH));
        assert_eq!(ui.state_mut().take_changed_preferences(), None);

        ui.get_by_label("Settings").click();
        ui.run();
        ui.get_by_label("Slovenčina").click();
        ui.run();
        ui.get_by_label("Perspektíva");
        let kept = ui.state_mut().take_changed_preferences();
        assert_eq!(kept.and_then(|kept| kept.language), Some(slovak));
    }

    /// Assumes `KETCHUP_LANGUAGE` is unset, as on CI: it would override the choice.
    #[test]
    fn a_kept_language_wins_over_the_system_language_at_the_next_start() {
        for language in Language::all() {
            let kept = Preferences {
                language: Some(language),
            };
            let mut ui = harness(ViewerApp::new(None).with_preferences(kept));
            ui.get_by_label(language.text("viewer-settings"));
            assert_eq!(ui.state_mut().take_changed_preferences(), None);
        }
    }
}
