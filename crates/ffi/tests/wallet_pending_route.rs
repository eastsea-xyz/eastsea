//! B5 review round 2, finding 2: a remote wallet (no node on this Mac) sends
//! through a validator, but its reads go to follower Macs first — and a
//! follower never sees the validators' pending transactions, so it answers
//! `null` for a send that is waiting in a validator's mempool. The status
//! must come from the node that admitted the send, or from a validator when
//! a follower cannot answer; a follower's ignorance is never the answer.
//! Real QUIC on localhost (fake follower + fake validator), no network.

#[allow(dead_code)]
mod wallet_common;

use aether_crypto::Signer;
use aether_ffi::{pin_servers, submit_signed, tx_status};
use aether_types::{Address, Bytes, FeeVector, U256};
use wallet_common::{follower, holding, HELD_TX};

#[test]
fn a_remote_wallet_reads_its_pending_send_from_the_validator_that_admitted_it() {
    let follower = follower();
    let validator = holding();
    pin_servers(vec![follower.pinned()], vec![validator.pinned()]).unwrap();

    // A hash this process did not submit (e.g. after an app restart): the
    // follower answers null, so a validator is asked before "unknown".
    let st = tx_status(HELD_TX.into()).unwrap();
    assert_eq!(st.state, "pending", "a follower's null hid the validator's pending tx: {}", st.detail);
    assert_eq!(st.reason.as_deref(), Some("state_price_above_cap"));
    assert!(follower.served("aether_getReceipt") >= 1, "the ordinary read path still starts at a follower");

    // A send this process submitted: the validator that admitted it is asked
    // first, and the follower is not asked at all.
    let signer = aether_crypto::P256Signer::from_seed(&[9u8; 32]).unwrap();
    let call = aether_execution::EvmCall { to: Some(Address::repeat_byte(0x42)), value: U256::from(1u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
    let caps = FeeVector { exec: 2_000_000_000, state: 2_000_000_000_000, prove: 0 };
    let mut env = aether_execution::sign_call_with(&signer, 7_777, 0, caps, 1_000_000_000, &call).unwrap();
    let signature = env.signature.0[..64].to_vec();
    env.signature = Bytes::new();
    let h = submit_signed(serde_json::to_string(&env).unwrap(), signature, signer.public_key().bytes.clone()).unwrap();
    assert_eq!(h, HELD_TX);
    let asked_follower = follower.served("aether_getReceipt");
    let asked_validator = validator.served("aether_getReceipt");
    let st = tx_status(h).unwrap();
    assert_eq!(st.state, "pending", "{}", st.detail);
    assert_eq!(validator.served("aether_getReceipt"), asked_validator + 1, "the admitting validator answers first");
    assert_eq!(follower.served("aether_getReceipt"), asked_follower, "the follower is not asked for our own pending send");
}
