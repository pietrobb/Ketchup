//! The Plugins menu: the plugin folders under `%APPDATA%\Ketchup\plugins`,
//! read when the menu first opens or on Refresh, never watched in the
//! background. A folder that could not run is listed with the reason a run
//! would give. A run from the menu is the MCP `run_plugin` request on a bridge
//! without a socket: the same consent and folder hash, the same job, one Undo
//! step. Its inputs come from a form built from the manifest's declarations
//! and are checked by the run, as for MCP.
use super::plugin::{PluginAnswer, RunPluginRequest, default_plugin_timeout_ms};
use super::plugin_manifest::{self, Description, InputType};
use super::window_request::{Polled, WindowRequest};
use super::*;
use std::fs;

/// `%APPDATA%\Ketchup\plugins`.
fn default_root() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA");
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME");
    base.map(PathBuf::from)
        .filter(|base| base.is_absolute())
        .map(|base| base.join("Ketchup").join("plugins"))
}

/// `code: reason — cause …` of a recorded rejection.
pub(super) fn rejection_text(rejection: &Value) -> String {
    let mut text = format!(
        "{}: {}",
        rejection["code"].as_str().unwrap_or_default(),
        rejection["reason"].as_str().unwrap_or_default()
    );
    let causes = rejection["causes"].as_array().into_iter().flatten();
    for cause in causes.filter_map(Value::as_str) {
        text.push_str(" — ");
        text.push_str(cause);
    }
    text
}

/// The rejection `code` just recorded, as MCP gets it.
fn recorded(code: &str) -> Value {
    rejection_result(code, take_error_details())
}

fn no_root() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "no per-user application data folder (APPDATA)",
    )
}

/// One plugin folder as the menu lists it.
pub(crate) struct ListedPlugin {
    folder: PathBuf,
    /// Its name and inputs, or the rejection a run would get.
    entry: Result<Description, Value>,
}

fn scan(root: &Path) -> io::Result<Vec<ListedPlugin>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut listed = Vec::new();
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        // A loose file is no plugin; a link is listed and refused like any folder.
        if !kind.is_dir() && !kind.is_symlink() {
            continue;
        }
        take_error_details();
        let folder = entry.path();
        let entry = plugin_manifest::describe(&folder).map_err(recorded);
        listed.push(ListedPlugin { folder, entry });
    }
    listed.sort_by(|a, b| a.folder.cmp(&b.folder));
    Ok(listed)
}

struct FormField {
    name: String,
    kind: InputType,
    text: String,
    checked: bool,
}

impl FormField {
    /// The typed value; text that is no number goes as text, so the run
    /// refuses it with the manifest's type. An empty number is left out.
    fn value(&self) -> Option<Value> {
        let text = self.text.trim();
        let number = text.replace(',', ".");
        match self.kind {
            InputType::Boolean => Some(Value::Bool(self.checked)),
            InputType::String => Some(Value::String(self.text.clone())),
            _ if text.is_empty() => None,
            InputType::Integer => Some(
                number
                    .parse::<i64>()
                    .map_or_else(|_| json!(text), Value::from),
            ),
            InputType::Number => Some(
                number
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map_or_else(|| json!(text), Value::Number),
            ),
        }
    }
}

struct InputForm {
    folder: PathBuf,
    name: String,
    fields: Vec<FormField>,
}

/// What the last plugin run or folder action ended with.
struct Outcome {
    title: String,
    text: String,
}

pub(crate) struct PluginMenu {
    root: Option<PathBuf>,
    listed: Option<io::Result<Vec<ListedPlugin>>>,
    form: Option<InputForm>,
    /// A plugin started from the menu, by name.
    run: Option<WindowRequest<String>>,
    outcome: Option<Outcome>,
}

impl PluginMenu {
    pub(crate) fn new() -> Self {
        Self::at(default_root())
    }

    pub(crate) fn at(root: Option<PathBuf>) -> Self {
        Self {
            root,
            listed: None,
            form: None,
            run: None,
            outcome: None,
        }
    }

    pub(crate) const fn is_running(&self) -> bool {
        self.run.is_some()
    }

    fn refresh(&mut self) {
        self.listed = Some(match &self.root {
            Some(root) => scan(root),
            None => Err(no_root()),
        });
    }
}

impl KetchupApp {
    pub(crate) fn show_plugins_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(480.0);
        if self.live.plugin_menu.listed.is_none() {
            self.live.plugin_menu.refresh();
        }
        let menu = &self.live.plugin_menu;
        let mut chosen = None;
        if let Some(run) = &menu.run {
            ui.label(self.catalog.format(
                "plugins-running",
                &BTreeMap::from([("name", run.action.clone())]),
            ));
        }
        match menu.listed.as_ref() {
            Some(Ok(listed)) if listed.is_empty() => {
                let folder = menu.root.as_deref().unwrap_or(Path::new("")).display();
                ui.weak(self.catalog.format(
                    "plugins-none",
                    &BTreeMap::from([("folder", folder.to_string())]),
                ));
            }
            Some(Ok(listed)) => {
                for (index, plugin) in listed.iter().enumerate() {
                    match &plugin.entry {
                        Ok(description) => {
                            let button = egui::Button::new(&description.name);
                            if ui
                                .add_enabled(menu.run.is_none(), button)
                                .on_hover_text(plugin.folder.display().to_string())
                                .clicked()
                            {
                                chosen = Some(index);
                                ui.close();
                            }
                        }
                        Err(reason) => {
                            let name = plugin.folder.file_name().unwrap_or_default();
                            let label = self.catalog.format(
                                "plugins-unavailable",
                                &BTreeMap::from([("name", name.to_string_lossy().into_owned())]),
                            );
                            ui.add_enabled(false, egui::Button::new(label));
                            let reason = rejection_text(reason);
                            ui.add(
                                egui::Label::new(egui::RichText::new(reason).weak().small()).wrap(),
                            );
                        }
                    }
                }
            }
            Some(Err(error)) => {
                let root = menu.root.as_deref().unwrap_or(Path::new("")).display();
                let reason = format!("{root}: {error}");
                ui.add(egui::Label::new(egui::RichText::new(reason).weak()).wrap());
            }
            None => {}
        }
        ui.separator();
        if ui
            .button(self.catalog.text("plugins-open-folder"))
            .clicked()
        {
            self.open_plugins_folder();
            ui.close();
        }
        if ui.button(self.catalog.text("plugins-refresh")).clicked() {
            self.live.plugin_menu.refresh();
        }
        if let Some(index) = chosen {
            self.choose_plugin(ui.ctx(), index);
        }
    }

    fn open_plugins_folder(&mut self) {
        let opened = match self.live.plugin_menu.root.clone() {
            Some(root) => fs::create_dir_all(&root).and_then(|()| self.dialogs.show_folder(&root)),
            None => Err(no_root()),
        };
        if let Err(error) = opened {
            self.live.plugin_menu.outcome = Some(Outcome {
                title: self.catalog.text("plugins-open-folder"),
                text: error.to_string(),
            });
        }
    }

    /// A plugin without inputs runs at once; one with inputs asks for them.
    fn choose_plugin(&mut self, context: &egui::Context, index: usize) {
        let Some(Ok(listed)) = &self.live.plugin_menu.listed else {
            return;
        };
        let Some(ListedPlugin {
            folder,
            entry: Ok(description),
        }) = listed.get(index)
        else {
            return;
        };
        let fields = description
            .inputs
            .iter()
            .map(|input| FormField {
                name: input.name.clone(),
                kind: input.kind,
                text: match &input.default {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Bool(_)) | None => String::new(),
                    Some(value) => value.to_string(),
                },
                checked: input
                    .default
                    .as_ref()
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
            .collect::<Vec<_>>();
        let form = InputForm {
            folder: folder.clone(),
            name: description.name.clone(),
            fields,
        };
        self.live.plugin_menu.outcome = None;
        if form.fields.is_empty() {
            self.start_window_plugin(context, form);
        } else {
            self.live.plugin_menu.form = Some(form);
        }
    }

    fn start_window_plugin(&mut self, context: &egui::Context, form: InputForm) {
        let request = RunPluginRequest {
            plugin: Some(form.folder.to_string_lossy().into_owned()),
            inputs: Some(
                form.fields
                    .iter()
                    .filter_map(|field| Some((field.name.clone(), field.value()?)))
                    .collect(),
            ),
            timeout_ms: default_plugin_timeout_ms(),
            ..RunPluginRequest::default()
        };
        let run = WindowRequest::start(self, context, form.name, |runner, app, queued| {
            let answer = PluginAnswer {
                id: queued.id,
                reply: queued.reply,
                cancelled: queued.cancelled,
            };
            runner.start_plugin(app, context, answer, request, false);
        });
        self.live.plugin_menu.run = Some(run);
    }

    /// Drives a plugin run from the menu; taken out of the app meanwhile, so
    /// it is not a busy reason for its own publication.
    pub(crate) fn poll_window_plugin(&mut self, context: &egui::Context) {
        let Some(run) = self.live.plugin_menu.run.take() else {
            return;
        };
        let (title, text) = match run.poll(self, context) {
            Polled::Running(run) => {
                self.live.plugin_menu.run = Some(run);
                return;
            }
            Polled::Answered(name, Ok(result)) => {
                let published = result["published"] == true;
                let key = if published {
                    "plugins-published"
                } else {
                    "plugins-unchanged"
                };
                (name, self.catalog.text(key))
            }
            Polled::Answered(name, Err(refusal)) => (name, rejection_text(&refusal)),
            Polled::Disconnected(name) => (
                name,
                rejection_text(&rejection_result(
                    "apply_and_verify_worker_disconnected",
                    None,
                )),
            ),
        };
        self.live.plugin_menu.outcome = Some(Outcome { title, text });
    }

    /// The input form, the running plugin or how the last run ended.
    pub(crate) fn show_plugin_window(&mut self, context: &egui::Context) {
        let menu = &mut self.live.plugin_menu;
        let title = match (&menu.run, &menu.form, &menu.outcome) {
            (Some(run), _, _) => run.action.clone(),
            (None, Some(form), _) => form.name.clone(),
            (None, None, Some(outcome)) => outcome.title.clone(),
            (None, None, None) => return,
        };
        let catalog = &self.catalog;
        let (mut run, mut close) = (false, false);
        egui::Window::new(title)
            .id(egui::Id::new("plugin-window"))
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                if let Some(running) = &menu.run {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(catalog.format(
                            "plugins-running",
                            &BTreeMap::from([("name", running.action.clone())]),
                        ));
                    });
                    return;
                }
                if let Some(form) = &mut menu.form {
                    egui::Grid::new("plugin-inputs").show(ui, |ui| {
                        for field in &mut form.fields {
                            let label = ui.label(&field.name);
                            if field.kind == InputType::Boolean {
                                ui.checkbox(&mut field.checked, "").labelled_by(label.id);
                            } else {
                                ui.text_edit_singleline(&mut field.text)
                                    .labelled_by(label.id);
                            }
                            ui.end_row();
                        }
                    });
                    ui.horizontal(|ui| {
                        run = ui.button(catalog.text("plugins-run")).clicked();
                        close = ui.button(catalog.text("plugins-cancel")).clicked();
                    });
                    return;
                }
                if let Some(outcome) = &menu.outcome {
                    ui.add(egui::Label::new(&outcome.text).wrap());
                    close = ui.button(catalog.text("plugins-close")).clicked();
                }
            });
        if close {
            menu.form = None;
            menu.outcome = None;
        }
        if run && let Some(form) = menu.form.take() {
            self.start_window_plugin(context, form);
        }
    }
}
