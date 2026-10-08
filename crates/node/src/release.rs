//! Chain-driven release discovery. Published events supply bytes; approval
//! comes from pinned code, finalized storage commitments and builder signatures.
//! A post-state is announced only after the next finalized block certifies it.

use crate::roster::ReleasePin;
use aether_execution::{Event, Receipt, WorldState};
use aether_types::{Address, B256, U256};
use base64::Engine as _;
use p256::ecdsa::{signature::Verifier as _, Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const WAIT_SECONDS: u64 = 72 * 60 * 60;
/// Every new-genesis release, including emergencies, keeps both barriers.
pub const WAIT_BLOCKS: u64 = WAIT_SECONDS;
pub const SLOT_BLOCKS: u64 = 600;
const MAX_MANIFEST_BYTES: usize = 16_384;
const MAX_SIGNATURE_BYTES: usize = 4_096;
/// Escaped JSON strings can be six times their raw length; only two payloads
/// are retained, regardless of how many permissionless entries are published.
const MAX_CACHE_BYTES: usize = 256 * 1024;
pub(crate) const CACHE_KEY: &str = "release-payloads-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Payload {
    index: u64,
    manifest: String,
    signatures: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Manifest {
    chain_id: u64,
    log_address: String,
    platform: String,
    version: String,
    build: String,
    emergency: bool,
    sparkle_ed_signature: String,
    artifacts: Vec<Value>,
    #[serde(default)]
    install_after_height: Option<u64>,
    #[serde(default)]
    restart_slot_height: Option<u64>,
}

#[derive(Deserialize)]
struct BuilderSignature {
    public_key: String,
    signature: String,
}

#[derive(Clone)]
struct Approved {
    payload: Payload,
    manifest: Manifest,
    build_number: u64,
    manifest_hash: String,
    archive_sha256: String,
    signatures_hash: String,
    published_block: u64,
    published_at: u64,
    approvals: usize,
    required_approvals: usize,
    install_after_height: u64,
    install_after_timestamp_ms: u64,
}

/// Only raw payloads are durable. Trust, approval count and timing are always
/// recomputed from the pin and the restored chain state.
#[derive(Default, Serialize, Deserialize)]
struct Cache {
    current: Option<Payload>,
    pending: Option<Payload>,
}

#[derive(Clone, Default)]
pub(crate) struct Watcher {
    pub(crate) pin: Option<ReleasePin>,
    current: Option<Approved>,
    pending: Option<Approved>,
    state_height: u64,
    certified_timestamp_ms: u64,
    dirty: bool,
}

impl Watcher {
    pub(crate) fn pinned(network: Option<&Value>, chain_id: u64) -> Option<ReleasePin> {
        let network = network?;
        if network.get("chain_id")?.as_u64()? != chain_id {
            return None;
        }
        let pin: ReleasePin = serde_json::from_value(network.get("release")?.clone()).ok()?;
        pin.validate().ok()?;
        (pin.log.parse::<Address>().ok()? != Address::ZERO).then_some(pin)
    }

    pub(crate) fn new(pin: Option<ReleasePin>) -> Self {
        Self {
            pin,
            ..Self::default()
        }
    }

    pub(crate) fn restore(&mut self, bytes: &[u8], state: &WorldState, chain_id: u64, height: u64) {
        if bytes.len() > MAX_CACHE_BYTES {
            return;
        }
        let Ok(cache) = serde_json::from_slice::<Cache>(bytes) else {
            return;
        };
        for payload in cache.current.into_iter().chain(cache.pending) {
            self.stage(payload, state, chain_id, height);
        }
    }

    pub(crate) fn discover(
        &mut self,
        receipts: &[Receipt],
        state: &WorldState,
        chain_id: u64,
        height: u64,
    ) {
        let Some(address) = self
            .pin
            .as_ref()
            .and_then(|p| p.log.parse::<Address>().ok())
        else {
            return;
        };
        for receipt in receipts.iter().filter(|r| r.success) {
            for event in &receipt.events {
                if event.address == address {
                    if let Some(payload) = decode_published(event) {
                        self.stage(payload, state, chain_id, height);
                    }
                }
            }
        }
    }

    /// An upstream status may carry bytes missed by checkpoint sync. It is
    /// merely a hint: the same local code, storage and signature checks apply.
    pub(crate) fn discover_status(
        &mut self,
        release: &Value,
        state: &WorldState,
        chain_id: u64,
        height: u64,
    ) {
        let (Some(index), Some(manifest), Some(signatures)) = (
            release["index"].as_u64(),
            release["manifest"].as_str(),
            release["signatures"].as_str(),
        ) else {
            return;
        };
        if manifest.len() > MAX_MANIFEST_BYTES || signatures.len() > MAX_SIGNATURE_BYTES {
            return;
        }
        if self
            .pending
            .as_ref()
            .or(self.current.as_ref())
            .is_some_and(|r| {
                r.payload.index == index
                    && r.payload.manifest == manifest
                    && r.payload.signatures == signatures
            })
        {
            return;
        }
        self.stage(
            Payload {
                index,
                manifest: manifest.to_owned(),
                signatures: signatures.to_owned(),
            },
            state,
            chain_id,
            height,
        );
    }

    fn stage(&mut self, payload: Payload, state: &WorldState, chain_id: u64, height: u64) {
        let Some(pin) = &self.pin else { return };
        let Some(approved) = verify(pin, chain_id, state, height, payload) else {
            return;
        };
        let known = self.pending.as_ref().or(self.current.as_ref());
        if known.is_some_and(|old| old.build_number >= approved.build_number) {
            return;
        }
        self.pending = Some(approved);
        self.dirty = true;
    }

    /// `state` is the parent post-state whose root the new finalized header
    /// commits. Current-height receipt bytes remain staged until the next call.
    pub(crate) fn certify(
        &mut self,
        state: &WorldState,
        chain_id: u64,
        state_height: u64,
        timestamp_ms: u64,
    ) {
        let Some(pin) = &self.pin else { return };
        if let Some(pending) = self.pending.take() {
            if let Some(approved) =
                verify(pin, chain_id, state, state_height, pending.payload.clone())
            {
                if self
                    .current
                    .as_ref()
                    .is_none_or(|old| approved.build_number > old.build_number)
                {
                    self.current = Some(approved);
                    self.dirty = true;
                }
            } else {
                // A restored candidate may name a state later than a rolled
                // back checkpoint. Never announce it without its commitment.
                self.dirty = true;
            }
        }
        self.current = self
            .current
            .take()
            .and_then(|old| verify(pin, chain_id, state, state_height, old.payload));
        self.state_height = state_height;
        self.certified_timestamp_ms = timestamp_ms;
    }

    /// A snapshot jump or durable rollback must certify its own state before
    /// it can announce cached discovery from the previous head.
    pub(crate) fn rebase(&mut self, state: &WorldState, chain_id: u64, height: u64) {
        let payloads = self
            .current
            .take()
            .map(|r| r.payload)
            .into_iter()
            .chain(self.pending.take().map(|r| r.payload))
            .collect::<Vec<_>>();
        for payload in payloads {
            self.stage(payload, state, chain_id, height);
        }
        self.state_height = 0;
        self.certified_timestamp_ms = 0;
    }

    pub(crate) fn cache_if_dirty(&mut self) -> Option<Vec<u8>> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        serde_json::to_vec(&Cache {
            current: self.current.as_ref().map(|r| r.payload.clone()),
            pending: self.pending.as_ref().map(|r| r.payload.clone()),
        })
        .ok()
    }

    pub(crate) fn status(&self, chain_id: u64, height: u64, restart_slot: Value) -> Value {
        let (Some(pin), Some(r)) = (&self.pin, &self.current) else {
            return Value::Null;
        };
        // Clock rollback cannot make an otherwise valid discovery installable.
        if self.certified_timestamp_ms / 1_000 < r.published_at {
            return Value::Null;
        }
        json!({
            "index": r.payload.index, "chain_id": chain_id, "log_address": pin.log,
            "version": r.manifest.version, "build": r.manifest.build,
            "manifest_hash": r.manifest_hash, "archive_sha256": r.archive_sha256,
            "signatures_hash": r.signatures_hash,
            "manifest": r.payload.manifest, "signatures": r.payload.signatures,
            "approved_at_height": r.published_block, "published_at": r.published_at,
            "install_after_height": r.install_after_height,
            "install_after_timestamp_ms": r.install_after_timestamp_ms,
            "state_height": self.state_height, "certified_timestamp_ms": self.certified_timestamp_ms,
            "emergency": r.manifest.emergency, "approvals": r.approvals,
            "required_approvals": r.required_approvals, "artifacts": r.manifest.artifacts,
            "ready": self.state_height >= r.install_after_height && height > r.install_after_height
                && self.certified_timestamp_ms >= r.install_after_timestamp_ms,
            "restart_slot": restart_slot,
            "restart_slot_height": r.manifest.restart_slot_height,
        })
    }
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn digest_hex(s: &str) -> bool {
    s.len() == 64 && hex::decode(s).is_ok()
}

fn verify(
    pin: &ReleasePin,
    chain_id: u64,
    state: &WorldState,
    height: u64,
    payload: Payload,
) -> Option<Approved> {
    let address: Address = pin.log.parse().ok()?;
    let code_hash: B256 = pin.code_hash.parse().ok()?;
    if state.code_hash(&address) != code_hash
        || payload.manifest.is_empty()
        || payload.manifest.len() > MAX_MANIFEST_BYTES
        || payload.signatures.is_empty()
        || payload.signatures.len() > MAX_SIGNATURE_BYTES
    {
        return None;
    }
    let count = state.storage(&address, U256::ZERO);
    if U256::from(payload.index) >= count {
        return None;
    }
    let offset = payload.index.checked_mul(4)?;
    let base = U256::from_be_bytes(alloy_primitives::keccak256([0u8; 32]).0) + U256::from(offset);
    let manifest_hash = hash(payload.manifest.as_bytes());
    let signatures_hash = hash(payload.signatures.as_bytes());
    if format!("{:064x}", state.storage(&address, base)) != manifest_hash
        || format!("{:064x}", state.storage(&address, base + U256::from(2))) != signatures_hash
    {
        return None;
    }
    let archive_sha256 = format!("{:064x}", state.storage(&address, base + U256::from(1)));
    let meta = state.storage(&address, base + U256::from(3));
    let published_block = (meta & U256::from(u64::MAX)).to::<u64>();
    let published_at = ((meta >> 64usize) & U256::from(u64::MAX)).to::<u64>();
    let emergency = ((meta >> 128usize) & U256::from(0xff)).to::<u8>();
    if published_block == 0
        || published_block > height
        || published_at == 0
        || emergency > 1
        || meta >> 136usize != U256::ZERO
    {
        return None;
    }
    let manifest: Manifest = serde_json::from_str(&payload.manifest).ok()?;
    if manifest.chain_id != chain_id
        || !manifest.log_address.eq_ignore_ascii_case(&pin.log)
        || manifest.platform != "macos-arm64-dmg"
        || manifest.emergency != (emergency == 1)
        || manifest.version.is_empty()
        || manifest.version.len() > 128
        || manifest.build.is_empty()
        || manifest.build.len() > 64
        || manifest.sparkle_ed_signature.is_empty()
        || manifest.sparkle_ed_signature.len() > 256
        || manifest.artifacts.is_empty()
        || manifest.artifacts.len() > 16
    {
        return None;
    }
    if base64::engine::general_purpose::STANDARD
        .decode(&manifest.sparkle_ed_signature)
        .ok()?
        .len()
        != 64
    {
        return None;
    }
    let build_number = manifest.build.parse::<u64>().ok()?;
    let mut names = BTreeSet::new();
    let mut archive_matches = false;
    for item in &manifest.artifacts {
        let name = item["name"].as_str()?;
        let sha256 = item["sha256"].as_str()?;
        if name.is_empty()
            || name.len() > 512
            || !names.insert(name)
            || !digest_hex(sha256)
            || item
                .get("size")
                .is_some_and(|s| s.as_u64().is_none_or(|n| n == 0))
        {
            return None;
        }
        if name == "EastSea.dmg" {
            archive_matches = sha256.eq_ignore_ascii_case(&archive_sha256);
        }
    }
    if !archive_matches {
        return None;
    }
    let signatures: Vec<BuilderSignature> = serde_json::from_str(&payload.signatures).ok()?;
    if signatures.is_empty() || signatures.len() > 3 {
        return None;
    }
    let mut valid = BTreeSet::new();
    for signed in signatures {
        let key = signed.public_key.to_ascii_lowercase();
        if !pin
            .builder_keys
            .iter()
            .any(|k| k.eq_ignore_ascii_case(&key))
            || !valid.insert(key.clone())
        {
            return None;
        }
        let key_bytes = hex::decode(&key).ok()?;
        let signature_bytes = hex::decode(&signed.signature).ok()?;
        if key_bytes.len() != 65 || signature_bytes.len() != 64 {
            return None;
        }
        // Builder-sign/CryptoKit accepts both ECDSA s representatives. The
        // transaction-only low-s policy must not discard an approved release.
        VerifyingKey::from_sec1_bytes(&key_bytes)
            .ok()?
            .verify(
                payload.manifest.as_bytes(),
                &Signature::from_slice(&signature_bytes).ok()?,
            )
            .ok()?;
    }
    let required_approvals = if emergency == 1 {
        pin.emergency_threshold
    } else {
        pin.threshold
    } as usize;
    if valid.len() < required_approvals {
        return None;
    }
    let install_after_height = published_block
        .checked_add(WAIT_BLOCKS)?
        .max(manifest.install_after_height.unwrap_or(0))
        .max(manifest.restart_slot_height.unwrap_or(0));
    let install_after_timestamp_ms = published_at.checked_add(WAIT_SECONDS)?.checked_mul(1_000)?;
    Some(Approved {
        payload,
        manifest,
        build_number,
        manifest_hash,
        archive_sha256,
        signatures_hash,
        published_block,
        published_at,
        approvals: valid.len(),
        required_approvals,
        install_after_height,
        install_after_timestamp_ms,
    })
}

/// Decode the canonical Solidity event ABI with limits before allocation.
fn decode_published(event: &Event) -> Option<Payload> {
    if event.topics.len() != 3
        || event.topics[0] != alloy_primitives::keccak256(b"Published(uint256,bytes32,bytes,bytes)")
    {
        return None;
    }
    let index_word = U256::from_be_bytes(event.topics[1].0);
    if index_word > U256::from(u64::MAX) {
        return None;
    }
    let data = event.data.as_ref();
    if data.len() > MAX_MANIFEST_BYTES + MAX_SIGNATURE_BYTES + 192 || data.len() < 128 {
        return None;
    }
    let read_word = |at: usize| -> Option<usize> {
        let w = U256::from_be_slice(data.get(at..at.checked_add(32)?)?);
        (w <= U256::from(data.len())).then(|| w.to::<usize>())
    };
    let manifest_at = read_word(0)?;
    let signatures_at = read_word(32)?;
    if manifest_at != 64 || !signatures_at.is_multiple_of(32) {
        return None;
    }
    let manifest_len = read_word(manifest_at)?;
    if manifest_len == 0 || manifest_len > MAX_MANIFEST_BYTES {
        return None;
    }
    let manifest_start = manifest_at.checked_add(32)?;
    let manifest_end = manifest_start.checked_add(manifest_len)?;
    let padded_manifest_end = manifest_end.next_multiple_of(32);
    if signatures_at != padded_manifest_end
        || data
            .get(manifest_end..padded_manifest_end)?
            .iter()
            .any(|b| *b != 0)
    {
        return None;
    }
    let signatures_len = read_word(signatures_at)?;
    if signatures_len == 0 || signatures_len > MAX_SIGNATURE_BYTES {
        return None;
    }
    let signatures_start = signatures_at.checked_add(32)?;
    let signatures_end = signatures_start.checked_add(signatures_len)?;
    if signatures_end.next_multiple_of(32) != data.len()
        || data.get(signatures_end..)?.iter().any(|b| *b != 0)
    {
        return None;
    }
    let manifest = std::str::from_utf8(data.get(manifest_start..manifest_end)?).ok()?;
    if B256::from_slice(Sha256::digest(manifest.as_bytes()).as_ref()) != event.topics[2] {
        return None;
    }
    let signatures = std::str::from_utf8(data.get(signatures_start..signatures_end)?).ok()?;
    Some(Payload {
        index: index_word.to::<u64>(),
        manifest: manifest.to_owned(),
        signatures: signatures.to_owned(),
    })
}

/// Read-only chain-assigned restart decision. A caller names its validator key;
/// the active committee in state assigns the seat, never the caller or RPC.
pub(crate) fn restart_slot(g: &crate::chain::Inner, key: Option<&str>, now_ms: u64) -> Value {
    let state_committee = aether_rewards::committee(&g.finalized.state);
    let committee = if state_committee.is_empty() {
        &g.committee.members
    } else {
        &state_committee
    };
    let n = committee.len();
    let seat = key.and_then(|key| {
        committee.iter().position(|(k, _)| {
            k.trim_start_matches("0x")
                .eq_ignore_ascii_case(key.trim_start_matches("0x"))
        })
    });
    let height = g.finalized.height;
    let current_slot = (n != 0).then(|| (height / SLOT_BLOCKS) as usize % n);
    let in_slot = seat.is_some() && seat == current_slot;
    let next_slot_height = seat.map(|i| {
        let current = current_slot.unwrap_or(0);
        let advance = (i + n - current) % n;
        (height / SLOT_BLOCKS)
            .saturating_add(advance as u64)
            .saturating_mul(SLOT_BLOCKS)
    });
    let f = n.saturating_sub(1) / 3;
    let quorum = n.saturating_sub(f);
    let window = (4 * n) as u64;
    let proposers = g
        .blocks
        .range(height.saturating_sub(window.saturating_sub(1))..=height)
        .filter(|(h, _)| **h != 0)
        .map(|(_, b)| b.proposer)
        .collect::<BTreeSet<_>>();
    let required_proposers = quorum.saturating_add(1);
    let active = committee
        .iter()
        .filter_map(|(key, _)| {
            let bytes = hex::decode(key.trim_start_matches("0x")).ok()?;
            if bytes.len() != 32 {
                return None;
            }
            aether_crypto::address_of(&aether_crypto::PublicKey {
                scheme: aether_types::SignerScheme::Ed25519,
                bytes,
            })
            .ok()
        })
        .collect::<BTreeSet<_>>();
    let distinct_proposers = proposers.iter().filter(|p| active.contains(p)).count();
    let slot_end_height = (height / SLOT_BLOCKS)
        .saturating_add(1)
        .saturating_mul(SLOT_BLOCKS);
    let jitter_blocks = key
        .map(|key| {
            let mut seed = key.as_bytes().to_vec();
            if let Some(release) = &g.release_watcher.current {
                seed.extend_from_slice(release.manifest_hash.as_bytes());
            }
            let digest = Sha256::digest(&seed);
            u64::from_be_bytes(digest[..8].try_into().expect("eight digest bytes")) % 181
        })
        .unwrap_or(0);
    // Reserve the 15s lease, up to 10s block age, 1s future skew and one
    // rounding second: 27 blocks at the new-genesis timestamp floor. Legacy
    // consensus has no such floor, so a height cannot bound its lease safely.
    let lease_margin_blocks = 27_000u64.div_ceil(crate::application::MIN_BLOCK_INTERVAL_MS);
    let bounded_block_time = g.cfg.node_rewards || g.cfg.history_v2;
    let inside_window = bounded_block_time
        && height % SLOT_BLOCKS >= jitter_blocks
        && height % SLOT_BLOCKS < SLOT_BLOCKS.saturating_sub(lease_margin_blocks);
    let healthy = g.net_height.is_none_or(|h| h <= height)
        && now_ms >= g.finalized.timestamp
        && now_ms - g.finalized.timestamp < 10_000;
    json!({ "height": height, "slot_blocks": SLOT_BLOCKS, "cycle_slots": n,
        "cycle_blocks": (n as u64).saturating_mul(SLOT_BLOCKS), "seat_index": seat,
        "committee_size": n, "faults": f, "quorum": quorum,
        "in_slot": in_slot, "next_slot_height": next_slot_height,
        "slot_end_height": slot_end_height, "jitter_blocks": jitter_blocks,
        "distinct_proposers": distinct_proposers, "required_proposers": required_proposers,
        "healthy": healthy, "allowed": in_slot && inside_window && healthy && distinct_proposers >= required_proposers })
}
