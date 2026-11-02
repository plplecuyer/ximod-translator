//! The timestamped history of the translations (A-18): every undoable
//! operation leaves one entry per line it changed — date and time, the
//! line's identity, translation, status and review before and after, the
//! provenance after, the operation —, kept per working file and written,
//! append only, next to the mod's dictionary (`<dictionary>.ximhist`, JSON
//! Lines, decision D-46). Pure library; the interface shows it and goes back
//! to a version.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tm::TmEntry;
use crate::undo::{EntryState, LogEvent, OpKind};

/// Default number of entries kept per file (`[History] MaxEntries`).
pub const DEFAULT_MAX: usize = 20_000;
/// Bounds of `MaxEntries`.
pub const MAX_RANGE: std::ops::RangeInclusive<usize> = 1_000..=200_000;

/// The identity of a line: record, field, FormID, EDID and rank in the
/// record — the key of the mod's dictionary.
pub type Key = (String, String, u32, String, i32);

pub fn key_of(e: &TmEntry) -> Key {
    (
        e.rec_sig.clone(),
        e.field_sig.clone(),
        e.formid,
        e.edid.clone(),
        e.index,
    )
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One entry of the history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Local date and time, `YYYY-MM-DDTHH:MM:SS`.
    pub t: String,
    pub k: Key,
    /// The source of the line at the time (« source différente » after an
    /// update of the mod).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub s: String,
    /// Translation before and after.
    #[serde(default)]
    pub b: String,
    #[serde(default)]
    pub a: String,
    /// Status keys before and after.
    #[serde(default)]
    pub sb: String,
    #[serde(default)]
    pub sa: String,
    /// Provenance after.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub p: String,
    /// The operation: its code ([`OpKind::code`]) and the argument of its name.
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg: Option<String>,
    /// An undo of that operation (« Annulation : … »).
    #[serde(default, skip_serializing_if = "is_false")]
    pub u: bool,
    /// Review before and after, when the operation touched it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ra: Option<String>,
}

impl Entry {
    /// Whether the entry records a change of the translation or the status
    /// (not of the review alone).
    pub fn changes_translation(&self) -> bool {
        self.b != self.a || self.sb != self.sa
    }
}

/// `YYYY-MM-DDTHH:MM:SS` (local time) of a moment in seconds.
pub fn iso(secs: i64) -> String {
    let (y, mo, d, h, mi, s) = crate::backup::local_datetime(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}

/// The parts of an ISO time (year, month, day, hour, minute, second).
pub fn iso_parts(t: &str) -> Option<(String, String, String, String, String, String)> {
    let (date, time) = t.split_once('T')?;
    let mut d = date.split('-');
    let mut h = time.split(':');
    Some((
        d.next()?.to_string(),
        d.next()?.to_string(),
        d.next()?.to_string(),
        h.next()?.to_string(),
        h.next()?.to_string(),
        h.next().unwrap_or("00").to_string(),
    ))
}

/// The status key a captured state derives to.
fn status_key(st: &EntryState) -> String {
    let e = TmEntry {
        trans: st.trans.clone(),
        status_sst: st.status_sst.clone(),
        status_eet: st.status_eet,
        ..Default::default()
    };
    crate::status::derive(&e).key().to_string()
}

/// The entries of one logged operation (R-A18-02): one per line whose
/// translation, status or review changed. The resumption of an autosave
/// leaves none (its own entries come back with it).
pub fn entries_of(ev: &LogEvent, lines: &[TmEntry]) -> Vec<Entry> {
    if ev.kind == OpKind::Resume {
        return Vec::new();
    }
    let t = iso(ev.secs);
    let mut out = Vec::new();
    for c in &ev.changes {
        let Some(e) = lines.get(c.idx) else {
            continue;
        };
        let (sb, sa) = (status_key(&c.before), status_key(&c.after));
        let review = c.before.review != c.after.review;
        if c.before.trans == c.after.trans && sb == sa && !review {
            continue;
        }
        out.push(Entry {
            t: t.clone(),
            k: key_of(e),
            s: e.source.clone(),
            b: c.before.trans.clone(),
            a: c.after.trans.clone(),
            sb,
            sa,
            p: c.after.prov.clone(),
            op: ev.kind.code().to_string(),
            arg: ev.label_arg.clone(),
            u: ev.undo,
            rb: review.then(|| c.before.review.clone()),
            ra: review.then(|| c.after.review.clone()),
        });
    }
    out
}

/// The `.ximhist` of a mod's dictionary.
pub fn file_for(mod_dict: &Path) -> PathBuf {
    mod_dict.with_extension("ximhist")
}

/// Read a `.ximhist` (missing file: empty; unreadable lines skipped).
pub fn read_file(path: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Append `new` to a `.ximhist` (D-46: append only); beyond `max` entries
/// the file is rewritten with the most recent ones. Returns the number of
/// entries the file holds.
pub fn append_file(path: &Path, new: &[Entry], max: usize) -> std::io::Result<usize> {
    let mut held = read_file(path).len();
    if !new.is_empty() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let mut buf = String::new();
        for e in new {
            buf.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
            buf.push('\n');
        }
        f.write_all(buf.as_bytes())?;
        held += new.len();
    }
    if held > max {
        let all = read_file(path);
        let keep = &all[all.len().saturating_sub(max)..];
        let mut buf = String::new();
        for e in keep {
            buf.push_str(&serde_json::to_string(e).map_err(std::io::Error::other)?);
            buf.push('\n');
        }
        std::fs::write(path, buf)?;
        held = keep.len();
    }
    Ok(held)
}

/// The history of a document: the entries read from its file (when it has
/// been read) followed by those of the session not yet written.
#[derive(Debug, Clone, Default)]
pub struct History {
    /// Every entry known, oldest first.
    pub entries: Vec<Entry>,
    /// The last `unsaved` entries are not in the file yet.
    pub unsaved: usize,
    /// The file the older entries were read from, once read.
    pub file: Option<PathBuf>,
}

impl History {
    /// A history carried by a pivot JSON or an autosave.
    pub fn from_carried(entries: Vec<Entry>) -> History {
        History {
            entries,
            unsaved: 0,
            file: None,
        }
    }

    /// Add the entries of logged operations, keeping at most `max`.
    pub fn record(&mut self, events: &[LogEvent], lines: &[TmEntry], max: usize) {
        for ev in events {
            let new = entries_of(ev, lines);
            self.unsaved += new.len();
            self.entries.extend(new);
        }
        self.trim(max);
    }

    /// Keep the `max` most recent entries.
    pub fn trim(&mut self, max: usize) {
        if self.entries.len() > max {
            let cut = self.entries.len() - max;
            self.entries.drain(..cut);
            self.unsaved = self.unsaved.min(self.entries.len());
        }
    }

    /// The entries not yet written.
    pub fn pending(&self) -> &[Entry] {
        &self.entries[self.entries.len() - self.unsaved..]
    }

    /// Read the file once (lazily: the history is read when it is looked
    /// at, D-43): its entries go before those of the session, without the
    /// ones already known (a pivot JSON or an autosave may carry them).
    pub fn load(&mut self, path: &Path) {
        if self.file.as_deref() == Some(path) {
            return;
        }
        let file = read_file(path);
        // What the session (or a carried history) has that the file has not.
        let extra: Vec<Entry> = std::mem::take(&mut self.entries)
            .into_iter()
            .filter(|e| !file.contains(e))
            .collect();
        self.unsaved = extra.len();
        self.entries = file;
        self.entries.extend(extra);
        self.file = Some(path.to_path_buf());
    }

    /// Merge entries carried by an autosave (B-11), as unsaved, without
    /// duplicates.
    pub fn merge_unsaved(&mut self, carried: Vec<Entry>) {
        for e in carried {
            if !self.entries.contains(&e) {
                self.entries.push(e);
                self.unsaved += 1;
            }
        }
    }

    /// Write the unsaved entries to `path`.
    pub fn save(&mut self, path: &Path, max: usize) -> std::io::Result<usize> {
        let held = append_file(path, self.pending(), max)?;
        self.unsaved = 0;
        self.file.get_or_insert_with(|| path.to_path_buf());
        self.trim(max);
        Ok(held)
    }

    /// The entries of a line, most recent first.
    pub fn of_key<'a>(&'a self, key: &'a Key) -> impl Iterator<Item = &'a Entry> + 'a {
        self.entries.iter().rev().filter(move |e| &e.k == key)
    }

    /// The change that gave a line its current translation, which
    /// « Revenir à la traduction précédente » undoes (R-A18-07): the most
    /// recent entry ending on `current`; its own retours arrière and the
    /// undo entries are passed over.
    pub fn previous_of(&self, key: &Key, current: &str) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| {
            &e.k == key
                && !e.u
                && e.op != OpKind::History.code()
                && e.changes_translation()
                && e.a == current
        })
    }
}

/// The CSV of entries (Exporter… of the window): `;`-separated, UTF-8 BOM
/// for Excel, with the given column titles and an operation label.
pub fn to_csv(
    entries: &[&Entry],
    titles: &[String],
    op_label: impl Fn(&Entry) -> String,
) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut out = String::from("\u{feff}");
    out.push_str(&titles.iter().map(|t| q(t)).collect::<Vec<_>>().join(";"));
    out.push('\n');
    for e in entries {
        let line = format!("{}:{} {} {:08X}", e.k.0, e.k.1, e.k.3, e.k.2);
        let row = [
            e.t.replace('T', " "),
            line,
            e.b.clone(),
            e.a.clone(),
            e.sb.clone(),
            e.sa.clone(),
            e.p.clone(),
            op_label(e),
        ];
        out.push_str(&row.iter().map(|c| q(c)).collect::<Vec<_>>().join(";"));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::{Change, UndoStack, record};

    fn line(rec: &str, fid: u32, src: &str) -> TmEntry {
        TmEntry {
            rec_sig: rec.into(),
            field_sig: "FULL".into(),
            formid: fid,
            edid: format!("E{fid}"),
            source: src.into(),
            ..Default::default()
        }
    }

    #[test]
    fn operations_leave_entries_undo_too() {
        let mut es = vec![line("WEAP", 1, "Sword"), line("ARMO", 2, "Hat")];
        let mut st = UndoStack::new(10);
        record(&mut st, &mut es, OpKind::Edit, None, |es| {
            es[0].trans = "Épée".into();
        });
        st.undo(&mut es).unwrap();
        let log = st.take_log();
        assert_eq!(log.len(), 2);
        let mut h = History::default();
        h.record(&log, &es, 100);
        assert_eq!(h.entries.len(), 2);
        let (e0, e1) = (&h.entries[0], &h.entries[1]);
        assert_eq!((e0.b.as_str(), e0.a.as_str(), e0.u), ("", "Épée", false));
        assert_eq!(e0.op, "edit");
        assert_eq!(e0.s, "Sword");
        assert_eq!((e1.b.as_str(), e1.a.as_str(), e1.u), ("Épée", "", true));
        assert_eq!(h.unsaved, 2);
        // After the undo, the line is empty again: nothing gave it.
        assert!(h.previous_of(&key_of(&es[0]), "").is_none());
        assert_eq!(h.previous_of(&key_of(&es[0]), "Épée").unwrap().b, "");
        // A change of the review alone carries rb / ra.
        let ev = LogEvent {
            secs: 0,
            kind: OpKind::Review,
            label_arg: None,
            undo: false,
            changes: vec![Change {
                idx: 1,
                before: EntryState::capture(&es[1]),
                after: EntryState {
                    review: "Chapeau".into(),
                    ..EntryState::capture(&es[1])
                },
            }],
        };
        let r = entries_of(&ev, &es);
        assert_eq!(r[0].ra.as_deref(), Some("Chapeau"));
        assert!(!r[0].changes_translation());
    }

    #[test]
    fn file_append_only_and_bounded() {
        let dir = std::env::temp_dir().join(format!("ximod-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("m.ximhist");
        let _ = std::fs::remove_file(&p);
        let mk = |i: usize| Entry {
            t: format!("2026-10-24T10:00:{:02}", i % 60),
            k: ("MISC".into(), "FULL".into(), i as u32, String::new(), 1),
            s: String::new(),
            b: String::new(),
            a: format!("t{i}"),
            sb: "to_translate".into(),
            sa: "pending".into(),
            p: "typed".into(),
            op: "edit".into(),
            arg: None,
            u: false,
            rb: None,
            ra: None,
        };
        let a: Vec<Entry> = (0..700).map(mk).collect();
        let b: Vec<Entry> = (700..1200).map(mk).collect();
        assert_eq!(append_file(&p, &a, 1000).unwrap(), 700);
        // T-A18-03: 1 200 changes, 1 000 kept, the most recent.
        assert_eq!(append_file(&p, &b, 1000).unwrap(), 1000);
        let back = read_file(&p);
        assert_eq!(back.len(), 1000);
        assert_eq!(back[0].a, "t200");
        assert_eq!(back[999].a, "t1199");
        // The JSON Lines shape of the specification.
        let first = std::fs::read_to_string(&p).unwrap();
        let l0 = first.lines().next().unwrap();
        assert!(
            l0.starts_with(r#"{"t":"2026-10-24T10:00:20","k":["MISC","FULL",200,"",1]"#),
            "{l0}"
        );
        // A history loads the file lazily, the session after it.
        let mut h = History::default();
        h.entries.push(mk(5000));
        h.unsaved = 1;
        h.load(&p);
        assert_eq!(h.entries.len(), 1001);
        assert_eq!(h.pending()[0].a, "t5000");
        h.save(&p, 1000).unwrap();
        assert_eq!(h.unsaved, 0);
        assert_eq!(read_file(&p).last().unwrap().a, "t5000");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn csv_export() {
        let e = Entry {
            t: "2026-10-24T10:00:00".into(),
            k: ("ARMO".into(), "FULL".into(), 2, "Hat".into(), 1),
            s: "Hat".into(),
            b: "Chapeau".into(),
            a: "Le \"chapeau\"".into(),
            sb: "pending".into(),
            sa: "validated".into(),
            p: "typed".into(),
            op: "edit".into(),
            arg: None,
            u: false,
            rb: None,
            ra: None,
        };
        let csv = to_csv(&[&e], &["a".into()], |_| "Saisie".into());
        assert!(csv.contains("\"Le \"\"chapeau\"\"\""));
        assert!(csv.contains("\"ARMO:FULL Hat 00000002\""));
    }
}
