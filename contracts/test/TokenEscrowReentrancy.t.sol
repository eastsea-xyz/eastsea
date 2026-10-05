// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

// F-01 (contracts audit 2026-10-05): a callback-capable token re-enters an
// escrow deposit mid-pull. The nested deposit is credited to its own record
// and the outer pull then credits the whole balance delta again, so the
// records promise more than the escrow holds. These tests pin the fix — a
// reentrancy guard on every state-changing entry — with directed PoCs and
// funds-conservation fuzz runs.

import {TokenEscrow, TokenLocker, TokenVesting} from "../src/TokenLocker.sol";

interface Vm {
    function warp(uint256) external;
    function prank(address) external;
    function expectRevert(bytes calldata) external;
}

contract PlainToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function approve(address spender, uint256 v) external returns (bool) {
        allowance[msg.sender][spender] = v;
        return true;
    }

    function transfer(address to, uint256 v) external returns (bool) {
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

/// A token that calls `puppet.strike()` once from inside transferFrom, the
/// way a callback-capable (ERC-777-style) token can.
contract CallbackToken is PlainToken {
    address public puppet;
    bool public armed;

    function arm(address p) external {
        puppet = p;
        armed = true;
    }

    function transferFrom(address from, address to, uint256 v) external override returns (bool) {
        allowance[from][msg.sender] -= v;
        balanceOf[from] -= v;
        balanceOf[to] += v;
        if (armed && v > 0) {
            armed = false; // single shot: the nested pull must not recurse
            Puppet(puppet).strike();
        }
        return true;
    }
}

/// What the token calls mid-deposit. Every attempt is wrapped in try/catch
/// (a swallowed revert leaves the outer deposit running), except in the
/// callback-failure test where the revert is meant to propagate.
contract Puppet {
    TokenLocker internal locker;
    TokenVesting internal vesting;
    bool internal armed;
    bool internal tryNestedLock;
    bool internal tryNestedCreate;
    /// type(uint256).max = do not touch.
    uint256 internal payoutLockId = type(uint256).max;
    uint256 internal claimStreamId = type(uint256).max;
    uint256 internal nestedAmount = 100e18;

    bool public nestedLockSucceeded;
    bool public nestedCreateSucceeded;
    bool public payoutSucceeded;
    bool public claimSucceeded;

    function prepare(TokenLocker l, TokenVesting v, address t) external {
        locker = l;
        vesting = v;
        CallbackToken(t).approve(address(l), type(uint256).max);
        CallbackToken(t).approve(address(v), type(uint256).max);
    }

    /// What the deposit under attack will do, from inside the callback.
    function arm(uint256 payLock, uint256 claimStream, bool nestedLock, bool nestedCreate) external {
        payoutLockId = payLock;
        claimStreamId = claimStream;
        tryNestedLock = nestedLock;
        tryNestedCreate = nestedCreate;
        armed = true;
    }

    function strike() external {
        if (!armed) return;
        armed = false;
        if (tryNestedLock) {
            try locker.lock(address(CallbackToken(msg.sender)), address(this), nestedAmount, uint64(block.timestamp + 1 days)) {
                nestedLockSucceeded = true;
            } catch {}
        }
        if (tryNestedCreate) {
            try vesting.create(
                address(CallbackToken(msg.sender)), address(this), nestedAmount, uint64(block.timestamp), 0, 30 days, false
            ) {
                nestedCreateSucceeded = true;
            } catch {}
        }
        if (payoutLockId != type(uint256).max) {
            try locker.withdraw(payoutLockId) {
                payoutSucceeded = true;
            } catch {}
        }
        if (claimStreamId != type(uint256).max) {
            try vesting.claim(claimStreamId) {
                claimSucceeded = true;
            } catch {}
        }
    }

    // The outer deposits the token interrupts.
    function doLock(address token, uint256 amount, uint64 until) external returns (uint256) {
        return locker.lock(token, address(0xBEEF), amount, until);
    }

    function doCreate(address token, uint256 amount) external returns (uint256) {
        return vesting.create(token, address(0xBEEF), amount, uint64(block.timestamp), 0, 30 days, true);
    }
}

/// strike() reverts on purpose, and nothing catches it: the whole deposit
/// must unwind.
contract RevertingPuppet {
    function strike() external pure {
        revert("callback failed");
    }
}

contract TokenEscrowReentrancyTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    function assertEq(uint256 a, uint256 b, string memory why) internal pure {
        if (a != b) revert(why);
    }

    function assertTrue(bool c, string memory why) internal pure {
        if (!c) revert(why);
    }

    function assertFalse(bool c, string memory why) internal pure {
        if (c) revert(why);
    }

    /// Sum of everything the locker still owes: the funds-conservation
    /// invariant is `token balance of the escrow >= this`, read from the
    /// contract's own records so an illicit nested credit counts too.
    function outstandingLocks(TokenLocker l, address t) internal view returns (uint256 sum) {
        for (uint256 i = 0; i < l.lockCount(); i++) {
            TokenLocker.Lock memory lk = l.lockAt(i);
            if (lk.amount != 0 && lk.token == t) sum += lk.amount;
        }
    }

    function outstandingStreams(TokenVesting v, address t) internal view returns (uint256 sum) {
        for (uint256 i = 0; i < v.streamCount(); i++) {
            TokenVesting.Stream memory s = v.streamAt(i);
            if (s.token == t) sum += s.deposited - s.withdrawn;
        }
    }

    // ---- F-01 PoC 1: a nested deposit must not double-credit (locker) ----

    function test_NestedLockIsBlockedNotDoubleCredited() public {
        TokenLocker l = new TokenLocker();
        CallbackToken t = new CallbackToken();
        Puppet p = new Puppet();
        t.mint(address(p), 1_000e18);
        p.prepare(l, new TokenVesting(), address(t));
        p.arm(type(uint256).max, type(uint256).max, true, false); // nested lock only
        t.arm(address(p));

        // The outer deposit; the token interrupts it with a nested lock of
        // the same size. Before the fix both were credited while the escrow
        // held only one of them.
        p.doLock(address(t), 100e18, uint64(block.timestamp + 1 days));

        assertEq(l.lockCount(), 1, "the nested lock must be refused");
        assertFalse(p.nestedLockSucceeded(), "no nested deposit may succeed mid-pull");
        assertEq(t.balanceOf(address(l)), 100e18, "the escrow holds exactly one deposit");
        assertEq(outstandingLocks(l, address(t)), 100e18, "records promise exactly what is held");
        assertEq(l.lockAt(0).amount, 100e18, "the outer lock recorded its own delta only");

        // And the beneficiary can still be paid in full.
        vm.warp(block.timestamp + 2 days);
        vm.prank(address(0xBEEF));
        l.withdraw(0);
        assertEq(t.balanceOf(address(0xBEEF)), 100e18, "the lock pays out fully");
        assertEq(t.balanceOf(address(l)), 0, "nothing is stranded");
    }

    // ---- F-01 PoC 1b: same attack on the vesting escrow ----

    function test_NestedCreateIsBlockedNotDoubleCredited() public {
        TokenVesting v = new TokenVesting();
        CallbackToken t = new CallbackToken();
        Puppet p = new Puppet();
        t.mint(address(p), 1_000e18);
        p.prepare(new TokenLocker(), v, address(t));
        p.arm(type(uint256).max, type(uint256).max, false, true); // nested create only
        t.arm(address(p));

        p.doCreate(address(t), 100e18);

        assertEq(v.streamCount(), 1, "the nested stream must be refused");
        assertFalse(p.nestedCreateSucceeded(), "no nested stream may be created mid-pull");
        assertEq(t.balanceOf(address(v)), 100e18, "the escrow holds exactly one deposit");
        assertEq(outstandingStreams(v, address(t)), 100e18, "records promise exactly what is held");
    }

    // ---- F-01 PoC 2: a payout mid-deposit corrupts the accounting ----

    function test_WithdrawDuringDepositIsBlocked() public {
        TokenLocker l = new TokenLocker();
        CallbackToken t = new CallbackToken();
        Puppet p = new Puppet();
        t.mint(address(this), 1_000e18);
        t.approve(address(l), type(uint256).max);
        t.mint(address(p), 1_000e18);
        p.prepare(l, new TokenVesting(), address(t));

        // A pre-existing, already-unlocked lock owned by the puppet (so its
        // re-entrant withdraw would pass the beneficiary check), to be
        // drained while the new deposit is in flight.
        vm.warp(1_700_000_000);
        l.lock(address(t), address(p), 50e18, uint64(block.timestamp + 1));
        vm.warp(block.timestamp + 1);
        p.arm(0, type(uint256).max, false, false); // withdraw only
        t.arm(address(p));

        // Before the fix the payout left mid-pull, the outer deposit then
        // credited only the shrunken delta, and the difference sat in the
        // escrow owed to nobody.
        p.doLock(address(t), 100e18, uint64(block.timestamp + 1 days));

        assertFalse(p.payoutSucceeded(), "no payout may run mid-deposit");
        assertEq(outstandingLocks(l, address(t)), t.balanceOf(address(l)), "records match the holdings exactly");
        assertEq(outstandingLocks(l, address(t)), 150e18, "both locks are intact");

        // Both locks still pay in full: the puppet's 50 and the outer 100.
        vm.warp(block.timestamp + 2 days);
        vm.prank(address(p));
        l.withdraw(0);
        vm.prank(address(0xBEEF));
        l.withdraw(1);
        assertEq(t.balanceOf(address(p)), 1_000e18 - 100e18 + 50e18, "the puppet kept its deposit and its lock");
        assertEq(t.balanceOf(address(0xBEEF)), 100e18, "the outer lock paid in full");
        assertEq(t.balanceOf(address(l)), 0, "nothing is stranded");
    }

    // ---- F-01 PoC 2b: same attack through vesting.claim ----

    function test_ClaimDuringCreateIsBlocked() public {
        TokenVesting v = new TokenVesting();
        CallbackToken t = new CallbackToken();
        Puppet p = new Puppet();
        t.mint(address(this), 1_000e18);
        t.approve(address(v), type(uint256).max);
        t.mint(address(p), 1_000e18);
        p.prepare(new TokenLocker(), v, address(t));

        // A fully vested stream owned by the puppet.
        vm.warp(1_700_000_000);
        uint256 old = v.create(address(t), address(p), 60e18, uint64(block.timestamp - 100 days), 0, 30 days, false);
        vm.warp(block.timestamp + 1);
        p.arm(type(uint256).max, old, false, false); // claim only
        t.arm(address(p));

        p.doCreate(address(t), 100e18);

        assertFalse(p.claimSucceeded(), "no claim may run mid-deposit");
        assertEq(outstandingStreams(v, address(t)), t.balanceOf(address(v)), "records match the holdings exactly");
        assertEq(outstandingStreams(v, address(t)), 160e18, "both streams are intact");
    }

    // ---- F-01 PoC 3: a failing callback unwinds the whole deposit ----

    function test_RevertingCallbackRevertsTheDeposit() public {
        TokenLocker l = new TokenLocker();
        CallbackToken t = new CallbackToken();
        RevertingPuppet p = new RevertingPuppet();
        t.mint(address(this), 1_000e18);
        t.approve(address(l), type(uint256).max);
        t.arm(address(p));

        vm.expectRevert(abi.encodeWithSelector(TokenEscrow.TokenTransferFailed.selector));
        l.lock(address(t), address(0xBEEF), 100e18, uint64(block.timestamp + 1 days));

        assertEq(l.lockCount(), 0, "no lock was recorded");
        assertEq(t.balanceOf(address(l)), 0, "no tokens stayed behind");
        assertEq(t.balanceOf(address(this)), 1_000e18, "the depositor kept everything");

        TokenVesting v = new TokenVesting();
        t.approve(address(v), type(uint256).max);
        t.arm(address(p));
        vm.expectRevert(abi.encodeWithSelector(TokenEscrow.TokenTransferFailed.selector));
        v.create(address(t), address(0xBEEF), 100e18, uint64(block.timestamp), 0, 30 days, false);
        assertEq(v.streamCount(), 0, "no stream was recorded");
    }

    // ---- F-01 invariant fuzz: the escrow never owes more than it holds ----

    uint256 constant FUZZ_OPS = 16;

    function _seed(uint256 seed, uint256 i) internal pure returns (uint256) {
        return uint256(keccak256(abi.encode(seed, i)));
    }

    /// Random deposits, extends and withdrawals against an honest token:
    /// the locker's balance must equal the sum of outstanding locks after
    /// every step.
    function testFuzz_LockerConservationHonestToken(uint256 seed) public {
        vm.warp(1_700_000_000);
        TokenLocker l = new TokenLocker();
        PlainToken t = new PlainToken();
        t.mint(address(this), 1_000_000e18);
        t.approve(address(l), type(uint256).max);

        uint256 count = 0;
        uint256 outstanding = 0;
        for (uint256 i = 0; i < FUZZ_OPS; i++) {
            uint256 r = _seed(seed, i);
            uint256 op = r % 10;
            if (op < 5 && count < 8) {
                uint256 amount = (r / 10 % 1_000e18) + 1;
                uint64 until = uint64(block.timestamp + (r / 1_000 % 90 days) + 1);
                l.lock(address(t), address(this), amount, until);
                outstanding += amount;
                count++;
            } else if (op < 7 && count > 0) {
                uint256 id = r / 10 % count;
                vm.warp(block.timestamp + 91 days); // whatever was locked is due
                uint256 amount = l.lockAt(id).amount;
                try l.withdraw(id) {
                    outstanding -= amount;
                } catch {}
            } else if (count > 0) {
                try l.extend(r / 10 % count, uint64(block.timestamp + 365 days)) {} catch {}
            }
            assertEq(t.balanceOf(address(l)), outstanding, "conservation");
        }
    }

    /// Same walk, but the token interrupts every other deposit with the
    /// puppet's nested deposit and payout attempts. The invariant demanded
    /// by the audit is `balance >= the contract's own outstanding claims`;
    /// after the fix it holds with equality.
    function testFuzz_LockerConservationUnderAttack(uint256 seed) public {
        vm.warp(1_700_000_000);
        TokenLocker l = new TokenLocker();
        CallbackToken t = new CallbackToken();
        Puppet p = new Puppet();
        t.mint(address(this), 1_000_000e18);
        t.approve(address(l), type(uint256).max);
        t.mint(address(p), 1_000_000e18);
        p.prepare(l, new TokenVesting(), address(t));

        uint256 count = 0;
        for (uint256 i = 0; i < FUZZ_OPS; i++) {
            uint256 r = _seed(seed, i);
            uint256 op = r % 10;
            if (op < 6 && count < 8) {
                if (i % 2 == 0 && count > 0) {
                    // The armed deposit: the puppet tries to stack a nested
                    // lock and to drain an unlocked one mid-pull. Warp first:
                    // the unlock time is measured from the warped clock.
                    vm.warp(block.timestamp + 91 days);
                    p.arm(r / 10_000 % count, type(uint256).max, true, false);
                    t.arm(address(p));
                    l.lock(
                        address(t), address(this), (r / 10 % 1_000e18) + 1, uint64(block.timestamp + (r / 1_000 % 90 days) + 1)
                    );
                } else {
                    l.lock(
                        address(t), address(this), (r / 10 % 1_000e18) + 1, uint64(block.timestamp + (r / 1_000 % 90 days) + 1)
                    );
                }
                count++;
            } else if (count > 0) {
                uint256 id = r / 10 % count;
                vm.warp(block.timestamp + 91 days);
                try l.withdraw(id) {} catch {}
            }
            assertTrue(
                t.balanceOf(address(l)) >= outstandingLocks(l, address(t)), "F-01: the escrow owes more than it holds"
            );
        }
        assertEq(t.balanceOf(address(l)), outstandingLocks(l, address(t)), "conservation with equality after the fix");
    }

    /// The vesting twin, with nested creates and mid-deposit claims.
    function testFuzz_VestingConservationUnderAttack(uint256 seed) public {
        vm.warp(1_700_000_000);
        TokenVesting v = new TokenVesting();
        CallbackToken t = new CallbackToken();
        Puppet p = new Puppet();
        t.mint(address(this), 1_000_000e18);
        t.approve(address(v), type(uint256).max);
        t.mint(address(p), 1_000_000e18);
        p.prepare(new TokenLocker(), v, address(t));

        uint256 count = 0;
        for (uint256 i = 0; i < FUZZ_OPS; i++) {
            uint256 r = _seed(seed, i);
            uint256 op = r % 10;
            if (op < 6 && count < 8) {
                bool cancelable = r / 100_000 % 2 == 0;
                if (i % 2 == 0 && count > 0) {
                    p.arm(type(uint256).max, r / 10_000 % count, false, true);
                    t.arm(address(p));
                    v.create(address(t), address(this), (r / 10 % 1_000e18) + 1, uint64(block.timestamp), 0, 30 days, cancelable);
                } else {
                    v.create(address(t), address(this), (r / 10 % 1_000e18) + 1, uint64(block.timestamp), 0, 30 days, cancelable);
                }
                count++;
            } else if (op < 8 && count > 0) {
                vm.warp(block.timestamp + 31 days); // fully vested
                try v.claim(r / 10 % count) {} catch {}
            } else if (count > 0) {
                vm.warp(block.timestamp + 31 days);
                try v.cancel(r / 10 % count) {} catch {}
            }
            assertTrue(
                t.balanceOf(address(v)) >= outstandingStreams(v, address(t)), "F-01: the escrow owes more than it holds"
            );
        }
        assertEq(t.balanceOf(address(v)), outstandingStreams(v, address(t)), "conservation with equality after the fix");
    }
}
