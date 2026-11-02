//! Appliquer les dictionnaires (B-06): the stack — or one file — applied
//! to a scope of lines with a rule for what may be overwritten, a matching
//! mode and an optional reset first. `plan` computes what would change
//! (the preview); `commit` writes a plan into the entries. Nothing here
//! touches the undo stack: the application records the plan as one
//! operation.

use std::sync::Arc;

use crate::basedb::{BaseDb, FillLevel};
use crate::dicts::Role;
use crate::status::{self, Status};
use crate::tm::TmEntry;

/// What may be overwritten (R-B06-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overwrite {
    /// Lines without a translation only (the behaviour at opening).
    #[default]
    None,
    /// Everything but Validé and Personnalisé.
    KeepValidated,
    /// Everything but En attente.
    KeepPending,
    /// En attente only.
    PendingOnly,
    All,
}

impl Overwrite {
    pub const ALL: [Overwrite; 5] = [
        Overwrite::None,
        Overwrite::KeepValidated,
        Overwrite::KeepPending,
        Overwrite::PendingOnly,
        Overwrite::All,
    ];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Overwrite::None => "apply-over-none",
            Overwrite::KeepValidated => "apply-over-keep-validated",
            Overwrite::KeepPending => "apply-over-keep-pending",
            Overwrite::PendingOnly => "apply-over-pending-only",
            Overwrite::All => "apply-over-all",
        }
    }

    /// Whether a translated line with this status may be replaced.
    pub fn allows(self, st: Status) -> bool {
        match self {
            Overwrite::None => false,
            Overwrite::KeepValidated => !matches!(st, Status::Validated | Status::Custom),
            Overwrite::KeepPending => st != Status::Pending,
            Overwrite::PendingOnly => st == Status::Pending,
            Overwrite::All => true,
        }
    }
}

/// How a line is matched (R-B06-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Exact, then text, per dictionary (the stack's own rule).
    #[default]
    Strict,
    /// Same record and field, same text.
    Relax,
    /// Same text, any field.
    TextOnly,
    /// Same text to the numbers, punctuation and case (B-19) → En attente.
    Normalized,
    /// Same record, field, FormID and EDID, text changed → En attente.
    FormId,
    /// Word heuristic within the threshold (B-01) → Traduit par similarité.
    Fuzzy,
}

impl Mode {
    pub const ALL: [Mode; 6] = [
        Mode::Strict,
        Mode::Relax,
        Mode::TextOnly,
        Mode::Normalized,
        Mode::FormId,
        Mode::Fuzzy,
    ];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Mode::Strict => "apply-mode-strict",
            Mode::Relax => "apply-mode-relax",
            Mode::TextOnly => "apply-mode-textonly",
            Mode::Normalized => "apply-mode-normalized",
            Mode::FormId => "apply-mode-formid",
            Mode::Fuzzy => "apply-mode-fuzzy",
        }
    }
}

/// One dictionary of the source, in priority order.
#[derive(Clone)]
pub struct Level {
    pub name: String,
    pub role: Role,
    pub db: Arc<BaseDb>,
    /// The word index, for the Approchant mode (B-01).
    pub fuzzy: Option<Arc<crate::fuzzy::FuzzyIndex>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub overwrite: Overwrite,
    pub mode: Mode,
    pub reset: bool,
    /// Threshold of the Approchant mode, in percent of words.
    pub threshold: u8,
    /// Whether the Strict mode includes the normalized tier, as the stack
    /// at opening does (`[Fuzzy] Normalize`, B-19).
    pub normalize: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            overwrite: Overwrite::None,
            mode: Mode::Strict,
            reset: false,
            threshold: 20,
            normalize: true,
        }
    }
}

/// One line the application would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub idx: usize,
    pub trans: String,
    pub status: Status,
    /// `None` for a line the reset empties and nothing refills.
    pub origin: Option<(String, Role, FillLevel)>,
    /// The source found by the normalized tier (B-19).
    pub found: Option<String>,
    /// The number of candidates of an ambiguous source (B-08), 0 otherwise:
    /// the line is then left En attente and marked.
    pub ambiguous: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub changes: Vec<Change>,
    /// Lines filled per level, in stack order.
    pub per_level: Vec<(String, usize)>,
    pub exact: usize,
    pub text: usize,
    pub text_only: usize,
    pub formid: usize,
    pub old: usize,
    /// Lines matched on the normalized form (B-19).
    pub normalized: usize,
    pub fuzzy: usize,
    /// Approchant: scores ≥ 90, 80–89, below 80.
    pub fuzzy_hi: usize,
    pub fuzzy_mid: usize,
    pub fuzzy_lo: usize,
    /// Lines that had a translation and get another one.
    pub overwritten: usize,
    /// Lines the reset empties and nothing refills.
    pub emptied: usize,
    /// Lines left En attente because their source is ambiguous (B-08).
    pub ambiguous: usize,
}

impl Plan {
    /// Lines that receive a translation.
    pub fn filled(&self) -> usize {
        self.exact
            + self.text
            + self.text_only
            + self.formid
            + self.old
            + self.normalized
            + self.fuzzy
    }
}

/// A match: the level, the translation, its status, the fill level and,
/// for the normalized tier, the source found.
type Found = (usize, String, Status, FillLevel, Option<String>);

/// The normalized tier of one level (B-19).
fn normalized(li: usize, db: &BaseDb, e: &TmEntry, form: Option<&str>) -> Option<Found> {
    let hit = db.lookup_normalized(e, form?)?;
    Some((
        li,
        hit.trans,
        Status::Pending,
        FillLevel::Normalized(hit.reports),
        Some(hit.source),
    ))
}

/// The match of one line against the levels.
fn lookup(e: &TmEntry, levels: &[Level], opts: &Options, form: Option<&str>) -> Option<Found> {
    let (mode, threshold) = (opts.mode, opts.threshold);
    if mode == Mode::Fuzzy {
        // The best score across the levels; the stack order breaks ties.
        let mut best: Option<(usize, String, u8)> = None;
        for (li, l) in levels.iter().enumerate() {
            let Some(ix) = &l.fuzzy else { continue };
            if let Some(sg) = ix.best(&e.source, threshold)
                && best.as_ref().is_none_or(|b| sg.score > b.2)
            {
                best = Some((li, sg.trans, sg.score));
            }
        }
        return best.map(|(li, t, sc)| (li, t, Status::BySimilarity, FillLevel::Fuzzy(sc), None));
    }
    for (li, l) in levels.iter().enumerate() {
        let personal = l.role == Role::Personal;
        match mode {
            Mode::Strict => {
                if let Some((t, st)) = l.db.lookup_exact(e) {
                    let st = st
                        .filter(|s| *s != Status::ToTranslate)
                        .unwrap_or(if personal {
                            Status::Custom
                        } else {
                            Status::Validated
                        });
                    return Some((li, t.to_string(), st, FillLevel::Exact, None));
                }
                if let Some(t) = l.db.lookup_text(e) {
                    return Some((
                        li,
                        t.to_string(),
                        Status::BySimilarity,
                        FillLevel::Text,
                        None,
                    ));
                }
                if let Some(t) = l.db.lookup_text_only(e) {
                    let st = if personal {
                        Status::Custom
                    } else {
                        Status::BySimilarity
                    };
                    return Some((li, t.to_string(), st, FillLevel::TextOnly, None));
                }
                if opts.normalize
                    && let Some(f) = normalized(li, &l.db, e, form)
                {
                    return Some(f);
                }
                // Old data of the mod's dictionary (B-07).
                if let Some(old) = l.db.old() {
                    if let Some((t, st)) = old.lookup_exact(e) {
                        let st = st
                            .filter(|s| *s != Status::ToTranslate)
                            .unwrap_or(Status::Validated);
                        return Some((li, t.to_string(), st, FillLevel::Old, None));
                    }
                    if let Some(t) = old.lookup_text(e) {
                        return Some((
                            li,
                            t.to_string(),
                            Status::BySimilarity,
                            FillLevel::Old,
                            None,
                        ));
                    }
                    if opts.normalize
                        && let Some(f) = normalized(li, old, e, form)
                    {
                        return Some(f);
                    }
                }
            }
            Mode::Normalized => {
                if let Some(f) = normalized(li, &l.db, e, form) {
                    return Some(f);
                }
                if let Some(old) = l.db.old()
                    && let Some(f) = normalized(li, old, e, form)
                {
                    return Some(f);
                }
            }
            Mode::Relax => {
                if let Some(t) = l.db.lookup_text(e) {
                    return Some((
                        li,
                        t.to_string(),
                        Status::BySimilarity,
                        FillLevel::Text,
                        None,
                    ));
                }
            }
            Mode::TextOnly => {
                if let Some(t) = l.db.lookup_any_text(e) {
                    let st = if personal {
                        Status::Custom
                    } else {
                        Status::BySimilarity
                    };
                    return Some((li, t.to_string(), st, FillLevel::TextOnly, None));
                }
            }
            Mode::Fuzzy => unreachable!("handled above"),
            Mode::FormId => {
                if let Some((_, t)) = l.db.lookup_by_id(e) {
                    return Some((li, t.to_string(), Status::Pending, FillLevel::FormId, None));
                }
            }
        }
    }
    None
}

/// The number of candidates of `source` across the levels when it is
/// ambiguous (B-08), 0 otherwise.
fn ambiguity_of(source: &str, levels: &[Level]) -> usize {
    if crate::ambiguity::personal_decides(source, levels.iter().map(|l| (l.role, &*l.db))) {
        return 0;
    }
    let n = crate::ambiguity::candidates(
        source,
        levels.iter().map(|l| (l.name.as_str(), l.role, &*l.db)),
    )
    .len();
    if n >= 2 { n } else { 0 }
}

/// Compute what the application would change on `lines`. `progress` is
/// called every 2048 lines with (done, total); returning false cancels
/// (an empty plan comes back).
pub fn plan(
    entries: &[TmEntry],
    lines: &[usize],
    levels: &[Level],
    opts: &Options,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Plan {
    let mut p = Plan {
        per_level: levels.iter().map(|l| (l.name.clone(), 0)).collect(),
        ..Default::default()
    };
    // One ambiguity lookup per distinct source (B-08).
    let mut amb_cache: std::collections::HashMap<String, usize> = Default::default();
    for (done, &i) in lines.iter().enumerate() {
        if done % 2048 == 0 && !progress(done, lines.len()) {
            return Plan::default();
        }
        let Some(e) = entries.get(i) else { continue };
        if e.unused {
            continue;
        }
        let st = status::derive(e);
        if st == Status::Ignored {
            continue;
        }
        let has = !e.trans.is_empty();
        // May this line receive a translation?
        let replaceable = !has || opts.overwrite.allows(st);
        // Does the reset empty it first?
        let resettable = has
            && opts.reset
            && (opts.overwrite == Overwrite::All
                || !matches!(st, Status::Validated | Status::Custom));
        if !replaceable && !resettable {
            continue;
        }
        // A reset line is empty when the dictionaries look at it.
        let want_form =
            matches!(opts.mode, Mode::Normalized) || (opts.mode == Mode::Strict && opts.normalize);
        let form = if want_form {
            crate::normalize::form(&e.source)
        } else {
            None
        };
        let found = lookup(e, levels, opts, form.as_deref());
        match found {
            Some((li, trans, new_st, level, found_src)) => {
                // Validé and Personnalisé need a real translation (A-03).
                let new_st = if matches!(new_st, Status::Validated | Status::Custom)
                    && (trans.is_empty() || trans == e.source)
                {
                    Status::BySimilarity
                } else {
                    new_st
                };
                if has && e.trans == trans && st == new_st {
                    continue; // nothing to change
                }
                if has {
                    p.overwritten += 1;
                }
                match level {
                    FillLevel::Exact => p.exact += 1,
                    FillLevel::Text => p.text += 1,
                    FillLevel::TextOnly => p.text_only += 1,
                    FillLevel::FormId | FillLevel::TargetFile | FillLevel::Online => p.formid += 1,
                    FillLevel::Old => p.old += 1,
                    FillLevel::Normalized(_) => p.normalized += 1,
                    FillLevel::Fuzzy(sc) => {
                        p.fuzzy += 1;
                        if sc >= 90 {
                            p.fuzzy_hi += 1;
                        } else if sc >= 80 {
                            p.fuzzy_mid += 1;
                        } else {
                            p.fuzzy_lo += 1;
                        }
                    }
                }
                p.per_level[li].1 += 1;
                // An ambiguous source at a text level (R-B08-01/02/03); the
                // normalized tier counts its own candidates.
                let ambiguous = if let FillLevel::Normalized(r) = level {
                    if r.amb >= 2 { r.amb as usize } else { 0 }
                } else if crate::ambiguity::level_can_be(level) && levels[li].role != Role::Personal
                {
                    *amb_cache
                        .entry(e.source.clone())
                        .or_insert_with(|| ambiguity_of(&e.source, levels))
                } else {
                    0
                };
                let new_st = if ambiguous > 0 {
                    p.ambiguous += 1;
                    Status::Pending
                } else {
                    new_st
                };
                p.changes.push(Change {
                    idx: i,
                    trans,
                    status: new_st,
                    origin: Some((levels[li].name.clone(), levels[li].role, level)),
                    found: found_src,
                    ambiguous,
                });
            }
            None => {
                if resettable {
                    p.emptied += 1;
                    p.changes.push(Change {
                        idx: i,
                        trans: String::new(),
                        status: Status::ToTranslate,
                        origin: None,
                        found: None,
                        ambiguous: 0,
                    });
                }
            }
        }
    }
    progress(lines.len(), lines.len());
    p
}

/// Write a plan into the entries. Returns the lines changed.
pub fn commit(entries: &mut [TmEntry], plan: &Plan) -> Vec<usize> {
    let mut touched = Vec::with_capacity(plan.changes.len());
    for c in &plan.changes {
        let Some(e) = entries.get_mut(c.idx) else {
            continue;
        };
        e.trans = c.trans.clone();
        // The provenance of the line (B-09): the level that filled it.
        e.prov = match &c.origin {
            Some((file, role, FillLevel::Normalized(r))) if !c.trans.is_empty() => {
                crate::provenance::normalized(*role, file, *r, c.found.as_deref().unwrap_or(""))
            }
            Some((file, role, level)) if !c.trans.is_empty() => {
                let code = crate::provenance::dict(*role, *level, file);
                if c.ambiguous > 0 {
                    crate::ambiguity::mark(&code, c.ambiguous)
                } else {
                    code
                }
            }
            _ => String::new(),
        };
        if c.trans.is_empty() {
            e.set_translated_flags(false);
            status::set(e, Status::ToTranslate);
        } else {
            e.set_translated_flags(!e.trans.is_empty());
            status::set(e, c.status);
        }
        touched.push(c.idx);
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::basedb::{BaseRow, write_ximdb};

    fn entry(
        rec: &str,
        field: &str,
        formid: u32,
        edid: &str,
        src: &str,
        tr: &str,
        st: Option<Status>,
    ) -> TmEntry {
        let mut e = TmEntry {
            rec_sig: rec.into(),
            field_sig: field.into(),
            formid,
            edid: edid.into(),
            source: src.into(),
            trans: tr.into(),
            ..Default::default()
        };
        if let Some(st) = st {
            status::set(&mut e, st);
        } else if !tr.is_empty() {
            e.set_translated_flags(true);
        }
        e
    }

    fn row(rec: &str, field: &str, formid: u32, edid: &str, src: &str, tr: &str) -> BaseRow {
        BaseRow {
            rec: rec.into(),
            field: field.into(),
            formid,
            edid: edid.into(),
            source: src.into(),
            trans: tr.into(),
            status: None,
            ..Default::default()
        }
    }

    fn levels(tag: &str) -> (std::path::PathBuf, Vec<Level>) {
        let dir = std::env::temp_dir().join(format!("ximod-apply-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let game = dir.join("Skyrim_ENG-FRA.ximdb");
        write_ximdb(
            vec![
                row("WEAP", "FULL", 1, "Sword", "Iron Sword", "Épée en fer"),
                row("WEAP", "FULL", 2, "Axe", "Iron Axe", "Hache en fer"),
                row("ARMO", "FULL", 3, "Hat", "Hat", "Chapeau"),
                row(
                    "INFO",
                    "NAM1",
                    9,
                    "Intro",
                    "You're finally awake.",
                    "Tu es enfin réveillé.",
                ),
            ],
            "fra",
            &game,
        )
        .unwrap();
        let perso = dir.join("personal_ENG-FRA.ximdb");
        write_ximdb(
            vec![BaseRow {
                rec: String::new(),
                field: String::new(),
                formid: 0,
                edid: String::new(),
                source: "Hat".into(),
                trans: "Couvre-chef".into(),
                status: None,
                ..Default::default()
            }],
            "fra",
            &perso,
        )
        .unwrap();
        let lv = vec![
            Level {
                name: "personal_ENG-FRA.ximdb".into(),
                role: Role::Personal,
                db: Arc::new(BaseDb::from_file(&perso).unwrap()),
                fuzzy: None,
            },
            Level {
                name: "Skyrim_ENG-FRA.ximdb".into(),
                role: Role::Game,
                db: Arc::new(BaseDb::from_file(&game).unwrap()),
                fuzzy: None,
            },
        ];
        (dir, lv)
    }

    fn doc() -> Vec<TmEntry> {
        vec![
            entry("WEAP", "FULL", 1, "Sword", "Iron Sword", "", None), // 0 exact, empty
            entry(
                "WEAP",
                "FULL",
                2,
                "Axe",
                "Iron Axe",
                "Hache",
                Some(Status::Validated),
            ), // 1 validated
            entry("WEAP", "FULL", 7, "Axe2", "Iron Axe", "", None),    // 2 text (other record)
            entry("ARMO", "FULL", 3, "Hat", "Hat", "", None),          // 3 personal first
            entry(
                "INFO",
                "NAM1",
                9,
                "Intro",
                "You're finally awake, prisoner.",
                "",
                None,
            ), // 4 formid (text changed)
            entry(
                "WEAP",
                "FULL",
                8,
                "Old",
                "Iron Sword",
                "ancienne",
                Some(Status::BySimilarity),
            ), // 5 by similarity
            entry(
                "WEAP",
                "FULL",
                1,
                "Sword",
                "Iron Sword",
                "x",
                Some(Status::Ignored),
            ), // 6 ignored
            entry("MISC", "DESC", 11, "M", "Hat", "", None),           // 7 text only (other field)
        ]
    }

    /// T-B06-01/02: scope, overwrite rules, strict matching and the counts.
    #[test]
    fn strict_scope_and_overwrite() {
        let (dir, lv) = levels("a");
        let d = doc();
        let all: Vec<usize> = (0..d.len()).collect();
        // Rien: empty lines only.
        let p = plan(&d, &all, &lv, &Options::default(), |_, _| true);
        let idx: Vec<usize> = p.changes.iter().map(|c| c.idx).collect();
        assert_eq!(idx, vec![0, 2, 3, 7]);
        assert_eq!((p.exact, p.text, p.text_only, p.overwritten), (1, 1, 2, 0));
        assert_eq!(
            p.changes[2].status,
            Status::Custom,
            "the personal dictionary gives Personnalisé"
        );
        assert_eq!(p.changes[1].status, Status::BySimilarity);
        assert_eq!(
            p.per_level,
            vec![
                ("personal_ENG-FRA.ximdb".to_string(), 2),
                ("Skyrim_ENG-FRA.ximdb".to_string(), 2)
            ]
        );
        // Scope: selection of one line.
        let p = plan(&d, &[2], &lv, &Options::default(), |_, _| true);
        assert_eq!(p.changes.len(), 1);
        // Tout sauf Validé: line 5 (by similarity) is overwritten, line 1 not.
        let p = plan(
            &d,
            &all,
            &lv,
            &Options {
                overwrite: Overwrite::KeepValidated,
                ..Default::default()
            },
            |_, _| true,
        );
        let idx: Vec<usize> = p.changes.iter().map(|c| c.idx).collect();
        assert_eq!(idx, vec![0, 2, 3, 5, 7]);
        assert_eq!(p.overwritten, 1);
        // Tout: line 1 too; the ignored line never.
        let p = plan(
            &d,
            &all,
            &lv,
            &Options {
                overwrite: Overwrite::All,
                ..Default::default()
            },
            |_, _| true,
        );
        let idx: Vec<usize> = p.changes.iter().map(|c| c.idx).collect();
        assert_eq!(idx, vec![0, 1, 2, 3, 5, 7]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-B01-04: the Approchant mode fills with the best score across the
    /// levels, the score in the provenance and in the buckets.
    #[test]
    fn fuzzy_mode() {
        let (dir, mut lv) = levels("c");
        for l in &mut lv {
            l.fuzzy = Some(Arc::new(
                crate::fuzzy::FuzzyIndex::build(l.db.rows_for_fuzzy(), l.db.rows(), |_, _| true)
                    .unwrap(),
            ));
        }
        let d = vec![
            TmEntry {
                rec_sig: "INFO".into(),
                field_sig: "NAM1".into(),
                formid: 9,
                edid: "Intro".into(),
                source: "You're finally awake, prisoner.".into(),
                ..Default::default()
            },
            TmEntry {
                rec_sig: "WEAP".into(),
                field_sig: "FULL".into(),
                formid: 5,
                edid: "X".into(),
                source: "Nothing like it at all".into(),
                ..Default::default()
            },
        ];
        let p = plan(
            &d,
            &[0, 1],
            &lv,
            &Options {
                mode: Mode::Fuzzy,
                threshold: 25,
                ..Default::default()
            },
            |_, _| true,
        );
        assert_eq!(p.changes.len(), 1);
        assert_eq!(p.changes[0].trans, "Tu es enfin réveillé.");
        assert_eq!(p.changes[0].status, Status::BySimilarity);
        assert_eq!(
            p.changes[0].origin.as_ref().map(|o| o.2),
            Some(FillLevel::Fuzzy(75))
        );
        assert_eq!((p.fuzzy, p.fuzzy_hi, p.fuzzy_mid, p.fuzzy_lo), (1, 0, 0, 1));
        let p = plan(
            &d,
            &[0, 1],
            &lv,
            &Options {
                mode: Mode::Fuzzy,
                threshold: 20,
                ..Default::default()
            },
            |_, _| true,
        );
        assert!(p.changes.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-B06-03/04: FormID seul gives the old translation as En attente;
    /// Relax and Texte seul; the reset empties what nothing refills.
    #[test]
    fn modes_and_reset() {
        let (dir, lv) = levels("b");
        let d = doc();
        let all: Vec<usize> = (0..d.len()).collect();
        let p = plan(
            &d,
            &all,
            &lv,
            &Options {
                mode: Mode::FormId,
                ..Default::default()
            },
            |_, _| true,
        );
        assert_eq!(p.changes.len(), 1);
        assert_eq!(
            (
                p.changes[0].idx,
                p.changes[0].status,
                p.changes[0].trans.as_str()
            ),
            (4, Status::Pending, "Tu es enfin réveillé.")
        );
        assert_eq!(p.formid, 1);
        let p = plan(
            &d,
            &all,
            &lv,
            &Options {
                mode: Mode::Relax,
                ..Default::default()
            },
            |_, _| true,
        );
        let idx: Vec<usize> = p.changes.iter().map(|c| c.idx).collect();
        assert_eq!(
            idx,
            vec![0, 2, 3],
            "relax: same field and text, the exact line included"
        );
        assert!(p.changes.iter().all(|c| c.status == Status::BySimilarity));
        let p = plan(
            &d,
            &all,
            &lv,
            &Options {
                mode: Mode::TextOnly,
                ..Default::default()
            },
            |_, _| true,
        );
        let idx: Vec<usize> = p.changes.iter().map(|c| c.idx).collect();
        assert_eq!(idx, vec![0, 2, 3, 7]);
        assert_eq!(
            p.changes[3].trans, "Couvre-chef",
            "the personal pair wins by priority"
        );
        // Reset + Rien: the similarity line is emptied (nothing refills it: exact needs EDID "Old").
        let p = plan(
            &d,
            &all,
            &lv,
            &Options {
                reset: true,
                ..Default::default()
            },
            |_, _| true,
        );
        let c5 = p.changes.iter().find(|c| c.idx == 5).unwrap();
        assert_eq!(c5.trans, "Épée en fer", "refilled by the text tier");
        let mut d2 = d.clone();
        d2[5].source = "Unknown text".into();
        let p = plan(
            &d2,
            &all,
            &lv,
            &Options {
                reset: true,
                ..Default::default()
            },
            |_, _| true,
        );
        let c5 = p.changes.iter().find(|c| c.idx == 5).unwrap();
        assert!(c5.trans.is_empty() && p.emptied == 1);
        // Validé is never emptied unless Tout.
        assert!(!p.changes.iter().any(|c| c.idx == 1));
        // Commit writes and reports the lines.
        let mut d3 = d2.clone();
        let touched = commit(&mut d3, &p);
        assert!(
            touched.contains(&5)
                && d3[5].trans.is_empty()
                && status::derive(&d3[5]) == Status::ToTranslate
        );
        assert_eq!(status::derive(&d3[3]), Status::Custom);
        // Cancel: an empty plan.
        let p = plan(&d, &all, &lv, &Options::default(), |_, _| false);
        assert!(p.changes.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
