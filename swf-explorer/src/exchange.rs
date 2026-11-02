//! The translator exchange format shared by the CLI and the GUI: one [`Row`] per
//! extracted string with an editable `traduction`, read/written as CSV (`;`
//! separator, Excel-FR friendly) or JSON.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::text::TextEntry;

/// One exchange row: the entry's stable id, its metadata, the original text, and
/// the (initially empty) translation to fill in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub reinjectable: bool,
    /// Heuristic: `texte` / `code` / `incertain` (aide au tri des chaînes AS).
    pub categorie: String,
    pub cle_scaleform: bool,
    pub html: bool,
    pub contexte: String,
    pub texte: String,
    #[serde(default)]
    pub traduction: String,
}

/// Build exchange rows from extracted entries (translations start empty).
pub fn rows_from(entries: &[TextEntry]) -> Vec<Row> {
    entries
        .iter()
        .map(|e| Row {
            id: e.id.clone(),
            kind: e.kind.as_str().to_string(),
            reinjectable: e.reinjectable,
            categorie: e.category.as_str().to_string(),
            cle_scaleform: e.scaleform_key,
            html: e.html,
            contexte: e.context.clone(),
            texte: e.text.clone(),
            traduction: String::new(),
        })
        .collect()
}

fn is_json(path: &Path) -> bool {
    path.extension()
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

/// Write rows to `path` (JSON if the extension is `.json`, else `;`-CSV).
pub fn write(path: &Path, rows: &[Row]) -> Result<()> {
    if is_json(path) {
        std::fs::write(path, serde_json::to_string_pretty(rows)?)?;
    } else {
        let mut wtr = csv::WriterBuilder::new().delimiter(b';').from_path(path)?;
        for r in rows {
            wtr.serialize(r)?;
        }
        wtr.flush()?;
    }
    Ok(())
}

/// Read rows from `path` (JSON or `;`-CSV, by extension).
pub fn read(path: &Path) -> Result<Vec<Row>> {
    if is_json(path) {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    } else {
        let mut rdr = csv::ReaderBuilder::new().delimiter(b';').from_path(path)?;
        let mut rows = Vec::new();
        for r in rdr.deserialize() {
            rows.push(r?);
        }
        Ok(rows)
    }
}

/// Map of `id → new text` from rows with a non-empty translation.
pub fn edits_map(rows: &[Row]) -> HashMap<String, String> {
    rows.iter()
        .filter(|r| !r.traduction.is_empty())
        .map(|r| (r.id.clone(), r.traduction.clone()))
        .collect()
}
