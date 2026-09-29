//! Calls, signed messages and storage layout of `AetherAccount`
//! (contracts/src/AetherAccount.sol): batched calls, k-of-n guardians with a
//! delayed, cancellable recovery, and owner keys that take over an address.

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use alloy_sol_types::{sol, SolCall, SolValue};

sol! {
    struct Call { address to; uint256 value; bytes data; }
    struct Key { bytes32 x; bytes32 y; }

    function execute(Call[] calls);
    function setGuardian(bytes32 x, bytes32 y);
    function setGuardians(Key[] keys, uint8 threshold, uint64 delay);
    function addGuardian(bytes32 x, bytes32 y);
    function proposeRecovery(Call[] calls, uint8[] idx, bytes32[] r, bytes32[] s);
    function executeRecovery(Call[] calls);
    function cancelRecovery();
    function addOwner(Key key);
    function removeOwner(uint256 index);
    function ownerExecute(Call[] calls, uint256 keyIndex, bytes32 r, bytes32 s);
    function addSession(Key key, uint128 perPayment, uint128 perDay, uint64 expires, address[] allow);
    function removeSession(uint256 index);
    function sessionExecute(Call[] calls, uint256 index, bytes32 r, bytes32 s);
}

/// Calls for `AetherAccount`: (to, value, data).
pub type AccountCall = (Address, U256, Bytes);

/// Recovery runs this long after it is proposed unless the owner chose otherwise.
pub const DEFAULT_DELAY: u64 = 48 * 3600;
/// The shortest delay an owner may set.
pub const MIN_DELAY: u64 = 600;
const RECOVERY_TAG: B256 = alloy_primitives::b256!("2c233498054fa207221bfd1e68b67c7d7015e57e5a1339c519f2003dc5d79db6");
const OWNER_TAG: B256 = alloy_primitives::b256!("5b1924c9dfca7a9507bcc724f7e21846bad5b98db30fd8c17bb48c142374b010");
const SESSION_TAG: B256 = alloy_primitives::b256!("2e4a90ade1d79b1a035dc199dd5094be459e7c7e62f5f46349a2f67f0e9cd513");
/// ERC-7201 slot of the account's `State` struct ("aether.account.recovery.v2").
pub const STATE_SLOT: B256 = alloy_primitives::b256!("adff66301d86ff00bc4d9a197c134a0e89462a08ef2bef88b92f69282c7ee500");

fn calls(c: &[AccountCall]) -> Vec<Call> {
    c.iter().map(|(to, value, data)| Call { to: *to, value: *value, data: data.clone() }).collect()
}

/// `execute(calls)`: several calls under one signature.
pub fn encode_execute(c: &[AccountCall]) -> Bytes {
    executeCall { calls: calls(c) }.abi_encode().into()
}

/// Decode a direct `execute` call for the node's non-consensus activity index.
pub fn decode_execute(input: &[u8]) -> Option<Vec<AccountCall>> {
    let call = executeCall::abi_decode_validate(input).ok()?;
    Some(call.calls.into_iter().map(|c| (c.to, c.value, c.data)).collect())
}

/// `setGuardian(x, y)`: one recovery device, default delay (zeros turn recovery off).
pub fn encode_set_guardian(x: [u8; 32], y: [u8; 32]) -> Bytes {
    setGuardianCall { x: x.into(), y: y.into() }.abi_encode().into()
}

/// `addGuardian(x, y)`: one more recovery device; the others, threshold and delay stay.
pub fn encode_add_guardian(x: [u8; 32], y: [u8; 32]) -> Bytes {
    addGuardianCall { x: x.into(), y: y.into() }.abi_encode().into()
}

/// `setGuardians(keys, threshold, delay)`.
pub fn encode_set_guardians(keys: &[([u8; 32], [u8; 32])], threshold: u8, delay: u64) -> Bytes {
    let keys = keys.iter().map(|(x, y)| Key { x: (*x).into(), y: (*y).into() }).collect();
    setGuardiansCall { keys, threshold, delay }.abi_encode().into()
}

/// The bytes each guardian signs for proposal `nonce` (the contract checks sha256 of them).
pub fn recovery_message(chain_id: u64, account: Address, nonce: u64, c: &[AccountCall]) -> Vec<u8> {
    (U256::from(chain_id), account, RECOVERY_TAG, U256::from(nonce), calls(c)).abi_encode_params()
}

/// `proposeRecovery(calls, idx, r, s)` from `(guardian index, r, s)`, any order.
pub fn encode_propose_recovery(c: &[AccountCall], sigs: &[(u8, [u8; 32], [u8; 32])]) -> Bytes {
    let mut sigs = sigs.to_vec();
    sigs.sort_by_key(|(i, _, _)| *i);
    proposeRecoveryCall {
        calls: calls(c),
        idx: sigs.iter().map(|(i, _, _)| *i).collect(),
        r: sigs.iter().map(|(_, r, _)| B256::from(*r)).collect(),
        s: sigs.iter().map(|(_, _, s)| B256::from(*s)).collect(),
    }
    .abi_encode()
    .into()
}

/// `executeRecovery(calls)`: after the delay, anyone runs the proposed calls.
pub fn encode_execute_recovery(c: &[AccountCall]) -> Bytes {
    executeRecoveryCall { calls: calls(c) }.abi_encode().into()
}

/// `cancelRecovery()` (the owner, via its own tx or an owner key).
pub fn encode_cancel_recovery() -> Bytes {
    cancelRecoveryCall {}.abi_encode().into()
}

/// `addOwner(key)`: a new device key that may drive the account by signature.
pub fn encode_add_owner(x: [u8; 32], y: [u8; 32]) -> Bytes {
    addOwnerCall { key: Key { x: x.into(), y: y.into() } }.abi_encode().into()
}

/// The bytes an owner key signs for `ownerExecute` with `nonce`.
pub fn owner_message(chain_id: u64, account: Address, nonce: u64, c: &[AccountCall]) -> Vec<u8> {
    (U256::from(chain_id), account, OWNER_TAG, U256::from(nonce), calls(c)).abi_encode_params()
}

/// `ownerExecute(calls, keyIndex, r, s)`.
pub fn encode_owner_execute(c: &[AccountCall], key_index: u64, r: [u8; 32], s: [u8; 32]) -> Bytes {
    ownerExecuteCall { calls: calls(c), keyIndex: U256::from(key_index), r: r.into(), s: s.into() }.abi_encode().into()
}

/// A session key's limits (amounts in wei; `expires` unix seconds, 0 = never).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionLimits {
    pub per_payment: u128,
    pub per_day: u128,
    pub expires: u64,
    /// Allowed recipients; empty = anyone.
    pub allow: Vec<Address>,
}

/// `addSession(key, perPayment, perDay, expires, allow)`.
pub fn encode_add_session(x: [u8; 32], y: [u8; 32], l: &SessionLimits) -> Bytes {
    addSessionCall { key: Key { x: x.into(), y: y.into() }, perPayment: l.per_payment, perDay: l.per_day, expires: l.expires, allow: l.allow.clone() }
        .abi_encode()
        .into()
}

/// `removeSession(index)`.
pub fn encode_remove_session(index: u64) -> Bytes {
    removeSessionCall { index: U256::from(index) }.abi_encode().into()
}

/// The bytes a session key signs for payment `nonce` of the session with `id`
/// (its unique id, not its index, so a re-added key cannot replay old signatures).
pub fn session_message(chain_id: u64, account: Address, id: u64, nonce: u64, c: &[AccountCall]) -> Vec<u8> {
    (U256::from(chain_id), account, SESSION_TAG, U256::from(id), U256::from(nonce), calls(c)).abi_encode_params()
}

/// `sessionExecute(calls, index, r, s)`.
pub fn encode_session_execute(c: &[AccountCall], index: u64, r: [u8; 32], s: [u8; 32]) -> Bytes {
    sessionExecuteCall { calls: calls(c), index: U256::from(index), r: r.into(), s: s.into() }.abi_encode().into()
}

/// Hash the contract stores for a pending proposal (`keccak256(abi.encode(calls))`).
pub fn calls_hash(c: &[AccountCall]) -> B256 {
    keccak256(calls(c).abi_encode())
}

/// Storage slots of `State` (read with state proofs by light clients).
pub mod slots {
    use super::*;

    fn at(offset: u64) -> U256 {
        U256::from_be_bytes(STATE_SLOT.0) + U256::from(offset)
    }

    pub fn guardian_count() -> U256 {
        at(0)
    }
    /// `threshold` (uint8, low byte) and `delay` (uint64, next 8 bytes) share one slot.
    pub fn threshold_and_delay() -> U256 {
        at(1)
    }
    pub fn recovery_nonce() -> U256 {
        at(2)
    }
    pub fn pending() -> U256 {
        at(3)
    }
    pub fn ready_at() -> U256 {
        at(4)
    }
    pub fn owner_count() -> U256 {
        at(5)
    }
    pub fn owner_nonce() -> U256 {
        at(6)
    }
    /// Slot of `guardians[i].x` (`.y` is the next slot).
    pub fn guardian(i: u64) -> U256 {
        U256::from_be_bytes(keccak256(at(0).to_be_bytes::<32>()).0) + U256::from(2 * i)
    }
    /// Slot of `owners[i].x` (`.y` is the next slot).
    pub fn owner(i: u64) -> U256 {
        U256::from_be_bytes(keccak256(at(5).to_be_bytes::<32>()).0) + U256::from(2 * i)
    }

    pub fn session_count() -> U256 {
        at(7)
    }

    /// Slots of session `i` (8 per session): key.x, key.y, [perPayment | perDay],
    /// [day | expires | spent], prevSpent, allow (length), nonce, id.
    pub fn session(i: u64) -> U256 {
        U256::from_be_bytes(keccak256(at(7).to_be_bytes::<32>()).0) + U256::from(8 * i)
    }

    /// Slot of `sessions[i].allow[j]`.
    pub fn session_allow(i: u64, j: u64) -> U256 {
        U256::from_be_bytes(keccak256((session(i) + U256::from(5u64)).to_be_bytes::<32>()).0) + U256::from(j)
    }

    /// `(perPayment, perDay)` from the session's third slot.
    pub fn unpack_limits(v: U256) -> (u128, u128) {
        let b = v.to_be_bytes::<32>();
        (u128::from_be_bytes(b[16..32].try_into().expect("16")), u128::from_be_bytes(b[0..16].try_into().expect("16")))
    }

    /// `(day, expires, spent)` from the session's fourth slot.
    pub fn unpack_usage(v: U256) -> (u64, u64, u128) {
        let b = v.to_be_bytes::<32>();
        (
            u64::from_be_bytes(b[24..32].try_into().expect("8")),
            u64::from_be_bytes(b[16..24].try_into().expect("8")),
            u128::from_be_bytes(b[0..16].try_into().expect("16")),
        )
    }

    /// What a session may still pay now (at `now`, unix seconds): perDay minus what
    /// it paid today and yesterday (UTC), as the contract counts it.
    pub fn left_now(per_day: u128, day: u64, spent: u128, prev_spent: u128, now: u64) -> u128 {
        let today = now / 86_400;
        let used = if day == today {
            spent + prev_spent
        } else if day + 1 == today {
            spent
        } else {
            0
        };
        per_day.saturating_sub(used)
    }

    /// Split the packed `threshold_and_delay` word.
    pub fn unpack_threshold_and_delay(v: U256) -> (u8, u64) {
        let b = v.to_be_bytes::<32>();
        (b[31], u64::from_be_bytes(b[23..31].try_into().expect("8 bytes")))
    }
}
