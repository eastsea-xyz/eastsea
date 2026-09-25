//! Optimistic parallel execution (docs/design/04-execution.md).
//!
//! 1. Every tx runs speculatively, in parallel, against the block's pre-state.
//! 2. Txs are committed in block order. A speculative result is reused only
//!    if nothing it read was changed by an earlier tx of the block; otherwise
//!    (or if it failed) the tx is re-executed on the current state.
//!
//! Signature checks do not depend on state, so all of them run in parallel up
//! front and re-executions reuse the result.
//!
//! The fee recipient is written by every tx. Its credit is applied as a delta
//! on the current balance, which is exact unless the tx looked at the
//! recipient in another way (BALANCE, EXTCODE*, calls, selfdestruct, sender or
//! recipient); such txs are re-executed. The result is therefore identical to
//! sequential execution, which the differential tests check.

use crate::block::{run_validated, BlockContext, TxRun};
use crate::tx::{validate_stateless, EvmCall};
use crate::world::WorldState;
use aether_types::{Address, TxEnvelope, U256};
use rayon::prelude::*;
use std::collections::HashSet;

/// Below this many txs speculation is not worth the thread hand-off.
const MIN_PARALLEL: usize = 4;

pub(crate) struct Scheduler<'a> {
    pre: &'a WorldState,
    beneficiary: Address,
    /// Decoded, signature-checked payload per tx.
    calls: Vec<Result<EvmCall, String>>,
    spec: Vec<Option<Result<TxRun, String>>>,
    /// Accounts whose balance/nonce/code/existence an earlier tx changed.
    dirty_accounts: HashSet<Address>,
    /// Storage slots an earlier tx changed.
    dirty_slots: HashSet<(Address, U256)>,
    pub(crate) reused: usize,
    pub(crate) reexecuted: usize,
}

impl<'a> Scheduler<'a> {
    pub(crate) fn new(pre: &'a WorldState, ctx: &BlockContext, txs: &[TxEnvelope], parallel: bool) -> Self {
        let check = |tx: &TxEnvelope| validate_stateless(tx, ctx.chain_id).map_err(|e| format!("{e:?}"));
        let (calls, spec) = if parallel && txs.len() >= MIN_PARALLEL {
            let calls: Vec<_> = txs.par_iter().map(check).collect();
            let spec = txs.par_iter().zip(calls.par_iter()).map(|(tx, c)| Some(c.clone().and_then(|c| run_validated(pre, ctx, tx, &c)))).collect();
            (calls, spec)
        } else {
            (txs.iter().map(check).collect(), txs.iter().map(|_| None).collect())
        };
        Scheduler { pre, calls, beneficiary: ctx.beneficiary, spec, dirty_accounts: HashSet::new(), dirty_slots: HashSet::new(), reused: 0, reexecuted: 0 }
    }

    /// Result of tx `i` on `state` (the state after txs `0..i` committed).
    pub(crate) fn run(&mut self, i: usize, state: &WorldState, ctx: &BlockContext, tx: &TxEnvelope) -> Result<TxRun, String> {
        if let Some(Some(Ok(run))) = self.spec.get_mut(i).map(Option::take) {
            if !run.touched_beneficiary && !self.conflicts(&run) {
                self.reused += 1;
                return Ok(self.rebase_fee(run, state));
            }
        }
        self.reexecuted += 1;
        let call = self.calls[i].clone()?;
        run_validated(state, ctx, tx, &call)
    }

    fn conflicts(&self, run: &TxRun) -> bool {
        run.changes
            .iter()
            .filter(|(a, _)| **a != self.beneficiary)
            .any(|(addr, acc)| self.dirty_accounts.contains(addr) || acc.storage.keys().any(|slot| self.dirty_slots.contains(&(*addr, *slot))))
    }

    /// Replace the speculative fee-recipient balance (pre + fee) by current + fee.
    fn rebase_fee(&self, mut run: TxRun, state: &WorldState) -> TxRun {
        if let Some(acc) = run.changes.get_mut(&self.beneficiary) {
            let fee = acc.info.balance.saturating_sub(self.pre.balance(&self.beneficiary));
            acc.info.balance = state.balance(&self.beneficiary) + fee;
            acc.info.nonce = state.nonce(&self.beneficiary);
        }
        run
    }

    /// Record what tx changes (call before committing them onto `state`).
    pub(crate) fn committing(&mut self, state: &WorldState, changes: &revm::state::EvmState) {
        for (addr, acc) in changes.iter() {
            if !acc.is_touched() || *addr == self.beneficiary {
                continue;
            }
            if acc.is_created()
                || acc.is_selfdestructed()
                || acc.is_empty()
                || acc.info.balance != state.balance(addr)
                || acc.info.nonce != state.nonce(addr)
                || acc.info.code_hash != state.code_hash(addr)
            {
                self.dirty_accounts.insert(*addr);
            }
            for (slot, v) in acc.storage.iter() {
                if v.is_changed() {
                    self.dirty_slots.insert((*addr, *slot));
                }
            }
        }
    }
}
