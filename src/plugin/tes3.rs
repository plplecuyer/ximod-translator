//! Morrowind (TES3) plugin reader.
//!
//! TES3 is a different container from the TES4 family used by every later
//! Bethesda title: records have a **16-byte header** (`name[4]`, `size[4]`,
//! `header1[4]`, `flags[4]`), there are **no GRUP containers**, subrecord sizes
//! are **`u32`** (not `u16`), there is **no compression** and **no FormID**.
//! Records are identified by an id subrecord (`NAME`, or `INAM` for `INFO`).
//!
//! Because so little is shared with the TES4 reader, TES3 gets its own parse and
//! extraction here; the common [`Plugin`]/[`Record`]/[`SubRecord`] model is
//! reused so the rest of the pipeline (translation memory, merge, output) is
//! agnostic to which engine produced the strings.

use anyhow::{Result, bail};

use super::record::{Plugin, Record, SubRecord};
use crate::games::{FieldCtx, FieldMode, Game, registry};
use crate::tm::{TmEntry, TmMeta, TranslationMemory};

fn read_u32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

fn sig4(b: &[u8]) -> String {
    b.iter().take(4).map(|&c| c as char).collect()
}

/// TES3 record header length in bytes.
const HEADER_LEN: usize = 16;

/// Parse a whole Morrowind plugin from bytes into the common [`Plugin`] model.
pub fn parse_tes3(data: &[u8]) -> Result<Plugin> {
    if data.len() < HEADER_LEN || &data[0..4] != b"TES3" {
        bail!(crate::l10n!("err-plugin-not-tes3"));
    }

    let mut plugin = Plugin {
        game: Game::Morrowind,
        header_len: HEADER_LEN,
        form_version: None,
        ..Default::default()
    };

    let mut p = 0usize;
    while p + HEADER_LEN <= data.len() {
        let sig = sig4(&data[p..p + 4]);
        let size = read_u32(data, p + 4) as usize;
        let flags = read_u32(data, p + 12);
        let body_start = p + HEADER_LEN;
        let body_end = body_start + size;
        if body_end > data.len() {
            break;
        }
        let mut subrecords = parse_subrecords(&data[body_start..body_end]);

        if sig == "TES3" {
            // The header record carries the masters and, inside its 300-byte
            // HEDR subrecord, the plugin author and description at fixed offsets.
            for s in &subrecords {
                if s.sig == "MAST" {
                    plugin.masters.push(s.as_zstring());
                }
            }
            if let Some(hedr) = subrecords.iter().find(|s| s.sig == "HEDR") {
                // HEDR: version f32[4], file type u32[4], author[32],
                // description[256], record count u32[4].
                if hedr.data.len() >= 296 {
                    let author = zfield(&hedr.data[8..40]);
                    let desc = zfield(&hedr.data[40..296]);
                    plugin.author = author.clone();
                    // Surface author/description as synthetic subrecords so the
                    // extractor treats them uniformly (ESP-ESM Translator keys
                    // them as TES3:CNAM / TES3:SNAM).
                    if !author.is_empty() {
                        subrecords.push(SubRecord {
                            sig: "CNAM".to_string(),
                            data: author.into_bytes(),
                        });
                    }
                    if !desc.is_empty() {
                        subrecords.push(SubRecord {
                            sig: "SNAM".to_string(),
                            data: desc.into_bytes(),
                        });
                    }
                }
            }
        }

        plugin.records.push(Record {
            sig,
            formid: 0, // TES3 has no FormID.
            flags,
            subrecords,
        });
        p = body_end;
    }

    Ok(plugin)
}

/// Parse a TES3 record body into subrecords: `name[4]`, `size[4]` (u32), data.
fn parse_subrecords(data: &[u8]) -> Vec<SubRecord> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 8 <= data.len() {
        let sig = sig4(&data[p..p + 4]);
        let size = read_u32(data, p + 4) as usize;
        p += 8;
        if p + size > data.len() {
            break;
        }
        out.push(SubRecord {
            sig,
            data: data[p..p + size].to_vec(),
        });
        p += size;
    }
    out
}

/// Decode a fixed-width, NUL-padded TES3 header string field.
/// Morrowind stores text as Windows-1252, so the fallback must decode that
/// (not raw Latin-1) to keep typographic punctuation intact.
fn zfield(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let slice = &b[..end];
    crate::tm::encoding::decode(slice, false)
}

/// Extract translatable strings from a parsed Morrowind plugin.
///
/// Differences from the TES4 extractor: there is no FormID (`formid` stays 0, so
/// the merge falls back to editor-id/text matching); the record id used for
/// matching is `INAM` for `INFO` records and `NAME` otherwise; and within a
/// `CELL` only the leading `NAME` is the cell's own name — every `NAME` after the
/// first `FRMR` reference block is a referenced object id, not translatable text.
pub fn extract_tes3(plugin: &Plugin, plugin_name: &str) -> TranslationMemory {
    let mut entries = Vec::new();
    let reg = registry();
    let fields = reg.fields(Game::Morrowind);
    for rec in &plugin.records {
        // The record's identifier: INFO topics key off INAM; everything else
        // off its NAME (the object id).
        let id_sig = if rec.sig == "INFO" { "INAM" } else { "NAME" };
        let edid = rec
            .subrecords
            .iter()
            .find(|s| s.sig == id_sig)
            .map(|s| s.as_zstring())
            .unwrap_or_default();

        let mut counters: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
        let mut seen_frmr = false;
        for sub in &rec.subrecords {
            if sub.sig == "FRMR" {
                seen_frmr = true;
            }
            if fields.limits(&rec.sig, &sub.sig).is_none() {
                continue;
            }
            // A CELL's own name is the NAME before its first reference block;
            // later NAMEs are object references, not cell names (a rule of the
            // format, kept in the engine).
            if rec.sig == "CELL" && sub.sig == "NAME" && seen_frmr {
                continue;
            }

            let source = sub.as_zstring();
            if source.is_empty() {
                continue;
            }
            let other = |f: &str| {
                rec.subrecords
                    .iter()
                    .find(|s| s.sig == f)
                    .map(|s| s.as_zstring())
            };
            let ctx = FieldCtx {
                edid: &edid,
                value: &source,
                other: &other,
            };
            let mode = fields.mode(&rec.sig, &sub.sig, &ctx);
            if mode == FieldMode::Ignore {
                continue;
            }

            let idx = counters.entry(sub.sig.clone()).or_insert(0);
            *idx += 1;

            let mut entry = TmEntry {
                formid: 0,
                edid: edid.clone(),
                rec_sig: rec.sig.clone(),
                field_sig: sub.sig.clone(),
                source,
                index: *idx,
                idstexte: -1,
                icon: -1,
                ..Default::default()
            };
            if mode == FieldMode::Locked || mode == FieldMode::Unused {
                crate::status::set(&mut entry, crate::status::Status::Ignored);
                entry.unused = mode == FieldMode::Unused;
            }
            entries.push(entry);
        }
    }

    let meta = TmMeta {
        game: Game::Morrowind.id().to_string(),
        masters: plugin.masters.clone(),
        source_format: "plugin".to_string(),
        plugin_name: plugin_name.to_string(),
        ..Default::default()
    };
    TranslationMemory::new(entries, meta)
}
