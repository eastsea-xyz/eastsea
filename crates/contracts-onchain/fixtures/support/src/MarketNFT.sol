// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC721} from "openzeppelin/token/ERC721/ERC721.sol";

/// @dev Harness NFT with independently configurable ERC-2981 failure modes.
contract MarketNFT is ERC721 {
    uint256 public nextId = 1;
    uint8 public royaltyMode;
    address public royaltyReceiver;
    uint256 public royaltyAmount;
    bool public failTransfers;

    constructor() ERC721("Market Test NFT", "MTN") {}
    function mint(address to) external returns (uint256 id) {
        id = nextId++;
        _mint(to, id);
    }
    function configureRoyalty(uint8 mode, address receiver, uint256 amount) external {
        require(mode <= 3, "mode");
        royaltyMode = mode; royaltyReceiver = receiver; royaltyAmount = amount;
    }
    function setFailTransfers(bool fail) external { failTransfers = fail; }
    function supportsInterface(bytes4 interfaceId) public view override returns (bool) {
        require(royaltyMode != 2, "interface failure");
        if (interfaceId == 0x2a55205a) return royaltyMode == 1 || royaltyMode == 3;
        return super.supportsInterface(interfaceId);
    }
    function royaltyInfo(uint256, uint256) external view returns (address receiver, uint256 amount) {
        require(royaltyMode != 3, "royalty failure");
        return (royaltyReceiver, royaltyAmount);
    }
    function transferFrom(address from, address to, uint256 id) public override {
        require(!failTransfers, "transfer failure");
        super.transferFrom(from, to, id);
    }
}
