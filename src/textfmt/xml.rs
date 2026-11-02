//! An XML reader that keeps positions (D-08, D-88): elements, attributes
//! and text with the byte span of each value in the text, comments,
//! processing instructions, CDATA and the doctype skipped over (and kept,
//! since nothing is rewritten). The restricted XPath of R-D08-02: `/a/b`,
//! `//b`, `*`, `@attr`, `@*`, `text()`, `[@k='v']`, several paths
//! separated by « | ».

/// An element.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Child>,
    pub parent: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    /// The unescaped value.
    pub value: String,
    /// The span of the escaped value (inside the quotes).
    pub span: (usize, usize),
    pub quote: char,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Child {
    Element(usize),
    Text(TextNode),
}

/// A text between tags, its surrounding blanks left out.
#[derive(Debug, Clone, PartialEq)]
pub struct TextNode {
    pub value: String,
    pub span: (usize, usize),
    pub cdata: bool,
}

/// The elements of a document; the top-level ones have no parent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
    pub elements: Vec<Element>,
    pub top: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub at: usize,
    pub what: &'static str,
}

/// Read an XML text.
pub fn parse(text: &str) -> Result<Doc, ParseError> {
    let s = text.as_bytes();
    let mut doc = Doc::default();
    let mut stack: Vec<usize> = Vec::new();
    let mut i = 0usize;
    let err = |at: usize, what: &'static str| ParseError { at, what };
    let mut text_start = 0usize;
    let flush_text = |doc: &mut Doc, stack: &[usize], from: usize, to: usize| {
        if from >= to {
            return;
        }
        let raw = &text[from..to];
        let lead = raw.len() - raw.trim_start().len();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        let node = TextNode {
            value: unescape(trimmed),
            span: (from + lead, from + lead + trimmed.len()),
            cdata: false,
        };
        if let Some(&p) = stack.last() {
            doc.elements[p].children.push(Child::Text(node));
        }
    };
    while i < s.len() {
        if s[i] != b'<' {
            i += 1;
            continue;
        }
        flush_text(&mut doc, &stack, text_start, i);
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or(err(i, "-->"))? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").ok_or(err(i, "]]>"))?;
            let (a, b) = (i + 9, i + end);
            if let Some(&p) = stack.last()
                && !text[a..b].trim().is_empty()
            {
                doc.elements[p].children.push(Child::Text(TextNode {
                    value: text[a..b].to_string(),
                    span: (a, b),
                    cdata: true,
                }));
            }
            i += end + 3;
        } else if rest.starts_with("<?") {
            i += rest.find("?>").ok_or(err(i, "?>"))? + 2;
        } else if rest.starts_with("<!") {
            // A doctype, internal subset included.
            let mut depth = 0i32;
            let mut j = i + 2;
            while j < s.len() {
                match s[j] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    b'>' if depth <= 0 => break,
                    _ => {}
                }
                j += 1;
            }
            i = j + 1;
        } else if rest.starts_with("</") {
            let end = rest.find('>').ok_or(err(i, ">"))?;
            let name = rest[2..end].trim();
            match stack.pop() {
                Some(e) if doc.elements[e].name == name => {}
                _ => return Err(err(i, "matching end tag")),
            }
            i += end + 1;
        } else {
            // A start tag.
            let mut j = i + 1;
            while j < s.len() && !matches!(s[j], b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>') {
                j += 1;
            }
            let name = text[i + 1..j].to_string();
            if name.is_empty() {
                return Err(err(i, "element name"));
            }
            let mut attrs = Vec::new();
            let closed;
            loop {
                while j < s.len() && matches!(s[j], b' ' | b'\t' | b'\r' | b'\n') {
                    j += 1;
                }
                match s.get(j) {
                    Some(b'>') => {
                        closed = false;
                        j += 1;
                        break;
                    }
                    Some(b'/') if s.get(j + 1) == Some(&b'>') => {
                        closed = true;
                        j += 2;
                        break;
                    }
                    Some(_) => {}
                    None => return Err(err(j, ">")),
                }
                let a0 = j;
                while j < s.len() && !matches!(s[j], b'=' | b' ' | b'\t' | b'\r' | b'\n' | b'>') {
                    j += 1;
                }
                let aname = text[a0..j].to_string();
                while j < s.len() && matches!(s[j], b' ' | b'\t' | b'\r' | b'\n') {
                    j += 1;
                }
                if s.get(j) != Some(&b'=') {
                    return Err(err(j, "="));
                }
                j += 1;
                while j < s.len() && matches!(s[j], b' ' | b'\t' | b'\r' | b'\n') {
                    j += 1;
                }
                let q = match s.get(j) {
                    Some(&c @ (b'"' | b'\'')) => c,
                    _ => return Err(err(j, "quote")),
                };
                let v0 = j + 1;
                let v1 = text[v0..].find(q as char).ok_or(err(v0, "quote"))? + v0;
                attrs.push(Attr {
                    name: aname,
                    value: unescape(&text[v0..v1]),
                    span: (v0, v1),
                    quote: q as char,
                });
                j = v1 + 1;
            }
            let idx = doc.elements.len();
            doc.elements.push(Element {
                name,
                attrs,
                children: Vec::new(),
                parent: stack.last().copied(),
            });
            match stack.last() {
                Some(&p) => doc.elements[p].children.push(Child::Element(idx)),
                None => doc.top.push(idx),
            }
            if !closed {
                stack.push(idx);
            }
            i = j;
        }
        text_start = i;
    }
    if !stack.is_empty() {
        return Err(err(s.len(), "end tag"));
    }
    Ok(doc)
}

/// The text of escaped XML (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`,
/// `&#n;`, `&#xh;`).
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.find('&') {
        out.push_str(&rest[..k]);
        rest = &rest[k..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escaped text for a text node (`quote: None`) or an attribute.
pub fn escape(s: &str, quote: Option<char>) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if quote == Some('"') => out.push_str("&quot;"),
            '\'' if quote == Some('\'') => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------- paths

/// What a path reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Hit {
    /// An attribute: element, attribute's rank.
    Attr(usize, usize),
    /// A text: element, child's rank.
    Text(usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Test {
    Name(String),
    Any,
    Attr(String),
    AnyAttr,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct XStep {
    desc: bool,
    test: Test,
    preds: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XPaths(Vec<Vec<XStep>>);

impl XPaths {
    pub fn parse(s: &str) -> Result<XPaths, PathError> {
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
        Ok(XPaths(out))
    }

    /// The values the paths reach: attributes, texts — an element reached
    /// gives its texts.
    pub fn select(&self, doc: &Doc) -> Vec<Hit> {
        let mut hits = Vec::new();
        for steps in &self.0 {
            // `None` is the document.
            let mut cur: Vec<Option<usize>> = vec![None];
            let mut last_hits: Vec<Hit> = Vec::new();
            for (k, st) in steps.iter().enumerate() {
                let final_step = k + 1 == steps.len();
                let mut scope: Vec<Option<usize>> = Vec::new();
                for c in &cur {
                    if st.desc {
                        scope.push(*c);
                        let mut all = Vec::new();
                        below(doc, *c, &mut all);
                        scope.extend(all.into_iter().map(Some));
                    } else {
                        scope.push(*c);
                    }
                }
                let mut next = Vec::new();
                match &st.test {
                    Test::Name(_) | Test::Any => {
                        for c in scope {
                            for e in kids(doc, c) {
                                let el = &doc.elements[e];
                                let name_ok = match &st.test {
                                    Test::Name(n) => el.name == *n,
                                    _ => true,
                                };
                                let preds_ok = st.preds.iter().all(|(a, v)| {
                                    el.attrs.iter().any(|x| x.name == *a && x.value == *v)
                                });
                                if name_ok && preds_ok && !next.contains(&Some(e)) {
                                    next.push(Some(e));
                                }
                            }
                        }
                    }
                    Test::Attr(_) | Test::AnyAttr | Test::Text => {
                        for c in scope.into_iter().flatten() {
                            let el = &doc.elements[c];
                            match &st.test {
                                Test::Attr(n) => {
                                    if let Some(r) = el.attrs.iter().position(|a| a.name == *n) {
                                        last_hits.push(Hit::Attr(c, r));
                                    }
                                }
                                Test::AnyAttr => {
                                    last_hits.extend((0..el.attrs.len()).map(|r| Hit::Attr(c, r)))
                                }
                                _ => {
                                    for (r, ch) in el.children.iter().enumerate() {
                                        if matches!(ch, Child::Text(_)) {
                                            last_hits.push(Hit::Text(c, r));
                                        }
                                    }
                                }
                            }
                        }
                        // An attribute or a text ends the path.
                        let _ = final_step;
                        cur = Vec::new();
                        break;
                    }
                }
                cur = next;
            }
            // Elements reached: their texts.
            for e in cur.into_iter().flatten() {
                for (r, ch) in doc.elements[e].children.iter().enumerate() {
                    if matches!(ch, Child::Text(_)) {
                        last_hits.push(Hit::Text(e, r));
                    }
                }
            }
            hits.extend(last_hits);
        }
        hits.sort_by_key(|h| doc.span(*h).0);
        hits.dedup();
        hits
    }
}

impl Doc {
    /// The span of a hit's value.
    pub fn span(&self, h: Hit) -> (usize, usize) {
        match h {
            Hit::Attr(e, r) => self.elements[e].attrs[r].span,
            Hit::Text(e, r) => match &self.elements[e].children[r] {
                Child::Text(t) => t.span,
                Child::Element(_) => (0, 0),
            },
        }
    }

    /// The path of an element: `/strings/string[3]` (rank among the
    /// siblings of the same name, from 1, when there are several).
    pub fn path_of(&self, e: usize) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(e);
        while let Some(c) = cur {
            let el = &self.elements[c];
            let sibs: Vec<usize> = match el.parent {
                Some(p) => self.elements[p]
                    .children
                    .iter()
                    .filter_map(|ch| match ch {
                        Child::Element(x) if self.elements[*x].name == el.name => Some(*x),
                        _ => None,
                    })
                    .collect(),
                None => self
                    .top
                    .iter()
                    .copied()
                    .filter(|x| self.elements[*x].name == el.name)
                    .collect(),
            };
            if sibs.len() > 1 {
                let k = sibs.iter().position(|x| *x == c).unwrap_or(0) + 1;
                parts.push(format!("{}[{k}]", el.name));
            } else {
                parts.push(el.name.clone());
            }
            cur = el.parent;
        }
        parts.reverse();
        format!("/{}", parts.join("/"))
    }

    /// The value of an attribute on the element or its nearest ancestor.
    pub fn attr_up(&self, e: usize, name: &str) -> Option<String> {
        let mut cur = Some(e);
        while let Some(c) = cur {
            if let Some(a) = self.elements[c].attrs.iter().find(|a| a.name == name) {
                return Some(a.value.clone());
            }
            cur = self.elements[c].parent;
        }
        None
    }

    /// Every attribute value and text, for the counts.
    pub fn all_values(&self) -> Vec<Hit> {
        let mut out = Vec::new();
        for (e, el) in self.elements.iter().enumerate() {
            out.extend((0..el.attrs.len()).map(|r| Hit::Attr(e, r)));
            for (r, ch) in el.children.iter().enumerate() {
                if matches!(ch, Child::Text(_)) {
                    out.push(Hit::Text(e, r));
                }
            }
        }
        out
    }
}

fn kids(doc: &Doc, c: Option<usize>) -> Vec<usize> {
    match c {
        None => doc.top.clone(),
        Some(e) => doc.elements[e]
            .children
            .iter()
            .filter_map(|ch| match ch {
                Child::Element(x) => Some(*x),
                _ => None,
            })
            .collect(),
    }
}

/// Every element below a node.
fn below(doc: &Doc, c: Option<usize>, out: &mut Vec<usize>) {
    for k in kids(doc, c) {
        out.push(k);
        below(doc, Some(k), out);
    }
}

fn parse_one(p: &str) -> Option<Vec<XStep>> {
    let mut out = Vec::new();
    let mut rest = p.trim();
    // A relative path starts below the document, anywhere.
    let mut desc = !rest.starts_with('/');
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("//") {
            desc = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('/') {
            rest = r;
        }
        // The step, up to the next `/` outside brackets.
        let mut depth = 0;
        let mut end = rest.len();
        for (k, c) in rest.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                '/' if depth == 0 => {
                    end = k;
                    break;
                }
                _ => {}
            }
        }
        let step = &rest[..end];
        rest = &rest[end..];
        if step.is_empty() {
            return None;
        }
        let (head, preds) = match step.find('[') {
            Some(k) => (&step[..k], &step[k..]),
            None => (step, ""),
        };
        let test = match head {
            "*" => Test::Any,
            "@*" => Test::AnyAttr,
            "text()" => Test::Text,
            h if h.starts_with('@') && h.len() > 1 => Test::Attr(h[1..].to_string()),
            h if h
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ':' | '.')) =>
            {
                Test::Name(h.to_string())
            }
            _ => return None,
        };
        let mut pv = Vec::new();
        let mut pr = preds;
        while !pr.is_empty() {
            let close = pr.find(']')?;
            let inner = pr[1..close].trim();
            let inner = inner.strip_prefix('@')?;
            let (k, v) = inner.split_once('=')?;
            let v = v.trim();
            let v = v
                .strip_prefix('\'')
                .and_then(|v| v.strip_suffix('\''))
                .or_else(|| v.strip_prefix('"').and_then(|v| v.strip_suffix('"')))?;
            pv.push((k.trim().to_string(), v.to_string()));
            pr = &pr[close + 1..];
        }
        out.push(XStep {
            desc,
            test,
            preds: pv,
        });
        desc = false;
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "<?xml version=\"1.0\"?>\n<!-- c -->\n<strings>\n  <string id=\"a\">Fish &amp; chips</string>\n  <string id=\"b\"><![CDATA[<b>Bold</b>]]></string>\n  <item label='Sword &lt;1&gt;' kind=\"x\"/>\n  <item label=\"Bow\" kind=\"y\">\n    text\n  </item>\n</strings>\n";

    #[test]
    fn values_with_their_spans() {
        let d = parse(T).unwrap();
        assert_eq!(d.top.len(), 1);
        let hits = XPaths::parse("//string/text()").unwrap().select(&d);
        assert_eq!(hits.len(), 2);
        let (a, b) = d.span(hits[0]);
        assert_eq!(&T[a..b], "Fish &amp; chips");
        let Child::Text(t) = &d.elements[1].children[0] else {
            panic!()
        };
        assert_eq!(t.value, "Fish & chips");
        let labels = XPaths::parse("//item/@label").unwrap().select(&d);
        assert_eq!(labels.len(), 2);
        let (a, b) = d.span(labels[0]);
        assert_eq!(&T[a..b], "Sword &lt;1&gt;");
        let y = XPaths::parse("/strings/item[@kind='y']")
            .unwrap()
            .select(&d);
        assert_eq!(y.len(), 1);
        let (a, b) = d.span(y[0]);
        assert_eq!(&T[a..b], "text");
        assert_eq!(d.path_of(3), "/strings/item[1]");
        assert_eq!(d.attr_up(1, "id").as_deref(), Some("a"));
        assert!(parse("<a><b></a>").is_err());
        assert!(XPaths::parse("//a[").is_err());
        assert_eq!(
            escape("a<b & \"c\"", Some('"')),
            "a&lt;b &amp; &quot;c&quot;"
        );
        assert_eq!(unescape("&#233;&#xE9;&bad;"), "éé&bad;");
    }
}
