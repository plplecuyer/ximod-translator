//! A translation job in a background thread (R-D01-03, R-D01-07): the
//! cache first, then the batches of the plan through the service, one
//! retry after a second, the rate kept by the limiter, every answer
//! reported as it comes; « Arrêter » is honoured after the current request.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use super::quota::{Item, Plan, RateLimiter};
use super::{Error, OnlineConfig, Service};

/// What the thread needs.
pub struct JobInput {
    pub service: Service,
    pub cfg: OnlineConfig,
    pub key: String,
    /// ISO 639-1 codes.
    pub src: String,
    pub dst: String,
    /// Protected texts, one per distinct source, keyed by their position.
    pub items: Vec<Item>,
    /// Where the cache file is (`None` = no cache).
    pub cache_path: Option<std::path::PathBuf>,
}

/// What the thread reports.
#[derive(Debug, Clone)]
pub enum JobMsg {
    /// Requests done / total, texts answered so far.
    Progress {
        done: usize,
        total: usize,
        received: usize,
    },
    /// One text's answer (`cached` when it came from the cache).
    Answer {
        key: usize,
        result: Result<String, Error>,
        cached: bool,
    },
    /// The end, with how many characters went to the service.
    Done { stopped: bool, sent_chars: usize },
}

/// The plan of a job, before it runs (the dialog's estimate).
pub fn plan_of(input: &JobInput) -> Plan {
    super::quota::plan(
        input.items.clone(),
        input.cfg.max_chars,
        input.cfg.max_lines,
    )
}

/// Start the job; the receiver gets the messages, the flag stops it.
pub fn start(input: JobInput, stop: Arc<AtomicBool>) -> Receiver<JobMsg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || run(input, stop, tx));
    rx
}

fn run(input: JobInput, stop: Arc<AtomicBool>, tx: Sender<JobMsg>) {
    let connector = super::connector(&input.service);
    let mut cache = input
        .cache_path
        .as_ref()
        .map(|p| super::cache::Cache::open_at(p.clone(), &input.dst));
    // The cache first.
    let mut remaining: Vec<Item> = Vec::new();
    let mut received = 0usize;
    for it in &input.items {
        match cache
            .as_ref()
            .and_then(|c| c.get(&it.text).map(str::to_string))
        {
            Some(answer) => {
                received += 1;
                let _ = tx.send(JobMsg::Answer {
                    key: it.key,
                    result: Ok(answer),
                    cached: true,
                });
            }
            None => remaining.push(it.clone()),
        }
    }
    let plan = super::quota::plan(remaining, input.cfg.max_chars, input.cfg.max_lines);
    for it in &plan.too_long {
        let _ = tx.send(JobMsg::Answer {
            key: it.key,
            result: Err(Error::TooLong),
            cached: false,
        });
    }
    let total = plan.batches.len();
    let _ = tx.send(JobMsg::Progress {
        done: 0,
        total,
        received,
    });
    let mut limiter = RateLimiter::new(input.cfg.max_rpm);
    let mut sent_chars = 0usize;
    let mut stopped = false;
    for (n, batch) in plan.batches.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        limiter.wait();
        let texts: Vec<String> = batch.iter().map(|i| i.text.clone()).collect();
        let chars: usize = texts.iter().map(|t| t.chars().count()).sum();
        let mut result =
            connector.translate(&input.cfg, &input.key, &texts, &input.src, &input.dst);
        if let Err(Error::Network(_))
        | Err(Error::Http {
            code: 500..=599, ..
        }) = &result
        {
            // One retry after a second (R-D01-07).
            std::thread::sleep(std::time::Duration::from_secs(1));
            limiter.wait();
            result = connector.translate(&input.cfg, &input.key, &texts, &input.src, &input.dst);
        }
        // An answer with fewer or more texts: the batch again, line by
        // line (R-D03-03).
        if let Err(Error::Incomplete { .. }) = &result
            && batch.len() > 1
        {
            let mut one_by_one = Vec::with_capacity(batch.len());
            let mut failed = None;
            for t in &texts {
                limiter.wait();
                sent_chars += t.chars().count();
                match connector.translate(
                    &input.cfg,
                    &input.key,
                    std::slice::from_ref(t),
                    &input.src,
                    &input.dst,
                ) {
                    Ok(mut a) if a.len() == 1 => one_by_one.push(a.remove(0)),
                    Ok(a) => {
                        failed = Some(Error::Incomplete {
                            got: a.len(),
                            sent: 1,
                        });
                        break;
                    }
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            result = match failed {
                None => Ok(one_by_one),
                Some(e) => Err(e),
            };
        }
        sent_chars += chars;
        match result {
            Ok(answers) => {
                for (it, a) in batch.iter().zip(answers) {
                    if let Some(c) = cache.as_mut() {
                        c.put(&it.text, &a);
                    }
                    received += 1;
                    let _ = tx.send(JobMsg::Answer {
                        key: it.key,
                        result: Ok(a),
                        cached: false,
                    });
                }
                if let Some(c) = cache.as_mut()
                    && let Err(e) = c.save()
                {
                    tracing::warn!("online cache: {e}");
                }
            }
            Err(e) => {
                for it in batch {
                    let _ = tx.send(JobMsg::Answer {
                        key: it.key,
                        result: Err(e.clone()),
                        cached: false,
                    });
                }
            }
        }
        let _ = tx.send(JobMsg::Progress {
            done: n + 1,
            total,
            received,
        });
    }
    let _ = tx.send(JobMsg::Done {
        stopped,
        sent_chars,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A job whose answers all come from the cache sends no request.
    #[test]
    fn cached_answers_need_no_service() {
        let d = std::env::temp_dir().join(format!("ximod_online_job_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("deepl_EN-FR.ximdb");
        let mut c = super::super::cache::Cache::open_at(p.clone(), "fr");
        c.put("Hello", "Bonjour");
        c.save().unwrap();
        let input = JobInput {
            service: Service::DeepL,
            cfg: OnlineConfig::default(),
            key: String::new(),
            src: "en".into(),
            dst: "fr".into(),
            items: vec![Item {
                key: 7,
                text: "Hello".into(),
            }],
            cache_path: Some(p),
        };
        let rx = start(input, Arc::new(AtomicBool::new(false)));
        let msgs: Vec<JobMsg> = rx.iter().collect();
        assert!(matches!(
            msgs[0],
            JobMsg::Answer {
                key: 7,
                cached: true,
                ..
            }
        ));
        assert!(matches!(
            msgs.last(),
            Some(JobMsg::Done {
                stopped: false,
                sent_chars: 0
            })
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Without a key, DeepL refuses before any network call; the job
    /// reports the error per text and ends.
    #[test]
    fn missing_key_is_reported_per_text() {
        let input = JobInput {
            service: Service::DeepL,
            cfg: OnlineConfig::default(),
            key: String::new(),
            src: "en".into(),
            dst: "fr".into(),
            items: vec![
                Item {
                    key: 1,
                    text: "a".into(),
                },
                Item {
                    key: 2,
                    text: "b".into(),
                },
            ],
            cache_path: None,
        };
        let rx = start(input, Arc::new(AtomicBool::new(false)));
        let msgs: Vec<JobMsg> = rx.iter().collect();
        let errs = msgs
            .iter()
            .filter(|m| {
                matches!(
                    m,
                    JobMsg::Answer {
                        result: Err(Error::NoKey),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(errs, 2);
    }
}
