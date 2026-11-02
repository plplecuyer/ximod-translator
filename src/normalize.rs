//! Normalized matching of the dictionaries (B-19). Two texts that differ
//! only by their numbers, their punctuation, their spacing or their case
//! share one **form** ([`form`]); a translation found through it is adapted
//! to the new source ([`transfer`]) — numbers, final punctuation and the
//! case of the first letter carried over — and the [`Reports`] say what was
//! carried. Pure functions over strings; the engine tags are compared as
//! they are (a different tag is another line).

use unicode_normalization::UnicodeNormalization;

use crate::tags::{self, TagSet};

/// What a transfer carried over (R-B19-03), and the number of distinct
/// results when several sources share the form (B-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Reports {
    pub bits: u8,
    /// Distinct adapted translations of the form (≥ 2: ambiguous).
    pub amb: u8,
}

impl Reports {
    pub const NUMBERS: u8 = 1;
    pub const PUNCT: u8 = 2;
    pub const CASE: u8 = 4;
    pub const FAILED: u8 = 8;

    pub fn has(self, bit: u8) -> bool {
        self.bits & bit != 0
    }

    /// The codes of the provenance: `n`, `p`, `c`, `!`.
    pub fn codes(self) -> String {
        let mut s = String::new();
        for (bit, c) in [
            (Self::NUMBERS, 'n'),
            (Self::PUNCT, 'p'),
            (Self::CASE, 'c'),
            (Self::FAILED, '!'),
        ] {
            if self.has(bit) {
                s.push(c);
            }
        }
        s
    }

    pub fn from_codes(s: &str) -> Reports {
        let mut r = Reports::default();
        for c in s.chars() {
            r.bits |= match c {
                'n' => Self::NUMBERS,
                'p' => Self::PUNCT,
                'c' => Self::CASE,
                '!' => Self::FAILED,
                _ => 0,
            };
        }
        r
    }

    /// The i18n keys of the reports, in order (long words).
    pub fn keys(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.has(Self::NUMBERS) {
            out.push("prov-report-numbers");
        }
        if self.has(Self::FAILED) {
            out.push("prov-report-numbers-failed");
        }
        if self.has(Self::PUNCT) {
            out.push("prov-report-punct");
        }
        if self.has(Self::CASE) {
            out.push("prov-report-case");
        }
        out
    }
}

/// One number of a text: its byte span and its value (sign and digits,
/// separators aside, so « 1,000 » and « 1 000 » are one value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Num {
    pub start: usize,
    pub end: usize,
    pub value: String,
}

/// The separators a number may hold between its digits.
fn is_sep(c: char) -> bool {
    matches!(c, '.' | ',' | '\u{a0}' | '\u{202f}' | '\'')
}

fn is_sign(c: char) -> bool {
    matches!(c, '+' | '-' | '\u{2212}')
}

/// The numbers of a text outside its tags (R-B19-01 c): a run of digits
/// with its decimal and thousands separators, a sign attached when it does
/// not follow a letter or a digit.
pub fn numbers(text: &str) -> Vec<Num> {
    let spans = tags::tags(text, &TagSet::any());
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if let Some(t) = spans.iter().find(|t| t.start == pos) {
            while i < chars.len() && chars[i].0 < t.end {
                i += 1;
            }
            continue;
        }
        let signed = is_sign(c)
            && chars.get(i + 1).is_some_and(|(_, d)| d.is_ascii_digit())
            && (i == 0 || !chars[i - 1].1.is_alphanumeric());
        if c.is_ascii_digit() || signed {
            let start = pos;
            let mut value = String::new();
            if signed {
                value.push('-');
                if c == '+' {
                    value.pop();
                    value.push('+');
                }
                i += 1;
            }
            while i < chars.len() {
                let d = chars[i].1;
                if d.is_ascii_digit() {
                    value.push(d);
                    i += 1;
                } else if is_sep(d) && chars.get(i + 1).is_some_and(|(_, n)| n.is_ascii_digit()) {
                    i += 1;
                } else {
                    break;
                }
            }
            let end = chars.get(i).map(|(p, _)| *p).unwrap_or(text.len());
            out.push(Num { start, end, value });
            continue;
        }
        i += 1;
    }
    out
}

/// A combining mark that NFKC leaves alone (Indic vowel signs and the
/// like): part of the word, never punctuation.
fn is_mark(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x0610..=0x061A
        | 0x064B..=0x065F | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0E31 | 0x0E34..=0x0E3A
        | 0x0E47..=0x0E4E | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
        | 0x302A..=0x302F | 0x3099..=0x309A | 0xFE20..=0xFE2F)
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{02bc}')
}

/// The sentence punctuation the form ignores; the other marks (brackets,
/// quotes, symbols) frame the text and are kept, every quotation mark as
/// `"`. `None` for a mark the form removes.
fn frame(c: char) -> Option<char> {
    if c.is_control()
        || matches!(
            c,
            '.' | ',' | ';' | ':' | '!' | '?' | '…' | '-' | '–' | '—' | '¡' | '¿' | '、' | '·'
        )
        || WIDE.iter().any(|(w, _)| *w == c)
        || is_apostrophe(c)
    {
        return None;
    }
    if matches!(
        c,
        '"' | '«' | '»' | '“' | '”' | '„' | '‟' | '‹' | '›' | '「' | '」' | '『' | '』'
    ) {
        return Some('"');
    }
    Some(c)
}

/// The normalized form of a text (R-B19-01): tags kept as they are, case
/// folded (NFKC, lower case), each number replaced by `#`, the sentence
/// punctuation removed (an apostrophe inside a word stays) while brackets,
/// quotes and symbols are kept, spaces reduced.
/// `None` when the form has no letter: a text made of numbers or of
/// punctuation is not a sentence and never matches.
pub fn form(text: &str) -> Option<String> {
    let spans = tags::tags(text, &TagSet::any());
    let mut out = String::with_capacity(text.len());
    let mut letter = false;
    let mut pos = 0;
    let mut push_plain = |seg: &str, out: &mut String| {
        if seg.is_empty() {
            return;
        }
        let seg: String = seg.nfkc().collect();
        let nums = numbers(&seg);
        let mut k = 0;
        let chars: Vec<(usize, char)> = seg.char_indices().collect();
        let mut j = 0;
        while j < chars.len() {
            let (p, c) = chars[j];
            if let Some(n) = nums.get(k)
                && n.start == p
            {
                out.push('#');
                while j < chars.len() && chars[j].0 < n.end {
                    j += 1;
                }
                k += 1;
                continue;
            }
            if c.is_alphanumeric() || is_mark(c) {
                if c.is_alphabetic() {
                    letter = true;
                }
                out.extend(c.to_lowercase());
            } else if c.is_whitespace() {
                out.push(' ');
            } else if is_apostrophe(c)
                && j > 0
                && chars[j - 1].1.is_alphanumeric()
                && chars.get(j + 1).is_some_and(|(_, n)| n.is_alphanumeric())
            {
                out.push('\'');
            } else if let Some(k) = frame(c) {
                // Brackets, quotes, symbols: kept, a translation without
                // them would lose them (« [ BACK ] » is not « Back »).
                out.push(' ');
                out.push(k);
                out.push(' ');
            } else {
                // Sentence punctuation, control: removed; it separates.
                out.push(' ');
            }
            j += 1;
        }
    };
    for t in &spans {
        push_plain(&text[pos..t.start], &mut out);
        out.push(' ');
        out.push_str(&t.text);
        out.push(' ');
        pos = t.end;
    }
    push_plain(&text[pos..], &mut out);
    if !letter {
        return None;
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// The full-width marks of Chinese and Japanese, with their ASCII
/// counterpart.
const WIDE: [(char, char); 6] = [
    ('。', '.'),
    ('！', '!'),
    ('？', '?'),
    ('，', ','),
    ('：', ':'),
    ('；', ';'),
];

fn is_final(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | ':' | ';' | ',') || WIDE.iter().any(|(w, _)| *w == c)
}

/// The final punctuation of a text: its trailing run of `. ! ? … : ; ,`
/// (or their full-width forms).
fn final_punct(text: &str) -> &str {
    let t = text.trim_end();
    let cut = t
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_final(*c))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(t.len());
    &t[cut..]
}

/// The punctuation in ASCII, to compare a full-width ending with its
/// source.
fn narrow(p: &str) -> String {
    p.chars()
        .map(|c| {
            WIDE.iter()
                .find(|(w, _)| *w == c)
                .map(|(_, a)| *a)
                .unwrap_or(c)
        })
        .collect()
}

/// Whether a character belongs to a script written without spaces and with
/// full-width marks (CJK ideographs, kana, hangul, full-width forms).
fn is_wide_script(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// The new final punctuation as the translation writes it: full width
/// after a CJK text.
fn widened(p: &str, after: Option<char>) -> String {
    if !after.is_some_and(is_wide_script) {
        return p.to_string();
    }
    p.chars()
        .map(|c| {
            WIDE.iter()
                .find(|(_, a)| *a == c)
                .map(|(w, _)| *w)
                .unwrap_or(c)
        })
        .collect()
}

/// The first letter of a text outside its tags: its byte position.
fn first_letter(text: &str) -> Option<(usize, char)> {
    let spans = tags::tags(text, &TagSet::any());
    text.char_indices()
        .find(|(p, c)| c.is_alphabetic() && !spans.iter().any(|t| *p >= t.start && *p < t.end))
}

/// Whether a text is written in capitals: two letters at least, none in
/// lower case (outside the tags).
fn all_caps(text: &str) -> bool {
    let spans = tags::tags(text, &TagSet::any());
    let letters: Vec<char> = text
        .char_indices()
        .filter(|(p, c)| c.is_alphabetic() && !spans.iter().any(|t| *p >= t.start && *p < t.end))
        .map(|(_, c)| c)
        .collect();
    letters.len() >= 2
        && letters.iter().all(|c| !c.is_lowercase())
        && letters.iter().any(|c| c.is_uppercase())
}

/// The separators of a number's text, in order.
fn seps(s: &str) -> Vec<char> {
    s.chars().filter(|c| is_sep(*c)).collect()
}

/// The new number written like the translation wrote the old one: its
/// separators mapped in order (« 1.5 » translated « 1,5 », « 2.5 » gives
/// « 2,5 »).
fn styled(new: &str, old_src: &str, old_trans: &str) -> String {
    if old_src == old_trans {
        return new.to_string();
    }
    let (a, b) = (seps(old_src), seps(old_trans));
    if a.len() != b.len() || a.is_empty() {
        return new.to_string();
    }
    new.chars()
        .map(|c| match a.iter().position(|x| *x == c) {
            Some(k) if is_sep(c) => b[k],
            _ => c,
        })
        .collect()
}

/// Adapt the translation `found_trans` of `found_src` to `new_src`, which
/// shares its form (R-B19-03): numbers, final punctuation, case of the
/// first letter. Nothing else is touched.
pub fn transfer(found_src: &str, found_trans: &str, new_src: &str) -> (String, Reports) {
    let mut r = Reports::default();
    let mut out = found_trans.to_string();
    // (a) Numbers.
    let f = numbers(found_src);
    let n = numbers(new_src);
    let differ = f.len() != n.len() || f.iter().zip(&n).any(|(a, b)| a.value != b.value);
    if differ {
        let t = numbers(&out);
        let mut used = vec![false; f.len()];
        let mut plan: Vec<(usize, usize, String)> = Vec::new();
        for tn in &t {
            if let Some(k) = (0..f.len()).find(|&k| !used[k] && f[k].value == tn.value) {
                used[k] = true;
                if let Some(nn) = n.get(k)
                    && nn.value != f[k].value
                {
                    let text = styled(
                        &new_src[nn.start..nn.end],
                        &found_src[f[k].start..f[k].end],
                        &out[tn.start..tn.end],
                    );
                    plan.push((tn.start, tn.end, text));
                }
            }
        }
        if f.len() == n.len() && used.iter().all(|u| *u) {
            for (s, e, text) in plan.into_iter().rev() {
                out.replace_range(s..e, &text);
            }
            r.bits |= Reports::NUMBERS;
        } else {
            r.bits |= Reports::FAILED;
        }
    }
    // (b) Final punctuation.
    let (fp, np) = (final_punct(found_src), final_punct(new_src));
    if fp != np {
        let trimmed = out.trim_end().to_string();
        let tp = final_punct(&trimmed);
        let ok = if fp.is_empty() {
            tp.is_empty()
        } else {
            narrow(tp) == narrow(fp)
        };
        if ok {
            let body = &trimmed[..trimmed.len() - tp.len()];
            let core = body.trim_end_matches([' ', '\u{a0}', '\u{202f}']);
            let space = &body[core.len()..];
            let keep_space = np.starts_with(['!', '?', ';', ':']) && !space.is_empty();
            let np = widened(&narrow(np), core.chars().last());
            let tail = &out[trimmed.len()..];
            out = format!("{core}{}{np}{tail}", if keep_space { space } else { "" });
            r.bits |= Reports::PUNCT;
        }
    }
    // (c) Case: a text in capitals against one that is not (« AMBER »
    // and « Amber »), the whole translation; else the first letter.
    let (fc, nc, tc) = (all_caps(found_src), all_caps(new_src), all_caps(&out));
    if fc != nc && (tc == fc) && first_letter(&out).is_some() {
        let next = if nc {
            out.to_uppercase()
        } else {
            let low = out.to_lowercase();
            match (first_letter(new_src), first_letter(&low)) {
                (Some((_, b)), Some((p, c))) if b.is_uppercase() => {
                    let mut t = low.clone();
                    t.replace_range(p..p + c.len_utf8(), &c.to_uppercase().collect::<String>());
                    t
                }
                _ => low,
            }
        };
        if next != out {
            out = next;
            r.bits |= Reports::CASE;
        }
    } else if let (Some((_, a)), Some((_, b))) = (first_letter(found_src), first_letter(new_src))
        && a != b
        && a.to_lowercase().eq(b.to_lowercase())
        && let Some((p, c)) = first_letter(&out)
    {
        let want: String = if b.is_uppercase() {
            c.to_uppercase().collect()
        } else {
            c.to_lowercase().collect()
        };
        if want != c.to_string() {
            out.replace_range(p..p + c.len_utf8(), &want);
            r.bits |= Reports::CASE;
        }
    }
    (out, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        assert_eq!(
            form("You have 3 days left!"),
            Some("you have # days left".into())
        );
        assert_eq!(form("you have 7 days left"), form("You have 5 days left."));
        assert_ne!(
            form("You have 3 days and 2 nights"),
            form("You have 3 days")
        );
        // Signs, separators, percents.
        assert_eq!(form("+5 Health"), form("-12 health"));
        assert_eq!(form("Costs 1,000 gold"), form("Costs 25 gold."));
        assert_eq!(form("Damage +25%"), Some("damage # %".into()));
        assert_eq!(form("Damage +25 %"), form("Damage -3%"));
        // An apostrophe inside a word stays; quotes are one mark.
        assert_eq!(form("Don't \"panic\""), Some("don't \" panic \"".into()));
        assert_eq!(form("Don't « panic »!"), form("don't \"panic\""));
        // Brackets frame the text: « [ BACK ] » is not « Back ».
        assert_ne!(form("[ BACK ]"), form("Back"));
        assert_eq!(form("[BACK]"), form("[ Back ]"));
        // Sentence punctuation inside goes.
        assert_eq!(form("Well, then: go"), form("Well then go!"));
        // Tags as they are, case and all.
        assert_eq!(
            form("Kill <Alias=Target> now!"),
            Some("kill <Alias=Target> now".into())
        );
        assert_ne!(form("Kill <Alias=Target>"), form("Kill <Alias=Victim>"));
        // NFKC: full-width letters and digits.
        assert_eq!(form("ＡＢＣ　１２"), Some("abc #".into()));
        // No letter: no form.
        assert_eq!(form("42"), None);
        assert_eq!(form("..."), None);
        assert_eq!(form(""), None);
    }

    #[test]
    fn numbers_of_a_text() {
        let n = numbers("Take 3 of 1,000 (+5) x-2 <Global=G1>");
        let v: Vec<&str> = n.iter().map(|x| x.value.as_str()).collect();
        assert_eq!(v, ["3", "1000", "+5", "2"]);
    }

    #[test]
    fn transfers() {
        let nnbsp = '\u{202f}';
        // T-B19-01: numbers and punctuation.
        let (t, r) = transfer(
            "You have 3 days left!",
            &format!("Il vous reste 3 jours{nnbsp}!"),
            "You have 5 days left.",
        );
        assert_eq!(t, "Il vous reste 5 jours.");
        assert_eq!(r.codes(), "np");
        // The other way: the narrow space is not invented, a kept one is.
        let (t, _) = transfer(
            "You have 3 days left.",
            "Il vous reste 3 jours.",
            "You have 4 days left!",
        );
        assert_eq!(t, "Il vous reste 4 jours!");
        // T-B19-02: case and numbers, no punctuation.
        let (t, r) = transfer(
            "You have 3 days left!",
            &format!("Il vous reste 3 jours{nnbsp}!"),
            "you have 7 days left",
        );
        assert_eq!(t, "il vous reste 7 jours");
        assert_eq!(r.codes(), "npc");
        // T-B19-03: numbers that cannot be carried.
        let (t, r) = transfer(
            "You have 3 days left.",
            "Il vous reste trois jours.",
            "You have 5 days left.",
        );
        assert_eq!(t, "Il vous reste trois jours.");
        assert_eq!(r.codes(), "!");
        // Reordered numbers follow their value; a decimal comma is kept.
        let (t, r) = transfer(
            "Deals 10 damage for 2.5 seconds",
            "Pendant 2,5 secondes, inflige 10 points de dégâts",
            "Deals 15 damage for 3.5 seconds",
        );
        assert_eq!(t, "Pendant 3,5 secondes, inflige 15 points de dégâts");
        assert_eq!(r.codes(), "n");
        // Same numbers, other spacing only: nothing to report.
        let (t, r) = transfer("Iron  Sword", "Épée en fer", "Iron Sword");
        assert_eq!((t.as_str(), r.bits), ("Épée en fer", 0));
        // The case of a first letter behind a tag.
        let (t, r) = transfer(
            "<font color='#FF0000'>Warning",
            "<font color='#FF0000'>Attention",
            "<font color='#FF0000'>warning",
        );
        assert_eq!(t, "<font color='#FF0000'>attention");
        assert_eq!(r.codes(), "c");
        // A text in capitals.
        let (t, r) = transfer("AMBER", "AMBRE", "Amber");
        assert_eq!((t.as_str(), r.codes().as_str()), ("Ambre", "c"));
        let (t, _) = transfer("Iron Ingot", "Lingot de fer", "IRON INGOT");
        assert_eq!(t, "LINGOT DE FER");
        // Chinese: the full-width marks.
        let (t, _) = transfer("Main Quest", "主線任務", "Main Quest.");
        assert_eq!(t, "主線任務。");
        let (t, r) = transfer("Done!", "完成！", "Done?");
        assert_eq!((t.as_str(), r.codes().as_str()), ("完成？", "p"));
    }

    #[test]
    fn report_codes_round_trip() {
        let r = Reports {
            bits: Reports::NUMBERS | Reports::FAILED | Reports::CASE,
            amb: 0,
        };
        assert_eq!(r.codes(), "nc!");
        assert_eq!(Reports::from_codes("nc!"), r);
    }
}
