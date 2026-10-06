// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IEastSeaLocalBrake} from "./IEastSeaLocalBrake.sol";

/// @notice Reference base for IEastSeaLocalBrake. An implementation supplies only
/// `_brakeCondition()` (deterministic, nonreverting) and `_brakeUri()`.
/// Put `whenEntryOpen` on entries only; never on exits.
abstract contract LocalBrake is IEastSeaLocalBrake {
    uint8 private _state;
    uint64 private _since;
    bytes32 private immutable _specSha256;

    error EntryBraked(bytes32 reason);

    /// Tag reported for an entry refused after the latch.
    bytes32 internal constant LATCHED = "LATCHED";

    constructor(bytes32 specSha256) {
        _specSha256 = specSha256;
    }

    /// @dev Must not revert and must depend only on on-chain state.
    function _brakeCondition() internal view virtual returns (bool holds, bytes32 reason);

    function _brakeUri() internal pure virtual returns (string memory);

    modifier whenEntryOpen() {
        if (_state != 0) revert EntryBraked(LATCHED);
        (bool holds, bytes32 reason) = _brakeCondition();
        if (holds) revert EntryBraked(reason);
        _;
    }

    function brakeState() external view returns (uint8 state, address guardian, uint64 since) {
        return (_state, address(0), _since);
    }

    function brakeSpec() external view returns (string memory uri, bytes32 docSha256) {
        return (_brakeUri(), _specSha256);
    }

    function brakePredicate() external view returns (bool holds, bytes32 reason) {
        return _brakeCondition();
    }

    function tripBrake() external returns (bool latched) {
        if (_state != 0) return true;
        (bool holds, bytes32 reason) = _brakeCondition();
        if (!holds) return false;
        _state = 1;
        _since = uint64(block.timestamp);
        emit BrakeLatched(reason, _since);
        return true;
    }
}
