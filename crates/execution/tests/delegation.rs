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
        fees: None,
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

// ---------------- recovery: k-of-n guardians, delay, cancel, owner keys ----------------

use aether_execution::account::{
    encode_add_owner, encode_cancel_recovery, encode_execute_recovery, encode_owner_execute, encode_propose_recovery, encode_set_guardian,
    encode_set_guardians, owner_message, recovery_message, AccountCall, MIN_DELAY,
};

fn sign_rs(signer: &P256Signer, msg: &[u8]) -> ([u8; 32], [u8; 32]) {
    let sig = signer.sign(msg).unwrap();
    (sig[..32].try_into().unwrap(), sig[32..64].try_into().unwrap())
}

fn key(signer: &P256Signer) -> ([u8; 32], [u8; 32]) {
    aether_crypto::p256_xy(&signer.public_key().bytes).unwrap()
}

fn at(number: u64, timestamp: u64) -> BlockContext {
    BlockContext { timestamp, ..ctx(number) }
}

/// Alice's account, delegated, with `guardians` set; plus a funded relayer.
struct Recovery {
    s: Setup,
    state: WorldState,
    relayer: P256Signer,
    relayer_nonce: std::cell::Cell<u64>,
}

impl Recovery {
    fn new(guardians: &[&P256Signer], threshold: u8, delay: u64) -> Self {
        let s = setup();
        let keys: Vec<_> = guardians.iter().map(|g| key(g)).collect();
        let t = tx(
            &s,
            0,
            EvmCall {
                to: Some(s.a),
                value: U256::ZERO,
                input: encode_execute(&[(s.a, U256::ZERO, encode_set_guardians(&keys, threshold, delay))]),
                gas_limit: 500_000,
                delegate: Some(AETHER_ACCOUNT),
            },
        );
        let out = execute_block(&s.pre, &at(1, 1_000), &[t]).unwrap();
        assert!(out.receipts[0].success, "setGuardians: {:?}", out.receipts[0]);
        let mut state = out.state;
        let relayer = P256Signer::from_seed(&seed(9)).unwrap();
        state.set_balance(aether_crypto::address_of(&relayer.public_key()).unwrap(), U256::from(10u128.pow(20))).unwrap();
        Recovery { s, state, relayer, relayer_nonce: std::cell::Cell::new(0) }
    }

    /// A relayed call to Alice's account at `timestamp`; applies it and returns success.
    fn relay(&mut self, input: Bytes, timestamp: u64) -> bool {
        let n = self.relayer_nonce.get();
        self.relayer_nonce.set(n + 1);
        let t = sign_call(&self.relayer, CHAIN, n, 1, &EvmCall { to: Some(self.s.a), value: U256::ZERO, input, gas_limit: 700_000, delegate: None }).unwrap();
        let out = execute_block(&self.state, &at(2, timestamp), &[t]).unwrap();
        self.state = out.state;
        out.receipts[0].success
    }

    /// Alice herself (main key) runs a self-call through `execute`.
    fn owner_self_call(&mut self, nonce: u64, data: Bytes, timestamp: u64) -> bool {
        let t = tx(
            &self.s,
            nonce,
            EvmCall { to: Some(self.s.a), value: U256::ZERO, input: encode_execute(&[(self.s.a, U256::ZERO, data)]), gas_limit: 500_000, delegate: None },
        );
        let out = execute_block(&self.state, &at(3, timestamp), &[t]).unwrap();
        self.state = out.state;
        out.receipts[0].success
    }

    fn sigs(&self, who: &[(u8, &P256Signer)], nonce: u64, calls: &[AccountCall]) -> Vec<(u8, [u8; 32], [u8; 32])> {
        who.iter()
            .map(|(i, g)| {
                let (r, s) = sign_rs(g, &recovery_message(CHAIN, self.s.a, nonce, calls));
                (*i, r, s)
            })
            .collect()
    }
}

#[test]
fn two_of_three_guardians_recover_after_the_delay_and_a_new_device_takes_over() {
    let (phone, ipad, friend) = (P256Signer::from_seed(&seed(2)).unwrap(), P256Signer::from_seed(&seed(3)).unwrap(), P256Signer::from_seed(&seed(4)).unwrap());
    let mut rc = Recovery::new(&[&phone, &ipad, &friend], 2, MIN_DELAY);
    // Alice lost her Mac. Her new Mac's Secure Enclave key becomes an owner of the same address.
    let new_mac = P256Signer::from_seed(&seed(5)).unwrap();
    let (nx, ny) = key(&new_mac);
    let calls: Vec<AccountCall> = vec![(rc.s.a, U256::ZERO, encode_add_owner(nx, ny))];

    let one = rc.sigs(&[(0, &phone)], 0, &calls);
    assert!(!rc.relay(encode_propose_recovery(&calls, &one), 2_000), "one of three is below the threshold");
    let two = rc.sigs(&[(2, &friend), (0, &phone)], 0, &calls);
    assert!(rc.relay(encode_propose_recovery(&calls, &two), 2_000), "two guardians propose");

    assert!(!rc.relay(encode_execute_recovery(&calls), 2_000 + MIN_DELAY - 1), "not before the delay");
    assert!(rc.relay(encode_execute_recovery(&calls), 2_000 + MIN_DELAY), "anyone runs it after the delay");
    assert!(!rc.relay(encode_execute_recovery(&calls), 2_000 + MIN_DELAY + 1), "runs once");

    // The new Mac now drives the account by signature (relayed; it holds no gas).
    let bob = Address::repeat_byte(0xb0);
    let pay: Vec<AccountCall> = vec![(bob, U256::from(10u128.pow(18)), Bytes::new())];
    let (r, s) = sign_rs(&new_mac, &owner_message(CHAIN, rc.s.a, 0, &pay));
    assert!(rc.relay(encode_owner_execute(&pay, 0, r, s), 3_000_000));
    assert_eq!(rc.state.balance(&bob), U256::from(10u128.pow(18)));
    assert!(!rc.relay(encode_owner_execute(&pay, 0, r, s), 3_000_001), "owner signatures do not replay");
    let (tr, ts) = sign_rs(&phone, &owner_message(CHAIN, rc.s.a, 1, &pay));
    assert!(!rc.relay(encode_owner_execute(&pay, 0, tr, ts), 3_000_002), "a guardian is not an owner");
}

#[test]
fn the_owner_cancels_a_recovery_it_did_not_ask_for() {
    let (phone, friend) = (P256Signer::from_seed(&seed(2)).unwrap(), P256Signer::from_seed(&seed(4)).unwrap());
    let mut rc = Recovery::new(&[&phone, &friend], 2, MIN_DELAY);
    // Colluding guardians try to sweep the account.
    let thief = Address::repeat_byte(0x7e);
    let sweep: Vec<AccountCall> = vec![(thief, U256::from(10u128.pow(20)), Bytes::new())];
    let sigs = rc.sigs(&[(0, &phone), (1, &friend)], 0, &sweep);
    assert!(rc.relay(encode_propose_recovery(&sweep, &sigs), 5_000));
    assert!(!rc.relay(encode_propose_recovery(&sweep, &sigs), 5_001), "one pending proposal at a time");
    // Alice still has her key and sees the pending recovery: she cancels within the delay.
    // Nonce 2: the delegating tx used 0 and its authorization 1.
    assert!(rc.owner_self_call(2, encode_cancel_recovery(), 5_100));
    assert!(!rc.relay(encode_execute_recovery(&sweep), 5_000 + MIN_DELAY), "cancelled recovery cannot run");
    assert_eq!(rc.state.balance(&thief), U256::ZERO);
    // The old signatures are spent (proposal nonce advanced).
    assert!(!rc.relay(encode_propose_recovery(&sweep, &sigs), 5_200), "no replay of a cancelled proposal");
}

#[test]
fn guardian_signatures_must_be_distinct_valid_and_cover_the_calls() {
    let (phone, friend, stranger) =
        (P256Signer::from_seed(&seed(2)).unwrap(), P256Signer::from_seed(&seed(4)).unwrap(), P256Signer::from_seed(&seed(6)).unwrap());
    let mut rc = Recovery::new(&[&phone, &friend], 2, MIN_DELAY);
    let calls: Vec<AccountCall> = vec![(Address::repeat_byte(0x5a), U256::from(1u64), Bytes::new())];
    let twice = rc.sigs(&[(0, &phone), (0, &phone)], 0, &calls);
    assert!(!rc.relay(encode_propose_recovery(&calls, &twice), 1_100), "the same guardian twice");
    let outsider = rc.sigs(&[(0, &phone), (1, &stranger)], 0, &calls);
    assert!(!rc.relay(encode_propose_recovery(&calls, &outsider), 1_100), "a key that is not a guardian");
    let other: Vec<AccountCall> = vec![(Address::repeat_byte(0x7e), U256::from(1u64), Bytes::new())];
    let for_other = rc.sigs(&[(0, &phone), (1, &friend)], 0, &other);
    assert!(!rc.relay(encode_propose_recovery(&calls, &for_other), 1_100), "signatures over different calls");
    let good = rc.sigs(&[(0, &phone), (1, &friend)], 0, &calls);
    assert!(rc.relay(encode_propose_recovery(&calls, &good), 1_100));
    assert!(!rc.relay(encode_execute_recovery(&other), 1_100 + MIN_DELAY), "only the proposed calls run");
}

#[test]
fn guardian_settings_are_validated() {
    let phone = P256Signer::from_seed(&seed(2)).unwrap();
    let s = setup();
    let set = |keys: &[([u8; 32], [u8; 32])], threshold: u8, delay: u64| {
        let t = tx(
            &s,
            0,
            EvmCall {
                to: Some(s.a),
                value: U256::ZERO,
                input: encode_execute(&[(s.a, U256::ZERO, encode_set_guardians(keys, threshold, delay))]),
                gas_limit: 500_000,
                delegate: Some(AETHER_ACCOUNT),
            },
        );
        execute_block(&s.pre, &at(1, 1), &[t]).unwrap().receipts[0].success
    };
    let k = key(&phone);
    assert!(set(&[k], 1, MIN_DELAY));
    assert!(!set(&[k], 1, MIN_DELAY - 1), "delay below the minimum");
    assert!(!set(&[k], 2, MIN_DELAY), "threshold above the guardian count");
    assert!(!set(&[k], 0, MIN_DELAY), "zero threshold with guardians");
    assert!(!set(&[k, k], 1, MIN_DELAY), "duplicate guardian");
    assert!(set(&[], 0, 0), "an empty list turns recovery off");
}

#[test]
fn one_recovery_device_uses_the_48_hour_default() {
    let phone = P256Signer::from_seed(&seed(2)).unwrap();
    let s = setup();
    let (gx, gy) = key(&phone);
    let t = tx(
        &s,
        0,
        EvmCall {
            to: Some(s.a),
            value: U256::ZERO,
            input: encode_execute(&[(s.a, U256::ZERO, encode_set_guardian(gx, gy))]),
            gas_limit: 500_000,
            delegate: Some(AETHER_ACCOUNT),
        },
    );
    let out = execute_block(&s.pre, &at(1, 1_000), &[t]).unwrap();
    assert!(out.receipts[0].success, "{:?}", out.receipts[0]);
    let mut rc = Recovery { state: out.state, relayer: P256Signer::from_seed(&seed(9)).unwrap(), relayer_nonce: std::cell::Cell::new(0), s };
    let r_addr = aether_crypto::address_of(&rc.relayer.public_key()).unwrap();
    rc.state.set_balance(r_addr, U256::from(10u128.pow(20))).unwrap();
    let safe = Address::repeat_byte(0x5a);
    let calls: Vec<AccountCall> = vec![(safe, U256::from(10u128.pow(20)), Bytes::new())];
    let sigs = rc.sigs(&[(0, &phone)], 0, &calls);
    assert!(rc.relay(encode_propose_recovery(&calls, &sigs), 10_000));
    assert!(!rc.relay(encode_execute_recovery(&calls), 10_000 + 47 * 3600), "47 h is too early");
    assert!(rc.relay(encode_execute_recovery(&calls), 10_000 + 48 * 3600));
    assert_eq!(rc.state.balance(&safe), U256::from(10u128.pow(20)));
}

#[test]
fn no_guardian_means_no_recovery_path() {
    let s = setup();
    let st = execute_block(&s.pre, &ctx(1), &[tx(&s, 0, batch(s.a, Some(AETHER_ACCOUNT), &[]))]).unwrap().state;
    let mut rc = Recovery { s, state: st, relayer: P256Signer::from_seed(&seed(9)).unwrap(), relayer_nonce: std::cell::Cell::new(0) };
    let r_addr = aether_crypto::address_of(&rc.relayer.public_key()).unwrap();
    rc.state.set_balance(r_addr, U256::from(10u128.pow(20))).unwrap();
    let phone = P256Signer::from_seed(&seed(2)).unwrap();
    let calls: Vec<AccountCall> = vec![(Address::repeat_byte(0x5a), U256::from(1u64), Bytes::new())];
    let sigs = rc.sigs(&[(0, &phone)], 0, &calls);
    assert!(!rc.relay(encode_propose_recovery(&calls, &sigs), 100));
}

// ---------------- session keys (limits enforced on chain) ----------------

use aether_execution::account::{encode_add_session, encode_session_execute, session_message, slots, SessionLimits};

const AETH: u128 = 1_000_000_000_000_000_000;

/// Alice's account with one session key (`agent`) under `limits`, added at t=1000.
fn with_session(agent: &P256Signer, limits: &SessionLimits) -> Recovery {
    let mut rc = Recovery::new(&[], 0, 0);
    let (x, y) = key(agent);
    // Nonce 2: the delegating tx used 0 and its authorization 1.
    assert!(rc.owner_self_call(2, encode_add_session(x, y, limits), 1_000), "addSession");
    rc
}

fn pay(rc: &mut Recovery, agent: &P256Signer, nonce: u64, calls: &[AccountCall], t: u64) -> bool {
    let (r, s) = sign_rs(agent, &session_message(CHAIN, rc.s.a, 0, nonce, calls));
    rc.relay(encode_session_execute(calls, 0, r, s), t)
}

fn to(addr: Address, aeth: u128) -> AccountCall {
    (addr, U256::from(aeth), Bytes::new())
}

#[test]
fn a_session_key_pays_within_its_limits_only() {
    let agent = P256Signer::from_seed(&seed(20)).unwrap();
    let limits = SessionLimits { per_payment: AETH, per_day: 3 * AETH, expires: 0, allow: vec![] };
    let mut rc = with_session(&agent, &limits);
    let bob = Address::repeat_byte(0xb0);
    assert!(pay(&mut rc, &agent, 0, &[to(bob, AETH)], 2_000));
    assert!(!pay(&mut rc, &agent, 0, &[to(bob, AETH)], 2_001), "no replay");
    assert!(!pay(&mut rc, &agent, 1, &[to(bob, AETH / 2 + 1), to(bob, AETH / 2)], 2_002), "per-payment limit counts the whole batch");
    assert!(pay(&mut rc, &agent, 1, &[to(bob, AETH)], 2_003));
    assert!(pay(&mut rc, &agent, 2, &[to(bob, AETH)], 2_004));
    assert!(!pay(&mut rc, &agent, 3, &[to(bob, 1)], 2_005), "daily limit reached");
    assert!(pay(&mut rc, &agent, 3, &[to(bob, AETH)], 2_000 + 86_400), "a new 24 h window");
    assert_eq!(rc.state.balance(&bob), U256::from(4 * AETH));
    // The limits and usage are readable from storage (for light-client proofs).
    let base = slots::session(0);
    assert_eq!(slots::unpack_limits(rc.state.storage(&rc.s.a, base + U256::from(2u64))), (AETH, 3 * AETH));
    let (window, spent, expires) = slots::unpack_window(rc.state.storage(&rc.s.a, base + U256::from(3u64)));
    assert_eq!((window, spent, expires), (2_000 + 86_400, AETH, 0));
    assert_eq!(rc.state.storage(&rc.s.a, base + U256::from(5u64)), U256::from(4u64), "session nonce");
    assert_eq!(rc.state.storage(&rc.s.a, slots::session_count()), U256::from(1u64));
}

#[test]
fn a_session_key_cannot_call_contracts_or_change_settings() {
    let agent = P256Signer::from_seed(&seed(21)).unwrap();
    let mut rc = with_session(&agent, &SessionLimits { per_payment: AETH, per_day: 10 * AETH, expires: 0, allow: vec![] });
    let (x, y) = key(&agent);
    let raise = SessionLimits { per_payment: 1_000 * AETH, per_day: 1_000 * AETH, expires: 0, allow: vec![] };
    let a = rc.s.a;
    assert!(!pay(&mut rc, &agent, 0, &[(a, U256::ZERO, encode_add_session(x, y, &raise))], 2_000), "no self-calls");
    assert!(!pay(&mut rc, &agent, 0, &[(Address::repeat_byte(0xc0), U256::from(1u64), Bytes::from_static(&[1, 2, 3, 4]))], 2_001), "no contract calls");
    let other = P256Signer::from_seed(&seed(22)).unwrap();
    assert!(!pay(&mut rc, &other, 0, &[to(Address::repeat_byte(0xb0), 1)], 2_002), "another key");
}

#[test]
fn session_allowlist_and_expiry_hold() {
    let agent = P256Signer::from_seed(&seed(23)).unwrap();
    let shop = Address::repeat_byte(0x5b);
    let mut rc = with_session(&agent, &SessionLimits { per_payment: AETH, per_day: 10 * AETH, expires: 50_000, allow: vec![shop] });
    assert!(!pay(&mut rc, &agent, 0, &[to(Address::repeat_byte(0xb0), 1)], 2_000), "recipient not allowed");
    assert!(pay(&mut rc, &agent, 0, &[to(shop, 1)], 2_001));
    assert!(!pay(&mut rc, &agent, 1, &[to(shop, 1)], 50_000), "expired");
}

#[test]
fn session_settings_are_validated() {
    let agent = P256Signer::from_seed(&seed(24)).unwrap();
    let (x, y) = key(&agent);
    let mut rc = Recovery::new(&[], 0, 0);
    let bad = SessionLimits { per_payment: 2 * AETH, per_day: AETH, expires: 0, allow: vec![] };
    assert!(!rc.owner_self_call(2, encode_add_session(x, y, &bad), 1_000), "per payment above per day");
    let zero = SessionLimits { per_payment: 0, per_day: AETH, expires: 0, allow: vec![] };
    assert!(!rc.owner_self_call(3, encode_add_session(x, y, &zero), 1_000), "zero per payment (the reverted tx used nonce 2)");
}
