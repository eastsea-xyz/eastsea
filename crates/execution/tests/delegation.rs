//! EIP-7702 delegation for P-256 accounts: batched calls through AetherAccount.

use aether_crypto::{P256Signer, Signer};
use aether_execution::{
    aether_account_code, encode_execute, execute_block, execute_block_sequential, sign_call, BlockContext, EvmCall, WorldState, AETHER_ACCOUNT,
};
use aether_types::{Address, Bytes, GasVector, TxEnvelope, U256};

const CHAIN: u64 = 7_777;

fn seed(b: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[0] = 0x77;
    s[31] = b;
    s
}

fn ctx(n: u64) -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: n,
        timestamp: n,
        beneficiary: Address::repeat_byte(0xbe),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
    }
}

struct Setup {
    alice: P256Signer,
    a: Address,
    pre: WorldState,
}

fn setup() -> Setup {
    let alice = P256Signer::from_seed(&seed(1)).unwrap();
    let a = aether_crypto::address_of(&alice.public_key()).unwrap();
    let mut pre = WorldState::default();
    pre.set_balance(a, U256::from(10u128.pow(21))).unwrap();
    pre.set_code(AETHER_ACCOUNT, aether_account_code()).unwrap();
    Setup { alice, a, pre }
}

fn tx(s: &Setup, nonce: u64, call: EvmCall) -> TxEnvelope {
    sign_call(&s.alice, CHAIN, nonce, 1, &call).unwrap()
}

fn batch(to: Address, delegate: Option<Address>, calls: &[(Address, u64)]) -> EvmCall {
    let calls: Vec<_> = calls.iter().map(|(t, v)| (*t, U256::from(*v), Bytes::new())).collect();
    EvmCall { to: Some(to), value: U256::ZERO, input: encode_execute(&calls), gas_limit: 300_000, delegate }
}

#[test]
fn delegate_and_batch_in_one_signed_tx() {
    let s = setup();
    let (bob, carol) = (Address::repeat_byte(0xb0), Address::repeat_byte(0xc0));
    // One tx: set the delegation AND run a two-transfer batch.
    let t = tx(&s, 0, batch(s.a, Some(AETHER_ACCOUNT), &[(bob, 5), (carol, 7)]));
    let out = execute_block(&s.pre, &ctx(1), std::slice::from_ref(&t)).unwrap();
    assert!(out.receipts[0].success, "{:?}", out.receipts[0]);
    let st = &out.state;
    assert_eq!(st.balance(&bob), U256::from(5u64));
    assert_eq!(st.balance(&carol), U256::from(7u64));
    assert_eq!(st.nonce(&s.a), 2, "tx nonce + the authorization nonce");
    let mut designator = vec![0xef, 0x01, 0x00];
    designator.extend_from_slice(AETHER_ACCOUNT.as_slice());
    assert_eq!(st.code(&s.a).to_vec(), designator, "7702 designator installed");
    assert!(out.bal.accounts.iter().any(|x| x.address == s.a && x.code_touched), "BAL records the code change");
    let seq = execute_block_sequential(&s.pre, &ctx(1), &[t]).unwrap();
    assert_eq!(seq.state.root(), out.state.root());

    // Later batches need no delegation field; the delegation persists.
    let dave = Address::repeat_byte(0xd0);
    let out2 = execute_block(st, &ctx(2), &[tx(&s, 2, batch(s.a, None, &[(dave, 3), (bob, 1)]))]).unwrap();
    assert!(out2.receipts[0].success);
    assert_eq!(out2.state.balance(&dave), U256::from(3u64));
    assert_eq!(out2.state.balance(&bob), U256::from(6u64));

    // Clearing: delegate to the zero address removes the code.
    let clear = EvmCall { to: Some(s.a), value: U256::ZERO, input: Bytes::new(), gas_limit: 100_000, delegate: Some(Address::ZERO) };
    let out3 = execute_block(&out2.state, &ctx(3), &[tx(&s, 3, clear)]).unwrap();
    assert!(out3.state.code(&s.a).is_empty(), "delegation cleared");
}

#[test]
fn nobody_else_can_drive_the_account() {
    let s = setup();
    let st = execute_block(&s.pre, &ctx(1), &[tx(&s, 0, batch(s.a, Some(AETHER_ACCOUNT), &[]))]).unwrap().state;
    // Mallory calls alice's account asking it to pay her.
    let mallory = P256Signer::from_seed(&seed(9)).unwrap();
    let m = aether_crypto::address_of(&mallory.public_key()).unwrap();
    let mut st = st;
    st.set_balance(m, U256::from(10u128.pow(20))).unwrap();
    let steal = sign_call(&mallory, CHAIN, 0, 1, &batch(s.a, None, &[(m, 1_000)])).unwrap();
    let out = execute_block(&st, &ctx(2), &[steal]).unwrap();
    assert!(!out.receipts[0].success, "OnlySelf revert");
    assert_eq!(out.state.balance(&s.a), st.balance(&s.a), "alice's balance untouched");
    assert!(out.state.balance(&m) < st.balance(&m), "mallory only paid gas");
}

#[test]
fn delegation_codec_round_trip_and_old_encodings_still_decode() {
    let with = batch(Address::repeat_byte(1), Some(AETHER_ACCOUNT), &[(Address::repeat_byte(2), 1)]);
    assert_eq!(EvmCall::decode(&with.encode()).unwrap(), with);
    let without = EvmCall { delegate: None, ..with.clone() };
    let enc = without.encode();
    assert_eq!(EvmCall::decode(&enc).unwrap(), without);
    assert_eq!(enc.len() + 21, with.encode().len(), "delegation is a pure trailer");
}
