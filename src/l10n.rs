//! Messages built away from the interface — the errors of the readers, the
//! writers, the archives, the online services… — carry an i18n key and its
//! arguments instead of a sentence: the interface translates them in the
//! language of the moment ([`crate::i18n::I18n::localize`]).
//!
//! The message travels as text (inside an `anyhow` error, a `String`, an
//! `io::Error`) in a readable form, `⟦key¦name≔value¦…⟧`, so that a log line
//! stays understandable; a value may itself hold a message, at any depth.
//!
//! ```
//! use ximod_translator::l10n;
//! let m = l10n!("err-archive-read", error = "denied");
//! assert_eq!(m, "⟦err-archive-read¦error≔denied⟧");
//! ```

use std::fmt::Display;

/// Opening bracket of an encoded message.
pub const OPEN: char = '⟦';
/// Closing bracket of an encoded message.
pub const CLOSE: char = '⟧';
/// Separator between the key and the arguments, and between arguments.
pub const SEP: char = '¦';
/// Between an argument's name and its value.
pub const EQ: char = '≔';

/// Encode a message: `key` and its arguments.
pub fn msg(key: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::new();
    out.push(OPEN);
    out.push_str(key);
    for (name, value) in args {
        out.push(SEP);
        out.push_str(name);
        out.push(EQ);
        out.push_str(&value.to_string());
    }
    out.push(CLOSE);
    out
}

/// `l10n!("key", name = value, …)`: an encoded message (see the module).
#[macro_export]
macro_rules! l10n {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::l10n::msg($key, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),*])
    };
}

/// One piece of a text: plain text or an encoded message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Msg {
        key: String,
        args: Vec<(String, String)>,
    },
}

/// Split `text` into plain text and messages (outermost level only: the
/// values keep their own nested messages).
pub fn pieces(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != OPEN {
            plain.push(c);
            continue;
        }
        // Find the matching CLOSE.
        let mut depth = 1;
        let mut end = None;
        for (j, d) in chars.by_ref() {
            if d == OPEN {
                depth += 1;
            } else if d == CLOSE {
                depth -= 1;
                if depth == 0 {
                    end = Some(j);
                    break;
                }
            }
        }
        let Some(end) = end else {
            // Unbalanced: kept as text.
            plain.push_str(&text[i..]);
            break;
        };
        if !plain.is_empty() {
            out.push(Piece::Text(std::mem::take(&mut plain)));
        }
        out.push(parse_inner(&text[i + OPEN.len_utf8()..end]));
    }
    if !plain.is_empty() {
        out.push(Piece::Text(plain));
    }
    out
}

/// `key¦name≔value¦…` (separators inside nested messages ignored).
fn parse_inner(inner: &str) -> Piece {
    let mut parts: Vec<String> = vec![String::new()];
    let mut depth = 0;
    for c in inner.chars() {
        match c {
            OPEN => depth += 1,
            CLOSE => depth -= 1,
            _ => {}
        }
        if c == SEP && depth == 0 {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    let key = parts.remove(0);
    let args = parts
        .into_iter()
        .map(|p| {
            let mut depth = 0;
            let mut cut = None;
            for (i, c) in p.char_indices() {
                match c {
                    OPEN => depth += 1,
                    CLOSE => depth -= 1,
                    EQ if depth == 0 => {
                        cut = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            match cut {
                Some(i) => (p[..i].to_string(), p[i + EQ.len_utf8()..].to_string()),
                None => (p, String::new()),
            }
        })
        .collect();
    Piece::Msg { key, args }
}

/// The text without its encoded messages: for a file written for other
/// tools (a comment generated at reading is not part of the translation).
pub fn strip(text: &str) -> String {
    if !has_msg(text) {
        return text.to_string();
    }
    pieces(text)
        .into_iter()
        .filter_map(|p| match p {
            Piece::Text(t) => Some(t),
            Piece::Msg { .. } => None,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Whether a text holds an encoded message.
pub fn has_msg(text: &str) -> bool {
    text.contains(OPEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_and_split() {
        let inner = crate::l10n!("err-bsa-read", error = "bad header");
        let outer = crate::l10n!("err-open", error = inner, n = 3);
        assert_eq!(
            outer,
            "⟦err-open¦error≔⟦err-bsa-read¦error≔bad header⟧¦n≔3⟧"
        );
        let p = pieces(&format!("a.esp: {outer}!"));
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], Piece::Text("a.esp: ".into()));
        assert_eq!(
            p[1],
            Piece::Msg {
                key: "err-open".into(),
                args: vec![
                    ("error".into(), "⟦err-bsa-read¦error≔bad header⟧".into()),
                    ("n".into(), "3".into())
                ]
            }
        );
        assert_eq!(p[2], Piece::Text("!".into()));
        assert!(!has_msg("plain"));
        // Unbalanced text is left alone.
        assert_eq!(pieces("x ⟦broken"), vec![Piece::Text("x ⟦broken".into())]);
    }
}
