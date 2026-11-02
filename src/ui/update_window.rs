//! Mise à jour du mod (B-07): state of the window; the drawing and the
//! operations live in `app.rs`, the comparison in `update.rs`.

use std::path::PathBuf;

use crate::update::Comparison;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    Added,
    Removed,
    #[default]
    Changed,
}

#[derive(Default)]
pub struct UpdateWindow {
    pub open: bool,
    pub needs_place: bool,
    pub doc_serial: Option<u64>,
    pub dict_path: Option<PathBuf>,
    /// Date of the dictionary file (seconds since the epoch).
    pub dict_modified: Option<i64>,
    pub comparison: Option<Comparison>,
    pub tab: Tab,
    pub message: Option<String>,
    pub confirm_forget: bool,
    /// The old / new source differences of the Modifiées tab (A-08),
    /// per line index, computed as the rows show.
    pub diffs: std::collections::HashMap<usize, crate::diff::Diff>,
}
