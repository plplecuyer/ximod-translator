//! The messages of the scripts of the games before Papyrus (C-07):
//! Oblivion, Fallout 3, New Vegas. A script record (SCPT) — and the result
//! script of a quest stage or a dialogue response — carries its source
//! (SCTX), its compiled form (SCDA) and its header (SCHR).
//!
//! The texts shown in game are the literals of the instructions `Message`
//! and `MessageBox` (the message, then each button) and of their script
//! extender forms `MessageEx` and `MessageBoxEx` (xOBSE, NVSE), where the
//! buttons follow the message in the same string, separated by `|`. Each
//! one is a [`Segment`]; the other strings of a script are left alone.
//!
//! Each literal is looked for in the compiled script, in the order of the
//! instructions, as the compiler writes a string argument: a `u16` length
//! then the bytes, `%r` turned into a line break. Found, its place is kept
//! and the writing ([`rewrite`]) corrects the source and the compiled form
//! together (D-66): the string and its length, the length of the
//! instruction around it, the length of the `Begin` block around it, the
//! compiled size in SCHR. The `If` jumps count statements, not bytes: they
//! do not move. Not found, the segment is locked « compilé introuvable ».

use std::collections::HashMap;

/// Instructions whose quoted strings are texts.
const INSTRUCTIONS: [&str; 4] = ["messageboxex", "messageex", "messagebox", "message"];

/// One quoted string of an instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Literal {
    /// The instruction as written (`MessageBox`).
    pub instr: String,
    /// Extender form: message and buttons in one string, `|`-separated.
    pub ex: bool,
    /// 0-based line of the instruction in the source.
    pub line: usize,
    /// The bytes between the quotes, in the source.
    pub start: usize,
    pub end: usize,
    /// The comment (`; …`) right above the instruction, if any.
    pub comment: Option<Vec<u8>>,
    /// Rank of the literal in its instruction (0: the message).
    pub rank: usize,
    /// Where it is in the compiled script: offset of its `u16` length, and
    /// whether `%r` was turned into a line break there.
    pub compiled: Option<(usize, bool)>,
}

/// A text line: one literal, or one part of an extender literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Index of its literal in [`analyze`]'s list.
    pub literal: usize,
    /// Part of the literal (0 for a plain one, 0 = message, 1… = buttons
    /// for an extender one).
    pub part: usize,
    /// Its bytes, as in the source.
    pub bytes: Vec<u8>,
    /// The message it belongs to (1-based in the script) and, for a
    /// button, its rank (1-based).
    pub message: usize,
    pub button: Option<usize>,
}

/// The literals of a source and the segments they give.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub literals: Vec<Literal>,
    pub segments: Vec<Segment>,
}

/// The instruction a source line starts with (after an optional reference
/// `player.`), lowercase, if it is one of [`INSTRUCTIONS`].
fn instruction_of(line: &[u8]) -> Option<(String, String)> {
    let s: String = line.iter().map(|&b| b as char).collect();
    let t = s.trim_start();
    let word: String = t
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .collect();
    let name = word.rsplit('.').next().unwrap_or(&word);
    let low = name.to_ascii_lowercase();
    INSTRUCTIONS
        .iter()
        .find(|i| **i == low)
        .map(|_| (name.to_string(), low))
}

/// `%r` (any case) turned into a line break, as the compiler does.
fn compiled_form(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 1 < raw.len() && (raw[i + 1] == b'r' || raw[i + 1] == b'R') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    out
}

/// Find `needle` as a `u16`-prefixed string in `scda` from `from`.
fn find_prefixed(scda: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.len() > u16::MAX as usize {
        return None;
    }
    let mut pat = (needle.len() as u16).to_le_bytes().to_vec();
    pat.extend_from_slice(needle);
    if from > scda.len() || pat.len() > scda.len() {
        return None;
    }
    scda[from..]
        .windows(pat.len())
        .position(|w| w == pat.as_slice())
        .map(|p| from + p)
}

/// Read a source (and its compiled form): the literals of the message
/// instructions, in order, and their segments.
pub fn analyze(sctx: &[u8], scda: Option<&[u8]>) -> Analysis {
    let mut a = Analysis::default();
    let mut line_start = 0usize;
    let mut prev_comment: Option<Vec<u8>> = None;
    let mut from = 0usize;
    let mut message = 0usize;
    let text_end = sctx.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    for (line_no, raw_line) in sctx[..text_end].split(|&b| b == b'\n').enumerate() {
        let this_start = line_start;
        line_start += raw_line.len() + 1;
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        let trimmed: &[u8] = {
            let k = line
                .iter()
                .position(|b| !b.is_ascii_whitespace())
                .unwrap_or(line.len());
            &line[k..]
        };
        if trimmed.is_empty() {
            continue;
        }
        if trimmed[0] == b';' {
            prev_comment = Some(trimmed[1..].trim_ascii().to_vec());
            continue;
        }
        let comment = prev_comment.take();
        let Some((instr, low)) = instruction_of(line) else {
            continue;
        };
        // The code part of the line: up to a `;` outside quotes.
        let mut quotes: Vec<(usize, usize)> = Vec::new();
        let mut open: Option<usize> = None;
        for (k, &b) in line.iter().enumerate() {
            match (b, open) {
                (b'"', None) => open = Some(k + 1),
                (b'"', Some(s)) => {
                    quotes.push((s, k));
                    open = None;
                }
                (b';', None) => break,
                _ => {}
            }
        }
        if quotes.is_empty() {
            continue;
        }
        let ex = low.ends_with("ex");
        message += 1;
        for (rank, (s, e)) in quotes.into_iter().enumerate() {
            let bytes = &line[s..e];
            let start = this_start + s;
            let end = this_start + e;
            // The compiled form: as written, or with `%r` as line breaks.
            let compiled = scda.and_then(|d| {
                if let Some(p) = find_prefixed(d, from, bytes) {
                    return Some((p, false));
                }
                let c = compiled_form(bytes);
                (c != bytes)
                    .then(|| find_prefixed(d, from, &c).map(|p| (p, true)))
                    .flatten()
            });
            if let (Some((p, conv)), Some(d)) = (compiled, scda) {
                let n = if conv {
                    compiled_form(bytes).len()
                } else {
                    bytes.len()
                };
                from = (p + 2 + n).min(d.len());
            }
            let li = a.literals.len();
            a.literals.push(Literal {
                instr: instr.clone(),
                ex,
                line: line_no,
                start,
                end,
                comment: if rank == 0 { comment.clone() } else { None },
                rank,
                compiled,
            });
            if ex {
                for (part, piece) in bytes.split(|&b| b == b'|').enumerate() {
                    if piece.iter().all(|b| b.is_ascii_whitespace()) {
                        continue;
                    }
                    a.segments.push(Segment {
                        literal: li,
                        part,
                        bytes: piece.to_vec(),
                        message,
                        button: (part > 0).then_some(part),
                    });
                }
            } else if !bytes.is_empty() {
                a.segments.push(Segment {
                    literal: li,
                    part: 0,
                    bytes: bytes.to_vec(),
                    message,
                    button: (rank > 0).then_some(rank),
                });
            }
        }
    }
    a
}

/// The statements of a compiled script: `(start, len field offset, body,
/// len)`; a reference call (`0x1C`) has its function's length field.
fn statements(scda: &[u8]) -> Vec<(usize, usize, usize, usize)> {
    let rd =
        |p: usize| -> Option<u16> { scda.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]])) };
    let mut out = Vec::new();
    let mut p = 0usize;
    while let Some(op) = rd(p) {
        let (lenf, body) = if op == 0x1C {
            (p + 6, p + 8)
        } else {
            (p + 2, p + 4)
        };
        let Some(len) = rd(lenf) else {
            break;
        };
        out.push((p, lenf, body, len as usize));
        p = body + len as usize;
    }
    out
}

/// Whether a compiled script is well formed: its statements chain to its
/// end and every `Begin` block ends on an `End`.
pub fn well_formed(scda: &[u8]) -> bool {
    let st = statements(scda);
    if st
        .last()
        .is_some_and(|(_, _, body, len)| body + len != scda.len())
    {
        return false;
    }
    let starts: std::collections::HashSet<usize> = st.iter().map(|s| s.0).collect();
    st.iter()
        .filter(|(s, _, _, _)| scda.get(*s..*s + 2) == Some(&[0x10, 0x00]))
        .all(|&(_, _, body, len)| {
            let Some(bl) = scda.get(body + 2..body + 6) else {
                return false;
            };
            let end = body + len + u32::from_le_bytes([bl[0], bl[1], bl[2], bl[3]]) as usize;
            end >= 4
                && starts.contains(&(end - 4))
                && scda.get(end - 4..end - 2) == Some(&[0x11, 0x00])
        })
}

/// A script rewritten: its source, compiled form and header (the last two
/// when the record has them).
pub type Rewritten = (Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>);

/// Rewrite a script's source, compiled form and header with new bytes by
/// segment index (`(literal, part)` order of [`Analysis::segments`]).
/// Returns `None` when nothing changes.
pub fn rewrite(
    sctx: &[u8],
    scda: Option<&[u8]>,
    schr: Option<&[u8]>,
    a: &Analysis,
    by_segment: &HashMap<usize, Vec<u8>>,
) -> Option<Rewritten> {
    // The new text of each literal touched.
    let mut new_lit: HashMap<usize, Vec<u8>> = HashMap::new();
    for (si, seg) in a.segments.iter().enumerate() {
        let Some(t) = by_segment.get(&si) else {
            continue;
        };
        let lit = &a.literals[seg.literal];
        let cur = new_lit
            .entry(seg.literal)
            .or_insert_with(|| sctx[lit.start..lit.end].to_vec());
        if lit.ex {
            let mut parts: Vec<Vec<u8>> = cur.split(|&b| b == b'|').map(<[u8]>::to_vec).collect();
            if seg.part < parts.len() {
                parts[seg.part] = t.clone();
            }
            *cur = parts.join(&b'|');
        } else {
            *cur = t.clone();
        }
    }
    new_lit.retain(|li, v| *v != sctx[a.literals[*li].start..a.literals[*li].end]);
    if new_lit.is_empty() {
        return None;
    }
    // The source: from the end, the ranges between the quotes.
    let mut order: Vec<usize> = new_lit.keys().copied().collect();
    order.sort_by_key(|li| std::cmp::Reverse(a.literals[*li].start));
    let mut src = sctx.to_vec();
    for li in &order {
        let l = &a.literals[*li];
        src.splice(l.start..l.end, new_lit[li].iter().copied());
    }
    // The compiled form.
    let mut out_scda = scda.map(<[u8]>::to_vec);
    let mut delta_total: i64 = 0;
    if let (Some(orig), Some(d)) = (scda, out_scda.as_mut()) {
        let stmts = statements(orig);
        // Begin blocks: (blockLen field, from, to).
        let begins: Vec<(usize, usize, usize)> = stmts
            .iter()
            .filter(|(s, _, _, _)| orig.get(*s..*s + 2) == Some(&[0x10, 0x00]))
            .filter_map(|&(_, _, body, len)| {
                let f = body + 2;
                let bl = orig.get(f..f + 4)?;
                let bl = u32::from_le_bytes([bl[0], bl[1], bl[2], bl[3]]) as usize;
                Some((f, body + len, body + len + bl))
            })
            .collect();
        let mut comp: Vec<(usize, usize)> = new_lit
            .keys()
            .filter_map(|li| a.literals[*li].compiled.map(|(p, _)| (p, *li)))
            .collect();
        comp.sort_by_key(|(p, _)| std::cmp::Reverse(*p));
        for (p, li) in comp {
            let l = &a.literals[li];
            let conv = l.compiled.map(|c| c.1).unwrap_or(false);
            let old_len = u16::from_le_bytes([d[p], d[p + 1]]) as usize;
            let mut bytes = new_lit[&li].clone();
            if conv {
                bytes = compiled_form(&bytes);
            }
            bytes.truncate(u16::MAX as usize);
            let delta = bytes.len() as i64 - old_len as i64;
            let mut rep = (bytes.len() as u16).to_le_bytes().to_vec();
            rep.extend_from_slice(&bytes);
            d.splice(p..p + 2 + old_len, rep);
            // The instruction around it.
            if let Some(&(_, lenf, _, _)) = stmts
                .iter()
                .find(|(_, _, body, len)| *body <= p && p < body + len)
            {
                let v = u16::from_le_bytes([d[lenf], d[lenf + 1]]) as i64 + delta;
                d[lenf..lenf + 2]
                    .copy_from_slice(&(v.clamp(0, u16::MAX as i64) as u16).to_le_bytes());
            }
            // The Begin block around it.
            for &(f, from, to) in &begins {
                if from <= p && p < to {
                    let v = u32::from_le_bytes([d[f], d[f + 1], d[f + 2], d[f + 3]]) as i64 + delta;
                    d[f..f + 4].copy_from_slice(&(v.max(0) as u32).to_le_bytes());
                }
            }
            delta_total += delta;
        }
    }
    let out_schr = match (schr, delta_total) {
        (Some(h), dt) if dt != 0 && h.len() >= 12 => {
            let mut h = h.to_vec();
            let v = u32::from_le_bytes([h[8], h[9], h[10], h[11]]) as i64 + dt;
            h[8..12].copy_from_slice(&(v.max(0) as u32).to_le_bytes());
            Some(h)
        }
        (Some(h), _) => Some(h.to_vec()),
        (None, _) => None,
    };
    Some((src, out_scda, out_schr))
}

/// The script blocks of a record's fields: `(SCHR, SCDA, SCTX)` indexes,
/// each block starting at a SCHR (or at the SCTX alone).
pub fn blocks(sigs: &[&str]) -> Vec<(Option<usize>, Option<usize>, usize)> {
    let mut out = Vec::new();
    let (mut schr, mut scda) = (None, None);
    for (i, s) in sigs.iter().enumerate() {
        match *s {
            "SCHR" | "SCHD" => {
                schr = Some(i);
                scda = None;
            }
            "SCDA" => scda = Some(i),
            "SCTX" => {
                out.push((schr, scda, i));
                schr = None;
                scda = None;
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p16(v: usize) -> [u8; 2] {
        (v as u16).to_le_bytes()
    }

    /// `scn T`, `begin GameMode`, `MessageBox "Hello %r world" "Yes" "No"`,
    /// `MessageBoxEx "Pick|One|Two"`, `end`, compiled by hand.
    fn sample() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let src = b"scn T\r\n\r\nbegin GameMode\r\n; ask the player\r\n  MessageBox \"Hello %r world\" \"Yes\" \"No\"\r\n  player.MessageBoxEx \"Pick|One|Two\" ; comment\r\nend\r\n\0".to_vec();
        let mut d = vec![0x1D, 0, 0, 0];
        // Begin: op, len 6, event, blockLen (filled below).
        d.extend_from_slice(&[0x10, 0, 6, 0, 0, 0, 0, 0, 0, 0]);
        let body_start = d.len();
        // MessageBox: op 0x1001, len, count 3, three strings.
        let strs: [&[u8]; 3] = [b"Hello \n world", b"Yes", b"No"];
        let plen = 2 + strs.iter().map(|s| 2 + s.len()).sum::<usize>();
        d.extend_from_slice(&[0x01, 0x10]);
        d.extend_from_slice(&p16(plen));
        d.extend_from_slice(&p16(3));
        for s in strs {
            d.extend_from_slice(&p16(s.len()));
            d.extend_from_slice(s);
        }
        // player.MessageBoxEx: 0x1C, ref 1, op 0x1500, len, count 1, string.
        let s = b"Pick|One|Two";
        d.extend_from_slice(&[0x1C, 0, 1, 0, 0x00, 0x15]);
        d.extend_from_slice(&p16(2 + 2 + s.len()));
        d.extend_from_slice(&p16(1));
        d.extend_from_slice(&p16(s.len()));
        d.extend_from_slice(s);
        d.extend_from_slice(&[0x11, 0, 0, 0]);
        let bl = (d.len() - body_start) as u32;
        d[10..14].copy_from_slice(&bl.to_le_bytes());
        let mut h = vec![0u8; 20];
        h[8..12].copy_from_slice(&(d.len() as u32).to_le_bytes());
        (src, d, h)
    }

    #[test]
    fn segments_and_compiled_places() {
        let (src, d, _) = sample();
        let a = analyze(&src, Some(&d));
        let texts: Vec<(String, usize, Option<usize>)> = a
            .segments
            .iter()
            .map(|s| {
                (
                    String::from_utf8_lossy(&s.bytes).into_owned(),
                    s.message,
                    s.button,
                )
            })
            .collect();
        assert_eq!(
            texts,
            vec![
                ("Hello %r world".into(), 1, None),
                ("Yes".into(), 1, Some(1)),
                ("No".into(), 1, Some(2)),
                ("Pick".into(), 2, None),
                ("One".into(), 2, Some(1)),
                ("Two".into(), 2, Some(2)),
            ]
        );
        assert!(a.literals.iter().all(|l| l.compiled.is_some()));
        assert!(a.literals[0].compiled.unwrap().1);
        assert_eq!(
            a.literals[0].comment.as_deref(),
            Some(&b"ask the player"[..])
        );
        assert_eq!(a.literals[0].instr, "MessageBox");
        assert_eq!(a.literals[3].instr, "MessageBoxEx");
    }

    #[test]
    fn rewrite_keeps_the_structure() {
        let (src, d, h) = sample();
        let a = analyze(&src, Some(&d));
        assert!(rewrite(&src, Some(&d), Some(&h), &a, &HashMap::new()).is_none());
        let mut t = HashMap::new();
        t.insert(0usize, b"Bonjour %r le monde".to_vec());
        t.insert(1usize, b"Oui".to_vec());
        t.insert(4usize, b"Un seul".to_vec());
        let (s2, d2, h2) = rewrite(&src, Some(&d), Some(&h), &a, &t).unwrap();
        let (d2, h2) = (d2.unwrap(), h2.unwrap());
        let b = analyze(&s2, Some(&d2));
        let got: Vec<String> = b
            .segments
            .iter()
            .map(|s| String::from_utf8_lossy(&s.bytes).into_owned())
            .collect();
        assert_eq!(
            got,
            vec!["Bonjour %r le monde", "Oui", "No", "Pick", "Un seul", "Two"]
        );
        assert!(b.literals.iter().all(|l| l.compiled.is_some()));
        // The statements still chain to the end, Begin covers up to End,
        // SCHR has the new size.
        let st = statements(&d2);
        let (s, _, body, len) = *st.last().unwrap();
        assert_eq!(body + len, d2.len());
        assert_eq!(&d2[s..s + 2], &[0x11, 0]);
        let bl = u32::from_le_bytes([d2[10], d2[11], d2[12], d2[13]]) as usize;
        assert_eq!(14 + bl, d2.len());
        assert_eq!(
            u32::from_le_bytes([h2[8], h2[9], h2[10], h2[11]]) as usize,
            d2.len()
        );
    }

    #[test]
    fn not_compiled_is_none() {
        let (src, _, _) = sample();
        let a = analyze(&src, Some(b"\x1d\0\0\0"));
        assert!(a.literals.iter().all(|l| l.compiled.is_none()));
        assert_eq!(a.segments.len(), 6);
    }
}
