use super::*;

fn temporary_home(name: &str) -> PathBuf {
    let home =
        std::env::temp_dir().join(format!("ketchup-mcp-setup-{name}-{}", std::process::id()));
    if home.exists() {
        std::fs::remove_dir_all(&home).unwrap();
    }
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// A clean profile: a home with Roaming and Local folders, no command on PATH.
fn clean_profile(home: &Path) -> Environment {
    Environment {
        app_data: Some(home.join("AppData").join("Roaming")),
        local_app_data: Some(home.join("AppData").join("Local")),
        ..Environment::in_home(home.to_owned())
    }
}

fn create_parent(file: &Path) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn claude_code_is_set_up_for_every_project_and_codex_with_the_same_server() {
    let executable = r"C:\Users\me\AppData\Local\Programs\Ketchup\Ketchup.exe";
    assert_eq!(
        Harness::ClaudeCode.add_command_line("claude", executable),
        r#"claude mcp add --scope user ketchup -- "C:\Users\me\AppData\Local\Programs\Ketchup\Ketchup.exe" --mcp"#
    );
    assert_eq!(
        Harness::Codex.add_command_line("codex", executable),
        r#"codex mcp add ketchup -- "C:\Users\me\AppData\Local\Programs\Ketchup\Ketchup.exe" --mcp"#
    );
}

#[test]
fn every_installed_client_is_connected_through_its_own_file_without_its_command() {
    let home = temporary_home("connect");
    let environment = clean_profile(&home);
    // Claude Code has run once (its file exists); the others have their folders.
    let claude_code = environment.config_file(Harness::ClaudeCode).unwrap();
    std::fs::write(&claude_code, r#"{"numStartups": 3}"#).unwrap();
    for harness in [Harness::Codex, Harness::ClaudeDesktop] {
        create_parent(&environment.config_file(harness).unwrap());
    }
    let supervisor = environment.config_file(Harness::Supervisor).unwrap();
    create_parent(&supervisor);
    let this = std::env::current_exe().unwrap();
    for row in environment.rows(&this) {
        assert_eq!(
            row.registration,
            Registration::NotSetUp,
            "{:?}",
            row.harness
        );
        let connected = environment.set_up(row.harness, &this);
        assert_eq!(connected.setup_error, None, "{:?}", row.harness);
        assert_eq!(
            connected.registration,
            Registration::ThisKetchup,
            "{:?}",
            row.harness
        );
    }
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&claude_code).unwrap()).unwrap();
    assert_eq!(config["numStartups"], 3);
    assert_eq!(config["mcpServers"]["ketchup"]["type"], "stdio");
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&supervisor).unwrap()).unwrap();
    assert_eq!(config["mcpServers"]["ketchup"]["args"][0], "--mcp");
    assert!(config["mcpServers"]["ketchup"]["env"].is_object());
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn connecting_supervisor_again_replaces_its_old_copy_and_keeps_its_other_servers() {
    let home = temporary_home("supervisor");
    let environment = clean_profile(&home);
    let file = environment.config_file(Harness::Supervisor).unwrap();
    assert!(
        file.ends_with(
            Path::new("Programs")
                .join("Supervisor")
                .join("skills")
                .join("mcp_servers.json")
        )
    );
    create_parent(&file);
    let copy = home
        .join("Supervisor")
        .join("Kecup")
        .join("ketchup-app.exe");
    let servers = serde_json::json!({"mcpServers": {
        "ketchup": {"command": copy, "args": ["--mcp"], "env": {}},
        "mail": {"command": "mail.exe", "args": [], "env": {"A": "1"}}
    }});
    std::fs::write(&file, servers.to_string()).unwrap();
    let this = std::env::current_exe().unwrap();
    assert_eq!(
        environment.registration(Harness::Supervisor, &this),
        Registration::Elsewhere {
            command: copy.to_string_lossy().into_owned(),
            exists: false
        }
    );
    let row = environment.set_up(Harness::Supervisor, &this);
    assert_eq!(row.registration, Registration::ThisKetchup);
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(config["mcpServers"]["mail"]["env"]["A"], "1");
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_configuration_kecup_cannot_read_is_named_and_left_alone() {
    let home = temporary_home("broken");
    let environment = clean_profile(&home);
    let file = environment.config_file(Harness::ClaudeDesktop).unwrap();
    create_parent(&file);
    std::fs::write(&file, "{ broken").unwrap();
    let row = environment.set_up(Harness::ClaudeDesktop, &std::env::current_exe().unwrap());
    let error = row.setup_error.unwrap();
    assert!(error.contains("claude_desktop_config.json"), "{error}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ broken");
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_command_outside_the_windows_path_still_counts_as_installed() {
    let home = temporary_home("local-bin");
    let environment = clean_profile(&home);
    let this = std::env::current_exe().unwrap();
    assert_eq!(
        environment.registration(Harness::ClaudeCode, &this),
        Registration::NotInstalled
    );
    let bin = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let program = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    std::fs::write(bin.join(program), "").unwrap();
    assert_eq!(
        environment.registration(Harness::ClaudeCode, &this),
        Registration::NotSetUp
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_claude_code_entry_naming_a_deleted_build_is_reported_with_its_path() {
    let home = temporary_home("stale");
    let gone = home.join("target").join("release").join("ketchup-app.exe");
    let config = serde_json::json!({
        "mcpServers": {"ketchup": {"type": "stdio", "command": gone, "args": ["--mcp"]}}
    });
    std::fs::write(home.join(".claude.json"), config.to_string()).unwrap();
    let environment = Environment::in_home(home.clone());
    let this = std::env::current_exe().unwrap();
    assert_eq!(
        environment.registration(Harness::ClaudeCode, &this),
        Registration::Elsewhere {
            command: gone.to_string_lossy().into_owned(),
            exists: false
        }
    );
    let config = serde_json::json!({
        "mcpServers": {"ketchup": {"command": this, "args": ["--mcp"]}}
    });
    std::fs::write(home.join(".claude.json"), config.to_string()).unwrap();
    assert_eq!(
        environment.registration(Harness::ClaudeCode, &this),
        Registration::ThisKetchup
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_build_inside_a_checkout_is_started_through_its_server_script() {
    let checkout = temporary_home("checkout");
    let script = checkout.join("scripts").join("ketchup-mcp-server.cmd");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, "").unwrap();
    let target = checkout.join("target");
    for build in [
        target.join("release").join("ketchup-app.exe"),
        target
            .join("mcp-windows")
            .join("0123abcd")
            .join("ketchup-app.exe"),
        target
            .join("worktree")
            .join("release")
            .join("ketchup-app.exe"),
    ] {
        assert_eq!(entry_point(&build), script, "{}", build.display());
    }
    // A window the dev server opened sees the registered script as itself,
    // so it offers no replacement with its own temporary copy.
    let config = serde_json::json!({
        "mcpServers": {"ketchup": {"command": script, "args": ["--mcp"]}}
    });
    std::fs::write(checkout.join(".claude.json"), config.to_string()).unwrap();
    let environment = Environment::in_home(checkout.clone());
    let window = target
        .join("mcp-windows")
        .join("0123abcd")
        .join("ketchup-app.exe");
    assert_eq!(
        environment.registration(Harness::ClaudeCode, &entry_point(&window)),
        Registration::ThisKetchup
    );
    let installed = checkout
        .join("Programs")
        .join("Ketchup")
        .join("Ketchup.exe");
    assert_eq!(entry_point(&installed), installed);
    std::fs::remove_file(&script).unwrap();
    let build = target.join("release").join("ketchup-app.exe");
    assert_eq!(entry_point(&build), build);
    std::fs::remove_dir_all(checkout).unwrap();
}

#[test]
fn a_client_without_its_command_or_folder_is_told_apart() {
    let home = temporary_home("absent");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let program = if cfg!(windows) {
        "claude.cmd"
    } else {
        "claude"
    };
    std::fs::write(bin.join(program), "").unwrap();
    let environment = Environment {
        path: std::env::join_paths([&bin]).unwrap(),
        ..Environment::in_home(home.clone())
    };
    let this = std::env::current_exe().unwrap();
    let rows = environment.rows(&this);
    assert_eq!(rows[0].registration, Registration::NotSetUp);
    for row in &rows[1..] {
        assert_eq!(
            row.registration,
            Registration::NotInstalled,
            "{:?}",
            row.harness
        );
    }
    std::fs::remove_dir_all(home).unwrap();
}
