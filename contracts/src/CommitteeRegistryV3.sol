// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import "./CommitteeRegistry.sol";

/// New-genesis registry. The original registry stays installed on testnet 7780.
/// A status word is (block.number + 1) << 1 | leaving; zero means never announced.
/// Signed, fee-free node beacons write this same mapping as a system write.
contract CommitteeRegistryV3 is CommitteeRegistry {
    mapping(uint256 => uint256) public availability;
    /// Last leaving height survives a back announcement for epoch accounting.
    mapping(uint256 => uint256) public lastLeaving;

    event Availability(uint256 index, uint64 epoch, bool leaving);

    function announceLeaving(bytes32 validatorKey) external {
        _announce(validatorKey, true);
    }

    function announceBack(bytes32 validatorKey) external {
        _announce(validatorKey, false);
    }

    /// Paid fallback for a Mac that does not use the fee-free slot answers.
    /// The system beacon writer applies the same announced-gap rule.
    function beacon(bytes32 validatorKey) external override {
        uint256 i = indexOf[validatorKey];
        if (i == 0 || i == type(uint256).max) revert Unknown();
        Candidate storage c = candidates[i - 1];
        if (c.beaconer != msg.sender) revert Unknown();
        uint64 e = epoch();
        if (e == c.lastEpoch) return;
        uint64 gap = e - c.lastEpoch;
        uint256 left = lastLeaving[i - 1];
        uint256 status = availability[i - 1];
        bool announced = left != 0 && (left - 1) / epochBlocks >= c.lastEpoch
            && status != 0 && ((status & 1) == 1 || ((status >> 1) - 1) / epochBlocks + 1 >= e);
        if (announced) {
            c.streak += 1;
        } else if (gap <= GRACE_EPOCHS) {
            c.streak += 1;
            c.missed += gap - 1;
        } else {
            c.streak = 1;
            c.missed = 0;
        }
        c.lastEpoch = e;
        emit Beacon(i - 1, e, c.streak);
    }

    function _announce(bytes32 validatorKey, bool leaving) private {
        uint256 i = indexOf[validatorKey];
        if (i == 0 || i == type(uint256).max || candidates[i - 1].beaconer != msg.sender) revert Unknown();
        uint64 e = epoch();
        availability[i - 1] = (block.number + 1) << 1 | (leaving ? 1 : 0);
        if (leaving) lastLeaving[i - 1] = block.number + 1;
        emit Availability(i - 1, e, leaving);
    }
}
