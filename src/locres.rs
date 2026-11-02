//! Import of Unreal Engine localization exported as CSV by **UEExtractor**
//! (<https://github.com/SolicenTEAM/UEExtractor>, MIT). The tool unpacks a game's
//! `.pak`/`.utoc` archives, reads each `Game.locres`, and writes a CSV with the
//! columns `key,source,Translation` — one row per localization entry, the `key`
//! being either a bare hash or a composite `Namespace::Key`, the `source` the
//! text for that CSV's language, and `Translation` left empty.
//!
//! This is how the base-game text of Oblivion Remastered is obtained: its strings
//! live in UE5 `.locres` files inside the paks, not in STRINGS files or inline in
//! the ESM, so the usual ximod extractors do not apply. To build a game
//! dictionary we take two such CSVs — one for the source language (e.g. English)
//! and one for the target language (e.g. French) — and pair their rows by `key`,
//! which is identical across languages.
//!
//! The CSV escaping mirrors UEExtractor's `LocresHelper`: real line breaks are
//! stored as the two characters `\n` (and `\r`, `\t`), a CRLF as `<cf>`; that is
//! undone here so the dictionary holds the real text. CSV-level quote doubling
//! (`""`) is handled by the parser itself.

use std::collections::HashMap;
use std::path::Path;

/// Parse a UEExtractor CSV into `(key, source_text)` pairs (the `Translation`
/// column is ignored). The `source_text` is fully un-escaped. Header, blank and
/// `#` comment lines are skipped. Fails only when the file cannot be read.
pub fn parse_csv(path: &Path) -> Result<Vec<(String, String)>, String> {
    let bytes = std::fs::read(path).map_err(|e| crate::l10n!("err-csv-read", error = e))?;
    // UEExtractor writes UTF-8 (with a BOM); decode leniently just in case.
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

    let mut out = Vec::new();
    for record in Rfc4180Records::new(text) {
        if record.len() < 2 {
            continue; // comment/footer line or malformed
        }
        let key = record[0].as_str();
        let source = &record[1];
        // Skip the header row and empty keys/sources.
        if key.is_empty() || source.is_empty() {
            continue;
        }
        if key == "key" && source == "source" {
            continue;
        }
        out.push((key.to_string(), unescape(source)));
    }
    Ok(out)
}

/// Parse a FModel locres JSON export into `(key, text)` pairs. The file is a
/// two-level object `{ "Namespace": { "Key": "text", … }, … }`; the composite
/// key `Namespace::Key` is used so it matches the CSV convention. JSON already
/// carries real control characters, so no un-escaping is needed.
pub fn parse_json(path: &Path) -> Result<Vec<(String, String)>, String> {
    let bytes = std::fs::read(path).map_err(|e| crate::l10n!("err-json-read", error = e))?;
    let root: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| crate::l10n!("err-json-invalid", error = e))?;
    let obj = root
        .as_object()
        .ok_or_else(|| crate::l10n!("err-locres-json"))?;

    let mut out = Vec::new();
    for (ns, entries) in obj {
        let Some(kv) = entries.as_object() else {
            continue; // ignore anything that is not a namespace → {key: text} map
        };
        for (key, value) in kv {
            if let Some(text) = value.as_str()
                && !text.is_empty()
            {
                out.push((format!("{ns}::{key}"), text.to_string()));
            }
        }
    }
    Ok(out)
}

/// Parse a locres export by extension: `.json` → FModel format, anything else
/// (typically `.csv`) → UEExtractor format.
pub fn parse_file(path: &Path) -> Result<Vec<(String, String)>, String> {
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        == Some(true);
    if is_json {
        parse_json(path)
    } else {
        parse_csv(path)
    }
}

/// Build `(source, translation)` pairs from a source-language export and a
/// target-language export (each a UEExtractor CSV or a FModel JSON), joined on
/// the shared `key`. Only keys present in both, with a non-empty translation
/// that differs from the source, are kept. Both files should be the same format
/// so their key representations match.
pub fn pair_files(src: &Path, tgt: &Path) -> Result<Vec<(String, String)>, String> {
    let src = parse_file(src)?;
    let tgt = parse_file(tgt)?;

    // Index the target language by key. The first value for a key wins (the
    // exporters emit distinct keys, so collisions are not expected).
    let mut tgt_by_key: HashMap<&str, &str> = HashMap::with_capacity(tgt.len());
    for (k, v) in &tgt {
        tgt_by_key.entry(k.as_str()).or_insert(v.as_str());
    }

    let mut rows = Vec::new();
    for (k, source) in &src {
        if let Some(trans) = tgt_by_key.get(k.as_str()) {
            let trans = *trans;
            if !trans.is_empty() && trans != source {
                rows.push((source.clone(), trans.to_string()));
            }
        }
    }
    Ok(rows)
}

/// Reverse UEExtractor's `LocresHelper.EscapeKey`: turn the escaped markers back
/// into real control characters. CSV quote-doubling is already undone by the
/// parser, so it is not repeated here.
fn unescape(s: &str) -> String {
    if !s.contains('\\') && !s.contains("<cf>") {
        return s.to_string();
    }
    s.replace("<cf>", "\r\n")
        .replace("\\n", "\n")
        .replace("\\r", "\r")
        .replace("\\t", "\t")
}

/// A minimal RFC 4180 CSV record iterator: comma-separated fields, `"`-quoted
/// fields may contain commas, newlines and doubled quotes (`""` → `"`). Records
/// are separated by LF or CRLF. Good enough for UEExtractor output (which also
/// escapes real newlines, so quoted fields rarely span physical lines).
struct Rfc4180Records<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Rfc4180Records<'a> {
    fn new(text: &'a str) -> Self {
        Rfc4180Records {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }
}

impl Iterator for Rfc4180Records<'_> {
    type Item = Vec<String>;

    fn next(&mut self) -> Option<Vec<String>> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let mut fields: Vec<String> = Vec::new();
        let mut field = String::new();
        let mut in_quotes = false;

        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if in_quotes {
                if c == b'"' {
                    // A doubled quote is an escaped quote; a single one ends the field.
                    if self.pos + 1 < self.bytes.len() && self.bytes[self.pos + 1] == b'"' {
                        field.push('"');
                        self.pos += 2;
                    } else {
                        in_quotes = false;
                        self.pos += 1;
                    }
                } else {
                    push_byte(&mut field, self.bytes, &mut self.pos);
                }
            } else {
                match c {
                    b'"' => {
                        in_quotes = true;
                        self.pos += 1;
                    }
                    b',' => {
                        fields.push(std::mem::take(&mut field));
                        self.pos += 1;
                    }
                    b'\n' => {
                        self.pos += 1;
                        fields.push(std::mem::take(&mut field));
                        return Some(fields);
                    }
                    b'\r' => {
                        // Consume CR (and a following LF) as one record separator.
                        self.pos += 1;
                        if self.pos < self.bytes.len() && self.bytes[self.pos] == b'\n' {
                            self.pos += 1;
                        }
                        fields.push(std::mem::take(&mut field));
                        return Some(fields);
                    }
                    _ => push_byte(&mut field, self.bytes, &mut self.pos),
                }
            }
        }
        fields.push(field);
        Some(fields)
    }
}

/// Append the (possibly multi-byte UTF-8) character starting at `*pos` to `field`
/// and advance `*pos` past it. Falls back to a lossy byte on invalid UTF-8.
fn push_byte(field: &mut String, bytes: &[u8], pos: &mut usize) {
    let start = *pos;
    let len = utf8_len(bytes[start]);
    let end = (start + len).min(bytes.len());
    match std::str::from_utf8(&bytes[start..end]) {
        Ok(s) => field.push_str(s),
        Err(_) => field.push(char::REPLACEMENT_CHARACTER),
    }
    *pos = end;
}

/// Length in bytes of a UTF-8 sequence from its leading byte (1 on error).
fn utf8_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(name: &str, content: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("ximod_locres_test_{name}"));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        p
    }

    #[test]
    fn parses_basic_rows_and_skips_header() {
        let csv = "\u{feff}key,source,Translation\n\
                   006D9B164CCF0C80C813E0AD70BB546C,LOW,\n\
                   018998E14DE1D62B0D6315BDBEEE6A56,Use Force,\n";
        let p = write_tmp("basic.csv", csv);
        let rows = parse_csv(&p).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            ("006D9B164CCF0C80C813E0AD70BB546C".into(), "LOW".into())
        );
        assert_eq!(rows[1].1, "Use Force");
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn handles_quotes_commas_and_escapes() {
        // A quoted field with a comma, doubled quotes, and an escaped newline.
        let csv = "key,source,Translation\n\
                   K1,\"The post process component has a separate \"\"Enabled\"\" flag,\\n next line\",\n";
        let p = write_tmp("quotes.csv", csv);
        let rows = parse_csv(&p).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].1,
            "The post process component has a separate \"Enabled\" flag,\n next line"
        );
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn crlf_marker_becomes_real_crlf() {
        let csv = "key,source,Translation\nK1,line1<cf>line2,\n";
        let p = write_tmp("cf.csv", csv);
        let rows = parse_csv(&p).unwrap();
        assert_eq!(rows[0].1, "line1\r\nline2");
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn pairs_two_languages_on_key() {
        let en = write_tmp(
            "en.csv",
            "key,source,Translation\nK1,Sword,\nK2,Shield,\nUI::Only,English only,\n",
        );
        let fr = write_tmp(
            "fr.csv",
            "key,source,Translation\nK1,Épée,\nK2,Shield,\nK3,Orphan,\n",
        );
        let rows = pair_files(&en, &fr).unwrap();
        // K1 differs → kept; K2 identical (untranslated) → skipped; UI::Only has
        // no target → skipped; K3 has no source → skipped.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], ("Sword".to_string(), "Épée".to_string()));
        let _ = std::fs::remove_file(en);
        let _ = std::fs::remove_file(fr);
    }

    #[test]
    fn parses_fmodel_json_and_pairs() {
        let en = write_tmp(
            "en.json",
            r#"{"ST_FullNames":{"K1":"Sword","K2":"Shield"},
                "ST_Descriptions":{"D1":"A blade.\nSharp."}}"#,
        );
        let de = write_tmp(
            "de.json",
            r#"{"ST_FullNames":{"K1":"Schwert","K2":"Shield"},
                "ST_Descriptions":{"D1":"Eine Klinge.\nScharf."}}"#,
        );
        let parsed = parse_json(&en).unwrap();
        assert_eq!(parsed.len(), 3);
        // Composite key and real newline from JSON escaping.
        assert!(
            parsed
                .iter()
                .any(|(k, v)| k == "ST_Descriptions::D1" && v.contains('\n'))
        );

        let rows = pair_files(&en, &de).unwrap();
        // K1 and D1 differ → kept; K2 identical → skipped.
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&("Sword".to_string(), "Schwert".to_string())));
        let _ = std::fs::remove_file(en);
        let _ = std::fs::remove_file(de);
    }
}
