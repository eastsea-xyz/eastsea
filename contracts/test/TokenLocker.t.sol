// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {TokenEscrow, TokenLocker, TokenVesting} from "../src/TokenLocker.sol";

interface Vm {
    function warp(uint256) external;
    function prank(address) external;
    function expectEmit(bool, bool, bool, bool) external;
    function expectRevert(bytes calldata) external;
}

contract ERC20Mock {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function approve(address spender, uint256 v) external returns (bool) {
        allowance[msg.sender][spender] = v;
        return true;
    }

    function transfer(address to, uint256 v) external virtual returns (bool) {
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v;
        return true;
    }

    function transferFrom(address from, address to, uint256 v) external virtual returns (bool) {
        allowance[from][msg.sender] -= v;
        balanceOf[from] -= v;
        balanceOf[to] += v;
        return true;
    }
}

/// A fee-on-transfer token: every move skims 1% to a collector, so the escrow
/// receives less than `transferFrom` was asked for.
contract FeeToken is ERC20Mock {
    address constant COLLECTOR = address(0xFEE);

    function transfer(address to, uint256 v) external override returns (bool) {
        _skim(msg.sender, to, v);
        return true;
    }

    function transferFrom(address from, address to, uint256 v) external override returns (bool) {
        allowance[from][msg.sender] -= v;
        _skim(from, to, v);
        return true;
    }

    function _skim(address from, address to, uint256 v) private {
        uint256 fee = v / 100;
        balanceOf[from] -= v;
        balanceOf[to] += v - fee;
        balanceOf[COLLECTOR] += fee;
    }
}

/// The beneficiary in the reentrancy tests: while the escrow pays it out, the
/// token calls back and it tries the same payout again.
contract ReentrantBeneficiary {
    TokenLocker locker;
    TokenVesting vesting;
    uint256 attackId;
    bool armed;
    bool public reentrySucceeded;

    function arm(TokenLocker l, TokenVesting v, uint256 id) external {
        locker = l;
        vesting = v;
        attackId = id;
        armed = true;
    }

    function withdraw() external {
        locker.withdraw(attackId);
    }

    function claim() external {
        vesting.claim(attackId);
    }

    /// Called by the token from inside transfer().
    function onTransfer() external {
        if (!armed) return;
        armed = false;
        try locker.withdraw(attackId) {
            reentrySucceeded = true;
        } catch {}
        try vesting.claim(attackId) {
            reentrySucceeded = true;
        } catch {}
    }
}

/// A token that calls `hook`'s onTransfer() whenever it is paid, i.e. from
/// inside the escrow's own payout transfer.
contract HookToken is ERC20Mock {
    address public hook;

    function setHook(address h) external {
        hook = h;
    }

    function transfer(address to, uint256 v) external override returns (bool) {
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v;
        if (to == hook && v > 0) ReentrantBeneficiary(to).onTransfer();
        return true;
    }
}

contract TokenLockerTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    address creator = address(0xC0DE);
    address beneficiary = address(0xBEEF);
    address stranger = address(0x9999);
    uint64 constant DAY = 1 days;
    /// 360 tokens over 360 days: exactly 1 token per day, no rounding.
    uint256 constant AMOUNT = 360e18;

    TokenLocker locker;
    TokenVesting vesting;
    ERC20Mock token;
    ERC20Mock other;
    FeeToken feeToken;
    HookToken hookToken;

    // Redeclared for expectEmit (this solc predates qualified event names).
    event Deposited(uint256 indexed id, address indexed token, address indexed beneficiary, address creator, uint256 amount, uint64 unlockAt);
    event Extended(uint256 indexed id, uint64 indexed from, uint64 indexed to);
    event Withdrawn(uint256 indexed id, address indexed token, address indexed beneficiary, uint256 amount);
    event StreamCreated(
        uint256 indexed id,
        address indexed token,
        address indexed beneficiary,
        address creator,
        uint256 amount,
        uint64 start,
        uint64 cliffEnd,
        uint64 end,
        bool cancelable
    );
    event Claimed(uint256 indexed id, address indexed token, address indexed beneficiary, uint256 amount);
    event Canceled(uint256 indexed id, uint256 paidToBeneficiary, uint256 refundedToCreator);

    function setUp() public {
        locker = new TokenLocker();
        vesting = new TokenVesting();
        token = new ERC20Mock();
        other = new ERC20Mock();
        feeToken = new FeeToken();
        hookToken = new HookToken();
        _fund(token);
        _fund(other);
        _fund(feeToken);
        _fund(hookToken);
    }

    function _fund(ERC20Mock tok) internal {
        tok.mint(creator, 1_000_000e18);
        vm.prank(creator);
        tok.approve(address(locker), type(uint256).max);
        vm.prank(creator);
        tok.approve(address(vesting), type(uint256).max);
    }

    function assertEq(uint256 a, uint256 b) internal pure {
        if (a != b) revert("uint256 mismatch");
    }

    function assertEq(address a, address b) internal pure {
        if (a != b) revert("address mismatch");
    }

    function assertTrue(bool c) internal pure {
        if (!c) revert("expected true");
    }

    function err(bytes4 e) internal pure returns (bytes memory) {
        return abi.encodeWithSelector(e);
    }

    function lockAs(address who, address tok, address to, uint256 v, uint64 until) internal returns (uint256 id) {
        vm.prank(who);
        id = locker.lock(tok, to, v, until);
    }

    // ---- lock ----

    function test_LockThenWithdrawAfterUnlock() public {
        uint64 until = uint64(block.timestamp + 30 * DAY);
        vm.expectEmit(true, true, true, true);
        emit Deposited(0, address(token), beneficiary, creator, 1000e18, until);
        uint256 id = lockAs(creator, address(token), beneficiary, 1000e18, until);

        assertEq(token.balanceOf(address(locker)), 1000e18);
        assertEq(locker.lockedTotal(beneficiary, address(token)), 1000e18);
        assertEq(locker.lockedUntil(beneficiary, address(token)), until);

        // Before the time: nobody can withdraw, only the beneficiary ever could.
        vm.prank(stranger);
        vm.expectRevert(err(TokenLocker.OnlyBeneficiary.selector));
        locker.withdraw(id);
        vm.prank(beneficiary);
        vm.expectRevert(abi.encodeWithSelector(TokenLocker.StillLocked.selector, until));
        locker.withdraw(id);

        vm.warp(block.timestamp + 30 * DAY);
        vm.expectEmit(true, true, true, true);
        emit Withdrawn(id, address(token), beneficiary, 1000e18);
        vm.prank(beneficiary);
        locker.withdraw(id);

        assertEq(token.balanceOf(beneficiary), 1000e18);
        assertEq(token.balanceOf(address(locker)), 0);
        assertEq(locker.lockedTotal(beneficiary, address(token)), 0);
        assertEq(locker.lockedUntil(beneficiary, address(token)), 0);
    }

    function test_WithdrawOnlyByBeneficiary() public {
        uint256 id = lockAs(creator, address(token), beneficiary, 1e18, uint64(block.timestamp + DAY));
        vm.warp(block.timestamp + 2 * DAY);
        vm.prank(creator);
        vm.expectRevert(err(TokenLocker.OnlyBeneficiary.selector));
        locker.withdraw(id);
        vm.prank(stranger);
        vm.expectRevert(err(TokenLocker.OnlyBeneficiary.selector));
        locker.withdraw(id);
    }

    function test_DoubleWithdrawReverts() public {
        uint256 id = lockAs(creator, address(token), beneficiary, 1e18, uint64(block.timestamp + DAY));
        vm.warp(block.timestamp + 2 * DAY);
        vm.prank(beneficiary);
        locker.withdraw(id);
        vm.prank(beneficiary);
        vm.expectRevert(abi.encodeWithSelector(TokenLocker.AlreadyWithdrawn.selector, id));
        locker.withdraw(id);
    }

    function test_ExtendByCreatorOrBeneficiary() public {
        uint64 until = uint64(block.timestamp + 30 * DAY);
        uint256 id = lockAs(creator, address(token), beneficiary, 1e18, until);
        vm.prank(stranger);
        vm.expectRevert(err(TokenLocker.OnlyCreatorOrBeneficiary.selector));
        locker.extend(id, uint64(block.timestamp + 90 * DAY));

        uint64 later = uint64(block.timestamp + 365 * DAY);
        vm.expectEmit(true, true, true, true);
        emit Extended(id, until, later);
        vm.prank(creator);
        locker.extend(id, later);
        assertEq(locker.lockedUntil(beneficiary, address(token)), later);

        // The beneficiary can push their own lock later too (a voluntary re-lock).
        uint64 latest = uint64(block.timestamp + 400 * DAY);
        vm.prank(beneficiary);
        locker.extend(id, latest);
        assertEq(locker.lockedUntil(beneficiary, address(token)), latest);
    }

    function test_ExtendNeverShorter() public {
        uint64 until = uint64(block.timestamp + 30 * DAY);
        uint256 id = lockAs(creator, address(token), beneficiary, 1e18, until);
        vm.prank(creator);
        vm.expectRevert(abi.encodeWithSelector(TokenLocker.CannotShorten.selector, until));
        locker.extend(id, uint64(block.timestamp + 29 * DAY));
        vm.prank(creator);
        vm.expectRevert(abi.encodeWithSelector(TokenLocker.CannotShorten.selector, until));
        locker.extend(id, until); // equal is not an extension either
    }

    function test_ExtendAfterWithdrawReverts() public {
        uint256 id = lockAs(creator, address(token), beneficiary, 1e18, uint64(block.timestamp + DAY));
        vm.warp(block.timestamp + 2 * DAY);
        vm.prank(beneficiary);
        locker.withdraw(id);
        vm.prank(creator);
        vm.expectRevert(abi.encodeWithSelector(TokenLocker.AlreadyWithdrawn.selector, id));
        locker.extend(id, uint64(block.timestamp + 90 * DAY));
    }

    function test_BadLockRejected() public {
        vm.prank(creator);
        vm.expectRevert(err(TokenLocker.BadLock.selector));
        locker.lock(address(token), address(0), 1e18, uint64(block.timestamp + DAY));
        vm.prank(creator);
        vm.expectRevert(err(TokenLocker.BadLock.selector));
        locker.lock(address(token), beneficiary, 1e18, uint64(block.timestamp)); // unlock now
        vm.prank(creator);
        vm.expectRevert(err(TokenLocker.BadLock.selector));
        locker.lock(address(token), beneficiary, 1e18, uint64(block.timestamp - 1));
    }

    function test_ZeroDepositReverts() public {
        vm.prank(creator);
        vm.expectRevert(err(TokenEscrow.NothingArrived.selector));
        locker.lock(address(token), beneficiary, 0, uint64(block.timestamp + DAY));
    }

    /// The launchpad badge: several locks across tokens, views aggregated and
    /// updated by withdrawal.
    function test_LaunchpadViewsAcrossLocks() public {
        uint64 d30 = uint64(block.timestamp + 30 * DAY);
        uint64 d90 = uint64(block.timestamp + 90 * DAY);
        uint64 d10 = uint64(block.timestamp + 10 * DAY);
        lockAs(creator, address(token), creator, 100e18, d30); // creator locks their own allocation
        lockAs(creator, address(token), creator, 50e18, d90);
        lockAs(creator, address(other), creator, 7e18, d10);
        lockAs(creator, address(other), beneficiary, 9e18, d90); // someone else's lock: not the creator's

        assertEq(locker.lockedTotal(creator, address(token)), 150e18);
        assertEq(locker.lockedUntil(creator, address(token)), d90);
        assertEq(locker.lockedTotal(creator, address(other)), 7e18);
        assertEq(locker.lockedUntil(creator, address(other)), d10);
        assertEq(locker.lockedUntil(beneficiary, address(other)), d90);
        assertEq(locker.lockIdsOf(creator).length, 3);

        // Withdraw one of the creator's locks: the badge follows.
        vm.warp(block.timestamp + 30 * DAY);
        vm.prank(creator);
        locker.withdraw(0);
        assertEq(locker.lockedTotal(creator, address(token)), 50e18);
        assertEq(locker.lockedUntil(creator, address(token)), d90);
    }

    function test_FeeOnTransferLock() public {
        uint64 until = uint64(block.timestamp + DAY);
        uint256 id = lockAs(creator, address(feeToken), beneficiary, 1000e18, until);
        // 1% was skimmed on the way in: the lock holds what arrived.
        assertEq(locker.lockedTotal(beneficiary, address(feeToken)), 990e18);
        assertEq(feeToken.balanceOf(address(locker)), 990e18);

        vm.warp(block.timestamp + 2 * DAY);
        vm.prank(beneficiary);
        locker.withdraw(id);
        assertEq(locker.lockedTotal(beneficiary, address(feeToken)), 0);
        // The escrow paid out its whole balance; the token skimmed the exit too.
        assertEq(feeToken.balanceOf(address(locker)), 0);
        assertEq(feeToken.balanceOf(beneficiary), 990e18 - 990e18 / 100);
    }

    function test_ReentrancyOnWithdraw() public {
        ReentrantBeneficiary attacker = new ReentrantBeneficiary();
        // A second lock for the same beneficiary keeps cover in the escrow, so a
        // late-settled double payout would actually go through here.
        lockAs(creator, address(hookToken), address(attacker), 1000e18, uint64(block.timestamp + DAY));
        uint256 id = lockAs(creator, address(hookToken), address(attacker), 1000e18, uint64(block.timestamp + DAY));
        attacker.arm(locker, vesting, id);
        hookToken.setHook(address(attacker));

        vm.warp(block.timestamp + 2 * DAY);
        attacker.withdraw();

        assertTrue(!attacker.reentrySucceeded());
        assertEq(hookToken.balanceOf(address(attacker)), 1000e18); // paid exactly once
        assertEq(hookToken.balanceOf(address(locker)), 1000e18); // the other lock is intact
    }

    // ---- vesting ----

    function vestAs(address who, address tok, address to, uint256 v, uint64 start, uint64 cliff, uint64 duration, bool cancelable)
        internal
        returns (uint256 id)
    {
        vm.prank(who);
        id = vesting.create(tok, to, v, start, cliff, duration, cancelable);
    }

    function test_LinearVestingClaims() public {
        uint64 start = uint64(block.timestamp);
        uint64 end = start + 360 * DAY;
        vm.expectEmit(true, true, true, true);
        emit StreamCreated(0, address(token), beneficiary, creator, AMOUNT, start, start, end, false);
        uint256 id = vestAs(creator, address(token), beneficiary, AMOUNT, start, 0, 360 * DAY, false);

        assertEq(vesting.claimable(id), 0);

        vm.warp(start + 90 * DAY);
        assertEq(vesting.vested(id), 90e18);
        assertEq(vesting.claimable(id), 90e18);
        vm.prank(beneficiary);
        vesting.claim(id);
        assertEq(token.balanceOf(beneficiary), 90e18);
        assertEq(vesting.claimable(id), 0); // claimed today, more vests tomorrow

        vm.warp(end);
        assertEq(vesting.vested(id), AMOUNT);
        assertEq(vesting.claimable(id), 270e18);
        vm.expectEmit(true, true, true, true);
        emit Claimed(id, address(token), beneficiary, 270e18);
        vm.prank(beneficiary);
        vesting.claim(id);
        assertEq(token.balanceOf(beneficiary), AMOUNT);

        vm.prank(beneficiary);
        vm.expectRevert(err(TokenVesting.NothingToClaim.selector));
        vesting.claim(id);
    }

    function test_Cliff() public {
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(token), beneficiary, AMOUNT, start, 90 * DAY, 360 * DAY, false);

        vm.warp(start + 89 * DAY);
        assertEq(vesting.vested(id), 0);
        vm.prank(beneficiary);
        vm.expectRevert(err(TokenVesting.NothingToClaim.selector));
        vesting.claim(id);

        // At the cliff the linear amount accrued so far unlocks at once.
        vm.warp(start + 90 * DAY);
        assertEq(vesting.vested(id), 90e18);
        vm.prank(beneficiary);
        vesting.claim(id);
        assertEq(token.balanceOf(beneficiary), 90e18);
    }

    function test_CliffEqualsDuration() public {
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(token), beneficiary, AMOUNT, start, 360 * DAY, 360 * DAY, false);
        vm.warp(start + 359 * DAY);
        assertEq(vesting.vested(id), 0);
        vm.warp(start + 360 * DAY);
        assertEq(vesting.vested(id), AMOUNT);
    }

    function test_BadStreamRejected() public {
        uint64 start = uint64(block.timestamp);
        vm.prank(creator);
        vm.expectRevert(err(TokenVesting.BadStream.selector));
        vesting.create(address(token), beneficiary, AMOUNT, start, 0, 0, false); // zero duration
        vm.prank(creator);
        vm.expectRevert(err(TokenVesting.BadStream.selector));
        vesting.create(address(token), beneficiary, AMOUNT, start, 361 * DAY, 360 * DAY, false); // cliff past the end
        vm.prank(creator);
        vm.expectRevert(err(TokenVesting.BadStream.selector));
        vesting.create(address(token), address(0), AMOUNT, start, 0, 360 * DAY, false);
    }

    function test_OnlyBeneficiaryClaims() public {
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(token), beneficiary, AMOUNT, start, 0, 360 * DAY, false);
        vm.warp(start + 10 * DAY);
        vm.prank(creator);
        vm.expectRevert(err(TokenVesting.OnlyBeneficiary.selector));
        vesting.claim(id);
        vm.prank(stranger);
        vm.expectRevert(err(TokenVesting.OnlyBeneficiary.selector));
        vesting.claim(id);
    }

    function test_FutureAndPastStart() public {
        // This forge's clock starts near zero: set a real one so a start in the
        // past can exist.
        vm.warp(1_700_000_000);
        uint64 now0 = uint64(block.timestamp);

        // A start in the future: nothing vests until it begins.
        uint256 a = vestAs(creator, address(token), beneficiary, AMOUNT, now0 + 30 * DAY, 0, 360 * DAY, false);
        vm.warp(now0 + 29 * DAY);
        assertEq(vesting.vested(a), 0);
        vm.warp(now0 + 66 * DAY); // 36 days into the stream
        assertEq(vesting.vested(a), 36e18);

        // A start in the past: the vesting already ran before creation.
        uint256 b = vestAs(creator, address(token), beneficiary, AMOUNT, now0 - 100 * DAY, 0, 360 * DAY, false);
        assertEq(vesting.vested(b), 166e18); // 100 days before creation + 66 since
        assertEq(vesting.claimable(b), 166e18);
    }

    function test_CancelMidVest() public {
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(token), beneficiary, AMOUNT, start, 0, 360 * DAY, true);
        vm.warp(start + 120 * DAY);

        vm.prank(stranger);
        vm.expectRevert(err(TokenVesting.OnlyCreator.selector));
        vesting.cancel(id);
        vm.expectEmit(true, true, true, true);
        emit Canceled(id, 120e18, 240e18);
        vm.prank(creator);
        vesting.cancel(id);

        assertEq(token.balanceOf(beneficiary), 120e18); // vested part still theirs
        assertEq(token.balanceOf(creator), 1_000_000e18 - AMOUNT + 240e18);
        assertEq(token.balanceOf(address(vesting)), 0);
        assertEq(vesting.vested(id), 120e18); // the entitlement froze at the cancel
        assertEq(vesting.claimable(id), 0);

        // Settled: no more claims, no second cancel.
        vm.prank(beneficiary);
        vm.expectRevert(err(TokenVesting.NothingToClaim.selector));
        vesting.claim(id);
        vm.prank(creator);
        vm.expectRevert(err(TokenVesting.AlreadyCanceled.selector));
        vesting.cancel(id);
    }

    function test_CancelBeforeCliffRefundsAll() public {
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(token), beneficiary, AMOUNT, start, 90 * DAY, 360 * DAY, true);
        vm.warp(start + 30 * DAY);
        vm.prank(creator);
        vesting.cancel(id);
        assertEq(token.balanceOf(beneficiary), 0);
        assertEq(token.balanceOf(creator), 1_000_000e18); // the whole deposit came back
        assertEq(vesting.vested(id), 0);
    }

    function test_CancelAfterEndPaysBeneficiaryEverything() public {
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(token), beneficiary, AMOUNT, start, 0, 360 * DAY, true);
        vm.warp(start + 400 * DAY);
        vm.prank(creator);
        vesting.cancel(id);
        assertEq(token.balanceOf(beneficiary), AMOUNT); // fully vested: cancel cannot claw back
        assertEq(token.balanceOf(creator), 1_000_000e18 - AMOUNT);
        assertEq(vesting.claimable(id), 0);
    }

    function test_DefaultNotCancelable() public {
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(token), beneficiary, AMOUNT, start, 0, 360 * DAY, false);
        vm.warp(start + 120 * DAY);
        vm.prank(creator);
        vm.expectRevert(err(TokenVesting.NotCancelable.selector));
        vesting.cancel(id);
        // The stream survives and keeps vesting.
        assertEq(vesting.claimable(id), 120e18);
    }

    function test_FeeOnTransferVesting() public {
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(feeToken), beneficiary, 1000e18, start, 0, 360 * DAY, true);
        TokenVesting.Stream memory s = vesting.streamAt(id);
        assertEq(s.deposited, 990e18); // what arrived, not what was asked

        vm.warp(start + 180 * DAY);
        assertEq(vesting.claimable(id), 495e18);
        vm.prank(beneficiary);
        vesting.claim(id);
        assertEq(feeToken.balanceOf(address(vesting)), 495e18);

        vm.prank(creator);
        vesting.cancel(id);
        assertEq(feeToken.balanceOf(address(vesting)), 0); // the other half went back to the creator
        assertEq(feeToken.balanceOf(creator), 1_000_000e18 - 1000e18 + 495e18 * 99 / 100); // the token skimmed the exit too
    }

    function test_ReentrancyOnClaim() public {
        ReentrantBeneficiary attacker = new ReentrantBeneficiary();
        uint64 start = uint64(block.timestamp);
        uint256 id = vestAs(creator, address(hookToken), address(attacker), AMOUNT, start, 0, 360 * DAY, false);
        attacker.arm(locker, vesting, id);
        hookToken.setHook(address(attacker));

        vm.warp(start + 180 * DAY);
        attacker.claim();

        assertTrue(!attacker.reentrySucceeded());
        assertEq(hookToken.balanceOf(address(attacker)), 180e18); // the mid-stream share, exactly once
        assertEq(hookToken.balanceOf(address(vesting)), 180e18); // the rest still vests
    }

    function test_UnknownIdsRevert() public {
        vm.expectRevert(err(TokenLocker.Unknown.selector));
        locker.withdraw(0);
        vm.expectRevert(err(TokenVesting.Unknown.selector));
        vesting.claim(0);
        vm.expectRevert(err(TokenVesting.Unknown.selector));
        vesting.vested(0);
    }
}
