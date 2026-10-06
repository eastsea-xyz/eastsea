// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Deterministic local entry brake (EastSea native plan, step 3; PRIMITIVES M2)
/// @notice Selector-compatible with the proposed registry interface
/// `brakeState()` / `brakeSpec()`, plus the permissionless latch every native
/// template exposes. There is no chain-wide brake and no central guardian.
///
/// Rules every implementation follows (checked by the executor harness):
/// 1. `guardian` is always `address(0)`. No founder, curator, publisher, oracle
///    owner or deployer can set, clear or choose the brake state.
/// 2. `brakePredicate()` is a pure function of on-chain state: no caller input,
///    no signature, no off-chain report. Anyone can evaluate it with eth_call.
/// 3. `tripBrake()` is callable by anyone and never reverts. It latches iff the
///    predicate holds and returns whether the brake is latched afterwards. While
///    the predicate does not hold it changes nothing (a caller cannot pause).
/// 4. The latch is monotonic: once `state == 1` it never returns to 0. There is
///    no unlatch, upgrade, owner or fallback path.
/// 5. Entry functions (new deposits, purchases, campaigns, borrows) re-evaluate
///    the predicate themselves and refuse while it holds, latched or not, so a
///    reverting entry is never relied on to persist the latch.
/// 6. Exits (withdraw, refund, repay, settle, claim) never check the brake.
///    They keep their own conservation, solvency and token-transfer
///    preconditions: "exit open" is not a solvency guarantee.
/// 7. `brakeSpec()` names the per-item specification (predicate, latch, exits)
///    and pins its SHA-256 so a manifest can record the exact document.
interface IEastSeaLocalBrake {
    /// @notice The first successful latch. `reason` is the implementation's predicate tag.
    event BrakeLatched(bytes32 indexed reason, uint64 since);

    /// @return state    0 = entry open, 1 = entry halted (latched; exits open). 2 is never used.
    /// @return guardian always address(0)
    /// @return since    block timestamp of the latch, 0 while open
    function brakeState() external view returns (uint8 state, address guardian, uint64 since);

    /// @return uri       where the per-item brake specification lives
    /// @return docSha256 SHA-256 of that document's exact bytes
    function brakeSpec() external view returns (string memory uri, bytes32 docSha256);

    /// @return holds  whether the deterministic predicate holds now
    /// @return reason the predicate tag that holds, zero otherwise
    function brakePredicate() external view returns (bool holds, bytes32 reason);

    /// @notice Permissionless and nonreverting. Latches iff `brakePredicate()` holds.
    /// @return latched whether the brake is latched after the call
    function tripBrake() external returns (bool latched);
}
