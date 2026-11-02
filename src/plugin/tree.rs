//! Fidelity tree for **writing** TES4-family plugins.
//!
//! The [`super::reader`] flattens a plugin into a list of records for string
//! *extraction*: it drops the GRUP hierarchy, the exact byte layout and the raw
//! record buffers — everything that writing a byte-faithful plugin back out
//! needs. This module is the complement: it parses a plugin into a tree that
//! preserves **every byte**, so an unmodified plugin round-trips identically
//! (the founding guardrail, J1 of the write plan), and only the records that are
//! actually translated are re-serialised (J2+).
//!
//! Scope: the TES4 family (record/GRUP headers of 20 bytes for Oblivion or 24
//! for Skyrim+/Fallout/Starfield). Morrowind (TES3) writing is a separate engine.

use anyhow::{Result, bail};

fn read_u32(d: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

/// A node in the plugin tree: either a record or a (possibly nested) GRUP.
#[derive(Debug, Clone)]
pub enum Node {
    Record(RecordNode),
    Group(GroupNode),
}

/// A record, kept with its exact header tail and raw data buffer.
///
/// `data` is stored **exactly as in the file** — still zlib-compressed if the
/// record's compressed flag is set. It is only decoded when the record is
/// modified (see [`super::write`]).
#[derive(Debug, Clone)]
pub struct RecordNode {
    /// 4-byte record signature.
    pub sig: [u8; 4],
    /// Header bytes after `sig` + `dataSize` (flags, FormID, version tail).
    /// Its length is `header_len - 8`.
    pub header_tail: Vec<u8>,
    /// Raw record data (compressed as stored). `dataSize` is derived from this
    /// on write, so patching `data` automatically updates the size.
    pub data: Vec<u8>,
}

impl RecordNode {
    pub fn sig_str(&self) -> String {
        self.sig.iter().map(|&c| c as char).collect()
    }
    pub fn flags(&self) -> u32 {
        u32::from_le_bytes([
            self.header_tail[0],
            self.header_tail[1],
            self.header_tail[2],
            self.header_tail[3],
        ])
    }
    pub fn formid(&self) -> u32 {
        u32::from_le_bytes([
            self.header_tail[4],
            self.header_tail[5],
            self.header_tail[6],
            self.header_tail[7],
        ])
    }
    pub fn set_flags(&mut self, flags: u32) {
        self.header_tail[0..4].copy_from_slice(&flags.to_le_bytes());
    }
}

/// A GRUP container, keeping its header tail (label, type, stamp) and children.
/// Its `groupSize` is recomputed from the children on write.
#[derive(Debug, Clone)]
pub struct GroupNode {
    /// Header bytes after `"GRUP"` + `groupSize` (label, type, stamp tail).
    pub header_tail: Vec<u8>,
    pub children: Vec<Node>,
}

impl GroupNode {
    /// GRUP type (`u32` at offset 12 of the header): 0 top, 1 world children,
    /// 2/3 interior cell block/sub-block, 4/5 exterior block/sub-block,
    /// 6 cell children, 7 topic children, 8/9 cell persistent/temporary children.
    pub fn group_type(&self) -> u32 {
        u32::from_le_bytes([
            self.header_tail[4],
            self.header_tail[5],
            self.header_tail[6],
            self.header_tail[7],
        ])
    }

    /// Whether this GRUP's parent is the **record immediately preceding it** as a
    /// sibling (World/Cell/Topic children) — such a parent must be carried along
    /// when the group is kept for a valid override.
    pub fn parented_by_preceding_record(&self) -> bool {
        matches!(self.group_type(), 1 | 6 | 7)
    }
}

/// A parsed plugin tree ready for faithful re-serialisation.
#[derive(Debug, Clone)]
pub struct PluginTree {
    /// Record/GRUP header length used for this plugin (20 or 24).
    pub header_len: usize,
    pub nodes: Vec<Node>,
}

/// Parse a plugin into a fidelity tree. `header_len` comes from the reader's
/// automatic 20-vs-24 detection.
pub fn parse_tree(data: &[u8], header_len: usize) -> Result<PluginTree> {
    if header_len < 16 {
        bail!(crate::l10n!("err-plugin-header", n = header_len));
    }
    let mut nodes = Vec::new();
    let consumed = parse_nodes(data, header_len, 0, &mut nodes)?;
    if consumed != data.len() {
        bail!(crate::l10n!(
            "err-plugin-trailing",
            read = consumed,
            total = data.len()
        ));
    }
    Ok(PluginTree { header_len, nodes })
}

/// Parse a run of sibling nodes (whole file or a GRUP body). Returns the number
/// of bytes consumed, which must equal `data.len()` for a well-formed run.
fn parse_nodes(data: &[u8], header_len: usize, depth: u32, out: &mut Vec<Node>) -> Result<usize> {
    if depth > 32 {
        bail!(crate::l10n!("err-plugin-nesting"));
    }
    let mut p = 0usize;
    while p + header_len <= data.len() {
        let is_grup = &data[p..p + 4] == b"GRUP";
        let size = read_u32(data, p + 4) as usize;
        let header_tail = data[p + 8..p + header_len].to_vec();

        if is_grup {
            // GRUP size includes the header.
            if size < header_len || p + size > data.len() {
                bail!(crate::l10n!("err-plugin-grup-size", offset = p));
            }
            let mut children = Vec::new();
            let body = &data[p + header_len..p + size];
            let used = parse_nodes(body, header_len, depth + 1, &mut children)?;
            if used != body.len() {
                bail!(crate::l10n!("err-plugin-grup-body", offset = p));
            }
            out.push(Node::Group(GroupNode {
                header_tail,
                children,
            }));
            p += size;
        } else {
            let data_start = p + header_len;
            let data_end = data_start + size;
            if data_end > data.len() {
                bail!(crate::l10n!("err-plugin-record", offset = p));
            }
            let sig = [data[p], data[p + 1], data[p + 2], data[p + 3]];
            out.push(Node::Record(RecordNode {
                sig,
                header_tail,
                data: data[data_start..data_end].to_vec(),
            }));
            p = data_end;
        }
    }
    Ok(p)
}

impl PluginTree {
    /// Re-serialise the tree to bytes. On an unmodified tree this reproduces the
    /// original file **byte for byte** (J1); after edits, record `dataSize` and
    /// every enclosing GRUP `groupSize` are recomputed from content.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for n in &self.nodes {
            write_node(n, self.header_len, &mut out);
        }
        out
    }

    /// Iterate all records in document order (descending into GRUPs), mutably.
    pub fn records_mut(&mut self) -> Vec<&mut RecordNode> {
        let mut acc = Vec::new();
        collect_records_mut(&mut self.nodes, &mut acc);
        acc
    }
}

fn collect_records_mut<'a>(nodes: &'a mut [Node], acc: &mut Vec<&'a mut RecordNode>) {
    for n in nodes {
        match n {
            Node::Record(r) => acc.push(r),
            Node::Group(g) => collect_records_mut(&mut g.children, acc),
        }
    }
}

fn write_node(node: &Node, header_len: usize, out: &mut Vec<u8>) {
    match node {
        Node::Record(r) => {
            out.extend_from_slice(&r.sig);
            out.extend_from_slice(&(r.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&r.header_tail);
            out.extend_from_slice(&r.data);
        }
        Node::Group(g) => {
            // Serialise children first so we can size the group.
            let mut child_bytes = Vec::new();
            for c in &g.children {
                write_node(c, header_len, &mut child_bytes);
            }
            let group_size = (header_len + child_bytes.len()) as u32;
            out.extend_from_slice(b"GRUP");
            out.extend_from_slice(&group_size.to_le_bytes());
            out.extend_from_slice(&g.header_tail);
            out.extend_from_slice(&child_bytes);
        }
    }
}
