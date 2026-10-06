//! Why a transaction is not in a block (contracts-live bug #5, 2026-10-06).
//!
//! A wallet that got a hash must always learn what became of it. A pending
//! transaction says what it waits for; one that left the mempool without being
//! included leaves a tombstone with the reason. Tombstones are node-local
//! memory: not consensus, not persisted, bounded to `TOMBSTONES` entries and
//! evicted least recently used.

use aether_types::TxHash;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Most tombstones one node remembers. At the 50,000-tx pool cap a full
/// eviction wave outruns it, but a wallet asks within seconds of a drop.
pub const TOMBSTONES: usize = 4096;

/// Why a transaction waits, or why it left the pool without being included.
/// Serialized as `{"kind": "...", ...}`; amounts are decimal wei strings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// The B5 state price is above the transaction's signed state cap. While
    /// it waits, `blocks` estimates how many blocks the refill needs to bring
    /// the price to the cap, if no more state is used (None: never).
    StatePriceAboveCap {
        cap: String,
        price: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        blocks: Option<u64>,
    },
    /// The exec or prove base fee is above the transaction's caps.
    FeeCapBelowBase,
    /// An earlier nonce of the same sender is missing; `expected` is the
    /// first nonce the chain or the pool still needs.
    NonceGap { expected: u64 },
    /// It waited the whole mempool TTL for no single reason above.
    Expired,
    /// Another transaction with the same sender and nonce was included.
    Replaced,
    /// A higher-paying transaction took its place in a full pool.
    Evicted,
    /// The sender's balance no longer covers it.
    Unaffordable,
}

impl Reason {
    /// Whether signing it again with the same nonce and a fresh fee can
    /// succeed (a replaced or unaffordable one cannot).
    pub fn resendable(&self) -> bool {
        !matches!(self, Reason::Replaced | Reason::Unaffordable)
    }
}

/// `hash → reason` for transactions that left the pool without a block.
#[derive(Debug, Default)]
pub struct Tombstones {
    by_hash: HashMap<TxHash, (Reason, u64)>,
    order: BTreeMap<u64, TxHash>,
    tick: u64,
}

impl Tombstones {
    fn touch(&mut self, h: TxHash, reason: Reason) {
        self.tick += 1;
        if let Some((_, old)) = self.by_hash.insert(h, (reason, self.tick)) {
            self.order.remove(&old);
        }
        self.order.insert(self.tick, h);
    }

    /// Remember why `h` left; the least recently used entry goes past the cap.
    pub fn record(&mut self, h: TxHash, reason: Reason) {
        self.touch(h, reason);
        while self.by_hash.len() > TOMBSTONES {
            let Some((_, old)) = self.order.pop_first() else { break };
            self.by_hash.remove(&old);
        }
    }

    /// The reason `h` left, if remembered; a lookup counts as a use.
    pub fn get(&mut self, h: &TxHash) -> Option<Reason> {
        let reason = self.by_hash.get(h)?.0.clone();
        self.touch(*h, reason.clone());
        Some(reason)
    }

    /// `h` is back in the pool (resubmitted) or included: no tombstone.
    pub fn forget(&mut self, h: &TxHash) {
        if let Some((_, t)) = self.by_hash.remove(h) {
            self.order.remove(&t);
        }
    }

    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(i: u64) -> TxHash {
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&i.to_be_bytes());
        TxHash::from(b)
    }

    #[test]
    fn tombstones_are_bounded_and_least_recently_used_goes_first() {
        let mut t = Tombstones::default();
        for i in 0..TOMBSTONES as u64 {
            t.record(h(i), Reason::Expired);
        }
        assert_eq!(t.len(), TOMBSTONES);
        // A wallet asked about the oldest one: it is now the most recent.
        assert_eq!(t.get(&h(0)), Some(Reason::Expired));
        t.record(h(TOMBSTONES as u64), Reason::Evicted);
        assert_eq!(t.len(), TOMBSTONES, "the map never grows past its cap");
        assert_eq!(t.get(&h(1)), None, "the least recently used entry left");
        assert_eq!(t.get(&h(0)), Some(Reason::Expired), "the one just read stayed");
        assert_eq!(t.get(&h(TOMBSTONES as u64)), Some(Reason::Evicted));
        t.forget(&h(0));
        assert_eq!(t.get(&h(0)), None);
        assert_eq!(t.len(), TOMBSTONES - 1);
    }

    #[test]
    fn reasons_serialize_for_wallets() {
        let r = Reason::StatePriceAboveCap { cap: "2".into(), price: "43".into(), blocks: Some(1_200) };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({ "kind": "state_price_above_cap", "cap": "2", "price": "43", "blocks": 1_200 })
        );
        let dropped = Reason::StatePriceAboveCap { cap: "2".into(), price: "43".into(), blocks: None };
        assert!(serde_json::to_value(&dropped).unwrap().get("blocks").is_none());
        assert_eq!(serde_json::to_value(Reason::NonceGap { expected: 7 }).unwrap(), serde_json::json!({ "kind": "nonce_gap", "expected": 7 }));
        assert!(Reason::Expired.resendable() && !Reason::Replaced.resendable());
    }
}
