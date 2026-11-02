//! Writing the mod's own dictionary (B-03, D-02) — the interface after a
//! save, an export or on request, and the command line (D-06) on
//! `--write-dict`: the same rows, the same old data, the same header.

use std::path::Path;

use crate::basedb::{BaseRow, HeaderExtra};
use crate::plugin_id::Ident;
use crate::tm::TmEntry;

/// The rows of the working dictionary: every line that is not « À
/// traduire » — translated, whatever its status, or Ignoré — with its
/// status; the lines never shown in game are left out.
pub fn rows(entries: &[TmEntry]) -> Vec<BaseRow> {
    entries
        .iter()
        // The VMAD lines as they are under the file's switch (C-08):
        // the dictionary keeps the work, the header the switch.
        .map(crate::vmad_lock::without_file_lock)
        .filter(|e| !e.unused && crate::status::derive(e) != crate::status::Status::ToTranslate)
        .map(|e| BaseRow::from_entry(&e, true))
        .collect()
}

/// What a write needs besides the lines.
pub struct Write<'a> {
    pub entries: &'a [TmEntry],
    /// The rules learned on the mod and those off (B-18).
    pub rules: Vec<crate::exclusions::Rule>,
    pub rules_off: Vec<String>,
    /// Forced pages of reading and writing (D-09).
    pub enc_in: Option<String>,
    pub enc_out: Option<String>,
    /// The target language.
    pub lang: &'a str,
    pub out: &'a Path,
    /// The identity of the plugin as opened (R-B12-01).
    pub ident: Option<&'a Ident>,
    /// Timestamped copies kept (B-11).
    pub backup_copies: usize,
    /// Old data kept that many months (0: forever, B-07).
    pub old_months: u32,
}

/// Write the dictionary; the number of rows written. The old data
/// (B-07): what the previous file had and the document no longer has,
/// dated today.
pub fn write(w: &Write) -> Result<usize, String> {
    let rows = rows(w.entries);
    let extra = HeaderExtra {
        enc_in: w.enc_in.clone(),
        enc_out: w.enc_out.clone(),
        vmad: crate::vmad_lock::state(w.entries).or(crate::basedb::peek_extra(w.out).vmad),
        rules: Some(w.rules.clone()),
        rules_off: Some(w.rules_off.clone()),
    };
    if let Some(parent) = w.out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let old = match crate::slow_io("mod dictionary read", || {
        crate::basedb::read_rows_and_old(w.out)
    }) {
        Ok((_, prev_rows, prev_old)) => crate::basedb::roll_old_data(
            &prev_rows,
            prev_old,
            &rows,
            &crate::backup::today(),
            w.old_months,
        ),
        Err(_) => Vec::new(),
    };
    crate::slow_io("mod dictionary write", || match w.ident {
        Some(id) => {
            let id = Ident {
                written: crate::backup::now_secs(),
                ..id.clone()
            };
            crate::basedb::write_ximdb_with(
                rows,
                old,
                w.lang,
                w.out,
                w.backup_copies,
                Some(&id),
                &extra,
            )
        }
        None => crate::basedb::write_ximdb_with(
            rows,
            old,
            w.lang,
            w.out,
            w.backup_copies,
            crate::basedb::peek_ident(w.out).as_ref(),
            &extra,
        ),
    })
}
