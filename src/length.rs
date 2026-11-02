//! Length counter (A-05): characters and bytes of a translation in the
//! target game's encoding, against the limits of its field rule (A-06:
//! `maxChars`, `maxBytes`, `newline`). Pure functions, shared by the grid
//! cell, the edit window and the quality checks (D-05).

use crate::games::Game;
use crate::tm::TmEntry;

/// The limits that apply to one line (R-A05-04).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    pub max_chars: Option<u32>,
    pub max_bytes: Option<u32>,
    /// `None` while the table does not know.
    pub newline: Option<bool>,
}

impl Limits {
    /// The limits of a field in a game's table: its signature rule.
    pub fn of(game: Game, rec: &str, fld: &str) -> Limits {
        match crate::games::registry().fields(game).limits(rec, fld) {
            Some(r) => Limits {
                max_chars: r.max_chars,
                max_bytes: r.max_bytes,
                newline: r.newline,
            },
            None => Limits::default(),
        }
    }

    pub fn has_limit(&self) -> bool {
        self.max_chars.is_some() || self.max_bytes.is_some()
    }
}

/// Length of a text in the game's encoding (R-A05-01): UTF-8 bytes, or one
/// byte per character in Windows-1252 (a character the code page cannot
/// represent still counts one; D-05 reports it).
pub fn byte_len(text: &str, utf8: bool) -> usize {
    if utf8 {
        text.len()
    } else {
        text.chars().count()
    }
}

/// Length of a text in a page of code (D-09): one byte per character in a
/// Windows page (a character the page cannot hold still counts one), the
/// encoded length otherwise.
pub fn byte_len_page(text: &str, page: crate::codepage::Page) -> usize {
    if page.is_single_byte() {
        text.chars().count()
    } else {
        page.encode(text).len()
    }
}

/// What the counter shows for one translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub chars: usize,
    pub bytes: usize,
    pub limits: Limits,
    /// The translation holds a line break although the field is single-line.
    pub bad_newline: bool,
}

/// How the counter is coloured (R-A05-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Within limits, or no limit known.
    Ok,
    /// At 90 % of the limit or more.
    Near,
    /// Over the limit.
    Over,
}

impl Report {
    pub fn new(text: &str, limits: Limits, utf8: bool) -> Report {
        Report::new_page(
            text,
            limits,
            if utf8 {
                crate::codepage::Page::Utf8
            } else {
                crate::codepage::Page::Win1252
            },
        )
    }

    /// [`Report::new`] in a page of code (D-09).
    pub fn new_page(text: &str, limits: Limits, page: crate::codepage::Page) -> Report {
        let chars = text.chars().count();
        let bytes = byte_len_page(text, page);
        let bad_newline = limits.newline == Some(false) && text.contains(['\n', '\r']);
        Report {
            chars,
            bytes,
            limits,
            bad_newline,
        }
    }

    /// The counter of an entry in a game.
    pub fn of(e: &TmEntry, game: Game) -> Report {
        Report::new(
            &e.trans,
            Limits::of(game, &e.rec_sig, &e.field_sig),
            game.uses_utf8(),
        )
    }

    /// The counter of an entry in a game, written in `page` (D-09).
    pub fn of_page(e: &TmEntry, game: Game, page: crate::codepage::Page) -> Report {
        Report::new_page(&e.trans, Limits::of(game, &e.rec_sig, &e.field_sig), page)
    }

    /// Over the character limit, the byte limit, or both (the most
    /// constraining one wins, R-A05-04).
    pub fn over(&self) -> bool {
        self.limits
            .max_chars
            .is_some_and(|m| self.chars > m as usize)
            || self
                .limits
                .max_bytes
                .is_some_and(|m| self.bytes > m as usize)
    }

    /// Fraction of the tightest limit used (0 when none).
    pub fn ratio(&self) -> f32 {
        let rc = self
            .limits
            .max_chars
            .map(|m| self.chars as f32 / m.max(1) as f32)
            .unwrap_or(0.0);
        let rb = self
            .limits
            .max_bytes
            .map(|m| self.bytes as f32 / m.max(1) as f32)
            .unwrap_or(0.0);
        rc.max(rb)
    }

    pub fn level(&self) -> Level {
        if self.over() {
            Level::Over
        } else if self.limits.has_limit() && self.ratio() >= 0.9 {
            Level::Near
        } else {
            Level::Ok
        }
    }

    /// Something the grid must show: over a limit or a forbidden line break
    /// (an anomaly for D-05).
    pub fn is_anomaly(&self) -> bool {
        self.over() || self.bad_newline
    }

    /// « 34 / 32 » — the count against the limit it breaks (or the tightest
    /// one): characters when a character limit exists, bytes otherwise.
    /// `unit` is the bytes unit of the interface language (« o », « B »).
    pub fn badge(&self, unit: &str) -> String {
        match (self.limits.max_chars, self.limits.max_bytes) {
            (Some(mc), Some(mb)) => {
                let over_c = self.chars > mc as usize;
                let over_b = self.bytes > mb as usize;
                if over_b && !over_c {
                    format!("{} / {} {unit}", self.bytes, mb)
                } else {
                    format!("{} / {}", self.chars, mc)
                }
            }
            (Some(mc), None) => format!("{} / {}", self.chars, mc),
            (None, Some(mb)) => format!("{} / {} {unit}", self.bytes, mb),
            (None, None) => self.chars.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lim(mc: Option<u32>, mb: Option<u32>, nl: Option<bool>) -> Limits {
        Limits {
            max_chars: mc,
            max_bytes: mb,
            newline: nl,
        }
    }

    #[test]
    fn bytes_follow_the_encoding() {
        // T-A05-02: « Épée » — 4 chars; 4 bytes in Windows-1252, 6 in UTF-8
        // (two accented letters at two bytes each; the test sheet's « 5 »
        // was a slip).
        assert_eq!(byte_len("Épée", false), 4);
        assert_eq!(byte_len("Épée", true), 6);
        let r = Report::new("Épée", Limits::default(), true);
        assert_eq!((r.chars, r.bytes), (4, 6));
    }

    #[test]
    fn levels_and_badge() {
        let l = lim(Some(32), None, Some(false));
        let r = Report::new(&"a".repeat(34), l, true);
        assert!(r.over());
        assert_eq!(r.level(), Level::Over);
        assert_eq!(r.badge("o"), "34 / 32");
        assert!(r.is_anomaly());
        let r = Report::new(&"a".repeat(29), l, true);
        assert_eq!(r.level(), Level::Near);
        let r = Report::new(&"a".repeat(10), l, true);
        assert_eq!(r.level(), Level::Ok);
        assert!(!r.is_anomaly());
        // No limit: no colour, no denominator (T-A05-04).
        let r = Report::new("hello", Limits::default(), true);
        assert_eq!(r.level(), Level::Ok);
        assert_eq!(r.badge("o"), "5");
    }

    #[test]
    fn tightest_limit_wins() {
        // 31 bytes but a UTF-8 text of 20 chars can weigh more.
        let l = lim(Some(40), Some(31), None);
        let r = Report::new(&"é".repeat(20), l, true); // 20 chars, 40 bytes
        assert!(r.over());
        assert_eq!(r.badge("o"), "40 / 31 o");
        let r = Report::new(&"é".repeat(20), l, false); // 20 bytes in cp1252
        assert!(!r.over());
    }

    #[test]
    fn newline_in_single_line_field() {
        let r = Report::new("Read\nthe note", lim(None, None, Some(false)), true);
        assert!(r.bad_newline && r.is_anomaly() && !r.over());
        let r = Report::new("Read\nthe note", lim(None, None, Some(true)), true);
        assert!(!r.bad_newline);
        let r = Report::new("Read\nthe note", lim(None, None, None), true);
        assert!(!r.bad_newline);
    }

    #[test]
    fn limits_come_from_the_shipped_tables() {
        // Morrowind names: 31 bytes, single line; a Skyrim description is
        // multi-line without a known limit.
        let l = Limits::of(Game::Morrowind, "WEAP", "FNAM");
        assert_eq!((l.max_bytes, l.newline), (Some(31), Some(false)));
        let l = Limits::of(Game::Skyrim, "BOOK", "DESC");
        assert_eq!((l.max_chars, l.newline), (None, Some(true)));
        assert_eq!(
            Limits::of(Game::SkyrimSE, "WEAP", "FULL").newline,
            Some(false)
        );
        assert_eq!(Limits::of(Game::Skyrim, "ZZZZ", "FULL"), Limits::default());
    }
}
