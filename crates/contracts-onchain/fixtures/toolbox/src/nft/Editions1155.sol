// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC1155} from "openzeppelin/token/ERC1155/ERC1155.sol";
import {ERC2981} from "openzeppelin/token/common/ERC2981.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {Base64} from "openzeppelin/utils/Base64.sol";
import {Strings} from "openzeppelin/utils/Strings.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";

/// @title 예제 2-B — 유한 에디션 판매 ERC-1155
/// @notice 크리에이터가 에디션(한도·단가·지갑당 상한·로열티)을 만들고,
///         누구나 정확한 가격을 내고 1개씩 민트한다. 판매대금은 컨트랙트에
///         쌓이고 크리에이터가 인출한다 — 이 예제는 자금을 보관한다.
/// @dev
///  상태 설계 (유료 상태 관점):
///   - Edition struct를 2슬롯으로 pack: [creator(20B)|fee(2B)|maxPerWallet(4B)|cap(6B)] + [minted(8B)|price(16B)].
///   - 인출 가능액은 price*minted - withdrawn로 계산한다 — credited 매핑 없음.
///     불변식: sum(withdrawable) == address(this).balance (테스트가 fuzz로 검증).
///   - 지갑당 상한은 ERC1155 balanceOf로 검사 — 별도 매핑 슬롯 없음.
///
///  안전 설계:
///   - mint는 payable + nonReentrant (F-01: 수신자 콜백 onERC1155Received가
///     있으므로 상태(minted)를 먼저 확정한다).
///   - msg.value는 price와 정확히 같아야 한다 — 과다 납부 환불 경로를 만들지
///     않는다 (환불 = 재진입·오류 표면).
///   - brake: createEdition/mint(신규 진입)만 막는다. withdraw는 항상 열린다.
contract Editions1155 is ERC1155, ERC2981, SimpleBrake, ReentrancyGuard {
    using Strings for uint256;

    error NotEditionCreator(uint256 editionId, address caller);
    error UnknownEdition(uint256 editionId);
    error EditionSoldOut(uint256 editionId, uint256 cap);
    error WalletCapReached(uint256 editionId, uint256 maxPerWallet);
    error WrongPrice(uint256 editionId, uint256 expected, uint256 sent);
    error NothingToWithdraw(uint256 editionId);
    error BadEditionParams();
    error NameTooLong();
    error TransferFailed();

    event EditionCreated(
        uint256 indexed editionId,
        address indexed creator,
        string name,
        uint256 cap,
        uint256 maxPerWallet,
        uint256 price,
        uint16 feeBps
    );
    event Minted(uint256 indexed editionId, address indexed buyer, uint256 paid);
    event Withdrawn(uint256 indexed editionId, address indexed to, uint256 amount);

    uint16 public constant FEE_CAP_BPS = 1000; // 로열티 상한 10%
    uint256 public constant NAME_MAX = 64;

    struct Edition {
        address creator; // 20B ─ slot 1 시작
        uint16 feeBps; //  2B
        uint32 maxPerWallet; //  4B
        uint48 cap; //  6B ─ slot 1 종료 (32B)
        uint64 minted; //  8B ─ slot 2 시작
        uint128 price; // 16B ─ slot 2 종료
    }

    uint256 private _nextEditionId = 1;
    mapping(uint256 => Edition) private _editions;
    /// editionId => 누적 인출액. credited는 price*minted로 계산 가능.
    mapping(uint256 => uint256) private _withdrawn;
    mapping(uint256 => string) private _names; // 에디션 이름 (uri용, 별도 슬롯)

    constructor(address brakeGuardian_) ERC1155("") SimpleBrake(brakeGuardian_) {}

    // ---- 에디션 생성 ----

    function createEdition(string memory name_, uint256 cap, uint256 maxPerWallet, uint256 price, uint16 feeBps)
        external
        whenEntryOpen
        returns (uint256 editionId)
    {
        if (bytes(name_).length == 0 || bytes(name_).length > NAME_MAX) revert NameTooLong();
        if (cap == 0 || cap > type(uint48).max) revert BadEditionParams();
        if (maxPerWallet == 0 || maxPerWallet > cap) revert BadEditionParams();
        if (feeBps > FEE_CAP_BPS) revert BadEditionParams();

        editionId = _nextEditionId++;
        _editions[editionId] = Edition(msg.sender, feeBps, uint32(maxPerWallet), uint48(cap), 0, uint128(price));
        _names[editionId] = name_;
        emit EditionCreated(editionId, msg.sender, name_, cap, maxPerWallet, price, feeBps);
    }

    // ---- 구매 ----

    /// @notice 1장 구매. msg.value는 price와 정확히 같아야 한다.
    function mint(uint256 editionId) external payable nonReentrant whenEntryOpen {
        Edition storage e = _editions[editionId];
        if (e.creator == address(0)) revert UnknownEdition(editionId);
        if (msg.value != e.price) revert WrongPrice(editionId, e.price, msg.value);
        if (e.minted >= e.cap) revert EditionSoldOut(editionId, e.cap);
        if (balanceOf(msg.sender, editionId) >= e.maxPerWallet) {
            revert WalletCapReached(editionId, e.maxPerWallet);
        }

        e.minted += 1; // effects 먼저 (F-01)
        // OZ ERC1155._mint는 수신자가 컨트랙트면 onERC1155Received를 검증한다.
        _mint(msg.sender, editionId, 1, ""); // 수신자 콜백은 마지막
        emit Minted(editionId, msg.sender, e.price);
    }

    // ---- 정산 ----

    /// @notice 크리에이터 인출. brake 걸린 상태에서도 열려 있다 (진입만 막는다).
    function withdraw(uint256 editionId) external nonReentrant {
        Edition storage e = _editions[editionId];
        if (e.creator != msg.sender) revert NotEditionCreator(editionId, msg.sender);
        uint256 amount = withdrawable(editionId);
        if (amount == 0) revert NothingToWithdraw(editionId);
        _withdrawn[editionId] += amount; // effects 먼저
        (bool ok,) = msg.sender.call{value: amount}("");
        if (!ok) revert TransferFailed(); // revert 시 상태 전체 롤백 — 불변식 유지
        emit Withdrawn(editionId, msg.sender, amount);
    }

    /// @notice 인출 가능 잔액 = price*minted - 누적 인출.
    function withdrawable(uint256 editionId) public view returns (uint256) {
        Edition storage e = _editions[editionId];
        return uint256(e.price) * uint256(e.minted) - _withdrawn[editionId];
    }

    // ---- 뷰 ----

    function editionOf(uint256 editionId)
        external
        view
        returns (
            address creator,
            string memory name_,
            uint256 cap,
            uint256 minted,
            uint256 maxPerWallet,
            uint256 price,
            uint16 feeBps
        )
    {
        Edition storage e = _editions[editionId];
        if (e.creator == address(0)) revert UnknownEdition(editionId);
        return (e.creator, _names[editionId], e.cap, e.minted, e.maxPerWallet, e.price, e.feeBps);
    }

    /// @notice 온체인 메타데이터. 외부 게이트웨이 없음.
    function uri(uint256 editionId) public view override returns (string memory) {
        Edition storage e = _editions[editionId];
        if (e.creator == address(0)) revert UnknownEdition(editionId);
        string memory json = string.concat(
            '{"name":"',
            _names[editionId],
            '","description":"Finite edition on EastSea (example 2-B).",',
            '"properties":{"edition":',
            editionId.toString(),
            ',"cap":',
            uint256(e.cap).toString(),
            ',"minted":',
            uint256(e.minted).toString(),
            ',"price_wei":',
            uint256(e.price).toString(),
            ',"royalty_bps":',
            uint256(e.feeBps).toString(),
            "}}"
        );
        return string.concat("data:application/json;base64,", Base64.encode(bytes(json)));
    }

    /// @notice 에디션별 로열티 (EIP-2981).
    function royaltyInfo(uint256 editionId, uint256 salePrice)
        public
        view
        override
        returns (address receiver, uint256 royaltyAmount)
    {
        Edition storage e = _editions[editionId];
        if (e.creator == address(0)) revert UnknownEdition(editionId);
        return (e.creator, (salePrice * e.feeBps) / 10000);
    }

    function supportsInterface(bytes4 interfaceId) public view override(ERC1155, ERC2981) returns (bool) {
        return super.supportsInterface(interfaceId);
    }
}
