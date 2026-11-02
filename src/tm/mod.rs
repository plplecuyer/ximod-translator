//! Translation-memory engine: the common model plus codecs for every format
//! XIMOD Translator reads and writes, and a single extension-driven dispatch.
//!
//! Formats: `.sst` (xTranslator), `.eet` (ESP-ESM Translator), `.dsd`
//! (Dynamic String Distributor JSON), the pivot `.json`, `.csv` (export only),
//! and three XML dialects selected by output suffix (`.eet.xml`, `.sst.xml`,
//! plain `.xml` = native).
//!
//! This is the Rust port of the reference `xmod_tm` utility.

pub mod apply;
pub mod binio;
pub mod dsd;
pub mod eet;
pub mod encoding;
pub mod model;
pub mod sst;
pub mod strings;
pub mod xml;

pub use model::{TmEntry, TmMeta, TranslationMemory};

use anyhow::{Result, bail};
use std::path::Path;

/// Which concrete format a file maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Sst,
    Eet,
    Dsd,
    Json,
    Csv,
    EetXml,
    SstXml,
    XimodXml,
}

/// Outcome of a write, mirroring the reference tool's status strings.
#[derive(Debug, Clone)]
pub enum WriteOutcome {
    Json,
    Csv,
    Sst,
    /// EET write and whether it was byte-exact or constructed.
    Eet(eet::EetWriteMode),
    /// DSD write with the number of entries emitted.
    Dsd(usize),
    Xml(&'static str),
    /// A single STRINGS file, with its kind name and the entry count.
    Strings(&'static str, usize),
}

/// Classify an *output* path (compound `.eet.xml` / `.sst.xml` are honoured).
pub fn output_format(path: &Path) -> Option<Format> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.ends_with(".eet.xml") {
        return Some(Format::EetXml);
    }
    if name.ends_with(".sst.xml") {
        return Some(Format::SstXml);
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .as_deref()
    {
        Some("sst") => Some(Format::Sst),
        Some("eet") => Some(Format::Eet),
        Some("dsd") => Some(Format::Dsd),
        Some("json") => Some(Format::Json),
        Some("csv") => Some(Format::Csv),
        Some("xml") => Some(Format::XimodXml),
        _ => None,
    }
}

// ------------------------------- loading -----------------------------------

/// Load a translation memory from a file, dispatching on its extension.
pub fn load(path: &Path) -> Result<TranslationMemory> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "eet" => eet::read_bytes(&std::fs::read(path)?),
        "sst" => sst::read_bytes(&std::fs::read(path)?),
        "xml" => xml::read_auto(&std::fs::read_to_string(path)?),
        "dsd" => {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
            dsd::from_value(&v)
        }
        "json" => load_json_or_dsd(&std::fs::read_to_string(path)?),
        "strings" | "dlstrings" | "ilstrings" => {
            let kind = strings::kind_from_extension(&ext).unwrap();
            // A loose STRINGS file carries no game/encoding; UTF-8 (SSE-era) is
            // the common case and the decoder falls back to Latin-1 anyway.
            strings::load_as_tm(&std::fs::read(path)?, kind, true)
        }
        other => bail!(crate::l10n!("err-format-in", ext = other)),
    }
}

/// A `.json` file may be the pivot (`{meta, entries}`) or a DSD array.
pub fn load_json_or_dsd(text: &str) -> Result<TranslationMemory> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    if v.is_array() {
        dsd::from_value(&v)
    } else {
        Ok(serde_json::from_value(v)?)
    }
}

// ------------------------------- saving ------------------------------------

/// Save a translation memory to a file, dispatching on its (compound) extension.
pub fn save(tm: &TranslationMemory, path: &Path) -> Result<WriteOutcome> {
    // STRINGS files are handled before the dictionary-format dispatch.
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if let Some(kind) = strings::kind_from_extension(&ext) {
        let bytes = strings::write_from_tm(tm, kind, true);
        let n = tm
            .entries
            .iter()
            .filter(|e| {
                e.idstexte >= 0 && strings::StringsKind::from_list_index(e.list_index) == kind
            })
            .count();
        std::fs::write(path, bytes)?;
        return Ok(WriteOutcome::Strings(kind.extension(), n));
    }
    let fmt = output_format(path)
        .ok_or_else(|| anyhow::anyhow!(crate::l10n!("err-format-out", path = path.display())))?;
    match fmt {
        Format::Json => {
            std::fs::write(path, serde_json::to_string_pretty(tm)?)?;
            Ok(WriteOutcome::Json)
        }
        Format::Csv => {
            std::fs::write(path, write_csv(tm))?;
            Ok(WriteOutcome::Csv)
        }
        Format::Sst => {
            std::fs::write(path, sst::write_bytes(tm))?;
            Ok(WriteOutcome::Sst)
        }
        Format::Eet => {
            let (bytes, mode) = eet::write_bytes(tm);
            std::fs::write(path, bytes)?;
            Ok(WriteOutcome::Eet(mode))
        }
        Format::Dsd => {
            let (value, n) = dsd::to_value(tm);
            std::fs::write(path, serde_json::to_string_pretty(&value)?)?;
            Ok(WriteOutcome::Dsd(n))
        }
        Format::EetXml => {
            std::fs::write(path, xml::write_eetxml(tm))?;
            Ok(WriteOutcome::Xml("eet"))
        }
        Format::SstXml => {
            std::fs::write(path, xml::write_sstxml(tm))?;
            Ok(WriteOutcome::Xml("sst"))
        }
        Format::XimodXml => {
            std::fs::write(path, xml::write_ximodxml(tm))?;
            Ok(WriteOutcome::Xml("ximod"))
        }
    }
}

/// Tabular preview (semicolon-separated, UTF-8 BOM for Excel).
pub fn write_csv(tm: &TranslationMemory) -> Vec<u8> {
    let mut s = String::from("\u{feff}");
    s.push_str("rec_sig;field_sig;formid;edid;status_sst;source;trans\n");
    let cell = |v: &str| v.replace('"', "\"\"");
    for e in &tm.entries {
        let quoted = |v: &str| format!("\"{}\"", cell(v));
        s.push_str(&format!(
            "{};{};{:08X};{};{};{};{}\n",
            quoted(&e.rec_sig),
            quoted(&e.field_sig),
            e.formid,
            quoted(&e.edid),
            quoted(&e.status_sst.join("|")),
            quoted(&e.source),
            quoted(&e.trans),
        ));
    }
    s.into_bytes()
}

/// High-level convert: load `src`, save to `dst`. Returns (entry count, source
/// format label, write outcome).
pub fn convert(src: &Path, dst: &Path) -> Result<(usize, String, WriteOutcome)> {
    let tm = load(src)?;
    let n = tm.entries.len();
    let srcfmt = tm.meta.source_format.clone();
    let outcome = save(&tm, dst)?;
    Ok((n, srcfmt, outcome))
}
