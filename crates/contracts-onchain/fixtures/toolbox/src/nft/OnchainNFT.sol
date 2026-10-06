// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC721} from "openzeppelin/token/ERC721/ERC721.sol";
import {ERC2981} from "openzeppelin/token/common/ERC2981.sol";
import {Base64} from "openzeppelin/utils/Base64.sol";
import {Strings} from "openzeppelin/utils/Strings.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";

/// @title 예제 2-A — 온체인 메타데이터 ERC-721
/// @notice 메타데이터(이름·특성·이미지)를 전부 체인 안에서 조립해 data URI로
///         돌려준다. 외부 IPFS/HTTP 게이트웨이에 의존하지 않는다 — 이미지가
///         죽으면 NFT도 죽는 일이 이 컨트랙트에는 없다.
/// @dev
///  상태 설계 (유료 상태 관점):
///   - 특성 4개를 한 슬롯에 pack (매핑 tokenId => uint256). 특성당 슬롯을
///     파면 tokenId마다 +300 units다.
///   - mint 1건 = 소유권 슬롯(ERC721 _owners) + 특성 슬롯 = 새 슬롯 2개.
///   - burn은 특성 슬롯을 지운다 (cleared slot — 새 슬롯으로 안 친다).
///   - tokenURI/jsonOf/svgOf는 view — 상태 비용 0.
///
///  권한 설계:
///   - 발행(mint)은 크리에이터만. 공개 판매는 예제 2-B(Editions1155)가
///     담당한다 — 자금을 보관하는 쪽과 아닌 쪽을 나눠 단순하게 유지.
///   - brake: mint(신규 진입)만 막는다. transfer/burn은 항상 열려 있다.
contract OnchainNFT is ERC721, ERC2981, SimpleBrake {
    using Strings for uint256;

    error NotCreator(address caller);
    error NotOwnerOf(uint256 tokenId, address caller);
    error MintedOut(uint256 next, uint256 max);
    error BadTrait();

    event Minted(uint256 indexed tokenId, address indexed to, uint256 packed);
    event Burned(uint256 indexed tokenId, address indexed by);

    address public immutable creator;
    uint256 public immutable maxSupply;

    uint256 private _nextTokenId = 1; // tokenId는 1부터 (0은 미발행 구분자)
    mapping(uint256 => uint256) private _traits;

    uint8 public constant TRAIT_MAX = 8; // 특성값 0..7

    constructor(
        string memory name_,
        string memory symbol_,
        uint256 maxSupply_,
        uint96 royaltyFeeNumerator_, // 판매가 대비 만분율, 예: 500 = 5%
        address brakeGuardian_
    ) ERC721(name_, symbol_) SimpleBrake(brakeGuardian_) {
        if (maxSupply_ == 0) revert BadTrait();
        creator = msg.sender;
        maxSupply = maxSupply_;
        _setDefaultRoyalty(msg.sender, royaltyFeeNumerator_);
    }

    modifier onlyCreator() {
        if (msg.sender != creator) revert NotCreator(msg.sender);
        _;
    }

    // ---- 발행 ----

    /// @notice 크리에이터 발행. 특성값은 각각 0..7.
    function mint(address to, uint8 color, uint8 shape, uint8 pattern, uint8 halo)
        external
        onlyCreator
        whenEntryOpen
        returns (uint256 tokenId)
    {
        tokenId = _nextTokenId;
        if (tokenId > maxSupply) revert MintedOut(tokenId, maxSupply);
        if (color >= TRAIT_MAX || shape >= TRAIT_MAX || pattern >= TRAIT_MAX || halo >= TRAIT_MAX) {
            revert BadTrait();
        }
        // pack: [0..7] color, [8..15] shape, [16..23] pattern, [24..31] halo,
        // [32..63] mintedAt. 상위 비트는 0.
        uint256 packed = uint256(color) | (uint256(shape) << 8) | (uint256(pattern) << 16) | (uint256(halo) << 24)
            | (block.timestamp << 32);
        _nextTokenId = tokenId + 1; // effects 먼저
        _traits[tokenId] = packed;
        _safeMint(to, tokenId); // 수신자 콜백은 마지막 (F-01: 상태 선행)
        emit Minted(tokenId, to, packed);
    }

    /// @notice 소유자 소각. 특성 슬롯을 반납한다.
    function burn(uint256 tokenId) external {
        if (_ownerOf(tokenId) != msg.sender) revert NotOwnerOf(tokenId, msg.sender);
        delete _traits[tokenId]; // cleared slot: 상태 비용 관점 반납
        _burn(tokenId);
        emit Burned(tokenId, msg.sender);
    }

    // ---- 특성 뷰 ----

    struct Traits {
        uint8 color;
        uint8 shape;
        uint8 pattern;
        uint8 halo;
        uint32 mintedAt;
    }

    function traitsOf(uint256 tokenId) public view returns (Traits memory t) {
        _requireOwned(tokenId); // 미발행/소각 토큰이면 여기서 revert
        uint256 p = _traits[tokenId];
        t.color = uint8(p);
        t.shape = uint8(p >> 8);
        t.pattern = uint8(p >> 16);
        t.halo = uint8(p >> 24);
        t.mintedAt = uint32(p >> 32);
    }

    // ---- 온체인 메타데이터 ----

    function tokenURI(uint256 tokenId) public view override returns (string memory) {
        return string.concat("data:application/json;base64,", Base64.encode(bytes(jsonOf(tokenId))));
    }

    /// @notice JSON 전문. image 필드에 SVG data URI가 박혀 있다.
    function jsonOf(uint256 tokenId) public view returns (string memory) {
        Traits memory t = traitsOf(tokenId);
        string memory name = string.concat(_baseName(), " #", tokenId.toString());
        return string.concat(
            '{"name":"',
            name,
            '","description":"Onchain example NFT - traits packed into one storage slot.",',
            '"image":"data:image/svg+xml;base64,',
            Base64.encode(bytes(svgOf(tokenId))),
            '","attributes":[',
            _attr("color", _colorName(t.color)),
            ",",
            _attr("shape", _shapeName(t.shape)),
            ",",
            _attr("pattern", _patternName(t.pattern)),
            ",",
            _attr("halo", _haloName(t.halo)),
            "]}"
        );
    }

    /// @notice 200x200 SVG. 배경=색, 도형=shape, 채움=pattern(불투명도), 테두리=halo.
    function svgOf(uint256 tokenId) public view returns (string memory) {
        Traits memory t = traitsOf(tokenId);
        string memory body = _shapeSvg(t.shape, _colorHex(t.pattern), _opacityOf(t.pattern));
        return string.concat(
            "<svg xmlns='http://www.w3.org/2000/svg' width='200' height='200'>",
            "<rect width='200' height='200' fill='",
            _colorHex(t.color),
            "'/>",
            body,
            "<circle cx='100' cy='100' r='88' fill='none' stroke='",
            _colorHex(t.halo),
            "' stroke-width='4'/>",
            "</svg>"
        );
    }

    function _baseName() internal view returns (string memory) {
        return name();
    }

    /// @dev pattern 0..7 -> fill-opacity 0.3 .. 1.0 (10단계 소수 문자열).
    function _opacityOf(uint8 pattern) private pure returns (string memory) {
        uint256 n = uint256(pattern) + 3; // 3..10
        if (n == 10) return "1.0";
        return string.concat("0.", Strings.toString(n));
    }

    function _attr(string memory k, string memory v) private pure returns (string memory) {
        return string.concat('{"trait_type":"', k, '","value":"', v, '"}');
    }

    // ---- 특성 이름 테이블 (storage 없는 pure 함수) ----

    function _colorName(uint8 v) private pure returns (string memory) {
        if (v == 0) return "teal";
        if (v == 1) return "coral";
        if (v == 2) return "indigo";
        if (v == 3) return "amber";
        if (v == 4) return "moss";
        if (v == 5) return "plum";
        if (v == 6) return "slate";
        return "cream";
    }

    function _colorHex(uint8 v) private pure returns (string memory) {
        if (v == 0) return "#2dd4bf";
        if (v == 1) return "#fb7185";
        if (v == 2) return "#818cf8";
        if (v == 3) return "#fbbf24";
        if (v == 4) return "#84cc16";
        if (v == 5) return "#c084fc";
        if (v == 6) return "#64748b";
        return "#fef3c7";
    }

    function _shapeName(uint8 v) private pure returns (string memory) {
        if (v == 0) return "orb";
        if (v == 1) return "block";
        if (v == 2) return "diamond";
        if (v == 3) return "peak";
        if (v == 4) return "hex";
        if (v == 5) return "twin";
        if (v == 6) return "gate";
        return "seed";
    }

    function _shapeSvg(uint8 shape, string memory fill, string memory opacity) private pure returns (string memory) {
        // 8도형: 중심부 기하 형태. r=56.
        if (shape == 0) {
            return string.concat("<circle cx='100' cy='100' r='56' fill='", fill, "' fill-opacity='", opacity, "'/>");
        }
        if (shape == 1) {
            return string.concat(
                "<rect x='48' y='48' width='104' height='104' fill='", fill, "' fill-opacity='", opacity, "'/>"
            );
        }
        if (shape == 2) {
            return string.concat(
                "<polygon points='100,40 160,100 100,160 40,100' fill='", fill, "' fill-opacity='", opacity, "'/>"
            );
        }
        if (shape == 3) {
            return
                string.concat(
                    "<polygon points='100,44 156,148 44,148' fill='", fill, "' fill-opacity='", opacity, "'/>"
                );
        }
        if (shape == 4) {
            return string.concat(
                "<polygon points='100,42 148,70 148,130 100,158 52,130 52,70' fill='",
                fill,
                "' fill-opacity='",
                opacity,
                "'/>"
            );
        }
        if (shape == 5) {
            return string.concat(
                "<circle cx='72' cy='100' r='34' fill='",
                fill,
                "' fill-opacity='",
                opacity,
                "'/><circle cx='128' cy='100' r='34' fill='",
                fill,
                "' fill-opacity='",
                opacity,
                "'/>"
            );
        }
        if (shape == 6) {
            return string.concat(
                "<rect x='84' y='40' width='32' height='120' rx='14' fill='", fill, "' fill-opacity='", opacity, "'/>"
            );
        }
        return string.concat("<circle cx='100' cy='100' r='20' fill='", fill, "' fill-opacity='", opacity, "'/>");
    }

    function _patternName(uint8 v) private pure returns (string memory) {
        if (v == 0) return "veil";
        if (v == 1) return "mist";
        if (v == 2) return "water";
        if (v == 3) return "glass";
        if (v == 4) return "leaf";
        if (v == 5) return "stone";
        if (v == 6) return "steel";
        return "gold";
    }

    function _haloName(uint8 v) private pure returns (string memory) {
        if (v == 0) return "none";
        if (v == 1) return "thin";
        if (v == 2) return "wide";
        if (v == 3) return "double";
        if (v == 4) return "glow";
        if (v == 5) return "ring";
        if (v == 6) return "arc";
        return "star";
    }

    // ---- 표준 인터페이스 ----

    function supportsInterface(bytes4 interfaceId) public view override(ERC721, ERC2981) returns (bool) {
        return super.supportsInterface(interfaceId);
    }
}
