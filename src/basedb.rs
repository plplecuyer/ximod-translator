//! Automatic pre-translation from existing databases (CDC §6).
//!
//! On loading a mod, XIMOD looks up each line in the translation databases the
//! user has accumulated, in two dedicated folders:
//!
//! - **bases de jeux** — the official base-game translations (the user supplies
//!   EET bases, optionally consolidated into a native XIMOD base, see
//!   [`compile_ximod_base`]);
//! - **traductions de mods** — the user's own previous mod translations.
//!
//! A line is auto-translated only when a database entry shares the **same
//! indicators** — GRUP (record signature), Champ (field signature), ID (FormID)
//! and EDID — *and* the same source text (so a mod that changed a vanilla line's
//! wording is never given the stale vanilla translation). Matching lines are
//! filled and marked **Validé**.

pub mod cache;
pub use cache::{BaseDbCache, FileDbCache};

use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::status::{self, Status};
use crate::tm::{self, TmEntry, TranslationMemory};

/// Source database extensions we can import (raw translation memories).
const BASE_EXTS: &[&str] = &["eet", "sst", "dsd", "json", "xml"];

/// Extension of the compact native XIMOD base produced by [`compile_ximod_base`].
pub const XIMDB_EXT: &str = "ximdb";

/// One row of a compact XIMOD base: GRUP, Champ, FormID, EDID, source, trans
/// and, since format 2 (D-02), the status key of the line (`"validated"`,
/// `"pending"`…); since format 4 (B-09, A-17, decision D-43), an object
/// `{ "p": provenance, "r": review }` when either is set. Serialized as a
/// JSON array of six to eight elements so the file stays small; a v1 file
/// (six elements) reads back with no status.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BaseRow {
    pub rec: String,
    pub field: String,
    pub formid: u32,
    pub edid: String,
    pub source: String,
    pub trans: String,
    /// Status key ([`Status::key`]); `None` in v1 files and shared bases.
    pub status: Option<String>,
    /// Provenance code ([`crate::provenance`]) of the mod's own dictionary
    /// (format 4); empty elsewhere.
    pub prov: String,
    /// Review text (A-17) of the mod's own dictionary (format 4).
    pub review: String,
}

impl BaseRow {
    /// A row from an entry, carrying its status when `with_status` (the mod's
    /// own working dictionary, D-02) or not (a shared base).
    pub fn from_entry(e: &tm::TmEntry, with_status: bool) -> BaseRow {
        BaseRow {
            rec: e.rec_sig.clone(),
            field: e.field_sig.clone(),
            formid: e.formid,
            edid: e.edid.clone(),
            source: e.source.clone(),
            trans: e.trans.clone(),
            status: with_status.then(|| status::derive(e).key().to_string()),
            prov: if with_status {
                e.prov.clone()
            } else {
                String::new()
            },
            review: if with_status {
                e.review.clone()
            } else {
                String::new()
            },
        }
    }
}

impl Serialize for BaseRow {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let extra = !self.prov.is_empty() || !self.review.is_empty();
        let n = if extra {
            8
        } else if self.status.is_some() {
            7
        } else {
            6
        };
        let mut seq = ser.serialize_seq(Some(n))?;
        seq.serialize_element(&self.rec)?;
        seq.serialize_element(&self.field)?;
        seq.serialize_element(&self.formid)?;
        seq.serialize_element(&self.edid)?;
        seq.serialize_element(&self.source)?;
        seq.serialize_element(&self.trans)?;
        if extra {
            seq.serialize_element(&self.status)?;
            let mut m = serde_json::Map::new();
            if !self.prov.is_empty() {
                m.insert("p".into(), self.prov.clone().into());
            }
            if !self.review.is_empty() {
                m.insert("r".into(), self.review.clone().into());
            }
            seq.serialize_element(&m)?;
        } else if let Some(st) = &self.status {
            seq.serialize_element(st)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for BaseRow {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = BaseRow;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an array of 6 to 8 elements")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<BaseRow, A::Error> {
                use serde::de::Error;
                let mut next = |i: usize| -> Result<_, A::Error> {
                    seq.next_element::<serde_json::Value>()?
                        .ok_or_else(|| A::Error::invalid_length(i, &self))
                };
                let s = |v: serde_json::Value, i: usize| -> Result<String, A::Error> {
                    match v {
                        serde_json::Value::String(s) => Ok(s),
                        other => Err(A::Error::custom(format!(
                            "element {i}: string expected, got {other}"
                        ))),
                    }
                };
                let rec = s(next(0)?, 0)?;
                let field = s(next(1)?, 1)?;
                let formid = next(2)?
                    .as_u64()
                    .ok_or_else(|| A::Error::custom("element 2: FormID expected"))?
                    as u32;
                let edid = s(next(3)?, 3)?;
                let source = s(next(4)?, 4)?;
                let trans = s(next(5)?, 5)?;
                let status = match seq.next_element::<serde_json::Value>()? {
                    Some(serde_json::Value::String(st)) => Some(st),
                    Some(serde_json::Value::Null) | None => None,
                    Some(other) => {
                        return Err(A::Error::custom(format!(
                            "element 6: status expected, got {other}"
                        )));
                    }
                };
                // Format 4: provenance and review (anything else is ignored).
                let (mut prov, mut review) = (String::new(), String::new());
                if let Some(serde_json::Value::Object(m)) =
                    seq.next_element::<serde_json::Value>()?
                {
                    if let Some(serde_json::Value::String(p)) = m.get("p") {
                        prov = p.clone();
                    }
                    if let Some(serde_json::Value::String(r)) = m.get("r") {
                        review = r.clone();
                    }
                }
                // Ignore any further element (forward compatibility).
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(BaseRow {
                    rec,
                    field,
                    formid,
                    edid,
                    source,
                    trans,
                    status,
                    prov,
                    review,
                })
            }
        }
        de.deserialize_seq(V)
    }
}

/// Current on-disk format of a `.ximdb` (2: rows may carry a status, D-02).
/// Format 3 (B-05): a row may have no signature (record and field empty,
/// FormID 0, EDID empty) — a text-only pair, matched on the source text
/// whatever the field. A v2 reader accepts those rows as ordinary ones.
/// Format 4 (B-09, A-17): an eighth element `{p, r}` per row of the mod's
/// own dictionary, ignored by the older readers.
pub const XIMDB_FORMAT: u32 = 4;

/// The on-disk compact base: a version tag, the **target language** (ISO 639-3)
/// the base translates into, and the rows. Only the lookup fields (plus the
/// status) are stored, so it is an order of magnitude smaller and faster to
/// load than a full pivot memory. `lang` comes right after `v` so it can be
/// read from just the first bytes of the file (see [`peek_ximdb_lang`]).
#[derive(Serialize, Deserialize)]
struct BaseFile {
    v: u32,
    #[serde(default)]
    lang: String,
    /// The plugin the mod's dictionary was written for (B-12): name, size,
    /// CRC-32 (hexadecimal) and date of the write; optional, ignored by the
    /// readers that do not know them, read from the first bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plugin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    crc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written: Option<i64>,
    /// The settings kept per file (D-09, C-08): the forced pages of reading
    /// and writing, the VMAD switch; optional like the identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enc_in: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enc_out: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vmad: Option<bool>,
    /// The exclusion rules learned on the mod and those turned off for it
    /// (B-18), optional like the rest of the header.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rules: Vec<crate::exclusions::Rule>,
    #[serde(default, rename = "rulesOff", skip_serializing_if = "Vec::is_empty")]
    rules_off: Vec<String>,
    e: Vec<BaseRow>,
    /// Old data (B-07): the lines a mod lost at an update, with the date
    /// they disappeared; absent from a file that has none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    o: Vec<OldRow>,
}

/// A row of the old data: the row plus the day it left the document
/// (`YYYY-MM-DD`), as an eight-element array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldRow {
    pub row: BaseRow,
    pub date: String,
}

impl Serialize for OldRow {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let r = &self.row;
        let mut seq = ser.serialize_seq(Some(8))?;
        seq.serialize_element(&r.rec)?;
        seq.serialize_element(&r.field)?;
        seq.serialize_element(&r.formid)?;
        seq.serialize_element(&r.edid)?;
        seq.serialize_element(&r.source)?;
        seq.serialize_element(&r.trans)?;
        seq.serialize_element(&r.status.clone().unwrap_or_default())?;
        seq.serialize_element(&self.date)?;
        seq.end()
    }
}

impl<'de> Deserialize<'de> for OldRow {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        // Read the array as values, build the row from the first seven and
        // take the eighth as the date.
        let v: Vec<serde_json::Value> = Vec::deserialize(de)?;
        if v.len() < 6 {
            return Err(serde::de::Error::custom(
                "old row: at least 6 elements expected",
            ));
        }
        let s = |i: usize| -> Result<String, D::Error> {
            match v.get(i) {
                Some(serde_json::Value::String(x)) => Ok(x.clone()),
                Some(serde_json::Value::Null) | None => Ok(String::new()),
                Some(other) => Err(serde::de::Error::custom(format!(
                    "old row element {i}: string expected, got {other}"
                ))),
            }
        };
        let formid = v[2]
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("old row element 2: FormID expected"))?
            as u32;
        let status = s(6)?;
        let row = BaseRow {
            rec: s(0)?,
            field: s(1)?,
            formid,
            edid: s(3)?,
            source: s(4)?,
            trans: s(5)?,
            status: if status.is_empty() {
                None
            } else {
                Some(status)
            },
            ..Default::default()
        };
        Ok(OldRow { row, date: s(7)? })
    }
}

/// The exact key of a row (record, field, FormID, EDID lower-cased, source).
pub fn row_key(r: &BaseRow) -> (String, String, u32, String, String) {
    (
        r.rec.clone(),
        r.field.clone(),
        r.formid,
        r.edid.to_lowercase(),
        r.source.clone(),
    )
}

/// Exact key: the four CDC indicators plus the source text (so records that
/// share a FormID/EDID but differ in wording never collide, and the source is
/// verified by construction). Only built for entries that actually carry the
/// strong indicators (a non-zero FormID **and** a non-empty EDID) — the case of
/// EET game bases and localized plugins.
type ExactKey = (String, String, u32, String, String); // rec, field, formid, edid_lc, source
type IdKey = (String, String, u32, String); // rec, field, formid, edid_lc

/// Loose key: record + field + source. Every translated line is indexed here, so
/// formats that omit the FormID or EDID (notably binary **SST**) — and any
/// cross-format reuse of an identical string — still match.
type LooseKey = (String, String, String); // rec, field, source

/// An in-memory index of every translated line found in the base folders, built
/// as two tiers so it works whatever indicators a source format preserves.
#[derive(Debug, Clone, Default)]
pub struct BaseDb {
    /// Full-indicator matches → translation. Highest confidence (marked Validé).
    exact: HashMap<ExactKey, String>,
    /// record+field+source matches → translation. Fallback for SST / fuzzy reuse
    /// (marked « Traduit par similarité »).
    loose: HashMap<LooseKey, String>,
    /// The status stored with an exact row (format 2): only the mod's own
    /// working dictionary carries it (B-03, decision D-02), so a reopened
    /// mod finds its lines Validé, En attente, Personnalisé… or Ignoré as
    /// they were left, not all Validé.
    exact_status: HashMap<ExactKey, Status>,
    /// The provenance and the review stored with an exact row of the mod's
    /// own dictionary (format 4, B-09): a reopened line gets them back.
    exact_extra: HashMap<ExactKey, (String, String)>,
    /// Text-only pairs (B-05): source text → translation, any field.
    text: HashMap<String, String>,
    /// Record, field, FormID, EDID → (source, translation): the lines of a
    /// record whose text changed (mode FormID seul, B-06; updates, B-07).
    by_id: HashMap<IdKey, (String, String)>,
    /// Every distinct translation of a source text, with its number of
    /// rows, built on first use: the text-only pair first, then the signed
    /// rows (mode Texte seul of B-06; the candidates of B-08).
    by_source: std::sync::OnceLock<HashMap<String, Vec<(String, u32)>>>,
    /// The earlier translations of a (record, field, source) key that a
    /// later row replaced (B-08: several translations in one dictionary).
    loose_dups: HashMap<LooseKey, Vec<String>>,
    /// The rows of a (record, field, source) key that carry its current
    /// translation, when more than one (A-20: the N of the concordance).
    loose_rows: HashMap<LooseKey, u32>,
    /// The old data of a mod's dictionary (B-07), indexed apart: consulted
    /// after the current rows, provenance « ancienne donnée ».
    old: Option<Box<BaseDb>>,
    /// The normalized forms of the sources (B-19), built on first use.
    norm: std::sync::OnceLock<NormIndex>,
    files: usize,
    /// Rows read (lines of the file), for the dictionary tree (B-02).
    rows: usize,
    /// Rows without a signature.
    unsigned: usize,
}

/// The index of the normalized forms of a dictionary (B-19): the pairs
/// (source, translation) once, then by (record, field, form) and by form.
#[derive(Debug, Clone, Default)]
struct NormIndex {
    pairs: Vec<(String, String)>,
    field: HashMap<(String, String, String), Vec<u32>>,
    text: HashMap<String, Vec<u32>>,
}

/// A translation found on the normalized form (B-19), adapted to the new
/// source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormHit {
    pub trans: String,
    /// The source of the dictionary that matched.
    pub source: String,
    pub reports: crate::normalize::Reports,
}

/// The forms of the lines of a document, computed once for every
/// dictionary of the stack (B-19).
#[derive(Debug, Default)]
pub struct FormCache {
    forms: Vec<Option<Option<String>>>,
}

impl FormCache {
    pub fn new() -> FormCache {
        FormCache::default()
    }

    /// The form of line `idx`, whose source is `source`.
    pub fn get(&mut self, idx: usize, source: &str) -> Option<&str> {
        if self.forms.len() <= idx {
            self.forms.resize(idx + 1, None);
        }
        self.forms[idx]
            .get_or_insert_with(|| crate::normalize::form(source))
            .as_deref()
    }
}

/// At most this many sources of one form are compared (a form shared by
/// hundreds of rows is a template, not a sentence).
const NORM_CANDIDATES: usize = 16;

impl BaseDb {
    /// Number of distinct loose (record+field+source) keys indexed.
    pub fn len(&self) -> usize {
        self.loose.len()
    }
    pub fn is_empty(&self) -> bool {
        self.loose.is_empty()
    }
    /// Number of database files that were loaded.
    pub fn files(&self) -> usize {
        self.files
    }
    /// Number of rows read from the file(s).
    pub fn rows(&self) -> usize {
        self.rows
    }
    /// Whether every row is a text-only pair (B-05).
    pub fn is_text_only(&self) -> bool {
        self.rows > 0 && self.unsigned == self.rows
    }
    /// The text-only pairs (B-05).
    pub fn text_pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.text.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
    /// Every current pair, signed or not, for the fragment translation
    /// (B-14): the text-only pairs first.
    pub fn fragment_pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.text
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .chain(
                self.loose
                    .iter()
                    .map(|((_, _, src), t)| (src.as_str(), t.as_str())),
            )
    }

    // ---- Lookups without side effects (the application dialog, B-06) ----

    /// Exact tier: the translation and the stored status, if any.
    pub fn lookup_exact(&self, e: &TmEntry) -> Option<(&str, Option<Status>)> {
        if e.formid == 0 || e.edid.is_empty() {
            return None;
        }
        let k = (
            e.rec_sig.clone(),
            e.field_sig.clone(),
            e.formid,
            e.edid.to_lowercase(),
            e.source.clone(),
        );
        let st = self.exact_status.get(&k).copied();
        match self.exact.get(&k) {
            Some(t) => Some((t.as_str(), st)),
            None => None,
        }
    }

    /// The provenance and the review stored with the exact row of a line
    /// (the mod's own dictionary, format 4).
    pub fn lookup_extra(&self, e: &TmEntry) -> Option<&(String, String)> {
        if e.formid == 0 || e.edid.is_empty() || self.exact_extra.is_empty() {
            return None;
        }
        self.exact_extra.get(&(
            e.rec_sig.clone(),
            e.field_sig.clone(),
            e.formid,
            e.edid.to_lowercase(),
            e.source.clone(),
        ))
    }

    /// Text tier: same record and field, same text.
    pub fn lookup_text(&self, e: &TmEntry) -> Option<&str> {
        self.loose
            .get(&(e.rec_sig.clone(), e.field_sig.clone(), e.source.clone()))
            .map(String::as_str)
    }

    /// Text-only tier (B-05): the unsigned pairs.
    pub fn lookup_text_only(&self, e: &TmEntry) -> Option<&str> {
        self.text.get(e.source.trim()).map(String::as_str)
    }

    /// Any row with the same text, signed or not (mode Texte seul, B-06):
    /// the text-only pair when there is one (the user's deliberate
    /// terminology), else a signed row.
    pub fn lookup_any_text(&self, e: &TmEntry) -> Option<&str> {
        self.text_candidates(e.source.trim())
            .first()
            .map(|(t, _)| t.as_str())
    }

    /// The distinct translations of a source text in this dictionary,
    /// trimmed for the comparison, each with its number of rows (B-08): the
    /// text-only pair first, then the signed rows, the most frequent first.
    pub fn text_candidates(&self, source: &str) -> &[(String, u32)] {
        let map = self.by_source.get_or_init(|| {
            let mut m: HashMap<String, Vec<(String, u32)>> = HashMap::new();
            fn add(list: &mut Vec<(String, u32)>, t: &str) {
                match list.iter_mut().find(|(x, _)| x.trim() == t.trim()) {
                    Some((_, n)) => *n += 1,
                    None => list.push((t.to_string(), 1)),
                }
            }
            for (key, t) in &self.loose {
                let list = m.entry(key.2.trim().to_string()).or_default();
                add(list, t);
                if let Some(dups) = self.loose_dups.get(key) {
                    for d in dups {
                        add(list, d);
                    }
                }
            }
            for list in m.values_mut() {
                list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            }
            for (k, t) in &self.text {
                let list = m.entry(k.clone()).or_default();
                list.retain(|(x, _)| x.trim() != t.trim());
                list.insert(0, (t.clone(), 1));
            }
            m
        });
        map.get(source.trim()).map(Vec::as_slice).unwrap_or(&[])
    }

    fn norm_index(&self) -> &NormIndex {
        self.norm.get_or_init(|| {
            let mut rows: Vec<(&str, &str, &str, &str)> = Vec::new();
            for ((rec, field, src), t) in &self.loose {
                rows.push((rec, field, src, t));
                if let Some(dups) = self
                    .loose_dups
                    .get(&(rec.clone(), field.clone(), src.clone()))
                {
                    for d in dups {
                        rows.push((rec, field, src, d));
                    }
                }
            }
            for (src, t) in &self.text {
                rows.push(("", "", src, t));
            }
            // A stable order: the hash maps are not.
            rows.sort_unstable();
            let mut ix = NormIndex::default();
            let mut forms: HashMap<&str, Option<String>> = HashMap::new();
            for (rec, field, src, t) in rows {
                let Some(f) = forms
                    .entry(src)
                    .or_insert_with(|| crate::normalize::form(src))
                    .clone()
                else {
                    continue;
                };
                let k = ix.pairs.len() as u32;
                ix.pairs.push((src.trim().to_string(), t.to_string()));
                if !rec.is_empty() || !field.is_empty() {
                    ix.field
                        .entry((rec.to_string(), field.to_string(), f.clone()))
                        .or_default()
                        .push(k);
                }
                ix.text.entry(f).or_default().push(k);
            }
            ix
        })
    }

    /// The normalized tier (R-B19-02): same record, field and form, else
    /// same form in any field; the translation adapted to the line's
    /// source (R-B19-03). Several sources whose adapted translations differ
    /// make the line ambiguous (`reports.amb`).
    pub fn lookup_normalized(&self, e: &TmEntry, form: &str) -> Option<NormHit> {
        if self.loose.is_empty() && self.text.is_empty() {
            return None;
        }
        let ix = self.norm_index();
        let list = ix
            .field
            .get(&(e.rec_sig.clone(), e.field_sig.clone(), form.to_string()))
            .or_else(|| ix.text.get(form))?;
        let mut first: Option<NormHit> = None;
        let mut distinct: Vec<String> = Vec::new();
        for &k in list.iter().take(NORM_CANDIDATES) {
            let (src, t) = &ix.pairs[k as usize];
            let (out, reports) = crate::normalize::transfer(src, t, e.source.trim());
            if !distinct.iter().any(|d| d.trim() == out.trim()) {
                distinct.push(out.clone());
            }
            if first.is_none() {
                first = Some(NormHit {
                    trans: out,
                    source: src.clone(),
                    reports,
                });
            }
        }
        let mut hit = first?;
        if distinct.len() >= 2 {
            hit.reports.amb = distinct.len().min(255) as u8;
        }
        Some(hit)
    }

    /// Same record, field, FormID and EDID with another text: the old text
    /// and its translation (mode FormID seul, B-06).
    pub fn lookup_by_id(&self, e: &TmEntry) -> Option<(&str, &str)> {
        if e.formid == 0 || e.edid.is_empty() {
            return None;
        }
        let k = (
            e.rec_sig.clone(),
            e.field_sig.clone(),
            e.formid,
            e.edid.to_lowercase(),
        );
        match self.by_id.get(&k) {
            Some((src, t)) if src != &e.source => Some((src.as_str(), t.as_str())),
            Some(_) => None,
            None => self.old.as_ref().and_then(|o| o.lookup_by_id(e)),
        }
    }

    /// Index one translated row into both tiers. Later inserts win, so scanning
    /// mod translations after game bases lets the user's own work override the
    /// vanilla translation for the same key.
    #[allow(clippy::too_many_arguments)]
    fn add_row(
        &mut self,
        rec: String,
        field: String,
        formid: u32,
        edid: String,
        source: String,
        trans: String,
        status: Option<Status>,
    ) {
        self.rows += 1;
        // A text-only pair (B-05): matched on the text, any field.
        if rec.is_empty() && field.is_empty() {
            self.unsigned += 1;
            let key = source.trim().to_string();
            if !trans.is_empty() && !key.is_empty() {
                self.text.insert(key, trans);
            }
            return;
        }
        // A line of the working dictionary may have a status and no
        // translation (Ignoré, En attente): its exact key is kept so the
        // line comes back as it was.
        if trans.is_empty() && status.is_none() {
            return;
        }
        if formid != 0 && !edid.is_empty() {
            let key = (
                rec.clone(),
                field.clone(),
                formid,
                edid.to_lowercase(),
                source.clone(),
            );
            if !trans.is_empty() {
                self.exact.insert(key.clone(), trans.clone());
                self.by_id.insert(
                    (rec.clone(), field.clone(), formid, edid.to_lowercase()),
                    (source.clone(), trans.clone()),
                );
            }
            match status {
                Some(st) if st != Status::ToTranslate => {
                    self.exact_status.insert(key, st);
                }
                _ => {
                    self.exact_status.remove(&key);
                }
            }
        }
        if !trans.is_empty() {
            let key = (rec, field, source);
            match self.loose.insert(key.clone(), trans.clone()) {
                Some(prev) if prev.trim() != trans.trim() => {
                    self.loose_rows.remove(&key);
                    let d = self.loose_dups.entry(key).or_default();
                    if !d.iter().any(|t| t.trim() == prev.trim()) {
                        d.push(prev);
                    }
                }
                Some(_) => *self.loose_rows.entry(key).or_insert(1) += 1,
                None => {}
            }
        }
    }

    /// Index every effectively-translated entry of a memory.
    pub fn add_memory(&mut self, tm: &TranslationMemory) {
        for e in &tm.entries {
            if e.is_translated() {
                self.add_row(
                    e.rec_sig.clone(),
                    e.field_sig.clone(),
                    e.formid,
                    e.edid.clone(),
                    e.source.clone(),
                    e.trans.clone(),
                    None,
                );
            }
        }
    }

    /// Index rows directly (tests and tools): translations only.
    pub fn add_rows(&mut self, rows: &[BaseRow]) {
        for r in rows {
            self.add_row(
                r.rec.clone(),
                r.field.clone(),
                r.formid,
                r.edid.clone(),
                r.source.clone(),
                r.trans.clone(),
                None,
            );
        }
    }

    /// Index a compact XIMOD base file. `with_status`: keep the status of
    /// the rows (the mod's own dictionary, B-03); a shared base scanned
    /// among many is read for its translations only.
    fn add_ximdb(&mut self, base: BaseFile, with_status: bool) {
        for r in base.e {
            let st = if with_status {
                r.status.as_deref().and_then(Status::from_key)
            } else {
                None
            };
            if with_status
                && (!r.prov.is_empty() || !r.review.is_empty())
                && r.formid != 0
                && !r.edid.is_empty()
            {
                self.exact_extra.insert(
                    (
                        r.rec.clone(),
                        r.field.clone(),
                        r.formid,
                        r.edid.to_lowercase(),
                        r.source.clone(),
                    ),
                    (r.prov.clone(), r.review.clone()),
                );
            }
            self.add_row(r.rec, r.field, r.formid, r.edid, r.source, r.trans, st);
        }
        // The old data (B-07) serve only as the mod's own dictionary.
        if with_status && !base.o.is_empty() {
            let mut old = BaseDb::default();
            for OldRow { row: r, .. } in base.o {
                let st = r.status.as_deref().and_then(Status::from_key);
                old.add_row(r.rec, r.field, r.formid, r.edid, r.source, r.trans, st);
            }
            self.old = Some(Box::new(old));
        }
    }

    /// Rows of the old data (B-07).
    pub fn old_rows(&self) -> usize {
        self.old.as_ref().map(|o| o.rows).unwrap_or(0)
    }

    /// The old data as an index, if any.
    pub fn old(&self) -> Option<&BaseDb> {
        self.old.as_deref()
    }

    /// Every (record, field, source, translation) pair of the dictionary
    /// with its number of rows (A-20): the rows by record and field with
    /// their earlier translations, the text-only pairs (empty record and
    /// field), then the old data. Returns the number of rows visited; `f`
    /// returns `false` to stop.
    pub fn for_each_pair(&self, mut f: impl FnMut(&str, &str, &str, &str, u32) -> bool) -> usize {
        let mut n = 0;
        let mut go = |rec: &str, field: &str, src: &str, t: &str, rows: u32| {
            n += rows as usize;
            f(rec, field, src, t, rows)
        };
        for (key, t) in &self.loose {
            let rows = self.loose_rows.get(key).copied().unwrap_or(1);
            if !go(&key.0, &key.1, &key.2, t, rows) {
                return n;
            }
            if let Some(dups) = self.loose_dups.get(key) {
                for d in dups {
                    if !go(&key.0, &key.1, &key.2, d, 1) {
                        return n;
                    }
                }
            }
        }
        for (src, t) in &self.text {
            if !go("", "", src, t, 1) {
                return n;
            }
        }
        if let Some(old) = &self.old {
            for (key, t) in &old.loose {
                let rows = old.loose_rows.get(key).copied().unwrap_or(1);
                if !go(&key.0, &key.1, &key.2, t, rows) {
                    return n;
                }
            }
        }
        n
    }

    /// Every translated text the heuristic may suggest (B-01): the rows by
    /// record and field, the text-only pairs, then the old data flagged.
    pub fn rows_for_fuzzy(&self) -> Vec<(String, String, String, String, bool)> {
        let mut out: Vec<(String, String, String, String, bool)> =
            Vec::with_capacity(self.loose.len() + self.text.len() + self.old_rows());
        for ((rec, field, src), t) in &self.loose {
            out.push((rec.clone(), field.clone(), src.clone(), t.clone(), false));
        }
        for (src, t) in &self.text {
            out.push((String::new(), String::new(), src.clone(), t.clone(), false));
        }
        if let Some(old) = &self.old {
            for ((rec, field, src), t) in &old.loose {
                out.push((rec.clone(), field.clone(), src.clone(), t.clone(), true));
            }
        }
        out
    }

    /// Build a one-file index from a single base — a compiled `.ximdb` or any raw
    /// base (EET/SST/DSD/JSON/XML). Used when the user *applies* one specific base
    /// to the active document.
    pub fn from_file(path: &Path) -> Option<Self> {
        let mut db = BaseDb::default();
        if is_ximdb(path) {
            db.add_ximdb(read_ximdb(path)?, true);
        } else if has_base_ext(path) {
            db.add_memory(&tm::load(path).ok()?);
        } else {
            return None;
        }
        db.files = 1;
        Some(db)
    }

    /// Scan the given directories (in order) for translation databases and index
    /// them, keeping only the bases that translate into `lang` (ISO 639-3).
    /// Missing directories are skipped.
    ///
    /// When a directory contains compiled `.ximdb` bases, only those whose
    /// language matches `lang` are loaded (they supersede the raw sources they
    /// were built from). A directory without any `.ximdb` falls back to importing
    /// its raw bases (EET/SST/…), which carry no reliable language tag and are
    /// therefore loaded regardless — it is up to the user to keep a single
    /// language of raw bases per folder. An empty `lang` disables filtering.
    pub fn scan(dirs: &[PathBuf], lang: &str) -> Self {
        let mut db = BaseDb::default();
        for dir in dirs {
            db.scan_dir(dir, lang);
        }
        db
    }

    /// Index one directory's direct files, then recurse into its sub-directories.
    /// Mod dictionaries live in per-game sub-folders
    /// (`assets/bases/mods/<Jeu>/…`), so the walk must descend; the
    /// ximdb-vs-raw-base decision is made independently per directory.
    fn scan_dir(&mut self, dir: &Path, lang: &str) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<PathBuf> = Vec::new();
        let mut subdirs: Vec<PathBuf> = Vec::new();
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                subdirs.push(p);
            } else if p.is_file() {
                files.push(p);
            }
        }
        files.sort();
        let ximdb: Vec<&PathBuf> = files.iter().filter(|p| is_ximdb(p)).collect();
        if !ximdb.is_empty() {
            for p in ximdb {
                let flang = peek_ximdb_lang(p);
                let keep = lang.is_empty()
                    || match &flang {
                        Some(l) => l.is_empty() || l == lang,
                        None => true,
                    };
                if keep && let Some(base) = read_ximdb(p) {
                    self.add_ximdb(base, false);
                    self.files += 1;
                }
            }
        } else {
            for p in &files {
                if !has_base_ext(p) {
                    continue;
                }
                if let Ok(tm) = tm::load(p) {
                    self.add_memory(&tm);
                    self.files += 1;
                }
            }
        }
        subdirs.sort();
        for sub in &subdirs {
            self.scan_dir(sub, lang);
        }
    }

    /// Auto-translate a memory in place. For every not-yet-translated line:
    ///
    /// - an **exact** match (record + field + FormID + EDID + source) fills the
    ///   translation and marks it **Validé** — the precise vanilla-override case;
    /// - otherwise a **loose** match (record + field + source) fills it and marks
    ///   it **Traduit par similarité** — this is what makes SST bases (no EDID)
    ///   and any identical-string reuse usable.
    ///
    /// Returns how many lines were auto-translated.
    pub fn auto_translate(&self, tm: &mut TranslationMemory) -> usize {
        self.auto_translate_entries(&mut tm.entries)
    }

    /// [`auto_translate`](Self::auto_translate) on a slice of entries (the
    /// undo wrapper works on the slice).
    pub fn auto_translate_entries(&self, entries: &mut [TmEntry]) -> usize {
        self.auto_translate_entries_with(entries, |_, _| {})
    }

    /// [`auto_translate_entries`](Self::auto_translate_entries) telling, for
    /// each line filled, at which level it matched (the provenance, B-02).
    pub fn auto_translate_entries_with(
        &self,
        entries: &mut [TmEntry],
        on_fill: impl FnMut(usize, FillLevel),
    ) -> usize {
        self.auto_translate_entries_opts(entries, None, on_fill)
    }

    /// [`auto_translate_entries_norm`](Self::auto_translate_entries_norm)
    /// without the normalized tier.
    pub fn auto_translate_entries_opts(
        &self,
        entries: &mut [TmEntry],
        override_status: Option<Status>,
        mut on_fill: impl FnMut(usize, FillLevel),
    ) -> usize {
        self.auto_translate_entries_norm(entries, override_status, None, |i, l, _| on_fill(i, l))
    }

    /// [`auto_translate_entries_with`](Self::auto_translate_entries_with)
    /// with the status given on an exact or a text-only match overridden
    /// (`Some(Custom)` for the personal dictionary, B-04: it is the user's
    /// own terminology); a stored status (mod dictionary) still wins, and
    /// the text tier stays « par similarité ». With `norm`, the normalized
    /// tier (B-19) follows the text-only one and the old data: En attente,
    /// `on_fill` receiving the source found.
    pub fn auto_translate_entries_norm(
        &self,
        entries: &mut [TmEntry],
        override_status: Option<Status>,
        mut norm: Option<&mut FormCache>,
        mut on_fill: impl FnMut(usize, FillLevel, Option<&str>),
    ) -> usize {
        let mut n = 0;
        for (idx, e) in entries.iter_mut().enumerate() {
            // Translated, deliberately ignored, or never shown in game: untouched.
            if e.is_translated() || e.unused || status::derive(e) == Status::Ignored {
                continue;
            }
            // Exact tier (only when the line itself carries the strong indicators).
            if e.formid != 0 && !e.edid.is_empty() {
                let k = (
                    e.rec_sig.clone(),
                    e.field_sig.clone(),
                    e.formid,
                    e.edid.to_lowercase(),
                    e.source.clone(),
                );
                // The mod's own dictionary brings the status back (B-03):
                // with its text, or without one (Ignoré, En attente).
                let stored = self.exact_status.get(&k).copied();
                // Its provenance and review too (format 4, B-09).
                let extra = self.exact_extra.get(&k);
                match (self.exact.get(&k), stored) {
                    (Some(trans), st) => {
                        e.trans = trans.clone();
                        restore_extra(e, extra);
                        let st = match st {
                            Some(st) if st.applicable_to(e) => st,
                            _ => override_status.unwrap_or(Status::Validated),
                        };
                        status::set(e, st);
                        on_fill(idx, FillLevel::Exact, None);
                        n += 1;
                        continue;
                    }
                    (None, Some(st)) if st.applicable_to(e) => {
                        restore_extra(e, extra);
                        status::set(e, st);
                        on_fill(idx, FillLevel::Exact, None);
                        n += 1;
                        continue;
                    }
                    _ => {}
                }
            }
            // Loose tier: record + field + source.
            let lk = (e.rec_sig.clone(), e.field_sig.clone(), e.source.clone());
            if let Some(trans) = self.loose.get(&lk) {
                e.trans = trans.clone();
                e.prov.clear();
                status::set(e, Status::BySimilarity);
                on_fill(idx, FillLevel::Text, None);
                n += 1;
                continue;
            }
            // Text-only tier (B-05): same text, any field.
            if let Some(trans) = self.text.get(e.source.trim()) {
                e.trans = trans.clone();
                e.prov.clear();
                status::set(e, override_status.unwrap_or(Status::BySimilarity));
                on_fill(idx, FillLevel::TextOnly, None);
                n += 1;
                continue;
            }
            // Normalized tier (B-19): same text to the numbers, the
            // punctuation and the case → En attente.
            if let Some(cache) = norm.as_deref_mut()
                && let Some(form) = cache.get(idx, &e.source)
                && let Some(hit) = self.lookup_normalized(e, form)
            {
                e.trans = hit.trans;
                e.prov.clear();
                status::set(e, Status::Pending);
                on_fill(idx, FillLevel::Normalized(hit.reports), Some(&hit.source));
                n += 1;
                continue;
            }
            // Old data (B-07): a line that came back, or the same text.
            if let Some(old) = &self.old {
                if let Some((trans, st)) = old.lookup_exact(e) {
                    e.trans = trans.to_string();
                    e.prov.clear();
                    let st = match st {
                        Some(st) if st != Status::ToTranslate && st.applicable_to(e) => st,
                        _ => Status::Validated,
                    };
                    status::set(e, st);
                    on_fill(idx, FillLevel::Old, None);
                    n += 1;
                    continue;
                }
                if let Some(trans) = old.lookup_text(e) {
                    e.trans = trans.to_string();
                    e.prov.clear();
                    status::set(e, Status::BySimilarity);
                    on_fill(idx, FillLevel::Old, None);
                    n += 1;
                    continue;
                }
                // The old data get the normalized tier too (R-B19-06).
                if let Some(cache) = norm.as_deref_mut()
                    && let Some(form) = cache.get(idx, &e.source)
                    && let Some(hit) = old.lookup_normalized(e, form)
                {
                    e.trans = hit.trans;
                    e.prov.clear();
                    status::set(e, Status::Pending);
                    on_fill(idx, FillLevel::Normalized(hit.reports), Some(&hit.source));
                    n += 1;
                }
            }
        }
        n
    }
}

/// Give a line the provenance and the review stored with its row (the
/// mod's own dictionary), or no provenance: the stack names the level.
fn restore_extra(e: &mut TmEntry, extra: Option<&(String, String)>) {
    match extra {
        Some((p, r)) => {
            e.prov = p.clone();
            if !r.is_empty() {
                e.review = r.clone();
            }
        }
        None => e.prov.clear(),
    }
}

/// Clear every base-derived ("by similarity") translation so a subsequent
/// [`BaseDb::auto_translate`] regenerates it from the current database, while
/// leaving manual and validated lines untouched. Only the loose-tier status
/// [`Status::BySimilarity`] is targeted (never `Validated`, `Modified`,
/// `Custom`…). Returns how many lines were reset.
pub fn reset_by_similarity(tm: &mut TranslationMemory) -> usize {
    reset_by_similarity_entries(&mut tm.entries)
}

/// [`reset_by_similarity`] on a slice of entries.
pub fn reset_by_similarity_entries(entries: &mut [TmEntry]) -> usize {
    let mut n = 0;
    for e in entries.iter_mut() {
        if crate::status::derive(e) == Status::BySimilarity {
            e.trans.clear();
            e.prov.clear();
            crate::status::set(e, Status::ToTranslate);
            n += 1;
        }
    }
    n
}

/// At which level a dictionary filled a line (the provenance, B-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillLevel {
    /// Record, field, FormID, EDID and source identical.
    Exact,
    /// Same record and field, same source text.
    Text,
    /// Same text, any field (B-05).
    TextOnly,
    /// Word heuristic, with its score in percent (B-01).
    Fuzzy(u8),
    /// From the old data of the mod's dictionary (B-07).
    Old,
    /// Same record, field, FormID and EDID, text changed (B-06).
    FormId,
    /// The value of the existing target file of a text document (C-05).
    TargetFile,
    /// An online service's answer (D-01).
    Online,
    /// Same text to the numbers, punctuation and case (B-19), with what
    /// was carried over.
    Normalized(crate::normalize::Reports),
}

impl FillLevel {
    pub fn i18n_key(self) -> &'static str {
        match self {
            FillLevel::Exact => "origin-level-exact",
            FillLevel::Text => "origin-level-text",
            FillLevel::TextOnly => "origin-level-textonly",
            FillLevel::Fuzzy(_) => "origin-level-fuzzy",
            FillLevel::Old => "origin-level-old",
            FillLevel::FormId => "origin-level-formid",
            FillLevel::TargetFile => "origin-level-target-file",
            FillLevel::Online => "origin-level-online",
            FillLevel::Normalized(_) => "origin-level-normalized",
        }
    }
}

/// The rows of a `.ximdb` as they are, with the language it declares
/// (the editor, B-04).
pub fn read_rows(path: &Path) -> Result<(String, Vec<BaseRow>), String> {
    let base = read_ximdb(path)
        .ok_or_else(|| crate::l10n!("err-ximdb-unreadable-path", path = path.display()))?;
    Ok((base.lang, base.e))
}

/// The rows and the old data of a `.ximdb` (B-07).
pub fn read_rows_and_old(path: &Path) -> Result<(String, Vec<BaseRow>, Vec<OldRow>), String> {
    let base = read_ximdb(path)
        .ok_or_else(|| crate::l10n!("err-ximdb-unreadable-path", path = path.display()))?;
    Ok((base.lang, base.e, base.o))
}

/// The old data of a mod's dictionary after a write (R-B07-01): the
/// previous current rows whose exact key the new rows no longer have join
/// the old data with `today`; an old row whose key is current again
/// leaves; keys are unique, the earliest date kept; rows older than
/// `keep_months` (0 = forever) are dropped.
pub fn roll_old_data(
    prev_rows: &[BaseRow],
    prev_old: Vec<OldRow>,
    new_rows: &[BaseRow],
    today: &str,
    keep_months: u32,
) -> Vec<OldRow> {
    use std::collections::HashSet;
    let current: HashSet<_> = new_rows.iter().map(row_key).collect();
    let mut out: Vec<OldRow> = Vec::new();
    let mut seen: HashSet<_> = HashSet::new();
    for o in prev_old {
        let k = row_key(&o.row);
        if current.contains(&k) || !seen.insert(k) {
            continue;
        }
        out.push(o);
    }
    for r in prev_rows {
        let k = row_key(r);
        if current.contains(&k) || !seen.insert(k) {
            continue;
        }
        out.push(OldRow {
            row: r.clone(),
            date: today.to_string(),
        });
    }
    if keep_months > 0 {
        let limit = crate::backup::date_minus_months(today, keep_months);
        out.retain(|o| o.date.is_empty() || o.date.as_str() >= limit.as_str());
    }
    out
}

/// [`write_ximdb_rotating`] with the old data (B-07). The identity of the
/// plugin already in the file's header is kept (B-12).
pub fn write_ximdb_full(
    rows: Vec<BaseRow>,
    old: Vec<OldRow>,
    lang: &str,
    out: &Path,
    copies: usize,
) -> Result<usize, String> {
    let ident = peek_ident(out);
    let extra = peek_extra(out);
    write_ximdb_with(rows, old, lang, out, copies, ident.as_ref(), &extra)
}

/// [`write_ximdb_full`] with the identity of the plugin the mod's
/// dictionary is written for (R-B12-01); the other header settings are
/// kept from the file.
pub fn write_ximdb_ident(
    rows: Vec<BaseRow>,
    old: Vec<OldRow>,
    lang: &str,
    out: &Path,
    copies: usize,
    ident: Option<&crate::plugin_id::Ident>,
) -> Result<usize, String> {
    let extra = peek_extra(out);
    write_ximdb_with(rows, old, lang, out, copies, ident, &extra)
}

/// The settings a mod's dictionary keeps for its file in its header (D-09,
/// C-08): absent = automatic / default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderExtra {
    /// Forced page of reading (WHATWG name).
    pub enc_in: Option<String>,
    /// Forced page of writing.
    pub enc_out: Option<String>,
    /// « Traduire les chaînes VMAD de ce fichier ».
    pub vmad: Option<bool>,
    /// The learned local exclusion rules and the rules off (B-18).
    pub rules: Option<Vec<crate::exclusions::Rule>>,
    pub rules_off: Option<Vec<String>>,
}

/// Write a mod's dictionary with its identity and its settings.
pub fn write_ximdb_with(
    rows: Vec<BaseRow>,
    old: Vec<OldRow>,
    lang: &str,
    out: &Path,
    copies: usize,
    ident: Option<&crate::plugin_id::Ident>,
    extra: &HeaderExtra,
) -> Result<usize, String> {
    crate::backup::rotate_backup(out, copies)
        .map_err(|e| crate::l10n!("err-backup-copy", error = e))?;
    let n = rows.len();
    let base = BaseFile {
        v: XIMDB_FORMAT,
        lang: lang.to_string(),
        plugin: ident.map(|i| i.plugin.clone()),
        size: ident.map(|i| i.size),
        crc: ident.map(|i| i.crc_hex()),
        written: ident.map(|i| i.written).filter(|w| *w > 0),
        enc_in: extra.enc_in.clone().filter(|s| !s.is_empty()),
        enc_out: extra.enc_out.clone().filter(|s| !s.is_empty()),
        vmad: extra.vmad.filter(|v| *v),
        rules: extra.rules.clone().unwrap_or_default(),
        rules_off: extra.rules_off.clone().unwrap_or_default(),
        e: rows,
        o: old,
    };
    let bytes =
        serde_json::to_vec(&base).map_err(|e| crate::l10n!("err-ximdb-serialize", error = e))?;
    crate::config::paths::write_atomic(out, &bytes)
        .map_err(|e| crate::l10n!("err-ximdb-write", error = e))?;
    let _ = crate::backup::purge_backups(out, copies);
    Ok(n)
}

/// The first bytes of a `.ximdb`: its header, up to the rows.
fn peek_head(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    // The header ends where the rows begin; learned rules (B-18) can make
    // it longer than the first 2 KB: read on, up to 4 MB.
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 2048];
    loop {
        let k = f.read(&mut chunk).ok()?;
        buf.extend_from_slice(&chunk[..k]);
        let text = String::from_utf8_lossy(&buf);
        if let Some(i) = text.find("\"e\":[").or_else(|| text.find("\"e\": [")) {
            return Some(text[..i].to_string());
        }
        if k == 0 || buf.len() > 4 << 20 {
            return Some(text.into_owned());
        }
    }
}

/// A JSON value of the header, by key.
fn head_value<'a>(head: &'a str, key: &str) -> Option<&'a str> {
    let k = format!("\"{key}\"");
    let i = head.find(&k)? + k.len();
    let rest = head[i..].trim_start().strip_prefix(':')?.trim_start();
    Some(rest)
}

/// The settings of the file in the header of a `.ximdb` (D-09, C-08).
pub fn peek_extra(path: &Path) -> HeaderExtra {
    let Some(head) = peek_head(path) else {
        return HeaderExtra::default();
    };
    let string = |key: &str| -> Option<String> {
        let rest = head_value(&head, key)?;
        let mut it = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        it.next()?.ok()
    };
    HeaderExtra {
        enc_in: string("enc_in"),
        enc_out: string("enc_out"),
        vmad: head_value(&head, "vmad").map(|r| r.starts_with("true")),
        rules: list(&head, "rules"),
        rules_off: list(&head, "rulesOff"),
    }
}

/// A list of the header (B-18), read with serde from its first byte.
fn list<T: serde::de::DeserializeOwned>(head: &str, key: &str) -> Option<Vec<T>> {
    let rest = head_value(head, key)?;
    let mut it = serde_json::Deserializer::from_str(rest).into_iter::<Vec<T>>();
    it.next()?.ok()
}

/// The identity of the plugin in the header of a `.ximdb` (B-12), read
/// from its first bytes; `None` for a file without it (written before).
pub fn peek_ident(path: &Path) -> Option<crate::plugin_id::Ident> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 2048];
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..]).ok()?;
        if k == 0 {
            break;
        }
        n += k;
    }
    let text = String::from_utf8_lossy(&buf[..n]);
    // The header ends where the rows begin.
    let head = match text.find("\"e\"") {
        Some(i) => &text[..i],
        None => &text[..],
    };
    let value_after = |key: &str| -> Option<&str> {
        let k = format!("\"{key}\"");
        let i = head.find(&k)? + k.len();
        let rest = head[i..].trim_start().strip_prefix(':')?.trim_start();
        Some(rest)
    };
    let plugin: String = {
        let rest = value_after("plugin")?;
        let mut it = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        it.next()?.ok()?
    };
    let number = |key: &str| -> Option<i64> {
        let rest = value_after(key)?;
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    };
    let size = number("size")? as u64;
    let crc = {
        let rest = value_after("crc")?.strip_prefix('"')?;
        crate::plugin_id::parse_crc(&rest[..rest.find('"')?])?
    };
    Some(crate::plugin_id::Ident {
        plugin,
        size,
        crc,
        written: number("written").unwrap_or(0),
    })
}

/// Public wrappers for the dictionary tree (B-02).
pub fn is_ximdb_path(p: &Path) -> bool {
    is_ximdb(p)
}
pub fn is_raw_base_path(p: &Path) -> bool {
    has_base_ext(p)
}
pub fn peek_lang(p: &Path) -> Option<String> {
    peek_ximdb_lang(p)
}
/// The format version in the first bytes of a `.ximdb`.
pub fn peek_version(p: &Path) -> Option<u32> {
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut buf = [0u8; 64];
    let n = f.read(&mut buf).ok()?;
    let s: String = String::from_utf8_lossy(&buf[..n])
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let key = "\"v\":";
    let start = s.find(key)? + key.len();
    let digits: String = s[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}
/// Whether a `.ximdb` looks like a text-only base (B-05): its first row
/// has no signature. Read from the first bytes, so the tree classifies a
/// file without parsing it; the index knows for sure once loaded.
pub fn is_text_only_hint(p: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(p) else {
        return false;
    };
    let mut buf = [0u8; 512];
    let Ok(n) = f.read(&mut buf) else {
        return false;
    };
    let s: String = String::from_utf8_lossy(&buf[..n])
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let Some(start) = s.find("\"e\":[[") else {
        return false;
    };
    s[start + 6..].starts_with("\"\",\"\",")
}

/// Read a text file of pairs « source<TAB>traduction » (one per line,
/// UTF-8, `#` comments and empty lines skipped) into text-only rows (B-05).
pub fn read_text_pairs(path: &Path) -> Result<Vec<BaseRow>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| crate::l10n!("err-read-path", path = path.display(), error = e))?;
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((src, tr)) = line.split_once('\t') else {
            continue;
        };
        let (src, tr) = (src.trim(), tr.trim());
        if src.is_empty() || tr.is_empty() {
            continue;
        }
        rows.push(BaseRow {
            rec: String::new(),
            field: String::new(),
            formid: 0,
            edid: String::new(),
            source: src.to_string(),
            trans: tr.to_string(),
            status: None,
            ..Default::default()
        });
    }
    Ok(rows)
}
/// Modification time of a file, for the per-file cache.
pub fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    crate::slow_io("mtime", || std::fs::metadata(p).ok()?.modified().ok())
}

/// Whether a path has a raw-database extension we can import.
fn has_base_ext(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| BASE_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Whether a path is a compact XIMOD base.
fn is_ximdb(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(XIMDB_EXT))
        == Some(true)
}

/// Read a compact XIMOD base from disk.
fn read_ximdb(path: &Path) -> Option<BaseFile> {
    let f = std::fs::File::open(path).ok()?;
    serde_json::from_reader(BufReader::new(f)).ok()
}

/// Read only the `lang` field of a `.ximdb` from its first bytes (so we can skip
/// a non-matching multi-megabyte base without parsing it whole).
fn peek_ximdb_lang(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 256];
    let n = f.read(&mut buf).ok()?;
    // Tolerant of spaces around the colon (hand-written or pretty files).
    let s: String = String::from_utf8_lossy(&buf[..n])
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let key = "\"lang\":\"";
    let start = s.find(key)? + key.len();
    let rest = &s[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Compile the given raw database files into a single **compact language-tagged
/// XIMOD base** at `out`. Only the six lookup fields of each *translated* line
/// are kept. `lang` is the target-language ISO 639-3 code stored in the base.
/// Returns `(files_read, entries_written)`.
pub fn compile_files(files: &[PathBuf], lang: &str, out: &Path) -> Result<(usize, usize), String> {
    let mut rows: Vec<BaseRow> = Vec::new();
    let mut read = 0usize;
    for p in files {
        if !has_base_ext(p) {
            continue;
        }
        if let Ok(tm) = tm::load(p) {
            for e in tm.entries.iter().filter(|e| e.is_translated()) {
                rows.push(BaseRow::from_entry(e, false));
            }
            read += 1;
        }
    }
    let n = write_ximdb(rows, lang, out)?;
    Ok((read, n))
}

/// Compile every raw base found directly in `dir` into a single language-tagged
/// XIMOD base at `out` (merges all games into one file).
pub fn compile_dir(dir: &Path, lang: &str, out: &Path) -> Result<(usize, usize), String> {
    compile_files(&raw_bases_in(dir)?, lang, out)
}

/// Compile each raw base found directly in `dir` into **its own** language-tagged
/// XIMOD base (`<nom>.ximdb`, one base per game). Returns `(files_compiled,
/// total_entries_written)`.
pub fn compile_dir_per_file(dir: &Path, lang: &str) -> Result<(usize, usize), String> {
    let paths = raw_bases_in(dir)?;
    let mut files = 0usize;
    let mut total = 0usize;
    for p in paths {
        let out = p.with_extension(XIMDB_EXT);
        let (_, n) = compile_files(std::slice::from_ref(&p), lang, &out)?;
        files += 1;
        total += n;
    }
    Ok((files, total))
}

/// The raw base files directly inside `dir`, sorted.
fn raw_bases_in(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| crate::l10n!("err-read-dir", error = e))?;
    let mut paths: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && has_base_ext(p))
        .collect();
    paths.sort();
    Ok(paths)
}

/// Build a translation memory from a compact XIMOD base, so it can be exported
/// back to EET/SST. Each stored row becomes a translated entry.
pub fn ximdb_to_tm(path: &Path) -> Result<TranslationMemory, String> {
    let base = read_ximdb(path).ok_or_else(|| crate::l10n!("err-ximdb-unreadable"))?;
    let entries = base
        .e
        .into_iter()
        .map(|r| {
            // Repair C1 control characters left by an older Latin-1 mis-decode so
            // dictionaries built before the fix display correctly (e.g. the German
            // en-dash that showed as a box). No-op for already-clean text.
            let source = crate::tm::encoding::repair_c1_controls(&r.source).into_owned();
            let trans = crate::tm::encoding::repair_c1_controls(&r.trans).into_owned();
            let mut e = tm::TmEntry {
                rec_sig: r.rec,
                field_sig: r.field,
                formid: r.formid,
                edid: r.edid,
                source,
                trans,
                prov: r.prov,
                review: r.review,
                ..Default::default()
            };
            match r.status.as_deref().and_then(Status::from_key) {
                Some(st) => status::set(&mut e, st),
                None => e.set_translated_flags(true),
            }
            e
        })
        .collect();
    let meta = tm::TmMeta {
        lang_dest: base.lang,
        ..Default::default()
    };
    Ok(TranslationMemory::new(entries, meta))
}

/// The raw base files (EET/SST/…) directly inside `dir`, sorted; empty on error.
/// Public wrapper over the internal lister, for the "dictionary from a game" flow.
pub fn raw_bases_in_dir(dir: &Path) -> Vec<PathBuf> {
    raw_bases_in(dir).unwrap_or_default()
}

/// Write a XIMOD base (format 2) from pre-built rows and a target-language
/// tag, atomically. Used by the dictionary builders and by the mod's own
/// dictionary. Returns the number of rows written.
pub fn write_ximdb(rows: Vec<BaseRow>, lang: &str, out: &Path) -> Result<usize, String> {
    let n = rows.len();
    let base = BaseFile {
        v: XIMDB_FORMAT,
        lang: lang.to_string(),
        plugin: None,
        size: None,
        crc: None,
        written: None,
        enc_in: None,
        enc_out: None,
        vmad: None,
        rules: Vec::new(),
        rules_off: Vec::new(),
        e: rows,
        o: Vec::new(),
    };
    let bytes =
        serde_json::to_vec(&base).map_err(|e| crate::l10n!("err-ximdb-serialize", error = e))?;
    crate::config::paths::write_atomic(out, &bytes)
        .map_err(|e| crate::l10n!("err-ximdb-write", error = e))?;
    Ok(n)
}

/// [`write_ximdb`] with the timestamped copies of B-11 (R-B11-01): the
/// previous file goes to `backups/`, the write happens, then only the
/// `copies` most recent copies are kept. `copies == 0` overwrites in place.
pub fn write_ximdb_rotating(
    rows: Vec<BaseRow>,
    lang: &str,
    out: &Path,
    copies: usize,
) -> Result<usize, String> {
    crate::backup::rotate_backup(out, copies)
        .map_err(|e| crate::l10n!("err-backup-copy", error = e))?;
    let n = write_ximdb(rows, lang, out)?;
    let _ = crate::backup::purge_backups(out, copies);
    Ok(n)
}

/// Convert a `.ximdb` to an EET/SST file (format chosen from the `out`
/// extension). Returns the number of entries written.
pub fn export_ximdb(input: &Path, out: &Path) -> Result<usize, String> {
    let mem = ximdb_to_tm(input)?;
    let n = mem.entries.len();
    tm::save(&mem, out)
        .map_err(|e| crate::l10n!("err-write-path", path = out.display(), error = e))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tm::TmEntry;

    fn entry(rec: &str, field: &str, formid: u32, edid: &str, src: &str, tr: &str) -> TmEntry {
        TmEntry {
            rec_sig: rec.into(),
            field_sig: field.into(),
            formid,
            edid: edid.into(),
            source: src.into(),
            trans: tr.into(),
            ..Default::default()
        }
    }

    fn mem(entries: Vec<TmEntry>) -> TranslationMemory {
        TranslationMemory {
            meta: Default::default(),
            entries,
            history: Vec::new(),
        }
    }

    #[test]
    fn ximdb_reverse_and_export_roundtrip() {
        let dir = std::env::temp_dir();
        let ximdb = dir.join("xim_rt_test.ximdb");
        let base = BaseFile {
            v: 1,
            lang: "fra".into(),
            plugin: None,
            size: None,
            crc: None,
            written: None,
            enc_in: None,
            enc_out: None,
            vmad: None,
            rules: Vec::new(),
            rules_off: Vec::new(),
            o: Vec::new(),
            e: vec![
                BaseRow {
                    rec: "WEAP".into(),
                    field: "FULL".into(),
                    formid: 0x123,
                    edid: "IronSword".into(),
                    source: "Iron Sword".into(),
                    trans: "Épée de fer".into(),
                    status: None,
                    ..Default::default()
                },
                BaseRow {
                    rec: "BOOK".into(),
                    field: "DESC".into(),
                    formid: 0,
                    edid: String::new(),
                    source: "Hello".into(),
                    trans: "Bonjour".into(),
                    status: None,
                    ..Default::default()
                },
            ],
        };
        serde_json::to_writer(std::fs::File::create(&ximdb).unwrap(), &base).unwrap();

        // ximdb → TM keeps the six fields and the target language.
        let m = ximdb_to_tm(&ximdb).unwrap();
        assert_eq!(m.entries.len(), 2);
        assert_eq!(m.meta.lang_dest, "fra");
        let e0 = &m.entries[0];
        assert_eq!(
            (
                e0.rec_sig.as_str(),
                e0.field_sig.as_str(),
                e0.formid,
                e0.edid.as_str()
            ),
            ("WEAP", "FULL", 0x123, "IronSword")
        );
        assert_eq!(e0.trans, "Épée de fer");
        assert!(e0.is_translated());

        // export → a pivot JSON reloads to the same entry count.
        let out = dir.join("xim_rt_test.json");
        assert_eq!(export_ximdb(&ximdb, &out).unwrap(), 2);
        assert_eq!(tm::load(&out).unwrap().entries.len(), 2);

        let _ = std::fs::remove_file(&ximdb);
        let _ = std::fs::remove_file(&out);
    }

    fn base() -> BaseDb {
        let tm = mem(vec![
            entry(
                "WEAP",
                "FULL",
                0x123,
                "IronSword",
                "Iron Sword",
                "Épée en fer",
            ),
            entry("BOOK", "DESC", 0x456, "Note01", "A note.", "Une note."),
        ]);
        let mut db = BaseDb::default();
        db.add_memory(&tm);
        db
    }

    #[test]
    fn matches_all_indicators_and_source() {
        let db = base();
        let mut tm = mem(vec![entry(
            "WEAP",
            "FULL",
            0x123,
            "IronSword",
            "Iron Sword",
            "",
        )]);
        let n = db.auto_translate(&mut tm);
        assert_eq!(n, 1);
        assert_eq!(tm.entries[0].trans, "Épée en fer");
        assert_eq!(status::derive(&tm.entries[0]), Status::Validated);
    }

    #[test]
    fn edid_is_case_insensitive() {
        let db = base();
        let mut tm = mem(vec![entry(
            "WEAP",
            "FULL",
            0x123,
            "ironsword",
            "Iron Sword",
            "",
        )]);
        assert_eq!(db.auto_translate(&mut tm), 1);
    }

    #[test]
    fn changed_source_is_not_translated() {
        // Same indicators, but the mod changed the wording → must NOT apply the
        // stale base translation.
        let db = base();
        let mut tm = mem(vec![entry(
            "WEAP",
            "FULL",
            0x123,
            "IronSword",
            "Steel Sword",
            "",
        )]);
        assert_eq!(db.auto_translate(&mut tm), 0);
        assert!(tm.entries[0].trans.is_empty());
    }

    #[test]
    fn different_indicator_is_not_translated() {
        let db = base();
        // Same source & EDID but a different field signature.
        let mut tm = mem(vec![entry(
            "WEAP",
            "DESC",
            0x123,
            "IronSword",
            "Iron Sword",
            "",
        )]);
        assert_eq!(db.auto_translate(&mut tm), 0);
    }

    #[test]
    fn retranslate_auto_regenerates_similar_but_keeps_manual() {
        let db = base();
        // Two loose-matching lines (formid 0 → no exact tier) and one manual line.
        let mut tm = mem(vec![
            entry("WEAP", "FULL", 0, "", "Iron Sword", ""), // will auto → BySimilarity
            entry("WEAP", "FULL", 0, "", "Iron Sword", ""), // this one we override manually
            entry("BOOK", "DESC", 0, "", "A note.", ""),    // auto → BySimilarity
        ]);
        db.auto_translate(&mut tm);
        assert_eq!(status::derive(&tm.entries[0]), Status::BySimilarity);
        assert_eq!(tm.entries[0].trans, "Épée en fer");

        // Simulate a manual edit on entry 1: user text + a non-auto status.
        tm.entries[1].trans = "Ma traduction".into();
        status::set(&mut tm.entries[1], Status::ModifiedProposal);

        // Re-translate auto lines: resets only the two still-BySimilarity lines.
        let reset = reset_by_similarity(&mut tm);
        assert_eq!(reset, 2, "only the two by-similarity lines are reset");
        assert!(tm.entries[0].trans.is_empty());
        assert!(tm.entries[2].trans.is_empty());
        assert_eq!(
            tm.entries[1].trans, "Ma traduction",
            "manual edit preserved"
        );

        // Regenerate from the base.
        db.auto_translate(&mut tm);
        assert_eq!(tm.entries[0].trans, "Épée en fer", "regenerated");
        assert_eq!(tm.entries[2].trans, "Une note.", "regenerated");
        assert_eq!(
            tm.entries[1].trans, "Ma traduction",
            "still the manual edit"
        );
    }

    #[test]
    fn already_translated_is_left_untouched() {
        let db = base();
        let mut tm = mem(vec![entry(
            "WEAP",
            "FULL",
            0x123,
            "IronSword",
            "Iron Sword",
            "Ma trad",
        )]);
        assert_eq!(db.auto_translate(&mut tm), 0);
        assert_eq!(tm.entries[0].trans, "Ma trad");
    }

    #[test]
    fn mods_override_games_by_scan_order() {
        // Game base then mod base with the same key → mod translation wins.
        let mut db = BaseDb::default();
        db.add_memory(&mem(vec![entry(
            "WEAP",
            "FULL",
            1,
            "X",
            "Sword",
            "Épée (jeu)",
        )]));
        db.add_memory(&mem(vec![entry(
            "WEAP",
            "FULL",
            1,
            "X",
            "Sword",
            "Épée (mod)",
        )]));
        let mut tm = mem(vec![entry("WEAP", "FULL", 1, "X", "Sword", "")]);
        db.auto_translate(&mut tm);
        assert_eq!(tm.entries[0].trans, "Épée (mod)");
    }

    #[test]
    fn sst_like_without_edid_matches_loosely() {
        // A base entry like a binary SST record: FormID present, EDID absent.
        let mut db = BaseDb::default();
        db.add_memory(&mem(vec![entry(
            "WEAP",
            "FULL",
            0x55,
            "",
            "Steel Dagger",
            "Dague en acier",
        )]));
        // A plugin mod line carries both FormID and EDID, same rec/field/source.
        let mut tm = mem(vec![entry(
            "WEAP",
            "FULL",
            0x55,
            "SteelDagger",
            "Steel Dagger",
            "",
        )]);
        assert_eq!(db.auto_translate(&mut tm), 1);
        assert_eq!(tm.entries[0].trans, "Dague en acier");
        // A loose (record+field+source) match is flagged for review, not blindly
        // validated like an exact-indicator match.
        assert_eq!(status::derive(&tm.entries[0]), Status::BySimilarity);
    }

    #[test]
    fn loose_requires_same_record_and_field() {
        let mut db = BaseDb::default();
        db.add_memory(&mem(vec![entry("WEAP", "FULL", 0, "", "Open", "Ouvrir")]));
        // Same source text but a different record type → no loose match.
        let mut tm = mem(vec![entry("BOOK", "FULL", 0, "", "Open", "")]);
        assert_eq!(db.auto_translate(&mut tm), 0);
    }

    #[test]
    fn scan_descends_into_mod_game_subfolders() {
        // Mod dictionaries live in assets/bases/mods/<Jeu>/… — the scan must
        // recurse to find them.
        let root = std::env::temp_dir().join("ximod_scan_recursive_test");
        let _ = std::fs::remove_dir_all(&root);
        let sub = root.join("mods").join("SkyrimSE");
        std::fs::create_dir_all(&sub).unwrap();
        let out = sub.join("MonMod_ENG-FRA.ximdb");
        let rows = vec![BaseRow {
            rec: "WEAP".into(),
            field: "FULL".into(),
            formid: 1,
            edid: "X".into(),
            source: "Sword".into(),
            trans: "Épée".into(),
            status: None,
            ..Default::default()
        }];
        write_ximdb(rows, "fra", &out).unwrap();

        let db = BaseDb::scan(&[root.join("mods")], "fra");
        assert_eq!(db.files, 1, "the ximdb in mods/SkyrimSE should be found");
        let mut tm = mem(vec![entry("WEAP", "FULL", 1, "X", "Sword", "")]);
        assert_eq!(db.auto_translate(&mut tm), 1);
        assert_eq!(tm.entries[0].trans, "Épée");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ximdb_v1_and_v2_rows_read_and_round_trip() {
        // A v1 file (six-element rows) reads back without status.
        let dir = std::env::temp_dir().join(format!("ximod-ximdb-v2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let v1 = dir.join("v1.ximdb");
        std::fs::write(&v1, r#"{"v":1,"lang":"fra","e":[["WEAP","FULL",291,"IronSword","Iron Sword","Épée de fer"]]}"#).unwrap();
        let base = read_ximdb(&v1).unwrap();
        assert_eq!(base.e.len(), 1);
        assert_eq!(base.e[0].formid, 291);
        assert_eq!(base.e[0].status, None);
        let m = ximdb_to_tm(&v1).unwrap();
        assert!(m.entries[0].is_translated());
        assert_eq!(status::derive(&m.entries[0]), Status::BySimilarity);

        // A v2 file keeps the status per row; rows without one stay accepted.
        let mut e = entry("BOOK", "FULL", 5, "Note", "A Note", "Une note");
        status::set(&mut e, Status::Pending);
        let rows = vec![
            BaseRow::from_entry(&e, true),
            BaseRow::from_entry(&entry("ARMO", "FULL", 6, "Hat", "Hat", "Chapeau"), false),
        ];
        let v2 = dir.join("v2.ximdb");
        assert_eq!(write_ximdb(rows.clone(), "fra", &v2).unwrap(), 2);
        let text = std::fs::read_to_string(&v2).unwrap();
        assert!(text.starts_with(r#"{"v":4,"lang":"fra""#), "{text}");
        assert!(text.contains(r#""Une note","pending"]"#), "{text}");
        assert!(text.contains(r#""Chapeau"]"#), "{text}");
        assert_eq!(peek_ximdb_lang(&v2).as_deref(), Some("fra"));
        let back = read_ximdb(&v2).unwrap();
        assert_eq!(back.v, XIMDB_FORMAT);
        assert_eq!(back.e, rows);
        let m = ximdb_to_tm(&v2).unwrap();
        assert_eq!(status::derive(&m.entries[0]), Status::Pending);
        assert_eq!(status::derive(&m.entries[1]), Status::BySimilarity);
        // Applied as the mod's own dictionary, the stored status comes back
        // (B-03); a row without one is Validé on an exact match, as before.
        let db = BaseDb::from_file(&v2).unwrap();
        let mut tm = mem(vec![
            entry("BOOK", "FULL", 5, "Note", "A Note", ""),
            entry("ARMO", "FULL", 6, "Hat", "Hat", ""),
        ]);
        assert_eq!(db.auto_translate(&mut tm), 2);
        assert_eq!(status::derive(&tm.entries[0]), Status::Pending);
        assert_eq!(status::derive(&tm.entries[1]), Status::Validated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-B09-03 / T-B09-05: format 4 carries the provenance and the review
    /// of the mod's own rows; they come back with the line; a format 2 file
    /// still reads, and the next write is format 4.
    #[test]
    fn ximdb_v4_keeps_provenance_and_review() {
        let dir = std::env::temp_dir().join(format!("ximod-ximdb-v4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = entry("BOOK", "FULL", 5, "Note", "A Note", "Une note");
        status::set(&mut e, Status::Validated);
        e.prov = "online:deepl".into();
        e.review = "Une petite note".into();
        let plain = entry("ARMO", "FULL", 6, "Hat", "Hat", "Chapeau");
        let rows = vec![
            BaseRow::from_entry(&e, true),
            BaseRow::from_entry(&plain, true),
        ];
        let p = dir.join("v4.ximdb");
        write_ximdb(rows.clone(), "fra", &p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains(r#""Une note","validated",{"p":"online:deepl","r":"Une petite note"}]"#),
            "{text}"
        );
        // A row without provenance nor review keeps seven elements.
        assert!(text.contains(r#""Chapeau","by_similarity"]"#), "{text}");
        assert_eq!(read_ximdb(&p).unwrap().e, rows);
        // A shared base never carries them.
        assert!(BaseRow::from_entry(&e, false).prov.is_empty());
        // Reopened through the mod's dictionary: provenance and review back.
        let db = BaseDb::from_file(&p).unwrap();
        let mut tm = mem(vec![
            entry("BOOK", "FULL", 5, "Note", "A Note", ""),
            entry("ARMO", "FULL", 6, "Hat", "Hat", ""),
        ]);
        assert_eq!(db.auto_translate(&mut tm), 2);
        assert_eq!(tm.entries[0].prov, "online:deepl");
        assert_eq!(tm.entries[0].review, "Une petite note");
        assert!(tm.entries[1].prov.is_empty(), "the stack names the level");
        // As a document, too.
        let m = ximdb_to_tm(&p).unwrap();
        assert_eq!(m.entries[0].prov, "online:deepl");
        // A format 2 file reads; it is rewritten as format 4.
        let v2 = dir.join("v2.ximdb");
        std::fs::write(
            &v2,
            r#"{"v":2,"lang":"fra","e":[["BOOK","FULL",5,"Note","A Note","Une note","pending"]]}"#,
        )
        .unwrap();
        let back = read_ximdb(&v2).unwrap();
        assert_eq!(back.e[0].status.as_deref(), Some("pending"));
        write_ximdb(back.e, "fra", &v2).unwrap();
        assert_eq!(read_ximdb(&v2).unwrap().v, 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-B05-01: text-only pairs match any field, after the exact and the
    /// text tiers; a whole base of them is text-only; the tab import reads
    /// pairs and skips the rest.
    #[test]
    fn text_only_pairs_match_any_field() {
        let dir = std::env::temp_dir().join(format!("ximod-ximdb-b05-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let unsigned = |s: &str, t: &str| BaseRow {
            rec: String::new(),
            field: String::new(),
            formid: 0,
            edid: String::new(),
            source: s.into(),
            trans: t.into(),
            status: None,
            ..Default::default()
        };
        let path = dir.join("fragments_ENG-FRA.ximdb");
        write_ximdb(
            vec![
                unsigned("of Burning", "de feu"),
                unsigned("Steel Sword", "Épée en acier"),
            ],
            "fra",
            &path,
        )
        .unwrap();
        assert!(is_text_only_hint(&path));
        let db = BaseDb::from_file(&path).unwrap();
        assert!(db.is_text_only());
        assert_eq!(db.text_pairs().count(), 2);
        let mut tm = mem(vec![
            entry("WEAP", "FULL", 1, "Sword", "Steel Sword", ""),
            entry("ARMO", "DESC", 2, "Hat", " Steel Sword ", ""),
            entry("WEAP", "FULL", 3, "Sword2", "Steel Sword of Burning", ""),
        ]);
        assert_eq!(db.auto_translate(&mut tm), 2);
        assert_eq!(tm.entries[0].trans, "Épée en acier");
        assert_eq!(
            tm.entries[1].trans, "Épée en acier",
            "any field, edges trimmed"
        );
        assert_eq!(status::derive(&tm.entries[1]), Status::BySimilarity);
        assert!(tm.entries[2].trans.is_empty(), "the whole text must match");
        // A signed base is not text-only, and a mixed one neither.
        let mixed = dir.join("mixed_ENG-FRA.ximdb");
        write_ximdb(
            vec![
                BaseRow::from_entry(
                    &entry("WEAP", "FULL", 1, "Sword", "Steel Sword", "Épée"),
                    false,
                ),
                unsigned("of Burning", "de feu"),
            ],
            "fra",
            &mixed,
        )
        .unwrap();
        assert!(!is_text_only_hint(&mixed));
        let db = BaseDb::from_file(&mixed).unwrap();
        assert!(!db.is_text_only());
        let mut tm = mem(vec![entry("BOOK", "FULL", 9, "X", "of Burning", "")]);
        assert_eq!(
            db.auto_translate(&mut tm),
            1,
            "its unsigned rows still serve"
        );
        // The tab import.
        let txt = dir.join("pairs.txt");
        std::fs::write(
            &txt,
            "# commentaire\nIron\tFer\n\nno tab here\n Steel \t Acier \r\n\tvide\n",
        )
        .unwrap();
        let rows = read_text_pairs(&txt).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[1].source.as_str(), rows[1].trans.as_str()),
            ("Steel", "Acier")
        );
        assert!(rows.iter().all(|r| r.rec.is_empty() && r.field.is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-B07-01: the old data round-trip, roll at each write (dated,
    /// unique, back to current when the line returns, purged by age), and
    /// serve the stack after the current rows with the « ancienne » level.
    #[test]
    fn old_data_roll_and_serve() {
        let dir = std::env::temp_dir().join(format!("ximod-ximdb-b07-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("MonMod_ENG-FRA.ximdb");
        let mut v1 = entry(
            "INFO",
            "NAM1",
            7,
            "Intro",
            "You're finally awake.",
            "Tu es enfin réveillé.",
        );
        status::set(&mut v1, Status::Validated);
        let v1b = entry("WEAP", "FULL", 1, "Sword", "Iron Sword", "Épée en fer");
        let rows_v1 = vec![
            BaseRow::from_entry(&v1, true),
            BaseRow::from_entry(&v1b, true),
        ];
        write_ximdb_full(rows_v1.clone(), Vec::new(), "fra", &path, 0).unwrap();
        // v2: the reply disappeared, the sword stays.
        let rows_v2 = vec![BaseRow::from_entry(&v1b, true)];
        let (_, prev, prev_old) = read_rows_and_old(&path).unwrap();
        let old = roll_old_data(&prev, prev_old, &rows_v2, "2026-10-02", 0);
        assert_eq!(old.len(), 1);
        assert_eq!(
            (
                old[0].row.edid.as_str(),
                old[0].date.as_str(),
                old[0].row.status.as_deref()
            ),
            ("Intro", "2026-10-02", Some("validated"))
        );
        write_ximdb_full(rows_v2.clone(), old, "fra", &path, 0).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""o":[["INFO","NAM1",7,"Intro","You're finally awake.","Tu es enfin réveillé.","validated","2026-10-02"]]"#), "{text}");
        // The index: current rows, then the old ones with their status.
        let db = BaseDb::from_file(&path).unwrap();
        assert_eq!((db.rows(), db.old_rows()), (1, 1));
        let mut tm = mem(vec![
            entry("INFO", "NAM1", 7, "Intro", "You're finally awake.", ""),
            entry("INFO", "NAM1", 8, "Intro2", "You're finally awake.", ""),
        ]);
        let mut levels = Vec::new();
        assert_eq!(
            db.auto_translate_entries_with(&mut tm.entries, |i, l| levels.push((i, l))),
            2
        );
        assert_eq!(
            status::derive(&tm.entries[0]),
            Status::Validated,
            "the line came back with its status"
        );
        assert_eq!(status::derive(&tm.entries[1]), Status::BySimilarity);
        assert_eq!(levels, vec![(0, FillLevel::Old), (1, FillLevel::Old)]);
        // FormID seul reaches the old data too.
        let changed = entry(
            "INFO",
            "NAM1",
            7,
            "Intro",
            "You're finally awake, prisoner.",
            "",
        );
        assert_eq!(
            db.lookup_by_id(&changed),
            Some(("You're finally awake.", "Tu es enfin réveillé."))
        );
        // v3: the reply is back → it leaves the old data; a later roll keeps the earliest date.
        let (_, prev, prev_old) = read_rows_and_old(&path).unwrap();
        let old = roll_old_data(&prev, prev_old, &rows_v1, "2026-11-01", 0);
        assert!(old.is_empty());
        let (_, prev, prev_old) = read_rows_and_old(&path).unwrap();
        let old = roll_old_data(&prev, prev_old.clone(), &[], "2026-11-01", 0);
        assert_eq!(old.len(), 2, "the sword joins, the reply keeps its date");
        assert_eq!(old[0].date, "2026-10-02");
        assert_eq!(old[1].date, "2026-11-01");
        // Purge by age: 1 month keeps only what is at most a month old.
        let old = roll_old_data(&prev, prev_old, &[], "2026-11-20", 1);
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].row.edid, "Sword");
        // A reader of the old data: a v2 file without `o` still reads.
        let plain = dir.join("plain.ximdb");
        write_ximdb(rows_v1, "fra", &plain).unwrap();
        let (_, _, o) = read_rows_and_old(&plain).unwrap();
        assert!(o.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-B03-01: the working dictionary restores Ignoré (even without a
    /// translation) and Personnalisé; a loose match stays « par similarité »;
    /// the same file scanned as a shared base restores nothing.
    #[test]
    fn mod_dictionary_restores_the_statuses() {
        let dir = std::env::temp_dir().join(format!("ximod-ximdb-b03-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut ignored = entry("WEAP", "FULL", 1, "Sword", "Sword", "");
        status::set(&mut ignored, Status::Ignored);
        let mut custom = entry("WEAP", "FULL", 2, "Axe", "Axe", "Hache");
        status::set(&mut custom, Status::Custom);
        let mut validated = entry("WEAP", "FULL", 3, "Bow", "Bow", "Arc");
        status::set(&mut validated, Status::Validated);
        let mut pending = entry("WEAP", "FULL", 4, "Mace", "Mace", "");
        status::set(&mut pending, Status::Pending);
        let rows = vec![
            BaseRow::from_entry(&ignored, true),
            BaseRow::from_entry(&custom, true),
            BaseRow::from_entry(&validated, true),
            BaseRow::from_entry(&pending, true),
        ];
        let path = dir.join("MonMod_ENG-FRA.ximdb");
        write_ximdb(rows, "fra", &path).unwrap();

        let db = BaseDb::from_file(&path).unwrap();
        let mut tm = mem(vec![
            entry("WEAP", "FULL", 1, "Sword", "Sword", ""),
            entry("WEAP", "FULL", 2, "Axe", "Axe", ""),
            // Same text, another record: loose tier.
            entry("WEAP", "FULL", 9, "Bow2", "Bow", ""),
            entry("WEAP", "FULL", 4, "Mace", "Mace", ""),
        ]);
        assert_eq!(db.auto_translate(&mut tm), 4);
        assert_eq!(
            status::derive(&tm.entries[3]),
            Status::Pending,
            "En attente without a text comes back"
        );
        assert!(tm.entries[3].trans.is_empty());
        assert_eq!(status::derive(&tm.entries[0]), Status::Ignored);
        assert!(tm.entries[0].trans.is_empty());
        assert_eq!(status::derive(&tm.entries[1]), Status::Custom);
        assert_eq!(tm.entries[1].trans, "Hache");
        assert_eq!(status::derive(&tm.entries[2]), Status::BySimilarity);
        assert_eq!(tm.entries[2].trans, "Arc");

        // Scanned as a shared base: translations only, Validé on exact.
        let shared = BaseDb::scan(std::slice::from_ref(&dir), "fra");
        let mut tm = mem(vec![
            entry("WEAP", "FULL", 1, "Sword", "Sword", ""),
            entry("WEAP", "FULL", 2, "Axe", "Axe", ""),
        ]);
        assert_eq!(
            shared.auto_translate(&mut tm),
            1,
            "the Ignoré row has no translation to give"
        );
        assert_eq!(status::derive(&tm.entries[0]), Status::ToTranslate);
        assert_eq!(status::derive(&tm.entries[1]), Status::Validated);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
