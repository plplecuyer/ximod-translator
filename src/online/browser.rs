//! Ctrl+G without a service (D-117, R-T03): the browser opens the site the
//! user chose in Paramètres › En ligne — a translation site that receives
//! the line, or an assistant that receives a translation instruction
//! followed by the line. Nothing is sent by the application itself: the
//! browser opens a page, the user copies the answer back.
//!
//! The address formats of the assistants are not documented by their
//! publishers and may change; an assistant therefore also gets its text in
//! the clipboard, so that a refused address still leaves something to paste.

use crate::tags::{self, TagSet};

/// Beyond this length an address is too long for some browsers: the text
/// goes to the clipboard and the site opens empty (R-T03).
pub const MAX_URL: usize = 2000;

/// The sites Ctrl+G can open (`[Online] BrowserSite`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Site {
    /// Google Translate, as in 0.3.1.
    #[default]
    Google,
    Deepl,
    Chatgpt,
    Claude,
    Perplexity,
}

impl Site {
    pub const ALL: [Site; 5] = [
        Site::Google,
        Site::Deepl,
        Site::Chatgpt,
        Site::Claude,
        Site::Perplexity,
    ];

    /// The value of `[Online] BrowserSite`.
    pub fn id(self) -> &'static str {
        match self {
            Site::Google => "google",
            Site::Deepl => "deepl",
            Site::Chatgpt => "chatgpt",
            Site::Claude => "claude",
            Site::Perplexity => "perplexity",
        }
    }

    /// The site of a configuration value; Google for an unknown one.
    pub fn from_id(s: &str) -> Site {
        Site::ALL
            .into_iter()
            .find(|x| x.id().eq_ignore_ascii_case(s.trim()))
            .unwrap_or_default()
    }

    /// The message key of its name in the list.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Site::Google => "browser-site-google",
            Site::Deepl => "browser-site-deepl",
            Site::Chatgpt => "browser-site-chatgpt",
            Site::Claude => "browser-site-claude",
            Site::Perplexity => "browser-site-perplexity",
        }
    }

    /// An assistant receives an instruction, a translation site the line.
    pub fn is_assistant(self) -> bool {
        matches!(self, Site::Chatgpt | Site::Claude | Site::Perplexity)
    }

    /// The address with `text`, the languages in ISO 639-1 (`src` empty:
    /// detected by the site).
    fn url_with(self, text: &str, src: &str, dst: &str) -> String {
        let enc = encode(text);
        match self {
            Site::Google => format!(
                "https://translate.google.com/?sl={}&tl={dst}&text={enc}&op=translate",
                or_auto(src)
            ),
            // DeepL reads the text in the fragment, a slash written « \/ ».
            Site::Deepl => format!(
                "https://www.deepl.com/translator#{}/{dst}/{}",
                or_auto(src),
                enc.replace("%2F", "%5C%2F")
            ),
            Site::Chatgpt => format!("https://chatgpt.com/?q={enc}"),
            Site::Claude => format!("https://claude.ai/new?q={enc}"),
            Site::Perplexity => format!("https://www.perplexity.ai/search?q={enc}"),
        }
    }

    /// The page without text, where the user pastes.
    fn home(self, src: &str, dst: &str) -> String {
        match self {
            Site::Google => format!(
                "https://translate.google.com/?sl={}&tl={dst}&op=translate",
                or_auto(src)
            ),
            Site::Deepl => format!("https://www.deepl.com/translator#{}/{dst}/", or_auto(src)),
            Site::Chatgpt => "https://chatgpt.com/".into(),
            Site::Claude => "https://claude.ai/new".into(),
            Site::Perplexity => "https://www.perplexity.ai/".into(),
        }
    }
}

fn or_auto(src: &str) -> &str {
    if src.trim().is_empty() { "auto" } else { src }
}

/// Percent-encoding of everything but the unreserved characters.
pub fn encode(text: &str) -> String {
    let mut enc = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                enc.push(b as char)
            }
            _ => enc.push_str(&format!("%{b:02X}")),
        }
    }
    enc
}

/// What the languages and the game of the line are.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// ISO 639-1, empty when unknown.
    pub src: String,
    pub dst: String,
    /// The game's name, for the instruction.
    pub game: String,
    /// The tags to list in the instruction: every `<…>` of a plugin line
    /// (`<mag>`, `<dur>`, `<1>` are not among the game's checked tags, but
    /// an assistant must copy them all), the placeholders of a text file.
    pub tags: TagSet,
}

/// What Ctrl+G does: the address to open, the text to put in the
/// clipboard, and whether the page opens without it (a line too long).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    pub url: String,
    pub clipboard: Option<String>,
    pub too_long: bool,
}

/// The opening of `line` on `site` (R-T03).
pub fn opening(site: Site, line: &str, cx: &Context) -> Opening {
    let text = if site.is_assistant() {
        instruction(line, cx)
    } else {
        line.to_string()
    };
    let url = site.url_with(&text, &cx.src, &cx.dst);
    if url.len() > MAX_URL {
        return Opening {
            url: site.home(&cx.src, &cx.dst),
            clipboard: Some(text),
            too_long: true,
        };
    }
    Opening {
        url,
        // An assistant's address may be refused: the text stays at hand.
        clipboard: site.is_assistant().then_some(text),
        too_long: false,
    }
}

/// The instruction an assistant receives, followed by the line: the rules
/// of the profiles' prompt (D-03) for one text — languages, the line's tags
/// and numbers to copy, line breaks to keep, the answer alone. In English,
/// like the profiles: every model reads it.
pub fn instruction(line: &str, cx: &Context) -> String {
    let lang = |iso1: &str| {
        if iso1.trim().is_empty() {
            "its language".to_string()
        } else {
            super::profile::lang_name(iso1)
        }
    };
    let mut s = String::new();
    s.push_str("You are a professional translator of video game texts");
    if !cx.game.trim().is_empty() {
        s.push_str(&format!(" ({})", cx.game.trim()));
    }
    s.push_str(".\n");
    s.push_str(&format!(
        "Translate the text below from {} to {}.\nRules:\n",
        lang(&cx.src),
        lang(&cx.dst)
    ));
    s.push_str(
        "- Answer with the translation only: no commentary, no quotation marks, no code fence.\n",
    );
    let mut found: Vec<String> = Vec::new();
    for t in tags::tags(line, &cx.tags) {
        if !found.contains(&t.text) {
            found.push(t.text);
        }
    }
    if !found.is_empty() {
        s.push_str(&format!(
            "- Copy these tags exactly, never translate, remove or add one: {}.\n",
            found.join(" ")
        ));
    }
    let mut numbers: Vec<String> = Vec::new();
    for n in tags::numbers(line, &cx.tags) {
        if !numbers.contains(&n.text) {
            numbers.push(n.text);
        }
    }
    if !numbers.is_empty() {
        s.push_str(&format!("- Keep these numbers: {}.\n", numbers.join(", ")));
    }
    if line.contains('\n') {
        s.push_str("- Keep every line break where it is.\n");
    }
    s.push_str(
        "- Keep the tone, the register and the punctuation of the original; keep proper nouns.\n",
    );
    s.push_str("\nText:\n");
    s.push_str(line);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx() -> Context {
        Context {
            src: "en".into(),
            dst: "fr".into(),
            game: "The Elder Scrolls V: Skyrim Special Edition".into(),
            tags: TagSet::any(),
        }
    }

    #[test]
    fn each_site_opens_with_the_line_and_the_languages() {
        let line = "Iron Sword / <mag> points";
        let g = opening(Site::Google, line, &cx());
        assert!(
            g.url
                .starts_with("https://translate.google.com/?sl=en&tl=fr&text=Iron%20Sword")
        );
        assert_eq!(g.clipboard, None);
        let d = opening(Site::Deepl, line, &cx());
        assert!(
            d.url
                .starts_with("https://www.deepl.com/translator#en/fr/Iron%20Sword%20%5C%2F")
        );
        for (site, base) in [
            (Site::Chatgpt, "https://chatgpt.com/?q="),
            (Site::Claude, "https://claude.ai/new?q="),
            (Site::Perplexity, "https://www.perplexity.ai/search?q="),
        ] {
            let o = opening(site, line, &cx());
            assert!(o.url.starts_with(base), "{}", o.url);
            assert!(!o.too_long);
            // The instruction with the line, also in the clipboard.
            let text = o.clipboard.unwrap();
            assert!(text.contains("from English to French"), "{text}");
            assert!(text.ends_with(line));
            assert!(o.url.contains(&encode("from English to French")));
        }
        // No source language: detected by the site.
        let c = Context {
            src: String::new(),
            ..cx()
        };
        assert!(opening(Site::Google, "x", &c).url.contains("sl=auto&tl=fr"));
    }

    #[test]
    fn a_long_line_goes_through_the_clipboard() {
        let line = "a".repeat(2500);
        for site in Site::ALL {
            let o = opening(site, &line, &cx());
            assert!(o.too_long, "{site:?}");
            assert!(o.url.len() < 100, "{}", o.url);
            assert!(o.clipboard.as_deref().unwrap().ends_with(&line));
        }
    }

    #[test]
    fn the_instruction_lists_the_tags_and_numbers_of_the_line() {
        let text = instruction(
            "Deals <mag> damage for 10 seconds.\nMax. <1> times, 25%.",
            &cx(),
        );
        assert!(text.contains("tags exactly"), "{text}");
        assert!(text.contains("<mag>") && text.contains("<1>"), "{text}");
        assert!(text.contains("Keep these numbers: 10"), "{text}");
        assert!(text.contains("line break"), "{text}");
        assert!(text.contains("Skyrim Special Edition"));
        let plain = instruction("Iron Sword", &cx());
        assert!(!plain.contains("tags exactly"));
        assert!(!plain.contains("numbers"));
    }

    #[test]
    fn the_whole_text_is_encoded() {
        // (The test of the Google address of 0.3.1, kept.)
        assert_eq!(
            encode("Salt & Pepper #2\nA/B é"),
            "Salt%20%26%20Pepper%20%232%0AA%2FB%20%C3%A9"
        );
    }

    #[test]
    fn the_setting_reads_back() {
        for s in Site::ALL {
            assert_eq!(Site::from_id(s.id()), s);
        }
        assert_eq!(Site::from_id("unknown"), Site::Google);
        assert_eq!(Site::from_id(" Claude "), Site::Claude);
    }
}
