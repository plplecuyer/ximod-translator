//! The Relecture column (A-17): a reviewer's text on every line, beside the
//! translation, that the translator accepts or not line by line. Pure
//! library: the commands on a set of lines and the loading of a reviewed
//! copy (matching, preview, changes); the interface makes them undoable
//! operations.

use std::collections::HashMap;
use std::path::Path;

use crate::status::{self, Status};
use crate::tm::{TmEntry, TranslationMemory};

/// Whether a line can take part in a review command: shown in game and not
/// deliberately ignored (as the other commands of the row menu, R-M01-08).
fn workable(e: &TmEntry) -> bool {
    !e.unused && status::derive(e) != Status::Ignored
}

/// Whether the review of a line differs from its translation (the accent
/// colour of the column, the ≠ filter).
pub fn differs(e: &TmEntry) -> bool {
    !e.review.is_empty() && e.review != e.trans
}

/// « Traduction = Relecture » (R-A17-03): a non-empty review different from
/// the translation replaces it; the line becomes Validé (decision D-45) —
/// En attente when Validé is not possible (a text equal to the source) —
/// with the provenance « relecture »; the review is kept. Returns the lines
/// changed.
pub fn accept(entries: &mut [TmEntry], lines: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    for &i in lines {
        let Some(e) = entries.get_mut(i) else {
            continue;
        };
        if !workable(e) || !differs(e) {
            continue;
        }
        e.trans = e.review.clone();
        e.prov = crate::provenance::REVIEW.to_string();
        e.set_translated_flags(true);
        let st = if Status::Validated.applicable_to(e) {
            Status::Validated
        } else {
            Status::Pending
        };
        status::set(e, st);
        out.push(i);
    }
    out
}

/// « Relecture = Traduction » (R-A17-04): the reviewer starts from the
/// translator's text. Returns the lines changed.
pub fn from_trans(entries: &mut [TmEntry], lines: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    for &i in lines {
        let Some(e) = entries.get_mut(i) else {
            continue;
        };
        if !workable(e) || e.trans.is_empty() || e.review == e.trans {
            continue;
        }
        e.review = e.trans.clone();
        out.push(i);
    }
    out
}

/// « Effacer la relecture » (R-A17-04). Returns the lines changed.
pub fn clear(entries: &mut [TmEntry], lines: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    for &i in lines {
        let Some(e) = entries.get_mut(i) else {
            continue;
        };
        if e.review.is_empty() {
            continue;
        }
        e.review.clear();
        out.push(i);
    }
    out
}

/// How the lines of the reviewed copy find their line in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Match {
    /// Record, field and source identical (and the FormID / EDID when both
    /// sides have one).
    #[default]
    Record,
    /// Same source text, whatever the field.
    Text,
}

/// The options of « Charger dans la relecture » (R-A17-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadOptions {
    pub matching: Match,
    /// Fill only the lines whose review would differ from the translation.
    pub only_diff: bool,
    /// Replace a review already there.
    pub overwrite: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        LoadOptions {
            matching: Match::Record,
            only_diff: true,
            overwrite: false,
        }
    }
}

/// What loading a reviewed copy would do: the counts of the preview and the
/// reviews to write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadPlan {
    /// Translated lines of the file.
    pub total: usize,
    /// Lines of the file that found a line in the document.
    pub matched: usize,
    /// Lines of the file without a line in the document (source changed or
    /// absent).
    pub unmatched: usize,
    /// Lines of the document whose incoming review differs from the
    /// translation.
    pub diff: usize,
    /// Lines of the document matched that already have another review.
    pub existing: usize,
    /// The reviews to write: (line, text).
    pub changes: Vec<(usize, String)>,
}

/// The reviewer's text of a line of the copy: its translation, or, for a
/// STRINGS file of the target language, its text.
fn reviewed_text(e: &TmEntry, strings: bool) -> &str {
    if strings { &e.source } else { &e.trans }
}

/// Compute what loading `copy` into the review of `entries` would change.
pub fn plan(entries: &[TmEntry], copy: &TranslationMemory, opts: LoadOptions) -> LoadPlan {
    let strings = copy.meta.source_format == "strings";
    // Index the document.
    let mut by_rec: HashMap<(&str, &str, &str), Vec<usize>> = HashMap::new();
    let mut by_text: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut by_id: HashMap<(u8, i32), Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if !workable(e) {
            continue;
        }
        if strings {
            if e.idstexte >= 0 {
                by_id.entry((e.list_index, e.idstexte)).or_default().push(i);
            }
            continue;
        }
        match opts.matching {
            Match::Record => by_rec
                .entry((e.rec_sig.as_str(), e.field_sig.as_str(), e.source.as_str()))
                .or_default()
                .push(i),
            Match::Text => by_text.entry(e.source.trim()).or_default().push(i),
        }
    }
    let mut p = LoadPlan::default();
    // One review per line of the document: the first line of the copy wins.
    let mut taken: HashMap<usize, String> = HashMap::new();
    for c in &copy.entries {
        let text = reviewed_text(c, strings);
        if text.is_empty() {
            continue;
        }
        p.total += 1;
        let hits: Vec<usize> = if strings {
            by_id
                .get(&(c.list_index, c.idstexte))
                .cloned()
                .unwrap_or_default()
        } else {
            match opts.matching {
                Match::Record => by_rec
                    .get(&(c.rec_sig.as_str(), c.field_sig.as_str(), c.source.as_str()))
                    .map(|v| {
                        v.iter()
                            .copied()
                            .filter(|&i| {
                                let e = &entries[i];
                                (e.formid == 0 || c.formid == 0 || e.formid == c.formid)
                                    && (e.edid.is_empty()
                                        || c.edid.is_empty()
                                        || e.edid.eq_ignore_ascii_case(&c.edid))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                Match::Text => by_text.get(c.source.trim()).cloned().unwrap_or_default(),
            }
        };
        if hits.is_empty() {
            p.unmatched += 1;
            continue;
        }
        p.matched += 1;
        for i in hits {
            taken.entry(i).or_insert_with(|| text.to_string());
        }
    }
    let mut lines: Vec<usize> = taken.keys().copied().collect();
    lines.sort_unstable();
    for i in lines {
        let text = taken.remove(&i).unwrap_or_default();
        let e = &entries[i];
        let different = text != e.trans;
        if different {
            p.diff += 1;
        }
        let has_other = !e.review.is_empty() && e.review != text;
        if has_other {
            p.existing += 1;
        }
        if e.review == text || (opts.only_diff && !different) || (has_other && !opts.overwrite) {
            continue;
        }
        p.changes.push((i, text));
    }
    p
}

/// Write the reviews of a plan. Returns the lines changed.
pub fn commit(entries: &mut [TmEntry], plan: &LoadPlan) -> Vec<usize> {
    let mut out = Vec::with_capacity(plan.changes.len());
    for (i, text) in &plan.changes {
        if let Some(e) = entries.get_mut(*i) {
            e.review = text.clone();
            out.push(*i);
        }
    }
    out
}

/// Read a reviewed copy: any file the application opens as a translation
/// (XML, pivot JSON, SST, EET, DSD, `.ximdb`, STRINGS of the target
/// language).
pub fn read_copy(path: &Path) -> Result<TranslationMemory, String> {
    if crate::basedb::is_ximdb_path(path) {
        return crate::basedb::ximdb_to_tm(path);
    }
    crate::tm::load(path).map_err(|e| e.to_string())
}

/// The lines of a document that carry a review (the warning of Enregistrer
/// sous… in a format without it, R-A17-07).
pub fn count(entries: &[TmEntry]) -> usize {
    entries.iter().filter(|e| !e.review.is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tm::TmMeta;

    fn e(rec: &str, fid: u32, src: &str, tr: &str) -> TmEntry {
        TmEntry {
            rec_sig: rec.into(),
            field_sig: "FULL".into(),
            formid: fid,
            edid: format!("E{fid}"),
            source: src.into(),
            trans: tr.into(),
            ..Default::default()
        }
    }

    fn tm(entries: Vec<TmEntry>) -> TranslationMemory {
        TranslationMemory::new(entries, TmMeta::default())
    }

    #[test]
    fn accept_copy_clear() {
        let mut es = vec![
            e("WEAP", 1, "Sword", "Épée"),
            e("ARMO", 2, "Hat", "Chapeau"),
        ];
        es[0].review = "Lame".into();
        es[1].review = "Chapeau".into();
        // Only the differing review is accepted (T-A17-02).
        assert_eq!(accept(&mut es, &[0, 1]), vec![0]);
        assert_eq!(es[0].trans, "Lame");
        assert_eq!(status::derive(&es[0]), Status::Validated);
        assert_eq!(es[0].prov, crate::provenance::REVIEW);
        assert_eq!(es[0].review, "Lame", "the review is kept");
        // A review equal to the source cannot be Validé: En attente.
        let mut s = vec![e("NPC_", 3, "Hod", "Hodd")];
        s[0].review = "Hod".into();
        accept(&mut s, &[0]);
        assert_eq!(status::derive(&s[0]), Status::Pending);
        assert_eq!(clear(&mut es, &[0, 1]), vec![0, 1]);
        assert!(es[0].review.is_empty());
        assert_eq!(from_trans(&mut es, &[0]), vec![0]);
        assert_eq!(es[0].review, "Lame");
        // An ignored line is left alone.
        let mut ig = vec![e("MISC", 4, "Gem", "Gemme")];
        ig[0].review = "Pierre".into();
        status::set(&mut ig[0], Status::Ignored);
        assert!(accept(&mut ig, &[0]).is_empty());
    }

    #[test]
    fn plan_counts_and_options() {
        let mut doc = vec![
            e("WEAP", 1, "Sword", "Épée"),
            e("ARMO", 2, "Hat", "Chapeau"),
            e("MISC", 3, "Gem", "Gemme"),
            e("MISC", 4, "Ring", "Anneau"),
        ];
        doc[3].review = "Bague".into();
        let copy = tm(vec![
            e("WEAP", 1, "Sword", "Lame"),      // differs
            e("ARMO", 2, "Hat", "Chapeau"),     // same as the translation
            e("MISC", 3, "Gem (new)", "Gemme"), // source changed: unmatched
            e("MISC", 4, "Ring", "Alliance"),   // differs, review present
            e("MISC", 5, "Unknown", ""),        // untranslated: not counted
        ]);
        let p = plan(&doc, &copy, LoadOptions::default());
        assert_eq!((p.total, p.matched, p.unmatched), (4, 3, 1));
        assert_eq!((p.diff, p.existing), (2, 1));
        assert_eq!(p.changes, vec![(0, "Lame".to_string())]);
        // Overwrite, and every matched line.
        let all = LoadOptions {
            only_diff: false,
            overwrite: true,
            ..Default::default()
        };
        let p = plan(&doc, &copy, all);
        assert_eq!(p.changes.len(), 3);
        let before: Vec<String> = doc.iter().map(|x| x.trans.clone()).collect();
        commit(&mut doc, &p);
        assert_eq!(doc[3].review, "Alliance");
        // The translations are untouched (T-A17-01).
        assert_eq!(
            doc.iter().map(|x| x.trans.clone()).collect::<Vec<_>>(),
            before
        );
        // Text-only matching finds the changed record by its text.
        let copy2 = tm(vec![e("BOOK", 9, "Gem", "Joyau")]);
        let p = plan(
            &doc,
            &copy2,
            LoadOptions {
                matching: Match::Text,
                ..Default::default()
            },
        );
        assert_eq!(p.changes, vec![(2, "Joyau".to_string())]);
        // A different FormID is not the same line.
        let copy3 = tm(vec![e("WEAP", 7, "Sword", "Lame")]);
        let mut other = copy3.entries[0].clone();
        other.edid = "E1".into();
        let p = plan(&doc, &tm(vec![other]), LoadOptions::default());
        assert_eq!(p.matched, 0);
    }

    #[test]
    fn strings_of_the_target_language() {
        let mut doc = vec![e("WEAP", 1, "Sword", "Épée")];
        doc[0].idstexte = 42;
        let copy = TranslationMemory::new(
            vec![TmEntry {
                source: "Lame".into(),
                idstexte: 42,
                ..Default::default()
            }],
            TmMeta {
                source_format: "strings".into(),
                ..Default::default()
            },
        );
        let p = plan(&doc, &copy, LoadOptions::default());
        assert_eq!(p.changes, vec![(0, "Lame".to_string())]);
    }
}
