//! Icons of the interface: the Phosphor icon font, registered as a fallback
//! of the proportional family, so an icon is a plain string anywhere a label
//! is.
//!
//! The font and the code points are those of egui-phosphor 0.7.3 (Phosphor
//! icons, MIT licence, `icons/LICENSE-Phosphor.txt`), kept in the project when
//! egui moved to 0.37 (D-103): egui-phosphor follows egui one version behind,
//! and only these icons are used.

/// The Phosphor Regular font, embedded.
pub const FONT: &[u8] = include_bytes!("icons/Phosphor.ttf");

/// Register the icon font, second in the proportional family: after the text
/// font (Phosphor maps a–z to the blank glyphs of its ligatures), before the
/// other fallbacks.
pub fn add_to_fonts(fonts: &mut eframe::egui::FontDefinitions) {
    fonts.font_data.insert(
        "phosphor".into(),
        std::sync::Arc::new(eframe::egui::FontData::from_static(FONT)),
    );
    if let Some(keys) = fonts
        .families
        .get_mut(&eframe::egui::FontFamily::Proportional)
    {
        let at = keys.len().min(1);
        keys.insert(at, "phosphor".into());
    }
}

pub const APPROXIMATE_EQUALS: &str = "\u{EDAA}";
pub const REDO: &str = "\u{E036}";
pub const UNDO: &str = "\u{E038}";
pub const ARROW_DOWN: &str = "\u{E03E}";
pub const RETURN: &str = "\u{E044}";
pub const ARROW_RIGHT: &str = "\u{E06C}";
pub const ARROW_UP: &str = "\u{E08E}";
pub const BACKSPACE: &str = "\u{E0AE}";
pub const BOOK_OPEN_TEXT: &str = "\u{E8F2}";
pub const CARET_DOWN: &str = "\u{E136}";
pub const CARET_LEFT: &str = "\u{E138}";
pub const CARET_RIGHT: &str = "\u{E13A}";
pub const CARET_UP: &str = "\u{E13C}";
pub const CHECK: &str = "\u{E182}";
pub const CHECKS: &str = "\u{E53A}";
pub const CIRCLE: &str = "\u{E18A}";
pub const CIRCLE_HALF: &str = "\u{E18C}";
pub const DOT: &str = "\u{ECDE}";
pub const DOTS_THREE: &str = "\u{E1FE}";
pub const ENVELOPE_SIMPLE: &str = "\u{E218}";
pub const FLAG: &str = "\u{E244}";
pub const FUNNEL: &str = "\u{E266}";
pub const GEAR: &str = "\u{E270}";
pub const INFO: &str = "\u{E2CE}";
pub const LIST_BULLETS: &str = "\u{E2F2}";
pub const LOCK: &str = "\u{E2FA}";
pub const PENCIL_SIMPLE: &str = "\u{E3B4}";
pub const PROHIBIT: &str = "\u{E3DE}";
pub const QUOTES: &str = "\u{E660}";
pub const STAR: &str = "\u{E46A}";
pub const WARNING: &str = "\u{E4E0}";
pub const WARNING_CIRCLE: &str = "\u{E4E2}";
pub const X: &str = "\u{E4F6}";
pub const X_CIRCLE: &str = "\u{E4F8}";

/// The carriage-return glyph of the special characters, as a character.
pub const RETURN_CHAR: char = '\u{E044}';

/// The right arrow as a character, for the display of special characters
/// (a tabulation).
pub const ARROW_RIGHT_CHAR: char = '\u{E06C}';
