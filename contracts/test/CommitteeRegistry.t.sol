// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

// The founder reserve keys never join the candidate pool (finding 1,
// 2026-09-29). The deployed bytecode is fixed — the proving program pins it —
// so the genesis state does the work instead: rewards::set_reserve prewrites
// type(uint256).max into this contract's `indexOf` slot for every reserve key,
// and `register()` checks `indexOf` before anything else, so a direct
// registration transaction (a valid one, not one the app built) reverts
// Known(). These tests pin the two facts that sentinel rests on: the storage
// layout (`indexOf` is mapping slot 3, the slot the Rust writer computes) and
// the revert order (Known() comes before the attestation precompile — the
// r and s below are zeros and are never reached).
//
// No forge-std here either (this toolchain predates the JSON cheatcodes):
// asserts revert on failure, negative cases use try/catch.

import {CommitteeRegistry} from "../src/CommitteeRegistry.sol";

interface Vm {
    function store(address, bytes32, bytes32) external;
    function load(address, bytes32) external view returns (bytes32);
}

abstract contract Test {
    function ok(bool c, string memory why) internal pure {
        if (!c) revert(why);
    }

    function eq(uint256 a, uint256 b, string memory why) internal pure {
        ok(a == b, why);
    }

    function revertedWith(bytes memory reason, bytes4 sel, string memory why) internal pure {
        ok(bytes4(reason) == sel, why);
    }
}

contract CommitteeRegistryTest is Test {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    CommitteeRegistry registry;
    bytes32 constant RESERVE_KEY = bytes32(uint256(0x5101)); // rewards::RESERVE_KEYS style
    bytes32 constant PLAIN_KEY = bytes32(uint256(0x6101));

    /// The word the genesis system write parks for a reserve key: indexOf's
    /// mapping slot is `key . uint256(3)` (crates/rewards set_reserve).
    function sentinelSlot(bytes32 key) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(key, bytes32(uint256(3))));
    }

    function setUp() public {
        registry = new CommitteeRegistry();
        // The genesis parameters (registrar key, epochBlocks) are storage
        // writes, not constructor arguments; None of them matter here.
        vm.store(address(registry), sentinelSlot(RESERVE_KEY), bytes32(type(uint256).max));
    }

    /// A direct registration of a reserve key — a caller with the fields of a
    /// valid one — reverts Known() on the sentinel alone.
    function test_the_genesis_sentinel_refuses_a_direct_registration() public {
        eq(registry.indexOf(RESERVE_KEY), type(uint256).max, "the sentinel word is the mapping's");
        eq(
            uint256(vm.load(address(registry), sentinelSlot(RESERVE_KEY))),
            type(uint256).max,
            "and it sits at the slot rewards::set_reserve writes"
        );
        try registry.register(RESERVE_KEY, bytes32(uint256(7)), address(this), bytes32(0), bytes32(0)) {
            revert("a reserve key must not register");
        } catch (bytes memory r) {
            revertedWith(r, CommitteeRegistry.Known.selector, "Known(), before the attestation check");
        }
        eq(registry.count(), 0, "nothing registered");
    }

    /// The same call on a key without a sentinel gets past Known() and dies at
    /// the attestation precompile instead: the revert above is the sentinel's
    /// doing, and the honest path (which the registrar's signature opens) is
    /// untouched.
    function test_a_key_without_the_sentinel_reaches_the_attestation_check() public {
        try registry.register(PLAIN_KEY, bytes32(uint256(7)), address(this), bytes32(0), bytes32(0)) {
            revert("zero r/s cannot pass the precompile");
        } catch (bytes memory r) {
            revertedWith(r, CommitteeRegistry.BadAttestation.selector, "BadAttestation, not Known");
        }
        eq(registry.count(), 0, "nothing registered");
    }
}
