//! An AI client set up for a Kečup that is gone, told when the window opens
//! (feedback 2026-11-10): after a move to another folder (Manual Alpha to the
//! release), Claude Code still started the deleted `Ketchup.exe` and every
//! Kečup tool failed with "file not found", while Connect AI, which says so
//! and offers the replacement, was never opened. The window now reads the
//! clients once at start-up and asks before writing any of them.

use crate::app::connect_ai::{HarnessAnswer, HarnessWork, mcp_server_command};
use crate::live_bridge::mcp_setup::{HarnessRow, Registration};
use crate::*;

/// Where the start-up check of the AI clients is.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) enum StaleClientCheck {
    /// No check: a window made by a test or a script, not the user's.
    #[default]
    Off,
    /// The user's window, its clients not read yet.
    Due,
    /// The clients at these rows start a program that no longer exists.
    Offered(Vec<usize>),
    /// They are being set up to start this Kečup.
    Fixing(Vec<usize>),
    /// Set up; how it went for each.
    Fixed(Vec<usize>),
    /// Nothing to tell, or the user closed it.
    Done,
}

impl KetchupApp {
    pub(crate) fn show_stale_ai_clients(&mut self, context: &egui::Context) {
        if matches!(
            self.live.stale_clients,
            StaleClientCheck::Off | StaleClientCheck::Done
        ) {
            return;
        }
        // Connect AI shares the rows and their work; either takes the answer.
        self.take_harness_answer();
        let server = match mcp_server_command() {
            Ok(server) => server,
            Err(error) => {
                ketchup_rejection::log!("the AI clients are not checked at start-up: {error}");
                self.live.stale_clients = StaleClientCheck::Done;
                return;
            }
        };
        let executable = PathBuf::from(server);
        match std::mem::take(&mut self.live.stale_clients) {
            StaleClientCheck::Off | StaleClientCheck::Done => {}
            StaleClientCheck::Due => self.check_ai_clients(context, &executable),
            StaleClientCheck::Offered(stale) => self.offer_fix(context, stale, &executable),
            StaleClientCheck::Fixing(stale) => {
                self.live.stale_clients = if self.live.harness_work.is_some() {
                    StaleClientCheck::Fixing(stale)
                } else {
                    StaleClientCheck::Fixed(stale)
                };
            }
            StaleClientCheck::Fixed(stale) => self.show_fixed(context, stale),
        }
    }

    /// Read the clients once, unless Connect AI already has; then offer the
    /// fix if any starts a program that is gone.
    fn check_ai_clients(&mut self, context: &egui::Context, executable: &Path) {
        let Some(rows) = &self.live.harnesses else {
            if self.live.harness_work.is_none() {
                let (read, executable) = (self.live.harness_rows, executable.to_owned());
                self.live.harness_work =
                    HarnessWork::start(context, move || HarnessAnswer::Rows(read(&executable)));
            }
            self.live.stale_clients = if self.live.harness_work.is_some() {
                StaleClientCheck::Due
            } else {
                StaleClientCheck::Done
            };
            return;
        };
        let stale: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| starts_a_deleted_program(row))
            .map(|(index, _)| index)
            .collect();
        self.live.stale_clients = if stale.is_empty() {
            StaleClientCheck::Done
        } else {
            StaleClientCheck::Offered(stale)
        };
    }

    fn offer_fix(&mut self, context: &egui::Context, stale: Vec<usize>, executable: &Path) {
        let lines: Vec<String> = self
            .live
            .harnesses
            .iter()
            .flatten()
            .enumerate()
            .filter(|(index, _)| stale.contains(index))
            .filter_map(|(_, row)| match &row.registration {
                Registration::Elsewhere { command, .. } => {
                    Some(format!("{}: {command}", row.harness.name()))
                }
                _ => None,
            })
            .collect();
        let (mut fix, mut later) = (false, false);
        self.notice_window(context, "stale-ai-title", |app, ui| {
            ui.label(app.catalog.text("stale-ai-description"));
            for line in &lines {
                ui.monospace(line);
            }
            ui.horizontal(|ui| {
                // Connect AI may be setting a client up; the fix waits for it.
                fix = ui
                    .add_enabled(
                        app.live.harness_work.is_none(),
                        egui::Button::new(app.catalog.text("connect-ai-replace")),
                    )
                    .clicked();
                later = ui.button(app.catalog.text("stale-ai-later")).clicked();
            });
        });
        self.live.stale_clients = if fix {
            self.set_up_harnesses(context, &stale, executable);
            StaleClientCheck::Fixing(stale)
        } else if later {
            StaleClientCheck::Done
        } else {
            StaleClientCheck::Offered(stale)
        };
    }

    /// Which clients now start this Kečup, and why any other could not be set up.
    fn show_fixed(&mut self, context: &egui::Context, stale: Vec<usize>) {
        let rows: Vec<HarnessRow> = self
            .live
            .harnesses
            .iter()
            .flatten()
            .enumerate()
            .filter(|(index, _)| stale.contains(index))
            .map(|(_, row)| row.clone())
            .collect();
        let fixed: Vec<&str> = rows
            .iter()
            .filter(|row| row.registration == Registration::ThisKetchup)
            .map(|row| row.harness.name())
            .collect();
        let mut close = false;
        self.notice_window(context, "stale-ai-title", |app, ui| {
            if !fixed.is_empty() {
                ui.label(app.catalog.format(
                    "stale-ai-fixed",
                    &BTreeMap::from([("clients", fixed.join(", "))]),
                ));
            }
            for row in rows
                .iter()
                .filter(|row| row.registration != Registration::ThisKetchup)
            {
                let reason = match &row.setup_error {
                    Some(error) => app.catalog.text_because("connect-ai-set-up-failed", error),
                    None => app.harness_state(row),
                };
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{}: {reason}", row.harness.name()),
                );
            }
            close = ui.button(app.catalog.text("stale-ai-close")).clicked();
        });
        self.live.stale_clients = if close {
            StaleClientCheck::Done
        } else {
            StaleClientCheck::Fixed(stale)
        };
    }

    fn notice_window(
        &mut self,
        context: &egui::Context,
        title: &str,
        content: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        egui::Window::new(self.catalog.text(title))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 96.0))
            .show(context, |ui| content(self, ui));
    }
}

/// The client's `ketchup` starts a program that is not there any more. One
/// that exists may be another Kečup on purpose and is left to Connect AI.
fn starts_a_deleted_program(row: &HarnessRow) -> bool {
    matches!(
        row.registration,
        Registration::Elsewhere { exists: false, .. }
    )
}

#[cfg(test)]
mod tests {
    use super::StaleClientCheck;
    use crate::KetchupApp;
    use crate::app::connect_ai::HarnessWork;
    use crate::live_bridge::mcp_setup::{Harness, HarnessRow, Registration};
    use eframe::egui;
    use egui_kittest::{Harness as Kittest, kittest::Queryable as _};
    use std::collections::BTreeMap;

    const GONE: &str = r"C:\Users\someone\AppData\Local\Programs\Ketchup Manual Alpha\Ketchup.exe";

    fn row(
        harness: Harness,
        file: Option<std::path::PathBuf>,
        registration: Registration,
    ) -> HarnessRow {
        HarnessRow {
            harness,
            file,
            registration,
            setup_error: None,
        }
    }

    fn gone() -> Registration {
        Registration::Elsewhere {
            command: GONE.to_owned(),
            exists: false,
        }
    }

    /// Frames run after each step: a fixed count, not `run()`'s "until no repaint", which
    /// fails when the model's exact evaluation keeps waking the window for its progress.
    const FRAMES: usize = 4;

    fn window(app: KetchupApp) -> Kittest<'static, KetchupApp> {
        let mut harness = Kittest::builder()
            .with_size(egui::Vec2::new(1600.0, 1000.0))
            .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
        harness.run_steps(FRAMES);
        harness
    }

    /// Frames until no client is being read or set up, and one more for the
    /// notice to follow.
    fn settle(harness: &mut Kittest<'_, KetchupApp>) {
        while let Some(work) = harness.state_mut().live.harness_work.take() {
            // The answer, waited for, goes back for the next frame to take.
            let answer = work
                .answer
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("the clients' work did not end");
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            sender.send(answer).unwrap();
            harness.state_mut().live.harness_work = Some(HarnessWork { answer: receiver });
            harness.run_steps(FRAMES);
        }
        harness.run_steps(FRAMES);
    }

    fn starting_a_deleted_kecup(_: &std::path::Path) -> Vec<HarnessRow> {
        vec![
            row(Harness::ClaudeCode, None, gone()),
            row(Harness::Codex, None, Registration::ThisKetchup),
            row(
                Harness::ClaudeDesktop,
                None,
                Registration::Elsewhere {
                    command: r"C:\Tools\other-ketchup.exe".to_owned(),
                    exists: true,
                },
            ),
        ]
    }

    fn all_set_up(_: &std::path::Path) -> Vec<HarnessRow> {
        vec![row(Harness::ClaudeCode, None, Registration::ThisKetchup)]
    }

    #[test]
    fn the_users_window_names_a_client_starting_a_deleted_kecup_and_offers_the_fix() {
        let mut app = KetchupApp::new();
        app.live.harness_rows = starting_a_deleted_kecup;
        app.live.stale_clients = StaleClientCheck::Due;
        let mut harness = window(app);
        settle(&mut harness);
        let catalog = harness.state().catalog.clone();
        harness.get_by_label(&catalog.text("stale-ai-title"));
        harness.get_by_label(&format!("{}: {GONE}", Harness::ClaudeCode.name()));
        // Another Kečup that exists may be on purpose; only the missing one is named.
        assert!(
            harness
                .query_by_label_contains(r"C:\Tools\other-ketchup.exe")
                .is_none()
        );
        harness.get_by_label(&catalog.text("connect-ai-replace"));
        harness
            .get_by_label(&catalog.text("stale-ai-later"))
            .click();
        harness.run_steps(FRAMES);
        assert_eq!(harness.state().live.stale_clients, StaleClientCheck::Done);
        assert!(
            harness
                .query_by_label(&catalog.text("stale-ai-title"))
                .is_none()
        );
    }

    #[test]
    fn nothing_is_shown_when_every_client_starts_a_kecup_that_exists() {
        let mut app = KetchupApp::new();
        app.live.harness_rows = all_set_up;
        app.live.stale_clients = StaleClientCheck::Due;
        let mut harness = window(app);
        settle(&mut harness);
        assert_eq!(harness.state().live.stale_clients, StaleClientCheck::Done);
        let catalog = harness.state().catalog.clone();
        assert!(
            harness
                .query_by_label(&catalog.text("stale-ai-title"))
                .is_none()
        );
    }

    /// A window made by a test or a script reads no client and asks nothing.
    #[test]
    fn a_window_not_made_for_the_user_reads_no_client() {
        let mut app = KetchupApp::new();
        app.live.harness_rows = starting_a_deleted_kecup;
        let mut harness = window(app);
        harness.run_steps(FRAMES);
        assert!(harness.state().live.harness_work.is_none());
        assert!(harness.state().live.harnesses.is_none());
        let catalog = harness.state().catalog.clone();
        assert!(
            harness
                .query_by_label(&catalog.text("stale-ai-title"))
                .is_none()
        );
    }

    /// The fix writes the client's file to start this Kečup, nothing else of
    /// it, and says to restart the client.
    #[test]
    fn the_fix_points_the_client_at_this_kecup_and_says_to_restart_it() {
        let folder = tempfile::tempdir().unwrap();
        let config = folder.path().join("mcp_servers.json");
        let other = serde_json::json!({"command": "python", "args": ["other.py"]});
        std::fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({"mcpServers": {
                "ketchup": {"command": GONE, "args": ["--mcp"]},
                "other": other,
            }}))
            .unwrap(),
        )
        .unwrap();
        let mut app = KetchupApp::new();
        app.live.harnesses = Some(vec![row(Harness::Supervisor, Some(config.clone()), gone())]);
        app.live.stale_clients = StaleClientCheck::Due;
        let mut harness = window(app);
        let catalog = harness.state().catalog.clone();
        harness
            .get_by_label(&catalog.text("connect-ai-replace"))
            .click();
        harness.run_steps(FRAMES);
        settle(&mut harness);
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        let command = written["mcpServers"]["ketchup"]["command"]
            .as_str()
            .unwrap();
        assert_ne!(command, GONE);
        assert!(std::path::Path::new(command).is_file(), "{command}");
        assert_eq!(written["mcpServers"]["other"], other);
        assert_eq!(
            harness.state().live.harnesses.as_ref().unwrap()[0].registration,
            Registration::ThisKetchup
        );
        let fixed = catalog.format(
            "stale-ai-fixed",
            &BTreeMap::from([("clients", Harness::Supervisor.name().to_owned())]),
        );
        harness.get_by_label(&fixed);
        harness
            .get_by_label(&catalog.text("stale-ai-close"))
            .click();
        harness.run_steps(FRAMES);
        assert_eq!(harness.state().live.stale_clients, StaleClientCheck::Done);
    }
}
