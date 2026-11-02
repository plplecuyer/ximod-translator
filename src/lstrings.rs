//! Localiser un mod (C-09): the fields a game reads as string identifiers
//! in a localized plugin (its « champs localisables », each with the list
//! it goes in — STRINGS, DLSTRINGS or ILSTRINGS), derived from the game's
//! own localized masters when the bases are compiled, and the conversion
//! of a plugin whose texts are inline into a localized plugin and its
//! STRINGS, the other way round of the delocalization.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::games::{FieldCtx, FieldMode, Game, registry};
use crate::plugin::record::{FLAG_COMPRESSED, FLAG_LOCALIZED};
use crate::tm::TranslationMemory;
use crate::tm::strings::StringsKind;

/// The longest text a STRINGS file holds (R-C09-05).
pub const MAX_TEXT_BYTES: usize = 0xFFFF;

/// One localizable field: its list, and the condition that makes it one
/// (a game setting is a string only when its EDID starts with « s »).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub list: StringsKind,
    pub edid_prefix: Option<String>,
}

/// The localizable fields of a game (R-C09-01).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub game: String,
    /// The master the table was derived from; empty for the shipped one.
    pub derived_from: String,
    pub date: String,
    pub fields: HashMap<String, Field>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum FieldJson {
    List(String),
    Cond {
        list: String,
        #[serde(
            rename = "edidPrefix",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        edid_prefix: Option<String>,
    },
}

#[derive(Serialize, Deserialize)]
struct TableJson {
    format: u32,
    game: String,
    #[serde(default)]
    derived_from: String,
    #[serde(default)]
    date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    note: String,
    fields: std::collections::BTreeMap<String, FieldJson>,
}

/// The file name of a game's table.
pub fn file_name(game: Game) -> String {
    format!("{}.json", game.id())
}

fn list_name(k: StringsKind) -> &'static str {
    match k {
        StringsKind::Strings => "strings",
        StringsKind::DlStrings => "dlstrings",
        StringsKind::IlStrings => "ilstrings",
    }
}

fn list_of(name: &str) -> Option<StringsKind> {
    match name.trim().to_lowercase().as_str() {
        "strings" => Some(StringsKind::Strings),
        "dlstrings" => Some(StringsKind::DlStrings),
        "ilstrings" => Some(StringsKind::IlStrings),
        _ => None,
    }
}

impl Table {
    pub fn parse(text: &str) -> Option<Table> {
        let j: TableJson = serde_json::from_str(text).ok()?;
        let mut fields = HashMap::new();
        for (sig, f) in j.fields {
            let field = match f {
                FieldJson::List(l) => Field {
                    list: list_of(&l)?,
                    edid_prefix: None,
                },
                FieldJson::Cond { list, edid_prefix } => Field {
                    list: list_of(&list)?,
                    edid_prefix,
                },
            };
            fields.insert(sig.to_uppercase(), field);
        }
        Some(Table {
            game: j.game,
            derived_from: j.derived_from,
            date: j.date,
            fields,
        })
    }

    pub fn to_json(&self) -> String {
        let fields = self
            .fields
            .iter()
            .map(|(k, f)| {
                let v = match &f.edid_prefix {
                    Some(p) => FieldJson::Cond {
                        list: list_name(f.list).into(),
                        edid_prefix: Some(p.clone()),
                    },
                    None => FieldJson::List(list_name(f.list).into()),
                };
                (k.clone(), v)
            })
            .collect();
        serde_json::to_string_pretty(&TableJson {
            format: 1,
            game: self.game.clone(),
            derived_from: self.derived_from.clone(),
            date: self.date.clone(),
            note: String::new(),
            fields,
        })
        .unwrap_or_default()
    }

    /// The table of a game: the one derived from its masters (in the user's
    /// data folder) when there is one, else the shipped one; `None` for a
    /// game without STRINGS.
    pub fn load(game: Game, user_dir: &Path) -> Option<Table> {
        let name = file_name(game);
        let user = user_dir.join("lstrings").join(&name);
        if let Some(t) = std::fs::read_to_string(&user)
            .ok()
            .and_then(|s| Table::parse(&s))
        {
            return Some(t);
        }
        let shipped = crate::data::find_data_file(&format!("lstrings/{name}"))?;
        Table::parse(&std::fs::read_to_string(shipped).ok()?)
    }

    /// The table of a game for the writers that do not know the user's
    /// folder (the delocalization): the derived one when the configuration
    /// is set up, else the shipped one.
    pub fn shipped_or_derived(game: Game) -> Option<Table> {
        Table::load(game, &crate::config::paths::get().data_dir)
    }

    /// Where the derived table of a game is written.
    pub fn user_path(game: Game, user_dir: &Path) -> PathBuf {
        user_dir.join("lstrings").join(file_name(game))
    }

    /// The list of a field of a record, if the engine reads it as an
    /// identifier.
    pub fn list(&self, rec: &str, fld: &str, edid: &str) -> Option<StringsKind> {
        let f = self.fields.get(&format!("{rec}:{fld}"))?;
        match &f.edid_prefix {
            Some(p) if !edid.starts_with(p.as_str()) => None,
            _ => Some(f.list),
        }
    }

    /// The number of fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

// --------------------------------------------------------------------------
// Walking a plugin without copying it
// --------------------------------------------------------------------------

fn u32_at(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

/// Call `f(sig, edid, fields)` for every record of a plugin (GRUPs walked,
/// compressed records inflated), the header record left out.
fn for_each_record(
    data: &[u8],
    mut f: impl FnMut(&str, &str, &[crate::plugin::record::SubRecord]),
) -> Result<()> {
    let hl = crate::plugin::reader::detect_header_len(data);
    let mut g = |sig: &str, edid: &str, subs: &[crate::plugin::record::SubRecord]| {
        if sig != "TES4" {
            f(sig, edid, subs);
        }
    };
    for_each_record_in(data, hl, 0, data.len(), &mut g)
}

fn for_each_record_in(
    data: &[u8],
    hl: usize,
    start: usize,
    end: usize,
    f: &mut impl FnMut(&str, &str, &[crate::plugin::record::SubRecord]),
) -> Result<()> {
    let mut p = start;
    while p + hl <= end {
        let size = u32_at(data, p + 4) as usize;
        if &data[p..p + 4] == b"GRUP" {
            if size < hl || p + size > end {
                bail!(crate::l10n!("err-plugin-grup-size", offset = p));
            }
            for_each_record_in(data, hl, p + hl, p + size, f)?;
            p += size;
        } else {
            if p + hl + size > end {
                bail!(crate::l10n!("err-plugin-record", offset = p));
            }
            visit(data, hl, p, size, f)?;
            p += hl + size;
        }
    }
    Ok(())
}

fn visit(
    data: &[u8],
    hl: usize,
    p: usize,
    size: usize,
    f: &mut impl FnMut(&str, &str, &[crate::plugin::record::SubRecord]),
) -> Result<()> {
    let sig: String = data[p..p + 4].iter().map(|&c| c as char).collect();
    let flags = u32_at(data, p + 8);
    let raw = &data[p + hl..p + hl + size];
    let body;
    let fields_bytes: &[u8] = if flags & FLAG_COMPRESSED != 0 {
        body = crate::plugin::reader::decompress_record(raw)?;
        &body
    } else {
        raw
    };
    let subs = crate::plugin::reader::parse_subrecords(fields_bytes);
    let edid = subs
        .iter()
        .find(|s| s.sig == "EDID")
        .map(|s| s.as_zstring())
        .unwrap_or_default();
    f(&sig, &edid, &subs);
    Ok(())
}

// --------------------------------------------------------------------------
// Derivation (R-C09-01, decision D-62)
// --------------------------------------------------------------------------

/// The signatures that hold text in every record that has them.
const ALWAYS_TEXT: &[&str] = &[
    "FULL", "DESC", "NAM1", "SHRT", "ITXT", "BTXT", "RDMP", "EPF2", "ATTX",
];

/// Whether a signature holds text in every record that has it (`FULL`,
/// `DESC`…).
pub fn always_text(fld: &str) -> bool {
    ALWAYS_TEXT.contains(&fld)
}

/// Whether a field may be localizable: a field of the translatable table,
/// or a signature that is text wherever it appears. A flag, a counter or a
/// reference of another signature may hold small numbers that are
/// identifiers by chance (`CELL:DATA`).
fn candidate(fields: &crate::games::FieldTable, rec: &str, fld: &str) -> bool {
    fields.limits(rec, fld).is_some() || ALWAYS_TEXT.contains(&fld)
}

#[derive(Default)]
struct Tally {
    total: usize,
    hits: [usize; 3],
    values: HashSet<u32>,
    /// Per first letter of the EDID: (total, hits) — a game setting is a
    /// string by its EDID.
    by_letter: HashMap<char, (usize, usize)>,
}

/// Derive the table of a game from one of its localized masters and the
/// identifiers of its STRINGS, DLSTRINGS and ILSTRINGS (`ids[list_index]`):
/// a field whose 4-byte values are nearly all identifiers of one list, and
/// mostly distinct (a counter repeats its values, an identifier does not),
/// is localizable, in that list. A field that is so only for the records
/// whose EDID begins with one letter (the game settings) gets the condition.
pub fn derive(
    game: Game,
    master_name: &str,
    master: &[u8],
    ids: &[HashSet<u32>; 3],
    date: &str,
) -> Result<Table> {
    let mut tallies: HashMap<String, Tally> = HashMap::new();
    let reg = registry();
    let texts = reg.fields(game);
    for_each_record(master, |rec, edid, subs| {
        for s in subs {
            if s.data.len() != 4 || !candidate(texts, rec, &s.sig) {
                continue;
            }
            let v = u32::from_le_bytes([s.data[0], s.data[1], s.data[2], s.data[3]]);
            let t = tallies.entry(format!("{rec}:{}", s.sig)).or_default();
            t.total += 1;
            let letter = edid.chars().next().unwrap_or(' ');
            let bl = t.by_letter.entry(letter).or_insert((0, 0));
            bl.0 += 1;
            if v == 0 {
                continue;
            }
            let mut hit = false;
            for (k, set) in ids.iter().enumerate() {
                if set.contains(&v) {
                    t.hits[k] += 1;
                    hit = true;
                }
            }
            if hit {
                bl.1 += 1;
                t.values.insert(v);
            }
        }
    })?;
    let mut fields = HashMap::new();
    for (sig, t) in tallies {
        let (best, n) = t
            .hits
            .iter()
            .enumerate()
            .max_by_key(|(_, n)| **n)
            .map(|(k, n)| (k, *n))
            .unwrap_or((0, 0));
        if n == 0 {
            continue;
        }
        let list = match best {
            0 => StringsKind::Strings,
            1 => StringsKind::DlStrings,
            _ => StringsKind::IlStrings,
        };
        let distinct = t.values.len() * 2 >= n;
        if n * 10 >= t.total * 9 && distinct {
            fields.insert(
                sig,
                Field {
                    list,
                    edid_prefix: None,
                },
            );
            continue;
        }
        // Only some records: one EDID letter whose values are all ids.
        if sig.starts_with("GMST:")
            && let Some((letter, _)) = t
                .by_letter
                .iter()
                .filter(|(_, (tot, hits))| *tot > 0 && hits * 10 >= tot * 9)
                .max_by_key(|(_, (_, hits))| *hits)
        {
            fields.insert(
                sig,
                Field {
                    list,
                    edid_prefix: Some(letter.to_string()),
                },
            );
        }
    }
    Ok(Table {
        game: game.id().to_string(),
        derived_from: master_name.to_string(),
        date: date.to_string(),
        fields,
    })
}

// --------------------------------------------------------------------------
// Conversion (R-C09-02 to R-C09-05)
// --------------------------------------------------------------------------

/// What a conversion did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Identifiers given (1 to n).
    pub ids: u32,
    /// Texts per list: target language / source language.
    pub per_list: [usize; 3],
    /// String fields the table does not know (REC:FLD, count).
    pub unknown: Vec<(String, usize)>,
}

/// The plugin written, the target texts and the source texts of each list.
pub struct Localized {
    pub plugin: Vec<u8>,
    pub target: Vec<(StringsKind, Vec<(u32, String)>)>,
    pub source: Vec<(StringsKind, Vec<(u32, String)>)>,
    pub report: Report,
}

/// Effective translations, `FormID → (field, index) → text`.
fn translations(tm: &TranslationMemory) -> HashMap<u32, HashMap<(String, i32), String>> {
    let mut idx: HashMap<u32, HashMap<(String, i32), String>> = HashMap::new();
    for e in &tm.entries {
        if e.formid != 0 && e.is_translated() {
            idx.entry(e.formid)
                .or_default()
                .insert((e.field_sig.clone(), e.index), e.trans.clone());
        }
    }
    idx
}

/// Whether some bytes read as a text (a NUL-terminated string of printable
/// characters), not a number or a reference.
fn looks_like_text(d: &[u8]) -> bool {
    // Four bytes are as likely a reference or a number.
    if d.len() < 5 || *d.last().unwrap_or(&1) != 0 {
        return false;
    }
    let body = &d[..d.len() - 1];
    let Ok(text) = std::str::from_utf8(body) else {
        // A page of code: printable bytes only.
        return !body.contains(&0) && body.iter().all(|&b| b >= 0x20);
    };
    // A path or a file name is not a text of the game.
    if text.contains('\\') || text.contains('/') {
        return false;
    }
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    letters >= 2
        && text
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
}

/// Localize a plugin whose texts are inline (R-C09-02): every localizable
/// field gets an identifier, in file order from 1, its text going to the
/// list of the field — the translation of an exported line in `target`,
/// the original text in `source`; an empty field gets 0. The header is
/// flagged Localized; the records that change are written uncompressed;
/// the others stay byte for byte.
pub fn localize(
    source: &[u8],
    tm: &TranslationMemory,
    table: &Table,
    game: Game,
) -> Result<Localized> {
    let plugin = crate::plugin::reader::parse(source)?;
    if plugin.localized {
        bail!(crate::l10n!("err-localize-already"));
    }
    let trans = translations(tm);
    let reg = registry();
    let fields = reg.fields(game);
    let mut tree = crate::plugin::tree::parse_tree(source, plugin.header_len)?;
    let mut next: u32 = 1;
    let mut target: [Vec<(u32, String)>; 3] = Default::default();
    let mut src_out: [Vec<(u32, String)>; 3] = Default::default();
    let mut unknown: HashMap<String, usize> = HashMap::new();
    let mut too_long: Option<(String, String)> = None;
    for rec in tree.records_mut() {
        let sig = rec.sig_str();
        if sig == "TES4" {
            continue;
        }
        let compressed = rec.flags() & FLAG_COMPRESSED != 0;
        let body = if compressed {
            crate::plugin::reader::decompress_record(&rec.data)?
        } else {
            rec.data.clone()
        };
        let mut subs = crate::plugin::write::parse_fields(&body);
        let edid = subs
            .iter()
            .find(|s| s.0 == "EDID")
            .map(|s| zstring(&s.1))
            .unwrap_or_default();
        let snapshot: Vec<(String, String)> =
            subs.iter().map(|(k, v)| (k.clone(), zstring(v))).collect();
        let other = |f: &str| {
            snapshot
                .iter()
                .find(|(k, _)| k == f)
                .map(|(_, v)| v.clone())
        };
        let rec_trans = trans.get(&rec.formid());
        let mut counters: HashMap<String, i32> = HashMap::new();
        let mut changed = false;
        for (fsig, data) in subs.iter_mut() {
            // The scripts' strings stay in the block (C-08), translated.
            if fsig == crate::vmad_lock::FIELD {
                if let Some(b) = rec_trans.and_then(|m| {
                    crate::plugin::vmad::translate_record(&sig, data, m, game.uses_utf8())
                }) {
                    *data = b;
                    changed = true;
                }
                continue;
            }
            let Some(list) = table.list(&sig, fsig, &edid) else {
                if fields.is_string_field(fsig) && looks_like_text(data) {
                    *unknown.entry(format!("{sig}:{fsig}")).or_insert(0) += 1;
                }
                continue;
            };
            let text = zstring(data);
            // The instance index of extraction: kept entries only.
            let ctx = FieldCtx {
                edid: &edid,
                value: &text,
                other: &other,
            };
            let kept = !text.is_empty()
                && fields.limits(&sig, fsig).is_some()
                && fields.mode(&sig, fsig, &ctx) != FieldMode::Ignore;
            let idx = if kept {
                let c = counters.entry(fsig.clone()).or_insert(0);
                *c += 1;
                Some(*c)
            } else {
                None
            };
            let id = if text.is_empty() {
                0
            } else {
                let id = next;
                next += 1;
                let shown = idx
                    .and_then(|i| rec_trans.and_then(|m| m.get(&(fsig.clone(), i))))
                    .cloned()
                    .unwrap_or_else(|| text.clone());
                if crate::tm::encoding::encode(&shown, game.uses_utf8()).len() >= MAX_TEXT_BYTES
                    && too_long.is_none()
                {
                    too_long = Some((edid.clone(), fsig.clone()));
                }
                let k = list.list_index() as usize;
                target[k].push((id, shown));
                src_out[k].push((id, text));
                id
            };
            *data = id.to_le_bytes().to_vec();
            changed = true;
        }
        if !changed {
            continue;
        }
        rec.data = crate::plugin::write::serialize_fields(&subs);
        if compressed {
            let flags = rec.flags() & !FLAG_COMPRESSED;
            rec.set_flags(flags);
        }
    }
    if let Some((edid, field)) = too_long {
        bail!(crate::l10n!(
            "err-localize-too-long",
            edid = edid,
            field = field
        ));
    }
    if let Some(crate::plugin::tree::Node::Record(tes4)) = tree.nodes.first_mut() {
        let flags = tes4.flags() | FLAG_LOCALIZED;
        tes4.set_flags(flags);
    }
    let mut report = Report {
        ids: next - 1,
        ..Default::default()
    };
    let kinds = [
        StringsKind::Strings,
        StringsKind::DlStrings,
        StringsKind::IlStrings,
    ];
    for (k, list) in target.iter().enumerate() {
        report.per_list[k] = list.len();
    }
    let mut unknown: Vec<(String, usize)> = unknown.into_iter().collect();
    unknown.sort();
    report.unknown = unknown;
    let pack = |lists: [Vec<(u32, String)>; 3]| -> Vec<(StringsKind, Vec<(u32, String)>)> {
        lists
            .into_iter()
            .enumerate()
            .filter(|(_, l)| !l.is_empty())
            .map(|(k, l)| (kinds[k], l))
            .collect()
    };
    Ok(Localized {
        plugin: tree.write(),
        target: pack(target),
        source: pack(src_out),
        report,
    })
}

fn zstring(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    crate::tm::encoding::decode(&data[..end], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_round_trip_and_condition() {
        let text = r#"{ "format": 1, "game": "SkyrimSE", "fields": {
            "ACTI:FULL": "strings", "BOOK:DESC": "dlstrings", "INFO:NAM1": "ilstrings",
            "GMST:DATA": { "list": "strings", "edidPrefix": "s" } } }"#;
        let t = Table::parse(text).unwrap();
        assert_eq!(t.list("BOOK", "DESC", ""), Some(StringsKind::DlStrings));
        assert_eq!(t.list("INFO", "NAM1", ""), Some(StringsKind::IlStrings));
        assert_eq!(t.list("GMST", "DATA", "sGold"), Some(StringsKind::Strings));
        assert_eq!(t.list("GMST", "DATA", "fGold"), None);
        assert_eq!(t.list("WEAP", "EDID", ""), None);
        let back = Table::parse(&t.to_json()).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn shipped_tables_parse() {
        for g in ["Skyrim", "SkyrimSE", "Fallout4", "Starfield"] {
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/data/lstrings")
                .join(format!("{g}.json"));
            let t = Table::parse(&std::fs::read_to_string(&p).unwrap()).unwrap();
            assert!(t.len() > 80, "{g}");
            assert_eq!(
                t.list("INFO", "NAM1", ""),
                Some(StringsKind::IlStrings),
                "{g}"
            );
            assert_eq!(
                t.list("INFO", "RNAM", ""),
                Some(StringsKind::Strings),
                "{g}"
            );
            assert_eq!(
                t.list("BOOK", "DESC", ""),
                Some(StringsKind::DlStrings),
                "{g}"
            );
            assert_eq!(
                t.list("ACTI", "FULL", ""),
                Some(StringsKind::Strings),
                "{g}"
            );
        }
    }

    #[test]
    fn text_or_not() {
        assert!(looks_like_text(b"Iron Sword\0"));
        assert!(!looks_like_text(&[1, 0, 0, 0]));
        assert!(!looks_like_text(b"\0"));
    }
}
