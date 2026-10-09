//! Catch-up requests spread over every upstream (2026-10-07: the founder's
//! Mac, 10,320 blocks behind, gained ~1.5 blocks a second).
//!
//! The follower used to fire 256 block requests at once at ONE validator,
//! whose transport admits 16 per peer and 32 a second: 240 came back "server
//! busy", and every one of those rotated the shared connection to another
//! validator mid-round — which also landed the snapshot jump's chunk requests
//! on a validator holding a different snapshot ("snapshot moved on"). Here:
//!
//! - each source gets at most [`PER_SOURCE`] requests in flight (half the
//!   transport's per-peer cap), on a connection of its own, and spans of
//!   blocks are dealt round-robin over all sources;
//! - a span is one `aether_getFinalizedRange` request where the source has it
//!   (upgraded validators), or single `aether_getFinalized` requests where it
//!   does not (the 7780 validators of 2026-09-29) — remembered per source;
//! - "server busy" is waited out (the server's `retry_after_ms`, else an
//!   exponential backoff), never treated as a bad source;
//! - every block is verified exactly as before (`follow::check`: the
//!   committee's certificate, then the block decoded canonically).

use crate::block::Block;
use crate::follow::{check, Upstream};
use aether_light::ValidatorSet;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

/// Blocks per span (one range request, or this many single requests).
pub const RANGE: u64 = 32;
/// Requests one iroh source may have in flight while following: half the
/// transport's per-peer cap of 16 (`aether_net`), so wallets on the same
/// node id and the height probes still get through (`Upstream::per_source`).
pub const PER_SOURCE: usize = 8;
/// How many times one request is retried while its source answers busy.
const BUSY_ATTEMPTS: u32 = 8;
/// The first wait after a busy answer without a hint (old servers), doubled
/// up to [`BUSY_MAX_WAIT`].
const BUSY_FIRST_WAIT: Duration = Duration::from_millis(50);
const BUSY_MAX_WAIT: Duration = Duration::from_secs(2);

/// Sources known not to serve `aether_getFinalizedRange` (older binaries),
/// by URL or node id. A source is asked once; afterwards singles go straight out.
static NO_RANGE: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn range_supported(key: &str) -> bool {
    !NO_RANGE
        .lock()
        .expect("range support")
        .as_ref()
        .is_some_and(|s| s.contains(key))
}

fn note_no_range(key: String) {
    NO_RANGE
        .lock()
        .expect("range support")
        .get_or_insert_with(HashSet::new)
        .insert(key);
}

/// Whether a refusal means "this method does not exist here" (an older
/// server, or a gateway that does not serve it), not a failure of the call.
pub fn unknown_method(e: &str) -> bool {
    e.contains("method not found") || e.contains("not a public read method")
}

/// Whether an answer is the transport's (or a handler's) "server busy".
pub fn is_busy(e: &str) -> bool {
    e.contains("server busy")
}

/// How long to wait after the `attempt`-th busy answer (0-based): the
/// server's hint when it sent one, else an exponential backoff. Both are
/// bounded: a peer's hint must not suspend following indefinitely.
fn busy_wait(e: &str, attempt: u32) -> Duration {
    aether_net::busy_retry_after(e)
        .unwrap_or_else(|| {
            BUSY_FIRST_WAIT
                .saturating_mul(1 << attempt.min(8))
                .min(BUSY_MAX_WAIT)
        })
        .max(Duration::from_millis(5))
        .min(BUSY_MAX_WAIT)
}

/// One request to source `i`, waiting out "server busy" (bounded): a full
/// server is asked again, not abandoned, and its connection is kept.
pub async fn call_patient(
    up: &Upstream,
    i: usize,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    let mut attempt = 0;
    loop {
        match up.call_at(i, method, params.clone()).await {
            Err(e) if is_busy(&e) && attempt + 1 < BUSY_ATTEMPTS => {
                tokio::time::sleep(busy_wait(&e, attempt)).await;
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// [`check`] on the blocking pool: a certificate is a pairing, and a span's
/// worth of them verifies in parallel instead of one by one on the runtime.
async fn check_off_thread(
    set: &ValidatorSet,
    h: u64,
    v: Value,
) -> Result<Option<Certified>, String> {
    let set = set.clone();
    tokio::task::spawn_blocking(move || check(&set, h, v))
        .await
        .map_err(|e| format!("certificate check: {e}"))?
}

/// Request slots per source: no source ever sees more than its share at once.
pub struct Slots(Vec<tokio::sync::Semaphore>);

impl Slots {
    pub fn new(sources: usize, per_source: usize) -> Self {
        Slots(
            (0..sources.max(1))
                .map(|_| tokio::sync::Semaphore::new(per_source.max(1)))
                .collect(),
        )
    }

    async fn take(&self, i: usize) -> tokio::sync::SemaphorePermit<'_> {
        self.0[i % self.0.len()]
            .acquire()
            .await
            .expect("slots are never closed")
    }
}

/// `from..=to` cut into spans of at most `RANGE` heights, ascending.
pub fn spans(from: u64, to: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut a = from;
    while a <= to {
        let b = to.min(a.saturating_add(RANGE - 1));
        out.push((a, b));
        if b == u64::MAX {
            break;
        }
        a = b + 1;
    }
    out
}

/// A verified block and its canonical proof (as `aether_getFinalized` serves it).
pub type Certified = (Block, Value);

/// The certified blocks `from..=to` (a prefix of them: a source stops where
/// it has nothing yet), from the source span `k` is dealt to, then the
/// others in turn. The longest verified prefix wins; when no source gave any
/// block, the last error comes back (a "pruned" one sends the caller to the
/// era file, as before).
pub async fn fetch_span(
    up: &Upstream,
    set: &ValidatorSet,
    slots: &Slots,
    k: usize,
    from: u64,
    to: u64,
) -> Result<Vec<Certified>, String> {
    let n = up.sources();
    if n == 0 {
        return Err("no upstream".into());
    }
    let want = (to - from + 1) as usize;
    let mut best: Vec<Certified> = Vec::new();
    let mut last_err: Option<String> = None;
    for j in 0..n {
        let i = (k + j) % n;
        match fetch_span_from(up, set, slots, i, from, to).await {
            Ok(got) if got.len() == want => return Ok(got),
            Ok(got) => {
                if got.len() > best.len() {
                    best = got;
                }
            }
            Err(e) => {
                // A pruned answer outranks a busy or transport one: it says
                // where the blocks are (the era file).
                if last_err.as_deref().is_none_or(|l| !l.contains("pruned")) {
                    last_err = Some(e);
                }
            }
        }
    }
    match (best.is_empty(), last_err) {
        (true, Some(e)) => Err(e),
        _ => Ok(best),
    }
}

/// [`fetch_span`] from source `i` alone.
async fn fetch_span_from(
    up: &Upstream,
    set: &ValidatorSet,
    slots: &Slots,
    i: usize,
    from: u64,
    to: u64,
) -> Result<Vec<Certified>, String> {
    let key = up.source_key(i);
    let mut out = Vec::new();
    if to > from && range_supported(&key) {
        let mut cursor = from;
        while cursor <= to {
            let answer = {
                let _slot = slots.take(i).await;
                call_patient(
                    up,
                    i,
                    "aether_getFinalizedRange",
                    &json!([cursor, to - cursor + 1]),
                )
                .await
            };
            match answer {
                Ok(Value::Array(items)) => {
                    if items.is_empty() {
                        return Ok(out);
                    }
                    let checked = futures::future::join_all(
                        (cursor..=to)
                            .zip(items)
                            .map(|(h, v)| check_off_thread(set, h, v)),
                    )
                    .await;
                    for c in checked {
                        match c? {
                            Some(b) => out.push(b),
                            None => return Ok(out),
                        }
                    }
                    if out.len() as u64 == to - from + 1 {
                        return Ok(out);
                    }
                    // A byte-budget-limited response is only a prefix. Obtain
                    // its suffix too, so the descending walk can reach its anchor.
                    cursor = from + out.len() as u64;
                }
                Ok(other) if other.is_null() => return Ok(out),
                Ok(_) => return Err("a block range answer that is not a list".into()),
                Err(e) if unknown_method(&e) => {
                    note_no_range(key);
                    break;
                }
                Err(_) if !out.is_empty() => return Ok(out),
                Err(e) => return Err(e),
            }
        }
    }
    let singles = (from + out.len() as u64..=to).map(|h| async move {
        let v = {
            let _slot = slots.take(i).await;
            call_patient(up, i, "aether_getFinalized", &json!([h])).await?
        };
        check_off_thread(set, h, v).await
    });
    for r in futures::future::join_all(singles).await {
        match r {
            Ok(Some(b)) => out.push(b),
            Ok(None) => break,
            Err(e) if out.is_empty() => return Err(e),
            Err(_) => break,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_cover_the_range_in_order_without_overlap() {
        assert_eq!(spans(1, 1), vec![(1, 1)]);
        assert_eq!(spans(1, 64), vec![(1, 32), (33, 64)]);
        assert_eq!(spans(10, 50), vec![(10, 41), (42, 50)]);
        assert!(spans(5, 4).is_empty());
    }

    #[test]
    fn busy_waits_follow_the_hint_else_back_off() {
        let hinted = format!("{}; retry_after_ms=32", aether_net::BUSY);
        assert_eq!(
            busy_wait(&hinted, 5),
            Duration::from_millis(32),
            "the server's hint wins"
        );
        let oversized = format!("{}; retry_after_ms={}", aether_net::BUSY, u64::MAX);
        assert_eq!(
            busy_wait(&oversized, 0),
            BUSY_MAX_WAIT,
            "peer hints cannot overflow the sleep deadline or hold catch-up indefinitely"
        );
        let old = aether_net::BUSY;
        assert_eq!(busy_wait(old, 0), BUSY_FIRST_WAIT);
        assert_eq!(busy_wait(old, 1), BUSY_FIRST_WAIT * 2);
        assert_eq!(busy_wait(old, 30), BUSY_MAX_WAIT, "bounded");
        assert!(is_busy(old) && !is_busy("method not found: x"));
    }

    #[test]
    fn an_unknown_method_marks_the_source_as_singles_only() {
        assert!(unknown_method(
            "{\"code\":-32601,\"message\":\"method not found: aether_getFinalizedRange\"}"
        ));
        assert!(!unknown_method("server busy"));
        let key = "http://spread-test.invalid".to_string();
        assert!(range_supported(&key));
        note_no_range(key.clone());
        assert!(!range_supported(&key));
    }
}
