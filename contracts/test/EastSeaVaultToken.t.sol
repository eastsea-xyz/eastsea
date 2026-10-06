// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

// F-03 (contracts audit 2026-10-05), vault half: a queued ERC-20 withdrawal
// whose token has no code, or reports success without paying, must not emit
// WithdrawalExecuted. No-code tokens are refused at proposal time; at
// execution the recipient's balance must actually rise by the amount.

import {EastSeaVault, EastSeaVaultFactory} from "../src/EastSeaVault.sol";
import {LibP256, Vm} from "./EastSeaVault.t.sol";

/// A dishonest token: has code, answers true, never moves a balance.
contract VaultLiarToken {
    mapping(address => uint256) public balanceOf;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function transfer(address, uint256) external pure returns (bool) {
        return true;
    }
}

/// A fee-on-transfer token: every arrival is skimmed 1%.
contract VaultFeeToken {
    mapping(address => uint256) public balanceOf;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function transfer(address to, uint256 v) external returns (bool) {
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v - v / 100;
        return true;
    }
}

contract EastSeaVaultTokenTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    uint256 constant D0 = 0x1001;
    uint256 constant D1 = 0x1002;
    uint256 constant K = 0x2001;

    EastSeaVaultFactory factory;
    EastSeaVault vault;
    address recipient = address(0xBEEF);

    function setUp() public {
        factory = new EastSeaVaultFactory();
        EastSeaVault.Key[] memory ks = new EastSeaVault.Key[](2);
        (bytes32 x0, bytes32 y0) = LibP256.derivePub(D0);
        ks[0] = EastSeaVault.Key(x0, y0);
        (bytes32 x1, bytes32 y1) = LibP256.derivePub(D1);
        ks[1] = EastSeaVault.Key(x1, y1);
        vault = EastSeaVault(payable(factory.create(ks, 2, 1 ether, 24 hours, bytes32(uint256(0x71)))));
    }

    // ---- helpers ----

    function assertEq(uint256 a, uint256 b, string memory why) internal pure {
        if (a != b) revert(why);
    }

    function err(bytes4 e) internal pure returns (bytes memory) {
        return abi.encodeWithSelector(e);
    }

    function withdrawSig(uint256 id, address tok, address to, uint256 amount, uint256 d)
        internal
        view
        returns (bytes32 r, bytes32 s)
    {
        return LibP256.sign(d, vault.withdrawDigest(id, tok, to, amount), K);
    }

    /// Propose an ERC-20 withdrawal and approve it to the 2-of-2 threshold.
    function readyTokenWithdrawal(address tok, uint256 amount, uint256 id) internal {
        (bytes32 r, bytes32 s) = withdrawSig(id, tok, recipient, amount, D0);
        vault.proposeWithdrawal(tok, recipient, amount, 0, r, s);
        (r, s) = withdrawSig(id, tok, recipient, amount, D1);
        vault.approve(id, 1, r, s);
    }

    // ---- F-03: a no-code token never reaches the queue ----

    function test_NoCodeTokenProposalIsRefused() public {
        (bytes32 r, bytes32 s) = withdrawSig(1, address(0xDEAD), recipient, 100e18, D0);
        vm.expectRevert(err(EastSeaVault.BadToken.selector));
        vault.proposeWithdrawal(address(0xDEAD), recipient, 100e18, 0, r, s);
        assertEq(vault.proposalCount(), 0, "nothing was queued");
    }

    // ---- F-03: a lying token cannot buy a WithdrawalExecuted event ----

    function test_ExecutionRefusesATokenThatPaysNothing() public {
        VaultLiarToken liar = new VaultLiarToken();
        liar.mint(address(vault), 100e18);

        readyTokenWithdrawal(address(liar), 100e18, 1);
        vm.warp(block.timestamp + 25 hours);

        vm.expectRevert(abi.encodeWithSelector(EastSeaVault.ShortDelivery.selector, 0, 100e18));
        vault.execute(1);
        assertEq(liar.balanceOf(recipient), 0, "nothing was paid");
    }

    // ---- F-03: the movement must be exact — a fee-on-transfer token is refused ----

    function test_ExecutionRefusesAFeeOnTransferToken() public {
        VaultFeeToken fee = new VaultFeeToken();
        fee.mint(address(vault), 100e18);

        readyTokenWithdrawal(address(fee), 100e18, 1);
        vm.warp(block.timestamp + 25 hours);

        // 1% is skimmed on arrival, so the recipient's delta is 99e18
        vm.expectRevert(abi.encodeWithSelector(EastSeaVault.ShortDelivery.selector, 99e18, 100e18));
        vault.execute(1);
        assertEq(fee.balanceOf(recipient), 0, "nothing left the vault");
        assertEq(fee.balanceOf(address(vault)), 100e18, "the vault kept everything");
    }
}
