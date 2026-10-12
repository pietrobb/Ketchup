//! Plugins menu in an offscreen window: the folders under the plugins folder
//! are listed, one that could not run with the reason MCP gets, and a plugin
//! run from the menu takes the same consent as MCP `run_plugin`, one Undo step.
use super::run_plugin::{EXTRUSION, folder_request, height, height_folder};
use super::*;
use crate::live_bridge::measurement::tests::integration_support::file_turn;
use crate::live_bridge::plugin_menu::{PluginMenu, rejection_text};
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// A request over the window's bridge, answered while the window runs frames.
struct Client {
    stream: TcpStream,
    token: String,
    id: u64,
}

impl Client {
    fn connect(harness: &mut Harness<'_, KetchupApp>) -> Self {
        let context = harness.ctx.clone();
        let address = harness.state_mut().enable_live_bridge(&context).unwrap();
        let token = harness.state().live_bridge_credentials().unwrap().token;
        Self {
            stream: TcpStream::connect(address).unwrap(),
            token,
            id: 0,
        }
    }

    fn answer(&mut self, harness: &mut Harness<'_, KetchupApp>, request: Request) -> Response {
        self.id += 1;
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            id: self.id,
            token: self.token.clone(),
            request,
        })
        .unwrap();
        let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
        frame.extend(bytes);
        self.stream.write_all(&frame).unwrap();
        let mut reader = self.stream.try_clone().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_secs(120)))
            .unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut header = [0; 4];
            reader.read_exact(&mut header).unwrap();
            let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
            reader.read_exact(&mut bytes).unwrap();
            tx.send(serde_json::from_slice::<Response>(&bytes).unwrap())
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Ok(response) = rx.recv_timeout(Duration::from_millis(5)) {
                return response;
            }
            assert!(Instant::now() < deadline, "no answer");
            harness.step();
        }
    }
}

fn open_menu(harness: &mut Harness<'_, KetchupApp>) {
    let menu = harness.state().catalog.text("menu-plugins");
    harness.get_by_role_and_label(Role::Button, &menu).click();
    harness.run_steps(2);
}

#[test]
fn the_plugins_menu_lists_every_folder_and_runs_one_with_the_mcp_consent() {
    let _turn = file_turn();
    let root = tempfile::tempdir().unwrap();
    let (folder, _) = height_folder(root.path());
    let broken = root.path().join("broken");
    std::fs::create_dir(&broken).unwrap();
    std::fs::write(broken.join("plugin.json"), r#"{"name": "Broken"}"#).unwrap();
    std::fs::write(root.path().join("notes.txt"), "not a plugin").unwrap();

    // The user allows the first prompt and refuses any further one.
    let dialogs = ScriptedFileDialogs::new()
        .queue_high_risk_approval(1)
        .queue_refused_high_risk();
    let probe = dialogs.clone();
    let mut app = KetchupApp::new();
    app.selection.clear();
    app.dialogs = Box::new(dialogs);
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceGrounded {
                id: OccurrenceId(1),
                grounded: true,
            },
        ]))
        .unwrap();
    crate::tests::install_initial_graph_result(&mut app);
    app.live.plugin_menu = PluginMenu::at(Some(root.path().to_owned()));
    let original = height(&app);
    let undo = app.undo_step_count();
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    let mut client = Client::connect(&mut harness);

    // The broken folder is listed as unavailable with the reason MCP gets.
    let refused = client.answer(
        &mut harness,
        Request::RunPlugin(folder_request(&broken, json!({}))),
    );
    assert_eq!(refused.error.as_deref(), Some("plugin_manifest_invalid"));
    let reason = rejection_text(refused.result.as_ref().unwrap());
    open_menu(&mut harness);
    let catalog = &harness.state().catalog;
    let unavailable = catalog.format(
        "plugins-unavailable",
        &BTreeMap::from([("name", "broken".to_owned())]),
    );
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Height")
            .is_some()
    );
    for shown in [unavailable.as_str(), reason.as_str()] {
        assert!(
            harness.query_by_label(shown).is_some(),
            "{shown:?} not in the menu"
        );
    }
    assert!(harness.query_by_label("notes.txt").is_none());

    // The valid one asks for its inputs, defaults filled in from the manifest.
    harness
        .get_by_role_and_label(Role::Button, "Height")
        .click();
    harness.run_steps(2);
    let feature = harness.get_by_role_and_label(Role::TextInput, "feature_id");
    assert_eq!(
        feature.value().as_deref(),
        Some(EXTRUSION.to_string().as_str())
    );
    let field = harness.get_by_role_and_label(Role::TextInput, "height_mm");
    field.focus();
    field.type_text("35");
    harness.run_steps(2);
    let run = harness.state().catalog.text("plugins-run");
    harness.get_by_role_and_label(Role::Button, &run).click();
    harness.step();

    // Busy while it runs: an MCP edit is refused, the document is untouched.
    assert!(harness.state().live.plugin_menu.is_running());
    assert_eq!(
        busy::reject_if_busy(&busy::blockers(harness.state(), false)),
        Err("busy")
    );
    let deadline = Instant::now() + Duration::from_secs(120);
    while harness.state().live.plugin_menu.is_running() {
        assert!(Instant::now() < deadline, "the plugin did not finish");
        harness.step();
    }
    harness.run_steps(2);
    let prompts = probe.high_risk_prompts();
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].contains(&folder.display().to_string()),
        "{}",
        prompts[0]
    );
    let published = harness.state().catalog.text("plugins-published");
    assert!(harness.query_by_label(&published).is_some());
    assert_eq!(height(harness.state()), 35.0);
    assert_eq!(harness.state().undo_step_count(), undo + 1);

    // The consent given in the window is the one MCP needs: no second prompt.
    let response = client.answer(
        &mut harness,
        Request::RunPlugin(folder_request(&folder, json!({"height_mm": 40}))),
    );
    assert!(response.ok, "{:?}", response.result);
    assert_eq!(probe.high_risk_prompts().len(), 1);
    assert_eq!(height(harness.state()), 40.0);

    // Undo steps back one plugin run at a time.
    for expected in [35.0, original] {
        let response = client.answer(&mut harness, Request::Undo { expected: None });
        assert!(response.ok, "{:?}", response.error);
        assert_eq!(height(harness.state()), expected);
    }
    assert_eq!(harness.state().undo_step_count(), undo);
}

#[test]
fn open_plugins_folder_creates_it_and_refresh_reads_it_again() {
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("Ketchup").join("plugins");
    let dialogs = ScriptedFileDialogs::new();
    let probe = dialogs.clone();
    let mut app = KetchupApp::new();
    app.dialogs = Box::new(dialogs);
    app.live.plugin_menu = PluginMenu::at(Some(plugins.clone()));
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    open_menu(&mut harness);
    let none = harness.state().catalog.format(
        "plugins-none",
        &BTreeMap::from([("folder", plugins.display().to_string())]),
    );
    assert!(harness.query_by_label(&none).is_some());
    let open = harness.state().catalog.text("plugins-open-folder");
    harness.get_by_role_and_label(Role::Button, &open).click();
    harness.run_steps(2);
    assert!(plugins.is_dir());
    assert_eq!(probe.shown_folders(), vec![plugins.clone()]);

    // A folder added later shows up only after Refresh.
    let (_, _) = height_folder(&plugins);
    open_menu(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Height")
            .is_none()
    );
    let refresh = harness.state().catalog.text("plugins-refresh");
    harness
        .get_by_role_and_label(Role::Button, &refresh)
        .click();
    harness.run_steps(2);
    // A click closes the menu, as every menu entry does.
    assert!(
        harness
            .query_by_role_and_label(Role::Button, &refresh)
            .is_none()
    );
    open_menu(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(Role::Button, "Height")
            .is_some()
    );
}
