//! A JSON reader that keeps positions (D-08, D-88): every string knows the
//! byte span of its escaped content in the text, so a translation can be
//! put in its place without touching anything else — indentation, order of
//! the keys, numbers, comments. Tolerant: `//` and `/* */` comments and
//! trailing commas are read (and kept, since nothing is rewritten).
//!
//! The paths (R-D08-02): `$`, `.key`, `['key']`, `[n]`, `[*]`, `.*`,
//! `..key` (recursive), `..*`, several paths separated by « | ».

/// A value and where it is.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Object(Vec<(String, Node)>),
    Array(Vec<Node>),
    /// The unescaped text.
    Str(String),
    /// A number, `true`, `false`, `null`.
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub value: Value,
    /// For a string: the span of its escaped content (inside the quotes);
    /// otherwise the span of the whole value.
    pub span: (usize, usize),
}

/// Where the reading failed: the byte offset and what was expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub at: usize,
    pub what: &'static str,
}

impl ParseError {
    /// « line:column » of the offset.
    pub fn position(&self, text: &str) -> (usize, usize) {
        let before = &text[..self.at.min(text.len())];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (line, col)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
}

/// Read a JSON text.
pub fn parse(text: &str) -> Result<Node, ParseError> {
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
    };
    p.ws()?;
    let n = p.value()?;
    p.ws()?;
    if p.i < p.s.len() {
        return Err(p.err("end of text"));
    }
    Ok(n)
}

impl<'a> Parser<'a> {
    fn err(&self, what: &'static str) -> ParseError {
        ParseError { at: self.i, what }
    }

    /// Whitespace and comments.
    fn ws(&mut self) -> Result<(), ParseError> {
        loop {
            while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\r' | b'\n') {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                match self.text[self.i + 2..].find("*/") {
                    Some(k) => self.i += k + 4,
                    None => return Err(self.err("*/")),
                }
            } else {
                return Ok(());
            }
        }
    }

    fn value(&mut self) -> Result<Node, ParseError> {
        match self.s.get(self.i) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => {
                let (v, span) = self.string()?;
                Ok(Node {
                    value: Value::Str(v),
                    span,
                })
            }
            Some(_) => {
                let start = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_alphanumeric()
                        || matches!(self.s[self.i], b'-' | b'+' | b'.'))
                {
                    self.i += 1;
                }
                if self.i == start {
                    return Err(self.err("value"));
                }
                Ok(Node {
                    value: Value::Other,
                    span: (start, self.i),
                })
            }
            None => Err(self.err("value")),
        }
    }

    fn object(&mut self) -> Result<Node, ParseError> {
        let start = self.i;
        self.i += 1;
        let mut members = Vec::new();
        loop {
            self.ws()?;
            match self.s.get(self.i) {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b'"') => {}
                _ => return Err(self.err("\" or }")),
            }
            let (key, _) = self.string()?;
            self.ws()?;
            if self.s.get(self.i) != Some(&b':') {
                return Err(self.err(":"));
            }
            self.i += 1;
            self.ws()?;
            let v = self.value()?;
            members.push((key, v));
            self.ws()?;
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(self.err(", or }")),
            }
        }
        Ok(Node {
            value: Value::Object(members),
            span: (start, self.i),
        })
    }

    fn array(&mut self) -> Result<Node, ParseError> {
        let start = self.i;
        self.i += 1;
        let mut items = Vec::new();
        loop {
            self.ws()?;
            if self.s.get(self.i) == Some(&b']') {
                self.i += 1;
                break;
            }
            items.push(self.value()?);
            self.ws()?;
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(self.err(", or ]")),
            }
        }
        Ok(Node {
            value: Value::Array(items),
            span: (start, self.i),
        })
    }

    /// A string: its text and the span of its escaped content.
    fn string(&mut self) -> Result<(String, (usize, usize)), ParseError> {
        self.i += 1;
        let start = self.i;
        let mut escaped = false;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                let end = self.i;
                self.i += 1;
                return Ok((unescape(&self.text[start..end]), (start, end)));
            }
            self.i += 1;
        }
        Err(self.err("\""))
    }
}

/// The text of an escaped JSON string content.
pub fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = (0..4).filter_map(|_| it.next()).collect();
                let Ok(u) = u32::from_str_radix(&hex, 16) else {
                    out.push_str("\\u");
                    out.push_str(&hex);
                    continue;
                };
                if (0xD800..0xDC00).contains(&u) {
                    // A surrogate pair.
                    let mut probe = it.clone();
                    if probe.next() == Some('\\') && probe.next() == Some('u') {
                        let lo: String = (0..4).filter_map(|_| probe.next()).collect();
                        if let Ok(l) = u32::from_str_radix(&lo, 16)
                            && (0xDC00..0xE000).contains(&l)
                        {
                            let cp = 0x10000 + ((u - 0xD800) << 10) + (l - 0xDC00);
                            if let Some(ch) = char::from_u32(cp) {
                                out.push(ch);
                                it = probe;
                                continue;
                            }
                        }
                    }
                    out.push('\u{FFFD}');
                } else {
                    out.push(char::from_u32(u).unwrap_or('\u{FFFD}'));
                }
            }
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

/// The escaped content of a JSON string (non-ASCII kept as it is).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------- paths

/// One step from a node to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Key(String),
    Index(usize),
}

/// One segment of a path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Child(String),
    Index(usize),
    Wild,
    /// `..name` (`None`: `..*`).
    Desc(Option<String>),
}

/// A path that does not parse: the path and the offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError(pub String);

/// Paths separated by « | ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths(Vec<Vec<Seg>>);

impl Paths {
    pub fn parse(s: &str) -> Result<Paths, PathError> {
        let mut out = Vec::new();
        for one in s.split('|') {
            let one = one.trim();
            if one.is_empty() {
                continue;
            }
            out.push(parse_one(one).ok_or_else(|| PathError(one.to_string()))?);
        }
        if out.is_empty() {
            return Err(PathError(s.to_string()));
        }
        Ok(Paths(out))
    }

    /// The trails of the nodes the paths reach, in the order of the text,
    /// each once.
    pub fn select(&self, root: &Node) -> Vec<Vec<Step>> {
        let mut found: Vec<(usize, Vec<Step>)> = Vec::new();
        for segs in &self.0 {
            let mut cur: Vec<Vec<Step>> = vec![Vec::new()];
            for seg in segs {
                let mut next = Vec::new();
                for trail in &cur {
                    let Some(n) = at(root, trail) else { continue };
                    match seg {
                        Seg::Child(k) => {
                            if let Value::Object(m) = &n.value
                                && m.iter().any(|(key, _)| key == k)
                            {
                                next.push(push(trail, Step::Key(k.clone())));
                            }
                        }
                        Seg::Index(i) => {
                            if let Value::Array(a) = &n.value
                                && *i < a.len()
                            {
                                next.push(push(trail, Step::Index(*i)));
                            }
                        }
                        Seg::Wild => children(n, trail, &mut next),
                        Seg::Desc(name) => {
                            let mut all = Vec::new();
                            descendants(n, trail, &mut all);
                            for t in all {
                                let Some(d) = at(root, &t) else { continue };
                                match name {
                                    Some(k) => {
                                        if let Value::Object(m) = &d.value
                                            && m.iter().any(|(key, _)| key == k)
                                        {
                                            next.push(push(&t, Step::Key(k.clone())));
                                        }
                                    }
                                    None => children(d, &t, &mut next),
                                }
                            }
                        }
                    }
                }
                cur = next;
            }
            for t in cur {
                if let Some(n) = at(root, &t) {
                    found.push((n.span.0, t));
                }
            }
        }
        found.sort_by_key(|f| f.0);
        found.dedup_by(|a, b| a.0 == b.0);
        found.into_iter().map(|(_, t)| t).collect()
    }
}

fn push(trail: &[Step], s: Step) -> Vec<Step> {
    let mut t = trail.to_vec();
    t.push(s);
    t
}

fn children(n: &Node, trail: &[Step], out: &mut Vec<Vec<Step>>) {
    match &n.value {
        Value::Object(m) => {
            for (k, _) in m {
                out.push(push(trail, Step::Key(k.clone())));
            }
        }
        Value::Array(a) => {
            for i in 0..a.len() {
                out.push(push(trail, Step::Index(i)));
            }
        }
        _ => {}
    }
}

/// The node itself and all below it.
fn descendants(n: &Node, trail: &[Step], out: &mut Vec<Vec<Step>>) {
    out.push(trail.to_vec());
    match &n.value {
        Value::Object(m) => {
            for (k, v) in m {
                descendants(v, &push(trail, Step::Key(k.clone())), out);
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                descendants(v, &push(trail, Step::Index(i)), out);
            }
        }
        _ => {}
    }
}

/// The node at the end of a trail.
pub fn at<'n>(root: &'n Node, trail: &[Step]) -> Option<&'n Node> {
    let mut n = root;
    for s in trail {
        n = match (s, &n.value) {
            (Step::Key(k), Value::Object(m)) => &m.iter().find(|(key, _)| key == k)?.1,
            (Step::Index(i), Value::Array(a)) => a.get(*i)?,
            _ => return None,
        };
    }
    Some(n)
}

/// A trail as text: `pages[1].content[3].text`.
pub fn trail_text(trail: &[Step]) -> String {
    let mut s = String::new();
    for st in trail {
        match st {
            Step::Key(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s.push_str(k);
            }
            Step::Index(i) => s.push_str(&format!("[{i}]")),
        }
    }
    s
}

fn parse_one(p: &str) -> Option<Vec<Seg>> {
    let p = p.strip_prefix('$').unwrap_or(p);
    let b: Vec<char> = p.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    let name = |b: &[char], i: &mut usize| -> String {
        let start = *i;
        while *i < b.len() && !matches!(b[*i], '.' | '[') {
            *i += 1;
        }
        b[start..*i].iter().collect()
    };
    // A path may start with a bare name (`content[*].text`).
    if i < b.len() && !matches!(b[i], '.' | '[') {
        let n = name(&b, &mut i);
        out.push(if n == "*" { Seg::Wild } else { Seg::Child(n) });
    }
    while i < b.len() {
        match b[i] {
            '.' if b.get(i + 1) == Some(&'.') => {
                i += 2;
                if b.get(i) == Some(&'[') {
                    // `..[*]`
                    let close = b[i..].iter().position(|&c| c == ']')? + i;
                    let inner: String = b[i + 1..close].iter().collect();
                    if inner.trim() != "*" {
                        return None;
                    }
                    out.push(Seg::Desc(None));
                    i = close + 1;
                } else {
                    let n = name(&b, &mut i);
                    if n.is_empty() {
                        return None;
                    }
                    out.push(Seg::Desc(if n == "*" { None } else { Some(n) }));
                }
            }
            '.' => {
                i += 1;
                let n = name(&b, &mut i);
                if n.is_empty() {
                    return None;
                }
                out.push(if n == "*" { Seg::Wild } else { Seg::Child(n) });
            }
            '[' => {
                let close = b[i..].iter().position(|&c| c == ']')? + i;
                let inner: String = b[i + 1..close].iter().collect();
                let inner = inner.trim();
                if inner == "*" {
                    out.push(Seg::Wild);
                } else if let Ok(n) = inner.parse::<usize>() {
                    out.push(Seg::Index(n));
                } else if (inner.starts_with('\'') && inner.ends_with('\'') && inner.len() >= 2)
                    || (inner.starts_with('"') && inner.ends_with('"') && inner.len() >= 2)
                {
                    out.push(Seg::Child(inner[1..inner.len() - 1].to_string()));
                } else {
                    return None;
                }
                i = close + 1;
            }
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_point_at_the_escaped_content() {
        let t = "{\n  // a comment\n  \"a\": \"x\\\"y\", \"n\": 3,\n  \"l\": [\"é\", {\"b\": \"z\"},],\n}";
        let root = parse(t).unwrap();
        let Value::Object(m) = &root.value else {
            panic!()
        };
        let a = &m[0].1;
        assert_eq!(a.value, Value::Str("x\"y".into()));
        assert_eq!(&t[a.span.0..a.span.1], "x\\\"y");
        assert_eq!(m[1].1.value, Value::Other);
        let p = Paths::parse("$..b | $.l[0]").unwrap();
        let trails = p.select(&root);
        assert_eq!(trail_text(&trails[0]), "l[0]");
        assert_eq!(trail_text(&trails[1]), "l[1].b");
        assert!(Paths::parse("$.[").is_err());
        assert!(parse("{\"a\": }").is_err());
        assert_eq!(unescape("\\ud83d\\ude00 \\u00e9"), "😀 é");
        assert_eq!(escape("a\"b\n\u{1}"), "a\\\"b\\n\\u0001");
    }

    #[test]
    fn wildcards_and_recursion() {
        let t = r#"{"pages":[{"content":[{"text":"A","options":["x","y"]}]}],"text":"T"}"#;
        let root = parse(t).unwrap();
        let all = Paths::parse("$..text | $..options[*]")
            .unwrap()
            .select(&root);
        let s: Vec<String> = all.iter().map(|t| trail_text(t)).collect();
        assert_eq!(
            s,
            vec![
                "pages[0].content[0].text",
                "pages[0].content[0].options[0]",
                "pages[0].content[0].options[1]",
                "text"
            ]
        );
        let p = Paths::parse("pages[*].content[*].text")
            .unwrap()
            .select(&root);
        assert_eq!(p.len(), 1);
        assert_eq!(Paths::parse("$..*").unwrap().select(&root).len(), 9);
    }
}
