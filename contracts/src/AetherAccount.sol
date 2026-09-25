// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// Code that an Aether account (a P-256 Secure Enclave key) delegates to with
/// EIP-7702: the account keeps its address and key, and gains batched calls.
/// It runs in the account's own context, so only the account itself (a tx it
/// signed, sent to its own address) may drive it.
contract AetherAccount {
    struct Call {
        address to;
        uint256 value;
        bytes data;
    }

    error OnlySelf();
    error CallFailed(uint256 index, bytes reason);

    event Executed(uint256 calls);

    /// Several calls, all or nothing, under one signature (one Touch ID).
    function execute(Call[] calldata calls) external payable {
        if (msg.sender != address(this)) revert OnlySelf();
        for (uint256 i = 0; i < calls.length; i++) {
            (bool ok, bytes memory reason) = calls[i].to.call{value: calls[i].value}(calls[i].data);
            if (!ok) revert CallFailed(i, reason);
        }
        emit Executed(calls.length);
    }

    receive() external payable {}
}
