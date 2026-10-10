//! Chain state shared by the consensus application, RPC and gossip.
//!
//! Every verified or proposed block's post-state is kept by digest so
//! competing forks can be validated; the finalized head is what RPC serves.

use crate::block::{Block, Payload, PublicKey};
use crate::inclusion::{self, InclusionPool};
use crate::store::{Commit, Store, StoreError};
use crate::tombstone::{Reason as DropReason, Tombstones};
use aether_crypto::{address_of, PublicKey as AetherPk};
use aether_execution::{
    execute_block, fees, BlockContext, BlockOutcome, FeePolicy, Receipt, WorldState,
};
use aether_hash::ChainHasher;
use aether_types::{Address, Canonical, FeeVector, GasVector, SignerScheme, TxEnvelope, TxHash, B256, U256};
use commonware_consensus::Heightable;
use commonware_cryptography::{sha256::Digest, Digestible};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_TXS_PER_BLOCK: usize = 2_000;
pub const MAX_MEMPOOL: usize = 50_000;
/// Pending txs one sender may have (spam from one key cannot fill the pool).
pub const MAX_PER_SENDER: usize = 64;
/// Largest tx admitted, by its canonical encoding. Contract deploys are the
/// biggest legitimate payloads and the EVM caps their initcode at 49 KiB
/// (EIP-3860), so 128 KiB takes any real call while keeping 50,000 txs well
/// under a block's 8 MiB decode cap. Local admission policy, not consensus.
pub const MAX_TX_BYTES: usize = 128 * 1024;
/// All pending txs together, by the same count: their encodings' bytes. Like
/// `MAX_MEMPOOL`, it rejects new txs when full — except that a strictly
/// higher-fee tx may evict the lowest-fee entry to make room (fee networks
/// only; see `add_to_mempool`).
pub const MAX_MEMPOOL_BYTES: usize = 64 * 1024 * 1024;
/// A pending tx that has not made it into a block in this long leaves the pool
/// (a nonce gap it cannot close, or fee caps the base fee stays above).
pub const MEMPOOL_TTL: Duration = Duration::from_secs(10 * 60);
/// Zero-fee entries (nothing over the base fee, no tip) are capped at a
/// fraction of the pool (R2-6, audit round 2, 2026-10-03): without a cap a
/// zero-balance spammer's free txs could hold every slot and crowd paying
/// senders out. Fee networks only — 7780 has no fees and one lane as before.
pub const MAX_FREE_MEMPOOL: usize = MAX_MEMPOOL / 4;
/// Bytes the free lane may hold (A3-3, audit round 3, 2026-10-04): the count
/// quota above left the whole byte budget reachable by free txs — a few
/// hundred fat zero-fee calls, far under `MAX_FREE_MEMPOOL` — so zero-fee
/// calldata spam could still starve paying senders of bytes. One eighth keeps
/// the lane's byte share strictly below the three quarters of entries paying
/// senders always keep, however fat the zero-fee traffic: 8 MiB seats ~64
/// maximum-size txs (or any number of small ones) while ≥ 56 MiB stays
/// reserved for fees. Fee networks only — 7780 has no fees and one lane as
/// before.
pub const MAX_FREE_MEMPOOL_BYTES: usize = MAX_MEMPOOL_BYTES / 8;
/// How long a tx whose fee caps sit below the base fee may hold pool capacity
/// waiting for the fee to come down (R2-6): the base fee falls by a full
/// target per empty block, so any spike a rational cap waited out is gone in
/// well under a minute. Past the wait the entry leaves instead of squatting
/// until the TTL; the sender always knows, because admission refuses
/// below-cap txs — re-submitting at higher caps is its move.
pub const MEMPOOL_FEE_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub limits: GasVector,
    pub alloc: Vec<(Address, U256)>,
    /// Fee policy v0 (docs/research/tokenomics-2026.md): exponential base fees,
    /// base exec burned, prove fees and 20% of tips to the prover escrow.
    pub fees: bool,
    /// DeviceCheck registrar key: predeploys the voting-node registry at genesis.
    pub registrar: Option<([u8; 32], [u8; 32])>,
    /// Blocks per voting-node epoch (0 = the default).
    pub epoch_blocks: u64,
    /// Voting-set draw parameters (None = the defaults).
    pub min_streak: Option<u64>,
    pub draw_epochs: Option<u64>,
    /// History v2, for a new genesis only (docs/research/history-compression-2026.md,
    /// roadmap B): a block with no transactions and no proofs records no
    /// proof-market statement, so runs of empty blocks leave the state root and
    /// chain metadata unchanged and cost a few bytes each in era files; the
    /// node also seals every 8192 finalized blocks into an era file. Bound to
    /// the genesis hash, so nodes can never disagree on it. Off on 7780.
    pub history_v2: bool,
    /// Protocol rules from height 0 (docs/design/15-node-rewards.md: the
    /// mainnet runs the newest rules — proof market, registry v2 with its
    /// registration cap, 16-seat growth — without an upgrade; see
    /// `crate::mainnet`). 1 (or 0): the testnet's genesis, byte-identical,
    /// later protocols arrive by committee-signed upgrade.
    pub protocol: u32,
    /// Node rewards (docs/design/15-node-rewards.md): half of each block's
    /// issuance to the operators whose Macs beaconed, half to provers, 1/16 cap
    /// per operator. A new network's genesis parameter (needs the registry);
    /// off keeps the testnet's rules and genesis.
    pub node_rewards: bool,
    /// The genesis validators (network.json `genesis_validators`, kept there
    /// across handoffs), which the genesis state records as the first voting
    /// committee (`rewards::set_committee`): with the running committee and
    /// every decided next roster in state, each node derives them the same way
    /// — validator, follower, or synced from a snapshot — and a handoff binds
    /// to the committed roster (finding 2). Node rewards only; empty on 7780.
    pub committee: Vec<(String, String)>,
    /// Founder reserve keys (docs/design/12-launch-plan.md, "창업자 Mac 안전망";
    /// needs node rewards): up to three voting keys. From protocol 4 they stay
    /// eligible standby through four independent operators and repair vacancies.
    pub reserve: Option<Reserve>,
    /// The consensus group this chain is (13-roadmap.md, 그룹 분열 준비):
    /// 0 is the only group today. Blocks of a group run only that group's
    /// txs, and the group is part of what a finalization certificate signs,
    /// so no group's proof of finality passes as another's. Fixed at genesis
    /// (a group-1 chain is a new genesis of its own).
    pub group: u16,
    /// Seats the voting committee grows to before draws start swapping
    /// instead of adding (`rotation::GROW_UNTIL` is the default; a new
    /// network's genesis parameter, at least four).
    pub max_committee: usize,
}

/// The founder's reserve validator keys, a genesis parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reserve {
    /// The founder's operator address (its registered Macs are not independent).
    pub operator: Address,
    /// (ed25519 key hex, iroh node id) of each reserve key.
    pub members: Vec<(String, String)>,
}

impl Reserve {
    /// (key, node id) bytes of each reserve key.
    pub fn bytes(&self) -> Result<Vec<aether_rewards::ReserveKey>, String> {
        self.members
            .iter()
            .map(|(k, n)| {
                let key: [u8; 32] = hex::decode(k.trim_start_matches("0x"))
                    .ok()
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| format!("reserve key {k}: 32-byte hex"))?;
                let node = n
                    .parse::<aether_net::EndpointId>()
                    .map_err(|e| format!("reserve node {n}: {e}"))?;
                Ok((key, *node.as_bytes()))
            })
            .collect()
    }

    /// The reserve set at genesis in `state`, if any.
    pub fn of(state: &WorldState) -> Option<crate::rotation::Reserve> {
        let (operator, keys) = aether_rewards::reserve(state)?;
        Some(crate::rotation::Reserve {
            operator: format!("{operator:#x}"),
            members: keys
                .iter()
                .filter_map(|(k, n)| {
                    aether_net::EndpointId::from_bytes(n)
                        .ok()
                        .map(|id| (hex::encode(k), id.to_string()))
                })
                .collect(),
        })
    }
}

impl ChainConfig {
    pub fn genesis_state(&self) -> WorldState {
        assert!((4..=crate::rotation::MAX_VOTING_NODES).contains(&self.max_committee), "max_committee must be 4..=128");
        assert!(
            (self.group == 0 && self.max_committee == crate::rotation::GROW_UNTIL)
                || (self.node_rewards && self.history_v2),
            "group and custom max_committee require a node_rewards/history-v2 genesis"
        );
        assert!(self.group == 0 || (self.registrar.is_none() && self.reserve.is_none()), "registry and rewards reserve belong to root group 0");
        let mut s = WorldState::default();
        for (a, v) in &self.alloc {
            s.set_balance(*a, *v).expect("genesis balance fits u128");
        }
        // The account contract P-256 accounts delegate to (EIP-7702) for batched calls.
        s.set_code(
            aether_execution::AETHER_ACCOUNT,
            aether_execution::aether_account_code_for_genesis(self.node_rewards, self.history_v2),
        )
        .expect("predeploy");
        if self.node_rewards && self.history_v2 {
            s.set_code(aether_rewards::REWARDS, aether_rewards::randomness_code())
                .expect("randomness predeploy");
            aether_rewards::set_max_committee(&mut s, self.max_committee as u64);
            // The app-release log (docs/design/19, checklist B6): at a fixed
            // address with known code, so the shipped network.json can pin
            // both before the ceremony record freezes its bytes.
            s.set_code(aether_execution::release_log::ADDRESS, aether_execution::release_log::code())
                .expect("release log predeploy");
            // Keep the released genesis catalogue for protocols 1..=3 so a
            // cold participant derives the existing network's original root.
            for (address, code, _) in aether_execution::predeploys::all() {
                s.set_code(address, code).expect("standard predeploy");
            }
            // Permit2 belongs only to a protocol-4-or-later genesis. This is
            // the frozen genesis protocol, never the running implementation
            // or a later committee-signed activation on an existing chain.
            if self.protocol >= 4 {
                s.set_code(crate::predeploys::PERMIT2, crate::predeploys::permit2_code())
                    .expect("Permit2 genesis predeploy");
            }
        }
        if let Some(key) = self.registrar {
            let d = aether_execution::registry::Params::default();
            let params = aether_execution::registry::Params {
                epoch_blocks: if self.epoch_blocks == 0 {
                    d.epoch_blocks
                } else {
                    self.epoch_blocks
                },
                min_streak: self.min_streak.unwrap_or(d.min_streak),
                draw_epochs: self.draw_epochs.unwrap_or(d.draw_epochs),
            };
            aether_execution::registry::predeploy(&mut s, key, params).expect("registry predeploy");
            if self.node_rewards {
                aether_rewards::enable(&mut s);
                aether_rewards::set_committee(&mut s, &self.committee)
                    .expect("genesis committee members parse");
                if let Some(r) = &self.reserve {
                    let keys = r.bytes().expect("valid reserve keys");
                    aether_rewards::set_reserve(&mut s, r.operator, &keys)
                        .expect("reserve keys");
                }
            }
        }
        // A genesis above protocol 1 installs what that protocol's activation
        // installs (the same one-time changes an activation block applies), so
        // a new network runs the newest rules from height 0 with no signed
        // upgrade (docs/design/15-node-rewards.md, gap G1).
        for p in 2..=self.protocol {
            aether_execution::forks::activate(p, &mut s).expect("genesis activation");
        }
        if self.node_rewards && self.history_v2 && self.registrar.is_some() {
            aether_rewards::registry_v3::genesis(&mut s).expect("new-genesis registry v3");
        }
        s
    }
}

/// Deterministic public development keys (like hardhat/anvil accounts).
/// NEVER use these for anything of value.
pub fn dev_seed(i: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[0] = 0xae;
    s[31] = i;
    s
}

pub fn dev_accounts(n: u8) -> Vec<(u8, Address)> {
    (1..=n)
        .map(|i| {
            let s = aether_crypto::P256Signer::from_seed(&dev_seed(i)).expect("valid dev seed");
            (
                i,
                address_of(&aether_crypto::Signer::public_key(&s)).expect("address"),
            )
        })
        .collect()
}

pub fn leader_address(leader: &PublicKey) -> Address {
    let pk = AetherPk {
        scheme: SignerScheme::Ed25519,
        bytes: leader.as_ref().to_vec(),
    };
    address_of(&pk).expect("ed25519 address")
}

#[derive(Clone)]
pub struct Executed {
    pub height: u64,
    pub digest: Digest,
    pub timestamp: u64,
    pub state: WorldState,
    pub receipts: Vec<Receipt>,
    pub tx_hashes: Vec<TxHash>,
    pub gas: GasVector,
    /// New storage slots counted against this block's 512-slot cap.
    pub new_slots: u64,
    /// Transaction and receipt bytes counted against this block's persistence cap.
    pub persistent_bytes: u64,
    /// Exact execution settlement for incremental archive append checks.
    pub settlement: fees::Settlement,
    pub proposer: Address,
    /// Base fees this block paid.
    pub base_fee: FeeVector,
    /// Fee-market excess after this block (its child's base fee derives from it).
    pub excess: GasVector,
    /// Independent encoded-payload debt; zero on legacy chains and genesis.
    pub archive_excess: u64,
    /// The latest committee handoff in this block's ancestry (inclusive).
    pub handoff: Option<Arc<crate::handoff::Pending>>,
    /// The latest draw seed in this block's ancestry (inclusive), with the height that carried it.
    pub seed: Option<Arc<(u64, aether_light::block::Seed)>>,
    /// Peaks of the MMR over blocks 0..=height; its root goes in the child's payload.
    pub history: Arc<aether_state::mmr::Mmr>,
    /// Protocol activations put on chain up to this block (committee-signed upgrades).
    pub schedule: Arc<crate::upgrade::Schedule>,
    /// What a proof of this block proves, and its escrow share (the child records both, protocol 2).
    pub statement: Statement,
    /// Proofs this block paid: (proven height, prover, amount). Node rewards
    /// paid by the first block of an epoch appear with the block's own height
    /// as the proven height (no block proves itself).
    pub payouts: Vec<(u64, Address, U256)>,
    /// Ids (`registrations::id`) of the free-lane registrations this block
    /// carried, for their pseudo-receipts when it finalizes.
    pub registration_ids: Vec<TxHash>,
}

/// A block's statement commitment (aether_proving::block) and escrow share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Statement {
    pub commitment: [u8; 32],
    pub escrow: U256,
}

/// Proofs per block, and the largest proof accepted (protocol 2).
pub const MAX_PROOFS_PER_BLOCK: usize = 2;
pub const MAX_PROOF_BYTES: usize = 128 << 10;
/// Real proofs are ~98 KB; anything much smaller is refused before any work.
const MIN_PROOF_BYTES: usize = 32 << 10;
/// Proof RPC: at most one verification started per interval (it is public).
const PROOF_CHECK_INTERVAL: Duration = Duration::from_millis(500);
/// Blocks without proofs in this node's proposals after one with proofs lost.
const PROOF_BACKOFF: u64 = 100;

/// Checks a block proof against a statement commitment (the pinned sidecar in a node).
pub trait ProofVerifier: Send + Sync {
    fn verify(&self, proof: &[u8], commitment: [u8; 32]) -> bool;
    /// Non-consensus program identity for compatibility diagnostics.
    fn program_id(&self) -> Option<String> { None }
    /// Some(answer) when the verifier actually decided; None when it could not
    /// (a transient failure, never to be remembered as a rejection).
    fn decide(&self, proof: &[u8], commitment: [u8; 32]) -> Option<bool> {
        Some(self.verify(proof, commitment))
    }
}

/// Stage-wise progress (red team 2026-09-29 #2): a node can be slow but
/// healthy — downloading a certified snapshot, re-opening its database after
/// a full disk, replaying a backlog — and a watchdog that looks at height
/// alone kills exactly those. Every real step of work ticks `activity`, and
/// each long stage names itself, so a watcher can tell "stuck" (nothing
/// moves) from "busy on something that is not a block" (height frozen,
/// activity advancing). Process-wide on purpose: whatever inside this node
/// is making progress is a reason not to kill it.
static ACTIVITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STAGE: Mutex<Option<&'static str>> = Mutex::new(None);

/// One step of real work happened (a block committed, a snapshot chunk
/// fetched, a store re-open attempted, an upstream answer taken).
pub fn tick() {
    ACTIVITY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// How many steps of work this process has done (starts at 0 each run).
pub fn activity() -> u64 {
    ACTIVITY.load(std::sync::atomic::Ordering::Relaxed)
}

/// Name the long stage this node is in (`"snapshot"`, `"storage"`…), or
/// `None` to say it is back to work a height already reports.
pub fn set_stage(stage: Option<&'static str>) {
    if let Ok(mut s) = STAGE.lock() {
        *s = stage;
    }
}

/// The stage [`set_stage`] last named.
pub fn stage() -> Option<&'static str> {
    STAGE.lock().ok().and_then(|s| *s)
}

/// Exact preimage of the chain metadata a block leaves outside the state tree.
pub fn meta_bytes(
    excess: &GasVector,
    handoff: Option<&crate::handoff::Pending>,
    seed: Option<&(u64, aether_light::block::Seed)>,
    schedule: &[crate::upgrade::Activation],
    statement: &Statement,
) -> Vec<u8> {
    // Protocol-1 encoding until an activation carries a registrar or a statement
    // is recorded: binaries of either protocol agree on protocol-1 blocks.
    let schedule: Vec<Value> = schedule
        .iter()
        .map(|a| match a.registrar {
            None => serde_json::json!([a.protocol, a.at]),
            Some(r) => serde_json::json!([a.protocol, a.at, r]),
        })
        .collect();
    if *statement == Statement::default() {
        serde_json::to_vec(&(excess, handoff, seed, schedule))
    } else {
        serde_json::to_vec(&(excess, handoff, seed, schedule, statement))
    }
    .expect("metadata serializes")
}

/// Hash of the chain metadata a block leaves outside the state tree.
pub fn meta_digest(
    excess: &GasVector,
    handoff: Option<&crate::handoff::Pending>,
    seed: Option<&(u64, aether_light::block::Seed)>,
    schedule: &[crate::upgrade::Activation],
    statement: &Statement,
) -> B256 {
    aether_light::chain_meta_digest(&meta_bytes(excess, handoff, seed, schedule, statement), 0)
}

impl Executed {
    pub fn meta_digest(&self) -> B256 {
        meta_digest_with_archive(
            &self.excess,
            self.handoff.as_deref(),
            self.seed.as_deref(),
            &self.schedule,
            &self.statement,
            self.archive_excess,
        )
    }

    /// The protocol whose rules the block after this one follows.
    pub fn next_protocol(&self) -> u32 {
        crate::upgrade::protocol_at(&self.schedule, self.height + 1)
    }
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct BlockSummary {
    pub height: u64,
    pub hash: String,
    pub parent: String,
    pub timestamp_ms: u64,
    pub proposer: Address,
    /// State root after executing this block (committed in the child header).
    pub state_root: B256,
    pub parent_state_root: B256,
    pub txs: Vec<TxHash>,
    pub gas_used: u64,
    pub prove_gas: u64,
    #[serde(default)]
    pub base_fee: FeeVector,
    #[serde(default)]
    pub excess: GasVector,
    /// Stored through versioned store/snapshot envelopes; legacy postcard
    /// summaries retain their exact field layout.
    #[serde(skip)]
    pub archive_excess: u64,
}

/// Preserve the legacy metadata commitment when archive debt is zero, and
/// certify nonzero debt without changing legacy payload/proof statement fields.
pub fn meta_digest_with_archive(
    excess: &GasVector,
    handoff: Option<&crate::handoff::Pending>,
    seed: Option<&(u64, aether_light::block::Seed)>,
    schedule: &[crate::upgrade::Activation],
    statement: &Statement,
    archive_excess: u64,
) -> B256 {
    aether_light::chain_meta_digest(&meta_bytes(excess, handoff, seed, schedule, statement), archive_excess)
}

/// Parent metadata authenticated by the finalized head's payload. A restored
/// checkpoint may not retain its parent yet; no witness is served in that case.
pub fn upgrade_metadata(g: &Inner) -> Option<Value> {
    let head = g.finalized_block.as_ref().filter(|b|
        b.digest() == g.finalized.digest && b.height.get() == g.finalized.height
    )?;
    let parent = g.executed.get(&head.parent)?;
    if parent.height.checked_add(1) != Some(g.finalized.height) {
        return None;
    }
    let encoded = meta_bytes(
        &parent.excess, parent.handoff.as_deref(), parent.seed.as_deref(),
        &parent.schedule, &parent.statement,
    );
    Some(serde_json::json!({
        "height": parent.height, "encoded": aether_light::to_hex(&encoded),
        "archive_excess": parent.archive_excess,
    }))
}

/// A summary's rough share of the history caches: itself plus each tx hash.
pub(crate) fn summary_bytes(s: &BlockSummary) -> u64 {
    (std::mem::size_of::<BlockSummary>() + s.txs.len() * std::mem::size_of::<TxHash>()) as u64
}

/// A receipt's rough share: itself, its return data, and its events.
pub(crate) fn receipt_bytes(r: &Receipt) -> u64 {
    let events: usize = r
        .events
        .iter()
        .map(|e| std::mem::size_of::<aether_execution::Event>() + e.topics.len() * 32 + e.data.len())
        .sum();
    (std::mem::size_of::<Receipt>() + r.output.len() + events) as u64
}

/// A free-lane registration's derived confirmation. Its registry state is
/// durable; this notification does not claim transaction receipt inclusion.
fn registration_receipt(hash: TxHash) -> Receipt {
    Receipt {
        tx_hash: hash,
        success: true,
        gas_used: 0,
        prove_gas: 0,
        state_gas: 0,
        state_fee: U256::ZERO,
        contract_address: None,
        logs: 0,
        output: Default::default(),
        events: vec![],
    }
}

/// The history caches' estimated bytes: kept summaries plus kept receipts.
fn caches_bytes_of(blocks: &BTreeMap<u64, BlockSummary>, receipts: &HashMap<TxHash, (u64, Receipt)>) -> u64 {
    blocks.values().map(summary_bytes).sum::<u64>() + receipts.values().map(|(_, r)| receipt_bytes(r)).sum::<u64>()
}

/// Legacy networks keep their archive in redb; their memory copy never grows
/// beyond this ceiling, including when no resource monitor is installed.
const LEGACY_HISTORY_CACHE_BYTES: u64 = 64 << 20;

pub(crate) fn legacy_cache_budget() -> u64 {
    crate::resources::monitor().map_or(LEGACY_HISTORY_CACHE_BYTES, |m| {
        m.limits.max_memory.min(LEGACY_HISTORY_CACHE_BYTES)
    })
}

/// Conservative allocation estimate for a full state, computed outside the
/// consensus lock once when it is remembered. BTree nodes hold at most eleven
/// entries and non-root nodes at least five; allow 1280 bytes per stem node and
/// 512 per value node, including allocation overhead. Code is charged in full
/// even when another version shares its Arc. Journals and receipts count too.
fn execution_bytes(e: &Executed) -> u64 {
    let mut stems = 0u64;
    let mut values = 0u64;
    let mut tree = 0u64;
    let mut previous = None;
    for (key, _) in e.state.repo().entries() {
        let stem: [u8; 31] = key[..31].try_into().expect("stem");
        if previous != Some(stem) {
            if values > 0 { tree += (values / 5 + 1) * 512; }
            stems += 1;
            values = 0;
            previous = Some(stem);
        }
        values += 1;
    }
    if values > 0 { tree += (values / 5 + 1) * 512; }
    if stems > 0 { tree += (stems / 5 + 1) * 1280; }
    let journal = e.state.journal();
    tree + std::mem::size_of::<Executed>() as u64 + 256
        + e.state.codes().values().map(|code| code.len() as u64 + 256).sum::<u64>()
        + (journal.writes.capacity() * std::mem::size_of::<(aether_state::TreeKey, Option<aether_state::Value>)>()) as u64
        + journal.codes.iter().map(|(_, code)| code.len() as u64 + 64).sum::<u64>()
        + (journal.codes.capacity() * std::mem::size_of::<(B256, aether_types::Bytes)>()) as u64
        + e.receipts.iter().map(receipt_bytes).sum::<u64>()
        + ((e.receipts.capacity() - e.receipts.len()) * std::mem::size_of::<Receipt>()) as u64
        + (e.tx_hashes.capacity() * std::mem::size_of::<TxHash>()) as u64
}

struct ProvingInput {
    encoded: Box<[u8]>,
    commitment: [u8; 32],
    bytes: u64,
}

fn retained_bytes(g: &Inner) -> u64 {
    g.caches_bytes.saturating_add(g.execution_sizes.values().sum::<u64>()).saturating_add(g.proving_bytes)
        .saturating_add((g.executed.capacity() * (std::mem::size_of::<(Digest, Arc<Executed>)>() + 16)) as u64)
        .saturating_add((g.execution_sizes.capacity() * (std::mem::size_of::<(Digest, u64)>() + 16)) as u64)
        .saturating_add((g.proving_inputs.capacity() * (std::mem::size_of::<(Digest, Arc<ProvingInput>)>() + 16)) as u64)
        .saturating_add((g.recent.capacity() * std::mem::size_of::<Block>()) as u64)
}

fn trim_proving_inputs(g: &mut Inner) {
    let retained: std::collections::HashSet<_> = g.recent.iter().map(Block::digest).collect();
    g.proving_inputs.retain(|digest, _| retained.contains(digest));
    if g.proving_inputs.capacity() > g.proving_inputs.len().saturating_mul(4) + 16 { g.proving_inputs.shrink_to_fit(); }
    if g.recent.capacity() > g.recent.len().saturating_mul(4) + 16 { g.recent.shrink_to_fit(); }
    g.proving_bytes = g.proving_inputs.values().map(|input| input.bytes).sum();
}

/// Bring optional state and proving retention inside `budget`, preserving the
/// finalized head and its parent. Legacy history rows also have an independent
/// 64 MiB ceiling and remain in redb; history v2 evicts only sealed eras.
fn trim_caches(g: &mut Inner, budget: u64) {
    use aether_state::mmr::ERA_LEN;
    g.history_budget = budget;
    // Leave a quarter for witness construction, decoding, and other working
    // allocations. Head and its immediate parent are mandatory working state;
    // a budget smaller than those cannot be met by optional-cache eviction.
    let budget = budget.saturating_sub(budget / 4);
    let parent = g.finalized_block.as_ref().map(|b| b.parent);
    while retained_bytes(g) > budget {
        let oldest = g.executed.iter()
            .filter(|(digest, _)| **digest != g.finalized.digest && Some(**digest) != parent)
            .min_by_key(|(_, e)| e.height).map(|(digest, _)| *digest);
        let Some(digest) = oldest else { break };
        g.executed.remove(&digest);
        g.execution_sizes.remove(&digest);
    }
    while retained_bytes(g) > budget && !g.recent.is_empty() {
        g.recent.pop_front();
        trim_proving_inputs(g);
    }
    if !g.cfg.history_v2 {
        if g.store.is_none() {
            return; // a memory-only chain has no durable archive to back eviction
        }
        // Optional allocation pressure must not bypass the independent
        // legacy row ceiling, even when the total retention budget is larger.
        let row_budget = budget.min(legacy_cache_budget());
        let head = g.finalized.height;
        let mut floor = g.cache_below;
        while g.caches_bytes > row_budget || retained_bytes(g) > budget {
            let Some((&height, _)) = g.blocks.first_key_value() else { break };
            if height >= head {
                break;
            }
            let (_, summary) = g.blocks.pop_first().expect("first cached summary");
            g.caches_bytes = g.caches_bytes.saturating_sub(summary_bytes(&summary));
            for hash in &summary.txs {
                if let Some((_, receipt)) = g.receipts.remove(hash) {
                    g.caches_bytes = g.caches_bytes.saturating_sub(receipt_bytes(&receipt));
                }
            }
            floor = height + 1;
        }
        if g.cfg.node_rewards {
            // Registration confirmations are derived receipts whose ids are
            // not in the block's transaction list. Their cache expires too.
            g.receipts.retain(|_, (h, _)| *h >= floor);
            g.caches_bytes = caches_bytes_of(&g.blocks, &g.receipts);
        }
        if g.caches_bytes > row_budget || retained_bytes(g) > budget {
            // A single block's receipt set may exceed the budget too. The
            // head's state/summary remain; receipt RPC reads durable rows.
            g.receipts.clear();
            g.caches_bytes = caches_bytes_of(&g.blocks, &g.receipts);
        }
        g.cache_below = g.cache_below.max(floor);
        tracing::debug!(floor, bytes = g.caches_bytes, budget = row_budget,
            "evicted legacy history cache rows; archival rows remain in redb");
        return;
    }
    if retained_bytes(g) <= budget {
        return;
    }
    let Some(era_dir) = g.store.as_ref().map(|s| s.era_dir()) else { return };
    let open = g.history_index.as_ref().map_or(0, |i| i.eras.len() as u64);
    let head = g.finalized.height;
    while retained_bytes(g) > budget {
        let Some(&first) = g.blocks.keys().next() else { return };
        let era = first / ERA_LEN;
        if era >= open || (era + 1) * ERA_LEN > head {
            return; // the open (or still uncounted) era is never dropped
        }
        if !era_dir.join(crate::era::file_name(era)).exists() {
            return; // not sealed yet: nothing would back the heights
        }
        let floor = (era + 1) * ERA_LEN;
        g.blocks = g.blocks.split_off(&floor);
        g.receipts.retain(|_, (h, _)| *h >= floor);
        g.caches_bytes = caches_bytes_of(&g.blocks, &g.receipts);
        g.cache_below = g.cache_below.max(floor);
        tracing::warn!(
            era,
            floor,
            bytes = g.caches_bytes,
            budget,
            "history caches over their memory budget: dropped the oldest sealed era's summaries and receipts (blocks still served from the era file)"
        );
    }
}

pub struct Inner {
    pub cfg: ChainConfig,
    /// Node-local bounded delivery; consensus only publishes a height reference.
    pub push: Arc<crate::rpc_push::Hub>,
    /// Non-consensus discovery derived from pinned release commitments.
    pub(crate) release_watcher: crate::release::Watcher,
    executed: HashMap<Digest, Arc<Executed>>,
    execution_sizes: HashMap<Digest, u64>,
    pub finalized: Arc<Executed>,
    pub blocks: BTreeMap<u64, BlockSummary>,
    pub receipts: HashMap<TxHash, (u64, Receipt)>,
    /// Derived finalized discovery facts, with a separate lock so searching
    /// never holds the consensus/state lock while it ranks records.
    pub search: Arc<Mutex<crate::search::SearchIndex>>,
    pub search_sources: crate::search_sources::SearchSources,
    pub mempool: BTreeMap<TxHash, TxEnvelope>,
    /// When each mempool tx arrived (inclusion lists name the oldest).
    arrivals: HashMap<TxHash, Instant>,
    /// Each mempool tx's encoded size, and their sum: the pool's byte budget
    /// (`MAX_MEMPOOL_BYTES`) charges and releases exactly what was admitted.
    sizes: HashMap<TxHash, usize>,
    mempool_bytes: usize,
    /// Zero-fee entries in the pool (fee networks only; `MAX_FREE_MEMPOOL`
    /// caps them — R2-6).
    free_in_pool: usize,
    /// Bytes held by those zero-fee entries (`MAX_FREE_MEMPOOL_BYTES` caps
    /// them — A3-3): the free lane's share of the byte budget, kept in step
    /// with `free_in_pool`.
    free_mempool_bytes: usize,
    /// Why each recent tx left the pool without a block (bug #5): bounded,
    /// node-local memory that `aether_getReceipt` answers from.
    pub tombstones: Tombstones,
    /// Txs named by inclusion lists (FOCIL).
    pub inclusion: InclusionPool,
    /// Devnet fault injection: act as a proposer that censors this sender and
    /// ignores inclusion lists. Used to test censorship resistance.
    pub censor: Option<Address>,
    /// Devnet fault injection: leave this sender out of mempool ordering but
    /// still honour inclusion lists (its txs then land only via lists).
    pub deprioritize: Option<Address>,
    /// Durable finalized state; None = memory only (tests).
    store: Option<Arc<Store>>,
    /// Pending txs per sender (bounded by `MAX_PER_SENDER`).
    pub pending_by_sender: HashMap<Address, usize>,
    /// Each pending sender's nonces in the pool, with how many entries hold
    /// each (B5 review round 2, finding 4): a receipt read finds one
    /// sender's queue in O(log n + 64) instead of scanning the whole pool
    /// under the chain lock. Kept in step with `mempool` wherever it changes.
    nonces_by_sender: HashMap<Address, BTreeMap<u64, u32>>,
    /// The running voting set (validators only; empty on followers).
    pub committee: crate::rotation::Committee,
    /// Committee identity: handoffs must be signed under it (None: devnet dealer keys).
    pub identity: Option<aether_light::Identity>,
    /// First height of the epoch this node's voting set runs (0 = genesis).
    pub epoch_start: u64,
    /// Last height of the running epoch once a handoff is finalized (MAX = open);
    /// the consensus epocher reads it, so the running set re-proposes its last
    /// block until it is final, then stops.
    pub epoch_end: Arc<std::sync::atomic::AtomicU64>,
    /// The voting set proposed for the current ceremony window (from its draw).
    pub proposal: Option<(u64, Vec<(String, String)>)>,
    /// A committee-signed handoff waiting to be put in a block.
    pub handoff_ready: Option<aether_light::block::Handoff>,
    /// The Macs eligible for the current draw, frozen at its first block: (draw, pool).
    pub pool: Option<(u64, Vec<(String, String)>)>,
    /// A committee-signed draw seed waiting to be put in a block.
    pub seed_ready: Option<aether_light::block::Seed>,
    /// Roots of the complete 8192-block eras and the open era's MMR leaves,
    /// from genesis (history proofs read at most two eras' blocks, never all).
    /// None when this node started from a checkpoint (it has no early blocks).
    pub history_index: Option<Arc<aether_state::mmr::EraIndex>>,
    /// The newest protocol this node runs; blocks under a later one are refused.
    pub protocol: u32,
    /// One-time state changes of each protocol at its activation block.
    pub migrate: aether_execution::forks::Migration,
    /// Committee-signed upgrades this node knows of (from its upgrades folder),
    /// put on chain by its proposals until they are there.
    pub upgrades_known: Vec<crate::upgrade::SignedUpgrade>,
    /// Signed notices from finalized blocks that have not activated yet.
    pub upgrade_notices: Vec<crate::upgrade::SignedUpgrade>,
    /// Checks block proofs (protocol 2); None: this node refuses blocks carrying proofs.
    pub verifier: Option<Arc<dyn ProofVerifier>>,
    /// Proofs received and verified locally, waiting to go in a block.
    pub proof_pool: Vec<aether_light::block::ProofClaim>,
    /// The last finalized blocks (the prover builds its inputs from them).
    recent: std::collections::VecDeque<Block>,
    /// The full finalized head, including quiet blocks, authenticates the
    /// parent's metadata independently of the prover's statement window.
    finalized_block: Option<Block>,
    /// Heights this node's prover already took up.
    attempted: std::collections::BTreeSet<u64>,
    /// Verified competing proofs are permanent local losses, even if a pool
    /// entry is subsequently dropped. A sidecar restart must not retry them.
    lost_proofs: std::collections::BTreeSet<u64>,
    /// Local proving input retention, independent of consensus validation.
    prover_window: usize,
    proving_inputs: HashMap<Digest, Arc<ProvingInput>>,
    proving_bytes: u64,
    history_budget: u64,
    proof_observers: Vec<std::sync::mpsc::Sender<(u64, Instant)>>,
    /// The running job may outlive eviction from the scheduling window.
    proving_height: Option<u64>,
    /// When the proof RPC last started a verification (rate limit).
    last_proof_check: Option<Instant>,
    /// This node's last proposal carrying proofs: (height, block).
    proof_proposal: Option<(u64, Digest)>,
    /// No proofs in this node's proposals before this height.
    proof_backoff_until: u64,
    /// Proof submissions (claim output + proof) this node's RPC already refused.
    rejected: std::collections::HashSet<[u8; 32]>,
    /// Beacon answers checked against the finalized state, waiting for a
    /// block: by (epoch, slot, candidate index).
    beacon_pool: BTreeMap<(u64, u64, u64), aether_light::block::BeaconAnswer>,
    /// Where answers this node takes first go out to the other validators (validators only).
    pub beacon_out: Option<tokio::sync::mpsc::UnboundedSender<aether_light::block::BeaconAnswer>>,
    /// Free-lane registrations checked against the finalized state, waiting
    /// for a block: one in flight per operator (its signed nonce is one-shot).
    registration_pool: BTreeMap<Address, aether_light::block::NodeRegistration>,
    /// Where registrations this node takes first go out to the other validators (validators only).
    pub registration_out: Option<tokio::sync::mpsc::UnboundedSender<aether_light::block::NodeRegistration>>,
    /// Pruning (roadmap B4): first height whose summary and receipts are kept.
    pub pruned_below: u64,
    /// The history caches' memory budget (`--max-memory`): first height still
    /// cached after eviction. Legacy rows remain in redb; history v2's
    /// evicted eras remain in files (`old_block`, `era_leaves`).
    pub cache_below: u64,
    /// Estimated bytes of the kept block summaries and receipts.
    caches_bytes: u64,
    /// The last era read back from its file (old blocks served over RPC).
    era_cache: Option<(u64, Arc<crate::era::Era>)>,
    /// The network's finalized height, as this node last heard from its
    /// upstream (None: nothing told it). How far behind it is shows in
    /// `aether_status` (`catching_up`, `behind`) and gates acting as a
    /// validator while still catching up.
    pub net_height: Option<u64>,
    /// Replay mode (`follow` catching up): commits skip the per-block fsync.
    /// Every block being replayed is certified and re-fetchable, so a power
    /// loss only replays them; the first durable commit after it anchors the file.
    pub relaxed: bool,
}

#[cfg(test)]
impl Inner {
    /// Attach an isolated archive copy for RPC representation measurements.
    /// No production build can replace a chain's genesis-bound store this way.
    pub(crate) fn set_test_archive_store(&mut self, store: Arc<Store>) {
        self.store = Some(store);
    }
}

#[derive(Clone)]
pub struct Chain(pub Arc<Mutex<Inner>>);

#[derive(Debug)]
pub enum ChainError {
    BadPayload,
    ReceiptsRootMismatch,
    ParentRootMismatch,
    Exec(String),
    BalMismatch,
    GasMismatch,
    UnknownParent,
    Store(String),
    /// A different block was finalized at a height we already finalized: a
    /// safety failure (or this node was pointed at another chain). Never ignored.
    ConflictingFinality {
        height: u64,
    },
    BadHandoff(String),
    /// The block, or a tx in it, belongs to another consensus group.
    WrongGroup,
    /// Wrong protocol version, an invalid upgrade, or one this node does not run.
    Protocol(String),
    /// The payload's history root or parent metadata hash does not match the chain before it.
    HistoryMismatch,
}

impl Chain {
    pub fn new(cfg: ChainConfig) -> (Self, Block) {
        Self::new_with_search_sources(cfg, Default::default())
    }

    pub fn new_with_search_sources(cfg: ChainConfig, sources: crate::search_sources::SearchSources) -> (Self, Block) {
        let sources = sources.validated().expect("valid search protocol source pins");
        let state = cfg.genesis_state();
        let genesis = Block::genesis_with(cfg.chain_id, state.root(), cfg.history_v2, cfg.group);
        // A genesis above protocol 1 carries its activation from height 0: the
        // rules (installed in `genesis_state` above) apply to every block, and
        // committee-signed upgrades still append after it. Protocol 1 keeps the
        // empty schedule, so 7780's genesis is byte-identical.
        let schedule = (cfg.protocol > 1)
            .then(|| {
                vec![crate::upgrade::Activation { protocol: cfg.protocol, at: 0, registrar: None }]
            })
            .unwrap_or_default();
        let exec = Arc::new(Executed {
            height: 0,
            digest: genesis.digest(),
            timestamp: 0,
            state,
            receipts: vec![],
            tx_hashes: vec![],
            gas: GasVector::default(),
            new_slots: 0,
            persistent_bytes: 0,
            settlement: fees::Settlement::default(),
            proposer: Address::ZERO,
            base_fee: FeeVector::default(),
            excess: GasVector::default(),
            archive_excess: 0,
            handoff: None,
            seed: None,
            history: Arc::new(aether_state::mmr::Mmr::default().append(
                &ChainHasher::new(),
                0,
                &digest_bytes(&genesis.digest()),
            )),
            schedule: Arc::new(schedule),
            statement: Statement::default(),
            payouts: vec![],
            registration_ids: vec![],
        });
        let mut executed = HashMap::new();
        executed.insert(genesis.digest(), exec.clone());
        let execution_sizes = HashMap::from([(genesis.digest(), execution_bytes(&exec))]);
        let mut blocks = BTreeMap::new();
        blocks.insert(0, summary(&genesis, &exec, B256::ZERO));
        let caches_bytes = blocks.values().map(summary_bytes).sum::<u64>();
        let inner = Inner {
            cfg,
            push: Arc::new(crate::rpc_push::Hub::default()),
            release_watcher: crate::release::Watcher::default(),
            executed,
            execution_sizes,
            finalized: exec,
            blocks,
            receipts: HashMap::new(),
            search: Arc::new(Mutex::new(crate::search::SearchIndex::new())),
            search_sources: sources,
            mempool: BTreeMap::new(),
            pending_by_sender: HashMap::new(),
            nonces_by_sender: HashMap::new(),
            arrivals: HashMap::new(),
            sizes: HashMap::new(),
            mempool_bytes: 0,
            free_in_pool: 0,
            free_mempool_bytes: 0,
            tombstones: Tombstones::default(),
            inclusion: InclusionPool::default(),
            censor: None,
            committee: Default::default(),
            identity: None,
            epoch_start: 0,
            epoch_end: Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
            proposal: None,
            history_index: Some(Arc::new({
                let mut idx = aether_state::mmr::EraIndex::default();
                idx.push(
                    &ChainHasher::new(),
                    aether_state::mmr::leaf(
                        &ChainHasher::new(),
                        0,
                        &digest_bytes(&genesis.digest()),
                    ),
                );
                idx
            })),
            handoff_ready: None,
            pool: None,
            seed_ready: None,
            deprioritize: None,
            store: None,
            protocol: crate::upgrade::implements(),
            migrate: aether_execution::forks::activate,
            upgrades_known: Vec::new(),
            upgrade_notices: Vec::new(),
            verifier: None,
            proof_pool: Vec::new(),
            recent: Default::default(),
            finalized_block: None,
            attempted: Default::default(),
            lost_proofs: Default::default(),
            prover_window: 0,
            proving_inputs: HashMap::new(),
            proving_bytes: 0,
            history_budget: crate::resources::monitor().map(|m| m.limits.max_memory).unwrap_or_else(crate::resources::default_cache_budget),
            proof_observers: Vec::new(),
            proving_height: None,
            last_proof_check: None,
            proof_proposal: None,
            proof_backoff_until: 0,
            rejected: Default::default(),
            beacon_pool: BTreeMap::new(),
            beacon_out: None,
            registration_pool: BTreeMap::new(),
            registration_out: None,
            pruned_below: 0,
            cache_below: 0,
            caches_bytes,
            era_cache: None,
            net_height: None,
            relaxed: false,
        };
        (Chain(Arc::new(Mutex::new(inner))), genesis)
    }

    /// Open with durable state: resume from the stored checkpoint, or start at
    /// genesis and persist it.
    pub fn open(cfg: ChainConfig, store: Store) -> Result<(Self, Block), StoreError> {
        Self::open_with_search_sources(cfg, store, Default::default())
    }

    pub fn open_with_search_sources(cfg: ChainConfig, store: Store, sources: crate::search_sources::SearchSources) -> Result<(Self, Block), StoreError> {
        let cache_budget = (!cfg.history_v2).then(legacy_cache_budget);
        let (chain, genesis) = Self::new_with_search_sources(cfg, sources.clone());
        // The store belongs to one genesis: refuse data of another instead of diverging from it.
        let ours = genesis_digest(&genesis);
        match store.meta(GENESIS)? {
            Some(d) if d.as_slice() != ours.as_slice() => return Err(StoreError::OtherGenesis),
            Some(_) => {}
            // Only a new store takes this genesis; an unmarked one with a chain is of unknown origin.
            None if store.head()?.is_none() => store.put_meta(GENESIS, &ours)?,
            None => return Err(StoreError::OtherGenesis),
        }
        let store = Arc::new(store);
        let checkpoint = match cache_budget {
            Some(budget) => store.load_with_cache_budget(budget)?,
            None => store.load()?,
        };
        match checkpoint {
            Some(cp) => {
                use commonware_codec::DecodeExt;
                let search = match store.search_index(cp.height, cp.digest, sources.fingerprint())? {
                    Some(index) => index,
                    None => {
                        let index = rebuild_search_cache(&cp, &sources);
                        store.put_search_index(cp.height, cp.digest, &index, sources.fingerprint())?;
                        index
                    }
                };
                let digest = Digest::decode(cp.digest.as_slice())
                    .map_err(|_| StoreError::Corrupt("digest"))?;
                let summary = cp.blocks.get(&cp.height).cloned();
                let mut state = cp.state;
                state.clear_journal();
                let exec = Arc::new(Executed {
                    height: cp.height,
                    digest,
                    timestamp: summary.as_ref().map(|b| b.timestamp_ms).unwrap_or_default(),
                    state,
                    receipts: vec![],
                    tx_hashes: summary.as_ref().map(|b| b.txs.clone()).unwrap_or_default(),
                    gas: GasVector::default(),
                    new_slots: 0,
                    persistent_bytes: 0,
                    settlement: fees::Settlement::default(),
                    proposer: summary.as_ref().map(|b| b.proposer).unwrap_or_default(),
                    base_fee: summary.as_ref().map(|b| b.base_fee).unwrap_or_default(),
                    excess: summary.as_ref().map(|b| b.excess).unwrap_or_default(),
                    archive_excess: summary.as_ref().map(|b| b.archive_excess).unwrap_or_default(),
                    handoff: cp.handoff.map(Arc::new),
                    seed: cp.seed.map(Arc::new),
                    history: Arc::new(cp.history),
                    schedule: Arc::new(cp.schedule),
                    statement: cp.statement,
                    payouts: vec![],
                    registration_ids: vec![],
                });
                let bytes = execution_bytes(&exec);
                let mut g = chain.lock();
                g.search = Arc::new(Mutex::new(search));
                g.executed.insert(digest, exec.clone());
                g.execution_sizes.insert(digest, bytes);
                // History proofs need every block from genesis; a checkpoint-started node has none before it.
                // A pruned store keeps the roots of the eras it dropped instead (roadmap B4).
                g.history_index = match cache_budget {
                    Some(_) => rebuild_history_index_from_store(&store, &cp.era_roots, cp.pruned_below, exec.height, &exec.history),
                    None => rebuild_history_index(&cp.blocks, &cp.era_roots, cp.pruned_below, exec.height, &exec.history),
                }.map(Arc::new);
                g.pruned_below = cp.pruned_below;
                g.cache_below = cp.blocks.keys().next().copied().unwrap_or(cp.height);
                g.finalized = exec;
                g.upgrade_notices = cp.upgrade_notices;
                g.blocks = cp.blocks;
                g.receipts = cp.receipts;
                g.caches_bytes = caches_bytes_of(&g.blocks, &g.receipts);
                // Eras completed before a restart but not sealed yet — unless
                // the disk is below its free-space floor (the blocks stay
                // staged in the store; a later start's seal catches up).
                if g.cfg.history_v2 && crate::resources::disk_ok() {
                    let (store, head) = (store.clone(), g.finalized.height);
                    std::thread::spawn(move || crate::era::seal_pending(&store, head));
                }
                g.store = Some(store);
            }
            None => {
                let mut g = chain.lock();
                let genesis_exec = g.finalized.clone();
                let summary = g.blocks.get(&0).cloned().expect("genesis summary");
                let (genesis_bytes, empty) = (
                    commonware_codec::Encode::encode(&genesis),
                    aether_state::mmr::Mmr::default(),
                );
                store.commit(Commit {
                    height: 0,
                    digest: digest_bytes(&genesis_exec.digest),
                    root: genesis_exec.state.root(),
                    diff: genesis_exec.state.journal(),
                    summary: &summary,
                    receipts: vec![],
                    handoff: None,
                    seed: None,
                    history: &genesis_exec.history,
                    schedule: &genesis_exec.schedule,
                    upgrade_notices: &[],
                    statement: &genesis_exec.statement,
                    staged: g.cfg.history_v2.then(|| crate::store::Staged {
                        block: &genesis_bytes,
                        era_start: Some(&empty),
                    }),
                })?;
                g.store = Some(store);
            }
        }
        // A checkpoint's kept history may already be over the memory budget.
        chain.trim_history_caches();
        Ok((chain, genesis))
    }

    pub fn finalized_height(&self) -> u64 {
        self.lock().finalized.height
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().expect("chain lock poisoned")
    }

    pub fn get(&self, d: &Digest) -> Option<Arc<Executed>> {
        self.lock().executed.get(d).cloned()
    }

    /// A shared immutable recent execution for RPC fanout. Never copies a
    /// block's receipts or state while holding the consensus lock.
    pub(crate) fn executed_at(&self, height: u64) -> Option<Arc<Executed>> {
        let g = self.lock();
        let finalized_hash = &g.blocks.get(&height)?.hash;
        // The cache also holds speculative forks at this height. Only the
        // digest in the finalized summary may label subscription receipts.
        g.executed.values().find(|e| e.height == height && e.digest.to_string() == *finalized_hash).cloned()
    }

    /// The durable store, if any (finality proofs a follower kept).
    pub fn store(&self) -> Option<Arc<Store>> {
        self.lock().store.clone()
    }

    pub fn cfg(&self) -> ChainConfig {
        self.lock().cfg.clone()
    }

    /// Install the network.json release pin without changing genesis or
    /// consensus metadata. Recovered payloads are untrusted until reverified.
    pub fn watch_releases(&self, network: Option<&Value>) {
        let mut g = self.lock();
        let pin = crate::release::Watcher::pinned(network, g.cfg.chain_id);
        if g.release_watcher.pin == pin { return; }
        let mut watcher = crate::release::Watcher::new(pin);
        if watcher.pin.is_some() {
            if let Some(store) = &g.store {
                if let Ok(Some(bytes)) = store.meta(crate::release::CACHE_KEY) {
                    watcher.restore(&bytes, &g.finalized.state, g.cfg.chain_id, g.finalized.height);
                }
            }
            // Also recover a payload when a commit landed before its cache
            // write or this RPC attached after the publication was finalized.
            for (_, receipt) in g.receipts.values() {
                watcher.discover(std::slice::from_ref(receipt), &g.finalized.state, g.cfg.chain_id, g.finalized.height);
            }
        }
        g.release_watcher = watcher;
    }

    /// Checkpoint followers can receive exact payload bytes in the status
    /// they already request. An upstream claim never supplies approval.
    pub fn discover_release_hint(&self, value: &Value) {
        let mut g = self.lock();
        let finalized = g.finalized.clone();
        let chain_id = g.cfg.chain_id;
        g.release_watcher.discover_status(value, &finalized.state, chain_id, finalized.height);
    }

    /// How many blocks behind the network this node last knew itself to be
    /// (0 when caught up — and also when nothing ever told it a height, which
    /// is not the same thing: see `behind_known`).
    pub fn behind(&self) -> u64 {
        let g = self.lock();
        g.net_height.map_or(0, |n| n.saturating_sub(g.finalized.height))
    }

    /// `behind()`, but None while no network height is known: a node nothing
    /// has answered is not "0 behind" — it is any number of blocks stale, and
    /// must not act as though it were current. `catch_up` returns only once
    /// this is Some, and keeps the last height it heard so the margin stays
    /// meaningful after it (2026-09-29).
    pub fn behind_known(&self) -> Option<u64> {
        let g = self.lock();
        g.net_height.map(|n| n.saturating_sub(g.finalized.height))
    }

    /// Replay mode for `follow`: while a certified backlog is fetched and
    /// re-executed, commits skip the per-block fsync — every block is
    /// re-fetchable, so a power loss only replays them. Cleared at the tip:
    /// the next durable commit then anchors everything the replay built.
    pub fn set_relaxed(&self, relaxed: bool) {
        self.lock().relaxed = relaxed;
    }

    /// Re-open the store after a storage failure (docs/design/24-self-healing.md
    /// layer 1), synchronously where the failed commit happened: consensus on
    /// this node waits here — it must not finalize past a block that is not on
    /// disk. The in-memory finalized head never ran past a commit that failed,
    /// so nothing rolls back: once the database is re-opened and the disk
    /// takes a write again (a probe commit), the caller retries its commit.
    /// A heal that keeps failing ends the process with the storage exit code,
    /// so the app restarts the node, which catches up before it votes again.
    pub fn heal_store(&self) {
        let Some(store) = self.store() else { return };
        // A store healing under backoff is a stage (red team #2): each re-open
        // attempt ticks, so a watcher sees work while the height is frozen.
        set_stage(Some("storage"));
        if let Err(e) = crate::store::Recovery::from_env().reopen(&store) {
            tracing::error!(%e, "the store database did not recover; exiting so the app restarts the node");
            std::process::exit(crate::store::EXIT_STORAGE);
        }
        set_stage(None);
    }

    /// Adopt a committee-certified snapshot as the finalized head, executing
    /// nothing (`follow::jump` checked it against the certified block after
    /// it, and `snapshot::install_over` already wrote the store). Blocks
    /// between the old head and this one are skipped: as on a checkpoint
    /// start there is no early history index (the gap's blocks stay fetchable
    /// from era files, verified by the history root), while everything kept —
    /// old summaries, receipts, proofs — is certified history of this same
    /// chain. Nothing else moves: the mempool self-prunes against the new
    /// state on the next finalize, and pooled beacons older than the new head
    /// cannot make it into a block.
    pub fn adopt(&self, exec: Arc<Executed>, summary: BlockSummary) {
        let bytes = execution_bytes(&exec);
        let mut g = self.lock();
        let height = exec.height;
        let search = g.search.clone();
        let search_sources = g.search_sources.clone();
        let search_store = g.store.clone();
        let unchanged_search_head = g.finalized.height == height && g.finalized.digest == exec.digest;
        let search_digest = digest_bytes(&exec.digest);
        let search_at = exec.timestamp / 1000;
        let chain_id = g.cfg.chain_id;
        g.release_watcher.rebase(&exec.state, chain_id, height);
        g.executed.clear();
        g.executed.insert(exec.digest, exec.clone());
        g.execution_sizes.clear();
        g.execution_sizes.insert(exec.digest, bytes);
        let sb = summary_bytes(&summary);
        let old = g.blocks.insert(height, summary);
        g.caches_bytes = g.caches_bytes.saturating_add(sb).saturating_sub(old.as_ref().map(summary_bytes).unwrap_or(0));
        g.finalized = exec;
        let mut proven: Vec<_> = g.attempted.iter().copied().chain(g.proving_height)
            .filter(|h| aether_execution::proofs::prover(&g.finalized.state, *h).is_some()).collect();
        proven.sort_unstable();
        proven.dedup();
        for height in proven {
            g.notice_proof(height);
        }
        // The new head's base fee re-sorts the pool between paying and free
        // lanes; keep the quota's count and byte share true to it until the
        // next finalize.
        let base = Self::next_base_fee(&g.cfg, &g.finalized);
        if g.cfg.fees {
            let mut free = 0;
            let mut free_bytes = 0;
            for (h, t) in g.mempool.iter() {
                if effective_fee(t, base) == 0 {
                    free += 1;
                    free_bytes += g.sizes.get(h).copied().unwrap_or_default();
                }
            }
            g.free_in_pool = free;
            g.free_mempool_bytes = free_bytes;
        } else {
            g.free_in_pool = 0;
            g.free_mempool_bytes = 0;
        }
        g.history_index = None;
        // Old finalized blocks are no longer provable here (their states are
        // gone); do not let them hold the prover's queue.
        g.recent.clear();
        g.proving_inputs.clear();
        g.proving_bytes = 0;
        g.finalized_block = None;
        g.push.publish(height);
        drop(g);
        if let Some(restored) = search_store.as_ref().and_then(|store| store.search_index(height, search_digest, search_sources.fingerprint()).ok().flatten()) {
            *search.lock().expect("search index") = restored;
            return;
        }
        if unchanged_search_head { return; }
        // A snapshot contains state, not intervening discovery events. Old
        // resolver facts could have changed during the gap; report that gap.
        let mut index = search.lock().expect("search index");
        *index = crate::search::SearchIndex::new();
        index.apply(crate::search::SearchEvent::Tick { at: search_at });
        index.mark_history_incomplete();
    }

    /// Key rounds only go up: a handoff built on `parent` must carry a round
    /// above the last handoff's in its ancestry (so none is accepted twice).
    fn round_floor(parent: &Executed) -> u64 {
        parent
            .handoff
            .as_ref()
            .map(|p| p.handoff.round)
            .unwrap_or(0)
    }

    /// Whether this node's voting set has handed over before the block after
    /// `parent`: it then builds and votes for nothing past the switch.
    pub fn retired_after(&self, parent: &Executed) -> bool {
        let start = self.lock().epoch_start;
        parent
            .handoff
            .as_ref()
            .is_some_and(|p| p.switch > start && parent.height + 1 >= p.switch)
    }

    /// The handoff state after `block`: the parent's, or the one it carries
    /// (valid only when signed by the committee, no other still pending, and —
    /// on a node-rewards network — only to the roster the chain committed).
    fn next_handoff(
        &self,
        height: u64,
        parent: &Executed,
        carried: Option<&aether_light::block::Handoff>,
        state: &WorldState,
    ) -> Result<Option<Arc<crate::handoff::Pending>>, ChainError> {
        let Some(h) = carried else {
            return Ok(parent.handoff.clone());
        };
        if parent.handoff.as_ref().is_some_and(|p| height < p.switch) {
            return Err(ChainError::BadHandoff(
                "another handoff is still pending".into(),
            ));
        }
        // Rounds only go up: a signed handoff cannot be replayed later.
        if h.round <= Self::round_floor(parent) {
            return Err(ChainError::BadHandoff(format!(
                "handoff round {} is not above {}",
                h.round,
                Self::round_floor(parent)
            )));
        }
        let (identity, chain_id) = {
            let g = self.lock();
            (g.identity, g.cfg.chain_id)
        };
        let identity = identity.ok_or_else(|| {
            ChainError::BadHandoff("no committee identity (devnet dealer keys)".into())
        })?;
        crate::handoff::verify(chain_id, &identity, h).map_err(ChainError::BadHandoff)?;
        // Finding 2 (red team, 2026-09-29): a node-rewards network commits the
        // decided next roster in state, and a handoff may only hand the voting
        // set over to exactly that roster — neither the proposer nor the
        // signing committee can choose a different one. `state` is the block's
        // pre state, so it also holds a commitment this same block makes when
        // its seed lands with the handoff (`next_seed` refuses the seed right
        // after, and with it the block). No commitment for the draw, no
        // handoff: every committee change goes through a committed roster.
        if aether_rewards::enabled(&parent.state) {
            let draw = current_draw(&parent.state, height);
            let committed = aether_rewards::next_roster(state).filter(|(d, _)| *d == draw);
            if !committed.is_some_and(|(_, roster)| {
                aether_rewards::same_roster(&roster, &h.members)
                    || (chain_id != 7_780 && crate::handoff::roster_allowed(chain_id, &roster, &h.members))
            }) {
                return Err(ChainError::BadHandoff(
                    "not a ready subset of the roster the chain committed for this draw".into(),
                ));
            }
        }
        Ok(Some(Arc::new(crate::handoff::Pending {
            at: height,
            switch: height + crate::handoff::DELAY,
            handoff: h.clone(),
        })))
    }

    /// After a restart: a finalized handoff still ends this node's epoch at its
    /// switch, and a proposal in the current ceremony window still stands.
    pub fn resume(&self) {
        let mut g = self.lock();
        if let Some(p) = g.finalized.handoff.clone() {
            if p.switch > g.epoch_start {
                g.epoch_end
                    .store(p.switch - 1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let current = current_draw(&g.finalized.state, g.finalized.height);
        if aether_rewards::enabled(&g.finalized.state) {
            // Node-rewards networks keep both in state (see `finalize`).
            g.pool = aether_rewards::draw_pool(&g.finalized.state).filter(|(d, _)| *d == current);
            g.proposal = aether_rewards::next_roster(&g.finalized.state).filter(|(d, _)| *d == current);
            return;
        }
        let load = |key: &str| g.store.as_ref().and_then(|s| s.meta(key).ok().flatten());
        let proposal: Option<(u64, Vec<(String, String)>)> = load(PROPOSAL)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .flatten();
        let pool: Option<(u64, Vec<(String, String)>)> = load(POOL)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .flatten();
        g.proposal = proposal;
        if g.proposal.as_ref().is_some_and(|(d, _)| *d != current)
            && !proposal_in_window(&g, g.finalized.height, &g.finalized.state)
        {
            g.proposal = None;
        }
        g.pool = pool.filter(|(d, _)| *d == current);
    }

    /// The seed state after a block at `height`: the parent's, or the one it
    /// carries (only the current draw's, once, signed by the committee).
    pub fn next_seed(
        &self,
        height: u64,
        parent: &Executed,
        carried: Option<&aether_light::block::Seed>,
    ) -> Result<Option<Arc<(u64, aether_light::block::Seed)>>, ChainError> {
        let Some(s) = carried else {
            return Ok(parent.seed.clone());
        };
        let draw = current_draw(&parent.state, height);
        if s.draw != draw || draw == 0 || parent.seed.as_ref().is_some_and(|p| p.1.draw >= draw) {
            return Err(ChainError::BadHandoff(format!(
                "seed for draw {} in draw {draw}",
                s.draw
            )));
        }
        let (identity, chain_id) = {
            let g = self.lock();
            (g.identity, g.cfg.chain_id)
        };
        let identity = identity.ok_or_else(|| {
            ChainError::BadHandoff("no committee identity (devnet dealer keys)".into())
        })?;
        crate::handoff::verify_seed(chain_id, &identity, s).map_err(ChainError::BadHandoff)?;
        Ok(Some(Arc::new((height, s.clone()))))
    }

    /// The draw seed to put in a block built on `parent`, if one is ready and not yet there.
    pub fn seed_for(&self, parent: &Executed) -> Option<aether_light::block::Seed> {
        let ready = self.lock().seed_ready.clone()?;
        let draw = current_draw(&parent.state, parent.height + 1);
        (ready.draw == draw && parent.seed.as_ref().is_none_or(|p| p.1.draw < draw))
            .then_some(ready)
    }

    /// The handoff to put in a block built on `parent`, if one is ready and allowed.
    pub fn handoff_for(&self, parent: &Executed) -> Option<aether_light::block::Handoff> {
        let ready = self.lock().handoff_ready.clone()?;
        if ready.round <= Self::round_floor(parent) {
            return None;
        }
        let pending = parent.handoff.as_ref();
        if pending.is_some_and(|p| parent.height + 1 < p.switch || p.handoff == ready) {
            return None;
        }
        Some(ready)
    }

    /// Execution context of `block` on top of `parent` (base fees derive from the parent's excess).
    pub fn block_context(cfg: &ChainConfig, block: &Block, parent: &Executed) -> BlockContext {
        let proposer = leader_address(&block.context.leader);
        let state_growth = cfg.node_rewards || cfg.history_v2;
        let fees = cfg.fees.then(|| FeePolicy {
            base: Self::next_base_fee(cfg, parent),
            proposer,
        });
        let mut limits = cfg.limits;
        // The legacy state dimension was unlimited and unused. A finite value
        // activates state pricing and the rolling disk budget on new-genesis chains.
        limits.state = if state_growth { fees::state_block_limit(parent.excess.state) } else { u64::MAX };
        BlockContext {
            chain_id: cfg.chain_id,
            number: block.height().get(),
            timestamp: block.timestamp / 1000,
            beneficiary: if fees.is_some() {
                aether_execution::FEE_COLLECTOR
            } else {
                proposer
            },
            limits,
            fees,
        }
    }

    /// Base fees the child of `parent` pays.
    pub fn next_base_fee(cfg: &ChainConfig, parent: &Executed) -> FeeVector {
        let mut base = if cfg.fees {
            fees::base_fee(parent.excess, cfg.limits)
        } else {
            FeeVector::default()
        };
        if cfg.node_rewards || cfg.history_v2 {
            base.state = fees::state_base_fee(parent.excess.state);
        }
        base
    }

    /// Validate and execute `block` on top of `parent`, remembering the result.
    pub fn execute(&self, block: &Block, parent: &Executed) -> Result<Arc<Executed>, ChainError> {
        self.execute_as(block, parent, false)
    }

    /// Execute a block the committee already finalized: its proofs were
    /// verified by the quorum that certified it, so this node applies them
    /// without asking its own verifier (whose local failure must never stop it
    /// from following the chain).
    fn execute_certified(
        &self,
        block: &Block,
        parent: &Executed,
    ) -> Result<Arc<Executed>, ChainError> {
        self.execute_as(block, parent, true)
    }

    fn execute_as(
        &self,
        block: &Block,
        parent: &Executed,
        certified: bool,
    ) -> Result<Arc<Executed>, ChainError> {
        if let Some(done) = self.get(&block.digest()) {
            return Ok(done);
        }
        let payload = block.payload().ok_or(ChainError::BadPayload)?;
        // Canonical bytes only: no unknown fields, no second encoding of the same block.
        if payload.to_bytes() != block.data {
            return Err(ChainError::BadPayload);
        }
        if payload.parent_state_root != parent.state.root() {
            return Err(ChainError::ParentRootMismatch);
        }
        if payload.history_root != B256::from(parent.history.root(&ChainHasher::new())) {
            return Err(ChainError::HistoryMismatch);
        }
        if payload.parent_meta != parent.meta_digest() {
            return Err(ChainError::HistoryMismatch);
        }
        if payload.txs.len() > MAX_TXS_PER_BLOCK {
            return Err(ChainError::BadPayload);
        }
        // The block belongs to this chain's group and runs only its txs: a
        // group's certificate covers the payload (group included), so a block
        // of another group never finalizes here even before this check.
        let cfg = self.cfg();
        if (cfg.node_rewards || cfg.history_v2)
            && (!controls_within_archive_reserve(&payload)
                || block.data.len() as u64 > payload_archive_limit(parent, &payload))
        {
            return Err(ChainError::Protocol("encoded payload exceeds archive growth budget".into()));
        }
        if payload.group != cfg.group {
            return Err(ChainError::WrongGroup);
        }
        if payload.txs.iter().any(|tx| tx.header.group() != cfg.group) {
            return Err(ChainError::WrongGroup);
        }
        let (pre, payouts) = self.pre_state_with(
            parent,
            payload.version,
            &payload.proofs,
            &payload.beacons,
            &payload.registrations,
            payload.seed.as_ref(),
            certified,
        )?;
        let schedule =
            self.next_schedule(block.height().get(), parent, payload.upgrade.as_ref())?;
        let ctx = Self::block_context(&cfg, block, parent);
        let mut out = execute_block(&pre, &ctx, &payload.txs)
            .map_err(|e| ChainError::Exec(format!("{e:?}")))?;
        // New-genesis certificates bind the result of this block's execution,
        // not a delayed result in its child. Legacy 7780 carries no such field.
        let expected_receipts_root = (cfg.node_rewards || cfg.history_v2)
            .then(|| aether_execution::receipt::receipt_root(&out.receipts));
        if payload.receipts_root != expected_receipts_root {
            return Err(ChainError::ReceiptsRootMismatch);
        }
        // Protocol-1 blocks keep no statement (their metadata stays protocol-1).
        let statement = if records_statement(&cfg, &payload) {
            statement(&ctx, &payload.txs, &pre, &out)
        } else {
            [0; 32]
        };
        with_activation(&pre, &mut out);
        if out.bal != payload.bal {
            return Err(ChainError::BalMismatch);
        }
        if out.gas != payload.gas {
            return Err(ChainError::GasMismatch);
        }
        let handoff = self.next_handoff(block.height().get(), parent, payload.handoff.as_ref(), &pre)?;
        let seed = self.next_seed(block.height().get(), parent, payload.seed.as_ref())?;
        let tx_hashes = payload.txs.iter().map(aether_execution::tx_hash).collect();
        let registration_ids = payload.registrations.iter().map(crate::registrations::id).collect();
        Ok(self.remember(
            block, parent, &ctx, out, tx_hashes, handoff, seed, schedule, statement, payouts, registration_ids,
        ))
    }

    /// The state the block after `parent` executes on: the parent's, plus the
    /// one-time changes of each protocol that activates at that block. Refuses
    /// a block whose version is not the scheduled one, or is newer than this node runs.
    #[allow(clippy::type_complexity)]
    pub fn pre_state<'a>(
        &self,
        parent: &'a Executed,
        version: u32,
        proofs: &[aether_light::block::ProofClaim],
        seed: Option<&aether_light::block::Seed>,
        certified: bool,
    ) -> Result<(std::borrow::Cow<'a, WorldState>, Vec<(u64, Address, U256)>), ChainError> {
        self.pre_state_with(parent, version, proofs, &[], &[], seed, certified)
    }

    /// `pre_state` of a block that also carries beacon answers and free-lane
    /// registrations, and may carry the draw seed whose voting-set decision
    /// becomes state.
    #[allow(clippy::type_complexity)]
    pub fn pre_state_with<'a>(
        &self,
        parent: &'a Executed,
        version: u32,
        proofs: &[aether_light::block::ProofClaim],
        answers: &[aether_light::block::BeaconAnswer],
        registrations: &[aether_light::block::NodeRegistration],
        seed: Option<&aether_light::block::Seed>,
        certified: bool,
    ) -> Result<(std::borrow::Cow<'a, WorldState>, Vec<(u64, Address, U256)>), ChainError> {
        let (running, migrate, verifier, history_v2, chain_id) = {
            let g = self.lock();
            (g.protocol, g.migrate, g.verifier.clone(), g.cfg.history_v2, g.cfg.chain_id)
        };
        let want = parent.next_protocol();
        if version != want {
            return Err(ChainError::Protocol(format!(
                "block under protocol {version}, scheduled {want}"
            )));
        }
        if version > running {
            return Err(ChainError::Protocol(format!(
                "UPGRADE REQUIRED: the chain runs protocol {version}, this node {running}"
            )));
        }
        let before = crate::upgrade::protocol_at(&parent.schedule, parent.height);
        if version < 2 && !proofs.is_empty() {
            return Err(ChainError::Protocol("proofs before protocol 2".into()));
        }
        let records = version >= 2 && parent.statement != Statement::default();
        let rotates = parent
            .schedule
            .iter()
            .any(|a| a.at == parent.height + 1 && a.registrar.is_some());
        // The record of the block that just left the claim window goes (so state stays bounded).
        let expired = (parent.height + 1).checked_sub(aether_execution::proofs::EXPIRY + 1);
        // Under history v2 empty blocks record nothing, so pruning cannot wait for a record.
        let stale = history_v2
            && expired.is_some_and(|old| aether_execution::proofs::recorded(&parent.state, old));
        let distributes = aether_rewards::distributes(&parent.state, parent.height + 1);
        let slots = aether_rewards::beacons::touches(&parent.state, parent.height + 1);
        // The node-rewards words that keep the committee in state (finding 2):
        // the draw's pool freezes at its first block, the running committee
        // rewrites at each switch, the decided next roster commits with a draw
        // seed, a reserve reseat or an early replacement, and the overdue count
        // ticks at every epoch.
        let params = aether_execution::registry::params(&parent.state);
        let span = params.epoch_blocks * params.draw_epochs;
        let rewards_on = aether_rewards::enabled(&parent.state);
        let freezes =
            rewards_on && parent.height > 0 && (parent.height + 1).is_multiple_of(span);
        // A registry epoch's first block: where the voting-set rules the draw
        // does not own (early replacement, the reserve seating, the overdue
        // count) decide, from the state this block's distribution just wrote.
        let boundary = rewards_on
            && parent.height > 0
            && (parent.height + 1).is_multiple_of(params.epoch_blocks);
        let epochs = boundary && aether_rewards::reserve(&parent.state).is_some();
        // A committee takes over here: its members become the recorded running
        // committee, and the seating of the founder's reserve keys becomes
        // state (read below, after `distribute`).
        let switches = rewards_on
            && parent.handoff.as_ref().is_some_and(|p| p.switch == parent.height + 1);
        if !answers.is_empty() && !aether_rewards::enabled(&parent.state) {
            return Err(ChainError::Protocol("beacon answers without node rewards".into()));
        }
        if !registrations.is_empty() && !aether_rewards::enabled(&parent.state) {
            return Err(ChainError::Protocol("free registrations without node rewards".into()));
        }
        if version == before
            && (version < crate::upgrade::RESERVE_FLOOR_PROTOCOL || seed.is_none())
            && !records
            && !rotates
            && !stale
            && !distributes
            && !slots
            && !switches
            && !freezes
            && !boundary
            && proofs.is_empty()
            && answers.is_empty()
            && registrations.is_empty()
        {
            return Ok((std::borrow::Cow::Borrowed(&parent.state), vec![]));
        }
        let mut state = parent.state.clone();
        // Its journal then holds only these system writes (see `with_activation`).
        state.clear_journal();
        for p in before + 1..=version {
            migrate(p, &mut state)
                .map_err(|e| ChainError::Protocol(format!("activating protocol {p}: {e}")))?;
        }
        // A committee-signed upgrade may replace the registrar key when it activates.
        for (x, y) in parent
            .schedule
            .iter()
            .filter(|a| a.at == parent.height + 1)
            .filter_map(|a| a.registrar)
        {
            aether_execution::registry::set_registrar(&mut state, (x.0, y.0));
        }
        if records {
            aether_execution::proofs::record(
                &mut state,
                parent.height,
                parent.statement.commitment,
                parent.statement.escrow,
            );
        }
        if records || stale {
            if let Some(old) = expired {
                aether_execution::proofs::prune(&mut state, old);
            }
        }
        // A draw's pool is frozen from the state its first block builds on
        // (before its seed exists), the same list `finalize` used to freeze
        // node-locally: now every node holds it in state.
        if freezes && !aether_rewards::registry_v3::is_v3(&state) {
            let pool = crate::rotation::eligible(
                &parent.state,
                (parent.height + 1) / params.epoch_blocks,
                params.min_streak,
            );
            aether_rewards::freeze_pool(&mut state, (parent.height + 1) / span, &pool)
                .map_err(|e| ChainError::Exec(format!("draw pool: {e}")))?;
        }
        struct Certified;
        impl ProofVerifier for Certified {
            fn verify(&self, _: &[u8], _: [u8; 32]) -> bool {
                true
            }
        }
        let verifier: Option<&dyn ProofVerifier> = if certified {
            Some(&Certified)
        } else {
            verifier.as_deref()
        };
        let mut payouts = Vec::new();
        if distributes {
            // The last epoch's node rewards, credited to operators directly (no claim).
            let d = aether_rewards::distribute(&mut state, parent.height + 1)
                .map_err(|e| ChainError::Exec(format!("node rewards: {e}")))?;
            payouts.extend(
                d.paid
                    .into_iter()
                    .filter(|(_, a)| !a.is_zero())
                    .map(|(op, a)| (parent.height + 1, op, a)),
            );
            // The epoch opening here gets its randomness word: the latest draw
            // seed on chain (a committee threshold signature — no single
            // proposer can bias it), published before the epoch began. It is
            // predictable before its epoch, so contracts commit before the
            // draw seed is published and reveal after the epoch opens.
            if history_v2 {
                if let Some((_, s)) = parent.seed.as_deref() {
                    let epoch_blocks = aether_execution::registry::epoch_blocks(&state);
                    let signature = hex::decode(&s.signature)
                        .map_err(|_| ChainError::Protocol("invalid recorded draw seed".into()))?;
                    aether_rewards::set_randomness(&mut state, (parent.height + 1) / epoch_blocks, s.draw, &signature);
                }
            }
        }
        if freezes && aether_rewards::registry_v3::is_v3(&state) {
            let epoch = (parent.height + 1) / params.epoch_blocks;
            let pool = crate::rotation::eligible(&state, epoch, params.min_streak);
            aether_rewards::freeze_pool(&mut state, (parent.height + 1) / span, &pool)
                .map_err(|e| ChainError::Exec(format!("draw pool: {e}")))?;
        }
        // The committee taking over here is the chain's record of itself: the
        // running committee word becomes the handoff's members (what every
        // later draw and reserve reseat derives from), and the founder's
        // reserve keys seated or unseated become state (the seating itself
        // lives in the node). After `distribute`, which pays the epoch by the
        // committee that ran it; the next epoch's `distribute` then reads this
        // word. The roster it committed has served its purpose.
        if switches {
            let pending = parent.handoff.as_ref().expect("a handoff switches here");
            aether_rewards::set_committee(&mut state, &pending.handoff.members)
                .map_err(|e| ChainError::BadHandoff(format!("switch: {e}")))?;
            aether_rewards::switch_reserve(&mut state, parent.height + 1, &pending.handoff.members);
            aether_rewards::clear_roster(&mut state);
        }
        // With the draw's seed on chain, every node draws the same next voting
        // set — and commits it: only a handoff naming exactly this roster may
        // carry the committee over (`next_handoff`). The same computation
        // `finalize` runs node-locally on the other networks, on the same
        // inputs — the profiles this block's distribution wrote above, the
        // running committee from state — so every node commits the same roster.
        if let Some(s) = seed {
            let draw = current_draw(&parent.state, parent.height + 1);
            if s.draw == draw
                && draw > 0
                && (version < crate::upgrade::RESERVE_FLOOR_PROTOCOL
                    || parent.handoff.as_ref().is_none_or(|p| p.switch <= parent.height + 1))
                && parent.seed.as_ref().is_none_or(|p| p.1.draw < draw)
                && (!aether_rewards::registry_v3::is_v3(&state)
                    || aether_rewards::next_roster(&state).is_none_or(|(d, _)| d != draw))
            {
                if let Some((_, pool)) = aether_rewards::draw_pool(&state).filter(|(d, _)| *d == draw) {
                    // Protocol 4 keeps the frozen draw's membership ceiling,
                    // but a delayed seed must not reinstall a proven departing
                    // or silent key. Boundary-only `eligible` cannot be used
                    // here: honest mid-epoch answers advance `last_epoch`.
                    let mut pool = pool;
                    if version >= crate::upgrade::RESERVE_FLOOR_PROTOCOL {
                        let epoch = (parent.height + 1) / params.epoch_blocks;
                        let recents = crate::rotation::recents(&state);
                        let departures = crate::rotation::departures(&state);
                        pool.retain(|(k, _)| !departures.contains(k)
                            && !matches!(recents.get(k).copied(), Some((e, last, prev))
                                if e.checked_add(1) == Some(epoch) && last != aether_rewards::beacons::NO_COUNT
                                    && prev != aether_rewards::beacons::NO_COUNT
                                    && last < crate::rotation::SILENT_BELOW && prev < crate::rotation::SILENT_BELOW));
                    }
                    let seed_bytes = hex::decode(&s.signature).unwrap_or_default();
                    // The per-operator seat cap is a protocol-2 rule: before it, every key is its own operator.
                    let ops = crate::rotation::operators(&parent.state);
                    let capped = version >= 2;
                    let operator = |k: &str| {
                        ops.get(k).map(|o| if capped { o.clone() } else { k.to_string() })
                    };
                    let running = crate::rotation::Committee { members: aether_rewards::committee(&state) };
                    let reserve = Reserve::of(&parent.state);
                    // Each seat goes where it lifts the committee's worst hour
                    // of the day (13-roadmap.md, F): profiles from state, so
                    // every node reads the same hours.
                    let availability = (version >= 3 || reserve.is_some())
                        .then(|| crate::rotation::availability(&state));
                    let hours = |k: &str| availability.as_ref().and_then(|a| a.get(k).copied());
                    // Protocol 3: qualifying Macs join (up to 16 seats) instead of
                    // replacing members, each where the odds need it most.
                    let repair = reserve.as_ref().filter(|_| version >= crate::upgrade::RESERVE_FLOOR_PROTOCOL
                        && parent.handoff.as_ref().is_none_or(|p| p.switch <= parent.height + 1)).and_then(|r| {
                        let recents = crate::rotation::recents(&state);
                        let departures = crate::rotation::departures(&state);
                        crate::rotation::reserve_replacement(
                            &running, &pool, &seed_bytes, |k| ops.get(k).cloned(), hours,
                            |k| recents.get(k).copied(), |k| departures.contains(k),
                            (parent.height + 1) / params.epoch_blocks, r,
                        )
                    });
                    // A repair is the final roster: the frozen pool can still
                    // contain the unavailable key, so seating it again would
                    // undo the one-for-one replacement.
                    let drawn = repair.or_else(|| {
                    let drawn = if version >= 3 {
                        crate::rotation::draw_spread_capped(&pool, &seed_bytes, operator, &running, hours, self.cfg().max_committee)
                    } else {
                        crate::rotation::draw_capped(&pool, &seed_bytes, operator, &running, self.cfg().max_committee)
                    };
                    // Founder reserve keys join or leave with the draw too.
                    match reserve {
                        Some(r) => crate::rotation::with_reserve_for(
                            version,
                            drawn,
                            &pool,
                            &seed_bytes,
                            |k: &str| ops.get(k).cloned(),
                            &r,
                            &running,
                            hours,
                        ),
                        None => drawn,
                    }
                    });
                    if let Some(members) = drawn {
                        aether_rewards::commit_roster(&mut state, draw, &members)
                            .map_err(|e| ChainError::Exec(format!("roster: {e}")))?;
                    }
                }
            }
        }
        // The voting-set rules of an epoch boundary the draw does not own (its
        // own first block leaves the set to the draw): early replacement first,
        // then the founder reserve keys' seating — the order `finalize` runs
        // them in on the other networks decides which one commits here. Each
        // recomputes the set for the epoch ahead — the frozen pool, the
        // recorded committee, the parent's digest for ticket order, the
        // profiles this block's distribution wrote — and commits it as this
        // draw's roster. The seed for this draw, or a handoff still on its way,
        // each stand in both' way, as the node-local rules they replace did.
        let departures = crate::rotation::departures(&state);
        let urgent_leave = aether_rewards::registry_v3::is_v3(&state)
            && aether_rewards::committee(&state).iter().any(|(k, _)| departures.contains(k));
        // A proven single vacancy in four seats cannot wait for the next
        // day's draw. The reserve repair still respects pending handoffs and
        // an already committed roster below.
        let reserve_replaced = if boundary && version >= crate::upgrade::RESERVE_FLOOR_PROTOCOL {
            Reserve::of(&parent.state).and_then(|reserve| {
                let running = crate::rotation::Committee { members: aether_rewards::committee(&state) };
                if running.members.len() != aether_consensus::committee::MIN_OPEN_COMMITTEE {
                    return None;
                }
                let epoch = (parent.height + 1) / params.epoch_blocks;
                let eligibility_state = if aether_rewards::registry_v3::is_v3(&state) { &state } else { &parent.state };
                let pool = crate::rotation::eligible(eligibility_state, epoch, params.min_streak);
                let ops = crate::rotation::operators(&parent.state);
                let availability = crate::rotation::availability(&state);
                let recents = crate::rotation::recents(&state);
                crate::rotation::reserve_replacement(
                    &running, &pool, &digest_bytes(&parent.digest),
                    |k| ops.get(k).cloned(), |k| availability.get(k).copied(),
                    |k| recents.get(k).copied(), |k| departures.contains(k), epoch, &reserve,
                )
            })
        } else { None };
        if boundary
            && (!freezes || ((urgent_leave || reserve_replaced.is_some()) && seed.is_none()))
            && parent.handoff.as_ref().is_none_or(|p| p.switch <= parent.height + 1)
            && aether_rewards::next_roster(&state).is_none_or(|(d, _)| d != current_draw(&parent.state, parent.height + 1))
        {
            let running = crate::rotation::Committee { members: aether_rewards::committee(&state) };
            let epoch = (parent.height + 1) / params.epoch_blocks;
            if !running.members.is_empty() {
                let eligibility_state = if aether_rewards::registry_v3::is_v3(&state) { &state } else { &parent.state };
                let pool = crate::rotation::eligible(eligibility_state, epoch, params.min_streak);
                let ops = crate::rotation::operators(&parent.state);
                let availability = crate::rotation::availability(&state);
                let recents = crate::rotation::recents(&state);
                let hours = |k: &str| availability.get(k).copied();
                // Early replacement (13-roadmap.md, F): a member silent through
                // the last two epochs hands its seat to the candidate the
                // spread rule picks, while the old quorum still stands.
                let replaced = reserve_replaced.or_else(|| crate::rotation::replace_unavailable(
                    &running,
                    &pool,
                    &digest_bytes(&parent.digest),
                    |k: &str| ops.get(k).cloned(),
                    hours,
                    |k: &str| recents.get(k).copied(),
                    |k: &str| departures.contains(k),
                    epoch,
                ));
                let members = match (replaced, Reserve::of(&parent.state)) {
                    (Some(members), _) => {
                        tracing::info!(
                            members = members.len(),
                            "a silent member is replaced while the quorum still stands"
                        );
                        Some(members)
                    }
                    // Missing seats use standby through four operators;
                    // larger committees retain the night-time survival rule.
                    (None, Some(reserve)) => crate::rotation::with_reserve_for(
                        version,
                        None,
                        &pool,
                        &digest_bytes(&parent.digest),
                        |k: &str| ops.get(k).cloned(),
                        &reserve,
                        &running,
                        hours,
                    )
                    .map(|members| {
                        tracing::info!(
                            members = members.len(),
                            "founder reserve keys change the voting set"
                        );
                        members
                    }),
                    (None, None) => None,
                };
                if let Some(members) = members {
                    aether_rewards::commit_roster(
                        &mut state,
                        current_draw(&parent.state, parent.height + 1),
                        &members,
                    )
                    .map_err(|e| ChainError::Exec(format!("roster: {e}")))?;
                }
            }
        }
        // Finding 6: count the epochs the reserve keys hold seats nobody needs
        // — four independent operators before protocol 4, five from it — and stop their
        // service credit past the grace (`rewards::reserve_served`). The count
        // this block writes is the epoch's that opens here; `distribute` above
        // read the word the boundary before it wrote.
        if epochs {
            let epoch = (parent.height + 1) / params.epoch_blocks;
            if let Some(reserve) = Reserve::of(&parent.state) {
                let eligibility_state = if aether_rewards::registry_v3::is_v3(&state) { &state } else { &parent.state };
                let pool = crate::rotation::eligible(eligibility_state, epoch, params.min_streak);
                let ops = crate::rotation::operators(&state);
                let expiry = if version >= crate::upgrade::RESERVE_FLOOR_PROTOCOL {
                    aether_rewards::RESERVE_STANDBY_MAX_OPERATORS + 1
                } else {
                    aether_consensus::committee::MIN_OPEN_COMMITTEE
                };
                let on = crate::rotation::independent(&pool, |k| ops.get(k).cloned(), &reserve)
                    >= expiry
                    && aether_rewards::seated(&state).0 > 0;
                let (_, so_far) = aether_rewards::overdue(&state);
                let count = if on { so_far + 1 } else { 0 };
                aether_rewards::set_overdue(&mut state, epoch, count);
                if count > aether_rewards::RESERVE_GRACE_EPOCHS {
                    let message = if version >= crate::upgrade::RESERVE_FLOOR_PROTOCOL {
                        "founder reserve keys still hold seats with five or more independent operators: their service credit has stopped"
                    } else {
                        "founder reserve keys still hold seats with four or more independent operators: their service credit has stopped"
                    };
                    tracing::warn!(
                        epoch,
                        count,
                        "{message}"
                    );
                }
            }
        }
        // This epoch's beacon slots and the hash of a slot's block, then the answers.
        aether_rewards::beacons::on_block(&mut state, parent.height + 1, digest_bytes(&parent.digest));
        crate::beacons::apply(&mut state, chain_id, parent.height + 1, answers)
            .map_err(|e| ChainError::Protocol(format!("beacons: {e}")))?;
        // The block's free-lane registrations, after the answers (both are
        // system writes checked against this same state).
        crate::registrations::apply(&mut state, chain_id, parent.height + 1, registrations)
            .map_err(|e| ChainError::Protocol(format!("registrations: {e}")))?;
        payouts.extend(pay_proofs(&mut state, parent.height + 1, proofs, verifier)?);
        Ok((std::borrow::Cow::Owned(state), payouts))
    }

    /// Local assignment over the finalized registry. The consensus proof
    /// market still accepts any operator's valid proof at any age.
    pub fn provable_for(
        &self,
        prover: Address,
        config: &crate::prover_assignment::Config,
        now_ms: u64,
    ) -> Option<(Arc<Executed>, Arc<Executed>, Block)> {
        // Snapshot only Arc-backed blocks/state and bounded local bookkeeping.
        // Registry reads and O(window * registry) scoring must not hold the
        // mutex used by finalization.
        let (head, recent, attempted, lost, pooled, available) = {
            let g = self.lock();
            (g.finalized.clone(), g.recent.iter().map(|b| (b.digest(), b.parent, b.height().get(), b.timestamp)).collect::<Vec<_>>(), g.attempted.clone(), g.lost_proofs.clone(),
             g.proof_pool.iter().map(|c| c.height).collect::<std::collections::HashSet<_>>(),
             g.executed.keys().copied().collect::<std::collections::HashSet<_>>())
        };
        let operators: Vec<_> = aether_execution::registry::candidates(&head.state)
            .into_iter().map(|c| c.operator).collect();
        let open: Vec<_> = recent.iter().filter_map(|&(digest, parent, h, timestamp_ms)| {
            if attempted.contains(&h) || lost.contains(&h) || pooled.contains(&h)
                || aether_execution::proofs::prover(&head.state, h).is_some()
                || (h != head.height && aether_execution::proofs::claimable(&head.state, h, head.height + 1).is_err())
                || !available.contains(&digest) || !available.contains(&parent)
            { return None; }
            Some(crate::prover_assignment::OpenBlock { height: h, timestamp_ms })
        }).collect();
        let height = crate::prover_assignment::select(&open, &operators, prover, now_ms, config)?;
        let digest = recent.iter().find(|b| b.2 == height)?.0;
        let mut g = self.lock();
        if g.finalized.digest != head.digest || g.attempted.contains(&height)
            || g.lost_proofs.contains(&height) || g.proof_pool.iter().any(|c| c.height == height)
            || aether_execution::proofs::prover(&g.finalized.state, height).is_some()
            || !g.recent.iter().any(|b| b.digest() == digest)
        { return None; }
        let block = g.recent.iter().find(|b| b.digest() == digest)?.clone();
        let pick = (g.executed.get(&block.digest())?.clone(), g.executed.get(&block.parent)?.clone(), block);
        g.attempted.insert(height);
        Some(pick)
    }

    pub fn set_prover_window(&self, window: usize) {
        let mut g = self.lock();
        g.prover_window = window.min(crate::prover_assignment::MAX_WINDOW);
        while g.recent.len() > g.prover_window { g.recent.pop_front(); }
        trim_proving_inputs(&mut g);
    }

    /// Dispatch from a compact, byte-budgeted witness. No full historical
    /// state is held for a grace rescue. Decoding and witness replay happen
    /// after releasing the consensus mutex.
    pub fn proving_input_for(
        &self,
        prover: Address,
        config: &crate::prover_assignment::Config,
        now_ms: u64,
    ) -> Result<Option<(u64, usize, aether_proving::block::BlockInput)>, (u64, String)> {
        let (head, recent, attempted, lost, pooled, inputs) = {
            let g = self.lock();
            (g.finalized.clone(), g.recent.iter().map(|b| (b.digest(), b.parent, b.height().get(), b.timestamp)).collect::<Vec<_>>(), g.attempted.clone(), g.lost_proofs.clone(),
             g.proof_pool.iter().map(|c| c.height).collect::<std::collections::HashSet<_>>(),
             g.proving_inputs.keys().copied().collect::<std::collections::HashSet<_>>())
        };
        let operators: Vec<_> = aether_execution::registry::candidates(&head.state)
            .into_iter().map(|c| c.operator).collect();
        let open: Vec<_> = recent.iter().filter_map(|&(digest, _parent, h, timestamp_ms)| {
            if attempted.contains(&h) || lost.contains(&h) || pooled.contains(&h)
                || !inputs.contains(&digest)
                || aether_execution::proofs::prover(&head.state, h).is_some()
                || (h != head.height && aether_execution::proofs::claimable(&head.state, h, head.height + 1).is_err())
            { return None; }
            Some(crate::prover_assignment::OpenBlock { height: h, timestamp_ms })
        }).collect();
        let Some(height) = crate::prover_assignment::select(&open, &operators, prover, now_ms, config) else { return Ok(None) };
        let digest = recent.iter().find(|b| b.2 == height).expect("selected block").0;
        let retained = {
            let mut g = self.lock();
            if g.finalized.digest != head.digest || g.attempted.contains(&height)
                || g.lost_proofs.contains(&height) || g.proof_pool.iter().any(|c| c.height == height)
                || aether_execution::proofs::prover(&g.finalized.state, height).is_some()
                || !g.recent.iter().any(|b| b.digest() == digest)
            { return Ok(None); }
            let Some(input) = g.proving_inputs.get(&digest).cloned() else { return Ok(None) };
            g.attempted.insert(height);
            input
        };
        let mut input: aether_proving::block::BlockInput = postcard::from_bytes(&retained.encoded)
            .map_err(|e| (height, format!("retained prover input: {e}")))?;
        input.prover = prover;
        let statement = aether_proving::block::execute(&input).map_err(|e| (height, format!("witness replay: {e:?}")))?;
        if statement.commitment() != retained.commitment {
            return Err((height, "retained witness does not restate the finalized statement".into()));
        }
        Ok(Some((height, input.txs.len(), input)))
    }

    /// Only verified pool entries or finalized state count as proof sightings.
    pub fn proof_seen(&self, height: u64) -> bool {
        let g = self.lock();
        g.lost_proofs.contains(&height)
            || g.proof_pool.iter().any(|c| c.height == height)
            || aether_execution::proofs::prover(&g.finalized.state, height).is_some()
    }

    pub(crate) fn observe_proofs(&self) -> std::sync::mpsc::Receiver<(u64, Instant)> {
        let (send, recv) = std::sync::mpsc::channel();
        self.lock().proof_observers.push(send);
        recv
    }

    pub(crate) fn set_proving_height(&self, height: Option<u64>) {
        self.lock().proving_height = height;
    }

    /// A proving attempt at `height` failed for the prover's own sake — its
    /// sidecar died or could not be talked to, not the block — so the height
    /// may be picked again once the sidecar is replaced.
    pub fn retry_proof(&self, height: u64) {
        let mut g = self.lock();
        if !g.lost_proofs.contains(&height) {
            g.attempted.remove(&height);
        }
    }

    /// Inclusion proof of block `height` under the history root of block
    /// `anchor` (which commits blocks 0..anchor), and the block's hash. Reads
    /// the era roots plus at most two eras' block hashes under the lock; the
    /// hashing happens outside it.
    pub fn history_proof(
        &self,
        height: u64,
        anchor: u64,
    ) -> Result<(aether_state::mmr::MmrProof, String), String> {
        use aether_state::mmr::ERA_LEN;
        {
            let g = self.lock();
            if anchor == 0 || height >= anchor || anchor > g.finalized.height {
                return Err("need height < anchor <= finalized height".into());
            }
        }
        let (index, eras) = self.era_leaves(&[height / ERA_LEN, anchor / ERA_LEN])?;
        let h = ChainHasher::new();
        let open = index.eras.len() as u64;
        let e = height / ERA_LEN;
        let hash = if e < open {
            eras.get(&e).map(|(_, hashes)| hashes[(height % ERA_LEN) as usize])
        } else {
            self.block_summary(height)
                .map_err(|e| e.to_string())?
                .and_then(|b| hex::decode(&b.hash).ok())
                .and_then(|d| d.try_into().ok())
        }
        .ok_or("block not kept here")?;
        let proof = index
            .prove(&h, anchor, height, |e| eras.get(&e).map(|(l, _)| l.clone()))
            .ok_or("no proof")?;
        Ok((proof, hex::encode(hash)))
    }

    /// Leaf digests and block hashes of the complete eras among `wanted`: from
    /// the kept block summaries, or from the era file once they are pruned (read
    /// back and checked against the era root this node keeps).
    #[allow(clippy::type_complexity)]
    fn era_leaves(
        &self,
        wanted: &[u64],
    ) -> Result<
        (
            Arc<aether_state::mmr::EraIndex>,
            BTreeMap<u64, (Vec<[u8; 32]>, Vec<[u8; 32]>)>,
        ),
        String,
    > {
        use aether_state::mmr::ERA_LEN;
        let h = ChainHasher::new();
        let (index, kept, from_file, from_store, store) = {
            let g = self.lock();
            let index = g
                .history_index
                .clone()
                .ok_or("this node started from a checkpoint and keeps no early history")?;
            let open = index.eras.len() as u64;
            let (mut kept, mut from_file, mut from_store) = (BTreeMap::new(), Vec::new(), Vec::new());
            for &e in wanted.iter().filter(|e| **e < open) {
                if e * ERA_LEN < g.pruned_below.max(g.cache_below) {
                    if !g.cfg.history_v2 && e * ERA_LEN >= g.pruned_below {
                        from_store.push(e);
                    } else {
                        from_file.push(e);
                    }
                    continue;
                }
                let hashes: Option<Vec<[u8; 32]>> = g
                    .blocks
                    .range(e * ERA_LEN..(e + 1) * ERA_LEN)
                    .map(|(_, b)| hex::decode(&b.hash).ok()?.try_into().ok())
                    .collect();
                kept.insert(e, hashes.ok_or("bad block hash")?);
            }
            (index, kept, from_file, from_store, g.store.clone())
        };
        let mut out = BTreeMap::new();
        for (e, hashes) in kept {
            let leaves = hashes
                .iter()
                .enumerate()
                .map(|(i, d)| aether_state::mmr::leaf(&h, e * ERA_LEN + i as u64, d))
                .collect();
            out.insert(e, (leaves, hashes));
        }
        for e in from_file {
            let era = self.load_era(store.as_deref(), e, Some(&index.eras[e as usize]))?;
            let hashes: Vec<[u8; 32]> = era.blocks.iter().map(|b| digest_bytes(&b.digest())).collect();
            let leaves = hashes
                .iter()
                .enumerate()
                .map(|(i, d)| aether_state::mmr::leaf(&h, e * ERA_LEN + i as u64, d))
                .collect();
            out.insert(e, (leaves, hashes));
        }
        for e in from_store {
            let store = store.as_ref().ok_or("no archival store")?;
            let hashes = store.block_hashes(e * ERA_LEN..(e + 1) * ERA_LEN).map_err(|e| e.to_string())?;
            let leaves: Vec<_> = hashes.iter().enumerate()
                .map(|(i, d)| aether_state::mmr::leaf(&h, e * ERA_LEN + i as u64, d)).collect();
            if aether_state::mmr::subtree_root(&h, &leaves) != index.eras[e as usize] {
                return Err("archival summaries do not match the retained era root".into());
            }
            out.insert(e, (leaves, hashes));
        }
        Ok((index, out))
    }

    /// Era `era` from this node's era file, read back and checked (against
    /// `root` when given); the last one read is cached.
    pub fn load_era(
        &self,
        store: Option<&Store>,
        era: u64,
        root: Option<&[u8; 32]>,
    ) -> Result<Arc<crate::era::Era>, String> {
        if let Some((e, cached)) = self.lock().era_cache.clone() {
            if e == era && root.is_none_or(|r| *r == cached.root) {
                return Ok(cached);
            }
        }
        let store = store.ok_or("no store")?;
        let path = store.era_dir().join(crate::era::file_name(era));
        let bytes = std::fs::read(&path)
            .map_err(|_| format!("pruned: era {era} is pruned here and its file is not kept"))?;
        let decoded = crate::era::read(&bytes, root).map_err(|e| format!("era {era} file: {e}"))?;
        if decoded.index != era {
            return Err(format!("era file {} holds era {}", path.display(), decoded.index));
        }
        let decoded = Arc::new(decoded);
        self.lock().era_cache = Some((era, decoded.clone()));
        Ok(decoded)
    }

    /// Proof that era `era`'s root is in the history under block `anchor`'s
    /// history root (`aether_light::verify_era_root`): what a peer checks an
    /// era file it fetched from this node against.
    pub fn era_proof(&self, era: u64, anchor: u64) -> Result<aether_state::mmr::MmrProof, String> {
        use aether_state::mmr::{ERA_BITS, ERA_LEN};
        {
            // `anchor` counts blocks: up to the whole finalized chain (the history
            // root the next block will commit, what a peer at the same head holds).
            // `era` is caller-supplied (`aether_eraProof`, public-allowlisted):
            // saturate the addition too — with overflow checks on, `era + 1`
            // panicked on the maximum era before the multiply ran, and the
            // process-wide hook turned an out-of-domain ask into a node exit
            // (pre-audit 7b PA7B-02).
            let g = self.lock();
            if anchor > g.finalized.height + 1 || era.saturating_add(1).saturating_mul(ERA_LEN) > anchor {
                return Err("need a complete era below anchor <= finalized height + 1".into());
            }
        }
        let (index, eras) = self.era_leaves(&[anchor / ERA_LEN])?;
        let open = index.eras.len() as u64;
        aether_state::mmr::prove_by_eras(
            &ChainHasher::new(),
            anchor,
            era * ERA_LEN,
            ERA_BITS,
            &index.eras,
            |e| {
                if e == open {
                    Some(index.open.clone())
                } else {
                    eras.get(&e).map(|(l, _)| l.clone())
                }
            },
        )
        .ok_or_else(|| "no proof".to_string())
    }

    /// Pruning (roadmap B4): drop the block summaries, receipts and finality
    /// proofs below `cutoff` (an era boundary at or below the head), on disk
    /// first and then in memory. The roots of the eras below it stay, so
    /// history proofs keep working (from the era files while they are kept).
    pub fn prune(&self, cutoff: u64) -> Result<crate::store::PruneReport, String> {
        use aether_state::mmr::ERA_LEN;
        let (store, roots) = {
            let g = self.lock();
            if cutoff <= g.pruned_below {
                return Ok(Default::default());
            }
            if !cutoff.is_multiple_of(ERA_LEN) || cutoff > g.finalized.height {
                return Err(format!("cannot prune below {cutoff}: not an era boundary at or below the head"));
            }
            let roots: Vec<(u64, [u8; 32])> = match &g.history_index {
                Some(idx) => (g.pruned_below / ERA_LEN..cutoff / ERA_LEN)
                    .filter_map(|e| idx.eras.get(e as usize).map(|r| (e, *r)))
                    .collect(),
                None => Vec::new(),
            };
            (g.store.clone(), roots)
        };
        let report = match &store {
            Some(s) => s.prune_below(cutoff, &roots).map_err(|e| e.to_string())?,
            None => Default::default(),
        };
        let mut g = self.lock();
        g.blocks = g.blocks.split_off(&cutoff);
        g.receipts.retain(|_, (h, _)| *h >= cutoff);
        g.pruned_below = cutoff;
        g.caches_bytes = caches_bytes_of(&g.blocks, &g.receipts);
        Ok(report)
    }

    /// Trim history under the monitor's memory budget. Durable legacy chains
    /// also have a 64 MiB ceiling when no monitor is installed.
    pub fn trim_history_caches(&self) {
        let mut g = self.lock();
        let budget = g.history_budget;
        trim_caches(&mut g, budget);
    }

    /// A finalized summary, using durable rows on a cache miss. Disk I/O is
    /// outside the chain lock and reads never repopulate the history cache.
    pub fn block_summary(&self, height: u64) -> Result<Option<BlockSummary>, StoreError> {
        let store = {
            let g = self.lock();
            if let Some(summary) = g.blocks.get(&height) {
                return Ok(Some(summary.clone()));
            }
            g.store.clone()
        };
        match store {
            Some(store) => store.block_summary(height),
            None => Ok(None),
        }
    }

    /// A finalized receipt after eviction, without growing the cache.
    pub fn receipt(&self, hash: &TxHash) -> Result<Option<(u64, Receipt)>, StoreError> {
        let store = {
            let g = self.lock();
            if let Some(receipt) = g.receipts.get(hash) {
                return Ok(Some(receipt.clone()));
            }
            // A tiny budget may evict the pseudo-receipt map entry, but the
            // finalized head's bounded ids still prove its confirmation.
            if g.finalized.registration_ids.contains(hash) {
                return Ok(Some((g.finalized.height, registration_receipt(*hash))));
            }
            g.store.clone()
        };
        match store {
            Some(store) => store.receipt(hash),
            None => Ok(None),
        }
    }

    /// Finalized summaries in a caller-bounded inclusive range. One read
    /// transaction backs archived ranges; memory-only chains use their maps.
    pub fn block_summaries(&self, from: u64, to: u64) -> Result<Vec<BlockSummary>, StoreError> {
        if from > to {
            return Ok(Vec::new());
        }
        let store = {
            let g = self.lock();
            match &g.store {
                Some(store) => store.clone(),
                None => return Ok(g.blocks.range(from..=to).map(|(_, b)| b.clone()).collect()),
            }
        };
        store.block_summaries(from..=to)
    }

    /// The newest finalized summaries, newest first, including evicted rows.
    pub fn recent_block_summaries(&self, limit: usize) -> Result<Vec<BlockSummary>, StoreError> {
        let limit = limit.min(100);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let head = self.finalized_height();
        let mut summaries = self.block_summaries(head.saturating_sub(limit as u64 - 1), head)?;
        summaries.reverse();
        Ok(summaries)
    }

    /// Estimated history bytes, including full states and compact proving inputs.
    pub fn caches_bytes(&self) -> u64 {
        retained_bytes(&self.lock())
    }

    /// Estimated bytes of cached summaries and receipts only. Full execution
    /// states and compact proving inputs are included by `caches_bytes`.
    pub fn history_rows_bytes(&self) -> u64 {
        self.lock().caches_bytes
    }

    /// The trim with an explicit budget (tests).
    pub fn trim_history_caches_with(&self, budget: u64) {
        trim_caches(&mut self.lock(), budget);
    }

    /// Finalized block `height` read back from its era file (pruned heights).
    pub fn old_block(&self, height: u64) -> Result<Block, String> {
        use aether_state::mmr::ERA_LEN;
        let (store, root) = {
            let g = self.lock();
            let e = height / ERA_LEN;
            (g.store.clone(), g.history_index.as_ref().and_then(|i| i.eras.get(e as usize).copied()))
        };
        let era = self.load_era(store.as_deref(), height / ERA_LEN, root.as_ref())?;
        Ok(era.blocks[(height % ERA_LEN) as usize].clone())
    }

    /// Rewards `prover` received for proofs (this node's record since it started keeping one).
    pub fn rewards(&self, prover: &Address) -> Vec<Value> {
        self.recent_rewards(prover, usize::MAX)
    }

    /// The newest `limit` rewards of `prover`, oldest first.
    pub fn recent_rewards(&self, prover: &Address, limit: usize) -> Vec<Value> {
        // Read the store without holding the chain lock (a long history must not stall consensus).
        let store = self.lock().store.clone();
        let rows = store
            .and_then(|s| s.rewards(&prover.0 .0, limit).ok())
            .unwrap_or_default();
        rows.iter()
            .filter_map(|r| serde_json::from_slice(r).ok())
            .collect()
    }

    /// One page of `prover`'s reward records, newest first, with the cursor
    /// that continues older and how many exist in total (`aether_rewardsPage`).
    pub fn rewards_page(&self, prover: &Address, before: Option<&str>, limit: usize) -> Result<Value, String> {
        let store = self.lock().store.clone().ok_or("this node keeps no reward records")?;
        let (rows, next_cursor, total) = store.rewards_page(&prover.0 .0, before, limit).map_err(|e| e.to_string())?;
        Ok(json!({
            "rewards": rows.iter().filter_map(|r| serde_json::from_slice::<Value>(r).ok()).collect::<Vec<_>>(),
            "next_cursor": next_cursor,
            "total": total,
        }))
    }

    /// Node-sourced account activity. This is a display index, not a proof of
    /// account balance; callers continue to verify balances separately.
    pub fn account_history(&self, address: &Address, before: Option<&str>, limit: usize) -> Result<crate::account_history::Page, String> {
        let store = self.lock().store.clone().ok_or("this node has no finalized history store")?;
        store.account_history(&address.0 .0, before, limit).map_err(|e| e.to_string())
    }

    /// Keep a proof (verified by this node's verifier) for this node's next proposals.
    pub fn add_proof(&self, claim: aether_light::block::ProofClaim) -> Result<(), String> {
        self.add_proof_from(claim, false)
    }

    /// A proof this node's own prover made: no rate limit (nobody else can use this path).
    pub fn add_own_proof(&self, claim: aether_light::block::ProofClaim) -> Result<(), String> {
        self.add_proof_from(claim, true)
    }

    fn add_proof_from(
        &self,
        claim: aether_light::block::ProofClaim,
        own: bool,
    ) -> Result<(), String> {
        let open = |g: &Inner, h: u64| {
            let f = &g.finalized;
            aether_execution::proofs::claimable(&f.state, h, f.height + 1)
                .or_else(|e| {
                    if h == f.height && aether_execution::proofs::prover(&f.state, h).is_none() {
                        Ok(f.statement.commitment)
                    } else {
                        Err(e)
                    }
                })
                .map_err(|e| format!("{e:?}"))
        };
        let (verifier, commitment) = {
            let mut g = self.lock();
            // Cheap refusals first: a block already covered, or verifying too often (this is a public RPC).
            if g.proof_pool.iter().any(|c| c.height == claim.height) {
                return Err("a proof of this block is already waiting".into());
            }
            // Malformed submissions never take a verification slot.
            if !(2 * MIN_PROOF_BYTES..=2 * MAX_PROOF_BYTES).contains(&claim.proof.len())
                || !claim.proof.len().is_multiple_of(2)
                || !claim.proof.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("the proof is not a hex string of an allowed size".into());
            }
            let now = Instant::now();
            if !own
                && g.last_proof_check
                    .is_some_and(|t| now.duration_since(t) < PROOF_CHECK_INTERVAL)
            {
                return Err("busy; try again shortly".into());
            }
            let c = open(&g, claim.height)?;
            if !own {
                g.last_proof_check = Some(now);
            }
            (
                g.verifier
                    .clone()
                    .ok_or("this node does not verify proofs")?,
                c,
            )
        };
        let bytes = hex::decode(&claim.proof).map_err(|_| "proof is not hex")?;
        let output = aether_proving::block::claim(commitment, claim.prover);
        let key = *blake3::Hasher::new()
            .update(&output)
            .update(&bytes)
            .finalize()
            .as_bytes();
        if self.lock().rejected.contains(&key) {
            return Err("this proof was already refused".into());
        }
        match (bytes.len() <= MAX_PROOF_BYTES)
            .then(|| verifier.decide(&bytes, output))
            .flatten()
        {
            Some(true) => {}
            Some(false) => {
                // A definite refusal is remembered for the public RPC only (block proofs are always checked afresh).
                let mut g = self.lock();
                if g.rejected.len() > 4096 {
                    g.rejected.clear();
                }
                g.rejected.insert(key);
                return Err("the proof does not verify".into());
            }
            None => return Err("the proof could not be checked now; try again".into()),
        }
        let mut g = self.lock();
        // Still open after the (slow) check: not proven or expired meanwhile.
        open(&g, claim.height)?;
        if !g.proof_pool.iter().any(|c| c.height == claim.height) {
            g.notice_proof(claim.height);
            g.proof_pool.push(claim);
        }
        Ok(())
    }

    /// Verified proofs this node may put in the block after `parent` (first come,
    /// at most two, still claimable there), unless its recent proposals with
    /// proofs did not make it (some validators could not verify them).
    pub fn proofs_for(&self, parent: &Executed) -> Vec<aether_light::block::ProofClaim> {
        if parent.next_protocol() < 2 {
            return vec![];
        }
        let g = self.lock();
        if g.proof_backoff_until > parent.height {
            return vec![];
        }
        let next = parent.height + 1;
        let open = |h: u64| {
            aether_execution::proofs::prover(&parent.state, h).is_none()
                && (h == parent.height
                    || aether_execution::proofs::claimable(&parent.state, h, next).is_ok())
        };
        let mut seen = std::collections::BTreeSet::new();
        g.proof_pool
            .iter()
            .filter(|c| open(c.height) && seen.insert(c.height))
            .take(MAX_PROOFS_PER_BLOCK)
            .cloned()
            .collect()
    }

    /// A proposal carrying proofs at `height` was built; if a different block
    /// is finalized there, stop proposing proofs for a while.
    pub fn proposed_with_proofs(&self, height: u64, digest: Digest) {
        self.lock().proof_proposal = Some((height, digest));
    }

    /// Whether block `height` can still be proven (recorded or the head, not proven, not expired).
    pub fn proof_open(&self, height: u64) -> bool {
        let g = self.lock();
        let f = &g.finalized;
        aether_execution::proofs::prover(&f.state, height).is_none()
            && (height == f.height
                || aether_execution::proofs::claimable(&f.state, height, f.height + 1).is_ok())
    }

    /// Drop these proofs from the pool (they no longer verify here).
    pub fn drop_proofs(&self, heights: &[u64]) {
        self.lock()
            .proof_pool
            .retain(|c| !heights.contains(&c.height));
    }

    /// Take a beacon answer into the pool if it is valid for the next block on
    /// the finalized head. Ok(true) when it is new here.
    pub fn add_beacon(&self, a: aether_light::block::BeaconAnswer) -> Result<bool, String> {
        let (state, height, chain_id) = {
            let g = self.lock();
            let f = &g.finalized;
            // The next block's slot writes first: the hash of a slot's block lands there.
            let view = crate::beacons::next_view(&f.state, f.height + 1, digest_bytes(&f.digest));
            (view, f.height + 1, g.cfg.chain_id)
        };
        let checked = crate::beacons::verify(&state, chain_id, height, &a)?;
        let key = (checked.due.epoch, checked.due.slot, a.index);
        let mut g = self.lock();
        if g.beacon_pool.contains_key(&key) {
            return Ok(false);
        }
        if g.beacon_pool.len() >= 16 * aether_rewards::beacons::MAX_ANSWERS_PER_BLOCK {
            return Err("beacon pool full".into());
        }
        g.beacon_pool.insert(key, a);
        Ok(true)
    }

    /// `add_beacon`, then send a new answer on to the other validators.
    pub fn submit_beacon(&self, a: aether_light::block::BeaconAnswer) -> Result<bool, String> {
        let new = self.add_beacon(a.clone())?;
        if new {
            if let Some(out) = self.lock().beacon_out.as_ref() {
                let _ = out.send(a);
            }
        }
        Ok(new)
    }

    /// Pooled answers valid in the block after `parent`, one per (Mac, slot).
    pub fn beacons_for(&self, parent: &Executed) -> Vec<aether_light::block::BeaconAnswer> {
        if !aether_rewards::enabled(&parent.state) {
            return vec![];
        }
        let (pool, chain_id): (Vec<_>, u64) = {
            let g = self.lock();
            (g.beacon_pool.values().cloned().collect(), g.cfg.chain_id)
        };
        // Check them on the parent with this block's slot writes applied (a slot's hash lands here).
        let height = parent.height + 1;
        let mut state = crate::beacons::next_view(&parent.state, height, digest_bytes(&parent.digest));
        let mut out = Vec::new();
        for a in pool {
            if out.len() == aether_rewards::beacons::MAX_ANSWERS_PER_BLOCK {
                break;
            }
            if let Ok(c) = crate::beacons::verify(&state, chain_id, height, &a) {
                aether_rewards::beacons::record(&mut state, &c.candidate, &c.due, c.attested);
                out.push(a);
            }
        }
        out
    }

    /// Take a free-lane registration into the pool if it is valid for the next
    /// block on the finalized head (a later one for the same operator replaces
    /// an unconfirmed earlier item: its nonce is only spent on chain).
    /// Ok(true) when it is new here.
    pub fn add_registration(&self, r: aether_light::block::NodeRegistration) -> Result<bool, String> {
        let (state, height, chain_id) = {
            let g = self.lock();
            let f = &g.finalized;
            (f.state.clone(), f.height + 1, g.cfg.chain_id)
        };
        crate::registrations::verify(&state, chain_id, height, &r)?;
        let mut g = self.lock();
        if g.registration_pool.len() >= aether_execution::registry::MAX_PER_EPOCH as usize {
            return Err("registration pool full".into());
        }
        let new = g.registration_pool.insert(r.operator, r).is_none();
        Ok(new)
    }

    /// `add_registration`, then send a new item on to the other validators.
    pub fn submit_registration(&self, r: aether_light::block::NodeRegistration) -> Result<bool, String> {
        let new = self.add_registration(r.clone())?;
        if new {
            if let Some(out) = self.lock().registration_out.as_ref() {
                let _ = out.send(r);
            }
        }
        Ok(new)
    }

    /// Pooled registrations valid in the block after `parent`, checked and
    /// recorded on a copy of the parent's state so several items line up in
    /// nonce order, at most a block's worth.
    pub fn registrations_for(&self, parent: &Executed) -> Vec<aether_light::block::NodeRegistration> {
        if !aether_rewards::enabled(&parent.state) {
            return vec![];
        }
        let (pool, chain_id): (Vec<_>, u64) = {
            let g = self.lock();
            (g.registration_pool.values().cloned().collect(), g.cfg.chain_id)
        };
        let height = parent.height + 1;
        let mut state = parent.state.clone();
        let mut out = Vec::new();
        for r in pool {
            if out.len() == aether_execution::registry::MAX_FREE_PER_BLOCK {
                break;
            }
            if crate::registrations::apply(&mut state, chain_id, height, std::slice::from_ref(&r)).is_ok() {
                out.push(r);
            }
        }
        out
    }

    /// Blocks' notice between an upgrade landing on chain and its activation:
    /// one voting-node epoch, so every running Mac sees it well before (the
    /// genesis epoch length on a chain without a registry).
    fn notice(state: &WorldState, genesis_epoch: u64) -> u64 {
        if state.code(&aether_execution::registry::REGISTRY).is_empty() && genesis_epoch > 0 {
            return genesis_epoch;
        }
        aether_execution::registry::params(state).epoch_blocks
    }

    /// Whether `u` may go on chain in the block after `parent`: signed for this
    /// chain, a newer protocol than any scheduled, activating after the last one
    /// and at least `notice` blocks later, and of bounded size.
    fn admissible_upgrade(
        parent: &Executed,
        cfg: &ChainConfig,
        signed: &crate::upgrade::SignedUpgrade,
    ) -> Result<(), String> {
        let u = &signed.upgrade;
        // Protocol-1 nodes cannot read a registrar change: it may only be announced under protocol 2.
        if u.registrar.is_some() && parent.next_protocol() < 2 {
            return Err("a registrar change needs protocol 2".into());
        }
        let chain_id = cfg.chain_id;
        let mainnet_rules = cfg.node_rewards || cfg.history_v2;
        if mainnet_rules && parent.schedule.iter().filter(|a| a.at > parent.height).count() >= 16 {
            return Err("too many pending upgrades".into());
        }
        if u.emergency && !mainnet_rules {
            return Err("emergency upgrades require new-genesis rules".into());
        }
        if !u.emergency && !signed.emergency_approvals.is_empty() {
            return Err("ordinary upgrade carries emergency approvals".into());
        }
        let notice = if mainnet_rules && !u.emergency {
            // Drill builds (dev-drill feature) may shorten this through
            // AETHER_DEV_UPGRADE_NOTICE; shipped builds always get None here.
            crate::upgrade::dev_notice().unwrap_or(crate::upgrade::MAINNET_NOTICE_BLOCKS)
        } else {
            Self::notice(&parent.state, cfg.epoch_blocks)
        };
        use crate::upgrade::{MAX_FIELD, MAX_RELEASES};
        let height = parent.height + 1;
        let (last_protocol, last_at) = parent
            .schedule
            .last()
            .map(|a| (a.protocol, a.at))
            .unwrap_or((1, 0));
        if u.chain_id != chain_id {
            return Err("upgrade for another chain".into());
        }
        if u.protocol <= last_protocol || u.activate_at <= last_at {
            return Err(format!(
                "protocol {} at {} is not after {last_protocol} at {last_at}",
                u.protocol, u.activate_at
            ));
        }
        if u.activate_at < height.saturating_add(notice) {
            return Err(format!(
                "activation at {} gives less than {notice} blocks of notice",
                u.activate_at
            ));
        }
        if u.emergency {
            crate::upgrade::verify_emergency(signed, &aether_rewards::committee(&parent.state))?;
        }
        let long = |s: &String| s.len() > MAX_FIELD;
        if u.releases.len() > MAX_RELEASES
            || long(&u.notes)
            || u.releases
                .iter()
                .any(|r| long(&r.platform) || long(&r.version) || long(&r.blake3) || long(&r.url))
        {
            return Err("upgrade too large".into());
        }
        Ok(())
    }

    /// The activation schedule after a block at `height`: the parent's, plus the
    /// upgrade it carries (checked and verified under the committee identity).
    fn next_schedule(
        &self,
        height: u64,
        parent: &Executed,
        carried: Option<&crate::upgrade::SignedUpgrade>,
    ) -> Result<Arc<crate::upgrade::Schedule>, ChainError> {
        let Some(s) = carried else {
            return Ok(parent.schedule.clone());
        };
        debug_assert_eq!(height, parent.height + 1);
        let (identity, cfg) = {
            let g = self.lock();
            (g.identity, g.cfg.clone())
        };
        Self::admissible_upgrade(parent, &cfg, s).map_err(ChainError::Protocol)?;
        let identity = identity.ok_or_else(|| {
            ChainError::Protocol("no committee identity (devnet dealer keys)".into())
        })?;
        crate::upgrade::verify(&identity, s).map_err(ChainError::Protocol)?;
        Ok(scheduled(&parent.schedule, &s.upgrade))
    }

    /// The upgrade to put in a block built on `parent`: the first known one not
    /// yet on chain that may go there now.
    pub fn upgrade_for(&self, parent: &Executed) -> Option<crate::upgrade::SignedUpgrade> {
        let g = self.lock();
        g.upgrades_known
            .iter()
            .find(|s| Self::admissible_upgrade(parent, &g.cfg, s).is_ok())
            .cloned()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn remember(
        &self,
        block: &Block,
        parent: &Executed,
        ctx: &BlockContext,
        out: BlockOutcome,
        tx_hashes: Vec<TxHash>,
        handoff: Option<Arc<crate::handoff::Pending>>,
        seed: Option<Arc<(u64, aether_light::block::Seed)>>,
        schedule: Arc<crate::upgrade::Schedule>,
        statement: [u8; 32],
        payouts: Vec<(u64, Address, U256)>,
        registration_ids: Vec<TxHash>,
    ) -> Arc<Executed> {
        let escrow = out.settlement.to_escrow;
        let (base_fee, mut excess) = match &ctx.fees {
            Some(f) => (
                f.base,
                fees::next_excess(parent.excess, out.gas, ctx.limits),
            ),
            None => (FeeVector::default(), GasVector::default()),
        };
        // State growth is charged and bounded even on a new-genesis devnet
        // with execution/proving fees disabled. Legacy metadata stays zero.
        if ctx.limits.state <= fees::MAX_STATE_UNITS_PER_BLOCK {
            excess.state = fees::next_state_excess(parent.excess.state, out.gas.state);
        }
        let exec = Arc::new(Executed {
            height: block.height().get(),
            digest: block.digest(),
            timestamp: block.timestamp,
            state: out.state,
            receipts: out.receipts,
            tx_hashes,
            gas: out.gas,
            new_slots: out.new_slots,
            persistent_bytes: out.persistent_bytes,
            settlement: out.settlement,
            proposer: leader_address(&block.context.leader),
            base_fee,
            excess,
            archive_excess: if ctx.limits.state <= fees::MAX_STATE_UNITS_PER_BLOCK {
                fees::next_archive_excess(parent.archive_excess, block.data.len() as u64)
            } else { 0 },
            handoff,
            seed,
            history: Arc::new(parent.history.append(
                &ChainHasher::new(),
                block.height().get(),
                &digest_bytes(&block.digest()),
            )),
            schedule,
            statement: if statement == [0; 32] {
                Statement::default()
            } else {
                Statement {
                    commitment: statement,
                    escrow,
                }
            },
            payouts,
            registration_ids,
        });
        let bytes = execution_bytes(&exec);
        let mut g = self.lock();
        g.executed.insert(block.digest(), exec.clone());
        g.execution_sizes.insert(block.digest(), bytes);
        exec
    }

    /// Candidate txs for a proposal: inclusion-list txs (each with the sender's
    /// earlier-nonce chain that must land before it), then the rest of the
    /// mempool (lowest nonce first per sender).
    pub fn mempool_candidates(&self) -> Vec<TxEnvelope> {
        let g = self.lock();
        let censored = |t: &TxEnvelope| {
            g.censor == Some(t.header.sender) || g.deprioritize == Some(t.header.sender)
        };
        let listed = if g.censor.is_some() {
            Vec::new()
        } else {
            g.inclusion.for_proposal()
        };
        let listed_hashes: std::collections::HashSet<TxHash> =
            listed.iter().map(aether_execution::tx_hash).collect();
        let rest: Vec<TxEnvelope> = g
            .mempool
            .iter()
            .filter(|(h, t)| !listed_hashes.contains(*h) && !censored(t))
            .map(|(_, t)| t.clone())
            .collect();
        // Listed txs ride with their senders' earlier nonces ahead of everything
        // else. One nonce order over the whole pool (the 2026-09-29 stall fix)
        // let a listed tx at a later nonce be pushed past the block's tx budget
        // by a flood of unrelated nonce-0 txs — block after block, without
        // anyone censoring anything (2026-09-29 red-team 5). The chain that
        // must land first goes first, so a listed tx needs only its own
        // sender's earlier nonces, never room past two thousand strangers.
        let mut wanted: HashMap<Address, u64> = HashMap::with_capacity(listed.len());
        for t in &listed {
            wanted
                .entry(t.header.sender)
                .and_modify(|top| *top = (*top).max(t.header.nonce))
                .or_insert(t.header.nonce);
        }
        let mut head: Vec<(bool, TxEnvelope)> = Vec::new();
        let mut tail: Vec<(bool, TxEnvelope)> = Vec::new();
        for t in rest {
            match wanted.get(&t.header.sender) {
                Some(top) if t.header.nonce <= *top => head.push((false, t)),
                _ => tail.push((false, t)),
            }
        }
        head.extend(listed.into_iter().map(|t| (true, t)));
        // Within the head (and between senders in it) nonce order still rules:
        // a listed tx whose sender's earlier nonces sit in the mempool would
        // fail up front, never be retried, and the block would then wrongly
        // leave it out — the original stall. For equal nonces a listed tx
        // still comes first.
        head.sort_by_key(|(listed, t)| (t.header.nonce, !*listed, t.header.sender));
        tail.sort_by_key(|(_, t)| (t.header.nonce, t.header.sender));
        head.extend(tail);
        let mut txs: Vec<TxEnvelope> = head.into_iter().map(|(_, t)| t).collect();
        txs.truncate(MAX_TXS_PER_BLOCK);
        txs
    }

    /// The oldest mempool txs waiting at least `min_age`, for this node's inclusion list.
    pub fn inclusion_candidates(&self, min_age: Duration, now: Instant) -> Vec<TxEnvelope> {
        let g = self.lock();
        let mut waiting: Vec<(Instant, TxHash)> = g
            .arrivals
            .iter()
            .filter(|(h, t)| {
                now.saturating_duration_since(**t) >= min_age && g.mempool.contains_key(*h)
            })
            .map(|(h, t)| (*t, *h))
            .collect();
        waiting.sort();
        waiting
            .into_iter()
            .take(inclusion::MAX_IL_TXS)
            .filter_map(|(_, h)| g.mempool.get(&h).cloned())
            .collect()
    }

    /// Listed txs that `exec` (a verified block) wrongly left out. Empty for a
    /// censoring devnet node, which ignores the lists.
    pub fn inclusion_violations(
        &self,
        exec: &Executed,
        ctx: &BlockContext,
        now: Instant,
    ) -> Vec<TxHash> {
        let listed: Vec<TxEnvelope> = {
            let g = self.lock();
            if g.censor.is_some() {
                return Vec::new();
            }
            // Another group's tx is not this proposer's to include (the chain
            // rejects it), so leaving it out is not censoring.
            g.inclusion.enforceable(now).into_iter().filter(|t| t.header.group() == g.cfg.group).collect()
        };
        let full = exec.tx_hashes.len() >= MAX_TXS_PER_BLOCK;
        inclusion::violations(&listed, &exec.tx_hashes, full, &exec.state, ctx, exec.gas, exec.new_slots, exec.persistent_bytes)
    }

    /// Archive-aware FOCIL append check. Execute only the prospective listed
    /// transaction and merge the exact BAL/gas/receipt commitment so all its
    /// encoded bytes must fit the same certified archive capacity.
    pub fn inclusion_violations_in_payload(
        &self,
        exec: &Executed,
        ctx: &BlockContext,
        now: Instant,
        parent: &Executed,
        payload: &Payload,
    ) -> Vec<TxHash> {
        if ctx.limits.state > fees::MAX_STATE_UNITS_PER_BLOCK {
            return self.inclusion_violations(exec, ctx, now);
        }
        let listed = {
            let g = self.lock();
            if g.censor.is_some() || exec.tx_hashes.len() >= MAX_TXS_PER_BLOCK {
                return vec![];
            }
            let present: std::collections::HashSet<_> = exec.tx_hashes.iter().collect();
            g.inclusion.enforceable(now).into_iter()
                .filter(|tx| tx.header.group() == g.cfg.group
                    && !present.contains(&aether_execution::tx_hash(tx)))
                .collect::<Vec<_>>()
        };
        let previous = BlockOutcome {
            state: exec.state.clone(),
            bal: payload.bal.clone(),
            receipts: exec.receipts.clone(),
            gas: exec.gas,
            persistent_bytes: exec.persistent_bytes,
            new_slots: exec.new_slots,
            settlement: exec.settlement,
        };
        listed.into_iter().filter_map(|tx| {
            let out = aether_execution::block::append_block_preview(&exec.state, ctx, &previous, &tx)?;
            let mut appended = payload.clone();
            appended.txs.push(tx.clone());
            appended.bal = out.bal;
            appended.gas = out.gas;
            appended.receipts_root = Some(aether_execution::receipt::receipt_root(&out.receipts));
            (appended.to_bytes().len() as u64 <= payload_archive_limit(parent, &appended))
                .then(|| aether_execution::tx_hash(&tx))
        }).collect()
    }

    /// Returns false if the pool is full or the tx is already known.
    /// Admit a (signature-checked) tx: `Ok(true)` if new, `Ok(false)` if already
    /// known, `Err` if it could never execute or the pool is full, so txs that
    /// would only ever be skipped cannot pile up for free. On a fee network a
    /// full pool still admits a strictly higher-fee tx by evicting the
    /// lowest-fee entry that is safe to drop (R2-6, audit round 2, 2026-10-03);
    /// without fees (7780) a full pool refuses, as it always did.
    pub fn add_to_mempool(&self, tx: TxEnvelope) -> Result<bool, String> {
        let h = aether_execution::tx_hash(&tx);
        let size = tx.to_canonical_bytes().len();
        let mut g = self.lock();
        if g.receipts.contains_key(&h) || g.mempool.contains_key(&h) {
            return Ok(false);
        }
        if size > MAX_TX_BYTES {
            return Err(format!("transaction is {size} bytes, over the {MAX_TX_BYTES}-byte cap"));
        }
        let state = &g.finalized.state;
        if tx.header.nonce < state.nonce(&tx.header.sender) {
            return Err("nonce already used".into());
        }
        // Another group's tx can never run in this chain's blocks.
        if tx.header.group() != g.cfg.group {
            return Err(format!("tx of group {} on a group-{} chain", tx.header.group(), g.cfg.group));
        }
        let base = Self::next_base_fee(&g.cfg, &g.finalized);
        if g.cfg.fees || g.cfg.node_rewards || g.cfg.history_v2 {
            admissible(&tx, state, base)?;
            if base.state != 0 {
                let ctx = BlockContext {
                    chain_id: g.cfg.chain_id,
                    number: g.finalized.height + 1,
                    timestamp: g.finalized.timestamp / 1_000 + 1,
                    beneficiary: if g.cfg.fees { aether_execution::FEE_COLLECTOR } else { Address::ZERO },
                    limits: GasVector { state: fees::state_block_limit(g.finalized.excess.state), ..g.cfg.limits },
                    fees: g.cfg.fees.then_some(FeePolicy { base, proposer: Address::ZERO }),
                };
                aether_execution::check_admission(state, &ctx, &tx)?;
            }
        }
        if g.pending_by_sender
            .get(&tx.header.sender)
            .copied()
            .unwrap_or(0)
            >= MAX_PER_SENDER
        {
            return Err(format!("sender has {MAX_PER_SENDER} pending transactions"));
        }
        // A tx that pays nothing at the current base fee takes a free-lane
        // slot, and the free lane is a quota of the pool (R2-6): zero-balance
        // spam cannot crowd paying senders out of their three quarters. Its
        // bytes are a share of the budget too (A3-3): the count quota alone
        // left the whole 64 MiB reachable by a few hundred fat free txs. Fee
        // networks only — 7780 has a single lane, as it always had.
        let free = g.cfg.fees && effective_fee(&tx, base) == 0;
        if free && g.free_in_pool >= MAX_FREE_MEMPOOL {
            return Err(format!(
                "free lane full: {MAX_FREE_MEMPOOL} of {MAX_MEMPOOL} entries already pay nothing"
            ));
        }
        if free && g.free_mempool_bytes + size > MAX_FREE_MEMPOOL_BYTES {
            return Err(format!(
                "free lane byte budget full: zero-fee entries already hold {} of the {MAX_FREE_MEMPOOL_BYTES}-byte share",
                g.free_mempool_bytes
            ));
        }
        let count_full = g.mempool.len() >= MAX_MEMPOOL;
        let bytes_full = g.mempool_bytes + size > MAX_MEMPOOL_BYTES;
        if (count_full || bytes_full)
            && (!g.cfg.fees || free || !evict_for(&mut g, &tx, base, size))
        {
            return Err(if count_full {
                "mempool full".into()
            } else {
                "mempool byte budget full".into()
            });
        }
        *g.pending_by_sender.entry(tx.header.sender).or_default() += 1;
        if free {
            g.free_in_pool += 1;
            g.free_mempool_bytes += size;
        }
        g.index_nonce(&tx);
        g.mempool.insert(h, tx);
        g.sizes.insert(h, size);
        g.mempool_bytes += size;
        g.arrivals.insert(h, Instant::now());
        g.tombstones.forget(&h);
        Ok(true)
    }

    /// Adopt a finalized block (delivered in order by marshal) and persist it.
    pub fn finalize(&self, block: &Block) -> Result<(), ChainError> {
        // A commit attempt is work, even one that fails on a full disk: the
        // store's re-opens tick too, so a healing node never reads as stuck.
        tick();
        let height = block.height().get();
        let already_finalized = {
            let g = self.lock();
            height <= g.finalized.height && height != 0
        };
        if already_finalized {
            // At-least-once delivery must still match the durable hash after
            // its memory copy was evicted.
            let ours = self.block_summary(height).map_err(|e| ChainError::Store(e.to_string()))?;
            if ours.is_some_and(|b| b.hash != format!("{}", block.digest())) {
                return Err(ChainError::ConflictingFinality { height });
            }
            return Ok(());
        }
        let exec = match self.get(&block.digest()) {
            Some(e) => e,
            None => {
                // Not verified locally (e.g. backfilled): execute on the finalized parent.
                let parent = self.lock().finalized.clone();
                if parent.digest != block.parent {
                    return Err(ChainError::UnknownParent);
                }
                self.execute_certified(block, &parent)?
            }
        };
        let payload = block.payload().ok_or(ChainError::BadPayload)?;
        let summary = summary(block, &exec, payload.parent_state_root);
        let exec_bytes = execution_bytes(&exec);
        // Optional proving work stays outside the consensus mutex, including
        // for locally proposed blocks whose execution was already remembered.
        let retain_input = self.lock().prover_window > 0 && exec.statement != Statement::default();
        let proving_input = if retain_input {
            self.get(&block.parent).and_then(|parent| {
                let result = crate::prover::input_for(self, &exec, &parent, block, Address::ZERO)
                    .and_then(|input| crate::prover_input::encode(&input).map_err(|e| (exec.height, e)));
                match result {
                    Ok(encoded) => {
                        let bytes = encoded.len() as u64 + block.data.len() as u64
                            + std::mem::size_of::<ProvingInput>() as u64 + std::mem::size_of::<Block>() as u64 + 256;
                        Some(Arc::new(ProvingInput { encoded: encoded.into_boxed_slice(), commitment: exec.statement.commitment, bytes }))
                    }
                    Err((height, error)) => {
                        tracing::warn!(height, %error, "could not retain a compact proving input");
                        None
                    }
                }
            })
        } else { None };
        let search = self.lock().search.clone();
        let search_sources = self.lock().search_sources.clone();
        let mut search_index = search.lock().expect("search index");
        let search_events = crate::search_events::block_events(&exec.receipts, &exec.state, &search_sources, block.timestamp / 1000);
        let search_delta = search_index.apply_batch(&search_events);
        let (store, history_v2, compact_swaps, previous_history, relaxed, mut release_watcher, previous, chain_id) = {
            let g = self.lock();
            (g.store.clone(), g.cfg.history_v2, g.cfg.node_rewards || g.cfg.history_v2,
                g.finalized.history.clone(), g.relaxed, g.release_watcher.clone(), g.finalized.clone(), g.cfg.chain_id)
        };
        // The next finalized header certifies its parent's post-state. Event
        // bytes from this block remain discovery until that anchor exists.
        if release_watcher.pin.is_some() {
            if previous.height.checked_add(1) == Some(exec.height) && payload.parent_state_root == previous.state.root() {
                release_watcher.certify(&previous.state, chain_id, previous.height, block.timestamp);
            }
            release_watcher.discover(&exec.receipts, &exec.state, chain_id, exec.height);
        }
        let release_cache = release_watcher.cache_if_dirty();
        let mut upgrade_notices = self.lock().upgrade_notices.clone();
        upgrade_notices.retain(|s| s.upgrade.activate_at > exec.height);
        if let Some(s) = &payload.upgrade {
            upgrade_notices.push(s.clone());
        }
        // History v2: keep the block for its era file; an era's first block also keeps the history before it.
        let staged = history_v2.then(|| commonware_codec::Encode::encode(block));
        let era_start = (history_v2 && exec.height.is_multiple_of(aether_state::mmr::ERA_LEN))
            .then(|| {
                if exec.height == 0 {
                    Default::default()
                } else {
                    (*previous_history).clone()
                }
            });
        if let Some(store) = store {
            // Save only bounded raw payloads, never a cached approval. A crash
            // before the state commit leaves a future candidate that fails
            // verification on restore; the previous valid payload stays too.
            if let Some(bytes) = release_cache {
                store.put_meta(crate::release::CACHE_KEY, &bytes)
                    .map_err(|e| ChainError::Store(e.to_string()))?;
            }
            // Disk first: the in-memory head never runs ahead of what survives a crash.
            let mut account_rows = Vec::new();
            let mut account_delegations = {
                let g = self.lock();
                let mut by_sender = HashMap::new();
                for tx in &payload.txs {
                    by_sender.entry(tx.header.sender).or_insert_with(|| {
                        let code = g.finalized.state.code(&tx.header.sender);
                        code.len() == 23
                            && code[..3] == [0xef, 0x01, 0x00]
                            && code[3..] == aether_execution::AETHER_ACCOUNT.0 .0
                    });
                }
                by_sender
            };
            let fees_on = {
                let g = self.lock();
                g.cfg.fees
            };
            for (index, (tx, receipt)) in payload.txs.iter().zip(&exec.receipts).enumerate() {
                if let aether_types::TxPayload::Plain(bytes) = &tx.payload {
                    if let Ok(call) = aether_execution::EvmCall::decode(bytes) {
                        if let Some(delegate) = call.delegate {
                            account_delegations.insert(tx.header.sender, delegate == aether_execution::AETHER_ACCOUNT);
                        }
                    }
                }
                let is_aether_account = account_delegations.get(&tx.header.sender).copied().unwrap_or(false);
                // What this tx cost its sender, from the receipt: exec gas at
                // the price it paid (EIP-1559 under a fee policy, the cap
                // otherwise), the prove fee, and the state fee — the wallet's
                // balance breakdown itemizes every wei of it.
                let exec_price = if fees_on {
                    tx.header.tip.saturating_add(exec.base_fee.exec).min(tx.header.max_fee.exec)
                } else {
                    tx.header.max_fee.exec
                };
                let prove_price = if fees_on { exec.base_fee.prove } else { 0 };
                let fee_wei = U256::from(receipt.gas_used) * U256::from(exec_price)
                    + U256::from(receipt.prove_gas) * U256::from(prove_price)
                    + receipt.state_fee;
                // Block time is milliseconds; `exec.timestamp` is the seconds
                // the EVM wants, and writing it as ms put every row in 1970.
                account_rows.extend(crate::account_history::transaction(tx, receipt, exec.height, index as u32, block.timestamp, fee_wei, is_aether_account, compact_swaps));
            }
            for (index, (proven, address, amount)) in exec.payouts.iter().enumerate() {
                account_rows.push(crate::account_history::reward(*address, exec.height, payload.txs.len() as u32 + index as u32, block.timestamp, *amount, *proven == exec.height));
            }
            for (index, registration) in payload.registrations.iter().enumerate() {
                account_rows.push(crate::account_history::registration(registration.operator, exec.height,
                    (payload.txs.len() + exec.payouts.len() + index) as u32, block.timestamp,
                    crate::registrations::id(registration)));
            }
            let write = Commit {
                height: exec.height,
                digest: digest_bytes(&exec.digest),
                root: exec.state.root(),
                diff: exec.state.journal(),
                summary: &summary,
                receipts: exec.tx_hashes.iter().copied().zip(exec.receipts.iter()).collect(),
                handoff: exec.handoff.as_deref().filter(|p| p.at == exec.height),
                seed: exec.seed.as_deref().filter(|s| s.0 == exec.height),
                history: &exec.history,
                schedule: &exec.schedule,
                upgrade_notices: &upgrade_notices,
                statement: &exec.statement,
                staged: staged.as_ref().map(|b| crate::store::Staged {
                    block: b,
                    era_start: era_start.as_ref(),
                }),
            };
            if let Err(error) = store.commit_with_search(write, &account_rows, &search_delta, &search_index, search_sources.fingerprint(), relaxed) {
                search_index.undo(&search_delta);
                return Err(ChainError::Store(error.to_string()));
            }
            // An era's last block: seal it into a file, off the consensus path.
            // Below the free-space floor the seal waits — the blocks stay
            // staged in the store, and a later start's `seal_pending` catches up.
            if history_v2 && (exec.height + 1).is_multiple_of(aether_state::mmr::ERA_LEN) {
                let era = exec.height / aether_state::mmr::ERA_LEN;
                if crate::resources::disk_ok() {
                    std::thread::spawn(move || crate::era::seal_logged(&store, era));
                } else {
                    tracing::warn!(era, "disk below the free-space floor: era sealing waits for space");
                }
            }
        }
        let mut g = self.lock();
        // A concurrent byte-budget trim can evict a remembered candidate
        // while its input is being built. Finality must charge and retain it.
        g.executed.insert(exec.digest, exec.clone());
        g.execution_sizes.insert(exec.digest, exec_bytes);
        g.release_watcher = release_watcher;
        g.upgrade_notices = upgrade_notices;
        for (h, r) in exec.tx_hashes.iter().zip(&exec.receipts) {
            let rb = receipt_bytes(r);
            let old = g.receipts.insert(*h, (exec.height, r.clone())).map(|(_, r)| receipt_bytes(&r));
            g.caches_bytes = g.caches_bytes.saturating_add(rb).saturating_sub(old.unwrap_or(0));
            g.mempool.remove(h);
            g.tombstones.forget(h);
            if let Some(s) = g.sizes.remove(h) {
                g.mempool_bytes -= s;
            }
        }
        let state = exec.state.clone();
        // Drop what can no longer run: a used nonce, a sender whose balance no
        // longer covers the tx (it paid for other txs meanwhile), or a tx left
        // waiting past the TTL. Otherwise txs admitted while affordable would
        // hold per-sender slots and pool space forever: enough of them would
        // fill the pool and stop the chain from taking new txs.
        let base = Self::next_base_fee(&g.cfg, &exec);
        let fees = g.cfg.fees;
        let now = Instant::now();
        let inner = &mut *g;
        let arrivals = &inner.arrivals;
        // Every departure leaves a tombstone with its reason (bug #5): a
        // wallet that holds the hash learns what happened instead of nothing.
        let gaps = first_missing_nonces(&inner.mempool, &state);
        let tombstones = &mut inner.tombstones;
        inner.mempool.retain(|h, tx| {
            let gap = gaps.get(&tx.header.sender).copied().filter(|e| tx.header.nonce > *e);
            match drop_reason(tx, arrivals.get(h).copied(), now, &state, base, fees, gap) {
                None => true,
                Some(reason) => {
                    tombstones.record(*h, reason);
                    false
                }
            }
        });
        inner.arrivals.retain(|h, _| inner.mempool.contains_key(h));
        // The byte budget follows the pool: sizes of txs that left give their
        // bytes back, so the budget never leaks by a dropped tx.
        {
            let (mempool, sizes) = (&inner.mempool, &mut inner.sizes);
            let mut freed = 0;
            sizes.retain(|h, s| {
                let kept = mempool.contains_key(h);
                if !kept {
                    freed += *s;
                }
                kept
            });
            inner.mempool_bytes -= freed;
        }
        inner.pending_by_sender.clear();
        inner.nonces_by_sender.clear();
        inner.free_in_pool = 0;
        inner.free_mempool_bytes = 0;
        for (h, t) in inner.mempool.iter() {
            *inner.pending_by_sender.entry(t.header.sender).or_default() += 1;
            *inner.nonces_by_sender.entry(t.header.sender).or_default().entry(t.header.nonce).or_default() += 1;
            if fees && effective_fee(t, base) == 0 {
                inner.free_in_pool += 1;
                inner.free_mempool_bytes += inner.sizes.get(h).copied().unwrap_or_default();
            }
        }
        inner.inclusion.prune(&state, exec.height, Instant::now());
        // One leaf per height, in order (genesis is delivered again at start-up).
        if let Some(idx) = g
            .history_index
            .as_mut()
            .filter(|i| i.leaves() == exec.height)
        {
            Arc::make_mut(idx).push(
                &ChainHasher::new(),
                aether_state::mmr::leaf(
                    &ChainHasher::new(),
                    exec.height,
                    &digest_bytes(&exec.digest),
                ),
            );
        }
        let sb = summary_bytes(&summary);
        let old = g.blocks.insert(exec.height, summary);
        g.caches_bytes = g.caches_bytes.saturating_add(sb).saturating_sub(old.as_ref().map(summary_bytes).unwrap_or(0));
        let previous = std::mem::replace(&mut g.finalized, exec.clone());
        // A finalized handoff ends the running epoch before its switch height.
        if let Some(p) = exec.handoff.as_ref().filter(|p| p.at == exec.height) {
            if p.switch > g.epoch_start {
                g.epoch_end
                    .store(p.switch - 1, std::sync::atomic::Ordering::SeqCst);
            }
            if g.handoff_ready.as_ref() == Some(&p.handoff) {
                g.handoff_ready = None;
            }
            // This draw's voting set is on its way in: nobody reshares to it again.
            g.proposal = None;
            keep(&g.store, PROPOSAL, &g.proposal);
        }
        // A draw's pool is frozen from the state its first block builds on
        // (before its seed exists): node-rewards networks froze it in state
        // (`rewards::freeze_pool`), the others freeze it here as before.
        let params = aether_execution::registry::params(&previous.state);
        let span = params.epoch_blocks * params.draw_epochs;
        if exec.height > 0 && exec.height.is_multiple_of(span) {
            g.pool = if aether_rewards::enabled(&exec.state) {
                aether_rewards::draw_pool(&exec.state)
            } else {
                let pool = crate::rotation::eligible(
                    &previous.state,
                    exec.height / params.epoch_blocks,
                    params.min_streak,
                );
                Some((exec.height / span, pool))
            };
            if !proposal_in_window(&g, exec.height, &exec.state) {
                g.proposal = None;
            }
            keep(&g.store, POOL, &g.pool);
            if let Some((draw, pool)) = &g.pool {
                crate::rotation::log_draw_pool(*draw, pool.len(), crate::rotation::open_seats_at(&g, exec.height));
            }
        }
        // With the draw's seed on chain, everyone draws the same next voting
        // set. Node-rewards networks committed it with the block that carries
        // the seed (`pre_state_with`); the others draw it here as before.
        if let Some(s) = exec.seed.as_ref().filter(|s| s.0 == exec.height) {
            if g.seed_ready.as_ref() == Some(&s.1) {
                g.seed_ready = None;
            }
            // Node-rewards networks committed the roster with the seed's own
            // block (`pre_state_with`); the others draw it here as before.
            if !aether_rewards::enabled(&exec.state)
                && !proposal_in_window(&g, exec.height, &exec.state)
            {
                if let Some((draw, pool)) = g.pool.clone().filter(|(d, _)| *d == s.1.draw) {
                    let seed = hex::decode(&s.1.signature).unwrap_or_default();
                    // The per-operator seat cap is a protocol-2 rule: before it, every key is its own operator.
                    let ops = crate::rotation::operators(&exec.state);
                    let capped = exec.next_protocol() >= 2;
                    let operator = |k: &str| {
                        ops.get(k)
                            .map(|o| if capped { o.clone() } else { k.to_string() })
                    };
                    // Protocol 3: qualifying Macs join (up to 16 seats) instead of
                    // replacing members — and each seat goes where it lifts the
                    // committee's worst hour of the day (13-roadmap.md, F).
                    let reserve = Reserve::of(&exec.state);
                    let availability = (exec.next_protocol() >= 3 || reserve.is_some())
                        .then(|| crate::rotation::availability(&exec.state));
                    let hours = |k: &str| availability.as_ref().and_then(|a| a.get(k).copied());
                    let drawn = if exec.next_protocol() >= 3 {
                        crate::rotation::draw_spread(&pool, &seed, operator, &g.committee, hours, crate::rotation::GROW_UNTIL)
                    } else {
                        crate::rotation::draw(&pool, &seed, operator, &g.committee)
                    };
                    // Founder reserve keys join or leave with the draw too.
                    let drawn = match reserve {
                        Some(r) => crate::rotation::with_reserve_for(
                            crate::upgrade::protocol_at(&exec.schedule, exec.height),
                            drawn,
                            &pool,
                            &seed,
                            |k: &str| ops.get(k).cloned(),
                            &r,
                            &g.committee,
                            hours,
                        ),
                        None => drawn,
                    };
                    g.proposal = drawn.map(|m| (draw, m));
                    keep(&g.store, PROPOSAL, &g.proposal);
                }
            }
        }
        // Node-rewards networks keep the draw's pool and the decided next
        // roster in state — every node's the same, whatever its network.json
        // — and the node-local mirrors follow it.
        if aether_rewards::enabled(&exec.state) {
            let draw = current_draw(&exec.state, exec.height);
            g.pool = aether_rewards::draw_pool(&exec.state).filter(|(d, _)| *d == draw);
            g.proposal = aether_rewards::next_roster(&exec.state).filter(|(d, _)| *d == draw);
            keep(&g.store, POOL, &g.pool);
            keep(&g.store, PROPOSAL, &g.proposal);
        }
        for (proven, prover, amount) in &exec.payouts {
            let kind = if *proven == exec.height {
                "node"
            } else {
                "proof"
            };
            let record = serde_json::json!({ "kind": kind, "proven": proven, "amount": amount, "height": exec.height, "timestamp_ms": exec.timestamp });
            if let Some(Err(e)) = g.store.as_ref().map(|s| {
                s.put_reward(
                    &prover.0 .0,
                    exec.height,
                    *proven,
                    record.to_string().as_bytes(),
                )
            }) {
                tracing::warn!(%e, "could not keep a reward record");
            }
        }
        // Our proposal with proofs lost at this height: others may not verify them.
        if g.proof_proposal
            .is_some_and(|(h, d)| h == exec.height && d != exec.digest)
        {
            g.proof_backoff_until = exec.height + PROOF_BACKOFF;
            g.proof_proposal = None;
        }
        for claim in &payload.proofs {
            g.notice_proof(claim.height);
        }
        g.finalized_block = Some(block.clone());
        // Retain compact witnesses only for an enabled prover. Neither an
        // unpaid statement nor its parent exempts a full state from eviction.
        g.recent.retain(|b| aether_execution::proofs::claimable(&exec.state, b.height().get(), exec.height + 1).is_ok());
        if g.prover_window > 0 && proving_input.is_some() {
            g.recent.push_back(block.clone());
            g.proving_inputs.insert(block.digest(), proving_input.expect("retained input"));
        }
        while g.recent.len() > g.prover_window {
            g.recent.pop_front();
        }
        trim_proving_inputs(&mut g);
        let floor = exec.height.saturating_sub(64);
        g.executed.retain(|_, e| e.height >= floor);
        let retained: std::collections::HashSet<_> = g.executed.keys().copied().collect();
        g.execution_sizes.retain(|digest, _| retained.contains(digest));
        // Proofs of blocks now proven (or expired) leave the pool.
        let now = exec.height;
        let state = &exec.state;
        g.proof_pool.retain(|c| {
            aether_execution::proofs::claimable(state, c.height, now + 1).is_ok() || c.height == now
        });
        let oldest = g.recent.front().map_or(now, |b| b.height().get());
        g.attempted.retain(|h| *h >= oldest);
        g.lost_proofs.retain(|h| *h >= oldest);
        // Answers recorded, or of slots whose window closed, leave the pool.
        if !g.beacon_pool.is_empty() {
            use aether_rewards::beacons;
            let epoch_blocks = aether_execution::registry::epoch_blocks(state);
            let epoch = (now + 1) / epoch_blocks;
            let window = beacons::layout(epoch_blocks).map_or(0, |l| l.1);
            g.beacon_pool.retain(|(e, slot, index), _| {
                if let Some((signed_height, _)) = beacons::availability_of(*slot) {
                    return signed_height > now && *e == epoch;
                }
                let b = beacons::beacon(state, *index);
                let recorded = b.epoch == *e && b.mask & (1 << slot) != 0;
                let open = *e == epoch && beacons::slot(state, *slot).is_some_and(|h| h + window > now);
                open && !recorded
            });
        }
        // A free-lane item succeeded: its id gets a receipt the wallet polls
        // like a tx's (in memory here; the registry state itself is the durable
        // record). Items whose key landed (however it got there) or that
        // expired leave the pool.
        if !exec.registration_ids.is_empty() {
            for id in &exec.registration_ids {
                let receipt = registration_receipt(*id);
                let bytes = receipt_bytes(&receipt);
                let old = g.receipts.insert(*id, (exec.height, receipt)).map(|(_, r)| receipt_bytes(&r));
                g.caches_bytes = g.caches_bytes.saturating_add(bytes).saturating_sub(old.unwrap_or(0));
            }
        }
        if !g.registration_pool.is_empty() {
            let next = exec.height + 1;
            g.registration_pool.retain(|_, r| {
                aether_execution::registry::index_of(state, &r.validator_key.0) == 0 && next <= r.expiry
            });
        }
        // Early replacement (node-rewards networks) and the reserve seating
        // both commit their roster in state (see `pre_state_with`); only the
        // node-local reserve rule of the other networks runs here.
        reserve_step(&mut g, &previous, &exec);
        // Last: keep optional retention and durable history caches within
        // their budgets, preserving the finalized head and its parent.
        let budget = g.history_budget;
        trim_caches(&mut g, budget);
        // Disk and execution succeeded. Registration takes this same lock to
        // capture its watermark, so replay/live cannot miss a commit. This
        // bounded synchronous send never waits for a client or builds a proof.
        g.push.publish(exec.height);
        Ok(())
    }
}

/// The fee-policy checks a block would reject the tx for, applied at admission:
/// caps at least the current base fees (zero when uncongested), a prove budget covering the gas limit, and a
/// balance covering value, max exec fee and prove budget.
fn admissible(tx: &TxEnvelope, state: &WorldState, base: FeeVector) -> Result<(), String> {
    if tx.header.max_fee.exec < base.exec || tx.header.max_fee.prove < base.prove {
        return Err("fee caps below the base fee".into());
    }
    if base.state != 0 && tx.header.gas.state > 0 && tx.header.max_fee.state < base.state {
        return Err("state fee cap below the state base price".into());
    }
    affordable(tx, state, base)
}

/// What a tx would actually pay at `base` (R2-6's ranking fee): exec gas at
/// the base fee plus its tip, capped by `max_fee.exec` (EIP-1559 shape), plus
/// prove gas at the base fee capped by its prove cap. Zero means the tx pays
/// nothing at this base fee — a free-lane entry.
fn effective_fee(tx: &TxEnvelope, base: FeeVector) -> u128 {
    let exec = base.exec.saturating_add(tx.header.tip).min(tx.header.max_fee.exec);
    let prove = base.prove.min(tx.header.max_fee.prove);
    exec.saturating_add(prove)
}

/// Make room for `tx` (`size` bytes, paying `effective_fee(tx, base)`) by
/// evicting the lowest-fee pending entries that may safely leave (R2-6): only
/// a sender's highest pending nonce — its runnable prefix stays runnable —
/// and never a tx an inclusion list names (FOCIL: what a list names is this
/// node's obligation to propose, whatever its fee). Only strictly lower-fee
/// entries go; an equal fee never displaces its peer. True when the pool now
/// holds the count and byte room `tx` needs.
///
/// The decision comes first, the removals after (A3-3, audit round 3): the
/// complete evictable set is walked in (fee, arrival) order to find the
/// shortest prefix whose departure makes the newcomer fit. No prefix does —
/// the newcomer cannot fit even with every eligible victim gone — and false
/// comes back with the pool exactly as it was, so a rejected admission never
/// deletes paying entries for free.
fn evict_for(g: &mut Inner, tx: &TxEnvelope, base: FeeVector, size: usize) -> bool {
    let room = |g: &Inner| g.mempool.len() < MAX_MEMPOOL && g.mempool_bytes + size <= MAX_MEMPOOL_BYTES;
    if room(g) {
        return true;
    }
    // A sender's nonce chain leaves from its end only: the highest pending
    // nonce is the one entry of the chain whose removal strands nothing.
    let mut tops: HashMap<Address, u64> = HashMap::with_capacity(g.pending_by_sender.len());
    for t in g.mempool.values() {
        tops.entry(t.header.sender)
            .and_modify(|top| *top = (*top).max(t.header.nonce))
            .or_insert(t.header.nonce);
    }
    let mut candidates: Vec<(u128, Instant, TxHash, Address)> = g
        .mempool
        .iter()
        .filter(|(h, t)| t.header.nonce == tops[&t.header.sender] && !g.inclusion.contains(h))
        .map(|(h, t)| {
            (
                effective_fee(t, base),
                g.arrivals.get(h).copied().unwrap_or_else(Instant::now),
                *h,
                t.header.sender,
            )
        })
        .collect();
    candidates.sort_unstable();
    let floor = effective_fee(tx, base);
    // Decide: walk the strictly-lower-fee prefix, simulating each departure,
    // until the newcomer fits. Candidates sort by fee, so the eligible ones
    // are exactly this prefix.
    let fits = |removed: usize, freed: usize| {
        g.mempool.len() - removed < MAX_MEMPOOL && g.mempool_bytes - freed + size <= MAX_MEMPOOL_BYTES
    };
    let mut freed = 0;
    let mut take: Option<usize> = None;
    for (i, (their_fee, _, h, _)) in candidates.iter().enumerate() {
        if *their_fee >= floor {
            break; // nothing strictly lower remains
        }
        freed += g.sizes.get(h).copied().unwrap_or_default();
        if fits(i + 1, freed) {
            take = Some(i + 1);
            break;
        }
    }
    let Some(take) = take else {
        return false; // cannot fit even after evicting every eligible victim
    };
    // Remove: the pool only now changes, and only by the prefix that pays
    // for the newcomer's room.
    for (_, _, h, sender) in candidates.into_iter().take(take) {
        let evicted = g.mempool.remove(&h).expect("a candidate is pending");
        g.unindex_nonce(&evicted);
        g.tombstones.record(h, DropReason::Evicted);
        let freed = g.sizes.remove(&h).unwrap_or_default();
        g.mempool_bytes -= freed;
        g.arrivals.remove(&h);
        let pending = g.pending_by_sender.entry(sender).or_default();
        *pending -= 1;
        if *pending == 0 {
            g.pending_by_sender.remove(&sender);
        }
        if effective_fee(&evicted, base) == 0 {
            g.free_in_pool -= 1;
            g.free_mempool_bytes -= freed;
        }
    }
    true
}

/// Whether a pending tx stays in the pool after a block: its nonce is still
/// ahead, its sender can still pay for it, and it has not waited past the TTL.
/// A tx whose fee caps are below the base fee waits for the fee to come down
/// for `MEMPOOL_FEE_WAIT` (not the whole TTL, R2-6): the base fee falls by a
/// target per empty block, so a wait worth making is over in well under a
/// minute, and past it the entry stops holding paying capacity.
#[cfg(test)]
fn keep_in_pool(
    tx: &TxEnvelope,
    arrived: Option<Instant>,
    now: Instant,
    state: &WorldState,
    base: FeeVector,
    fees: bool,
) -> bool {
    drop_reason(tx, arrived, now, state, base, fees, None).is_none()
}

/// `keep_in_pool`'s rule, answering why a tx leaves (None: it stays). The
/// rule is unchanged by bug #5; only the reason is now kept. `gap` is the
/// first nonce its sender is missing, when this tx's nonce is beyond it.
/// A TTL departure names what it waited for: a state cap under the B5 price
/// first (the stress run's 78 silent drops), then a nonce gap, then exec or
/// prove caps under the base fee.
fn drop_reason(
    tx: &TxEnvelope,
    arrived: Option<Instant>,
    now: Instant,
    state: &WorldState,
    base: FeeVector,
    fees: bool,
    gap: Option<u64>,
) -> Option<DropReason> {
    if tx.header.nonce < state.nonce(&tx.header.sender) {
        return Some(DropReason::Replaced);
    }
    let waited = |d: Duration| arrived.is_some_and(|t| now.saturating_duration_since(t) >= d);
    let below_exec = fees && (tx.header.max_fee.exec < base.exec || tx.header.max_fee.prove < base.prove);
    if waited(MEMPOOL_TTL) {
        return Some(if let Some(r) = state_price_wait(&tx.header, base, None) {
            r
        } else if let Some(expected) = gap {
            DropReason::NonceGap { expected }
        } else if below_exec {
            DropReason::FeeCapBelowBase
        } else {
            DropReason::Expired
        });
    }
    if below_exec && waited(MEMPOOL_FEE_WAIT) {
        return Some(DropReason::FeeCapBelowBase);
    }
    // A retained envelope waits when a price exceeds its signed cap. That
    // unchargeable price cannot turn a covered budget into a balance loss.
    let payable = FeeVector {
        prove: base.prove.min(tx.header.max_fee.prove),
        state: base.state.min(tx.header.max_fee.state),
        ..base
    };
    if (!fees && base.state == 0) || affordable(tx, state, payable).is_ok() {
        None
    } else {
        Some(DropReason::Unaffordable)
    }
}

/// `StatePriceAboveCap` when the tx's signed state cap is under the B5
/// price it would pay now; `excess` (the state debt) adds the refill estimate.
fn state_price_wait(header: &aether_types::TxHeader, base: FeeVector, excess: Option<u64>) -> Option<DropReason> {
    let cap = header.max_fee.state;
    (base.state != 0 && header.gas.state > 0 && cap < base.state).then(|| DropReason::StatePriceAboveCap {
        cap: cap.to_string(),
        price: base.state.to_string(),
        blocks: excess.and_then(|e| fees::blocks_until_state_price_at_most(e, cap)),
    })
}

/// Each pending sender's first nonce that neither the chain nor the pool
/// has: its txs above it wait behind a gap.
fn first_missing_nonces(pool: &BTreeMap<TxHash, TxEnvelope>, state: &WorldState) -> HashMap<Address, u64> {
    let mut nonces: HashMap<Address, std::collections::BTreeSet<u64>> = HashMap::new();
    for t in pool.values() {
        nonces.entry(t.header.sender).or_default().insert(t.header.nonce);
    }
    nonces
        .into_iter()
        .map(|(sender, set)| {
            let mut next = state.nonce(&sender);
            while set.contains(&next) {
                next += 1;
            }
            (sender, next)
        })
        .collect()
}

impl Inner {
    fn notice_proof(&mut self, height: u64) {
        self.lost_proofs.insert(height);
        let seen = Instant::now();
        self.proof_observers.retain(|observer| observer.send((height, seen)).is_ok());
    }

    /// Count `tx`'s nonce in its sender's index (before it enters the pool).
    fn index_nonce(&mut self, tx: &TxEnvelope) {
        *self.nonces_by_sender.entry(tx.header.sender).or_default().entry(tx.header.nonce).or_default() += 1;
    }

    /// Uncount `tx`'s nonce (it left the pool).
    fn unindex_nonce(&mut self, tx: &TxEnvelope) {
        let sender = tx.header.sender;
        let Some(nonces) = self.nonces_by_sender.get_mut(&sender) else { return };
        if let Some(n) = nonces.get_mut(&tx.header.nonce) {
            *n -= 1;
            if *n == 0 {
                nonces.remove(&tx.header.nonce);
            }
        }
        if nonces.is_empty() {
            self.nonces_by_sender.remove(&sender);
        }
    }

    /// Put `tx` in the pool with its index entry, skipping admission (tests
    /// and fixtures that stage a pool directly).
    pub fn insert_pending(&mut self, h: TxHash, tx: TxEnvelope) {
        if let Some(old) = self.mempool.remove(&h) {
            self.unindex_nonce(&old);
        }
        self.index_nonce(&tx);
        self.mempool.insert(h, tx);
    }
}

#[cfg(test)]
thread_local! {
    /// Pool entries one `pending_facts` call examined (finding 4's check).
    static PENDING_SCANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What a pending tx's reason depends on, copied out under the chain lock in
/// O(log n + its sender's ≤ 64 nonces) — the receipt handler then computes
/// the reason (and the refill estimate) after releasing the lock (B5 review
/// round 2, finding 4).
#[derive(Clone, Debug)]
pub struct PendingFacts {
    header: aether_types::TxHeader,
    base: FeeVector,
    excess: u64,
    fees: bool,
    /// The sender's next nonce on chain.
    chain_nonce: u64,
    /// The sender's nonces in the pool, from `chain_nonce` up.
    queued: Vec<u64>,
}

pub fn pending_facts(g: &Inner, tx: &TxEnvelope) -> PendingFacts {
    let chain_nonce = g.finalized.state.nonce(&tx.header.sender);
    let queued: Vec<u64> = g
        .nonces_by_sender
        .get(&tx.header.sender)
        .map(|n| n.range(chain_nonce..).map(|(k, _)| *k).collect())
        .unwrap_or_default();
    #[cfg(test)]
    PENDING_SCANNED.with(|c| c.set(c.get() + queued.len()));
    PendingFacts {
        header: tx.header.clone(),
        base: Chain::next_base_fee(&g.cfg, &g.finalized),
        excess: g.finalized.excess.state,
        fees: g.cfg.fees,
        chain_nonce,
        queued,
    }
}

impl PendingFacts {
    /// Why it is not in a block yet (see `pending_reason`). No lock needed.
    pub fn reason(&self) -> Option<DropReason> {
        if let Some(r) = state_price_wait(&self.header, self.base, Some(self.excess)) {
            return Some(r);
        }
        let mut next = self.chain_nonce;
        for n in &self.queued {
            if *n != next {
                break;
            }
            next += 1;
        }
        if self.header.nonce > next {
            return Some(DropReason::NonceGap { expected: next });
        }
        (self.fees && (self.header.max_fee.exec < self.base.exec || self.header.max_fee.prove < self.base.prove))
            .then_some(DropReason::FeeCapBelowBase)
    }
}

/// Why a pending tx is not in a block yet, at the next block's prices
/// (bug #5): its state cap under the B5 price (with the refill estimate), a
/// nonce gap before it, or exec/prove caps under the base fee. None: nothing
/// holds it back but its turn.
pub fn pending_reason(g: &Inner, tx: &TxEnvelope) -> Option<DropReason> {
    pending_facts(g, tx).reason()
}

/// The payload decodes, the prove budget covers the gas limit, and the sender's
/// balance covers value, gas at its fee cap and the prove budget.
fn affordable(tx: &TxEnvelope, state: &WorldState, base: FeeVector) -> Result<(), String> {
    let aether_types::TxPayload::Plain(bytes) = &tx.payload else {
        return Err("encrypted payloads are not supported yet".into());
    };
    let call = aether_execution::EvmCall::decode(bytes).map_err(|e| format!("payload: {e:?}"))?;
    if tx.header.gas.prove < call.gas_limit {
        return Err("prove budget below the gas limit".into());
    }
    let need = U256::from(call.gas_limit)
        .checked_mul(U256::from(tx.header.max_fee.exec))
        .and_then(|g| {
            U256::from(tx.header.gas.prove)
                .checked_mul(U256::from(base.prove))
                .and_then(|p| g.checked_add(p))
        })
        .and_then(|n| n.checked_add(call.value))
        .and_then(|n| n.checked_add(U256::from(tx.header.gas.state) * U256::from(base.state)));
    if need.is_none_or(|n| state.balance(&tx.header.sender) < n) {
        return Err("insufficient funds for value, gas and prove budget".into());
    }
    Ok(())
}

fn digest_bytes(d: &Digest) -> [u8; 32] {
    d.as_ref().try_into().expect("sha256 digest is 32 bytes")
}

/// The history index at restart: roots of the pruned eras (roadmap B4), then a
/// leaf per kept summary up to the head. None when a height is missing (a node
/// started from a checkpoint) or the result is not the history the store
/// committed (a damaged root table).
fn rebuild_history_index(
    blocks: &BTreeMap<u64, BlockSummary>,
    era_roots: &[[u8; 32]],
    pruned_below: u64,
    head: u64,
    history: &aether_state::mmr::Mmr,
) -> Option<aether_state::mmr::EraIndex> {
    use aether_state::mmr::ERA_LEN;
    let h = ChainHasher::new();
    let pruned_eras = pruned_below / ERA_LEN;
    if !pruned_below.is_multiple_of(ERA_LEN) || (era_roots.len() as u64) < pruned_eras {
        return None;
    }
    let mut idx = aether_state::mmr::EraIndex {
        eras: era_roots[..pruned_eras as usize].to_vec(),
        open: Vec::new(),
    };
    for k in pruned_below..=head {
        let d = blocks
            .get(&k)
            .and_then(|b| hex::decode(&b.hash).ok())
            .and_then(|d| <[u8; 32]>::try_from(d).ok())?;
        idx.push(&h, aether_state::mmr::leaf(&h, k, &d));
    }
    if pruned_below > 0 && idx.mmr(&h) != *history {
        tracing::warn!(
            pruned_below,
            "stored era roots do not match the committed history; history proofs are off"
        );
        return None;
    }
    Some(idx)
}

/// Rebuild the compact history index with at most one era of hashes in
/// memory, so an archival legacy database does not become a giant cache
/// during restart. Missing or inconsistent rows disable history proofs.
fn rebuild_history_index_from_store(
    store: &Store,
    era_roots: &[[u8; 32]],
    pruned_below: u64,
    head: u64,
    history: &aether_state::mmr::Mmr,
) -> Option<aether_state::mmr::EraIndex> {
    use aether_state::mmr::ERA_LEN;
    let h = ChainHasher::new();
    let pruned_eras = pruned_below / ERA_LEN;
    if !pruned_below.is_multiple_of(ERA_LEN) || (era_roots.len() as u64) < pruned_eras {
        return None;
    }
    let mut index = aether_state::mmr::EraIndex {
        eras: era_roots[..pruned_eras as usize].to_vec(),
        open: Vec::new(),
    };
    let end = head.checked_add(1)?;
    let mut from = pruned_below;
    while from < end {
        let to = from.saturating_add(ERA_LEN).min(end);
        let hashes = store.block_hashes(from..to).ok()?;
        for (offset, hash) in hashes.iter().enumerate() {
            index.push(&h, aether_state::mmr::leaf(&h, from + offset as u64, hash));
        }
        from = to;
    }
    if index.mmr(&h) != *history {
        tracing::warn!("archival summaries do not match the committed history; history proofs are off");
        return None;
    }
    Some(index)
}

fn summary(block: &Block, e: &Executed, parent_state_root: B256) -> BlockSummary {
    BlockSummary {
        height: e.height,
        hash: format!("{}", block.digest()),
        parent: format!("{}", block.parent),
        timestamp_ms: block.timestamp,
        proposer: e.proposer,
        state_root: e.state.root(),
        parent_state_root,
        txs: e.tx_hashes.clone(),
        gas_used: e.gas.exec,
        prove_gas: e.gas.prove,
        base_fee: e.base_fee,
        excess: e.excess,
        archive_excess: e.archive_excess,
    }
}

/// Store keys: the proposed voting set, the frozen draw pool, and the genesis the data belongs to.
const PROPOSAL: &str = "proposal";
pub const GENESIS: &str = "genesis";

/// The genesis block hash a store is marked with.
pub fn genesis_digest(genesis: &Block) -> [u8; 32] {
    digest_bytes(&genesis.digest())
}
const POOL: &str = "pool";

/// Founder reserve keys (genesis parameter), the node-local rule of the
/// networks without node rewards: at every registry epoch's first block, seat
/// them for the next epoch while fewer than four independent operators
/// qualify, and let them go once four or more do. From four on, the keys are a
/// liveness safety net (13-roadmap.md, F): they seat themselves while the
/// committee's worst hour of the day risks losing its quorum. Proposes nothing
/// while a proposal or a handoff is already on its way.
fn reserve_step(g: &mut Inner, previous: &Executed, exec: &Executed) {
    // Node-rewards networks run the rule as a system write (see
    // `pre_state_with`): state-derived, so it binds the handoff that follows.
    if aether_rewards::enabled(&exec.state) {
        return;
    }
    let Some(reserve) = Reserve::of(&exec.state) else {
        return;
    };
    let params = aether_execution::registry::params(&previous.state);
    let handing_over = exec
        .handoff
        .as_ref()
        .is_some_and(|p| p.switch > exec.height);
    // A draw's first block leaves it to the draw (which applies the same rule with its seed).
    let draw_start = exec
        .height
        .is_multiple_of(params.epoch_blocks * params.draw_epochs);
    if exec.height == 0
        || draw_start
        || !exec.height.is_multiple_of(params.epoch_blocks)
        || g.committee.members.is_empty()
        || g.proposal.is_some()
        || handing_over
    {
        return;
    }
    let pool = crate::rotation::eligible(
        &previous.state,
        exec.height / params.epoch_blocks,
        params.min_streak,
    );
    let ops = crate::rotation::operators(&exec.state);
    let availability = crate::rotation::availability(&exec.state);
    let next = crate::rotation::with_reserve_for(
        crate::upgrade::protocol_at(&exec.schedule, exec.height),
        None,
        &pool,
        exec.digest.as_ref(),
        |k: &str| ops.get(k).cloned(),
        &reserve,
        &g.committee,
        |k: &str| availability.get(k).copied(),
    );
    if let Some(members) = next {
        tracing::info!(
            members = members.len(),
            "founder reserve keys change the voting set"
        );
        g.proposal = Some((current_draw(&exec.state, exec.height), members));
        keep(&g.store, PROPOSAL, &g.proposal);
    }
}

/// A new-genesis proposal remains available while its ceremony can finish,
/// even when a short devnet draw ends. 7780 keeps its draw-scoped proposal.
pub(crate) fn proposal_in_window(g: &Inner, height: u64, state: &WorldState) -> bool {
    if g.cfg.chain_id == 7_780 || aether_rewards::enabled(state) {
        return false;
    }
    let Some((draw, _)) = g.proposal.as_ref() else { return false };
    let params = aether_execution::registry::params(state);
    let span = params.epoch_blocks.saturating_mul(params.draw_epochs);
    let first = draw.saturating_mul(span).max(g.epoch_start);
    let reserve = Reserve::of(state).map_or(0, |r| r.members.len());
    let players = crate::supervisor::reshare_attempt_players(g.committee.members.len(), reserve);
    match (
        crate::supervisor::reshare_attempt(first, g.epoch_start, players),
        crate::supervisor::reshare_attempt(height, g.epoch_start, players),
    ) {
        (Ok(first), Ok(now)) => first == now,
        _ => false,
    }
}

/// The draw a block at `height` belongs to (draws start at multiples of epoch_blocks × draw_epochs).
fn current_draw(state: &WorldState, height: u64) -> u64 {
    let p = aether_execution::registry::params(state);
    // Saturating: a genesis is bounded (roster.rs), but a state word is read here.
    height / p.epoch_blocks.saturating_mul(p.draw_epochs).max(1)
}

/// Upgrade backfill streams kept receipt events in canonical block/tx/log
/// order. Older stores do not retain every signed transaction alongside its
/// receipt, so the activity signal stays unavailable for one complete week.
fn rebuild_search_cache(cp: &crate::store::Checkpoint, sources: &crate::search_sources::SearchSources) -> crate::search::SearchIndex {
    use crate::search::{SearchEvent, SearchIndex};
    let mut index = SearchIndex::new();
    let mut missing = cp.pruned_below != 0;
    let mut expected_height = 0;
    for block in cp.blocks.values() {
        if block.height != expected_height || (block.height != 0 && block.timestamp_ms == 0) {
            // Pre-gap resolver facts may have been changed by missing events.
            index = SearchIndex::new();
            missing = true;
        }
        expected_height = block.height.saturating_add(1);
        let at = block.timestamp_ms / 1000;
        index.apply(SearchEvent::Tick { at });
        for hash in &block.txs {
            if let Some((_, receipt)) = cp.receipts.get(hash).filter(|(height, _)| *height == block.height) {
                if receipt.success {
                    for event in &receipt.events {
                        for decoded in crate::search_events::decode_from_source(event, &cp.state, sources, at) { index.apply(decoded); }
                    }
                }
            } else { missing = true; }
        }
    }
    missing |= expected_height != cp.height.saturating_add(1);
    if missing { index.mark_history_incomplete(); }
    if cp.height != 0 { index.mark_usage_incomplete(); }
    index
}

fn keep<T: Serialize + ?Sized>(store: &Option<Arc<Store>>, key: &str, value: &T) {
    if let Some(store) = store {
        if let Err(e) = store.put_meta(key, &serde_json::to_vec(value).expect("serializes")) {
            tracing::warn!(%e, key, "could not keep the voting-set draw state");
        }
    }
}

/// What a proposal carries besides transactions.
#[derive(Clone, Default)]
pub struct Extras {
    pub handoff: Option<aether_light::block::Handoff>,
    pub seed: Option<aether_light::block::Seed>,
    pub upgrade: Option<crate::upgrade::SignedUpgrade>,
    pub proofs: Vec<aether_light::block::ProofClaim>,
    pub beacons: Vec<aether_light::block::BeaconAnswer>,
    /// The group the block belongs to (0 = the default; proposers set their
    /// chain's group, and the chain refuses any other).
    pub group: u16,
    pub registrations: Vec<aether_light::block::NodeRegistration>,
}

/// Build a payload on `parent`; `pre` is `Chain::pre_state` for the parent's next protocol.
pub fn build_payload(
    parent: &Executed,
    pre: &WorldState,
    ctx: &BlockContext,
    candidates: Vec<TxEnvelope>,
    extras: Extras,
) -> (Payload, aether_execution::BlockOutcome) {
    let group = extras.group;
    let candidates: Vec<_> = candidates.into_iter().filter(|tx| tx.header.group() == group).collect();
    let metered = ctx.limits.state <= fees::MAX_STATE_UNITS_PER_BLOCK;
    // Ordinary blocks keep the existing execution path and serialize once.
    // Only archive congestion needs incremental, exact payload selection.
    let (txs, out) = aether_execution::build_block(pre, ctx, candidates.clone());
    let payload = payload_with(parent, txs, &out, extras.clone(), metered);
    if !metered || payload.to_bytes().len() as u64 <= payload_archive_limit(parent, &payload) {
        return (payload, out);
    }
    let (txs, out) = aether_execution::block::build_block_filtered(pre, ctx, candidates, |txs, out| {
        let trial = payload_with(parent, txs.to_vec(), out, extras.clone(), true);
        trial.to_bytes().len() as u64 <= payload_archive_limit(parent, &trial)
    });
    (payload_with(parent, txs, &out, extras, metered), out)
}

fn payload_with(
    parent: &Executed,
    txs: Vec<TxEnvelope>,
    out: &BlockOutcome,
    extras: Extras,
    metered: bool,
) -> Payload {
    Payload {
        version: parent.next_protocol(),
        parent_state_root: parent.state.root(),
        history_root: B256::from(parent.history.root(&ChainHasher::new())),
        parent_meta: parent.meta_digest(),
        receipts_root: metered.then(|| aether_execution::receipt::receipt_root(&out.receipts)),
        txs,
        bal: out.bal.clone(),
        gas: out.gas,
        handoff: extras.handoff,
        seed: extras.seed,
        upgrade: extras.upgrade,
        proofs: extras.proofs,
        beacons: extras.beacons,
        group: extras.group,
        registrations: extras.registrations,
    }
}

/// Consensus archive capacity, with a certified reserve for control traffic.
/// Below the reserve, a block without controls can only carry the canonical
/// empty payload. Such blocks always leave room to refill rather than letting
/// optional traffic or small transactions indefinitely starve an upgrade.
pub fn payload_archive_limit(parent: &Executed, payload: &Payload) -> u64 {
    let available = fees::encoded_payload_limit(parent.archive_excess);
    if payload.handoff.is_some() || payload.seed.is_some() || payload.upgrade.is_some() {
        return available;
    }
    let mut empty = archive_control_payload(payload);
    empty.handoff = None;
    empty.seed = None;
    empty.upgrade = None;
    available.saturating_sub(fees::CONTROL_ARCHIVE_RESERVE)
        .max(empty.to_bytes().len() as u64)
        .min(available)
}

fn archive_control_payload(payload: &Payload) -> Payload {
    let mut control = payload.clone();
    control.txs.clear();
    control.proofs.clear();
    control.beacons.clear();
    control.registrations.clear();
    control.bal = Default::default();
    control.gas = GasVector::default();
    control.receipts_root = payload.receipts_root.map(|_| aether_execution::receipt::receipt_root(&[]));
    control
}

/// Every individual control item must fit the protected reserve, including
/// its empty header. Combined controls may use the full available bucket.
/// This is a hard liveness bound, independent of committee-signed contents.
fn controls_within_archive_reserve(payload: &Payload) -> bool {
    let base = archive_control_payload(payload);
    let mut handoff = base.clone();
    handoff.seed = None;
    handoff.upgrade = None;
    let mut seed = base.clone();
    seed.handoff = None;
    seed.upgrade = None;
    let mut upgrade = base;
    upgrade.handoff = None;
    upgrade.seed = None;
    [handoff, seed, upgrade].iter().all(|p| p.to_bytes().len() as u64 <= fees::CONTROL_ARCHIVE_RESERVE)
}

impl Extras {
    /// Select control traffic before optional system writes are applied. When
    /// a control item cannot fit, save capacity using an empty proposal until
    /// it can: optional proof traffic cannot starve a handoff or upgrade.
    pub fn fit_archive_budget(&mut self, parent: &Executed, metered: bool) -> bool {
        if !metered { return false; }
        let projected = |extras: &Self| Payload {
            version: parent.next_protocol(),
            parent_state_root: parent.state.root(),
            history_root: B256::from(parent.history.root(&ChainHasher::new())),
            parent_meta: parent.meta_digest(),
            receipts_root: Some(B256::ZERO),
            txs: vec![],
            bal: Default::default(),
            gas: GasVector::default(),
            handoff: extras.handoff.clone(),
            seed: extras.seed.clone(),
            upgrade: extras.upgrade.clone(),
            proofs: extras.proofs.clone(),
            beacons: extras.beacons.clone(),
            group: extras.group,
            registrations: extras.registrations.clone(),
        };
        if !controls_within_archive_reserve(&projected(self)) {
            // Such a signed control is invalid under this protocol; waiting
            // cannot make it fit the protected reserve. Keep proposing and
            // report the actionable need to sign a smaller control payload.
            tracing::warn!("control payload exceeds protected archive reserve; dropping from proposal");
            let mut one = self.clone();
            one.handoff = None;
            one.seed = None;
            if !controls_within_archive_reserve(&projected(&one)) { self.upgrade = None; }
            one = self.clone();
            one.upgrade = None;
            one.seed = None;
            if !controls_within_archive_reserve(&projected(&one)) { self.handoff = None; }
            one = self.clone();
            one.upgrade = None;
            one.handoff = None;
            if !controls_within_archive_reserve(&projected(&one)) { self.seed = None; }
        }
        let fits = |extras: &Self| {
            let payload = projected(extras);
            payload.to_bytes().len() as u64 <= payload_archive_limit(parent, &payload)
        };
        while !fits(self) {
            if self.proofs.pop().is_some() { continue; }
            if self.beacons.pop().is_some() { continue; }
            if self.registrations.pop().is_some() { continue; }
            // Independent upgrades go first. A handoff can require this same
            // block's seed to commit its roster, so retain that pair or publish
            // the seed alone and allow the handoff at the following height.
            let mut single = self.clone();
            single.handoff = None;
            single.seed = None;
            if single.upgrade.is_some() && fits(&single) { *self = single; return false; }
            single = self.clone();
            single.upgrade = None;
            if single.handoff.is_some() && fits(&single) { *self = single; return false; }
            single.handoff = None;
            if single.seed.is_some() && fits(&single) { *self = single; return false; }
            // Every remaining valid individual control fits the reserve, so
            // inability to fit implies capacity is below it. Empty blocks are
            // then also the only FOCIL-valid no-control proposal and refill.
            self.handoff = None;
            self.seed = None;
            self.upgrade = None;
            return true;
        }
        false
    }
}

/// An activation block's persisted diff starts with the activation's writes
/// (execution clears the journal it starts from).
#[allow(clippy::ptr_arg)] // Owned (migrated) vs Borrowed (untouched) is what matters here
pub fn with_activation(pre: &std::borrow::Cow<'_, WorldState>, out: &mut BlockOutcome) {
    if let std::borrow::Cow::Owned(migrated) = pre {
        out.state.prepend_journal(migrated.journal());
    }
}

/// Whether a block gets a proof-market statement (recorded by its child).
/// Protocol-1 blocks never do; under history v2 neither does a block with no
/// transactions and no proofs: there is nothing to prove, and an empty block
/// then leaves the state root and chain metadata as they were.
pub fn records_statement(cfg: &ChainConfig, payload: &Payload) -> bool {
    payload.version >= 2 && !(cfg.history_v2 && payload.txs.is_empty() && payload.proofs.is_empty())
}

/// The statement commitment of a block that ran `txs` on `pre` under `ctx`.
/// Its pre-state is the one the transactions ran on (after the block's system
/// writes: activation, records, payouts, which validators check directly).
pub fn statement(
    ctx: &BlockContext,
    txs: &[TxEnvelope],
    pre: &WorldState,
    out: &BlockOutcome,
) -> [u8; 32] {
    aether_proving::block::BlockStatement {
        ctx: ctx.clone(),
        txs_hash: aether_proving::block::txs_hash(txs),
        activate: vec![],
        pre_state_root: pre.root(),
        post_state_root: out.state.root(),
        gas: out.gas,
    }
    .commitment()
}

/// Check and pay a block's proofs (at most two, distinct heights, bounded,
/// each verified against the recorded statement of its block).
fn pay_proofs(
    state: &mut WorldState,
    height: u64,
    proofs: &[aether_light::block::ProofClaim],
    verifier: Option<&dyn ProofVerifier>,
) -> Result<Vec<(u64, Address, U256)>, ChainError> {
    let bad = |m: String| ChainError::Protocol(m);
    if proofs.len() > MAX_PROOFS_PER_BLOCK {
        return Err(bad(format!("more than {MAX_PROOFS_PER_BLOCK} proofs")));
    }
    let mut paid = Vec::new();
    for c in proofs {
        let verifier = verifier.ok_or_else(|| bad("no proof verifier on this node".into()))?;
        if c.proof.len() > 2 * MAX_PROOF_BYTES || c.height >= height {
            return Err(bad(format!("proof of block {} is out of bounds", c.height)));
        }
        let bytes = hex::decode(&c.proof).map_err(|_| bad("proof is not hex".into()))?;
        let commitment = aether_execution::proofs::claimable(state, c.height, height)
            .map_err(|e| bad(format!("proof of block {}: {e:?}", c.height)))?;
        // The proof's output binds the payout address: nobody can reroute it.
        match verifier.decide(&bytes, aether_proving::block::claim(commitment, c.prover)) {
            Some(true) => {}
            Some(false) => return Err(bad(format!("proof of block {} does not verify", c.height))),
            None => return Err(ChainError::Exec("proof verifier unavailable; no verdict on this block".into())),
        }
        let amount = if aether_rewards::enabled(state) {
            // Node rewards: issuance only to registered operators, 1/16 of the epoch's proof share each.
            aether_rewards::pay_proof(state, c.height, height, c.prover).map(|(paid, _)| paid)
        } else {
            aether_execution::proofs::pay(state, c.height, height, c.prover)
        }
        .map_err(|e| bad(format!("{e:?}")))?;
        paid.push((c.height, c.prover, amount));
    }
    Ok(paid)
}

/// `schedule` with `u`'s activation appended.
pub fn scheduled(
    schedule: &[crate::upgrade::Activation],
    u: &crate::upgrade::Upgrade,
) -> Arc<crate::upgrade::Schedule> {
    let mut next = schedule.to_vec();
    next.push(crate::upgrade::Activation {
        protocol: u.protocol,
        at: u.activate_at,
        registrar: u.registrar,
    });
    Arc::new(next)
}

#[cfg(test)]
mod pool_tests {
    use super::*;
    use aether_execution::EvmCall;
    use aether_types::{Bytes, TxHeader, TxPayload};

    const GWEI: u128 = 1_000_000_000;

    fn tx(sender: Address, nonce: u64, value: u64) -> TxEnvelope {
        let call = EvmCall {
            to: Some(Address::repeat_byte(0xaa)),
            value: U256::from(value),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        let payload = call.encode();
        let header = TxHeader {
            chain_id: 7780,
            sender,
            nonce,
            gas: GasVector {
                exec: 21_000,
                state: 0,
                prove: 21_000,
            },
            max_fee: FeeVector {
                exec: GWEI,
                state: 0,
                prove: 0,
            },
            tip: GWEI,
            payload_commitment: aether_execution::tx::payload_commitment(&payload),
            scheme: SignerScheme::P256,
            group: None,
        };
        TxEnvelope {
            header,
            payload: TxPayload::Plain(Bytes::from(payload)),
            signature: Bytes::new(),
        }
    }

    fn funded(a: Address, wei: u128) -> WorldState {
        let mut s = WorldState::default();
        s.set_balance(a, U256::from(wei)).unwrap();
        s
    }

    fn cfg(alloc: Vec<(Address, U256)>) -> ChainConfig {
        ChainConfig {
            chain_id: 7780,
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
            alloc,
            fees: false,
            registrar: None,
            epoch_blocks: 0,
            min_streak: None,
            draw_epochs: None,
            history_v2: false,
            protocol: 1,
            node_rewards: false,
            committee: vec![],
            reserve: None,
            group: 0,
            max_committee: crate::rotation::GROW_UNTIL,
        }
    }

    #[test]
    fn finalized_reward_records_keep_the_block_timestamp_in_milliseconds() {
        let dir = std::env::temp_dir().join(format!("aether-reward-timestamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("state.redb")).unwrap();
        let config = cfg(vec![]);
        let (chain, genesis) = Chain::open(config.clone(), store).unwrap();
        let parent = chain.get(&genesis.digest()).unwrap();
        let (block, exec) = build(&chain, &parent, &genesis, vec![]);
        let prover = Address::repeat_byte(0xc1);
        // Supply accepted payouts at the finalized-recording boundary; this
        // tests stored reward metadata without generating a cryptographic proof.
        let mut paid = (*exec).clone();
        paid.payouts = vec![(0, prover, U256::from(5)), (exec.height, prover, U256::from(7))];
        chain.lock().executed.insert(block.digest(), Arc::new(paid));
        chain.finalize(&block).unwrap();
        let rows = chain.rewards(&prover);
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row["timestamp_ms"], block.timestamp, "reward time uses the certified block's millisecond unit");
            assert_eq!(row["height"], exec.height);
        }
        assert!(rows.iter().any(|row| row["kind"] == "proof"));
        assert!(rows.iter().any(|row| row["kind"] == "node"));
        drop(chain);
        let (reopened, _) = Chain::open(config, Store::open(&dir.join("state.redb")).unwrap()).unwrap();
        let page = reopened.rewards_page(&prover, None, 10).unwrap();
        assert_eq!(page["total"], 2);
        for row in page["rewards"].as_array().unwrap() {
            assert_eq!(row["timestamp_ms"], block.timestamp, "persisted reward pages preserve milliseconds after restart");
        }
        drop(reopened);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn archive_reserve_refills_with_empty_blocks_and_selects_fitting_control() {
        let mut config = cfg(vec![]);
        config.chain_id = 7_778;
        config.protocol = 3;
        config.history_v2 = true;
        let (chain, genesis) = Chain::new(config);
        let mut parent = (*chain.get(&genesis.digest()).unwrap()).clone();
        parent.archive_excess = fees::MAX_ENCODED_PAYLOAD_BYTES - 40_000;
        let upgrade = crate::upgrade::SignedUpgrade {
            upgrade: aether_light::block::Upgrade {
                chain_id: 7_778, protocol: 4, activate_at: 604_800,
                emergency: false,
                releases: (0..16).map(|_| aether_light::block::Release {
                    platform: "p".repeat(512), version: "v".repeat(512),
                    blake3: "ab".repeat(32), url: "u".repeat(512),
                }).collect(),
                notes: "n".repeat(512), registrar: None,
            },
            signature: "ab".repeat(48), emergency_approvals: vec![],
        };
        let handoff = aether_light::block::Handoff {
            round: 1, output: "ab".repeat(30_000), members: vec![], ready: vec![],
            signature: "ab".repeat(48),
        };
        let mut extras = Extras { upgrade: Some(upgrade), handoff: Some(handoff), ..Default::default() };
        assert!(!extras.fit_archive_budget(&parent, true));
        assert!(extras.upgrade.is_some(), "a fitting upgrade is not blocked by the combined controls");
        assert!(extras.handoff.is_none(), "the larger handoff stays deferred");

        let ctx = Chain::block_context(&chain.cfg(), &genesis, &parent);
        let out = execute_block(&parent.state, &ctx, &[]).unwrap();
        let empty = payload_with(&parent, vec![], &out, Extras::default(), true);
        assert_eq!(payload_archive_limit(&parent, &empty), empty.to_bytes().len() as u64);
        assert!((empty.to_bytes().len() as u64) < fees::ENCODED_PAYLOAD_BYTES_PER_BLOCK);
        assert!(fees::next_archive_excess(parent.archive_excess, empty.to_bytes().len() as u64) < parent.archive_excess);
        let mut oversized = empty.clone();
        oversized.seed = Some(aether_light::block::Seed { draw: 1, signature: "x".repeat(fees::CONTROL_ARCHIVE_RESERVE as usize) });
        assert!(!controls_within_archive_reserve(&oversized));
    }

    #[test]
    fn new_genesis_keeps_a_proposal_across_short_draws_until_the_ceremony_window_ends() {
        let mut config = cfg(vec![]);
        config.chain_id = 7_799;
        config.registrar = Some(([1; 32], [2; 32]));
        config.epoch_blocks = 40;
        config.draw_epochs = Some(1);
        let (chain, _) = Chain::new(config);
        let mut g = chain.lock();
        g.committee.members = (0..4).map(|i| (i.to_string(), i.to_string())).collect();
        g.proposal = Some((2, vec![("candidate".into(), "node".into())]));
        assert!(proposal_in_window(&g, 120, &g.finalized.state), "the next draw keeps the first proposal");
        assert!(proposal_in_window(&g, 320, &g.finalized.state), "a late supervisor sees the same proposal");
        let boundary = crate::supervisor::reshare_attempt_blocks(5).unwrap();
        assert!(!proposal_in_window(&g, boundary, &g.finalized.state), "a later attempt can draw a new set");
        g.cfg.chain_id = 7_780;
        assert!(!proposal_in_window(&g, 120, &g.finalized.state), "7780 still expires proposals each draw");
    }

    /// An unsigned envelope (admission checks no signature) with `input` bytes
    /// of calldata and a 3M gas limit, as a deploy-sized call would carry.
    fn fat(sender: Address, nonce: u64, input: Vec<u8>) -> TxEnvelope {
        let call = EvmCall {
            to: Some(Address::repeat_byte(0xaa)),
            value: U256::ZERO,
            input: Bytes::from(input),
            gas_limit: 3_000_000,
            delegate: None,
        };
        let payload = call.encode();
        TxEnvelope {
            header: TxHeader {
                chain_id: 7780,
                sender,
                nonce,
                gas: GasVector { exec: 3_000_000, state: 0, prove: 3_000_000 },
                max_fee: FeeVector::default(),
                tip: 0,
                payload_commitment: aether_execution::tx::payload_commitment(&payload),
                scheme: SignerScheme::P256,
                group: None,
            },
            payload: TxPayload::Plain(Bytes::from(payload)),
            signature: Bytes::new(),
        }
    }

    /// `mempool_candidates` as it was before the 2026-09-29 fix (482400e):
    /// listed txs first, then the rest sorted by (nonce, sender). Kept verbatim
    /// so the regression tests below can show what it did to a listed tx whose
    /// sender's earlier nonces were still in the pool.
    fn candidates_2026_09_28(chain: &Chain) -> Vec<TxEnvelope> {
        let g = chain.lock();
        let censored = |t: &TxEnvelope| {
            g.censor == Some(t.header.sender) || g.deprioritize == Some(t.header.sender)
        };
        let listed = if g.censor.is_some() {
            Vec::new()
        } else {
            g.inclusion.for_proposal()
        };
        let listed_hashes: std::collections::HashSet<TxHash> =
            listed.iter().map(aether_execution::tx_hash).collect();
        let mut rest: Vec<TxEnvelope> = g
            .mempool
            .iter()
            .filter(|(h, t)| !listed_hashes.contains(*h) && !censored(t))
            .map(|(_, t)| t.clone())
            .collect();
        rest.sort_by_key(|t| (t.header.nonce, t.header.sender));
        let mut txs = listed;
        txs.extend(rest);
        txs.truncate(MAX_TXS_PER_BLOCK);
        txs
    }

    /// The 2026-09-28 fix's single nonce order over the whole pool, kept as the
    /// flood regression's baseline: it is what let 2,000 unrelated nonce-0 txs
    /// push a listed tx at a later nonce past the block's tx budget, block
    /// after block, without the inclusion check ever seeing it appendable.
    fn candidates_one_nonce_order(chain: &Chain) -> Vec<TxEnvelope> {
        let g = chain.lock();
        let censored = |t: &TxEnvelope| {
            g.censor == Some(t.header.sender) || g.deprioritize == Some(t.header.sender)
        };
        let listed = if g.censor.is_some() {
            Vec::new()
        } else {
            g.inclusion.for_proposal()
        };
        let listed_hashes: std::collections::HashSet<TxHash> =
            listed.iter().map(aether_execution::tx_hash).collect();
        let rest: Vec<TxEnvelope> = g
            .mempool
            .iter()
            .filter(|(h, t)| !listed_hashes.contains(*h) && !censored(t))
            .map(|(_, t)| t.clone())
            .collect();
        let mut txs: Vec<(bool, TxEnvelope)> = rest.into_iter().map(|t| (false, t)).collect();
        txs.extend(listed.into_iter().map(|t| (true, t)));
        txs.sort_by_key(|(listed, t)| (t.header.nonce, !*listed, t.header.sender));
        let mut txs: Vec<TxEnvelope> = txs.into_iter().map(|(_, t)| t).collect();
        txs.truncate(MAX_TXS_PER_BLOCK);
        txs
    }

    /// Signed transfers from `signer`, one per nonce in `nonces`: cheap enough
    /// that a block holds them all, so what lands is exactly what the ordering
    /// tried, in the order it tried it.
    fn transfers(key: &aether_crypto::P256Signer, nonces: std::ops::Range<u64>) -> Vec<TxEnvelope> {
        nonces
            .map(|nonce| {
                aether_execution::sign_call(key, 7780, nonce, 1, &EvmCall {
                    to: Some(Address::repeat_byte(0xaa)),
                    value: U256::from(1),
                    input: Bytes::new(),
                    gas_limit: 21_000,
                    delegate: None,
                })
                .unwrap()
            })
            .collect()
    }

    /// An inclusion list holding `txs` this node accepted at `seen` (bypassing
    /// gossip verification, which is another node's job).
    fn accept_list(chain: &Chain, txs: Vec<TxEnvelope>, seen: Instant) {
        use commonware_cryptography::Signer as _;
        let key = commonware_cryptography::ed25519::PrivateKey::from_seed(1);
        let il = inclusion::InclusionList::sign(&key, 1, 1_000, txs);
        assert!(chain.lock().inclusion.accept(&il, seen));
    }

    /// Build and execute (but not finalize) the block after `last` with `txs`,
    /// also returning its `BlockContext`: the append check must judge the block
    /// in the same context that built it.
    fn build_ctx(
        chain: &Chain,
        parent: &Arc<Executed>,
        last: &Block,
        txs: Vec<TxEnvelope>,
    ) -> (Arc<Executed>, BlockContext) {
        use crate::block::{Context, EPOCH};
        use commonware_consensus::types::{Round, View};
        use commonware_cryptography::Signer as _;
        let height = last.height.next();
        let leader = commonware_cryptography::ed25519::PrivateKey::from_seed(1).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), last.digest()) };
        let skeleton = Block::new(context.clone(), last.digest(), height, height.get() * 1_000, bytes::Bytes::new());
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
        let (pre, _) = chain.pre_state_with(parent, parent.next_protocol(), &[], &[], &[], None, false).unwrap();
        let (payload, _) = build_payload(parent, &pre, &ctx, txs, Extras::default());
        let block = Block::new(context, last.digest(), height, height.get() * 1_000, payload.to_bytes());
        (chain.execute(&block, parent).unwrap(), ctx)
    }

    /// `sender`'s nonces that made it into `exec` (from `txs`), in block order.
    fn landed_nonces(exec: &Executed, txs: &[TxEnvelope], sender: Address) -> Vec<u64> {
        let mine: HashMap<TxHash, u64> = txs
            .iter()
            .filter(|t| t.header.sender == sender)
            .map(|t| (aether_execution::tx_hash(t), t.header.nonce))
            .collect();
        exec.tx_hashes.iter().filter_map(|h| mine.get(h).copied()).collect()
    }

    /// Build and execute (but not finalize) the block after `last` with `txs`.
    fn build(chain: &Chain, parent: &Arc<Executed>, last: &Block, txs: Vec<TxEnvelope>) -> (Block, Arc<Executed>) {
        use crate::block::{Context, EPOCH};
        use commonware_consensus::types::{Round, View};
        use commonware_cryptography::Signer as _;
        let height = last.height.next();
        let leader = commonware_cryptography::ed25519::PrivateKey::from_seed(1).public_key();
        let context = Context { round: Round::new(EPOCH, View::new(height.get())), leader, parent: (View::new(height.get() - 1), last.digest()) };
        let skeleton = Block::new(context.clone(), last.digest(), height, height.get() * 1_000, bytes::Bytes::new());
        let ctx = Chain::block_context(&chain.cfg(), &skeleton, parent);
        let (pre, _) = chain.pre_state_with(parent, parent.next_protocol(), &[], &[], &[], None, false).unwrap();
        let (payload, _) = build_payload(parent, &pre, &ctx, txs, Extras::default());
        let block = Block::new(context, last.digest(), height, height.get() * 1_000, payload.to_bytes());
        let exec = chain.execute(&block, parent).unwrap();
        (block, exec)
    }

    /// The 2026-10-06 rehearsal failure: a new-genesis (history v2) chain
    /// whose blocks carry only system writes (beacon answers, free-lane
    /// registrations) records no statement, so a prover has nothing to prove
    /// and no proof reward can ever be paid. The first block with a paid
    /// transaction is provable, and its prover input restates exactly the
    /// statement the chain recorded.
    #[test]
    fn a_quiet_history_v2_chain_has_nothing_to_prove_until_a_transaction_lands() {
        let key = aether_crypto::P256Signer::from_seed(&[7; 32]).unwrap();
        let sender = aether_crypto::address_of(&aether_crypto::Signer::public_key(&key)).unwrap();
        let mut config = cfg(vec![(sender, U256::from(10u128.pow(21)))]);
        config.protocol = 3;
        config.history_v2 = true;
        let (chain, genesis) = Chain::new(config);
        let prover = Address::repeat_byte(0xc1);
        chain.set_prover_window(crate::prover_assignment::Config::default().window);
        let mut parent = chain.get(&genesis.digest()).unwrap();
        let mut last = genesis.clone();
        for _ in 0..4 {
            let (block, exec) = build(&chain, &parent, &last, vec![]);
            chain.finalize(&block).unwrap();
            assert_eq!(exec.statement, Statement::default(), "an empty v2 block records no statement");
            (parent, last) = (exec, block);
        }
        assert!(
            matches!(crate::prover::next_job(&chain, prover), Ok(None)),
            "only empty blocks: nothing to prove"
        );
        // A paid transfer with a state budget, as `aether send` signs one on a new genesis.
        let mut transfer = transfers(&key, 0..1).remove(0);
        transfer.header.gas.state = 1_000;
        transfer.header.max_fee.state = Chain::next_base_fee(&chain.cfg(), &parent).state;
        let mut sig = aether_crypto::Signer::sign(&key, &transfer.signing_bytes()).unwrap();
        sig.extend_from_slice(&aether_crypto::Signer::public_key(&key).bytes);
        transfer.signature = Bytes::from(sig);
        let (block, exec) = build(&chain, &parent, &last, vec![transfer]);
        chain.finalize(&block).unwrap();
        assert_eq!(exec.tx_hashes.len(), 1, "the transfer landed");
        assert_ne!(exec.statement, Statement::default(), "a block with a transaction records a statement");
        let (height, txs, input) = crate::prover::next_job(&chain, prover)
            .expect("the input builds")
            .expect("the transfer's block is provable");
        assert_eq!((height, txs), (block.height().get(), 1));
        // The sidecar and its guest decode the bytes the node hands over
        // (postcard::from_bytes::<BlockInput>, apps/prover): the plain derive
        // drops an ungrouped tx's `group` and cannot be read back.
        assert!(
            postcard::from_bytes::<aether_proving::block::BlockInput>(&postcard::to_allocvec(&input).unwrap()).is_err(),
            "the derive's bytes are what the 2026-10-06 sidecar refused"
        );
        let bytes = crate::prover_input::encode(&input).expect("the prover input encodes");
        let back: aether_proving::block::BlockInput = postcard::from_bytes(&bytes).expect("the sidecar decodes it");
        assert_eq!(back.txs, input.txs);
        assert_eq!(
            aether_proving::block::output(&back).unwrap(),
            aether_proving::block::claim(exec.statement.commitment, prover),
            "the guest's output from the decoded input is the claim validators check"
        );
        assert_eq!(
            aether_proving::block::execute(&input).unwrap().commitment(),
            exec.statement.commitment,
            "the proof would state what the chain recorded"
        );
        assert!(matches!(crate::prover::next_job(&chain, prover), Ok(None)), "each block is taken once");
    }

    #[test]
    fn quiet_heads_keep_their_authenticated_parent_metadata() {
        for (protocol, history_v2) in [(1, false), (3, true)] {
            let mut config = cfg(vec![]);
            config.protocol = protocol;
            config.history_v2 = history_v2;
            let (chain, genesis) = Chain::new(config);
            let mut parent = chain.lock().finalized.clone();
            let mut last = genesis;
            for _ in 0..3 {
                let (block, exec) = build(&chain, &parent, &last, vec![]);
                chain.finalize(&block).unwrap();
                assert_eq!(exec.statement, Statement::default());
                let witness = upgrade_metadata(&chain.lock()).expect("quiet finalized heads still authenticate their parent's metadata");
                assert_eq!(witness["height"], parent.height);
                let encoded = aether_light::from_hex(witness["encoded"].as_str().unwrap()).unwrap();
                assert_eq!(aether_light::chain_meta_digest(&encoded, witness["archive_excess"].as_u64().unwrap()), block.payload().unwrap().parent_meta);
                (parent, last) = (exec, block);
            }
        }
    }

    #[test]
    fn h04_disabled_proving_keeps_only_the_ordinary_state_versions() {
        let mut config = cfg(vec![]);
        config.protocol = 2;
        let (chain, mut last) = Chain::new(config);
        let mut parent = chain.lock().finalized.clone();
        for _ in 0..160 {
            let (block, exec) = build(&chain, &parent, &last, vec![]);
            chain.finalize(&block).unwrap();
            (parent, last) = (exec, block);
        }
        let g = chain.lock();
        assert_eq!(g.executed.len(), 65, "a node without a prover must not pin unpaid statements or parents");
        assert!(g.recent.is_empty(), "disabled proving has no statement window");
    }

    fn h04_budgeted_rescue(sparse: bool, window: usize) {
        let key = aether_crypto::P256Signer::from_seed(&[7; 32]).unwrap();
        let sender = aether_crypto::address_of(&aether_crypto::Signer::public_key(&key)).unwrap();
        let mut alloc = vec![(sender, U256::from(10u128.pow(21)))];
        for i in 0..1024u64 {
            let mut bytes = [0u8; 20];
            bytes[12..].copy_from_slice(&i.to_be_bytes());
            alloc.push((Address::from(bytes), U256::from(1)));
        }
        let mut config = cfg(alloc);
        config.protocol = 3;
        config.history_v2 = sparse;
        let (chain, mut last) = Chain::new(config);
        chain.set_prover_window(window);
        // Scale the documented 8 GB Mac's 1 GB cache budget down by 128 to
        // expose state-copy growth without allocating gigabytes in a test.
        let budget = crate::resources::cache_budget_for(8 * crate::resources::GB) / 128;
        assert_eq!(budget, 8 * 1024 * 1024);
        chain.trim_history_caches_with(budget);
        let mut parent = chain.lock().finalized.clone();
        let mut nonce = 0;
        let mut first_commitment = [0; 32];
        for h in 1..=240 {
            let txs = if sparse && h % 4 == 1 {
                let mut transfer = transfers(&key, nonce..nonce + 1).remove(0);
                nonce += 1;
                transfer.header.gas.state = 1_000;
                transfer.header.max_fee.state = Chain::next_base_fee(&chain.cfg(), &parent).state;
                let mut signature = aether_crypto::Signer::sign(&key, &transfer.signing_bytes()).unwrap();
                signature.extend_from_slice(&aether_crypto::Signer::public_key(&key).bytes);
                transfer.signature = Bytes::from(signature);
                vec![transfer]
            } else { vec![] };
            let (block, exec) = build(&chain, &parent, &last, txs);
            chain.finalize(&block).unwrap();
            if h == 1 { first_commitment = exec.statement.commitment; }
            (parent, last) = (exec, block);
        }
        chain.trim_history_caches_with(budget);
        let (states, versions) = {
            let g = chain.lock();
            let bytes = g.executed.values().map(|e| {
                e.state.repo().entries().count() as u64 * 64
                    + e.state.codes().values().map(|c| c.len() as u64).sum::<u64>()
            }).sum::<u64>();
            (bytes, g.executed.len())
        };
        let charged = chain.caches_bytes();
        let measured = states.max(charged);
        println!("H04 sparse={sparse} window={window}: state_payload={states}, charged={charged}, versions={versions}, budget={budget}");
        assert!(measured <= budget * 3 / 4, "retention must fit the measured budget with 25% headroom: {measured} > {}", budget * 3 / 4);
        assert!(charged >= states, "state versions must be charged to the history budget");
        assert!(versions <= 65, "compact rescue inputs must not pin additional full states");
        let prover = Address::repeat_byte(0xf0);
        let assignment = crate::prover_assignment::Config { window, ..Default::default() };
        let now_ms = last.timestamp + assignment.grace.as_millis() as u64 + 1;
        let (height, _, input) = chain.proving_input_for(prover, &assignment, now_ms).unwrap().expect("a grace rescue still has a usable compact input");
        assert_eq!(height, 1, "the oldest unpaid statement survives quiet blocks and budget trimming");
        assert_eq!(aether_proving::block::execute(&input).unwrap().commitment(), first_commitment);
        assert_eq!(aether_proving::block::output(&input).unwrap(), aether_proving::block::claim(first_commitment, prover));
    }

    #[test]
    fn h04_adjacent_unpaid_history_fits_budget_and_can_rescue() {
        h04_budgeted_rescue(false, crate::prover_assignment::MAX_WINDOW);
    }

    #[test]
    fn h04_sparse_unpaid_history_fits_budget_and_can_rescue() {
        h04_budgeted_rescue(true, crate::prover_assignment::MAX_WINDOW);
    }

    #[test]
    fn a_checkpoint_proof_notifies_a_flight_evicted_from_a_small_window() {
        let mut config = cfg(vec![]);
        config.protocol = 2;
        let (chain, genesis) = Chain::new(config);
        chain.set_prover_window(1);
        let parent = chain.lock().finalized.clone();
        let (first, parent) = build(&chain, &parent, &genesis, vec![]);
        chain.finalize(&first).unwrap();
        chain.lock().attempted.insert(1);
        chain.set_proving_height(Some(1));
        let notices = chain.observe_proofs();
        let (second, _) = build(&chain, &parent, &first, vec![]);
        chain.finalize(&second).unwrap();
        assert!(!chain.lock().attempted.contains(&1), "the pending window already evicted this job");
        // Adopt's input is certified by its caller. Here construct the paid
        // proof marker directly to isolate the notification bookkeeping.
        let mut imported = (*chain.lock().finalized).clone();
        aether_execution::proofs::pay(&mut imported.state, 1, 3, Address::repeat_byte(0x88)).unwrap();
        let summary = chain.lock().blocks[&2].clone();
        chain.adopt(Arc::new(imported), summary);
        assert_eq!(notices.recv_timeout(std::time::Duration::from_secs(1)).unwrap().0, 1);
    }

    #[test]
    fn a_tx_its_sender_can_no_longer_pay_for_leaves_the_pool() {
        let a = Address::repeat_byte(1);
        let t = tx(a, 0, 1_000);
        let now = Instant::now();
        let enough = funded(a, 21_000 * GWEI + 1_000);
        assert!(keep_in_pool(
            &t,
            Some(now),
            now,
            &enough,
            FeeVector::default(),
            true
        ));
        let drained = funded(a, 10);
        assert!(
            !keep_in_pool(&t, Some(now), now, &drained, FeeVector::default(), true),
            "admitted while affordable, then the balance went elsewhere"
        );
        assert!(
            keep_in_pool(&t, Some(now), now, &drained, FeeVector::default(), false),
            "without fees there is nothing to pay"
        );
    }

    #[test]
    fn a_tx_waiting_past_the_ttl_leaves_even_if_it_could_still_run() {
        let a = Address::repeat_byte(2);
        let gapped = tx(a, 5, 1); // nonce 0..4 never arrive: it can never run
        let state = funded(a, 10u128.pow(18));
        let t0 = Instant::now();
        assert!(keep_in_pool(
            &gapped,
            Some(t0),
            t0 + MEMPOOL_TTL / 2,
            &state,
            FeeVector::default(),
            true
        ));
        assert!(!keep_in_pool(
            &gapped,
            Some(t0),
            t0 + MEMPOOL_TTL,
            &state,
            FeeVector::default(),
            true
        ));
    }

    #[test]
    fn fee_caps_below_the_base_fee_wait_rather_than_leave() {
        let a = Address::repeat_byte(3);
        let t = tx(a, 0, 1);
        let state = funded(a, 10u128.pow(18));
        let now = Instant::now();
        let high = FeeVector {
            exec: 5 * GWEI,
            state: 0,
            prove: 0,
        };
        assert!(
            admissible(&t, &state, high).is_err(),
            "not admitted while the base fee is above its cap"
        );
        assert!(
            keep_in_pool(&t, Some(now), now, &state, high, true),
            "but an already pending one may wait for the fee to fall"
        );
    }

    /// A transfer signed with a state cap, as a wallet signs it on a paid-state chain.
    fn capped(sender: Address, nonce: u64, state_cap: u128) -> TxEnvelope {
        let mut t = tx(sender, nonce, 1);
        t.header.gas.state = 300;
        t.header.max_fee.state = state_cap;
        t
    }

    #[test]
    fn r14_a_state_price_rise_keeps_a_signed_budget_affordable_tx_pending() {
        let a = Address::repeat_byte(4);
        let cap = 2 * fees::STATE_UNIT_PRICE;
        let mut t = capped(a, 0, cap);
        t.header.max_fee.prove = GWEI;
        let signed_budget = u128::from(t.header.gas.exec) * t.header.max_fee.exec
            + u128::from(t.header.gas.prove) * t.header.max_fee.prove
            + u128::from(t.header.gas.state) * cap
            + 1;
        let state = funded(a, signed_budget);
        let paid = FeeVector { exec: GWEI, state: cap, prove: GWEI };
        assert!(admissible(&t, &state, paid).is_ok(), "the complete signed maximum is affordable");
        let price = fees::state_base_fee(97_147);
        assert!(price > cap);
        let high = FeeVector { state: price, ..paid };
        let t0 = Instant::now();
        assert_eq!(
            drop_reason(&t, Some(t0), t0 + MEMPOOL_TTL / 2, &state, high, true, None),
            None,
            "R14: an unchargeable state price must not drop a signed-budget-affordable transaction"
        );
        assert_eq!(drop_reason(&t, Some(t0), t0 + MEMPOOL_TTL / 2, &state, paid, true, None), None,
            "the transaction is retained when the price refills to its cap");
        assert_eq!(
            drop_reason(&t, Some(t0), t0 + MEMPOOL_TTL, &state, high, true, None),
            Some(DropReason::StatePriceAboveCap { cap: cap.to_string(), price: price.to_string(), blocks: None }),
            "the existing TTL still applies to fee-cap waiting"
        );
        assert_eq!(
            drop_reason(&t, Some(t0), t0, &funded(a, signed_budget - 1), high, true, None),
            Some(DropReason::Unaffordable),
            "a real balance loss is still detected while the state price is high"
        );
    }

    #[test]
    fn r14_a_prove_price_rise_keeps_a_signed_budget_affordable_tx_pending() {
        let a = Address::repeat_byte(4);
        let mut t = tx(a, 0, 1);
        t.header.max_fee.prove = GWEI;
        let signed_budget = u128::from(t.header.gas.exec) * t.header.max_fee.exec
            + u128::from(t.header.gas.prove) * t.header.max_fee.prove
            + 1;
        let state = funded(a, signed_budget);
        let paid = FeeVector { exec: GWEI, state: 0, prove: GWEI };
        assert!(admissible(&t, &state, paid).is_ok());
        let high = FeeVector { prove: 2 * GWEI, ..paid };
        let t0 = Instant::now();
        assert_eq!(
            drop_reason(&t, Some(t0), t0, &state, high, true, None),
            None,
            "R14: an unchargeable prove price must not drop a signed-budget-affordable transaction"
        );
        assert_eq!(
            drop_reason(&t, Some(t0), t0 + MEMPOOL_FEE_WAIT, &state, high, true, None),
            Some(DropReason::FeeCapBelowBase),
            "the existing exec/prove fee waiting limit still applies"
        );
    }

    /// Contracts-live bug #5: the stress run's 78 transfers sat under the
    /// risen B5 price until the TTL and vanished with no reason anywhere.
    /// The rule that drops them is unchanged; the reason is now named.
    #[test]
    fn a_tx_the_state_price_outran_leaves_with_that_reason() {
        let a = Address::repeat_byte(4);
        let cap = 2 * fees::STATE_UNIT_PRICE;
        let t = capped(a, 0, cap);
        let state = funded(a, 10u128.pow(20));
        let price = fees::state_base_fee(97_147); // after the stress burst block
        let base = FeeVector { exec: 0, state: price, prove: 0 };
        let t0 = Instant::now();
        assert_eq!(drop_reason(&t, Some(t0), t0 + MEMPOOL_TTL / 2, &state, base, true, None), None, "it still waits");
        assert_eq!(
            drop_reason(&t, Some(t0), t0 + MEMPOOL_TTL, &state, base, true, Some(0)),
            Some(DropReason::StatePriceAboveCap { cap: cap.to_string(), price: price.to_string(), blocks: None }),
            "the price, not the gap, is what it waited for"
        );
        // A tx whose cap meets the price leaves at the TTL for other reasons.
        let paid = FeeVector { state: fees::STATE_UNIT_PRICE, ..base };
        assert_eq!(drop_reason(&t, Some(t0), t0 + MEMPOOL_TTL, &state, paid, true, Some(3)), Some(DropReason::NonceGap { expected: 3 }));
        assert_eq!(drop_reason(&t, Some(t0), t0 + MEMPOOL_TTL, &state, paid, true, None), Some(DropReason::Expired));
        // The balance went elsewhere.
        assert_eq!(drop_reason(&t, Some(t0), t0, &funded(a, 10), paid, true, None), Some(DropReason::Unaffordable));
    }

    #[test]
    fn a_nonce_gap_is_the_first_nonce_neither_chain_nor_pool_has() {
        let (a, b) = (Address::repeat_byte(5), Address::repeat_byte(6));
        let state = funded(a, 1);
        let pool: BTreeMap<TxHash, TxEnvelope> = [tx(a, 0, 1), tx(a, 1, 1), tx(a, 3, 1), tx(b, 1, 1)]
            .into_iter()
            .map(|t| (aether_execution::tx_hash(&t), t))
            .collect();
        let gaps = first_missing_nonces(&pool, &state);
        assert_eq!(gaps[&a], 2, "0 and 1 run; 3 waits for 2");
        assert_eq!(gaps[&b], 0, "b's nonce 1 waits for 0");
    }

    /// A pending tx under the state price says so over RPC, with the price,
    /// its cap and how many blocks the refill needs (bug #5).
    #[test]
    fn a_pending_tx_says_what_it_waits_for() {
        let a = Address::repeat_byte(8);
        let (chain, _) = Chain::new(cfg(vec![(a, U256::from(10u128.pow(22)))]));
        let mut g = chain.lock();
        g.cfg.history_v2 = true; // a paid-state genesis prices state
        let mut head = (*g.finalized).clone();
        head.excess.state = 97_147;
        g.finalized = Arc::new(head);
        let cap = 2 * fees::STATE_UNIT_PRICE;
        let stuck = capped(a, 0, cap);
        let price = fees::state_base_fee(97_147);
        assert_eq!(
            pending_reason(&g, &stuck),
            Some(DropReason::StatePriceAboveCap {
                cap: cap.to_string(),
                price: price.to_string(),
                blocks: fees::blocks_until_state_price_at_most(97_147, cap),
            })
        );
        let blocks = fees::blocks_until_state_price_at_most(97_147, cap).unwrap();
        assert!(blocks > 1_000, "{blocks}");
        // Behind a gap: nonce 1 with nonce 0 nowhere.
        let behind = capped(a, 1, price);
        assert_eq!(pending_reason(&g, &behind), Some(DropReason::NonceGap { expected: 0 }));
        g.insert_pending(aether_execution::tx_hash(&stuck), stuck.clone());
        assert_eq!(pending_reason(&g, &behind), None, "nonce 0 is pending: only its turn holds it");
    }

    /// B5 review round 2, finding 4: a receipt read for one pending tx looks
    /// at its own sender's queue (≤ 64 nonces, from an index), not the whole
    /// pool, under the chain lock — before the fix the gap check scanned all
    /// 2,001 entries here (up to 50,000 on a full pool). The index follows the
    /// pool through admission, eviction and finalization, and the answer is
    /// the same as before.
    #[test]
    fn a_pending_lookup_reads_only_its_senders_entries() {
        let a = Address::repeat_byte(8);
        let (chain, _) = Chain::new(cfg(vec![(a, U256::from(10u128.pow(22)))]));
        let mut g = chain.lock();
        let cap = 2 * fees::STATE_UNIT_PRICE;
        for i in 0..2_000u64 {
            let mut b = [0u8; 20];
            b[12..].copy_from_slice(&(i + 1_000).to_be_bytes());
            let t = capped(Address::from(b), 0, cap);
            g.insert_pending(aether_execution::tx_hash(&t), t);
        }
        let first = capped(a, 0, cap);
        g.insert_pending(aether_execution::tx_hash(&first), first);
        let behind = capped(a, 1, cap);
        PENDING_SCANNED.with(|c| c.set(0));
        assert_eq!(pending_reason(&g, &behind), None, "nonce 0 is pending: only its turn holds it");
        let scanned = PENDING_SCANNED.with(|c| c.get());
        assert!(scanned <= MAX_PER_SENDER, "one receipt read examined {scanned} pool entries under the chain lock");
        assert_eq!(pending_reason(&g, &capped(a, 2, cap)), Some(DropReason::NonceGap { expected: 1 }));
        // The facts are copied out; the reason needs no lock.
        let facts = pending_facts(&g, &capped(a, 2, cap));
        drop(g);
        assert_eq!(facts.reason(), Some(DropReason::NonceGap { expected: 1 }));
    }

    /// The sender index stays in step with the pool: admitted and finalized
    /// txs come and go from it exactly as from `mempool`.
    #[test]
    fn the_sender_nonce_index_follows_the_pool() {
        let key = aether_crypto::P256Signer::from_seed(&[9; 32]).unwrap();
        let a = address_of(&aether_crypto::Signer::public_key(&key)).unwrap();
        let (chain, genesis) = Chain::new(cfg(vec![(a, U256::from(10u128.pow(22)))]));
        let parent = chain.get(&genesis.digest()).unwrap();
        let txs = transfers(&key, 0..3);
        for t in &txs {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        let index = |g: &Inner| g.nonces_by_sender.get(&a).map(|n| n.keys().copied().collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!(index(&chain.lock()), vec![0, 1, 2]);
        let (block, _) = build(&chain, &parent, &genesis, vec![txs[0].clone()]);
        chain.finalize(&block).unwrap();
        let g = chain.lock();
        assert_eq!(index(&g), vec![1, 2], "the included nonce left the index with the pool entry");
        let mut rebuilt: Vec<u64> = g.mempool.values().filter(|t| t.header.sender == a).map(|t| t.header.nonce).collect();
        rebuilt.sort_unstable();
        assert_eq!(index(&g), rebuilt);
    }

    /// Departures at finalization leave tombstones: a gapped tx past the TTL
    /// (`nonce_gap`) and a same-nonce twin of an included tx (`replaced`);
    /// the included tx itself has a receipt, never a tombstone.
    #[test]
    fn finalized_departures_leave_tombstones() {
        let key = aether_crypto::P256Signer::from_seed(&[9; 32]).unwrap();
        let a = address_of(&aether_crypto::Signer::public_key(&key)).unwrap();
        let (chain, genesis) = Chain::new(cfg(vec![(a, U256::from(10u128.pow(22)))]));
        let parent = chain.get(&genesis.digest()).unwrap();
        let landing = transfers(&key, 0..1).remove(0);
        // Its twin: nonce 0 again, another payload.
        let twin = aether_execution::sign_call(&key, 7780, 0, 1, &EvmCall {
            to: Some(Address::repeat_byte(0xbb)),
            value: U256::from(2),
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        })
        .unwrap();
        // Nonce 5 of the same sender, waiting past the TTL behind nonces 1..4.
        let gapped = transfers(&key, 5..6).remove(0);
        let (hl, ht, hg) = (aether_execution::tx_hash(&landing), aether_execution::tx_hash(&twin), aether_execution::tx_hash(&gapped));
        for t in [&landing, &twin, &gapped] {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        chain.lock().arrivals.insert(hg, Instant::now() - MEMPOOL_TTL);
        let (block, exec) = build(&chain, &parent, &genesis, vec![landing]);
        assert_eq!(exec.tx_hashes, vec![hl]);
        chain.finalize(&block).unwrap();
        let mut g = chain.lock();
        assert!(g.mempool.is_empty(), "the twin and the gapped tx left, as before");
        assert_eq!(g.tombstones.get(&ht), Some(DropReason::Replaced));
        assert_eq!(g.tombstones.get(&hg), Some(DropReason::NonceGap { expected: 1 }));
        assert_eq!(g.tombstones.get(&hl), None, "included: a receipt, no tombstone");
        assert!(g.receipts.contains_key(&hl));
    }

    #[test]
    fn a_tx_over_the_size_cap_is_refused_while_any_legit_deploy_fits() {
        let sender = Address::repeat_byte(7);
        let (chain, _) = Chain::new(cfg(vec![(sender, U256::from(10u128.pow(22)))]));
        let err = chain.add_to_mempool(fat(sender, 0, vec![9u8; 200 * 1024])).unwrap_err();
        assert!(err.contains("over the") && err.contains("cap"), "{err}");
        assert!(chain.lock().mempool.is_empty(), "nothing was admitted");
        // Deploy initcode is capped at 49 KiB by the EVM (EIP-3860); even twice
        // that stays a normal pool citizen.
        assert_eq!(chain.add_to_mempool(fat(sender, 0, vec![9u8; 100 * 1024])), Ok(true));
        let g = chain.lock();
        let (h, t) = g.mempool.iter().next().unwrap();
        assert_eq!(g.mempool_bytes, t.to_canonical_bytes().len());
        assert_eq!(g.mempool_bytes, g.sizes[h]);
    }

    #[test]
    fn the_byte_budget_bounds_the_pool_and_comes_back_with_a_block() {
        use aether_crypto::{P256Signer, Signer};
        let keys: Vec<P256Signer> = (0..17u8)
            .map(|i| {
                let mut s = [0u8; 32];
                s[0] = 0x5a;
                s[31] = i + 1;
                P256Signer::from_seed(&s).unwrap()
            })
            .collect();
        let senders: Vec<Address> = keys.iter().map(|k| address_of(&k.public_key()).unwrap()).collect();
        let (chain, genesis) = Chain::new(cfg(senders.iter().cloned().map(|a| (a, U256::from(10u128.pow(22)))).collect()));

        // The first sender's earliest nonces, signed, so a block can land them.
        // 130 KB of nonzero calldata floors at 21k + 130k×40 = 5.22M gas
        // (EIP-7623), so the calls carry 6M each.
        let landing: Vec<TxEnvelope> = (0..12)
            .map(|nonce| {
                aether_execution::sign_call(&keys[0], 7780, nonce, 1, &EvmCall {
                    to: Some(Address::repeat_byte(0xaa)),
                    value: U256::ZERO,
                    input: Bytes::from(vec![7u8; 130_000]),
                    gas_limit: 6_000_000,
                    delegate: None,
                })
                .unwrap()
            })
            .collect();
        // Fill past the 64 MiB budget: plenty of count slots, every sender
        // under its per-sender cap, every tx under the size cap, so the byte
        // budget is what stops this. The signed txs go in first so the block
        // below lands exactly what the pool holds.
        let mut admitted = 0;
        let mut full = String::new();
        for t in &landing {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
            admitted += 1;
        }
        'fill: for (i, sender) in senders[0..16].iter().enumerate() {
            let nonces = if i == 0 { 12..MAX_PER_SENDER as u64 } else { 0..MAX_PER_SENDER as u64 };
            for nonce in nonces {
                match chain.add_to_mempool(fat(*sender, nonce, vec![7u8; 130_000])) {
                    Ok(true) => admitted += 1,
                    Err(e) => {
                        full = e;
                        break 'fill;
                    }
                    Ok(false) => unreachable!("a fresh nonce cannot be known"),
                }
            }
        }
        assert_eq!(full, "mempool byte budget full", "the budget, not another cap, fills first");
        {
            let g = chain.lock();
            // One more fat tx would not have fit: the budget, not the loop, stopped it.
            assert!(MAX_MEMPOOL_BYTES - g.mempool_bytes < 130_200);
            assert!(g.mempool_bytes <= MAX_MEMPOOL_BYTES);
            assert_eq!(g.mempool_bytes, g.sizes.values().sum::<usize>(), "the budget is exactly the pool");
            assert_eq!(g.sizes.len(), g.mempool.len());
        }

        // A block lands the first sender's signed earliest nonces (each floors
        // at ~5.2M gas, so a 30M block holds about five); their bytes leave the
        // budget.
        let parent = chain.lock().finalized.clone();
        let (block, exec) = build(&chain, &parent, &genesis, landing);
        let landed = exec.tx_hashes.len();
        assert!(landed >= 2, "{landed} of the fat txs landed");
        let before = chain.lock();
        let (bytes_before, sizes_before) = (before.mempool_bytes, before.sizes.clone());
        drop(before);
        chain.finalize(&block).unwrap();
        let freed: usize = exec.tx_hashes.iter().map(|h| sizes_before[h]).sum();
        assert!(freed > 0);
        {
            let g = chain.lock();
            assert_eq!(g.mempool.len(), admitted - landed);
            assert_eq!(g.mempool_bytes, bytes_before - freed, "the budget comes back with the block");
            assert_eq!(g.sizes.values().sum::<usize>(), g.mempool_bytes);
        }
        assert_eq!(
            chain.add_to_mempool(fat(senders[16], 0, vec![7u8; 130_000])),
            Ok(true),
            "the freed budget admits again"
        );
    }

    /// The 2026-09-29 testnet stall as a unit: one sender with 64 pending txs,
    /// an inclusion list naming nonces 32..47. The listed tx sits at its
    /// sender's nonce 32 while nonces 0..31 are still in the pool, so a proposer
    /// that tried the list first skipped it for good — the voters then saw
    /// "nonce 32 appendable but missing" and refused every proposal at height
    /// 104408. Ordered with the pool (the fix), everything lands and the append
    /// check holds.
    #[test]
    fn listed_txs_at_a_later_nonce_land_after_their_senders_earlier_ones() {
        use aether_crypto::{P256Signer, Signer as _};
        let mut seed = [0u8; 32];
        seed[0] = 0x5b;
        let key = P256Signer::from_seed(&seed).unwrap();
        let sender = address_of(&key.public_key()).unwrap();
        let (chain, genesis) = Chain::new(cfg(vec![(sender, U256::from(10u128.pow(24)))]));
        let pending = transfers(&key, 0..64);
        for t in &pending {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        let seen = Instant::now();
        accept_list(&chain, pending[32..48].to_vec(), seen); // all 16 list slots
        let now = seen + inclusion::FREEZE + Duration::from_millis(250); // past inclusion::FREEZE
        let parent = chain.lock().finalized.clone();

        // The fix: one nonce order over listed and unlisted alike.
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, chain.mempool_candidates());
        assert_eq!(
            landed_nonces(&exec, &pending, sender),
            (0..64).collect::<Vec<u64>>(),
            "a contiguous run, the listed nonces with it"
        );
        assert!(
            chain.inclusion_violations(&exec, &ctx, now).is_empty(),
            "nothing listed is missing and appendable"
        );

        // The pre-fix ordering: the list went before the sender's nonces 0..31,
        // so nonce 32 was tried against state nonce 0, skipped, and never
        // retried (it was not in `rest` either).
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, candidates_2026_09_28(&chain));
        assert_eq!(
            landed_nonces(&exec, &pending, sender),
            (0..32).collect::<Vec<u64>>(),
            "the listed txs were tried too early and never retried"
        );
        assert_eq!(
            chain.inclusion_violations(&exec, &ctx, now),
            vec![aether_execution::tx_hash(&pending[32])],
            "nonce 32 appendable but missing: the stall every validator saw"
        );
    }

    /// The same with several senders: only the named senders' earlier nonces
    /// hold their listed txs back, and every sender's landed nonces must stay
    /// one contiguous run.
    #[test]
    fn listed_txs_of_several_senders_wait_for_their_own_earlier_nonces() {
        use aether_crypto::{P256Signer, Signer as _};
        let key = |b: u8| {
            let mut seed = [0u8; 32];
            seed[0] = 0x5c;
            seed[31] = b;
            P256Signer::from_seed(&seed).unwrap()
        };
        let (a, b, c) = (key(1), key(2), key(3));
        let (addr_a, addr_b, addr_c) = (
            address_of(&a.public_key()).unwrap(),
            address_of(&b.public_key()).unwrap(),
            address_of(&c.public_key()).unwrap(),
        );
        let (chain, genesis) = Chain::new(cfg(vec![
            (addr_a, U256::from(10u128.pow(24))),
            (addr_b, U256::from(10u128.pow(24))),
            (addr_c, U256::from(10u128.pow(24))),
        ]));
        // A's nonces 32..37 and B's first two txs are listed; C is not named.
        let (tx_a, tx_b, tx_c) = (transfers(&a, 0..64), transfers(&b, 0..2), transfers(&c, 0..16));
        let mut listed = tx_a[32..38].to_vec();
        listed.extend_from_slice(&tx_b);
        for t in tx_a.iter().chain(&tx_b).chain(&tx_c) {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        let seen = Instant::now();
        accept_list(&chain, listed, seen);
        let now = seen + inclusion::FREEZE + Duration::from_millis(250); // past inclusion::FREEZE
        let parent = chain.lock().finalized.clone();

        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, chain.mempool_candidates());
        assert_eq!(landed_nonces(&exec, &tx_a, addr_a), (0..64).collect::<Vec<u64>>());
        assert_eq!(landed_nonces(&exec, &tx_b, addr_b), (0..2).collect::<Vec<u64>>());
        assert_eq!(landed_nonces(&exec, &tx_c, addr_c), (0..16).collect::<Vec<u64>>());
        assert!(chain.inclusion_violations(&exec, &ctx, now).is_empty());

        // Before the fix A's listed txs burned their chance before A's nonces
        // 0..31 lifted the state nonce; B's listed txs were at B's next nonce
        // already, so only A's tx went missing.
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, candidates_2026_09_28(&chain));
        assert_eq!(landed_nonces(&exec, &tx_a, addr_a), (0..32).collect::<Vec<u64>>());
        assert_eq!(landed_nonces(&exec, &tx_b, addr_b), (0..2).collect::<Vec<u64>>());
        assert_eq!(landed_nonces(&exec, &tx_c, addr_c), (0..16).collect::<Vec<u64>>());
        assert_eq!(
            chain.inclusion_violations(&exec, &ctx, now),
            vec![aether_execution::tx_hash(&tx_a[32])]
        );
    }

    /// The other hole in a single nonce order over the pool (2026-09-29
    /// red-team 5): 2,000 unrelated nonce-0 txs fill the block's whole tx
    /// budget before a listed tx at a later nonce is reached — truncated out
    /// every time, though nothing about it is unappendable. Its sender's
    /// earlier-nonce chain goes ahead of the strangers now, so it lands.
    #[test]
    fn a_listed_tx_survives_a_flood_of_unrelated_nonce_zero_txs() {
        use aether_crypto::{P256Signer, Signer as _};
        let mut seed = [0u8; 32];
        seed[0] = 0x5d;
        let key = P256Signer::from_seed(&seed).unwrap();
        let sender = address_of(&key.public_key()).unwrap();
        let strangers: Vec<Address> = (1..=MAX_TXS_PER_BLOCK as u64).map(|i| Address::from_word(U256::from(i).into())).collect();
        let mut alloc = vec![(sender, U256::from(10u128.pow(24)))];
        alloc.extend(strangers.iter().cloned().map(|a| (a, U256::from(10u128.pow(24)))));
        let (chain, genesis) = Chain::new(cfg(alloc));

        let pending = transfers(&key, 0..64);
        for t in &pending {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        for (i, a) in strangers.iter().enumerate() {
            assert_eq!(chain.add_to_mempool(tx(*a, 0, 1 + i as u64)), Ok(true), "stranger {i}");
        }
        let seen = Instant::now();
        accept_list(&chain, pending[32..48].to_vec(), seen); // all 16 list slots
        let now = seen + Duration::from_secs(1); // past inclusion::FREEZE
        let parent = chain.lock().finalized.clone();

        // The fix: the listed txs and the nonces they stand on come first, so
        // the strangers' volume cannot push them past the 2,000-tx cut. Gas
        // still stops the block mid-stranger (30M gas, 21k a tx) — after the
        // listed chain has landed.
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, chain.mempool_candidates());
        assert_eq!(
            landed_nonces(&exec, &pending, sender),
            (0..48).collect::<Vec<u64>>(),
            "the sender's nonces through the last listed one, strangers after"
        );
        assert!(
            chain.inclusion_violations(&exec, &ctx, now).is_empty(),
            "nothing listed was pushed past the tx budget"
        );

        // The 2026-09-28 single nonce order this replaces: nonce 0 goes first
        // — 2,001 txs of it — and everything at a later nonce fell past the
        // 2,000-tx cut. Worse, the sender's nonces 0..31 were cut with them,
        // so the inclusion check never saw the listed txs appendable and
        // raised no alarm: the push-out was quiet, block after block.
        let strung_out = candidates_one_nonce_order(&chain);
        assert_eq!(strung_out.len(), MAX_TXS_PER_BLOCK, "the strangers fill the budget");
        assert!(
            strung_out.iter().all(|t| t.header.nonce == 0),
            "and nothing at a later nonce survives the cut"
        );
        let offered: std::collections::HashSet<TxHash> =
            strung_out.iter().map(aether_execution::tx_hash).collect();
        for t in &pending[32..48] {
            assert!(!offered.contains(&aether_execution::tx_hash(t)), "the listed tx was offered");
        }
        let (exec, ctx) = build_ctx(&chain, &parent, &genesis, strung_out);
        assert_eq!(landed_nonces(&exec, &pending, sender), Vec::<u64>::new());
        assert!(
            chain.inclusion_violations(&exec, &ctx, now).is_empty(),
            "nothing appendable missing: the inclusion check stays quiet — that is the hole"
        );
    }

    /// A fee-network `cfg` (7780's shape with fees on — the mainnet rules).
    fn fee_cfg(alloc: Vec<(Address, U256)>) -> ChainConfig {
        ChainConfig { fees: true, ..cfg(alloc) }
    }

    /// A zero-balance sender's zero-tip, zero-cap call (unsigned: admission
    /// checks no signature): pays nothing while the base fee is zero, which is
    /// the audit's free spam.
    fn free_tx(sender: Address, nonce: u64) -> TxEnvelope {
        let call = EvmCall {
            to: Some(Address::repeat_byte(0xaa)),
            value: U256::ZERO,
            input: Bytes::new(),
            gas_limit: 21_000,
            delegate: None,
        };
        let payload = call.encode();
        TxEnvelope {
            header: TxHeader {
                chain_id: 7780,
                sender,
                nonce,
                gas: GasVector { exec: 21_000, state: 0, prove: 21_000 },
                max_fee: FeeVector::default(),
                tip: 0,
                payload_commitment: aether_execution::tx::payload_commitment(&payload),
                scheme: SignerScheme::P256,
                group: None,
            },
            payload: TxPayload::Plain(Bytes::from(payload)),
            signature: Bytes::new(),
        }
    }

    /// `tx` with an explicit exec tip and cap (the prove cap stays 0).
    fn priced(sender: Address, nonce: u64, tip: u128, cap: u128) -> TxEnvelope {
        let mut t = tx(sender, nonce, 1);
        t.header.tip = tip;
        t.header.max_fee.exec = cap;
        t
    }

    /// Fresh addresses (no balance, no nonce), as many as a spammer mints keys.
    fn strangers(n: usize) -> Vec<Address> {
        (1..=n as u64).map(|i| Address::from_word(U256::from(i).into())).collect()
    }

    /// More fresh addresses past the first `skip` words, disjoint from
    /// [`strangers`] of the same size: two spam crowds must never share a key.
    fn strangers_after(skip: usize, n: usize) -> Vec<Address> {
        (skip as u64 + 1..=skip as u64 + n as u64)
            .map(|i| Address::from_word(U256::from(i).into()))
            .collect()
    }

    /// R2-6 (2026-10-03 audit round 2), the PoC as a test: while the base fee
    /// is zero, a zero-balance attacker with fresh keys fills the pool with
    /// zero-tip, zero-cap calls — 50,000 of them from 782 keys — and a funded
    /// sender's higher-fee tx then heard `Err("mempool full")`. The free lane
    /// now stops at its pool-wide quota, and the paying tx walks straight in
    /// over the three-quarters of the pool it can never be crowded out of.
    #[test]
    fn zero_fee_spam_cannot_fill_the_pool_or_exclude_a_paying_sender() {
        let payer = Address::repeat_byte(0x11);
        let (chain, _) = Chain::new(fee_cfg(vec![(payer, U256::from(10u128.pow(20)))]));
        let mut admitted = 0;
        'spam: for sender in strangers(MAX_MEMPOOL / MAX_PER_SENDER + 8) {
            for nonce in 0..MAX_PER_SENDER as u64 {
                match chain.add_to_mempool(free_tx(sender, nonce)) {
                    Ok(true) => admitted += 1,
                    Ok(false) => unreachable!("a fresh tx cannot be known"),
                    Err(e) => {
                        assert!(e.contains("free lane"), "the quota refuses more spam: {e}");
                        assert_eq!(admitted, MAX_FREE_MEMPOOL, "the quota, not another cap, fills first");
                        break 'spam;
                    }
                }
            }
        }
        {
            let g = chain.lock();
            assert_eq!(g.mempool.len(), MAX_FREE_MEMPOOL);
            assert_eq!(g.free_in_pool, MAX_FREE_MEMPOOL, "every spam tx is a free entry");
        }
        // Before the fix this returned Err("mempool full") over a full pool of
        // zero-fee spam; the pool is three-quarters empty now, so it is simply
        // admitted.
        assert!(chain.add_to_mempool(priced(payer, 0, GWEI, GWEI)).unwrap());
        let g = chain.lock();
        assert_eq!(g.mempool.len(), MAX_FREE_MEMPOOL + 1);
        assert_eq!(g.free_in_pool, MAX_FREE_MEMPOOL, "the paying tx is no free entry");
    }

    /// A pool full of fee-paying txs still admits a strictly higher-fee one by
    /// evicting the lowest-fee entry, and never evicts for an equal fee.
    #[test]
    fn a_full_pool_admits_a_strictly_higher_fee_and_refuses_an_equal_one() {
        // 782 senders × 64 txs at tip 1 gwei = 50,048 candidates: enough to
        // fill the count cap mid-way through the last sender. `richer` is
        // funded alongside them, outside the fill, for the eviction below.
        let richer = Address::repeat_byte(0x22);
        let fillers = strangers(MAX_MEMPOOL / MAX_PER_SENDER + 2);
        let mut alloc: Vec<(Address, U256)> =
            fillers.iter().cloned().map(|a| (a, U256::from(10u128.pow(18)))).collect();
        alloc.push((richer, U256::from(10u128.pow(18))));
        let (chain, _) = Chain::new(fee_cfg(alloc));
        let mut admitted = 0;
        let mut full = String::new();
        'fill: for sender in &fillers {
            for nonce in 0..MAX_PER_SENDER as u64 {
                match chain.add_to_mempool(priced(*sender, nonce, GWEI, GWEI)) {
                    Ok(true) => admitted += 1,
                    Err(e) => {
                        full = e;
                        break 'fill;
                    }
                    Ok(false) => unreachable!("a fresh tx cannot be known"),
                }
            }
        }
        assert_eq!(full, "mempool full", "an equal fee never evicts anything");
        assert_eq!(admitted, MAX_MEMPOOL);
        assert_eq!(chain.lock().free_in_pool, 0, "tip-1 txs pay at base fee 0");

        // Tip 2 gwei strictly outranks the tip-1 floor: one entry makes way.
        assert!(chain.add_to_mempool(priced(richer, 0, 2 * GWEI, 2 * GWEI)).unwrap());
        let g = chain.lock();
        assert_eq!(g.mempool.len(), MAX_MEMPOOL, "one lowest-fee entry made way");
        assert_eq!(g.free_in_pool, 0);
    }

    /// Eviction never breaks a nonce chain: only a sender's highest pending
    /// nonce may leave, so the survivors stay a runnable prefix. The oldest
    /// free entries are one sender's nonces 0..3 — without the rule the
    /// eviction would take nonce 0 (oldest, same fee) and strand 1 and 2 past
    /// a gap nothing can close. With it, the end of the chain goes instead.
    #[test]
    fn eviction_never_breaks_a_nonce_chain() {
        let payer = Address::repeat_byte(0x33);
        let chained_sender = Address::repeat_byte(0x44);
        let fillers = strangers(MAX_MEMPOOL - 3);
        let mut alloc: Vec<(Address, U256)> =
            fillers.iter().cloned().map(|a| (a, U256::from(10u128.pow(17)))).collect();
        alloc.push((payer, U256::from(10u128.pow(18))));
        let (chain, _) = Chain::new(fee_cfg(alloc));
        let chained: Vec<TxEnvelope> = (0..3).map(|n| free_tx(chained_sender, n)).collect();
        for t in &chained {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        // One tip-1 tx per funded stranger tops the pool up to exactly the
        // count cap — no failed attempt, so nothing is evicted yet.
        for sender in &fillers {
            assert_eq!(chain.add_to_mempool(priced(*sender, 0, GWEI, GWEI)), Ok(true));
        }
        assert_eq!(chain.lock().mempool.len(), MAX_MEMPOOL);
        // A tip-1 tx outranks only the free chain entries; their runnable
        // prefix survives, the end of the chain makes way.
        assert!(chain.add_to_mempool(priced(payer, 0, GWEI, GWEI)).unwrap());
        let g = chain.lock();
        assert_eq!(g.mempool.len(), MAX_MEMPOOL, "exactly one entry made way");
        for t in &chained[..2] {
            assert!(g.mempool.contains_key(&aether_execution::tx_hash(t)), "the runnable prefix survives");
        }
        assert!(!g.mempool.contains_key(&aether_execution::tx_hash(&chained[2])), "the chain's end, never its head");
    }

    /// Eviction never touches what an inclusion list names (FOCIL): a listed
    /// tx is this node's obligation to propose and its voters' to check for,
    /// whatever happens to its mempool slot. The listed entries here are the
    /// only free ones in a full pool — exactly what a fee comparison would
    /// evict first — and they all survive a higher-fee admission.
    #[test]
    fn eviction_spares_what_inclusion_lists_name() {
        use commonware_cryptography::Signer as _;
        let payer = Address::repeat_byte(0x55);
        let listed_free = strangers(inclusion::MAX_POOL);
        let junk = strangers_after(inclusion::MAX_POOL, MAX_MEMPOOL - inclusion::MAX_POOL);
        let mut alloc: Vec<(Address, U256)> =
            junk.iter().cloned().map(|a| (a, U256::from(10u128.pow(17)))).collect();
        alloc.push((payer, U256::from(10u128.pow(18))));
        let (chain, _) = Chain::new(fee_cfg(alloc));
        let listed: Vec<TxEnvelope> = listed_free.iter().map(|a| free_tx(*a, 0)).collect();
        for t in &listed {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        let seen = Instant::now();
        let key = commonware_cryptography::ed25519::PrivateKey::from_seed(1);
        for (i, chunk) in listed.chunks(inclusion::MAX_IL_TXS).enumerate() {
            let il = inclusion::InclusionList::sign(&key, 1, 2_000 + i as u64, chunk.to_vec());
            assert!(chain.lock().inclusion.accept(&il, seen));
        }
        assert_eq!(chain.lock().inclusion.len(), inclusion::MAX_POOL, "every listed tx is held");
        // Unlisted strangers top the pool up to exactly the count cap.
        for sender in &junk {
            assert_eq!(chain.add_to_mempool(priced(*sender, 0, GWEI, GWEI)), Ok(true));
        }
        assert_eq!(chain.lock().mempool.len(), MAX_MEMPOOL);
        assert!(chain.add_to_mempool(priced(payer, 0, 2 * GWEI, 2 * GWEI)).unwrap());
        let g = chain.lock();
        assert_eq!(g.mempool.len(), MAX_MEMPOOL, "one unlisted entry made way");
        for t in &listed {
            assert!(g.mempool.contains_key(&aether_execution::tx_hash(t)), "a listed tx never leaves for a fee");
        }
    }

    /// When nothing can leave — every lower-fee entry is named by an inclusion
    /// list and the rest fee as much as the newcomer — a full pool still says
    /// no instead of breaking an obligation or evicting an equal fee.
    #[test]
    fn a_full_pool_with_nothing_evictable_still_says_no() {
        use commonware_cryptography::Signer as _;
        let payer = Address::repeat_byte(0x66);
        let listed_free = strangers(inclusion::MAX_POOL);
        let dear = strangers_after(inclusion::MAX_POOL, MAX_MEMPOOL - inclusion::MAX_POOL);
        let mut alloc: Vec<(Address, U256)> =
            dear.iter().cloned().map(|a| (a, U256::from(10u128.pow(18)))).collect();
        alloc.push((payer, U256::from(10u128.pow(18))));
        let (chain, _) = Chain::new(fee_cfg(alloc));
        let listed: Vec<TxEnvelope> = listed_free.iter().map(|a| free_tx(*a, 0)).collect();
        for t in &listed {
            assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
        }
        let seen = Instant::now();
        let key = commonware_cryptography::ed25519::PrivateKey::from_seed(1);
        for (i, chunk) in listed.chunks(inclusion::MAX_IL_TXS).enumerate() {
            let il = inclusion::InclusionList::sign(&key, 1, 3_000 + i as u64, chunk.to_vec());
            assert!(chain.lock().inclusion.accept(&il, seen));
        }
        // Tip-5 txs fill the rest; the pool never gets to evict while filling
        // (the count cap is only reached with the last of them).
        for sender in &dear {
            assert_eq!(chain.add_to_mempool(priced(*sender, 0, 5 * GWEI, 5 * GWEI)), Ok(true));
        }
        assert_eq!(chain.lock().mempool.len(), MAX_MEMPOOL);
        // A tip-2 tx would happily evict the free listed entries (0 < 2 gwei)
        // and cannot touch the tip-5 ones (5 > 2): refused, obligation intact.
        assert_eq!(chain.add_to_mempool(priced(payer, 0, 2 * GWEI, 2 * GWEI)).unwrap_err(), "mempool full");
        let g = chain.lock();
        for t in &listed {
            assert!(g.mempool.contains_key(&aether_execution::tx_hash(t)));
        }
    }

    /// A3-3 (2026-10-04 audit round 3), the PoC as a test: the 64 MiB byte
    /// budget full of funded high-fee calldata in several 64-nonce chains, each
    /// chain ending in a small zero-fee tip, then a fat tx of the fill's own
    /// size that pays more than those tips but less than the fat mass. The
    /// small tips are its only eligible victims and together free far too few
    /// bytes, so admission must fail — and it must fail without deleting
    /// anything. Before the fix, `evict_for` removed each eligible tip as it
    /// walked them, returned false, and the RPC rejected the newcomer anyway:
    /// a denial-of-include the attacker paid nothing for and could repeat on
    /// rearranged tips. The mass fill stops while there is still room (the
    /// newcomer is exactly a fat tx's size, so the leftover cannot absorb it)
    /// because a fill that ran to refusal would itself evict the tips, which
    /// is R2-6 working, not the bug under test.
    #[test]
    fn a_rejected_fee_bump_leaves_the_pool_untouched() {
        let payer = Address::repeat_byte(0x88);
        let chains = strangers(9); // eight complete chains, the ninth supplies fill mass
        let mut alloc: Vec<(Address, U256)> =
            chains.iter().cloned().map(|a| (a, U256::from(10u128.pow(24)))).collect();
        alloc.push((payer, U256::from(10u128.pow(24))));
        let (chain, _) = Chain::new(fee_cfg(alloc));
        let fat_size = fat(chains[0], 0, vec![7u8; 130_000]).to_canonical_bytes().len();

        // Chains 0..=7: nonces 0..=62 of funded 5-gwei calldata, nonce 63 the
        // small zero-fee tip that is the only thing a cheaper tx could evict.
        let mut tips = Vec::new();
        let mut listed_fat = None;
        for (ci, sender) in chains[0..8].iter().enumerate() {
            for nonce in 0..63u64 {
                let mut t = fat(*sender, nonce, vec![7u8; 130_000]);
                t.header.tip = 5 * GWEI;
                t.header.max_fee.exec = 5 * GWEI;
                assert_eq!(chain.add_to_mempool(t.clone()), Ok(true));
                if ci == 1 && nonce == 30 {
                    listed_fat = Some(t);
                }
            }
            let tip = free_tx(*sender, 63);
            assert_eq!(chain.add_to_mempool(tip.clone()), Ok(true));
            tips.push(tip);
        }
        // The ninth chain's fat mass exhausts the budget, stopping while there
        // is room so nothing is evicted on the way in; its top nonce is a fat
        // one, so it contributes no eligible victim of its own.
        let mut mass = 0;
        'fill: for nonce in 0..64u64 {
            if chain.lock().mempool_bytes + fat_size > MAX_MEMPOOL_BYTES {
                break 'fill;
            }
            let mut t = fat(chains[8], nonce, vec![7u8; 130_000]);
            t.header.tip = 5 * GWEI;
            t.header.max_fee.exec = 5 * GWEI;
            assert_eq!(chain.add_to_mempool(t), Ok(true));
            mass += 1;
        }
        assert!(mass > 0, "the ninth chain really did add fill mass");
        // One tip and one fat tx are this node's inclusion obligations: a
        // refusal that deleted them would trade an obligation for nothing.
        let seen = Instant::now();
        accept_list(&chain, vec![tips[0].clone(), listed_fat.expect("nonce 30 of chain 1")], seen);

        let before = {
            let g = chain.lock();
            assert!(g.mempool_bytes + fat_size > MAX_MEMPOOL_BYTES, "the byte budget, not the count, is full");
            (
                g.mempool.keys().cloned().collect::<Vec<TxHash>>(),
                g.mempool_bytes,
                g.sizes.clone(),
                g.arrivals.clone(),
                g.pending_by_sender.clone(),
                g.free_in_pool,
                g.inclusion.len(),
            )
        };
        let mut newcomer = fat(payer, 0, vec![7u8; 130_000]);
        newcomer.header.tip = GWEI;
        newcomer.header.max_fee.exec = GWEI;
        assert_eq!(newcomer.to_canonical_bytes().len(), fat_size, "the newcomer is exactly a fill tx's size");
        // More than the zero-fee tips pay, less than the fat mass: the tips
        // are its only eligible victims, and they free too few bytes to fit.
        assert_eq!(
            chain.add_to_mempool(newcomer.clone()),
            Err("mempool byte budget full".to_string())
        );
        let g = chain.lock();
        assert_eq!(
            g.mempool.keys().cloned().collect::<Vec<TxHash>>(),
            before.0,
            "no entry left for a tx that did not get in"
        );
        assert_eq!(g.mempool_bytes, before.1, "the byte budget is unchanged");
        assert_eq!(g.sizes, before.2);
        assert_eq!(g.arrivals, before.3, "arrival order is unchanged");
        assert_eq!(g.pending_by_sender, before.4, "sender counts are unchanged");
        assert_eq!(g.free_in_pool, before.5, "the free lane is unchanged");
        assert_eq!(g.inclusion.len(), before.6, "inclusion obligations are unchanged");
        for t in &tips {
            assert!(g.mempool.contains_key(&aether_execution::tx_hash(t)), "every small tip survived the refusal");
        }
        assert!(!g.mempool.contains_key(&aether_execution::tx_hash(&newcomer)));
    }

    /// A3-3's other half: the free lane's cap is a byte reservation, not only
    /// a count. The R2-6 count quota left the whole 64 MiB budget reachable by
    /// free txs — a few hundred fat ones, far under the 12,500-entry quota —
    /// so zero-fee calldata could still starve paying senders of bytes. Free
    /// entries now hold at most their share of the byte budget; a free tx past
    /// it is refused, and paying senders keep the rest to themselves.
    #[test]
    fn free_txs_cannot_hold_more_than_their_byte_share_of_the_budget() {
        let payer = Address::repeat_byte(0x77);
        let senders = strangers(540);
        let (chain, _) = Chain::new(fee_cfg(vec![(payer, U256::from(10u128.pow(18)))]));
        let fat_size = fat(senders[0], 0, vec![7u8; 130_000]).to_canonical_bytes().len();
        let mut admitted = 0;
        let mut refused = String::new();
        'fill: for sender in &senders {
            match chain.add_to_mempool(fat(*sender, 0, vec![7u8; 130_000])) {
                Ok(true) => admitted += 1,
                Ok(false) => unreachable!("a fresh tx cannot be known"),
                Err(e) => {
                    refused = e;
                    break 'fill;
                }
            }
        }
        assert!(
            refused.starts_with("free lane byte budget full"),
            "the byte share, not the quota or count, stops the fill: {refused}"
        );
        {
            let g = chain.lock();
            // The pool so far is nothing but free entries, so the bytes they
            // hold are the free lane's, and they sit at its share.
            assert_eq!(g.free_in_pool, admitted, "every fat spam tx is free");
            assert_eq!(g.mempool_bytes, g.sizes.values().sum::<usize>());
            assert!(
                g.mempool_bytes + fat_size > MAX_FREE_MEMPOOL_BYTES,
                "the share, not the senders, stopped the fill"
            );
            assert!(g.mempool_bytes <= MAX_FREE_MEMPOOL_BYTES, "the free lane holds at most its share");
        }
        // The seven eighths the free lane can never touch are the paying
        // senders': a fat paying tx walks in without evicting anything.
        let (len, bytes) = {
            let g = chain.lock();
            (g.mempool.len(), g.mempool_bytes)
        };
        let mut big = fat(payer, 0, vec![7u8; 120_000]);
        big.header.tip = GWEI;
        big.header.max_fee.exec = GWEI;
        assert!(chain.add_to_mempool(big).unwrap());
        let g = chain.lock();
        assert_eq!(g.mempool.len(), len + 1, "nothing made way for a tx with the budget to itself");
        assert!(g.mempool_bytes > bytes);
        assert_eq!(g.free_in_pool, admitted, "the paying tx is no free entry");
        assert!(g.mempool_bytes <= MAX_MEMPOOL_BYTES);
    }

    /// The fix must not overcorrect into never evicting: a paying newcomer
    /// that CAN fit after evicting still evicts and lands. The budget fills
    /// with tip-1 fat txs over a fat zero-fee lane — the fill's tail displaces
    /// the free lane exactly as R2-6 intended — and once even the crumbs are
    /// gone, a tip-2 fat tx displaces exactly one tip-1 entry and walks in.
    #[test]
    fn a_paying_newcomer_that_fits_still_evicts_correctly() {
        let payer = Address::repeat_byte(0x99);
        let lane = strangers(64); // ~64 fat free txs ≈ the 8 MiB share
        let fillers = strangers_after(64, 540);
        let crumbs = strangers_after(64 + 540, 600);
        let mut alloc: Vec<(Address, U256)> =
            fillers.iter().cloned().map(|a| (a, U256::from(10u128.pow(24)))).collect();
        alloc.extend(crumbs.iter().cloned().map(|a| (a, U256::from(10u128.pow(18)))));
        alloc.push((payer, U256::from(10u128.pow(24))));
        let (chain, _) = Chain::new(fee_cfg(alloc));
        let fat_size = fat(lane[0], 0, vec![7u8; 130_000]).to_canonical_bytes().len();
        for sender in &lane {
            assert_eq!(chain.add_to_mempool(fat(*sender, 0, vec![7u8; 130_000])), Ok(true));
        }
        assert_eq!(chain.lock().free_in_pool, lane.len(), "the lane sits in the pool free");
        let mut full = String::new();
        'fill: for sender in &fillers {
            let mut t = fat(*sender, 0, vec![7u8; 130_000]);
            t.header.tip = GWEI;
            t.header.max_fee.exec = GWEI;
            match chain.add_to_mempool(t) {
                Ok(true) => (),
                Ok(false) => unreachable!("a fresh tx cannot be known"),
                Err(e) => {
                    full = e;
                    break 'fill;
                }
            }
        }
        {
            let g = chain.lock();
            assert_eq!(full, "mempool byte budget full");
            assert!(g.mempool_bytes + fat_size > MAX_MEMPOOL_BYTES, "the byte budget, not the count, is full");
            assert_eq!(g.free_in_pool, 0, "the fill's tail displaced the free lane, never a tip-1 peer");
        }
        'crumbs: for sender in &crumbs {
            match chain.add_to_mempool(priced(*sender, 0, GWEI, GWEI)) {
                Ok(true) => (),
                Ok(false) => unreachable!("a fresh tx cannot be known"),
                Err(e) => {
                    assert_eq!(e, "mempool byte budget full");
                    break 'crumbs;
                }
            }
        }
        let (len, bytes) = {
            let g = chain.lock();
            (g.mempool.len(), g.mempool_bytes)
        };
        let mut big = fat(payer, 0, vec![7u8; 126_000]);
        big.header.tip = 2 * GWEI;
        big.header.max_fee.exec = 2 * GWEI;
        assert!(chain.add_to_mempool(big.clone()).unwrap());
        let g = chain.lock();
        assert_eq!(g.mempool.len(), len, "exactly one entry made way");
        assert!(g.mempool.contains_key(&aether_execution::tx_hash(&big)));
        assert!(g.mempool_bytes < bytes, "the evicted entry gave more bytes back than the newcomer brought");
        assert!(g.mempool_bytes <= MAX_MEMPOOL_BYTES);
    }

    /// A tx whose fee caps sit below the base fee cannot be included while the
    /// fee is up there, and stops holding pool capacity after
    /// `MEMPOOL_FEE_WAIT` — well before the TTL — so paying capacity comes
    /// back promptly (R2-6). The sender always knows: admission refuses
    /// below-cap txs, so re-submitting at higher caps is the wallet's move.
    #[test]
    fn a_tx_below_the_base_fee_leaves_after_the_fee_wait_not_the_ttl() {
        let a = Address::repeat_byte(5);
        let t = tx(a, 0, 1);
        let state = funded(a, 10u128.pow(18));
        let t0 = Instant::now();
        let high = FeeVector { exec: 5 * GWEI, state: 0, prove: 0 };
        assert!(
            keep_in_pool(&t, Some(t0), t0 + MEMPOOL_FEE_WAIT / 2, &state, high, true),
            "a short spike may still pass before the wait"
        );
        assert!(
            !keep_in_pool(&t, Some(t0), t0 + MEMPOOL_FEE_WAIT, &state, high, true),
            "past the wait the entry stops holding paying capacity"
        );
        assert!(
            keep_in_pool(&t, Some(t0), t0 + MEMPOOL_FEE_WAIT, &state, FeeVector::default(), true),
            "at a base fee it can pay, only the TTL rules"
        );
    }

    /// 7780's shape (no fees) is one free lane as before: the quota and
    /// fee-based eviction do not exist there, the count cap behaves exactly as
    /// it always did.
    #[test]
    fn without_fees_the_pool_is_one_lane_as_before() {
        let (chain, _) = Chain::new(cfg(vec![]));
        let mut admitted = 0;
        let mut full = String::new();
        'fill: for sender in strangers(MAX_MEMPOOL / MAX_PER_SENDER + 2) {
            for nonce in 0..MAX_PER_SENDER as u64 {
                match chain.add_to_mempool(free_tx(sender, nonce)) {
                    Ok(true) => admitted += 1,
                    Ok(false) => unreachable!("a fresh tx cannot be known"),
                    Err(e) => {
                        full = e;
                        break 'fill;
                    }
                }
            }
        }
        assert_eq!(full, "mempool full", "the count cap, no quota before it");
        assert_eq!(admitted, MAX_MEMPOOL, "every zero-fee tx was admissible");
        assert_eq!(chain.lock().free_in_pool, 0, "the free counter is a fee-network concern");
    }
}
