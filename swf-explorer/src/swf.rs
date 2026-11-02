//! Minimal SWF container: read the header, decompress the body, walk the tag
//! stream **preserving every tag's raw bytes**, and re-emit a valid SWF.
//!
//! The round-trip philosophy is deliberately conservative: we never re-serialize
//! tags we don't touch. Each tag keeps its exact payload bytes and its original
//! header form (short vs long length), so a load→save with no edits is
//! byte-identical inside the body. Only the tags whose text we change are
//! rewritten, and only their length header is recomputed. This avoids the huge
//! surface of fully modelling every SWF tag type.
//!
//! Containers & compression, fully supported:
//! - **Flash**: `FWS` (uncompressed) / `CWS` (zlib) — Skyrim / Fallout.
//! - **Scaleform GFx**: `GFX` (uncompressed) / `CFX` (zlib) — Starfield. Same tag
//!   stream as SWF, plus Scaleform tags (codes ≥ 1000) that we preserve verbatim.
//!
//! `ZWS` (Flash LZMA) is detected and reported as unsupported (re-save as zlib).

use std::io::{Read, Write};

use anyhow::{Result, bail};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};

/// Which container family holds the tag stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// Flash SWF (`FWS`/`CWS`).
    Flash,
    /// Scaleform GFx (`GFX`/`CFX`).
    Gfx,
}

/// How the body is stored on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packing {
    /// Uncompressed (`FWS` / `GFX`).
    None,
    /// zlib-compressed body (`CWS` / `CFX`).
    Zlib,
}

/// One SWF tag: its code and its raw payload bytes (header excluded).
#[derive(Debug, Clone)]
pub struct Tag {
    /// Tag type code (e.g. 37 = DefineEditText, 82 = DoABC, 12 = DoAction).
    pub code: u16,
    /// Raw payload bytes, exactly as stored (never re-serialized unless edited).
    pub body: Vec<u8>,
    /// Whether the original used the long (6-byte) header even though it fits in
    /// the short form — preserved so unedited tags round-trip byte-for-byte.
    pub force_long: bool,
}

/// A parsed SWF: version, packing, the raw pre-tag header bytes (frame size RECT
/// + frame rate + frame count), and the tag list.
#[derive(Debug, Clone)]
pub struct Swf {
    pub version: u8,
    pub container: Container,
    pub packing: Packing,
    /// Bytes from the start of the (uncompressed) body up to the first tag:
    /// the frame-size `RECT`, frame rate and frame count. Preserved verbatim.
    pub header_rest: Vec<u8>,
    pub tags: Vec<Tag>,
}

impl Swf {
    /// A short human label of the container/packing (e.g. `"GFX Scaleform (zlib)"`).
    pub fn kind_label(&self) -> &'static str {
        match (self.container, self.packing) {
            (Container::Flash, Packing::None) => "SWF (non compressé)",
            (Container::Flash, Packing::Zlib) => "SWF (zlib)",
            (Container::Gfx, Packing::None) => "GFX Scaleform (non compressé)",
            (Container::Gfx, Packing::Zlib) => "CFX Scaleform (zlib)",
        }
    }

    /// Parse a SWF or Scaleform GFx from disk bytes.
    pub fn parse(bytes: &[u8]) -> Result<Swf> {
        if bytes.len() < 8 {
            bail!("fichier trop court pour un SWF/GFX");
        }
        let sig = &bytes[0..3];
        let version = bytes[3];
        let (container, packing) = match sig {
            b"FWS" => (Container::Flash, Packing::None),
            b"CWS" => (Container::Flash, Packing::Zlib),
            b"GFX" => (Container::Gfx, Packing::None),
            b"CFX" => (Container::Gfx, Packing::Zlib),
            b"ZWS" => bail!(
                "SWF compressé en LZMA (ZWS) non pris en charge par cette version — \
                 ré-enregistrez en zlib (CWS) ou non compressé (FWS)"
            ),
            _ => bail!("signature inconnue (ni FWS/CWS/ZWS, ni GFX/CFX)"),
        };

        let body = match packing {
            Packing::None => bytes[8..].to_vec(),
            Packing::Zlib => {
                let mut out = Vec::new();
                ZlibDecoder::new(&bytes[8..])
                    .read_to_end(&mut out)
                    .map_err(|e| anyhow::anyhow!("décompression zlib du SWF : {e}"))?;
                out
            }
        };

        // The body starts with the frame-size RECT, then frame rate (u16) and
        // frame count (u16). We keep those bytes verbatim.
        let rect_len = rect_byte_len(&body)?;
        let first_tag = rect_len
            .checked_add(4)
            .filter(|&n| n <= body.len())
            .ok_or_else(|| anyhow::anyhow!("en-tête SWF incohérent"))?;
        let header_rest = body[..first_tag].to_vec();
        let tags = parse_tags(&body[first_tag..])?;

        Ok(Swf {
            version,
            container,
            packing,
            header_rest,
            tags,
        })
    }

    /// Re-emit the SWF to disk bytes, re-compressing as the original packing.
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut body = self.header_rest.clone();
        for t in &self.tags {
            write_tag_header(&mut body, t.code, t.body.len(), t.force_long)?;
            body.extend_from_slice(&t.body);
        }
        let file_length = 8u32
            .checked_add(u32::try_from(body.len()).map_err(|_| anyhow::anyhow!("SWF trop grand"))?)
            .ok_or_else(|| anyhow::anyhow!("SWF trop grand"))?;

        let mut out = Vec::with_capacity(body.len());
        out.extend_from_slice(match (self.container, self.packing) {
            (Container::Flash, Packing::None) => b"FWS",
            (Container::Flash, Packing::Zlib) => b"CWS",
            (Container::Gfx, Packing::None) => b"GFX",
            (Container::Gfx, Packing::Zlib) => b"CFX",
        });
        out.push(self.version);
        out.extend_from_slice(&file_length.to_le_bytes());
        match self.packing {
            Packing::None => out.extend_from_slice(&body),
            Packing::Zlib => {
                let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
                enc.write_all(&body)
                    .map_err(|e| anyhow::anyhow!("compression zlib : {e}"))?;
                let comp = enc
                    .finish()
                    .map_err(|e| anyhow::anyhow!("compression zlib : {e}"))?;
                out.extend_from_slice(&comp);
            }
        }
        Ok(out)
    }
}

/// Byte length of the leading `RECT` (frame size): 5 bits for the nbits field,
/// then `4 * nbits` bits, rounded up to whole bytes.
fn rect_byte_len(body: &[u8]) -> Result<usize> {
    if body.is_empty() {
        bail!("corps SWF vide");
    }
    let nbits = (body[0] >> 3) as usize;
    Ok((5 + nbits * 4).div_ceil(8))
}

/// Parse the tag stream into `(code, body, force_long)` entries, stopping after
/// the `End` tag (code 0).
fn parse_tags(data: &[u8]) -> Result<Vec<Tag>> {
    let mut tags = Vec::new();
    let mut p = 0usize;
    while p + 2 <= data.len() {
        let hdr = u16::from_le_bytes([data[p], data[p + 1]]);
        p += 2;
        let code = hdr >> 6;
        let short_len = (hdr & 0x3F) as usize;
        let (len, force_long) = if short_len == 0x3F {
            if p + 4 > data.len() {
                bail!("tag SWF tronqué (longueur longue)");
            }
            let l = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) as usize;
            p += 4;
            (l, false)
        } else {
            (short_len, false)
        };
        if p + len > data.len() {
            bail!("tag SWF tronqué (code {code}, longueur {len})");
        }
        // A short_len < 0x3F stored in long form can't be distinguished here (the
        // reader collapses it); real files never do that, so `force_long` is only
        // ever set when a genuinely long body is used. Detect the "would fit short
        // but used long" case: if len < 0x3F but we read the long form.
        let used_long = short_len == 0x3F;
        let force_long = force_long || (used_long && len < 0x3F);
        tags.push(Tag {
            code,
            body: data[p..p + len].to_vec(),
            force_long,
        });
        p += len;
        if code == 0 {
            break;
        }
    }
    Ok(tags)
}

/// Write a tag record header (code + length), using the long form when the body
/// is ≥ 63 bytes or when `force_long` preserves the original encoding.
fn write_tag_header(out: &mut Vec<u8>, code: u16, len: usize, force_long: bool) -> Result<()> {
    if len >= 0x3F || force_long {
        let hdr = (code << 6) | 0x3F;
        out.extend_from_slice(&hdr.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(len)
                .map_err(|_| anyhow::anyhow!("tag trop grand"))?
                .to_le_bytes(),
        );
    } else {
        let hdr = (code << 6) | (len as u16);
        out.extend_from_slice(&hdr.to_le_bytes());
    }
    Ok(())
}
