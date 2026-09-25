//! EIP-7864 key derivation and account encoding.
//!
//! key = H(0^12 ‖ address ‖ overflow ‖ tree_index as 31-byte BE)[:31] ‖ sub_index
//!       (go-ethereum bintrie V3; cross-checked against the `ubt` crate)
//! - basic data  : tree_index 0, sub 0  (version | code_size u24 | nonce u64 | balance u128)
//! - code hash   : tree_index 0, sub 1
//! - storage < 64: tree_index 0, sub 64 + slot
//! - code chunks : position 128 + chunk_id
//! - other slots : position 256^31 + slot

use crate::tree::{TreeKey, Value};
use aether_hash::Hasher;
use alloy_primitives::{Address, U256};

pub const BASIC_DATA_LEAF_KEY: u8 = 0;
pub const CODE_HASH_LEAF_KEY: u8 = 1;
pub const HEADER_STORAGE_OFFSET: u64 = 64;
pub const CODE_OFFSET: u64 = 128;
pub const STEM_SUBTREE_WIDTH: u64 = 256;
const CHUNK_BYTES: usize = 31;
const PUSH1: u8 = 0x60;
const PUSH32: u8 = 0x7f;

pub fn tree_key<H: Hasher>(h: &H, address: &Address, tree_index: U256, sub_index: u8) -> TreeKey {
    // go-ethereum bintrie V3 / ubt: H(0^12 ‖ address ‖ overflow ‖ tree_index as 31-byte BE)
    let overflow = (tree_index >> 248) != U256::ZERO;
    let be = tree_index.to_be_bytes::<32>();
    let mut buf = [0u8; 64];
    buf[12..32].copy_from_slice(address.as_slice());
    buf[32] = overflow as u8;
    buf[33..].copy_from_slice(&be[1..]);
    let digest = h.hash_bytes(&buf); // no zero rule for key derivation (matches reference)
    let mut key = [0u8; 32];
    key[..31].copy_from_slice(&digest[..31]);
    key[31] = sub_index;
    key
}

pub fn basic_data_key<H: Hasher>(h: &H, a: &Address) -> TreeKey {
    tree_key(h, a, U256::ZERO, BASIC_DATA_LEAF_KEY)
}

pub fn code_hash_key<H: Hasher>(h: &H, a: &Address) -> TreeKey {
    tree_key(h, a, U256::ZERO, CODE_HASH_LEAF_KEY)
}

pub fn storage_slot_key<H: Hasher>(h: &H, a: &Address, slot: U256) -> TreeKey {
    let header_slots = U256::from(CODE_OFFSET - HEADER_STORAGE_OFFSET);
    if slot < header_slots {
        let pos = HEADER_STORAGE_OFFSET + slot.to::<u64>();
        tree_key(h, a, U256::ZERO, pos as u8)
    } else {
        // pos = 256^31 + slot. 256^31 is a multiple of 256, so the sub-index is the
        // low byte of `slot` and tree_index = 256^30 + (slot >> 8); no overflow.
        let tree_index = (U256::from(1u8) << 240) + (slot >> 8);
        tree_key(h, a, tree_index, slot.byte(0))
    }
}

pub fn code_chunk_key<H: Hasher>(h: &H, a: &Address, chunk_id: u64) -> TreeKey {
    let pos = CODE_OFFSET as u128 + chunk_id as u128;
    tree_key(h, a, U256::from(pos / STEM_SUBTREE_WIDTH as u128), (pos % STEM_SUBTREE_WIDTH as u128) as u8)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BasicData {
    pub version: u8,
    pub code_size: u32,
    pub nonce: u64,
    pub balance: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    CodeTooLarge,
    BalanceOverflow,
}

impl BasicData {
    pub fn encode(&self) -> Result<Value, LayoutError> {
        if self.code_size >= 1 << 24 {
            return Err(LayoutError::CodeTooLarge);
        }
        let mut v = [0u8; 32];
        v[0] = self.version;
        v[5..8].copy_from_slice(&self.code_size.to_be_bytes()[1..]);
        v[8..16].copy_from_slice(&self.nonce.to_be_bytes());
        v[16..32].copy_from_slice(&self.balance.to_be_bytes());
        Ok(v)
    }

    pub fn decode(v: &Value) -> BasicData {
        let mut cs = [0u8; 4];
        cs[1..].copy_from_slice(&v[5..8]);
        BasicData {
            version: v[0],
            code_size: u32::from_be_bytes(cs),
            nonce: u64::from_be_bytes(v[8..16].try_into().expect("8 bytes")),
            balance: u128::from_be_bytes(v[16..32].try_into().expect("16 bytes")),
        }
    }

    /// EIP-7864 stores balance as u128; larger EVM balances are rejected.
    pub fn with_balance(mut self, b: U256) -> Result<Self, LayoutError> {
        self.balance = u128::try_from(b).map_err(|_| LayoutError::BalanceOverflow)?;
        Ok(self)
    }
}

/// Split bytecode into 32-byte chunks: 1 byte = count of leading bytes that are
/// PUSH data continued from the previous chunk (capped at 31), then 31 code bytes.
pub fn chunkify_code(code: &[u8]) -> Vec<Value> {
    let padded_len = code.len().div_ceil(CHUNK_BYTES) * CHUNK_BYTES;
    let mut pushdata_left = vec![0u8; padded_len + 33];
    let mut pos = 0;
    while pos < code.len() {
        let op = code[pos];
        pos += 1;
        if (PUSH1..=PUSH32).contains(&op) {
            let n = (op - PUSH1 + 1) as usize;
            for x in 0..n {
                pushdata_left[pos + x] = (n - x) as u8;
            }
            pos += n;
        }
    }
    (0..padded_len)
        .step_by(CHUNK_BYTES)
        .map(|start| {
            let mut chunk = [0u8; 32];
            chunk[0] = pushdata_left[start].min(CHUNK_BYTES as u8);
            let end = (start + CHUNK_BYTES).min(code.len());
            chunk[1..1 + end - start].copy_from_slice(&code[start..end]);
            chunk
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_hash::Blake3;

    #[test]
    fn account_fields_share_one_stem() {
        let a = Address::repeat_byte(0xaa);
        let (b, c, s0, s63, code0) = (
            basic_data_key(&Blake3, &a),
            code_hash_key(&Blake3, &a),
            storage_slot_key(&Blake3, &a, U256::ZERO),
            storage_slot_key(&Blake3, &a, U256::from(63)),
            code_chunk_key(&Blake3, &a, 0),
        );
        for k in [c, s0, s63, code0] {
            assert_eq!(k[..31], b[..31], "header fields co-located in one stem");
        }
        assert_eq!((b[31], c[31], s0[31], s63[31], code0[31]), (0, 1, 64, 127, 128));
    }

    #[test]
    fn main_storage_lives_elsewhere_and_groups_by_256() {
        let a = Address::repeat_byte(1);
        let hdr = basic_data_key(&Blake3, &a);
        let s64 = storage_slot_key(&Blake3, &a, U256::from(64));
        let s65 = storage_slot_key(&Blake3, &a, U256::from(65));
        let s300 = storage_slot_key(&Blake3, &a, U256::from(300));
        assert_ne!(s64[..31], hdr[..31]);
        assert_eq!(s64[..31], s65[..31]);
        assert_eq!((s64[31], s65[31]), (64, 65));
        assert_ne!(s300[..31], s64[..31]);
        // max slot must not overflow
        let _ = storage_slot_key(&Blake3, &a, U256::MAX);
    }

    #[test]
    fn code_chunks_past_128_move_to_next_stem() {
        let a = Address::repeat_byte(2);
        assert_eq!(code_chunk_key(&Blake3, &a, 127)[..31], basic_data_key(&Blake3, &a)[..31]);
        assert_ne!(code_chunk_key(&Blake3, &a, 128)[..31], basic_data_key(&Blake3, &a)[..31]);
        assert_eq!(code_chunk_key(&Blake3, &a, 128)[31], 0);
    }

    #[test]
    fn basic_data_round_trip_and_limits() {
        let d = BasicData { version: 0, code_size: 0x12_3456, nonce: 7, balance: u128::MAX };
        assert_eq!(BasicData::decode(&d.encode().unwrap()), d);
        assert_eq!(BasicData { code_size: 1 << 24, ..d }.encode(), Err(LayoutError::CodeTooLarge));
        assert!(BasicData::default().with_balance(U256::from(u128::MAX) + U256::from(1)).is_err());
    }

    #[test]
    fn chunkify_marks_pushdata_continuation() {
        // 30 x STOP, then PUSH4 aa bb cc dd: the push data straddles chunks.
        let mut code = vec![0u8; 30];
        code.extend_from_slice(&[0x63, 0xaa, 0xbb, 0xcc, 0xdd, 0x00]);
        let chunks = chunkify_code(&code);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0][0], 0);
        assert_eq!(chunks[0][31], 0x63);
        assert_eq!(chunks[1][0], 4, "4 leading bytes are PUSH4 data");
        assert_eq!(&chunks[1][1..6], &[0xaa, 0xbb, 0xcc, 0xdd, 0x00]);
    }

    #[test]
    fn chunkify_caps_leading_count_at_31() {
        // PUSH32 at offset 0 then 32 data bytes: chunk 1 starts with 2 more push bytes.
        let mut code = vec![PUSH32];
        code.extend(std::iter::repeat_n(0xee, 32));
        let chunks = chunkify_code(&code);
        assert_eq!(chunks[0][0], 0);
        assert_eq!(chunks[1][0], 2);
        let mut long = vec![PUSH32];
        long.extend(std::iter::repeat_n(0x01, 32));
        long.extend(std::iter::repeat_n(0x00, 60));
        assert!(chunkify_code(&long).iter().all(|c| c[0] <= 31));
    }
}
