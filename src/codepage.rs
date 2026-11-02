//! Pages de code (D-09): the encodings the older games (Morrowind, Oblivion,
//! Fallout 3 / New Vegas, Skyrim LE) store their text in, besides UTF-8 and
//! Windows-1252 — Central European, Cyrillic, Greek, Turkish, Hebrew, Arabic,
//! Baltic, Vietnamese, Thai, Japanese, Chinese and Korean — with the page a
//! language is written in (`assets/data/codepages.json`), a detection when
//! no language is known, and a lossless round trip for the single-byte
//! pages (an undefined byte is kept in the private use area, U+F700 + byte).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// The pages the application knows. [`Page::Utf8`] is the one of the 64-bit
/// games; the others are single-byte Windows pages, or the multi-byte pages
/// of the East Asian languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Utf8,
    Win1250,
    Win1251,
    Win1252,
    Win1253,
    Win1254,
    Win1255,
    Win1256,
    Win1257,
    Win1258,
    Win874,
    ShiftJis,
    Gbk,
    EucKr,
    Big5,
}

/// Private use area where an undefined byte of a single-byte page is kept.
const PRIVATE_BASE: u32 = 0xF700;
/// Characters of the sample each candidate page is scored on (R-D09-02):
/// enough for the detector, small enough for fifteen decodings.
const DETECT_CHARS: usize = 3_000;

impl Page {
    /// Every page, in the order of the lists.
    pub const ALL: [Page; 15] = [
        Page::Utf8,
        Page::Win1252,
        Page::Win1250,
        Page::Win1251,
        Page::Win1253,
        Page::Win1254,
        Page::Win1255,
        Page::Win1256,
        Page::Win1257,
        Page::Win1258,
        Page::Win874,
        Page::ShiftJis,
        Page::Gbk,
        Page::EucKr,
        Page::Big5,
    ];

    /// The WHATWG name, as stored in files (`enc_in`, `enc_out`).
    pub fn id(self) -> &'static str {
        match self {
            Page::Utf8 => "utf-8",
            Page::Win1250 => "windows-1250",
            Page::Win1251 => "windows-1251",
            Page::Win1252 => "windows-1252",
            Page::Win1253 => "windows-1253",
            Page::Win1254 => "windows-1254",
            Page::Win1255 => "windows-1255",
            Page::Win1256 => "windows-1256",
            Page::Win1257 => "windows-1257",
            Page::Win1258 => "windows-1258",
            Page::Win874 => "windows-874",
            Page::ShiftJis => "shift_jis",
            Page::Gbk => "gbk",
            Page::EucKr => "euc-kr",
            Page::Big5 => "big5",
        }
    }

    /// The i18n key of the displayed name (« Cyrillique (Windows-1251) »).
    pub fn name_key(self) -> &'static str {
        match self {
            Page::Utf8 => "page-utf-8",
            Page::Win1250 => "page-windows-1250",
            Page::Win1251 => "page-windows-1251",
            Page::Win1252 => "page-windows-1252",
            Page::Win1253 => "page-windows-1253",
            Page::Win1254 => "page-windows-1254",
            Page::Win1255 => "page-windows-1255",
            Page::Win1256 => "page-windows-1256",
            Page::Win1257 => "page-windows-1257",
            Page::Win1258 => "page-windows-1258",
            Page::Win874 => "page-windows-874",
            Page::ShiftJis => "page-shift-jis",
            Page::Gbk => "page-gbk",
            Page::EucKr => "page-euc-kr",
            Page::Big5 => "page-big5",
        }
    }

    /// The short label of the tab chip (« 1251 », « UTF-8 »).
    pub fn short(self) -> &'static str {
        match self {
            Page::Utf8 => "UTF-8",
            Page::Win1250 => "1250",
            Page::Win1251 => "1251",
            Page::Win1252 => "1252",
            Page::Win1253 => "1253",
            Page::Win1254 => "1254",
            Page::Win1255 => "1255",
            Page::Win1256 => "1256",
            Page::Win1257 => "1257",
            Page::Win1258 => "1258",
            Page::Win874 => "874",
            Page::ShiftJis => "932",
            Page::Gbk => "936",
            Page::EucKr => "949",
            Page::Big5 => "950",
        }
    }

    /// A page by its WHATWG name or a usual alias; `None` when unknown.
    pub fn from_id(id: &str) -> Option<Page> {
        let k = id.trim().to_lowercase().replace('_', "-");
        let k = k.as_str();
        Some(match k {
            "utf-8" | "utf8" => Page::Utf8,
            "windows-1250" | "cp1250" | "1250" => Page::Win1250,
            "windows-1251" | "cp1251" | "1251" => Page::Win1251,
            "windows-1252" | "cp1252" | "1252" | "ansi" | "latin1" | "iso-8859-1" => Page::Win1252,
            "windows-1253" | "cp1253" | "1253" => Page::Win1253,
            "windows-1254" | "cp1254" | "1254" => Page::Win1254,
            "windows-1255" | "cp1255" | "1255" => Page::Win1255,
            "windows-1256" | "cp1256" | "1256" => Page::Win1256,
            "windows-1257" | "cp1257" | "1257" => Page::Win1257,
            "windows-1258" | "cp1258" | "1258" => Page::Win1258,
            "windows-874" | "cp874" | "874" | "tis-620" => Page::Win874,
            "shift-jis" | "sjis" | "cp932" | "932" | "ms932" => Page::ShiftJis,
            "gbk" | "gb2312" | "cp936" | "936" | "gb18030" => Page::Gbk,
            "euc-kr" | "cp949" | "949" | "ks-c-5601-1987" => Page::EucKr,
            "big5" | "cp950" | "950" => Page::Big5,
            _ => return None,
        })
    }

    /// Whether one byte is one character (the Windows pages).
    pub fn is_single_byte(self) -> bool {
        !matches!(
            self,
            Page::Utf8 | Page::ShiftJis | Page::Gbk | Page::EucKr | Page::Big5
        )
    }

    fn encoding(self) -> &'static encoding_rs::Encoding {
        match self {
            Page::Utf8 => encoding_rs::UTF_8,
            Page::Win1250 => encoding_rs::WINDOWS_1250,
            Page::Win1251 => encoding_rs::WINDOWS_1251,
            Page::Win1252 => encoding_rs::WINDOWS_1252,
            Page::Win1253 => encoding_rs::WINDOWS_1253,
            Page::Win1254 => encoding_rs::WINDOWS_1254,
            Page::Win1255 => encoding_rs::WINDOWS_1255,
            Page::Win1256 => encoding_rs::WINDOWS_1256,
            Page::Win1257 => encoding_rs::WINDOWS_1257,
            Page::Win1258 => encoding_rs::WINDOWS_1258,
            Page::Win874 => encoding_rs::WINDOWS_874,
            Page::ShiftJis => encoding_rs::SHIFT_JIS,
            Page::Gbk => encoding_rs::GBK,
            Page::EucKr => encoding_rs::EUC_KR,
            Page::Big5 => encoding_rs::BIG5,
        }
    }

    /// The 256 characters of a single-byte page (an undefined byte goes to
    /// the private use area), built once per page.
    fn table(self) -> &'static [char; 256] {
        static TABLES: OnceLock<HashMap<Page, Box<[char; 256]>>> = OnceLock::new();
        let tables = TABLES.get_or_init(|| {
            let mut m = HashMap::new();
            for p in Page::ALL {
                if !p.is_single_byte() {
                    continue;
                }
                let mut t = [' '; 256];
                for (b, slot) in t.iter_mut().enumerate() {
                    let b = b as u8;
                    *slot = if p == Page::Win1252 {
                        // Windows-1252 keeps the table of the first step
                        // (the five undefined bytes map straight through).
                        crate::tm::encoding::win1252_char(b)
                    } else {
                        let one = [b];
                        let (s, _, _) = p.encoding().decode(&one);
                        match s.chars().next() {
                            Some('\u{FFFD}') | None => {
                                char::from_u32(PRIVATE_BASE + b as u32).unwrap_or('\u{FFFD}')
                            }
                            Some(c) => c,
                        }
                    };
                }
                m.insert(p, Box::new(t));
            }
            m
        });
        tables.get(&self).expect("single-byte page")
    }

    /// Decode bytes in this page, without a NUL. Lossless for a single-byte
    /// page; a bad multi-byte sequence becomes U+FFFD.
    pub fn decode(self, bytes: &[u8]) -> String {
        match self {
            Page::Utf8 => match std::str::from_utf8(bytes) {
                Ok(s) => crate::tm::encoding::repair_c1_controls(s).into_owned(),
                Err(_) => String::from_utf8_lossy(bytes).into_owned(),
            },
            p if p.is_single_byte() => {
                let t = p.table();
                bytes.iter().map(|&b| t[b as usize]).collect()
            }
            p => {
                let (s, _) = p.encoding().decode_without_bom_handling(bytes);
                s.into_owned()
            }
        }
    }

    /// Encode a character: its bytes, or `None` when the page has no place
    /// for it.
    pub fn encode_char(self, c: char) -> Option<Vec<u8>> {
        match self {
            Page::Utf8 => {
                let mut buf = [0u8; 4];
                Some(c.encode_utf8(&mut buf).as_bytes().to_vec())
            }
            p if p.is_single_byte() => {
                let cp = c as u32;
                if (PRIVATE_BASE..PRIVATE_BASE + 256).contains(&cp) {
                    return Some(vec![(cp - PRIVATE_BASE) as u8]);
                }
                if p == Page::Win1252 {
                    return crate::tm::encoding::win1252_byte(c).map(|b| vec![b]);
                }
                let t = p.table();
                t.iter().position(|&x| x == c).map(|b| vec![b as u8])
            }
            p => {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                let (bytes, _, had_errors) = p.encoding().encode(s);
                if had_errors {
                    None
                } else {
                    Some(bytes.into_owned())
                }
            }
        }
    }

    /// Encode a string, `?` for what the page cannot represent.
    pub fn encode(self, s: &str) -> Vec<u8> {
        if self == Page::Utf8 {
            return s.as_bytes().to_vec();
        }
        let mut out = Vec::with_capacity(s.len());
        for c in s.chars() {
            match self.encode_char(c) {
                Some(b) => out.extend_from_slice(&b),
                None => out.push(b'?'),
            }
        }
        out
    }

    /// The characters of `s` the page cannot represent, with their position
    /// (in characters), each once.
    pub fn unencodable(self, s: &str) -> Vec<(usize, char)> {
        if self == Page::Utf8 {
            return Vec::new();
        }
        let mut out: Vec<(usize, char)> = Vec::new();
        for (i, c) in s.chars().enumerate() {
            if self.encode_char(c).is_none() && !out.iter().any(|(_, x)| *x == c) {
                out.push((i, c));
            }
        }
        out
    }

    /// Whether every character of `s` has a place in the page.
    pub fn can_encode(self, s: &str) -> bool {
        self == Page::Utf8 || s.chars().all(|c| self.encode_char(c).is_some())
    }
}

/// The pages and the languages they serve (`assets/data/codepages.json`).
#[derive(Debug, Clone, Default)]
pub struct Pages {
    by_language: HashMap<String, Page>,
}

#[derive(Deserialize)]
struct PagesFile {
    #[serde(default)]
    pages: Vec<PageEntry>,
}

#[derive(Deserialize)]
struct PageEntry {
    id: String,
    #[serde(default)]
    languages: Vec<String>,
}

impl Pages {
    pub fn parse(text: &str) -> Pages {
        let mut out = Pages::default();
        if let Ok(f) = serde_json::from_str::<PagesFile>(text) {
            for e in f.pages {
                if let Some(p) = Page::from_id(&e.id) {
                    for l in e.languages {
                        out.by_language.insert(l.trim().to_lowercase(), p);
                    }
                }
            }
        }
        out
    }

    /// The pages shipped with the application, read once.
    pub fn builtin() -> &'static Pages {
        static PAGES: OnceLock<Pages> = OnceLock::new();
        PAGES.get_or_init(|| {
            match crate::data::find_data_file("codepages.json")
                .and_then(|p| std::fs::read_to_string(p).ok())
            {
                Some(t) => Pages::parse(&t),
                None => Pages::parse(include_str!("../assets/data/codepages.json")),
            }
        })
    }

    /// The page a language (ISO 639-3) is written in; Windows-1252 for a
    /// language the file does not name (R-D09-01).
    pub fn for_language(&self, lang: &str) -> Page {
        let l = lang.trim().to_lowercase();
        if l.is_empty() {
            return Page::Win1252;
        }
        self.by_language.get(&l).copied().unwrap_or(Page::Win1252)
    }

    /// Whether a language belongs to a page (the detection checks it).
    pub fn serves(&self, page: Page, lang: &str) -> bool {
        self.for_language(lang) == page
    }

    /// The pages worth trying on unknown text: the single-byte pages and
    /// the East Asian ones.
    pub fn candidates(&self) -> Vec<Page> {
        Page::ALL
            .iter()
            .copied()
            .filter(|p| *p != Page::Utf8)
            .collect()
    }
}

/// Why a page was chosen at reading (R-D09-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// The game reads UTF-8, or the bytes were valid UTF-8.
    Utf8,
    /// The page of the declared source language.
    SourceLanguage,
    /// Chosen by the language detector among the candidates.
    Detected,
    /// Windows-1252, nothing else known.
    Default,
    /// Forced by the user.
    Forced,
}

impl Reason {
    pub fn key(self) -> &'static str {
        match self {
            Reason::Utf8 => "encoding-reason-utf8",
            Reason::SourceLanguage => "encoding-reason-source",
            Reason::Detected => "encoding-reason-detected",
            Reason::Default => "encoding-reason-default",
            Reason::Forced => "encoding-reason-forced",
        }
    }
}

/// What a reading decided: the page and why, and the source language the
/// detection proposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub page: Page,
    pub reason: Reason,
    pub language: Option<String>,
}

/// Whether a text decoded by the Windows-1252 fallback: every character
/// has a place in that page (a UTF-8 string of another script has not).
pub fn from_win1252(s: &str) -> bool {
    s.chars().all(|c| {
        let cp = c as u32;
        cp < 0x80 || crate::tm::encoding::win1252_byte(c).is_some()
    })
}

/// The bytes a text decoded in `page` came from (lossless for a single-byte
/// page, see [`Page::decode`]).
pub fn recover(s: &str, page: Page) -> Vec<u8> {
    page.encode(s)
}

/// Decide the page of texts read in Windows-1252 by the fallback (the ones
/// with characters above ASCII), by the language detector (R-D09-02 c):
/// each candidate page decodes the same bytes, the detector scores each
/// decoding, and the page whose best language it serves, with the highest
/// confidence, wins. `None` when nothing beats the fallback.
pub fn detect(texts: &[&str], pages: &Pages) -> Option<(Page, String, f64)> {
    let bytes: Vec<Vec<u8>> = texts
        .iter()
        .filter(|t| t.chars().any(|c| c as u32 >= 0x80))
        .map(|t| recover(t, Page::Win1252))
        .collect();
    if bytes.is_empty() {
        return None;
    }
    let mut best: Option<(Page, String, f64)> = None;
    for page in pages.candidates() {
        let decoded: Vec<String> = bytes.iter().map(|b| page.decode(b)).collect();
        // A page that leaves undefined bytes or replacement characters is
        // not the one.
        if decoded.iter().any(|s| {
            s.chars().any(|c| {
                c == '\u{FFFD}' || (PRIVATE_BASE..PRIVATE_BASE + 256).contains(&(c as u32))
            })
        }) {
            continue;
        }
        // Letters among the characters above ASCII: punctuation-only
        // decodings are unlikely.
        let (letters, high) =
            decoded
                .iter()
                .flat_map(|s| s.chars())
                .fold((0usize, 0usize), |(l, h), c| {
                    if (c as u32) < 0x80 {
                        (l, h)
                    } else {
                        (l + usize::from(c.is_alphabetic()), h + 1)
                    }
                });
        if high == 0 || letters * 2 < high {
            continue;
        }
        let sample = crate::langdetect::sample_n(decoded.iter().map(String::as_str), DETECT_CHARS);
        let cands = crate::langdetect::detect(&sample);
        for c in cands {
            if !pages.serves(page, &c.iso3) {
                continue;
            }
            let score = c.confidence * (letters as f64 / high as f64);
            if best.as_ref().is_none_or(|b| score > b.2) {
                best = Some((page, c.iso3.clone(), score));
            }
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_aliases() {
        for p in Page::ALL {
            assert_eq!(Page::from_id(p.id()), Some(p), "{}", p.id());
        }
        assert_eq!(Page::from_id("CP1251"), Some(Page::Win1251));
        assert_eq!(Page::from_id("Shift_JIS"), Some(Page::ShiftJis));
        assert_eq!(Page::from_id("nope"), None);
    }

    #[test]
    fn single_byte_pages_are_lossless() {
        let all: Vec<u8> = (0..=255u8).collect();
        for p in Page::ALL.iter().copied().filter(|p| p.is_single_byte()) {
            let s = p.decode(&all);
            assert_eq!(s.chars().count(), 256, "{}", p.id());
            assert_eq!(p.encode(&s), all, "{}", p.id());
        }
    }

    #[test]
    fn cyrillic_and_polish() {
        let ru = Page::Win1251.decode(b"\xc1\xe0\xed\xe4\xe8\xf2");
        assert_eq!(ru, "Бандит");
        assert_eq!(Page::Win1251.encode("Бандит"), b"\xc1\xe0\xed\xe4\xe8\xf2");
        let pl = Page::Win1250.decode(b"Bia\xb3a Gra\xf1");
        assert_eq!(pl, "Biała Grań");
        // What a page cannot hold.
        assert_eq!(Page::Win1251.unencodable("œuf"), vec![(0, 'œ')]);
        assert_eq!(Page::Win1251.encode("œuf"), b"?uf");
        assert!(Page::Win1252.unencodable("Biała").len() == 1);
        assert!(Page::Utf8.unencodable("Biała Бандит 日本").is_empty());
    }

    #[test]
    fn east_asian_pages() {
        let jp = "日本語";
        let b = Page::ShiftJis.encode(jp);
        assert_eq!(Page::ShiftJis.decode(&b), jp);
        assert!(Page::Gbk.can_encode("中文"));
        assert!(!Page::Win1251.can_encode("日本語"));
        assert_eq!(Page::Big5.decode(&Page::Big5.encode("繁體")), "繁體");
    }

    #[test]
    fn language_pages() {
        let pages = Pages::parse(include_str!("../assets/data/codepages.json"));
        assert_eq!(pages.for_language("rus"), Page::Win1251);
        assert_eq!(pages.for_language("pol"), Page::Win1250);
        assert_eq!(pages.for_language("fra"), Page::Win1252);
        assert_eq!(pages.for_language("xyz"), Page::Win1252);
        assert_eq!(pages.for_language(""), Page::Win1252);
        assert!(pages.serves(Page::Win1251, "ukr"));
        assert!(from_win1252("Bia³a Grañ"));
        assert!(!from_win1252("Biała"));
    }

    #[test]
    fn detects_russian_and_polish() {
        let pages = Pages::parse(include_str!("../assets/data/codepages.json"));
        let ru_bytes: Vec<Vec<u8>> = [
            "Будучи неуязвимым, нельзя притвориться мертвым.",
            "У меня не получается сломать замок.",
            "Не обнаружен OBSE! Установите его.",
            "Железный меч",
            "Добро пожаловать, путник.",
        ]
        .iter()
        .map(|s| Page::Win1251.encode(s))
        .collect();
        let ru: Vec<String> = ru_bytes.iter().map(|b| Page::Win1252.decode(b)).collect();
        let got = detect(&ru.iter().map(String::as_str).collect::<Vec<_>>(), &pages).unwrap();
        assert_eq!(got.0, Page::Win1251, "{got:?}");
        assert_eq!(got.1, "rus");
        let pl_bytes: Vec<Vec<u8>> = [
            "Biała Grań",
            "Koszary straży miejskiej",
            "Posiadłość Śnieżnych-Stóp",
            "Zażółć gęślą jaźń, powiedział strażnik.",
            "Wejście do miasta jest zamknięte na noc.",
        ]
        .iter()
        .map(|s| Page::Win1250.encode(s))
        .collect();
        let pl: Vec<String> = pl_bytes.iter().map(|b| Page::Win1252.decode(b)).collect();
        let got = detect(&pl.iter().map(String::as_str).collect::<Vec<_>>(), &pages).unwrap();
        assert_eq!(got.0, Page::Win1250, "{got:?}");
        assert_eq!(got.1, "pol");
        // French in 1252: nothing beats the fallback (1252 is not a candidate
        // that needs changing).
        let fr = ["Épée en fer de givre", "Journal du chasseur — page 3"];
        let got = detect(&fr, &pages);
        assert!(got.as_ref().is_none_or(|g| g.0 == Page::Win1252), "{got:?}");
    }
}
