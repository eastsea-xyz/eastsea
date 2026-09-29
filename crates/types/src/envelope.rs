use crate::{put_bytes, put_u64, Canonical, Hash};
use alloy_primitives::{Address, Bytes};
use serde::{Deserialize, Serialize};

/// Three-dimensional gas: EVM execution, state growth (EIP-8037), proving cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GasVector {
    pub exec: u64,
    pub state: u64,
    pub prove: u64,
}

impl GasVector {
    pub fn checked_add(self, o: GasVector) -> Option<GasVector> {
        Some(GasVector { exec: self.exec.checked_add(o.exec)?, state: self.state.checked_add(o.state)?, prove: self.prove.checked_add(o.prove)? })
    }

    /// True when every dimension fits within `limit`.
    pub fn fits(&self, limit: &GasVector) -> bool {
        self.exec <= limit.exec && self.state <= limit.state && self.prove <= limit.prove
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeVector {
    pub exec: u128,
    pub state: u128,
    pub prove: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SignerScheme {
    /// Secure Enclave / passkey signer, verified with P256VERIFY (EIP-7951).
    P256 = 1,
    Secp256k1 = 2,
    Ed25519 = 3,
}

impl SignerScheme {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::P256),
            2 => Some(Self::Secp256k1),
            3 => Some(Self::Ed25519),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxHeader {
    pub chain_id: u64,
    pub sender: Address,
    pub nonce: u64,
    pub gas: GasVector,
    pub max_fee: FeeVector,
    /// Max priority fee per exec gas (EIP-1559 tip), paid on top of the base fee within `max_fee.exec`.
    #[serde(default)]
    pub tip: u128,
    /// Commitment to the payload; for encrypted payloads it is checked after decryption.
    pub payload_commitment: Hash,
    pub scheme: SignerScheme,
    /// The group this tx runs in (0 = the only group today). None is group 0
    /// and keeps the v2 signing bytes, so every existing signature stays valid;
    /// any other group signs under the v3 tag instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u16>,
}

impl TxHeader {
    /// The group this tx belongs to: None means 0, the only group today.
    pub fn group(&self) -> u16 {
        self.group.unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxPayload {
    /// EIP-2718 encoded EVM transaction body.
    Plain(Bytes),
    /// Timelock-encrypted payload (phase 2).
    Encrypted { epoch: u64, ciphertext: Bytes },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxEnvelope {
    pub header: TxHeader,
    pub payload: TxPayload,
    pub signature: Bytes,
}

impl Canonical for GasVector {
    fn encode_canonical(&self, out: &mut alloc::vec::Vec<u8>) {
        put_u64(out, self.exec);
        put_u64(out, self.state);
        put_u64(out, self.prove);
    }
}

impl Canonical for FeeVector {
    fn encode_canonical(&self, out: &mut alloc::vec::Vec<u8>) {
        out.extend_from_slice(&self.exec.to_be_bytes());
        out.extend_from_slice(&self.state.to_be_bytes());
        out.extend_from_slice(&self.prove.to_be_bytes());
    }
}

impl Canonical for TxHeader {
    fn encode_canonical(&self, out: &mut alloc::vec::Vec<u8>) {
        match self.group {
            None => out.extend_from_slice(b"aether/tx-header/v2"),
            // A group other than 0 changes what is signed (v3): a signer that
            // knows no group can never produce these bytes by accident.
            Some(group) => {
                out.extend_from_slice(b"aether/tx-header/v3");
                out.extend_from_slice(&group.to_be_bytes());
            }
        }
        put_u64(out, self.chain_id);
        out.extend_from_slice(self.sender.as_slice());
        put_u64(out, self.nonce);
        self.gas.encode_canonical(out);
        self.max_fee.encode_canonical(out);
        out.extend_from_slice(&self.tip.to_be_bytes());
        out.extend_from_slice(self.payload_commitment.as_slice());
        out.push(self.scheme as u8);
    }
}

impl Canonical for TxPayload {
    fn encode_canonical(&self, out: &mut alloc::vec::Vec<u8>) {
        match self {
            TxPayload::Plain(b) => {
                out.push(0);
                put_bytes(out, b);
            }
            TxPayload::Encrypted { epoch, ciphertext } => {
                out.push(1);
                put_u64(out, *epoch);
                put_bytes(out, ciphertext);
            }
        }
    }
}

impl TxEnvelope {
    /// Bytes the sender signs: the header (which commits to the payload).
    pub fn signing_bytes(&self) -> alloc::vec::Vec<u8> {
        self.header.to_canonical_bytes()
    }
}

impl Canonical for TxEnvelope {
    fn encode_canonical(&self, out: &mut alloc::vec::Vec<u8>) {
        self.header.encode_canonical(out);
        self.payload.encode_canonical(out);
        put_bytes(out, &self.signature);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;

    fn header(nonce: u64) -> TxHeader {
        TxHeader {
            chain_id: 7,
            sender: Address::repeat_byte(1),
            nonce,
            gas: GasVector { exec: 21_000, state: 0, prove: 100 },
            max_fee: FeeVector::default(),
            tip: 0,
            payload_commitment: B256::repeat_byte(9),
            scheme: SignerScheme::P256,
            group: None,
        }
    }

    #[test]
    fn canonical_encoding_is_deterministic_and_field_sensitive() {
        assert_eq!(header(1).to_canonical_bytes(), header(1).to_canonical_bytes());
        assert_ne!(header(1).to_canonical_bytes(), header(2).to_canonical_bytes());
        let mut h = header(1);
        h.scheme = SignerScheme::Ed25519;
        assert_ne!(h.to_canonical_bytes(), header(1).to_canonical_bytes());
    }

    #[test]
    fn group_none_keeps_v2_bytes_and_other_groups_sign_differently() {
        let v2 = header(1).to_canonical_bytes();
        assert_eq!(&v2[..19], b"aether/tx-header/v2");
        assert_eq!(header(1).group(), 0);
        // A group tx signs under the v3 tag: never the same bytes as v2.
        let mut g7 = header(1);
        g7.group = Some(7);
        let v3 = g7.to_canonical_bytes();
        assert_eq!(&v3[..19], b"aether/tx-header/v3");
        assert_eq!(&v3[19..21], 7u16.to_be_bytes());
        assert_ne!(v2, v3);
        let mut g8 = g7.clone();
        g8.group = Some(8);
        assert_ne!(v3, g8.to_canonical_bytes());
        // Serde keeps the field optional, so old JSON round-trips unchanged.
        let json = serde_json::to_string(&header(1)).unwrap();
        assert!(!json.contains("group"));
        assert_eq!(serde_json::from_str::<TxHeader>(&json).unwrap(), header(1));
    }

    #[test]
    fn gas_vector_limits() {
        let limit = GasVector { exec: 10, state: 10, prove: 10 };
        assert!(GasVector { exec: 10, state: 0, prove: 10 }.fits(&limit));
        assert!(!GasVector { exec: 0, state: 0, prove: 11 }.fits(&limit));
        assert_eq!(GasVector { exec: u64::MAX, ..Default::default() }.checked_add(GasVector { exec: 1, ..Default::default() }), None);
    }

    #[test]
    fn scheme_round_trip() {
        for s in [SignerScheme::P256, SignerScheme::Secp256k1, SignerScheme::Ed25519] {
            assert_eq!(SignerScheme::from_u8(s as u8), Some(s));
        }
        assert_eq!(SignerScheme::from_u8(0), None);
    }
}
