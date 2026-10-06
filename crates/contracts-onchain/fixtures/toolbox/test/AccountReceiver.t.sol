// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC721} from "openzeppelin/token/ERC721/ERC721.sol";
import {ERC1155} from "openzeppelin/token/ERC1155/ERC1155.sol";

interface ReceiverVm {
    function etch(address, bytes calldata) external;
    function readLine(string calldata) external returns (string memory);
    function parseBytes(string calldata) external pure returns (bytes memory);
}

/// The slice of EastSeaAccount these tests call.
interface IEastSeaAccountReceiver {
    function supportsInterface(bytes4) external view returns (bool);
}

contract ReceiverNft is ERC721 {
    constructor() ERC721("Receiver Nft", "RNFT") {}

    function mint(address to, uint256 id) external {
        _mint(to, id);
    }
}

contract ReceiverTokens is ERC1155 {
    constructor() ERC1155("") {}

    /// ERC1155._mint runs the receiver check, but the minter here (the test)
    /// is itself a contract without the hooks — mint through _update, the
    /// no-acceptance-check path the standard leaves to mints.
    function mint(address to, uint256 id, uint256 amount) external {
        uint256[] memory ids = new uint256[](1);
        uint256[] memory amounts = new uint256[](1);
        ids[0] = id;
        amounts[0] = amount;
        _update(address(0), to, ids, amounts);
    }
}

/// B2 with real OpenZeppelin tokens: ERC-721 `safeTransferFrom` and ERC-1155
/// single and batch safe transfers deliver to a delegated account through its
/// receiver hooks. The account address runs the exact runtime code a new
/// genesis predeploys at 0x…7702 (crates/execution/src/aether_account_v2.bin.hex),
/// which is what an EIP-7702 delegation executes; `vm.etch` stands in for the
/// delegation itself.
contract AccountReceiverTest {
    ReceiverVm constant vm = ReceiverVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    string constant ACCOUNT_CODE = "../../../execution/src/aether_account_v2.bin.hex";
    /// Any account address: the hooks do not depend on which key owns it.
    address constant ACCOUNT = 0x148f24F9be765d8AF6CB9480A39936499D267cc4;

    function account() private returns (IEastSeaAccountReceiver a) {
        vm.etch(ACCOUNT, vm.parseBytes(string.concat("0x", vm.readLine(ACCOUNT_CODE))));
        a = IEastSeaAccountReceiver(ACCOUNT);
    }

    function testErc721SafeTransferFromDeliversToTheDelegatedAccount() public {
        ReceiverNft nft = new ReceiverNft();
        nft.mint(address(this), 7);
        IEastSeaAccountReceiver a = account();
        nft.safeTransferFrom(address(this), address(a), 7);
        require(nft.ownerOf(7) == address(a), "the account owns the token");
        require(a.supportsInterface(0x150b7a02), "ERC-165 advertises the ERC-721 hook");
    }

    function testErc1155SafeTransferDeliversToTheDelegatedAccount() public {
        ReceiverTokens tokens = new ReceiverTokens();
        tokens.mint(address(this), 3, 10);
        IEastSeaAccountReceiver a = account();
        tokens.safeTransferFrom(address(this), address(a), 3, 10, "");
        require(tokens.balanceOf(address(a), 3) == 10, "single send");
        require(a.supportsInterface(0x4e2312e0), "ERC-165 advertises the ERC-1155 hooks");
    }

    function testErc1155BatchSafeTransferDeliversToTheDelegatedAccount() public {
        ReceiverTokens tokens = new ReceiverTokens();
        tokens.mint(address(this), 1, 10);
        tokens.mint(address(this), 2, 20);
        IEastSeaAccountReceiver a = account();
        uint256[] memory ids = new uint256[](2);
        ids[0] = 1;
        ids[1] = 2;
        uint256[] memory amounts = new uint256[](2);
        amounts[0] = 6;
        amounts[1] = 16;
        tokens.safeBatchTransferFrom(address(this), address(a), ids, amounts, "");
        require(tokens.balanceOf(address(a), 1) == 6, "batch first");
        require(tokens.balanceOf(address(a), 2) == 16, "batch second");
        require(a.supportsInterface(0x4e2312e0), "ERC-165 advertises the batch interface");
    }
}
