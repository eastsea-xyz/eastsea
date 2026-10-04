// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// Code that an EastSea account (a P-256 Secure Enclave key) delegates to with
/// EIP-7702: the account keeps its address and key, and gains batched calls and
/// social recovery. It runs in the account's own context, so only the account
/// itself (a tx it signed, sent to its own address) may change its settings.
///
/// Recovery (v2): k of n guardian devices propose calls (e.g. "add my new
/// device as an owner key", or "move the funds"); they run only after a delay
/// the owner chose (default 48 h), and the owner can cancel in between. Owner
/// keys added this way drive the account by signature (`ownerExecute`), so a
/// new device takes over the same address.
///
/// Session keys pay native transfers or explicitly configured ERC-20 transfers.
/// A compromised agent may still spend within the owner's limits.
///
/// Limit: EIP-7702 cannot revoke the account's original key. Recovery is for a
/// lost key; whoever holds a stolen original key can still move funds.
contract EastSeaAccount {
    struct Call {
        address to;
        uint256 value;
        bytes data;
    }

    /// Uncompressed P-256 public key.
    struct Key {
        bytes32 x;
        bytes32 y;
    }

    error OnlySelf();
    error CallFailed(uint256 index, bytes reason);
    error BadGuardians();
    error NoGuardians();
    error BadSignatures();
    error RecoveryPending();
    error NoPendingRecovery();
    error NotYet(uint64 readyAt);
    error WrongCalls();
    error BadOwnerSignature();
    error BadSession();
    error SessionExpired();
    error NotAPayment(uint256 index);
    error RecipientNotAllowed(address to);
    error OverPaymentLimit(uint256 total, uint256 limit);
    error OverDailyLimit(uint256 spent, uint256 limit);
    error TokenNotAllowed(address token);
    error TokenTransferFailed(uint256 index);

    event Executed(uint256 calls);
    event GuardiansSet(uint256 count, uint8 threshold, uint64 delay);
    event RecoveryProposed(uint256 nonce, bytes32 callsHash, uint64 readyAt);
    event RecoveryCancelled(bytes32 callsHash);
    event RecoveryExecuted(bytes32 callsHash, uint256 calls);
    event OwnerAdded(bytes32 x, bytes32 y);
    event OwnerRemoved(bytes32 x, bytes32 y);
    event OwnerExecuted(uint256 nonce, uint256 calls);
    event SessionAdded(uint256 index, bytes32 x, bytes32 y, uint128 perPayment, uint128 perDay, uint64 expires);
    event SessionRemoved(uint256 index);
    event SessionPaid(uint256 index, uint256 nonce, uint256 total);
    event SessionTokenSet(uint256 index, address token, uint128 perPayment, uint128 perDay);

    /// P256VERIFY (EIP-7951 / RIP-7212): sha256 digest, r, s, x, y -> 1 on success.
    address constant P256VERIFY = address(0x100);
    uint256 constant MAX_GUARDIANS = 8;
    uint256 constant MAX_OWNERS = 8;
    uint64 constant MIN_DELAY = 10 minutes;
    uint64 constant DEFAULT_DELAY = 48 hours;
    /// Domain tags of the two signed messages (keccak256 of "aether.recovery" / "aether.owner").
    bytes32 constant RECOVERY_TAG = 0x2c233498054fa207221bfd1e68b67c7d7015e57e5a1339c519f2003dc5d79db6;
    bytes32 constant OWNER_TAG = 0x5b1924c9dfca7a9507bcc724f7e21846bad5b98db30fd8c17bb48c142374b010;
    /// keccak256("aether.session")
    bytes32 constant SESSION_TAG = 0x2e4a90ade1d79b1a035dc199dd5094be459e7c7e62f5f46349a2f67f0e9cd513;
    uint256 constant MAX_SESSIONS = 8;
    uint256 constant MAX_ALLOWED = 16;
    bytes4 constant TRANSFER_SELECTOR = 0xa9059cbb;

    /// Storage lives in the delegating account itself, so use a namespaced slot
    /// (ERC-7201) no other code at this address will collide with:
    /// keccak256(abi.encode(uint256(keccak256("aether.account.recovery.v2")) - 1)) & ~0xff
    bytes32 constant STATE_SLOT = 0xadff66301d86ff00bc4d9a197c134a0e89462a08ef2bef88b92f69282c7ee500;

    struct State {
        Key[] guardians;
        uint8 threshold;
        uint64 delay;
        /// Proposals made so far (each proposal's signatures cover its nonce).
        uint256 recoveryNonce;
        /// keccak256(abi.encode(calls)) of the pending proposal, or zero.
        bytes32 pending;
        uint64 readyAt;
        Key[] owners;
        uint256 ownerNonce;
        Session[] sessions;
        /// Every session ever added gets a new id, which its signatures cover.
        uint256 sessionSerial;
        mapping(uint256 => mapping(address => TokenLimit)) tokenLimits;
    }

    struct TokenLimit {
        uint128 perPayment;
        uint128 perDay;
        uint64 day;
        uint128 spent;
        uint128 prevSpent;
    }

    struct Session {
        Key key;
        uint128 perPayment;
        uint128 perDay;
        /// Paid in UTC day `day` (unix time / 1 day) and in the day before it.
        uint64 day;
        /// Unix time after which the key stops working (0 = never).
        uint64 expires;
        uint128 spent;
        uint128 prevSpent;
        /// Allowed recipients; empty = anyone.
        address[] allow;
        uint256 nonce;
        /// Unique per account and never reused: a removed and re-added key starts
        /// a new signature domain, so its old signatures cannot replay.
        uint256 id;
    }

    function _state() private pure returns (State storage st) {
        bytes32 slot = STATE_SLOT;
        assembly {
            st.slot := slot
        }
    }

    modifier onlySelf() {
        if (msg.sender != address(this)) revert OnlySelf();
        _;
    }

    // ---- calls ----

    /// Several calls, all or nothing, under one signature (one Touch ID).
    function execute(Call[] calldata calls) external payable onlySelf {
        _run(calls);
        emit Executed(calls.length);
    }

    function _run(Call[] calldata calls) private {
        for (uint256 i = 0; i < calls.length; i++) {
            (bool ok, bytes memory reason) = calls[i].to.call{value: calls[i].value}(calls[i].data);
            if (!ok) revert CallFailed(i, reason);
        }
    }

    function _verify(bytes32 digest, bytes32 r, bytes32 s, Key memory k) private view returns (bool) {
        (bool ok, bytes memory out) = P256VERIFY.staticcall(abi.encodePacked(digest, r, s, k.x, k.y));
        return ok && out.length == 32 && abi.decode(out, (uint256)) == 1;
    }

    // ---- guardians and delayed recovery ----

    /// Set the recovery devices: `threshold` of `keys` must sign a proposal, which
    /// runs `delay` seconds later unless the owner cancels. Clears any pending
    /// proposal. An empty list (threshold 0) turns recovery off.
    function setGuardians(Key[] calldata keys, uint8 threshold, uint64 delay) public onlySelf {
        if (keys.length > MAX_GUARDIANS) revert BadGuardians();
        if (keys.length == 0 ? threshold != 0 : (threshold == 0 || threshold > keys.length)) revert BadGuardians();
        if (keys.length > 0 && delay < MIN_DELAY) revert BadGuardians();
        for (uint256 i = 0; i < keys.length; i++) {
            if (keys[i].x == bytes32(0) && keys[i].y == bytes32(0)) revert BadGuardians();
            for (uint256 j = 0; j < i; j++) {
                if (keys[j].x == keys[i].x && keys[j].y == keys[i].y) revert BadGuardians();
            }
        }
        State storage st = _state();
        delete st.guardians;
        for (uint256 i = 0; i < keys.length; i++) {
            st.guardians.push(keys[i]);
        }
        st.threshold = threshold;
        st.delay = delay;
        _clearPending(st);
        emit GuardiansSet(keys.length, threshold, delay);
    }

    /// One recovery device with the default 48 h delay (zeros turn recovery off).
    function setGuardian(bytes32 x, bytes32 y) external onlySelf {
        Key[] memory keys = new Key[](x == bytes32(0) && y == bytes32(0) ? 0 : 1);
        if (keys.length == 1) keys[0] = Key(x, y);
        this.setGuardians(keys, uint8(keys.length), DEFAULT_DELAY);
    }

    /// Add one recovery device next to the current ones, keeping the threshold
    /// and delay (the first device: 1-of-1, default delay). It reads the list on
    /// chain, so a wallet never rewrites guardians from a possibly stale copy.
    function addGuardian(bytes32 x, bytes32 y) external onlySelf {
        State storage st = _state();
        if (x == bytes32(0) && y == bytes32(0)) revert BadGuardians();
        if (st.guardians.length >= MAX_GUARDIANS) revert BadGuardians();
        for (uint256 i = 0; i < st.guardians.length; i++) {
            if (st.guardians[i].x == x && st.guardians[i].y == y) revert BadGuardians();
        }
        if (st.guardians.length == 0) {
            st.threshold = 1;
            st.delay = DEFAULT_DELAY;
        }
        st.guardians.push(Key(x, y));
        emit GuardiansSet(st.guardians.length, st.threshold, st.delay);
    }

    function guardians()
        external
        view
        returns (Key[] memory keys, uint8 threshold, uint64 delay, uint256 recoveryNonce, bytes32 pending, uint64 readyAt)
    {
        State storage st = _state();
        return (st.guardians, st.threshold, st.delay, st.recoveryNonce, st.pending, st.readyAt);
    }

    /// What each guardian signs (SHA-256, as a Secure Enclave signs).
    function recoveryDigest(Call[] calldata calls, uint256 nonce) public view returns (bytes32) {
        return sha256(abi.encode(block.chainid, address(this), RECOVERY_TAG, nonce, calls));
    }

    /// Start a recovery: `threshold` guardian signatures over these calls and the
    /// current proposal nonce, from distinct guardians (`idx` strictly increasing).
    /// Anyone may relay it. The calls run after the delay via `executeRecovery`.
    function proposeRecovery(Call[] calldata calls, uint8[] calldata idx, bytes32[] calldata r, bytes32[] calldata s) external {
        State storage st = _state();
        if (st.threshold == 0) revert NoGuardians();
        if (st.pending != bytes32(0)) revert RecoveryPending();
        if (idx.length < st.threshold || idx.length != r.length || idx.length != s.length) revert BadSignatures();
        uint256 nonce = st.recoveryNonce;
        bytes32 digest = recoveryDigest(calls, nonce);
        for (uint256 i = 0; i < idx.length; i++) {
            if (i > 0 && idx[i] <= idx[i - 1]) revert BadSignatures();
            if (idx[i] >= st.guardians.length) revert BadSignatures();
            if (!_verify(digest, r[i], s[i], st.guardians[idx[i]])) revert BadSignatures();
        }
        st.recoveryNonce = nonce + 1;
        st.pending = keccak256(abi.encode(calls));
        st.readyAt = uint64(block.timestamp) + st.delay;
        emit RecoveryProposed(nonce, st.pending, st.readyAt);
    }

    /// Run the pending recovery once its delay has passed. Anyone may call it.
    function executeRecovery(Call[] calldata calls) external {
        State storage st = _state();
        bytes32 h = st.pending;
        if (h == bytes32(0)) revert NoPendingRecovery();
        if (block.timestamp < st.readyAt) revert NotYet(st.readyAt);
        if (keccak256(abi.encode(calls)) != h) revert WrongCalls();
        _clearPending(st);
        _run(calls);
        emit RecoveryExecuted(h, calls.length);
    }

    /// The owner stops a pending recovery (e.g. one the owner did not ask for).
    function cancelRecovery() external onlySelf {
        State storage st = _state();
        if (st.pending == bytes32(0)) revert NoPendingRecovery();
        emit RecoveryCancelled(st.pending);
        _clearPending(st);
    }

    function _clearPending(State storage st) private {
        st.pending = bytes32(0);
        st.readyAt = 0;
    }

    // ---- owner keys (new devices that took over the account) ----

    function addOwner(Key calldata key) external onlySelf {
        State storage st = _state();
        if (st.owners.length >= MAX_OWNERS || (key.x == bytes32(0) && key.y == bytes32(0))) revert BadGuardians();
        st.owners.push(key);
        emit OwnerAdded(key.x, key.y);
    }

    function removeOwner(uint256 index) external onlySelf {
        State storage st = _state();
        Key memory k = st.owners[index];
        st.owners[index] = st.owners[st.owners.length - 1];
        st.owners.pop();
        emit OwnerRemoved(k.x, k.y);
    }

    function owners() external view returns (Key[] memory keys, uint256 nonce) {
        State storage st = _state();
        return (st.owners, st.ownerNonce);
    }

    /// What an owner key signs.
    function ownerDigest(Call[] calldata calls, uint256 nonce) public view returns (bytes32) {
        return sha256(abi.encode(block.chainid, address(this), OWNER_TAG, nonce, calls));
    }

    /// Calls authorized by an owner key's signature. Anyone may relay it; the
    /// nonce prevents replay. Self-calls here can change settings (e.g. cancel a
    /// recovery or replace guardians).
    function ownerExecute(Call[] calldata calls, uint256 keyIndex, bytes32 r, bytes32 s) external {
        State storage st = _state();
        if (keyIndex >= st.owners.length) revert BadOwnerSignature();
        uint256 nonce = st.ownerNonce;
        if (!_verify(ownerDigest(calls, nonce), r, s, st.owners[keyIndex])) revert BadOwnerSignature();
        st.ownerNonce = nonce + 1;
        _run(calls);
        emit OwnerExecuted(nonce, calls.length);
    }

    // ---- session keys (limited, e.g. for AI agents) ----

    function addSession(Key calldata key, uint128 perPayment, uint128 perDay, uint64 expires, address[] calldata allow) external onlySelf {
        State storage st = _state();
        if (st.sessions.length >= MAX_SESSIONS || (key.x == bytes32(0) && key.y == bytes32(0))) revert BadSession();
        if (perPayment == 0 || perPayment > perDay || allow.length > MAX_ALLOWED) revert BadSession();
        Session storage ss = st.sessions.push();
        ss.id = ++st.sessionSerial;
        ss.key = key;
        ss.perPayment = perPayment;
        ss.perDay = perDay;
        ss.expires = expires;
        for (uint256 i = 0; i < allow.length; i++) {
            ss.allow.push(allow[i]);
        }
        emit SessionAdded(st.sessions.length - 1, key.x, key.y, perPayment, perDay, expires);
    }

    /// Remove a session key (the last one takes its index).
    function removeSession(uint256 index) external onlySelf {
        State storage st = _state();
        if (index >= st.sessions.length) revert BadSession();
        uint256 last = st.sessions.length - 1;
        if (index != last) {
            Session storage dst = st.sessions[index];
            Session storage src = st.sessions[last];
            dst.key = src.key;
            dst.perPayment = src.perPayment;
            dst.perDay = src.perDay;
            dst.day = src.day;
            dst.spent = src.spent;
            dst.prevSpent = src.prevSpent;
            dst.expires = src.expires;
            delete dst.allow;
            for (uint256 i = 0; i < src.allow.length; i++) {
                dst.allow.push(src.allow[i]);
            }
            dst.nonce = src.nonce;
            dst.id = src.id;
        }
        delete st.sessions[last].allow;
        st.sessions.pop();
        emit SessionRemoved(index);
    }

    /// Token amounts are base units of this token, never AETH wei. A removed
    /// session's id cannot be reused, so its old token permissions stay inert.
    function setSessionToken(uint256 index, address token, uint128 perPayment, uint128 perDay) external onlySelf {
        State storage st = _state();
        if (index >= st.sessions.length || token == address(0) || token == address(this) || token.code.length == 0
            || perPayment == 0 || perPayment > perDay) revert BadSession();
        TokenLimit storage limit = st.tokenLimits[st.sessions[index].id][token];
        limit.perPayment = perPayment;
        limit.perDay = perDay;
        limit.day = 0;
        limit.spent = 0;
        limit.prevSpent = 0;
        emit SessionTokenSet(index, token, perPayment, perDay);
    }

    function sessionToken(uint256 index, address token) external view returns (TokenLimit memory) {
        State storage st = _state();
        if (index >= st.sessions.length) revert BadSession();
        return st.tokenLimits[st.sessions[index].id][token];
    }

    function sessionCount() external view returns (uint256) {
        return _state().sessions.length;
    }

    function session(uint256 index) external view returns (Session memory) {
        return _state().sessions[index];
    }

    /// What a session key signs: bound to the session's id, not its index.
    function sessionDigest(Call[] calldata calls, uint256 id, uint256 nonce) public view returns (bytes32) {
        return sha256(abi.encode(block.chainid, address(this), SESSION_TAG, id, nonce, calls));
    }

    /// Plain payments signed by a session key, within its limits. Anyone may
    /// relay it (usually the session key's own address, which pays the gas).
    function sessionExecute(Call[] calldata calls, uint256 index, bytes32 r, bytes32 s) external {
        State storage st = _state();
        if (index >= st.sessions.length) revert BadSession();
        Session storage ss = st.sessions[index];
        if (ss.expires != 0 && block.timestamp >= ss.expires) revert SessionExpired();
        uint256 nonce = ss.nonce;
        if (!_verify(sessionDigest(calls, ss.id, nonce), r, s, ss.key)) revert BadSession();
        uint256 total = 0;
        for (uint256 i = 0; i < calls.length; i++) {
            if (calls[i].to == address(this)) revert NotAPayment(i);
            if (calls[i].data.length == 0) {
                if (ss.allow.length > 0 && !_allowed(ss.allow, calls[i].to)) revert RecipientNotAllowed(calls[i].to);
                total += calls[i].value;
            } else {
                if (calls[i].to.code.length == 0) revert TokenNotAllowed(calls[i].to);
                (address recipient, uint256 amount) = _tokenTransfer(calls[i], i);
                if (ss.allow.length > 0 && !_allowed(ss.allow, recipient)) revert RecipientNotAllowed(recipient);
                // Count every call to this token in the signed batch as one
                // payment. Only the first occurrence updates its daily usage.
                bool first = true;
                for (uint256 j = 0; j < i; j++) {
                    if (calls[j].to == calls[i].to && calls[j].data.length != 0) first = false;
                }
                if (first) {
                    for (uint256 j = i + 1; j < calls.length; j++) {
                        if (calls[j].to == calls[i].to && calls[j].data.length != 0) {
                            (, uint256 nextAmount) = _tokenTransfer(calls[j], j);
                            amount += nextAmount;
                        }
                    }
                    _spendToken(st.tokenLimits[ss.id][calls[i].to], calls[i].to, amount);
                }
            }
        }
        if (total > ss.perPayment) revert OverPaymentLimit(total, ss.perPayment);
        // Any 24 hours overlap at most two consecutive UTC days, so capping each
        // day plus the day before caps every 24-hour span at perDay.
        uint64 today = uint64(block.timestamp / 1 days);
        if (today != ss.day) {
            ss.prevSpent = today == ss.day + 1 ? ss.spent : 0;
            ss.spent = 0;
            ss.day = today;
        }
        uint256 used = uint256(ss.prevSpent) + ss.spent + total;
        if (used > ss.perDay) revert OverDailyLimit(used, ss.perDay);
        ss.spent += uint128(total);
        ss.nonce = nonce + 1;
        _runSession(calls);
        emit SessionPaid(index, nonce, total);
    }

    function _runSession(Call[] calldata calls) private {
        for (uint256 i = 0; i < calls.length; i++) {
            (bool ok, bytes memory result) = calls[i].to.call{value: calls[i].value}(calls[i].data);
            if (!ok) revert CallFailed(i, result);
            if (calls[i].data.length != 0 && result.length != 0
                && (result.length != 32 || !abi.decode(result, (bool)))) revert TokenTransferFailed(i);
        }
    }

    function _tokenTransfer(Call calldata c, uint256 index) private pure returns (address recipient, uint256 amount) {
        if (c.value != 0 || c.data.length != 68 || bytes4(c.data[:4]) != TRANSFER_SELECTOR) revert NotAPayment(index);
        // abi.decode also rejects malformed address padding. No trailing data,
        // approve, transferFrom, or arbitrary token method can enter this path.
        (recipient, amount) = abi.decode(c.data[4:], (address, uint256));
        if (recipient == address(0) || amount == 0) revert NotAPayment(index);
    }

    function _spendToken(TokenLimit storage limit, address token, uint256 amount) private {
        if (limit.perPayment == 0) revert TokenNotAllowed(token);
        if (amount > limit.perPayment) revert OverPaymentLimit(amount, limit.perPayment);
        uint64 today = uint64(block.timestamp / 1 days);
        if (today != limit.day) {
            limit.prevSpent = today == limit.day + 1 ? limit.spent : 0;
            limit.spent = 0;
            limit.day = today;
        }
        uint256 used = uint256(limit.prevSpent) + limit.spent + amount;
        if (used > limit.perDay) revert OverDailyLimit(used, limit.perDay);
        limit.spent += uint128(amount);
    }

    function _allowed(address[] storage allow, address to) private view returns (bool) {
        for (uint256 i = 0; i < allow.length; i++) {
            if (allow[i] == to) return true;
        }
        return false;
    }

    receive() external payable {}
}
