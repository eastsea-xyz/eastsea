//! Consensus block: Commonware's certifiable block envelope around an Aether payload.
//! Structure adapted from alto-types (MIT OR Apache-2.0, commonwarexyz/alto).

use aether_types::{Address, BlockAccessList, GasVector, TxEnvelope, B256};
use bytes::{Buf, BufMut, Bytes};
use commonware_codec::{varint::UInt, BufsMut, Encode, EncodeSize, Error, RangeCfg, Read, ReadExt, Write};
use commonware_consensus::{
    simplex::types::Context as CContext,
    types::{Epoch, Height, Round, View},
    CertifiableBlock, Heightable,
};
use commonware_cryptography::{ed25519, sha256::Digest, Digest as _, Digestible, Hasher, Sha256, Signer};
use serde::{Deserialize, Serialize};

pub type PublicKey = ed25519::PublicKey;
pub type Context = CContext<Digest, PublicKey>;
pub const EPOCH: Epoch = Epoch::zero();

/// Aether block contents. Order-first (design D3): `parent_state_root` is the
/// state after executing the parent; this block's own result appears in its child.
/// Light clients read payloads leniently (a later protocol may add fields they
/// do not need); nodes accept only the canonical encoding of the fields they
/// know, so a node behind a newer protocol stops instead of misreading it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    /// Protocol version whose rules this block follows: 1 until a
    /// committee-signed upgrade on chain activates a later one.
    #[serde(default)]
    pub version: u32,
    pub parent_state_root: B256,
    /// Merkle Mountain Range root of every earlier block's hash: a certificate
    /// on this block proves all history before it (aether_state::mmr).
    #[serde(default)]
    pub history_root: B256,
    /// Hash of the parent's chain metadata outside the state tree (fee-market
    /// excess, pending committee handoff, latest draw seed): a checkpoint
    /// snapshot carrying them is authenticated by this block's certificate.
    #[serde(default)]
    pub parent_meta: B256,
    pub txs: Vec<TxEnvelope>,
    pub bal: BlockAccessList,
    pub gas: GasVector,
    /// The running committee hands the key over to a new voting set
    /// (docs/design/07-consensus.md); the switch happens a fixed distance later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<Handoff>,
    /// The running committee's threshold signature on a draw number: the seed
    /// the next voting set is drawn with (unique, so it cannot be ground).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<Seed>,
    /// A committee-signed protocol upgrade, put on chain so that every node and
    /// every checkpoint learns its activation height the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade: Option<SignedUpgrade>,
    /// Proofs of earlier blocks (protocol 2): the first valid one of a block
    /// is paid its escrow share and issuance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proofs: Vec<ProofClaim>,
    /// Beacon answers of registered Macs (node rewards networks only,
    /// docs/design/15-node-rewards.md): no transaction and no fee, so a Mac
    /// with a zero balance answers. Validators check every signature.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beacons: Vec<BeaconAnswer>,
    /// Voting-node registrations (node rewards networks only,
    /// docs/design/22-gas-pool.md 2층): no transaction and no fee, so a new
    /// Mac with a zero balance registers even while the chain is congested.
    /// Validators check the registrar's attestation and the operator wallet's
    /// relay signature; a block with an invalid item is invalid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registrations: Vec<NodeRegistration>,
}

/// A registration riding a block's free lane instead of a paid contract call:
/// the same content the contract's `register` takes, plus the operator wallet's
/// domain-separated signature over it (with a one-shot nonce and an expiry)
/// and the compressed key that address derives `operator` from. Fixed-size, so
/// a count of items bounds the lane's bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRegistration {
    /// The wallet registering itself as operator (pays nothing here).
    pub operator: Address,
    /// The Mac's voting key (ed25519, as the registry stores it).
    pub validator_key: B256,
    /// The Mac's iroh node id.
    pub node_id: B256,
    /// Where liveness beacons for this Mac are sent (usually the operator).
    pub beaconer: Address,
    /// The registrar's P-256 signature (r‖s) over
    /// `registry::attestation_message` — given only for a fresh DeviceCheck
    /// token of one Mac, exactly what the contract path checks.
    pub attestation: Bytes,
    /// The operator wallet's P-256 signature (r‖s) over
    /// `registry::relay_message`: nobody relays another wallet's attestation.
    pub signature: Bytes,
    /// The wallet's compressed P-256 key (33 bytes); `address_of` gives `operator`.
    pub operator_key: Bytes,
    /// Must equal the chain's spent-item count for `operator` (one shot).
    pub nonce: u64,
    /// The item is valid in blocks at heights up to this one.
    pub expiry: u64,
}

/// A registered Mac's answer to a beacon slot: its voting key's signature over
/// (chain, epoch, slot, the slot block's hash). The epoch is the block's own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeaconAnswer {
    /// Registry candidate index of the Mac.
    pub index: u64,
    pub slot: u64,
    /// Ed25519 signature (hex).
    pub signature: String,
    /// The day's re-attestation, when due: the registrar's P-256 signature
    /// after a fresh DeviceCheck token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attest: Option<Reattestation>,
}

/// The registrar's signature (r, s hex) over `aether_rewards::beacons::reattest_message`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reattestation {
    pub period: u64,
    pub r: String,
    pub s: String,
}

/// A proof that block `height` executed as its recorded statement says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofClaim {
    pub height: u64,
    /// Paid on inclusion.
    pub prover: Address,
    /// The serialized proof (hex).
    pub proof: String,
}

/// A release implementing a protocol version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// e.g. "macos-arm64-dmg", "linux-x86_64".
    pub platform: String,
    pub version: String,
    /// BLAKE3 of the artifact, hex.
    pub blake3: String,
    pub url: String,
}

/// A protocol upgrade: the version, the first height its rules apply to, and
/// the releases that implement it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upgrade {
    pub chain_id: u64,
    pub protocol: u32,
    /// First height the new rules apply to.
    pub activate_at: u64,
    pub releases: Vec<Release>,
    #[serde(default)]
    pub notes: String,
    /// A new registrar P-256 key (x, y), in effect from `activate_at`; all
    /// zeros stops registrations. Only the committee can change it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registrar: Option<(B256, B256)>,
}

/// An upgrade with the committee's threshold signature (nodes verify it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedUpgrade {
    pub upgrade: Upgrade,
    /// Codec bytes (hex) of the BLS12-381 (MinSig) signature.
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seed {
    pub draw: u64,
    /// BLS12-381 threshold signature (hex).
    pub signature: String,
}

/// A new voting set and its key sharing (same committee identity), signed by
/// the running committee's threshold key. Plain data here; nodes verify it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    /// Key round of the new sharing.
    pub round: u64,
    /// Public DKG output (hex codec bytes): the new sharing of the same identity.
    pub output: String,
    /// The new voting set, in roster order: (ed25519 key hex, iroh node id).
    pub members: Vec<(String, String)>,
    /// BLS12-381 threshold signature (hex) of the running committee.
    pub signature: String,
}

impl Payload {
    pub fn to_bytes(&self) -> Bytes {
        Bytes::from(serde_json::to_vec(self).expect("payload serializes"))
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        serde_json::from_slice(b).ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub context: Context,
    pub parent: Digest,
    pub height: Height,
    /// Milliseconds since the Unix epoch.
    pub timestamp: u64,
    /// Encoded `Payload`.
    pub data: Bytes,
    digest: Digest,
}

impl Block {
    pub fn genesis(chain_id: u64, genesis_root: B256) -> Self {
        Self::genesis_with(chain_id, genesis_root, false)
    }

    /// Genesis of a chain; `history_v2` networks (a new genesis only) get a
    /// genesis hash of their own, so no node can run them under other rules.
    pub fn genesis_with(chain_id: u64, genesis_root: B256, history_v2: bool) -> Self {
        let context =
            Context { round: Round::new(EPOCH, View::zero()), leader: ed25519::PrivateKey::from_seed(0).public_key(), parent: (View::zero(), Digest::EMPTY) };
        let payload = Payload { version: 1, parent_state_root: genesis_root, ..Default::default() };
        let tag = if history_v2 {
            Sha256::hash(&[b"aether-genesis".as_slice(), &chain_id.to_be_bytes(), b"history-v2"])
        } else {
            Sha256::hash(&[b"aether-genesis".as_slice(), &chain_id.to_be_bytes()])
        };
        Self::new(context, tag, Height::zero(), 0, payload.to_bytes())
    }

    fn compute_digest(context: &Context, parent: &Digest, height: Height, timestamp: u64, data: &[u8]) -> Digest {
        let mut h = Sha256::default();
        h.update(&context.encode()).update(parent).update(&height.get().to_be_bytes()).update(&timestamp.to_be_bytes()).update(data);
        h.finalize().1
    }

    pub fn new(context: Context, parent: Digest, height: Height, timestamp: u64, data: Bytes) -> Self {
        let digest = Self::compute_digest(&context, &parent, height, timestamp, &data);
        Self { context, parent, height, timestamp, data, digest }
    }

    pub fn payload(&self) -> Option<Payload> {
        Payload::from_bytes(&self.data)
    }

    pub fn codec_config(max_size: u32) -> RangeCfg<usize> {
        RangeCfg::from(..=max_size as usize)
    }
}

impl Write for Block {
    fn write(&self, w: &mut impl BufMut) {
        self.context.write(w);
        self.parent.write(w);
        self.height.write(w);
        UInt(self.timestamp).write(w);
        self.data.write(w);
    }

    fn write_bufs(&self, w: &mut impl BufsMut) {
        self.context.write_bufs(w);
        self.parent.write_bufs(w);
        self.height.write_bufs(w);
        UInt(self.timestamp).write_bufs(w);
        self.data.write_bufs(w);
    }
}

impl Read for Block {
    type Cfg = RangeCfg<usize>;

    fn read_cfg(r: &mut impl Buf, cfg: &Self::Cfg) -> Result<Self, Error> {
        let context = Context::read(r)?;
        let parent = Digest::read(r)?;
        let height = Height::read(r)?;
        let timestamp = UInt::read(r)?.0;
        let data = Bytes::read_cfg(r, cfg)?;
        Ok(Self::new(context, parent, height, timestamp, data))
    }
}

impl EncodeSize for Block {
    fn encode_size(&self) -> usize {
        self.context.encode_size() + self.parent.encode_size() + self.height.encode_size() + UInt(self.timestamp).encode_size() + self.data.encode_size()
    }

    fn encode_inline_size(&self) -> usize {
        self.context.encode_inline_size()
            + self.parent.encode_inline_size()
            + self.height.encode_inline_size()
            + UInt(self.timestamp).encode_inline_size()
            + self.data.encode_inline_size()
    }
}

impl Digestible for Block {
    type Digest = Digest;
    fn digest(&self) -> Digest {
        self.digest
    }
}

impl commonware_consensus::Block for Block {
    fn parent(&self) -> Digest {
        self.parent
    }
}

impl Heightable for Block {
    fn height(&self) -> Height {
        self.height
    }
}

impl CertifiableBlock for Block {
    type Context = Context;
    fn context(&self) -> Context {
        self.context.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_codec::Decode;

    #[test]
    fn codec_round_trip_preserves_digest_and_payload() {
        let g = Block::genesis(7, B256::repeat_byte(3));
        let bytes = g.encode();
        let back = Block::decode_cfg(bytes, &Block::codec_config(1 << 20)).unwrap();
        assert_eq!(back, g);
        assert_eq!(back.digest(), g.digest());
        assert_eq!(back.payload().unwrap().parent_state_root, B256::repeat_byte(3));
        assert_ne!(Block::genesis(8, B256::repeat_byte(3)).digest(), g.digest());
        // History v2 is bound to the genesis hash; the original genesis is unchanged.
        assert_eq!(Block::genesis_with(7, B256::repeat_byte(3), false), g);
        assert_ne!(Block::genesis_with(7, B256::repeat_byte(3), true).digest(), g.digest());
    }
}
