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
    error NoGuardian();
    error BadGuardianSignature();

    event Executed(uint256 calls);
    event GuardianSet(bytes32 x, bytes32 y);
    event GuardianExecuted(uint256 nonce, uint256 calls);

    /// P256VERIFY (EIP-7951 / RIP-7212): sha256 digest, r, s, x, y -> 1 on success.
    address constant P256VERIFY = address(0x100);

    /// Storage lives in the delegating account itself, so use a namespaced slot
    /// (ERC-7201 style) that no other code at this address will collide with.
    /// keccak256(abi.encode(uint256(keccak256("aether.account.guardian")) - 1)) & ~0xff
    bytes32 constant GUARDIAN_SLOT = 0x814c365e7a9c4fa1d0da41caf4c2bc2af8cb172a9b60ca9932fe088002417e00;

    struct Guardian {
        bytes32 x;
        bytes32 y;
        uint256 nonce;
    }

    function _guardian() private pure returns (Guardian storage g) {
        bytes32 slot = GUARDIAN_SLOT;
        assembly {
            g.slot := slot
        }
    }

    /// Several calls, all or nothing, under one signature (one Touch ID).
    function execute(Call[] calldata calls) external payable {
        if (msg.sender != address(this)) revert OnlySelf();
        for (uint256 i = 0; i < calls.length; i++) {
            (bool ok, bytes memory reason) = calls[i].to.call{value: calls[i].value}(calls[i].data);
            if (!ok) revert CallFailed(i, reason);
        }
        emit Executed(calls.length);
    }

    /// Register (or replace, or clear with zeros) a recovery key: the P-256
    /// public key of a second device's Secure Enclave.
    function setGuardian(bytes32 x, bytes32 y) external {
        if (msg.sender != address(this)) revert OnlySelf();
        Guardian storage g = _guardian();
        g.x = x;
        g.y = y;
        emit GuardianSet(x, y);
    }

    function guardian() external view returns (bytes32 x, bytes32 y, uint256 nonce) {
        Guardian storage g = _guardian();
        return (g.x, g.y, g.nonce);
    }

    /// What the guardian signs (SHA-256, as a Secure Enclave signs).
    function guardianDigest(Call[] calldata calls, uint256 nonce) public view returns (bytes32) {
        return sha256(abi.encode(block.chainid, address(this), nonce, calls));
    }

    /// Run calls authorized by the guardian key: recovery when the main key is
    /// lost. Anyone may relay it; the nonce prevents replay.
    function guardianExecute(Call[] calldata calls, bytes32 r, bytes32 s) external {
        Guardian storage g = _guardian();
        if (g.x == bytes32(0) && g.y == bytes32(0)) revert NoGuardian();
        uint256 nonce = g.nonce;
        bytes32 digest = guardianDigest(calls, nonce);
        (bool ok, bytes memory out) = P256VERIFY.staticcall(abi.encodePacked(digest, r, s, g.x, g.y));
        if (!ok || out.length != 32 || abi.decode(out, (uint256)) != 1) revert BadGuardianSignature();
        g.nonce = nonce + 1;
        for (uint256 i = 0; i < calls.length; i++) {
            (bool success, bytes memory reason) = calls[i].to.call{value: calls[i].value}(calls[i].data);
            if (!success) revert CallFailed(i, reason);
        }
        emit GuardianExecuted(nonce, calls.length);
    }

    receive() external payable {}
}
