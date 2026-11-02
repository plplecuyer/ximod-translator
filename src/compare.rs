//! Comparer deux dictionnaires (D-07, R-D07-04 / 05 / 06, D-89): two
//! translation files — or two documents — put side by side: the sources
//! they share, the translations identical or divergent, the lines of one
//! side only; a CSV report and a merge into a dictionary whose divergent
//! lines are marked ambiguous, to be settled in the grid.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::basedb::BaseRow;
use crate::status::{self, Status};
use crate::tm::{TmEntry, TranslationMemory};

/// How the lines of the two sides are paired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Record, field, identifier (FormID or EDID), rank — and the source
    /// when both sides are translations.
    Record,
    /// The source text alone.
    Text,
}

/// What a line of the report is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Same,
    Diff,
    OnlyA,
    OnlyB,
}

impl Category {
    pub fn code(self) -> &'static str {
        match self {
            Category::Same => "same",
            Category::Diff => "diff",
            Category::OnlyA => "only_a",
            Category::OnlyB => "only_b",
        }
    }
}

/// One source of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub category: Category,
    pub rec: String,
    pub field: String,
    pub edid: String,
    pub formid: u32,
    pub source: String,
    /// The distinct translations of each side (the text, for a side that
    /// has none: a translated plugin).
    pub a: Vec<String>,
    pub b: Vec<String>,
    /// The line of each side (its first), for the grid.
    pub line_a: Option<usize>,
    pub line_b: Option<usize>,
    /// Both sides give the status Validé.
    pub validated: bool,
}

/// The comparison (R-D07-04).
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The mode used (Record falls back to Text without signatures).
    pub mode: Option<Mode>,
    pub a_lines: usize,
    pub b_lines: usize,
    pub rows: Vec<Row>,
}

impl Report {
    pub fn count(&self, c: Category) -> usize {
        self.rows.iter().filter(|r| r.category == c).count()
    }

    pub fn common(&self) -> usize {
        self.count(Category::Same) + self.count(Category::Diff)
    }
}

/// A side has translations (a dictionary) or only texts (a plugin).
fn bilingual(tm: &TranslationMemory) -> bool {
    tm.entries.iter().any(|e| !e.trans.trim().is_empty())
}

fn has_sigs(tm: &TranslationMemory) -> bool {
    tm.entries.iter().any(|e| !e.rec_sig.is_empty())
}

fn norm(s: &str, fold: bool) -> String {
    if fold {
        s.trim().to_lowercase()
    } else {
        s.to_string()
    }
}

fn workable(e: &TmEntry) -> bool {
    !e.unused && !e.source.trim().is_empty()
}

/// The identifier of a record: its FormID without the load index, else its
/// EDID.
fn ident(e: &TmEntry) -> String {
    if e.formid != 0 {
        format!("{:06X}", e.formid & 0x00FF_FFFF)
    } else {
        e.edid.to_lowercase()
    }
}

struct Group {
    first: usize,
    values: Vec<String>,
    validated: bool,
}

fn groups(
    tm: &TranslationMemory,
    mode: Mode,
    with_source: bool,
    bi: bool,
    fold: bool,
) -> (Vec<String>, HashMap<String, Group>) {
    let mut order = Vec::new();
    let mut map: HashMap<String, Group> = HashMap::new();
    for (i, e) in tm.entries.iter().enumerate() {
        if !workable(e) {
            continue;
        }
        let key = match mode {
            Mode::Text => norm(&e.source, fold),
            Mode::Record => {
                let mut k = format!(
                    "{}\u{1f}{}\u{1f}{}\u{1f}{}",
                    e.rec_sig,
                    e.field_sig,
                    ident(e),
                    e.index
                );
                if with_source {
                    k.push('\u{1f}');
                    k.push_str(&norm(&e.source, fold));
                }
                k
            }
        };
        let value = if bi {
            e.trans.clone()
        } else {
            e.source.clone()
        };
        let valid = status::derive(e) == Status::Validated;
        match map.get_mut(&key) {
            Some(g) => {
                if !value.trim().is_empty() && !g.values.contains(&value) {
                    g.values.push(value);
                }
                g.validated &= valid;
            }
            None => {
                order.push(key.clone());
                map.insert(
                    key,
                    Group {
                        first: i,
                        values: if value.trim().is_empty() {
                            Vec::new()
                        } else {
                            vec![value]
                        },
                        validated: valid,
                    },
                );
            }
        }
    }
    (order, map)
}

/// Compare two sides (R-D07-04): `mode` Record falls back to Text when a
/// side has no signatures (D-89); `fold` ignores case and edge spaces in
/// the sources and in the verdict.
pub fn compare(a: &TranslationMemory, b: &TranslationMemory, mode: Mode, fold: bool) -> Report {
    let mode = if mode == Mode::Record && has_sigs(a) && has_sigs(b) {
        Mode::Record
    } else {
        Mode::Text
    };
    let (bi_a, bi_b) = (bilingual(a), bilingual(b));
    let with_source = bi_a && bi_b;
    let (order_a, ga) = groups(a, mode, with_source, bi_a, fold);
    let (order_b, gb) = groups(b, mode, with_source, bi_b, fold);
    let set = |v: &[String]| -> BTreeSet<String> { v.iter().map(|s| norm(s, fold)).collect() };
    let row = |c: Category, e: &TmEntry, a: Vec<String>, b: Vec<String>, la, lb, v| Row {
        category: c,
        rec: e.rec_sig.clone(),
        field: e.field_sig.clone(),
        edid: e.edid.clone(),
        formid: e.formid,
        source: e.source.clone(),
        a,
        b,
        line_a: la,
        line_b: lb,
        validated: v,
    };
    let mut rows = Vec::new();
    for k in &order_a {
        let g = &ga[k];
        let e = &a.entries[g.first];
        match gb.get(k) {
            Some(h) => {
                let c = if set(&g.values) == set(&h.values) {
                    Category::Same
                } else {
                    Category::Diff
                };
                rows.push(row(
                    c,
                    e,
                    g.values.clone(),
                    h.values.clone(),
                    Some(g.first),
                    Some(h.first),
                    g.validated && h.validated,
                ));
            }
            None => rows.push(row(
                Category::OnlyA,
                e,
                g.values.clone(),
                Vec::new(),
                Some(g.first),
                None,
                false,
            )),
        }
    }
    for k in &order_b {
        if ga.contains_key(k) {
            continue;
        }
        let h = &gb[k];
        rows.push(row(
            Category::OnlyB,
            &b.entries[h.first],
            Vec::new(),
            h.values.clone(),
            None,
            Some(h.first),
            false,
        ));
    }
    Report {
        mode: Some(mode),
        a_lines: a.entries.iter().filter(|e| workable(e)).count(),
        b_lines: b.entries.iter().filter(|e| workable(e)).count(),
        rows,
    }
}

/// The report as CSV (R-D07-05): UTF-8 with BOM, `;`, a header, the
/// category as a code, several translations joined by « | ».
pub fn to_csv(r: &Report, file_a: &str, file_b: &str) -> String {
    use crate::scan::csv_cell as c;
    let mut s = String::from(
        "\u{feff}category;record;field;edid;source;translation_a;translation_b;file_a;file_b\r\n",
    );
    for row in &r.rows {
        let cells = [
            row.category.code().to_string(),
            c(&row.rec),
            c(&row.field),
            c(&row.edid),
            c(&row.source),
            c(&row.a.join(" | ")),
            c(&row.b.join(" | ")),
            c(file_a),
            c(file_b),
        ];
        s.push_str(&cells.join(";"));
        s.push_str("\r\n");
    }
    s
}

/// Which side wins a divergent line in the merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefer {
    A,
    B,
}

/// The rows of the merge (R-D07-06): every source of either side, once —
/// identical as they are, divergent with the preferred side's translation
/// and the ambiguity mark « |amb:2 », the one side's otherwise; En attente
/// but where both sides say Validé.
pub fn merge(r: &Report, prefer: Prefer, file: &str) -> (Vec<BaseRow>, usize) {
    let mut out = Vec::new();
    let mut amb = 0usize;
    let base = crate::provenance::dict(
        crate::dicts::Role::Mod,
        crate::basedb::FillLevel::Exact,
        file,
    );
    for row in &r.rows {
        let (trans, ambiguous) = match row.category {
            Category::Same => (row.a.first().or(row.b.first()).cloned(), false),
            Category::Diff => {
                let (first, other) = match prefer {
                    Prefer::A => (&row.a, &row.b),
                    Prefer::B => (&row.b, &row.a),
                };
                (first.first().or(other.first()).cloned(), true)
            }
            Category::OnlyA => (row.a.first().cloned(), row.a.len() > 1),
            Category::OnlyB => (row.b.first().cloned(), row.b.len() > 1),
        };
        let Some(trans) = trans.filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        let st = if row.validated && !ambiguous {
            Status::Validated
        } else {
            Status::Pending
        };
        if ambiguous {
            amb += 1;
        }
        out.push(BaseRow {
            rec: row.rec.clone(),
            field: row.field.clone(),
            formid: row.formid,
            edid: row.edid.clone(),
            source: row.source.clone(),
            trans,
            status: Some(st.key().to_string()),
            prov: if ambiguous {
                crate::ambiguity::mark(&base, 2)
            } else {
                base.clone()
            },
            review: String::new(),
        });
    }
    (out, amb)
}

/// Write the merge as a dictionary (format 4).
pub fn write_merge(rows: Vec<BaseRow>, lang: &str, out: &Path) -> Result<usize, String> {
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    crate::basedb::write_ximdb_with(
        rows,
        Vec::new(),
        lang,
        out,
        0,
        None,
        &crate::basedb::HeaderExtra::default(),
    )
}

/// Read a side from a file: a dictionary (ximdb), a translation base (DSD,
/// pivot, SST, EET, XML), a plugin (its texts, STRINGS resolved).
pub fn read_side(path: &Path, source: &str) -> Result<TranslationMemory, String> {
    if crate::basedb::is_ximdb_path(path) {
        return crate::basedb::ximdb_to_tm(path);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if crate::games::registry().is_plugin_extension(&ext) {
        let mut tm = crate::plugin::read_file_as(path, None).map_err(|e| e.to_string())?;
        if tm.entries.iter().any(|e| e.idstexte >= 0) {
            crate::headless::resolve_strings(path, &mut tm, source);
        }
        return Ok(tm);
    }
    crate::tm::load(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(rec: &str, formid: u32, src: &str, tr: &str) -> TmEntry {
        TmEntry {
            rec_sig: rec.into(),
            field_sig: "FULL".into(),
            formid,
            source: src.into(),
            trans: tr.into(),
            ..Default::default()
        }
    }

    fn tm(entries: Vec<TmEntry>) -> TranslationMemory {
        TranslationMemory {
            entries,
            ..Default::default()
        }
    }

    #[test]
    fn categories_fold_and_merge() {
        let a = tm(vec![
            e("WEAP", 0x0200_0001, "Sword", "Épée"),
            e("WEAP", 0x0200_0002, "Bow", "arc "),
            e("WEAP", 0x0200_0003, "Axe", "Hache"),
            e("WEAP", 0x0200_0004, "Mace", "Masse"),
        ]);
        let b = tm(vec![
            e("WEAP", 0x0300_0001, "Sword", "Épée"),
            e("WEAP", 0x0300_0002, "Bow", "Arc"),
            e("WEAP", 0x0300_0003, "Axe", "Cognée"),
            e("ARMO", 0x0300_0005, "Helm", "Casque"),
        ]);
        let r = compare(&a, &b, Mode::Record, true);
        assert_eq!(r.mode, Some(Mode::Record));
        assert_eq!(
            (
                r.count(Category::Same),
                r.count(Category::Diff),
                r.count(Category::OnlyA),
                r.count(Category::OnlyB)
            ),
            (2, 1, 1, 1)
        );
        let strict = compare(&a, &b, Mode::Record, false);
        assert_eq!(strict.count(Category::Same), 1);
        let (rows, amb) = merge(&r, Prefer::B, "m.ximdb");
        assert_eq!((rows.len(), amb), (5, 1));
        let axe = rows.iter().find(|x| x.source == "Axe").unwrap();
        assert_eq!(axe.trans, "Cognée");
        assert!(axe.prov.ends_with("|amb:2"));
        let csv = to_csv(&r, "a.json", "b.json");
        assert!(csv.starts_with("\u{feff}category;"));
        assert_eq!(csv.lines().count(), 6);
        // A translated plugin (texts only) against a dictionary: by record.
        let fr = tm(vec![
            e("WEAP", 0x0100_0001, "Épée", ""),
            e("WEAP", 0x0100_0003, "Hache", ""),
        ]);
        let r = compare(&a, &fr, Mode::Record, true);
        assert_eq!(r.count(Category::Same), 2);
        // Text alone without signatures.
        let mut t = b.clone();
        for x in &mut t.entries {
            x.rec_sig.clear();
        }
        assert_eq!(compare(&a, &t, Mode::Record, true).mode, Some(Mode::Text));
    }
}
