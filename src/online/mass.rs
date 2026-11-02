//! Traduction de masse (D-04): the lines a scope sends and those it
//! leaves (R-D04-01 / 02), the distinct sources protected once, the
//! answers restored and posed En attente with their provenance, the
//! account of what was received, refused (by reason) and left by a stop
//! (R-D04-03). Shared by the « Traduire en ligne » dialog and the command
//! line, so both write the same lines, statuses and provenances.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::Receiver;

use super::job::JobMsg;
use super::protect::{self, Token};
use super::{Error, cache::Cache};
use crate::games::Game;
use crate::status::{self, Status};
use crate::tags::TagSet;
use crate::tm::TmEntry;

/// A distinct source sent once.
#[derive(Debug, Clone)]
pub struct Sent {
    pub protected: String,
    pub tokens: Vec<Token>,
    pub original: String,
}

/// The lines a scope leaves aside (R-D04-02).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skipped {
    /// Locked by the exclusion rules, Ignoré, never shown in game: never
    /// sent.
    pub locked: usize,
    /// Already translated, not sent again (« Seulement les lignes à
    /// traduire »).
    pub done: usize,
}

/// The lines of `base` a job sends (R-D04-01), and those it leaves.
pub fn select(
    entries: &[TmEntry],
    base: impl IntoIterator<Item = usize>,
    only_untranslated: bool,
) -> (Vec<usize>, Skipped) {
    let mut out = Vec::new();
    let mut skipped = Skipped::default();
    for i in base {
        let Some(e) = entries.get(i) else {
            continue;
        };
        if e.source.trim().is_empty() {
            continue;
        }
        if e.unused || status::derive(e) == Status::Ignored {
            skipped.locked += 1;
            continue;
        }
        if only_untranslated && !e.trans.trim().is_empty() {
            skipped.done += 1;
            continue;
        }
        out.push(i);
    }
    (out, skipped)
}

/// The lines whose Validé / Personnalisé translation a job would replace
/// (asked before, R-D01-02).
pub fn protected_lines(entries: &[TmEntry], lines: &[usize]) -> usize {
    lines
        .iter()
        .filter(|&&i| {
            entries.get(i).is_some_and(|e| {
                !e.trans.is_empty()
                    && matches!(status::derive(e), Status::Validated | Status::Custom)
            })
        })
        .count()
}

/// The distinct sources of lines, protected, and line → source.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    pub items: Vec<Sent>,
    pub lines: Vec<(usize, usize)>,
}

impl Prepared {
    /// The items of the job (protected texts keyed by position).
    pub fn job_items(&self) -> Vec<super::quota::Item> {
        self.items
            .iter()
            .enumerate()
            .map(|(k, s)| super::quota::Item {
                key: k,
                text: s.protected.clone(),
            })
            .collect()
    }

    /// The distinct sources the cache already answers (R-D04-02).
    pub fn cached(&self, cache: &Cache) -> usize {
        self.items
            .iter()
            .filter(|s| cache.get(&s.protected).is_some())
            .count()
    }

    /// The lines of the distinct sources the cache answers.
    pub fn cached_lines(&self, cache: &Cache) -> usize {
        self.lines
            .iter()
            .filter(|(_, k)| {
                self.items
                    .get(*k)
                    .is_some_and(|s| cache.get(&s.protected).is_some())
            })
            .count()
    }
}

/// Protect the sources of `lines`, each distinct one once (R-D01-03).
pub fn prepare(
    entries: &[TmEntry],
    lines: &[usize],
    game: Game,
    opts: protect::Options,
) -> Prepared {
    let mut p = Prepared::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    for &i in lines {
        let Some(e) = entries.get(i) else {
            continue;
        };
        let set = TagSet::for_entry(e, game);
        let (text, tokens) = protect::protect(&e.source, &set, opts);
        let key = match index.get(&text) {
            Some(&k) => k,
            None => {
                let k = p.items.len();
                index.insert(text.clone(), k);
                p.items.push(Sent {
                    protected: text,
                    tokens,
                    original: e.source.clone(),
                });
                k
            }
        };
        p.lines.push((i, key));
    }
    p
}

/// Why an answer was not posed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// A protected token lost, doubled or altered.
    TokenLost,
    /// Nothing came back.
    Empty,
    /// The source again.
    Unchanged,
    /// Longer than a request may be: not sent.
    TooLong,
    /// The service's error (HTTP code, 0 without one) and its message.
    Service { code: u16, message: String },
}

/// What became of a distinct source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The translation, tokens restored.
    Taken(String),
    Refused(Reason),
    /// Not sent: the job was stopped before (R-D04-03).
    Left,
}

/// The answers of a job restored, one per distinct source.
pub fn outcomes(items: &[Sent], answers: &HashMap<usize, Result<String, Error>>) -> Vec<Outcome> {
    items
        .iter()
        .enumerate()
        .map(|(k, sent)| match answers.get(&k) {
            Some(Ok(a)) => match protect::restore(a, &sent.tokens, &sent.original) {
                Ok(t) => Outcome::Taken(t),
                Err(protect::Refusal::TokenLost(_)) => Outcome::Refused(Reason::TokenLost),
                Err(protect::Refusal::Empty) => Outcome::Refused(Reason::Empty),
                Err(protect::Refusal::Unchanged) => Outcome::Refused(Reason::Unchanged),
            },
            Some(Err(Error::TooLong)) => Outcome::Refused(Reason::TooLong),
            Some(Err(e)) => Outcome::Refused(Reason::Service {
                code: e.code(),
                message: e.message(),
            }),
            None => Outcome::Left,
        })
        .collect()
}

/// The account of a job, in lines (R-D04-02).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub received: usize,
    pub refused: usize,
    pub left: usize,
    /// The refused lines per reason.
    pub by_reason: BTreeMap<Reason, usize>,
}

/// Pose the answers on the lines: the translation, En attente, the
/// provenance; never Validé (R-D04-03). Lines refused or left keep what
/// they had.
pub fn apply(
    entries: &mut [TmEntry],
    lines: &[(usize, usize)],
    outcomes: &[Outcome],
    prov: &str,
) -> Summary {
    let mut s = Summary::default();
    for (line, key) in lines {
        match outcomes.get(*key) {
            Some(Outcome::Taken(t)) => {
                if let Some(e) = entries.get_mut(*line) {
                    e.trans = t.clone();
                    e.prov = prov.to_string();
                    status::set(e, Status::Pending);
                    s.received += 1;
                }
            }
            Some(Outcome::Refused(r)) => {
                s.refused += 1;
                *s.by_reason.entry(r.clone()).or_default() += 1;
            }
            Some(Outcome::Left) | None => s.left += 1,
        }
    }
    s
}

/// A job's messages gathered to its end (the command line, the tests).
#[derive(Debug, Default)]
pub struct Collected {
    pub answers: HashMap<usize, Result<String, Error>>,
    pub stopped: bool,
    pub sent_chars: usize,
    pub cached: usize,
    /// Requests done / planned.
    pub done: usize,
    pub total: usize,
}

/// Drain a job; `on_progress(done, total)` after each request (to stop it,
/// for example).
pub fn collect(rx: Receiver<JobMsg>, mut on_progress: impl FnMut(usize, usize)) -> Collected {
    let mut c = Collected::default();
    for m in rx {
        match m {
            JobMsg::Progress { done, total, .. } => {
                c.done = done;
                c.total = total;
                on_progress(done, total);
            }
            JobMsg::Answer {
                key,
                result,
                cached,
            } => {
                if cached {
                    c.cached += 1;
                }
                c.answers.insert(key, result);
            }
            JobMsg::Done {
                stopped,
                sent_chars,
            } => {
                c.stopped = stopped;
                c.sent_chars = sent_chars;
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str, trans: &str) -> TmEntry {
        TmEntry {
            source: source.into(),
            trans: trans.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_scope_leaves_locked_and_translated_lines() {
        let mut v = vec![
            entry("Sword", ""),
            entry("Shield", "Bouclier"),
            entry("Bow", ""),
            entry("", ""),
            entry("Unused", ""),
        ];
        status::set(&mut v[2], Status::Ignored);
        v[4].unused = true;
        let (lines, sk) = select(&v, 0..v.len(), true);
        assert_eq!(lines, vec![0]);
        assert_eq!(sk, Skipped { locked: 2, done: 1 });
        let (lines, sk) = select(&v, 0..v.len(), false);
        assert_eq!(lines, vec![0, 1]);
        assert_eq!(sk.done, 0);
    }

    #[test]
    fn answers_are_posed_pending_and_counted() {
        let mut v = vec![entry("Sword", ""), entry("Sword", ""), entry("Bow", "")];
        let p = prepare(&v, &[0, 1, 2], Game::SkyrimSE, protect::Options::all());
        assert_eq!(p.items.len(), 2);
        let mut answers = HashMap::new();
        answers.insert(0, Ok("Épée".to_string()));
        let out = outcomes(&p.items, &answers);
        assert_eq!(out[1], Outcome::Left);
        let s = apply(&mut v, &p.lines, &out, "online:deepl");
        assert_eq!((s.received, s.refused, s.left), (2, 0, 1));
        assert_eq!(status::derive(&v[0]), Status::Pending);
        assert_eq!(v[0].prov, "online:deepl");
        assert!(v[2].trans.is_empty());
        answers.insert(1, Ok("Bow".to_string()));
        let out = outcomes(&p.items, &answers);
        let s = apply(&mut v, &p.lines, &out, "online:deepl");
        assert_eq!(s.by_reason.get(&Reason::Unchanged), Some(&1));
    }
}
