//! String encoding for plugin output: the Bethesda engines store text either as
//! Windows-1252 (older titles) or UTF-8 (the 64-bit Creation Engine ones). This
//! module centralises encoding, decoding and **safe length capping** so the
//! plugin writer, the STRINGS writer and the reader all agree.

/// Largest byte length (including the terminating NUL) allowed for an inline
/// plugin field written without the `XXXX` escape. xTranslator caps at `0xFFF0`
/// to stay clear of the `u16` size limit; we do the same.
pub const MAX_ZSTRING: usize = 0xFFF0;

use crate::codepage::Page;
use std::cell::Cell;

/// How bytes are read while a file is being parsed (D-09): the page, and
/// whether it is forced (`strict`) or only the fallback after the UTF-8
/// test (R-D09-02 a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Codec {
    pub page: Page,
    pub strict: bool,
}

thread_local! {
    /// The codec of the reading in progress on this thread (`None`: the
    /// historical behaviour, UTF-8 else Windows-1252).
    static INPUT: Cell<Option<Codec>> = const { Cell::new(None) };
    /// The page of the writing in progress on this thread (`None`: the
    /// game's choice, UTF-8 or Windows-1252).
    static OUTPUT: Cell<Option<Page>> = const { Cell::new(None) };
}

/// Run `f` with the bytes of every decoded string read in `codec`; the
/// parsers deep in the plugin reader do not know the document, this does.
pub fn with_input<R>(codec: Option<Codec>, f: impl FnOnce() -> R) -> R {
    let before = INPUT.with(|c| c.replace(codec));
    let out = f();
    INPUT.with(|c| c.set(before));
    out
}

/// Run `f` with every string encoded in `page` (`None`: the game's choice).
pub fn with_output<R>(page: Option<Page>, f: impl FnOnce() -> R) -> R {
    let before = OUTPUT.with(|c| c.replace(page));
    let out = f();
    OUTPUT.with(|c| c.set(before));
    out
}

/// The page writing uses on this thread, given the game's choice.
pub fn output_page(utf8: bool) -> Page {
    OUTPUT
        .with(|c| c.get())
        .unwrap_or(if utf8 { Page::Utf8 } else { Page::Win1252 })
}

/// The page a writing in progress imposed on this thread, if any.
pub fn output_override() -> Option<Page> {
    OUTPUT.with(|c| c.get())
}

/// The codec reading uses on this thread, if one was set.
pub fn input_codec() -> Option<Codec> {
    INPUT.with(|c| c.get())
}

/// Decode plugin/STRINGS bytes to a `String`, **auto-detecting** the code page.
///
/// Bethesda plugin and STRINGS files carry no explicit charset marker, so the
/// encoding is inferred from the bytes: if they form valid UTF-8 the text is
/// UTF-8 (the 64-bit Creation Engine titles — Skyrim SE/AE, Fallout 4/76,
/// Starfield); otherwise it is decoded as **Windows-1252** (the older engines —
/// Morrowind, Oblivion, Fallout 3/NV, Skyrim LE). The strict UTF-8 test makes
/// the choice unambiguous: a lone high byte such as `0x96` (en-dash) is invalid
/// UTF-8 and can only be Windows-1252.
///
/// The Windows-1252 branch decodes the 0x80–0x9F block correctly (typographic
/// punctuation: en-dash, curly quotes, the euro sign, …) rather than as raw
/// Latin-1, which would turn those bytes into C1 control characters shown as a
/// box/tofu. It is the exact inverse of [`encode_win1252`].
///
/// `utf8` is the game's *declared* code page (see `Game::uses_utf8`). It is kept
/// for API symmetry with [`encode`] and as documentation of the caller's intent;
/// the content-based test above is what actually decides, so the decode is
/// robust even when a plugin was re-saved in a different encoding by another
/// tool.
pub fn decode(bytes: &[u8], utf8: bool) -> String {
    let _ = utf8;
    // A reading in progress may impose its page (D-09): forced, the page
    // decodes everything; otherwise it replaces the Windows-1252 fallback.
    if let Some(codec) = input_codec() {
        if codec.strict {
            return codec.page.decode(bytes);
        }
        return match std::str::from_utf8(bytes) {
            Ok(s) => repair_c1_controls(s).into_owned(),
            Err(_) => codec.page.decode(bytes),
        };
    }
    // Lossless when valid UTF-8; otherwise fall back to Windows-1252 (never panic).
    // Either way, repair stray C1 controls from a prior Latin-1 mis-decode (e.g.
    // a UTF-8 file that embeds U+0096 instead of the en-dash).
    match std::str::from_utf8(bytes) {
        Ok(s) => repair_c1_controls(s).into_owned(),
        Err(_) => decode_win1252(bytes),
    }
}

/// Decode a byte slice as Windows-1252. ASCII and the Latin-1 upper range
/// (0xA0–0xFF) map directly; the 0x80–0x9F block maps to its Windows-1252
/// punctuation code points. The five bytes undefined in Windows-1252
/// (0x81, 0x8D, 0x8F, 0x90, 0x9D) map to the matching C1 control so that
/// decoding never loses a byte.
pub fn decode_win1252(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| win1252_to_char(b)).collect()
}

/// Repair C1 control characters (U+0080–U+009F) left over from an earlier
/// Latin-1 mis-decode of Windows-1252 bytes.
///
/// These code points are non-printable controls that never legitimately occur
/// in game text; when they appear (e.g. U+0096 instead of the en-dash "–") it is
/// because a byte in the 0x80–0x9F range was once decoded as raw Latin-1 and
/// then stored as UTF-8. Re-mapping each such char through the Windows-1252
/// table restores the intended punctuation. Text without C1 controls is returned
/// unchanged (borrowed, no allocation), so this is cheap to run on every string.
///
/// This lets dictionaries built by older versions display correctly without
/// having to be regenerated.
pub fn repair_c1_controls(s: &str) -> std::borrow::Cow<'_, str> {
    if !s.chars().any(|c| ('\u{0080}'..='\u{009F}').contains(&c)) {
        return std::borrow::Cow::Borrowed(s);
    }
    let fixed: String = s
        .chars()
        .map(|c| {
            if ('\u{0080}'..='\u{009F}').contains(&c) {
                win1252_to_char(c as u8)
            } else {
                c
            }
        })
        .collect();
    std::borrow::Cow::Owned(fixed)
}

/// Map a single Windows-1252 byte to its Unicode code point.
pub fn win1252_char(b: u8) -> char {
    win1252_to_char(b)
}

/// The Windows-1252 byte of a character, `None` when the page has none.
pub fn win1252_byte(c: char) -> Option<u8> {
    let cp = c as u32;
    if cp < 0x80 || (0xA0..=0xFF).contains(&cp) {
        return Some(cp as u8);
    }
    // The five undefined slots map straight through at decoding.
    if (0x81..=0x9F).contains(&cp) {
        return Some(cp as u8);
    }
    let b = encode_win1252(&c.to_string())[0];
    (b != b'?').then_some(b)
}

/// Map a single Windows-1252 byte to its Unicode code point.
fn win1252_to_char(b: u8) -> char {
    match b {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        // ASCII, the Latin-1 upper range, and the five undefined 0x80–0x9F
        // slots (0x81/0x8D/0x8F/0x90/0x9D) map straight through.
        other => other as char,
    }
}

/// Encode a string (no NUL) in the plugin's code page — the page of the
/// writing in progress when one is set (D-09), else UTF-8 or Windows-1252.
pub fn encode(s: &str, utf8: bool) -> Vec<u8> {
    match output_page(utf8) {
        Page::Utf8 => s.as_bytes().to_vec(),
        Page::Win1252 => encode_win1252(s),
        p => p.encode(s),
    }
}

/// Encode a string, capping the result so that appending a NUL keeps it within
/// `max` bytes, **never splitting a multi-byte character**. Returns the bytes
/// without the NUL; callers append it.
pub fn encode_capped(s: &str, utf8: bool, max: usize) -> Vec<u8> {
    let budget = max.saturating_sub(1); // leave room for the NUL
    let page = output_page(utf8);
    let mut out = Vec::new();
    for ch in s.chars() {
        // Encode this character on its own, then check the running budget.
        let mut buf = [0u8; 4];
        let piece: Vec<u8> = match page {
            Page::Utf8 => ch.encode_utf8(&mut buf).as_bytes().to_vec(),
            Page::Win1252 => encode_win1252(&ch.to_string()),
            p => p.encode_char(ch).unwrap_or_else(|| vec![b'?']),
        };
        if out.len() + piece.len() > budget {
            break; // stop on a character boundary
        }
        out.extend_from_slice(&piece);
    }
    out
}

/// Best-effort Windows-1252 encoding: ASCII and Latin-1 map directly, the 0x80–
/// 0x9F special block is mapped, and anything else becomes `?`.
pub fn encode_win1252(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            if cp < 0x80 || (0xA0..=0xFF).contains(&cp) {
                cp as u8
            } else {
                match c {
                    '€' => 0x80,
                    '‚' => 0x82,
                    'ƒ' => 0x83,
                    '„' => 0x84,
                    '…' => 0x85,
                    '†' => 0x86,
                    '‡' => 0x87,
                    'ˆ' => 0x88,
                    '‰' => 0x89,
                    'Š' => 0x8A,
                    '‹' => 0x8B,
                    'Œ' => 0x8C,
                    'Ž' => 0x8E,
                    '‘' => 0x91,
                    '’' => 0x92,
                    '“' => 0x93,
                    '”' => 0x94,
                    '•' => 0x95,
                    '–' => 0x96,
                    '—' => 0x97,
                    '˜' => 0x98,
                    '™' => 0x99,
                    'š' => 0x9A,
                    '›' => 0x9B,
                    'œ' => 0x9C,
                    'ž' => 0x9E,
                    'Ÿ' => 0x9F,
                    _ => b'?',
                }
            }
        })
        .collect()
}

/// Encode a NUL-terminated zstring for an inline plugin field, capped safely at
/// [`MAX_ZSTRING`] on a character boundary.
pub fn encode_zstring(s: &str, utf8: bool) -> Vec<u8> {
    let mut out = encode_capped(s, utf8, MAX_ZSTRING);
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win1252_accents_roundtrip_via_latin1() {
        let enc = encode("Épée à ïôû", false);
        // Each of these code points is in the Latin-1 range → one byte each.
        assert_eq!(decode(&enc, false), "Épée à ïôû");
    }

    #[test]
    fn win1252_punctuation_decodes_not_as_control() {
        // Regression: the German dictionary showed a box for the en-dash.
        // Byte 0x96 is EN DASH in Windows-1252, not the U+0096 control char.
        let raw = b"4 Punkte \x96 Mitglied des Einsatzteams";
        assert_eq!(decode(raw, false), "4 Punkte – Mitglied des Einsatzteams");
        // A few more of the 0x80–0x9F punctuation bytes.
        assert_eq!(decode(b"\x93hi\x94", false), "“hi”");
        assert_eq!(decode(b"a\x92b", false), "a’b");
        assert_eq!(decode(b"\x85", false), "…");
        assert_eq!(decode(b"\x80", false), "€");
    }

    #[test]
    fn repairs_c1_controls_from_legacy_files() {
        // A dictionary built by an older version stored the en-dash as the C1
        // control U+0096 (valid UTF-8, but a non-printable box). Reading it back
        // must restore the real en-dash.
        let legacy = "4 Punkte \u{0096} Mitglied des Einsatzteams";
        assert_eq!(
            repair_c1_controls(legacy),
            "4 Punkte – Mitglied des Einsatzteams"
        );
        // Clean text is returned borrowed (unchanged).
        assert!(matches!(
            repair_c1_controls("déjà vu — ok"),
            std::borrow::Cow::Borrowed(_)
        ));
        // A UTF-8 byte stream embedding U+0096 is repaired by decode() too.
        assert_eq!(decode("a\u{0096}b".as_bytes(), true), "a–b");
    }

    #[test]
    fn win1252_full_range_roundtrips() {
        // Every character Windows-1252 can encode must survive encode→decode.
        for b in 0u8..=0xFF {
            let ch = super::win1252_to_char(b);
            let enc = encode_win1252(&ch.to_string());
            // Undefined slots encode to '?'; skip those (not representable).
            if enc == [b] {
                assert_eq!(decode(&enc, false), ch.to_string());
            }
        }
    }

    #[test]
    fn utf8_passthrough() {
        let enc = encode("Épée 日本", true);
        assert_eq!(decode(&enc, true), "Épée 日本");
    }

    #[test]
    fn capping_never_splits_utf8() {
        // 100 two-byte chars = 200 bytes; cap to 10 → at most 9 content bytes,
        // i.e. 4 chars (8 bytes), never a half character.
        let s: String = "é".repeat(100);
        let enc = encode_capped(&s, true, 10);
        assert!(enc.len() <= 9);
        assert_eq!(enc.len() % 2, 0, "no half UTF-8 sequence");
        // The capped bytes must still decode cleanly.
        assert!(std::str::from_utf8(&enc).is_ok());
    }

    #[test]
    fn zstring_is_capped_and_terminated() {
        let s: String = "A".repeat(70_000);
        let z = encode_zstring(&s, false);
        assert!(z.len() <= MAX_ZSTRING);
        assert_eq!(*z.last().unwrap(), 0, "NUL-terminated");
    }
}
