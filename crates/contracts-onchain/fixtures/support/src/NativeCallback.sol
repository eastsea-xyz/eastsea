// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @dev Harness-only caller/receiver for testing payout callbacks and failures.
contract NativeCallback {
    address public target;
    bytes public data;
    bool public callbackAttempted;
    bool public innerSuccess;
    bytes public innerOutput;
    bool public rejectPayment;
    bool public nftCallback;
    bool public rejectNFT;
    bool private inCallback;

    function configure(address target_, bytes calldata data_) external {
        target = target_; data = data_;
        callbackAttempted = false; innerSuccess = false; delete innerOutput;
    }
    function setRejectPayment(bool reject) external { rejectPayment = reject; }
    function setNFTCallback(bool enabled) external { nftCallback = enabled; }
    function setRejectNFT(bool reject) external { rejectNFT = reject; }
    function execute(address target_, bytes calldata data_, uint256 value)
        external payable returns (bool success, bytes memory result)
    {
        (success, result) = target_.call{value: value}(data_);
    }
    function executeOrRevert(address target_, bytes calldata data_, uint256 value)
        external payable returns (bytes memory result)
    {
        bool success;
        (success, result) = target_.call{value: value}(data_);
        if (!success) {
            assembly { revert(add(result, 32), mload(result)) }
        }
    }
    receive() external payable {
        require(!rejectPayment, "reject payment");
        _attemptCallback();
    }
    function _attemptCallback() private {
        if (target != address(0) && !inCallback) {
            inCallback = true; callbackAttempted = true;
            (innerSuccess, innerOutput) = target.call(data);
            inCallback = false;
        }
    }
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4) {
        require(!rejectNFT, "reject NFT");
        if (nftCallback) _attemptCallback();
        return this.onERC721Received.selector;
    }
    function onERC1155Received(address, address, uint256, uint256, bytes calldata) external pure returns (bytes4) {
        return this.onERC1155Received.selector;
    }
    function onERC1155BatchReceived(address, address, uint256[] calldata, uint256[] calldata, bytes calldata)
        external pure returns (bytes4)
    { return this.onERC1155BatchReceived.selector; }
}
