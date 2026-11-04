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

/// The installer runs `--mcp-setup` hidden: it connects the installed
/// Supervisor, while a running dev instance keeps its own Kečup until the
/// user clicks Connect on its row.
#[test]
fn mcp_setup_connects_the_installed_supervisor_and_leaves_other_instances() {
    let home = temporary_home("installed-only");
    let dev = home.join("DevSupervisor");
    let environment = Environment {
        supervisor_folders: Ok(vec![dev.clone()]),
        ..clean_profile(&home)
    };
    let installed = environment.config_file(Harness::Supervisor).unwrap();
    create_parent(&installed);
    let skills = dev.join("skills");
    std::fs::create_dir_all(&skills).unwrap();
    std::fs::write(skills.join("mcp_bridge.py"), "").unwrap();
    let dev_file = skills.join("mcp_servers.json");
    let dev_servers = serde_json::json!({"mcpServers": {
        "ketchup": {"command": r"C:\dev\ketchup-mcp-server.cmd", "args": ["--mcp"]}
    }})
    .to_string();
    std::fs::write(&dev_file, &dev_servers).unwrap();
    let this = std::env::current_exe().unwrap();
    let rows = environment.set_up_installed(&this);
    assert_eq!(std::fs::read_to_string(&dev_file).unwrap(), dev_servers);
    let supervisor: Vec<_> = rows
        .iter()
        .filter(|row| row.harness == Harness::Supervisor)
        .map(|row| (row.file.clone(), row.registration.clone()))
        .collect();
    assert_eq!(
        supervisor,
        [(Some(installed), Registration::ThisKetchup)],
        "only the installed Supervisor is reported and connected"
    );
    std::fs::remove_dir_all(home).unwrap();
}

/// Review 2026-11-04 P3: a configuration that is no JSON or TOML showed as
/// "not set up", and `--mcp-setup` then tried to write it. It says why it
/// cannot be read and stays as it is; an empty file is a new one.
#[test]
fn a_configuration_that_cannot_be_read_says_why_and_is_left_as_it_is() {
    let home = temporary_home("unreadable");
    let environment = clean_profile(&home);
    let claude = environment.config_file(Harness::ClaudeCode).unwrap();
    let codex = environment.config_file(Harness::Codex).unwrap();
    create_parent(&codex);
    let broken_json = "{\"mcpServers\": {\n";
    let broken_toml = "[mcp_servers.ketchup\ncommand = 'x'\n";
    std::fs::write(&claude, broken_json).unwrap();
    std::fs::write(&codex, broken_toml).unwrap();
    let this = std::env::current_exe().unwrap();
    let rows = environment.set_up_installed(&this);
    for harness in [Harness::ClaudeCode, Harness::Codex] {
        let row = rows.iter().find(|row| row.harness == harness).unwrap();
        let Registration::Unreadable { reason } = &row.registration else {
            panic!("{row:?}");
        };
        assert!(reason.to_string().contains("line"), "{reason}");
        assert_eq!(row.setup_error, None, "nothing was written");
        assert!(describe(row, &this).contains(&reason.to_string()));
    }
    assert_eq!(std::fs::read_to_string(&claude).unwrap(), broken_json);
    assert_eq!(std::fs::read_to_string(&codex).unwrap(), broken_toml);
    for file in [&claude, &codex] {
        std::fs::write(file, "").unwrap();
    }
    for harness in [Harness::ClaudeCode, Harness::Codex] {
        assert_eq!(
            environment.registration(harness, &this),
            Registration::NotSetUp
        );
    }
    std::fs::remove_dir_all(home).unwrap();
}

/// Review 2026-11-04 P3: when the system did not list the running programs
/// the Supervisor instances were silently none. A row says they could not be
/// listed and why.
#[test]
fn running_supervisors_that_could_not_be_listed_get_a_row_saying_why() {
    let home = temporary_home("unlisted");
    let environment = Environment {
        supervisor_folders: Err(std::io::Error::other("snapshot refused").into()),
        ..clean_profile(&home)
    };
    let this = std::env::current_exe().unwrap();
    let rows = environment.rows(&this);
    let unlisted: Vec<_> = rows
        .iter()
        .filter(|row| matches!(row.registration, Registration::InstancesUnlisted { .. }))
        .collect();
    assert_eq!(unlisted.len(), 1, "{rows:?}");
    assert_eq!(unlisted[0].harness, Harness::Supervisor);
    assert_eq!(
        unlisted[0].registration,
        Registration::InstancesUnlisted {
            reason: std::io::Error::other("snapshot refused").into()
        }
    );
    std::fs::remove_dir_all(home).unwrap();
}

/// The installed Supervisor of 9. 10. starts
/// `%LOCALAPPDATA%\Programs\Supervisor\Kecup\ketchup-app.exe`, and its
/// `mcp_bridge.py` expands the variables (`os.path.expandvars`): the row names
/// the program that exists, not a missing one. Cargo names the package's
/// folder to the tests it runs, so the program here is its `Cargo.toml`.
#[test]
fn a_supervisor_command_with_a_windows_variable_names_the_program_it_starts() {
    let home = temporary_home("supervisor-variable");
    let environment = clean_profile(&home);
    let file = environment.config_file(Harness::Supervisor).unwrap();
    create_parent(&file);
    let command = Path::new("%CARGO_MANIFEST_DIR%").join("Cargo.toml");
    let servers = serde_json::json!({"mcpServers": {"ketchup": {"command": command}}});
    std::fs::write(&file, servers.to_string()).unwrap();
    let program = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let this = std::env::current_exe().unwrap();
    assert_eq!(
        environment.registration(Harness::Supervisor, &this),
        Registration::Elsewhere {
            command: program.to_string_lossy().into_owned(),
            exists: true
        }
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn variables_are_expanded_as_supervisor_does_and_an_unknown_one_stays() {
    // Cargo names the package to the tests it runs.
    assert_eq!(
        expand_variables(r"a\%CARGO_PKG_NAME%\b"),
        r"a\ketchup-app\b"
    );
    assert_eq!(
        expand_variables("%KETCHUP_NO_SUCH_VARIABLE%\\x"),
        "%KETCHUP_NO_SUCH_VARIABLE%\\x"
    );
    assert_eq!(expand_variables("100%% and 5%"), "100% and 5%");
}

/// Review 2026-11-04 P3: Supervisor's `os.path.expandvars` on Windows also
/// reads `$NAME` and `${NAME}` and leaves single quoted text alone. Each
/// expected value is what Python 3.11's `ntpath.expandvars` gives.
#[test]
fn dollar_variables_and_quotes_are_read_as_supervisor_reads_them() {
    for (text, expanded) in [
        (r"a\$CARGO_PKG_NAME\b", r"a\ketchup-app\b"),
        (r"a\${CARGO_PKG_NAME}\b", r"a\ketchup-app\b"),
        (r"$KETCHUP_NO_SUCH\x", r"$KETCHUP_NO_SUCH\x"),
        ("${KETCHUP_NO_SUCH}", "${KETCHUP_NO_SUCH}"),
        (
            "'$CARGO_PKG_NAME' %CARGO_PKG_NAME%",
            "'$CARGO_PKG_NAME' ketchup-app",
        ),
        ("it's '$CARGO_PKG_NAME' here", "it's 'ketchup-app' here"),
        ("don't $CARGO_PKG_NAME", "don't $CARGO_PKG_NAME"),
        ("$$CARGO_PKG_NAME", "$CARGO_PKG_NAME"),
        ("${CARGO_PKG_NAME", "${CARGO_PKG_NAME"),
        ("$ alone", "$ alone"),
        ("%CARGO_PKG_NAME", "%CARGO_PKG_NAME"),
        (
            "$CARGO_PKG_NAME-$CARGO_PKG_NAME.exe",
            "$CARGO_PKG_NAME-ketchup-app.exe",
        ),
        ("%=x% $", "%=x% $"),
    ] {
        assert_eq!(expand_variables(text), expanded, "{text}");
    }
}

/// Capability review run 432: a Supervisor reads `skills\mcp_servers.json` of
/// its own instance folder (its `load_skills(cwd)`, and `mcp_bridge.py` takes
/// the file next to itself), so the dev checkout and every project's instance
/// have their own file besides the installed copy.
#[test]
fn every_supervisor_instance_is_a_row_and_connect_writes_only_its_file() {
    let home = temporary_home("supervisor-instances");
    let mut environment = clean_profile(&home);
    let installed = environment.config_file(Harness::Supervisor).unwrap();
    create_parent(&installed);
    std::fs::write(&installed, r#"{"mcpServers": {}}"#).unwrap();
    let dev = home.join("Sources").join("Supervisor");
    let reviewer = home.join("Sources").join("Ketchup").join("Dohlad");
    for folder in [&dev, &reviewer] {
        std::fs::create_dir_all(folder.join("skills")).unwrap();
        std::fs::write(folder.join("skills").join("mcp_bridge.py"), "").unwrap();
    }
    let old = home.join("old").join("Ketchup.exe");
    let reviewer_file = reviewer.join("skills").join("mcp_servers.json");
    let servers = serde_json::json!({"mcpServers": {"ketchup": {"command": old}}});
    std::fs::write(&reviewer_file, servers.to_string()).unwrap();
    // An instance without the MCP bridge reads no MCP servers.
    let without_bridge = home.join("Sources").join("Django");
    std::fs::create_dir_all(without_bridge.join("skills")).unwrap();
    // The installed copy, also found running, is listed once.
    let installed_folder = installed.parent().unwrap().parent().unwrap().to_owned();
    std::fs::write(installed.with_file_name("mcp_bridge.py"), "").unwrap();
    environment.supervisor_folders = Ok(vec![
        dev.clone(),
        installed_folder,
        reviewer.clone(),
        without_bridge,
        dev.clone(),
    ]);
    let this = std::env::current_exe().unwrap();

    let rows: Vec<HarnessRow> = environment
        .rows(&this)
        .into_iter()
        .filter(|row| row.harness == Harness::Supervisor)
        .collect();
    let dev_file = dev.join("skills").join("mcp_servers.json");
    let files: Vec<_> = rows.iter().map(|row| row.file.clone().unwrap()).collect();
    assert_eq!(
        files,
        [installed.clone(), dev_file.clone(), reviewer_file.clone()]
    );
    assert_eq!(rows[1].registration, Registration::NotSetUp);
    assert_eq!(
        rows[2].registration,
        Registration::Elsewhere {
            command: old.to_string_lossy().into_owned(),
            exists: false
        }
    );

    let connected = environment.set_up_in(Harness::Supervisor, rows[1].file.clone(), &this);
    assert_eq!(connected.registration, Registration::ThisKetchup);
    assert_eq!(connected.file.as_ref(), Some(&dev_file));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&dev_file).unwrap()).unwrap();
    assert_eq!(config["mcpServers"]["ketchup"]["args"][0], "--mcp");
    assert_eq!(
        std::fs::read_to_string(&installed).unwrap(),
        r#"{"mcpServers": {}}"#
    );
    assert_eq!(
        std::fs::read_to_string(&reviewer_file).unwrap(),
        servers.to_string()
    );
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

/// Review 2026-11-04 SEC-142: a project's own `ketchup` entry wins over the
/// user's in that project, but the row read only the user's; and the hidden
/// `--mcp-setup` of the installer replaced a registration that starts another
/// program that works.
#[test]
fn a_project_entry_counts_and_mcp_setup_keeps_a_working_registration() {
    let home = temporary_home("project-scope");
    let environment = Environment::in_home(home.clone());
    let this = std::env::current_exe().unwrap();
    let gone = home.join("target").join("debug").join("ketchup-app.exe");
    let file = home.join(".claude.json");
    let config = serde_json::json!({
        "mcpServers": {"ketchup": {"command": this, "args": ["--mcp"]}},
        "projects": {"C:/work": {"mcpServers": {"ketchup": {"command": gone, "args": ["--mcp"]}}}}
    });
    std::fs::write(&file, config.to_string()).unwrap();
    assert_eq!(
        environment.registration(Harness::ClaudeCode, &this),
        Registration::Elsewhere {
            command: gone.to_string_lossy().into_owned(),
            exists: false
        }
    );
    let row = environment.set_up(Harness::ClaudeCode, &this);
    assert_eq!(row.registration, Registration::ThisKetchup);

    // Another program that exists stays as the user set it up.
    let other = home.join("Other").join("Ketchup.exe");
    create_parent(&other);
    std::fs::write(&other, "").unwrap();
    let working = serde_json::json!({
        "mcpServers": {"ketchup": {"command": other, "args": ["--mcp"]}}
    })
    .to_string();
    std::fs::write(&file, &working).unwrap();
    let rows = environment.set_up_installed(&this);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), working);
    let claude = rows
        .iter()
        .find(|row| row.harness == Harness::ClaudeCode)
        .unwrap();
    assert!(
        matches!(
            &claude.registration,
            Registration::Elsewhere { exists: true, .. }
        ),
        "{claude:?}"
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
    // Every client gets a row, also the ones not on this computer.
    assert_eq!(
        rows.iter().map(|row| row.harness).collect::<Vec<_>>(),
        [
            Harness::ClaudeCode,
            Harness::Codex,
            Harness::ClaudeDesktop,
            Harness::Supervisor
        ]
    );
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
