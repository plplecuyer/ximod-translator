//! TES4 write engine (modes 3/4 of the CDC).
//!
//! Produces a translated plugin from an original plugin plus a translation
//! memory whose entries carry translations. The strategy mirrors xTranslator's
//! `saveEsp`: keep every untouched record's raw bytes (so it stays byte-identical
//! — the J1 guardrail still holds for the parts we don't change), and only
//! re-serialise the records that actually receive a translation, recomputing
//! their `dataSize` and every enclosing GRUP `groupSize` (handled by the
//! fidelity tree in [`super::tree`]).
//!
//! This module covers **mode 4** (full translated esp) for **non-localized**
//! plugins (Skyrim LE, Oblivion, Fallout 3/NV) — the prototype target of the
//! write plan. Localized plugins (SSE/FO4/Starfield, external STRINGS) and mode 3
//! (override patch) build on this and come next.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use anyhow::{Result, bail};
use flate2::read::ZlibDecoder;

use super::localized::StringsBundle;
use super::record::{FLAG_COMPRESSED, FLAG_LOCALIZED};
use super::tree::{GroupNode, Node, PluginTree, parse_tree};
use crate::games::{FieldCtx, FieldMode, FieldTable, Game, registry};
use crate::tm::TranslationMemory;

/// TES4 header flag: the plugin is light (ESL) — a different, compact FormID
/// scheme this override engine does not yet remap.
const FLAG_ESL: u32 = 0x0000_0200;

/// Effective translations indexed by `FormID → (field, index) → translation`.
type TransIndex = HashMap<u32, HashMap<(String, i32), String>>;

fn build_translation_index(tm: &TranslationMemory, skip_header: bool) -> TransIndex {
    let mut idx: TransIndex = HashMap::new();
    for e in &tm.entries {
        if !e.is_translated() {
            continue;
        }
        if skip_header && e.formid == 0 {
            continue; // TES4 header fields are not patched in override mode.
        }
        idx.entry(e.formid)
            .or_default()
            .insert((e.field_sig.clone(), e.index), e.trans.clone());
    }
    idx
}

/// Build a full translated plugin (mode 4): the whole original plugin, with every
/// effective translation reinjected. Untranslated records are preserved
/// byte-for-byte.
///
/// `source` is the original plugin's bytes. The plugin must be non-localized
/// (its strings stored inline); localized plugins are out of scope here.
pub fn write_full_esp(source: &[u8], tm: &TranslationMemory) -> Result<Vec<u8>> {
    let plugin = super::reader::parse(source)?;
    if plugin.localized {
        bail!(crate::l10n!("err-write-localized-inline"));
    }
    if plugin.game == Game::Morrowind {
        bail!(crate::l10n!("err-write-tes3"));
    }
    let game = plugin.game;

    // Index the effective translations by FormID → (field, index) → translation.
    // FormIDs are unique within a plugin, so this addresses each record precisely.
    let by_formid = build_translation_index(tm, false);
    let reg = registry();
    let table = reg.fields(game);

    let mut tree = parse_tree(source, plugin.header_len)?;
    let mut patched = 0usize;
    for rec in tree.records_mut() {
        let formid = rec.formid();
        let Some(fields) = by_formid.get(&formid) else {
            continue; // No translation targets this record — leave it untouched.
        };
        if patch_record(rec, fields, table, game)? {
            patched += 1;
        }
    }

    if patched == 0 {
        // Nothing matched: return the identity round-trip rather than a subtly
        // different file. (Also the correct result when the TM is empty.)
        return Ok(tree.write());
    }
    Ok(tree.write())
}

/// What a delocalization did — so the caller can tell a complete conversion from
/// a partial one.
#[derive(Debug, Clone, Default)]
pub struct DelocalizeReport {
    /// Table fields inlined (translated when available, else the resolved original).
    pub table_inlined: usize,
    /// Localized string fields found **outside** the translatable table and
    /// inlined from the STRINGS bundle (kept in the original language). Their
    /// presence means the conversion was partial: these are not translated.
    pub extra_inlined: usize,
    /// `(record, field, id)` for string fields whose id was **not** in the bundle
    /// (e.g. wrong-language STRINGS) — inlined as empty and worth surfacing.
    pub unresolved: Vec<(String, String, u32)>,
}

impl DelocalizeReport {
    /// A fully clean conversion: no extra (untranslated) inlines, nothing unresolved.
    pub fn is_complete(&self) -> bool {
        self.extra_inlined == 0 && self.unresolved.is_empty()
    }
}

/// Delocalize a localized plugin into a **standalone esp**: every localized
/// string field (which holds a 4-byte string id) is rewritten to hold the text
/// **inline**, and the `localized` header flag is cleared. The result no longer
/// needs its external STRINGS files.
///
/// Because the `localized` flag is global, clearing it is only safe if *every*
/// lstring is inlined. So beyond the translatable-table fields (inlined with
/// their translation, else their resolved original), this also catches **lstrings
/// outside the table**: any 4-byte field whose signature is a known string
/// signature and whose value is a real string id in `bundle` is inlined from the
/// bundle (untranslated). The [`DelocalizeReport`] records how many such extra
/// fields were inlined so a partial conversion is never silent.
///
/// `bundle` is required for a localized plugin (it is the source of originals and
/// the ground truth for detecting leftover lstrings). A non-localized plugin is
/// handled as a plain full esp (`bundle` may be `None`).
pub fn write_delocalized_esp(
    source: &[u8],
    tm: &TranslationMemory,
    bundle: Option<&StringsBundle>,
) -> Result<(Vec<u8>, DelocalizeReport)> {
    let plugin = super::reader::parse(source)?;
    if !plugin.localized {
        // Nothing to inline; a "full esp" already stores strings inline.
        return Ok((write_full_esp(source, tm)?, DelocalizeReport::default()));
    }
    if plugin.game == Game::Morrowind {
        bail!(crate::l10n!("err-write-tes3-localized"));
    }
    let game = plugin.game;
    let bundle = match bundle {
        Some(b) if !b.is_empty() => b,
        _ => bail!(crate::l10n!("err-write-no-strings")),
    };

    // Effective translations for table fields, keyed like extraction.
    let mut trans: HashMap<u32, HashMap<(String, i32), String>> = HashMap::new();
    for e in &tm.entries {
        if e.formid != 0 && e.is_translated() {
            trans
                .entry(e.formid)
                .or_default()
                .insert((e.field_sig.clone(), e.index), e.trans.clone());
        }
    }

    let mut tree = parse_tree(source, plugin.header_len)?;
    let mut report = DelocalizeReport::default();
    let reg = registry();
    let table = reg.fields(game);
    let lstrings = crate::lstrings::Table::shipped_or_derived(game);
    for rec in tree.records_mut() {
        let empty = HashMap::new();
        let rec_trans = trans.get(&rec.formid()).unwrap_or(&empty);
        delocalize_record(
            rec,
            rec_trans,
            bundle,
            (table, lstrings.as_ref()),
            game,
            &mut report,
        )?;
    }

    // Clear the localized flag on the TES4 header.
    if let Some(Node::Record(tes4)) = tree.nodes.first_mut() {
        let flags = tes4.flags() & !FLAG_LOCALIZED;
        tes4.set_flags(flags);
    }

    Ok((tree.write(), report))
}

/// Inline a record's localized id fields. Table fields use their translation
/// (else the bundle original); string-signature fields outside the table that
/// still hold a valid string id are inlined from the bundle (untranslated) so the
/// standalone plugin stays valid. Counting for table fields mirrors extraction.
fn delocalize_record(
    rec: &mut super::tree::RecordNode,
    rec_trans: &HashMap<(String, i32), String>,
    bundle: &StringsBundle,
    tables: (&FieldTable, Option<&crate::lstrings::Table>),
    game: Game,
    report: &mut DelocalizeReport,
) -> Result<bool> {
    let (table, lstrings) = tables;
    let sig = rec.sig_str();
    if sig == "TES4" {
        return Ok(false); // header author/description are already inline
    }
    let was_compressed = rec.flags() & FLAG_COMPRESSED != 0;
    let field_bytes = if was_compressed {
        decompress(&rec.data)?
    } else {
        rec.data.clone()
    };
    let mut subs = parse_subrecords(&field_bytes);

    let edid = subs
        .iter()
        .find(|s| s.0 == "EDID")
        .map(|s| zstring(&s.1))
        .unwrap_or_default();

    // Other fields of the record, for `fieldEquals` conditions (A-06). Read
    // from a snapshot: the loop below mutates the fields.
    let snapshot: Vec<(String, String)> =
        subs.iter().map(|(k, v)| (k.clone(), zstring(v))).collect();
    let other = |f: &str| {
        snapshot
            .iter()
            .find(|(k, _)| k == f)
            .map(|(_, v)| v.clone())
    };

    let mut counters: HashMap<String, i32> = HashMap::new();
    let mut changed = false;
    for (ssig, sdata) in subs.iter_mut() {
        // The scripts' strings are never lstrings (C-08): inline, translated
        // when exported.
        if ssig == crate::vmad_lock::FIELD {
            if let Some(b) = super::vmad::translate_record(&sig, sdata, rec_trans, game.uses_utf8())
            {
                *sdata = b;
                changed = true;
            }
            continue;
        }
        // Localized ids carry no text: the mode is decided on signature, EDID
        // and other fields, exactly as at extraction.
        let ctx = FieldCtx {
            edid: &edid,
            value: "",
            other: &other,
        };
        let is_table = table.mode(&sig, ssig, &ctx) != FieldMode::Ignore;

        if is_table {
            // Localized field holds a ≥4-byte id (extraction used the first 4).
            if sdata.len() < 4 {
                continue;
            }
            let id = u32::from_le_bytes([sdata[0], sdata[1], sdata[2], sdata[3]]);
            let idx = counters.entry(ssig.clone()).or_insert(0);
            *idx += 1;
            let text = if let Some(t) = rec_trans.get(&(ssig.clone(), *idx)) {
                report.table_inlined += 1;
                t.clone()
            } else if let Some(orig) = bundle.get(id) {
                report.table_inlined += 1;
                orig.to_string()
            } else {
                report.unresolved.push((sig.clone(), ssig.clone(), id));
                String::new()
            };
            *sdata = encode_zstring(&text, game);
            changed = true;
        } else if sdata.len() == 4
            && match lstrings {
                // The fields the engine reads as identifiers (C-09): a flag
                // or a counter of a string signature is left alone.
                Some(t) => {
                    t.list(&sig, ssig, &edid).is_some() || crate::lstrings::always_text(ssig)
                }
                None => !matches!(ssig.as_str(), "DATA" | "ONAM") && table.is_string_field(ssig),
            }
        {
            // Possible lstring outside the table: only inline if the value is a
            // real string id (avoids corrupting a coincidental FormID/float).
            let id = u32::from_le_bytes([sdata[0], sdata[1], sdata[2], sdata[3]]);
            if let Some(orig) = bundle.get(id) {
                *sdata = encode_zstring(orig, game);
                report.extra_inlined += 1;
                changed = true;
            }
        }
    }

    if !changed {
        return Ok(false);
    }
    rec.data = serialize_subrecords(&subs);
    if was_compressed {
        let flags = rec.flags() & !FLAG_COMPRESSED;
        rec.set_flags(flags);
    }
    Ok(true)
}

/// Build an **override patch** (mode 3): a small plugin that masters the original
/// (and its masters) and contains **only** the translated records, copied from
/// the original with their strings replaced. It does not redistribute the mod.
///
/// `original_name` is the original plugin's file name, added as the last master.
/// Because the patch's master list is `[original masters…] + [original]`, the
/// original's own records already sit at that master index, so **no FormID
/// remapping is needed** — every FormID (record ids and internal references)
/// stays valid unchanged.
pub fn write_override_esp(
    source: &[u8],
    tm: &TranslationMemory,
    original_name: &str,
) -> Result<Vec<u8>> {
    write_override_esp_with(source, tm, original_name, &OverrideOptions::default()).map(|(b, _)| b)
}

/// Options of the override patch (S-01).
#[derive(Default, Clone, Copy)]
pub struct OverrideOptions<'a> {
    /// The original's resolved STRINGS: required when the original is
    /// localized (the patch is then written delocalized, R-S01-01).
    pub bundle: Option<&'a StringsBundle>,
    /// Set the light (ESL) flag when the game and the patch allow it
    /// (R-S01-03); a light original keeps its flag regardless.
    pub esl_if_eligible: bool,
}

/// What an override patch contains (S-01), computed by [`plan_override`]
/// before writing and returned by [`write_override_esp_with`] after.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverrideReport {
    /// The original is localized: the patch is written delocalized.
    pub localized: bool,
    /// Records carrying at least one exported field.
    pub records: usize,
    /// Text fields inlined from the STRINGS (translated or original).
    pub delocalized: usize,
    /// String ids of kept records that no STRINGS file resolves
    /// `(record, field, id)`; those fields are written empty (R-S01-02).
    pub unresolved: Vec<(String, String, u32)>,
    /// The patch may carry the light flag (game supports ESL, no own record
    /// outside the light range).
    pub esl_eligible: bool,
    /// The light flag is set in the written patch.
    pub esl: bool,
}

/// Compute the override patch's report without writing it (for the export
/// dialog, R-S01-02).
pub fn plan_override(
    source: &[u8],
    tm: &TranslationMemory,
    opts: &OverrideOptions,
) -> Result<OverrideReport> {
    build_override(source, tm, "", opts).map(|(_, r)| r)
}

/// Build a translation **override patch** (mode 3) — see [`write_override_esp`]
/// — with the options of S-01: a localized original is written delocalized
/// (its kept records get their texts inline, the header loses the localized
/// flag) and the light flag can be set when eligible. Returns the bytes and
/// the report.
pub fn write_override_esp_with(
    source: &[u8],
    tm: &TranslationMemory,
    original_name: &str,
    opts: &OverrideOptions,
) -> Result<(Vec<u8>, OverrideReport)> {
    let (tree, report) = build_override(source, tm, original_name, opts)?;
    Ok((tree.write(), report))
}

fn build_override(
    source: &[u8],
    tm: &TranslationMemory,
    original_name: &str,
    opts: &OverrideOptions,
) -> Result<(PluginTree, OverrideReport)> {
    let plugin = super::reader::parse(source)?;
    if plugin.game == Game::Morrowind {
        bail!(crate::l10n!("err-write-tes3"));
    }
    let game = plugin.game;
    let mut report = OverrideReport {
        localized: plugin.localized,
        ..Default::default()
    };
    let bundle = if plugin.localized {
        match opts.bundle {
            Some(b) if !b.is_empty() => Some(b),
            _ => bail!(crate::l10n!("err-write-no-strings")),
        }
    } else {
        None
    };

    let by_formid = build_translation_index(tm, true);
    if by_formid.is_empty() {
        bail!(crate::l10n!("err-write-override-empty"));
    }
    let keep: HashSet<u32> = by_formid.keys().copied().collect();
    report.records = keep.len();

    let mut tree = parse_tree(source, plugin.header_len)?;
    let is_light = matches!(tree.nodes.first(), Some(Node::Record(t)) if t.flags() & FLAG_ESL != 0);

    // Prune to the translated records plus the parent chain (World/Cell/Topic)
    // and the TES4 header.
    tree.nodes = prune_nodes(&tree.nodes, &keep);

    // Patch the translated records (parent records carried for validity are not
    // in `by_formid`, so they stay byte-for-byte copies). A localized original
    // is delocalized record by record (R-S01-01): translation for the exported
    // fields, resolved original for the other text fields.
    let reg = registry();
    let table = reg.fields(game);
    let lstrings = crate::lstrings::Table::shipped_or_derived(game);
    let mut deloc = DelocalizeReport::default();
    for rec in tree.records_mut() {
        if let Some(fields) = by_formid.get(&rec.formid()) {
            match bundle {
                Some(b) => {
                    delocalize_record(
                        rec,
                        fields,
                        b,
                        (table, lstrings.as_ref()),
                        game,
                        &mut deloc,
                    )?;
                }
                None => {
                    patch_record(rec, fields, table, game)?;
                }
            }
        }
    }
    if bundle.is_some() {
        report.delocalized = deloc.table_inlined + deloc.extra_inlined;
        report.unresolved = deloc.unresolved;
    }

    // The patch masters the original *and its masters*, the original appended
    // last. Crucially this holds for ESL originals too: a light plugin's own
    // records are stored in-file with the same master-count self-index as a
    // regular plugin (the 0xFE runtime slot is assigned by the engine, not the
    // file), so appending the original at exactly that index keeps every FormID
    // valid — no compaction or remap. See the light-validity guard below.
    let mut new_masters = plugin.masters.clone();
    new_masters.push(original_name.to_string());
    if new_masters.len() > 0xFD {
        bail!(crate::l10n!("err-write-masters", n = new_masters.len()));
    }

    // An override patch defines no new records of its own (every record overrides
    // a master), so an ESL original yields a valid ESL patch. Guard the light
    // contract defensively: any record that *would* be the patch's own new record
    // (high byte = master count) must fit the 12-bit object range.
    let own_index = new_masters.len() as u32;
    let light_violation = first_light_violation(&tree.nodes, own_index);
    if is_light && let Some(bad) = light_violation {
        bail!(crate::l10n!(
            "err-write-esl-range",
            formid = format!("{bad:08X}")
        ));
    }
    // R-S01-03: the light flag can be added when the game knows light plugins
    // and no own record breaks the range.
    report.esl_eligible =
        reg.spec(game).plugin_extensions.iter().any(|e| e == "esl") && light_violation.is_none();
    report.esl = is_light || (opts.esl_if_eligible && report.esl_eligible);

    // Count content records (excluding the TES4 header) for HEDR.numRecords.
    let num_records = count_records(&tree.nodes).saturating_sub(1) as i32;

    // Rewrite the header: new master list, patched record count. The original's
    // plugin-type flags (ESM/ESL bits) are preserved — a translation patch lives
    // at the same tier as the mod it patches (an .esm override stays .esm, an
    // .esl stays .esl), matching how authors ship them.
    rebuild_header(&mut tree, &new_masters, num_records)?;
    if let Some(Node::Record(tes4)) = tree.nodes.first_mut() {
        let mut flags = tes4.flags();
        if bundle.is_some() {
            flags &= !FLAG_LOCALIZED; // the patch carries its texts inline
        }
        if report.esl {
            flags |= FLAG_ESL;
        }
        tes4.set_flags(flags);
    }

    Ok((tree, report))
}

/// Recursively prune a sibling list to only the nodes needed for the override:
/// the TES4 header, records whose FormID is translated, GRUPs that still contain
/// kept descendants, and — for World/Cell/Topic children groups — the parent
/// record that immediately precedes such a group.
fn prune_nodes(nodes: &[Node], keep: &HashSet<u32>) -> Vec<Node> {
    let mut kept: Vec<Option<Node>> = Vec::with_capacity(nodes.len());
    let mut needs_parent: Vec<bool> = vec![false; nodes.len()];

    for node in nodes {
        match node {
            Node::Record(r) => {
                let keep_it = r.sig_str() == "TES4" || keep.contains(&r.formid());
                kept.push(if keep_it { Some(node.clone()) } else { None });
            }
            Node::Group(g) => {
                let children = prune_nodes(&g.children, keep);
                if children.is_empty() {
                    kept.push(None);
                } else {
                    let survives_children_group = g.parented_by_preceding_record();
                    kept.push(Some(Node::Group(GroupNode {
                        header_tail: g.header_tail.clone(),
                        children,
                    })));
                    if survives_children_group {
                        needs_parent[kept.len() - 1] = true;
                    }
                }
            }
        }
    }

    // Carry the parent record for surviving World/Cell/Topic children groups.
    for (i, &needs) in needs_parent.iter().enumerate() {
        if needs {
            let mut j = i;
            while j > 0 {
                j -= 1;
                if matches!(nodes[j], Node::Record(_)) {
                    if kept[j].is_none() {
                        kept[j] = Some(nodes[j].clone());
                    }
                    break;
                }
            }
        }
    }

    kept.into_iter().flatten().collect()
}

/// For a light (ESL) patch: find the first record that would be the patch's own
/// new record (FormID high byte == the patch's own master index) yet falls
/// outside the 12-bit light object range. Override patches have no own records,
/// so this normally returns `None`; it exists to catch malformed input rather
/// than silently emit an invalid ESL.
fn first_light_violation(nodes: &[Node], own_index: u32) -> Option<u32> {
    for n in nodes {
        match n {
            Node::Record(r) => {
                let f = r.formid();
                if (f >> 24) == own_index && (f & 0x00FF_FFFF) > 0x0FFF {
                    return Some(f);
                }
            }
            Node::Group(g) => {
                if let Some(bad) = first_light_violation(&g.children, own_index) {
                    return Some(bad);
                }
            }
        }
    }
    None
}

fn count_records(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            Node::Record(_) => 1,
            Node::Group(g) => count_records(&g.children),
        })
        .sum()
}

/// Rewrite the TES4 header record: replace its master list and patch
/// `HEDR.numRecords`. The record's flags are left untouched, so the patch keeps
/// the original's plugin tier (an ESM override stays an ESM).
fn rebuild_header(tree: &mut PluginTree, masters: &[String], num_records: i32) -> Result<()> {
    let Some(Node::Record(tes4)) = tree.nodes.first_mut() else {
        bail!(crate::l10n!("err-plugin-no-tes4"));
    };
    if tes4.sig_str() != "TES4" {
        bail!(crate::l10n!("err-plugin-no-tes4"));
    }

    let subs = parse_subrecords(&tes4.data);
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();

    // HEDR first, with patched record count (numRecords is the i32 at offset 4).
    if let Some((_, hedr)) = subs.iter().find(|(s, _)| s == "HEDR") {
        let mut h = hedr.clone();
        if h.len() >= 8 {
            h[4..8].copy_from_slice(&num_records.to_le_bytes());
        }
        out.push(("HEDR".to_string(), h));
    }
    // Author / description, if present.
    for tag in ["CNAM", "SNAM"] {
        if let Some((_, d)) = subs.iter().find(|(s, _)| s == tag) {
            out.push((tag.to_string(), d.clone()));
        }
    }
    // New master list: MAST(name) + DATA(8 zero bytes) per master.
    for m in masters {
        let mut name = m.clone().into_bytes();
        name.push(0);
        out.push(("MAST".to_string(), name));
        out.push(("DATA".to_string(), vec![0u8; 8]));
    }
    // Preserve any other header subrecords (e.g. INTV), dropping the old
    // MAST/DATA and ONAM (overrides list, only meaningful for ESM).
    for (s, d) in &subs {
        if !matches!(
            s.as_str(),
            "HEDR" | "CNAM" | "SNAM" | "MAST" | "DATA" | "ONAM"
        ) {
            out.push((s.clone(), d.clone()));
        }
    }

    tes4.data = serialize_subrecords(&out);
    // Flags are preserved as-is (ESM/ESL tier kept). ESL originals are rejected
    // earlier, so no compact-FormID concern remains here.
    Ok(())
}

/// Patch one record's subrecords in place. Returns whether anything changed.
///
/// The per-field instance counting mirrors `reader::extract_strings` *exactly*
/// (same translatable predicate, same GMST/empty/LOC_ skips) so that the 1-based
/// `index` used by the dictionary lines up with the right subrecord instance.
fn patch_record(
    rec: &mut super::tree::RecordNode,
    fields: &HashMap<(String, i32), String>,
    table: &FieldTable,
    game: Game,
) -> Result<bool> {
    let sig = rec.sig_str();
    let was_compressed = rec.flags() & FLAG_COMPRESSED != 0;

    // Decode the field stream (decompress if needed).
    let field_bytes = if was_compressed {
        decompress(&rec.data)?
    } else {
        rec.data.clone()
    };
    let mut subs = parse_subrecords(&field_bytes);

    // The record's editor id and the other fields (context of the rules).
    let edid = subs
        .iter()
        .find(|s| s.0 == "EDID")
        .map(|s| zstring(&s.1))
        .unwrap_or_default();
    let snapshot: Vec<(String, String)> =
        subs.iter().map(|(k, v)| (k.clone(), zstring(v))).collect();
    let other = |f: &str| {
        snapshot
            .iter()
            .find(|(k, _)| k == f)
            .map(|(_, v)| v.clone())
    };

    let mut counters: HashMap<String, i32> = HashMap::new();
    // The messages of the scripts before Papyrus (C-07): source, compiled
    // form and header together.
    let mut changed = patch_sctx(&sig, &mut subs, fields, game);
    for (ssig, sdata) in subs.iter_mut() {
        if table.limits(&sig, ssig).is_none() || ssig == "SCTX" {
            continue;
        }
        // The scripts' strings (C-08): only the translated slots change.
        if ssig == crate::vmad_lock::FIELD {
            if let Some(b) = super::vmad::translate_record(&sig, sdata, fields, game.uses_utf8()) {
                *sdata = b;
                changed = true;
            }
            continue;
        }
        let source = zstring(sdata);
        if source.is_empty() {
            continue;
        }
        // Same predicate as extraction (A-06): ignored fields are neither
        // counted nor written; locked ones are counted, and written only when
        // the dictionary holds a translation for them (the user unlocked them).
        let ctx = FieldCtx {
            edid: &edid,
            value: &source,
            other: &other,
        };
        if table.mode(&sig, ssig, &ctx) == FieldMode::Ignore {
            continue;
        }
        // Nth kept instance of this field.
        let idx = counters.entry(ssig.clone()).or_insert(0);
        *idx += 1;
        if let Some(trans) = fields.get(&(ssig.clone(), *idx)) {
            *sdata = encode_zstring(trans, game);
            changed = true;
        }
    }

    if !changed {
        return Ok(false);
    }

    // Re-serialise the field stream and store it uncompressed (clearing the
    // compressed flag) — a valid, simpler choice than recompressing.
    let new_bytes = serialize_subrecords(&subs);
    rec.data = new_bytes;
    if was_compressed {
        let flags = rec.flags() & !FLAG_COMPRESSED;
        rec.set_flags(flags);
    }
    Ok(true)
}

/// Write the translated messages of a record's scripts (C-07), keyed like
/// the extraction: `("SCTX", rank)` over the record's scripts in order.
/// Returns whether anything changed.
fn patch_sctx(
    _rec: &str,
    subs: &mut [(String, Vec<u8>)],
    fields: &HashMap<(String, i32), String>,
    game: Game,
) -> bool {
    if !fields.keys().any(|(f, _)| f == "SCTX") {
        return false;
    }
    let sigs: Vec<String> = subs.iter().map(|s| s.0.clone()).collect();
    let sig_refs: Vec<&str> = sigs.iter().map(String::as_str).collect();
    let mut rank = 0i32;
    let mut changed = false;
    for (schr, scda, sctx) in super::sctx::blocks(&sig_refs) {
        let a = super::sctx::analyze(&subs[sctx].1, scda.map(|i| subs[i].1.as_slice()));
        let mut by_segment: HashMap<usize, Vec<u8>> = HashMap::new();
        for si in 0..a.segments.len() {
            rank += 1;
            if let Some(t) = fields.get(&("SCTX".to_string(), rank)) {
                by_segment.insert(si, crate::tm::encoding::encode(t, game.uses_utf8()));
            }
        }
        if by_segment.is_empty() {
            continue;
        }
        let Some((src, comp, head)) = super::sctx::rewrite(
            &subs[sctx].1,
            scda.map(|i| subs[i].1.as_slice()),
            schr.map(|i| subs[i].1.as_slice()),
            &a,
            &by_segment,
        ) else {
            continue;
        };
        subs[sctx].1 = src;
        if let (Some(i), Some(c)) = (scda, comp) {
            subs[i].1 = c;
        }
        if let (Some(i), Some(h)) = (schr, head) {
            subs[i].1 = h;
        }
        changed = true;
    }
    changed
}

/// Decompress a compressed record body: `u32` decompressed size + zlib stream.
fn decompress(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.len() < 4 {
        bail!(crate::l10n!("err-plugin-compressed"));
    }
    let decomp_size = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    let mut out = Vec::with_capacity(decomp_size);
    ZlibDecoder::new(&raw[4..]).read_to_end(&mut out)?;
    Ok(out)
}

/// Parse a field stream into `(signature, data)` pairs, honouring the `XXXX`
/// large-field escape. Kept local (the reader's version returns a different type)
/// and paired with [`serialize_subrecords`] so a parse→serialise round-trips.
fn parse_subrecords(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut p = 0usize;
    let mut override_size: Option<usize> = None;
    while p + 6 <= data.len() {
        let sig: String = data[p..p + 4].iter().map(|&c| c as char).collect();
        let declared = u16::from_le_bytes([data[p + 4], data[p + 5]]) as usize;
        p += 6;
        if sig == "XXXX" {
            if p + 4 <= data.len() {
                override_size =
                    Some(
                        u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]])
                            as usize,
                    );
            }
            p += declared;
            continue;
        }
        let real = override_size.take().unwrap_or(declared);
        if p + real > data.len() {
            break;
        }
        out.push((sig, data[p..p + real].to_vec()));
        p += real;
    }
    out
}

/// [`parse_subrecords`] for the other writers (C-09).
pub(crate) fn parse_fields(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    parse_subrecords(data)
}

/// [`serialize_subrecords`] for the other writers (C-09).
pub(crate) fn serialize_fields(subs: &[(String, Vec<u8>)]) -> Vec<u8> {
    serialize_subrecords(subs)
}

/// Serialise `(signature, data)` pairs back to a field stream, emitting the
/// `XXXX` escape for any field larger than a `u16`.
fn serialize_subrecords(subs: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (sig, data) in subs {
        let sb = sig.as_bytes();
        if data.len() > 0xFFFF {
            // XXXX header carrying the real size, then the field with size 0.
            out.extend_from_slice(b"XXXX");
            out.extend_from_slice(&4u16.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(sb);
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(data);
        } else {
            out.extend_from_slice(sb);
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
            out.extend_from_slice(data);
        }
    }
    out
}

/// Decode a NUL-terminated zstring (UTF-8, Latin-1 fallback). Trailing NUL removed.
fn zstring(data: &[u8]) -> String {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    crate::tm::encoding::decode(&data[..end], true)
}

/// Encode a translation as a NUL-terminated zstring in the plugin's code page:
/// UTF-8 for the 64-bit engines, Windows-1252 for the older ones, safely capped.
fn encode_zstring(s: &str, game: Game) -> Vec<u8> {
    crate::tm::encoding::encode_zstring(s, game.uses_utf8())
}
