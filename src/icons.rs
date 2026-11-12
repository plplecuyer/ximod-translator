//! Icons of the interface: the Phosphor icon font (egui-phosphor), registered
//! as a fallback of the proportional and monospace families, so an icon is a
//! plain string anywhere a label is.

pub use egui_phosphor::regular::{
    APPROXIMATE_EQUALS, ARROW_CLOCKWISE as REDO, ARROW_COUNTER_CLOCKWISE as UNDO, ARROW_DOWN,
    ARROW_ELBOW_DOWN_LEFT as RETURN, ARROW_RIGHT, ARROW_UP, BACKSPACE, BOOK_OPEN_TEXT, CARET_DOWN,
    CARET_LEFT, CARET_RIGHT, CARET_UP, CHECK, CHECKS, CIRCLE, CIRCLE_HALF, DOT, DOTS_THREE,
    ENVELOPE_SIMPLE, FUNNEL, GEAR, INFO, LIST_BULLETS, LOCK, PENCIL_SIMPLE, PROHIBIT, QUOTES, STAR,
    WARNING, WARNING_CIRCLE, X, X_CIRCLE,
};

/// The carriage-return glyph of the special characters, as a character.
pub const RETURN_CHAR: char = '\u{E044}';

/// The right arrow as a character, for the display of special characters
/// (a tabulation).
pub const ARROW_RIGHT_CHAR: char = '\u{E06C}';
