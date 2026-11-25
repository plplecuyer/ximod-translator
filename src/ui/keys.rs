//! Keyboard shortcuts that hold on every system and layout (Windows
//! recette of 13/11/2026: Ctrl+Maj+U and Ctrl+Maj+J did nothing).
//!
//! egui matches a shortcut on the *logical* key of the event, the key the
//! layout produces. When the layout or the system turns Ctrl+letter into
//! something else — a control character mapped to Entrée (Ctrl+J, Ctrl+M),
//! Retour arrière (Ctrl+H), or no key at all — the shortcut never matches.
//! [`TakeShortcut::take_shortcut`] then falls back on the *physical* key,
//! but only when the logical key is not itself a letter: on an AZERTY
//! keyboard the physical W is the logical Z, and Ctrl+Z must not close the
//! document.
//!
//! `XIMOD_KEY_LOG=1` writes every key event to the log (logical key,
//! physical key, modifiers), to find out what a system sends.

use eframe::egui;

/// A letter key (A to Z).
pub fn is_letter(k: egui::Key) -> bool {
    use egui::Key::*;
    matches!(
        k,
        A | B
            | C
            | D
            | E
            | F
            | G
            | H
            | I
            | J
            | K
            | L
            | M
            | N
            | O
            | P
            | Q
            | R
            | S
            | T
            | U
            | V
            | W
            | X
            | Y
            | Z
    )
}

/// Whether a key event is the shortcut `sc`: its logical key, or its
/// physical key when the logical one is not a letter.
pub fn event_matches(ev: &egui::Event, sc: &egui::KeyboardShortcut) -> bool {
    let egui::Event::Key {
        key,
        physical_key,
        pressed: true,
        modifiers,
        ..
    } = ev
    else {
        return false;
    };
    if !modifiers.matches_logically(sc.modifiers) {
        return false;
    }
    *key == sc.logical_key || (!is_letter(*key) && *physical_key == Some(sc.logical_key))
}

/// `consume_shortcut` with the physical fallback.
pub trait TakeShortcut {
    fn take_shortcut(&mut self, sc: &egui::KeyboardShortcut) -> bool;
}

impl TakeShortcut for egui::InputState {
    fn take_shortcut(&mut self, sc: &egui::KeyboardShortcut) -> bool {
        if self.consume_shortcut(sc) {
            return true;
        }
        match self.events.iter().position(|e| event_matches(e, sc)) {
            Some(i) => {
                self.events.remove(i);
                true
            }
            None => false,
        }
    }
}

/// Whether the pressed modifiers are exactly those of `sc` (E-15): a
/// configurable shortcut answers its own combination only, so that
/// Ctrl+Alt+U never fires a command left on Ctrl+U. Shift is tolerated on
/// a punctuation key, which some layouts produce with it.
fn modifiers_exact(pressed: egui::Modifiers, sc: &egui::KeyboardShortcut) -> bool {
    let cmd = |m: egui::Modifiers| m.command || m.ctrl || m.mac_cmd;
    let p = sc.modifiers;
    let shift_ok =
        pressed.shift == p.shift || (pressed.shift && !p.shift && is_punctuation(sc.logical_key));
    cmd(pressed) == cmd(p) && pressed.alt == p.alt && shift_ok
}

fn is_punctuation(k: egui::Key) -> bool {
    use egui::Key::*;
    matches!(
        k,
        Comma
            | Period
            | Semicolon
            | Colon
            | Minus
            | Plus
            | Equals
            | Slash
            | Backslash
            | Quote
            | Backtick
            | OpenBracket
            | CloseBracket
    )
}

/// [`event_matches`] with the modifiers exact.
pub fn event_matches_exact(ev: &egui::Event, sc: &egui::KeyboardShortcut) -> bool {
    match ev {
        egui::Event::Key {
            pressed: true,
            modifiers,
            ..
        } => modifiers_exact(*modifiers, sc) && event_matches(ev, sc),
        _ => false,
    }
}

/// The command `c` was pressed (its shortcut in force, E-15): the event is
/// consumed. A command without a shortcut never answers.
pub trait TakeCmd {
    fn take_cmd(&mut self, c: crate::shortcuts::Cmd) -> bool;
}

impl TakeCmd for egui::InputState {
    fn take_cmd(&mut self, c: crate::shortcuts::Cmd) -> bool {
        let Some(sc) = crate::shortcuts::get(c) else {
            return false;
        };
        match self.events.iter().position(|e| event_matches_exact(e, &sc)) {
            Some(i) => {
                self.events.remove(i);
                true
            }
            None => false,
        }
    }
}

/// Whether `XIMOD_KEY_LOG` asks for the key events in the log.
pub fn key_log_enabled() -> bool {
    std::env::var("XIMOD_KEY_LOG").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Log the key events of this frame (diagnostic, English like the log).
pub fn log_keys(ctx: &egui::Context) {
    ctx.input(|i| {
        for e in &i.events {
            if let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } = e
            {
                tracing::info!(
                    "key: logical {key:?}, physical {physical_key:?}, ctrl {} shift {} alt {} command {}",
                    modifiers.ctrl,
                    modifiers.shift,
                    modifiers.alt,
                    modifiers.command
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, KeyboardShortcut, Modifiers};

    fn key(logical: Key, physical: Option<Key>, m: Modifiers) -> Event {
        Event::Key {
            key: logical,
            physical_key: physical,
            pressed: true,
            repeat: false,
            modifiers: m,
        }
    }

    #[test]
    fn the_physical_key_helps_only_when_the_logical_one_is_no_letter() {
        let cs = Modifiers::COMMAND.plus(Modifiers::SHIFT);
        let journal = KeyboardShortcut::new(cs, Key::J);
        // Ctrl+J delivered as Entrée by the system: found by its position.
        assert!(event_matches(&key(Key::Enter, Some(Key::J), cs), &journal));
        assert!(event_matches(&key(Key::J, Some(Key::J), cs), &journal));
        // Without Shift, not the shortcut.
        assert!(!event_matches(
            &key(Key::J, Some(Key::J), Modifiers::COMMAND),
            &journal
        ));
        // AZERTY: Ctrl+Z is on the physical W; it must not be Ctrl+W.
        let close = KeyboardShortcut::new(Modifiers::COMMAND, Key::W);
        assert!(!event_matches(
            &key(Key::Z, Some(Key::W), Modifiers::COMMAND),
            &close
        ));
        // Exact modifiers (E-15): Ctrl+Alt+U is not Ctrl+U, nor Ctrl+Maj+U
        // Ctrl+Alt+U; the physical fallback still holds.
        let cu = KeyboardShortcut::new(Modifiers::COMMAND, Key::U);
        let cau = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::U);
        let ca = Modifiers::COMMAND.plus(Modifiers::ALT);
        assert!(event_matches(&key(Key::U, Some(Key::U), ca), &cu));
        assert!(!event_matches_exact(&key(Key::U, Some(Key::U), ca), &cu));
        assert!(event_matches_exact(&key(Key::U, Some(Key::U), ca), &cau));
        assert!(!event_matches_exact(&key(Key::U, Some(Key::U), cs), &cau));
        assert!(event_matches_exact(
            &key(Key::Enter, Some(Key::J), cs),
            &journal
        ));
        // Ctrl+, on a layout that needs Shift for the comma.
        let settings = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
        assert!(event_matches_exact(&key(Key::Comma, None, cs), &settings));
        // A plain Entrée is never a letter shortcut.
        assert!(!event_matches(
            &key(Key::Enter, Some(Key::Enter), cs),
            &journal
        ));
    }
}
