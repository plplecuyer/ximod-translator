//! Bethesda plugin (.esp/.esm/.esl) reader.
//!
//! Walks the TES4-family binary layout — top-level records and (possibly
//! nested) GRUP containers — parsing each record's subrecords, transparently
//! zlib-decompressing compressed records and honouring the `XXXX` large-field
//! escape.
//!
//! Vendored from XIMOD Translator (`src/plugin/reader.rs`), trimmed to the
//! `parse` path only (the translation-memory extraction lives in XIMOD and is
//! not needed by the explorer).

use std::io::Read;

use anyhow::{Result, bail};
use flate2::read::ZlibDecoder;

use super::games::Game;
use super::record::{FLAG_COMPRESSED, FLAG_LOCALIZED, Plugin, Record, SubRecord};

/// Modern TES4 record/GRUP header length (Skyrim, Fallout, Starfield).
const HEADER_LEN_MODERN: usize = 24;
/// Oblivion's TES4 record/GRUP header length (no form-version field).
const HEADER_LEN_OBLIVION: usize = 20;

fn read_u16(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}
fn read_u32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

/// Determine the record/GRUP header length by locating the TES4 record's first
/// subrecord, which is always `HEDR`. In a modern plugin it sits at offset 24, in
/// an Oblivion plugin at offset 20 (the header lacks the 4-byte form-version /
/// version-control tail). This makes the 20-vs-24 choice independently of any
/// game selection — the whole point being that detection is automatic.
fn detect_header_len(data: &[u8]) -> usize {
    if data.len() >= 24 && &data[20..24] == b"HEDR" {
        HEADER_LEN_OBLIVION
    } else {
        // Modern layout (HEDR at 24), and the safe default for anything else.
        HEADER_LEN_MODERN
    }
}

/// Parse a whole plugin from bytes, detecting the format regime and game.
pub fn parse(data: &[u8]) -> Result<Plugin> {
    let mut plugin = Plugin::default();
    if data.len() < 16 {
        bail!("file too short to be a plugin");
    }

    let magic = sig4(&data[0..4]);
    if magic == "TES3" {
        // Morrowind is a fundamentally different container (16-byte headers,
        // 4-byte subrecord sizes, no GRUP, no FormID): hand it to the TES3 parser.
        return super::tes3::parse_tes3(data);
    }

    let header_len = detect_header_len(data);
    plugin.header_len = header_len;
    // The form version lives at offset 20 of a record header — only present in
    // the modern 24-byte layout (absent from Oblivion's 20-byte header).
    let form_version = if header_len == HEADER_LEN_MODERN && data.len() >= 22 {
        Some(read_u16(data, 20))
    } else {
        None
    };
    plugin.form_version = form_version;

    walk(data, 0, header_len, &mut plugin)?;

    // Now that the masters are known, resolve the game automatically.
    plugin.game = Game::detect(&magic, &plugin.masters, form_version);
    Ok(plugin)
}

/// Recursively walk a slice of top-level entries (whole file, or a GRUP's body).
fn walk(data: &[u8], depth: u32, header_len: usize, plugin: &mut Plugin) -> Result<()> {
    if depth > 32 {
        bail!("plugin group nesting too deep (possible corruption)");
    }
    let mut p = 0usize;
    while p + header_len <= data.len() {
        let sig = sig4(&data[p..p + 4]);
        let size = read_u32(data, p + 4) as usize;

        if sig == "GRUP" {
            // GRUP size includes the header (same length as a record header).
            let total = size;
            if total < header_len || p + total > data.len() {
                break;
            }
            let body = &data[p + header_len..p + total];
            walk(body, depth + 1, header_len, plugin)?;
            p += total;
        } else {
            // Record: size is the data length after the header.
            let data_start = p + header_len;
            let data_end = data_start + size;
            if data_end > data.len() {
                break;
            }
            let flags = read_u32(data, p + 8);
            let formid = read_u32(data, p + 12);
            let raw = &data[data_start..data_end];

            let field_bytes = if flags & FLAG_COMPRESSED != 0 {
                decompress_record(raw)?
            } else {
                raw.to_vec()
            };
            let subrecords = parse_subrecords(&field_bytes);
            let record = Record {
                sig: sig.clone(),
                formid,
                flags,
                subrecords,
            };

            if sig == "TES4" {
                plugin.localized = flags & FLAG_LOCALIZED != 0;
                for s in &record.subrecords {
                    match s.sig.as_str() {
                        "MAST" => plugin.masters.push(s.as_zstring()),
                        "CNAM" => plugin.author = s.as_zstring(),
                        _ => {}
                    }
                }
            }

            plugin.records.push(record);
            p += header_len + size;
        }
    }
    Ok(())
}

/// Decompress a compressed record body: u32 decompressed size + zlib stream.
pub(crate) fn decompress_record(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.len() < 4 {
        bail!("compressed record too short");
    }
    let decomp_size = read_u32(raw, 0) as usize;
    let mut out = Vec::with_capacity(decomp_size);
    ZlibDecoder::new(&raw[4..]).read_to_end(&mut out)?;
    Ok(out)
}

/// Parse a record's field stream into subrecords, honouring the `XXXX` escape
/// used when a field is larger than a u16.
pub(crate) fn parse_subrecords(data: &[u8]) -> Vec<SubRecord> {
    let mut out = Vec::new();
    let mut p = 0usize;
    let mut override_size: Option<usize> = None;
    while p + 6 <= data.len() {
        let sig = sig4(&data[p..p + 4]);
        let declared = read_u16(data, p + 4) as usize;
        p += 6;
        if sig == "XXXX" {
            // The 4-byte payload is the real size of the next subrecord.
            if p + 4 <= data.len() {
                override_size = Some(read_u32(data, p) as usize);
            }
            p += declared;
            continue;
        }
        let real = override_size.take().unwrap_or(declared);
        if p + real > data.len() {
            break;
        }
        out.push(SubRecord {
            sig,
            data: data[p..p + real].to_vec(),
        });
        p += real;
    }
    out
}

fn sig4(b: &[u8]) -> String {
    b.iter().take(4).map(|&c| c as char).collect()
}
