//! Dynamic String Distributor (DSD) codec — a plain JSON array, one object per
//! translated string, consumed at runtime by the SKSE "Dynamic String
//! Distributor" framework (Skyrim SE/AE). It does not redistribute the mod: the
//! `form_id` references the original plugin, which must be installed.

use anyhow::Result;
use serde_json::{Map, Value, json};

use super::model::{TmEntry, TmMeta, TranslationMemory};

/// Placeholder used when the mod's own plugin file name is unknown (it is stored
/// in neither EET nor SST).
pub const PLUGIN_PLACEHOLDER: &str = "<A_COMPLETER>.esp";

/// Parse a DSD JSON array (already deserialized to a `Value`).
pub fn from_value(v: &Value) -> Result<TranslationMemory> {
    let arr = v.as_array().cloned().unwrap_or_default();
    let mut entries = Vec::with_capacity(arr.len());
    for o in &arr {
        let fid = o.get("form_id").and_then(|x| x.as_str()).unwrap_or("");
        let (hexid, plug) = match fid.split_once('|') {
            Some((h, p)) => (h, p.to_string()),
            None => (fid, String::new()),
        };
        let formid = u32::from_str_radix(hexid.trim(), 16).unwrap_or(0);
        let typ = o.get("type").and_then(|x| x.as_str()).unwrap_or("");
        let mut parts = typ.split_whitespace();
        let rec_sig = parts.next().unwrap_or("").to_string();
        let field_sig = parts.next().unwrap_or("").to_string();
        let src = o
            .get("original")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let tr = o
            .get("string")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let index = o.get("index").and_then(|x| x.as_i64()).unwrap_or(1) as i32;
        let status = o
            .get("status")
            .and_then(|x| x.as_str())
            .unwrap_or("TranslationComplete")
            .to_string();
        let edid = o
            .get("editor_id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();

        let translated = !tr.is_empty();
        entries.push(TmEntry {
            source: src,
            trans: tr,
            formid,
            edid,
            rec_sig,
            field_sig,
            index,
            idstexte: -1,
            icon: -1,
            plugin: plug,
            dsd_status: Some(status),
            status_eet: if translated { 99 } else { -1 },
            status_sst: if translated {
                vec!["translated".to_string()]
            } else {
                Vec::new()
            },
            ..Default::default()
        });
    }
    let meta = TmMeta {
        source_format: "dsd".to_string(),
        ..Default::default()
    };
    Ok(TranslationMemory::new(entries, meta))
}

/// Serialize the DSD JSON for a plugin's translated memory, ready to drop in
/// `SKSE/Plugins/DynamicStringDistributor/<plugin>/`. Returns the pretty JSON
/// bytes and the number of translated entries emitted. The mod's own records key
/// off `meta.plugin_name` (set by the plugin extractor); master-owned records key
/// off the master list — so it works straight from an extracted+merged plugin.
pub fn to_json_bytes(tm: &TranslationMemory) -> (Vec<u8>, usize) {
    let (value, n) = to_value(tm);
    let bytes = serde_json::to_vec_pretty(&value).unwrap_or_default();
    (bytes, n)
}

/// Build the DSD JSON value. Only entries with an effective translation are
/// included (as EET does). Returns the value and the number of entries written.
pub fn to_value(tm: &TranslationMemory) -> (Value, usize) {
    let masters = &tm.meta.masters;
    let mod_plugin = if tm.meta.plugin_name.is_empty() {
        PLUGIN_PLACEHOLDER.to_string()
    } else {
        tm.meta.plugin_name.clone()
    };

    let mut arr = Vec::new();
    for e in &tm.entries {
        // DSD does not address the scripts' strings (C-08).
        if !e.is_translated() || e.field_sig == crate::vmad_lock::FIELD {
            continue;
        }
        // Resolve the owning plugin: explicit, else master by FormID high byte,
        // else the mod's own plugin (placeholder if unknown).
        let plug = if !e.plugin.is_empty() {
            e.plugin.clone()
        } else {
            let hi = (e.formid >> 24) as usize;
            masters
                .get(hi)
                .cloned()
                .unwrap_or_else(|| mod_plugin.clone())
        };
        let typ = [e.rec_sig.as_str(), e.field_sig.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let status = e
            .dsd_status
            .clone()
            .unwrap_or_else(|| "TranslationComplete".to_string());

        let mut obj = Map::new();
        obj.insert("editor_id".into(), json!(e.edid));
        obj.insert(
            "form_id".into(),
            json!(format!("{:08X}|{}", e.formid, plug)),
        );
        obj.insert(
            "index".into(),
            json!(if e.index == 0 { 1 } else { e.index }),
        );
        obj.insert("type".into(), json!(typ));
        obj.insert("original".into(), json!(e.source));
        obj.insert("string".into(), json!(e.trans));
        obj.insert("status".into(), json!(status));
        arr.push(Value::Object(obj));
    }
    let n = arr.len();
    (Value::Array(arr), n)
}
