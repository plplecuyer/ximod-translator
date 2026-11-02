//! A CSV reader that keeps positions (D-08, D-88): each cell with the span
//! of its text (quotes included) so a translation can be written in its
//! place, the rest of the file untouched. The separator is given or found
//! in the first line (`;`, `,`, tab); `"…"` quotes with `""` inside.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// The text, quotes removed and `""` made `"`.
    pub value: String,
    /// The whole cell in the text, quotes included.
    pub span: (usize, usize),
    pub quoted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    pub sep: char,
    pub rows: Vec<Vec<Cell>>,
    /// Where each row's content ends (before its line break).
    pub row_end: Vec<usize>,
}

/// The separator of a file: the most frequent of `;`, `,` and tab in its
/// first line, outside quotes.
pub fn detect_sep(text: &str) -> char {
    let first = text.lines().next().unwrap_or("");
    let mut counts = [(';', 0usize), (',', 0), ('\t', 0)];
    let mut quoted = false;
    for c in first.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted {
            for x in counts.iter_mut() {
                if x.0 == c {
                    x.1 += 1;
                }
            }
        }
    }
    counts
        .iter()
        .max_by_key(|x| x.1)
        .filter(|x| x.1 > 0)
        .map_or(';', |x| x.0)
}

/// The separator a format names (`;`, `,`, `tab`, `\t`, empty = found).
pub fn sep_of(setting: &str, text: &str) -> char {
    match setting.trim() {
        "" | "auto" => detect_sep(text),
        "tab" | "\\t" => '\t',
        s => s.chars().next().unwrap_or(';'),
    }
}

/// Read a CSV text.
pub fn parse(text: &str, sep: char) -> Table {
    let b = text.as_bytes();
    let sepb = sep as u8;
    let mut t = Table {
        sep,
        ..Default::default()
    };
    let mut i = 0usize;
    while i < b.len() {
        let mut row = Vec::new();
        loop {
            // One cell.
            let start = i;
            let mut value = String::new();
            let quoted = b.get(i) == Some(&b'"');
            if quoted {
                i += 1;
                let mut seg = i;
                while i < b.len() {
                    if b[i] == b'"' {
                        if b.get(i + 1) == Some(&b'"') {
                            value.push_str(&text[seg..=i]);
                            i += 2;
                            seg = i;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                value.push_str(&text[seg..i.min(b.len())]);
                i = (i + 1).min(b.len());
                // Anything up to the separator belongs to the cell.
                while i < b.len() && b[i] != sepb && b[i] != b'\n' && b[i] != b'\r' {
                    i += 1;
                }
            } else {
                while i < b.len() && b[i] != sepb && b[i] != b'\n' && b[i] != b'\r' {
                    i += 1;
                }
                value.push_str(&text[start..i]);
            }
            row.push(Cell {
                value,
                span: (start, i),
                quoted,
            });
            if i < b.len() && b[i] == sepb {
                i += 1;
                continue;
            }
            break;
        }
        t.row_end.push(i);
        if b.get(i) == Some(&b'\r') {
            i += 1;
        }
        if b.get(i) == Some(&b'\n') {
            i += 1;
        }
        // A blank line is no row.
        if !(row.len() == 1 && row[0].value.is_empty() && !row[0].quoted) {
            t.rows.push(row);
        } else {
            t.row_end.pop();
        }
    }
    t
}

/// A cell's text for a value: quoted when the cell was, or when the value
/// needs it.
pub fn quote(value: &str, sep: char, was_quoted: bool) -> String {
    let needs = value.contains(sep)
        || value.contains('"')
        || value.contains('\n')
        || value.contains('\r')
        || value.starts_with(' ')
        || value.ends_with(' ');
    if was_quoted || needs {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_and_spans() {
        let t = "id;source;translation\r\nk1;\"Hello; \"\"you\"\"\";\r\n\r\nk2;Bye;Au revoir\r\n";
        assert_eq!(detect_sep(t), ';');
        let tab = parse(t, ';');
        assert_eq!(tab.rows.len(), 3);
        let c = &tab.rows[1][1];
        assert_eq!(c.value, "Hello; \"you\"");
        assert!(c.quoted);
        assert_eq!(&t[c.span.0..c.span.1], "\"Hello; \"\"you\"\"\"");
        assert_eq!(tab.rows[1][2].value, "");
        assert_eq!(tab.rows[1][2].span.0, tab.rows[1][2].span.1);
        assert_eq!(tab.rows[2][2].value, "Au revoir");
        assert_eq!(quote("a;b", ';', false), "\"a;b\"");
        assert_eq!(quote("a", ';', true), "\"a\"");
        assert_eq!(sep_of("tab", ""), '\t');
        assert_eq!(detect_sep("a,b,c\n"), ',');
    }
}
