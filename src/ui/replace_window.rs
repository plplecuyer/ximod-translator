//! Rechercher et remplacer (A-11): the state of the independent window
//! (one at a time, on the active document) and its pure helpers; the
//! drawing and the operations live in `app.rs`, which owns the documents
//! and the undo.

use crate::replace::{Matcher, Mode, Preview, Rule, Scope, Target};

/// Entries kept per field (R-A11-08).
pub const HISTORY: usize = 10;

pub struct ReplaceWindow {
    pub open: bool,
    pub needs_place: bool,
    pub find: String,
    pub with: String,
    pub target: Target,
    pub scope: Scope,
    pub mode: Mode,
    pub case_sensitive: bool,
    pub whole: bool,
    /// Last patterns of the session, most recent first.
    pub find_history: Vec<String>,
    pub with_history: Vec<String>,
    /// Time of the last keystroke (preview recomputed 250 ms later).
    pub typed_at: Option<f64>,
    /// The compiled pattern as last applied, or the error shown in red.
    pub matcher: Option<Matcher>,
    pub error: Option<String>,
    pub preview: Preview,
    /// What the preview was computed for (document revision, scope size).
    pub preview_key: Option<(u64, usize, usize)>,
    /// Transient message under the buttons.
    pub message: Option<String>,
    /// Focus the Rechercher field, text selected (opening, Ctrl+H).
    pub focus_find: bool,
    /// The matrix chosen (B-17): `(file language, name)`; `None` = the
    /// rule typed below.
    pub matrix: Option<(String, String)>,
    pub direction: crate::matrices::Direction,
    /// The matrices offered for the target language, and what they were
    /// read for: `(language, generation of the files)`.
    pub matrices: Vec<(String, crate::matrices::Matrix)>,
    pub matrices_key: Option<(String, u64)>,
    /// The matrix preview and what it was computed for.
    pub matrix_preview: Option<MatrixPreview>,
    pub matrix_preview_key: Option<(String, bool, u64, usize, usize, u64)>,
    /// The name typed for a new matrix (« Ajouter à une matrice… »).
    pub new_matrix: String,
}

/// What a matrix would do on the scope (R-B17-02).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatrixPreview {
    pub rules: usize,
    pub lines: usize,
    pub first: Option<(String, String)>,
    /// The error of a rule that does not compile (1-based rule, message).
    pub error: Option<(usize, String)>,
}

impl Default for ReplaceWindow {
    fn default() -> Self {
        ReplaceWindow {
            open: false,
            needs_place: true,
            find: String::new(),
            with: String::new(),
            target: Target::Trans,
            scope: Scope::Document,
            mode: Mode::Normal,
            case_sensitive: false,
            whole: false,
            find_history: Vec::new(),
            with_history: Vec::new(),
            typed_at: None,
            matcher: None,
            error: None,
            preview: Preview::default(),
            preview_key: None,
            message: None,
            focus_find: true,
            matrix: None,
            direction: crate::matrices::Direction::Direct,
            matrices: Vec::new(),
            matrices_key: None,
            matrix_preview: None,
            matrix_preview_key: None,
            new_matrix: String::new(),
        }
    }
}

impl ReplaceWindow {
    /// The matrix chosen, when still offered.
    pub fn chosen_matrix(&self) -> Option<&crate::matrices::Matrix> {
        let (lang, name) = self.matrix.as_ref()?;
        self.matrices
            .iter()
            .find(|(l, m)| l == lang && &m.name == name)
            .map(|(_, m)| m)
    }

    /// The rule as typed.
    pub fn rule(&self) -> Rule {
        Rule {
            find: self.find.clone(),
            replace: self.with.clone(),
            mode: self.mode,
            case_sensitive: self.case_sensitive,
            whole: self.whole,
        }
    }

    /// Compile the typed pattern into `matcher`, or set `error`; an empty
    /// pattern is neither.
    pub fn compile(&mut self) {
        self.typed_at = None;
        self.preview_key = None;
        match self.rule().compile() {
            Ok(m) => {
                self.matcher = Some(m);
                self.error = None;
            }
            Err(e) => {
                self.matcher = None;
                self.error = if e.is_empty() { None } else { Some(e) };
            }
        }
    }

    /// Remember the current patterns (most recent first, ten at most).
    pub fn remember(&mut self) {
        push_history(&mut self.find_history, &self.find);
        push_history(&mut self.with_history, &self.with);
    }

    /// Whether the buttons may act: a compiled pattern on a replaceable target.
    pub fn can_replace(&self) -> bool {
        self.matcher.is_some() && self.target.replaceable()
    }
}

pub fn push_history(h: &mut Vec<String>, value: &str) {
    if value.is_empty() {
        return;
    }
    h.retain(|v| v != value);
    h.insert(0, value.to_string());
    h.truncate(HISTORY);
}

/// The line after (or before) `current` among `matches`, wrapping around;
/// `None` when there is no match. Without a current line, the first (or
/// the last).
pub fn next_match(matches: &[usize], current: Option<usize>, dir: i32) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let n = matches.len();
    let pos = current.and_then(|c| matches.iter().position(|&m| m == c));
    let next = match (pos, dir >= 0) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(p), true) => (p + 1) % n,
        (Some(p), false) => (p + n - 1) % n,
    };
    // Without a current match, land on the first line after the cursor.
    if pos.is_none()
        && let Some(c) = current
    {
        if dir >= 0 {
            if let Some(p) = matches.iter().position(|&m| m > c) {
                return Some(matches[p]);
            }
        } else if let Some(p) = matches.iter().rposition(|&m| m < c) {
            return Some(matches[p]);
        }
    }
    Some(matches[next])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_deduplicated() {
        let mut h = Vec::new();
        for i in 0..12 {
            push_history(&mut h, &format!("p{i}"));
        }
        push_history(&mut h, "p5");
        push_history(&mut h, "");
        assert_eq!(h.len(), HISTORY);
        assert_eq!(h[0], "p5");
        assert_eq!(h.iter().filter(|v| *v == "p5").count(), 1);
    }

    #[test]
    fn next_match_wraps_and_lands_after_the_cursor() {
        let m = [3usize, 7, 12];
        assert_eq!(next_match(&m, None, 1), Some(3));
        assert_eq!(next_match(&m, None, -1), Some(12));
        assert_eq!(next_match(&m, Some(7), 1), Some(12));
        assert_eq!(next_match(&m, Some(12), 1), Some(3), "wraps");
        assert_eq!(next_match(&m, Some(3), -1), Some(12));
        assert_eq!(
            next_match(&m, Some(5), 1),
            Some(7),
            "cursor between matches"
        );
        assert_eq!(next_match(&m, Some(5), -1), Some(3));
        assert_eq!(next_match(&[], Some(5), 1), None);
    }

    #[test]
    fn compile_tracks_the_error() {
        let mut w = ReplaceWindow {
            find: "(a|".into(),
            mode: Mode::Regex,
            ..Default::default()
        };
        w.compile();
        assert!(w.matcher.is_none() && w.error.is_some() && !w.can_replace());
        w.find = "a".into();
        w.compile();
        assert!(w.can_replace());
        w.target = Target::Source;
        assert!(!w.can_replace(), "the source is read-only on a plugin");
        w.find.clear();
        w.compile();
        assert!(w.matcher.is_none() && w.error.is_none());
    }
}
