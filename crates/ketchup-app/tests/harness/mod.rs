//! Offscreen driver for the designed shell.
//!
//! [`Shell`] runs the real `KetchupApp` user interface without a window, a
//! renderer, or the operating system pointer, on top of `egui_kittest`. Widgets
//! are addressed through the AccessKit tree, so a query resolves the same
//! rectangle the user would click, sees controls that are scrolled out of view,
//! and never depends on which glyph or translation happens to be painted.
//!
//! Two rules keep the workflows honest:
//!
//! * address commands by [`AppCommand`], never by a literal string — the
//!   expected label is resolved through the shell's own [`LocaleCatalog`];
//! * assert on document state (revision, canonical digest, occurrence counts),
//!   never on painted text — text is a fragile proxy for state we hold directly.

#![allow(dead_code)]

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{AppCommand, KetchupApp};
use ketchup_assistant::sidecar::{
    AssistantCadDeletePolicy, AssistantCadEditOperation, AssistantCadEditProgram,
    AssistantCadEntitySelector, AssistantCadPartFeature, AssistantCadRotation,
    AssistantPrincipalPlane, AssistantSketchEntity, AssistantStructureChange,
    AssistantWorkplaneSpec,
};
use ketchup_interaction::{LocaleCatalog, Vec3};

use crate::integration_support::{FileTurn, file_turn};

use eframe::egui::{self, Key, Modifiers, Pos2, Rect, Vec2, accesskit::Role};

const SCREEN: Vec2 = Vec2::new(1600.0, 1000.0);
/// Longest [`Shell::wait_until`] waits; generous because the whole workspace's tests share
/// the machine with exact workers.
pub const WAIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
/// Pause between polls of [`Shell::wait_until`].
const WAIT_POLL: std::time::Duration = std::time::Duration::from_millis(5);

/// Polls `done` until it holds or `within` elapses; false on timeout. Tests that drive
/// their own `Harness` (frame timing of real documents) wait here instead of sleeping.
pub fn poll_until(within: std::time::Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + within;
    loop {
        if done() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(WAIT_POLL);
    }
}

/// One extruded rectangular part: `size_mm` is plan width, plan depth and
/// height; `origin_mm` is its minimum corner.
pub struct BoxPart<'a> {
    pub name: &'a str,
    pub size_mm: [f64; 3],
    pub origin_mm: [f64; 3],
}

/// Replace every occurrence with the given boxes as one typed CAD program, the
/// way an MCP client builds a scene (one revision, one undo step).
pub fn replace_scene_with_boxes(app: &mut KetchupApp, boxes: &[BoxPart<'_>]) {
    let existing = app
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.id().0)
        .collect::<Vec<_>>();
    let mut operations = vec![AssistantCadEditOperation::Delete {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: existing,
        },
        dependency_policy: AssistantCadDeletePolicy::RemoveReferences,
    }];
    operations.extend(boxes.iter().map(|part| {
        let [width, depth, height] = part.size_mm;
        let corners = [[0.0, 0.0], [width, 0.0], [width, depth], [0.0, depth]];
        AssistantCadEditOperation::CreatePart {
            holes: Vec::new(),
            pockets: Vec::new(),
            name: part.name.to_owned(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: (0..4)
                .map(|index| AssistantSketchEntity::Line {
                    id: index as u64 + 1,
                    start_mm: corners[index],
                    end_mm: corners[(index + 1) % 4],
                })
                .collect(),
            constraints: Vec::new(),
            feature: AssistantCadPartFeature::Extrusion {
                distance_mm: height,
            },
            translation_mm: part.origin_mm,
            rotation: None,
        }
    }));
    app.headless_apply_cad_program(&AssistantCadEditProgram { operations })
        .unwrap();
}

/// Move one occurrence by `delta_mm` as one typed CAD program.
pub fn translate_occurrence(app: &mut KetchupApp, occurrence_id: u64, delta_mm: [f64; 3]) {
    let operations = vec![AssistantCadEditOperation::Transform {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![occurrence_id],
        },
        translation_mm: delta_mm,
        rotation: None,
    }];
    app.headless_apply_cad_program(&AssistantCadEditProgram { operations })
        .unwrap();
}

/// Rotate one occurrence about a world axis through `pivot_mm` as one typed CAD program.
pub fn rotate_occurrence(
    app: &mut KetchupApp,
    occurrence_id: u64,
    pivot_mm: [f64; 3],
    axis: [f64; 3],
    angle_degrees: f64,
) {
    let operations = vec![AssistantCadEditOperation::Transform {
        selector: AssistantCadEntitySelector::Occurrences {
            occurrence_ids: vec![occurrence_id],
        },
        translation_mm: [0.0; 3],
        rotation: Some(AssistantCadRotation {
            pivot_mm,
            axis,
            angle_degrees,
        }),
    }];
    app.headless_apply_cad_program(&AssistantCadEditProgram { operations })
        .unwrap();
}

/// Commits one scene-tree change as a typed `structure` program, the MCP path;
/// false when the document refuses it.
pub fn apply_structure(app: &mut KetchupApp, change: AssistantStructureChange) -> bool {
    app.headless_apply_cad_program(&AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::Structure { change }],
    })
    .is_ok()
}

/// Sets one feature dimension with a typed `set_dimension` program, the MCP
/// path; false when the document refuses it.
pub fn set_feature_dimension(app: &mut KetchupApp, feature_id: u64, value_mm: f64) -> bool {
    app.headless_apply_cad_program(&AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::SetDimension {
            feature_id,
            constraint_id: None,
            value_mm,
        }],
    })
    .is_ok()
}

/// A running instance of the shell, driven entirely in memory.
pub struct Shell {
    harness: Harness<'static, KetchupApp>,
    /// Rectangle of the menu button opened last, used to tell a command in the
    /// open menu apart from an identically named control elsewhere in the shell.
    open_menu: Option<Rect>,
    /// The clock the shell sees. Driven explicitly so that two clicks pair up
    /// into a double click only where a test asks for one.
    clock: f64,
    /// Declared last so it is released only after the harness and its workers.
    _turn: FileTurn,
}

/// Long enough that egui reads the next click as a new one, not as the second
/// half of a double click.
const GESTURE_GAP: f64 = 1.0;
/// Short enough that two clicks separated by it read as a double click.
const CLICK_STEP: f64 = 0.02;

impl Shell {
    /// Start a shell with native dialogs stubbed out by a cancelling script.
    pub fn new() -> Self {
        Self::with_dialogs(ScriptedFileDialogs::new())
    }

    /// Start a shell with a complete injected locale catalog.
    pub fn with_catalog(catalog: LocaleCatalog) -> Self {
        Self::with_catalog_and_dialogs(catalog, ScriptedFileDialogs::new())
    }

    pub fn with_catalog_and_dialogs(catalog: LocaleCatalog, dialogs: ScriptedFileDialogs) -> Self {
        Self::build(KetchupApp::with_catalog(catalog).with_dialogs(Box::new(dialogs)))
    }

    /// Start a shell whose file dialogs answer from `dialogs` instead of the
    /// operating system, so the File workflow can be replayed offscreen.
    pub fn with_dialogs(dialogs: ScriptedFileDialogs) -> Self {
        Self::build(KetchupApp::new().with_dialogs(Box::new(dialogs)))
    }

    pub fn with_dialogs_at_size(dialogs: ScriptedFileDialogs, size: Vec2) -> Self {
        Self::build_at_size(KetchupApp::new().with_dialogs(Box::new(dialogs)), size)
    }

    fn build(app: KetchupApp) -> Self {
        Self::build_at_size(app, SCREEN)
    }

    fn build_at_size(mut app: KetchupApp, size: Vec2) -> Self {
        let turn = file_turn();
        // The product hides the manual CAD panels; most shell tests drive them.
        app.set_manual_cad_panels_visible(true);
        let mut harness = Harness::builder()
            .with_size(size)
            // Exact workers can legitimately request more than the default four
            // frames before publishing, but the replay must remain bounded.
            .with_max_steps(64)
            // The default frame step is a quarter of a second, which is longer
            // than the double-click delay — two clicks would never pair up.
            .with_step_dt(1.0 / 60.0)
            .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
        // Without this, collapsing headers, tooltips and fade-ins need a
        // variable number of frames and every assertion becomes a race.
        harness.ctx.style_mut(|style| {
            style.animation_time = 0.0;
        });
        harness.run();
        let mut shell = Self {
            harness,
            open_menu: None,
            clock: 0.0,
            _turn: turn,
        };
        shell.gap();
        shell
    }

    /// Advance the shell clock so the next gesture stands on its own.
    fn gap(&mut self) {
        self.advance(GESTURE_GAP);
    }

    fn advance(&mut self, seconds: f64) {
        self.clock += seconds;
        self.harness.input_mut().time = Some(self.clock);
    }

    /// The shell state under test.
    pub fn app(&self) -> &KetchupApp {
        self.harness.state()
    }

    /// Mutable shell state, for arranging a scenario before replaying input.
    pub fn app_mut(&mut self) -> &mut KetchupApp {
        self.harness.state_mut()
    }

    /// The localization catalog the shell paints with.
    pub fn catalog(&self) -> &LocaleCatalog {
        self.app().catalog()
    }

    /// Run frames until the shell stops asking for a repaint.
    pub fn settle(&mut self) {
        self.run_idle();
    }

    /// Like `Harness::run`, but a shell still repainting for background work (an exact
    /// preview being evaluated on a loaded machine) gets until [`WAIT_DEADLINE`] instead
    /// of a fixed frame count.
    fn run_idle(&mut self) {
        let mut last_error = String::new();
        let idle = self.wait_until(|shell| match shell.harness.try_run() {
            Ok(_) => true,
            Err(error) => {
                last_error = error.to_string();
                false
            }
        });
        assert!(idle, "{last_error}");
    }

    /// Poll `done` (which drives frames with `step` or `settle` and checks the condition)
    /// until it holds, letting background work (exact worker, file I/O) publish
    /// between polls. Returns false after [`WAIT_DEADLINE`] so the caller can report the
    /// state it was waiting on. This is the one place a UI test waits: a slow machine needs
    /// one deadline raised, not a guess in every test.
    pub fn wait_until(&mut self, mut done: impl FnMut(&mut Self) -> bool) -> bool {
        poll_until(WAIT_DEADLINE, || done(self))
    }

    pub fn step(&mut self) {
        self.harness.step();
    }

    pub fn output_shapes(&self) -> &[egui::epaint::ClippedShape] {
        &self.harness.output().shapes
    }

    pub fn initialize_gpu(&mut self) {
        self.harness
            .render()
            .expect("initialize real offscreen wgpu renderer");
    }

    pub fn enable_live_bridge(&mut self) {
        let context = self.harness.ctx.clone();
        self.harness
            .state_mut()
            .enable_live_bridge(&context)
            .expect("enable the headless live bridge");
    }

    pub fn enable_live_consent_broker(&mut self) {
        let context = self.harness.ctx.clone();
        self.harness
            .state_mut()
            .enable_live_consent_broker(&context)
            .expect("enable the headless live consent broker");
    }

    pub fn render_gpu(&mut self) {
        self.harness
            .render()
            .expect("service the isolated CAD image callback");
    }

    pub fn save_render(&mut self, path: &std::path::Path) -> [u32; 2] {
        let image = self
            .harness
            .render()
            .expect("render the complete headless Ketchup window");
        let size = [image.width(), image.height()];
        image.save(path).expect("save the headless render");
        size
    }

    pub fn render_viewport_pixels(&mut self) -> Vec<u8> {
        let rect = self.viewport_rect();
        let image = self
            .harness
            .render()
            .expect("render the complete headless Ketchup window");
        let x_min = rect.min.x.max(0.0).floor() as u32;
        let y_min = rect.min.y.max(0.0).floor() as u32;
        let x_max = rect.max.x.min(image.width() as f32).ceil() as u32;
        let y_max = rect.max.y.min(image.height() as f32).ceil() as u32;
        let mut pixels = Vec::with_capacity(((x_max - x_min) * (y_max - y_min) * 4) as usize);
        for y in y_min..y_max {
            for x in x_min..x_max {
                pixels.extend_from_slice(&image.get_pixel(x, y).0);
            }
        }
        pixels
    }

    pub fn has_paint_callback(&self) -> bool {
        fn contains(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Callback(_) => true,
                egui::Shape::Vec(shapes) => shapes.iter().any(contains),
                _ => false,
            }
        }
        self.harness
            .output()
            .shapes
            .iter()
            .any(|shape| contains(&shape.shape))
    }

    /// The 3D viewport rectangle of the current layout.
    ///
    /// Panics rather than returning a zero rectangle, so a test that interacts
    /// before the first frame fails loudly instead of clicking the origin.
    pub fn viewport_rect(&self) -> Rect {
        self.app()
            .viewport_rect()
            .expect("the viewport has not been laid out yet — run a frame first")
    }

    /// Screen position of the centre of an occurrence's top face.
    ///
    /// Aiming at real geometry instead of at a fixed screen offset keeps a test
    /// valid under both the converging and the parallel projection.
    pub fn top_face_centre(&self, occurrence_id: u64) -> Pos2 {
        let rect = self.viewport_rect();
        let (origin, size) = self
            .app()
            .occurrence_box_geometry(occurrence_id)
            .expect("the occurrence must be part of the active scene");
        self.app().project_to_screen(
            Vec3::new(
                origin.x + size.x * 0.5,
                origin.y + size.y * 0.5,
                origin.z + size.z,
            ),
            rect,
        )
    }

    /// Whether `command` is currently offered anywhere in the shell.
    pub fn offers(&self, command: AppCommand) -> bool {
        let label = self.app().command_label(command);
        self.harness
            .query_all_by_role_and_label(Role::Button, &label)
            .next()
            .is_some()
    }

    /// Rectangle of the top-most visible control that dispatches `command`.
    pub fn command_rect(&self, command: AppCommand) -> Rect {
        let label = self.app().command_label(command);
        self.harness
            .query_all_by_role_and_label(Role::Button, &label)
            .min_by(|left, right| left.rect().top().total_cmp(&right.rect().top()))
            .expect("the visible command control must exist")
            .rect()
    }

    /// Move screen-reader focus to a command through its AccessKit action.
    pub fn focus_command(&mut self, command: AppCommand) {
        let label = self.app().command_label(command);
        self.harness
            .get_by_role_and_label(Role::Button, &label)
            .focus();
        self.run_idle();
    }

    /// Move keyboard focus to a labeled text input through AccessKit.
    pub fn focus_text_input(&mut self, label: &str) {
        self.harness
            .get_by_role_and_label(Role::TextInput, label)
            .focus();
        self.run_idle();
    }

    pub fn focus_text_input_once(&mut self, label: &str) {
        self.harness
            .get_by_role_and_label(Role::TextInput, label)
            .focus();
        self.harness.step();
    }

    pub fn focus_combo_box(&mut self, label: &str) {
        self.harness
            .get_by_role_and_label(Role::ComboBox, label)
            .focus();
        self.run_idle();
    }

    /// Whether a button with `label` is marked selected (a highlighted row).
    pub fn button_is_selected(&self, label: &str) -> bool {
        self.harness
            .query_all_by_role_and_label(Role::Button, label)
            .any(|node| node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
    }

    pub fn has_role_and_label(&self, role: Role, label: &str) -> bool {
        self.harness
            .query_all_by_role_and_label(role, label)
            .next()
            .is_some()
    }

    pub fn click_role_and_label(&mut self, role: Role, label: &str) {
        self.activate_role_and_label(role, label);
    }

    /// Whether the command is the focused node in the current AccessKit tree.
    pub fn command_is_focused(&self, command: AppCommand) -> bool {
        let label = self.app().command_label(command);
        self.harness
            .query_all_by_role_and_label(Role::Button, &label)
            .any(|node| node.is_focused())
    }

    /// Rectangle of a localized menu node in the current AccessKit tree.
    pub fn menu_rect(&self, key: &str) -> Rect {
        let label = self.catalog().text(key);
        self.harness
            .query_all_by_role_and_label(Role::Button, &label)
            .min_by(|left, right| left.rect().top().total_cmp(&right.rect().top()))
            .expect("the localized menu must exist")
            .rect()
    }

    /// Bounding boxes of all visible nodes published to assistive technology.
    /// Nodes wholly outside the window are scrolled out of the dock, not visible.
    pub fn visible_accesskit_rects(&self) -> Vec<(String, Rect)> {
        let window = self.harness.ctx.screen_rect();
        self.harness
            .query_all_by(|node| !node.is_hidden() && node.bounding_box().is_some())
            .filter(|node| node.rect().intersects(window))
            .map(|node| {
                let accesskit = node.accesskit_node();
                (
                    format!(
                        "{:?} label={:?} value={:?}",
                        accesskit.role(),
                        accesskit.label().unwrap_or_default(),
                        accesskit.value().unwrap_or_default()
                    ),
                    node.rect(),
                )
            })
            .collect()
    }

    /// Whether assistive technology can currently see a node with `label`.
    pub fn has_visible_label(&self, label: &str) -> bool {
        self.harness
            .query_all_by(|node| {
                !node.is_hidden()
                    && (node.label().as_deref() == Some(label)
                        || node.value().as_deref() == Some(label))
            })
            .next()
            .is_some()
    }

    /// Click the control that dispatches `command`.
    ///
    /// The tool rail paints glyphs and the menus paint translated names; both
    /// resolve to the same accessible name, so this works for either.
    pub fn click_command(&mut self, command: AppCommand) {
        self.gap();
        let label = self.app().command_label(command);
        let open_menu = self.open_menu;
        let enabled = self.app().command_is_enabled(command);
        let mut candidates: Vec<_> = self
            .harness
            .query_all_by_role_and_label(Role::Button, &label)
            .map(|node| (node.rect(), node))
            .collect();
        assert!(
            !candidates.is_empty(),
            "the shell does not currently offer {command:?} (accessible name {label:?})"
        );
        if candidates.len() > 1 {
            // The same command can legitimately appear twice, e.g. Make Unique
            // in both the Model menu and the Outliner. Prefer the item of the
            // menu the test just opened.
            let menu = open_menu.expect(
                "several controls carry this accessible name — open the menu you mean first",
            );
            candidates.retain(|(rect, _)| rect.top() >= menu.bottom() - 1.0);
            candidates.sort_by(|(a, _), (b, _)| a.top().total_cmp(&b.top()));
        }
        let node = &candidates
            .first()
            .expect("the open menu must contain the command")
            .1;
        if enabled {
            node.click_accesskit();
        } else {
            // Disabled controls expose no accessibility Click action.
            node.click();
        }
        self.open_menu = None;
        self.run_idle();
        if let Some(menu) = open_menu
            && !enabled
            && egui::Popup::is_any_open(&self.harness.ctx)
        {
            // Disabled items do not dismiss menus. Close the menu explicitly
            // before the next gesture, without sending Escape to the CAD tool.
            self.click_at(menu.center());
        }
    }

    /// Activate a node through AccessKit rather than a synthetic pointer.
    ///
    /// The right dock scrolls, so a control can be published to assistive
    /// technology yet lie outside the painted viewport; a pointer click would
    /// then land on nothing. Screen-reader activation is what a keyboard or
    /// AT user does, and it reaches the control wherever it sits.
    pub fn activate_role_and_label(&mut self, role: Role, label: &str) {
        self.gap();
        self.harness
            .get_by_role_and_label(role, label)
            .click_accesskit();
        self.run_idle();
    }

    pub fn click_button_label(&mut self, label: &str) {
        self.gap();
        self.harness
            .get_by_role_and_label(Role::Button, label)
            .click_accesskit();
        self.run_idle();
    }

    /// Open a menu of the menu bar, identified by its localization key.
    pub fn open_menu(&mut self, key: &str) {
        self.gap();
        let label = self.catalog().text(key);
        let node = self.harness.get_by_role_and_label(Role::Button, &label);
        let rect = node.rect();
        node.click();
        self.open_menu = Some(rect);
        // egui lays a new popup out in an invisible sizing pass first; only the
        // second frame paints it where its items take clicks.
        self.harness.step();
        self.harness.step();
    }

    /// Open a menu and then click one of its commands.
    pub fn click_menu_command(&mut self, menu_key: &str, command: AppCommand) {
        self.open_menu(menu_key);
        self.click_command(command);
    }

    /// Click the outliner row whose accessible name is `label`.
    pub fn click_row(&mut self, label: &str) {
        self.gap();
        self.harness
            .get_by_role_and_label(Role::Button, label)
            .click();
        self.run_idle();
    }

    /// Double-click the outliner row whose accessible name is `label`.
    pub fn double_click_row(&mut self, label: &str) {
        let position = self
            .harness
            .get_by_role_and_label(Role::Button, label)
            .rect()
            .center();
        self.double_click_at(position);
    }

    /// Click the outliner row while holding keyboard modifiers.
    pub fn click_row_with(&mut self, label: &str, modifiers: Modifiers) {
        let position = self
            .harness
            .get_by_role_and_label(Role::Button, label)
            .rect()
            .center();
        self.click_at_with(position, modifiers);
    }

    /// Move the synthetic pointer and run a frame.
    pub fn move_pointer(&mut self, position: Pos2) {
        self.event(egui::Event::PointerMoved(position));
    }

    /// Rest the pointer on `position` long enough for its tooltip to open.
    pub fn hover(&mut self, position: Pos2) {
        self.move_pointer(position);
        self.run_idle();
        self.advance(1.0);
        self.run_idle();
    }

    /// Scroll the wheel over `position` by a point delta.
    pub fn scroll_at(&mut self, position: Pos2, delta_y: f32) {
        self.move_pointer(position);
        self.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, delta_y),
            modifiers: Modifiers::NONE,
        });
    }

    /// Press, release, and settle a primary click at `position`.
    ///
    /// Used for the viewport, which is a painted surface rather than a widget
    /// and therefore has no accessible node to address.
    pub fn click_at(&mut self, position: Pos2) {
        self.click_at_with(position, Modifiers::NONE);
    }

    /// Click once without waiting for hover tooltips to stop requesting repaints.
    pub fn click_at_once(&mut self, position: Pos2) {
        self.gap();
        self.move_pointer(position);
        self.button(position, Modifiers::NONE, true);
        self.advance(CLICK_STEP);
        self.button(position, Modifiers::NONE, false);
    }

    /// Click at `position` with `modifiers` held.
    pub fn click_at_with(&mut self, position: Pos2, modifiers: Modifiers) {
        self.gap();
        self.move_pointer(position);
        self.harness.input_mut().modifiers = modifiers;
        self.button(position, modifiers, true);
        self.advance(CLICK_STEP);
        self.button(position, modifiers, false);
        self.harness.input_mut().modifiers = Modifiers::NONE;
        self.run_idle();
    }

    /// Press and release the secondary button at `position` to open a viewport menu.
    pub fn secondary_click_at(&mut self, position: Pos2) {
        self.gap();
        self.move_pointer(position);
        for pressed in [true, false] {
            self.event(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            self.advance(CLICK_STEP);
        }
        self.run_idle();
    }

    /// Press and release twice at `position`, close enough in time to read as a
    /// double click, which is how the shell enters a group or component context.
    pub fn double_click_at(&mut self, position: Pos2) {
        self.gap();
        self.move_pointer(position);
        for _ in 0..2 {
            self.button(position, Modifiers::NONE, true);
            self.advance(CLICK_STEP);
            self.button(position, Modifiers::NONE, false);
            self.advance(CLICK_STEP);
        }
        self.run_idle();
    }

    /// Press or release the synthetic primary button without completing the gesture.
    pub fn set_primary_button(&mut self, position: Pos2, modifiers: Modifiers, pressed: bool) {
        self.harness.input_mut().modifiers = modifiers;
        self.button(position, modifiers, pressed);
    }

    /// Press at `from`, drag through interpolated steps, and release at `to`.
    pub fn drag(&mut self, from: Pos2, to: Pos2) {
        self.drag_with(from, to, Modifiers::NONE);
    }

    /// Drag from `from` to `to` with `modifiers` held for the whole gesture.
    pub fn drag_with(&mut self, from: Pos2, to: Pos2, modifiers: Modifiers) {
        self.gap();
        self.move_pointer(from);
        self.harness.input_mut().modifiers = modifiers;
        self.button(from, modifiers, true);
        for step in 1..=8_u8 {
            let t = f32::from(step) / 8.0;
            self.advance(CLICK_STEP);
            self.event(egui::Event::PointerMoved(from + (to - from) * t));
        }
        self.button(to, modifiers, false);
        self.harness.input_mut().modifiers = Modifiers::NONE;
        self.run_idle();
    }

    /// Drag from `from` to `to`, calling `observe` after every single frame of
    /// the gesture and of the frames that follow the release.
    ///
    /// A gesture that commits a revision can leave one frame painted from state
    /// that the commit invalidated; that frame is gone by the time [`Self::drag`]
    /// returns, so only per-frame observation can see it.
    pub fn drag_observing(&mut self, from: Pos2, to: Pos2, mut observe: impl FnMut(&KetchupApp)) {
        self.gap();
        self.move_pointer(from);
        self.button(from, Modifiers::NONE, true);
        observe(self.app());
        for step in 1..=8_u8 {
            let t = f32::from(step) / 8.0;
            self.advance(CLICK_STEP);
            self.event(egui::Event::PointerMoved(from + (to - from) * t));
            observe(self.app());
        }
        self.button(to, Modifiers::NONE, false);
        observe(self.app());
        for _ in 0..8 {
            self.step();
            observe(self.app());
        }
    }

    /// Orbit the camera by holding the secondary button and moving the pointer.
    ///
    /// Each step is a real frame, which is what the desktop shell paints while
    /// the user drags, so a test can measure interactive camera cost.
    pub fn orbit_drag(&mut self, from: Pos2, step_delta: Vec2, steps: u32) {
        self.gap();
        self.move_pointer(from);
        self.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        let mut position = from;
        for _ in 0..steps {
            self.advance(CLICK_STEP);
            position += step_delta;
            self.event(egui::Event::PointerMoved(position));
        }
        self.event(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
    }

    /// Send a key press and release with the given modifiers held.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.harness.input_mut().modifiers = modifiers;
        for pressed in [true, false] {
            self.event(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
        self.harness.step();
        self.harness.input_mut().modifiers = Modifiers::NONE;
        self.harness.step();
    }

    /// Hold or release keyboard modifiers for one rendered interaction state.
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.harness.input_mut().modifiers = modifiers;
        self.run_idle();
    }

    /// Send the native event emitted by egui-winit for Ctrl+C.
    pub fn native_copy(&mut self) {
        self.event(egui::Event::Copy);
        self.run_idle();
    }

    /// Send the native event emitted by egui-winit for Ctrl+V.
    pub fn native_paste(&mut self) {
        self.native_paste_text("Ketchup object selection");
    }

    /// Ctrl+V while the system clipboard holds `text`.
    pub fn native_paste_text(&mut self, text: &str) {
        self.event(egui::Event::Paste(text.to_owned()));
        self.run_idle();
    }

    /// Send a key press and release with no modifiers.
    pub fn press_key(&mut self, key: Key) {
        self.harness.key_press(key);
        self.harness.step();
    }

    /// Type text into whatever widget currently has keyboard focus.
    pub fn type_text(&mut self, text: &str) {
        self.event(egui::Event::Text(text.to_owned()));
        self.run_idle();
    }

    pub fn type_text_once(&mut self, text: &str) {
        self.event(egui::Event::Text(text.to_owned()));
    }

    fn button(&mut self, position: Pos2, modifiers: Modifiers, pressed: bool) {
        self.event(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }

    fn event(&mut self, event: egui::Event) {
        self.harness.input_mut().events.push(event);
        self.harness.step();
    }
}

/// The `Shift` modifier, which extends a selection.
pub fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::NONE
    }
}

/// The `Alt` modifier used for deliberate pick-through without desktop input.
pub fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Modifiers::NONE
    }
}

/// The `Ctrl`/`Cmd` modifier as the shell's shortcuts expect it.
pub fn ctrl() -> Modifiers {
    Modifiers {
        command: true,
        ctrl: true,
        ..Modifiers::NONE
    }
}

/// `Ctrl+Shift`, as the documented Ungroup shortcut expects it.
pub fn ctrl_shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..ctrl()
    }
}
