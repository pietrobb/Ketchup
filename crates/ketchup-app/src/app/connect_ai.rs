//! "Connect AI": Kečup has no built-in chat. Any MCP client (Claude Code,
//! Codex, …) starts `ketchup --mcp` and drives this window after the user
//! approves it here; this section shows the status, what Claude Code and Codex
//! start as `ketchup` with a button that sets them up, and the set-up to copy.

use crate::live_bridge::mcp_setup::{Environment, Harness, HarnessRow, Registration, entry_point};
use crate::*;

impl KetchupApp {
    pub(crate) fn show_connect_ai(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(self.catalog.text("connect-ai-title"))
            .id_salt("connect-ai")
            .default_open(false)
            .show(ui, |ui| {
                let status = if self.live.consent_attached || self.live.launch_attached {
                    "connect-ai-connected"
                } else {
                    "connect-ai-waiting"
                };
                ui.label(egui::RichText::new(self.catalog.text(status)).strong());
                ui.label(self.catalog.text("connect-ai-description"));
                let Some(server) = mcp_server_command() else {
                    ui.weak(self.catalog.text("connect-ai-executable-unknown"));
                    return;
                };
                self.show_harnesses(ui, Path::new(&server));
                ui.add_space(6.0);
                ui.weak(self.catalog.text("connect-ai-manual"));
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

    /// One line per harness: what it starts as `ketchup`, and Set up when that
    /// is not this Kečup.
    fn show_harnesses(&mut self, ui: &mut egui::Ui, executable: &Path) {
        let Some(environment) = Environment::current() else {
            return;
        };
        let rows = self
            .live
            .harnesses
            .get_or_insert_with(|| environment.rows(executable))
            .clone();
        let mut set_up = None;
        for row in &rows {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.strong(row.harness.name());
                ui.label(self.harness_state(row));
                let action = match row.registration {
                    Registration::NotInstalled | Registration::ThisKetchup => None,
                    Registration::NotSetUp => Some("connect-ai-set-up"),
                    Registration::Elsewhere { .. } => Some("connect-ai-replace"),
                };
                if let Some(action) = action
                    && ui.button(self.catalog.text(action)).clicked()
                {
                    set_up = Some(row.harness);
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
        if let Some(harness) = set_up {
            let row = environment.set_up(harness, executable);
            if let Some(rows) = &mut self.live.harnesses
                && let Some(slot) = rows.iter_mut().find(|slot| slot.harness == harness)
            {
                *slot = row;
            }
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
            Harness::ClaudeCode.add_command_line(executable),
        ),
        (
            "connect-ai-codex",
            Harness::Codex.add_command_line(executable),
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
        let title = harness.state().catalog.text("connect-ai-title");
        harness.get_by_label(&title).click();
        harness.run();
        harness
    }

    #[test]
    fn the_section_shows_the_status_and_three_set_ups_to_copy() {
        let harness = open_section(KetchupApp::new());
        let waiting = harness.state().catalog.text("connect-ai-waiting");
        harness.get_by_label(&waiting);
        let copy = harness.state().catalog.text("connect-ai-copy");
        assert_eq!(harness.get_all_by_label(&copy).count(), 3);
    }

    #[test]
    fn a_harness_starting_a_deleted_build_is_named_with_its_path_and_offered_a_replacement() {
        let mut app = KetchupApp::new();
        let gone = r"C:\Sources\Ketchup\target\release\ketchup-app.exe";
        app.live.harnesses = Some(vec![
            HarnessRow {
                harness: Harness::ClaudeCode,
                registration: Registration::Elsewhere {
                    command: gone.to_owned(),
                    exists: false,
                },
                setup_error: None,
            },
            HarnessRow {
                harness: Harness::Codex,
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
