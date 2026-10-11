//! "Connect AI": Kečup has no built-in chat. Any MCP client (Claude Code,
//! Codex, …) starts `ketchup --mcp` and drives this window after the user
//! approves it here; this section shows the status and the client set-up to copy.

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
}

/// The program an MCP client starts: this executable in server mode.
fn mcp_server_command() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(str::to_owned))
}

/// What to paste into each client, keyed by its label.
fn client_setups(executable: &str) -> [(&'static str, String); 3] {
    let quoted = format!("\"{executable}\"");
    let json = serde_json::json!({
        "mcpServers": {"ketchup": {"command": executable, "args": ["--mcp"]}}
    });
    [
        (
            "connect-ai-claude-code",
            format!("claude mcp add ketchup -- {quoted} --mcp"),
        ),
        (
            "connect-ai-codex",
            format!("codex mcp add ketchup -- {quoted} --mcp"),
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
    use eframe::egui;
    use egui_kittest::{Harness, kittest::Queryable as _};

    #[test]
    fn the_section_shows_the_status_and_three_set_ups_to_copy() {
        let mut harness = Harness::builder()
            .with_size(egui::Vec2::new(1600.0, 1000.0))
            .build_state(
                |context, app: &mut KetchupApp| app.ui(context),
                KetchupApp::new(),
            );
        harness.run();
        let title = harness.state().catalog.text("connect-ai-title");
        harness.get_by_label(&title).click();
        harness.run();
        let waiting = harness.state().catalog.text("connect-ai-waiting");
        harness.get_by_label(&waiting);
        let copy = harness.state().catalog.text("connect-ai-copy");
        assert_eq!(harness.get_all_by_label(&copy).count(), 3);
    }

    #[test]
    fn every_client_setup_starts_this_executable_in_server_mode() {
        let executable = r"C:\Program Files\Kečup\ketchup-app.exe";
        let [claude, codex, json] = client_setups(executable);
        assert_eq!(
            claude.1,
            r#"claude mcp add ketchup -- "C:\Program Files\Kečup\ketchup-app.exe" --mcp"#
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
