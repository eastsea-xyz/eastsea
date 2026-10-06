// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;
import {ERC721} from "openzeppelin/token/ERC721/ERC721.sol";
import {FixedPriceMarket} from "../src/market/FixedPriceMarket.sol";

interface MarketVm { function deal(address account, uint256 amount) external; }

/// A misconfigured collection advertises royalties to the unset sentinel.
contract ZeroReceiverRoyaltyNFT is ERC721 {
    constructor() ERC721("Zero Receiver", "ZERO") { _mint(msg.sender, 1); }
    function supportsInterface(bytes4 id) public view override returns (bool) {
        return id == 0x2a55205a || super.supportsInterface(id);
    }
    function royaltyInfo(uint256, uint256 price) external pure returns (address, uint256) {
        return (address(0), price / 2);
    }
}

contract MarketRoyaltyTest {
    MarketVm constant vm = MarketVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return this.onERC721Received.selector;
    }
    function testInvalidRoyaltyReceiverCannotStrandSellerProceeds() public {
        FixedPriceMarket market = new FixedPriceMarket(address(this));
        ZeroReceiverRoyaltyNFT nft = new ZeroReceiverRoyaltyNFT();
        nft.approve(address(market), 1);
        market.list(nft, 1, 1 ether);
        vm.deal(address(this), 1 ether);
        market.buy{value: 1 ether}(1);
        require(market.credits(address(this)) == 1 ether, "seller proceeds stranded");
        require(market.credits(address(0)) == 0, "unclaimable zero-address credit");
        market.withdraw();
        require(address(market).balance == 0, "market remainder");
    }
    receive() external payable {}
}
