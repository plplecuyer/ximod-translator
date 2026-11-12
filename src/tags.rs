//! Aliases and numbers (A-07): the engine tags (`<Alias=…>`, `<Global=…>`,
//! `<Token.Name=…>`, `<BaseName>`, `<Pronoun…>`, book markup…) and the
//! numbers of a source, compared with those of its translation; the
//! anomalies found and the two repairs, « Rétablir les alias » and
//! « Rétablir les nombres ». Pure functions over strings; the tag list is
//! the game's (`tags` of its fields file, A-06).

use crate::games::Game;

/// Which `<…>` tags count: the game's list (names, `*` suffix = prefix),
/// or every tag when the list is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagSet {
    pub names: Vec<String>,
}

impl TagSet {
    pub fn of(game: Game) -> TagSet {
        TagSet {
            names: crate::games::registry().fields(game).tags().to_vec(),
        }
    }

    /// Every `<…>` tag counts.
    pub fn any() -> TagSet {
        TagSet::default()
    }

    /// The set of a line: the placeholders of text documents (`{0}`,
    /// `%s`, C-05) for a TXT line, the game's tags otherwise.
    pub fn for_entry(e: &crate::tm::TmEntry, game: Game) -> TagSet {
        if crate::textfmt::is_text_field(&e.field_sig) {
            TagSet {
                names: crate::textfmt::text_tags().to_vec(),
            }
        } else {
            TagSet::of(game)
        }
    }

    /// Whether `{…}` placeholders count (`{*}` listed).
    fn braces(&self) -> bool {
        self.names.iter().any(|n| n.starts_with('{'))
    }

    /// Whether `%x` placeholders count (`%*` listed).
    fn percents(&self) -> bool {
        self.names.iter().any(|n| n.starts_with('%'))
    }

    pub fn from_names(names: &[&str]) -> TagSet {
        TagSet {
            names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Whether a tag (its text inside the brackets) is one of the game's:
    /// its name — up to the first `=`, `.`, `:` or blank — equals a listed
    /// name, or the tag starts with a listed prefix (`Alias*`), case aside.
    fn accepts(&self, inner: &str) -> bool {
        if self.names.is_empty() {
            return true;
        }
        let name = inner
            .split(|c: char| matches!(c, '=' | '.' | ':') || c.is_whitespace())
            .next()
            .unwrap_or(inner);
        self.names.iter().any(|n| match n.strip_suffix('*') {
            Some(prefix) => inner
                .get(..prefix.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(prefix)),
            None => name.eq_ignore_ascii_case(n),
        })
    }
}

/// One tag with its byte span in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// The tags of a text in order of appearance (R-A07-01): `<` followed by a
/// non-blank, up to the next `>`, with no nested `<`; plus, when the set
/// lists them, the `{…}` and `%x` placeholders of text files (C-05).
pub fn tags(text: &str, set: &TagSet) -> Vec<Span> {
    let mut out = angle_tags(text, set);
    if set.braces() || set.percents() {
        out.extend(placeholders(text, set));
        out.sort_by_key(|s| s.start);
    }
    out
}

/// `{…}` (no nested brace, no blank) and `%` followed by letters or
/// digits (`%s`, `%d`, `%1`).
fn placeholders(text: &str, set: &TagSet) -> Vec<Span> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && set.braces() {
            let rest = &text[i + 1..];
            if let Some(close) = rest.find(['}', '{'])
                && rest.as_bytes()[close] == b'}'
                && !rest[..close].contains(char::is_whitespace)
            {
                let end = i + 1 + close + 1;
                out.push(Span {
                    text: text[i..end].to_string(),
                    start: i,
                    end,
                });
                i = end;
                continue;
            }
        }
        if bytes[i] == b'%' && set.percents() {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            if j > i + 1 {
                out.push(Span {
                    text: text[i..j].to_string(),
                    start: i,
                    end: j,
                });
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn angle_tags(text: &str, set: &TagSet) -> Vec<Span> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &text[i + 1..];
            let first = rest.chars().next();
            if first.is_some_and(|c| !c.is_whitespace() && c != '<' && c != '>')
                && let Some(close) = rest.find(['>', '<'])
                && rest.as_bytes()[close] == b'>'
            {
                let inner = &rest[..close];
                let end = i + 1 + close + 1;
                if set.accepts(inner) {
                    out.push(Span {
                        text: text[i..end].to_string(),
                        start: i,
                        end,
                    });
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The numbers of a text (digits with an optional decimal separator `.` or
/// `,`), outside its tags.
pub fn numbers(text: &str, set: &TagSet) -> Vec<Span> {
    let mut tag_spans = tags(text, &TagSet::any());
    // The placeholders of a text line are not numbers (`{0}`, `%1`).
    tag_spans.extend(placeholders(text, set));
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(t) = tag_spans.iter().find(|t| t.start == i) {
            i = t.end;
            continue;
        }
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            // A decimal part: separator followed by digits.
            if i + 1 < bytes.len()
                && matches!(bytes[i], b'.' | b',')
                && bytes[i + 1].is_ascii_digit()
            {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            out.push(Span {
                text: text[start..i].to_string(),
                start,
                end: i,
            });
            continue;
        }
        i += 1;
    }
    out
}

/// What the comparison found (R-A07-02).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// In the source, not in the translation.
    pub missing_tags: Vec<String>,
    /// In the translation only.
    pub unknown_tags: Vec<String>,
    pub missing_numbers: Vec<String>,
    pub unknown_numbers: Vec<String>,
    /// Same tags, another order (a warning only).
    pub order_differs: bool,
    /// The source has no tag and no number.
    pub nothing_to_compare: bool,
}

impl Report {
    /// Compare a source and its translation; an empty translation has
    /// nothing to compare (R-A07-05).
    pub fn new(source: &str, trans: &str, set: &TagSet) -> Report {
        let mut r = Report::default();
        let st: Vec<String> = tags(source, set).into_iter().map(|s| s.text).collect();
        let sn: Vec<String> = numbers(source, set).into_iter().map(|s| s.text).collect();
        r.nothing_to_compare = st.is_empty() && sn.is_empty();
        if trans.trim().is_empty() {
            return r;
        }
        let tt: Vec<String> = tags(trans, set).into_iter().map(|s| s.text).collect();
        let tn: Vec<String> = numbers(trans, set).into_iter().map(|s| s.text).collect();
        let (missing, unknown) = diff_multiset(&st, &tt, true);
        r.missing_tags = missing;
        r.unknown_tags = unknown;
        let (missing, unknown) = diff_multiset(&sn, &tn, false);
        r.missing_numbers = missing;
        r.unknown_numbers = unknown;
        if r.missing_tags.is_empty() && r.unknown_tags.is_empty() && !st.is_empty() {
            let a: Vec<String> = st.iter().map(|s| s.to_lowercase()).collect();
            let b: Vec<String> = tt.iter().map(|s| s.to_lowercase()).collect();
            r.order_differs = a != b;
        }
        r
    }

    /// The report of an entry in a game.
    pub fn of(e: &crate::tm::TmEntry, game: Game) -> Report {
        Report::new(&e.source, &e.trans, &TagSet::for_entry(e, game))
    }

    /// Something is missing or unknown (an anomaly for D-05).
    pub fn is_error(&self) -> bool {
        !(self.missing_tags.is_empty()
            && self.unknown_tags.is_empty()
            && self.missing_numbers.is_empty()
            && self.unknown_numbers.is_empty())
    }

    /// Error or order warning: shown by the « Anomalies » filter.
    pub fn is_anomaly(&self) -> bool {
        self.is_error() || self.order_differs
    }

    pub fn is_clean(&self) -> bool {
        !self.is_anomaly()
    }
}

/// Multiset difference, case-insensitive for tags: `(in a not in b, in b
/// not in a)`, in order of appearance.
fn diff_multiset(a: &[String], b: &[String], fold_case: bool) -> (Vec<String>, Vec<String>) {
    let key = |s: &String| {
        if fold_case {
            s.to_lowercase()
        } else {
            s.clone()
        }
    };
    let mut remaining_b: Vec<Option<String>> = b.iter().map(|s| Some(key(s))).collect();
    let mut missing = Vec::new();
    for x in a {
        let k = key(x);
        match remaining_b
            .iter()
            .position(|y| y.as_deref() == Some(k.as_str()))
        {
            Some(p) => remaining_b[p] = None,
            None => missing.push(x.clone()),
        }
    }
    let unknown: Vec<String> = b
        .iter()
        .zip(remaining_b.iter())
        .filter(|(_, r)| r.is_some())
        .map(|(s, _)| s.clone())
        .collect();
    (missing, unknown)
}

/// Result of a repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fixed {
    pub text: String,
    /// Tags (or numbers) replaced in place.
    pub replaced: usize,
    /// Tags (or numbers) appended at the end of the text.
    pub appended: usize,
}

/// « Rétablir les alias » (R-A07-04): the i-th tag of the translation
/// becomes the i-th tag of the source; when the translation has fewer, the
/// missing ones are appended, space-separated; when it has more, the extra
/// ones are left.
pub fn fix_tags(source: &str, trans: &str, set: &TagSet) -> Fixed {
    let src = tags(source, set);
    let dst = tags(trans, set);
    fix_spans(trans, &src, &dst)
}

/// « Rétablir les nombres », the same way.
pub fn fix_numbers(source: &str, trans: &str, set: &TagSet) -> Fixed {
    let src = numbers(source, set);
    let dst = numbers(trans, set);
    fix_spans(trans, &src, &dst)
}

fn fix_spans(trans: &str, src: &[Span], dst: &[Span]) -> Fixed {
    let mut out = String::with_capacity(trans.len() + 16);
    let mut last = 0;
    let mut replaced = 0;
    for (i, d) in dst.iter().enumerate() {
        out.push_str(&trans[last..d.start]);
        match src.get(i) {
            Some(s) => {
                if s.text != d.text {
                    replaced += 1;
                }
                out.push_str(&s.text);
            }
            None => out.push_str(&d.text),
        }
        last = d.end;
    }
    out.push_str(&trans[last..]);
    let mut appended = 0;
    for s in src.iter().skip(dst.len()) {
        if !out.is_empty() && !out.ends_with(char::is_whitespace) {
            out.push(' ');
        }
        out.push_str(&s.text);
        appended += 1;
    }
    Fixed {
        text: out,
        replaced,
        appended,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ce() -> TagSet {
        TagSet::from_names(&[
            "Alias*", "Global*", "Token*", "BaseName", "Pronoun*", "font", "/font", "br",
        ])
    }

    fn texts(v: &[Span]) -> Vec<&str> {
        v.iter().map(|s| s.text.as_str()).collect()
    }

    /// R-C05-06: the placeholders of an MCM value count as tags, not as
    /// numbers; a plugin line ignores them.
    #[test]
    fn text_placeholders_count_as_tags() {
        let set = TagSet::from_names(&["{*}", "%*"]);
        let src = "Level {0} of {1}: %s items, 12 gold, {a b}";
        assert_eq!(texts(&tags(src, &set)), vec!["{0}", "{1}", "%s"]);
        assert_eq!(texts(&numbers(src, &set)), vec!["12"]);
        let r = Report::new(src, "Niveau {0} : %s objets, 12 or", &set);
        assert_eq!(r.missing_tags, vec!["{1}"]);
        assert!(r.is_error());
        let game = TagSet::from_names(&["Alias*"]);
        assert!(tags(src, &game).is_empty());
        assert_eq!(texts(&numbers(src, &game)), vec!["0", "1", "12"]);
        let mut e = crate::tm::TmEntry {
            source: "Hello {0}".into(),
            trans: "Bonjour".into(),
            field_sig: "TXT".into(),
            ..Default::default()
        };
        assert!(Report::of(&e, Game::SkyrimSE).is_error());
        // A plugin line: « {0} » is no tag, its 0 is a number.
        e.field_sig = "FULL".into();
        e.trans = "Bonjour 0".into();
        assert!(!Report::of(&e, Game::SkyrimSE).is_error());
    }

    /// Recette of lot 1 (étape 3): the magic effect placeholders of
    /// Skyrim are tags — a translation that drops one is an error.
    #[test]
    fn skyrim_magic_placeholders_are_checked() {
        let e = crate::tm::TmEntry {
            rec_sig: "MGEF".into(),
            field_sig: "DNAM".into(),
            source: "Deals <mag> damage for <dur> seconds in <area> feet.".into(),
            trans: "Inflige <mag> points de dégâts pendant 10 secondes.".into(),
            ..Default::default()
        };
        for game in [Game::Skyrim, Game::SkyrimSE] {
            let r = Report::of(&e, game);
            assert_eq!(r.missing_tags, vec!["<dur>", "<area>"], "{game:?}");
            assert!(r.is_error());
        }
    }

    #[test]
    fn tags_are_extracted_in_order_and_filtered_by_the_game_list() {
        let t = tags(
            "Bring <Alias=Victim> to <Alias.Location=Home>, <BaseName> <Pronoun>; a < b > c <unknown>",
            &ce(),
        );
        assert_eq!(
            texts(&t),
            [
                "<Alias=Victim>",
                "<Alias.Location=Home>",
                "<BaseName>",
                "<Pronoun>"
            ]
        );
        // Every tag counts when the list is empty; « < b > » (blank after <) never.
        let t = tags("a < b > c <unknown> <x", &TagSet::any());
        assert_eq!(texts(&t), ["<unknown>"]);
        let t = tags("<font face='$HandwrittenFont'>Text</font>", &ce());
        assert_eq!(texts(&t), ["<font face='$HandwrittenFont'>", "</font>"]);
    }

    #[test]
    fn numbers_skip_the_tags() {
        let n = numbers("3 jours, 2,5 kg et <Global=Gold10> 1.000", &ce());
        assert_eq!(texts(&n), ["3", "2,5", "1.000"]);
    }

    #[test]
    fn acceptance_t_a07_01() {
        let set = ce();
        let r = Report::new(
            "Bring <Alias=Victim> to <Alias=Location>",
            "Amenez <Alias=Victime> à <Alias=Location>",
            &set,
        );
        assert_eq!(r.missing_tags, ["<Alias=Victim>"]);
        assert_eq!(r.unknown_tags, ["<Alias=Victime>"]);
        assert!(r.is_error());
        let f = fix_tags(
            "Bring <Alias=Victim> to <Alias=Location>",
            "Amenez <Alias=Victime> à <Alias=Location>",
            &set,
        );
        assert_eq!(f.text, "Amenez <Alias=Victim> à <Alias=Location>");
        assert_eq!((f.replaced, f.appended), (1, 0));
    }

    #[test]
    fn acceptance_t_a07_02_numbers_appended() {
        let set = ce();
        let r = Report::new("3 jours", "trois jours", &set);
        assert_eq!(r.missing_numbers, ["3"]);
        let f = fix_numbers("3 jours", "trois jours", &set);
        assert_eq!(f.text, "trois jours 3");
        assert_eq!((f.replaced, f.appended), (0, 1));
    }

    #[test]
    fn acceptance_t_a07_03_order_is_a_warning() {
        let set = ce();
        let r = Report::new("<Alias=A> and <Alias=B>", "<Alias=B> et <Alias=A>", &set);
        assert!(r.order_differs && !r.is_error() && r.is_anomaly());
        // Case does not matter.
        let r = Report::new("<Alias=A>", "<alias=a>", &set);
        assert!(r.is_clean());
    }

    #[test]
    fn acceptance_t_a07_04_empty_translation() {
        let r = Report::new("<Alias=A> 3", "", &ce());
        assert!(r.is_clean() && !r.nothing_to_compare);
        let r = Report::new("plain text", "texte", &ce());
        assert!(r.is_clean() && r.nothing_to_compare);
    }

    #[test]
    fn repairs_keep_the_rest_of_the_text() {
        let set = ce();
        let f = fix_tags("<Alias=A> x <Alias=B> y <Alias=C>", "<Alias=Z> a", &set);
        assert_eq!(f.text, "<Alias=A> a <Alias=B> <Alias=C>");
        assert_eq!((f.replaced, f.appended), (1, 2));
        // Nothing to do: unchanged.
        let f = fix_numbers("10 gold", "10 pièces", &set);
        assert_eq!(f.text, "10 pièces");
        assert_eq!((f.replaced, f.appended), (0, 0));
    }

    #[test]
    fn the_shipped_tables_list_the_engine_tags() {
        assert!(TagSet::of(Game::Skyrim).names.iter().any(|n| n == "Alias*"));
        assert!(
            TagSet::of(Game::SkyrimSE)
                .names
                .iter()
                .any(|n| n == "Alias*"),
            "inherited"
        );
        assert!(TagSet::of(Game::Morrowind).names.is_empty());
    }
}
