//! The sort of the grid (A-14, D-110): a click on a title, the cycle
//! ascending / descending / order of the file, and « Ordre du fichier ».

use super::XimodApp;
use crate::sort::{Sort, SortCol, SortDir};
use crate::ui::grid::{H_COMMENT, H_FILE, H_PROV, H_REVIEW};
use eframe::egui;

impl XimodApp {
    /// The column and direction of the active tab's sort.
    pub(crate) fn sort_state(&self) -> Option<(SortCol, SortDir)> {
        self.docs
            .get(self.active)
            .and_then(|d| d.sort.as_ref())
            .map(|s| (s.col, s.dir))
    }

    /// A click on the title of `col` (R-A14-02): the next step of the
    /// cycle, computed once for every line of the document (R-A14-03).
    pub(crate) fn sort_by_column(&mut self, ctx: &egui::Context, col: SortCol) {
        let Some(dir) = crate::sort::next_dir(self.sort_state(), col) else {
            self.sort_file_order();
            return;
        };
        // The Provenance and Commentaire columns sort what they show.
        let texts = match col {
            SortCol::Prov => {
                self.ensure_prov_texts();
                self.prov_texts.as_ref().map(|(_, _, t)| t.0.clone())
            }
            SortCol::Comment => {
                self.ensure_comment_texts(ctx);
                self.comment_texts.as_ref().map(|(_, t)| t.0.clone())
            }
            _ => None,
        };
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        let started = std::time::Instant::now();
        doc.sort = Some(Sort::new(
            &doc.tm.entries,
            col,
            dir,
            texts.as_deref().map(Vec::as_slice),
        ));
        tracing::debug!(
            "sort of {} lines in {} ms",
            doc.tm.entries.len(),
            started.elapsed().as_millis()
        );
        self.grid_scroll_to = self.selection.cursor;
    }

    /// « Ordre du fichier » (R-A14-02): the view goes back to the order of
    /// the file.
    pub(crate) fn sort_file_order(&mut self) {
        let Some(doc) = self.docs.get_mut(self.active) else {
            return;
        };
        if doc.sort.take().is_some() {
            self.status = self.i18n.t("sort-file-order-done");
            self.grid_scroll_to = self.selection.cursor;
        }
    }

    /// « Tri : Source, croissant » while a sort is on (status bar).
    pub(crate) fn sort_status_text(&self) -> Option<String> {
        let (col, dir) = self.sort_state()?;
        let headers = self.grid_headers();
        let name = match col {
            SortCol::Status => self.i18n.t("sort-col-status"),
            SortCol::Record => headers[1].clone(),
            SortCol::Field => headers[2].clone(),
            SortCol::Edid => headers[3].clone(),
            SortCol::Source => headers[4].clone(),
            SortCol::Trans => headers[5].clone(),
            SortCol::Review => headers[H_REVIEW].clone(),
            SortCol::Prov => headers[H_PROV].clone(),
            SortCol::Comment => headers[H_COMMENT].clone(),
            SortCol::File => headers[H_FILE].clone(),
        };
        let key = match dir {
            SortDir::Asc => "sort-status-asc",
            SortDir::Desc => "sort-status-desc",
        };
        Some(self.i18n.t_arg(key, "col", &name))
    }
}
