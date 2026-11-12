//! The debugging export (A-22, D-113): the reverse check of the export
//! dialog and what the journal says of a debugging export.

use super::XimodApp;
use crate::export::ExportPolicy;
use crate::journal::{Kind, Level};

impl XimodApp {
    /// The exported lines whose translation already starts with an
    /// identifier (R-A22-03), computed again only when the document or the
    /// policy changed.
    /// Returns those lines and the number of lines a debugging export
    /// writes with their identifier.
    pub(crate) fn export_ids_present(&mut self, plain: &ExportPolicy) -> (Vec<usize>, usize) {
        let Some(doc) = self.docs.get(self.active) else {
            return (Vec::new(), 0);
        };
        let key = (doc.serial, doc.undo.revision, plain.include);
        if let Some((k, rows, n)) = &self.export_ids_cache
            && *k == key
        {
            return (rows.clone(), *n);
        }
        let rows = plain.ids_present(&doc.tm);
        let n = plain.ids_lines(&doc.tm);
        self.export_ids_cache = Some((key, rows.clone(), n));
        (rows, n)
    }

    /// « voir les lignes »: the lines in the grid, behind a filter chip.
    pub(crate) fn export_ids_show_rows(&mut self, rows: Vec<usize>) {
        if rows.is_empty() {
            return;
        }
        let text = self
            .i18n
            .t_arg("cli-export-ids-left", "n", &rows.len().to_string());
        let path = match self.docs.get(self.active) {
            Some(d) => d.path.clone(),
            None => return,
        };
        let set = crate::filter::AnomalySet(std::sync::Arc::new(rows.into_iter().collect()));
        self.filter_journal = Some((text, set, path));
        self.pending_export = None;
    }

    /// The journal of a debugging export: its form, and the lines cut to
    /// their field's limit (R-A22-01, R-A22-02).
    pub(crate) fn journal_debug_export(&mut self, policy: &ExportPolicy) {
        let Some(form) = policy.ids else {
            return;
        };
        let file = self
            .docs
            .get(self.active)
            .map(|d| d.file_name())
            .unwrap_or_default();
        let mut text = self.i18n.t_arg("export-debug-title", "file", &file);
        text.push_str(&format!(" ({})", self.i18n.t(form.i18n_key())));
        self.journal_add(Level::Warn, Kind::Export, &text);
        let cut = self
            .docs
            .get(self.active)
            .map(|d| policy.ids_truncated(&d.tm))
            .unwrap_or(0);
        if cut > 0 {
            let msg = self
                .i18n
                .t_arg("export-ids-truncated", "n", &cut.to_string());
            self.journal_add(Level::Warn, Kind::Export, &msg);
        }
    }
}
