// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// A shared vault ("금고", docs/design/16-vault.md): non-custodial, no admin,
/// no upgrade path, no fees. The owners are P-256 public keys (the user's Mac
/// and iPhone Secure Enclave keys, plus family or team members) with an
/// M-of-N threshold, like EastSeaAccount's guardian keys.
///
/// Money leaves two ways:
///   - `spend`: ONE owner's signature moves native AETH at once, within the
///     vault-wide daily limit (any 24 hours, UTC-day accounting as in
///     EastSeaAccount's session keys).
///   - the queue: anything else (amounts over the limit, ERC-20 transfers)
///     becomes a proposal that `threshold` distinct owners must sign, and that
///     anyone may execute `delay` seconds after the LAST needed approval.
///     ANY single owner can cancel while it is pending (a veto during the
///     delay). Owner, threshold, limit and delay changes ride the same path:
///     a settings proposal replaces the whole config and wipes the queue, so
///     approvals never outlive the owner set they were counted against.
///
/// Owners are keys, not addresses, so every owner action is a P-256 signature
/// over a digest binding chain id, the vault address and a nonce (proposal id,
/// spend nonce or cancel nonce); anyone relays the transaction.
contract EastSeaVault {
    struct Key {
        bytes32 x;
        bytes32 y;
    }

    /// What a queued proposal does. `None` marks a slot that was never created,
    /// or whose proposal was executed or cancelled.
    enum Kind {
        None,
        Withdraw,
        Settings
    }

    struct Proposal {
        Kind kind;
        /// Queue era at creation. A settings execution opens a new era, and
        /// proposals from an older one are dead (their approvals were signed
        /// by the owner set that is being replaced).
        uint256 epoch;
        /// Bitmask over current owner indices.
        uint256 approvals;
        /// 0 until the threshold is reached; then executable after this time.
        uint64 readyAt;
        // ---- withdrawals ----
        /// address(0) = native AETH.
        address token;
        address to;
        uint256 amount;
        // ---- settings (the full replacement, validated at propose time) ----
        Key[] owners;
        uint8 newThreshold;
        uint128 newDailyLimit;
        uint64 newDelay;
    }

    error BadConfig();
    error BadSignature();
    error UnknownProposal();
    error AlreadyApproved();
    error NotReady();
    error NotYet(uint64 readyAt);
    error OverDailyLimit(uint256 spent, uint256 limit);
    error TransferFailed();
    error TokenTransferFailed();
    /// F-03: the withdrawal "token" has no code, or is not shaped like an
    /// ERC-20 whose balanceOf can be read.
    error BadToken();
    /// F-03: the transfer reported success but `to` received less than the
    /// signed amount (a no-pay or fee-on-transfer token).
    error ShortDelivery(uint256 received, uint256 expected);

    event WithdrawalProposed(uint256 indexed id, address token, address indexed to, uint256 amount);
    event SettingsProposed(uint256 indexed id, Key[] newOwners, uint8 newThreshold, uint128 newDailyLimit, uint64 newDelay);
    /// `readyAt != 0` here means the threshold was just reached.
    event Approved(uint256 indexed id, uint256 indexed ownerIndex, uint256 approvals, uint64 readyAt);
    event WithdrawalExecuted(uint256 indexed id, address token, address indexed to, uint256 amount);
    /// All proposals from earlier eras died with this settings change.
    event SettingsExecuted(uint256 indexed id, uint256 indexed era);
    event Canceled(uint256 indexed id, uint256 indexed ownerIndex);
    event Spent(uint256 indexed nonce, address indexed to, uint256 amount, uint256 indexed ownerIndex, uint128 spentToday);

    /// P256VERIFY (EIP-7951 / RIP-7212): sha256 digest, r, s, x, y -> 1 on success.
    address constant P256VERIFY = address(0x100);
    uint256 constant MAX_OWNERS = 8;
    uint64 constant MIN_DELAY = 24 hours;
    /// What the app passes at creation (12-launch-plan.md: 24~48 h).
    uint64 constant DEFAULT_DELAY = 48 hours;
    /// Domain tags of the four signed messages (keccak256 of "aether.vault.*").
    bytes32 constant WITHDRAW_TAG = 0x21858ffa6f9c911943ef018a75e7da08734a90ab7da4e5d6dfccf97774c11469;
    bytes32 constant SETTINGS_TAG = 0x0210597db69caa67467d9c98b53a47f1d05b6676e7829a940f6164a894393649;
    bytes32 constant SPEND_TAG = 0x395741c5d5c1686a2a5bfd2dfbf0e21a5103dd6d7f7125d96a2082d36d9cda4a;
    bytes32 constant CANCEL_TAG = 0xc479f5f30d0b8d05e9705da782781a9966ba8b708f6d7e5fd03dd102a368fb1e;

    Key[] public owners;
    uint8 public threshold;
    uint64 public delay;
    uint128 public dailyLimit;
    /// Daily-limit accounting (any 24 hours span at most two consecutive UTC
    /// days, so capping each day plus the day before caps every span).
    uint64 public spendDay;
    uint128 public spentToday;
    uint128 public spentPrev;
    uint256 public spendNonce;
    uint256 public cancelNonce;
    /// Proposals so far; proposal ids start at 1 and are never reused.
    uint256 public proposalCount;
    uint256 public queueEra;
    mapping(uint256 => Proposal) private _proposals;

    constructor(Key[] memory keys, uint8 threshold_, uint128 dailyLimit_, uint64 delay_) {
        _checkConfig(keys, threshold_, delay_);
        for (uint256 i = 0; i < keys.length; i++) {
            owners.push(keys[i]);
        }
        threshold = threshold_;
        dailyLimit = dailyLimit_;
        delay = delay_;
    }

    receive() external payable {}

    // ---- views for the app ----

    function ownerCount() external view returns (uint256) {
        return owners.length;
    }

    function proposal(uint256 id) external view returns (Proposal memory) {
        return _proposal(id);
    }

    /// What one owner may still `spend` right now under the daily limit.
    function dailyAvailable() external view returns (uint128) {
        uint256 today = block.timestamp / 1 days;
        uint256 used = today == spendDay ? uint256(spentPrev) + spentToday : (today == spendDay + 1 ? spentToday : 0);
        return uint128(dailyLimit >= used ? dailyLimit - used : 0);
    }

    // ---- what each owner signs (SHA-256, as a Secure Enclave signs) ----

    function spendDigest(address to, uint256 amount, uint256 nonce) public view returns (bytes32) {
        return sha256(abi.encode(block.chainid, address(this), SPEND_TAG, nonce, to, amount));
    }

    function withdrawDigest(uint256 id, address token, address to, uint256 amount) public view returns (bytes32) {
        return sha256(abi.encode(block.chainid, address(this), WITHDRAW_TAG, id, token, to, amount));
    }

    function settingsDigest(uint256 id, Key[] memory keys, uint8 newThreshold, uint128 newDailyLimit, uint64 newDelay)
        public
        view
        returns (bytes32)
    {
        return sha256(abi.encode(block.chainid, address(this), SETTINGS_TAG, id, keys, newThreshold, newDailyLimit, newDelay));
    }

    function cancelDigest(uint256 id, uint256 nonce) public view returns (bytes32) {
        return sha256(abi.encode(block.chainid, address(this), CANCEL_TAG, nonce, id));
    }

    // ---- one owner, native AETH, within the daily limit ----

    /// One owner's signature moves `amount` AETH to `to` at once, if the
    /// vault's last 24 hours stay within `dailyLimit`. Anyone may relay.
    function spend(address to, uint256 amount, uint256 ownerIndex, bytes32 r, bytes32 s) external {
        Key memory k = _ownerAt(ownerIndex);
        uint256 nonce = spendNonce;
        if (!_verify(spendDigest(to, amount, nonce), r, s, k)) revert BadSignature();
        uint256 today = block.timestamp / 1 days;
        if (today != spendDay) {
            spentPrev = today == spendDay + 1 ? spentToday : 0;
            spentToday = 0;
            spendDay = uint64(today);
        }
        uint256 used = uint256(spentPrev) + spentToday + amount;
        if (used > dailyLimit) revert OverDailyLimit(used, dailyLimit);
        spendNonce = nonce + 1;
        spentToday += uint128(amount);
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert TransferFailed();
        emit Spent(nonce, to, amount, ownerIndex, spentToday);
    }

    // ---- the queue: withdrawals and settings changes ----

    /// Queue an above-limit (or ERC-20) transfer. The first owner's signature
    /// comes along; more arrive via `approve`.
    function proposeWithdrawal(address token, address to, uint256 amount, uint256 ownerIndex, bytes32 r, bytes32 s)
        external
        returns (uint256 id)
    {
        Key memory k = _ownerAt(ownerIndex);
        id = ++proposalCount;
        if (!_verify(withdrawDigest(id, token, to, amount), r, s, k)) revert BadSignature();
        // F-03: refuse a no-code "token" at the door, so owners cannot queue
        // (and later "execute") a withdrawal that can never pay anything.
        if (token != address(0) && token.code.length == 0) revert BadToken();
        Proposal storage p = _proposals[id];
        p.kind = Kind.Withdraw;
        p.epoch = queueEra;
        p.token = token;
        p.to = to;
        p.amount = amount;
        emit WithdrawalProposed(id, token, to, amount);
        _recordApproval(id, p, ownerIndex);
    }

    /// Queue a full settings replacement (owners, threshold, daily limit,
    /// delay). The vault never changes its config any other way.
    function proposeSettings(
        Key[] calldata newOwners,
        uint8 newThreshold,
        uint128 newDailyLimit,
        uint64 newDelay,
        uint256 ownerIndex,
        bytes32 r,
        bytes32 s
    ) external returns (uint256 id) {
        Key memory k = _ownerAt(ownerIndex);
        _checkConfig(newOwners, newThreshold, newDelay);
        id = ++proposalCount;
        if (!_verify(settingsDigest(id, newOwners, newThreshold, newDailyLimit, newDelay), r, s, k)) revert BadSignature();
        Proposal storage p = _proposals[id];
        p.kind = Kind.Settings;
        p.epoch = queueEra;
        for (uint256 i = 0; i < newOwners.length; i++) {
            p.owners.push(newOwners[i]);
        }
        p.newThreshold = newThreshold;
        p.newDailyLimit = newDailyLimit;
        p.newDelay = newDelay;
        emit SettingsProposed(id, newOwners, newThreshold, newDailyLimit, newDelay);
        _recordApproval(id, p, ownerIndex);
    }

    /// Add one owner's approval of a pending proposal. Every approver signs
    /// the same digest, so relaying them is order-free.
    function approve(uint256 id, uint256 ownerIndex, bytes32 r, bytes32 s) external {
        Key memory k = _ownerAt(ownerIndex);
        Proposal storage p = _proposal(id);
        bytes32 digest = p.kind == Kind.Withdraw
            ? withdrawDigest(id, p.token, p.to, p.amount)
            : settingsDigest(id, p.owners, p.newThreshold, p.newDailyLimit, p.newDelay);
        if (!_verify(digest, r, s, k)) revert BadSignature();
        _recordApproval(id, p, ownerIndex);
    }

    /// Any ONE owner stops a pending proposal — before the threshold is
    /// reached, or during the delay.
    function cancel(uint256 id, uint256 ownerIndex, bytes32 r, bytes32 s) external {
        Key memory k = _ownerAt(ownerIndex);
        _proposal(id);
        uint256 nonce = cancelNonce;
        if (!_verify(cancelDigest(id, nonce), r, s, k)) revert BadSignature();
        cancelNonce = nonce + 1;
        delete _proposals[id];
        emit Canceled(id, ownerIndex);
    }

    /// Run a proposal whose threshold was met `delay` seconds ago. Anyone may
    /// relay it. State is settled before the transfer, so a re-entering token
    /// contract finds the proposal already gone.
    function execute(uint256 id) external {
        Proposal storage p = _proposal(id);
        if (p.readyAt == 0) revert NotReady();
        if (block.timestamp < p.readyAt) revert NotYet(p.readyAt);
        if (p.kind == Kind.Withdraw) {
            address token = p.token;
            address to = p.to;
            uint256 amount = p.amount;
            delete _proposals[id];
            if (token == address(0)) {
                (bool ok,) = to.call{value: amount}("");
                if (!ok) revert TransferFailed();
            } else {
                // F-03: an event is a receipt, not a transfer. Measure what
                // `to` actually received: a dishonest token can answer true
                // while moving nothing, and only the balance delta proves the
                // payment happened. Fee-on-transfer tokens are refused here —
                // a vault pays exactly what its owners signed.
                uint256 before = _balanceOfAt(token, to);
                (bool ok, bytes memory out) = token.call(abi.encodeWithSelector(bytes4(0xa9059cbb), to, amount)); // transfer(address,uint256)
                if (!ok || (out.length != 0 && !abi.decode(out, (bool)))) revert TokenTransferFailed();
                uint256 received = _balanceOfAt(token, to) - before;
                if (received != amount) revert ShortDelivery(received, amount);
            }
            emit WithdrawalExecuted(id, token, to, amount);
        } else {
            Key[] memory newOwners = p.owners;
            uint8 newThreshold = p.newThreshold;
            uint128 newDailyLimit = p.newDailyLimit;
            uint64 newDelay = p.newDelay;
            delete _proposals[id];
            queueEra += 1; // every other pending proposal dies with the old owner set
            delete owners;
            for (uint256 i = 0; i < newOwners.length; i++) {
                owners.push(newOwners[i]);
            }
            threshold = newThreshold;
            dailyLimit = newDailyLimit;
            delay = newDelay;
            emit SettingsExecuted(id, queueEra);
        }
    }

    // ---- internals ----

    function _ownerAt(uint256 index) private view returns (Key memory) {
        if (index >= owners.length) revert BadSignature();
        return owners[index];
    }

    function _proposal(uint256 id) private view returns (Proposal storage p) {
        p = _proposals[id];
        if (id == 0 || id > proposalCount || p.kind == Kind.None || p.epoch != queueEra) revert UnknownProposal();
    }

    function _recordApproval(uint256 id, Proposal storage p, uint256 ownerIndex) private {
        if (p.approvals & (1 << ownerIndex) != 0) revert AlreadyApproved();
        p.approvals |= 1 << ownerIndex;
        if (p.readyAt == 0 && _count(p.approvals) >= threshold) {
            p.readyAt = uint64(block.timestamp) + delay;
        }
        emit Approved(id, ownerIndex, p.approvals, p.readyAt);
    }

    function _count(uint256 mask) private pure returns (uint256 c) {
        while (mask != 0) {
            c += mask & 1;
            mask >>= 1;
        }
    }

    function _checkConfig(Key[] memory keys, uint8 threshold_, uint64 delay_) private pure {
        if (keys.length == 0 || keys.length > MAX_OWNERS) revert BadConfig();
        if (threshold_ == 0 || threshold_ > keys.length) revert BadConfig();
        if (delay_ < MIN_DELAY) revert BadConfig();
        for (uint256 i = 0; i < keys.length; i++) {
            if (keys[i].x == bytes32(0) && keys[i].y == bytes32(0)) revert BadConfig();
            for (uint256 j = 0; j < i; j++) {
                if (keys[j].x == keys[i].x && keys[j].y == keys[i].y) revert BadConfig();
            }
        }
    }

    function _verify(bytes32 digest, bytes32 r, bytes32 s, Key memory k) private view returns (bool) {
        (bool ok, bytes memory out) = P256VERIFY.staticcall(abi.encodePacked(digest, r, s, k.x, k.y));
        return ok && out.length == 32 && abi.decode(out, (uint256)) == 1;
    }

    /// F-03: read a token balance through a staticcall, so a no-code address
    /// (or a non-ERC-20) fails cleanly instead of reading as zero.
    function _balanceOfAt(address token, address who) private view returns (uint256 balance) {
        (bool ok, bytes memory out) = token.staticcall(abi.encodeWithSelector(bytes4(0x70a08231), who)); // balanceOf(address)
        if (!ok || out.length != 32) revert BadToken();
        balance = abi.decode(out, (uint256));
    }
}

/// Deploys vaults with CREATE2: the address is a function of the owners and
/// the salt (and the other creation parameters), so the app can show the
/// address before deploying, and re-derive it on any machine. No admin, no
/// registry: the factory only ever deploys.
contract EastSeaVaultFactory {
    event VaultCreated(address indexed vault, bytes32 indexed salt, uint8 threshold, uint128 dailyLimit, uint64 delay);

    function create(EastSeaVault.Key[] calldata keys, uint8 threshold, uint128 dailyLimit, uint64 delay, bytes32 salt)
        external
        returns (address)
    {
        EastSeaVault vault = new EastSeaVault{salt: salt}(keys, threshold, dailyLimit, delay);
        emit VaultCreated(address(vault), salt, threshold, dailyLimit, delay);
        return address(vault);
    }

    /// Where `create` with these exact arguments will deploy.
    function predict(EastSeaVault.Key[] calldata keys, uint8 threshold, uint128 dailyLimit, uint64 delay, bytes32 salt)
        external
        view
        returns (address)
    {
        bytes32 initCode = keccak256(abi.encodePacked(type(EastSeaVault).creationCode, abi.encode(keys, threshold, dailyLimit, delay)));
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), address(this), salt, initCode)))));
    }
}
