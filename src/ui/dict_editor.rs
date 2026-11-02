//! The dictionary editor window (B-04): state of the window; the drawing
//! and the file operations live in `app.rs` and `dict_edit.rs`.

use crate::basedb::BaseRow;
use crate::dict_edit::DictDoc;

#[derive(Default)]
pub struct DictEditor {
    pub open: bool,
    pub needs_place: bool,
    pub doc: Option<DictDoc>,
    pub filter: String,
    /// Time of the last keystroke in the filter (applied 250 ms later).
    pub typed_at: Option<f64>,
    /// Row indices shown, in the sort order.
    pub rows: Vec<usize>,
    /// What `rows` was computed for: (filter, sort, rows count, dirty tick).
    pub rows_key: Option<(String, (usize, bool), usize, u64)>,
    pub tick: u64,
    pub sort: (usize, bool),
    pub selected: Option<usize>,
    /// The edit zone.
    pub e_source: String,
    pub e_field: String,
    pub e_trans: String,
    pub message: Option<String>,
    pub confirm_close: bool,
    /// Focus the Source field (after Ajouter).
    pub focus_source: bool,
    /// Show the old data (B-07) instead of the current rows.
    pub show_old: bool,
}

impl DictEditor {
    /// Open on a dictionary; the window goes (back) to the front.
    pub fn open_on(&mut self, doc: DictDoc) {
        if !self.open {
            self.needs_place = true;
        }
        self.open = true;
        self.doc = Some(doc);
        self.filter.clear();
        self.typed_at = None;
        self.rows.clear();
        self.rows_key = None;
        self.selected = None;
        self.show_old = false;
        self.sort = (0, true);
        self.clear_edit();
        self.message = None;
        self.confirm_close = false;
        self.tick += 1;
    }

    pub fn clear_edit(&mut self) {
        self.e_source.clear();
        self.e_field.clear();
        self.e_trans.clear();
    }

    /// Put a row into the edit zone.
    pub fn load_edit(&mut self, row: &BaseRow) {
        self.e_source = row.source.clone();
        self.e_field = if row.rec.is_empty() && row.field.is_empty() {
            String::new()
        } else {
            format!("{}:{}", row.rec, row.field)
        };
        self.e_trans = row.trans.clone();
    }

    /// The row described by the edit zone, keeping the identifiers of
    /// `base` when the signature did not change (a replaced signed pair
    /// keeps its FormID and EDID).
    pub fn edited_row(&self, base: Option<&BaseRow>) -> Option<BaseRow> {
        let source = self.e_source.trim();
        let trans = self.e_trans.trim();
        if source.is_empty() || trans.is_empty() {
            return None;
        }
        let (rec, field) = parse_sig(&self.e_field)?;
        let mut row = BaseRow {
            rec: rec.clone(),
            field: field.clone(),
            formid: 0,
            edid: String::new(),
            source: source.to_string(),
            trans: trans.to_string(),
            status: None,
            ..Default::default()
        };
        if let Some(b) = base
            && b.rec == rec
            && b.field == field
        {
            row.formid = b.formid;
            row.edid = b.edid.clone();
            row.status = b.status.clone();
        }
        Some(row)
    }

    /// Recompute the visible rows when the filter, the sort or the file changed.
    pub fn refresh_rows(&mut self) {
        let Some(doc) = &self.doc else {
            self.rows.clear();
            return;
        };
        let key = (
            self.filter.clone(),
            self.sort,
            doc.rows.len() + doc.old.len() * 1_000_003 + usize::from(self.show_old),
            self.tick,
        );
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        let mut rows = if self.show_old {
            doc.filter_old(&self.filter)
        } else {
            doc.filter(&self.filter)
        };
        let (col, asc) = self.sort;
        let show_old = self.show_old;
        let at = |i: usize| -> &BaseRow {
            if show_old {
                &doc.old[i].row
            } else {
                &doc.rows[i]
            }
        };
        rows.sort_by(|&a, &b| {
            let (ra, rb) = (at(a), at(b));
            let ord = match col {
                1 => ra.trans.to_lowercase().cmp(&rb.trans.to_lowercase()),
                2 => {
                    (ra.rec.as_str(), ra.field.as_str()).cmp(&(rb.rec.as_str(), rb.field.as_str()))
                }
                3 => ra.status.cmp(&rb.status),
                _ => ra.source.to_lowercase().cmp(&rb.source.to_lowercase()),
            };
            if asc { ord } else { ord.reverse() }
        });
        self.rows = rows;
        self.rows_key = Some(key);
        if let Some(sel) = self.selected
            && !self.rows.contains(&sel)
        {
            self.selected = None;
        }
    }

    /// Something changed in the file: the rows are recomputed next frame.
    pub fn touch(&mut self) {
        self.tick += 1;
    }
}

/// « REC:FLD » → (rec, field); empty → text-only; anything else is refused.
pub fn parse_sig(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    if s.is_empty() || s == "—" {
        return Some((String::new(), String::new()));
    }
    let (rec, field) = s.split_once(':')?;
    let (rec, field) = (rec.trim().to_uppercase(), field.trim().to_uppercase());
    if rec.is_empty() || field.is_empty() {
        return None;
    }
    Some((rec, field))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_parsing() {
        assert_eq!(parse_sig(""), Some((String::new(), String::new())));
        assert_eq!(parse_sig(" — "), Some((String::new(), String::new())));
        assert_eq!(parse_sig("misc:full"), Some(("MISC".into(), "FULL".into())));
        assert_eq!(parse_sig("MISC"), None);
        assert_eq!(parse_sig(":FULL"), None);
    }

    #[test]
    fn edited_row_keeps_identifiers_of_the_same_signature() {
        let base = BaseRow {
            rec: "MISC".into(),
            field: "FULL".into(),
            formid: 7,
            edid: "X".into(),
            source: "a".into(),
            trans: "b".into(),
            status: Some("validated".into()),
            ..Default::default()
        };
        let mut ed = DictEditor::default();
        ed.load_edit(&base);
        assert_eq!(ed.e_field, "MISC:FULL");
        ed.e_trans = "c".into();
        let r = ed.edited_row(Some(&base)).unwrap();
        assert_eq!((r.formid, r.edid.as_str(), r.trans.as_str()), (7, "X", "c"));
        ed.e_field.clear();
        let r = ed.edited_row(Some(&base)).unwrap();
        assert_eq!(
            (r.formid, r.rec.as_str()),
            (0, ""),
            "a text-only pair drops them"
        );
        ed.e_trans.clear();
        assert!(ed.edited_row(None).is_none());
    }

    #[test]
    fn rows_follow_filter_and_sort() {
        let mut ed = DictEditor::default();
        let mut doc = DictDoc {
            path: std::path::PathBuf::from("x.ximdb"),
            lang: "fra".into(),
            rows: Vec::new(),
            old: Vec::new(),
            dirty: false,
            read_only: false,
        };
        doc.rows
            .push(crate::dict_edit::text_pair("Dwemer Ruins", "Ruines dwemer"));
        doc.rows.push(crate::dict_edit::text_pair("Axe", "Hache"));
        doc.rows.push(crate::dict_edit::text_pair("Bow", "Arc"));
        ed.open_on(doc);
        ed.refresh_rows();
        assert_eq!(ed.rows, vec![1, 2, 0], "sorted by source");
        ed.sort = (1, true);
        ed.refresh_rows();
        assert_eq!(ed.rows, vec![2, 1, 0], "by translation: Arc, Hache, Ruines");
        ed.filter = "dwem".into();
        ed.refresh_rows();
        assert_eq!(ed.rows, vec![0]);
    }
}
