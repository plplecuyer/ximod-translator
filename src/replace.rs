//! Rechercher et remplacer (A-11): the pattern (normal, extended, regex),
//! its targets and scopes, counting and replacing in texts, and the rule
//! files. Pure functions; the window and the undo live in the UI.

use serde::{Deserialize, Serialize};

use crate::tm::TmEntry;

/// How the pattern is read (R-A11-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Normal,
    /// `\n`, `\r`, `\t`, `\\` interpreted in the pattern and the replacement.
    Extended,
    /// Rust regex; `$1`…`$9` and `${name}` in the replacement.
    Regex,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Normal, Mode::Extended, Mode::Regex];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Mode::Normal => "replace-mode-normal",
            Mode::Extended => "replace-mode-extended",
            Mode::Regex => "replace-mode-regex",
        }
    }
}

/// What is searched (R-A11-01); only the translation can be replaced on
/// a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    Trans,
    Source,
    Id,
    Sig,
}

impl Target {
    pub const ALL: [Target; 4] = [Target::Trans, Target::Source, Target::Id, Target::Sig];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Target::Trans => "filter-in-trans",
            Target::Source => "filter-in-source",
            Target::Id => "filter-in-id",
            Target::Sig => "filter-in-sig",
        }
    }

    pub fn replaceable(self) -> bool {
        self == Target::Trans
    }

    /// The text of an entry for this target.
    pub fn text_of(self, e: &TmEntry) -> String {
        match self {
            Target::Trans => e.trans.clone(),
            Target::Source => e.source.clone(),
            Target::Id => {
                if e.edid.is_empty() {
                    format!("{:08X}", e.formid)
                } else {
                    e.edid.clone()
                }
            }
            Target::Sig => format!("{}:{}", e.rec_sig, e.field_sig),
        }
    }
}

/// Which lines (R-A11-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    Document,
    View,
    Selection,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::Document, Scope::View, Scope::Selection];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Scope::Document => "replace-scope-document",
            Scope::View => "replace-scope-view",
            Scope::Selection => "replace-scope-selection",
        }
    }
}

/// One search / replace rule, as typed or as read from a rules file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rule {
    pub find: String,
    #[serde(default)]
    pub replace: String,
    #[serde(default)]
    pub mode: Mode,
    /// Respect the case.
    #[serde(default, rename = "case")]
    pub case_sensitive: bool,
    /// Whole word only.
    #[serde(default)]
    pub whole: bool,
}

/// Interpret `\n`, `\r`, `\t` and `\\` (extended mode).
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('r') => {
                    chars.next();
                    out.push('\r');
                }
                Some('t') => {
                    chars.next();
                    out.push('\t');
                }
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A compiled rule: one regular expression whatever the mode, and the
/// replacement text in the regex crate's syntax.
#[derive(Debug, Clone)]
pub struct Matcher {
    re: regex::Regex,
    replacement: String,
}

impl Rule {
    /// Compile (R-A11-02/03): an invalid expression is the error to show.
    pub fn compile(&self) -> Result<Matcher, String> {
        if self.find.is_empty() {
            return Err(String::new());
        }
        let (body, replacement) = match self.mode {
            Mode::Normal => (regex::escape(&self.find), self.replace.replace('$', "$$")),
            Mode::Extended => (
                regex::escape(&unescape(&self.find)),
                unescape(&self.replace).replace('$', "$$"),
            ),
            Mode::Regex => (self.find.clone(), self.replace.clone()),
        };
        let body = if self.whole {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        let re = regex::RegexBuilder::new(&body)
            .case_insensitive(!self.case_sensitive)
            .size_limit(1 << 22)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Matcher { re, replacement })
    }
}

impl Matcher {
    pub fn count(&self, text: &str) -> usize {
        self.re.find_iter(text).count()
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    /// Replace every occurrence; `(new text, occurrences)`.
    pub fn replace_all(&self, text: &str) -> (String, usize) {
        let n = self.count(text);
        if n == 0 {
            return (text.to_string(), 0);
        }
        (
            self.re
                .replace_all(text, self.replacement.as_str())
                .into_owned(),
            n,
        )
    }
}

/// What the preview says (R-A11-03).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    pub occurrences: usize,
    pub lines: usize,
    /// First matching line: the text before and after.
    pub first: Option<(usize, String, String)>,
}

/// Count the occurrences over the given lines (entry indices, in order).
pub fn preview(entries: &[TmEntry], lines: &[usize], target: Target, m: &Matcher) -> Preview {
    let mut p = Preview::default();
    for &i in lines {
        let Some(e) = entries.get(i) else { continue };
        let text = target.text_of(e);
        let n = m.count(&text);
        if n == 0 {
            continue;
        }
        p.occurrences += n;
        p.lines += 1;
        if p.first.is_none() {
            p.first = Some((i, text.clone(), m.replace_all(&text).0));
        }
    }
    p
}

/// The lines (entry indices) that match, in order.
pub fn matching_lines(
    entries: &[TmEntry],
    lines: &[usize],
    target: Target,
    m: &Matcher,
) -> Vec<usize> {
    lines
        .iter()
        .copied()
        .filter(|&i| {
            entries
                .get(i)
                .is_some_and(|e| m.is_match(&target.text_of(e)))
        })
        .collect()
}

/// A rules file (R-A11-06).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesFile {
    pub format: u32,
    pub rules: Vec<Rule>,
}

pub const RULES_FORMAT: u32 = 1;

impl RulesFile {
    pub fn from_json(text: &str) -> Result<RulesFile, String> {
        let f: RulesFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if f.format != RULES_FORMAT {
            return Err(crate::l10n!(
                "err-rules-format",
                found = f.format,
                expected = RULES_FORMAT
            ));
        }
        Ok(f)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn load(path: &std::path::Path) -> Result<RulesFile, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        RulesFile::from_json(&text)
    }

    /// Append a rule to a file, created when missing.
    pub fn append_to(path: &std::path::Path, rule: Rule) -> Result<usize, String> {
        let mut f = if path.is_file() {
            RulesFile::load(path)?
        } else {
            RulesFile {
                format: RULES_FORMAT,
                rules: Vec::new(),
            }
        };
        f.rules.push(rule);
        let n = f.rules.len();
        crate::config::paths::write_atomic(path, f.to_json().as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(n)
    }
}

/// Render the special characters of a text for display (R-A11-07): LF
/// « ¶ », CR « ↵ », TAB « → », no-break space « ° ». `keep_breaks` keeps
/// the line feed after its glyph (read-only labels); otherwise every
/// substitution is one character for one, so the cursor of a text field
/// keeps its positions.
pub fn show_special(text: &str, keep_breaks: bool) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\n' => {
                out.push('¶');
                if keep_breaks {
                    out.push('\n');
                }
            }
            '\r' => out.push(crate::icons::RETURN_CHAR),
            '\t' => out.push(crate::icons::ARROW_RIGHT_CHAR),
            '\u{a0}' => out.push('°'),
            other => out.push(other),
        }
    }
    out
}

/// Whether a text holds something [`show_special`] would show.
pub fn has_special(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\n' | '\r' | '\t' | '\u{a0}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(find: &str, replace: &str, mode: Mode, case: bool, whole: bool) -> Rule {
        Rule {
            find: find.into(),
            replace: replace.into(),
            mode,
            case_sensitive: case,
            whole,
        }
    }

    #[test]
    fn regex_groups_t_a11_01() {
        let m = rule(r"(\w+) of (\w+)", "$2 de $1", Mode::Regex, false, false)
            .compile()
            .unwrap();
        assert_eq!(
            m.replace_all("Sword of Flames"),
            ("Flames de Sword".to_string(), 1)
        );
        assert_eq!(m.count("Ring of Fire and Cup of Tea"), 2);
    }

    #[test]
    fn extended_mode_t_a11_02() {
        let m = rule(r"\n", " ", Mode::Extended, false, false)
            .compile()
            .unwrap();
        assert_eq!(m.replace_all("a\nb\nc"), ("a b c".to_string(), 2));
        assert_eq!(unescape(r"x\ty\\z\q"), "x\ty\\z\\q");
        // Normal mode takes the backslash literally.
        let m = rule(r"\n", "-", Mode::Normal, false, false)
            .compile()
            .unwrap();
        assert_eq!(m.count("a\nb"), 0);
        assert_eq!(m.count(r"a\nb"), 1);
    }

    #[test]
    fn whole_word_and_case_t_a11_03() {
        let m = rule("Ash", "Cendre", Mode::Normal, false, true)
            .compile()
            .unwrap();
        assert_eq!(
            m.replace_all("Ash and Ashfall"),
            ("Cendre and Ashfall".to_string(), 1)
        );
        let m = rule("ash", "x", Mode::Normal, true, false)
            .compile()
            .unwrap();
        assert_eq!(m.count("Ash ash"), 1);
        let m = rule("ash", "x", Mode::Normal, false, false)
            .compile()
            .unwrap();
        assert_eq!(m.count("Ash ash"), 2);
        // A dollar in a literal replacement stays a dollar.
        let m = rule("a", "$1", Mode::Normal, false, false)
            .compile()
            .unwrap();
        assert_eq!(m.replace_all("a").0, "$1");
        assert!(
            rule("(a|", "", Mode::Regex, false, false)
                .compile()
                .is_err()
        );
        assert!(rule("", "", Mode::Normal, false, false).compile().is_err());
    }

    #[test]
    fn preview_counts_lines_and_occurrences() {
        let mk = |t: &str| TmEntry {
            trans: t.into(),
            source: "s".into(),
            ..Default::default()
        };
        let entries = vec![
            mk("Ring of Fire"),
            mk("none"),
            mk("Cup of Tea, Pot of Gold"),
        ];
        let m = rule(r"(\w+) of (\w+)", "$2 de $1", Mode::Regex, false, false)
            .compile()
            .unwrap();
        let p = preview(&entries, &[0, 1, 2], Target::Trans, &m);
        assert_eq!((p.occurrences, p.lines), (3, 2));
        assert_eq!(
            p.first,
            Some((0, "Ring of Fire".into(), "Fire de Ring".into()))
        );
        assert_eq!(
            matching_lines(&entries, &[2, 1, 0], Target::Trans, &m),
            vec![2, 0]
        );
        assert_eq!(
            preview(&entries, &[1], Target::Trans, &m),
            Preview::default()
        );
    }

    #[test]
    fn rules_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("ximod_rules_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("rules.json");
        assert_eq!(
            RulesFile::append_to(&p, rule("a", "b", Mode::Normal, false, false)).unwrap(),
            1
        );
        assert_eq!(
            RulesFile::append_to(&p, rule(r"\n", "", Mode::Extended, true, true)).unwrap(),
            2
        );
        let f = RulesFile::load(&p).unwrap();
        assert_eq!(f.rules[1].mode, Mode::Extended);
        assert!(f.rules[1].case_sensitive && f.rules[1].whole);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"mode\": \"extended\"") && text.contains("\"case\": true"));
        assert!(RulesFile::from_json("{\"format\":9,\"rules\":[]}").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn special_characters_are_glyphs_only() {
        assert_eq!(
            show_special("a\nb\tc\r d\u{a0}e", false),
            format!(
                "a¶b{}c{} d°e",
                crate::icons::ARROW_RIGHT_CHAR,
                crate::icons::RETURN_CHAR
            )
        );
        assert_eq!(show_special("a\nb", true), "a¶\nb");
        assert_eq!(
            show_special("a\nb", false).chars().count(),
            "a\nb".chars().count()
        );
        assert!(has_special("x\ty") && !has_special("xy"));
    }
}
