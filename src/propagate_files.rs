//! Propagation between open files (B-25): a translation written in one file
//! fills the lines of the same source of the other open files that have
//! none (or only a translation « par similarité »), En attente, provenance
//! « propagation (<fichier>) », as an undoable operation of each file.

use std::collections::HashMap;

use crate::status::{self, Status};
use crate::tm::TmEntry;
use crate::undo::{EntryState, LogEvent, Op, OpKind, op_from_before};

/// The key of a source: the text without its outer spaces.
pub fn key(source: &str) -> &str {
    source.trim()
}

/// The translations an operation log wrote (R-B25-01): (source key,
/// translation), the last one of a source winning. Undone operations and
/// the propagations themselves are left out (no ping-pong, R-B25-02), so
/// is the propagation dialog (A-12), which decides for the other open
/// files itself (C-04, R-C04-03).
pub fn written(events: &[LogEvent], entries: &[TmEntry]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for ev in events {
        if ev.undo || matches!(ev.kind, OpKind::PropagateFiles | OpKind::Propagate) {
            continue;
        }
        for c in &ev.changes {
            if c.after.trans.is_empty() || c.after.trans == c.before.trans {
                continue;
            }
            let Some(e) = entries.get(c.idx) else {
                continue;
            };
            if e.unused {
                continue;
            }
            let k = key(&e.source);
            if k.is_empty() {
                continue;
            }
            match at.get(k) {
                Some(&i) => out[i].1 = c.after.trans.clone(),
                None => {
                    at.insert(k.to_string(), out.len());
                    out.push((k.to_string(), c.after.trans.clone()));
                }
            }
        }
    }
    out
}

/// Whether a line of another file may receive a propagated translation:
/// shown in game, without translation or « par similarité ».
pub fn may_receive(e: &TmEntry) -> bool {
    !e.unused
        && matches!(
            status::derive(e),
            Status::ToTranslate | Status::BySimilarity
        )
}

/// Fill the lines of `entries` whose source is one of `written` (R-B25-01):
/// the operation to push in that file's undo stack, `None` when nothing
/// changed.
pub fn apply(entries: &mut [TmEntry], written: &[(String, String)], origin: &str) -> Option<Op> {
    if written.is_empty() {
        return None;
    }
    let wanted: HashMap<&str, &str> = written
        .iter()
        .map(|(k, t)| (k.as_str(), t.as_str()))
        .collect();
    let mut before: Vec<(usize, EntryState)> = Vec::new();
    let mut texts: Vec<(usize, String)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if !may_receive(e) {
            continue;
        }
        if let Some(t) = wanted.get(key(&e.source))
            && e.trans != *t
        {
            before.push((i, EntryState::capture(e)));
            texts.push((i, (*t).to_string()));
        }
    }
    if before.is_empty() {
        return None;
    }
    let prov = crate::provenance::propagate_file(origin);
    for (i, t) in texts {
        let e = &mut entries[i];
        e.trans = t;
        e.prov = prov.clone();
        e.set_translated_flags(true);
        status::set(e, Status::Pending);
    }
    let mut op = op_from_before(OpKind::PropagateFiles, &before, entries);
    op.label_arg = Some(origin.to_string());
    Some(op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::Change;

    fn e(src: &str, tr: &str, st: Status) -> TmEntry {
        let mut e = TmEntry {
            source: src.into(),
            trans: tr.into(),
            ..Default::default()
        };
        e.set_translated_flags(!tr.is_empty());
        if !tr.is_empty() {
            status::set(&mut e, st);
        }
        e
    }

    fn typed(idx: usize, before: &str, after: &str) -> LogEvent {
        let st = |t: &str| EntryState {
            trans: t.into(),
            ..Default::default()
        };
        LogEvent {
            secs: 0,
            kind: OpKind::Edit,
            label_arg: None,
            undo: false,
            changes: vec![Change {
                idx,
                before: st(before),
                after: st(after),
            }],
        }
    }

    #[test]
    fn written_then_applied() {
        let origin = vec![
            e("Iron Sword of Frost ", "", Status::ToTranslate),
            e("Steel Dagger", "", Status::ToTranslate),
        ];
        let mut ev = vec![
            typed(0, "", "Épée"),
            typed(0, "Épée", "Épée en fer de givre"),
        ];
        // An undone operation and a propagation are not propagated.
        let mut undone = typed(1, "", "Dague");
        undone.undo = true;
        ev.push(undone);
        let mut prop = typed(1, "", "Dague en acier");
        prop.kind = OpKind::PropagateFiles;
        ev.push(prop);
        // Nor the propagation dialog (C-04).
        let mut dlg = typed(1, "", "Dague d'acier");
        dlg.kind = OpKind::Propagate;
        ev.push(dlg);
        let w = written(&ev, &origin);
        assert_eq!(
            w,
            vec![(
                "Iron Sword of Frost".to_string(),
                "Épée en fer de givre".to_string()
            )]
        );
        let mut patch = vec![
            e(" Iron Sword of Frost", "", Status::ToTranslate),
            e("Iron Sword of Frost", "Épée de givre", Status::Validated),
            e("Iron Sword of Frost", "Épée ?", Status::BySimilarity),
            e("Iron Sword of Frost", "", Status::ToTranslate),
        ];
        patch[3].unused = true;
        let op = apply(&mut patch, &w, "MonMod.esp").unwrap();
        assert_eq!(op.kind, OpKind::PropagateFiles);
        assert_eq!(op.label_arg.as_deref(), Some("MonMod.esp"));
        assert_eq!(op.changes.len(), 2);
        assert_eq!(patch[0].trans, "Épée en fer de givre");
        assert_eq!(status::derive(&patch[0]), Status::Pending);
        assert_eq!(patch[0].prov, "propagate:MonMod.esp");
        assert_eq!(patch[1].trans, "Épée de givre");
        assert_eq!(patch[2].trans, "Épée en fer de givre");
        assert!(patch[3].trans.is_empty());
        // Nothing more to do the second time.
        assert!(apply(&mut patch, &w, "MonMod.esp").is_none());
    }
}
