//! XML import/export for the three dialects XIMOD Translator interoperates with:
//!
//! * ESP-ESM Translator export — `<DocumentElement>` / `<ESP>`
//! * xTranslator export — `<SSTXMLRessources>` (`<Params>` + `<Content>`)
//! * XIMOD native — `<XimodTM>` (lossless dump of the common model)
//!
//! Reading uses a tiny generic element tree (built with quick-xml); writing is
//! done by hand so each dialect's exact layout — attribute order, self-closing
//! empty tags, indentation — is preserved.

use anyhow::{Result, bail};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;

use super::model::{TmEntry, TmMeta, TranslationMemory};

// ----------------------------- generic tree --------------------------------

#[derive(Debug, Default)]
struct Node {
    tag: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, tag: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.tag == tag)
    }
    fn children_named(&self, tag: &str) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(move |c| c.tag == tag)
    }
    /// Trimmed text of a named child (empty string if absent).
    fn child_text(&self, tag: &str) -> String {
        self.child(tag)
            .map(|c| c.text.trim().to_string())
            .unwrap_or_default()
    }
    fn child_int(&self, tag: &str, default: i64) -> i64 {
        let t = self.child_text(tag);
        if t.is_empty() {
            default
        } else {
            t.parse().unwrap_or(default)
        }
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Parse an XML document into a single root node.
fn parse(xml: &str) -> Result<Node> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;

    let read_attrs = |e: &quick_xml::events::BytesStart| -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for a in e.attributes() {
            let a = a?;
            let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
            let val = a.unescape_value()?.into_owned();
            out.push((key, val));
        }
        Ok(out)
    };

    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                stack.push(Node {
                    tag,
                    attrs: read_attrs(&e)?,
                    ..Default::default()
                });
            }
            Event::Empty(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let node = Node {
                    tag,
                    attrs: read_attrs(&e)?,
                    ..Default::default()
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root = Some(node),
                }
            }
            Event::Text(e) => {
                let txt = e.unescape()?.into_owned();
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&txt);
                }
            }
            Event::CData(e) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(&e));
                }
            }
            Event::End(_) => {
                if let Some(node) = stack.pop() {
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(node),
                        None => root = Some(node),
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    root.ok_or_else(|| anyhow::anyhow!(crate::l10n!("err-xml-empty")))
}

// ------------------------------ readers ------------------------------------

/// Read an ESP-ESM Translator XML export (`<DocumentElement>`).
pub fn read_eetxml(xml: &str) -> Result<TranslationMemory> {
    let root = parse(xml)?;
    let mut entries = Vec::new();
    for esp in root.children_named("ESP") {
        let id = esp.child_text("ID");
        let formid = if id.is_empty() {
            0
        } else {
            u32::from_str_radix(&id, 16).unwrap_or(0)
        };
        let status = esp.child_int("STATUS", -1) as i16;
        entries.push(TmEntry {
            source: esp.child_text("ORIGINAL"),
            trans: esp.child_text("TRADUIT"),
            formid,
            edid: esp.child_text("EDID"),
            rec_sig: esp.child_text("GRUP"),
            field_sig: esp.child_text("CHAMP"),
            comment: esp.child_text("COMMENTAIRE"),
            perso: esp.child_text("PERSO"),
            index: esp.child_int("INDEX", 1) as i32,
            idstexte: esp.child_int("IDSTEXTE", -1) as i32,
            icon: esp.child_int("ICON", -1) as i32,
            status_eet: status,
            status_sst: if status == 99 {
                vec!["translated".to_string()]
            } else {
                Vec::new()
            },
            ..Default::default()
        });
    }
    let meta = TmMeta {
        source_format: "eetxml".to_string(),
        ..Default::default()
    };
    Ok(TranslationMemory::new(entries, meta))
}

/// Read an xTranslator XML export (`<SSTXMLRessources>`).
pub fn read_sstxml(xml: &str) -> Result<TranslationMemory> {
    let root = parse(xml)?;
    let (mut addon, mut lang_s, mut lang_d, mut version) =
        (String::new(), String::new(), String::new(), 2i32);
    if let Some(params) = root.child("Params") {
        addon = params.child_text("Addon");
        lang_s = params.child_text("Source");
        lang_d = params.child_text("Dest");
        version = params.child_int("Version", 2) as i32;
    }

    let mut entries = Vec::new();
    let content = root.child("Content");
    let strings: Vec<&Node> = match content {
        Some(c) => c.children_named("String").collect(),
        None => Vec::new(),
    };
    for st in strings {
        let rec_node = st.child("REC");
        let rec_txt = rec_node
            .map(|n| n.text.trim().to_string())
            .unwrap_or_default();
        let (grup, champ) = match rec_txt.split_once(':') {
            Some((g, c)) => (g.to_string(), c.to_string()),
            None => (rec_txt.clone(), String::new()),
        };
        let list_index = st.attr("List").and_then(|s| s.parse().ok()).unwrap_or(0u8);
        let id = rec_node
            .and_then(|n| n.attr("id"))
            .and_then(|s| s.parse::<i32>().ok());
        let idmax = rec_node
            .and_then(|n| n.attr("idMax"))
            .and_then(|s| s.parse::<u16>().ok());
        let index = id.map(|x| x + 1).unwrap_or(1);
        let index_max = idmax.unwrap_or(0);
        let trans = st.child_text("Dest");
        let translated = !trans.is_empty();
        entries.push(TmEntry {
            source: st.child_text("Source"),
            trans,
            edid: st.child_text("EDID"),
            rec_sig: grup,
            field_sig: champ,
            index,
            index_max,
            list_index,
            idstexte: -1,
            icon: -1,
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
        masters: if addon.is_empty() {
            Vec::new()
        } else {
            vec![addon.clone()]
        },
        source_format: "sstxml".to_string(),
        addon,
        lang_source: lang_s,
        lang_dest: lang_d,
        sstxml_version: version,
        ..Default::default()
    };
    Ok(TranslationMemory::new(entries, meta))
}

/// Read the XIMOD native XML (`<XimodTM>`).
pub fn read_ximodxml(xml: &str) -> Result<TranslationMemory> {
    let root = parse(xml)?;
    let mut meta = TmMeta {
        source_format: "ximodxml".to_string(),
        ..Default::default()
    };
    if let Some(m) = root.child("Meta") {
        meta.game = m.child_text("Game");
        meta.addon = m.child_text("Addon");
        meta.lang_source = m.child_text("LangSource");
        meta.lang_dest = m.child_text("LangDest");
        if let Some(masters) = m.child("Masters") {
            meta.masters = masters
                .children_named("Master")
                .map(|n| n.text.trim().to_string())
                .collect();
        }
        if let Some(cl) = m.child("ColabLabels") {
            for c in cl.children_named("Colab") {
                let cid = c.attr("id").and_then(|s| s.parse().ok()).unwrap_or(0);
                meta.colab_labels.push((cid, c.text.trim().to_string()));
            }
        }
    }
    let mut entries = Vec::new();
    if let Some(ents) = root.child("Entries") {
        for el in ents.children_named("Entry") {
            let id = el.child_text("formid");
            let formid = if id.is_empty() {
                0
            } else {
                u32::from_str_radix(&id, 16).unwrap_or(0)
            };
            let status_sst: Vec<String> = el
                .child_text("status_sst")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            entries.push(TmEntry {
                source: el.child_text("source"),
                trans: el.child_text("trans"),
                formid,
                edid: el.child_text("edid"),
                rec_sig: el.child_text("rec_sig"),
                field_sig: el.child_text("field_sig"),
                comment: el.child_text("comment"),
                perso: el.child_text("perso"),
                index: el.child_int("index", 1) as i32,
                index_max: el.child_int("index_max", 0) as u16,
                idstexte: el.child_int("idstexte", -1) as i32,
                icon: el.child_int("icon", -1) as i32,
                list_index: el.child_int("list_index", 0) as u8,
                colab_id: el.child_int("colab_id", 0) as u8,
                rhash: el.child_int("rhash", 0) as u32,
                strid: el.child_int("strid", 0) as i32,
                status_eet: el.child_int("status_eet", -1) as i16,
                status_sst,
                prov: el.child_text("prov"),
                review: el.child_text("review"),
                ..Default::default()
            });
        }
    }
    Ok(TranslationMemory::new(entries, meta))
}

/// Detect the dialect by root tag and dispatch.
pub fn read_auto(xml: &str) -> Result<TranslationMemory> {
    let root = parse(xml)?;
    match root.tag.as_str() {
        "DocumentElement" => read_eetxml(xml),
        "SSTXMLRessources" => read_sstxml(xml),
        "XimodTM" => read_ximodxml(xml),
        other => bail!(crate::l10n!("err-xml-root", root = other)),
    }
}

// ------------------------------ writers ------------------------------------

fn esc_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Write an ESP-ESM Translator XML export.
pub fn write_eetxml(tm: &TranslationMemory) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<DocumentElement>\n");
    for e in &tm.entries {
        s.push_str("  <ESP>\n");
        s.push_str(&format!("    <GRUP>{}</GRUP>\n", esc_text(&e.rec_sig)));
        s.push_str(&format!("    <ID>{:08X}</ID>\n", e.formid));
        s.push_str(&format!("    <EDID>{}</EDID>\n", esc_text(&e.edid)));
        s.push_str(&format!("    <CHAMP>{}</CHAMP>\n", esc_text(&e.field_sig)));
        s.push_str(&format!(
            "    <ORIGINAL>{}</ORIGINAL>\n",
            esc_text(&e.source)
        ));
        s.push_str(&format!("    <TRADUIT>{}</TRADUIT>\n", esc_text(&e.trans)));
        if e.perso.is_empty() {
            s.push_str("    <PERSO />\n");
        } else {
            s.push_str(&format!("    <PERSO>{}</PERSO>\n", esc_text(&e.perso)));
        }
        s.push_str(&format!(
            "    <INDEX>{}</INDEX>\n",
            if e.index == 0 { 1 } else { e.index }
        ));
        s.push_str(&format!("    <STATUS>{}</STATUS>\n", e.status_eet));
        s.push_str(&format!("    <IDSTEXTE>{}</IDSTEXTE>\n", e.idstexte));
        let comment = crate::l10n::strip(&e.comment);
        if comment.is_empty() {
            s.push_str("    <COMMENTAIRE />\n");
        } else {
            s.push_str(&format!(
                "    <COMMENTAIRE>{}</COMMENTAIRE>\n",
                esc_text(&comment)
            ));
        }
        s.push_str(&format!("    <ICON>{}</ICON>\n", e.icon));
        s.push_str("  </ESP>\n");
    }
    s.push_str("</DocumentElement>");
    s
}

/// Write an xTranslator XML export, reconstructing `id`/`idMax` on `<REC>` by
/// grouping entries per (EDID, REC).
pub fn write_sstxml(tm: &TranslationMemory) -> String {
    let n = tm.entries.len();
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, e) in tm.entries.iter().enumerate() {
        let rec = format!("{}:{}", e.rec_sig, e.field_sig);
        groups.entry((e.edid.clone(), rec)).or_default().push(i);
    }
    let mut id_attr = vec![None; n];
    let mut idmax_attr = vec![None; n];
    for idxs in groups.values() {
        if idxs.len() > 1 {
            let mx = idxs.len() - 1;
            for (pos, &gi) in idxs.iter().enumerate() {
                id_attr[gi] = Some(pos);
                idmax_attr[gi] = Some(mx);
            }
        }
    }

    let addon = if !tm.meta.addon.is_empty() {
        tm.meta.addon.clone()
    } else if !tm.meta.plugin_name.is_empty() {
        tm.meta.plugin_name.clone()
    } else {
        tm.meta.masters.first().cloned().unwrap_or_default()
    };
    let src = if tm.meta.lang_source.is_empty() {
        "en"
    } else {
        &tm.meta.lang_source
    };
    let dst = if tm.meta.lang_dest.is_empty() {
        "fr"
    } else {
        &tm.meta.lang_dest
    };

    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    s.push_str("<SSTXMLRessources>\n  <Params>\n");
    s.push_str(&format!("    <Addon>{}</Addon>\n", esc_text(&addon)));
    s.push_str(&format!("    <Source>{}</Source>\n", esc_text(src)));
    s.push_str(&format!("    <Dest>{}</Dest>\n", esc_text(dst)));
    s.push_str(&format!(
        "    <Version>{}</Version>\n",
        tm.meta.sstxml_version
    ));
    s.push_str("  </Params>\n  <Content>\n");
    for (i, e) in tm.entries.iter().enumerate() {
        let rec = format!("{}:{}", e.rec_sig, e.field_sig);
        s.push_str(&format!("    <String List=\"{}\">\n", e.list_index));
        s.push_str(&format!("      <EDID>{}</EDID>\n", esc_text(&e.edid)));
        match (id_attr[i], idmax_attr[i]) {
            (Some(id), Some(mx)) => s.push_str(&format!(
                "      <REC id=\"{id}\" idMax=\"{mx}\">{}</REC>\n",
                esc_text(&rec)
            )),
            _ => s.push_str(&format!("      <REC>{}</REC>\n", esc_text(&rec))),
        }
        s.push_str(&format!("      <Source>{}</Source>\n", esc_text(&e.source)));
        s.push_str(&format!("      <Dest>{}</Dest>\n", esc_text(&e.trans)));
        s.push_str("    </String>\n");
    }
    s.push_str("  </Content>\n</SSTXMLRessources>");
    s
}

/// Write the XIMOD native XML (lossless dump of the common model).
pub fn write_ximodxml(tm: &TranslationMemory) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<XimodTM version=\"1\">\n");
    s.push_str("  <Meta>\n");
    s.push_str(&format!("    <Game>{}</Game>\n", esc_text(&tm.meta.game)));
    s.push_str(&format!(
        "    <SourceFormat>{}</SourceFormat>\n",
        esc_text(&tm.meta.source_format)
    ));
    if !tm.meta.addon.is_empty() {
        s.push_str(&format!(
            "    <Addon>{}</Addon>\n",
            esc_text(&tm.meta.addon)
        ));
    }
    if !tm.meta.lang_source.is_empty() {
        s.push_str(&format!(
            "    <LangSource>{}</LangSource>\n",
            esc_text(&tm.meta.lang_source)
        ));
    }
    if !tm.meta.lang_dest.is_empty() {
        s.push_str(&format!(
            "    <LangDest>{}</LangDest>\n",
            esc_text(&tm.meta.lang_dest)
        ));
    }
    s.push_str("    <Masters>\n");
    for m in &tm.meta.masters {
        s.push_str(&format!("      <Master>{}</Master>\n", esc_text(m)));
    }
    s.push_str("    </Masters>\n    <ColabLabels>\n");
    for (cid, lbl) in &tm.meta.colab_labels {
        s.push_str(&format!(
            "      <Colab id=\"{cid}\">{}</Colab>\n",
            esc_text(lbl)
        ));
    }
    s.push_str("    </ColabLabels>\n  </Meta>\n  <Entries>\n");
    for e in &tm.entries {
        s.push_str("    <Entry>\n");
        s.push_str(&format!(
            "      <rec_sig>{}</rec_sig>\n",
            esc_text(&e.rec_sig)
        ));
        s.push_str(&format!(
            "      <field_sig>{}</field_sig>\n",
            esc_text(&e.field_sig)
        ));
        s.push_str(&format!("      <edid>{}</edid>\n", esc_text(&e.edid)));
        s.push_str(&format!("      <formid>{:08X}</formid>\n", e.formid));
        s.push_str(&format!("      <source>{}</source>\n", esc_text(&e.source)));
        s.push_str(&format!("      <trans>{}</trans>\n", esc_text(&e.trans)));
        s.push_str(&format!("      <perso>{}</perso>\n", esc_text(&e.perso)));
        s.push_str(&format!(
            "      <comment>{}</comment>\n",
            esc_text(&crate::l10n::strip(&e.comment))
        ));
        s.push_str(&format!("      <index>{}</index>\n", e.index));
        s.push_str(&format!("      <index_max>{}</index_max>\n", e.index_max));
        s.push_str(&format!("      <idstexte>{}</idstexte>\n", e.idstexte));
        s.push_str(&format!("      <icon>{}</icon>\n", e.icon));
        s.push_str(&format!(
            "      <status_eet>{}</status_eet>\n",
            e.status_eet
        ));
        s.push_str(&format!(
            "      <list_index>{}</list_index>\n",
            e.list_index
        ));
        s.push_str(&format!("      <colab_id>{}</colab_id>\n", e.colab_id));
        s.push_str(&format!("      <rhash>{}</rhash>\n", e.rhash));
        s.push_str(&format!("      <strid>{}</strid>\n", e.strid));
        s.push_str(&format!(
            "      <status_sst>{}</status_sst>\n",
            esc_text(&e.status_sst.join(","))
        ));
        // Provenance and review (B-09, A-17): only when set.
        if !e.prov.is_empty() {
            s.push_str(&format!("      <prov>{}</prov>\n", esc_text(&e.prov)));
        }
        if !e.review.is_empty() {
            s.push_str(&format!("      <review>{}</review>\n", esc_text(&e.review)));
        }
        s.push_str("    </Entry>\n");
    }
    s.push_str("  </Entries>\n</XimodTM>");
    s
}
