//! Lazy, memory-light plugin index for large files.
//!
//! A single **header-only** pass builds an index of every record (signature,
//! FormID, flags, and where its body lives) plus the worldspace → cell →
//! reference hierarchy — **without decompressing or copying any record body**.
//! Interpreting a record's fields (EDID, FULL name, cell coordinates, a
//! reference's base object) is deferred to lazy accessors that decode a single
//! record on demand. Combined with the virtualized tables (only ~50 rows on
//! screen), this keeps a 1.35 GB `Starfield.esm` responsive.

use std::collections::HashMap;

use anyhow::{Result, bail};

use super::games::Game;
use super::reader::{decompress_record, parse_subrecords};
use super::record::{FLAG_COMPRESSED, FLAG_LOCALIZED, SubRecord};

/// Placed-reference record signatures that belong to a cell.
const REF_SIGS: &[&str] = &[
    "REFR", "ACHR", "ACRE", "PGRE", "PHZD", "PMIS", "PARW", "PBAR", "PBEA", "PCON", "PFLA", "PLYR",
];

const HEADER_LEN_MODERN: usize = 24;
const HEADER_LEN_OBLIVION: usize = 20;

/// Above this many records we skip the FormID→record map (base-object type
/// resolution) to keep memory bounded on very large masters; references then
/// show their own signature as Type.
const FORMID_MAP_LIMIT: usize = 400_000;

/// One indexed record: header facts + where its (possibly compressed) body is.
#[derive(Clone)]
pub struct RecEntry {
    pub sig: [u8; 4],
    pub formid: u32,
    pub flags: u32,
    pub body_off: usize,
    pub body_len: u32,
}

impl RecEntry {
    pub fn sig_str(&self) -> String {
        self.sig.iter().map(|&c| c as char).collect()
    }
    pub fn compressed(&self) -> bool {
        self.flags & FLAG_COMPRESSED != 0
    }
}

/// An indexed cell: its record plus its worldspace context (from the GRUP tree).
#[derive(Clone)]
pub struct CellEntry {
    pub rec: usize,
    pub world: Option<u32>,
    pub interior: bool,
}

/// A decoded reference row (built on demand for a selected cell).
#[derive(Clone, Default)]
pub struct RefRow {
    pub formid: u32,
    pub edid: String,
    pub type_sig: String,
    /// Index of the reference's own record (for the detail view).
    pub rec: usize,
}

#[derive(Default)]
struct Ctx {
    world: Option<u32>,
    cell: Option<u32>,
}

pub struct PluginIndex {
    pub game: Game,
    pub masters: Vec<String>,
    pub author: String,
    pub localized: bool,
    pub form_version: Option<u16>,
    pub header_len: usize,
    /// Morrowind (TES3) plugin: subrecord sizes are `u32`, records have no
    /// FormID, and CELL records carry their references inline.
    pub tes3: bool,

    /// Every record, in file order.
    pub records: Vec<RecEntry>,
    /// Non-header record indices (excludes the TES4/TES3 file header).
    pub obj_indices: Vec<usize>,
    /// Record signatures present with counts, sorted by signature.
    pub types: Vec<(String, usize)>,
    /// Per-signature record-index lists, each pre-sorted by FormID.
    pub type_lists: HashMap<String, Vec<usize>>,
    /// All object records sorted by FormID (for the "Tous" FormID ordering).
    pub by_formid_all: Vec<usize>,
    /// All object records grouped by signature then FormID (the "Tous" Type order).
    pub by_type_concat: Vec<usize>,

    /// Worldspace record indices.
    pub worlds: Vec<usize>,
    /// Cells with their worldspace context.
    pub cells: Vec<CellEntry>,
    /// Reference record indices keyed by their parent cell FormID.
    pub refs_by_cell: HashMap<u32, Vec<usize>>,

    /// FormID → record index (base-object type resolution); empty on huge files.
    by_formid: HashMap<u32, usize>,

    /// TES3 only: lowercased object id (`NAME`) → record signature, to resolve the
    /// type of a cell's inline references (which point to objects by id, not FormID).
    tes3_id_type: HashMap<String, String>,
}

fn read_u16(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}
fn read_u32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}
fn sig4(b: &[u8]) -> String {
    b.iter().take(4).map(|&c| c as char).collect()
}

fn detect_header_len(data: &[u8]) -> usize {
    if data.len() >= 24 && &data[20..24] == b"HEDR" {
        HEADER_LEN_OBLIVION
    } else {
        HEADER_LEN_MODERN
    }
}

impl PluginIndex {
    /// Build the index from the whole plugin bytes (typically an mmap).
    pub fn build(data: &[u8]) -> Result<PluginIndex> {
        if data.len() < 16 {
            bail!("file too short to be a plugin");
        }
        let magic = sig4(&data[0..4]);
        let is_tes3 = magic == "TES3";
        let header_len = if is_tes3 { 16 } else { detect_header_len(data) };
        let form_version = if !is_tes3 && header_len == HEADER_LEN_MODERN && data.len() >= 22 {
            Some(read_u16(data, 20))
        } else {
            None
        };

        let mut idx = PluginIndex {
            game: Game::Unknown,
            masters: Vec::new(),
            author: String::new(),
            localized: false,
            form_version,
            header_len,
            tes3: is_tes3,
            records: Vec::new(),
            obj_indices: Vec::new(),
            types: Vec::new(),
            type_lists: HashMap::new(),
            by_formid_all: Vec::new(),
            by_type_concat: Vec::new(),
            worlds: Vec::new(),
            cells: Vec::new(),
            refs_by_cell: HashMap::new(),
            by_formid: HashMap::new(),
            tes3_id_type: HashMap::new(),
        };

        if is_tes3 {
            idx.walk_tes3(data);
        } else {
            let mut ctx = Ctx::default();
            idx.walk(data, 0, data.len(), header_len, 0, &mut ctx);
        }

        // Header facts from the first record (TES4/TES3), which is small and
        // never compressed.
        if let Some(first) = idx.records.first().cloned() {
            let fields = decode_fields(data, &first, is_tes3);
            idx.localized = first.flags & FLAG_LOCALIZED != 0;
            for f in &fields {
                match f.sig.as_str() {
                    "MAST" => idx.masters.push(f.as_zstring()),
                    "CNAM" => idx.author = f.as_zstring(),
                    _ => {}
                }
            }
        }
        idx.game = if is_tes3 {
            Game::Morrowind
        } else {
            Game::detect(&magic, &idx.masters, form_version)
        };

        idx.finish_orderings();
        Ok(idx)
    }

    /// Header-only recursive walk of the TES4-family GRUP tree.
    fn walk(
        &mut self,
        data: &[u8],
        base: usize,
        len: usize,
        header_len: usize,
        depth: u32,
        ctx: &mut Ctx,
    ) {
        if depth > 32 {
            return;
        }
        let end = base + len;
        let mut p = base;
        while p + header_len <= end {
            let is_grup = &data[p..p + 4] == b"GRUP";
            let size = read_u32(data, p + 4) as usize;

            if is_grup {
                if size < header_len || p + size > end {
                    break;
                }
                let gtype = read_u32(data, p + 12);
                let label = read_u32(data, p + 8);
                let mut child = Ctx {
                    world: ctx.world,
                    cell: ctx.cell,
                };
                match gtype {
                    0 => {
                        child.world = None;
                        child.cell = None;
                    }
                    1 => child.world = Some(label),
                    2 | 3 => child.world = None,
                    6 => child.cell = Some(label),
                    8..=10 if child.cell.is_none() => child.cell = Some(label),
                    _ => {}
                }
                self.walk(
                    data,
                    p + header_len,
                    size - header_len,
                    header_len,
                    depth + 1,
                    &mut child,
                );
                p += size;
            } else {
                let flags = read_u32(data, p + 8);
                let formid = read_u32(data, p + 12);
                let sig = [data[p], data[p + 1], data[p + 2], data[p + 3]];
                let body_off = p + header_len;
                if body_off + size > data.len() {
                    break;
                }
                let ri = self.records.len();
                self.records.push(RecEntry {
                    sig,
                    formid,
                    flags,
                    body_off,
                    body_len: size as u32,
                });
                self.classify(ri, &sig, ctx);
                p += header_len + size;
            }
        }
    }

    /// Header-only walk for Morrowind (16-byte headers, no GRUP, no FormID).
    fn walk_tes3(&mut self, data: &[u8]) {
        const HL: usize = 16;
        let mut p = 0usize;
        while p + HL <= data.len() {
            let size = read_u32(data, p + 4) as usize;
            let flags = read_u32(data, p + 12);
            let sig = [data[p], data[p + 1], data[p + 2], data[p + 3]];
            let body_off = p + HL;
            if body_off + size > data.len() {
                break;
            }
            let ri = self.records.len();
            self.records.push(RecEntry {
                sig,
                formid: 0,
                flags,
                body_off,
                body_len: size as u32,
            });
            let s = sig4(&sig);
            if s != "TES3" {
                self.obj_indices.push(ri);
                self.type_lists.entry(s.clone()).or_default().push(ri);
            }
            // Classify cells and index object ids for reference-type resolution.
            let body = &data[body_off..body_off + size];
            if s == "CELL" {
                let fields = parse_subrecords_tes3(body);
                let interior = fields
                    .iter()
                    .find(|f| f.sig == "DATA")
                    .map(|f| {
                        f.data.len() >= 4
                            && u32::from_le_bytes([f.data[0], f.data[1], f.data[2], f.data[3]])
                                & 0x01
                                != 0
                    })
                    .unwrap_or(false);
                self.cells.push(CellEntry {
                    rec: ri,
                    world: None,
                    interior,
                });
            } else if s != "TES3" && s != "INFO" && s != "DIAL" && s != "LAND" && s != "PGRD" {
                // Map the record's id (first `NAME`) to its type, for cell refs.
                if let Some(id) = first_name_tes3(body) {
                    self.tes3_id_type.entry(id.to_lowercase()).or_insert(s);
                }
            }
            p = body_off + size;
        }
    }

    fn classify(&mut self, ri: usize, sig: &[u8; 4], ctx: &Ctx) {
        let s = sig4(sig);
        if s == "WRLD" {
            self.worlds.push(ri);
        } else if s == "CELL" {
            self.cells.push(CellEntry {
                rec: ri,
                world: ctx.world,
                interior: ctx.world.is_none(),
            });
        } else if REF_SIGS.contains(&s.as_str())
            && let Some(cell) = ctx.cell
        {
            self.refs_by_cell.entry(cell).or_default().push(ri);
        }
        if s != "TES4" && s != "TES3" {
            self.obj_indices.push(ri);
            self.type_lists.entry(s).or_default().push(ri);
        }
    }

    /// Build the type list, the pre-sorted per-type and global orderings, and
    /// (for small enough plugins) the FormID→record map.
    fn finish_orderings(&mut self) {
        for list in self.type_lists.values_mut() {
            list.sort_by_key(|&i| self.records[i].formid);
        }
        let mut types: Vec<(String, usize)> = self
            .type_lists
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect();
        types.sort_by(|a, b| a.0.cmp(&b.0));

        // "Tous" grouped by signature then FormID.
        let mut by_type_concat = Vec::with_capacity(self.obj_indices.len());
        for (sig, _) in &types {
            if let Some(list) = self.type_lists.get(sig) {
                by_type_concat.extend_from_slice(list);
            }
        }
        // "Tous" by FormID.
        let mut by_formid_all = self.obj_indices.clone();
        by_formid_all.sort_by_key(|&i| self.records[i].formid);

        self.types = types;
        self.by_type_concat = by_type_concat;
        self.by_formid_all = by_formid_all;

        if self.records.len() <= FORMID_MAP_LIMIT {
            let mut map = HashMap::with_capacity(self.records.len());
            for (i, r) in self.records.iter().enumerate() {
                map.entry(r.formid).or_insert(i);
            }
            self.by_formid = map;
        }
    }

    // ---- lazy accessors -------------------------------------------------

    /// A record's EDID and display name (FULL, or localized string-table id).
    pub fn edid_name(&self, data: &[u8], i: usize, localized: bool) -> (String, String) {
        let fields = decode_fields(data, &self.records[i], self.tes3);
        if self.tes3 {
            return tes3_edid_name(&self.records[i].sig_str(), &fields);
        }
        let edid = fields
            .iter()
            .find(|s| s.sig == "EDID")
            .map(|s| s.as_zstring())
            .unwrap_or_default();
        let name = name_of(&fields, localized);
        (edid, name)
    }

    /// Decode all subrecords of a record on demand (used by the text extractor).
    pub fn record_fields(&self, data: &[u8], i: usize) -> Vec<SubRecord> {
        decode_fields(data, &self.records[i], self.tes3)
    }

    /// The record's raw field bytes, decompressed if needed (for the hex view).
    pub fn record_body(&self, data: &[u8], i: usize) -> Vec<u8> {
        let e = &self.records[i];
        let body = &data[e.body_off..e.body_off + e.body_len as usize];
        if e.compressed() {
            decompress_record(body).unwrap_or_default()
        } else {
            body.to_vec()
        }
    }

    /// A cell's exterior grid coordinates, if any.
    pub fn coords(&self, data: &[u8], i: usize) -> Option<(i32, i32)> {
        let fields = decode_fields(data, &self.records[i], self.tes3);
        if self.tes3 {
            // TES3: the grid lives in the 12-byte CELL `DATA` (flags, gridX, gridY);
            // interior cells (flag 0x01) have no meaningful grid.
            let d = fields.iter().find(|s| s.sig == "DATA")?;
            if d.data.len() < 12
                || u32::from_le_bytes([d.data[0], d.data[1], d.data[2], d.data[3]]) & 0x01 != 0
            {
                return None;
            }
            return Some((
                i32::from_le_bytes([d.data[4], d.data[5], d.data[6], d.data[7]]),
                i32::from_le_bytes([d.data[8], d.data[9], d.data[10], d.data[11]]),
            ));
        }
        let xclc = fields.iter().find(|s| s.sig == "XCLC")?;
        if xclc.data.len() < 8 {
            return None;
        }
        Some((
            i32::from_le_bytes([xclc.data[0], xclc.data[1], xclc.data[2], xclc.data[3]]),
            i32::from_le_bytes([xclc.data[4], xclc.data[5], xclc.data[6], xclc.data[7]]),
        ))
    }

    /// The references of a cell, decoded on demand (EDID, FormID, resolved Type).
    /// For TES3, `key` is the CELL **record index** (Morrowind cells have no
    /// FormID and carry their references inline); for TES4+ it is the cell FormID.
    pub fn cell_refs(&self, data: &[u8], key: u32) -> Vec<RefRow> {
        if self.tes3 {
            return self.cell_refs_tes3(data, key as usize);
        }
        let Some(list) = self.refs_by_cell.get(&key) else {
            return Vec::new();
        };
        list.iter()
            .map(|&i| {
                let e = &self.records[i];
                let fields = decode_fields(data, e, self.tes3);
                let edid = fields
                    .iter()
                    .find(|s| s.sig == "EDID")
                    .map(|s| s.as_zstring())
                    .unwrap_or_default();
                let base = fields
                    .iter()
                    .find(|s| s.sig == "NAME")
                    .and_then(|s| s.as_u32());
                let type_sig = base
                    .and_then(|b| self.by_formid.get(&b))
                    .map(|&bi| self.records[bi].sig_str())
                    .unwrap_or_else(|| e.sig_str());
                RefRow {
                    formid: e.formid,
                    edid,
                    type_sig,
                    rec: i,
                }
            })
            .collect()
    }

    /// TES3 cell references: a CELL record holds its objects inline as repeating
    /// groups that each start with an `FRMR` (reference index) followed by a
    /// `NAME` (the placed object's editor id). We split on `FRMR` and take the
    /// first following `NAME` as the reference's id.
    fn cell_refs_tes3(&self, data: &[u8], rec: usize) -> Vec<RefRow> {
        let Some(e) = self.records.get(rec) else {
            return Vec::new();
        };
        let fields = decode_fields(data, e, true);
        let mut rows: Vec<RefRow> = Vec::new();
        let mut cur: Option<RefRow> = None;
        for s in &fields {
            match s.sig.as_str() {
                "FRMR" => {
                    if let Some(r) = cur.take() {
                        rows.push(r);
                    }
                    cur = Some(RefRow {
                        formid: s.as_u32().unwrap_or(0),
                        edid: String::new(),
                        type_sig: String::new(),
                        rec,
                    });
                }
                "NAME" => {
                    if let Some(r) = cur.as_mut()
                        && r.edid.is_empty()
                    {
                        let id = s.as_zstring();
                        r.type_sig = self
                            .tes3_id_type
                            .get(&id.to_lowercase())
                            .cloned()
                            .unwrap_or_default();
                        r.edid = id;
                    }
                }
                _ => {}
            }
        }
        if let Some(r) = cur.take() {
            rows.push(r);
        }
        rows
    }
}

/// Decode a record's subrecords, decompressing first if needed. Uncompressed
/// records are parsed straight from the mmap slice with no copy. `tes3` selects
/// the Morrowind subrecord layout (`u32` sizes) over the TES4+ one (`u16`).
fn decode_fields(data: &[u8], e: &RecEntry, tes3: bool) -> Vec<SubRecord> {
    let body = &data[e.body_off..e.body_off + e.body_len as usize];
    let parse = |b: &[u8]| {
        if tes3 {
            parse_subrecords_tes3(b)
        } else {
            parse_subrecords(b)
        }
    };
    if e.compressed() {
        // TES3 records are never compressed, but keep the path uniform.
        match decompress_record(body) {
            Ok(d) => parse(&d),
            Err(_) => Vec::new(),
        }
    } else {
        parse(body)
    }
}

/// Parse a Morrowind (TES3) record body into subrecords: `name[4]`, `size[4]`
/// (`u32`), data. No `XXXX` escape and no `u16` sizes (that's the TES4 layout).
fn parse_subrecords_tes3(data: &[u8]) -> Vec<SubRecord> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 8 <= data.len() {
        let sig = sig4(&data[p..p + 4]);
        let size = read_u32(data, p + 4) as usize;
        p += 8;
        if p + size > data.len() {
            break;
        }
        out.push(SubRecord {
            sig,
            data: data[p..p + size].to_vec(),
        });
        p += size;
    }
    out
}

/// TES3 EDID / display-name mapping. Morrowind uses `NAME` for the object id and
/// `FNAM` for the display name; `INFO` (dialogue response) is keyed by `INAM` and
/// its text lives in `NAME`.
fn tes3_edid_name(sig: &str, fields: &[SubRecord]) -> (String, String) {
    let get = |k: &str| {
        fields
            .iter()
            .find(|s| s.sig == k)
            .map(|s| s.as_zstring())
            .unwrap_or_default()
    };
    if sig == "INFO" {
        (get("INAM"), get("NAME"))
    } else {
        (get("NAME"), get("FNAM"))
    }
}

/// Light scan for the first `NAME` subrecord id in a TES3 record body (used to
/// map object ids to their type without decoding every subrecord).
fn first_name_tes3(body: &[u8]) -> Option<String> {
    let mut p = 0usize;
    while p + 8 <= body.len() {
        let is_name = &body[p..p + 4] == b"NAME";
        let size = read_u32(body, p + 4) as usize;
        p += 8;
        if p + size > body.len() {
            break;
        }
        if is_name {
            let d = &body[p..p + size];
            let end = d.iter().position(|&c| c == 0).unwrap_or(d.len());
            return Some(d[..end].iter().map(|&c| c as char).collect());
        }
        p += size;
    }
    None
}

fn name_of(fields: &[SubRecord], localized: bool) -> String {
    let Some(full) = fields.iter().find(|s| s.sig == "FULL") else {
        return String::new();
    };
    if localized {
        match full.as_u32() {
            Some(id) if id != 0 => format!("[lstring {id:08X}]"),
            _ => String::new(),
        }
    } else {
        full.as_zstring()
    }
}

#[cfg(test)]
mod tes3_tests {
    use super::*;

    // Real-data check: build the index from a Morrowind plugin at /tmp and verify
    // TES3 subrecord decoding, EDID/Name mapping, and cell + inline-ref parsing.
    // Skips cleanly when the sample is absent.
    #[test]
    fn tribunal_tes3_index() {
        let Ok(data) = std::fs::read("/tmp/Tribunal.esm") else {
            return;
        };
        let idx = PluginIndex::build(&data).expect("build tes3 index");
        assert!(idx.tes3, "should be detected as TES3");
        assert_eq!(idx.game, Game::Morrowind);

        // EDID/Name populate for a normal object record (find a BOOK).
        let book = idx
            .type_lists
            .get("BOOK")
            .and_then(|v| v.first().copied())
            .expect("a BOOK record");
        let (edid, name) = idx.edid_name(&data, book, false);
        assert!(!edid.is_empty(), "BOOK edid should be non-empty");
        assert!(!name.is_empty(), "BOOK name (FNAM) should be non-empty");

        // Cells are classified, and at least one has inline references.
        assert!(!idx.cells.is_empty(), "cells should be found");
        let with_refs = idx
            .cells
            .iter()
            .map(|c| idx.cell_refs(&data, c.rec as u32).len())
            .max()
            .unwrap_or(0);
        assert!(with_refs > 0, "at least one cell should have references");
    }
}
