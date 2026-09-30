// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {CommitteeRegistryV3} from "../src/CommitteeRegistryV3.sol";

interface VmV3 {
    function store(address, bytes32, bytes32) external;
    function prank(address) external;
    function roll(uint256) external;
}

contract CommitteeRegistryV3Test {
    VmV3 constant vm = VmV3(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    bytes32 constant KEY = bytes32(uint256(0x1234));

    function test_only_the_beaconer_can_announce_and_back_uses_the_same_slot() public {
        CommitteeRegistryV3 registry = new CommitteeRegistryV3();
        vm.store(address(registry), bytes32(uint256(2)), bytes32(uint256(1))); // one candidate
        vm.store(address(registry), bytes32(uint256(4)), bytes32(uint256(3_600)));
        vm.store(address(registry), keccak256(abi.encode(KEY, uint256(3))), bytes32(uint256(1)));
        bytes32 base = keccak256(abi.encode(uint256(2)));
        vm.store(address(registry), bytes32(uint256(base) + 3), bytes32(uint256(uint160(address(this)))));

        vm.prank(address(0xBEEF));
        try registry.announceLeaving(KEY) {
            revert("foreign beaconer announced");
        } catch (bytes memory reason) {
            require(bytes4(reason) == bytes4(keccak256("Unknown()")), "wrong rejection");
        }
        registry.announceLeaving(KEY);
        require(registry.availability(0) == (block.number + 1) * 2 + 1, "leaving word");
        uint256 left = block.number + 1;
        require(registry.lastLeaving(0) == left, "last leave recorded");
        vm.roll(30 * 3_600);
        registry.announceBack(KEY);
        require(registry.availability(0) == (block.number + 1) * 2, "back word");
        require(registry.lastLeaving(0) == left, "last leave survives back");

        // The inherited paid beacon path also preserves streak after a long
        // announced sleep; old registry versions keep their original rule.
        vm.store(address(registry), bytes32(uint256(base) + 4), bytes32(uint256(1) << 64));
        registry.beacon(KEY);
        (, , , , , uint64 lastEpoch, uint64 streak, uint64 missed) = registry.candidates(0);
        require(lastEpoch == 30 && streak == 2 && missed == 0, "announced gap is neutral");
    }
}
