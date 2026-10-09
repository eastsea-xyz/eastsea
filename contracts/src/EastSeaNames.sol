// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// The `.sea` name service (docs/design/26-name-service.md, launch item E18):
/// fixed burn fees, no auction. Immutable, no owner, no admin, no upgrade, no
/// pause — the same rules for everyone, forever.
///
/// Registration is commit-reveal: `commit(keccak256(name, owner, salt,
/// relayer))` with the burned COMMIT_BOND, wait MIN_COMMIT_AGE, then
/// `register(name, owner, salt, relayer)` with the fee. The commitment binds
/// the name AND the owner AND the relayer, and its slot is bound to the
/// committer who posted the bond: an unexpired commitment is frozen — not
/// even its own committer can rewrite its timestamp — so a front-runner
/// cannot reset a victim's window by re-committing the same hash (A5-3), and
/// only the committer or the relayer they designated may reveal.
///
/// The fee is fixed by name length, paid in native DBLN, and BURNED (sent to
/// BURN_ADDRESS). Nobody receives anything: no fee recipient, no treasury, no
/// premium names. URI action hosts are reserved to prevent routing ambiguity.
/// The burn address is 0x…dEaD — the ecosystem's keyless convention; address(0) is
/// deliberately NOT used because it is the "unset" sentinel all over this
/// contract. Over-payment is refunded in the same call; state is settled
/// before either transfer (checks-effects-interactions), so re-entrancy from
/// the refund finds the commitment already spent.
///
/// COMMIT_BOND is burned when the commitment is posted, never refunded, and
/// credited toward the registration fee when the same committer reveals
/// within the window — an honest self-reveal pays exactly the fee overall.
/// It prevents free blind commitment writes (A5-1); the chain-level state
/// fee is the real defence against arbitrary storage-writing contracts.
/// An expired, unrevealed commitment
/// can be reclaimed by anyone via `clear`.
///
/// Registration lasts REGISTRATION_PERIOD; anyone may renew (a gift to the
/// owner, priced at the same fee) up to GRACE_PERIOD past expiry, after which
/// the name is released and free to register again — with no stale resolver
/// data (the release is lazy: views go quiet immediately, and the next
/// registration sweeps the old record).
///
/// Root owners create and delete free subdomain records. A child records its
/// parent's generation, and resolution validates every ancestor back to the
/// root. Root expiry (without grace), transfer, re-registration, or renewal
/// after expiry invalidates existing children. Ancestor deletion/recreation
/// never revives descendants; they must be recreated explicitly.
///
/// Owners are addresses, and EastSea accounts are smart accounts, so an owner
/// may well be a contract. This contract never calls its owners; ownership
/// grants no callback surface.
contract EastSeaNames {
    struct Record {
        address owner;
        uint64 expires;
        /// Kept so reverseOf can return the name without another lookup.
        string name;
        address pendingOwner;
        address addr;
        bytes32[] textKeys;
        mapping(bytes32 => string) texts;
        /// Zero for registrable roots. Children reference a strictly shorter
        /// hostname and the parent's generation when they were created.
        bytes32 parent;
        uint64 generation;
        uint64 parentGeneration;
    }

    error InvalidName();
    error InvalidOwner();
    error UnknownCommitment();
    error CommitTooNew(uint256 age);
    error CommitTooOld();
    error CommitmentActive();
    error CommitmentNotExpired();
    error NotCommitter();
    error WrongBondValue();
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

    event CommitmentMade(bytes32 indexed commitment, address indexed committer);
    event CommitmentCleared(bytes32 indexed commitment, address indexed by);
    event Registered(string name, bytes32 indexed node, address indexed owner, uint64 expires, uint256 fee);
    event Renewed(bytes32 indexed node, uint64 newExpires, uint256 fee);
    event Burned(uint256 amount);
    event TransferProposed(bytes32 indexed node, address indexed from, address indexed to);
    event TransferAccepted(bytes32 indexed node, address indexed from, address indexed to);
    event AddrSet(bytes32 indexed node, address indexed addr);
    event TextSet(bytes32 indexed node, string key, string value);
    event ReverseSet(address indexed account, bytes32 indexed node, string name);
    event SubdomainCreated(string name, bytes32 indexed node, bytes32 indexed parent, address indexed owner);
    event SubdomainDeleted(bytes32 indexed node, bytes32 indexed parent);

    /// Keyless, codeless, nobody's. address(0) is reserved for "unset".
    address payable public constant BURN_ADDRESS = payable(0x000000000000000000000000000000000000dEaD);
    /// Short names are scarce, so they cost more to squat. 3 chars: 2, 4: 0.5, 5+: 0.1 DBLN.
    uint256 public constant FEE_3 = 2 ether;
    uint256 public constant FEE_4 = 0.5 ether;
    uint256 public constant FEE_5_PLUS = 0.1 ether;
    uint256 public constant MIN_NAME_LENGTH = 3;
    uint256 public constant MAX_NAME_LENGTH = 32;
    uint256 public constant MAX_LABEL_LENGTH = 63;
    uint256 public constant MAX_HOSTNAME_LENGTH = 253;
    uint256 public constant REGISTRATION_PERIOD = 365 days;
    /// After this the root is free again. Root views stop at expiry+grace;
    /// children stop at expiry.
    uint64 public constant GRACE_PERIOD = 30 days;
    /// Commit must age this long (and no longer than MAX_COMMIT_AGE).
    uint64 public constant MIN_COMMIT_AGE = 60 seconds;
    uint64 public constant MAX_COMMIT_AGE = 24 hours;
    uint256 public constant MAX_TEXT_KEYS = 4;
    uint256 public constant TEXT_KEY_MAX_LENGTH = 32;
    uint256 public constant TEXT_VALUE_MAX_LENGTH = 128;
    /// Burned when a commitment is posted: never refunded, credited toward
    /// the fee on the committer's own reveal. One tenth of the lowest
    /// registration fee (FEE_5_PLUS), so an honest self-reveal pays exactly
    /// the fee overall — see docs/design/26-name-service.md (A5-1).
    uint256 public constant COMMIT_BOND = 0.01 ether;

    uint256 public totalBurned;
    /// commitment hash => (committer, timestamp). The slot is bound to the
    /// committer who paid its bond; frozen while unexpired (A5-3).
    mapping(bytes32 => Commitment) private _commitments;
    /// account => node claimed as its primary name.
    mapping(address => bytes32) private _reverse;
    mapping(bytes32 => Record) private _records;

    struct Commitment {
        /// Bond payer; only they (or the relayer in the hash) may reveal.
        address committer;
        uint64 committedAt;
    }

    // ---- commit-reveal ----

    /// Post a commitment hash and burn the bond. The hash must be
    /// `keccak256(name, owner, salt, relayer)`: the relayer is the one other
    /// account allowed to reveal (address(0) = nobody but the committer);
    /// `owner` stays inside the hash, so registering FOR someone else works.
    ///
    /// An existing unexpired commitment is frozen: re-committing the same
    /// hash reverts (CommitmentActive) no matter who asks — not even the
    /// original committer can move its timestamp (A5-3). Only a slot past
    /// MAX_COMMIT_AGE — dead for revealing anyway — may be replaced, and the
    /// replacement becomes a fresh commitment of its poster.
    function commit(bytes32 commitment) external payable {
        Commitment storage c = _commitments[commitment];
        if (c.committer != address(0) && block.timestamp - c.committedAt < MAX_COMMIT_AGE) {
            revert CommitmentActive();
        }
        if (msg.value != COMMIT_BOND) revert WrongBondValue();
        c.committer = msg.sender;
        c.committedAt = uint64(block.timestamp);
        totalBurned += COMMIT_BOND;
        emit CommitmentMade(commitment, msg.sender);
        emit Burned(COMMIT_BOND);
        (bool burned,) = BURN_ADDRESS.call{value: COMMIT_BOND}("");
        if (!burned) revert BurnFailed();
    }

    /// Reveal: register `name` for `owner` using the salt and relayer from
    /// the commitment. Only the committer or the designated relayer may
    /// reveal (the owner can still be anyone — it is inside the hash). The
    /// committer's burned bond is credited toward the fee, so a self-reveal
    /// pays fee - COMMIT_BOND here and exactly the fee overall; a relayer
    /// reveal pays the full fee. Any excess is refunded in the same call.
    function register(string calldata name, address owner, bytes32 salt, address relayer) external payable {
        if (!isValidName(name)) revert InvalidName();
        if (owner == address(0)) revert InvalidOwner();
        bytes32 commitment = keccak256(abi.encodePacked(name, owner, salt, relayer));
        Commitment storage c = _commitments[commitment];
        if (c.committer == address(0)) revert UnknownCommitment();
        if (msg.sender != c.committer && msg.sender != relayer) revert NotCommitter();
        uint256 age = block.timestamp - c.committedAt;
        if (age < MIN_COMMIT_AGE) revert CommitTooNew(age);
        if (age >= MAX_COMMIT_AGE) revert CommitTooOld();
        bytes32 node = nodeFor(name);
        Record storage r = _records[node];
        if (_live(r)) revert NameTaken();
        uint256 fee = feeFor(name); // fee >= FEE_5_PLUS > COMMIT_BOND
        uint256 due = msg.sender == c.committer ? fee - COMMIT_BOND : fee;
        if (msg.value < due) revert InsufficientFee(due);

        // Effects: everything settles before the transfers, so a re-entrant
        // call from the refund finds its commitment already spent.
        delete _commitments[commitment];
        _sweep(node, r); // a dead record sits here: clear its stale data
        r.owner = owner;
        r.expires = uint64(block.timestamp + REGISTRATION_PERIOD);
        r.name = name;
        totalBurned += due;
        emit Registered(name, node, owner, r.expires, fee);
        emit Burned(due);

        // Interactions: burn, then refund.
        (bool burned,) = BURN_ADDRESS.call{value: due}("");
        if (!burned) revert BurnFailed();
        uint256 refund = msg.value - due;
        if (refund > 0) {
            (bool ok,) = msg.sender.call{value: refund}("");
            if (!ok) revert RefundFailed();
        }
    }

    /// Free the storage of a commitment that expired unrevealed (A5-1).
    /// Anyone may call once the reveal window is over; the slot is deleted
    /// and nothing is paid out — the bond was already burned, and paying a
    /// clearer's bounty would need a held (refundable) bond, i.e. a drain
    /// surface. After a clear the hash is free for a fresh commitment.
    function clear(bytes32 commitment) external {
        Commitment storage c = _commitments[commitment];
        if (c.committer == address(0)) revert UnknownCommitment();
        if (block.timestamp - c.committedAt < MAX_COMMIT_AGE) revert CommitmentNotExpired();
        delete _commitments[commitment];
        emit CommitmentCleared(commitment, msg.sender);
    }

    /// Extend by one REGISTRATION_PERIOD from the CURRENT expiry (not from
    /// now), by anyone, at the same fixed fee. During grace this means the
    /// lapsed time is the owner's loss.
    function renew(string calldata name) external payable {
        bytes32 node = _registrationNode(name);
        Record storage r = _records[node];
        if (r.owner == address(0)) revert Unregistered();
        if (!_live(r)) revert Released();
        uint256 fee = feeFor(name);
        if (msg.value < fee) revert InsufficientFee(fee);

        uint64 newExpires = uint64(uint256(r.expires) + REGISTRATION_PERIOD);
        // An uninterrupted renewal preserves children. Once the paid year
        // lapsed, its children expired and must never be revived by grace.
        if (block.timestamp >= r.expires) r.generation += 1;
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
        bytes32 node = _registrationNode(name);
        Record storage r = _records[node];
        _requireLiveOwner(r);
        r.pendingOwner = to;
        emit TransferProposed(node, r.owner, to);
    }

    function transferAccept(string calldata name) external {
        bytes32 node = _registrationNode(name);
        Record storage r = _records[node];
        address previous = r.owner;
        if (!_live(r) || r.pendingOwner == address(0) || msg.sender != r.pendingOwner) revert NotPendingOwner();
        delete r.pendingOwner;
        r.owner = msg.sender;
        r.generation += 1; // new ownership never inherits stale child records
        emit TransferAccepted(node, previous, msg.sender);
    }

    // ---- subdomains ----

    /// Create a full `.sea` child hostname for free. All ancestors must
    /// exist, and only the active root owner may manage children. The
    /// address record is a recipient, not a delegated child owner.
    function createSubdomain(string calldata name, address a) external {
        (bytes32 node, bytes32 parent, bytes32 root) = _subdomainNodes(name);
        Record storage rootRecord = _records[root];
        _requireActiveRootOwner(rootRecord);
        Record storage parentRecord = _records[parent];
        if (!_live(parentRecord)) revert Unregistered();
        Record storage r = _records[node];
        if (_live(r)) revert NameTaken();
        _sweep(node, r);
        r.owner = rootRecord.owner;
        r.name = name;
        r.expires = rootRecord.expires;
        r.parent = parent;
        r.parentGeneration = parentRecord.generation;
        r.addr = a;
        emit SubdomainCreated(name, node, parent, r.owner);
        emit AddrSet(node, a);
    }

    /// Removing an ancestor increments its generation. Descendants become
    /// inert immediately without enumerating the subtree, even if the same
    /// hostname is created again later.
    function deleteSubdomain(string calldata name) external {
        (bytes32 node, bytes32 parent, bytes32 root) = _subdomainNodes(name);
        _requireActiveRootOwner(_records[root]);
        Record storage r = _records[node];
        if (!_live(r)) revert Unregistered();
        _sweep(node, r);
        emit SubdomainDeleted(node, parent);
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

    /// Registrability: a bare label or `label.sea`, 3-32 lowercase LDH
    /// bytes, excluding URI action hosts. IDN decoding is not implemented:
    /// `xn--...` is an ordinary ASCII label, subject to the same rules.
    function isValidName(string memory name) public pure returns (bool) {
        bytes memory b = bytes(name);
        uint256 end = _hasSeaSuffix(b) ? b.length - 4 : b.length;
        if (end < MIN_NAME_LENGTH || end > MAX_NAME_LENGTH || !_validLabel(b, 0, end)) return false;
        return !_reserved(keccak256(_slice(b, 0, end)));
    }

    /// Hostname syntax is broader than registrability: each lowercase LDH
    /// label is 1-63 bytes, the complete name is at most 253 bytes, and the
    /// TLD must be `.sea`. A trailing dot and external DNS TLDs are invalid.
    function isValidHostname(string memory name) public pure returns (bool) {
        bytes memory b = bytes(name);
        if (b.length > MAX_HOSTNAME_LENGTH || !_hasSeaSuffix(b)) return false;
        uint256 end = b.length - 4;
        uint256 start;
        uint256 rootStart;
        for (uint256 i; i <= end; i++) {
            if (i != end && b[i] != 0x2e) continue;
            if (!_validLabel(b, start, i)) return false;
            rootStart = start;
            start = i + 1;
        }
        return !_reserved(keccak256(_slice(b, rootStart, end)));
    }

    /// Fixed fee by the registrable label's length, excluding `.sea`.
    /// Subdomains cannot be registered or renewed and have no name fee.
    function feeFor(string memory name) public pure returns (uint256) {
        if (!isValidName(name)) revert InvalidName();
        bytes memory b = bytes(name);
        uint256 length = _hasSeaSuffix(b) ? b.length - 4 : b.length;
        if (length == 3) return FEE_3;
        if (length == 4) return FEE_4;
        return FEE_5_PLUS;
    }

    /// Preserve the legacy root hash: `harbor` and `harbor.sea` both hash
    /// `harbor`. Children hash their complete canonical hostname. `.aeth`
    /// remains a legacy-chain client alias, never a new contract input.
    function nodeFor(string memory name) public pure returns (bytes32) {
        bytes memory b = bytes(name);
        if (!_hasSeaSuffix(b)) {
            if (!_validLabel(b, 0, b.length) || _reserved(keccak256(b))) revert InvalidName();
            return keccak256(b);
        }
        if (!isValidHostname(name)) revert InvalidName();
        uint256 end = b.length - 4;
        if (_rootLabelStart(b, end) == 0) return keccak256(_slice(b, 0, end));
        return keccak256(b);
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

    /// Roots expose raw expiry, including after release. A live child
    /// inherits the root's current expiry; an invalid child returns zero.
    function expiresOf(bytes32 node) external view returns (uint64) {
        Record storage r = _records[node];
        return r.parent == bytes32(0) ? r.expires : _subdomainExpiry(r);
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

    /// Roots retain their old grace behavior. Children require uninterrupted
    /// ancestry and expire exactly with the root, without grace.
    function _live(Record storage r) private view returns (bool) {
        if (r.owner == address(0)) return false;
        if (r.parent == bytes32(0)) return block.timestamp < uint256(r.expires) + GRACE_PERIOD;
        return _subdomainExpiry(r) != 0;
    }

    function _subdomainExpiry(Record storage r) private view returns (uint64) {
        bytes32 parent = r.parent;
        uint64 expectedGeneration = r.parentGeneration;
        while (parent != bytes32(0)) {
            Record storage p = _records[parent];
            if (p.owner == address(0) || p.generation != expectedGeneration) return 0;
            if (p.parent == bytes32(0)) return block.timestamp < p.expires ? p.expires : 0;
            expectedGeneration = p.parentGeneration;
            parent = p.parent;
        }
        return 0;
    }

    function _requireLiveOwner(Record storage r) private view {
        if (!_live(r) || r.owner != msg.sender) revert NotOwner();
    }

    function _requireActiveRootOwner(Record storage r) private view {
        if (r.owner != msg.sender || block.timestamp >= r.expires) revert NotOwner();
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
        delete r.parent;
        delete r.parentGeneration;
        r.generation += 1; // never reset: otherwise old descendants revive
        // r.name stays; it is overwritten on the next registration.
    }

    function _registrationNode(string memory name) private pure returns (bytes32) {
        if (!isValidName(name)) revert InvalidName();
        return nodeFor(name);
    }

    function _subdomainNodes(string memory name) private pure returns (bytes32 node, bytes32 parent, bytes32 root) {
        if (!isValidHostname(name)) revert InvalidName();
        bytes memory b = bytes(name);
        uint256 end = b.length - 4;
        uint256 firstDot;
        while (b[firstDot] != 0x2e) firstDot++;
        if (firstDot == end) revert InvalidName(); // registrable root, not a child
        node = keccak256(b);
        parent = nodeFor(string(_slice(b, firstDot + 1, b.length)));
        root = keccak256(_slice(b, _rootLabelStart(b, end), end));
    }

    function _hasSeaSuffix(bytes memory b) private pure returns (bool) {
        return b.length > 4 && b[b.length - 4] == 0x2e && b[b.length - 3] == 0x73
            && b[b.length - 2] == 0x65 && b[b.length - 1] == 0x61;
    }

    function _validLabel(bytes memory b, uint256 start, uint256 end) private pure returns (bool) {
        if (end <= start || end - start > MAX_LABEL_LENGTH) return false;
        if (b[start] == 0x2d || b[end - 1] == 0x2d) return false;
        for (uint256 i = start; i < end; i++) {
            bytes1 c = b[i];
            if (!((c >= 0x61 && c <= 0x7a) || (c >= 0x30 && c <= 0x39) || c == 0x2d)) return false;
        }
        return true;
    }

    function _rootLabelStart(bytes memory b, uint256 end) private pure returns (uint256 start) {
        start = end;
        while (start > 0 && b[start - 1] != 0x2e) start--;
    }

    function _slice(bytes memory b, uint256 start, uint256 end) private pure returns (bytes memory result) {
        result = new bytes(end - start);
        for (uint256 i; i < result.length; i++) result[i] = b[start + i];
    }

    function _reserved(bytes32 label) private pure returns (bool) {
        return label == keccak256("pay") || label == keccak256("call") || label == keccak256("connect")
            || label == keccak256("tx") || label == keccak256("app") || label == keccak256("follow")
            || label == keccak256("name") || label == keccak256("wallet") || label == keccak256("settings")
            || label == keccak256("send") || label == keccak256("receive") || label == keccak256("sign")
            || label == keccak256("deploy") || label == keccak256("open");
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
