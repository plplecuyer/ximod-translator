//! Travailler sur plusieurs fichiers (B-25): the propagation of a
//! translation to the lines of the same source of the other open files, and
//! the « Tous les mods » tab — a synthesis of every open file in one grid,
//! whose operations are carried out in each line's own file (decision D-57).

use std::path::PathBuf;

use super::{Doc, XimodApp};
use crate::undo::{Change, EntryState, LogEvent, Op};

/// The part of the synthesis that comes from one file.
#[derive(Debug, Clone)]
pub(crate) struct Slice {
    /// The file's document serial.
    pub serial: u64,
    /// First line of the file in the synthesis, number of lines.
    pub start: usize,
    pub len: usize,
    /// The file's undo revision the slice was copied at.
    pub revision: u64,
}

/// The synthesis: where its lines come from, and the files its operations
/// were carried out in (the Ctrl+Z of the tab).
#[derive(Debug, Clone, Default)]
pub(crate) struct AllMods {
    pub slices: Vec<Slice>,
    /// Serials of the files touched, in order (undo goes backwards).
    pub trail: Vec<u64>,
    /// Serials of the files undone (redo).
    pub redo: Vec<u64>,
}

impl AllMods {
    /// The slice of synthesis line `i`.
    pub fn slice_of(&self, i: usize) -> Option<&Slice> {
        let k = self.slices.partition_point(|s| s.start <= i);
        let s = self.slices.get(k.checked_sub(1)?)?;
        (i < s.start + s.len).then_some(s)
    }
}

/// The path of the synthesis document (never a file).
pub(crate) const ALL_PATH: &str = "\u{1}all-mods";

impl XimodApp {
    /// The index of the « Tous les mods » document, when present (always
    /// the last).
    pub(crate) fn all_index(&self) -> Option<usize> {
        self.docs.iter().rposition(|d| d.all.is_some())
    }

    /// Whether the active document is the synthesis.
    pub(crate) fn all_active(&self) -> bool {
        self.docs.get(self.active).is_some_and(|d| d.all.is_some())
    }

    /// The number of open files (the synthesis left out).
    pub(crate) fn file_count(&self) -> usize {
        self.docs.iter().filter(|d| d.all.is_none()).count()
    }

    /// The document index of a serial.
    pub(crate) fn doc_of_serial(&self, serial: u64) -> Option<usize> {
        self.docs.iter().position(|d| d.serial == serial)
    }

    /// The file a synthesis line comes from: (document index, its line).
    pub(crate) fn all_origin(&self, line: usize) -> Option<(usize, usize)> {
        let all = self.docs.get(self.all_index()?)?.all.as_ref()?;
        let s = all.slice_of(line)?;
        Some((self.doc_of_serial(s.serial)?, line - s.start))
    }

    /// Each frame, before the history (R-B25-04/05/07): the tab appears
    /// with the second file and goes with the last but one; the operations
    /// done in it are carried out in their files; the lines of a file
    /// changed elsewhere are copied again.
    pub(crate) fn all_mods_tick(&mut self) {
        let files = self.file_count();
        match self.all_index() {
            None if files >= 2 => self.all_mods_build(),
            Some(i) if files < 2 => {
                self.docs.remove(i);
                if self.active >= self.docs.len() {
                    self.active = self.docs.len().saturating_sub(1);
                }
                return;
            }
            Some(_) => {}
            None => return,
        }
        self.all_mods_flush();
        self.all_mods_refresh();
        // The toolbar shows the settings of the current line's file, read
        // only (R-B25-04).
        if let Some(ai) = self.all_index() {
            let origin = self
                .selection
                .cursor
                .filter(|_| self.active == ai)
                .and_then(|l| self.all_origin(l))
                .map(|(d, _)| d)
                .unwrap_or(0);
            if let Some(settings) = self.docs.get(origin).map(|d| d.settings.clone()) {
                self.docs[ai].settings = settings;
            }
        }
    }

    /// (Re)build the synthesis from the open files, in the order of the
    /// tabs (R-B25-04).
    fn all_mods_build(&mut self) {
        let mut entries = Vec::new();
        let mut slices = Vec::new();
        let mut first: Option<(crate::tm::TmMeta, crate::doc_settings::DocSettings)> = None;
        for d in self.docs.iter().filter(|d| d.all.is_none()) {
            if first.is_none() {
                first = Some((d.tm.meta.clone(), d.settings.clone()));
            }
            let name = d.file_name();
            slices.push(Slice {
                serial: d.serial,
                start: entries.len(),
                len: d.tm.entries.len(),
                revision: d.undo.revision,
            });
            entries.extend(d.tm.entries.iter().map(|e| {
                let mut e = e.clone();
                e.plugin = name.clone();
                e
            }));
        }
        let Some((meta, settings)) = first else {
            return;
        };
        let tm = crate::tm::TranslationMemory {
            meta,
            ..Default::default()
        };
        let mut doc = Doc::new(
            PathBuf::from(ALL_PATH),
            tm,
            None,
            settings,
            self.config.undo_depth,
        );
        // The lines as the files have them (Doc::new would mark them).
        doc.tm.entries = entries;
        doc.mark_saved();
        doc.all = Some(Box::new(AllMods {
            slices,
            ..Default::default()
        }));
        match self.all_index() {
            Some(i) => {
                // Keep the trail of the tab across a rebuild.
                let old = self.docs[i].all.take();
                if let (Some(old), Some(new)) = (old, doc.all.as_mut()) {
                    new.trail = old.trail;
                    new.redo = old.redo;
                }
                doc.undo.revision = self.docs[i].undo.revision + 1;
                self.docs[i] = doc;
            }
            None => self.docs.push(doc),
        }
        tracing::info!(
            "all mods: synthesis of {} files, {} lines",
            self.file_count(),
            self.docs.last().map(|d| d.tm.entries.len()).unwrap_or(0)
        );
    }

    /// Carry the operations done in the synthesis out in their files
    /// (R-B25-05): one operation per file touched, in that file's stack.
    pub(crate) fn all_mods_flush(&mut self) {
        let Some(ai) = self.all_index() else {
            return;
        };
        let log = self.docs[ai].undo.take_log();
        if log.is_empty() {
            return;
        }
        let Some(all) = self.docs[ai].all.as_ref() else {
            return;
        };
        let slices = all.slices.clone();
        let mut trail: Vec<u64> = Vec::new();
        for ev in log.iter().filter(|e| !e.undo) {
            for s in &slices {
                let mine: Vec<&Change> = ev
                    .changes
                    .iter()
                    .filter(|c| c.idx >= s.start && c.idx < s.start + s.len)
                    .collect();
                if mine.is_empty() {
                    continue;
                }
                let Some(di) = self.doc_of_serial(s.serial) else {
                    continue;
                };
                let doc = &mut self.docs[di];
                let mut changes = Vec::with_capacity(mine.len());
                for c in mine {
                    let i = c.idx - s.start;
                    let Some(e) = doc.tm.entries.get_mut(i) else {
                        continue;
                    };
                    let before = EntryState::capture(e);
                    c.after.restore(e);
                    if before != c.after {
                        changes.push(Change {
                            idx: i,
                            before,
                            after: c.after.clone(),
                        });
                    }
                }
                if changes.is_empty() {
                    continue;
                }
                doc.undo.push(Op {
                    kind: ev.kind,
                    label_arg: ev.label_arg.clone(),
                    changes,
                });
                trail.push(s.serial);
            }
        }
        // The synthesis keeps no stack of its own: its Ctrl+Z goes through
        // the files (R-B25-05).
        let revs: Vec<(u64, u64)> = self
            .docs
            .iter()
            .map(|d| (d.serial, d.undo.revision))
            .collect();
        let ai_doc = &mut self.docs[ai];
        ai_doc.undo.clear();
        ai_doc.mark_saved();
        if let Some(all) = ai_doc.all.as_mut() {
            if !trail.is_empty() {
                all.redo.clear();
            }
            all.trail.extend(trail.iter().copied());
            // What was just carried out is already in the synthesis.
            for s in all.slices.iter_mut() {
                if trail.contains(&s.serial)
                    && let Some((_, r)) = revs.iter().find(|(ser, _)| *ser == s.serial)
                {
                    s.revision = *r;
                }
            }
        }
    }

    /// Copy again the lines of the files changed outside the synthesis
    /// (R-B25-05): a file opened or closed rebuilds it, a file changed
    /// refreshes its slice only.
    fn all_mods_refresh(&mut self) {
        let Some(ai) = self.all_index() else {
            return;
        };
        let serials: Vec<(u64, usize)> = self
            .docs
            .iter()
            .filter(|d| d.all.is_none())
            .map(|d| (d.serial, d.tm.entries.len()))
            .collect();
        let same_shape = self.docs[ai].all.as_ref().is_some_and(|a| {
            a.slices.len() == serials.len()
                && a.slices
                    .iter()
                    .zip(&serials)
                    .all(|(s, (ser, n))| s.serial == *ser && s.len == *n)
        });
        if !same_shape {
            let was_active = self.active == ai;
            self.all_mods_build();
            if was_active {
                self.selection.clear();
            }
            return;
        }
        let stale: Vec<(usize, Slice)> = self.docs[ai]
            .all
            .as_ref()
            .map(|a| {
                a.slices
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        self.docs
                            .iter()
                            .find(|d| d.serial == s.serial)
                            .is_some_and(|d| d.undo.revision != s.revision)
                    })
                    .map(|(k, s)| (k, s.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if stale.is_empty() {
            return;
        }
        for (k, s) in stale {
            let Some(di) = self.doc_of_serial(s.serial) else {
                continue;
            };
            let (states, rev): (Vec<EntryState>, u64) = {
                let d = &self.docs[di];
                (
                    d.tm.entries.iter().map(EntryState::capture).collect(),
                    d.undo.revision,
                )
            };
            let ad = &mut self.docs[ai];
            for (j, st) in states.iter().enumerate() {
                if let Some(e) = ad.tm.entries.get_mut(s.start + j) {
                    st.restore(e);
                }
            }
            if let Some(a) = ad.all.as_mut() {
                a.slices[k].revision = rev;
            }
        }
        // A new revision: the view and the row heights are recomputed.
        let ad = &mut self.docs[ai];
        ad.undo.clear();
        ad.mark_saved();
    }

    /// Ctrl+Z in the synthesis (R-B25-05): the last operation it carried
    /// out, undone in its file. `false` when the active tab is a file.
    pub(crate) fn all_mods_undo(&mut self, redo: bool) -> bool {
        if !self.all_active() {
            return false;
        }
        self.all_mods_flush();
        let ai = self.active;
        let serial = self.docs[ai]
            .all
            .as_mut()
            .and_then(|a| if redo { a.redo.pop() } else { a.trail.pop() });
        let Some(serial) = serial else {
            self.status = self.i18n.t(if redo { "redo-none" } else { "undo-none" });
            return true;
        };
        let Some(di) = self.doc_of_serial(serial) else {
            return true;
        };
        let name = self.docs[di].file_name();
        let doc = &mut self.docs[di];
        let peek = if redo {
            doc.undo.peek_redo()
        } else {
            doc.undo.peek_undo()
        };
        let op = peek
            .map(|(k, arg, _)| self.i18n.t_arg(k.i18n_key(), "name", arg.unwrap_or("")))
            .unwrap_or_default();
        let done = if redo {
            doc.undo.redo(&mut doc.tm.entries)
        } else {
            doc.undo.undo(&mut doc.tm.entries)
        };
        if done.is_some() {
            if let Some(a) = self.docs[ai].all.as_mut() {
                if redo {
                    a.trail.push(serial);
                } else {
                    a.redo.push(serial);
                }
            }
            self.all_mods_refresh();
            let key = if redo {
                "all-mods-redo"
            } else {
                "all-mods-undo"
            };
            self.status = self.i18n.t_args(key, &[("file", &name), ("op", &op)]);
        }
        true
    }

    /// Whether the synthesis can undo / redo, and what (the toolbar).
    pub(crate) fn all_mods_peek(
        &self,
        redo: bool,
    ) -> Option<(crate::undo::OpKind, Option<&str>, usize)> {
        let ai = self.all_index()?;
        let a = self.docs[ai].all.as_ref()?;
        let serial = if redo { a.redo.last() } else { a.trail.last() }?;
        let d = self.docs.iter().find(|d| d.serial == *serial)?;
        if redo {
            d.undo.peek_redo()
        } else {
            d.undo.peek_undo()
        }
    }

    /// The propagation between open files (R-B25-01/02): what the logs of
    /// this frame wrote fills the lines of the same source of the other
    /// files of the same languages.
    pub(crate) fn propagate_across(&mut self, logs: &[Vec<LogEvent>]) {
        if !self.config.propagate_files || self.file_count() < 2 {
            return;
        }
        let mut done: Vec<(String, usize)> = Vec::new();
        for (oi, log) in logs.iter().enumerate() {
            if log.is_empty() || self.docs.get(oi).is_none_or(|d| d.all.is_some()) {
                continue;
            }
            let written = crate::propagate_files::written(log, &self.docs[oi].tm.entries);
            if written.is_empty() {
                continue;
            }
            let origin = self.docs[oi].file_name();
            done.extend(self.propagate_to_others(oi, &written, &origin));
        }
        self.report_propagation(&done);
    }

    /// Fill the other files from `written` (also « Appliquer aux fichiers
    /// ouverts », R-B25-03): (file, lines) per file touched.
    fn propagate_to_others(
        &mut self,
        origin_idx: usize,
        written: &[(String, String)],
        origin: &str,
    ) -> Vec<(String, usize)> {
        let (src, dst) = {
            let s = &self.docs[origin_idx].settings;
            (s.source.clone(), s.target.clone())
        };
        let mut out = Vec::new();
        if dst.trim().is_empty() {
            return out;
        }
        for ti in 0..self.docs.len() {
            if ti == origin_idx || self.docs[ti].all.is_some() {
                continue;
            }
            let d = &mut self.docs[ti];
            // The same languages only.
            if d.settings.target != dst || d.settings.source != src {
                continue;
            }
            if let Some(op) = crate::propagate_files::apply(&mut d.tm.entries, written, origin) {
                let n = op.changes.len();
                d.undo.push(op);
                out.push((d.file_name(), n));
            }
        }
        out
    }

    /// « Saisie propagée à … » in the status bar.
    fn report_propagation(&mut self, done: &[(String, usize)]) {
        if done.is_empty() {
            return;
        }
        let n: usize = done.iter().map(|(_, n)| n).sum();
        let mut files: Vec<&str> = done.iter().map(|(f, _)| f.as_str()).collect();
        files.dedup();
        let msg = self.i18n.t_args(
            "propagate-files-done",
            &[
                ("n", &crate::ui::export_dialog::fmt_count(&self.i18n, n)),
                ("files", &files.join(", ")),
            ],
        );
        tracing::info!("propagation across files: {n} lines");
        self.status = self
            .i18n
            .dash(&msg, &self.i18n.t("propagate-files-undo-note"));
    }

    /// Appliquer aux fichiers ouverts (menu of the line): the translations
    /// of the lines to the other open files.
    pub(crate) fn propagate_selection_to_files(&mut self, lines: &[usize]) {
        self.close_edit_session();
        self.all_mods_flush();
        // Per file of origin: its lines.
        let mut per: Vec<(usize, Vec<(String, String)>)> = Vec::new();
        for &l in lines {
            let (di, li) = if self.all_active() {
                match self.all_origin(l) {
                    Some(x) => x,
                    None => continue,
                }
            } else {
                (self.active, l)
            };
            let Some(e) = self.docs.get(di).and_then(|d| d.tm.entries.get(li)) else {
                continue;
            };
            let k = crate::propagate_files::key(&e.source).to_string();
            if e.trans.is_empty() || e.unused || k.is_empty() {
                continue;
            }
            let t = e.trans.clone();
            match per.iter_mut().find(|(d, _)| *d == di) {
                Some((_, v)) => v.push((k, t)),
                None => per.push((di, vec![(k, t)])),
            }
        }
        let mut done = Vec::new();
        for (di, written) in per {
            let origin = self.docs[di].file_name();
            done.extend(self.propagate_to_others(di, &written, &origin));
        }
        if done.is_empty() {
            self.status = self.i18n.t("propagate-files-none");
        } else {
            self.report_propagation(&done);
        }
    }

    /// Ctrl+Tab / Ctrl+Maj+Tab: the next / previous tab, the synthesis
    /// included (R-B25-04).
    pub(crate) fn cycle_tab(&mut self, back: bool) {
        let n = self.docs.len();
        if n < 2 {
            return;
        }
        self.close_edit_session();
        self.active = if back {
            (self.active + n - 1) % n
        } else {
            (self.active + 1) % n
        };
        self.selection.clear();
    }

    /// The name of a tab: the file's, or « Tous les mods (n) ».
    pub(crate) fn tab_name(&self, i: usize) -> String {
        match self.docs.get(i) {
            Some(d) if d.all.is_some() => {
                self.i18n
                    .t_arg("tab-all-mods", "n", &self.file_count().to_string())
            }
            Some(d) => d.file_name(),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_of_a_line() {
        let a = AllMods {
            slices: vec![
                Slice {
                    serial: 1,
                    start: 0,
                    len: 3,
                    revision: 0,
                },
                Slice {
                    serial: 2,
                    start: 3,
                    len: 0,
                    revision: 0,
                },
                Slice {
                    serial: 3,
                    start: 3,
                    len: 2,
                    revision: 0,
                },
            ],
            ..Default::default()
        };
        assert_eq!(a.slice_of(0).map(|s| s.serial), Some(1));
        assert_eq!(a.slice_of(2).map(|s| s.serial), Some(1));
        assert_eq!(a.slice_of(3).map(|s| s.serial), Some(3));
        assert_eq!(a.slice_of(4).map(|s| s.serial), Some(3));
        assert!(a.slice_of(5).is_none());
    }
}
