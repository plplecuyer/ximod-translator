//! Bethesda external STRINGS files (`.STRINGS` / `.DLSTRINGS` / `.ILSTRINGS`).
//!
//! Localized plugins (Skyrim SE, Fallout 4, Starfield) store no text inline: each
//! translatable field holds a 4-byte **string id**, and the text lives in three
//! sidecar files `<Plugin>_<lang>.STRINGS` etc. String ids are unique across the
//! whole plugin (partitioned by category into the three files), so the three can
//! be merged into a single id→text table.
//!
//! Vendored (read path only) from XIMOD Translator's `tm/strings.rs`, with a
//! self-contained decoder.

use anyhow::{Result, bail};

/// Which of the three files (controls the data-blob string layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringsKind {
    /// `.STRINGS`: NUL-terminated strings.
    Strings,
    /// `.DLSTRINGS`: `u32` length (incl. NUL) then the string.
    DlStrings,
    /// `.ILSTRINGS`: same length-prefixed layout as `.DLSTRINGS`.
    IlStrings,
}

impl StringsKind {
    fn length_prefixed(self) -> bool {
        !matches!(self, StringsKind::Strings)
    }
    pub fn extension(self) -> &'static str {
        match self {
            StringsKind::Strings => "STRINGS",
            StringsKind::DlStrings => "DLSTRINGS",
            StringsKind::IlStrings => "ILSTRINGS",
        }
    }
}

/// Map a file extension to a STRINGS kind, if it is one.
pub fn kind_from_extension(ext: &str) -> Option<StringsKind> {
    match ext.to_ascii_uppercase().as_str() {
        "STRINGS" => Some(StringsKind::Strings),
        "DLSTRINGS" => Some(StringsKind::DlStrings),
        "ILSTRINGS" => Some(StringsKind::IlStrings),
        _ => None,
    }
}

/// Decode raw string bytes. Valid UTF-8 is always decoded as UTF-8 first — this
/// makes the result correct even when the game (hence the `utf8` hint) was
/// mis-detected, which is what turned UTF-8 "Crédits" into Latin-1 "CrÃ©dits".
/// Only genuinely non-UTF-8 bytes fall back: `from_utf8_lossy` for UTF-8 engines,
/// Windows-1252/Latin-1 for the classic ones.
fn decode(raw: &[u8], utf8: bool) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        Err(_) => {
            if utf8 {
                String::from_utf8_lossy(raw).into_owned()
            } else {
                raw.iter().map(|&b| b as char).collect()
            }
        }
    }
}

fn read_u32(d: &[u8], p: usize) -> Result<u32> {
    if p + 4 > d.len() {
        bail!("STRINGS file truncated at offset {p}");
    }
    Ok(u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]))
}

/// Parse a STRINGS file into `(id, text)` pairs.
pub fn parse(bytes: &[u8], kind: StringsKind, utf8: bool) -> Result<Vec<(u32, String)>> {
    if bytes.len() < 8 {
        bail!("STRINGS file too short");
    }
    let count = read_u32(bytes, 0)? as usize;
    let data_size = read_u32(bytes, 4)? as usize;
    let dir_start = 8;
    let data_start = dir_start + count * 8;
    if data_start + data_size > bytes.len() {
        bail!("STRINGS directory/data sizes exceed file length");
    }
    let data = &bytes[data_start..data_start + data_size];

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let id = read_u32(bytes, dir_start + i * 8)?;
        let off = read_u32(bytes, dir_start + i * 8 + 4)? as usize;
        if off > data.len() {
            bail!("STRINGS entry {id} offset out of bounds");
        }
        let raw = if kind.length_prefixed() {
            let len = read_u32(data, off)? as usize;
            let s = off + 4;
            if s + len > data.len() {
                bail!("STRINGS entry {id} length out of bounds");
            }
            let end = if len > 0 { s + len - 1 } else { s };
            &data[s..end]
        } else {
            let end = data[off..]
                .iter()
                .position(|&b| b == 0)
                .map(|p| off + p)
                .unwrap_or(data.len());
            &data[off..end]
        };
        out.push((id, decode(raw, utf8)));
    }
    Ok(out)
}
