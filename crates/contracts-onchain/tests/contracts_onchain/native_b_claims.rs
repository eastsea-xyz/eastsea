//! B0 claims head-to-head: Uniswap MerkleDistributor (one deployment per
//! allocation, GPL fixture) against the native ClaimCampaigns (one shared
//! instance per token) on the same genesis, token, wallet and 4,096-leaf
//! allocation. Run with CONTRACTS_ONCHAIN_WORKFLOWS set to keep the records.
use super::harness::*;
use super::native_b_common::*;
use alloy_primitives::keccak256;
use alloy_sol_types::{sol, SolCall, SolValue};
use serde_json::Value;

sol! {
    interface Distributor {
        function claim(uint256 index, address account, uint256 amount, bytes32[] merkleProof);
        function isClaimed(uint256 index) returns (bool);
    }
    interface Campaigns {
        function create(uint256 expectedId, bytes32 root, uint32 leafCount, address refundRecipient, uint64 deadline, uint128 amount, bytes32 dataHash) returns (uint256);
        function claim(uint256 id, uint256 index, address account, uint128 amount, bytes32[] proof);
        function close(uint256 id);
        function prune(uint256 id, uint256 startWord, uint256 count);
        function tripBrake();
        function bitmapWord(uint256 id, uint256 word) returns (uint256);
    }
}

const LEAVES: usize = 4096;
const AMOUNT: u64 = 1_000;
const ISSUER: u8 = 0;
const RELAYER: u8 = 5;
/// Claim window in heights for the native campaign (the original has none).
const WINDOW: u64 = 50_000;

/// Leaf owners: indices 0/1 fresh holders, 2/256 warm holders, 3 a fresh
/// address with no key (relayed), 4/5 for the rejected and late paths.
fn account(h: &Harness, index: usize) -> Address {
    match index {
        0 => h.addr(10),
        1 => h.addr(11),
        2 => h.addr(1),
        256 => h.addr(2),
        4 => h.addr(12),
        5 => h.addr(13),
        6 => h.addr(3),
        _ => Address::from_word(keccak256((index as u64).to_be_bytes())),
    }
}

fn levels(leaves: Vec<B256>, sorted: bool) -> Vec<Vec<B256>> {
    let mut out = vec![leaves];
    while out.last().unwrap().len() > 1 {
        let next = out
            .last()
            .unwrap()
            .chunks(2)
            .map(|p| {
                let (a, b) = if sorted && p[1] < p[0] { (p[1], p[0]) } else { (p[0], p[1]) };
                keccak256([a.as_slice(), b.as_slice()].concat())
            })
            .collect();
        out.push(next);
    }
    out
}

fn proof(levels: &[Vec<B256>], index: usize) -> Vec<B256> {
    levels[..levels.len() - 1].iter().enumerate().map(|(k, level)| level[(index >> k) ^ 1]).collect()
}

/// OZ sorted-pair tree over `keccak256(abi.encodePacked(index, account, amount))`.
fn uniswap_tree(h: &Harness) -> Vec<Vec<B256>> {
    let leaves = (0..LEAVES)
        .map(|i| keccak256([U256::from(i).to_be_bytes::<32>().as_slice(), account(h, i).as_slice(), U256::from(AMOUNT).to_be_bytes::<32>().as_slice()].concat()))
        .collect();
    levels(leaves, true)
}

/// Positional tree over the domain-separated native leaf (tools/claims_tree.py).
fn native_tree(h: &Harness, instance: Address, id: u64) -> Vec<Vec<B256>> {
    let domain = keccak256("eastsea.native.claims.leaf.v1");
    let leaves = (0..LEAVES)
        .map(|i| keccak256((domain, U256::from(CHAIN), instance, U256::from(id), U256::from(i), account(h, i), U256::from(AMOUNT)).abi_encode_params()))
        .collect();
    levels(leaves, false)
}

fn total() -> U256 {
    U256::from(AMOUNT) * U256::from(LEAVES)
}

struct Original {
    h: Harness,
    token: Address,
    distributor: Address,
    tree: Vec<Vec<B256>>,
}

impl Original {
    fn claim_input(&self, index: usize, amount: u64, proof_len: usize) -> Vec<u8> {
        let mut p = proof(&self.tree, index);
        p.truncate(proof_len);
        Distributor::claimCall { index: U256::from(index), account: account(&self.h, index), amount: U256::from(amount), merkleProof: p }.abi_encode()
    }

    /// Issuer: deploy one distributor per allocation, then fund it.
    fn open(&mut self, label: &str) -> Address {
        let tree = uniswap_tree(&self.h);
        let args = (self.token, tree.last().unwrap()[0]).abi_encode_params();
        let distributor = deploy(&mut self.h, ISSUER, "gpl/MerkleDistributor", args, &format!("{label}/deploy-distributor"));
        let fund = Erc20::transferCall { to: distributor, amount: total() }.abi_encode();
        self.h.ok(ISSUER, self.token, fund, U256::ZERO, &format!("{label}/fund-transfer"));
        self.tree = tree;
        distributor
    }
}

struct Native {
    h: Harness,
    token: Address,
    instance: Address,
    deadline: u64,
}

impl Native {
    fn tree(&self) -> Vec<Vec<B256>> {
        native_tree(&self.h, self.instance, 1)
    }

    fn claim_input(&self, tree: &[Vec<B256>], index: usize, amount: u64, proof_len: usize) -> Vec<u8> {
        let mut p = proof(tree, index);
        p.truncate(proof_len);
        Campaigns::claimCall { id: U256::from(1), index: U256::from(index), account: account(&self.h, index), amount: amount as u128, proof: p }.abi_encode()
    }

    /// Issuer: one batch approve(exact) -> create -> approve(0), one prompt.
    fn open(&mut self, id: u64, window: u64, label: &str) {
        let tree = native_tree(&self.h, self.instance, id);
        let create = Campaigns::createCall {
            expectedId: U256::from(id),
            root: tree.last().unwrap()[0],
            leafCount: LEAVES as u32,
            refundRecipient: self.h.addr(ISSUER),
            deadline: self.h.number() + window,
            amount: total().to::<u128>(),
            dataHash: keccak256(format!("campaign {id} csv")),
        };
        self.deadline = create.deadline;
        let calls = [approve(self.token, self.instance, total()), call(self.instance, create.abi_encode()), approve(self.token, self.instance, U256::ZERO)];
        self.h.self_batch(ISSUER, &calls, &format!("{label}/batch-approve-create-approve0"), true);
    }
}

fn original() -> (Vec<Value>, Original) {
    let mut h = Harness::new();
    let token = funded_token(&mut h);
    let mut o = Original { h, token, distributor: Address::ZERO, tree: Vec::new() };
    let mut out = Vec::new();

    begin(&mut o.h, "b0/original/open-campaign", &[ISSUER], &[ISSUER]);
    o.h.phase("action");
    o.distributor = o.open("open");
    out.push(finish(&mut o.h).1);

    begin(&mut o.h, "b0/original/open-campaign-repeat", &[ISSUER], &[]);
    o.h.phase("action");
    o.open("open-repeat");
    out.push(finish(&mut o.h).1);
    // Claims below run against the first allocation.
    o.tree = uniswap_tree(&o.h);

    begin(&mut o.h, "b0/original/claims", &[10, 11, 1, 2], &[]);
    o.h.phase("action");
    let d = o.distributor;
    for (actor, index, label) in [(10, 0, "claim/fresh-holder/first-in-word"), (11, 1, "claim/fresh-holder/same-word"), (1, 2, "claim/warm-holder/same-word"), (2, 256, "claim/warm-holder/first-in-word"), (RELAYER, 3, "claim/relayed/fresh-address")] {
        let input = o.claim_input(index, AMOUNT, 12);
        o.h.ok(actor, d, input, U256::ZERO, label);
    }
    out.push(finish(&mut o.h).1);

    begin(&mut o.h, "b0/original/rejected", &[10, 12], &[]);
    o.h.phase("action");
    let replay = o.claim_input(0, AMOUNT, 12);
    o.h.revert(10, d, replay, U256::ZERO, "reject/replay");
    let wrong_amount = o.claim_input(4, AMOUNT + 1, 12);
    o.h.revert(12, d, wrong_amount, U256::ZERO, "reject/invalid-proof");
    let short = o.claim_input(4, AMOUNT, 11);
    o.h.revert(12, d, short, U256::ZERO, "reject/short-proof");
    out.push(finish(&mut o.h).1);

    // No deadline, no close: a late claim still pays and the unclaimed rest
    // stays in the distributor forever (no sweep in this version).
    begin(&mut o.h, "b0/original/end-of-life", &[13], &[]);
    o.h.note("MerkleDistributor has no deadline, close or sweep: unclaimed funds never return");
    o.h.phase("action");
    let now = o.h.timestamp() + WINDOW + 10;
    o.h.at(now);
    let late = o.claim_input(5, AMOUNT, 12);
    o.h.ok(13, d, late, U256::ZERO, "claim/late-still-paid");
    out.push(finish(&mut o.h).1);

    // Deficit: the issuer seizes the distributor; claims fail one by one once
    // the balance is short. No brake exists to latch.
    begin(&mut o.h, "b0/original/deficit", &[3], &[]);
    o.h.phase("action");
    let held = balance(&o.h, o.token, d);
    seize(&mut o.h, o.token, d, held - U256::from(AMOUNT / 2));
    let blocked = o.claim_input(6, AMOUNT, 12);
    o.h.revert(3, d, blocked, U256::ZERO, "claim/blocked-by-deficit");
    out.push(finish(&mut o.h).1);
    (out, o)
}

fn native() -> (Vec<Value>, Native) {
    let mut h = Harness::new();
    let token = funded_token(&mut h);
    let mut n = Native { h, token, instance: Address::ZERO, deadline: 0 };
    let mut out = Vec::new();

    begin(&mut n.h, "b0/native/open-campaign", &[ISSUER], &[ISSUER]);
    n.instance = deploy(&mut n.h, ISSUER, "native/ClaimCampaigns", (token, B256::ZERO).abi_encode_params(), "setup/deploy-shared-instance");
    n.h.phase("action");
    n.open(1, WINDOW, "open");
    let deadline = n.deadline;
    out.push(finish(&mut n.h).1);

    begin(&mut n.h, "b0/native/open-campaign-repeat", &[ISSUER], &[]);
    n.h.phase("action");
    // A longer window keeps campaign 2 live for the deficit case below.
    n.open(2, 100 * WINDOW, "open-repeat");
    out.push(finish(&mut n.h).1);

    let tree = n.tree();
    let c = n.instance;
    begin(&mut n.h, "b0/native/claims", &[10, 11, 1, 2], &[]);
    n.h.phase("action");
    for (actor, index, label) in [(10, 0, "claim/fresh-holder/first-in-word"), (11, 1, "claim/fresh-holder/same-word"), (1, 2, "claim/warm-holder/same-word"), (2, 256, "claim/warm-holder/first-in-word"), (RELAYER, 3, "claim/relayed/fresh-address")] {
        let input = n.claim_input(&tree, index, AMOUNT, 12);
        n.h.ok(actor, c, input, U256::ZERO, label);
    }
    out.push(finish(&mut n.h).1);

    begin(&mut n.h, "b0/native/rejected", &[10, 12], &[]);
    n.h.phase("action");
    let replay = n.claim_input(&tree, 0, AMOUNT, 12);
    n.h.revert(10, c, replay, U256::ZERO, "reject/replay");
    let wrong_amount = n.claim_input(&tree, 4, AMOUNT + 1, 12);
    n.h.revert(12, c, wrong_amount, U256::ZERO, "reject/invalid-proof");
    let short = n.claim_input(&tree, 4, AMOUNT, 11);
    n.h.revert(12, c, short, U256::ZERO, "reject/short-proof");
    let trip = Campaigns::tripBrakeCall {}.abi_encode();
    n.h.revert(12, c, trip, U256::ZERO, "reject/trip-brake-predicate-false");
    out.push(finish(&mut n.h).1);

    // After the deadline: a late claim is refused, anyone closes (refund to
    // the fixed recipient) and prunes the two touched bitmap words.
    begin(&mut n.h, "b0/native/end-of-life", &[13], &[]);
    n.h.phase("action");
    let after = n.h.timestamp() + (deadline - n.h.number()) + 1;
    n.h.at(after);
    assert!(n.h.number() > deadline);
    let late = n.claim_input(&tree, 5, AMOUNT, 12);
    n.h.revert(13, c, late, U256::ZERO, "claim/late-refused");
    let refund_before = balance(&n.h, token, n.h.addr(ISSUER));
    n.h.ok(RELAYER, c, Campaigns::closeCall { id: U256::from(1) }.abi_encode(), U256::ZERO, "close/refund-remainder");
    assert_eq!(balance(&n.h, token, n.h.addr(ISSUER)) - refund_before, total() - U256::from(5 * AMOUNT));
    n.h.ok(RELAYER, c, Campaigns::pruneCall { id: U256::from(1), startWord: U256::ZERO, count: U256::from(2) }.abi_encode(), U256::ZERO, "prune/2-touched-words");
    out.push(finish(&mut n.h).1);

    // Deficit: the issuer seizes the shared balance. Anyone latches the brake;
    // every claim on the instance is blocked until someone recapitalises.
    begin(&mut n.h, "b0/native/deficit", &[3], &[]);
    n.h.phase("action");
    seize(&mut n.h, token, c, U256::from(AMOUNT / 2));
    n.h.ok(RELAYER, c, Campaigns::tripBrakeCall {}.abi_encode(), U256::ZERO, "brake/trip-latch");
    let tree2 = native_tree(&n.h, c, 2);
    let mut p = proof(&tree2, 6);
    p.truncate(12);
    let blocked = Campaigns::claimCall { id: U256::from(2), index: U256::from(6), account: account(&n.h, 6), amount: AMOUNT as u128, proof: p }.abi_encode();
    n.h.revert(3, c, blocked, U256::ZERO, "claim/blocked-by-deficit");
    out.push(finish(&mut n.h).1);
    (out, n)
}

fn step<'a>(records: &'a [Value], workflow: &str, label: &str) -> &'a Value {
    records
        .iter()
        .find(|r| r["workflow"].as_str().unwrap().ends_with(workflow))
        .and_then(|r| r["steps"].as_array().unwrap().iter().find(|s| s["label"] == label))
        .unwrap_or_else(|| panic!("{workflow} {label}"))
}

fn slots(s: &Value) -> u64 {
    s["units"]["new_slots"].as_u64().unwrap()
}

#[test]
fn b0_claims_original_vs_native_on_the_executor() {
    let (orig, o) = original();
    let (nat, n) = native();
    // Bitmap: one slot for the first claim in a 256-index word, none after.
    for r in [&orig, &nat] {
        let w = if r[0]["workflow"].as_str().unwrap().contains("native") { "native/claims" } else { "original/claims" };
        assert_eq!(slots(step(r, w, "claim/fresh-holder/first-in-word")), 2, "{w}: bitmap word + new holder");
        assert_eq!(slots(step(r, w, "claim/fresh-holder/same-word")), 1, "{w}: new holder only");
        assert_eq!(slots(step(r, w, "claim/warm-holder/same-word")), 0, "{w}");
        assert_eq!(slots(step(r, w, "claim/warm-holder/first-in-word")), 1, "{w}");
    }
    // Per-allocation code is the original's recurring cost; the native
    // campaign pays three words instead.
    let deploy = step(&orig, "original/open-campaign-repeat", "open-repeat/deploy-distributor");
    assert!(deploy["units"]["code_bytes"].as_u64().unwrap() > 2_000);
    let create = step(&nat, "native/open-campaign-repeat", "open-repeat/batch-approve-create-approve0");
    assert_eq!(slots(create), 3, "three campaign words; allowance returns to zero inside the batch");
    assert_eq!(create["units"]["code_bytes"], 0);
    // Exits: native closes and prunes; the original still pays late.
    assert!(step(&nat, "native/end-of-life", "close/refund-remainder")["success"].as_bool().unwrap());
    assert!(step(&orig, "original/end-of-life", "claim/late-still-paid")["success"].as_bool().unwrap());
    let word = Campaigns::bitmapWordCall { id: U256::from(1), word: U256::ZERO }.abi_encode();
    assert_eq!(U256::from_be_slice(&n.h.view(0, n.instance, word)), U256::ZERO, "pruned");
    let claimed = Distributor::isClaimedCall { index: U256::from(5) }.abi_encode();
    assert_eq!(U256::from_be_slice(&o.h.view(0, o.distributor, claimed)), U256::ONE);
}
