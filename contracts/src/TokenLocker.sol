// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// Token lock and vesting (docs/design/17-token-tools.md): immutable, admin-less,
/// fee-free escrows in the spirit of Jupiter Lock — tokens sit in the contract
/// until the rule the depositor wrote at deposit says otherwise, and nobody,
/// not even the depositor, can bend that rule afterwards. Team and LP tokens
/// locked here are the "creator's tokens locked until …" trust signal the
/// launchpad page and the wallet show ([12-launch-plan.md](../../docs/design/12-launch-plan.md),
/// launchpad row: fair-launch guards).
///
/// TokenLocker — deposit ERC-20 (or LP) tokens until a timestamp; only the
/// beneficiary withdraws, after the unlock time. A lock's time can be pushed
/// later (`extend`, by creator or beneficiary) but never shortened.
///
/// TokenVesting — a linear stream with an optional cliff; the beneficiary
/// claims what has vested at any time. A stream is cancelable only if its
/// creator chose so at creation (default not); cancelling pays the vested part
/// to the beneficiary and returns the rest to the creator.
///
/// Both measure balance deltas rather than requested amounts, so fee-on-transfer
/// tokens lock and vest what actually arrived, and both settle their state
/// before moving tokens, so a re-entering token contract finds nothing left.

/// Escrow plumbing shared by the two contracts: pull tokens measuring what
/// actually arrived, and pay out tolerating tokens that return no bool.
abstract contract TokenEscrow {
    error TokenTransferFailed();
    /// The transfer "succeeded" but nothing arrived (a zero deposit).
    error NothingArrived();
    /// Not a contract, or not the shape of an ERC-20 we can read.
    error BadToken();

    /// Pull `amount` from the caller, returning what actually arrived.
    function _pull(address token, uint256 amount) internal returns (uint256 received) {
        uint256 before = _balanceOf(token);
        (bool ok, bytes memory out) =
            token.call(abi.encodeWithSelector(bytes4(0x23b872dd), msg.sender, address(this), amount)); // transferFrom
        if (!ok || (out.length != 0 && !abi.decode(out, (bool)))) revert TokenTransferFailed();
        received = _balanceOf(token) - before;
        if (received == 0) revert NothingArrived();
    }

    function _push(address token, address to, uint256 amount) internal {
        (bool ok, bytes memory out) = token.call(abi.encodeWithSelector(bytes4(0xa9059cbb), to, amount)); // transfer
        if (!ok || (out.length != 0 && !abi.decode(out, (bool)))) revert TokenTransferFailed();
    }

    function _balanceOf(address token) private view returns (uint256 balance) {
        (bool ok, bytes memory out) = token.staticcall(abi.encodeWithSelector(bytes4(0x70a08231), address(this))); // balanceOf
        if (!ok || out.length != 32) revert BadToken();
        balance = abi.decode(out, (uint256));
    }
}

/// An ERC-20 (or LP token) escrow: deposited tokens are released to the
/// beneficiary at `unlockAt` and to nobody else, ever. The time can only be
/// pushed later, so a "locked for a year" badge on a launchpad page cannot be
/// walked back. Amounts are what arrived, so fee-on-transfer tokens work.
contract TokenLocker is TokenEscrow {
    struct Lock {
        address token;
        /// Who deposited (may extend; in the launchpad flow, the creator).
        address creator;
        /// The only address that can ever withdraw.
        address beneficiary;
        /// Still held in escrow; zero once withdrawn.
        uint256 amount;
        uint64 unlockAt;
    }

    error BadLock();
    error Unknown();
    error OnlyCreatorOrBeneficiary();
    error OnlyBeneficiary();
    error CannotShorten(uint64 unlockAt);
    error StillLocked(uint64 unlockAt);
    error AlreadyWithdrawn(uint256 id);

    event Deposited(uint256 indexed id, address indexed token, address indexed beneficiary, address creator, uint256 amount, uint64 unlockAt);
    event Extended(uint256 indexed id, uint64 indexed from, uint64 indexed to);
    event Withdrawn(uint256 indexed id, address indexed token, address indexed beneficiary, uint256 amount);

    Lock[] public locks;
    mapping(address => uint256[]) private _idsOf;

    /// Deposit `amount` of `token` (pulled from the caller) for `beneficiary`
    /// until `unlockAt`. Anyone may lock for anyone; the caller is the creator.
    function lock(address token, address beneficiary, uint256 amount, uint64 unlockAt) external returns (uint256 id) {
        if (beneficiary == address(0) || unlockAt <= block.timestamp) revert BadLock();
        uint256 received = _pull(token, amount);
        id = locks.length;
        locks.push(Lock(token, msg.sender, beneficiary, received, unlockAt));
        _idsOf[beneficiary].push(id);
        emit Deposited(id, token, beneficiary, msg.sender, received, unlockAt);
    }

    /// Push a lock's unlock time later. Only its creator or beneficiary may,
    /// and only while the lock still holds tokens.
    function extend(uint256 id, uint64 newUnlockAt) external {
        Lock storage l = _lockAt(id);
        if (l.amount == 0) revert AlreadyWithdrawn(id);
        if (msg.sender != l.creator && msg.sender != l.beneficiary) revert OnlyCreatorOrBeneficiary();
        if (newUnlockAt <= l.unlockAt) revert CannotShorten(l.unlockAt);
        emit Extended(id, l.unlockAt, newUnlockAt);
        l.unlockAt = newUnlockAt;
    }

    /// The beneficiary takes the whole lock once it has unlocked.
    function withdraw(uint256 id) external {
        Lock storage l = _lockAt(id);
        if (msg.sender != l.beneficiary) revert OnlyBeneficiary();
        uint256 amount = l.amount;
        if (amount == 0) revert AlreadyWithdrawn(id);
        if (uint64(block.timestamp) < l.unlockAt) revert StillLocked(l.unlockAt);
        l.amount = 0; // settled before the transfer, so a re-entry finds it gone
        _push(l.token, l.beneficiary, amount);
        emit Withdrawn(id, l.token, l.beneficiary, amount);
    }

    // ---- views (launchpad page, wallet) ----

    function lockCount() external view returns (uint256) {
        return locks.length;
    }

    function lockAt(uint256 id) external view returns (Lock memory) {
        return _lockAt(id);
    }

    /// Every lock ever made for `beneficiary`, spent ones included.
    function lockIdsOf(address beneficiary) external view returns (uint256[] memory) {
        return _idsOf[beneficiary];
    }

    /// `beneficiary`'s tokens of `token` still sitting in this escrow.
    function lockedTotal(address beneficiary, address token) external view returns (uint256 total) {
        uint256[] memory ids = _idsOf[beneficiary];
        for (uint256 i = 0; i < ids.length; i++) {
            Lock storage l = locks[ids[i]];
            if (l.amount != 0 && l.token == token) total += l.amount;
        }
    }

    /// The latest unlock time among those locks: the "creator's tokens locked
    /// until …" badge. Zero when nothing is locked.
    function lockedUntil(address beneficiary, address token) external view returns (uint64 until) {
        uint256[] memory ids = _idsOf[beneficiary];
        for (uint256 i = 0; i < ids.length; i++) {
            Lock storage l = locks[ids[i]];
            if (l.amount != 0 && l.token == token && l.unlockAt > until) until = l.unlockAt;
        }
    }

    function _lockAt(uint256 id) private view returns (Lock storage l) {
        if (id >= locks.length) revert Unknown();
        l = locks[id];
    }
}

/// A linear vesting escrow with an optional cliff (Sablier/Streamflow shaped):
/// `deposited` tokens vest evenly from `start` to `end`, nothing before the
/// cliff, and the beneficiary claims what has vested whenever they like. A
/// stream is cancelable only if its creator chose so at creation — the default
/// is not — and cancelling pays the vested part to the beneficiary, so a
/// creator can never use cancellation to claw back what has already vested.
contract TokenVesting is TokenEscrow {
    struct Stream {
        address token;
        address creator;
        address beneficiary;
        /// What actually arrived at creation.
        uint256 deposited;
        /// Claimed by the beneficiary so far (or settled by a cancel).
        uint256 withdrawn;
        uint64 start;
        /// Nothing vests before this (equals `start` when there is no cliff).
        uint64 cliffEnd;
        uint64 end;
        bool cancelable;
        bool canceled;
        /// What a cancel returned to the creator.
        uint256 refunded;
    }

    error BadStream();
    error Unknown();
    error OnlyBeneficiary();
    error OnlyCreator();
    error NotCancelable();
    error AlreadyCanceled();
    error NothingToClaim();

    event StreamCreated(
        uint256 indexed id, address indexed token, address indexed beneficiary, address creator, uint256 amount, uint64 start, uint64 cliffEnd, uint64 end, bool cancelable
    );
    event Claimed(uint256 indexed id, address indexed token, address indexed beneficiary, uint256 amount);
    event Canceled(uint256 indexed id, uint256 paidToBeneficiary, uint256 refundedToCreator);

    Stream[] public streams;
    mapping(address => uint256[]) private _idsOf;

    /// Vest `amount` of `token` (pulled from the caller) to `beneficiary`,
    /// linearly over `duration` from `start` (past or future), with an
    /// optional `cliff` (at most `duration`). `cancelable == false` is the
    /// default for team and advisor grants: the creator keeps no way out.
    function create(
        address token,
        address beneficiary,
        uint256 amount,
        uint64 start,
        uint64 cliff,
        uint64 duration,
        bool cancelable
    ) external returns (uint256 id) {
        if (beneficiary == address(0) || duration == 0 || cliff > duration) revert BadStream();
        uint256 received = _pull(token, amount);
        id = streams.length;
        streams.push(Stream(token, msg.sender, beneficiary, received, 0, start, start + cliff, start + duration, cancelable, false, 0));
        _idsOf[beneficiary].push(id);
        emit StreamCreated(id, token, beneficiary, msg.sender, received, start, start + cliff, start + duration, cancelable);
    }

    /// What has vested so far (the whole deposit once `end` has passed, zero
    /// before the cliff, and the settled amount after a cancel).
    function vested(uint256 id) public view returns (uint256) {
        Stream storage s = _streamAt(id);
        if (s.canceled) return s.deposited - s.refunded;
        return _vestedNow(s);
    }

    /// What the beneficiary could claim right now (always zero after a cancel,
    /// which settles the whole stream in one transaction).
    function claimable(uint256 id) public view returns (uint256) {
        Stream storage s = _streamAt(id);
        if (s.canceled) return 0;
        return _vestedNow(s) - s.withdrawn;
    }

    /// The beneficiary claims everything vested and not yet claimed.
    function claim(uint256 id) external {
        Stream storage s = _streamAt(id);
        if (msg.sender != s.beneficiary) revert OnlyBeneficiary();
        uint256 amount = claimable(id);
        if (amount == 0) revert NothingToClaim();
        s.withdrawn += amount; // settled before the transfer, so a re-entry finds it gone
        _push(s.token, s.beneficiary, amount);
        emit Claimed(id, s.token, s.beneficiary, amount);
    }

    /// The creator ends a cancelable stream: the vested part goes to the
    /// beneficiary, the unvested part back to the creator.
    function cancel(uint256 id) external {
        Stream storage s = _streamAt(id);
        if (msg.sender != s.creator) revert OnlyCreator();
        if (!s.cancelable) revert NotCancelable();
        if (s.canceled) revert AlreadyCanceled();
        uint256 vestedNow = _vestedNow(s);
        uint256 payout = vestedNow - s.withdrawn;
        uint256 refund = s.deposited - vestedNow;
        s.canceled = true;
        s.refunded = refund;
        s.withdrawn = s.deposited; // the beneficiary's share is settled by the payout below
        if (payout > 0) _push(s.token, s.beneficiary, payout);
        if (refund > 0) _push(s.token, msg.sender, refund);
        emit Canceled(id, payout, refund);
    }

    // ---- views ----

    function streamCount() external view returns (uint256) {
        return streams.length;
    }

    function streamAt(uint256 id) external view returns (Stream memory) {
        return _streamAt(id);
    }

    function streamIdsOf(address beneficiary) external view returns (uint256[] memory) {
        return _idsOf[beneficiary];
    }

    function _streamAt(uint256 id) private view returns (Stream storage s) {
        if (id >= streams.length) revert Unknown();
        s = streams[id];
    }

    function _vestedNow(Stream storage s) private view returns (uint256) {
        if (block.timestamp < s.cliffEnd) return 0;
        if (block.timestamp >= s.end) return s.deposited;
        return s.deposited * (block.timestamp - s.start) / (s.end - s.start);
    }
}
