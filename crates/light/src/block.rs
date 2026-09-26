//! Consensus block: Commonware's certifiable block envelope around an Aether payload.
//! Structure adapted from alto-types (MIT OR Apache-2.0, commonwarexyz/alto).

use aether_types::{BlockAccessList, GasVector, TxEnvelope, B256};
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub parent_state_root: B256,
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
        let context =
            Context { round: Round::new(EPOCH, View::zero()), leader: ed25519::PrivateKey::from_seed(0).public_key(), parent: (View::zero(), Digest::EMPTY) };
        let payload = Payload { parent_state_root: genesis_root, ..Default::default() };
        let tag = Sha256::hash(&[b"aether-genesis".as_slice(), &chain_id.to_be_bytes()]);
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
    }
}
