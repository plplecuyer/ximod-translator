//! The two locks of the VMAD lines (C-08, D-63).
//!
//! * The **file switch** « Traduire les chaînes VMAD de ce fichier »:
//!   unchecked (the default), every VMAD line is Ignoré with the provenance
//!   « verrouillée : VMAD désactivé » and the export leaves the blocks alone.
//!   The lock is laid *over* the line: a status bit ([`OFF_BIT`]) that the
//!   status derivation reads as Ignoré, and the provenance `lock:vmad-off`
//!   followed by the previous one. Lifting it gives back the status and the
//!   provenance the line had, exactly (and undo works on it like on any
//!   status change).
//! * The **rules** of the scripts' strings ([`crate::script_rules`]), applied
//!   with the switch checked to the lines without translation: Ignoré with
//!   `lock:<rule>`. Such a line is unlocked by hand (a status or a
//!   translation), the file lock never line by line.

use crate::script_rules::Rules;
use crate::status::{self, Status};
use crate::tm::TmEntry;

/// The field of the VMAD lines.
pub const FIELD: &str = "VMAD";
/// The status bit of the file lock.
pub const OFF_BIT: &str = "vmadOff";
/// Provenance prefix of every lock.
pub const LOCK: &str = "lock:";
/// The rule id of the file lock.
pub const RULE_OFF: &str = "vmad-off";

/// Whether a line is a VMAD line.
pub fn is_vmad(e: &TmEntry) -> bool {
    e.field_sig == FIELD
}

/// Whether the file lock holds the line.
pub fn is_file_locked(e: &TmEntry) -> bool {
    e.status_sst.iter().any(|s| s == OFF_BIT)
}

/// The rule of a locked line's provenance (`vmad-off`, `key`…).
pub fn lock_rule(prov: &str) -> Option<&str> {
    let rest = prov.strip_prefix(LOCK)?;
    Some(rest.split('>').next().unwrap_or(rest))
}

/// Lay the file lock on a line (idempotent).
pub fn lock_file(e: &mut TmEntry) {
    if is_file_locked(e) {
        return;
    }
    e.status_sst.push(OFF_BIT.to_string());
    let prev = std::mem::take(&mut e.prov);
    e.prov = if prev.is_empty() {
        format!("{LOCK}{RULE_OFF}")
    } else {
        format!("{LOCK}{RULE_OFF}>{prev}")
    };
}

/// Lift the file lock (idempotent): the status and provenance before it.
pub fn unlock_file(e: &mut TmEntry) {
    if !is_file_locked(e) {
        return;
    }
    e.status_sst.retain(|s| s != OFF_BIT);
    let off = format!("{LOCK}{RULE_OFF}");
    e.prov = match e.prov.strip_prefix(&off) {
        Some(rest) => rest.strip_prefix('>').unwrap_or(rest).to_string(),
        None => e.prov.clone(),
    };
}

/// The line as it is under the file lock (for the mod's dictionary, which
/// keeps the work, not the lock).
pub fn without_file_lock(e: &TmEntry) -> std::borrow::Cow<'_, TmEntry> {
    if is_file_locked(e) {
        let mut c = e.clone();
        unlock_file(&mut c);
        std::borrow::Cow::Owned(c)
    } else {
        std::borrow::Cow::Borrowed(e)
    }
}

/// A translation typed on a line a rule locked (`before_prov`): the line
/// takes the status of a typed line (the provenance « saisie » follows
/// from the settling of the edit).
pub fn unlock_by_hand(e: &mut TmEntry, before_trans: &str, before_prov: &str) {
    let by_rule = lock_rule(before_prov).is_some_and(|r| r != RULE_OFF);
    if by_rule && !is_file_locked(e) && e.trans != before_trans && !e.trans.is_empty() {
        status::set(e, Status::ToTranslate);
    }
}

/// A status set by hand on a line a rule locked: unless it is Ignoré
/// again, the lock is lifted and the provenance is « saisie ».
pub fn status_by_hand(e: &mut TmEntry) {
    let by_rule = lock_rule(&e.prov).is_some_and(|r| r != RULE_OFF);
    if by_rule && !is_file_locked(e) && status::derive(e) != Status::Ignored {
        e.prov = crate::provenance::TYPED.to_string();
    }
}

/// Lift every lock of the VMAD lines before the dictionaries fill them:
/// the file lock, and the rule locks of the lines without translation
/// (back to À traduire).
pub fn release(entries: &mut [TmEntry]) {
    for e in entries.iter_mut().filter(|e| e.field_sig == FIELD) {
        unlock_file(e);
        if e.trans.is_empty() && e.prov.starts_with(LOCK) {
            status::set(e, Status::ToTranslate);
            e.prov.clear();
        }
    }
}

/// Lay the locks after the dictionaries filled the lines: the rules on
/// the lines still À traduire without translation, then the file lock
/// when the switch is off. Returns the number of lines locked by a rule.
pub fn apply(entries: &mut [TmEntry], on: bool, rules: &Rules) -> usize {
    let mut by_rule = 0;
    for e in entries.iter_mut().filter(|e| e.field_sig == FIELD) {
        if e.trans.is_empty()
            && e.prov.is_empty()
            && status::derive(e) == Status::ToTranslate
            && let Some(rule) = rules.lock_of(&e.source)
        {
            status::set(e, Status::Ignored);
            e.prov = format!("{LOCK}{rule}");
            by_rule += 1;
        }
        if !on {
            lock_file(e);
        }
    }
    by_rule
}

/// Lay the file lock on every VMAD line when the switch is off (the rules
/// are the exclusions', B-18).
pub fn lay_file(entries: &mut [TmEntry], on: bool) {
    if on {
        return;
    }
    for e in entries.iter_mut().filter(|e| e.field_sig == FIELD) {
        lock_file(e);
    }
}

/// Turn the switch: on lifts the file lock (and replays the rules), off
/// lays it. Returns the lines changed.
pub fn switch(entries: &mut [TmEntry], on: bool, rules: &Rules) -> usize {
    let mut n = 0;
    for e in entries.iter_mut().filter(|e| e.field_sig == FIELD) {
        if on == is_file_locked(e) {
            n += 1;
        }
    }
    if on {
        for e in entries.iter_mut().filter(|e| e.field_sig == FIELD) {
            unlock_file(e);
        }
        apply(entries, true, rules);
    } else {
        for e in entries.iter_mut().filter(|e| e.field_sig == FIELD) {
            lock_file(e);
        }
    }
    n
}

/// The state of a document's switch from its lines: `None` without VMAD
/// line, else whether they are free of the file lock.
pub fn state(entries: &[TmEntry]) -> Option<bool> {
    let mut any = false;
    for e in entries.iter().filter(|e| e.field_sig == FIELD) {
        if is_file_locked(e) {
            return Some(false);
        }
        any = true;
    }
    any.then_some(true)
}

/// The number of VMAD lines.
pub fn count(entries: &[TmEntry]) -> usize {
    entries.iter().filter(|e| e.field_sig == FIELD).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(src: &str) -> TmEntry {
        let mut e = TmEntry {
            source: src.into(),
            field_sig: FIELD.into(),
            rec_sig: "QUST".into(),
            formid: 0x0100_0800,
            index: 1,
            ..Default::default()
        };
        lock_file(&mut e);
        e
    }

    #[test]
    fn the_file_lock_is_laid_over_and_lifted_exactly() {
        let mut e = line("Skip the introduction?");
        assert_eq!(status::derive(&e), Status::Ignored);
        assert_eq!(e.prov, "lock:vmad-off");
        assert_eq!(lock_rule(&e.prov), Some("vmad-off"));
        unlock_file(&mut e);
        assert_eq!(status::derive(&e), Status::ToTranslate);
        assert!(e.prov.is_empty());
        // A translated, validated line: locked, then back as it was.
        e.trans = "Passer l'introduction ?".into();
        status::set(&mut e, Status::Validated);
        e.prov = "typed".into();
        let before = e.clone();
        lock_file(&mut e);
        lock_file(&mut e);
        assert_eq!(status::derive(&e), Status::Ignored);
        assert_eq!(e.prov, "lock:vmad-off>typed");
        assert_eq!(status::derive(&without_file_lock(&e)), Status::Validated);
        unlock_file(&mut e);
        assert_eq!(e, before);
    }

    #[test]
    fn switch_rules_and_state() {
        let rules = Rules::builtin();
        let mut es = vec![
            line("Skip the introduction?"),
            line("$SKI_Help"),
            line("Help"),
        ];
        assert_eq!(state(&es), Some(false));
        assert_eq!(switch(&mut es, true, &rules), 3);
        assert_eq!(state(&es), Some(true));
        let st: Vec<(Status, &str)> = es
            .iter()
            .map(|e| (status::derive(e), e.prov.as_str()))
            .collect();
        assert_eq!(
            st,
            vec![
                (Status::ToTranslate, ""),
                (Status::Ignored, "lock:key"),
                (Status::Ignored, "lock:word"),
            ]
        );
        // Off, then on again: the same.
        switch(&mut es, false, &rules);
        assert!(es.iter().all(|e| status::derive(e) == Status::Ignored));
        switch(&mut es, true, &rules);
        assert_eq!(status::derive(&es[0]), Status::ToTranslate);
        assert_eq!(es[1].prov, "lock:key");
        // Release before the dictionaries: everything back to À traduire.
        release(&mut es);
        assert!(
            es.iter()
                .all(|e| status::derive(e) == Status::ToTranslate && e.prov.is_empty())
        );
        assert_eq!(state(&[]), None);
    }
}
