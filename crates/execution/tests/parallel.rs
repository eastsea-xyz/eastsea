//! Differential tests: speculative parallel execution must equal sequential
//! execution exactly (state root, BAL, receipts, gas, included txs), including
//! conflicts, contract storage contention and txs that touch the fee recipient.

use aether_crypto::{Ed25519Signer, P256Signer, Signer};
use aether_execution::{
    build_block, build_block_sequential, execute_block, execute_block_sequential, sign_call, BlockContext, BlockOutcome, EvmCall, WorldState,
};
use aether_types::{Address, Bytes, GasVector, TxEnvelope, U256};
use proptest::prelude::*;

const CHAIN: u64 = 7_777;
/// Runtime: slot0 += 1.
const COUNTER_INIT: &[u8] =
    &[0x60, 0x0a, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x0a, 0x60, 0x00, 0xf3, 0x60, 0x00, 0x54, 0x60, 0x01, 0x01, 0x60, 0x00, 0x55, 0x00];
/// Runtime: slot0 = BALANCE(COINBASE). Reads the fee recipient mid-block.
const COINBASE_PEEK_INIT: &[u8] = &[0x60, 0x06, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, 0x06, 0x60, 0x00, 0xf3, 0x41, 0x31, 0x60, 0x00, 0x55, 0x00];

fn seed(b: u8) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[0] = 0x5e;
    s[31] = b;
    s
}

struct World {
    users: Vec<P256Signer>,
    proposer: Ed25519Signer,
    ctx: BlockContext,
    pre: WorldState,
}

fn addr(s: &dyn Signer) -> Address {
    aether_crypto::address_of(&s.public_key()).unwrap()
}

fn world(n_users: u8) -> World {
    let users: Vec<P256Signer> = (1..=n_users).map(|i| P256Signer::from_seed(&seed(i)).unwrap()).collect();
    let proposer = Ed25519Signer::from_seed(&seed(200));
    let mut pre = WorldState::default();
    for u in &users {
        pre.set_balance(addr(u), U256::from(10u128.pow(22))).unwrap();
    }
    pre.set_balance(addr(&proposer), U256::from(10u128.pow(20))).unwrap();
    pre.set_code(aether_execution::AETHER_ACCOUNT, aether_execution::aether_account_code()).unwrap();
    let ctx = BlockContext {
        chain_id: CHAIN,
        number: 1,
        timestamp: 1_700_000_001,
        beneficiary: addr(&proposer),
        limits: GasVector { exec: 30_000_000, state: u64::MAX, prove: 200_000_000 },
    };
    World { users, proposer, ctx, pre }
}

fn call(to: Option<Address>, value: u64, input: &[u8], gas: u64) -> EvmCall {
    EvmCall { to, value: U256::from(value), input: Bytes::copy_from_slice(input), gas_limit: gas, delegate: None }
}

fn assert_same(a: &BlockOutcome, b: &BlockOutcome) {
    assert_eq!(a.state.root(), b.state.root(), "state root");
    assert_eq!(a.bal, b.bal, "BAL");
    assert_eq!(a.receipts, b.receipts, "receipts");
    assert_eq!(a.gas, b.gas, "gas");
}

/// Deploy a counter and a coinbase-peeker in a setup block; return post-state and addresses.
fn with_contracts(w: &World) -> (WorldState, Address, Address) {
    let deploy = |nonce, code: &[u8]| sign_call(&w.users[0], CHAIN, nonce, 1, &call(None, 0, code, 3_000_000)).unwrap();
    let out = execute_block(&w.pre, &w.ctx, &[deploy(0, COUNTER_INIT), deploy(1, COINBASE_PEEK_INIT)]).unwrap();
    let c = out.receipts[0].contract_address.unwrap();
    let p = out.receipts[1].contract_address.unwrap();
    (out.state, c, p)
}

#[derive(Debug, Clone)]
enum Op {
    Transfer {
        from: usize,
        to: usize,
        value: u64,
    },
    ToProposer {
        from: usize,
        value: u64,
    },
    Counter {
        from: usize,
    },
    Peek {
        from: usize,
    },
    FromProposer {
        to: usize,
        value: u64,
    },
    BadNonce {
        from: usize,
    },
    /// Delegate to AetherAccount (first time) and batch-pay two targets.
    Batch {
        from: usize,
        a: usize,
        b: usize,
    },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (0..6usize, 0..9usize, 1..1_000u64).prop_map(|(from, to, value)| Op::Transfer { from, to, value }),
        1 => (0..6usize, 1..1_000u64).prop_map(|(from, value)| Op::ToProposer { from, value }),
        3 => (0..6usize).prop_map(|from| Op::Counter { from }),
        1 => (0..6usize).prop_map(|from| Op::Peek { from }),
        1 => (0..9usize, 1..1_000u64).prop_map(|(to, value)| Op::FromProposer { to, value }),
        1 => (0..6usize).prop_map(|from| Op::BadNonce { from }),
        2 => (0..6usize, 0..9usize, 0..9usize).prop_map(|(from, a, b)| Op::Batch { from, a, b }),
    ]
}

/// Turn ops into correctly-nonced signed txs (BadNonce skips ahead on purpose).
fn txs(w: &World, pre: &WorldState, counter: Address, peek: Address, ops: &[Op]) -> Vec<TxEnvelope> {
    let mut nonces: Vec<u64> = w.users.iter().map(|u| pre.nonce(&addr(u))).collect();
    let mut pn = pre.nonce(&addr(&w.proposer));
    let target = |i: usize| if i < w.users.len() { addr(&w.users[i]) } else { Address::repeat_byte(0xa0 + i as u8) };
    let mut out = Vec::new();
    for o in ops {
        let (signer, nonce, c): (&dyn Signer, &mut u64, EvmCall) = match o {
            Op::Transfer { from, to, value } => (&w.users[*from], &mut nonces[*from], call(Some(target(*to)), *value, &[], 21_000)),
            Op::ToProposer { from, value } => (&w.users[*from], &mut nonces[*from], call(Some(w.ctx.beneficiary), *value, &[], 21_000)),
            Op::Counter { from } => (&w.users[*from], &mut nonces[*from], call(Some(counter), 0, &[], 100_000)),
            Op::Peek { from } => (&w.users[*from], &mut nonces[*from], call(Some(peek), 0, &[], 100_000)),
            Op::FromProposer { to, value } => (&w.proposer, &mut pn, call(Some(target(*to)), *value, &[], 21_000)),
            Op::Batch { from, a, b } => {
                let me = addr(&w.users[*from]);
                let calls = vec![(target(*a), U256::from(3u64), Bytes::new()), (target(*b), U256::from(4u64), Bytes::new())];
                let c = EvmCall {
                    to: Some(me),
                    value: U256::ZERO,
                    input: aether_execution::encode_execute(&calls),
                    gas_limit: 300_000,
                    delegate: Some(aether_execution::AETHER_ACCOUNT),
                };
                out.push(sign_call(&w.users[*from], CHAIN, nonces[*from], 1, &c).unwrap());
                // A self-delegation also consumes the authorization nonce.
                nonces[*from] += 2;
                continue;
            }
            Op::BadNonce { from } => {
                let tx = sign_call(&w.users[*from], CHAIN, nonces[*from] + 5, 1, &call(Some(target(0)), 1, &[], 21_000)).unwrap();
                out.push(tx);
                continue;
            }
        };
        out.push(sign_call(signer, CHAIN, *nonce, 1 + (*nonce % 3) as u128, &c).unwrap());
        *nonce += 1;
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn parallel_equals_sequential(ops in prop::collection::vec(op(), 1..80)) {
        let w = world(6);
        let (pre, counter, peek) = with_contracts(&w);
        let candidates = txs(&w, &pre, counter, peek, &ops);

        // Proposer path: same txs kept, same outcome.
        let (inc_p, out_p) = build_block(&pre, &w.ctx, candidates.clone());
        let (inc_s, out_s) = build_block_sequential(&pre, &w.ctx, candidates);
        prop_assert_eq!(&inc_p, &inc_s);
        assert_same(&out_p, &out_s);

        // Validator path on the resulting block.
        let v_p = execute_block(&pre, &w.ctx, &inc_p).unwrap();
        let v_s = execute_block_sequential(&pre, &w.ctx, &inc_s).unwrap();
        assert_same(&v_p, &v_s);
        assert_same(&v_p, &out_p);
    }
}

#[test]
fn contention_and_fee_recipient_reads_match_sequential() {
    let w = world(6);
    let (pre, counter, peek) = with_contracts(&w);
    // Many counter increments (all conflict), interleaved with coinbase peeks
    // that must see the fee recipient's running balance.
    let ops: Vec<Op> = (0..60).map(|i| if i % 5 == 0 { Op::Peek { from: i % 6 } } else { Op::Counter { from: i % 6 } }).collect();
    let block = txs(&w, &pre, counter, peek, &ops);
    let p = execute_block(&pre, &w.ctx, &block).unwrap();
    let s = execute_block_sequential(&pre, &w.ctx, &block).unwrap();
    assert_same(&p, &s);
    assert_eq!(p.state.storage(&counter, U256::ZERO), U256::from(48u64));
    assert_ne!(p.state.storage(&peek, U256::ZERO), U256::ZERO, "peek saw a funded fee recipient");
}

#[test]
fn independent_transfers_parallelize() {
    let w = world(6);
    let block: Vec<TxEnvelope> = (0..600u64)
        .map(|i| {
            let u = (i % 6) as usize;
            sign_call(&w.users[u], CHAIN, i / 6, 1, &call(Some(Address::repeat_byte(0x10 + (i % 200) as u8)), 1, &[], 21_000)).unwrap()
        })
        .collect();
    let p = execute_block(&w.pre, &w.ctx, &block).unwrap();
    let s = execute_block_sequential(&w.pre, &w.ctx, &block).unwrap();
    assert_same(&p, &s);
}

/// Timing, not correctness: `cargo test --release -p aether-execution --test parallel -- --ignored --nocapture`
#[test]
#[ignore = "benchmark"]
fn bench_parallel_vs_sequential() {
    let mut w = world(24);
    w.ctx.limits.exec = 1_000_000_000;
    w.ctx.limits.prove = 1_000_000_000;
    let (pre, counter, _) = with_contracts(&w);
    let transfers: Vec<TxEnvelope> = (0..2400u64)
        .map(|i| {
            let u = (i % 24) as usize;
            sign_call(&w.users[u], CHAIN, pre.nonce(&addr(&w.users[u])) + i / 24, 1, &call(Some(Address::repeat_byte((i % 250) as u8)), 1, &[], 21_000))
                .unwrap()
        })
        .collect();
    let contended: Vec<TxEnvelope> = (0..600u64)
        .map(|i| {
            let u = (i % 24) as usize;
            sign_call(&w.users[u], CHAIN, pre.nonce(&addr(&w.users[u])) + i / 24, 1, &call(Some(counter), 0, &[], 100_000)).unwrap()
        })
        .collect();
    // One tx per sender: no account conflicts at all.
    let many = world(240);
    let distinct: Vec<TxEnvelope> =
        (0..240usize).map(|u| sign_call(&many.users[u], CHAIN, 0, 1, &call(Some(Address::repeat_byte(u as u8)), 1, &[], 21_000)).unwrap()).collect();
    let t = std::time::Instant::now();
    let s = execute_block_sequential(&many.pre, &w.ctx, &distinct).unwrap();
    let ts = t.elapsed();
    let t = std::time::Instant::now();
    let p = execute_block(&many.pre, &w.ctx, &distinct).unwrap();
    let tp = t.elapsed();
    assert_same(&p, &s);
    println!("240 txs from 240 senders: sequential {ts:?}, parallel {tp:?} ({:.2}x)", ts.as_secs_f64() / tp.as_secs_f64());
    let t = std::time::Instant::now();
    for tx in &distinct {
        let _ = aether_execution::can_append(&many.pre, &w.ctx, GasVector::default(), tx);
    }
    println!("  of which signature check + revm (240 runs, no commit): {:?}", t.elapsed());

    for (name, block) in [("2400 independent transfers", &transfers), ("600 calls to one counter", &contended)] {
        let t = std::time::Instant::now();
        let s = execute_block_sequential(&pre, &w.ctx, block).unwrap();
        let ts = t.elapsed();
        let t = std::time::Instant::now();
        let p = execute_block(&pre, &w.ctx, block).unwrap();
        let tp = t.elapsed();
        assert_same(&p, &s);
        println!("{name}: sequential {ts:?}, parallel {tp:?} ({:.2}x)", ts.as_secs_f64() / tp.as_secs_f64());
    }
}
