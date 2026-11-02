//! Bethesda external STRINGS files (`.STRINGS` / `.DLSTRINGS` / `.ILSTRINGS`).
//!
//! Localized plugins (Skyrim SE, Fallout 4, Starfield) do not store text inline:
//! each translatable field holds a 4-byte **string id**, and the text lives in
//! three sidecar files named `<Plugin>_<lang>.STRINGS` etc. This module reads and
//! writes those files, which is what makes localized plugins translatable and
//! what the "STRINGS" output mode (CDC mode 1) produces.
//!
//! Layout of every file:
//! - header: `count: u32`, `dataSize: u32`;
//! - directory: `count` × (`id: u32`, `offset: u32`) — offset into the data blob;
//! - data blob (`dataSize` bytes): at each offset a string, either NUL-terminated
//!   (`.STRINGS`) or `u32` length-prefixed (`.DLSTRINGS` / `.ILSTRINGS`).

use anyhow::{Result, bail};

use super::encoding::{decode, encode};
use super::model::{TmEntry, TmMeta, TranslationMemory};

/// Map a file extension to a STRINGS kind, if it is one.
pub fn kind_from_extension(ext: &str) -> Option<StringsKind> {
    match ext.to_ascii_uppercase().as_str() {
        "STRINGS" => Some(StringsKind::Strings),
        "DLSTRINGS" => Some(StringsKind::DlStrings),
        "ILSTRINGS" => Some(StringsKind::IlStrings),
        _ => None,
    }
}

/// Load a STRINGS file directly into a translation memory (the strings-only
/// editing workflow, CDC mode 1). Each entry carries its id and list; `source`
/// is the file's text and `trans` starts empty (to be filled). There is no
/// record/field context — that is what the hybrid (plugin-loaded) workflow adds.
pub fn load_as_tm(bytes: &[u8], kind: StringsKind, utf8: bool) -> Result<TranslationMemory> {
    let entries = parse(bytes, kind, utf8)?
        .into_iter()
        .map(|(id, text)| TmEntry {
            source: text,
            idstexte: id as i32,
            icon: -1,
            list_index: kind.list_index(),
            index: 1,
            ..Default::default()
        })
        .collect();
    let meta = TmMeta {
        source_format: "strings".to_string(),
        ..Default::default()
    };
    Ok(TranslationMemory::new(entries, meta))
}

/// Serialize the entries of a translation memory that belong to one STRINGS kind
/// (by `list_index`) back into that file: id + text (translation when effective,
/// else the source). Used to round-trip / save a strings-loaded document.
pub fn write_from_tm(tm: &TranslationMemory, kind: StringsKind, utf8: bool) -> Vec<u8> {
    let mut entries: Vec<(u32, String)> = tm
        .entries
        .iter()
        .filter(|e| e.idstexte >= 0 && StringsKind::from_list_index(e.list_index) == kind)
        .map(|e| {
            let text = if e.is_translated() {
                e.trans.clone()
            } else {
                e.source.clone()
            };
            (e.idstexte as u32, text)
        })
        .collect();
    entries.sort_by_key(|(id, _)| *id);
    write(&entries, kind, utf8)
}

/// Which of the three files, i.e. how strings are laid out in the data blob.
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

    /// The list index used in the translation-memory model (0/1/2).
    pub fn list_index(self) -> u8 {
        match self {
            StringsKind::Strings => 0,
            StringsKind::DlStrings => 1,
            StringsKind::IlStrings => 2,
        }
    }

    pub fn from_list_index(i: u8) -> StringsKind {
        match i {
            1 => StringsKind::DlStrings,
            2 => StringsKind::IlStrings,
            _ => StringsKind::Strings,
        }
    }

    /// The uppercase file extension for this kind.
    pub fn extension(self) -> &'static str {
        match self {
            StringsKind::Strings => "STRINGS",
            StringsKind::DlStrings => "DLSTRINGS",
            StringsKind::IlStrings => "ILSTRINGS",
        }
    }
}

fn read_u32(d: &[u8], p: usize) -> Result<u32> {
    if p + 4 > d.len() {
        bail!(crate::l10n!("err-data-end", offset = p));
    }
    Ok(u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]))
}

/// Parse a STRINGS file into `(id, text)` pairs.
pub fn parse(bytes: &[u8], kind: StringsKind, utf8: bool) -> Result<Vec<(u32, String)>> {
    if bytes.len() < 8 {
        bail!(crate::l10n!("err-strings-short"));
    }
    let count = read_u32(bytes, 0)? as usize;
    let data_size = read_u32(bytes, 4)? as usize;
    let dir_start = 8;
    let data_start = dir_start + count * 8;
    if data_start + data_size > bytes.len() {
        bail!(crate::l10n!("err-strings-short"));
    }
    let data = &bytes[data_start..data_start + data_size];

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let id = read_u32(bytes, dir_start + i * 8)?;
        let off = read_u32(bytes, dir_start + i * 8 + 4)? as usize;
        if off > data.len() {
            bail!(crate::l10n!("err-strings-entry", id = id));
        }
        let raw = if kind.length_prefixed() {
            let len = read_u32(data, off)? as usize;
            let s = off + 4;
            if s + len > data.len() {
                bail!(crate::l10n!("err-strings-entry", id = id));
            }
            // Length includes the trailing NUL.
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

/// Build a STRINGS file from `(id, text)` pairs. Strings are laid out
/// sequentially in the order given.
pub fn write(entries: &[(u32, String)], kind: StringsKind, utf8: bool) -> Vec<u8> {
    let mut dir = Vec::with_capacity(entries.len() * 8);
    let mut data = Vec::new();
    for (id, s) in entries {
        let offset = data.len() as u32;
        dir.extend_from_slice(&id.to_le_bytes());
        dir.extend_from_slice(&offset.to_le_bytes());
        let mut enc = encode(s, utf8);
        enc.push(0); // NUL terminator
        if kind.length_prefixed() {
            data.extend_from_slice(&(enc.len() as u32).to_le_bytes());
        }
        data.extend_from_slice(&enc);
    }
    let mut out = Vec::with_capacity(8 + dir.len() + data.len());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&dir);
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_strings() {
        let entries = vec![
            (1u32, "Iron Sword".to_string()),
            (2, "Steel Dagger".to_string()),
            (7, "Épée en argent".to_string()),
        ];
        for kind in [
            StringsKind::Strings,
            StringsKind::DlStrings,
            StringsKind::IlStrings,
        ] {
            let bytes = write(&entries, kind, false);
            let back = parse(&bytes, kind, false).unwrap();
            assert_eq!(back, entries, "{:?} round-trip", kind);
        }
    }

    #[test]
    fn roundtrip_utf8() {
        let entries = vec![(10u32, "日本語のテキスト".to_string())];
        let bytes = write(&entries, StringsKind::DlStrings, true);
        let back = parse(&bytes, StringsKind::DlStrings, true).unwrap();
        assert_eq!(back, entries);
    }

    #[test]
    fn tm_import_edit_export_roundtrip() {
        // Strings-only workflow (mode 1): load a .STRINGS into a TM, translate
        // one entry, write it back — the untranslated one keeps its source.
        let file = write(
            &[(1u32, "Iron Sword".into()), (2u32, "Steel Dagger".into())],
            StringsKind::Strings,
            true,
        );
        let mut tm = load_as_tm(&file, StringsKind::Strings, true).unwrap();
        assert_eq!(tm.meta.source_format, "strings");
        assert_eq!(tm.entries.len(), 2);
        assert!(
            tm.entries
                .iter()
                .all(|e| e.idstexte >= 0 && e.trans.is_empty())
        );

        // Translate id 1 only.
        for e in &mut tm.entries {
            if e.idstexte == 1 {
                e.trans = "Épée de fer".into();
            }
        }
        let out = write_from_tm(&tm, StringsKind::Strings, true);
        let back = parse(&out, StringsKind::Strings, true).unwrap();
        assert_eq!(
            back,
            vec![
                (1u32, "Épée de fer".to_string()),  // translated
                (2u32, "Steel Dagger".to_string()), // untranslated → source kept
            ]
        );
    }
}
