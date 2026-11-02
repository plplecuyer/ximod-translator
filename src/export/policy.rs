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
        let needs_copy = tm
            .entries
            .iter()
            .any(|e| e.is_translated() && !self.exportable(e));
        if !needs_copy {
            return Cow::Borrowed(tm);
        }
        let mut copy = tm.clone();
        for e in copy.entries.iter_mut() {
            if e.is_translated() && !self.exportable(e) {
                e.trans.clear();
            }
        }
        Cow::Owned(copy)
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
