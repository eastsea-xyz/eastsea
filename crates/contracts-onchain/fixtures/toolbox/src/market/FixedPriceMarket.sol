// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC721} from "openzeppelin/token/ERC721/IERC721.sol";
import {IERC165} from "openzeppelin/utils/introspection/IERC165.sol";
import {IERC2981} from "openzeppelin/interfaces/IERC2981.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";

/// @title 예제 3 — 고정가 NFT 마켓플레이스 (에스크로 + 로열티 + pull payments)
/// @notice 판매자는 NFT를 마켓에 맡기고(list) 고정가를 붙인다. 구매자가
///         정확한 가격을 내면 NFT를 받고, 대금은 즉시 push되지 않고
///         판매자·로열티 수신자의 크레딧으로 적립된다. 인출은 각자 pull.
/// @dev
///  F-01 (예치 재진입) 대응 — 이 컨트랙트의 존재 이유:
///   - 대금은 절대 즉시 전송하지 않는다 (pull payments). 구매 시 크레딧
///     슬롯만 옮긴다.
///   - buy: listing 삭제(=정산 확정)를 NFT 전달보다 먼저.
///   - nonReentrant: buy(구매자 ERC-721 콜백 표면)와 withdraw(판매자
///     receive() 표면) 모두에.
///   - 실제 PoC 테스트가 두 경로를 다 검증한다.
///
///  로열티 (EIP-2981):
///   - NFT가 IERC2981을 지원하면 royaltyInfo로 판매가를 분할한다.
///   - 미지원이면 전액 판매자. 지원 여부는 supportsInterface로 —
///     정적 코드가 없는 주소(F-03 계열)는 지원하지 않는 것으로 간주.
///
///  F-04 (무한 view): 리스팅 목록 순회 뷰가 없다. 개별 listingOf만.
///  brake: list/buy(신규 진입)만 막는다. cancel/withdraw는 항상 열린다.
///
///  상태 비용: 리스팅 1건 = Listing struct 4슬롯. 크레딧은 주소당 1슬롯
///  (첫 정산 때만).
contract FixedPriceMarket is SimpleBrake, ReentrancyGuard {
    error NotOwner();
    error NotApproved();
    error NotSeller(uint256 listingId, address caller);
    error UnknownListing(uint256 listingId);
    error WrongPrice(uint256 listingId, uint256 expected, uint256 sent);
    error AlreadySold(uint256 listingId);
    error ZeroPrice();
    error ZeroCredit();
    error TransferFailed();

    event Listed(
        uint256 indexed listingId, address indexed seller, address indexed token, uint256 tokenId, uint256 price
    );
    event Sold(uint256 indexed listingId, address indexed buyer, uint256 price, uint256 royalty);
    event Cancelled(uint256 indexed listingId);
    event Withdrawn(address indexed to, uint256 amount);

    struct Listing {
        address seller; // 20B
        address token; // 20B (seller와 합쳐 40B — 별도 슬롯)
        uint256 tokenId; // 슬롯
        uint256 price; // 슬롯
    }

    uint256 private _nextListingId = 1;
    mapping(uint256 => Listing) private _listings; // listingId => Listing (0 = 없음)
    /// 수신자 => 인출 가능 크레딧 (로열티 포함). 불변식: sum == balance.
    mapping(address => uint256) public credits;

    constructor(address brakeGuardian_) SimpleBrake(brakeGuardian_) {}

    // ---- 마켓은 ERC-721을 받을 수 있어야 한다 (에스크로) ----

    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return this.onERC721Received.selector;
    }

    // ---- 판매 등록 ----

    /// @notice 판매자가 NFT를 마켓에 에스크로한다. 사전에
    ///         setApprovalForAll(마켓, true) 또는 approve(마켓, tokenId) 필요.
    function list(IERC721 token, uint256 tokenId, uint256 price) external whenEntryOpen returns (uint256 listingId) {
        if (price == 0) revert ZeroPrice();
        if (token.ownerOf(tokenId) != msg.sender) revert NotOwner();
        if (!(token.getApproved(tokenId) == address(this) || token.isApprovedForAll(msg.sender, address(this)))) {
            revert NotApproved();
        }

        listingId = _nextListingId++;
        _listings[listingId] = Listing(msg.sender, address(token), tokenId, price);
        token.transferFrom(msg.sender, address(this), tokenId); // 에스크로 (external call 마지막)
        emit Listed(listingId, msg.sender, address(token), tokenId, price);
    }

    // ---- 구매 ----

    /// @notice 정확한 가격으로 구매. NFT는 즉시, 대금은 크레딧으로.
    function buy(uint256 listingId) external payable nonReentrant whenEntryOpen {
        Listing storage l = _listings[listingId];
        if (l.seller == address(0)) revert UnknownListing(listingId);
        if (msg.value != l.price) revert WrongPrice(listingId, l.price, msg.value);

        // delete로 스토리지가 0화되기 전에 필요한 값을 전부 지역 복사한다
        // (storage 포인터를 delete 뒤에 읽는 건 0이 나온다).
        address seller_ = l.seller;
        IERC721 token_ = IERC721(l.token);
        uint256 tokenId_ = l.tokenId;
        uint256 price_ = l.price;

        // 로열티 분할 먼저 계산 (view 호출)
        (uint256 sellerAmount, uint256 royalty, address royaltyReceiver) = _split(l);

        // effects: 정산을 확정한다 — NFT 전달보다 먼저.
        delete _listings[listingId];
        credits[seller_] += sellerAmount;
        if (royalty > 0) credits[royaltyReceiver] += royalty;

        // interactions: NFT 전달 (구매자 콜백 — nonReentrant가 재진입 차단)
        token_.safeTransferFrom(address(this), msg.sender, tokenId_);
        emit Sold(listingId, msg.sender, price_, royalty);
    }

    // ---- 취소 ----

    /// @notice 판매 철회. brake 걸려도 열려 있다 (자구 경로).
    function cancel(uint256 listingId) external nonReentrant {
        Listing storage l = _listings[listingId];
        if (l.seller != msg.sender) revert NotSeller(listingId, msg.sender);

        IERC721 token = IERC721(l.token);
        uint256 tokenId = l.tokenId;
        delete _listings[listingId];
        token.safeTransferFrom(address(this), msg.sender, tokenId);
        emit Cancelled(listingId);
    }

    // ---- 정산 ----

    /// @notice 크레딧 인출 (판매대금 + 로열티). brake 걸려도 열려 있다.
    function withdraw() external nonReentrant {
        uint256 amount = credits[msg.sender];
        if (amount == 0) revert ZeroCredit();
        credits[msg.sender] = 0; // effects 먼저
        (bool ok,) = msg.sender.call{value: amount}("");
        if (!ok) revert TransferFailed();
        emit Withdrawn(msg.sender, amount);
    }

    // ---- 뷰 ----

    function listingOf(uint256 listingId)
        external
        view
        returns (address seller, address token, uint256 tokenId, uint256 price, bool active)
    {
        Listing storage l = _listings[listingId];
        return (l.seller, l.token, l.tokenId, l.price, l.seller != address(0));
    }

    /// @dev EIP-2981 지원 여부 + 금액 분할. 미지원/무코드(F-03 계열)는 전액 판매자.
    function _split(Listing storage l)
        private
        view
        returns (uint256 sellerAmount, uint256 royalty, address royaltyReceiver)
    {
        royalty = 0;
        royaltyReceiver = address(0);
        if (l.token.code.length > 0) {
            try IERC165(l.token).supportsInterface(type(IERC2981).interfaceId) returns (bool ok) {
                if (ok) {
                    (royaltyReceiver, royalty) = IERC2981(l.token).royaltyInfo(l.tokenId, l.price);
                }
            } catch {}
        }
        // An unset recipient cannot pull its credit. Treat malformed optional
        // royalty metadata as no royalty so sale proceeds remain withdrawable.
        if (royaltyReceiver == address(0)) royalty = 0;
        if (royalty > l.price) royalty = l.price; // 악의적 과다 로열티 클램프
        sellerAmount = l.price - royalty;
    }
}
