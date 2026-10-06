// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Editions1155} from "../src/nft/Editions1155.sol";

interface EditionsVm {
    function expectRevert(bytes4) external;
}

/// Local snapshot regression: narrowing must never alter the advertised sale.
contract EditionsBoundsTest {
    EditionsVm constant vm = EditionsVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testWalletCapAboveUint32IsRejected() public {
        Editions1155 editions = new Editions1155(address(this));
        vm.expectRevert(Editions1155.BadEditionParams.selector);
        editions.createEdition("wrapped cap", uint256(1) << 32, uint256(1) << 32, 1, 0);
    }

    function testPriceAboveUint128IsRejected() public {
        Editions1155 editions = new Editions1155(address(this));
        vm.expectRevert(Editions1155.BadEditionParams.selector);
        editions.createEdition("wrapped price", 1, 1, uint256(1) << 128, 0);
    }

    function testExactPackedFieldBoundsRoundTrip() public {
        Editions1155 editions = new Editions1155(address(this));
        uint256 id = editions.createEdition("bounds", type(uint48).max, type(uint32).max, type(uint128).max, 1000);
        (,, uint256 cap,, uint256 walletCap, uint256 price,) = editions.editionOf(id);
        require(cap == type(uint48).max, "cap narrowed");
        require(walletCap == type(uint32).max, "wallet cap narrowed");
        require(price == type(uint128).max, "price narrowed");
    }
}
