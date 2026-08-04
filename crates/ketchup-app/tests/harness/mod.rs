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
use egui_kittest::kittest::Queryable as _;
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{AppCommand, KetchupApp};
use ketchup_interaction::LocaleCatalog;

use eframe::egui::{self, Key, Modifiers, Pos2, Rect, Vec2, accesskit::Role};

const SCREEN: Vec2 = Vec2::new(1600.0, 1000.0);

/// A running instance of the shell, driven entirely in memory.
pub struct Shell {
    harness: Harness<'static, KetchupApp>,
    /// Rectangle of the menu button opened last, used to tell a command in the
    /// open menu apart from an identically named control elsewhere in the shell.
    open_menu: Option<Rect>,
    /// The clock the shell sees. Driven explicitly so that two clicks pair up
    /// into a double click only where a test asks for one.
    clock: f64,
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

    /// Start a shell whose file dialogs answer from `dialogs` instead of the
    /// operating system, so the File workflow can be replayed offscreen.
    pub fn with_dialogs(dialogs: ScriptedFileDialogs) -> Self {
        let app = KetchupApp::new().with_dialogs(Box::new(dialogs));
        let mut harness = Harness::builder()
            .with_size(SCREEN)
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
        self.harness.run();
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

    /// Whether `command` is currently offered anywhere in the shell.
    pub fn offers(&self, command: AppCommand) -> bool {
        let label = self.app().command_label(command);
        self.harness
            .query_all_by_role_and_label(Role::Button, &label)
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
        candidates
            .first()
            .expect("the open menu must contain the command")
            .1
            .click();
        self.open_menu = None;
        self.harness.run();
    }

    /// Open a menu of the menu bar, identified by its localization key.
    pub fn open_menu(&mut self, key: &str) {
        self.gap();
        let label = self.catalog().text(key);
        let node = self.harness.get_by_role_and_label(Role::Button, &label);
        let rect = node.rect();
        node.click();
        self.open_menu = Some(rect);
        self.harness.run();
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
        self.harness.run();
    }

    /// Move the synthetic pointer and run a frame.
    pub fn move_pointer(&mut self, position: Pos2) {
        self.event(egui::Event::PointerMoved(position));
    }

    /// Press, release, and settle a primary click at `position`.
    ///
    /// Used for the viewport, which is a painted surface rather than a widget
    /// and therefore has no accessible node to address.
    pub fn click_at(&mut self, position: Pos2) {
        self.click_at_with(position, Modifiers::NONE);
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
        self.harness.run();
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
        self.harness.run();
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
        self.harness.run();
    }

    /// Send a key press and release with the given modifiers held.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.harness.key_press_modifiers(modifiers, key);
        self.harness.run();
    }

    /// Send the native event emitted by egui-winit for Ctrl+C.
    pub fn native_copy(&mut self) {
        self.event(egui::Event::Copy);
        self.harness.run();
    }

    /// Send the native event emitted by egui-winit for Ctrl+V.
    pub fn native_paste(&mut self) {
        self.event(egui::Event::Paste("Ketchup object selection".to_owned()));
        self.harness.run();
    }

    /// Send a key press and release with no modifiers.
    pub fn press_key(&mut self, key: Key) {
        self.harness.key_press(key);
        self.harness.run();
    }

    /// Type text into whatever widget currently has keyboard focus.
    pub fn type_text(&mut self, text: &str) {
        self.event(egui::Event::Text(text.to_owned()));
        self.harness.run();
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
