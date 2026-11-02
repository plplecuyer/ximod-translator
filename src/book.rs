//! Book text parsing for the in-UI preview (CDC §8 — aperçus).
//!
//! Bethesda book bodies come in two flavours:
//!
//! - **plain text** (older lore books, and most translation-memory corpora) —
//!   paragraphs separated by blank lines, single line breaks kept;
//! - **marked-up** (Skyrim SE and many mods) — with `[pagebreak]`, `<p>` /
//!   `<p align='center'>`, `<br>`, `<font face='$HandwrittenFont'>…</font>`,
//!   `<img src='…'>`, and simple `<b>`/`<i>`.
//!
//! This module turns either into a flat list of [`Block`]s the UI can render.
//! It is deliberately tolerant: unknown tags are dropped, malformed tags kept as
//! text, and both CRLF and LF are handled.

/// Paragraph alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// A styled run of text within a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub italic: bool,
    pub bold: bool,
    /// The requested Bethesda font face (e.g. `"$HandwrittenFont"`), if any, so
    /// the renderer can map it to a user-supplied game font.
    pub face: Option<String>,
}

/// A renderable block of a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph { runs: Vec<Run>, align: Align },
    PageBreak,
    Image(String),
}

/// Whether the text carries any Bethesda book markup (so the caller can offer a
/// richer preview / know a mod's books are formatted).
pub fn has_markup(text: &str) -> bool {
    let l = text.to_ascii_lowercase();
    l.contains("[pagebreak]")
        || l.contains("<p>")
        || l.contains("<p ")
        || l.contains("<br")
        || l.contains("<font")
        || l.contains("<img")
        || l.contains("<div")
}

#[derive(Default)]
struct Parser {
    blocks: Vec<Block>,
    runs: Vec<Run>,
    buf: String,
    align: Align,
    italic: i32,
    bold: i32,
    /// Current requested font face (from the innermost open `<font face=…>`).
    face: Option<String>,
    /// Per open `<font>`: (italic increment, bold increment, previous face) to
    /// restore on `</font>`.
    font_stack: Vec<(i32, i32, Option<String>)>,
}

impl Parser {
    fn flush_run(&mut self) {
        if !self.buf.is_empty() {
            let text = std::mem::take(&mut self.buf);
            self.runs.push(Run {
                text,
                italic: self.italic > 0,
                bold: self.bold > 0,
                face: self.face.clone(),
            });
        }
    }

    fn flush_para(&mut self) {
        self.flush_run();
        if !self.runs.is_empty() {
            let runs = std::mem::take(&mut self.runs);
            self.blocks.push(Block::Paragraph {
                runs,
                align: self.align,
            });
        }
    }

    fn open_tag(&mut self, raw: &str) {
        let inner = raw.trim();
        let closing = inner.starts_with('/');
        let name_part = inner.trim_start_matches('/');
        let name: String = name_part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if closing {
            match name.as_str() {
                "font" => {
                    if let Some((i, b, prev)) = self.font_stack.pop() {
                        self.flush_run();
                        self.italic -= i;
                        self.bold -= b;
                        self.face = prev;
                    }
                }
                "b" | "strong" => {
                    self.flush_run();
                    self.bold = (self.bold - 1).max(0);
                }
                "i" | "em" => {
                    self.flush_run();
                    self.italic = (self.italic - 1).max(0);
                }
                _ => {}
            }
            return;
        }
        match name.as_str() {
            "p" | "div" => {
                self.flush_para();
                self.align = parse_align(inner);
            }
            "br" => self.buf.push('\n'),
            "font" => {
                self.flush_run();
                let face = attr(inner, "face").filter(|f| !f.is_empty());
                let it = face
                    .as_ref()
                    .map(|f| f.to_ascii_lowercase().contains("hand"))
                    .unwrap_or(false) as i32;
                self.italic += it;
                self.font_stack.push((it, 0, self.face.clone()));
                if face.is_some() {
                    self.face = face;
                }
            }
            "img" => {
                self.flush_para();
                let src = attr(inner, "src").unwrap_or_default();
                self.blocks.push(Block::Image(src));
            }
            "b" | "strong" => {
                self.flush_run();
                self.bold += 1;
            }
            "i" | "em" => {
                self.flush_run();
                self.italic += 1;
            }
            _ => {} // unknown tag → dropped
        }
    }
}

/// Parse a book body into renderable blocks.
pub fn parse(text: &str) -> Vec<Block> {
    let norm = text.replace("\r\n", "\n").replace('\r', "\n");
    let chars: Vec<char> = norm.chars().collect();
    let n = chars.len();
    let mut p = Parser::default();
    let mut i = 0;
    while i < n {
        // [pagebreak] (case-insensitive).
        if chars[i] == '[' && starts_with_ci(&chars[i..], "[pagebreak]") {
            p.flush_para();
            p.align = Align::Left;
            p.blocks.push(Block::PageBreak);
            i += "[pagebreak]".len();
            continue;
        }
        // A tag <...>.
        if chars[i] == '<' {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == '>') {
                let raw: String = chars[i + 1..i + 1 + rel].iter().collect();
                p.open_tag(&raw);
                i += rel + 2;
                continue;
            }
            // No closing '>' → literal.
            p.buf.push('<');
            i += 1;
            continue;
        }
        // Newlines: 2+ (ignoring inline spaces/tabs) → paragraph break; else soft.
        if chars[i] == '\n' {
            let mut j = i + 1;
            let mut nl = 1;
            while j < n && (chars[j] == '\n' || chars[j] == ' ' || chars[j] == '\t') {
                if chars[j] == '\n' {
                    nl += 1;
                }
                j += 1;
            }
            if nl >= 2 {
                p.flush_para();
                p.align = Align::Left;
            } else {
                p.buf.push('\n');
            }
            i = j;
            continue;
        }
        p.buf.push(chars[i]);
        i += 1;
    }
    p.flush_para();
    p.blocks
}

/// Whether `hay` (a char slice) starts with `needle`, case-insensitively.
fn starts_with_ci(hay: &[char], needle: &str) -> bool {
    let nd: Vec<char> = needle.chars().collect();
    if hay.len() < nd.len() {
        return false;
    }
    hay.iter()
        .zip(nd.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Extract an attribute value (`name='v'`, `name="v"`, or `name=v`) from a tag's
/// inner text, case-insensitive on the name.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{}=", name.to_ascii_lowercase());
    let pos = lower.find(&key)? + key.len();
    let rest = &tag[pos..];
    let bytes = rest.as_bytes();
    if bytes.is_empty() {
        return Some(String::new());
    }
    let (quote, start) = match bytes[0] {
        b'\'' => (Some('\''), 1),
        b'"' => (Some('"'), 1),
        _ => (None, 0),
    };
    let rest2 = &rest[start..];
    let end = match quote {
        Some(q) => rest2.find(q).unwrap_or(rest2.len()),
        None => rest2
            .find(|c: char| c.is_whitespace())
            .unwrap_or(rest2.len()),
    };
    Some(rest2[..end].to_string())
}

fn parse_align(tag: &str) -> Align {
    match attr(tag, "align")
        .map(|a| a.to_ascii_lowercase())
        .as_deref()
    {
        Some("center") => Align::Center,
        Some("right") => Align::Right,
        _ => Align::Left,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para_text(b: &Block) -> String {
        match b {
            Block::Paragraph { runs, .. } => runs.iter().map(|r| r.text.as_str()).collect(),
            _ => String::new(),
        }
    }

    #[test]
    fn plain_paragraphs_split_on_blank_lines() {
        let blocks = parse("Title line\r\n \r\nFirst para.\r\n \r\nSecond para.");
        let paras: Vec<String> = blocks.iter().map(para_text).collect();
        assert_eq!(paras, ["Title line", "First para.", "Second para."]);
    }

    #[test]
    fn single_newline_is_a_soft_break() {
        let blocks = parse("line one\nline two");
        assert_eq!(blocks.len(), 1);
        assert_eq!(para_text(&blocks[0]), "line one\nline two");
    }

    #[test]
    fn pagebreak_and_paragraphs() {
        let blocks = parse("Page one text[pagebreak]Page two text");
        assert!(matches!(blocks[1], Block::PageBreak));
        assert_eq!(para_text(&blocks[0]), "Page one text");
        assert_eq!(para_text(&blocks[2]), "Page two text");
    }

    #[test]
    fn font_handwritten_is_italic() {
        let blocks = parse("<font face='$HandwrittenFont'>a note</font>");
        if let Block::Paragraph { runs, .. } = &blocks[0] {
            assert_eq!(runs[0].text, "a note");
            assert!(runs[0].italic);
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn p_align_center_and_tags() {
        let blocks = parse("<p align='center'>Centered</p><p>Left para</p>");
        assert!(matches!(
            blocks[0],
            Block::Paragraph {
                align: Align::Center,
                ..
            }
        ));
        assert_eq!(para_text(&blocks[0]), "Centered");
        assert!(matches!(
            blocks[1],
            Block::Paragraph {
                align: Align::Left,
                ..
            }
        ));
    }

    #[test]
    fn img_becomes_a_block_and_unknown_tags_dropped() {
        let blocks = parse("before<img src='book.dds' width=100>after<unknown>x</unknown>");
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Image(s) if s == "book.dds"))
        );
        let text: String = blocks.iter().map(para_text).collect();
        assert!(text.contains("before") && text.contains("after") && text.contains('x'));
    }

    #[test]
    fn br_is_a_line_break() {
        let blocks = parse("a<br>b");
        assert_eq!(para_text(&blocks[0]), "a\nb");
    }

    #[test]
    fn font_face_is_captured_and_restored() {
        let blocks = parse("<font face='$HandwrittenFont'>note</font> plain");
        if let Block::Paragraph { runs, .. } = &blocks[0] {
            assert_eq!(runs[0].face.as_deref(), Some("$HandwrittenFont"));
            assert!(runs[0].italic);
            // After </font>, the face is cleared for following text.
            assert!(
                runs.iter()
                    .any(|r| r.text.contains("plain") && r.face.is_none())
            );
        } else {
            panic!("expected paragraph");
        }
    }

    #[test]
    fn has_markup_detection() {
        assert!(has_markup("hello [pagebreak] world"));
        assert!(has_markup("<p>x</p>"));
        assert!(has_markup("<font face='x'>y</font>"));
        assert!(!has_markup(
            "just plain prose, even with [sic] editorial notes"
        ));
    }
}
