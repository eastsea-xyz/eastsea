// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SignatureChecker} from "openzeppelin/utils/cryptography/SignatureChecker.sol";

/// @dev Exposes OpenZeppelin's SignatureChecker, the check Permit2-style
/// verifiers run: ecrecover for code-less signers, ERC-1271 for accounts
/// with code (an EIP-7702-delegated EastSea account).
contract SignatureCheckerProbe {
    function isValidSignatureNow(address signer, bytes32 hash, bytes calldata signature) external view returns (bool) {
        return SignatureChecker.isValidSignatureNow(signer, hash, signature);
    }
}
