//! Apply a translation memory (a dictionary) onto a set of source strings —
//! typically the strings just extracted from a plugin.
//!
//! This is the heart of "translate a mod": the plugin gives every translatable
//! string with full context (FormID, EDID, record, field, index) but no
//! translation; a dictionary (SST / EET / DSD / XML / pivot) gives
//! source → translation mappings, with varying amounts of context. Matching one
//! onto the other is what xTranslator and ESP-ESM Translator do internally.
//!
//! Matching is layered, most precise first, so a dictionary made from the exact
//! same plugin matches by identifier, while a dictionary from another version
//! (or another tool that lacks FormIDs) still matches by text:
//!
//! 1. `(FormID, record, field, index)` — exact, when the dictionary has FormIDs.
//! 2. `(EDID, record, field, index)` — exact, when it has editor IDs.
//! 3. `(record, field, source text)` — context-scoped text match (the standard
//!    translation-memory behaviour).
//! 4. `(source text)` — last-resort text match across fields.

use std::collections::HashMap;

use super::model::{TmEntry, TranslationMemory};

/// Which layer produced a match (for diagnostics and tuning).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchLevel {
    FormId,
    Edid,
    Context,
    Source,
}

/// Summary of a merge.
#[derive(Debug, Clone, Default)]
pub struct MergeReport {
    pub total: usize,
    pub matched: usize,
    pub by_formid: usize,
    pub by_edid: usize,
    pub by_context: usize,
    pub by_source: usize,
    /// Of the matched, how many produced an actual change (trans ≠ source).
    pub effective: usize,
}

impl MergeReport {
    pub fn unmatched(&self) -> usize {
        self.total - self.matched
    }
    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.matched as f64 / self.total as f64
        }
    }
}

/// Options controlling how aggressively to match.
#[derive(Debug, Clone, Copy)]
pub struct MergeOptions {
    /// Allow the last-resort `(source)`-only match. Off by default because a
    /// bare source string can be ambiguous across unrelated fields.
    pub allow_source_only: bool,
    /// Overwrite an existing non-empty translation on the target.
    pub overwrite: bool,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            allow_source_only: true,
            overwrite: false,
        }
    }
}

type Key3 = (u32, String, String, i32);
type KeyE = (String, String, String, i32);
type KeyC = (String, String, String);

struct Index {
    by_formid: HashMap<Key3, String>,
    by_edid: HashMap<KeyE, String>,
    by_context: HashMap<KeyC, String>,
    by_source: HashMap<String, String>,
    /// Context/source keys that mapped to more than one distinct translation.
    ambiguous_context: std::collections::HashSet<KeyC>,
    ambiguous_source: std::collections::HashSet<String>,
}

impl Index {
    fn build(dict: &TranslationMemory) -> Self {
        let mut by_formid = HashMap::new();
        let mut by_edid = HashMap::new();
        let mut by_context: HashMap<KeyC, String> = HashMap::new();
        let mut by_source: HashMap<String, String> = HashMap::new();
        let mut ambiguous_context = std::collections::HashSet::new();
        let mut ambiguous_source = std::collections::HashSet::new();

        for e in &dict.entries {
            if e.trans.is_empty() {
                continue;
            }
            if e.formid != 0 && !e.rec_sig.is_empty() {
                by_formid
                    .entry((e.formid, e.rec_sig.clone(), e.field_sig.clone(), e.index))
                    .or_insert_with(|| e.trans.clone());
            }
            if !e.edid.is_empty() {
                by_edid
                    .entry((
                        e.edid.clone(),
                        e.rec_sig.clone(),
                        e.field_sig.clone(),
                        e.index,
                    ))
                    .or_insert_with(|| e.trans.clone());
            }
            let ck = (e.rec_sig.clone(), e.field_sig.clone(), e.source.clone());
            match by_context.get(&ck) {
                Some(prev) if *prev != e.trans => {
                    ambiguous_context.insert(ck);
                }
                None => {
                    by_context.insert(ck, e.trans.clone());
                }
                _ => {}
            }
            match by_source.get(&e.source) {
                Some(prev) if *prev != e.trans => {
                    ambiguous_source.insert(e.source.clone());
                }
                None => {
                    by_source.insert(e.source.clone(), e.trans.clone());
                }
                _ => {}
            }
        }

        Index {
            by_formid,
            by_edid,
            by_context,
            by_source,
            ambiguous_context,
            ambiguous_source,
        }
    }

    /// Find a translation for a target entry, returning the winning level.
    fn lookup(&self, e: &TmEntry, opts: &MergeOptions) -> Option<(String, MatchLevel)> {
        if e.formid != 0
            && let Some(t) =
                self.by_formid
                    .get(&(e.formid, e.rec_sig.clone(), e.field_sig.clone(), e.index))
        {
            return Some((t.clone(), MatchLevel::FormId));
        }
        if !e.edid.is_empty()
            && let Some(t) = self.by_edid.get(&(
                e.edid.clone(),
                e.rec_sig.clone(),
                e.field_sig.clone(),
                e.index,
            ))
        {
            return Some((t.clone(), MatchLevel::Edid));
        }
        let ck = (e.rec_sig.clone(), e.field_sig.clone(), e.source.clone());
        if !self.ambiguous_context.contains(&ck)
            && let Some(t) = self.by_context.get(&ck)
        {
            return Some((t.clone(), MatchLevel::Context));
        }
        if opts.allow_source_only
            && !e.source.is_empty()
            && !self.ambiguous_source.contains(&e.source)
            && let Some(t) = self.by_source.get(&e.source)
        {
            return Some((t.clone(), MatchLevel::Source));
        }
        None
    }
}

/// Apply `dict` onto `target` in place, filling in translations. Returns a
/// report of what matched and how.
pub fn merge(
    target: &mut TranslationMemory,
    dict: &TranslationMemory,
    opts: MergeOptions,
) -> MergeReport {
    let index = Index::build(dict);
    let mut report = MergeReport {
        total: target.entries.len(),
        ..Default::default()
    };

    for e in &mut target.entries {
        if !opts.overwrite && !e.trans.is_empty() && e.trans != e.source {
            // Already translated; leave it.
            continue;
        }
        if let Some((trans, level)) = index.lookup(e, &opts) {
            e.trans = trans;
            e.set_translated_flags(true);
            report.matched += 1;
            match level {
                MatchLevel::FormId => report.by_formid += 1,
                MatchLevel::Edid => report.by_edid += 1,
                MatchLevel::Context => report.by_context += 1,
                MatchLevel::Source => report.by_source += 1,
            }
            if e.trans != e.source {
                report.effective += 1;
            }
        }
    }

    // Carry the dictionary's languages/addon onto the target when it lacks them.
    if target.meta.lang_source.is_empty() {
        target.meta.lang_source = dict.meta.lang_source.clone();
    }
    if target.meta.lang_dest.is_empty() {
        target.meta.lang_dest = dict.meta.lang_dest.clone();
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tm::{TmEntry, TmMeta, TranslationMemory};

    fn entry(rec: &str, field: &str, src: &str, formid: u32, edid: &str, index: i32) -> TmEntry {
        TmEntry {
            source: src.into(),
            rec_sig: rec.into(),
            field_sig: field.into(),
            formid,
            edid: edid.into(),
            index,
            ..Default::default()
        }
    }

    #[test]
    fn matches_by_formid_then_falls_back_to_text() {
        let mut target = TranslationMemory::new(
            vec![
                entry("WEAP", "FULL", "Sword", 0x0100_0001, "MySword", 1),
                entry("ARMO", "FULL", "Shield", 0x0100_0002, "MyShield", 1),
            ],
            TmMeta::default(),
        );
        let mut d1 = entry("WEAP", "FULL", "Sword", 0x0100_0001, "", 1);
        d1.trans = "Épée".into();
        let mut d2 = entry("ARMO", "FULL", "Shield", 0, "", 1); // no formid -> text match
        d2.trans = "Bouclier".into();
        let dict = TranslationMemory::new(vec![d1, d2], TmMeta::default());

        let report = merge(&mut target, &dict, MergeOptions::default());
        assert_eq!(report.matched, 2);
        assert_eq!(report.by_formid, 1);
        assert_eq!(report.by_context, 1);
        assert_eq!(target.entries[0].trans, "Épée");
        assert_eq!(target.entries[1].trans, "Bouclier");
    }

    #[test]
    fn ambiguous_source_is_not_applied_blindly() {
        let mut target = TranslationMemory::new(
            vec![entry("MISC", "FULL", "Key", 0, "", 1)],
            TmMeta::default(),
        );
        let mut a = entry("MISC", "FULL", "Key", 0, "", 1);
        a.trans = "Clé".into();
        let mut b = entry("BOOK", "FULL", "Key", 0, "", 1);
        b.trans = "Touche".into();
        let dict = TranslationMemory::new(vec![a, b], TmMeta::default());
        // Context (MISC,FULL,Key) is unambiguous -> "Clé".
        let report = merge(&mut target, &dict, MergeOptions::default());
        assert_eq!(report.matched, 1);
        assert_eq!(target.entries[0].trans, "Clé");
    }
}
