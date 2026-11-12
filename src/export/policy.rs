//! Which lines an export carries (S-04, R-S04-01/02).
//!
//! Every writer decides to include a line by `e.is_translated()`. Rather than
//! threading a parameter through all of them, the policy produces a *filtered
//! copy* of the memory where the translation of the non-exportable lines is
//! emptied: for STRINGS and full plugins the line then falls back on its source
//! text, for the override patch and the DSD it simply disappears — exactly
//! R-S04-02, without touching the writers or their tests. The clone of a large
//! memory costs a few milliseconds.

use std::borrow::Cow;

use crate::status::{self, COUNT, Status};
use crate::tm::{TmEntry, TranslationMemory};

/// The statuses an export carries: Validé always; À traduire and Ignoré never;
/// the others only when explicitly included (R-S04-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportPolicy {
    /// Statuses explicitly included, indexed by [`Status::index`].
    pub include: [bool; COUNT],
    /// Export for debugging in game (A-22, D-113): each text written is
    /// prefixed by its identifier; never remembered.
    pub ids: Option<IdForm>,
}

/// The identifier a debugging export puts before each text (R-A22-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdForm {
    /// « [0401A2F3] texte ».
    FormId,
    /// « [SPEL:DESC] texte ».
    Sig,
    /// « [0401A2F3 SPEL:DESC] texte ».
    Both,
}

impl IdForm {
    pub const ALL: [IdForm; 3] = [IdForm::FormId, IdForm::Sig, IdForm::Both];

    /// The name of the form on the command line, in the batch files and the
    /// reports.
    pub fn id(self) -> &'static str {
        match self {
            IdForm::FormId => "formid",
            IdForm::Sig => "sig",
            IdForm::Both => "both",
        }
    }

    pub fn from_id(s: &str) -> Option<IdForm> {
        Self::ALL
            .into_iter()
            .find(|f| f.id().eq_ignore_ascii_case(s.trim()))
    }

    /// The message key of the form (the export dialog).
    pub fn i18n_key(self) -> &'static str {
        match self {
            IdForm::FormId => "export-ids-formid",
            IdForm::Sig => "export-ids-sig",
            IdForm::Both => "export-ids-both",
        }
    }

    /// The prefix of a line, its space included: the FormID in eight
    /// upper-case hexadecimal digits (« — » without one), the record and
    /// field signatures.
    pub fn prefix(self, e: &TmEntry) -> String {
        let fid = if e.formid == 0 {
            "—".to_string()
        } else {
            format!("{:08X}", e.formid)
        };
        let sig = format!("{}:{}", e.rec_sig, e.field_sig);
        match self {
            IdForm::FormId => format!("[{fid}] "),
            IdForm::Sig => format!("[{sig}] "),
            IdForm::Both => format!("[{fid} {sig}] "),
        }
    }
}

/// Whether a text starts with an identifier as a debugging export writes
/// it (R-A22-03): « [0401A2F3] » or « [SPEL:DESC] ».
pub fn starts_with_id(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('[') else {
        return false;
    };
    let Some(end) = rest.find("] ") else {
        return false;
    };
    let inner = &rest[..end];
    let hex = inner.len() == 8
        && inner
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase());
    let sig = inner.len() == 9
        && inner.as_bytes()[4] == b':'
        && inner
            .chars()
            .enumerate()
            .all(|(i, c)| i == 4 || c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    hex || sig
}

/// Per-status tally of an export (R-S04-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportCounts {
    /// Lines exported with the Validé status.
    pub validated: usize,
    /// Lines exported thanks to an explicit inclusion, per status index.
    pub by_status: [usize; COUNT],
    /// Total of exported lines.
    pub total: usize,
    /// Translated lines left out.
    pub excluded: usize,
}

impl ExportPolicy {
    /// The default policy: validated lines only (R-S04-04).
    pub fn validated_only() -> Self {
        Self::default()
    }

    /// The statuses the user may include (everything but Validé, À traduire
    /// and Ignoré), in the dialog's order: the most common first.
    pub fn optional_statuses() -> impl Iterator<Item = Status> {
        [
            Status::BySimilarity,
            Status::Pending,
            Status::ModifiedProposal,
            Status::Custom,
        ]
        .into_iter()
    }

    pub fn is_optional(s: Status) -> bool {
        !matches!(s, Status::Validated | Status::ToTranslate | Status::Ignored)
    }

    /// Whether any optional status is included (the warning is shown then).
    pub fn includes_any(&self) -> bool {
        Self::optional_statuses().any(|s| self.include[s.index()])
    }

    pub fn with(mut self, s: Status, on: bool) -> Self {
        if Self::is_optional(s) {
            self.include[s.index()] = on;
        }
        self
    }

    /// Whether a line with this status is exported.
    pub fn allows(&self, s: Status) -> bool {
        s.is_exported() || (Self::is_optional(s) && self.include[s.index()])
    }

    /// Whether the entry is exported: translated *and* allowed by status.
    pub fn exportable(&self, e: &TmEntry) -> bool {
        e.is_translated() && self.allows(status::derive(e))
    }

    /// The memory as the writers must see it: borrowed when every translated
    /// line is exportable, else a copy where the excluded lines carry no
    /// translation (they fall back on the source, or vanish from a diff).
    pub fn filtered<'a>(&self, tm: &'a TranslationMemory) -> Cow<'a, TranslationMemory> {
        // A text document keeps its own line-ending setting (R-T02).
        let eol = tm.meta.source_format != "text";
        let needs_copy = self.ids.is_some()
            || tm.entries.iter().any(|e| {
                e.is_translated()
                    && (!self.exportable(e)
                        || (eol && source_line_endings(&e.source, &e.trans).is_some()))
            });
        if !needs_copy {
            return Cow::Borrowed(tm);
        }
        let mut copy = tm.clone();
        let game = crate::games::Game::from_id(&tm.meta.game);
        for e in copy.entries.iter_mut() {
            if e.is_translated() && !self.exportable(e) {
                e.trans.clear();
            }
            if eol && let Some(t) = source_line_endings(&e.source, &e.trans) {
                e.trans = t;
            }
            if let Some(form) = self.ids {
                Self::prefix_line(e, form, game);
            }
        }
        Cow::Owned(copy)
    }

    /// A debugging export (R-A22-01): the text written — the translation,
    /// or the source of a line without exported translation — prefixed by
    /// its identifier, cut to the field's limit after the prefix; never a
    /// line Ignoré or never shown, which the game does not display.
    fn prefix_line(e: &mut TmEntry, form: IdForm, game: crate::games::Game) {
        if e.unused || status::derive(e) == Status::Ignored {
            return;
        }
        let base = if e.trans.is_empty() {
            e.source.clone()
        } else {
            e.trans.clone()
        };
        if base.trim().is_empty() {
            return;
        }
        let prefix = form.prefix(e);
        let mut text = format!("{prefix}{base}");
        if let Some(max) = crate::length::Limits::of(game, &e.rec_sig, &e.field_sig).max_chars {
            let max = (max as usize).max(prefix.chars().count());
            if text.chars().count() > max {
                text = text.chars().take(max).collect();
            }
        }
        e.trans = text;
    }

    /// The text a debugging export prefixes on a line — its exported
    /// translation, or its source — or `None` for a line it leaves alone
    /// (Ignoré, never shown, empty).
    fn debug_base<'a>(&self, e: &'a TmEntry) -> Option<&'a str> {
        if e.unused || status::derive(e) == Status::Ignored {
            return None;
        }
        let base = if e.is_translated() && self.exportable(e) {
            &e.trans
        } else {
            &e.source
        };
        (!base.trim().is_empty()).then_some(base.as_str())
    }

    /// The lines a debugging export writes with their identifier: every
    /// line the game shows, translated or not.
    pub fn ids_lines(&self, tm: &TranslationMemory) -> usize {
        tm.entries
            .iter()
            .filter(|e| self.debug_base(e).is_some())
            .count()
    }

    /// The lines a debugging export cuts to their field's limit.
    pub fn ids_truncated(&self, tm: &TranslationMemory) -> usize {
        let Some(form) = self.ids else {
            return 0;
        };
        let game = crate::games::Game::from_id(&tm.meta.game);
        tm.entries
            .iter()
            .filter(|e| {
                self.debug_base(e).is_some_and(|base| {
                    crate::length::Limits::of(game, &e.rec_sig, &e.field_sig)
                        .max_chars
                        .is_some_and(|m| {
                            form.prefix(e).chars().count() + base.chars().count() > m as usize
                        })
                })
            })
            .count()
    }

    /// The exported lines whose translation already starts with an
    /// identifier (R-A22-03): the reverse check.
    pub fn ids_present(&self, tm: &TranslationMemory) -> Vec<usize> {
        tm.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.exportable(e) && starts_with_id(&e.trans))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn counts(&self, tm: &TranslationMemory) -> ExportCounts {
        let mut c = ExportCounts::default();
        for e in &tm.entries {
            if !e.is_translated() {
                continue;
            }
            let s = status::derive(e);
            if s.is_exported() {
                c.validated += 1;
                c.total += 1;
            } else if self.allows(s) {
                c.by_status[s.index()] += 1;
                c.total += 1;
            } else {
                c.excluded += 1;
            }
        }
        c
    }
}

/// The translation written with the line endings of its source (R-T02,
/// D-115), or `None` when it already has them or the source decides
/// nothing: a source with CRLF only turns each lone LF of the translation
/// into CRLF; a source with LF only turns each CRLF into LF; a source
/// without line break, or mixed, leaves the translation as it is. The
/// memory keeps the text as typed; only the export follows the source.
pub fn source_line_endings(source: &str, trans: &str) -> Option<String> {
    if !trans.contains('\n') {
        return None;
    }
    let lf = source.matches('\n').count();
    if lf == 0 {
        return None;
    }
    let crlf = source.matches("\r\n").count();
    let cr = source.matches('\r').count();
    let out = if crlf == lf && cr == crlf {
        trans.replace("\r\n", "\n").replace('\n', "\r\n")
    } else if crlf == 0 && cr == 0 {
        trans.replace("\r\n", "\n")
    } else {
        return None;
    };
    (out != trans).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(src: &str, trans: &str, s: Status) -> TmEntry {
        let mut e = TmEntry {
            source: src.into(),
            trans: trans.into(),
            ..Default::default()
        };
        status::set(&mut e, s);
        e
    }

    fn mem() -> TranslationMemory {
        TranslationMemory {
            meta: Default::default(),
            entries: vec![
                entry("a", "A", Status::Validated),
                entry("b", "B", Status::BySimilarity),
                entry("c", "C", Status::Pending),
                entry("d", "D", Status::ModifiedProposal),
                entry("e", "E", Status::Custom),
                entry("f", "F", Status::Ignored),
                entry("g", "", Status::ToTranslate),
            ],
            history: Vec::new(),
        }
    }

    #[test]
    fn line_endings_follow_the_source_at_export() {
        assert_eq!(
            source_line_endings("a\r\nb\r\nc", "x\ny\r\nz").as_deref(),
            Some("x\r\ny\r\nz")
        );
        assert_eq!(
            source_line_endings("a\nb", "x\r\ny").as_deref(),
            Some("x\ny")
        );
        // Already the same, no line break, or a mixed source: unchanged.
        assert_eq!(source_line_endings("a\r\nb", "x\r\ny"), None);
        assert_eq!(source_line_endings("ab", "x\ny"), None);
        assert_eq!(source_line_endings("a\r\nb\nc", "x\ny"), None);
        assert_eq!(source_line_endings("a\r\nb", "xy"), None);

        let mut tm = mem();
        tm.entries[0].source = "A\r\nB".into();
        tm.entries[0].trans = "a\nb".into();
        let before = tm.clone();
        let out = ExportPolicy::validated_only().filtered(&tm);
        assert_eq!(out.entries[0].trans, "a\r\nb");
        assert_eq!(
            tm.entries, before.entries,
            "the memory keeps the typed text"
        );
        // A text document keeps its own setting.
        tm.meta.source_format = "text".into();
        assert_eq!(
            ExportPolicy::validated_only().filtered(&tm).entries[0].trans,
            "a\nb"
        );
    }

    #[test]
    fn a_debugging_export_prefixes_every_shown_line_without_touching_the_memory() {
        let mut tm = mem();
        for (i, e) in tm.entries.iter_mut().enumerate() {
            e.formid = 0x0401_A2F0 + i as u32;
            e.rec_sig = "SPEL".into();
            e.field_sig = "FULL".into();
        }
        let before = tm.clone();
        let p = ExportPolicy {
            ids: Some(IdForm::FormId),
            ..ExportPolicy::validated_only()
        };
        let out = p.filtered(&tm);
        // Validé: its translation; excluded or untranslated: the source;
        // Ignoré: no text and no prefix, as before.
        assert_eq!(out.entries[0].trans, "[0401A2F0] A");
        assert_eq!(out.entries[1].trans, "[0401A2F1] b");
        assert_eq!(out.entries[5].trans, "");
        assert_eq!(out.entries[6].trans, "[0401A2F6] g");
        assert_eq!(tm.entries, before.entries, "the memory is never changed");
        // The count the dialog shows is the number of prefixed lines.
        let prefixed = out
            .entries
            .iter()
            .filter(|e| starts_with_id(&e.trans))
            .count();
        assert_eq!(p.ids_lines(&tm), prefixed);
        assert_eq!(prefixed, 6);
        let sig = ExportPolicy {
            ids: Some(IdForm::Both),
            ..ExportPolicy::validated_only()
        };
        assert_eq!(sig.filtered(&tm).entries[0].trans, "[0401A2F0 SPEL:FULL] A");
        // Without the option: the same memory as before A-22.
        assert!(matches!(
            ExportPolicy::validated_only().filtered(&before),
            Cow::Owned(_)
        ));
        let plain = ExportPolicy::validated_only()
            .filtered(&before)
            .into_owned();
        assert!(plain.entries.iter().all(|e| !starts_with_id(&e.trans)));
    }

    #[test]
    fn the_reverse_check_finds_identifiers_already_typed() {
        assert!(starts_with_id("[04012AB1] Épée"));
        assert!(starts_with_id("[SPEL:DESC] Épée"));
        assert!(!starts_with_id("[Apo] Épée"));
        assert!(!starts_with_id("[04012ab1] Épée"));
        assert!(!starts_with_id("Épée [04012AB1] "));
        let mut tm = mem();
        tm.entries[0].trans = "[04012AB1] A".into();
        tm.entries[1].trans = "[04012AB2] B".into();
        let p = ExportPolicy::validated_only();
        // Only the exported lines count: the second is not Validé.
        assert_eq!(p.ids_present(&tm), vec![0]);
        assert_eq!(IdForm::from_id("SIG"), Some(IdForm::Sig));
        assert_eq!(IdForm::from_id("x"), None);
    }

    #[test]
    fn allows_per_status() {
        let p = ExportPolicy::validated_only();
        assert!(p.allows(Status::Validated));
        for s in [
            Status::BySimilarity,
            Status::Pending,
            Status::ModifiedProposal,
            Status::Custom,
        ] {
            assert!(!p.allows(s));
            assert!(p.with(s, true).allows(s));
        }
        // Never, even when asked.
        assert!(
            !p.with(Status::ToTranslate, true)
                .allows(Status::ToTranslate)
        );
        assert!(!p.with(Status::Ignored, true).allows(Status::Ignored));
        assert!(!p.includes_any());
        assert!(p.with(Status::Pending, true).includes_any());
        assert_eq!(ExportPolicy::optional_statuses().count(), 4);
    }

    #[test]
    fn filtered_empties_excluded_lines_only() {
        let tm = mem();
        let p = ExportPolicy::validated_only();
        let f = p.filtered(&tm);
        assert!(matches!(f, Cow::Owned(_)));
        let trans: Vec<&str> = f.entries.iter().map(|e| e.trans.as_str()).collect();
        assert_eq!(trans, vec!["A", "", "", "", "", "", ""]);
        // Sources and statuses of the kept lines untouched.
        assert_eq!(f.entries[0].source, "a");
        assert_eq!(status::derive(&f.entries[0]), Status::Validated);
        // Including everything optional keeps the Ignoré line out.
        let all = ExportPolicy::optional_statuses().fold(p, |p, s| p.with(s, true));
        let f = all.filtered(&tm);
        let trans: Vec<&str> = f.entries.iter().map(|e| e.trans.as_str()).collect();
        assert_eq!(trans, vec!["A", "B", "C", "D", "E", "", ""]);
        // A memory with only validated lines is borrowed.
        let tm2 = TranslationMemory {
            meta: Default::default(),
            entries: vec![entry("a", "A", Status::Validated)],
            history: Vec::new(),
        };
        assert!(matches!(p.filtered(&tm2), Cow::Borrowed(_)));
    }

    #[test]
    fn counts_match_filter() {
        let tm = mem();
        let c = ExportPolicy::validated_only().counts(&tm);
        assert_eq!((c.validated, c.total, c.excluded), (1, 1, 5));
        let c = ExportPolicy::validated_only()
            .with(Status::BySimilarity, true)
            .with(Status::Custom, true)
            .counts(&tm);
        assert_eq!((c.validated, c.total, c.excluded), (1, 3, 3));
        assert_eq!(c.by_status[Status::BySimilarity.index()], 1);
        assert_eq!(c.by_status[Status::Custom.index()], 1);
        assert_eq!(c.by_status[Status::Pending.index()], 0);
    }
}
