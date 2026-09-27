//! Durable state: what is committed comes back with the same EIP-7864 root.

use aether_crypto::P256Signer;
use aether_execution::{build_block, sign_call, BlockContext, EvmCall, WorldState};
use aether_node::chain::{dev_seed, BlockSummary};
use aether_node::store::{Commit, Store, StoreError};
use aether_types::{Address, Bytes, GasVector, U256};

const CHAIN: u64 = 7777;
const COUNTER_INIT: &str = "600a600c600039600a6000f360005460010160005500";

fn signer(dev: u8) -> P256Signer {
    P256Signer::from_seed(&dev_seed(dev)).unwrap()
}

fn sender(dev: u8) -> Address {
    aether_crypto::address_of(&aether_crypto::Signer::public_key(&signer(dev))).unwrap()
}

fn ctx(n: u64) -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: n,
        timestamp: n,
        beneficiary: Address::repeat_byte(0x77),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        fees: None,
    }
}

fn summary(height: u64, s: &WorldState) -> BlockSummary {
    BlockSummary {
        height,
        hash: format!("{height:064x}"),
        parent: String::new(),
        timestamp_ms: height * 1000,
        proposer: Address::ZERO,
        state_root: s.root(),
        parent_state_root: Default::default(),
        txs: vec![],
        gas_used: 0,
        prove_gas: 0,
        base_fee: Default::default(),
        excess: Default::default(),
    }
}

fn commit(store: &Store, height: u64, s: &WorldState) {
    let sm = summary(height, s);
    store
        .commit(Commit {
            height,
            digest: [height as u8; 32],
            root: s.root(),
            diff: s.journal(),
            summary: &sm,
            receipts: vec![],
            handoff: None,
            seed: None,
            history: &Default::default(),
        })
        .unwrap();
}

#[test]
fn committed_state_reloads_with_identical_root() {
    let dir = std::env::temp_dir().join(format!("aether-store-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("state.redb");
    let store = Store::open(&path).unwrap();
    assert!(store.load().unwrap().is_none());

    // Genesis: two funded accounts.
    let mut s = WorldState::default();
    s.set_balance(sender(1), U256::from(10u128.pow(24))).unwrap();
    s.set_balance(sender(2), U256::from(10u128.pow(24))).unwrap();
    commit(&store, 0, &s);

    // Block 1: deploy a contract (code + chunks) and a transfer.
    let deploy = EvmCall { to: None, value: U256::ZERO, input: Bytes::from(hex::decode(COUNTER_INIT).unwrap()), gas_limit: 3_000_000, delegate: None };
    let pay = EvmCall { to: Some(Address::repeat_byte(0xb0)), value: U256::from(5u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
    let txs = vec![sign_call(&signer(1), CHAIN, 0, 1, &deploy).unwrap(), sign_call(&signer(2), CHAIN, 0, 1, &pay).unwrap()];
    let (included, out) = build_block(&s, &ctx(1), txs);
    assert_eq!(included.len(), 2);
    let contract = out.receipts[0].contract_address.expect("deployed");
    commit(&store, 1, &out.state);

    // Block 2: call the counter (storage write) and drain dev 2 to zero-ish.
    let call = EvmCall { to: Some(contract), value: U256::ZERO, input: Bytes::new(), gas_limit: 100_000, delegate: None };
    let (_, out2) = build_block(&out.state, &ctx(2), vec![sign_call(&signer(1), CHAIN, 1, 1, &call).unwrap()]);
    assert_eq!(out2.state.storage(&contract, U256::ZERO), U256::from(1u64));
    commit(&store, 2, &out2.state);
    drop(store);

    let cp = Store::open(&path).unwrap().load().unwrap().expect("checkpoint");
    assert_eq!(cp.height, 2);
    assert_eq!(cp.digest, [2u8; 32]);
    assert_eq!(cp.state.root(), out2.state.root());
    assert_eq!(cp.state.storage(&contract, U256::ZERO), U256::from(1u64));
    assert_eq!(cp.state.code(&contract), out2.state.code(&contract));
    assert_eq!(cp.state.balance(&Address::repeat_byte(0xb0)), U256::from(5u64));
    assert_eq!(cp.blocks.keys().copied().collect::<Vec<_>>(), vec![0, 1, 2]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tampered_state_is_detected() {
    let dir = std::env::temp_dir().join(format!("aether-store-tamper-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("state.redb");
    let store = Store::open(&path).unwrap();
    let mut s = WorldState::default();
    s.set_balance(sender(1), U256::from(1000u64)).unwrap();
    commit(&store, 0, &s);

    // A diff that changes state without matching the recorded root.
    let mut forged = s.clone();
    forged.clear_journal();
    forged.set_balance(sender(1), U256::from(999_999u64)).unwrap();
    let sm = summary(1, &s);
    store
        .commit(Commit {
            height: 1,
            digest: [1; 32],
            root: s.root(),
            diff: forged.journal(),
            summary: &sm,
            receipts: vec![],
            handoff: None,
            seed: None,
            history: &Default::default(),
        })
        .unwrap();

    match store.load() {
        Err(StoreError::RootMismatch { height: 1, .. }) => {}
        other => panic!("expected root mismatch, got {:?}", other.map(|c| c.map(|c| c.height))),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
