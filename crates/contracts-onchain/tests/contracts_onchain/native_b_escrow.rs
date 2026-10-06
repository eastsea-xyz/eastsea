//! B2 escrow head-to-head: the toolbox's MilestoneEscrow (native DBLN) against
//! the native EscrowBook with the same asset (DBLN, no arbiter), on the same
//! genesis and wallet. EscrowBook's arbiter and ERC-20 instances, which the
//! original cannot express, are recorded separately.
use super::harness::*;
use super::native_b_common::*;
use alloy_primitives::keccak256;
use alloy_sol_types::{sol, SolCall, SolValue};
use serde_json::Value;

sol! {
    interface Milestone {
        function createDeal(address seller, uint64 milestoneCount) payable;
        function approveMilestone(uint256 dealId, uint64 index, uint256 amount);
        function sellerWithdraw(uint256 dealId);
        function buyerRefund(uint256 dealId);
    }
    interface Book {
        function create(address seller, uint128 amount, uint64 acceptBy, uint64 deliverBy, uint8 policy, bytes32 termsHash) payable returns (uint256);
        function accept(uint256 id);
        function cancel(uint256 id);
        function release(uint256 id);
        function refund(uint256 id);
        function propose(uint256 id, uint128 sellerAward);
        function acceptProposal(uint256 id, uint64 round, uint128 sellerAward);
        function dispute(uint256 id);
        function rule(uint256 id, uint128 sellerAward);
        function resolveTimeout(uint256 id);
        function pay(uint256 id, bool toSeller);
        function tripBrake();
    }
}

const BUYER: u8 = 0;
const SELLER: u8 = 1;
const RELAYER: u8 = 5;
const ARBITER: u8 = 7;
const PRICE: u64 = 1_000_000;
const ACCEPT_WINDOW: u64 = 100;
const DELIVER_WINDOW: u64 = 1_000;
const RULING_WINDOW: u64 = 500;
const BUYER_REFUND: u8 = 0;
const SELLER_PAYMENT: u8 = 1;

fn id(n: u64) -> U256 {
    U256::from(n)
}

/// Original: one shared MilestoneEscrow, deals keyed by a counter.
struct Original {
    h: Harness,
    escrow: Address,
    next: u64,
}

impl Original {
    fn create(&mut self, milestones: u64, label: &str) -> u64 {
        let seller = self.h.addr(SELLER);
        let input = Milestone::createDealCall { seller, milestoneCount: milestones }.abi_encode();
        self.h.ok(BUYER, self.escrow, input, U256::from(PRICE), label);
        self.next += 1;
        self.next - 1
    }

    fn ok(&mut self, actor: u8, input: Vec<u8>, label: &str) {
        self.h.ok(actor, self.escrow, input, U256::ZERO, label);
    }

    fn revert(&mut self, actor: u8, input: Vec<u8>, label: &str) {
        self.h.revert(actor, self.escrow, input, U256::ZERO, label);
    }
}

/// Native: one shared EscrowBook per (asset, arbiter, ruling window).
struct Native {
    h: Harness,
    book: Address,
    next: u64,
    /// `address(0)` for DBLN, else the ERC-20 the instance holds.
    asset: Address,
}

impl Native {
    fn deploy(h: &mut Harness, asset: Address, arbiter: Address, window: u64, label: &str) -> Address {
        let args = (asset, arbiter, window, keccak256("native/escrow/DESIGN.md"), B256::ZERO).abi_encode_params();
        deploy(h, BUYER, "native/EscrowBook", args, label)
    }

    fn create_to(&mut self, seller: Address, policy: u8, label: &str) -> u64 {
        let now = self.h.timestamp();
        let input = Book::createCall {
            seller,
            amount: u128::from(PRICE),
            acceptBy: now + ACCEPT_WINDOW,
            deliverBy: now + DELIVER_WINDOW,
            policy,
            termsHash: keccak256(format!("terms {}", self.next)),
        }
        .abi_encode();
        if self.asset == Address::ZERO {
            self.h.ok(BUYER, self.book, input, U256::from(PRICE), label);
        } else {
            let calls = [approve(self.asset, self.book, U256::from(PRICE)), call(self.book, input), approve(self.asset, self.book, U256::ZERO)];
            self.h.self_batch(BUYER, &calls, label, true);
        }
        self.next += 1;
        self.next - 1
    }

    fn create(&mut self, policy: u8, label: &str) -> u64 {
        let seller = self.h.addr(SELLER);
        self.create_to(seller, policy, label)
    }

    fn ok(&mut self, actor: u8, input: Vec<u8>, label: &str) {
        self.h.ok(actor, self.book, input, U256::ZERO, label);
    }

    fn revert(&mut self, actor: u8, input: Vec<u8>, label: &str) {
        self.h.revert(actor, self.book, input, U256::ZERO, label);
    }

    fn pay(&mut self, actor: u8, deal: u64, to_seller: bool, label: &str) {
        self.ok(actor, Book::payCall { id: id(deal), toSeller: to_seller }.abi_encode(), label);
    }
}

fn original() -> Vec<Value> {
    let mut h = Harness::new();
    funded_token(&mut h);
    let mut out = Vec::new();
    begin(&mut h, "b2/original/deal-lifecycle-cold", &[BUYER, SELLER], &[BUYER]);
    let escrow = deploy(&mut h, BUYER, "toolbox/MilestoneEscrow", (Address::ZERO,).abi_encode_params(), "setup/deploy-shared-instance");
    h.phase("action");
    let mut o = Original { h, escrow, next: 1 };
    let lifecycle = |o: &mut Original| {
        let d = o.create(1, "create/fund");
        o.ok(BUYER, Milestone::approveMilestoneCall { dealId: id(d), index: 0, amount: U256::from(PRICE) }.abi_encode(), "release/approve-milestone");
        o.ok(SELLER, Milestone::sellerWithdrawCall { dealId: id(d) }.abi_encode(), "pay/seller-withdraw");
        d
    };
    lifecycle(&mut o);
    out.push(finish(&mut o.h).1);

    begin(&mut o.h, "b2/original/deal-lifecycle-warm", &[BUYER, SELLER], &[]);
    o.h.phase("action");
    let done = lifecycle(&mut o);
    out.push(finish(&mut o.h).1);

    // Split 60/40: two milestones, approve one part, refund the rest.
    begin(&mut o.h, "b2/original/split", &[BUYER, SELLER], &[]);
    o.h.phase("action");
    let d = o.create(2, "create/fund");
    o.ok(BUYER, Milestone::approveMilestoneCall { dealId: id(d), index: 0, amount: U256::from(PRICE * 6 / 10) }.abi_encode(), "split/approve-milestone-60");
    o.ok(BUYER, Milestone::buyerRefundCall { dealId: id(d) }.abi_encode(), "pay/buyer-refund-40");
    o.ok(SELLER, Milestone::sellerWithdrawCall { dealId: id(d) }.abi_encode(), "pay/seller-withdraw-60");
    out.push(finish(&mut o.h).1);

    // Seller never shows up: the buyer can always take unapproved funds back
    // (there is no acceptance step, so the seller is never protected either).
    begin(&mut o.h, "b2/original/no-show", &[BUYER], &[]);
    o.h.note("no acceptance, timeout or silence policy: a buyer who disappears after delivery leaves the seller unpaid forever");
    o.h.phase("action");
    let d = o.create(1, "create/fund");
    o.ok(BUYER, Milestone::buyerRefundCall { dealId: id(d) }.abi_encode(), "pay/buyer-refund");
    out.push(finish(&mut o.h).1);

    begin(&mut o.h, "b2/original/rejected", &[BUYER, SELLER], &[]);
    o.h.phase("action");
    let d = o.create(2, "create/fund");
    o.revert(SELLER, Milestone::approveMilestoneCall { dealId: id(d), index: 0, amount: U256::from(PRICE / 2) }.abi_encode(), "reject/wrong-actor");
    o.ok(BUYER, Milestone::approveMilestoneCall { dealId: id(d), index: 0, amount: U256::from(PRICE / 2) }.abi_encode(), "release/approve-milestone-50");
    o.revert(BUYER, Milestone::approveMilestoneCall { dealId: id(d), index: 0, amount: U256::from(PRICE / 2) }.abi_encode(), "reject/duplicate-milestone");
    o.revert(SELLER, Milestone::sellerWithdrawCall { dealId: id(done) }.abi_encode(), "reject/nothing-to-withdraw");
    out.push(finish(&mut o.h).1);
    out
}

fn native() -> Vec<Value> {
    let mut h = Harness::new();
    let token = funded_token(&mut h);
    let mut out = Vec::new();
    begin(&mut h, "b2/native/deal-lifecycle-cold", &[BUYER, SELLER], &[BUYER]);
    let book = Native::deploy(&mut h, Address::ZERO, Address::ZERO, 0, "setup/deploy-shared-instance");
    h.phase("action");
    let mut n = Native { h, book, next: 1, asset: Address::ZERO };
    let lifecycle = |n: &mut Native| {
        let d = n.create(BUYER_REFUND, "create/fund");
        n.ok(SELLER, Book::acceptCall { id: id(d) }.abi_encode(), "accept");
        n.ok(BUYER, Book::releaseCall { id: id(d) }.abi_encode(), "release");
        n.pay(SELLER, d, true, "pay/seller-final-deletes-record");
        d
    };
    lifecycle(&mut n);
    out.push(finish(&mut n.h).1);

    begin(&mut n.h, "b2/native/deal-lifecycle-warm", &[BUYER, SELLER], &[]);
    n.h.phase("action");
    let done = lifecycle(&mut n);
    out.push(finish(&mut n.h).1);

    begin(&mut n.h, "b2/native/split", &[BUYER, SELLER], &[]);
    n.h.phase("action");
    let d = n.create(BUYER_REFUND, "create/fund");
    n.ok(SELLER, Book::acceptCall { id: id(d) }.abi_encode(), "accept");
    let award = u128::from(PRICE * 6 / 10);
    n.ok(BUYER, Book::proposeCall { id: id(d), sellerAward: award }.abi_encode(), "split/propose-60-first");
    n.ok(SELLER, Book::acceptProposalCall { id: id(d), round: 1, sellerAward: award }.abi_encode(), "split/accept-proposal");
    n.pay(BUYER, d, false, "pay/buyer-40");
    n.pay(SELLER, d, true, "pay/seller-60-final-deletes-record");
    out.push(finish(&mut n.h).1);

    // Seller never accepts: the buyer withdraws, or anyone lapses it at acceptBy.
    begin(&mut n.h, "b2/native/no-show", &[BUYER], &[]);
    n.h.phase("action");
    let d = n.create(BUYER_REFUND, "create/fund");
    n.ok(BUYER, Book::cancelCall { id: id(d) }.abi_encode(), "cancel/by-buyer");
    n.pay(BUYER, d, false, "pay/buyer-refund-final");
    let d = n.create(BUYER_REFUND, "create/fund-2");
    let at = n.h.timestamp() + ACCEPT_WINDOW;
    n.h.at(at);
    n.ok(RELAYER, Book::cancelCall { id: id(d) }.abi_encode(), "cancel/lapse-by-anyone");
    n.pay(RELAYER, d, false, "pay/buyer-refund-by-anyone");
    out.push(finish(&mut n.h).1);

    // Silence after acceptance: anyone applies the agreed policy at deliverBy.
    begin(&mut n.h, "b2/native/silence", &[BUYER, SELLER], &[]);
    n.h.phase("action");
    let refund = n.create(BUYER_REFUND, "create/fund-buyer-refund-policy");
    n.ok(SELLER, Book::acceptCall { id: id(refund) }.abi_encode(), "accept");
    let paid = n.create(SELLER_PAYMENT, "create/fund-seller-payment-policy");
    n.ok(SELLER, Book::acceptCall { id: id(paid) }.abi_encode(), "accept-2");
    let at = n.h.timestamp() + DELIVER_WINDOW;
    n.h.at(at);
    n.ok(RELAYER, Book::resolveTimeoutCall { id: id(refund) }.abi_encode(), "timeout/buyer-refund-policy");
    n.pay(RELAYER, refund, false, "pay/buyer-by-anyone");
    n.ok(RELAYER, Book::resolveTimeoutCall { id: id(paid) }.abi_encode(), "timeout/seller-payment-policy");
    n.pay(RELAYER, paid, true, "pay/seller-by-anyone");
    out.push(finish(&mut n.h).1);

    begin(&mut n.h, "b2/native/rejected", &[BUYER, SELLER], &[]);
    n.h.phase("action");
    let d = n.create(BUYER_REFUND, "create/fund");
    n.revert(BUYER, Book::acceptCall { id: id(d) }.abi_encode(), "reject/wrong-actor");
    n.ok(SELLER, Book::acceptCall { id: id(d) }.abi_encode(), "accept");
    n.revert(SELLER, Book::releaseCall { id: id(d) }.abi_encode(), "reject/seller-cannot-release");
    n.revert(RELAYER, Book::resolveTimeoutCall { id: id(d) }.abi_encode(), "reject/timeout-too-early");
    n.revert(SELLER, Book::acceptProposalCall { id: id(d), round: 1, sellerAward: 1 }.abi_encode(), "reject/stale-proposal");
    n.revert(SELLER, Book::payCall { id: id(done), toSeller: true }.abi_encode(), "reject/pay-deleted-deal");
    n.revert(RELAYER, Book::tripBrakeCall {}.abi_encode(), "reject/trip-brake-predicate-false");
    out.push(finish(&mut n.h).1);

    // Arbiter instance: a capability the original does not have.
    begin(&mut n.h, "b2/native-arbiter/dispute", &[BUYER, SELLER, ARBITER], &[]);
    let arbiter = n.h.addr(ARBITER);
    let book = Native::deploy(&mut n.h, Address::ZERO, arbiter, RULING_WINDOW, "setup/deploy-arbiter-instance");
    n.h.phase("action");
    let mut a = Native { h: std::mem::take(&mut n.h), book, next: 1, asset: Address::ZERO };
    let d = a.create(BUYER_REFUND, "create/fund");
    a.ok(SELLER, Book::acceptCall { id: id(d) }.abi_encode(), "accept");
    a.ok(SELLER, Book::disputeCall { id: id(d) }.abi_encode(), "dispute");
    a.ok(ARBITER, Book::ruleCall { id: id(d), sellerAward: u128::from(PRICE * 7 / 10) }.abi_encode(), "rule/70-30");
    a.pay(SELLER, d, true, "pay/seller-70");
    a.pay(BUYER, d, false, "pay/buyer-30-final");
    let d = a.create(BUYER_REFUND, "create/fund-2");
    a.ok(SELLER, Book::acceptCall { id: id(d) }.abi_encode(), "accept-2");
    a.ok(BUYER, Book::disputeCall { id: id(d) }.abi_encode(), "dispute-2");
    let at = a.h.timestamp() + RULING_WINDOW;
    a.h.at(at);
    a.ok(RELAYER, Book::resolveTimeoutCall { id: id(d) }.abi_encode(), "timeout/absent-arbiter");
    a.pay(RELAYER, d, false, "pay/buyer-by-anyone");
    out.push(finish(&mut a.h).1);

    // ERC-20 instance: 3-call batch create and a fresh-holder seller.
    begin(&mut a.h, "b2/native-erc20/deal-lifecycle", &[BUYER, 10], &[]);
    let book = Native::deploy(&mut a.h, token, Address::ZERO, 0, "setup/deploy-erc20-instance");
    a.h.phase("action");
    let mut e = Native { h: std::mem::take(&mut a.h), book, next: 1, asset: token };
    let seller = e.h.addr(10);
    let d = e.create_to(seller, BUYER_REFUND, "create/batch-approve-create-approve0");
    e.ok(10, Book::acceptCall { id: id(d) }.abi_encode(), "accept");
    e.ok(BUYER, Book::releaseCall { id: id(d) }.abi_encode(), "release");
    e.pay(10, d, true, "pay/fresh-holder-seller-final");
    out.push(finish(&mut e.h).1);
    out
}

fn step<'a>(records: &'a [Value], workflow: &str, label: &str) -> &'a Value {
    records
        .iter()
        .find(|r| r["workflow"].as_str().unwrap().ends_with(workflow))
        .and_then(|r| r["steps"].as_array().unwrap().iter().find(|s| s["label"] == label))
        .unwrap_or_else(|| panic!("{workflow} {label}"))
}

#[test]
fn b2_escrow_original_vs_native_on_the_executor() {
    let orig = original();
    let nat = native();
    let slots = |s: &Value| s["units"]["new_slots"].as_u64().unwrap();
    // Four words per native deal; the original also stores four (plus one
    // per approved milestone) and never deletes them.
    assert_eq!(slots(step(&nat, "native/deal-lifecycle-warm", "create/fund")), 4);
    assert_eq!(slots(step(&orig, "original/deal-lifecycle-warm", "create/fund")), 4);
    assert_eq!(slots(step(&orig, "original/deal-lifecycle-warm", "release/approve-milestone")), 1);
    assert_eq!(slots(step(&nat, "native/split", "split/propose-60-first")), 1);
    let warm = |r: &[Value], w: &str| r.iter().find(|x| x["workflow"].as_str().unwrap().ends_with(w)).unwrap()["totals"]["warm"]["transactions"].as_u64().unwrap();
    assert_eq!((warm(&orig, "original/deal-lifecycle-warm"), warm(&nat, "native/deal-lifecycle-warm")), (3, 4));
}
