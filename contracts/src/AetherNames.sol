// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// The `.aeth` name service (docs/design/26-name-service.md, launch item E18):
/// fixed burn fees, no auction. Immutable, no owner, no admin, no upgrade, no
/// pause — the same rules for everyone, forever.
///
/// Registration is commit-reveal: `commit(keccak256(name, owner, salt))`, wait
/// MIN_COMMIT_AGE, then `register(name, owner, salt)` with the fee. The
/// commitment binds the name AND the owner, so a front-runner watching the
/// reveal cannot reuse the victim's salt, and their own fresh commitment is
/// still too young when the victim's already-aged reveal lands.
///
/// The fee is fixed by name length, paid in native AETH, and BURNED (sent to
/// BURN_ADDRESS). Nobody receives anything: no fee recipient, no treasury, no
/// premium or reserved names (12-launch-plan.md 원칙: 비수탁, 수수료 0). The burn
/// address is 0x…dEaD — the ecosystem's keyless convention; address(0) is
/// deliberately NOT used because it is the "unset" sentinel all over this
/// contract. Over-payment is refunded in the same call; state is settled
/// before either transfer (checks-effects-interactions), so re-entrancy from
/// the refund finds the commitment already spent.
///
/// Registration lasts REGISTRATION_PERIOD; anyone may renew (a gift to the
/// owner, priced at the same fee) up to GRACE_PERIOD past expiry, after which
/// the name is released and free to register again — with no stale resolver
/// data (the release is lazy: views go quiet immediately, and the next
/// registration sweeps the old record).
///
/// Owners are addresses, and Aether accounts are smart accounts, so an owner
/// may well be a contract. This contract never calls its owners; ownership
/// grants no callback surface.
contract AetherNames {
    struct Record {
        address owner;
        uint64 expires;
        /// Kept so reverseOf can return the name without another lookup.
        string name;
        address pendingOwner;
        address addr;
        bytes32[] textKeys;
        mapping(bytes32 => string) texts;
    }

    error InvalidName();
    error InvalidOwner();
    error UnknownCommitment();
    error CommitTooNew(uint256 age);
    error CommitTooOld();
    error NameTaken();
    error Unregistered();
    error Released();
    error InsufficientFee(uint256 fee);
    error BurnFailed();
    error RefundFailed();
    error NotOwner();
    error NotPendingOwner();
    error BadTextKey();
    error TextValueTooLong();
    error TooManyTextRecords();
    error ReverseMismatch();

    event CommitmentMade(bytes32 indexed commitment);
    event Registered(string name, bytes32 indexed node, address indexed owner, uint64 expires, uint256 fee);
    event Renewed(bytes32 indexed node, uint64 newExpires, uint256 fee);
    event Burned(uint256 amount);
    event TransferProposed(bytes32 indexed node, address indexed from, address indexed to);
    event TransferAccepted(bytes32 indexed node, address indexed from, address indexed to);
    event AddrSet(bytes32 indexed node, address indexed addr);
    event TextSet(bytes32 indexed node, string key, string value);
    event ReverseSet(address indexed account, bytes32 indexed node, string name);

    /// Keyless, codeless, nobody's. address(0) is reserved for "unset".
    address payable public constant BURN_ADDRESS = payable(0x000000000000000000000000000000000000dEaD);
    /// Short names are scarce, so they cost more to squat. 3 chars: 2, 4: 0.5, 5+: 0.1 AETH.
    uint256 public constant FEE_3 = 2 ether;
    uint256 public constant FEE_4 = 0.5 ether;
    uint256 public constant FEE_5_PLUS = 0.1 ether;
    uint256 public constant MIN_NAME_LENGTH = 3;
    uint256 public constant MAX_NAME_LENGTH = 32;
    uint256 public constant REGISTRATION_PERIOD = 365 days;
    /// After this the name is free again. Views go quiet at expiry+grace.
    uint64 public constant GRACE_PERIOD = 30 days;
    /// Commit must age this long (and no longer than MAX_COMMIT_AGE).
    uint64 public constant MIN_COMMIT_AGE = 60 seconds;
    uint64 public constant MAX_COMMIT_AGE = 24 hours;
    uint256 public constant MAX_TEXT_KEYS = 4;
    uint256 public constant TEXT_KEY_MAX_LENGTH = 32;
    uint256 public constant TEXT_VALUE_MAX_LENGTH = 128;

    uint256 public totalBurned;
    /// commitment hash => commit timestamp. Anyone may commit for anyone.
    mapping(bytes32 => uint256) private _commitments;
    /// account => node claimed as its primary name.
    mapping(address => bytes32) private _reverse;
    mapping(bytes32 => Record) private _records;

    // ---- commit-reveal ----

    /// Post a commitment (permissionless; re-committing refreshes the window).
    function commit(bytes32 commitment) external {
        _commitments[commitment] = block.timestamp;
        emit CommitmentMade(commitment);
    }

    /// Reveal: register `name` for `owner` using the salt from the commitment.
    /// The payer (msg.sender) may be anyone; the fee is burned, any excess is
    /// refunded to the payer in the same call.
    function register(string calldata name, address owner, bytes32 salt) external payable {
        if (!isValidName(name)) revert InvalidName();
        if (owner == address(0)) revert InvalidOwner();
        bytes32 commitment = keccak256(abi.encodePacked(name, owner, salt));
        uint256 committedAt = _commitments[commitment];
        if (committedAt == 0) revert UnknownCommitment();
        uint256 age = block.timestamp - committedAt;
        if (age < MIN_COMMIT_AGE) revert CommitTooNew(age);
        if (age >= MAX_COMMIT_AGE) revert CommitTooOld();
        bytes32 node = nodeFor(name);
        Record storage r = _records[node];
        if (_live(r)) revert NameTaken();
        uint256 fee = feeFor(name);
        if (msg.value < fee) revert InsufficientFee(fee);

        // Effects: everything settles before the transfers, so a re-entrant
        // call from the refund finds its commitment already spent.
        delete _commitments[commitment];
        _sweep(node, r); // a dead record sits here: clear its stale data
        r.owner = owner;
        r.expires = uint64(block.timestamp + REGISTRATION_PERIOD);
        r.name = name;
        totalBurned += fee;
        emit Registered(name, node, owner, r.expires, fee);
        emit Burned(fee);

        // Interactions: burn, then refund.
        (bool burned,) = BURN_ADDRESS.call{value: fee}("");
        if (!burned) revert BurnFailed();
        uint256 refund = msg.value - fee;
        if (refund > 0) {
            (bool ok,) = msg.sender.call{value: refund}("");
            if (!ok) revert RefundFailed();
        }
    }

    /// Extend by one REGISTRATION_PERIOD from the CURRENT expiry (not from
    /// now), by anyone, at the same fixed fee. During grace this means the
    /// lapsed time is the owner's loss.
    function renew(string calldata name) external payable {
        bytes32 node = nodeFor(name);
        Record storage r = _records[node];
        if (r.owner == address(0)) revert Unregistered();
        if (!_live(r)) revert Released();
        uint256 fee = feeFor(name);
        if (msg.value < fee) revert InsufficientFee(fee);

        uint64 newExpires = uint64(uint256(r.expires) + REGISTRATION_PERIOD);
        r.expires = newExpires;
        totalBurned += fee;
        emit Renewed(node, newExpires, fee);
        emit Burned(fee);

        (bool burned,) = BURN_ADDRESS.call{value: fee}("");
        if (!burned) revert BurnFailed();
        uint256 refund = msg.value - fee;
        if (refund > 0) {
            (bool ok,) = msg.sender.call{value: refund}("");
            if (!ok) revert RefundFailed();
        }
    }

    // ---- ownership ----

    /// Two-step transfer: propose (address(0) cancels), then the new owner
    /// accepts. A mistyped destination can be dropped before it does harm.
    function transferPropose(string calldata name, address to) external {
        bytes32 node = nodeFor(name);
        Record storage r = _records[node];
        _requireLiveOwner(r);
        r.pendingOwner = to;
        emit TransferProposed(node, r.owner, to);
    }

    function transferAccept(string calldata name) external {
        bytes32 node = nodeFor(name);
        Record storage r = _records[node];
        address previous = r.owner;
        if (!_live(r) || r.pendingOwner == address(0) || msg.sender != r.pendingOwner) revert NotPendingOwner();
        delete r.pendingOwner;
        r.owner = msg.sender;
        emit TransferAccepted(node, previous, msg.sender);
    }

    // ---- resolver records ----

    /// The name's primary address. Moving it retires any reverse claim the
    /// previous address held on this name.
    function setAddr(string calldata name, address a) external {
        bytes32 node = nodeFor(name);
        Record storage r = _records[node];
        _requireLiveOwner(r);
        address previous = r.addr;
        r.addr = a;
        if (previous != address(0) && previous != a && _reverse[previous] == node) delete _reverse[previous];
        emit AddrSet(node, a);
    }

    /// One bounded set of text records per name: at most MAX_TEXT_KEYS
    /// distinct keys (lowercase [a-z0-9-], 1-32 bytes), values up to
    /// TEXT_VALUE_MAX_LENGTH bytes. An empty value deletes (and frees the slot).
    function setText(string calldata name, string calldata key, string calldata value) external {
        bytes32 node = nodeFor(name);
        Record storage r = _records[node];
        _requireLiveOwner(r);
        if (!_validTextKey(bytes(key))) revert BadTextKey();
        if (bytes(value).length > TEXT_VALUE_MAX_LENGTH) revert TextValueTooLong();
        bytes32 k = keccak256(bytes(key));
        bool known = _hasKey(r, k);
        if (bytes(value).length == 0) {
            delete r.texts[k];
            if (known) _removeKey(r, k);
        } else {
            if (!known) {
                if (r.textKeys.length >= MAX_TEXT_KEYS) revert TooManyTextRecords();
                r.textKeys.push(k);
            }
            r.texts[k] = value;
        }
        emit TextSet(node, key, value);
    }

    /// Claim `name` as msg.sender's primary name. Only the name's owner can
    /// do this, and only when the name's address record IS msg.sender — so
    /// nobody can pin a name on someone else's address.
    function setReverse(string calldata name) external {
        bytes32 node = nodeFor(name);
        Record storage r = _records[node];
        _requireLiveOwner(r);
        if (r.addr != msg.sender) revert ReverseMismatch();
        _reverse[msg.sender] = node;
        emit ReverseSet(msg.sender, node, name);
    }

    // ---- pure ----

    /// Lowercase ASCII [a-z0-9-], 3-32 bytes, no leading/trailing hyphen, no
    /// double hyphen at positions 3-4 (the punycode `xn--` shape — homograph
    /// hygiene).
    function isValidName(string calldata name) public pure returns (bool) {
        bytes memory b = bytes(name);
        if (b.length < MIN_NAME_LENGTH || b.length > MAX_NAME_LENGTH) return false;
        if (b[0] == 0x2d || b[b.length - 1] == 0x2d) return false;
        if (b.length >= 4 && b[2] == 0x2d && b[3] == 0x2d) return false;
        for (uint256 i = 0; i < b.length; i++) {
            bytes1 c = b[i];
            bool ok = (c >= 0x61 && c <= 0x7a) || (c >= 0x30 && c <= 0x39) || c == 0x2d;
            if (!ok) return false;
        }
        return true;
    }

    /// The fixed fee for a name, by length. Invalid names never reach payment
    /// (register reverts first), so this simply buckets by length.
    function feeFor(string calldata name) public pure returns (uint256) {
        bytes memory b = bytes(name);
        if (b.length == 3) return FEE_3;
        if (b.length == 4) return FEE_4;
        return FEE_5_PLUS;
    }

    function nodeFor(string calldata name) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(name));
    }

    // ---- views ----

    /// address(0) once the name is past its grace period (released).
    function ownerOf(bytes32 node) external view returns (address) {
        Record storage r = _records[node];
        return _live(r) ? r.owner : address(0);
    }

    function pendingOwnerOf(bytes32 node) external view returns (address) {
        Record storage r = _records[node];
        return _live(r) ? r.pendingOwner : address(0);
    }

    /// Raw expiry (also readable for released names); grace runs to
    /// expires + GRACE_PERIOD.
    function expiresOf(bytes32 node) external view returns (uint64) {
        return _records[node].expires;
    }

    function addrOf(bytes32 node) external view returns (address) {
        Record storage r = _records[node];
        return _live(r) ? r.addr : address(0);
    }

    function textOf(bytes32 node, string calldata key) external view returns (string memory) {
        Record storage r = _records[node];
        if (!_live(r)) return "";
        return r.texts[keccak256(bytes(key))];
    }

    /// The account's primary name, but only while it is honest: the forward
    /// record must still be live and point back at the account. Stale claims
    /// answer "" (setAddr and the sweep also remove them eagerly).
    function reverseOf(address account) external view returns (string memory) {
        bytes32 node = _reverse[account];
        Record storage r = _records[node];
        if (!_live(r) || r.addr != account) return "";
        return r.name;
    }

    // ---- internals ----

    /// A record counts while owner is set and now < expires + grace.
    function _live(Record storage r) private view returns (bool) {
        return r.owner != address(0) && block.timestamp < uint256(r.expires) + GRACE_PERIOD;
    }

    function _requireLiveOwner(Record storage r) private view {
        if (!_live(r) || r.owner != msg.sender) revert NotOwner();
    }

    /// Clear a dead record so the next registration starts fresh: texts,
    /// pending transfer, address record, and the reverse claim it anchored.
    function _sweep(bytes32 node, Record storage r) private {
        if (r.addr != address(0) && _reverse[r.addr] == node) delete _reverse[r.addr];
        for (uint256 i = 0; i < r.textKeys.length; i++) delete r.texts[r.textKeys[i]];
        delete r.textKeys;
        delete r.owner;
        delete r.expires;
        delete r.pendingOwner;
        delete r.addr;
        // r.name stays; it is overwritten on the next registration.
    }

    function _validTextKey(bytes memory b) private pure returns (bool) {
        if (b.length == 0 || b.length > TEXT_KEY_MAX_LENGTH) return false;
        for (uint256 i = 0; i < b.length; i++) {
            bytes1 c = b[i];
            bool ok = (c >= 0x61 && c <= 0x7a) || (c >= 0x30 && c <= 0x39) || c == 0x2d;
            if (!ok) return false;
        }
        return true;
    }

    function _hasKey(Record storage r, bytes32 k) private view returns (bool) {
        for (uint256 i = 0; i < r.textKeys.length; i++) {
            if (r.textKeys[i] == k) return true;
        }
        return false;
    }

    function _removeKey(Record storage r, bytes32 k) private {
        for (uint256 i = 0; i < r.textKeys.length; i++) {
            if (r.textKeys[i] == k) {
                r.textKeys[i] = r.textKeys[r.textKeys.length - 1];
                r.textKeys.pop();
                return;
            }
        }
    }
}
