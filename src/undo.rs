//! Undo / redo of the translatable content of a document (A-01).
//!
//! Pure library, no egui. The unit is the *operation* as the user triggers it
//! (R-A01-01): a cell edit from focus gain to focus loss, a status change, a
//! base application, a re-translation, a propagation… Each operation stores,
//! for every line it touched, the old and the new [`EntryState`] (translation,
//! status flags), so undo and redo are exact and O(lines touched).
//!
//! Mass operations are recorded by snapshot and difference ([`record`]):
//! simple, and correct even when the operation touches lines indirectly.
//! Small frequent operations (one cell, one status) push their [`Op`]
//! directly.

use std::collections::VecDeque;

use crate::tm::TmEntry;

/// Default number of operations kept per document (`[Editing] UndoDepth`).
pub const DEFAULT_DEPTH: usize = 200;
/// Bounds of `UndoDepth`.
pub const DEPTH_RANGE: std::ops::RangeInclusive<usize> = 10..=1000;
/// Memory bound of one stack (R-A01-03): beyond it the oldest operations go.
pub const CAP_BYTES: usize = 64 * 1024 * 1024;

/// What an operation may change on a line: the translation, the status
/// flags, the provenance (B-09, R-B09-03) and the review (A-17).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryState {
    pub trans: String,
    pub status_sst: Vec<String>,
    pub status_eet: i16,
    pub prov: String,
    pub review: String,
}

impl EntryState {
    pub fn capture(e: &TmEntry) -> Self {
        EntryState {
            trans: e.trans.clone(),
            status_sst: e.status_sst.clone(),
            status_eet: e.status_eet,
            prov: e.prov.clone(),
            review: e.review.clone(),
        }
    }

    pub fn restore(&self, e: &mut TmEntry) {
        e.trans = self.trans.clone();
        e.status_sst = self.status_sst.clone();
        e.status_eet = self.status_eet;
        e.prov = self.prov.clone();
        e.review = self.review.clone();
    }

    /// Approximate memory footprint, for the stack's byte bound.
    pub fn bytes(&self) -> usize {
        32 + self.trans.len()
            + self.prov.len()
            + self.review.len()
            + self.status_sst.iter().map(|s| s.len() + 24).sum::<usize>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub idx: usize,
    pub before: EntryState,
    pub after: EntryState,
}

/// The kinds of operations (their `op-*` message keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// Typing in a cell (one operation per cell, focus gain to focus loss).
    Edit,
    /// A status change on one line or a selection.
    Status,
    /// A base or dictionary applied (at load, manually, or after a setting change).
    ApplyBase,
    /// « Re-traduire les lignes auto ».
    Retranslate,
    /// Propagation to the hidden duplicates (unique-lines view).
    Propagate,
    /// Resumption of an autosaved state (B-11).
    Resume,
    /// « Rétablir les alias / les nombres » (A-07): `label_arg` names which.
    Fix,
    /// Rechercher et remplacer (A-11): one replacement or a rules file.
    Replace,
    /// Traduction en ligne (D-01): `label_arg` names the service.
    Online,
    /// The review of lines (A-17): typed, copied from the translation,
    /// cleared or loaded from a reviewed copy.
    Review,
    /// « Traduction = Relecture » (A-17).
    ReviewAccept,
    /// « Retour arrière (historique) » (A-18).
    History,
    /// A translation received from another open file (B-25): `label_arg`
    /// names that file.
    PropagateFiles,
    /// « Traduire les chaînes VMAD de ce fichier » checked / unchecked (C-08).
    VmadOn,
    VmadOff,
    /// A replacement matrix applied (B-17): `label_arg` names it.
    Matrix,
    /// An exclusion rule turned on / off for the document (B-18):
    /// `label_arg` names it.
    RuleOn,
    RuleOff,
    /// A rule learned from a line set Ignoré by hand (B-18).
    Generalize,
    /// The exclusion rules applied again (B-18).
    RulesApply,
    /// « Appliquer les assistants » (B-15): `label_arg` says how many lines.
    Assist,
    /// Transformer ▸ (A-13): `label_arg` names the command.
    Transform,
}

impl OpKind {
    /// i18n key of the operation's name.
    pub fn i18n_key(self) -> &'static str {
        match self {
            OpKind::Edit => "op-edit",
            OpKind::Status => "op-status",
            OpKind::ApplyBase => "op-apply-base",
            OpKind::Retranslate => "op-retranslate",
            OpKind::Propagate => "op-propagate",
            OpKind::Resume => "op-resume",
            OpKind::Fix => "op-fix",
            OpKind::Replace => "op-replace",
            OpKind::Online => "op-online",
            OpKind::Review => "op-review",
            OpKind::ReviewAccept => "op-review-accept",
            OpKind::History => "op-history",
            OpKind::PropagateFiles => "op-propagate-files",
            OpKind::VmadOn => "op-vmad-enable",
            OpKind::VmadOff => "op-vmad-disable",
            OpKind::Matrix => "op-matrix",
            OpKind::RuleOn => "op-rule-on",
            OpKind::RuleOff => "op-rule-off",
            OpKind::Generalize => "op-generalize",
            OpKind::RulesApply => "op-rules-apply",
            OpKind::Assist => "op-assist",
            OpKind::Transform => "op-transform",
        }
    }

    /// Every kind, for [`OpKind::from_code`].
    pub const ALL: [OpKind; 22] = [
        OpKind::Edit,
        OpKind::Status,
        OpKind::ApplyBase,
        OpKind::Retranslate,
        OpKind::Propagate,
        OpKind::Resume,
        OpKind::Fix,
        OpKind::Replace,
        OpKind::Online,
        OpKind::Review,
        OpKind::ReviewAccept,
        OpKind::History,
        OpKind::PropagateFiles,
        OpKind::VmadOn,
        OpKind::VmadOff,
        OpKind::Matrix,
        OpKind::RuleOn,
        OpKind::RuleOff,
        OpKind::Generalize,
        OpKind::RulesApply,
        OpKind::Assist,
        OpKind::Transform,
    ];

    /// The code of the operation in the history (A-18): stable from one
    /// language to the other.
    pub fn code(self) -> &'static str {
        &self.i18n_key()[3..]
    }

    pub fn from_code(code: &str) -> Option<OpKind> {
        OpKind::ALL.into_iter().find(|k| k.code() == code)
    }

    /// The provenance a line gets when the operation changes its
    /// translation without naming one (R-B09-02); `None` keeps it.
    pub fn default_prov(self) -> Option<&'static str> {
        match self {
            OpKind::Edit => Some(crate::provenance::TYPED),
            OpKind::Propagate => Some(crate::provenance::PROPAGATE),
            OpKind::Fix | OpKind::Replace => Some(crate::provenance::REPLACE),
            OpKind::ReviewAccept => Some(crate::provenance::REVIEW),
            OpKind::History => Some(crate::provenance::HISTORY),
            // A transformation is the translator's own hand (R-A13-08).
            OpKind::Transform => Some(crate::provenance::TYPED),
            OpKind::Status
            | OpKind::PropagateFiles
            | OpKind::ApplyBase
            | OpKind::Retranslate
            | OpKind::Resume
            | OpKind::Online
            | OpKind::Review
            | OpKind::VmadOn
            | OpKind::VmadOff
            | OpKind::Matrix
            | OpKind::RuleOn
            | OpKind::RuleOff
            | OpKind::Generalize
            | OpKind::RulesApply
            | OpKind::Assist => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub kind: OpKind,
    /// Argument of the operation's name (the base's name for `ApplyBase`).
    pub label_arg: Option<String>,
    pub changes: Vec<Change>,
}

impl Op {
    pub fn bytes(&self) -> usize {
        self.changes
            .iter()
            .map(|c| 16 + c.before.bytes() + c.after.bytes())
            .sum()
    }

    /// Lines touched, in index order, without duplicates.
    pub fn touched(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.changes.iter().map(|c| c.idx).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Outcome of an undo or a redo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoResult {
    pub kind: OpKind,
    pub label_arg: Option<String>,
    /// Lines touched, sorted.
    pub touched: Vec<usize>,
}

impl UndoResult {
    pub fn first_idx(&self) -> Option<usize> {
        self.touched.first().copied()
    }

    pub fn count(&self) -> usize {
        self.touched.len()
    }
}

/// An operation as the history sees it (A-18): pushed, undone (its changes
/// reversed, `undo`) or redone, with the moment it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEvent {
    pub secs: i64,
    pub kind: OpKind,
    pub label_arg: Option<String>,
    pub undo: bool,
    pub changes: Vec<Change>,
}

impl LogEvent {
    fn of(op: &Op, undo: bool) -> LogEvent {
        let changes = if undo {
            op.changes
                .iter()
                .rev()
                .map(|c| Change {
                    idx: c.idx,
                    before: c.after.clone(),
                    after: c.before.clone(),
                })
                .collect()
        } else {
            op.changes.clone()
        };
        LogEvent {
            secs: crate::backup::now_secs(),
            kind: op.kind,
            label_arg: op.label_arg.clone(),
            undo,
            changes,
        }
    }
}

/// One document's undo and redo stacks (R-A01-02).
#[derive(Debug)]
pub struct UndoStack {
    undo: VecDeque<Op>,
    redo: Vec<Op>,
    cap_ops: usize,
    cap_bytes: usize,
    bytes: usize,
    /// Bumped at every change of the document's content (operations, undo,
    /// redo, live typing): lets callers memoise anything derived from it.
    pub revision: u64,
    /// What happened since the last [`UndoStack::take_log`] (A-18).
    log: Vec<LogEvent>,
    /// Bumped at each operation pushed (not at undo, redo or typing): the
    /// derived lines follow their parent after one (B-21).
    pub pushes: u64,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(DEFAULT_DEPTH)
    }
}

impl UndoStack {
    pub fn new(cap_ops: usize) -> Self {
        UndoStack {
            undo: VecDeque::new(),
            redo: Vec::new(),
            cap_ops: cap_ops.max(1),
            cap_bytes: CAP_BYTES,
            bytes: 0,
            revision: 0,
            log: Vec::new(),
            pushes: 0,
        }
    }

    /// Change the number of operations kept (R-A01-03); older ones go now.
    pub fn set_capacity(&mut self, cap_ops: usize) {
        self.cap_ops = cap_ops.max(1);
        self.evict();
    }

    pub fn capacity(&self) -> usize {
        self.cap_ops
    }

    /// Note a content change that is not (yet) an operation, e.g. live typing.
    pub fn touch(&mut self) {
        self.revision += 1;
    }

    /// Record an operation: clears the redo stack and evicts the oldest
    /// operations beyond the bounds. An operation with no change is ignored.
    pub fn push(&mut self, op: Op) {
        if op.changes.is_empty() {
            return;
        }
        self.redo.clear();
        self.bytes += op.bytes();
        self.log.push(LogEvent::of(&op, false));
        self.undo.push_back(op);
        self.evict();
        self.revision += 1;
        self.pushes += 1;
    }

    /// The last operation pushed (and not undone).
    pub fn last_op(&self) -> Option<&Op> {
        self.undo.back()
    }

    /// Add changes to the last operation (the derived lines that follow
    /// their parent, B-21): one Ctrl+Z undoes both.
    pub fn extend_last(&mut self, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        let Some(op) = self.undo.back_mut() else {
            return;
        };
        let extra = Op {
            kind: op.kind,
            label_arg: op.label_arg.clone(),
            changes,
        };
        self.bytes += extra.bytes();
        self.log.push(LogEvent::of(&extra, false));
        op.changes.extend(extra.changes);
        self.revision += 1;
    }

    fn evict(&mut self) {
        while self.undo.len() > self.cap_ops || (self.bytes > self.cap_bytes && self.undo.len() > 1)
        {
            if let Some(old) = self.undo.pop_front() {
                self.bytes = self.bytes.saturating_sub(old.bytes());
            }
        }
    }

    pub fn undo(&mut self, entries: &mut [TmEntry]) -> Option<UndoResult> {
        let op = self.undo.pop_back()?;
        self.bytes = self.bytes.saturating_sub(op.bytes());
        // Restore in reverse so a line changed twice in one operation ends on
        // its first `before`.
        for c in op.changes.iter().rev() {
            if let Some(e) = entries.get_mut(c.idx) {
                c.before.restore(e);
            }
        }
        let res = UndoResult {
            kind: op.kind,
            label_arg: op.label_arg.clone(),
            touched: op.touched(),
        };
        self.log.push(LogEvent::of(&op, true));
        self.redo.push(op);
        self.revision += 1;
        Some(res)
    }

    pub fn redo(&mut self, entries: &mut [TmEntry]) -> Option<UndoResult> {
        let op = self.redo.pop()?;
        for c in &op.changes {
            if let Some(e) = entries.get_mut(c.idx) {
                c.after.restore(e);
            }
        }
        let res = UndoResult {
            kind: op.kind,
            label_arg: op.label_arg.clone(),
            touched: op.touched(),
        };
        self.bytes += op.bytes();
        self.log.push(LogEvent::of(&op, false));
        self.undo.push_back(op);
        self.revision += 1;
        Some(res)
    }

    /// The next operation undo would revert: its kind, argument and line count.
    pub fn peek_undo(&self) -> Option<(OpKind, Option<&str>, usize)> {
        self.undo
            .back()
            .map(|op| (op.kind, op.label_arg.as_deref(), op.touched().len()))
    }

    pub fn peek_redo(&self) -> Option<(OpKind, Option<&str>, usize)> {
        self.redo
            .last()
            .map(|op| (op.kind, op.label_arg.as_deref(), op.touched().len()))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    /// The operations pushed, undone or redone since the last call (A-18).
    pub fn take_log(&mut self) -> Vec<LogEvent> {
        std::mem::take(&mut self.log)
    }

    /// Forget everything (file reloaded, R-A01-06).
    pub fn clear(&mut self) {
        self.log.clear();
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
        self.revision += 1;
    }
}

/// Run a mass operation on `entries` and record it by snapshot and
/// difference. Nothing is pushed when nothing changed.
pub fn record<R>(
    stack: &mut UndoStack,
    entries: &mut [TmEntry],
    kind: OpKind,
    label_arg: Option<String>,
    f: impl FnOnce(&mut [TmEntry]) -> R,
) -> R {
    let before: Vec<EntryState> = entries.iter().map(EntryState::capture).collect();
    let r = f(entries);
    let default = kind.default_prov();
    let changes: Vec<Change> = entries
        .iter_mut()
        .enumerate()
        .filter_map(|(i, e)| {
            if e.trans != before[i].trans || e.prov != before[i].prov {
                crate::provenance::settle(e, &before[i].trans, &before[i].prov, default);
            }
            let after = EntryState::capture(e);
            (after != before[i]).then(|| Change {
                idx: i,
                before: before[i].clone(),
                after,
            })
        })
        .collect();
    stack.push(Op {
        kind,
        label_arg,
        changes,
    });
    r
}

/// Build the operation of a small edit from captured `before` states (one
/// cell, its propagated duplicates, one status): only the lines whose state
/// differs are kept.
/// The provenance of the changed lines is settled on the way (R-B09-02).
pub fn op_from_before(kind: OpKind, before: &[(usize, EntryState)], entries: &mut [TmEntry]) -> Op {
    let default = kind.default_prov();
    let changes = before
        .iter()
        .filter_map(|(idx, b)| {
            let e = entries.get_mut(*idx)?;
            crate::provenance::settle(e, &b.trans, &b.prov, default);
            let after = EntryState::capture(e);
            (after != *b).then(|| Change {
                idx: *idx,
                before: b.clone(),
                after,
            })
        })
        .collect();
    Op {
        kind,
        label_arg: None,
        changes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{self, Status};

    fn entries(n: usize) -> Vec<TmEntry> {
        (0..n)
            .map(|i| TmEntry {
                source: format!("s{i}"),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn record_undo_redo_round_trip() {
        let mut es = entries(5);
        let mut st = UndoStack::new(10);
        let n = record(
            &mut st,
            &mut es,
            OpKind::ApplyBase,
            Some("base".into()),
            |es| {
                for e in es.iter_mut().take(3) {
                    e.trans = format!("T{}", e.source);
                    status::set(e, Status::BySimilarity);
                }
                3
            },
        );
        assert_eq!(n, 3);
        assert_eq!(st.len(), 1);
        assert_eq!(st.peek_undo(), Some((OpKind::ApplyBase, Some("base"), 3)));
        let r = st.undo(&mut es).unwrap();
        assert_eq!(r.touched, vec![0, 1, 2]);
        assert_eq!(r.first_idx(), Some(0));
        assert!(
            es.iter()
                .all(|e| e.trans.is_empty() && e.status_sst.is_empty())
        );
        assert!(st.can_redo() && !st.can_undo());
        let r = st.redo(&mut es).unwrap();
        assert_eq!(r.count(), 3);
        assert_eq!(es[2].trans, "Ts2");
        assert_eq!(status::derive(&es[2]), Status::BySimilarity);
        assert!(es[3].trans.is_empty());
    }

    #[test]
    fn nothing_changed_pushes_nothing_and_new_op_clears_redo() {
        let mut es = entries(2);
        let mut st = UndoStack::new(10);
        record(&mut st, &mut es, OpKind::Retranslate, None, |_| ());
        assert!(st.is_empty());
        let rev = st.revision;
        record(&mut st, &mut es, OpKind::Edit, None, |es| {
            es[0].trans = "a".into()
        });
        assert!(st.revision > rev);
        st.undo(&mut es);
        assert!(st.can_redo());
        record(&mut st, &mut es, OpKind::Edit, None, |es| {
            es[1].trans = "b".into()
        });
        assert!(!st.can_redo(), "a new operation empties the redo stack");
        assert_eq!(es[0].trans, "");
        assert_eq!(es[1].trans, "b");
    }

    #[test]
    fn capacity_and_memory_bounds() {
        let mut es = entries(1);
        let mut st = UndoStack::new(3);
        for i in 0..5 {
            record(&mut st, &mut es, OpKind::Edit, None, |es| {
                es[0].trans = format!("v{i}")
            });
        }
        assert_eq!(st.len(), 3, "only the last three operations are kept");
        assert!(st.undo(&mut es).is_some());
        assert!(st.undo(&mut es).is_some());
        assert!(st.undo(&mut es).is_some());
        assert!(st.undo(&mut es).is_none());
        assert_eq!(es[0].trans, "v1", "the oldest kept operation restores v1");
        // Shrinking the capacity evicts now.
        let mut st = UndoStack::new(10);
        for i in 0..8 {
            record(&mut st, &mut es, OpKind::Edit, None, |es| {
                es[0].trans = format!("w{i}")
            });
        }
        st.set_capacity(2);
        assert_eq!(st.len(), 2);
        // Byte bound: a huge operation evicts older ones but is kept itself.
        let mut st = UndoStack::new(100);
        st.cap_bytes = 1000;
        record(&mut st, &mut es, OpKind::Edit, None, |es| {
            es[0].trans = "small".into()
        });
        record(&mut st, &mut es, OpKind::Edit, None, |es| {
            es[0].trans = "x".repeat(2000)
        });
        assert_eq!(st.len(), 1);
        assert_eq!(st.peek_undo().map(|(k, _, _)| k), Some(OpKind::Edit));
    }

    #[test]
    fn small_ops_from_before_states() {
        let mut es = entries(3);
        let before = vec![
            (1, EntryState::capture(&es[1])),
            (2, EntryState::capture(&es[2])),
        ];
        es[1].trans = "x".into();
        let op = op_from_before(OpKind::Edit, &before, &mut es);
        assert_eq!(op.changes.len(), 1);
        assert_eq!(op.changes[0].idx, 1);
        let mut st = UndoStack::default();
        st.push(op);
        st.undo(&mut es);
        assert_eq!(es[1].trans, "");
        assert_eq!(st.capacity(), DEFAULT_DEPTH);
        st.clear();
        assert!(!st.can_redo() && !st.can_undo());
    }
}
