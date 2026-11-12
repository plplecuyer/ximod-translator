//! The marks of a line (A-16, D-111): the checks the application already
//! makes — tags and numbers, length and line breaks, encoding, the last
//! quality check, ambiguity, the spell checker's last scan — gathered in one
//! computation, plus two trivial ones (spaces at the ends, double space).
//! The grid shows them as a margin bar and in a rich tooltip; the nature of
//! a line (dialogue, book, script, message) gives the icon of its St cell.
//! Nothing here knows the interface: a mark is a message key and its
//! arguments.

use crate::codepage::Page;
use crate::games::Game;
use crate::tm::TmEntry;

/// What a line is, for the icon of the St cell (R-A16-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nature {
    Dialogue,
    Book,
    Script,
    Message,
}

impl Nature {
    /// The message key naming the nature.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Nature::Dialogue => "nature-dialogue",
            Nature::Book => "nature-book",
            Nature::Script => "nature-script",
            Nature::Message => "nature-message",
        }
    }
}

/// The nature of a line, from its record and field; `None` for the others.
pub fn nature(e: &TmEntry) -> Option<Nature> {
    let (rec, fld) = (e.rec_sig.as_str(), e.field_sig.as_str());
    if fld == crate::vmad_lock::FIELD || rec == "SCPT" {
        return Some(Nature::Script);
    }
    match (rec, fld) {
        ("INFO", "NAM1") | ("INFO", "RNAM") | ("DIAL", "FULL") => Some(Nature::Dialogue),
        // Morrowind keeps its replies in INFO:NAME.
        ("INFO", "NAME") => Some(Nature::Dialogue),
        ("BOOK", "DESC") | ("BOOK", "TEXT") | ("NOTE", "TNAM") | ("TERM", "DESC") => {
            Some(Nature::Book)
        }
        ("MESG", "DESC") | ("MESG", "FULL") | ("MESG", "ITXT") | ("QUST", "NNAM") => {
            Some(Nature::Message)
        }
        _ => None,
    }
}

/// How grave a mark is: the margin bar takes the colour of the gravest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Spelling,
    Warning,
    Error,
}

/// One finding on a line: a message key and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub severity: Severity,
    pub key: &'static str,
    pub args: Vec<(&'static str, String)>,
}

impl Mark {
    fn new(severity: Severity, key: &'static str) -> Mark {
        Mark {
            severity,
            key,
            args: Vec::new(),
        }
    }

    fn arg(mut self, name: &'static str, value: impl ToString) -> Mark {
        self.args.push((name, value.to_string()));
        self
    }
}

/// What the marks of a line depend on besides the line itself.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub game: Game,
    /// The page of code of the output: what cannot be written in it.
    pub page: Page,
    /// The text of the last quality check for this line, if it was flagged.
    pub quality: Option<&'a str>,
    /// The words the spell checker flagged in this line at its last scan of
    /// the document (none when it was not run).
    pub spelling: &'a [String],
}

/// The marks of a line, gravest first (stable within a gravity).
pub fn of(e: &TmEntry, cx: &Context<'_>) -> Vec<Mark> {
    let mut out = Vec::new();
    if e.unused {
        return out;
    }
    let tags = crate::tags::Report::of(e, cx.game);
    for t in &tags.missing_tags {
        out.push(Mark::new(Severity::Error, "mark-alias-missing").arg("tag", t));
    }
    for t in &tags.unknown_tags {
        out.push(Mark::new(Severity::Error, "mark-alias-unknown").arg("tag", t));
    }
    for n in &tags.missing_numbers {
        out.push(Mark::new(Severity::Error, "mark-number-missing").arg("n", n));
    }
    for n in &tags.unknown_numbers {
        out.push(Mark::new(Severity::Warning, "mark-number-unknown").arg("n", n));
    }
    if tags.order_differs {
        out.push(Mark::new(Severity::Warning, "mark-alias-order"));
    }
    let len = crate::length::Report::of_page(e, cx.game, cx.page);
    if len.over() {
        let max = len.limits.max_chars.or(len.limits.max_bytes).unwrap_or(0);
        let n = if len.limits.max_chars.is_some() {
            len.chars
        } else {
            len.bytes
        };
        out.push(
            Mark::new(Severity::Warning, "mark-too-long")
                .arg("n", n)
                .arg("max", max),
        );
    }
    if len.bad_newline {
        out.push(Mark::new(Severity::Error, "mark-newline"));
    }
    if !e.trans.is_empty() && !cx.page.can_encode(&e.trans) {
        let mut chars: Vec<char> = cx
            .page
            .unencodable(&e.trans)
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        chars.dedup();
        let shown: String = chars.iter().take(8).collect();
        out.push(
            Mark::new(Severity::Error, "mark-encoding")
                .arg("enc", cx.page.short())
                .arg("chars", shown),
        );
    }
    if spaces_differ(&e.source, &e.trans) {
        out.push(Mark::new(Severity::Warning, "mark-spaces"));
    }
    if e.trans.contains("  ") && !e.source.contains("  ") {
        out.push(Mark::new(Severity::Warning, "mark-double-space"));
    }
    if let Some(text) = cx.quality {
        out.push(Mark::new(Severity::Warning, "mark-quality").arg("text", text));
    }
    if let Some(n) = crate::ambiguity::count_of(e) {
        out.push(Mark::new(Severity::Warning, "mark-ambiguous").arg("n", n));
    }
    for w in cx.spelling {
        out.push(Mark::new(Severity::Spelling, "mark-spelling").arg("word", w));
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.severity));
    out
}

/// The gravest severity of `marks`, for the margin bar.
pub fn worst(marks: &[Mark]) -> Option<Severity> {
    marks.iter().map(|m| m.severity).max()
}

/// The translation starts or ends with spaces the source does not have
/// (a source that has them is followed).
fn spaces_differ(source: &str, trans: &str) -> bool {
    if trans.trim().is_empty() {
        return false;
    }
    let lead = |s: &str| s.len() - s.trim_start().len();
    let tail = |s: &str| s.len() - s.trim_end().len();
    (lead(trans) > 0 && lead(source) == 0) || (tail(trans) > 0 && tail(source) == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rec: &str, fld: &str, source: &str, trans: &str) -> TmEntry {
        TmEntry {
            rec_sig: rec.into(),
            field_sig: fld.into(),
            source: source.into(),
            trans: trans.into(),
            ..TmEntry::default()
        }
    }

    fn cx() -> Context<'static> {
        Context {
            game: Game::SkyrimSE,
            page: Page::Utf8,
            quality: None,
            spelling: &[],
        }
    }

    #[test]
    fn natures_come_from_record_and_field() {
        assert_eq!(
            nature(&entry("INFO", "NAM1", "", "")),
            Some(Nature::Dialogue)
        );
        assert_eq!(nature(&entry("BOOK", "DESC", "", "")), Some(Nature::Book));
        assert_eq!(nature(&entry("QUST", "VMAD", "", "")), Some(Nature::Script));
        assert_eq!(
            nature(&entry("MESG", "DESC", "", "")),
            Some(Nature::Message)
        );
        assert_eq!(nature(&entry("WEAP", "FULL", "", "")), None);
    }

    #[test]
    fn a_missing_number_is_an_error_first() {
        let e = entry(
            "MGEF",
            "DNAM",
            "Deals 25 damage for 10 seconds.",
            "Inflige 25 points de dégâts.  ",
        );
        let m = of(&e, &cx());
        assert_eq!(worst(&m), Some(Severity::Error));
        assert_eq!(m[0].key, "mark-number-missing");
        assert_eq!(m[0].args, vec![("n", "10".to_string())]);
        let keys: Vec<&str> = m.iter().map(|m| m.key).collect();
        assert!(keys.contains(&"mark-spaces"), "{keys:?}");
        assert!(keys.contains(&"mark-double-space"), "{keys:?}");
    }

    #[test]
    fn clean_lines_and_lines_never_shown_have_no_mark() {
        let e = entry("WEAP", "FULL", "Iron Sword", "Épée en fer");
        assert!(of(&e, &cx()).is_empty());
        let mut u = entry("MGEF", "DNAM", "<mag>", "");
        u.trans = "rien".into();
        u.unused = true;
        assert!(of(&u, &cx()).is_empty());
        // Spaces the source has are followed.
        let e = entry("WEAP", "FULL", "Iron Sword ", "Épée en fer ");
        assert!(of(&e, &cx()).is_empty());
    }

    #[test]
    fn quality_ambiguity_and_spelling_are_carried() {
        let e = entry("WEAP", "FULL", "Iron Sword", "Épée en fre");
        let words = vec!["fre".to_string()];
        let c = Context {
            quality: Some("Ponctuation finale différente"),
            spelling: &words,
            ..cx()
        };
        let m = of(&e, &c);
        assert_eq!(worst(&m), Some(Severity::Warning));
        assert_eq!(m.last().unwrap().key, "mark-spelling");
        let s = Context {
            spelling: &words,
            ..cx()
        };
        assert_eq!(worst(&of(&e, &s)), Some(Severity::Spelling));
    }

    #[test]
    fn what_the_output_page_cannot_write_is_an_error() {
        let e = entry("WEAP", "FULL", "Iron Sword", "Меч");
        let c = Context {
            page: Page::Win1252,
            ..cx()
        };
        let m = of(&e, &c);
        assert_eq!(m[0].key, "mark-encoding");
        assert_eq!(worst(&m), Some(Severity::Error));
    }
}
