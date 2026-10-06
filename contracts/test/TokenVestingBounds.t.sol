// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {TokenVesting} from "../src/TokenLocker.sol";

interface VestingBoundsVm {
    function warp(uint256 timestamp) external;
    function prank(address caller) external;
}

/// A plain ERC-20 with no supply cap below uint256: valid large deposits must
/// remain claimable throughout the schedule, not just after the end.
contract VestingBoundsToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external { balanceOf[to] += amount; }
    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }
    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }
    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract TokenVestingBoundsTest {
    VestingBoundsVm constant vm = VestingBoundsVm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    address constant BENEFICIARY = address(0xBEEF);

    function stream(uint256 amount, uint64 duration) private returns (TokenVesting vesting, VestingBoundsToken token) {
        vm.warp(1_000);
        vesting = new TokenVesting();
        token = new VestingBoundsToken();
        token.mint(address(this), amount);
        token.approve(address(vesting), amount);
        vesting.create(address(token), BENEFICIARY, amount, 1_000, 0, duration, true);
    }

    function test_MaxDepositMidpointClaimAndFinalClaimConserveSupply() public {
        (TokenVesting vesting, VestingBoundsToken token) = stream(type(uint256).max, 100);
        vm.warp(1_050);
        require(vesting.vested(0) == type(uint256).max / 2, "midpoint floor");
        vm.prank(BENEFICIARY);
        vesting.claim(0);
        require(token.balanceOf(BENEFICIARY) == type(uint256).max / 2, "partial claim");
        vm.warp(1_100);
        vm.prank(BENEFICIARY);
        vesting.claim(0);
        require(token.balanceOf(BENEFICIARY) == type(uint256).max, "final claim");
        require(token.balanceOf(address(vesting)) == 0, "escrow settled");
    }

    function test_MaxDepositPartialClaimThenCancelConservesSupply() public {
        (TokenVesting vesting, VestingBoundsToken token) = stream(type(uint256).max, 100);
        vm.warp(1_025);
        vm.prank(BENEFICIARY);
        vesting.claim(0);
        vm.warp(1_050);
        vesting.cancel(0);
        uint256 paid = token.balanceOf(BENEFICIARY);
        uint256 refunded = token.balanceOf(address(this));
        require(paid == type(uint256).max / 2, "cancel pays all vested");
        require(refunded == type(uint256).max - paid, "unvested refund");
        require(token.balanceOf(address(vesting)) == 0, "escrow settled");
        require(vesting.claimable(0) == 0, "canceled cannot claim again");
    }

    function test_MaxDepositNonDivisibleScheduleRoundsDown() public {
        (TokenVesting vesting,) = stream(type(uint256).max, 3);
        vm.warp(1_002);
        // 2**256 - 1 is divisible by 3; the expected value avoids an
        // intermediate product and is independent of implementation details.
        require(vesting.vested(0) == (type(uint256).max / 3) * 2, "two-thirds");
    }

    function testFuzz_ClaimThenCancelConservesDeposit(uint256 amount, uint64 elapsed, uint64 duration) public {
        if (amount == 0) amount = 1;
        // Cover the full representable schedule range while leaving room for
        // the stream's start time in the checked uint64 end calculation.
        duration = duration % (type(uint64).max - 1_000) + 1;
        elapsed %= duration;
        (TokenVesting vesting, VestingBoundsToken token) = stream(amount, duration);
        vm.warp(1_000 + elapsed);
        uint256 vested = vesting.vested(0);
        require(vested <= amount, "vesting bounded by deposit");
        if (vested != 0) {
            vm.prank(BENEFICIARY);
            vesting.claim(0);
        }
        vesting.cancel(0);
        require(token.balanceOf(BENEFICIARY) == vested, "vested allocation conserved");
        require(token.balanceOf(address(this)) == amount - vested, "refund conserved");
        require(token.balanceOf(address(vesting)) == 0, "escrow settled");
    }
}
