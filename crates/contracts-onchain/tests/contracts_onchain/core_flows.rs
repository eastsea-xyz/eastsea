//! Escrow and distribution entry points, through signed paid-state blocks.
use super::harness::*;
use alloy_primitives::keccak256;
use alloy_sol_types::{sol, SolCall, SolValue};

sol! {
    interface Swap {
        function lock(address recipient, bytes32 hashlock, uint64 timelock, address token, uint256 amount) external payable returns (uint256);
        function claim(uint256 id, bytes preimage) external;
        function refund(uint256 id) external;
        function swapCount() external view returns (uint256);
    }
    interface Token {
        function mint(address to, uint256 amount) external;
        function approve(address spender, uint256 amount) external returns (bool);
        function transfer(address to, uint256 amount) external returns (bool);
        function balanceOf(address who) external view returns (uint256);
        function setFeeBps(uint16 feeBps) external;
        function setFailTransfers(bool fail) external;
        function configureCallback(address target, bytes input, bool enabled) external;
        function callbackAttempted() external view returns (bool);
        function innerSuccess() external view returns (bool);
        function innerOutput() external view returns (bytes);
    }
    interface Callback {
        function configure(address target, bytes input) external;
        function setRejectPayment(bool reject) external;
        function callbackAttempted() external view returns (bool);
        function innerSuccess() external view returns (bool);
    }
    interface Locker {
        function lock(address token, address beneficiary, uint256 amount, uint64 unlockAt) external returns (uint256);
        function extend(uint256 id, uint64 newUnlockAt) external;
        function withdraw(uint256 id) external;
        function lockCount() external view returns (uint256);
        function lockedTotal(address beneficiary, address token) external view returns (uint256);
    }
    interface Vesting {
        function create(address token, address beneficiary, uint256 amount, uint64 start, uint64 cliff, uint64 duration, bool cancelable) external returns (uint256);
        function claim(uint256 id) external;
        function cancel(uint256 id) external;
        function claimable(uint256 id) external view returns (uint256);
        function streamCount() external view returns (uint256);
    }
    interface Distributor {
        function claim(uint256 index, address account, uint256 amount, bytes32[] proof) external;
        function sweep() external;
        function isClaimed(uint256 index) external view returns (bool);
    }
    interface Factory {
        function create(address token, bytes32 merkleRoot, uint64 ends, uint256 total) external returns (address);
        function count() external view returns (uint256);
        function campaigns(uint256 index) external view returns (address);
    }
    interface Batch {
        function send(address token, address[] to, uint256[] amounts) external;
    }
}

fn zero() -> U256 {
    U256::ZERO
}
fn u(n: u64) -> U256 {
    U256::from(n)
}
fn uint(h: &mut Harness, to: Address, input: Vec<u8>) -> U256 {
    U256::abi_decode(&h.view(0, to, input)).unwrap()
}
fn boolean(h: &mut Harness, to: Address, input: Vec<u8>) -> bool {
    bool::abi_decode(&h.view(0, to, input)).unwrap()
}
fn guard_revert(h: &mut Harness, token: Address) {
    let output = Token::innerOutputCall::abi_decode_returns(&h.view(
        0,
        token,
        Token::innerOutputCall {}.abi_encode(),
    ))
    .unwrap();
    assert!(output.len() >= 4, "missing callback revert output");
    assert_eq!(&output[..4], &keccak256("Reentered()")[..4]);
}
fn balance(h: &mut Harness, token: Address, who: Address) -> U256 {
    uint(h, token, Token::balanceOfCall { who }.abi_encode())
}
fn error(
    h: &mut Harness,
    actor: u8,
    to: Address,
    input: Vec<u8>,
    value: U256,
    sig: &str,
    label: &str,
) {
    let receipt = h.revert(actor, to, input, value, label);
    assert!(
        receipt.output.len() >= 4,
        "{label}: missing revert selector"
    );
    assert_eq!(
        &receipt.output[..4],
        &keccak256(sig.as_bytes())[..4],
        "{label}"
    );
}
fn token(h: &mut Harness, spender: Address, fee: u16) -> Address {
    let token = h.deploy("support/TestToken", (fee,).abi_encode_params());
    let owner = h.addr(0);
    h.ok(
        0,
        token,
        Token::mintCall {
            to: owner,
            amount: u(1_000_000),
        }
        .abi_encode(),
        zero(),
        "token/mint",
    );
    h.ok(
        0,
        token,
        Token::approveCall {
            spender,
            amount: U256::MAX,
        }
        .abi_encode(),
        zero(),
        "token/approve",
    );
    token
}
fn secret_hash() -> B256 {
    // SHA-256("secret"), used by the contract's SHA-256 precompile.
    "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b"
        .parse()
        .unwrap()
}
fn lock_swap(recipient: Address, deadline: u64, token: Address, amount: U256) -> Vec<u8> {
    Swap::lockCall {
        recipient,
        hashlock: secret_hash(),
        timelock: deadline,
        token,
        amount,
    }
    .abi_encode()
}

fn atomic_swap_lifecycle(artifact: &str) {
    let mut h = Harness::new();
    let swap = h.deploy(artifact, vec![]);
    let recipient = h.addr(1);
    let deadline = h.timestamp() + 1_000;
    for (recipient, hashlock, timelock, amount, value, label) in [
        (
            Address::ZERO,
            secret_hash(),
            deadline,
            u(100),
            u(100),
            "swap/zero-recipient",
        ),
        (
            swap,
            secret_hash(),
            deadline,
            u(100),
            u(100),
            "swap/self-recipient",
        ),
        (
            recipient,
            B256::ZERO,
            deadline,
            u(100),
            u(100),
            "swap/zero-hashlock",
        ),
        (
            recipient,
            secret_hash(),
            0,
            u(100),
            u(100),
            "swap/expired-lock",
        ),
        (
            recipient,
            secret_hash(),
            deadline,
            zero(),
            zero(),
            "swap/zero-amount",
        ),
        (
            recipient,
            secret_hash(),
            deadline,
            u(100),
            u(99),
            "swap/value-mismatch",
        ),
        (
            recipient,
            secret_hash(),
            deadline,
            U256::MAX,
            zero(),
            "swap/max-value-mismatch",
        ),
    ] {
        error(
            &mut h,
            0,
            swap,
            Swap::lockCall {
                recipient,
                hashlock,
                timelock,
                token: Address::ZERO,
                amount,
            }
            .abi_encode(),
            value,
            "BadLock()",
            label,
        );
    }
    error(
        &mut h,
        2,
        swap,
        Swap::claimCall {
            id: U256::MAX,
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "UnknownSwap()",
        "swap/unknown-claim",
    );
    error(
        &mut h,
        2,
        swap,
        Swap::refundCall { id: zero() }.abi_encode(),
        zero(),
        "UnknownSwap()",
        "swap/unknown-refund",
    );
    h.ok(
        0,
        swap,
        lock_swap(recipient, deadline, Address::ZERO, u(100)),
        u(100),
        "swap/native-lock",
    );
    error(
        &mut h,
        2,
        swap,
        Swap::claimCall {
            id: zero(),
            preimage: Bytes::from_static(b"wrong"),
        }
        .abi_encode(),
        zero(),
        "WrongPreimage()",
        "swap/wrong-preimage",
    );
    error(
        &mut h,
        2,
        swap,
        Swap::refundCall { id: zero() }.abi_encode(),
        zero(),
        "NotExpired()",
        "swap/early-refund",
    );
    let before = h.state.balance(&recipient);
    h.ok(
        2,
        swap,
        Swap::claimCall {
            id: zero(),
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "swap/sponsored-claim",
    );
    assert_eq!(h.state.balance(&recipient), before + u(100));
    error(
        &mut h,
        2,
        swap,
        Swap::claimCall {
            id: zero(),
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "AlreadySettled()",
        "swap/double-claim",
    );
    error(
        &mut h,
        2,
        swap,
        Swap::refundCall { id: zero() }.abi_encode(),
        zero(),
        "AlreadySettled()",
        "swap/refund-claimed",
    );
    h.ok(
        0,
        swap,
        lock_swap(recipient, deadline, Address::ZERO, u(100)),
        u(100),
        "swap/native-refund-lock",
    );
    h.at(deadline);
    error(
        &mut h,
        2,
        swap,
        Swap::claimCall {
            id: u(1),
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "Expired()",
        "swap/claim-at-deadline",
    );
    let sender = h.addr(0);
    let before = h.state.balance(&sender);
    h.ok(
        2,
        swap,
        Swap::refundCall { id: u(1) }.abi_encode(),
        zero(),
        "swap/permissionless-refund",
    );
    assert_eq!(h.state.balance(&sender), before + u(100));
    error(
        &mut h,
        2,
        swap,
        Swap::refundCall { id: u(1) }.abi_encode(),
        zero(),
        "AlreadySettled()",
        "swap/double-refund",
    );
    assert_eq!(
        uint(&mut h, swap, Swap::swapCountCall {}.abi_encode()),
        u(2)
    );

    let t = token(&mut h, swap, 0);
    let deadline = h.timestamp() + 1_000;
    error(
        &mut h,
        0,
        swap,
        lock_swap(recipient, deadline, t, u(100)),
        u(1),
        "BadLock()",
        "swap/token-with-native-value",
    );
    let eoa = h.addr(3);
    error(
        &mut h,
        0,
        swap,
        lock_swap(recipient, deadline, eoa, u(100)),
        zero(),
        "UnsupportedToken()",
        "swap/eoa-token",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "swap/token-fail-on",
    );
    error(
        &mut h,
        0,
        swap,
        lock_swap(recipient, deadline, t, u(100)),
        zero(),
        "TransferFailed()",
        "swap/transfer-from-failure",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "swap/token-fail-off",
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 100 }.abi_encode(),
        zero(),
        "swap/deposit-fee-on",
    );
    error(
        &mut h,
        0,
        swap,
        lock_swap(recipient, deadline, t, u(100)),
        zero(),
        "UnsupportedToken()",
        "swap/fee-token-deposit",
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 0 }.abi_encode(),
        zero(),
        "swap/deposit-fee-off",
    );
    h.ok(
        0,
        swap,
        lock_swap(recipient, deadline, t, u(100)),
        zero(),
        "swap/token-lock",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "swap/token-payout-fail-on",
    );
    error(
        &mut h,
        2,
        swap,
        Swap::claimCall {
            id: u(2),
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "TransferFailed()",
        "swap/token-payout-refused",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "swap/token-payout-fail-off",
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 100 }.abi_encode(),
        zero(),
        "swap/payout-fee-on",
    );
    error(
        &mut h,
        2,
        swap,
        Swap::claimCall {
            id: u(2),
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "UnsupportedToken()",
        "swap/fee-token-payout",
    );
    assert_eq!(balance(&mut h, t, swap), u(100));
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 0 }.abi_encode(),
        zero(),
        "swap/payout-fee-off",
    );
    h.ok(
        2,
        swap,
        Swap::claimCall {
            id: u(2),
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "swap/token-claim",
    );
    assert_eq!(balance(&mut h, t, recipient), u(100));
    h.ok(
        0,
        swap,
        lock_swap(recipient, deadline, t, u(100)),
        zero(),
        "swap/token-refund-lock",
    );
    h.at(deadline);
    h.ok(
        2,
        swap,
        Swap::refundCall { id: u(3) }.abi_encode(),
        zero(),
        "swap/token-refund",
    );
    assert_eq!(balance(&mut h, t, swap), zero());
}

#[test]
fn atomic_swap_all_entry_points() {
    atomic_swap_lifecycle("core/AtomicSwap");
}

#[test]
fn atomic_swap_evm_same_entry_points() {
    atomic_swap_lifecycle("core/AtomicSwapEVM");
}

#[test]
fn atomic_swap_native_payout_callback_cannot_settle_twice() {
    let mut h = Harness::new();
    let swap = h.deploy("core/AtomicSwap", vec![]);
    let receiver = h.deploy("support/NativeCallback", vec![]);
    h.ok(
        0,
        receiver,
        Callback::configureCall {
            target: swap,
            input: Swap::claimCall {
                id: zero(),
                preimage: Bytes::from_static(b"secret"),
            }
            .abi_encode()
            .into(),
        }
        .abi_encode(),
        zero(),
        "swap/callback-configure",
    );
    let deadline = h.timestamp() + 100;
    h.ok(
        0,
        swap,
        lock_swap(receiver, deadline, Address::ZERO, u(100)),
        u(100),
        "swap/callback-lock",
    );
    h.ok(
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: true }.abi_encode(),
        zero(),
        "swap/reject-payment-on",
    );
    error(
        &mut h,
        2,
        swap,
        Swap::claimCall {
            id: zero(),
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "TransferFailed()",
        "swap/native-payment-refused",
    );
    assert_eq!(h.state.balance(&swap), u(100));
    h.ok(
        0,
        receiver,
        Callback::setRejectPaymentCall { reject: false }.abi_encode(),
        zero(),
        "swap/reject-payment-off",
    );
    h.ok(
        2,
        swap,
        Swap::claimCall {
            id: zero(),
            preimage: Bytes::from_static(b"secret"),
        }
        .abi_encode(),
        zero(),
        "swap/callback-claim",
    );
    assert!(boolean(
        &mut h,
        receiver,
        Callback::callbackAttemptedCall {}.abi_encode()
    ));
    assert!(!boolean(
        &mut h,
        receiver,
        Callback::innerSuccessCall {}.abi_encode()
    ));
    assert_eq!(h.state.balance(&receiver), u(100));
    assert_eq!(h.state.balance(&swap), zero());
}

#[test]
fn token_locker_permissions_expiry_fee_token_and_guard() {
    let mut h = Harness::new();
    let locker = h.deploy("core/TokenLocker", vec![]);
    let t = token(&mut h, locker, 100);
    let beneficiary = h.addr(1);
    let unlock = h.timestamp() + 100;
    error(
        &mut h,
        0,
        locker,
        Locker::lockCall {
            token: t,
            beneficiary: Address::ZERO,
            amount: u(100),
            unlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "BadLock()",
        "locker/zero-beneficiary",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::lockCall {
            token: t,
            beneficiary,
            amount: u(100),
            unlockAt: 0,
        }
        .abi_encode(),
        zero(),
        "BadLock()",
        "locker/expired-lock",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::lockCall {
            token: t,
            beneficiary,
            amount: zero(),
            unlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "NothingArrived()",
        "locker/zero-deposit",
    );
    let eoa = h.addr(4);
    error(
        &mut h,
        0,
        locker,
        Locker::lockCall {
            token: eoa,
            beneficiary,
            amount: u(1),
            unlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "BadToken()",
        "locker/eoa-token",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "locker/token-fail-on",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::lockCall {
            token: t,
            beneficiary,
            amount: u(100),
            unlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "TokenTransferFailed()",
        "locker/token-refuses-deposit",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "locker/token-fail-off",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::lockCall {
            token: t,
            beneficiary,
            amount: U256::MAX,
            unlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "TokenTransferFailed()",
        "locker/max-unfunded-deposit",
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 10_000 }.abi_encode(),
        zero(),
        "locker/total-fee-on",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::lockCall {
            token: t,
            beneficiary,
            amount: u(100),
            unlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "NothingArrived()",
        "locker/total-fee-deposit",
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 100 }.abi_encode(),
        zero(),
        "locker/normal-fee-restore",
    );
    h.ok(
        0,
        locker,
        Locker::lockCall {
            token: t,
            beneficiary,
            amount: u(100),
            unlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "locker/fee-token-lock",
    );
    assert_eq!(
        uint(
            &mut h,
            locker,
            Locker::lockedTotalCall {
                beneficiary,
                token: t
            }
            .abi_encode()
        ),
        u(99)
    );
    error(
        &mut h,
        1,
        locker,
        Locker::withdrawCall { id: U256::MAX }.abi_encode(),
        zero(),
        "Unknown()",
        "locker/unknown-withdraw",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::extendCall {
            id: U256::MAX,
            newUnlockAt: unlock + 1,
        }
        .abi_encode(),
        zero(),
        "Unknown()",
        "locker/unknown-extend",
    );
    error(
        &mut h,
        2,
        locker,
        Locker::extendCall {
            id: zero(),
            newUnlockAt: unlock + 1,
        }
        .abi_encode(),
        zero(),
        "OnlyCreatorOrBeneficiary()",
        "locker/stranger-extend",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::extendCall {
            id: zero(),
            newUnlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "CannotShorten(uint64)",
        "locker/equal-extension",
    );
    error(
        &mut h,
        1,
        locker,
        Locker::extendCall {
            id: zero(),
            newUnlockAt: unlock - 1,
        }
        .abi_encode(),
        zero(),
        "CannotShorten(uint64)",
        "locker/shorten",
    );
    h.ok(
        0,
        locker,
        Locker::extendCall {
            id: zero(),
            newUnlockAt: unlock + 10,
        }
        .abi_encode(),
        zero(),
        "locker/creator-extend",
    );
    h.ok(
        1,
        locker,
        Locker::extendCall {
            id: zero(),
            newUnlockAt: unlock + 20,
        }
        .abi_encode(),
        zero(),
        "locker/beneficiary-extend",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::withdrawCall { id: zero() }.abi_encode(),
        zero(),
        "OnlyBeneficiary()",
        "locker/wrong-withdrawer",
    );
    error(
        &mut h,
        1,
        locker,
        Locker::withdrawCall { id: zero() }.abi_encode(),
        zero(),
        "StillLocked(uint64)",
        "locker/early-withdraw",
    );
    h.at(unlock + 20);
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "locker/payout-fail-on",
    );
    error(
        &mut h,
        1,
        locker,
        Locker::withdrawCall { id: zero() }.abi_encode(),
        zero(),
        "TokenTransferFailed()",
        "locker/token-refuses-payout",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "locker/payout-fail-off",
    );
    h.ok(
        1,
        locker,
        Locker::withdrawCall { id: zero() }.abi_encode(),
        zero(),
        "locker/withdraw",
    );
    assert_eq!(balance(&mut h, t, beneficiary), u(99)); // 1% of 99 truncates to zero.
    error(
        &mut h,
        1,
        locker,
        Locker::withdrawCall { id: zero() }.abi_encode(),
        zero(),
        "AlreadyWithdrawn(uint256)",
        "locker/double-withdraw",
    );
    error(
        &mut h,
        0,
        locker,
        Locker::extendCall {
            id: zero(),
            newUnlockAt: u64::MAX,
        }
        .abi_encode(),
        zero(),
        "AlreadyWithdrawn(uint256)",
        "locker/extend-withdrawn",
    );

    let unlock = h.timestamp() + 100;
    h.ok(
        0,
        t,
        Token::configureCallbackCall {
            target: locker,
            input: Locker::withdrawCall { id: zero() }.abi_encode().into(),
            enabled: true,
        }
        .abi_encode(),
        zero(),
        "locker/guard-configure",
    );
    h.ok(
        0,
        locker,
        Locker::lockCall {
            token: t,
            beneficiary,
            amount: u(100),
            unlockAt: unlock,
        }
        .abi_encode(),
        zero(),
        "locker/guard-deposit",
    );
    assert!(boolean(
        &mut h,
        t,
        Token::callbackAttemptedCall {}.abi_encode()
    ));
    assert!(!boolean(&mut h, t, Token::innerSuccessCall {}.abi_encode()));
    guard_revert(&mut h, t);
    assert_eq!(
        uint(&mut h, locker, Locker::lockCountCall {}.abi_encode()),
        u(2)
    );
    assert_eq!(balance(&mut h, t, locker), u(99));
}

#[test]
fn token_vesting_cliff_claim_cancel_permissions_and_guard() {
    let mut h = Harness::new();
    let vesting = h.deploy("core/TokenVesting", vec![]);
    let t = token(&mut h, vesting, 100);
    let beneficiary = h.addr(1);
    let start = h.timestamp() + 100;
    for (beneficiary, cliff, duration, label) in [
        (Address::ZERO, 0, 100, "vesting/zero-beneficiary"),
        (beneficiary, 0, 0, "vesting/zero-duration"),
        (beneficiary, 101, 100, "vesting/cliff-after-end"),
    ] {
        error(
            &mut h,
            0,
            vesting,
            Vesting::createCall {
                token: t,
                beneficiary,
                amount: u(1_000),
                start,
                cliff,
                duration,
                cancelable: false,
            }
            .abi_encode(),
            zero(),
            "BadStream()",
            label,
        );
    }
    error(
        &mut h,
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: zero(),
            start,
            cliff: 0,
            duration: 100,
            cancelable: false,
        }
        .abi_encode(),
        zero(),
        "NothingArrived()",
        "vesting/zero-deposit",
    );
    // uint64 addition must revert cleanly even after the ERC-20 pull.
    error(
        &mut h,
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: u(1_000),
            start: u64::MAX,
            cliff: 1,
            duration: 1,
            cancelable: true,
        }
        .abi_encode(),
        zero(),
        "Panic(uint256)",
        "vesting/time-overflow",
    );
    let eoa = h.addr(4);
    error(
        &mut h,
        0,
        vesting,
        Vesting::createCall {
            token: eoa,
            beneficiary,
            amount: u(1_000),
            start,
            cliff: 0,
            duration: 100,
            cancelable: false,
        }
        .abi_encode(),
        zero(),
        "BadToken()",
        "vesting/eoa-token",
    );
    error(
        &mut h,
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: U256::MAX,
            start,
            cliff: 0,
            duration: 100,
            cancelable: false,
        }
        .abi_encode(),
        zero(),
        "TokenTransferFailed()",
        "vesting/max-unfunded-deposit",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "vesting/deposit-fail-on",
    );
    error(
        &mut h,
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: u(1_000),
            start,
            cliff: 0,
            duration: 100,
            cancelable: false,
        }
        .abi_encode(),
        zero(),
        "TokenTransferFailed()",
        "vesting/token-refuses-deposit",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "vesting/deposit-fail-off",
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 10_000 }.abi_encode(),
        zero(),
        "vesting/total-fee-on",
    );
    error(
        &mut h,
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: u(1_000),
            start,
            cliff: 0,
            duration: 100,
            cancelable: false,
        }
        .abi_encode(),
        zero(),
        "NothingArrived()",
        "vesting/total-fee-deposit",
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 100 }.abi_encode(),
        zero(),
        "vesting/normal-fee-restore",
    );
    h.ok(
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: u(1_000),
            start,
            cliff: 50,
            duration: 100,
            cancelable: false,
        }
        .abi_encode(),
        zero(),
        "vesting/fee-token-stream",
    );
    assert_eq!(balance(&mut h, t, vesting), u(990));
    error(
        &mut h,
        1,
        vesting,
        Vesting::claimCall { id: U256::MAX }.abi_encode(),
        zero(),
        "Unknown()",
        "vesting/unknown-claim",
    );
    error(
        &mut h,
        0,
        vesting,
        Vesting::cancelCall { id: U256::MAX }.abi_encode(),
        zero(),
        "Unknown()",
        "vesting/unknown-cancel",
    );
    error(
        &mut h,
        0,
        vesting,
        Vesting::claimCall { id: zero() }.abi_encode(),
        zero(),
        "OnlyBeneficiary()",
        "vesting/wrong-claimant",
    );
    error(
        &mut h,
        1,
        vesting,
        Vesting::claimCall { id: zero() }.abi_encode(),
        zero(),
        "NothingToClaim()",
        "vesting/before-cliff",
    );
    error(
        &mut h,
        1,
        vesting,
        Vesting::cancelCall { id: zero() }.abi_encode(),
        zero(),
        "OnlyCreator()",
        "vesting/wrong-canceller",
    );
    error(
        &mut h,
        0,
        vesting,
        Vesting::cancelCall { id: zero() }.abi_encode(),
        zero(),
        "NotCancelable()",
        "vesting/immutable-stream",
    );
    h.at(start + 49);
    error(
        &mut h,
        1,
        vesting,
        Vesting::claimCall { id: zero() }.abi_encode(),
        zero(),
        "NothingToClaim()",
        "vesting/one-second-before-cliff",
    );
    h.at(start + 50);
    h.ok(
        1,
        vesting,
        Vesting::claimCall { id: zero() }.abi_encode(),
        zero(),
        "vesting/cliff-claim",
    );
    assert_eq!(balance(&mut h, t, beneficiary), u(491)); // 495 minus 4 transfer fee.
    h.at(start + 100);
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "vesting/payout-fail-on",
    );
    error(
        &mut h,
        1,
        vesting,
        Vesting::claimCall { id: zero() }.abi_encode(),
        zero(),
        "TokenTransferFailed()",
        "vesting/token-refuses-claim",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "vesting/payout-fail-off",
    );
    h.ok(
        1,
        vesting,
        Vesting::claimCall { id: zero() }.abi_encode(),
        zero(),
        "vesting/final-claim",
    );
    assert_eq!(balance(&mut h, t, beneficiary), u(982));
    error(
        &mut h,
        1,
        vesting,
        Vesting::claimCall { id: zero() }.abi_encode(),
        zero(),
        "NothingToClaim()",
        "vesting/double-final-claim",
    );

    let start = h.timestamp() + 20;
    h.ok(
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: u(1_000),
            start,
            cliff: 0,
            duration: 100,
            cancelable: true,
        }
        .abi_encode(),
        zero(),
        "vesting/cancelable-stream",
    );
    h.at(start + 50);
    let owner = h.addr(0);
    let before_owner = balance(&mut h, t, owner);
    let before_beneficiary = balance(&mut h, t, beneficiary);
    h.ok(
        0,
        vesting,
        Vesting::cancelCall { id: u(1) }.abi_encode(),
        zero(),
        "vesting/cancel-half-vested",
    );
    assert_eq!(balance(&mut h, t, owner), before_owner + u(491));
    assert_eq!(balance(&mut h, t, beneficiary), before_beneficiary + u(491));
    assert_eq!(
        uint(
            &mut h,
            vesting,
            Vesting::claimableCall { id: u(1) }.abi_encode()
        ),
        zero()
    );
    error(
        &mut h,
        0,
        vesting,
        Vesting::cancelCall { id: u(1) }.abi_encode(),
        zero(),
        "AlreadyCanceled()",
        "vesting/double-cancel",
    );
    error(
        &mut h,
        1,
        vesting,
        Vesting::claimCall { id: u(1) }.abi_encode(),
        zero(),
        "NothingToClaim()",
        "vesting/claim-canceled",
    );
    let start = h.timestamp() + 100;
    h.ok(
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: u(1_000),
            start,
            cliff: 0,
            duration: 100,
            cancelable: true,
        }
        .abi_encode(),
        zero(),
        "vesting/future-cancelable-stream",
    );
    h.ok(
        0,
        vesting,
        Vesting::cancelCall { id: u(2) }.abi_encode(),
        zero(),
        "vesting/cancel-before-start",
    );
    assert_eq!(balance(&mut h, t, vesting), zero());

    h.ok(
        0,
        t,
        Token::configureCallbackCall {
            target: vesting,
            input: Vesting::claimCall { id: zero() }.abi_encode().into(),
            enabled: true,
        }
        .abi_encode(),
        zero(),
        "vesting/guard-configure",
    );
    h.ok(
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: u(1_000),
            start,
            cliff: 0,
            duration: 100,
            cancelable: false,
        }
        .abi_encode(),
        zero(),
        "vesting/guard-create",
    );
    assert!(boolean(
        &mut h,
        t,
        Token::callbackAttemptedCall {}.abi_encode()
    ));
    assert!(!boolean(&mut h, t, Token::innerSuccessCall {}.abi_encode()));
    guard_revert(&mut h, t);
    assert_eq!(
        uint(&mut h, vesting, Vesting::streamCountCall {}.abi_encode()),
        u(4)
    );
}

fn leaf(index: U256, account: Address, amount: U256) -> B256 {
    keccak256((index, account, amount).abi_encode_packed())
}

#[test]
fn token_vesting_max_deposit_claim_and_cancel_regression() {
    let mut h = Harness::new();
    let vesting = h.deploy("core/TokenVesting", vec![]);
    let t = h.deploy("support/TestToken", (0u16,).abi_encode_params());
    let owner = h.addr(0);
    let beneficiary = h.addr(1);
    h.ok(
        0,
        t,
        Token::mintCall {
            to: owner,
            amount: U256::MAX,
        }
        .abi_encode(),
        zero(),
        "vesting/max-supply-mint",
    );
    h.ok(
        0,
        t,
        Token::approveCall {
            spender: vesting,
            amount: U256::MAX,
        }
        .abi_encode(),
        zero(),
        "vesting/max-supply-approve",
    );
    let start = h.timestamp() + 20;
    h.ok(
        0,
        vesting,
        Vesting::createCall {
            token: t,
            beneficiary,
            amount: U256::MAX,
            start,
            cliff: 0,
            duration: 100,
            cancelable: true,
        }
        .abi_encode(),
        zero(),
        "vesting/max-deposit-create",
    );
    h.at(start + 25);
    h.ok(
        1,
        vesting,
        Vesting::claimCall { id: zero() }.abi_encode(),
        zero(),
        "vesting/max-deposit-quarter-claim",
    );
    assert_eq!(balance(&mut h, t, beneficiary), U256::MAX / u(4));
    h.at(start + 50);
    h.ok(
        0,
        vesting,
        Vesting::cancelCall { id: zero() }.abi_encode(),
        zero(),
        "vesting/max-deposit-half-cancel",
    );
    let paid = U256::MAX / u(2);
    assert_eq!(balance(&mut h, t, beneficiary), paid);
    assert_eq!(balance(&mut h, t, owner), U256::MAX - paid);
    assert_eq!(balance(&mut h, t, vesting), zero());
    assert_eq!(
        uint(
            &mut h,
            vesting,
            Vesting::claimableCall { id: zero() }.abi_encode()
        ),
        zero()
    );
}

#[test]
fn merkle_sorted_pair_proofs_and_same_index_callback() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let a = h.addr(1);
    let b = h.addr(2);
    let t = token(&mut h, Address::ZERO, 0);
    let l0 = leaf(zero(), a, u(100));
    let l1 = leaf(u(1), b, u(200));
    let root = if l0 <= l1 {
        keccak256((l0, l1).abi_encode_packed())
    } else {
        keccak256((l1, l0).abi_encode_packed())
    };
    let d = h.deploy(
        "core/MerkleDistributor",
        (t, owner, root, 0u64).abi_encode_params(),
    );
    h.ok(
        0,
        t,
        Token::transferCall {
            to: d,
            amount: u(300),
        }
        .abi_encode(),
        zero(),
        "distributor/two-leaf-fund",
    );
    error(
        &mut h,
        3,
        d,
        Distributor::claimCall {
            index: zero(),
            account: a,
            amount: u(100),
            proof: vec![B256::ZERO],
        }
        .abi_encode(),
        zero(),
        "BadProof()",
        "distributor/wrong-sibling",
    );
    let claim = Distributor::claimCall {
        index: zero(),
        account: a,
        amount: u(100),
        proof: vec![l1],
    }
    .abi_encode();
    h.ok(
        0,
        t,
        Token::configureCallbackCall {
            target: d,
            input: claim.clone().into(),
            enabled: true,
        }
        .abi_encode(),
        zero(),
        "distributor/same-index-callback-configure",
    );
    h.ok(3, d, claim, zero(), "distributor/sorted-pair-first");
    assert!(boolean(
        &mut h,
        t,
        Token::callbackAttemptedCall {}.abi_encode()
    ));
    assert!(!boolean(&mut h, t, Token::innerSuccessCall {}.abi_encode()));
    let callback = Token::innerOutputCall::abi_decode_returns(&h.view(
        0,
        t,
        Token::innerOutputCall {}.abi_encode(),
    ))
    .unwrap();
    assert_eq!(&callback[..4], &keccak256("AlreadyClaimed(uint256)")[..4]);
    h.ok(
        3,
        d,
        Distributor::claimCall {
            index: u(1),
            account: b,
            amount: u(200),
            proof: vec![l0],
        }
        .abi_encode(),
        zero(),
        "distributor/sorted-pair-second",
    );
    assert_eq!(balance(&mut h, t, a), u(100));
    assert_eq!(balance(&mut h, t, b), u(200));
    assert_eq!(balance(&mut h, t, d), zero());
}

#[test]
fn merkle_distributor_proof_sponsorship_expiry_and_sweep() {
    let mut h = Harness::new();
    let owner = h.addr(0);
    let recipient = h.addr(1);
    let t = token(&mut h, Address::ZERO, 0);
    let root = leaf(zero(), recipient, u(100));
    let ends = h.timestamp() + 100;
    let d = h.deploy(
        "core/MerkleDistributor",
        (t, owner, root, ends).abi_encode_params(),
    );
    h.ok(
        0,
        t,
        Token::transferCall {
            to: d,
            amount: u(200),
        }
        .abi_encode(),
        zero(),
        "distributor/fund",
    );
    error(
        &mut h,
        2,
        d,
        Distributor::claimCall {
            index: zero(),
            account: recipient,
            amount: u(101),
            proof: vec![],
        }
        .abi_encode(),
        zero(),
        "BadProof()",
        "distributor/bad-amount-proof",
    );
    let stranger = h.addr(2);
    error(
        &mut h,
        2,
        d,
        Distributor::claimCall {
            index: zero(),
            account: stranger,
            amount: u(100),
            proof: vec![],
        }
        .abi_encode(),
        zero(),
        "BadProof()",
        "distributor/wrong-recipient-proof",
    );
    error(
        &mut h,
        2,
        d,
        Distributor::claimCall {
            index: U256::MAX,
            account: recipient,
            amount: U256::MAX,
            proof: vec![B256::ZERO],
        }
        .abi_encode(),
        zero(),
        "BadProof()",
        "distributor/max-invalid-proof",
    );
    error(
        &mut h,
        1,
        d,
        Distributor::sweepCall {}.abi_encode(),
        zero(),
        "OnlyCreator()",
        "distributor/wrong-sweeper",
    );
    error(
        &mut h,
        0,
        d,
        Distributor::sweepCall {}.abi_encode(),
        zero(),
        "NotYet(uint64)",
        "distributor/early-sweep",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "distributor/fail-on",
    );
    error(
        &mut h,
        2,
        d,
        Distributor::claimCall {
            index: zero(),
            account: recipient,
            amount: u(100),
            proof: vec![],
        }
        .abi_encode(),
        zero(),
        "TransferFailed()",
        "distributor/failed-transfer",
    );
    assert!(!boolean(
        &mut h,
        d,
        Distributor::isClaimedCall { index: zero() }.abi_encode()
    ));
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "distributor/fail-off",
    );
    h.ok(
        2,
        d,
        Distributor::claimCall {
            index: zero(),
            account: recipient,
            amount: u(100),
            proof: vec![],
        }
        .abi_encode(),
        zero(),
        "distributor/sponsored-claim",
    );
    assert_eq!(balance(&mut h, t, recipient), u(100));
    assert!(boolean(
        &mut h,
        d,
        Distributor::isClaimedCall { index: zero() }.abi_encode()
    ));
    error(
        &mut h,
        2,
        d,
        Distributor::claimCall {
            index: zero(),
            account: recipient,
            amount: u(100),
            proof: vec![],
        }
        .abi_encode(),
        zero(),
        "AlreadyClaimed(uint256)",
        "distributor/double-claim",
    );
    h.at(ends);
    error(
        &mut h,
        2,
        d,
        Distributor::claimCall {
            index: u(1),
            account: recipient,
            amount: u(100),
            proof: vec![],
        }
        .abi_encode(),
        zero(),
        "Ended(uint64)",
        "distributor/claim-at-end",
    );
    h.ok(
        0,
        d,
        Distributor::sweepCall {}.abi_encode(),
        zero(),
        "distributor/sweep",
    );
    assert_eq!(balance(&mut h, t, d), zero());
    h.ok(
        0,
        d,
        Distributor::sweepCall {}.abi_encode(),
        zero(),
        "distributor/empty-resweep",
    );
    let perpetual = h.deploy(
        "core/MerkleDistributor",
        (t, owner, leaf(zero(), recipient, zero()), 0u64).abi_encode_params(),
    );
    error(
        &mut h,
        0,
        perpetual,
        Distributor::sweepCall {}.abi_encode(),
        zero(),
        "NoEnd()",
        "distributor/perpetual-no-sweep",
    );
    h.ok(
        2,
        perpetual,
        Distributor::claimCall {
            index: zero(),
            account: recipient,
            amount: zero(),
            proof: vec![],
        }
        .abi_encode(),
        zero(),
        "distributor/zero-leaf",
    );
}

#[test]
fn merkle_factory_atomic_funding_create2_and_rejections() {
    let mut h = Harness::new();
    let factory = h.deploy("core/MerkleDistributorFactory", vec![]);
    let t = token(&mut h, factory, 0);
    let recipient = h.addr(1);
    let root = leaf(zero(), recipient, u(100));
    error(
        &mut h,
        0,
        factory,
        Factory::createCall {
            token: t,
            merkleRoot: root,
            ends: 1,
            total: u(100),
        }
        .abi_encode(),
        zero(),
        "BadEnd(uint64)",
        "factory/expired-end",
    );
    error(
        &mut h,
        0,
        factory,
        Factory::createCall {
            token: t,
            merkleRoot: root,
            ends: 0,
            total: zero(),
        }
        .abi_encode(),
        zero(),
        "BadTotal()",
        "factory/zero-funding",
    );
    let eoa = h.addr(3);
    error(
        &mut h,
        0,
        factory,
        Factory::createCall {
            token: eoa,
            merkleRoot: root,
            ends: 0,
            total: u(100),
        }
        .abi_encode(),
        zero(),
        "NotAContract()",
        "factory/eoa-token",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "factory/fail-on",
    );
    error(
        &mut h,
        0,
        factory,
        Factory::createCall {
            token: t,
            merkleRoot: root,
            ends: 0,
            total: u(100),
        }
        .abi_encode(),
        zero(),
        "TransferFailed()",
        "factory/token-refuses-funding",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "factory/fail-off",
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 100 }.abi_encode(),
        zero(),
        "factory/fee-on",
    );
    error(
        &mut h,
        0,
        factory,
        Factory::createCall {
            token: t,
            merkleRoot: root,
            ends: 0,
            total: u(100),
        }
        .abi_encode(),
        zero(),
        "Underfunded(uint256,uint256)",
        "factory/fee-token-rejected",
    );
    assert_eq!(
        uint(&mut h, factory, Factory::countCall {}.abi_encode()),
        zero()
    );
    h.ok(
        0,
        t,
        Token::setFeeBpsCall { feeBps: 0 }.abi_encode(),
        zero(),
        "factory/fee-off",
    );
    let first = h.ok(
        0,
        factory,
        Factory::createCall {
            token: t,
            merkleRoot: root,
            ends: 0,
            total: u(100),
        }
        .abi_encode(),
        zero(),
        "factory/create",
    );
    let d = Address::abi_decode(&first.output).unwrap();
    assert_eq!(balance(&mut h, t, d), u(100));
    assert_eq!(
        Address::abi_decode(&h.view(
            0,
            factory,
            Factory::campaignsCall { index: zero() }.abi_encode()
        ))
        .unwrap(),
        d
    );
    h.ok(
        2,
        d,
        Distributor::claimCall {
            index: zero(),
            account: recipient,
            amount: u(100),
            proof: vec![],
        }
        .abi_encode(),
        zero(),
        "factory/child-claim",
    );
    let ends = h.timestamp() + 100;
    let second = h.ok(
        0,
        factory,
        Factory::createCall {
            token: t,
            merkleRoot: root,
            ends,
            total: u(100),
        }
        .abi_encode(),
        zero(),
        "factory/create-second-campaign",
    );
    assert_ne!(Address::abi_decode(&second.output).unwrap(), d);
    assert_eq!(
        uint(&mut h, factory, Factory::countCall {}.abi_encode()),
        u(2)
    );
}

#[test]
fn token_batch_atomicity_allowance_and_fee_token() {
    let mut h = Harness::new();
    let batch = h.deploy("core/TokenBatch", vec![]);
    let t = token(&mut h, batch, 100);
    let owner = h.addr(0);
    let a = h.addr(1);
    let b = h.addr(2);
    error(
        &mut h,
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![a],
            amounts: vec![],
        }
        .abi_encode(),
        zero(),
        "LengthMismatch()",
        "batch/length-mismatch",
    );
    error(
        &mut h,
        0,
        batch,
        Batch::sendCall {
            token: owner,
            to: vec![a],
            amounts: vec![u(100)],
        }
        .abi_encode(),
        zero(),
        "NotAContract()",
        "batch/eoa-token",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: true }.abi_encode(),
        zero(),
        "batch/fail-on",
    );
    error(
        &mut h,
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![a],
            amounts: vec![u(100)],
        }
        .abi_encode(),
        zero(),
        "TransferFailed()",
        "batch/token-refuses",
    );
    h.ok(
        0,
        t,
        Token::setFailTransfersCall { fail: false }.abi_encode(),
        zero(),
        "batch/fail-off",
    );
    h.ok(
        0,
        t,
        Token::approveCall {
            spender: batch,
            amount: u(150),
        }
        .abi_encode(),
        zero(),
        "batch/partial-allowance",
    );
    h.revert(
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![a, b],
            amounts: vec![u(100), u(100)],
        }
        .abi_encode(),
        zero(),
        "batch/second-transfer-rolls-back-first",
    );
    assert_eq!(balance(&mut h, t, a), zero());
    assert_eq!(balance(&mut h, t, b), zero());
    assert_eq!(balance(&mut h, t, owner), u(1_000_000));
    h.ok(
        0,
        t,
        Token::approveCall {
            spender: batch,
            amount: U256::MAX,
        }
        .abi_encode(),
        zero(),
        "batch/max-allowance",
    );
    // A self-transfer has no sender balance delta and is intentionally refused
    // by the batch's exact-movement receipt check, even for an honest ERC-20.
    error(
        &mut h,
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![owner],
            amounts: vec![u(1)],
        }
        .abi_encode(),
        zero(),
        "NotMoved(uint256,uint256)",
        "batch/self-recipient-not-moved",
    );
    h.revert(
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![Address::ZERO],
            amounts: vec![u(1)],
        }
        .abi_encode(),
        zero(),
        "batch/zero-recipient-token-rejects",
    );
    h.ok(
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![a, b],
            amounts: vec![u(100), u(200)],
        }
        .abi_encode(),
        zero(),
        "batch/fee-token-send",
    );
    assert_eq!(balance(&mut h, t, a), u(99));
    assert_eq!(balance(&mut h, t, b), u(198));
    assert_eq!(balance(&mut h, t, batch), zero());
    h.ok(
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![],
            amounts: vec![],
        }
        .abi_encode(),
        zero(),
        "batch/empty-batch",
    );
    h.ok(
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![a],
            amounts: vec![zero()],
        }
        .abi_encode(),
        zero(),
        "batch/zero-send",
    );
    h.revert(
        0,
        batch,
        Batch::sendCall {
            token: t,
            to: vec![a],
            amounts: vec![U256::MAX],
        }
        .abi_encode(),
        zero(),
        "batch/max-unfunded-send",
    );
}
