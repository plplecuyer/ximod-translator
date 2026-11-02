//! Protection before sending and restoration after (R-D02-01..03): the
//! tags and aliases of A-07 and the placeholders of text files become
//! `<a_A>`, `<a_B>`…, the numbers `<n_A>`…, the line breaks `<br_A>`…;
//! each token comes back once or the answer is refused.

use crate::tags::TagSet;

/// A protected fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// `a_A`, `n_B`, `br_C`.
    pub name: String,
    /// The original text.
    pub original: String,
}

/// What to protect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub tags: bool,
    pub numbers: bool,
    pub newlines: bool,
}

impl Options {
    pub fn all() -> Options {
        Options {
            tags: true,
            numbers: true,
            newlines: true,
        }
    }
}

/// `A`, `B`… `Z`, `AA`, `AB`…
pub fn letters(i: usize) -> String {
    let mut n = i;
    let mut out = Vec::new();
    loop {
        out.push((b'A' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    out.iter().rev().collect()
}

/// Protect a text: the tokens replace the fragments, in order of
/// appearance; the list says what each token stands for.
pub fn protect(text: &str, set: &TagSet, opts: Options) -> (String, Vec<Token>) {
    // Spans (start, end, kind) to replace, non-overlapping, sorted.
    let mut spans: Vec<(usize, usize, &str)> = Vec::new();
    if opts.tags {
        for s in crate::tags::tags(text, set) {
            spans.push((s.start, s.end, "a"));
        }
    }
    if opts.numbers {
        for s in crate::tags::numbers(text, set) {
            if !spans.iter().any(|(a, b, _)| s.start < *b && s.end > *a) {
                spans.push((s.start, s.end, "n"));
            }
        }
    }
    if opts.newlines {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                spans.push((i, i + 2, "br"));
                i += 2;
            } else if bytes[i] == b'\n' || bytes[i] == b'\r' {
                spans.push((i, i + 1, "br"));
                i += 1;
            } else {
                i += 1;
            }
        }
    }
    spans.sort_by_key(|s| s.0);
    let mut out = String::with_capacity(text.len());
    let mut tokens = Vec::new();
    let mut counts = std::collections::HashMap::new();
    let mut pos = 0;
    for (start, end, kind) in spans {
        if start < pos {
            continue;
        }
        out.push_str(&text[pos..start]);
        let k = counts.entry(kind).or_insert(0usize);
        let name = format!("{kind}_{}", letters(*k));
        *k += 1;
        out.push_str(&format!("<{name}>"));
        tokens.push(Token {
            name,
            original: text[start..end].to_string(),
        });
        pos = end;
    }
    out.push_str(&text[pos..]);
    (out, tokens)
}

/// The token names of protected texts (`a_A`, `n_B`…), for the services
/// that take a list of tags to leave alone.
pub fn token_names(texts: &[String]) -> Vec<String> {
    let re = token_regex();
    let mut names: Vec<String> = Vec::new();
    for t in texts {
        for c in re.captures_iter(t) {
            let name = format!("{}_{}", c[1].to_lowercase(), c[2].to_uppercase());
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn token_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)<\s*/?\s*(a|n|br)_([a-z]+)\s*/?\s*>").unwrap())
}

/// Why an answer is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A token absent, doubled or altered.
    TokenLost(String),
    /// Nothing came back.
    Empty,
    /// The answer is the source again (languages differ).
    Unchanged,
}

/// Restore the originals in an answer (R-D02-02 / 03): each token must
/// come back exactly once (`<a_a>`, `< a_A >`, `<a_A/>` are normalised);
/// the order may differ.
pub fn restore(answer: &str, tokens: &[Token], original: &str) -> Result<String, Refusal> {
    if answer.trim().is_empty() {
        return Err(Refusal::Empty);
    }
    let re = token_regex();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut out = String::with_capacity(answer.len());
    let mut pos = 0;
    for c in re.captures_iter(answer) {
        let m = c.get(0).unwrap();
        let name = format!("{}_{}", c[1].to_lowercase(), c[2].to_uppercase());
        out.push_str(&answer[pos..m.start()]);
        match tokens.iter().find(|t| t.name == name) {
            Some(t) => {
                *seen.entry(name).or_insert(0) += 1;
                out.push_str(&t.original);
            }
            None => return Err(Refusal::TokenLost(name)),
        }
        pos = m.end();
    }
    out.push_str(&answer[pos..]);
    for t in tokens {
        match seen.get(&t.name) {
            Some(1) => {}
            _ => return Err(Refusal::TokenLost(t.name.clone())),
        }
    }
    if out.trim() == original.trim() && original.chars().any(|c| c.is_alphabetic()) {
        return Err(Refusal::Unchanged);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> TagSet {
        TagSet::from_names(&["Alias*", "Global*", "{*}", "%*"])
    }

    #[test]
    fn letters_count_past_z() {
        assert_eq!(letters(0), "A");
        assert_eq!(letters(25), "Z");
        assert_eq!(letters(26), "AA");
        assert_eq!(letters(27), "AB");
        assert_eq!(letters(52), "BA");
    }

    /// T-D02-01: aliases, numbers and line breaks protected, restored,
    /// refused when lost.
    #[test]
    fn protects_and_restores() {
        let src = "Give <Alias=Victim> 3 gold.\nNow {0}, 2.5 times.";
        let (sent, tokens) = protect(src, &set(), Options::all());
        assert_eq!(sent, "Give <a_A> <n_A> gold.<br_A>Now <a_B>, <n_B> times.");
        assert_eq!(tokens.len(), 5);
        assert_eq!(
            token_names(std::slice::from_ref(&sent)),
            vec!["a_A", "n_A", "br_A", "a_B", "n_B"]
        );
        let answer = "Donnez <n_A> pièces d'or à <a_a>.<BR_A>Maintenant < a_B >, <n_B/> fois.";
        let back = restore(answer, &tokens, src).unwrap();
        assert_eq!(
            back,
            "Donnez 3 pièces d'or à <Alias=Victim>.\nMaintenant {0}, 2.5 fois."
        );
        // A lost token.
        assert_eq!(
            restore(
                "Donnez <n_A> pièces à <a_A>.<br_A>Maintenant <a_B>, fois.",
                &tokens,
                src
            ),
            Err(Refusal::TokenLost("n_B".into()))
        );
        // A doubled token.
        assert!(matches!(
            restore("<a_A> <a_A> <n_A> <br_A> <a_B> <n_B>", &tokens, src),
            Err(Refusal::TokenLost(_))
        ));
        // An unknown token.
        assert!(matches!(
            restore("<a_C> <a_A> <n_A> <br_A> <a_B> <n_B>", &tokens, src),
            Err(Refusal::TokenLost(_))
        ));
        assert_eq!(restore("   ", &tokens, src), Err(Refusal::Empty));
        assert_eq!(restore(&sent, &tokens, src), Err(Refusal::Unchanged));
    }

    #[test]
    fn options_select_what_is_protected() {
        let src = "Level 3\nnext <Alias=X>";
        let o = Options {
            tags: false,
            numbers: false,
            newlines: true,
        };
        let (sent, tokens) = protect(src, &set(), o);
        assert_eq!(sent, "Level 3<br_A>next <Alias=X>");
        assert_eq!(tokens.len(), 1);
        let (plain, t) = protect("No tags here", &set(), Options::all());
        assert_eq!(plain, "No tags here");
        assert!(t.is_empty());
        assert_eq!(
            restore("Pas de balise ici", &t, "No tags here").unwrap(),
            "Pas de balise ici"
        );
        assert_eq!(
            restore("No tags here", &t, "No tags here"),
            Err(Refusal::Unchanged)
        );
        assert_eq!(
            restore("42", &[], "42").unwrap(),
            "42",
            "a number alone may stay"
        );
    }
}
