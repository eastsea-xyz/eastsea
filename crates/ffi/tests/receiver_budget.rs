//! Public wallet sends must quote and sign enough state and execution gas for
//! empty-calldata receivers. Loopback RPC and synthetic keys only.

#[path = "common/http.rs"]
mod http;

use aether_crypto::{address_of, P256Signer, Signer};
use aether_execution::block::{receipt_persistent_bytes, tx_persistent_bytes};
use aether_execution::fees::{STATE_SLOT_UNITS, STATE_UNIT_PRICE};
use aether_execution::tx::{signed_fee_maximum, PLAIN_TRANSFER_GAS};
use aether_execution::{
    check_admission_cost, execute_block, BlockContext, FeePolicy, WorldState, FEE_COLLECTOR,
};
use aether_ffi::{prepare_transfer, prepare_transfer_at, transfer_quote, use_local_node};
use aether_types::{Address, Bytes, FeeVector, GasVector, TxEnvelope, U256};
use http::RpcFixture;
use serde_json::json;

const CHAIN: u64 = 7_777;
const VALUE: u64 = 4_883;
// Store caller, origin, chain id, value, block number and timestamp in six
// fresh slots, matching the owned-devnet B5 receiver.
const RECEIVER_CODE: &[u8] = &[
    0x33, 0x60, 0x00, 0x55, 0x32, 0x60, 0x01, 0x55, 0x46, 0x60, 0x02, 0x55, 0x34, 0x60, 0x03, 0x55,
    0x43, 0x60, 0x04, 0x55, 0x42, 0x60, 0x05, 0x55, 0x00,
];

struct ResetLocalNode;
impl Drop for ResetLocalNode {
    fn drop(&mut self) {
        use_local_node(None);
    }
}

#[test]
fn public_transfer_quote_and_preparation_cover_contract_and_delegated_receivers() {
    let delegate = Address::repeat_byte(0x77);
    let mut designator = vec![0xef, 0x01, 0x00];
    designator.extend_from_slice(delegate.as_slice());
    // This test binary owns the FFI globals; all recipient cases run in order.
    for (seed, recipient, code) in [
        (1, Address::repeat_byte(0x55), RECEIVER_CODE.to_vec()),
        (2, Address::repeat_byte(0x56), designator),
        (3, Address::repeat_byte(0x57), Vec::new()),
        // An unavailable code lookup must not claim the recipient has no code.
        (4, Address::repeat_byte(0x58), RECEIVER_CODE.to_vec()),
        (5, Address::repeat_byte(0x59), RECEIVER_CODE.to_vec()),
        (6, Address::repeat_byte(0x5a), RECEIVER_CODE.to_vec()),
    ] {
        let signer = P256Signer::from_seed(&[seed; 32]).unwrap();
        let sender = address_of(&signer.public_key()).unwrap();
        let balance = U256::from(10u128.pow(24));
        let served_code = format!("0x{}", alloy_primitives::hex::encode(&code));
        let fixture = RpcFixture::start(move |request| match request["method"].as_str().unwrap() {
            "aether_status" => json!({
                "chain_id": CHAIN,
                "base_fee": { "exec": "0", "state": STATE_UNIT_PRICE.to_string(), "prove": "0" },
            }),
            "eth_getCode" => {
                assert_eq!(
                    request["params"][0]
                        .as_str()
                        .unwrap()
                        .parse::<Address>()
                        .unwrap(),
                    recipient
                );
                match seed {
                    4 => serde_json::Value::Null,
                    5 => json!(""),
                    6 => json!("0x0x"),
                    _ => json!(served_code),
                }
            }
            "eth_getBalance" => json!(format!("0x{balance:x}")),
            "eth_getTransactionCount" => json!("0x0"),
            method => panic!("unexpected fixture method: {method}"),
        });
        use_local_node(Some(fixture.port));
        let _reset = ResetLocalNode;

        let quote = transfer_quote(recipient.to_string(), 4).unwrap();
        let prepared = prepare_transfer(
            signer.public_key().bytes.clone(),
            recipient.to_string(),
            VALUE.to_string(),
            Some(quote.fee_wei.clone()),
            4,
        )
        .unwrap();
        let resent = prepare_transfer_at(
            signer.public_key().bytes.clone(),
            recipient.to_string(),
            VALUE.to_string(),
            Some(quote.fee_wei.clone()),
            4,
            0,
        )
        .unwrap();
        assert_eq!(
            resent.envelope_json, prepared.envelope_json,
            "resend signs the same budgets at the same snapshot"
        );
        let mut tx: TxEnvelope = serde_json::from_str(&prepared.envelope_json).unwrap();
        assert_eq!(prepared.signing_message, tx.signing_bytes());
        assert_eq!(
            quote.fee_wei.parse::<u128>().unwrap(),
            signed_fee_maximum(&tx.header.gas, &tx.header.max_fee)
        );
        if code.is_empty() {
            assert_eq!(tx.header.gas.exec, PLAIN_TRANSFER_GAS);
            assert_eq!(
                tx.header.gas.state, 216,
                "the EOA transfer budget stays unchanged"
            );
        } else {
            assert!(
                tx.header.gas.state > 216,
                "a receiver must reserve its execution's state growth"
            );
        }
        let mut signature = signer.sign(&prepared.signing_message).unwrap();
        signature.extend_from_slice(&signer.public_key().bytes);
        tx.signature = Bytes::from(signature);

        let mut state = WorldState::default();
        state.set_balance(sender, balance).unwrap();
        if !code.is_empty() {
            state
                .set_code(recipient, Bytes::from(code.clone()))
                .unwrap();
        }
        if seed == 2 {
            state
                .set_code(delegate, Bytes::from_static(RECEIVER_CODE))
                .unwrap();
        }
        let context = BlockContext {
            chain_id: CHAIN,
            number: 1,
            timestamp: 1,
            beneficiary: FEE_COLLECTOR,
            limits: GasVector {
                exec: 30_000_000,
                state: 100_000,
                prove: 200_000_000,
            },
            fees: Some(FeePolicy {
                base: FeeVector {
                    exec: 0,
                    state: STATE_UNIT_PRICE,
                    prove: 0,
                },
                proposer: Address::repeat_byte(0xbe),
            }),
        };
        let admitted = check_admission_cost(&state, &context, &tx)
            .expect("the wallet's signed send is admitted");
        let out = execute_block(&state, &context, std::slice::from_ref(&tx)).unwrap();
        let receipt = &out.receipts[0];
        assert!(
            receipt.success,
            "wallet execution gas must cover all six writes"
        );
        assert_eq!(out.gas, admitted.gas);
        assert_eq!(out.state.balance(&recipient), U256::from(VALUE));
        assert_eq!(
            receipt.state_fee,
            U256::from(receipt.state_gas) * U256::from(STATE_UNIT_PRICE)
        );
        assert_eq!(
            balance - out.state.balance(&sender),
            U256::from(VALUE) + receipt.state_fee
        );
        assert!(receipt.state_gas <= tx.header.gas.state);
        assert!(
            receipt.state_fee < U256::from(quote.fee_wei.parse::<u128>().unwrap()),
            "unused maximum fee is not charged"
        );
        if !code.is_empty() {
            let expected = [
                U256::from_be_slice(sender.as_slice()),
                U256::from_be_slice(sender.as_slice()),
                U256::from(CHAIN),
                U256::from(VALUE),
                U256::from(1u64),
                U256::from(1u64),
            ];
            for (slot, value) in expected.into_iter().enumerate() {
                assert_eq!(out.state.storage(&recipient, U256::from(slot)), value);
                if seed == 2 {
                    assert_eq!(
                        out.state.storage(&delegate, U256::from(slot)),
                        U256::ZERO,
                        "delegation writes the recipient's storage"
                    );
                }
            }
            let archived =
                (tx_persistent_bytes(&tx) + receipt_persistent_bytes(receipt)).div_ceil(32);
            assert_eq!(receipt.state_gas, 6 * STATE_SLOT_UNITS + archived);
            assert_eq!(admitted.new_slots, 6);
        }
    }
}
