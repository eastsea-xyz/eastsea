//! The read-only view behind the RPC method `aether_rewardStatus`
//! (docs/design/15-node-rewards.md): anyone can check how many operators
//! shared the last epoch's node pool — so how much the 1/16 cap held back —
//! and where one operator's Macs stand. The app's rewards card reads this.
//!
//! Weights come from the same pure functions `distribute` runs
//! (`rewards::operator_weights`, `rewards::apply_reserve_credit`,
//! `rewards::share`), so the count and the expected share cannot disagree
//! with what the chain pays.
//!
//! A Mac's beacon record is one word per epoch that the next epoch's answers
//! overwrite, so the view pins one snapshot per epoch: the first finalized
//! height at which the epoch is observed. That is usually the distribution
//! block itself, and always before the new epoch's answers start landing
//! (a slot's hash is recorded one block after its block, and answers come
//! later still), so what the pin sees is what `distribute` counted. Later
//! calls in the same epoch are served the pin instead of recounting a state
//! that has already moved on. Two things can still make a late first
//! observation differ from the distribution itself (a restart, or nobody
//! asking before the Macs moved on): a Mac that answered the new epoch reads
//! as 0 for the old one — its record is gone — and on the epoch that ends a
//! warm-up day the levels have already taken their step. The founder's
//! reserve credit drifts the same way: it is counted through the reserve
//! predicate, so an epoch first observed after every Mac moved on can show
//! it where the distribution paid none. The first epoch of
//! a chain, still being answered, is counted live instead of pinned.
//!
//! A network whose genesis did not turn node rewards on (testnet 7780)
//! answers `{"enabled": false}` and nothing else.

use aether_execution::registry;
use aether_execution::WorldState;
use aether_rewards as rewards;
use aether_rewards::beacons;
use aether_types::{Address, U256};
use alloy_primitives::B256;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// One epoch's standing, pinned at its first sight (see the module doc): the
/// weights `distribute` used, and per candidate the slots it answered (`None`
/// when no usable record remains — the next epoch's answers replaced it, or
/// the Mac registered after the snapshot).
#[derive(Clone)]
struct Pinned {
    /// Which chain and which epoch the snapshot describes.
    chain: u64,
    epoch: u64,
    /// State root of the epoch's distribution block, when this node kept it.
    root: Option<B256>,
    weights: BTreeMap<Address, u64>,
    answered: BTreeMap<u64, Option<u64>>,
}

/// Recent pins, most recent first. One node serves one chain, so a couple of
/// slots cover an epoch boundary's stragglers — and tests that run in
/// parallel, each on its own chain, keep their own pin.
static PINNED: Mutex<Vec<Pinned>> = Mutex::new(Vec::new());
/// How many pins to keep before the oldest falls off.
const PINS_KEPT: usize = 16;

/// The operator weights of epoch `epoch` as `distribute` computes them from
/// this state. Public so the equality with real distributions stays tested
/// and anyone auditing N can run the same count.
pub fn epoch_weights(state: &WorldState, epoch: u64) -> BTreeMap<Address, u64> {
    scan(state, epoch).0
}

/// Weights and per-candidate answered-slot counts of `epoch` from this state.
fn scan(state: &WorldState, epoch: u64) -> (BTreeMap<Address, u64>, BTreeMap<u64, Option<u64>>) {
    let candidates = registry::candidates(state);
    let macs: Vec<rewards::Mac> = candidates.iter().map(|c| rewards::mac(state, c.index)).collect();
    let records: Vec<beacons::Beacon> = candidates.iter().map(|c| beacons::beacon(state, c.index)).collect();
    let mut answered: Vec<u64> = records.iter().map(|b| b.answered(epoch)).collect();
    // What to show per Mac: its count while the record still speaks of `epoch`
    // (an older record answers 0); `None` once a newer epoch's answers ate it.
    let shown: BTreeMap<_, _> = candidates
        .iter()
        .zip(records.iter())
        .map(|(c, b)| (c.index, (b.epoch <= epoch).then(|| b.answered(epoch))))
        .collect();
    // The founder's reserve credit counts for the weights exactly as
    // `distribute` paid it; the slots shown per Mac stay its own answers.
    rewards::apply_reserve_credit(state, epoch, &candidates, &mut answered);
    (rewards::operator_weights(&candidates, &macs, &answered), shown)
}

/// The pinned snapshot of the epoch, computed on the first call that sees it.
fn pinned(chain: u64, state: &WorldState, epoch: u64, root: Option<B256>) -> Pinned {
    let mut guard = PINNED.lock().expect("reward status cache");
    if let Some(p) = guard.iter().find(|p| p.chain == chain && p.epoch == epoch && p.root == root) {
        return p.clone();
    }
    let (weights, answered) = scan(state, epoch);
    let fresh = Pinned { chain, epoch, root, weights, answered };
    guard.insert(0, fresh.clone());
    guard.truncate(PINS_KEPT);
    fresh
}

/// The result of the RPC method `aether_rewardStatus [operator?]`: the
/// finalized chain's node-rewards standing (see the module doc). `root` is
/// the state root of the last epoch's distribution block when this node kept
/// it, and `received` what that distribution actually paid `operator`, when
/// this node knows.
pub fn status(
    chain: u64,
    state: &WorldState,
    height: u64,
    root: Option<B256>,
    operator: Option<Address>,
    received: Option<U256>,
) -> Value {
    if !rewards::enabled(state) {
        return json!({ "enabled": false });
    }
    let epoch_blocks = registry::epoch_blocks(state);
    let epoch = height / epoch_blocks;
    let last = epoch.saturating_sub(1);
    // The first epoch is still being answered, nothing was distributed yet:
    // count it live instead of pinning a partial first sight.
    let p = if epoch == 0 {
        let (weights, answered) = scan(state, 0);
        Pinned { chain, epoch: 0, root, weights, answered }
    } else {
        pinned(chain, state, last, root)
    };
    let pool = rewards::node_pool(last, epoch_blocks);
    let total: u64 = p.weights.values().sum();
    let mut v = json!({
        "enabled": true,
        "height": height,
        "epoch": epoch,
        "last_epoch": last,
        "epoch_blocks": epoch_blocks,
        "slots_per_epoch": rewards::SLOTS,
        // N: the operators that shared the last epoch's pool, counted exactly
        // as `distribute` counts them (the shared `operator_weights`).
        "operators_online_last_epoch": p.weights.len(),
        "max_share": rewards::MAX_SHARE,
        "node_pool_last_epoch": pool.to_string(),
        "issuance_per_block_now": rewards::issuance(height).to_string(),
    });
    if let Some(op) = operator {
        v["operator"] = operator_view(state, epoch, &p, pool, total, op, received);
    }
    v
}

/// One operator's standing: its Macs live, its share of the last epoch's pool
/// from the pin. `capped` says the 1/16 limit is what held the share down —
/// a proportional split of the weights would have paid more.
fn operator_view(
    state: &WorldState,
    epoch: u64,
    p: &Pinned,
    pool: U256,
    total: u64,
    op: Address,
    received: Option<U256>,
) -> Value {
    // Answers count only while the Mac's re-attestation covers the period of
    // the epoch's last slot — the rest of the epoch, as `beacons::check` sees it.
    let period = beacons::period(state, epoch, rewards::SLOTS - 1);
    let macs: Vec<Value> = registry::candidates(state)
        .into_iter()
        .filter(|c| c.operator == op)
        .map(|c| {
            let b = beacons::beacon(state, c.index);
            let m = rewards::mac(state, c.index);
            json!({
                "index": c.index,
                "answered_slots_this_epoch": b.answered(epoch),
                "answered_slots_last_epoch": p.answered.get(&c.index).copied().flatten(),
                "warmup_level": m.level,
                // "정상 몫의 몇 %" (docs/design/15): (14 + level) / 28, integer percent.
                "warmup_percent": m.warmup() * 100 / (2 * rewards::WARMUP_STEPS),
                "attested_period": b.attested,
                "reattest_ok": b.attested == Some(period)
                    || period <= c.registered_epoch / rewards::DAY_EPOCHS + 1,
            })
        })
        .collect();
    let w = p.weights.get(&op).copied().unwrap_or_default();
    json!({
        "address": op,
        "macs": macs,
        "weight_last_epoch": w,
        "expected_share_last_epoch": rewards::share(pool, w, total).to_string(),
        // What the last distribution actually credited. Null: this address was
        // paid nothing at it, or this node keeps no reward records.
        "received_last_distribution": received.map(|r| r.to_string()),
        "capped": w * rewards::MAX_SHARE > total,
    })
}

/// What the distribution block at `distribution_height` paid, from a node's
/// newest reward records (`aether_rewards`): the newest node-kind record,
/// when it is that distribution's — an older newest node record means the
/// last distribution paid this address nothing.
pub fn received_from_records(records: &[Value], distribution_height: u64) -> Option<U256> {
    let newest_node = records.iter().rev().find(|r| r.get("kind").and_then(Value::as_str) == Some("node"))?;
    if newest_node.get("height").and_then(Value::as_u64) != Some(distribution_height) {
        return None;
    }
    newest_node
        .get("amount")
        .and_then(Value::as_str)
        .and_then(|a| U256::from_str_radix(a, 10).ok())
}
