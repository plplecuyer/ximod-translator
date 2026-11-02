//! The VMAD subrecord (C-08): the scripts attached to a record of Skyrim,
//! Fallout 4 or Starfield, with the values of their properties and, for
//! quests, dialogues, packages, perks, scenes and terminals, their script
//! fragments.
//!
//! The block is read sequentially: version (2 to 5 for Skyrim, 6 for
//! Fallout 4 and Starfield), object format, the scripts and their properties,
//! then the fragment section of the record type. Each **string property**
//! (type 2), each element of a **string array** (12) and each string member
//! of a **structure** (Fallout 4, 7 and 17) is a [`Slot`]: where its
//! length-prefixed bytes lie in the block and what it is. The strings of the
//! fragment section (file, script and fragment names) are identifiers and
//! never slots; the scripts attached to a quest's aliases are read like the
//! record's own.
//!
//! A version or a type the reader does not know, or a length that does not
//! land on the end of the block, stops the reading of the block
//! ([`Unread`]): it yields no slot and is written back as it is.
//!
//! The writing ([`rewrite`]) replaces only the slots it is given, with their
//! new length, and copies every other byte of the block as read: the
//! fields the reader skips are never regenerated.

use std::collections::HashMap;

/// A string of the block: offset of its `u16` length, byte length of the
/// text, its decoded text and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// Offset of the `u16` length in the block.
    pub offset: usize,
    /// Length of the text, in bytes.
    pub len: usize,
    /// The script.
    pub script: String,
    /// The property, with the array index and the structure member:
    /// `Pages[2]`, `DeadCountGroups[0].description`.
    pub prop: String,
}

/// Why a block was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// A version outside 2–6.
    Version(u16),
    /// An object format other than 1 or 2.
    ObjectFormat(u16),
    /// A property type the reader does not know.
    Type(u8),
    /// The block ends inside a value.
    Truncated,
    /// The reading ends before the block does (bytes left).
    Trailing(usize),
}

impl std::fmt::Display for Unread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Technical words for the log, not the interface.
        match self {
            Unread::Version(v) => write!(f, "version {v}"),
            Unread::ObjectFormat(v) => write!(f, "object format {v}"),
            Unread::Type(t) => write!(f, "property type {t}"),
            Unread::Truncated => write!(f, "truncated"),
            Unread::Trailing(n) => write!(f, "{n} bytes left"),
        }
    }
}

struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
    slots: Vec<Slot>,
    /// The scripts the block names (attached, of the aliases, of the
    /// fragments), for the scripts of a mod (C-06).
    names: Vec<String>,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Unread> {
        if self.p + n > self.b.len() {
            return Err(Unread::Truncated);
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> Result<(), Unread> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Result<u8, Unread> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Unread> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, Unread> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    /// A `u16`-prefixed string, as an identifier (Latin-1 is enough for
    /// script and property names).
    fn name(&mut self) -> Result<String, Unread> {
        let n = self.u16()? as usize;
        Ok(self.take(n)?.iter().map(|&c| c as char).collect())
    }
    /// A `u16`-prefixed string that is a slot.
    fn slot(&mut self, script: &str, prop: String) -> Result<(), Unread> {
        let offset = self.p;
        let n = self.u16()? as usize;
        self.skip(n)?;
        self.slots.push(Slot {
            offset,
            len: n,
            script: script.to_string(),
            prop,
        });
        Ok(())
    }
    /// The count of an array (`u32`), bounded by what is left.
    fn count(&mut self, item: usize) -> Result<usize, Unread> {
        let n = self.u32()? as usize;
        if n.saturating_mul(item.max(1)) > self.b.len() - self.p {
            return Err(Unread::Truncated);
        }
        Ok(n)
    }
}

/// A property value of type `typ`.
fn value(c: &mut Cursor, typ: u8, script: &str, prop: &str) -> Result<(), Unread> {
    match typ {
        // Object: FormID, alias, unused — 8 bytes in both formats.
        1 => c.skip(8),
        2 => c.slot(script, prop.to_string()),
        3 | 4 => c.skip(4),
        5 => c.skip(1),
        // Structure (Fallout 4): members `name, type, status, value`.
        7 => {
            let n = c.count(4)?;
            for _ in 0..n {
                member(c, script, prop)?;
            }
            Ok(())
        }
        11 => {
            let n = c.count(8)?;
            c.skip(8 * n)
        }
        12 => {
            let n = c.count(2)?;
            for i in 0..n {
                c.slot(script, format!("{prop}[{i}]"))?;
            }
            Ok(())
        }
        13 | 14 => {
            let n = c.count(4)?;
            c.skip(4 * n)
        }
        15 => {
            let n = c.count(1)?;
            c.skip(n)
        }
        // Array of structures: per element, its members.
        17 => {
            let n = c.count(4)?;
            for i in 0..n {
                let m = c.count(4)?;
                let at = format!("{prop}[{i}]");
                for _ in 0..m {
                    member(c, script, &at)?;
                }
            }
            Ok(())
        }
        t => Err(Unread::Type(t)),
    }
}

/// A member of a structure: name, type, status, value.
fn member(c: &mut Cursor, script: &str, at: &str) -> Result<(), Unread> {
    let name = c.name()?;
    let typ = c.u8()?;
    c.u8()?;
    value(c, typ, script, &format!("{at}.{name}"))
}

/// One script entry: name, status (version ≥ 4), properties.
fn script(c: &mut Cursor, version: u16) -> Result<(), Unread> {
    let name = c.name()?;
    c.names.push(name.clone());
    if version >= 4 {
        c.u8()?;
    }
    let n = c.u16()?;
    for _ in 0..n {
        let prop = c.name()?;
        let typ = c.u8()?;
        if version >= 4 {
            c.u8()?;
        }
        value(c, typ, &name, &prop)?;
    }
    Ok(())
}

fn scripts(c: &mut Cursor, version: u16) -> Result<(), Unread> {
    let n = c.u16()?;
    for _ in 0..n {
        script(c, version)?;
    }
    Ok(())
}

/// A fragment: unknown byte, script name, fragment name.
fn fragment(c: &mut Cursor) -> Result<(), Unread> {
    c.u8()?;
    let script = c.name()?;
    c.names.push(script);
    c.name()?;
    Ok(())
}

/// The head of a fragment section: the file name (Skyrim) or a whole script
/// entry (version 6), whose properties are identifiers of the fragments.
fn fragment_head(c: &mut Cursor, version: u16) -> Result<(), Unread> {
    if version >= 6 {
        // The fragment script's properties are not texts of the record:
        // read on a copy so that they make no slot.
        let keep = c.slots.len();
        script(c, version)?;
        c.slots.truncate(keep);
        Ok(())
    } else {
        let file = c.name()?;
        c.names.push(file);
        Ok(())
    }
}

/// The fragment section of `rec` (after the scripts). `false` when the
/// record type has none known.
fn fragments(c: &mut Cursor, rec: &str, version: u16) -> Result<bool, Unread> {
    match rec {
        "INFO" | "PACK" | "SCEN" => {
            c.u8()?;
            let flags = c.u8()?;
            fragment_head(c, version)?;
            for _ in 0..flags.count_ones() {
                fragment(c)?;
            }
            if rec == "SCEN" {
                let n = c.u16()?;
                for _ in 0..n {
                    // Phase flag, phase index, two unknown, the fragment.
                    c.skip(5)?;
                    fragment(c)?;
                }
            }
        }
        "PERK" => {
            c.u8()?;
            fragment_head(c, version)?;
            let n = c.u16()?;
            for _ in 0..n {
                c.skip(4)?;
                fragment(c)?;
            }
        }
        "TERM" | "TMLM" => {
            c.u8()?;
            fragment_head(c, version)?;
            let n = c.u16()?;
            for _ in 0..n {
                c.skip(4)?;
                fragment(c)?;
            }
        }
        "QUST" => {
            c.u8()?;
            let n = c.u16()?;
            fragment_head(c, version)?;
            for _ in 0..n {
                // Stage, unknown, stage index.
                c.skip(8)?;
                fragment(c)?;
            }
            // The aliases: object, version, object format, scripts — the
            // alias scripts' properties are read like the record's.
            let aliases = c.u16()?;
            for _ in 0..aliases {
                c.skip(8)?;
                let v = c.u16()?;
                c.u16()?;
                scripts(c, v)?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Read a VMAD block of a record of signature `rec`: its string slots in
/// block order, or why it was not read.
pub fn parse(rec: &str, data: &[u8]) -> Result<Vec<Slot>, Unread> {
    read(rec, data).map(|c| c.0)
}

/// The scripts a VMAD block names: attached to the record or its aliases,
/// and the fragment scripts (C-06). Empty when the block is not read.
pub fn script_names(rec: &str, data: &[u8]) -> Vec<String> {
    read(rec, data).map(|c| c.1).unwrap_or_default()
}

fn read(rec: &str, data: &[u8]) -> Result<(Vec<Slot>, Vec<String>), Unread> {
    let mut c = Cursor {
        b: data,
        p: 0,
        slots: Vec::new(),
        names: Vec::new(),
    };
    let version = c.u16()?;
    if !(2..=6).contains(&version) {
        return Err(Unread::Version(version));
    }
    let format = c.u16()?;
    if !(1..=2).contains(&format) {
        return Err(Unread::ObjectFormat(format));
    }
    scripts(&mut c, version)?;
    if c.p < data.len() {
        fragments(&mut c, rec, version)?;
    }
    if c.p != data.len() {
        return Err(Unread::Trailing(data.len() - c.p));
    }
    Ok((c.slots, c.names))
}

/// The decoded text of a slot.
pub fn text(data: &[u8], slot: &Slot) -> String {
    let start = slot.offset + 2;
    crate::tm::encoding::decode(&data[start..start + slot.len], true)
}

/// Rewrite a block: the slots of `by_rank` (1-based rank in the block)
/// receive their new bytes, every other byte is copied. A text longer than
/// 65 535 bytes is cut to the last whole character below.
pub fn rewrite(data: &[u8], slots: &[Slot], by_rank: &HashMap<usize, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 64);
    let mut p = 0usize;
    for (i, s) in slots.iter().enumerate() {
        let Some(bytes) = by_rank.get(&(i + 1)) else {
            continue;
        };
        out.extend_from_slice(&data[p..s.offset]);
        let n = bytes.len().min(u16::MAX as usize);
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&bytes[..n]);
        p = s.offset + 2 + s.len;
    }
    out.extend_from_slice(&data[p..]);
    out
}

/// Rewrite the block with translations by rank, encoded for the game
/// (UTF-8 or the output page, D-09). `None` when nothing applies.
pub fn translate(
    rec: &str,
    data: &[u8],
    trans: &HashMap<usize, String>,
    utf8: bool,
) -> Option<Vec<u8>> {
    if trans.is_empty() {
        return None;
    }
    let slots = parse(rec, data).ok()?;
    let by_rank: HashMap<usize, Vec<u8>> = trans
        .iter()
        .filter(|(r, _)| **r >= 1 && **r <= slots.len())
        .map(|(r, t)| {
            (
                *r,
                crate::tm::encoding::encode_capped(t, utf8, u16::MAX as usize),
            )
        })
        .collect();
    if by_rank.is_empty() {
        return None;
    }
    Some(rewrite(data, &slots, &by_rank))
}

/// [`translate`] from the translations of a record, keyed like the
/// extraction: `("VMAD", rank)`.
pub fn translate_record(
    rec: &str,
    data: &[u8],
    fields: &HashMap<(String, i32), String>,
    utf8: bool,
) -> Option<Vec<u8>> {
    let by_rank: HashMap<usize, String> = fields
        .iter()
        .filter(|((f, i), _)| f == crate::vmad_lock::FIELD && *i >= 1)
        .map(|((_, i), t)| (*i as usize, t.clone()))
        .collect();
    translate(rec, data, &by_rank, utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u16).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    /// Version 5: one script, a string, an int, a string array.
    fn skyrim_block() -> Vec<u8> {
        let mut b = vec![5, 0, 2, 0, 1, 0];
        ws(&mut b, "MCM_skipper");
        b.push(0);
        b.extend_from_slice(&3u16.to_le_bytes());
        ws(&mut b, "ModName");
        b.extend_from_slice(&[2, 1]);
        ws(&mut b, "Skipper");
        ws(&mut b, "Version");
        b.extend_from_slice(&[3, 1, 7, 0, 0, 0]);
        ws(&mut b, "Pages");
        b.extend_from_slice(&[12, 1]);
        b.extend_from_slice(&2u32.to_le_bytes());
        ws(&mut b, "Help");
        ws(&mut b, "Files");
        b
    }

    #[test]
    fn strings_and_arrays_are_slots() {
        let b = skyrim_block();
        let slots = parse("QUST", &b).unwrap();
        let got: Vec<(String, String)> = slots
            .iter()
            .map(|s| (format!("{}.{}", s.script, s.prop), text(&b, s)))
            .collect();
        assert_eq!(
            got,
            vec![
                ("MCM_skipper.ModName".into(), "Skipper".into()),
                ("MCM_skipper.Pages[0]".into(), "Help".into()),
                ("MCM_skipper.Pages[1]".into(), "Files".into()),
            ]
        );
    }

    #[test]
    fn rewrite_touches_only_the_translated_slot() {
        let b = skyrim_block();
        let mut t = HashMap::new();
        t.insert(2usize, "Aide et informations".to_string());
        let out = translate("QUST", &b, &t, true).unwrap();
        let slots = parse("QUST", &out).unwrap();
        assert_eq!(text(&out, &slots[1]), "Aide et informations");
        assert_eq!(text(&out, &slots[0]), "Skipper");
        assert_eq!(text(&out, &slots[2]), "Files");
        assert_eq!(out.len(), b.len() + "Aide et informations".len() - 4);
        // Nothing to write: identical.
        let same = rewrite(&b, &slots_of(&b), &HashMap::new());
        assert_eq!(same, b);
    }

    fn slots_of(b: &[u8]) -> Vec<Slot> {
        parse("QUST", b).unwrap()
    }

    #[test]
    fn unknown_version_type_and_length() {
        let mut b = skyrim_block();
        b[0] = 9;
        assert_eq!(parse("QUST", &b), Err(Unread::Version(9)));
        let mut b = skyrim_block();
        b.push(0);
        // A QUST fragment section that does not fit.
        assert!(parse("QUST", &b).is_err());
        let mut b = skyrim_block();
        b.push(1);
        assert!(matches!(parse("ACTI", &b), Err(Unread::Trailing(1))));
        let mut b = vec![5, 0, 2, 0, 1, 0];
        ws(&mut b, "S");
        b.push(0);
        b.extend_from_slice(&1u16.to_le_bytes());
        ws(&mut b, "P");
        b.extend_from_slice(&[6, 1, 0]);
        assert_eq!(parse("ACTI", &b), Err(Unread::Type(6)));
        assert_eq!(parse("ACTI", &b[..3]), Err(Unread::Truncated));
    }
}
