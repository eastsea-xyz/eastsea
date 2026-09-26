//! Consensus application: propose / verify / report finalized blocks.
//! Pacing and timestamp rules adapted from alto-chain (MIT OR Apache-2.0).

use crate::block::{Block, Context};
use crate::chain::{build_payload, Chain, Executed};
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
        if self.chain.rotation_due(&parent).is_some() {
            // The next voting set takes over from here (`aether run` reshares).
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
        let (payload, out) = build_payload(&parent, &ctx, self.chain.mempool_candidates());
        let tx_hashes = payload.txs.iter().map(aether_execution::tx_hash).collect();
        let block = Block::new(context, parent_block.digest(), height, ts, payload.to_bytes());
        self.chain.remember(&block, &parent, &ctx, out, tx_hashes);
        info!(height = %height, txs = payload.txs.len(), "proposed");
        Some(block)
    }

    async fn verify(&mut self, (rt, _): (E, Self::Context), mut ancestry: impl Ancestry<Self::Block>) -> bool {
        let Some(block) = ancestry.next().await else { return false };
        let Some(parent_block) = ancestry.next().await else { return false };
        if block.timestamp < parent_block.timestamp || block.timestamp > MAX_BLOCK_TIMESTAMP_MS {
            return false;
        }
        // Never reject on the local clock (certification must be deterministic); wait out skew.
        rt.sleep_until(SystemTime::UNIX_EPOCH + Duration::from_millis(block.timestamp.saturating_sub(MAX_FUTURE_SKEW_MS))).await;

        let Some(parent) = self.resolve(parent_block, ancestry).await else { return false };
        if self.chain.rotation_due(&parent).is_some() {
            return false;
        }
        match self.chain.execute(&block, &parent) {
            Ok(exec) => {
                // FOCIL: refuse to vote for a block that censors listed txs.
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
                Err(e) => warn!(height = %block.height(), ?e, "failed to adopt finalized block"),
            }
            ack.acknowledge();
        }
        Feedback::Ok
    }
}
