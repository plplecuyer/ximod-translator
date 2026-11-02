//! Cell View model: reconstructs the worldspace → cell → reference hierarchy
//! that the flat record list drops.
//!
//! It walks the fidelity GRUP tree ([`super::tree`]) carrying a small context
//! (current worldspace FormID, current cell FormID), so every CELL is attached to
//! its worldspace (or "Interiors") and every placed reference (REFR/ACHR/…) to
//! its cell. Reference "Type" is resolved to its base object's record signature
//! when that base object lives in the same plugin (single-plugin scope; master
//! resolution is a later, load-order milestone).

use std::collections::HashMap;

use super::reader::{decompress_record, parse_subrecords};
use super::record::{FLAG_COMPRESSED, SubRecord};
use super::tree::{Node, RecordNode, parse_tree};

/// Placed-reference record signatures that belong to a cell.
const REF_SIGS: &[&str] = &[
    "REFR", "ACHR", "ACRE", "PGRE", "PHZD", "PMIS", "PARW", "PBAR", "PBEA", "PCON", "PFLA", "PLYR",
];

#[derive(Debug, Clone, Default)]
pub struct WorldRow {
    pub formid: u32,
    pub edid: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct CellRow {
    pub formid: u32,
    pub edid: String,
    pub name: String,
    pub interior: bool,
    /// Parent worldspace FormID, or `None` for interior cells.
    pub world: Option<u32>,
    /// Exterior grid coordinates, when present.
    pub coords: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Default)]
pub struct RefRow {
    pub formid: u32,
    pub edid: String,
    pub base: u32,
    /// Base object's record signature when resolvable in this plugin, else the
    /// reference's own signature (REFR/ACHR/…).
    pub type_sig: String,
}

#[derive(Debug, Clone, Default)]
pub struct CellView {
    pub worlds: Vec<WorldRow>,
    pub cells: Vec<CellRow>,
    /// References keyed by their parent cell FormID.
    pub refs: HashMap<u32, Vec<RefRow>>,
}

#[derive(Clone, Copy, Default)]
struct Ctx {
    world: Option<u32>,
    cell: Option<u32>,
}

/// Build the Cell View from raw plugin bytes. Returns an empty view (never an
/// error) when the plugin is not a well-formed TES4-family file — the Object
/// Window still works in that case.
pub fn build(data: &[u8], header_len: usize, localized: bool) -> CellView {
    let Ok(tree) = parse_tree(data, header_len) else {
        return CellView::default();
    };

    // First pass: FormID → record signature, for base-object type resolution.
    let mut base_types: HashMap<u32, String> = HashMap::new();
    collect_base_types(&tree.nodes, &mut base_types);

    // Second pass: walk with context, filling the view.
    let mut cv = CellView::default();
    let mut ctx = Ctx::default();
    walk(&tree.nodes, &mut ctx, localized, &base_types, &mut cv);
    cv
}

fn collect_base_types(nodes: &[Node], out: &mut HashMap<u32, String>) {
    for n in nodes {
        match n {
            Node::Record(r) => {
                out.entry(r.formid()).or_insert_with(|| r.sig_str());
            }
            Node::Group(g) => collect_base_types(&g.children, out),
        }
    }
}

fn walk(
    nodes: &[Node],
    ctx: &mut Ctx,
    localized: bool,
    base_types: &HashMap<u32, String>,
    cv: &mut CellView,
) {
    for n in nodes {
        match n {
            Node::Record(r) => {
                let sig = r.sig_str();
                if sig == "WRLD" {
                    let f = fields(r);
                    cv.worlds.push(WorldRow {
                        formid: r.formid(),
                        edid: zfield(&f, "EDID"),
                        name: name_field(&f, localized),
                    });
                } else if sig == "CELL" {
                    let f = fields(r);
                    let coords = coords_field(&f);
                    // A cell is interior when it has no exterior grid *and* no
                    // parent worldspace (the world's persistent cell also lacks a
                    // grid but is not an interior).
                    let interior = coords.is_none() && ctx.world.is_none();
                    cv.cells.push(CellRow {
                        formid: r.formid(),
                        edid: zfield(&f, "EDID"),
                        name: name_field(&f, localized),
                        interior,
                        world: ctx.world,
                        coords,
                    });
                } else if REF_SIGS.contains(&sig.as_str())
                    && let Some(cell) = ctx.cell
                {
                    let f = fields(r);
                    let base = f
                        .iter()
                        .find(|s| s.sig == "NAME")
                        .and_then(|s| s.as_u32())
                        .unwrap_or(0);
                    let type_sig = base_types
                        .get(&base)
                        .cloned()
                        .unwrap_or_else(|| sig.clone());
                    cv.refs.entry(cell).or_default().push(RefRow {
                        formid: r.formid(),
                        edid: zfield(&f, "EDID"),
                        base,
                        type_sig,
                    });
                }
            }
            Node::Group(g) => {
                let gt = g.group_type();
                let label = u32::from_le_bytes([
                    g.header_tail[0],
                    g.header_tail[1],
                    g.header_tail[2],
                    g.header_tail[3],
                ]);
                let mut child = *ctx;
                match gt {
                    0 => {
                        // Top group (label = record signature): new record type.
                        child.world = None;
                        child.cell = None;
                    }
                    1 => child.world = Some(label), // World Children (label = WRLD FormID)
                    2 | 3 => child.world = None,    // Interior cell block / sub-block
                    // 4 | 5 exterior block/sub-block: keep the worldspace context.
                    6 => child.cell = Some(label), // Cell Children (label = CELL FormID)
                    // Persistent / temporary / visible-distant children: inherit
                    // the cell context, falling back to the group label.
                    8..=10 if child.cell.is_none() => child.cell = Some(label),
                    _ => {}
                }
                walk(&g.children, &mut child, localized, base_types, cv);
            }
        }
    }
}

/// Decode a record node's fields (decompressing first if the record is stored
/// compressed). Best-effort: a decompression failure yields no fields.
fn fields(r: &RecordNode) -> Vec<SubRecord> {
    let bytes = if r.flags() & FLAG_COMPRESSED != 0 {
        decompress_record(&r.data).unwrap_or_default()
    } else {
        r.data.clone()
    };
    parse_subrecords(&bytes)
}

fn zfield(fields: &[SubRecord], sig: &str) -> String {
    fields
        .iter()
        .find(|s| s.sig == sig)
        .map(|s| s.as_zstring())
        .unwrap_or_default()
}

/// A record's display name from its `FULL` field (localized → string-table id).
fn name_field(fields: &[SubRecord], localized: bool) -> String {
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

/// Exterior grid coordinates from a cell's `XCLC` field (i32 X, i32 Y).
fn coords_field(fields: &[SubRecord]) -> Option<(i32, i32)> {
    let xclc = fields.iter().find(|s| s.sig == "XCLC")?;
    if xclc.data.len() < 8 {
        return None;
    }
    let x = i32::from_le_bytes([xclc.data[0], xclc.data[1], xclc.data[2], xclc.data[3]]);
    let y = i32::from_le_bytes([xclc.data[4], xclc.data[5], xclc.data[6], xclc.data[7]]);
    Some((x, y))
}
