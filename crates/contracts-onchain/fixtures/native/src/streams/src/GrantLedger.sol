// SPDX-License-Identifier: MIT
pragma solidity ^0.8.31;

import {NativeBrake} from "../../common/src/NativeBrake.sol";
import {TransientLock} from "../../common/src/TransientLock.sol";
import {ExactToken, IERC20Min} from "../../common/src/ExactToken.sol";

/// @title GrantLedger: already-funded, non-revocable vesting grants
/// @notice One shared immutable instance per exact-transfer ERC-20. Anyone
///         funds a grant for a fixed beneficiary with a cliff-to-end linear
///         schedule. Accrual is lazy: nothing happens per second; the
///         beneficiary (or anyone paying the fee) calls `claim` whenever they
///         want the earned part. There is no cancellation, clawback, schedule
///         edit, admin, fee or upgrade. The final payout deletes the grant.
/// @dev Design: native/streams/DESIGN.md. Two words per live grant:
///        S0 = beneficiary(160) | start(32) | cliff(32) | end(32)
///        S1 = total(128) | released(128)
///      Earned(t) = 0                                  if t <= cliff
///                = total                              if t >= end
///                = floor(total * (t - cliff) / (end - cliff)) otherwise
///      total < 2^128 and (t - cliff) < 2^32, so the product is < 2^160 and
///      cannot overflow uint256; t is the full block.timestamp, never
///      truncated to 32 bits. `start` is recorded for display only: this
///      schedule has no catch-up at the cliff (see README).
contract GrantLedger is NativeBrake, TransientLock {
    uint256 public constant MAX_BATCH = 8;

    address public immutable token;
    bytes32 public immutable tokenCodeHash;

    struct Meta {
        uint64 nextId;
        uint64 brakeSince;
        uint128 outstanding;
    }

    struct Grant {
        address beneficiary; // S0
        uint32 start; // S0
        uint32 cliff; // S0
        uint32 end; // S0
        uint128 total; // S1
        uint128 released; // S1
    }

    struct GrantParams {
        address beneficiary;
        uint128 total;
        uint32 start;
        uint32 cliff;
        uint32 end;
    }

    Meta private _meta;
    mapping(uint256 => Grant) private _grants;

    event Created(
        uint256 indexed id,
        address indexed creator,
        address indexed beneficiary,
        uint256 total,
        uint32 start,
        uint32 cliff,
        uint32 end
    );
    event Claimed(uint256 indexed id, address indexed beneficiary, uint256 amount);

    error TokenHasNoCode();
    error BadBeneficiary();
    error ZeroAmount();
    error BadSchedule(uint32 start, uint32 cliff, uint32 end);
    error BadBatchSize(uint256 n);
    error OutstandingOverflow();
    error IdsExhausted();
    error UnknownGrant(uint256 id);
    error NothingToClaim(uint256 id);
    error BackingShortfall(uint256 balance, uint256 outstanding);

    constructor(address token_, bytes32 brakeDocSha256) NativeBrake(brakeDocSha256) {
        if (token_.code.length == 0) revert TokenHasNoCode();
        token = token_;
        tokenCodeHash = token_.codehash;
        _meta.nextId = 1;
    }

    // ------------------------------------------------------------------ entry

    /// @notice Fund one grant exactly. The caller keeps no authority over it.
    function create(GrantParams calldata p) external lock returns (uint256 id) {
        _requireEntryOpen();
        Meta memory m = _meta;
        id = m.nextId; // < 2^64-1: the brake predicate closes entry at the max
        _store(id, p);
        uint256 owed = uint256(m.outstanding) + p.total;
        if (owed > type(uint128).max) revert OutstandingOverflow();
        // Casts are bounded: id < 2^64-1 (entry brake) and owed checked above.
        // forge-lint: disable-next-line(unsafe-typecast)
        _meta = Meta({nextId: uint64(id + 1), brakeSince: m.brakeSince, outstanding: uint128(owed)});
        ExactToken.pullExact(token, msg.sender, p.total);
    }

    /// @notice Fund up to eight grants with one exact pull of their sum.
    function createBatch(GrantParams[] calldata ps) external lock returns (uint256 firstId) {
        _requireEntryOpen();
        uint256 n = ps.length;
        if (n == 0 || n > MAX_BATCH) revert BadBatchSize(n);
        Meta memory m = _meta;
        firstId = m.nextId;
        if (firstId + n > type(uint64).max) revert IdsExhausted();
        uint256 sum;
        for (uint256 i; i < n; i++) {
            _store(firstId + i, ps[i]);
            sum += ps[i].total;
        }
        uint256 owed = uint256(m.outstanding) + sum;
        if (owed > type(uint128).max) revert OutstandingOverflow();
        // Casts are bounded by the two checks above.
        // forge-lint: disable-next-line(unsafe-typecast)
        _meta = Meta({nextId: uint64(firstId + n), brakeSince: m.brakeSince, outstanding: uint128(owed)});
        ExactToken.pullExact(token, msg.sender, sum);
    }

    // ------------------------------------------------------------------- exit

    /// @notice Pay the earned, unreleased amount to the fixed beneficiary.
    ///         Anyone may call (and pay the fee); the money only goes to the
    ///         beneficiary. Reverts when nothing is payable, so a successful
    ///         call always moved money.
    function claim(uint256 id) external lock returns (uint256 amount) {
        Grant storage g = _grants[id];
        address to = g.beneficiary;
        if (to == address(0)) revert UnknownGrant(id);
        uint128 total = g.total;
        uint128 released = g.released;
        amount = _earned(total, g.cliff, g.end, block.timestamp) - released;
        if (amount == 0) revert NothingToClaim(id);

        if (released + amount == total) {
            delete _grants[id]; // final payout retires both words
        } else {
            // released + amount < total < 2^128
            // forge-lint: disable-next-line(unsafe-typecast)
            g.released = uint128(released + amount);
        }
        // amount <= total < 2^128
        // forge-lint: disable-next-line(unsafe-typecast)
        uint128 owed = _meta.outstanding - uint128(amount);
        _meta.outstanding = owed;

        ExactToken.pushExact(token, to, amount);
        _requireBacked(owed);
        emit Claimed(id, to, amount);
    }

    // ------------------------------------------------------------------ views

    function nextId() external view returns (uint256) {
        return _meta.nextId;
    }

    function outstanding() external view returns (uint256) {
        return _meta.outstanding;
    }

    function grant(uint256 id) external view returns (Grant memory) {
        return _grants[id];
    }

    /// @notice Earned amount at time `t` (0 for unknown or completed grants).
    function earnedAt(uint256 id, uint256 t) public view returns (uint256) {
        Grant storage g = _grants[id];
        if (g.beneficiary == address(0)) return 0;
        return _earned(g.total, g.cliff, g.end, t);
    }

    /// @notice What `claim` would pay now ("Available now" in the wallet).
    function claimable(uint256 id) external view returns (uint256) {
        Grant storage g = _grants[id];
        if (g.beneficiary == address(0)) return 0;
        return _earned(g.total, g.cliff, g.end, block.timestamp) - g.released;
    }

    // --------------------------------------------------------------- internal

    function _store(uint256 id, GrantParams calldata p) private {
        if (p.beneficiary == address(0) || p.beneficiary == address(this)) revert BadBeneficiary();
        if (p.total == 0) revert ZeroAmount();
        if (!(p.start <= p.cliff && p.cliff < p.end)) revert BadSchedule(p.start, p.cliff, p.end);
        _grants[id] = Grant({
            beneficiary: p.beneficiary, start: p.start, cliff: p.cliff, end: p.end, total: p.total, released: 0
        });
        emit Created(id, msg.sender, p.beneficiary, p.total, p.start, p.cliff, p.end);
    }

    function _earned(uint128 total, uint32 cliff, uint32 end, uint256 t) private pure returns (uint256) {
        if (t >= end) return total;
        if (t <= cliff) return 0;
        // total < 2^128, t - cliff < end - cliff < 2^32: no overflow.
        return uint256(total) * (t - cliff) / (end - cliff);
    }

    /// @dev A payout never consumes another grant's backing.
    function _requireBacked(uint256 owed) private view {
        uint256 bal = IERC20Min(token).balanceOf(address(this));
        if (bal < owed) revert BackingShortfall(bal, owed);
    }

    function _brakeReasons() internal view override returns (uint8 r) {
        if (token.codehash != tokenCodeHash) r |= REASON_CODE_CHANGED;
        if (IERC20Min(token).balanceOf(address(this)) < _meta.outstanding) r |= REASON_DEFICIT;
        if (_meta.nextId == type(uint64).max) r |= REASON_IDS_EXHAUSTED;
    }

    function _brakeSince() internal view override returns (uint64) {
        return _meta.brakeSince;
    }

    function _setBrakeSince(uint64 h) internal override {
        _meta.brakeSince = h;
    }

    function _brakeUri() internal pure override returns (string memory) {
        return "native/streams/SECURITY.md#brake";
    }
}
