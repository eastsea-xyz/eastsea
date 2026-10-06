// SPDX-License-Identifier: MIT
pragma solidity ^0.8.31;

/// @title Local deterministic entry brake for EastSea-native templates
/// @notice Shape follows PRIMITIVES.md M2: `brakeState()` and `brakeSpec()`,
///         plus the permissionless latch every native design specifies.
/// @dev Defined by Lane B because the Lane A0 shared brake file did not exist
///      when B0-B2 were written. If A0 lands a different file, this one is the
///      one to reconcile; the semantics below are the ones B0-B2 rely on.
///
///      - `guardian` is always address(0). No founder, curator, publisher,
///        oracle owner or creator can pause anything.
///      - The brake is a pure function of on-chain facts (`brakePredicate`).
///        Each instance documents its predicate bits in its SECURITY.md.
///      - `state` is 0 (entry open) or 1 (new entry closed; every exit stays
///        callable under its ordinary authorisation, deadline and solvency
///        checks). State 2 ("full stop") is never used by native templates.
///      - Entry points re-check the live predicate and revert when it holds.
///        A revert cannot persist a latch, so the latch is a separate call.
///      - `tripBrake()` succeeds only while the predicate holds and latches
///        permanently: a later recovery of the predicate does not reopen
///        entry. A second call after latching reverts.
///      - `since` is the block height of the latch (0 = never latched). An
///        unlatched but currently tripped predicate reports state 1, since 0.
interface INativeBrake {
    /// @notice Predicate bits shared by the Lane B templates.
    /// bit 0: the pinned token's code hash changed.
    /// bit 1: actual custody is below aggregate outstanding liability.
    /// bit 2: the id counter is exhausted.
    event BrakeLatched(uint64 sinceHeight, uint8 reasons);

    error BrakePredicateFalse();
    error BrakeAlreadyLatched(uint64 sinceHeight);
    error EntryBraked(uint8 reasons, uint64 sinceHeight);

    function brakeState() external view returns (uint8 state, address guardian, uint64 since);

    function brakeSpec() external view returns (string memory uri, bytes32 docSha256);

    /// @return reasons nonzero when the deterministic predicate holds now
    function brakePredicate() external view returns (uint8 reasons);

    /// @notice Anyone may latch the entry brake while the predicate holds.
    function tripBrake() external;
}
