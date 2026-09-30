// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// Candidates for the validator committee (docs/design/07-consensus.md, "open
/// committee"). A Mac registered with Apple DeviceCheck by the registrar (one
/// device = one candidate) joins with its validator key; it proves it is alive
/// every epoch with a beacon, which also builds its contribution streak.
/// Nodes select each epoch's committee from this state, deterministically.
///
/// The operator of a candidate is the account that registered it (the owner's
/// wallet, Touch ID); selection caps every operator below 1/3 of the seats. The
/// node itself sends the beacons from its own account (`beaconer`), unattended.
contract CommitteeRegistry {
    struct Candidate {
        address operator;
        /// Ed25519 consensus key.
        bytes32 validatorKey;
        /// iroh node id (Ed25519) the validator is reached at.
        bytes32 nodeId;
        /// The node's own account, which sends the beacons automatically.
        address beaconer;
        uint64 registeredEpoch;
        /// Last epoch a beacon arrived in, and consecutive epochs so far.
        uint64 lastEpoch;
        uint64 streak;
        /// Epochs missed within the grace period since the streak began: nodes
        /// draw only Macs up at least 95% of the time (missed * 20 <= streak).
        uint64 missed;
    }

    error NotRegistrar();
    error Known();
    error Unknown();
    error BadAttestation();
    error TooManyThisEpoch();

    event Registered(uint256 index, address operator, bytes32 validatorKey, bytes32 nodeId);
    event Beacon(uint256 index, uint64 epoch, uint64 streak);

    address constant P256VERIFY = address(0x100);
    /// Epochs a candidate may miss before its streak restarts (24 h).
    uint64 public constant GRACE_EPOCHS = 24;

    /// Registrar's P-256 key (x, y): its signature attests a DeviceCheck registration.
    /// Fixed at genesis (storage slots 0 and 1); changed only by a signed upgrade.
    bytes32 public registrarX;
    bytes32 public registrarY;

    Candidate[] public candidates;
    /// validatorKey => index + 1. A node-rewards genesis with founder reserve
    /// keys prewrites type(uint256).max sentinels here for each reserve key
    /// (rewards::set_reserve, slot keccak256(key . bytes32(3))), so this
    /// contract's own `register()` reverts Known() on a direct registration of
    /// one — the deployed bytecode never changes.
    mapping(bytes32 => uint256) public indexOf;
    /// Blocks per epoch, fixed at genesis (slot 4; one hour of 1 s blocks on the testnet).
    uint64 public epochBlocks;
    /// Epochs of unbroken liveness before a Mac can be drawn into the voting set (slot 5).
    uint256 public minStreak;
    /// Epochs between voting-set draws (slot 6). Nodes draw the set from the
    /// Macs eligible at a draw, in an order fixed by the committee's threshold
    /// signature on the draw number (docs/research/voting-set-security-2026.md).
    uint256 public drawEpochs;
    /// Protocol 2: at most this many new candidates per epoch (slot 7; 0 = no limit),
    /// so even a stolen registrar key cannot flood the pool.
    uint256 public maxPerEpoch;
    /// The epoch and count of registrations so far (slots 8 and 9).
    uint256 public regEpoch;
    uint256 public regCount;

    function epoch() public view returns (uint64) {
        return uint64(block.number / epochBlocks);
    }

    function count() external view returns (uint256) {
        return candidates.length;
    }

    /// What the registrar signs (SHA-256 of it, P-256) after DeviceCheck accepted the device.
    function attestationDigest(address operator, bytes32 validatorKey, bytes32 nodeId, address beaconer) public view returns (bytes32) {
        return sha256(abi.encode(block.chainid, address(this), operator, validatorKey, nodeId, beaconer));
    }

    /// Join as a candidate; the caller becomes its operator.
    function register(bytes32 validatorKey, bytes32 nodeId, address beaconer, bytes32 r, bytes32 s) external {
        if (indexOf[validatorKey] != 0) revert Known();
        bytes32 digest = attestationDigest(msg.sender, validatorKey, nodeId, beaconer);
        (bool ok, bytes memory out) = P256VERIFY.staticcall(abi.encodePacked(digest, r, s, registrarX, registrarY));
        if (!ok || out.length != 32 || abi.decode(out, (uint256)) != 1) revert BadAttestation();
        uint64 e = epoch();
        if (maxPerEpoch != 0) {
            if (regEpoch != e) {
                regEpoch = e;
                regCount = 0;
            }
            if (regCount >= maxPerEpoch) revert TooManyThisEpoch();
            regCount += 1;
        }
        candidates.push(Candidate(msg.sender, validatorKey, nodeId, beaconer, e, e, 1, 0));
        indexOf[validatorKey] = candidates.length;
        emit Registered(candidates.length - 1, msg.sender, validatorKey, nodeId);
    }

    /// Liveness for this epoch, sent by the node's own account (one counts per epoch).
    function beacon(bytes32 validatorKey) external virtual {
        uint256 i = indexOf[validatorKey];
        if (i == 0) revert Unknown();
        Candidate storage c = candidates[i - 1];
        if (c.beaconer != msg.sender) revert Unknown();
        uint64 e = epoch();
        if (e == c.lastEpoch) return;
        uint64 gap = e - c.lastEpoch;
        if (gap <= GRACE_EPOCHS) {
            c.streak += 1;
            c.missed += gap - 1;
        } else {
            c.streak = 1;
            c.missed = 0;
        }
        c.lastEpoch = e;
        emit Beacon(i - 1, e, c.streak);
    }
}
