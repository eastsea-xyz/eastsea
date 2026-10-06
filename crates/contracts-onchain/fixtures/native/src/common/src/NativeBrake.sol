// SPDX-License-Identifier: MIT
pragma solidity ^0.8.31;

import {INativeBrake} from "./INativeBrake.sol";

/// @title Shared latch logic for INativeBrake
/// @dev The inheriting contract owns its packed storage word; this base only
///      reads and writes the latch height through the two hooks, so no extra
///      storage slot is occupied for the brake.
abstract contract NativeBrake is INativeBrake {
    uint8 internal constant REASON_CODE_CHANGED = 1;
    uint8 internal constant REASON_DEFICIT = 2;
    uint8 internal constant REASON_IDS_EXHAUSTED = 4;

    /// @dev sha256 of the brake document the deployer ships with this
    ///      instance (zero = not pinned). Immutable; costs code bytes only.
    bytes32 private immutable _brakeDocSha256;

    constructor(bytes32 brakeDocSha256) {
        _brakeDocSha256 = brakeDocSha256;
    }

    /// @dev Bitmask of predicate reasons holding right now.
    function _brakeReasons() internal view virtual returns (uint8);

    /// @dev Latch height stored in the inheriting contract's packed word.
    function _brakeSince() internal view virtual returns (uint64);

    function _setBrakeSince(uint64 height) internal virtual;

    /// @dev Relative location of the brake document inside the template.
    function _brakeUri() internal pure virtual returns (string memory);

    /// @dev Call at the top of every new-entry path.
    function _requireEntryOpen() internal view {
        uint64 since = _brakeSince();
        uint8 reasons = _brakeReasons();
        if (since != 0 || reasons != 0) revert EntryBraked(reasons, since);
    }

    function brakeState() external view returns (uint8 state, address guardian, uint64 since) {
        since = _brakeSince();
        state = (since != 0 || _brakeReasons() != 0) ? 1 : 0;
        guardian = address(0);
    }

    function brakeSpec() external view returns (string memory uri, bytes32 docSha256) {
        return (_brakeUri(), _brakeDocSha256);
    }

    function brakePredicate() external view returns (uint8 reasons) {
        return _brakeReasons();
    }

    function tripBrake() external {
        uint64 since = _brakeSince();
        if (since != 0) revert BrakeAlreadyLatched(since);
        uint8 reasons = _brakeReasons();
        if (reasons == 0) revert BrakePredicateFalse();
        // block.number is never zero on a live chain; guard anyway so a latch
        // can never be stored as "not latched".
        uint64 height = block.number == 0 ? 1 : uint64(block.number);
        _setBrakeSince(height);
        emit BrakeLatched(height, reasons);
    }
}
