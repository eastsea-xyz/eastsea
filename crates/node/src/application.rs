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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::{info, warn};

/// Fixed consensus cutoff for timestamps (2200-01-01) so validity never depends on platform limits.
const MAX_BLOCK_TIMESTAMP_MS: u64 = 7_258_118_400_000;
const MAX_FUTURE_SKEW_MS: u64 = 1_000;
/// A new-genesis block must advance certified chain time by at least one second.
pub const MIN_BLOCK_INTERVAL_MS: u64 = 1_000;
static BELOW_FLOOR_WARNED: AtomicBool = AtomicBool::new(false);

fn mainnet_rules(chain: &Chain) -> bool {
    let cfg = chain.cfg();
    cfg.node_rewards || cfg.history_v2
}

fn proposal_delay_ms(configured: u64, new_genesis: bool) -> u64 {
    if new_genesis { configured.max(MIN_BLOCK_INTERVAL_MS) } else { configured }
}

fn valid_block_timestamp(timestamp: u64, parent: u64, new_genesis: bool) -> bool {
    let interval = if new_genesis { MIN_BLOCK_INTERVAL_MS } else { 0 };
    parent.checked_add(interval).is_some_and(|minimum| timestamp >= minimum && timestamp <= MAX_BLOCK_TIMESTAMP_MS)
}

#[derive(Clone)]
pub struct Application {
    chain: Chain,
    delay_ms: u64,
    proposals_enabled: Arc<AtomicBool>,
}

impl Application {
    pub fn new(chain: Chain, delay_ms: u64) -> Self {
        let delay = proposal_delay_ms(delay_ms, mainnet_rules(&chain));
        if delay != delay_ms && !BELOW_FLOOR_WARNED.swap(true, Ordering::Relaxed) {
            warn!(configured_ms = delay_ms, minimum_ms = MIN_BLOCK_INTERVAL_MS, "block-time-ms is below the new-genesis consensus floor; using the floor");
        }
        Self { chain, delay_ms: delay, proposals_enabled: Arc::new(AtomicBool::new(true)) }
    }

    /// Fault injection: keep voting, but produce no blocks when elected leader.
    pub fn with_proposal_switch(mut self, enabled: Arc<AtomicBool>) -> Self {
        self.proposals_enabled = enabled;
        self
    }

    /// Height of the finalized state this node already holds (restored from disk).
    pub fn finalized_height(&self) -> u64 {
        self.chain.finalized_height()
    }

    /// Prepare pooled extras without losing an accepted proof because an
    /// unrelated item became invalid, such as a registration at key rotation.
    #[allow(clippy::type_complexity)]
    fn proposal_pre_state<'a>(
        &self,
        parent: &'a Executed,
        extras: &mut Extras,
    ) -> Result<
        (
            std::borrow::Cow<'a, aether_execution::WorldState>,
            Vec<(u64, aether_types::Address, aether_types::U256)>,
        ),
        crate::chain::ChainError,
    > {
        let attempt = self.chain.pre_state_with(
            parent,
            parent.next_protocol(),
            &extras.proofs,
            &extras.beacons,
            &extras.registrations,
            extras.seed.as_ref(),
            false,
        );
        let attempt = match attempt {
            Err(e) if !extras.beacons.is_empty() || !extras.registrations.is_empty() => {
                warn!(
                    ?e,
                    "retrying proposal without beacon answers and registrations"
                );
                extras.beacons.clear();
                extras.registrations.clear();
                // Verified pool admission stops competing jobs permanently.
                // Keep that claim available unless the proof itself also fails.
                self.chain.pre_state(
                    parent,
                    parent.next_protocol(),
                    &extras.proofs,
                    extras.seed.as_ref(),
                    false,
                )
            }
            other => other,
        };
        match attempt {
            // An unavailable verifier gives no verdict on accepted claims.
            // Keep them for a proposal after the sidecar has recovered.
            Err(e @ crate::chain::ChainError::Exec(_)) => Err(e),
            Err(e) if !extras.proofs.is_empty() => {
                // Establish that the proof-free block works before deleting
                // claims: a failing activation or seed cannot invalidate them.
                let fallback = self.chain.pre_state(
                    parent,
                    parent.next_protocol(),
                    &[],
                    extras.seed.as_ref(),
                    false,
                )?;
                warn!(
                    ?e,
                    "dropping pooled proofs that failed proposal construction"
                );
                self.chain
                    .drop_proofs(&extras.proofs.iter().map(|c| c.height).collect::<Vec<_>>());
                extras.proofs.clear();
                // The seed remains in the payload and in its pre-state.
                Ok(fallback)
            }
            other => other,
        }
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
        if !self.proposals_enabled.load(Ordering::SeqCst) || !crate::resources::disk_ok() {
            return None;
        }
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
        if !crate::resources::disk_ok() {
            return None;
        }
        let ts = now.max(min_ts);
        if ts > MAX_BLOCK_TIMESTAMP_MS {
            return None;
        }

        let height = parent_block.height.next();
        let cfg = self.chain.cfg();
        let skeleton = Block::new(context.clone(), parent_block.digest(), height, ts, bytes::Bytes::new());
        let ctx = Chain::block_context(&cfg, &skeleton, &parent);
        let mut extras = Extras {
            handoff: self.chain.handoff_for(&parent),
            seed: self.chain.seed_for(&parent),
            upgrade: self.chain.upgrade_for(&parent),
            proofs: self.chain.proofs_for(&parent),
            beacons: self.chain.beacons_for(&parent),
            group: cfg.group,
            registrations: self.chain.registrations_for(&parent),
        };
        let saving_for_control = extras.fit_archive_budget(&parent, cfg.node_rewards || cfg.history_v2);
        if saving_for_control {
            warn!(height = %height, "deferring control payload until archive budget refills");
        }
        // Under the parent's next protocol, with its one-time changes if it activates here.
        let (pre, payouts) = match self.proposal_pre_state(&parent, &mut extras) {
            Ok(pre) => pre,
            Err(e) => {
                warn!(?e, "not proposing");
                return None;
            }
        };
        let candidates = if saving_for_control { vec![] } else { self.chain.mempool_candidates() };
        let (payload, mut out) = build_payload(&parent, &pre, &ctx, candidates, extras);
        if (cfg.node_rewards || cfg.history_v2)
            && payload.to_bytes().len() as u64 > crate::chain::payload_archive_limit(&parent, &payload) {
            warn!(height = %height, "empty/control payload exceeds archive budget; not proposing");
            return None;
        }
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
        if !crate::resources::disk_ok() {
            return None;
        }
        self.chain.remember(&block, &parent, &ctx, out, tx_hashes, pending, seed, schedule, statement, payouts, registration_ids);
        info!(height = %height, txs = payload.txs.len(), "proposed");
        Some(block)
    }

    async fn verify(&mut self, (rt, _): (E, Self::Context), mut ancestry: impl Ancestry<Self::Block>) -> bool {
        if !crate::resources::disk_ok() {
            return false;
        }
        let Some(block) = ancestry.next().await else { return false };
        let Some(parent_block) = ancestry.next().await else { return false };
        if !valid_block_timestamp(block.timestamp, parent_block.timestamp, mainnet_rules(&self.chain)) {
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
                if !crate::resources::disk_ok() {
                    return false;
                }
                // FOCIL: refuse to notarize a block that censors listed txs.
                let ctx = Chain::block_context(&self.chain.cfg(), &block, &parent);
                let payload = block.payload().expect("executed canonical payload");
                let missing = self.chain.inclusion_violations_in_payload(&exec, &ctx, std::time::Instant::now(), &parent, &payload);
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
            // Dropping an unacknowledged Update::Block makes marshal exit. Keep
            // its delivery pending, without touching the state store, until
            // the monitor sees enough free space to resume safely.
            if !crate::resources::disk_ok() {
                tracing::warn!(height = %block.height(), "disk almost full: waiting before finalized state commit");
                while !crate::resources::disk_ok() {
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
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

#[cfg(test)]
mod timestamp_tests {
    use super::*;

    #[test]
    fn one_tick_after_parent_is_only_valid_on_legacy_genesis() {
        assert!(valid_block_timestamp(10_001, 10_000, false));
        assert!(!valid_block_timestamp(10_001, 10_000, true));
        assert!(valid_block_timestamp(11_000, 10_000, true));
        assert!(!valid_block_timestamp(10_000, 10_000, true));
        assert!(!valid_block_timestamp(11_000, u64::MAX, true));
    }

    #[test]
    fn proposer_delay_below_mainnet_floor_cannot_produce_invalid_timestamp() {
        assert_eq!(proposal_delay_ms(100, true), MIN_BLOCK_INTERVAL_MS);
        assert_eq!(proposal_delay_ms(100, false), 100);
        assert_eq!(proposal_delay_ms(1_500, true), 1_500);
    }
}

#[cfg(test)]
mod proof_recovery_tests {
    use super::*;
    use crate::block::EPOCH;
    use crate::chain::{ChainConfig, ProofVerifier};
    use crate::upgrade::{combine, sign_emergency_partial, Upgrade};
    use aether_crypto::{P256Signer, Signer as _};
    use aether_execution::{proofs, registry, WorldState};
    use aether_light::block::{NodeRegistration, ProofClaim};
    use aether_types::{Address, GasVector, U256};
    use commonware_codec::Encode as _;
    use commonware_consensus::types::{Round, View};
    use commonware_cryptography::{ed25519, Signer as _};

    const CHAIN_ID: u64 = 7_797;
    const EPOCH_BLOCKS: u64 = 10;
    const PROOF_BYTES: usize = 32 << 10;

    struct PaddedEcho;

    impl ProofVerifier for PaddedEcho {
        fn verify(&self, proof: &[u8], output: [u8; 32]) -> bool {
            proof.len() == PROOF_BYTES
                && proof[..32] == output
                && proof[32..].iter().all(|byte| *byte == 0)
        }
    }

    struct Unavailable;

    impl ProofVerifier for Unavailable {
        fn verify(&self, _: &[u8], _: [u8; 32]) -> bool {
            false
        }
        fn decide(&self, _: &[u8], _: [u8; 32]) -> Option<bool> {
            None
        }
    }

    struct Net {
        chain: Chain,
        parent: Arc<Executed>,
        last: Block,
        registrar: P256Signer,
        prover: P256Signer,
        commitment: [u8; 32],
    }

    impl Net {
        /// An authenticated registrar upgrade is announced at height 1 and
        /// activates at 11. The prover registers at 1; its proof is still unpaid.
        fn before_rotation() -> Self {
            let registrar = P256Signer::from_seed(&[7; 32]).unwrap();
            let prover = P256Signer::from_seed(&[8; 32]).unwrap();
            let keys: Vec<_> = (1..=4).map(ed25519::PrivateKey::from_seed).collect();
            let cfg = ChainConfig {
                chain_id: CHAIN_ID,
                limits: GasVector {
                    exec: 30_000_000,
                    state: u64::MAX,
                    prove: 200_000_000,
                },
                alloc: vec![],
                fees: false,
                registrar: Some(aether_crypto::p256_xy(&registrar.public_key().bytes).unwrap()),
                epoch_blocks: EPOCH_BLOCKS,
                min_streak: Some(0),
                draw_epochs: Some(7),
                history_v2: false,
                protocol: 2,
                node_rewards: true,
                committee: keys
                    .iter()
                    .enumerate()
                    .map(|(i, key)| {
                        (
                            hex::encode(key.public_key().encode()),
                            aether_net::SecretKey::from_bytes(&[i as u8 + 1; 32])
                                .public()
                                .to_string(),
                        )
                    })
                    .collect(),
                reserve: None,
                group: 0,
                max_committee: crate::rotation::GROW_UNTIL,
            };
            let (chain, genesis) = Chain::new(cfg);
            let (_, sharing, shares) = aether_light::devnet_threshold(4);
            {
                let mut g = chain.lock();
                g.identity = Some(*sharing.public());
                g.protocol = 3;
                g.verifier = Some(Arc::new(PaddedEcho));
            }
            chain.set_prover_window(crate::prover_assignment::Config::default().window);
            let parent = chain.lock().finalized.clone();
            let mut net = Self {
                chain,
                parent,
                last: genesis,
                registrar,
                prover,
                commitment: [0; 32],
            };
            let next_registrar = P256Signer::from_seed(&[9; 32]).unwrap();
            let (x, y) = aether_crypto::p256_xy(&next_registrar.public_key().bytes).unwrap();
            let upgrade = Upgrade {
                chain_id: CHAIN_ID,
                protocol: 3,
                activate_at: EPOCH_BLOCKS + 1,
                emergency: true,
                releases: vec![],
                notes: String::new(),
                registrar: Some((x.into(), y.into())),
            };
            let approvals: Vec<_> = shares
                .iter()
                .zip(&keys)
                .take(3)
                .map(|((_, share), key)| sign_emergency_partial(&upgrade, share, key))
                .collect();
            let registration = net.registration(&net.prover, 1);
            net.step(Extras {
                upgrade: Some(combine(&sharing, &approvals).unwrap()),
                registrations: vec![registration],
                ..Extras::default()
            });
            net.commitment = net.parent.statement.commitment;
            while net.parent.height < EPOCH_BLOCKS {
                net.step(Extras::default());
            }
            net
        }

        fn address(&self) -> Address {
            aether_crypto::address_of(&self.prover.public_key()).unwrap()
        }

        fn registration(&self, operator: &P256Signer, index: u8) -> NodeRegistration {
            let address = aether_crypto::address_of(&operator.public_key()).unwrap();
            let key: [u8; 32] = ed25519::PrivateKey::from_seed(u64::from(index))
                .public_key()
                .encode()
                .as_ref()
                .try_into()
                .unwrap();
            let node = *aether_net::SecretKey::from_bytes(&[index; 32])
                .public()
                .as_bytes();
            let attestation = self
                .registrar
                .sign(&registry::attestation_message(
                    CHAIN_ID, address, key, node, address,
                ))
                .unwrap();
            let message = registry::relay_message(
                CHAIN_ID,
                address,
                &key,
                &node,
                address,
                &attestation,
                0,
                100,
            );
            NodeRegistration {
                operator: address,
                validator_key: key.into(),
                node_id: node.into(),
                beaconer: address,
                signature: operator.sign(&message).unwrap().into(),
                attestation: attestation.into(),
                operator_key: operator.public_key().bytes.into(),
                nonce: 0,
                expiry: 100,
            }
        }

        fn claim(&self) -> ProofClaim {
            let mut proof = vec![0; PROOF_BYTES];
            proof[..32].copy_from_slice(&aether_proving::block::claim(
                self.commitment,
                self.address(),
            ));
            ProofClaim {
                height: 1,
                prover: self.address(),
                proof: hex::encode(proof),
            }
        }

        fn pooled_extras(&self) -> Extras {
            let other = P256Signer::from_seed(&[10; 32]).unwrap();
            assert!(self
                .chain
                .add_registration(self.registration(&other, 5))
                .unwrap());
            Extras {
                proofs: self.chain.proofs_for(&self.parent),
                registrations: self.chain.registrations_for(&self.parent),
                ..Extras::default()
            }
        }

        fn block(&self, pre: &WorldState, extras: Extras) -> Block {
            let height = self.last.height.next();
            let context = Context {
                round: Round::new(EPOCH, View::new(height.get())),
                leader: ed25519::PrivateKey::from_seed(1).public_key(),
                parent: (View::new(height.get() - 1), self.last.digest()),
            };
            let timestamp = height.get() * MIN_BLOCK_INTERVAL_MS;
            let skeleton = Block::new(
                context.clone(),
                self.last.digest(),
                height,
                timestamp,
                bytes::Bytes::new(),
            );
            let ctx = Chain::block_context(&self.chain.cfg(), &skeleton, &self.parent);
            let (payload, _) = build_payload(&self.parent, pre, &ctx, vec![], extras);
            Block::new(
                context,
                self.last.digest(),
                height,
                timestamp,
                payload.to_bytes(),
            )
        }

        fn commit(&mut self, block: Block) -> Arc<Executed> {
            let executed = self.chain.execute(&block, &self.parent).unwrap();
            self.chain.finalize(&block).unwrap();
            self.parent = executed.clone();
            self.last = block;
            executed
        }

        fn step(&mut self, extras: Extras) -> Arc<Executed> {
            let block = {
                let (pre, _) = self
                    .chain
                    .pre_state_with(
                        &self.parent,
                        self.parent.next_protocol(),
                        &extras.proofs,
                        &extras.beacons,
                        &extras.registrations,
                        extras.seed.as_ref(),
                        false,
                    )
                    .unwrap();
                self.block(&pre, extras)
            };
            self.commit(block)
        }
    }

    #[test]
    fn old_registrar_registration_keeps_verified_proof_includable_and_paid_once() {
        let mut net = Net::before_rotation();
        let claim = net.claim();
        net.chain.add_own_proof(claim.clone()).unwrap();
        let mut extras = net.pooled_extras();
        assert_eq!((extras.proofs.len(), extras.registrations.len()), (1, 1));
        let error = net
            .chain
            .pre_state_with(
                &net.parent,
                net.parent.next_protocol(),
                &extras.proofs,
                &[],
                &extras.registrations,
                None,
                false,
            )
            .err()
            .expect("the old registrar's registration fails at activation");
        assert!(
            matches!(error, crate::chain::ChainError::Protocol(ref reason) if reason.contains("registrar"))
        );

        let before = net.parent.state.balance(&net.address());
        let app = Application::new(net.chain.clone(), 0);
        let (pre, payouts) = app.proposal_pre_state(&net.parent, &mut extras).unwrap();
        assert_eq!(
            extras.proofs.len(),
            1,
            "a rejected registration must not discard a verified unpaid proof"
        );
        assert!(extras.registrations.is_empty());
        assert_eq!(
            net.chain.proofs_for(&net.parent).len(),
            1,
            "the accepted proof remains available to subsequent proposals"
        );
        assert_eq!(payouts.len(), 1);
        assert_eq!((payouts[0].0, payouts[0].1), (1, net.address()));
        let paid = payouts[0].2;
        assert!(paid > U256::ZERO);
        assert_eq!(pre.balance(&net.address()), before + paid);
        let block = net.block(&pre, extras);
        drop(pre);
        let executed = net.commit(block.clone());
        assert_eq!(executed.payouts, payouts);
        assert_eq!(proofs::prover(&executed.state, 1), Some(net.address()));
        assert_eq!(executed.state.balance(&net.address()), before + paid);
        assert!(net.chain.add_own_proof(claim.clone()).is_err());
        assert!(net
            .chain
            .pre_state(
                &net.parent,
                net.parent.next_protocol(),
                &[claim],
                None,
                false
            )
            .is_err());
        net.chain.finalize(&block).unwrap();
        net.step(Extras::default());
        assert_eq!(
            net.parent.state.balance(&net.address()),
            before + paid,
            "replaying finality and the next block cannot pay the proof twice"
        );
    }

    #[test]
    fn sidecar_retry_preserves_pending_proof_after_registrar_rotation_failure() {
        let mut net = Net::before_rotation();
        let config = crate::prover_assignment::Config::default();
        let now = net.last.timestamp;
        let first = net.chain.provable_for(net.address(), &config, now).unwrap();
        assert_eq!(first.0.height, 1);
        // The service uses retry_proof when replacing a dead sidecar. A job
        // with no accepted competitor must become dispatchable again.
        net.chain.retry_proof(1);
        net.chain.lock().verifier = Some(Arc::new(PaddedEcho));
        assert_eq!(
            net.chain
                .provable_for(net.address(), &config, now)
                .unwrap()
                .0
                .height,
            1
        );

        net.chain.add_own_proof(net.claim()).unwrap();
        let mut extras = net.pooled_extras();
        let app = Application::new(net.chain.clone(), 0);
        net.chain.lock().verifier = Some(Arc::new(Unavailable));
        let error = app
            .proposal_pre_state(&net.parent, &mut extras)
            .err()
            .expect("an unavailable verifier postpones the proposal without rejecting its proof");
        assert!(matches!(error, crate::chain::ChainError::Exec(_)));
        // Replacement after an accepted claim must keep cancellation intact,
        // while the accepted proof still has a path to automatic inclusion.
        net.chain.lock().verifier = Some(Arc::new(PaddedEcho));
        net.chain.retry_proof(1);
        assert!(net.chain.proof_seen(1));
        assert_ne!(
            net.chain
                .provable_for(net.address(), &config, now + 60_001)
                .map(|job| job.0.height),
            Some(1)
        );
        assert_eq!(net.chain.proofs_for(&net.parent).len(), 1, "a sidecar restart must not leave the accepted height permanently suppressed without its pending claim");
        assert_eq!(extras.proofs.len(), 1);
        let (pre, _) = app.proposal_pre_state(&net.parent, &mut extras).unwrap();
        drop(pre);
        let paid = net.step(extras);
        assert_eq!(proofs::prover(&paid.state, 1), Some(net.address()));
        assert_eq!(
            paid.payouts
                .iter()
                .filter(|(height, _, _)| *height == 1)
                .count(),
            1
        );
    }
}
