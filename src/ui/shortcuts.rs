//! Every keyboard shortcut, as plain tables the app reads and the tests
//! check. They follow Aseprite where Aseprite has one, so its users'
//! fingers already know them.

use super::app::{Action, Panel};
use super::tools::Tool;
use crate::editor::Command;
use egui::{Key, KeyboardShortcut, Modifiers};

/// Shortcuts with Ctrl (⌘ on macOS) or Alt. These work even while typing.
pub fn chords() -> Vec<(KeyboardShortcut, Action)> {
    let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
    let command_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
    let alt = |key| KeyboardShortcut::new(Modifiers::ALT, key);
    // Ctrl+Shift+Z comes before Ctrl+Z, so the longer chord wins.
    vec![
        (command_shift(Key::Z), Action::Edit(Command::Redo)),
        (command(Key::Z), Action::Edit(Command::Undo)),
        (command(Key::Y), Action::Edit(Command::Redo)),
        (command(Key::K), Action::CommandPalette),
        (command_shift(Key::S), Action::SaveAs),
        (command(Key::S), Action::Save),
        (command(Key::O), Action::Open),
        (command(Key::N), Action::New),
        (command(Key::W), Action::Close),
        (command(Key::C), Action::Edit(Command::Copy)),
        (command(Key::X), Action::Edit(Command::Cut)),
        (command(Key::V), Action::Edit(Command::Paste { at: None })),
        (command(Key::A), Action::Edit(Command::SelectAll)),
        (command(Key::D), Action::Edit(Command::SelectNone)),
        (command_shift(Key::I), Action::Edit(Command::InvertSelection)),
        (command(Key::E), Action::Toggle(Panel::Export)),
        (command(Key::Plus), Action::ZoomIn),
        (command(Key::Equals), Action::ZoomIn),
        (command(Key::Minus), Action::ZoomOut),
        (command(Key::Num0), Action::ZoomToFit),
        (alt(Key::OpenBracket), Action::PreviousFrame),
        (alt(Key::CloseBracket), Action::NextFrame),
    ]
}

/// Shift plus a letter: whole-cel transforms, which are easy to hit by
/// accident, so like Aseprite they need Shift.
pub fn shifted_keys() -> Vec<(Key, Action)> {
    vec![
        (Key::H, Action::Edit(Command::FlipHorizontal)),
        (Key::V, Action::Edit(Command::FlipVertical)),
        (Key::R, Action::Edit(Command::RotateClockwise)),
    ]
}

/// Bare keys, only when no text field has focus: every tool's letter, then
/// the rest.
pub fn single_keys() -> Vec<(Key, Action)> {
    let mut keys: Vec<(Key, Action)> =
        Tool::ALL.iter().filter_map(|&tool| tool.shortcut().map(|key| (key, Action::UseTool(tool)))).collect();
    keys.extend([
        (Key::X, Action::SwapColours),
        (Key::Enter, Action::PlayPause),
        (Key::Delete, Action::Edit(Command::DeleteSelection)),
        (Key::Backspace, Action::Edit(Command::DeleteSelection)),
        (Key::Escape, Action::Edit(Command::Anchor)),
        (Key::Comma, Action::PreviousFrame),
        (Key::Period, Action::NextFrame),
        (Key::Tab, Action::ToggleOnionSkin),
    ]);
    keys.extend(nudges().into_iter().map(|(key, dx, dy)| (key, Action::Edit(Command::MoveFloating { dx, dy }))));
    keys
}

/// Arrow keys move the selection by one pixel.
fn nudges() -> [(Key, i32, i32); 4] {
    [(Key::ArrowLeft, -1, 0), (Key::ArrowRight, 1, 0), (Key::ArrowUp, 0, -1), (Key::ArrowDown, 0, 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare(key: Key) -> Option<Action> {
        single_keys().into_iter().find(|(k, _)| *k == key).map(|(_, action)| action)
    }

    fn chord(modifiers: Modifiers, key: Key) -> Option<Action> {
        chords().into_iter().find(|(s, _)| *s == KeyboardShortcut::new(modifiers, key)).map(|(_, action)| action)
    }

    #[test]
    fn the_aseprite_keys_do_what_aseprite_users_expect() {
        for (key, tool) in [
            (Key::B, Tool::Pencil),
            (Key::E, Tool::Eraser),
            (Key::G, Tool::Fill),
            (Key::I, Tool::Eyedropper),
            (Key::L, Tool::Line),
            (Key::U, Tool::Rectangle),
            (Key::M, Tool::Select),
            (Key::Q, Tool::Lasso),
            (Key::W, Tool::Wand),
            (Key::V, Tool::Move),
        ] {
            assert_eq!(bare(key), Some(Action::UseTool(tool)), "{key:?}");
        }
        assert_eq!(bare(Key::X), Some(Action::SwapColours));
        assert_eq!(bare(Key::Enter), Some(Action::PlayPause));
        assert_eq!(chord(Modifiers::ALT, Key::OpenBracket), Some(Action::PreviousFrame));
        assert_eq!(chord(Modifiers::ALT, Key::CloseBracket), Some(Action::NextFrame));
        assert_eq!(chord(Modifiers::COMMAND, Key::Z), Some(Action::Edit(Command::Undo)));
        assert_eq!(chord(Modifiers::COMMAND, Key::Y), Some(Action::Edit(Command::Redo)));
        assert_eq!(chord(Modifiers::COMMAND, Key::K), Some(Action::CommandPalette));
    }

    #[test]
    fn no_key_does_two_things() {
        let mut bare_keys: Vec<Key> = single_keys().into_iter().map(|(key, _)| key).collect();
        let before = bare_keys.len();
        bare_keys.sort_by_key(|key| key.name());
        bare_keys.dedup();
        assert_eq!(bare_keys.len(), before, "two bare-key bindings share a key");
        let mut chord_keys: Vec<KeyboardShortcut> = chords().into_iter().map(|(chord, _)| chord).collect();
        let before = chord_keys.len();
        chord_keys.sort_by_key(|chord| format!("{chord:?}"));
        chord_keys.dedup();
        assert_eq!(chord_keys.len(), before, "two chords share a key");
    }

    #[test]
    fn redo_with_shift_is_checked_before_plain_undo() {
        let order: Vec<KeyboardShortcut> = chords().into_iter().map(|(chord, _)| chord).collect();
        let redo =
            order.iter().position(|c| *c == KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z));
        let undo = order.iter().position(|c| *c == KeyboardShortcut::new(Modifiers::COMMAND, Key::Z));
        assert!(redo < undo, "egui consumes the first match, so Ctrl+Shift+Z must come first");
    }
}
