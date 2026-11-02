//! Common translation-memory model shared by every format.
//!
//! This is the Rust port of the unified entry/meta model used by the reference
//! `xmod_tm` utility: a single in-memory representation into which SST, EET,
//! DSD and the three XML dialects are all read, and from which they are all
//! written. Fields that only exist in one format are carried through so that a
//! round-trip in the *same* format is faithful.

use serde::{Deserialize, Serialize};

/// The eight `sStrParam` status bits of an xTranslator SST record, in bit order.
pub const STATUS_BITS: [&str; 8] = [
    "translated",
    "lockedTrans",
    "incompleteTrans",
    "validated",
    "deprecated1",
    "deprecated2",
    "oldData",
    "pending",
];

/// One translatable string, in the tool's common representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TmEntry {
    /// Original (source-language) text.
    #[serde(default)]
    pub source: String,
    /// Translated (target-language) text.
    #[serde(default)]
    pub trans: String,
    /// Record FormID (0 when unknown, e.g. xTranslator XML).
    #[serde(default)]
    pub formid: u32,
    /// Editor ID (empty when the format does not store it, e.g. binary SST).
    #[serde(default)]
    pub edid: String,
    /// Record signature, e.g. "MESG".
    #[serde(default)]
    pub rec_sig: String,
    /// Subrecord / field signature, e.g. "FULL".
    #[serde(default)]
    pub field_sig: String,
    /// Author comment (EET only).
    #[serde(default)]
    pub comment: String,
    /// 1-based instance index within an (EDID, field) group.
    #[serde(default = "default_index")]
    pub index: i32,
    /// EET string-table id (-1 when unused).
    #[serde(default = "minus_one")]
    pub idstexte: i32,
    /// EET icon id (-1 when unused).
    #[serde(default = "minus_one")]
    pub icon: i32,
    /// EET "PERSO" custom field.
    #[serde(default)]
    pub perso: String,
    /// SST list index (0 STRINGS / 1 DLSTRINGS / 2 ILSTRINGS).
    #[serde(default)]
    pub list_index: u8,
    /// SST max index within the group.
    #[serde(default)]
    pub index_max: u16,
    /// SST record hash (xTranslator's `rHash`).
    #[serde(default)]
    pub rhash: u32,
    /// SST collaboration id.
    #[serde(default)]
    pub colab_id: u8,
    /// SST string id.
    #[serde(default)]
    pub strid: i32,
    /// EET status code (99 = translated).
    #[serde(default = "minus_one_16")]
    pub status_eet: i16,
    /// SST status bit names (see [`STATUS_BITS`]).
    #[serde(default)]
    pub status_sst: Vec<String>,
    /// DSD source-plugin file name (owner of the FormID).
    #[serde(default)]
    pub plugin: String,
    /// DSD status string, if it came from a DSD file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dsd_status: Option<String>,
    /// « Jamais affichée en jeu » (field mode `unused`, A-06): listed in red
    /// with the status Ignoré, hidden unless the filter bar shows them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unused: bool,
    /// Raw record bytes (hex) preserved from an EET source for byte-exact
    /// rewriting. `None` for every other source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eet_raw: Option<String>,
    /// Where the translation came from (B-09), coded ([`crate::provenance`]);
    /// empty without translation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prov: String,
    /// The reviewer's text (A-17); empty when the line was not reviewed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub review: String,
}

fn default_index() -> i32 {
    1
}
fn minus_one() -> i32 {
    -1
}
fn minus_one_16() -> i16 {
    -1
}

impl Default for TmEntry {
    fn default() -> Self {
        Self {
            source: String::new(),
            trans: String::new(),
            formid: 0,
            edid: String::new(),
            rec_sig: String::new(),
            field_sig: String::new(),
            comment: String::new(),
            index: 1,
            idstexte: -1,
            icon: -1,
            perso: String::new(),
            list_index: 0,
            index_max: 0,
            rhash: 0,
            colab_id: 0,
            strid: 0,
            status_eet: -1,
            status_sst: Vec::new(),
            plugin: String::new(),
            dsd_status: None,
            unused: false,
            eet_raw: None,
            prov: String::new(),
            review: String::new(),
        }
    }
}

impl TmEntry {
    /// True when the entry carries an effective translation (non-empty and
    /// different from the source) — the DSD inclusion rule.
    pub fn is_translated(&self) -> bool {
        !self.trans.is_empty() && self.trans != self.source
    }

    /// Fill the SST/EET status fields consistently from a "translated" flag.
    pub fn set_translated_flags(&mut self, translated: bool) {
        if translated {
            self.status_eet = 99;
            if self.status_sst.is_empty() {
                self.status_sst.push("translated".to_string());
            }
        } else {
            self.status_eet = -1;
            self.status_sst.clear();
        }
    }
}

/// File-level metadata shared across formats.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TmMeta {
    /// EET game name ("Skyrim", "Starfield"…).
    #[serde(default)]
    pub game: String,
    /// Master plugins referenced by the translation.
    #[serde(default)]
    pub masters: Vec<String>,
    /// SST collaboration labels: `(id, label)`.
    #[serde(default)]
    pub colab_labels: Vec<(i32, String)>,
    /// Which format the data was originally read from.
    #[serde(default)]
    pub source_format: String,
    /// Raw EET header bytes (hex) for byte-exact rewriting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eet_header: Option<String>,
    /// EET header version field (default 1).
    #[serde(default = "one_i32")]
    pub eet_version: i32,
    /// EET header reserved field (default 3).
    #[serde(default = "three_i32")]
    pub eet_reserved: i32,
    /// xTranslator XML addon name (`<Params><Addon>`), also usable as plugin.
    #[serde(default)]
    pub addon: String,
    /// xTranslator XML source language (`<Params><Source>`).
    #[serde(default)]
    pub lang_source: String,
    /// xTranslator XML target language (`<Params><Dest>`).
    #[serde(default)]
    pub lang_dest: String,
    /// xTranslator XML version field (`<Params><Version>`), default 2.
    #[serde(default = "two_i32")]
    pub sstxml_version: i32,
    /// The mod's own plugin file name, if known (for DSD `form_id`).
    #[serde(default)]
    pub plugin_name: String,
    /// The text format a text document was read with (C-05).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text_format: String,
    /// The pages of code forced for the file (D-09), WHATWG names; empty =
    /// automatic.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub enc_in: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub enc_out: String,
    /// « Traduire les chaînes VMAD de ce fichier » (C-08).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vmad: bool,
    /// The VMAD blocks the reading could not follow: `(record, reason)`;
    /// kept as they are at the writing (C-08). Not saved.
    #[serde(skip)]
    pub vmad_unread: Vec<(String, String)>,
    /// The exclusion rules learned on this mod (B-18, local ones).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<crate::exclusions::Rule>,
    /// The exclusion rules turned off for this document (B-18).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules_off: Vec<String>,
}

fn one_i32() -> i32 {
    1
}
fn two_i32() -> i32 {
    2
}
fn three_i32() -> i32 {
    3
}

/// A loaded translation memory: metadata plus its entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranslationMemory {
    pub meta: TmMeta,
    pub entries: Vec<TmEntry>,
    /// The history of the translations (A-18), carried by the pivot JSON
    /// and the autosave.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<crate::history::Entry>,
}

impl TranslationMemory {
    pub fn new(entries: Vec<TmEntry>, meta: TmMeta) -> Self {
        Self {
            meta,
            entries,
            history: Vec::new(),
        }
    }
}

/// Normalized "letters and digits only" form of a string (EET cache helper).
pub fn norm_strip(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Infer an EET game name from a master list (mirrors `xmod_tm._infer_game`).
pub fn infer_game(masters: &[String]) -> String {
    let joined = masters.join(" ").to_lowercase();
    for (key, name) in [
        ("starfield", "Starfield"),
        ("fallout4", "Fallout4"),
        ("fallout76", "Fallout76"),
        ("falloutnv", "FalloutNV"),
        ("fallout3", "Fallout3"),
        ("skyrim", "Skyrim"),
        ("oblivion", "Oblivion"),
        ("morrowind", "Morrowind"),
    ] {
        if joined.contains(key) {
            return name.to_string();
        }
    }
    "Skyrim".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_norm_strip() {
        assert_eq!(norm_strip("Cure Conditions!"), "CureConditions");
    }

    #[test]
    fn test_infer_game() {
        assert_eq!(infer_game(&["Starfield.esm".into()]), "Starfield");
        assert_eq!(infer_game(&[]), "Skyrim");
    }

    #[test]
    fn test_is_translated() {
        let mut e = TmEntry {
            source: "Leave".into(),
            trans: "Leave".into(),
            ..TmEntry::default()
        };
        assert!(!e.is_translated());
        e.trans = "Partir".into();
        assert!(e.is_translated());
    }
}
