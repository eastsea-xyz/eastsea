// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {LocalBrake} from "./LocalBrake.sol";

interface IBalanceToken {
    function balanceOf(address who) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

/// @notice Test instrument: the reference implementation of a local
/// deterministic entry brake over a single-token deposit vault.
///
/// Brake specification (this file's SHA-256 is pinned at deployment and
/// returned by `brakeSpec()`):
/// - Predicate, evaluated from chain state only:
///   `DEFICIT`    token.balanceOf(vault) < liabilities (e.g. an issuer seizure);
///   `CODE`       the token's code hash differs from the hash pinned at deployment;
///   `UNREADABLE` balanceOf fails or returns malformed data.
/// - Latch: anyone calls `tripBrake()`; it never reverts, latches iff the
///   predicate holds, and the latch is permanent. Guardian is address(0).
/// - Entry: `deposit` re-evaluates the predicate and refuses while it holds,
///   and always after the latch.
/// - Exit: `withdraw` never checks the brake. It pays a depositor's own
///   recorded balance while the vault can transfer it; under a deficit the
///   last withdrawers can fail. Exit open is not a solvency guarantee.
/// - No owner, admin, rescue, upgrade or fallback function exists.
contract BrakeReferenceVault is LocalBrake {
    IBalanceToken public immutable token;
    bytes32 public immutable tokenCodeHash;
    uint256 public liabilities;
    mapping(address => uint256) public balanceOf;

    error ZeroAmount();
    error InexactTransfer(uint256 received, uint256 expected);
    error Insufficient(uint256 balance, uint256 requested);
    error TransferFailed();

    event Deposited(address indexed who, uint256 amount);
    event Withdrawn(address indexed who, uint256 amount);

    constructor(IBalanceToken token_, bytes32 specSha256) LocalBrake(specSha256) {
        token = token_;
        tokenCodeHash = address(token_).codehash;
    }

    function _brakeUri() internal pure override returns (string memory) {
        return "eastsea-contracts-onchain:fixtures/support/src/brake/BrakeReferenceVault.sol";
    }

    function _brakeCondition() internal view override returns (bool, bytes32) {
        if (address(token).codehash != tokenCodeHash) return (true, "CODE");
        (bool ok, bytes memory out) =
            address(token).staticcall(abi.encodeWithSelector(IBalanceToken.balanceOf.selector, address(this)));
        if (!ok || out.length != 32) return (true, "UNREADABLE");
        if (abi.decode(out, (uint256)) < liabilities) return (true, "DEFICIT");
        return (false, bytes32(0));
    }

    /// Entry: exact-transfer deposit.
    function deposit(uint256 amount) external whenEntryOpen {
        if (amount == 0) revert ZeroAmount();
        uint256 before = token.balanceOf(address(this));
        if (!token.transferFrom(msg.sender, address(this), amount)) revert TransferFailed();
        uint256 received = token.balanceOf(address(this)) - before;
        if (received != amount) revert InexactTransfer(received, amount);
        balanceOf[msg.sender] += amount;
        liabilities += amount;
        emit Deposited(msg.sender, amount);
    }

    /// Exit: never braked.
    function withdraw(uint256 amount) external {
        uint256 held = balanceOf[msg.sender];
        if (amount == 0) revert ZeroAmount();
        if (amount > held) revert Insufficient(held, amount);
        balanceOf[msg.sender] = held - amount;
        liabilities -= amount;
        if (!token.transfer(msg.sender, amount)) revert TransferFailed();
        emit Withdrawn(msg.sender, amount);
    }
}
