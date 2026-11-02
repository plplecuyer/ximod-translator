//! C-04 on real plugins: a line whose source is twice in the mod and once
//! in an open patch — the Communes tab lists the three, the patch's with
//! its file; the propagation dialog extended to the patch writes one
//! operation per file, each undone in its file (T-C04-01, T-C04-03,
//! T-C04-05).

use std::path::Path;

use ximod_translator::common::{self, File, Hit, SourceIndex};
use ximod_translator::plugin;
use ximod_translator::propagate;
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;
use ximod_translator::undo::{EntryState, OpKind, UndoStack, op_from_before};

fn entries() -> Vec<TmEntry> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skyrimle_riverwood.esp");
    plugin::read_file(&p).unwrap().entries
}

#[test]
fn common_lines_across_open_files() {
    let mut mod_ = entries();
    // A source shown once in the mod: give it a twin, keep one copy in
    // the patch.
    let ix0 = SourceIndex::build(&mod_);
    let i = (0..mod_.len())
        .find(|&k| {
            !mod_[k].unused
                && !mod_[k].source.trim().is_empty()
                && ix0.lines(common::key(&mod_[k].source)).len() == 1
        })
        .unwrap();
    let j = (0..mod_.len())
        .find(|&k| k != i && !mod_[k].unused && mod_[k].source != mod_[i].source)
        .unwrap();
    mod_[j].source = format!("{} ", mod_[i].source);
    let src = mod_[i].source.clone();
    let mut patch: Vec<TmEntry> = entries()
        .into_iter()
        .filter(|e| e.source == src)
        .take(1)
        .collect();
    patch[0].plugin = "MonMod - Patch A.esp".into();
    let (im, ip) = (SourceIndex::build(&mod_), SourceIndex::build(&patch));
    let files = [
        File {
            entries: &mod_,
            index: &im,
        },
        File {
            entries: &patch,
            index: &ip,
        },
    ];

    // T-C04-01: the twin here, the patch's line after.
    let f = common::gather(&files, 0, i, false);
    assert_eq!((f.here, f.others), (1, 1));
    assert_eq!(
        f.hits,
        vec![Hit { file: 0, idx: j }, Hit { file: 1, idx: 0 }]
    );

    // T-C04-05: a source alone.
    let alone = (0..mod_.len())
        .find(|&k| {
            !mod_[k].unused
                && !mod_[k].source.trim().is_empty()
                && im.lines(common::key(&mod_[k].source)).len() == 1
                && ip.lines(common::key(&mod_[k].source)).is_empty()
        })
        .unwrap();
    assert_eq!(common::gather(&files, 0, alone, false).total(), 0);

    // T-C04-03: Appliquer à toutes — the line translated, the dialog
    // lists the twin (by its key) and the patch's line, one operation per
    // file, each undone in its file.
    let before_j = mod_[j].trans.clone();
    mod_[i].trans = "Traduction".into();
    mod_[i].set_translated_flags(true);
    status::set(&mut mod_[i], Status::Validated);
    let mut groups = propagate::groups_open(&mod_, &[i]);
    propagate::extend(&mut groups, 1, &patch, &ip);
    propagate::drop_empty(&mut groups);
    assert_eq!(groups.len(), 1);
    let files_of: Vec<Option<usize>> = groups[0].targets.iter().map(|t| t.file).collect();
    assert_eq!(files_of, vec![None, Some(1)], "outer spaces left out");
    let (mut s_mod, mut s_patch) = (UndoStack::new(50), UndoStack::new(50));
    let before: Vec<(usize, EntryState)> = vec![(j, EntryState::capture(&mod_[j]))];
    assert_eq!(propagate::apply(&mut mod_, &groups), vec![j]);
    s_mod.push(op_from_before(OpKind::Propagate, &before, &mut mod_));
    let before: Vec<(usize, EntryState)> = vec![(0, EntryState::capture(&patch[0]))];
    let touched = propagate::apply_file(&mut patch, &groups, Some(1));
    assert_eq!(touched, vec![0]);
    let op = op_from_before(OpKind::Propagate, &before, &mut patch);
    s_patch.push(op);
    assert_eq!(patch[0].trans, "Traduction");
    assert_eq!(status::derive(&patch[0]), Status::Validated);
    assert_eq!(mod_[j].trans, "Traduction");
    // Ctrl+Z in each file gives its line back.
    let was_j = before_j;
    s_mod.undo(&mut mod_).unwrap();
    assert_eq!(mod_[j].trans, was_j);
    s_patch.undo(&mut patch).unwrap();
    assert_ne!(patch[0].trans, "Traduction");
}
