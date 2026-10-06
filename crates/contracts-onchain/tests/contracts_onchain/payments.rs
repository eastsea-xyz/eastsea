//! Native payment examples, run through the paid-state block executor.
use super::harness::*;
use alloy_primitives::keccak256;
use alloy_sol_types::{abi::TokenSeq, sol, SolCall, SolType, SolValue};

sol! {
    // `u8` is not a `SolValue`; the generated call encodes uint8 exactly.
    function engageBrake(uint8 state);
}

fn call<T: SolValue>(signature: &str, arguments: T) -> Vec<u8>
where
    for<'a> <T::SolType as SolType>::Token<'a>: TokenSeq<'a>,
{
    let mut data = keccak256(signature).as_slice()[..4].to_vec();
    data.extend(arguments.abi_encode_params());
    data
}

fn n(value: u64) -> U256 {
    U256::from(value)
}

fn word(data: &[u8], index: usize) -> U256 {
    U256::from_be_slice(&data[index * 32..(index + 1) * 32])
}

fn read<T: SolValue>(h: &mut Harness, to: Address, signature: &str, args: T) -> Bytes
where
    for<'a> <T::SolType as SolType>::Token<'a>: TokenSeq<'a>,
{
    h.view(0, to, call(signature, args))
}

fn bad(
    h: &mut Harness,
    actor: u8,
    to: Address,
    data: Vec<u8>,
    value: U256,
    error: &str,
    label: &str,
) {
    let receipt = h.revert(actor, to, data, value, label);
    assert!(
        receipt.output.len() >= 4,
        "{label}: missing revert selector"
    );
    assert_eq!(
        &receipt.output[..4],
        &keccak256(error).as_slice()[..4],
        "{label}"
    );
}

fn brake(h: &mut Harness, to: Address) {
    bad(
        h,
        2,
        to,
        engageBrakeCall { state: 1u8 }.abi_encode(),
        U256::ZERO,
        "NotBrakeGuardian(address)",
        "brake/wrong guardian",
    );
    for state in [0u8, 3u8, u8::MAX] {
        bad(
            h,
            0,
            to,
            engageBrakeCall { state }.abi_encode(),
            U256::ZERO,
            "BrakeOnlyStronger(uint8,uint8)",
            "brake/invalid state",
        );
    }
    h.ok(
        0,
        to,
        engageBrakeCall { state: 1u8 }.abi_encode(),
        U256::ZERO,
        "brake/entry stop",
    );
    for state in [0u8, 1u8] {
        bad(
            h,
            0,
            to,
            engageBrakeCall { state }.abi_encode(),
            U256::ZERO,
            "BrakeOnlyStronger(uint8,uint8)",
            "brake/monotonicity",
        );
    }
    h.ok(
        0,
        to,
        engageBrakeCall { state: 2u8 }.abi_encode(),
        U256::ZERO,
        "brake/full stop",
    );
    for state in [1u8, 2u8] {
        bad(
            h,
            0,
            to,
            engageBrakeCall { state }.abi_encode(),
            U256::ZERO,
            "BrakeOnlyStronger(uint8,uint8)",
            "brake/no weakening or repeat",
        );
    }
    let state = read(h, to, "brakeState()", ());
    assert_eq!(word(&state, 0), n(2));
}

#[test]
fn invoice_book_lifecycle_and_every_guard() {
    let mut h = Harness::new();
    let guardian = h.addr(0);
    let payee = h.addr(1);
    let book = h.deploy("toolbox/InvoiceBook", (guardian, payee).abi_encode_params());
    h.deploy_revert(
        "toolbox/InvoiceBook",
        (guardian, Address::ZERO).abi_encode_params(),
        "invoice/zero payee",
    );
    bad(
        &mut h,
        2,
        book,
        call("issue(uint128,uint48,string)", (n(100), n(1000), "memo")),
        U256::ZERO,
        "NotPayee(address)",
        "invoice/issue wrong caller",
    );
    for (amount, period, error) in [(0, 1000, "ZeroAmount()"), (100, 0, "ZeroPeriod()")] {
        bad(
            &mut h,
            1,
            book,
            call("issue(uint128,uint48,string)", (n(amount), n(period), "")),
            U256::ZERO,
            error,
            "invoice/zero input",
        );
    }
    bad(
        &mut h,
        2,
        book,
        vec![],
        n(1),
        "PaymentNotAllowed()",
        "invoice/direct payment",
    );
    for signature in ["settle(uint256)", "purge(uint256)"] {
        bad(
            &mut h,
            2,
            book,
            call(signature, (n(999),)),
            U256::ZERO,
            "UnknownInvoice(uint256)",
            "invoice/unknown id",
        );
    }
    bad(
        &mut h,
        1,
        book,
        call("void(uint256)", (n(999),)),
        U256::ZERO,
        "UnknownInvoice(uint256)",
        "invoice/void unknown",
    );
    h.ok(
        1,
        book,
        call(
            "issue(uint128,uint48,string)",
            (n(100), n(1000), "first invoice"),
        ),
        U256::ZERO,
        "invoice/issue",
    );
    let due = word(&read(&mut h, book, "invoices(uint256)", (n(1),)), 1).to::<u64>();
    for paid in [0, 99, 101] {
        bad(
            &mut h,
            2,
            book,
            call("settle(uint256)", (n(1),)),
            n(paid),
            "WrongAmount(uint256,uint256)",
            "invoice/wrong payment",
        );
    }
    bad(
        &mut h,
        2,
        book,
        call("void(uint256)", (n(1),)),
        U256::ZERO,
        "NotPayee(address)",
        "invoice/void wrong caller",
    );
    bad(
        &mut h,
        2,
        book,
        call("purge(uint256)", (n(1),)),
        U256::ZERO,
        "NotPastDue(uint256,uint256)",
        "invoice/early purge",
    );
    h.at(due);
    h.ok(
        2,
        book,
        call("settle(uint256)", (n(1),)),
        n(100),
        "invoice/settle exact deadline",
    );
    assert_eq!(word(&read(&mut h, book, "totalSettled()", ()), 0), n(100));
    assert_eq!(
        word(&read(&mut h, book, "invoices(uint256)", (n(1),)), 0),
        U256::ZERO
    );
    bad(
        &mut h,
        2,
        book,
        call("settle(uint256)", (n(1),)),
        n(100),
        "UnknownInvoice(uint256)",
        "invoice/double settle",
    );
    h.ok(
        1,
        book,
        call("issue(uint128,uint48,string)", (n(100), n(1000), "void")),
        U256::ZERO,
        "invoice/issue to void",
    );
    h.ok(
        1,
        book,
        call("void(uint256)", (n(2),)),
        U256::ZERO,
        "invoice/void",
    );
    bad(
        &mut h,
        1,
        book,
        call(
            "issue(uint128,uint48,string)",
            (n(1), U256::from((1u64 << 48) - 1), ""),
        ),
        U256::ZERO,
        "Panic(uint256)",
        "invoice/maximum period checked overflow",
    );
    h.ok(
        1,
        book,
        call("issue(uint128,uint48,string)", (n(100), n(1000), "expired")),
        U256::ZERO,
        "invoice/issue to expire",
    );
    let due = word(&read(&mut h, book, "invoices(uint256)", (n(3),)), 1).to::<u64>();
    h.at(due + 1);
    bad(
        &mut h,
        2,
        book,
        call("settle(uint256)", (n(3),)),
        n(100),
        "InvoiceExpired(uint256,uint256)",
        "invoice/expired exact payment",
    );
    h.ok(
        2,
        book,
        call("purge(uint256)", (n(3),)),
        U256::ZERO,
        "invoice/purge expired",
    );
    h.ok(
        1,
        book,
        call(
            "issue(uint128,uint48,string)",
            (n(100), n(1000), "braked settle"),
        ),
        U256::ZERO,
        "invoice/issue before brake",
    );
    h.ok(
        1,
        book,
        call(
            "issue(uint128,uint48,string)",
            (U256::from(u128::MAX), n(1000), "max amount"),
        ),
        U256::ZERO,
        "invoice/maximum amount",
    );
    let due = word(&read(&mut h, book, "invoices(uint256)", (n(5),)), 1).to::<u64>();
    h.ok(
        1,
        book,
        call(
            "issue(uint128,uint48,string)",
            (n(100), n(1000), "braked void"),
        ),
        U256::ZERO,
        "invoice/issue before brake void",
    );
    brake(&mut h, book);
    bad(
        &mut h,
        1,
        book,
        call("issue(uint128,uint48,string)", (n(1), n(1), "")),
        U256::ZERO,
        "BrakedNewEntry(uint8)",
        "invoice/brake blocks issue",
    );
    h.ok(
        2,
        book,
        call("settle(uint256)", (n(4),)),
        n(100),
        "invoice/settle while fully braked",
    );
    h.ok(
        1,
        book,
        call("void(uint256)", (n(6),)),
        U256::ZERO,
        "invoice/void while fully braked",
    );
    h.at(due + 1);
    bad(
        &mut h,
        2,
        book,
        call("settle(uint256)", (n(5),)),
        U256::ZERO,
        "WrongAmount(uint256,uint256)",
        "invoice/amount guard precedes expiry",
    );
    h.ok(
        2,
        book,
        call("purge(uint256)", (n(5),)),
        U256::ZERO,
        "invoice/purge while braked",
    );
    bad(
        &mut h,
        2,
        book,
        call("purge(uint256)", (n(5),)),
        U256::ZERO,
        "UnknownInvoice(uint256)",
        "invoice/double purge",
    );
}

#[test]
fn milestone_escrow_partial_approval_withdrawal_refund_and_guards() {
    let mut h = Harness::new();
    let guardian = h.addr(0);
    let seller = h.addr(2);
    let buyer = h.addr(1);
    let escrow = h.deploy("toolbox/MilestoneEscrow", (guardian,).abi_encode_params());
    bad(
        &mut h,
        1,
        escrow,
        vec![],
        n(1),
        "NativeTransferFailed()",
        "escrow/direct payment",
    );
    for (who, count, value, error) in [
        (seller, 2, 0, "ZeroAmount()"),
        (Address::ZERO, 2, 100, "NotDealParty()"),
        (buyer, 2, 100, "NotDealParty()"),
        (seller, 0, 100, "ZeroMilestones()"),
    ] {
        bad(
            &mut h,
            1,
            escrow,
            call("createDeal(address,uint64)", (who, n(count))),
            n(value),
            error,
            "escrow/invalid deal",
        );
    }
    for signature in ["sellerWithdraw(uint256)", "buyerRefund(uint256)"] {
        bad(
            &mut h,
            1,
            escrow,
            call(signature, (n(999),)),
            U256::ZERO,
            "DealNotFound(uint256)",
            "escrow/unknown deal",
        );
    }
    bad(
        &mut h,
        1,
        escrow,
        call(
            "approveMilestone(uint256,uint64,uint256)",
            (n(999), n(0), n(1)),
        ),
        U256::ZERO,
        "DealNotFound(uint256)",
        "escrow/approve unknown",
    );
    h.ok(
        1,
        escrow,
        call("createDeal(address,uint64)", (seller, n(2))),
        n(100),
        "escrow/create",
    );
    bad(
        &mut h,
        2,
        escrow,
        call(
            "approveMilestone(uint256,uint64,uint256)",
            (n(1), n(0), n(1)),
        ),
        U256::ZERO,
        "NotDealParty()",
        "escrow/approve wrong caller",
    );
    for (index, amount, error) in [
        (n(0), U256::ZERO, "ZeroAmount()"),
        (n(2), n(1), "IndexOutOfRange(uint64,uint64)"),
        (U256::from(u64::MAX), n(1), "IndexOutOfRange(uint64,uint64)"),
        (n(0), U256::MAX, "ExceedsDeposit(uint256,uint256)"),
    ] {
        bad(
            &mut h,
            1,
            escrow,
            call(
                "approveMilestone(uint256,uint64,uint256)",
                (n(1), index, amount),
            ),
            U256::ZERO,
            error,
            "escrow/invalid approval",
        );
    }
    bad(
        &mut h,
        1,
        escrow,
        call("sellerWithdraw(uint256)", (n(1),)),
        U256::ZERO,
        "NotDealParty()",
        "escrow/withdraw wrong caller",
    );
    bad(
        &mut h,
        2,
        escrow,
        call("sellerWithdraw(uint256)", (n(1),)),
        U256::ZERO,
        "NothingToWithdraw()",
        "escrow/withdraw unapproved",
    );
    bad(
        &mut h,
        2,
        escrow,
        call("buyerRefund(uint256)", (n(1),)),
        U256::ZERO,
        "NotDealParty()",
        "escrow/refund wrong caller",
    );
    h.ok(
        1,
        escrow,
        call(
            "approveMilestone(uint256,uint64,uint256)",
            (n(1), n(0), n(40)),
        ),
        U256::ZERO,
        "escrow/approve first",
    );
    bad(
        &mut h,
        1,
        escrow,
        call(
            "approveMilestone(uint256,uint64,uint256)",
            (n(1), n(0), n(1)),
        ),
        U256::ZERO,
        "MilestoneAlreadyApproved(uint256,uint64)",
        "escrow/double approval",
    );
    brake(&mut h, escrow);
    bad(
        &mut h,
        1,
        escrow,
        call("createDeal(address,uint64)", (seller, n(1))),
        n(1),
        "BrakedNewEntry(uint8)",
        "escrow/braked entry",
    );
    h.ok(
        2,
        escrow,
        call("sellerWithdraw(uint256)", (n(1),)),
        U256::ZERO,
        "escrow/withdraw while braked",
    );
    bad(
        &mut h,
        2,
        escrow,
        call("sellerWithdraw(uint256)", (n(1),)),
        U256::ZERO,
        "NothingToWithdraw()",
        "escrow/double withdrawal",
    );
    h.ok(
        1,
        escrow,
        call("buyerRefund(uint256)", (n(1),)),
        U256::ZERO,
        "escrow/refund unapproved remainder",
    );
    let info = read(&mut h, escrow, "dealInfo(uint256)", (n(1),));
    assert_eq!(word(&info, 2), U256::ZERO);
    assert_eq!(word(&info, 3), U256::ZERO);
    bad(
        &mut h,
        1,
        escrow,
        call("buyerRefund(uint256)", (n(1),)),
        U256::ZERO,
        "NothingToRefund()",
        "escrow/double refund",
    );
}

#[test]
fn crowdfund_success_overfunding_and_failed_campaign_refunds() {
    let mut h = Harness::new();
    let guardian = h.addr(0);
    let beneficiary = h.addr(2);
    let contributor = h.addr(1);
    for (who, goal, duration) in [
        (Address::ZERO, 100, 1000),
        (beneficiary, 0, 1000),
        (beneficiary, 100, 0),
    ] {
        h.deploy_revert(
            "toolbox/AllOrNothingCrowdfund",
            (guardian, who, n(goal), n(duration)).abi_encode_params(),
            "crowdfund/invalid constructor",
        );
    }
    let fund = h.deploy(
        "toolbox/AllOrNothingCrowdfund",
        (guardian, beneficiary, n(100), n(1000)).abi_encode_params(),
    );
    bad(
        &mut h,
        1,
        fund,
        vec![],
        n(1),
        "NativeTransferFailed()",
        "crowdfund/direct payment",
    );
    bad(
        &mut h,
        1,
        fund,
        call("contribute()", ()),
        U256::ZERO,
        "ZeroAmount()",
        "crowdfund/zero contribution",
    );
    bad(
        &mut h,
        1,
        fund,
        call("refund()", ()),
        U256::ZERO,
        "NothingToRefund()",
        "crowdfund/no contribution",
    );
    bad(
        &mut h,
        1,
        fund,
        call("withdraw()", ()),
        U256::ZERO,
        "NothingToWithdraw()",
        "crowdfund/under target",
    );
    h.ok(
        1,
        fund,
        call("contribute()", ()),
        n(40),
        "crowdfund/contribute",
    );
    bad(
        &mut h,
        1,
        fund,
        call("refund()", ()),
        U256::ZERO,
        "RefundNotAllowed(uint256,uint256,uint256)",
        "crowdfund/early refund",
    );
    h.ok(
        3,
        fund,
        call("contribute()", ()),
        n(70),
        "crowdfund/overfund final contribution",
    );
    assert_eq!(word(&read(&mut h, fund, "state()", ()), 0), n(1));
    bad(
        &mut h,
        1,
        fund,
        call("contribute()", ()),
        n(1),
        "AlreadyFunded(uint256,uint256)",
        "crowdfund/closed funded entry",
    );
    bad(
        &mut h,
        1,
        fund,
        call("refund()", ()),
        U256::ZERO,
        "RefundNotAllowed(uint256,uint256,uint256)",
        "crowdfund/success forbids refund",
    );
    brake(&mut h, fund);
    bad(
        &mut h,
        1,
        fund,
        call("contribute()", ()),
        n(1),
        "BrakedNewEntry(uint8)",
        "crowdfund/braked entry",
    );
    h.ok(
        4,
        fund,
        call("withdraw()", ()),
        U256::ZERO,
        "crowdfund/permissionless withdraw while braked",
    );
    assert_eq!(word(&read(&mut h, fund, "state()", ()), 0), n(3));
    bad(
        &mut h,
        2,
        fund,
        call("withdraw()", ()),
        U256::ZERO,
        "NothingToWithdraw()",
        "crowdfund/double withdrawal",
    );

    let failed = h.deploy(
        "toolbox/AllOrNothingCrowdfund",
        (guardian, beneficiary, n(100), n(1000)).abi_encode_params(),
    );
    h.ok(
        1,
        failed,
        call("contribute()", ()),
        n(20),
        "crowdfund/failed campaign contribution",
    );
    let deadline = word(&read(&mut h, failed, "deadline()", ()), 0).to::<u64>();
    h.at(deadline);
    bad(
        &mut h,
        1,
        failed,
        call("refund()", ()),
        U256::ZERO,
        "RefundNotAllowed(uint256,uint256,uint256)",
        "crowdfund/refund deadline boundary",
    );
    h.at(deadline + 1);
    bad(
        &mut h,
        3,
        failed,
        call("contribute()", ()),
        n(1),
        "DeadlinePassed(uint256,uint256)",
        "crowdfund/late contribution",
    );
    assert_eq!(word(&read(&mut h, failed, "state()", ()), 0), n(2));
    h.ok(
        1,
        failed,
        call("refund()", ()),
        U256::ZERO,
        "crowdfund/failed campaign full refund",
    );
    assert_eq!(
        word(
            &read(&mut h, failed, "contributions(address)", (contributor,)),
            0
        ),
        U256::ZERO
    );
    bad(
        &mut h,
        1,
        failed,
        call("refund()", ()),
        U256::ZERO,
        "NothingToRefund()",
        "crowdfund/double refund",
    );
}

#[test]
fn subscription_extension_dust_cancellation_expiry_and_brake() {
    let mut h = Harness::new();
    let guardian = h.addr(0);
    let payee = h.addr(2);
    let user = h.addr(1);
    h.deploy_revert(
        "toolbox/SubscriptionManager",
        (guardian, Address::ZERO, n(2)).abi_encode_params(),
        "subscription/zero payee",
    );
    h.deploy_revert(
        "toolbox/SubscriptionManager",
        (guardian, payee, U256::ZERO).abi_encode_params(),
        "subscription/zero rate",
    );
    let sub = h.deploy(
        "toolbox/SubscriptionManager",
        (guardian, payee, n(2)).abi_encode_params(),
    );
    bad(
        &mut h,
        1,
        sub,
        vec![],
        n(1),
        "PaymentTooSmall(uint256,uint256)",
        "subscription/direct payment",
    );
    for amount in [0, 1] {
        bad(
            &mut h,
            1,
            sub,
            call("subscribe()", ()),
            n(amount),
            "PaymentTooSmall(uint256,uint256)",
            "subscription/subsecond payment",
        );
    }
    bad(
        &mut h,
        1,
        sub,
        call("cancel()", ()),
        U256::ZERO,
        "NotSubscribed()",
        "subscription/absent cancellation",
    );
    bad(
        &mut h,
        3,
        sub,
        call("settleExpired(address)", (user,)),
        U256::ZERO,
        "NothingToSettle()",
        "subscription/absent settlement",
    );
    bad(
        &mut h,
        3,
        sub,
        call("claimRevenue()", ()),
        U256::ZERO,
        "NothingToClaim()",
        "subscription/no revenue",
    );
    h.ok(
        1,
        sub,
        call("subscribe()", ()),
        n(2001),
        "subscription/subscribe with dust",
    );
    let first_expiry = word(&read(&mut h, sub, "subscribers(address)", (user,)), 0);
    h.ok(
        1,
        sub,
        call("subscribe()", ()),
        n(2000),
        "subscription/extend active period",
    );
    assert_eq!(
        word(&read(&mut h, sub, "subscribers(address)", (user,)), 0),
        first_expiry + n(1000)
    );
    assert_eq!(word(&read(&mut h, sub, "refundReserve()", ()), 0), n(4000));
    assert_eq!(word(&read(&mut h, sub, "claimableRevenue()", ()), 0), n(1));
    h.ok(
        3,
        sub,
        call("claimRevenue()", ()),
        U256::ZERO,
        "subscription/permissionless dust payout",
    );
    bad(
        &mut h,
        3,
        sub,
        call("settleExpired(address)", (user,)),
        U256::ZERO,
        "NothingToSettle()",
        "subscription/active settlement",
    );
    brake(&mut h, sub);
    bad(
        &mut h,
        1,
        sub,
        call("subscribe()", ()),
        n(2),
        "BrakedNewEntry(uint8)",
        "subscription/braked entry",
    );
    h.ok(
        1,
        sub,
        call("cancel()", ()),
        U256::ZERO,
        "subscription/cancel while braked",
    );
    assert_eq!(
        word(&read(&mut h, sub, "refundReserve()", ()), 0),
        U256::ZERO
    );
    bad(
        &mut h,
        1,
        sub,
        call("cancel()", ()),
        U256::ZERO,
        "NotSubscribed()",
        "subscription/double cancel",
    );
    h.ok(
        3,
        sub,
        call("claimRevenue()", ()),
        U256::ZERO,
        "subscription/consumed time revenue",
    );
    let sub2 = h.deploy(
        "toolbox/SubscriptionManager",
        (guardian, payee, n(2)).abi_encode_params(),
    );
    h.ok(
        1,
        sub2,
        call("subscribe()", ()),
        n(2000),
        "subscription/expiry flow subscribe",
    );
    let expiry = word(&read(&mut h, sub2, "subscribers(address)", (user,)), 0).to::<u64>();
    h.at(expiry);
    bad(
        &mut h,
        1,
        sub2,
        call("cancel()", ()),
        U256::ZERO,
        "NotSubscribed()",
        "subscription/cancel exact expiry",
    );
    h.ok(
        3,
        sub2,
        call("settleExpired(address)", (user,)),
        U256::ZERO,
        "subscription/settle expired permissionlessly",
    );
    bad(
        &mut h,
        3,
        sub2,
        call("settleExpired(address)", (user,)),
        U256::ZERO,
        "NothingToSettle()",
        "subscription/double settle",
    );
    h.ok(
        3,
        sub2,
        call("claimRevenue()", ()),
        U256::ZERO,
        "subscription/expired revenue payout",
    );
    h.ok(
        1,
        sub2,
        call("subscribe()", ()),
        n(2000),
        "subscription/restart expired period",
    );
    let expiry = word(&read(&mut h, sub2, "subscribers(address)", (user,)), 0).to::<u64>();
    h.at(expiry + 1);
    h.ok(
        1,
        sub2,
        call("subscribe()", ()),
        n(2000),
        "subscription/subscribe auto-settles expired streak",
    );
    assert_eq!(word(&read(&mut h, sub2, "refundReserve()", ()), 0), n(2000));
    assert_eq!(
        word(&read(&mut h, sub2, "claimableRevenue()", ()), 0),
        n(2000)
    );
}

#[test]
fn agent_vending_delivery_refund_deadlines_and_every_guard() {
    let mut h = Harness::new();
    let guardian = h.addr(0);
    let agent = h.addr(2);
    let buyer = h.addr(1);
    for (who, price, window) in [
        (Address::ZERO, 100, 1000),
        (agent, 0, 1000),
        (agent, 100, 0),
    ] {
        h.deploy_revert(
            "toolbox/AgentVending",
            (guardian, who, n(price), n(window)).abi_encode_params(),
            "vending/invalid constructor",
        );
    }
    let vending = h.deploy(
        "toolbox/AgentVending",
        (guardian, agent, n(100), n(1000)).abi_encode_params(),
    );
    bad(
        &mut h,
        1,
        vending,
        vec![],
        n(1),
        "PaymentNotAllowed()",
        "vending/direct payment",
    );
    for paid in [0, 99, 101] {
        bad(
            &mut h,
            1,
            vending,
            call("order(bytes32)", (B256::ZERO,)),
            n(paid),
            "WrongPrice(uint256,uint256)",
            "vending/wrong price",
        );
    }
    bad(
        &mut h,
        2,
        vending,
        call("deliver(uint256,bytes32)", (n(999), B256::ZERO)),
        U256::ZERO,
        "UnknownOrder(uint256)",
        "vending/deliver unknown",
    );
    bad(
        &mut h,
        1,
        vending,
        call("refund(uint256)", (n(999),)),
        U256::ZERO,
        "UnknownOrder(uint256)",
        "vending/refund unknown",
    );
    h.ok(
        1,
        vending,
        call("order(bytes32)", (B256::ZERO,)),
        n(100),
        "vending/order zero commitment permitted",
    );
    let due = word(&read(&mut h, vending, "orders(uint256)", (n(1),)), 1).to::<u64>();
    bad(
        &mut h,
        1,
        vending,
        call("deliver(uint256,bytes32)", (n(1), B256::ZERO)),
        U256::ZERO,
        "NotAgent(address)",
        "vending/delivery wrong caller",
    );
    bad(
        &mut h,
        2,
        vending,
        call("refund(uint256)", (n(1),)),
        U256::ZERO,
        "NotBuyer(address)",
        "vending/refund wrong caller",
    );
    bad(
        &mut h,
        1,
        vending,
        call("refund(uint256)", (n(1),)),
        U256::ZERO,
        "StillOpen(uint256,uint256)",
        "vending/early refund",
    );
    h.at(due);
    h.ok(
        2,
        vending,
        call("deliver(uint256,bytes32)", (n(1), B256::ZERO)),
        U256::ZERO,
        "vending/delivery exact deadline",
    );
    assert_eq!(
        word(&read(&mut h, vending, "orders(uint256)", (n(1),)), 0),
        U256::ZERO
    );
    bad(
        &mut h,
        2,
        vending,
        call("deliver(uint256,bytes32)", (n(1), B256::ZERO)),
        U256::ZERO,
        "UnknownOrder(uint256)",
        "vending/double delivery",
    );
    bad(
        &mut h,
        1,
        vending,
        call("refund(uint256)", (n(1),)),
        U256::ZERO,
        "UnknownOrder(uint256)",
        "vending/refund delivered order",
    );
    h.ok(
        1,
        vending,
        call("order(bytes32)", (B256::repeat_byte(0xff),)),
        n(100),
        "vending/order max commitment",
    );
    let due = word(&read(&mut h, vending, "orders(uint256)", (n(2),)), 1).to::<u64>();
    brake(&mut h, vending);
    bad(
        &mut h,
        1,
        vending,
        call("order(bytes32)", (B256::ZERO,)),
        n(100),
        "BrakedNewEntry(uint8)",
        "vending/braked entry",
    );
    h.at(due);
    bad(
        &mut h,
        1,
        vending,
        call("refund(uint256)", (n(2),)),
        U256::ZERO,
        "StillOpen(uint256,uint256)",
        "vending/refund exact deadline",
    );
    h.at(due + 1);
    bad(
        &mut h,
        2,
        vending,
        call("deliver(uint256,bytes32)", (n(2), B256::ZERO)),
        U256::ZERO,
        "DeliveryTooLate(uint256,uint256)",
        "vending/late delivery",
    );
    h.ok(
        1,
        vending,
        call("refund(uint256)", (n(2),)),
        U256::ZERO,
        "vending/refund while fully braked",
    );
    assert_eq!(
        word(&read(&mut h, vending, "orders(uint256)", (n(2),)), 0),
        U256::ZERO
    );
    bad(
        &mut h,
        1,
        vending,
        call("refund(uint256)", (n(2),)),
        U256::ZERO,
        "UnknownOrder(uint256)",
        "vending/double refund",
    );
    assert_eq!(word(&read(&mut h, vending, "orderCount()", ()), 0), n(2));
    assert_ne!(buyer, Address::ZERO);
}

fn configure(h: &mut Harness, receiver: Address, target: Address, data: Vec<u8>) {
    h.ok(
        0,
        receiver,
        call("configure(address,bytes)", (target, Bytes::from(data))),
        U256::ZERO,
        "callback/configure",
    );
}

fn reject(h: &mut Harness, receiver: Address, reject: bool) {
    h.ok(
        0,
        receiver,
        call("setRejectPayment(bool)", (reject,)),
        U256::ZERO,
        "callback/payment policy",
    );
}

fn forward(
    h: &mut Harness,
    receiver: Address,
    target: Address,
    data: Vec<u8>,
    value: U256,
    label: &str,
) {
    let receipt = h.ok(
        0,
        receiver,
        call(
            "execute(address,bytes,uint256)",
            (target, Bytes::from(data), value),
        ),
        value,
        label,
    );
    assert_eq!(
        word(&receipt.output, 0),
        n(1),
        "{label}: nested call failed"
    );
}

fn guarded_callback(h: &mut Harness, receiver: Address) {
    assert_eq!(word(&read(h, receiver, "callbackAttempted()", ()), 0), n(1));
    assert_eq!(
        word(&read(h, receiver, "innerSuccess()", ()), 0),
        U256::ZERO
    );
}

fn forward_bad(
    h: &mut Harness,
    receiver: Address,
    target: Address,
    data: Vec<u8>,
    error: &str,
    label: &str,
) {
    let receipt = h.ok(
        0,
        receiver,
        call(
            "execute(address,bytes,uint256)",
            (target, Bytes::from(data), U256::ZERO),
        ),
        U256::ZERO,
        label,
    );
    assert_eq!(
        word(&receipt.output, 0),
        U256::ZERO,
        "{label}: nested call unexpectedly succeeded"
    );
    let offset = word(&receipt.output, 1).to::<usize>();
    let length = word(&receipt.output[offset..], 0).to::<usize>();
    assert!(length >= 4, "{label}: missing nested revert selector");
    assert_eq!(
        &receipt.output[offset + 32..offset + 36],
        &keccak256(error).as_slice()[..4],
        "{label}"
    );
}

#[test]
fn all_payment_payouts_reject_cleanly_then_resist_reentrancy() {
    let mut h = Harness::new();
    let guardian = h.addr(0);
    let receiver = h.deploy("support/NativeCallback", vec![]);
    let book = h.deploy(
        "toolbox/InvoiceBook",
        (guardian, receiver).abi_encode_params(),
    );
    forward(
        &mut h,
        receiver,
        book,
        call(
            "issue(uint128,uint48,string)",
            (n(100), n(10000), "callback payee"),
        ),
        U256::ZERO,
        "invoice/contract payee issue",
    );
    reject(&mut h, receiver, true);
    bad(
        &mut h,
        1,
        book,
        call("settle(uint256)", (n(1),)),
        n(100),
        "SettleTransferFailed()",
        "invoice/rejecting payee rolls back",
    );
    assert_eq!(
        word(&read(&mut h, book, "invoices(uint256)", (n(1),)), 0),
        n(100)
    );
    reject(&mut h, receiver, false);
    configure(&mut h, receiver, book, call("settle(uint256)", (n(1),)));
    let before = h.state.balance(&receiver);
    h.ok(
        1,
        book,
        call("settle(uint256)", (n(1),)),
        n(100),
        "invoice/payee reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + n(100));
    assert_eq!(h.state.balance(&book), U256::ZERO);

    let escrow = h.deploy("toolbox/MilestoneEscrow", (guardian,).abi_encode_params());
    h.ok(
        1,
        escrow,
        call("createDeal(address,uint64)", (receiver, n(1))),
        n(100),
        "escrow/contract seller",
    );
    h.ok(
        1,
        escrow,
        call(
            "approveMilestone(uint256,uint64,uint256)",
            (n(1), n(0), n(100)),
        ),
        U256::ZERO,
        "escrow/approve contract seller",
    );
    reject(&mut h, receiver, true);
    forward_bad(
        &mut h,
        receiver,
        escrow,
        call("sellerWithdraw(uint256)", (n(1),)),
        "NativeTransferFailed()",
        "escrow/rejecting seller",
    );
    assert_eq!(
        word(&read(&mut h, escrow, "dealInfo(uint256)", (n(1),)), 3),
        n(100)
    );
    reject(&mut h, receiver, false);
    configure(
        &mut h,
        receiver,
        escrow,
        call("sellerWithdraw(uint256)", (n(1),)),
    );
    let before = h.state.balance(&receiver);
    forward(
        &mut h,
        receiver,
        escrow,
        call("sellerWithdraw(uint256)", (n(1),)),
        U256::ZERO,
        "escrow/seller reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + n(100));
    assert_eq!(h.state.balance(&escrow), U256::ZERO);

    let fund = h.deploy(
        "toolbox/AllOrNothingCrowdfund",
        (guardian, receiver, n(100), n(10000)).abi_encode_params(),
    );
    h.ok(
        1,
        fund,
        call("contribute()", ()),
        n(100),
        "crowdfund/contract beneficiary",
    );
    reject(&mut h, receiver, true);
    bad(
        &mut h,
        2,
        fund,
        call("withdraw()", ()),
        U256::ZERO,
        "NativeTransferFailed()",
        "crowdfund/rejecting beneficiary",
    );
    assert_eq!(word(&read(&mut h, fund, "state()", ()), 0), n(1));
    reject(&mut h, receiver, false);
    configure(&mut h, receiver, fund, call("withdraw()", ()));
    let before = h.state.balance(&receiver);
    h.ok(
        2,
        fund,
        call("withdraw()", ()),
        U256::ZERO,
        "crowdfund/beneficiary reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + n(100));
    assert_eq!(h.state.balance(&fund), U256::ZERO);

    let sub = h.deploy(
        "toolbox/SubscriptionManager",
        (guardian, receiver, n(2)).abi_encode_params(),
    );
    h.ok(
        1,
        sub,
        call("subscribe()", ()),
        n(2001),
        "subscription/contract payee",
    );
    reject(&mut h, receiver, true);
    bad(
        &mut h,
        2,
        sub,
        call("claimRevenue()", ()),
        U256::ZERO,
        "PayoutFailed()",
        "subscription/rejecting payee",
    );
    reject(&mut h, receiver, false);
    configure(&mut h, receiver, sub, call("claimRevenue()", ()));
    let before = h.state.balance(&receiver);
    h.ok(
        2,
        sub,
        call("claimRevenue()", ()),
        U256::ZERO,
        "subscription/payee reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + n(1));
    assert_eq!(h.state.balance(&sub), n(2000));

    let vending = h.deploy(
        "toolbox/AgentVending",
        (guardian, receiver, n(100), n(10000)).abi_encode_params(),
    );
    h.ok(
        1,
        vending,
        call("order(bytes32)", (B256::ZERO,)),
        n(100),
        "vending/contract agent",
    );
    reject(&mut h, receiver, true);
    forward_bad(
        &mut h,
        receiver,
        vending,
        call("deliver(uint256,bytes32)", (n(1), B256::ZERO)),
        "PayoutFailed()",
        "vending/rejecting agent",
    );
    assert_ne!(
        word(&read(&mut h, vending, "orders(uint256)", (n(1),)), 0),
        U256::ZERO
    );
    reject(&mut h, receiver, false);
    configure(
        &mut h,
        receiver,
        vending,
        call("deliver(uint256,bytes32)", (n(1), B256::ZERO)),
    );
    let before = h.state.balance(&receiver);
    forward(
        &mut h,
        receiver,
        vending,
        call("deliver(uint256,bytes32)", (n(1), B256::ZERO)),
        U256::ZERO,
        "vending/agent reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + n(100));
    assert_eq!(h.state.balance(&vending), U256::ZERO);
}

#[test]
fn native_refund_receivers_cannot_reenter_and_rejection_preserves_principal() {
    let mut h = Harness::new();
    let guardian = h.addr(0);
    let seller = h.addr(2);
    let receiver = h.deploy("support/NativeCallback", vec![]);
    let escrow = h.deploy("toolbox/MilestoneEscrow", (guardian,).abi_encode_params());
    forward(
        &mut h,
        receiver,
        escrow,
        call("createDeal(address,uint64)", (seller, n(1))),
        n(100),
        "escrow/contract buyer creates",
    );
    reject(&mut h, receiver, true);
    forward_bad(
        &mut h,
        receiver,
        escrow,
        call("buyerRefund(uint256)", (n(1),)),
        "NativeTransferFailed()",
        "escrow/rejecting buyer rollback",
    );
    assert_eq!(
        word(&read(&mut h, escrow, "dealInfo(uint256)", (n(1),)), 2),
        n(100)
    );
    assert_eq!(h.state.balance(&escrow), n(100));
    reject(&mut h, receiver, false);
    configure(
        &mut h,
        receiver,
        escrow,
        call("buyerRefund(uint256)", (n(1),)),
    );
    let before = h.state.balance(&receiver);
    forward(
        &mut h,
        receiver,
        escrow,
        call("buyerRefund(uint256)", (n(1),)),
        U256::ZERO,
        "escrow/buyer reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + n(100));
    assert_eq!(h.state.balance(&escrow), U256::ZERO);

    let fund = h.deploy(
        "toolbox/AllOrNothingCrowdfund",
        (guardian, seller, n(100), n(1000)).abi_encode_params(),
    );
    forward(
        &mut h,
        receiver,
        fund,
        call("contribute()", ()),
        n(20),
        "crowdfund/contract contributor",
    );
    let deadline = word(&read(&mut h, fund, "deadline()", ()), 0).to::<u64>();
    h.at(deadline + 1);
    reject(&mut h, receiver, true);
    forward_bad(
        &mut h,
        receiver,
        fund,
        call("refund()", ()),
        "NativeTransferFailed()",
        "crowdfund/rejecting contributor rollback",
    );
    assert_eq!(
        word(
            &read(&mut h, fund, "contributions(address)", (receiver,)),
            0
        ),
        n(20)
    );
    reject(&mut h, receiver, false);
    configure(&mut h, receiver, fund, call("refund()", ()));
    let before = h.state.balance(&receiver);
    forward(
        &mut h,
        receiver,
        fund,
        call("refund()", ()),
        U256::ZERO,
        "crowdfund/contributor reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + n(20));
    assert_eq!(h.state.balance(&fund), U256::ZERO);

    let sub = h.deploy(
        "toolbox/SubscriptionManager",
        (guardian, seller, n(2)).abi_encode_params(),
    );
    forward(
        &mut h,
        receiver,
        sub,
        call("subscribe()", ()),
        n(2000),
        "subscription/contract subscriber",
    );
    reject(&mut h, receiver, true);
    forward_bad(
        &mut h,
        receiver,
        sub,
        call("cancel()", ()),
        "RefundFailed()",
        "subscription/rejecting subscriber rollback",
    );
    assert_eq!(word(&read(&mut h, sub, "refundReserve()", ()), 0), n(2000));
    reject(&mut h, receiver, false);
    configure(&mut h, receiver, sub, call("cancel()", ()));
    let expiry = word(&read(&mut h, sub, "subscribers(address)", (receiver,)), 0).to::<u64>();
    let expected_refund = n((expiry - h.timestamp()) * 2);
    let before = h.state.balance(&receiver);
    forward(
        &mut h,
        receiver,
        sub,
        call("cancel()", ()),
        U256::ZERO,
        "subscription/subscriber reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + expected_refund);
    assert_eq!(
        word(&read(&mut h, sub, "refundReserve()", ()), 0),
        U256::ZERO
    );

    let vending = h.deploy(
        "toolbox/AgentVending",
        (guardian, seller, n(100), n(1000)).abi_encode_params(),
    );
    forward(
        &mut h,
        receiver,
        vending,
        call("order(bytes32)", (B256::ZERO,)),
        n(100),
        "vending/contract buyer orders",
    );
    let due = word(&read(&mut h, vending, "orders(uint256)", (n(1),)), 1).to::<u64>();
    h.at(due + 1);
    reject(&mut h, receiver, true);
    forward_bad(
        &mut h,
        receiver,
        vending,
        call("refund(uint256)", (n(1),)),
        "PayoutFailed()",
        "vending/rejecting buyer rollback",
    );
    assert_eq!(h.state.balance(&vending), n(100));
    reject(&mut h, receiver, false);
    configure(&mut h, receiver, vending, call("refund(uint256)", (n(1),)));
    let before = h.state.balance(&receiver);
    forward(
        &mut h,
        receiver,
        vending,
        call("refund(uint256)", (n(1),)),
        U256::ZERO,
        "vending/buyer reentrancy denied",
    );
    guarded_callback(&mut h, receiver);
    assert_eq!(h.state.balance(&receiver), before + n(100));
    assert_eq!(h.state.balance(&vending), U256::ZERO);
}

#[test]
fn subscription_payment_width_bounds_preserve_principal() {
    let mut h = Harness::new();
    let guardian = h.addr(0);
    let payee = h.addr(2);
    let user = h.addr(1);
    let sub = h.deploy(
        "toolbox/SubscriptionManager",
        (guardian, payee, n(1)).abi_encode_params(),
    );
    bad(
        &mut h,
        1,
        sub,
        call("subscribe()", ()),
        U256::from(1u128 << 64),
        "AmountTooLarge()",
        "subscription/uint64 duration overflow regression",
    );
    assert_eq!(
        word(&read(&mut h, sub, "refundReserve()", ()), 0),
        U256::ZERO
    );
    let near_expiry_limit = U256::from(u64::MAX - h.timestamp());
    h.ok(
        1,
        sub,
        call("subscribe()", ()),
        near_expiry_limit,
        "subscription/exact uint64 expiry boundary",
    );
    assert_eq!(
        word(&read(&mut h, sub, "subscribers(address)", (user,)), 0),
        U256::from(u64::MAX)
    );
    bad(
        &mut h,
        1,
        sub,
        call("subscribe()", ()),
        n(1),
        "AmountTooLarge()",
        "subscription/uint64 cumulative expiry overflow regression",
    );

    let rate = U256::from(1u128 << 32);
    let sub2 = h.deploy(
        "toolbox/SubscriptionManager",
        (guardian, payee, rate).abi_encode_params(),
    );
    let overflow = U256::from(1u128 << 88);
    bad(
        &mut h,
        1,
        sub2,
        call("subscribe()", ()),
        overflow,
        "AmountTooLarge()",
        "subscription/uint88 principal overflow regression",
    );
    assert_eq!(h.state.balance(&sub2), U256::ZERO);
    h.ok(
        1,
        sub2,
        call("subscribe()", ()),
        overflow - rate,
        "subscription/principal immediately below uint88 bound",
    );
    bad(
        &mut h,
        1,
        sub2,
        call("subscribe()", ()),
        rate,
        "AmountTooLarge()",
        "subscription/uint88 cumulative principal overflow regression",
    );
    assert_eq!(
        word(&read(&mut h, sub2, "refundReserve()", ()), 0),
        overflow - rate
    );
    let principal_max = overflow - n(1);
    let sub3 = h.deploy(
        "toolbox/SubscriptionManager",
        (guardian, payee, principal_max).abi_encode_params(),
    );
    h.ok(
        1,
        sub3,
        call("subscribe()", ()),
        principal_max,
        "subscription/exact uint88 principal boundary",
    );
    assert_eq!(
        word(&read(&mut h, sub3, "subscribers(address)", (user,)), 1),
        principal_max
    );
}
