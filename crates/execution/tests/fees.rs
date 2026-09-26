//! Fee policy v0 (docs/research/tokenomics-2026.md): base exec fee burned,
//! prove fee to the prover escrow, tips split 60/20/20, and value conserved.

use aether_crypto::{P256Signer, Signer};
use aether_execution::{
    build_block, build_block_sequential, execute_block, execute_block_sequential, sign_call, sign_call_with, BlockContext, EvmCall, FeePolicy, WorldState,
    FEE_COLLECTOR, PROVER_ESCROW,
};
use aether_types::{Address, Bytes, FeeVector, GasVector, U256};

const CHAIN: u64 = 7_777;
const GWEI: u128 = 1_000_000_000;
const PROPOSER: Address = Address::repeat_byte(0xbe);
/// Runtime: slot0 += 1 (a few interpreted steps, so prove gas > 0).
const COUNTER_INIT: &[u8] = &[
    0x60, 0x0a, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x0a, 0x60, 0x00, 0xf3, // init
    0x60, 0x00, 0x54, 0x60, 0x01, 0x01, 0x60, 0x00, 0x55, 0x00, // runtime
];

fn seed(b: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[31] = b;
    s
}

fn ctx(base: FeeVector) -> BlockContext {
    BlockContext {
        chain_id: CHAIN,
        number: 1,
        timestamp: 1_700_000_001,
        beneficiary: FEE_COLLECTOR,
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
        fees: Some(FeePolicy { base, proposer: PROPOSER }),
    }
}

fn base() -> FeeVector {
    FeeVector { exec: 3 * GWEI, state: 0, prove: GWEI }
}

fn total(s: &WorldState, addrs: &[Address]) -> U256 {
    addrs.iter().map(|a| s.balance(a)).fold(U256::ZERO, |x, y| x + y)
}

#[test]
fn fees_split_and_value_is_conserved() {
    let signers: Vec<P256Signer> = (1..=4).map(|i| P256Signer::from_seed(&seed(i)).unwrap()).collect();
    let addrs: Vec<Address> = signers.iter().map(|s| aether_crypto::address_of(&s.public_key()).unwrap()).collect();
    let mut pre = WorldState::default();
    for a in &addrs {
        pre.set_balance(*a, U256::from(10u128.pow(21))).unwrap();
    }
    let bob = Address::repeat_byte(0xb0);
    let mut txs = Vec::new();
    for (i, s) in signers.iter().enumerate() {
        // Tip of (i + 1) gwei on top of the 3 gwei base.
        let fee = FeeVector { exec: (4 + i as u128) * GWEI, state: 0, prove: GWEI };
        let call = if i == 0 {
            EvmCall { to: None, value: U256::ZERO, input: Bytes::from_static(COUNTER_INIT), gas_limit: 200_000, delegate: None }
        } else {
            EvmCall { to: Some(bob), value: U256::from(1_000u64), input: Bytes::new(), gas_limit: 21_000, delegate: None }
        };
        txs.push(sign_call_with(s, CHAIN, 0, fee, (1 + i as u128) * GWEI, &call).unwrap());
    }

    let c = ctx(base());
    let out = execute_block(&pre, &c, &txs).unwrap();
    let st = &out.settlement;
    let exec_gas: u128 = out.receipts.iter().map(|r| r.gas_used as u128).sum();
    let prove_gas: u128 = out.receipts.iter().map(|r| r.prove_gas as u128).sum();
    assert!(prove_gas > 0);
    let tips: u128 = out.receipts.iter().enumerate().map(|(i, r)| r.gas_used as u128 * (i as u128 + 1) * GWEI).sum();
    assert_eq!(st.tips, U256::from(tips));
    assert_eq!(st.prove_fees, U256::from(prove_gas * GWEI));
    assert_eq!(st.to_proposer, U256::from(tips * 60 / 100));
    assert_eq!(out.state.balance(&PROPOSER), st.to_proposer);
    assert_eq!(out.state.balance(&PROVER_ESCROW), st.to_escrow);
    assert_eq!(st.to_escrow, U256::from(tips * 20 / 100 + prove_gas * GWEI));
    assert_eq!(out.state.balance(&FEE_COLLECTOR), U256::ZERO, "collector is swept every block");

    // Conservation: what senders lost = transfers + burned base + burned tips + proposer + escrow.
    let burned_base = U256::from(exec_gas * 3 * GWEI);
    let everyone = [addrs.clone(), vec![bob, PROPOSER, PROVER_ESCROW, FEE_COLLECTOR]].concat();
    assert_eq!(total(&pre, &everyone), total(&out.state, &everyone) + burned_base + st.burned_tips);

    // The BAL names every fee account whose balance moved.
    let touched: Vec<Address> = out.bal.accounts.iter().map(|x| x.address).collect();
    assert!(touched.contains(&PROPOSER) && touched.contains(&PROVER_ESCROW));

    // Builder, validator, parallel and sequential agree.
    let (included, built) = build_block(&pre, &c, txs.clone());
    assert_eq!(included.len(), txs.len());
    assert_eq!(built.state.root(), out.state.root());
    assert_eq!(build_block_sequential(&pre, &c, txs.clone()).1.state.root(), out.state.root());
    assert_eq!(execute_block_sequential(&pre, &c, &txs).unwrap().state.root(), out.state.root());
}

#[test]
fn underpriced_txs_are_invalid() {
    let s = P256Signer::from_seed(&seed(9)).unwrap();
    let a = aether_crypto::address_of(&s.public_key()).unwrap();
    let mut pre = WorldState::default();
    pre.set_balance(a, U256::from(10u128.pow(21))).unwrap();
    let call = EvmCall { to: Some(Address::repeat_byte(1)), value: U256::from(1u64), input: Bytes::new(), gas_limit: 21_000, delegate: None };
    let c = ctx(base());
    // Exec cap below the base fee.
    let low = sign_call(&s, CHAIN, 0, 2 * GWEI, &call).unwrap();
    assert!(execute_block(&pre, &c, &[low.clone()]).is_err());
    assert!(build_block(&pre, &c, vec![low]).0.is_empty());
    // Prove cap below the base prove fee.
    let low_prove = sign_call_with(&s, CHAIN, 0, FeeVector { exec: 5 * GWEI, state: 0, prove: GWEI - 1 }, 2 * GWEI, &call).unwrap();
    assert!(execute_block(&pre, &c, &[low_prove]).is_err());
    // Balance cannot cover value + max exec fee + max prove fee.
    let mut poor = WorldState::default();
    poor.set_balance(a, U256::from(21_000u128 * 5 * GWEI)).unwrap();
    let ok_price = sign_call(&s, CHAIN, 0, 5 * GWEI, &call).unwrap();
    assert!(execute_block(&poor, &c, &[ok_price.clone()]).is_err());
    assert!(execute_block(&pre, &c, &[ok_price]).is_ok());
}

#[test]
fn self_paid_tips_cost_the_proposer() {
    // The proposer tipping itself gets back only 60%: fake activity burns 20% + base.
    let s = P256Signer::from_seed(&seed(7)).unwrap();
    let a = aether_crypto::address_of(&s.public_key()).unwrap();
    let mut pre = WorldState::default();
    pre.set_balance(a, U256::from(10u128.pow(21))).unwrap();
    let mut c = ctx(base());
    c.fees = Some(FeePolicy { base: base(), proposer: a });
    let tx = sign_call(&s, CHAIN, 0, 103 * GWEI, &EvmCall { to: Some(a), value: U256::ZERO, input: Bytes::new(), gas_limit: 21_000, delegate: None }).unwrap();
    let out = execute_block(&pre, &c, &[tx]).unwrap();
    assert!(out.state.balance(&a) < pre.balance(&a));
    let lost = pre.balance(&a) - out.state.balance(&a);
    assert_eq!(lost, U256::from(21_000u128 * 3 * GWEI + 21_000u128 * 100 * GWEI * 40 / 100));
}
