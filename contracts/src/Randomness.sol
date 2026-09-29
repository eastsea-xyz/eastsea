// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// @notice Read-only view at REWARDS (0x0000000000000000000000000000000000007704).
/// The node writes keccak256("aether-randomness/v1" || uint64 epoch ||
/// uint64 draw || threshold BLS signature) to slot (9 << 200) | epoch in the
/// first block of each epoch.
/// Zero means that epoch has not opened or no signed draw seed existed yet.
/// A single proposer cannot bias the threshold signature. Its word is known
/// before the epoch opens (possibly several epochs ahead). Accept commitments
/// only before the draw seed is published; reveal after the target epoch opens.
contract Randomness {
    function randomness(uint64 epoch) external view returns (uint256 word) {
        uint256 slot = (uint256(9) << 200) | epoch;
        assembly {
            word := sload(slot)
        }
    }
}
