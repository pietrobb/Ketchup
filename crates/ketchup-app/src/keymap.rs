//! Every keyboard shortcut of the shell in one table. Pressing a chord runs
//! its command, and menus, tooltips and the Shortcuts window print the same
//! chord, so a shortcut can never be documented without working or work
//! without being documented.

use eframe::egui::{self, Event, Key, KeyboardShortcut, ModifierNames, Modifiers};
use ketchup_interaction::LocaleCatalog;

use super::{AppCommand, Axis};

/// A clipboard request the platform delivers instead of a key press.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClipboardRequest {
    Copy,
    Cut,
    Paste,
}

pub(crate) struct Binding {
    pub(crate) command: AppCommand,
    /// The first chord is the one menus print; the rest do the same.
    pub(crate) chords: &'static [KeyboardShortcut],
    pub(crate) clipboard: Option<ClipboardRequest>,
    /// Also listens while a text field has keyboard focus. Such a chord is
    /// only observed, so the text field still receives the key.
    pub(crate) while_typing: bool,
}

const fn chord(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(modifiers, key)
}

const fn on_canvas(command: AppCommand, chords: &'static [KeyboardShortcut]) -> Binding {
    Binding {
        command,
        chords,
        clipboard: None,
        while_typing: false,
    }
}

const fn everywhere(command: AppCommand, chords: &'static [KeyboardShortcut]) -> Binding {
    Binding {
        command,
        chords,
        clipboard: None,
        while_typing: true,
    }
}

const fn clipboard(
    command: AppCommand,
    chords: &'static [KeyboardShortcut],
    request: ClipboardRequest,
) -> Binding {
    Binding {
        command,
        chords,
        clipboard: Some(request),
        while_typing: false,
    }
}

const CTRL: Modifiers = Modifiers::COMMAND;
const CTRL_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const NONE: Modifiers = Modifiers::NONE;

/// Checked in order and the first pressed chord wins, so a chord with more
/// modifiers comes before the same key with fewer (Save As before Save).
pub(crate) const KEYMAP: &[Binding] = &[
    everywhere(AppCommand::New, &[chord(CTRL, Key::N)]),
    everywhere(AppCommand::Open, &[chord(CTRL, Key::O)]),
    everywhere(AppCommand::SaveAs, &[chord(CTRL_SHIFT, Key::S)]),
    everywhere(AppCommand::Save, &[chord(CTRL, Key::S)]),
    on_canvas(
        AppCommand::Redo,
        &[chord(CTRL, Key::Y), chord(CTRL_SHIFT, Key::Z)],
    ),
    on_canvas(AppCommand::Undo, &[chord(CTRL, Key::Z)]),
    clipboard(
        AppCommand::Copy,
        &[chord(CTRL, Key::C)],
        ClipboardRequest::Copy,
    ),
    clipboard(
        AppCommand::Cut,
        &[chord(CTRL, Key::X)],
        ClipboardRequest::Cut,
    ),
    clipboard(
        AppCommand::Paste,
        &[chord(CTRL, Key::V)],
        ClipboardRequest::Paste,
    ),
    on_canvas(AppCommand::Duplicate, &[chord(CTRL, Key::D)]),
    on_canvas(AppCommand::SelectAll, &[chord(CTRL, Key::A)]),
    on_canvas(AppCommand::InvertSelection, &[chord(CTRL, Key::I)]),
    on_canvas(AppCommand::Ungroup, &[chord(CTRL_SHIFT, Key::G)]),
    on_canvas(AppCommand::Group, &[chord(CTRL, Key::G)]),
    on_canvas(AppCommand::Delete, &[chord(NONE, Key::Delete)]),
    on_canvas(AppCommand::Select, &[chord(NONE, Key::Space)]),
    on_canvas(AppCommand::Line, &[chord(NONE, Key::L)]),
    on_canvas(AppCommand::Rectangle, &[chord(NONE, Key::R)]),
    on_canvas(AppCommand::Circle, &[chord(NONE, Key::C)]),
    on_canvas(AppCommand::Arc, &[chord(NONE, Key::A)]),
    on_canvas(AppCommand::Polygon, &[chord(NONE, Key::N)]),
    on_canvas(AppCommand::Ellipse, &[chord(NONE, Key::E)]),
    on_canvas(AppCommand::Spline, &[chord(NONE, Key::S)]),
    on_canvas(AppCommand::PlanarOffset, &[chord(NONE, Key::F)]),
    on_canvas(AppCommand::PushPull, &[chord(NONE, Key::P)]),
    on_canvas(AppCommand::Move, &[chord(NONE, Key::M)]),
    on_canvas(AppCommand::Rotate, &[chord(NONE, Key::Q)]),
    on_canvas(AppCommand::Mirror, &[chord(NONE, Key::I)]),
    on_canvas(AppCommand::Measure, &[chord(NONE, Key::T)]),
    on_canvas(AppCommand::Orbit, &[chord(NONE, Key::O)]),
    on_canvas(AppCommand::Pan, &[chord(NONE, Key::H)]),
    on_canvas(AppCommand::HomeView, &[chord(NONE, Key::Home)]),
    on_canvas(AppCommand::ZoomFit, &[chord(Modifiers::SHIFT, Key::Z)]),
    on_canvas(
        AppCommand::ZoomIn,
        &[chord(CTRL, Key::Plus), chord(CTRL, Key::Equals)],
    ),
    on_canvas(AppCommand::ZoomOut, &[chord(CTRL, Key::Minus)]),
    everywhere(AppCommand::Shortcuts, &[chord(NONE, Key::F1)]),
    everywhere(AppCommand::CommandSearch, &[chord(CTRL, Key::K)]),
    everywhere(AppCommand::Deselect, &[chord(NONE, Key::Escape)]),
];

/// Confirms the shown preview.
pub(crate) const CONFIRM: Key = Key::Enter;
/// Picks the next of several overlapping objects under the pointer.
pub(crate) const CYCLE_OVERLAP: KeyboardShortcut = chord(NONE, Key::Tab);
/// Pins a drawing or transform direction to a coloured axis; the last one
/// releases the lock.
pub(crate) const AXIS_LOCKS: [(Key, Option<Axis>); 4] = [
    (Key::ArrowRight, Some(Axis::X)),
    (Key::ArrowLeft, Some(Axis::Y)),
    (Key::ArrowUp, Some(Axis::Z)),
    (Key::ArrowDown, None),
];

pub(crate) fn binding(command: AppCommand) -> Option<&'static Binding> {
    KEYMAP.iter().find(|binding| binding.command == command)
}

/// The first bound command pressed this frame. `typing` is whether a text
/// field has keyboard focus; then only [`Binding::while_typing`] chords
/// listen and they leave the key to the text field.
pub(crate) fn pressed(input: &mut egui::InputState, typing: bool) -> Option<AppCommand> {
    KEYMAP
        .iter()
        .filter(|binding| !typing || binding.while_typing)
        .find(|binding| {
            let requested = binding.clipboard.is_some_and(|request| {
                let before = input.events.len();
                input
                    .events
                    .retain(|event| !is_clipboard_request(event, request));
                input.events.len() != before
            });
            requested
                || binding.chords.iter().any(|chord| {
                    if binding.while_typing {
                        is_pressed(input, chord)
                    } else {
                        input.consume_shortcut(chord)
                    }
                })
        })
        .map(|binding| binding.command)
}

fn is_pressed(input: &egui::InputState, chord: &KeyboardShortcut) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            Event::Key { key, pressed: true, modifiers, .. }
                if *key == chord.logical_key && modifiers.matches_logically(chord.modifiers)
        )
    })
}

fn is_clipboard_request(event: &Event, request: ClipboardRequest) -> bool {
    matches!(
        (event, request),
        (Event::Copy, ClipboardRequest::Copy)
            | (Event::Cut, ClipboardRequest::Cut)
            | (Event::Paste(_), ClipboardRequest::Paste)
    )
}

/// The chord as menus print it, `Ctrl+Shift+S`; empty for an unbound
/// command. Keys with a one-character symbol print it, other keys are named
/// by the catalog (`key-space`, `key-escape`, ...).
pub(crate) fn shortcut_text(catalog: &LocaleCatalog, command: AppCommand) -> String {
    binding(command).map_or_else(String::new, |binding| {
        chord_text(catalog, &binding.chords[0])
    })
}

pub(crate) fn chord_text(catalog: &LocaleCatalog, chord: &KeyboardShortcut) -> String {
    let names = ModifierNames::NAMES;
    let mut text = names.format(&chord.modifiers, cfg!(target_os = "macos"));
    if !text.is_empty() {
        text.push_str(names.concat);
    }
    text.push_str(&key_text(catalog, chord.logical_key));
    text
}

fn key_text(catalog: &LocaleCatalog, key: Key) -> String {
    let symbol = key.symbol_or_name();
    if symbol.chars().count() == 1 {
        symbol.to_owned()
    } else {
        catalog.text(&key_name_key(key))
    }
}

pub(crate) fn key_name_key(key: Key) -> String {
    format!("key-{}", key.name().to_lowercase())
}
