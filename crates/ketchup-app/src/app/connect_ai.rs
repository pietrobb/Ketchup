//! "Connect AI": Kečup has no built-in chat. Any MCP client (Claude Code,
//! Codex, …) starts `ketchup --mcp` and drives this window after the user
//! approves it here; this section shows the status, what each known client
//! (Claude Code, Codex, Claude Desktop, Supervisor) starts as `ketchup` with a
//! Connect button that writes its configuration, and, folded away, the set-up
//! to copy into any other client.

use crate::live_bridge::mcp_setup::{Environment, Harness, HarnessRow, Registration, entry_point};
use crate::*;

impl KetchupApp {
    pub(crate) fn show_connect_ai(&mut self, ui: &mut egui::Ui) {
        section_header(ui, self.palette(), &self.catalog.text("connect-ai-title"));
        ui.label(
            egui::RichText::new(
                self.connected_ai_text()
                    .unwrap_or_else(|| self.catalog.text("connect-ai-waiting")),
            )
            .strong(),
        );
        ui.label(self.catalog.text("connect-ai-description"));
        if let Some(id) = self.live_consent_instance_id() {
            ui.weak(self.catalog.format(
                "connect-ai-this-window",
                &BTreeMap::from([("name", ketchup_mcp::short_name(id))]),
            ));
            // Picking the window is separate from setting up a client: the
            // client's file is shared by every window and set up only once.
            if ui
                .button(self.catalog.text("connect-ai-use-this-window"))
                .clicked()
            {
                self.live.chosen_window_error =
                    self.mark_chosen_window().err().map(|e| e.to_string());
            }
        }
        let Some(server) = mcp_server_command() else {
            ui.weak(self.catalog.text("connect-ai-executable-unknown"));
            return;
        };
        let attached = self.ai_attached();
        let set_up = attached
            || self.live.harnesses.as_ref().is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.registration == Registration::ThisKetchup)
            });
        for (key, done) in [
            ("connect-ai-step-connect", set_up),
            ("connect-ai-step-restart", attached),
            ("connect-ai-step-attached", attached),
        ] {
            self.show_step(ui, key, done);
        }
        self.show_harnesses(ui, Path::new(&server));
        ui.add_space(6.0);
        // Copying is the fallback for a client Connect does not know.
        egui::CollapsingHeader::new(self.catalog.text("connect-ai-manual"))
            .id_salt("connect-ai-manual")
            .default_open(false)
            .show(ui, |ui| {
                for (label, text) in client_setups(&server) {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.strong(self.catalog.text(label));
                        if ui
                            .small_button(self.catalog.text("connect-ai-copy"))
                            .clicked()
                        {
                            ui.ctx().copy_text(text.clone());
                        }
                    });
                    ui.add(egui::Label::new(egui::RichText::new(text).monospace().small()).wrap());
                }
            });
    }

    /// One line per configuration file of a harness (one per Supervisor
    /// instance): what it starts as `ketchup`, and Set up when that is not
    /// this Kečup. The environment, which lists the running Supervisors, is
    /// read only when the rows are, not every frame.
    fn show_harnesses(&mut self, ui: &mut egui::Ui, executable: &Path) {
        let rows = match &self.live.harnesses {
            Some(rows) => rows.clone(),
            None => {
                let Some(environment) = Environment::current() else {
                    return;
                };
                self.live
                    .harnesses
                    .insert(environment.rows(executable))
                    .clone()
            }
        };
        let mut set_up = None;
        for (index, row) in rows.iter().enumerate() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.strong(row.harness.name());
                ui.label(self.harness_state(row));
                if let Some(file) = &row.file
                    && row.registration != Registration::NotInstalled
                {
                    ui.weak(file.display().to_string());
                }
                let action = match row.registration {
                    Registration::NotInstalled | Registration::ThisKetchup => None,
                    Registration::NotSetUp => Some("connect-ai-set-up"),
                    Registration::Elsewhere { .. } => Some("connect-ai-replace"),
                };
                if let Some(action) = action
                    && ui.button(self.catalog.text(action)).clicked()
                {
                    set_up = Some(index);
                }
            });
            if let Some(error) = &row.setup_error {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    self.catalog.text_because("connect-ai-set-up-failed", error),
                );
            }
        }
        if ui
            .small_button(self.catalog.text("connect-ai-check-again"))
            .clicked()
        {
            self.live.harnesses = None;
        }
        if set_up.is_some() {
            self.live.chosen_window_error = self.mark_chosen_window().err().map(|e| e.to_string());
        }
        if let Some(error) = &self.live.chosen_window_error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                self.catalog
                    .text_because("connect-ai-mark-window-failed", error),
            );
        }
        if let Some(index) = set_up
            && let Some(environment) = Environment::current()
        {
            let row = &rows[index];
            let row = environment.set_up_in(row.harness, row.file.clone(), executable);
            if let Some(slot) = self
                .live
                .harnesses
                .as_mut()
                .and_then(|rows| rows.get_mut(index))
            {
                *slot = row;
            }
        }
    }

    /// One numbered step, marked done or not from what the window sees.
    fn show_step(&self, ui: &mut egui::Ui, key: &str, done: bool) {
        let (mark, color, state) = if done {
            ("✔", self.palette().code().ok, "connect-ai-step-done")
        } else {
            ("✖", self.palette().faint, "connect-ai-step-not-done")
        };
        let text = self.catalog.text(key);
        let response = ui
            .horizontal(|ui| {
                ui.colored_label(color, mark);
                ui.label(&text);
            })
            .response;
        // A screen reader hears the step and whether it is done, not a glyph.
        let spoken = format!("{text} {}", self.catalog.text(state));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
    }

    /// "Connected AI: <name>" while an AI holds the window.
    fn connected_ai_text(&self) -> Option<String> {
        self.ai_attached().then(|| match &self.live.client_name {
            Some(name) => self
                .catalog
                .format("ai-connected", &BTreeMap::from([("client", name.clone())])),
            None => self.catalog.text("ai-connected-unnamed"),
        })
    }

    /// The status line's always visible part of the connection: which AI holds
    /// the window, and Disconnect.
    pub(crate) fn show_connected_ai(&mut self, ui: &mut egui::Ui) {
        let Some(text) = self.connected_ai_text() else {
            return;
        };
        ui.label(egui::RichText::new(text).color(self.palette().accent));
        if ui
            .small_button(self.catalog.text("ai-disconnect"))
            .clicked()
        {
            self.disconnect_ai();
        }
    }

    fn harness_state(&self, row: &HarnessRow) -> String {
        match &row.registration {
            Registration::NotInstalled => self.catalog.text("connect-ai-not-installed"),
            Registration::NotSetUp => self.catalog.text("connect-ai-not-set-up"),
            Registration::ThisKetchup => self.catalog.text("connect-ai-set-up-here"),
            Registration::Elsewhere { command, exists } => {
                let key = if *exists {
                    "connect-ai-starts-another"
                } else {
                    "connect-ai-starts-missing"
                };
                self.catalog
                    .format(key, &BTreeMap::from([("path", command.clone())]))
            }
        }
    }
}

/// The program an MCP client starts: this executable in server mode, or in a
/// development checkout its server script.
fn mcp_server_command() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|path| entry_point(&path))
        .and_then(|path| path.to_str().map(str::to_owned))
}

/// What to paste into each client, keyed by its label.
fn client_setups(executable: &str) -> [(&'static str, String); 3] {
    let json = serde_json::json!({
        "mcpServers": {"ketchup": {"command": executable, "args": ["--mcp"]}}
    });
    [
        (
            "connect-ai-claude-code",
            Harness::ClaudeCode.add_command_line("claude", executable),
        ),
        (
            "connect-ai-codex",
            Harness::Codex.add_command_line("codex", executable),
        ),
        (
            "connect-ai-json",
            serde_json::to_string_pretty(&json).unwrap_or_default(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::client_setups;
    use crate::KetchupApp;
    use crate::live_bridge::mcp_setup::{Harness, HarnessRow, Registration};
    use eframe::egui;
    use egui_kittest::{Harness as Kittest, kittest::Queryable as _};

    fn open_section(app: KetchupApp) -> Kittest<'static, KetchupApp> {
        let mut harness = Kittest::builder()
            .with_size(egui::Vec2::new(1600.0, 1000.0))
            .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
        harness.run();
        crate::tests::shell::open_dock_tab(&mut harness, crate::DockTab::Ai);
        harness
    }

    #[test]
    fn the_section_shows_the_status_and_folds_away_three_set_ups_to_copy() {
        let mut harness = open_section(KetchupApp::new());
        let waiting = harness.state().catalog.text("connect-ai-waiting");
        harness.get_by_label(&waiting);
        let copy = harness.state().catalog.text("connect-ai-copy");
        assert_eq!(harness.query_all_by_label(&copy).count(), 0);
        let manual = harness.state().catalog.text("connect-ai-manual");
        harness.get_by_label(&manual).click();
        harness.run();
        assert_eq!(harness.get_all_by_label(&copy).count(), 3);
    }

    #[test]
    fn every_known_client_has_a_row_and_one_not_set_up_offers_connect() {
        let mut app = KetchupApp::new();
        let row = |harness, registration| HarnessRow {
            harness,
            file: None,
            registration,
            setup_error: None,
        };
        app.live.harnesses = Some(vec![
            row(Harness::ClaudeCode, Registration::ThisKetchup),
            row(Harness::Codex, Registration::NotInstalled),
            row(Harness::ClaudeDesktop, Registration::NotInstalled),
            row(Harness::Supervisor, Registration::NotSetUp),
        ]);
        let harness = open_section(app);
        for client in Harness::ALL {
            harness.get_by_label(client.name());
        }
        let connect = harness.state().catalog.text("connect-ai-set-up");
        assert_eq!(harness.get_all_by_label(&connect).count(), 1);
        // Claude Code starts this Kečup, but no AI has attached yet.
        let catalog = &harness.state().catalog;
        let step =
            |key: &str, state: &str| format!("{} {}", catalog.text(key), catalog.text(state));
        harness.get_by_label(&step("connect-ai-step-connect", "connect-ai-step-done"));
        harness.get_by_label(&step("connect-ai-step-restart", "connect-ai-step-not-done"));
        harness.get_by_label(&step(
            "connect-ai-step-attached",
            "connect-ai-step-not-done",
        ));
    }

    /// #2687 P1 (points 4, 9, 10): which AI holds the window stays in the
    /// status line next to Disconnect, and every step is done.
    #[test]
    fn an_attached_ai_is_named_in_the_status_line_with_disconnect() {
        let mut app = KetchupApp::new();
        app.live.consent_attached = true;
        app.live.client_name = Some("Claude Code".to_owned());
        app.live.harnesses = Some(Vec::new());
        let mut harness = open_section(app);
        let catalog = harness.state().catalog.clone();
        let named = catalog.format(
            "ai-connected",
            &std::collections::BTreeMap::from([("client", "Claude Code".to_owned())]),
        );
        assert_eq!(
            harness.get_all_by_label(&named).count(),
            2,
            "status line and AI tab"
        );
        let done = catalog.text("connect-ai-step-done");
        for key in [
            "connect-ai-step-connect",
            "connect-ai-step-restart",
            "connect-ai-step-attached",
        ] {
            harness.get_by_label(&format!("{} {done}", catalog.text(key)));
        }

        harness.get_by_label(&catalog.text("ai-disconnect")).click();
        harness.run();
        assert!(!harness.state().ai_attached());
        assert!(harness.query_by_label(&named).is_none());
        harness.get_by_label(&catalog.text("connect-ai-waiting"));
    }

    /// #2687 P1 (point 3): before anything is set up the AI tab lists steps
    /// 1, 2, 3 top to bottom, each heard as not done.
    #[test]
    fn the_steps_are_numbered_top_to_bottom_and_none_is_done_before_set_up() {
        let mut app = KetchupApp::new();
        app.live.harnesses = Some(Vec::new());
        let harness = open_section(app);
        let catalog = harness.state().catalog.clone();
        let not_done = catalog.text("connect-ai-step-not-done");
        let tops: Vec<f32> = [
            "connect-ai-step-connect",
            "connect-ai-step-restart",
            "connect-ai-step-attached",
        ]
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let text = catalog.text(key);
            assert!(text.starts_with(&format!("{}.", index + 1)), "{text}");
            harness
                .get_by_label(&format!("{text} {not_done}"))
                .rect()
                .min
                .y
        })
        .collect();
        assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
        let done = catalog.text("connect-ai-step-done");
        assert!(
            harness
                .query_by_label_contains(&format!(" {done}"))
                .is_none()
        );
    }

    /// #2687 P1 (points 4, 9): with another tab open the status line still
    /// names the attached AI next to Disconnect, and Disconnect detaches it.
    #[test]
    fn the_attached_ai_and_disconnect_stay_visible_with_the_ai_tab_closed() {
        let mut app = KetchupApp::new();
        app.live.consent_attached = true;
        app.live.client_name = Some("Codex".to_owned());
        let mut harness = Kittest::builder()
            .with_size(egui::Vec2::new(1600.0, 1000.0))
            .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
        harness.run();
        let catalog = harness.state().catalog.clone();
        assert!(
            harness
                .query_by_label(&catalog.text("connect-ai-title"))
                .is_none(),
            "the AI tab is not open"
        );
        let named = catalog.format(
            "ai-connected",
            &std::collections::BTreeMap::from([("client", "Codex".to_owned())]),
        );
        assert_eq!(harness.get_all_by_label(&named).count(), 1);
        harness.get_by_label(&catalog.text("ai-disconnect")).click();
        harness.run();
        assert!(!harness.state().ai_attached());
        assert!(harness.query_by_label(&named).is_none());
        assert!(
            harness
                .query_by_label(&catalog.text("ai-disconnect"))
                .is_none()
        );
    }

    /// #2687 P1: the running Supervisors are listed (a scan of every process)
    /// when the AI tab first shows its rows and on Check again, not on every
    /// frame: rows already read stay as they are while the tab repaints.
    #[test]
    fn the_rows_are_read_once_and_on_check_again_not_every_frame() {
        let mut harness = open_section(KetchupApp::new());
        assert!(harness.state().live.harnesses.is_some());
        let marker = std::path::PathBuf::from(r"C:\not-read-from-disk\config.toml");
        harness.state_mut().live.harnesses = Some(vec![HarnessRow {
            harness: Harness::Codex,
            file: Some(marker.clone()),
            registration: Registration::NotSetUp,
            setup_error: None,
        }]);
        let files = |harness: &Kittest<'_, KetchupApp>| -> Vec<Option<std::path::PathBuf>> {
            let rows = harness.state().live.harnesses.as_ref();
            rows.map(|rows| rows.iter().map(|row| row.file.clone()).collect())
                .unwrap_or_default()
        };
        for _ in 0..10 {
            harness.step();
        }
        assert_eq!(files(&harness), vec![Some(marker.clone())]);
        let again = harness.state().catalog.text("connect-ai-check-again");
        harness.get_by_label(&again).click();
        harness.run();
        assert!(!files(&harness).contains(&Some(marker)));
    }

    /// #2687 point 16: Connect in this window's AI tab makes it the window a
    /// `connect` without an id goes to, and the tab shows the short name the
    /// AI lists this window by.
    #[test]
    fn connect_marks_this_window_for_a_connect_without_an_id() {
        let folder = tempfile::tempdir().unwrap();
        let discovery = folder.path().join("live-instances");
        let mut app = KetchupApp::new();
        app.enable_live_consent_broker_in(&egui::Context::default(), &discovery)
            .unwrap();
        let id = app.live_consent_instance_id().unwrap().to_owned();
        let config = folder.path().join("mcp_servers.json");
        app.live.harnesses = Some(vec![HarnessRow {
            harness: Harness::Supervisor,
            file: Some(config.clone()),
            registration: Registration::NotSetUp,
            setup_error: None,
        }]);
        let mut harness = open_section(app);
        let catalog = harness.state().catalog.clone();
        assert_eq!(ketchup_mcp::chosen_window(&discovery).unwrap(), None);
        harness
            .get_by_label(&catalog.text("connect-ai-set-up"))
            .click();
        harness.run();
        assert_eq!(
            ketchup_mcp::chosen_window(&discovery).unwrap().as_deref(),
            Some(id.as_str())
        );
        assert!(harness.state().live.chosen_window_error.is_none());
        assert!(config.is_file(), "Connect still writes the client's file");
        let name = catalog.format(
            "connect-ai-this-window",
            &std::collections::BTreeMap::from([("name", ketchup_mcp::short_name(&id))]),
        );
        assert!(name.contains(&ketchup_mcp::short_name(&id)), "{name}");
        harness.get_by_label(&name);
    }

    /// #2687 point 16 once a client is set up: its file is shared by every
    /// window, so window B shows the client as starting this Kečup and has
    /// nothing to set up, yet the user still picks B over A, where Connect
    /// was clicked earlier.
    #[test]
    fn a_second_window_is_picked_after_the_client_is_already_set_up() {
        let folder = tempfile::tempdir().unwrap();
        let discovery = folder.path().join("live-instances");
        let mut first = KetchupApp::new();
        first
            .enable_live_consent_broker_in(&egui::Context::default(), &discovery)
            .unwrap();
        first.mark_chosen_window().unwrap();
        let first_id = first.live_consent_instance_id().unwrap().to_owned();
        let mut second = KetchupApp::new();
        second
            .enable_live_consent_broker_in(&egui::Context::default(), &discovery)
            .unwrap();
        let second_id = second.live_consent_instance_id().unwrap().to_owned();
        assert_ne!(first_id, second_id);
        second.live.harnesses = Some(vec![HarnessRow {
            harness: Harness::ClaudeCode,
            file: None,
            registration: Registration::ThisKetchup,
            setup_error: None,
        }]);
        let mut harness = open_section(second);
        let catalog = harness.state().catalog.clone();
        assert!(
            harness
                .query_by_label(&catalog.text("connect-ai-set-up"))
                .is_none(),
            "the client is already set up"
        );
        let pick = catalog.text("connect-ai-use-this-window");
        assert!(
            harness.query_by_label(&pick).is_some(),
            "window B offers no way to be picked; `connect` stays with A"
        );
        harness.get_by_label(&pick).click();
        harness.run();
        assert_eq!(
            ketchup_mcp::chosen_window(&discovery).unwrap().as_deref(),
            Some(second_id.as_str())
        );
        assert!(harness.state().live.chosen_window_error.is_none());
        drop(first);
    }

    #[test]
    fn a_harness_starting_a_deleted_build_is_named_with_its_path_and_offered_a_replacement() {
        let mut app = KetchupApp::new();
        let gone = r"C:\Sources\Ketchup\target\release\ketchup-app.exe";
        app.live.harnesses = Some(vec![
            HarnessRow {
                harness: Harness::ClaudeCode,
                file: None,
                registration: Registration::Elsewhere {
                    command: gone.to_owned(),
                    exists: false,
                },
                setup_error: None,
            },
            HarnessRow {
                harness: Harness::Codex,
                file: None,
                registration: Registration::NotInstalled,
                setup_error: None,
            },
        ]);
        let harness = open_section(app);
        let catalog = &harness.state().catalog;
        let missing = catalog.format(
            "connect-ai-starts-missing",
            &std::collections::BTreeMap::from([("path", gone.to_owned())]),
        );
        assert!(missing.contains(gone), "{missing}");
        harness.get_by_label(&missing);
        harness.get_by_label(&catalog.text("connect-ai-not-installed"));
        assert_eq!(
            harness
                .get_all_by_label(&catalog.text("connect-ai-replace"))
                .count(),
            1
        );
        assert!(
            harness
                .query_by_label(&catalog.text("connect-ai-set-up"))
                .is_none()
        );
    }

    #[test]
    fn every_client_setup_starts_this_executable_in_server_mode() {
        let executable = r"C:\Program Files\Kečup\ketchup-app.exe";
        let [claude, codex, json] = client_setups(executable);
        assert_eq!(
            claude.1,
            r#"claude mcp add --scope user ketchup -- "C:\Program Files\Kečup\ketchup-app.exe" --mcp"#
        );
        assert!(
            codex
                .1
                .starts_with("codex mcp add ketchup -- \"C:\\Program Files")
        );
        let parsed: serde_json::Value = serde_json::from_str(&json.1).unwrap();
        assert_eq!(parsed["mcpServers"]["ketchup"]["command"], executable);
        assert_eq!(parsed["mcpServers"]["ketchup"]["args"][0], "--mcp");
    }
}
