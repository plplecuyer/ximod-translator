//! The seven translation statuses (CDC §7) and their default pastel colors.
//!
//! The reference tools (xTranslator / ESP-ESM Translator) tag each line with a
//! colored status. XIMOD adopts the CDC's seven-status scheme. A status is
//! *derived* from an entry's stored flags (so imported memories show sensible
//! colors) and can be *set* back onto those flags by the UI.

use crate::tm::TmEntry;

/// Number of distinct statuses.
pub const COUNT: usize = 7;

/// A line's translation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// À traduire — not yet translated.
    ToTranslate,
    /// Validé — reviewed and confirmed (the only status exported by default).
    Validated,
    /// En attente — translated but awaiting review.
    Pending,
    /// Traduit par similarité — filled from a translation memory / fuzzy match.
    BySimilarity,
    /// Proposition modifiée — an edited suggestion.
    ModifiedProposal,
    /// Personnalisé — a custom/locked value.
    Custom,
    /// Ignoré — deliberately left out of the translation/export.
    Ignored,
}

impl Status {
    /// All statuses in display order.
    pub const ALL: [Status; COUNT] = [
        Status::ToTranslate,
        Status::Validated,
        Status::Pending,
        Status::BySimilarity,
        Status::ModifiedProposal,
        Status::Custom,
        Status::Ignored,
    ];

    /// Position in [`Status::ALL`] (also the config color index).
    pub fn index(self) -> usize {
        Status::ALL.iter().position(|s| *s == self).unwrap()
    }

    /// Stable identifier used in the config file.
    pub fn key(self) -> &'static str {
        match self {
            Status::ToTranslate => "to_translate",
            Status::Validated => "validated",
            Status::Pending => "pending",
            Status::BySimilarity => "by_similarity",
            Status::ModifiedProposal => "modified",
            Status::Custom => "custom",
            Status::Ignored => "ignored",
        }
    }

    /// Parse from a stable key.
    pub fn from_key(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|st| st.key() == s)
    }

    /// Whether a line may receive this status by a user action (A-03,
    /// R-A03-02): Validé and Personnalisé need a translation; a line never
    /// shown in game takes nothing.
    pub fn applicable_to(self, e: &TmEntry) -> bool {
        // A line never shown in game, or held by its file's VMAD switch
        // (C-08, never lifted line by line), takes nothing.
        if e.unused || crate::vmad_lock::is_file_locked(e) {
            return false;
        }
        !(matches!(self, Status::Validated | Status::Custom) && !e.is_translated())
    }

    /// The i18n key for the status legend.
    pub fn legend_key(self) -> &'static str {
        match self {
            Status::ToTranslate => "status-to-translate",
            Status::Validated => "status-validated",
            Status::Pending => "status-pending",
            Status::BySimilarity => "status-by-similarity",
            Status::ModifiedProposal => "status-modified",
            Status::Custom => "status-custom",
            Status::Ignored => "status-ignored",
        }
    }

    /// Default **pastel** color, `(r, g, b)`.
    pub fn default_rgb(self) -> (u8, u8, u8) {
        match self {
            Status::ToTranslate => (240, 240, 242),  // soft near-white
            Status::Validated => (183, 223, 180),    // pastel green
            Status::Pending => (247, 214, 168),      // pastel orange
            Status::BySimilarity => (176, 224, 222), // pastel turquoise
            Status::ModifiedProposal => (211, 211, 216), // pastel gray
            Status::Custom => (230, 220, 190),       // pastel beige
            Status::Ignored => (240, 190, 190),      // pastel red
        }
    }

    /// A short glyph shown in the status cell.
    pub fn glyph(self) -> &'static str {
        match self {
            Status::ToTranslate => crate::icons::DOT,
            Status::Validated => crate::icons::CHECKS,
            Status::Pending => crate::icons::DOTS_THREE,
            Status::BySimilarity => crate::icons::APPROXIMATE_EQUALS,
            Status::ModifiedProposal => crate::icons::PENCIL_SIMPLE,
            Status::Custom => crate::icons::STAR,
            Status::Ignored => crate::icons::PROHIBIT,
        }
    }

    /// Whether an entry with this status is exported (CDC §7: only Validé).
    pub fn is_exported(self) -> bool {
        matches!(self, Status::Validated)
    }
}

/// Derive the status of an entry from its stored flags and text. Best-effort so
/// imported memories (SST bits / EET codes) show meaningful colors.
pub fn derive(e: &TmEntry) -> Status {
    let has = |name: &str| e.status_sst.iter().any(|s| s == name);
    // The file lock of the VMAD lines (C-08) holds whatever lies under it.
    if has("ignored") || has(crate::vmad_lock::OFF_BIT) {
        Status::Ignored
    } else if has("validated") {
        Status::Validated
    } else if has("pending") || has("incompleteTrans") {
        Status::Pending
    } else if has("lockedTrans") {
        Status::Custom
    } else if has("modified") {
        Status::ModifiedProposal
    } else if e.trans.is_empty() {
        Status::ToTranslate
    } else {
        // Translated with no more specific flag: a memory/fuzzy fill.
        Status::BySimilarity
    }
}

/// Write a status back onto an entry's flags (so it round-trips and re-derives).
pub fn set(e: &mut TmEntry, status: Status) {
    e.status_sst.clear();
    match status {
        Status::ToTranslate => {
            e.status_eet = -1;
        }
        Status::Validated => {
            e.status_sst = vec!["translated".into(), "validated".into()];
            e.status_eet = 99;
        }
        Status::Pending => {
            e.status_sst = vec!["translated".into(), "pending".into()];
            e.status_eet = 99;
        }
        Status::BySimilarity => {
            e.status_sst = vec!["translated".into()];
            e.status_eet = 99;
        }
        Status::ModifiedProposal => {
            e.status_sst = vec!["translated".into(), "modified".into()];
            e.status_eet = 99;
        }
        Status::Custom => {
            e.status_sst = vec!["translated".into(), "lockedTrans".into()];
            e.status_eet = 99;
        }
        Status::Ignored => {
            e.status_sst = vec!["ignored".into()];
            e.status_eet = -1;
        }
    }
}

/// The status of a line whose translation was just typed by hand
/// (decision D-94, manual § 9): a line that had no translation becomes
/// Validé — the typed text is the translator's own; an automatic
/// translation (Traduit par similarité, En attente) retouched becomes
/// Proposition modifiée; Validé, Personnalisé and Proposition modifiée
/// keep their status; a translation erased brings the line back to À
/// traduire. `before_trans` / `before` are the line before the typing; a
/// line Ignoré that the typing unlocked (a rule's lock, C-08 / B-18) counts
/// as a line without translation.
pub fn after_typing(e: &mut TmEntry, before_trans: &str, before: Status) {
    if e.trans == before_trans || e.unused {
        return;
    }
    let now = derive(e);
    let before = if before == Status::Ignored && now != Status::Ignored {
        Status::ToTranslate
    } else {
        before
    };
    if before == Status::Ignored {
        return;
    }
    if e.trans.is_empty() {
        set(e, Status::ToTranslate);
        return;
    }
    let target = if before_trans.is_empty() || before == Status::ToTranslate {
        Status::Validated
    } else if matches!(before, Status::BySimilarity | Status::Pending) {
        Status::ModifiedProposal
    } else {
        return;
    };
    if target.applicable_to(e) {
        set(e, target);
    }
}

#[cfg(test)]
mod tests {
    fn typed(before_trans: &str, before: Status, now: &str) -> Status {
        let mut e = TmEntry {
            source: "Iron Sword".into(),
            trans: before_trans.into(),
            ..Default::default()
        };
        set(&mut e, before);
        let b = derive(&e);
        e.trans = now.into();
        after_typing(&mut e, before_trans, b);
        derive(&e)
    }

    #[test]
    fn typing_gives_the_status_of_d94() {
        assert_eq!(
            typed("", Status::ToTranslate, "Épée en fer"),
            Status::Validated
        );
        assert_eq!(
            typed("Épée de fer", Status::BySimilarity, "Épée en fer"),
            Status::ModifiedProposal
        );
        assert_eq!(
            typed("Épée de fer", Status::Pending, "Épée en fer"),
            Status::ModifiedProposal
        );
        assert_eq!(
            typed("Épée de fer", Status::Validated, "Épée en fer"),
            Status::Validated
        );
        assert_eq!(
            typed("Épée de fer", Status::Custom, "Épée en fer"),
            Status::Custom
        );
        assert_eq!(
            typed("Épée de fer", Status::Validated, ""),
            Status::ToTranslate
        );
        // The same text as the source is not a real translation: not Validé.
        assert_ne!(
            typed("", Status::ToTranslate, "Iron Sword"),
            Status::Validated
        );
        // Unchanged text: nothing moves.
        assert_eq!(
            typed("Épée", Status::BySimilarity, "Épée"),
            Status::BySimilarity
        );
    }

    use super::*;

    #[test]
    fn applicable_statuses_follow_r_a03_02() {
        let mut e = TmEntry {
            source: "a".into(),
            ..Default::default()
        };
        assert!(!Status::Validated.applicable_to(&e), "no translation");
        assert!(!Status::Custom.applicable_to(&e));
        assert!(Status::Pending.applicable_to(&e));
        assert!(Status::Ignored.applicable_to(&e));
        e.trans = "b".into();
        assert!(Status::Validated.applicable_to(&e));
        e.unused = true;
        assert!(
            !Status::Validated.applicable_to(&e) && !Status::Ignored.applicable_to(&e),
            "never shown in game"
        );
    }

    #[test]
    fn keys_roundtrip() {
        for s in Status::ALL {
            assert_eq!(Status::from_key(s.key()), Some(s));
            assert_eq!(Status::ALL[s.index()], s);
        }
    }

    #[test]
    fn set_then_derive_is_stable() {
        for s in Status::ALL {
            let mut e = TmEntry {
                source: "Hello".into(),
                // ToTranslate means "no translation yet"; every other status
                // implies a translation exists.
                trans: if s == Status::ToTranslate {
                    String::new()
                } else {
                    "Bonjour".into()
                },
                ..Default::default()
            };
            set(&mut e, s);
            assert_eq!(derive(&e), s, "{s:?} must round-trip through set/derive");
        }
    }

    #[test]
    fn untranslated_derives_to_translate() {
        let e = TmEntry {
            source: "Hi".into(),
            ..Default::default()
        };
        assert_eq!(derive(&e), Status::ToTranslate);
    }

    #[test]
    fn only_validated_is_exported() {
        assert!(Status::Validated.is_exported());
        assert!(!Status::Pending.is_exported());
        assert!(!Status::BySimilarity.is_exported());
    }
}
