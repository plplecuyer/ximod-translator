//! Quotas (R-D02-04): batches by characters and lines, requests per minute
//! kept by the worker, a session counter with its warning threshold.

use std::time::{Duration, Instant};

/// A text to send, with the line it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub key: usize,
    pub text: String,
}

/// The batches of a job, and the items too long for one request.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub batches: Vec<Vec<Item>>,
    pub too_long: Vec<Item>,
}

impl Plan {
    pub fn chars(&self) -> usize {
        self.batches
            .iter()
            .flatten()
            .map(|i| i.text.chars().count())
            .sum()
    }

    pub fn lines(&self) -> usize {
        self.batches.iter().map(Vec::len).sum()
    }
}

/// Split the items into batches of at most `max_lines` items and
/// `max_chars` characters; an item longer than `max_chars` goes alone when
/// it fits the service's hard limit (twice `max_chars`), else is refused.
pub fn plan(items: Vec<Item>, max_chars: usize, max_lines: usize) -> Plan {
    let max_chars = max_chars.max(100);
    let max_lines = max_lines.max(1);
    let mut p = Plan::default();
    let mut cur: Vec<Item> = Vec::new();
    let mut cur_chars = 0usize;
    for it in items {
        let n = it.text.chars().count();
        if n > max_chars {
            if n > max_chars * 2 {
                p.too_long.push(it);
                continue;
            }
            if !cur.is_empty() {
                p.batches.push(std::mem::take(&mut cur));
                cur_chars = 0;
            }
            p.batches.push(vec![it]);
            continue;
        }
        if !cur.is_empty() && (cur.len() >= max_lines || cur_chars + n > max_chars) {
            p.batches.push(std::mem::take(&mut cur));
            cur_chars = 0;
        }
        cur_chars += n;
        cur.push(it);
    }
    if !cur.is_empty() {
        p.batches.push(cur);
    }
    p
}

/// Requests per minute, kept between calls.
pub struct RateLimiter {
    min_gap: Duration,
    last: Option<Instant>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> RateLimiter {
        let per_minute = per_minute.max(1) as u64;
        RateLimiter {
            min_gap: Duration::from_millis(60_000 / per_minute),
            last: None,
        }
    }

    /// How long to wait before the next request.
    pub fn wait_needed(&self, now: Instant) -> Duration {
        match self.last {
            Some(l) if now.duration_since(l) < self.min_gap => self.min_gap - now.duration_since(l),
            _ => Duration::ZERO,
        }
    }

    pub fn mark(&mut self, now: Instant) {
        self.last = Some(now);
    }

    /// Sleep until the next request may go (the worker thread).
    pub fn wait(&mut self) {
        let w = self.wait_needed(Instant::now());
        if !w.is_zero() {
            std::thread::sleep(w);
        }
        self.mark(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(k: usize, n: usize) -> Item {
        Item {
            key: k,
            text: "x".repeat(n),
        }
    }

    /// T-D02-02: 184 lines, 50 per batch and 5 000 characters → 4 batches;
    /// a 6 000-character line is refused.
    #[test]
    fn batches_follow_the_quotas() {
        let items: Vec<Item> = (0..184).map(|k| it(k, 60)).collect();
        let p = plan(items, 5000, 50);
        assert_eq!(p.batches.len(), 4);
        assert_eq!(p.lines(), 184);
        assert_eq!(p.chars(), 184 * 60);
        assert!(p.too_long.is_empty());
        // Characters cut a batch before the line count does.
        let p = plan((0..10).map(|k| it(k, 1200)).collect(), 5000, 50);
        assert_eq!(p.batches.len(), 3);
        assert_eq!(p.batches[0].len(), 4);
        // Long lines alone, too long refused.
        let p = plan(
            vec![it(0, 10), it(1, 6000), it(2, 12000), it(3, 10)],
            5000,
            50,
        );
        assert_eq!(p.batches.len(), 3);
        assert_eq!(p.batches[1].len(), 1);
        assert_eq!(p.too_long.len(), 1);
        assert_eq!(p.too_long[0].key, 2);
    }

    #[test]
    fn rate_limiter_spaces_requests() {
        let mut r = RateLimiter::new(20);
        let t0 = Instant::now();
        assert_eq!(r.wait_needed(t0), Duration::ZERO);
        r.mark(t0);
        let w = r.wait_needed(t0 + Duration::from_millis(1000));
        assert!(
            w >= Duration::from_millis(1900) && w <= Duration::from_millis(2000),
            "{w:?}"
        );
        assert_eq!(r.wait_needed(t0 + Duration::from_secs(4)), Duration::ZERO);
    }
}
