// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.19;

import {ReleaseLog} from "../src/ReleaseLog.sol";

contract ReleaseLogTest {
    function testAppendOnlyAndOpenPublication() public {
        ReleaseLog log = new ReleaseLog();
        bytes memory manifest = hex"414554484552";
        bytes memory sigs = hex"010203";
        uint256 first = log.publish(manifest, bytes32(uint256(5)), sigs, false);
        uint256 second = log.publish(hex"01", bytes32(uint256(6)), hex"04", true);
        require(first == 0 && second == 1 && log.count() == 2, "indices");
        (bytes32 m, bytes32 archive, bytes32 signatures, uint64 blockNumber, uint64 at, bool emergency) = log.entries(0);
        require(m == sha256(manifest), "manifest hash");
        require(archive == bytes32(uint256(5)), "archive hash");
        require(signatures == sha256(sigs), "signatures hash");
        require(blockNumber == block.number && at == block.timestamp && !emergency, "publication metadata");
        (,,,,, emergency) = log.entries(1);
        require(emergency, "emergency recorded");
    }

    function testEmptyPayloadCannotBePublished() public {
        ReleaseLog log = new ReleaseLog();
        (bool ok,) = address(log).call(abi.encodeCall(log.publish, (hex"", bytes32(0), hex"01", false)));
        require(!ok, "empty manifest accepted");
        (ok,) = address(log).call(abi.encodeCall(log.publish, (hex"01", bytes32(0), hex"", false)));
        require(!ok, "empty signatures accepted");
        require(log.count() == 0, "invalid entry stored");
    }

    function testOversizedPayloadCannotBePublished() public {
        ReleaseLog log = new ReleaseLog();
        (bool ok,) = address(log).call(abi.encodeCall(log.publish,
            (new bytes(log.MAX_MANIFEST_BYTES() + 1), bytes32(0), hex"01", false)));
        require(!ok, "oversized manifest accepted");
        (ok,) = address(log).call(abi.encodeCall(log.publish,
            (hex"01", bytes32(0), new bytes(log.MAX_SIGNATURE_BYTES() + 1), false)));
        require(!ok, "oversized signatures accepted");
        require(log.count() == 0, "invalid entry stored");
    }
}
