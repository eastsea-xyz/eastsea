// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {EastSeaAccount} from "../src/EastSeaAccount.sol";
import {LibP256} from "./EastSeaVault.t.sol";

interface VmSession {
    function prank(address) external;
    function warp(uint256) external;
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

contract EastSeaAccountSessionTest {
    VmSession constant vm = VmSession(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant PAYEE = address(0xB0B);
    /// The session key's private scalar; the P256VERIFY precompile checks what
    /// LibP256 signs with it.
    uint256 constant SESSION_PRIVATE_KEY = 0x5e5e;
    EastSeaAccount account;
    TestToken token;

    function setUp() public {
        account = new EastSeaAccount();
        token = new TestToken();
        token.mint(address(account), 1_000);
        (bytes32 x, bytes32 y) = LibP256.derivePub(SESSION_PRIVATE_KEY);
        address[] memory allow = new address[](1);
        allow[0] = PAYEE;
        vm.prank(address(account));
        account.addSession(EastSeaAccount.Key(x, y), 10, 20, 0, allow);
        vm.prank(address(account));
        account.setSessionToken(0, address(token), 100, 150);
    }

    function calls(bytes memory data) private view returns (EastSeaAccount.Call[] memory c) {
        c = new EastSeaAccount.Call[](1);
        c[0] = EastSeaAccount.Call(address(token), 0, data);
    }

    /// The session key's signature over this batch, for the session's current
    /// id and nonce (what the account checks on chain). Computed before an
    /// `expectRevert`, so the revert expectation still covers the execution.
    function sessionSig(EastSeaAccount.Call[] memory c) private view returns (bytes32 r, bytes32 s) {
        EastSeaAccount.Session memory ss = account.session(0);
        bytes32 digest = account.sessionDigest(c, ss.id, ss.nonce);
        return LibP256.sign(SESSION_PRIVATE_KEY, digest, 0x3001);
    }

    function pay(uint256 amount) private {
        EastSeaAccount.Call[] memory c = calls(abi.encodeWithSelector(token.transfer.selector, PAYEE, amount));
        (bytes32 r, bytes32 s) = sessionSig(c);
        account.sessionExecute(c, 0, r, s);
    }

    function testTokenPaymentAndCaps() public {
        vm.warp(100_000);
        pay(80);
        require(token.balanceOf(PAYEE) == 80, "payment");
        EastSeaAccount.Call[] memory c = calls(abi.encodeWithSelector(token.transfer.selector, PAYEE, 101));
        (bytes32 r, bytes32 s) = sessionSig(c);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.OverPaymentLimit.selector, 101, 100));
        account.sessionExecute(c, 0, r, s);
        pay(70);
        c = calls(abi.encodeWithSelector(token.transfer.selector, PAYEE, 1));
        (r, s) = sessionSig(c);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.OverDailyLimit.selector, 151, 150));
        account.sessionExecute(c, 0, r, s);
        vm.warp(100_000 + 86_400);
        (r, s) = sessionSig(c);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.OverDailyLimit.selector, 151, 150));
        account.sessionExecute(c, 0, r, s);
    }

    function testBatchCannotSplitAroundTokenPerPaymentCap() public {
        EastSeaAccount.Call[] memory c = new EastSeaAccount.Call[](2);
        c[0] = EastSeaAccount.Call(address(token), 0, abi.encodeWithSelector(token.transfer.selector, PAYEE, 60));
        c[1] = EastSeaAccount.Call(address(token), 0, abi.encodeWithSelector(token.transfer.selector, PAYEE, 50));
        (bytes32 r, bytes32 s) = sessionSig(c);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.OverPaymentLimit.selector, 110, 100));
        account.sessionExecute(c, 0, r, s);
        require(token.balanceOf(PAYEE) == 0, "batch reverted");
    }

    function testUnlistedPayeeAndTokenRefused() public {
        EastSeaAccount.Call[] memory c = calls(abi.encodeWithSelector(token.transfer.selector, address(0xBAD), 1));
        (bytes32 r, bytes32 s) = sessionSig(c);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.RecipientNotAllowed.selector, address(0xBAD)));
        account.sessionExecute(c, 0, r, s);
        TestToken unknown = new TestToken();
        EastSeaAccount.Call[] memory u = new EastSeaAccount.Call[](1);
        u[0] = EastSeaAccount.Call(address(unknown), 0, abi.encodeWithSelector(unknown.transfer.selector, PAYEE, 1));
        (r, s) = sessionSig(u);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.TokenNotAllowed.selector, address(unknown)));
        account.sessionExecute(u, 0, r, s);
    }

    function testMaliciousCalldataRefused() public {
        EastSeaAccount.Call[] memory c =
            calls(abi.encodeWithSelector(token.approve.selector, PAYEE, type(uint256).max));
        (bytes32 r, bytes32 s) = sessionSig(c);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.NotAPayment.selector, 0));
        account.sessionExecute(c, 0, r, s);
        EastSeaAccount.Call[] memory t = calls(
            abi.encodePacked(abi.encodeWithSelector(token.transfer.selector, PAYEE, 1), bytes32(uint256(1)))
        );
        (r, s) = sessionSig(t);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.NotAPayment.selector, 0));
        account.sessionExecute(t, 0, r, s);
    }

    function testStoppedSessionRevertsOnNextPayment() public {
        vm.prank(address(account));
        account.removeSession(0);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.BadSession.selector));
        account.sessionExecute(calls(abi.encodeWithSelector(token.transfer.selector, PAYEE, 1)), 0, 0, 0);
    }

    function testFalseTokenTransferIsNotConfirmed() public {
        FalseToken bad = new FalseToken();
        vm.prank(address(account));
        account.setSessionToken(0, address(bad), 10, 20);
        EastSeaAccount.Call[] memory c = new EastSeaAccount.Call[](1);
        c[0] = EastSeaAccount.Call(address(bad), 0, abi.encodeWithSelector(bad.transfer.selector, PAYEE, 1));
        (bytes32 r, bytes32 s) = sessionSig(c);
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.TokenTransferFailed.selector, 0));
        account.sessionExecute(c, 0, r, s);
    }

    function testUnsignedBatchIsRefused() public {
        vm.expectRevert(abi.encodeWithSelector(EastSeaAccount.BadSession.selector));
        account.sessionExecute(calls(abi.encodeWithSelector(token.transfer.selector, PAYEE, 1)), 0, 0, 0);
    }
}
