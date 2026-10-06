//! P-256 authorization, delegated accounts, vaults and genesis registries.
use super::harness::*;
use aether_crypto::{P256Signer, Signer};
use aether_execution::{account as acc, EvmCall};
use alloy_primitives::{address, b256, keccak256};
use alloy_sol_types::{sol, sol_data, SolCall, SolType, SolValue};

// `u8` is not a `SolValue` (alloy reserves it for bytes), so uint8 fields are
// encoded through explicit Solidity types; the bytes equal Solidity's ABI.
type VaultConfig = (
    sol_data::Array<VaultKey>,
    sol_data::Uint<8>,
    sol_data::Uint<128>,
    sol_data::Uint<64>,
);
type VaultSettingsMessage = (
    sol_data::Uint<256>,
    sol_data::Address,
    sol_data::FixedBytes<32>,
    sol_data::Uint<256>,
    sol_data::Array<VaultKey>,
    sol_data::Uint<8>,
    sol_data::Uint<128>,
    sol_data::Uint<64>,
);

sol! {
    struct VaultKey { bytes32 x; bytes32 y; }
    function spend(address to, uint256 amount, uint256 ownerIndex, bytes32 r, bytes32 s);
    function proposeWithdrawal(address token, address to, uint256 amount, uint256 ownerIndex, bytes32 r, bytes32 s);
    function proposeSettings(VaultKey[] newOwners, uint8 newThreshold, uint128 newDailyLimit, uint64 newDelay, uint256 ownerIndex, bytes32 r, bytes32 s);
    function approve(uint256 id, uint256 ownerIndex, bytes32 r, bytes32 s);
    function cancel(uint256 id, uint256 ownerIndex, bytes32 r, bytes32 s);
    function execute(uint256 id);
    function predict(VaultKey[] keys, uint8 threshold, uint128 dailyLimit, uint64 delay, bytes32 salt) returns (address);
    function create(VaultKey[] keys, uint8 threshold, uint128 dailyLimit, uint64 delay, bytes32 salt) returns (address);
    function removeOwner(uint256 index);
    function announceLeaving(bytes32 validatorKey);
    function announceBack(bytes32 validatorKey);
    function register(bytes32 validatorKey, bytes32 nodeId, address beaconer, bytes32 r, bytes32 s);
    function beacon(bytes32 validatorKey);
    function publish(bytes manifest, bytes32 archiveSha256, bytes builderSigs, bool emergency) returns (uint256);
    function count() returns (uint256);
    function randomness(uint64 epoch) returns (uint256);
    function mint(address to, uint256 amount);
    function setFeeBps(uint16 bps);
    function setFailTransfers(bool fail);
    function setCallback(address target, bytes data);
    function configure(address target_, bytes data_);
    function setRejectPayment(bool reject);
    function balanceOf(address who) returns (uint256);
    function callbackAttempted() returns (bool);
    function innerSuccess() returns (bool);
}

fn xy(s: &P256Signer) -> ([u8; 32], [u8; 32]) {
    aether_crypto::p256_xy(&s.public_key().bytes).unwrap()
}
fn key(s: &P256Signer) -> VaultKey {
    let (x, y) = xy(s);
    VaultKey {
        x: x.into(),
        y: y.into(),
    }
}
fn sig(s: &P256Signer, msg: &[u8]) -> (B256, B256) {
    let bytes = s.sign(msg).unwrap();
    (
        B256::from_slice(&bytes[..32]),
        B256::from_slice(&bytes[32..64]),
    )
}
fn self_call(h: &mut Harness, who: u8, data: Bytes, label: &str) {
    let a = h.addr(who);
    h.ok(
        who,
        a,
        acc::encode_execute(&[(a, U256::ZERO, data)]).to_vec(),
        U256::ZERO,
        label,
    );
}
fn delegate(h: &mut Harness, who: u8, implementation: Address) {
    h.transact(
        who,
        EvmCall {
            to: Some(h.addr(who)),
            value: U256::ZERO,
            input: acc::encode_execute(&[]),
            gas_limit: 16_777_216,
            delegate: Some(implementation),
        },
        "EastSeaAccount/7702-install",
        true,
    );
}
fn session_input(h: &Harness, calls: &[acc::AccountCall], id: u64, nonce: u64) -> Vec<u8> {
    let (r, s) = sig(
        &h.signer(1),
        &acc::session_message(CHAIN, h.addr(0), id, nonce, calls),
    );
    acc::encode_session_execute(calls, 0, r.0, s.0).to_vec()
}

#[test]
fn account_session_limits_nonce_expiry_revoke_and_original_key() {
    let mut h = Harness::new();
    let implementation = h.deploy("core/EastSeaAccount", vec![]);
    delegate(&mut h, 0, implementation);
    let a = h.addr(0);
    let recipient = h.addr(2);
    let (x, y) = xy(&h.signer(1));
    let limits = acc::SessionLimits {
        per_payment: 10,
        per_day: 15,
        expires: 0,
        allow: vec![recipient],
    };
    let add = acc::encode_add_session(x, y, &limits);
    h.revert(
        2,
        a,
        add.to_vec(),
        U256::ZERO,
        "EastSeaAccount/session-only-self",
    );
    self_call(&mut h, 0, add, "EastSeaAccount/session-add");
    let payment = vec![(recipient, U256::from(10), Bytes::new())];
    let encoded = session_input(&h, &payment, 1, 0);
    let before = h.state.balance(&recipient);
    h.ok(
        3,
        a,
        encoded.clone(),
        U256::ZERO,
        "EastSeaAccount/session-payment",
    );
    assert_eq!(h.state.balance(&recipient), before + U256::from(10));
    h.revert(3, a, encoded, U256::ZERO, "EastSeaAccount/session-replay");
    for (calls, label) in [
        (
            vec![(recipient, U256::from(11), Bytes::new())],
            "session-per-payment",
        ),
        (
            vec![(recipient, U256::from(6), Bytes::new())],
            "session-per-day",
        ),
        (
            vec![(h.addr(4), U256::ONE, Bytes::new())],
            "session-recipient",
        ),
        (vec![(a, U256::ZERO, Bytes::new())], "session-self-call"),
        (
            vec![(recipient, U256::ZERO, Bytes::from(vec![1, 2]))],
            "session-non-token",
        ),
    ] {
        let input = session_input(&h, &calls, 1, 1);
        h.revert(3, a, input, U256::ZERO, label);
    }
    // A zero transfer is still a signed payment and consumes a nonce.
    let zero = vec![(recipient, U256::ZERO, Bytes::new())];
    let input = session_input(&h, &zero, 1, 1);
    h.ok(3, a, input, U256::ZERO, "EastSeaAccount/session-zero");
    self_call(
        &mut h,
        0,
        acc::encode_remove_session(0),
        "EastSeaAccount/session-revoke",
    );
    let input = session_input(&h, &payment, 1, 2);
    h.revert(3, a, input, U256::ZERO, "EastSeaAccount/revoked-session");
    // Re-adding a key creates a fresh signature domain; old id=1 stays invalid.
    let expires = h.timestamp() + 30;
    self_call(
        &mut h,
        0,
        acc::encode_add_session(x, y, &acc::SessionLimits { expires, ..limits }),
        "EastSeaAccount/session-readd",
    );
    let stale = session_input(&h, &zero, 1, 0);
    h.revert(
        3,
        a,
        stale,
        U256::ZERO,
        "EastSeaAccount/session-serial-replay",
    );
    let valid = session_input(&h, &zero, 2, 0);
    h.ok(3, a, valid, U256::ZERO, "EastSeaAccount/session-new-domain");
    h.at(expires);
    let expired = session_input(&h, &zero, 2, 1);
    h.revert(3, a, expired, U256::ZERO, "EastSeaAccount/session-expired");
    self_call(
        &mut h,
        0,
        acc::encode_remove_session(0),
        "EastSeaAccount/session-revoke-expired",
    );
    h.revert(
        0,
        a,
        acc::encode_remove_session(0).to_vec(),
        U256::ZERO,
        "EastSeaAccount/session-missing",
    );
    for bad in [
        acc::encode_add_session(
            [0; 32],
            [0; 32],
            &acc::SessionLimits {
                per_payment: 1,
                per_day: 1,
                expires: 0,
                allow: vec![],
            },
        ),
        acc::encode_add_session(
            x,
            y,
            &acc::SessionLimits {
                per_payment: 0,
                per_day: 1,
                expires: 0,
                allow: vec![],
            },
        ),
        acc::encode_add_session(
            x,
            y,
            &acc::SessionLimits {
                per_payment: 2,
                per_day: 1,
                expires: 0,
                allow: vec![],
            },
        ),
        acc::encode_add_session(
            x,
            y,
            &acc::SessionLimits {
                per_payment: 1,
                per_day: 1,
                expires: 0,
                allow: vec![recipient; 17],
            },
        ),
        acc::encode_set_session_token(0, recipient, 1, 1),
    ] {
        h.revert(
            0,
            a,
            bad.to_vec(),
            U256::ZERO,
            "EastSeaAccount/bad-session-config",
        );
    }
    // EIP-7702 revocation leaves the original P-256 key fully usable.
    h.transact(
        0,
        EvmCall {
            to: Some(a),
            value: U256::ZERO,
            input: Bytes::new(),
            gas_limit: 16_777_216,
            delegate: Some(Address::ZERO),
        },
        "EastSeaAccount/7702-revoke",
        true,
    );
    assert!(h.state.code(&a).is_empty());
    h.ok(
        0,
        recipient,
        vec![],
        U256::ONE,
        "EastSeaAccount/original-key-after-revoke",
    );
}

#[test]
fn account_guardian_recovery_owner_execution_and_f05() {
    let mut h = Harness::new();
    let implementation = h.deploy("core/EastSeaAccount", vec![]);
    delegate(&mut h, 0, implementation);
    let a = h.addr(0);
    let guardian = xy(&h.signer(1));
    let owner = xy(&h.signer(2));
    let recovered = vec![(a, U256::ZERO, acc::encode_add_owner(owner.0, owner.1))];
    h.revert(
        3,
        a,
        acc::encode_propose_recovery(&recovered, &[]).to_vec(),
        U256::ZERO,
        "EastSeaAccount/no-guardians",
    );
    h.revert(
        3,
        a,
        acc::encode_execute_recovery(&recovered).to_vec(),
        U256::ZERO,
        "EastSeaAccount/no-recovery",
    );
    h.revert(
        0,
        a,
        acc::encode_cancel_recovery().to_vec(),
        U256::ZERO,
        "EastSeaAccount/cancel-none",
    );
    for (keys, threshold, delay) in [
        (vec![], 1, 600),
        (vec![guardian], 0, 600),
        (vec![guardian], 2, 600),
        (vec![guardian], 1, 599),
        (vec![([0; 32], [0; 32])], 1, 600),
        (vec![guardian, guardian], 1, 600),
        (vec![guardian; 9], 1, 600),
    ] {
        h.revert(
            0,
            a,
            acc::encode_set_guardians(&keys, threshold, delay).to_vec(),
            U256::ZERO,
            "EastSeaAccount/bad-guardians",
        );
    }
    self_call(
        &mut h,
        0,
        acc::encode_set_guardians(&[guardian], 1, 600),
        "EastSeaAccount/guardians-set",
    );
    h.revert(
        0,
        a,
        acc::encode_add_guardian(guardian.0, guardian.1).to_vec(),
        U256::ZERO,
        "EastSeaAccount/guardian-duplicate",
    );
    h.revert(
        0,
        a,
        acc::encode_add_guardian([0; 32], [0; 32]).to_vec(),
        U256::ZERO,
        "EastSeaAccount/guardian-zero",
    );
    h.revert(
        3,
        a,
        acc::encode_propose_recovery(&recovered, &[(0, [0; 32], [0; 32])]).to_vec(),
        U256::ZERO,
        "EastSeaAccount/recovery-forged",
    );
    let sign_recovery = |h: &Harness, nonce| {
        let (r, s) = sig(
            &h.signer(1),
            &acc::recovery_message(CHAIN, a, nonce, &recovered),
        );
        acc::encode_propose_recovery(&recovered, &[(0, r.0, s.0)]).to_vec()
    };
    let input = sign_recovery(&h, 0);
    h.ok(
        3,
        a,
        input.clone(),
        U256::ZERO,
        "EastSeaAccount/recovery-propose",
    );
    h.revert(
        3,
        a,
        input,
        U256::ZERO,
        "EastSeaAccount/recovery-already-pending",
    );
    h.revert(
        3,
        a,
        acc::encode_execute_recovery(&recovered).to_vec(),
        U256::ZERO,
        "EastSeaAccount/recovery-too-early",
    );
    self_call(
        &mut h,
        0,
        acc::encode_cancel_recovery(),
        "EastSeaAccount/recovery-cancel",
    );
    let input = sign_recovery(&h, 1);
    h.ok(3, a, input, U256::ZERO, "EastSeaAccount/recovery-repropose");
    h.at(h.timestamp() + 601);
    h.revert(
        3,
        a,
        acc::encode_execute_recovery(&[]).to_vec(),
        U256::ZERO,
        "EastSeaAccount/recovery-wrong-calls",
    );
    h.ok(
        3,
        a,
        acc::encode_execute_recovery(&recovered).to_vec(),
        U256::ZERO,
        "EastSeaAccount/recovery-execute",
    );
    h.revert(
        3,
        a,
        acc::encode_execute_recovery(&recovered).to_vec(),
        U256::ZERO,
        "EastSeaAccount/recovery-twice",
    );
    let payment = vec![(h.addr(4), U256::from(7), Bytes::new())];
    let (r, s) = sig(&h.signer(2), &acc::owner_message(CHAIN, a, 0, &payment));
    let owner_call = acc::encode_owner_execute(&payment, 0, r.0, s.0).to_vec();
    h.ok(
        3,
        a,
        owner_call.clone(),
        U256::ZERO,
        "EastSeaAccount/recovered-owner-payment",
    );
    h.revert(3, a, owner_call, U256::ZERO, "EastSeaAccount/owner-replay");
    h.revert(
        3,
        a,
        acc::encode_owner_execute(&payment, 9, r.0, s.0).to_vec(),
        U256::ZERO,
        "EastSeaAccount/owner-missing",
    );
    // F-05: guardians cannot invalidate the original account key. Assert its
    // authority after successful recovery, rather than treating recovery as rotation.
    self_call(
        &mut h,
        0,
        acc::encode_execute(&payment),
        "EastSeaAccount/F05-original-key-after-recovery",
    );
    self_call(
        &mut h,
        0,
        removeOwnerCall { index: U256::ZERO }.abi_encode().into(),
        "EastSeaAccount/owner-remove",
    );
    h.revert(
        0,
        a,
        removeOwnerCall { index: U256::ZERO }.abi_encode(),
        U256::ZERO,
        "EastSeaAccount/owner-remove-missing",
    );
    self_call(
        &mut h,
        0,
        acc::encode_set_guardian([0; 32], [0; 32]),
        "EastSeaAccount/guardian-disable",
    );
    self_call(
        &mut h,
        0,
        acc::encode_add_guardian(guardian.0, guardian.1),
        "EastSeaAccount/guardian-add",
    );
    h.revert(
        0,
        a,
        acc::encode_add_owner([0; 32], [0; 32]).to_vec(),
        U256::ZERO,
        "EastSeaAccount/owner-zero",
    );
}

const SPEND_TAG: B256 = b256!("395741c5d5c1686a2a5bfd2dfbf0e21a5103dd6d7f7125d96a2082d36d9cda4a");
const WITHDRAW_TAG: B256 =
    b256!("21858ffa6f9c911943ef018a75e7da08734a90ab7da4e5d6dfccf97774c11469");
const SETTINGS_TAG: B256 =
    b256!("0210597db69caa67467d9c98b53a47f1d05b6676e7829a940f6164a894393649");
const CANCEL_TAG: B256 = b256!("c479f5f30d0b8d05e9705da782781a9966ba8b708f6d7e5fd03dd102a368fb1e");
fn withdraw_sig(
    h: &Harness,
    vault: Address,
    owner: u8,
    id: u64,
    token: Address,
    to: Address,
    amount: U256,
) -> (B256, B256) {
    sig(
        &h.signer(owner),
        &(
            U256::from(CHAIN),
            vault,
            WITHDRAW_TAG,
            U256::from(id),
            token,
            to,
            amount,
        )
            .abi_encode_params(),
    )
}

#[test]
fn vault_native_spend_queue_cancel_settings_and_factory() {
    let mut h = Harness::new();
    let keys = vec![key(&h.signer(1)), key(&h.signer(2))];
    let v = h.deploy(
        "core/EastSeaVault",
        VaultConfig::abi_encode_params(&(keys.clone(), 2u8, 15u128, 86_400u64)),
    );
    h.ok(0, v, vec![], U256::from(1_000), "EastSeaVault/deposit");
    let to = h.addr(4);
    let spend_input = |h: &Harness, amount: U256, nonce| {
        let (r, s) = sig(
            &h.signer(1),
            &(
                U256::from(CHAIN),
                v,
                SPEND_TAG,
                U256::from(nonce),
                to,
                amount,
            )
                .abi_encode_params(),
        );
        spendCall {
            to,
            amount,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode()
    };
    let input = spend_input(&h, U256::from(10), 0u64);
    let before = h.state.balance(&to);
    h.ok(3, v, input.clone(), U256::ZERO, "EastSeaVault/spend");
    assert_eq!(h.state.balance(&to), before + U256::from(10));
    h.revert(3, v, input, U256::ZERO, "EastSeaVault/spend-replay");
    let input = spend_input(&h, U256::from(6), 1u64);
    h.revert(3, v, input, U256::ZERO, "EastSeaVault/daily-limit");
    h.revert(
        3,
        v,
        spendCall {
            to,
            amount: U256::ONE,
            ownerIndex: U256::from(9),
            r: B256::ZERO,
            s: B256::ZERO,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/wrong-owner",
    );
    // Conservative rolling limit retains yesterday's usage across midnight.
    h.at((h.timestamp() / 86_400 + 1) * 86_400);
    let input = spend_input(&h, U256::from(6), 1u64);
    h.revert(3, v, input, U256::ZERO, "EastSeaVault/midnight-limit");
    h.at((h.timestamp() / 86_400 + 1) * 86_400);
    let input = spend_input(&h, U256::from(15), 1u64);
    h.ok(3, v, input, U256::ZERO, "EastSeaVault/daily-reset");
    let input = spend_input(&h, U256::ZERO, 2u64);
    h.ok(3, v, input, U256::ZERO, "EastSeaVault/spend-zero");
    let input = spend_input(&h, U256::MAX, 3u64);
    h.revert(3, v, input, U256::ZERO, "EastSeaVault/spend-max");
    let amount = U256::from(100);
    let (r, s) = withdraw_sig(&h, v, 1, 1, Address::ZERO, to, amount);
    h.ok(
        3,
        v,
        proposeWithdrawalCall {
            token: Address::ZERO,
            to,
            amount,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/withdraw-propose",
    );
    h.revert(
        3,
        v,
        executeCall { id: U256::ONE }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/threshold-unmet",
    );
    h.revert(
        3,
        v,
        approveCall {
            id: U256::ONE,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/duplicate-approval",
    );
    let (r, s) = withdraw_sig(&h, v, 2, 1, Address::ZERO, to, amount);
    h.ok(
        3,
        v,
        approveCall {
            id: U256::ONE,
            ownerIndex: U256::ONE,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/approve",
    );
    h.revert(
        3,
        v,
        executeCall { id: U256::ONE }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/timelock-early",
    );
    h.at(h.timestamp() + 86_401);
    let before = h.state.balance(&to);
    h.ok(
        3,
        v,
        executeCall { id: U256::ONE }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/withdraw-execute",
    );
    assert_eq!(h.state.balance(&to), before + amount);
    h.revert(
        3,
        v,
        executeCall { id: U256::ONE }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/withdraw-twice",
    );
    // A no-code ERC-20 cannot be queued even with a valid owner's signature.
    let (r, s) = withdraw_sig(&h, v, 1, 2, to, to, amount);
    h.revert(
        3,
        v,
        proposeWithdrawalCall {
            token: to,
            to,
            amount,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/no-code-token",
    );
    let (r, s) = withdraw_sig(&h, v, 1, 2, Address::ZERO, to, amount);
    h.ok(
        3,
        v,
        proposeWithdrawalCall {
            token: Address::ZERO,
            to,
            amount,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/cancel-proposal",
    );
    h.revert(
        3,
        v,
        cancelCall {
            id: U256::from(2),
            ownerIndex: U256::ZERO,
            r: B256::ZERO,
            s: B256::ZERO,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/cancel-forged",
    );
    let (r, s) = sig(
        &h.signer(2),
        &(U256::from(CHAIN), v, CANCEL_TAG, U256::ZERO, U256::from(2)).abi_encode_params(),
    );
    h.ok(
        3,
        v,
        cancelCall {
            id: U256::from(2),
            ownerIndex: U256::ONE,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/cancel",
    );
    h.revert(
        3,
        v,
        cancelCall {
            id: U256::from(2),
            ownerIndex: U256::ONE,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/cancel-twice",
    );
    let new_keys = vec![key(&h.signer(1))];
    let settings_message = VaultSettingsMessage::abi_encode_params(&(
        U256::from(CHAIN),
        v,
        SETTINGS_TAG,
        U256::from(3),
        new_keys.clone(),
        1u8,
        25u128,
        86_400u64,
    ));
    let (r, s) = sig(&h.signer(1), &settings_message);
    h.revert(
        3,
        v,
        proposeSettingsCall {
            newOwners: vec![],
            newThreshold: 1,
            newDailyLimit: 25,
            newDelay: 86_400,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/bad-settings",
    );
    h.ok(
        3,
        v,
        proposeSettingsCall {
            newOwners: new_keys.clone(),
            newThreshold: 1,
            newDailyLimit: 25,
            newDelay: 86_400,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/settings-propose",
    );
    let (r, s) = sig(&h.signer(2), &settings_message);
    h.ok(
        3,
        v,
        approveCall {
            id: U256::from(3),
            ownerIndex: U256::ONE,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/settings-approve",
    );
    h.at(h.timestamp() + 86_401);
    h.ok(
        3,
        v,
        executeCall { id: U256::from(3) }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/settings-execute",
    );
    assert_eq!(
        h.state.storage(&v, U256::from(7)),
        U256::ONE,
        "queue era advances"
    );
    let factory = h.deploy("core/EastSeaVaultFactory", vec![]);
    let salt = B256::repeat_byte(7);
    let prediction = h.view(
        0,
        factory,
        predictCall {
            keys: keys.clone(),
            threshold: 2,
            dailyLimit: 15,
            delay: 86_400,
            salt,
        }
        .abi_encode(),
    );
    let predicted = Address::abi_decode(&prediction).unwrap();
    h.ok(
        0,
        factory,
        createCall {
            keys: keys.clone(),
            threshold: 2,
            dailyLimit: 15,
            delay: 86_400,
            salt,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVaultFactory/create",
    );
    assert!(!h.state.code(&predicted).is_empty());
    h.revert(
        0,
        factory,
        createCall {
            keys,
            threshold: 2,
            dailyLimit: 15,
            delay: 86_400,
            salt,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVaultFactory/duplicate-salt",
    );
    h.revert(
        0,
        factory,
        createCall {
            keys: new_keys,
            threshold: 0,
            dailyLimit: 15,
            delay: 86_400,
            salt: B256::repeat_byte(8),
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVaultFactory/bad-config",
    );
}

#[test]
fn registries_attestation_beacons_bounds_and_v3_availability() {
    for artifact in ["core/CommitteeRegistry", "core/CommitteeRegistryV3"] {
        let mut h = Harness::new();
        let deployed = h.deploy(artifact, vec![]);
        let registry = aether_execution::registry::REGISTRY;
        // A genesis allocation installs compiled runtime with constructor-less
        // registrar and epoch slots; subsequent calls run only via execute_block.
        h.state.set_code(registry, h.state.code(&deployed)).unwrap();
        h.label(registry, artifact);
        let (x, y) = xy(&h.signer(1));
        h.state
            .set_storage(registry, U256::ZERO, U256::from_be_bytes(x));
        h.state
            .set_storage(registry, U256::ONE, U256::from_be_bytes(y));
        h.state.set_storage(registry, U256::from(4), U256::from(10));
        h.state.set_storage(registry, U256::from(7), U256::ONE);
        let operator = h.addr(2);
        let beaconer = h.addr(3);
        let vk = B256::repeat_byte(11);
        let node = B256::repeat_byte(12);
        let attest = |h: &Harness, validator: B256| {
            sig(
                &h.signer(1),
                &(
                    U256::from(CHAIN),
                    registry,
                    operator,
                    validator,
                    node,
                    beaconer,
                )
                    .abi_encode_params(),
            )
        };
        h.revert(
            2,
            registry,
            registerCall {
                validatorKey: vk,
                nodeId: node,
                beaconer,
                r: B256::ZERO,
                s: B256::ZERO,
            }
            .abi_encode(),
            U256::ZERO,
            "CommitteeRegistry/bad-attestation",
        );
        let (r, s) = attest(&h, vk);
        h.ok(
            2,
            registry,
            registerCall {
                validatorKey: vk,
                nodeId: node,
                beaconer,
                r,
                s,
            }
            .abi_encode(),
            U256::ZERO,
            "CommitteeRegistry/register",
        );
        h.revert(
            2,
            registry,
            registerCall {
                validatorKey: vk,
                nodeId: node,
                beaconer,
                r,
                s,
            }
            .abi_encode(),
            U256::ZERO,
            "CommitteeRegistry/duplicate-key",
        );
        h.revert(
            2,
            registry,
            beaconCall { validatorKey: vk }.abi_encode(),
            U256::ZERO,
            "CommitteeRegistry/wrong-beaconer",
        );
        h.revert(
            3,
            registry,
            beaconCall {
                validatorKey: B256::ZERO,
            }
            .abi_encode(),
            U256::ZERO,
            "CommitteeRegistry/unknown-beacon",
        );
        // Calls each occupy a block: align to a fresh epoch for cap assertions.
        h.advance(10 - h.number() % 10);
        let second = B256::repeat_byte(13);
        let (r, s) = attest(&h, second);
        h.ok(
            2,
            registry,
            registerCall {
                validatorKey: second,
                nodeId: node,
                beaconer,
                r,
                s,
            }
            .abi_encode(),
            U256::ZERO,
            "CommitteeRegistry/next-epoch-register",
        );
        let third = B256::repeat_byte(14);
        let (r, s) = attest(&h, third);
        h.revert(
            2,
            registry,
            registerCall {
                validatorKey: third,
                nodeId: node,
                beaconer,
                r,
                s,
            }
            .abi_encode(),
            U256::ZERO,
            "CommitteeRegistry/epoch-cap",
        );
        h.ok(
            3,
            registry,
            beaconCall { validatorKey: vk }.abi_encode(),
            U256::ZERO,
            "CommitteeRegistry/beacon",
        );
        let candidates = aether_execution::registry::candidates(&h.state);
        assert_eq!(candidates.len(), 2);
        assert!(candidates[0].streak >= 2);
        if artifact.ends_with("V3") {
            h.revert(
                2,
                registry,
                announceLeavingCall { validatorKey: vk }.abi_encode(),
                U256::ZERO,
                "CommitteeRegistryV3/wrong-announcer",
            );
            h.revert(
                3,
                registry,
                announceBackCall {
                    validatorKey: B256::ZERO,
                }
                .abi_encode(),
                U256::ZERO,
                "CommitteeRegistryV3/unknown-back",
            );
            h.ok(
                3,
                registry,
                announceLeavingCall { validatorKey: vk }.abi_encode(),
                U256::ZERO,
                "CommitteeRegistryV3/leaving",
            );
            let before = aether_execution::registry::candidates(&h.state)[0].streak;
            h.advance(300);
            h.ok(
                3,
                registry,
                announceBackCall { validatorKey: vk }.abi_encode(),
                U256::ZERO,
                "CommitteeRegistryV3/back",
            );
            h.ok(
                3,
                registry,
                beaconCall { validatorKey: vk }.abi_encode(),
                U256::ZERO,
                "CommitteeRegistryV3/announced-gap-beacon",
            );
            let c = &aether_execution::registry::candidates(&h.state)[0];
            assert_eq!(c.streak, before + 1);
            assert_eq!(c.missed, candidates[0].missed);
        } else {
            h.advance(300);
            h.ok(
                3,
                registry,
                beaconCall { validatorKey: vk }.abi_encode(),
                U256::ZERO,
                "CommitteeRegistry/grace-reset",
            );
            let c = &aether_execution::registry::candidates(&h.state)[0];
            assert_eq!((c.streak, c.missed), (1, 0));
        }
    }
}

#[test]
fn release_log_permissionless_predeploy_boundaries_and_randomness_view() {
    let mut h = Harness::new();
    h.deploy("core/ReleaseLog", vec![]);
    let log = aether_execution::release_log::ADDRESS;
    h.state
        .set_code(log, aether_execution::release_log::code())
        .unwrap();
    let input = |manifest: Vec<u8>, sigs: Vec<u8>, emergency| {
        publishCall {
            manifest: manifest.into(),
            archiveSha256: B256::repeat_byte(1),
            builderSigs: sigs.into(),
            emergency,
        }
        .abi_encode()
    };
    h.revert(
        1,
        log,
        input(vec![], vec![1], false),
        U256::ZERO,
        "ReleaseLog/empty-manifest",
    );
    h.revert(
        1,
        log,
        input(vec![1], vec![], false),
        U256::ZERO,
        "ReleaseLog/empty-signatures",
    );
    h.revert(
        1,
        log,
        input(vec![1; 16_385], vec![1], false),
        U256::ZERO,
        "ReleaseLog/oversized-manifest",
    );
    h.revert(
        1,
        log,
        input(vec![1], vec![1; 4_097], false),
        U256::ZERO,
        "ReleaseLog/oversized-signatures",
    );
    // F-07: successful logging is never evidence of builder approval. Arbitrary
    // signature bytes and an arbitrary caller are valid publish inputs.
    h.ok(
        7,
        log,
        input(vec![1], vec![0xff], false),
        U256::ZERO,
        "ReleaseLog/F07-permissionless-publish",
    );
    h.ok(
        8,
        log,
        input(vec![1; 16_384], vec![1; 4_096], true),
        U256::ZERO,
        "ReleaseLog/max-emergency-publish",
    );
    let count = h.view(0, log, countCall {}.abi_encode());
    assert_eq!(U256::abi_decode(&count).unwrap(), U256::from(2));
    let random = h.deploy("core/Randomness", vec![]);
    let rewards = address!("0000000000000000000000000000000000007704");
    h.state.set_code(rewards, h.state.code(&random)).unwrap();
    h.label(rewards, "core/Randomness");
    for target in [random, rewards] {
        let missing = h.view(0, target, randomnessCall { epoch: 5 }.abi_encode());
        assert_eq!(U256::abi_decode(&missing).unwrap(), U256::ZERO);
        let word = U256::from_be_bytes(keccak256(b"threshold-signature-fixture").0);
        h.state
            .set_storage(target, (U256::from(9) << 200) | U256::from(5), word);
        let present = h.view(0, target, randomnessCall { epoch: 5 }.abi_encode());
        assert_eq!(U256::abi_decode(&present).unwrap(), word);
        let other = h.view(0, target, randomnessCall { epoch: u64::MAX }.abi_encode());
        assert_eq!(U256::abi_decode(&other).unwrap(), U256::ZERO);
    }
}

#[test]
fn account_token_sessions_batch_limits_failure_and_reentry() {
    let mut h = Harness::new();
    let implementation = h.deploy("core/EastSeaAccount", vec![]);
    let token = h.deploy("support/TestToken", (0u16,).abi_encode_params());
    delegate(&mut h, 0, implementation);
    let a = h.addr(0);
    let recipient = h.addr(2);
    let (x, y) = xy(&h.signer(1));
    self_call(
        &mut h,
        0,
        acc::encode_add_session(
            x,
            y,
            &acc::SessionLimits {
                per_payment: 10,
                per_day: 15,
                expires: 0,
                allow: vec![recipient],
            },
        ),
        "EastSeaAccount/token-session-add",
    );
    h.ok(
        0,
        token,
        mintCall {
            to: a,
            amount: U256::from(100),
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaAccount/token-fund",
    );
    let transfer = |amount: u64| {
        (
            token,
            U256::ZERO,
            acc::encode_token_transfer(recipient, U256::from(amount)),
        )
    };
    let input = session_input(&h, &[transfer(5)], 1, 0);
    h.revert(3, a, input, U256::ZERO, "EastSeaAccount/token-not-allowed");
    for (target, payment, daily) in [
        (Address::ZERO, 1, 1),
        (a, 1, 1),
        (recipient, 1, 1),
        (token, 0, 1),
        (token, 2, 1),
    ] {
        h.revert(
            0,
            a,
            acc::encode_set_session_token(0, target, payment, daily).to_vec(),
            U256::ZERO,
            "EastSeaAccount/token-bad-policy",
        );
    }
    self_call(
        &mut h,
        0,
        acc::encode_set_session_token(0, token, 10, 15),
        "EastSeaAccount/token-policy",
    );
    for (calls, label) in [
        (vec![transfer(0)], "EastSeaAccount/token-zero"),
        (
            vec![(
                token,
                U256::ZERO,
                acc::encode_token_transfer(Address::ZERO, U256::ONE),
            )],
            "EastSeaAccount/token-zero-recipient",
        ),
        (
            vec![(
                token,
                U256::ONE,
                acc::encode_token_transfer(recipient, U256::ONE),
            )],
            "EastSeaAccount/token-native-value",
        ),
        (
            vec![(token, U256::ZERO, Bytes::from(vec![1; 68]))],
            "EastSeaAccount/token-wrong-selector",
        ),
        (
            vec![transfer(6), transfer(5)],
            "EastSeaAccount/token-aggregate-payment-limit",
        ),
    ] {
        let input = session_input(&h, &calls, 1, 0);
        h.revert(3, a, input, U256::ZERO, label);
    }
    let batch = vec![transfer(6), transfer(4)];
    let input = session_input(&h, &batch, 1, 0);
    h.ok(
        3,
        a,
        input.clone(),
        U256::ZERO,
        "EastSeaAccount/token-batch",
    );
    h.revert(3, a, input, U256::ZERO, "EastSeaAccount/token-batch-replay");
    let value = h.view(0, token, balanceOfCall { who: recipient }.abi_encode());
    assert_eq!(U256::abi_decode(&value).unwrap(), U256::from(10));
    let input = session_input(&h, &[transfer(6)], 1, 1);
    h.revert(3, a, input, U256::ZERO, "EastSeaAccount/token-daily-limit");
    h.ok(
        0,
        token,
        setFailTransfersCall { fail: true }.abi_encode(),
        U256::ZERO,
        "EastSeaAccount/token-failure-on",
    );
    let input = session_input(&h, &[transfer(1)], 1, 1);
    h.revert(3, a, input, U256::ZERO, "EastSeaAccount/token-false-return");
    h.ok(
        0,
        token,
        setFailTransfersCall { fail: false }.abi_encode(),
        U256::ZERO,
        "EastSeaAccount/token-failure-off",
    );
    let input = session_input(&h, &[transfer(1)], 1, 1);
    h.ok(
        0,
        token,
        setCallbackCall {
            target: a,
            data: input.clone().into(),
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaAccount/token-reentry-configure",
    );
    h.ok(
        3,
        a,
        input,
        U256::ZERO,
        "EastSeaAccount/token-reentry-outer",
    );
    assert!(bool::abi_decode(&h.view(0, token, callbackAttemptedCall {}.abi_encode())).unwrap());
    assert!(
        !bool::abi_decode(&h.view(0, token, innerSuccessCall {}.abi_encode())).unwrap(),
        "session nonce is consumed before token callbacks"
    );
}

#[test]
fn vault_exact_token_delivery_and_native_reentry() {
    let mut h = Harness::new();
    let keys = vec![key(&h.signer(1))];
    let v = h.deploy(
        "core/EastSeaVault",
        VaultConfig::abi_encode_params(&(keys, 1u8, 15u128, 86_400u64)),
    );
    let token = h.deploy("support/TestToken", (0u16,).abi_encode_params());
    let receiver = h.deploy("support/NativeCallback", vec![]);
    let to = h.addr(4);
    h.ok(
        0,
        token,
        mintCall {
            to: v,
            amount: U256::from(100),
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/token-fund",
    );
    h.ok(0, v, vec![], U256::from(100), "EastSeaVault/native-fund");
    let queue = |h: &mut Harness, id, token, to| {
        let amount = U256::from(10);
        let (r, s) = withdraw_sig(h, v, 1, id, token, to, amount);
        h.ok(
            3,
            v,
            proposeWithdrawalCall {
                token,
                to,
                amount,
                ownerIndex: U256::ZERO,
                r,
                s,
            }
            .abi_encode(),
            U256::ZERO,
            "EastSeaVault/queue-payout",
        );
    };
    queue(&mut h, 1u64, token, to);
    h.at(h.timestamp() + 86_401);
    h.ok(
        0,
        token,
        setFeeBpsCall { bps: 1_000 }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/token-fee-on",
    );
    h.revert(
        3,
        v,
        executeCall { id: U256::ONE }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/fee-on-transfer-refused",
    );
    h.ok(
        0,
        token,
        setFeeBpsCall { bps: 0 }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/token-fee-off",
    );
    h.ok(
        0,
        token,
        setFailTransfersCall { fail: true }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/token-false-on",
    );
    h.revert(
        3,
        v,
        executeCall { id: U256::ONE }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/token-false-refused",
    );
    h.ok(
        0,
        token,
        setFailTransfersCall { fail: false }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/token-false-off",
    );
    h.ok(
        0,
        token,
        setCallbackCall {
            target: v,
            data: executeCall { id: U256::ONE }.abi_encode().into(),
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/token-reentry-configure",
    );
    h.ok(
        3,
        v,
        executeCall { id: U256::ONE }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/token-withdraw-reentry",
    );
    assert!(bool::abi_decode(&h.view(0, token, callbackAttemptedCall {}.abi_encode())).unwrap());
    assert!(!bool::abi_decode(&h.view(0, token, innerSuccessCall {}.abi_encode())).unwrap());
    assert_eq!(
        U256::abi_decode(&h.view(0, token, balanceOfCall { who: to }.abi_encode())).unwrap(),
        U256::from(10)
    );
    queue(&mut h, 2u64, Address::ZERO, receiver);
    h.at(h.timestamp() + 86_401);
    h.ok(
        0,
        receiver,
        setRejectPaymentCall { reject: true }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/native-reject-on",
    );
    let spend_message = (
        U256::from(CHAIN),
        v,
        SPEND_TAG,
        U256::ZERO,
        receiver,
        U256::ONE,
    )
        .abi_encode_params();
    let (r, s) = sig(&h.signer(1), &spend_message);
    h.revert(
        3,
        v,
        spendCall {
            to: receiver,
            amount: U256::ONE,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/spend-transfer-failed",
    );
    h.revert(
        3,
        v,
        executeCall { id: U256::from(2) }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/native-payout-refused",
    );
    h.ok(
        0,
        receiver,
        setRejectPaymentCall { reject: false }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/native-reject-off",
    );
    h.ok(
        0,
        receiver,
        configureCall {
            target_: v,
            data_: executeCall { id: U256::from(2) }.abi_encode().into(),
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/native-reentry-configure",
    );
    h.ok(
        3,
        v,
        executeCall { id: U256::from(2) }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/native-withdraw-reentry",
    );
    assert!(bool::abi_decode(&h.view(0, receiver, callbackAttemptedCall {}.abi_encode())).unwrap());
    assert!(!bool::abi_decode(&h.view(0, receiver, innerSuccessCall {}.abi_encode())).unwrap());
    assert_eq!(h.state.balance(&receiver), U256::from(10));
    h.ok(
        3,
        v,
        spendCall {
            to: receiver,
            amount: U256::ONE,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/spend-retry-after-failure",
    );
    // A contract with code but no ERC-20 balance ABI gets as far as execution,
    // then rejects without deleting its pending proposal or moving assets.
    queue(&mut h, 3u64, receiver, to);
    h.at(h.timestamp() + 86_401);
    h.revert(
        3,
        v,
        executeCall { id: U256::from(3) }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/non-erc20-contract",
    );
    let replacement = vec![key(&h.signer(1))];
    let message = VaultSettingsMessage::abi_encode_params(&(
        U256::from(CHAIN),
        v,
        SETTINGS_TAG,
        U256::from(4),
        replacement.clone(),
        1u8,
        25u128,
        86_400u64,
    ));
    let (r, s) = sig(&h.signer(1), &message);
    h.ok(
        3,
        v,
        proposeSettingsCall {
            newOwners: replacement,
            newThreshold: 1,
            newDailyLimit: 25,
            newDelay: 86_400,
            ownerIndex: U256::ZERO,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "EastSeaVault/queue-replacement-settings",
    );
    queue(&mut h, 5u64, Address::ZERO, to);
    h.at(h.timestamp() + 86_401);
    h.ok(
        3,
        v,
        executeCall { id: U256::from(4) }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/replace-settings-invalidates-queue",
    );
    h.revert(
        3,
        v,
        executeCall { id: U256::from(5) }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/old-owner-era-withdrawal",
    );
    h.revert(
        3,
        v,
        executeCall { id: U256::from(3) }.abi_encode(),
        U256::ZERO,
        "EastSeaVault/old-owner-era-malformed-token",
    );
}

#[test]
fn vault_constructor_rejects_all_invalid_config_shapes() {
    let mut h = Harness::new();
    let k = key(&h.signer(1));
    for (keys, threshold, delay) in [
        (vec![], 1u8, 86_400u64),
        (vec![k.clone()], 0, 86_400),
        (vec![k.clone()], 2, 86_400),
        (vec![k.clone()], 1, 86_399),
        (
            vec![VaultKey {
                x: B256::ZERO,
                y: B256::ZERO,
            }],
            1,
            86_400,
        ),
        (vec![k.clone(), k.clone()], 1, 86_400),
        (vec![k; 9], 1, 86_400),
    ] {
        h.deploy_revert(
            "core/EastSeaVault",
            VaultConfig::abi_encode_params(&(keys, threshold, 1u128, delay)),
            "EastSeaVault/invalid-constructor",
        );
    }
}

#[test]
fn account_collection_bounds_owner_only_mutators_and_failed_batches() {
    let mut h = Harness::new();
    let implementation = h.deploy("core/EastSeaAccount", vec![]);
    let receiver = h.deploy("support/NativeCallback", vec![]);
    delegate(&mut h, 0, implementation);
    let a = h.addr(0);
    let guardian = xy(&h.signer(1));
    let limits = acc::SessionLimits {
        per_payment: u128::MAX,
        per_day: u128::MAX,
        expires: 0,
        allow: vec![],
    };
    for input in [
        acc::encode_execute(&[]),
        acc::encode_set_guardian(guardian.0, guardian.1),
        acc::encode_set_guardians(&[guardian], 1, 600),
        acc::encode_add_guardian(guardian.0, guardian.1),
        acc::encode_cancel_recovery(),
        acc::encode_add_owner(guardian.0, guardian.1),
        removeOwnerCall { index: U256::ZERO }.abi_encode().into(),
        acc::encode_add_session(guardian.0, guardian.1, &limits),
        acc::encode_remove_session(0),
        acc::encode_set_session_token(0, receiver, 1, 1),
    ] {
        let receipt = h.revert(
            3,
            a,
            input.to_vec(),
            U256::ZERO,
            "EastSeaAccount/only-self-mutation",
        );
        assert_eq!(
            receipt.output.as_ref(),
            &keccak256(b"OnlySelf()").as_slice()[..4]
        );
    }
    // Distinct valid guardian keys reach the exact maximum; the ninth fails.
    for actor in 1..=8 {
        let (x, y) = xy(&h.signer(actor));
        self_call(
            &mut h,
            0,
            acc::encode_add_guardian(x, y),
            "EastSeaAccount/guardian-up-to-eight",
        );
    }
    let (x, y) = xy(&h.signer(9));
    h.revert(
        0,
        a,
        acc::encode_add_guardian(x, y).to_vec(),
        U256::ZERO,
        "EastSeaAccount/guardian-cap",
    );
    for actor in 1..=8 {
        let (x, y) = xy(&h.signer(actor));
        self_call(
            &mut h,
            0,
            acc::encode_add_owner(x, y),
            "EastSeaAccount/owner-up-to-eight",
        );
    }
    h.revert(
        0,
        a,
        acc::encode_add_owner(x, y).to_vec(),
        U256::ZERO,
        "EastSeaAccount/owner-cap",
    );
    for _ in 0..8 {
        self_call(
            &mut h,
            0,
            acc::encode_add_session(guardian.0, guardian.1, &limits),
            "EastSeaAccount/session-up-to-eight",
        );
    }
    h.revert(
        0,
        a,
        acc::encode_add_session(x, y, &limits).to_vec(),
        U256::ZERO,
        "EastSeaAccount/session-cap",
    );
    self_call(
        &mut h,
        0,
        acc::encode_remove_session(0),
        "EastSeaAccount/session-remove-swaps-last",
    );
    let calls = vec![(h.addr(2), U256::ONE, Bytes::new())];
    let input = session_input(&h, &calls, 8, 0);
    h.ok(
        3,
        a,
        input,
        U256::ZERO,
        "EastSeaAccount/moved-session-retains-id",
    );
    // Failure of a later call rolls back an earlier successful transfer.
    h.ok(
        0,
        receiver,
        setRejectPaymentCall { reject: true }.abi_encode(),
        U256::ZERO,
        "EastSeaAccount/reject-receiver-configure",
    );
    let calls = vec![
        (h.addr(2), U256::ONE, Bytes::new()),
        (receiver, U256::ONE, Bytes::new()),
    ];
    h.revert(
        0,
        a,
        acc::encode_execute(&calls).to_vec(),
        U256::ZERO,
        "EastSeaAccount/batch-transfer-atomicity",
    );
    let (r, s) = sig(&h.signer(1), &acc::owner_message(CHAIN, a, 0, &calls));
    h.revert(
        3,
        a,
        acc::encode_owner_execute(&calls, 0, r.0, s.0).to_vec(),
        U256::ZERO,
        "EastSeaAccount/owner-transfer-failure",
    );
    let (r, s) = sig(&h.signer(1), &acc::recovery_message(CHAIN, a, 0, &calls));
    h.revert(
        3,
        a,
        acc::encode_propose_recovery(&calls, &[(9, r.0, s.0)]).to_vec(),
        U256::ZERO,
        "EastSeaAccount/recovery-index-out-of-range",
    );
    h.revert(
        3,
        a,
        acc::encode_propose_recovery(&calls, &[(0, r.0, s.0), (0, r.0, s.0)]).to_vec(),
        U256::ZERO,
        "EastSeaAccount/recovery-duplicate-index",
    );
    h.ok(
        3,
        a,
        acc::encode_propose_recovery(&calls, &[(0, r.0, s.0)]).to_vec(),
        U256::ZERO,
        "EastSeaAccount/failing-recovery-propose",
    );
    h.at(h.timestamp() + acc::DEFAULT_DELAY + 1);
    h.revert(
        3,
        a,
        acc::encode_execute_recovery(&calls).to_vec(),
        U256::ZERO,
        "EastSeaAccount/recovery-transfer-failure",
    );
    self_call(
        &mut h,
        0,
        acc::encode_cancel_recovery(),
        "EastSeaAccount/failed-recovery-still-cancelable",
    );
}

#[test]
fn release_log_duplicate_manifests_and_archives_are_append_only() {
    let mut h = Harness::new();
    let log = aether_execution::release_log::ADDRESS;
    h.state
        .set_code(log, aether_execution::release_log::code())
        .unwrap();
    let manifest = Bytes::from_static(b"same-manifest");
    let archive = B256::repeat_byte(0xab);
    for actor in [1, 2, 3] {
        h.ok(
            actor,
            log,
            publishCall {
                manifest: manifest.clone(),
                archiveSha256: archive,
                builderSigs: Bytes::from(vec![actor]),
                emergency: actor == 3,
            }
            .abi_encode(),
            U256::ZERO,
            "ReleaseLog/duplicate-content-appends",
        );
    }
    assert_eq!(
        U256::abi_decode(&h.view(0, log, countCall {}.abi_encode())).unwrap(),
        U256::from(3)
    );
    // Contract intentionally keeps all posts: trust filtering/deduplication is
    // a client decision and neither manifest nor archive hash is a unique key.
    let base = U256::from_be_bytes(keccak256(U256::ZERO.to_be_bytes::<32>()).0);
    for index in 0..3 {
        assert_eq!(
            h.state.storage(&log, base + U256::from(index * 4 + 1)),
            U256::from_be_bytes(archive.0)
        );
    }
}

#[test]
fn registry_attestations_bind_every_field_and_registrar_rotation() {
    let mut h = Harness::new();
    let deployed = h.deploy("core/CommitteeRegistryV3", vec![]);
    let registry = aether_execution::registry::REGISTRY;
    h.state.set_code(registry, h.state.code(&deployed)).unwrap();
    h.label(registry, "core/CommitteeRegistryV3");
    let registrar = xy(&h.signer(1));
    aether_execution::registry::set_registrar(&mut h.state, registrar);
    h.state.set_storage(registry, U256::from(4), U256::from(10));
    let operator = h.addr(2);
    let beaconer = h.addr(3);
    let vk = B256::repeat_byte(31);
    let node = B256::repeat_byte(32);
    let correct = (U256::from(CHAIN), registry, operator, vk, node, beaconer);
    let mut messages = vec![];
    for field in 0..6 {
        let mut changed = correct;
        match field {
            0 => changed.0 = U256::from(CHAIN + 1),
            1 => changed.1 = deployed,
            2 => changed.2 = h.addr(4),
            3 => changed.3 = B256::repeat_byte(33),
            4 => changed.4 = B256::repeat_byte(34),
            _ => changed.5 = h.addr(4),
        }
        messages.push(changed.abi_encode_params());
    }
    for message in messages {
        let (r, s) = sig(&h.signer(1), &message);
        h.revert(
            2,
            registry,
            registerCall {
                validatorKey: vk,
                nodeId: node,
                beaconer,
                r,
                s,
            }
            .abi_encode(),
            U256::ZERO,
            "CommitteeRegistryV3/attestation-domain-and-field-binding",
        );
    }
    let (r, s) = sig(&h.signer(4), &correct.abi_encode_params());
    h.revert(
        2,
        registry,
        registerCall {
            validatorKey: vk,
            nodeId: node,
            beaconer,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/impostor-registrar",
    );
    let (r, s) = sig(&h.signer(1), &correct.abi_encode_params());
    h.revert(
        4,
        registry,
        registerCall {
            validatorKey: vk,
            nodeId: node,
            beaconer,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/attestation-stolen-by-caller",
    );
    let next = xy(&h.signer(5));
    aether_execution::registry::set_registrar(&mut h.state, next);
    h.revert(
        2,
        registry,
        registerCall {
            validatorKey: vk,
            nodeId: node,
            beaconer,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/retired-registrar",
    );
    let (r, s) = sig(&h.signer(5), &correct.abi_encode_params());
    h.ok(
        2,
        registry,
        registerCall {
            validatorKey: vk,
            nodeId: node,
            beaconer,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/rotated-registrar",
    );
    aether_execution::registry::set_registrar(&mut h.state, ([0; 32], [0; 32]));
    let other = B256::repeat_byte(35);
    let (r, s) = sig(
        &h.signer(5),
        &(U256::from(CHAIN), registry, operator, other, node, beaconer).abi_encode_params(),
    );
    h.revert(
        2,
        registry,
        registerCall {
            validatorKey: other,
            nodeId: node,
            beaconer,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/registrar-revoked",
    );
    assert_eq!(aether_execution::registry::candidates(&h.state).len(), 1);
    let sentinel = B256::repeat_byte(36);
    let slot = U256::from_be_bytes(keccak256((sentinel, U256::from(3)).abi_encode_params()).0);
    h.state.set_storage(registry, slot, U256::MAX);
    h.revert(
        3,
        registry,
        beaconCall {
            validatorKey: sentinel,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/reserve-key-beacon",
    );
    h.revert(
        3,
        registry,
        announceLeavingCall {
            validatorKey: sentinel,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/reserve-key-leaving",
    );
    h.revert(
        3,
        registry,
        announceBackCall {
            validatorKey: sentinel,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/reserve-key-back",
    );
    h.revert(
        2,
        registry,
        registerCall {
            validatorKey: sentinel,
            nodeId: node,
            beaconer,
            r,
            s,
        }
        .abi_encode(),
        U256::ZERO,
        "CommitteeRegistryV3/reserve-key-register",
    );
}
