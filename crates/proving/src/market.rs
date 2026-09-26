//! Prover escrow payouts (docs/research/tokenomics-2026.md R3).
//!
//! Each finalized block puts its prove fees and 20% of its tips in escrow.
//! Proofs trail the chain; whoever first proves a stretch of it (a chunk) is
//! paid that stretch's share of the escrow, proportional to prove gas:
//!
//! - First valid proof wins. A later proof that overlaps proven work is paid
//!   only for the part nobody proved yet (a checkpoint proof never supersedes).
//! - Payouts accrue to `claimable` and are withdrawn later (pull, not push).
//! - Escrow left unproven for `EXPIRY` blocks is burned, never refunded to the
//!   proposer (a refund would pay proposers to obstruct proving).
//!
//! Positions are counted in *units*: a block's transactions, or one unit for a
//! block with none (empty blocks still need proving for the chain to chain).
//! The caller verifies the proof (`Verifier`) before `submit`; this module
//! only does the accounting, deterministically, so every validator agrees.

use aether_types::{Address, Hash, U256};
use std::collections::{BTreeMap, HashMap};

/// Unproven escrow is burned after ~30 days of 1 s blocks.
pub const EXPIRY: u64 = 2_592_000;

/// A position in the chain: unit `unit` of block `height`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub height: u64,
    pub unit: u32,
}

/// A proven stretch `[from, to)`; `to` may point one past a block's last unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub from: Pos,
    pub to: Pos,
}

/// What a chunk proof commits to (hashed into its public input).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkCommitment {
    pub chain_id: u64,
    pub span: Span,
    pub pre_state_root: Hash,
    pub post_state_root: Hash,
    pub prove_gas: u64,
    /// Guest program the proof is for (its image id / hash).
    pub program: Hash,
}

impl ChunkCommitment {
    pub fn hash(&self) -> Hash {
        let mut h = blake3::Hasher::new();
        h.update(b"aether/chunk/v1");
        h.update(&self.chain_id.to_be_bytes());
        for p in [self.span.from, self.span.to] {
            h.update(&p.height.to_be_bytes());
            h.update(&p.unit.to_be_bytes());
        }
        h.update(self.pre_state_root.as_slice());
        h.update(self.post_state_root.as_slice());
        h.update(&self.prove_gas.to_be_bytes());
        h.update(self.program.as_slice());
        Hash::from(*h.finalize().as_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketError {
    EmptySpan,
    /// The span reaches a block the market does not hold (not finalized, or expired).
    UnknownBlock(u64),
    /// Everything in the span was already proven.
    AlreadyProven,
}

#[derive(Clone, Debug)]
struct BlockEscrow {
    amount: U256,
    paid: U256,
    /// Prove-gas weight per unit (1 each when the block used no prove gas).
    weights: Vec<u64>,
    total: u64,
    proven: Vec<bool>,
}

impl BlockEscrow {
    fn new(amount: U256, prove_gas_per_tx: &[u64]) -> Self {
        let weights: Vec<u64> = if prove_gas_per_tx.iter().any(|&g| g > 0) { prove_gas_per_tx.to_vec() } else { vec![1; prove_gas_per_tx.len().max(1)] };
        let total = weights.iter().sum();
        BlockEscrow { amount, paid: U256::ZERO, proven: vec![false; weights.len()], weights, total }
    }

    fn units(&self) -> u32 {
        self.weights.len() as u32
    }
}

/// Escrow ledger and prover balances (deterministic; part of chain state).
#[derive(Clone, Debug, Default)]
pub struct ProofMarket {
    blocks: BTreeMap<u64, BlockEscrow>,
    claimable: HashMap<Address, U256>,
    pub burned: U256,
}

impl ProofMarket {
    /// A block was finalized: `amount` went to escrow; `prove_gas_per_tx` from its receipts.
    pub fn record_block(&mut self, height: u64, amount: U256, prove_gas_per_tx: &[u64]) {
        self.blocks.insert(height, BlockEscrow::new(amount, prove_gas_per_tx));
    }

    /// Units in block `height` (for building spans), if held.
    pub fn units(&self, height: u64) -> Option<u32> {
        self.blocks.get(&height).map(BlockEscrow::units)
    }

    /// Credit `prover` for the unproven part of `span` (its proof already verified).
    pub fn submit(&mut self, span: Span, prover: Address) -> Result<U256, MarketError> {
        if span.to <= span.from {
            return Err(MarketError::EmptySpan);
        }
        for h in span.from.height..=span.to.height {
            if !self.blocks.contains_key(&h) {
                return Err(MarketError::UnknownBlock(h));
            }
        }
        let mut payout = U256::ZERO;
        for h in span.from.height..=span.to.height {
            let b = self.blocks.get_mut(&h).expect("checked above");
            let start = if h == span.from.height { span.from.unit.min(b.units()) } else { 0 };
            let end = if h == span.to.height { span.to.unit.min(b.units()) } else { b.units() };
            let mut share = U256::ZERO;
            for k in start..end {
                let k = k as usize;
                if b.proven[k] {
                    continue;
                }
                b.proven[k] = true;
                share += b.amount * U256::from(b.weights[k]) / U256::from(b.total.max(1));
            }
            // Whoever completes a block also gets its rounding dust: payouts sum to the escrow.
            if b.proven.iter().all(|&p| p) {
                share = b.amount - b.paid;
            }
            b.paid += share;
            payout += share;
        }
        if payout.is_zero() && self.span_all_proven(span) {
            return Err(MarketError::AlreadyProven);
        }
        *self.claimable.entry(prover).or_default() += payout;
        Ok(payout)
    }

    fn span_all_proven(&self, span: Span) -> bool {
        (span.from.height..=span.to.height).all(|h| {
            let b = &self.blocks[&h];
            let start = if h == span.from.height { span.from.unit.min(b.units()) } else { 0 };
            let end = if h == span.to.height { span.to.unit.min(b.units()) } else { b.units() };
            (start..end).all(|k| b.proven[k as usize])
        })
    }

    /// What `prover` can withdraw now.
    pub fn claimable(&self, prover: &Address) -> U256 {
        self.claimable.get(prover).copied().unwrap_or_default()
    }

    /// Withdraw everything `prover` earned (moved from the escrow account by the caller).
    pub fn claim(&mut self, prover: &Address) -> U256 {
        self.claimable.remove(prover).unwrap_or_default()
    }

    /// Burn what stayed unproven for `EXPIRY` blocks; drop fully paid blocks.
    /// Returns the amount burned now.
    pub fn expire(&mut self, current_height: u64) -> U256 {
        let mut burned = U256::ZERO;
        self.blocks.retain(|&h, b| {
            let done = b.paid == b.amount;
            if !done && h + EXPIRY <= current_height {
                burned += b.amount - b.paid;
                return false;
            }
            !done
        });
        self.burned += burned;
        burned
    }

    /// Escrow still owed to future proofs.
    pub fn outstanding(&self) -> U256 {
        self.blocks.values().map(|b| b.amount - b.paid).fold(U256::ZERO, |a, b| a + b)
    }

    /// Unproven prove gas (the backlog a v1 fee controller would price in).
    pub fn backlog(&self) -> u64 {
        self.blocks.values().flat_map(|b| b.weights.iter().zip(&b.proven).filter(|(_, &p)| !p).map(|(w, _)| *w)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Address = Address::repeat_byte(0xa1);
    const B: Address = Address::repeat_byte(0xb2);

    fn pos(height: u64, unit: u32) -> Pos {
        Pos { height, unit }
    }

    fn span(h1: u64, u1: u32, h2: u64, u2: u32) -> Span {
        Span { from: pos(h1, u1), to: pos(h2, u2) }
    }

    fn market() -> ProofMarket {
        let mut m = ProofMarket::default();
        m.record_block(1, U256::from(1_000u64), &[100, 300, 600]);
        m.record_block(2, U256::from(7u64), &[]); // empty block
        m.record_block(3, U256::from(10u64), &[0, 0, 0]); // no prove gas: equal weights
        m
    }

    #[test]
    fn whole_range_pays_the_whole_escrow() {
        let mut m = market();
        let paid = m.submit(span(1, 0, 3, 3), A).unwrap();
        assert_eq!(paid, U256::from(1_017u64));
        assert_eq!(m.claimable(&A), paid);
        assert_eq!(m.outstanding(), U256::ZERO);
        assert_eq!(m.claim(&A), paid);
        assert_eq!(m.claimable(&A), U256::ZERO);
    }

    #[test]
    fn chunks_are_paid_by_prove_gas_and_sum_to_the_escrow() {
        let mut m = market();
        assert_eq!(m.submit(span(1, 0, 1, 1), A).unwrap(), U256::from(100u64));
        assert_eq!(m.submit(span(1, 1, 1, 3), B).unwrap(), U256::from(900u64));
        // Equal weights with rounding: the prover completing the block gets the dust.
        assert_eq!(m.submit(span(3, 0, 3, 2), A).unwrap(), U256::from(6u64));
        assert_eq!(m.submit(span(3, 2, 3, 3), B).unwrap(), U256::from(4u64));
        assert_eq!(m.claimable(&A) + m.claimable(&B), U256::from(1_010u64));
    }

    #[test]
    fn first_valid_proof_wins_and_checkpoints_get_only_the_rest() {
        let mut m = market();
        m.submit(span(1, 1, 1, 2), A).unwrap(); // 300
        assert_eq!(m.submit(span(1, 1, 1, 2), B), Err(MarketError::AlreadyProven));
        // A checkpoint over everything pays B only what A did not prove.
        assert_eq!(m.submit(span(1, 0, 3, 3), B).unwrap(), U256::from(1_017u64 - 300));
        assert_eq!(m.claimable(&A), U256::from(300u64));
        assert_eq!(m.submit(span(1, 0, 3, 3), A), Err(MarketError::AlreadyProven));
    }

    #[test]
    fn empty_blocks_are_one_unit() {
        let mut m = market();
        assert_eq!(m.units(2), Some(1));
        assert_eq!(m.submit(span(2, 0, 2, 1), A).unwrap(), U256::from(7u64));
    }

    #[test]
    fn unknown_or_empty_spans_are_rejected() {
        let mut m = market();
        assert_eq!(m.submit(span(1, 2, 1, 2), A), Err(MarketError::EmptySpan));
        assert_eq!(m.submit(span(3, 0, 4, 1), A), Err(MarketError::UnknownBlock(4)));
        assert_eq!(m.outstanding(), U256::from(1_017u64), "a rejected span changes nothing");
    }

    #[test]
    fn unproven_escrow_burns_after_expiry_and_is_not_refunded() {
        let mut m = market();
        m.submit(span(1, 0, 1, 1), A).unwrap(); // 100 of block 1 paid
        assert_eq!(m.backlog(), 300 + 600 + 1 + 3);
        assert_eq!(m.expire(EXPIRY), U256::ZERO, "not expired yet");
        assert_eq!(m.expire(EXPIRY + 3), U256::from(900u64 + 7 + 10));
        assert_eq!(m.burned, U256::from(917u64));
        assert_eq!(m.outstanding(), U256::ZERO);
        assert_eq!(m.submit(span(1, 1, 1, 3), B), Err(MarketError::UnknownBlock(1)));
        assert_eq!(m.claimable(&A), U256::from(100u64), "earned payouts survive expiry");
    }

    #[test]
    fn commitment_binds_every_field() {
        let c = ChunkCommitment {
            chain_id: 1,
            span: span(1, 0, 1, 3),
            pre_state_root: Hash::ZERO,
            post_state_root: Hash::repeat_byte(1),
            prove_gas: 1_000,
            program: Hash::repeat_byte(9),
        };
        let mut d = c.clone();
        d.span.to.unit = 2;
        assert_ne!(c.hash(), d.hash());
        let mut e = c.clone();
        e.program = Hash::repeat_byte(8);
        assert_ne!(c.hash(), e.hash());
        assert_eq!(c.hash(), c.clone().hash());
    }
}
