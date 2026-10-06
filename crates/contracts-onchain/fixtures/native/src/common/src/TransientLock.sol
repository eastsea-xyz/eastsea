// SPDX-License-Identifier: MIT
pragma solidity ^0.8.31;

/// @title Transaction-scoped reentrancy lock
/// @dev Uses transient storage (EIP-1153, live in EastSea's Osaka executor; see
///      proof/probes H01). The lock clears before the final state diff, so it
///      never occupies a paid storage slot (PRIMITIVES / streams design: "no
///      persistent locked flag is silently assumed free").
abstract contract TransientLock {
    error Reentrancy();

    bool private transient _locked;

    modifier lock() {
        if (_locked) revert Reentrancy();
        _locked = true;
        _;
        _locked = false;
    }
}
