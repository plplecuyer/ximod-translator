//! xTranslator `.sst` dictionary codec.
//!
//! Decoded from xTranslator's open-source (MPL 2.0). Strings are UTF-16LE.
//! The magic is "SSU" + a digit: SSU2 = v1 … SSU9 = v8 (current). Reading
//! supports every version; writing always emits SSU9 (v8).

use anyhow::{Result, bail};

use super::binio::{Cur, put_i32, put_sig, put_sst_str, put_u16, put_u32};
use super::model::{STATUS_BITS, TmEntry, TmMeta, TranslationMemory};

/// "SSU9" as a little-endian u32 (version 8).
const SST_MAGIC_V8: u32 = 0x3955_5353;

/// Map an "SSU?" magic (read as LE u32) to its internal version number.
fn magic_to_version(magic: u32) -> Option<u8> {
    match magic {
        0x3255_5353 => Some(1), // SSU2
        0x3355_5353 => Some(2), // SSU3
        0x3455_5353 => Some(3), // SSU4
        0x3555_5353 => Some(4), // SSU5
        0x3655_5353 => Some(5), // SSU6
        0x3755_5353 => Some(6), // SSU7
        0x3855_5353 => Some(7), // SSU8
        0x3955_5353 => Some(8), // SSU9
        _ => None,
    }
}

fn status_from_bits(sp: u8) -> Vec<String> {
    (0..8)
        .filter(|i| sp & (1 << i) != 0)
        .map(|i| STATUS_BITS[i].to_string())
        .collect()
}

/// Read an SST dictionary from raw bytes.
pub fn read_bytes(data: &[u8]) -> Result<TranslationMemory> {
    let mut c = Cur::new(data);
    let magic = c.u32()?;
    let Some(v) = magic_to_version(magic) else {
        bail!(crate::l10n!(
            "err-sst-version",
            magic = format!("{magic:#010x}")
        ));
    };

    if v > 3 {
        let _flag = c.u8()?;
    }

    let mut masters = Vec::new();
    if v > 7 {
        let n = c.i32()?;
        for _ in 0..n {
            masters.push(c.sst_str()?);
        }
    }

    let mut colab_labels = Vec::new();
    if v > 6 {
        let n = c.i32()?;
        for _ in 0..n {
            let cid = c.i32()?;
            let lbl = c.sst_str()?;
            colab_labels.push((cid, lbl));
        }
    }

    let mut entries = Vec::new();
    while !c.eof() {
        let list_index = c.u8()?;
        let mut strid = 0i32;
        let mut formid = 0u32;
        let mut rec_sig = String::new();
        let mut field_sig = String::new();
        let mut index = 0u16;
        let mut index_max = 0u16;
        let mut rhash = 0u32;
        let mut colab_id = 0u8;

        if v > 1 {
            strid = c.i32()?;
            formid = c.u32()?;
            if v > 4 {
                rec_sig = c.sig()?;
            }
            field_sig = c.sig()?;
            if v > 2 {
                index = c.u16()?;
            }
            if v > 3 {
                index_max = c.u16()?;
                rhash = c.u32()?;
            }
            if v > 5 {
                colab_id = c.u8()?;
            }
        }

        let sp = c.u8()?;
        let status_sst = status_from_bits(sp);
        let source = c.sst_str()?;
        let trans = c.sst_str()?;

        let status_eet = if trans.is_empty() { -1 } else { 99 };
        entries.push(TmEntry {
            source,
            trans,
            formid,
            rec_sig,
            field_sig,
            // xTranslator stores this index 0-based; the rest of the tool (EET,
            // XML, plugin extraction) is 1-based, so normalize on read (and undo
            // it on write, keeping the round-trip byte-exact).
            index: index as i32 + 1,
            index_max,
            rhash,
            colab_id,
            strid,
            list_index,
            status_eet,
            status_sst,
            idstexte: -1,
            icon: -1,
            ..Default::default()
        });
    }

    let meta = TmMeta {
        masters,
        colab_labels,
        source_format: "sst".to_string(),
        ..Default::default()
    };
    Ok(TranslationMemory::new(entries, meta))
}

/// Serialize a translation memory to SST bytes (always SSU9 / v8).
pub fn write_bytes(tm: &TranslationMemory) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(&mut out, SST_MAGIC_V8);
    out.push(0); // flag

    put_i32(&mut out, tm.meta.masters.len() as i32);
    for m in &tm.meta.masters {
        put_sst_str(&mut out, m);
    }

    put_i32(&mut out, tm.meta.colab_labels.len() as i32);
    for (cid, lbl) in &tm.meta.colab_labels {
        put_i32(&mut out, *cid);
        put_sst_str(&mut out, lbl);
    }

    for e in &tm.entries {
        out.push(e.list_index);
        put_i32(&mut out, e.strid);
        put_u32(&mut out, e.formid);
        put_sig(&mut out, &e.rec_sig);
        put_sig(&mut out, &e.field_sig);
        // Undo the +1 normalization applied on read (see read_bytes).
        put_u16(&mut out, (e.index - 1).max(0) as u16);
        put_u16(&mut out, e.index_max);
        put_u32(&mut out, e.rhash);
        out.push(e.colab_id);

        // sStrParam bits, dropping 'validated' (as SaveSSTFile does).
        let mut sp = 0u8;
        for (i, name) in STATUS_BITS.iter().enumerate() {
            if *name == "validated" {
                continue;
            }
            if e.status_sst.iter().any(|s| s == name) {
                sp |= 1 << i;
            }
        }
        out.push(sp);

        put_sst_str(&mut out, &e.source);
        put_sst_str(&mut out, &e.trans);
    }

    out
}
