//! ESP-ESM Translator `.eet` binary database codec.
//!
//! Decoded by reverse engineering and validated on real Skyrim/Starfield files.
//! Reading is fully faithful; writing is byte-exact when the data originated
//! from an EET file (raw record bytes are preserved), and "constructed" (valid
//! structure, but the proprietary 4-byte cache hash is regenerated to zero)
//! when the data comes from any other format — because that hash has not been
//! decoded.

use anyhow::{Result, bail};

use super::binio::{Cur, put_eet_str, put_i16, put_i32, put_u16, put_u32};
use super::model::{TmEntry, TmMeta, TranslationMemory, infer_game, norm_strip};

/// Read an EET database from raw bytes.
pub fn read_bytes(data: &[u8]) -> Result<TranslationMemory> {
    let mut c = Cur::new(data);
    c.expect_tag(b"EET_")?;
    let version = c.i32()?;
    let reserved = c.i32()?;
    c.expect_tag(b"GAME")?;
    let gl = c.u16()? as usize;
    let game = String::from_utf8_lossy(c.bytes(gl)?).into_owned();
    c.expect_tag(b"LINE")?;
    let count = c.u32()? as usize;
    let header_end = c.pos;
    let header_hex = hex::encode(&data[0..header_end]);

    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let rec_start = c.pos;
        let size = c.u32()? as usize;
        let next = rec_start + 4 + size;
        if next > data.len() {
            bail!(crate::l10n!("err-data-end", offset = rec_start));
        }
        let grup = c.eet_str()?;
        let id = c.eet_str()?;
        let edid = c.eet_str()?;
        let champ = c.eet_str()?;
        let orig = c.eet_str()?;
        let trad = c.eet_str()?;
        let perso = c.eet_str()?;
        let index = c.i32()?;
        let status = c.i16()?;
        let idst = c.i32()?;
        let comment = c.eet_str()?;
        // ICON is the last i32 of the record.
        let icon_bytes = c.slice(next - 4, next)?;
        let icon = i32::from_le_bytes([icon_bytes[0], icon_bytes[1], icon_bytes[2], icon_bytes[3]]);

        let formid = if id.is_empty() {
            0
        } else {
            u32::from_str_radix(&id, 16).unwrap_or(0)
        };
        let raw_hex = hex::encode(&data[rec_start..next]);

        let mut e = TmEntry {
            source: orig,
            trans: trad,
            formid,
            edid,
            rec_sig: grup,
            field_sig: champ,
            comment,
            index,
            idstexte: idst,
            icon,
            perso,
            status_eet: status,
            eet_raw: Some(raw_hex),
            ..Default::default()
        };
        e.status_sst = if status == 99 {
            vec!["translated".to_string()]
        } else {
            Vec::new()
        };
        entries.push(e);
        c.pos = next;
    }

    let mut meta = TmMeta {
        game,
        source_format: "eet".to_string(),
        eet_header: Some(header_hex),
        eet_version: version,
        eet_reserved: reserved,
        ..Default::default()
    };

    // Masters are listed in the first TES4 record's comment ("… Master Files …").
    for e in &entries {
        if e.rec_sig == "TES4" && e.comment.contains("Master Files") {
            for line in e.comment.lines() {
                let line = line.trim();
                if line.len() >= 2
                    && line.as_bytes()[..2].iter().all(|b| b.is_ascii_digit())
                    && line.contains(" - ")
                    && let Some((_, name)) = line.split_once(" - ")
                {
                    meta.masters.push(name.to_string());
                }
            }
            break;
        }
    }

    Ok(TranslationMemory::new(entries, meta))
}

/// Result of writing an EET file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EetWriteMode {
    /// Reproduced byte-for-byte from a preserved EET source.
    Exact,
    /// Constructed from another format; cache hash regenerated to zero.
    Constructed,
}

/// Serialize a translation memory to EET bytes.
pub fn write_bytes(tm: &TranslationMemory) -> (Vec<u8>, EetWriteMode) {
    // Byte-exact path: header + every preserved raw record.
    if let Some(header_hex) = &tm.meta.eet_header
        && tm.entries.iter().all(|e| e.eet_raw.is_some())
        && let Ok(mut out) = hex::decode(header_hex)
    {
        let mut ok = true;
        for e in &tm.entries {
            match hex::decode(e.eet_raw.as_ref().unwrap()) {
                Ok(raw) => out.extend_from_slice(&raw),
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return (out, EetWriteMode::Exact);
        }
    }

    // Constructed path.
    let game = if tm.meta.game.is_empty() {
        infer_game(&tm.meta.masters)
    } else {
        tm.meta.game.clone()
    };
    let mut out = Vec::new();
    out.extend_from_slice(b"EET_");
    put_i32(&mut out, tm.meta.eet_version);
    put_i32(&mut out, tm.meta.eet_reserved);
    out.extend_from_slice(b"GAME");
    let game_bytes = game.as_bytes();
    put_u16(&mut out, game_bytes.len() as u16);
    out.extend_from_slice(game_bytes);
    out.extend_from_slice(b"LINE");
    put_u32(&mut out, tm.entries.len() as u32);

    for e in &tm.entries {
        let mut body = Vec::new();
        put_eet_str(&mut body, &e.rec_sig);
        put_eet_str(&mut body, &format!("{:08X}", e.formid));
        put_eet_str(&mut body, &e.edid);
        put_eet_str(&mut body, &e.field_sig);
        put_eet_str(&mut body, &e.source);
        put_eet_str(&mut body, &e.trans);
        put_eet_str(&mut body, &e.perso);
        put_i32(&mut body, if e.index == 0 { 1 } else { e.index });
        put_i16(&mut body, e.status_eet);
        put_i32(&mut body, e.idstexte);
        put_eet_str(&mut body, &crate::l10n::strip(&e.comment));
        // Regenerated cache: hash1=0, stripped form, hash2=0, variabilized="", varcount=0.
        put_u32(&mut body, 0);
        put_eet_str(&mut body, &norm_strip(&e.source));
        put_u32(&mut body, 0);
        put_eet_str(&mut body, "");
        put_u32(&mut body, 0);
        put_i32(&mut body, e.icon);

        put_u32(&mut out, body.len() as u32);
        out.extend_from_slice(&body);
    }

    (out, EetWriteMode::Constructed)
}

// Tiny local hex helpers (avoids pulling in an extra crate).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
            s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
        }
        s
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, &'static str> {
        let bytes = s.as_bytes();
        if !bytes.len().is_multiple_of(2) {
            return Err("odd hex length");
        }
        let mut out = Vec::with_capacity(bytes.len() / 2);
        let val = |c: u8| -> Result<u8, &'static str> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => Err("invalid hex digit"),
            }
        };
        for pair in bytes.as_chunks::<2>().0.iter() {
            out.push((val(pair[0])? << 4) | val(pair[1])?);
        }
        Ok(out)
    }
}
