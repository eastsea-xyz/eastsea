// SPDX-License-Identifier: MIT
pragma solidity ^0.8.31;

/// @notice Minimal ERC-20 surface used by the native templates.
interface IERC20Min {
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

/// @title Exact-transfer token movement
/// @notice Native templates admit only assets whose transfers move exactly the
///         stated amount (PRIMITIVES "Inherited design rules"). Fee-on-transfer,
///         rebasing-down on transfer and no-op "true" tokens are rejected at
///         the movement that exposes them, so they can never create an
///         unbacked liability.
/// @dev Every movement is measured by balance deltas on both sides. A token
///      that reports success without moving funds, or moves a different
///      amount, reverts the whole call; callers rely on that full rollback
///      to keep their bookkeeping unchanged.
library ExactToken {
    error TokenHasNoCode(address token);
    error TokenCallFailed(address token);
    error NotExactTransfer(uint256 expected, uint256 observed);
    error NativeTransferFailed(address to, uint256 amount);
    error SelfTransfer();

    function _call(address token, bytes memory data) private {
        if (token.code.length == 0) revert TokenHasNoCode(token);
        (bool ok, bytes memory ret) = token.call(data);
        if (!ok) revert TokenCallFailed(token);
        if (ret.length != 0 && (ret.length != 32 || abi.decode(ret, (uint256)) != 1)) {
            revert TokenCallFailed(token);
        }
    }

    /// @dev Pull `amount` from `from`; the instance balance must rise by exactly `amount`.
    function pullExact(address token, address from, uint256 amount) internal {
        uint256 before = IERC20Min(token).balanceOf(address(this));
        _call(token, abi.encodeCall(IERC20Min.transferFrom, (from, address(this), amount)));
        uint256 afterBal = IERC20Min(token).balanceOf(address(this));
        if (afterBal < before || afterBal - before != amount) {
            revert NotExactTransfer(amount, afterBal < before ? 0 : afterBal - before);
        }
    }

    /// @dev Push `amount` to `to`; the instance balance must fall by exactly
    ///      `amount` and the recipient balance must rise by exactly `amount`.
    function pushExact(address token, address to, uint256 amount) internal {
        if (to == address(this)) revert SelfTransfer();
        uint256 selfBefore = IERC20Min(token).balanceOf(address(this));
        uint256 toBefore = IERC20Min(token).balanceOf(to);
        _call(token, abi.encodeCall(IERC20Min.transfer, (to, amount)));
        uint256 selfAfter = IERC20Min(token).balanceOf(address(this));
        uint256 toAfter = IERC20Min(token).balanceOf(to);
        if (selfAfter > selfBefore || selfBefore - selfAfter != amount) {
            revert NotExactTransfer(amount, selfAfter > selfBefore ? 0 : selfBefore - selfAfter);
        }
        if (toAfter < toBefore || toAfter - toBefore != amount) {
            revert NotExactTransfer(amount, toAfter < toBefore ? 0 : toAfter - toBefore);
        }
    }

    /// @dev Native DBLN payout. Forwards all gas: the recipient is a fixed
    ///      party, the caller is protected by the template's reentrancy lock,
    ///      and a reverting recipient only blocks its own payout.
    function pushNative(address to, uint256 amount) internal {
        if (to == address(this)) revert SelfTransfer();
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert NativeTransferFailed(to, amount);
    }
}
