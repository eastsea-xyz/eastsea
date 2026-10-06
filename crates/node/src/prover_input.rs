//! The bytes the proving sidecar and its guest read a `BlockInput` from.
//!
//! Both decode the input with `postcard::from_bytes::<BlockInput>`. postcard
//! is not self-describing: a struct is its fields in order, every field always
//! present. `TxHeader::group` is `#[serde(skip_serializing_if = "Option::is_none")]`
//! (so the JSON block payload and the signed bytes of every existing tx stay
//! as they were), which makes `postcard::to_allocvec(&input)` drop the field
//! for an ungrouped tx while the decoder still reads one: every input with a
//! transaction failed with "input is not a postcard BlockInput" (2026-10-06
//! rehearsal; on chains since 2026-09-30).
//!
//! The derive is consensus-relevant twice (the JSON payload, and the guest's
//! `txs_hash` over the postcard bytes of the txs), so it stays. The node writes
//! the decoder's layout instead: the same fields in the same order, with
//! `group` always present. The guest program, its id, and every commitment
//! are unchanged.

use aether_proving::block::BlockInput;
use aether_types::{Address, Bytes, FeeVector, GasVector, Hash, SignerScheme, TxEnvelope, TxPayload};
use serde::Serialize;

#[derive(Serialize)]
struct Input<'a> {
    ctx: &'a aether_execution::BlockContext,
    txs: Vec<Tx<'a>>,
    activate: &'a [u32],
    pre_state_root: &'a aether_types::B256,
    witness: &'a aether_execution::StateWitness,
    prover: &'a Address,
}

#[derive(Serialize)]
struct Tx<'a> {
    header: Header<'a>,
    payload: &'a TxPayload,
    signature: &'a Bytes,
}

#[derive(Serialize)]
struct Header<'a> {
    chain_id: u64,
    sender: &'a Address,
    nonce: u64,
    gas: &'a GasVector,
    max_fee: &'a FeeVector,
    tip: u128,
    payload_commitment: &'a Hash,
    scheme: &'a SignerScheme,
    /// Always written: the decoder reads an `Option` tag here.
    group: Option<u16>,
}

fn tx(t: &TxEnvelope) -> Tx<'_> {
    let h = &t.header;
    Tx {
        header: Header {
            chain_id: h.chain_id,
            sender: &h.sender,
            nonce: h.nonce,
            gas: &h.gas,
            max_fee: &h.max_fee,
            tip: h.tip,
            payload_commitment: &h.payload_commitment,
            scheme: &h.scheme,
            group: h.group,
        },
        payload: &t.payload,
        signature: &t.signature,
    }
}

/// The postcard bytes of `input` as the sidecar and guest decode them, checked
/// by decoding them back: an input the prover cannot read is an error here,
/// with its reason, not a refusal from the sidecar later.
pub fn encode(input: &BlockInput) -> Result<Vec<u8>, String> {
    let wire = Input {
        ctx: &input.ctx,
        txs: input.txs.iter().map(tx).collect(),
        activate: &input.activate,
        pre_state_root: &input.pre_state_root,
        witness: &input.witness,
        prover: &input.prover,
    };
    let bytes = postcard::to_allocvec(&wire).map_err(|e| format!("encode the prover input: {e}"))?;
    let back: BlockInput = postcard::from_bytes(&bytes).map_err(|e| format!("the prover could not decode this input: {e}"))?;
    if back.txs != input.txs || back.ctx != input.ctx || back.pre_state_root != input.pre_state_root || back.prover != input.prover {
        return Err("the prover input does not decode to itself".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_crypto::{P256Signer, Signer};
    use aether_execution::{EvmCall, WorldState};
    use aether_types::U256;

    fn input(group: Option<u16>) -> BlockInput {
        let key = P256Signer::from_seed(&[3; 32]).unwrap();
        let mut pre = WorldState::default();
        pre.set_balance(aether_crypto::address_of(&key.public_key()).unwrap(), U256::from(10u128.pow(21))).unwrap();
        let call = EvmCall { to: Some(Address::repeat_byte(0x40)), value: U256::from(1u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
        let mut tx = aether_execution::sign_call(&key, 7, 0, 1, &call).unwrap();
        tx.header.group = group;
        let ctx = aether_execution::BlockContext {
            chain_id: 7,
            number: 1,
            timestamp: 1,
            beneficiary: Address::repeat_byte(0xbe),
            limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: u64::MAX },
            fees: None,
        };
        // The witness is built without executing (a group-tagged signature need not verify here).
        BlockInput {
            witness: pre.witness_for(&pre),
            pre_state_root: pre.root(),
            ctx,
            txs: vec![tx],
            activate: vec![],
            prover: Address::repeat_byte(7),
        }
    }

    #[test]
    fn an_ungrouped_tx_is_written_the_way_the_sidecar_reads_it() {
        let ungrouped = input(None);
        let derived = postcard::to_allocvec(&ungrouped).unwrap();
        assert!(postcard::from_bytes::<BlockInput>(&derived).is_err(), "the plain derive drops the field");
        let wire = encode(&ungrouped).unwrap();
        let back: BlockInput = postcard::from_bytes(&wire).unwrap();
        assert_eq!(back.txs, ungrouped.txs);
        // The guest's txs_hash re-serializes with the derive: unchanged.
        assert_eq!(aether_proving::block::txs_hash(&back.txs), aether_proving::block::txs_hash(&ungrouped.txs));
    }

    #[test]
    fn a_grouped_tx_encodes_exactly_as_the_derive_does() {
        let grouped = input(Some(1));
        assert_eq!(encode(&grouped).unwrap(), postcard::to_allocvec(&grouped).unwrap());
    }
}
