// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SignatureChecker} from "openzeppelin/utils/cryptography/SignatureChecker.sol";

/// @notice Test instrument: a state-changing ERC-1271 consumer. Anyone may
/// relay a signer's approval of `payload`; OpenZeppelin SignatureChecker
/// verifies it (ERC-1271 for a delegated EastSea account). The digest binds
/// this contract and chain; each (signer, payload) is accepted once and the
/// replay record is retained.
contract SignedIntentBook {
    mapping(address => mapping(bytes32 => bool)) public accepted;

    error BadSignature();
    error Replayed();

    event Accepted(address indexed signer, bytes32 indexed payload, address relayer);

    function digest(bytes32 payload) public view returns (bytes32) {
        return keccak256(abi.encode(address(this), block.chainid, payload));
    }

    function accept(address signer, bytes32 payload, bytes calldata signature) external {
        if (accepted[signer][payload]) revert Replayed();
        if (!SignatureChecker.isValidSignatureNow(signer, digest(payload), signature)) revert BadSignature();
        accepted[signer][payload] = true;
        emit Accepted(signer, payload, msg.sender);
    }
}
