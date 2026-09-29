// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {AetherAccount} from "../src/AetherAccount.sol";

interface VmSession {
    function prank(address) external;
    function warp(uint256) external;
    function mockCall(address, bytes calldata, bytes calldata) external;
    function expectRevert(bytes calldata) external;
}

contract TestToken {
    mapping(address => uint256) public balanceOf;
    function mint(address to, uint256 amount) external { balanceOf[to] += amount; }
    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }
    function approve(address, uint256) external pure returns (bool) { return true; }
}

contract FalseToken {
    function transfer(address, uint256) external pure returns (bool) { return false; }
}

contract AetherAccountSessionTest {
    VmSession constant vm = VmSession(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant PAYEE = address(0xB0B);
    AetherAccount account;
    TestToken token;

    function setUp() public {
        account = new AetherAccount();
        token = new TestToken();
        token.mint(address(account), 1_000);
        vm.mockCall(address(0x100), bytes(""), abi.encode(uint256(1)));
        address[] memory allow = new address[](1);
        allow[0] = PAYEE;
        vm.prank(address(account));
        account.addSession(AetherAccount.Key(bytes32(uint256(1)), bytes32(uint256(2))), 10, 20, 0, allow);
        vm.prank(address(account));
        account.setSessionToken(0, address(token), 100, 150);
    }

    function calls(bytes memory data) private view returns (AetherAccount.Call[] memory c) {
        c = new AetherAccount.Call[](1);
        c[0] = AetherAccount.Call(address(token), 0, data);
    }

    function pay(uint256 amount) private {
        account.sessionExecute(calls(abi.encodeWithSelector(token.transfer.selector, PAYEE, amount)), 0, 0, 0);
    }

    function testTokenPaymentAndCaps() public {
        vm.warp(100_000);
        pay(80);
        require(token.balanceOf(PAYEE) == 80, "payment");
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.OverPaymentLimit.selector, 101, 100));
        pay(101);
        pay(70);
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.OverDailyLimit.selector, 151, 150));
        pay(1);
        vm.warp(100_000 + 86_400);
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.OverDailyLimit.selector, 151, 150));
        pay(1);
    }

    function testBatchCannotSplitAroundTokenPerPaymentCap() public {
        AetherAccount.Call[] memory c = new AetherAccount.Call[](2);
        c[0] = AetherAccount.Call(address(token), 0, abi.encodeWithSelector(token.transfer.selector, PAYEE, 60));
        c[1] = AetherAccount.Call(address(token), 0, abi.encodeWithSelector(token.transfer.selector, PAYEE, 50));
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.OverPaymentLimit.selector, 110, 100));
        account.sessionExecute(c, 0, 0, 0);
        require(token.balanceOf(PAYEE) == 0, "batch reverted");
    }

    function testUnlistedPayeeAndTokenRefused() public {
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.RecipientNotAllowed.selector, address(0xBAD)));
        account.sessionExecute(calls(abi.encodeWithSelector(token.transfer.selector, address(0xBAD), 1)), 0, 0, 0);
        TestToken unknown = new TestToken();
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.TokenNotAllowed.selector, address(unknown)));
        AetherAccount.Call[] memory c = new AetherAccount.Call[](1);
        c[0] = AetherAccount.Call(address(unknown), 0, abi.encodeWithSelector(unknown.transfer.selector, PAYEE, 1));
        account.sessionExecute(c, 0, 0, 0);
    }

    function testMaliciousCalldataRefused() public {
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.NotAPayment.selector, 0));
        account.sessionExecute(calls(abi.encodeWithSelector(token.approve.selector, PAYEE, type(uint256).max)), 0, 0, 0);
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.NotAPayment.selector, 0));
        account.sessionExecute(calls(abi.encodePacked(abi.encodeWithSelector(token.transfer.selector, PAYEE, 1), bytes32(uint256(1)))), 0, 0, 0);
    }

    function testStoppedSessionRevertsOnNextPayment() public {
        vm.prank(address(account));
        account.removeSession(0);
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.BadSession.selector));
        pay(1);
    }

    function testFalseTokenTransferIsNotConfirmed() public {
        FalseToken bad = new FalseToken();
        vm.prank(address(account));
        account.setSessionToken(0, address(bad), 10, 20);
        AetherAccount.Call[] memory c = new AetherAccount.Call[](1);
        c[0] = AetherAccount.Call(address(bad), 0, abi.encodeWithSelector(bad.transfer.selector, PAYEE, 1));
        vm.expectRevert(abi.encodeWithSelector(AetherAccount.TokenTransferFailed.selector, 0));
        account.sessionExecute(c, 0, 0, 0);
    }
}
