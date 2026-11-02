//! Bethesda plugin (.esp/.esm/.esl) reader.
//!
//! Walks the TES4-family binary layout — top-level records and (possibly
//! nested) GRUP containers — parsing each record's subrecords, transparently
//! zlib-decompressing compressed records and honouring the `XXXX` large-field
//! escape. From the parsed records it extracts the translatable strings into the
//! common translation-memory model.

use std::io::Read;
use std::path::Path;

use anyhow::{Result, bail};
use flate2::read::ZlibDecoder;

use memmap2::Mmap;

use super::record::{FLAG_COMPRESSED, FLAG_LOCALIZED, Plugin, Record, SubRecord};
use crate::games::{FieldCtx, FieldMode, FieldTable, Game, registry};
use crate::tm::{TmEntry, TmMeta, TranslationMemory};

/// Modern TES4 record/GRUP header length (Skyrim, Fallout, Starfield).
const HEADER_LEN_MODERN: usize = 24;
/// Oblivion's TES4 record/GRUP header length (no form-version field).
const HEADER_LEN_OBLIVION: usize = 20;

fn read_u16(d: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([d[p], d[p + 1]])
}
fn read_u32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

/// Determine the record/GRUP header length by locating the TES4 record's first
/// subrecord, which is always `HEDR`. In a modern plugin it sits at offset 24, in
/// an Oblivion plugin at offset 20 (the header lacks the 4-byte form-version /
/// version-control tail). This makes the 20-vs-24 choice independently of any
/// game selection — the whole point being that detection is automatic.
pub(crate) fn detect_header_len(data: &[u8]) -> usize {
    if data.len() >= 24 && &data[20..24] == b"HEDR" {
        HEADER_LEN_OBLIVION
    } else {
        // Modern layout (HEDR at 24), and the safe default for anything else.
        HEADER_LEN_MODERN
    }
}

/// Parse a whole plugin from bytes, detecting the format regime and game.
pub fn parse(data: &[u8]) -> Result<Plugin> {
    let mut plugin = Plugin::default();
    if data.len() < 16 {
        bail!(crate::l10n!("err-plugin-short"));
    }

    let magic = sig4(&data[0..4]);
    if magic == "TES3" {
        // Morrowind is a fundamentally different container (16-byte headers,
        // 4-byte subrecord sizes, no GRUP, no FormID): hand it to the TES3 parser.
        return super::tes3::parse_tes3(data);
    }

    let header_len = detect_header_len(data);
    plugin.header_len = header_len;
    // The form version lives at offset 20 of a record header — only present in
    // the modern 24-byte layout (absent from Oblivion's 20-byte header).
    let form_version = if header_len == HEADER_LEN_MODERN && data.len() >= 22 {
        Some(read_u16(data, 20))
    } else {
        None
    };
    plugin.form_version = form_version;

    walk(data, 0, header_len, &mut plugin)?;

    // Now that the masters are known, resolve the game automatically.
    plugin.game = Game::detect(&magic, &plugin.masters, form_version);
    Ok(plugin)
}

/// Recursively walk a slice of top-level entries (whole file, or a GRUP's body).
fn walk(data: &[u8], depth: u32, header_len: usize, plugin: &mut Plugin) -> Result<()> {
    if depth > 32 {
        bail!(crate::l10n!("err-plugin-nesting"));
    }
    let mut p = 0usize;
    while p + header_len <= data.len() {
        let sig = sig4(&data[p..p + 4]);
        let size = read_u32(data, p + 4) as usize;

        if sig == "GRUP" {
            // GRUP size includes the header (same length as a record header).
            let total = size;
            if total < header_len || p + total > data.len() {
                break;
            }
            let body = &data[p + header_len..p + total];
            walk(body, depth + 1, header_len, plugin)?;
            p += total;
        } else {
            // Record: size is the data length after the header.
            let data_start = p + header_len;
            let data_end = data_start + size;
            if data_end > data.len() {
                break;
            }
            let flags = read_u32(data, p + 8);
            let formid = read_u32(data, p + 12);
            let raw = &data[data_start..data_end];

            let field_bytes = if flags & FLAG_COMPRESSED != 0 {
                decompress_record(raw)?
            } else {
                raw.to_vec()
            };
            let subrecords = parse_subrecords(&field_bytes);
            let record = Record {
                sig: sig.clone(),
                formid,
                flags,
                subrecords,
            };

            if sig == "TES4" {
                plugin.localized = flags & FLAG_LOCALIZED != 0;
                for s in &record.subrecords {
                    match s.sig.as_str() {
                        "MAST" => plugin.masters.push(s.as_zstring()),
                        "CNAM" => plugin.author = s.as_zstring(),
                        _ => {}
                    }
                }
            }

            plugin.records.push(record);
            p += header_len + size;
        }
    }
    Ok(())
}

/// Decompress a compressed record body: u32 decompressed size + zlib stream.
pub(crate) fn decompress_record(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.len() < 4 {
        bail!(crate::l10n!("err-plugin-compressed"));
    }
    let decomp_size = read_u32(raw, 0) as usize;
    let mut out = Vec::with_capacity(decomp_size);
    ZlibDecoder::new(&raw[4..]).read_to_end(&mut out)?;
    Ok(out)
}

/// Parse a record's field stream into subrecords, honouring the `XXXX` escape
/// used when a field is larger than a u16.
pub(crate) fn parse_subrecords(data: &[u8]) -> Vec<SubRecord> {
    let mut out = Vec::new();
    let mut p = 0usize;
    let mut override_size: Option<usize> = None;
    while p + 6 <= data.len() {
        let sig = sig4(&data[p..p + 4]);
        let declared = read_u16(data, p + 4) as usize;
        p += 6;
        if sig == "XXXX" {
            // The 4-byte payload is the real size of the next subrecord.
            if p + 4 <= data.len() {
                override_size = Some(read_u32(data, p) as usize);
            }
            p += declared;
            continue;
        }
        let real = override_size.take().unwrap_or(declared);
        if p + real > data.len() {
            break;
        }
        out.push(SubRecord {
            sig,
            data: data[p..p + real].to_vec(),
        });
        p += real;
    }
    out
}

pub(crate) fn sig4(b: &[u8]) -> String {
    b.iter().take(4).map(|&c| c as char).collect()
}

/// Extract translatable strings from a parsed plugin into a translation memory.
///
/// For non-localized plugins the text is read inline. For localized plugins the
/// field holds a 4-byte string id (stored in `idstexte`); the actual text lives
/// in external STRINGS files and is left empty here.
pub fn extract_strings(plugin: &Plugin, plugin_name: &str) -> TranslationMemory {
    // Morrowind uses a wholly different record layout and matching (no FormID,
    // CELL name nuance); delegate to the TES3 extractor.
    if plugin.game == Game::Morrowind {
        return super::tes3::extract_tes3(plugin, plugin_name);
    }
    let mut entries = Vec::new();
    let mut unread = Vec::new();
    let reg = registry();
    let fields = reg.fields(plugin.game);
    for rec in &plugin.records {
        let edid = rec.editor_id().unwrap_or_default();
        push_record_entries(
            fields,
            plugin.localized,
            &rec.sig,
            rec.formid,
            &edid,
            &rec.subrecords,
            &mut Out {
                entries: &mut entries,
                unread: &mut unread,
            },
        );
    }

    let meta = TmMeta {
        // The precisely detected game (Skyrim LE vs SE, etc.), not just the
        // master-name guess — this feeds the UI's game-dependent behaviour.
        game: plugin.game.id().to_string(),
        masters: plugin.masters.clone(),
        source_format: "plugin".to_string(),
        plugin_name: plugin_name.to_string(),
        vmad_unread: unread,
        ..Default::default()
    };
    TranslationMemory::new(entries, meta)
}

/// Where the entries of the records go, with the VMAD blocks not read
/// (C-08): `(record, reason)`.
struct Out<'a> {
    entries: &'a mut Vec<TmEntry>,
    unread: &'a mut Vec<(String, String)>,
}

/// The lines of a VMAD block (C-08): one per string slot, field `VMAD`,
/// index = rank of the string in the block, comment `script.property`.
/// They come locked (« VMAD désactivé », D-63) until the file's switch
/// unlocks them; empty strings make no line but keep their rank.
fn push_vmad_entries(rec_sig: &str, formid: u32, edid: &str, data: &[u8], out: &mut Out) {
    let slots = match super::vmad::parse(rec_sig, data) {
        Ok(s) => s,
        Err(why) => {
            let who = if edid.is_empty() {
                format!("{rec_sig} {formid:08X}")
            } else {
                format!("{rec_sig} {edid}")
            };
            out.unread.push((who, why.to_string()));
            return;
        }
    };
    for (i, slot) in slots.iter().enumerate() {
        let source = super::vmad::text(data, slot);
        if source.is_empty() {
            continue;
        }
        let mut entry = TmEntry {
            formid,
            edid: edid.to_string(),
            rec_sig: rec_sig.to_string(),
            field_sig: crate::vmad_lock::FIELD.to_string(),
            index: i as i32 + 1,
            source,
            idstexte: -1,
            icon: -1,
            comment: crate::l10n!("vmad-comment", script = slot.script, prop = slot.prop),
            ..Default::default()
        };
        crate::vmad_lock::lock_file(&mut entry);
        out.entries.push(entry);
    }
}

/// The lines of a script's messages (C-07): one per segment (message,
/// button), field `SCTX`, index = rank in the record (its scripts in
/// order); a literal not found in the compiled script is locked
/// « compilé introuvable ».
fn push_sctx_entries(
    rec_sig: &str,
    formid: u32,
    edid: &str,
    sctx: &[u8],
    scda: Option<&[u8]>,
    rank: &mut i32,
    out: &mut Out,
) {
    let a = super::sctx::analyze(sctx, scda);
    for seg in &a.segments {
        *rank += 1;
        let lit = &a.literals[seg.literal];
        let mut comment = match seg.button {
            Some(b) => crate::l10n!(
                "sctx-comment-button",
                instr = lit.instr,
                n = b,
                m = seg.message
            ),
            None => crate::l10n!("sctx-comment", instr = lit.instr, n = seg.message),
        };
        if let Some(c) = a.literals[seg.literal..]
            .iter()
            .rev()
            .chain(a.literals[..seg.literal].iter().rev())
            .find(|l| l.line == lit.line && l.rank == 0)
            .and_then(|l| l.comment.as_ref())
            .filter(|c| !c.is_empty())
        {
            comment = format!("{comment} — ; {}", crate::tm::encoding::decode(c, false));
        }
        let mut entry = TmEntry {
            formid,
            edid: edid.to_string(),
            rec_sig: rec_sig.to_string(),
            field_sig: "SCTX".to_string(),
            index: *rank,
            source: crate::tm::encoding::decode(&seg.bytes, false),
            idstexte: -1,
            icon: -1,
            comment,
            ..Default::default()
        };
        if lit.compiled.is_none() {
            crate::status::set(&mut entry, crate::status::Status::Ignored);
            entry.prov = format!("{}compiled", crate::vmad_lock::LOCK);
        }
        out.entries.push(entry);
    }
}

/// Build the translation-memory entries of a single record. This is the one
/// source of truth for entry construction, shared by [`extract_strings`] (which
/// works from a fully parsed [`Plugin`]) and the streaming reader
/// [`extract_streaming`] (which decodes records on the fly). Any change here
/// applies to both, so their output stays identical.
fn push_record_entries(
    fields: &FieldTable,
    localized: bool,
    rec_sig: &str,
    formid: u32,
    edid: &str,
    subrecords: &[SubRecord],
    out: &mut Out,
) {
    // Per-field 1-based instance counter within this record.
    let mut counters: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    // Another field of the record, for `fieldEquals` conditions (A-06).
    let other = |f: &str| {
        subrecords
            .iter()
            .find(|s| s.sig == f)
            .map(|s| s.as_zstring())
    };
    // Object templates (Fallout 4, Fallout 76, Starfield): after `OBTE` and
    // until `STOP`, the record carries one combination per template —
    // `[OBTF] FULL OBTS` — whose FULL is the template's name. A template
    // flagged « Editor Only » (`OBTF` present) is never shown in game; one
    // without the flag (legendary uniques) names the item and stays
    // translatable.
    let mut in_templates = false;
    let mut editor_only = false;
    // The script blocks of the record (C-07): SCHR, SCDA, SCTX.
    let sigs: Vec<&str> = subrecords.iter().map(|s| s.sig.as_str()).collect();
    let script_blocks = super::sctx::blocks(&sigs);
    let mut sctx_rank = 0i32;
    for (k, sub) in subrecords.iter().enumerate() {
        match sub.sig.as_str() {
            "OBTE" => {
                in_templates = true;
                editor_only = false;
            }
            "STOP" => in_templates = false,
            "OBTF" if in_templates => editor_only = true,
            "OBTS" if in_templates => editor_only = false,
            _ => {}
        }
        // The translatable-field table follows the automatically detected game
        // (A-06): fields it does not list are not even decoded.
        if fields.limits(rec_sig, &sub.sig).is_none() {
            continue;
        }
        // The scripts' strings (C-08): inline even in a localized plugin.
        if sub.sig == crate::vmad_lock::FIELD {
            push_vmad_entries(rec_sig, formid, edid, &sub.data, out);
            continue;
        }
        // The messages of a script before Papyrus (C-07).
        if sub.sig == "SCTX" {
            let scda = script_blocks
                .iter()
                .find(|b| b.2 == k)
                .and_then(|b| b.1)
                .map(|i| subrecords[i].data.as_slice());
            push_sctx_entries(rec_sig, formid, edid, &sub.data, scda, &mut sctx_rank, out);
            continue;
        }
        let template_name = in_templates && sub.sig == "FULL";
        let template_unused = template_name && editor_only;
        let mut entry = TmEntry {
            formid,
            edid: edid.to_string(),
            rec_sig: rec_sig.to_string(),
            field_sig: sub.sig.clone(),
            idstexte: -1,
            icon: -1,
            ..Default::default()
        };
        // The TES4 header's author (CNAM) and description (SNAM) are always
        // stored inline, even in a localized plugin — they are not lstrings.
        let inline_field = rec_sig == "TES4";
        if localized && !inline_field {
            match sub.as_u32() {
                // Identifier 0 is the engine's empty string: nothing to
                // translate, as an empty inline field (C-09).
                Some(0) => continue,
                Some(id) => entry.idstexte = id as i32,
                None => continue,
            }
        } else {
            entry.source = sub.as_zstring();
            // A subrecord that decodes to nothing is not translatable text
            // (a field that happens to share a signature but holds binary data in
            // this game). Skip it rather than emit noise.
            if entry.source.is_empty() {
                continue;
            }
        }
        // Mode in context (GMST only for 's…' settings, `LOC_*` keys locked…).
        let ctx = FieldCtx {
            edid,
            value: &entry.source,
            other: &other,
        };
        match fields.mode(rec_sig, &sub.sig, &ctx) {
            FieldMode::Ignore => continue,
            FieldMode::Locked => crate::status::set(&mut entry, crate::status::Status::Ignored),
            FieldMode::Unused => {
                crate::status::set(&mut entry, crate::status::Status::Ignored);
                entry.unused = true;
            }
            FieldMode::Translate => {}
        }
        if template_unused {
            // Editor-only object template: never shown in game.
            crate::status::set(&mut entry, crate::status::Status::Ignored);
            entry.unused = true;
        } else if template_name && entry.comment.is_empty() {
            entry.comment = crate::l10n!("comment-template-name");
        }

        // Assign the 1-based instance index only to entries we keep.
        let idx = counters.entry(sub.sig.clone()).or_insert(0);
        *idx += 1;
        entry.index = *idx;

        out.entries.push(entry);
    }
}

/// Streaming extraction: walk the plugin **once** and build the translation
/// memory directly, **skipping records whose type carries no translatable text**
/// (REFR, NAVM, cell children…) — the bulk of a large plugin. Those are never
/// decompressed or field-parsed, which is what makes a 1.35 GB Starfield.esm load
/// quickly. Output is byte-for-byte identical to `parse` + [`extract_strings`].
pub(crate) fn extract_streaming(data: &[u8], plugin_name: &str) -> Result<TranslationMemory> {
    extract_streaming_as(data, plugin_name, None)
}

/// [`extract_streaming`] with the game imposed rather than detected (E-12:
/// « recharger en tant que <jeu> »): the field table of `game_override`
/// decides what is extracted, and the memory is tagged with it.
pub(crate) fn extract_streaming_as(
    data: &[u8],
    plugin_name: &str,
    game_override: Option<Game>,
) -> Result<TranslationMemory> {
    if data.len() < 16 {
        bail!(crate::l10n!("err-plugin-short"));
    }
    let magic = sig4(&data[0..4]);
    if magic == "TES3" {
        // Morrowind is small and has a wholly different layout; reuse the direct
        // parser (no lazy skipping needed).
        let plugin = super::tes3::parse_tes3(data)?;
        return Ok(super::tes3::extract_tes3(&plugin, plugin_name));
    }

    let header_len = detect_header_len(data);
    let form_version = if header_len == HEADER_LEN_MODERN && data.len() >= 22 {
        Some(read_u16(data, 20))
    } else {
        None
    };

    // Phase A: read the TES4 header (record at offset 0) for masters + the
    // localized flag, so the game can be detected before the main walk.
    let hdr_flags = read_u32(data, 8);
    let localized = hdr_flags & FLAG_LOCALIZED != 0;
    let hdr_size = read_u32(data, 4) as usize;
    let mut masters = Vec::new();
    if header_len + hdr_size <= data.len() {
        for s in parse_subrecords(&data[header_len..header_len + hdr_size]) {
            if s.sig == "MAST" {
                masters.push(s.as_zstring());
            }
        }
    }
    let game = match game_override {
        Some(g) if g != Game::Unknown => g,
        _ => Game::detect(&magic, &masters, form_version),
    };
    let reg = registry();
    let fields = reg.fields(game);

    // Phase B: walk everything, decoding only translatable record types.
    let mut entries = Vec::new();
    let mut unread = Vec::new();
    walk_extract(
        data,
        0,
        header_len,
        fields,
        localized,
        &mut Out {
            entries: &mut entries,
            unread: &mut unread,
        },
    )?;

    let meta = TmMeta {
        game: game.id().to_string(),
        masters,
        source_format: "plugin".to_string(),
        plugin_name: plugin_name.to_string(),
        vmad_unread: unread,
        ..Default::default()
    };
    Ok(TranslationMemory::new(entries, meta))
}

/// Recursive walk for [`extract_streaming`]: descend GRUPs, and for each record
/// decode + emit entries only when its signature can hold translatable text.
fn walk_extract(
    d: &[u8],
    depth: u32,
    header_len: usize,
    fields: &FieldTable,
    localized: bool,
    out: &mut Out,
) -> Result<()> {
    if depth > 32 {
        bail!(crate::l10n!("err-plugin-nesting"));
    }
    let mut p = 0usize;
    while p + header_len <= d.len() {
        let sig = sig4(&d[p..p + 4]);
        let size = read_u32(d, p + 4) as usize;

        if sig == "GRUP" {
            let total = size;
            if total < header_len || p + total > d.len() {
                break;
            }
            walk_extract(
                &d[p + header_len..p + total],
                depth + 1,
                header_len,
                fields,
                localized,
                out,
            )?;
            p += total;
        } else {
            let data_start = p + header_len;
            let data_end = data_start + size;
            if data_end > d.len() {
                break;
            }
            // Skip whole records whose type carries no translatable text — the
            // key optimization: no decompression, no field parsing for them.
            let flags = read_u32(d, p + 8);
            let raw = &d[data_start..data_end];
            if fields.record_may_have_fields(&sig)
                || may_have_any_record_field(&sig, flags, raw, fields)
            {
                let formid = read_u32(d, p + 12);
                let subrecords = if flags & FLAG_COMPRESSED != 0 {
                    parse_subrecords(&decompress_record(raw)?)
                } else {
                    parse_subrecords(raw)
                };
                let edid = subrecords
                    .iter()
                    .find(|s| s.sig == "EDID")
                    .map(|s| s.as_zstring())
                    .unwrap_or_default();
                push_record_entries(fields, localized, &sig, formid, &edid, &subrecords, out);
            }
            p += header_len + size;
        }
    }
    Ok(())
}

/// Records whose type has no field of its own may still carry a joker
/// field (`*:VMAD`, C-08): an uncompressed record is scanned for it without
/// decoding; a compressed one is decoded, except the bulky types that never
/// carry scripts (landscape, navigation meshes).
fn may_have_any_record_field(sig: &str, flags: u32, raw: &[u8], fields: &FieldTable) -> bool {
    let any = fields.any_record_fields();
    if any.is_empty() {
        return false;
    }
    if flags & FLAG_COMPRESSED != 0 {
        return !matches!(sig, "LAND" | "NAVM" | "NAVI");
    }
    let mut p = 0usize;
    let mut big: Option<usize> = None;
    while p + 6 <= raw.len() {
        let s = &raw[p..p + 4];
        let declared = read_u16(raw, p + 4) as usize;
        p += 6;
        if s == b"XXXX" {
            if p + 4 <= raw.len() {
                big = Some(read_u32(raw, p) as usize);
            }
            p += declared;
            continue;
        }
        if any.iter().any(|f| f.as_bytes() == s) {
            return true;
        }
        p += big.take().unwrap_or(declared);
    }
    false
}

/// Convenience: read a plugin file and extract its translatable strings, using
/// the file name as the plugin name. The file is **memory-mapped** and walked
/// once, skipping non-translatable record types (see [`extract_streaming`]).
pub fn read_file(path: &Path) -> Result<TranslationMemory> {
    read_file_as(path, None)
}

/// [`read_file_as`] with the bytes of the strings read in `codec` (D-09):
/// forced, the page decodes everything; otherwise it replaces the
/// Windows-1252 fallback after the UTF-8 test.
pub fn read_file_with(
    path: &Path,
    game: Option<Game>,
    codec: Option<crate::tm::encoding::Codec>,
) -> Result<TranslationMemory> {
    crate::tm::encoding::with_input(codec, || read_file_as(path, game))
}

/// [`read_file`] with the game imposed (E-12, « recharger en tant que »);
/// `None` or `Unknown` means detected from the file.
pub fn read_file_as(path: &Path, game: Option<Game>) -> Result<TranslationMemory> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let file = std::fs::File::open(path)?;
    // SAFETY: read-only mapping; the map stays valid after the File is dropped.
    let mmap = unsafe { Mmap::map(&file)? };
    // The file's magic picks the engine (TES3 / TES4); see `crate::engine`.
    crate::engine::for_data(&mmap[..]).read_as(&mmap[..], &name, game)
}

#[cfg(test)]
mod template_tests {
    use super::*;
    use crate::plugin::record::SubRecord;

    fn sub(sig: &str, text: &str) -> SubRecord {
        let mut data = text.as_bytes().to_vec();
        if !text.is_empty() {
            data.push(0);
        }
        SubRecord {
            sig: sig.into(),
            data,
        }
    }

    /// Object templates (Fallout 4 / Starfield): the editor-only template
    /// names are « never shown in game », the unique's name stays
    /// translatable, the item name is untouched.
    #[test]
    fn editor_only_template_names_are_unused() {
        let fields = ximod_games::fields::FieldTable::builtin(ximod_games::Game::Starfield);
        let subrecords = vec![
            sub("EDID", "LaserRifle"),
            sub("FULL", "Laser Rifle"),
            sub("OBTE", "\u{2}"),
            sub("OBTF", ""),
            sub("FULL", "Laser Rifle - Scoped"),
            sub("OBTS", "x"),
            sub("FULL", "Keelhauler"),
            sub("OBTS", "x"),
            sub("STOP", ""),
            sub("DESC", "A rifle."),
        ];
        let mut out = Vec::new();
        let mut unread = Vec::new();
        push_record_entries(
            &fields,
            false,
            "WEAP",
            0x1234,
            "LaserRifle",
            &subrecords,
            &mut Out {
                entries: &mut out,
                unread: &mut unread,
            },
        );
        let fulls: Vec<(&str, bool, &str)> = out
            .iter()
            .filter(|e| e.field_sig == "FULL")
            .map(|e| (e.source.as_str(), e.unused, e.comment.as_str()))
            .collect();
        assert_eq!(
            fulls,
            vec![
                ("Laser Rifle", false, ""),
                ("Laser Rifle - Scoped", true, ""),
                ("Keelhauler", false, "⟦comment-template-name⟧"),
            ]
        );
        assert!(out.iter().any(|e| e.field_sig == "DESC" && !e.unused));
    }
}
