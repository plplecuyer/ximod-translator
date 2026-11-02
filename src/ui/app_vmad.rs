//! The scripts' strings attached to the records (C-08): the switch
//! « Traduire les chaînes VMAD de ce fichier » (Outils ▸ Scripts), its
//! memory in the mod's dictionary and the pivot, the locks laid around the
//! filling by the dictionaries, the counters of the status bar.

use super::{Doc, XimodApp};
use crate::dict_stack::DictStack;
use crate::script_rules::Rules;
use crate::tm::TmEntry;
use crate::undo::{OpKind, record};
use crate::vmad_lock;

/// The rules, loaded once (the user's copy, else the shipped file).
/// The rules, read at each use: a rule removed from the user's copy no
/// longer applies at the next opening (T-C06-05).
fn rules() -> Rules {
    Rules::installed()
}

impl XimodApp {
    /// The switch of a document as its files remember it: the header of
    /// the mod's dictionary when there is one, else the pivot's meta (a
    /// plugin opened for the first time: off, D-63).
    pub(crate) fn vmad_remembered(&self, doc: &Doc) -> bool {
        if !doc.settings.target.trim().is_empty() {
            let out =
                DictStack::mod_dict_path(&doc.path, &doc.settings, &self.config.mod_base_dir());
            if out.is_file() {
                return crate::basedb::peek_extra(&out).vmad.unwrap_or(false);
            }
        }
        doc.tm.meta.vmad
    }

    /// Lift the locks before the dictionaries fill the lines.
    pub(crate) fn vmad_release(entries: &mut [TmEntry]) {
        vmad_lock::release(entries);
    }

    /// Lay the file lock after the dictionaries filled the lines and the
    /// exclusion rules locked theirs (B-18), when the switch is off.
    pub(crate) fn vmad_lay(entries: &mut [TmEntry], on: bool) {
        vmad_lock::lay_file(entries, on);
    }

    /// Outils ▸ Scripts ▸ Traduire les chaînes VMAD de ce fichier: one
    /// undoable operation (R-C08-03).
    pub(crate) fn vmad_switch(&mut self, on: bool) {
        self.close_edit_session();
        let active = self.active;
        let Some(doc) = self.docs.get_mut(active) else {
            return;
        };
        if doc.all.is_some() {
            return;
        }
        let kind = if on { OpKind::VmadOn } else { OpKind::VmadOff };
        let rules = rules();
        let n = record(&mut doc.undo, &mut doc.tm.entries, kind, None, |es| {
            vmad_lock::switch(es, on, &rules)
        });
        doc.tm.meta.vmad = on;
        // The switch is a change of the work even when no line moves.
        doc.undo.touch();
        let _ = n;
        let count = vmad_lock::count(&doc.tm.entries);
        self.status = self.i18n.dash(
            &self.i18n.t(kind.i18n_key()),
            &self
                .i18n
                .t_arg("status-vmad-count", "n", &count.to_string()),
        );
    }

    /// After an undo or a redo, the switch follows the lines.
    pub(crate) fn vmad_sync(&mut self) {
        if let Some(doc) = self.docs.get_mut(self.active)
            && let Some(on) = vmad_lock::state(&doc.tm.entries)
        {
            doc.tm.meta.vmad = on;
        }
    }

    /// The switch of the active document: `None` when it has no VMAD line.
    pub(crate) fn vmad_active_state(&self) -> Option<bool> {
        let doc = self.active_doc()?;
        if doc.all.is_some() {
            return None;
        }
        vmad_lock::state(&doc.tm.entries)
    }

    /// The counters of the status bar: VMAD lines, blocks not read.
    pub(crate) fn vmad_counters(&self) -> Option<(String, Option<String>)> {
        let doc = self.active_doc()?;
        let n = vmad_lock::count(&doc.tm.entries);
        let unread = doc.tm.meta.vmad_unread.len();
        if n == 0 && unread == 0 {
            return None;
        }
        let count = self.i18n.t_arg("status-vmad-count", "n", &n.to_string());
        let unread = (unread > 0).then(|| {
            self.i18n
                .t_arg("status-vmad-unread", "n", &unread.to_string())
        });
        Some((count, unread))
    }

    /// The blocks not read, in the log (R-C08-01), once at the opening.
    pub(crate) fn vmad_log_unread(&self, doc: &Doc) {
        for (who, why) in &doc.tm.meta.vmad_unread {
            tracing::warn!(
                "{}",
                self.i18n
                    .t_args("log-vmad-unread", &[("edid", who), ("reason", why)])
            );
        }
    }
}
