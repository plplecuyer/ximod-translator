//! Minimal little-endian binary cursor and writer helpers used by the SST and
//! EET codecs. Everything is bounds-checked and returns `anyhow::Result` so a
//! truncated or malformed file is a clean error, never a panic.

use anyhow::{Context, Result, bail};

/// A read cursor over an in-memory byte buffer.
pub struct Cur<'a> {
    data: &'a [u8],
    pub pos: usize,
}

impl<'a> Cur<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            bail!(crate::l10n!("err-data-end", offset = self.pos));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    /// Read `n` raw bytes.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.take(n)
    }

    /// Peek a fixed-size tag without advancing, and check it matches.
    pub fn expect_tag(&mut self, tag: &[u8]) -> Result<()> {
        let got = self.take(tag.len())?;
        if got != tag {
            bail!(crate::l10n!(
                "err-data-corrupt",
                offset = self.pos - tag.len()
            ));
        }
        Ok(())
    }

    /// EET string: u32 byte length + UTF-8 bytes.
    pub fn eet_str(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let b = self.take(n)?;
        Ok(String::from_utf8_lossy(b).into_owned())
    }

    /// SST string: i32 byte length + UTF-16LE bytes.
    pub fn sst_str(&mut self) -> Result<String> {
        let n = self.i32()?;
        if n < 0 {
            bail!(crate::l10n!("err-data-corrupt", offset = self.pos));
        }
        let b = self.take(n as usize)?;
        if b.len() % 2 != 0 {
            bail!(crate::l10n!("err-data-corrupt", offset = self.pos));
        }
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&units))
    }

    /// SST 4-byte signature (latin-1, trailing NULs trimmed).
    pub fn sig(&mut self) -> Result<String> {
        let b = self.take(4)?;
        let s: String = b.iter().map(|&c| c as char).collect();
        Ok(s.trim_end_matches('\u{0}').to_string())
    }

    /// Absolute slice (used to capture raw EET record bytes).
    pub fn slice(&self, start: usize, end: usize) -> Result<&'a [u8]> {
        self.data
            .get(start..end)
            .with_context(|| crate::l10n!("err-data-end", offset = start))
    }
}

// --------------------------- writers ---------------------------------------

/// Append a little-endian u16.
pub fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Append a little-endian i16.
pub fn put_i16(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Append a little-endian u32.
pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Append a little-endian i32.
pub fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Append an EET string: u32 UTF-8 byte length + bytes.
pub fn put_eet_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    put_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}

/// Append an SST string: u32 UTF-16LE byte length + bytes.
pub fn put_sst_str(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    put_u32(out, units.len() as u32);
    out.extend_from_slice(&units);
}

/// Append a 4-byte SST signature (latin-1, NUL-padded, truncated to 4).
pub fn put_sig(out: &mut Vec<u8>, sig: &str) {
    let mut buf = [0u8; 4];
    for (i, c) in sig.chars().take(4).enumerate() {
        buf[i] = c as u8;
    }
    out.extend_from_slice(&buf);
}
