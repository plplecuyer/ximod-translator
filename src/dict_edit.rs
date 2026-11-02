//! An editable dictionary file (B-04): the rows of a `.ximdb` in memory,
//! changed by the editor window or by « Ajouter à mon dictionnaire
//! personnel », written back in format 3 with the timestamped copies of
//! B-11. The personal dictionary is one of them.

use std::path::{Path, PathBuf};

use crate::basedb::{self, BaseRow};

/// What happened to a pair put into the dictionary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Put {
    Added,
    Replaced,
    /// Already there with the same translation.
    Same,
}

#[derive(Debug, Clone)]
pub struct DictDoc {
    pub path: PathBuf,
    /// Target language (ISO 639-3) the file declares.
    pub lang: String,
    pub rows: Vec<BaseRow>,
    /// The old data of a mod's dictionary (B-07).
    pub old: Vec<basedb::OldRow>,
    pub dirty: bool,
    /// A dictionary shipped with the program: saved elsewhere only.
    pub read_only: bool,
}

impl DictDoc {
    /// Read a `.ximdb`, or start an empty one for `lang` when the file does
    /// not exist yet.
    pub fn load(path: &Path, lang: &str) -> Result<DictDoc, String> {
        if !path.is_file() {
            return Ok(DictDoc {
                path: path.to_path_buf(),
                lang: lang.to_string(),
                rows: Vec::new(),
                old: Vec::new(),
                dirty: false,
                read_only: false,
            });
        }
        let (file_lang, rows, old) = basedb::read_rows_and_old(path)?;
        let lang = if file_lang.is_empty() {
            lang.to_string()
        } else {
            file_lang
        };
        Ok(DictDoc {
            path: path.to_path_buf(),
            lang,
            rows,
            old,
            dirty: false,
            read_only: false,
        })
    }

    pub fn name(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string()
    }

    /// The row with the same source — and the same signature when the
    /// pair carries one (text-only pairs compare on the text alone).
    pub fn find(&self, row: &BaseRow) -> Option<usize> {
        let src = row.source.trim();
        self.rows.iter().position(|r| {
            r.source.trim() == src
                && r.rec == row.rec
                && r.field == row.field
                && (row.rec.is_empty()
                    || (r.formid == row.formid && r.edid.eq_ignore_ascii_case(&row.edid)))
        })
    }

    /// Add a pair, or replace the one with the same source.
    pub fn put(&mut self, row: BaseRow) -> Put {
        match self.find(&row) {
            Some(i) if self.rows[i].trans == row.trans && self.rows[i].status == row.status => {
                Put::Same
            }
            Some(i) => {
                self.rows[i] = row;
                self.dirty = true;
                Put::Replaced
            }
            None => {
                self.rows.push(row);
                self.dirty = true;
                Put::Added
            }
        }
    }

    pub fn remove(&mut self, i: usize) -> Option<BaseRow> {
        if i < self.rows.len() {
            self.dirty = true;
            Some(self.rows.remove(i))
        } else {
            None
        }
    }

    /// Write the file (format 3) with `copies` timestamped copies.
    pub fn save(&mut self, copies: usize) -> Result<usize, String> {
        self.save_as(&self.path.clone(), copies)
    }

    pub fn save_as(&mut self, path: &Path, copies: usize) -> Result<usize, String> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let n = basedb::write_ximdb_full(
            self.rows.clone(),
            self.old.clone(),
            &self.lang,
            path,
            copies,
        )?;
        self.path = path.to_path_buf();
        self.dirty = false;
        self.read_only = false;
        Ok(n)
    }

    /// Remove an old row (B-07).
    pub fn remove_old(&mut self, i: usize) -> Option<basedb::OldRow> {
        if i < self.old.len() {
            self.dirty = true;
            Some(self.old.remove(i))
        } else {
            None
        }
    }

    /// Put an old row back among the current ones (B-07).
    pub fn restore_old(&mut self, i: usize) -> bool {
        match self.remove_old(i) {
            Some(o) => {
                self.put(o.row);
                true
            }
            None => false,
        }
    }

    /// Old rows whose source or translation contains `needle`.
    pub fn filter_old(&self, needle: &str) -> Vec<usize> {
        let needle = needle.trim().to_lowercase();
        self.old
            .iter()
            .enumerate()
            .filter(|(_, o)| {
                needle.is_empty()
                    || o.row.source.to_lowercase().contains(&needle)
                    || o.row.trans.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Rows whose source or translation contains `needle` (case-insensitive),
    /// in file order; every row when `needle` is empty.
    pub fn filter(&self, needle: &str) -> Vec<usize> {
        let needle = needle.trim().to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                needle.is_empty()
                    || r.source.to_lowercase().contains(&needle)
                    || r.trans.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

/// A text-only pair (B-05), the shape of a personal entry by default.
pub fn text_pair(source: &str, trans: &str) -> BaseRow {
    BaseRow {
        rec: String::new(),
        field: String::new(),
        formid: 0,
        edid: String::new(),
        source: source.trim().to_string(),
        trans: trans.trim().to_string(),
        status: None,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ximod-dictedit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// T-B04-03: add, replace, same, remove, save and reload.
    #[test]
    fn put_remove_save_round_trip() {
        let path = tmp("personal_ENG-FRA.ximdb");
        let _ = std::fs::remove_file(&path);
        let mut d = DictDoc::load(&path, "fra").unwrap();
        assert!(d.rows.is_empty() && !d.dirty);
        assert_eq!(d.put(text_pair("Dwemer", "Dwemer")), Put::Added);
        assert_eq!(
            d.put(text_pair(" Dwemer ", "Dwemer")),
            Put::Same,
            "edges trimmed"
        );
        assert_eq!(d.put(text_pair("Dwemer", "Dwémer")), Put::Replaced);
        assert_eq!(d.rows.len(), 1);
        // A signed pair is another row than the text-only one.
        let signed = BaseRow {
            rec: "MISC".into(),
            field: "FULL".into(),
            formid: 7,
            edid: "DwemerIngot".into(),
            source: "Dwemer".into(),
            trans: "Dwemer (lingot)".into(),
            status: None,
            ..Default::default()
        };
        assert_eq!(d.put(signed.clone()), Put::Added);
        assert_eq!(d.put(signed), Put::Same);
        assert_eq!(d.filter("dwé"), vec![0]);
        assert_eq!(d.filter("").len(), 2);
        assert_eq!(d.save(0).unwrap(), 2);
        assert!(!d.dirty);
        let back = DictDoc::load(&path, "fra").unwrap();
        assert_eq!(back.rows, d.rows);
        assert_eq!(back.lang, "fra");
        let mut back = back;
        assert!(back.remove(0).is_some());
        assert!(back.dirty && back.rows.len() == 1);
        assert!(back.remove(5).is_none());
        let _ = std::fs::remove_file(&path);
    }
}
