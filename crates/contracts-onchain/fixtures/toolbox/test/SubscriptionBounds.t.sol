// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SubscriptionManager} from "../src/subscription/SubscriptionManager.sol";

interface VmSubscriptionBounds {
    function deal(address account, uint256 balance) external;
    function expectRevert(bytes4 selector) external;
    function warp(uint256 timestamp) external;
}

/// Regression tests for narrowing casts in the vendored subscription example.
contract SubscriptionBoundsTest {
    VmSubscriptionBounds private constant vm = VmSubscriptionBounds(address(uint160(uint256(keccak256("hevm cheat code")))));
    bytes4 private constant TOO_LARGE = bytes4(keccak256("AmountTooLarge()"));

    receive() external payable {}

    function test_secondsAboveUint64CannotChargeWithoutAddingTime() public {
        vm.warp(1_000_000);
        SubscriptionManager sub = new SubscriptionManager(address(this), address(0xbeef), 1);
        uint256 amount = uint256(1) << 64;
        vm.deal(address(this), amount);
        vm.expectRevert(TOO_LARGE);
        sub.subscribe{value: amount}();
        require(sub.refundReserve() == 0 && address(sub).balance == 0, "failed payment changed reserve");
    }

    function test_valueAboveUint88CannotBecomeAnUnreleasableReserve() public {
        vm.warp(1_000_000);
        SubscriptionManager sub = new SubscriptionManager(address(this), address(0xbeef), uint256(1) << 32);
        uint256 amount = uint256(1) << 88; // duration fits uint64; principal does not fit uint88
        vm.deal(address(this), amount);
        vm.expectRevert(TOO_LARGE);
        sub.subscribe{value: amount}();
        require(sub.refundReserve() == 0 && address(sub).balance == 0, "truncated principal was retained");
    }

    function test_cumulativePrincipalCannotCrossUint88() public {
        vm.warp(1_000_000);
        uint256 rate = uint256(1) << 32;
        uint256 first = (uint256(1) << 88) - rate;
        SubscriptionManager sub = new SubscriptionManager(address(this), address(0xbeef), rate);
        vm.deal(address(this), first + rate);
        sub.subscribe{value: first}();
        (uint64 expiry, uint88 principal) = sub.subscribers(address(this));
        vm.expectRevert(TOO_LARGE);
        sub.subscribe{value: rate}();
        (uint64 afterExpiry, uint88 afterPrincipal) = sub.subscribers(address(this));
        require(afterExpiry == expiry && afterPrincipal == principal, "failed extension changed subscriber");
        require(sub.refundReserve() == first && address(sub).balance == first, "failed extension changed principal");
    }

    function test_exactUint64ExpiryBoundaryAcceptedAndExtensionRejected() public {
        vm.warp(1_000_000);
        SubscriptionManager sub = new SubscriptionManager(address(this), address(0xbeef), 1);
        uint256 amount = uint256(type(uint64).max) - block.timestamp;
        vm.deal(address(this), amount + 1);
        sub.subscribe{value: amount}();
        (uint64 expiry,) = sub.subscribers(address(this));
        require(expiry == type(uint64).max, "valid exact expiry boundary rejected");
        vm.expectRevert(TOO_LARGE);
        sub.subscribe{value: 1}();
    }

    function test_exactUint88PrincipalBoundaryAccepted() public {
        vm.warp(1_000_000);
        uint256 amount = type(uint88).max;
        SubscriptionManager sub = new SubscriptionManager(address(this), address(0xbeef), amount);
        vm.deal(address(this), amount);
        sub.subscribe{value: amount}();
        (uint64 expiry, uint88 principal) = sub.subscribers(address(this));
        require(expiry == block.timestamp + 1 && principal == amount, "valid exact principal boundary rejected");
        require(sub.refundReserve() == amount, "principal reserve mismatch");
    }
}
