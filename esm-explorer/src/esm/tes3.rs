//! Morrowind (TES3) plugin reader.
//!
//! TES3 is a different container from the TES4 family used by every later
//! Bethesda title: records have a **16-byte header** (`name[4]`, `size[4]`,
//! `header1[4]`, `flags[4]`), there are **no GRUP containers**, subrecord sizes
//! are **`u32`** (not `u16`), there is **no compression** and **no FormID**.
//!
//! Vendored from XIMOD Translator (`src/plugin/tes3.rs`), trimmed to the parse
//! path (the translation-memory extraction stays in XIMOD).

use anyhow::{Result, bail};

use super::games::Game;
use super::record::{Plugin, Record, SubRecord};

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
        bail!("not a TES3 (Morrowind) plugin");
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
                    // rest of the pipeline treats them uniformly.
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
fn zfield(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let slice = &b[..end];
    match std::str::from_utf8(slice) {
        Ok(s) => s.to_string(),
        Err(_) => slice.iter().map(|&c| c as char).collect(),
    }
}
