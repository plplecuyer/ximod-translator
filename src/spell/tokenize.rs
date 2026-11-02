//! Word tokenizer for spell-checking translated strings.
//!
//! Game strings are *not* plain prose: they are peppered with engine tags
//! (`<Alias=Player>`, `<mag>`, `<global=…>`), formatting placeholders (`%s`,
//! `%.0f`, `%1`) and identifier-like tokens (`Health_Regen`, `Attack2`). Feeding
//! those to Hunspell would drown the real spelling issues in noise, so this
//! tokenizer strips them and yields only natural-language words, each with its
//! byte span in the original string (for later highlighting / replacement).
//!
//! A "word" is a run of alphabetic characters that may contain internal
//! apostrophes (`'` / `’`) and hyphens — kept so elisions (`l'épée`) and
//! compounds (`peut-être`) survive to the checker, which knows how to split
//! them. Words shorter than two letters, or glued to a digit/underscore (so part
//! of an identifier), are dropped.

/// A word found in a string, with its byte offsets `[start, end)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub word: String,
    pub start: usize,
    pub end: usize,
}

/// True for the connector characters allowed *inside* a word.
fn is_connector(c: char) -> bool {
    c == '\'' || c == '\u{2019}' || c == '-'
}

/// True for a character that starts/continues a bare word.
fn is_word_alpha(c: char) -> bool {
    c.is_alphabetic()
}

/// Extract the natural-language words of `text`, skipping engine tags,
/// placeholders and identifiers.
pub fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    // Current word buffer and the byte offset of its first char.
    let mut buf = String::new();
    let mut buf_start = 0usize;
    let mut buf_end = 0usize;
    // True when the char immediately preceding the current run was a digit or
    // underscore — i.e. the run is part of an identifier (`re_Attack`).
    let mut run_is_ident = false;
    // When inside a bracketed span, the closer we are waiting for.
    let mut skip_until: Option<char> = None;
    // The last char we saw (to detect digit/underscore adjacency).
    let mut prev: Option<char> = None;

    let mut chars = text.char_indices().peekable();

    // Flush the current buffer as a token. `terminator` is the char that ended
    // the run (None at end-of-string).
    macro_rules! flush {
        ($terminator:expr) => {{
            if !buf.is_empty() {
                // Trim connector chars from both ends, adjusting the span.
                let trimmed_start = buf.chars().take_while(|c| is_connector(*c)).count();
                let trimmed_end = buf.chars().rev().take_while(|c| is_connector(*c)).count();
                let core: String = buf
                    .chars()
                    .skip(trimmed_start)
                    .take(buf.chars().count() - trimmed_start - trimmed_end)
                    .collect();
                let start = buf_start
                    + buf.chars().take(trimmed_start).map(|c| c.len_utf8()).sum::<usize>();
                let end = buf_end
                    - buf.chars().rev().take(trimmed_end).map(|c| c.len_utf8()).sum::<usize>();
                let term: Option<char> = $terminator;
                let ident_after = matches!(term, Some(c) if c.is_ascii_digit() || c == '_');
                if !run_is_ident
                    && !ident_after
                    && core.chars().filter(|c| is_word_alpha(*c)).count() >= 2
                {
                    out.push(Token { word: core, start, end });
                }
                buf.clear();
            }
            // `run_is_ident` is (re)initialized whenever a new run begins, so it
            // needs no reset here.
        }};
    }

    while let Some((i, c)) = chars.next() {
        // Inside a bracketed engine tag: consume until the matching closer.
        if let Some(close) = skip_until {
            if c == close {
                skip_until = None;
            }
            prev = Some(c);
            continue;
        }

        // Opening bracket → skip the whole span.
        let opener = match c {
            '<' => Some('>'),
            '{' => Some('}'),
            '[' => Some(']'),
            _ => None,
        };
        if let Some(close) = opener {
            flush!(Some(c));
            skip_until = Some(close);
            prev = Some(c);
            continue;
        }

        // Formatting placeholder: `%` followed by a printf-style spec.
        if c == '%' {
            flush!(Some(c));
            // Consume the spec: digits, dots, `+`/`-`/`#`, then one letter, or a
            // literal `%%`.
            while let Some(&(_, n)) = chars.peek() {
                if n.is_ascii_alphanumeric() || matches!(n, '.' | '+' | '#' | '%') {
                    let last = n.is_ascii_alphabetic() || n == '%';
                    chars.next();
                    if last {
                        break;
                    }
                } else {
                    break;
                }
            }
            prev = Some('%');
            continue;
        }

        // Backslash escape (`\n`, `\t`): drop the backslash and the next char.
        if c == '\\' {
            flush!(Some(c));
            chars.next();
            prev = Some('\\');
            continue;
        }

        if is_word_alpha(c) || (is_connector(c) && !buf.is_empty()) {
            if buf.is_empty() {
                buf_start = i;
                run_is_ident = matches!(prev, Some(p) if p.is_ascii_digit() || p == '_');
            }
            buf.push(c);
            buf_end = i + c.len_utf8();
        } else {
            // A digit or underscore glued to the run poisons it as an identifier.
            flush!(Some(c));
        }
        prev = Some(c);
    }
    flush!(None);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        tokens(text).into_iter().map(|t| t.word).collect()
    }

    #[test]
    fn plain_sentence() {
        assert_eq!(
            words("The quick brown fox"),
            ["The", "quick", "brown", "fox"]
        );
    }

    #[test]
    fn skips_engine_tags() {
        assert_eq!(words("Hello <Alias=Player> friend"), ["Hello", "friend"]);
        assert_eq!(
            words("Deals <mag> damage to <global=WICount>"),
            ["Deals", "damage", "to"]
        );
        assert_eq!(words("Take [the] {bag}"), ["Take"]);
    }

    #[test]
    fn skips_placeholders_and_escapes() {
        assert_eq!(
            words("You have %d gold and %.2f coins"),
            ["You", "have", "gold", "and", "coins"]
        );
        assert_eq!(words("Line one\\nLine two"), ["Line", "one", "Line", "two"]);
        assert_eq!(words("100%% sure"), ["sure"]);
    }

    #[test]
    fn skips_identifiers() {
        assert_eq!(words("set Health_Regen now"), ["set", "now"]);
        assert_eq!(words("Attack2 hit MQ101 target"), ["hit", "target"]);
    }

    #[test]
    fn keeps_elisions_and_compounds() {
        assert_eq!(words("l'épée de peut-être"), ["l'épée", "de", "peut-être"]);
        // Surrounding punctuation is trimmed, inner apostrophe kept.
        assert_eq!(words("'aujourd'hui'"), ["aujourd'hui"]);
    }

    #[test]
    fn byte_spans_are_correct() {
        let text = "aa bb";
        let toks = tokens(text);
        assert_eq!(
            toks[0],
            Token {
                word: "aa".into(),
                start: 0,
                end: 2
            }
        );
        assert_eq!(
            toks[1],
            Token {
                word: "bb".into(),
                start: 3,
                end: 5
            }
        );
        // The span round-trips to the original slice, even with multibyte chars.
        let t = "où";
        let tk = tokens(t);
        assert_eq!(&t[tk[0].start..tk[0].end], "où");
    }

    #[test]
    fn single_letters_dropped() {
        assert_eq!(words("a I y"), Vec::<String>::new());
    }
}
