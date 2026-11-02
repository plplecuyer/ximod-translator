//! Translatable-text extraction and re-injection for SWF.
//!
//! Coverage (V1), by how safely each round-trips:
//! - **DefineEditText** (dynamic text fields) — extract **and** re-inject. The
//!   initial text is the last field of the tag, so rewriting it is safe.
//! - **AS3 string constant pool** (DoABC/DoABC2) — extract **and** re-inject.
//!   ABC code references strings by *index*, not byte offset, so rewriting a
//!   constant-pool string never invalidates the bytecode.
//! - **AS2 strings** (DoAction / DoInitAction: push-strings and constant pool) —
//!   extract **and** re-inject, via the [`crate::as2`] disassembler/reassembler,
//!   which recomputes every jump offset and block size after an edit. A stream is
//!   only rewritten if its no-edit reassembly is byte-identical (else left as-is).
//! - **Static text** (DefineText / DefineText2) — **extract only** (glyph indices
//!   mapped to characters via the referenced font's code table). Re-injection
//!   needs font glyph remapping (planned).
//!
//! Scaleform `$keys` (e.g. `$SKILLS`) are flagged: in Bethesda games these are
//! resolved from `Interface/Translations/*.txt`, not from the SWF.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::swf::Swf;

const DEFINE_FONT2: u16 = 48;
const DEFINE_FONT3: u16 = 75;
const DEFINE_TEXT: u16 = 11;
const DEFINE_TEXT2: u16 = 33;
const DEFINE_EDIT_TEXT: u16 = 37;
const DO_ACTION: u16 = 12;
const DO_INIT_ACTION: u16 = 59;
const DO_ABC: u16 = 72;
const DO_ABC2: u16 = 82;

/// What kind of text an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    EditText,
    StaticText,
    As2String,
    As3String,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::EditText => "EditText",
            Kind::StaticText => "StaticText",
            Kind::As2String => "As2String",
            Kind::As3String => "As3String",
        }
    }
}

/// Heuristic classification of a string: is it likely displayed text worth
/// translating, likely a code identifier (leave alone), or unclear? Applied to
/// ActionScript strings (AS2/AS3), which mix UI text with function/variable names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    /// Displayed text — safe to translate.
    Text,
    /// A code identifier (path, camelCase name, `_global`…) — do not translate.
    Code,
    /// Ambiguous (e.g. a single bare word) — review before translating.
    Uncertain,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Text => "texte",
            Category::Code => "code",
            Category::Uncertain => "incertain",
        }
    }
}

/// Is `p` a bare ActionScript identifier token?
fn is_ident(p: &str) -> bool {
    !p.is_empty()
        && p.chars().enumerate().all(|(i, c)| {
            if i == 0 {
                c.is_ascii_alphabetic() || c == '_' || c == '$'
            } else {
                c.is_ascii_alphanumeric() || c == '_' || c == '$'
            }
        })
}

/// Does the token have internal camelCase (a lowercase immediately followed by an
/// uppercase), a strong "this is a code name" signal?
fn has_camel(t: &str) -> bool {
    let b = t.as_bytes();
    (1..b.len()).any(|i| b[i - 1].is_ascii_lowercase() && b[i].is_ascii_uppercase())
}

/// Classify an ActionScript string as displayed text, code, or uncertain.
pub fn classify(s: &str) -> Category {
    let t = s.trim();
    if t.is_empty() {
        return Category::Text;
    }
    // Strong display signals: HTML markup, whitespace, or non-ASCII letters.
    let has_html = t.contains('<') && t.contains('>');
    let has_ws = t.chars().any(|c| c.is_whitespace());
    let has_nonascii_alpha = t.chars().any(|c| c.is_alphabetic() && !c.is_ascii());
    if has_html || has_ws || has_nonascii_alpha {
        return Category::Text;
    }
    // Strong code signals: namespaces, paths, or engine globals.
    if t.contains("::") || t.contains('/') || t.starts_with('_') || t.starts_with('$') {
        return Category::Code;
    }
    // Dotted path of identifiers (e.g. `com.foo.Bar`).
    if t.contains('.') && t.split('.').all(is_ident) {
        return Category::Code;
    }
    // A single identifier token: camelCase → code; otherwise a bare word → review.
    if is_ident(t) {
        return if has_camel(t) {
            Category::Code
        } else {
            Category::Uncertain
        };
    }
    // Sentence-like punctuation → text.
    if t.chars().any(|c| {
        matches!(
            c,
            ',' | '!' | '?' | ';' | ':' | '«' | '»' | '%' | '(' | ')' | '\''
        )
    }) {
        return Category::Text;
    }
    Category::Uncertain
}

/// One translatable string found in a SWF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEntry {
    /// Stable id that survives extract → edit → inject (e.g. `et:12`, `as3:5:34`).
    pub id: String,
    pub kind: Kind,
    /// Character id for text tags (DefineEditText / DefineText), if any.
    pub character_id: Option<u16>,
    /// The DefineEditText HTML flag (text is HTML markup).
    pub html: bool,
    /// True when the string is a Scaleform translation key (`$…`) — translated via
    /// the game's translate files, not by editing the SWF.
    pub scaleform_key: bool,
    /// Whether this entry can be written back by [`inject`] in this version.
    pub reinjectable: bool,
    /// Heuristic: displayed text / code identifier / uncertain (for AS strings;
    /// always `Text` for DefineEditText and static text).
    pub category: Category,
    /// Short context hint (variable name, font name, action).
    pub context: String,
    /// The extracted, editable text.
    pub text: String,
}

/// Extract every translatable string from `swf`, in tag order.
pub fn extract(swf: &Swf) -> Vec<TextEntry> {
    let fonts = build_font_tables(swf);
    let mut out = Vec::new();
    for (ti, tag) in swf.tags.iter().enumerate() {
        match tag.code {
            DEFINE_EDIT_TEXT => {
                if let Some((cid, html, text)) = parse_edit_text(&tag.body) {
                    out.push(TextEntry {
                        id: format!("et:{ti}"),
                        kind: Kind::EditText,
                        character_id: Some(cid),
                        html,
                        scaleform_key: is_scaleform_key(&text),
                        reinjectable: true,
                        category: Category::Text,
                        context: "DefineEditText".into(),
                        text,
                    });
                }
            }
            DEFINE_TEXT | DEFINE_TEXT2 => {
                if let Some((cid, text)) =
                    parse_static_text(&tag.body, tag.code == DEFINE_TEXT2, &fonts)
                    && !text.is_empty()
                {
                    out.push(TextEntry {
                        id: format!("st:{ti}"),
                        kind: Kind::StaticText,
                        character_id: Some(cid),
                        html: false,
                        scaleform_key: is_scaleform_key(&text),
                        reinjectable: false,
                        category: Category::Text,
                        context: "DefineText".into(),
                        text,
                    });
                }
            }
            DO_ACTION | DO_INIT_ACTION => {
                let body = as2_actions(tag.code, &tag.body);
                let (slots, reinjectable) = crate::as2::extract_slots(body);
                for (si, s) in slots.into_iter().enumerate() {
                    if s.trim().is_empty() {
                        continue;
                    }
                    out.push(TextEntry {
                        id: format!("as2:{ti}:{si}"),
                        kind: Kind::As2String,
                        character_id: None,
                        html: false,
                        scaleform_key: is_scaleform_key(&s),
                        reinjectable,
                        category: classify(&s),
                        context: "ActionScript 2".into(),
                        text: s,
                    });
                }
            }
            DO_ABC | DO_ABC2 => {
                if let Some(strings) = abc_strings(&tag.body, tag.code == DO_ABC2) {
                    for (si, s) in strings.into_iter().enumerate() {
                        if s.trim().is_empty() {
                            continue;
                        }
                        out.push(TextEntry {
                            id: format!("as3:{ti}:{si}"),
                            kind: Kind::As3String,
                            character_id: None,
                            html: false,
                            scaleform_key: is_scaleform_key(&s),
                            reinjectable: true,
                            category: classify(&s),
                            context: "ActionScript 3".into(),
                            text: s,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Result of an [`inject`] pass.
#[derive(Debug, Default, Clone)]
pub struct InjectReport {
    pub applied: usize,
    pub skipped: usize,
    pub messages: Vec<String>,
}

/// Apply `edits` (id → new text) to `swf` in place. Only reinjectable kinds are
/// written; others are counted as skipped with a message.
pub fn inject(swf: &mut Swf, edits: &HashMap<String, String>) -> InjectReport {
    let mut rep = InjectReport::default();

    // Group AS3/AS2 edits per tag so we rewrite each action/ABC block once.
    let mut as3_by_tag: HashMap<usize, HashMap<usize, String>> = HashMap::new();
    let mut as2_by_tag: HashMap<usize, HashMap<usize, String>> = HashMap::new();

    for (id, new_text) in edits {
        let parts: Vec<&str> = id.split(':').collect();
        match parts.as_slice() {
            ["et", ti] => {
                let Ok(ti) = ti.parse::<usize>() else {
                    rep.skipped += 1;
                    continue;
                };
                match swf.tags.get_mut(ti).filter(|t| t.code == DEFINE_EDIT_TEXT) {
                    Some(tag) => match replace_edit_text(&tag.body, new_text) {
                        Some(nb) => {
                            tag.body = nb;
                            rep.applied += 1;
                        }
                        None => {
                            rep.skipped += 1;
                            rep.messages
                                .push(format!("{id} : DefineEditText illisible"));
                        }
                    },
                    None => {
                        rep.skipped += 1;
                        rep.messages.push(format!("{id} : tag introuvable"));
                    }
                }
            }
            ["as3", ti, si] => {
                if let (Ok(ti), Ok(si)) = (ti.parse::<usize>(), si.parse::<usize>()) {
                    as3_by_tag
                        .entry(ti)
                        .or_default()
                        .insert(si, new_text.clone());
                } else {
                    rep.skipped += 1;
                }
            }
            ["as2", ti, si] => {
                if let (Ok(ti), Ok(si)) = (ti.parse::<usize>(), si.parse::<usize>()) {
                    as2_by_tag
                        .entry(ti)
                        .or_default()
                        .insert(si, new_text.clone());
                } else {
                    rep.skipped += 1;
                }
            }
            ["st", ..] => {
                rep.skipped += 1;
                rep.messages.push(format!(
                    "{id} : texte statique non réinjectable (remappage de glyphes requis)"
                ));
            }
            _ => {
                rep.skipped += 1;
                rep.messages.push(format!("{id} : identifiant inconnu"));
            }
        }
    }

    for (ti, per) in as3_by_tag {
        match swf.tags.get_mut(ti) {
            Some(tag) if tag.code == DO_ABC || tag.code == DO_ABC2 => {
                match rewrite_abc_strings(&tag.body, tag.code == DO_ABC2, &per) {
                    Some(nb) => {
                        tag.body = nb;
                        rep.applied += per.len();
                    }
                    None => {
                        rep.skipped += per.len();
                        rep.messages.push(format!("as3:{ti} : bloc ABC illisible"));
                    }
                }
            }
            _ => {
                rep.skipped += per.len();
                rep.messages.push(format!("as3:{ti} : tag introuvable"));
            }
        }
    }

    for (ti, per) in as2_by_tag {
        match swf.tags.get_mut(ti) {
            Some(tag) if tag.code == DO_ACTION || tag.code == DO_INIT_ACTION => {
                // DoInitAction keeps a 2-byte sprite id before the action stream.
                let prefix = if tag.code == DO_INIT_ACTION {
                    2.min(tag.body.len())
                } else {
                    0
                };
                match crate::as2::reinject(&tag.body[prefix..], &per) {
                    Some(actions) => {
                        let mut nb = tag.body[..prefix].to_vec();
                        nb.extend_from_slice(&actions);
                        tag.body = nb;
                        rep.applied += per.len();
                    }
                    None => {
                        rep.skipped += per.len();
                        rep.messages.push(format!(
                            "as2:{ti} : flux ActionScript non ré-injectable en toute sécurité (ignoré)"
                        ));
                    }
                }
            }
            _ => {
                rep.skipped += per.len();
                rep.messages.push(format!("as2:{ti} : tag introuvable"));
            }
        }
    }

    rep
}

/// The action-stream portion of a DoAction / DoInitAction tag body (skipping the
/// 2-byte sprite id that prefixes DoInitAction).
fn as2_actions(code: u16, body: &[u8]) -> &[u8] {
    if code == DO_INIT_ACTION {
        body.get(2..).unwrap_or(&[])
    } else {
        body
    }
}

/// A Scaleform translation key: a `$` immediately followed by a letter/underscore.
fn is_scaleform_key(s: &str) -> bool {
    let t = s.trim_start();
    let mut c = t.chars();
    c.next() == Some('$')
        && c.next()
            .map(|ch| ch.is_alphabetic() || ch == '_')
            .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// DefineEditText
// ---------------------------------------------------------------------------

/// Byte offset (within a DefineEditText body) where the InitialText string
/// starts, plus (character id, html flag). Returns `None` if there's no text.
fn edit_text_layout(b: &[u8]) -> Option<(usize, u16, bool)> {
    let cid = u16::from_le_bytes([*b.first()?, *b.get(1)?]);
    let mut p = 2usize;
    p += rect_len(b.get(p..)?)?; // Bounds RECT, then byte-aligned flags
    let f1 = *b.get(p)?;
    let f2 = *b.get(p + 1)?;
    p += 2;
    let has_text = f1 & 0x80 != 0;
    let has_text_color = f1 & 0x04 != 0;
    let has_max_length = f1 & 0x02 != 0;
    let has_font = f1 & 0x01 != 0;
    let has_font_class = f2 & 0x80 != 0;
    let has_layout = f2 & 0x20 != 0;
    let html = f2 & 0x02 != 0;
    if has_font {
        p += 2; // FontID
        if has_font_class {
            p = skip_cstr(b, p)?;
        }
        p += 2; // FontHeight
    }
    if has_text_color {
        p += 4;
    }
    if has_max_length {
        p += 2;
    }
    if has_layout {
        p += 9;
    }
    p = skip_cstr(b, p)?; // VariableName
    if !has_text {
        return None;
    }
    if p > b.len() {
        return None;
    }
    Some((p, cid, html))
}

fn parse_edit_text(b: &[u8]) -> Option<(u16, bool, String)> {
    let (off, cid, html) = edit_text_layout(b)?;
    let end = b[off..].iter().position(|&c| c == 0).map(|n| off + n)?;
    Some((cid, html, decode_str(&b[off..end])))
}

/// Rebuild a DefineEditText body with a new InitialText (kept as the last field).
fn replace_edit_text(b: &[u8], new_text: &str) -> Option<Vec<u8>> {
    let (off, _cid, _html) = edit_text_layout(b)?;
    let mut out = Vec::with_capacity(off + new_text.len() + 1);
    out.extend_from_slice(&b[..off]);
    out.extend_from_slice(new_text.as_bytes());
    out.push(0);
    Some(out)
}

// ---------------------------------------------------------------------------
// Static text: DefineText / DefineText2
// ---------------------------------------------------------------------------

/// FontID → (glyph index → character), from DefineFont2/3 code tables.
type FontTables = HashMap<u16, Vec<char>>;

fn build_font_tables(swf: &Swf) -> FontTables {
    let mut m = FontTables::new();
    for tag in &swf.tags {
        if (tag.code == DEFINE_FONT2 || tag.code == DEFINE_FONT3)
            && let Some((id, codes)) = parse_font2_codes(&tag.body)
        {
            m.insert(id, codes);
        }
    }
    m
}

/// Parse a DefineFont2/3 body's code table into glyph→char.
fn parse_font2_codes(b: &[u8]) -> Option<(u16, Vec<char>)> {
    let id = u16::from_le_bytes([*b.first()?, *b.get(1)?]);
    let flags = *b.get(2)?;
    let wide_offsets = flags & 0x08 != 0;
    let wide_codes = flags & 0x04 != 0;
    let name_len = *b.get(4)? as usize; // [3] = language code
    let mut p = 5 + name_len;
    let num_glyphs = u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]) as usize;
    p += 2;
    let offset_base = p;
    let ow = if wide_offsets { 4 } else { 2 };
    // Read the CodeTableOffset (the entry after the NumGlyphs glyph offsets).
    let cto_pos = offset_base + num_glyphs * ow;
    let code_table_offset = read_uint(b, cto_pos, ow)? as usize;
    let mut cp = offset_base + code_table_offset;
    let mut codes = Vec::with_capacity(num_glyphs);
    for _ in 0..num_glyphs {
        let code = if wide_codes {
            let v = u16::from_le_bytes([*b.get(cp)?, *b.get(cp + 1)?]);
            cp += 2;
            v as u32
        } else {
            let v = *b.get(cp)? as u32;
            cp += 1;
            v
        };
        codes.push(char::from_u32(code).unwrap_or('\u{fffd}'));
    }
    Some((id, codes))
}

fn parse_static_text(b: &[u8], text2: bool, fonts: &FontTables) -> Option<(u16, String)> {
    let cid = u16::from_le_bytes([*b.first()?, *b.get(1)?]);
    let mut p = 2usize;
    p += rect_len(b.get(p..)?)?; // TextBounds
    p += matrix_len(b.get(p..)?)?; // TextMatrix (byte-aligned after)
    let glyph_bits = *b.get(p)? as usize;
    let advance_bits = *b.get(p + 1)? as usize;
    p += 2;

    let mut out = String::new();
    let mut cur_font: Option<&Vec<char>> = None;
    loop {
        let flags = *b.get(p)?;
        p += 1;
        if flags == 0 {
            break; // end of records
        }
        let is_control = flags & 0x80 != 0;
        if is_control {
            let has_font = flags & 0x08 != 0;
            let has_color = flags & 0x04 != 0;
            let has_y = flags & 0x02 != 0;
            let has_x = flags & 0x01 != 0;
            if has_font {
                let fid = u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]);
                p += 2;
                cur_font = fonts.get(&fid);
            }
            if has_color {
                p += if text2 { 4 } else { 3 };
            }
            if has_x {
                p += 2;
            }
            if has_y {
                p += 2;
            }
            if has_font {
                p += 2; // TextHeight
            }
        } else {
            // Glyph record: flags low 7 bits = glyph count.
            let count = (flags & 0x7F) as usize;
            let mut br = BitReader::new(b.get(p..)?);
            for _ in 0..count {
                let gi = br.read_bits(glyph_bits)? as usize;
                let _adv = br.read_bits(advance_bits)?;
                if let Some(f) = cur_font
                    && let Some(ch) = f.get(gi)
                {
                    out.push(*ch);
                }
            }
            p += br.byte_len();
        }
    }
    Some((cid, out))
}

// ---------------------------------------------------------------------------
// ActionScript 2 — see the `as2` module (disassembler / reassembler).
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ActionScript 3: DoABC constant-pool strings (extract + inject)
// ---------------------------------------------------------------------------

/// Locate the ABC bytes inside a DoABC/DoABC2 tag body.
fn abc_bytes(b: &[u8], has_name: bool) -> Option<&[u8]> {
    if !has_name {
        return Some(b);
    }
    // DoABC2: u32 flags, then a NUL-terminated name, then the ABC block.
    let start = skip_cstr(b, 4)?;
    b.get(start..)
}

/// Byte span `[start, end)` of the string-constant section within an ABC block,
/// and the decoded strings (index 1..count; index 0 is the implicit empty).
fn abc_string_section(abc: &[u8]) -> Option<(usize, usize, Vec<String>)> {
    let mut p = 4usize; // minor u16 + major u16
    // int, uint pools: count u30, then (count-1) s32/u32 varints.
    for _ in 0..2 {
        let (count, np) = read_u30(abc, p)?;
        p = np;
        for _ in 1..count.max(1) {
            p = skip_varint(abc, p)?;
        }
    }
    // double pool: count u30, then (count-1) * 8 bytes.
    let (dcount, np) = read_u30(abc, p)?;
    p = np + 8 * (dcount.saturating_sub(1)) as usize;
    if p > abc.len() {
        return None;
    }
    // string pool.
    let sec_start = p;
    let (scount, np) = read_u30(abc, p)?;
    p = np;
    let mut strings = Vec::new();
    for _ in 1..scount.max(1) {
        let (len, np) = read_u30(abc, p)?;
        p = np;
        let s = abc.get(p..p + len as usize)?;
        strings.push(decode_str(s));
        p += len as usize;
    }
    Some((sec_start, p, strings))
}

fn abc_strings(body: &[u8], has_name: bool) -> Option<Vec<String>> {
    let abc = abc_bytes(body, has_name)?;
    let (_s, _e, strings) = abc_string_section(abc)?;
    Some(strings)
}

/// Rewrite the ABC string pool with `edits` (string index → new text), leaving
/// int/uint/double pools and everything after the strings untouched (ABC refers
/// to strings by index, so byte lengths here are free to change).
fn rewrite_abc_strings(
    body: &[u8],
    has_name: bool,
    edits: &HashMap<usize, String>,
) -> Option<Vec<u8>> {
    let prefix_len = if has_name { skip_cstr(body, 4)? } else { 0 };
    let abc = &body[prefix_len..];
    let (sec_start, sec_end, strings) = abc_string_section(abc)?;

    // Rebuild the string section: count u30 + each (u30 len + bytes).
    let mut sec = Vec::new();
    write_u30(&mut sec, (strings.len() + 1) as u32); // count includes implicit index 0
    for (i, original) in strings.iter().enumerate() {
        let s = edits.get(&i).unwrap_or(original);
        let bytes = s.as_bytes();
        write_u30(&mut sec, bytes.len() as u32);
        sec.extend_from_slice(bytes);
    }

    let mut out = Vec::with_capacity(body.len());
    out.extend_from_slice(&body[..prefix_len]); // DoABC2 flags + name
    out.extend_from_slice(&abc[..sec_start]); // minor/major + int/uint/double pools
    out.extend_from_slice(&sec); // rewritten string pool
    out.extend_from_slice(&abc[sec_end..]); // the rest of the ABC, unchanged
    Some(out)
}

// ---------------------------------------------------------------------------
// Small binary helpers
// ---------------------------------------------------------------------------

/// Decode a byte string as UTF-8, falling back to Latin-1 for invalid sequences.
fn decode_str(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

/// Advance past a NUL-terminated string; returns the index after the NUL.
fn skip_cstr(b: &[u8], start: usize) -> Option<usize> {
    let n = b.get(start..)?.iter().position(|&c| c == 0)?;
    Some(start + n + 1)
}

/// Byte length of a leading `RECT` in `b`.
fn rect_len(b: &[u8]) -> Option<usize> {
    let nbits = (*b.first()? >> 3) as usize;
    Some((5 + nbits * 4).div_ceil(8))
}

/// Byte length of a leading `MATRIX` in `b` (bit-packed, then byte-aligned).
fn matrix_len(b: &[u8]) -> Option<usize> {
    let mut br = BitReader::new(b);
    let has_scale = br.read_bits(1)? == 1;
    if has_scale {
        let n = br.read_bits(5)? as usize;
        br.read_bits(n * 2)?;
    }
    let has_rotate = br.read_bits(1)? == 1;
    if has_rotate {
        let n = br.read_bits(5)? as usize;
        br.read_bits(n * 2)?;
    }
    let nt = br.read_bits(5)? as usize;
    br.read_bits(nt * 2)?;
    Some(br.byte_len())
}

fn read_uint(b: &[u8], p: usize, width: usize) -> Option<u32> {
    match width {
        2 => Some(u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]) as u32),
        4 => Some(u32::from_le_bytes([
            *b.get(p)?,
            *b.get(p + 1)?,
            *b.get(p + 2)?,
            *b.get(p + 3)?,
        ])),
        _ => None,
    }
}

/// Read an ABC `u30` (variable-length, 7 bits/byte). Returns (value, next_pos).
fn read_u30(b: &[u8], mut p: usize) -> Option<(u32, usize)> {
    let mut val = 0u32;
    let mut shift = 0;
    for _ in 0..5 {
        let byte = *b.get(p)?;
        p += 1;
        val |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some((val, p));
        }
        shift += 7;
    }
    Some((val, p))
}

/// Skip a variable-length integer (u32/s32 varint), returning the next position.
fn skip_varint(b: &[u8], mut p: usize) -> Option<usize> {
    for _ in 0..5 {
        let byte = *b.get(p)?;
        p += 1;
        if byte & 0x80 == 0 {
            return Some(p);
        }
    }
    Some(p)
}

/// Write a value as an ABC `u30` (variable-length, 7 bits/byte).
fn write_u30(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

/// MSB-first bit reader over a byte slice.
struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader { data, bit: 0 }
    }
    fn read_bits(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.bit / 8)?;
            let b = (byte >> (7 - (self.bit % 8))) & 1;
            v = (v << 1) | b as u32;
            self.bit += 1;
        }
        Some(v)
    }
    /// Bytes consumed so far, rounded up.
    fn byte_len(&self) -> usize {
        self.bit.div_ceil(8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swf::{Container, Packing, Swf, Tag};

    #[test]
    fn scaleform_key_detection() {
        assert!(is_scaleform_key("$SKILLS"));
        assert!(is_scaleform_key("  $Gold"));
        assert!(is_scaleform_key("$_priv"));
        assert!(!is_scaleform_key("$100")); // $ then digit = money, not a key
        assert!(!is_scaleform_key("Perk Name"));
        assert!(!is_scaleform_key(""));
    }

    #[test]
    fn classify_code_vs_text() {
        // Clear code identifiers.
        for s in [
            "_global",
            "gotoAndStop",
            "com.foo.Bar",
            "HUDMovieBaseInstance",
            "flash.text::TextField",
        ] {
            assert_eq!(classify(s), Category::Code, "{s} should be code");
        }
        // Clear display text.
        for s in [
            "Start a New Game?",
            "Local map goes here!",
            "Épée longue",
            "<p>Hi</p>",
        ] {
            assert_eq!(classify(s), Category::Text, "{s} should be text");
        }
        // Ambiguous bare words → review.
        for s in ["SKILLS", "Gold", "Level"] {
            assert_eq!(classify(s), Category::Uncertain, "{s} should be uncertain");
        }
    }

    #[test]
    fn u30_roundtrip() {
        for v in [0u32, 1, 63, 127, 128, 300, 16384, 1_000_000, u32::MAX] {
            let mut b = Vec::new();
            write_u30(&mut b, v);
            let (r, n) = read_u30(&b, 0).unwrap();
            assert_eq!(r, v, "value {v}");
            assert_eq!(n, b.len());
        }
    }

    /// Build a DefineEditText body: id, empty bounds RECT, HasText flag only,
    /// empty variable name, then the initial text.
    fn edit_text_tag(id: u16, text: &str) -> Tag {
        let mut body = Vec::new();
        body.extend_from_slice(&id.to_le_bytes());
        body.push(0x00); // RECT nbits=0 → 1 byte
        body.push(0x80); // f1: HasText
        body.push(0x00); // f2
        body.push(0x00); // VariableName ""
        body.extend_from_slice(text.as_bytes());
        body.push(0x00); // InitialText NUL
        Tag {
            code: DEFINE_EDIT_TEXT,
            body,
            force_long: false,
        }
    }

    fn synthetic_swf(tags: Vec<Tag>) -> Swf {
        // header_rest: frame-size RECT (1 byte) + frame rate u16 + frame count u16.
        Swf {
            version: 6,
            container: Container::Flash,
            packing: Packing::None,
            header_rest: vec![0x00, 0, 0, 1, 0],
            tags,
        }
    }

    #[test]
    fn edit_text_extract_and_inject_roundtrip() {
        let mut swf = synthetic_swf(vec![
            edit_text_tag(1, "Hello"),
            edit_text_tag(2, "$Gold"),
            Tag {
                code: 0,
                body: vec![],
                force_long: false,
            }, // End
        ]);

        let entries = extract(&swf);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "et:0");
        assert_eq!(entries[0].text, "Hello");
        assert!(entries[0].reinjectable);
        assert!(!entries[0].scaleform_key);
        assert!(entries[1].scaleform_key); // "$Gold"

        // Inject a translation with accents into the first field.
        let mut edits = HashMap::new();
        edits.insert("et:0".to_string(), "Bonjour à toi".to_string());
        let rep = inject(&mut swf, &edits);
        assert_eq!(rep.applied, 1);

        // Re-emit and re-parse: the edit persists, the other field is untouched.
        let bytes = swf.write().unwrap();
        let swf2 = Swf::parse(&bytes).unwrap();
        let e2 = extract(&swf2);
        assert_eq!(e2[0].text, "Bonjour à toi");
        assert_eq!(e2[1].text, "$Gold");
    }

    #[test]
    fn container_roundtrip_is_byte_identical_when_unedited() {
        let swf = synthetic_swf(vec![
            edit_text_tag(1, "Hi"),
            Tag {
                code: 0,
                body: vec![],
                force_long: false,
            },
        ]);
        let a = swf.write().unwrap();
        let b = Swf::parse(&a).unwrap().write().unwrap();
        assert_eq!(a, b, "FWS load→save must be byte-identical");
    }

    #[test]
    fn gfx_container_roundtrips() {
        let swf = Swf {
            version: 12,
            container: Container::Gfx,
            packing: Packing::None,
            header_rest: vec![0x00, 0, 0, 1, 0],
            tags: vec![
                edit_text_tag(1, "Menu"),
                Tag {
                    code: 0,
                    body: vec![],
                    force_long: false,
                },
            ],
        };
        let bytes = swf.write().unwrap();
        assert_eq!(&bytes[0..3], b"GFX", "GFX container keeps its signature");
        let back = Swf::parse(&bytes).unwrap();
        assert_eq!(back.container, Container::Gfx);
        assert_eq!(extract(&back)[0].text, "Menu");
    }

    #[test]
    fn as2_extract_and_inject_roundtrip() {
        // ActionConstantPool: 0x88, len, count=1, "Menu\0", then End.
        let mut body = vec![0x88];
        let payload = {
            let mut p = Vec::new();
            p.extend_from_slice(&1u16.to_le_bytes());
            p.extend_from_slice(b"Menu\0");
            p
        };
        body.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        body.extend_from_slice(&payload);
        body.push(0x00); // End action
        let mut swf = synthetic_swf(vec![
            Tag {
                code: DO_ACTION,
                body,
                force_long: false,
            },
            Tag {
                code: 0,
                body: vec![],
                force_long: false,
            },
        ]);
        let entries = extract(&swf);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "Menu");
        assert_eq!(entries[0].kind, Kind::As2String);
        assert!(entries[0].reinjectable); // clean stream → editable

        // Re-inject a longer translation and confirm it round-trips.
        let mut edits = HashMap::new();
        edits.insert(entries[0].id.clone(), "Menu Principal".to_string());
        let rep = inject(&mut swf, &edits);
        assert_eq!(rep.applied, 1);
        let e2 = extract(&swf);
        assert_eq!(e2[0].text, "Menu Principal");
    }
}
