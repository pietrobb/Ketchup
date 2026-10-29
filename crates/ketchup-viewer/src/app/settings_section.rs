//! The Settings menu: the interface language and the projection packages
//! open in. A choice applies at once and the host keeps it for the next
//! start ([`ViewerApp::take_changed_preferences`]).

use super::ViewerApp;
use crate::{preferences::Preferences, text::Language};
use eframe::egui;
use ketchup_view_format::FittedProjection;

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
            ui.separator();
            ui.label(self.t("viewer-projection"));
            for (projection, key) in [
                (None, "viewer-projection-package"),
                (
                    Some(FittedProjection::Perspective),
                    "viewer-projection-perspective",
                ),
                (
                    Some(FittedProjection::Orthographic),
                    "viewer-projection-orthographic",
                ),
            ] {
                if ui
                    .radio(self.preferences.projection == projection, self.t(key))
                    .clicked()
                {
                    self.choose_projection(projection);
                    ui.close();
                }
            }
        });
    }

    /// From now on and for every package opened later; following the
    /// package leaves the view on screen as it is until the next Fit or scene.
    fn choose_projection(&mut self, projection: Option<FittedProjection>) {
        self.preferences.projection = projection;
        self.preferences_changed = true;
        self.perspective = projection.map(super::is_perspective);
        if let (Some(perspective), Some(camera)) = (self.perspective, &mut self.camera) {
            camera.set_perspective(perspective);
        }
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
        assert!(ui.query_by_label("Perspektíva").is_none());
        assert!(ui.query_by_label("Nastavenia").is_none());
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

    fn package_bytes(fitted: Option<FittedProjection>) -> Vec<u8> {
        let mut package = crate::model::tests::package(0.1);
        package.manifest.fitted_projection = fitted;
        let mut bytes = Vec::new();
        package.write(&mut bytes).unwrap();
        bytes
    }

    fn opened_in_perspective(
        chosen: Option<FittedProjection>,
        authored: Option<FittedProjection>,
    ) -> bool {
        let mut app = ViewerApp::new(None).with_preferences(Preferences {
            language: Some(Language::ENGLISH),
            projection: chosen,
        });
        app.open(package_bytes(authored).as_slice()).unwrap();
        app.camera.as_ref().unwrap().is_perspective()
    }

    #[test]
    fn the_customer_s_projection_wins_over_the_author_s_which_wins_over_perspective() {
        use FittedProjection::{Orthographic, Perspective};
        assert!(opened_in_perspective(None, None));
        assert!(!opened_in_perspective(None, Some(Orthographic)));
        assert!(opened_in_perspective(None, Some(Perspective)));
        assert!(opened_in_perspective(Some(Perspective), Some(Orthographic)));
        assert!(!opened_in_perspective(Some(Orthographic), None));
        assert!(!opened_in_perspective(
            Some(Orthographic),
            Some(Perspective)
        ));
    }

    #[test]
    fn a_package_without_the_author_s_projection_stays_readable_by_older_viewers() {
        let old = package_bytes(None);
        assert!(!old.windows(17).any(|window| window == b"fitted_projection"));
        let authored = package_bytes(Some(FittedProjection::Orthographic));
        let reopened = ketchup_view_format::Package::read(authored.as_slice()).unwrap();
        assert_eq!(
            reopened.manifest.fitted_projection,
            Some(FittedProjection::Orthographic)
        );
    }

    #[test]
    fn the_projection_chosen_in_settings_applies_now_on_fit_and_to_the_next_package() {
        let mut app = ViewerApp::new(None).with_language(Language::ENGLISH);
        app.open(package_bytes(None).as_slice()).unwrap();
        let mut ui = harness(app);
        let perspective = |ui: &egui_kittest::Harness<'_, ViewerApp>| {
            ui.state().camera.as_ref().unwrap().is_perspective()
        };
        assert!(perspective(&ui));
        ui.get_by_label("Settings").click();
        ui.run();
        ui.get_by_label("Parallel").click();
        ui.run();
        assert!(!perspective(&ui));
        ui.get_by_label("Fit").click();
        ui.run();
        assert!(!perspective(&ui), "Fit keeps the chosen projection");
        let kept = ui.state_mut().take_changed_preferences().unwrap();
        assert_eq!(kept.projection, Some(FittedProjection::Orthographic));
        assert_eq!(Preferences::parse(&kept.to_text()), kept);

        let next = package_bytes(Some(FittedProjection::Perspective));
        ui.state_mut().open(next.as_slice()).unwrap();
        ui.run();
        assert!(!perspective(&ui), "the next package opens in it too");

        ui.get_by_label("Settings").click();
        ui.run();
        ui.get_by_label("As the author set").click();
        ui.run();
        ui.get_by_label("Fit").click();
        ui.run();
        assert!(perspective(&ui), "back to the author's projection");
    }

    /// Assumes `KETCHUP_LANGUAGE` is unset, as on CI: it would override the choice.
    #[test]
    fn a_kept_language_wins_over_the_system_language_at_the_next_start() {
        for language in Language::all() {
            let kept = Preferences {
                language: Some(language),
                projection: None,
            };
            let mut ui = harness(ViewerApp::new(None).with_preferences(kept));
            ui.get_by_label(language.text("viewer-settings"));
            assert_eq!(ui.state_mut().take_changed_preferences(), None);
        }
    }
}
