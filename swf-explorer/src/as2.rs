//! ActionScript 2 (AVM1) disassembler / reassembler for **safe string editing**.
//!
//! Editing a string literal changes the byte length of the action stream, which
//! shifts every subsequent byte — breaking the byte offsets baked into `Jump`,
//! `If`, and the block sizes of `DefineFunction`, `DefineFunction2`, `With` and
//! `Try`. To edit safely we disassemble the stream into instructions, resolve
//! every offset/size to an **instruction boundary**, edit the strings, then
//! re-lay-out and recompute all offsets and sizes from the new positions.
//!
//! Safety net: [`reinject`] only rewrites a stream whose *no-edit* reassembly is
//! byte-identical to the original. If our model doesn't perfectly capture some
//! exotic stream, we decline to edit it rather than risk corruption.

use std::collections::HashMap;

const ACTION_CONSTANT_POOL: u8 = 0x88;
const ACTION_PUSH: u8 = 0x96;
const ACTION_JUMP: u8 = 0x99;
const ACTION_IF: u8 = 0x9D;
const ACTION_WITH: u8 = 0x94;
const ACTION_DEFINE_FUNCTION: u8 = 0x9B;
const ACTION_DEFINE_FUNCTION2: u8 = 0x8E;
const ACTION_TRY: u8 = 0x8F;

/// A pushed value (ActionPush): a string literal (editable) or an opaque value
/// kept as its exact bytes (type byte included).
enum PushVal {
    Str(String),
    Raw(Vec<u8>),
}

/// One decoded AVM1 instruction.
enum Ins {
    /// Single-byte action (code < 0x80).
    Simple(u8),
    /// Opaque long action: code + raw data (no offsets inside).
    Raw { code: u8, data: Vec<u8> },
    /// ActionConstantPool: the string table.
    Pool(Vec<String>),
    /// ActionPush: a list of typed values.
    Push(Vec<PushVal>),
    /// ActionJump: branch to instruction `target`.
    Jump { target: usize },
    /// ActionIf: conditional branch to instruction `target`.
    If { target: usize },
    /// A block whose trailing u16 code-size covers instructions `[self+1, body_end)`
    /// (With / DefineFunction / DefineFunction2). `head` is the action data minus
    /// that trailing u16.
    Block {
        code: u8,
        head: Vec<u8>,
        body_end: usize,
    },
    /// ActionTry: `head` holds flags + the 3 block sizes + catch name/register;
    /// the try/catch/finally bodies span up to the given instruction indices.
    Try {
        head: Vec<u8>,
        try_end: usize,
        catch_end: usize,
        finally_end: usize,
    },
}

fn rd_u16(b: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]))
}

/// Disassemble an AVM1 action stream. Returns `None` if anything is malformed or
/// an offset doesn't land on an instruction boundary (we then decline to edit).
fn disassemble(bytes: &[u8]) -> Option<Vec<Ins>> {
    // First pass: split into (pos, code, data) records.
    let mut raw: Vec<(usize, u8, Vec<u8>)> = Vec::new();
    let mut p = 0usize;
    while p < bytes.len() {
        let code = bytes[p];
        if code < 0x80 {
            raw.push((p, code, Vec::new()));
            p += 1;
        } else {
            let len = rd_u16(bytes, p + 1)? as usize;
            let start = p + 3;
            let end = start.checked_add(len)?;
            if end > bytes.len() {
                return None;
            }
            raw.push((p, code, bytes[start..end].to_vec()));
            p = end;
        }
    }
    let total = bytes.len();

    // Map a byte position to an instruction index (or `n` for end-of-stream).
    let mut pos_idx: HashMap<usize, usize> = HashMap::with_capacity(raw.len() + 1);
    for (i, (pos, _, _)) in raw.iter().enumerate() {
        pos_idx.insert(*pos, i);
    }
    pos_idx.insert(total, raw.len());
    let end_of = |k: usize| -> usize {
        if k + 1 < raw.len() {
            raw[k + 1].0
        } else {
            total
        }
    };
    let resolve = |target: i64| -> Option<usize> {
        if target < 0 {
            return None;
        }
        pos_idx.get(&(target as usize)).copied()
    };

    let mut out = Vec::with_capacity(raw.len());
    for (k, (_pos, code, data)) in raw.iter().enumerate() {
        let ins = match *code {
            c if c < 0x80 => Ins::Simple(c),
            ACTION_CONSTANT_POOL => Ins::Pool(parse_pool(data)?),
            ACTION_PUSH => Ins::Push(parse_push(data)?),
            ACTION_JUMP | ACTION_IF => {
                let off = i16::from_le_bytes([*data.first()?, *data.get(1)?]) as i64;
                let target = resolve(end_of(k) as i64 + off)?;
                if *code == ACTION_JUMP {
                    Ins::Jump { target }
                } else {
                    Ins::If { target }
                }
            }
            ACTION_WITH | ACTION_DEFINE_FUNCTION | ACTION_DEFINE_FUNCTION2 => {
                if data.len() < 2 {
                    return None;
                }
                let cs = rd_u16(data, data.len() - 2)? as i64;
                let head = data[..data.len() - 2].to_vec();
                let body_end = resolve(end_of(k) as i64 + cs)?;
                Ins::Block {
                    code: *code,
                    head,
                    body_end,
                }
            }
            ACTION_TRY => {
                if data.len() < 7 {
                    return None;
                }
                let try_sz = rd_u16(data, 1)? as i64;
                let catch_sz = rd_u16(data, 3)? as i64;
                let finally_sz = rd_u16(data, 5)? as i64;
                let base = end_of(k) as i64;
                let try_end = resolve(base + try_sz)?;
                let catch_end = resolve(base + try_sz + catch_sz)?;
                let finally_end = resolve(base + try_sz + catch_sz + finally_sz)?;
                Ins::Try {
                    head: data.clone(),
                    try_end,
                    catch_end,
                    finally_end,
                }
            }
            _ => Ins::Raw {
                code: *code,
                data: data.clone(),
            },
        };
        out.push(ins);
    }
    Some(out)
}

fn parse_pool(data: &[u8]) -> Option<Vec<String>> {
    let count = rd_u16(data, 0)? as usize;
    let mut p = 2usize;
    let mut v = Vec::with_capacity(count);
    for _ in 0..count {
        let end = data.get(p..)?.iter().position(|&c| c == 0)? + p;
        v.push(decode(&data[p..end]));
        p = end + 1;
    }
    Some(v)
}

fn parse_push(data: &[u8]) -> Option<Vec<PushVal>> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p < data.len() {
        let t = data[p];
        if t == 0 {
            let end = data.get(p + 1..)?.iter().position(|&c| c == 0)? + p + 1;
            out.push(PushVal::Str(decode(&data[p + 1..end])));
            p = end + 1;
        } else {
            let size = match t {
                1 => 4, // float
                2 | 3 => 0,
                4 => 1, // register
                5 => 1, // bool
                6 => 8, // double
                7 => 4, // integer
                8 => 1, // constant8
                9 => 2, // constant16
                _ => return None,
            };
            let end = p + 1 + size;
            if end > data.len() {
                return None;
            }
            out.push(PushVal::Raw(data[p..end].to_vec()));
            p = end;
        }
    }
    Some(out)
}

/// Encoded byte size of one instruction.
fn size_of(ins: &Ins) -> usize {
    match ins {
        Ins::Simple(_) => 1,
        Ins::Raw { data, .. } => 3 + data.len(),
        Ins::Pool(v) => 3 + 2 + v.iter().map(|s| s.len() + 1).sum::<usize>(),
        Ins::Push(pvs) => {
            3 + pvs
                .iter()
                .map(|v| match v {
                    PushVal::Str(s) => 2 + s.len(),
                    PushVal::Raw(b) => b.len(),
                })
                .sum::<usize>()
        }
        Ins::Jump { .. } | Ins::If { .. } => 5,
        Ins::Block { head, .. } => 3 + head.len() + 2,
        Ins::Try { head, .. } => 3 + head.len(),
    }
}

/// Reassemble instructions, recomputing every offset and block size from the new
/// layout. Returns `None` on overflow of a 16-bit offset/size field.
fn assemble(instrs: &[Ins]) -> Option<Vec<u8>> {
    // New position of each instruction (index n = end of stream).
    let mut pos = Vec::with_capacity(instrs.len() + 1);
    let mut running = 0usize;
    for ins in instrs {
        pos.push(running);
        running += size_of(ins);
    }
    pos.push(running);

    let s16 = |from_end: usize, target: usize| -> Option<[u8; 2]> {
        let delta = pos[target] as i64 - from_end as i64;
        Some((i16::try_from(delta).ok()?).to_le_bytes())
    };
    let u16b = |a: usize, b: usize| -> Option<[u8; 2]> {
        Some((u16::try_from(pos[b].checked_sub(pos[a])?).ok()?).to_le_bytes())
    };

    let mut out = Vec::with_capacity(running);
    for (i, ins) in instrs.iter().enumerate() {
        match ins {
            Ins::Simple(c) => out.push(*c),
            Ins::Raw { code, data } => {
                out.push(*code);
                out.extend_from_slice(&(data.len() as u16).to_le_bytes());
                out.extend_from_slice(data);
            }
            Ins::Pool(v) => {
                let mut body = Vec::new();
                body.extend_from_slice(&(v.len() as u16).to_le_bytes());
                for s in v {
                    body.extend_from_slice(s.as_bytes());
                    body.push(0);
                }
                out.push(ACTION_CONSTANT_POOL);
                out.extend_from_slice(&(body.len() as u16).to_le_bytes());
                out.extend_from_slice(&body);
            }
            Ins::Push(pvs) => {
                let mut body = Vec::new();
                for v in pvs {
                    match v {
                        PushVal::Str(s) => {
                            body.push(0);
                            body.extend_from_slice(s.as_bytes());
                            body.push(0);
                        }
                        PushVal::Raw(b) => body.extend_from_slice(b),
                    }
                }
                out.push(ACTION_PUSH);
                out.extend_from_slice(&(body.len() as u16).to_le_bytes());
                out.extend_from_slice(&body);
            }
            Ins::Jump { target } | Ins::If { target } => {
                let code = if matches!(ins, Ins::Jump { .. }) {
                    ACTION_JUMP
                } else {
                    ACTION_IF
                };
                out.push(code);
                out.extend_from_slice(&2u16.to_le_bytes());
                out.extend_from_slice(&s16(pos[i] + 5, *target)?);
            }
            Ins::Block {
                code,
                head,
                body_end,
            } => {
                let cs = u16b(i + 1, *body_end)?;
                out.push(*code);
                out.extend_from_slice(&((head.len() + 2) as u16).to_le_bytes());
                out.extend_from_slice(head);
                out.extend_from_slice(&cs);
            }
            Ins::Try {
                head,
                try_end,
                catch_end,
                finally_end,
            } => {
                let mut h = head.clone();
                h[1..3].copy_from_slice(&u16b(i + 1, *try_end)?);
                h[3..5].copy_from_slice(&u16b(*try_end, *catch_end)?);
                h[5..7].copy_from_slice(&u16b(*catch_end, *finally_end)?);
                out.push(ACTION_TRY);
                out.extend_from_slice(&(h.len() as u16).to_le_bytes());
                out.extend_from_slice(&h);
            }
        }
    }
    Some(out)
}

/// The string slots of a stream, in canonical order (constant-pool entries and
/// pushed strings, in stream order). Index positions are stable and used as the
/// `si` in an `as2:{tag}:{si}` id (empty slots are kept so indices don't shift).
fn collect_slots(instrs: &[Ins]) -> Vec<String> {
    let mut v = Vec::new();
    for ins in instrs {
        match ins {
            Ins::Pool(p) => v.extend(p.iter().cloned()),
            Ins::Push(pvs) => {
                for pv in pvs {
                    if let PushVal::Str(s) = pv {
                        v.push(s.clone());
                    }
                }
            }
            _ => {}
        }
    }
    v
}

fn apply_slot_edits(instrs: &mut [Ins], edits: &HashMap<usize, String>) {
    let mut slot = 0usize;
    for ins in instrs.iter_mut() {
        match ins {
            Ins::Pool(p) => {
                for s in p.iter_mut() {
                    if let Some(n) = edits.get(&slot) {
                        *s = n.clone();
                    }
                    slot += 1;
                }
            }
            Ins::Push(pvs) => {
                for pv in pvs.iter_mut() {
                    if let PushVal::Str(s) = pv {
                        if let Some(n) = edits.get(&slot) {
                            *s = n.clone();
                        }
                        slot += 1;
                    }
                }
            }
            _ => {}
        }
    }
}

fn decode(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

/// Extract the canonical string slots of an action stream, plus whether the
/// stream can be safely re-injected (disassembles and round-trips byte-exact).
pub fn extract_slots(action_bytes: &[u8]) -> (Vec<String>, bool) {
    match disassemble(action_bytes) {
        Some(instrs) => {
            let reinjectable = assemble(&instrs)
                .map(|b| b == action_bytes)
                .unwrap_or(false);
            (collect_slots(&instrs), reinjectable)
        }
        None => (linear_slots(action_bytes), false),
    }
}

/// Reinject edited strings (slot index → new text). Returns the rewritten action
/// bytes, or `None` if the stream can't be edited safely.
pub fn reinject(action_bytes: &[u8], edits: &HashMap<usize, String>) -> Option<Vec<u8>> {
    let mut instrs = disassemble(action_bytes)?;
    // Safety net: only edit streams we can reproduce byte-for-byte untouched.
    if assemble(&instrs)?.as_slice() != action_bytes {
        return None;
    }
    apply_slot_edits(&mut instrs, edits);
    assemble(&instrs)
}

/// Fallback slot listing when disassembly fails: a linear scan of ConstantPool
/// and Push strings (extraction/visibility only — never re-injected).
fn linear_slots(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p < bytes.len() {
        let code = bytes[p];
        p += 1;
        if code < 0x80 {
            continue;
        }
        let Some(len) = rd_u16(bytes, p).map(|v| v as usize) else {
            break;
        };
        p += 2;
        let Some(data) = bytes.get(p..p + len) else {
            break;
        };
        match code {
            ACTION_CONSTANT_POOL => {
                if let Some(v) = parse_pool(data) {
                    out.extend(v);
                }
            }
            ACTION_PUSH => {
                if let Some(pvs) = parse_push(data) {
                    for pv in pvs {
                        if let PushVal::Str(s) = pv {
                            out.push(s);
                        }
                    }
                }
            }
            _ => {}
        }
        p += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build an action stream: ConstantPool["A","BB"], Push "hi", a backward Jump
    // to the start, then End. Exercises string edit + offset recompute.
    fn sample() -> Vec<u8> {
        let mut s = Vec::new();
        // ConstantPool
        let mut pool = Vec::new();
        pool.extend_from_slice(&2u16.to_le_bytes());
        pool.extend_from_slice(b"A\0BB\0");
        s.push(ACTION_CONSTANT_POOL);
        s.extend_from_slice(&(pool.len() as u16).to_le_bytes());
        s.extend_from_slice(&pool);
        // Push string "hi"
        let mut push = vec![0u8];
        push.extend_from_slice(b"hi\0");
        s.push(ACTION_PUSH);
        s.extend_from_slice(&(push.len() as u16).to_le_bytes());
        s.extend_from_slice(&push);
        // Jump back to offset 0: target = end_of_jump + off = 0 → off = -(end)
        let end_after_jump = s.len() + 5;
        let off = -(end_after_jump as i64) as i16;
        s.push(ACTION_JUMP);
        s.extend_from_slice(&2u16.to_le_bytes());
        s.extend_from_slice(&off.to_le_bytes());
        // End
        s.push(0);
        s
    }

    #[test]
    fn identity_roundtrip() {
        let s = sample();
        let ins = disassemble(&s).expect("disasm");
        assert_eq!(
            assemble(&ins).unwrap(),
            s,
            "no-edit reassembly must be exact"
        );
    }

    #[test]
    fn slots_are_in_order() {
        let s = sample();
        let (slots, reinj) = extract_slots(&s);
        assert!(reinj);
        assert_eq!(slots, vec!["A", "BB", "hi"]);
    }

    #[test]
    fn edit_changes_length_and_fixes_jump() {
        let s = sample();
        // Replace "hi" (slot 2) with a longer accented string.
        let mut edits = HashMap::new();
        edits.insert(2usize, "bonjour à toi".to_string());
        let out = reinject(&s, &edits).expect("reinject");
        assert_ne!(out.len(), s.len(), "length must change");
        // Re-disassemble: the new stream is well-formed and the jump still targets 0.
        let ins = disassemble(&out).expect("re-disasm");
        let slots = collect_slots(&ins);
        assert_eq!(slots, vec!["A", "BB", "bonjour à toi"]);
        // And the jump offset now reflects the new layout (still lands on a boundary).
        assert_eq!(assemble(&ins).unwrap(), out);
    }
}
