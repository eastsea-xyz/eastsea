// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SimpleMultisig} from "../src/multisig/SimpleMultisig.sol";

interface VmMultisigFunding {
    function deal(address account, uint256 balance) external;
    function addr(uint256 key) external returns (address);
    function sign(uint256 key, bytes32 digest) external returns (uint8, bytes32, bytes32);
}

contract MultisigFundingTest {
    VmMultisigFunding constant vm = VmMultisigFunding(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testNativeFundingAndSignedPayout() public {
        uint256 ownerKey = 7;
        address[] memory owners = new address[](1);
        owners[0] = vm.addr(ownerKey);
        SimpleMultisig wallet = new SimpleMultisig(owners, 1);
        vm.deal(address(this), 10);
        (bool funded,) = address(wallet).call{value: 10}("");
        require(funded && address(wallet).balance == 10, "native funding refused");
        address payable recipient = payable(address(0xbeef));
        uint256 beforeBalance = recipient.balance;
        bytes32 digest = wallet.getTransactionHash(recipient, 6, "", 1);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(ownerKey, digest);
        bytes[] memory signatures = new bytes[](1);
        signatures[0] = abi.encodePacked(r, s, v);
        wallet.execute(recipient, 6, "", 1, signatures);
        require(address(wallet).balance == 4 && recipient.balance == beforeBalance + 6, "payout mismatch");
        require(wallet.executed(digest), "execution missing");
    }

    function testUnknownSelectorStillRefused() public {
        address[] memory owners = new address[](1);
        owners[0] = address(this);
        SimpleMultisig wallet = new SimpleMultisig(owners, 1);
        (bool accepted,) = address(wallet).call(hex"ffffffff");
        require(!accepted, "unknown selector accepted");
    }
}
