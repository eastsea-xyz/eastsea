//! Consensus application: propose / verify / report finalized blocks.
//! Pacing and timestamp rules adapted from alto-chain (MIT OR Apache-2.0).

use crate::block::{Block, Context};
use crate::chain::{build_payload, Chain, Executed, Extras};
use aether_light::Scheme;
use commonware_actor::Feedback;
use commonware_consensus::{
    marshal::{ancestry::Ancestry, Update},
    Application as ConsensusApplication, Heightable, Reporter,
};
use commonware_cryptography::Digestible;
use commonware_runtime::{Clock, Metrics, Spawner, Storage};
use commonware_utils::{Acknowledgement, SystemTimeExt};
use futures::StreamExt;
use rand::Rng;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

/// Fixed consensus cutoff for timestamps (2200-01-01) so validity never depends on platform limits.
const MAX_BLOCK_TIMESTAMP_MS: u64 = 7_258_118_400_000;
const MAX_FUTURE_SKEW_MS: u64 = 1_000;

#[derive(Clone)]
pub struct Application {
    chain: Chain,
    delay_ms: u64,
}

impl Application {
    pub fn new(chain: Chain, delay_ms: u64) -> Self {
        Self { chain, delay_ms }
    }

    /// Height of the finalized state this node already holds (restored from disk).
    pub fn finalized_height(&self) -> u64 {
        self.chain.finalized_height()
    }

    /// Re-execute an already finalized block during startup recovery.
    pub fn replay(&self, block: &Block) -> Result<(), crate::chain::ChainError> {
        self.chain.finalize(block)
    }

    /// Post-state of `first`, re-executing from the nearest known ancestor if needed
    /// (after a restart or when joining late). `rest` yields `first`'s ancestors.
    async fn resolve<A: Ancestry<Block>>(&self, first: Arc<Block>, mut rest: A) -> Option<Arc<Executed>> {
        let mut pending = vec![first];
        let mut base = loop {
            let top = pending.last().expect("non-empty");
            if let Some(e) = self.chain.get(&top.digest()) {
                pending.pop();
                break e;
            }
            match rest.next().await {
                Some(b) => pending.push(b),
                None => return None,
            }
        };
        while let Some(b) = pending.pop() {
            match self.chain.execute(&b, &base) {
                Ok(e) => base = e,
                Err(e) => {
                    warn!(height = %b.height(), ?e, "ancestor failed to re-execute");
                    return None;
                }
            }
        }
        Some(base)
    }
}

impl<E> ConsensusApplication<E> for Application
where
    E: Rng + Spawner + Metrics + Clock + Storage,
{
    type SigningScheme = Scheme;
    type Context = Context;
    type Block = Block;
    type Input = ();

    async fn propose(&mut self, (rt, context): (E, Self::Context), mut ancestry: impl Ancestry<Self::Block>, _input: ()) -> Option<Self::Block> {
        let parent_block = ancestry.next().await?;
        let parent = self.resolve(parent_block.clone(), ancestry).await?;
        if self.chain.retired_after(&parent) {
            // Handed over: the next voting set builds from the switch height.
            return None;
        }

        // Pace proposals from the parent's timestamp.
        let min_ts = parent_block.timestamp.checked_add(self.delay_ms)?;
        let mut now = rt.current().epoch_millis();
        if now < min_ts {
            rt.sleep_until(SystemTime::UNIX_EPOCH + Duration::from_millis(min_ts)).await;
            now = rt.current().epoch_millis();
        }
        let ts = now.max(min_ts);
        if ts > MAX_BLOCK_TIMESTAMP_MS {
            return None;
        }

        let height = parent_block.height.next();
        let cfg = self.chain.cfg();
        let skeleton = Block::new(context.clone(), parent_block.digest(), height, ts, bytes::Bytes::new());
        let ctx = Chain::block_context(&cfg, &skeleton, &parent);
        let extras = Extras {
            handoff: self.chain.handoff_for(&parent),
            seed: self.chain.seed_for(&parent),
            upgrade: self.chain.upgrade_for(&parent),
            proofs: self.chain.proofs_for(&parent),
            beacons: self.chain.beacons_for(&parent),
            registrations: self.chain.registrations_for(&parent),
        };
        // Under the parent's next protocol, with its one-time changes if it activates here.
        let attempt = self.chain.pre_state_with(&parent, parent.next_protocol(), &extras.proofs, &extras.beacons, &extras.registrations, extras.seed.as_ref(), false);
        let mut extras = extras;
        let (pre, payouts) = match attempt {
            Ok(pre) => pre,
            // Pooled proofs that no longer verify here: drop them and propose without.
            // (Beacon answers and registrations were checked against this
            // parent: they go too, only for this block.)
            Err(e) if !extras.proofs.is_empty() || !extras.beacons.is_empty() || !extras.registrations.is_empty() => {
                warn!(?e, "dropping pooled proofs, beacon answers and registrations from this proposal");
                self.chain.drop_proofs(&extras.proofs.iter().map(|c| c.height).collect::<Vec<_>>());
                extras.proofs.clear();
                extras.beacons.clear();
                extras.registrations.clear();
                // The seed stays: this block still carries it, so its commitment still happens.
                match self.chain.pre_state(&parent, parent.next_protocol(), &[], extras.seed.as_ref(), false) {
                    Ok(pre) => pre,
                    Err(e) => {
                        warn!(?e, "not proposing");
                        return None;
                    }
                }
            }
            Err(e) => {
                warn!(?e, "not proposing");
                return None;
            }
        };
        let (payload, mut out) = build_payload(&parent, &pre, &ctx, self.chain.mempool_candidates(), extras);
        let statement = if crate::chain::records_statement(&cfg, &payload) { crate::chain::statement(&ctx, &payload.txs, &pre, &out) } else { [0; 32] };
        crate::chain::with_activation(&pre, &mut out);
        drop(pre);
        let tx_hashes = payload.txs.iter().map(aether_execution::tx_hash).collect();
        let registration_ids = payload.registrations.iter().map(crate::registrations::id).collect();
        let block = Block::new(context, parent_block.digest(), height, ts, payload.to_bytes());
        if !payload.proofs.is_empty() {
            self.chain.proposed_with_proofs(height.get(), block.digest());
        }
        let pending = match &payload.handoff {
            Some(h) => {
                Some(std::sync::Arc::new(crate::handoff::Pending { at: height.get(), switch: height.get() + crate::handoff::DELAY, handoff: h.clone() }))
            }
            None => parent.handoff.clone(),
        };
        let seed = payload.seed.as_ref().map(|s| std::sync::Arc::new((height.get(), s.clone()))).or_else(|| parent.seed.clone());
        let schedule = payload.upgrade.as_ref().map(|u| crate::chain::scheduled(&parent.schedule, &u.upgrade)).unwrap_or_else(|| parent.schedule.clone());
        self.chain.remember(&block, &parent, &ctx, out, tx_hashes, pending, seed, schedule, statement, payouts, registration_ids);
        info!(height = %height, txs = payload.txs.len(), "proposed");
        Some(block)
    }

    async fn verify(&mut self, (rt, _): (E, Self::Context), mut ancestry: impl Ancestry<Self::Block>) -> bool {
        let Some(block) = ancestry.next().await else { return false };
        let Some(parent_block) = ancestry.next().await else { return false };
        if block.timestamp < parent_block.timestamp || block.timestamp > MAX_BLOCK_TIMESTAMP_MS {
            return false;
        }
        // Never reject on the local clock (a verdict must not depend on when it is asked); wait out skew.
        // This verdict is the notarize vote (marshal `Inline`, see `crate::voting`), so a rule that
        // depends on what this node has seen, like FOCIL below, may live here and nowhere later.
        rt.sleep_until(SystemTime::UNIX_EPOCH + Duration::from_millis(block.timestamp.saturating_sub(MAX_FUTURE_SKEW_MS))).await;

        let Some(parent) = self.resolve(parent_block, ancestry).await else { return false };
        if self.chain.retired_after(&parent) {
            return false;
        }
        // Execution (and any proof verification it calls) runs on its own thread,
        // so a slow proof verifier never stalls the consensus executor.
        let (chain, b, p) = (self.chain.clone(), block.clone(), parent.clone());
        let executed = match rt.child("execute").dedicated().spawn(move |_| async move { chain.execute(&b, &p) }).await {
            Ok(r) => r,
            Err(e) => {
                warn!(height = %block.height(), ?e, "execution task failed");
                return false;
            }
        };
        match executed {
            Ok(exec) => {
                // FOCIL: refuse to notarize a block that censors listed txs.
                let ctx = Chain::block_context(&self.chain.cfg(), &block, &parent);
                let missing = self.chain.inclusion_violations(&exec, &ctx, std::time::Instant::now());
                if !missing.is_empty() {
                    warn!(height = %block.height(), missing = missing.len(), first = %missing[0], "inclusion list violated; not voting");
                    return false;
                }
                true
            }
            Err(e) => {
                warn!(height = %block.height(), ?e, "rejected block");
                false
            }
        }
    }
}

impl Reporter for Application {
    type Activity = Update<Block>;

    fn report(&mut self, activity: Self::Activity) -> Feedback {
        if let Update::Block(block, ack) = activity {
            match self.chain.finalize(&block) {
                Ok(()) => {
                    let g = self.chain.lock();
                    info!(
                        height = %block.height(),
                        txs = g.finalized.tx_hashes.len(),
                        root = %g.finalized.state.root(),
                        "finalized"
                    );
                }
                Err(e @ crate::chain::ChainError::ConflictingFinality { .. }) => {
                    tracing::error!(height = %block.height(), ?e, "CONFLICTING FINALIZED BLOCK: this node's chain differs from the network's; stop and investigate")
                }
                // Storage, not the block: heal it here, synchronously
                // (docs/design/24-self-healing.md layer 1) — marshal waits on
                // this call, so consensus on this node pauses until the block
                // is on disk, then continues without a restart. Only a stored
                // block is ever acknowledged: acknowledging here would leave a
                // gap in the store that no restart repairs, while an
                // unacknowledged one is redelivered ("at-least-once delivery").
                Err(crate::chain::ChainError::Store(e)) => {
                    tracing::error!(height = %block.height(), %e, "storage failed while committing a finalized block; re-opening the database");
                    let mut on_disk = false;
                    let mut still_storage = false;
                    for _ in 0..3 {
                        // Exits with the storage code if the disk never heals.
                        self.chain.heal_store();
                        match self.chain.finalize(&block) {
                            Ok(()) => {
                                on_disk = true;
                                break;
                            }
                            // The disk took the probe write but not the commit.
                            Err(crate::chain::ChainError::Store(e)) => {
                                still_storage = true;
                                tracing::warn!(height = %block.height(), %e, "the re-opened store refused the commit again");
                            }
                            Err(e) => {
                                warn!(height = %block.height(), ?e, "failed to adopt finalized block");
                                break;
                            }
                        }
                    }
                    if on_disk {
                        let g = self.chain.lock();
                        info!(
                            height = %block.height(),
                            txs = g.finalized.tx_hashes.len(),
                            root = %g.finalized.state.root(),
                            "finalized"
                        );
                    } else if still_storage {
                        // A disk that takes a probe but not a block is still
                        // full: exit with the storage code rather than loop.
                        tracing::error!("the store still refuses commits; exiting so the app restarts the node");
                        std::process::exit(crate::store::EXIT_STORAGE);
                    }
                    // Otherwise the block itself is the problem: not
                    // acknowledged — marshal stops rather than mark a block
                    // this node does not hold as delivered.
                    if on_disk {
                        ack.acknowledge();
                    }
                    return Feedback::Ok;
                }
                Err(e) => warn!(height = %block.height(), ?e, "failed to adopt finalized block"),
            }
            ack.acknowledge();
        }
        Feedback::Ok
    }
}
