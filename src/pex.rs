//! Compiled Papyrus scripts, `.pex` (C-06): reading, classification of the
//! strings, writing.
//!
//! Formats: Skyrim LE and SE (3.1, 3.2, big-endian, game 1), Fallout 4 and
//! 76 (3.9, little-endian, game 2, with structures and property groups),
//! Starfield (3.12, little-endian, game 4, with the guards of an object).
//! A number the reader does not know is not read.
//!
//! The file is read in full — header, string table, debug information,
//! user flags, objects with their structures, variables, properties,
//! states and functions, and every instruction with its operands — so that
//! each string of the table is classed: an **identifier** when a name or a
//! type refers to it (object, parent, state, function, parameter, local,
//! variable, property, structure, debug information, user flag) or an
//! operand of identifier type does; a **literal** when an operand of string
//! type, or the initial value of a variable, refers to it. A string may be
//! both. Every literal operand keeps its byte offset.
//!
//! The writing ([`write`]) rewrites the string table only: a literal alone
//! takes its translation in place; a string that is also an identifier
//! keeps its place (the names stay intact) and its translation is added at
//! the end of the table, the literal operands being re-pointed to it. Every
//! other byte of the file is copied.

use std::collections::{HashMap, HashSet};

/// Why a script was not read (or written).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PexError {
    /// Not a compiled script.
    Magic,
    /// A version the reader does not know.
    Version(u8, u8),
    /// The file ends inside a structure, or a size does not hold.
    Truncated,
    /// An instruction the reader does not know.
    Opcode(u8),
    /// A value type the reader does not know.
    ValueType(u8),
    /// The table would have more than 65 535 strings.
    TooMany,
}

impl std::fmt::Display for PexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Technical words for the log; the interface has its messages.
        match self {
            PexError::Magic => write!(f, "not a compiled script"),
            PexError::Version(a, b) => write!(f, "version {a}.{b}"),
            PexError::Truncated => write!(f, "truncated"),
            PexError::Opcode(o) => write!(f, "opcode {o}"),
            PexError::ValueType(t) => write!(f, "value type {t}"),
            PexError::TooMany => write!(f, "more than 65535 strings"),
        }
    }
}

/// Where a literal is used: the function, and the call it is an argument of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    /// Byte offset of the `u16` string index of the operand.
    pub offset: usize,
    /// `Object.State.Function` (the state left out when it is the empty
    /// one); `Object` for an initial value.
    pub function: String,
    /// The function called with the literal as an argument
    /// (`Debug.Notification`, `Type.Method`, `Method`).
    pub call: Option<String>,
}

/// A read script.
#[derive(Debug, Clone)]
pub struct Pex {
    pub major: u8,
    pub minor: u8,
    pub game_id: u16,
    pub big_endian: bool,
    /// The source file name the compiler recorded.
    pub source: String,
    /// The name of the (first) object: the script's name.
    pub object: String,
    /// The strings of the table, as bytes.
    pub strings: Vec<Vec<u8>>,
    /// Per string: referred to as an identifier.
    pub ident: Vec<bool>,
    /// Per string: its uses as a literal, in file order.
    pub uses: Vec<Vec<Use>>,
    /// Byte range of the table (count and strings).
    table: (usize, usize),
}

impl Pex {
    /// The format, `3.2`.
    pub fn version(&self) -> String {
        format!("{}.{}", self.major, self.minor)
    }

    /// Whether a string is a literal (used as one at least once).
    pub fn is_literal(&self, i: usize) -> bool {
        self.uses.get(i).is_some_and(|u| !u.is_empty())
    }

    /// The indexes of the literals, in table order.
    pub fn literals(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.strings.len()).filter(|i| self.is_literal(*i))
    }
}

/// Arguments of each instruction (fixed part); the calls end with a
/// variadic part (a count, then that many values).
fn op_args(op: u8, starfield: bool) -> Option<(usize, bool)> {
    Some(match op {
        0 => (0, false),
        1..=9 => (3, false),
        10..=14 => (2, false),
        15..=19 => (3, false),
        20 => (1, false),
        21 | 22 => (2, false),
        23 => (3, true),
        24 => (2, true),
        25 => (3, true),
        26 => (1, false),
        27..=29 => (3, false),
        30 | 31 => (2, false),
        32 | 33 => (3, false),
        34 | 35 => (4, false),
        // Fallout 4.
        36 => (3, false),
        37 => (1, false),
        38 | 39 => (3, false),
        40 | 41 => (5, false),
        42 | 43 => (3, false),
        44 => (1, false),
        45 => (3, false),
        46 => (1, false),
        // Starfield (guards, matching structures).
        47 if starfield => (6, false),
        48 | 49 if starfield => (0, true),
        50 if starfield => (1, true),
        _ => return None,
    })
}

/// A value read: its type, and the string index or integer it holds.
#[derive(Debug, Clone, Copy)]
enum Value {
    None,
    Ident(u16),
    Str,
    Int(i32),
    Other,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
    be: bool,
    ident: Vec<bool>,
    uses: Vec<Vec<Use>>,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], PexError> {
        if self.p + n > self.b.len() {
            return Err(PexError::Truncated);
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, PexError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, PexError> {
        let s = self.take(2)?;
        let a = [s[0], s[1]];
        Ok(if self.be {
            u16::from_be_bytes(a)
        } else {
            u16::from_le_bytes(a)
        })
    }
    fn u32(&mut self) -> Result<u32, PexError> {
        let s = self.take(4)?;
        let a = [s[0], s[1], s[2], s[3]];
        Ok(if self.be {
            u32::from_be_bytes(a)
        } else {
            u32::from_le_bytes(a)
        })
    }
    fn skip(&mut self, n: usize) -> Result<(), PexError> {
        self.take(n).map(|_| ())
    }
    fn wstring(&mut self) -> Result<Vec<u8>, PexError> {
        let n = self.u16()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    /// A string index referred to as a name.
    fn name(&mut self) -> Result<u16, PexError> {
        let i = self.u16()?;
        match self.ident.get_mut(i as usize) {
            Some(f) => *f = true,
            None => return Err(PexError::Truncated),
        }
        Ok(i)
    }
    /// A value; a string one is recorded as a literal use.
    fn value(&mut self, function: &str, call: Option<&str>) -> Result<Value, PexError> {
        let t = self.u8()?;
        Ok(match t {
            0 => Value::None,
            1 => Value::Ident(self.name()?),
            2 => {
                let offset = self.p;
                let i = self.u16()?;
                let Some(u) = self.uses.get_mut(i as usize) else {
                    return Err(PexError::Truncated);
                };
                u.push(Use {
                    offset,
                    function: function.to_string(),
                    call: call.map(str::to_string),
                });
                Value::Str
            }
            3 => {
                let v = self.u32()? as i32;
                Value::Int(v)
            }
            4 => {
                self.skip(4)?;
                Value::Other
            }
            5 => {
                self.skip(1)?;
                Value::Other
            }
            t => return Err(PexError::ValueType(t)),
        })
    }
}

/// What a function knows to name the calls: the names of the strings and
/// the types of the object's variables and properties.
struct Ctx<'a> {
    strings: &'a [Vec<u8>],
    object: String,
    /// Variable or property name → type name (lowercase key).
    members: HashMap<String, String>,
    starfield: bool,
}

impl Ctx<'_> {
    fn s(&self, i: u16) -> String {
        self.strings
            .get(i as usize)
            .map(|b| b.iter().map(|&c| c as char).collect())
            .unwrap_or_default()
    }
}

/// A function: signature, locals, instructions.
fn function(r: &mut Reader, ctx: &Ctx, name: &str) -> Result<(), PexError> {
    r.name()?; // return type
    r.name()?; // doc string
    r.u32()?; // user flags
    r.u8()?; // flags
    let mut types: HashMap<String, String> = HashMap::new();
    for _ in 0..2 {
        // Parameters, then locals.
        let n = r.u16()?;
        for _ in 0..n {
            let v = r.name()?;
            let t = r.name()?;
            types.insert(ctx.s(v).to_lowercase(), ctx.s(t));
        }
    }
    let n = r.u16()?;
    for _ in 0..n {
        let op = r.u8()?;
        let (fixed, variadic) = op_args(op, ctx.starfield).ok_or(PexError::Opcode(op))?;
        // The fixed operands name the call of the variadic ones.
        let mut vals = Vec::with_capacity(fixed);
        for _ in 0..fixed {
            vals.push(r.value(name, None)?);
        }
        if !variadic {
            continue;
        }
        let call = match (op, vals.as_slice()) {
            (25, [Value::Ident(class), Value::Ident(f), ..]) => {
                Some(format!("{}.{}", ctx.s(*class), ctx.s(*f)))
            }
            (23, [Value::Ident(f), obj, ..]) => {
                let ty = match obj {
                    Value::Ident(o) => {
                        let on = ctx.s(*o).to_lowercase();
                        if on == "self" {
                            Some(ctx.object.clone())
                        } else {
                            types.get(&on).or_else(|| ctx.members.get(&on)).cloned()
                        }
                    }
                    _ => None,
                };
                Some(match ty {
                    Some(t) => format!("{t}.{}", ctx.s(*f)),
                    None => ctx.s(*f),
                })
            }
            (24, [Value::Ident(f), ..]) => Some(ctx.s(*f)),
            _ => None,
        };
        let count = match r.value(name, None)? {
            Value::Int(n) if n >= 0 => n as usize,
            _ => return Err(PexError::Truncated),
        };
        if count > r.b.len() {
            return Err(PexError::Truncated);
        }
        for _ in 0..count {
            r.value(name, call.as_deref())?;
        }
    }
    Ok(())
}

/// Read a compiled script.
pub fn parse(b: &[u8]) -> Result<Pex, PexError> {
    if b.len() < 4 {
        return Err(PexError::Truncated);
    }
    let be = match b[..4] {
        [0xFA, 0x57, 0xC0, 0xDE] => true,
        [0xDE, 0xC0, 0x57, 0xFA] => false,
        _ => return Err(PexError::Magic),
    };
    let mut r = Reader {
        b,
        p: 4,
        be,
        ident: Vec::new(),
        uses: Vec::new(),
    };
    let major = r.u8()?;
    let minor = r.u8()?;
    let game_id = r.u16()?;
    let known = matches!(
        (major, minor, be),
        (3, 1, true) | (3, 2, true) | (3, 9, false) | (3, 12, false)
    );
    if !known {
        return Err(PexError::Version(major, minor));
    }
    let fo4 = minor >= 9;
    let starfield = minor >= 12;
    r.skip(8)?; // compilation time
    let source = String::from_utf8_lossy(&r.wstring()?).into_owned();
    r.wstring()?; // user
    r.wstring()?; // machine
    let table_start = r.p;
    let n = r.u16()? as usize;
    let mut strings = Vec::with_capacity(n);
    for _ in 0..n {
        strings.push(r.wstring()?);
    }
    let table_end = r.p;
    r.ident = vec![false; n];
    r.uses = vec![Vec::new(); n];
    // Debug information.
    if r.u8()? != 0 {
        r.skip(8)?;
        let nf = r.u16()?;
        for _ in 0..nf {
            r.name()?;
            r.name()?;
            r.name()?;
            r.u8()?;
            let ni = r.u16()? as usize;
            r.skip(2 * ni)?;
        }
        if fo4 {
            let ng = r.u16()?;
            for _ in 0..ng {
                r.name()?;
                r.name()?;
                r.name()?;
                r.u32()?;
                let nn = r.u16()?;
                for _ in 0..nn {
                    r.name()?;
                }
            }
            let no = r.u16()?;
            for _ in 0..no {
                r.name()?;
                r.name()?;
                let nn = r.u16()?;
                for _ in 0..nn {
                    r.name()?;
                }
            }
        }
    }
    // User flags.
    let nu = r.u16()?;
    for _ in 0..nu {
        r.name()?;
        r.u8()?;
    }
    // Objects.
    let no = r.u16()?;
    let mut object = String::new();
    let strings_ref = strings.clone();
    for k in 0..no {
        let oname = r.name()?;
        let start = r.p;
        let size = r.u32()? as usize;
        let mut ctx = Ctx {
            strings: &strings_ref,
            object: String::new(),
            members: HashMap::new(),
            starfield,
        };
        ctx.object = ctx.s(oname);
        if k == 0 {
            object = ctx.object.clone();
        }
        let oname_s = ctx.object.clone();
        r.name()?; // parent
        r.name()?; // doc string
        if fo4 {
            r.u8()?; // const
        }
        r.u32()?; // user flags
        r.name()?; // auto state
        if fo4 {
            let ns = r.u16()?;
            for _ in 0..ns {
                r.name()?;
                let nm = r.u16()?;
                for _ in 0..nm {
                    r.name()?;
                    r.name()?;
                    r.u32()?;
                    r.value(&oname_s, None)?;
                    r.u8()?;
                    r.name()?;
                }
            }
        }
        let nv = r.u16()?;
        for _ in 0..nv {
            let v = r.name()?;
            let t = r.name()?;
            ctx.members.insert(ctx.s(v).to_lowercase(), ctx.s(t));
            r.u32()?;
            r.value(&oname_s, None)?;
            if fo4 {
                r.u8()?;
            }
        }
        if starfield {
            let ng = r.u16()?;
            for _ in 0..ng {
                r.name()?;
            }
        }
        let np = r.u16()?;
        for _ in 0..np {
            let pn = r.name()?;
            let pt = r.name()?;
            ctx.members.insert(ctx.s(pn).to_lowercase(), ctx.s(pt));
            r.name()?; // doc
            r.u32()?;
            let flags = r.u8()?;
            let fname = format!("{oname_s}.{}", ctx.s(pn));
            if flags & 4 != 0 {
                r.name()?;
            } else {
                if flags & 1 != 0 {
                    function(&mut r, &ctx, &fname)?;
                }
                if flags & 2 != 0 {
                    function(&mut r, &ctx, &fname)?;
                }
            }
        }
        let nst = r.u16()?;
        for _ in 0..nst {
            let st = r.name()?;
            let st_s = ctx.s(st);
            let nfn = r.u16()?;
            for _ in 0..nfn {
                let f = r.name()?;
                let fname = if st_s.is_empty() {
                    format!("{oname_s}.{}", ctx.s(f))
                } else {
                    format!("{oname_s}.{st_s}.{}", ctx.s(f))
                };
                function(&mut r, &ctx, &fname)?;
            }
        }
        if r.p - start != size {
            return Err(PexError::Truncated);
        }
    }
    if r.p != b.len() {
        return Err(PexError::Truncated);
    }
    Ok(Pex {
        major,
        minor,
        game_id,
        big_endian: be,
        source,
        object,
        strings,
        ident: r.ident,
        uses: r.uses,
        table: (table_start, table_end),
    })
}

/// Rewrite a script with translations by string index (bytes in the
/// script's encoding). A literal alone is replaced in place; a string that
/// is also an identifier gets a new string at the end of the table and its
/// literal operands point to it. Indexes that are not literals are ignored.
pub fn write(b: &[u8], pex: &Pex, trans: &HashMap<usize, Vec<u8>>) -> Result<Vec<u8>, PexError> {
    let mut strings = pex.strings.clone();
    let mut remap: HashMap<usize, u16> = HashMap::new();
    let mut keys: Vec<&usize> = trans.keys().collect();
    keys.sort();
    for &i in keys {
        if !pex.is_literal(i) {
            continue;
        }
        let t = trans[&i].clone();
        if pex.ident[i] {
            if strings.len() >= u16::MAX as usize {
                return Err(PexError::TooMany);
            }
            remap.insert(i, strings.len() as u16);
            strings.push(t);
        } else {
            strings[i] = t;
        }
    }
    let put16 = |out: &mut Vec<u8>, v: u16| {
        out.extend_from_slice(&if pex.big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        })
    };
    let (start, end) = pex.table;
    let mut out = Vec::with_capacity(b.len() + 256);
    out.extend_from_slice(&b[..start]);
    put16(&mut out, strings.len() as u16);
    for s in &strings {
        let n = s.len().min(u16::MAX as usize);
        put16(&mut out, n as u16);
        out.extend_from_slice(&s[..n]);
    }
    let shift = out.len() as isize - end as isize;
    out.extend_from_slice(&b[end..]);
    let patch: HashSet<usize> = remap.keys().copied().collect();
    for i in patch {
        let to = remap[&i];
        for u in &pex.uses[i] {
            let at = (u.offset as isize + shift) as usize;
            let v = if pex.big_endian {
                to.to_be_bytes()
            } else {
                to.to_le_bytes()
            };
            out[at..at + 2].copy_from_slice(&v);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small Skyrim script, built by hand: one object, one variable with
    /// a string initial value, one function calling Debug.Notification
    /// with a literal, and a literal that is also a property name.
    pub(crate) fn sample(be: bool) -> Vec<u8> {
        let mut b = Vec::new();
        let p16 = |b: &mut Vec<u8>, v: u16| {
            b.extend_from_slice(&if be { v.to_be_bytes() } else { v.to_le_bytes() })
        };
        let p32 = |b: &mut Vec<u8>, v: u32| {
            b.extend_from_slice(&if be { v.to_be_bytes() } else { v.to_le_bytes() })
        };
        let ws = |b: &mut Vec<u8>, s: &str| {
            let n = s.len() as u16;
            b.extend_from_slice(&if be { n.to_be_bytes() } else { n.to_le_bytes() });
            b.extend_from_slice(s.as_bytes());
        };
        if be {
            b.extend_from_slice(&[0xFA, 0x57, 0xC0, 0xDE, 3, 2, 0, 1]);
        } else {
            b.extend_from_slice(&[0xDE, 0xC0, 0x57, 0xFA, 3, 9, 2, 0]);
        }
        b.extend_from_slice(&[0; 8]);
        ws(&mut b, "Test.psc");
        ws(&mut b, "u");
        ws(&mut b, "m");
        let strings = [
            "Test",               // 0 object
            "",                   // 1 empty
            "Quest",              // 2 parent
            "OnInit",             // 3 function
            "None",               // 4 type
            "debug",              // 5 class
            "Notification",       // 6 function
            "::nonevar",          // 7 local
            "Hello, Dragonborn!", // 8 literal
            "Title",              // 9 property name AND literal
            "String",             // 10 type
            "Greeting",           // 11 variable
            "Welcome",            // 12 initial value
        ];
        p16(&mut b, strings.len() as u16);
        for s in strings {
            ws(&mut b, s);
        }
        b.push(0); // no debug info
        p16(&mut b, 0); // user flags
        p16(&mut b, 1); // one object
        p16(&mut b, 0); // name "Test"
        let size_at = b.len();
        p32(&mut b, 0);
        p16(&mut b, 2); // parent
        p16(&mut b, 1); // doc
        if !be {
            b.push(0); // const
        }
        p32(&mut b, 0);
        p16(&mut b, 1); // auto state
        if !be {
            p16(&mut b, 0); // structs
        }
        p16(&mut b, 1); // one variable
        p16(&mut b, 11);
        p16(&mut b, 10);
        p32(&mut b, 0);
        b.push(2);
        p16(&mut b, 12); // = "Welcome"
        if !be {
            b.push(0);
        }
        p16(&mut b, 1); // one property: Title (auto)
        p16(&mut b, 9);
        p16(&mut b, 10);
        p16(&mut b, 1);
        p32(&mut b, 0);
        b.push(4);
        p16(&mut b, 11);
        p16(&mut b, 1); // one state
        p16(&mut b, 1); // ""
        p16(&mut b, 1); // one function
        p16(&mut b, 3); // OnInit
        p16(&mut b, 4); // returns None
        p16(&mut b, 1);
        p32(&mut b, 0);
        b.push(0);
        p16(&mut b, 0); // params
        p16(&mut b, 1); // locals
        p16(&mut b, 7);
        p16(&mut b, 4);
        p16(&mut b, 2); // two instructions
        // callstatic debug Notification ::nonevar [ "Hello…" ]
        b.push(25);
        b.push(1);
        p16(&mut b, 5);
        b.push(1);
        p16(&mut b, 6);
        b.push(1);
        p16(&mut b, 7);
        b.push(3);
        p32(&mut b, 1);
        b.push(2);
        p16(&mut b, 8);
        // assign ::nonevar "Title"
        b.push(13);
        b.push(1);
        p16(&mut b, 7);
        b.push(2);
        p16(&mut b, 9);
        let size = (b.len() - size_at) as u32;
        let s = if be {
            size.to_be_bytes()
        } else {
            size.to_le_bytes()
        };
        b[size_at..size_at + 4].copy_from_slice(&s);
        b
    }

    #[test]
    fn classes_the_strings() {
        for be in [true, false] {
            let b = sample(be);
            let p = parse(&b).unwrap();
            assert_eq!(p.object, "Test");
            let lits: Vec<usize> = p.literals().collect();
            assert_eq!(lits, vec![8, 9, 12]);
            assert!(p.ident[9] && !p.ident[8] && !p.ident[12]);
            assert!(p.ident[3] && p.ident[0]);
            let u = &p.uses[8][0];
            assert_eq!(u.function, "Test.OnInit");
            assert_eq!(u.call.as_deref(), Some("debug.Notification"));
            assert_eq!(p.uses[12][0].function, "Test");
            assert_eq!(p.uses[9][0].call, None);
        }
    }

    #[test]
    fn writes_in_place_or_at_the_end() {
        for be in [true, false] {
            let b = sample(be);
            let p = parse(&b).unwrap();
            // Nothing: identical.
            assert_eq!(write(&b, &p, &HashMap::new()).unwrap(), b);
            let mut t = HashMap::new();
            t.insert(8usize, "Bonjour, Enfant de dragon !".as_bytes().to_vec());
            t.insert(9usize, "Titre".as_bytes().to_vec());
            t.insert(3usize, "NePasToucher".as_bytes().to_vec()); // a name: ignored
            let out = write(&b, &p, &t).unwrap();
            let q = parse(&out).unwrap();
            assert_eq!(q.strings[8], b"Bonjour, Enfant de dragon !");
            assert_eq!(q.strings[3], b"OnInit");
            // The property name stays; the literal points to a new string.
            assert_eq!(q.strings[9], b"Title");
            assert_eq!(q.strings.len(), p.strings.len() + 1);
            let last = q.strings.len() - 1;
            assert_eq!(q.strings[last], b"Titre");
            assert!(q.is_literal(last) && !q.is_literal(9));
            // Translation removed: identical again.
            assert_eq!(write(&b, &p, &HashMap::new()).unwrap(), b);
        }
    }

    #[test]
    fn unknown_version_and_truncated() {
        let mut b = sample(true);
        b[4] = 9;
        b[5] = 9;
        assert_eq!(parse(&b).err(), Some(PexError::Version(9, 9)));
        let b = sample(false);
        assert_eq!(parse(&b[..b.len() - 3]).err(), Some(PexError::Truncated));
        assert_eq!(parse(b"abcd").err(), Some(PexError::Magic));
    }
}
