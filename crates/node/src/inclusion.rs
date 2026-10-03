//! FOCIL-style inclusion lists (docs/design/07-consensus.md, D12 stage 1).
//!
//! Each height, a small committee of validators signs the oldest txs waiting in
//! its mempool and gossips them. A proposer puts every listed tx it holds first.
//! A voter refuses to notarize a block that leaves out a listed tx which would
//! still have been valid appended to the end of that block (the "append check"),
//! so one censoring proposer cannot keep a tx out: its blocks miss the quorum
//! and the view passes to the next leader.
//!
//! Enforcement is a voting rule, not a validity rule: finalized blocks are
//! never re-judged against inclusion lists (backfill, replay). Voters only
//! enforce lists they have held for `FREEZE`, so a list that reached them has
//! had time to reach the proposer too (lists are re-gossiped once on receipt).

use aether_execution::{can_append, tx_hash, validate_stateless, BlockContext, WorldState};
use aether_types::{GasVector, TxEnvelope, TxHash};
use commonware_codec::{DecodeExt, Encode};
use commonware_cryptography::{ed25519, Signer as _, Verifier as _};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const IL_NAMESPACE: &[u8] = b"_AETHER_DEVNET_V1_INCLUSION";
pub const COMMITTEE_SIZE: u64 = 8;
pub const MAX_IL_TXS: usize = 16;
/// Lists younger than this are not enforced by voters. Long enough that a
/// listed tx has reached the proposer before it builds, even when a loaded
/// machine delays gossip and verification (at 750 ms, testnet validators on
/// one busy Mac refused about one proposal a minute for a tx the proposer
/// had not yet seen, and each refusal cost a view timeout; 2026-09-29).
pub const FREEZE: Duration = Duration::from_secs(3);
const MAX_AGE: Duration = Duration::from_secs(120);
/// Listed txs this pool holds at most. The mempool's eviction rule spares
/// every listed tx it can (R2-6): what a list names is this node's obligation.
pub const MAX_POOL: usize = 4096;

/// A committee member's signed list for `height`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InclusionList {
    pub height: u64,
    /// 1-based validator index.
    pub member: u64,
    pub txs: Vec<TxEnvelope>,
    /// ed25519 signature (codec bytes, hex) over `signing_message`.
    pub signature: String,
}

fn signing_message(height: u64, member: u64, txs: &[TxEnvelope]) -> Vec<u8> {
    let mut m = Vec::with_capacity(16 + 32 * txs.len());
    m.extend_from_slice(&height.to_be_bytes());
    m.extend_from_slice(&member.to_be_bytes());
    for t in txs {
        m.extend_from_slice(tx_hash(t).as_slice());
    }
    m
}

/// Members for `height` among validators `1..=n`: everyone when `n` is at most
/// the committee size, otherwise a window rotating by height.
pub fn committee(height: u64, n: u64) -> Vec<u64> {
    if n <= COMMITTEE_SIZE {
        return (1..=n).collect();
    }
    let start = (height * COMMITTEE_SIZE) % n;
    (0..COMMITTEE_SIZE).map(|k| (start + k) % n + 1).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum IlError {
    NotMember,
    TooManyTxs,
    BadSignature,
    InvalidTx,
}

impl InclusionList {
    pub fn sign(key: &ed25519::PrivateKey, member: u64, height: u64, txs: Vec<TxEnvelope>) -> Self {
        let sig = key.sign(IL_NAMESPACE, &signing_message(height, member, &txs));
        InclusionList { height, member, txs, signature: hex::encode(sig.encode()) }
    }

    /// `validators[i]` is the key of validator `i + 1`.
    pub fn verify(&self, validators: &[ed25519::PublicKey], chain_id: u64) -> Result<(), IlError> {
        let n = validators.len() as u64;
        if self.member == 0 || self.member > n || !committee(self.height, n).contains(&self.member) {
            return Err(IlError::NotMember);
        }
        if self.txs.len() > MAX_IL_TXS {
            return Err(IlError::TooManyTxs);
        }
        let bytes = hex::decode(&self.signature).map_err(|_| IlError::BadSignature)?;
        let sig = ed25519::Signature::decode(bytes.as_slice()).map_err(|_| IlError::BadSignature)?;
        let pk = &validators[(self.member - 1) as usize];
        if !pk.verify(IL_NAMESPACE, &signing_message(self.height, self.member, &self.txs), &sig) {
            return Err(IlError::BadSignature);
        }
        if self.txs.iter().any(|t| validate_stateless(t, chain_id).is_err()) {
            return Err(IlError::InvalidTx);
        }
        Ok(())
    }
}

struct Entry {
    tx: TxEnvelope,
    first_seen: Instant,
}

/// Listed txs this node holds, with when it first saw each listed.
#[derive(Default)]
pub struct InclusionPool {
    entries: HashMap<TxHash, Entry>,
    seen: HashSet<(u64, u64)>,
}

impl InclusionPool {
    /// Adds a verified list. Returns false if this (member, height) was already seen.
    pub fn accept(&mut self, il: &InclusionList, now: Instant) -> bool {
        if !self.seen.insert((il.member, il.height)) {
            return false;
        }
        for tx in &il.txs {
            if self.entries.len() >= MAX_POOL {
                break;
            }
            self.entries.entry(tx_hash(tx)).or_insert_with(|| Entry { tx: tx.clone(), first_seen: now });
        }
        true
    }

    /// Everything listed, lowest nonce first per sender: what a proposer includes first.
    pub fn for_proposal(&self) -> Vec<TxEnvelope> {
        let mut txs: Vec<TxEnvelope> = self.entries.values().map(|e| e.tx.clone()).collect();
        txs.sort_by_key(|t| (t.header.nonce, t.header.sender, tx_hash(t)));
        txs
    }

    /// Listed txs a voter enforces: held for at least `FREEZE`.
    pub fn enforceable(&self, now: Instant) -> Vec<TxEnvelope> {
        let mut txs: Vec<TxEnvelope> = self.entries.values().filter(|e| now.saturating_duration_since(e.first_seen) >= FREEZE).map(|e| e.tx.clone()).collect();
        txs.sort_by_key(|t| (t.header.nonce, t.header.sender, tx_hash(t)));
        txs
    }

    /// Drop txs the finalized state has consumed (nonce passed) and stale entries.
    pub fn prune(&mut self, finalized: &WorldState, finalized_height: u64, now: Instant) {
        self.entries.retain(|_, e| e.tx.header.nonce >= finalized.nonce(&e.tx.header.sender) && now.saturating_duration_since(e.first_seen) < MAX_AGE);
        self.seen.retain(|(_, h)| *h + 64 > finalized_height);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether `h` is named by a held list: a tx an inclusion list names is
    /// this node's obligation to propose, so mempool eviction spares it
    /// (R2-6) whatever its fee.
    pub fn contains(&self, h: &TxHash) -> bool {
        self.entries.contains_key(h)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Listed txs a block wrongly left out: not in the block and still appendable
/// to its post-state within its remaining gas. Empty when the block is full.
pub fn violations(listed: &[TxEnvelope], block_txs: &[TxHash], block_full: bool, post: &WorldState, ctx: &BlockContext, used: GasVector) -> Vec<TxHash> {
    if block_full {
        return Vec::new();
    }
    let present: HashSet<&TxHash> = block_txs.iter().collect();
    listed.iter().filter(|t| !present.contains(&tx_hash(t)) && can_append(post, ctx, used, t)).map(tx_hash).collect()
}
