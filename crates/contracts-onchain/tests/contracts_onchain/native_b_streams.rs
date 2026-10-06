//! B1 streams head-to-head: the toolbox's LinearVesting (one deployment per
//! grant) and TokenTimeLock (shared, beneficiary-keyed) against the native
//! GrantLedger, on the same genesis, token, wallet and cliff-to-end curve
//! (no catch-up at the cliff on all three paths).
use super::harness::*;
use super::native_b_common::*;
use alloy_sol_types::{sol, SolCall, SolValue};
use serde_json::Value;

sol! {
    interface Vesting {
        function claim() returns (uint256);
        function start() returns (uint48);
    }
    interface TimeLock {
        function lockFor(address beneficiary, uint256 amount, uint256 cliffSec, uint256 durationSec);
        function release();
    }
    interface Ledger {
        struct GrantParams { address beneficiary; uint128 total; uint32 start; uint32 cliff; uint32 end; }
        function create(GrantParams p) returns (uint256);
        function createBatch(GrantParams[] ps) returns (uint256);
        function claim(uint256 id) returns (uint256);
        function tripBrake();
    }
}

const EMPLOYER: u8 = 0;
const RELAYER: u8 = 5;
const TOTAL: u64 = 4_000;
const CLIFF: u64 = 100;
const DURATION: u64 = 500;

#[derive(Clone, Copy, PartialEq)]
enum Path {
    LinearVesting,
    TimeLock,
    Ledger,
}

impl Path {
    fn name(self) -> &'static str {
        match self {
            Path::LinearVesting => "original-linearvesting",
            Path::TimeLock => "original-tokentimelock",
            Path::Ledger => "native",
        }
    }
}

struct Run {
    h: Harness,
    path: Path,
    token: Address,
    shared: Address,
    out: Vec<Value>,
}

/// A created grant: where to claim and the schedule's start time.
struct Grant {
    target: Address,
    id: u64,
    start: u64,
}

impl Run {
    fn new(path: Path) -> Self {
        let mut h = Harness::new();
        let token = funded_token(&mut h);
        Self { h, path, token, shared: Address::ZERO, out: Vec::new() }
    }

    fn params(&self, beneficiary: Address) -> Ledger::GrantParams {
        let start = self.h.timestamp() as u32;
        Ledger::GrantParams { beneficiary, total: TOTAL.into(), start, cliff: start + CLIFF as u32, end: start + DURATION as u32 }
    }

    /// Deploy the shared instance (TokenTimeLock, GrantLedger) as setup.
    fn deploy_shared(&mut self) {
        self.shared = match self.path {
            Path::LinearVesting => Address::ZERO,
            Path::TimeLock => deploy(&mut self.h, EMPLOYER, "toolbox/TokenTimeLock", (Address::ZERO, self.token).abi_encode_params(), "setup/deploy-shared-instance"),
            Path::Ledger => deploy(&mut self.h, EMPLOYER, "native/GrantLedger", (self.token, B256::ZERO).abi_encode_params(), "setup/deploy-shared-instance"),
        };
    }

    /// The employer funds one grant with the fewest signatures each path allows.
    fn create(&mut self, beneficiary: Address, label: &str) -> Grant {
        let total = U256::from(TOTAL);
        match self.path {
            Path::LinearVesting => {
                // The constructor pulls from the deployer: approve the predicted
                // CREATE address first (a deployment cannot join a batch).
                let me = self.h.addr(EMPLOYER);
                let predicted = me.create(self.h.nonce(EMPLOYER) + 1);
                self.h.ok(EMPLOYER, self.token, approve(self.token, predicted, total).2.to_vec(), U256::ZERO, &format!("{label}/approve-predicted"));
                let args = (self.token, beneficiary, total, U256::from(CLIFF), U256::from(DURATION)).abi_encode_params();
                let vesting = deploy(&mut self.h, EMPLOYER, "toolbox/LinearVesting", args, &format!("{label}/deploy-per-grant"));
                assert_eq!(vesting, predicted);
                let start = Vesting::startCall::abi_decode_returns(&self.h.view(0, vesting, Vesting::startCall {}.abi_encode())).unwrap();
                Grant { target: vesting, id: 0, start: start.to::<u64>() }
            }
            Path::TimeLock => {
                let start = self.h.timestamp();
                let lock = TimeLock::lockForCall { beneficiary, amount: total, cliffSec: U256::from(CLIFF), durationSec: U256::from(DURATION) };
                let calls = [approve(self.token, self.shared, total), call(self.shared, lock.abi_encode()), approve(self.token, self.shared, U256::ZERO)];
                self.h.self_batch(EMPLOYER, &calls, &format!("{label}/batch-approve-lockFor-approve0"), true);
                Grant { target: self.shared, id: 0, start }
            }
            Path::Ledger => {
                let p = self.params(beneficiary);
                let id = self.next_id();
                let calls = [approve(self.token, self.shared, total), call(self.shared, Ledger::createCall { p: p.clone() }.abi_encode()), approve(self.token, self.shared, U256::ZERO)];
                self.h.self_batch(EMPLOYER, &calls, &format!("{label}/batch-approve-create-approve0"), true);
                Grant { target: self.shared, id, start: u64::from(p.start) }
            }
        }
    }

    fn next_id(&self) -> u64 {
        let out = self.h.view(0, self.shared, alloy_primitives::keccak256("nextId()")[..4].to_vec());
        U256::from_be_slice(&out).to::<u64>()
    }

    fn claim_input(&self, g: &Grant) -> Vec<u8> {
        match self.path {
            Path::LinearVesting => Vesting::claimCall {}.abi_encode(),
            Path::TimeLock => TimeLock::releaseCall {}.abi_encode(),
            Path::Ledger => Ledger::claimCall { id: U256::from(g.id) }.abi_encode(),
        }
    }

    /// Cliff plus four claims (three partial, one final) by the beneficiary.
    fn lifecycle(&mut self, beneficiary: u8, g: &Grant, before_cliff: bool) {
        let to = self.h.addr(beneficiary);
        let before = balance(&self.h, self.token, to);
        if before_cliff {
            self.h.at(g.start + CLIFF - 10);
            let input = self.claim_input(g);
            // LinearVesting pays zero and succeeds; the others refuse.
            let ok = self.path == Path::LinearVesting;
            self.h.transact(beneficiary, aether_execution::EvmCall { to: Some(g.target), value: U256::ZERO, input: input.into(), gas_limit: TX_GAS, delegate: None }, "claim/before-cliff", ok);
        }
        for (k, label) in [(1, "claim/1-partial"), (2, "claim/2-partial"), (3, "claim/3-partial"), (4, "claim/4-final")] {
            self.h.at(g.start + CLIFF + k * (DURATION - CLIFF) / 4);
            let input = self.claim_input(g);
            self.h.ok(beneficiary, g.target, input, U256::ZERO, label);
        }
        assert_eq!(balance(&self.h, self.token, to) - before, U256::from(TOTAL));
    }
}

fn run(path: Path) -> Run {
    let mut r = Run::new(path);
    let p = path.name();

    // Cold: wallet delegation and any shared deployment are setup; the grant,
    // a premature claim and four claims to a fresh token holder are the action.
    begin(&mut r.h, &format!("b1/{p}/grant-lifecycle-cold"), &[EMPLOYER, 10], &[EMPLOYER]);
    r.deploy_shared();
    r.h.phase("action");
    let to = r.h.addr(10);
    let g = r.create(to, "create");
    r.lifecycle(10, &g, true);
    r.out.push(finish(&mut r.h).1);

    // A shared instance normally holds other grants: keep one unclaimed grant
    // (not recorded) so its token balance slot stays occupied, as it would
    // in use. Without it the drained balance is re-occupied for 100 u.
    if path != Path::LinearVesting {
        r.create(Address::repeat_byte(0xee), "background");
    }

    // Warm: the same grant to a warm holder on an existing instance.
    begin(&mut r.h, &format!("b1/{p}/grant-lifecycle-warm"), &[EMPLOYER, 1], &[]);
    r.h.phase("action");
    let to = r.h.addr(1);
    let g = r.create(to, "create");
    r.lifecycle(1, &g, false);
    r.out.push(finish(&mut r.h).1);

    // Eight grants at once with the cheapest batching each path allows.
    begin(&mut r.h, &format!("b1/{p}/create-8"), &[EMPLOYER], &[]);
    r.h.phase("action");
    let beneficiaries: Vec<Address> = (0..8u8).map(|i| Address::repeat_byte(0xb0 + i)).collect();
    match path {
        Path::LinearVesting => {
            let me = r.h.addr(EMPLOYER);
            let n = r.h.nonce(EMPLOYER);
            let approvals: Vec<_> = (1..=8).map(|k| approve(r.token, me.create(n + k), U256::from(TOTAL))).collect();
            r.h.self_batch(EMPLOYER, &approvals, "create-8/batch-approve-8-predicted", true);
            for (k, b) in beneficiaries.iter().enumerate() {
                let args = (r.token, *b, U256::from(TOTAL), U256::from(CLIFF), U256::from(DURATION)).abi_encode_params();
                deploy(&mut r.h, EMPLOYER, "toolbox/LinearVesting", args, &format!("create-8/deploy-{}", k + 1));
            }
        }
        Path::TimeLock => {
            let sum = U256::from(TOTAL * 8);
            let mut calls = vec![approve(r.token, r.shared, sum)];
            calls.extend(beneficiaries.iter().map(|b| call(r.shared, TimeLock::lockForCall { beneficiary: *b, amount: U256::from(TOTAL), cliffSec: U256::from(CLIFF), durationSec: U256::from(DURATION) }.abi_encode())));
            calls.push(approve(r.token, r.shared, U256::ZERO));
            r.h.self_batch(EMPLOYER, &calls, "create-8/batch-approve-8xlockFor-approve0", true);
        }
        Path::Ledger => {
            let sum = U256::from(TOTAL * 8);
            let ps: Vec<_> = beneficiaries.iter().map(|b| r.params(*b)).collect();
            let calls = [approve(r.token, r.shared, sum), call(r.shared, Ledger::createBatchCall { ps: ps.clone() }.abi_encode()), approve(r.token, r.shared, U256::ZERO)];
            r.h.self_batch(EMPLOYER, &calls, "create-8/batch-approve-createBatch8-approve0", true);
            let mut singles = vec![approve(r.token, r.shared, sum)];
            singles.extend(ps.iter().map(|p| call(r.shared, Ledger::createCall { p: p.clone() }.abi_encode())));
            singles.push(approve(r.token, r.shared, U256::ZERO));
            r.h.self_batch(EMPLOYER, &singles, "create-8/batch-approve-8xcreate-approve0", true);
        }
    }
    r.out.push(finish(&mut r.h).1);

    // Relayed claims, rejected calls and a deficit.
    begin(&mut r.h, &format!("b1/{p}/relayed-rejected-deficit"), &[EMPLOYER, 2, 3], &[]);
    r.h.phase("action");
    let g = r.create(r.h.addr(2), "create");
    r.h.at(g.start + CLIFF + 100);
    let input = r.claim_input(&g);
    match path {
        // Anyone may submit; the money still goes to the beneficiary.
        Path::LinearVesting | Path::Ledger => {
            r.h.ok(RELAYER, g.target, input.clone(), U256::ZERO, "claim/relayed");
        }
        // release() pays msg.sender's own lock: a relayer has nothing to release.
        Path::TimeLock => {
            r.h.revert(RELAYER, g.target, input.clone(), U256::ZERO, "claim/relayed-unsupported");
        }
    }
    // Premature claims are measured in the cold lifecycle ("claim/before-cliff");
    // after the cliff every second accrues, so no later call is empty.
    match path {
        Path::Ledger => {
            let unknown = Ledger::claimCall { id: U256::from(999) }.abi_encode();
            r.h.revert(3, r.shared, unknown, U256::ZERO, "reject/unknown-grant");
            let trip = Ledger::tripBrakeCall {}.abi_encode();
            r.h.revert(3, r.shared, trip, U256::ZERO, "reject/trip-brake-predicate-false");
        }
        Path::TimeLock => {
            let lock = TimeLock::lockForCall { beneficiary: r.h.addr(2), amount: U256::from(TOTAL), cliffSec: U256::from(CLIFF), durationSec: U256::from(DURATION) };
            let calls = [approve(r.token, r.shared, U256::from(TOTAL)), call(r.shared, lock.abi_encode()), approve(r.token, r.shared, U256::ZERO)];
            r.h.self_batch(EMPLOYER, &calls, "reject/second-concurrent-grant-same-beneficiary", false);
        }
        Path::LinearVesting => {}
    }
    // Deficit: the issuer seizes part of the custody balance.
    let custody = if path == Path::LinearVesting { g.target } else { r.shared };
    seize(&mut r.h, r.token, custody, U256::from(TOTAL / 2));
    if path == Path::Ledger {
        r.h.ok(RELAYER, r.shared, Ledger::tripBrakeCall {}.abi_encode(), U256::ZERO, "brake/trip-latch");
    }
    r.h.at(g.start + DURATION);
    if path == Path::TimeLock {
        // No solvency check: the payout consumes other grants' backing.
        r.h.ok(2, g.target, input, U256::ZERO, "claim/paid-from-other-grants-backing");
    } else {
        r.h.revert(2, g.target, input, U256::ZERO, "claim/blocked-by-deficit");
    }
    r.out.push(finish(&mut r.h).1);
    r
}

fn slots(records: &[Value], workflow: &str, label: &str) -> u64 {
    records
        .iter()
        .find(|r| r["workflow"].as_str().unwrap().ends_with(workflow))
        .and_then(|r| r["steps"].as_array().unwrap().iter().find(|s| s["label"] == label))
        .unwrap_or_else(|| panic!("{workflow} {label}"))["units"]["new_slots"]
        .as_u64()
        .unwrap()
}

#[test]
fn b1_streams_originals_vs_native_on_the_executor() {
    let lv = run(Path::LinearVesting);
    let tl = run(Path::TimeLock);
    let gl = run(Path::Ledger);
    // Native create: two grant words, allowance back to zero in the batch.
    assert_eq!(slots(&gl.out, "native/grant-lifecycle-warm", "create/batch-approve-create-approve0"), 2);
    assert_eq!(slots(&tl.out, "tokentimelock/grant-lifecycle-warm", "create/batch-approve-lockFor-approve0"), 2);
    // LinearVesting: the allowance slot occupied before the deploy is paid.
    assert_eq!(slots(&lv.out, "linearvesting/grant-lifecycle-warm", "create/approve-predicted"), 1);
    // First claim to a fresh holder pays the holder slot on every path.
    for (r, w) in [(&lv.out, "linearvesting"), (&tl.out, "tokentimelock"), (&gl.out, "native")] {
        assert_eq!(slots(r, &format!("{w}/grant-lifecycle-cold"), "claim/1-partial"), 1 + u64::from(w == "linearvesting"), "{w}");
    }
}
