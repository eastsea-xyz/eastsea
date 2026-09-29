// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

/// Append-only transparency log for Mac releases. Anyone can publish; clients
/// decide trust by checking the builder signatures against their pinned keys.
contract ReleaseLog {
    struct Entry {
        bytes32 manifestHash;
        bytes32 archiveSha256;
        bytes32 signaturesHash;
        uint64 publishedBlock;
        uint64 publishedAt;
        bool emergency;
    }

    Entry[] public entries;

    event Published(
        uint256 indexed index,
        bytes32 indexed manifestHash,
        bytes manifest,
        bytes builderSigs
    );

    error EmptyManifest();
    error EmptySignatures();
    error TooLarge();

    uint256 public constant MAX_MANIFEST_BYTES = 16_384;
    uint256 public constant MAX_SIGNATURE_BYTES = 4_096;

    function count() external view returns (uint256) {
        return entries.length;
    }

    /// The full payload is logged as an event; its hashes and publication
    /// metadata remain in storage for light-client Merkle proof verification.
    function publish(bytes calldata manifest, bytes32 archiveSha256, bytes calldata builderSigs, bool emergency) external returns (uint256 index) {
        if (manifest.length == 0) revert EmptyManifest();
        if (builderSigs.length == 0) revert EmptySignatures();
        if (manifest.length > MAX_MANIFEST_BYTES || builderSigs.length > MAX_SIGNATURE_BYTES) revert TooLarge();
        bytes32 manifestHash = sha256(manifest);
        bytes32 signaturesHash = sha256(builderSigs);
        index = entries.length;
        entries.push(Entry(manifestHash, archiveSha256, signaturesHash, uint64(block.number), uint64(block.timestamp), emergency));
        emit Published(index, manifestHash, manifest, builderSigs);
    }
}
