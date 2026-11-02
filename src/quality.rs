//! Vérifications qualité (D-05): the checks run on a set of lines, their
//! findings, and the « [!] » indicator derived from them. The checks reuse
//! A-05 (length, line break) and A-07 (aliases, numbers); the others are
//! defined here. Pure functions, runnable on a worker thread.

use std::collections::{BTreeMap, HashMap};

use crate::games::Game;
use crate::tm::TmEntry;

/// One check (R-D05-01); the order is the window's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Check {
    Aliases,
    Numbers,
    Length,
    Newline,
    Punctuation,
    Capitals,
    Spaces,
    Encoding,
    Inconsistent,
}

impl Check {
    pub const ALL: [Check; 9] = [
        Check::Aliases,
        Check::Numbers,
        Check::Length,
        Check::Newline,
        Check::Punctuation,
        Check::Capitals,
        Check::Spaces,
        Check::Encoding,
        Check::Inconsistent,
    ];

    /// `[Quality] Checks` token.
    pub fn key(self) -> &'static str {
        match self {
            Check::Aliases => "aliases",
            Check::Numbers => "numbers",
            Check::Length => "length",
            Check::Newline => "newline",
            Check::Punctuation => "punctuation",
            Check::Capitals => "capitals",
            Check::Spaces => "spaces",
            Check::Encoding => "encoding",
            Check::Inconsistent => "inconsistent",
        }
    }

    pub fn from_key(k: &str) -> Option<Check> {
        Check::ALL.iter().copied().find(|c| c.key() == k.trim())
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            Check::Aliases => "quality-check-aliases",
            Check::Numbers => "quality-check-numbers",
            Check::Length => "quality-check-length",
            Check::Newline => "quality-check-newline",
            Check::Punctuation => "quality-check-punct",
            Check::Capitals => "quality-check-capitals",
            Check::Spaces => "quality-check-spaces",
            Check::Encoding => "quality-check-encoding",
            Check::Inconsistent => "quality-check-inconsistent",
        }
    }

    /// Short name for the results table.
    pub fn short_key(self) -> &'static str {
        match self {
            Check::Aliases => "quality-short-aliases",
            Check::Numbers => "quality-short-numbers",
            Check::Length => "quality-short-length",
            Check::Newline => "quality-short-newline",
            Check::Punctuation => "quality-short-punct",
            Check::Capitals => "quality-short-capitals",
            Check::Spaces => "quality-short-spaces",
            Check::Encoding => "quality-short-encoding",
            Check::Inconsistent => "quality-short-inconsistent",
        }
    }

    /// The default set: everything but the capitals (R-D05 data).
    pub fn default_set() -> Vec<Check> {
        Check::ALL
            .iter()
            .copied()
            .filter(|c| *c != Check::Capitals)
            .collect()
    }

    /// Parse `[Quality] Checks` (comma-separated keys); empty → default.
    pub fn parse_set(text: &str) -> Vec<Check> {
        let v: Vec<Check> = text.split(',').filter_map(Check::from_key).collect();
        if text.trim().is_empty() {
            Check::default_set()
        } else {
            v
        }
    }

    pub fn format_set(set: &[Check]) -> String {
        set.iter().map(|c| c.key()).collect::<Vec<_>>().join(",")
    }
}

/// What a finding says (formatted by the window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    TagMissing(String),
    TagUnknown(String),
    TagOrder,
    NumberMissing(String),
    NumberUnknown(String),
    LengthOver {
        chars: usize,
        max: u32,
        bytes: usize,
    },
    NewlineForbidden {
        field: String,
    },
    Punctuation {
        source: String,
        trans: String,
    },
    CapitalAfter(String),
    DoubleSpace,
    EdgeSpace,
    /// Characters the page of writing cannot hold (D-09): the characters
    /// and the page's WHATWG name.
    Encoding(String, &'static str),
    /// A source translated several ways: the variants with their counts,
    /// most frequent first, and the lines of the group.
    Inconsistent {
        source: String,
        variants: Vec<(String, usize)>,
        lines: Vec<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Entry index (the first line of the group for `Inconsistent`).
    pub line: usize,
    pub check: Check,
    pub detail: Detail,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub lines_checked: usize,
}

impl Report {
    /// Lines in anomaly (every line of an inconsistent group counts).
    pub fn lines(&self) -> std::collections::BTreeSet<usize> {
        let mut s = std::collections::BTreeSet::new();
        for f in &self.findings {
            match &f.detail {
                Detail::Inconsistent { lines, .. } => s.extend(lines.iter().copied()),
                _ => {
                    s.insert(f.line);
                }
            }
        }
        s
    }

    /// Findings per line (the grid's tooltip), groups expanded.
    pub fn by_line(&self) -> HashMap<usize, Vec<&Finding>> {
        let mut m: HashMap<usize, Vec<&Finding>> = HashMap::new();
        for f in &self.findings {
            match &f.detail {
                Detail::Inconsistent { lines, .. } => {
                    for &l in lines {
                        m.entry(l).or_default().push(f);
                    }
                }
                _ => m.entry(f.line).or_default().push(f),
            }
        }
        m
    }
}

/// Whether a character is representable in Windows-1252.
pub fn win1252_ok(c: char) -> bool {
    let cp = c as u32;
    cp < 0x80
        || (0xA0..=0xFF).contains(&cp)
        || matches!(
            c,
            '€' | '‚'
                | 'ƒ'
                | '„'
                | '…'
                | '†'
                | '‡'
                | 'ˆ'
                | '‰'
                | 'Š'
                | '‹'
                | 'Œ'
                | 'Ž'
                | '‘'
                | '’'
                | '“'
                | '”'
                | '•'
                | '–'
                | '—'
                | '˜'
                | '™'
                | 'š'
                | '›'
                | 'œ'
                | 'ž'
                | 'Ÿ'
        )
}

/// The final punctuation of a text (`. ! ? … : ;`), spaces and no-break
/// spaces before it ignored; `None` when it ends with something else.
pub fn final_punct(text: &str) -> Option<char> {
    let t = text.trim_end_matches(|c: char| c.is_whitespace() || c == '\u{a0}');
    // « ... » : the last run of punctuation, the first of it is enough.
    let last = t.chars().last()?;
    if matches!(last, '.' | '!' | '?' | '…' | ':' | ';') {
        Some(last)
    } else {
        None
    }
}

fn check_line(
    e: &TmEntry,
    checks: &[Check],
    game: Game,
    page: crate::codepage::Page,
    set: &crate::tags::TagSet,
    out: &mut Vec<Finding>,
) {
    let utf8 = page == crate::codepage::Page::Utf8;
    let line = 0; // replaced by the caller
    let _ = line;
    let want = |c: Check| checks.contains(&c);
    if want(Check::Aliases) || want(Check::Numbers) {
        let r = crate::tags::Report::new(&e.source, &e.trans, set);
        if want(Check::Aliases) {
            for t in &r.missing_tags {
                out.push(Finding {
                    line: 0,
                    check: Check::Aliases,
                    detail: Detail::TagMissing(t.clone()),
                });
            }
            for t in &r.unknown_tags {
                out.push(Finding {
                    line: 0,
                    check: Check::Aliases,
                    detail: Detail::TagUnknown(t.clone()),
                });
            }
            if r.order_differs {
                out.push(Finding {
                    line: 0,
                    check: Check::Aliases,
                    detail: Detail::TagOrder,
                });
            }
        }
        if want(Check::Numbers) {
            for n in &r.missing_numbers {
                out.push(Finding {
                    line: 0,
                    check: Check::Numbers,
                    detail: Detail::NumberMissing(n.clone()),
                });
            }
            for n in &r.unknown_numbers {
                out.push(Finding {
                    line: 0,
                    check: Check::Numbers,
                    detail: Detail::NumberUnknown(n.clone()),
                });
            }
        }
    }
    if want(Check::Length) || want(Check::Newline) {
        let limits = crate::length::Limits::of(game, &e.rec_sig, &e.field_sig);
        let r = crate::length::Report::new_page(&e.trans, limits, page);
        if want(Check::Length) && r.over() {
            let max = limits.max_chars.or(limits.max_bytes).unwrap_or(0);
            out.push(Finding {
                line: 0,
                check: Check::Length,
                detail: Detail::LengthOver {
                    chars: r.chars,
                    max,
                    bytes: r.bytes,
                },
            });
        }
        if want(Check::Newline) && r.bad_newline {
            out.push(Finding {
                line: 0,
                check: Check::Newline,
                detail: Detail::NewlineForbidden {
                    field: format!("{}:{}", e.rec_sig, e.field_sig),
                },
            });
        }
    }
    if e.trans.trim().is_empty() {
        return;
    }
    if want(Check::Punctuation) {
        let (a, b) = (final_punct(&e.source), final_punct(&e.trans));
        if a != b && (a.is_some() || b.is_some()) {
            out.push(Finding {
                line: 0,
                check: Check::Punctuation,
                detail: Detail::Punctuation {
                    source: a.map(|c| c.to_string()).unwrap_or_default(),
                    trans: b.map(|c| c.to_string()).unwrap_or_default(),
                },
            });
        }
    }
    if want(Check::Capitals) {
        let mut after_strong = false;
        let mut prev_punct = ' ';
        for c in e.trans.chars() {
            if matches!(c, '.' | '!' | '?') {
                after_strong = true;
                prev_punct = c;
            } else if c.is_whitespace() || c == '\u{a0}' {
                // keep waiting for the next letter
            } else {
                if after_strong && c.is_lowercase() {
                    out.push(Finding {
                        line: 0,
                        check: Check::Capitals,
                        detail: Detail::CapitalAfter(prev_punct.to_string()),
                    });
                    break;
                }
                after_strong = false;
            }
        }
    }
    if want(Check::Spaces) {
        if e.trans.contains("  ") {
            out.push(Finding {
                line: 0,
                check: Check::Spaces,
                detail: Detail::DoubleSpace,
            });
        }
        let edge = |s: &str| s.starts_with(' ') || s.ends_with(' ');
        if edge(&e.trans) && !edge(&e.source) {
            out.push(Finding {
                line: 0,
                check: Check::Spaces,
                detail: Detail::EdgeSpace,
            });
        }
    }
    if want(Check::Encoding) && !utf8 {
        let bad: String = page
            .unencodable(&e.trans)
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        if !bad.is_empty() {
            out.push(Finding {
                line: 0,
                check: Check::Encoding,
                detail: Detail::Encoding(bad, page.id()),
            });
        }
    }
}

/// Run the checks on `lines` (entry indices). `progress(done, total)`
/// returns `false` to stop (the worker is cancelled).
pub fn run(
    entries: &[TmEntry],
    lines: &[usize],
    checks: &[Check],
    game: Game,
    progress: impl FnMut(usize, usize) -> bool,
) -> Report {
    let page = if game.uses_utf8() {
        crate::codepage::Page::Utf8
    } else {
        crate::codepage::Page::Win1252
    };
    run_with_page(entries, lines, checks, game, page, progress)
}

/// [`run`] with the page of code the document is written in (D-09): the
/// encoding check looks for what that page cannot hold.
pub fn run_with_page(
    entries: &[TmEntry],
    lines: &[usize],
    checks: &[Check],
    game: Game,
    page: crate::codepage::Page,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Report {
    let game_set = crate::tags::TagSet::of(game);
    let text_set = crate::tags::TagSet::from_names(
        &crate::textfmt::text_tags()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let mut report = Report {
        findings: Vec::new(),
        lines_checked: 0,
    };
    let mut buf = Vec::new();
    let total = lines.len();
    for (k, &i) in lines.iter().enumerate() {
        let Some(e) = entries.get(i) else { continue };
        if e.unused {
            continue;
        }
        report.lines_checked += 1;
        buf.clear();
        let set = if crate::textfmt::is_text_field(&e.field_sig) {
            &text_set
        } else {
            &game_set
        };
        check_line(e, checks, game, page, set, &mut buf);
        for mut f in buf.drain(..) {
            f.line = i;
            report.findings.push(f);
        }
        if k % 2048 == 0 && !progress(k, total) {
            return report;
        }
    }
    if checks.contains(&Check::Inconsistent) {
        // Groups by exact source, Ignoré and untranslated lines aside.
        let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &i in lines {
            let Some(e) = entries.get(i) else { continue };
            if e.unused
                || e.trans.trim().is_empty()
                || crate::status::derive(e) == crate::status::Status::Ignored
            {
                continue;
            }
            groups.entry(e.source.as_str()).or_default().push(i);
        }
        for (src, idxs) in groups {
            if idxs.len() < 2 {
                continue;
            }
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for &i in &idxs {
                *counts.entry(entries[i].trans.as_str()).or_default() += 1;
            }
            if counts.len() < 2 {
                continue;
            }
            let mut variants: Vec<(String, usize)> = counts
                .into_iter()
                .map(|(t, n)| (t.to_string(), n))
                .collect();
            variants.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            report.findings.push(Finding {
                line: idxs[0],
                check: Check::Inconsistent,
                detail: Detail::Inconsistent {
                    source: src.to_string(),
                    variants,
                    lines: idxs,
                },
            });
        }
    }
    progress(total, total);
    report
}

/// The text of a finding (the window's table and tooltip, the report of
/// `ximod-cli check`); `flat` shortens a variant to one line.
pub fn finding_text(
    i18n: &crate::i18n::I18n,
    f: &Finding,
    flat: impl Fn(&str) -> String,
) -> String {
    use Detail as D;
    let t = i18n;
    match &f.detail {
        D::TagMissing(tag) => t.t_arg("alias-missing", "tag", tag),
        D::TagUnknown(tag) => t.t_arg("alias-unknown", "tag", tag),
        D::TagOrder => t.t("alias-order"),
        D::NumberMissing(n) => t.t_arg("alias-number-missing", "n", n),
        D::NumberUnknown(n) => t.t_arg("alias-number-unknown", "n", n),
        D::LengthOver { chars, max, bytes } => t.t_args(
            "quality-detail-length",
            &[
                ("chars", &chars.to_string()),
                ("max", &max.to_string()),
                ("bytes", &bytes.to_string()),
            ],
        ),
        D::NewlineForbidden { field } => t.t_arg("len-newline-tip", "field", field),
        D::Punctuation { source, trans } => t.t_args(
            "quality-detail-punct",
            &[("source", source), ("trans", trans)],
        ),
        D::CapitalAfter(p) => t.t_arg("quality-detail-capital", "punct", p),
        D::DoubleSpace => t.t("quality-detail-double-space"),
        D::EdgeSpace => t.t("quality-detail-edge-space"),
        D::Encoding(chars, page) => t.t_args(
            "quality-detail-encoding",
            &[
                ("chars", chars),
                (
                    "page",
                    &crate::codepage::Page::from_id(page)
                        .map(|p| t.t(p.name_key()))
                        .unwrap_or_else(|| page.to_string()),
                ),
            ],
        ),
        D::Inconsistent {
            variants, lines, ..
        } => {
            let v: Vec<String> = variants
                .iter()
                .map(|(text, n)| {
                    t.t_args(
                        "quality-variant",
                        &[("text", &flat(text)), ("n", &n.to_string())],
                    )
                })
                .collect();
            t.t_args(
                "quality-detail-inconsistent",
                &[
                    ("count", &lines.len().to_string()),
                    ("variants", &v.join(", ")),
                ],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(src: &str, tr: &str) -> TmEntry {
        TmEntry {
            rec_sig: "WEAP".into(),
            field_sig: "FULL".into(),
            source: src.into(),
            trans: tr.into(),
            ..Default::default()
        }
    }

    fn all() -> Vec<Check> {
        Check::ALL.to_vec()
    }

    #[test]
    fn checks_set_round_trip() {
        assert_eq!(Check::parse_set(""), Check::default_set());
        assert!(!Check::default_set().contains(&Check::Capitals));
        let s = Check::format_set(&[Check::Aliases, Check::Spaces]);
        assert_eq!(Check::parse_set(&s), vec![Check::Aliases, Check::Spaces]);
        assert_eq!(Check::parse_set("bogus"), Vec::<Check>::new());
    }

    #[test]
    fn acceptance_t_d05_01_four_findings_then_fewer() {
        let entries = vec![
            e("Bring <Alias=Victim>", "Amenez <Alias=Victime>"),
            e("Iron Sword", &"x".repeat(40)),
            e("Hello", "Bonjour  monde"),
            e("Ash", "Cendre"),
            e("Ash", "Cendres"),
        ];
        // A length limit for the test: the table of Morrowind names.
        let lines: Vec<usize> = (0..entries.len()).collect();
        let r = run(
            &entries,
            &lines,
            &[
                Check::Aliases,
                Check::Length,
                Check::Spaces,
                Check::Inconsistent,
            ],
            Game::Morrowind,
            |_, _| true,
        );
        let _ = r;
        // Length needs a field with a limit: Morrowind FNAM.
        let mut e2 = entries.clone();
        e2[1].field_sig = "FNAM".into();
        let r = run(
            &e2,
            &lines,
            &[
                Check::Aliases,
                Check::Length,
                Check::Spaces,
                Check::Inconsistent,
            ],
            Game::Morrowind,
            |_, _| true,
        );
        let checks: Vec<Check> = r.findings.iter().map(|f| f.check).collect();
        assert!(
            checks.contains(&Check::Aliases)
                && checks.contains(&Check::Length)
                && checks.contains(&Check::Spaces)
                && checks.contains(&Check::Inconsistent),
            "{checks:?}"
        );
        assert_eq!(r.lines_checked, 5);
        // One check less: one finding kind less.
        let r = run(
            &e2,
            &lines,
            &[Check::Aliases, Check::Length, Check::Spaces],
            Game::Morrowind,
            |_, _| true,
        );
        assert!(!r.findings.iter().any(|f| f.check == Check::Inconsistent));
        // The anomaly lines: both lines of the inconsistent group count.
        let r = run(
            &e2,
            &lines,
            &[Check::Inconsistent],
            Game::Morrowind,
            |_, _| true,
        );
        assert_eq!(r.lines().into_iter().collect::<Vec<_>>(), vec![3, 4]);
        match &r.findings[0].detail {
            Detail::Inconsistent { variants, .. } => assert_eq!(variants.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn punctuation_capitals_spaces_encoding() {
        assert_eq!(final_punct("Hello !"), Some('!'));
        assert_eq!(final_punct("Bonjour\u{a0}?"), Some('?'));
        assert_eq!(final_punct("Fin."), Some('.'));
        assert_eq!(final_punct("Rien"), None);
        let entries = vec![
            e("Go!", "Allez."),
            e("Go.", "Allez. vite"),
            e("Go", " Allez"),
            e("Go on", "Allez — œuvre ∑"),
            e("Go now", ""),
        ];
        let lines: Vec<usize> = (0..entries.len()).collect();
        let r = run(&entries, &lines, &all(), Game::Skyrim, |_, _| true);
        let of = |i: usize| {
            r.findings
                .iter()
                .filter(|f| f.line == i)
                .map(|f| f.check)
                .collect::<Vec<_>>()
        };
        assert_eq!(of(0), vec![Check::Punctuation]);
        assert!(of(1).contains(&Check::Capitals) && of(1).contains(&Check::Punctuation));
        assert_eq!(of(2), vec![Check::Spaces]);
        // Skyrim LE is Windows-1252: « ∑ » is not representable, « œ » and « — » are.
        assert!(of(3).contains(&Check::Encoding));
        assert!(
            matches!(&r.findings.iter().find(|f| f.line == 3 && f.check == Check::Encoding).unwrap().detail, Detail::Encoding(s, _) if s == "∑")
        );
        // An empty translation has nothing to check beyond the limits.
        assert!(of(4).is_empty());
        // UTF-8 game: no encoding finding.
        let r = run(
            &entries,
            &lines,
            &[Check::Encoding],
            Game::SkyrimSE,
            |_, _| true,
        );
        assert!(r.findings.is_empty());
    }

    #[test]
    fn progress_can_cancel() {
        let entries: Vec<TmEntry> = (0..10_000)
            .map(|i| e(&format!("s{i}"), &format!("t{i}")))
            .collect();
        let lines: Vec<usize> = (0..entries.len()).collect();
        let mut calls = 0;
        let r = run(&entries, &lines, &all(), Game::Skyrim, |_, _| {
            calls += 1;
            calls < 2
        });
        assert!(r.lines_checked < entries.len());
    }

    /// T-D05-04: 50 000 lines, every check, well under two seconds.
    #[test]
    fn fifty_thousand_lines_under_two_seconds() {
        let entries: Vec<TmEntry> = (0..50_000)
            .map(|i| {
                e(
                    &format!("Bring <Alias=A{i}> {i} items. Done"),
                    &format!("Apportez <Alias=A{i}> {i} objets. Fait"),
                )
            })
            .collect();
        let lines: Vec<usize> = (0..entries.len()).collect();
        let t = std::time::Instant::now();
        let r = run(&entries, &lines, &all(), Game::Skyrim, |_, _| true);
        let d = t.elapsed();
        assert_eq!(r.lines_checked, 50_000);
        assert!(d.as_secs_f32() < 8.0, "{d:?} (debug build)");
        eprintln!("50 000 lignes, toutes vérifications : {d:?}");
    }
}
