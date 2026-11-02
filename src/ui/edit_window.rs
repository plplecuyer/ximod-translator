//! The edit window (A-02): one independent window, tied to the active
//! document and its current line — source and translation in wide areas,
//! the line's identity, context tabs, navigation and status buttons. The
//! drawing lives in `app.rs` (it needs the whole application state); this
//! module keeps the window's own state and the pure helpers.

use crate::tm::TmEntry;

/// The context tabs (R-A02-05). `Aliases` arrives with A-07.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditTab {
    Record,
    Aliases,
    /// Vocabulaire (B-01, B-14).
    Vocab,
    /// Communes (C-04): the lines of the same source.
    Common,
    /// Script (C-06, C-07): the script of a line and its source.
    Script,
    /// Dialogue (C-01, C-02, C-13).
    Dialogue,
    /// Historique (A-18): the entries of the line.
    History,
}

impl EditTab {
    /// `[Editing] EditTab` value.
    pub fn key(self) -> &'static str {
        match self {
            EditTab::Record => "record",
            EditTab::Aliases => "aliases",
            EditTab::Vocab => "vocab",
            EditTab::Common => "common",
            EditTab::Script => "script",
            EditTab::Dialogue => "dialogue",
            EditTab::History => "history",
        }
    }

    pub fn from_key(k: &str) -> EditTab {
        match k.trim() {
            "aliases" => EditTab::Aliases,
            "vocab" => EditTab::Vocab,
            "common" => EditTab::Common,
            "script" => EditTab::Script,
            "dialogue" => EditTab::Dialogue,
            "history" => EditTab::History,
            _ => EditTab::Record,
        }
    }

    /// The tabs available at this stage (the others appear with their lot).
    pub const AVAILABLE: [EditTab; 7] = [
        EditTab::Record,
        EditTab::Aliases,
        EditTab::Vocab,
        EditTab::Common,
        EditTab::Script,
        EditTab::Dialogue,
        EditTab::History,
    ];
}

/// State of the open edit window.
pub struct EditWindow {
    /// Entry index displayed (in the active document).
    pub line: usize,
    /// Place the viewport on the next frame (just opened).
    pub needs_place: bool,
    /// Give the focus to the Translation area, cursor at the end.
    pub focus_request: bool,
    /// Transient message replacing the hint line until the next action.
    pub message: Option<String>,
    pub tab: EditTab,
    /// Put the cursor of the Translation area there at the next focus
    /// (after an instant translation, B-14); at the end otherwise.
    pub cursor_at: Option<usize>,
    /// An instant translation of the whole source waiting for « Remplacer »
    /// (R-B14-01): the new text and the report.
    pub confirm: Option<InstantConfirm>,
}

/// A whole-source instant translation awaiting confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantConfirm {
    pub text: String,
    pub message: String,
    /// The provenance the text will carry (B-09).
    pub prov: String,
}

impl EditWindow {
    pub fn new(line: usize, tab: EditTab) -> Self {
        let tab = if EditTab::AVAILABLE.contains(&tab) {
            tab
        } else {
            EditTab::Record
        };
        EditWindow {
            line,
            needs_place: true,
            focus_request: true,
            message: None,
            tab,
            cursor_at: None,
            confirm: None,
        }
    }

    /// Show another line: the focus goes back to the Translation area.
    pub fn go_to(&mut self, line: usize) {
        if self.line != line {
            self.line = line;
            self.message = None;
        }
        self.focus_request = true;
    }
}

/// 1-based position of a line in the filtered view, if it is shown.
pub fn position_in_view(visible: &[usize], line: usize) -> Option<usize> {
    visible.iter().position(|&v| v == line).map(|p| p + 1)
}

/// The other translatable fields of the same record (same FormID), in file
/// order (R-A02-05). Empty when the line has no FormID.
pub fn record_siblings(entries: &[TmEntry], line: usize) -> Vec<usize> {
    let Some(e) = entries.get(line) else {
        return Vec::new();
    };
    if e.formid == 0 {
        return Vec::new();
    }
    let same_plugin = |o: &TmEntry| o.plugin.eq_ignore_ascii_case(&e.plugin);
    entries
        .iter()
        .enumerate()
        .filter(|(i, o)| *i != line && o.formid == e.formid && !o.unused && same_plugin(o))
        .map(|(i, _)| i)
        .collect()
}

/// Whether Enter may insert a line break in this field (R-A02-04): the
/// `newline` flag of the game's field table; a field without a rule, or
/// whose flag is still unknown, admits it (nothing is known against it).
pub fn newline_allowed(game: crate::games::Game, rec: &str, fld: &str) -> bool {
    crate::games::registry()
        .fields(game)
        .limits(rec, fld)
        .and_then(|r| r.newline)
        .unwrap_or(true)
}

/// One line of text at most `max` characters, « … » when cut.
pub fn one_line(text: &str, max: usize) -> String {
    let flat: String = text
        .split(['\r', '\n'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(&format!(" {} ", crate::icons::RETURN_CHAR));
    let mut out: String = flat.chars().take(max).collect();
    if flat.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(formid: u32, fld: &str, unused: bool) -> TmEntry {
        TmEntry {
            formid,
            rec_sig: "WEAP".into(),
            field_sig: fld.into(),
            source: "x".into(),
            unused,
            ..Default::default()
        }
    }

    #[test]
    fn siblings_share_the_formid_and_skip_unused_lines() {
        let entries = vec![
            e(1, "FULL", false),
            e(1, "DESC", false),
            e(2, "FULL", false),
            e(1, "NAM1", true),
            e(0, "FULL", false),
        ];
        assert_eq!(record_siblings(&entries, 0), vec![1]);
        assert_eq!(record_siblings(&entries, 1), vec![0]);
        assert_eq!(record_siblings(&entries, 2), Vec::<usize>::new());
        // No FormID: no context.
        assert_eq!(record_siblings(&entries, 4), Vec::<usize>::new());
    }

    #[test]
    fn position_follows_the_view_order() {
        let visible = [7usize, 3, 9];
        assert_eq!(position_in_view(&visible, 3), Some(2));
        assert_eq!(position_in_view(&visible, 4), None);
    }

    #[test]
    fn one_line_flattens_and_cuts() {
        assert_eq!(one_line("a\nb", 10), "a \u{E044} b");
        assert_eq!(one_line("abcdef", 3), "abc…");
        assert_eq!(one_line("abc", 3), "abc");
    }

    #[test]
    fn tab_keys_round_trip() {
        assert_eq!(EditTab::from_key(EditTab::Record.key()), EditTab::Record);
        assert_eq!(EditTab::from_key("whatever"), EditTab::Record);
        assert_eq!(EditTab::from_key(EditTab::Common.key()), EditTab::Common);
        assert_eq!(EditTab::AVAILABLE[3], EditTab::Common);
        assert!(EditTab::AVAILABLE.contains(&EditTab::Aliases));
        assert_eq!(EditWindow::new(0, EditTab::Aliases).tab, EditTab::Aliases);
    }
}
