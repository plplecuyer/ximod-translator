//! The display order of a text written partly or wholly from right to left
//! (E-08, D-119). The text stays in logical order — the order typed, the
//! order written to every file; only its display is computed here: the
//! bidirectional algorithm (unicode-bidi) resolves the levels of a
//! paragraph, then each line, once cut, gives its runs in visual order. The
//! shaping of each run (joined Arabic letters) is left to egui.
//!
//! Tags, parameters and numbers are isolates: never turned round, placed as
//! one block at their rank in the sentence.

use std::ops::Range;

use unicode_bidi::{BidiInfo, Level};

use crate::tags::{self, TagSet};

/// A run of a line: a byte range of the text and its direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub range: Range<usize>,
    pub rtl: bool,
}

/// A character of a script written from right to left (Hebrew, Arabic,
/// Syriac, Thaana, N'Ko, Samaritan, Mandaic, Adlam… and their
/// presentation forms): the quick test that keeps every other text off the
/// bidirectional algorithm.
pub fn is_rtl_char(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF
        | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

/// The text holds a right-to-left character.
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_rtl_char)
}

/// Whether a language (ISO 639-3) is written from right to left
/// (`Languages.json`, `rtl`).
pub fn language_is_rtl(iso3: &str) -> bool {
    crate::data::languages().is_rtl(iso3)
}

/// The display needs the algorithm: a text with a right-to-left
/// character, or any text of a right-to-left column (its punctuation goes
/// to the left).
pub fn needed(text: &str, base_rtl: bool) -> bool {
    (base_rtl && !text.is_empty()) || has_rtl(text)
}

/// The direction of a text of the interface: its first strong character.
pub fn first_strong_is_rtl(text: &str) -> bool {
    for c in text.chars() {
        if is_rtl_char(c) {
            return true;
        }
        if c.is_alphabetic() {
            return false;
        }
    }
    false
}

/// The isolates of a text: its tags (any), its parameters and its
/// numbers, sorted, never overlapping.
pub fn isolates(text: &str) -> Vec<Range<usize>> {
    let set = TagSet::any();
    let mut out: Vec<Range<usize>> = tags::tags(text, &set)
        .into_iter()
        .map(|s| s.start..s.end)
        .collect();
    out.extend(
        tags::numbers(text, &set)
            .into_iter()
            .map(|s| s.start..s.end),
    );
    out.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(out.len());
    for r in out {
        match merged.last_mut() {
            Some(last) if r.start < last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// Left-to-right isolate and its end (UAX #9).
const LRI: char = '\u{2066}';
const PDI: char = '\u{2069}';

/// The runs of each line of a paragraph (no line feed in it), in visual
/// order: `lines` cut the paragraph (byte ranges, in order, covering it),
/// `base_rtl` is the direction of the paragraph, `isolates` the spans kept
/// whole. The ranges are those of `text`.
pub fn visual_lines(
    text: &str,
    base_rtl: bool,
    isolates: &[Range<usize>],
    lines: &[Range<usize>],
) -> Vec<Vec<Run>> {
    if !needed(text, base_rtl) {
        return lines
            .iter()
            .map(|l| {
                if l.is_empty() {
                    Vec::new()
                } else {
                    vec![Run {
                        range: l.clone(),
                        rtl: false,
                    }]
                }
            })
            .collect();
    }
    // The text with each isolate between LRI and PDI; `orig[i]` is the
    // offset in `text` of the byte `i` of the marked text (`None` for a
    // mark).
    let mark_len = LRI.len_utf8();
    let mut marked = String::with_capacity(text.len() + isolates.len() * 2 * mark_len);
    let mut orig: Vec<Option<usize>> = Vec::with_capacity(marked.capacity());
    let mut opens = isolates.iter().map(|r| r.start).peekable();
    let mut closes = isolates.iter().map(|r| r.end).peekable();
    let push_mark = |m: char, marked: &mut String, orig: &mut Vec<Option<usize>>| {
        marked.push(m);
        orig.extend(std::iter::repeat_n(None, m.len_utf8()));
    };
    for (i, c) in text.char_indices() {
        while closes.peek() == Some(&i) {
            closes.next();
            push_mark(PDI, &mut marked, &mut orig);
        }
        while opens.peek() == Some(&i) {
            opens.next();
            push_mark(LRI, &mut marked, &mut orig);
        }
        marked.push(c);
        orig.extend((0..c.len_utf8()).map(|k| Some(i + k)));
    }
    while closes.next().is_some() {
        push_mark(PDI, &mut marked, &mut orig);
    }
    // The offset in the marked text where a line starting (or ending) at
    // `x` in `text` begins: after the PDI that close at `x`, before the
    // LRI that open there.
    let pos = |x: usize| -> usize {
        if x >= text.len() {
            return marked.len();
        }
        let first = orig
            .iter()
            .position(|o| *o == Some(x))
            .unwrap_or(marked.len());
        // Step back over the LRI that open at `x`.
        let mut p = first;
        while p >= mark_len && orig[p - 1].is_none() && marked[p - mark_len..p].starts_with(LRI) {
            p -= mark_len;
        }
        p
    };
    let base = if base_rtl { Level::rtl() } else { Level::ltr() };
    let info = BidiInfo::new(&marked, Some(base));
    let Some(para) = info.paragraphs.first() else {
        return lines.iter().map(|_| Vec::new()).collect();
    };
    lines
        .iter()
        .map(|l| {
            let (a, b) = (pos(l.start), pos(l.end));
            if a >= b {
                return Vec::new();
            }
            let (levels, runs) = info.visual_runs(para, a..b);
            runs.into_iter()
                .filter_map(|r| {
                    let rtl = levels[r.start].is_rtl();
                    // Every byte of a character maps to its own offset.
                    let start = *orig[r.clone()].iter().flatten().next()?;
                    let end = *orig[r].iter().flatten().last()? + 1;
                    Some(Run {
                        range: start..end,
                        rtl,
                    })
                })
                .collect()
        })
        .collect()
}

/// [`visual_lines`] for a single line, with the text's own isolates.
pub fn visual_line(text: &str, base_rtl: bool) -> Vec<Run> {
    let whole = 0..text.len();
    visual_lines(
        text,
        base_rtl,
        &isolates(text),
        std::slice::from_ref(&whole),
    )
    .pop()
    .unwrap_or_default()
}

/// The mirror of a bracket or a quote in a right-to-left run.
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        _ => c,
    }
}

/// The pieces of a run, from the left, as the shaping must receive them.
/// egui shapes each piece on its own and turns round only the letters of
/// a right-to-left script: in a right-to-left run, each word of that script
/// is a piece kept in logical order, and the pieces — words, spaces,
/// punctuation (reversed and mirrored) — are put from the left.
pub fn pieces(text: &str, run: &Run) -> Vec<String> {
    pieces_at(text, run).into_iter().map(|p| p.text).collect()
}

/// A piece of a run as displayed, with the range of `text` it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub range: Range<usize>,
    pub text: String,
}

/// [`pieces`] with their ranges (a highlight of the concordance follows
/// them).
pub fn pieces_at(text: &str, run: &Run) -> Vec<Piece> {
    let s = &text[run.range.clone()];
    if !run.rtl {
        return vec![Piece {
            range: run.range.clone(),
            text: s.to_string(),
        }];
    }
    // Tokens: a word of the right-to-left script (with its marks), or a
    // run of anything else.
    let mut tokens: Vec<(bool, Range<usize>)> = Vec::new();
    for (i, c) in s.char_indices() {
        let at = run.range.start + i;
        let word = is_rtl_char(c) || (c.is_alphanumeric() && !c.is_ascii());
        match tokens.last_mut() {
            Some((w, r)) if *w == word => r.end = at + c.len_utf8(),
            _ => tokens.push((word, at..at + c.len_utf8())),
        }
    }
    tokens
        .into_iter()
        .rev()
        .map(|(word, r)| Piece {
            text: if word {
                text[r.clone()].to_string()
            } else {
                text[r.clone()].chars().rev().map(mirror).collect()
            },
            range: r,
        })
        .collect()
}

/// The text of a line as displayed, its runs put end to end (a right-to-left
/// run is kept in logical order: its shaping turns it round). For the tests
/// and the diagnostics.
pub fn display_runs(text: &str, runs: &[Run]) -> Vec<String> {
    runs.iter()
        .map(|r| text[r.range.clone()].to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quick_test() {
        assert!(!has_rtl("Iron Sword, 250 000 or"));
        assert!(has_rtl("سيف حديدي"));
        assert!(has_rtl("חרב ברזל"));
        assert!(!needed("Iron Sword", false));
        assert!(needed("Iron Sword", true));
        assert!(needed("سيف", false));
        assert!(first_strong_is_rtl("« ملف »"));
        assert!(!first_strong_is_rtl("123 File"));
    }

    #[test]
    fn a_latin_text_is_one_run() {
        let r = visual_line("Iron Sword", false);
        assert_eq!(
            r,
            vec![Run {
                range: 0..10,
                rtl: false
            }]
        );
    }

    /// T-E08-02: « اضغط <Alias=Player> 25 مرة » — the tag and the number
    /// are neither turned round nor moved out of their rank.
    #[test]
    fn acceptance_t_e08_02_tag_and_number_kept() {
        let t = "اضغط <Alias=Player> 25 مرة";
        let runs = visual_line(t, true);
        let shown = display_runs(t, &runs);
        // Visual order, from the left: the last word, the number, the tag,
        // the first word (each Arabic run shaped right to left by egui).
        let words: Vec<&str> = shown
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(words, ["مرة", "25", "<Alias=Player>", "اضغط"]);
        let tag = runs
            .iter()
            .find(|r| t[r.range.clone()].contains("Alias"))
            .unwrap();
        assert_eq!(&t[tag.range.clone()], "<Alias=Player>");
        assert!(!tag.rtl);
        // The same in Hebrew.
        let h = "לחץ <Alias=Player> 25 פעמים";
        let shown = display_runs(h, &visual_line(h, true));
        let words: Vec<&str> = shown
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(words, ["פעמים", "25", "<Alias=Player>", "לחץ"]);
    }

    #[test]
    fn an_arabic_word_in_a_latin_sentence() {
        let t = "The word سيف means sword";
        let runs = visual_line(t, false);
        let shown = display_runs(t, &runs);
        assert_eq!(shown.len(), 3);
        assert!(runs[1].rtl && !runs[0].rtl && !runs[2].rtl);
        assert_eq!(shown[1].trim(), "سيف");
    }

    #[test]
    fn the_lines_of_a_paragraph_keep_their_words() {
        // Two lines cut after the second word: each line reorders its own
        // words, the first line holding the first words of the sentence.
        let t = "واحد اثنان ثلاثة أربعة";
        let cut = t.find("ثلاثة").unwrap();
        let lines = visual_lines(t, true, &[], &[0..cut, cut..t.len()]);
        assert_eq!(lines.len(), 2);
        let first: String = display_runs(t, &lines[0]).concat();
        assert!(first.contains("واحد") && first.contains("اثنان"));
        let second: String = display_runs(t, &lines[1]).concat();
        assert!(second.contains("ثلاثة") && second.contains("أربعة"));
    }

    #[test]
    fn the_isolates_are_the_tags_and_numbers() {
        let t = "<Alias=A> 25 x {0}";
        let iso = isolates(t);
        assert_eq!(&t[iso[0].clone()], "<Alias=A>");
        assert_eq!(&t[iso[1].clone()], "25");
    }

    #[test]
    fn the_languages_table_gives_the_direction() {
        for l in ["ara", "fas", "urd", "heb", "yid"] {
            assert!(language_is_rtl(l), "{l}");
        }
        for l in ["fra", "eng", "zho", "rus"] {
            assert!(!language_is_rtl(l), "{l}");
        }
    }
}

#[cfg(test)]
mod pieces_tests {
    use super::*;

    /// A right-to-left run is given to the shaping word by word, from the
    /// left, the brackets mirrored.
    #[test]
    fn a_run_in_pieces() {
        let t = "فتح تعاويذ الخبراء (250,000 ذهب)";
        let runs = visual_line(t, true);
        let shown: Vec<String> = runs.iter().flat_map(|r| pieces(t, r)).collect();
        assert_eq!(
            shown,
            [
                "(",
                "ذهب",
                " ",
                "250,000",
                ") ",
                "الخبراء",
                " ",
                "تعاويذ",
                " ",
                "فتح"
            ]
        );
    }
}

#[cfg(test)]
mod pieces_at_tests {
    use super::*;

    /// Each piece knows where it comes from: the concordance highlights the
    /// right word.
    #[test]
    fn the_pieces_keep_their_ranges() {
        let t = "يُلحق <mag> من الضرر";
        for run in visual_line(t, true) {
            for p in pieces_at(t, &run) {
                let src = &t[p.range.clone()];
                if run.rtl && src.chars().any(is_rtl_char) {
                    assert_eq!(p.text, src);
                } else {
                    let back: String = p.text.chars().rev().map(mirror).collect();
                    assert!(p.text == src || back == src, "{src:?} / {:?}", p.text);
                }
            }
        }
    }
}
