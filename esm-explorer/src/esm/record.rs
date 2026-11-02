//! In-memory representation of a parsed Bethesda plugin (TES4-family format:
//! Skyrim, Fallout 3/NV/4/76, Starfield, and Oblivion — the reader adapts its
//! record/GRUP header length to the detected regime, so both the 24-byte modern
//! header and Oblivion's 20-byte header are handled. Morrowind (TES3) is a
//! distinct format with a dedicated parser).
//!
//! Vendored from XIMOD Translator (`src/plugin/record.rs`), unchanged except for
//! the module path of `Game`. Kept API-compatible so this explorer can later be
//! folded into XIMOD Translator by simply pointing at its parser.

/// Record header flag: the record's data is zlib-compressed.
pub const FLAG_COMPRESSED: u32 = 0x0004_0000;

/// TES4 header flag (on the top TES4 record): strings are localized (stored in
/// external STRINGS/DLSTRINGS/ILSTRINGS files as 4-byte ids).
pub const FLAG_LOCALIZED: u32 = 0x0000_0080;

/// One subrecord (field) inside a record.
#[derive(Debug, Clone)]
pub struct SubRecord {
    /// 4-character signature (e.g. "FULL", "EDID").
    pub sig: String,
    /// Raw field data (already decompressed if the record was compressed).
    pub data: Vec<u8>,
}

impl SubRecord {
    /// Decode the data as a NUL-terminated zstring (UTF-8, falling back to
    /// Latin-1 for the classic Windows-1252 plugins). Trailing NUL removed.
    pub fn as_zstring(&self) -> String {
        let end = self
            .data
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.data.len());
        let slice = &self.data[..end];
        match std::str::from_utf8(slice) {
            Ok(s) => s.to_string(),
            Err(_) => slice.iter().map(|&b| b as char).collect(),
        }
    }

    /// Interpret the data as a little-endian u32 (used for localized string ids).
    pub fn as_u32(&self) -> Option<u32> {
        if self.data.len() >= 4 {
            Some(u32::from_le_bytes([
                self.data[0],
                self.data[1],
                self.data[2],
                self.data[3],
            ]))
        } else {
            None
        }
    }
}

/// One record and its parsed subrecords.
#[derive(Debug, Clone)]
pub struct Record {
    pub sig: String,
    pub formid: u32,
    pub flags: u32,
    pub subrecords: Vec<SubRecord>,
}

impl Record {
    /// The record's editor id (EDID subrecord), if present.
    pub fn editor_id(&self) -> Option<String> {
        self.subrecords
            .iter()
            .find(|s| s.sig == "EDID")
            .map(|s| s.as_zstring())
    }

    /// The first subrecord with the given signature, if present.
    pub fn field(&self, sig: &str) -> Option<&SubRecord> {
        self.subrecords.iter().find(|s| s.sig == sig)
    }
}

/// Parsed plugin: its top record flags plus every non-GRUP record found.
#[derive(Debug, Clone, Default)]
pub struct Plugin {
    /// Master files this plugin depends on (from TES4 MAST subrecords, in order).
    pub masters: Vec<String>,
    /// Whether the plugin uses localized (external) strings.
    pub localized: bool,
    /// Author string (TES4 CNAM), if any.
    pub author: String,
    /// The game automatically detected from the plugin (magic + masters + form
    /// version). Drives the translatable-field set — the user never selects a
    /// game manually.
    pub game: super::games::Game,
    /// TES4 "form version" (`u16` at offset 20 of a record header); `None` for
    /// Oblivion (20-byte header) and TES3.
    pub form_version: Option<u16>,
    /// Record/GRUP header length actually used to parse this plugin (20 or 24).
    pub header_len: usize,
    /// All records (GRUP containers themselves are not kept, only their records).
    pub records: Vec<Record>,
}
