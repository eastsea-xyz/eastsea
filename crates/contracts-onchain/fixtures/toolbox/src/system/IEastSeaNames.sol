// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice 배포된 EastSeaNames 컨트랙트를 호출할 때 쓰는 최소 인터페이스.
///         전체 구현은 src/system/EastSeaNames.sol (벤더) 또는 체인의 실제 배포본.
/// @dev 슬롯 절약을 위해 예제 컨트랙트는 이 인터페이스만 import한다.
interface IEastSeaNames {
    function commit(bytes32 commitment) external payable;
    function register(string calldata name, address owner, bytes32 salt, address relayer) external payable;
    function clear(bytes32 commitment) external;
    function renew(string calldata name) external payable;
    function transferPropose(string calldata name, address to) external;
    function transferAccept(string calldata name) external;
    function createSubdomain(string calldata name, address a) external;
    function deleteSubdomain(string calldata name) external;
    function setAddr(string calldata name, address a) external;
    function setText(string calldata name, string calldata key, string calldata value) external;
    function setReverse(string calldata name) external;

    function isValidName(string calldata name) external pure returns (bool);
    function isValidHostname(string calldata name) external pure returns (bool);
    function feeFor(string calldata name) external pure returns (uint256);
    function nodeFor(string calldata name) external pure returns (bytes32);
    function ownerOf(bytes32 node) external view returns (address);
    function pendingOwnerOf(bytes32 node) external view returns (address);
    function expiresOf(bytes32 node) external view returns (uint64);
    function addrOf(bytes32 node) external view returns (address);
    function textOf(bytes32 node, string calldata key) external view returns (string memory);
    function reverseOf(address account) external view returns (string memory);
}
