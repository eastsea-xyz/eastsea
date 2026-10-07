//! Backfill after a jump (the founder's rule, 2026-10-07: "새로 키면 무조건
//! 점핑부터 하고 그전 블록들을 역으로 따라잡아야"): a follower that was far
//! behind jumps FIRST to the network's certified snapshot, so it is at the tip
//! — balances verified, beacons current — within minutes. The gap's certified
//! blocks then come back in the background, newest to oldest, at low
//! priority:
//!
//! - each block is checked by its own committee certificate (`follow::check`)
//!   AND hash-linked to the already-trusted chain above it: the walk starts at
//!   the parent hash the jumped-to (certified) block names, and every block
//!   fetched must be exactly the parent the block above it names;
//! - it pauses whenever the follower itself is behind the tip (the tip first);
//! - it stops at the retention window (`prune::DEFAULT_RETAIN_DAYS` of 1 s
//!   blocks) or where the network itself pruned (the era files hold the rest);
//! - what it keeps is the gap's certified blocks and their certificates (the
//!   finality archive), so this node serves `aether_getFinalized` for them
//!   to other followers and wallets. It does not re-execute the gap: plain
//!   followers never replay it (account-activity rows of the gap stay with
//!   era replay), and archive nodes never jump at all (audit 7 A7-1).
//!
//! The plan persists in the store (`backfill` meta), so a restart resumes it;
//! `aether_status.backfill` reports it for the app ("이전 내역을 불러오는 중").

use crate::chain::Chain;
use crate::follow::{FinalityArchive, Upstream, BEHIND_MARGIN};
use crate::spread::{fetch_span, Slots, RANGE};
use aether_light::ValidatorSet;
use commonware_codec::Decode as _;
use commonware_cryptography::Digestible as _;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{info, warn};

/// Requests per source the backfill may have in flight: low priority, a
/// quarter of what following itself uses.
const PER_SOURCE: usize = 2;
/// Blocks kept behind the tip: the pruning design's retention window.
pub const RETAIN_BLOCKS: u64 = crate::prune::DEFAULT_RETAIN_DAYS * 86_400;
/// Store meta key of the persisted plan.
const META: &str = "backfill";
/// How long to wait after a round that fetched nothing before asking again.
const RETRY: Duration = Duration::from_secs(2);

/// Heights still to fill, inclusive, walked from `high` down to `low`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gap {
    pub low: u64,
    pub high: u64,
}

/// The running backfill's (low, high, next to fill), for `aether_status`.
static STATUS: Mutex<Option<(u64, u64, u64)>> = Mutex::new(None);

fn set_status(s: Option<(u64, u64, u64)>) {
    if let Ok(mut g) = STATUS.lock() {
        *g = s;
    }
}

/// What `aether_status` reports: null when no backfill is running (history
/// complete as far as this node keeps it), else the range and how much of
/// it is left — the app says "이전 내역을 불러오는 중" while it is not null.
pub fn status() -> Value {
    match STATUS.lock().ok().and_then(|g| *g) {
        Some((low, high, next)) => json!({
            "low": low,
            "high": high,
            "next": next,
            "remaining": (next + 1).saturating_sub(low),
        }),
        None => Value::Null,
    }
}

/// The lowest height a backfill below `high` goes to.
pub fn floor(high: u64) -> u64 {
    high.saturating_sub(RETAIN_BLOCKS).max(1)
}

/// The persisted plan, if one is unfinished.
pub fn stored(chain: &Chain) -> Option<Gap> {
    let bytes = chain.store()?.meta(META).ok()??;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let gap = Gap { low: v["low"].as_u64()?, high: v["high"].as_u64()? };
    (gap.low <= gap.high).then_some(gap)
}

fn persist(chain: &Chain, gap: Option<Gap>) {
    let Some(store) = chain.store() else { return };
    let bytes = gap.map_or_else(Vec::new, |g| json!({ "low": g.low, "high": g.high }).to_string().into_bytes());
    if let Err(e) = store.put_meta(META, &bytes) {
        warn!(?e, "could not keep the backfill plan");
    }
}

/// Plan the backfill of `low..=high` (the heights a jump skipped), merged
/// with an unfinished plan, bounded by the retention window. Persisted.
pub fn plan(chain: &Chain, low: u64, high: u64) -> Option<Gap> {
    let (mut low, mut high) = (low, high);
    if let Some(old) = stored(chain) {
        low = low.min(old.low);
        high = high.max(old.high);
    }
    let gap = Gap { low: low.max(floor(high)), high };
    let gap = (gap.low <= gap.high).then_some(gap);
    persist(chain, gap);
    gap
}

/// The hash of block `h` from what this node already trusts: its own block
/// summaries (the jumped-to block is one: its hash was checked against the
/// certified block after it, `Snapshot::check`), else a certificate it
/// archived. `None` when it knows neither. (A jumped-to summary carries no
/// parent hash — the snapshot does not know it — so the walk anchors on the
/// block's own hash and fetches it first.)
fn hash_of(chain: &Chain, archive: &FinalityArchive, h: u64) -> Option<String> {
    if let Some(s) = chain.lock().blocks.get(&h).filter(|s| !s.hash.is_empty()) {
        return Some(s.hash.clone());
    }
    let proof = archive.get(h)?;
    let bytes = aether_light::from_hex(proof["block"].as_str()?).ok()?;
    let block = crate::block::Block::decode_cfg(bytes.as_slice(), &crate::block::Block::codec_config(aether_light::MAX_BLOCK_BYTES)).ok()?;
    Some(format!("{}", block.digest()))
}

/// Fill `gap`, newest to oldest (see the module docs). Returns the lowest
/// height filled (`gap.high + 1` when nothing was).
pub async fn run(chain: Chain, upstream: Arc<Upstream>, set: ValidatorSet, archive: Arc<FinalityArchive>, gap: Gap) -> u64 {
    info!(low = gap.low, high = gap.high, "backfilling the gap's certified blocks, newest first");
    let slots = Slots::new(upstream.sources(), PER_SOURCE);
    // The walk starts AT the trusted block above the gap: the first block
    // fetched must hash to what this node already trusts there.
    let anchor = gap.high + 1;
    let mut expected = hash_of(&chain, &archive, anchor);
    let mut next = if expected.is_some() { anchor } else { gap.high };
    let mut k = 0usize;
    set_status(Some((gap.low, gap.high, next)));
    while next >= gap.low {
        // The tip first: while the follower is behind, the backfill waits.
        if chain.behind() > BEHIND_MARGIN {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let a = next.saturating_sub(RANGE - 1).max(gap.low);
        k += 1;
        let blocks = match fetch_span(&upstream, &set, &slots, k, a, next).await {
            Ok(b) if b.len() as u64 == next - a + 1 => b,
            Ok(_) => {
                tokio::time::sleep(RETRY).await;
                continue;
            }
            Err(e) if e.contains("pruned") => {
                info!(height = next, "the network pruned below here; the era files hold the rest");
                break;
            }
            Err(e) => {
                warn!(height = next, %e, "backfill");
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        // Newest first, each the exact parent the block above it names.
        for (block, proof) in blocks.into_iter().rev() {
            let h = block.height.get();
            let digest = format!("{}", block.digest());
            if expected.as_ref().is_some_and(|e| *e != digest) {
                tracing::error!(height = h, %digest, expected = ?expected, "a certified block does not link to the chain above it; backfill stopped");
                set_status(None);
                return h + 1;
            }
            archive.insert(h, proof);
            expected = Some(format!("{}", block.parent));
        }
        if a == gap.low {
            next = a.saturating_sub(1);
            break;
        }
        next = a - 1;
        set_status(Some((gap.low, gap.high, next)));
        persist(&chain, Some(Gap { low: gap.low, high: next }));
    }
    persist(&chain, None);
    set_status(None);
    info!(low = next + 1, high = gap.high, "backfill finished");
    next + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_is_the_retention_window() {
        assert_eq!(floor(10_000), 1, "a short chain backfills to its start");
        assert_eq!(floor(RETAIN_BLOCKS + 500), 500);
    }
}
