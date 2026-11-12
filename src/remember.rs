//! Remembered answers of confirmation dialogs (E-03, D-109).
//!
//! A dialog that may be remembered asks [`Remembered::get`] first: when an
//! answer is known it is not shown and the answer applies as if the user had
//! clicked it (the interface notes it in the journal). Only the questions of
//! [`Question`] are eligible — never one whose answer can lose work — and
//! « Annuler » is never an answer. Permanent answers live in Config.ini,
//! `[Confirmations] <question>=<answer>`; session answers are never written.
//! The command line neither asks nor reads them.

use std::collections::{BTreeMap, HashMap};

/// The dialogs whose answer may be remembered (R-E03-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Question {
    /// Propagation to the lines of the same source, proposed after F5 or
    /// Ctrl+↓ (A-12).
    Propagate,
    /// « Est-ce la bonne sortie ? » before the export.
    OutputConfirm,
    /// Replace the translation by the instant translation (B-14).
    InstantReplace,
    /// Personal dictionary: pairs already known with another translation.
    PersonalConflict,
    /// Online translation: validated or custom lines would be replaced.
    OnlineOverwrite,
    /// Online translation: the volume of the session is exceeded.
    OnlineVolume,
    /// Saving a dictionary: is it the right game?
    DictGame,
    /// Clear the journal of the session (E-01).
    JournalClear,
}

/// The answer: the index of the chosen button in [`Question::answers`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Answer(pub u8);

impl Answer {
    /// The first (and, for most questions, only) remembered answer.
    pub const YES: Answer = Answer(0);
}

/// How long an answer is remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Until the application is closed.
    Session,
    /// In Config.ini, until « Réafficher les questions ».
    Forever,
}

impl Scope {
    /// The message key of the scope (Paramètres › Général).
    pub fn i18n_key(self) -> &'static str {
        match self {
            Scope::Session => "settings-remembered-scope-session",
            Scope::Forever => "settings-remembered-scope-forever",
        }
    }
}

impl Question {
    /// Every eligible question, in the order of the settings list.
    pub const ALL: [Question; 8] = [
        Question::Propagate,
        Question::OutputConfirm,
        Question::InstantReplace,
        Question::PersonalConflict,
        Question::OnlineOverwrite,
        Question::OnlineVolume,
        Question::DictGame,
        Question::JournalClear,
    ];

    /// The stable name under `[Confirmations]`.
    pub fn key(self) -> &'static str {
        match self {
            Question::Propagate => "Propagate",
            Question::OutputConfirm => "OutputConfirm",
            Question::InstantReplace => "InstantReplace",
            Question::PersonalConflict => "PersonalConflict",
            Question::OnlineOverwrite => "OnlineOverwrite",
            Question::OnlineVolume => "OnlineVolume",
            Question::DictGame => "DictGame",
            Question::JournalClear => "JournalClear",
        }
    }

    /// The question named `key` in Config.ini (case-insensitive).
    pub fn from_key(key: &str) -> Option<Question> {
        Self::ALL
            .into_iter()
            .find(|q| q.key().eq_ignore_ascii_case(key.trim()))
    }

    /// The message key of the question's short name.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Question::Propagate => "question-propagate",
            Question::OutputConfirm => "question-output",
            Question::InstantReplace => "question-instant",
            Question::PersonalConflict => "question-personal",
            Question::OnlineOverwrite => "question-online-overwrite",
            Question::OnlineVolume => "question-online-volume",
            Question::DictGame => "question-dict-game",
            Question::JournalClear => "question-journal-clear",
        }
    }

    /// The answers that may be remembered: `(Config.ini value, message key
    /// of the button)`, indexed by [`Answer`].
    pub fn answers(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Question::Propagate => &[("yes", "answer-propagate")],
            Question::OutputConfirm | Question::OnlineOverwrite | Question::OnlineVolume => {
                &[("yes", "btn-yes")]
            }
            Question::InstantReplace => &[("yes", "instant-replace")],
            Question::PersonalConflict => {
                &[("replace", "personal-replace"), ("keep", "personal-keep")]
            }
            Question::DictGame => &[("yes", "btn-validate")],
            Question::JournalClear => &[("yes", "journal-clear")],
        }
    }

    /// The Config.ini value of `a`, `None` when it is not an answer.
    pub fn value(self, a: Answer) -> Option<&'static str> {
        self.answers().get(a.0 as usize).map(|(v, _)| *v)
    }

    /// The message key of the button of `a`.
    pub fn answer_key(self, a: Answer) -> Option<&'static str> {
        self.answers().get(a.0 as usize).map(|(_, k)| *k)
    }

    /// The answer written `value` in Config.ini (case-insensitive).
    pub fn parse(self, value: &str) -> Option<Answer> {
        self.answers()
            .iter()
            .position(|(v, _)| v.eq_ignore_ascii_case(value.trim()))
            .map(|i| Answer(i as u8))
    }
}

/// The remembered answers of the session and the permanent ones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Remembered {
    session: HashMap<Question, Answer>,
    permanent: HashMap<Question, Answer>,
}

impl Remembered {
    /// The permanent answers of `[Confirmations]`; unknown questions and
    /// values are ignored.
    pub fn from_config(map: &BTreeMap<String, String>) -> Self {
        let mut r = Self::default();
        for (k, v) in map {
            if let Some(q) = Question::from_key(k)
                && let Some(a) = q.parse(v)
            {
                r.permanent.insert(q, a);
            }
        }
        r
    }

    /// The permanent answers, as `[Confirmations]` writes them.
    pub fn to_config(&self) -> BTreeMap<String, String> {
        self.permanent
            .iter()
            .filter_map(|(q, a)| Some((q.key().to_string(), q.value(*a)?.to_string())))
            .collect()
    }

    /// The remembered answer of `q`, if any.
    pub fn get(&self, q: Question) -> Option<Answer> {
        self.session
            .get(&q)
            .or_else(|| self.permanent.get(&q))
            .copied()
    }

    /// The scope of the remembered answer of `q`.
    pub fn scope(&self, q: Question) -> Option<Scope> {
        if self.session.contains_key(&q) {
            Some(Scope::Session)
        } else if self.permanent.contains_key(&q) {
            Some(Scope::Forever)
        } else {
            None
        }
    }

    /// Remember `a` for `q`; false (nothing kept) when `a` is not one of the
    /// question's answers. A new answer replaces the other scope's.
    pub fn remember(&mut self, q: Question, a: Answer, scope: Scope) -> bool {
        if q.value(a).is_none() {
            return false;
        }
        self.session.remove(&q);
        self.permanent.remove(&q);
        match scope {
            Scope::Session => self.session.insert(q, a),
            Scope::Forever => self.permanent.insert(q, a),
        };
        true
    }

    /// Forget everything, session and permanent (« Réafficher les
    /// questions »).
    pub fn clear(&mut self) {
        self.session.clear();
        self.permanent.clear();
    }

    /// Nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.session.is_empty() && self.permanent.is_empty()
    }

    /// `(session, permanent)` counts.
    pub fn counts(&self) -> (usize, usize) {
        (self.session.len(), self.permanent.len())
    }

    /// The remembered answers, in the order of [`Question::ALL`].
    pub fn list(&self) -> Vec<(Question, Answer, Scope)> {
        Question::ALL
            .into_iter()
            .filter_map(|q| Some((q, self.get(q)?, self.scope(q)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for q in Question::ALL {
            assert!(seen.insert(q.key()));
            assert_eq!(Question::from_key(q.key()), Some(q));
            assert_eq!(Question::from_key(&q.key().to_lowercase()), Some(q));
            assert!(!q.answers().is_empty());
            for i in 0..q.answers().len() {
                let a = Answer(i as u8);
                assert_eq!(q.parse(q.value(a).unwrap()), Some(a));
            }
        }
        assert_eq!(Question::from_key("CloseUnsaved"), None);
    }

    #[test]
    fn config_round_trip_ignores_unknown_entries() {
        let mut map = BTreeMap::new();
        map.insert("Propagate".to_string(), "yes".to_string());
        map.insert("PersonalConflict".to_string(), "Keep".to_string());
        map.insert("CloseUnsaved".to_string(), "yes".to_string());
        map.insert("JournalClear".to_string(), "maybe".to_string());
        let r = Remembered::from_config(&map);
        assert_eq!(r.get(Question::Propagate), Some(Answer::YES));
        assert_eq!(r.get(Question::PersonalConflict), Some(Answer(1)));
        assert_eq!(r.get(Question::JournalClear), None);
        assert_eq!(r.scope(Question::Propagate), Some(Scope::Forever));
        let out = r.to_config();
        assert_eq!(out.len(), 2);
        assert_eq!(out["Propagate"], "yes");
        assert_eq!(out["PersonalConflict"], "keep");
    }

    #[test]
    fn session_answers_are_not_written() {
        let mut r = Remembered::default();
        assert!(r.is_empty());
        assert!(r.remember(Question::OutputConfirm, Answer::YES, Scope::Session));
        assert_eq!(r.get(Question::OutputConfirm), Some(Answer::YES));
        assert_eq!(r.scope(Question::OutputConfirm), Some(Scope::Session));
        assert!(r.to_config().is_empty());
        assert_eq!(r.counts(), (1, 0));
    }

    #[test]
    fn a_new_scope_replaces_the_other_and_clear_forgets_all() {
        let mut r = Remembered::default();
        r.remember(Question::Propagate, Answer::YES, Scope::Session);
        r.remember(Question::Propagate, Answer::YES, Scope::Forever);
        assert_eq!(r.counts(), (0, 1));
        r.remember(Question::DictGame, Answer::YES, Scope::Session);
        assert_eq!(
            r.list(),
            vec![
                (Question::Propagate, Answer::YES, Scope::Forever),
                (Question::DictGame, Answer::YES, Scope::Session),
            ]
        );
        r.clear();
        assert!(r.is_empty());
        assert!(r.to_config().is_empty());
    }

    #[test]
    fn an_index_out_of_range_is_not_an_answer() {
        let mut r = Remembered::default();
        assert!(!r.remember(Question::JournalClear, Answer(1), Scope::Forever));
        assert!(r.is_empty());
        assert!(r.remember(Question::PersonalConflict, Answer(1), Scope::Forever));
    }
}
