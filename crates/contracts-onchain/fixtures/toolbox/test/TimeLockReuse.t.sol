// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {TokenTimeLock} from "../src/lock/TokenTimeLock.sol";
import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";
import {FixedSupplyToken} from "../src/token/FixedSupplyToken.sol";

interface VmTimeLockReuse {
    function warp(uint256 timestamp) external;
    function expectPartialRevert(bytes4 selector) external;
}

contract TimeLockReuseTest {
    VmTimeLockReuse constant vm = VmTimeLockReuse(address(uint160(uint256(keccak256("hevm cheat code")))));

    function fixture() private returns (FixedSupplyToken token, TokenTimeLock lock) {
        vm.warp(1_000_000);
        token = new FixedSupplyToken("Lock Test", "LOCK", 200, address(this));
        lock = new TokenTimeLock(address(this), IERC20(address(token)));
        token.approve(address(lock), type(uint256).max);
        lock.lockFor(address(this), 100, 0, 10);
    }

    function testFullyReleasedGrantCanBeReplacedAndHistoryRemainsUntilReplacement() public {
        (FixedSupplyToken token, TokenTimeLock lock) = fixture();
        vm.warp(block.timestamp + 11);
        lock.release();
        (uint128 amount, uint128 released,,,) = lock.locks(address(this));
        require(amount == 100 && released == 100, "completed history lost");
        require(lock.releasable(address(this)) == 0, "completed grant still payable");
        lock.lockFor(address(this), 75, 0, 5);
        (amount, released,,,) = lock.locks(address(this));
        require(amount == 75 && released == 0, "new grant not recorded");
        vm.warp(block.timestamp + 6);
        lock.release();
        require(token.balanceOf(address(this)) == 200, "new grant not paid");
    }

    function testActiveGrantCannotBeReplaced() public {
        (, TokenTimeLock lock) = fixture();
        vm.expectPartialRevert(TokenTimeLock.AlreadyLocked.selector);
        lock.lockFor(address(this), 75, 0, 5);
    }

    function testMaturedButUnreleasedGrantCannotBeReplaced() public {
        (, TokenTimeLock lock) = fixture();
        vm.warp(block.timestamp + 11);
        vm.expectPartialRevert(TokenTimeLock.AlreadyLocked.selector);
        lock.lockFor(address(this), 75, 0, 5);
    }
}
